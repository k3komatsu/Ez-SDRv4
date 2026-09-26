//! The Kernel path end to end on the wall-clock fake device: no hardware needed.
//! The Specs are the acceptance crate's own experiment documents, unchanged.

use std::collections::BTreeMap;

use ezsdr_acceptance::experiments;
use ezsdr_acceptance::rig::{self, TempDir};
use ezsdr_kernel::coordinator::{self, RunHandleError};
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::time::TimePoint;
use ezsdr_uhd_spike::{ProfileOpts, assemble, open_devices, profile};
use serde_json::{Value, json};

const RATE: f64 = 1e6;

fn run(spec: &Value, radio: Value, inputs: BTreeMap<ezsdr_kernel::hash::ContentHash, Vec<u8>>, seconds: f64, dir: &TempDir) -> Manifest {
    let o = ProfileOpts {
        args: "fake",
        radio,
        clock_source: "internal",
        time_source: "internal",
        rf_path: "cabled",
        start_lead_ns: 300_000_000,
    };
    let doc = profile(&o, &dir.0);
    let binding = ezsdr_kernel::binding::BindingProfile::from_json(&doc).unwrap();
    let devices = open_devices(&binding).unwrap();
    let mut run = coordinator::start_spec_run(spec, &doc, assemble(&doc, inputs, &devices).unwrap()).expect("Spec Run starts");
    let t0 = run.start_instant();
    let horizon = t0.map(|t| TimePoint::new(t.domain, t.ticks + (seconds * 200e6) as i64));
    if let Some(h) = horizon {
        match run.run_until_end(h) {
            Ok(()) | Err(RunHandleError::Ended { .. }) => {}
            Err(e) => panic!("{e}"),
        }
    }
    run.finish()
}

fn show(m: &Manifest) -> String {
    let v = serde_json::to_value(m).unwrap();
    serde_json::to_string_pretty(&json!({
        "termination": v["termination"], "run": v["run"]["execution_class"], "failure": v["sections"]["ezsdr.failure"],
        "sections": v["sections"], "artifacts": v["artifacts"], "counters": v["events"]["counters"],
    }))
    .unwrap()
}

#[test]
fn m1_timed_capture_through_the_kernel() {
    let dir = TempDir::new("m1");
    // v3 behaviour 2 (§61): capture 5 000 samples starting at sample 20 000 after T0.
    let mut spec = experiments::receive(1, RATE, 1e9, None);
    spec["schedule"] = json!([]);
    let spec = experiments::with_timed_capture(spec, 5_000, 20_000);
    let m = run(&spec, json!({}), BTreeMap::new(), 0.2, &dir);
    let summary = show(&m);
    let cap = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture").unwrap_or_else(|| panic!("no capture:\n{summary}"));
    let samples = rig::read_capture(cap, 1);
    assert_eq!(samples[0].len(), 5_000, "{summary}");
    // The fake's ramp is (device sample index mod 65536) / 65536, so the first captured
    // sample says which device sample the capture started on: T0's plus 20 000.
    let rx = m.clocks.sample_clocks.iter().find(|c| c.stream.path == "usrp/rx").expect("rx clock");
    let ratio = rx.root_ticks_per_tick.num() as i64;
    let want = |i: i64| ((rx.origin.ticks / ratio + 20_000 + i) % 65_536) as f32 / 65_536.0;
    assert_eq!(samples[0][0].0, want(0), "{summary}");
    assert_eq!(samples[0][4_999].0, want(4_999), "{summary}");
    assert_eq!(cap.continuity[0].valid[0][0].start.ticks, 20_000, "{summary}");
    println!("{summary}");
}

fn ramp(n: usize) -> Vec<(f32, f32)> {
    (0..n).map(|i| (0.5 + i as f32 / (2 * n) as f32, -0.25)).collect()
}

