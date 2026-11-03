//! Note 45 experiment E2b (measurement tooling): the speed of a timed receive update, today's owner (uhd-control and uhd-rx) against the
//! new runtime (E2a's brain, `S` and `R_k`), both over one Real-clock `FakeDevice` behind a light timing decorator. Test code only.
//! The definitions are the brief's (`~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2b.md`, with its amendments):
//!   * release window 3 ms (`ezsdr-radio/src/x3xx.rs`); both delays are measured from the host instant of `e - window`;
//!   * entry delay = `Device::apply` entry minus that instant (diagnostic); acceptance delay = `apply` returned `Ok` minus that instant
//!     (binding); applied = the fake's DSP log has the value at tick `e`, matched one-to-one with the table;
//!   * loss = overflow events, missing samples (gap test at the sink reader), publication drops, over one observation window.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value as Json, json};

use ezsdr_kernel::stream::{BackPressure, BlockRef, DataLink, DropCarry, PublishOutcome};

use crate::brain::cell::Refusal;
use crate::brain::core::NotReserved;
use crate::device::{Applied, Device, DeviceError, Dir, FakeDevice, Iq, RxRecv, Settings, TxReport};
use crate::host::HostClock;

mod newrt;
mod runs;
mod today;

/// The release window (`ezsdr-radio/src/x3xx.rs`: `release_window_ns`).
pub(super) const WINDOW_NS: i64 = 3_000_000;
/// The device lead of the booking floor (`profile.rs`: `DEVICE_LEAD_NS`).
pub(super) const LEAD_NS: i64 = 2_000_000;
/// Each update enters its owner this long before its instant (brief amendment 2).
pub(super) const INGRESS_NS: i64 = 50_000_000;
/// An update is outstanding until its apply returns or, if it never does, this long after its instant (it then counts as a failure).
pub(super) const EXPIRY_NS: i64 = 10_000_000;
/// An update that enters its owner later than this before its instant fails.
pub(super) const INGRESS_LATE_NS: i64 = 10_000_000;
/// At most this many updates are booked and not yet applied (bounded lookahead).
pub(super) const LOOKAHEAD: usize = 8;
/// The first table instant after the stream's start of observation.
pub(super) const FIRST_NS: i64 = 300_000_000;
pub(super) const FREQS: [f64; 2] = [2.0e9, 2.001e9];
const LINK_CAPACITY: u32 = 4096;

// ------------------------------------------------------------------------------------------------ histograms

/// A power-of-two histogram of nanosecond durations, lock-free, for the diagnostic runs' cell hold and wait times.
pub(crate) struct Hist {
    buckets: [AtomicU64; 40],
    count: AtomicU64,
    sum: AtomicU64,
    max: AtomicU64,
}

impl Hist {
    const fn new() -> Hist {
        Hist { buckets: [const { AtomicU64::new(0) }; 40], count: AtomicU64::new(0), sum: AtomicU64::new(0), max: AtomicU64::new(0) }
    }

    pub(crate) fn record(&self, d: Duration) {
        let ns = d.as_nanos().min(u64::MAX as u128) as u64;
        self.buckets[(64 - ns.leading_zeros()).min(39) as usize].fetch_add(1, Relaxed);
        self.count.fetch_add(1, Relaxed);
        self.sum.fetch_add(ns, Relaxed);
        self.max.fetch_max(ns, Relaxed);
    }

    fn reset(&self) {
        self.buckets.iter().for_each(|b| b.store(0, Relaxed));
        [&self.count, &self.sum, &self.max].iter().for_each(|a| a.store(0, Relaxed));
    }

    /// The upper bound of the bucket that holds the nearest-rank percentile (`per_mille` of 1000).
    fn upper(&self, per_mille: u64) -> u64 {
        let n = self.count.load(Relaxed);
        let target = (n * per_mille).div_ceil(1000).max(1);
        let mut seen = 0;
        for (i, b) in self.buckets.iter().enumerate() {
            seen += b.load(Relaxed);
            if seen >= target {
                return if i == 0 { 0 } else { 1u64 << i };
            }
        }
        0
    }

    fn json(&self) -> Json {
        let n = self.count.load(Relaxed);
        json!({ "count": n, "mean_ns": self.sum.load(Relaxed).checked_div(n), "max_ns": self.max.load(Relaxed),
                "p50_le_ns": self.upper(500), "p99_le_ns": self.upper(990), "p999_le_ns": self.upper(999) })
    }
}

pub(crate) static CELL_WAIT: Hist = Hist::new();
pub(crate) static CELL_HOLD: Hist = Hist::new();
static INSTRUMENT: AtomicBool = AtomicBool::new(false);

/// Whether the cell guards time their wait and hold (diagnostic runs only).
pub(crate) fn instrument_on() -> bool {
    INSTRUMENT.load(Relaxed)
}

fn set_instrument(on: bool) {
    CELL_WAIT.reset();
    CELL_HOLD.reset();
    R_CELL_WAIT.reset();
    R_CELL_HOLD.reset();
    INSTRUMENT.store(on, Relaxed);
}

/// The receive result and the range of its samples a publication gave the user.
pub(crate) type BlockSink = Box<dyn Fn(&RxRecv, std::ops::Range<usize>) + Send + Sync>;

