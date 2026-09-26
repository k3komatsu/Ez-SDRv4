mod support;

use std::collections::BTreeMap;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::event::EventKind;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{CleanupMode, RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::TimePoint;
use serde_json::json;
use support::{artifact, end_session, finish_at, root, section, session_run, spec_run, T0};

fn complete(spec: &serde_json::Value, profile_name: &str, selector: serde_json::Value, environment: serde_json::Value, test: &str, horizon: i64) -> Manifest {
    let temp = rig::TempDir::new(test);
    let profile = rig::spec_profile(profile_name, selector, &temp.0, environment);
    let run = spec_run(&temp, spec, &profile, Default::default());
    let clock = root(&run);
    finish_at(run, clock, T0 + horizon)
}

#[test]
fn v58_01_one_spec_two_mock_profiles() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(10_000));
    let left = complete(&spec, "x310-like", json!({ "id": "mock" }), json!({}), "v58-01-x310", 20_000_000);
    let right = complete(&spec, "ideal", json!({ "id": "mock" }), json!({}), "v58-01-ideal", 20_000_000);
    assert_eq!(left.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(right.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(left.artifacts.len(), 1);
    assert_eq!(right.artifacts.len(), 1);
    assert_eq!(artifact(&left, "rec").size_bytes, 80_000);
    assert_eq!(artifact(&right, "rec").size_bytes, 80_000);
    assert!(!artifact(&left, "rec").partial);
    assert!(!artifact(&right, "rec").partial);
    assert_eq!(left.spec.body, right.spec.body);
}

#[test]
fn v58_02_ten_virtual_seconds_run_faster_than_wall_clock() {
    let temp = rig::TempDir::new("v58-02");
    let spec = experiments::receive(1, 1.0e6, 1.0e9, None);
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = spec_run(&temp, &spec, &profile, Default::default());
    let clock = root(&run);
    let began = std::time::Instant::now();
    run.run_until_end(ezsdr_kernel::time::TimePoint::new(clock, T0 + 10_000_000_000)).unwrap();
    assert!(began.elapsed().as_secs() < 10);
    assert!(run.now().ticks >= T0 + 10_000_000_000);
    let manifest = run.finish();
    let stats = section(&manifest, "ezsdr.radio.mock.mock.stats");
    let samples = stats["rx_samples"].as_u64().unwrap();
    assert!((10_000_000..=10_010_000).contains(&samples));
}

#[test]
fn v58_10_experiments_name_no_mock_type() {
    let source = include_str!("../src/experiments.rs");
    for forbidden in ["mock", "Mock", "x310", "ideal", "sim-engine", "sim_engine", "ezsdr_radio_mock"] {
        assert!(!source.contains(forbidden), "found forbidden name {forbidden}");
    }
    for line in source.lines().filter(|line| line.trim_start().starts_with("use ")) {
        assert!(
            ["use ezsdr_kernel", "use serde_json", "use std"].iter().any(|prefix| line.trim_start().starts_with(prefix)),
            "unexpected import: {line}"
        );
    }
}

#[test]
fn v58_03_same_seed_same_manifest_projection() {
    let temp = rig::TempDir::new("v58-03-same");
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(20_000));
    let selector = json!({ "id": "mock", "block_len_jitter": true, "rx_test_pattern": "ramp" });
    let first_profile = rig::spec_profile("x310-like", selector.clone(), &temp.0, json!({ "sim.seed": 7 }));
    let mut first = spec_run(&temp, &spec, &first_profile, BTreeMap::new());
    let clock = root(&first);
    first.advance_to(TimePoint::new(clock, T0 + 25_000_000)).unwrap();
    let first = first.finish();
    let second_profile = rig::spec_profile("x310-like", selector, &temp.0, json!({ "sim.seed": 7 }));
    let mut second = spec_run(&temp, &spec, &second_profile, BTreeMap::new());
    let clock = root(&second);
    second.advance_to(TimePoint::new(clock, T0 + 25_000_000)).unwrap();
    let second = second.finish();
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
}

