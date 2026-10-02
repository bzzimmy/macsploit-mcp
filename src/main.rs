mod broker;
mod http;
mod install;
mod mcp;

use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

const BRIDGE_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 8766);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(err) = install::install_bridge() {
        eprintln!("could not install bridge: {err:#}");
    }

    let broker = Arc::new(broker::Broker::default());
    let http_broker = broker.clone();
    tokio::spawn(async move {
        if let Err(err) = http::serve(http_broker, BRIDGE_ADDR).await {
            eprintln!("bridge server on {BRIDGE_ADDR} failed: {err:#}");
        }
    });

    mcp::Server::new(broker)
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
