//! Maintenance table 3, sub-step C: the chunked late burst, cell C1 of `plan/phase8/bench.md` (AGENTS §9: a hardware test whose expectation comes from
//! a rule has a rehearsal on the FakeDevice). One lifted body, `late_chunked_cell`, runs on the USRP (`hardware.rs`, `hw_mt3_c1_chunked_late_burst`) and
//! on `FakeDevice` (`fake.rs`, the rehearsal); the verdict is the pure `late_chunked_verdict`, so the evidence it needs is a plain value that its unit
//! tests build by hand.
//!
//! The rule under test (UR-33, spec 18): the X310's radio drops a late packet at one sample per tick and the DUC carries a burst's first time to its end,
//! so a device burst whose first timed packet arrives late is dropped whole, through its EOB, with no BurstAck (VERIFIED in the FPGA source, and on the
//! bench for one 20 000-sample send at 1 MS/s; the Module's chunked shape, other rates and board #2 are INFERRED until this cell runs).
//!
//! A trial sends an on-time control burst of the same chunked shape and hears it (the receiver and the loopback work, and the tone is the one the
//! late burst would carry), then the late burst, whose first sample's time is `LATE_MS` before the host's reference instant (the host sends the first chunk at or
//! after it, so the first send is at least that late). The late burst is heard only if some of it played. Nothing heard proves nothing without the control,
//! complete receive coverage of the late burst's interval, and a last chunk handed at least `LAST_CHUNK_MARGIN_TICKS` (1 ms) before its own time (a device that dropped
//! each late packet alone would play it), so the verdict is INCONCLUSIVE without them. The control's reports are read after the late burst, so nothing
//! delays the late send; reports are attributed by their ticks.

#![allow(dead_code)]

use std::time::{Duration as Wall, Instant};

use ezsdr_radio::segment::send_cap;
use ezsdr_radio::x3xx::PACKETS;
use ezsdr_radio_uhd::{Device, Iq, RxRecv, TxCode, TxReport};
use serde_json::{Value as Json, json};

use super::observations::HeardInterval;
use super::raw::{TICKS_PER_MS, align_to, d9_channel, drain_without_clock, iq, raw_rx_on, raw_tx_on, tone_wave};
use super::tone::{QUIET_CHECK_MIN_POWER, Verdict, contiguous_windows_at, mean_power, window_tone_at};
use super::{MCR, iii};

/// The rates cell C1 runs at (bench.md: C2 adds 100 and 200 MS/s and board #2).
pub const C1_RATES: [f64; 2] = [1e6, 10e6];
/// C2's transmit rates (the receive stays at `C2_RX_RATE`, which keeps C1's tone window and threshold).
pub const C2_RATES: [f64; 2] = [100e6, 200e6];
pub const C2_RX_RATE: f64 = 10e6;
/// C3's rates.
pub const C3_RATES: [f64; 2] = [1e6, 10e6];
/// C3's first send: UR-22's minimum first send of an open burst (`ezsdr_radio::x3xx::PACKETS.min_open_first_send`).
pub const C3_HEAD: usize = 32;
/// Trials per rate on the bench.
pub const C1_TRIALS: usize = 10;
/// How long before the host's reference instant the late burst's first sample is timed: the host sends the first chunk at or after that instant, so the
/// first send is at least this late (the verdict checks what matters, that the last chunk was handed well before its own time: `Evidence::late_last`).
pub const LATE_MS: i64 = 1;
/// The transmit tone's offset from the carrier at every rate: the 100 kHz the other bench tools use, so the carrier plus the tone stays inside `SAFE_TX_HZ`.
pub const TONE_HZ: f64 = 100e3;
/// How long before its own time the late burst's last chunk must have been handed: 1 ms. The check measures when the host's send returned, not when the
/// packet reached the radio, so the margin covers the host-to-radio delay and the read's own latency (INFERRED to be far under it).
const LAST_CHUNK_MARGIN_TICKS: i64 = TICKS_PER_MS;
/// Sends per C1 burst: the Module's chunks (`segment::send_cap`) of a 20 ms burst at 1 MS/s (16 ms at 10 MS/s: C1 stays as it is).
pub const CHUNKS: usize = 8;
/// The length of every burst after C1, at every rate: the last chunk can then be handed well before its own time even when the host feeds only a little
/// faster than real time (INFERRED: 10 GbE at 200 MS/s is about 1.5x).
pub const BURST_MS: i64 = 20;
/// The Module's in-flight window (`x3xx` timing, 10 ms): a chunk is sent once it is inside it, as the Module does.
const WINDOW_TICKS: i64 = 10 * TICKS_PER_MS;
/// The samples of a tone window (the probes' window, `tone.rs`).
const WINDOW: usize = 25;
const TX_TIMEOUT: Wall = Wall::from_millis(500);

/// Everything `late_chunked_verdict` reads of one trial. Ticks are device ticks; the sent intervals are on the transmit axis, the receive blocks and the
/// heard intervals on the receive axis, which the loopback delays.
#[derive(Clone, Debug)]
pub struct Evidence {
    /// Root ticks a receive sample: the axis of the heard intervals, the receive blocks and the tone windows.
    pub per: i64,
    /// Root ticks a transmit sample (C2 receives at 10 MS/s whatever it transmits at).
    pub tx_per: i64,
    /// Root ticks of one send (the chunk).
    pub chunk_ticks: i64,
    /// The late burst's expected interval `[first sample, end)`.
    pub late: (i64, i64),
    /// The control burst's expected interval.
    pub control: (i64, i64),
    /// Failed device calls (`rx_start`, a send, a report read).
    pub errors: Vec<String>,
    /// The receive blocks `(first tick, samples)` in the order they came.
    pub blocks: Vec<(i64, usize)>,
    /// Each receive outcome that was not samples, with the stream position it came at (the end of the last block before it; `None` before the first).
    pub other: Vec<Option<i64>>,
    /// The tone heard, on the receive axis.
    pub heard: Vec<HeardInterval>,
    /// Every transmit report of the trial, in order.
    pub reports: Vec<TxReport>,
    /// The late burst's last send: the device time read when it had been handed, and its own first sample's time. A device that drops each late packet alone
    /// plays a chunk handed before its own time, so silence means something only if this one was handed at least `LAST_CHUNK_MARGIN_TICKS` before it (`None`: it
    /// was not handed).
    pub late_last: Option<(i64, i64)>,
    /// The late burst's first send (a two-channel trial records it: where the host's latency put the first send against the first sample's time, which decides C5's
    /// transition): the device time read when it had been handed, and its own first sample's time.
    pub late_first: Option<(i64, i64)>,
}

