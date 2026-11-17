use std::path::{Path, PathBuf};

use ezsdr_mock_radio::BLOCK_LEN;
use ezsdr_radio::channels::{self, Description};
use ezsdr_radio::measurement::{self, Record};
use ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN;
const FIXTURE: &str = include_str!("../../ezsdr-radio/tests/fixtures/measurement/records/valid.json");

fn descriptions(record: &Record) -> (Description, Description) {
    let layout = channels::from_selector(&record.selector).expect("measurement selector is a valid x3xx layout");
    (Description::new(&layout, BLOCK_LEN, &ezsdr_mock_radio::Profile::terms()), Description::new(&layout, DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()))
}

struct FileComparison {
    file: PathBuf,
    mock_violations: Vec<measurement::Violation>,
    uhd_violations: Vec<measurement::Violation>,
}

fn records_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ezsdr-radio/measurements")
}

fn compare_files(directory: &Path) -> Vec<FileComparison> {
    if !directory.is_dir() {
        return Vec::new();
    }
    let mut files = std::fs::read_dir(directory)
        .expect("the Radio Model measurements directory is readable")
        .map(|entry| entry.expect("a measurement directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect::<Vec<_>>();
    files.sort();
    let mut comparisons = Vec::with_capacity(files.len());
    for file in files {
        let bytes = std::fs::read(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        let record: Record = serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{}: invalid measurement record: {error}", file.display()));
        let (mock, uhd) = descriptions(&record);
        comparisons.push(FileComparison { file, mock_violations: measurement::compare(&record, &mock), uhd_violations: measurement::compare(&record, &uhd) });
    }
    comparisons
}

fn check_comparison(comparison: &FileComparison) -> Result<(), String> {
    if !comparison.mock_violations.is_empty() {
        return Err(format!("{} MockRadio: {:?}", comparison.file.display(), comparison.mock_violations));
    }
    if !comparison.uhd_violations.is_empty() {
        return Err(format!("{} UHD Module: {:?}", comparison.file.display(), comparison.uhd_violations));
    }
    Ok(())
}

fn assert_files_pass(directory: &Path) {
    for comparison in compare_files(directory) {
        let result = check_comparison(&comparison);
        assert!(result.is_ok(), "{result:?}");
    }
}

#[test]
fn measurement_files_compare_against_mock_and_uhd_descriptions() {
    assert_files_pass(&records_dir());
}

#[test]
fn measurement_fixture_files_enforce_provider_specific_rejections() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ezsdr-radio/tests/fixtures/measurement/records");
    let comparisons = compare_files(&directory);
    assert_eq!(comparisons.len(), 3);

    let find = |name: &str| comparisons.iter().find(|item| item.file.file_name().is_some_and(|file| file == name)).unwrap();
    let both = find("both-reject.json");
    for (provider, violations) in [("MockRadio", &both.mock_violations), ("UHD", &both.uhd_violations)] {
        for (key, reason) in [("radio.timing.min_timed_command_lead_ns", "below measured"), ("radio.perf.rx_bytes_per_s", "exceeds measured")] {
            assert!(violations.iter().any(|violation| violation.key == key && violation.reason.contains(reason)), "{provider} must reject {key}: {violations:?}");
        }
    }
    let both_gate = check_comparison(both);
    assert!(matches!(both_gate.as_ref(), Err(error) if error.contains("MockRadio")), "the shared gate must report MockRadio first: {both_gate:?}");

    let uhd_only = find("uhd-only-rejects.json");
    assert!(uhd_only.mock_violations.is_empty(), "MockRadio must accept the 300 MSample repeat maximum: {:?}", uhd_only.mock_violations);
    assert!(uhd_only.uhd_violations.iter().any(|violation| violation.key == "radio.tx.repeat_max_samples"), "UHD must reject the 300 MSample repeat maximum: {:?}", uhd_only.uhd_violations);
    let uhd_gate = check_comparison(uhd_only);
    assert!(matches!(uhd_gate.as_ref(), Err(error) if error.contains("UHD Module")), "the shared gate must reject UHD's fixture: {uhd_gate:?}");

    let valid = find("valid.json");
    assert!(valid.mock_violations.is_empty(), "MockRadio rejects the passing fixture: {:?}", valid.mock_violations);
    assert!(valid.uhd_violations.is_empty(), "UHD rejects the passing fixture: {:?}", valid.uhd_violations);
    let valid_gate = check_comparison(valid);
    assert!(valid_gate.is_ok(), "the shared gate must accept the passing fixture: {valid_gate:?}");
}

#[test]
fn fake_source_is_rejected_by_both_providers_only_for_source() {
    let mut record: Record = serde_json::from_str(FIXTURE).unwrap();
    record.source = measurement::Source::Fake;
    let (mock, uhd) = descriptions(&record);
    for (provider, description) in [("MockRadio", &mock), ("UHD", &uhd)] {
        let violations = measurement::compare(&record, description);
        assert_eq!(violations.len(), 1, "{provider}: {violations:?}");
        assert_eq!(violations[0].key, "source", "{provider}: {violations:?}");
    }
}
