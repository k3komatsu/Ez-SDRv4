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
                // The restart gap MR-3 claims is 50 ms (at 10 Msps, 100 ns a sample).
                let map = &capture_of(&manifest, "rec").continuity[0];
                let drops: Vec<_> = map.gaps.iter().filter(|g| g.cause == ezsdr_kernel::stream::GapCause::LinkDrop {}).collect();
                println!(
                    "B5 {} gaps: {} link drops of {} samples in all, from sample {:?} to {:?}",
                    map.gaps.len(),
                    drops.len(),
                    drops.iter().map(|g| g.len).sum::<u64>(),
                    drops.first().map(|g| g.start.ticks),
                    drops.last().map(|g| g.start.ticks + g.len as i64)
                );
                for gap in map.gaps.iter().filter(|g| g.cause != ezsdr_kernel::stream::GapCause::LinkDrop {}) {
                    println!("B5 gap {:?} at receive sample {} for {} samples ({} ms at 10 Msps), lost {:?}", gap.cause, gap.start.ticks, gap.len, gap.len as f64 / 1e4, gap.lost);
                }
                println!("B5 capture {} … {}, {} valid segment(s)", map.first.ticks, map.end.ticks, map.valid[0].len());
                println!("B5 timing {}", section(&manifest, "timing"));
                println!("B5 sample clocks: {:?}", manifest.clocks.sample_clocks);
                for event in events_of(&manifest, "radio.RX_OVERFLOW") {
                    println!("B5 RX_OVERFLOW: {:?}", ezsdr_radio::payloads::RxOverflowPayload::from_payload(&event.payload));
                }
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
        let envelope = bench_envelope(&*device);
        let mut run = session(&bench_profile(&*device, &dir, serde_json::json!({}), envelope, true), device);
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

// ---------------------------------------------------------------- B8 below the Module
//
// bench.md B8's rows that the Module's own rules keep a Session from probing (UR-21
// refuses a burst closer than its 2 ms device lead; the restart lead, the in-flight
// window and the queue depth are constants): measured on the `Device` itself, as the
// Module would drive it. RF as bench.md says: the transmitter only inside
// `SAFE_TX_HZ`, at most 2 Msps (so the emission stays inside the bench envelope's
// 999–1001 MHz), 0 dB, amplitude ≤ 0.4, into the 30 dB loopback.

const SAFE_TX_HZ: std::ops::RangeInclusive<f64> = 999_500_000.0..=1_000_500_000.0;
const TICKS_PER_MS: i64 = 200_000;

/// The device with its time set to zero on the internal reference.
fn raw() -> Arc<dyn Device> {
    let device = usrp();
    device.set_sources("internal", "internal").unwrap();
    device.set_time_zero(false).unwrap();
    std::thread::sleep(Wall::from_millis(100));
    device
}

fn raw_rx(device: &dyn Device, rate: f64, freq: f64) {
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("RX2".to_owned()) };
    device.apply(Dir::Rx, 0, &settings, None).unwrap();
    device.rx_open(1).unwrap();
}

/// The transmitter inside bench.md's RF conditions, or a panic before anything is sent.
fn raw_tx(device: &dyn Device, rate: f64, freq: f64) {
    assert!(SAFE_TX_HZ.contains(&freq) && rate <= 2e6, "bench.md RF safety: {freq} Hz at {rate} S/s");
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("TX/RX".to_owned()) };
    let applied = device.apply(Dir::Tx, 0, &settings, None).unwrap();
    assert!(applied.gain == 0.0 && SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
    device.tx_open(1).unwrap();
}

fn iq(samples: &[(f32, f32)]) -> Vec<ezsdr_radio_uhd::Iq> {
    samples.iter().map(|(re, im)| [*re, *im]).collect()
}

/// What `rx_recv` returned over a stretch: the blocks' (first tick, length), and every
/// other outcome with the device time it was seen at.
#[derive(Default, Debug)]
struct Read {
    blocks: Vec<(i64, usize)>,
    samples: Vec<(i64, ezsdr_radio_uhd::Iq)>,
    other: Vec<(i64, String)>,
}

impl Read {
    fn end(&self, per_sample: i64) -> Option<i64> {
        self.blocks.last().map(|(t, n)| t + *n as i64 * per_sample)
    }
}

/// Reads until `until(read)` holds or `limit` passes; `keep`, the ticks a sample, keeps
/// the samples with their ticks.
fn read(device: &dyn Device, limit: Wall, keep: Option<i64>, until: impl Fn(&Read) -> bool) -> Read {
    let mut out = Read::default();
    let begun = Instant::now();
    while begun.elapsed() < limit && !until(&out) {
        match device.rx_recv(2_000, Wall::from_millis(100)) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                out.blocks.push((first_tick, samples[0].len()));
                if let Some(per) = keep {
                    out.samples.extend(samples[0].iter().enumerate().map(|(i, s)| (first_tick + i as i64 * per, *s)));
                }
            }
            other => out.other.push((device.time_now().unwrap_or(-1), format!("{other:?}"))),
        }
    }
    out
}

