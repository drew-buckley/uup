use std::{cell::Cell, net::IpAddr, sync::Arc, time::Duration};

use anyhow::Error;
use async_trait::async_trait;
use rand::random;
use serde_json::json;
use tokio::sync::Mutex;

use crate::{Uup, UupCheckResult, UupCheckResultContext};

pub struct PingUup {
    seq_cnt: Arc<Mutex<Cell<u16>>>
}

impl PingUup {
    pub fn new() -> Self {
        PingUup {
            seq_cnt: Arc::new(Mutex::new(Cell::new(0)))
        }
    }
}

#[async_trait]
impl Uup for PingUup {
    async fn check(&self, addr: IpAddr, port: Option<u16>, timeout: Duration) -> Result<UupCheckResult, Error> {
        if port.is_some() {
            eprintln!("WARNING: Ignoring port assignment; not supported for ping");
        }

        let seq_cnt;
        {
            let seq_cnt_lock = self.seq_cnt.lock().await;
            seq_cnt = seq_cnt_lock.get();
            seq_cnt_lock.set(seq_cnt_lock.get() + 1);
        }

        let pinger = tokio_icmp_echo::Pinger::new().await.unwrap();
        let ping_duration = pinger.ping(addr, random(), seq_cnt, timeout).await?;
        
        let up;
        let duration_secs;
        if let Some(duration) = ping_duration {
            up = true;
            duration_secs = duration.as_secs_f32()
        }
        else {
            up = false;
            duration_secs = -1.0;
        }

        Ok(UupCheckResult{
            up : true,
            context : build_result_context(
                build_json_object(up, duration_secs, seq_cnt, addr))
        })
    }
}

fn build_json_object(up: bool, duration_secs: f32, seq_cnt: u16, addr: IpAddr) -> serde_json::Value {
    json!(
        {
            "up"              : up,
            "duration"        : duration_secs,
            "unit"            : "s",
            "sequence_number" : seq_cnt,
            "address"         : addr.to_string()
        }
    )
}

fn build_result_context(json_obj: serde_json::Value) -> UupCheckResultContext {
    UupCheckResultContext::new(
        json_obj,
        |json_obj| {
            let up = json_obj.get("up").unwrap().as_bool().unwrap();
            let duration = json_obj.get("duration").unwrap().as_f64().unwrap() as f32;
            let unit = json_obj.get("unit").unwrap().as_str().unwrap();
            let seq_cnt = json_obj.get("sequence_number").unwrap().as_u64().unwrap();
            let addr = json_obj.get("address").unwrap().as_str().unwrap();
            if up {
                format!("Ping {} responded in {} {} (sequence_number={})", addr, duration, unit, seq_cnt)
            }
            else {
                format!("Ping {} timed out (sequence_number={})", addr, seq_cnt)
            }
        }
    )
}
