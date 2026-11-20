//! MR-35 (stage 4a, sub-step B): a receive segment of an `x3xx` channel-mode stream begins with `rx_fill_samples(D)` exact zeros, D its decimation, at every start
//! (the first segment, a restart, a retune's values, a timed start) and after an overrun's restart; the samples after the head are the ones the
//! clock puts there. `ideal` and the MR-13 patterns have no head.
//!
//! Two oracles read the samples. A repeated ramp on the air before a restart is read by its magnitude (a phase turns it, the magnitude stays): the transmit
//! side runs at 200 MS/s (5 root ticks a sample, 45 samples of path delay), so receive sample `k` of a segment at decimation `d` that starts at root tick `o`
//! hears transmit sample `(o − T) / 5 + d·k − 45` of the ramp, `T` the burst's start. The receiver's noise, present from sample 0 of the first segment, is read
//! against the same Run on `ideal`, which has no head: the same seed draws the same noise, so a magnitude after the head must equal the `ideal` run's.

use super::*;
use ezsdr_kernel::stream::BlockFlags;
use ezsdr_radio::x3xx::rx_fill_samples;

const T0: i64 = 2_000_000_000;
const SEED: u64 = 3;
/// The ramp's start on the transmit clock, in root ticks (200 MS/s: 5 ticks a sample).
const TX_AT: i64 = T0 + 2_500_000;
const WAVE: usize = 2_000;

