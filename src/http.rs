use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use crate::broker::{Broker, JobResult, SessionInfo};

pub async fn serve(broker: Arc<Broker>, addr: SocketAddr) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/poll", post(poll))
        .route("/result", post(result))
        .with_state(broker);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn poll(State(broker): State<Arc<Broker>>, Json(info): Json<SessionInfo>) -> Response {
    match broker.poll(info).await {
        Some(job) => Json(job).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn result(State(broker): State<Arc<Broker>>, Json(result): Json<JobResult>) -> StatusCode {
    broker.complete(result).await;
    StatusCode::NO_CONTENT
}
