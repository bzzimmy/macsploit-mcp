use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ErrorData, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::broker::JobResult;
use crate::mcp::Server;
use crate::workspace;

/// About 3K tokens.
const INLINE_CHARS: usize = 12_000;

#[derive(Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Luau source for quick one-off checks. `return` values are sent back.
    code: Option<String>,
    /// Path to a `.lua`/`.luau` file to run instead, relative to the working directory.
    file: Option<PathBuf>,
    /// Seconds to wait for the script to finish. Default 30, max 600.
    timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecuteOutput {
    ok: bool,
    /// Return values as JSON. Instances become full paths, other userdata strings.
    returns: Vec<Value>,
    output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// File with the complete result, set when it was too large to return inline.
    #[serde(skip_serializing_if = "Option::is_none")]
    full_output: Option<String>,
    /// Set when the client was kicked or disconnected: the game is dead until you rejoin.
    #[serde(skip_serializing_if = "Option::is_none")]
    disconnected: Option<String>,
}

#[tool_router(router = execute_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Run Luau in the connected Roblox client with the full MacSploit (sUNC) API. \
                       Pass `code` for quick one-off checks, or `file` for scripts you'll iterate on: \
                       edit the file and rerun instead of resending code. Returns print/warn output \
                       and return values. Filter in Luau and return only what you need; results over \
                       ~3K tokens are cut and saved to `full_output`. Rejoin with `TeleportService` \
                       (the call returns before the teleport; the next call waits for the new \
                       session). To capture outgoing remotes, hook both `__namecall` \
                       (`hookmetamethod`) and `FireServer`/`InvokeServer` (`hookfunction`), since \
                       direct calls bypass `__namecall`; skip your own calls with `checkcaller()` and \
                       keep the log in `getgenv()`.",
        output_schema = schema_for_output::<ExecuteOutput>(),
        annotations(read_only_hint = false, destructive_hint = true, open_world_hint = true)
    )]
    async fn execute(
        &self,
        Parameters(p): Parameters<ExecuteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let wait = Duration::from_secs(p.timeout_secs.unwrap_or(30).clamp(1, 600));
        let result = match load(p.code, p.file.as_deref()) {
            Ok(code) => self.bridge.execute(code, wait).await,
            Err(err) => Err(err),
        };
        let output = match result {
            Ok(result) => ExecuteOutput::from(result).fit(save),
            Err(err) => ExecuteOutput::failed(format!("{err:#}")),
        };
        super::structured(&output, output.ok)
    }
}

/// The Luau to run: inline `code` or the contents of `file`, exactly one of them.
fn load(code: Option<String>, file: Option<&Path>) -> Result<String> {
    match (code, file) {
        (Some(code), None) => Ok(code),
        (None, Some(file)) => {
            let path = std::path::absolute(file)?;
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("Could not read {}", path.display()))?;
            if source.trim().is_empty() {
                bail!("{} is empty.", path.display());
            }
            Ok(source)
        }
        _ => bail!("Pass exactly one of `code` or `file`."),
    }
}

fn save(text: &str) -> Result<PathBuf> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    workspace::write("output", &format!("{millis}.txt"), text)
}

impl From<JobResult> for ExecuteOutput {
    fn from(result: JobResult) -> Self {
        let returns = result
            .returns
            .into_iter()
            .map(|json| serde_json::from_str(&json).unwrap_or(Value::String(json)))
            .collect();
        Self {
            ok: result.ok,
            returns,
            output: result.output,
            error: result.error,
            full_output: None,
            disconnected: result.disconnected,
        }
    }
}

impl ExecuteOutput {
    fn failed(error: String) -> Self {
        Self {
            ok: false,
            returns: Vec::new(),
            output: Vec::new(),
            error: Some(error),
            full_output: None,
            disconnected: None,
        }
    }

