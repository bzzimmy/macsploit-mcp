use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::{ServerHandler, tool_handler};

use crate::bridge::Bridge;

#[derive(Clone)]
pub struct Server {
    pub(crate) bridge: Arc<Bridge>,
    tool_router: ToolRouter<Self>,
}

impl Server {
    pub fn new(bridge: Arc<Bridge>) -> Self {
        Self {
            bridge,
            tool_router: Self::execute_router(),
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
    instructions = "Runs Luau inside the user's live Roblox client through MacSploit. \
                    The executor API is sUNC: https://docs.sunc.io. Filter and aggregate in Luau \
                    and return only what you need; large results are saved to a file you can grep."
)]
impl ServerHandler for Server {}
