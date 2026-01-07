//! Fixture verification and deterministic materialization.

use anyhow::{anyhow, Context, Result};
use filetime::FileTime;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::hashing::{sha256_file, sha256_hex};
use crate::paths::validate_relative_path;

/// Fixture manifest format (authoritative metadata for fixture contents).
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureManifest {
    pub(crate) version: u32,
    pub(crate) description: String,
    pub(crate) entries: Vec<FixtureEntry>,
}

/// Catalog entry describing an available fixture.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureCatalogEntry {
    pub(crate) id: String,
    pub(crate) description: String,
}

/// A single fixture entry describing a file or directory.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureEntry {
    pub(crate) path: String,
    #[serde(rename = "type")]
    pub(crate) entry_type: String,
    pub(crate) mode: String,
    #[serde(default)]
    pub(crate) size: Option<u64>,
    #[serde(default)]
    pub(crate) sha256: Option<String>,
    pub(crate) mtime: i64,
}

/// A fixture materialized into a temporary run root.
pub(crate) struct PreparedFixture {
    pub(crate) fixture_root: PathBuf,
    pub(crate) fixture_hash: String,
    _temp_dir: TempDir,
}

/// Structured errors produced while preparing fixtures.
pub(crate) struct FixtureError {
    pub(crate) message: String,
    pub(crate) details: Vec<String>,
    pub(crate) is_missing: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
}

/// Build an absolute fixture directory from the repo fixture root.
pub(crate) fn fixture_root(root: &Path, fixture_id: &str) -> Result<PathBuf> {
    validate_relative_path(fixture_id)?;
    Ok(root.join(fixture_id))
}

/// Load the fixture catalog and return the allowed fixture IDs.
pub(crate) fn load_fixture_catalog(fixtures_root: &Path) -> Result<HashSet<String>> {
    let catalog_path = fixtures_root.join("catalog.json");
    let bytes = fs::read(&catalog_path)
        .with_context(|| format!("read fixture catalog {}", catalog_path.display()))?;
    let entries: Vec<FixtureCatalogEntry> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse fixture catalog {}", catalog_path.display()))?;

    let mut ids = HashSet::new();
    for entry in entries {
        validate_relative_path(&entry.id)
            .with_context(|| format!("fixture catalog id {}", entry.id))?;
        if entry.description.trim().is_empty() {
            return Err(anyhow!(
                "fixture catalog entry {} missing description",
                entry.id
            ));
        }
        if !ids.insert(entry.id.clone()) {
            return Err(anyhow!("duplicate fixture catalog entry {}", entry.id));
        }
        let fixture_dir = fixtures_root.join(&entry.id);
        if !fixture_dir.is_dir() {
            return Err(anyhow!(
                "fixture catalog entry not found on disk: {}",
                fixture_dir.display()
            ));
        }
        let manifest_path = fixture_dir.join("manifest.json");
        let tree_path = fixture_dir.join("tree");
        if !manifest_path.is_file() || !tree_path.is_dir() {
            return Err(anyhow!(
                "fixture catalog entry missing manifest.json or tree/: {}",
                entry.id
            ));
        }
    }
    Ok(ids)
}

