use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::broker::{Broker, JobResult};
use crate::http::{self, ExecuteRequest, ExecuteResponse};

pub const BRIDGE_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 8766);

/// Owns the bridge port if free; otherwise forwards to the process that does.
pub struct Bridge {
    broker: Arc<Broker>,
    serving: Mutex<bool>,
    client: reqwest::Client,
}

impl Bridge {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            broker: Arc::new(Broker::default()),
            serving: Mutex::new(false),
            client: reqwest::Client::new(),
        })
    }

    pub async fn execute(&self, code: String, wait: Duration) -> Result<JobResult> {
        if self.ensure_serving().await {
            return self.broker.execute(code, wait).await;
        }
        match self.forward(&code, wait).await {
            Err(err) if is_connect(&err) && self.ensure_serving().await => {
                self.broker.execute(code, wait).await
            }
            result => result,
        }
    }

    /// Binds the bridge port unless another process already holds it.
    pub async fn ensure_serving(&self) -> bool {
        let mut serving = self.serving.lock().await;
        if !*serving && let Ok(listener) = TcpListener::bind(BRIDGE_ADDR).await {
            let broker = self.broker.clone();
            tokio::spawn(async move {
                if let Err(err) = http::serve(broker, listener).await {
                    eprintln!("bridge server failed: {err:#}");
                }
            });
            *serving = true;
        }
        *serving
    }

    async fn forward(&self, code: &str, wait: Duration) -> Result<JobResult> {
        let response = self
            .client
            .post(format!("http://{BRIDGE_ADDR}/execute"))
            .json(&ExecuteRequest {
                code: code.to_owned(),
                timeout_secs: wait.as_secs(),
            })
            .timeout(wait + Duration::from_secs(15))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("Port 8766 is used by another program.");
        }
        response
            .json::<ExecuteResponse>()
            .await
            .context("Port 8766 is used by another program.")?
            .map_err(|err| anyhow!(err))
    }
}

fn is_connect(err: &anyhow::Error) -> bool {
    err.downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_connect)
}
