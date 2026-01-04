use std::path::{Path, PathBuf};
use std::process::Command;

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn ls_help_available(ls_path: &Path) -> bool {
    ["--help", "-h"].iter().any(|arg| {
        let output = match Command::new(ls_path)
            .arg(arg)
            .env_clear()
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .env("TERM", "dumb")
            .output()
        {
            Ok(output) => output,
            Err(_) => return false,
        };
        !output.stdout.is_empty() || !output.stderr.is_empty()
    })
}

fn find_surface_json(out_dir: &Path) -> Option<PathBuf> {
    let surface_root = out_dir.join("surface");
    let entries = std::fs::read_dir(surface_root).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("surface.json");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[test]
fn surface_extracts_t0_t1_with_planner() {
    let Some(ls_path) = find_in_path("ls") else {
        return;
    };
    if !ls_help_available(&ls_path) {
        return;
    }

    let bin = env!("CARGO_BIN_EXE_binary-man");
    let planner = env!("CARGO_BIN_EXE_planner_stub");
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let out_dir = temp_dir.path().join("out");

    let status = Command::new(bin)
        .arg("surface")
        .arg(&ls_path)
        .arg("--out-dir")
        .arg(&out_dir)
        .env("BVM_PLANNER_CMD", planner)
        .status()
        .expect("run surface");
    assert!(status.success());

    let surface_path = find_surface_json(&out_dir).expect("surface.json path");
    let content = std::fs::read_to_string(&surface_path).expect("read surface report");
    let report: serde_json::Value = serde_json::from_str(&content).expect("parse surface report");

    let options = report
        .get("options")
        .and_then(|value| value.as_array())
        .expect("options array");
    assert!(!options.is_empty());

    let higher = report.get("higher_tiers").expect("higher tiers");
    assert_eq!(
        higher.get("t2").and_then(|value| value.as_str()),
        Some("not_evaluated")
    );
    assert_eq!(
        higher.get("t3").and_then(|value| value.as_str()),
        Some("not_evaluated")
    );
    assert_eq!(
        higher.get("t4").and_then(|value| value.as_str()),
        Some("not_evaluated")
    );

    let first = &options[0];
    assert!(first.get("existence").is_some());
    assert!(first.get("binding").is_some());
}
