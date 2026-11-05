//! The bench steps of `plan/phase7/bench.md` (UR-34), on a USRP X310 or X300 whose slot A
//! holds the bench board, cabled in loopback (the `x3xx` profile; the layout is slot A's
//! board and any of its kind after it, `layout_of`).
//! Compiled only with the feature `uhd`, every test ignored, the device from
//! `EZSDR_UHD_ARGS` (GZ-8). Run each as `bench.md` says;
//! each prints what `bench-results.md` records.
#![cfg(feature = "uhd")]

mod common;

use std::sync::Arc;
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::module_api::Authority;
use ezsdr_kernel::time::{ClockRegistry, TimePoint};
use ezsdr_radio_uhd::{Device, DeviceAuthority, Dir, Settings, UhdRadio};
use serde_json::Value as Json;

use common::probe_device::wait_for_rx_start_and_sample;
use common::tone::{Verdict, classify_tone, continuation_in_loopback, median_freq, tone_verdict, window_power, window_power_at};
use common::*;

fn args() -> String {
    std::env::var("EZSDR_UHD_ARGS").expect("EZSDR_UHD_ARGS names the device, e.g. addr=192.168.40.2")
}

fn usrp() -> Arc<dyn Device> {
    // Inside a record run the handle is guarded: after the record's latch holds, no device is opened and no call reaches one.
    measurement::guard_open(|| ezsdr_radio_uhd::open(&args()).expect("the USRP opens"))
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b1_probe() {
    let device = usrp();
    println!("B1 describe: {}", serde_json::to_string_pretty(&device.describe()).unwrap());
    assert_eq!(device.master_clock_rate(), 200_000_000, "B0.3: add master_clock_rate=200e6");
    // The layout's channels and front ends, as UR-5 checks them.
    let board = board_of(&*device);
    println!("B1 board: {}", board.name());
    let (rx, tx) = layout_of(&*device);
    for (dir, elements) in [(Dir::Rx, &rx), (Dir::Tx, &tx)] {
        let have = device.channels(dir);
        let need = elements.as_array().map_or(0, Vec::len);
        let names: Vec<_> = (0..have).map(|chan| device.front_end(dir, chan)).collect();
        println!("B1 {dir:?}: {have} channels, front ends {names:?}");
        assert!(have >= need, "the layout needs {need} {dir:?} channels, the device has {have}");
        // UR-5's comparison on real hardware: the channels the layout names must be the
        // board the layout is of, not merely whatever `layout_of` read off the device.
        for name in names.iter().take(need) {
            assert!(name.as_ref().unwrap().starts_with(board.uhd_name()), "{name:?} is not a {} front end", board.uhd_name());
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
        let at = TimePoint::new(root, time.now(root).unwrap().ticks_in(root).unwrap() + 2_000_000);
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
    let (ours, theirs) = (time.now(root).unwrap().ticks_in(root).unwrap(), device.time_now().unwrap());
    std::thread::sleep(Wall::from_secs(10));
    let drift = (time.now(root).unwrap().ticks_in(root).unwrap() - ours) - (device.time_now().unwrap() - theirs);
    println!("B2 anchor drift over {:?}: {drift} ticks", begun.elapsed());
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b3_receive_at_t0() {
    let manifest = rehearse_receive_at_t0(usrp());
    println!("B3 applied: {}", section(&manifest, "applied"));
    println!("B3 timing: {}", section(&manifest, "timing"));
    // UR-31: a Run on the real device records the device's fidelity, real on every aspect
    // (the exit review's UR-31 row, 2026-10-01).
    use ezsdr_kernel::module_api::{CoercionFidelity, EnvelopeFidelity, RfFidelity, TransportFidelity};
    let fidelity = &manifest.run.fidelity;
    println!("B3 fidelity: {fidelity:?}");
    assert_eq!(fidelity.timing, EnvelopeFidelity::Real);
    assert_eq!(fidelity.continuity, EnvelopeFidelity::Real);
    assert_eq!(fidelity.coercion, CoercionFidelity::Real);
    assert_eq!(fidelity.rf, RfFidelity::Real);
    assert_eq!(fidelity.transport, TransportFidelity::Real);
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
                    drops.first().map(|g| g.start.ticks_in(map.domain).unwrap()),
                    drops.last().map(|g| g.start.ticks_in(map.domain).unwrap() + g.len as i64)
                );
                for gap in map.gaps.iter().filter(|g| g.cause != ezsdr_kernel::stream::GapCause::LinkDrop {}) {
                    println!(
                        "B5 gap {:?} at receive sample {} for {} samples ({} ms at 10 Msps), lost {:?}",
                        gap.cause,
                        gap.start.ticks_in(map.domain).unwrap(),
                        gap.len,
                        gap.len as f64 / 1e4,
                        gap.lost
                    );
                }
                println!("B5 capture {} … {}, {} valid segment(s)", map.first.ticks_in(map.domain).unwrap(), map.end.ticks_in(map.domain).unwrap(), map.valid[0].len());
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
        let profile = bench_profile(&*device, &dir, serde_json::json!({}), envelope, true);
        let mut run = session_with(&profile, device, &rx0_tx0(&profile));
        past_t0(&mut run, ms(1));
        past_tx_origin(&mut run);
        let (bytes, _) = waveform_of(&pn(100));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
        let at = TimePoint::new(clock.domain, (run.now().ticks_in(clock.root).unwrap() + lead_us * 200 - clock.origin.ticks_in(clock.root).unwrap()).div_euclid(200) + 1);
        let _ = run.submit(verb("send", resource("radio/tx/s0"), Some(at), &[]), Some(&bytes));
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
    // Manual: unplug the 10 GbE cable within the next 300 s, during a receive Run. The Run
    // is advanced a second at a time until it is no longer running; the host's UTC at that
    // point is printed, to set against the instant the host's NIC lost its carrier.
    let dir = TempDir::new();
    let device = usrp();
    let mut run = spec_run(&receive_spec(1, 1e6, bench_hz(&*device), None), &bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false), device, Default::default());
    println!("B9: unplug the cable now (receive Run; up to 300 s); started at UTC {:.3}", utc_now());
    let ended = until_not_running(&mut run, 300);
    let manifest = run.finish();
    println!("B9 receive: {ended}; termination {:?}", manifest.termination.reason);
    println!("B9 receive DEVICE_LOST: {:?}", events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST));
    println!("B9 receive DEVICE_LOST at UTC {}", lost_utc(&manifest));
    println!("B9 receive rejected: {}", section(&manifest, "rejected"));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_unplug_transmit_only() {
    // Manual: unplug the 10 GbE cable within the next 300 s, during a transmit-only Session
    // with nothing sent (bench.md B9's second case): only uhd-clock's and the idle time
    // reads touch the device (UR-29's 500 ms read).
    let dir = TempDir::new();
    let mut run = tx_only_session(usrp(), &dir);
    println!("B9: unplug the cable now (transmit-only Session; up to 300 s); started at UTC {:.3}", utc_now());
    let ended = until_not_running(&mut run, 300);
    let manifest = run.finish();
    println!("B9 transmit only: {ended}; termination {:?}", manifest.termination.reason);
    println!("B9 transmit only DEVICE_LOST: {:?}", events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST));
    println!("B9 transmit only DEVICE_LOST at UTC {}", lost_utc(&manifest));
    println!("B9 transmit only rejected: {}", section(&manifest, "rejected"));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_unplug_and_reopen() {
    // Manual (Review S, NB-S2, TG-S2): unplug the cable during a receive Run; after its
    // DEVICE_LOST plug it back in. The test opens the device again in the same process —
    // `UhdDevice::open` first frees the kept device once it answers — runs B3's receive on
    // it, and then the process exits, which should not abort.
    let dir = TempDir::new();
    let device = usrp();
    let mut run = spec_run(&receive_spec(1, 1e6, bench_hz(&*device), None), &bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false), device, Default::default());
    println!("B9: unplug the cable now (receive Run; up to 300 s); started at UTC {:.3}", utc_now());
    let ended = until_not_running(&mut run, 300);
    let manifest = run.finish();
    println!("B9 reopen: {ended}; termination {:?}; DEVICE_LOST at UTC {}", manifest.termination.reason, lost_utc(&manifest));
    println!("B9 reopen: devices kept after the loss: {}", ezsdr_radio_uhd::uhd_kept_count());
    println!("B9: plug the cable back in now (waiting up to 300 s for the device); at UTC {:.3}", utc_now());
    let deadline = Instant::now() + Wall::from_secs(300);
    let device = loop {
        match ezsdr_radio_uhd::open(&args()) {
            Ok(device) if device.time_now().is_ok() => break device,
            _ if Instant::now() < deadline => std::thread::sleep(Wall::from_secs(2)),
            Ok(_) | Err(_) => panic!("the device did not come back within 300 s"),
        }
    };
    println!("B9 reopen: opened again at UTC {:.3}; devices kept now: {} (Review T, TG-T2: 1 before, 0 once the open's reclaim freed it)", utc_now(), ezsdr_radio_uhd::uhd_kept_count());
    let manifest = rehearse_receive_at_t0(device);
    println!("B9 reopen: B3 on the reopened device: termination {:?}", manifest.termination.reason);
    println!("B9 reopen: the process exits next, at UTC {:.3}", utc_now());
}

/// The UTC of the Manifest's first `DEVICE_LOST`, from the Run's root-to-UTC relation
/// (TM-18; the root's 200 MHz ticks, 5 ns each).
fn lost_utc(manifest: &ezsdr_kernel::manifest::Manifest) -> String {
    let Some(event) = events_of(manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST).into_iter().next() else { return "-".to_owned() };
    let utc = ezsdr_kernel::id::ClockDomainId::UTC;
    match manifest.clocks.relations.iter().find(|r| r.source() == event.time.domain() && r.target() == utc) {
        Some(r) => format!(
            "{:.3}",
            (r.offset().ticks_in(r.offset().domain()).unwrap() + (event.time.ticks_in(event.time.domain()).unwrap() - r.measured_at().ticks_in(r.measured_at().domain()).unwrap()) * 5) as f64 / 1e9
        ),
        None => "- (no relation to UTC)".to_owned(),
    }
}

/// The host's UTC in seconds.
fn utc_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// Advances `run` a second at a time until it is no longer running or `limit_s` pass;
/// says which, with the host's UTC then.
fn until_not_running(run: &mut ezsdr_kernel::coordinator::RunHandle, limit_s: u64) -> String {
    let deadline = Instant::now() + Wall::from_secs(limit_s);
    while Instant::now() < deadline {
        let step = run.advance_to(after(run, ms(1_000)));
        if step.is_err() || !matches!(run.state(), ezsdr_kernel::run::RunState::Running {}) {
            return format!("not running at UTC {:.3} ({step:?}, {:?})", utc_now(), run.state());
        }
    }
    format!("still running after {limit_s} s, at UTC {:.3}", utc_now())
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_usrp2_probe() {
    let device = usrp();
    println!("B9 USRP2 describe: {}", serde_json::to_string_pretty(&device.describe()).unwrap());
    // A USRP2 is not an X3xx at all: its front ends name no board of this stage, so the
    // binding carries a UBX layout and UR-5 must refuse it for the master clock.
    let binding = serde_json::from_value(serde_json::json!({
        "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
        "profile": x3xx(),
        "selector": { "args": args(), "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }
    }))
    .unwrap();
    let name = ezsdr_radio::x3xx::PROFILE;
    let refused = UhdRadio::from_binding(&binding, device).err().expect("UR-5 refuses a USRP2");
    println!("B9 UR-5: {}", refused.message);
    assert!(refused.message.starts_with(&format!("UR-5: profile {name} needs a 200 MHz master clock")));
}

// ---------------------------------------------------------------- B8 below the Module
//
// bench.md B8's rows that the Module's own rules keep a Session from probing (UR-21
// refuses a burst closer than its 2 ms device lead; the restart lead and the in-flight
// window are constants): measured on the `Device` itself, as the
// Module would drive it. RF as bench.md says: the transmitter only inside
// `SAFE_TX_HZ`, at most 2 Msps (so the emission stays inside the bench envelope's
// 999–1001 MHz), 0 dB, amplitude ≤ 0.4, into the 30 dB loopback.

const TICKS_PER_MS: i64 = 200_000;

/// The device channel the raw probes open: 0 (slot A) unless `EZSDR_D9_CHANNEL` names another
/// (1 = slot B on an X300 with two boards), so the D9 probes can run on either.
fn d9_channel() -> usize {
    static CHANNEL: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CHANNEL.get_or_init(|| std::env::var("EZSDR_D9_CHANNEL").ok().map_or(0, |text| text.parse().expect("EZSDR_D9_CHANNEL names a device channel")))
}

/// The device with its time set to zero on the internal reference.
fn raw() -> Arc<dyn Device> {
    let device = usrp();
    set_sources_with_pll_retries(&*device, "internal", "internal").unwrap();
    device.set_time_zero(false).unwrap();
    std::thread::sleep(Wall::from_millis(100));
    device
}

fn raw_rx(device: &dyn Device, rate: f64, freq: f64) {
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("RX2".to_owned()) };
    device.apply(Dir::Rx, d9_channel(), &settings, None).unwrap();
    device.rx_open(0, &[d9_channel()]).unwrap();
}

/// The transmitter inside bench.md's RF conditions, or a panic before anything is sent.
fn raw_tx(device: &dyn Device, rate: f64, freq: f64) {
    assert!(SAFE_TX_HZ.contains(&freq) && rate <= 2e6, "bench.md RF safety: {freq} Hz at {rate} S/s");
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("TX/RX".to_owned()) };
    let applied = device.apply(Dir::Tx, d9_channel(), &settings, None).unwrap();
    assert!(applied.gain == 0.0 && SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
    device.tx_open(0, &[d9_channel()]).unwrap();
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
    /// Every outcome in order, samples as `"samples"`: the quiet interval is counted over this, not over `other`.
    events: Vec<String>,
}

impl Read {
    fn end(&self, per_sample: i64) -> Option<i64> {
        self.blocks.last().map(|(t, n)| t + *n as i64 * per_sample)
    }
}

/// Reads until `until(read)` holds or `limit` passes; `keep`, the ticks a sample, keeps
/// the samples with their ticks.
fn read(device: &dyn Device, limit: Wall, keep: Option<i64>, until: impl Fn(&Read) -> bool) -> Read {
    read_on(device, 0, limit, keep, until)
}

/// `read` on stream `k`'s streamer.
fn read_on(device: &dyn Device, k: usize, limit: Wall, keep: Option<i64>, until: impl Fn(&Read) -> bool) -> Read {
    let mut out = Read::default();
    let begun = Instant::now();
    while begun.elapsed() < limit && !until(&out) {
        match device.rx_recv(k, 2_000, Wall::from_millis(100), false) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                out.blocks.push((first_tick, samples[0].len()));
                out.events.push("samples".to_owned());
                if let Some(per) = keep {
                    out.samples.extend(samples[0].iter().enumerate().map(|(i, s)| (first_tick + i as i64 * per, *s)));
                }
            }
            other => {
                out.events.push(format!("{other:?}"));
                out.other.push((device.time_now().unwrap_or(-1), format!("{other:?}")));
            }
        }
    }
    out
}

/// A drain's result: what was read, and whether the stream reached a quiet interval (two timeouts in a row with no
/// packet between them) before the limit. A drain that ran out of time is not quiet.
#[derive(Default, Debug)]
struct Drain {
    read: Read,
    quiet: bool,
}

/// Reads until two consecutive timeouts: the stream has stopped. The quiet count resets on every packet.
fn drain(device: &dyn Device) -> Drain {
    let read = read(device, Wall::from_secs(3), None, |r| n47::drain_quiet(&r.events));
    let quiet = n47::drain_quiet(&read.events);
    Drain { read, quiet }
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
        device.rx_start(0, start).unwrap();
        let before = read(&*device, Wall::from_secs(2), None, |r| r.end(200).is_some_and(|e| e >= start + 200 * TICKS_PER_MS));
        let issued_before = device.time_now().unwrap();
        let at = issued_before + lead_ms * TICKS_PER_MS;
        device.rx_stop(0, if lead_ms == 0 { None } else { Some(at) }).unwrap();
        let issued = device.time_now().unwrap();
        let after = drain(&*device);
        let end = after.read.end(200).or(before.end(200)).unwrap();
        println!(
            "B8 rx stop lead {lead_ms} ms: first block {:?} (start {start}); stop issued at {issued_before}…{issued} for {}; last sample end {end}: {:+.3} ms from the issue, {:+.3} ms from the stop's time; after the stop: {} blocks, {:?}",
            before.blocks.first(),
            if lead_ms == 0 { "now".to_owned() } else { at.to_string() },
            (end - issued) as f64 / TICKS_PER_MS as f64,
            (end - at) as f64 / TICKS_PER_MS as f64,
            after.read.blocks.len(),
            after.read.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
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
    while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
    let wave = iq(&pn(100));
    let mut observations = Vec::new();
    for lead_us in [3_000i64, 2_000, 1_500, 1_000, 700, 500, 300, 200, 100, 50, 0] {
        let mut outcomes = Vec::new();
        let mut send_us = Vec::new();
        for _ in 0..8 {
            let now = device.time_now().unwrap();
            let at = now + lead_us * 200;
            let begun = Instant::now();
            device.tx_send(0, &[&wave], Some(at), true, true, Wall::from_millis(100)).unwrap();
            send_us.push(begun.elapsed().as_micros());
            let mut codes = Vec::new();
            let waited = Instant::now();
            while waited.elapsed() < Wall::from_millis(100) {
                match device.tx_async(0, Wall::from_millis(20)).unwrap() {
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
        observations.push(serde_json::json!({ "lead_us": lead_us, "late": late, "send_us": send_us, "outcomes": outcomes }));
    }
    measurement::push_raw_observation("hw_b8_raw_device_lead", 88, serde_json::json!({ "leads": observations }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_restart_lead() {
    // The restart lead (UR-25, 50 ms INFERRED): a stop, optionally a rate change (`cold`),
    // then a timed start `lead` ahead; is the start honoured at its tick, or late?
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    let start = (device.time_now().unwrap() / 200 + 50_000) * 200;
    device.rx_start(0, start).unwrap();
    let _ = read(&*device, Wall::from_secs(2), None, |r| r.end(200).is_some_and(|e| e >= start + 100 * TICKS_PER_MS));
    let mut rate = 1e6;
    let mut observations = Vec::new();
    // `reopen` also makes a new receive streamer, as uhd-rx's `cold` switch does (UR-25).
    for (cold, reopen) in [(false, false), (true, false), (true, true)] {
        for lead_ms in [100i64, 50, 25, 10, 5, 2, 1] {
            let stopped = device.time_now().unwrap();
            device.rx_stop(0, None).unwrap();
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
                device.rx_open(0, &[0]).unwrap();
                println!("B8 restart: rx_open took {:.3} ms", begun.elapsed().as_secs_f64() * 1e3);
            }
            let per = (200e6 / rate) as i64;
            let now = device.time_now().unwrap();
            let at = (now + lead_ms * TICKS_PER_MS) / per * per;
            device.rx_start(0, at).unwrap();
            // Blocks from before `now` are the old stream's tail (the stop takes ~0.4 ms).
            let got = read(&*device, Wall::from_secs(2), None, |r| r.blocks.iter().any(|(t, _)| *t >= now) || r.other.iter().any(|(_, o)| o.starts_with("LateCommand")));
            let first_new = got.blocks.iter().find(|(t, _)| *t >= now).map(|(t, _)| *t);
            println!(
                "B8 restart {} lead {lead_ms} ms: stop→start issued {:.3} ms (apply {apply_ms:.3} ms); first new block {first_new:?} for {at} ({}); {} old blocks drained; other {:?}",
                match (cold, reopen) {
                    (false, _) => "warm",
                    (true, false) => "cold (rate change)",
                    (true, true) => "cold (rate change, new streamer)",
                },
                (now - stopped) as f64 / TICKS_PER_MS as f64,
                first_new.map_or("none".to_owned(), |t| format!("{:+} ticks", t - at)),
                got.blocks.iter().filter(|(t, _)| *t < now).count(),
                got.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
            );
            observations.push(serde_json::json!({
                "cold": cold, "reopen": reopen, "lead_ms": lead_ms, "stopped": stopped,
                "now": now, "start_at": at, "first_new": first_new, "apply_ms": apply_ms,
                "blocks": got.blocks, "other": got.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>(),
            }));
            let _ = read(&*device, Wall::from_millis(200), None, |_| false);
        }
    }
    device.rx_stop(0, None).unwrap();
    let _ = drain(&*device);
    measurement::push_raw_observation("hw_b8_raw_restart_lead", 21, serde_json::json!({ "restarts": observations }));
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
    let mut observations = Vec::new();
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
            device.rx_start(0, start).unwrap();
            let burst: Vec<_> = (0..20_000).map(|i| tone_at(start + i * 200)).collect();
            let sent = device.tx_send(0, &[&burst], Some(start), true, true, Wall::from_millis(500)).unwrap();
            let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= start + 25 * TICKS_PER_MS));
            device.rx_stop(0, None).unwrap();
            let _ = drain(&*device);
            let mut reports = Vec::new();
            while let Some(report) = device.tx_async(0, Wall::from_millis(10)).unwrap() {
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
        observations.push(serde_json::json!({ "frequency_hz": f0, "timed": timed, "phases_deg": phases, "spread_deg": spread }));
    }
    measurement::push_raw_observation("hw_b8_raw_timed_tune_phase", 24, serde_json::json!({ "cases": observations }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_in_flight_window() {
    // UR-23's in-flight window (10 ms, INFERRED): a continuous burst at 2 Msps paced to keep
    // at most `window` of samples ahead of the device's time; does the device underflow?
    let device = raw();
    raw_tx(&*device, 2e6, 1e9);
    while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
    let wave = iq(&pn(1_000));
    let mut observations = Vec::new();
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
            device.tx_send(0, &[&wave], at, k == 0, k + 1 == buffers, Wall::from_millis(100)).unwrap();
            while let Some(report) = device.tx_async(0, Wall::ZERO).unwrap() {
                reports.push(report);
            }
        }
        let waited = Instant::now();
        while waited.elapsed() < Wall::from_millis(200) {
            if let Some(report) = device.tx_async(0, Wall::from_millis(20)).unwrap() {
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
        observations.push(serde_json::json!({
            "window_us": window_us,
            "underflow": count(ezsdr_radio_uhd::TxCode::Underflow),
            "underflow_in_packet": count(ezsdr_radio_uhd::TxCode::UnderflowInPacket),
            "time_error": count(ezsdr_radio_uhd::TxCode::TimeError),
            "sequence_error": count(ezsdr_radio_uhd::TxCode::SeqError) + count(ezsdr_radio_uhd::TxCode::SeqErrorInBurst),
            "burst_ack": count(ezsdr_radio_uhd::TxCode::BurstAck),
            "first_reports": reports.iter().take(5).map(|r| format!("{r:?}")).collect::<Vec<_>>(),
        }));
    }
    measurement::push_raw_observation("hw_b8_raw_in_flight_window", 7, serde_json::json!({ "windows": observations }));
}

// Parked (note 47 stage 2): `hw_b8_cold_change_capture`, `hw_b8_cold_change_capture_low_rate` and
// `hw_b8_cold_change_timing_at_the_extremes` measured a receive rate change during a Run, which no longer exists.
// The restored receive start-lead group takes their place: `measurement::measure_rx_start_lead` and its
// `_with_tx` companion, run by `hw_n47_rx_start_lead_group` and by `hw_measurements_record`.

// ---------------------------------------------------------------- B8 through the Module

/// A Session that asked for transmit c0 only at connect: no receive stream is ever in the Run.
fn tx_only_session(device: Arc<dyn Device>, dir: &TempDir) -> ezsdr_kernel::coordinator::RunHandle {
    let envelope = bench_envelope(&*device);
    let profile = bench_profile(&*device, dir, serde_json::json!({}), envelope, true);
    let mut run = session_with(&profile, device, &tx_only(&profile));
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    run
}

fn tx_session(device: Arc<dyn Device>, dir: &TempDir) -> ezsdr_kernel::coordinator::RunHandle {
    let envelope = bench_envelope(&*device);
    let profile = bench_profile(&*device, dir, serde_json::json!({}), envelope, true);
    let mut run = session_with(&profile, device, &rx0_tx0(&profile));
    past_t0(&mut run, ms(1));
    past_tx_origin(&mut run);
    run
}

/// The receive SampleClock's origin (root ticks) and root ticks a sample.
fn rx_clock(manifest: &ezsdr_kernel::manifest::Manifest) -> (i64, i64) {
    let clock = manifest.clocks.sample_clocks.iter().find(|r| r.stream.to_string().ends_with("usrp/rx/s0")).unwrap();
    (clock.origin.ticks_in(clock.root).unwrap(), clock.root_ticks_per_tick.num() as i64)
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_stop_end() {
    // bench.md B8: a repeat stopped by `Stop`; where its record ends and where the loop
    // stops hearing it, against the instant the Stop was submitted (UR-23: by s + 10 ms).
    let mut observations = Vec::new();
    for trial in 0..3 {
        let dir = TempDir::new();
        let mut run = tx_session(usrp(), &dir);
        let (bytes, _) = waveform_of(&pn(1_000));
        assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&bytes)).unwrap()));
        let at = after_ticks(&run, ms(20));
        assert!(admitted(&run.submit(verb("capture", output("rec"), Some(at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(100_000))]), None).unwrap()));
        let _ = run.advance_to(TimePoint::new(at.domain(), at.ticks_in(at.domain()).unwrap() + ms(50)));
        let s = run.now();
        let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
        let horizon = after_ticks(&run, ms(3_000));
        let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
        let manifest = run.finish();
        let artifact = capture_of(&manifest, "rec");
        let first = artifact.continuity[0].first.ticks_in(artifact.continuity[0].domain).unwrap();
        let samples = read_capture(&artifact, 1).remove(0);
        let power: Vec<f32> = samples.chunks(100).map(|c| c.iter().map(|(re, im)| re * re + im * im).sum::<f32>() / c.len() as f32).collect();
        let steady = power[..20].iter().sum::<f32>() / 20.0;
        let last = power.iter().rposition(|p| *p > steady / 4.0).unwrap();
        let (origin, per) = rx_clock(&manifest);
        // The received sample's instant less the loop's 44-sample delay (B6).
        let heard_until = origin + (first + (last as i64 + 1) * 100 - 44) * per;
        println!(
            "B8 stop end, trial {trial}: Stop submitted at root {}; the loop heard the repeat until root {heard_until} ({:+.3} ms from the Stop); steady power {steady:.2e}, after {:.2e}; bursts {}",
            s.ticks_in(s.domain()).unwrap(),
            (heard_until - s.ticks_in(s.domain()).unwrap()) as f64 / TICKS_PER_MS as f64,
            power[last + 2..].iter().sum::<f32>() / (power.len() - last - 2).max(1) as f32,
            section(&manifest, "bursts")
        );
        let tx_clocks: Vec<_> =
            manifest.clocks.sample_clocks.iter().filter(|r| r.stream.to_string().ends_with("usrp/tx/s0")).map(|r| (r.origin.ticks_in(r.root).unwrap(), r.root_ticks_per_tick.num())).collect();
        println!("B8 stop end, trial {trial}: tx clocks {tx_clocks:?}");
        observations.push(serde_json::json!({
            "trial": trial,
            "stop_tick": s.ticks_in(s.domain()).unwrap(),
            "heard_until_tick": heard_until,
            "offset_ms": (heard_until - s.ticks_in(s.domain()).unwrap()) as f64 / TICKS_PER_MS as f64,
            "bursts": section(&manifest, "bursts"),
            "tx_clocks": tx_clocks,
        }));
    }
    measurement::push_raw_observation("hw_b8_stop_end", 3, serde_json::json!({ "trials": observations }));
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
        assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&repeat)).unwrap()));
        wait(&mut run, ms(100));
        let (burst, _) = waveform_of(&pn(100));
        let at = tx_at(&run, ms(lead_ms));
        let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(at), &[]), Some(&burst)).unwrap();
        wait(&mut run, ms(100));
        let manifest = run.finish();
        println!("B8 preempt lead {lead_ms} ms: {:?}; TIME_ERROR {:?}", entry.outcome, events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>());
        println!("B8 preempt lead {lead_ms} ms: asked {}; bursts {}", at.ticks_in(at.domain()).unwrap(), section(&manifest, "bursts"));
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
    let tone: Vec<_> = (0..60_000)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
            [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
        })
        .collect();
    for lead_us in [10_000i64, 5_000, 2_000, 1_000, 500, 200, 0] {
        device.apply(Dir::Rx, 0, &Settings { freq: Some(1e9), ..Settings::default() }, None).unwrap();
        let start = (device.time_now().unwrap() / 200 + 100_000) * 200;
        device.rx_start(0, start).unwrap();
        device.tx_send(0, &[&tone], Some(start), true, true, Wall::from_millis(500)).unwrap();
        while device.time_now().unwrap() < start + 20 * TICKS_PER_MS {
            std::thread::sleep(Wall::from_micros(200));
        }
        let now = device.time_now().unwrap();
        let at = now + lead_us * 200;
        let begun = Instant::now();
        device.apply(Dir::Rx, 0, &Settings { freq: Some(1_000_050_000.0), ..Settings::default() }, (lead_us > 0).then_some(at)).unwrap();
        let apply_us = begun.elapsed().as_micros();
        let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= start + 55 * TICKS_PER_MS));
        device.rx_stop(0, None).unwrap();
        let _ = drain(&*device);
        while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
        // The tone's frequency over windows of 25 samples.
        let windows: Vec<(i64, f64)> = got
            .samples
            .chunks_exact(25)
            .map(|w| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for pair in w.windows(2) {
                    let (a, b) = (pair[0].1, pair[1].1);
                    re += f64::from(b[0] * a[0] + b[1] * a[1]);
                    im += f64::from(b[1] * a[0] - b[0] * a[1]);
                }
                (w[0].0, im.atan2(re) / (2.0 * std::f64::consts::PI) * 1e6)
            })
            .filter(|(t, _)| *t >= start + TICKS_PER_MS && *t < start + 55 * TICKS_PER_MS)
            .collect();
        let begins = windows.iter().find(|(_, f)| (f - 100e3).abs() > 10e3).map(|(t, _)| *t);
        let settled = windows.iter().rposition(|(_, f)| (f - 50e3).abs() > 2e3).and_then(|i| windows.get(i + 1)).map(|(t, _)| *t);
        let us = |t: Option<i64>| t.map(|t| format!("{:+.1} µs", (t - at) as f64 / 200.0));
        println!(
            "B8 retune lead {lead_us} µs: asked at {at} (issued at {now}, apply {apply_us} µs); the tone leaves 100 kHz at {:?}, settles at 50 kHz at {:?} (from the asked instant); before {:.0} Hz, after {:.0} Hz",
            us(begins),
            us(settled),
            windows.first().map_or(0.0, |w| w.1),
            windows.last().map_or(0.0, |w| w.1)
        );
    }
}

