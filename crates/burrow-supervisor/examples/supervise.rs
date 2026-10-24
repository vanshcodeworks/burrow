//! Manual smoke test for the supervisor.
//!
//! Runs an arbitrary program inside a Job Object, forwards its output, and
//! stops the whole tree on Ctrl+C.
//!
//! ```powershell
//! cargo run -p burrow-supervisor --example supervise -- cmd /C echo hello
//! ```

use std::time::Duration;

use burrow_supervisor::{ManagedProcess, ProcessSpec, ShutdownOutcome};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut arguments = std::env::args_os().skip(1);
    let program = arguments.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: cargo run -p burrow-supervisor --example supervise -- <program> [args...]"
        )
    })?;

    let spec = ProcessSpec::new("demo", program)
        .args(arguments)
        .memory_limit_bytes(512 * 1024 * 1024)
        .grace_period(Duration::from_secs(5));

    let mut process = ManagedProcess::spawn(spec).await?;
    println!(
        "started `{}` with pid {}; press Ctrl+C to stop it",
        process.name(),
        process.pid()
    );

    tokio::select! {
        status = process.wait() => {
            let status = status?;
            println!("process exited: {status:?}");
            let outcome = process.shutdown(Some(Duration::ZERO)).await?;
            println!("cleanup: {outcome:?}");
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            println!("Ctrl+C received; shutting down the process tree");
            let outcome = process.shutdown(None).await?;
            if let ShutdownOutcome::Forced {
                graceful_signal_sent: false,
                ..
            } = &outcome
            {
                println!("note: no console was available, so the tree was terminated directly");
            }
            println!("shutdown outcome: {outcome:?}");
        }
    }

    Ok(())
}