/// Reads until two timeouts in a row: the stream has stopped.
fn drain(device: &dyn Device) -> Read {
    read(device, Wall::from_secs(3), None, |r| r.other.len() >= 2 && r.other[r.other.len() - 2..].iter().all(|(_, o)| o == "Timeout"))
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_rx_timed_stop() {
    // UR-25's INFERRED point: does the X3x0 honour the time of a continuous stream's stop?
    // (UHD 4.10's radio_rx_core.v: "timed STOP commands are not supported".)
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    for lead_ms in [0, 1, 5, 20, 100, 500] {
        let start = (device.time_now().unwrap() / 200 + 20_000) * 200;
        device.rx_start(start).unwrap();
        let before = read(&*device, Wall::from_secs(2), None, |r| r.end(200).is_some_and(|e| e >= start + 200 * TICKS_PER_MS));
        let issued_before = device.time_now().unwrap();
        let at = issued_before + lead_ms * TICKS_PER_MS;
        device.rx_stop(if lead_ms == 0 { None } else { Some(at) }).unwrap();
        let issued = device.time_now().unwrap();
        let after = drain(&*device);
        let end = after.end(200).or(before.end(200)).unwrap();
        println!(
            "B8 rx stop lead {lead_ms} ms: first block {:?} (start {start}); stop issued at {issued_before}…{issued} for {}; last sample end {end}: {:+.3} ms from the issue, {:+.3} ms from the stop's time; after the stop: {} blocks, {:?}",
            before.blocks.first(),
            if lead_ms == 0 { "now".to_owned() } else { at.to_string() },
            (end - issued) as f64 / TICKS_PER_MS as f64,
            (end - at) as f64 / TICKS_PER_MS as f64,
            after.blocks.len(),
            after.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_device_lead() {
    // The device lead: the least lead from `time_now()` at which a timed burst is played
    // and not reported late (below UR-21's 2 ms, which the Module never probes).
    let device = raw();
    raw_tx(&*device, 1e6, 1e9);
    while device.tx_async(Wall::from_millis(10)).is_some() {}
    let wave = iq(&pn(100));
    for lead_us in [3_000i64, 2_000, 1_500, 1_000, 700, 500, 300, 200, 100, 50, 0] {
        let mut outcomes = Vec::new();
        let mut send_us = Vec::new();
        for _ in 0..8 {
            let now = device.time_now().unwrap();
            let at = now + lead_us * 200;
            let begun = Instant::now();
            device.tx_send(&[&wave], Some(at), true, true, Wall::from_millis(100)).unwrap();
            send_us.push(begun.elapsed().as_micros());
            let mut codes = Vec::new();
            let waited = Instant::now();
            while waited.elapsed() < Wall::from_millis(100) {
                match device.tx_async(Wall::from_millis(20)) {
                    Some(report) => {
                        codes.push(format!("{:?}", report.code));
                        if matches!(report.code, ezsdr_radio_uhd::TxCode::BurstAck) {
                            break;
                        }
                    }
                    None if !codes.is_empty() => break,
                    None => {}
                }
            }
            outcomes.push(codes.join("+"));
            std::thread::sleep(Wall::from_millis(20));
        }
        let late = outcomes.iter().filter(|o| o.contains("TimeError")).count();
        println!("B8 device lead {lead_us} µs: {late}/8 late; send took {send_us:?} µs; outcomes {outcomes:?}");
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_restart_lead() {
    // The restart lead (UR-25, 50 ms INFERRED): a stop, optionally a rate change (`cold`),
    // then a timed start `lead` ahead; is the start honoured at its tick, or late?
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    let start = (device.time_now().unwrap() / 200 + 50_000) * 200;
    device.rx_start(start).unwrap();
    let _ = read(&*device, Wall::from_secs(2), None, |r| r.end(200).is_some_and(|e| e >= start + 100 * TICKS_PER_MS));
    let mut rate = 1e6;
    // `reopen` also makes a new receive streamer, as uhd-rx's `cold` switch does (UR-25).
    for (cold, reopen) in [(false, false), (true, false), (true, true)] {
        for lead_ms in [100i64, 50, 25, 10, 5, 2, 1] {
            let stopped = device.time_now().unwrap();
            device.rx_stop(None).unwrap();
            let mut apply_ms = 0.0;
            if cold {
                rate = if rate == 1e6 { 2e6 } else { 1e6 };
                let begun = Instant::now();
                device.apply(Dir::Rx, 0, &Settings { rate: Some(rate), ..Settings::default() }, None).unwrap();
                apply_ms = begun.elapsed().as_secs_f64() * 1e3;
            }
            if reopen {
                let _ = drain(&*device);
                let begun = Instant::now();
                device.rx_open(1).unwrap();
                println!("B8 restart: rx_open took {:.3} ms", begun.elapsed().as_secs_f64() * 1e3);
            }
            let per = (200e6 / rate) as i64;
            let now = device.time_now().unwrap();
            let at = (now + lead_ms * TICKS_PER_MS) / per * per;
            device.rx_start(at).unwrap();
            // Blocks from before `now` are the old stream's tail (the stop takes ~0.4 ms).
            let got = read(&*device, Wall::from_secs(2), None, |r| r.blocks.iter().any(|(t, _)| *t >= now) || r.other.iter().any(|(_, o)| o.starts_with("LateCommand")));
            let first_new = got.blocks.iter().find(|(t, _)| *t >= now).map(|(t, _)| *t);
            println!(
                "B8 restart {} lead {lead_ms} ms: stop→start issued {:.3} ms (apply {apply_ms:.3} ms); first new block {first_new:?} for {at} ({}); {} old blocks drained; other {:?}",
                match (cold, reopen) { (false, _) => "warm", (true, false) => "cold (rate change)", (true, true) => "cold (rate change, new streamer)" },
                (now - stopped) as f64 / TICKS_PER_MS as f64,
                first_new.map_or("none".to_owned(), |t| format!("{:+} ticks", t - at)),
                got.blocks.iter().filter(|(t, _)| *t < now).count(),
                got.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
            );
            let _ = read(&*device, Wall::from_millis(200), None, |_| false);
        }
    }
    device.rx_stop(None).unwrap();
    let _ = drain(&*device);
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_queue_depth() {
    // UR-24's queue depth (16, INFERRED): timed OBX tunes queued ahead of their time; the
    // call that blocks marks the depth. Nothing is streamed, so nothing is emitted.
    let device = raw();
    for dir in [Dir::Rx, Dir::Tx] {
        let base = device.time_now().unwrap() + 3_000 * TICKS_PER_MS;
        let mut took = Vec::new();
        for i in 0..40i64 {
            let freq = if i % 2 == 0 { 999_600_000.0 } else { 1_000_400_000.0 };
            let begun = Instant::now();
            let result = device.apply(dir, 0, &Settings { freq: Some(freq), ..Settings::default() }, Some(base + i * TICKS_PER_MS));
            let ms = begun.elapsed().as_secs_f64() * 1e3;
            took.push(format!("{ms:.2}"));
            if result.is_err() || ms > 500.0 {
                println!("B8 queue {dir:?}: call {} took {ms:.1} ms: {result:?}", i + 1);
                break;
            }
        }
        println!("B8 queue {dir:?}: apply ms per call {took:?}");
        std::thread::sleep(Wall::from_millis(3_200));
        println!("B8 queue {dir:?}: after the queue, untimed {:?}", device.apply(dir, 0, &Settings { freq: Some(1e9), ..Settings::default() }, None));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_timed_tune_phase() {
    // `phase_behavior_on_retune: random_unless_timed_tune` (UR-9): after the LOs have been
    // elsewhere, is the loop's phase the same when both return by a timed tune, and random
    // when they return untimed? A 100 kHz tone, its phase a function of the device tick.
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    let f_bb = 100e3;
    let tone_at = |tick: i64| {
        let phase = 2.0 * std::f64::consts::PI * f_bb * (tick as f64 / 200e6);
        [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
    };
    // 1 GHz and a frequency no integer multiple of a reference lands on (fractional-N).
    for (f0, timed) in [(1e9, true), (1e9, false), (1_000_123_400.0, true), (1_000_123_400.0, false)] {
        assert!(SAFE_TX_HZ.contains(&(f0 + f_bb)));
        let mut phases = Vec::new();
        for cycle in 0..6 {
            // Scramble both LOs, then return to 1 GHz.
            for dir in [Dir::Tx, Dir::Rx] {
                device.apply(dir, 0, &Settings { freq: Some(999_700_000.0 + cycle as f64 * 50_000.0), ..Settings::default() }, None).unwrap();
            }
            let tune = (device.time_now().unwrap() / 200 + 40_000) * 200;
            for dir in [Dir::Tx, Dir::Rx] {
                device.apply(dir, 0, &Settings { freq: Some(f0), ..Settings::default() }, timed.then_some(tune)).unwrap();
            }
            let start = tune + 100 * TICKS_PER_MS;
            device.rx_start(start).unwrap();
            let burst: Vec<_> = (0..20_000).map(|i| tone_at(start + i * 200)).collect();
            let sent = device.tx_send(&[&burst], Some(start), true, true, Wall::from_millis(500)).unwrap();
            let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= start + 25 * TICKS_PER_MS));
            device.rx_stop(None).unwrap();
            let _ = drain(&*device);
            let mut reports = Vec::new();
            while let Some(report) = device.tx_async(Wall::from_millis(10)) {
                reports.push(format!("{:?}", report.code));
            }
            let rms = (got.samples.iter().map(|(_, s)| f64::from(s[0] * s[0] + s[1] * s[1])).sum::<f64>() / got.samples.len().max(1) as f64).sqrt();
            println!("B8 phase: sent {sent}; received {} samples from {:?}, rms {rms:.5}; tx reports {reports:?}; other {:?}", got.samples.len(), got.blocks.first(), got.other);
            // The middle 10 ms of the burst, against the tone at the same tick.
            let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0);
            for (tick, s) in got.samples.iter().filter(|(t, _)| *t >= start + 5 * TICKS_PER_MS && *t < start + 15 * TICKS_PER_MS) {
                let r = tone_at(*tick);
                re += f64::from(s[0] * r[0] + s[1] * r[1]);
                im += f64::from(s[1] * r[0] - s[0] * r[1]);
                n += 1;
            }
            let gain = re.hypot(im) / (n.max(1) as f64 * 0.16);
            let phase = im.atan2(re).to_degrees();
            phases.push(phase);
            println!("B8 phase {f0} Hz {} tune, cycle {cycle}: {n} samples, gain {gain:.4}, phase {phase:.1}°", if timed { "timed" } else { "untimed" });
        }
        let spread = phases.iter().map(|p| ((p - phases[0] + 540.0).rem_euclid(360.0) - 180.0).abs()).fold(0.0, f64::max);
        println!("B8 phase {f0} Hz {} tune: phases {phases:.1?}, largest difference from the first {spread:.1}°", if timed { "timed" } else { "untimed" });
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_in_flight_window() {
    // UR-23's in-flight window (10 ms, INFERRED): a continuous burst at 2 Msps paced to keep
    // at most `window` of samples ahead of the device's time; does the device underflow?
    let device = raw();
    raw_tx(&*device, 2e6, 1e9);
    while device.tx_async(Wall::from_millis(10)).is_some() {}
    let wave = iq(&pn(1_000));
    for window_us in [10_000u64, 5_000, 3_000, 2_000, 1_000, 500, 250] {
        let now = device.time_now().unwrap();
        let host_now = Instant::now();
        let lead = 50 * TICKS_PER_MS;
        let start = now + lead;
        let host_start = host_now + Wall::from_nanos(lead as u64 * 5);
        let buffers = 4_000u64; // 2 s at 2 Msps, 0.5 ms a buffer
        let mut reports = Vec::new();
        for k in 0..buffers {
            // Buffer k plays from host_start + k · 0.5 ms; send it `window` before that.
            let due = host_start + Wall::from_micros(k * 500);
            let send_at = due.checked_sub(Wall::from_micros(window_us)).unwrap_or(host_now);
            while Instant::now() < send_at {
                std::hint::spin_loop();
            }
            let at = (k == 0).then_some(start);
            device.tx_send(&[&wave], at, k == 0, k + 1 == buffers, Wall::from_millis(100)).unwrap();
            while let Some(report) = device.tx_async(Wall::ZERO) {
                reports.push(report);
            }
        }
        let waited = Instant::now();
        while waited.elapsed() < Wall::from_millis(200) {
            if let Some(report) = device.tx_async(Wall::from_millis(20)) {
                reports.push(report);
            }
        }
        let count = |code: ezsdr_radio_uhd::TxCode| reports.iter().filter(|r| r.code == code).count();
        println!(
            "B8 in-flight window {window_us} µs: underflow {}, underflow in packet {}, time error {}, seq error {}, burst ack {}; first reports {:?}",
            count(ezsdr_radio_uhd::TxCode::Underflow),
            count(ezsdr_radio_uhd::TxCode::UnderflowInPacket),
            count(ezsdr_radio_uhd::TxCode::TimeError),
            count(ezsdr_radio_uhd::TxCode::SeqError) + count(ezsdr_radio_uhd::TxCode::SeqErrorInBurst),
            count(ezsdr_radio_uhd::TxCode::BurstAck),
            reports.iter().take(5).collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_cold_change_capture() {
    // UR-25 on the bench: a capture spanning a `cold` receive rate change. Before the fix of
    // design-notes §11 F1 uhd-rx handed the device a timed stop for e1 a restart lead early,
    // and the X300 stopped at once (hw_b8_raw_rx_timed_stop): 50 ms lost before e1. Now the
    // old clock's samples should reach e1 and the new clock's start at e2, on time.
    cold_change_capture(1e6, 2e6, None);
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_cold_change_capture_low_rate() {
    // Review N, B1 and B2 on the bench: the lowest rate, 200 MHz / 512, with the default
    // block and with the largest (65 536 samples, 168 ms).
    cold_change_capture(390_625.0, 400_000.0, None);
    cold_change_capture(390_625.0, 2e6, Some(65_536));
}

/// A Session receiving at `from`; a capture across a `cold` change to `to`; prints the
/// Manifest's view and asserts the old clock's samples reach e₁, the new clock's begin at
/// e₂ and no `LATE_COMMAND`.
fn cold_change_capture(from: f64, to: f64, block_len: Option<u32>) {
    let dir = TempDir::new();
    let device = usrp();
    let selector = block_len.map_or(serde_json::json!({}), |n| serde_json::json!({ "block_len": n }));
    let mut run = session(&bench_profile(&*device, &dir, selector, serde_json::json!({}), true), device);
    past_t0(&mut run, ms(100));
    if from != 1e6 {
        assert!(admitted(&run.submit(set("radio.rx.sample_rate_hz", ezsdr_kernel::spec::Value::Num(from)), None).unwrap()));
        wait(&mut run, ms(300));
    }
    let at = after_ticks(&run, ms(20));
    let n = (from * 0.12) as i64 + (to * 0.1) as i64;
    assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::Int(n))]), None).unwrap()));
    wait(&mut run, ms(60));
    let booked = run.now();
    let entry = run.submit(set("radio.rx.sample_rate_hz", ezsdr_kernel::spec::Value::Num(to)), None).unwrap();
    println!("B8 cold {from} → {to} S/s, block_len {block_len:?}: capture asked at {at:?}; rate change submitted at {booked:?}: {:?}", entry.outcome);
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    wait(&mut run, ms(300));
    let manifest = run.finish();
    println!("B8 cold: termination {:?}", manifest.termination.reason);
    println!("B8 cold: sample clocks {:?}", manifest.clocks.sample_clocks);
    println!("B8 cold: timing {}", section(&manifest, "timing"));
    println!("B8 cold: applied {}", section(&manifest, "applied"));
    println!("B8 cold: stats {}", section(&manifest, "stats"));
    let artifact = capture_of(&manifest, "rec");
    println!("B8 cold: artifact {} continuity {:?}", artifact.id, artifact.continuity);
    let late = events_of(&manifest, "radio.LATE_COMMAND");
    for event in &manifest.events.delivered {
        if !event.kind.to_string().starts_with("ezsdr.") {
            println!("B8 cold: event {} from {:?} at {:?}: {}", event.kind, event.source, event.time, event.payload);
        }
    }
    let clocks: Vec<_> = manifest.clocks.sample_clocks.iter().filter(|r| r.stream.to_string().ends_with("usrp/rx")).collect();
    let (old, new) = (clocks[clocks.len() - 2], clocks[clocks.len() - 1]);
    let e1_k = (old.ended_at.unwrap().ticks - old.origin.ticks) / old.root_ticks_per_tick.num() as i64;
    assert_eq!(artifact.continuity.len(), 2, "{:?}", artifact.continuity);
    assert_eq!(artifact.continuity[0].end.ticks, e1_k, "the old clock's samples end at e₁");
    assert_eq!(artifact.continuity[1].domain, new.domain);
    assert_eq!(artifact.continuity[1].first.ticks, 0, "the new clock's samples begin at e₂");
    assert!(artifact.continuity.iter().all(|m| m.gaps.is_empty()));
    assert!(late.is_empty(), "{late:?}");
}

// ---------------------------------------------------------------- B8 through the Module

/// The transmit sample `lead` root ticks from now, on the running transmit clock.
fn tx_at(run: &ezsdr_kernel::coordinator::RunHandle, lead: i64) -> TimePoint {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx") && r.ended_at.is_none()).unwrap();
    let n = clock.root_ticks_per_tick.num() as i64;
    TimePoint::new(clock.domain, (run.now().ticks + lead - clock.origin.ticks + n - 1).div_euclid(n))
}

fn tx_session(device: Arc<dyn Device>, dir: &TempDir) -> ezsdr_kernel::coordinator::RunHandle {
    let envelope = bench_envelope(&*device);
    let mut run = session(&bench_profile(&*device, dir, serde_json::json!({}), envelope, true), device);
    past_t0(&mut run, ms(1));
    assert!(admitted(&run.submit(set("radio.tx.channels", ezsdr_kernel::spec::Value::Int(1)), None).unwrap()));
    run
}

/// The receive SampleClock's origin (root ticks) and root ticks a sample.
fn rx_clock(manifest: &ezsdr_kernel::manifest::Manifest) -> (i64, i64) {
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.stream.to_string().ends_with("usrp/rx")).unwrap();
    (clock.origin.ticks, clock.root_ticks_per_tick.num() as i64)
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_stop_end() {
    // bench.md B8: a repeat stopped by `Stop`; where its record ends and where the loop
    // stops hearing it, against the instant the Stop was submitted (UR-23: by s + 10 ms).
    for trial in 0..3 {
        let dir = TempDir::new();
        let mut run = tx_session(usrp(), &dir);
        let (bytes, _) = waveform_of(&pn(1_000));
        assert!(admitted(&run.submit(verb("start_repeat", "radio/tx", None, &[]), Some(&bytes)).unwrap()));
        let at = after_ticks(&run, ms(20));
        assert!(admitted(&run.submit(verb("capture", "sink/rec", Some(at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::Int(100_000))]), None).unwrap()));
        let _ = run.advance_to(TimePoint::new(at.domain, at.ticks + ms(50)));
        let s = run.now();
        let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(ezsdr_kernel::id::ResourceId::parse("radio/tx").unwrap()) }, None);
        let horizon = after_ticks(&run, ms(3_000));
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
        let manifest = run.finish();
        let artifact = capture_of(&manifest, "rec");
        let first = artifact.continuity[0].first.ticks;
        let samples = read_capture(&artifact, 1).remove(0);
        let power: Vec<f32> = samples.chunks(100).map(|c| c.iter().map(|(re, im)| re * re + im * im).sum::<f32>() / c.len() as f32).collect();
        let steady = power[..20].iter().sum::<f32>() / 20.0;
        let last = power.iter().rposition(|p| *p > steady / 4.0).unwrap();
        let (origin, per) = rx_clock(&manifest);
        // The received sample's instant less the loop's 44-sample delay (B6).
        let heard_until = origin + (first + (last as i64 + 1) * 100 - 44) * per;
        println!(
            "B8 stop end, trial {trial}: Stop submitted at root {}; the loop heard the repeat until root {heard_until} ({:+.3} ms from the Stop); steady power {steady:.2e}, after {:.2e}; bursts {}",
            s.ticks,
            (heard_until - s.ticks) as f64 / TICKS_PER_MS as f64,
            power[last + 2..].iter().sum::<f32>() / (power.len() - last - 2).max(1) as f32,
            section(&manifest, "bursts")
        );
        println!("B8 stop end, trial {trial}: tx clocks {:?}", manifest.clocks.sample_clocks.iter().filter(|r| r.stream.to_string().ends_with("usrp/tx")).map(|r| (r.origin.ticks, r.root_ticks_per_tick.num())).collect::<Vec<_>>());
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_preemption() {
    // bench.md B8: a burst preempting a running repeat at leads of 20, 10, 5, 3 ms (UR-21:
    // the in-flight window bounds it, 10 ms).
    for lead_ms in [20, 10, 5, 3] {
        let dir = TempDir::new();
        let mut run = tx_session(usrp(), &dir);
        let (repeat, _) = waveform_of(&pn(1_000));
        assert!(admitted(&run.submit(verb("start_repeat", "radio/tx", None, &[]), Some(&repeat)).unwrap()));
        wait(&mut run, ms(100));
        let (burst, _) = waveform_of(&pn(100));
        let at = tx_at(&run, ms(lead_ms));
        let entry = run.submit(verb("send", "radio/tx", Some(at), &[]), Some(&burst)).unwrap();
        wait(&mut run, ms(100));
        let manifest = run.finish();
        println!("B8 preempt lead {lead_ms} ms: {:?}; TIME_ERROR {:?}", entry.outcome, events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>());
        println!("B8 preempt lead {lead_ms} ms: asked {}; bursts {}", at.ticks, section(&manifest, "bursts"));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_timed_retune() {
    // bench.md B8's timed retunes: a receive retune at a lead from `time_now()` (0: untimed),
    // seen in a 100 kHz loopback tone moving to 50 kHz; when does it begin and settle?
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    let tone: Vec<_> = (0..60_000).map(|i| {
        let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
        [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
    }).collect();
    for lead_us in [10_000i64, 5_000, 2_000, 1_000, 500, 200, 0] {
        device.apply(Dir::Rx, 0, &Settings { freq: Some(1e9), ..Settings::default() }, None).unwrap();
        let start = (device.time_now().unwrap() / 200 + 100_000) * 200;
        device.rx_start(start).unwrap();
        device.tx_send(&[&tone], Some(start), true, true, Wall::from_millis(500)).unwrap();
        while device.time_now().unwrap() < start + 20 * TICKS_PER_MS {
            std::thread::sleep(Wall::from_micros(200));
        }
        let now = device.time_now().unwrap();
        let at = now + lead_us * 200;
        let begun = Instant::now();
        device.apply(Dir::Rx, 0, &Settings { freq: Some(1_000_050_000.0), ..Settings::default() }, (lead_us > 0).then_some(at)).unwrap();
        let apply_us = begun.elapsed().as_micros();
        let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= start + 55 * TICKS_PER_MS));
        device.rx_stop(None).unwrap();
        let _ = drain(&*device);
        while device.tx_async(Wall::from_millis(10)).is_some() {}
        // The tone's frequency over windows of 25 samples.
        let windows: Vec<(i64, f64)> = got.samples.chunks_exact(25).map(|w| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for pair in w.windows(2) {
                let (a, b) = (pair[0].1, pair[1].1);
                re += f64::from(b[0] * a[0] + b[1] * a[1]);
                im += f64::from(b[1] * a[0] - b[0] * a[1]);
            }
            (w[0].0, im.atan2(re) / (2.0 * std::f64::consts::PI) * 1e6)
        }).filter(|(t, _)| *t >= start + TICKS_PER_MS && *t < start + 55 * TICKS_PER_MS).collect();
        let begins = windows.iter().find(|(_, f)| (f - 100e3).abs() > 10e3).map(|(t, _)| *t);
        let settled = windows.iter().rposition(|(_, f)| (f - 50e3).abs() > 2e3).and_then(|i| windows.get(i + 1)).map(|(t, _)| *t);
        let us = |t: Option<i64>| t.map(|t| format!("{:+.1} µs", (t - at) as f64 / 200.0));
        println!(
            "B8 retune lead {lead_us} µs: asked at {at} (issued at {now}, apply {apply_us} µs); the tone leaves 100 kHz at {:?}, settles at 50 kHz at {:?} (from the asked instant); before {:.0} Hz, after {:.0} Hz",
            us(begins), us(settled),
            windows.first().map_or(0.0, |w| w.1), windows.last().map_or(0.0, |w| w.1)
        );
    }
}

// ---------------------------------------------------------------- beyond bench.md: rates, delay, endurance

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_perf_rx_rates() {
    // The receive rate the bench sustains through the Module (Phase 8's PerformanceEnvelope):
    // 3 s at each rate, no capture written.
    for rate in [10e6, 20e6, 25e6, 40e6, 50e6, 100e6, 200e6] {
        let dir = TempDir::new();
        let device = usrp();
        let doc = bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false);
        let mut run = spec_run(&receive_spec(1, rate, bench_hz(&*device), None), &doc, device, Default::default());
        let horizon = after_ticks(&run, ms(5_000));
        let result = run.advance_to(horizon);
        let manifest = run.finish();
        println!(
            "B8 rate {} Msps: {result:?}; termination {:?}; overflows {}; stats {}",
            rate / 1e6,
            manifest.termination.reason,
            events_of(&manifest, "radio.RX_OVERFLOW").len(),
            section(&manifest, "stats")
        );
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b6_delay_by_rate() {
    // The transmit-to-receive delay at each rate the bench may emit at (≤ 2 Msps): the
    // profile's `tx_path_delay_samples` is one number (45).
    let wave = pn(1_000);
    let (bytes, waveform) = waveform_of(&wave);
    for rate in [400e3, 500e3, 1e6, 2e6] {
        let dir = TempDir::new();
        let device = usrp();
        let mut spec = with_burst(with_tx(receive_spec(1, rate, bench_hz(&*device), None), rate), &waveform, false, "drop_and_flag", 10_000);
        spec["schedule"].as_array_mut().unwrap().push(serde_json::json!({
            "at": { "clock": "radio", "offset_ticks": 10_000 },
            "action": { "kind": "update_parameter", "target": serde_json::to_value(ezsdr_kernel::id::ResourceId::parse("sink/rec").unwrap()).unwrap(),
                        "key": "sink.capture_samples", "value": 5_000, "class": "block_boundary" }
        }));
        let inputs = std::collections::BTreeMap::from([(waveform.hash.clone(), bytes.clone())]);
        let doc = bench_profile(&*device, &dir, serde_json::json!({}), bench_envelope(&*device), false);
        let manifest = captured(spec_run(&spec, &doc, device, inputs));
        let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
        diagnose(&format!("B6 {} ksps", rate / 1e3), &samples, &wave);
        let at = correlate(&samples, &wave);
        println!("B6 {} ksps: delay {at:?} samples = {:?} µs; TIME_ERROR {}", rate / 1e3, at.map(|a| a as f64 / rate * 1e6), events_of(&manifest, "radio.TIME_ERROR").len());
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_long_receive() {
    // B9's other half without the unplug: a 120 s receive Run at 10 Msps raises no false
    // DEVICE_LOST (UR-29) and ends as the client ended it.
    let dir = TempDir::new();
    let device = usrp();
    let doc = bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false);
    let mut run = spec_run(&receive_spec(1, 10e6, bench_hz(&*device), None), &doc, device, Default::default());
    let horizon = after_ticks(&run, ms(122_000));
    let result = run.advance_to(horizon);
    let manifest = run.finish();
    let lost = events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST);
    println!("B9 long receive: {result:?}; termination {:?}; DEVICE_LOST {lost:?}; overflows {}; stats {}", manifest.termination.reason, events_of(&manifest, "radio.RX_OVERFLOW").len(), section(&manifest, "stats"));
    assert!(lost.is_empty());
    assert!(matches!(manifest.termination.reason, ezsdr_kernel::run::Termination::Stopped { cause: ezsdr_kernel::run::StopCause::Client {} }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_transmit_only_session() {
    // B9's transmit-only Session without the unplug: 60 s with the transmitter enabled and
    // nothing sent, the receiver off if the Session allows it; no false DEVICE_LOST.
    let dir = TempDir::new();
    let mut run = tx_session(usrp(), &dir);
    let off = run.submit(set("radio.rx.channels", ezsdr_kernel::spec::Value::Int(0)), None).unwrap();
    println!("B9 transmit only: radio.rx.channels 0: {:?}", off.outcome);
    wait(&mut run, ms(60_000));
    let manifest = run.finish();
    let lost = events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST);
    println!("B9 transmit only: termination {:?}; DEVICE_LOST {lost:?}; TX_UNDERFLOW {}; timing {}", manifest.termination.reason, events_of(&manifest, "radio.TX_UNDERFLOW").len(), section(&manifest, "timing"));
    assert!(lost.is_empty());
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b2_drift_60s() {
    // UR-8's INFERRED drift bound: the device against the host over 60 s, every 10 s.
    let device = usrp();
    let authority = DeviceAuthority::new(device.clone(), Arc::new(ClockRegistry::new()), "internal", "internal", &args()).unwrap();
    let (time, root) = (authority.time(), authority.root());
    let begun = Instant::now();
    let (ours, theirs) = (time.now(root).unwrap().ticks, device.time_now().unwrap());
    for step in 1..=6 {
        std::thread::sleep(Wall::from_secs(10));
        let (a, b) = (time.now(root).unwrap().ticks - ours, device.time_now().unwrap() - theirs);
        println!("B2 drift after {:?}: host-derived {a} ticks, device {b} ticks, difference {} ticks ({:.2} ppm)", begun.elapsed(), a - b, (a - b) as f64 / b as f64 * 1e6);
        let _ = step;
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_burst_gap() {
    // UR-23's back-to-back bursts: a burst ends with end-of-burst at tick T and the next
    // starts with a timed start-of-burst at T + gap samples. Which gap does the device take
    // without a late report (hw_b8_preemption saw late_at_device at gap 0)?
    let device = raw();
    raw_tx(&*device, 1e6, 1e9);
    while device.tx_async(Wall::from_millis(10)).is_some() {}
    let (a, b) = (iq(&pn(1_000)), iq(&pn(100)));
    for gap in [0i64, 1, 2, 3, 5, 10, 20, 50, 100, 1_000] {
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
            device.tx_send(&[&a], Some(t0), true, true, Wall::from_millis(100)).unwrap();
            device.tx_send(&[&b], Some(t0 + (1_000 + gap) * 200), true, true, Wall::from_millis(100)).unwrap();
            let mut codes = Vec::new();
            let waited = Instant::now();
            while waited.elapsed() < Wall::from_millis(150) {
                if let Some(report) = device.tx_async(Wall::from_millis(20)) {
                    codes.push(format!("{:?}", report.code));
                }
            }
            codes.dedup();
            outcomes.push(codes.join("+"));
        }
        println!("B8 burst gap {gap} samples: {outcomes:?}");
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_burst_at_a_sent_burst_s_end() {
    // Review N, B3 on the bench: a burst booked at the end of one whose last buffer already
    // went out (8 ms before its end) continues its device burst and is played.
    for trial in 0..3 {
        let dir = TempDir::new();
        let mut run = tx_session(usrp(), &dir);
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let (first, _) = waveform_of(&pn(30_000));
        assert!(admitted(&run.submit(verb("send", "radio/tx", Some(a), &[]), Some(&first)).unwrap()));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx") && r.ended_at.is_none()).unwrap();
        let a_end = clock.origin.ticks + (a.ticks + 30_000) * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain, a.ticks + 30_000);
        let (second, _) = waveform_of(&pn(1_000));
        let entry = run.submit(verb("send", "radio/tx", Some(b), &[]), Some(&second)).unwrap();
        wait(&mut run, ms(60));
        let manifest = run.finish();
        println!("B8 burst at a sent burst's end, trial {trial}: {:?}; {} ms before A's end; TIME_ERROR {:?}; async {}; bursts {}",
            entry.outcome, (a_end - entry.time.ticks) as f64 / TICKS_PER_MS as f64,
            events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
            section(&manifest, "async"), section(&manifest, "bursts"));
        assert!(events_of(&manifest, "radio.TIME_ERROR").is_empty());
        assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"));
    }
}