/// What the new runtime's threads count and hand on in a measurement run (counters only; no per-block row, nothing allocated per block).
pub(crate) struct Bench {
    /// What `R` does with the samples a publication gave the user (the per-block publication work, like today's owner's): the receive result
    /// and the range of its samples the cell accepted, called after the cell accepted the publication and only for that range.
    pub(crate) sink: BlockSink,
    pub(crate) counters: bool,
    pub(crate) publish_retries: AtomicU64,
    pub(crate) stale_rounds: AtomicU64,
    /// Blocks the cell accepted a publication of (with at least one sample, or none).
    pub(crate) publishes: AtomicU64,
    pub(crate) take_dropped: AtomicU64,
    /// The samples a publication discarded, by reason (the protocol's: before the segment's origin, at or after the cut).
    pub(crate) discarded_before_origin: AtomicU64,
    pub(crate) discarded_after_cut: AtomicU64,
    /// Per value (id - 1), the reservation attempts aggregated: bounded by the table, never one record per attempt (an id beyond the table counts
    /// `reserve_dropped`).
    pub(crate) reserve: Mutex<Vec<ReserveAgg>>,
    pub(crate) reserve_dropped: AtomicU64,
    /// Per value, host ns at four stages of its path (diagnostic runs with `stages` or `sample`): the brain's dequeue of its release, the command pushed
    /// to `S`, `S`'s dequeue, and the first brain operation after which the value is no longer in flight (its retirement, as the brain's operation
    /// ends). Preallocated; a stamp is one store.
    pub(crate) stamps: Option<Vec<[AtomicI64; 4]>>,
    /// Reservation attempts rejected at the in-flight bound (counted with `counters`).
    pub(crate) bound_rejections: AtomicU64,
    /// Diagnostic runs with `stamps`: after the brain's latest operation, K's load (the values counted over every chain at the brain's time) and
    /// D's occupancy (the values resident on stream 0), the retirements so far, and the ids resident then (the brain's thread alone writes them).
    pub(crate) k_occupancy: AtomicUsize,
    pub(crate) d_occupancy: AtomicUsize,
    /// Counter runs: the largest K load and D occupancy seen after a reservation attempt (the brain's thread alone writes them).
    pub(crate) k_max: AtomicUsize,
    pub(crate) d_max: AtomicUsize,
    pub(crate) retired: AtomicU64,
    last_in_flight: Mutex<Vec<u64>>,
    /// The host clock's reading, for the stamps.
    pub(crate) clock: Arc<Timing>,
}

/// The stages of a release's path inside the new runtime that `Bench::stage` stamps.
#[derive(Clone, Copy)]
pub(crate) enum Stage {
    BrainTake = 0,
    Pushed = 1,
    STake = 2,
    Retired = 3,
}

/// The reasons a reservation attempt is rejected, in the order of [`reason_index`].
pub(crate) const REASONS: [&str; 8] = ["k", "d", "phase", "channel", "generation", "origin", "cut", "progress"];

pub(crate) fn reason_index(reason: NotReserved) -> usize {
    match reason {
        NotReserved::DeviceBound => 0,
        NotReserved::ChainBound => 1,
        NotReserved::Refused(Refusal::Phase) => 2,
        NotReserved::Refused(Refusal::Channel) => 3,
        NotReserved::Refused(Refusal::Generation) => 4,
        NotReserved::Refused(Refusal::Origin) => 5,
        NotReserved::Refused(Refusal::Cut) => 6,
        NotReserved::Refused(Refusal::Progress) => 7,
    }
}

/// One value's reservation attempts: the first attempt's host ns, the reserving attempt's (0: never reserved), the host ns of its first rejection at
/// K and at D (0: none), the attempts, and the rejections by reason.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct ReserveAgg {
    pub(crate) first_ns: i64,
    pub(crate) reserved_ns: i64,
    pub(crate) first_k_ns: i64,
    pub(crate) first_d_ns: i64,
    pub(crate) attempts: u32,
    pub(crate) by_reason: [u32; REASONS.len()],
}

impl Bench {
    pub(crate) fn count(&self, counter: &AtomicU64) {
        if self.counters {
            counter.fetch_add(1, Relaxed);
        }
    }

    pub(crate) fn add(&self, counter: &AtomicU64, n: u64) {
        if self.counters {
            counter.fetch_add(n, Relaxed);
        }
    }

    /// One reservation attempt of the brain (called outside every cell lock), with the reason when it was rejected and K's load and D's occupancy
    /// after the attempt.
    pub(crate) fn reserve_attempt(&self, id: u64, reserved: bool, reason: Option<NotReserved>, k_load: usize, d_load: usize) {
        if !self.counters {
            return;
        }
        self.k_max.fetch_max(k_load, Relaxed);
        self.d_max.fetch_max(d_load, Relaxed);
        let at = self.clock.ns();
        let mut table = self.reserve.lock().unwrap_or_else(|e| e.into_inner());
        let Some(agg) = (id as usize).checked_sub(1).and_then(|i| table.get_mut(i)) else {
            self.reserve_dropped.fetch_add(1, Relaxed);
            return;
        };
        if agg.attempts == 0 {
            agg.first_ns = at;
        }
        agg.attempts += 1;
        if reserved {
            agg.reserved_ns = at;
        } else if let Some(reason) = reason {
            agg.by_reason[reason_index(reason)] += 1;
            match reason {
                NotReserved::DeviceBound => {
                    agg.first_k_ns = if agg.first_k_ns == 0 { at } else { agg.first_k_ns };
                    self.bound_rejections.fetch_add(1, Relaxed);
                }
                NotReserved::ChainBound => {
                    agg.first_d_ns = if agg.first_d_ns == 0 { at } else { agg.first_d_ns };
                    self.bound_rejections.fetch_add(1, Relaxed);
                }
                NotReserved::Refused(_) => {}
            }
        }
    }