/// The delay the control tone arrived with: its heard onset less its first sample's tick, when the control was heard (at least 90 % of its samples, its
/// onset within a tone window before to one chunk after its first sample's tick). A window that straddles the onset can read as tone (a mixed window's coherence
/// is not 1, but it can pass the bar) and a clean one starts up to a window after it, so the delay is read to about 25 samples either way.
fn control_delay(e: &Evidence) -> Option<i64> {
    let (from, to) = e.control;
    let slack = WINDOW as i64 * e.per;
    let heard: Vec<&HeardInterval> = e.heard.iter().filter(|iv| iv.first_tick >= from - slack && iv.first_tick < to + e.chunk_ticks).collect();
    let samples: usize = heard.iter().map(|iv| iv.samples).sum();
    let onset = heard.iter().map(|iv| iv.first_tick).min()?;
    (samples as i64 * 10 >= (to - from) / e.per * 9 && onset < from + e.chunk_ticks).then_some((onset - from).max(0))
}

/// Whether the receive blocks cover `[from, to)` without a gap: one run of blocks that each start where the one before ended.
fn covers(blocks: &[(i64, usize)], per: i64, from: i64, to: i64) -> bool {
    let mut run: Option<(i64, i64)> = None;
    for (first, len) in blocks {
        let end = first + *len as i64 * per;
        run = match run {
            Some((start, last)) if last == *first => Some((start, end)),
            _ => Some((*first, end)),
        };
        if run.is_some_and(|(start, last)| start <= from && last >= to) {
            return true;
        }
    }
    false
}

fn is_late_report(code: TxCode) -> bool {
    matches!(code, TxCode::TimeError | TxCode::Underflow | TxCode::UnderflowInPacket)
}

/// The verdict of one trial, in this order. FAIL is positive evidence and holds whatever the coverage: a BurstAck whose tick is at or after the late
/// burst's first sample (the control has its own, before), or a heard interval that overlaps the late burst's interval, the loopback delay taken off,
/// not only one inside it. INCONCLUSIVE when the evidence that makes silence mean something is missing: a failed call, receive blocks that do not cover
/// the interval plus a chunk each side without a gap, a non-sample receive outcome inside that range, a control that was not heard or was itself late,
/// a report whose tick cannot attribute it, a last chunk not handed 1 ms before its own time, or no TimeError (the burst was not late). PASS only with all of it,
/// nothing heard in the interval and no BurstAck.
pub fn late_chunked_verdict(e: &Evidence) -> Verdict {
    let (from, to) = e.late;
    let delay = control_delay(e);
    let shift = delay.unwrap_or(0);
    let acks: Vec<i64> = e.reports.iter().filter(|r| r.code == TxCode::BurstAck).filter_map(|r| r.tick).filter(|tick| *tick >= from).collect();
    if !acks.is_empty() {
        return Verdict::Fail(format!("a BurstAck for the late burst at tick(s) {acks:?}: a late burst is dropped without one"));
    }
    let played: Vec<(i64, i64)> = e.heard.iter().map(|iv| (iv.first_tick - shift, iv.last_tick - shift + e.per)).filter(|(first, end)| *first < to && *end > from).collect();
    if !played.is_empty() {
        return Verdict::Fail(format!("heard {played:?} (loopback delay {shift} ticks taken off) inside the late burst's interval {from}..{to}: part of a late burst played"));
    }
    let reasons = inconclusive_reasons(e, delay);
    if reasons.is_empty() {
        Verdict::Pass(format!("no 100 kHz tone window was detected in the late burst's interval, no BurstAck for it, a TimeError came; the control was heard (delay {shift} ticks)"))
    } else {
        Verdict::Inconclusive(reasons.join("; "))
    }
}

/// Why the verdict is INCONCLUSIVE (empty: nothing is missing), given the control's `delay`.
pub fn late_chunked_reasons(e: &Evidence) -> Vec<String> {
    inconclusive_reasons(e, control_delay(e))
}

fn inconclusive_reasons(e: &Evidence, delay: Option<i64>) -> Vec<String> {
    let (from, to) = e.late;
    let shift = delay.unwrap_or(0);
    let mut reasons: Vec<String> = e.errors.iter().map(|error| format!("a device call failed: {error}")).collect();
    let (want_from, want_to) = (from + shift - e.chunk_ticks, to + shift + e.chunk_ticks);
    if !covers(&e.blocks, e.per, want_from, want_to) {
        reasons.push(format!("receive blocks do not cover {want_from}..{want_to} without a gap"));
    }
    if e.other.iter().any(|at| at.is_some_and(|at| at >= want_from && at < want_to)) {
        reasons.push("a receive outcome other than samples came inside that range".to_owned());
    }
    if delay.is_none() {
        reasons.push("the control burst was not heard".to_owned());
    }
    match e.late_last {
        Some((handed, own)) if handed + LAST_CHUNK_MARGIN_TICKS <= own => {}
        other => reasons.push(format!(
            "the late burst's last send was not handed 1 ms before its own time {other:?} (handed, own): a device that drops late packets one by one could have dropped only the early ones"
        )),
    }
    if e.reports.iter().any(|r| r.tick.is_none() && (r.code == TxCode::BurstAck || is_late_report(r.code))) {
        reasons.push("a report without a tick cannot be attributed to the control or the late burst".to_owned());
    }
    if e.reports.iter().any(|r| is_late_report(r.code) && r.tick.is_some_and(|tick| tick < from)) {
        reasons.push("the control burst was itself late or starved".to_owned());
    }
    if !e.reports.iter().any(|r| r.code == TxCode::TimeError && r.tick.is_some_and(|tick| tick >= from)) {
        reasons.push("no TimeError: the burst was not late, so nothing was proved".to_owned());
    }
    reasons
}

/// The tone windows of `samples` (25 samples, each one spacing apart) as intervals: coherent over 0.9 (`tone.rs`'s monitor bar), at `tone_hz` within a fifth
/// and above the 4-LSB amplitude floor (`QUIET_CHECK_MIN_POWER`), so noise and the FakeDevice's ramp (a coherent window at 0 Hz) are not a tone. Windows
/// that follow one another join; ticks are the samples' own.
pub fn tone_intervals(samples: &[(i64, Iq)], per: i64, tone_hz: f64) -> Vec<HeardInterval> {
    let mut intervals: Vec<HeardInterval> = Vec::new();
    for w in contiguous_windows_at(samples, per) {
        let (coherence, hz) = window_tone_at(w, per);
        if coherence <= 0.9 || (hz - tone_hz).abs() > tone_hz / 5.0 || mean_power(w) <= QUIET_CHECK_MIN_POWER {
            continue;
        }
        let (first, last) = (w[0].0, w[WINDOW - 1].0);
        match intervals.last_mut() {
            Some(open) if open.last_tick + per == first => {
                open.last_tick = last;
                open.samples += WINDOW;
            }
            _ => intervals.push(HeardInterval { first_tick: first, last_tick: last, samples: WINDOW }),
        }
    }
    intervals
}

