mod bridge;
mod broker;
mod http;
mod install;
mod mcp;
mod tools;
mod workspace;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

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

    mcp::Server::new(bridge)
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
