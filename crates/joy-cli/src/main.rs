// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

// Nothing of the developer's shell and session reaches the unit tests
// of this crate (JOY-02BB-C7).
#[cfg(test)]
joy_test_env::isolate!();

fn main() -> std::process::ExitCode {
    let result = joy_cli::cli_main();
    if let Err(error) = &result {
        // word for word what Rust says for a `main` that returns the error
        eprintln!("Error: {error:?}");
    }
    // a hint that must be seen comes after the output, the error included
    joy_cli::say_what_waits();
    if result.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
