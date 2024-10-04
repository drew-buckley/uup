use std::{net::IpAddr, time::Duration};

use anyhow::Error;
use async_trait::async_trait;

pub mod ping;

pub struct UupCheckResultContext {
    json: serde_json::Value,
    to_human_readable: Box<dyn Fn(&serde_json::Value) -> String>
}

impl UupCheckResultContext {
    pub fn new(json: serde_json::Value, to_human_readable: impl Fn(&serde_json::Value) -> String + 'static) -> Self {
        UupCheckResultContext {
            json,
            to_human_readable: Box::new(to_human_readable)
        }
    }

    pub fn get_context_str(&self, output_json: bool) -> String {
        if output_json {
            self.json.to_string()
        }
        else {
            (self.to_human_readable)(&self.json)
        }
    }
}

pub struct UupCheckResult {
    pub context: UupCheckResultContext,
    pub up: bool
}

#[async_trait]
pub trait Uup {
    async fn check(&self, host: IpAddr, port: Option<u16>, timeout: Duration) -> Result<UupCheckResult, Error>;
}

