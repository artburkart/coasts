//! Integration tests for CLI exit codes.
//!
//! These run the compiled `coast` binary as a subprocess and verify that
//! informational commands (help, version) exit 0 while actual errors exit
//! non-zero.

use assert_cmd::{cargo_bin, Command};
use predicates::prelude::*;

fn coast_cmd() -> Command {
    Command::new(cargo_bin!("coast"))
}

// ---------------------------------------------------------------------------
// Top-level help and version — must exit 0
// ---------------------------------------------------------------------------

#[test]
fn help_flag_exits_zero() {
    coast_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Manage isolated development environments",
        ));
}

#[test]
fn version_flag_exits_zero() {
    coast_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("coast"));
}

#[test]
fn short_help_flag_exits_zero() {
    coast_cmd()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Manage isolated development environments",
        ));
}

#[test]
fn short_version_flag_exits_zero() {
    coast_cmd()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("coast"));
}

// ---------------------------------------------------------------------------
// Subcommand help — must exit 0
// ---------------------------------------------------------------------------

#[test]
fn docs_help_exits_zero() {
    coast_cmd()
        .args(["docs", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docs"));
}

#[test]
fn search_docs_help_exits_zero() {
    coast_cmd()
        .args(["search-docs", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("search"));
}

#[test]
fn build_help_exits_zero() {
    coast_cmd().args(["build", "--help"]).assert().success();
}

#[test]
fn ls_help_exits_zero() {
    coast_cmd().args(["ls", "--help"]).assert().success();
}

#[test]
fn run_help_exits_zero() {
    coast_cmd().args(["run", "--help"]).assert().success();
}

#[test]
fn lookup_help_exits_zero() {
    coast_cmd().args(["lookup", "--help"]).assert().success();
}

#[test]
fn daemon_help_exits_zero() {
    coast_cmd().args(["daemon", "--help"]).assert().success();
}

#[test]
fn exec_help_exits_zero() {
    coast_cmd().args(["exec", "--help"]).assert().success();
}

// ---------------------------------------------------------------------------
// Actual parse errors — must exit 2 (clap convention)
// ---------------------------------------------------------------------------

#[test]
fn unknown_flag_exits_two() {
    coast_cmd()
        .arg("--bogus-flag")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error"));
}

#[test]
fn missing_required_arg_exits_two() {
    // `coast run` requires a positional <NAME> argument
    coast_cmd().arg("run").assert().code(2);
}

#[test]
fn unknown_subcommand_exits_two() {
    coast_cmd().arg("nonexistent-subcommand").assert().code(2);
}

// ---------------------------------------------------------------------------
// No subcommand — shows help and exits 0 (not an error)
// ---------------------------------------------------------------------------

#[test]
fn no_args_shows_help_exits_zero() {
    coast_cmd().assert().success();
}
