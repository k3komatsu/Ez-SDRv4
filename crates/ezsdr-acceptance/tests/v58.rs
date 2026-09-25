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
    let stats = section(&manifest, "ezsdr.radio.mock.stats");
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
    assert_eq!(section(&seven, "ezsdr.radio.mock.stats")["rx_blocks"], 12);
    assert_eq!(section(&eight, "ezsdr.radio.mock.stats")["rx_blocks"], 11);
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
    for name in ["ezsdr.links", "ezsdr.radio.mock.envelope", "ezsdr.radio.mock.bursts", "ezsdr.radio.mock.faults", "ezsdr.radio.mock.rejected", "ezsdr.radio.mock.stats", "ezsdr.radio.mock.applied"] {
        assert!(manifest.sections.contains_key(&ezsdr_kernel::spec::Namespace::parse(name).unwrap()), "missing section {name}");
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
    let burst: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.bursts")[0].clone()).unwrap();
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
    assert_ne!(section(&plain, "ezsdr.radio.mock.stats")["rx_blocks"], section(&jitter, "ezsdr.radio.mock.stats")["rx_blocks"]);
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
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.bursts").clone()).unwrap();
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
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "ezsdr.radio.mock.bursts").clone()).unwrap();
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
    assert_eq!(section(&manifest, "ezsdr.radio.mock.rejected").as_array().unwrap().len(), 0);
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
