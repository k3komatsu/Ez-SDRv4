//! Markers a test writes for the runners (note 51 §6.3): `plan/maintenance/tools/mutate.py` and
//! `gate.py` read them from the output of `cargo test` to tell a bound that expired in real
//! time from an assertion that failed. Shared like `watchdog.rs`: `crates/ezsdr-acceptance`'s
//! tests include this file with `#[path]`.
//!
//! Both markers are one `std::io::stderr().write_all`, not `eprintln!`: libtest captures
//! `print!`/`eprint!` and a captured line is lost when the process aborts (the watchdog's case),
//! whereas a direct write is neither captured nor buffered. The writer does not panic; the caller does.
#![allow(dead_code)]

use std::io::Write;

fn write_line(line: String) {
    let _ = std::io::stderr().write_all(line.as_bytes());
}

fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

fn current_test() -> String {
    std::thread::current().name().unwrap_or("a test").to_owned()
}

/// `EZSDR-INFRA: <test name>: <msg>`: a real-time bound expired, so the verdict says nothing
/// about the code under test. A mutation row that is killed this way is `ERROR` unless it
/// declares `"kill": "bound"`.
pub fn infra(msg: &str) {
    infra_for(&current_test(), msg);
}

/// [`infra`] for a thread that is not the test's (the watchdog).
pub fn infra_for(test: &str, msg: &str) {
    write_line(format!("EZSDR-INFRA: {}: {}\n", one_line(test), one_line(msg)));
}

/// `EZSDR-TIMING: <package> <test binary> <test path>: <msg>`: the test failed because a
/// timing precondition was missed, and a serial run may succeed. The runners re-run the test
/// alone, at most twice; the line carries what an exact re-run needs.
pub fn timing(msg: &str) {
    write_line(timing_line(env!("CARGO_PKG_NAME"), env!("CARGO_CRATE_NAME"), &current_test(), msg));
}

fn timing_line(package: &str, binary: &str, test: &str, msg: &str) -> String {
    let (test, msg) = (one_line(test), one_line(msg));
    format!("EZSDR-TIMING: {package} {binary} {test}: {msg} (serial re-run: cargo test -p {package} --test {binary} -- --exact --test-threads=1 {test})\n")
}

/// Which real-time bound ended a Run that did not reach `Running` (note 51 §6.2): the Kernel's
/// KC-12a or KC-21a (the reason's prefix), a cleanup step abandoned on its deadline (RS-8a:
/// `timed_out` in `cleanup_failures`), or a Provider thread that did not join (a `rejected` note).
/// `None` for any other end, which is the caller's assertion to make.
pub fn creation_limit(reason: &str, cleanup_timed_out: bool, rejected: &[&str]) -> Option<&'static str> {
    if reason.starts_with("KC-12a:") {
        Some("KC-12a")
    } else if reason.starts_with("KC-21a:") {
        Some("KC-21a")
    } else if cleanup_timed_out {
        Some("RS-8a")
    } else if rejected.iter().any(|note| note.contains("did not join within")) {
        Some("join")
    } else {
        None
    }
}

#[test]
fn a_marker_is_one_line_naming_the_test_and_the_serial_re_run() {
    assert_eq!(timing_line("p", "b", "m::t", "late\nagain"), "EZSDR-TIMING: p b m::t: late again (serial re-run: cargo test -p p --test b -- --exact --test-threads=1 m::t)\n");
}

#[test]
fn the_creation_limits_are_told_apart_by_reason_prefix_timed_out_and_the_join_note() {
    let join = ["did not join within 1 s; left detached"];
    assert_eq!(creation_limit("KC-12a: prepare of radio did not return within 5000 ms", false, &[]), Some("KC-12a"));
    assert_eq!(creation_limit("KC-21a: a did not finish its Actions within 5000 ms", false, &join), Some("KC-21a"));
    assert_eq!(creation_limit("UR-12: refused", true, &[]), Some("RS-8a"));
    assert_eq!(creation_limit("UR-12: refused", false, &["x", join[0]]), Some("join"));
    assert_eq!(creation_limit("UR-12: refused", false, &["KC-12a: not at the start"]), None);
    assert_eq!(creation_limit("the KC-12a: later in the text", false, &[]), None);
}
