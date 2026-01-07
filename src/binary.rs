//! Binary path resolution and hashing helpers.

use anyhow::{anyhow, Context, Result};
use std::env;
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

/// Resolved target details, preserving argv[0] path and canonical identity.
pub(crate) struct BinaryTarget {
    pub(crate) exec_path: PathBuf,
    pub(crate) resolved_path: PathBuf,
}

/// Resolve a binary path or name, searching PATH when needed.
pub(crate) fn resolve_binary_input(value: &str) -> Result<BinaryTarget> {
    if value.trim().is_empty() {
        return Err(anyhow!("binary is empty"));
    }
    if value.contains('/') {
        let exec_path = normalize_exec_path(Path::new(value))?;
        let resolved_path = resolve_binary(&exec_path)?;
        return Ok(BinaryTarget {
            exec_path,
            resolved_path,
        });
    }
    let path_var = env::var_os("PATH").ok_or_else(|| anyhow!("PATH is not set"))?;
    let cwd = env::current_dir().context("resolve cwd for PATH search")?;
    let mut last_err = None;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(value);
        let exec_path = if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        };
        if !exec_path.exists() {
            continue;
        }
        match resolve_binary(&exec_path) {
            Ok(resolved_path) => {
                return Ok(BinaryTarget {
                    exec_path,
                    resolved_path,
                })
            }
            Err(err) => last_err = Some(err),
        }
    }
    if let Some(err) = last_err {
        Err(err)
    } else {
        Err(anyhow!("binary not found in PATH"))
    }
}

fn normalize_exec_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = env::current_dir().context("resolve cwd for binary path")?;
    Ok(cwd.join(path))
}

/// Hash the binary contents using SHA-256.
pub(crate) fn hash_binary(path: &Path) -> Result<String> {
    sha256_file(path).context("hash binary")
}