// ---------------------------------------------------------------- MA-1: the LO phase after an untimed receive tune

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_ma1_phase_after_a_pre_start_tune() {
    // MA-1 (UR-24's Ceiling, KL-32): the receive frequency a Session asks for at connect is applied untimed, so the loopback's phase
    // has no timed tune's guarantee. `EZSDR_MA1_SESSIONS` Sessions (5 by default), each with its own device open; the spread of their
    // phases is the observation. The old probe's second way, a timed tune while the stream runs, has no Run-time form any more
    // (note 47 stage 2). A loopback measures the two LOs' relative phase only, so this says nothing of which one varies, and N
    // Sessions are an observation, not a proof of a guarantee. No value is made. RF as bench.md says: slot A, 0 dB (the envelope),
    // amplitude at most 0.5 (`transmit_samples`), the frequency inside SAFE_TX_HZ, into the 30 dB loopback (the attenuator is
    // confirmed by run-record.sh, which names slot A for a preflight test).
    let sessions: usize = std::env::var("EZSDR_MA1_SESSIONS").ok().map_or(5, |text| text.parse().expect("EZSDR_MA1_SESSIONS is a count"));
    assert!((2..=50).contains(&sessions), "2 to 50 Sessions");
    let wave = transmit_samples(1_000);
    let (bytes, _) = waveform_of(&wave);
    let mut phases = Vec::new();
    let mut inconclusive = Vec::new();
    for session in 0..sessions {
        match ma1_run(usrp(), &wave, &bytes) {
            Ok(phase) => {
                println!("MA-1 Session {session}: phase {phase:.1}°");
                phases.push(phase);
            }
            Err(reason) => {
                println!("MA-1 Session {session}: INCONCLUSIVE: {reason}");
                inconclusive.push(reason);
            }
        }
    }
    let spread = common::tone::circular_spread_deg(&phases);
    println!("MA-1: {} phases {phases:.1?}, largest difference {spread:?}°, {} INCONCLUSIVE", phases.len(), inconclusive.len());
    // Printed only: this probe is not one of the record's raw probes, so nothing collects an observation.
    println!(
        "MA-1 result: {}",
        serde_json::json!({
            "phases_deg": phases, "spread_deg": spread, "inconclusive": inconclusive, "sessions": sessions,
            "note": "a loopback measures the two LOs' relative phase; N Sessions are an observation, not a proof of a guarantee; fewer than two readings have no spread; no value is made",
        })
    );
}

// ---------------------------------------------------------------- design note 39: DDC/DUC timed commands
// The probes of plan/phase8/39-dsp-timed-commands.md §3; run as plan/phase7/bench.md says.

/// `raw`, as the concrete device: the probes also set the DDC/DUC shift alone (`tune_dsp`).
fn raw_uhd() -> Arc<ezsdr_radio_uhd::UhdDevice> {
    let device = Arc::new(ezsdr_radio_uhd::UhdDevice::open(&args()).expect("the USRP opens"));
    if d9_channel() > 0 {
        // Channel 1 is slot B: name both boards as the Module's own from_binding does, and check the
        // channel exists (a one-board device has none).
        for dir in [Dir::Rx, Dir::Tx] {
            device.set_subdev_spec(dir, "A:0 B:0").expect("the two boards, as channels 0 and 1");
            assert!(device.channels(dir) > d9_channel(), "channel {} is not in the device's spec", d9_channel());
        }
    }
    set_sources_with_pll_retries(&*device, "internal", "internal").unwrap();
    device.set_time_zero(false).unwrap();
    std::thread::sleep(Wall::from_millis(100));
    device
}

/// A 100 kHz tone at 1 Msps, amplitude 0.4.
fn tone_1msps(n: usize) -> Vec<ezsdr_radio_uhd::Iq> {
    (0..n)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
            [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
        })
        .collect()
}

/// The received tone's frequency over windows of 25 samples at 1 Msps, as `hw_b8_raw_timed_retune`.
fn tone_freqs(samples: &[(i64, ezsdr_radio_uhd::Iq)]) -> Vec<(i64, f64)> {
    samples
        .chunks_exact(25)
        .map(|w| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for pair in w.windows(2) {
                let (a, b) = (pair[0].1, pair[1].1);
                re += f64::from(b[0] * a[0] + b[1] * a[1]);
                im += f64::from(b[1] * a[0] - b[0] * a[1]);
            }
            (w[0].0, im.atan2(re) / (2.0 * std::f64::consts::PI) * 1e6)
        })
        .collect()
}

/// One sample's frequency from the phase step to the next, for the first `n` samples from `from`.
fn first_steps(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, n: usize) -> Vec<String> {
    samples
        .windows(2)
        .filter(|w| w[0].0 >= from)
        .take(n)
        .map(|w| {
            let (a, b) = (w[0].1, w[1].1);
            let step = f64::from(b[1] * a[0] - b[0] * a[1]).atan2(f64::from(b[0] * a[0] + b[1] * a[1]));
            format!("{:.0}", step / (2.0 * std::f64::consts::PI) * 1e6)
        })
        .collect()
}

/// The tick at which a tone first appears at or after `from`: the first of three consecutive
/// 25-sample windows whose coherence is over 0.5 and whose power is over 4 × `floor`, the reading
/// `classify_tone` gives a tone; `None` when there is no such run.
fn tone_start(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, floor: f64) -> Option<i64> {
    let windows: Vec<(i64, bool)> = samples
        .chunks_exact(25)
        .filter(|w| w[24].0 - w[0].0 == 24 * 200 && w[0].0 >= from)
        .map(|w| {
            let (coherence, _) = common::tone::window_tone(w);
            let power = w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 25.0;
            (w[0].0, coherence > 0.5 && power > 4.0 * floor)
        })
        .collect();
    windows.windows(3).find(|run| run.iter().all(|(_, tone)| *tone) && run[2].0 - run[0].0 == 50 * 200).map(|run| run[0].0)
}

fn flush_tx(device: &dyn Device) -> Vec<String> {
    let mut reports = Vec::new();
    while let Some(report) = device.tx_async(0, Wall::from_millis(10)).unwrap() {
        reports.push(format!("{:?}", report.code));
    }
    reports
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p1_ddc_entries_wait_for_samples() {
    // F2: timed DDC shifts given while nothing streams, then an untimed shift; when the
    // stream starts, which shift does the loopback tone show, from which sample? Three
    // entries at most, far below the 32 the FPGA holds.
    let device = raw_uhd();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    let chan = d9_channel();
    let tone = tone_1msps(40_000);
    for (case, untimed_after) in [("untimed before the entries' instants", false), ("untimed after the entries' instants", true)] {
        device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
        device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
        let now = device.time_now().unwrap();
        let given: Vec<f64> = [20e3, 40e3, 60e3].iter().enumerate().map(|(i, hz)| device.tune_dsp(Dir::Rx, chan, *hz, Some(now + (50 + i as i64) * TICKS_PER_MS)).unwrap()).collect();
        if untimed_after {
            while device.time_now().unwrap() < now + 60 * TICKS_PER_MS {
                std::thread::sleep(Wall::from_millis(1));
            }
        }
        let untimed = device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
        let start = (device.time_now().unwrap().max(now + 60 * TICKS_PER_MS) / 200 + 100_000) * 200;
        device.rx_start(0, start).unwrap();
        device.tx_send(0, &[&tone], Some(start), true, true, Wall::from_millis(500)).unwrap();
        let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= start + 30 * TICKS_PER_MS));
        device.rx_stop(0, None).unwrap();
        let _ = drain(&*device);
        let reports = flush_tx(&*device);
        let freqs = tone_freqs(&got.samples);
        println!(
            "D9 P1 {case}: timed shifts given {given:?}, untimed {untimed}; tone first windows {:?}; median 5–30 ms {:?} Hz; first samples' steps {:?}; tx {reports:?}; other {:?}",
            freqs.iter().take(4).map(|(t, f)| format!("{:+.0} µs {f:.0}", (t - start) as f64 / 200.0)).collect::<Vec<_>>(),
            median_freq(&freqs, start + 5 * TICKS_PER_MS, start + 30 * TICKS_PER_MS),
            first_steps(&got.samples, start, 8),
            got.other,
        );
    }
    device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p2_duc_entry_after_its_samples() {
    // F3: a timed DUC shift at e, 20 ms into a 60 ms burst, given before the burst is sent
    // and given 3 ms before e after the samples covering e were sent: where does the
    // loopback tone move?
    let device = raw_uhd();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    assert!(SAFE_TX_HZ.contains(&(1e9 + 50e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 - 50e3 + 100e3)));
    let chan = d9_channel();
    let tone = tone_1msps(60_000);
    for case in ["given before the burst", "given 3 ms before e, after its samples"] {
        device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
        device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
        let start = (device.time_now().unwrap() / 200 + 150_000) * 200;
        let e = start + 20 * TICKS_PER_MS;
        device.rx_start(0, start - 5 * TICKS_PER_MS).unwrap();
        let (head, tail) = tone.split_at(25_000);
        let mut log = Vec::new();
        if case.starts_with("given before") {
            log.push(format!("dsp {} at {}", device.tune_dsp(Dir::Tx, chan, 50e3, Some(e)).unwrap(), device.time_now().unwrap()));
        }
        let sent = device.tx_send(0, &[head], Some(start), true, false, Wall::from_secs(2)).unwrap();
        log.push(format!("head {sent} sent by {}", device.time_now().unwrap()));
        if !case.starts_with("given before") {
            while device.time_now().unwrap() < e - 3 * TICKS_PER_MS {
                std::thread::sleep(Wall::from_micros(200));
            }
            log.push(format!("dsp {} at {}", device.tune_dsp(Dir::Tx, chan, 50e3, Some(e)).unwrap(), device.time_now().unwrap()));
        }
        let sent = device.tx_send(0, &[tail], None, false, true, Wall::from_secs(2)).unwrap();
        log.push(format!("tail {sent} sent by {}", device.time_now().unwrap()));
        let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|end| end >= start + 60 * TICKS_PER_MS));
        device.rx_stop(0, None).unwrap();
        let _ = drain(&*device);
        let reports = flush_tx(&*device);
        let freqs = tone_freqs(&got.samples);
        let moves = freqs.iter().find(|(t, f)| *t >= start + TICKS_PER_MS && (f - 100e3).abs() > 10e3).map(|(t, _)| (t - e) as f64 / 200.0);
        println!(
            "D9 P2 {case}: e = start + 20 ms; the tone leaves 100 kHz at {moves:?} µs from e; median before e {:?}, after e + 5 ms {:?} Hz; {log:?}; tx {reports:?}",
            median_freq(&freqs, start + TICKS_PER_MS, e),
            median_freq(&freqs, e + 5 * TICKS_PER_MS, start + 60 * TICKS_PER_MS),
        );
    }
    device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p3_duc_entries_between_bursts() {
    // F4: a timed DUC shift in the gap between two bursts, then a timed shift in the next
    // gap followed at once by an untimed one: which shift does each burst carry?
    let device = raw_uhd();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    assert!(SAFE_TX_HZ.contains(&(1e9 + 80e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 - 80e3 + 100e3)));
    let chan = d9_channel();
    device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
    device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
    let burst = tone_1msps(10_000);
    let s1 = (device.time_now().unwrap() / 200 + 150_000) * 200;
    let at = |ms: i64| s1 + ms * TICKS_PER_MS;
    device.rx_start(0, at(-5)).unwrap();
    let mut log = vec![format!("gap 1: dsp {}", device.tune_dsp(Dir::Tx, chan, 50e3, Some(at(20))).unwrap())];
    device.tx_send(0, &[&burst], Some(at(0)), true, true, Wall::from_secs(1)).unwrap();
    device.tx_send(0, &[&burst], Some(at(30)), true, true, Wall::from_secs(1)).unwrap();
    log.push(format!("gap 2 at {}: timed dsp {}", device.time_now().unwrap(), device.tune_dsp(Dir::Tx, chan, 80e3, Some(at(50))).unwrap()));
    log.push(format!("then untimed dsp {}", device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap()));
    device.tx_send(0, &[&burst], Some(at(60)), true, true, Wall::from_secs(1)).unwrap();
    let got = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|e| e >= at(75)));
    device.rx_stop(0, None).unwrap();
    let _ = drain(&*device);
    let reports = flush_tx(&*device);
    let freqs = tone_freqs(&got.samples);
    for (name, from) in [("burst 1 (0 ms)", 0), ("burst 2 (30 ms, after a timed +50 kHz at 20 ms)", 30), ("burst 3 (60 ms, after a timed +80 kHz at 50 ms then an untimed 0)", 60)] {
        println!(
            "D9 P3 {name}: first window {:?} Hz, median {:?} Hz, first samples' steps {:?}",
            freqs.iter().find(|(t, _)| *t >= at(from)).map(|(_, f)| f.round()),
            median_freq(&freqs, at(from) + TICKS_PER_MS, at(from + 10)),
            first_steps(&got.samples, at(from), 6),
        );
    }
    println!("D9 P3: {log:?}; tx {reports:?}; other {:?}", got.other);
    device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
}

// ---------------------------------------------------------------- design note 42: D9a probes P4–P7
// Output only, no assertion; the transmitter is used, so bench.md's attenuator is confirmed
// first (≥ 30 dB in the loopback). Each gives a timed DSP entry at most eight.

/// Reads until two timeouts in a row, keeping the samples (what a stopped stream still delivers).
fn read_tail(device: &dyn Device) -> Read {
    read(device, Wall::from_secs(3), Some(200), |r| r.other.len() >= 2 && r.other[r.other.len() - 2..].iter().all(|(_, o)| o == "Timeout"))
}