/// Holds until the device's time reaches `tick`, on the device's own clock (the manual one on the fake, the real one on the USRP), and returns the time it read.
/// `bench1115::wait_device_time` does not fit: it is private and sleeps on the thread, which would never move a manual clock this thread holds. A failed time
/// read is retried for 50 ms (it is transient on a device that recovers) and then is an error: the caller must not send on a wait that did not happen, or a
/// burst meant to be late goes out on time.
pub fn wait_until(device: &dyn Device, tick: i64) -> Result<i64, String> {
    let mut failed = 0;
    loop {
        match device.time_now() {
            Ok(now) if now >= tick => return Ok(now),
            Ok(_) => device.host().sleep(Wall::from_micros(200)),
            Err(error) if failed >= 50 => return Err(format!("time_now: {}", error.message)),
            Err(_) => {
                failed += 1;
                device.host().sleep(Wall::from_millis(1));
            }
        }
    }
}

/// One send of a chunked burst: its first sample's time and the device time read once it had been handed.
#[derive(Clone, Copy, Debug)]
pub struct Sent {
    pub at: i64,
    pub handed: i64,
}

/// How `send_chunked` sends one burst: from `first_at`, `per` root ticks a sample, `head` samples first and `chunk` after, `channels` buffers a send, the
/// last send carrying the EOB when `eob`, and the device time read after every send when `each` (else after the last one only: at 100 and 200 MS/s a burst
/// is 100 to 200 sends, and a time read is a control call; after the first one too when `first`).
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub first_at: i64,
    pub per: i64,
    pub chunk: usize,
    pub head: usize,
    pub eob: bool,
    pub each: bool,
    pub first: bool,
    pub channels: usize,
}

/// Sends `wave` as one device burst of timed sends (SOB on the first, each with its first sample's time), each once it is inside the in-flight window, as the
/// Module does; a short send's rest follows with its own time. Returns the sends with a time read (`Plan::each`, `Plan::first`), the last one at least. The wait reads the device
/// time only when the last read is not already inside the window.
pub fn send_chunked(device: &dyn Device, wave: &[Iq], plan: Plan) -> Result<Vec<Sent>, String> {
    let (mut sent, mut known, mut out) = (0, i64::MIN, Vec::new());
    while sent < wave.len() {
        let n = if sent == 0 { plan.head } else { plan.chunk }.min(wave.len() - sent);
        let at = plan.first_at + sent as i64 * plan.per;
        if known < at - WINDOW_TICKS {
            known = wait_until(device, at - WINDOW_TICKS)?;
        }
        let buffers = vec![&wave[sent..sent + n]; plan.channels];
        let took = device.tx_send(0, &buffers, Some(at), sent == 0, plan.eob && sent + n == wave.len(), TX_TIMEOUT).map_err(|error| error.message)?;
        if took == 0 {
            return Err(format!("the device took none of the samples sent at sample {sent}"));
        }
        sent += took;
        if plan.each || (plan.first && at == plan.first_at) || sent == wave.len() {
            known = device.time_now().map_err(|error| error.message)?;
            out.push(Sent { at, handed: known });
        }
    }
    Ok(out)
}

/// Every report that is there, until three quiet reads of 20 ms.
pub fn read_reports(device: &dyn Device, errors: &mut Vec<String>) -> Vec<TxReport> {
    let begun = Instant::now();
    let (mut quiet, mut out) = (0, Vec::new());
    while begun.elapsed() < Wall::from_secs(2) && quiet < 3 {
        match device.tx_async(0, Wall::from_millis(20)) {
            Ok(Some(report)) => {
                quiet = 0;
                out.push(report);
            }
            Ok(None) => quiet += 1,
            Err(error) => {
                errors.push(format!("tx_async: {}", error.message));
                break;
            }
        }
    }
    out
}

/// What the receive thread read: the blocks (the same ticks for every channel) and the samples of each channel, the stream position each other outcome came at
/// (the end of the last block before it, `None` before the first) and the failed calls.
struct RxLog {
    blocks: Vec<(i64, usize)>,
    samples: Vec<Vec<(i64, Iq)>>,
    other: Vec<Option<i64>>,
    errors: Vec<String>,
}

/// The receive thread is a participant of the device's clock like the sending thread (`late_chunked_cell`'s): the clock stands while it is descheduled, so
/// a loaded host cannot make the fake report the reader as 100 ms behind (an `Overflow`, which would turn the trial INCONCLUSIVE). A real clock registers
/// nothing. It reads until the blocks reach `end_tick`, or a call fails (a closed streamer fails at once and would be polled to the limit), or 10 s.
fn read_rx(device: &dyn Device, per: i64, n: usize, end_tick: i64, channels: usize) -> RxLog {
    let _driver = device.host().drive();
    let mut log = RxLog { blocks: Vec::new(), samples: vec![Vec::new(); channels], other: Vec::new(), errors: Vec::new() };
    let (begun, mut end) = (Instant::now(), None);
    while begun.elapsed() < Wall::from_secs(10) && end.is_none_or(|end| end < end_tick) {
        match device.rx_recv(0, n, Wall::from_millis(100), false) {
            RxRecv::Samples { first_tick, samples } => {
                log.blocks.push((first_tick, samples[0].len()));
                end = Some(first_tick + samples[0].len() as i64 * per);
                for (kept, got) in log.samples.iter_mut().zip(&samples) {
                    kept.extend(got.iter().enumerate().map(|(i, s)| (first_tick + i as i64 * per, *s)));
                }
            }
            failed @ RxRecv::Failed(_) => {
                log.other.push(end);
                log.errors.push(format!("rx_recv: {failed:?}"));
                break;
            }
            _ => log.other.push(end),
        }
    }
    log
}

/// What one trial of a late-burst cell sends and how: the rates (C2 transmits at 100 or 200 MS/s and receives at 10), the burst's samples (control and late
/// burst alike), the late burst's first send when it is not a chunk (C3), the late burst's first sample's time less the reference instant in ticks (C1: 1 ms
/// before; C5's marginal cells straddle it), and the channels (C5 transmits and receives on both slots as one two-channel stream).
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub tx_rate: f64,
    pub rx_rate: f64,
    pub burst: usize,
    pub head: Option<usize>,
    pub first_offset: i64,
    pub channels: usize,
}

/// The Module's send at `rate` (`segment::send_cap` over the in-flight window).
pub fn chunk_at(rate: f64) -> usize {
    usize::try_from(send_cap(WINDOW_TICKS, MCR / rate as i64, &PACKETS)).expect("a send is a positive count")
}

/// The samples of a `BURST_MS` burst at `rate`.
pub fn burst_samples(rate: f64) -> usize {
    (rate * BURST_MS as f64 / 1_000.0).round() as usize
}

impl Shape {
    /// C1: `CHUNKS` chunks at `rate` both ways, the late burst's first sample timed `LATE_MS` before the reference instant.
    pub fn c1(rate: f64) -> Shape {
        Shape { tx_rate: rate, rx_rate: rate, burst: CHUNKS * chunk_at(rate), head: None, first_offset: -LATE_MS * TICKS_PER_MS, channels: 1 }
    }

    /// C1's trial with a `BURST_MS` burst, transmitted at `tx_rate` and received at `rx_rate`.
    pub fn c2(tx_rate: f64, rx_rate: f64) -> Shape {
        Shape { tx_rate, rx_rate, burst: burst_samples(tx_rate), ..Shape::c1(tx_rate) }
    }

