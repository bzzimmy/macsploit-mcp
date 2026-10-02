use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

const POLL_HOLD: Duration = Duration::from_secs(10);
const BRIDGE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: u64,
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct JobResult {
    pub id: u64,
    pub ok: bool,
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
        let mut rx = self.rx.lock().await;
        let deadline = tokio::time::Instant::now() + POLL_HOLD;
        while let Ok(Some(job)) = tokio::time::timeout_at(deadline, rx.recv()).await {
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
        if !self.connected() {
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

    fn connected(&self) -> bool {
        self.last_poll
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() < BRIDGE_TTL)
    }
}
