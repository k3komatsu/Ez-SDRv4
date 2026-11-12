use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("acceptance crate has a workspace root").to_path_buf()
}

fn cargo_metadata() -> Value {
    let output = Command::new(env!("CARGO")).args(["metadata", "--format-version", "1", "--no-deps"]).current_dir(workspace_root()).output().expect("run cargo metadata");
    assert!(output.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn workspace_packages(metadata: &Value) -> BTreeSet<String> {
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .map(|package| format!("{} {}", package["name"].as_str().expect("package name"), package["version"].as_str().expect("package version")))
        .collect()
}

#[test]
fn po_02_every_crate_forbids_unsafe_code() {
    let crates = workspace_root().join("crates");
    let mut members: Vec<_> = fs::read_dir(crates).unwrap().map(Result::unwrap).filter(|entry| entry.file_type().unwrap().is_dir()).collect();
    members.sort_by_key(|entry| entry.file_name());
    assert!(!members.is_empty());
    for member in members {
        let name = member.file_name();
        let name = name.to_string_lossy();
        let crate_dir = member.path();
        let lib = fs::read_to_string(crate_dir.join("src/lib.rs")).unwrap_or_else(|error| panic!("{name}/src/lib.rs: {error}"));
        if name == "ezsdr-radio-uhd" {
            // GZ-3: the one exception. `deny` at the root, and `unsafe` only in
            // `src/uhd.rs`, compiled with the feature `uhd` and allowed there alone.
            assert!(lib.contains("#![deny(unsafe_code)]"), "{name} must deny unsafe code");
            assert!(
                lib.contains("#[cfg(feature = \"uhd\")]\n#[allow(unsafe_code)]\nmod uhd;"),
                "{name}: `mod uhd` must be compiled only with the feature `uhd` and be the only module allowing unsafe"
            );
            let tests = crate_dir.join("tests");
            let tests = if tests.is_dir() { rust_files(&tests) } else { Vec::new() };
            for file in rust_files(&crate_dir.join("src")).into_iter().chain(tests) {
                if file.ends_with("src/uhd.rs") {
                    continue;
                }
                let source = fs::read_to_string(&file).unwrap();
                let code: String = source.lines().map(|line| line.split("//").next().unwrap_or_default()).collect();
                let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                let token = code.match_indices("unsafe").any(|(at, _)| !ident(code[..at].chars().next_back()) && !ident(code[at + 6..].chars().next()));
                assert!(!token, "{}: `unsafe` outside src/uhd.rs (GZ-3)", file.display());
            }
        } else {
            assert!(lib.contains("#![forbid(unsafe_code)]"), "{name} must forbid unsafe code");
        }
        assert!(lib.contains("#![warn(missing_docs)]"), "{name} must warn on missing docs");
        let manifest = fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
        for field in ["edition", "rust-version", "license"] {
            assert!(manifest.contains(&format!("{field}.workspace = true")), "{name} must inherit {field}");
        }
    }
}

#[test]
fn po_04_the_lock_gains_no_external_package() {
    let root = workspace_root();
    let workspace = workspace_packages(&cargo_metadata());
    let baseline: BTreeSet<String> =
        fs::read_to_string(root.join("crates/ezsdr-acceptance/tests/baseline_external_packages.txt")).unwrap().lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect();
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    for package in lock.split("[[package]]").skip(1) {
        let name = package.lines().find_map(|line| line.trim().strip_prefix("name = \"")).and_then(|line| line.strip_suffix('\"')).expect("external lock entry has a name");
        let version = package.lines().find_map(|line| line.trim().strip_prefix("version = \"")).and_then(|line| line.strip_suffix('\"')).expect("lock entry has a version");
        let key = format!("{name} {version}");
        if package.lines().any(|line| line.trim_start().starts_with("source = ")) {
            assert!(baseline.contains(&key), "new external package: {key}");
        } else {
            assert!(workspace.contains(&key), "path package is not a workspace member: {key}");
        }
    }
}

#[test]
fn ma_03_no_module_crate_depends_on_another() {
    let metadata = cargo_metadata();
    let modules = BTreeSet::from(["ezsdr-sim-engine", "ezsdr-mock-radio", "ezsdr-link-host", "ezsdr-sink-capture", "ezsdr-exec-native", "ezsdr-radio-uhd"]);
    let expected_normal = BTreeMap::from([
        ("ezsdr-kernel", BTreeSet::from(["serde", "serde_json", "schemars", "sha2"])),
        ("ezsdr-radio", BTreeSet::from(["ezsdr-kernel", "serde", "serde_json", "schemars"])),
        ("ezsdr-sim", BTreeSet::from(["ezsdr-kernel", "serde", "serde_json", "schemars"])),
        ("ezsdr-sink", BTreeSet::from(["ezsdr-kernel", "serde", "serde_json", "schemars"])),
        ("ezsdr-hostmem", BTreeSet::from(["ezsdr-kernel"])),
        ("ezsdr-sim-engine", BTreeSet::from(["ezsdr-kernel", "ezsdr-sim", "serde_json"])),
        ("ezsdr-mock-radio", BTreeSet::from(["ezsdr-kernel", "ezsdr-radio", "ezsdr-sim", "ezsdr-hostmem", "serde", "serde_json"])),
        ("ezsdr-link-host", BTreeSet::from(["ezsdr-kernel"])),
        ("ezsdr-sink-capture", BTreeSet::from(["ezsdr-kernel", "ezsdr-sink", "ezsdr-hostmem", "serde_json"])),
        ("ezsdr-exec-native", BTreeSet::from(["ezsdr-kernel", "serde_json"])),
        // Phase 7 §5: the UHD Module uses the Kernel, the radio Vocabulary and hostmem.
        ("ezsdr-radio-uhd", BTreeSet::from(["ezsdr-kernel", "ezsdr-radio", "ezsdr-hostmem", "serde_json"])),
        // The server is the Runtime that compiles the Modules in, not a Module (Phase 6 §5).
        (
            "ezsdr-server",
            BTreeSet::from([
                "ezsdr-kernel",
                "ezsdr-radio",
                "ezsdr-sim",
                "ezsdr-sink",
                "ezsdr-sim-engine",
                "ezsdr-mock-radio",
                "ezsdr-link-host",
                "ezsdr-sink-capture",
                "ezsdr-exec-native",
                "ezsdr-radio-uhd",
                "serde",
                "serde_json",
                "schemars",
            ]),
        ),
        (
            "ezsdr-acceptance",
            BTreeSet::from([
                "ezsdr-kernel",
                "ezsdr-radio",
                "ezsdr-sim",
                "ezsdr-sink",
                "ezsdr-hostmem",
                "ezsdr-sim-engine",
                "ezsdr-mock-radio",
                "ezsdr-link-host",
                "ezsdr-sink-capture",
                "ezsdr-exec-native",
                "ezsdr-server",
                "ezsdr-radio-uhd",
                "serde_json",
            ]),
        ),
    ]);
    for package in metadata["packages"].as_array().expect("metadata packages") {
        let package_name = package["name"].as_str().unwrap();
        let mut normal = BTreeSet::new();
        let mut dev = BTreeSet::new();
        for dependency in package["dependencies"].as_array().expect("package dependencies") {
            let dependency_name = dependency["name"].as_str().unwrap();
            match dependency["kind"].as_str() {
                None => {
                    normal.insert(dependency_name);
                }
                Some("dev") => {
                    dev.insert(dependency_name);
                    if dependency_name == "ezsdr-kernel" {
                        let features: BTreeSet<_> = dependency["features"].as_array().unwrap().iter().map(|feature| feature.as_str().unwrap()).collect();
                        assert_eq!(features, BTreeSet::from(["testing"]), "{package_name} Kernel dev-dependency features");
                    }
                }
                Some(kind) => panic!("unexpected {kind} dependency in {package_name}"),
            }
        }
        let expected = expected_normal.get(package_name).unwrap_or_else(|| panic!("unexpected workspace crate {package_name}"));
        assert_eq!(&normal, expected, "{package_name} normal dependencies must match plan/phase2/00-overview.md §5");
        let expected_dev = if package_name == "ezsdr-kernel" {
            BTreeSet::from(["ezsdr-kernel", "syn"])
        } else if package_name == "ezsdr-server" {
            // Its own `test-support` seams, which only its tests enable (#89's watch; plan/phase2/00-overview.md §5).
            BTreeSet::from(["ezsdr-kernel", "ezsdr-server"])
        } else if package_name == "ezsdr-radio-uhd" {
            // Spec 18 §6: tests/fake.rs assembles a Run as the server's catalogue does,
            // with the capture Sink and the host Link (Phase 7; workspace crates only).
            BTreeSet::from(["ezsdr-kernel", "ezsdr-sink", "ezsdr-sink-capture", "ezsdr-link-host"])
        } else {
            BTreeSet::from(["ezsdr-kernel"])
        };
        assert!(dev.is_subset(&expected_dev), "{package_name} dev-dependencies exceed PO-8's allow-list");
        if modules.contains(package_name) {
            assert!(normal.is_disjoint(&modules), "Module {package_name} depends on another Module");
        }
    }
}

#[test]
fn po_11_no_hashmap_and_no_wall_clock_in_simulation_code() {
    const FORBIDDEN: [&str; 7] = ["HashMap", "HashSet", "SystemTime", "Instant::now", "thread::spawn", "thread::Builder", "rand::"];
    let root = workspace_root();
    let new_crates = [
        "ezsdr-acceptance",
        "ezsdr-hostmem",
        "ezsdr-link-host",
        "ezsdr-mock-radio",
        "ezsdr-radio",
        "ezsdr-sim",
        "ezsdr-sim-engine",
        "ezsdr-sink",
        "ezsdr-sink-capture",
        "ezsdr-exec-native",
        "ezsdr-server",
    ];
    let mut source_roots: Vec<PathBuf> = new_crates.iter().map(|name| root.join("crates").join(name).join("src")).collect();
    source_roots.push(root.join("crates/ezsdr-kernel/src/coordinator"));
    // GZ-4: `paced.rs` runs only in HardwareInLoop and Hardware, which read the host
    // clock by definition; `kg_02_the_simulation_class_starts_no_thread` keeps a
    // Simulation Run out of it.
    let paced = root.join("crates/ezsdr-kernel/src/coordinator/paced.rs");
    // The stdio server's input thread reads no clock and only reports the client's departure
    // (KC-36, EA-15); the Simulation end instant it causes is recorded in `termination.at` (#89).
    let input = root.join("crates/ezsdr-server/src/transport.rs");
    // The binary's fail-stop timer (EA-15, A1) is a wall-clock thread by definition; it runs after the input ended.
    let backstop = root.join("crates/ezsdr-server/src/main.rs");
    for source_root in source_roots {
        for file in rust_files(&source_root).into_iter().filter(|file| *file != paced && *file != input && *file != backstop) {
            let source = fs::read_to_string(&file).unwrap();
            for (line_number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or_default();
                for forbidden in FORBIDDEN {
                    assert!(!code.contains(forbidden), "{}:{} contains {forbidden}", file.display(), line_number + 1);
                }
            }
        }
    }
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn gz_08_hardware_tests_are_gated() {
    let root = workspace_root();
    let hardware = root.join("crates/ezsdr-radio-uhd/tests/hardware.rs");
    let source = fs::read_to_string(&hardware).unwrap();
    assert!(source.contains("#![cfg(feature = \"uhd\")]"), "GZ-8: hardware.rs compiles only with the feature `uhd`");
    assert!(source.contains("EZSDR_UHD_ARGS"), "GZ-8: the device comes from EZSDR_UHD_ARGS");
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let tests: Vec<usize> = (0..lines.len()).filter(|at| lines[*at] == "#[test]").collect();
    assert!(!tests.is_empty());
    for at in tests {
        assert_eq!(lines.get(at + 1).copied(), Some("#[ignore = \"needs a USRP: see plan/phase7/bench.md\"]"), "GZ-8: hardware.rs:{} is not ignored", at + 1);
    }
    // No other test reads the device string: the hardware tests live in hardware.rs only.
    let this = root.join("crates/ezsdr-acceptance/tests/governance.rs");
    for member in fs::read_dir(root.join("crates")).unwrap().map(Result::unwrap) {
        let tests = member.path().join("tests");
        if !tests.is_dir() {
            continue;
        }
        for file in rust_files(&tests).into_iter().filter(|file| *file != hardware && *file != this) {
            assert!(!fs::read_to_string(&file).unwrap().contains("EZSDR_UHD_ARGS"), "GZ-8: {} reads EZSDR_UHD_ARGS", file.display());
        }
    }
}

// ---------------------------------------------------------------- the Run wrapper's scan (note 51 stage 2a)
//
// The UHD tests drive their Runs through `tests/common/run.rs`, which tells a real-time bound from a failure of the code under test, and read time
// only through the rigs. `scan` finds the ways around it that a new test could take. It reads tokens, not the time sources reached through a
// helper, so it stops the new bypass, not every bypass. Comments and string literals are blanked first, so only code counts.

struct Tok {
    text: String,
    line: usize,
}

/// `source` with comments, string literals and char literals replaced by spaces (line breaks kept).
fn blank(source: &str) -> String {
    let c: Vec<char> = source.chars().collect();
    let ident = |at: usize| c.get(at).is_some_and(|ch| ch.is_alphanumeric() || *ch == '_');
    let at = |i: usize| c.get(i).copied();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let blanked = |ch: char| if ch == '\n' { '\n' } else { ' ' };
    while i < c.len() {
        if c[i] == '/' && at(i + 1) == Some('/') {
            while i < c.len() && c[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c[i] == '/' && at(i + 1) == Some('*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if c[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blanked(c[i]));
                    i += 1;
                }
            }
        } else if c[i] == 'r' && (i == 0 || !ident(i - 1) || (c[i - 1] == 'b' && (i < 2 || !ident(i - 2)))) && matches!(at(i + 1), Some('"' | '#')) {
            let hashes = c[i + 1..].iter().take_while(|ch| **ch == '#').count();
            if at(i + 1 + hashes) != Some('"') {
                out.push('r');
                i += 1;
                continue;
            }
            let mut end = i + 2 + hashes;
            while end < c.len() && !(c[end] == '"' && c[end + 1..].iter().take(hashes).filter(|ch| **ch == '#').count() == hashes) {
                end += 1;
            }
            let end = (end + 1 + hashes).min(c.len());
            out.extend(c[i..end].iter().map(|ch| blanked(*ch)));
            i = end;
        } else if c[i] == '"' {
            out.push(' ');
            i += 1;
            while i < c.len() && c[i] != '"' {
                let width = if c[i] == '\\' { 2 } else { 1 };
                out.extend(c[i..(i + width).min(c.len())].iter().map(|ch| blanked(*ch)));
                i += width;
            }
            out.push(' ');
            i += 1;
        } else if c[i] == '\'' && at(i + 1) == Some('\\') {
            let end = (i + 2..c.len().min(i + 12)).find(|at| c[*at] == '\'').map_or(i + 1, |at| at + 1);
            out.extend(c[i..end].iter().map(|ch| blanked(*ch)));
            i = end;
        } else if c[i] == '\'' && at(i + 2) == Some('\'') {
            out.push_str("   ");
            i += 3;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// Identifiers (and numbers) and single punctuation characters; `::` is two `:` tokens.
fn tokens(code: &str) -> Vec<Tok> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut line = 1;
    let mut word = String::new();
    let flush = |word: &mut String, line: usize, toks: &mut Vec<Tok>| {
        if !word.is_empty() {
            toks.push(Tok { text: std::mem::take(word), line });
        }
    };
    for ch in code.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
            continue;
        }
        flush(&mut word, line, &mut toks);
        if ch == '\n' {
            line += 1;
        } else if !ch.is_whitespace() {
            toks.push(Tok { text: ch.to_string(), line });
        }
    }
    flush(&mut word, line, &mut toks);
    toks
}

fn at_pattern(toks: &[Tok], i: usize, pattern: &[&str]) -> bool {
    pattern.iter().enumerate().all(|(k, text)| toks.get(i + k).is_some_and(|tok| tok.text == *text))
}

/// Files of `ezsdr-acceptance`'s tests that create no FakeDevice Run: Mock or stepped Runs, a Provider-direct rig, or this scan.
const ACCEPTANCE_EXEMPT: [&str; 9] = ["v58.rs", "v61.rs", "reactive.rs", "parity.rs", "measurements.rs", "x3xx.rs", "o1_oracle.rs", "repeat_switch.rs", "governance.rs"];
const ACCEPTANCE_EXEMPT_DIRS: [&str; 2] = ["support/", "oracle/"];

/// `(file, fn, reason)`: a `#[test]` fn that reads time itself. Most are Authority, `Direct`-rig or device tests with no Run; one drives a Run and
/// measures a property of time (the reason says which).
const TIME_ALLOWED: [(&str, &str, &str); 22] = [
    ("crates/ezsdr-radio-uhd/tests/common/probe_device.rs", "drain_without_reader_resumes_after_rx_panic", "no Run: a probe device read through the device's clock"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "a_reopen_of_the_same_stream_and_a_disjoint_channel_set_are_allowed", "no Run: a guarded device"),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "direct_rig_a_wait_that_ends_on_a_poll_instant_still_receives_at_the_next_poll",
        "the Direct rig's own tests: they read the device clock and the wall on purpose",
    ),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "direct_rig_dropped_joins_uhd_clock_even_asleep_inside_its_read", "the Direct rig's own tests: they read the device clock and the wall on purpose"),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "direct_rig_pushes_after_the_test_thread_was_away_are_received_at_the_same_instant",
        "the Direct rig's own tests: they read the device clock and the wall on purpose",
    ),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "direct_rig_settle_advances_the_device_clock_by_its_duration_and_wait_until_reaches_its_tick",
        "the Direct rig's own tests: they read the device clock and the wall on purpose",
    ),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "direct_rig_the_device_clock_stands_still_while_the_test_thread_works_after_each_wait",
        "the Direct rig's own tests: they read the device clock and the wall on purpose",
    ),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "direct_rig_two_rigs_on_one_thread_move_only_the_clock_the_thread_waits_on",
        "the Direct rig's own tests: they read the device clock and the wall on purpose",
    ),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "direct_rig_unwound_joins_uhd_clock_even_asleep_inside_its_read", "the Direct rig's own tests: they read the device clock and the wall on purpose"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "raw_group_limit_stops_every_later_call", "no Run: the raw group's wall-clock limit"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial", "no Run: a device lost on its own host clock"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_callbacks_run_outside_the_schedule_lock", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_cancel_wakes_a_waiting_next_wakeup", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_no_device_read_follows_the_authority_s_drop", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_now_is_monotonic_across_a_reanchor", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_now_tracks_the_device_while_no_call_runs", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_schedule_accepts_an_instant_already_passed", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_schedule_wakes_a_waiting_next_wakeup", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_the_authority_paces_to_the_device", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_07_the_last_device_handle_is_not_dropped_on_uhd_clock", "an Authority test (UR-7): it reads the device clock and the wall on purpose, with no Run"),
    (
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one",
        "a Run on a Fake whose uhd-tx is descheduled: a 5 s observed-entry handshake, then the Run waits on the manual or the real clock as the case says",
    ),
    ("crates/ezsdr-radio-uhd/tests/fake.rs", "ur_33_a_real_fake_s_time_starts_at_its_own_creation", "no Run: the Fake's own clock"),
];

