mod support;

use std::collections::BTreeMap;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::event::EventKind;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::run::{CleanupMode, RunState, Stage, StopCause, Termination};
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::time::TimePoint;
use ezsdr_radio::payloads::{RxOverflowCause, RxOverflowPayload};
use serde_json::json;
use support::{T0, artifact, asking, end_session, finish_at, loopback_run, output, resource, root, section, session_run, source, spec_run};

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
    let left = complete(&spec, "x3xx", json!({ "id": "mock" }), json!({}), "v58-01-x310", 20_000_000);
    let right = complete(&spec, "ideal", json!({ "id": "mock" }), json!({}), "v58-01-ideal", 20_000_000);
    assert_eq!(left.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(right.termination.reason, Termination::Stopped { cause: StopCause::Client {} });
    assert_eq!(left.artifacts.len(), 1);
    assert_eq!(right.artifacts.len(), 1);
    assert_eq!(artifact(&left, "rec", 0).size_bytes, 80_000);
    assert_eq!(artifact(&right, "rec", 0).size_bytes, 80_000);
    assert!(!artifact(&left, "rec", 0).partial);
    assert!(!artifact(&right, "rec", 0).partial);
    assert_eq!(left.spec.body, right.spec.body);
}

#[test]
fn v58_02_ten_virtual_seconds_run_faster_than_wall_clock() {
    let temp = rig::TempDir::new("v58-02");
    let spec = experiments::receive(1, 1.0e6, 1.0e9, None);
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = spec_run(&temp, &spec, &profile, Default::default());
    let clock = root(&run);
    let began = std::time::Instant::now();
    run.run_until_end(ezsdr_kernel::time::TimePoint::new(clock, T0 + 10_000_000_000)).unwrap();
    assert!(began.elapsed().as_secs() < 10);
    assert!(run.now().ticks_in(run.now().domain()).unwrap() >= T0 + 10_000_000_000);
    let manifest = run.finish();
    let stats = section(&manifest, "mock", "ezsdr.radio.mock.stats");
    let samples = stats["rx_samples"].as_u64().unwrap();
    assert!((10_000_000..=10_010_000).contains(&samples));
}

#[test]
fn v58_10_experiments_name_no_mock_type() {
    // The experiments and, since Phase 5, the responder component: application logic,
    // which speaks the Kernel contract and — the responder — the native Executor's
    // component ABI, and names no Mock (GX-6).
    let sources = [
        ("experiments.rs", include_str!("../src/experiments.rs"), &["use ezsdr_kernel", "use serde_json", "use std"][..]),
        ("responder.rs", include_str!("../src/responder.rs"), &["use ezsdr_kernel", "use ezsdr_exec_native", "use serde_json", "use std"][..]),
    ];
    for (file, source, allowed) in sources {
        // `crate::` and `super::` would reach the rig, which builds the simulated radios
        // (Review G, P2-4).
        for forbidden in ["mock", "Mock", "x310", "ideal", "sim-engine", "sim_engine", "ezsdr_mock_radio", "pub use", "crate::", "super::"] {
            assert!(!source.contains(forbidden), "{file}: found forbidden name {forbidden}");
        }
        // Every crate path, not only a `use` line: an inline `ezsdr_radio::…` is a
        // dependency too (Review F, P2-5).
        let crates: Vec<&str> = allowed.iter().map(|prefix| prefix.trim_start_matches("use ")).collect();
        for (at, _) in source.match_indices("ezsdr_") {
            let name: String = source[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            assert!(crates.contains(&name.as_str()), "{file}: names crate {name}");
        }
        for line in source.lines().filter(|line| line.trim_start().starts_with("use ")) {
            assert!(allowed.iter().any(|prefix| line.trim_start().starts_with(prefix)), "{file}: unexpected import: {line}");
        }
    }
    // Since Phase 6 the Python package and its examples: they name Vocabulary keys, verbs
    // and event kinds, and no Module, device profile or simulation section (EA-18, GY-6).
    let python = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python");
    let mut scanned = 0;
    for dir in ["ezsdr", "examples"] {
        for entry in std::fs::read_dir(python.join(dir)).expect("the Python sources are readable") {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "py") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for forbidden in
                ["mock", "Mock", "x310", "ideal", "sim-engine", "sim_engine", "sim.channel", "sim.seed", "sim.faults", "ezsdr.radio", "ezsdr.sink", "ezsdr.sim", "ezsdr.link", "ezsdr.exec"]
            {
                assert!(!source.contains(forbidden), "{}: found forbidden name {forbidden}", path.display());
            }
            scanned += 1;
        }
    }
    assert!(scanned >= 6, "the Python package and examples, bench_loopback.py included, were found ({scanned} files)");
}

#[test]
fn v58_03_same_seed_same_manifest_projection() {
    let temp = rig::TempDir::new("v58-03-same");
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(20_000));
    let selector = json!({ "id": "mock", "block_len_jitter": true, "rx_test_pattern": "ramp" });
    let first_profile = rig::spec_profile("x3xx", selector.clone(), &temp.0, json!({ "sim.seed": 7 }));
    let mut first = spec_run(&temp, &spec, &first_profile, BTreeMap::new());
    let clock = root(&first);
    first.advance_to(TimePoint::new(clock, T0 + 25_000_000)).unwrap();
    let first = first.finish();
    let second_profile = rig::spec_profile("x3xx", selector, &temp.0, json!({ "sim.seed": 7 }));
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
        let profile = rig::spec_profile("x3xx", selector.clone(), &temp.0, json!({ "sim.seed": seed }));
        let mut run = spec_run(&temp, &spec, &profile, BTreeMap::new());
        let clock = root(&run);
        run.advance_to(TimePoint::new(clock, T0 + 25_000_000)).unwrap();
        run.finish()
    };
    let seven = run_seed(7);
    let eight = run_seed(8);
    assert_eq!(artifact(&seven, "rec", 0).hash, artifact(&eight, "rec", 0).hash);
    assert_eq!(section(&seven, "mock", "ezsdr.radio.mock.stats")["rx_blocks"], 12);
    assert_eq!(section(&eight, "mock", "ezsdr.radio.mock.stats")["rx_blocks"], 10);
}

#[test]
fn v58_04_mock_events_reach_counters_policy_and_manifest() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    let manifest = complete(&spec, "x3xx", json!({ "id": "mock" }), faults.clone(), "v58-04", 160_000_000);
    let rows = &manifest.events.counters;
    assert_eq!(rows.iter().find(|row| row.source == source("mock/rx/s0") && row.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap().count, 1);
    let events: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(events.len(), 1);
    // MR-37: the Mock raises it on the hot path, as a device's sample path would, so the
    // Manifest holds RM-24's bytes, which the radio Vocabulary decodes (RS-32a withdrawn).
    assert!(events[0].payload.is_array(), "{}", events[0].payload);
    assert_eq!(RxOverflowPayload::from_payload(&events[0].payload).unwrap().cause, RxOverflowCause::Overrun);
    assert_eq!(artifact(&manifest, "rec", 0).marks.len(), 1);
    assert_eq!(artifact(&manifest, "rec", 0).marks[0].kind.as_str(), ezsdr_radio::kinds::RX_OVERFLOW);

    let mut stopped = spec.clone();
    stopped["policies"]["failure"] = json!({ "radio.RX_OVERFLOW": "stop" });
    let temp = rig::TempDir::new("v58-04-stop");
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, faults);
    let run = spec_run(&temp, &stopped, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 160_000_000);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse("radio.RX_OVERFLOW").unwrap() } });
    assert!(manifest.run.transitions.iter().any(|entry| entry.state == RunState::Stopping { mode: CleanupMode::Orderly }));
}

