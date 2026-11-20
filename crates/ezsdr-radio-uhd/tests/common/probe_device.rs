use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration as Wall, Instant};

use ezsdr_kernel::module_api::Fidelity;
use ezsdr_radio_uhd::{Applied, Device, DeviceError, Dir, HostClock, Iq, RxRecv, Settings, TxCode, TxReport};
use serde_json::{Value as Json, json};

pub const PROBE_ROW_LIMIT: usize = 65_536;

/// What a `tx_send` is, for the hold (bt2 C7): four disjoint inputs, so that a hold on a device burst's first send never reaches its close, a flush or a continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendClass {
    /// T3's untimed four-zero start-and-end send.
    Flush,
    /// A device burst's end: end-of-burst, no start, at most two samples (UR-22's close).
    Close,
    /// A device burst's first send (any start of burst that is not a flush, a short closed one included).
    Start,
    /// Every other send of samples.
    Continuation,
}

impl SendClass {
    pub fn name(self) -> &'static str {
        match self {
            SendClass::Flush => "flush",
            SendClass::Close => "close",
            SendClass::Start => "start",
            SendClass::Continuation => "continuation",
        }
    }
}

/// The class of one `tx_send`.
pub fn send_class(at: Option<i64>, sob: bool, eob: bool, samples: &[&[Iq]]) -> SendClass {
    if at.is_none() && sob && eob && !samples.is_empty() && samples.iter().all(|channel| channel.len() == 4 && channel.iter().all(|sample| *sample == Iq::default())) {
        SendClass::Flush
    } else if sob {
        SendClass::Start
    } else if eob && at.is_some() && !samples.is_empty() && samples.iter().all(|channel| channel.len() <= 2) {
        SendClass::Close
    } else {
        SendClass::Continuation
    }
}

/// How long a held send stays in the call: a duration, or until an instant of the probe's own axis ([`ProbeDevice::clock_ns`]), so that a Session can fix the instant
/// its send returns whatever instant the Module handed it over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    For(Wall),
    Until(u64),
}

/// A stall the probe puts in front of one `tx_send` (a host that was descheduled, bt2 C7), armed by [`ProbeDevice::arm_hold`]. Each entry fires once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxHold {
    /// The `nth` (1-based) `Start` handed since arming.
    Start { nth: usize, hold: Hold },
    /// The first `Continuation` handed at least `after` (on the clock axis) after the first `Start` since arming.
    Midplay { after: Wall, hold: Hold },
    /// Every `Start` handed since arming; the plan keeps this entry (bt2 C7's storm, S8).
    EveryStart(Hold),
}

fn hold_wall(hold: Hold, entered: u64) -> Wall {
    match hold {
        Hold::For(wall) => wall,
        Hold::Until(until) => Wall::from_nanos(until.saturating_sub(entered)),
    }
}

#[derive(Default)]
struct HoldState {
    plan: Vec<TxHold>,
    starts: usize,
    first_start_ns: Option<u64>,
}

type ApplyHook = Box<dyn FnMut(usize) + Send>;
type RxScript = Box<dyn FnMut(usize) -> RxRecv + Send>;
type KeptBlocks = Option<Vec<(i64, Vec<Iq>)>>;

struct ReceiveGate {
    paused: bool,
    active: usize,
}

/// Test-only Device forwarder for recording Session RX ordering on the host clock.
pub struct ProbeDevice {
    inner: Arc<dyn Device>,
    origin: Instant,
    rows: Mutex<Vec<Json>>,
    dropped_rows: AtomicUsize,
    tx_eob_count: AtomicUsize,
    /// The calls in order: `apply`, `apply_failed`, `time_now`, `rx_open`, `tx_open`, `rx_recv`, `rx_stop`, `rx_close` and `drop`.
    control_log: Arc<Mutex<Vec<&'static str>>>,
    /// Replaces what `rx_recv` returns while it holds a sequence; an empty sequence times out.
    rx_script: Mutex<Option<RxScript>>,
    rx_calls: AtomicUsize,
    fail_restoring_apply: AtomicBool,
    /// Called with the count (1-based) of the untimed `apply` about to go to the device.
    before_untimed_apply: Mutex<Option<ApplyHook>>,
    untimed_applies: AtomicUsize,
    /// The untimed `apply` (1-based) that fails, once counted down to 1.
    fail_untimed_apply: Mutex<Option<usize>>,
    tx_generations: Mutex<BTreeMap<usize, u64>>,
    tx_underflow_after_send: AtomicBool,
    tx_underflow_ready: AtomicBool,
    /// The armed holds and what they count (bt2 C7); `None` until `arm_hold`.
    hold: Mutex<Option<HoldState>>,
    /// The first channel of every block `rx_recv` returned, once `keep_rx_blocks` turned keeping on.
    rx_blocks: Mutex<KeptBlocks>,
    gate: Mutex<ReceiveGate>,
    ready: Condvar,
}

struct ResumeReaderOnDrop<'a>(&'a ProbeDevice);

impl Drop for ResumeReaderOnDrop<'_> {
    fn drop(&mut self) {
        self.0.resume_reader();
    }
}

pub fn successful_tx_eob(row: &Json) -> bool {
    if !(row["call"] == "tx_send" && row["eob"].as_bool() == Some(true)) {
        return false;
    }
    let Some(accepted) = row["result"]["accepted_samples"].as_u64() else {
        return false;
    };
    row["sample_counts"].as_array().is_some_and(|counts| !counts.is_empty() && counts.iter().all(|count| count.as_u64() == Some(accepted)))
}

