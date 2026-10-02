use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ErrorData, tool, tool_router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::broker::JobResult;
use crate::mcp::Server;
use crate::workspace;

const COLLECT: &str = include_str!("../../lua/collect_scripts.lua");
const FALLBACK: &str = include_str!("../../lua/decompile_scripts.lua");
const LISTED_FAILURES: usize = 20;

#[derive(Deserialize, JsonSchema)]
pub struct DumpParams {
    /// Only dump scripts whose path contains this text (case-insensitive), e.g. `ReplicatedStorage`.
    filter: Option<String>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct DumpOutput {
    ok: bool,
    /// Folder with one `.luau` file per script, mirroring the game tree.
    dir: String,
    written: usize,
    failed: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Collected {
    place_id: u64,
    scripts: Vec<Script>,
}

#[derive(Deserialize)]
struct Script {
    path: Vec<String>,
    class: String,
    bytecode: String,
}

#[tool_router(router = dump_scripts_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Decompile the game's client scripts (LocalScripts, ModuleScripts) into \
                       `.luau` files mirroring the instance tree, then read and search them with \
                       your file tools. Replaces the previous dump unless `filter` is set.",
        output_schema = schema_for_output::<DumpOutput>(),
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn dump_scripts(
        &self,
        Parameters(p): Parameters<DumpParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let output = dump(self, p.filter.as_deref())
            .await
            .unwrap_or_else(|err| DumpOutput {
                error: Some(format!("{err:#}")),
                ..DumpOutput::default()
            });
        super::structured(&output, output.ok)
    }
}

async fn dump(server: &Server, filter: Option<&str>) -> Result<DumpOutput> {
    let wait = Duration::from_secs(60);
    let collected: Collected = first_return(server.bridge.execute(COLLECT.into(), wait).await?)?;
    let filter = filter.map(str::to_lowercase);
    let scripts: Vec<(usize, Script)> = collected
        .scripts
        .into_iter()
        .enumerate()
        .filter(|(_, s)| {
            filter
                .as_ref()
                .is_none_or(|f| s.path.join(".").to_lowercase().contains(f))
        })
        .collect();

    let mut sources = HashMap::new();
    let mut errors = HashMap::new();
    for (_, script) in &scripts {
        let bytecode = &script.bytecode;
        if sources.contains_key(bytecode) || errors.contains_key(bytecode) {
            continue;
        }
        match server.decompiler.decompile(bytecode).await {
            Ok(source) => sources.insert(bytecode.clone(), source),
            Err(err) => errors.insert(bytecode.clone(), format!("{err:#}")),
        };
    }

    if !errors.is_empty() {
        let mut ids = HashMap::new();
        for (i, script) in &scripts {
            if errors.contains_key(&script.bytecode) {
                ids.entry(i + 1).or_insert(&script.bytecode);
            }
        }
        let list: Vec<String> = ids.keys().map(ToString::to_string).collect();
        let code = format!("local ids = {{{}}}\n{FALLBACK}", list.join(","));
        let recovered: Value = first_return(server.bridge.execute(code, wait).await?)?;
        for (id, bytecode) in ids {
            match recovered.get(id.to_string()).and_then(Value::as_str) {
                Some(source) => {
                    errors.remove(bytecode);
                    sources.insert(bytecode.clone(), source.to_owned());
                }
                None => {
                    if let Some(err) = errors.get_mut(bytecode) {
                        err.push_str("; MacSploit fallback failed or ran out of time");
                    }
                }
            }
        }
    }

    let root = workspace::dir(&collected.place_id.to_string())?;
    if filter.is_none() {
        std::fs::remove_dir_all(&root)?;
        std::fs::create_dir_all(&root)?;
    }
    let mut used = HashSet::new();
    let mut output = DumpOutput {
        ok: true,
        dir: root.display().to_string(),
        ..DumpOutput::default()
    };
    for (_, script) in &scripts {
        let full_name = script.path.join(".");
        let Some(source) = sources.get(&script.bytecode) else {
            output.failed += 1;
            if output.failures.len() < LISTED_FAILURES {
                output
                    .failures
                    .push(format!("{full_name}: {}", errors[&script.bytecode]));
            }
            continue;
        };
        let path = file_path(&root, &script.path, &mut used);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &path,
            format!("-- {full_name} ({})\n\n{source}", script.class),
        )
        .with_context(|| format!("writing {}", path.display()))?;
        output.written += 1;
    }
    Ok(output)
}

fn first_return<T: DeserializeOwned>(result: JobResult) -> Result<T> {
    if !result.ok {
        bail!("{}", result.error.unwrap_or_default());
    }
    let json = result
        .returns
        .into_iter()
        .next()
        .context("script returned nothing")?;
    Ok(serde_json::from_str(&json)?)
}

/// `root/A/B/Name.luau`, with `~2`, `~3`… for siblings sharing a name (case-insensitively).
fn file_path(root: &Path, segments: &[String], used: &mut HashSet<String>) -> PathBuf {
    let (name, dirs) = segments
        .split_last()
        .map_or(("_", &[][..]), |(n, d)| (n.as_str(), d));
    let dir = dirs
        .iter()
        .fold(root.to_path_buf(), |dir, d| dir.join(sanitize(d)));
    let base = sanitize(name);
    let mut path = dir.join(format!("{base}.luau"));
    let mut n = 2;
    while !used.insert(path.to_string_lossy().to_lowercase()) {
        path = dir.join(format!("{base}~{n}.luau"));
        n += 1;
    }
    path
}

fn sanitize(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c == '/' || c.is_control() { '_' } else { c })
        .take(100)
        .collect();
    if clean.chars().all(|c| c == '.') {
        "_".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(path: &str) -> Vec<String> {
        path.split('.').map(str::to_owned).collect()
    }

    #[test]
    fn mirrors_tree_and_dedupes_siblings() {
        let root = Path::new("/r");
        let mut used = HashSet::new();
        let mut path = |p: &str| file_path(root, &segments(p), &mut used);
        assert_eq!(
            path("Workspace.Model.Script"),
            Path::new("/r/Workspace/Model/Script.luau")
        );
        assert_eq!(
            path("Workspace.Model.script"),
            Path::new("/r/Workspace/Model/script~2.luau")
        );
        assert_eq!(
            path("Workspace.Model.Script"),
            Path::new("/r/Workspace/Model/Script~3.luau")
        );
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("a/b"), "a_b");
        assert_eq!(sanitize(".."), "_");
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize("Drooling Zombie"), "Drooling Zombie");
    }
}
