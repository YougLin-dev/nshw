//! The public CLI surfaces, exercised through the built binary.

use assert_cmd::Command;
use predicates::prelude::*;

fn nshw() -> Command {
    Command::cargo_bin("nshw").expect("the nshw binary")
}

#[test]
fn reports_the_selector_version() {
    nshw()
        .args(["toolchain", "version"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "nshw {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn toolchain_accepts_help_flags() {
    for flag in ["help", "--help", "-h"] {
        nshw()
            .args(["toolchain", flag])
            .assert()
            .success()
            .stdout(predicate::str::contains("nshw toolchain status"));
    }
}

#[test]
fn an_unknown_administration_command_explains_itself() {
    nshw()
        .args(["toolchain", "frobnicate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unknown command frobnicate; try nshw toolchain help",
        ));
}

#[test]
fn status_outside_an_app_reports_the_missing_manifest() {
    let dir = tempfile::tempdir().expect("temp dir");
    nshw()
        .current_dir(dir.path())
        .args(["toolchain", "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no nsh.json found"));
}

#[cfg(unix)]
#[test]
fn outside_an_app_forwards_to_the_installed_fallback() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let fallback = dir.path().join("nsh");
    std::fs::write(&fallback, "#!/bin/sh\nprintf 'fallback:%s\\n' \"$*\"\n").expect("script");
    let mut permissions = std::fs::metadata(&fallback)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fallback, permissions).expect("chmod");

    nshw()
        .current_dir(dir.path())
        .env("NSH_FALLBACK", &fallback)
        .args(["help"])
        .assert()
        .success()
        .stdout("fallback:help\n");
}