pub fn receive_sample_follows_start(start: &Json, sample: &Json) -> bool {
    start["call"] == "rx_start"
        && start["result"].is_null()
        && sample["call"] == "rx_recv"
        && sample["outcome"]["kind"] == "samples"
        && start["host_after_ns"].as_u64().zip(sample["host_before_ns"].as_u64()).is_some_and(|(finished, begun)| begun >= finished)
        && start["at"].as_i64().zip(sample["outcome"]["first_tick"].as_i64()).is_some_and(|(at, first)| first >= at)
}

/// The `start_index`th (0-based) `rx_start` and the first sample after it, if the probe has both now.
pub fn rx_start_and_sample(probe: &ProbeDevice, start_index: usize) -> Option<Json> {
    probe.with_rows(|rows| {
        let start = rows.iter().filter(|row| row["call"] == "rx_start").nth(start_index)?;
        let sample = rows.iter().find(|row| receive_sample_follows_start(start, row))?;
        Some(json!({ "rx_start": start, "rx_sample": sample }))
    })
}

/// Waits for the `start_index`th (0-based) `rx_start` and the first sample after it; `None` at `timeout`.
pub fn wait_for_rx_start_and_sample(probe: &ProbeDevice, start_index: usize, timeout: Wall) -> Option<Json> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = rx_start_and_sample(probe, start_index) {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Wall::from_millis(1));
    }
}

impl Drop for ProbeDevice {
    fn drop(&mut self) {
        self.control_log.lock().unwrap().push("drop");
    }
}

impl ProbeDevice {
    pub fn new(inner: Arc<dyn Device>) -> ProbeDevice {
        ProbeDevice {
            inner,
            origin: Instant::now(),
            rows: Mutex::new(Vec::new()),
            dropped_rows: AtomicUsize::new(0),
            tx_eob_count: AtomicUsize::new(0),
            control_log: Arc::new(Mutex::new(Vec::new())),
            rx_script: Mutex::new(None),
            rx_calls: AtomicUsize::new(0),
            fail_restoring_apply: AtomicBool::new(false),
            before_untimed_apply: Mutex::new(None),
            untimed_applies: AtomicUsize::new(0),
            fail_untimed_apply: Mutex::new(None),
            tx_generations: Mutex::new(BTreeMap::new()),
            tx_underflow_after_send: AtomicBool::new(false),
            tx_underflow_ready: AtomicBool::new(false),
            hold: Mutex::new(None),
            rx_blocks: Mutex::new(None),
            gate: Mutex::new(ReceiveGate { paused: false, active: 0 }),
            ready: Condvar::new(),
        }
    }

    pub fn host_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// The axis of the stamps that map to device ticks: the inner device's host clock when it is a manual one (so that a stall slept on it moves every stamp, and a stamp
    /// maps through [`Self::clock_anchor`] as it does on hardware), [`Self::host_ns`] otherwise. The `tx_send` rows' stamps, the `time_now` rows' and the anchor's are on it;
    /// the receive rows' stay on `host_ns`, which the S9 bench tool measures real time with. The H4 bench tool reads the `tx_send` rows' stamps and the anchor too, so those
    /// are on this axis there as well; its receive rows are on `host_ns`.
    pub fn clock_ns(&self) -> u64 {
        let host = self.inner.host();
        if host.is_manual() { u64::try_from(host.now().as_nanos()).unwrap_or(u64::MAX) } else { self.host_ns() }
    }

    /// Arms the holds of `plan`; the counts start at zero now, so no send before this call is held or starts a `Midplay`'s clock.
    pub fn arm_hold(&self, plan: Vec<TxHold>) {
        *self.hold.lock().unwrap() = Some(HoldState { plan, ..HoldState::default() });
    }

    /// The hold the send of `class`, entered at `entered`, falls into (zero when none), and the counts it moves.
    fn hold_for(&self, class: SendClass, entered: u64) -> Wall {
        let mut state = self.hold.lock().unwrap();
        let Some(state) = state.as_mut() else { return Wall::ZERO };
        if class == SendClass::Start {
            state.starts += 1;
            state.first_start_ns.get_or_insert(entered);
        }
        // The storm's hold: every start, its plan entry kept (bt2 C7's S8).
        if class == SendClass::Start {
            if let Some(hold) = state.plan.iter().find_map(|hold| match *hold {
                TxHold::EveryStart(hold) => Some(hold),
                _ => None,
            }) {
                return hold_wall(hold, entered);
            }
        }
        let (starts, first) = (state.starts, state.first_start_ns);
        let fires = |hold: &TxHold| match *hold {
            TxHold::Start { nth, .. } => class == SendClass::Start && starts == nth,
            TxHold::Midplay { after, .. } => class == SendClass::Continuation && first.is_some_and(|first| entered.saturating_sub(first) >= after.as_nanos() as u64),
            TxHold::EveryStart(_) => false,
        };
        let Some(at) = state.plan.iter().position(fires) else { return Wall::ZERO };
        let hold = match state.plan.remove(at) {
            TxHold::Start { hold, .. } | TxHold::Midplay { hold, .. } => hold,
            TxHold::EveryStart(_) => unreachable!("EveryStart never fires the one-shot plan"),
        };
        hold_wall(hold, entered)
    }

    /// The rows in order. Their `host_*_ns` stamps are on two axes: `tx_send`, `time_now` and the anchor (`clock_anchor`) are on [`Self::clock_ns`], while `rx_start`,
    /// `rx_stop`, `rx_recv` and `device_drain` are on [`Self::host_ns`]. The two are one axis on a real clock; they differ only on a manual one.
    pub fn rows(&self) -> Vec<Json> {
        self.rows.lock().unwrap().clone()
    }