#[test]
fn m3_timed_burst_and_capture_loop_back() {
    let dir = TempDir::new("m3");
    // v61_03's Spec: a burst at sample 5 000 on the transmit clock, a capture at 5 000 on
    // the receive clock. On the fake, receive is transmit looped back sample for sample.
    let samples = ramp(1_000);
    let (bytes, waveform) = experiments::waveform_of(&samples);
    let spec = experiments::with_timed_capture(experiments::transmit(RATE, &waveform, false, "drop_and_flag", 5_000, None), 1_000, 5_000);
    let m = run(&spec, json!({}), BTreeMap::from([(waveform.hash.clone(), bytes)]), 0.2, &dir);
    let summary = show(&m);
    let cap = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture").unwrap_or_else(|| panic!("no capture:\n{summary}"));
    let got = rig::read_capture(cap, 1).remove(0);
    // The transmit clock starts at arm and the receive clock at T0; the Kernel resolves
    // both schedule entries against T0 (KC-16) and re-expresses the burst on the transmit
    // clock (SC-23b), so both land on the same device instant.
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> =
        serde_json::from_value(serde_json::to_value(&m).unwrap()["sections"]["ezsdr.radio.uhd.usrp.bursts"].clone()).unwrap();
    assert_eq!(bursts.len(), 1, "{summary}");
    let tx = m.clocks.sample_clocks.iter().find(|c| c.stream.path == "usrp/tx").unwrap();
    let rx = m.clocks.sample_clocks.iter().find(|c| c.stream.path == "usrp/rx").unwrap();
    let ratio = rx.root_ticks_per_tick.num() as i64;
    let burst_at = tx.origin.ticks + bursts[0].target.ticks * ratio;
    let capture_at = rx.origin.ticks + 5_000 * ratio;
    assert_eq!(burst_at, capture_at, "{summary}");
    assert_eq!(got.len(), 1_000);
    assert_eq!(got, samples, "{summary}");
}

#[test]
fn m2_a_stalled_host_is_an_overflow_with_a_gap() {
    let dir = TempDir::new("m2");
    let mut spec = experiments::receive(1, RATE, 1e9, None);
    spec["schedule"] = json!([]);
    // 400 000 samples = 0.4 s from sample 0; the rx loop stalls 300 ms after 50 ms.
    let spec = experiments::with_timed_capture(spec, 400_000, 0);
    let m = run(&spec, json!({ "rx_stall_after_ms": 400, "rx_stall_ms": 300 }), BTreeMap::new(), 1.0, &dir);
    let summary = show(&m);
    let v = serde_json::to_value(&m).unwrap();
    assert!(v["sections"]["ezsdr.radio.uhd.usrp.stats"]["rx_overflows"].as_i64().unwrap_or(0) >= 1, "{summary}");
    let overflows: Vec<_> = m.events.delivered.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::RX_OVERFLOW).collect();
    assert_eq!(overflows.len(), 1, "{summary}");
    let cap = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture").unwrap_or_else(|| panic!("no capture:\n{summary}"));
    println!("overflow {:?}\ncontinuity {}", overflows[0].payload, serde_json::to_string(&cap.continuity).unwrap());
    assert!(cap.continuity[0].valid[0].len() >= 2, "a gap splits the valid ranges\n{summary}");
}

#[test]
fn m5_repeat_wraps_and_stops() {
    let dir = TempDir::new("m5");
    let (bytes, waveform) = experiments::waveform_of(&ramp(1_000));
    let spec = experiments::transmit(RATE, &waveform, true, "send_asap_and_flag", 10_000, None);
    let m = run(&spec, json!({}), BTreeMap::from([(waveform.hash.clone(), bytes)]), 0.3, &dir);
    let summary = show(&m);
    println!("{summary}");
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> =
        serde_json::from_value(serde_json::to_value(&m).unwrap()["sections"]["ezsdr.radio.uhd.usrp.bursts"].clone()).unwrap();
    assert_eq!(bursts.len(), 1, "{summary}");
    assert!(bursts[0].wraps >= 100, "{summary}");
    assert_eq!(bursts[0].end, ezsdr_kernel::stream::BurstEnd::Stop, "{summary}");
}

#[test]
fn m6_session_capture_at_a_sample_index() {
    use ezsdr_kernel::id::ResourceId;
    use ezsdr_kernel::session::{Outcome, SessionAction};
    use ezsdr_kernel::spec::{Ident, Key, Namespace, Value as V};
    let dir = TempDir::new("m6");
    let o = ProfileOpts { args: "fake", radio: json!({}), clock_source: "internal", time_source: "internal", rf_path: "cabled", start_lead_ns: 300_000_000 };
    let doc = ezsdr_uhd_spike::session_profile(&o, &dir.0);
    let devices = open_devices(&ezsdr_kernel::binding::BindingProfile::from_json(&doc).unwrap()).unwrap();
    let mut run = coordinator::connect(&doc, assemble(&doc, BTreeMap::new(), &devices).unwrap(), ezsdr_kernel::run::Lease::attached())
        .expect("Session connects");
    let Some(t0) = run.start_instant() else { panic!("no T0:\n{}", show(&run.finish())) };
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + 200_000)).unwrap();
    let rx = run.sample_clocks().into_iter().find(|r| r.stream.path == "usrp/rx").expect("rx clock");
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: ResourceId::parse("rec").unwrap(),
                at: Some(TimePoint::new(rx.domain, 12_345)),
                params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), V::Int(100))]),
            },
            None,
        )
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }), "{entry:?}");
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + 40_000_000)).unwrap();
    let m = run.finish();
    let summary = show(&m);
    let cap = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture").unwrap_or_else(|| panic!("no capture:\n{summary}"));
    let got = rig::read_capture(cap, 1).remove(0);
    let ratio = rx.root_ticks_per_tick.num() as i64;
    let want = |i: i64| ((rx.origin.ticks / ratio + 12_345 + i) % 65_536) as f32 / 65_536.0;
    assert_eq!(got[0].0, want(0), "{summary}");
    assert_eq!(got[99].0, want(99), "{summary}");
}

