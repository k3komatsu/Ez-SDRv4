//! The bench steps of `plan/phase7/bench.md` (UR-34), on a USRP X310 with one OBX
//! (`x310-obx`), the profile the one its front ends name.
//! Compiled only with the feature `uhd`, every test ignored, the device from
//! `EZSDR_UHD_ARGS` (GZ-8). Run each as `bench.md` says;
//! each prints what `bench-results.md` records.
#![cfg(feature = "uhd")]

mod common;

use std::sync::Arc;
use std::time::{Duration as Wall, Instant};

use common::*;
use ezsdr_kernel::module_api::Authority;
use ezsdr_kernel::time::{ClockRegistry, TimePoint};
use ezsdr_radio_uhd::{Device, DeviceAuthority, Dir, Settings, UhdRadio};

fn args() -> String {
    std::env::var("EZSDR_UHD_ARGS").expect("EZSDR_UHD_ARGS names the device, e.g. addr=192.168.40.2")
}

fn usrp() -> Arc<dyn Device> {
    ezsdr_radio_uhd::open(&args()).expect("the USRP opens")
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b1_probe() {
    let device = usrp();
    println!("B1 describe: {}", serde_json::to_string_pretty(&device.describe()).unwrap());
    assert_eq!(device.master_clock_rate(), 200_000_000, "B0.3: add master_clock_rate=200e6");
    // The profile's channels and front ends, as UR-5 checks them.
    let profile = profile_of(&*device);
    println!("B1 profile: {}", profile.name());
    assert_eq!(profile, ezsdr_radio_uhd::profile::Profile::X310Obx, "bench.md: the bench is one OBX in slot A");
    let need = profile.description(2_000).max_channels as usize;
    for dir in [Dir::Rx, Dir::Tx] {
        let have = device.channels(dir);
        let names: Vec<_> = (0..have).map(|chan| device.front_end(dir, chan)).collect();
        println!("B1 {dir:?}: {have} channels, front ends {names:?}");
        assert!(have >= need, "{} needs {need} {dir:?} channels", profile.name());
        for name in &names[..need] {
            assert!(name.as_ref().unwrap().starts_with(profile.front_end()), "{name:?} is not a {} front end", profile.front_end());
        }
    }
    println!("B1 ref_locked: {:?}", device.ref_locked());
    let mut last = device.time_now().unwrap();
    for _ in 0..3 {
        std::thread::sleep(Wall::from_millis(100));
        let now = device.time_now().unwrap();
        let advanced = now - last;
        println!("B1 time advanced {advanced} ticks in 100 ms");
        assert!((advanced - 20_000_000).abs() <= 200_000, "{advanced}");
        last = now;
    }
    // What a Session would inherit if nothing configured the transmitter (UR-25).
    for chan in 0..device.channels(Dir::Tx) {
        println!("B1 tx {chan}: {:?}", device.apply(Dir::Tx, chan, &Settings::default(), None));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b2_authority() {
    let device = usrp();
    let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", &args()).unwrap();
    let (time, root) = (authority.time(), authority.root());
    let mut late = Vec::new();
    for _ in 0..100 {
        let at = TimePoint::new(root, time.now(root).unwrap().ticks + 2_000_000);
        time.schedule(at, Box::new(|_| {})).unwrap();
        let begun = Instant::now();
        authority.next_wakeup().unwrap();
        late.push(begun.elapsed().as_micros() as i64 - 10_000);
    }
    late.sort();
    println!("B2 lateness µs: median {} max {}", late[50], late[99]);
    assert!(late[50] < 1_000 && late[99] < 25_000);
    let relations = authority.relations();
    println!("B2 relations: {}", serde_json::to_string_pretty(&relations).unwrap());
    assert_eq!(relations.len(), 2);
    let begun = Instant::now();
    let (ours, theirs) = (time.now(root).unwrap().ticks, device.time_now().unwrap());
    std::thread::sleep(Wall::from_secs(10));
    let drift = (time.now(root).unwrap().ticks - ours) - (device.time_now().unwrap() - theirs);
    println!("B2 anchor drift over {:?}: {drift} ticks", begun.elapsed());
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b3_receive_at_t0() {
    let manifest = rehearse_receive_at_t0(usrp());
    println!("B3 applied: {}", section(&manifest, "applied"));
    println!("B3 timing: {}", section(&manifest, "timing"));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b4_capture_at_a_sample_index() {
    let _ = rehearse_capture_at_a_sample_index(usrp());
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b5_overflow() {
    for stall in [300, 1_000, 2_000] {
        let result = std::panic::catch_unwind(|| rehearse_overflow(usrp(), Wall::from_millis(500), Wall::from_millis(stall)));
        match result {
            Ok(manifest) => {
                println!("B5 overflowed with a {stall} ms stall: {}", section(&manifest, "stats"));
                return;
            }
            Err(_) => println!("B5 no overflow with a {stall} ms stall; the socket buffer absorbed it"),
        }
    }
    panic!("B5: no stall overflowed; record net.core.rmem_max");
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b6_txrx_and_repeat() {
    let (heard, looped) = rehearse_txrx_and_repeat(usrp(), false);
    println!("B6 bursts: {} / {}", section(&heard, "bursts"), section(&looped, "bursts"));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b7_session_loopback() {
    let manifest = rehearse_session_loopback(usrp(), false);
    println!("B7 time errors: {:?}", events_of(&manifest, "radio.TIME_ERROR"));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_leads() {
    // A measurement, not a pass/fail step: bursts at leads from their receipt, with the
    // device's late reports counted (the device lead, the delivery allowance).
    for lead_us in [10_000, 5_000, 3_000, 2_000, 1_500, 1_000, 500] {
        let dir = TempDir::new();
        let device = usrp();
        let mut run = session(&bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), true), device);
        past_t0(&mut run, ms(1));
        assert!(admitted(&run.submit(set("radio.tx.channels", ezsdr_kernel::spec::Value::Int(1)), None).unwrap()));
        let (bytes, _) = waveform_of(&pn(100));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx") && r.ended_at.is_none()).unwrap();
        let at = TimePoint::new(clock.domain, (run.now().ticks + lead_us * 200 - clock.origin.ticks).div_euclid(200) + 1);
        let _ = run.submit(verb("send", "radio/tx", Some(at), &[]), Some(&bytes));
        wait(&mut run, ms(100));
        let manifest = run.finish();
        println!("B8 lead {lead_us} µs: TIME_ERROR {:?}", events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>());
        println!("B8 lead {lead_us} µs: timing {}", section(&manifest, "timing"));
        println!("B8 lead {lead_us} µs: bursts {}", section(&manifest, "bursts"));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_unplug() {
    // Manual: unplug the 10 GbE cable during the next 60 s.
    let dir = TempDir::new();
    let device = usrp();
    let mut run = spec_run(&receive_spec(1, 1e6, bench_hz(&*device), None), &bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false), device, Default::default());
    println!("B9: unplug the cable now");
    let result = run.run_until_end(after(&run, ms(60_000)));
    let manifest = run.finish();
    println!("B9 result {result:?}; termination {:?}", manifest.termination.reason);
    println!("B9 DEVICE_LOST: {:?}", events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_usrp2_probe() {
    let device = usrp();
    println!("B9 USRP2 describe: {}", serde_json::to_string_pretty(&device.describe()).unwrap());
    let binding = serde_json::from_value(serde_json::json!({
        "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } },
        "profile": profile_of(&*device).profile_ref(),
        "selector": { "args": args() }
    }))
    .unwrap();
    let name = profile_of(&*device).name();
    let refused = UhdRadio::from_binding(&binding, device).err().expect("UR-5 refuses a USRP2");
    println!("B9 UR-5: {}", refused.message);
    assert!(refused.message.starts_with(&format!("UR-5: profile {name} needs a 200 MHz master clock")));
}