#[test]
fn v58_05_device_lost_aborts_with_full_cleanup() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 5_000_000, "fault": "device_lost", "target": "radio" }] });
    let manifest = complete(&spec, "x3xx", json!({ "id": "mock" }), faults, "v58-05", 10_000_000);
    assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
    assert!(manifest.run.transitions.iter().any(|entry| entry.state == RunState::Stopping { mode: CleanupMode::Abort }));
    assert!(manifest.termination.cleanup_failures.is_empty());
    assert!(artifact(&manifest, "rec", 0).partial);
    // MR-20 (spec 20, VF-3): the loss at 5 ms publishes the block in progress up to it.
    assert_eq!(artifact(&manifest, "rec", 0).size_bytes, 40_000);
    assert!(manifest.hash.is_some());
}

#[test]
fn v58_06_injected_overflow_is_a_uhd_overflow() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    let manifest = complete(&spec, "x3xx", json!({ "id": "mock" }), faults, "v58-06-overflow", 160_000_000);
    let capture = artifact(&manifest, "rec", 0);
    assert_eq!(capture.size_bytes, 800_000);
    assert_eq!(capture.continuity[0].gaps.len(), 1);
    let gap = &capture.continuity[0].gaps[0];
    assert_eq!(gap.cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
    assert_eq!(gap.start.ticks_in(gap.start.domain()).unwrap(), 1_000);
    assert_eq!(gap.len, 50_000);
    assert_eq!(gap.lost, Some(50_000));
    assert_eq!(gap.link_dropped, 0);
    let event = manifest.events.delivered.iter().find(|event| event.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).unwrap();
    assert_eq!(RxOverflowPayload::from_payload(&event.payload).unwrap(), RxOverflowPayload { cause: RxOverflowCause::Overrun, lost: 50_000 });
}

#[test]
fn v51_an_overflowed_capture_is_a_sigmf_recording() {
    // HD-15: v58_06's overflow Run, read as SigMF. The Dataset holds only delivered
    // samples, so the capture segment after the gap starts where the first run ends in
    // the file, and its global index is 50 000 samples further on.
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_overflow", "target": "radio" }] });
    let temp = rig::TempDir::new("v51-sigmf");
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, faults);
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 160_000_000);
    let capture = artifact(&manifest, "rec", 0);
    let data = capture.uri.strip_prefix("file://").unwrap();
    assert!(data.ends_with(".sigmf-data"), "{data}");
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(data).with_extension("sigmf-meta")).unwrap()).unwrap();
    assert_eq!(meta["global"]["core:datatype"], "cf32_le");
    assert_eq!(meta["global"]["core:sample_rate"], json!(1_000_000.0));
    assert_eq!(meta["global"]["ezsdr:partial"], false);
    assert_eq!(
        meta["captures"],
        json!([
            { "core:sample_start": 0, "core:global_index": 0 },
            { "core:sample_start": 1_000, "core:global_index": 51_000 }
        ])
    );
    let gaps = meta["global"]["ezsdr:gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0]["cause"], json!({ "kind": "overflow_restart" }));
    assert_eq!((gaps[0]["global_index"].clone(), gaps[0]["len"].clone(), gaps[0]["lost"].clone()), (json!(1_000), json!(50_000), json!(50_000)));
}

#[test]
fn v58_06_sequence_error_is_seq_discontinuity() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(100_000));
    let faults = json!({ "sim.faults": [{ "at_ns": 1_000_000, "fault": "rx_sequence_error", "target": "radio" }] });
    let manifest = complete(&spec, "x3xx", json!({ "id": "mock" }), faults, "v58-06-seq", 160_000_000);
    let gaps = &artifact(&manifest, "rec", 0).continuity[0].gaps;
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].cause, ezsdr_kernel::stream::GapCause::SequenceError {});
    assert_eq!(gaps[0].start.ticks_in(gaps[0].start.domain()).unwrap(), 1_000);
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
    let manifest = complete(&spec, "x3xx", json!({ "id": "mock" }), faults, "v58-06-overlap", 160_000_000);
    let gap = &artifact(&manifest, "rec", 0).continuity[0].gaps[0];
    assert_eq!(gap.cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
    assert_eq!(gap.start.ticks_in(gap.start.domain()).unwrap(), 1_000);
    assert_eq!(gap.len, 59_000);
    assert_eq!(gap.lost, Some(59_000));
}

#[test]
fn v58_07_manifest_records_every_input_and_output() {
    let temp = rig::TempDir::new("v58-07");
    let (bytes, waveform) = experiments::waveform(1_000);
    let spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 10_000, Some(10_000));
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 20_000_000);
    assert_eq!(manifest.inputs, vec![waveform]);
    assert_eq!(manifest.artifacts.len(), 1);
    for module in ["ezsdr.sim-engine", "ezsdr.radio.mock", "ezsdr.link.host", "ezsdr.sink.capture"] {
        assert!(support::modules(&manifest).contains(&module));
    }
    for vocabulary in ["radio", "sim", "sink"] {
        assert!(manifest.vocabularies.keys().any(|id| id.as_str() == vocabulary));
    }
    for stream in ["mock/rx/s0", "mock/tx/s0"] {
        assert!(manifest.clocks.sample_clocks.iter().any(|clock| clock.stream.path() == stream));
    }
    assert_eq!(manifest.links.len(), manifest.plan.as_ref().unwrap().links.len());
    // MR-27: a MockRadio section carries no instance id; the Kernel files it under the
    // instance's source (RS-39).
    for suffix in ["envelope", "bursts", "faults", "rejected", "stats"] {
        let name = format!("ezsdr.radio.mock.{suffix}");
        assert!(manifest.section(&source("mock"), &name).is_some(), "missing section {name}");
    }
    // The documents as parsed, with every default written out (RS-45).
    let spec = ezsdr_kernel::spec::ExperimentSpec::from_json(&spec).unwrap();
    let profile = ezsdr_kernel::binding::BindingProfile::from_json(&profile).unwrap();
    assert_eq!(manifest.spec.body, serde_json::to_value(spec).unwrap());
    assert_eq!(manifest.binding.body, serde_json::to_value(profile).unwrap());
}

