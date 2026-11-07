mod support;

use std::collections::BTreeMap;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::session::SessionAction;
use ezsdr_kernel::spec::{Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::time::TimePoint;
use serde_json::json;
use support::{T0, end_session, loopback_run, resource, root, section};

/// Plays a repeat on stream 0, then submits a `start_repeat` with `params` while it plays.
fn play_then_repeat(test: &str, params: BTreeMap<Key, Value>) -> Manifest {
    let temp = rig::TempDir::new(test);
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    run.submit(
        SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: Default::default() },
        Some(&bytes),
    )
    .unwrap();
    run.advance_to(TimePoint::new(clock, T0 + 20_000_000)).unwrap();
    run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params }, Some(&bytes))
        .unwrap();
    end_session(run, clock, T0 + 60_000_000)
}

#[test]
fn ea_16_a_marked_start_repeat_switches_and_an_unmarked_one_is_refused() {
    let mark = BTreeMap::from([(Key::parse("radio.repeat_operation").unwrap(), Value::Scalar(Scalar::from("switch")))]);
    let marked = play_then_repeat("ea-16-switch-marked", mark);
    let rows = section(&marked, "mock", "ezsdr.radio.mock.switches").as_array().unwrap();
    assert_eq!(rows.len(), 1, "the marked burst is a switch");
    assert_eq!(rows[0]["outcome"], "applied");
    assert!(!marked.events.delivered.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED), "a switch while a repeat plays is taken");

    // The same verb with no mark is a plain repeat, which a playing stream refuses.
    let plain = play_then_repeat("ea-16-switch-plain", BTreeMap::new());
    assert!(section(&plain, "mock", "ezsdr.radio.mock.switches").as_array().unwrap().is_empty());
    assert!(plain.events.delivered.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
}

#[test]
fn ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end() {
    // RM-15, RM-16 (sequence 1): an untimed repeat submitted right after a transmit stream's Stop is placed past the Stop region, with no
    // TIME_ERROR and no refusal. The region is the 10 ms window past the Stop's sample (KL-13, KL-15).
    let temp = rig::TempDir::new("ea-16-untimed-after-stop");
    let profile = rig::session_profile("x3xx", json!({ "id": "mock" }), &temp.0, json!({}));
    let mut run = loopback_run(&temp, &profile);
    let clock = root(&run);
    run.advance_to(TimePoint::new(clock, T0 + 1_000_000)).unwrap();
    let (bytes, _) = experiments::waveform(1_000);
    let repeat =
        SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: resource("radio/tx/s0"), at: None, params: Default::default() };
    run.submit(repeat.clone(), Some(&bytes)).unwrap();
    run.advance_to(TimePoint::new(clock, T0 + 20_000_000)).unwrap();
    run.submit(SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    let now = run.now();
    let stop_root = now.ticks_in(now.domain()).unwrap();
    let record = run.sample_clocks().into_iter().find(|r| r.stream.path().ends_with("tx/s0")).expect("the transmit SampleClock");
    let origin = record.origin.ticks_in(record.origin.domain()).unwrap();
    let stop = (stop_root - origin) / record.root_ticks_per_tick.num() as i64;
    run.submit(repeat, Some(&bytes)).unwrap();
    let manifest = end_session(run, clock, T0 + 60_000_000);
    let rows = section(&manifest, "mock", "ezsdr.radio.mock.bursts").as_array().unwrap();
    assert_eq!(rows.len(), 2, "the repeat and the one after the Stop: {rows:?}");
    let gap = rows[1]["target"]["ticks"].as_i64().unwrap() - stop;
    assert!(gap >= 9_900, "the repeat starts past the Stop region, {gap} samples after the Stop: {rows:?}");
    assert!(rows[1]["late_by"].is_null(), "an untimed repeat reports no lateness: {rows:?}");
    assert!(manifest.events.delivered.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR && event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED));
}