/// Verify and materialize a fixture into a temporary run root.
pub(crate) fn prepare_fixture(fixture_dir: &Path) -> Result<PreparedFixture, FixtureError> {
    if !fixture_dir.exists() {
        return Err(FixtureError {
            message: format!("fixture not found: {}", fixture_dir.display()),
            details: Vec::new(),
            is_missing: true,
        });
    }
    let manifest_path = fixture_dir.join("manifest.json");
    let tree_path = fixture_dir.join("tree");
    if !manifest_path.exists() || !tree_path.exists() {
        return Err(FixtureError {
            message: "fixture missing manifest.json or tree/".to_string(),
            details: Vec::new(),
            is_missing: true,
        });
    }

    let manifest = load_manifest(&manifest_path).map_err(|err| FixtureError {
        message: "fixture manifest invalid".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    validate_manifest(&manifest).map_err(|err| FixtureError {
        message: "fixture manifest failed validation".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    verify_fixture_tree(&tree_path, &manifest, false).map_err(|err| FixtureError {
        message: "fixture tree failed validation".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;

    let fixture_hash = canonical_manifest_hash(&manifest).map_err(|err| FixtureError {
        message: "fixture manifest hashing failed".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;

    let temp_dir = TempDir::new().map_err(|err| FixtureError {
        message: "failed to create temp dir".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    let fixture_root = temp_dir.path().join("fixture");
    fs::create_dir_all(&fixture_root).map_err(|err| FixtureError {
        message: "failed to create fixture dir".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    copy_tree(&tree_path, &fixture_root).map_err(|err| FixtureError {
        message: "failed to copy fixture tree".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    apply_manifest(&fixture_root, &manifest).map_err(|err| FixtureError {
        message: "failed to apply fixture manifest".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;
    verify_fixture_tree(&fixture_root, &manifest, true).map_err(|err| FixtureError {
        message: "fixture materialization failed verification".to_string(),
        details: vec![err.to_string()],
        is_missing: false,
    })?;

    Ok(PreparedFixture {
        _temp_dir: temp_dir,
        fixture_root,
        fixture_hash,
    })
}

/// Validate a fixture on disk without materializing it.
pub(crate) fn validate_fixture(fixture_dir: &Path) -> Result<String> {
    if !fixture_dir.exists() {
        return Err(anyhow!(
            "fixture not found: {}",
            fixture_dir.display()
        ));
    }
    let manifest_path = fixture_dir.join("manifest.json");
    let tree_path = fixture_dir.join("tree");
    if !manifest_path.exists() || !tree_path.exists() {
        return Err(anyhow!("fixture missing manifest.json or tree/"));
    }

    let manifest = load_manifest(&manifest_path).context("load fixture manifest")?;
    validate_manifest(&manifest).context("validate fixture manifest")?;
    verify_fixture_tree(&tree_path, &manifest, false).context("verify fixture tree")?;
    canonical_manifest_hash(&manifest).context("hash fixture manifest")
}

fn load_manifest(path: &Path) -> Result<FixtureManifest> {
    let bytes = fs::read(path).with_context(|| format!("read manifest {}", path.display()))?;
    let manifest: FixtureManifest =
        serde_json::from_slice(&bytes).context("parse fixture manifest")?;
    Ok(manifest)
}

fn validate_manifest(manifest: &FixtureManifest) -> Result<()> {
    if manifest.version != 1 {
        return Err(anyhow!("unsupported manifest version {}", manifest.version));
    }
    let mut seen = HashSet::new();
    for entry in &manifest.entries {
        validate_relative_path(&entry.path)?;
        if !seen.insert(entry.path.clone()) {
            return Err(anyhow!("duplicate manifest entry {}", entry.path));
        }
        let kind = match entry.entry_type.as_str() {
            "file" => EntryKind::File,
            "dir" => EntryKind::Dir,
            "symlink" => {
                return Err(anyhow!("symlink entries are not supported"));
            }
            other => {
                return Err(anyhow!("unsupported entry type {other}"));
            }
        };
        parse_mode(&entry.mode)?;
        if entry.mtime < 0 {
            return Err(anyhow!("mtime must be >= 0"));
        }
        match kind {
            EntryKind::File => {
                if entry.size.is_none() || entry.sha256.is_none() {
                    return Err(anyhow!("file entry missing size or sha256"));
                }
            }
            EntryKind::Dir => {
                if entry.size.is_some() || entry.sha256.is_some() {
                    return Err(anyhow!("dir entry must not include size or sha256"));
                }
            }
        }
    }
    Ok(())
}

fn canonical_manifest_hash(manifest: &FixtureManifest) -> Result<String> {
    let mut normalized = manifest.clone();
    normalized.entries.sort_by(|a, b| a.path.cmp(&b.path));
    let bytes = serde_json::to_vec(&normalized).context("serialize manifest")?;
    Ok(sha256_hex(&bytes))
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(anyhow!(
                "symlink in fixture tree: {}",
                entry.path().display()
            ));
        }
        let rel = entry.path().strip_prefix(src)?;
        let dest_path = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &dest_path)?;
        } else {
            return Err(anyhow!(
                "unsupported fixture entry: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn apply_manifest(root: &Path, manifest: &FixtureManifest) -> Result<()> {
    for entry in &manifest.entries {
        let target = root.join(&entry.path);
        let mode = parse_mode(&entry.mode)?;
        let mtime = FileTime::from_unix_time(entry.mtime, 0);
        let metadata =
            fs::symlink_metadata(&target).with_context(|| format!("stat {}", target.display()))?;
        if entry.entry_type == "file" && !metadata.is_file() {
            return Err(anyhow!("expected file for {}", entry.path));
        }
        if entry.entry_type == "dir" && !metadata.is_dir() {
            return Err(anyhow!("expected dir for {}", entry.path));
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(mode))
            .with_context(|| format!("set permissions {}", target.display()))?;
        filetime::set_file_times(&target, mtime, mtime)
            .with_context(|| format!("set mtime {}", target.display()))?;
    }
    Ok(())
}

fn verify_fixture_tree(
    root: &Path,
    manifest: &FixtureManifest,
    check_metadata: bool,
) -> Result<()> {
    let actual_kinds = scan_fixture_tree(root)?;
    let (expected, expected_kinds) = manifest_entries(manifest)?;

    for (path, kind) in expected_kinds {
        let actual_kind = actual_kinds
            .get(&path)
            .ok_or_else(|| anyhow!("missing entry {}", path.display()))?;
        if *actual_kind != kind {
            return Err(anyhow!("entry type mismatch {}", path.display()));
        }
    }
    for path in actual_kinds.keys() {
        if !expected.contains_key(path) {
            return Err(anyhow!("unexpected entry {}", path.display()));
        }
    }

    for (path, entry) in expected {
        if entry.entry_type == "file" {
            let target = root.join(&path);
            let metadata = fs::metadata(&target)?;
            let size = metadata.len();
            if let Some(expected_size) = entry.size {
                if size != expected_size {
                    return Err(anyhow!("size mismatch for {}", path.display()));
                }
            }
            let hash = sha256_file(&target)?;
            if let Some(expected_hash) = entry.sha256 {
                if hash != expected_hash {
                    return Err(anyhow!("sha256 mismatch for {}", path.display()));
                }
            }
        }
        if check_metadata {
            let target = root.join(&path);
            let metadata = fs::metadata(&target)?;
            let actual_mode = metadata.permissions().mode() & 0o7777;
            let expected_mode = parse_mode(&entry.mode)?;
            if actual_mode != expected_mode {
                return Err(anyhow!("mode mismatch for {}", path.display()));
            }
            let mtime = FileTime::from_last_modification_time(&metadata).unix_seconds();
            if mtime != entry.mtime {
                return Err(anyhow!("mtime mismatch for {}", path.display()));
            }
        }
    }
    Ok(())
}

fn scan_fixture_tree(root: &Path) -> Result<HashMap<PathBuf, EntryKind>> {
    let mut kinds = HashMap::new();
    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(anyhow!(
                "symlink in fixture tree: {}",
                entry.path().display()
            ));
        }
        let rel = entry.path().strip_prefix(root)?;
        let kind = if entry.file_type().is_dir() {
            EntryKind::Dir
        } else if entry.file_type().is_file() {
            EntryKind::File
        } else {
            return Err(anyhow!(
                "unsupported fixture entry: {}",
                entry.path().display()
            ));
        };
        kinds.insert(rel.to_path_buf(), kind);
    }
    Ok(kinds)
}

fn manifest_entries(
    manifest: &FixtureManifest,
) -> Result<(HashMap<PathBuf, FixtureEntry>, HashMap<PathBuf, EntryKind>)> {
    let mut entries = HashMap::new();
    let mut kinds = HashMap::new();
    for entry in &manifest.entries {
        let path = PathBuf::from(&entry.path);
        let kind = match entry.entry_type.as_str() {
            "file" => EntryKind::File,
            "dir" => EntryKind::Dir,
            _ => return Err(anyhow!("unsupported manifest entry type")),
        };
        entries.insert(path.clone(), entry.clone());
        kinds.insert(path, kind);
    }
    Ok((entries, kinds))
}

fn parse_mode(value: &str) -> Result<u32> {
    u32::from_str_radix(value, 8).map_err(|_| anyhow!("invalid mode {value}"))
}
