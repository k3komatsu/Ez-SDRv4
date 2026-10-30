//! Stage 3c-3a: throughput conditions over a layout's channels and streams (design 41 §8, UR-34): the
//! stage list of a direction, one stage's Run with per-stream evidence, the capacity over the conditions,
//! and the Device wrapper that counts what each stream moved.

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use ezsdr_kernel::module_api::Fidelity;
use ezsdr_radio_uhd::{Applied, DeviceError, HostClock, Iq, TxCode, TxReport};

use super::*;
use super::super as common;

const NO_STREAM: usize = usize::MAX;
const TEST: &str = "tests/hardware.rs::hw_measurements_record";

/// Faults a rehearsal injects into the counting wrapper of this thread's throughput stages (stage 3c-3a tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct InjectedFault {
    /// `rx_recv` of this stream only times out: one stream starves.
    pub starve_rx: Option<usize>,
    /// `tx_send` of this stream accepts one sample fewer than it was given: a partial send.
    pub short_tx: Option<usize>,
    /// This stream's `BurstAck` never reaches the Provider.
    pub swallow_ack: Option<usize>,
    /// The `BurstAck` of this channel (of any stream) never reaches the Provider: one channel of a stream is unacknowledged.
    pub swallow_ack_channel: Option<usize>,
}

thread_local! {
    static FAULT: std::cell::Cell<InjectedFault> = const { std::cell::Cell::new(InjectedFault { starve_rx: None, short_tx: None, swallow_ack: None, swallow_ack_channel: None }) };
}

/// Runs `body` with `fault` injected into the stages it measures, and removes it again.
pub fn with_injected_fault<T>(fault: InjectedFault, body: impl FnOnce() -> T) -> T {
    struct Remove;
    impl Drop for Remove {
        fn drop(&mut self) { FAULT.with(|cell| cell.set(InjectedFault::default())); }
    }
    FAULT.with(|cell| cell.set(fault));
    let _remove = Remove;
    body()
}

/// The record's latch (stage 3c-3a): once a device call fails in a way that makes the handle unsafe to touch again, the
/// first reason stays here and every later call through a `GuardedDevice` is refused without reaching the device.
#[derive(Default)]
pub struct Latch(Mutex<Option<String>>);

impl Latch {
    pub fn new() -> Arc<Latch> { Arc::new(Latch::default()) }
    pub fn reason(&self) -> Option<String> { self.0.lock().unwrap().clone() }
    /// Records `reason` unless the latch already holds one.
    pub fn set(&self, reason: impl Into<String>) {
        let mut held = self.0.lock().unwrap();
        if held.is_none() { *held = Some(reason.into()); }
    }
}

thread_local! {
    static RECORD_LATCH: RefCell<Option<Arc<Latch>>> = const { RefCell::new(None) };
}

/// Installs (or removes) the latch of the record this thread runs: its Provider groups and raw probes open their devices
/// through `guard_if_recording`, which wraps each in a `GuardedDevice` on this latch.
pub fn install_record_latch(latch: Option<Arc<Latch>>) {
    RECORD_LATCH.with(|cell| *cell.borrow_mut() = latch);
}

/// Why the record's latch holds, once it does.
pub fn record_latch_reason() -> Option<String> {
    RECORD_LATCH.with(|cell| cell.borrow().as_ref().and_then(|latch| latch.reason()))
}

/// Whether `message` is UHD's control-path timeout.
pub fn is_control_timeout(message: &str) -> bool {
    message.contains("Control operation timed out")
}

/// A forwarding `Device` that latches on any `apply` error, on a control timeout or a device loss from any call, on a
/// streamer left open (`latch_if_streamers_open`) and on an open of channels another open stream still holds; after the
/// latch it answers every call that would reach the device with an error (or nothing) instead of forwarding it, and when
/// dropped latched it leaks the inner handle, so no teardown or time read reaches it, unwinding included. Its description,
/// master clock and fidelity are read once when it is made, so that none of them reaches the device later either.
pub struct GuardedDevice {
    inner: Option<Arc<dyn Device>>,
    latch: Arc<Latch>,
    open: Mutex<BTreeMap<(bool, usize), Vec<usize>>>,
    described: Json,
    master_clock: u64,
    fidelity: Fidelity,
}

impl GuardedDevice {
    pub fn new(inner: Arc<dyn Device>, latch: Arc<Latch>) -> GuardedDevice {
        GuardedDevice {
            described: inner.describe(), master_clock: inner.master_clock_rate(), fidelity: inner.fidelity(),
            inner: Some(inner), latch, open: Mutex::new(BTreeMap::new()),
        }
    }

    /// A guard on the installed record latch (a fresh one when none is installed).
    pub fn recording(inner: Arc<dyn Device>) -> GuardedDevice {
        GuardedDevice::new(inner, RECORD_LATCH.with(|cell| cell.borrow().clone()).unwrap_or_default())
    }

    fn inner(&self) -> &Arc<dyn Device> { self.inner.as_ref().expect("the guard holds its device until dropped") }

    pub fn latched(&self) -> Option<String> { self.latch.reason() }

    /// Latches when a streamer of this device is still open at the end of a group (UHD aborts on a second open of a front end).
    pub fn latch_if_streamers_open(&self, after: &str) -> Option<String> {
        let open: Vec<String> = self.open.lock().unwrap().keys().map(|(rx, k)| format!("{}{k}", if *rx { "rx" } else { "tx" })).collect();
        if !open.is_empty() { self.latch.set(format!("{after} left streamers open: {open:?}")); }
        self.latched()
    }

    fn refusal(&self) -> Option<DeviceError> {
        self.latch.reason().map(|reason| DeviceError::failed(format!("latched ({reason}); nothing more reaches the device")))
    }

    /// An open whose channels another open stream of the same direction still holds is refused before it reaches the device, and
    /// latches (UHD aborts on a front end opened by a second stream while the first holds it). A re-open of the same stream replaces
    /// its entry and is forwarded: `UhdDevice` drops the stream's old streamer first. An empty channel list is channel 0.
    fn open_stream(&self, rx: bool, stream: usize, channels: &[usize], open: impl FnOnce() -> Result<(), DeviceError>) -> Result<(), DeviceError> {
        if let Some(error) = self.refusal() { return Err(error); }
        let wanted: Vec<usize> = if channels.is_empty() { vec![0] } else { channels.to_vec() };
        let held = self.open.lock().unwrap().iter()
            .find(|((open_rx, open_stream), held)| *open_rx == rx && *open_stream != stream && held.iter().any(|chan| wanted.contains(chan)))
            .map(|((_, open_stream), held)| (*open_stream, held.clone()));
        if let Some((other, held)) = held {
            let name = if rx { "rx" } else { "tx" };
            self.latch.set(format!("{name}{stream} {wanted:?} was opened while {name}{other} still held {held:?}"));
            return Err(self.refusal().expect("the latch was just set"));
        }
        let result = self.note(open(), false);
        if result.is_ok() { self.open.lock().unwrap().insert((rx, stream), wanted); }
        result
    }

    /// Passes `result` on, latching when it is a failure the latch is for: any error of an `apply`, a control timeout or a loss of any call.
    pub fn note<T>(&self, result: Result<T, DeviceError>, apply: bool) -> Result<T, DeviceError> {
        if let Err(error) = &result {
            if apply || error.lost || is_control_timeout(&error.message) { self.latch.set(format!("{}: {}", if apply { "apply failed" } else { "device call failed" }, error.message)); }
        }
        result
    }
}

impl Drop for GuardedDevice {
    fn drop(&mut self) {
        // The handle of a latched record is leaked: UHD's teardown reads the device's time and touches its control port.
        if self.latch.reason().is_some() { std::mem::forget(self.inner.take()); }
    }
}

macro_rules! refuse_or {
    ($self:ident, $call:expr) => {
        match $self.refusal() { Some(error) => Err(error), None => $self.note($call, false) }
    };
}

