use std::path::PathBuf;

use anyhow::{Context, Result};

const BRIDGE: &str = include_str!("../lua/MCPBridge.lua");

pub fn install_bridge() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join("Documents/Macsploit Automatic Execution");
    let path = dir.join("MCPBridge.lua");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(BRIDGE) {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&path, BRIDGE).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("installed bridge: {}", path.display());
    }
    Ok(path)
}
