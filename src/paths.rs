//! Path validation helpers for scenario and fixture IDs.

use anyhow::{anyhow, Result};
use std::path::Path;

/// Validate that a string is a clean, relative path without `.` or `..`.
pub(crate) fn validate_relative_path(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("value is empty"));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(anyhow!("path must be relative"));
    }
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_)
            | std::path::Component::CurDir => {
                return Err(anyhow!("path contains invalid component"));
            }
            std::path::Component::Normal(_) => {}
        }
    }
    Ok(())
}