impl Device for GuardedDevice {
    fn describe(&self) -> Json { self.described.clone() }
    fn fidelity(&self) -> Fidelity { self.fidelity }
    fn master_clock_rate(&self) -> u64 { self.master_clock }
    fn channels(&self, dir: Dir) -> usize { if self.refusal().is_some() { 0 } else { self.inner().channels(dir) } }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> { refuse_or!(self, self.inner().front_end(dir, chan)) }
    fn serial(&self) -> Result<String, DeviceError> { refuse_or!(self, self.inner().serial()) }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> { refuse_or!(self, self.inner().set_subdev_spec(dir, markup)) }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> { refuse_or!(self, self.inner().rates(dir, chan)) }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> { refuse_or!(self, self.inner().set_sources(clock, time)) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> { refuse_or!(self, self.inner().set_time_zero(at_next_pps)) }
    fn time_now(&self) -> Result<i64, DeviceError> { refuse_or!(self, self.inner().time_now()) }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> { refuse_or!(self, self.inner().ref_locked()) }
    fn apply(&self, dir: Dir, chan: usize, settings: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        match self.refusal() { Some(error) => Err(error), None => self.note(self.inner().apply(dir, chan, settings, at), true) }
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.open_stream(true, stream, channels, || self.inner().rx_open(stream, channels))
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { refuse_or!(self, self.inner().rx_start(stream, at)) }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { refuse_or!(self, self.inner().rx_start_now(stream)) }
    fn rx_packet_samples(&self, stream: usize) -> usize { if self.refusal().is_some() { 0 } else { self.inner().rx_packet_samples(stream) } }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { refuse_or!(self, self.inner().rx_stop(stream, at)) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall, one_packet: bool) -> RxRecv {
        if let Some(error) = self.refusal() { return RxRecv::Failed(error); }
        let result = self.inner().rx_recv(stream, n, timeout, one_packet);
        if let RxRecv::Failed(error) = &result {
            if error.lost || is_control_timeout(&error.message) { self.latch.set(format!("device call failed: {}", error.message)); }
        }
        result
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.open_stream(false, stream, channels, || self.inner().tx_open(stream, channels))
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        refuse_or!(self, self.inner().tx_send(stream, samples, at, sob, eob, timeout))
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> { refuse_or!(self, self.inner().tx_async(stream, timeout)) }
    fn rx_close(&self, stream: usize) {
        if self.refusal().is_some() { return; }
        self.open.lock().unwrap().remove(&(true, stream));
        self.inner().rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        if self.refusal().is_some() { return; }
        self.open.lock().unwrap().remove(&(false, stream));
        self.inner().tx_close(stream)
    }
    fn close_streams(&self) {
        if self.refusal().is_some() { return; }
        self.open.lock().unwrap().clear();
        self.inner().close_streams()
    }
    fn host(&self) -> HostClock { self.inner().host() }
    fn mark_lost(&self) { self.inner().mark_lost() }
    // Not through `refuse_or!`: a refused priority (EPERM is UHD's OS error, which `note` latches as lost) must not poison the guard
    // (UR-23). A latched guard still refuses, like every other call.
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        if let Some(error) = self.refusal() { return Err(error); }
        self.inner().raise_thread_priority()
    }
}

/// A `GuardedDevice` on the installed record latch over the device `opener` opens. A latched record opens nothing: `opener` is
/// not called, and this panics, so the caller's probe or group is recorded as failed.
pub fn open_record_guard(opener: impl FnOnce() -> Arc<dyn Device>) -> Arc<GuardedDevice> {
    let latch = RECORD_LATCH.with(|cell| cell.borrow().clone()).unwrap_or_default();
    if let Some(reason) = latch.reason() { panic!("not opening the device: the record's latch holds ({reason})"); }
    Arc::new(GuardedDevice::new(opener(), latch))
}

/// `open_record_guard` as a plain `Device`, or just what `opener` opens when no record is running.
pub fn guard_open(opener: impl FnOnce() -> Arc<dyn Device>) -> Arc<dyn Device> {
    if RECORD_LATCH.with(|cell| cell.borrow().is_none()) { return opener(); }
    open_record_guard(opener)
}

#[derive(Default)]
struct Counts {
    rx: BTreeMap<usize, u64>,
    tx: BTreeMap<usize, u64>,
    short: BTreeMap<usize, u64>,
    open: BTreeSet<(bool, usize)>,
    opens: Vec<String>,
}

/// A `Device` that forwards to another and counts, per stream, the samples `rx_recv` returned and `tx_send` accepted,
/// the sends that took fewer samples than given, the streamers open now and the order they were opened in.
pub struct CountingDevice {
    inner: Arc<dyn Device>,
    counts: Mutex<Counts>,
    fault: InjectedFault,
    swallowed: AtomicUsize,
}

impl CountingDevice {
    pub fn new(inner: Arc<dyn Device>) -> CountingDevice {
        CountingDevice { inner, counts: Mutex::new(Counts::default()), fault: FAULT.with(std::cell::Cell::get), swallowed: AtomicUsize::new(0) }
    }

    pub fn rx_samples(&self, stream: usize) -> u64 { self.counts.lock().unwrap().rx.get(&stream).copied().unwrap_or(0) }
    pub fn tx_accepted(&self, stream: usize) -> u64 { self.counts.lock().unwrap().tx.get(&stream).copied().unwrap_or(0) }
    pub fn tx_short_sends(&self, stream: usize) -> u64 { self.counts.lock().unwrap().short.get(&stream).copied().unwrap_or(0) }
    pub fn swallowed_acks(&self) -> usize { self.swallowed.load(AtomicOrdering::Acquire) }
    /// The streamers still open, as `rx<k>` / `tx<k>`.
    pub fn open_streams(&self) -> Vec<String> {
        self.counts.lock().unwrap().open.iter().map(|(rx, k)| format!("{}{k}", if *rx { "rx" } else { "tx" })).collect()
    }
    /// Every `rx_open` / `tx_open` in order, as `rx<k>[channels]`.
    pub fn open_order(&self) -> Vec<String> { self.counts.lock().unwrap().opens.clone() }
}

impl Device for CountingDevice {
    fn describe(&self) -> Json { self.inner.describe() }
    fn fidelity(&self) -> Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> { self.inner.set_subdev_spec(dir, markup) }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> { self.inner.rates(dir, chan) }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> { self.inner.set_sources(clock, time) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> { self.inner.set_time_zero(at_next_pps) }
    fn time_now(&self) -> Result<i64, DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> { self.inner.ref_locked() }
    fn apply(&self, dir: Dir, chan: usize, settings: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> { self.inner.apply(dir, chan, settings, at) }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let result = self.inner.rx_open(stream, channels);
        if result.is_ok() {
            let mut counts = self.counts.lock().unwrap();
            counts.open.insert((true, stream));
            counts.opens.push(format!("rx{stream}{channels:?}"));
        }
        result
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.inner.rx_start_now(stream) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall, one_packet: bool) -> RxRecv {
        if self.fault.starve_rx == Some(stream) {
            self.inner.host().sleep(timeout);
            return RxRecv::Timeout;
        }
        let result = self.inner.rx_recv(stream, n, timeout, one_packet);
        if let RxRecv::Samples { samples, .. } = &result {
            *self.counts.lock().unwrap().rx.entry(stream).or_default() += samples.first().map_or(0, Vec::len) as u64;
        }
        result
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        let result = self.inner.tx_open(stream, channels);
        if result.is_ok() {
            let mut counts = self.counts.lock().unwrap();
            counts.open.insert((false, stream));
            counts.opens.push(format!("tx{stream}{channels:?}"));
        }
        result
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        let given = samples.first().map_or(0, |channel| channel.len());
        let mut accepted = self.inner.tx_send(stream, samples, at, sob, eob, timeout)?;
        if self.fault.short_tx == Some(stream) && accepted > 1 { accepted -= 1; }
        let mut counts = self.counts.lock().unwrap();
        *counts.tx.entry(stream).or_default() += accepted as u64;
        if accepted < given { *counts.short.entry(stream).or_default() += 1; }
        Ok(accepted)
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> {
        let report = self.inner.tx_async(stream, timeout)?;
        let swallow = |report: &TxReport| self.fault.swallow_ack == Some(stream) || self.fault.swallow_ack_channel == Some(report.channel);
        if report.as_ref().is_some_and(|report| report.code == TxCode::BurstAck && swallow(report)) {
            self.swallowed.fetch_add(1, AtomicOrdering::AcqRel);
            return Ok(None);
        }
        Ok(report)
    }
    fn rx_close(&self, stream: usize) {
        self.counts.lock().unwrap().open.remove(&(true, stream));
        self.inner.rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        self.counts.lock().unwrap().open.remove(&(false, stream));
        self.inner.tx_close(stream)
    }
    fn close_streams(&self) {
        self.counts.lock().unwrap().open.clear();
        self.inner.close_streams()
    }
    fn host(&self) -> HostClock { self.inner.host() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> { self.inner.raise_thread_priority() }
}

/// One stream of a stage: the layout's channels it carries and its rate.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamPlan {
    pub channels: Vec<usize>,
    pub rate_hz: u64,
}

/// One throughput Run of the condition set: the streams of each direction and the family it belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub id: String,
    pub family: &'static str,
    pub rx: Vec<StreamPlan>,
    pub tx: Vec<StreamPlan>,
}

impl Stage {
    /// The bytes per second of `direction`'s streams, which is the value's T (RM-32 budgets each direction alone).
    pub fn bytes(&self, direction: Direction, wire_bytes: u64) -> Option<u64> {
        let streams = if direction == Direction::Rx { &self.rx } else { &self.tx };
        streams.iter().try_fold(0u64, |total, stream| {
            total.checked_add(u64::try_from(stream.channels.len()).ok()?.checked_mul(stream.rate_hz)?.checked_mul(wire_bytes)?)
        })
    }

    /// The condition row of this stage (design 41 §2): each stream's channels and rate, and the streams that run together.
    pub fn condition(&self) -> Condition {
        let channels = |stream: &StreamPlan| stream.channels.iter().map(|chan| *chan as u32).collect::<Vec<_>>();
        let mut streams = Vec::new();
        let mut refs = Vec::new();
        for (k, stream) in self.rx.iter().enumerate() {
            streams.push(stream_condition(Direction::Rx, k as u32, channels(stream), stream.rate_hz as f64));
            refs.push(stream_ref(Direction::Rx, k as u32));
        }
        for (k, stream) in self.tx.iter().enumerate() {
            streams.push(stream_condition(Direction::Tx, k as u32, channels(stream), stream.rate_hz as f64));
            refs.push(stream_ref(Direction::Tx, k as u32));
        }
        Condition { id: self.id.clone(), streams, concurrent: if refs.len() > 1 { refs } else { Vec::new() }, reopened: false }
    }
}

fn rate_tag(rate_hz: u64) -> String {
    if rate_hz % 1_000_000 == 0 { format!("{}m", rate_hz / 1_000_000) } else { format!("{rate_hz}hz") }
}

/// The stages of `direction`'s throughput beyond the one-channel sweep, over the rate grid `rates` (ascending), for a
/// layout with `channels` channels in that direction: with two or more channels, (a) one stream of all channels,
/// (b) one stream each at one rate, (c) one stream each, channel 0 at a grid rate and channel 1 at the next lower
/// (BP4b; channel k the k-th lower). Each family runs its rates in ascending order of bytes per second, as the
/// one-channel sweep does. No stage runs the other direction too: a value is one direction's alone (RM-32 budgets
/// each direction), and the two values are tried together once, afterwards (`measure_duplex_at_limits`, stage 3e).
pub fn throughput_stages(direction: Direction, channels: usize, rates: &[u64]) -> Vec<Stage> {
    let prefix = if direction == Direction::Rx { "rx" } else { "tx" };
    let mut stages = Vec::new();
    let one = |channels: Vec<usize>, rate_hz: u64| StreamPlan { channels, rate_hz };
    let place = |direction: Direction, streams: Vec<StreamPlan>| -> (Vec<StreamPlan>, Vec<StreamPlan>) {
        if direction == Direction::Rx { (streams, Vec::new()) } else { (Vec::new(), streams) }
    };
    if channels >= 2 {
        for rate in rates {
            let (rx, tx) = place(direction, vec![one((0..channels).collect(), *rate)]);
            stages.push(Stage { id: format!("{prefix}-grouped-{}", rate_tag(*rate)), family: "grouped", rx, tx });
        }
        for rate in rates {
            let (rx, tx) = place(direction, (0..channels).map(|chan| one(vec![chan], *rate)).collect());
            stages.push(Stage { id: format!("{prefix}-split-{}", rate_tag(*rate)), family: "split", rx, tx });
        }
        for top in channels - 1..rates.len() {
            let picked: Vec<u64> = (0..channels).map(|chan| rates[top - chan]).collect();
            let (rx, tx) = place(direction, picked.iter().enumerate().map(|(chan, rate)| one(vec![chan], *rate)).collect());
            let tags: Vec<String> = picked.iter().map(|rate| rate_tag(*rate)).collect();
            stages.push(Stage { id: format!("{prefix}-mixed-{}", tags.join("-")), family: "mixed", rx, tx });
        }
    }
    stages
}

/// One stage's place in the capacity: its condition, the bytes per second of the value's direction, and its class
/// (`None`: the stage did not run).
#[derive(Clone, Debug)]
pub struct FamilyStage {
    pub condition: String,
    /// The condition's family and layout without its rate (`rx-grouped`, `rx-sweep`), by which a stopper is matched to its own
    /// earlier stages.
    pub family: String,
    pub bytes: u64,
    pub class: Option<TrialClass>,
}

/// The capacity over a set of conditions (P3 (1) widened): the largest T such that every stage of every condition that
/// has a stage at a T' of T or less succeeded. A condition applies to a T only where it has a stage; a stage that
/// did not run, was rejected or did not succeed stops the climb, and the first T never becomes the capacity.
#[derive(Clone, Debug)]
pub struct Capacity {
    pub best: Option<(u64, String)>,
    pub reason: Option<String>,
    pub observation: Json,
}

pub fn capacity_over_conditions(stages: &[FamilyStage]) -> Capacity {
    let totals: BTreeSet<u64> = stages.iter().map(|stage| stage.bytes).collect();
    let mut best = None;
    let mut blocker: Option<String> = None;
    let mut blocked: Option<(&FamilyStage, u64)> = None;
    for total in &totals {
        let at: Vec<&FamilyStage> = stages.iter().filter(|stage| stage.bytes == *total).collect();
        if let Some(stage) = at.iter().find(|stage| stage.class != Some(TrialClass::Success)) {
            blocker = Some(format!("condition `{}` at {} bytes/s is {}", stage.condition, total, stage.class.map_or("not run", trial_class_name)));
            blocked = Some((stage, *total));
            break;
        }
        best = Some((*total, at[0].condition.clone()));
    }
    // The value names a condition that succeeded at its T: the stage of the stopper's own family and layout that passed at T
    // (its actual id, whatever its rate), else the first that passed there. The stopper is the blocker, recorded apart.
    if let (Some((total, named)), Some((stopper, _))) = (best.as_mut(), blocked) {
        if let Some(stage) = stages.iter().find(|stage| stage.bytes == *total && stage.family == stopper.family && stage.class == Some(TrialClass::Success)) { *named = stage.condition.clone(); }
    }
    let reason = best.is_none().then(|| match &blocker {
        Some(blocker) => format!("No bytes-per-second step passed in every applicable condition: {blocker}."),
        None => "No throughput condition ran.".to_owned(),
    });
    let mut by_condition: BTreeMap<&str, (Vec<u64>, Vec<Json>)> = BTreeMap::new();
    for stage in stages {
        let row = by_condition.entry(stage.condition.as_str()).or_default();
        match stage.class {
            Some(TrialClass::Success) => row.0.push(stage.bytes),
            other => row.1.push(json!({ "bytes_per_s": stage.bytes, "class": other.map_or("NotRun", trial_class_name) })),
        }
    }
    let conditions: Json = by_condition.into_iter().map(|(id, (passed, failed))| (id.to_owned(), json!({ "passed_bytes_per_s": passed, "failed": failed }))).collect::<serde_json::Map<_, _>>().into();
    let applicable: Json = totals.iter().map(|total| (total.to_string(), json!(stages.iter().filter(|stage| stage.bytes == *total).map(|stage| stage.condition.as_str()).collect::<Vec<_>>()))).collect::<serde_json::Map<_, _>>().into();
    Capacity {
        observation: json!({
            "rule": "the capacity is the largest T such that every condition that has a stage at a T' <= T succeeded at every such stage; a condition with no stage at a T does not judge it",
            "capacity_bytes_per_s": best.as_ref().map(|(bytes, _)| bytes), "deciding_condition": best.as_ref().map(|(_, id)| id),
            "first_blocker": blocker, "blocker_condition": blocked.map(|(stage, _)| stage.condition.as_str()), "blocker_bytes_per_s": blocked.map(|(_, total)| total), "conditions": conditions, "applicable_conditions_by_bytes_per_s": applicable,
        }),
        best, reason,
    }
}

/// The capacity of `key` over `stages`: the measured row, or `unmeasured` with the reason; with the observation's body.
pub fn throughput_value(key: &str, stages: &[FamilyStage]) -> Result<(ValueRecord, Json), String> {
    let capacity = capacity_over_conditions(stages);
    let key = Key::parse(key).map_err(|error| error.to_string())?;
    let value = match &capacity.best {
        Some((bytes, condition)) => ValueRecord::Measured {
            key, worst: model::Worst::Number(*bytes as f64), trials: 1, condition: condition.clone(), test: TEST.to_owned(),
        },
        None => ValueRecord::Unmeasured { key, reason: capacity.reason.clone().unwrap_or_default() },
    };
    Ok((value, capacity.observation))
}

fn worse(first: TrialClass, second: TrialClass) -> TrialClass {
    let rank = |class: TrialClass| match class {
        TrialClass::AdmissionRejection => 5, TrialClass::ProviderFailure => 4, TrialClass::DeviceFailure => 3,
        TrialClass::Unattributable => 2, TrialClass::Incomplete => 1, TrialClass::Success => 0,
    };
    if rank(second) > rank(first) { second } else { first }
}

/// What one stream did in a stage.
#[derive(Clone, Debug)]
pub struct StreamEvidence {
    pub direction: Direction,
    pub stream: usize,
    pub class: TrialClass,
    pub detail: Json,
}

/// A stage's class: the worst of its streams'; a stage with no stream evidence is `Incomplete`.
pub fn stage_class(streams: &[StreamEvidence]) -> TrialClass {
    streams.iter().map(|stream| stream.class).reduce(worse).unwrap_or(TrialClass::Incomplete)
}

/// The byte counters of `nics`: `[rx_bytes, tx_bytes]`, each `None` where the interface or counter is unreadable.
fn nic_counters(nics: &[String]) -> BTreeMap<String, [Option<u64>; 2]> {
    let read = |nic: &str, name: &str| std::fs::read_to_string(format!("/sys/class/net/{nic}/statistics/{name}")).ok().and_then(|text| text.trim().parse().ok());
    nics.iter().map(|nic| (nic.clone(), [read(nic, "rx_bytes"), read(nic, "tx_bytes")])).collect()
}

fn nic_delta(before: &BTreeMap<String, [Option<u64>; 2]>, after: &BTreeMap<String, [Option<u64>; 2]>) -> Json {
    after.iter().map(|(nic, now)| {
        let was = before.get(nic).copied().unwrap_or([None, None]);
        let diff = |index: usize| now[index].zip(was[index]).and_then(|(now, was)| now.checked_sub(was));
        (nic.clone(), json!({ "rx_bytes": diff(0), "tx_bytes": diff(1) }))
    }).collect::<serde_json::Map<_, _>>().into()
}

/// The bench profile of a stage: `bench_profile` with one capture sink and one placement for each receive stream's port.
fn stage_profile(device: &dyn Device, dir: &common::TempDir, stage: &Stage) -> Json {
    let environment = if stage.tx.is_empty() { json!({}) } else { common::bench_envelope(device) };
    let mut doc = common::bench_profile(device, dir, json!({}), environment, false);
    for k in 1..stage.rx.len() {
        let sink = doc["bindings"]["rec"].clone();
        doc["bindings"][format!("rec{k}")] = sink;
        let link = json!({
            "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
            "from": { "component": "radio", "port": format!("rx{k}") }, "to": { "component": format!("rec{k}"), "port": "in" }
        });
        doc["placements"]["links"].as_array_mut().expect("the bench profile's links").push(link);
    }
    doc
}

/// The Spec of a stage: each stream's rate and channels (each channel at its board's default frequency), a capture
/// Sink on each receive port, and a repeating burst of `waveform` on each transmit stream.
fn stage_spec(device: &dyn Device, stage: &Stage, waveform: &ArtifactRef) -> Json {
    let eq = |value: Json| json!({ "kind": "eq", "value": value });
    let mut requires = serde_json::Map::new();
    if stage.rx.is_empty() { requires.insert("radio.rx.c0.stream".to_owned(), eq(json!(-1))); }
    for (direction, name, streams) in [(Dir::Rx, "rx", &stage.rx), (Dir::Tx, "tx", &stage.tx)] {
        for (k, stream) in streams.iter().enumerate() {
            requires.insert(format!("radio.{name}.s{k}.sample_rate_hz"), eq(json!(stream.rate_hz as f64)));
            for chan in &stream.channels {
                requires.insert(format!("radio.{name}.c{chan}.stream"), eq(json!(k)));
                requires.insert(format!("radio.{name}.c{chan}.frequency_hz"), eq(json!(common::channel_hz(device, direction, *chan))));
            }
        }
    }
    let sinks = stage.rx.len().max(1);
    let outputs: Vec<Json> = (0..sinks).map(|k| json!({
        "id": if k == 0 { "rec".to_owned() } else { format!("rec{k}") }, "kind": "sink.capture", "params": {},
        "feed": { "port": { "component": "radio", "port": format!("rx{k}") }, "policy": "drop_oldest", "capacity": 64 }
    })).collect();
    let schedule: Vec<Json> = (0..stage.tx.len()).map(|k| json!({
        "at": { "clock": "radio", "offset_ticks": 0 },
        "action": { "kind": "tx_burst", "target": serde_json::to_value(common::resource(&format!("radio/tx/s{k}"))).unwrap(),
            "waveform": waveform, "repeat": true, "late_policy": "send_asap_and_flag", "metadata": {} }
    })).collect();
    let mut spec = json!({
        "version": 1,
        "requirements": { "vocabularies": [{ "id": "radio", "major": 2 }, { "id": "sink", "major": 1 }] },
        "resources": { "radio": { "kind": "radio.device", "requires": requires } },
        "outputs": outputs
    });
    if !schedule.is_empty() { spec["schedule"] = Json::Array(schedule); }
    spec
}

/// What one stage's Run came to.
#[derive(Clone, Debug)]
pub struct StageResult {
    pub class: TrialClass,
    /// A streamer was still open when the Run ended.
    pub left_open: bool,
    pub observation: Json,
}

/// The admission ceiling of a candidate Run: one `link_gbps` link's line rate in each direction (an upper bound for
/// the Run to be admitted under, not what the link carries), over the profile's wire bytes per sample (RM-32).
fn candidate_performance(base: &ezsdr_radio::PerformanceEnvelope, link_gbps: i64) -> ezsdr_radio::PerformanceEnvelope {
    let ceiling = link_gbps.saturating_mul(125_000_000);
    ezsdr_radio::PerformanceEnvelope { rx_bytes_per_s: ceiling, tx_bytes_per_s: ceiling, wire_bytes_per_sample: base.wire_bytes_per_sample }
}

/// Runs one stage on `device` for `plan.throughput_seconds` and judges each stream on its own: a receive stream by its
/// own samples against its SampleClock's interval, a transmit stream by the samples the device accepted, its one burst
/// and its one acknowledgement; any overflow, underflow, partial send or async failure fails the stage. A Run that
/// validate or prepare refused is an `AdmissionRejection`. A streamer still open after the Run, or a control timeout,
/// is an error that stops the group (the control path is marked wedged, and nothing more is sent to the handle).
pub fn run_stage(device: Arc<dyn Device>, plan: &MeasurementPlan, stage: &Stage, wire_bytes: u64) -> Result<StageResult, String> {
    let counted = Arc::new(CountingDevice::new(device));
    let device: Arc<dyn Device> = counted.clone();
    let dir = common::TempDir::new();
    let doc = stage_profile(&*device, &dir, stage);
    let binding = radio_binding(&doc)?;
    let described = description(&binding, ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN, &ezsdr_radio_uhd::profile::terms())?;
    let link_gbps = doc["bindings"]["radio"]["selector"]["link_gbps"][0].as_i64().unwrap_or(10);
    let performance = candidate_performance(&described.performance, link_gbps);
    let (waveform_bytes, waveform) = common::transmit_waveform(plan.tx_waveform_samples);
    let spec = stage_spec(&*device, stage, &waveform);
    let inputs = BTreeMap::from([(waveform.hash.clone(), waveform_bytes)]);
    let nics_before = nic_counters(&plan.nics);
    let assembly = common::assembly(&doc, device.clone(), inputs, |radio| radio.with_measurement_performance(performance));
    let mut run = common::start_spec_run(&spec, &doc, assembly).map_err(|error| error.to_string())?;
    let streams_json = |streams: &[StreamPlan]| streams.iter().map(|stream| json!({ "channels": stream.channels, "rate_hz": stream.rate_hz })).collect::<Vec<_>>();
    let head = json!({ "family": stage.family, "rx_streams": streams_json(&stage.rx), "tx_streams": streams_json(&stage.tx),
        "duration_secs": plan.throughput_seconds, "admission_ceiling_bytes_per_s": performance.rx_bytes_per_s });
    let with_head = |extra: Json| { let mut row = head.clone(); row.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default()); row };
    if let Some(reason) = rejected_run_reason(&run) {
        return Ok(StageResult { class: TrialClass::AdmissionRejection, left_open: false, observation: with_head(json!({ "run_termination": reason })) });
    }
    let master = device.master_clock_rate();
    let t0 = run.start_instant().ok_or("throughput Run has no T0")?;
    let root = t0.domain();
    let start = t0.ticks_in(root).map_err(|error| error.to_string())?;
    let duration_ticks = i64::try_from(master.checked_mul(plan.throughput_seconds).ok_or("throughput duration overflow")?).map_err(|_| "throughput horizon overflow".to_owned())?;
    let mut horizon = start.checked_add(duration_ticks).ok_or("throughput horizon overflow")?;
    let mut scheduled = Vec::new();
    for (k, stream) in stage.tx.iter().enumerate() {
        let suffix = format!("usrp/tx/s{k}");
        let clock = run.sample_clocks().into_iter().rev().find(|clock| clock.stream.to_string().ends_with(&suffix) && clock.ended_at.is_none())
            .ok_or_else(|| format!("no active transmit stream-{k} SampleClock"))?;
        if !clock_rate_matches(&clock, stream.rate_hz) { return Err(format!("transmit stream {k} declared the wrong SampleClock rate for {} Hz", stream.rate_hz)); }
        if clock.root != root { return Err("throughput T0 and a SampleClock have different root clocks".to_owned()); }
        let (target, burst_root) = sample_at_or_after(&clock, start)?;
        horizon = horizon.max(burst_root.checked_add(duration_ticks).ok_or("throughput horizon overflow")?);
        scheduled.push((clock.domain, target));
    }
    // A Run that the Provider's policy stopped before the horizon (a device lost, say) still has its manifest to judge.
    let ended_early = match advance_root(&mut run, root, horizon) {
        Ok(()) => None,
        Err(error) if !matches!(run.state(), RunState::Running {}) => Some(error),
        Err(error) => return Err(error),
    };
    let packets: Vec<u64> = (0..stage.rx.len()).map(|k| device.rx_packet_samples(k).max(1) as u64).collect();
    let manifest = run.finish();
    let nics = nic_delta(&nics_before, &nic_counters(&plan.nics));
    // A streamer still open when its Run ended cannot be opened again (UHD aborts on a second open of a front end): the
    // stage is a failure and the group stops after it; the device guard latches at the group's end.
    let open = counted.open_streams();
    let events = serde_json::to_value(&manifest.events.delivered).map_err(|error| error.to_string())?;
    let overflow = manifest.events.delivered.iter().any(|event| event.kind.as_str() == "radio.RX_OVERFLOW");
    let underflow = manifest.events.delivered.iter().filter(|event| event.kind.as_str() == "radio.TX_UNDERFLOW").count();
    let mut streams = Vec::new();
    for (k, stream) in stage.rx.iter().enumerate() {
        let suffix = format!("usrp/rx/s{k}");
        let mut clocks = manifest.clocks.sample_clocks.iter().filter(|clock| clock.stream.to_string().ends_with(&suffix));
        let first_clock = clocks.next();
        let interval = first_clock.and_then(|clock| if clocks.next().is_some() { None } else { expected_rx_samples(clock, stream.rate_hz, master) });
        let span = first_clock.filter(|_| interval.is_some()).and_then(|clock| Some((clock.origin.ticks_in(clock.root).ok()?, clock.ended_at?.ticks_in(clock.root).ok()?)));
        let actual = counted.rx_samples(k);
        // The device returned the samples of each `rx_recv` whole, so the count passes the cut by up to one request (the block length).
        let slack = packets[k].max(u64::from(ezsdr_radio_uhd::profile::DEFAULT_BLOCK_LEN));
        let class = throughput_trial_class(actual, interval.map(|(_, samples)| samples), slack, overflow);
        streams.push(StreamEvidence { direction: Direction::Rx, stream: k, class, detail: json!({
            "direction": "rx", "stream": k, "channels": stream.channels, "rate_hz": stream.rate_hz, "expected_samples": interval.map(|(_, samples)| samples),
            "sample_clock_interval_ticks": interval.map(|(ticks, _)| ticks), "sample_clock_origin_tick": span.map(|(origin, _)| origin), "sample_clock_end_tick": span.map(|(_, end)| end), "actual_samples": actual, "packet_samples": packets[k], "tolerance_samples": slack,
            "rx_overflow": overflow, "class": trial_class_name(class),
        }) });
    }
    let async_all = manifest.section(&common::usrp(), "ezsdr.radio.uhd.async").ok_or("throughput Run has no async section")?;
    let bursts_all = manifest.section(&common::usrp(), "ezsdr.radio.uhd.bursts").ok_or("throughput Run has no burst section")?;
    let rejected = manifest.section(&common::usrp(), "ezsdr.radio.uhd.rejected").ok_or("throughput Run has no rejected section")?;
    for (k, stream) in stage.tx.iter().enumerate() {
        let (domain, target) = scheduled[k];
        let stream_rows: Vec<&Json> = async_all.as_array().into_iter().flatten().filter(|row| row["stream"].as_u64().is_none_or(|found| found == k as u64)).collect();
        // A stream of m channels is acknowledged once per channel: exactly one `BurstAck` of each of m channels is the one
        // acknowledgement the evidence reads (the first channel's); any other count leaves it with none.
        let acks: Vec<&Json> = stream_rows.iter().copied().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] != true).collect();
        let ack_channels: BTreeSet<u64> = acks.iter().filter_map(|row| row["channel"].as_u64()).collect();
        let acknowledged = acks.len() == stream.channels.len() && ack_channels.len() == stream.channels.len();
        let first_ack_channel = ack_channels.iter().next().copied();
        let async_rows: Vec<Json> = stream_rows.iter().copied()
            .filter(|row| row["code"] != "BurstAck" || (acknowledged && row["channel"].as_u64() == first_ack_channel)).cloned().collect();
        let burst_rows: Vec<Json> = bursts_all.as_array().into_iter().flatten()
            .filter(|row| serde_json::from_value::<TimePoint>(row["target"].clone()).is_ok_and(|point| point.domain() == domain)).cloned().collect();
        let accepted = counted.tx_accepted(k);
        let short = counted.tx_short_sends(k);
        // The Provider's rejection notes name no stream, so they are read once for the stage below, not per stream.
        let evidence = tx_throughput_evidence(TxThroughputInput {
            stats: &json!({ "tx_samples": accepted }), async_reports: &Json::Array(async_rows), events: &events, bursts: &Json::Array(burst_rows),
            rejected: &json!([]), clocks: &manifest.clocks.sample_clocks, scheduled_target: target, rate_hz: stream.rate_hz, master_clock_hz: master,
            duration_secs: plan.throughput_seconds, stream: k,
        });
        let class = if short > 0 { worse(evidence.class, TrialClass::DeviceFailure) } else { evidence.class };
        streams.push(StreamEvidence { direction: Direction::Tx, stream: k, class, detail: json!({
            "direction": "tx", "stream": k, "channels": stream.channels, "rate_hz": stream.rate_hz, "scheduled_burst_target": target,
            "burst_count": evidence.burst_count, "burst_blocks": evidence.burst_blocks, "burst_start_tick": evidence.burst_start_tick, "burst_acks_found": acks.len(), "burst_acks_expected": stream.channels.len(), "burst_ack_tick": evidence.burst_ack_tick,
            "minimum_ack_tick": evidence.minimum_ack_tick, "minimum_samples": evidence.minimum_samples, "actual_samples": accepted,
            "short_sends": short, "tx_underflow_events": evidence.underflow_events, "async_failures": evidence.async_failures,
            "class": trial_class_name(class),
        }) });
    }
    let short_send_rejections: Vec<String> = rejected.as_array().into_iter().flatten().filter_map(|row| row["reason"].as_str())
        .filter(|reason| reason.contains("short send") || reason.contains("send did not complete")).map(str::to_owned).collect();
    let mut class = stage_class(&streams);
    if underflow > 0 || overflow || !short_send_rejections.is_empty() || !open.is_empty() || ended_early.is_some() { class = worse(class, TrialClass::DeviceFailure); }
    let observation = with_head(json!({
        "streams": streams.iter().map(|stream| stream.detail.clone()).collect::<Vec<_>>(),
        "run_ended_early": ended_early, "tx_underflow_events": underflow, "rx_overflow": overflow, "short_send_rejections": short_send_rejections, "streamers_left_open": open,
        "streamers_opened_in_order": counted.open_order(), "nic_bytes": nics,
        "link_note": "UHD 4.10 picks a link per channel connection by the number of assignments per direction, not by bytes (graph_stream_manager.cpp:188-219,252-284); a stream's number is not a physical link",
        "stage": candidate_stage_json(&stage.id, stage.bytes(Direction::Rx, wire_bytes).or(stage.bytes(Direction::Tx, wire_bytes)).unwrap_or(0), &[class], &[None]),
    }));
    Ok(StageResult { class, left_open: !open.is_empty(), observation })
}

