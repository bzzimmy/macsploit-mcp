use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use crate::broker::Broker;

const MAX_OUTPUT_CHARS: usize = 50_000;

#[derive(Clone)]
pub struct Server {
    broker: Arc<Broker>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Luau source. Runs in `MacSploit`'s executor environment; `return` values are sent back.
    code: String,
    /// Seconds to wait for the script to finish. Default 30.
    timeout_secs: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct ExecuteOutput {
    ok: bool,
    returns: Vec<String>,
    output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tool_router]
impl Server {
    pub fn new(broker: Arc<Broker>) -> Self {
        Self { broker }
    }

    #[tool(
        description = "Run Luau in the connected Roblox client with the full MacSploit API. \
                       Captures print/warn output and return values (tables as JSON).",
        output_schema = schema_for_output::<ExecuteOutput>(),
        annotations(read_only_hint = false, destructive_hint = true, open_world_hint = true)
    )]
    async fn execute(
        &self,
        Parameters(p): Parameters<ExecuteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let wait = Duration::from_secs(p.timeout_secs.unwrap_or(30).clamp(1, 600));
        let output = match self.broker.execute(p.code, wait).await {
            Ok(result) => ExecuteOutput {
                ok: result.ok,
                returns: result.returns,
                output: truncate(&result.output),
                error: result.error,
            },
            Err(err) => ExecuteOutput {
                ok: false,
                returns: Vec::new(),
                output: Vec::new(),
                error: Some(err.to_string()),
            },
        };
        let value = serde_json::to_value(&output)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(if output.ok {
            CallToolResult::structured(value)
        } else {
            CallToolResult::structured_error(value)
        })
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "`tool_handler` generates async trait methods that don't await"
)]
#[tool_handler(
    name = "macsploit",
    instructions = "Execute Luau inside a live Roblox client through MacSploit."
)]
impl ServerHandler for Server {}

fn truncate(lines: &[String]) -> Vec<String> {
    let mut total = 0;
    let mut kept = Vec::new();
    for line in lines {
        total += line.len();
        if total > MAX_OUTPUT_CHARS {
            kept.push(format!(
                "[truncated: {} more lines]",
                lines.len() - kept.len()
            ));
            break;
        }
        kept.push(line.clone());
    }
    kept
}