#[test]
fn v58_03_the_seed_changes_block_boundaries_not_data() {
    let temp = rig::TempDir::new("v58-03-seeds");
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(20_000));
    let selector = json!({ "id": "mock", "block_len_jitter": true, "rx_test_pattern": "ramp" });
    let run_seed = |seed| {
        let profile = rig::spec_profile("x310-like", selector.clone(), &temp.0, json!({ "sim.seed": seed }));
        let mut run = spec_run(&temp, &spec, &profile, BTreeMap::new());
        let clock = root(&run);
        run.advance_to(TimePoint::new(clock, T0 + 25_000_000)).unwrap();
        run.finish()
    };
    let seven = run_seed(7);
    let eight = run_seed(8);
    assert_eq!(artifact(&seven, "rec").hash, artifact(&eight, "rec").hash);
    assert_eq!(section(&seven, "ezsdr.radio.mock.mock.stats")["rx_blocks"], 12);
    assert_eq!(section(&eight, "ezsdr.radio.mock.mock.stats")["rx_blocks"], 11);
}

#[test]
fn v58_04_mock_events_reach_counters_policy_and_manifest() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    let manifest = complete(&spec, "x310-like", json!({ "id": "mock" }), faults.clone(), "v58-04", 160_000_000);
    let rows = &manifest.events.counters;
    assert_eq!(rows.iter().find(|row| row.source.path == "mock/rx" && row.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap().count, 1);
    let events: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["cause"], "overrun");
    assert_eq!(artifact(&manifest, "rec").marks.len(), 1);
    assert_eq!(artifact(&manifest, "rec").marks[0].kind.as_str(), ezsdr_radio::kinds::RX_OVERFLOW);

    let mut stopped = spec.clone();
    stopped["policies"]["failure"] = json!({ "radio.RX_OVERFLOW": "stop" });
    let temp = rig::TempDir::new("v58-04-stop");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, faults);
    let run = spec_run(&temp, &stopped, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 160_000_000);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: EventKind::parse("radio.RX_OVERFLOW").unwrap() } });
    assert!(manifest.run.transitions.iter().any(|entry| entry.state == RunState::Stopping { mode: CleanupMode::Orderly }));
}

#[test]
fn v58_05_device_lost_aborts_with_full_cleanup() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 5_000_000, "fault": "device_lost", "target": "radio" }] });
    let manifest = complete(&spec, "x310-like", json!({ "id": "mock" }), faults, "v58-05", 10_000_000);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
    assert!(manifest.run.transitions.iter().any(|entry| entry.state == RunState::Stopping { mode: CleanupMode::Abort }));
    assert!(manifest.termination.cleanup_failures.is_empty());
    assert!(artifact(&manifest, "rec").partial);
    assert_eq!(artifact(&manifest, "rec").size_bytes, 32_000);
    assert!(manifest.hash.is_some());
}

#[test]
fn v58_06_injected_overflow_is_a_uhd_overflow() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    let manifest = complete(&spec, "x310-like", json!({ "id": "mock" }), faults, "v58-06-overflow", 160_000_000);
    let capture = artifact(&manifest, "rec");
    assert_eq!(capture.size_bytes, 800_000);
    assert_eq!(capture.continuity[0].gaps.len(), 1);
    let gap = &capture.continuity[0].gaps[0];
    assert_eq!(gap.cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
    assert_eq!(gap.start.ticks, 1_000);
    assert_eq!(gap.len, 50_000);
    assert_eq!(gap.lost, Some(50_000));
    assert_eq!(gap.link_dropped, 0);
    let event = manifest.events.delivered.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap();
    assert_eq!(event.payload, json!({ "cause": "overrun", "lost": 50_000, "restart_gap_ns": 50_000_000 }));
}

#[test]
fn v58_06_sequence_error_is_seq_discontinuity() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" }] });
    let manifest = complete(&spec, "x310-like", json!({ "id": "mock" }), faults, "v58-06-seq", 160_000_000);
    let gaps = &artifact(&manifest, "rec").continuity[0].gaps;
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].cause, ezsdr_kernel::stream::GapCause::SequenceError {});
    assert_eq!(gaps[0].start.ticks, 1_000);
    assert_eq!(gaps[0].len, 2_000);
    assert_eq!(gaps[0].lost, Some(2_000));
}