/// The rate of the stream a lead or gap measurement keeps running on the other direction (stage 3c-3a).
pub const COMPANION_RATE_HZ: u64 = 10_000_000;

/// Starts a repeating transmit stream at `COMPANION_RATE_HZ` on a Session.
pub fn start_companion_tx(run: &mut RunHandle, bytes: &[u8]) -> Result<(), String> {
    enable_tx(run)?;
    let entry = run.submit(common::set("radio.tx.s0.sample_rate_hz", ezsdr_kernel::spec::Value::num(COMPANION_RATE_HZ as f64).ok_or("invalid companion rate")?), None)
        .map_err(|error| error.to_string())?;
    if !common::admitted(&entry) { return Err(format!("the companion transmit rate was not admitted: {:?}", entry.outcome)); }
    common::past_tx_origin(run);
    start_repeat(run, bytes)
}

/// Sets the Session's receive stream to `COMPANION_RATE_HZ` and lets it settle.
pub fn start_companion_rx(run: &mut RunHandle) -> Result<(), String> {
    common::past_t0(run, common::ms(1));
    let entry = run.submit(common::set("radio.rx.s0.sample_rate_hz", ezsdr_kernel::spec::Value::num(COMPANION_RATE_HZ as f64).ok_or("invalid companion rate")?), None)
        .map_err(|error| error.to_string())?;
    if !common::admitted(&entry) { return Err(format!("the companion receive rate was not admitted: {:?}", entry.outcome)); }
    common::wait(run, common::ms(100));
    Ok(())
}