/// The default `x3xx` layout is one 1 GbE link, 100 MB/s; the 100 and 200 MS/s streams of these cases need a 10 GbE link.
fn ten_gbe() -> [(&'static str, Value); 1] {
    [("link_gbps", Value::List(vec![Scalar::from(10)]))]
}

fn rate(d: u64) -> f64 {
    200e6 / d as f64
}

fn rx_rate(d: u64) -> (&'static str, Constraint) {
    ("radio.rx.s0.sample_rate_hz", eq(Scalar::try_from(rate(d)).unwrap()))
}

/// MR-17's overflow restart gap from the x3xx profile, in root ticks (the tests' root is 1 GHz).
fn gap_ns() -> i64 {
    ezsdr_radio::x3xx::timing().overflow_restart_gap_ns
}

fn start_rx(at: Option<i64>) -> Action {
    Action::Command { target: tgt("mock/rx/s0"), verb: Ident::parse("start_rx").unwrap(), params: BTreeMap::new(), at: at.map(|tick| AbsoluteDeadline::new(TimePoint::new(ROOT, tick))) }
}

fn update(name: &str, value: f64) -> Action {
    Action::UpdateParameter { target: tgt("mock"), key: key(name), value: Value::num(value).unwrap(), class: UpdateClass::Cold, at: None }
}

/// One published block with the root tick of its SampleClock's sample zero.
struct Got {
    origin: i64,
    first: i64,
    flags: BlockFlags,
    samples: Vec<(f32, f32)>,
}

fn blocks(radio: &Radio, world: &World) -> Vec<Got> {
    let records = world.clocks.sample_clock_records();
    let mut out = Vec::new();
    while let Some((block, _)) = radio.link.receive() {
        let header = block.header().clone();
        let domain = header.first_sample_time.domain();
        let origin = records.iter().find(|record| record.domain == domain).expect("the block's SampleClock").origin.ticks_in(ROOT).unwrap();
        let bytes = block.host_bytes().unwrap();
        let len = header.len as usize;
        let samples = (0..len).map(|index| ezsdr_hostmem::read_cf32(bytes, len, 0, index)).collect();
        out.push(Got { origin, first: header.first_sample_time.ticks_in(domain).unwrap(), flags: header.flags, samples });
    }
    out
}

/// The samples of the latest segment, by index from its sample zero; a missing stretch (a restart's gap) is None.
fn latest(got: &[Got]) -> (i64, Vec<Option<(f32, f32)>>) {
    let origin = got.iter().map(|block| block.origin).max().expect("a block");
    let mut samples: Vec<Option<(f32, f32)>> = Vec::new();
    for block in got.iter().filter(|block| block.origin == origin) {
        samples.resize((block.first as usize + block.samples.len()).max(samples.len()), None);
        for (index, sample) in block.samples.iter().enumerate() {
            samples[block.first as usize + index] = Some(*sample);
        }
    }
    (origin, samples)
}

fn magnitude((re, im): (f32, f32)) -> f64 {
    f64::from(re).hypot(f64::from(im))
}

/// What the ramp's repeat puts at receive sample `k` of a segment at decimation `d` starting at root tick `origin`.
fn heard(origin: i64, d: u64, k: i64) -> f64 {
    let wave = ramp(WAVE);
    let j = (origin - TX_AT).div_euclid(5) + d as i64 * k - 45;
    assert!(j >= 0, "the source is on the air");
    magnitude(wave[j as usize % WAVE])
}

/// The 16 samples from `from` are what the clock puts there: several samples against their own ticks, so a signal shifted by the head or by one transmit
/// sample differs.
fn assert_on_time(origin: i64, samples: &[Option<(f32, f32)>], from: usize, d: u64, gain: f64, what: &str) {
    assert!(samples.len() >= from + 16, "{what}: {} samples", samples.len());
    for (k, sample) in samples.iter().enumerate().skip(from).take(16) {
        let got = magnitude(sample.expect("a delivered sample"));
        let want = gain * heard(origin, d, k as i64);
        assert!((got - want).abs() < 1e-5, "{what}: sample {k}: magnitude {got}, the clock puts {want} there");
    }
}

/// The first `head` samples are exact zero where the source is on the air, the next 16 are on time, and no sample after the head is zero to the segment's
/// end: the head is counted from the start, not again in each block (the segment is several blocks).
fn assert_head_then_on_time(origin: i64, samples: &[Option<(f32, f32)>], from: usize, d: u64, gain: f64, what: &str) {
    let head = rx_fill_samples(d) as usize;
    assert!(samples.len() >= from + head + 16, "{what}: {} samples", samples.len());
    for (k, sample) in samples.iter().enumerate().skip(from).take(head) {
        let sample = sample.expect("a delivered sample");
        assert!(heard(origin, d, k as i64) > 0.0 && sample.0.to_bits() == 0 && sample.1.to_bits() == 0, "{what}: sample {k} of the head is exact zero, got {sample:?}");
    }
    assert_on_time(origin, samples, from + head, d, gain, what);
    assert!(samples.len() > from + 3 * 2_000, "{what}: the segment runs past two block boundaries");
    for (k, sample) in samples.iter().enumerate().skip(from + head) {
        assert!(sample.is_some_and(|sample| magnitude(sample) > 0.0), "{what}: sample {k} after the head is zero or missing");
    }
}

/// An `x3xx` Mock on a loopback at receive decimation `d`, run from `T0`, with the ramp on the air from `TX_AT` (a 200 MS/s transmit stream).
fn sourced(d: u64, root_hz: u64) -> (World, Radio) {
    let world = World::with_root(json!({ "sim.seed": SEED, "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] } }), root_hz);
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200e6).unwrap())),
        ("radio.tx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
        ("radio.rx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
        rx_rate(d),
    ];
    let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, selector: &ten_gbe(), ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    step(&world, &mut [&mut radio], T0);
    let waveform = world.waveform(&ramp(WAVE));
    radio.push(burst(&radio, &world, waveform, TX_AT / 5, true));
    step(&world, &mut [&mut radio], T0);
    (world, radio)
}

/// How a restart's values are staged between the `Stop` and the start.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// An untimed start with the values the stream had.
    Restart,
    /// A receive gain staged: the next start takes it (KL-35).
    Retune,
    /// A timed start.
    Timed,
}

/// Runs the first segment at `d` to `T1`, restarts it as `kind` and returns the new segment's samples.
fn restarted(d: u64, kind: Kind) -> (i64, Vec<Option<(f32, f32)>>, f64) {
    let (world, mut radio) = sourced(d, 1_000_000_000);
    let t1 = T0 + 3_000_000;
    step(&world, &mut [&mut radio], t1);
    radio.push(Action::Stop { target: Some(tgt("mock/rx/s0")) });
    let mut gain = 1.0;
    let mut at = None;
    match kind {
        Kind::Restart => {}
        Kind::Retune => {
            radio.push(update("radio.rx.c0.gain_db", 3.0));
            gain = 10f64.powf(3.0 / 20.0);
        }
        Kind::Timed => at = Some(t1 + 30_000_000),
    }
    radio.push(start_rx(at));
    step(&world, &mut [&mut radio], t1);
    step(&world, &mut [&mut radio], t1 + 40_000_000 + 20_000 * 5 * d as i64);
    let got = blocks(&radio, &world);
    let (origin, samples) = latest(&got);
    let first = got.iter().find(|block| block.origin == origin && block.first == 0).expect("the segment's first block");
    assert_eq!(first.flags, BlockFlags::NONE, "a head adds no flag and no gap");
    (origin, samples, gain)
}

#[test]
fn s4a_every_receive_restart_begins_with_the_head_of_its_rate() {
    // (b) an untimed restart at each class (D = 25, 10, 4, 2, 1 and 200: 2, 13, 18, 13, 2 and 24 zeros), (c) a staged receive gain that the next start takes; the timed start is the next test's.
    // The Mock keeps a stream's rate for the whole Run (KL-7), so a new rate is the Fake's case (UR-33), and a retune of the frequency would silence the loopback.
    for (d, kind) in [(200, Kind::Restart), (25, Kind::Restart), (10, Kind::Restart), (4, Kind::Restart), (2, Kind::Restart), (1, Kind::Restart), (200, Kind::Retune)] {
        let (origin, samples, gain) = restarted(d, kind);
        assert_head_then_on_time(origin, &samples, 0, d, gain, &format!("{kind:?} D {d}"));
    }
}

#[test]
fn s4a_a_timed_start_begins_at_its_instant_with_the_head() {
    let (origin, samples, _) = restarted(200, Kind::Timed);
    assert_eq!(origin, T0 + 3_000_000 + 30_000_000, "the segment begins at its instant, the head inside it");
    assert_head_then_on_time(origin, &samples, 0, 200, 1.0, "timed");
}

#[test]
fn s4a_a_head_is_counted_in_samples_whatever_the_root() {
    // A 400 MHz root: the sample clock at 100 MS/s (D = 2, class 1: 13) is 4 root ticks a sample, which is class 2's (18) when mistaken for the decimation.
    let world = World::with_root(
        json!({ "sim.seed": SEED, "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }], "noise_dbfs": { "radio": -40.0 } } }),
        400_000_000,
    );
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), rx_rate(2)];
    let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, selector: &ten_gbe(), ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 800_000_000))).unwrap();
    step(&world, &mut [&mut radio], 800_000_000);
    step(&world, &mut [&mut radio], 800_100_000);
    let (_, samples) = latest(&blocks(&radio, &world));
    let zeros = samples.iter().take_while(|sample| sample.is_some_and(|(re, im)| re.to_bits() == 0 && im.to_bits() == 0)).count();
    assert_eq!(zeros, 13, "the head of D = 2");
}

