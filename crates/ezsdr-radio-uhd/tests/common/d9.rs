//! Shared raw D9 TX sequences for the hardware probes and their FakeDevice differential.

use std::time::{Duration as Wall, Instant};

use ezsdr_radio_uhd::{Device, DeviceError, Dir, Iq, Settings, TxCode, TxReport};

use super::SAFE_TX_HZ;

pub const SAMPLE_TICKS: i64 = 200;
const MS: i64 = 200_000;
const BASE_HZ: f64 = 1e9;
const SHIFT_HZ: f64 = 50e3;
const TX_TIMEOUT: Wall = Wall::from_secs(2);

pub type DspTune<'a> = dyn FnMut(Dir, usize, f64, Option<i64>) -> Result<f64, DeviceError> + 'a;

#[derive(Debug, Default)]
pub struct Capture {
    pub blocks: Vec<(i64, usize)>,
    pub samples: Vec<(i64, Iq)>,
    pub other: Vec<(i64, String)>,
}

impl Capture {
    pub fn end(&self) -> Option<i64> {
        self.blocks.last().map(|(tick, n)| tick + *n as i64 * SAMPLE_TICKS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P5Case {
    StaleHeader,
    NoStaleHeader,
    FlushAfterReopen,
}

impl P5Case {
    pub fn label(self) -> &'static str {
        match self {
            Self::StaleHeader => "ended with a time",
            Self::NoStaleHeader => "ended without a time",
            Self::FlushAfterReopen => "ended with a time, then a two-zero flush after reopen",
        }
    }
}

#[derive(Debug)]
pub struct P5Result {
    pub case: P5Case,
    pub old_header: Option<i64>,
    pub start: i64,
    pub e: i64,
    pub dsp: f64,
    pub flush_reports: Vec<TxReport>,
    pub capture: Capture,
    pub reports: Vec<TxReport>,
}

#[derive(Debug)]
pub struct P6Result {
    pub case: &'static str,
    pub start: i64,
    pub e: i64,
    pub dsp: f64,
    pub capture: Capture,
    pub reports: Vec<TxReport>,
}

#[derive(Debug)]
pub struct StarvationResult {
    pub start: i64,
    pub queued_end: i64,
    pub resumed_at: i64,
    pub send: Result<usize, DeviceError>,
    pub capture: Capture,
    pub reports: Vec<TxReport>,
}

#[derive(Debug)]
pub struct D70Result {
    pub case: &'static str,
    pub rx_at: i64,
    pub start: i64,
    pub priming_reports: Vec<TxReport>,
    pub capture: Capture,
    pub reports: Vec<TxReport>,
}

pub fn tone_1msps(n: usize) -> Vec<Iq> {
    (0..n).map(|i| {
        let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
        [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
    }).collect()
}

/// Opens/configures the same 1 Msps, 1 GHz, 0 dB raw loopback used by the bench probes.
pub fn open_raw(device: &dyn Device, chan: usize) {
    assert!(SAFE_TX_HZ.contains(&BASE_HZ) && SAFE_TX_HZ.contains(&(BASE_HZ + SHIFT_HZ + 100e3)));
    assert!(tone_1msps(25).iter().all(|sample| f64::from(sample[0]).hypot(f64::from(sample[1])) <= 0.5), "bench.md RF safety: probe amplitude is at most 0.5");
    device.apply(Dir::Rx, chan, &Settings { rate: Some(1e6), freq: Some(BASE_HZ), gain: Some(0.0), antenna: Some("RX2".to_owned()) }, None).unwrap();
    device.rx_open(0, &[chan]).unwrap();
    let applied = device.apply(Dir::Tx, chan, &Settings { rate: Some(1e6), freq: Some(BASE_HZ), gain: Some(0.0), antenna: Some("TX/RX".to_owned()) }, None).unwrap();
    assert_eq!(applied.gain, 0.0);
    assert!(SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
    device.tx_open(0, &[chan]).unwrap();
}

fn tune_zero(tune: &mut DspTune<'_>, chan: usize) {
    tune(Dir::Rx, chan, 0.0, None).unwrap();
    tune(Dir::Tx, chan, 0.0, None).unwrap();
}

fn after_ms(device: &dyn Device, ms: i64) -> i64 {
    (device.time_now().unwrap() / SAMPLE_TICKS + ms * 1_000) * SAMPLE_TICKS
}

/// The device host clock is manual in the fake and real on hardware; this waits on its own axis.
fn wait_until(device: &dyn Device, tick: i64) {
    while device.time_now().unwrap() < tick {
        device.host().sleep(Wall::from_micros(200));
    }
}

fn read_until(device: &dyn Device, end: i64) -> Capture {
    let mut out = Capture::default();
    let begun = Instant::now();
    while begun.elapsed() < Wall::from_secs(3) && out.end().is_none_or(|got| got < end) {
        match device.rx_recv(0, 2_000, Wall::from_millis(100)) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                out.blocks.push((first_tick, samples[0].len()));
                out.samples.extend(samples[0].iter().enumerate().map(|(i, sample)| (first_tick + i as i64 * SAMPLE_TICKS, *sample)));
            }
            other => out.other.push((device.time_now().unwrap_or(-1), format!("{other:?}"))),
        }
    }
    assert!(out.end().is_some_and(|got| got >= end), "raw probe capture ended at {:?}, wanted {end}; {:?}", out.end(), out.other);
    out
}

fn drain_rx(device: &dyn Device) {
    let begun = Instant::now();
    let mut timeouts = 0;
    while begun.elapsed() < Wall::from_secs(3) && timeouts < 2 {
        match device.rx_recv(0, 2_000, Wall::from_millis(100)) {
            ezsdr_radio_uhd::RxRecv::Timeout => timeouts += 1,
            _ => timeouts = 0,
        }
    }
}

fn reports(device: &dyn Device, wait: Wall) -> Vec<TxReport> {
    let until = device.host().now() + wait;
    let mut out = Vec::new();
    loop {
        let remaining = until.saturating_sub(device.host().now());
        if remaining.is_zero() { break; }
        match device.tx_async(0, remaining.min(Wall::from_millis(10))).unwrap() {
            Some(report) => out.push(report),
            None => break,
        }
    }
    out
}

/// Raw-probe reset: a two-zero untimed SOB/EOB and its channel BurstAck.
fn flush_two_zeros(device: &dyn Device) -> Vec<TxReport> {
    let zeros = [Iq::default(); 2];
    assert_eq!(device.tx_send(0, &[&zeros], None, true, true, Wall::from_secs(1)).unwrap(), 2);
    let until = device.host().now() + Wall::from_millis(100);
    let mut out = Vec::new();
    while device.host().now() < until {
        if let Some(report) = device.tx_async(0, until.saturating_sub(device.host().now()).min(Wall::from_millis(10))).unwrap() {
            let ack = report.code == TxCode::BurstAck && report.channel == 0;
            out.push(report);
            if ack { break; }
        }
    }
    out
}

fn reopen_after_reset(device: &dyn Device, chan: usize) {
    device.rx_close(0);
    device.tx_close(0);
    device.set_time_zero(false).unwrap();
    device.rx_open(0, &[chan]).unwrap();
    device.tx_open(0, &[chan]).unwrap();
}

fn timed_chunks(device: &dyn Device, start: i64, tone: &[Iq], range: std::ops::Range<usize>, first: bool, last: bool) {
    let mut k = range.start;
    while k < range.end {
        let n = 2_000.min(range.end - k);
        let eob = last && k + n == range.end;
        let sent = device.tx_send(0, &[&tone[k..k + n]], Some(start + k as i64 * SAMPLE_TICKS), first && k == 0, eob, TX_TIMEOUT).unwrap();
        assert_eq!(sent, n, "the raw probe's timed chunk is accepted");
        k += n;
    }
}

pub fn p5_stale_header(device: &dyn Device, chan: usize, tune: &mut DspTune<'_>, cases: &[P5Case]) -> Vec<P5Result> {
    open_raw(device, chan);
    let tone = tone_1msps(60_000);
    let mut out = Vec::new();
    for &case in cases {
        tune_zero(tune, chan);
        let old_start = after_ms(device, 400);
        let old_header = (case != P5Case::NoStaleHeader).then_some(old_start + 10 * MS);
        device.rx_start(0, old_start - 5 * MS).unwrap();
        device.tx_send(0, &[&tone[..10_000]], Some(old_start), true, false, TX_TIMEOUT).unwrap();
        let end_at = (case != P5Case::NoStaleHeader).then_some(old_start + 10 * MS);
        device.tx_send(0, &[&[]], end_at, false, true, TX_TIMEOUT).unwrap();
        let _ = read_until(device, old_start + 15 * MS);
        device.rx_stop(0, None).unwrap();
        drain_rx(device);
        let _ = reports(device, Wall::from_millis(10));
        reopen_after_reset(device, chan);
        tune_zero(tune, chan);
        let flush_reports = if case == P5Case::FlushAfterReopen { flush_two_zeros(device) } else { Vec::new() };
        let start = after_ms(device, 150);
        let e = start + 20 * MS;
        device.rx_start(0, start - 5 * MS).unwrap();
        let dsp = tune(Dir::Tx, chan, SHIFT_HZ, Some(e)).unwrap();
        device.tx_send(0, &[&tone], Some(start), true, true, TX_TIMEOUT).unwrap();
        let capture = read_until(device, start + 60 * MS);
        device.rx_stop(0, None).unwrap();
        drain_rx(device);
        let tx_reports = reports(device, Wall::from_millis(10));
        out.push(P5Result { case, old_header, start, e, dsp, flush_reports, capture, reports: tx_reports });
    }
    out
}

pub fn p6_every_chunk_timed(device: &dyn Device, chan: usize, tune: &mut DspTune<'_>) -> (Vec<P6Result>, StarvationResult) {
    open_raw(device, chan);
    let tone = tone_1msps(60_000);
    let mut out = Vec::new();
    for (case, late_tune) in [("given before the burst", false), ("given 3 ms before e", true)] {
        tune_zero(tune, chan);
        let _ = flush_two_zeros(device);
        let start = after_ms(device, 150);
        let e = start + 20 * MS;
        device.rx_start(0, start - 5 * MS).unwrap();
        if late_tune {
            timed_chunks(device, start, &tone, 0..25_000, true, false);
            wait_until(device, e - 3 * MS);
        }
        let dsp = tune(Dir::Tx, chan, SHIFT_HZ, Some(e)).unwrap();
        timed_chunks(device, start, &tone, if late_tune { 25_000..60_000 } else { 0..60_000 }, !late_tune, true);
        let capture = read_until(device, start + 60 * MS);
        device.rx_stop(0, None).unwrap();
        drain_rx(device);
        out.push(P6Result { case, start, e, dsp, capture, reports: reports(device, Wall::from_millis(10)) });
    }

    tune(Dir::Tx, chan, 0.0, None).unwrap();
    let _ = flush_two_zeros(device);
    let start = after_ms(device, 150);
    device.rx_start(0, start - 5 * MS).unwrap();
    let queued_end = start + 20 * MS;
    timed_chunks(device, start, &tone, 0..20_000, true, false);
    wait_until(device, queued_end + 5 * MS);
    let resumed_at = device.time_now().unwrap();
    let send = device.tx_send(0, &[&tone[20_000..22_000]], Some(queued_end), false, true, TX_TIMEOUT);
    let capture = read_until(device, resumed_at + 5 * MS);
    device.rx_stop(0, None).unwrap();
    drain_rx(device);
    let tx_reports = reports(device, Wall::from_millis(10));
    (out, StarvationResult { start, queued_end, resumed_at, send, capture, reports: tx_reports })
}

pub fn p7_untimed_continuations(device: &dyn Device, chan: usize, tune: &mut DspTune<'_>) -> P6Result {
    open_raw(device, chan);
    let tone = tone_1msps(60_000);
    tune_zero(tune, chan);
    let flush_reports = flush_two_zeros(device);
    let start = after_ms(device, 150);
    let e = start + 20 * MS;
    device.rx_start(0, start - 5 * MS).unwrap();
    let dsp = tune(Dir::Tx, chan, SHIFT_HZ, Some(e)).unwrap();
    device.tx_send(0, &[&tone[..2_000]], Some(start), true, false, TX_TIMEOUT).unwrap();
    let mut k = 2_000;
    while k < 60_000 {
        let n = 2_000.min(60_000 - k);
        let sent = device.tx_send(0, &[&tone[k..k + n]], None, false, k + n == 60_000, TX_TIMEOUT).unwrap();
        assert_eq!(sent, n);
        k += n;
    }
    let capture = read_until(device, start + 60 * MS);
    device.rx_stop(0, None).unwrap();
    drain_rx(device);
    let mut tx_reports = flush_reports;
    tx_reports.extend(reports(device, Wall::from_millis(10)));
    P6Result { case: "current untimed-continuation shape", start, e, dsp, capture, reports: tx_reports }
}

pub fn d70_one_sample_then_two(device: &dyn Device, chan: usize, tune: &mut DspTune<'_>) -> Vec<D70Result> {
    open_raw(device, chan);
    let tone = tone_1msps(30_000);
    let mut out = Vec::new();
    for case in ["one-sample burst", "two-zero-sample end burst"] {
        tune_zero(tune, chan);
        let rx_at = after_ms(device, 20);
        device.rx_start(0, rx_at).unwrap();
        wait_until(device, rx_at + MS);
        let priming_reports = if case.starts_with("one") {
            let zero = [Iq::default()];
            assert_eq!(device.tx_send(0, &[&zero], None, true, true, Wall::from_secs(1)).unwrap(), 1);
            reports(device, Wall::from_millis(10))
        } else {
            flush_two_zeros(device)
        };
        let start = after_ms(device, 150);
        device.tx_send(0, &[&tone], Some(start), true, true, TX_TIMEOUT).unwrap();
        let capture = read_until(device, start + 40 * MS);
        device.rx_stop(0, None).unwrap();
        drain_rx(device);
        out.push(D70Result { case, rx_at, start, priming_reports, capture, reports: reports(device, Wall::from_millis(10)) });
    }
    out
}

pub fn report_codes(reports: &[TxReport]) -> Vec<TxCode> {
    reports.iter().map(|report| report.code).collect()
}
