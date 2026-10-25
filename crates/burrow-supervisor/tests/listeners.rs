//! Listener inspection tests.
//!
//! These bind real sockets in the test process and ask the IP Helper wrapper
//! to find them. The wildcard case is the one `doctor` warns about: a service
//! that listens on `0.0.0.0` without `expose = true`.

use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn listening_sockets_finds_loopback_and_wildcard_binds() {
    let loopback = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let loopback_port = loopback.local_addr().expect("loopback address").port();
    let wildcard = TcpListener::bind(("0.0.0.0", 0)).expect("bind wildcard");
    let wildcard_port = wildcard.local_addr().expect("wildcard address").port();

    let listeners =
        burrow_supervisor::listening_sockets(&[std::process::id()]).expect("inspect listeners");

    let loopback = listeners
        .iter()
        .find(|listener| listener.port == loopback_port)
        .expect("loopback listener was not reported");
    assert_eq!(loopback.address, "127.0.0.1");
    assert!(!loopback.wildcard);
    assert_eq!(loopback.root_pid, std::process::id());

    let wildcard = listeners
        .iter()
        .find(|listener| listener.port == wildcard_port)
        .expect("wildcard listener was not reported");
    assert_eq!(wildcard.address, "0.0.0.0");
    assert!(wildcard.wildcard);
    assert_eq!(wildcard.root_pid, std::process::id());

    // IPv6 can be disabled on a machine; only assert when the bind works.
    if let Ok(v6) = TcpListener::bind(("::1", 0)) {
        let port = v6.local_addr().expect("IPv6 address").port();
        let listeners = burrow_supervisor::listening_sockets(&[std::process::id()])
            .expect("inspect IPv6 listeners");
        let listener = listeners
            .iter()
            .find(|listener| listener.port == port)
            .expect("IPv6 loopback listener was not reported");
        assert_eq!(listener.address, "::1");
        assert!(!listener.wildcard);
    }

    assert!(
        burrow_supervisor::listening_sockets(&[])
            .expect("empty roots")
            .is_empty(),
        "no roots must inspect nothing"
    );
}

#[test]
fn listening_sockets_attributes_a_child_listener_to_the_supervised_root() {
    let probe = TcpListener::bind(("127.0.0.1", 0)).expect("reserve a port");
    let port = probe.local_addr().expect("probe address").port();
    drop(probe);

    let testbin = env!("CARGO_BIN_EXE_burrow-testbin");
    let mut host = Command::new(testbin)
        .args(["host", testbin, "serve", &port.to_string()])
        .spawn()
        .expect("spawn testbin host");
    let root = host.id();

    let started = Instant::now();
    let listener = loop {
        let listeners =
            burrow_supervisor::listening_sockets(&[root]).expect("inspect child listeners");
        if let Some(listener) = listeners.iter().find(|listener| listener.port == port) {
            break listener.clone();
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "child listener was not reported"
        );
        std::thread::sleep(Duration::from_millis(100));
    };

    assert_ne!(listener.pid, root, "listener must belong to the child");
    assert_eq!(
        listener.root_pid, root,
        "child listener must be attributed to the supervised root"
    );

    host.kill().expect("kill host");
    let _ = host.wait();

    // Killing the host closes the job handle; KILL_ON_JOB_CLOSE must take the
    // child down with it.
    let stopped = Instant::now();
    loop {
        let listeners = burrow_supervisor::listening_sockets(&[root]).expect("inspect after kill");
        if !listeners.iter().any(|listener| listener.port == port) {
            break;
        }
        assert!(
            stopped.elapsed() < Duration::from_secs(10),
            "child listener survived the host kill"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