fn report_d9_p4(result: common::d9::P4Result) {
    let freqs = tone_freqs(&result.after.samples);
    let seen = classify_tone(&result.after.samples, result.again, result.again + 100 * TICKS_PER_MS, result.again + 5 * TICKS_PER_MS, 100e3);
    println!(
        "D9 P4 {}: e = start + 20 ms; unconsumed entry before the restart: {}; tone windows after the restart {} (noise power {:?}, threshold {:.1e}; the frequencies below mean something only where the tone is seen); stop called at {}, returned {} (e − stop call {:.0} µs); last sample before the restart at {:?} ({:?} µs from e); restart at {}: first windows {:?}; median 1–10 ms after {:?} Hz; first steps {:?}; first stream's median e − 7..e − 5 ms {:?} Hz; {:?}; tx {:?}",
        result.case.label(),
        result.last.is_some_and(|tick| tick < result.e) && result.stopped < result.e,
        seen.after,
        seen.floor,
        seen.threshold,
        result.stopping,
        result.stopped,
        (result.e - result.stopping) as f64 / 200.0,
        result.last,
        result.last.map(|tick| (tick - result.e) as f64 / 200.0),
        result.again,
        freqs.iter().take(3).map(|(tick, freq)| format!("{:+.0} µs {freq:.0}", (tick - result.again) as f64 / 200.0)).collect::<Vec<_>>(),
        median_freq(&freqs, result.again + TICKS_PER_MS, result.again + 10 * TICKS_PER_MS),
        first_steps(&result.after.samples, result.again, 6),
        median_freq(&tone_freqs(&result.before.samples), result.e - 7 * TICKS_PER_MS, result.e - 5 * TICKS_PER_MS),
        result.log,
        result.reports,
    );
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p4_ddc_entry_after_an_early_stop() {
    // R2: the shared A/C device sequence is model-checked on FakeDevice; this raw probe retains
    // the RF-tone verdict. B remains hardware-only because it characterizes AUTO/AUTO tuning.
    let device = raw_uhd();
    let chan = d9_channel();
    common::d9::open_raw(&*device, chan);
    assert!(SAFE_TX_HZ.contains(&(1e9 + 50e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 + 100e3)));
    let mut tune = |dir, channel, offset, at| device.tune_dsp(dir, channel, offset, at);
    let a = common::d9::p4_dsp_only(&*device, chan, &mut tune, common::d9::P4Case::DspOnly);
    report_d9_p4(a);

    let tone = tone_1msps(100_000);
    device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
    device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
    let baseline = device.tune_auto(Dir::Rx, chan, 1e9, None).unwrap();
    let mut log = vec![format!("baseline rf/dsp {baseline:?}")];
    let start = (device.time_now().unwrap() / 200 + 100_000) * 200;
    let e = start + 20 * TICKS_PER_MS;
    device.rx_start(0, start).unwrap();
    device.tx_send(0, &[&tone[..30_000]], Some(start), true, true, Wall::from_secs(2)).unwrap();
    let mut valid = false;
    for delta in [7.0, 37.0, 191.0, 1_001.0] {
        let before = device.time_now().unwrap();
        let (rf, dsp) = device.tune_auto(Dir::Rx, chan, 1e9 + delta, Some(e)).unwrap();
        log.push(format!("tune_auto +{delta} Hz: rf {rf} dsp {dsp} (call {before}..{})", device.time_now().unwrap()));
        if dsp != baseline.1 {
            valid = true;
            break;
        }
    }
    let got = read(&*device, Wall::from_secs(2), Some(200), |_| device.time_now().unwrap() >= e - 5 * TICKS_PER_MS);
    let stopping = device.time_now().unwrap();
    device.rx_stop(0, None).unwrap();
    let stopped = device.time_now().unwrap();
    let tail = read_tail(&*device);
    let last = tail.samples.last().or(got.samples.last()).map(|(t, _)| *t);
    let again = (device.time_now().unwrap() / 200 + 20_000) * 200;
    device.rx_start(0, again).unwrap();
    device.tx_send(0, &[&tone], Some(again), true, true, Wall::from_secs(2)).unwrap();
    let after = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|end| end >= again + 20 * TICKS_PER_MS));
    device.rx_stop(0, None).unwrap();
    let _ = drain(&*device);
    let reports = flush_tx(&*device);
    let freqs = tone_freqs(&after.samples);
    let unconsumed = last.is_some_and(|t| t < e) && stopped < e;
    let seen = classify_tone(&after.samples, again, again + 100 * TICKS_PER_MS, again + 5 * TICKS_PER_MS, 100e3);
    println!(
        "D9 P4 B: AUTO/AUTO{}: e = start + 20 ms; unconsumed entry before the restart: {unconsumed}; tone windows after the restart {} (noise power {:?}, threshold {:.1e}; the frequencies below mean something only where the tone is seen); stop called at {stopping}, returned {stopped} (e − stop call {:.0} µs); last sample before the restart at {last:?} ({:?} µs from e); restart at {again}: first windows {:?}; median 1–10 ms after {:?} Hz; first steps {:?}; first stream's median e − 7..e − 5 ms {:?} Hz; {log:?}; tx {reports:?}",
        if valid { "" } else { " — INCONCLUSIVE: the tune's DSP value did not change" },
        seen.after,
        seen.floor,
        seen.threshold,
        (e - stopping) as f64 / 200.0,
        last.map(|t| (t - e) as f64 / 200.0),
        freqs.iter().take(3).map(|(t, f)| format!("{:+.0} µs {f:.0}", (t - again) as f64 / 200.0)).collect::<Vec<_>>(),
        median_freq(&freqs, again + TICKS_PER_MS, again + 10 * TICKS_PER_MS),
        first_steps(&after.samples, again, 6),
        median_freq(&tone_freqs(&got.samples), e - 7 * TICKS_PER_MS, e - 5 * TICKS_PER_MS),
    );

    let c = common::d9::p4_dsp_only(&*device, chan, &mut tune, common::d9::P4Case::DspOnlyReopen);
    report_d9_p4(c);
    device.tune_dsp(Dir::Rx, chan, 0.0, None).unwrap();
    device.tune_dsp(Dir::Tx, chan, 0.0, None).unwrap();
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p5_a_stale_timed_header_after_a_reopen() {
    // F7: compare the stale-header case, its no-header control and the two-zero reset after reopen.
    let cases: Vec<_> = match std::env::var("EZSDR_D9_P5_CASE").as_deref() {
        Ok("timed") => vec![common::d9::P5Case::StaleHeader],
        Ok("untimed") => vec![common::d9::P5Case::NoStaleHeader],
        Ok("reopen") => vec![common::d9::P5Case::FlushAfterReopen],
        _ => vec![common::d9::P5Case::StaleHeader, common::d9::P5Case::NoStaleHeader, common::d9::P5Case::FlushAfterReopen],
    };
    assert!(SAFE_TX_HZ.contains(&(1e9 + 50e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 - 50e3 + 100e3)));
    let device = raw_uhd();
    let chan = d9_channel();
    let mut tune = |dir, channel, hz, at| device.tune_dsp(dir, channel, hz, at);
    for result in common::d9::p5_stale_header(&*device, chan, &mut tune, &cases) {
        let report = classify_tone(&result.capture.samples, result.start, result.start + 60 * TICKS_PER_MS, result.e, 100e3);
        let established = result.old_header.map(|header| header >= result.e);
        let verdict = match established {
            Some(false) => "INCONCLUSIVE: the previous timed header's timestamp is before the new command's instant",
            _ if !report.usable() => "INCONCLUSIVE: no usable signal windows",
            Some(true) if !result.flush_reports.is_empty() => "stale header cleared by the acknowledged two-zero flush; the move should be at e, not before",
            Some(true) => "stale-header prerequisite established",
            None => "no timed header was left (the control case)",
        };
        println!(
            "D9 P5 first burst {}: previous timed header {:?}, new instant e = {} (header ≥ e: {established:?}) — {verdict}; DUC shift {}; flush tx {:?}; {}; tx {:?}",
            result.case.label(),
            result.old_header,
            result.e,
            result.dsp,
            result.flush_reports,
            tone_verdict(&report),
            result.reports
        );
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p6_every_chunk_timed() {
    assert!(SAFE_TX_HZ.contains(&(1e9 + 50e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 - 50e3 + 100e3)));
    let device = raw_uhd();
    let chan = d9_channel();
    let mut tune = |dir, channel, hz, at| device.tune_dsp(dir, channel, hz, at);
    let (results, starvation) = common::d9::p6_every_chunk_timed(&*device, chan, &mut tune);
    for result in results {
        let report = classify_tone(&result.capture.samples, result.start, result.start + 60 * TICKS_PER_MS, result.e, 100e3);
        println!("D9 P6 {}: e = start + 20 ms; {}; DUC {}; tx {:?}", result.case, tone_verdict(&report), result.dsp, result.reports);
    }
    let loopback = continuation_in_loopback(&starvation.capture.samples, starvation.start, starvation.queued_end, starvation.queued_end)
        .map_or_else(|why| why, |n| format!("{n} samples over half the queued part's median |I| + |Q|"));
    println!(
        "D9 P6 starvation: queued end {}; timed continuation at {} sent at {}: {:?}; tx {:?}; continuation loopback: {loopback}; receive errors {:?}",
        starvation.queued_end, starvation.queued_end, starvation.resumed_at, starvation.send, starvation.reports, starvation.capture.other
    );
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d9_p7_the_current_transmit_shape() {
    let device = raw_uhd();
    assert!(SAFE_TX_HZ.contains(&(1e9 + 50e3 + 100e3)) && SAFE_TX_HZ.contains(&(1e9 - 50e3 + 100e3)));
    let chan = d9_channel();
    let mut tune = |dir, channel, hz, at| device.tune_dsp(dir, channel, hz, at);
    let result = common::d9::p7_untimed_continuations(&*device, chan, &mut tune);
    let report = classify_tone(&result.capture.samples, result.start, result.start + 60 * TICKS_PER_MS, result.e, 100e3);
    println!("D9 P7 current shape: DUC {}; e = start + 20 ms; {}; tx {:?}", result.dsp, tone_verdict(&report), result.reports);
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_d70_one_sample_burst_then_timed_burst() {
    let device = raw_uhd();
    assert!(SAFE_TX_HZ.contains(&(1e9 + 100e3)));
    let chan = d9_channel();
    let mut tune = |dir, channel, hz, at| device.tune_dsp(dir, channel, hz, at);
    for result in common::d9::d70_one_sample_then_two(&*device, chan, &mut tune) {
        let floor = window_power(&result.capture.samples, result.rx_at + TICKS_PER_MS, result.rx_at + 8 * TICKS_PER_MS);
        let starts = floor.and_then(|floor| tone_start(&result.capture.samples, result.rx_at, floor)).map(|t| (t - result.start) as f64 / common::d9::SAMPLE_TICKS as f64);
        println!(
            "D70 {}: tone starts at {starts:?} µs from its timed start; priming tx {:?}; tx {:?}; receive errors {:?}",
            result.case,
            result.priming_reports,
            result.reports,
            result.capture.other.iter().filter(|(_, outcome)| outcome != "Timeout").collect::<Vec<_>>()
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
    let mut observations = Vec::new();
    for rate in [400e3, 500e3, 1e6, 2e6] {
        let dir = TempDir::new();
        let device = usrp();
        let mut spec = with_burst(with_tx(receive_spec(1, rate, bench_hz(&*device), None), rate), &waveform, false, "drop_and_flag", 10_000);
        spec["schedule"].as_array_mut().unwrap().push(serde_json::json!({
            "at": { "clock": "radio", "offset_ticks": 10_000 },
            "action": { "kind": "update_parameter", "target": serde_json::to_value(output("rec")).unwrap(),
                        "key": "sink.capture_samples", "value": 5_000 }
        }));
        let inputs = std::collections::BTreeMap::from([(waveform.hash.clone(), bytes.clone())]);
        let doc = bench_profile(&*device, &dir, serde_json::json!({}), bench_envelope(&*device), false);
        let manifest = captured(spec_run(&spec, &doc, device, inputs));
        let samples = read_capture(&capture_of(&manifest, "rec"), 1).remove(0);
        diagnose(&format!("B6 {} ksps", rate / 1e3), &samples, &wave);
        let at = correlate(&samples, &wave);
        println!("B6 {} ksps: delay {at:?} samples = {:?} µs; TIME_ERROR {}", rate / 1e3, at.map(|a| a as f64 / rate * 1e6), events_of(&manifest, "radio.TIME_ERROR").len());
        observations.push(serde_json::json!({
            "rate_hz": rate,
            "correlation_delay_samples": at,
            "total_delay_us": at.map(|a| a as f64 / rate * 1e6),
            "time_error_count": events_of(&manifest, "radio.TIME_ERROR").len(),
        }));
    }
    measurement::push_raw_observation("hw_b6_delay_by_rate", 4, serde_json::json!({ "rates": observations }));
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
    println!(
        "B9 long receive: {result:?}; termination {:?}; DEVICE_LOST {lost:?}; overflows {}; stats {}",
        manifest.termination.reason,
        events_of(&manifest, "radio.RX_OVERFLOW").len(),
        section(&manifest, "stats")
    );
    assert!(lost.is_empty());
    assert!(matches!(manifest.termination.reason, ezsdr_kernel::run::Termination::Stopped { cause: ezsdr_kernel::run::StopCause::Client {} }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b9_transmit_only_session() {
    // B9's transmit-only Session without the unplug: 60 s with the transmitter enabled and
    // nothing sent, the receiver off if the Session allows it; no false DEVICE_LOST.
    // EZSDR_B9_SECONDS sets the duration (60 by default). A smoke: it sends nothing.
    let seconds: i64 = std::env::var("EZSDR_B9_SECONDS").map_or(60, |text| text.parse().expect("EZSDR_B9_SECONDS names whole seconds"));
    let dir = TempDir::new();
    let mut run = tx_only_session(usrp(), &dir);
    wait(&mut run, ms(seconds * 1_000));
    let manifest = run.finish();
    let lost = events_of(&manifest, ezsdr_kernel::event::EventKind::DEVICE_LOST);
    println!(
        "B9 transmit only: termination {:?}; DEVICE_LOST {lost:?}; TX_UNDERFLOW {}; timing {}",
        manifest.termination.reason,
        events_of(&manifest, "radio.TX_UNDERFLOW").len(),
        section(&manifest, "timing")
    );
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
    let (ours, theirs) = (time.now(root).unwrap().ticks_in(root).unwrap(), device.time_now().unwrap());
    for step in 1..=6 {
        std::thread::sleep(Wall::from_secs(10));
        let (a, b) = (time.now(root).unwrap().ticks_in(root).unwrap() - ours, device.time_now().unwrap() - theirs);
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
    while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
    let (a, b) = (iq(&pn(1_000)), iq(&pn(100)));
    let mut observations = Vec::new();
    for gap in [0i64, 1, 2, 3, 5, 10, 20, 50, 100, 1_000] {
        let mut outcomes = Vec::new();
        for _ in 0..4 {
            let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
            device.tx_send(0, &[&a], Some(t0), true, true, Wall::from_millis(100)).unwrap();
            device.tx_send(0, &[&b], Some(t0 + (1_000 + gap) * 200), true, true, Wall::from_millis(100)).unwrap();
            let mut codes = Vec::new();
            let waited = Instant::now();
            while waited.elapsed() < Wall::from_millis(150) {
                if let Some(report) = device.tx_async(0, Wall::from_millis(20)).unwrap() {
                    codes.push(format!("{:?}", report.code));
                }
            }
            codes.dedup();
            outcomes.push(codes.join("+"));
        }
        println!("B8 burst gap {gap} samples: {outcomes:?}");
        observations.push(serde_json::json!({ "gap_samples": gap, "outcomes": outcomes }));
    }
    measurement::push_raw_observation("hw_b8_raw_burst_gap", 40, serde_json::json!({ "gaps": observations }));
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
        assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&first)).unwrap()));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
        let a_end = clock.origin.ticks_in(clock.root).unwrap() + (a.ticks_in(clock.domain).unwrap() + 30_000) * clock.root_ticks_per_tick.num() as i64;
        while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
            wait(&mut run, ms(1) / 4);
        }
        let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + 30_000);
        let (second, _) = waveform_of(&pn(1_000));
        let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&second)).unwrap();
        wait(&mut run, ms(60));
        let manifest = run.finish();
        println!(
            "B8 burst at a sent burst's end, trial {trial}: {:?}; {} ms before A's end; TIME_ERROR {:?}; async {}; bursts {}",
            entry.outcome,
            (a_end - entry.time.ticks_in(entry.time.domain()).unwrap()) as f64 / TICKS_PER_MS as f64,
            events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
            section(&manifest, "async"),
            section(&manifest, "bursts")
        );
        assert!(events_of(&manifest, "radio.TIME_ERROR").is_empty());
        assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_raw_empty_eob_gap() {
    // Review O, O-B1 (INFERRED there): UHD sends an empty end-of-burst as one zero sample
    // (tx_streamer_impl.hpp:266–276). Does a burst A closed that way, as uhd-tx now closes
    // every burst a device lead before its end, push the next burst's gap-0 drop
    // (hw_b8_raw_burst_gap) one sample later? Against A closed with end-of-burst on its
    // last data buffer, as at bc0db98.
    let device = raw();
    raw_tx(&*device, 1e6, 1e9);
    while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
    let (a, b) = (iq(&pn(1_000)), iq(&pn(100)));
    let mut observations = Vec::new();
    for empty_eob in [true, false] {
        for gap in [0i64, 1, 2, 3, 5] {
            let mut outcomes = Vec::new();
            for _ in 0..4 {
                let t0 = (device.time_now().unwrap() / 200 + 20_000) * 200;
                let end = t0 + 1_000 * 200;
                if empty_eob {
                    device.tx_send(0, &[&a], Some(t0), true, false, Wall::from_millis(100)).unwrap();
                    // As uhd-tx: the end-of-burst a device lead (2 ms) before the end.
                    while device.time_now().unwrap() < end - 2 * TICKS_PER_MS {
                        std::thread::sleep(Wall::from_micros(100));
                    }
                    let empty: Vec<ezsdr_radio_uhd::Iq> = Vec::new();
                    device.tx_send(0, &[&empty], None, false, true, Wall::from_millis(100)).unwrap();
                } else {
                    device.tx_send(0, &[&a], Some(t0), true, true, Wall::from_millis(100)).unwrap();
                }
                device.tx_send(0, &[&b], Some(end + gap * 200), true, true, Wall::from_millis(100)).unwrap();
                let mut codes = Vec::new();
                let waited = Instant::now();
                while waited.elapsed() < Wall::from_millis(150) {
                    if let Some(report) = device.tx_async(0, Wall::from_millis(20)).unwrap() {
                        codes.push(format!("{:?}", report.code));
                    }
                }
                codes.dedup();
                outcomes.push(codes.join("+"));
            }
            let late = outcomes.iter().filter(|o| o.contains("TimeError")).count();
            println!(
                "B8 A closed by {}, B at gap {gap} samples: {late}/4 late; {outcomes:?}",
                if empty_eob { "an empty end-of-burst 2 ms before its end" } else { "end-of-burst on its last data buffer" }
            );
            observations.push(serde_json::json!({ "empty_eob": empty_eob, "gap_samples": gap, "late": late, "outcomes": outcomes }));
        }
    }
    measurement::push_raw_observation("hw_b8_raw_empty_eob_gap", 40, serde_json::json!({ "gaps": observations }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_b8_burst_one_sample_after_a_burst() {
    // Review O, O-B1 on the bench: a burst at a burst's end + 2 samples, booked before the
    // first's last buffer went out and after, is played (UR-22 R1: the held-tail close is
    // unchanged, so end + 1 is late; an empty end-of-burst made it late too: hw_b8_raw_empty_eob_gap).
    // End + 1 is the late trial: one late TIME_ERROR, and no send at that instant.
    for (late_booking, offset) in [(false, 30_002), (true, 30_002), (false, 30_001), (true, 30_001)] {
        let dir = TempDir::new();
        let mut run = tx_session(usrp(), &dir);
        wait(&mut run, ms(1));
        let a = tx_at(&run, ms(40));
        let (first, _) = waveform_of(&pn(30_000));
        assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&first)).unwrap()));
        let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
        let a_end = clock.origin.ticks_in(clock.root).unwrap() + (a.ticks_in(clock.domain).unwrap() + 30_000) * clock.root_ticks_per_tick.num() as i64;
        if late_booking {
            while run.now().ticks_in(clock.root).unwrap() < a_end - ms(8) {
                wait(&mut run, ms(1) / 4);
            }
        }
        let b = TimePoint::new(a.domain(), a.ticks_in(clock.domain).unwrap() + offset);
        let (second, _) = waveform_of(&pn(1_000));
        let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&second)).unwrap();
        wait(&mut run, ms(80));
        let manifest = run.finish();
        println!(
            "B8 burst at end + {} samples, booked late {late_booking}: {:?}; TIME_ERROR {:?}; async {}; bursts {}",
            offset - 30_000,
            entry.outcome,
            events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
            section(&manifest, "async"),
            section(&manifest, "bursts")
        );
        let late = events_of(&manifest, "radio.TIME_ERROR");
        if offset == 30_001 {
            assert_eq!(late.len(), 1, "end + 1 is one late TIME_ERROR: {late:?}");
            assert_eq!(late[0].payload["cause"], "late");
        } else {
            assert!(late.is_empty());
        }
        assert!(section(&manifest, "async").as_array().unwrap().iter().all(|r| r["code"] != "TimeError" && r["code"] != "Underflow"));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_s1_short_first_sends() {
    // UR-22 (S1) on the bench, print only: a burst of 1, 2, 3 or 4 samples at 1 MS/s, then a 10 000-sample burst 10 000 samples later. Every async report and the bursts are printed for the design note.
    let device = usrp();
    pin_slot_a(&*device);
    for samples in [1usize, 2, 3, 4] {
        let dir = TempDir::new();
        let mut run = tx_session(device.clone(), &dir);
        let (short, _) = waveform_of(&[(0.4f32, 0.0f32); 4][..samples]);
        let (long, _) = waveform_of(&tone_wave(1e6, 10_000, 100e3));
        let a = tx_at(&run, ms(40));
        let b = TimePoint::new(a.domain(), a.ticks_in(a.domain()).unwrap() + 10_000);
        assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&short)).unwrap()));
        assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&long)).unwrap()));
        wait(&mut run, ms(200));
        let manifest = run.finish();
        println!(
            "S1 first send of {samples} samples: TIME_ERROR {:?}; TX_UNDERFLOW {}; async {}; bursts {}",
            events_of(&manifest, "radio.TIME_ERROR").iter().map(|e| e.payload.clone()).collect::<Vec<_>>(),
            events_of(&manifest, "radio.TX_UNDERFLOW").len(),
            section(&manifest, "async"),
            section(&manifest, "bursts")
        );
    }
}

/// Where a block sequence leaves a gap or an overlap: (expected first tick, got).
fn breaks(r: &Read, per: i64) -> Vec<(i64, i64)> {
    r.blocks.windows(2).filter(|w| w[0].0 + w[0].1 as i64 * per != w[1].0).map(|w| (w[0].0 + w[0].1 as i64 * per, w[1].0)).collect()
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_m1_rx_front_end_moves_between_streamers() {
    // 1b-4e-2's INFERRED point (UR-25, UR-33): a front end that a closed streamer held opens
    // on another stream's streamer and streams from its timed start without a gap. A second
    // open while one streamer holds the front end is not tried here: on UHD 4.10 it is
    // refused ("Attempting to reconnect output port"), but the attempt cuts the holder's
    // edge ("no neighbour found" on its stop) and the process then aborts (`map::at`,
    // SIGABRT); plan/phase8/bench-results.md, 2026-10-21.
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    let per = 200; // ticks a sample at 1 MS/s on the 200 MHz master clock
    let start = (device.time_now().unwrap() / per + 50_000) * per;
    device.rx_start(0, start).unwrap();
    let first = read_on(&*device, 0, Wall::from_secs(2), None, |r| r.end(per).is_some_and(|e| e >= start + 100 * TICKS_PER_MS));
    device.rx_stop(0, None).unwrap();
    let rest = read_on(&*device, 0, Wall::from_secs(3), None, |r| r.other.len() >= 2 && r.other[r.other.len() - 2..].iter().all(|(_, o)| o == "Timeout"));
    println!(
        "M1 rx s0: first block {:?}, {} blocks, breaks {:?}, after stop {} blocks, other {:?}",
        first.blocks.first(),
        first.blocks.len(),
        breaks(&first, per),
        rest.blocks.len(),
        first.other.iter().chain(&rest.other).filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
    );
    assert_eq!(first.blocks.first().map(|b| b.0), Some(start), "stream 0 starts at its tick");
    device.rx_close(0);
    device.rx_open(1, &[0]).expect("the front end opens on stream 1 once stream 0's streamer is closed");
    let start = (device.time_now().unwrap() / per + 50_000) * per;
    device.rx_start(1, start).unwrap();
    let moved = read_on(&*device, 1, Wall::from_secs(2), None, |r| r.end(per).is_some_and(|e| e >= start + 500 * TICKS_PER_MS));
    let other: Vec<_> = moved.other.iter().filter(|(_, o)| o != "Timeout").collect();
    println!(
        "M1 rx s1 after the move: first block {:?} (start {start}), {} blocks to {:?}, breaks {:?}, other {other:?}",
        moved.blocks.first(),
        moved.blocks.len(),
        moved.end(per),
        breaks(&moved, per)
    );
    assert_eq!(moved.blocks.first().map(|b| b.0), Some(start), "stream 1 starts at its tick");
    assert!(breaks(&moved, per).is_empty() && other.is_empty(), "stream 1 streams 500 ms without a gap");
    device.rx_stop(1, None).unwrap();
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_m2_tx_front_end_moves_between_streamers() {
    // As M1 for transmit: a burst on stream 0, its streamer closed, the front end opened on
    // stream 1 and a burst from there, while a receiver on the loopback reads throughout;
    // both tones arrive. (A second open while one streamer holds the front end: see M1.)
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    while device.tx_async(0, Wall::from_millis(10)).unwrap().is_some() {}
    let (per, ms) = (200, TICKS_PER_MS);
    // A 100 kHz tone of amplitude 0.3, 20 ms long.
    let tone: Vec<ezsdr_radio_uhd::Iq> = (0..20_000)
        .map(|i| {
            let p = 2.0 * std::f32::consts::PI * 0.1 * i as f32;
            [0.3 * p.cos(), 0.3 * p.sin()]
        })
        .collect();
    let t0 = (device.time_now().unwrap() / per + 200_000) * per;
    let (b0, b1) = (t0 + 100 * ms, t0 + 600 * ms);
    // Waits for stream k's burst acknowledgement, keeping every report.
    let ack = |k: usize| {
        let mut codes = Vec::new();
        let begun = Instant::now();
        while begun.elapsed() < Wall::from_secs(2) && !codes.iter().any(|c| c == "BurstAck") {
            if let Some(report) = device.tx_async(k, Wall::from_millis(50)).unwrap() {
                codes.push(format!("{:?}", report.code));
            }
        }
        codes
    };
    device.rx_start(0, t0).unwrap();
    let (rx, acks0, acks1, opened) = std::thread::scope(|scope| {
        let reader = scope.spawn(|| read_on(&*device, 0, Wall::from_secs(4), Some(per), |r| r.end(per).is_some_and(|e| e >= t0 + 800 * ms)));
        device.tx_send(0, &[&tone], Some(b0), true, true, Wall::from_millis(500)).unwrap();
        let acks0 = ack(0);
        device.tx_close(0);
        let opened = device.tx_open(1, &[0]);
        let acks1 = if opened.is_ok() {
            device.tx_send(1, &[&tone], Some(b1), true, true, Wall::from_millis(500)).unwrap();
            ack(1)
        } else {
            Vec::new()
        };
        (reader.join().unwrap(), acks0, acks1, opened)
    });
    device.rx_stop(0, None).unwrap();
    let rms = |from: i64, to: i64| {
        let w: Vec<f32> = rx.samples.iter().filter(|(t, _)| *t >= from && *t < to).map(|(_, s)| s[0] * s[0] + s[1] * s[1]).collect();
        ((w.iter().sum::<f32>() / w.len().max(1) as f32).sqrt(), w.len())
    };
    let (quiet, burst0, burst1) = (rms(t0 + 20 * ms, t0 + 80 * ms), rms(b0 + 2 * ms, b0 + 18 * ms), rms(b1 + 2 * ms, b1 + 18 * ms));
    println!(
        "M2 tx: open on s1 {opened:?}; acks s0 {acks0:?}, s1 {acks1:?}; rx rms (value, samples): quiet {quiet:?}, s0 burst {burst0:?}, s1 burst {burst1:?}; rx first {:?}, breaks {:?}, other {:?}",
        rx.blocks.first(),
        breaks(&rx, per),
        rx.other.iter().filter(|(_, o)| o != "Timeout").collect::<Vec<_>>()
    );
    opened.expect("the front end opens on stream 1 once stream 0's streamer is closed");
    assert!(acks0 == ["BurstAck"] && acks1 == ["BurstAck"], "both bursts are played on time");
    assert!(burst0.1 > 0 && burst1.1 > 0 && burst0.0 > 10.0 * quiet.0 && burst1.0 > 10.0 * quiet.0, "both bursts reach the loopback receiver");
}

// ----------------------------------------------------------- stage 3c-2b observations

const LOOPBACK_DELAY_SAMPLES: i64 = 44;
const SIGNAL_THRESHOLD: f64 = 0.001;

fn ticks_for_wall(ticks: i64, master_clock_hz: u64) -> Wall {
    let ns = (u128::try_from(ticks.max(0)).unwrap() * 1_000_000_000).div_ceil(u128::from(master_clock_hz));
    Wall::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX))
}