    /// C2's shape at one rate with the late burst's first send of `C3_HEAD` samples.
    pub fn c3(rate: f64) -> Shape {
        Shape { head: Some(C3_HEAD), ..Shape::c2(rate, rate) }
    }

    /// Two channels at 1 MS/s, the late burst's first sample timed `first_offset` ticks from the reference instant.
    pub fn c5(first_offset: i64) -> Shape {
        Shape { first_offset, channels: 2, ..Shape::c2(1e6, 1e6) }
    }
}

/// One trial of the cell: receive from 20 ms on, the control burst at 60 ms, the late burst's first sample timed `shape.first_offset` from the host's
/// reference instant (130 ms; nothing but the control's sends comes between), and the receive read to two chunks and a ms past the late burst's end by a
/// thread of its own, so the receiver never waits on a send. Every report is read at the end. One evidence per channel: its reports, its heard intervals.
fn late_chunked_trial(device: &dyn Device, shape: &Shape, wave: &[Iq], chunk: usize, rate_errors: &[String]) -> Vec<Evidence> {
    let (tx_per, rx_per) = (MCR / shape.tx_rate as i64, MCR / shape.rx_rate as i64);
    let chunk_ticks = chunk as i64 * tx_per;
    let d0 = device.time_now().expect("the device's time");
    let (rx_at, control_at, reference) = (align_to(d0 + 20 * TICKS_PER_MS, rx_per), align_to(d0 + 60 * TICKS_PER_MS, tx_per), align_to(d0 + 130 * TICKS_PER_MS, tx_per));
    let late_at = align_to(reference + shape.first_offset, tx_per);
    let len = wave.len() as i64 * tx_per;
    let (control, late) = ((control_at, control_at + len), (late_at, late_at + len));
    let read_to = late.1 + 2 * chunk_ticks + TICKS_PER_MS;
    let mut errors = rate_errors.to_vec();
    if let Err(error) = device.rx_start(0, rx_at) {
        errors.push(format!("rx_start: {}", error.message));
    }
    let n = (shape.rx_rate / 1_000.0) as usize;
    let control_plan = Plan { first_at: control_at, per: tx_per, chunk, head: chunk, eob: true, each: false, first: false, channels: shape.channels };
    let late_plan = Plan { first_at: late_at, head: shape.head.unwrap_or(chunk), first: shape.channels > 1, ..control_plan };
    let (mut late_last, mut late_first) = (None, None);
    let log = std::thread::scope(|scope| {
        let reader = scope.spawn(|| read_rx(device, rx_per, n.max(1_000), read_to, shape.channels));
        if let Err(error) = send_chunked(device, wave, control_plan) {
            errors.push(format!("control burst: {error}"));
        }
        match wait_until(device, reference) {
            Ok(_) => match send_chunked(device, wave, late_plan) {
                Ok(sent) => {
                    late_last = sent.last().map(|last| (last.handed, last.at));
                    late_first = sent.first().filter(|_| late_plan.first).map(|first| (first.handed, first.at));
                }
                Err(error) => errors.push(format!("late burst: {error}")),
            },
            Err(error) => errors.push(format!("the wait for the late burst: {error}")),
        }
        if let Err(error) = wait_until(device, read_to + TICKS_PER_MS) {
            errors.push(format!("the wait for the receive: {error}"));
        }
        // Polled on the device's clock: a join would hold this thread busy, and a manual clock does not run while a participant is busy.
        while !reader.is_finished() {
            device.host().sleep(Wall::from_millis(1));
        }
        reader.join().expect("the receive thread")
    });
    let reports = read_reports(device, &mut errors);
    if let Err(error) = device.rx_stop(0) {
        errors.push(format!("rx_stop: {}", error.message));
    }
    let _ = drain_without_clock(device, Wall::from_millis(10), Wall::from_millis(300));
    errors.extend(log.errors.iter().cloned());
    (0..shape.channels)
        .map(|channel| Evidence {
            per: rx_per,
            tx_per,
            chunk_ticks,
            late,
            control,
            errors: errors.clone(),
            blocks: log.blocks.clone(),
            other: log.other.clone(),
            heard: tone_intervals(&log.samples[channel], rx_per, TONE_HZ),
            reports: reports.iter().filter(|r| r.channel == channel).copied().collect(),
            late_last,
            late_first,
        })
        .collect()
}

/// One trial of a cell: its transmit rate, its number and what it read (one evidence per channel).
pub struct Trial {
    pub rate: f64,
    pub trial: usize,
    pub evidence: Vec<Evidence>,
}

impl Trial {
    /// The trial's verdict: every channel's `late_chunked_verdict`, a FAIL in any is the trial's.
    pub fn verdict(&self) -> Verdict {
        iii::all_of(self.evidence.iter().map(late_chunked_verdict).collect())
    }

    /// The trial as the bench logs it, one object a channel: the intervals, the receive coverage and the reports by code, so a verdict can be read back from the log.
    pub fn trace(&self) -> Json {
        Json::Array(self.evidence.iter().enumerate().map(|(channel, e)| self.channel_trace(channel, e)).collect())
    }

    fn channel_trace(&self, channel: usize, e: &Evidence) -> Json {
        let count = |code: TxCode| e.reports.iter().filter(|r| r.code == code).count();
        json!({
            "rate_hz": self.rate, "trial": self.trial, "channel": channel, "per_ticks": e.tx_per, "rx_per_ticks": e.per, "chunk_ticks": e.chunk_ticks, "late": [e.late.0, e.late.1],
            "control": [e.control.0, e.control.1],
            "control_delay_ticks": control_delay(e), "late_last_handed_and_own_time": e.late_last, "late_first_handed_and_own_time": e.late_first, "errors": e.errors,
            "blocks": e.blocks.len(), "blocks_first": e.blocks.first(), "blocks_end": e.blocks.last().map(|(first, len)| first + *len as i64 * e.per),
            "other_positions": e.other, "heard": e.heard.iter().map(|iv| [iv.first_tick, iv.last_tick, iv.samples as i64]).collect::<Vec<_>>(),
            "reports": { "BurstAck": count(TxCode::BurstAck), "TimeError": count(TxCode::TimeError), "Underflow": count(TxCode::Underflow) + count(TxCode::UnderflowInPacket) },
            "late_time_errors": e.reports.iter().filter(|r| r.code == TxCode::TimeError && r.tick.is_some_and(|tick| tick >= e.late.0)).count(),
            "acks_at_or_after_late": e.reports.iter().filter(|r| r.code == TxCode::BurstAck && r.tick.is_some_and(|tick| tick >= e.late.0)).count(),
        })
    }
}

/// The device channels a shape transmits and receives on: slot A's (`d9_channel`) alone, or both slots.
fn shape_channels(shape: &Shape) -> Vec<usize> {
    if shape.channels == 1 { vec![d9_channel()] } else { (0..shape.channels).collect() }
}

