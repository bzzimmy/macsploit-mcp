use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::timeout;

const POLL_HOLD: Duration = Duration::from_secs(10);
const SESSION_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session: String,
    pub user: String,
    pub place_id: u64,
    pub job_id: String,
}

#[derive(Clone, Debug, Serialize)]
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

struct Session {
    info: SessionInfo,
    tx: mpsc::UnboundedSender<Job>,
    rx: Arc<Mutex<mpsc::UnboundedReceiver<Job>>>,
    last_seen: Instant,
}

#[derive(Default)]
pub struct Broker {
    sessions: Mutex<HashMap<String, Session>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<JobResult>>>,
    next_id: AtomicU64,
}

impl Broker {
    pub async fn poll(&self, info: SessionInfo) -> Option<Job> {
        let rx = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.entry(info.session.clone()).or_insert_with(|| {
                eprintln!("bridge connected: {} in place {}", info.user, info.place_id);
                let (tx, rx) = mpsc::unbounded_channel();
                Session {
                    info: info.clone(),
                    tx,
                    rx: Arc::new(Mutex::new(rx)),
                    last_seen: Instant::now(),
                }
            });
            session.info = info;
            session.last_seen = Instant::now();
            session.rx.clone()
        };
        let mut rx = rx.lock().await;
        timeout(POLL_HOLD, rx.recv()).await.ok().flatten()
    }

    pub async fn complete(&self, result: JobResult) {
        if let Some(tx) = self.pending.lock().await.remove(&result.id) {
            let _ = tx.send(result);
        }
    }

    pub async fn sessions(&self) -> Vec<SessionInfo> {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| s.last_seen.elapsed() < SESSION_TTL);
        sessions.values().map(|s| s.info.clone()).collect()
    }

    pub async fn execute(
        &self,
        code: String,
        session: Option<&str>,
        wait: Duration,
    ) -> Result<JobResult> {
        let tx = self.pick(session).await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (done_tx, done_rx) = oneshot::channel();
        self.pending.lock().await.insert(id, done_tx);
        tx.send(Job { id, code })
            .map_err(|_| anyhow!("Session disconnected before the script was sent."))?;
        match timeout(wait, done_rx).await {
            Ok(Ok(result)) => Ok(result),
            _ => {
                self.pending.lock().await.remove(&id);
                bail!(
                    "No result after {}s. The script may still be running in game.",
                    wait.as_secs()
                )
            }
        }
    }

    async fn pick(&self, session: Option<&str>) -> Result<mpsc::UnboundedSender<Job>> {
        let live = self.sessions().await;
        let sessions = self.sessions.lock().await;
        if let Some(id) = session {
            return sessions
                .get(id)
                .filter(|s| s.last_seen.elapsed() < SESSION_TTL)
                .map(|s| s.tx.clone())
                .ok_or_else(|| {
                    anyhow!("Unknown session `{id}`. Live sessions: {}", describe(&live))
                });
        }
        match live.as_slice() {
            [] => bail!(
                "No Roblox client connected. Open Roblox with MacSploit and join a game; \
                 if the bridge was just installed, rejoin once so autoexec runs it."
            ),
            [only] => Ok(sessions[&only.session].tx.clone()),
            _ => bail!(
                "Multiple clients connected; pass `session`. Live sessions: {}",
                describe(&live)
            ),
        }
    }
}

fn describe(sessions: &[SessionInfo]) -> String {
    if sessions.is_empty() {
        return "none".into();
    }
    sessions
        .iter()
        .map(|s| format!("{} ({}, place {})", s.session, s.user, s.place_id))
        .collect::<Vec<_>>()
        .join(", ")
}