/// `(file, fn)`: where `Expect::Any` may appear, besides `run.rs` that defines it: the observation-only loops that record how a Run ended.
const ANY_ALLOWED: [(&str, &str); 3] = [
    ("crates/ezsdr-radio-uhd/tests/common/mod.rs", "perf_rx_rate_trial"),
    ("crates/ezsdr-radio-uhd/tests/common/mod.rs", "b8_lead_trial"),
    ("crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs", "run_stage"),
];

const RUN_FILE: &str = "crates/ezsdr-radio-uhd/tests/common/run.rs";
const TIME_SOURCES: [&[&str]; 6] = [&["thread", ":", ":", "sleep"], &["Instant", ":", ":", "now"], &[".", "elapsed", "("], &["recv_timeout", "("], &[".", "time_now", "("], &[".", "host", "("]];

struct Frame {
    name: String,
    test: bool,
    depth: usize,
}

/// Violations of the wrapper's rules in `source`, the text of `path` (relative to the workspace, `/`-separated). `used` collects the allow-list
/// rows the file relied on, so that the real-tree test can refuse a row that nothing needs.
fn scan_used(path: &str, source: &str, used: &mut BTreeSet<String>) -> Vec<String> {
    let uhd = path.starts_with("crates/ezsdr-radio-uhd/tests/");
    let acceptance = path.strip_prefix("crates/ezsdr-acceptance/tests/").is_some_and(|rest| !ACCEPTANCE_EXEMPT.contains(&rest) && !ACCEPTANCE_EXEMPT_DIRS.iter().any(|dir| rest.starts_with(dir)));
    let runs = (uhd || acceptance) && path != RUN_FILE;
    let time = path == "crates/ezsdr-radio-uhd/tests/fake.rs" || path.starts_with("crates/ezsdr-radio-uhd/tests/common/") || path == "crates/ezsdr-acceptance/tests/uhd.rs";
    let handle = path == "crates/ezsdr-server/tests/protocol.rs";
    let fake_assembly = path.starts_with("crates/ezsdr-acceptance/tests/") && path != "crates/ezsdr-acceptance/tests/uhd.rs";
    if !(runs || time || handle || fake_assembly) {
        return Vec::new();
    }
    let toks = tokens(&blank(source));
    let mut found = Vec::new();
    let (mut depth, mut nest) = (0usize, 0usize);
    let mut frames: Vec<Frame> = Vec::new();
    let (mut pending_test, mut pending_fn): (bool, Option<(String, bool)>) = (false, None);
    for (i, tok) in toks.iter().enumerate() {
        let text = tok.text.as_str();
        match text {
            "#" if at_pattern(&toks, i, &["#", "[", "test", "]"]) => pending_test = true,
            "fn" if toks.get(i + 1).is_some_and(|name| name.text.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_')) => {
                pending_fn = Some((toks[i + 1].text.clone(), std::mem::take(&mut pending_test)));
                nest = 0;
            }
            "(" | "[" => nest += 1,
            ")" | "]" => nest = nest.saturating_sub(1),
            ";" if nest == 0 => pending_fn = None,
            "{" => {
                depth += 1;
                if let Some((name, test)) = pending_fn.take() {
                    frames.push(Frame { name, test, depth });
                }
            }
            "}" => {
                if frames.last().is_some_and(|frame| frame.depth == depth) {
                    frames.pop();
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        let (in_fn, in_test) = (frames.last().map_or("", |frame| frame.name.as_str()), frames.last().is_some_and(|frame| frame.test));
        let mut violation = |what: &str| found.push(format!("{path}:{}: {what} (in fn `{in_fn}`)", tok.line));
        if runs {
            if text == "RunHandle" {
                violation("a raw RunHandle: use the wrapper `Run`");
            }
            if (text == "connect" || text == "start_spec_run") && !(i >= 3 && at_pattern(&toks, i - 3, &["Run", ":", ":"])) && !(i >= 1 && toks[i - 1].text == "fn") {
                violation(&format!("a raw `{text}`, called, imported or aliased: create the Run through `Run::connect` / `Run::start_spec`"));
            }
            if at_pattern(&toks, i, &["Expect", ":", ":", "Any"]) {
                if ANY_ALLOWED.iter().any(|(file, name)| *file == path && *name == in_fn) {
                    used.insert(format!("any {path} {in_fn}"));
                } else {
                    violation("`Expect::Any` outside the named observation-only loops");
                }
            }
        }
        if time && in_test {
            for pattern in TIME_SOURCES {
                if at_pattern(&toks, i, pattern) {
                    if TIME_ALLOWED.iter().any(|(file, name, _)| *file == path && *name == in_fn) {
                        used.insert(format!("time {path} {in_fn}"));
                    } else {
                        violation(&format!("`{}` read by a test itself: move it into a helper or a rig", pattern.concat()));
                    }
                }
            }
        }
        if fake_assembly && text == "assemble_on" {
            violation("`assemble_on` outside `uhd.rs`: a FakeDevice assembly would reach a raw `connect` (`support::session_run_with`) past the wrapper");
        }
        if handle && at_pattern(&toks, i, &[".", "handle", "("]) && in_fn != "handle" {
            violation("`.handle(` outside `fn handle`, the door that carries the diagnostic delay");
        }
    }
    found
}

fn scan(path: &str, source: &str) -> Vec<String> {
    scan_used(path, source, &mut BTreeSet::new())
}

const FAKE: &str = "crates/ezsdr-radio-uhd/tests/fake.rs";
const COMMON: &str = "crates/ezsdr-radio-uhd/tests/common/mod.rs";
const HARDWARE: &str = "crates/ezsdr-radio-uhd/tests/hardware.rs";
const PROTOCOL: &str = "crates/ezsdr-server/tests/protocol.rs";

#[test]
fn run_scan_the_tree_is_clean_and_every_allowance_is_used() {
    let root = workspace_root();
    let mut used = BTreeSet::new();
    let mut found = Vec::new();
    for dir in ["crates/ezsdr-radio-uhd/tests", "crates/ezsdr-acceptance/tests", "crates/ezsdr-server/tests"] {
        for file in rust_files(&root.join(dir)) {
            let path = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            found.extend(scan_used(&path, &fs::read_to_string(&file).unwrap(), &mut used));
        }
    }
    assert!(found.is_empty(), "the Run wrapper's scan (note 51 stage 2a):\n{}", found.join("\n"));
    for (file, name, _) in TIME_ALLOWED {
        assert!(used.contains(&format!("time {file} {name}")), "the time allow-list row {file} {name} is not needed any more: remove it");
    }
    for (file, name) in ANY_ALLOWED {
        assert!(used.contains(&format!("any {file} {name}")), "the `Expect::Any` allow-list row {file} {name} is not needed any more: remove it");
    }
}

#[test]
fn run_scan_finds_a_raw_run_creation_however_it_is_written() {
    let hits = |source: &str| scan(COMMON, source).len();
    assert_eq!(hits("fn t() { let run = connect(&p, &i, a, l); }"), 1);
    assert_eq!(hits("fn t() { let run = ezsdr_kernel::coordinator::connect(&p, &i, a, l); }"), 1);
    assert_eq!(hits("fn t() { let run = start_spec_run(&s, &p, a); }"), 1);
    assert_eq!(hits("fn t() { connect (&p); }"), 1, "whitespace before the parenthesis");
    assert_eq!(hits("fn t() { connect\n(&p); }"), 1, "a line break before the parenthesis");
    assert_eq!(hits("fn t() { coordinator::\nstart_spec_run(&s); }"), 1, "a line break inside the path");
    assert_eq!(hits("use ezsdr_kernel::coordinator::connect as open;\nfn t() { open(&p); }"), 1, "a renamed import");
    assert_eq!(hits("use ezsdr_kernel::coordinator::{Assembly, start_spec_run as begin};\nfn t() { begin(&s); }"), 1, "a renamed import in a group");
    assert_eq!(hits("fn t() { let create = ezsdr_kernel::coordinator::connect;\n let run = create(&p, &i, a, l); }"), 1, "a function value, called later under another name");
    assert_eq!(hits("fn t() { let begin = start_spec_run; }"), 1, "a function value");
    assert_eq!(hits("fn t() { let all = [connect, other]; }"), 1, "a function value in an array");
    assert_eq!(hits("fn t(run: &mut RunHandle) {}"), 1, "the raw handle type");
    assert_eq!(hits("fn t() { let a = connect(&p); let b = start_spec_run(&s); }"), 2);
    let message = &scan(COMMON, "fn t() {\n    connect(&p);\n}")[0];
    assert!(message.contains(&format!("{COMMON}:2:")) && message.contains("in fn `t`"), "{message}");
}

#[test]
fn run_scan_allows_the_wrapper_and_what_only_looks_like_a_raw_call() {
    let hits = |source: &str| scan(COMMON, source).len();
    assert_eq!(hits("fn t() { Run::connect(&p, &i, a, l, d); Run::start_spec(&s, &p, a, d); }"), 0, "the wrapper's constructors");
    assert_eq!(hits("fn t() { Run :: connect (&p); }"), 0, "the wrapper, spaced");
    assert_eq!(hits("fn t() { disconnect(&p); run.disconnect(); }"), 0);
    assert_eq!(hits("fn t(e: RunHandleError) -> RunHandleError { e }"), 0, "the error type");
    assert_eq!(hits("fn connect(a: u8) {}"), 0, "a definition");
    assert_eq!(hits("// connect(a)\n/// start_spec_run(a)\n/* connect(a) */ fn t() { let s = \"connect(a)\"; let r = r#\"start_spec_run(a)\"#; }"), 0, "comments and strings");
    assert_eq!(scan(RUN_FILE, "fn t() { connect(&p); start_spec_run(&s); let h: RunHandle; }").len(), 0, "run.rs is the one file that calls them");
}

#[test]
fn run_scan_reads_time_only_in_test_fns_and_lets_the_listed_ones_pass() {
    let hits = |path: &str, source: &str| scan(path, source).len();
    for source in ["std::thread::sleep(d)", "let t = Instant::now()", "t.elapsed()", "rx.recv_timeout(d)", "d.time_now()", "d.host()"] {
        assert_eq!(hits(FAKE, &format!("#[test]\nfn t() {{ {source}; }}")), 1, "{source}");
        assert_eq!(hits(FAKE, &format!("fn helper() {{ {source}; }}")), 0, "a helper may read time: {source}");
    }
    assert_eq!(hits(FAKE, "#[test]\nfn t() { let f = || std::thread::sleep(d); f(); }"), 1, "a closure inside a test");
    assert_eq!(hits(FAKE, "#[test]\n#[should_panic(expected = \"x\")]\nfn t() { std::thread::sleep(d); }"), 1, "another attribute between");
    assert_eq!(hits(FAKE, "#[test]\nfn t() { fn inner() { std::thread::sleep(d); } inner(); }"), 0, "a helper nested in a test is a helper");
    assert_eq!(hits(FAKE, "#[test]\nfn a() {}\nfn helper() { std::thread::sleep(d); }"), 0, "after a test ends its helper is not one");
    assert_eq!(hits(FAKE, "#[test]\nfn t(x: [u8; 4]) { std::thread::sleep(d); }"), 1, "a `;` in the header does not end it");
    assert_eq!(hits(HARDWARE, "#[test]\nfn t() { std::thread::sleep(d); }"), 0, "the bench tests read device time by design");
    assert_eq!(hits("crates/ezsdr-acceptance/tests/uhd.rs", "#[test]\nfn t() { Instant::now(); }"), 1);
    assert_eq!(hits(COMMON, "#[test]\nfn t() { Instant::now(); }"), 1);
    assert_eq!(hits("crates/ezsdr-acceptance/tests/v58.rs", "#[test]\nfn t() { Instant::now(); }"), 0, "outside the files the rule covers");
    let (file, name, _) = TIME_ALLOWED[0];
    assert_eq!(hits(file, &format!("#[test]\nfn {name}() {{ std::thread::sleep(d); }}")), 0, "a listed fn passes");
    assert_eq!(hits(file, &format!("#[test]\nfn {name}_other() {{ std::thread::sleep(d); }}")), 1, "another fn of that file does not");
    let elsewhere = if file == FAKE { COMMON } else { FAKE };
    assert_eq!(hits(elsewhere, &format!("#[test]\nfn {name}() {{ std::thread::sleep(d); }}")), 1, "the same name in another file does not");
}

#[test]
fn run_scan_covers_the_files_it_names_and_skips_the_others() {
    let raw = "fn t() { connect(&p); }";
    for path in [
        "crates/ezsdr-radio-uhd/tests/fake.rs",
        "crates/ezsdr-radio-uhd/tests/hardware.rs",
        "crates/ezsdr-radio-uhd/tests/new_file.rs",
        "crates/ezsdr-radio-uhd/tests/common/measurement/x.rs",
        "crates/ezsdr-acceptance/tests/uhd.rs",
        "crates/ezsdr-acceptance/tests/a_new_file.rs",
    ] {
        assert_eq!(scan(path, raw).len(), 1, "{path}");
    }
    for path in [
        "crates/ezsdr-acceptance/tests/v58.rs",
        "crates/ezsdr-acceptance/tests/parity.rs",
        "crates/ezsdr-acceptance/tests/support/mod.rs",
        "crates/ezsdr-acceptance/tests/oracle/x.rs",
        "crates/ezsdr-kernel/tests/x.rs",
        "crates/ezsdr-radio-uhd/src/lib.rs",
    ] {
        assert_eq!(scan(path, raw).len(), 0, "{path}");
    }
}

#[test]
fn run_scan_a_fake_device_assembly_is_made_only_in_uhd_rs() {
    let source = "fn t() { let a = rig::assemble_on(&p, i, d); }";
    for path in [
        "crates/ezsdr-acceptance/tests/support/mod.rs",
        "crates/ezsdr-acceptance/tests/v58.rs",
        "crates/ezsdr-acceptance/tests/oracle/x.rs",
        "crates/ezsdr-acceptance/tests/a_new_file.rs",
        "crates/ezsdr-acceptance/tests/governance.rs",
    ] {
        assert_eq!(scan(path, source).len(), 1, "{path}");
    }
    assert_eq!(scan("crates/ezsdr-acceptance/tests/uhd.rs", source).len(), 0, "the file whose Runs the wrapper wraps");
    assert_eq!(scan("crates/ezsdr-acceptance/tests/support/mod.rs", "fn t() { rig :: assemble_on (&p); }").len(), 1, "spaced");
    assert_eq!(scan("crates/ezsdr-acceptance/tests/support/mod.rs", "fn t() { let f = rig::assemble_on; }").len(), 1, "a function value");
    assert_eq!(
        scan("crates/ezsdr-acceptance/tests/support/mod.rs", "// assemble_on\nfn t() { let s = \"assemble_on\"; assemble(&p); assemble_on_other(&p); }").len(),
        0,
        "comments, strings and other names"
    );
    assert_eq!(scan("crates/ezsdr-acceptance/src/rig.rs", source).len(), 0, "the definition is outside the tests");
    assert_eq!(scan("crates/ezsdr-radio-uhd/tests/common/mod.rs", source).len(), 0, "only the acceptance tests have this helper");
}

#[test]
fn run_scan_allows_expect_any_in_its_named_loops_only() {
    let any = |path: &str, name: &str| scan(path, &format!("fn {name}() {{ let d = Declared {{ end: Some(Expect::Any), ..Default::default() }}; }}")).len();
    for (file, name) in ANY_ALLOWED {
        assert_eq!(any(file, name), 0, "{file} {name}");
        assert_eq!(any(file, &format!("{name}_other")), 1, "another fn of {file}");
        let elsewhere = if file == FAKE { COMMON } else { FAKE };
        assert_eq!(any(elsewhere, name), 1, "{name} in another file");
    }
    assert_eq!(scan(FAKE, "fn t() { Expect :: Any; }").len(), 1, "spaced");
    assert_eq!(scan(FAKE, "// Expect::Any\nfn t() { let s = \"Expect::Any\"; }").len(), 0, "a comment and a string");
    assert_eq!(scan(RUN_FILE, "fn t() { Expect::Any; }").len(), 0, "run.rs defines it");
    assert_eq!(scan(FAKE, "const A: Expect = Expect::Any;").len(), 1, "outside any fn");
}

#[test]
fn run_scan_the_diagnostic_door_is_the_only_handle_call() {
    assert_eq!(scan(PROTOCOL, "fn handle(s: &mut Server) { s.handle(r, b) }").len(), 0);
    assert_eq!(scan(PROTOCOL, "fn t() { s.handle(r, b); }").len(), 1);
    assert_eq!(scan(PROTOCOL, "fn t() { Server::new(c).handle(r, b); }").len(), 1);
    assert_eq!(scan(PROTOCOL, "fn t() { handle(&mut s, r, b); }").len(), 0, "the door itself");
    assert_eq!(scan(FAKE, "fn t() { s.handle(r, b); }").len(), 0, "only protocol.rs");
}
