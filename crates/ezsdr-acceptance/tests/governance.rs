use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("acceptance crate has a workspace root")
        .to_path_buf()
}

fn cargo_metadata() -> Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root())
        .output()
        .expect("run cargo metadata");
    assert!(output.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn workspace_packages(metadata: &Value) -> BTreeSet<String> {
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .map(|package| {
            format!(
                "{} {}",
                package["name"].as_str().expect("package name"),
                package["version"].as_str().expect("package version")
            )
        })
        .collect()
}

#[test]
fn po_02_every_crate_forbids_unsafe_code() {
    let crates = workspace_root().join("crates");
    let mut members: Vec<_> = fs::read_dir(crates)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .collect();
    members.sort_by_key(|entry| entry.file_name());
    assert!(!members.is_empty());
    for member in members {
        let name = member.file_name();
        let name = name.to_string_lossy();
        let crate_dir = member.path();
        let lib = fs::read_to_string(crate_dir.join("src/lib.rs"))
            .unwrap_or_else(|error| panic!("{name}/src/lib.rs: {error}"));
        assert!(lib.contains("#![forbid(unsafe_code)]"), "{name} must forbid unsafe code");
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
    let baseline: BTreeSet<String> = fs::read_to_string(
        root.join("crates/ezsdr-acceptance/tests/baseline_external_packages.txt"),
    )
    .unwrap()
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_owned)
    .collect();
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    for package in lock.split("[[package]]").skip(1) {
        let name = package
            .lines()
            .find_map(|line| line.trim().strip_prefix("name = \""))
            .and_then(|line| line.strip_suffix('\"'))
            .expect("external lock entry has a name");
        let version = package
            .lines()
            .find_map(|line| line.trim().strip_prefix("version = \""))
            .and_then(|line| line.strip_suffix('\"'))
            .expect("lock entry has a version");
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
    let modules = BTreeSet::from([
        "ezsdr-sim-engine",
        "ezsdr-mock-radio",
        "ezsdr-link-host",
        "ezsdr-sink-capture",
        "ezsdr-exec-native",
    ]);
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
        ("ezsdr-acceptance", BTreeSet::from([
            "ezsdr-kernel", "ezsdr-radio", "ezsdr-sim", "ezsdr-sink", "ezsdr-hostmem",
            "ezsdr-sim-engine", "ezsdr-mock-radio", "ezsdr-link-host", "ezsdr-sink-capture",
            "serde_json",
        ])),
    ]);
    for package in metadata["packages"].as_array().expect("metadata packages") {
        let package_name = package["name"].as_str().unwrap();
        let mut normal = BTreeSet::new();
        let mut dev = BTreeSet::new();
        for dependency in package["dependencies"].as_array().expect("package dependencies") {
            let dependency_name = dependency["name"].as_str().unwrap();
            match dependency["kind"].as_str() {
                None => { normal.insert(dependency_name); }
                Some("dev") => {
                    dev.insert(dependency_name);
                    if dependency_name == "ezsdr-kernel" {
                        let features: BTreeSet<_> = dependency["features"].as_array().unwrap().iter()
                            .map(|feature| feature.as_str().unwrap()).collect();
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
    const FORBIDDEN: [&str; 6] = [
        "HashMap",
        "HashSet",
        "SystemTime",
        "Instant::now",
        "thread::spawn",
        "rand::",
    ];
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
    ];
    let mut source_roots: Vec<PathBuf> = new_crates
        .iter()
        .map(|name| root.join("crates").join(name).join("src"))
        .collect();
    source_roots.push(root.join("crates/ezsdr-kernel/src/coordinator"));
    for source_root in source_roots {
        for file in rust_files(&source_root) {
            let source = fs::read_to_string(&file).unwrap();
            for (line_number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or_default();
                for forbidden in FORBIDDEN {
                    assert!(
                        !code.contains(forbidden),
                        "{}:{} contains {forbidden}",
                        file.display(),
                        line_number + 1
                    );
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