#[test]
fn v58_06_overflows_inside_a_gap_extend_the_recorded_loss() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [
        { "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" },
        { "at_ns": 10_000_000, "fault": "rx_overflow", "target": "radio" }
    ] });
    let manifest = complete(&spec, "x310-like", json!({ "id": "mock" }), faults, "v58-06-overlap", 160_000_000);
    let gap = &artifact(&manifest, "rec").continuity[0].gaps[0];
    assert_eq!(gap.cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
    assert_eq!(gap.start.ticks, 1_000);
    assert_eq!(gap.len, 59_000);
    assert_eq!(gap.lost, Some(59_000));
}

#[test]
fn v58_07_manifest_records_every_input_and_output() {
    let temp = rig::TempDir::new("v58-07");
    let (bytes, waveform) = experiments::waveform(1_000);
    let spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 10_000, Some(10_000));
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 20_000_000);
    assert_eq!(manifest.inputs, vec![waveform]);
    assert_eq!(manifest.artifacts.len(), 1);
    for module in ["ezsdr.sim-engine", "ezsdr.radio.mock", "ezsdr.link.host", "ezsdr.sink.capture"] {
        assert!(support::modules(&manifest).contains(&module));
    }
    for vocabulary in ["radio", "sim", "sink"] { assert!(manifest.vocabularies.keys().any(|id| id.as_str() == vocabulary)); }
    for stream in ["mock/rx", "mock/tx"] { assert!(manifest.clocks.sample_clocks.iter().any(|clock| clock.stream.path == stream)); }
    for name in ["ezsdr.links", "ezsdr.radio.mock.mock.envelope", "ezsdr.radio.mock.mock.bursts", "ezsdr.radio.mock.mock.faults", "ezsdr.radio.mock.mock.rejected", "ezsdr.radio.mock.mock.stats", "ezsdr.radio.mock.mock.applied"] {
        assert!(manifest.sections.contains_key(&ezsdr_kernel::spec::Namespace::parse(name).unwrap()), "missing section {name}");
        // MR-27: a MockRadio section is under its instance's own id, so a Run with two
        // Mocks keeps both sets; the unqualified name must be gone.
        if name.starts_with("ezsdr.radio.mock.") {
            let unqualified = name.replace(".mock.", ".");
            assert!(!manifest.sections.contains_key(&ezsdr_kernel::spec::Namespace::parse(&unqualified).unwrap()), "{unqualified} must not exist");
        }
    }
    assert_eq!(manifest.spec.body, spec);
    assert_eq!(manifest.binding.body, profile);
}

#[test]
fn v58_11_short_lead_burst_is_a_time_error() {
    let temp = rig::TempDir::new("v58-11-short");
    let profile = rig::session_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: Value::Int(1) }, None).unwrap();
    run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: Some(TimePoint::new(clock, T0 + 2_000_000)), params: Default::default() }, Some(&bytes)).unwrap();
    let manifest = end_session(run, clock, T0 + 10_000_000);
    let events: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["cause"], "late");
    assert_eq!(events[0].payload["outcome"], "send_asap");
    assert_eq!(events[0].payload["late_by_ns"], 1_000_000);
    assert_eq!(events[0].payload["target"]["ticks"], 1_000);
    let burst: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.mock.bursts")[0].clone()).unwrap();
    assert_eq!(burst.late_by.unwrap().ticks, 1_000_000);
}

#[test]
fn v58_11_19_5_msps_is_coerced_to_20() {
    let mut spec = experiments::receive(1, 19.5e6, 1.0e9, Some(1_000));
    spec["policies"]["coercion"] = json!({ "radio.rx.sample_rate_hz": "warn" });
    let temp = rig::TempDir::new("v58-11-coerce");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 1_000_000);
    let preview = manifest.admission.coercions_preview.iter().find(|item| item.coercion.key.as_str() == "radio.rx.sample_rate_hz").unwrap();
    assert_eq!(preview.coercion.requested, Value::Num(19.5e6));
    assert_eq!(preview.coercion.applied, Value::Num(20.0e6));
    assert_eq!(manifest.prepare.reports[0].effective[&Key::parse("radio.rx.sample_rate_hz").unwrap()], Value::Num(20.0e6));
    let rx = manifest.clocks.sample_clocks.iter().find(|clock| clock.stream.path == "mock/rx").unwrap();
    assert_eq!((rx.nominal_rate.num(), rx.nominal_rate.den()), (20_000_000, 1));

    let refused_spec = experiments::receive(1, 19.5e6, 1.0e9, Some(1_000));
    let run = spec_run(&temp, &refused_spec, &profile, BTreeMap::new());
    let refused = run.finish();
    assert_eq!(refused.termination.reason, Termination::Failed { stage: Stage::Validate });
}

