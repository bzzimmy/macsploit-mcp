pub mod execute;

use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use serde::Serialize;

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