/// The condition row of a lead measured with a stream of `companion` running: the subject's stream at `rate_hz` and the companion's at `COMPANION_RATE_HZ`.
pub fn companion_condition(id: &str, subject: Direction, rate_hz: u64, companion: Direction) -> Condition {
    Condition {
        id: id.to_owned(),
        streams: vec![stream_condition(subject, 0, vec![0], rate_hz as f64), stream_condition(companion, 0, vec![0], COMPANION_RATE_HZ as f64)],
        concurrent: vec![stream_ref(subject, 0), stream_ref(companion, 0)], reopened: false,
    }
}

/// What a companion stream did over a Run, for the record only: the Run-wide samples it moved and its faults. The numbers
/// are not attributed to the trials of a lead or gap, so no value is decided from them.
pub fn companion_observation(manifest: &Manifest, direction: Direction) -> Json {
    let (stat, event) = if direction == Direction::Tx { ("tx_samples", "radio.TX_UNDERFLOW") } else { ("rx_samples", "radio.RX_OVERFLOW") };
    json!({
        "observation_only": true, "direction": if direction == Direction::Tx { "tx" } else { "rx" }, "rate_hz": COMPANION_RATE_HZ,
        "run_samples_moved": manifest.section(&common::usrp(), "ezsdr.radio.uhd.stats").and_then(|stats| stats.get(stat)).and_then(Json::as_u64),
        "faults": manifest.events.delivered.iter().filter(|row| row.kind.as_str() == event).count(),
        "note": "Run-wide counts, not attributed to the trials: the condition's candidates are recorded here and do not enter the value row, which stays from the solo conditions",
    })
}

