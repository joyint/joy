// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

#[test]
fn cli_tests() {
    trycmd::TestCases::new().case("tests/cmd/*.toml");
}

#[test]
fn version_matches_cargo_toml() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_joy"))
        .arg("--version")
        .output()
        .expect("failed to run joy --version");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = format!("joy {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        stdout, expected,
        "joy --version must match Cargo.toml version"
    );
}
