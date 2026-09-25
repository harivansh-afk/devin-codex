//! Best-effort local records containing only explicitly selected diagnostic fields.
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

pub(crate) fn path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|root| root.join("devin-codex/gateway.jsonl"))
}

pub(crate) async fn record(path: Option<PathBuf>, mut record: Value) {
    let Some(path) = path else {
        return;
    };
    record["timestamp"] = json!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    // A disk/logging failure must never turn a successful inference into a failure.
    let _ = tokio::task::spawn_blocking(move || append(&path, &record)).await;
}

fn append(path: &Path, record: &Value) -> io::Result<()> {
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    let mut options = fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        directory.mode(0o700);
        options
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    directory.create(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() >= MAX_LOG_BYTES {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Ok(());
        }
    }
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    if metadata.len().saturating_add(line.len() as u64) > MAX_LOG_BYTES {
        return Ok(());
    }
    file.write_all(&line)
}

#[cfg(test)]
#[path = "../tests/unit/diagnostics.rs"]
mod tests;
