//! Fixtures for `mutate.py --e2e` (note 51 §6.4): each is a test that returns at once unless
//! `EZSDR_INFRA_FIXTURE` names it, so the default suite runs them as no-ops. The e2e driver
//! runs one at a time through the same `cargo_test()` a mutation row uses and checks the
//! verdict the runner draws from the output. They are not `#[ignore]`: that is the
//! hardware tests' mark.

use crate::common::*;
use ezsdr_radio_uhd::{Device, FakeConfig};
use serde_json::json;
use std::time::Duration as Wall;

fn selected(name: &str) -> bool {
    std::env::var("EZSDR_INFRA_FIXTURE").is_ok_and(|selected| selected == name)
}

/// 1: an assertion fails, with no marker.
#[test]
fn infra_fixture_assert_failure() {
    if selected("assert_failure") {
        assert_eq!(1 + 1, 3, "the fixture's own assertion");
    }
}

/// 2: the clock never moves, so the test never ends and the watchdog (`EZSDR_TEST_WATCHDOG_SECS=1`) aborts with its marker.
#[test]
fn infra_fixture_stopped_clock() {
    if selected("stopped_clock") {
        let device = fake(FakeConfig::default());
        device.host().sleep(Wall::from_secs(3600));
    }
}

/// 3: the Session is created on a device whose `prepare` takes longer than KC-12a's 5 s, so `connect` returns a Run that has ended.
#[test]
fn infra_fixture_creation_limit() {
    if selected("creation_limit") {
        let dir = TempDir::new();
        let device = fake_real(FakeConfig { apply_delay: Wall::from_secs(8), ..FakeConfig::default() });
        let _run = session(&profile(&dir, json!({}), json!({}), true), device);
        unreachable!("the Session cannot be created");
    }
}

/// 3b: the same limit on a Spec's Run, which `spec_run` (through `start_spec`) classifies instead of `session_on`.
#[test]
fn infra_fixture_creation_limit_spec() {
    if selected("creation_limit_spec") {
        let dir = TempDir::new();
        let device = fake_real(FakeConfig { apply_delay: Wall::from_secs(8), ..FakeConfig::default() });
        let _run = spec_run(&receive_spec(1, 1e6, 1e9, None), &profile(&dir, json!({}), json!({}), false), device, Default::default());
        unreachable!("the Spec's Run cannot be created");
    }
}

/// 5: the process aborts with no marker.
#[test]
fn infra_fixture_abort_without_marker() {
    if selected("abort_without_marker") {
        std::process::abort();
    }
}

/// A timing precondition missed on every run.
#[test]
fn infra_fixture_timing_always() {
    if selected("timing_always") {
        timing("the fixture always misses its timing precondition");
        panic!("timing precondition missed");
    }
}

/// A timing precondition missed on the first run only: the state directory the driver passes (`EZSDR_INFRA_FIXTURE_STATE`) remembers it.
#[test]
fn infra_fixture_timing_once() {
    if selected("timing_once") {
        let seen = std::path::Path::new(&std::env::var("EZSDR_INFRA_FIXTURE_STATE").expect("the driver passes a state directory")).join("seen");
        if !seen.exists() {
            std::fs::write(&seen, b"").unwrap();
            timing("the fixture misses its timing precondition once");
            panic!("timing precondition missed");
        }
    }
}