    /// Stamps `stage` of value `id` (diagnostic runs with `stages`; an id beyond the table is not stamped).
    pub(crate) fn stage(&self, stage: Stage, id: u64) {
        if let Some(slot) = self.stamps.as_ref().and_then(|stamps| stamps.get((id as usize).wrapping_sub(1))) {
            slot[stage as usize].store(self.clock.ns(), Relaxed);
        }
    }

    /// The brain's operation has run: `ids` are the values resident on stream 0 now (D's occupancy) and `k_load` is K's load. Each value that was
    /// resident after the operation before and is not now retired during this operation (stamped at the operation's end). Diagnostic runs with
    /// `stamps` only.
    pub(crate) fn observe_in_flight(&self, ids: &[u64], k_load: usize) {
        if self.stamps.is_none() {
            return;
        }
        self.k_occupancy.store(k_load, Relaxed);
        self.d_occupancy.store(ids.len(), Relaxed);
        let mut last = self.last_in_flight.lock().unwrap_or_else(|e| e.into_inner());
        for id in last.iter().filter(|id| !ids.contains(id)) {
            self.stage(Stage::Retired, *id);
            self.retired.fetch_add(1, Relaxed);
        }
        last.clear();
        last.extend_from_slice(ids);
    }
}

/// `R`'s own cell waits and holds, per acquisition (the view and the write of each block), recorded after the lock is released (diagnostic runs).
pub(crate) static R_CELL_WAIT: Hist = Hist::new();
pub(crate) static R_CELL_HOLD: Hist = Hist::new();

// ------------------------------------------------------------------------------------------------ the decorator

/// One timed `apply`: the instant `e`, the host time at the call's entry and at its return, and its result.
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    pub(super) e: i64,
    pub(super) entry_ns: i64,
    pub(super) ret_ns: i64,
    pub(super) ok: bool,
    pub(super) lost: bool,
    /// The frequency the call carried: the booking as the device saw it.
    pub(super) freq: f64,
    /// The owner's identity of the request this call carries (the new runtime's command id, tagged by `S` just before the call), 0 when the owner
    /// tags none: today's owner exposes no booking identity without a production change.
    pub(super) id: u64,
}

#[derive(Default)]
pub(super) struct Counts {
    pub(super) calls: AtomicU64,
    pub(super) blocks: AtomicU64,
    pub(super) samples: AtomicU64,
    pub(super) overflows: AtomicU64,
    pub(super) other: AtomicU64,
    /// The device tick after the last sample the receive returned, and the ticks of one sample (for the backlog probe).
    pub(super) last_end: AtomicI64,
    pub(super) tps: AtomicI64,
}

/// A light timing decorator over a device: the timed applies' host times, and the receive results' counts. Bounded: the rows are
/// preallocated (a full table counts `dropped_rows`), and a receive allocates nothing.
pub(super) struct Timing {
    inner: Arc<dyn Device>,
    host: HostClock,
    rows: Mutex<Vec<Row>>,
    /// The instants of the timed applies that have returned (the ingress rule reads it: an update is outstanding until its apply returns).
    returned: Mutex<std::collections::BTreeSet<i64>>,
    cap: usize,
    pub(super) dropped_rows: AtomicU64,
    pub(super) untimed: AtomicU64,
    pub(super) counts: Counts,
    counting: bool,
    /// The request identity the calling thread set for its next `apply` (taken by that `apply`): one applier thread per run.
    tag: AtomicU64,
}

impl Timing {
    pub(super) fn new(inner: Arc<dyn Device>, cap: usize, counting: bool) -> Arc<Timing> {
        let host = inner.host();
        Arc::new(Timing {
            inner,
            host,
            rows: Mutex::new(Vec::with_capacity(cap)),
            returned: Mutex::new(Default::default()),
            cap,
            dropped_rows: AtomicU64::new(0),
            untimed: AtomicU64::new(0),
            counts: Counts::default(),
            counting,
            tag: AtomicU64::new(0),
        })
    }

    /// The next `apply` carries request `id` (the owner's identity of the booking it makes).
    pub(super) fn tag(&self, id: u64) {
        self.tag.store(id, Relaxed);
    }

    pub(super) fn ns(&self) -> i64 {
        self.host.now().as_nanos() as i64
    }

    /// Whether a timed apply for instant `e` has returned.
    pub(super) fn has_returned(&self, e: i64) -> bool {
        self.returned.lock().unwrap_or_else(|e| e.into_inner()).contains(&e)
    }