/// What a channel's `apply` read back, against what was asked: an error text for each rate that is not the requested one (the trial is then INCONCLUSIVE; the
/// RF bounds `raw_tx_on` asserts are the same whatever rate the device settled on).
pub fn rate_errors(what: &str, want: f64, applied: &[ezsdr_radio_uhd::Applied]) -> Vec<String> {
    applied.iter().enumerate().filter(|(_, a)| a.rate != want).map(|(channel, a)| format!("the {what} rate of channel {channel} is {} S/s, not the requested {want} S/s", a.rate)).collect()
}

/// A tone burst of `n` samples at `rate` as the device takes it (`TONE_HZ` from the carrier, amplitude 0.4: `tone_wave` asserts its bound).
pub fn tone_burst(rate: f64, n: usize) -> Vec<Iq> {
    iq(&tone_wave(rate, n, TONE_HZ))
}

/// Opens a one-channel transmit stream at `rate` inside bench.md's RF envelope (`raw_tx_on`), prints the rate it read back, and returns the errors for a rate that is not
/// the requested one (C4 and C6 carry them into every trial).
pub fn open_tx_checked(device: &dyn Device, rate: f64) -> Vec<String> {
    let applied = raw_tx_on(device, &[d9_channel()], rate, 1e9);
    println!("MT3 rates: transmit {:?} S/s (requested {rate})", applied.iter().map(|a| a.rate).collect::<Vec<_>>());
    rate_errors("transmit", rate, &applied)
}

/// The cell for `shape` on `device`, which opens its streamers (1 GHz, 0 dB, `raw_tx_on` refuses anything outside bench.md's RF envelope), prints the rates they
/// read back, and closes them again. The caller is the only driver of the device's clock while it runs.
pub fn late_chunked_cell(device: &dyn Device, shape: Shape, trials: usize) -> Vec<Trial> {
    let _driver = device.host().drive();
    let channels = shape_channels(&shape);
    let rx = raw_rx_on(device, &channels, shape.rx_rate, 1e9);
    let tx = raw_tx_on(device, &channels, shape.tx_rate, 1e9);
    println!(
        "MT3 rates: transmit {:?} receive {:?} S/s (requested {} and {})",
        tx.iter().map(|a| a.rate).collect::<Vec<_>>(),
        rx.iter().map(|a| a.rate).collect::<Vec<_>>(),
        shape.tx_rate,
        shape.rx_rate
    );
    let errors: Vec<String> = [rate_errors("transmit", shape.tx_rate, &tx), rate_errors("receive", shape.rx_rate, &rx)].concat();
    let chunk = chunk_at(shape.tx_rate);
    let wave = tone_burst(shape.tx_rate, shape.burst);
    let out = (0..trials).map(|trial| Trial { rate: shape.tx_rate, trial, evidence: late_chunked_trial(device, &shape, &wave, chunk, &errors) }).collect();
    device.rx_close(0);
    device.tx_close(0);
    out
}

/// The cell's verdict: PASS only if every trial passes, FAIL if any fails, else INCONCLUSIVE; no trial is INCONCLUSIVE.
pub fn cell_verdict(trials: &[Trial]) -> Verdict {
    cell_of(trials.iter().map(Trial::verdict).collect())
}

/// A cell's verdict from its trials' (C1-C3, C6): PASS only if every trial passes, FAIL if any fails, else INCONCLUSIVE; no trial is INCONCLUSIVE.
pub fn cell_of(verdicts: Vec<Verdict>) -> Verdict {
    if verdicts.is_empty() {
        return Verdict::Inconclusive("no trial was run".to_owned());
    }
    iii::all_of(verdicts)
}

/// The ticks of the TimeErrors a late burst drew, in report order (the control's, before the late burst, are left out).
pub fn late_ticks(e: &Evidence) -> Vec<i64> {
    e.reports.iter().filter(|r| r.code == TxCode::TimeError).filter_map(|r| r.tick).filter(|tick| *tick >= e.late.0).collect()
}

/// What the late reports' ticks show (C2's observation, C3's span): an observation, never a verdict term. The FPGA's error FIFO holds 32 entries, so it can drop reports
/// (R7): `skipped` lists the gaps that are a whole multiple (two or more) of the modal gap, which a report that went missing would make.
#[derive(Debug, PartialEq)]
pub struct Gaps {
    pub ticks: Vec<i64>,
    pub gaps: Vec<i64>,
    /// The commonest gap, the smaller on a tie.
    pub modal: Option<i64>,
    /// The indexes into `gaps` of the multiples of the modal gap.
    pub skipped: Vec<usize>,
    /// The first report's tick to the last's: it omits the last packet's own drain (the DUC consumes it after it is reported).
    pub partial_span: Option<i64>,
}

pub fn gaps_of(ticks: &[i64]) -> Gaps {
    let gaps: Vec<i64> = ticks.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let modal = gaps.iter().map(|gap| (gaps.iter().filter(|other| *other == gap).count(), -gap)).max().map(|(_, negated)| -negated);
    let skipped = gaps.iter().enumerate().filter(|(_, gap)| modal.is_some_and(|modal| modal > 0 && **gap >= 2 * modal && **gap % modal == 0)).map(|(index, _)| index).collect();
    Gaps { ticks: ticks.to_vec(), gaps, modal, skipped, partial_span: ticks.first().zip(ticks.last()).map(|(first, last)| last - first) }
}

/// The log line of a late trial's report ticks: all of them in order, the consecutive gaps, the modal gap in samples (INFERRED packet length: the Device exposes no
/// transmit packet size, and the gaps are not an independent measure of it), the multiples of it and the partial span against the burst's own length.
pub fn gaps_line(cell: &str, rate: f64, trial: usize, g: &Gaps, tx_per: i64, burst_ticks: i64) -> String {
    let modal = g
        .modal
        .map_or("none".to_owned(), |modal| format!("{modal} ticks = {} samples + {} ticks (INFERRED packet length: the Device exposes no transmit packet size)", modal / tx_per, modal % tx_per));
    let span = g
        .partial_span
        .map_or("none".to_owned(), |span| format!("{span} ticks against S x per {burst_ticks} ticks, difference {} (a partial span: it omits the last packet's own drain)", span - burst_ticks));
    format!(
        "MT3 {cell} gaps {} MS/s trial {trial}: {} late reports, ticks {:?}, gaps {:?}, modal gap {modal}, skipped-report gaps (a multiple of the modal) at {:?}, span {span}",
        rate / 1e6,
        g.ticks.len(),
        g.ticks,
        g.gaps,
        g.skipped
    )
}