#[test]
fn v58_11_short_lead_burst_is_a_time_error() {
    let temp = rig::TempDir::new("v58-11-short");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    run.submit(
        SessionAction::Vocabulary {
            ns: Namespace::parse("radio").unwrap(),
            verb: Ident::parse("start_repeat").unwrap(),
            target: resource("radio/tx/s0"),
            at: Some(TimePoint::new(clock, T0 + 2_000_000)),
            params: Default::default(),
        },
        Some(&bytes),
    )
    .unwrap();
    let manifest = end_session(run, clock, T0 + 60_000_000);
    // The transmit clock the Session asked for at connect begins at T0 + 3 ms (RM-25); the burst
    // for T0 + 2 ms is its tick T0/1000 + 2 000 (1 ms before the origin), before that origin, so it is late by 1 ms and starts at
    // the origin (RM-15).
    let events: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["cause"], "late");
    assert_eq!(events[0].payload["outcome"], "send_asap");
    assert_eq!(events[0].payload["late_by_ns"], 1_000_000);
    assert_eq!(events[0].payload["target"]["ticks"], T0 / 1_000 + 2_000);
    let burst: ezsdr_kernel::stream::BurstRecord = serde_json::from_value(section(&manifest, "mock", "ezsdr.radio.mock.bursts")[0].clone()).unwrap();
    assert_eq!(burst.late_by.unwrap().ticks_in(burst.late_by.unwrap().domain()).unwrap(), 1_000_000);
    assert_eq!(burst.target.ticks_in(burst.target.domain()).unwrap(), T0 / 1_000 + 3_000);
}

#[test]
fn v58_11_19_5_msps_is_coerced_to_20() {
    let mut spec = experiments::receive(1, 19.5e6, 1.0e9, Some(1_000));
    spec["policies"]["coercion"] = json!({ "radio.rx.s0.sample_rate_hz": "warn" });
    let temp = rig::TempDir::new("v58-11-coerce");
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 1_000_000);
    let preview = manifest.admission.coercions_preview.iter().find(|item| item.coercion.key.as_str() == "radio.rx.s0.sample_rate_hz").unwrap();
    assert_eq!(preview.coercion.requested, Value::num(19.5e6).unwrap());
    assert_eq!(preview.coercion.applied, Value::num(20.0e6).unwrap());
    assert_eq!(manifest.prepare.reports[&ezsdr_kernel::spec::Ident::parse("radio").unwrap()].effective[&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()], Value::num(20.0e6).unwrap());
    let rx = manifest.clocks.sample_clocks.iter().find(|clock| clock.stream.path() == "mock/rx/s0").unwrap();
    assert_eq!((rx.nominal_rate.num(), rx.nominal_rate.den()), (20_000_000, 1));

    let refused_spec = experiments::receive(1, 19.5e6, 1.0e9, Some(1_000));
    let run = spec_run(&temp, &refused_spec, &profile, BTreeMap::new());
    let refused = run.finish();
    assert!(matches!(refused.termination.reason, Termination::Failed { stage: Stage::Validate, .. }));
}

#[test]
fn v58_11_beyond_the_performance_envelope_is_rejected_at_validate() {
    let spec = experiments::receive(2, 200.0e6, 1.0e9, None);
    let temp = rig::TempDir::new("v58-11-envelope");
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let manifest = run.finish();
    assert!(matches!(manifest.termination.reason, Termination::Failed { stage: Stage::Validate, .. }));
    let rejected = manifest.admission.rejected.iter().find(|item| item.key.as_str() == "radio.rx.s0.sample_rate_hz").unwrap();
    assert!(rejected.reason.starts_with("RM-32"));
}

#[test]
fn v58_12_jitter_leaves_the_capture_unchanged() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(12_345));
    let selector = |jitter| json!({ "id": "mock", "block_len_jitter": jitter, "rx_test_pattern": "ramp" });
    let plain = complete(&spec, "x3xx", selector(false), json!({ "sim.seed": 7 }), "v58-12-plain", 20_000_000);
    let jitter = complete(&spec, "x3xx", selector(true), json!({ "sim.seed": 7 }), "v58-12-jitter", 20_000_000);
    assert_eq!(artifact(&plain, "rec", 0).hash, artifact(&jitter, "rec", 0).hash);
    assert_eq!(artifact(&plain, "rec", 0).continuity, artifact(&jitter, "rec", 0).continuity);
    assert_ne!(section(&plain, "mock", "ezsdr.radio.mock.stats")["rx_blocks"], section(&jitter, "mock", "ezsdr.radio.mock.stats")["rx_blocks"]);
}

#[test]
fn v58_13_session_manifest_has_log_waveform_and_capture() {
    let temp = rig::TempDir::new("v58-13");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    let b = run
        .submit(
            SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: Default::default() },
            Some(&bytes),
        )
        .unwrap();
    let c = run
        .submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: output("rec"),
                at: None,
                params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(5_000))]),
            },
            None,
        )
        .unwrap();
    assert_eq!([b.seq, c.seq], [0, 1], "the first logged call is the repeat: connect put the channels in their streams");
    assert!([b, c].iter().all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
    let manifest = end_session(run, clock, T0 + 60_000_000);
    assert_eq!(manifest.action_log.len(), 2);
    assert_eq!(manifest.inputs.len(), 1);
    assert_eq!(artifact(&manifest, "rec", 0).size_bytes, 40_000);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "mock", "ezsdr.radio.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].end, ezsdr_kernel::stream::BurstEnd::Stop);
    // RS-19: the untimed repeat is admitted no earlier than the transmit clock allows, so it
    // starts on time.
    assert_eq!(bursts[0].late_by, None);
    assert!(!manifest.events.delivered.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR));
}

#[test]
fn rs_19_an_untimed_repeat_in_a_session_starts_at_the_clock_origin() {
    // A Session that asked for tx c0 at connect: the transmit clock begins a start lead after the Run's start
    // (RM-25) and RS-19 admits the untimed repeat no earlier than that origin.
    let temp = rig::TempDir::new("rs-19-enable");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    let repeat = run
        .submit(
            SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: Default::default() },
            Some(&bytes),
        )
        .unwrap();
    assert!(matches!(repeat.outcome, Outcome::Admitted { .. }));
    let manifest = end_session(run, clock, T0 + 60_000_000);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "mock", "ezsdr.radio.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    // The burst starts at the clock's origin (T0 + 3 ms), on time.
    assert_eq!((bursts[0].target.ticks_in(bursts[0].target.domain()).unwrap(), bursts[0].late_by), (T0 / 1_000 + 3_000, None));
    assert!(!manifest.events.delivered.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR));
}