    /// Keeps the result under `INLINE_CHARS`, saving the complete version when trimmed.
    fn fit(mut self, save: impl FnOnce(&str) -> Result<PathBuf>) -> Self {
        if serde_json::to_string(&self).map_or(0, |json| json.len()) <= INLINE_CHARS {
            return self;
        }
        match save(&self.render()) {
            Ok(path) => self.full_output = Some(path.display().to_string()),
            Err(err) => eprintln!("could not save full output: {err:#}"),
        }
        let mut budget = INLINE_CHARS;
        for value in &mut self.returns {
            *value = clip(value.take(), &mut budget);
        }
        let total = self.output.len();
        let kept = self
            .output
            .iter()
            .take_while(|line| {
                let fits = line.len() <= budget;
                budget = budget.saturating_sub(line.len());
                fits
            })
            .count();
        if kept < total {
            self.output.truncate(kept);
            self.output
                .push(format!("[truncated: {} more lines]", total - kept));
        }
        self
    }

    fn render(&self) -> String {
        let mut text = String::new();
        if let Some(error) = &self.error {
            let _ = write!(text, "-- error\n{error}\n\n");
        }
        if !self.output.is_empty() {
            let _ = write!(text, "-- output\n{}\n\n", self.output.join("\n"));
        }
        for (i, value) in self.returns.iter().enumerate() {
            let body = match value {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let _ = write!(text, "-- return {}\n{body}\n\n", i + 1);
        }
        text
    }
}

fn clip(value: Value, budget: &mut usize) -> Value {
    let len = value.to_string().len();
    if len <= *budget {
        *budget -= len;
        return value;
    }
    let marker = format!("…[truncated: {len} chars]");
    let clipped = match value {
        Value::String(s) if *budget > marker.len() => {
            let prefix: String = s.chars().take(*budget - marker.len()).collect();
            Value::String(prefix + &marker)
        }
        _ => Value::String(marker),
    };
    *budget = budget.saturating_sub(clipped.to_string().len());
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(returns: &[&str], output: Vec<String>) -> JobResult {
        JobResult {
            id: 0,
            ok: true,
            returns: returns.iter().map(ToString::to_string).collect(),
            output,
            error: None,
            disconnected: None,
        }
    }

    #[test]
    fn loads_code_or_file() {
        let dir = std::env::temp_dir().join(format!("macsploit-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("script.luau");
        let empty = dir.join("empty.luau");
        std::fs::write(&script, "return 1").unwrap();
        std::fs::write(&empty, " \n").unwrap();

        assert_eq!(load(Some("return 2".into()), None).unwrap(), "return 2");
        assert_eq!(load(None, Some(&script)).unwrap(), "return 1");
        let error = |code: Option<&str>, file: Option<&Path>| {
            format!("{:#}", load(code.map(Into::into), file).unwrap_err())
        };
        assert!(error(None, None).starts_with("Pass exactly one"));
        assert!(error(Some("x"), Some(&script)).starts_with("Pass exactly one"));
        assert!(error(None, Some(&dir.join("missing.luau"))).starts_with("Could not read"));
        assert!(error(None, Some(&empty)).ends_with("is empty."));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_returns_as_json() {
        let out = ExecuteOutput::from(result(&["1", "\"a\"", "{\"k\":[true]}", "nan"], vec![]));
        assert_eq!(
            out.returns,
            [
                Value::from(1),
                Value::from("a"),
                serde_json::json!({"k": [true]}),
                Value::from("nan")
            ]
        );
    }

    #[test]
    fn small_results_stay_inline() {
        let out = ExecuteOutput::from(result(&["1"], vec!["print: hi".into()]))
            .fit(|_| panic!("should not save"));
        assert_eq!(out.full_output, None);
        assert_eq!(out.output, ["print: hi"]);
    }

    #[test]
    fn large_results_are_trimmed_and_saved() {
        let big = format!("\"{}\"", "x".repeat(50_000));
        let lines = (0..2_000).map(|i| format!("print: line {i}")).collect();
        let mut saved = String::new();
        let out = ExecuteOutput::from(result(&[&big], lines)).fit(|text| {
            saved = text.to_owned();
            Ok(PathBuf::from("/tmp/full.txt"))
        });
        assert_eq!(out.full_output.as_deref(), Some("/tmp/full.txt"));
        assert!(serde_json::to_string(&out).unwrap().len() <= INLINE_CHARS + 200);
        assert!(
            out.returns[0]
                .as_str()
                .unwrap()
                .ends_with("[truncated: 50002 chars]")
        );
        assert!(out.output.last().unwrap().starts_with("[truncated:"));
        assert!(saved.contains("print: line 1999") && saved.contains(&"x".repeat(50_000)));
    }
}