fn wait_until_host(deadline: Instant) {
    while Instant::now() < deadline {
        std::thread::sleep((deadline - Instant::now()).min(Wall::from_millis(1)));
    }
}

fn read_until_tick_without_clock(device: &dyn Device, end_tick: i64, ticks_per_sample: i64, limit: Wall) -> Read {
    let mut out = Read::default();
    let begun = Instant::now();
    while begun.elapsed() < limit && out.end(ticks_per_sample).is_none_or(|end| end < end_tick) {
        match device.rx_recv(0, 2_000, Wall::from_millis(100), false) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                out.blocks.push((first_tick, samples[0].len()));
                out.samples.extend(samples[0].iter().enumerate().map(|(i, sample)| (first_tick + i as i64 * ticks_per_sample, *sample)));
            }
            other => out.other.push((-1, format!("{other:?}"))),
        }
    }
    out
}

fn drain_without_clock(device: &dyn Device, timeout: Wall, limit: Wall) -> Vec<Json> {
    let begun = Instant::now();
    let mut idle = 0;
    let mut rows = Vec::new();
    while begun.elapsed() < limit && idle < 2 {
        match device.rx_recv(0, 2_000, timeout, false) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                idle = 0;
                rows.push(serde_json::json!({ "kind": "samples", "first_tick": first_tick, "samples": samples[0].len() }));
            }
            ezsdr_radio_uhd::RxRecv::Timeout => {
                idle += 1;
                rows.push(serde_json::json!({ "kind": "timeout" }));
            }
            other => {
                idle = 0;
                rows.push(serde_json::json!({ "kind": "other", "result": format!("{other:?}") }));
            }
        }
    }
    rows
}

fn raw_report_rows(device: &dyn Device, limit: Wall) -> Vec<Json> {
    let begun = Instant::now();
    let mut quiet = 0;
    let mut rows = Vec::new();
    while begun.elapsed() < limit {
        match device.tx_async(0, Wall::from_millis(20)) {
            Ok(Some(report)) => {
                quiet = 0;
                rows.push(serde_json::json!({ "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel }));
            }
            Ok(None) => {
                quiet += 1;
                if quiet >= 3 {
                    break;
                }
            }
            Err(error) => {
                rows.push(serde_json::json!({ "code": "tx_async_error", "error": error.message }));
                break;
            }
        }
    }
    rows
}

fn raw_send_row(device: &dyn Device, wave: &[ezsdr_radio_uhd::Iq], at: Option<i64>, sob: bool, eob: bool, trial_origin: Instant) -> Json {
    assert!(wave.iter().all(|sample| f64::from(sample[0]).hypot(f64::from(sample[1])) <= 0.5), "bench RF safety: TX amplitude exceeds 0.5");
    let host_before_ns = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let result = device.tx_send(0, &[wave], at, sob, eob, Wall::from_millis(100));
    let host_after_ns = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let accepted = result.as_ref().ok().copied();
    serde_json::json!({
        "requested_samples": wave.len(), "accepted_samples": accepted,
        "difference_samples": accepted.map(|count| wave.len() as i64 - count as i64),
        "at_tick": at, "sob": sob, "eob": eob,
        "host_before_ns": host_before_ns, "host_after_ns": host_after_ns,
        "error": result.err().map(|error| error.message),
    })
}

fn correlation_json(result: observations::CorrelationOffset) -> Json {
    serde_json::json!({
        "status": format!("{:?}", result.status), "matched_tick": result.matched_tick,
        "sample_offset": result.sample_offset, "peak": result.peak,
        "second_peak": result.second_peak, "median": result.median,
    })
}

fn safe_pn_iq(n: usize) -> Vec<ezsdr_radio_uhd::Iq> {
    pn(n).into_iter().map(|(i, q)| [i * 0.7, q * 0.7]).collect()
}

fn restore_raw_probe(device: &dyn Device) -> Json {
    let mut errors = Vec::new();
    if let Err(error) = device.rx_stop(0, None) {
        errors.push(format!("rx_stop: {}", error.message));
    }
    let drain = drain_without_clock(device, Wall::from_millis(10), Wall::from_millis(300));
    device.rx_close(0);
    device.tx_close(0);
    let mut applied = Vec::new();
    for (dir, antenna) in [(Dir::Rx, "RX2"), (Dir::Tx, "TX/RX")] {
        for chan in 0..device.channels(dir) {
            let settings = Settings { rate: Some(1e6), freq: Some(1e9), gain: Some(0.0), antenna: Some(antenna.to_owned()) };
            match device.apply(dir, chan, &settings, None) {
                Ok(readback) => {
                    applied.push(serde_json::json!({ "direction": dir.name(), "channel": chan, "rate": readback.rate, "freq": readback.freq, "gain": readback.gain, "antenna": readback.antenna }))
                }
                Err(error) => errors.push(format!("{} channel {chan}: {}", dir.name(), error.message)),
            }
        }
    }
    serde_json::json!({ "rx_stopped": true, "rx_drain": drain, "tx_streamer_closed": true, "applied": applied, "errors": errors })
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_h1_late_multi_packet_burst() {
    // H1 compares a single timed send with later untimed sends after its first packet is late.
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    let wave = tone_1msps(20_000);
    assert!(wave.iter().all(|sample| f64::from(sample[0]).hypot(f64::from(sample[1])) <= 0.5), "bench RF safety");
    let mut trials = Vec::new();
    for mode in ["all_timed", "timed_head_untimed_tail"] {
        for late_ms in [1i64, 5, 10] {
            for trial in 0..3 {
                let trial_origin = Instant::now();
                let host_before = Instant::now();
                let device_time_host_before_ns = u64::try_from(host_before.duration_since(trial_origin).as_nanos()).unwrap_or(u64::MAX);
                let d0 = device.time_now().unwrap();
                let host_after = Instant::now();
                let device_time_host_after_ns = u64::try_from(host_after.duration_since(trial_origin).as_nanos()).unwrap_or(u64::MAX);
                let rx_start = d0 + 20 * TICKS_PER_MS;
                let send_reference = d0 + 70 * TICKS_PER_MS;
                let tx_at = send_reference - late_ms * TICKS_PER_MS;
                let rx_start_before = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                let rx_start_result = device.rx_start(0, rx_start);
                let rx_start_after = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                wait_until_host(host_after + ticks_for_wall(send_reference - d0, device.master_clock_rate()));
                let send_host_ns = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                let mut sends = Vec::new();
                if mode == "all_timed" {
                    sends.push(raw_send_row(&*device, &wave, Some(tx_at), true, true, trial_origin));
                } else {
                    sends.push(raw_send_row(&*device, &wave[..500], Some(tx_at), true, false, trial_origin));
                    let rest = &wave[500..];
                    let chunks: Vec<_> = rest.chunks(5_000).collect();
                    for (index, chunk) in chunks.iter().enumerate() {
                        sends.push(raw_send_row(&*device, chunk, None, false, index + 1 == chunks.len(), trial_origin));
                    }
                }
                let received = read_until_tick_without_clock(&*device, d0 + 110 * TICKS_PER_MS, 200, Wall::from_millis(500));
                let intervals = observations::heard_intervals(&received.samples, 200, SIGNAL_THRESHOLD, LOOPBACK_DELAY_SAMPLES);
                let reports = raw_report_rows(&*device, Wall::from_secs(1));
                device.rx_stop(0, None).unwrap_or_else(|error| println!("H1 rx_stop: {}", error.message));
                let drain = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_millis(300));
                trials.push(serde_json::json!({
                    "mode": mode, "late_ms_from_pretrial_device_read_inferred": late_ms, "trial": trial,
                    "d0_tick": d0, "device_time_read_host_before_ns": device_time_host_before_ns,
                    "device_time_read_host_after_ns": device_time_host_after_ns,
                    "rx_start_tick": rx_start, "rx_start_host_before_ns": rx_start_before,
                    "rx_start_host_after_ns": rx_start_after, "rx_start_error": rx_start_result.err().map(|error| error.message),
                    "tx_reference_tick": send_reference, "requested_tx_tick": tx_at,
                    "send_host_ns_after_reference_wait": send_host_ns, "sends": sends,
                    "reports": reports, "report_counts": observations::count_report_codes(&reports),
                    "heard_intervals": intervals.iter().map(|interval| serde_json::json!({
                        "first_tick": interval.first_tick, "last_tick": interval.last_tick, "samples": interval.samples,
                    })).collect::<Vec<_>>(), "signal_amplitude_threshold": SIGNAL_THRESHOLD,
                    "received_blocks": received.blocks, "receive_other": received.other, "drain": drain,
                }));
            }
        }
    }
    let cleanup = restore_raw_probe(&*device);
    println!("H1 late multi-packet burst: {} trials", trials.len());
    measurement::push_raw_observation(
        "hw_h1_late_multi_packet_burst",
        18,
        serde_json::json!({
            "trials": trials, "loopback_delay_samples": LOOPBACK_DELAY_SAMPLES,
            "amplitude_threshold": SIGNAL_THRESHOLD, "cleanup": cleanup,
            "timing_note": "Each trial reads device time once; requested lateness is inferred from that read and the host wait, not re-read from the device.",
        }),
    );
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_h2_underflow_mid_burst() {
    // H2 starves an open burst, then resumes with untimed PN samples and records their alignment.
    let device = raw();
    raw_rx(&*device, 1e6, 1e9);
    raw_tx(&*device, 1e6, 1e9);
    let wave = safe_pn_iq(20_000);
    assert!(wave.iter().all(|sample| f64::from(sample[0]).hypot(f64::from(sample[1])) <= 0.5), "bench RF safety");
    let head = &wave[..10_000];
    let tail = &wave[10_000..];
    let per_sample = 200;
    let mut observed_underflow = Vec::new();
    let mut no_underflow = Vec::new();
    for trial in 0..3 {
        let _ = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_millis(300));
        let trial_origin = Instant::now();
        let host_before = Instant::now();
        let device_time_host_before_ns = u64::try_from(host_before.duration_since(trial_origin).as_nanos()).unwrap_or(u64::MAX);
        let d0 = device.time_now().unwrap();
        let host_after = Instant::now();
        let device_time_host_after_ns = u64::try_from(host_after.duration_since(trial_origin).as_nanos()).unwrap_or(u64::MAX);
        let rx_start = d0 + 60 * TICKS_PER_MS;
        let tx_start = d0 + 80 * TICKS_PER_MS;
        let tail_send_tick = tx_start + 20 * TICKS_PER_MS;
        let rx_started = device.rx_start(0, rx_start);
        let head_send = raw_send_row(&*device, head, Some(tx_start), true, false, trial_origin);
        wait_until_host(host_after + ticks_for_wall(tail_send_tick - d0, device.master_clock_rate()));
        let tail_send_host_ns = u64::try_from(trial_origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let tail_send = raw_send_row(&*device, tail, None, false, true, trial_origin);
        let received = read_until_tick_without_clock(&*device, d0 + 135 * TICKS_PER_MS, per_sample, Wall::from_millis(700));
        let reports = raw_report_rows(&*device, Wall::from_secs(1));
        let report_counts = observations::count_report_codes(&reports);
        let underflow = report_counts.contains_key("Underflow") || report_counts.contains_key("UnderflowInPacket");
        let expected_head = tx_start + LOOPBACK_DELAY_SAMPLES * per_sample;
        let expected_tail = tx_start + 10 * TICKS_PER_MS + LOOPBACK_DELAY_SAMPLES * per_sample;
        let head_match = observations::correlation_offset(&received.samples, &head[..1_024], expected_head, per_sample, 20_000);
        let tail_match = observations::correlation_offset(&received.samples, &tail[..1_024], expected_tail, per_sample, 30_000);
        let row = serde_json::json!({
            "trial": trial, "d0_tick": d0,
            "device_time_read_host_before_ns": device_time_host_before_ns,
            "device_time_read_host_after_ns": device_time_host_after_ns,
            "rx_start_tick": rx_start, "rx_start_error": rx_started.err().map(|error| error.message),
            "tx_start_tick": tx_start, "planned_head_end_tick": tx_start + 10 * TICKS_PER_MS,
            "starvation_after_head_end_ms": 10, "tail_send_inferred_tick": tail_send_tick,
            "tail_send_host_ns": tail_send_host_ns, "sends": [head_send, tail_send],
            "underflow_observed": underflow, "reports": reports, "report_counts": report_counts,
            "underflow_report_ticks": reports.iter().filter(|r| matches!(r["code"].as_str(), Some("Underflow" | "UnderflowInPacket"))).map(|r| r["tick"].clone()).collect::<Vec<_>>(),
            "head_correlation": correlation_json(head_match), "tail_correlation": correlation_json(tail_match),
            "tail_heard_tick_source_axis": tail_match.matched_tick.map(|tick| tick - LOOPBACK_DELAY_SAMPLES * per_sample),
            "tail_offset_samples": tail_match.sample_offset, "tail_heard": tail_match.status == observations::CorrelationStatus::Found,
            "loopback_delay_samples": LOOPBACK_DELAY_SAMPLES,
            "received_blocks": received.blocks, "receive_other": received.other,
        });
        if underflow {
            observed_underflow.push(row);
        } else {
            no_underflow.push(row);
        }
        device.rx_stop(0, None).unwrap_or_else(|error| println!("H2 rx_stop: {}", error.message));
        let _ = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_millis(300));
    }
    let cleanup = restore_raw_probe(&*device);
    println!("H2 underflow mid-burst: underflow {}, no underflow {}", observed_underflow.len(), no_underflow.len());
    measurement::push_raw_observation(
        "hw_h2_underflow_mid_burst",
        3,
        serde_json::json!({
            "observed_underflow": observed_underflow, "no_underflow_observed": no_underflow,
            "starvation_ms": 10, "cleanup": cleanup,
            "correlation_note": "PN correlation uses a 1,024-sample prefix; ties and weak peaks are recorded as ambiguous or absent.",
        }),
    );
}

fn tx_send_rows_since(probe: &probe_device::ProbeDevice, count: usize) -> Vec<Json> {
    probe.rows().into_iter().filter(|row| row["call"] == "tx_send").skip(count).collect()
}

fn wait_for_tx_eob(probe: &probe_device::ProbeDevice, eob_count: usize, timeout: Wall) -> Option<Json> {
    let deadline = Instant::now() + timeout;
    loop {
        if probe.tx_eob_count() > eob_count {
            return probe.rows().into_iter().filter(|row| row["call"] == "tx_send" && row["eob"].as_bool() == Some(true)).nth(eob_count);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Wall::from_millis(1));
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_txp_b0_send_sizes() {
    // TXP B0 (diagnostic, about 5 s a rate): window/4 limits TX to 2,500 / 12,500 samples at 1 / 5 MS/s;
    // DEFAULT_BLOCK_LEN remains 2,000 for RX. Flushes are left out; closing sends (end of burst) are checked
    // against T2 (at least two samples) and are not counted as payload.
    assert_eq!(ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, 2_000, "the RX block is still 2,000 samples");
    for (rate, chunk) in [(1e6, 2_500usize), (5e6, 12_500)] {
        let inner = usrp();
        let probe = Arc::new(probe_device::ProbeDevice::new(inner));
        let dir = TempDir::new();
        // Gain 0 dB and the rate are asked at connect (note 47 stage 2).
        let profile = bench_profile(&*probe, &dir, serde_json::json!({}), bench_envelope(&*probe), true);
        let initial = with_initial(
            rx0_tx0(&profile),
            &[("radio.tx.c0.gain_db", ezsdr_kernel::spec::Scalar::try_from(0.0).unwrap()), ("radio.tx.s0.sample_rate_hz", ezsdr_kernel::spec::Scalar::try_from(rate).unwrap())],
        );
        let mut run = session_with(&profile, probe.clone(), &initial);
        past_t0(&mut run, ms(1));
        past_tx_origin(&mut run);
        let wave: Vec<_> = pn(31_000).into_iter().map(|(re, im)| (re * 0.7, im * 0.7)).collect();
        assert!(wave.iter().all(|(re, im)| f64::from(*re).hypot(f64::from(*im)) <= 0.5), "bench RF safety");
        let (bytes, _) = waveform_of(&wave);
        let baseline = probe.count("tx_send");
        assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&bytes)).unwrap()));
        wait(&mut run, ms(5_000));
        let _ = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None);
        wait(&mut run, ms(200));
        let _ = run.finish();
        let rows = tx_send_rows_since(&probe, baseline);
        let sizes: Vec<usize> =
            rows.iter().filter(|row| row["tx_flush"] != true && row["eob"] != true).filter_map(|row| row["sample_counts"][0].as_u64()).filter(|n| *n > 0).map(|n| n as usize).collect();
        let closing: Vec<usize> = rows.iter().filter(|row| row["tx_flush"] != true && row["eob"] == true).filter_map(|row| row["sample_counts"][0].as_u64()).map(|n| n as usize).collect();
        println!("B0 sizes rate {rate}: {} payload sends, sizes {sizes:?}; closing sends {closing:?}", sizes.len());
        assert!(!sizes.is_empty(), "the repeat sent nothing at {rate}");
        assert!(closing.iter().all(|n| *n >= 2), "T2: a closing send has fewer than two samples at {rate}: {closing:?}");
        let mut offset = 0;
        for (index, size) in sizes.iter().copied().enumerate() {
            let expected = chunk.min(31_000 - offset);
            if index == 0 {
                assert!((1..=expected).contains(&size), "the first timed SOB takes at most the window-allowed buffer: {sizes:?}");
            } else {
                assert_eq!(size, expected, "continuation/wrap sizing at rate {rate}, waveform offset {offset}: {sizes:?}");
            }
            offset = (offset + size) % 31_000;
        }
    }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_h5_held_last_sample() {
    // Prebooked adjacent and positive-gap controls, then isolated bursts with no queued successor.
    let inner = usrp();
    let probe = Arc::new(probe_device::ProbeDevice::new(inner));
    let dir = TempDir::new();
    let mut run = tx_session(probe.clone(), &dir);
    let wave: Vec<_> = pn(100).into_iter().map(|(re, im)| (re * 0.7, im * 0.7)).collect();
    assert!(wave.iter().all(|(re, im)| f64::from(*re).hypot(f64::from(*im)) <= 0.5), "bench RF safety");
    let (bytes, _) = waveform_of(&wave);
    let mut cases = Vec::new();
    let mut case_ranges = Vec::new();
    for (name, gap_samples) in [("adjacent_control", 0i64), ("positive_gap", 3_000i64)] {
        let base = tx_at(&run, ms(1_000));
        let first_tick = base.ticks_in(base.domain()).unwrap();
        let interval = 100 + gap_samples;
        let end_tick = first_tick + 199 * interval + 100;
        let tx_send_baseline = probe.count("tx_send");
        case_ranges.push((base.domain(), first_tick, end_tick));
        let mut submits = Vec::new();
        for index in 0..200i64 {
            let at = TimePoint::new(base.domain(), base.ticks_in(base.domain()).unwrap() + index * interval);
            let host_before_ns = probe.host_ns();
            let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(at), &[]), Some(&bytes)).unwrap();
            let host_after_ns = probe.host_ns();
            submits.push(serde_json::json!({
                "index": index, "requested_start": at, "entry_time": entry.time,
                "host_before_ns": host_before_ns, "host_after_ns": host_after_ns,
                "outcome": entry.outcome,
            }));
        }
        wait(&mut run, ms(2_000));
        cases.push(serde_json::json!({
            "name": name, "rate_hz": 1e6, "bursts_requested": 200, "samples_per_burst": 100,
            "gap_samples": gap_samples, "first_requested_start_tick": first_tick,
            "end_tick_exclusive": end_tick, "next_bursts_prebooked": true, "submit_entries": submits,
            "tx_send_records": tx_send_rows_since(&probe, tx_send_baseline),
        }));
    }

    let mut deadline_submits = Vec::new();
    let mut deadline_start_ticks = Vec::new();
    let tx_send_baseline = probe.count("tx_send");
    let mut termination_reason = "all_bursts_submitted";
    for index in 0..200i64 {
        let at = tx_at(&run, ms(15));
        let start_tick = at.ticks_in(at.domain()).unwrap();
        let previous_tick = deadline_start_ticks.last().copied();
        let tx_eob_baseline_for_burst = probe.tx_eob_count();
        let host_before_ns = probe.host_ns();
        let entry = run.submit(verb("send", resource("radio/tx/s0"), Some(at), &[]), Some(&bytes)).unwrap();
        let host_after_ns = probe.host_ns();
        deadline_start_ticks.push(start_tick);
        wait(&mut run, ms(17));
        let eob = wait_for_tx_eob(&probe, tx_eob_baseline_for_burst, Wall::from_secs(1));
        let accepted_eob = eob.as_ref().is_some_and(probe_device::successful_tx_eob);
        let closure_status = match eob.as_ref() {
            None => "not_observed",
            Some(_) if accepted_eob => "accepted",
            Some(_) => "incomplete",
        };
        deadline_submits.push(serde_json::json!({
            "index": index, "requested_start": at, "entry_time": entry.time,
            "host_before_ns": host_before_ns, "host_after_ns": host_after_ns, "outcome": entry.outcome,
            "requested_gap_after_previous_samples": previous_tick.map(|previous| start_tick - previous - 100),
            "accepted_eob_before_next_submit": accepted_eob, "closure_status": closure_status,
            "incomplete_closure": if closure_status == "incomplete" { eob.clone() } else { None }, "eob_record": eob,
        }));
        if deadline_submits.last().unwrap()["accepted_eob_before_next_submit"] != true {
            termination_reason = "accepted_eob_not_observed";
            break;
        }
    }
    let first_deadline_tick = deadline_start_ticks.first().copied().unwrap_or(0);
    let end_deadline_tick = deadline_start_ticks.last().copied().unwrap_or(first_deadline_tick) + 100;
    case_ranges.push((tx_at(&run, ms(15)).domain(), first_deadline_tick, end_deadline_tick));
    let incomplete_closures: Vec<_> = deadline_submits.iter().filter(|submit| submit["closure_status"] == "incomplete").cloned().collect();
    cases.push(serde_json::json!({
        "name": "positive_gap_deadline", "rate_hz": 1e6,
        "bursts_requested": 200, "bursts_submitted": deadline_submits.len(), "samples_per_burst": 100,
        "first_requested_start_tick": first_deadline_tick, "end_tick_exclusive": end_deadline_tick,
        "next_burst_submitted_only_after_prior_accepted_eob": true,
        "termination_reason": termination_reason, "incomplete_closures": incomplete_closures,
        "submit_entries": deadline_submits,
        "tx_send_records": tx_send_rows_since(&probe, tx_send_baseline),
    }));

    let manifest = run.finish();
    let burst_records = bursts(&manifest);
    for (case, (domain, first, end)) in cases.iter_mut().zip(case_ranges) {
        let records: Vec<_> = burst_records
            .iter()
            .filter(|record| record.requested_target.unwrap_or(record.target).ticks_in(domain).is_ok_and(|tick| (first..end).contains(&tick)))
            .map(|record| serde_json::to_value(record).unwrap())
            .collect();
        case["burst_record_count"] = serde_json::json!(records.len());
        case["burst_records"] = Json::Array(records);
    }
    let async_reports = section(&manifest, "async").clone();
    let events: Vec<_> = ["radio.TIME_ERROR", "radio.TX_UNDERFLOW"]
        .into_iter()
        .flat_map(|kind| events_of(&manifest, kind).into_iter().map(move |event| serde_json::json!({ "kind": kind, "time": event.time, "payload": event.payload })))
        .collect();
    let event_counts = serde_json::json!({
        "radio.TIME_ERROR": events.iter().filter(|event| event["kind"] == "radio.TIME_ERROR").count(),
        "radio.TX_UNDERFLOW": events.iter().filter(|event| event["kind"] == "radio.TX_UNDERFLOW").count(),
    });
    let cleanup = restore_raw_probe(&*probe);
    println!(
        "H5 held-last-sample observation: {} submits, {} burst records",
        cases.iter().map(|case| case["bursts_submitted"].as_u64().unwrap_or_else(|| case["bursts_requested"].as_u64().unwrap_or(0))).sum::<u64>(),
        section(&manifest, "bursts").as_array().map_or(0, Vec::len)
    );
    measurement::push_raw_observation(
        "hw_h5_held_last_sample",
        600,
        serde_json::json!({
            "cases": cases, "all_burst_records": section(&manifest, "bursts"),
            "termination": manifest.termination.clone(),
            "async": async_reports, "async_report_counts": observations::count_report_codes(&async_reports.as_array().cloned().unwrap_or_default()),
            "events": events, "event_counts": event_counts, "rejected": section(&manifest, "rejected"),
            "timing": section(&manifest, "timing"), "cleanup": cleanup,
            "note": "Observation from an unladen bench with TX recording instrumentation; recorder overhead was not measured, so this is not a measurement of the held-sample deadline (INFERRED).",
        }),
    );
}