#[test]
fn v58_11_beyond_the_performance_envelope_is_rejected_at_validate() {
    let spec = experiments::receive(2, 200.0e6, 1.0e9, None);
    let temp = rig::TempDir::new("v58-11-envelope");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let manifest = run.finish();
    assert_eq!(manifest.termination.reason, Termination::Failed { stage: Stage::Validate });
    let rejected = manifest.admission.rejected.iter().find(|item| item.key.as_str() == "radio.rx.sample_rate_hz").unwrap();
    assert!(rejected.reason.starts_with("RM-7"));
}

#[test]
fn v58_12_jitter_leaves_the_capture_unchanged() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(12_345));
    let selector = |jitter| json!({ "id": "mock", "block_len_jitter": jitter, "rx_test_pattern": "ramp" });
    let plain = complete(&spec, "x310-like", selector(false), json!({ "sim.seed": 7 }), "v58-12-plain", 20_000_000);
    let jitter = complete(&spec, "x310-like", selector(true), json!({ "sim.seed": 7 }), "v58-12-jitter", 20_000_000);
    assert_eq!(artifact(&plain, "rec").hash, artifact(&jitter, "rec").hash);
    assert_eq!(artifact(&plain, "rec").continuity, artifact(&jitter, "rec").continuity);
    assert_ne!(section(&plain, "ezsdr.radio.mock.mock.stats")["rx_blocks"], section(&jitter, "ezsdr.radio.mock.mock.stats")["rx_blocks"]);
}

#[test]
fn v58_13_session_manifest_has_log_waveform_and_capture() {
    let temp = rig::TempDir::new("v58-13");
    let profile = rig::session_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    let a = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: Value::Int(1) }, None).unwrap();
    let b = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: Default::default() }, Some(&bytes)).unwrap();
    let c = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: ResourceId::parse("rec").unwrap(), at: None, params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::Int(5_000))]) }, None).unwrap();
    assert_eq!([a.seq, b.seq, c.seq], [0, 1, 2]);
    assert!([a, b, c].iter().all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
    let manifest = end_session(run, clock, T0 + 20_000_000);
    assert_eq!(manifest.action_log.len(), 3);
    assert_eq!(manifest.inputs.len(), 1);
    assert_eq!(artifact(&manifest, "rec_0").size_bytes, 40_000);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].end, ezsdr_kernel::stream::BurstEnd::Stop);
}

#[test]
fn v58_14_profiles_differing_only_in_environment_both_run() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(10_000));
    let base = complete(&spec, "x310-like", json!({ "id": "mock" }), json!({ "sim.seed": 1 }), "v58-14-a", 20_000_000);
    let other = complete(&spec, "x310-like", json!({ "id": "mock" }), json!({ "sim.seed": 2, "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 0.0, "hi_hz": 7.0e9 }] } }), "v58-14-b", 20_000_000);
    assert_eq!(base.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(other.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(base.spec.body, other.spec.body);
    assert_eq!(base.artifacts.len(), 1);
    assert_eq!(other.artifacts.len(), 1);
}

#[test]
fn v58_15_repeat_wraps_without_a_gap() {
    let (bytes, waveform) = experiments::waveform(1_000);
    let spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 10_000, None);
    let temp = rig::TempDir::new("v58-15");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash, bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 15_000_000);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert!(bursts[0].wraps >= 4);
    assert!(bursts[0].samples >= 4_000);
    assert_eq!(bursts[0].end, ezsdr_kernel::stream::BurstEnd::Stop);
    assert!(!manifest.events.delivered.iter().any(|event| [ezsdr_radio::kinds::TIME_ERROR, ezsdr_radio::kinds::TX_UNDERFLOW].contains(&event.kind.as_str())));
}

#[test]
fn v58_16_runtime_retune_outside_the_rf_envelope_is_rejected() {
    let temp = rig::TempDir::new("v58-16-rf");
    let profile = rig::session_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 2.4e9, "hi_hz": 2.5e9 }] } }));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let submit = |run: &mut ezsdr_kernel::coordinator::RunHandle, key: &str, value| run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse(key).unwrap(), value }, None).unwrap();
    let a = submit(&mut run, "radio.tx.frequency_hz", Value::Num(2.45e9));
    let b = submit(&mut run, "radio.tx.channels", Value::Int(1));
    let c = submit(&mut run, "radio.tx.frequency_hz", Value::Num(2.6e9));
    assert!(matches!(a.outcome, Outcome::Admitted { .. }));
    assert!(matches!(b.outcome, Outcome::Admitted { .. }));
    let Outcome::Rejected { violations } = c.outcome else { panic!("outside band was admitted") };
    assert!(violations.iter().any(|violation| violation.check.as_str() == "radio.rf_envelope" && violation.key.as_ref().is_some_and(|key| key.as_str() == "radio.tx.frequency_hz")));
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.tx.frequency_hz").unwrap()], Value::Num(2.45e9));
    end_session(run, clock, T0 + 5_000_000);
}

