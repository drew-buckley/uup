use std::{collections::HashMap, net::{IpAddr, ToSocketAddrs}, process::ExitCode, time::Duration};

use anyhow::Error;
use argh::FromArgs;
use dns_lookup::lookup_host;
use futures::{Future, Stream};
use regex::Regex;
use tokio::{signal::unix::{signal, SignalKind}, time::sleep};
use uup::{ping::PingUup, Uup};

const PING_TYPE: &str = "ping";

const EXIT_CODE_HOST_UP: u8 = 0;
const EXIT_CODE_HOST_DOWN: u8 = 1;
const EXIT_CODE_ERROR: u8 = 2;

const RUNMODE_ONESHOT: &str = "oneshot";
const RUNMODE_FOREVER: &str = "forever";
const RUNMODE_COUNT: &str = "count";

enum RunMode {
    OneShot,
    Forever,
    Count(u128)
}

#[derive(FromArgs)]
/// U up? Remote host availability querier.
struct Args {
    /// run mode (either "oneshot", "forever", or "count")
    #[argh(positional)]
    run_mode: String,

    /// protocol type i.e. ping (default)
    #[argh(option, short = 't', long = "type", default = "get_type_default().to_string()")]
    protocol: String,

    /// host to query
    #[argh(option, short = 'h', long = "host")]
    host: Option<String>,

    /// port number to use (if relevant to the protocol)
    #[argh(option, short = 'p', long = "port")]
    port: Option<u16>,

    /// port number to use (if using count run mode)
    #[argh(option, short = 'c', long = "count")]
    count: Option<u128>,

    /// timeout in seconds (float)
    #[argh(option, short = 's', long = "timeout", default = "get_timeout_default()")]
    timeout: f32,

    /// delay between attempts in seconds (float)
    #[argh(option, short = 'd', long = "delay", default = "get_delay_default()")]
    delay: f32,

    /// set to expect *all* queries to succeed
    #[argh(switch, short = 'e', long = "exclusive")]
    exclusive: bool,

    /// STDOUT formatted to JSON
    #[argh(switch, short = 'j', long = "json")]
    print_json: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Args = argh::from_env();
    let mut protocol_type_map: HashMap<&str, Box<dyn Uup>> = HashMap::new();
    protocol_type_map.insert(PING_TYPE, Box::new(PingUup::new()));

    let run_modes = vec![
        RUNMODE_ONESHOT,
        RUNMODE_FOREVER,
        RUNMODE_COUNT
    ];

    let run_mode;
    let protocol;
    let addr_str;
    if !run_modes.contains(&args.run_mode.as_str()) {
        if let Ok(addrs) = lookup_host(args.run_mode.as_str()) {
            protocol = PING_TYPE;
            run_mode = RunMode::Forever;
            addr_str = addrs[0].to_string();
        }
        else {
            eprintln!("First argument must be run mode or resolvable hostname");
            return ExitCode::from(EXIT_CODE_ERROR)
        }
    }
    else {
        run_mode = match args.run_mode.as_str() {
            RUNMODE_ONESHOT => RunMode::OneShot,
            RUNMODE_FOREVER => RunMode::Forever,
            RUNMODE_COUNT => {
                if let Some(count) = args.count {
                    RunMode::Count(count)
                }
                else {
                    eprintln!("Must include --count argument when running in \"count\" mode");
                    return ExitCode::from(EXIT_CODE_ERROR)
                }
            }
            _ => {
                eprintln!("Unrecognized run mode: {}", args.run_mode);
                return ExitCode::from(EXIT_CODE_ERROR)
            }
        };
        protocol = args.protocol.as_str();
        addr_str = match args.host {
            Some(addr) => addr,
            None => {
                eprintln!("Must set --host argument");
                return ExitCode::from(EXIT_CODE_ERROR)
            }
        }
    }

    if args.timeout <= 0.0 {
        eprintln!("Timeout must be >0.0; got: {}", args.timeout);
        return ExitCode::from(EXIT_CODE_ERROR);
    }

