//! Container fallback: `use = "container:IMAGE"` services.
//!
//! Burrow is not a container runtime. When a service has no trustworthy
//! native Windows build, this module delegates to a container engine the
//! user installed (Docker or Podman), runs the container as a loopback-only
//! child of the supervisor, and labels it clearly in `burrow status`.
//!
//! Limitations, documented in `docs/config-reference.md` and `docs/security.md`:
//!
//! * The container filesystem is ephemeral (`--rm`): `burrow snapshot` does
//!   not cover container services.
//! * A crash of the daemon can leave the container running, because a Job
//!   Object only guarantees the death of the engine's client process. The
//!   next start removes a leftover container with `rm -f` before running a
//!   new one.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};

/// A discovered container engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEngine {
    /// Absolute engine executable, for example `docker.exe`.
    pub program: PathBuf,
    /// Lower-case engine name used in messages, for example `docker`.
    pub display: String,
}

/// Engine names tried in order when `BURROW_CONTAINER_ENGINE` is unset.
const ENGINE_ORDER: [&str; 2] = ["docker", "podman"];

/// Finds a container engine on `PATH` (or via `BURROW_CONTAINER_ENGINE`).
pub fn discover() -> Option<ContainerEngine> {
    let candidates: Vec<String> = match std::env::var("BURROW_CONTAINER_ENGINE") {
        Ok(name) if !name.trim().is_empty() => vec![name.trim().to_owned()],
        _ => ENGINE_ORDER.iter().map(ToString::to_string).collect(),
    };

    let fallback = PathBuf::from(".");
    for name in candidates {
        if let Ok(program) = crate::win::resolve_executable(
            OsString::from(&name).as_os_str(),
            &fallback,
            std::env::var_os("PATH").as_deref(),
        ) {
            return Some(ContainerEngine {
                program,
                display: name.to_ascii_lowercase(),
            });
        }
    }
    None
}

/// Checks that the engine binary answers `version`.
///
/// This catches "installed but the daemon/machine is not running" before a
/// project starts, when the failure is still cheap to explain.
pub async fn engine_ready(engine: &ContainerEngine) -> Result<()> {
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(&engine.program)
            .arg("version")
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .context("the container engine did not answer `version` in time")?
    .with_context(|| format!("could not run `{}`", engine.program.display()))?;

    if output.status.success() {
        return Ok(());
    }
    let tail = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "the container engine `{}` is installed but not responding\n\
         help: start the engine (for example Docker Desktop), then retry\n{}",
        engine.display,
        tail.lines().take(3).collect::<Vec<_>>().join("\n")
    )
}

/// Builds the argument list for `docker run` / `podman run`.
///
/// The port mapping is loopback-only. `env` entries are passed with `-e`;
/// the caller picks which variables make sense inside the container.
pub fn run_args(
    container_name: &str,
    image: &str,
    host_port: u16,
    container_port: u16,
    env: &BTreeMap<String, String>,
    memory_bytes: Option<u64>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        container_name.into(),
        "-p".into(),
        format!("127.0.0.1:{host_port}:{container_port}").into(),
    ];
    if let Some(bytes) = memory_bytes {
        args.push("--memory".into());
        args.push(bytes.to_string().into());
    }
    for (key, value) in env {
        args.push("-e".into());
        args.push(format!("{key}={value}").into());
    }
    args.push(image.into());
    args
}

/// Arguments that stop a container gracefully.
pub fn stop_args(container_name: &str, grace_seconds: u64) -> Vec<OsString> {
    vec![
        "stop".into(),
        "--time".into(),
        grace_seconds.to_string().into(),
        container_name.into(),
    ]
}

/// Arguments that remove a leftover container from a crashed run.
pub fn remove_args(container_name: &str) -> Vec<OsString> {
    vec!["rm".into(), "-f".into(), container_name.into()]
}

/// Container name used for one service of one project.
///
/// Project and service names are validated as `[a-z0-9-]`, which keeps the
/// result a legal container name.
pub fn container_name(project: &str, service: &str) -> String {
    format!("burrow-{project}-{service}")
}

#[cfg(test)]
mod tests {
    use super::{container_name, run_args, stop_args};

    #[test]
    fn run_args_map_the_port_to_loopback_and_pass_env_in_order() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("PORT".to_owned(), "8080".to_owned());
        env.insert("Z-last".to_owned(), "value with spaces".to_owned());

        let args = run_args(
            "burrow-shop-cache",
            "redis:7",
            45123,
            6379,
            &env,
            Some(268_435_456),
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = rendered.join(" ");
        assert_eq!(rendered[0], "run");
        assert!(joined.contains("--name burrow-shop-cache"), "{joined}");
        assert!(joined.contains("-p 127.0.0.1:45123:6379"), "{joined}");
        assert!(joined.contains("--memory 268435456"), "{joined}");
        assert!(joined.contains("-e PORT=8080"), "{joined}");
        // BTreeMap order keeps the command deterministic.
        let port_index = joined.find("-e PORT").expect("PORT passed");
        let last_index = joined.find("-e Z-last").expect("Z-last passed");
        assert!(port_index < last_index);
        assert!(joined.ends_with("redis:7"), "{joined}");
    }

    #[test]
    fn run_args_omit_the_memory_flag_when_not_configured() {
        let args = run_args(
            "burrow-shop-cache",
            "redis:7",
            45123,
            6379,
            &Default::default(),
            None,
        );
        let joined = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!joined.contains("--memory"), "{joined}");
    }

    #[test]
    fn stop_args_carry_the_grace_period() {
        let args = stop_args("burrow-shop-cache", 10);
        let joined = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(joined, "stop --time 10 burrow-shop-cache");
    }

    #[test]
    fn container_names_combine_project_and_service() {
        assert_eq!(container_name("shop", "cache"), "burrow-shop-cache");
    }
}
