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
const SESSION_ENDED: &str = "The game client running this script closed (teleport or rejoin) before it \
                             finished. Run it again if needed.";

#[derive(Debug, Serialize)]
pub struct Job {
    pub id: u64,
    pub code: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JobResult {
    pub id: u64,
    pub ok: bool,
    /// Each return value as JSON text.
    #[serde(default)]
    pub returns: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
    pub error: Option<String>,
    /// Kick or disconnect reason, when the game is dead.
    #[serde(default)]
    pub disconnected: Option<String>,
}

struct Pending {
    done: oneshot::Sender<JobResult>,
    /// Bridge session that took the job.
    session: Option<String>,
}

pub struct Broker {
    tx: mpsc::UnboundedSender<Job>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Job>>,
    pending: Mutex<HashMap<u64, Pending>>,
    /// Bridge session that polled last; each game client has its own.
    session: Mutex<String>,
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
            session: Mutex::default(),
            last_poll: Mutex::default(),
            polled: Notify::new(),
            next_id: AtomicU64::default(),
        }
    }
}

impl Broker {
    pub async fn poll(&self, session: &str) -> Option<Job> {
        if !self.connected() {
            eprintln!("bridge connected");
        }
        *self.last_poll.lock().unwrap() = Some(Instant::now());
        self.polled.notify_waiters();
        let mut rx = self.rx.lock().await;
        // Checked after the lock, so jobs the previous session's last poll took are seen.
        if *self.session.lock().unwrap() != session {
            self.end_session(session);
        }
        let deadline = Instant::now() + POLL_HOLD;
        while let Ok(Some(job)) = timeout_at(deadline, rx.recv()).await {
            if let Some(pending) = self.pending.lock().unwrap().get_mut(&job.id) {
                pending.session = Some(session.to_owned());
                return Some(job);
            }
        }
        None
    }

    pub fn complete(&self, result: JobResult) {
        if let Some(pending) = self.pending.lock().unwrap().remove(&result.id) {
            let _ = pending.done.send(result);
        }
    }

    /// A new game client is polling, so jobs the previous one took will never finish.
    fn end_session(&self, new: &str) {
        let old = std::mem::replace(&mut *self.session.lock().unwrap(), new.to_owned());
        let mut pending = self.pending.lock().unwrap();
        for (id, job) in pending.extract_if(|_, job| job.session.as_ref() == Some(&old)) {
            let _ = job.done.send(JobResult {
                id,
                error: Some(SESSION_ENDED.into()),
                ..JobResult::default()
            });
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
        let (done, done_rx) = oneshot::channel();
        let job = Pending {
            done,
            session: None,
        };
        self.pending.lock().unwrap().insert(id, job);
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
            disconnected: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_bridge_then_round_trips() {
        let broker = Arc::new(Broker::default());
        let bridge = broker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let job = bridge.poll("a").await.unwrap();
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
            let job = bridge.poll("a").await.unwrap();
            bridge.complete(reply(&job));
        });
        let result = broker.execute("fresh".into(), Duration::from_secs(5)).await;
        assert_eq!(result.unwrap().returns, ["\"fresh\""]);
    }

    #[tokio::test(start_paused = true)]
    async fn new_session_ends_jobs_taken_by_the_old_one() {
        let broker = Arc::new(Broker::default());
        let (old, new) = (broker.clone(), broker.clone());
        // The old client's poll takes the job, then the client closes; the new one polls meanwhile.
        tokio::spawn(async move { old.poll("old").await });
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            new.poll("new").await
        });
        tokio::time::sleep(Duration::from_secs(2)).await;
        let started = Instant::now();
        let result = broker.execute("lost".into(), Duration::from_secs(60)).await;
        assert_eq!(result.unwrap().error.as_deref(), Some(SESSION_ENDED));
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
}