/// What a direction's throughput adds beyond the one-channel sweep.
#[derive(Default)]
pub struct ExtraThroughput {
    pub conditions: Vec<Condition>,
    pub observations: Vec<Observation>,
    pub stages: Vec<FamilyStage>,
}

/// Runs `throughput_stages` of `direction` and returns their rows. A stage that fails (its `Err` or a panic) is that stage's
/// failure, with an observation, and ends the climb: the stages before it keep their rows and the stages after it are `not run`.
pub fn extra_throughput(device: &Arc<dyn Device>, plan: &MeasurementPlan, direction: Direction, channels: usize, wire_bytes: u64) -> Result<ExtraThroughput, String> {
    let rates = if direction == Direction::Rx { &plan.rx_rates_hz } else { &plan.tx_throughput_rates_hz };
    run_stages(direction, throughput_stages(direction, channels, rates), wire_bytes, |stage| run_stage(device.clone(), plan, stage, wire_bytes))
}

/// `extra_throughput` over `stages`, each run by `run`.
fn run_stages(direction: Direction, stages: Vec<Stage>, wire_bytes: u64, mut run: impl FnMut(&Stage) -> Result<StageResult, String>) -> Result<ExtraThroughput, String> {
    let mut extra = ExtraThroughput::default();
    let name = if direction == Direction::Rx { "rx" } else { "tx" };
    let mut stopped = false;
    for stage in stages {
        let bytes = stage.bytes(direction, wire_bytes).ok_or("stage bytes per second overflow")?;
        extra.conditions.push(stage.condition());
        // After a stage that wedged the handle or failed nothing more runs: the stages left are `not run`, which blocks the capacity.
        if stopped || record_latch_reason().is_some() {
            extra.stages.push(FamilyStage { condition: stage.id.clone(), family: format!("{name}-{}", stage.family), bytes, class: None });
            continue;
        }
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&stage))) {
            Ok(outcome) => outcome,
            Err(payload) => Err(GroupRun::panic_text(payload)),
        };
        let observed = match outcome {
            Ok(result) => {
                stopped = result.left_open;
                extra.stages.push(FamilyStage { condition: stage.id.clone(), family: format!("{name}-{}", stage.family), bytes, class: Some(result.class) });
                result.observation
            }
            Err(error) => {
                stopped = true;
                extra.stages.push(FamilyStage { condition: stage.id.clone(), family: format!("{name}-{}", stage.family), bytes, class: None });
                json!({ "family": stage.family, "stage_failed": error, "note": "the stage failed before it could be judged: it is `not run`, and no later stage of this direction runs" })
            }
        };
        extra.observations.push(observation(format!("provider_{name}_throughput_{}", stage.id), TEST, Some(stage.id.clone()), 1, observed)?);
    }
    Ok(extra)
}