#[test]
fn v58_13_a_session_stop_of_the_recorder_keeps_a_partial_capture() {
    // Phase 2's named Gate X risk: a `Stop` for the recorder routed through the Kernel's
    // admission and dispatch, not handed to the Sink directly (HD-11, HD-13).
    let temp = rig::TempDir::new("v58-13-stop");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    let capture = |run: &mut ezsdr_kernel::coordinator::RunHandle, samples: i64| {
        run.submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: output("rec"),
                at: None,
                params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(samples))]),
            },
            None,
        )
        .unwrap()
    };
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    assert!(matches!(capture(&mut run, 50_000).outcome, Outcome::Admitted { .. }));
    run.advance_to(TimePoint::new(clock, T0 + 10_000_000)).unwrap();
    let stop = run.submit(SessionAction::Stop { target: Some(output("rec")) }, None).unwrap();
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }), "{:?}", stop.outcome);
    run.advance_to(TimePoint::new(clock, T0 + 11_000_000)).unwrap();
    assert!(matches!(capture(&mut run, 1_000).outcome, Outcome::Admitted { .. }));
    let manifest = end_session(run, clock, T0 + 20_000_000);
    let stopped = artifact(&manifest, "rec", 0);
    let later = artifact(&manifest, "rec", 1);
    // The Stop finished the recording capture with what it held — samples 1 000 to
    // 9 999, delivered before T0 + 10 ms — and the Sink went on to serve the next one.
    assert!(stopped.partial);
    assert_eq!(
        (stopped.continuity[0].first.ticks_in(stopped.continuity[0].first.domain()).unwrap(), stopped.continuity[0].end.ticks_in(stopped.continuity[0].end.domain()).unwrap(), stopped.size_bytes),
        (1_000, 10_000, 72_000)
    );
    assert!(!later.partial);
    assert_eq!((later.continuity[0].first.ticks_in(later.continuity[0].first.domain()).unwrap(), later.size_bytes), (11_000, 8_000));
    assert_eq!(manifest.action_log.len(), 3);
    // HD-15: the partial capture is a Recording too, and says it is partial.
    let data = stopped.uri.strip_prefix("file://").unwrap();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(data).with_extension("sigmf-meta")).unwrap()).unwrap();
    assert_eq!(meta["global"]["ezsdr:partial"], true);
    assert_eq!(meta["captures"], json!([{ "core:sample_start": 0, "core:global_index": 1_000 }]));
}

#[test]
fn v58_14_profiles_differing_only_in_environment_both_run() {
    let spec = experiments::receive(1, 1.0e6, 1.0e9, Some(10_000));
    let base = complete(&spec, "x3xx", json!({ "id": "mock" }), json!({ "sim.seed": 1 }), "v58-14-a", 20_000_000);
    let other = complete(&spec, "x3xx", json!({ "id": "mock" }), json!({ "sim.seed": 2, "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 0.0, "hi_hz": 7.0e9 }] } }), "v58-14-b", 20_000_000);
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
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash, bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 15_000_000);
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "mock", "ezsdr.radio.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert!(bursts[0].wraps >= 4);
    assert!(bursts[0].samples >= 4_000);
    assert_eq!(bursts[0].end, ezsdr_kernel::stream::BurstEnd::Stop);
    assert!(!manifest.events.delivered.iter().any(|event| [ezsdr_radio::kinds::TIME_ERROR, ezsdr_radio::kinds::TX_UNDERFLOW].contains(&event.kind.as_str())));
}

#[test]
fn v58_16_a_value_outside_the_rf_envelope_is_refused_at_connect_and_a_transmit_runtime_change_has_no_class() {
    // Note 47 stage 3: the envelope is checked on the initial values; a transmit change during the Run is refused as a key with no update class
    // until stage 4 (RM-27). The receive forms are the two tests below.
    let temp = rig::TempDir::new("v58-16-rf");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 2.4e9, "hi_hz": 2.5e9 }] } }));
    let connect = |frequency: f64| {
        let assembly = rig::assemble(&profile, BTreeMap::new());
        let initial = asking("radio", &[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.tx.c0.frequency_hz", Scalar::Num(ezsdr_kernel::spec::Finite::new(frequency).unwrap()))]);
        ezsdr_kernel::coordinator::connect(&profile, &initial, assembly, ezsdr_kernel::run::Lease::attached())
    };
    let refused = connect(2.6e9).expect("a refused Session is a failed Run");
    assert!(
        matches!(refused.state(), RunState::CleanedUp { termination: Termination::Failed { reason, .. } } if reason.contains("radio.rf_envelope")),
        "an initial frequency outside the band is refused by the envelope: {:?}",
        refused.state()
    );
    let mut run = connect(2.45e9).expect("an initial frequency inside the band connects");
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let mut refused_seqs = Vec::new();
    for frequency in [2.45e9, 2.6e9] {
        let entry = run.submit(SessionAction::SetParameter { target: resource("radio"), key: Key::parse("radio.tx.c0.frequency_hz").unwrap(), value: Value::num(frequency).unwrap() }, None).unwrap();
        refused_seqs.push(entry.seq);
        let Outcome::Rejected { violations } = entry.outcome else { panic!("a runtime radio value change was admitted") };
        assert!(violations.iter().any(|violation| violation.check.as_str() == "ezsdr.update_class" && violation.reason.contains("declares no update class")), "{violations:?}");
    }
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.tx.c0.frequency_hz").unwrap()], Value::num(2.45e9).unwrap());
    let manifest = end_session(run, clock, T0 + 5_000_000);
    for seq in refused_seqs {
        assert!(manifest.action_log.iter().any(|logged| logged.seq == seq && matches!(logged.outcome, Outcome::Rejected { .. })), "the rejection of action {seq} is logged");
    }
}

#[test]
fn v58_16_a_receive_antenna_outside_the_rf_envelope_is_refused_on_a_stopped_stream() {
    // Note 47 stage 3, §58 #16 in its envelope form: on a stopped receive stream a value outside the RF envelope is refused at admission,
    // logged, and never dispatched (RM-19, RM-21); a value inside it, or one a Provider's `coerce` refuses, is judged by those.
    let temp = rig::TempDir::new("v58-16-rx-antenna");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 1.0e8, "hi_hz": 6.0e9 }], "antenna_ports": ["RX2"] } }));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let stop = run.submit(SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
    assert!(matches!(stop.outcome, Outcome::Admitted { .. }));
    let set = |run: &mut ezsdr_kernel::coordinator::RunHandle, key: &str, value: Value| {
        run.submit(SessionAction::SetParameter { target: resource("radio"), key: Key::parse(key).unwrap(), value }, None).unwrap()
    };
    let outside = set(&mut run, "radio.rx.c0.antenna", Value::from("TX/RX"));
    let Outcome::Rejected { violations } = &outside.outcome else { panic!("an antenna outside the envelope was admitted: {:?}", outside.outcome) };
    assert!(violations.iter().any(|v| v.check.as_str() == "radio.rf_envelope" && v.key.as_ref().is_some_and(|k| k.as_str() == "radio.rx.c0.antenna")), "{violations:?}");
    // A frequency above the board's range is refused by the Provider's coerce at admission, whatever the stream state.
    let beyond = set(&mut run, "radio.rx.c0.frequency_hz", Value::num(7.0e9).unwrap());
    assert!(matches!(beyond.outcome, Outcome::Rejected { .. }), "{:?}", beyond.outcome);
    let inside = set(&mut run, "radio.rx.c0.antenna", Value::from("RX2"));
    assert!(matches!(inside.outcome, Outcome::Admitted { .. }), "{:?}", inside.outcome);
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.rx.c0.antenna").unwrap()], Value::from("RX2"), "the refused value was never admitted");
    let seqs = [outside.seq, beyond.seq];
    let manifest = end_session(run, clock, T0 + 5_000_000);
    for seq in seqs {
        let logged = manifest.action_log.iter().find(|logged| logged.seq == seq).expect("the refusal is logged");
        assert!(matches!(logged.outcome, Outcome::Rejected { .. }), "action {seq}");
    }
    let rejected = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count();
    assert_eq!(rejected, 0, "nothing was dispatched to the Provider, so it refused nothing");
}

