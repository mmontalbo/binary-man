//! Environment contract applied to every scenario execution.

use serde::Serialize;
use std::process::Command;

/// `LC_ALL` value enforced for deterministic output.
pub(crate) const ENV_LC_ALL: &str = "C";
/// `TZ` value enforced for deterministic timestamps.
pub(crate) const ENV_TZ: &str = "UTC";
/// `TERM` value enforced for non-interactive output.
pub(crate) const ENV_TERM: &str = "dumb";
/// Minimal `PATH` exposed inside the sandbox.
pub(crate) const ENV_PATH: &str = "/bin:/usr/bin";

/// Environment contract recorded in evidence metadata.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct EnvContract {
    #[serde(rename = "LC_ALL")]
    pub(crate) lc_all: String,
    #[serde(rename = "TZ")]
    pub(crate) tz: String,
    #[serde(rename = "TERM")]
    pub(crate) term: String,
}

/// Return the canonical environment contract for metadata.
pub(crate) fn env_contract() -> EnvContract {
    EnvContract {
        lc_all: ENV_LC_ALL.to_string(),
        tz: ENV_TZ.to_string(),
        term: ENV_TERM.to_string(),
    }
}

/// Apply the environment contract to a command (clears existing env first).
pub(crate) fn apply_env_contract(command: &mut Command) {
    command.env_clear();
    command.env("LC_ALL", ENV_LC_ALL);
    command.env("TZ", ENV_TZ);
    command.env("TERM", ENV_TERM);
    command.env("PATH", ENV_PATH);
}
