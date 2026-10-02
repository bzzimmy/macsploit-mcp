pub mod dump_scripts;
pub mod execute;

use std::pin::pin;
use std::time::Duration;

use rmcp::model::{CallToolResult, ProgressNotificationParam, RequestMetaObject};
use rmcp::{ErrorData, Peer, RoleServer};
use serde::Serialize;
use tokio::time::{Instant, interval_at};

const PROGRESS_EVERY: Duration = Duration::from_secs(20);

/// Awaits `work`, sending progress every 20s so clients that time out idle requests keep waiting.
async fn with_progress<T>(
    meta: &RequestMetaObject,
    peer: &Peer<RoleServer>,
    work: impl Future<Output = T>,
) -> T {
    let Some(token) = meta.get_progress_token() else {
        return work.await;
    };
    let mut work = pin!(work);
    let started = Instant::now();
    let mut ticks = interval_at(started + PROGRESS_EVERY, PROGRESS_EVERY);
    loop {
        tokio::select! {
            output = &mut work => return output,
            _ = ticks.tick() => {
                let elapsed = started.elapsed();
                let progress = ProgressNotificationParam::new(token.clone(), elapsed.as_secs_f64())
                    .with_message(format!("Running for {}s", elapsed.as_secs()));
                let _ = peer.notify_progress(progress).await;
            }
        }
    }
}

/// Structured result; `isError` lets clients surface failures as tool errors.
fn structured(value: &impl Serialize, ok: bool) -> Result<CallToolResult, ErrorData> {
    let value =
        serde_json::to_value(value).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(if ok {
        CallToolResult::structured(value)
    } else {
        CallToolResult::structured_error(value)
    })
}