#[test]
fn kc_24_a_burst_after_the_transmit_clock_ends_is_refused_at_admission() {
    let temp = rig::TempDir::new("kc-24-ended-tx");
    let profile = rig::session_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    for channels in [1, 0] {
        let entry = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: Value::Int(channels) }, None).unwrap();
        assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    }
    let (bytes, _) = experiments::waveform(1_000);
    let entry = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: Default::default() }, Some(&bytes)).unwrap();
    let Outcome::Rejected { violations } = entry.outcome else { panic!("a burst on an ended transmit clock was admitted") };
    assert!(violations.iter().any(|violation| violation.reason.starts_with("SC-23: ") && violation.reason.ends_with("has no running transmit SampleClock")));
    let manifest = end_session(run, clock, T0 + 5_000_000);
    assert_eq!(section(&manifest, "ezsdr.radio.mock.mock.rejected").as_array().unwrap().len(), 0);
}

#[test]
fn v58_16_runtime_rate_beyond_the_envelope_is_rejected() {
    let temp = rig::TempDir::new("v58-16-rate");
    let profile = rig::session_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let a = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.rx.channels").unwrap(), value: Value::Int(2) }, None).unwrap();
    let prior = run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.rx.sample_rate_hz").unwrap()].clone();
    let b = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.rx.sample_rate_hz").unwrap(), value: Value::Num(200.0e6) }, None).unwrap();
    assert!(matches!(a.outcome, Outcome::Admitted { .. }));
    let Outcome::Rejected { violations } = b.outcome else { panic!("over-envelope rate was admitted") };
    assert!(violations.iter().any(|violation| violation.check.as_str() == "ezsdr.coercion" && violation.reason.contains("RM-7")));
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.rx.sample_rate_hz").unwrap()], prior);
    end_session(run, clock, T0 + 5_000_000);
}

// ---------------------------------------------------------------- Phase 3: the SimulationChannel

fn ramp(len: usize) -> Vec<(f32, f32)> {
    (0..len).map(|n| ((n + 1) as f32 / 4096.0, -((n + 1) as f32) / 8192.0)).collect()
}

fn link_run(temp: &rig::TempDir, tx: &str, rx: &str, profile: &str, rx_jitter: bool, environment: serde_json::Value, samples: &[(f32, f32)]) -> (Manifest, Vec<(f32, f32)>) {
    let (bytes, waveform) = experiments::waveform_of(samples);
    let spec = experiments::link(tx, rx, 1.0e6, &waveform, 10_000, 20_000);
    let profile = rig::link_profile(profile, tx, rx, rx_jitter, &temp.0, environment);
    let run = spec_run(temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 25_000_000);
    let capture = rig::read_capture(artifact(&manifest, "rec"), 1).remove(0);
    (manifest, capture)
}

fn coupling(tx: &str, rx: &str, gain_db: f64, delay_ns: u64, noise_dbfs: Option<f64>, seed: u64) -> serde_json::Value {
    let mut channel = json!({ "couplings": [{ "tx": tx, "tx_channel": 0, "rx": rx, "rx_channel": 0, "gain_db": gain_db, "delay_ns": delay_ns }] });
    if let Some(noise) = noise_dbfs {
        channel["noise_dbfs"] = json!({ rx: noise });
    }
    json!({ "sim.seed": seed, "sim.channel": channel })
}