/// Runs the cell and prints its lines: `MT3 <name> trace`, `VERDICT mt3 <name>` a trial, and when `with_gaps` `MT3 <name> gaps` a late trial, then `MT3 <name> cell`.
/// `Some(text)` when the cell is not PASS.
pub fn report_cell(device: &dyn Device, name: &str, shape: Shape, trials: usize, with_gaps: bool) -> Option<String> {
    let rate = shape.tx_rate / 1e6;
    let trials = late_chunked_cell(device, shape, trials);
    for trial in &trials {
        println!("MT3 {name} trace {}", trial.trace());
        println!("VERDICT mt3 {} {rate} MS/s trial {}: {:?}", name.to_lowercase(), trial.trial, trial.verdict());
        for e in trial.evidence.iter().filter(|_| with_gaps) {
            println!("{}", gaps_line(name, trial.rate, trial.trial, &gaps_of(&late_ticks(e)), e.tx_per, e.late.1 - e.late.0));
        }
    }
    let verdict = cell_verdict(&trials);
    println!("MT3 {name} cell {rate} MS/s: {verdict:?}");
    (!matches!(verdict, Verdict::Pass(_))).then(|| format!("{rate} MS/s: {verdict:?}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const PER: i64 = 200;
    const CHUNK: i64 = 500_000;
    const DELAY: i64 = 44 * PER;
    /// The delay the verdict reads off the control: its onset is 5 samples into the first tone window.
    const MEASURED: i64 = DELAY + 5 * PER;

    /// A report of channel 0 (the other cells' unit tests use it too).
    pub(crate) fn report(code: TxCode, tick: Option<i64>) -> TxReport {
        TxReport { code, tick, channel: 0 }
    }

    /// 1 MS/s, 2 500-sample chunks, 8 per burst: a control at 10 000 000 and a late burst at 30 000 000, 4 000 000 ticks (20 000 samples) each, the loopback
    /// 44 samples late, the receive from 2 000 000 to 40 000 000 in 1 ms blocks, the control heard, nothing else heard, the control's BurstAck and two TimeErrors.
    pub(crate) fn passing() -> Evidence {
        let blocks = (0..190).map(|k| (2_000_000 + k * 200_000, 1_000)).collect();
        let control = (10_000_000, 14_000_000);
        Evidence {
            per: PER,
            tx_per: PER,
            chunk_ticks: CHUNK,
            late: (30_000_000, 34_000_000),
            control,
            errors: Vec::new(),
            blocks,
            other: Vec::new(),
            heard: vec![HeardInterval { first_tick: control.0 + DELAY + 5 * PER, last_tick: control.1 + DELAY - 5 * PER, samples: 19_990 }],
            reports: vec![report(TxCode::BurstAck, Some(14_000_000)), report(TxCode::TimeError, Some(30_000_000)), report(TxCode::TimeError, Some(30_500_000))],
            late_last: Some((31_000_000, 33_500_000)),
            late_first: None,
        }
    }

    fn assert_kind(e: &Evidence, want: &str, what: &str) {
        late_chunked_verdict(e).assert_kind(want, what);
    }

    /// A heard interval of `samples` samples starting at `first`.
    fn heard(first: i64, samples: i64) -> HeardInterval {
        HeardInterval { first_tick: first, last_tick: first + (samples - 1) * PER, samples: samples as usize }
    }

    #[test]
    fn mt3_c1_verdict_passes_with_complete_evidence() {
        assert_kind(&passing(), "PASS", "complete coverage, the control heard, nothing heard in the late interval, no BurstAck for it, a TimeError");
    }

    #[test]
    fn mt3_c1_verdict_absent_receive_is_inconclusive_not_pass() {
        // The control and everything else as in the passing case: only the receive evidence is missing.
        let mut no_blocks = passing();
        no_blocks.blocks.clear();
        assert_kind(&no_blocks, "INCONCLUSIVE", "no receive block at all");
        let mut failed = passing();
        failed.errors = vec!["rx_start: the stream is not open".to_owned()];
        assert_kind(&failed, "INCONCLUSIVE", "rx_start failed");
    }

    #[test]
    fn mt3_c1_verdict_incomplete_receive_is_inconclusive() {
        let late = passing().late;
        let mut gap = passing();
        gap.blocks.retain(|(first, _)| *first != 32_000_000);
        assert_eq!(gap.blocks.len(), 189);
        assert_kind(&gap, "INCONCLUSIVE", "a block inside the interval is missing");
        let mut ends_early = passing();
        ends_early.blocks.truncate(160);
        assert_kind(&ends_early, "INCONCLUSIVE", "the blocks end (at 34 000 000) before the interval's end and its margin");
        let mut no_margin_after = passing();
        no_margin_after.blocks.retain(|(first, _)| *first + 200_000 < late.1 + MEASURED + CHUNK);
        assert_kind(&no_margin_after, "INCONCLUSIVE", "the margin after the interval is missing by under one block");
        let mut no_margin_before = passing();
        no_margin_before.blocks.retain(|(first, _)| *first > late.0 + MEASURED - CHUNK);
        assert_kind(&no_margin_before, "INCONCLUSIVE", "the margin before the interval is missing by under one block");
        let mut exactly = passing();
        exactly.blocks.clear();
        let mut at = late.0 + MEASURED - CHUNK;
        while at < late.1 + MEASURED + CHUNK {
            exactly.blocks.push((at, 1_000));
            at += 200_000;
        }
        assert_kind(&exactly, "PASS", "blocks that cover the range exactly, the control heard");
    }

    #[test]
    fn mt3_c1_verdict_without_the_control_is_inconclusive() {
        let mut not_heard = passing();
        not_heard.heard.clear();
        assert_kind(&not_heard, "INCONCLUSIVE", "the control was not heard");
        let mut half = passing();
        half.heard = vec![heard(10_000_000 + DELAY, 10_000)];
        assert_kind(&half, "INCONCLUSIVE", "half of the control was heard");
        let mut late_control = passing();
        late_control.reports.push(report(TxCode::TimeError, Some(11_000_000)));
        assert_kind(&late_control, "INCONCLUSIVE", "the control itself drew a TimeError");
    }

    #[test]
    fn mt3_c1_verdict_a_receive_outcome_inside_the_range_is_inconclusive() {
        let mut inside = passing();
        inside.other = vec![Some(31_000_000)];
        assert_kind(&inside, "INCONCLUSIVE", "an overflow in the interval");
        let mut margin = passing();
        margin.other = vec![Some(30_000_000 - CHUNK + MEASURED)];
        assert_kind(&margin, "INCONCLUSIVE", "an overflow at the margin's first tick");
        let mut outside = passing();
        outside.other = vec![None, Some(3_000_000), Some(39_000_000)];
        assert_kind(&outside, "PASS", "outcomes before the first block, before the margin and after it");
    }

    #[test]
    fn mt3_c1_verdict_a_heard_interval_inside_the_late_interval_fails() {
        let mut inside = passing();
        inside.heard.push(heard(30_000_000 + DELAY + 500_000, 2_500));
        assert_kind(&inside, "FAIL", "the first chunk of the late burst was heard");
        let mut incomplete = inside.clone();
        incomplete.blocks.truncate(160);
        assert_kind(&incomplete, "FAIL", "partial transmission is positive evidence whatever the coverage");
        let mut no_control = inside.clone();
        no_control.heard.remove(0);
        assert_kind(&no_control, "FAIL", "and whatever the control");
    }

    #[test]
    fn mt3_c1_verdict_a_heard_interval_that_straddles_the_late_interval_fails() {
        let late = passing().late;
        // Starts 10 samples before the late burst's first sample (the delay taken off) and runs 200 samples into it.
        let mut from_before = passing();
        from_before.heard.push(heard(late.0 + DELAY - 10 * PER, 210));
        assert_kind(&from_before, "FAIL", "a heard interval over the start of the late interval");
        // Starts 100 samples before its end and runs past it.
        let mut past_the_end = passing();
        past_the_end.heard.push(heard(late.1 + DELAY - 100 * PER, 300));
        assert_kind(&past_the_end, "FAIL", "a heard interval over the end of the late interval");
        // Ends exactly where the late interval begins: touching is not overlapping.
        let mut touching = passing();
        touching.heard.push(heard(late.0 + MEASURED - 100 * PER, 100));
        assert_kind(&touching, "PASS", "a heard interval that ends where the late interval starts");
        // The loopback delay is taken off: a tone that arrives before the delayed start is not the late burst's.
        let mut before_the_delay = passing();
        before_the_delay.heard.push(heard(late.0 + 5 * PER, 20));
        assert_kind(&before_the_delay, "PASS", "a tone inside the delay after the late interval's first tick");
    }

    #[test]
    fn mt3_c1_verdict_a_burst_ack_attributed_by_tick() {
        let mut late_ack = passing();
        late_ack.reports.push(report(TxCode::BurstAck, Some(34_000_000)));
        assert_kind(&late_ack, "FAIL", "a BurstAck at the late burst's end");
        let mut first_sample = passing();
        first_sample.reports.push(report(TxCode::BurstAck, Some(30_000_000)));
        assert_kind(&first_sample, "FAIL", "a BurstAck at the late burst's first tick");
        let mut incomplete = late_ack.clone();
        incomplete.blocks.clear();
        assert_kind(&incomplete, "FAIL", "a BurstAck fails whatever the coverage");
        // The passing case already carries the control's BurstAck (tick 14 000 000): it is the control's by its tick.
        let mut before = passing();
        before.reports.push(report(TxCode::BurstAck, Some(29_999_999)));
        assert_kind(&before, "PASS", "a BurstAck one tick before the late burst's first tick is the control's");
        let mut tickless = passing();
        tickless.reports.push(report(TxCode::BurstAck, None));
        assert_kind(&tickless, "INCONCLUSIVE", "a BurstAck without a tick cannot be attributed");
    }

    #[test]
    fn mt3_c1_verdict_a_last_chunk_handed_after_its_own_time_is_inconclusive() {
        // The late burst's last chunk is timed 33 500 000; a device that drops late packets one by one drops only the chunks that are late at their own time.
        let mut after = passing();
        after.late_last = Some((33_500_001, 33_500_000));
        assert_kind(&after, "INCONCLUSIVE", "the last chunk was handed after its own time: silence could be per-packet drops of the early chunks");
        let mut at = passing();
        at.late_last = Some((33_500_000, 33_500_000));
        assert_kind(&at, "INCONCLUSIVE", "handed at its own time is not 1 ms before it");
        let mut short = passing();
        short.late_last = Some((33_300_001, 33_500_000));
        assert_kind(&short, "INCONCLUSIVE", "handed 199 999 ticks (under 1 ms) before its own time");
        let mut before = passing();
        before.late_last = Some((33_300_000, 33_500_000));
        assert_kind(&before, "PASS", "handed exactly 1 ms (200 000 ticks) before its own time");
        let mut none = passing();
        none.late_last = None;
        assert_kind(&none, "INCONCLUSIVE", "the last send was not handed");
        let mut played = after.clone();
        played.heard.push(heard(31_000_000 + MEASURED, 2_500));
        assert_kind(&played, "FAIL", "what was heard still fails, whenever the last chunk was handed");
    }

    #[test]
    fn mt3_c1_verdict_without_a_time_error_is_inconclusive() {
        let mut none = passing();
        none.reports.retain(|r| r.code != TxCode::TimeError);
        assert_kind(&none, "INCONCLUSIVE", "no TimeError: the burst was not late");
        let mut control_only = passing();
        control_only.reports.retain(|r| r.code != TxCode::TimeError);
        control_only.reports.push(report(TxCode::TimeError, Some(11_000_000)));
        assert_kind(&control_only, "INCONCLUSIVE", "a TimeError before the late burst is the control's, not the late burst's");
        let mut tickless = passing();
        tickless.reports.retain(|r| r.code != TxCode::TimeError);
        tickless.reports.push(report(TxCode::TimeError, None));
        assert_kind(&tickless, "INCONCLUSIVE", "a TimeError without a tick is not evidence of the late burst");
    }

    /// C2's passing case: transmitted at 100 MS/s (2 ticks a sample), received at 10 MS/s (20 ticks a sample), 20 ms (4 000 000 ticks) bursts in 20 000-sample chunks
    /// (40 000 ticks), the loopback 44 receive samples late, the receive in blocks of 1 000 samples (20 000 ticks), everything else as `passing`.
    fn passing_split() -> Evidence {
        let (per, delay) = (20, 44 * 20);
        let control = (10_000_000, 14_000_000);
        Evidence {
            per,
            tx_per: 2,
            chunk_ticks: 40_000,
            late: (30_000_000, 34_000_000),
            control,
            errors: Vec::new(),
            blocks: (0..1_900).map(|k| (2_000_000 + k * 20_000, 1_000)).collect(),
            other: Vec::new(),
            heard: vec![HeardInterval { first_tick: control.0 + delay + 5 * per, last_tick: control.1 + delay - 5 * per, samples: 199_990 }],
            reports: vec![report(TxCode::BurstAck, Some(14_000_000)), report(TxCode::TimeError, Some(30_000_000)), report(TxCode::TimeError, Some(30_040_000))],
            late_last: Some((31_000_000, 33_960_000)),
            late_first: None,
        }
    }

    #[test]
    fn mt3_c2_verdict_reads_the_receive_axis_at_the_receive_spacing() {
        // Transmit and receive spacings differ by 10: the control's 199 990 heard samples are 99.995 % of the 200 000 receive samples of its interval (at the transmit
        // spacing the interval would hold 2 000 000), and the blocks that follow one another at 20 ticks a sample cover the late interval without a gap.
        assert_kind(&passing_split(), "PASS", "complete evidence at 10 MS/s receive and 100 MS/s transmit");
        let mut half = passing_split();
        half.heard[0].samples = 100_000;
        assert_kind(&half, "INCONCLUSIVE", "half of the control was heard");
        let mut late_tone = passing_split();
        late_tone.heard.push(heard_at(30_000_000 + 44 * 20, 2_500, 20));
        assert_kind(&late_tone, "FAIL", "a tone inside the late interval, on the receive axis");
    }

    /// A heard interval of `samples` samples starting at `first`, `per` ticks apart.
    fn heard_at(first: i64, samples: i64, per: i64) -> HeardInterval {
        HeardInterval { first_tick: first, last_tick: first + (samples - 1) * per, samples: samples as usize }
    }

    #[test]
    fn mt3_c2_shapes_state_the_burst_the_rates_and_the_first_send() {
        // 20 ms at every rate: 20 000 samples at 1 MS/s (8 chunks of 2 500, C1's own), 200 000 at 10 MS/s, 2 and 4 million at 100 and 200 MS/s; the Module's send is at most 20 000.
        assert_eq!([1e6, 10e6, 100e6, 200e6].map(burst_samples), [20_000, 200_000, 2_000_000, 4_000_000]);
        assert_eq!([1e6, 10e6, 100e6, 200e6].map(chunk_at), [2_500, 20_000, 20_000, 20_000]);
        // C1 stays as it was: 8 chunks, 16 ms at 10 MS/s, 1 ms late, one channel, no short head.
        let c1 = Shape::c1(10e6);
        assert_eq!((c1.burst, c1.first_offset, c1.channels, c1.head, c1.rx_rate), (160_000, -TICKS_PER_MS, 1, None, 10e6));
        let c2 = Shape::c2(200e6, 10e6);
        assert_eq!((c2.tx_rate, c2.rx_rate, c2.burst, c2.first_offset, c2.head), (200e6, 10e6, 4_000_000, -TICKS_PER_MS, None));
        let c3 = Shape::c3(10e6);
        assert_eq!((c3.tx_rate, c3.rx_rate, c3.burst, c3.head), (10e6, 10e6, 200_000, Some(32)));
        let c5 = Shape::c5(-40_000);
        assert_eq!((c5.channels, c5.first_offset, c5.tx_rate, c5.burst), (2, -40_000, 1e6, 20_000));
    }

    #[test]
    fn mt3_c2_a_rate_that_is_not_the_requested_one_is_an_error() {
        let applied = |rate: f64| ezsdr_radio_uhd::Applied { rate, freq: 1e9, gain: 0.0, antenna: None };
        assert!(rate_errors("transmit", 100e6, &[applied(100e6)]).is_empty());
        let errors = rate_errors("transmit", 100e6, &[applied(100e6), applied(50e6)]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("channel 1") && errors[0].contains("transmit"), "{errors:?}");
        let mut trial = passing_split();
        trial.errors = errors;
        assert_kind(&trial, "INCONCLUSIVE", "a trial that carries a rate error");
    }

    #[test]
    fn mt3_c2_late_ticks_are_the_time_errors_of_the_late_burst_in_order() {
        let mut e = passing_split();
        e.reports = vec![
            report(TxCode::TimeError, Some(11_000_000)),
            report(TxCode::BurstAck, Some(14_000_000)),
            report(TxCode::TimeError, Some(30_000_000)),
            report(TxCode::Underflow, Some(31_000_000)),
            report(TxCode::TimeError, None),
            report(TxCode::TimeError, Some(30_040_000)),
        ];
        assert_eq!(late_ticks(&e), vec![30_000_000, 30_040_000], "the control's TimeError, the ACK, the Underflow and a tickless report are not late ticks");
    }

    #[test]
    fn mt3_c2_gaps_find_the_modal_gap_and_the_skipped_reports() {
        // Packets of 10 ticks and 2 clocks of overhead: gaps of 12, a missing report makes one of 24.
        let g = gaps_of(&[100, 112, 124, 148, 160, 172]);
        assert_eq!((g.gaps.clone(), g.modal, g.skipped.clone(), g.partial_span), (vec![12, 12, 24, 12, 12], Some(12), vec![2], Some(72)));
        // A gap that is not a whole multiple is not a skipped report; a tie takes the smaller gap; one tick has no gap and an empty span of 0; none has nothing.
        assert!(gaps_of(&[0, 12, 24, 54, 66, 78]).skipped.is_empty(), "a gap of 30 against a modal 12 is not a whole multiple");
        assert_eq!(gaps_of(&[0, 5, 15]).modal, Some(5));
        assert_eq!(gaps_of(&[7]), Gaps { ticks: vec![7], gaps: vec![], modal: None, skipped: vec![], partial_span: Some(0) });
        assert_eq!(gaps_of(&[]).partial_span, None);
        let line = gaps_line("C2", 100e6, 3, &g, 2, 4_000_000);
        assert!(line.starts_with("MT3 C2 gaps 100 MS/s trial 3: 6 late reports, ticks [100, 112, 124, 148, 160, 172]"), "{line}");
        assert!(line.contains("modal gap 12 ticks = 6 samples + 0 ticks (INFERRED packet length"), "{line}");
        assert!(line.contains("at [2]") && line.contains("span 72 ticks against S x per 4000000 ticks, difference -3999928 (a partial span"), "{line}");
    }

    #[test]
    fn mt3_c1_the_tone_detector_hears_a_tone_and_neither_noise_nor_a_ramp() {
        let per = 200;
        let tone = |from: usize, n: usize, amplitude: f32| -> Vec<(i64, Iq)> {
            (from..from + n)
                .map(|i| {
                    let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6);
                    (i as i64 * per, [amplitude * phase.cos() as f32, amplitude * phase.sin() as f32])
                })
                .collect()
        };
        // A tone at the loopback's level over 100 samples hears as one interval of four windows.
        assert_eq!(tone_intervals(&tone(0, 100, 0.01), per, 100e3), vec![HeardInterval { first_tick: 0, last_tick: 99 * per, samples: 100 }]);
        // Below the amplitude floor it is not heard (about 1 LSB).
        assert!(tone_intervals(&tone(0, 100, 0.00003), per, 100e3).is_empty());
        // At another frequency it is not heard.
        assert!(tone_intervals(&tone(0, 100, 0.01), per, 300e3).is_empty());
        // The fake's ramp (real, rising, at 0 Hz) is coherent and loud but not the tone.
        let ramp: Vec<(i64, Iq)> = (0..100).map(|i| (i * per, [(i as f32 * 100.0 % 65_536.0) / 65_536.0 + 0.1, 0.0])).collect();
        assert!(tone_intervals(&ramp, per, 100e3).is_empty());
        // A tone whose phase jitters (coherence about 0.7, at the right frequency and level) is not the tone.
        let jittery: Vec<(i64, Iq)> = (0..100)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * 100e3 * (i as f64 / 1e6) + 0.4 * if i % 2 == 0 { 1.0 } else { -1.0 };
                (i as i64 * per, [0.01 * phase.cos() as f32, 0.01 * phase.sin() as f32])
            })
            .collect();
        assert!(tone_intervals(&jittery, per, 100e3).is_empty());
        // A hole in the ticks breaks the window.
        let mut holed = tone(0, 100, 0.01);
        holed.iter_mut().skip(40).for_each(|(tick, _)| *tick += per);
        assert_eq!(tone_intervals(&holed, per, 100e3).len(), 2, "window 0, then windows 2 and 3 after the hole: window 1 straddles it");
    }
}
