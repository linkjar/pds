//! The binary itself: the exit codes of its commands, and `serve` from the
//! listener to the signal that stops it.
//!
//! Each child runs with a cleared environment and the variables of its case,
//! so the environment of the developer does not change a result.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{ADMIN_PASSWORD, ALICE, DataDir, JWT_SECRET};
use linkjar_pds::serve::{app_state, serve_until};

fn command(vars: &[(&str, &str)], args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_linkjar-pds"));
    command.env_clear().envs(vars.iter().copied()).args(args);
    command
}

fn run(vars: &[(&str, &str)], args: &[&str]) -> Output {
    command(vars, args).output().unwrap()
}

fn exit(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The variables that point a command at `dir`.
fn data_vars(dir: &DataDir) -> Vec<(&'static str, String)> {
    vec![
        ("PDS_HOSTNAME", "pds.test".to_owned()),
        ("PDS_DATA_DIRECTORY", dir.root.to_str().unwrap().to_owned()),
        (
            "PDS_BLOBSTORE_DISK_LOCATION",
            dir.root.join("blocks").to_str().unwrap().to_owned(),
        ),
        ("PDS_JWT_SECRET", JWT_SECRET.to_owned()),
        ("PDS_ADMIN_PASSWORD", ADMIN_PASSWORD.to_owned()),
        ("PDS_DID_PLC_URL", "http://127.0.0.1:9".to_owned()),
    ]
}

fn borrowed<'a>(vars: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    vars.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// A port that nothing listens on now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_command_line_that_is_not_one_exits_2() {
    for args in [
        &[][..],
        &["frobnicate"],
        &["serve", "now"],
        &["check-config", "now"],
        &["health", "now"],
        &["verify", "--yaml"],
        &["verify", "--json", "--typo"],
        &["export-car"],
        &["export-car", ALICE, "out.car"],
    ] {
        assert_eq!(exit(&run(&[], args)), 2, "{args:?}");
    }
    assert_eq!(exit(&run(&[], &["version"])), 0);
    assert_eq!(exit(&run(&[], &["help"])), 0);
}

#[test]
fn check_config_needs_the_secrets_and_a_valid_value() {
    let secrets = [("PDS_JWT_SECRET", "a"), ("PDS_ADMIN_PASSWORD", "b")];
    let valid = run(&secrets, &["check-config"]);
    assert_eq!(exit(&valid), 0);
    assert_eq!(
        String::from_utf8_lossy(&valid.stdout),
        "the configuration is valid\n"
    );
    assert_eq!(stderr(&valid), "");

    let missing = run(&[("PDS_JWT_SECRET", "a")], &["check-config"]);
    assert_eq!(exit(&missing), 2);
    assert!(stderr(&missing).contains("PDS_ADMIN_PASSWORD is required"));

    let invalid = run(
        &[secrets[0], secrets[1], ("PDS_PORT", "70000")],
        &["check-config"],
    );
    assert_eq!(exit(&invalid), 2);
}

#[test]
fn check_config_warns_on_an_unknown_variable_and_refuses_an_entryway_one() {
    let secrets = [("PDS_JWT_SECRET", "a"), ("PDS_ADMIN_PASSWORD", "b")];
    let unknown = run(
        &[secrets[0], secrets[1], ("PDS_JWT_SECERT", "c")],
        &["check-config"],
    );
    assert_eq!(exit(&unknown), 0);
    assert!(
        stderr(&unknown).contains("warning: unknown variable PDS_JWT_SECERT"),
        "{}",
        stderr(&unknown)
    );

    let refused = run(
        &[
            secrets[0],
            secrets[1],
            ("PDS_ENTRYWAY_URL", "https://e.test"),
        ],
        &["check-config"],
    );
    assert_eq!(exit(&refused), 2);
    assert!(stderr(&refused).contains("PDS_ENTRYWAY_URL"));
}

