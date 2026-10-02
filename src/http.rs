use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::bridge::BRIDGE_ADDR;
use crate::broker::{Broker, JobResult};

#[derive(Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub code: String,
    pub timeout_secs: u64,
}

pub type ExecuteResponse = Result<JobResult, String>;

pub async fn serve(broker: Arc<Broker>, listener: TcpListener) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/poll", post(poll))
        .route("/result", post(result))
        .route("/execute", post(execute))
        .layer(middleware::from_fn(local_only))
        .with_state(broker);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Rejects browser requests, including DNS rebinding, so only local tools can reach the bridge.
async fn local_only(request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .is_some_and(|host| host.as_bytes() == BRIDGE_ADDR.to_string().as_bytes());
    if !host_ok || headers.contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

async fn poll(State(broker): State<Arc<Broker>>) -> Response {
    match broker.poll().await {
        Some(job) => Json(job).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn result(State(broker): State<Arc<Broker>>, Json(result): Json<JobResult>) -> StatusCode {
    broker.complete(result);
    StatusCode::NO_CONTENT
}

async fn execute(
    State(broker): State<Arc<Broker>>,
    Json(request): Json<ExecuteRequest>,
) -> Json<ExecuteResponse> {
    let wait = Duration::from_secs(request.timeout_secs);
    Json(
        broker
            .execute(request.code, wait)
            .await
            .map_err(|err| err.to_string()),
    )
}