#[test]
fn v58_16_an_allowed_value_for_a_streaming_stream_is_refused_by_the_provider_and_matched_by_action_id() {
    // The Kernel admits an allowed value; the Provider refuses it because the stream is streaming, naming the Action by its id (RM-21, RM-22).
    let temp = rig::TempDir::new("v58-16-streaming");
    let profile =
        rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({ "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 1.0e8, "hi_hz": 6.0e9 }], "antenna_ports": ["RX2", "TX/RX"] } }));
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let entry = run.submit(SessionAction::SetParameter { target: resource("radio"), key: Key::parse("radio.rx.c0.antenna").unwrap(), value: Value::from("TX/RX") }, None).unwrap();
    let Outcome::Admitted { dispatched, .. } = &entry.outcome else { panic!("an allowed value was rejected by the Kernel: {:?}", entry.outcome) };
    let id = serde_json::to_value(dispatched[0]).unwrap();
    // KC-27: the admitted configuration holds the value the Provider refused.
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()][&Key::parse("radio.rx.c0.antenna").unwrap()], Value::from("TX/RX"));
    let manifest = end_session(run, clock, T0 + 5_000_000);
    let refused: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].payload["action"], id, "matched by the Action's id");
    assert_eq!(refused[0].payload["kind"], "update_parameter");
    assert!(refused[0].payload["reason"].as_str().unwrap().contains("streaming"), "{}", refused[0].payload["reason"]);
}

#[test]
fn v58_16_a_rate_beyond_the_envelope_is_refused_at_connect_and_a_runtime_change_has_no_class() {
    let temp = rig::TempDir::new("v58-16-rate");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::new());
    let initial = asking("radio", &[("radio.rx.c0.stream", Scalar::Int(0)), ("radio.rx.s0.sample_rate_hz", Scalar::Num(ezsdr_kernel::spec::Finite::new(200.0e6).unwrap()))]);
    // The envelope is checked on the initial value, at validation (RM-32).
    let coerced = ezsdr_kernel::coordinator::connect(&profile, &initial, assembly, ezsdr_kernel::run::Lease::attached()).expect("connects");
    assert!(matches!(&coerced.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, reason } } if reason.contains("RM-32")), "{:?}", coerced.state());
    let _ = coerced.finish();
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let prior = run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()).cloned();
    let entry = run.submit(SessionAction::SetParameter { target: resource("radio"), key: Key::parse("radio.rx.s0.sample_rate_hz").unwrap(), value: Value::num(200.0e6).unwrap() }, None).unwrap();
    let Outcome::Rejected { violations } = &entry.outcome else { panic!("a runtime radio value change was admitted") };
    assert!(violations.iter().any(|violation| violation.check.as_str() == "ezsdr.update_class" && violation.reason.contains("declares no update class")), "{violations:?}");
    assert_eq!(run.effective()[&Ident::parse("radio").unwrap()].get(&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()).cloned(), prior);
    let seq = entry.seq;
    let manifest = end_session(run, clock, T0 + 5_000_000);
    assert!(manifest.action_log.iter().any(|logged| logged.seq == seq && matches!(logged.outcome, Outcome::Rejected { .. })), "the rejection is logged");
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
    let capture = rig::read_capture(artifact(&manifest, "rec", 0), 1).remove(0);
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
    // Decision 2, RS-39: two MockRadio instances write the same section names, and the
    // Kernel files each under its writer's source, so both survive. Keyed by name alone,
    // the last instance written won and the other's records were silently gone.
    let temp = rig::TempDir::new("v58-08-sections");
    let (manifest, _) = link_run(&temp, "a", "b", "x3xx", false, coupling("a", "b", -6.0, 1_000, Some(-30.0), 0), &ramp(3_000));
    for suffix in ["bursts", "envelope", "faults", "rejected", "stats"] {
        let name = format!("ezsdr.radio.mock.{suffix}");
        let writers: Vec<_> = manifest.sections.iter().filter(|s| s.name.as_str() == name).map(|s| s.source.clone()).collect();
        assert_eq!(writers, [source("dev_rx"), source("dev_tx")], "{name}");
    }
    // the two are genuinely different documents, not one written twice
    assert_eq!(section(&manifest, "dev_tx", "ezsdr.radio.mock.stats")["rx_blocks"], 0);
    assert_ne!(section(&manifest, "dev_rx", "ezsdr.radio.mock.stats")["rx_blocks"], 0);
    assert_eq!(section(&manifest, "dev_tx", "ezsdr.radio.mock.bursts").as_array().unwrap().len(), 1, "SC-28: the transmitter's burst record");
    assert_eq!(section(&manifest, "dev_rx", "ezsdr.radio.mock.bursts").as_array().unwrap().len(), 0, "the receiver transmits nothing");

    // and the winner does not depend on the selector: the other jitter value, same ids
    let (jitter, _) = link_run(&temp, "a", "b", "x3xx", true, coupling("a", "b", -6.0, 1_000, Some(-30.0), 0), &ramp(3_000));
    assert_eq!(section(&jitter, "dev_tx", "ezsdr.radio.mock.bursts").as_array().unwrap().len(), 1);
    assert_ne!(section(&jitter, "dev_rx", "ezsdr.radio.mock.stats")["rx_blocks"], 0);
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
        let profile = rig::link_profile("x3xx", tx, rx, false, &temp.0, environment);
        let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        let clock = root(&run);
        let manifest = finish_at(run, clock, T0 + 25_000_000);
        assert_eq!(manifest.termination.reason, Termination::Stopped { cause: StopCause::Policy { event: EventKind::parse(EventKind::DEVICE_LOST).unwrap() } });
        let stats = section(&manifest, "dev_rx", "ezsdr.radio.mock.stats");
        (stats["rx_blocks"].clone(), stats["rx_samples"].clone())
    };
    let transmitter_first = run("a", "b");
    let receiver_first = run("z", "a");
    assert_eq!(transmitter_first, receiver_first);
    assert_eq!(receiver_first, (json!(1), json!(2_000)));

    // Both radios lose their device in that round: each is reported, whichever sorts
    // first (Review D, P1-1: the second used to be found and dropped).
    for (tx, rx) in [("a", "b"), ("z", "a")] {
        let temp = rig::TempDir::new("kd-01-both");
        let (bytes, waveform) = experiments::waveform_of(&ramp(3_000));
        let spec = experiments::link(tx, rx, 1.0e6, &waveform, 10_000, 20_000);
        let mut environment = coupling(tx, rx, -6.0, 1_000, None, 0);
        environment["sim.faults"] = json!([
            { "at_ns": 1_999_001, "fault": "device_lost", "target": tx },
            { "at_ns": 1_999_001, "fault": "device_lost", "target": rx }
        ]);
        let profile = rig::link_profile("x3xx", tx, rx, false, &temp.0, environment);
        let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        let clock = root(&run);
        let manifest = finish_at(run, clock, T0 + 25_000_000);
        let mut lost: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).map(|event| event.source.clone()).collect();
        lost.sort();
        assert_eq!(lost, [source("dev_rx"), source("dev_tx")], "tx {tx}, rx {rx}");
    }
}