    pub(super) fn rows(&self) -> Vec<Row> {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Device for Timing {
    fn describe(&self) -> Json {
        self.inner.describe()
    }
    fn fidelity(&self) -> ezsdr_kernel::module_api::Fidelity {
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
        self.inner.time_now()
    }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> {
        self.inner.ref_locked()
    }
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        let id = self.tag.swap(0, Relaxed);
        let entry_ns = self.ns();
        let result = self.inner.apply(dir, chan, s, at);
        let ret_ns = self.ns();
        match at {
            Some(e) => {
                self.returned.lock().unwrap_or_else(|e| e.into_inner()).insert(e);
                let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
                if rows.len() < self.cap {
                    rows.push(Row { e, entry_ns, ret_ns, ok: result.is_ok(), lost: result.as_ref().err().is_some_and(|error| error.lost), freq: s.freq.unwrap_or(f64::NAN), id });
                } else {
                    self.dropped_rows.fetch_add(1, Relaxed);
                }
            }
            None => {
                self.untimed.fetch_add(1, Relaxed);
            }
        }
        result
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.inner.rx_open(stream, channels)
    }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        self.inner.rx_start(stream, at)
    }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> {
        self.inner.rx_start_now(stream)
    }
    fn rx_packet_samples(&self, stream: usize) -> usize {
        self.inner.rx_packet_samples(stream)
    }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        self.inner.rx_stop(stream, at)
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Duration, one_packet: bool) -> RxRecv {
        let result = self.inner.rx_recv(stream, n, timeout, one_packet);
        if self.counting {
            self.counts.calls.fetch_add(1, Relaxed);
            match &result {
                RxRecv::Samples { first_tick, samples } => {
                    let len = samples.first().map_or(0, Vec::len) as i64;
                    self.counts.blocks.fetch_add(1, Relaxed);
                    self.counts.samples.fetch_add(len as u64, Relaxed);
                    self.counts.last_end.store(first_tick + len * self.counts.tps.load(Relaxed), Relaxed);
                }
                RxRecv::Overflow { .. } => {
                    self.counts.overflows.fetch_add(1, Relaxed);
                }
                RxRecv::Timeout => {}
                _ => {
                    self.counts.other.fetch_add(1, Relaxed);
                }
            }
        }
        result
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.inner.tx_open(stream, channels)
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<TxReport>, DeviceError> {
        self.inner.tx_async(stream, timeout)
    }
    fn rx_close(&self, stream: usize) {
        self.inner.rx_close(stream)
    }
    fn tx_close(&self, stream: usize) {
        self.inner.tx_close(stream)
    }
    fn close_streams(&self) {
        self.inner.close_streams()
    }
    fn host(&self) -> HostClock {
        self.host.clone()
    }
    fn mark_lost(&self) {
        self.inner.mark_lost()
    }
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        self.inner.raise_thread_priority()
    }
}

// ------------------------------------------------------------------------------------------------ the publishing end of the link

/// The link both owners publish to, seen at its publishing end: a block's index is its first sample time (the sample clock's count from the
/// segment's origin, which both owners write), so a jump past the end of the block before is a gap in the published stream, counted here once:
/// the leading gap (the first block's index) and each later one. One publisher thread per link.
pub(super) struct Tap {
    inner: Arc<dyn DataLink>,
    /// The end index of the last block published.
    end: AtomicI64,
    first: AtomicI64,
    missing: AtomicU64,
    blocks: AtomicU64,
    samples: AtomicU64,
}

/// What the tap saw: the published stream's gaps and its extent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TapStats {
    pub(super) blocks: u64,
    pub(super) samples: u64,
    /// The leading gap and the gaps between blocks, in samples.
    pub(super) missing: u64,
    /// The end index of the last block (0 before the first).
    pub(super) end: i64,
}

impl Tap {
    pub(super) fn new(inner: Arc<dyn DataLink>) -> Arc<Tap> {
        Arc::new(Tap { inner, end: AtomicI64::new(0), first: AtomicI64::new(i64::MIN), missing: AtomicU64::new(0), blocks: AtomicU64::new(0), samples: AtomicU64::new(0) })
    }

    pub(super) fn stats(&self) -> TapStats {
        TapStats { blocks: self.blocks.load(Relaxed), samples: self.samples.load(Relaxed), missing: self.missing.load(Relaxed), end: self.end.load(Relaxed) }
    }
}

impl DataLink for Tap {
    fn publish(&self, b: BlockRef) -> PublishOutcome {
        let tp = b.first_sample_time();
        let k = tp.ticks_in(tp.domain()).unwrap_or(0); // own domain: the gap test reads each block in its SampleClock
        let len = i64::from(b.header().len);
        let expected = self.end.load(Relaxed);
        if k > expected {
            self.missing.fetch_add((k - expected) as u64, Relaxed);
        }
        self.first.compare_exchange(i64::MIN, k, Relaxed, Relaxed).ok();
        self.end.store(expected.max(k + len), Relaxed);
        self.blocks.fetch_add(1, Relaxed);
        self.samples.fetch_add(len as u64, Relaxed);
        self.inner.publish(b)
    }
    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.inner.receive()
    }
    fn drops(&self) -> u64 {
        self.inner.drops()
    }
    fn take_drop_carry(&self) -> DropCarry {
        self.inner.take_drop_carry()
    }
    fn policy(&self) -> BackPressure {
        self.inner.policy()
    }
}

// ------------------------------------------------------------------------------------------------ time mapping

/// The device tick <-> host nanosecond map of a Real-clock fake with `drift_ppm = 0` (ticks follow the host exactly), anchored by the
/// tightest of several `time_now` brackets; `unc_ns` is that bracket's half width.
#[derive(Clone, Copy, Debug)]
pub(super) struct Offset {
    pub(super) mcr: u64,
    ref_tick: i64,
    ref_ns: i64,
    pub(super) unc_ns: i64,
}

