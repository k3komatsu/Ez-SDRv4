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

/// For every token: the innermost fn it is in (`""` outside any) and whether that fn is a `#[test]` fn. A closure belongs to the fn around it; a fn nested
/// in another is its own.
fn fn_context(toks: &[Tok]) -> Vec<(String, bool)> {
    let (mut depth, mut nest) = (0usize, 0usize);
    let mut frames: Vec<Frame> = Vec::new();
    let (mut pending_test, mut pending_fn): (bool, Option<(String, bool)>) = (false, None);
    let mut context = Vec::with_capacity(toks.len());
    for (i, tok) in toks.iter().enumerate() {
        match tok.text.as_str() {
            "#" if at_pattern(toks, i, &["#", "[", "test", "]"]) => pending_test = true,
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
        context.push((frames.last().map_or(String::new(), |frame| frame.name.clone()), frames.last().is_some_and(|frame| frame.test)));
    }
    context
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
    let context = fn_context(&toks);
    let mut found = Vec::new();
    for (i, tok) in toks.iter().enumerate() {
        let text = tok.text.as_str();
        let (in_fn, in_test) = (context[i].0.as_str(), context[i].1);
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

// ---------------------------------------------------------------- the Kernel's wall expiries (note 51 stage 3c-2)
//
// Every test of the Kernel's device-paced file runs on the host clock, where a wall interval that judges a positive outcome is a flake on a loaded host. One
// rule: a positive wall expiry is `from_secs(n)` with n >= 3. `kernel_wall_scan` finds what breaks it in every fn of the file, helpers included: R1 a `<` / `<=`
// against a `from_millis`, `from_micros`, `from_nanos`, `from_secs_f32` / `_f64` or `from_secs(n < 3)` constructor (reached by any path), R2 such a constructor
// inside the call of `wait_for`, `wait_for_count`, `wait_until`, `wait_parked_until`, `wait_timeout`, `park_timeout`, `recv_timeout` or `recv_deadline`, R3 any
// call of a path ending in `sleep`. A hit is allowed only as a pinned occurrence: `(fn, call, exact count, reason)`, so a new sleep in a pinned fn is a
// violation. Limits, stated: only a bound spelt as one of those forms is seen, so `elapsed().as_*() < n`, a mirrored `bound > elapsed()`, `Wall::new(..)`, a
// non-decimal literal and a computed one (`from_secs(10 - 8)`: only the first token is read) pass, and so does a wall read hidden in another file.

const KERNEL_FILE: &str = "crates/ezsdr-kernel/tests/device_paced.rs";

const NEGATIVE: &str = "negative: a slow host only weakens it";
const LOWER_BOUND: &str = "lower bound: cannot false-kill";
const TTL: &str = "TTL is the subject";

/// `(fn, call, exact count, reason)`: `call` is the occurrence's own tokens, joined without spaces and spelt as the code spells it.
const KERNEL_WALL_ALLOWED: [(&str, &str, usize, &str); 11] = [
    ("kg_02_a_failed_sink_is_not_stepped_again", "std::thread::sleep(Wall::from_millis(100))", 1, NEGATIVE),
    ("kc_29_an_end_the_data_thread_requested_is_cleaned_up_at_the_next_call", "std::thread::sleep(Wall::from_millis(50))", 1, NEGATIVE),
    ("kg_03_an_abort_stops_the_data_thread", "std::thread::sleep(Wall::from_millis(50))", 1, NEGATIVE),
    ("kg_03_dropping_a_live_device_paced_handle_cleans_up", "std::thread::sleep(Wall::from_millis(20))", 1, NEGATIVE),
    ("kg_21a_submit_cannot_complete_before_the_booking_gate_opens", "recv_timeout(Wall::from_millis(150))", 1, NEGATIVE),
    // The call's string argument is blanked, so it reads `wait_for(,…)`; the result is ignored (`let _ =`): a pause in `Gate::check`, never a verdict.
    ("check", "wait_for(,Wall::from_millis(200))", 1, NEGATIVE),
    ("tm_16c_a_paced_callback_sees_now_at_or_after_its_instant", "std::thread::sleep(Wall::from_millis(2))", 1, LOWER_BOUND),
    ("tm_16c_a_paced_authority_accepts_an_instant_already_passed", "std::thread::sleep(Wall::from_millis(6))", 1, LOWER_BOUND),
    ("kc_36_renew_moves_the_deadline_the_data_thread_checks", "std::thread::sleep(Wall::from_millis(50))", 1, TTL),
    ("kc_36_renew_moves_the_deadline_the_data_thread_checks", "<Wall::from_millis(600)", 1, TTL),
    ("kc_36_adopt_clears_the_deadline_the_data_thread_checks", "std::thread::sleep(Wall::from_millis(300))", 1, TTL),
];

const WALL_CALLS: [&str; 8] = ["wait_for", "wait_for_count", "wait_until", "wait_parked_until", "wait_timeout", "park_timeout", "recv_timeout", "recv_deadline"];

const INT_SUFFIXES: [&str; 12] = ["u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"];

/// A decimal integer literal token, underscores and a type suffix (`1_u64`) removed.
fn int_literal(text: &str) -> Option<u64> {
    let text = INT_SUFFIXES.iter().find_map(|suffix| text.strip_suffix(suffix)).unwrap_or(text);
    text.replace('_', "").parse().ok()
}

/// Whether token `j` is the constructor of a wall interval shorter than 3 s: `from_secs(n)` with a literal n < 3, or any finer unit; reached by a path
/// (`Wall::from_millis`), whatever the type is called.
fn short_wall_at(toks: &[Tok], j: usize) -> bool {
    let named = |name: &str| toks[j].text == name;
    let path = j >= 2 && toks[j - 1].text == ":" && toks[j - 2].text == ":";
    let open = toks.get(j + 1).is_some_and(|tok| tok.text == "(");
    path && open
        && (["from_millis", "from_micros", "from_nanos", "from_secs_f32", "from_secs_f64"].iter().any(|name| named(name))
            || (named("from_secs") && toks.get(j + 2).and_then(|tok| int_literal(&tok.text)).is_some_and(|secs| secs < 3)))
}

/// The tokens `from..=to`, joined without spaces.
fn joined(toks: &[Tok], from: usize, to: usize) -> String {
    toks[from..=to.min(toks.len() - 1)].iter().map(|tok| tok.text.as_str()).collect()
}

type Allowed<'a> = &'a [(&'a str, &'a str, usize, &'a str)];

/// Violations of the Kernel wall rule in `source` (the text of `KERNEL_FILE`) against `allowed`; the pinned occurrences it relied on go to `used` as `fn call`.
fn wall_scan(source: &str, allowed: Allowed, used: &mut BTreeSet<String>) -> Vec<String> {
    let toks = tokens(&blank(source));
    let context = fn_context(&toks);
    let mut hits: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, tok) in toks.iter().enumerate() {
        let call = if tok.text == "<" {
            // R1: `<` or `<=`, then a path whose last segment is the constructor.
            let mut j = i + 1 + usize::from(toks.get(i + 1).is_some_and(|next| next.text == "="));
            while at_pattern(&toks, j + 1, &[":", ":"]) {
                j += 3;
            }
            (j < toks.len() && short_wall_at(&toks, j)).then(|| joined(&toks, i, paren_close(&toks, j + 1)))
        } else if WALL_CALLS.contains(&tok.text.as_str()) && at_pattern(&toks, i + 1, &["("]) && !(i > 0 && toks[i - 1].text == "fn") {
            // R2: a short literal inside the call's parentheses.
            let close = paren_close(&toks, i + 1);
            (i + 2..close).any(|j| short_wall_at(&toks, j)).then(|| joined(&toks, i, close))
        } else if tok.text == "sleep" && at_pattern(&toks, i + 1, &["("]) && !(i > 0 && (toks[i - 1].text == "." || toks[i - 1].text == "fn")) {
            // R3: any call of a path ending in `sleep`, from the path's start.
            let mut start = i;
            while start >= 3 && at_pattern(&toks, start - 2, &[":", ":"]) && toks[start - 3].text.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_') {
                start -= 3;
            }
            Some(joined(&toks, start, paren_close(&toks, i + 1)))
        } else {
            None
        };
        if let Some(call) = call {
            hits.entry((context[i].0.clone(), call)).or_default().push(tok.line);
        }
    }
    let mut found = Vec::new();
    for ((name, call), lines) in hits {
        match allowed.iter().find(|(fn_name, pinned, _, _)| *fn_name == name && *pinned == call) {
            Some((_, _, count, _)) if *count == lines.len() => {
                used.insert(format!("{name} {call}"));
            }
            Some((_, _, count, _)) => found.push(format!("{KERNEL_FILE}:{}: `{call}` occurs {} times in fn `{name}`, its allowance pins {count}", lines[0], lines.len())),
            None => found.push(format!("{KERNEL_FILE}:{}: a positive wall expiry shorter than 3 s, or a sleep: `{call}` (in fn `{name}`)", lines[0])),
        }
    }
    found
}

fn kernel_wall_scan(source: &str, used: &mut BTreeSet<String>) -> Vec<String> {
    wall_scan(source, &KERNEL_WALL_ALLOWED, used)
}

#[test]
fn kernel_wall_the_tree_is_clean_and_every_allowance_is_used() {
    let mut used = BTreeSet::new();
    let found = kernel_wall_scan(&fs::read_to_string(workspace_root().join(KERNEL_FILE)).unwrap(), &mut used);
    assert!(found.is_empty(), "the Kernel wall scan (note 51 stage 3c-2):\n{}", found.join("\n"));
    for (name, call, _, _) in KERNEL_WALL_ALLOWED {
        assert!(used.contains(&format!("{name} {call}")), "the allowance {name} {call} is not needed any more: remove it");
    }
}

#[test]
fn kernel_wall_scan_finds_a_sleep_and_a_short_expiry_however_it_is_written() {
    let hits = |source: &str| kernel_wall_scan(source, &mut BTreeSet::new()).len();
    let body = |statement: &str| format!("#[test]\nfn t() {{ {statement} }}");
    assert_eq!(hits(&body("sleep(Wall::from_millis(100)); assert!(count(&probe, \"x\") >= 50);")), 1, "an unpinned sleep");
    assert_eq!(hits(&body("std::thread::sleep(d);")), 1, "a sleep of any argument");
    assert_eq!(hits(&body("use std::thread::sleep; sleep(d);")), 1, "a sleep imported and called by its last segment");
    assert_eq!(hits(&body("thread::sleep(d); std::thread::sleep(d);")), 2, "two spellings are two calls");
    for (source, count) in [
        ("assert!(begun.elapsed() < Wall::from_secs(1));", 1),
        ("assert!(begun.elapsed() < Wall::from_secs(2));", 1),
        ("assert!(begun.elapsed() < Wall::from_secs(3));", 0),
        ("assert!(begun.elapsed() < Wall::from_secs(2_0));", 0),
        ("assert!(begun.elapsed() < Wall::from_secs(1_u64));", 1),
        ("assert!(begun.elapsed() < Wall::from_millis(200u32));", 1),
        ("assert!(begun.elapsed() < Wall::from_secs(10_u64));", 0),
        ("assert!(begun.elapsed() < Wall::from_millis(5));", 1),
        ("assert!(begun.elapsed() <= Wall::from_millis(5));", 1),
        ("assert!(begun.elapsed() < std::time::Duration::from_micros(5));", 1),
        ("assert!(waited < Wall::from_secs(10));", 0),
        ("assert!(waited < Wall::from_millis(900));", 1),
        ("assert!(waited >= Wall::from_millis(900));", 0),
        ("let v: Vec<Wall> = f(); let w = Wall::from_millis(5);", 0),
    ] {
        assert_eq!(hits(&body(source)), count, "{source}");
    }
    for (source, count) in [
        ("probe.wait_for(\"x\", Wall::from_millis(500));", 1),
        ("probe.wait_for(\"x\", Wall::from_secs(1));", 1),
        ("probe.wait_for(\"x\", Wall::from_secs(10));", 0),
        ("rx.recv_timeout(Wall::from_secs(1_u64));", 1),
        ("rx.recv_timeout(Wall::from_secs(10_u64));", 0),
        ("probe.wait_for_count(\"x\", 2, Wall::from_secs(1));", 1),
        ("probe.wait_until(|lines| lines.len() > 1, Wall::from_millis(10));", 1),
        ("let (g, _) = cv.wait_timeout(guard, Wall::from_millis(10)).unwrap();", 1),
        ("std::thread::park_timeout(Wall::from_millis(10));", 1),
        ("rx.recv_deadline(Instant::now() + Wall::from_millis(10));", 1),
        ("rx.recv_deadline(Instant::now() + Wall::from_secs(10));", 0),
        ("park.wait_parked_until(tick, Wall::from_millis(10));", 1),
        ("rx.recv_timeout(Wall::from_millis(100));", 1),
        ("assert!(rx.recv_timeout(Wall::from_secs(5)).is_err());", 0),
        ("let _ = after(&run, Wall::from_millis(30));", 0),
    ] {
        assert_eq!(hits(&body(source)), count, "{source}");
    }
}

#[test]
fn kernel_wall_scan_reads_every_fn_and_not_comments_or_strings() {
    let hits = |source: &str| kernel_wall_scan(source, &mut BTreeSet::new()).len();
    assert_eq!(hits("fn helper() { std::thread::sleep(d); }"), 1, "a helper is scanned");
    assert_eq!(hits("#[test]\nfn t() { let f = || std::thread::sleep(d); f(); }"), 1, "a closure belongs to its test");
    assert_eq!(hits("#[test]\nfn t() { fn inner() { std::thread::sleep(d); } inner(); }"), 1, "a fn nested in a test is scanned");
    assert_eq!(hits("// std::thread::sleep(d)\n#[test]\nfn t() { let s = \"sleep(d)\"; let r = \"x < Wall::from_millis(5)\"; }"), 0, "comments and strings");
    assert_eq!(hits("#[test]\nfn t() { x.sleep(d); }"), 0, "a method of that name");
}

#[test]
fn kernel_wall_scan_pins_each_allowance_to_its_fn_call_and_count() {
    let call = "std::thread::sleep(Wall::from_millis(100))";
    let table = [("pinned", call, 2, NEGATIVE), ("other", call, 1, NEGATIVE)];
    let source = |name: &str, calls: &[&str]| format!("#[test]\nfn {name}() {{ {} }}", calls.iter().map(|call| format!("{call};")).collect::<String>());
    let run = |source: &str| {
        let mut used = BTreeSet::new();
        (wall_scan(source, &table, &mut used).len(), used.contains(&format!("pinned {call}")))
    };
    assert_eq!(run(&source("pinned", &[call, call])), (0, true), "the pinned occurrences pass and are used");
    // Sol's bypass: a third sleep, with a count it asserts on, inside an approved fn.
    assert_eq!(run(&source("pinned", &[call, call, call, "assert!(count(&probe, \"rec:block:\") >= 50)"])), (1, false), "one more than pinned is refused");
    assert_eq!(run(&source("pinned", &[call, call, "std::thread::sleep(Wall::from_millis(250))"])).0, 1, "a different duration is not the pinned call");
    assert_eq!(run(&source("pinned", &[call])), (1, false), "one fewer than pinned: refused, and the row is not used");
    assert_eq!(run(&source("pinned", &[])), (0, false), "none: the row is not used");
    assert_eq!(run(&source("unlisted", &[call, call])), (1, false), "a pinned call in an unlisted fn");
    assert_eq!(run(&source("other", &[call, call])).0, 1, "another fn's pin counts for that fn alone");
    for (name, call, count, _) in KERNEL_WALL_ALLOWED {
        let mut used = BTreeSet::new();
        assert!(kernel_wall_scan(&source(name, &vec![call; count]), &mut used).is_empty(), "{name} {call}");
        assert!(used.contains(&format!("{name} {call}")), "{name} {call}");
    }
}

// ---------------------------------------------------------------- the rehearsals' wiring (note 51 stage 2c)
//
// A rehearsal test judges each trial through `Rehearsal::trial` and ends in `Rehearsal::finish`. No load-free run and no mutation row notices a wrapper that
// was dropped at one trial site (with no lateness evidence the bare assert passes), a `finish()` that is gone (skipped trials then pass silently) or an
// old assert left beside a wrapper. `rehearsal_findings` counts them in the text of the test file, against a table the reviewed diff carries.

/// What one rehearsal test must contain, outside comments and strings: `Rehearsal::new()` once; `sites` calls of `.trial(`; `.finish()` once, after the last
/// of them; and, outside every `.trial(…)` span: `direct` occurrences of `Verdict::Inconclusive` (the deliberately late cases, judged by the shared verdict),
/// `outside` assert-family macros and `calls` calls of a function whose name ends in `_verdict` or `_outcome`.
struct Site {
    test: &'static str,
    sites: usize,
    direct: usize,
    outside: usize,
    calls: usize,
}

/// The rehearsals that keep their own tolerance and must not go through `Rehearsal` (the stage 2c plan, §2.2).
const LEFT_ALONE: [&str; 3] = ["rehearsal_b7_refusal_holds_on_a_two_board_layout", "rehearsal_b7_session_loopback", "rehearsal_b7_session_loopback_on_one_obx"];

const ASSERT_MACROS: &[&str] = &["assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne", "panic", "unreachable"];

/// The token range of the body of `fn name`, braces excluded.
fn fn_body(toks: &[Tok], name: &str) -> Option<std::ops::Range<usize>> {
    let at = (0..toks.len()).find(|&i| toks[i].text == "fn" && toks.get(i + 1).is_some_and(|tok| tok.text == name))?;
    let (mut nest, mut open) = (0usize, None);
    for (i, tok) in toks.iter().enumerate().skip(at) {
        match tok.text.as_str() {
            "(" | "[" => nest += 1,
            ")" | "]" => nest = nest.saturating_sub(1),
            "{" if nest == 0 => {
                open = Some(i);
                break;
            }
            _ => {}
        }
    }
    let open = open?;
    let mut depth = 0usize;
    for (i, tok) in toks.iter().enumerate().skip(open) {
        match tok.text.as_str() {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + 1..i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The index of the `)` that closes the `(` at `open`.
fn paren_close(toks: &[Tok], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, tok) in toks.iter().enumerate().skip(open) {
        match tok.text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

/// Whether token `i` is the name of an assert-family macro call: an exact name (so `debug_assert!` is its own name and `my_assert!` is none), followed by
/// `!`, and not a method name.
fn is_assert_macro(toks: &[Tok], i: usize) -> bool {
    ASSERT_MACROS.contains(&toks[i].text.as_str()) && toks.get(i + 1).is_some_and(|tok| tok.text == "!") && !(i > 0 && toks[i - 1].text == ".")
}

fn rehearsal_findings(source: &str, table: &[Site]) -> Vec<String> {
    let toks = tokens(&blank(source));
    let mut findings = Vec::new();
    for site in table {
        let Some(body) = fn_body(&toks, site.test) else {
            findings.push(format!("{}: no such fn", site.test));
            continue;
        };
        let at = |pattern: &[&str]| -> Vec<usize> { body.clone().filter(|&i| at_pattern(&toks, i, pattern)).collect() };
        let news = at(&["Rehearsal", ":", ":", "new", "(", ")"]);
        let spans: Vec<(usize, usize)> = at(&[".", "trial", "("]).into_iter().map(|i| (i, paren_close(&toks, i + 2))).collect();
        let finishes = at(&[".", "finish", "(", ")"]);
        let in_span = |i: usize| spans.iter().any(|&(open, close)| open <= i && i <= close);
        let found_sites = spans.len();
        let found_direct = at(&["Verdict", ":", ":", "Inconclusive"]).into_iter().filter(|&i| !in_span(i)).count();
        let found_outside = body.clone().filter(|&i| is_assert_macro(&toks, i) && !in_span(i)).count();
        let found_calls =
            body.clone().filter(|&i| (toks[i].text.ends_with("_verdict") || toks[i].text.ends_with("_outcome")) && toks.get(i + 1).is_some_and(|tok| tok.text == "(") && !in_span(i)).count();
        if news.len() != 1 {
            findings.push(format!("{}: Rehearsal::new() found {} of 1", site.test, news.len()));
        }
        if found_sites != site.sites {
            findings.push(format!("{}: .trial( sites {found_sites} of {}", site.test, site.sites));
        }
        if finishes.len() != 1 {
            findings.push(format!("{}: .finish() found {} of 1", site.test, finishes.len()));
        } else if spans.iter().map(|&(_, close)| close).max().is_some_and(|close| finishes[0] < close) {
            findings.push(format!("{}: .finish() is not after the last .trial(", site.test));
        }
        if found_direct != site.direct {
            findings.push(format!("{}: Verdict::Inconclusive outside the spans {found_direct} of {}", site.test, site.direct));
        }
        if found_outside != site.outside {
            findings.push(format!("{}: assert-family macros outside the spans: outside {found_outside} of {}", site.test, site.outside));
        }
        if found_calls != site.calls {
            findings.push(format!("{}: *_verdict( / *_outcome( calls outside the spans: calls {found_calls} of {}", site.test, site.calls));
        }
    }
    for name in LEFT_ALONE {
        if let Some(body) = fn_body(&toks, name) {
            if body.clone().any(|i| at_pattern(&toks, i, &["Rehearsal", ":", ":"])) {
                findings.push(format!("{name}: a rehearsal that keeps its tolerance goes through Rehearsal"));
            }
        }
    }
    findings
}

/// The ten rehearsals of `fake.rs` (note 51 stage 2c, §1.6; the tenth is the Stop's late N2, fix 1), counted in the final text. `outside` counts the
/// assert-family macros the bodies keep beside their wrappers: p70's cross-trial comparison with its `else` branch's `panic!`, and the late N2's `assert_eq!`
/// that N2's drop is there (the held tail's two deliberately late cases are wrapped sites too, with a `Lateness` less C's own late drop); `calls` counts `*_verdict(` and `*_outcome(` calls beside the wrappers: none.
const REHEARSALS: &[Site] = &[
    Site { test: "n47s5_front_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_long_close_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_repeat_switch_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_untimed_rehearsal_on_the_fake", sites: 2, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_held_tail_and_stop_rehearsal_on_the_fake", sites: 6, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_stop_n2_late_drop_is_host_latency_on_both_lines", sites: 1, direct: 0, outside: 1, calls: 0 },
    Site { test: "n47s5_stop_end_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "s1_short_first_sends_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 0, calls: 0 },
    Site { test: "p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake", sites: 1, direct: 0, outside: 2, calls: 0 },
];

#[test]
fn the_rehearsal_sites_are_wired() {
    let source = fs::read_to_string(workspace_root().join(FAKE)).unwrap();
    let found = rehearsal_findings(&source, REHEARSALS);
    assert!(found.is_empty(), "the rehearsals' wiring (note 51 stage 2c):\n{}", found.join("\n"));
}

#[cfg(test)]
mod rehearsal_wiring_tests {
    use super::*;

    const TRIAL: &str = "rehearsal.trial(\"a\", &t.lateness, || { let o = front_outcome(&t); assert!(!matches!(o, Verdict::Fail(_)), \"{o:?}\"); });";
    const TWO: Site = Site { test: "two", sites: 2, direct: 0, outside: 0, calls: 0 };

    /// A test `two` with `trials` wrapped sites, `before` between its setup and the sites and `tail` after them.
    fn source(trials: usize, before: &str, tail: &str) -> String {
        format!("fn two() {{\n let mut rehearsal = Rehearsal::new();\n let t = trial();\n {before}\n {}\n {tail}\n}}\nfn other() {{ rehearsal.trial(\"x\", &l, || {{}}); }}\n", TRIAL.repeat(trials))
    }

    fn findings(source: &str, table: &[Site]) -> Vec<String> {
        rehearsal_findings(source, table)
    }

    fn one_finding(source: &str, table: &[Site], needle: &str) {
        let found = findings(source, table);
        assert!(found.len() == 1 && found[0].contains(needle), "wanted one finding with {needle:?}, got {found:?}");
    }

    #[test]
    fn rehearsal_findings_flags_a_site_removed_and_a_site_added() {
        assert_eq!(findings(&source(2, "", "rehearsal.finish();"), &[TWO]), Vec::<String>::new(), "a conforming source, with a `.trial(` in another fn");
        one_finding(&source(1, "", "rehearsal.finish();"), &[TWO], "sites 1 of 2");
        one_finding(&source(3, "", "rehearsal.finish();"), &[TWO], "sites 3 of 2");
        let table = [Site { sites: 6, direct: 0, outside: 0, calls: 0, test: "held" }];
        let held = |trials: usize, n7: &str| format!("fn held() {{ let mut rehearsal = Rehearsal::new(); {} {n7} rehearsal.finish(); }}", TRIAL.repeat(trials));
        assert_eq!(findings(&held(6, ""), &table), Vec::<String>::new(), "the held-tail shape: six wrapped sites, the two late cases among them");
        let n7 = "let o = held_refused_verdict(&late); assert!(matches!(o, Verdict::Inconclusive(_)));";
        let found = findings(&held(5, n7), &table);
        assert!(found.iter().any(|text| text.contains("sites 5 of 6")), "an N7 case left unwrapped is a finding: {found:?}");
        assert!(found.iter().any(|text| text.contains("Verdict::Inconclusive outside the spans 1 of 0")), "{found:?}");
        assert!(found.iter().any(|text| text.contains("outside 1 of 0")) && found.iter().any(|text| text.contains("calls 1 of 0")), "{found:?}");
    }

    #[test]
    fn rehearsal_findings_flags_finish_and_new() {
        one_finding(&source(2, "", ""), &[TWO], ".finish() found 0 of 1");
        one_finding(&source(2, "", "rehearsal.finish(); rehearsal.finish();"), &[TWO], ".finish() found 2 of 1");
        let early = format!("fn two() {{ let mut rehearsal = Rehearsal::new(); let t = trial(); {TRIAL} rehearsal.finish(); {TRIAL} }}");
        one_finding(&early, &[TWO], "not after the last .trial(");
        one_finding(&source(2, "", "rehearsal.finish();").replace("Rehearsal::new()", "Default::default()"), &[TWO], "Rehearsal::new() found 0 of 1");
        one_finding(&source(2, "let again = Rehearsal::new();", "rehearsal.finish();"), &[TWO], "Rehearsal::new() found 2 of 1");
    }

    #[test]
    fn rehearsal_findings_ignores_comments_and_strings_and_checks_the_left_alone_tests() {
        let one = Site { sites: 1, ..TWO };
        assert_eq!(findings(&source(1, "// rehearsal.trial(\"c\", &t, || {});\n let s = \"rehearsal.trial(\";", "rehearsal.finish();"), &[one]), Vec::<String>::new(), "text is not a site");
        let only_text = "fn two() { let mut rehearsal = Rehearsal::new(); // rehearsal.trial(\"c\", &t, || {});\n let s = \"rehearsal.trial(\"; rehearsal.finish(); }";
        one_finding(only_text, &[Site { sites: 1, ..TWO }], "sites 0 of 1");
        let left = format!("fn {}() {{ let mut rehearsal = Rehearsal::new(); }}", LEFT_ALONE[1]);
        let found = findings(&left, &[]);
        assert!(found.len() == 1 && found[0].contains(LEFT_ALONE[1]), "{found:?}");
        assert!(findings(&format!("fn {}() {{ let x = 1; }}", LEFT_ALONE[1]), &[]).is_empty(), "a left-alone test without Rehearsal is fine");
    }

    #[test]
    fn rehearsal_findings_counts_asserts_outside_the_spans() {
        let finish = "rehearsal.finish();";
        assert!(findings(&source(2, "", finish), &[TWO]).is_empty(), "the asserts inside the spans are not counted as outside");
        one_finding(&source(2, "assert!(!matches!(o, Verdict::Fail(_)));", finish), &[TWO], "outside 1 of 0");
        for text in ["assert_eq!(a, b);", "assert_ne!(a, b);", "panic!(\"x\");", "unreachable!();"] {
            one_finding(&source(2, text, finish), &[TWO], "outside 1 of 0");
        }
        one_finding(&source(2, "assert!(a);", finish), &[Site { outside: 2, ..TWO }], "outside 1 of 2");
        assert!(findings(&source(2, "assert!(a); assert!(b);", finish), &[Site { outside: 2, ..TWO }]).is_empty());
        one_finding(&source(2, "", finish), &[Site { outside: 1, ..TWO }], "outside 0 of 1");
        assert!(findings(&source(2, "if let [a, b] = trials[..] { assert_eq!(a.1, b.1) }", finish), &[Site { outside: 1, ..TWO }]).is_empty());
    }

    #[test]
    fn rehearsal_findings_ignores_assert_names_in_comments_and_strings() {
        let before = "// assert!(x);\n let s = \"panic!(...)\"; /* assert_eq!(a, b) */ let r = r#\"unreachable!()\"#;";
        assert!(findings(&source(2, before, "rehearsal.finish();"), &[TWO]).is_empty());
    }

    #[test]
    fn rehearsal_findings_matches_names_on_identifier_boundaries() {
        let finish = "rehearsal.finish();";
        assert!(findings(&source(2, "debug_assert!(x);", finish), &[Site { outside: 1, ..TWO }]).is_empty(), "debug_assert! is a name of its own, counted once");
        one_finding(&source(2, "debug_assert!(x);", finish), &[TWO], "outside 1 of 0");
        for text in ["my_assert!(x);", "reassert!(x);", "r.assert!(x);", "assert_these!(x);"] {
            assert!(findings(&source(2, text, finish), &[TWO]).is_empty(), "{text} is no assert-family macro");
        }
    }

    #[test]
    fn rehearsal_findings_counts_verdict_calls_outside_the_spans() {
        one_finding(&source(2, "let o = front_outcome(&t);", "rehearsal.finish();"), &[TWO], "calls 1 of 0");
        one_finding(&source(2, "let o = held_refused_verdict(&t);", "rehearsal.finish();"), &[TWO], "calls 1 of 0");
        assert!(findings(&source(2, "let o = held_played_verdict(&t, 2);", "rehearsal.finish();"), &[Site { calls: 1, ..TWO }]).is_empty());
        one_finding(&source(2, "", "rehearsal.finish();"), &[Site { calls: 1, ..TWO }], "calls 0 of 1");
        assert!(findings(&source(2, "let o = a_verdict_helper(&t); let p = trial_run(&t);", "rehearsal.finish();"), &[TWO]).is_empty(), "only names ending in _verdict or _outcome");
    }
}