fn session_event_rows(manifest: &ezsdr_kernel::manifest::Manifest, kind: &str) -> Vec<Json> {
    events_of(manifest, kind).into_iter().map(|event| serde_json::json!({ "time": event.time, "payload": event.payload })).collect()
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_56_stop_between_starts() {
    // #56: a Stop of the receive stream that lands between a start's request e1 and its first segment e2. The stream is asked for
    // at 2 MS/s at connect; a first Stop, then `start_rx` (e1 is its request, e2 the origin of the segment it opens), then a second
    // Stop about 10 ms later (note 47 stage 2: the old probe changed the rate at Run time and stopped between that change's two starts).
    let mut trials = Vec::new();
    for trial in 0..3 {
        let inner = usrp();
        let probe = Arc::new(probe_device::ProbeDevice::new(inner));
        let dir = TempDir::new();
        let profile = bench_profile(&*probe, &dir, serde_json::json!({}), serde_json::json!({}), true);
        let initial = with_initial(rx0(&profile), &[("radio.rx.s0.sample_rate_hz", ezsdr_kernel::spec::Scalar::try_from(2e6).unwrap())]);
        let mut run = session_with(&profile, probe.clone(), &initial);
        past_t0(&mut run, ms(100));
        let first_stop = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
        wait(&mut run, ms(20));
        let start_entry = run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap();
        wait(&mut run, ms(10));
        let stop_host_before_ns = probe.host_ns();
        let stop_entry = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
        let stop_host_after_ns = probe.host_ns();
        wait(&mut run, ms(120));
        let device_drain = probe.drain_without_reader(0, 2_000, Wall::from_millis(10), Wall::from_millis(100));
        let recovery_start_baseline = probe.count("rx_start");
        let recovery_entry = run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap();
        let recovery_device_start_and_sample = wait_for_rx_start_and_sample(&probe, recovery_start_baseline, Wall::from_secs(2));
        let capture_entry = recovery_device_start_and_sample.as_ref().map(|_| {
            let capture_at = after_ticks(&run, ms(60));
            let entry = run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(20_000))]), None).unwrap();
            let horizon = after_ticks(&run, ms(3_000));
            let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
            wait(&mut run, ms(100));
            entry
        });
        let manifest = run.finish();
        let timing = section(&manifest, "timing").as_array().cloned().unwrap_or_default();
        let e1 = start_entry.time.ticks_in(start_entry.time.domain()).ok();
        let recovery_tick = recovery_entry.time.ticks_in(recovery_entry.time.domain()).ok();
        let start_segment = timing
            .iter()
            .find(|row| row["what"] == "rx_segment" && e1.zip(recovery_tick).is_some_and(|(e, recovery)| row["origin"].as_i64().is_some_and(|origin| origin >= e && origin < recovery)))
            .cloned();
        let e2 = start_segment.as_ref().and_then(|row| row["origin"].as_i64());
        let stop_tick = stop_entry.time.ticks_in(stop_entry.time.domain()).ok();
        let nominal_placement = match (e1, e2, stop_tick) {
            (Some(e1), Some(e2), Some(stop)) if stop > e1 && stop < e2 => "between_e1_e2",
            (Some(e1), Some(_), Some(stop)) if stop <= e1 => "at_or_before_e1",
            (Some(_), Some(_), Some(_)) => "at_or_after_e2",
            _ => "not_classifiable_from_recorded_segment",
        };
        let device_rows = probe.rows();
        let rx_calls: Vec<_> = device_rows.iter().filter(|row| matches!(row["call"].as_str(), Some("rx_start" | "rx_stop" | "rx_recv" | "device_drain"))).cloned().collect();
        let restart = rx_calls.iter().find(|row| row["call"] == "rx_start" && row["at"].as_i64() == e2).cloned();
        let stop_rx_call = rx_calls.iter().find(|row| row["call"] == "rx_stop" && row["host_before_ns"].as_u64().is_some_and(|time| time >= stop_host_before_ns)).cloned();
        let device_order = match (&stop_rx_call, &restart) {
            (Some(stop), Some(start)) => match (stop["host_after_ns"].as_u64(), start["host_before_ns"].as_u64()) {
                (Some(stopped), Some(started)) if stopped <= started => "rx_stop_before_restart_rx_start",
                (Some(_), Some(_)) => "host_call_intervals_overlap",
                _ => "host_call_times_missing",
            },
            (None, Some(_)) => "rx_stop_after_stop_submit_not_observed",
            (Some(_), None) => "restart_rx_start_not_observed",
            (None, None) => "neither_call_observed",
        };
        let capture = manifest
            .artifacts
            .iter()
            .find(|(name, _)| name.as_str() == "rec")
            .and_then(|(_, artifacts)| artifacts.first())
            .map(|artifact| serde_json::json!({ "continuity": artifact.continuity, "uri": artifact.uri }));
        let cleanup = restore_raw_probe(&*probe);
        trials.push(serde_json::json!({
            "trial": trial, "first_stop": serde_json::to_value(&first_stop).unwrap(), "start_rx": serde_json::to_value(&start_entry).unwrap(),
            "e1_tick": e1, "e2_tick": e2, "stop_placement_nominal": nominal_placement,
            "stop_submit": serde_json::to_value(&stop_entry).unwrap(),
            "stop_host_before_ns": stop_host_before_ns, "stop_host_after_ns": stop_host_after_ns,
            "recovery_start_rx": serde_json::to_value(&recovery_entry).unwrap(),
            "recovery_device_start_and_sample": recovery_device_start_and_sample,
            "recovery_capture": capture_entry.as_ref().map(|entry| serde_json::to_value(entry).unwrap()),
            "device_call_order": {
                "first_rx_stop_after_stop_submit": stop_rx_call,
                "restart_rx_start": restart,
                "ordering": device_order,
            },
            "timing": timing.iter().filter(|row| matches!(row["what"].as_str(), Some("rx_stop_untimed" | "rx_segment" | "stop"))).cloned().collect::<Vec<_>>(),
            "device_rx_calls": rx_calls, "device_level_drain": device_drain,
            "capture_written": capture.is_some(), "capture": capture, "async": section(&manifest, "async"), "rejected": section(&manifest, "rejected"),
            "command_rejected_events": session_event_rows(&manifest, "radio.COMMAND_REJECTED"),
            "time_error_events": session_event_rows(&manifest, "radio.TIME_ERROR"),
            "cleanup": cleanup,
        }));
    }
    println!("#56 Stop between starts: {} trials", trials.len());
    measurement::push_raw_observation("hw_56_stop_between_starts", 3, serde_json::json!({ "trials": trials }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_56_stop_lag() {
    let inner = usrp();
    let probe = Arc::new(probe_device::ProbeDevice::new(inner));
    let dir = TempDir::new();
    let mut run = session(&bench_profile(&*probe, &dir, serde_json::json!({}), serde_json::json!({}), true), probe.clone());
    past_t0(&mut run, ms(100));
    wait(&mut run, ms(60));
    let initial_receive = wait_for_rx_start_and_sample(&probe, 0, Wall::from_secs(2));
    let mut trials = Vec::new();
    let mut not_counted = Vec::new();
    if initial_receive.is_some() {
        for trial in 0..20 {
            let stop_baseline = probe.count("rx_stop");
            let host_before_ns = probe.host_ns();
            let entry = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
            let host_after_ns = probe.host_ns();
            let deadline = Instant::now() + Wall::from_secs(1);
            while probe.count("rx_stop") == stop_baseline && Instant::now() < deadline {
                std::thread::sleep(Wall::from_millis(1));
            }
            let rows = probe.rows();
            let stop_call = rows.iter().filter(|row| row["call"] == "rx_stop").nth(stop_baseline).cloned();
            let call_before = stop_call.as_ref().and_then(|row| row["host_before_ns"].as_u64());
            let call_after = stop_call.as_ref().and_then(|row| row["host_after_ns"].as_u64());
            let enable_baseline = probe.count("rx_start");
            let recovery_host_before_ns = probe.host_ns();
            let enable = run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap();
            let recovery = wait_for_rx_start_and_sample(&probe, enable_baseline, Wall::from_secs(2));
            let row = serde_json::json!({
                "trial": trial, "stop_submit": serde_json::to_value(&entry).unwrap(),
                "stop_submit_host_before_ns": host_before_ns, "stop_submit_host_after_ns": host_after_ns,
                "rx_stop_call": stop_call,
                "submit_to_rx_stop_start_ns": call_before.map(|call| if call >= host_after_ns { i64::try_from(call - host_after_ns).unwrap_or(i64::MAX) } else { -i64::try_from(host_after_ns - call).unwrap_or(i64::MAX) }),
                "submit_to_rx_stop_end_ns": call_after.map(|call| if call >= host_before_ns { i64::try_from(call - host_before_ns).unwrap_or(i64::MAX) } else { -i64::try_from(host_before_ns - call).unwrap_or(i64::MAX) }),
                "re_enable_start_rx": serde_json::to_value(&enable).unwrap(),
                "re_enable_submit_host_before_ns": recovery_host_before_ns,
                "recovery_device_start_and_sample": recovery,
            });
            if stop_call.is_some() && row["recovery_device_start_and_sample"].is_object() {
                trials.push(row);
            } else {
                not_counted.push(row);
            }
            if not_counted.last().is_some_and(|row| row["recovery_device_start_and_sample"].is_null()) {
                break;
            }
        }
    }
    let manifest = run.finish();
    let stop_timing = section(&manifest, "timing").as_array().unwrap().iter().filter(|row| row["what"] == "stop").cloned().collect::<Vec<_>>();
    let device_rx_calls: Vec<_> = probe.rows().into_iter().filter(|row| matches!(row["call"].as_str(), Some("rx_start" | "rx_stop" | "rx_recv"))).collect();
    let cleanup = restore_raw_probe(&*probe);
    println!("#56 Stop lag: {} counted Stops, {} not counted", trials.len(), not_counted.len());
    measurement::push_raw_observation(
        "hw_56_stop_lag",
        20,
        serde_json::json!({
            "initial_receive_start_and_sample": initial_receive,
            "trials": trials, "not_counted_trials": not_counted, "provider_stop_timing": stop_timing,
            "device_rx_calls": device_rx_calls,
            "session_log_stop_time_unit": "Run root ticks; rx_stop lag uses the shared host monotonic nanosecond clock.",
            "cleanup": cleanup,
        }),
    );
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_ur17_receive_call_duration() {
    let device = raw();
    let mut conditions = Vec::new();
    for rate in [390_625.0, 1e6, 10e6, 50e6] {
        for request_samples in [2_000usize, 65_536] {
            device.rx_close(0);
            raw_rx(&*device, rate, 1e9);
            let packet_samples = device.rx_packet_samples(0);
            let ticks_per_sample = (device.master_clock_rate() as f64 / rate).round() as i64;
            let now = device.time_now().unwrap();
            let start = (now + 10 * TICKS_PER_MS + ticks_per_sample - 1).div_euclid(ticks_per_sample) * ticks_per_sample;
            device.rx_start(0, start).unwrap();
            let timeout = Wall::from_secs(1);
            let expected_duration_ns = request_samples as f64 / rate * 1e9;
            let mut calls = Vec::new();
            let mut durations = Vec::new();
            let mut sample_counts = std::collections::BTreeMap::<usize, usize>::new();
            for index in 0..200 {
                let begun = Instant::now();
                let result = device.rx_recv(0, request_samples, timeout, false);
                let duration_ns = u64::try_from(begun.elapsed().as_nanos()).unwrap_or(u64::MAX);
                durations.push(duration_ns);
                let (kind, first_tick, last_tick, samples) = match result {
                    ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                        let count = samples.first().map_or(0, Vec::len);
                        let last = (count > 0).then_some(first_tick + (count as i64 - 1) * ticks_per_sample);
                        ("samples", Some(first_tick), last, count)
                    }
                    ezsdr_radio_uhd::RxRecv::Overflow { out_of_sequence } => (if out_of_sequence { "sequence_error" } else { "overflow" }, None, None, 0),
                    ezsdr_radio_uhd::RxRecv::LateCommand => ("late_command", None, None, 0),
                    ezsdr_radio_uhd::RxRecv::Alignment => ("alignment", None, None, 0),
                    ezsdr_radio_uhd::RxRecv::BadPacket => ("bad_packet", None, None, 0),
                    ezsdr_radio_uhd::RxRecv::Timeout => ("timeout", None, None, 0),
                    ezsdr_radio_uhd::RxRecv::Failed(_) => ("failed", None, None, 0),
                };
                let error = kind != "samples";
                *sample_counts.entry(samples).or_default() += 1;
                calls.push(serde_json::json!({
                    "index": index,
                    "class": if error { "error" } else if index == 0 { "beginning" } else { "steady" },
                    "rate_hz": rate, "packet_samples": packet_samples,
                    "request_samples": request_samples, "timeout_ns": timeout.as_nanos(),
                    "duration_ns": duration_ns, "expected_block_duration_ns": expected_duration_ns,
                    "duration_minus_block_ns": duration_ns as f64 - expected_duration_ns,
                    "result": kind, "first_tick": first_tick, "last_tick": last_tick,
                    "returned_samples": samples,
                }));
            }
            durations.sort_unstable();
            let summary = serde_json::json!({
                "min_ns": durations.first(), "median_ns": durations.get(durations.len() / 2), "max_ns": durations.last(),
                "returned_sample_count_distribution": sample_counts,
            });
            println!("UR-17 rate {rate} S/s request {request_samples}: {summary}");
            conditions.push(serde_json::json!({
                "rate_hz": rate, "request_samples": request_samples, "packet_samples": packet_samples,
                "calls": calls, "summary": summary,
            }));
            let _ = device.rx_stop(0, None);
            let _ = drain_without_clock(&*device, Wall::from_millis(10), Wall::from_millis(300));
        }
    }
    let raw_cleanup = restore_raw_probe(&*device);

    let inner = usrp();
    let probe = Arc::new(probe_device::ProbeDevice::new(inner));
    let dir = TempDir::new();
    let selector = serde_json::json!({ "block_len": 65_536 });
    // 50 MS/s is asked at connect; the cut is a Stop while the reader is paused, and `start_rx` makes the pending segment (note 47 stage 2).
    let profile = bench_profile(&*probe, &dir, selector, serde_json::json!({}), true);
    let initial = with_initial(rx0(&profile), &[("radio.rx.s0.sample_rate_hz", ezsdr_kernel::spec::Scalar::try_from(50e6).unwrap())]);
    let mut run = session_with(&profile, probe.clone(), &initial);
    past_t0(&mut run, ms(100));
    wait(&mut run, ms(300));
    probe.pause_reader();
    let before_count = probe.count("rx_recv");
    let pause_host_ns = probe.host_ns();
    let cut_entry = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
    let restart_entry = run.submit(verb("start_rx", resource("radio/rx/s0"), None, &[]), None).unwrap();
    std::thread::sleep(Wall::from_millis(300));
    let resume_host_ns = probe.host_ns();
    probe.resume_reader();
    wait(&mut run, ms(300));
    let manifest = run.finish();
    let device_rows = probe.rows();
    let rx_rows: Vec<_> = device_rows.iter().filter(|row| row["call"] == "rx_recv").cloned().collect();
    let cut_observation = serde_json::json!({
        "warmup_rx_calls_at_50msps": before_count,
        "paused_host_ns": pause_host_ns, "resume_host_ns": resume_host_ns,
        "stop_submit": serde_json::to_value(&cut_entry).unwrap(), "start_rx_submit": serde_json::to_value(&restart_entry).unwrap(),
        "timing": section(&manifest, "timing").as_array().unwrap().iter()
            .filter(|row| matches!(row["what"].as_str(), Some("rx_segment" | "rx_stop_untimed"))).cloned().collect::<Vec<_>>(),
        "rx_calls_before": rx_rows.iter().take(before_count).rev().take(5).cloned().collect::<Vec<_>>(),
        "rx_calls_after": rx_rows.iter().skip(before_count).take(20).cloned().collect::<Vec<_>>(),
        "overflow_events": session_event_rows(&manifest, "radio.RX_OVERFLOW"),
        "async": section(&manifest, "async"), "rejected": section(&manifest, "rejected"),
    });
    let session_cleanup = restore_raw_probe(&*probe);
    println!("UR-17 receive-call duration: {} conditions, overrun-near-cut calls {}", conditions.len(), cut_observation["rx_calls_after"].as_array().map_or(0, Vec::len));
    measurement::push_raw_observation(
        "hw_ur17_receive_call_duration",
        1_600,
        serde_json::json!({
            "calls_per_rate_and_length": 200, "conditions": conditions,
            "overrun_near_cut": cut_observation,
            "raw_cleanup": raw_cleanup, "session_cleanup": session_cleanup,
        }),
    );
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_h4_load() {
    let guard = measurement::open_record_guard(|| ezsdr_radio_uhd::open(&args()).expect("the USRP opens"));
    if guard.channels(Dir::Rx) == 0 || guard.channels(Dir::Tx) == 0 {
        measurement::push_raw_observation(
            "hw_h4_load",
            1,
            serde_json::json!({
                "status": "skipped_no_rx_tx_pair", "rx_channels": guard.channels(Dir::Rx), "tx_channels": guard.channels(Dir::Tx),
            }),
        );
        return;
    }
    let device: Arc<dyn Device> = guard.clone();
    let probe = Arc::new(common::probe_device::ProbeDevice::new(device));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::measure_h4_load(probe, measurement::H4_WINDOW_MS)));
    let open_streamers = guard.latch_if_streamers_open("hw_h4_load");
    let group = match outcome {
        Ok(Ok(group)) => group,
        Ok(Err(error)) => panic!("H4 load probe failed: {error}"),
        Err(payload) => std::panic::resume_unwind(payload),
    };
    let condition = group.conditions.into_iter().next().expect("H4 measurement describes its stream condition");
    let row = group.observations.into_iter().next().expect("H4 measurement returns its observation");
    let mut result = Json::Object(row.result.into_iter().collect());
    result["measurement_condition"] = serde_json::to_value(condition).expect("H4 condition serializes");
    result["open_streamers_after_run"] = serde_json::to_value(open_streamers).expect("streamer status serializes");
    measurement::push_raw_observation("hw_h4_load", row.trials, result);
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_measurements_record() {
    let metadata = measurement::process_metadata().expect("all EZSDR_MEASUREMENT_* values are valid before opening the USRP");
    let plan = measurement::MeasurementPlan::process_env().expect("measurement trial counts are valid before opening the USRP");
    measurement::clear_raw_observations();
    // The environment's selector (layout, image, links) is laid over every profile this record opens, raw probes included.
    set_selector_override(metadata.selector.clone());
    // One latch for the record: a failure that makes the handle unsafe to touch ends every later group and probe's device access.
    measurement::install_record_latch(Some(measurement::Latch::new()));
    // Each probe and group runs on its own: an error or a panic is that group's failure, its keys are
    // `unmeasured`, and the record is still written (stage 3c-2c).
    let mut groups = measurement::GroupRun::new(metadata.output.as_deref());
    let raw_probes: [(&str, fn()); 14] = [
        ("hw_b8_raw_device_lead", hw_b8_raw_device_lead),
        ("hw_b8_raw_restart_lead", hw_b8_raw_restart_lead),
        ("hw_b8_raw_in_flight_window", hw_b8_raw_in_flight_window),
        ("hw_b8_raw_burst_gap", hw_b8_raw_burst_gap),
        ("hw_b8_raw_empty_eob_gap", hw_b8_raw_empty_eob_gap),
        ("hw_b8_raw_timed_tune_phase", hw_b8_raw_timed_tune_phase),
        ("hw_b6_delay_by_rate", hw_b6_delay_by_rate),
        ("hw_b8_stop_end", hw_b8_stop_end),
        ("hw_h1_late_multi_packet_burst", hw_h1_late_multi_packet_burst),
        ("hw_h2_underflow_mid_burst", hw_h2_underflow_mid_burst),
        ("hw_h5_held_last_sample", hw_h5_held_last_sample),
        ("hw_56_stop_between_starts", hw_56_stop_between_starts),
        ("hw_56_stop_lag", hw_56_stop_lag),
        ("hw_ur17_receive_call_duration", hw_ur17_receive_call_duration),
    ];
    // The device is opened, described and identified first and closed again, so that no raw probe or group can stop the
    // record from being written; a failure here is recorded, and no record can be assembled.
    let setup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let device: Arc<dyn Device> = measurement::open_record_guard(|| ezsdr_radio_uhd::open(&args()).expect("the USRP opens"));
        let dir = TempDir::new();
        let doc = bench_profile(&*device, &dir, serde_json::json!({}), bench_envelope(&*device), false);
        let binding = measurement::radio_binding(&doc).expect("the bench profile's radio binding");
        let description = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).expect("the bound X3xx layout");
        let identity = (device.describe(), device.serial().expect("the device's serial before the measurement groups"));
        (binding, description, identity)
    }));
    let (binding, description, (device_description, serial)) = match setup {
        Ok(setup) => setup,
        Err(payload) => {
            let error = measurement::GroupRun::panic_text(payload);
            groups.record_failure("device_setup", error.clone());
            panic!("the device could not be opened and identified, so no record can be assembled (diagnostics in the partial file): {error}");
        }
    };
    measurement::run_raw_probes(&mut groups, &raw_probes, measurement::raw_probes_applicable(metadata.selector.as_ref()));
    // H4 uses the selected profile's channel 0, not the raw probes' slot-A 1 GHz assumption.
    groups.probe("hw_h4_load", hw_h4_load);
    let expected: std::collections::BTreeSet<_> = raw_probes.iter().map(|(name, _)| (*name).to_owned()).chain(std::iter::once("hw_h4_load".to_owned())).collect();
    assert_eq!(groups.probes(), &expected, "every raw probe either ran or failed");

    // The groups run on a device opened again, unless a probe latched the record: then none is opened and every group is not run.
    let reopened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| measurement::open_record_guard(|| ezsdr_radio_uhd::open(&args()).expect("the USRP opens"))));
    let (stopped, restored) = match reopened {
        Ok(guard) => {
            // From here the device lives in a lease that leaks the handle instead of dropping it unless the
            // raw group marks the run clean, so a failure or a panic anywhere before that never tears it down.
            let mut lease = measurement::DeviceLease::new(guard.clone());
            for (name, measure, keys) in measurement::provider_groups() {
                let keys: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
                // After the latch holds the guard refuses every call, and later groups are not run.
                groups.run(name, &keys, || match measurement::record_latch_reason() {
                    Some(reason) => Err(format!("not run: the device handle is latched ({reason}); nothing is sent to it")),
                    None => measure(lease.device(), &plan),
                });
                guard.latch_if_streamers_open(name);
            }
            // Both throughput values are decided: the two limits are tried together once (a value that is `unmeasured` is not).
            let wire_bytes = u64::try_from(description.performance.wire_bytes_per_sample).expect("the profile's wire bytes per sample");
            groups.run_duplex_pair(&plan, wire_bytes, || lease.device());
            guard.latch_if_streamers_open("duplex_at_the_measured_limits");
            let (stopped, restored) = match measurement::record_latch_reason() {
                Some(reason) => {
                    let skipped = format!("not run: the device handle is latched ({reason}); nothing is sent to it, which may need a power cycle");
                    groups.run("raw_coercion", &measurement::raw_group_keys(&description), || Err(skipped.clone()));
                    (Some(skipped), None)
                }
                None => {
                    let outcome = measurement::run_raw_group_leased(&mut lease, &mut groups, &description, &plan);
                    guard.latch_if_streamers_open("raw_coercion");
                    outcome
                }
            };
            let reopen_keys = ["radio.perf.rx_bytes_per_s"].into_iter().map(str::to_owned).collect::<Vec<_>>();
            let reopened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lease.reopen(guard, || ezsdr_radio_uhd::open(&args()).expect("the USRP reopens"))));
            match reopened {
                Ok(Ok(reopened_guard)) => {
                    let mut reopened_lease = measurement::DeviceLease::new(reopened_guard.clone());
                    let reopened_plan = plan.reopened_plan();
                    let name = "reopened_rx_throughput";
                    let keys = ["radio.perf.rx_bytes_per_s".to_owned()];
                    groups.run_reopened(name, &keys, measurement::ReopenedMerge::Minimum, || match measurement::record_latch_reason() {
                        Some(reason) => Err(format!("not run: the device handle is latched ({reason}); nothing is sent to it")),
                        None => measurement::measure_reopened_rx_throughput(reopened_lease.device(), &reopened_plan),
                    });
                    reopened_guard.latch_if_streamers_open(name);
                    if measurement::record_latch_reason().is_none() && groups.failed().is_empty() {
                        reopened_lease.mark_clean();
                    }
                    drop(reopened_guard);
                    drop(reopened_lease);
                }
                Ok(Err(reason)) => groups.skip_reopened(&reopen_keys, &reason),
                Err(payload) => groups.fail_reopened("provider_reopen", &reopen_keys, measurement::GroupRun::panic_text(payload)),
            }
            (stopped, restored)
        }
        Err(payload) => {
            let reason = measurement::GroupRun::panic_text(payload);
            let skipped = format!("not run: the device was not opened again ({reason})");
            for (name, _, keys) in measurement::provider_groups() {
                let keys: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
                groups.run(name, &keys, || Err(skipped.clone()));
            }
            groups.run("raw_coercion", &measurement::raw_group_keys(&description), || Err(skipped.clone()));
            (Some(skipped), None)
        }
    };
    let failed = groups.failed().to_vec();
    let measured = groups.take();
    let record = measurement::build_record_with_identity(ezsdr_radio::measurement::Source::Hardware, &binding, device_description, serial, &metadata, measured, &description)
        .expect("a hardware measurement record");
    let path = metadata.output.as_deref().expect("the output path is mandatory");
    let violations = measurement::write_record_before_profile_assert(path, &record, &description).expect("a structurally valid measurement record is written before profile comparison");
    groups.remove_partial();
    assert!(failed.is_empty(), "groups failed (their keys are unmeasured in the written record): {failed:?}");
    assert!(stopped.is_none(), "the raw group stopped (its keys are unmeasured in the written record): {stopped:?}");
    assert!(restored.is_none(), "restoring the unstreamed bench settings after the raw group: {restored:?}");
    assert!(violations.is_empty(), "measurement record comparison: {violations:?}");
    println!("Wrote hardware measurement record to {}", path.display());
}

