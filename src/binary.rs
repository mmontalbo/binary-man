//! Binary path resolution and hashing helpers.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::hashing::sha256_file;

/// Resolve a binary path, ensuring it exists and is executable.
///
/// Symlinks are resolved so hashing is stable even when the execution path
/// preserves argv[0] semantics.
pub(crate) fn resolve_binary(path: &Path) -> Result<PathBuf> {
    let resolved = fs::canonicalize(path)
        .with_context(|| format!("resolve binary path {}", path.display()))?;
    let metadata =
        fs::metadata(&resolved).with_context(|| format!("stat binary {}", resolved.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("binary is not a regular file"));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(anyhow!("binary is not executable"));
    }
    Ok(resolved)
}

/// Hash the binary contents using SHA-256.
pub(crate) fn hash_binary(path: &Path) -> Result<String> {
    sha256_file(path).context("hash binary")
}
