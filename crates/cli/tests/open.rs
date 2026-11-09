#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `termy <path>` must never launch a `target/*/termy` found relative to the
/// current directory: running the CLI inside an untrusted checkout would
/// execute that checkout's binary.
#[test]
fn open_ignores_app_binaries_in_current_directory() {
    let temp = tempfile::tempdir().unwrap();

    // Copy the CLI somewhere without a sibling app binary.
    let cli_dir = temp.path().join("cli");
    std::fs::create_dir_all(&cli_dir).unwrap();
    let cli = cli_dir.join("termy-cli");
    std::fs::copy(env!("CARGO_BIN_EXE_termy-cli"), &cli).unwrap();

    let checkout = temp.path().join("checkout");
    let marker = temp.path().join("executed");
    for profile in ["debug", "release"] {
        let dir = checkout.join("target").join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("termy");
        std::fs::write(&fake, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = Command::new(&cli)
        .arg(".")
        .current_dir(&checkout)
        .stdin(Stdio::null())
        .output()
        .unwrap();

    // The launch is detached, so give a wrongly spawned child time to run.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && !marker.exists() {
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(!marker.exists(), "CLI executed a cwd-relative app binary");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Termy app binary not found"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