impl Offset {
    pub(super) fn establish(fake: &FakeDevice, mcr: u64) -> Offset {
        let host = fake.host();
        let ns = || host.now().as_nanos() as i64;
        let mut best: Option<(i64, i64, i64)> = None;
        for _ in 0..25 {
            let (before, tick, after) = (ns(), fake.time_now().expect("the fake's clock"), ns());
            if best.is_none_or(|(b0, _, b1)| after - before < b1 - b0) {
                best = Some((before, tick, after));
            }
        }
        let (b0, tick, b1) = best.expect("a bracket");
        // The bracket's half width plus one device tick: ticks are whole, so a host instant maps to a tick only to within one.
        Offset { mcr, ref_tick: tick, ref_ns: (b0 + b1) / 2, unc_ns: (b1 - b0) / 2 + (1_000_000_000 + mcr as i64 - 1) / mcr as i64 }
    }

    pub(super) fn ns_of(&self, tick: i64) -> i64 {
        self.ref_ns + ((i128::from(tick) - i128::from(self.ref_tick)) * 1_000_000_000 / i128::from(self.mcr)) as i64
    }

    /// The first tick at or after host time `ns`.
    pub(super) fn tick_ceil(&self, ns: i64) -> i64 {
        let num = i128::from(ns - self.ref_ns) * i128::from(self.mcr);
        self.ref_tick + (num + 999_999_999).div_euclid(1_000_000_000) as i64
    }

    pub(super) fn tick_floor(&self, ns: i64) -> i64 {
        self.ref_tick + (i128::from(ns - self.ref_ns) * i128::from(self.mcr)).div_euclid(1_000_000_000) as i64
    }

    pub(super) fn ticks(&self, ns: i64) -> i64 {
        (i128::from(ns) * i128::from(self.mcr) / 1_000_000_000) as i64
    }
}

// ------------------------------------------------------------------------------------------------ the table

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// An update every 5 ms.
    Spaced,
    /// Groups of 4 updates 100 us apart, one group every 10 ms.
    Bursty,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Kind::Spaced => "spaced",
            Kind::Bursty => "bursty",
        }
    }

    /// Update `i`'s instant after the table's first.
    pub(super) fn rel_ns(self, i: usize) -> i64 {
        match self {
            Kind::Spaced => i as i64 * 5_000_000,
            Kind::Bursty => (i / 4) as i64 * 10_000_000 + (i % 4) as i64 * 100_000,
        }
    }
}

/// One table row with its instant in both axes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Upd {
    pub(super) i: usize,
    pub(super) e: i64,
    pub(super) e_ns: i64,
    pub(super) freq: f64,
}

