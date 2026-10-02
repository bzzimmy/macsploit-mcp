use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::{ServerHandler, tool_handler};

use crate::bridge::Bridge;
use crate::decompiler::Decompiler;

#[derive(Clone)]
pub struct Server {
    pub(crate) bridge: Arc<Bridge>,
    pub(crate) decompiler: Arc<Decompiler>,
    tool_router: ToolRouter<Self>,
}

impl Server {
    pub fn new(bridge: Arc<Bridge>, decompiler: Arc<Decompiler>) -> Self {
        Self {
            bridge,
            decompiler,
            tool_router: Self::execute_router() + Self::dump_scripts_router(),
        }
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "`tool_handler` generates async trait methods that don't await"
)]
#[tool_handler(
    router = self.tool_router,
    name = "macsploit",
    instructions = "Runs Luau in the user's live Roblox client through MacSploit (sUNC API: \
                    https://docs.sunc.io). Understand the game before writing code: inspect live \
                    state with small `execute` calls, and use `dump_scripts` plus your file tools \
                    to read its code. Verify results by returning state rather than assuming. \
                    State persists between runs until a rejoin; code spawned by a run keeps \
                    running after it returns, and later prints aren't captured."
)]
impl ServerHandler for Server {}
