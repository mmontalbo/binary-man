//! Resource limit setup for the child process.

use std::io;

use crate::scenario::ScenarioLimits;

/// Configure rlimits and session isolation for the child process.
pub(crate) fn configure_child(limits: ScenarioLimits) -> io::Result<()> {
    if unsafe { libc::setsid() } == -1 {
        return Err(io::Error::last_os_error());
    }
    let cpu_secs = limits.cpu_time_ms.div_ceil(1000);
    set_rlimit(libc::RLIMIT_CPU, cpu_secs, cpu_secs)?;
    set_rlimit(
        libc::RLIMIT_AS,
        limits.memory_kb.saturating_mul(1024),
        limits.memory_kb.saturating_mul(1024),
    )?;
    set_rlimit(
        libc::RLIMIT_FSIZE,
        limits.file_size_kb.saturating_mul(1024),
        limits.file_size_kb.saturating_mul(1024),
    )?;
    set_rlimit(libc::RLIMIT_NOFILE, 128, 128)?;
    Ok(())
}

fn set_rlimit(resource: libc::__rlimit_resource_t, cur: u64, max: u64) -> io::Result<()> {
    let lim = libc::rlimit {
        rlim_cur: cur as libc::rlim_t,
        rlim_max: max as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(resource, &lim) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
