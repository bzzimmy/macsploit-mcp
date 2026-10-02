use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, RequestMetaObject};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ErrorData, Peer, RoleServer, tool, tool_router};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::broker::JobResult;
use crate::mcp::Server;
use crate::workspace;

const COLLECT: &str = include_str!("../../lua/collect_scripts.lua");
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
    /// Set when the client was kicked or disconnected: the game is dead until you rejoin.
    #[serde(skip_serializing_if = "Option::is_none")]
    disconnected: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Collected {
    place_id: u64,
    player: String,
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
        meta: RequestMetaObject,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let output = super::with_progress(&meta, &peer, dump(self, p.filter.as_deref()))
            .await
            .unwrap_or_else(|err| DumpOutput {
                error: Some(format!("{err:#}")),
                ..DumpOutput::default()
            });
        super::structured(&output, output.ok)
    }
}

async fn dump(server: &Server, filter: Option<&str>) -> Result<DumpOutput> {
    server.decompiler.start().await?;
    let wait = Duration::from_secs(60);
    let job = server.bridge.execute(COLLECT.into(), wait).await?;
    let disconnected = job.disconnected.clone();
    let collected: Collected = first_return(job)?;
    let filter = filter.map(str::to_lowercase);

    // Copies of the same script (templates, per-player clones) share bytecode: write each once.
    let mut groups: Vec<Vec<Script>> = Vec::new();
    let mut by_bytecode = HashMap::new();
    for script in collected.scripts {
        let full_name = script.path.join(".").to_lowercase();
        if filter.as_ref().is_some_and(|f| !full_name.contains(f)) {
            continue;
        }
        let i = *by_bytecode
            .entry(script.bytecode.clone())
            .or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
        groups[i].push(script);
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
        disconnected,
        ..DumpOutput::default()
    };
    for mut group in groups {
        group.sort_by_key(|s| (rank(&s.path, &collected.player), s.path.len()));
        let main = &group[0];
        let full_name = main.path.join(".");
        let source = match server.decompiler.decompile(&main.bytecode).await {
            Ok(source) => source,
            Err(err) => {
                output.failed += 1;
                if output.failures.len() < LISTED_FAILURES {
                    output.failures.push(format!("{full_name}: {err:#}"));
                }
                continue;
            }
        };
        let also = if group.len() > 1 {
            let others: Vec<String> = group[1..].iter().map(|s| s.path.join(".")).collect();
            format!("-- Also at: {}\n", others.join(", "))
        } else {
            String::new()
        };
        let header = format!("-- {full_name} ({})\n{also}", main.class);
        let path = file_path(&root, &main.path, &mut used);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{header}\n{source}"))
            .with_context(|| format!("writing {}", path.display()))?;
        output.written += 1;
    }
    Ok(output)
}

/// Which copy to keep: the live one under the local player first, `Starter*` templates last.
fn rank(path: &[String], player: &str) -> u8 {
    let root = path.first().map_or("", String::as_str);
    let owner = path.get(1).map_or("", String::as_str);
    match root {
        "Players" | "Workspace" if owner == player => 0,
        "Players" | "Workspace" => 2,
        _ if root.starts_with("Starter") => 3,
        _ => 1,
    }
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
        .map(|c| {
            if c == '/' || c == ' ' || c.is_control() {
                '_'
            } else {
                c
            }
        })
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
        assert_eq!(sanitize("Drooling Zombie"), "Drooling_Zombie");
    }

    #[test]
    fn keeps_the_live_copy() {
        let mut copies = [
            "StarterGui.Main",
            "Workspace.Other.Main",
            "Players.Me.PlayerGui.Main",
            "ReplicatedStorage.Main",
        ]
        .map(segments);
        copies.sort_by_key(|p| rank(p, "Me"));
        assert_eq!(
            copies.map(|p| p.join(".")),
            [
                "Players.Me.PlayerGui.Main",
                "ReplicatedStorage.Main",
                "Workspace.Other.Main",
                "StarterGui.Main",
            ]
        );
    }
}
