//! Execution paths for scenarios (direct or sandboxed).

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::contract::{apply_env_contract, ENV_LC_ALL, ENV_PATH, ENV_TERM, ENV_TZ};
use crate::limits::configure_child;
use crate::scenario::ScenarioLimits;

/// Output captured from a single scenario execution.
pub(crate) struct RunResult {
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) wall_time_ms: u64,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// Execute the target binary directly on the host (debug mode).
///
/// The `binary` path is used as argv[0] to preserve multi-call behavior.
pub(crate) fn run_direct(
    binary: &Path,
    args: &[String],
    cwd: &Path,
    limits: ScenarioLimits,
) -> Result<RunResult> {
    let mut command = Command::new(binary);
    command.args(args);
    command.current_dir(cwd);
    apply_env_contract(&mut command);
    run_command(command, limits)
}

/// Execute the target binary inside a rootless bwrap sandbox.
///
/// `exec_binary` preserves argv[0] semantics, while `binary_source` is copied
/// into the sandbox to provide the executable bytes.
pub(crate) fn run_sandboxed(
    exec_binary: &Path,
    binary_source: &Path,
    args: &[String],
    fixture_root: &Path,
    limits: ScenarioLimits,
) -> Result<RunResult> {
    if !Path::new("/nix/store").exists() {
        return Err(anyhow!("expected /nix/store for sandbox mounts"));
    }

    let bwrap = Path::new("bwrap");
    let binary_name = exec_binary
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("binary");

    let run_root = fixture_root
        .parent()
        .ok_or_else(|| anyhow!("fixture root has no parent"))?;
    let bin_root = run_root.join("bin");
    fs::create_dir_all(&bin_root).context("create bin dir")?;
    let sandbox_binary = bin_root.join(binary_name);
    fs::copy(binary_source, &sandbox_binary).context("copy binary into run root")?;
    let metadata = fs::metadata(binary_source).context("stat binary for permissions")?;
    fs::set_permissions(&sandbox_binary, metadata.permissions())
        .context("apply binary permissions")?;

    let mut command = Command::new(bwrap);
    command.arg("--die-with-parent");
    command.arg("--unshare-net");
    command.arg("--tmpfs");
    command.arg("/");
    command.arg("--dir");
    command.arg("/proc");
    command.arg("--dir");
    command.arg("/dev");
    command.arg("--dir");
    command.arg("/tmp");
    command.arg("--dir");
    command.arg("/bin");
    command.arg("--dir");
    command.arg("/work");
    command.arg("--dir");
    command.arg("/nix");
    command.arg("--dir");
    command.arg("/nix/store");
    command.arg("--proc");
    command.arg("/proc");
    command.arg("--dev");
    command.arg("/dev");
    command.arg("--tmpfs");
    command.arg("/tmp");
    command.arg("--ro-bind").arg("/nix/store").arg("/nix/store");
    command.arg("--ro-bind").arg(&bin_root).arg("/bin");
    command.arg("--bind").arg(fixture_root).arg("/work");
    command.arg("--chdir");
    command.arg("/work");
    command.arg("--clearenv");
    command.arg("--setenv");
    command.arg("LC_ALL");
    command.arg(ENV_LC_ALL);
    command.arg("--setenv");
    command.arg("TZ");
    command.arg(ENV_TZ);
    command.arg("--setenv");
    command.arg("TERM");
    command.arg(ENV_TERM);
    command.arg("--setenv");
    command.arg("PATH");
    command.arg(ENV_PATH);
    command.arg("--");
    command.arg(format!("/bin/{binary_name}"));
    command.args(args);

    run_command(command, limits)
}

fn run_command(mut command: Command, limits: ScenarioLimits) -> Result<RunResult> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let limits_copy = limits;
    unsafe {
        command.pre_exec(move || configure_child(limits_copy));
    }

    let mut child = command.spawn().context("spawn command")?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("stdout not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("stderr not captured"))?;

    let stdout_handle = thread::spawn(move || read_all(stdout));
    let stderr_handle = thread::spawn(move || read_all(stderr));

    let timeout = Duration::from_millis(limits.wall_time_ms);
    let start = Instant::now();
    let mut timed_out = false;
    let exit_status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout {
            timed_out = true;
            kill_process_group(pid);
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(5));
    };

    let wall_time_ms = start.elapsed().as_millis() as u64;
    let stdout = stdout_handle.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    let stderr = stderr_handle.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    let exit_code = exit_status.code();

    Ok(RunResult {
        exit_code,
        timed_out,
        wall_time_ms,
        stdout,
        stderr,
    })
}

fn kill_process_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn read_all(mut reader: impl io::Read) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    Ok(buf)
}
