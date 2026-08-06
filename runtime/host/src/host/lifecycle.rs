use std::time::Duration;

use tokio::time::sleep;
use tracing::warn;

use crate::error::Result;

use super::{router_session, RuntimeHost};

impl RuntimeHost {
    pub async fn run_forever(self) -> Result<()> {
        self.run_reconnect_loop().await
    }

    async fn run_reconnect_loop(self) -> Result<()> {
        let mut backoff = Duration::from_millis(250);
        loop {
            match self.run_router_session_once().await {
                Ok(()) => {
                    backoff = Duration::from_millis(250);
                    warn!(
                        event = "runtime.router_disconnected",
                        reconnect_in_ms = backoff.as_millis() as u64
                    );
                }
                Err(error) => {
                    warn!(
                        event = "runtime.router_connection_error",
                        error = %format_args!("{error:#}"),
                        reconnect_in_ms = backoff.as_millis() as u64
                    );
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }

    async fn run_router_session_once(&self) -> Result<()> {
        router_session::run_once(self.clone()).await
    }
}