#[test]
fn v58_08_two_mock_radios_communicate_through_the_channel() {
    let samples = ramp(3_000);
    let temp = rig::TempDir::new("v58-08");
    let (manifest, capture) = link_run(&temp, "a", "b", "ideal", false, coupling("a", "b", -6.0, 1_000, None, 0), &samples);
    assert!(!serde_json::to_string(&manifest.spec.body).unwrap().contains("sim."), "the Spec carries no channel");
    assert_eq!(manifest.run.fidelity.rf, ezsdr_kernel::module_api::RfFidelity::ImpairmentModel);
    assert_eq!(capture.len(), 20_000);
    let gain = 10f64.powf(-6.0 / 20.0);
    for (k, (re, im)) in capture.iter().enumerate() {
        let n = k as i64 - 10_001;
        let expected = if (0..3_000).contains(&n) {
            let (wr, wi) = samples[n as usize];
            ((gain * f64::from(wr)) as f32, (gain * f64::from(wi)) as f32)
        } else {
            (0.0, 0.0)
        };
        assert_eq!((*re, *im), expected, "captured sample {k}");
    }
}

#[test]
fn v58_08_b_two_mocks_in_one_run_keep_both_sets_of_sections() {
    // MR-27: every MockRadio instance writes its six sections under its own id, because
    // `Manifest::write_section` inserts. With one shared name the last instance written won
    // and the other's records were silently gone, and *which* one won depended on the
    // selector sort, so it flipped with `block_len_jitter`.
    let temp = rig::TempDir::new("v58-08-sections");
    let (manifest, _) = link_run(
        &temp, "a", "b", "x310-like", false,
        coupling("a", "b", -6.0, 1_000, Some(-30.0), 0), &ramp(3_000),
    );
    for suffix in ["applied", "bursts", "envelope", "faults", "rejected", "stats"] {
        for id in ["dev_tx", "dev_rx"] {
            let name = format!("ezsdr.radio.mock.{id}.{suffix}");
            assert!(
                manifest.sections.contains_key(&Namespace::parse(&name).unwrap()),
                "{name} is missing: {sections:?}",
                sections = manifest.sections.keys().map(|n| n.as_str()).collect::<Vec<_>>()
            );
        }
    }
    // the two are genuinely different documents, not one written twice
    assert_eq!(section(&manifest, "ezsdr.radio.mock.dev_tx.stats")["rx_blocks"], 0);
    assert_ne!(section(&manifest, "ezsdr.radio.mock.dev_rx.stats")["rx_blocks"], 0);
    assert_eq!(section(&manifest, "ezsdr.radio.mock.dev_tx.bursts").as_array().unwrap().len(), 1, "SC-28: the transmitter's burst record");
    assert_eq!(section(&manifest, "ezsdr.radio.mock.dev_rx.bursts").as_array().unwrap().len(), 0, "the receiver transmits nothing");

    // and the winner does not depend on the selector: the other jitter value, same ids
    let (jitter, _) = link_run(
        &temp, "a", "b", "x310-like", true,
        coupling("a", "b", -6.0, 1_000, Some(-30.0), 0), &ramp(3_000),
    );
    assert_eq!(section(&jitter, "ezsdr.radio.mock.dev_tx.bursts").as_array().unwrap().len(), 1);
    assert_ne!(section(&jitter, "ezsdr.radio.mock.dev_rx.stats")["rx_blocks"], 0);
}

