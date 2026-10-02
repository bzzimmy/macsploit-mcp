mod bridge;
mod broker;
mod decompiler;
mod http;
mod install;
mod mcp;
mod tools;
mod workspace;

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use tokio::signal::unix::{SignalKind, signal};

use crate::decompiler::Decompiler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(err) = install::install_bridge() {
        eprintln!("could not install bridge: {err:#}");
    }

    let bridge = bridge::Bridge::new();
    if !bridge.ensure_serving().await {
        eprintln!(
            "{} is in use; forwarding to the process that owns it",
            bridge::BRIDGE_ADDR
        );
    }

    let decompiler = Arc::new(Decompiler::default());
    let service = mcp::Server::new(bridge, decompiler.clone())
        .serve(stdio())
        .await?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = service.waiting() => { result?; }
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    decompiler.stop().await;
    // Exit directly: tokio's blocking stdin reader would otherwise hold up runtime shutdown.
    std::process::exit(0)
}