/// The condition id of the pair of the two measured limits run at once (stage 3e).
pub const DUPLEX_PAIR: &str = "duplex-at-the-measured-limits";

/// What the pair runs: each direction's streams, as the condition that held that direction's throughput value had them.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplexLimits {
    pub rx_condition: String,
    pub rx: Vec<StreamPlan>,
    pub tx_condition: String,
    pub tx: Vec<StreamPlan>,
}

fn limit_streams(measured: &MeasurementGroup, key: &str, direction: Direction) -> Result<(String, Vec<StreamPlan>), String> {
    let condition = measured.values.iter().find_map(|row| match row {
        ValueRecord::Measured { key: found, condition, .. } if found.as_str() == key => Some(Ok(condition.clone())),
        ValueRecord::Unmeasured { key: found, reason } if found.as_str() == key => Some(Err(format!("{key} is unmeasured ({reason})"))),
        _ => None,
    }).unwrap_or_else(|| Err(format!("{key} was not decided")))?;
    let row = measured.conditions.iter().find(|row| row.id == condition).ok_or_else(|| format!("{key}'s condition `{condition}` has no condition row"))?;
    let mut streams: Vec<_> = row.streams.iter().filter(|stream| stream.direction == direction).collect();
    streams.sort_by_key(|stream| stream.stream);
    if streams.is_empty() { return Err(format!("{key}'s condition `{condition}` has no stream in its direction")); }
    let plans = streams.iter().map(|stream| StreamPlan { channels: stream.channels.iter().map(|chan| *chan as usize).collect(), rate_hz: stream.sample_rate_hz as u64 }).collect();
    Ok((condition, plans))
}

/// The streams the receive and the transmit throughput values held at, or why one of them cannot be tried (a value is
/// `unmeasured`). The pair is run only once both are decided.
pub fn duplex_limits(measured: &MeasurementGroup) -> Result<DuplexLimits, String> {
    match (limit_streams(measured, "radio.perf.rx_bytes_per_s", Direction::Rx), limit_streams(measured, "radio.perf.tx_bytes_per_s", Direction::Tx)) {
        (Ok((rx_condition, rx)), Ok((tx_condition, tx))) => Ok(DuplexLimits { rx_condition, rx, tx_condition, tx }),
        (rx, tx) => Err([rx.err(), tx.err()].into_iter().flatten().collect::<Vec<_>>().join("; ")),
    }
}

/// The measured limits of both directions tried together once, for `plan.throughput_seconds` (stage 3e, owner 2026-10-28):
/// each direction on the streams and rates that held its own value, the receive streams and the transmit streams at the
/// same time. It decides no value. The observation says whether the two values were shown at once or not, with each
/// stream's evidence, how much of the time transmit flowed and the stretch in which the two overlapped. With no limits
/// (a value is `unmeasured`) nothing runs and the observation says why.
pub fn measure_duplex_at_limits(device: Arc<dyn Device>, plan: &MeasurementPlan, limits: Result<DuplexLimits, String>, wire_bytes: u64) -> Result<MeasurementGroup, String> {
    let name = "provider_duplex_at_the_measured_limits";
    let note = "one confirmation of the two measured limits at once, for the duration given; it is no value, and no evidence for every pair the profile admits or for a longer run";
    let limits = match limits {
        Ok(limits) => limits,
        Err(reason) => {
            let result = json!({ "ran": false, "reason": format!("not run: {reason}"), "verdict": "the two values were not shown to hold at once: they were not both measured", "note": note });
            return Ok(MeasurementGroup { observations: vec![observation(name, TEST, None, 1, result)?], ..MeasurementGroup::default() });
        }
    };
    let stage = Stage { id: DUPLEX_PAIR.to_owned(), family: "duplex_at_the_measured_limits", rx: limits.rx.clone(), tx: limits.tx.clone() };
    let condition = stage.condition();
    let outcome = run_stage(device, plan, &stage, wire_bytes);
    let mut result = match &outcome {
        Ok(result) => result.observation.clone(),
        Err(error) => json!({ "stage_failed": error }),
    };
    let class = outcome.as_ref().ok().map(|result| result.class);
    let shown = class == Some(TrialClass::Success);
    let fields = result.as_object_mut().ok_or("the duplex pair's observation is not an object")?;
    fields.insert("ran".to_owned(), json!(outcome.is_ok()));
    fields.insert("rx_limit_condition".to_owned(), json!(limits.rx_condition));
    fields.insert("tx_limit_condition".to_owned(), json!(limits.tx_condition));
    fields.insert("class".to_owned(), json!(class.map_or("not_judged", trial_class_name)));
    fields.insert("verdict".to_owned(), json!(if shown { "the two values held at once for the duration given" } else { "the two values were not shown to hold at once" }));
    fields.insert("note".to_owned(), json!(note));
    fields.insert("overlap".to_owned(), duplex_overlap(fields.get("streams"), plan.throughput_seconds));
    let group = MeasurementGroup { conditions: vec![condition], observations: vec![observation(name, TEST, Some(DUPLEX_PAIR.to_owned()), 1, result)?], ..MeasurementGroup::default() };
    Ok(group)
}

/// How much of the run the two directions flowed together: each transmit stream's samples against the duration's, and the
/// root ticks in which the receive streams' SampleClocks and the transmit bursts overlapped (null where a bound is unknown).
fn duplex_overlap(streams: Option<&Json>, seconds: u64) -> Json {
    let rows: Vec<&Json> = streams.and_then(Json::as_array).into_iter().flatten().collect();
    let of = |direction: &str| rows.iter().filter(|row| row["direction"] == direction).copied().collect::<Vec<_>>();
    let tx_fraction: Vec<Json> = of("tx").iter().map(|row| {
        let expected = row["rate_hz"].as_u64().and_then(|rate| rate.checked_mul(seconds));
        json!({ "stream": row["stream"], "samples_accepted_over_samples_of_the_duration": row["actual_samples"].as_u64().zip(expected).map(|(actual, expected)| actual as f64 / expected as f64) })
    }).collect();
    let rx_span = of("rx").iter().map(|row| row["sample_clock_origin_tick"].as_i64().zip(row["sample_clock_end_tick"].as_i64())).collect::<Option<Vec<_>>>();
    let tx_span = of("tx").iter().map(|row| row["burst_start_tick"].as_i64().zip(row["burst_ack_tick"].as_i64())).collect::<Option<Vec<_>>>();
    let overlap_ticks = rx_span.zip(tx_span).and_then(|(rx, tx)| {
        let start = rx.iter().chain(&tx).map(|(origin, _)| *origin).max()?;
        let end = rx.iter().chain(&tx).map(|(_, end)| *end).min()?;
        Some((end - start).max(0))
    });
    json!({ "tx_flow": tx_fraction, "overlap_root_ticks": overlap_ticks })
}

/// UR-7 inside a record (stage 3e): the reference clock's PLL did not lock and the retries are used up, so it will not lock
/// for the next open either (it stayed unlocked until a power cycle on 2026-10-27). The record's latch is set, every later
/// probe and group is `not run`, and the record is written early. Does nothing outside a record or for any other error.
pub fn latch_record_on_pll_failure(error: &str) {
    if !error.contains(ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK) { return; }
    RECORD_LATCH.with(|cell| if let Some(latch) = cell.borrow().as_ref() {
        latch.set(format!("the reference clock's PLL did not lock after the UR-7 retries: {error}; it needs a power cycle"));
    });
}

#[cfg(all(test, not(feature = "uhd")))]
mod tests {
    use super::*;

    fn stage(condition: &str, bytes: u64, class: Option<TrialClass>) -> FamilyStage {
        FamilyStage { condition: condition.to_owned(), family: condition.to_owned(), bytes, class }
    }

