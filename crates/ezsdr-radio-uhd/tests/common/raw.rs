//! The raw `Device` calls of the hardware probes that open no Session: the device channel, the receive and transmit
//! openers, timed reads and drains, the burst sender and the slot A pin. Shared by `hardware.rs` and `iii.rs`.
#![allow(dead_code)]

use std::time::{Duration as Wall, Instant};

use ezsdr_radio_uhd::{Device, Dir, Settings};

use super::n47;
use super::{SAFE_TX_HZ, bench_hz, board_of};

pub const TICKS_PER_MS: i64 = 200_000;

/// The device channel the raw probes open: 0 (slot A) unless `EZSDR_D9_CHANNEL` names another
/// (1 = slot B on an X300 with two boards), so the D9 probes can run on either.
pub fn d9_channel() -> usize {
    static CHANNEL: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CHANNEL.get_or_init(|| std::env::var("EZSDR_D9_CHANNEL").ok().map_or(0, |text| text.parse().expect("EZSDR_D9_CHANNEL names a device channel")))
}

pub fn raw_rx(device: &dyn Device, rate: f64, freq: f64) {
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("RX2".to_owned()) };
    device.apply(Dir::Rx, d9_channel(), &settings).unwrap();
    device.rx_open(0, &[d9_channel()]).unwrap();
}

/// The transmitter inside bench.md's RF conditions, or a panic before anything is sent.
pub fn raw_tx(device: &dyn Device, rate: f64, freq: f64) {
    assert!(SAFE_TX_HZ.contains(&freq) && rate <= 2e6, "bench.md RF safety: {freq} Hz at {rate} S/s");
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(0.0), antenna: Some("TX/RX".to_owned()) };
    let applied = device.apply(Dir::Tx, d9_channel(), &settings).unwrap();
    assert!(applied.gain == 0.0 && SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
    device.tx_open(0, &[d9_channel()]).unwrap();
}

pub fn iq(samples: &[(f32, f32)]) -> Vec<ezsdr_radio_uhd::Iq> {
    samples.iter().map(|(re, im)| [*re, *im]).collect()
}

/// What `rx_recv` returned over a stretch: the blocks' (first tick, length), and every
/// other outcome with the device time it was seen at.
#[derive(Default, Debug)]
pub struct Read {
    pub blocks: Vec<(i64, usize)>,
    pub samples: Vec<(i64, ezsdr_radio_uhd::Iq)>,
    pub other: Vec<(i64, String)>,
    /// Every outcome in order, samples as `"samples"`: the quiet interval is counted over this, not over `other`.
    pub events: Vec<String>,
}

impl Read {
    pub fn end(&self, per_sample: i64) -> Option<i64> {
        self.blocks.last().map(|(t, n)| t + *n as i64 * per_sample)
    }
}

/// Reads until `until(read)` holds or `limit` passes; `keep`, the ticks a sample, keeps
/// the samples with their ticks.
pub fn read(device: &dyn Device, limit: Wall, keep: Option<i64>, until: impl Fn(&Read) -> bool) -> Read {
    read_on(device, 0, limit, keep, until)
}

/// `read` on stream `k`'s streamer.
pub fn read_on(device: &dyn Device, k: usize, limit: Wall, keep: Option<i64>, until: impl Fn(&Read) -> bool) -> Read {
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
pub struct Drain {
    pub read: Read,
    pub quiet: bool,
}

/// Reads until two consecutive timeouts: the stream has stopped. The quiet count resets on every packet.
pub fn drain(device: &dyn Device) -> Drain {
    let read = read(device, Wall::from_secs(3), None, |r| n47::drain_quiet(&r.events));
    let quiet = n47::drain_quiet(&read.events);
    Drain { read, quiet }
}

/// A tone `offset_hz` from the carrier at `rate` S/s, amplitude 0.4, `n` samples; its magnitude bound is asserted.
pub fn tone_wave(rate: f64, n: usize, offset_hz: f64) -> Vec<(f32, f32)> {
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
pub fn pin_slot_a(device: &dyn Device) -> f64 {
    let board = board_of(device);
    assert!(matches!(board, ezsdr_radio::x3xx::Board::Ubx | ezsdr_radio::x3xx::Board::Obx), "slot A holds a {}: these tests transmit on an OBX or UBX only", board.name());
    let hz = bench_hz(device);
    assert!(SAFE_TX_HZ.contains(&hz), "bench RF safety: {hz} Hz is outside SAFE_TX_HZ");
    hz
}

/// Aligns `tick` up to `per` ticks.
pub fn align_to(tick: i64, per: i64) -> i64 {
    tick.div_euclid(per) * per + if tick.rem_euclid(per) == 0 { 0 } else { per }
}

/// Sends `wave` as one device burst from `first_at`, in chunks, handling partial sends. The last two samples are kept
/// for the closing send, which always carries the EOB with at least two samples (a one-sample tail is padded with a
/// zero, as the Module's own end-of-burst path does). Returns the waveform samples accepted.
pub fn send_burst(device: &dyn Device, wave: &[ezsdr_radio_uhd::Iq], first_at: Option<i64>) -> usize {
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

/// Waits for the next burst ACK (or `timeout`) and returns it with every report read on the way.
pub fn wait_burst_ack(device: &dyn Device, timeout: Wall) -> (Option<i64>, Vec<ezsdr_radio_uhd::TxReport>) {
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