pub(super) fn table(kind: Kind, count: usize, off: &Offset, base_ns: i64) -> Vec<Upd> {
    (0..count)
        .map(|i| {
            let e = off.tick_ceil(base_ns + kind.rel_ns(i));
            Upd { i, e, e_ns: off.ns_of(e), freq: FREQS[i % 2] }
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------ statistics

/// The nearest-rank zero-based index of the `per_mille`-th permille of `n` values (p99 = 990, p99.9 = 999; 9,989 of 10,000).
pub(super) fn rank_index(n: usize, per_mille: usize) -> usize {
    (n * per_mille).div_ceil(1000).max(1) - 1
}

/// `{count, p50, p99, p99.9, max}` of nanosecond values, in microseconds.
pub(super) fn dist(values: &[i64]) -> Json {
    let mut v = values.to_vec();
    v.sort_unstable();
    let us = |ns: i64| ns as f64 / 1000.0;
    match v.len() {
        0 => json!({ "count": 0 }),
        n => json!({ "count": n, "p50_us": us(v[rank_index(n, 500)]), "p99_us": us(v[rank_index(n, 990)]), "p999_us": us(v[rank_index(n, 999)]), "max_us": us(v[n - 1]), "min_us": us(v[0]) }),
    }
}

pub(super) fn p99_ns(values: &[i64]) -> Option<i64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    (!v.is_empty()).then(|| v[rank_index(v.len(), 990)])
}

/// Why an update is not accounted as applied at its instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Why {
    /// No timed `apply` carried its instant.
    NoApply,
    /// The call was made twice for one instant.
    Duplicate,
    /// The device refused the call (not lost).
    Refused,
    /// The call failed with the device lost.
    Lost,
    /// The call returned `Ok` and the DSP log has no entry at its instant with its frequency.
    NotInDsp,
}

#[derive(Debug)]
pub(super) struct Accounting {
    /// Per update: the matched row (if any) and why it failed (`None` = applied at its instant).
    pub(super) per: Vec<(Option<Row>, Option<Why>)>,
    /// Timed `apply` calls whose instant is in no table row.
    pub(super) unexpected_rows: usize,
    /// Timed DSP entries no table row accounts for.
    pub(super) extra_dsp: usize,
    /// The first of those entries, `(tick, frequency)`: listed, never used to label an update.
    pub(super) extra_dsp_entries: Vec<(i64, f64)>,
}

impl Accounting {
    pub(super) fn failures(&self) -> usize {
        self.per.iter().filter(|(_, why)| why.is_some()).count()
    }
}

/// Matches the table with the decorator's rows and the fake's DSP log (`(tick, frequency, timed)`) one to one by instant and frequency:
/// never by when the log was read.
pub(super) fn account(table: &[Upd], rows: &[Row], dsp: &[(i64, f64, bool)]) -> Accounting {
    let mut by_e: std::collections::BTreeMap<i64, Vec<Row>> = std::collections::BTreeMap::new();
    for row in rows {
        by_e.entry(row.e).or_default().push(*row);
    }
    let mut taken: std::collections::BTreeMap<(i64, u64), usize> = std::collections::BTreeMap::new();
    let mut timed_dsp = 0;
    for &(tick, freq, timed) in dsp {
        if timed {
            *taken.entry((tick, freq.to_bits())).or_default() += 1;
            timed_dsp += 1;
        }
    }
    let in_table: std::collections::BTreeSet<i64> = table.iter().map(|u| u.e).collect();
    // Each update takes the DSP entry at its own instant with its own frequency, one to one; no other entry labels it.
    let mut matched_dsp = 0;
    let per = table
        .iter()
        .map(|u| {
            let in_dsp = taken.get_mut(&(u.e, u.freq.to_bits())).filter(|n| **n > 0).map(|n| *n -= 1).is_some();
            matched_dsp += usize::from(in_dsp);
            let calls = by_e.get(&u.e).map(Vec::as_slice).unwrap_or(&[]);
            let why = match (calls, in_dsp) {
                ([], _) => Some(Why::NoApply),
                (c, _) if c.len() > 1 => Some(Why::Duplicate),
                ([r], _) if r.lost => Some(Why::Lost),
                ([r], _) if !r.ok => Some(Why::Refused),
                (_, true) => None,
                _ => Some(Why::NotInDsp),
            };
            (calls.first().copied(), why)
        })
        .collect();
    let extra_dsp_entries: Vec<(i64, f64)> = taken.iter().filter(|(_, n)| **n > 0).flat_map(|(&(tick, bits), &n)| std::iter::repeat_n((tick, f64::from_bits(bits)), n)).take(10).collect();
    Accounting { per, unexpected_rows: rows.iter().filter(|r| !in_table.contains(&r.e)).count(), extra_dsp: timed_dsp - matched_dsp, extra_dsp_entries }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_index_is_nearest_rank_zero_based() {
        assert_eq!(rank_index(10_000, 999), 9_989);
        assert_eq!(rank_index(10_000, 990), 9_899);
        assert_eq!(rank_index(10_000, 500), 4_999);
        assert_eq!(rank_index(1, 999), 0);
        assert_eq!(rank_index(100, 990), 98);
    }

    #[test]
    fn dist_reads_the_sorted_ranks() {
        let values: Vec<i64> = (1..=10_000).rev().map(|v| v * 1000).collect();
        let d = dist(&values);
        assert_eq!(
            (d["count"].as_u64(), d["p99_us"].as_f64(), d["p999_us"].as_f64(), d["max_us"].as_f64(), d["p50_us"].as_f64()),
            (Some(10_000), Some(9_900.0), Some(9_990.0), Some(10_000.0), Some(5_000.0))
        );
    }

    fn upd(i: usize, e: i64) -> Upd {
        Upd { i, e, e_ns: e, freq: FREQS[i % 2] }
    }

    fn row(e: i64, ok: bool) -> Row {
        Row { e, entry_ns: 0, ret_ns: 0, ok, lost: false, freq: f64::NAN, id: 0 }
    }

    #[test]
    fn account_counts_every_unapplied_update_as_a_failure() {
        let table: Vec<Upd> = (0..6).map(|i| upd(i, 100 * (i as i64 + 1))).collect();
        // 0: applied; 1: refused; 2: no call; 3: ok but the log has it at another tick; 4: called twice; 5: ok but never in the DSP log.
        let rows = vec![row(100, true), row(200, false), row(400, true), row(500, true), row(500, true), row(600, true), row(999, true)];
        let dsp = vec![(100, FREQS[0], true), (450, FREQS[1], true), (500, FREQS[0], true), (7, 1.0, false), (777, 3.0, true)];
        let a = account(&table, &rows, &dsp);
        let why: Vec<_> = a.per.iter().map(|(_, w)| w.clone()).collect();
        assert_eq!(why, [None, Some(Why::Refused), Some(Why::NoApply), Some(Why::NotInDsp), Some(Why::Duplicate), Some(Why::NotInDsp)]);
        assert_eq!((a.failures(), a.unexpected_rows, a.extra_dsp), (5, 1, 2));
        assert_eq!(a.extra_dsp_entries, [(450, FREQS[1]), (777, 3.0)], "the stray entries are listed, in tick order, and name no update");
    }

    /// A DSP entry of the right frequency at another tick never labels a failed update (no frequency fallback): it is an extra entry.
    #[test]
    fn a_stray_dsp_entry_labels_no_update() {
        let table: Vec<Upd> = (0..4).map(|i| upd(i, 100 * (i as i64 + 1))).collect();
        // Updates 0 and 2 (frequency 0) fail: no DSP entry at their instants. A stray frequency-0 entry at 250 is nobody's.
        let rows: Vec<Row> = (1..=4).map(|i| row(100 * i, true)).collect();
        let dsp = vec![(200, FREQS[1], true), (400, FREQS[1], true), (250, FREQS[0], true)];
        let a = account(&table, &rows, &dsp);
        let why: Vec<_> = a.per.iter().map(|(_, w)| w.clone()).collect();
        assert_eq!(why, [Some(Why::NotInDsp), None, Some(Why::NotInDsp), None]);
        assert_eq!((a.extra_dsp, a.extra_dsp_entries), (1, vec![(250, FREQS[0])]));
    }

    #[test]
    fn account_matches_by_instant_and_frequency_not_by_order() {
        let table: Vec<Upd> = (0..3).map(|i| upd(i, 10 * (i as i64 + 1))).collect();
        let rows = vec![row(30, true), row(10, true), row(20, true)];
        let dsp = vec![(30, FREQS[0], true), (20, FREQS[1], true), (10, FREQS[0], true)];
        assert_eq!(account(&table, &rows, &dsp).failures(), 0);
        // The right tick with the other frequency is not the update's application.
        let wrong = vec![(30, FREQS[0], true), (20, FREQS[0], true), (10, FREQS[0], true)];
        assert_eq!(account(&table, &rows, &wrong).failures(), 1);
    }

    #[test]
    fn the_tables_have_the_fixed_shape() {
        let off = Offset { mcr: 200_000_000, ref_tick: 0, ref_ns: 0, unc_ns: 0 };
        let t = table(Kind::Bursty, 8, &off, 1_000_000_000);
        let gaps: Vec<i64> = t.windows(2).map(|w| w[1].e_ns - w[0].e_ns).collect();
        assert_eq!(gaps, [100_000, 100_000, 100_000, 9_700_000, 100_000, 100_000, 100_000]);
        let s = table(Kind::Spaced, 3, &off, 1_000_000_000);
        assert_eq!(s.iter().map(|u| u.e_ns).collect::<Vec<_>>(), [1_000_000_000, 1_005_000_000, 1_010_000_000]);
        assert_eq!(s.iter().map(|u| u.freq).collect::<Vec<_>>(), [FREQS[0], FREQS[1], FREQS[0]]);
    }

    #[test]
    fn the_decorator_records_timed_applies_and_receive_counts_and_forwards_the_rest() {
        let fake = Arc::new(FakeDevice::new(crate::device::FakeConfig { master_clock_rate: 1_000_000, ..Default::default() }));
        let timing = Timing::new(fake.clone(), 4, true);
        fake.set_time_zero(false).unwrap();
        let now = timing.time_now().unwrap();
        timing.apply(Dir::Rx, 0, &Settings { rate: Some(1e6), ..Settings::default() }, None).unwrap();
        timing.rx_open(0, &[0]).unwrap();
        timing.rx_start(0, now + 2_000).unwrap();
        timing.apply(Dir::Rx, 0, &Settings { freq: Some(1e9), ..Settings::default() }, Some(now + 50_000)).unwrap();
        let rows = timing.rows();
        assert_eq!((rows.len(), rows[0].e, rows[0].ok, rows[0].id), (1, now + 50_000, true, 0), "an untagged apply carries no identity");
        timing.tag(7);
        timing.apply(Dir::Rx, 0, &Settings { freq: Some(1.1e9), ..Settings::default() }, Some(now + 55_000)).unwrap();
        timing.apply(Dir::Rx, 0, &Settings { freq: Some(1.2e9), ..Settings::default() }, Some(now + 56_000)).unwrap();
        assert_eq!(timing.rows().iter().map(|r| r.id).collect::<Vec<_>>(), [0, 7, 0], "a tag names the next apply only");
        let rows = timing.rows()[..1].to_vec();
        assert!(rows[0].entry_ns <= rows[0].ret_ns);
        assert_eq!(timing.untimed.load(Relaxed), 1);
        timing.counts.tps.store(1, Relaxed);
        let mut blocks = 0;
        while blocks < 2 {
            if matches!(timing.rx_recv(0, 100, Duration::from_millis(50), false), RxRecv::Samples { .. }) {
                blocks += 1;
            }
        }
        assert_eq!((timing.counts.blocks.load(Relaxed), timing.counts.samples.load(Relaxed)), (2, 200));
        for _ in 0..8 {
            let _ = timing.apply(Dir::Rx, 0, &Settings::default(), Some(now + 60_000));
        }
        assert_eq!((timing.rows().len(), timing.dropped_rows.load(Relaxed)), (4, 7), "the rows are bounded and the overflow counted");
    }

    /// A rejected attempt is attributed to its reason in the value's own record: no record per attempt, an id beyond the table is counted and not stored.
    #[test]
    fn a_rejected_attempt_is_attributed_to_its_reason_per_value() {
        let fake = Arc::new(FakeDevice::new(crate::device::FakeConfig::default()));
        let timing = Timing::new(fake, 1, true);
        let bench = Bench {
            sink: Box::new(|_, _| {}),
            counters: true,
            publish_retries: AtomicU64::new(0),
            stale_rounds: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            take_dropped: AtomicU64::new(0),
            discarded_before_origin: AtomicU64::new(0),
            discarded_after_cut: AtomicU64::new(0),
            reserve: Mutex::new(vec![ReserveAgg::default(); 3]),
            reserve_dropped: AtomicU64::new(0),
            stamps: None,
            bound_rejections: AtomicU64::new(0),
            k_occupancy: AtomicUsize::new(0),
            d_occupancy: AtomicUsize::new(0),
            k_max: AtomicUsize::new(0),
            d_max: AtomicUsize::new(0),
            retired: AtomicU64::new(0),
            last_in_flight: Mutex::new(Vec::new()),
            clock: timing,
        };
        let bound = Some(NotReserved::DeviceBound);
        let phase = Some(NotReserved::Refused(Refusal::Phase));
        let chain = Some(NotReserved::ChainBound);
        for (id, reserved, reason, loads) in [
            (1, false, bound, (4, 3)),
            (1, false, bound, (4, 3)),
            (1, false, bound, (4, 3)),
            (1, false, phase, (2, 7)),
            (1, true, None, (3, 5)),
            (2, false, Some(NotReserved::Refused(Refusal::Cut)), (1, 1)),
            (3, false, chain, (2, 64)),
            (99, false, bound, (4, 4)),
        ] {
            bench.reserve_attempt(id, reserved, reason, loads.0, loads.1);
        }
        let table = bench.reserve.lock().unwrap().clone();
        assert_eq!((table[0].attempts, table[0].by_reason, table[0].reserved_ns != 0), (5, [3, 0, 1, 0, 0, 0, 0, 0], true), "bound three times, phase once, then reserved");
        assert_eq!((table[1].attempts, table[1].by_reason, table[1].reserved_ns), (1, [0, 0, 0, 0, 0, 0, 1, 0], 0), "a cut refusal, never reserved");
        assert_eq!(bench.reserve_dropped.load(Relaxed), 1, "id 99 is beyond the table");
        assert_eq!(bench.bound_rejections.load(Relaxed), 4, "only the three K rejections and the one D rejection count, not the phase or cut refusals (two, so a count of the others differs)");
        assert_eq!((table[0].first_k_ns != 0, table[0].first_d_ns), (true, 0), "value 1 was rejected at K, never at D");
        assert_eq!((table[1].first_k_ns, table[1].first_d_ns), (0, 0), "a refusal is neither");
        assert_eq!((table[2].attempts, table[2].first_k_ns, table[2].first_d_ns != 0, table[2].by_reason[1]), (1, 0, true, 1), "value 3 was rejected at D");
        assert_eq!((bench.k_max.load(Relaxed), bench.d_max.load(Relaxed)), (4, 64), "the largest K load and D occupancy after any attempt");
        assert_eq!(REASONS[reason_index(NotReserved::DeviceBound)], "k");
        assert_eq!(REASONS[reason_index(NotReserved::Refused(Refusal::Progress))], "progress");
    }

    /// The diagnostic's observation of the chain: the occupancy is what is in flight after the operation, and a value that left since the last
    /// operation is retired once, stamped, and counted; nothing is observed without the stamps.
    #[test]
    fn the_in_flight_observation_stamps_each_value_once_when_it_leaves() {
        let fake = Arc::new(FakeDevice::new(crate::device::FakeConfig::default()));
        let timing = Timing::new(fake, 1, true);
        let bench = |stamps: bool| Bench {
            sink: Box::new(|_, _| {}),
            counters: true,
            publish_retries: AtomicU64::new(0),
            stale_rounds: AtomicU64::new(0),
            publishes: AtomicU64::new(0),
            take_dropped: AtomicU64::new(0),
            discarded_before_origin: AtomicU64::new(0),
            discarded_after_cut: AtomicU64::new(0),
            reserve: Mutex::new(Vec::new()),
            reserve_dropped: AtomicU64::new(0),
            stamps: stamps.then(|| (0..5).map(|_| std::array::from_fn(|_| AtomicI64::new(0))).collect()),
            bound_rejections: AtomicU64::new(0),
            k_occupancy: AtomicUsize::new(0),
            d_occupancy: AtomicUsize::new(0),
            k_max: AtomicUsize::new(0),
            d_max: AtomicUsize::new(0),
            retired: AtomicU64::new(0),
            last_in_flight: Mutex::new(Vec::new()),
            clock: timing.clone(),
        };
        let b = bench(true);
        b.observe_in_flight(&[1, 2, 3], 2);
        assert_eq!((b.d_occupancy.load(Relaxed), b.k_occupancy.load(Relaxed), b.retired.load(Relaxed)), (3, 2, 0), "D counts the resident values, K the load passed");
        b.observe_in_flight(&[2, 3, 4], 4);
        b.observe_in_flight(&[2, 3, 4], 4);
        assert_eq!((b.d_occupancy.load(Relaxed), b.k_occupancy.load(Relaxed), b.retired.load(Relaxed)), (3, 4, 1), "value 1 left once");
        let stamped = |id: usize| b.stamps.as_ref().unwrap()[id - 1][Stage::Retired as usize].load(Relaxed) != 0;
        assert_eq!([1, 2, 3, 4].map(stamped), [true, false, false, false]);
        b.observe_in_flight(&[], 0);
        assert_eq!((b.d_occupancy.load(Relaxed), b.k_occupancy.load(Relaxed), b.retired.load(Relaxed)), (0, 0, 4));
        let off = bench(false);
        off.observe_in_flight(&[1], 1);
        assert_eq!((off.d_occupancy.load(Relaxed), off.k_occupancy.load(Relaxed), off.retired.load(Relaxed)), (0, 0, 0), "no observation without the stamps");
    }

    #[test]
    fn the_offset_maps_ticks_and_host_time_both_ways() {
        let fake = FakeDevice::new(crate::device::FakeConfig { master_clock_rate: 1_000_000, ..Default::default() });
        fake.set_time_zero(false).unwrap();
        let off = Offset::establish(&fake, 1_000_000);
        for tick in [-5, 0, 7, 123_456] {
            assert_eq!(off.tick_ceil(off.ns_of(tick)), tick);
            assert_eq!(off.tick_floor(off.ns_of(tick)), tick);
        }
        assert_eq!(off.tick_ceil(off.ns_of(10) + 1), 11);
        assert_eq!(off.ticks(3_000_000), 3_000);
        let t = fake.time_now().unwrap();
        let host = fake.host().now().as_nanos() as i64;
        assert!((off.tick_floor(host) - t).abs() < 2_000, "the map follows the fake's clock: {} vs {t}", off.tick_floor(host));
    }
}