    /// Reads the rows under the lock, without cloning them: the receive thread's `record` waits for the read, so keep `read` short.
    pub fn with_rows<R>(&self, read: impl FnOnce(&[Json]) -> R) -> R {
        read(&self.rows.lock().unwrap())
    }

    /// Records `PROBE_ROW_LIMIT + 1` rows, so that the last is dropped (the state a long receive leaves).
    pub fn overfill(&self) {
        for index in 0..=PROBE_ROW_LIMIT {
            self.record(json!({ "index": index }));
        }
    }

    /// Forgets the rows so far; the drop counter keeps its count.
    pub fn clear_rows(&self) {
        self.rows.lock().unwrap().clear();
    }

    /// An error naming the count when rows were dropped at the limit: a search of an incomplete record would report a start that never came.
    pub fn rows_complete(&self) -> Result<(), String> {
        match self.dropped_rows() {
            0 => Ok(()),
            dropped => Err(format!("the probe dropped {dropped} rows at its limit of {PROBE_ROW_LIMIT}; its record is incomplete")),
        }
    }

    /// From now on `rx_recv` also keeps the samples it returns, before any publication truncation.
    pub fn keep_rx_blocks(&self) {
        *self.rx_blocks.lock().unwrap() = Some(Vec::new());
    }

    /// The kept blocks, in the order `rx_recv` returned them: (first tick, channel 0's samples).
    pub fn rx_blocks(&self) -> Vec<(i64, Vec<Iq>)> {
        self.rx_blocks.lock().unwrap().clone().unwrap_or_default()
    }

    pub fn dropped_rows(&self) -> usize {
        self.dropped_rows.load(Ordering::Acquire)
    }

    pub fn count(&self, call: &str) -> usize {
        self.rows.lock().unwrap().iter().filter(|row| row["call"] == call).count()
    }

    /// Makes the `nth` untimed `apply` from now (1-based) fail, not lost.
    pub fn fail_untimed_apply(&self, nth: usize) {
        *self.fail_untimed_apply.lock().unwrap() = Some(nth);
    }

    pub fn control_log(&self) -> Vec<&'static str> {
        self.control_log.lock().unwrap().clone()
    }

    /// The log, which outlives the device (it records the `drop`).
    pub fn log_handle(&self) -> Arc<Mutex<Vec<&'static str>>> {
        self.control_log.clone()
    }

    /// From now on `rx_recv` returns these in order, then `Timeout`, and never reaches the inner device.
    pub fn script_rx(&self, outcomes: Vec<RxRecv>) {
        let mut outcomes: VecDeque<RxRecv> = outcomes.into();
        self.script_rx_with(move |_| {
            outcomes.pop_front().unwrap_or_else(|| {
                std::thread::sleep(Wall::from_millis(1));
                RxRecv::Timeout
            })
        });
    }

    /// From now on `rx_recv` returns what `script` says for each call (1-based count), and never reaches the inner
    /// device; the script may block, to hold a call in flight.
    pub fn script_rx_with(&self, script: impl FnMut(usize) -> RxRecv + Send + 'static) {
        *self.rx_script.lock().unwrap() = Some(Box::new(script));
    }

    /// Makes every untimed `apply` that restores the unstreamed settings (rate, frequency, gain and antenna at once) fail.
    pub fn fail_restoring_apply(&self) {
        self.fail_restoring_apply.store(true, Ordering::Release);
    }

    /// Runs `hook` with the count of each untimed `apply` just before it goes to the device.
    pub fn before_untimed_apply(&self, hook: impl FnMut(usize) + Send + 'static) {
        *self.before_untimed_apply.lock().unwrap() = Some(Box::new(hook));
    }

    pub fn tx_eob_count(&self) -> usize {
        self.tx_eob_count.load(Ordering::Acquire)
    }

    /// Injects one underflow report after the first successful experiment TX send.
    pub fn underflow_after_tx_send(&self) {
        self.tx_underflow_after_send.store(true, Ordering::Release);
    }

    /// A bracketed device-time sample for the H4 dispatch observation.
    pub fn clock_anchor(&self) -> Result<(i64, u64, u64), DeviceError> {
        self.read_time_now("h4_anchor")
    }

    pub fn inner(&self) -> &dyn Device {
        &*self.inner
    }

    pub fn pause_reader(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.paused = true;
        while gate.active != 0 {
            gate = self.ready.wait(gate).unwrap();
        }
    }

    pub fn resume_reader(&self) {
        self.gate.lock().unwrap().paused = false;
        self.ready.notify_all();
    }

    /// Pauses the Provider's reader, drains the device streamer, then resumes it.
    pub fn drain_without_reader(&self, stream: usize, request: usize, timeout: Wall, limit: Wall) -> Vec<Json> {
        self.pause_reader();
        let _resume = ResumeReaderOnDrop(self);

        let begun = Instant::now();
        let mut idle = 0;
        let mut drained = Vec::new();
        while begun.elapsed() < limit && idle < 2 {
            let before = self.host_ns();
            let packet_samples = self.inner.rx_packet_samples(stream);
            let result = self.inner.rx_recv(stream, request, timeout, false);
            let after = self.host_ns();
            let outcome = rx_outcome(&result);
            if outcome["kind"] == "timeout" {
                idle += 1;
            } else {
                idle = 0;
            }
            let row = json!({
                "call": "device_drain", "stream": stream, "request_samples": request,
                "packet_samples": packet_samples, "timeout_ns": timeout.as_nanos(),
                "host_before_ns": before, "host_after_ns": after,
                "outcome": outcome,
            });
            self.record(row.clone());
            drained.push(row);
        }
        drained
    }

    fn record(&self, row: Json) {
        self.record_with(row, |_| {});
    }

    /// Keeps `row` after `finish` has stamped it under the record's lock (so that a wait for the lock is inside any stamp `finish` takes).
    fn record_with(&self, mut row: Json, finish: impl FnOnce(&mut Json)) {
        let eob = row["call"] == "tx_send" && row["eob"].as_bool().unwrap_or(false);
        let mut rows = self.rows.lock().unwrap();
        finish(&mut row);
        if rows.len() < PROBE_ROW_LIMIT {
            rows.push(row);
        } else {
            self.dropped_rows.fetch_add(1, Ordering::Release);
        }
        drop(rows);
        if eob {
            self.tx_eob_count.fetch_add(1, Ordering::Release);
        }
    }

    fn read_time_now(&self, purpose: &str) -> Result<(i64, u64, u64), DeviceError> {
        self.read_time_now_from(purpose, None)
    }

    /// One inner `time_now`, recorded as a row. `entered` is the instant the whole call began, when this read is the Module's own (`Device::time_now`): the row then also
    /// carries `host_entered_ns` and `host_exit_ns`, the stamps of the whole call. The Authority brackets its read with its own host reads around that call
    /// (`authority.rs` `DeviceTime::read`), so its bracket includes the log push and the wait for the record's lock, which the inner call's stamps leave out; the exit is
    /// stamped under that lock.
    fn read_time_now_from(&self, purpose: &str, entered: Option<u64>) -> Result<(i64, u64, u64), DeviceError> {
        let host_before_ns = self.clock_ns();
        let result = self.inner.time_now();
        let host_after_ns = self.clock_ns();
        let value = match &result {
            Ok(tick) => json!({ "value": tick }),
            Err(error) => json!({ "error": error.message, "lost": error.lost }),
        };
        self.record_with(
            json!({ "call": "time_now", "purpose": purpose, "host_before_ns": host_before_ns,
            "host_after_ns": host_after_ns, "result": value }),
            |row| {
                if let Some(entered) = entered {
                    row["host_entered_ns"] = json!(entered);
                    row["host_exit_ns"] = json!(self.clock_ns());
                }
            },
        );
        result.map(|tick| (tick, host_before_ns, host_after_ns))
    }
}

