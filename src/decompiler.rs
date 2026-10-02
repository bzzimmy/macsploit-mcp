use std::fs::Permissions;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Opiumware's decompiler, vendored and embedded so we ship a single executable.
const BINARY: &[u8] = include_bytes!("../vendor/opiumware-decompiler");
const ERROR_PREFIX: &str = "-- decompiler error:";

/// Opiumware's local Luau decompiler, run as a private child process.
#[derive(Default)]
pub struct Decompiler {
    process: Mutex<Option<(Child, String)>>,
    client: reqwest::Client,
}

impl Decompiler {
    pub async fn decompile(&self, bytecode_base64: &str) -> Result<String> {
        let url = self.url().await?;
        let source = self
            .client
            .post(url)
            .body(bytecode_base64.to_owned())
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .text()
            .await?;
        if source.starts_with(ERROR_PREFIX) {
            bail!("{}", source.trim());
        }
        Ok(source)
    }

    pub async fn stop(&self) {
        if let Some((mut child, _)) = self.process.lock().await.take() {
            let _ = child.kill().await;
        }
    }

    async fn url(&self) -> Result<String> {
        let mut process = self.process.lock().await;
        if let Some((child, url)) = process.as_mut()
            && child.try_wait()?.is_none()
        {
            return Ok(url.clone());
        }
        let binary = extract()?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let child = Command::new(&binary)
            .env("DECOMP_HOST", "127.0.0.1")
            .env("DECOMP_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", binary.display()))?;
        wait_for_port(port).await?;
        let url = format!("http://127.0.0.1:{port}/");
        *process = Some((child, url.clone()));
        Ok(url)
    }
}

async fn wait_for_port(port: u16) -> Result<()> {
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("decompiler did not start")
}

/// Writes the embedded binary to the cache once, named by content hash so versions never collide.
fn extract() -> Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    BINARY.hash(&mut hasher);
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join("Library/Caches/macsploit-mcp");
    let path = dir.join(format!("opiumware-decompiler-{:016x}", hasher.finish()));
    if !path.exists() {
        std::fs::create_dir_all(&dir)?;
        let partial = path.with_extension(std::process::id().to_string());
        std::fs::write(&partial, BINARY)?;
        std::fs::set_permissions(&partial, Permissions::from_mode(0o755))?;
        std::fs::rename(&partial, &path)?;
    }
    Ok(path)
}