#[test]
fn kg_07_a_burst_at_t0_plus_n_samples_is_exact() {
    // Replaces Phase 4's K3 ceiling test (GY-3, Phase 7): with T0 on every declared
    // grid (KC-15) and the transmit clock on its lattice (RM-25, MR-9), a start lead
    // that is not a whole number of samples no longer moves a burst at T0 + n samples.
    let heard = |start_lead_ns: u64| {
        let temp = rig::TempDir::new("kg07");
        let samples = ramp(3_000);
        let (bytes, waveform) = experiments::waveform_of(&samples);
        let spec = experiments::link("a", "b", 1.0e6, &waveform, 10_000, 20_000);
        let mut environment = coupling("a", "b", 0.0, 1_000, None, 0);
        environment["ezsdr.time"] = json!({ "class": "simulation", "start_lead_ns": start_lead_ns });
        let profile = rig::link_profile("ideal", "a", "b", false, &temp.0, environment);
        let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
        let t0 = run.start_instant().unwrap().ticks_in(run.start_instant().unwrap().domain()).unwrap();
        let clock = root(&run);
        let manifest = finish_at(run, clock, T0 + 25_000_000);
        let capture = rig::read_capture(artifact(&manifest, "rec", 0), 1).remove(0);
        let first = capture.iter().position(|sample| *sample != (0.0, 0.0)).unwrap();
        let bursts: Vec<ezsdr_kernel::stream::BurstRecord> = serde_json::from_value(section(&manifest, "dev_tx", "ezsdr.radio.mock.bursts").clone()).unwrap();
        let burst = &bursts[0];
        let tx = manifest.clocks.sample_clocks.iter().find(|record| record.domain == burst.target.domain()).unwrap();
        let ratio = tx.root_ticks_per_tick;
        assert_eq!(ratio.den(), 1);
        assert_eq!(tx.origin.ticks_in(tx.origin.domain()).unwrap() % ratio.num() as i64, 0, "the transmit origin is on its lattice");
        let target = tx.origin.ticks_in(tx.origin.domain()).unwrap() + burst.target.ticks_in(burst.target.domain()).unwrap() * ratio.num() as i64;
        (first, target - t0, burst.requested_target.is_some(), burst.late_by.is_some())
    };
    for lead in [2_000_000_000, 2_000_000_500] {
        assert_eq!(heard(lead), (10_001, 10_000 * 1_000, false, false), "start_lead_ns {lead}");
    }
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
    let noisy = |seed| link_run(&temp, "a", "b", "x3xx", false, coupling("a", "b", -6.0, 1_000, Some(-30.0), seed), &samples);
    let (first, first_capture) = noisy(7);
    let (second, _) = noisy(7);
    let (other, other_capture) = noisy(8);
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
    assert_ne!(artifact(&first, "rec", 0).hash, artifact(&other, "rec", 0).hash);
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
    let (plain, _) = link_run(&temp, "a", "b", "x3xx", false, environment.clone(), &samples);
    let (jitter, _) = link_run(&temp, "a", "b", "x3xx", true, environment, &samples);
    // §58 #12: the channel's output does not depend on the block lengths. Two things have
    // to hold, and the first is what makes the second mean anything.
    assert_eq!(artifact(&plain, "rec", 0).hash, artifact(&jitter, "rec", 0).hash, "the capture must not depend on the block lengths");
    // MR-27: the Run holds two Mocks, so the counts have to be read from the *receiver's*
    // own section. The unqualified name used to compare the transmitter's, whose
    // `rx_blocks` is 0 in both runs, so the assertion passed whatever the receiver did.
    // `rx_samples` is the witness and not `rx_blocks`: the Run's end depends on which block
    // the capture completes in, so jittering the block lengths changes how many samples the
    // receiver was given, while the block *count* rounds to the same 14 either way.
    let receiver = |manifest: &ezsdr_kernel::manifest::Manifest| section(manifest, "dev_rx", "ezsdr.radio.mock.stats").clone();
    let (plain_rx, jitter_rx) = (receiver(&plain), receiver(&jitter));
    assert_eq!(section(&plain, "dev_tx", "ezsdr.radio.mock.stats")["rx_blocks"], 0, "the transmitter has no receive link");
    assert_ne!(plain_rx["rx_samples"], jitter_rx["rx_samples"], "the two runs must really use different block lengths, or the equal capture proves nothing");
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
        let assembly = rig::assemble(&profile, BTreeMap::new());
        let initial = support::rx0_tx0(&assembly);
        let mut run = support::session_run_with(&temp, &profile, assembly, &initial);
        let clock = root(&run);
        run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
        let (bytes, _) = experiments::waveform_of(&samples);
        let entries = [run
            .submit(
                SessionAction::Vocabulary {
                    ns: Namespace::parse("sink").unwrap(),
                    verb: Ident::parse("capture").unwrap(),
                    target: output("rec"),
                    at: Some(TimePoint::new(clock, T0 + 1_990_000)),
                    params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(5_000))]),
                },
                None,
            )
            .unwrap()];
        // T0 + 1 999 000 ns is the instant of the receiver's sample 1 999, the last of its
        // first block; the burst starts there, in the round that could have published it.
        run.advance_to(TimePoint::new(clock, T0 + 1_999_000)).unwrap();
        let burst = run
            .submit(
                SessionAction::Vocabulary {
                    ns: Namespace::parse("radio").unwrap(),
                    verb: Ident::parse("start_repeat").unwrap(),
                    target: resource(&format!("{tx}/tx/s0")),
                    at: None,
                    params: Default::default(),
                },
                Some(&bytes),
            )
            .unwrap();
        run.advance_to(TimePoint::new(clock, T0 + 3_500_500)).unwrap();
        let stop = run.submit(SessionAction::Stop { target: Some(resource(&format!("{tx}/tx/s0"))) }, None).unwrap();
        assert!(entries.iter().chain([&burst, &stop]).all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
        let manifest = end_session(run, clock, T0 + 8_000_000);
        let capture = artifact(&manifest, "rec", 0);
        (capture.continuity[0].first.ticks_in(capture.continuity[0].first.domain()).unwrap(), rig::read_capture(capture, 1).remove(0))
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
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let samples = ramp(1_000);
    let (bytes, _) = experiments::waveform_of(&samples);
    let entries = [
        run.submit(
            SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: Default::default() },
            Some(&bytes),
        )
        .unwrap(),
        run.submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: output("rec"),
                at: None,
                params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(5_000))]),
            },
            None,
        )
        .unwrap(),
    ];
    assert!(entries.iter().all(|entry| matches!(entry.outcome, Outcome::Admitted { .. })));
    let manifest = end_session(run, clock, T0 + 20_000_000);
    assert_eq!(manifest.action_log.len(), 2, "the repeat and the capture: no Action put a channel in a stream, connect did");
    let capture = artifact(&manifest, "rec", 0);
    let first = capture.continuity[0].first.ticks_in(capture.continuity[0].first.domain()).unwrap();
    let received = rig::read_capture(capture, 1).remove(0);
    assert_eq!(received.len(), 5_000);
    for (index, sample) in received.iter().enumerate() {
        let k = first + index as i64;
        assert_eq!(*sample, samples[((k - 1_000) % 1_000) as usize], "captured sample {k}");
    }
}

