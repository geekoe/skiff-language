#![allow(dead_code)]

use std::{
    ffi::OsStr,
    path::Path,
    process::{Command, Output},
};

pub struct CliCommand {
    command: Command,
}

impl CliCommand {
    pub fn new() -> Self {
        Self {
            command: Command::new(env!("CARGO_BIN_EXE_skiff-compiler")),
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.command.arg(arg);
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub fn compile(input: impl AsRef<Path>) -> Self {
        Self::new().arg(input.as_ref())
    }

    pub fn test(input: impl AsRef<Path>) -> Self {
        Self::new().arg("--test").arg(input.as_ref())
    }

    pub fn output(mut self) -> Output {
        self.command.output().unwrap()
    }
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