    #[test]
    fn stage_lists_follow_the_layout() {
        let rates = [100_000_000, 200_000_000];
        assert!(throughput_stages(Direction::Rx, 1, &rates).is_empty(), "one channel has only the one-channel sweep, and no stage runs both directions");
        let two = throughput_stages(Direction::Rx, 2, &rates);
        let ids: Vec<_> = two.iter().map(|stage| stage.id.as_str()).collect();
        assert_eq!(ids, ["rx-grouped-100m", "rx-grouped-200m", "rx-split-100m", "rx-split-200m", "rx-mixed-200m-100m"]);
        assert!(two.iter().all(|stage| stage.tx.is_empty()), "a receive stage has no transmit stream");
        let mixed = two.last().unwrap();
        assert_eq!(mixed.rx, [StreamPlan { channels: vec![0], rate_hz: 200_000_000 }, StreamPlan { channels: vec![1], rate_hz: 100_000_000 }], "channel 0 at the higher grid rate, channel 1 at the next lower");
        assert_eq!(mixed.bytes(Direction::Rx, 4), Some(1_200_000_000));
        assert_eq!(two[1].bytes(Direction::Rx, 4), Some(1_600_000_000), "one stream of both channels at 200 MS/s");
        assert_eq!(two[1].rx.len(), 1);
        assert_eq!(two[3].rx.len(), 2);
        let tx = throughput_stages(Direction::Tx, 2, &rates);
        assert!(tx.iter().all(|stage| stage.rx.is_empty() && !stage.tx.is_empty()));
    }

    fn measured_row(key: &str, condition: &str) -> ValueRecord {
        ValueRecord::Measured { key: Key::parse(key).unwrap(), worst: model::Worst::Number(1.0), trials: 1, condition: condition.to_owned(), test: TEST.to_owned() }
    }

