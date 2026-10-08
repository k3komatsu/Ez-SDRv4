mod support;

use std::collections::BTreeMap;

use ezsdr_acceptance::{experiments, rig};
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::manifest::Manifest;
use ezsdr_kernel::session::{Outcome, SessionAction};
use ezsdr_kernel::spec::{Ident, Key, Namespace, Value};
use ezsdr_kernel::time::TimePoint;
use serde_json::json;
use support::{artifact, finish_at, root, section, session_run, spec_run, T0};

#[test]
fn v61_01_repeat_is_continuous_across_the_wrap() {
    let (bytes, waveform) = experiments::waveform(1_000);
    let spec = experiments::transmit(1.0e6, &waveform, true, "send_asap_and_flag", 10_000, None);
    let temp = rig::TempDir::new("v61-01");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash, bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 15_000_000);

    let stats = section(&manifest, "ezsdr.radio.mock.mock.stats");
    let tx_blocks = stats["tx_blocks"].as_u64().expect("transmit block count");
    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> =
        serde_json::from_value(section(&manifest, "ezsdr.radio.mock.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].samples, tx_blocks * 1_000);
    assert_eq!(bursts[0].blocks, u32::try_from(tx_blocks).unwrap());
}

#[test]
fn v61_02_capture_starts_at_the_requested_sample_index() {
    let temp = rig::TempDir::new("v61-02");
    let profile = rig::session_profile(
        "x310-like",
        json!({ "id": "mock", "rx_test_pattern": "ramp" }),
        &temp.0,
        json!({}),
    );
    let mut run = session_run(&temp, &profile);
    let clock = root(&run);
    // RM-25: the receive clock is registered with the first block, published at 2 ms.
    run.advance_to(TimePoint::new(clock, T0 + 3_000_000)).unwrap();
    let rx_clock = run
        .sample_clocks()
        .into_iter()
        .find(|record| record.stream == ResourceId::parse("mock/rx").unwrap())
        .expect("receive SampleClock registered");
    let entry = run
        .submit(
            SessionAction::Vocabulary {
                ns: Namespace::parse("sink").unwrap(),
                verb: Ident::parse("capture").unwrap(),
                target: ResourceId::parse("rec").unwrap(),
                at: Some(TimePoint::new(rx_clock.domain, 12_345)),
                params: BTreeMap::from([(
                    Key::parse("sink.capture_samples").unwrap(),
                    Value::Int(100),
                )]),
            },
            None,
        )
        .unwrap();
    assert!(matches!(entry.outcome, Outcome::Admitted { .. }));
    let manifest = support::end_session(run, clock, T0 + 50_000_000);
    let samples = rig::read_capture(artifact(&manifest, "rec_0"), 1);
    assert_eq!(samples[0][0].0, 12_345.0 / 65_536.0);
    assert_eq!(samples[0][99].0, 12_444.0 / 65_536.0);
}

#[test]
fn v61_03_timed_start_of_tx_and_capture() {
    let (bytes, waveform) = experiments::waveform(1_000);
    let spec = experiments::with_timed_capture(
        experiments::transmit(1.0e6, &waveform, false, "drop_and_flag", 5_000, None),
        1_000,
        5_000,
    );
    let temp = rig::TempDir::new("v61-03");
    let profile = rig::spec_profile("x310-like", json!({ "id": "mock" }), &temp.0, json!({}));
    let run = spec_run(&temp, &spec, &profile, BTreeMap::from([(waveform.hash, bytes)]));
    let clock = root(&run);
    let manifest = finish_at(run, clock, T0 + 10_000_000);

    let bursts: Vec<ezsdr_kernel::stream::BurstRecord> =
        serde_json::from_value(section(&manifest, "ezsdr.radio.mock.mock.bursts").clone()).unwrap();
    assert_eq!(bursts.len(), 1);
    assert_eq!(bursts[0].target.ticks, 2_005_000);
    let capture = artifact(&manifest, "rec_0");
    assert_eq!(capture.continuity[0].valid[0][0].start.ticks, 5_000);
}

#[test]
fn v61_04_pps_source_is_armed_first_and_streams_align() {
    let temp = rig::TempDir::new("v61-04");
    let mut profile = rig::spec_profile("x310-like", json!({ "id": "unused" }), &temp.0, json!({}));
    let radio = profile["bindings"]["radio"].clone();
    let recorder = profile["bindings"]["rec"].clone();
    let mut pps = radio.clone();
    pps["selector"] = json!({ "id": "pps" });
    let mut follow = radio;
    follow["selector"] = json!({ "id": "follow", "arm_after": ["pps"] });
    let mut rec_pps = recorder.clone();
    rec_pps["selector"]["dir"] = json!(temp.0.join("rec-pps").to_string_lossy().to_string());
    let mut rec_follow = recorder;
    rec_follow["selector"]["dir"] = json!(temp.0.join("rec-follow").to_string_lossy().to_string());
    profile["bindings"] = json!({ "pps": pps, "follow": follow, "rec_pps": rec_pps, "rec": rec_follow, "sim": profile["bindings"]["sim"] });

    let mut first_link = profile["placements"]["links"][0].clone();
    first_link["from"]["component"] = json!("pps");
    first_link["to"]["component"] = json!("rec_pps");
    let mut second_link = first_link.clone();
    second_link["from"]["component"] = json!("follow");
    second_link["to"]["component"] = json!("rec");
    profile["placements"]["links"] = json!([first_link, second_link]);

    let radio_resource = json!({
        "kind": "radio.device",
        "requires": {
            "radio.rx.channels": { "kind": "eq", "value": 1 },
            "radio.rx.sample_rate_hz": { "kind": "eq", "value": 1.0e6 },
            "radio.rx.frequency_hz": { "kind": "eq", "value": 1.0e9 }
        }
    });
    let output = |id: &str, component: &str| json!({
        "id": id,
        "kind": "sink.capture",
        "feed": { "port": { "component": component, "port": "rx" }, "policy": "drop_oldest", "capacity": 64 },
        "params": {}
    });
    let spec = json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": { "pps": radio_resource.clone(), "follow": radio_resource },
        "outputs": [output("rec_pps", "pps"), output("rec", "follow")],
        "policies": {},
        "extensions": {}
    });
    let run = spec_run(&temp, &spec, &profile, BTreeMap::new());
    let clock = root(&run);
    let manifest: Manifest = finish_at(run, clock, T0 + 1_000_000);

    let plan = manifest.plan.as_ref().expect("execution plan recorded");
    let ids: Vec<&str> = plan.fragments.iter().map(|fragment| fragment.id.as_str()).collect();
    assert!(ids.iter().position(|id| *id == "pps") < ids.iter().position(|id| *id == "follow"));
    assert!(plan.deps.contains(&(Ident::parse("pps").unwrap(), Ident::parse("follow").unwrap())));
    for stream in ["pps/rx", "follow/rx"] {
        let record = manifest
            .clocks
            .sample_clocks
            .iter()
            .find(|record| record.stream == ResourceId::parse(stream).unwrap())
            .expect("receive SampleClock recorded");
        assert_eq!(record.origin, TimePoint::new(clock, T0));
    }
}