/// Vision §57's `sdr.tx.repeat(x); y = sdr.rx.capture(N)` as v57_a runs it on the Mock,
/// with one wait the Mock does not need (finding K5).
#[test]
fn m7_session_loopback_repeat_and_capture() {
    use ezsdr_kernel::id::ResourceId;
    use ezsdr_kernel::session::{Outcome, SessionAction};
    use ezsdr_kernel::spec::{Ident, Key, Namespace, Value as V};
    let dir = TempDir::new("m7");
    let o = ProfileOpts { args: "fake", radio: json!({}), clock_source: "internal", time_source: "internal", rf_path: "cabled", start_lead_ns: 300_000_000 };
    let doc = ezsdr_uhd_spike::session_profile(&o, &dir.0);
    let devices = open_devices(&ezsdr_kernel::binding::BindingProfile::from_json(&doc).unwrap()).unwrap();
    let mut run = coordinator::connect(&doc, assemble(&doc, BTreeMap::new(), &devices).unwrap(), ezsdr_kernel::run::Lease::attached()).unwrap();
    let t0 = run.start_instant().unwrap();
    run.advance_to(TimePoint::new(t0.domain, t0.ticks + 200_000)).unwrap();
    let samples = ramp(1_000);
    let (bytes, _) = experiments::waveform_of(&samples);
    let a = run.submit(SessionAction::SetParameter { target: ResourceId::parse("radio").unwrap(), key: Key::parse("radio.tx.channels").unwrap(), value: V::Int(1) }, None).unwrap();
    let now = run.now();
    let wait = std::env::var("NO_WAIT").is_err();
    if wait {
        run.advance_to(TimePoint::new(now.domain, now.ticks + 4_000_000)).unwrap();
    }
    let b = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("radio").unwrap(), verb: Ident::parse("start_repeat").unwrap(), target: ResourceId::parse("radio/tx").unwrap(), at: None, params: Default::default() }, Some(&bytes)).unwrap();
    let c = run.submit(SessionAction::Vocabulary { ns: Namespace::parse("sink").unwrap(), verb: Ident::parse("capture").unwrap(), target: ResourceId::parse("rec").unwrap(), at: None, params: BTreeMap::from([(Key::parse("sink.capture_samples").unwrap(), V::Int(5_000))]) }, None).unwrap();
    for e in [&a, &b, &c] {
        println!("{:?}", e.outcome);
    }
    assert!([&a, &b, &c].iter().all(|e| matches!(e.outcome, Outcome::Admitted { .. })));
    let now = run.now();
    run.advance_to(TimePoint::new(now.domain, now.ticks + 40_000_000)).unwrap();
    let m = run.finish();
    let summary = show(&m);
    let cap = m.artifacts.iter().find(|a| a.kind.as_str() == "sink.capture").unwrap_or_else(|| panic!("no capture:\n{summary}"));
    let got = rig::read_capture(cap, 1).remove(0);
    assert_eq!(got.len(), 5_000);
    // The capture is admitted at "now" and the repeat at now + the Provider's 2 ms lead
    // (RS-19), so the burst begins inside the capture; from there on every sample is
    // the waveform's, in cyclic order, with no gap at the wraps.
    let d = got.iter().position(|s| samples.contains(s)).unwrap_or_else(|| panic!("nothing transmitted was captured\n{summary}"));
    let start = samples.iter().position(|s| *s == got[d]).unwrap();
    assert_eq!(start, 0, "the burst starts at the waveform's first sample\n{summary}");
    assert!(d < 4_000, "{d}");
    for (i, s) in got.iter().enumerate().skip(d) {
        assert_eq!(*s, samples[(i - d) % 1_000], "sample {i}\n{summary}");
    }
    println!("burst begins {d} samples into the capture");
}