/// The start-lead group's value as a verdict: the measured smallest lead, or the reason no candidate passed.
fn start_lead_verdict(group: &measurement::MeasurementGroup) -> Verdict {
    use ezsdr_radio::measurement::{ValueRecord, Worst};
    match group.values.first() {
        Some(ValueRecord::Measured { worst: Worst::Number(ns), .. }) => n47::start_lead_value_verdict(Some(*ns), None),
        Some(ValueRecord::Measured { .. }) => n47::start_lead_value_verdict(None, Some("the value is not a number")),
        Some(ValueRecord::Unmeasured { reason, .. }) => n47::start_lead_value_verdict(None, Some(reason)),
        None => n47::start_lead_value_verdict(None, Some("the group supplied no value")),
    }
}

/// T3: the restored receive start-lead group on its own (note 47 stage 3's named follow-up before any profile
/// `rx_start_lead_ns` changes). `restart-same` keeps the values and `restart-retune` crosses the board's 1 GHz tuning
/// path on every restart; a stage fails on `radio.LATE_COMMAND` or `radio.START_LEAD_SHORT`, and each stage records
/// the smallest `lead_left_ns` seen.
#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_rx_start_lead_group() {
    let device = usrp();
    pin_slot_a(&*device);
    let plan = measurement::MeasurementPlan::process_env().expect("measurement trial counts are valid before opening the USRP");
    let group = measurement::measure_rx_start_lead(device.clone(), &plan).expect("the receive start-lead group");
    println!("n47 start lead: values {:?}; observations {:?}", group.values, group.observations);
    verdict("n47 start lead: the smallest candidate lead leaves every stage's restart at its origin", start_lead_verdict(&group));
    let companion = measurement::measure_rx_start_lead_with_tx(device, &plan).expect("the receive start-lead group with a transmit stream");
    println!("n47 start lead with tx: observations {:?}", companion.observations);
}

fn tx200_lengths() -> Vec<usize> {
    match std::env::var("EZSDR_TX200_LENGTHS") {
        Ok(text) => text
            .split(',')
            .map(|part| {
                let samples = part.trim().parse::<usize>().unwrap_or_else(|error| panic!("EZSDR_TX200_LENGTHS must be comma-separated complex-sample counts, got {part:?}: {error}"));
                assert!(samples > 0, "EZSDR_TX200_LENGTHS must be positive, got {samples}");
                samples
            })
            .collect(),
        Err(std::env::VarError::NotPresent) => vec![1_000, 2_000, 20_000, 200_000],
        Err(error) => panic!("EZSDR_TX200_LENGTHS is not readable: {error}"),
    }
}

/// The seconds each `hw_tx200_hot_path_probe` trial sends (`EZSDR_TX200_SECONDS`, 30).
fn tx200_seconds() -> u64 {
    let seconds = match std::env::var("EZSDR_TX200_SECONDS") {
        Ok(text) => text.trim().parse::<u64>().unwrap_or_else(|error| panic!("EZSDR_TX200_SECONDS must be whole seconds, got {text:?}: {error}")),
        Err(std::env::VarError::NotPresent) => 30,
        Err(error) => panic!("EZSDR_TX200_SECONDS is not readable: {error}"),
    };
    assert!(seconds > 0, "EZSDR_TX200_SECONDS must be positive, got {seconds}");
    seconds
}

/// One `TX200` line of the probe: what the stage's observation holds of the trial, plus that observation where the line
/// cannot fill a field (a `-`) or where a stream failed (`first_failure` carries the code alone: the tick and channel of a
/// failure are not in the observation).
fn tx200_line(length: usize, shape: &str, rate_hz: u64, result: &measurement::StageResult) -> String {
    let observation = &result.observation;
    let tx: Vec<&Json> = observation["streams"].as_array().into_iter().flatten().filter(|row| row["direction"] == "tx").collect();
    let present = !tx.is_empty();
    let sum = |leaf: &str| tx.iter().map(|row| row[leaf].as_u64()).try_fold(0u64, |total, value| total.checked_add(value?));
    // `burst_blocks` is one count for each burst row: how many device sends the Provider made in it.
    let sends = if present {
        tx.iter()
            .map(|row| match row.get("burst_blocks") {
                Some(Json::Array(blocks)) => blocks.iter().map(Json::as_u64).try_fold(0u64, |total, count| total.checked_add(count?)),
                Some(value) => value.as_u64(),
                None => None,
            })
            .try_fold(0u64, |total, count| total.checked_add(count?))
    } else {
        None
    };
    let samples: Option<Vec<u64>> = if present { tx.iter().map(|row| row["actual_samples"].as_u64()).collect() } else { None };
    let underflows = observation["tx_underflow_events"].as_u64().or_else(|| if present { sum("tx_underflow_events") } else { None });
    let failure = tx.iter().flat_map(|row| row["async_failures"].as_array().into_iter().flatten()).filter_map(Json::as_str).next();
    let line = format!(
        "TX200 len={length} shape={shape} rate={rate_hz} class={:?} underflows={} sends={} samples={} first_failure={}",
        result.class,
        underflows.map_or_else(|| "-".to_owned(), |count| count.to_string()),
        sends.map_or_else(|| "-".to_owned(), |count| count.to_string()),
        samples.as_ref().map_or_else(|| "-".to_owned(), |samples| samples.iter().map(u64::to_string).collect::<Vec<_>>().join(",")),
        failure.unwrap_or("none")
    );
    if failure.is_some() || sends.is_none() || samples.is_none() || underflows.is_none() { format!("{line} json={observation}") } else { line }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_tx200_hot_path_probe() {
    // Diagnostic (stage TX200; each trial's verdict is asserted by `tx200_trial`, and it writes no record): does the length of the repeat
    // waveform remove the 200 MS/s transmit underflow? Each trial runs through the throughput group's own stage runner
    // (`run_stage`: the same Spec and Run construction, the same waveform whose every magnitude is within 0.5), so the only
    // difference between the lengths is the plan's `tx_waveform_samples`. The three shapes are single 200 MS/s, and, only when
    // the selector names two transmit channels, grouped 40 MS/s a channel and split 100 MS/s a channel.
    let selector = std::env::var(measurement::SELECTOR_ENV).ok().map(|text| parse_selector(&text).expect("EZSDR_MEASUREMENT_SELECTOR holds a selector"));
    if let Some(selector) = &selector {
        set_selector_override(Some(selector.clone()));
    }
    let device = usrp();
    let (_, tx) = effective_layout(&*device);
    let two = tx.as_array().map_or(0, Vec::len) >= 2;
    let dir = TempDir::new();
    let doc = bench_profile(&*device, &dir, Json::Object(Default::default()), Json::Object(Default::default()), false);
    let binding = measurement::radio_binding(&doc).expect("the bench profile's radio binding");
    let described = measurement::description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms()).expect("the bound layout");
    let wire_bytes = u64::try_from(described.performance.wire_bytes_per_sample).expect("wire bytes per sample is positive");
    // Only the three shapes the probe compares: single 200 MS/s, and with two transmit channels grouped 40 MS/s a channel
    // and split 100 MS/s a channel. The plan carries only their rates (the whole sweep is the record's, not this probe's).
    let mut stages: Vec<(&str, measurement::Stage)> =
        vec![("single", measurement::Stage { id: "tx200-single".to_owned(), family: "tx200", rx: Vec::new(), tx: vec![measurement::StreamPlan { channels: vec![0], rate_hz: 200_000_000 }] })];
    if two {
        stages
            .push(("grouped", measurement::Stage { id: "tx200-grouped".to_owned(), family: "tx200", rx: Vec::new(), tx: vec![measurement::StreamPlan { channels: vec![0, 1], rate_hz: 40_000_000 }] }));
        stages.push((
            "split",
            measurement::Stage {
                id: "tx200-split".to_owned(),
                family: "tx200",
                rx: Vec::new(),
                tx: vec![measurement::StreamPlan { channels: vec![0], rate_hz: 100_000_000 }, measurement::StreamPlan { channels: vec![1], rate_hz: 100_000_000 }],
            },
        ));
    }
    let lengths = tx200_lengths();
    let mut rates: Vec<u64> = stages.iter().flat_map(|(_, stage)| stage.tx.iter().map(|stream| stream.rate_hz)).collect();
    rates.sort_unstable();
    rates.dedup();
    let mut plan = measurement::MeasurementPlan { throughput_seconds: tx200_seconds(), tx_throughput_rates_hz: rates, ..measurement::MeasurementPlan::default() };
    println!("TX200 probe: {} transmit channels, lengths {lengths:?}, {} s a trial, {} trials", tx.as_array().map_or(0, Vec::len), plan.throughput_seconds, lengths.len() * stages.len());
    let mut ran = 0;
    let mut failed = Vec::new();
    for length in &lengths {
        plan.tx_waveform_samples = *length;
        for (shape, stage) in &stages {
            let rate_hz = stage.tx.first().map_or(0, |stream| stream.rate_hz);
            match measurement::run_stage(device.clone(), &plan, stage, wire_bytes) {
                Ok(result) => {
                    println!("{}", tx200_line(*length, shape, rate_hz, &result));
                    if let Err(why) = tx200_trial(&result) {
                        failed.push(format!("len={length} shape={shape}: {why}"));
                    }
                }
                Err(error) => {
                    println!("TX200 len={length} shape={shape} rate={rate_hz} class=Error error={error}");
                    failed.push(format!("len={length} shape={shape}: {error}"));
                }
            }
            ran += 1;
        }
    }
    assert_eq!(ran, lengths.len() * stages.len(), "every TX200 trial ran");
    println!("TX200 verdict: {} of {ran} trials succeeded", ran - failed.len());
    assert!(failed.is_empty(), "TX200 trials that did not succeed: {failed:?}");
}

/// Whether one `hw_tx200_hot_path_probe` trial succeeded: class Success (the stage judges its burst and its acknowledgement),
/// transmit samples accepted, no async failure and no transmit underflow.
fn tx200_trial(result: &measurement::StageResult) -> Result<(), String> {
    if result.class != measurement::TrialClass::Success {
        return Err(format!("class {:?}", result.class));
    }
    let tx: Vec<&Json> = result.observation["streams"].as_array().into_iter().flatten().filter(|row| row["direction"] == "tx").collect();
    if tx.is_empty() || !tx.iter().all(|row| row["actual_samples"].as_u64().is_some_and(|n| n > 0)) {
        return Err("no accepted transmit samples in the observation".to_owned());
    }
    if let Some(failure) = tx.iter().flat_map(|row| row["async_failures"].as_array().into_iter().flatten()).next() {
        return Err(format!("an async failure: {failure}"));
    }
    let underflows = result.observation["tx_underflow_events"].as_u64().unwrap_or_else(|| tx.iter().filter_map(|row| row["tx_underflow_events"].as_u64()).sum());
    if underflows != 0 {
        return Err(format!("{underflows} transmit underflow(s)"));
    }
    Ok(())
}

// ---------------------------------------------------------------- bench 2026-11-01: production-path tests (#70, #72, P4, L1)
//
// Each test prints one VERDICT line per claim. FAIL is evidence against the claim; a missing precondition is INCONCLUSIVE
// and names it. The transmitter is slot A's OBX or UBX, inside `SAFE_TX_HZ`, at amplitude 0.4 (bench.md's 0.5 bound).

/// Prints the verdict line of `claim` and fails the test on FAIL.
fn verdict(claim: &str, outcome: Verdict) {
    let (word, detail) = match &outcome {
        Verdict::Pass(detail) => ("PASS", detail),
        Verdict::Inconclusive(detail) => ("INCONCLUSIVE", detail),
        Verdict::Fail(detail) => ("FAIL", detail),
    };
    println!("VERDICT {claim}: {word}: {detail}");
    if let Verdict::Fail(detail) = outcome {
        panic!("{claim}: {detail}");
    }
}

/// A tone `offset_hz` from the carrier at `rate` S/s, amplitude 0.4, `n` samples; its magnitude bound is asserted.
fn tone_wave(rate: f64, n: usize, offset_hz: f64) -> Vec<(f32, f32)> {
    let wave: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * offset_hz * (i as f64 / rate);
            ((0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32)
        })
        .collect();
    assert!(wave.iter().all(|(re, im)| re.hypot(*im) <= 0.5), "bench RF safety: a transmit waveform's magnitude is at most 0.5");
    wave
}

/// The bench frequency of slot A, refusing a CBX (its band starts at 1.2 GHz; the bench's attenuator is the OBX's and UBX's)
/// and any frequency outside `SAFE_TX_HZ`.
fn pin_slot_a(device: &dyn Device) -> f64 {
    let board = board_of(device);
    assert!(matches!(board, ezsdr_radio::x3xx::Board::Ubx | ezsdr_radio::x3xx::Board::Obx), "slot A holds a {}: these tests transmit on an OBX or UBX only", board.name());
    let hz = bench_hz(device);
    assert!(SAFE_TX_HZ.contains(&hz), "bench RF safety: {hz} Hz is outside SAFE_TX_HZ");
    hz
}

/// The samples of a capture with their root ticks, `per` root ticks apart, from `first`.
fn ticked(samples: &[(f32, f32)], first: i64, per: i64) -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
    samples.iter().enumerate().map(|(i, (re, im))| (first + i as i64 * per, [*re, *im])).collect()
}

/// Every sample of a probe's device blocks with its root tick (200 ticks apart at 1 Msps).
fn device_samples(blocks: &[(i64, Vec<ezsdr_radio_uhd::Iq>)]) -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
    blocks.iter().flat_map(|(first, samples)| samples.iter().enumerate().map(move |(i, sample)| (first + i as i64 * 200, *sample))).collect()
}

/// The root tick of a transmit instant on the running transmit clock.
fn transmit_root(run: &ezsdr_kernel::coordinator::RunHandle, at: &TimePoint) -> i64 {
    let clock = run.sample_clocks().into_iter().rev().find(|r| r.stream.to_string().ends_with("usrp/tx/s0") && r.ended_at.is_none()).unwrap();
    clock.origin.ticks_in(clock.root).unwrap() + at.ticks_in(clock.domain).unwrap() * clock.root_ticks_per_tick.num() as i64
}

/// T3's evidence in a Manifest: how many flushes were acknowledged (the async section), and the flushes refused.
fn flush_evidence(manifest: &ezsdr_kernel::manifest::Manifest) -> (usize, Vec<Json>) {
    let acked = section(manifest, "async").as_array().unwrap().iter().filter(|row| row["tx_flush"] == true && row["code"] == "BurstAck").count();
    let refused = section(manifest, "rejected").as_array().unwrap().iter().filter(|row| row["call"] == "tx_flush").cloned().collect();
    (acked, refused)
}

/// The transmit sends of a probe, flushes left out. Zero-length sends stay in: a closing one is judged by its count (T2).
fn data_sends(probe: &probe_device::ProbeDevice) -> Vec<Json> {
    probe.rows().into_iter().filter(|row| row["call"] == "tx_send" && row["tx_flush"] != true).collect()
}

/// One trial of `hw_p70_…`: the tone's onset (root ticks, `None` when there is no tone) and the burst's booked instant.
struct OneSample {
    onset: Option<i64>,
    burst: i64,
    manifest: ezsdr_kernel::manifest::Manifest,
}