#[test]
fn kd_01_a_faulted_round_does_not_depend_on_fragment_names() {
    // Phase 3's Review C P2-1, now MA-30's rule (Phase 4, KD-1): a transmitter that
    // reports `device_lost` in the round where the receiver publishes its first block
    // must not decide, by sorting before it, whether that block was published.
    let run = |tx: &str, rx: &str| {
        let temp = rig::TempDir::new("kd-01");
        let (bytes, waveform) = experiments::waveform_of(&ramp(3_000));
        let spec = experiments::link(tx, rx, 1.0e6, &waveform, 10_000, 20_000);
        let mut environment = coupling(tx, rx, -6.0, 1_000, None, 0);
        environment["sim.faults"] = json!([{ "at_ns": 1_999_001, "fault": "device_lost", "target": tx }]);
        let profile = rig::link_profile("x310-like", tx, rx, false, &temp.0, environment);
        let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        let clock = root(&run);
        let manifest = finish_at(run, clock, T0 + 25_000_000);
        assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { kind: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
        let stats = section(&manifest, "ezsdr.radio.mock.dev_rx.stats");
        (stats["rx_blocks"].clone(), stats["rx_samples"].clone())
    };
    let transmitter_first = run("a", "b");
    let receiver_first = run("z", "a");
    assert_eq!(transmitter_first, receiver_first);
    assert_eq!(receiver_first, (json!(1), json!(2_000)));
}

#[test]
fn v58_08_without_a_channel_the_same_spec_hears_nothing() {
    let temp = rig::TempDir::new("v58-08-none");
    let (manifest, capture) = link_run(&temp, "a", "b", "ideal", false, json!({}), &ramp(3_000));
    assert_eq!(manifest.run.fidelity.rf, ezsdr_kernel::module_api::RfFidelity::None);
    assert_eq!(capture.len(), 20_000);
    assert!(capture.iter().all(|sample| *sample == (0.0, 0.0)));
}

#[test]
fn v58_03_channel_noise_reproduces_with_its_seed() {
    let samples = ramp(3_000);
    let temp = rig::TempDir::new("v58-03-noise");
    let noisy = |seed| link_run(&temp, "a", "b", "x310-like", false, coupling("a", "b", -6.0, 1_000, Some(-30.0), seed), &samples);
    let (first, first_capture) = noisy(7);
    let (second, _) = noisy(7);
    let (other, other_capture) = noisy(8);
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
    assert_ne!(artifact(&first, "rec").hash, artifact(&other, "rec").hash);
    let power = |capture: &[(f32, f32)]| capture[..10_000].iter().map(|(re, im)| f64::from(*re).powi(2) + f64::from(*im).powi(2)).sum::<f64>() / 10_000.0;
    for capture in [&first_capture, &other_capture] {
        assert!((power(capture) - 0.001).abs() < 0.0001, "noise power {}", power(capture));
    }
}

#[test]
fn v58_12_the_channel_output_does_not_depend_on_block_lengths() {
    let samples = ramp(3_000);
    let environment = coupling("a", "b", -6.0, 1_000, Some(-30.0), 7);
    let temp = rig::TempDir::new("v58-12-channel");
    let (plain, _) = link_run(&temp, "a", "b", "x310-like", false, environment.clone(), &samples);
    let (jitter, _) = link_run(&temp, "a", "b", "x310-like", true, environment, &samples);
    // §58 #12: the channel's output does not depend on the block lengths. Two things have
    // to hold, and the first is what makes the second mean anything.
    assert_eq!(artifact(&plain, "rec").hash, artifact(&jitter, "rec").hash, "the capture must not depend on the block lengths");
    // MR-27: the Run holds two Mocks, so the counts have to be read from the *receiver's*
    // own section. The unqualified name used to compare the transmitter's, whose
    // `rx_blocks` is 0 in both runs, so the assertion passed whatever the receiver did.
    // `rx_samples` is the witness and not `rx_blocks`: the Run's end depends on which block
    // the capture completes in, so jittering the block lengths changes how many samples the
    // receiver was given, while the block *count* rounds to the same 14 either way.
    let receiver = |manifest: &ezsdr_kernel::manifest::Manifest| section(manifest, "ezsdr.radio.mock.dev_rx.stats").clone();
    let (plain_rx, jitter_rx) = (receiver(&plain), receiver(&jitter));
    assert_eq!(section(&plain, "ezsdr.radio.mock.dev_tx.stats")["rx_blocks"], 0, "the transmitter has no receive link");
    assert_ne!(
        plain_rx["rx_samples"], jitter_rx["rx_samples"],
        "the two runs must really use different block lengths, or the equal capture proves nothing"
    );
}

#[test]
fn v58_08_a_session_hears_a_burst_from_its_first_sample_in_either_instance_order() {
    // `ideal` (no command lead, no LO draws), one sample per microsecond. The transmitter is
    // `a` in one Run and `z` in the other, so the stepping loop visits it before the receiver
    // `b` in one and after it in the other (MA-30 orders by fragment id).
    let samples = ramp(1_000);
    let run = |tx: &str| {
        let temp = rig::TempDir::new(&format!("v58-08-session-{tx}"));
        let profile = rig::link_session_profile("ideal", tx, "b", &temp.0, coupling(tx, "b", 0.0, 0, None, 0));
        let mut run = session_run(&temp, &profile);
        let clock = root(&run);
        let rid = |path: String| ResourceId::parse(&path).unwrap();
        run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
        let (bytes, _) = experiments::waveform_of(&samples);
        let entries = [
            run.submit(SessionAction::SetParameter { target: rid(tx.to_owned()), key: Key::parse("radio.tx.channels").unwrap(), value: Value::Int(1) }, None).unwrap(),
            run.submit(SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: rid("rec".to_owned()), at: Some(TimePoint::new(clock, T0 + 1_990_000)), params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::Int(5_000))]) }, None).unwrap(),
        ];
        // T0 + 1 999 000 ns is the instant of the receiver's sample 1 999, the last of its
        // first block; the burst starts there, in the round that could have published it.
        run.advance_to(TimePoint::new(clock, T0 + 1_999_000)).unwrap();
        let burst = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: rid(format!("{tx}/tx")), at: None, params: Default::default() }, Some(&bytes)).unwrap();
        run.advance_to(TimePoint::new(clock, T0 + 3_500_500)).unwrap();
        let stop = run.submit(SessionAction::Stop { target: Some(rid(format!("{tx}/tx"))) }, None).unwrap();
        assert!(entries.iter().chain([&burst, &stop]).all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
        let manifest = end_session(run, clock, T0 + 8_000_000);
        let capture = artifact(&manifest, "rec_0");
        (capture.continuity[0].first.ticks, rig::read_capture(capture, 1).remove(0))
    };
    let (first_a, capture_a) = run("a");
    let (first_z, capture_z) = run("z");
    assert_eq!((first_a, &capture_a), (first_z, &capture_z), "CH-9: the capture does not depend on the stepping order");
    assert_eq!(first_a, 1_990);
    assert_eq!(capture_a.len(), 5_000);
    for (index, sample) in capture_a.iter().enumerate() {
        let k = first_a + index as i64;
        let expected = if (1_999..=3_500).contains(&k) { samples[((k - 1_999) % 1_000) as usize] } else { (0.0, 0.0) };
        assert_eq!(*sample, expected, "captured sample {k}");
    }
}