// ---------------------------------------------------------------- connect's initial streams (RS-12, RM-30)

fn bursts_of(manifest: &Manifest) -> usize {
    section(manifest, "mock", "ezsdr.radio.mock.bursts").as_array().map_or(0, Vec::len)
}

#[test]
fn v58_a_default_session_has_no_stream_and_nothing_flows() {
    // RM-30: no channel is in a stream unless the Session asks. The default profile still links the
    // radio's `rx0` to its recorder, and the Session connects, advances and ends with nothing flowing.
    for profile_name in ["x3xx", "ideal"] {
        let temp = rig::TempDir::new(&format!("v58-no-stream-{profile_name}"));
        let profile = rig::session_profile(profile_name, json!({ "id": "mock" }), &temp.0, json!({}));
        let assembly = rig::assemble(&profile, BTreeMap::new());
        let run = support::session_run_with(&temp, &profile, assembly, &Default::default());
        assert!(matches!(run.state(), RunState::Running {}), "{profile_name}: {:?}", run.state());
        let clock = root(&run);
        let effective = &run.effective()[&Ident::parse("radio").unwrap()];
        for number in 0..2 {
            for direction in ["rx", "tx"] {
                assert_eq!(effective[&Key::parse(&format!("radio.{direction}.c{number}.stream")).unwrap()], Value::from(-1), "{profile_name}: {direction} c{number}");
            }
        }
        let manifest = end_session(run, clock, T0 + 100_000_000);
        assert!(manifest.clocks.sample_clocks.is_empty(), "{profile_name}: no stream, no SampleClock: {:?}", manifest.clocks.sample_clocks);
        assert!(manifest.artifacts.is_empty(), "{profile_name}: nothing was recorded");
        assert_eq!(manifest.action_log.len(), 0);
    }
}

#[test]
fn v58_membership_alone_transmits_nothing() {
    // A transmit channel in a stream gives the stream its clock and sends no waveform (RM-30).
    let temp = rig::TempDir::new("v58-membership-alone");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::new());
    let initial = support::asking("radio", &[("radio.rx.c0.stream", ezsdr_kernel::spec::Scalar::Int(0)), ("radio.tx.c0.stream", ezsdr_kernel::spec::Scalar::Int(0))]);
    let mut run = support::session_run_with(&temp, &profile, assembly, &initial);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 5_000_000)).unwrap();
    let capture = SessionAction::Vocabulary {
        ns: Namespace::parse("sink").unwrap(),
        verb: Ident::parse("capture").unwrap(),
        target: output("rec"),
        at: None,
        params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), Value::from(2_000))]),
    };
    run.submit(capture, None).unwrap();
    let manifest = end_session(run, clock, T0 + 20_000_000);
    assert!(manifest.clocks.sample_clocks.iter().any(|c| c.stream.path() == "mock/tx/s0"), "the transmit stream has its clock");
    assert_eq!(bursts_of(&manifest), 0, "no burst was sent");
    assert!(rig::read_capture(artifact(&manifest, "rec", 0), 1).remove(0).iter().all(|s| *s == (0.0, 0.0)), "nothing was transmitted into the receiver");
}

#[test]
fn v58_connect_two_streams_each_have_their_own_clock() {
    // Streams are set at connect: two receive and two transmit streams of one channel each, a
    // SampleClock for each, and no Action that put a channel in a stream.
    let temp = rig::TempDir::new("v58-two-streams");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::new());
    let one = ezsdr_kernel::spec::Scalar::Int(1);
    let zero = ezsdr_kernel::spec::Scalar::Int(0);
    let initial = support::asking("radio", &[("radio.rx.c0.stream", zero.clone()), ("radio.rx.c1.stream", one.clone()), ("radio.tx.c0.stream", zero), ("radio.tx.c1.stream", one)]);
    let run = support::session_run_with(&temp, &profile, assembly, &initial);
    let clock = root(&run);
    let manifest = end_session(run, clock, T0 + 20_000_000);
    let mut streams: Vec<_> = manifest.clocks.sample_clocks.iter().map(|c| c.stream.path().to_owned()).collect();
    streams.sort();
    // The recorder is linked to `rx0` only, and a receive stream with no link has no block to give a clock.
    assert_eq!(streams, ["mock/rx/s0", "mock/tx/s0", "mock/tx/s1"], "one clock for each stream that runs, none shared");
    assert_eq!(manifest.action_log.len(), 0, "connect did it all");
}

#[test]
fn v58_initial_values_are_the_first_values() {
    // The rate a Session asks for at connect is the rate of its first SampleClock, and the
    // configuration it starts with holds the frequency and gain; a rate the grid does not hold is
    // coerced under a Session's `warn` and recorded (SB-45).
    let temp = rig::TempDir::new("v58-initial-values");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::new());
    let num = |x: f64| ezsdr_kernel::spec::Scalar::Num(ezsdr_kernel::spec::Finite::new(x).unwrap());
    let initial = support::asking(
        "radio",
        &[
            ("radio.rx.c0.stream", ezsdr_kernel::spec::Scalar::Int(0)),
            ("radio.rx.s0.sample_rate_hz", num(19.5e6)),
            ("radio.rx.c0.frequency_hz", num(2.45e9)),
            ("radio.rx.c0.gain_db", num(10.0)),
            ("radio.rx.c0.antenna", ezsdr_kernel::spec::Scalar::from("TX/RX")),
        ],
    );
    let run = support::session_run_with(&temp, &profile, assembly, &initial);
    let clock = root(&run);
    let effective = run.effective()[&Ident::parse("radio").unwrap()].clone();
    assert_eq!(effective[&Key::parse("radio.rx.s0.sample_rate_hz").unwrap()], Value::num(20.0e6).unwrap(), "coerced to the grid");
    assert_eq!(effective[&Key::parse("radio.rx.c0.frequency_hz").unwrap()], Value::num(2.45e9).unwrap());
    assert_eq!(effective[&Key::parse("radio.rx.c0.gain_db").unwrap()], Value::num(10.0).unwrap());
    assert_eq!(effective[&Key::parse("radio.rx.c0.antenna").unwrap()], Value::from("TX/RX"));
    let manifest = end_session(run, clock, T0 + 5_000_000);
    let rx = manifest.clocks.sample_clocks.iter().find(|c| c.stream.path() == "mock/rx/s0").expect("the receive clock");
    assert_eq!((rx.nominal_rate.num(), rx.nominal_rate.den()), (20_000_000, 1), "the first clock has the asked rate");
    let preview = manifest.admission.coercions_preview.iter().find(|item| item.coercion.key.as_str() == "radio.rx.s0.sample_rate_hz").expect("the coercion is recorded");
    assert_eq!((preview.coercion.requested.clone(), preview.coercion.applied.clone()), (Value::num(19.5e6).unwrap(), Value::num(20.0e6).unwrap()));
    assert_eq!(manifest.spec.body["resources"]["radio"]["requires"]["radio.rx.s0.sample_rate_hz"]["value"], 19.5e6, "the Spec holds what was asked, not what was applied");
    assert_eq!(manifest.spec.body["requirements"]["vocabularies"], json!([{ "id": "radio", "major": 2 }]));
}