    let exit_code;
    if let Some(uup_checker) = protocol_type_map.get(protocol) {
        if let Ok(ipaddr) = lookup_host(&addr_str) {
            let host_up = 
                run_uup_checker(
                    uup_checker,
                    run_mode,
                    ipaddr[0],
                    args.port,
                    Duration::from_secs_f32(args.timeout),
                    Duration::from_secs_f32(args.delay),
                    args.exclusive,
                    args.print_json)
                    .await.expect("Failed to run Uup Checker");
            if host_up {
                exit_code = ExitCode::from(EXIT_CODE_HOST_UP);
            }
            else {
                exit_code = ExitCode::from(EXIT_CODE_HOST_DOWN);
            }
        }
        else {
            eprintln!("Invalid IP address provided");
            exit_code = ExitCode::from(EXIT_CODE_ERROR);
        }
    }
    else {
        eprintln!("No supported protocol named: {}", args.protocol);
        eprintln!("Supported protocols:");
        for protocol in protocol_type_map.keys() {
            eprintln!("    {}", protocol);
        }
        eprintln!("");
        eprintln!("(Not seeing the one you want? It's feature may have not been included in the build)");
        exit_code = ExitCode::from(EXIT_CODE_ERROR);
    }

    exit_code
}

async fn run_uup_checker(
    uup_checker: &Box<dyn Uup>,
    run_mode: RunMode,
    ipaddr: IpAddr,
    port: Option<u16>,
    timeout: Duration,
    delay: Duration,
    exclusive: bool,
    output_json: bool
) -> Result<bool, Error> {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigint = signal(SignalKind::interrupt()).unwrap();

    let mut loop_count = 0u128;
    let mut host_is_up = exclusive;
    loop {
        let mut should_break = false;
        tokio::select! {
            result = uup_checker.check(ipaddr, port, timeout) => {
                let result = result?;
                if exclusive {
                    host_is_up &= result.up;
                }
                else {
                    host_is_up |= result.up;
                }
                println!("{}", result.context.get_context_str(output_json));
            }

            _ = sigterm.recv() => {
                eprintln!("Got SIGTERM; exiting");
                should_break = true;
            }

            _ = sigint.recv() => {
                eprintln!("Got SIGINT; exiting");
                should_break = true;
            }
        }

        let delay_sleep = sleep(delay);
        tokio::pin!(delay_sleep);
        tokio::select! {
            _ = &mut delay_sleep => { }

            _ = sigterm.recv() => {
                eprintln!("Got SIGTERM; exiting");
                should_break = true;
            }

            _ = sigint.recv() => {
                eprintln!("Got SIGINT; exiting");
                should_break = true;
            }
        }

        if !should_break {
            should_break = match run_mode {
                RunMode::OneShot => true,
                RunMode::Forever => false,
                RunMode::Count(max_count) => loop_count >= max_count,
            };
        }

        if should_break {
            break;
        }

        loop_count += 1;
    }

    Ok(host_is_up)
}

fn get_type_default() -> &'static str {
    PING_TYPE
}

fn get_timeout_default() -> f32 {
    1.0
}

fn get_delay_default() -> f32 {
    1.0
}

fn is_ipv4_address(addr: &str) -> bool {
    let re = Regex::new(r"^((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?\b){4}$").unwrap();
    let dates: Vec<&str> = re.find_iter(addr).map(|m| m.as_str()).collect();
    return dates.len() == 1 && dates[0] == addr;
}

#[cfg(test)]
mod tests {
    use rand::random;

    use super::*;

    #[test]
    fn test_is_ipv4_address() {
        fn rand_u8() -> u8 {
            random()
        }

        for _ in 0..100 {
            let ipv4_addr = format!("{}.{}.{}.{}",
                rand_u8(), rand_u8(), rand_u8(), rand_u8());
            assert!(is_ipv4_address(&ipv4_addr))
        }

        assert!(!is_ipv4_address("-192.168.1.1"));
        assert!(!is_ipv4_address("192.168.1.1d"));
        assert!(!is_ipv4_address("192.168.1"));
        assert!(!is_ipv4_address("not even close"));
    }
}
