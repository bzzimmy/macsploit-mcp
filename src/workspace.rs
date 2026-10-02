use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Result;

/// `.macsploit/` in the working directory, or a temp dir when there's no project to put it in.
static ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match std::env::current_dir() {
        Ok(cwd) if cwd != Path::new("/") && Some(&cwd) != home.as_ref() => cwd.join(".macsploit"),
        _ => std::env::temp_dir().join("macsploit-mcp"),
    }
});

/// Path of `subdir`, creating it and the root `.gitignore`.
pub fn dir(subdir: &str) -> Result<PathBuf> {
    let dir = ROOT.join(subdir);
    std::fs::create_dir_all(&dir)?;
    let ignore = ROOT.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    Ok(dir)
}

pub fn write(subdir: &str, name: &str, contents: &str) -> Result<PathBuf> {
    let path = dir(subdir)?.join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}