#[test]
fn v57_a_software_loopback_session_captures_what_it_transmits() {
    let temp = rig::TempDir::new("v57-loopback");
    let environment = json!({ "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] } });
    let profile = rig::session_profile("ideal", json!({ "id": "mock" }), &temp.0, environment);
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let samples = ramp(1_000);
    let (bytes, _) = experiments::waveform_of(&samples);
    let entries = [
        run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: Value::Int(1) }, None).unwrap(),
        run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: Default::default() }, Some(&bytes)).unwrap(),
        run.submit(SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: ResourceId::parse("rec").unwrap(), at: None, params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::Int(5_000))]) }, None).unwrap(),
    ];
    assert!(entries.iter().all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
    let manifest = end_session(run, clock, T0 + 20_000_000);
    let capture = artifact(&manifest, "rec_0");
    let first = capture.continuity[0].first.ticks;
    let received = rig::read_capture(capture, 1).remove(0);
    assert_eq!(received.len(), 5_000);
    for (index, sample) in received.iter().enumerate() {
        let k = first + index as i64;
        assert_eq!(*sample, samples[((k - 1_000) % 1_000) as usize], "captured sample {k}");
    }
}

#[test]
fn v58_03_a_run_reproduces_from_its_own_manifest() {
    let temp = rig::TempDir::new("v58-03-reproduce");
    let samples = ramp(3_000);
    let (first, _) = link_run(&temp, "a", "b", "x310-like", true, coupling("a", "b", -6.0, 1_000, Some(-30.0), 11), &samples);
    let (bytes, waveform) = experiments::waveform_of(&samples);
    assert_eq!(first.inputs, vec![waveform.clone()]);
    let rerun = spec_run(&temp, &first.spec.body, &first.binding.body, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let clock = root(&rerun);
    let second = finish_at(rerun, clock, T0 + 25_000_000);
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
}