fn rx_outcome(result: &RxRecv) -> Json {
    match result {
        RxRecv::Samples { first_tick, samples } => json!({
            "kind": "samples", "first_tick": first_tick,
            "samples": samples.first().map_or(0, Vec::len),
        }),
        RxRecv::Overflow { out_of_sequence } => json!({ "kind": "overflow", "out_of_sequence": out_of_sequence }),
        RxRecv::LateCommand => json!({ "kind": "late_command" }),
        RxRecv::Alignment => json!({ "kind": "alignment" }),
        RxRecv::BadPacket => json!({ "kind": "bad_packet" }),
        RxRecv::Timeout => json!({ "kind": "timeout" }),
        RxRecv::Failed(error) => json!({ "kind": "failed", "lost": error.lost, "message": error.message }),
    }
}

impl Device for ProbeDevice {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> Fidelity {
        self.inner.fidelity()
    }
    fn master_clock_rate(&self) -> u64 {
        self.inner.master_clock_rate()
    }
    fn channels(&self, dir: Dir) -> usize {
        self.inner.channels(dir)
    }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> {
        self.inner.front_end(dir, chan)
    }
    fn serial(&self) -> Result<String, DeviceError> {
        self.inner.serial()
    }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> {
        self.inner.set_subdev_spec(dir, markup)
    }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        self.inner.rates(dir, chan)
    }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> {
        self.inner.set_sources(clock, time)
    }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> {
        self.inner.set_time_zero(at_next_pps)
    }
    fn time_now(&self) -> Result<i64, DeviceError> {
        let entered = self.clock_ns();
        self.control_log.lock().unwrap().push("time_now");
        self.read_time_now_from("device_call", Some(entered)).map(|(tick, _, _)| tick)
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, dir: Dir, chan: usize, settings: &Settings) -> Result<Applied, DeviceError> {
        let count = self.untimed_applies.fetch_add(1, Ordering::AcqRel) + 1;
        let hook = self.before_untimed_apply.lock().unwrap().take();
        if let Some(mut hook) = hook {
            hook(count);
            *self.before_untimed_apply.lock().unwrap() = Some(hook);
        }
        if self.fail_restoring_apply.load(Ordering::Acquire) && settings.rate.is_some() && settings.freq.is_some() && settings.gain.is_some() && settings.antenna.is_some() {
            self.control_log.lock().unwrap().push("apply_failed");
            return Err(DeviceError::failed("probe: scripted restore failure"));
        }
        let mut failing = self.fail_untimed_apply.lock().unwrap();
        if let Some(remaining) = failing.as_mut() {
            *remaining -= 1;
            if *remaining == 0 {
                *failing = None;
                self.control_log.lock().unwrap().push("apply_failed");
                return Err(DeviceError::failed("probe: scripted apply failure"));
            }
        }
        drop(failing);
        self.control_log.lock().unwrap().push("apply");
        self.inner.apply(dir, chan, settings)
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.control_log.lock().unwrap().push("rx_open");
        self.inner.rx_open(stream, channels)
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        let before = self.host_ns();
        let result = self.inner.rx_start(stream, at);
        let after = self.host_ns();
        self.record(json!({ "call": "rx_start", "stream": stream, "at": at, "host_before_ns": before, "host_after_ns": after, "result": result.as_ref().err().map(|error| error.message.clone()) }));
        result
    }
    fn rx_packet_samples(&self, stream: usize) -> usize {
        self.inner.rx_packet_samples(stream)
    }
    fn rx_stop(&self, stream: usize) -> Result<(), DeviceError> {
        self.control_log.lock().unwrap().push("rx_stop");
        let before = self.host_ns();
        let result = self.inner.rx_stop(stream);
        let after = self.host_ns();
        self.record(json!({ "call": "rx_stop", "stream": stream, "host_before_ns": before, "host_after_ns": after, "result": result.as_ref().err().map(|error| error.message.clone()) }));
        result
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall, one_packet: bool) -> RxRecv {
        self.control_log.lock().unwrap().push("rx_recv");
        let count = self.rx_calls.fetch_add(1, Ordering::AcqRel) + 1;
        let script = self.rx_script.lock().unwrap().take();
        if let Some(mut script) = script {
            // The script runs unlocked, so that it can block while another thread works.
            let outcome = script(count);
            *self.rx_script.lock().unwrap() = Some(script);
            return outcome;
        }
        {
            let mut gate = self.gate.lock().unwrap();
            while gate.paused {
                gate = self.ready.wait(gate).unwrap();
            }
            gate.active += 1;
        }
        let before = self.host_ns();
        let packet_samples = self.inner.rx_packet_samples(stream);
        let result = self.inner.rx_recv(stream, n, timeout, one_packet);
        let after = self.host_ns();
        if let (RxRecv::Samples { first_tick, samples }, Some(blocks)) = (&result, self.rx_blocks.lock().unwrap().as_mut()) {
            blocks.push((*first_tick, samples[0].clone()));
        }
        self.record(json!({
            "call": "rx_recv", "stream": stream, "request_samples": n, "timeout_ns": timeout.as_nanos(),
            "packet_samples": packet_samples, "host_before_ns": before, "host_after_ns": after,
            "outcome": rx_outcome(&result),
        }));
        let mut gate = self.gate.lock().unwrap();
        gate.active -= 1;
        self.ready.notify_all();
        result
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.control_log.lock().unwrap().push("tx_open");
        let result = self.inner.tx_open(stream, channels);
        let generation = if result.is_ok() {
            let mut generations = self.tx_generations.lock().unwrap();
            let generation = generations.entry(stream).or_default();
            *generation += 1;
            *generation
        } else {
            self.tx_generations.lock().unwrap().get(&stream).copied().unwrap_or(0)
        };
        self.record(json!({ "call": "tx_open", "stream": stream, "channels": channels, "generation": generation,
            "result": result.as_ref().err().map(|error| error.message.clone()) }));
        result
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        let generation = self.tx_generations.lock().unwrap().get(&stream).copied().unwrap_or(0);
        // The call is entered, the hold (if any) is slept on the inner clock, and only then does the inner device see the send: `host_entered_ns` is where a stall's
        // evidence window begins, `host_before_ns` and `host_after_ns` bracket the inner call alone.
        let entered = self.clock_ns();
        let class = send_class(at, sob, eob, samples);
        let held = self.hold_for(class, entered);
        if !held.is_zero() {
            self.inner.host().sleep(held);
        }
        let before = self.clock_ns();
        let result = self.inner.tx_send(stream, samples, at, sob, eob, timeout);
        let after = self.clock_ns();
        let tx_flush = class == SendClass::Flush;
        let result_row = match &result {
            Ok(accepted_samples) => json!({ "accepted_samples": accepted_samples }),
            Err(error) => json!({ "error": error.message }),
        };
        self.record(json!({
            "call": "tx_send", "stream": stream, "generation": generation, "host_entered_ns": entered, "host_before_ns": before, "host_after_ns": after,
            "sample_counts": samples.iter().map(|channel| channel.len()).collect::<Vec<_>>(),
            "at": at, "sob": sob, "eob": eob, "tx_flush": tx_flush, "class": class.name(), "held_ns": held.as_nanos() as u64, "timeout_ns": timeout.as_nanos(), "result": result_row,
        }));
        if !tx_flush && result.as_ref().is_ok_and(|accepted| *accepted > 0) && self.tx_underflow_after_send.swap(false, Ordering::AcqRel) {
            self.tx_underflow_ready.store(true, Ordering::Release);
        }
        result
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> {
        if self.tx_underflow_ready.swap(false, Ordering::AcqRel) {
            return self.inner.time_now().map(|tick| Some(TxReport { code: TxCode::Underflow, tick: Some(tick), channel: 0 }));
        }
        self.inner.tx_async(stream, timeout)
    }
    fn rx_close(&self, stream: usize) {
        self.control_log.lock().unwrap().push("rx_close");
        self.inner.rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        self.inner.tx_close(stream)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn host(&self) -> HostClock {
        self.inner.host()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), ezsdr_radio_uhd::DeviceError> {
        self.inner.raise_thread_priority()
    }
}

#[cfg(all(test, not(feature = "uhd")))]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn successful_tx_eob_requires_an_accepted_eob() {
        assert!(successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[1],"result":{"accepted_samples":1}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[1],"result":{"accepted_samples":0}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"sample_counts":[100],"result":{"accepted_samples":99}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":false,"result":{"accepted_samples":100}})));
        assert!(!successful_tx_eob(&json!({"call":"tx_send","eob":true,"result":{"error":"device error"}})));
        assert!(!successful_tx_eob(&json!({"call":"rx_start","eob":true,"result":{"accepted_samples":0}})));
    }

    #[test]
    fn tx_eob_count_tracks_recorded_eob_sends() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..ezsdr_radio_uhd::FakeConfig::default() }));
        let probe = ProbeDevice::new(inner);
        probe.record(json!({"call":"tx_send","eob":false}));
        probe.record(json!({"call":"rx_stop","eob":true}));
        assert_eq!(probe.tx_eob_count(), 0);
        probe.record(json!({"call":"tx_send","eob":true}));
        assert_eq!(probe.tx_eob_count(), 1);
        assert_eq!(probe.rows().len(), 3);
    }

    #[test]
    fn probe_rows_stop_at_the_limit_and_count_dropped_rows() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..ezsdr_radio_uhd::FakeConfig::default() }));
        let probe = ProbeDevice::new(inner);
        probe.overfill();
        assert_eq!(probe.rows().len(), PROBE_ROW_LIMIT);
        assert_eq!(probe.dropped_rows(), 1);
        assert_eq!(PROBE_ROW_LIMIT, 65_536, "the cap is an independent budget of the bench's trials");
        assert!(probe.rows_complete().unwrap_err().contains("dropped 1 rows"));
        probe.clear_rows();
        probe.record(json!({"call":"rx_start"}));
        assert_eq!((probe.rows().len(), probe.count("rx_start"), probe.dropped_rows()), (1, 1, 1));
    }

    #[test]
    fn an_unfilled_probe_has_complete_rows() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..ezsdr_radio_uhd::FakeConfig::default() }));
        let probe = ProbeDevice::new(inner);
        for index in 0..PROBE_ROW_LIMIT {
            probe.record(json!({ "index": index }));
        }
        assert_eq!(probe.rows_complete(), Ok(()));
    }

    #[test]
    fn rx_start_and_sample_finds_the_nth_start_and_its_own_sample() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..ezsdr_radio_uhd::FakeConfig::default() }));
        let probe = ProbeDevice::new(inner);
        let start = |at: i64, after: u64| json!({"call":"rx_start","at":at,"host_after_ns":after,"result":null});
        let sample = |before: u64, first: i64| json!({"call":"rx_recv","host_before_ns":before,"outcome":{"kind":"samples","first_tick":first}});
        probe.record(start(100, 10));
        probe.record(sample(10, 100));
        probe.record(start(500, 50));
        assert!(rx_start_and_sample(&probe, 1).is_none(), "the second start has no sample yet");
        probe.record(sample(40, 500)); // before the second start finished
        assert!(rx_start_and_sample(&probe, 1).is_none(), "a sample from before the start's host time is not its own");
        probe.record(sample(60, 500));
        assert_eq!(rx_start_and_sample(&probe, 0).unwrap()["rx_sample"]["host_before_ns"], 10);
        let second = rx_start_and_sample(&probe, 1).unwrap();
        assert_eq!((second["rx_start"]["at"].as_i64(), second["rx_sample"]["host_before_ns"].as_u64()), (Some(500), Some(60)));
        assert!(rx_start_and_sample(&probe, 2).is_none());
    }

    #[test]
    fn receive_sample_follows_a_successful_device_start() {
        let start = json!({"call":"rx_start","at":100,"host_after_ns":10,"result":null});
        let sample = json!({"call":"rx_recv","host_before_ns":10,"outcome":{"kind":"samples","first_tick":100}});
        assert!(receive_sample_follows_start(&start, &sample));

        let mut stale_host_sample = sample.clone();
        stale_host_sample["host_before_ns"] = json!(9);
        assert!(!receive_sample_follows_start(&start, &stale_host_sample));
        let mut stale_tick_sample = sample.clone();
        stale_tick_sample["outcome"]["first_tick"] = json!(99);
        assert!(!receive_sample_follows_start(&start, &stale_tick_sample));
        let mut failed_start = start.clone();
        failed_start["result"] = json!("start failed");
        assert!(!receive_sample_follows_start(&failed_start, &sample));
    }

    /// One read through the probe as the Module makes it (`Device::time_now`, recorded with purpose `device_call`).
    fn module_read(probe: &ProbeDevice) {
        probe.time_now().unwrap();
    }

    /// A probe on a manual-clock fake with transmit stream 0 open, and the fake's host clock; a test makes its thread the clock's `drive`r, so that the clock stands still
    /// between its calls and a stall's sleep moves it by exactly that.
    fn tx_probe() -> (ProbeDevice, HostClock) {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig { clock: ezsdr_radio_uhd::ClockMode::Manual, ..ezsdr_radio_uhd::FakeConfig::default() }));
        inner.tx_open(0, &[0]).unwrap();
        let host = inner.host();
        (ProbeDevice::new(inner), host)
    }

    fn sample_rows(probe: &ProbeDevice) -> Vec<Json> {
        probe.rows().into_iter().filter(|row| row["call"] == "tx_send").collect()
    }

    const ONE: Wall = Wall::from_secs(1);

    /// A `tx_send` of `n` samples of 0.25, at a tick far past the fake's time unless `at` says otherwise.
    fn send(probe: &ProbeDevice, n: usize, at: Option<i64>, sob: bool, eob: bool) {
        let wave = vec![[0.25f32, 0.0]; n];
        probe.tx_send(0, &[&wave], at, sob, eob, ONE).unwrap();
    }

    #[test]
    fn probe_send_class_table() {
        let tone = [[0.25f32, 0.0]; 3];
        let zeros = [Iq::default(); 4];
        let two = [[0.25f32, 0.0]; 2];
        let class = |at: Option<i64>, sob, eob, samples: &[Iq]| send_class(at, sob, eob, &[samples]);
        assert_eq!(class(None, true, true, &zeros), SendClass::Flush, "T3's flush");
        assert_eq!(class(Some(10), true, true, &zeros), SendClass::Start, "a timed four-zero start is a start, not a flush");
        assert_eq!(class(None, true, true, &tone), SendClass::Start, "an untimed start with a tone is a start");
        assert_eq!(class(Some(10), true, true, &tone), SendClass::Start, "a short closed first send is a start");
        assert_eq!(class(Some(10), true, false, &tone), SendClass::Start);
        assert_eq!(class(Some(10), false, true, &two), SendClass::Close, "a two-sample end of burst is a close");
        assert_eq!(class(Some(10), false, true, &tone), SendClass::Continuation, "a three-sample end of burst is a continuation");
        assert_eq!(class(Some(10), false, false, &two), SendClass::Continuation, "a short send with no end of burst is a continuation");
        assert_eq!(class(None, false, true, &two), SendClass::Continuation, "an untimed end of burst is no close");
        assert_eq!(class(Some(10), false, false, &tone), SendClass::Continuation);
    }

    #[test]
    fn probe_hold_start_holds_only_the_nth_start() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        probe.arm_hold(vec![TxHold::Start { nth: 2, hold: Hold::For(Wall::from_millis(7)) }]);
        let before = host.now();
        send(&probe, 3, Some(1_000), true, false);
        assert_eq!(host.now(), before, "the first start is not held");
        send(&probe, 3, Some(1_100), false, false);
        send(&probe, 2, Some(1_103), false, true);
        send(&probe, 3, Some(2_000), true, false);
        assert_eq!(host.now() - before, Wall::from_millis(7), "the second start is held once");
        send(&probe, 3, Some(3_000), true, false);
        assert_eq!(host.now() - before, Wall::from_millis(7), "a third start is not");
        let rows = sample_rows(&probe);
        let held: Vec<(&str, u64)> = rows.iter().map(|row| (row["class"].as_str().unwrap(), row["held_ns"].as_u64().unwrap())).collect();
        assert_eq!(held, [("start", 0), ("continuation", 0), ("close", 0), ("start", 7_000_000), ("start", 0)]);
    }

    #[test]
    fn probe_hold_midplay_picks_a_continuation_after_the_play_time() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        probe.arm_hold(vec![TxHold::Midplay { after: Wall::from_millis(50), hold: Hold::For(Wall::from_millis(9)) }]);
        send(&probe, 3, Some(1_000), true, false);
        host.sleep(Wall::from_millis(20));
        send(&probe, 3, Some(1_003), false, false);
        assert!(sample_rows(&probe).iter().all(|row| row["held_ns"] == 0), "a continuation before `after` is not held");
        host.sleep(Wall::from_millis(30));
        send(&probe, 2, Some(1_006), false, true);
        assert_eq!(sample_rows(&probe)[2]["held_ns"], 0, "a close is not a continuation");
        send(&probe, 3, Some(1_008), false, false);
        let rows = sample_rows(&probe);
        assert_eq!((rows[3]["class"].as_str().unwrap(), rows[3]["held_ns"].as_u64().unwrap()), ("continuation", 9_000_000));
        send(&probe, 3, Some(1_011), false, false);
        assert_eq!(sample_rows(&probe)[4]["held_ns"], 0, "it fires once");
        // No start since arming: no clock to measure `after` from.
        let (idle, idle_host) = tx_probe();
        let _idle_driver = idle_host.drive();
        idle.arm_hold(vec![TxHold::Midplay { after: Wall::ZERO, hold: Hold::For(Wall::from_millis(9)) }]);
        send(&idle, 3, Some(1_003), false, false);
        assert_eq!(sample_rows(&idle)[0]["held_ns"], 0, "a continuation with no start before it is not midplay");
    }

    #[test]
    fn probe_hold_counts_only_from_arming() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        send(&probe, 3, Some(1_000), true, false);
        host.sleep(Wall::from_millis(80));
        probe.arm_hold(vec![TxHold::Start { nth: 1, hold: Hold::For(Wall::from_millis(7)) }, TxHold::Midplay { after: Wall::from_millis(50), hold: Hold::For(Wall::from_millis(3)) }]);
        send(&probe, 3, Some(1_003), false, false);
        assert_eq!(sample_rows(&probe)[1]["held_ns"], 0, "a start before arming counts for neither hold (its clock does not start the midplay's)");
        let before = host.now();
        send(&probe, 3, Some(2_000), true, false);
        assert_eq!(host.now() - before, Wall::from_millis(7), "the first start since arming is the first");
    }

    #[test]
    fn probe_hold_every_start_holds_starts_only_never_a_close_a_flush_or_a_continuation() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        probe.arm_hold(vec![TxHold::EveryStart(Hold::For(Wall::from_millis(7)))]);
        let before = host.now();
        let flush = [Iq::default(); 4];
        probe.tx_send(0, &[&flush], None, true, true, ONE).unwrap();
        send(&probe, 3, Some(1_000), true, false);
        send(&probe, 2, Some(1_003), false, true);
        send(&probe, 3, Some(1_005), false, false);
        send(&probe, 3, Some(2_000), true, false);
        assert_eq!(host.now() - before, Wall::from_millis(14), "both starts are held, nothing else");
        let rows = sample_rows(&probe);
        let held: Vec<(&str, u64)> = rows.iter().map(|row| (row["class"].as_str().unwrap(), row["held_ns"].as_u64().unwrap())).collect();
        assert_eq!(held, [("flush", 0), ("start", 7_000_000), ("close", 0), ("continuation", 0), ("start", 7_000_000)]);
    }

    #[test]
    fn probe_hold_until_is_an_instant_of_the_probe_axis_and_never_a_negative_hold() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        let until = probe.clock_ns() + 25_000_000;
        probe.arm_hold(vec![TxHold::Start { nth: 1, hold: Hold::Until(until) }, TxHold::Start { nth: 2, hold: Hold::Until(until) }]);
        send(&probe, 3, Some(1_000), true, false);
        assert_eq!(probe.clock_ns(), until, "the send returns at the instant");
        send(&probe, 3, Some(2_000), true, false);
        assert_eq!(probe.clock_ns(), until, "an instant already past holds nothing");
        assert_eq!(sample_rows(&probe)[1]["held_ns"], 0);
        assert!(host.now() >= Wall::from_nanos(until));
    }

    #[test]
    fn probe_hold_rows_carry_the_entry_before_and_after_stamps_on_the_inner_clock() {
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        probe.arm_hold(vec![TxHold::Start { nth: 1, hold: Hold::For(Wall::from_millis(40)) }]);
        let entered = probe.clock_ns();
        send(&probe, 3, Some(1_000), true, false);
        let row = &sample_rows(&probe)[0];
        let stamp = |name: &str| row[name].as_u64().unwrap();
        assert_eq!(stamp("host_entered_ns"), entered, "the stamp is taken before the sleep");
        assert_eq!(stamp("host_before_ns") - stamp("host_entered_ns"), 40_000_000, "the hold is the clock's time between the entry and the inner call");
        assert!(stamp("host_after_ns") >= stamp("host_before_ns"));
        assert_eq!((row["class"].as_str(), row["held_ns"].as_u64()), (Some("start"), Some(40_000_000)));
    }

    #[test]
    fn probe_stamps_map_through_the_anchor_on_the_manual_clock() {
        // Terra v3 2: a value mapped through `clock_anchor` is stamped on the anchor's axis. The manual clock runs no faster than real time and lags it by the busy time it
        // loses, so a stamp on real elapsed time cannot keep an exact tick equality (one tick = 5 ns) with the anchor's mapped axis; the 40 ms stall is in the mapped stamp.
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        probe.arm_hold(vec![TxHold::Start { nth: 1, hold: Hold::For(Wall::from_millis(40)) }]);
        let mcr = probe.master_clock_rate() as i128;
        let (tick0, before0, after0) = probe.clock_anchor().unwrap();
        send(&probe, 3, Some(1_000), true, false);
        let row = &sample_rows(&probe)[0];
        let (tick1, before1, after1) = probe.clock_anchor().unwrap();
        let map = |stamp: u64| tick0 as i128 + (stamp as i128 - ((before0 + after0) / 2) as i128) * mcr / 1_000_000_000;
        let (mapped, truth) = (map(row["host_after_ns"].as_u64().unwrap()), tick1 as i128 - (((before1 + after1) / 2) as i128 - row["host_after_ns"].as_u64().unwrap() as i128) * mcr / 1_000_000_000);
        assert_eq!(mapped, truth, "the stall's {} ticks are in the mapped stamp", 40_000_000 * mcr / 1_000_000_000);
        assert!(tick1 - tick0 >= (40_000_000 * mcr / 1_000_000_000) as i64);
    }

    #[test]
    fn probe_a_modules_clock_read_row_carries_the_stamps_of_the_whole_call_around_the_inner_ones() {
        // bt2 fix round 2, item 1: the Authority brackets its read around the whole `Device::time_now` call, so the row of that call carries the entry and the exit stamps
        // outside the inner call's; the anchor's reads (`clock_anchor`, purpose `h4_anchor`) keep the inner stamps only.
        let (probe, host) = tx_probe();
        let _driver = host.drive();
        module_read(&probe);
        probe.clock_anchor().unwrap();
        let rows = probe.rows();
        let read = rows.iter().find(|row| row["call"] == "time_now" && row["purpose"] == "device_call").expect("the Module's read");
        let stamp = |name: &str| read[name].as_u64().unwrap_or_else(|| panic!("{name}: {read}"));
        assert!(stamp("host_entered_ns") <= stamp("host_before_ns") && stamp("host_before_ns") <= stamp("host_after_ns") && stamp("host_after_ns") <= stamp("host_exit_ns"), "{read}");
        let anchor = rows.iter().find(|row| row["purpose"] == "h4_anchor").expect("the anchor's read");
        assert!(anchor.get("host_exit_ns").is_none(), "{anchor}");
    }

    #[test]
    fn drain_without_reader_resumes_after_rx_panic() {
        let inner = Arc::new(ezsdr_radio_uhd::FakeDevice::new(ezsdr_radio_uhd::FakeConfig {
            clock: ezsdr_radio_uhd::ClockMode::Manual,
            faults: vec![ezsdr_radio_uhd::FakeFault::RxPanic(Wall::ZERO)],
            ..ezsdr_radio_uhd::FakeConfig::default()
        }));
        inner.rx_open(0, &[0]).unwrap();
        let at = inner.time_now().unwrap() + inner.master_clock_rate() as i64;
        inner.rx_start(0, at).unwrap();
        let probe = ProbeDevice::new(inner);
        let panic = catch_unwind(AssertUnwindSafe(|| probe.drain_without_reader(0, 2_000, Wall::from_millis(1), Wall::from_millis(10))));
        assert!(panic.is_err());
        assert!(!probe.gate.lock().unwrap().paused);
    }
}