#[test]
fn v58_connect_refuses_what_the_layout_cannot_carry_at_validate() {
    // The rules a Spec's `requires` meets, RM-31 and RM-32 among them, judge an initial request at
    // Validate, before anything is prepared (SB-39).
    let ask = |requests: &[(&str, ezsdr_kernel::spec::Scalar)], name: &str| {
        let temp = rig::TempDir::new(name);
        // One 10 GbE link, as the default profile has (RM-32).
        let selector = json!({ "id": "mock", "master_clock_hz": [200_000_000], "image": ["hg"], "links": [1], "link_gbps": [10] });
        let profile = rig::session_profile("x3xx", selector, &temp.0, json!({}));
        let assembly = rig::assemble(&profile, BTreeMap::new());
        let run = support::session_run_with(&temp, &profile, assembly, &support::asking("radio", requests));
        run.finish()
    };
    use ezsdr_kernel::spec::{Finite, Scalar};
    let num = |x: f64| Scalar::Num(Finite::new(x).unwrap());
    // RM-31: a transmit channel in a stream and the same board's receive antenna on TX/RX.
    let duplex = ask(&[("radio.tx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.stream", Scalar::Int(0)), ("radio.rx.c0.antenna", Scalar::from("TX/RX"))], "v58-rm31");
    assert!(matches!(duplex.termination.reason, Termination::Failed { stage: Stage::Validate, .. }), "{:?}", duplex.termination.reason);
    assert!(duplex.admission.rejected.iter().any(|item| item.reason.starts_with("RM-31") && item.reason.contains("transmit channel c0")), "{:?}", duplex.admission.rejected);
    assert!(duplex.clocks.sample_clocks.is_empty(), "nothing started");
    // RM-32: 200 + 100 Msps is over one 10 GbE link; 200 + 50 is exactly at it.
    let streams =
        |second: f64| [("radio.rx.c0.stream", Scalar::Int(0)), ("radio.rx.c1.stream", Scalar::Int(1)), ("radio.rx.s0.sample_rate_hz", num(200.0e6)), ("radio.rx.s1.sample_rate_hz", num(second))];
    let over = ask(&streams(100.0e6), "v58-rm32-over");
    assert!(matches!(over.termination.reason, Termination::Failed { stage: Stage::Validate, .. }), "{:?}", over.termination.reason);
    assert!(over.admission.rejected.iter().any(|item| item.reason.starts_with("RM-32")), "{:?}", over.admission.rejected);
    assert!(over.clocks.sample_clocks.is_empty());
    let at = ask(&streams(50.0e6), "v58-rm32-at");
    assert!(!matches!(at.termination.reason, Termination::Failed { .. }), "{:?}", at.termination.reason);
    // A key no Vocabulary owns is refused at Validate too, and the Run exists to say so (RS-12).
    let unknown = ask(&[("zzz.x", Scalar::Int(1))], "v58-unknown-key");
    assert!(matches!(unknown.termination.reason, Termination::Failed { stage: Stage::Validate, .. }), "{:?}", unknown.termination.reason);
}

#[test]
fn v58_03_a_run_reproduces_from_its_own_manifest() {
    let temp = rig::TempDir::new("v58-03-reproduce");
    let samples = ramp(3_000);
    let (first, _) = link_run(&temp, "a", "b", "x3xx", true, coupling("a", "b", -6.0, 1_000, Some(-30.0), 11), &samples);
    let (bytes, waveform) = experiments::waveform_of(&samples);
    assert_eq!(first.inputs, vec![waveform.clone()]);
    let rerun = spec_run(&temp, &first.spec.body, &first.binding.body, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let clock = root(&rerun);
    let second = finish_at(rerun, clock, T0 + 25_000_000);
    assert_eq!(rig::determinism_projection(&first), rig::determinism_projection(&second));
}

#[test]
fn mr_34_a_scheduled_radio_value_update_is_refused_at_validate() {
    // A radio key with no update class (a rate: RM-27) cannot be scheduled in a Spec (RS-52, SB-16).
    let temp = rig::TempDir::new("mr-34-scheduled-refused");
    let (bytes, waveform) = experiments::waveform_of(&vec![(0.5, 0.0); 1_000]);
    let mut spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 1_000, Some(20_000));
    spec["schedule"].as_array_mut().unwrap().push(json!({
        "at": { "clock": "radio", "offset_ticks": 5_000 },
        "action": { "kind": "update_parameter", "target": { "kind": "resource", "resource": "radio", "path": "" }, "key": "radio.rx.s0.sample_rate_hz", "value": 1.0e6 }
    }));
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let run = ezsdr_kernel::coordinator::start_spec_run(&spec, &profile, assembly).expect("a refused Spec is a failed Run");
    assert!(
        matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, reason } } if reason.contains("radio.rx.s0.sample_rate_hz")),
        "the schedule's radio value update is the reason: {:?}",
        run.state()
    );
    // The control: the same Spec without that schedule item is not refused at Validate.
    let mut control = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 1_000, Some(20_000));
    control["schedule"].as_array_mut().unwrap().push(json!({
        "at": { "clock": "radio", "offset_ticks": 5_000 },
        "action": { "kind": "stop", "target": { "kind": "resource", "resource": "radio", "path": "" } }
    }));
    let (bytes, waveform) = experiments::waveform_of(&vec![(0.5, 0.0); 1_000]);
    let assembly = rig::assemble(&profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let run = ezsdr_kernel::coordinator::start_spec_run(&control, &profile, assembly).expect("the control Spec starts");
    assert!(!matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }), "{:?}", run.state());
    let _ = run.finish();
}

#[test]
fn mr_18_a_scheduled_receive_value_is_refused_by_the_provider_for_its_instant() {
    // A receive value is `cold` (RM-27), so a Spec's schedule of one passes validate; its Action carries an instant, which a staged value
    // never has (RM-21), so the Provider refuses it at run time, naming the cause. Values change through Session actions only.
    let temp = rig::TempDir::new("mr-18-scheduled-receive");
    let (bytes, waveform) = experiments::waveform_of(&vec![(0.5, 0.0); 1_000]);
    let mut spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 1_000, Some(20_000));
    spec["schedule"].as_array_mut().unwrap().push(json!({
        "at": { "clock": "radio", "offset_ticks": 5_000 },
        "action": { "kind": "update_parameter", "target": { "kind": "resource", "resource": "radio", "path": "" }, "key": "radio.rx.c0.frequency_hz", "value": 1.0e9 }
    }));
    let profile = rig::spec_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let assembly = rig::assemble(&profile, BTreeMap::from([(waveform.hash.clone(), bytes)]));
    let run = ezsdr_kernel::coordinator::start_spec_run(&spec, &profile, assembly).expect("a Spec starts");
    assert!(!matches!(run.state(), RunState::CleanedUp { termination: Termination::Failed { stage: Stage::Validate, .. } }), "validate passes: {:?}", run.state());
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 60_000_000);
    let refused: Vec<_> = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].payload["kind"], "update_parameter");
    assert!(refused[0].payload["reason"].as_str().unwrap().contains("instant"), "{}", refused[0].payload["reason"]);
}
