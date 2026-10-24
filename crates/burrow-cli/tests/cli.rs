use std::process::{Command, Output};

fn burrow() -> Command {
    Command::new(env!("CARGO_BIN_EXE_burrow"))
}

fn run(args: &[&str]) -> Output {
    burrow()
        .args(args)
        .output()
        .expect("failed to execute burrow")
}

fn assert_not_implemented(output: Output) {
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not implemented yet"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn version_reports_package_version() {
    let output = run(&["--version"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("burrow 0.1.0"), "stdout was:\n{stdout}");
}

#[test]
fn help_lists_the_full_command_surface() {
    let output = run(&["--help"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commands = [
        "init",
        "up",
        "down",
        "restart",
        "status",
        "logs",
        "open",
        "env",
        "exec",
        "doctor",
        "top",
        "pkg",
        "snapshot",
        "restore",
        "snapshots",
        "reset",
        "schema",
    ];

    for command in commands {
        assert!(
            stdout.contains(command),
            "help is missing `{command}`:\n{stdout}"
        );
    }
}

#[test]
fn up_reports_not_implemented() {
    assert_not_implemented(run(&["up"]));
}

#[test]
fn pkg_list_reports_not_implemented() {
    assert_not_implemented(run(&["pkg", "list"]));
}

#[test]
fn env_json_reports_not_implemented() {
    assert_not_implemented(run(&["env", "--format", "json"]));
}
