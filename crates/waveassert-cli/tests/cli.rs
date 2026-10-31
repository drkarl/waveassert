//! Smoke tests for the CLI crate: the binary runs, reports its version and
//! prints usage for anything else.

use std::process::Command;

#[test]
fn version_flag_prints_the_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_waveassert"))
        .arg("--version")
        .output()
        .expect("failed to run the waveassert binary");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("waveassert ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_arguments_prints_usage_and_exits_with_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_waveassert"))
        .output()
        .expect("failed to run the waveassert binary");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not implemented yet"), "stderr: {stderr}");
    assert!(
        stderr.contains("Usage: waveassert --version"),
        "stderr: {stderr}"
    );
}