/// One trial of `hw_p70_…`: a Session sends `samples` samples at `a` (one sample, or the two-sample control), then a 10 000-sample
/// tone at `a` + 5 ms, booked with the first (`late` false) or after `a` was sent. The capture starts 30 ms before the tone.
fn one_sample_trial(device: Arc<dyn Device>, samples: usize, late: bool) -> OneSample {
    let dir = TempDir::new();
    let mut run = tx_session(device, &dir);
    let (first, _) = waveform_of(&[(0.4f32, 0.0f32); 4][..samples]);
    let (tone, _) = waveform_of(&tone_wave(1e6, 10_000, 100e3));
    let a = tx_at(&run, ms(40));
    let b = TimePoint::new(a.domain(), a.ticks_in(a.domain()).unwrap() + 5_000);
    let capture_at = after_ticks(&run, ms(10));
    assert!(admitted(&run.submit(verb("capture", output("rec"), Some(capture_at), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(80_000))]), None).unwrap()));
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(a), &[]), Some(&first)).unwrap()));
    if late {
        wait(&mut run, ms(41));
    }
    assert!(admitted(&run.submit(verb("send", resource("radio/tx/s0"), Some(b), &[]), Some(&tone)).unwrap()));
    let burst = transmit_root(&run, &b);
    let horizon = after_ticks(&run, ms(3_000));
    let _ = run.wait_for(&[kind("sink.CAPTURE_WRITTEN")], 0, horizon);
    let manifest = run.finish();
    let (origin, per) = rx_clock(&manifest);
    let artifact = capture_of(&manifest, "rec");
    let first_tick = origin + artifact.continuity[0].first.ticks_in(artifact.continuity[0].domain).unwrap() * per;
    let blocks = ticked(&read_capture(&artifact, 1).remove(0), first_tick, per);
    let onset = window_power(&blocks, first_tick, burst - ms(2)).and_then(|floor| tone_start(&blocks, burst - ms(5), floor));
    OneSample { onset, burst, manifest }
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_p72_timed_tx_chunks_are_timed_and_flushed() {
    // #72 on the production path (D9b-1a): every send of a 60 ms device burst carries its time (T1), every closing send has at least
    // two samples, zero-length sends included (T2), and the flush is acknowledged (T3). Raw P6 runs on both boards separately.
    let device = usrp();
    let hz = pin_slot_a(&*device);
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    let (bytes, waveform) = waveform_of(&tone_wave(1e6, 60_000, 100e3));
    let spec = with_burst(with_tx(receive_spec(1, 1e6, hz, Some(90_000)), 1e6), &waveform, false, "send_asap_and_flag", 10_000);
    let dir = TempDir::new();
    let inputs = std::collections::BTreeMap::from([(waveform.hash.clone(), bytes)]);
    let manifest = captured(spec_run(&spec, &bench_profile(&*probe, &dir, serde_json::json!({}), bench_envelope(&*probe), false), probe.clone(), inputs));
    let sends = data_sends(&probe);
    let payload = sends.iter().filter(|row| row["sample_counts"][0].as_u64().unwrap_or(0) > 0).count();
    let untimed: Vec<&Json> = sends.iter().filter(|row| row["at"].is_null()).collect();
    let closing: Vec<Json> = sends.iter().filter(|row| row["eob"] == true).map(|row| row["sample_counts"].clone()).collect();
    let short: Vec<&Json> = sends.iter().filter(|row| row["eob"] == true && row["sample_counts"].as_array().is_some_and(|counts| counts.iter().any(|n| n.as_u64().unwrap_or(0) < 2))).collect();
    let (acked, refused) = flush_evidence(&manifest);
    let flushes = probe.rows().iter().filter(|row| row["call"] == "tx_send" && row["tx_flush"] == true).count();
    println!("#72 1a: {payload} sends with samples; closing sends {closing:?}; flushes sent {flushes}, acknowledged {acked}, refused {refused:?}");
    let outcome = if !refused.is_empty() {
        Verdict::Fail(format!("T3: a flush was refused: {refused:?}"))
    } else if !untimed.is_empty() {
        Verdict::Fail(format!("T1: {} send(s) carry no time: {untimed:?}", untimed.len()))
    } else if !short.is_empty() {
        Verdict::Fail(format!("T2: a closing send has fewer than two samples: {short:?}"))
    } else if payload < 2 {
        Verdict::Inconclusive(format!("only {payload} send(s) carried samples: no multi-chunk evidence"))
    } else if closing.is_empty() {
        Verdict::Inconclusive("no closing send was recorded: T2 not shown".to_owned())
    } else if flushes == 0 {
        Verdict::Inconclusive("no flush was sent: T3 not shown".to_owned())
    } else if acked < flushes {
        Verdict::Inconclusive(format!("{acked} of {flushes} flushes acknowledged: T3 not shown"))
    } else {
        Verdict::Pass(format!("{payload} sends, each timed; {} closing send(s) with two or more samples; {acked} flush(es) acknowledged", closing.len()))
    };
    verdict("#72 1a (T1, T2, T3 on the production path): every send is timed, closing sends have two samples, the flush is acknowledged", outcome);
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_p70_one_sample_burst_then_timed_burst() {
    // #70 on the production path (D9b-1a T2): a one-sample burst must leave the next timed burst's onset where a two-sample control
    // leaves it, for each booking order. The onset is compared, not the absolute time (the loopback adds about 44 samples).
    let device = usrp();
    pin_slot_a(&*device);
    for late in [false, true] {
        let order = if late { "the second booked after the first was sent" } else { "the second booked with the first" };
        let one = one_sample_trial(device.clone(), 1, late);
        let control = one_sample_trial(device.clone(), 4, late);
        for (case, trial) in [("one-sample", &one), ("four-sample control", &control)] {
            let (acked, refused) = flush_evidence(&trial.manifest);
            println!(
                "#70 {case}, {order}: onset {:?} samples from the booked instant; continuity gaps {}; TIME_ERROR {}; TX_UNDERFLOW {}; flushes acknowledged {acked}, refused {refused:?}",
                trial.onset.map(|t| (t - trial.burst) as f64 / 200.0),
                capture_of(&trial.manifest, "rec").continuity[0].gaps.len(),
                events_of(&trial.manifest, "radio.TIME_ERROR").len(),
                events_of(&trial.manifest, "radio.TX_UNDERFLOW").len()
            );
        }
        let events = events_of(&one.manifest, "radio.TIME_ERROR").len() + events_of(&one.manifest, "radio.TX_UNDERFLOW").len();
        let gaps = [&one.manifest, &control.manifest].iter().map(|m| capture_of(m, "rec").continuity[0].gaps.len()).sum::<usize>();
        let outcome = match (one.onset, control.onset) {
            _ if events > 0 => Verdict::Fail(format!("the one-sample trial reported {events} TIME_ERROR or TX_UNDERFLOW event(s)")),
            _ if gaps > 0 => Verdict::Inconclusive(format!("the receive timestamps have {gaps} continuity gap(s): the onset is not comparable")),
            (Some(a), Some(b)) => {
                let diff = ((a - one.burst) - (b - control.burst)) / 200;
                if diff.abs() <= 50 {
                    Verdict::Pass(format!("the onset differs from the control's by {diff} samples"))
                } else {
                    Verdict::Fail(format!("the one-sample burst moved the next burst's onset by {diff} samples"))
                }
            }
            _ => Verdict::Inconclusive("no tone onset in one of the trials: the positive signal is missing".to_owned()),
        };
        verdict(&format!("#70 (T2): a one-sample burst leaves the next timed burst's onset as a four-sample control's ({order})"), outcome);
    }
}

/// Aligns `tick` up to `per` ticks.
fn align_to(tick: i64, per: i64) -> i64 {
    tick.div_euclid(per) * per + if tick.rem_euclid(per) == 0 { 0 } else { per }
}

/// Sends `wave` as one device burst from `first_at`, in chunks, handling partial sends. The last two samples are kept
/// for the closing send, which always carries the EOB with at least two samples (a one-sample tail is padded with a
/// zero, as the Module's own end-of-burst path does). Returns the waveform samples accepted.
fn send_burst(device: &dyn Device, wave: &[ezsdr_radio_uhd::Iq], first_at: Option<i64>) -> usize {
    let mut accepted = 0usize;
    let mut at = first_at;
    while let Some(plan) = n47::burst_send(accepted, wave.len(), 20_000) {
        let padded = [wave[plan.offset], ezsdr_radio_uhd::Iq::default()];
        let slice: &[ezsdr_radio_uhd::Iq] = if plan.padded { &padded } else { &wave[plan.offset..plan.offset + plan.len] };
        let sent = device.tx_send(0, &[slice], at, accepted == 0, plan.eob, Wall::from_millis(500)).expect("the device accepts the burst");
        assert!(sent > 0, "the device accepted no samples of the burst");
        accepted += sent.min(plan.len - usize::from(plan.padded));
        at = None;
    }
    accepted
}

/// Drains the transmit reports that are already there, `None` when none.
fn drain_tx_reports(device: &dyn Device) -> Vec<ezsdr_radio_uhd::TxReport> {
    let mut reports = Vec::new();
    while let Some(report) = device.tx_async(0, Wall::from_millis(10)).expect("a transmit report") {
        reports.push(report);
    }
    reports
}

/// Waits for the next burst ACK (or `timeout`) and returns it with every report read on the way.
fn wait_burst_ack(device: &dyn Device, timeout: Wall) -> (Option<i64>, Vec<ezsdr_radio_uhd::TxReport>) {
    let begun = Instant::now();
    let mut reports = Vec::new();
    while begun.elapsed() < timeout {
        if let Some(report) = device.tx_async(0, Wall::from_millis(20)).expect("a transmit report") {
            let ack = (report.code == ezsdr_radio_uhd::TxCode::BurstAck).then_some(report.tick).flatten();
            reports.push(report);
            if ack.is_some() {
                return (ack, reports);
            }
        }
    }
    (None, reports)
}

/// The two-zero-sample untimed flush of a new transmit streamer, and the reports its ACK and a first control leave.
fn flush_transmit(device: &dyn Device) -> Vec<ezsdr_radio_uhd::TxReport> {
    let zeros = [ezsdr_radio_uhd::Iq::default(); 2];
    device.tx_send(0, &[&zeros], None, true, true, Wall::from_millis(500)).expect("the flush is sent");
    let (ack, _) = wait_burst_ack(device, Wall::from_secs(2));
    assert!(ack.is_some(), "the transmit flush's burst ACK");
    drain_tx_reports(device)
}

/// A call's durations under its statistics, as the control-call observation keeps them.
fn stats_row(name: &str, durations: &[u64]) -> Json {
    let stats = n47::call_stats(durations).expect("nonempty durations");
    serde_json::json!({ "call": name, "min_ns": stats.min_ns, "median_ns": stats.median_ns, "p99_ns": stats.p99_ns, "max_ns": stats.max_ns, "durations_ns": durations })
}

/// Times `reps` calls, returning each call's wall-clock nanoseconds.
fn timed_calls(reps: usize, mut call: impl FnMut(usize)) -> Vec<u64> {
    (0..reps)
        .map(|index| {
            let begun = Instant::now();
            call(index);
            begun.elapsed().as_nanos() as u64
        })
        .collect()
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_raw_control_call_durations() {
    // Note 47 T6: a control call's wall clock is its setter plus read-back/enqueue latency, not RF settling or timed
    // execution. Only a timed receive frequency change is timed: receive samples cover its instant. A timed transmit
    // change is not measured at all: a burst's untimed continuations do not drain the DUC (design note 39 §1 and raw
    // P7), so the timed commands would sit in the FIFO and could wedge the X300; queuing them is not a measurement
    // this probe may make. The antenna timing runs with both streams stopped and no transmit samples; the transmit
    // gain and rate timings run with no burst open.
    let device = raw();
    let hz = pin_slot_a(&*device);
    let chan = d9_channel();
    let mut rows = Vec::new();
    let reps = 50;
    let rx_small = timed_calls(reps, |index| {
        let freq = hz + if index % 2 == 0 { 1_000.0 } else { 2_000.0 };
        device.apply(Dir::Rx, chan, &Settings { freq: Some(freq), ..Settings::default() }, None).unwrap();
    });
    rows.push(stats_row("rx_apply_frequency_small_step", &rx_small));
    let rx_boundary = timed_calls(reps, |index| {
        let freq = if index % 2 == 0 { hz - 400e3 } else { hz + 400e3 };
        device.apply(Dir::Rx, chan, &Settings { freq: Some(freq), ..Settings::default() }, None).unwrap();
    });
    rows.push(stats_row("rx_apply_frequency_across_1_ghz", &rx_boundary));
    let tx_small = timed_calls(reps, |index| {
        let freq = hz + if index % 2 == 0 { 1_000.0 } else { 2_000.0 };
        device.apply(Dir::Tx, chan, &Settings { freq: Some(freq), ..Settings::default() }, None).unwrap();
    });
    rows.push(stats_row("tx_apply_frequency_small_step", &tx_small));
    let tx_boundary = timed_calls(reps, |index| {
        let freq = if index % 2 == 0 { hz - 400e3 } else { hz + 400e3 };
        assert!(SAFE_TX_HZ.contains(&freq), "bench RF safety: {freq} Hz is outside SAFE_TX_HZ");
        device.apply(Dir::Tx, chan, &Settings { freq: Some(freq), ..Settings::default() }, None).unwrap();
    });
    rows.push(stats_row("tx_apply_frequency_across_1_ghz", &tx_boundary));
    let rx_gain = timed_calls(reps, |index| {
        device.apply(Dir::Rx, chan, &Settings { gain: Some(if index % 2 == 0 { 0.0 } else { 1.0 }), ..Settings::default() }, None).unwrap();
    });
    rows.push(stats_row("rx_apply_gain", &rx_gain));
    // No burst is open and no samples flow: the transmit gain is the setter plus its read-back only.
    let tx_gain = timed_calls(reps, |index| {
        device.apply(Dir::Tx, chan, &Settings { gain: Some(if index % 2 == 0 { 0.0 } else { 1.0 }), ..Settings::default() }, None).unwrap();
    });
    // Back to the probes' initial state before anything else runs (the alternation may end at +1 dB).
    device.apply(Dir::Tx, chan, &Settings { gain: Some(0.0), ..Settings::default() }, None).unwrap();
    rows.push(stats_row("tx_apply_gain", &tx_gain));
    let antenna = timed_calls(reps, |index| {
        let antenna = if index % 2 == 0 { "TX/RX" } else { "RX2" };
        let applied = device.apply(Dir::Rx, chan, &Settings { antenna: Some(antenna.to_owned()), ..Settings::default() }, None).unwrap();
        assert_eq!(applied.antenna.as_deref(), Some(antenna), "the antenna's read-back");
    });
    device.apply(Dir::Rx, chan, &Settings { antenna: Some("RX2".to_owned()), ..Settings::default() }, None).unwrap();
    rows.push(stats_row("rx_apply_antenna", &antenna));
    let rates = timed_calls(reps, |index| {
        device.apply(Dir::Rx, chan, &Settings { rate: Some(if index % 2 == 0 { 2e6 } else { 1e6 }), ..Settings::default() }, None).unwrap();
    });
    device.apply(Dir::Rx, chan, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
    rows.push(stats_row("rx_apply_rate", &rates));
    let tx_rates = timed_calls(reps, |index| {
        device.apply(Dir::Tx, chan, &Settings { rate: Some(if index % 2 == 0 { 2e6 } else { 1e6 }), ..Settings::default() }, None).unwrap();
    });
    device.apply(Dir::Tx, chan, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
    rows.push(stats_row("tx_apply_rate", &tx_rates));
    let clocks = timed_calls(reps, |_| {
        device.time_now().unwrap();
    });
    rows.push(stats_row("time_now", &clocks));
    let opens = timed_calls(reps, |_| {
        device.rx_open(0, &[chan]).unwrap();
    });
    device.rx_close(0);
    rows.push(stats_row("rx_open", &opens));
    let tx_opens = timed_calls(reps, |_| {
        device.tx_open(0, &[chan]).unwrap();
    });
    device.tx_close(0);
    rows.push(stats_row("tx_open", &tx_opens));

    // A timed receive frequency change, with a receive stream covering the instant in the same direction.
    raw_rx(&*device, 1e6, hz);
    let start = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    device.rx_start(0, start).unwrap();
    let mut timed_rx = Vec::new();
    let mut lost = None;
    for index in 0..reps {
        let at = align_to(device.time_now().unwrap() + 100 * TICKS_PER_MS, 200);
        let begun = Instant::now();
        let result = device.apply(Dir::Rx, chan, &Settings { freq: Some(hz + if index % 2 == 0 { 1_000.0 } else { 2_000.0 }), ..Settings::default() }, Some(at));
        timed_rx.push(begun.elapsed().as_nanos() as u64);
        if let Err(error) = result {
            lost = Some(format!("the timed receive apply failed: {}", error.message));
            break;
        }
        let got = read(&*device, Wall::from_secs(2), None, |r| r.end(200).is_some_and(|end| end >= at + 5 * TICKS_PER_MS) || r.other.iter().any(|(_, other)| other.starts_with("LateCommand")));
        if got.other.iter().any(|(_, other)| other.starts_with("LateCommand")) {
            lost = Some(format!("the timed receive apply at {at} was late: coverage was lost"));
            break;
        }
        if got.end(200).is_none_or(|end| end < at + 5 * TICKS_PER_MS) {
            lost = Some(format!("the receive stream did not cover the timed apply at {at}: no samples past its instant"));
            break;
        }
    }
    device.rx_stop(0, None).ok();
    if let Some(reason) = lost {
        panic!(
            "n47 control calls: the timed receive apply lost coverage ({reason}); a timed command may still sit in the device's FIFO. Power-cycle the device before the next test; this probe issues no further apply and does not reopen the receive stream."
        );
    }
    let _ = drain(&*device);
    rows.push(stats_row("rx_apply_frequency_timed_with_samples", &timed_rx));
    raw_rx(&*device, 1e6, hz);
    println!(
        "n47 control calls: wall clock per call is setter + read-back/enqueue latency, not RF settling or timed execution; a timed transmit change is not measured (note 39 §1: untimed continuations do not drain the DUC)"
    );
    for row in &rows {
        println!("n47 control calls {}: min {} median {} p99 {} max {} ns", row["call"], row["min_ns"], row["median_ns"], row["p99_ns"], row["max_ns"]);
    }
    measurement::push_raw_observation(
        "hw_n47_raw_control_call_durations",
        (rows.len() * reps) as u32,
        serde_json::json!({
            "note": "wall clock per call is the setter plus read-back/enqueue latency, not RF settling or timed execution",
            "calls": rows,
        }),
    );
}

/// One buffer stall: what the drain showed (an overflow, no overflow with samples spanning the stall, or a trial
/// that says nothing), the drained samples' first and end ticks, and the outcomes seen.
fn buffer_stall(device: &dyn Device, stall_ms: u64, stall_start: i64, per: i64) -> (n47::StallOutcome, Option<(i64, i64)>, Vec<String>) {
    std::thread::sleep(Wall::from_millis(stall_ms));
    let begun = Instant::now();
    let limit = Wall::from_millis(stall_ms.saturating_add(3_000));
    let mut overflow = false;
    let mut idle = 0;
    let mut first: Option<i64> = None;
    let mut end: Option<i64> = None;
    let mut outcomes = Vec::new();
    while begun.elapsed() < limit && idle < 3 {
        match device.rx_recv(0, 2_000, Wall::from_millis(100), false) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                idle = 0;
                first.get_or_insert(first_tick);
                end = Some(first_tick + samples.first().map_or(0, Vec::len) as i64 * per);
                outcomes.push(format!("samples {first_tick} {}", samples.first().map_or(0, Vec::len)));
            }
            ezsdr_radio_uhd::RxRecv::Overflow { out_of_sequence } => {
                idle = 0;
                overflow = true;
                outcomes.push(format!("overflow {out_of_sequence}"));
            }
            ezsdr_radio_uhd::RxRecv::Timeout => idle += 1,
            ezsdr_radio_uhd::RxRecv::LateCommand => outcomes.push("late_command".to_owned()),
            other => outcomes.push(format!("{other:?}")),
        }
    }
    let outcome = n47::stall_outcome(overflow, first, end, stall_start, stall_ms, TICKS_PER_MS, 5 * TICKS_PER_MS);
    (outcome, first.zip(end), outcomes)
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_raw_rx_buffer_capacity() {
    // Note 47 T6: how long a receive reader may stall before the first overflow. Every trial proves its stream first
    // (the restart ran from its tick through 20 ms) and, when no overflow is reported, that the drain's samples span
    // the stall; a trial that proves neither is recorded incomplete and never used in the bisection. Samples are
    // counted and dropped, never kept.
    let device = raw();
    let hz = pin_slot_a(&*device);
    let rmem_max = std::fs::read_to_string("/proc/sys/net/core/rmem_max").unwrap_or_else(|_| "unknown".to_owned()).trim().to_owned();
    let rmem_default = std::fs::read_to_string("/proc/sys/net/core/rmem_default").unwrap_or_else(|_| "unknown".to_owned()).trim().to_owned();
    println!(
        "n47 buffer: net.core.rmem_max {rmem_max}, rmem_default {rmem_default}; device args {:?}; requested recv_buff_size: default (the args name none); actual socket size unknown (the Device trait does not read SO_RCVBUF; UHD's X300 default is about link_rate/50, 25 MB at 10 GbE, INFERRED from x300_eth_mgr.cpp:250-251)",
        args()
    );
    let mut observations = Vec::new();
    for rate in [1e6, 10e6, 50e6] {
        raw_rx(&*device, rate, hz);
        let per = (common::MCR as f64 / rate) as i64;
        let start = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, per);
        device.rx_start(0, start).unwrap();
        let baseline = read(&*device, Wall::from_secs(3), None, |r| r.end(per).is_some_and(|end| end >= start + 20 * TICKS_PER_MS));
        if !n47::restart_streamed(&baseline.blocks, per, start, start + 20 * TICKS_PER_MS) {
            println!("n47 buffer {rate}: the baseline did not stream from its tick through 20 ms ({} blocks); this rate is not measured", baseline.blocks.len());
            device.rx_stop(0, None).ok();
            let _ = drain(&*device);
            observations.push(serde_json::json!({ "rate_hz": rate, "baseline": "failed", "blocks": baseline.blocks.len() }));
            continue;
        }
        let mut trials: Vec<(u64, bool)> = Vec::new();
        let mut rows = Vec::new();
        let stop = {
            let mut test = |stall_ms: u64| -> Option<bool> {
                device.rx_stop(0, None).ok();
                let _ = drain(&*device);
                let start = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, per);
                device.rx_start(0, start).unwrap();
                let before = read(&*device, Wall::from_secs(2), None, |r| r.end(per).is_some_and(|end| end >= start + 20 * TICKS_PER_MS));
                if !n47::restart_streamed(&before.blocks, per, start, start + 20 * TICKS_PER_MS) {
                    rows.push(serde_json::json!({ "stall_ms": stall_ms, "incomplete": "the trial's restart did not stream from its tick through 20 ms", "blocks": before.blocks }));
                    return None;
                }
                let stall_start = before.end(per).unwrap_or(start);
                let (outcome, sample_span, outcomes) = buffer_stall(&*device, stall_ms, stall_start, per);
                match outcome {
                    n47::StallOutcome::Overflow => {
                        trials.push((stall_ms, true));
                        rows.push(
                            serde_json::json!({ "stall_ms": stall_ms, "overflow": true, "stall_start": stall_start, "samples": sample_span, "outcomes": outcomes.iter().take(20).collect::<Vec<_>>() }),
                        );
                        Some(true)
                    }
                    n47::StallOutcome::NoOverflow => {
                        trials.push((stall_ms, false));
                        rows.push(
                        serde_json::json!({ "stall_ms": stall_ms, "overflow": false, "stall_start": stall_start, "samples": sample_span, "outcomes": outcomes.iter().take(20).collect::<Vec<_>>() }),
                    );
                        Some(false)
                    }
                    n47::StallOutcome::Incomplete => {
                        rows.push(serde_json::json!({ "stall_ms": stall_ms, "incomplete": "the drain did not span the stall", "stall_start": stall_start, "samples": sample_span, "outcomes": outcomes.iter().take(20).collect::<Vec<_>>() }));
                        None
                    }
                }
            };
            let low = test(100);
            let mut stop = low.is_none();
            if low == Some(false) {
                let high = test(5_000);
                stop = high.is_none();
                if high == Some(true) {
                    let (mut ok, mut over) = (100u64, 5_000u64);
                    let mut steps = 0;
                    while over - ok > 1 && steps < 6 {
                        let mid = ok + (over - ok) / 2;
                        match test(mid) {
                            Some(true) => over = mid,
                            Some(false) => ok = mid,
                            None => {
                                stop = true;
                                break;
                            }
                        }
                        steps += 1;
                    }
                }
            }
            stop
        };
        device.rx_stop(0, None).ok();
        let _ = drain(&*device);
        let bracket = n47::overflow_bracket(&trials);
        if stop {
            println!("n47 buffer {rate}: a trial was incomplete; the bracket {bracket:?} uses only the complete trials {trials:?}");
        }
        println!("n47 buffer {rate}: baseline contiguous over 20 ms; bracket {bracket:?}; trials {trials:?}");
        observations.push(serde_json::json!({ "rate_hz": rate, "baseline": "contiguous", "bracket": format!("{bracket:?}"), "incomplete": stop, "trials": rows,
            "rmem_max": rmem_max, "rmem_default": rmem_default }));
        if rate == 50e6 {
            println!("n47 buffer 50 MS/s: the 2026-10-28 record sustains 800 MB/s at 200 MS/s (plan/phase8/bench-results.md:167); this baseline is the current board's check");
        }
    }
    measurement::push_raw_observation("hw_n47_raw_rx_buffer_capacity", 3, serde_json::json!({ "rates": observations }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_raw_tx_setting_after_a_burst_end() {
    // Note 47 T5: an untimed transmit setting m ms after the previous burst's end must leave that burst's tail intact;
    // the +5 ms gain trial changes 0 dB to +6 dB under the owner's attenuator confirmation (the owner allows the
    // transmit gain up to its maximum with the attenuator in the loopback, 2026-10-27). m = -5 ms is the
    // owner-approved disruptive control (EZSDR_N47_T5_CONTROL=1) and is never a production operation.
    let device = raw();
    let hz = pin_slot_a(&*device);
    let f2 = hz + 100e3;
    assert!(SAFE_TX_HZ.contains(&f2), "bench RF safety: {f2} Hz is outside SAFE_TX_HZ");
    raw_tx(&*device, 1e6, hz);
    flush_transmit(&*device);
    raw_rx(&*device, 1e6, hz);
    // The control burst measures the loopback's delay for this (1 MS/s, 1 MS/s) pair; B is judged against it.
    let control_at = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    device.rx_start(0, control_at - 20 * TICKS_PER_MS).unwrap();
    send_burst(&*device, &iq(&tone_wave(1e6, 5_000, 100e3)), Some(control_at));
    let (control_ack, _) = wait_burst_ack(&*device, Wall::from_secs(2));
    assert!(control_ack.is_some(), "the control burst's ACK");
    let control = read(&*device, Wall::from_secs(2), Some(200), |r| r.end(200).is_some_and(|end| end >= control_at + 10 * TICKS_PER_MS));
    device.rx_stop(0, None).unwrap();
    let _ = drain(&*device);
    let control_samples = control.samples;
    let control_onset =
        window_power(&control_samples, control_at - 20 * TICKS_PER_MS, control_at - 10 * TICKS_PER_MS).and_then(|floor| tone_start(&control_samples, control_at - 5 * TICKS_PER_MS, floor));
    let loop_delay = control_onset.map(|onset| onset - control_at);
    let group_start = Instant::now();
    let mut rows = Vec::new();
    let mut offsets: Vec<i64> = vec![0, 1, 5, 20];
    if std::env::var("EZSDR_N47_T5_CONTROL").ok().as_deref() == Some("1") {
        offsets.push(-5);
    }
    let mut trials: Vec<(i64, bool)> = offsets.into_iter().map(|m| (m, false)).collect();
    trials.push((5, true));
    for (m, gain_trial) in trials {
        if group_start.elapsed() > Wall::from_secs(120) {
            println!("n47 T5: the probe's time bound passed before m = {m} ms; stopping rather than queueing more settings");
            break;
        }
        // Restore the initial state between trials.
        device.apply(Dir::Tx, d9_channel(), &Settings { freq: Some(hz), gain: Some(0.0), ..Settings::default() }, None).unwrap();
        let a = align_to(device.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
        device.rx_start(0, a - 20 * TICKS_PER_MS).unwrap();
        let wave_a = iq(&tone_wave(1e6, 100_000, 100e3));
        send_burst(&*device, &wave_a, Some(a));
        let e = a + wave_a.len() as i64 * 200;
        // The apply is issued before A's burst ACK is awaited: the −5 ms control must be able to go out before
        // A ends, and its ACK is collected afterwards like the others.
        let target = e + m * TICKS_PER_MS;
        let begun = Instant::now();
        while device.time_now().unwrap() < target && begun.elapsed() < Wall::from_secs(2) {
            std::thread::sleep(Wall::from_micros(100));
        }
        let issued = device.time_now().unwrap();
        let applied = if gain_trial {
            device.apply(Dir::Tx, d9_channel(), &Settings { gain: Some(6.0), ..Settings::default() }, None).unwrap()
        } else {
            device.apply(Dir::Tx, d9_channel(), &Settings { freq: Some(f2), ..Settings::default() }, None).unwrap()
        };
        let returned = device.time_now().unwrap();
        if gain_trial {
            assert!((applied.gain - 6.0).abs() < 1e-9, "the gain's read-back: {applied:?}");
        } else {
            assert!((applied.freq - f2).abs() < 1.0, "the frequency's read-back: {applied:?}");
        }
        let (a_ack, _) = wait_burst_ack(&*device, Wall::from_secs(3));
        let b = align_to((e + m * TICKS_PER_MS + 50 * TICKS_PER_MS).max(returned + 52 * TICKS_PER_MS), 200);
        let wave_b = iq(&tone_wave(1e6, 50_000, 100e3));
        send_burst(&*device, &wave_b, Some(b));
        let (b_ack, _) = wait_burst_ack(&*device, Wall::from_secs(3));
        let got = read(&*device, Wall::from_secs(3), Some(200), |r| r.end(200).is_some_and(|end| end >= b + wave_b.len() as i64 * 200 + 30 * TICKS_PER_MS));
        device.rx_stop(0, None).unwrap();
        let _ = drain(&*device);
        let samples = got.samples;
        // A's windows are placed at where A's end appears in the capture: the control burst measured the loopback's
        // delay for this (TX rate, RX rate) pair; without it the windows stay unshifted and the trial says so.
        let shift = loop_delay.unwrap_or(0);
        let at_e = e + shift;
        let move_at = classify_tone(&samples, a, at_e, at_e, 100e3).moves_at;
        let tail_tone = common::tone::first_ms_frequency(&samples, at_e - TICKS_PER_MS, 200);
        let steady = window_power(&samples, a + shift + 10 * TICKS_PER_MS, at_e - 10 * TICKS_PER_MS);
        let tail = window_power(&samples, at_e - TICKS_PER_MS, at_e);
        let b_expected = loop_delay.map(|delay| b + delay);
        let b_onset = window_power(&samples, b - 20 * TICKS_PER_MS, b - 10 * TICKS_PER_MS).and_then(|floor| tone_start(&samples, b - 5 * TICKS_PER_MS, floor));
        let b_tone = b_expected.and_then(|onset| common::tone::first_ms_frequency(&samples, onset, 200));
        let b_power = b_expected.and_then(|onset| common::tone::first_ms_tone(&samples, onset, 200)).map(|first| first.power);
        let expected_b = if gain_trial { 100e3 } else { 200e3 };
        let (min_ratio, max_ratio) = if gain_trial { (10f64.powf(-n47::TAIL_POWER_DB / 10.0), 10f64.powf(n47::TAIL_POWER_DB / 10.0)) } else { (0.5, 2.0) };
        // `moves_at` is relative to the capture-frame `at_e`; `+ e` puts it in A's transmit frame, as `tail_verdict` compares it with `e`.
        let move_at = move_at.map(|at| at + e);
        let outcome = match (steady, tail) {
            (Some(steady), Some(tail)) => n47::tail_verdict(
                m,
                e,
                200,
                issued,
                returned,
                a_ack,
                100e3,
                tail_tone,
                Some((tail, steady)),
                min_ratio,
                max_ratio,
                move_at,
                b_tone,
                expected_b,
                b_onset,
                b_expected,
                1_000,
                b_power,
                if gain_trial { 6.0 } else { 0.0 },
            ),
            _ => Verdict::Inconclusive("A's steady or tail power was not captured".to_owned()),
        };
        println!(
            "n47 T5 m = {m:+} ms{}: apply issued at device {issued} and returned at {returned} (e = {e}, margin {:+.3} ms); A ack {a_ack:?} ({:?} from e), B {b}.., B ack {b_ack:?}; loop delay {loop_delay:?} (A's windows at {at_e}); move {move_at:?}, tail {tail_tone:?}, power {tail:?}/{steady:?}, B tone {b_tone:?} power {b_power:?}",
            if gain_trial { " (gain +6 dB)" } else { "" },
            (issued - e) as f64 / TICKS_PER_MS as f64,
            a_ack.map(|ack| ack - e)
        );
        verdict(&format!("n47 T5 m = {m:+} ms{}: the untimed setting leaves A's tail intact and B at {}", if gain_trial { " (gain)" } else { "" }, if gain_trial { "f1" } else { "f2" }), outcome);
        rows.push(serde_json::json!({ "m_ms": m, "gain_trial": gain_trial, "a": a, "e": e, "at_e": at_e, "loop_delay": loop_delay, "b": b, "issued": issued, "returned": returned,
            "a_ack": a_ack, "b_ack": b_ack, "move_at": move_at, "tail_tone": tail_tone, "tail_power": tail, "steady_power": steady, "b_tone": b_tone, "applied": format!("{applied:?}") }));
    }
    // The probes' initial state: 0 dB transmit gain (the gain trial raised it).
    device.apply(Dir::Tx, d9_channel(), &Settings { gain: Some(0.0), ..Settings::default() }, None).unwrap();
    measurement::push_raw_observation("hw_n47_raw_tx_setting_after_a_burst_end", rows.len() as u32, serde_json::json!({ "trials": rows }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_raw_rx_rate_on_an_open_streamer() {
    // Note 47 T4: a rate change on a held-open receive streamer. The tone source runs at a fixed 1 MS/s and is
    // restarted for each trial, then stopped and its burst acknowledged before the next; only the receive rate
    // changes. The old stream's packets are drained to a quiet interval before the rate is applied, every block at
    // or after the planned start is checked against the device's own packet timestamps, and a failed drain leaves the
    // rate unchanged and the trial INCONCLUSIVE.
    let inner = raw();
    let probe = Arc::new(probe_device::ProbeDevice::new(inner));
    probe.keep_rx_blocks();
    let hz = pin_slot_a(&*probe);
    let tone = iq(&tone_wave(1e6, 1_000_000, 100e3));
    raw_tx(&*probe, 1e6, hz);
    raw_rx(&*probe, 1e6, hz);
    let mut observations = Vec::new();
    let mut reports = Vec::new();
    for rate in [1e6, 2e6, 10e6, 1e6] {
        probe.rx_stop(0, None).ok();
        let stale = drain(&*probe);
        let per = (common::MCR as f64 / rate) as i64;
        if !stale.quiet {
            let opens = probe.control_log().iter().filter(|call| **call == "rx_open").count();
            verdict(
                &format!("n47 raw rx rate {rate:.0}: the read-back, the tick spacing and the fixed tone at the new rate"),
                n47::rx_rate_verdict(rate, rate, 0, &[], per, None, 100e3, opens, false),
            );
            observations.push(serde_json::json!({
                "rate_hz": rate, "incomplete": "the old stream did not drain quietly; no rate was applied",
                "stale_blocks": stale.read.blocks.len(), "stale_other": stale.read.other.iter().filter(|(_, other)| other != "Timeout").collect::<Vec<_>>(),
            }));
            continue;
        }
        let applied = probe.apply(Dir::Rx, d9_channel(), &Settings { rate: Some(rate), ..Settings::default() }, None).unwrap();
        let planned = align_to(probe.time_now().unwrap() + 50 * TICKS_PER_MS, per);
        let source_at = align_to(probe.time_now().unwrap() + 20 * TICKS_PER_MS, 200);
        let got = std::thread::scope(|scope| {
            let source = scope.spawn(|| send_burst(&*probe, &tone, Some(source_at)));
            probe.rx_start(0, planned).unwrap();
            let got =
                read(&*probe, Wall::from_secs(2), Some(per), |r| r.end(per).is_some_and(|end| end >= planned + 20 * TICKS_PER_MS) || r.other.iter().any(|(_, other)| other.starts_with("LateCommand")));
            probe.rx_stop(0, None).unwrap();
            source.join().expect("the tone source's send");
            got
        });
        // The source is stopped by its own end of burst, and that end is acknowledged before the next trial.
        let (source_ack, source_reports) = wait_burst_ack(&*probe, Wall::from_secs(3));
        reports.extend(source_reports);
        let fresh: Vec<(i64, usize)> = got.blocks.iter().copied().filter(|(tick, _)| *tick >= planned).collect();
        let tone_hz = common::tone::first_ms_frequency(&got.samples, planned, per);
        let opens = probe.control_log().iter().filter(|call| **call == "rx_open").count();
        verdict(
            &format!("n47 raw rx rate {rate:.0}: the read-back, the tick spacing and the fixed tone at the new rate"),
            n47::rx_rate_verdict(rate, applied.rate, planned, &fresh, per, tone_hz, 100e3, opens, true),
        );
        observations.push(serde_json::json!({
            "rate_hz": rate, "applied_rate": applied.rate, "applied_freq": applied.freq, "planned": planned, "per": per,
            "blocks": &fresh, "tone_hz": tone_hz, "source_at": source_at, "source_ack": source_ack,
            "stale_blocks": stale.read.blocks.len(),
            "stale_other": stale.read.other.iter().filter(|(_, other)| other != "Timeout").collect::<Vec<_>>(),
            "other": got.other.iter().filter(|(_, other)| other != "Timeout").collect::<Vec<_>>(),
        }));
    }
    probe.rx_stop(0, None).ok();
    let _ = drain(&*probe);
    println!("n47 raw rx rate: the tone source's burst ACKs {reports:?}");
    measurement::push_raw_observation("hw_n47_raw_rx_rate_on_an_open_streamer", 4, serde_json::json!({ "changes": observations }));
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_raw_tx_rate_on_an_open_streamer() {
    // Note 47 T4: a transmit rate change on a held-open streamer. The receive stays at 1 MS/s; a control burst at each
    // transmit rate calibrates the loopback's delay, so B's onset is judged against it rather than against one sample.
    let inner = raw();
    let probe = Arc::new(probe_device::ProbeDevice::new(inner));
    let hz = pin_slot_a(&*probe);
    raw_tx(&*probe, 1e6, hz);
    let flush = flush_transmit(&*probe);
    raw_rx(&*probe, 1e6, hz);
    let mut reports = flush;
    let rx_start = align_to(probe.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    probe.rx_start(0, rx_start).unwrap();
    let control_r1 = align_to(probe.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    send_burst(&*probe, &iq(&tone_wave(1e6, 5_000, 100e3)), Some(control_r1));
    let (_, control_reports) = wait_burst_ack(&*probe, Wall::from_secs(2));
    reports.extend(control_reports);
    reports.extend(drain_tx_reports(&*probe));
    let a = align_to(probe.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    let wave_a = iq(&tone_wave(1e6, 100_000, 100e3));
    send_burst(&*probe, &wave_a, Some(a));
    let e = a + wave_a.len() as i64 * 200;
    let (a_ack, a_reports) = wait_burst_ack(&*probe, Wall::from_secs(5));
    reports.extend(a_reports);
    reports.extend(drain_tx_reports(&*probe));
    while probe.time_now().unwrap() <= e + 5 * TICKS_PER_MS {
        std::thread::sleep(Wall::from_micros(200));
    }
    let applied = probe.apply(Dir::Tx, d9_channel(), &Settings { rate: Some(2e6), ..Settings::default() }, None).unwrap();
    let control_r2 = align_to(probe.time_now().unwrap() + 50 * TICKS_PER_MS, 200);
    send_burst(&*probe, &iq(&tone_wave(2e6, 10_000, 100e3)), Some(control_r2));
    let (_, control_reports) = wait_burst_ack(&*probe, Wall::from_secs(2));
    reports.extend(control_reports);
    reports.extend(drain_tx_reports(&*probe));
    let b = align_to((e + 5 * TICKS_PER_MS + 50 * TICKS_PER_MS).max(probe.time_now().unwrap() + 52 * TICKS_PER_MS), 200);
    let wave_b = iq(&tone_wave(2e6, 100_000, 100e3));
    send_burst(&*probe, &wave_b, Some(b));
    let (b_ack, b_reports) = wait_burst_ack(&*probe, Wall::from_secs(5));
    reports.extend(b_reports);
    reports.extend(drain_tx_reports(&*probe));
    // Counted after the last drain, so a late duplicate for A is still seen; a duplicate is a second ACK and the verdict fails.
    let a_acks = n47::burst_acks_in(&reports, a, e + 1);
    let b_acks = n47::burst_acks_in(&reports, b, b + wave_b.len() as i64 * 100 + 1);
    let got = read(&*probe, Wall::from_secs(3), Some(200), |r| r.end(200).is_some_and(|end| end >= b + wave_b.len() as i64 * 100 + 50 * TICKS_PER_MS));
    probe.rx_stop(0, None).ok();
    let samples = got.samples;
    let floor = window_power(&samples, control_r1 - 20 * TICKS_PER_MS, control_r1 - 10 * TICKS_PER_MS);
    let onset1 = floor.and_then(|floor| tone_start(&samples, control_r1 - 5 * TICKS_PER_MS, floor));
    let floor2 = window_power(&samples, control_r2 - 20 * TICKS_PER_MS, control_r2 - 10 * TICKS_PER_MS);
    let onset2 = floor2.and_then(|floor| tone_start(&samples, control_r2 - 5 * TICKS_PER_MS, floor));
    let delay2 = onset2.map(|two| two - control_r2);
    let b_expected = delay2.map(|delay| b + delay);
    let a_tail = common::tone::first_ms_frequency(&samples, e - TICKS_PER_MS, 200);
    let b_tone = b_expected.and_then(|onset| common::tone::first_ms_frequency(&samples, onset, 200));
    let floor_b = window_power(&samples, b - 20 * TICKS_PER_MS, b - 10 * TICKS_PER_MS);
    let b_onset = floor_b.and_then(|floor| tone_start(&samples, b - 5 * TICKS_PER_MS, floor));
    let count = |code: ezsdr_radio_uhd::TxCode| reports.iter().filter(|report| report.code == code).count();
    let opened = probe.control_log().iter().filter(|call| **call == "tx_open").count();
    verdict(
        "n47 raw tx rate 1 to 2 MS/s: A's ACK at its end, B at its tick after the loop's delay, both tones at their hertz",
        n47::tx_rate_verdict(
            2e6,
            applied.rate,
            a_ack,
            e,
            200,
            a_tail,
            100e3,
            b_onset,
            b_expected,
            1_000,
            b_tone,
            100e3,
            a_acks,
            b_acks,
            count(ezsdr_radio_uhd::TxCode::TimeError),
            count(ezsdr_radio_uhd::TxCode::Underflow) + count(ezsdr_radio_uhd::TxCode::UnderflowInPacket),
            opened,
        ),
    );
    println!(
        "n47 raw tx rate: A {a}..{e}, ack {a_ack:?} ({:?} from e); control {control_r1}/{control_r2}, onsets {onset1:?}/{onset2:?}, loop delay {delay2:?}; B {b}.., onset {b_onset:?}, ack {b_ack:?} ({:?} from B's end); reports {reports:?}",
        a_ack.map(|ack| ack - e),
        b_ack.map(|ack| ack - (b + wave_b.len() as i64 * 100))
    );
    measurement::push_raw_observation(
        "hw_n47_raw_tx_rate_on_an_open_streamer",
        1,
        serde_json::json!({ "a": a, "e": e, "b": b, "a_ack": a_ack, "b_ack": b_ack, "control": [control_r1, control_r2], "onsets": [onset1, onset2],
            "delay": delay2, "a_tail": a_tail, "b_tone": b_tone, "applied_rate": applied.rate, "applied_freq": applied.freq, "reports": reports.len() }),
    );
}

/// T2: note 47 stage 3 on the production path. A receive stream runs at f1 with a 100 kHz tone repeated by the same
/// board's transmitter; a value submitted while it streams is refused with its Action id and the tone stays; a
/// stream-scoped `Stop`, two values and a timed `start_rx` then give a segment at f1 + 50 kHz whose first block is at
/// the planned origin and whose first millisecond holds the tone at 50 kHz. The antenna is not exercised (the loopback
/// is wired to one port). `radio.START_LEAD_SHORT` is counted, not a failure (KL-35: the profile lead is not re-measured).
#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_n47_rx_restart_with_new_values() {
    let device = usrp();
    let hz = pin_slot_a(&*device);
    let probe = Arc::new(probe_device::ProbeDevice::new(device));
    probe.keep_rx_blocks();
    let dir = TempDir::new();
    let profile = bench_profile(&*probe, &dir, serde_json::json!({}), bench_envelope(&*probe), true);
    let mut run = session_with(&profile, probe.clone(), &rx0_tx0(&profile));
    past_t0(&mut run, ms(10));
    past_tx_origin(&mut run);
    let (bytes, _) = waveform_of(&tone_wave(1e6, 60_000, 100e3));
    assert!(admitted(&run.submit(verb("start_repeat", resource("radio/tx/s0"), None, &[]), Some(&bytes)).unwrap()));
    wait(&mut run, ms(60));
    let first_origin = probe.rows().into_iter().find(|row| row["call"] == "rx_start").and_then(|row| row["at"].as_i64());
    let at_first = device_samples(&probe.rx_blocks());
    let first_tone = match first_origin {
        Some(origin) => n47::tone_hold_verdict(&at_first, origin + ms(10), origin + ms(50), 200, 100e3),
        None => Verdict::Inconclusive("no rx_start row records the first segment's origin".to_owned()),
    };
    verdict("n47 stage 3 (a): the first segment holds the 100 kHz tone", first_tone);

    // (b) a receive value while the stream runs: the Kernel admits it, the Provider refuses it with its id, and the
    // stream keeps its tone and its timestamps.
    let entry = run.submit(set("radio.rx.c0.frequency_hz", ezsdr_kernel::spec::Value::num(hz + 50e3).unwrap()), None).unwrap();
    let dispatched: Vec<u64> = match &entry.outcome {
        ezsdr_kernel::session::Outcome::Admitted { dispatched, .. } => dispatched.iter().map(|id| id.0).collect(),
        ezsdr_kernel::session::Outcome::Rejected { .. } => Vec::new(),
    };
    wait(&mut run, ms(30));
    let events = run.events(0);
    let refusals = events.iter().filter(|event| event.kind == kind("radio.COMMAND_REJECTED") && event.payload["action"].as_u64().is_some_and(|id| dispatched.contains(&id))).count();
    verdict("n47 stage 3 (b): a receive value while the stream runs is refused with its Action id", n47::streaming_value_verdict(admitted(&entry), refusals));
    let after_refusal = device_samples(&probe.rx_blocks());
    let after_tone = match after_refusal.last() {
        Some((tick, _)) => n47::tone_hold_verdict(&after_refusal, tick - ms(20), *tick, 200, 100e3),
        None => Verdict::Inconclusive("no block after the refused value".to_owned()),
    };
    verdict("n47 stage 3 (b): the refused value leaves the stream at f1 without a gap", after_tone);

    // (c) Stop only the receive stream; set the new values; a timed start; the new segment at f1 + 50 kHz.
    let stop = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/rx/s0")) }, None).unwrap();
    assert!(admitted(&stop), "the receive Stop: {:?}", stop.outcome);
    for (key, value) in [("radio.rx.c0.frequency_hz", hz + 50e3), ("radio.rx.c0.gain_db", 6.0)] {
        let entry = run.submit(set(key, ezsdr_kernel::spec::Value::num(value).unwrap()), None).unwrap();
        assert!(admitted(&entry), "{key} on the stopped stream: {:?}", entry.outcome);
    }
    // RM-25 moves a request that is earlier than the previous cut plus the lead, or the ready instant plus the lead, to a
    // later lattice instant (bench 2026-11-05: a request 50 ms after the stop moved by 5.88 ms). The request is therefore
    // 200 ms ahead and on the 1 MS/s lattice, so the booked origin should equal it; the origin the Module booked is read
    // from its registered SampleClock below, never from the request or the device call.
    let ahead = after_ticks(&run, ms(200));
    let requested_tick = align_to(ahead.ticks_in(ahead.domain()).expect("the request in root ticks"), 200);
    let requested = TimePoint::new(ahead.domain(), requested_tick);
    // A capture from the request shows the published first block there, and its continuity carries the Sink's
    // GAP_BEFORE when the block began later.
    let capture = verb("capture", output("rec"), Some(requested), &[("sink.capture_samples", ezsdr_kernel::spec::Value::from(10_000))]);
    assert!(admitted(&run.submit(capture, None).unwrap()), "the capture at the restart's origin");
    let rx_starts_before = probe.count("rx_start");
    let clocks_before = run.sample_clocks().len();
    let start = run.submit(verb("start_rx", resource("radio/rx/s0"), Some(requested), &[]), None).unwrap();
    assert!(admitted(&start), "the timed restart: {:?}", start.outcome);
    wait(&mut run, ms(280));
    // The Module registers the restart's SampleClock at its first block (UR-17); its origin is the booked instant.
    let clocks = run.sample_clocks();
    let booked = (clocks.len() > clocks_before)
        .then(|| clocks.iter().rev().find(|clock| clock.stream.to_string().ends_with("usrp/rx/s0") && clock.ended_at.is_none()).and_then(|clock| clock.origin.ticks_in(clock.root).ok()))
        .flatten();
    println!("n47 stage 3 (c): requested {requested_tick}, booked origin {booked:?}");
    let planned_tick = booked.unwrap_or(requested_tick);
    // The quiet baseline is measured only after the transmit repeat is stopped, never from an interval in which the
    // receive was stopped (a stopped receive has no samples at all).
    let stop_repeat = run.submit(ezsdr_kernel::session::SessionAction::Stop { target: Some(resource("radio/tx/s0")) }, None).unwrap();
    assert!(admitted(&stop_repeat), "stopping the transmit repeat: {:?}", stop_repeat.outcome);
    wait(&mut run, ms(40));
    let manifest = run.finish();
    let blocks = device_samples(&probe.rx_blocks());
    let starts: Vec<Json> = probe.rows().into_iter().filter(|row| row["call"] == "rx_start").skip(rx_starts_before).collect();
    let origin = starts.first().and_then(|row| row["at"].as_i64());
    let first = n47::first_tick_at_or_after(&probe_block_lengths(&probe), planned_tick);
    let published = manifest.artifacts.iter().find(|(o, _)| o.as_str() == "rec").and_then(|(_, list)| list.first());
    let gap_before = published.and_then(|artifact| artifact.continuity.first()).is_some_and(|map| !map.gaps.is_empty());
    // RM-25 never books before the request, and this request is far enough ahead to be booked as asked: a booked origin
    // other than the request is a timed start that was not honoured (v3's onTime behaviour), whatever the device did.
    let booked_verdict = match booked {
        Some(tick) if tick == requested_tick => Verdict::Pass(format!("the booked origin is the request {requested_tick}")),
        Some(tick) => Verdict::Fail(format!("the booked origin {tick} is not the request {requested_tick}")),
        None => Verdict::Inconclusive("no SampleClock registered for the restart".to_owned()),
    };
    verdict("n47 stage 3 (c): the timed restart is booked at its request", booked_verdict);
    verdict("n47 stage 3 (c): the device start and the published first block are at the booked origin, without GAP_BEFORE", n47::restart_origin_verdict(booked, origin, first, gap_before));
    let floor = blocks.last().and_then(|(last, _)| window_power_at(&blocks, last - ms(11), last - ms(1), 200));
    let first_window = match floor {
        Some(floor) => {
            let (outcome, fill) = n47::restart_tone_verdict(&blocks, planned_tick, 200, floor, 50e3);
            println!("n47 stage 3 (c): restart fill {fill:?} (exact-zero samples, onset sample, from the booked origin)");
            outcome
        }
        None => Verdict::Inconclusive("no quiet window after the transmit repeat stopped: the noise floor is not measured".to_owned()),
    };
    verdict("n47 stage 3 (c): the new segment's first millisecond holds the tone at 50 kHz", first_window);
    let late = events_of(&manifest, "radio.LATE_COMMAND").len();
    verdict("n47 stage 3 (c): the restart is not reported late", n47::restart_late_verdict(late));
    let short = events_of(&manifest, "radio.START_LEAD_SHORT").len();
    println!("n47 stage 3 (c): radio.START_LEAD_SHORT counted {short} (reported, not a FAIL: the profile lead is not yet re-measured, KL-35)");
    match probe.rows().into_iter().find(|row| row["call"] == "rx_start").and_then(|row| row["at"].as_i64()) {
        Some(origin0) => {
            let blocks_at = probe_block_lengths(&probe);
            let before = blocks_at.iter().filter(|(tick, _)| *tick >= origin0 && *tick < planned_tick).copied().collect::<Vec<_>>();
            verdict("n47 stage 3 (b): the first segment has no gap across the refusal", n47::continuity_verdict(&before, 200, origin0, planned_tick));
        }
        None => verdict("n47 stage 3 (b): the first segment has no gap across the refusal", Verdict::Inconclusive("no rx_start row records the first segment's origin".to_owned())),
    }
    // (d) the read-back is the configure's: `Core::configure` records claimed/read_back/difference and the Manifest
    // exports them as `applied`. This is not a fresh measurement of the device after the Run.
    let applied = section(&manifest, "applied").as_array().cloned().unwrap_or_default();
    let read = |leaf: &str| applied.iter().rev().find(|row| row["key"].as_str().is_some_and(|key| key == format!("radio.rx.c0.{leaf}").as_str())).cloned();
    for (leaf, asked) in [("frequency_hz", hz + 50e3), ("gain_db", 6.0)] {
        let row = read(leaf);
        let verdict_name = format!("n47 stage 3 (d): the configure read back the new {leaf} (configure-time read-back, not a post-Run measurement)");
        match row {
            Some(row) => verdict(&verdict_name, n47::readback_verdict(asked, row["claimed"].as_f64(), row["read_back"].as_f64(), row["difference"].as_f64())),
            None => verdict(&verdict_name, n47::readback_verdict(asked, None, None, None)),
        }
    }
    println!("n47 stage 3: antenna not exercised (the loopback is wired to one port).");
}

/// The probe's kept device blocks as `(first tick, samples)`.
fn probe_block_lengths(probe: &probe_device::ProbeDevice) -> Vec<(i64, usize)> {
    probe.rx_blocks().iter().map(|(first, samples)| (*first, samples.len())).collect()
}

#[test]
#[ignore = "needs a USRP: see plan/phase7/bench.md"]
fn hw_noise_floor_receive() {
    // The quiet check after the ssh drop (plan/phase8/tools/ssh-drop-tx.sh): nothing transmitted, 20 000 receive samples; fewer than
    // 1 % of the 25-sample windows may hold a coherent tone (coherence over 0.5). This test's own usrp() call is the reopen.
    let device = usrp();
    let hz = pin_slot_a(&*device);
    let dir = TempDir::new();
    let manifest = captured(spec_run(&receive_spec(1, 1e6, hz, Some(20_000)), &bench_profile(&*device, &dir, serde_json::json!({}), serde_json::json!({}), false), device, Default::default()));
    let (origin, per) = rx_clock(&manifest);
    let artifact = capture_of(&manifest, "rec");
    let first = origin + artifact.continuity[0].first.ticks_in(artifact.continuity[0].domain).unwrap() * per;
    let blocks = ticked(&read_capture(&artifact, 1).remove(0), first, per);
    let windows = blocks.chunks_exact(25).count();
    let coherent = blocks.chunks_exact(25).filter(|w| common::tone::window_tone(w).0 > 0.5).count();
    let outcome = if coherent * 100 < windows {
        Verdict::Pass(format!("{coherent} of {windows} windows hold a coherent tone"))
    } else {
        Verdict::Fail(format!("{coherent} of {windows} windows hold a coherent tone"))
    };
    verdict("noise floor: a quiet receive holds no coherent tone (under 1 % of windows)", outcome);
}