    #[test]
    fn the_duplex_pair_runs_each_direction_on_the_streams_that_held_its_value() {
        // A 2-channel receive limit held by one stream of both channels at 100 MS/s; a transmit limit by two streams at 200 and 100.
        let mut measured = MeasurementGroup::default();
        measured.conditions.push(throughput_stages(Direction::Rx, 2, &[100_000_000, 200_000_000])[0].condition());
        measured.conditions.push(throughput_stages(Direction::Tx, 2, &[100_000_000, 200_000_000]).last().unwrap().condition());
        assert!(duplex_limits(&measured).unwrap_err().contains("was not decided"), "no value row, no pair");
        measured.values.push(measured_row("radio.perf.rx_bytes_per_s", "rx-grouped-100m"));
        measured.values.push(measured_row("radio.perf.tx_bytes_per_s", "tx-mixed-200m-100m"));
        let limits = duplex_limits(&measured).unwrap();
        assert_eq!(limits.rx, [StreamPlan { channels: vec![0, 1], rate_hz: 100_000_000 }], "the multi-channel capacity is not shrunk to channel 0's rate");
        assert_eq!(limits.tx, [StreamPlan { channels: vec![0], rate_hz: 200_000_000 }, StreamPlan { channels: vec![1], rate_hz: 100_000_000 }]);
        assert_eq!((limits.rx_condition.as_str(), limits.tx_condition.as_str()), ("rx-grouped-100m", "tx-mixed-200m-100m"));
        // Either value `unmeasured`: no pair, and the reason says which.
        measured.values[1] = ValueRecord::Unmeasured { key: Key::parse("radio.perf.tx_bytes_per_s").unwrap(), reason: "no step passed".to_owned() };
        let error = duplex_limits(&measured).unwrap_err();
        assert!(error.contains("radio.perf.tx_bytes_per_s is unmeasured") && !error.contains("rx_bytes_per_s"), "{error}");
        // An unmeasured value starts no pair: the Run is not made and no call reaches the device.
        let fake = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default()));
        let group = measure_duplex_at_limits(fake.clone(), &MeasurementPlan::rehearsal(), duplex_limits(&measured), 4).unwrap();
        assert!(group.conditions.is_empty() && group.observations[0].result["ran"] == false, "{:?}", group.observations[0].result);
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    }

    #[test]
    fn the_duplex_pair_is_not_run_without_both_values_and_says_so() {
        let fake = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default()));
        let device: Arc<dyn Device> = fake.clone();
        let group = measure_duplex_at_limits(device, &MeasurementPlan::rehearsal(), Err("radio.perf.rx_bytes_per_s is unmeasured (x)".to_owned()), 4).unwrap();
        assert!(group.values.is_empty() && group.conditions.is_empty(), "the pair decides no value");
        let row = &group.observations[0];
        assert_eq!(row.name, "provider_duplex_at_the_measured_limits");
        assert_eq!(row.result["ran"], false);
        assert!(row.result["reason"].as_str().unwrap().starts_with("not run: radio.perf.rx_bytes_per_s is unmeasured"));
        assert!(row.result["verdict"].as_str().unwrap().contains("not shown"));
        assert!(fake.calls().is_empty(), "no call reached the device: {:?}", fake.calls());
    }

    /// The sweep's stages and `throughput_stages`' grouped and split ones for two channels, with their real ids and families, at
    /// 4 wire bytes per sample.
    fn real_stages(rates: &[u64], class: impl Fn(&Stage) -> Option<TrialClass>) -> Vec<FamilyStage> {
        let mut stages: Vec<FamilyStage> = rates.iter().map(|rate| FamilyStage { condition: format!("rx-{}msps", rate / 1_000_000), family: "rx-sweep".to_owned(), bytes: rate * 4, class: Some(TrialClass::Success) }).collect();
        stages.extend(throughput_stages(Direction::Rx, 2, rates).iter().filter(|stage| stage.family != "mixed").map(|stage| FamilyStage {
            condition: stage.id.clone(), family: format!("rx-{}", stage.family), bytes: stage.bytes(Direction::Rx, 4).unwrap(), class: class(stage),
        }));
        stages
    }

    #[test]
    fn the_capacity_names_a_condition_that_passed_and_the_blocker_apart() {
        use TrialClass::*;
        let rates = [100_000_000, 200_000_000];
        // Grouped passes at 100 MS/s (800 MB/s) and fails at 200 MS/s: the climb stops at 800 MB/s, where the sweep's 200 MS/s is the
        // first stage listed. The value names grouped's own passing stage, with its real id, not the sweep's and not the failed one's.
        let capacity = capacity_over_conditions(&real_stages(&rates, |stage| Some(if stage.id == "rx-grouped-200m" { DeviceFailure } else { Success })));
        assert_eq!(capacity.best, Some((800_000_000, "rx-grouped-100m".to_owned())), "{capacity:?}");
        assert_eq!((capacity.observation["blocker_condition"].as_str(), capacity.observation["blocker_bytes_per_s"].as_u64()), (Some("rx-grouped-200m"), Some(1_600_000_000)));
        // The split family stopping it is named by its own stage instead.
        let split = capacity_over_conditions(&real_stages(&rates, |stage| Some(if stage.id == "rx-split-200m" { DeviceFailure } else { Success })));
        assert_eq!(split.best.map(|(_, id)| id), Some("rx-split-100m".to_owned()));
        // With no stage of the stopper's family passing at the capacity, the first that passed is named, never the failed one.
        let mixed = capacity_over_conditions(&[
            FamilyStage { condition: "rx-100msps".to_owned(), family: "rx-sweep".to_owned(), bytes: 400, class: Some(Success) },
            FamilyStage { condition: "rx-split-100m".to_owned(), family: "rx-split".to_owned(), bytes: 800, class: Some(Success) },
            FamilyStage { condition: "rx-grouped-200m".to_owned(), family: "rx-grouped".to_owned(), bytes: 1_600, class: Some(DeviceFailure) },
        ]);
        assert_eq!(mixed.best, Some((800, "rx-split-100m".to_owned())));
    }

    fn result_of(class: TrialClass) -> StageResult { StageResult { class, left_open: false, observation: json!({ "family": "split" }) } }

    #[test]
    fn a_stage_that_fails_ends_the_climb_and_keeps_the_stages_before_it() {
        let rates = [100_000_000, 200_000_000];
        let stages = || throughput_stages(Direction::Rx, 2, &rates);
        for panics in [false, true] {
            let mut ran = Vec::new();
            let extra = run_stages(Direction::Rx, stages(), 4, |stage| {
                ran.push(stage.id.clone());
                match ran.len() {
                    1 | 2 => Ok(result_of(TrialClass::Success)),
                    _ if panics => panic!("a stage panicked"),
                    _ => Err("no active transmit stream-1 SampleClock".to_owned()),
                }
            }).unwrap();
            assert_eq!(ran.len(), 3, "the stage after the failed one is not run");
            let classes: Vec<_> = extra.stages.iter().map(|stage| stage.class).collect();
            assert_eq!(classes, [Some(TrialClass::Success), Some(TrialClass::Success), None, None, None], "the stages before keep their class, the failed and the later are `not run`");
            assert_eq!(extra.observations.len(), 3);
            assert!(extra.observations[0].result["family"] == "split" && extra.observations[1].result["family"] == "split", "the earlier stages' observations stay");
            let failed = extra.observations[2].result["stage_failed"].as_str().unwrap();
            assert!(failed.contains(if panics { "a stage panicked" } else { "no active transmit" }), "{failed}");
            assert_eq!(extra.conditions.len(), 5, "every stage still has its condition row");
        }
    }

    #[test]
    fn a_stage_with_an_open_streamer_stops_later_stages() {
        let stages = throughput_stages(Direction::Rx, 2, &[100_000_000, 200_000_000]);
        let mut ran = Vec::new();
        let extra = run_stages(Direction::Rx, stages.into_iter().take(2).collect(), 4, |stage| {
            ran.push(stage.id.clone());
            Ok(StageResult { class: TrialClass::DeviceFailure, left_open: true, observation: json!({}) })
        }).unwrap();
        assert_eq!(ran.len(), 1, "the stage after the open streamer is not run");
        assert_eq!(extra.stages.iter().map(|stage| stage.class).collect::<Vec<_>>(), [Some(TrialClass::DeviceFailure), None]);
    }

    #[test]
    fn the_pll_retries_used_up_latch_the_record_and_nothing_more_is_opened() {
        let pll = || Err::<(), String>(format!("{} to its internal source", ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK));
        let latch = Latch::new();
        install_record_latch(Some(latch.clone()));
        // A retry that succeeds latches nothing.
        let mut calls = 0;
        assert!(common::open_with_pll_retries(|| { calls += 1; if calls < 3 { pll() } else { Ok(()) } }, 3, Wall::ZERO).is_ok());
        assert_eq!((calls, latch.reason()), (3, None));
        // An error that is not the PLL's is not retried and does not latch.
        let mut calls = 0;
        assert!(common::open_with_pll_retries(|| { calls += 1; Err::<(), _>("UR-7: set_sources failed".to_owned()) }, 3, Wall::ZERO).is_err());
        assert_eq!((calls, latch.reason()), (1, None));
        // A PLL that stays unlocked: the retries are used up, the latch is set, and no later opener is called.
        let mut calls = 0;
        assert!(common::open_with_pll_retries(|| { calls += 1; pll() }, 3, Wall::ZERO).is_err());
        assert_eq!(calls, 4, "the open and three retries");
        assert!(latch.reason().is_some_and(|reason| reason.contains("PLL did not lock") && reason.contains("power cycle")));
        let mut opened = 0;
        let later = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { let _ = open_record_guard(|| { opened += 1; Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default())) }); }));
        assert!(later.is_err() && opened == 0, "a latched record opens nothing");
        // The failed group and the groups after it are in the record, as failures and `not run`.
        let mut groups = GroupRun::new(None);
        let key = |name: &str| vec![name.to_owned()];
        groups.run("first", &key("radio.timing.tx_in_flight_ns"), || Err("the group failed".to_owned()));
        groups.run("second", &key("radio.timing.rx_start_lead_ns"), || match record_latch_reason() {
            Some(reason) => Err(format!("not run: the device handle is latched ({reason}); nothing is sent to it")),
            None => Ok(MeasurementGroup::default()),
        });
        groups.probe("a probe", || { let _ = open_record_guard(|| unreachable!("a latched record opens nothing")); });
        let names: Vec<_> = groups.failed().iter().map(|(name, error)| (name.as_str(), error.starts_with("not run"))).collect();
        assert_eq!(names, [("first", false), ("second", true), ("a probe", false)]);
        assert!(groups.failed()[2].1.contains("not opening the device"));
        let record = groups.take();
        assert_eq!(record.values.len(), 2, "each failed group's key is `unmeasured` in the record");
        // The pair is a group of its own: with the latch set it is `not run` and its device is not asked for.
        let mut pair_groups = GroupRun::new(None);
        pair_groups.run_duplex_pair(&MeasurementPlan::rehearsal(), 4, || unreachable!("a latched record asks for no device"));
        assert!(pair_groups.failed()[0].1.starts_with("not run: the device handle is latched"), "{:?}", pair_groups.failed());
        install_record_latch(None);
        // A probe's `set_sources` is retried and latches the same way, with UHD's text turned into the retried one.
        let latch = Latch::new();
        install_record_latch(Some(latch.clone()));
        let fake = ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { faults: vec![ezsdr_radio_uhd::FakeFault::ReferenceDoesNotLock], ..ezsdr_radio_uhd::FakeConfig::default() });
        let error = common::set_sources_with_pll_retries_every(&fake, "internal", "internal", Wall::ZERO).unwrap_err();
        assert!(error.starts_with(ezsdr_radio_uhd::REFERENCE_DID_NOT_LOCK), "{error}");
        assert_eq!(fake.calls().iter().filter(|call| call.starts_with("set_sources")).count(), 4, "the call and three retries");
        assert!(latch.reason().is_some());
        let healthy = ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default());
        install_record_latch(Some(Latch::new()));
        assert!(common::set_sources_with_pll_retries_every(&healthy, "internal", "internal", Wall::ZERO).is_ok());
        assert_eq!(record_latch_reason(), None, "a lock that comes needs no latch");
        install_record_latch(None);
        // Outside a record the same exhaustion latches nothing.
        let mut calls = 0;
        assert!(common::open_with_pll_retries(|| { calls += 1; pll() }, 1, Wall::ZERO).is_err());
        assert_eq!(record_latch_reason(), None);
    }

    #[test]
    fn capacity_of_one_condition_is_the_successful_prefix_of_the_sweep() {
        use TrialClass::*;
        let classes = [Success, Success, DeviceFailure, Success];
        let one: Vec<_> = classes.iter().enumerate().map(|(i, class)| stage("a", (i as u64 + 1) * 100, Some(*class))).collect();
        let candidates = BTreeMap::from([("a".to_owned(), classes.iter().enumerate().map(|(i, class)| CandidateStage { value: (i as u64 + 1) * 100, trials: vec![*class] }).collect())]);
        let summary = summarize_candidates(SummaryMode::Capacity, &candidates, 1).unwrap();
        assert_eq!(capacity_over_conditions(&one).best.map(|(bytes, _)| bytes), summary.worst);
        assert_eq!(summary.worst, Some(200), "the first failure is not the capacity, and a success above it does not count");
    }

    #[test]
    fn capacity_over_conditions_judges_each_t_by_the_conditions_that_have_it() {
        use TrialClass::*;
        // `b` has stages only at 150 and 300, so it neither judges 100 and 200 nor is missing there.
        let ok = |condition: &str, bytes| stage(condition, bytes, Some(Success));
        let all = [ok("a", 100), ok("a", 200), ok("a", 300), ok("b", 150), ok("b", 300)];
        assert_eq!(capacity_over_conditions(&all).best, Some((300, "a".to_owned())));
        let blocked = [ok("a", 100), ok("a", 200), ok("a", 300), ok("b", 150), stage("b", 250, Some(DeviceFailure))];
        let capacity = capacity_over_conditions(&blocked);
        assert_eq!(capacity.best.map(|(bytes, _)| bytes), Some(200), "b's failure at 250 stops the climb below a's 300");
        assert_eq!(capacity.observation["first_blocker"], "condition `b` at 250 bytes/s is DeviceFailure");
        let unmeasured = capacity_over_conditions(&[stage("a", 100, Some(AdmissionRejection)), ok("b", 200)]);
        assert!(unmeasured.best.is_none() && unmeasured.reason.as_deref().is_some_and(|reason| reason.contains("AdmissionRejection")), "{unmeasured:?}");
        let missing = capacity_over_conditions(&[ok("a", 100), stage("b", 100, None)]);
        assert!(missing.best.is_none(), "a condition that did not run is not a pass: {missing:?}");
        assert!(capacity_over_conditions(&[]).best.is_none());
    }

    #[test]
    fn a_stage_is_as_bad_as_its_worst_stream() {
        use TrialClass::*;
        let row = |stream, class| StreamEvidence { direction: Direction::Rx, stream, class, detail: json!({}) };
        assert_eq!(stage_class(&[row(0, Success), row(1, Success)]), Success);
        assert_eq!(stage_class(&[row(0, Success), row(1, Incomplete)]), Incomplete, "one starved stream is not hidden by the others");
        assert_eq!(stage_class(&[row(0, Incomplete), row(1, DeviceFailure)]), DeviceFailure);
        assert_eq!(stage_class(&[]), Incomplete);
    }

    #[test]
    fn the_guard_latches_on_apply_errors_timeouts_and_losses_only() {
        let latch = Latch::new();
        let guard = GuardedDevice::new(Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default())), latch.clone());
        assert!(guard.note(Err::<(), _>(DeviceError::failed("ordinary failure")), false).is_err());
        assert_eq!(latch.reason(), None, "an ordinary error of another call does not latch");
        assert!(is_control_timeout("UHD: Control operation timed out waiting for reply") && !is_control_timeout("short send"));
        let _ = guard.note(Err::<(), _>(DeviceError::failed("Control operation timed out")), false);
        assert!(latch.reason().is_some_and(|reason| reason.contains("timed out")));
        latch.set("second");
        assert!(latch.reason().is_some_and(|reason| !reason.contains("second")), "the first reason stays");
        let apply = Latch::new();
        let guard = GuardedDevice::new(Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig::default())), apply.clone());
        let _ = guard.note(Err::<(), _>(DeviceError::failed("any apply error")), true);
        assert!(apply.reason().is_some_and(|reason| reason.starts_with("apply failed")));
    }

    #[test]
    fn the_candidate_ceiling_is_one_links_line_rate_per_direction() {
        let base = ezsdr_radio::x3xx::performance(10);
        let candidate = candidate_performance(&base, 10);
        assert_eq!((candidate.rx_bytes_per_s, candidate.tx_bytes_per_s, candidate.wire_bytes_per_sample), (1_250_000_000, 1_250_000_000, 4));
        assert!(candidate.rx_bytes_per_s > base.rx_bytes_per_s);
    }
}