#[test]
fn verify_exits_0_when_clean_1_with_a_difference_and_2_when_it_cannot_run() {
    let dir = DataDir::new(3);
    let vars = data_vars(&dir);
    let vars = borrowed(&vars);

    let clean = run(&vars, &["verify", "--json"]);
    assert_eq!(exit(&clean), 0, "{}", stderr(&clean));
    let report: serde_json::Value = serde_json::from_slice(&clean.stdout).unwrap();
    assert_eq!(report["differences"], 0);
    assert_eq!(report["actors"], 3);

    // A break in the event log is reported and is not a difference.
    common::open(&dir.config.layout.sequencer_db)
        .execute(r#"delete from "repo_seq" where "seq" = 2"#, [])
        .unwrap();
    let broken = run(&vars, &["verify"]);
    assert_eq!(exit(&broken), 0, "{}", stderr(&broken));
    assert!(String::from_utf8_lossy(&broken.stdout).contains("sequence breaks   1\n"));

    std::fs::remove_file(dir.config.layout.actor_key(ALICE)).unwrap();
    let damaged = run(&vars, &["verify"]);
    assert_eq!(exit(&damaged), 1);
    assert!(String::from_utf8_lossy(&damaged.stdout).contains("key-unreadable"));

    std::fs::remove_file(&dir.config.layout.account_db).unwrap();
    assert_eq!(exit(&run(&vars, &["verify"])), 2);
}

#[test]
fn export_car_refuses_an_argument_that_is_not_a_did() {
    let dir = DataDir::new(3);
    let vars = data_vars(&dir);
    let vars = borrowed(&vars);
    let refused = run(&vars, &["export-car", "../../etc"]);
    assert_eq!(exit(&refused), 2);
    assert_eq!(refused.stdout, b"");

    let exported = run(&vars, &["export-car", ALICE]);
    assert_eq!(exit(&exported), 0, "{}", stderr(&exported));
    assert_ne!(exported.stdout, b"");
}

#[test]
fn health_exits_1_when_nothing_answers_and_when_the_configuration_is_invalid() {
    let port = free_port().to_string();
    let silent = run(&[("PDS_PORT", &port)], &["health"]);
    assert_eq!(exit(&silent), 1, "{}", stderr(&silent));
    assert!(stderr(&silent).contains("health"));

    assert_eq!(exit(&run(&[("PDS_PORT", "70000")], &["health"])), 1);
}

/// `serve` listens, `health` exits 0 against it, and `SIGTERM` stops it with
/// exit 0.
#[cfg(unix)]
#[test]
fn serve_answers_health_and_stops_on_sigterm() {
    let dir = DataDir::new(3);
    let port = free_port().to_string();
    let mut vars = data_vars(&dir);
    vars.push(("PDS_PORT", port));
    let vars = borrowed(&vars);
    let mut server = command(&vars, &["serve"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let started = Instant::now();
    while exit(&run(&vars, &["health"])) != 0 {
        assert!(
            server.try_wait().unwrap().is_none(),
            "the server exited before it was healthy"
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the server did not become healthy"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let killed = Command::new("kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let stopped = Instant::now();
    let status = loop {
        if let Some(status) = server.try_wait().unwrap() {
            break status;
        }
        if stopped.elapsed() > Duration::from_secs(20) {
            server.kill().unwrap();
            panic!("the server did not stop on SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    assert_eq!(exit(&run(&vars, &["health"])), 1);
}

/// A connection that never finishes its request does not hold the server
/// past the limit, and the limit starts at the signal.
#[tokio::test]
async fn shutdown_ends_at_the_limit_with_a_connection_open() {
    use tokio::io::AsyncWriteExt;

    let dir = DataDir::new(1);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let limit = Duration::from_millis(300);
    let server = tokio::spawn(serve_until(
        listener,
        app_state(&dir.config).unwrap(),
        async {
            let _ = stopped.await;
        },
        limit,
    ));

    let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
    slow.write_all(b"GET /xrpc/_health HTTP/1.1\r\nhost: pds.test\r\n")
        .await
        .unwrap();
    // Longer than the limit: the limit does not run before the signal.
    tokio::time::sleep(limit * 2).await;
    assert!(!server.is_finished());

    let told = Instant::now();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the server stopped within the limit")
        .unwrap()
        .unwrap();
    assert!(told.elapsed() >= limit, "{:?}", told.elapsed());
    drop(slow);
}

/// With no connection open the server stops at once.
#[tokio::test]
async fn shutdown_does_not_wait_for_the_limit_without_a_connection() {
    let dir = DataDir::new(1);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let served = serve_until(
        listener,
        app_state(&dir.config).unwrap(),
        std::future::ready(()),
        Duration::from_secs(60),
    );
    tokio::time::timeout(Duration::from_secs(10), served)
        .await
        .expect("the server stopped before the limit")
        .unwrap();
}