/// The Run's first segment on `profile` at decimation `d`, with the receiver's noise only (a coupling from a transmit stream that never transmits).
fn first_segment(profile: &str, d: u64) -> Vec<(f32, f32)> {
    let world =
        World::new(json!({ "sim.seed": SEED, "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }], "noise_dbfs": { "radio": -40.0 } } }));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), rx_rate(d)];
    let ten = ten_gbe();
    let mut radio = prepare(&world, Options { profile, constraints: &constraints, selector: if profile == "x3xx" { &ten } else { &[] }, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    step(&world, &mut [&mut radio], T0);
    step(&world, &mut [&mut radio], T0 + 2_000 * 5 * d as i64 + 1);
    latest(&blocks(&radio, &world)).1.into_iter().map(|sample| sample.expect("contiguous")).collect()
}

#[test]
fn s4a_the_first_segment_begins_with_the_head_and_the_noise_after_it_is_on_time() {
    // The noise is drawn for every sample, the head's included, so after the head an `x3xx` magnitude equals the `ideal` run's of the same seed; the `ideal` run
    // has no zeros, so the head is the `x3xx` Mock's doing. D = 25, 10, 4, 200 are the four classes (2, 13, 18, 24).
    for d in [25, 10, 4, 200, 2, 1] {
        let (x3xx, ideal) = (first_segment("x3xx", d), first_segment("ideal", d));
        let head = rx_fill_samples(d) as usize;
        assert!(ideal[..head + 16].iter().all(|sample| magnitude(*sample) > 0.0), "D {d}: the ideal run has noise from sample 0");
        assert!(x3xx[..head].iter().all(|(re, im)| re.to_bits() == 0 && im.to_bits() == 0), "D {d}: exact zeros for {head} samples");
        for k in head..head + 16 {
            assert!((magnitude(x3xx[k]) - magnitude(ideal[k])).abs() < 1e-6 && magnitude(x3xx[k]) > 0.0, "D {d}: sample {k} is the noise of its own draw");
        }
    }
}

#[test]
fn s4a_the_head_is_only_on_the_x3xx_signal_path() {
    // `ideal` in channel mode has no head (its first sample is noise); on `x3xx` without a channel the MR-13 ramp is the ramp from sample 0.
    assert!(magnitude(first_segment("ideal", 200)[0]) > 0.0);
    let world = World::new(json!({}));
    let selector = [("rx_test_pattern", Value::from("ramp".to_owned()))];
    let mut radio = prepare(&world, Options { profile: "x3xx", medium: false, selector: &selector, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    step(&world, &mut [&mut radio], T0);
    step(&world, &mut [&mut radio], T0 + 2_000_000 + 1);
    let (_, samples) = latest(&blocks(&radio, &world));
    for (k, sample) in samples.iter().enumerate().take(24).skip(1) {
        assert_eq!(sample.unwrap().0, k as f32 / 65_536.0, "the ramp, not a head, at sample {k}");
    }
}

#[test]
fn s4a_an_injected_overflow_restart_begins_with_the_head() {
    // MR-21: the restart after the fault's gap is a start; the SampleClock origin and the gap accounting (GAP_BEFORE | RESTARTED, lost) stay as they were.
    for d in [200, 25] {
        let world = World::new(json!({
            "sim.seed": SEED,
            "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] },
            "sim.faults": [{ "at_ns": 5_000_000, "fault": "rx_overflow", "target": "radio" }]
        }));
        let constraints = [
            ("radio.tx.c0.stream", eq(Scalar::from(0))),
            ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200e6).unwrap())),
            ("radio.tx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
            ("radio.rx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
            rx_rate(d),
        ];
        let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, selector: &ten_gbe(), ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
        step(&world, &mut [&mut radio], T0);
        let waveform = world.waveform(&ramp(WAVE));
        radio.push(burst(&radio, &world, waveform, TX_AT / 5, true));
        step(&world, &mut [&mut radio], T0);
        step(&world, &mut [&mut radio], T0 + gap_ns() + 5_000_000 + 20_000 * 5 * d as i64);
        let got = blocks(&radio, &world);
        let after = got.iter().find(|block| block.flags.bits() & BlockFlags::RESTARTED.bits() != 0).expect("a restarted block");
        assert!(after.flags.bits() & BlockFlags::GAP_BEFORE.bits() != 0 && after.first > 0, "the gap accounting is as before");
        let (origin, samples) = latest(&got);
        assert_eq!(origin, T0, "the SampleClock origin does not move");
        assert_head_then_on_time(origin, &samples, after.first as usize, d, 1.0, &format!("injected overflow, D {d}"));
    }
}

#[test]
fn s4a_a_sequence_error_skips_samples_and_has_no_head() {
    // MR-22: a sequence error skips a block's worth of samples (`GAP_BEFORE | SEQ_DISCONTINUITY`, no `RESTARTED`); it is not a start, so the samples after it are
    // the signal on its ticks with no zeros.
    for d in [200, 25] {
        let world = World::new(json!({
            "sim.seed": SEED,
            "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] },
            "sim.faults": [{ "at_ns": 5_000_000, "fault": "rx_sequence_error", "target": "radio" }]
        }));
        let constraints = [
            ("radio.tx.c0.stream", eq(Scalar::from(0))),
            ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200e6).unwrap())),
            ("radio.tx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
            ("radio.rx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
            rx_rate(d),
        ];
        let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, selector: &ten_gbe(), ..Options::default() }).unwrap();
        radio.mock.arm().unwrap();
        radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
        step(&world, &mut [&mut radio], T0);
        let waveform = world.waveform(&ramp(WAVE));
        radio.push(burst(&radio, &world, waveform, TX_AT / 5, true));
        step(&world, &mut [&mut radio], T0);
        step(&world, &mut [&mut radio], T0 + 10_000_000 + 20_000 * 5 * d as i64);
        let got = blocks(&radio, &world);
        let after = got.iter().find(|block| block.flags.bits() & BlockFlags::SEQ_DISCONTINUITY.bits() != 0).expect("a block after the sequence error");
        assert_eq!(after.flags, BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY);
        let (origin, samples) = latest(&got);
        assert_on_time(origin, &samples, after.first as usize, d, 1.0, &format!("sequence error, D {d}"));
        assert!(samples.iter().enumerate().skip(after.first as usize).all(|(_, sample)| sample.is_some_and(|sample| magnitude(sample) > 0.0)), "D {d}: no zero after the skip");
    }
}

#[test]
fn s4a_a_discard_does_not_restart_the_head() {
    // RM-38, MR-35: a full `Block` link makes the Mock read on and discard; the discard adds
    // `GAP_BEFORE` alone and is not a start, so the block after it begins with signal on its
    // own ticks, not with a head's zeros.
    let d = 200;
    let world = World::new(json!({ "sim.seed": SEED, "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] } }));
    let constraints = [
        ("radio.tx.c0.stream", eq(Scalar::from(0))),
        ("radio.tx.s0.sample_rate_hz", eq(Scalar::try_from(200e6).unwrap())),
        ("radio.tx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
        ("radio.rx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap())),
        rx_rate(d),
    ];
    let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, selector: &ten_gbe(), ..Options::default() }).unwrap();
    *radio.link.limit.lock().unwrap() = Some(1);
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    step(&world, &mut [&mut radio], T0);
    let waveform = world.waveform(&ramp(WAVE));
    radio.push(burst(&radio, &world, waveform, TX_AT / 5, true));
    step(&world, &mut [&mut radio], T0);
    step(&world, &mut [&mut radio], T0 + 5_000_000);
    let mut got = blocks(&radio, &world);
    assert_eq!(got.len(), 1, "one block queued, the second was discarded");
    *radio.link.limit.lock().unwrap() = None;
    step(&world, &mut [&mut radio], T0 + 5_999_001);
    got.extend(blocks(&radio, &world));
    let after = got.iter().find(|block| block.flags.bits() & BlockFlags::GAP_BEFORE.bits() != 0).expect("the block after the discard");
    assert_eq!(after.flags, BlockFlags::GAP_BEFORE, "a discard adds GAP_BEFORE alone");
    let (origin, samples) = latest(&got);
    assert_eq!(origin, T0);
    assert_on_time(origin, &samples, after.first as usize, d, 1.0, "the block after a discard");
    assert!(magnitude(samples[after.first as usize].expect("a delivered sample")) > 0.0, "no head restarted at the discard");
}
