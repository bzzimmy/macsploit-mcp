use std::collections::HashMap;
use std::pin::pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Instant, timeout, timeout_at};

const POLL_HOLD: Duration = Duration::from_secs(10);
const BRIDGE_TTL: Duration = Duration::from_secs(30);
const BRIDGE_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: u64,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobResult {
    pub id: u64,
    pub ok: bool,
    /// Each return value as JSON text.
    #[serde(default)]
    pub returns: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
    pub error: Option<String>,
}

pub struct Broker {
    tx: mpsc::UnboundedSender<Job>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Job>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<JobResult>>>,
    last_poll: Mutex<Option<Instant>>,
    polled: Notify,
    next_id: AtomicU64,
}

impl Default for Broker {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
            pending: Mutex::default(),
            last_poll: Mutex::default(),
            polled: Notify::new(),
            next_id: AtomicU64::default(),
        }
    }
}

impl Broker {
    pub async fn poll(&self) -> Option<Job> {
        if !self.connected() {
            eprintln!("bridge connected");
        }
        *self.last_poll.lock().unwrap() = Some(Instant::now());
        self.polled.notify_waiters();
        let mut rx = self.rx.lock().await;
        let deadline = Instant::now() + POLL_HOLD;
        while let Ok(Some(job)) = timeout_at(deadline, rx.recv()).await {
            if self.pending.lock().unwrap().contains_key(&job.id) {
                return Some(job);
            }
        }
        None
    }

    pub fn complete(&self, result: JobResult) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&result.id) {
            let _ = tx.send(result);
        }
    }

    pub async fn execute(&self, code: String, wait: Duration) -> Result<JobResult> {
        if !self.wait_connected().await {
            bail!(
                "No Roblox client connected. Open Roblox with MacSploit and join a game; \
                 if the bridge was just installed, rejoin once so autoexec runs it."
            );
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (done_tx, done_rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, done_tx);
        let _ = self.tx.send(Job { id, code });
        if let Ok(Ok(result)) = timeout(wait, done_rx).await {
            return Ok(result);
        }
        self.pending.lock().unwrap().remove(&id);
        bail!(
            "No result after {}s. The script may still be running in game.",
            wait.as_secs()
        )
    }

    async fn wait_connected(&self) -> bool {
        let deadline = Instant::now() + BRIDGE_WAIT;
        loop {
            let mut polled = pin!(self.polled.notified());
            polled.as_mut().enable();
            if self.connected() {
                return true;
            }
            if timeout_at(deadline, polled).await.is_err() {
                return false;
            }
        }
    }

    fn connected(&self) -> bool {
        self.last_poll
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < BRIDGE_TTL)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn reply(job: &Job) -> JobResult {
        JobResult {
            id: job.id,
            ok: true,
            returns: vec![format!("{:?}", job.code)],
            output: Vec::new(),
            error: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_bridge_then_round_trips() {
        let broker = Arc::new(Broker::default());
        let bridge = broker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let job = bridge.poll().await.unwrap();
            bridge.complete(reply(&job));
        });
        let result = broker.execute("hi".into(), Duration::from_secs(5)).await;
        assert_eq!(result.unwrap().returns, ["\"hi\""]);
    }

    #[tokio::test(start_paused = true)]
    async fn fails_without_bridge() {
        let err = Broker::default()
            .execute("x".into(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.to_string().starts_with("No Roblox client connected"));
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_jobs_are_not_delivered() {
        let broker = Arc::new(Broker::default());
        *broker.last_poll.lock().unwrap() = Some(Instant::now());
        let err = broker
            .execute("stale".into(), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(err.to_string().starts_with("No result after 1s"));

        let bridge = broker.clone();
        tokio::spawn(async move {
            let job = bridge.poll().await.unwrap();
            bridge.complete(reply(&job));
        });
        let result = broker.execute("fresh".into(), Duration::from_secs(5)).await;
        assert_eq!(result.unwrap().returns, ["\"fresh\""]);
    }
}
