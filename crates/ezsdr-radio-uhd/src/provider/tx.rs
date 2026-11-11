//! uhd-tx: owns the transmit streamer. It transmits held bursts in target order,
//! keeps at most the in-flight window handed to the device ahead of its time, joins a
//! burst at the open one's next sample, ends its clock at its cut, and reads the
//! asynchronous reports (UR-21…UR-23, UR-25, UR-28).

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration as Wall;

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::event::Severity;
use ezsdr_kernel::id::ClockDomainId;
use ezsdr_kernel::stream::{BlockFlags, BlockHeader, BurstOpen, BurstStep, BurstTracker, ChannelMask, Direction, LateOutcome, LatePolicy};
use ezsdr_kernel::time::{Duration, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::{TimeErrorCause, TimeErrorOutcome, TimeErrorPayload, TxUnderflowCause, TxUnderflowPayload};
use ezsdr_radio::staging::{CLOSE_GAP, CLOSED_SHORT_MAX, Outcome, Read, STOP_FRONT, Settle, TxBook, TxSend};
use serde_json::json;

use super::control::ColdConfig;
use super::core::{Clock, Core, Locked, Streams, lock, stream_key};
use crate::device::{Dir, Iq, MIN_FIRST_SEND, MIN_OPEN_FIRST_SEND, TxCode, TxReport};
use crate::host::HostClock;
use crate::profile::DEVICE_LEAD_NS;
use ezsdr_radio::timeline::{Channels, StreamKey};

/// A burst uhd-control booked (UR-21).
pub(crate) struct Held {
    pub k: i64,
    pub domain: ClockDomainId,
    pub samples: Arc<Vec<Vec<Iq>>>,
    pub waveform_len: usize,
    pub repeat: bool,
    /// The burst is a repeat switch (RM-37), taken at receipt at its boundary (`k`): the owner continues the device burst into it there.
    pub switch: bool,
    /// The burst has no requested instant (RS-19, RM-15): a late one moves at dispatch, silently, and no late policy applies.
    pub untimed: bool,
    pub open: BurstOpen,
    pub policy: LatePolicy,
    pub target: TimePoint,
    /// The values the burst takes, booked with it (RM-21); the owner applies them at its turn.
    pub settings: Arc<ColdConfig>,
}

impl ezsdr_radio::staging::Booked for Held {
    fn id(&self) -> Option<ezsdr_kernel::event::ActionId> {
        self.settings.action()
    }

    fn is_switch(&self) -> bool {
        self.switch
    }
}

impl Held {
    /// The length the booking record holds for this burst: `None` for a repeat, which never ends.
    pub(super) fn record_len(&self) -> Option<i64> {
        (!self.repeat).then_some(self.waveform_len as i64)
    }

    pub(super) fn tile_repeat(&mut self) {
        let len = self.waveform_len;
        let cap = TX_SEND_CAP as usize;
        if !self.repeat || len == 0 || len >= cap {
            return;
        }
        let tiled_len = ((cap + 2 * len - 2) / len) * len;
        let mut channels = Vec::with_capacity(self.samples.len());
        for channel in self.samples.iter() {
            let mut tiled = Vec::with_capacity(tiled_len);
            while tiled.len() < tiled_len {
                tiled.extend_from_slice(&channel[..len]);
            }
            channels.push(tiled);
        }
        self.samples = Arc::new(channels);
    }
}

/// What uhd-tx takes from its channel: only the end of the thread. Bookings and Stops reach it through the stream's list, in receipt order (UR-21, UR-26).
pub(crate) enum TxCmd {
    /// `Provider::stop`, after the Run's end was read from the list (UR-26).
    Shutdown,
}

struct PendingReport {
    target: TimePoint,
    channels: BTreeSet<usize>,
}

#[derive(Clone, Copy)]
struct Tail {
    samples: [Iq; TX_CHANNELS_MAX],
    channels: usize,
}

impl Tail {
    fn buffers(&self) -> [&[Iq]; TX_CHANNELS_MAX] {
        std::array::from_fn(|channel| std::slice::from_ref(&self.samples[channel]))
    }
}

#[derive(Clone, Copy, Default)]
// Keep the fixed-capacity tail inline; boxing it would allocate on the TX path.
#[allow(clippy::large_enum_variant)]
enum DeviceBurst {
    #[default]
    Closed,
    Open {
        /// The root tick of the device burst's first sample, its start-of-burst send's instant.
        first: i64,
        next: i64,
        tail: Option<Tail>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Recovery {
    #[default]
    Clean,
    ReopenNeeded,
}

#[derive(Default)]
struct TxChain {
    burst: DeviceBurst,
    front: Option<(ClockDomainId, i64)>,
    recovery: Recovery,
}

impl TxChain {
    fn next(&self) -> Option<i64> {
        match self.burst {
            DeviceBurst::Closed => None,
            DeviceBurst::Open { next, .. } => Some(next),
        }
    }

    fn tail(&self) -> Option<Tail> {
        match self.burst {
            DeviceBurst::Closed => None,
            DeviceBurst::Open { tail, .. } => tail,
        }
    }

    fn continues_at(&self, sample: i64) -> bool {
        self.next() == Some(sample)
    }

    fn burst_is_open(&self) -> bool {
        matches!(self.burst, DeviceBurst::Open { .. })
    }

    /// Whether an underflow report reaches the open device burst: one is open, and `tick` is none or not before the burst's first sample
    /// (an earlier tick is the device burst before's, #93).
    fn underflowed(&self, tick: Option<i64>) -> bool {
        match self.burst {
            DeviceBurst::Closed => false,
            DeviceBurst::Open { first, .. } => tick.is_none_or(|tick| tick >= first),
        }
    }

    fn front(&self) -> Option<(ClockDomainId, i64)> {
        self.front
    }

    #[cfg(test)]
    fn recovery(&self) -> Recovery {
        self.recovery
    }

    fn needs_reopen(&self) -> bool {
        self.recovery == Recovery::ReopenNeeded
    }

    fn open_device_burst(&mut self, first: i64, next: i64, tail: Option<Tail>) {
        assert!(matches!(self.burst, DeviceBurst::Closed));
        self.burst = DeviceBurst::Open { first, next, tail };
    }

    fn continue_device_burst(&mut self, next: i64, tail: Option<Tail>) {
        let DeviceBurst::Open { first, .. } = self.burst else { panic!("a device burst is open") };
        self.burst = DeviceBurst::Open { first, next, tail };
    }

    fn close_device_burst(&mut self) {
        self.burst = DeviceBurst::Closed;
    }

    fn advance_front(&mut self, domain: ClockDomainId, sample: i64) {
        let sample = self.front.filter(|(old_domain, _)| *old_domain == domain).map_or(sample, |(_, old)| old.max(sample));
        self.front = Some((domain, sample));
    }

    fn mark_reopen_needed(&mut self) {
        self.recovery = Recovery::ReopenNeeded;
    }

    fn reopen_succeeded(&mut self) {
        self.recovery = Recovery::Clean;
    }

    fn reopen_failed(&mut self) {
        self.burst = DeviceBurst::Closed;
        self.recovery = Recovery::Clean;
    }

    fn provider_stop(&mut self) {
        self.recovery = Recovery::Clean;
    }

    fn lose(&mut self) {
        self.burst = DeviceBurst::Closed;
        self.recovery = Recovery::Clean;
    }
}

pub(crate) struct Tx {
    core: Arc<Core>,
    /// The stream's number, `k` (RM-29): every device call and line reference names it.
    k: usize,
    cmds: Receiver<TxCmd>,
    /// The device's host clock (plan 30 §4.3): the idle wait and the shutdown's ACK window.
    host: HostClock,
    clock: Option<Clock>,
    /// The channel set of the stream's clock, for the bursts' channel count (RM-29).
    /// The streamer's set is the stream's record (UR-13), not this.
    channels: Channels,
    /// The generation of the streamer `unacked`'s correlations were taken against (UR-36,
    /// UR-28): the record's generation as this owner last read it, so that a streamer opened
    /// since is told from this one's.
    streamer: u64,
    tracker: Option<BurstTracker>,
    /// The root tick of the current clock's cut, from the stream's one segment as the last pass
    /// read it (UR-25): the origin when a loss or `Provider::stop` ended the clock at or before it.
    cut: Option<i64>,
    unacked: VecDeque<PendingReport>,
    /// One radio TIME_ERROR per late packet can name one Kernel burst more than once.
    late_reported: HashSet<TimePoint>,
    /// Per-streamer device-burst, safe-start, and recovery facts (design note 44 §4).
    chain: TxChain,
    contract: Option<DataContractId>,
}

const IDLE: Wall = Wall::from_micros(200);
const SEND_TIMEOUT: Wall = Wall::from_secs(1);
const TX_SEND_CAP: i64 = 20_000;
const TX_CHANNELS_MAX: usize = 64;

/// Whether the owner configures the values of generation `wanted` before a burst starts (R4, M1): the device
/// holds another generation, no device burst is open, the device time is at or after the end of the owner's last
/// closed device burst (`previous_end`, padding and held tail included), and no recovery is pending.
fn apply_due(mark: Option<u64>, wanted: u64, device_open: bool, device_now: i64, previous_end: Option<i64>, recovering: bool) -> bool {
    mark != Some(wanted) && !device_open && !recovering && previous_end.is_none_or(|end| device_now >= end)
}

/// The owner's turn for a burst's values before it starts (R4).
enum Values {
    Start,
    Configure(u64),
    Wait,
}

impl Tx {
    pub fn new(core: Arc<Core>, k: usize, cmds: Receiver<TxCmd>, clock: Option<Clock>, channels: Channels) -> Tx {
        let host = core.device.host();
        // The streamer this owner starts on: the record's, opened by `arm` or by uhd-control
        // (UR-36).
        let streamer = lock(&core.streams).stream(stream_key(Dir::Tx, k)).generation;
        Tx {
            core,
            k,
            cmds,
            host,
            tracker: clock.map(|c| BurstTracker::new(c.domain)),
            clock,
            channels,
            streamer,
            cut: None,
            unacked: VecDeque::new(),
            late_reported: HashSet::new(),
            chain: TxChain::default(),
            contract: Some(DataContractId::parse("ezsdr.stream.cf32").expect("cf32")),
        }
    }

    pub fn run(mut self) {
        // UR-23: real-time once, on the owner's own thread; a refusal is recorded and survived, not a device loss.
        match self.core.device.raise_thread_priority() {
            Ok(()) => self.core.stat("tx_realtime", 1),
            Err(error) => self.core.reject_note(json!({ "call": "set_thread_priority", "stream": self.k, "error": error.message })),
        }
        loop {
            if let Ok(TxCmd::Shutdown) | Err(TryRecvError::Disconnected) = self.cmds.try_recv() {
                return self.shutdown();
            }
            self.read_list();
            if self.core.is_lost() {
                self.host.sleep(Wall::from_millis(1));
                continue;
            }
            if !self.pass() {
                self.host.sleep(IDLE);
            }
        }
    }

    /// One turn of the run loop (UR-23, UR-25): the stream's cut, read again on every turn, and
    /// the report read's ownership are one section of the instance's line state; then the report
    /// read, then one step. A turn another holder owns the streamer for makes no device call at
    /// all — `reports` and `step` included — and nothing of the owner's state moves. False when
    /// the turn had nothing to do, which the run loop's idle wait follows.
    pub(super) fn pass(&mut self) -> bool {
        // An uncertain EOB invalidates this report queue. Replace its streamer before
        // consuming anything that could have been queued by the failed generation.
        if self.chain.needs_reopen() {
            return self.reopen_streamer();
        }
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let mut streams = lock(&core.streams);
        if !streams.begin_read(key) {
            return false;
        }
        self.read_cut(&streams, key);
        drop(streams);
        self.reports(Wall::ZERO);
        lock(&core.streams).end_read(key);
        self.step()
    }

    /// The stream's book, under the streams lock: this owner writes its held and open bookings only through it.
    fn with_book<R>(&self, f: impl FnOnce(&mut TxBook<Held>) -> R) -> R {
        f(&mut lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).book)
    }

    /// The streams lock for a send's plan or result, with its host-clock wait kept as the longest (note 49 §12).
    fn lock_for_send(&self) -> Locked<'_, Streams> {
        let asked = self.host.now();
        let streams = lock(&self.core.streams);
        self.core.stat_max("tx_lock_wait_max_ns", i64::try_from(self.host.now().saturating_sub(asked).as_nanos()).unwrap_or(i64::MAX));
        streams
    }

    /// The first sample the device has nothing past: the open booking's next sample, or the open device burst's.
    fn cursor(&self) -> Option<i64> {
        self.with_book(|book| book.open.as_ref().map(|open| open.next)).or_else(|| self.chain.next())
    }

    /// The stream's held bookings (read, not yet taken), under the streams lock beside the record; this owner is their one writer.
    fn with_held<R>(&self, f: impl FnOnce(&mut BTreeMap<i64, Held>) -> R) -> R {
        f(&mut lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).book.list.held)
    }

    fn held_first(&self) -> Option<i64> {
        self.with_held(|held| held.keys().next().copied())
    }

    fn hold(&self, held: Held) {
        self.with_held(|map| map.insert(held.k, held));
    }

    fn take_held(&self, k: i64) -> Option<Held> {
        self.with_held(|held| held.remove(&k))
    }

    fn take_all_held(&self) -> BTreeMap<i64, Held> {
        self.with_held(std::mem::take)
    }

    /// Reads the stream's list in receipt order, one entry per critical section (UR-21, UR-26): a booking is held, a halt cancels what was read before it.
    pub(super) fn read_list(&mut self) {
        loop {
            // Bound first: a `while let` on the lock keeps the guard through the body, which locks again.
            let read = lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).book.read();
            match read {
                None => break,
                Some(Read::Booking(held)) => self.receive(held),
                Some(Read::Halt { reason, cancelled }) => self.stop_now(reason, cancelled),
            }
        }
    }

    fn receive(&mut self, mut held: Held) {
        held.tile_repeat();
        let Some(clock) = self.clock else {
            // The stream has no clock after a failed streamer reopen (UR-22, UR-25).
            return self.cancel_unstarted();
        };
        debug_assert_eq!(clock.domain, held.domain, "receipt refuses another SampleClock");
        self.hold(held);
    }

    /// UR-25: the cut of the stream's one segment, read in the caller's section of the instance's
    /// line state. A cut exists only at the origin (a loss or `Provider::stop` at or before it).
    pub(super) fn read_cut(&mut self, streams: &Streams, key: StreamKey) {
        self.cut = self.clock.and_then(|clock| streams.line(key).and_then(|line| line.plan.first()).and_then(|segment| segment.cut).map(|cut| clock.instant(cut)));
    }

    /// UR-25: a burst booked while the stream has no clock (a failed streamer reopen ended it) is cancelled.
    fn cancel_unstarted(&self) {
        self.report_cancelled_burst("UR-25: its transmit clock ended before the burst could start", ezsdr_radio::CancelReason::ClockEnded);
    }

    fn remember_burst(&mut self, target: TimePoint) {
        // UR-28: a pending report's channels are the positions in the streamer — the
        // numbers UHD's asynchronous reports carry (4.10's `rfnoc_tx_streamer.cpp:152-159,
        // 312-319`; `FakeDevice`, `device.rs:1515-1516,1555-1556`) —, not the device
        // channels the stream runs: a stream opened for `{1}` reports channel 0. The set is
        // never empty: a burst is booked only on a stream this Module opened, and a stream
        // with no channel is refused (UR-21).
        self.unacked.push_back(PendingReport { target, channels: (0..self.channels.count() as usize).collect() });
    }

    /// Each channel's reports are ordered independently in UHD's shared queue.
    fn reported_targets(&mut self, channel: usize, tick: Option<i64>) -> Vec<TimePoint> {
        let matching: Vec<usize> = if let Some(tick) = tick {
            self.unacked
                .iter()
                .enumerate()
                .filter_map(|(index, pending)| {
                    let root_tick = self.core.clocks.convert(pending.target, self.core.root).and_then(|converted| converted.floor().ticks_in(self.core.root)).ok()?;
                    // Strictly before the tick: at 200 MS/s a burst one sample after the previous end starts at the ACK's own tick (UR-28).
                    (pending.channels.contains(&channel) && root_tick < tick).then_some(index)
                })
                .collect()
        } else {
            self.unacked.iter().position(|pending| pending.channels.contains(&channel)).into_iter().collect()
        };
        let mut retired = Vec::with_capacity(matching.len());
        for index in matching.into_iter().rev() {
            let pending = &mut self.unacked[index];
            retired.push(pending.target);
            pending.channels.remove(&channel);
            if pending.channels.is_empty() {
                self.unacked.remove(index);
            }
        }
        for target in &retired {
            if !self.target_pending(*target) {
                self.late_reported.remove(target);
            }
        }
        retired
    }

    fn time_error_target(&self, channel: usize, tick: Option<i64>) -> Option<TimePoint> {
        let mut targets = self.unacked.iter().filter(|pending| pending.channels.contains(&channel)).filter_map(|pending| {
            let target = pending.target;
            let root_tick = self.core.clocks.convert(target, self.core.root).and_then(|converted| converted.floor().ticks_in(self.core.root)).ok()?;
            tick.is_none_or(|tick| root_tick <= tick).then_some(target)
        });
        let target = targets.next()?;
        if tick.is_some() && targets.next().is_some() { None } else { Some(target) }
    }

    fn target_pending(&self, target: TimePoint) -> bool {
        self.unacked.iter().any(|pending| pending.target == target)
    }

    /// One unit of work; false when there was nothing to do.
    pub(super) fn step(&mut self) -> bool {
        if self.core.is_lost() {
            self.chain.lose();
            return false;
        }
        let Some(clock) = self.clock else { return false };
        // `now` is read in the section that finds nothing unread: every hand-over of this turn is bounded by an instant read while no Stop
        // waited, so a Stop received later cuts at or after it (RM-16), and a switch received later has its boundary past it (RM-37).
        let now = {
            let mut streams = lock(&self.core.streams);
            let stream = streams.stream_mut(stream_key(Dir::Tx, self.k));
            (!stream.book.list.unread()).then(|| {
                let now = self.core.now();
                // RM-37: the record applies a pending switch once device time reaches its boundary, whether or not a booking comes.
                stream.book.advance(clock.at_or_after(now));
                now
            })
        };
        self.core.probe("tx_step_guarded", None);
        let Some(now) = now else {
            self.read_list();
            return true;
        };
        let window = self.core.ticks(self.core.description.timing.tx_in_flight_ns);
        let window_cut = clock.at_or_after(now.saturating_add(window));
        let open = self.with_book(|book| book.open.as_ref().map(|open| (open.next, open.first, open.booking.k, open.booking.policy)));
        if open.is_none() && self.cut.is_some_and(|cut| now >= cut) {
            self.end_clock();
            return true;
        }
        if let Some((next, first, k, policy)) = open {
            // UR-21: a first send not yet handed over, whose start is no longer a device lead away, is judged by its late policy again.
            let late_start = first && !self.chain.continues_at(next) && !matches!(self.start_outcome(clock, k, policy), Ok(LateOutcome::OnTime {}));
            if late_start {
                self.with_book(TxBook::untake);
                return true;
            }
            if clock.instant(next) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true, self.cursor(), true);
                return true;
            }
            if self.held_first() == Some(next) {
                self.join_open(next);
                return true;
            }
            // The record refuses a booking inside the open burst's span, so this cannot be reached from receipt; if it were, the burst is refused.
            let stale = self.held_first().filter(|h| *h < next);
            debug_assert!(stale.is_none(), "a held burst starts before the open burst's next sample (RM-15)");
            if let Some(h) = stale {
                let held = self.take_held(h).expect("the first key");
                self.refuse(&held, Duration::new(ClockDomainId::HOST_MONOTONIC, 0), "UR-21: the burst starts before the open burst's next sample; it is refused");
                lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).book.drop_piece(h, held.record_len());
                return true;
            }
            let held_cut = self.held_first();
            return self.send(clock, held_cut, window_cut);
        }
        if let Some(c) = self.chain.next() {
            if clock.instant(c) < now {
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the transmit timeline expired; burst not resumed" }));
                self.abandon(true, self.cursor(), true);
                return true;
            }
            match self.held_first() {
                Some(k) if k == c && self.device_holds(k) => {}
                // Another burst is next, or none is booked yet: the device burst waits for a booking that may still continue
                // it, and ends before it runs dry once none can (Review N, B3; UR-23, RM-16).
                Some(_) | None => return self.close_unjoined(clock, c),
            }
        }
        if let Some(k) = self.held_first() {
            if k < window_cut {
                let held = self.take_held(k).expect("the first key");
                // Continuations have no timed SOB; a new device burst must still
                // have its device lead after waiting in the owner's queue.
                let held = if self.chain.continues_at(k) {
                    held
                } else {
                    match self.values_turn(clock, &held) {
                        Values::Start => {}
                        Values::Configure(generation) => return self.configure_turn(clock, held, generation),
                        Values::Wait => {
                            self.hold(held);
                            return false;
                        }
                    }
                    let Some(held) = self.dispatch_start(clock, held) else { return true };
                    held
                };
                if held.k >= window_cut {
                    self.hold(held);
                    return true;
                }
                if self.held_first().is_some_and(|k| k < held.k) {
                    self.hold(held);
                    return true;
                }
                let next = held.k;
                self.with_book(|book| book.take(held, next));
                let held_cut = self.held_first();
                return self.send(clock, held_cut, window_cut);
            }
        }
        false
    }

    /// The owner's close of the open device burst at sample `e` (UR-23, RM-16, §3): it ends once the record lets no booking
    /// continue the chain there. `continuation_open` and `close` share one critical section under the streams lock, so a
    /// booking received after the close is refused at receipt. False while a booking can still join.
    fn close_unjoined(&mut self, clock: Clock, e: i64) -> bool {
        let mut streams = lock(&self.core.streams);
        let Some(record_front) = streams.stream_mut(stream_key(Dir::Tx, self.k)).book.close_unjoined(e, self.core.now(), |k| self.core.join_by(clock, k)) else {
            return false;
        };
        drop(streams);
        self.end_open(true);
        if record_front.is_some_and(|front| self.chain.front().is_some_and(|(domain, device)| domain == clock.domain && device > front)) {
            self.core.stat("tx_close_past_record_front", 1);
        }
        true
    }

    /// What the owner does before `held` starts, for its values (R4): nothing when the device holds its
    /// generation, a configure when the guard allows it, and otherwise the burst waits for the guard.
    fn values_turn(&self, clock: Clock, held: &Held) -> Values {
        let Some(wanted) = held.settings.generation() else { return Values::Start };
        let mark = lock(&self.core.streams).stream(stream_key(Dir::Tx, self.k)).configured;
        if mark == Some(wanted) {
            return Values::Start;
        }
        let previous_end = self.chain.front().filter(|(domain, _)| *domain == clock.domain).map(|(_, sample)| sample);
        let device_now = clock.at_or_after(self.core.now());
        if apply_due(mark, wanted, self.chain.burst_is_open(), device_now, previous_end, self.chain.needs_reopen()) { Values::Configure(wanted) } else { Values::Wait }
    }

    /// Whether the device holds the values of the burst held at `k`, so that it may continue the open device burst (R4). A
    /// burst with no generation has none to hold.
    fn device_holds(&self, k: i64) -> bool {
        let mark = lock(&self.core.streams).stream(stream_key(Dir::Tx, self.k)).configured;
        self.with_held(|map| map.get(&k).is_some_and(|held| held.settings.generation().is_none_or(|wanted| mark == Some(wanted))))
    }

    /// The configure turn (R4, M2): the burst's values go to the device on the owner's claim, outside any device
    /// burst, and the turn returns with the burst held. The next turn drains the commands that came meanwhile (a
    /// `Stop` cancels the burst) and decides the burst again. A refused configure refuses the burst (R5); the stream
    /// goes on, and the burst stays counted in its `Bookings` to its booked end.
    fn configure_turn(&mut self, clock: Clock, held: Held, generation: u64) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let Some(claim) = lock(&core.streams).claim_owner(key) else {
            self.hold(held);
            return false;
        };
        let rate = core.mcr as f64 / clock.n as f64;
        let result = held.settings.configure_tx(&core, self.channels, self.k, rate);
        core.probe("tx_configure_before_mark", None);
        {
            let mut streams = lock(&core.streams);
            match &result {
                Ok(()) if !core.is_lost() => streams.record_configured(key, Some(generation)),
                Ok(()) => {}
                Err(_) => streams.record_configured(key, None),
            }
            streams.give_back(claim);
        }
        match result {
            Ok(()) => self.hold(held),
            Err(error) => {
                lock(&core.streams).stream_mut(key).book.drop_piece(held.k, held.record_len());
                core.device_failed(held.settings.action(), "tx_burst", &error);
            }
        }
        true
    }

    /// Hands the next buffer of the open burst to the device (UR-22, UR-23). False, with nothing handed over, while the window
    /// cannot yet take all that the other limits allow, except for the burst's first timed buffer, which takes what fits. The send is
    /// planned and issued in one section under the streams lock, made with no lock held, and completed in another section (UR-25).
    fn send(&mut self, clock: Clock, held_cut: Option<i64>, window_cut: i64) -> bool {
        if self.chain.needs_reopen() {
            return false;
        }
        let min_open = MIN_OPEN_FIRST_SEND as i64;
        let chunk = (self.core.ticks(self.core.description.timing.tx_in_flight_ns) / clock.n / 4).clamp(min_open, TX_SEND_CAP);
        let mut streams = self.lock_for_send();
        let book = &mut streams.stream_mut(stream_key(Dir::Tx, self.k)).book;
        let open = book.open.as_ref().expect("a burst is open");
        let (start, first) = (open.booking.k, open.first);
        let device_sob = first && !self.chain.continues_at(start);
        let next = open.next;
        let len = open.booking.waveform_len as i64;
        let offset = if open.booking.repeat { (next - start).rem_euclid(len) } else { next - start };
        let tiled = open.booking.repeat && open.booking.samples[0].len() > len as usize;
        let mut wanted = if tiled { chunk } else { chunk.min(len - offset) };
        if let Some(cut) = held_cut {
            wanted = wanted.min(cut - next);
        }
        let avail = window_cut - next;
        if avail < if device_sob { wanted.min(min_open) } else { wanted } {
            return false;
        }
        let count = wanted.min(avail);
        debug_assert!(count > 0, "the send is inside the in-flight window");
        let ends_waveform = !open.booking.repeat && offset + count == len;
        let ends_at_held = held_cut.is_some_and(|h| next + count == h);
        // A first send of at most 32 samples is one closed device burst, whether it ends the waveform or reaches the next held burst (UR-22).
        let short_first = device_sob && count <= min_open;
        let eob = ends_waveform || ends_at_held;
        // The Kernel's burst boundaries (`first`, `eob`) and the device's differ where one
        // burst continues the device burst of the one before (§11 F3).
        let continuing = first && !device_sob;
        // A Kernel burst's end is the device burst's only once no burst can still be booked
        // at its next sample (it is a device lead away): otherwise its last sample is held
        // back, and `step` sends it with end-of-burst at that deadline, or without it ahead
        // of a burst that continues the device burst there (Review N, B3; Review O, O-B1).
        // A short first buffer is not held: its start goes with it.
        let open_ended = clock.instant(next + count) - self.core.now() > self.core.device_lead();
        let hold = eob && !ends_at_held && open_ended && !short_first;
        // A short first send is closed here even where a booking follows at its end: that booking meets the padded front (UR-22).
        let device_eob = eob && (!ends_at_held || short_first) && !hold;
        let sending = (if hold { count - 1 } else { count }) as usize;
        // A closing packet is padded with zeros (UR-22: a first send to four, a continuation to two).
        let pad = if device_sob && device_eob { MIN_FIRST_SEND.saturating_sub(sending) } else { usize::from(device_eob && sending == 1) };
        let device_count = sending + pad;
        let (samples, burst_open) = (open.booking.samples.clone(), open.booking.open);
        book.issue(TxSend { count, device_count: device_count as i64, ends: eob });
        drop(streams);
        self.core.probe("tx_send_planned", None);
        let mut slices = [&[][..]; TX_CHANNELS_MAX];
        for (slice, channel) in slices.iter_mut().zip(samples.iter()) {
            *slice = &channel[offset as usize..(offset + count) as usize];
        }
        let channels = samples.len();
        // The held tail is at `start - 1`; the continuation cursor already follows it.
        if continuing {
            if let Some(tail) = self.chain.tail() {
                // The burst before's held sample, ahead of this one, no end-of-burst between;
                // without it this one would play a sample early, so its failure abandons this
                // burst, the device burst ended (Review Q, NB-Q2).
                let buffers = tail.buffers();
                let at = Some(clock.instant(next.saturating_sub(1)));
                let tail_sent = match self.core.device.tx_send(self.k, &buffers[..tail.channels], at, false, false, SEND_TIMEOUT) {
                    Ok(1) => true,
                    Ok(_) => {
                        self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                        false
                    }
                    Err(error) => {
                        self.core.device_failed(None, "tx_burst", &error);
                        false
                    }
                };
                if !tail_sent {
                    let Settle::Abandon { cursor } = self.complete_send(Outcome::TailShort) else { unreachable!("a failed tail abandons the burst") };
                    return self.abandon(true, Some(cursor), true);
                }
            }
        }
        let mut padded = [[Iq::default(); MIN_FIRST_SEND]; TX_CHANNELS_MAX];
        let mut sent = [&[][..]; TX_CHANNELS_MAX];
        if pad > 0 {
            for channel in 0..channels {
                padded[channel][..sending].copy_from_slice(&slices[channel][..sending]);
            }
            for channel in 0..channels {
                sent[channel] = &padded[channel][..device_count];
            }
        } else {
            for (slice, channel) in sent[..channels].iter_mut().zip(slices[..channels].iter()) {
                *slice = &channel[..sending];
            }
        }
        // T1: every payload-bearing packet names its first sample, even inside a device burst.
        let at = Some(clock.instant(next));
        let result = if device_count > 0 { self.core.device.tx_send(self.k, &sent[..channels], at, device_sob, device_eob, SEND_TIMEOUT) } else { Ok(0) };
        let settle = self.complete_send(result.as_ref().map_or(Outcome::Failed, |accepted| Outcome::Accepted(*accepted as i64)));
        // Held only once the samples before it went out: after a send that failed or came
        // up short it would play straight after what the device took (Review P, NB-1).
        let tail = hold.then(|| {
            let mut samples = [[0.0; 2]; TX_CHANNELS_MAX];
            for (sample, channel) in samples[..channels].iter_mut().zip(slices[..channels].iter()) {
                *sample = channel[sending];
            }
            Tail { samples, channels }
        });
        // The device burst is open when it was before this send, or this one's start went.
        let was_open = !first || continuing;
        if let Settle::Abandon { cursor } = settle {
            match result {
                Ok(accepted) => {
                    self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: a send did not complete within 1 s" }));
                    if device_sob && accepted > 0 {
                        self.remember_burst(TimePoint::new(clock.domain, start));
                    }
                    let result = self.abandon(was_open || accepted > 0, Some(cursor), !device_eob);
                    if device_eob {
                        self.core.reject_note(json!({ "call": "tx_close", "stream": self.k,
                            "error": format!("EOB send accepted {accepted} of {device_count} samples") }));
                        if self.core.is_lost() {
                            self.chain.lose();
                        } else {
                            self.chain.mark_reopen_needed();
                        }
                    }
                    return result;
                }
                Err(error) => {
                    // Whether UHD sent packets before the error is unknown: a start that may have
                    // gone is counted for its report, then closed by the two-sample EOB below.
                    self.core.device_failed(None, "tx_burst", &error);
                    if device_sob {
                        self.remember_burst(TimePoint::new(clock.domain, start));
                    }
                    let result = self.abandon(true, Some(cursor), !device_eob);
                    if device_eob {
                        self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": error.message }));
                        if self.core.is_lost() {
                            self.chain.lose();
                        } else {
                            self.chain.mark_reopen_needed();
                        }
                    }
                    return result;
                }
            }
        }
        let mut flags = BlockFlags::NONE;
        if first {
            flags = flags | BlockFlags::START_OF_BURST;
        }
        if eob {
            flags = flags | BlockFlags::END_OF_BURST;
        }
        let header = BlockHeader {
            first_sample_time: TimePoint::new(clock.domain, next),
            len: count as u32,
            channels: self.channels.count().max(1) as u16,
            direction: Direction::Tx,
            valid: ChannelMask::full(self.channels.count().max(1) as u16),
            flags,
            lost: None,
            contract: self.contract.take().expect("contract is available"),
        };
        let open_record = first.then_some(burst_open);
        if device_sob {
            self.remember_burst(TimePoint::new(clock.domain, start));
        }
        if first {
            self.core.stat("tx_bursts", 1);
        }
        self.core.stat("tx_samples", count);
        let next = next + count;
        if device_eob {
            self.chain.close_device_burst();
        } else if device_sob {
            self.chain.open_device_burst(clock.instant(start), next, tail);
        } else {
            self.chain.continue_device_burst(next, tail);
        }
        if let Some(tracker) = self.tracker.as_mut() {
            let step = tracker.on_block(&header, open_record);
            self.record(step);
        }
        self.contract = Some(header.contract);
        if device_eob {
            // UR-22: a closing send that is also the device burst's first send, of at most 32 padded samples, fronts at its padded end plus the
            // gap of the stream's rate; any other closing send at its end plus three. (A Kernel end the device burst outlives leaves it open
            // at the next sample.)
            let gap = if device_sob && device_count as i64 <= CLOSED_SHORT_MAX { self.short_gap(clock) } else { CLOSE_GAP };
            self.chain.advance_front(clock.domain, next.saturating_add(pad as i64 + gap));
        }
        true
    }

    /// A send's device outcome, in its own section under the streams lock: the book settles it (UR-22).
    fn complete_send(&self, outcome: Outcome) -> Settle {
        self.lock_for_send().stream_mut(stream_key(Dir::Tx, self.k)).book.complete(outcome)
    }

    fn record(&self, step: Result<BurstStep, ezsdr_kernel::stream::StreamError>) {
        let mut rec = lock(&self.core.rec);
        match step {
            Ok(BurstStep::Ended { record }) => rec.bursts.push(serde_json::to_value(record).expect("a record")),
            Ok(BurstStep::Discontinuity { closed, then_ended, .. }) => {
                rec.bursts.push(serde_json::to_value(closed).expect("a record"));
                if let Some(ended) = then_ended {
                    rec.bursts.push(serde_json::to_value(ended).expect("a record"));
                }
            }
            Ok(_) => {}
            Err(error) => rec.rejected.push(json!({ "burst_tracker": error.to_string() })),
        }
    }

    /// Ends the open burst without recalling what was handed over (RM-16), and with
    /// `device_eob` the device burst too, also one a held burst was to continue (§11 F3).
    fn end_open(&mut self, device_eob: bool) {
        let continuing = self.chain.next();
        let tail = self.chain.tail();
        let open = self.with_book(|book| book.open.as_ref().map(|open| (open.first, open.booking.k, open.next)));
        let device_open = open.map_or(continuing.is_some(), |(first, k, _)| !first || continuing == Some(k));
        if device_eob {
            self.chain.close_device_burst();
        }
        if device_eob && device_open && !self.core.is_lost() {
            let cursor = open.map_or(continuing, |(_, _, next)| Some(next));
            if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                let close_start = cursor.saturating_sub(if tail.is_some() { 1 } else { 0 });
                if self.close_device_burst(tail, cursor, clock) {
                    self.chain.advance_front(clock.domain, close_start.saturating_add(2 + CLOSE_GAP));
                }
            }
        } else if device_eob && self.core.is_lost() {
            self.chain.lose();
        }
        let Some(_) = self.with_book(TxBook::end_open) else { return };
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
    }

    /// Closes with exactly two samples and their first SampleClock instant: held tail + zero,
    /// or two zeros when there is no tail.
    fn close_device_burst(&mut self, tail: Option<Tail>, cursor: i64, clock: Clock) -> bool {
        if self.core.is_lost() {
            self.chain.lose();
            return false;
        }
        let has_tail = tail.is_some();
        let count = tail.as_ref().map_or(self.channels.count().max(1) as usize, |tail| tail.channels);
        let mut close = [[Iq::default(); 2]; TX_CHANNELS_MAX];
        if let Some(tail) = tail {
            for (samples, held) in close[..count].iter_mut().zip(tail.samples[..count].iter()) {
                samples[0] = *held;
            }
        }
        let buffers: Vec<&[Iq]> = close[..count].iter().map(|samples| &samples[..]).collect();
        let first = cursor.saturating_sub(if has_tail { 1 } else { 0 });
        let at = Some(clock.instant(first));
        match self.core.device.tx_send(self.k, &buffers, at, false, true, SEND_TIMEOUT) {
            Ok(2) => true,
            Ok(n) => {
                self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": format!("EOB close accepted {n} of 2 samples") }));
                if self.core.is_lost() {
                    self.chain.lose();
                } else {
                    self.chain.mark_reopen_needed();
                }
                false
            }
            Err(error) => {
                self.core.device_failed(None, "tx_close", &error);
                self.core.reject_note(json!({ "call": "tx_close", "stream": self.k, "error": error.message }));
                if error.lost {
                    self.chain.lose();
                } else {
                    self.chain.mark_reopen_needed();
                }
                false
            }
        }
    }

    fn reopen_streamer(&mut self) -> bool {
        let core = self.core.clone();
        let key = stream_key(Dir::Tx, self.k);
        let (claim, configured) = {
            let mut streams = lock(&core.streams);
            if streams.stop.is_some() || core.is_lost() {
                if core.is_lost() {
                    self.chain.lose();
                } else {
                    self.chain.provider_stop();
                }
                return false;
            }
            let Some(claim) = streams.claim_owner(key) else { return false };
            (claim, streams.stream(key).configured)
        };
        core.probe("tx_reopen_claimed", None);
        let result = core.reopen_tx_streamer(self.k, self.channels);
        core.probe("tx_reopen_done", None);
        if let (Ok(flush), Some(clock), Some(tick)) = (&result, self.clock, result.as_ref().ok().and_then(|flush| flush.ack_tick)) {
            if flush.acknowledged {
                // The flush is a closed short burst (UR-22, UR-33): its exclusive end is the ack's tick less one master tick.
                self.chain.advance_front(clock.domain, clock.at_or_after(tick.saturating_sub(1)).saturating_add(self.short_gap(clock)));
            }
        }
        let mut halted = false;
        let mut open_cancelled = false;
        {
            let mut streams = lock(&core.streams);
            if result.is_ok() {
                streams.record_configured(key, if core.is_lost() { None } else { configured });
            } else if !core.is_lost() && streams.stop.is_none() {
                open_cancelled = self.halt_failed_reopen(&mut streams, key);
                self.chain.reopen_failed();
                halted = true;
            } else if core.is_lost() {
                self.chain.lose();
            } else {
                self.chain.provider_stop();
            }
            streams.give_back(claim);
        }
        self.sync_streamer();
        if result.is_ok() {
            self.chain.reopen_succeeded();
        }
        if open_cancelled {
            self.report_cancelled_burst("UR-22: cancelled because the TX streamer could not be reopened", ezsdr_radio::CancelReason::ClockEnded);
        }
        if halted {
            for _ in self.take_all_held().into_values() {
                self.report_cancelled_burst("UR-22: cancelled because the TX streamer could not be reopened", ezsdr_radio::CancelReason::ClockEnded);
            }
        }
        if let Err(error) = &result {
            if !core.is_lost() && lock(&core.streams).stop.is_none() {
                self.core.device_failed(None, "tx_reopen", error);
            }
        }
        true
    }

    /// RM-25: a non-terminal reopen refusal ends the stream's clock: the open burst is cancelled and
    /// the stream has no clock after it. True when a burst was open, which the caller reports once the lock is released (UR-25).
    fn halt_failed_reopen(&mut self, streams: &mut Streams, key: StreamKey) -> bool {
        let cancelled = streams.stream_mut(key).book.end_open().is_some();
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        streams.stream_mut(key).book.clock_ended();
        streams.finished(key);
        self.clock = None;
        self.cut = None;
        self.tracker = None;
        cancelled
    }

    /// Always true: abandoning a burst at `cursor`, the first sample the device has nothing past, is work, which `send`'s callers return.
    fn abandon(&mut self, device_open: bool, cursor: Option<i64>, close_open_burst: bool) -> bool {
        // A send that failed or came up short: end the device burst if one is open, so
        // that none is left without end-of-burst (Review N, N7), with an empty buffer —
        // the burst's samples after what the device took are not sent (Review P, NB-1).
        // A switch at or after the handed cursor never starts, and its payload goes with it, held or unread (S4, A1). Only an abandoned
        // repeat takes its switch (KL-39).
        if let (Some(cursor), Some(clock)) = (cursor, self.clock) {
            // An abandoned repeat ends in the record at the handed cursor, with no in-flight window (the device has nothing past it) (S4,
            // #80). A switch joined at the cursor but not yet opened continues the repeat it replaces.
            let (repeat, id) = self
                .with_book(|book| book.open.as_ref().map(|open| (open.booking.repeat, open.booking.settings.action())))
                .unwrap_or_else(|| (self.with_held(|map| map.get(&cursor).is_some_and(|held| held.switch)), None));
            let cancelled = self.with_book(|book| book.abandon(repeat, id, cursor, clock.instant(cursor.saturating_add(STOP_FRONT))));
            for _ in 0..cancelled {
                self.report_cancelled_burst("UR-22: the repeat switch was cancelled with the repeat it replaces", ezsdr_radio::CancelReason::Stop);
            }
        }
        self.chain.close_device_burst();
        if device_open && close_open_burst && !self.core.is_lost() {
            if let (Some(clock), Some(cursor)) = (self.clock, cursor) {
                if self.close_device_burst(None, cursor, clock) {
                    // The two-zero close's padded end plus the gap of the stream's rate (UR-22, D1): the burst may be a closed short one on the device.
                    self.chain.advance_front(clock.domain, cursor.saturating_add(2).saturating_add(self.short_gap(clock)));
                }
            }
        } else if self.core.is_lost() {
            self.chain.lose();
        }
        if let Some(record) = self.tracker.as_mut().and_then(BurstTracker::stop) {
            lock(&self.core.rec).bursts.push(serde_json::to_value(record).expect("a record"));
        }
        self.with_book(TxBook::end_open);
        true
    }

    /// RM-15, UR-22: the samples past a closed short device burst's end that the next burst keeps, [`super::core::stream_gap`] of this stream's rate.
    fn short_gap(&self, clock: Clock) -> i64 {
        super::core::stream_gap(self.core.mcr, clock)
    }

    fn time_error(&self, outcome: TimeErrorOutcome, late_by: Duration, target: TimePoint) {
        // SC-27's late_by is a host.monotonic duration, so its ticks are nanoseconds.
        let late_by_ns = late_by.ticks_in(ClockDomainId::HOST_MONOTONIC).unwrap_or(i64::MAX);
        let payload = serde_json::to_value(TimeErrorPayload { cause: TimeErrorCause::Late, outcome, late_by_ns, target, action: None }).expect("a payload");
        self.core.emit(&self.core.stream_node(Dir::Tx, self.k), kinds::TIME_ERROR, Severity::Error, payload);
    }

    /// A burst the record cannot place (UR-21, UR-23): `TIME_ERROR { refused }`, then `COMMAND_REJECTED` naming its Action.
    fn refuse(&self, held: &Held, late_by: Duration, reason: &str) {
        self.time_error(TimeErrorOutcome::Refused, late_by, held.target);
        self.core.command_rejected(held.settings.action(), "tx_burst", reason);
    }

    fn report_send_asap(&self, held: &Held, late_by: Duration) {
        if held.open.late.is_none() {
            self.time_error(TimeErrorOutcome::SendAsap, late_by, held.target);
        }
    }

    /// UR-21: booking's verdict can expire before the first device hand-over.
    /// UR-21: the late policy's verdict on a device burst's start at `k`, judged at the decision instant and its device lead.
    fn start_outcome(&self, clock: Clock, k: i64, policy: LatePolicy) -> Result<LateOutcome, ezsdr_kernel::time::TimeError> {
        let lead = self.core.device_lead();
        let now = clock.at_or_after(ezsdr_radio::timeline::decided_at(self.core.now(), clock.origin, lead));
        policy.decide(&self.core.clocks, TimePoint::new(clock.domain, k), TimePoint::new(clock.domain, now), Duration::new(ClockDomainId::HOST_MONOTONIC, DEVICE_LEAD_NS))
    }

    fn dispatch_start(&mut self, clock: Clock, mut held: Held) -> Option<Held> {
        let lead = self.core.device_lead();
        let now = clock.at_or_after(ezsdr_radio::timeline::decided_at(self.core.now(), clock.origin, lead));
        let current = clock.at_or_after(clock.instant(now) + lead);
        let front = self.chain.front().filter(|(domain, _)| *domain == clock.domain).map(|(_, sample)| sample);
        let bound = ezsdr_radio::timeline::later_bound(current, front);
        // RM-15: an untimed head is late or on time by SendAsap's judgement only; the late policy is not consulted.
        let policy = if held.untimed { LatePolicy::SendAsapAndFlag } else { held.policy };
        let outcome = self.start_outcome(clock, held.k, policy);
        let outcome =
            outcome.and_then(|outcome| ezsdr_radio::timeline::late_policy_front_outcome(policy, held.k, current, front, |ticks| self.core.late_by(clock, ticks)).map(|front| front.unwrap_or(outcome)));
        match outcome {
            Ok(LateOutcome::OnTime {}) => Some(held),
            Ok(LateOutcome::SendAsap { .. }) if held.untimed => {
                let moved = {
                    let mut streams = lock(&self.core.streams);
                    streams.stream_mut(stream_key(Dir::Tx, self.k)).book.move_untimed(held.settings.action(), held.k, bound, held.record_len())
                };
                match moved {
                    Some(start) => {
                        held.k = start;
                        Some(held)
                    }
                    None => {
                        self.core.command_rejected(held.settings.action(), "tx_burst", "UR-21: the burst overlaps an accepted booking");
                        None
                    }
                }
            }
            Ok(late @ LateOutcome::SendAsap { late_by }) => {
                let start = bound;
                let moved = {
                    let mut streams = lock(&self.core.streams);
                    streams.stream_mut(stream_key(Dir::Tx, self.k)).book.move_start(held.settings.action(), held.k, start, held.record_len())
                };
                if moved.is_err() {
                    self.refuse(&held, late_by, "UR-21: the moved start overlaps an accepted booking");
                    return None;
                }
                held.k = start;
                self.report_send_asap(&held, late_by);
                held.open.late = Some(late);
                held.open.requested_target = held.open.requested_target.or(Some(held.target));
                Some(held)
            }
            Ok(LateOutcome::Drop { late_by }) => {
                // The region goes as a refused head's does (RM-15). No switch replaces a repeat dropped here: a switch is accepted only on a
                // repeat that has started on the device (UR-38), which is never judged at dispatch again.
                lock(&self.core.streams).stream_mut(stream_key(Dir::Tx, self.k)).book.drop_piece(held.k, held.record_len());
                self.time_error(TimeErrorOutcome::Drop, late_by, held.target);
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the late policy dropped the burst" }));
                None
            }
            Ok(LateOutcome::PlanViolation { late_by }) => {
                self.time_error(TimeErrorOutcome::PlanViolation, late_by, held.target);
                self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-21: the planned burst arrived late" }));
                None
            }
            Err(error) => {
                self.core.command_rejected(None, "tx_burst", &format!("UR-21: {error}"));
                None
            }
        }
    }

    /// A held burst at the open burst's next sample joins it (M3: a switch at its boundary and the joined
    /// continuation). Not judged for lateness here: the record refuses a booking inside the open burst's span, and a switch is
    /// never late (RM-37), so a late joined piece is refused at dispatch instead (`dispatch_start`, D24).
    fn join_open(&mut self, next: i64) {
        // UR-23: a burst at the open one's next sample continues the device burst without end-of-burst, as the X300 needs (§11 F3).
        // Otherwise nothing was sent and the new one starts on its own (Review L, P1-4).
        if self.chain.burst_is_open() || self.with_book(|book| book.open.as_ref().is_some_and(|open| !open.first)) {
            self.end_open(false);
            self.chain.continue_device_burst(next, self.chain.tail());
        } else {
            self.end_open(true);
        }
    }

    fn report_cancelled_burst(&self, reason: &str, cancel_reason: ezsdr_radio::CancelReason) {
        if ezsdr_radio::cancellation_reports(cancel_reason) {
            self.core.command_rejected(None, "tx_burst", reason);
        } else {
            self.core.reject_note(json!({ "kind": "tx_burst", "reason": reason }));
        }
    }

    /// A halt read from the list (UR-23, UR-26): ends the open burst and cancels `cancelled`, the bookings read before it.
    fn stop_now(&mut self, cancel_reason: ezsdr_radio::CancelReason, cancelled: BTreeMap<i64, Held>) {
        self.end_open(true);
        let reason = match cancel_reason {
            ezsdr_radio::CancelReason::Stop => "UR-26: cancelled by Stop",
            _ => "UR-26: cancelled by Provider::stop",
        };
        for _ in cancelled.into_values() {
            self.report_cancelled_burst(reason, cancel_reason);
        }
    }

    /// The clock ends (UR-25): the open burst ends and the stream has no clock after it. No booking is held: the Run end's halt was read
    /// before the cut check, and a booking after the cut is refused at receipt.
    fn end_clock(&mut self) {
        self.end_open(true);
        self.read_reports_before_close();
        self.clock = None;
        self.cut = None;
        lock(&self.core.streams).finished(stream_key(Dir::Tx, self.k));
        self.tracker = None;
    }

    /// UR-28, UR-36: a report queue belongs to the streamer of its generation. The record's
    /// generation moved whenever the owner reopened it, and with it the correlations of the bursts
    /// in flight against the replaced queue, which are dropped here before any report is
    /// correlated with them.
    fn sync_streamer(&mut self) {
        let streamer = lock(&self.core.streams).stream(stream_key(Dir::Tx, self.k)).generation;
        if streamer != self.streamer {
            self.unacked.clear();
            self.late_reported.clear();
        }
        self.streamer = streamer;
    }

    /// Reads any report made available by the old burst's end before its owner releases the
    /// streamer (UR-28, A10).
    fn read_reports_before_close(&mut self) {
        if self.core.is_lost() {
            return;
        }
        let stream = stream_key(Dir::Tx, self.k);
        // UR-25: a holder other than Free keeps the stream; the close then makes its reports
        // unread, rather than reading them behind the holder.
        if !lock(&self.core.streams).begin_read(stream) {
            return;
        }
        self.reports(Wall::ZERO);
        lock(&self.core.streams).end_read(stream);
    }

    /// UR-28: every report recorded; underflows and late bursts become events.
    fn reports(&mut self, timeout: Wall) {
        while !self.core.is_lost() {
            let report = {
                let _reports = self.core.lock_tx_reports(self.k);
                if let Some(report) = self.core.take_stashed_tx_report(self.k) { Ok(Some(report)) } else { self.core.device.tx_async(self.k, timeout) }
            };
            match report {
                Ok(Some(report)) => {
                    self.sync_streamer();
                    self.report(report)
                }
                Ok(None) => break,
                // UR-28: a failed read is the device's loss or a recorded error, never "no report".
                Err(error) if error.lost => {
                    self.core.device_lost(&error.message);
                    self.chain.lose();
                    return;
                }
                Err(error) => {
                    // Counted, with the first one's text: a failing read repeats at the poll rate.
                    let first = lock(&self.core.rec).stats.get("tx_errors").is_none_or(|n| *n == 0);
                    self.core.stat("tx_errors", 1);
                    if first {
                        self.core.reject_note(json!({ "call": "tx_async", "error": error.message }));
                    }
                    // UR-33: a read that failed at once costs the timeout it would have
                    // waited. A caller measuring clock time across a loop (shutdown's ACK
                    // window) then cannot spin without the clock ever advancing, which on
                    // the manual clock would stop the very time it measures and never end.
                    self.host.sleep(timeout);
                    break;
                }
            }
        }
    }

    fn report(&mut self, report: TxReport) {
        let raw = json!({
            "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel, "stream": self.k,
        });
        lock(&self.core.rec).device_async.push(raw);
        let time = match (self.clock, report.tick) {
            (Some(clock), Some(tick)) => TimePoint::new(clock.domain, (tick - clock.origin).div_euclid(clock.n)),
            _ => self.core.at(self.core.now()),
        };
        let underflow = |cause| {
            let payload = serde_json::to_value(TxUnderflowPayload { cause }).expect("a payload");
            self.core.emit_at(&self.core.stream_node(Dir::Tx, self.k), kinds::TX_UNDERFLOW, Severity::Warning, payload, time);
        };
        match report.code {
            TxCode::BurstAck => {
                self.reported_targets(report.channel, report.tick);
            }
            TxCode::Underflow | TxCode::UnderflowInPacket | TxCode::SeqError | TxCode::SeqErrorInBurst => {
                let cause = if matches!(report.code, TxCode::Underflow | TxCode::UnderflowInPacket) { TxUnderflowCause::Starved } else { TxUnderflowCause::Lost };
                underflow(cause);
                // Its nominal next sample no longer bounds actual playback. Do not
                // resume untimed payload on the shifted device timeline (UR-22).
                // A5 (F4): with no device burst open nothing was handed that could underflow, so the open booking waits for its own send.
                // An underflow's tick before the open device burst's first sample is the burst before's (#93, note 39 §6.6): reported, no
                // abandon. A sequence error's tick is not compared: its meaning is unmeasured, and it may be ahead of its packet's play time.
                let tick = if cause == TxUnderflowCause::Starved { report.tick } else { None };
                if self.chain.underflowed(tick) {
                    self.core.reject_note(json!({ "kind": "tx_burst", "reason": "UR-22: the device burst underflowed; burst not resumed" }));
                    self.abandon(true, self.cursor(), true);
                }
            }
            TxCode::TimeError => {
                // The report tick is radio-current-time, not the rejected packet's timestamp.
                // Only a unique pending burst whose first sample is no later than that tick fits.
                let Some(target) = self.time_error_target(report.channel, report.tick) else { return };
                let late_by_ns = match report.tick {
                    Some(tick) => {
                        let root = self.core.root;
                        let start = match self.core.clocks.convert(target, root).and_then(|c| c.floor().ticks_in(root)) {
                            Ok(start) => start,
                            Err(error) => return self.core.command_rejected(None, "tx_report", &format!("UR-28: {error}")),
                        };
                        match i64::try_from((i128::from(tick) - i128::from(start)) * 1_000_000_000 / i128::from(self.core.mcr)) {
                            Ok(ns) => ns,
                            Err(_) => return self.core.command_rejected(None, "tx_report", "UR-28: lateness overflow"),
                        }
                    }
                    None => 0,
                };
                if self.late_reported.insert(target) {
                    self.time_error(TimeErrorOutcome::LateAtDevice, Duration::new(ClockDomainId::HOST_MONOTONIC, late_by_ns), target);
                }
            }
            TxCode::Other(_) => {}
        }
    }

    fn shutdown(&mut self) {
        // A queued halt keeps its own reason; the rest goes with the Run's end.
        self.read_list();
        let cancelled = self.with_book(TxBook::cancel_all);
        self.stop_now(ezsdr_radio::CancelReason::ProviderStop, cancelled);
        self.chain.provider_stop();
        // 100 ms of the device's host clock for the last BURST_ACK (UR-26; plan 30 §4.3: it
        // waits for the device's acknowledgement, so it is measured in clock time). A loss —
        // found by a read here or recorded by another thread — ends the window: `reports` no
        // longer reads, so it no longer waits on the clock either, and the manual clock this
        // busy thread holds still would never reach `until` (UR-33).
        let until = self.host.now() + Wall::from_millis(100);
        while !self.core.is_lost() && self.host.now() < until {
            // UR-25: the shutdown's read is a report read all the same, but it takes no holder
            // check: a uhd-control the join bound left detached (UR-16) may never give its
            // claim back, and must not hold this window open.
            let stream = stream_key(Dir::Tx, self.k);
            lock(&self.core.streams).begin_shutdown_read(stream);
            self.reports(Wall::from_millis(10));
            lock(&self.core.streams).end_read(stream);
        }
    }
}

/// The O1 spec oracle (design note 49 §5.4.11), one source shared with the acceptance and Mock tests: it names no Provider type.
#[cfg(test)]
#[path = "../../../ezsdr-acceptance/tests/oracle/mod.rs"]
mod oracle;

#[cfg(test)]
mod tests {
    //! uhd-tx driven directly, where the order of its commands and steps matters and the
    //! Provider's threads cannot arrange it (the exit review's UR-21 clause).

    /// The open booking as a test reads and sets it, through the stream's book.
    impl Tx {
        pub(super) fn open_next(&self) -> Option<i64> {
            self.with_book(|book| book.open.as_ref().map(|open| open.next))
        }

        pub(super) fn open_first(&self) -> Option<bool> {
            self.with_book(|book| book.open.as_ref().map(|open| open.first))
        }

        pub(super) fn open_booking<R>(&self, f: impl FnOnce(&Held) -> R) -> Option<R> {
            self.with_book(|book| book.open.as_ref().map(|open| f(&open.booking)))
        }

        pub(super) fn poke_open(&self, held: Held, next: i64, first: bool) {
            self.with_book(|book| book.open = Some(ezsdr_radio::staging::Open::new(held, next, first)));
        }
    }

    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Duration as Wall;

    /// The gap of a closed short burst at 1 MS/s, `max(3, ceil(15 µs x 1 MS/s))`, written out from the arithmetic (UR-22).
    const GAP_1MSPS: i64 = 15;
    /// The same at 200 MS/s (`txp_high_rate_rig`): 15 µs is 3 000 samples.
    const GAP_200MSPS: i64 = 3_000;

    use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventKind};
    use ezsdr_kernel::hash::ContentHash;
    use ezsdr_kernel::module_api::{ActionReceiver, Dispatched, StopMode};

    use ezsdr_kernel::stream::{BurstOpen, LatePolicy};
    use ezsdr_kernel::time::{AbsoluteDeadline, TimePoint};

    use super::super::control::Control;
    use super::super::mode::DescriptionExt;
    use super::super::test_support::{ReportFeed, TxSpy};
    use super::{Held, Recovery, TX_CHANNELS_MAX, Tail, Tx, TxChain, TxCmd};
    use crate::device::{Device, Dir, FakeConfig, FakeDevice, FakeFault, Iq, Settings, TxCode, TxReport};
    use crate::provider::core::{Clock, Core, Holder, lock, stream_key};
    use crate::provider::test_support::{Frozen, device_at};
    use ezsdr_radio::timeline::Channels;

    mod e1;
    mod e2a;
    mod o1;

    fn held(clock: &super::Clock, k: i64, len: usize, policy: LatePolicy) -> Held {
        Held {
            k,
            domain: clock.domain,
            samples: Arc::new(vec![vec![[0.25f32, 0.0]; len]]),
            waveform_len: len,
            repeat: false,
            switch: false,
            untimed: false,
            open: BurstOpen { waveform_len: None, late: None, requested_target: None },
            policy,
            target: TimePoint::new(clock.domain, k),
            settings: Arc::new(super::ColdConfig::new(BTreeMap::new())),
        }
    }

    fn recovery_rig(device: Arc<dyn Device>) -> (Arc<Core>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, Clock, Arc<ezsdr_kernel::event::EventCollector>) {
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 200_000_000;
        let (core, time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, time, tx, clock, events)
    }

    #[test]
    fn tx_chain_device_burst_open_matrix() {
        let (core, _, _, _) = super::super::test_support::rig();
        let a = core.register(Dir::Tx, 200, 0).unwrap();
        let b = core.register(Dir::Tx, 100, 1_000_000).unwrap();
        let tail = Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 };
        let mut chain = TxChain::default();

        assert_eq!(chain.next(), None);
        assert_eq!(chain.front(), None);
        assert_eq!(chain.recovery(), Recovery::Clean);
        chain.open_device_burst(0, 20, None);
        assert_eq!(chain.next(), Some(20));
        assert!(chain.tail().is_none());

        chain.continue_device_burst(40, Some(tail));
        assert_eq!(chain.next(), Some(40));
        assert!(chain.continues_at(40));
        assert_eq!(chain.tail().map(|tail| tail.channels), Some(1));
        chain.advance_front(a.domain, 50);
        assert_eq!(chain.front(), Some((a.domain, 50)), "front remains independent of the open burst");

        chain.advance_front(a.domain, 45);
        assert_eq!(chain.front(), Some((a.domain, 50)), "same-domain front never regresses");
        chain.advance_front(b.domain, 3);
        assert_eq!(chain.front(), Some((b.domain, 3)), "a new clock replaces the stored front");
        chain.close_device_burst();
        assert_eq!(chain.next(), None);
        assert!(chain.tail().is_none());
    }

    #[test]
    fn tx_chain_recovery_transitions() {
        let mut chain = TxChain::default();
        chain.mark_reopen_needed();
        assert_eq!(chain.recovery(), Recovery::ReopenNeeded);

        chain.reopen_succeeded();
        assert_eq!(chain.recovery(), Recovery::Clean);
        chain.open_device_burst(0, 90, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        chain.mark_reopen_needed();
        chain.reopen_failed();
        assert_eq!(chain.recovery(), Recovery::Clean);
        assert!(chain.next().is_none() && chain.tail().is_none());
        chain.mark_reopen_needed();
        chain.provider_stop();
        assert_eq!(chain.recovery(), Recovery::Clean, "Provider::stop ends pending reopen work");
        chain.open_device_burst(0, 100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        chain.mark_reopen_needed();
        chain.lose();
        assert_eq!(chain.recovery(), Recovery::Clean);
        assert!(chain.next().is_none() && chain.tail().is_none());
    }

    struct NoActions;
    impl ActionReceiver for NoActions {
        fn recv(&self) -> Option<Dispatched> {
            None
        }
    }

    /// Transmit stream 0's clock, registered at `arm` (RM-25).
    fn tx_clock(core: &Core) -> Clock {
        lock(&core.streams).stream(stream_key(Dir::Tx, 0)).clock.expect("the transmit clock is registered at arm")
    }

    #[test]
    fn ur_22_uncertain_reopen_waits_for_owner_claim_and_stops_on_stop_or_loss() {
        let (core, device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(1));
        tx.chain.mark_reopen_needed();
        let key = stream_key(Dir::Tx, 0);
        let claim = lock(&core.streams).claim_owner(key).expect("test holds the stream");
        let before = device.calls();
        assert!(!tx.pass(), "a held stream claim defers the reopen for an idle turn");
        assert!(tx.chain.needs_reopen(), "a deferred reopen remains pending");
        assert_eq!(device.calls(), before, "the owner makes no call while another claim is held");
        lock(&core.streams).give_back(claim);
        assert!(tx.pass(), "the owner retries the reopen after the claim is returned");
        assert!(!tx.chain.needs_reopen());
        assert!(device.calls().iter().any(|call| call == "tx_send 0 n=4 at=- sob=true eob=true"), "the retry runs T3: {:?}", device.calls());

        let (stopped_core, stopped_device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut stopped = Tx::new(stopped_core.clone(), 0, cmds, None, Channels::first(1));
        stopped.chain.mark_reopen_needed();
        lock(&stopped_core.streams).stop = Some((stopped_core.now(), StopMode::Orderly));
        let before = stopped_device.calls();
        assert!(!stopped.pass(), "Stop ends an unavailable reopen");
        assert!(!stopped.chain.needs_reopen());
        assert_eq!(stopped_device.calls(), before, "no device call follows Stop");

        let (lost_core, lost_device, _, _) = super::super::test_support::rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut lost = Tx::new(lost_core.clone(), 0, cmds, None, Channels::first(1));
        lost.chain.mark_reopen_needed();
        lost_core.device_lost("test loss during reopen");
        let before = lost_device.calls();
        assert!(!lost.pass(), "device loss ends an unavailable reopen");
        assert!(!lost.chain.needs_reopen());
        assert_eq!(lost_device.calls(), before, "no device call follows loss");
    }

    #[test]
    fn ur_22_stop_and_loss_during_owner_recovery_release_the_claim() {
        for (at_ack_return, stop) in [(false, true), (false, false), (true, true), (true, false)] {
            let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
            let (core, _, _) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            spy.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            hold(&tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
            tx.chain.mark_reopen_needed();

            let (claim_reached, claim_release) = if at_ack_return {
                (None, None)
            } else {
                let (reached, wait) = std::sync::mpsc::channel();
                let release = Arc::new(std::sync::Barrier::new(2));
                let resume = release.clone();
                super::super::test_support::probe(&core, "tx_reopen_claimed", move |_| {
                    let _ = reached.send(());
                    resume.wait();
                });
                (Some(wait), Some(release))
            };
            let report_gate = at_ack_return.then(|| spy.stop_after("tx_async"));
            let (done, finished) = std::sync::mpsc::channel();
            let owner = core
                .device
                .host()
                .spawn("tx-recovery-race", move || {
                    let worked = tx.pass();
                    let _ = done.send((tx, worked));
                })
                .unwrap();
            if let Some(gate) = &report_gate {
                gate.wait();
                assert_eq!(spy.seen("tx_async"), 1, "the gated return is T3's final ACK read");
            } else {
                assert!(claim_reached.unwrap().recv_timeout(Wall::from_secs(10)).is_ok(), "recovery did not reach the post-claim gate");
            }
            let key = stream_key(Dir::Tx, 0);
            assert_eq!(lock(&core.streams).holder(key), Holder::Owner, "the owner holds its claim at the gate");

            if stop {
                let mut streams = lock(&core.streams);
                let at = core.now();
                streams.stop = Some((at, StopMode::Orderly));
                streams.end(&core, at, false, false);
                core.release_streams(streams);
            } else {
                core.device_lost("test loss during TX recovery");
            }
            let order_after_loss = (!stop).then(|| spy.order().len());
            if let Some(gate) = report_gate {
                gate.release();
            }
            if let Some(release) = claim_release {
                release.wait();
            }

            owner.join().unwrap();
            let (mut tx, worked) = finished.recv_timeout(Wall::from_secs(10)).unwrap();
            assert!(worked, "the owner handled the interrupted recovery");
            assert_eq!(lock(&core.streams).holder(key), Holder::Free, "the recovery claim is always returned");
            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Stop/loss must leave the chain Clean");
            assert!(held_keys(&tx).contains(&target), "recovery interruption does not refuse the pending burst");
            assert!(
                lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().is_some_and(|reason| reason.contains("could not be reopened"))),
                "Stop/loss is not reported as a failed-reopen cancellation"
            );
            assert!(spy.log().iter().filter(|call| call.starts_with("tx_send")).all(|call| call.contains("n=4 at=- sob=true eob=true")), "only T3, never experiment payload, is sent: {:?}", spy.log());
            if let Some(start) = order_after_loss {
                assert!(
                    spy.order()[start..].iter().all(|call| !matches!(*call, "tx_close" | "tx_open" | "tx_send" | "tx_async")),
                    "no TX device operation starts after loss publication: {:?}",
                    spy.order()
                );
            }

            halt(&mut tx);
            assert!(held_keys(&tx).is_empty(), "the Stop path settles the held burst");
        }
    }

    #[test]
    fn ur_22_underflow_close_failure_reopens_before_the_next_payload() {
        for (short_close, close_fails, reopen_fault, has_tail) in [
            (false, true, None, true),
            (false, true, None, false),
            (true, false, None, true),
            (false, true, Some(FakeFault::TxOpenFails(1)), true),
            (true, false, Some(FakeFault::RatesMismatch(0)), true),
        ] {
            let mut faults = Vec::new();
            if short_close {
                faults.push(FakeFault::ShortSend(1));
            }
            if let Some(fault) = reopen_fault.clone() {
                faults.push(fault);
            }
            let device = Arc::new(FakeDevice::new(FakeConfig { faults, ..FakeConfig::default() }));
            let (core, time, mut tx, clock, events) = recovery_rig(device.clone());
            device.set_time(0);

            let start = clock.at_or_after(device.time_now().unwrap() + core.ticks(5_000_000));
            let samples = [[0.1, 0.0]; 100];
            device.tx_send(0, &[&samples], Some(clock.instant(start)), true, false, Wall::from_secs(1)).unwrap();
            tx.chain.open_device_burst(0, start + samples.len() as i64, has_tail.then_some(Tail { samples: [[0.1, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
            let dry = clock.instant(start + samples.len() as i64) + core.ticks(5_000_000);
            time.advance_to(core.at(dry)).unwrap();
            device.set_time(dry);

            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            hold(&tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
            if close_fails {
                *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled non-lost EOB failure"));
            }
            tx.pass();
            assert!(tx.chain.needs_reopen(), "underflow's {short_close} close did not require recovery: {:?}", device.calls());
            assert_eq!(tx.chain.next(), None, "the failed close leaves no retained device cursor");
            assert!(tx.chain.tail().is_none(), "the abandoned tail is not replayed after failed close");
            assert!(tx.open_booking(|held| held.k) == Some(target) && tx.open_next() == Some(target) && tx.open_first() == Some(true), "B's new Kernel cursor stays unchanged at the recovery barrier");
            let target_send = format!("at={} sob=true", clock.instant(target));
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "B was sent on the uncertain streamer: {:?}", device.calls());

            *device.eob_error.lock().unwrap() = None;
            if reopen_fault.is_some() {
                use ezsdr_kernel::stream::{BlockFlags, BlockHeader, BurstTracker, ChannelMask, Direction};
                let mut tracker = BurstTracker::new(clock.domain);
                tracker
                    .on_block(
                        &BlockHeader {
                            first_sample_time: TimePoint::new(clock.domain, target),
                            len: 3,
                            channels: 1,
                            direction: Direction::Tx,
                            valid: ChannelMask::full(1),
                            flags: BlockFlags::START_OF_BURST,
                            lost: None,
                            contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
                        },
                        Some(BurstOpen::default()),
                    )
                    .unwrap();
                tx.tracker = Some(tracker);
                assert!(tx.pass(), "the failed reopen was not handled ({reopen_fault:?})");
                assert!(!tx.chain.needs_reopen(), "the failed reopen is no longer pending ({reopen_fault:?})");
                assert!(tx.clock.is_none(), "a failed reopen ends the stream's clock ({reopen_fault:?})");
                assert!(tx.chain.next().is_none() && tx.chain.tail().is_none(), "a failed reopen closes the device burst ({reopen_fault:?})");
                assert!(tx.open_next().is_none(), "B was cancelled from the send barrier ({reopen_fault:?})");
                assert!(!held_keys(&tx).contains(&target), "B left the held queue ({reopen_fault:?})");
                let rejected = lock(&core.rec).rejected.clone();
                assert!(rejected.iter().any(|row| row["reason"] == "UR-22: cancelled because the TX streamer could not be reopened"), "B's cancellation was recorded ({reopen_fault:?}): {rejected:?}");
                let record = lock(&core.rec).bursts.last().cloned().expect("the in-flight burst record was finished");
                assert_eq!(record["end"], "stop", "the in-flight burst record was finished ({reopen_fault:?}): {record}");
                assert_eq!(record["samples"], 3, "the in-flight burst record was preserved ({reopen_fault:?}): {record}");
                let calls = device.calls();
                assert!(calls.iter().all(|call| !call.contains(&target_send)), "B's payload was never sent ({reopen_fault:?}): {calls:?}");
                assert_eq!(device.transmitted(clock.instant(target), 3), [None, None, None], "no B samples reached the device ({reopen_fault:?})");
                let later = target + 1_000;
                let later_send = format!("at={} sob=true", clock.instant(later));
                let _ = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
                give(&mut tx, held(&clock, later, 1, LatePolicy::DropAndFlag));
                assert!(!held_keys(&tx).contains(&later), "a burst after the failed reopen is not held ({reopen_fault:?})");
                assert!(tx.clock.is_none(), "a burst after the failed reopen restarts the clock ({reopen_fault:?})");
                tx.pass();
                let calls = device.calls();
                assert!(calls.iter().all(|call| !call.contains(&target_send) && !call.contains(&later_send)), "a later turn sent a burst on the failed chain ({reopen_fault:?}): {calls:?}");
                let cancelled = lock(&core.rec).rejected.iter().filter(|row| row["reason"] == "UR-25: its transmit clock ended before the burst could start").count();
                assert_eq!(cancelled, 1, "the later burst's cancellation is reported once ({reopen_fault:?})");
                let commands =
                    events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count();
                assert_eq!(commands, 1, "the later burst's cancellation emits one COMMAND_REJECTED ({reopen_fault:?})");
                continue;
            }
            assert!(tx.pass(), "the owner did not replace the uncertain streamer");
            assert!(!tx.chain.needs_reopen());
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "the reopen turn sent B before its T3 completed: {:?}", device.calls());
            assert!(tx.pass(), "B was not sent after recovery");
            let calls = device.calls();
            let opens: Vec<_> = calls.iter().enumerate().filter(|(_, call)| call.starts_with("tx_open 0 ")).map(|(index, _)| index).collect();
            let reopened = *opens.get(1).expect("the streamer was reopened");
            let flush = calls.iter().enumerate().skip(reopened + 1).find(|(_, call)| *call == "tx_send 0 n=4 at=- sob=true eob=true").map(|(index, _)| index).expect("T3 flush");
            let payload = calls.iter().enumerate().skip(flush + 1).find(|(_, call)| call.contains(&target_send)).map(|(index, _)| index).expect("B's first payload");
            assert!(reopened < flush && flush < payload, "reopen/T3/payload order: {calls:?}");
        }
    }

    #[test]
    fn ur_22_owner_keeps_clean_recovery_after_an_unacknowledged_t3() {
        for fault in [FakeFault::DropBurstAck, FakeFault::ShortSend(0), FakeFault::FailSend(0)] {
            let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults: vec![fault.clone()], ..FakeConfig::default() }));
            let (core, _time, mut tx, clock, _events) = recovery_rig(device.clone());
            let old_front = (clock.domain, 123);
            tx.chain.advance_front(old_front.0, old_front.1);
            let old_target = TimePoint::new(clock.domain, 10);
            tx.remember_burst(old_target);
            let old_generation = tx.streamer;
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            hold(&tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
            tx.chain.mark_reopen_needed();

            let host = core.device.host();
            let clock_host = host.clone();
            let clock_driver = host.spawn("tx-flush-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");
            assert!(tx.pass(), "the owner did not complete its non-lost reopen ({fault:?})");
            clock_driver.join().unwrap();
            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Clean retains its current meaning ({fault:?})");
            assert_eq!(tx.chain.front(), Some(old_front), "an unacknowledged T3 has no tick to advance the front ({fault:?})");
            assert_ne!(tx.streamer, old_generation, "the replacement generation was adopted ({fault:?})");
            assert!(tx.unacked.is_empty(), "old-generation correlations are discarded ({fault:?})");
            assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1, "the incomplete T3 is recorded ({fault:?}): {:?}", lock(&core.rec).rejected);
            assert!(lock(&core.rec).device_async.iter().all(|row| row["tx_flush"] != true || row["code"] != "BurstAck"), "an absent acknowledgement is not recorded as one ({fault:?})");
            assert!(!device.calls().iter().any(|call| call.contains(&format!("at={} sob=true", clock.instant(target)))), "no experiment payload crossed the reopen turn ({fault:?})");

            assert!(tx.pass(), "Clean recovery did not permit the next owner dispatch ({fault:?})");
            assert!(
                device.calls().iter().any(|call| call == &format!("tx_send 0 n=4 at={} sob=true eob=true", clock.instant(target))),
                "the next timed dispatch follows today's Clean behavior ({fault:?}): {:?}",
                device.calls()
            );
            assert!(tx.unacked.iter().any(|pending| pending.target == TimePoint::new(clock.domain, target)), "the new-generation burst has its own correlation ({fault:?})");
        }

        let config = FakeConfig { clock: crate::ClockMode::Manual, faults: vec![FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck), FakeFault::DropBurstAck], ..FakeConfig::default() };
        let (core, device, _time, _) = super::super::test_support::rig_with(config, Vec::new());
        device.tx_open(0, &[0, 1]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(2));
        let old_front = (clock.domain, -1);
        tx.chain.advance_front(old_front.0, old_front.1);
        tx.chain.mark_reopen_needed();
        let host = core.device.host();
        let clock_host = host.clone();
        let clock_driver = host.spawn("tx-partial-ack-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");

        assert!(tx.pass(), "the owner did not finish the partial-ack reopen");
        clock_driver.join().unwrap();
        assert_eq!(tx.chain.recovery(), Recovery::Clean);
        assert_eq!(tx.chain.front(), Some(old_front), "one channel ack is not a completed T3 front");
        let rec = lock(&core.rec);
        assert_eq!(
            rec.device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| row["channel"].as_u64()).collect::<Vec<_>>(),
            [Some(0)],
            "the first channel ack is recorded while the second remains missing"
        );
        assert_eq!(rec.rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1);
    }

    #[test]
    fn ur_22_nonterminal_reopen_failures_cancel_held_initial_bursts() {
        for fault in [FakeFault::TxOpenFails(1), FakeFault::RatesMismatch(0)] {
            let device = Arc::new(FakeDevice::new(FakeConfig { faults: vec![fault.clone()], ..FakeConfig::default() }));
            let (core, _time, mut tx, clock, _events) = recovery_rig(device.clone());
            {
                let streams = lock(&core.streams);
                tx.read_cut(&streams, stream_key(Dir::Tx, 0));
                assert!(tx.clock.is_some(), "the initial segment is active before recovery ({fault:?})");
            }
            tx.chain.mark_reopen_needed();
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            hold(&tx, held(&clock, target, 3, LatePolicy::DropAndFlag));

            assert!(tx.pass(), "the failed reopen is handled ({fault:?})");
            assert!(!tx.chain.needs_reopen(), "recovery is no longer pending ({fault:?})");
            assert!(tx.clock.is_none(), "the initial segment was halted ({fault:?})");
            assert!(held_keys(&tx).is_empty(), "the held burst was cancelled ({fault:?})");
            let at = clock.instant(target);
            let calls = device.calls();
            assert!(!calls.iter().any(|call| call.contains(&format!("at={at} sob=true"))), "the held payload was never sent ({fault:?}): {calls:?}");
            assert_eq!(device.transmitted(at, 3), [None, None, None], "no held-burst samples reached the device ({fault:?})");
            let rejected = lock(&core.rec).rejected.clone();
            assert!(rejected.iter().any(|row| row["reason"] == "UR-22: cancelled because the TX streamer could not be reopened"), "the cancellation was recorded ({fault:?}): {rejected:?}");
        }
    }

    #[test]
    fn ur_26_provider_stop_ends_recovery_after_stop_and_keeps_shutdown_report_reads() {
        for fail_close in [false, true] {
            let faults = if fail_close { vec![FakeFault::ShortSend(0)] } else { Vec::new() };
            let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults, ..FakeConfig::default() }));
            let spy = Arc::new(TxSpy::new(inner));
            let (core, _, _) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            hold(&tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
            if fail_close {
                tx.chain.open_device_burst(0, 100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
            } else {
                tx.chain.mark_reopen_needed();
            }

            let host = core.device.host();
            let clock_host = host.clone();
            let clock_driver = host.spawn("tx-shutdown-deadline", move || clock_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");
            tx.shutdown();
            clock_driver.join().unwrap();

            assert_eq!(tx.chain.recovery(), Recovery::Clean, "Provider::stop follows stop_now (failed close={fail_close})");
            assert!(tx.chain.next().is_none() && tx.chain.tail().is_none());
            assert!(held_keys(&tx).is_empty(), "shutdown settles the held burst");
            let calls = spy.recs();
            assert_eq!(calls.iter().filter(|rec| rec.what == "tx_open").count(), 1, "shutdown never reopens the streamer: {calls:?}");
            assert!(calls.iter().any(|rec| rec.what == "tx_async"), "shutdown retains its report-read window: {calls:?}");
            assert!(!lock(&core.streams).reading(stream_key(Dir::Tx, 0)));
        }
    }

    #[test]
    fn ur_22_tx_stop_preserves_recovery_for_later_bursts() {
        for (fail_stop_close, has_tail) in [(true, true), (true, false), (false, false)] {
            let device = Arc::new(FakeDevice::new(FakeConfig::default()));
            let (core, _time, mut tx, clock, _events) = recovery_rig(device.clone());
            if fail_stop_close {
                tx.chain.open_device_burst(0, 100, has_tail.then_some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
                *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled Stop EOB failure"));
            } else {
                tx.chain.mark_reopen_needed();
            }

            halt(&mut tx);
            assert!(tx.chain.needs_reopen(), "ordinary Stop cleared the recovery requirement (failed close={fail_stop_close})");
            assert_eq!(tx.chain.next(), None, "Stop abandoned the failed-close cursor");
            assert!(tx.chain.tail().is_none(), "Stop abandoned the failed-close tail");
            *device.eob_error.lock().unwrap() = None;
            let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
            give(&mut tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
            let target_send = format!("at={} sob=true", clock.instant(target));

            assert!(tx.pass(), "Stop's pending recovery did not reopen the streamer");
            assert!(device.calls().iter().all(|call| !call.contains(&target_send)), "the reopen turn sent the later burst before T3: {:?}", device.calls());
            assert!(tx.pass(), "the later burst was not sent after recovery");
            let calls = device.calls();
            let reopened = calls.iter().enumerate().filter(|(_, call)| call.starts_with("tx_open 0 ")).nth(1).map(|(index, _)| index).expect("the streamer was reopened");
            let flush = calls.iter().enumerate().skip(reopened + 1).find(|(_, call)| *call == "tx_send 0 n=4 at=- sob=true eob=true").map(|(index, _)| index).expect("T3 flush");
            let payload = calls.iter().enumerate().skip(flush + 1).find(|(_, call)| call.contains(&target_send)).map(|(index, _)| index).expect("later burst payload");
            assert!(reopened < flush && flush < payload, "reopen/T3/payload order (failed close={fail_stop_close}): {calls:?}");
        }
    }

    #[test]
    fn ur_22_underflow_and_loss_end_the_damaged_burst() {
        for code in [crate::device::TxCode::Underflow, crate::device::TxCode::UnderflowInPacket, crate::device::TxCode::SeqError, crate::device::TxCode::SeqErrorInBurst] {
            let (core, device, time, events) = super::super::test_support::rig();
            device.tx_open(0, &[0]).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let mut burst = held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag);
            burst.repeat = true;
            give(&mut tx, burst);
            time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
            assert!(tx.step());
            give(&mut tx, held(&clock, 100_000, 10_000, LatePolicy::DropAndFlag));
            tx.report(crate::device::TxReport { code, tick: Some(clock.instant(22_000)), channel: 0 });
            assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
            assert!(!held_keys(&tx).contains(&20_000));
            assert!(held_keys(&tx).contains(&100_000), "future bursts retain their reservations");
            let sent = device.calls();
            assert!(!tx.step());
            halt(&mut tx);
            assert_eq!(device.calls(), sent, "no payload or second EOB after abandonment");
            assert_eq!(lock(&core.rec).bursts[0]["end"], "stop");
            assert!(
                events
                    .drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0))
                    .iter()
                    .any(|e| e.kind == EventKind::parse(ezsdr_radio::kinds::TX_UNDERFLOW).unwrap())
            );
        }
    }

    #[test]
    fn sc_25_the_underflow_event_carries_its_own_instant() {
        // SC-25: the event carries the time at which the underflow occurred. The
        // Authority's now is elsewhere — 11 ms of root time, and in the root domain —
        // and the sending clock's origin is not zero, its report tick one past sample
        // 22 000's, so neither "now when the report was read" nor a tick read without
        // that origin, nor a rounding up, can pass for the tick's floor.
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 1_000_003).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
        let underflow = |events: &Arc<EventCollector>| {
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().find(|e| e.kind.as_str() == ezsdr_radio::kinds::TX_UNDERFLOW).expect("TX_UNDERFLOW")
        };
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(clock.instant(22_000) + 1), channel: 0 });
        assert_eq!(underflow(&events).time, TimePoint::new(clock.domain, 22_000), "the report's tick in the sending SampleClock, floored, not the Authority's now");
        // A report with no tick is still timed: at the Authority's now.
        let now = core.now();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: None, channel: 0 });
        assert_eq!(underflow(&events).time, core.at(now));
    }

    #[test]
    fn ur_22_an_expired_timeline_is_not_resumed_before_its_report() {
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        give(&mut tx, held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag));
        time.advance_to(core.at(core.ticks(11_000_000))).unwrap();
        assert!(tx.step());
        time.advance_to(core.at(core.ticks(70_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open_next().is_none());
        assert_eq!(device.calls().iter().filter(|c| c.starts_with("tx_send ")).count(), 2, "one payload buffer and one empty EOB, no resumed payload");
    }

    #[test]
    fn ur_22_real_starvation_does_not_extend_the_stop_window() {
        // Intentionally starve the fake; the test does not require a millisecond
        // host wake-up. Synchronize the manual decision clock to the fake afterward.
        let (core, device, time, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let mut burst = held(&clock, 0, 100_000, LatePolicy::SendAsapAndFlag);
        burst.repeat = true;
        // Prime an untimed device burst to make starvation independent of a
        // host deadline for the initial timed SOB (covered by UR-21 tests).
        device.tx_send(0, &[&burst.samples[0][..2_000]], None, true, false, std::time::Duration::from_secs(1)).unwrap();
        tx.poke_open(burst, 2_000, false);
        // The primed send is the owner's device burst: an underflow ends only an open one (A5).
        tx.chain.open_device_burst(0, 2_000, None);
        tx.remember_burst(TimePoint::new(clock.domain, 0));
        std::thread::sleep(std::time::Duration::from_millis(60));
        time.advance_to(core.at(device.time_now().unwrap().max(core.now()))).unwrap();
        tx.reports(std::time::Duration::ZERO);
        assert!(lock(&core.rec).device_async.iter().any(|r| r["code"] == "Underflow"));
        assert!(tx.open_next().is_none());
        for _ in 0..20 {
            assert!(!tx.step());
        }
        let stop = device.time_now().unwrap();
        halt(&mut tx);
        std::thread::sleep(std::time::Duration::from_millis(20));
        tx.reports(std::time::Duration::ZERO);
        let rec = lock(&core.rec);
        let reports: Vec<_> = rec.device_async.iter().map(|report| report["code"].as_str().unwrap()).collect();
        assert_eq!(reports, ["Underflow", "BurstAck"], "the untimed DUC burst accepts its close at once: {:?}", rec.device_async);
        let end = rec.device_async.iter().find(|report| report["code"] == "BurstAck").unwrap()["tick"].as_i64().unwrap();
        assert!(end <= stop + core.ticks(10_000_000), "stop={stop}, playback end={end}");
        assert!(device.calls().iter().any(|call| call.contains("sob=false eob=true")), "the owner attempted a timed two-sample close: {:?}", device.calls());
        assert!(device.calls().iter().filter(|call| call.contains("sob=false eob=true")).all(|call| !call.contains("at=-")), "the close is passed to the DUC with a timestamp; stop={stop}");
    }

    #[test]
    fn ur_21_first_dispatch_rechecks_all_late_policies() {
        for (policy, expected) in [(LatePolicy::DropAndFlag, "drop"), (LatePolicy::RejectAtPlan, "plan_violation"), (LatePolicy::SendAsapAndFlag, "send_asap")] {
            let (core, device, time, events) = super::super::test_support::rig();
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            give(&mut tx, held(&clock, 20_000, 10_000, policy));
            assert!(!tx.step(), "still outside the in-flight window");
            time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
            assert!(tx.step());
            let events = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert!(events.iter().any(|e| e.payload["outcome"] == expected));
            assert_eq!(events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count(), 1, "one TIME_ERROR per late burst: {events:?}");
            if policy == LatePolicy::SendAsapAndFlag {
                let (k, open) = tx.open_booking(|held| (held.k, held.open)).unwrap();
                assert_eq!(k, 42_000);
                assert_eq!(open.requested_target, Some(TimePoint::new(clock.domain, 20_000)));
                assert!(matches!(open.late, Some(ezsdr_kernel::stream::LateOutcome::SendAsap { .. })));
            } else {
                assert!(tx.open_next().is_none());
                assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
            }
            assert!(!held_keys(&tx).contains(&20_000));
        }
    }

    #[test]
    fn ur_25_an_end_booked_after_the_first_read_cancels_the_held_burst_at_the_origin() {
        // UR-25, UR-26: an end booked after uhd-tx's first read ends the clock that has not begun at its origin. The Run end's
        // halt cancels the burst booked on that clock with `Provider::stop`'s note and no event; the pass after it ends the clock.
        use ezsdr_radio::timeline::{Channels, Config, Line, Stream};
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let origin = core.ticks(1_000_000_000);
        let c0 = core.register(Dir::Tx, 200, origin).unwrap();
        let config = Config { channels: Channels::first(1), ratio: ezsdr_kernel::time::Rational::new(200, 1).unwrap() };
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).clock = Some(c0);
            streams.lines.insert(stream_key(Dir::Tx, 0), Line::new(Stream { direction: ezsdr_kernel::stream::Direction::Tx, origin, config, lead: 0, call: 0, allowance: 0 }));
        }
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        give(&mut tx, held(&c0, 20_000, 1_000, LatePolicy::DropAndFlag));
        tx.pass();
        assert!(tx.cut.is_none());
        crate::provider::test_support::end_streams(&core, core.ticks(500_000_000), false, false);
        time.advance_to(core.at(origin)).unwrap();
        assert!(tx.pass(), "the first pass reads the halt");
        assert!(held_keys(&tx).is_empty());
        assert!(tx.clock.is_some(), "the clock ends on the next pass");
        tx.pass();
        assert!(tx.cut.is_none(), "the cut goes with the clock it ended");
        assert!(tx.clock.is_none());
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.iter().filter(|row| row["reason"] == "UR-26: cancelled by Provider::stop").count(), 1, "{rejected:?}");
        let rejected_events: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert!(rejected_events.is_empty(), "{rejected_events:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    #[test]
    fn ur_21_first_dispatch_preserves_an_on_time_start() {
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        give(&mut tx, held(&clock, 20_000, 10_000, LatePolicy::DropAndFlag));
        time.advance_to(core.at(core.ticks(18_000_000))).unwrap();
        assert!(tx.step());
        assert_eq!(tx.open_booking(|held| held.k).unwrap(), 20_000);
        assert!(tx.open_booking(|held| held.open.late).unwrap().is_none());
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert!(device.calls().iter().any(|c| c.contains("at=4000000 sob=true")));
    }

    #[test]
    fn ur_21_first_dispatch_refuses_a_moved_start_collision() {
        let (core, device, time, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        for k in [20_000, 42_000] {
            txp_book(&mut tx, &core, clock, held(&clock, k, 1_000, LatePolicy::SendAsapAndFlag));
        }
        time.advance_to(core.at(core.ticks(40_000_000))).unwrap();
        assert!(tx.step());
        assert!(tx.open_next().is_none());
        assert!(held_keys(&tx).contains(&42_000));
        assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|e| e.payload["outcome"] == "refused"));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    /// uhd-control on the transmit stream it begins at 1 MS/s from tick 0: the booking site
    /// whose on-time verdict a test reads beside uhd-tx's hand-over.
    fn booking_control(core: &Arc<Core>) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        booking_control_at(core, 0)
    }

    fn booking_control_at(core: &Arc<Core>, t0: i64) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        let clock = core.register(Dir::Tx, 200, t0).unwrap();
        booking_control_on(core, clock)
    }

    fn booking_control_on(core: &Arc<Core>, clock: Clock) -> (Control, std::sync::mpsc::Receiver<TxCmd>) {
        // As `arm` leaves the stream: the connect values are on the device, generation 0.
        lock(&core.streams).record_configured(stream_key(Dir::Tx, 0), Some(0));
        let (_to_tx, handed) = std::sync::mpsc::channel();
        let mut start = super::super::control::Start { t0: clock.origin, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        start.tx[0] = Some((clock, Channels::first(1)));
        (Control::new(core.clone(), Arc::new(NoActions), core.description.defaults(), false, start), handed)
    }

    fn waveform_input(name: &str, samples: &[[f32; 2]]) -> (ezsdr_kernel::manifest::ArtifactRef, Vec<u8>) {
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.iter().flat_map(|value| value.to_le_bytes())).collect();
        let waveform = ezsdr_kernel::manifest::ingest_input(
            ezsdr_kernel::spec::Ident::parse(name).unwrap(),
            ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
            format!("mem:{}", ContentHash::of_bytes(&bytes)),
            &bytes,
        );
        (waveform, bytes)
    }

    #[allow(clippy::too_many_arguments)]
    fn book_burst(control: &mut Control, core: &Core, id: u64, clock: Clock, k: i64, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, policy: LatePolicy) {
        book_burst_on(control, core, id, 0, clock, k, waveform, repeat, policy);
    }

    #[allow(clippy::too_many_arguments)]
    fn book_burst_on(control: &mut Control, core: &Core, id: u64, stream: usize, clock: Clock, k: i64, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, policy: LatePolicy) {
        book_with(control, core, id, stream, clock, k, waveform, repeat, policy, BTreeMap::new());
    }

    #[allow(clippy::too_many_arguments)]
    fn book_with(
        control: &mut Control,
        core: &Core,
        id: u64,
        stream: usize,
        clock: Clock,
        k: i64,
        waveform: &ezsdr_kernel::manifest::ArtifactRef,
        repeat: bool,
        policy: LatePolicy,
        metadata: BTreeMap<ezsdr_kernel::spec::Key, ezsdr_kernel::spec::Value>,
    ) {
        let node = core.description.stream_node(&core.id, Dir::Tx, stream);
        control.book(Dispatched::new(
            ActionId(id),
            Action::TxBurst {
                target: super::super::test_support::target_of(&node),
                waveform: waveform.clone(),
                repeat,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                requested_at: None,
                late_policy: policy,
                metadata,
            },
            Some(node),
        ));
    }

    /// The booking uhd-control put on transmit stream 0's list, taken off it without the owner reading it (UR-21).
    fn take_booking(core: &Core) -> Option<Held> {
        match lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.read() {
            Some(ezsdr_radio::staging::Read::Booking(held)) => Some(held),
            _ => None,
        }
    }

    fn drain_tx(tx: &mut Tx) {
        tx.read_list();
    }

    /// A booking received and read: the list's entry, then the owner's read (UR-21).
    fn give(tx: &mut Tx, held: Held) {
        lock(&tx.core.streams).stream_mut(stream_key(Dir::Tx, tx.k)).book.list.book(held);
        tx.read_list();
    }

    /// A Stop received and read (UR-26).
    fn halt(tx: &mut Tx) {
        lock(&tx.core.streams).stream_mut(stream_key(Dir::Tx, tx.k)).book.list.halt(ezsdr_radio::CancelReason::Stop);
        tx.read_list();
    }

    /// A burst held without a receipt: the state between a Stop's receipt and its read.
    fn hold(tx: &Tx, held: Held) {
        tx.hold(held);
    }

    fn held_keys(tx: &Tx) -> Vec<i64> {
        tx.with_held(|map| map.keys().copied().collect())
    }

    /// S1 (UHD): a booking inside a one-sample burst's front (its padded end plus the gap, 15 at 1 MS/s: `s + 19`) is refused at receipt; `s + 19` is accepted.
    #[test]
    fn ur_21_s1_overlap_refused_at_receipt() {
        let (a_wave, a_bytes) = waveform_input("s1_a", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("s1_b", &vec![[0.75, -0.25]; 100]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, _events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 2_001, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 3, clock, 2_005, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 4, clock, 2_019, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let rejected = lock(&core.rec).rejected.clone();
        let refused: Vec<_> = rejected.iter().filter(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst overlaps")).collect();
        assert_eq!(refused.len(), 2, "`s + 1` and `s + 5` are refused: {rejected:?}");
        assert!(held_keys(&tx).contains(&2_000) && held_keys(&tx).contains(&2_019), "A and `s + 19` are held: {:?}", held_keys(&tx));
        assert!(!held_keys(&tx).contains(&2_001) && !held_keys(&tx).contains(&2_005), "the refused bookings are not held");
    }

    /// S6 (UHD): a booking late by receipt moves to `first_free(ordinary)`, past the accepted span it would start in, with one `TIME_ERROR{send_asap}`.
    #[test]
    fn ur_21_s6_send_asap_moves_past_an_accepted_span() {
        let (a_wave, a_bytes) = waveform_input("s6_a", &vec![[0.25, 0.0]; 40]);
        let (b_wave, b_bytes) = waveform_input("s6_b", &vec![[0.75, -0.25]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 1_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let moved =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].payload["outcome"], "send_asap");
        assert!(held_keys(&tx).contains(&2_044), "B starts past A's front (2_044): {:?}", held_keys(&tx));
    }

    /// RM-14, RM-15 (UHD): lateness is decided before overlap, so a late `DropAndFlag` booking whose requested start meets an accepted span is
    /// dropped with `TIME_ERROR { drop }`, not refused for overlap.
    #[test]
    fn ur_21_s6_a_late_drop_and_flag_booking_is_dropped_before_the_overlap_is_judged() {
        let (a_wave, a_bytes) = waveform_input("s6_a", &vec![[0.25, 0.0]; 40]);
        let (b_wave, b_bytes) = waveform_input("s6_b", &vec![[0.75, -0.25]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 1_990, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let drops: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(drops.len(), 1, "{events:?}");
        assert_eq!(drops[0].payload["outcome"], "drop");
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "no overlap refusal: {events:?}");
    }

    /// RM-15 (UR-21, sequence 5, UHD only): an untimed head that is late at dispatch moves to the bound silently, with no `TIME_ERROR`, and
    /// the late policy is not consulted, so a `DropAndFlag` head moves too. Booked on time at 10 000 at device time 0; dispatched at 40 000.
    #[test]
    fn ur_21_s15_host_lateness_moves_an_untimed_burst_silently() {
        for policy in [LatePolicy::SendAsapAndFlag, LatePolicy::DropAndFlag] {
            let (core, device, time, events, clock, mut tx, mut control, (wave, _), _frozen) = untimed_rig(0);
            book_untimed(&mut control, &core, 1, clock, 10_000, &wave, false, policy);
            drain_tx(&mut tx);
            let head = tx.take_held(10_000).expect("the untimed head is booked at its instant");
            assert!(head.untimed, "the Module hands the flag to the owner");
            device_at(&core, &time, &device, txp_ticks(40_000));
            let lead = core.device_lead();
            let now = clock.at_or_after(ezsdr_radio::timeline::decided_at(core.now(), clock.origin, lead));
            let current = clock.at_or_after(clock.instant(now) + lead);
            let moved = tx.dispatch_start(clock, head).expect("a late untimed head is sent at the bound");
            assert_eq!(moved.k, current, "{policy:?}: the head moves to the bound");
            let errors = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count();
            assert_eq!(errors, 0, "{policy:?}: an untimed move is silent");
        }
    }

    /// RM-15, RM-16 (UR-21, sequence 5, M3): an untimed head moved at dispatch is placed after the front of the piece joined to it. Head H (40
    /// samples) at 10 000, piece P joined at 10 040 (front 10 084); the bound is 10 020, so H goes to P's front, not onto P.
    #[test]
    fn ur_21_s15_c_untimed_head_moved_past_its_joined_piece_goes_after_the_piece() {
        let (core, device, time, _events, clock, mut tx, mut control, (wave, _), _frozen) = untimed_rig(0);
        book_untimed(&mut control, &core, 1, clock, 10_000, &wave, false, LatePolicy::SendAsapAndFlag);
        book_burst(&mut control, &core, 2, clock, 10_040, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let head = tx.take_held(10_000).expect("the untimed head is booked");
        assert_eq!(tx.with_held(|map| map.get(&10_040).map(|piece| piece.k)), Some(10_040), "the piece joins the head's chain");
        device_at(&core, &time, &device, txp_ticks(8_020));
        let moved = tx.dispatch_start(clock, head).expect("a late untimed head is sent after its piece");
        assert_eq!(moved.k, 10_084, "the head is placed after the piece's front");
    }

    /// RM-15 (M3): an untimed head moved at dispatch is placed before a later booking it does not reach, not after it: the hand-over horizon
    /// does not apply to a placement at dispatch. Head H (40 samples) at 10 000, a timed chain C at 10 100; the bound is 10 020, and H is
    /// placed there, since its front (10 064) is before C.
    #[test]
    fn ur_21_s15_d_untimed_head_moved_before_a_later_booking_keeps_the_bound() {
        let (core, device, time, _events, clock, mut tx, mut control, (wave, _), _frozen) = untimed_rig(0);
        book_untimed(&mut control, &core, 1, clock, 10_000, &wave, false, LatePolicy::SendAsapAndFlag);
        book_burst(&mut control, &core, 2, clock, 10_100, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let head = tx.take_held(10_000).expect("the untimed head is booked");
        device_at(&core, &time, &device, txp_ticks(8_020));
        let moved = tx.dispatch_start(clock, head).expect("a late untimed head is sent at the bound");
        assert_eq!(moved.k, 10_020, "the head is placed at the bound, ahead of the later chain");
    }

    /// S6 (UHD, M1): a `SendAsap` move whose full front meets a later booking is placed after that booking's front, not refused (RM-16); the
    /// later booking is held where it is. The earlier chain at 9_960 is inside the horizon, so the first try is its front (10_002), which the booking at 10_010 meets.
    #[test]
    fn ur_21_s6_send_asap_past_a_later_booking_is_placed_after_it() {
        let (a_wave, a_bytes) = waveform_input("s6r_a", &vec![[0.25, 0.0]; 40]);
        let (b_wave, b_bytes) = waveform_input("s6r_b", &vec![[0.75, -0.25]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_960, &a_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 10_010, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 3, clock, 1_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let moved =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].payload["outcome"], "send_asap", "{moved:?}");
        let refused = lock(&core.rec).rejected.iter().filter(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst overlaps")).count();
        assert_eq!(refused, 0, "the moved booking is placed, not refused");
        assert!(held_keys(&tx).contains(&10_010) && held_keys(&tx).contains(&10_054), "the moved booking starts at the later booking's front: {:?}", held_keys(&tx));
    }

    /// Books a burst the Kernel stamped with no `at` (RS-19): `k` is its instant, and the Provider treats it as untimed (RM-15).
    #[allow(clippy::too_many_arguments)]
    fn book_untimed(control: &mut Control, core: &Core, id: u64, clock: Clock, k: i64, waveform: &ezsdr_kernel::manifest::ArtifactRef, repeat: bool, policy: LatePolicy) {
        let node = core.description.stream_node(&core.id, Dir::Tx, 0);
        control.book(Dispatched::new_untimed(
            ActionId(id),
            Action::TxBurst {
                target: super::super::test_support::target_of(&node),
                waveform: waveform.clone(),
                repeat,
                at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                requested_at: None,
                late_policy: policy,
                metadata: BTreeMap::new(),
            },
            Some(node),
        ));
    }

    /// The untimed rig of S6: the device at `now` samples, a 40-sample burst's waveform, the Provider's owner and its booking control.
    #[allow(clippy::type_complexity)]
    fn untimed_rig(
        now: i64,
    ) -> (
        Arc<Core>,
        Arc<FakeDevice>,
        Arc<ezsdr_kernel::time::ManualTimeAuthority>,
        Arc<EventCollector>,
        Clock,
        Tx,
        Control,
        (ezsdr_kernel::manifest::ArtifactRef, Vec<u8>),
        crate::provider::test_support::Frozen,
    ) {
        let (wave, bytes) = waveform_input("untimed", &vec![[0.25, 0.0]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes.clone()))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, now);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, device, time, events, clock, tx, control, (wave, bytes), frozen)
    }

    /// S5: of two overlapping bookings the one received second is refused, whatever their targets, so the
    /// receipt order decides which booking is kept. A reservation is not a hand-over: only `Tx::step` puts a kept burst into the device.
    #[test]
    fn ur_21_s5_receipt_order_decides_the_overlap() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples: Vec<[f32; 2]> = (0..2_000).map(|n| [-0.5, n as f32 / 2_000.0]).collect();
        let (a_wave, a_bytes) = waveform_input("a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("b", &b_samples);
        for (case, a_first) in [("A received first", true), ("B received first", false)] {
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let a = (1, 4_000, &a_wave, true);
            let b = (2, 3_000, &b_wave, false);
            let (first, second, kept) = if a_first { (a, b, 4_000) } else { (b, a, 3_000) };
            book_burst(&mut control, &core, first.0, clock, first.1, first.2, first.3, LatePolicy::SendAsapAndFlag);
            book_burst(&mut control, &core, second.0, clock, second.1, second.2, second.3, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            let refused: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
            assert_eq!(refused.len(), 1, "{case}: {refused:?}");
            assert_eq!(refused[0].payload["action"], second.0, "{case}: the refusal names the second booking");
            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
            assert_eq!(held_keys(&tx), vec![kept], "{case}: only the first booking is held");
            assert!(tx.step(), "{case}: the kept burst did not hand over");
            assert!(sends(&device).first().is_some_and(|send| send.1), "{case}: the kept burst opens a burst");
        }
    }

    /// RM-15 (note 47 stage 5 §1.3): the hand-over horizon is `handover_horizon(now, W)`, a sample of the stream's clock. C(40) at 12 000
    /// is beyond the horizon at receipt 0 (about 10 000), so the owner cannot have handed it: B(40) at 11 000, received after C, starts
    /// before it and is accepted, and both are held.
    #[test]
    fn ur_21_s5_c_a_booking_before_a_chain_beyond_the_horizon_is_accepted() {
        let (c_wave, c_bytes) = waveform_input("s5_c", &vec![[0.25, 0.0]; 40]);
        let (b_wave, b_bytes) = waveform_input("s5_b2", &vec![[0.75, -0.25]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(c_wave.hash.clone(), Arc::<[u8]>::from(c_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 12_000, &c_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 11_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let refused: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert!(refused.is_empty(), "nothing overlaps and the owner has not handed C: {refused:?}");
        assert_eq!(held_keys(&tx), vec![11_000, 12_000]);
    }

    /// #79 (RM-15, note 47 stage 5 §1.3): B(40) at 5 000 is received at 0, inside the 10 ms window, and the owner hands its first send
    /// at once. A(40) at 2 000, received after it, starts before B and may be on the device, so it is refused at receipt and never
    /// judged late at dispatch; B plays on time.
    #[test]
    fn ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt() {
        let (a_wave, a_bytes) = waveform_input("s5_a", &vec![[0.25, 0.0]; 40]);
        let (b_wave, b_bytes) = waveform_input("s5_b", &vec![[0.75, -0.25]; 40]);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "B is handed at device time 0");
        assert_eq!(sends(&device).first().map(|send| send.1), Some(true), "B's first send opens its burst: {:?}", device.calls());
        book_burst(&mut control, &core, 2, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let refused: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(refused.len(), 1, "A is refused at receipt: {refused:?}");
        assert_eq!(refused[0].payload["action"], 2, "the refusal names A");
        assert!(refused[0].payload["reason"].as_str().unwrap().contains("starts before a booking that may already be handed"), "{refused:?}");
        assert!(!held_keys(&tx).contains(&2_000), "A is not held: {:?}", held_keys(&tx));
        device_at(&core, &time, &device, core.ticks(6_000_000));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "no late drop: {events:?}");
        assert_eq!(device.transmitted(clock.instant(5_000), 39).iter().filter(|sample| sample.is_some()).count(), 39, "B plays: {:?}", device.calls());
    }

    #[test]
    fn ur_21_a_send_asap_move_is_reported_once_across_handovers() {
        let (a_wave, a_bytes) = waveform_input("one_sample_front", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("rechecked_start", &[[0.75, -0.25]]);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample predecessor was not handed over");
        assert_eq!(tx.chain.front(), Some((clock.domain, 10_018)), "the padded device EOB establishes the safe restart sample");

        // B is received at 9 ms, after its target 9 999: the receipt moves it to the first free sample at or after its
        // ordinary start, and reports the move once.
        device_at(&core, &time, &device, core.ticks(9_000_000));
        book_burst(&mut control, &core, 2, clock, 9_999, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let first: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].payload["outcome"], "send_asap");
        assert_eq!(first[0].payload["target"]["ticks"], 9_999);
        assert!(held_keys(&tx).contains(&11_000), "B was not placed at its first free start: {:?}", held_keys(&tx));
        let calls = device.calls().len();

        assert!(tx.step(), "B did not hand over");
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "the same action emitted a second TIME_ERROR"
        );
        assert!(device.calls().len() > calls, "B was not sent");
        assert_eq!(device.transmitted(clock.instant(11_000), 1), vec![Some([0.75, -0.25])], "B plays from its placed start");
    }

    #[test]
    fn ur_21_a_send_asap_judged_again_at_dispatch_is_not_reported_again() {
        let (a_wave, a_bytes) = waveform_input("one_sample_front", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("rejudged_start", &[[0.75, -0.25]]);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the one-sample predecessor was not handed over");
        device_at(&core, &time, &device, core.ticks(9_000_000));
        book_burst(&mut control, &core, 2, clock, 9_999, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let booked = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).count();
        assert_eq!(booked, 1, "the move at booking is reported once");
        device_at(&core, &time, &device, core.ticks(11_200_000));
        assert!(tx.step(), "the held burst was not dispatched");
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "the dispatch judgement reported the same move again"
        );
    }

    #[test]
    fn ur_21_tx_front_is_per_stream() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("stream0", &a_samples);
        let (b_wave, b_bytes) = waveform_input("stream1", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        for channel in [0, 1] {
            device.apply(Dir::Tx, channel, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            lock(&core.streams).record_configured(stream_key(Dir::Tx, channel), Some(0));
        }
        let c0 = core.register_in(Dir::Tx, 0, 200, 0).unwrap();
        let c1 = core.register_in(Dir::Tx, 1, 200, 0).unwrap();
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let (_to1, cmds1) = std::sync::mpsc::channel();
        let start = super::super::control::Start { t0: 0, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![Some((c0, Channels::first(1))), Some((c1, Channels::first(1)))] };
        let mut control = Control::new(core.clone(), Arc::new(NoActions), core.description.defaults(), false, start);
        let mut tx0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut tx1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(1));
        book_burst_on(&mut control, &core, 1, 0, c0, 4_000, &a_wave, true, LatePolicy::DropAndFlag);
        book_burst_on(&mut control, &core, 2, 1, c1, 3_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx0);
        drain_tx(&mut tx1);
        assert!(tx0.step() && tx1.step());
        assert_eq!(tx1.chain.next(), Some(5_000), "stream 0's hand-over front did not leak");
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send 1 ")));
        assert!(
            device.calls().iter().any(|call| { call.starts_with("tx_send 1 ") && call.contains(&format!("at={} sob=true", c1.instant(3_000))) }),
            "stream 1's B did not start at its own target: {:?}",
            device.calls()
        );
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    }

    #[test]
    fn ur_21_a_target_after_a_completed_burst_is_on_time() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("expired_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("expired_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(2_000_000));
        assert!(tx.step(), "A's EOB was not sent");
        assert_eq!(tx.chain.front(), Some((clock.domain, 4_004)));
        device_at(&core, &time, &device, core.ticks(10_000_000));
        book_burst(&mut control, &core, 2, clock, 13_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step(), "the burst after the front's expiry did not dispatch");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(13_000))) && call.contains("sob=true")));
    }

    #[test]
    fn ur_21_send_does_not_cross_the_in_flight_window() {
        let a_samples = vec![[0.25, 0.0]; 30_000];
        let b_samples = vec![[0.75, -0.25], [0.5, 0.25]];
        let (a_wave, a_bytes) = waveform_input("window_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("window_boundary", &b_samples);
        for (rate, root_ticks_per_sample, start, expected_cut) in [(1e6, 200, 9_000, 10_000), (390_625.0, 512, 979, 3_907)] {
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(rate), ..Settings::default() }).unwrap();
            let clock = core.register(Dir::Tx, root_ticks_per_sample, 0).unwrap();
            let (mut control, cmds) = booking_control_on(&core, clock);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, start, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.step() {}
            let case = format!("{rate} samples/s");
            assert_eq!(tx.open_next().unwrap(), expected_cut, "{case}: exclusive first-unhanded sample");
            let handed = (expected_cut - start) as usize;
            assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), handed, "{case}: total payload handed");
            assert_eq!(device.transmitted(clock.instant(start), handed), vec![Some([0.25, 0.0]); handed], "{case}: payload values");
            assert_eq!(device.transmitted(clock.instant(expected_cut), 1), vec![None], "{case}: the boundary sample is not handed");
            // The open repeat's span runs on past the cut: a booking at the cut is refused at receipt, not admitted.
            let calls = device.calls().len();
            book_burst(&mut control, &core, 2, clock, expected_cut, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            let refused: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
            assert_eq!(refused.len(), 1, "{case}: {refused:?}");
            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
            assert_eq!(device.calls().len(), calls, "{case}: the refused booking reached the device");
        }
    }

    #[test]
    fn ur_21_tx_reads_in_flight_window_from_description() {
        let samples = vec![[0.25, 0.0]; 10_000];
        let (wave, bytes) = waveform_input("custom_window", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 4_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes.clone()))]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert_eq!(tx.open_next().unwrap(), 4_000);
        assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), 1_000);

        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 4_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(!tx.step(), "a burst at +5 ms is outside the custom 4 ms window");
        assert!(held_keys(&tx).contains(&5_000));
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_send ")), "the outside-window burst reached the device: {:?}", device.calls());
    }

    #[test]
    fn measurement_candidate_in_flight_window_sets_the_tx_chunk_size() {
        let samples = vec![[0.25, 0.0]; 10_000];
        let (wave, bytes) = waveform_input("candidate_chunk", &samples);
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(10_000);
        description.timing.tx_in_flight_ns = 8_000_000;
        let (core, time, _) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]), description);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (mut control, cmds) = booking_control_on(&core, clock);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert_eq!(tx.open_next().unwrap(), 5_000, "the candidate 8 ms window hands over its 2 ms chunk");
        assert_eq!(sends(&device).iter().map(|(n, _, _)| n).sum::<usize>(), 2_000);
    }

    #[test]
    fn measurement_candidate_admission_lead_does_not_override_the_device_lead() {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.min_timed_command_lead_ns = 1_000_000;
        let (core, _time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, commands) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, commands, Some(clock), Channels::first(1));
        let target = clock.at_or_after(tx.core.now() + tx.core.ticks(1_500_000));
        hold(&tx, held(&clock, target, 100, LatePolicy::DropAndFlag));
        assert!(tx.step(), "the queued burst is evaluated by the Provider");
        assert!(tx.open_next().is_none(), "the burst inside the device lead is dropped");
        let errors: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].payload["outcome"], "drop");
        assert!(device.calls().iter().all(|call| !call.starts_with("tx_send ")), "a dropped burst reached the device: {:?}", device.calls());
    }

    #[test]
    fn ur_21_a_new_burst_without_an_open_predecessor_is_on_time() {
        let samples = vec![[0.75, -0.25]];
        let (wave, bytes) = waveform_input("alone", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 5_000, &wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert_eq!(sends(&device).last(), Some(&(4, true, true)), "a one-sample first send is padded to four (UR-22)");
        assert_eq!(device.transmitted(clock.instant(5_000), 1), vec![Some([0.75, -0.25])]);
    }

    #[test]
    fn ur_21_s2_a_finite_front_stops_at_its_end() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("finite_predecessor", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_finite", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 3_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(tx.open_next().is_none() && tx.chain.next() == Some(5_000));
        device_at(&core, &time, &device, core.ticks(3_000_000));
        assert!(tx.step(), "the final sample and EOB were not handed");
        assert_eq!(tx.chain.front(), Some((clock.domain, 5_004)));
        book_burst(&mut control, &core, 2, clock, 6_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(6_000))) && call.contains("sob=true")));
    }

    #[test]
    fn ur_21_a_finite_burst_inside_the_window_is_refused() {
        let a_samples = vec![[0.25, 0.0]; 27_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("long_finite", &a_samples);
        let (b_wave, b_bytes) = waveform_input("inside_long_finite", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open_next().is_none_or(|next| next < 10_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open_next().unwrap(), 10_000, "finite A is clamped by W, not its end");
        let calls = device.calls().len();
        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "B is refused, not late");
        assert_eq!(device.calls().len(), calls, "B reached the device");
    }

    /// S7: a handed repeat holds its whole span until a Stop, so a booking inside it is refused at receipt
    /// (even a late one, which is not moved) and the repeat keeps its cursor.
    #[test]
    fn ur_21_s7_a_handed_repeat_refuses_a_booking_inside_its_span() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (a_wave, a_bytes) = waveform_input("held_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("inside_held_repeat", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open_next().is_none_or(|next| next < 10_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open_next().unwrap(), 10_000);
        book_burst(&mut control, &core, 2, clock, 6_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let refused: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert_eq!(refused[0].payload["action"], 2);
        assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
        assert_eq!(tx.chain.next(), Some(10_000));
        assert!(held_keys(&tx).is_empty(), "B was not held");
        assert_eq!(sends(&device).iter().filter(|(_, sob, _)| *sob).count(), 1);
    }

    /// A late SendAsap move finds no free sample behind a handed repeat (`first_free` is `None`): the booking is
    /// refused at receipt with a `refused` TIME_ERROR, and the repeat keeps its cursor.
    #[test]
    fn ur_21_a_late_move_into_a_handed_repeat_is_refused() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("cursor_repeat", &a_samples);
        let b_samples = vec![[0.75, -0.25]; 2_000];
        let (b_wave, b_bytes) = waveform_input("moved_after_cursor", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        while tx.open_next().is_none_or(|next| next < 10_000) {
            assert!(tx.step());
        }
        device_at(&core, &time, &device, core.ticks(9_000_000));
        book_burst(&mut control, &core, 2, clock, 5_000, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0].payload["action"], 2);
        assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"));
        let moved: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].payload["outcome"], "refused");
        assert_eq!(moved[0].payload["target"]["ticks"], 5_000);
        assert!(held_keys(&tx).is_empty());
        assert_eq!(tx.chain.next(), Some(10_000), "the repeat keeps its cursor");
    }

    /// S4 (RM-15): a finite head longer than 32 samples holds `[s, e + 4)`, so a booking at `e + 3` is refused at
    /// receipt and a booking at `e + 4` is on time.
    #[test]
    fn ur_21_s4_a_booking_at_the_finite_front_is_on_time() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25], [0.5, 0.25]];
        let (a_wave, a_bytes) = waveform_input("finite_head", &a_samples);
        let (b_wave, b_bytes) = waveform_input("boundary_b", &b_samples);
        for (target, refused) in [(4_503, true), (4_504, false)] {
            let case = format!("B at {target}");
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, false, LatePolicy::DropAndFlag);
            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{case}: on time either way");
            let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
            if refused {
                assert_eq!(rejected.len(), 1, "{case}: {rejected:?}");
                assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{case}");
                assert_eq!(held_keys(&tx), vec![2_500], "{case}");
            } else {
                assert!(rejected.is_empty(), "{case}: {rejected:?}");
                assert_eq!(held_keys(&tx), vec![2_500, target], "{case}");
            }
        }
    }

    /// Samples `[(i + 1) / 100 000, k / 10 000]` of a burst at `k`, as `txp_burst` makes them, as the device plays them.
    fn played(k: i64, len: usize) -> Vec<Option<[f32; 2]>> {
        (0..len).map(|i| Some([(i + 1) as f32 / 100_000.0, k as f32 / 10_000.0])).collect()
    }

    /// S4b (§2, R4-5): A(40) is handed, C(10) is booked early at `s + 100`, then B(10) joins A at `s + 40`, received before
    /// `J(s + 40)`. B continues A with no end-of-burst at `s + 40`, and C plays on time at `s + 100`.
    #[test]
    fn ur_21_s4b_three_bookings_b_continues_a_and_c_plays_on_time() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_100, 10, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_040, 10, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert!(sends(&device).iter().all(|send| !send.2), "no end-of-burst before B's end: {:?}", device.calls());
        device_at(&core, &time, &device, core.ticks(15_500_000));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 2, "A and B are one burst, C another: {:?}", device.calls());
        assert_eq!(sends(&device).iter().filter(|send| send.2).count(), 2, "{:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(20_000), 50), [played(20_000, 40), played(20_040, 10)].concat());
        assert_eq!(device.transmitted(clock.instant(20_100), 10), played(20_100, 10));
        assert_eq!(past_front(&core), 0, "no owner close past the record's front");
    }

    /// S4b at the deadline (UR-21, RM-15): B(10) joins A at `s + 40` and is received exactly at `J(s + 40)`, so the record
    /// accepts it and the device burst continues: no end-of-burst at `s + 40`, B plays from `s + 40`.
    #[test]
    fn ur_21_s4b_booking_exactly_at_the_deadline_continues() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, core.join_by(clock, 20_040));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_040, 10, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, core.ticks(15_500_000));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 1, "A and B are one burst: {:?}", device.calls());
        assert_eq!(sends(&device).iter().filter(|send| send.2).count(), 1, "no end-of-burst at s + 40: {:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(20_000), 50), [played(20_000, 40), played(20_040, 10)].concat(), "{:?}", device.calls());
        assert_eq!(past_front(&core), 0);
    }

    /// M1 (R4-m2, row 56): A(40) is handed and C(40) is booked early at `s + 42`, which A's record front allows. The owner
    /// closes A at `J(s + 40)` (148 µs before the step), so C starts its own burst on time; the close at the lead deadline
    /// `instant(s + 40) − lead` would start C late.
    #[test]
    fn ur_21_m1_148us_booked_early_at_the_chain_front_plays_on_time() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_044, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(15_188_000));
        txp_drain(&mut tx);
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 2, "C starts its own burst at J + 148 µs: {:?}", device.calls());
        device_at(&core, &time, &device, core.ticks(21_000_000));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert_eq!(device.transmitted(clock.instant(20_044), 40), played(20_044, 40));
        assert_eq!(past_front(&core), 0, "A's close is at its record front");
    }

    /// M1 (R4-m2, row 57; S1): B(10) joins A at `s + 40` and C(10) joins B at `s + 50`; both are booked (record and list in one receipt)
    /// before the owner reaches the close point after `J(s + 40)`. B and C continue A with no end-of-burst at `s + 40`.
    #[test]
    fn ur_21_m1_joins_booked_early_continue_the_device_burst() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_040, 10, false, LatePolicy::DropAndFlag));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_050, 10, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(15_500_000));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 1, "B and C continue A: {:?}", device.calls());
        assert_eq!(sends(&device).iter().filter(|send| send.2).count(), 1, "{:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(20_000), 60), [played(20_000, 40), played(20_040, 10), played(20_050, 10)].concat());
        assert_eq!(past_front(&core), 0);
    }

    /// M1 (c, R4-m2, row 58): a join received at `J(s + 40)` is accepted, and one received a tick later is refused. The
    /// record decides both at the receipt.
    #[test]
    fn ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record() {
        for (case, offset, accepted) in [("at J(e)", 0, true), ("one tick after J(e)", 1, false)] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
            txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
            device_at(&core, &time, &device, core.ticks(12_000_000));
            txp_drain(&mut tx);
            let deadline = clock.instant(20_040) - core.ticks(5_000_000);
            device_at(&core, &time, &device, deadline + offset);
            txp_drain(&mut tx);
            let mut streams = lock(&core.streams);
            let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
            let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
            let decided = busy.accept(20_040, Some(10), core.now(), |k| core.join_by(clock, k), horizon);
            drop(streams);
            assert_eq!(decided.is_ok(), accepted, "{case}");
        }
    }

    /// S2 (note 47 stage 5, Opus re-check): A(40), B(10) joined at `s + 40` and C(4) joined at `s + 50`, all booked early. A's configure
    /// is refused and A leaves, so B is the head and at most 32 samples: the record closes it at its standalone front (UR-22, UR-23).
    /// B goes out as one burst closed at its end, and C, which joined it, is decided by its late policy: not played.
    #[test]
    fn ur_21_s2_a_short_surviving_head_is_closed_and_a_piece_joined_behind_it_is_late() {
        let (core, device, time, mut tx, _frozen, events, _to_tx) = owner_with(1, vec![FakeFault::TxApplyRefused(0, F2)]);
        let clock = tx.clock.expect("a clock");
        for (k, len, id, generation, frequency) in [(40_000, 40, 1, 1, F2), (40_040, 10, 2, 2, F3), (40_050, 4, 3, 2, F3)] {
            txp_book(&mut tx, &core, clock, valued(&clock, k, len, id, generation, frequency, 1));
        }
        turns(&core, &time, &device, &mut tx, 1, 60, 1);
        let drained = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(drained.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(), [ezsdr_radio::kinds::COMMAND_REJECTED, ezsdr_radio::kinds::TIME_ERROR], "{:?}", device.calls());
        assert_eq!(
            (drained[0].payload["action"].clone(), drained[1].payload["outcome"].clone()),
            (serde_json::json!(1), serde_json::json!("drop")),
            "A is refused; C is dropped late: {:?}",
            device.calls()
        );
        assert_eq!(sends(&device), [(10, true, true)], "B is one burst closed at its end: {:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(40_040), 10), vec![Some([0.25, 0.0]); 10]);
        assert_eq!(device.transmitted(clock.instant(40_050), 4), vec![None; 4], "C is not played: {:?}", device.calls());
        let reports: Vec<TxCode> = std::iter::from_fn(|| device.tx_async(0, std::time::Duration::ZERO).unwrap()).map(|report| report.code).collect();
        assert_eq!(reports, [TxCode::BurstAck], "the device acknowledges B's burst and reports no underflow: {:?}", device.calls());
    }

    /// S2 (note 47 stage 5, Opus re-check): A(40) at 40 000 is refused and leaves; B(10) at 40 040, C(4) at 40 050 and D(10) at 40 054
    /// are joined behind it. B, left as a standalone closed short head, fronts at 40 065 (UR-22), so E(10) at 40 056, received before D
    /// is dispatched, is refused at receipt, B plays, and C and D are late at dispatch under their DropAndFlag policy.
    #[test]
    fn ur_21_s2_c_and_d_joined_behind_a_refused_head_are_late_at_dispatch() {
        let (core, device, time, mut tx, _frozen, events, _to_tx) = owner_with(1, vec![FakeFault::TxApplyRefused(0, F2)]);
        let clock = tx.clock.expect("a clock");
        let bursts = [(40_000, 40, 1, 1, F2), (40_040, 10, 2, 2, F3), (40_050, 4, 3, 2, F3), (40_054, 10, 4, 2, F3)]
            .map(|(k, len, id, generation, frequency)| valued(&clock, k, len, id, generation, frequency, 1));
        for burst in bursts {
            txp_book(&mut tx, &core, clock, burst);
        }
        let mut receipt = None;
        for ms in 1..=60 {
            device_at(&core, &time, &device, core.ticks(ms * 1_000_000));
            for _ in 0..8 {
                drain_tx(&mut tx);
                if receipt.is_none() && !held_keys(&tx).contains(&40_000) && held_keys(&tx).contains(&40_054) {
                    let mut streams = lock(&core.streams);
                    let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
                    let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
                    receipt = Some(busy.accept(40_056, Some(10), core.now(), |k| core.join_by(clock, k), horizon));
                }
                if !tx.step() {
                    break;
                }
            }
        }
        assert_eq!(receipt, Some(Err(ezsdr_radio::staging::Overlap::Booking)), "E, received after A has left and before D is dispatched, is refused at receipt: {:?}", device.calls());
        let drained = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(
            drained.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(),
            [ezsdr_radio::kinds::COMMAND_REJECTED, ezsdr_radio::kinds::TIME_ERROR, ezsdr_radio::kinds::TIME_ERROR],
            "{:?}",
            device.calls()
        );
        assert_eq!(drained[1].payload["outcome"], "drop", "C is late at dispatch: {:?}", device.calls());
        assert_eq!(drained[2].payload["outcome"], "drop", "D is inside B's front, so late at dispatch: {:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(40_040), 10), vec![Some([0.25, 0.0]); 10], "B plays as a standalone closed short head: {:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(40_050), 14), vec![None; 14], "C and D are not sent: {:?}", device.calls());
    }

    /// Q-6 (§1.4, row 60): an owner close whose device front is past the record's front is counted once. A close at the
    /// record's front, as in S4b, is not.
    #[test]
    fn ur_21_stat_counts_owner_closes_past_the_record_front() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, core.ticks(15_500_000));
        txp_drain(&mut tx);
        assert_eq!(past_front(&core), 0, "the owner closed at the record's front");

        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, core.ticks(12_000_000));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, core.ticks(15_500_000));
        tx.chain.advance_front(clock.domain, 20_100);
        assert!(tx.close_unjoined(clock, 20_040));
        assert_eq!(past_front(&core), 1, "a device front past the record's front is counted");
    }

    #[test]
    fn ur_23_stop_preserves_handed_cursor_after_stop() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_stop", &b_samples);
        for delta in 0..=5 {
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_500, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            while tx.open_next().is_none_or(|next| next < 10_000) {
                assert!(tx.step(), "A stopped making progress before the 10 ms hand-over frontier");
            }
            assert_eq!(tx.open_next().unwrap(), 10_000, "the future A is handed to H before Stop");
            control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
            drain_tx(&mut tx);
            let record = lock(&core.rec).bursts.last().cloned().expect("the stopped repeat record");
            assert_eq!(record["end"], "stop");
            assert_eq!(record["samples"], 7_500);
            assert_eq!(sends(&device).last(), Some(&(2, false, true)), "Stop closes the device burst with two zeros at H");

            let target = 10_000 + delta;
            let before = device.calls().len();
            device_at(&core, &time, &device, core.ticks((delta + 1) * 1_000 + 1));
            book_burst(&mut control, &core, 3 + delta as u64, clock, target, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "B at {target} is on time at receipt");
            if delta < 5 {
                // The Stop's region [10 000, 10 005) (the stop bound is the safe sample H + 5) holds these starts.
                let refused: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
                assert_eq!(refused.len(), 1, "B at {target}: {refused:?}");
                assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "B at {target}");
                assert_eq!(device.calls().len(), before, "a refused B reached the device");
            } else {
                assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "the safe sample is accepted");
                assert!(tx.step(), "B at {target} was not handed over");
                assert_eq!(sends(&device).last(), Some(&(4, true, true)), "B starts with a timed SOB after H+5, padded to four");
                assert_eq!(device.transmitted(clock.instant(10_000), 6), vec![Some([0.0, 0.0]), Some([0.0, 0.0]), None, None, None, Some([0.75, -0.25])]);
                device_at(&core, &time, &device, clock.instant(10_009) + 1);
                tx.reports(Wall::ZERO);
                assert!(lock(&core.rec).device_async.iter().any(|row| row["code"] == "BurstAck" && row["tick"] == clock.instant(10_009) + 1), "the safe burst's acknowledgement is read");
            }
        }
    }

    /// A booking accepted at the front (10 018, A's padded end plus the gap) and not yet handed over is cancelled by a Stop and never sent (RM-16):
    /// the one-sample A at 9 999 is handed and padded, and the Stop sends nothing more.
    #[test]
    fn ur_23_stop_does_not_send_unhanded_window_edge_payload() {
        let (a_wave, a_bytes) = waveform_input("window_edge_a", &[[0.5, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("window_edge_b", &[[0.0, 0.5]]);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 2, clock, 10_018, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED),
            "B at the front is accepted"
        );
        assert!(tx.step(), "the one-sample A at 9 999 was not handed over");
        assert_eq!(sends(&device).last(), Some(&(4, true, true)), "A is padded and closed at once; B is not handed yet");
        halt(&mut tx);
        assert_eq!(sends(&device).last(), Some(&(4, true, true)), "Stop sends nothing after the closed device burst");

        assert_eq!(lock(&core.rec).bursts.len(), 1, "only A has a burst record");
        assert_eq!(lock(&core.rec).bursts[0]["samples"], 1);
        assert_eq!(device.transmitted(clock.instant(9_999), 4), vec![Some([0.5, 0.0]), Some([0.0, 0.0]), Some([0.0, 0.0]), Some([0.0, 0.0])], "FakeDevice received only A and its padding, not B");
        device_at(&core, &time, &device, core.ticks(10_020));
        assert_eq!(device.transmitted(clock.instant(10_018), 1), vec![None], "B was not sent");
    }

    /// The rig of a dropped window-edge head (RM-16, N-P6): device at 0 with a 10 ms window; A (one sample at 9 999) is booked, then a
    /// Stop at device 0 drops it, and its region reaches its own front, 10 018. Returns the rig with the booking control, and the waveform
    /// of a later booking.
    #[allow(clippy::type_complexity)]
    fn dropped_head_rig()
    -> (Arc<Core>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, Control, ezsdr_kernel::manifest::ArtifactRef, crate::provider::test_support::Frozen) {
        let (a_wave, a_bytes) = waveform_input("region_a", &[[0.5, 0.0]]);
        let (d_wave, d_bytes) = waveform_input("region_d", &[[0.0, 0.5]]);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (d_wave.hash.clone(), Arc::<[u8]>::from(d_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 9_999, &a_wave, false, LatePolicy::DropAndFlag);
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        drain_tx(&mut tx);
        (core, time, events, clock, tx, control, d_wave, frozen)
    }

    /// RM-16 (UHD, N-P6): a booking at 10 004 inside a dropped head's region follows its late policy. DropAndFlag is refused at receipt;
    /// SendAsapAndFlag moves it to 10 018 with `TIME_ERROR{send_asap}`.
    #[test]
    fn ur_23_a_booking_in_a_dropped_head_s_region_follows_its_late_policy() {
        for policy in [LatePolicy::DropAndFlag, LatePolicy::SendAsapAndFlag] {
            let case = format!("{policy:?}");
            let (core, _time, events, clock, mut tx, mut control, d_wave, _frozen) = dropped_head_rig();
            book_burst(&mut control, &core, 3, clock, 10_004, &d_wave, false, policy);
            drain_tx(&mut tx);
            let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            let refused = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count();
            let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            match policy {
                LatePolicy::DropAndFlag => {
                    assert_eq!(refused, 1, "{case}: {events:?}");
                    assert!(late.is_empty(), "{case}: a refused booking is not late");
                }
                _ => {
                    assert_eq!(refused, 0, "{case}: {events:?}");
                    assert_eq!(late.len(), 1, "{case}: {events:?}");
                    assert_eq!(late[0].payload["outcome"], "send_asap", "{case}");
                    assert!(held_keys(&tx).contains(&10_018), "{case}: the moved booking starts at 10 018: {:?}", held_keys(&tx));
                }
            }
        }
    }

    /// RM-16 (UHD, N-P6): a booking at the region's end, 10 013, is admitted on time: no refusal and no `TIME_ERROR`.
    #[test]
    fn ur_23_a_booking_at_a_dropped_head_s_front_is_admitted() {
        let (core, _time, events, clock, mut tx, mut control, d_wave, _frozen) = dropped_head_rig();
        book_burst(&mut control, &core, 3, clock, 10_018, &d_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "{events:?}");
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert!(held_keys(&tx).contains(&10_018), "the booking at 10 018 is held: {:?}", held_keys(&tx));
    }

    /// RM-15, RM-16 (UHD, N-P6): an untimed booking after a dropped head is placed at its front, 10 018, with no `TIME_ERROR` and no
    /// refusal: the late policy is not consulted.
    #[test]
    fn ur_23_an_untimed_booking_after_a_dropped_head_is_placed_at_its_front() {
        let (core, _time, events, clock, mut tx, mut control, d_wave, _frozen) = dropped_head_rig();
        book_untimed(&mut control, &core, 3, clock, 10_004, &d_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "{events:?}");
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "{events:?}");
        assert!(tx.with_held(|map| map.get(&10_018).is_some_and(|held| held.untimed)), "the untimed booking is placed at 10 018: {:?}", held_keys(&tx));
    }

    #[test]
    fn ur_23_stop_closes_a_running_repeat_at_its_handed_cursor() {
        let samples = vec![[0.25, 0.0]; 2_000];
        let (wave, bytes) = waveform_input("running_repeat", &samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let (core, time, _) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes))]));
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &wave, true, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(4_000_000));
        while tx.open_next().is_some_and(|next| next < 12_000) {
            assert!(tx.step());
        }
        assert_eq!(tx.open_next().unwrap(), 12_000);
        control.book(Dispatched::new(ActionId(2), Action::Stop { target: Some(super::super::test_support::target_of(&core.tx_id)) }, Some(core.tx_id.clone())));
        drain_tx(&mut tx);
        let record = lock(&core.rec).bursts.last().cloned().unwrap();
        assert_eq!(record["end"], "stop");
        assert_eq!(record["samples"], 10_000);
        assert_eq!(device.transmitted(clock.instant(2_000), 10_002), vec![Some([0.25, 0.0]); 10_000].into_iter().chain([Some([0.0, 0.0]), Some([0.0, 0.0])]).collect::<Vec<_>>());
    }

    #[test]
    fn ur_21_a_finite_burst_with_a_held_tail_keeps_its_front() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples: Vec<[f32; 2]> = (0..1_000).map(|n| [-0.5, n as f32 / 1_000.0]).collect();
        let (a_wave, a_bytes) = waveform_input("finite_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("finite_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 {
            assert!(tx.step(), "A did not hand over its three chunks");
        }
        assert!(tx.open_next().is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1), "A's last sample is held: {:?}", device.calls());

        // The held tail's front is `e + 4` = 8 004 (RM-15): 8 003 is refused at receipt, 8 004 is on time.
        book_burst(&mut control, &core, 2, clock, 8_003, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 3, clock, 8_004, &b_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0].payload["action"], 2);
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "8 004 is on time");
        assert_eq!(held_keys(&tx), vec![8_004]);
    }

    #[test]
    fn ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst() {
        let (a_wave, a_bytes) = waveform_input("one_sample_eob", &[[0.25, 0.0]]);
        let (b_wave, b_bytes) = waveform_input("one_sample_successor", &[[0.75, -0.25]]);
        for (target, refused) in [(2_004, true), (2_018, true), (2_019, false)] {
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(tx.step(), "A's one-sample device burst was not ended");
            assert_eq!(tx.chain.front(), Some((clock.domain, 2_019)), "A's padded device burst ends at 2 004; its safe sample is the 15-sample gap later");
            assert_eq!(sends(&device).first(), Some(&(4, true, true)));

            book_burst(&mut control, &core, 2, clock, target, &b_wave, false, LatePolicy::SendAsapAndFlag);
            drain_tx(&mut tx);
            let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "B at {target} is on time at receipt");
            if refused {
                let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
                assert_eq!(rejected.len(), 1, "B at {target}: {rejected:?}");
                assert!(rejected[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "B at {target}");
            } else {
                assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED), "B at {target} is accepted");
                assert!(tx.step(), "B was not dispatched");
                assert_eq!(sends(&device).last(), Some(&(4, true, true)));
                assert_eq!(device.transmitted(clock.instant(2_019), 1), vec![Some([0.75, -0.25])], "B's actual start");
            }
        }
    }

    #[test]
    fn ur_21_a_post_eob_restart_uses_the_safe_sample() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples = vec![[0.75, -0.25]];
        let (a_wave, a_bytes) = waveform_input("eob_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("eob_b", &b_samples);
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let device = Arc::new(FakeDevice::new(config));
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
        drain_tx(&mut tx);
        for _ in 0..3 {
            assert!(tx.step());
        }
        assert!(tx.open_next().is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().is_some());
        device_at(&core, &time, &device, core.ticks(6_000_000));
        assert!(tx.step(), "the held last sample was not closed with EOB");
        assert_eq!(tx.chain.front(), Some((clock.domain, 8_004)));
        assert_eq!(sends(&device).last(), Some(&(2, false, true)));

        // The front after the close is 8 004 (the close's exclusive end 8 001, plus three): a booking at 8 000, received
        // after its join deadline, is refused; 8 004 is on time (RM-15, the M1 rule).
        book_burst(&mut control, &core, 2, clock, 8_000, &b_wave, false, LatePolicy::DropAndFlag);
        book_burst(&mut control, &core, 3, clock, 8_004, &b_wave, false, LatePolicy::SendAsapAndFlag);
        drain_tx(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let rejected: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert_eq!(rejected[0].payload["action"], 2);
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR), "8 004 is on time");
        assert!(tx.step(), "the safe sample was not dispatched after A's EOB");
        assert_eq!(sends(&device).last(), Some(&(4, true, true)), "a timed SOB at the safe sample");
        assert_eq!(device.transmitted(clock.instant(8_004), 1), vec![Some([0.75, -0.25])]);
        device_at(&core, &time, &device, clock.instant(8_006));
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow" && row["code"] != "TimeError"));
    }

    #[test]
    fn ur_21_a_burst_inside_the_in_flight_window_is_refused() {
        // RM-15 at the receipt site: a booking inside the span of an accepted finite burst is refused, whether its
        // receipt is on time or late, for both late policies (the receipt rule decides, not the hand-over).
        // On the controlled clock, A is booked 9 ms ahead and B 7 ms ahead of the device's time.
        let a_samples: Vec<[f32; 2]> = vec![[0.25, 0.0]; 30_000];
        let b_samples: Vec<[f32; 2]> = vec![[-0.5, 0.25]; 1_000];
        let bytes = |samples: &[[f32; 2]]| {
            let mut out = Vec::with_capacity(samples.len() * 8);
            for sample in samples {
                out.extend_from_slice(&sample[0].to_le_bytes());
                out.extend_from_slice(&sample[1].to_le_bytes());
            }
            out
        };
        let (a_bytes, b_bytes) = (bytes(&a_samples), bytes(&b_samples));
        let (a_hash, b_hash) = (ContentHash::of_bytes(&a_bytes), ContentHash::of_bytes(&b_bytes));
        let input = |name: &str, bytes: &[u8]| {
            ezsdr_kernel::manifest::ingest_input(
                ezsdr_kernel::spec::Ident::parse(name).unwrap(),
                ezsdr_kernel::spec::Namespace::parse("ezsdr.input").unwrap(),
                format!("mem:{}", ContentHash::of_bytes(bytes)),
                bytes,
            )
        };
        let (a_wave, b_wave) = (input("waveform", &a_bytes), input("waveform2", &b_bytes));
        for policy in [LatePolicy::SendAsapAndFlag, LatePolicy::DropAndFlag] {
            let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(
                device.clone(),
                Vec::new(),
                2_000,
                BTreeMap::from([(a_hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]),
            );
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, _handed) = booking_control(&core);
            let clock = tx_clock(&core);
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let target = super::super::test_support::target_of(&core.tx_id);
            let mut book = |id: u64, k: i64, repeat: bool, wave: &ezsdr_kernel::manifest::ArtifactRef| {
                control.book(Dispatched::new(
                    ActionId(id),
                    Action::TxBurst {
                        target: target.clone(),
                        waveform: wave.clone(),
                        repeat,
                        at: AbsoluteDeadline::new(TimePoint::new(clock.domain, k)),
                        requested_at: None,
                        late_policy: policy,
                        metadata: BTreeMap::new(),
                    },
                    Some(core.tx_id.clone()),
                ));
            };
            let none = |events: &Arc<EventCollector>| events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            // A at 29 ms booked at 20 ms: 9 ms ahead, so the booking is on time.
            device_at(&core, &time, &device, core.ticks(20_000_000));
            book(1, 29_000, false, &a_wave);
            let a = take_booking(&core).expect("A booked");
            assert_eq!(a.k, 29_000, "{policy:?}: the booking moved A, so the receipt was late");
            assert!(none(&events).is_empty(), "{policy:?}: the booking emitted a verdict");
            give(&mut tx, a);
            // The 1 000-sample SOB is followed by two 2 500-sample sends; the next full chunk
            // does not fit in the 37 ms edge at 27 ms (UR-23).
            for at in [20_i64, 23, 25] {
                device_at(&core, &time, &device, core.ticks(at * 1_000_000));
                assert!(tx.step(), "{policy:?}: no hand-over at {at} ms");
            }
            device_at(&core, &time, &device, core.ticks(27_000_000));
            assert!(!tx.step(), "{policy:?}: the 2 000 samples before the 37 ms window edge were handed in a short buffer");
            assert_eq!(tx.open_next().expect("A is open"), 35_000, "{policy:?}: A's first unhanded sample");
            // B, 7 ms ahead at 27 ms, starts inside A's span (A runs to 59 002): it is refused at receipt, on time or not,
            // for both policies; no move or drop verdict is made, and A keeps its cursor.
            let calls = device.calls().len();
            book(2, 34_000, true, &b_wave);
            let refused: Vec<_> = none(&events).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
            assert_eq!(refused.len(), 1, "{policy:?}: {refused:?}");
            assert_eq!(refused[0].payload["action"], 2, "{policy:?}");
            assert!(refused[0].payload["reason"].as_str().unwrap().contains("overlaps an accepted booking"), "{policy:?}");
            assert!(take_booking(&core).is_none(), "{policy:?}: the refused B was booked on the list");
            assert!(tx.open_booking(|held| held.k) == Some(29_000), "{policy:?}: A is not still open");
            assert_eq!(tx.open_next().unwrap(), 35_000, "{policy:?}: A's first unhanded sample");
            assert_eq!(device.calls().len(), calls, "{policy:?}: the refusal called the device");
        }
    }

    #[test]
    fn ur_21_a_late_joined_burst_cannot_move_and_is_refused() {
        // D24: B (10 samples) is booked with A, as A's joined continuation at A's end, 1 000 after A's start. A is abandoned mid-send
        // (a failed send), so B is dispatched as a non-head joined piece, and the host is late for it: `send_asap` would move it, but the
        // record cannot move a joined piece, so B is refused at dispatch (`TIME_ERROR { refused }`, `COMMAND_REJECTED`) and nothing of B is sent.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig_faults(Some(2_032_000), 2_000, 1e6, 200, vec![FakeFault::FailSend(0)]);
        let action = |burst: &mut Held, id: u64| burst.settings = Arc::new(super::ColdConfig::of(BTreeMap::new(), ActionId(id)));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 1_000, false, LatePolicy::DropAndFlag));
        let mut b = txp_burst(&clock, 21_000, 10, false, LatePolicy::SendAsapAndFlag);
        action(&mut b, 2);
        txp_book(&mut tx, &core, clock, b);
        device_at(&core, &time, &device, txp_ticks(18_000));
        txp_drain(&mut tx);
        assert!(tx.open_next().is_none() && held_keys(&tx) == [21_000], "A was abandoned by the failed send; B waits: {:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(19_001));
        txp_drain(&mut tx);
        let emitted = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let errors = emitted.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
        assert_eq!(errors.len(), 1, "only B is late: {emitted:?}");
        assert_eq!(errors[0].payload["outcome"], "refused");
        let rejected: Vec<_> = emitted.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && e.payload["action"] == 2).collect();
        assert_eq!(rejected.len(), 1, "{emitted:?}");
        assert_eq!(rejected[0].payload["reason"], "UR-21: the moved start overlaps an accepted booking", "refused at dispatch, not on the join");
        assert!(held_keys(&tx).is_empty(), "B is not held: {:?}", held_keys(&tx));
        assert_eq!(device.transmitted(clock.instant(21_000), 10), vec![None; 10], "nothing of B is sent: {:?}", device.calls());
    }
    #[test]
    fn ur_29_eob_device_errors_are_escalated() {
        for lost in [true, false] {
            for tail in [None, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 })] {
                let (core, device, _, events) = super::super::test_support::rig();
                device.tx_open(0, &[0]).unwrap();
                let clock = core.register(Dir::Tx, 200, 0).unwrap();
                let (_to_tx, cmds) = std::sync::mpsc::channel();
                let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
                *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost, message: "controlled EOB failure".to_owned() });
                tx.close_device_burst(tail, 100, clock);
                assert_eq!(core.is_lost(), lost, "EOB failure must reach UR-29");
                let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
                let expected = if lost { EventKind::DEVICE_LOST } else { ezsdr_radio::kinds::COMMAND_REJECTED };
                assert_eq!(emitted.iter().filter(|event| event.kind.as_str() == expected).count(), 1);
                if lost {
                    tx.close_device_burst(None, 100, clock);
                    assert!(events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
                    assert_eq!(device.calls().iter().filter(|call| call.as_str() == "mark_lost").count(), 1);
                }
            }
        }
    }

    #[test]
    fn ur_29_non_lost_payload_eob_error_after_concurrent_loss_does_not_reopen() {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _, mut tx, clock, _events) = recovery_rig(device.clone());
        tx.chain.advance_front(clock.domain, 123);
        let front = tx.chain.front();
        let target = clock.at_or_after(core.now() + core.ticks(60_000_000));
        give(&mut tx, held(&clock, target, 1, LatePolicy::DropAndFlag));
        *device.eob_error.lock().unwrap() = Some(crate::DeviceError::failed("controlled non-lost payload EOB failure"));
        let (entered, release) = device.gate_next_payload_eob_send();
        let (finished, result) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("tx-eob-loss-race", move || {
                let worked = tx.step();
                let _ = finished.send((tx, worked));
            })
            .unwrap();

        assert!(entered.recv_timeout(Wall::from_secs(10)).is_ok(), "the payload-bearing EOB send did not reach its gate");
        core.device_lost("test loss while the payload EOB send is outstanding");
        let calls_after_loss = device.calls();
        release.send(()).unwrap();
        owner.join().unwrap();
        let (mut tx, worked) = result.recv_timeout(Wall::from_secs(10)).expect("the TX step returned");

        assert!(worked);
        assert!(core.is_lost());
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none(), "the failed burst is closed");
        assert!(!tx.chain.needs_reopen(), "published loss must not reintroduce ReopenNeeded");
        assert_eq!(tx.chain.recovery(), Recovery::Clean);
        assert_eq!(tx.chain.front(), front, "the failed EOB leaves the safe front unchanged");
        assert_eq!(device.calls(), calls_after_loss, "the outstanding send made no device call after loss publication");
        assert!(!tx.step(), "the lost owner does no further work");
        assert_eq!(device.calls(), calls_after_loss, "no later device operation follows loss");
    }

    #[test]
    fn ur_29_eob_loss_at_the_clock_end_stops_device_calls() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(1));
        tx.chain.open_device_burst(0, 100, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost: true, message: "controlled EOB loss at the cut".to_owned() });
        let before = device.calls().len();
        tx.end_clock();
        tx.reports(std::time::Duration::ZERO);
        assert!(core.is_lost());
        assert_eq!(&device.calls()[before..], &["mark_lost"]);
        assert!(tx.clock.is_none());
    }

    #[test]
    fn ur_29_eob_loss_from_report_prevents_next_burst_send() {
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.chain.open_device_burst(0, 100, None);
        hold(&tx, held(&clock, 3000, 10, LatePolicy::DropAndFlag));
        *device.eob_error.lock().unwrap() = Some(crate::device::DeviceError { lost: true, message: "controlled EOB loss after underflow".to_owned() });
        let before = device.calls().len();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: None, channel: 0 });
        assert!(core.is_lost());
        assert!(!tx.step());
        assert!(held_keys(&tx).contains(&3000));
        assert_eq!(&device.calls()[before..], &["mark_lost"]);
    }

    #[test]
    fn ur_28_per_channel_ack_preserves_the_next_burst() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(2));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);
        for channel in [0, 1] {
            tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(a.ticks_in(clock.domain).unwrap()) + 1), channel });
        }
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(b.ticks_in(clock.domain).unwrap()) + 200), channel: 0 });
        let emitted = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(emitted.iter().any(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR), "burst B TIME_ERROR must survive both ACKs of A");
    }

    #[test]
    fn ur_28_ambiguous_time_errors_leave_each_channel_pending() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(2));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(25_000)), channel: 0 });

        assert!(time_errors(&events).is_empty(), "a current-time tick matching both bursts is ambiguous");
        let channels = std::collections::BTreeSet::from([0, 1]);
        assert_eq!(tx.unacked.iter().map(|pending| (pending.target, pending.channels.clone())).collect::<Vec<_>>(), [(a, channels.clone()), (b, channels)]);
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 1, "the raw radio-current-time report is retained");
    }

    #[test]
    fn ur_28_late_burst_error_and_later_ack_use_report_ticks() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let late = TimePoint::new(clock.domain, 10_000);
        let future = TimePoint::new(clock.domain, 1_000_000);
        tx.remember_burst(late);
        tx.remember_burst(future);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(15_000)), channel: 0 });
        let errors = time_errors(&events);
        assert_eq!(errors.len(), 1, "the current-time report uniquely names the late burst");
        assert_eq!(errors[0].payload["target"], serde_json::to_value(late).unwrap());
        assert_eq!(tx.unacked.len(), 2, "a TimeError does not retire its burst");

        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(1_000_100)), channel: 0 });
        assert!(tx.unacked.is_empty(), "the later burst's ack also retires the earlier unacknowledged burst");
    }

    #[test]
    fn ur_28_a_ticked_ack_does_not_retire_a_future_burst() {
        let (core, _, _, _) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(15_000)), channel: 0 });

        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [b], "an ack retires every earlier target, but not one starting after its tick");
    }

    #[test]
    fn ur_28_a_tickless_burst_ack_retires_only_the_first_pending_target() {
        let (core, _, _, _) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [b], "a tickless ack uses the first-pending rule");
        tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
        assert!(tx.unacked.is_empty());
    }

    #[test]
    fn ur_28_tickless_time_errors_keep_the_first_pending_target() {
        use crate::device::{TxCode, TxReport};
        for channels in [1, 2] {
            let (core, _, _, events) = super::super::test_support::rig();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels as u32));
            let a = TimePoint::new(clock.domain, 10_000);
            let b = TimePoint::new(clock.domain, 20_000);
            tx.remember_burst(a);
            tx.remember_burst(b);
            tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 9 });
            assert_eq!(tx.unacked.len(), 2, "invalid channel must not consume a burst");
            if channels == 2 {
                // Channel 0 may finish B while channel 1 has not finished A.
                for _ in 0..2 {
                    tx.report(TxReport { code: TxCode::BurstAck, tick: None, channel: 0 });
                }
                assert_eq!(tx.unacked.len(), 2);
            }
            let channel = channels - 1;
            for _ in 0..2 {
                tx.report(TxReport { code: TxCode::TimeError, tick: None, channel });
            }
            let errors = events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
            assert_eq!(errors.len(), 1, "repeated reports are deduplicated for the first pending target: {errors:?}");
            assert_eq!(errors[0].payload["target"], serde_json::to_value(a).unwrap());
            assert_eq!(tx.unacked.len(), 2, "TimeErrors do not retire their report correlations");
            assert!(tx.unacked.iter().all(|pending| pending.channels.contains(&channel)));
            assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 2, "both raw reports remain recorded");
        }
    }

    #[test]
    fn ur_28_time_error_after_abandoned_burst_does_not_claim_the_next_burst() {
        let (core, _, _, events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let a = TimePoint::new(clock.domain, 10_000);
        let b = TimePoint::new(clock.domain, 20_000);
        tx.remember_burst(a);
        tx.poke_open(held(&clock, 10_000, 1, LatePolicy::DropAndFlag), 10_001, false);
        assert!(tx.abandon(false, tx.cursor(), false), "A was not abandoned");
        assert!(tx.open_next().is_none());
        tx.remember_burst(b);

        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(25_000)), channel: 0 });
        assert!(time_errors(&events).is_empty(), "A's late report was assigned to B");
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [a, b], "the ambiguous report claimed neither correlation");
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 1, "the raw radio-current-time report is retained");
    }

    #[test]
    fn ur_28_streamer_generation_clears_late_report_deduplication() {
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        let target = TimePoint::new(clock.domain, 100);
        tx.remember_burst(target);
        tx.report(TxReport { code: TxCode::TimeError, tick: Some(clock.instant(101)), channel: 0 });
        assert!(tx.late_reported.contains(&target));
        assert_eq!(time_errors(&events).len(), 1, "the pending target was reported late");

        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(1)));
        tx.sync_streamer();
        assert!(tx.unacked.is_empty(), "the previous generation's correlation remains");
        assert!(!tx.late_reported.contains(&target), "the previous generation's deduplication entry remains");
    }

    /// The TIME_ERROR events the rig has emitted since the last call.
    fn time_errors(events: &EventCollector) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect()
    }

    #[test]
    fn ur_28_a_streamer_the_record_holds_keeps_its_correlations() {
        // UR-28, UR-36: the correlations of the bursts in flight belong to the streamer of their
        // generation. An open of the stream — the owner's reopen — replaces the queue and drops
        // them; while the record holds the same streamer, they stand.
        let (core, device, _, _) = super::super::test_support::rig();
        device.tx_open(0, &[0]).unwrap();
        let key = stream_key(Dir::Tx, 0);
        lock(&core.streams).record_open(key, Some(Channels::first(1)));
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.remember_burst(TimePoint::new(clock.domain, 100));
        lock(&core.streams).record_open(key, Some(Channels::first(1)));
        tx.sync_streamer();
        assert!(tx.unacked.is_empty(), "a new streamer's queue starts empty");
        tx.remember_burst(TimePoint::new(clock.domain, 100));
        tx.sync_streamer();
        assert_eq!(tx.unacked.len(), 1, "the record holds the same streamer: {:?}", device.calls());
    }

    #[test]
    fn ur_28_delayed_time_error_uses_its_original_clock() {
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[0, 1]).unwrap();
        lock(&core.streams).record_open(stream_key(Dir::Tx, 0), Some(Channels::first(2)));
        let old = core.register(Dir::Tx, 200, 0).unwrap();
        let target = TimePoint::new(old.domain, 10_000);
        core.clocks.end(old.domain, core.at(20_000_000)).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(old), Channels::first(2));
        tx.remember_burst(target);
        tx.end_clock();
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(old.instant(target.ticks_in(old.domain).unwrap()) + 200_000), channel: 0 });
        // This channel's report has no tick; it is still the same Kernel burst, so it is deduplicated.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: None, channel: 1 });
        let errors: Vec<_> =
            events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "only the uniquely attributable old burst reports: {errors:?}");
        assert_eq!(errors[0].payload["late_by_ns"], 1_000_000);
        assert_eq!(errors[0].payload["target"], serde_json::to_value(target).unwrap());
        assert_eq!(tx.unacked.len(), 1, "TimeErrors leave correlations pending");
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "TimeError").count(), 2, "ambiguous reports remain visible in the raw log");
    }

    #[test]
    fn ur_25_the_line_lock_is_free_inside_every_transmit_device_call() {
        // UR-25 (design 33 D15): uhd-tx holds no line lock across its device calls — a
        // continuation's held sample, its own payload, the end of burst that closes the
        // device burst and the asynchronous report read — and the spy proves the sequence passed each one.
        let spy = Arc::new(crate::provider::test_support::LockSpy::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        spy.tx_open(0, &[0]).unwrap();
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(c0), Channels::first(1));
        // A burst continuing the device burst of the one before: its held sample goes first,
        // then its own payload; its last sample is held for the end of burst.
        tx.chain.open_device_burst(0, 20_020, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));
        give(&mut tx, held(&c0, 20_020, 10, LatePolicy::DropAndFlag));
        time.advance_to(core.at(3_600_000)).unwrap();
        assert!(tx.step());
        // The clock's end: ends the open device burst (with its held sample). The reports are
        // read after it, so that the held sample's end of burst is the clock's.
        give(&mut tx, held(&c0, 20_100, 10, LatePolicy::DropAndFlag));
        tx.end_clock();
        tx.reports(std::time::Duration::ZERO);
        assert_eq!(spy.seen("tx_open"), 1, "the test's own open");
        assert!(spy.seen("tx_send") >= 3, "the tail, the payload and the end of burst: {}", spy.seen("tx_send"));
        assert!(spy.seen("tx_async") > 0, "the spy never saw tx_async");
    }

    #[test]
    fn ur_28_a_report_is_correlated_by_its_position_in_the_streamer() {
        // UR-28 (Sol 1): `TxReport.channel` is the position in the streamer, not the device
        // channel (UHD 4.10's `rfnoc_tx_streamer.cpp:152-159,312-319`; `FakeDevice`,
        // `device.rs:1515-1516,1555-1556`). Stream 0 opened for `{1}` — device channel 1 —
        // is position 0 there: its burst's ACK retires its pending report, and a later
        // TIME_ERROR names that burst rather than the first one.
        let (core, device, _, events) = super::super::test_support::rig();
        device.tx_open(0, &[1]).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(0).with(1));
        let first = TimePoint::new(clock.domain, 20_000);
        let second = TimePoint::new(clock.domain, 40_000);
        // What each burst's start-of-burst books.
        tx.remember_burst(first);
        tx.remember_burst(second);
        assert_eq!(tx.unacked.len(), 2);
        assert_eq!(tx.unacked[0].channels.iter().copied().collect::<Vec<_>>(), [0], "the position, not the device channel");
        // The first burst's ACK retires that position; with only B pending, its TIME_ERROR is
        // attributable to B. The TIME_ERROR itself never consumes the BurstAck correlation.
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(30_000)), channel: 0 });
        assert_eq!(tx.unacked.len(), 1, "the ACK retired no report");
        assert_eq!(tx.unacked[0].target, second);
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(41_000)), channel: 0 });
        assert_eq!(tx.unacked.len(), 1, "TIME_ERROR is not an acknowledgement");
        assert_eq!(tx.unacked[0].target, second);
        let late: Vec<_> = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{late:?}");
        assert_eq!(late[0].payload["target"], serde_json::to_value(second).unwrap(), "the report names the second burst");
        assert_eq!(late[0].payload["outcome"], "late_at_device");
        tx.report(crate::device::TxReport { code: crate::device::TxCode::TimeError, tick: Some(clock.instant(41_100)), channel: 0 });
        assert!(
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR),
            "duplicate packet errors produce one burst event"
        );
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: None, channel: 0 });
        assert!(tx.unacked.is_empty(), "the BurstAck retires B");
    }

    // ------------------------------------------------------------ 1b-4d-1b: ownership, reading, stop

    use crate::ClockMode;
    use crate::provider::test_support::LockSpy;

    /// The manual-clock `LockSpy` rig the ownership series runs on (UR-25).
    fn spy_rig() -> (Arc<Core>, Arc<LockSpy>) {
        let spy = Arc::new(LockSpy::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, _) = crate::provider::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        (core, spy)
    }

    /// The transmit record, as the ownership tests read it.
    fn tx_record(core: &Core) -> (bool, bool, Option<u64>) {
        let streams = lock(&core.streams);
        let k = stream_key(Dir::Tx, 0);
        (streams.holder(k) != Holder::Free, streams.reading(k), streams.stream(k).configured)
    }

    /// A layout with two transmit streams and no receive channel (RM-29).
    fn two_tx_description() -> ezsdr_radio::channels::Description {
        use ezsdr_kernel::spec::{Ident, Scalar, Value};
        let selector = std::collections::BTreeMap::from([
            (Ident::parse("rx").unwrap(), Value::List(Vec::new())),
            (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        ]);
        let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
        ezsdr_radio::channels::Description::new(&layout, 2_000, &crate::profile::terms())
    }

    #[test]
    fn ur_28_a_later_stream_s_reopen_discards_only_its_correlations() {
        // UR-28, UR-36 (T3): the streamer's generation is per stream; an open of stream 1
        // discards stream 1's pending correlations alone.
        let inner = Arc::new(crate::device::FakeDevice::new(crate::device::FakeConfig::default()));
        let feed = Arc::new(ReportFeed::new(inner.clone()));
        let (core, _, _) = super::super::test_support::rig_on_description(feed.clone(), Vec::new(), BTreeMap::new(), two_tx_description());
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let (_to1, cmds1) = std::sync::mpsc::channel();
        let mut s0 = Tx::new(core.clone(), 0, cmds0, None, Channels::first(1));
        let mut s1 = Tx::new(core.clone(), 1, cmds1, None, Channels::first(1));
        s0.remember_burst(TimePoint::new(core.root, 0));
        s1.remember_burst(TimePoint::new(core.root, 0));
        core.open_streamer(Dir::Tx, 1, Channels::first(1)).unwrap();
        feed.give(TxReport { code: TxCode::Other(0x40), tick: Some(0), channel: 0 });
        s1.reports(std::time::Duration::ZERO);
        assert_eq!(feed.reads(0), 0, "stream 0's streamer was not read");
        assert!(feed.reads(1) >= 1, "stream 1's streamer was read");
        assert_eq!(lock(&core.rec).device_async.last().map(|row| row["stream"].as_u64()), Some(Some(1)), "the row names the report's stream");
        assert!(s1.unacked.is_empty(), "stream 1's old correlations are dropped");
        assert_eq!(s0.unacked.len(), 1, "stream 0's correlation stands");
        // The same row from stream 0: its `stream` is the report's, not a constant 1.
        feed.give(TxReport { code: TxCode::Other(0x40), tick: Some(0), channel: 0 });
        s0.reports(std::time::Duration::ZERO);
        assert_eq!(lock(&core.rec).device_async.last().map(|row| row["stream"].as_u64()), Some(Some(0)), "stream 0's row names its stream");
        assert_eq!(
            inner.calls().iter().filter(|call| call.starts_with("tx_send")).map(String::as_str).collect::<Vec<_>>(),
            ["tx_send 1 n=4 at=- sob=true eob=true"],
            "only the open-time flush was sent: {:?}",
            inner.calls()
        );
    }

    #[test]
    fn ur_25_a_detached_control_does_not_hold_the_shutdown_s_read() {
        // UR-25, UR-16: a uhd-control the join bound left detached may never clear its mark;
        // the shutdown's ACK window still reads the reports.
        let (core, spy) = spy_rig();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        let claim = lock(&core.streams).claim_owner(stream_key(Dir::Tx, 0)).expect("the stream is free");
        std::mem::forget(claim);
        tx.shutdown();
        assert!(spy.seen("tx_async") >= 1, "the shutdown's read waited for a detached control");
        assert!(!tx_record(&core).1, "the shutdown's read was not taken back");
    }

    #[test]
    fn ur_26_stop_cancels_the_held_burst_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        hold(&tx, held(&current, 20, 1, LatePolicy::DropAndFlag));

        halt(&mut tx);

        assert!(held_keys(&tx).is_empty());
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Stop"), "{rejected:?}");
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
        assert!(!events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    // ------------------------------------------ bursts continued across a burst's end (UR-23)

    /// A-0: `manual_rig` with the device's clock held still (M3), A's burst at 40 000 with
    /// 30 000 samples driven in 13 steps of up to 2 500 samples, so its last sample is held
    /// back and `continues_at` is 70 000 (design note 37, T1–T5).
    fn at_a_held_tail(faults: Vec<crate::device::FakeFault>) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, crate::provider::test_support::Frozen) {
        let config = FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        give(&mut tx, held(&clock, 40_000, 30_000, LatePolicy::DropAndFlag));
        for step in 0..12 {
            let at = 31_000_000 + step * 2_500_000;
            device_at(&core, &time, &device, core.ticks(at));
            assert!(tx.step(), "A is not dispatched at {at} ns");
        }
        device_at(&core, &time, &device, core.ticks(61_000_000));
        assert!(tx.step(), "the final pre-window sample is not handed");
        assert!(
            tx.open_next().is_none() && tx.chain.next() == Some(70_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1),
            "A's last sample is not the held tail: {:?}",
            device.calls()
        );
        (core, device, time, tx, frozen)
    }

    /// A burst that continues A's device burst at `k`: `len` samples of a waveform unlike
    /// `held`'s uniform one, so that a continuation's order shows in the values.
    fn continuation(clock: &Clock, k: i64, len: usize) -> Held {
        let samples: Vec<[f32; 2]> = (0..len).map(|i| [-0.25 - i as f32 / 4_000.0, 0.125]).collect();
        let mut burst = held(clock, k, len, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![samples]);
        burst
    }

    /// Each `tx_send` the fake saw: `n`, start-of-burst, end-of-burst.
    fn sends(device: &FakeDevice) -> Vec<(usize, bool, bool)> {
        device
            .calls()
            .iter()
            .filter(|call| call.starts_with("tx_send "))
            .map(|call| {
                let field = |key: &str| call.split_whitespace().find_map(|word| word.strip_prefix(key));
                (field("n=").unwrap().parse().unwrap(), field("sob=") == Some("true"), field("eob=") == Some("true"))
            })
            .collect()
    }

    /// A one-shot burst of `samples` on each of `channels` buffers, booked at sample `k` of
    /// `clock`.
    fn one_shot(clock: &super::Clock, k: i64, channels: usize, samples: Vec<[f32; 2]>) -> Held {
        let waveform_len = samples.len();
        Held {
            k,
            domain: clock.domain,
            samples: Arc::new(vec![samples; channels.max(1)]),
            waveform_len,
            repeat: false,
            switch: false,
            untimed: false,
            open: BurstOpen { waveform_len: None, late: None, requested_target: None },
            policy: LatePolicy::DropAndFlag,
            target: TimePoint::new(clock.domain, k),
            settings: Arc::new(super::ColdConfig::new(BTreeMap::new())),
        }
    }

    #[test]
    fn ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it() {
        // T1: B booked at A's end after A's last buffer went out (inside the in-flight
        // window) is played, continuing A's device burst: A's held last sample ahead of B's
        // first, one device burst of 31 000 samples, no end-of-burst between.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_000, 1_000);
        let second = b.samples[0].clone();
        give(&mut tx, b);
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "A's held sample is not taken ahead of B");
        device_at(&core, &time, &device, core.ticks(69_000_000));
        assert!(tx.step(), "B's held sample is not ended");
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).chain(second.iter().copied().map(Some)).collect();
        whole.push(Some([0.0, 0.0]));
        assert_eq!(device.transmitted(clock.instant(40_000), 31_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert_eq!(sends.iter().map(|(n, _, _)| n).sum::<usize>(), 31_001, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        assert!(
            lock(&core.rec).rejected.iter().all(|row| !row["reason"].as_str().unwrap_or("").starts_with("UR-21") && !row["reason"].as_str().unwrap_or("").starts_with("UR-22")),
            "{:?}",
            lock(&core.rec).rejected
        );
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it() {
        // T2: a `Stop` while the device burst waits for a continuation ends it with one
        // end-of-burst, A's held last sample, before it runs dry.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        device_at(&core, &time, &device, core.ticks(63_000_000));
        halt(&mut tx);
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        whole.push(Some([0.0, 0.0]));
        assert_eq!(device.transmitted(clock.instant(40_000), 30_001), whole, "{:?}", device.calls());
        let sends = sends(&device);
        // A's held last payload sample leads the two-sample close; its padding zero is at 70 000.
        assert_eq!(sends.last(), Some(&(2, false, true)), "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_stop_ends_the_burst_within_the_in_flight_window() {
        // UR-23, RM-16: payload stops at the first SampleClock sample at or after `s + W`;
        // the two-zero EOB is separate. The fake's Session test reads the receipt instant.
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        give(&mut tx, held(&clock, 20_000, 30_000, LatePolicy::DropAndFlag));
        // First hand-over at 12 ms: A's start 9 ms ahead, one 2 000-sample buffer sent, its
        // end exactly one window after this instant.
        device_at(&core, &time, &device, core.ticks(12_000_000));
        for _ in 0..8 {
            tx.step();
        }
        assert_eq!(tx.open_next().expect("A is open"), 22_000, "the hand-over went past the in-flight window: {:?}", device.calls());
        halt(&mut tx);
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let window = core.ticks(core.description.timing.tx_in_flight_ns);
        let stop = core.now();
        let record = lock(&core.rec).bursts.last().cloned().expect("A's record");
        assert_eq!(record["end"], "stop", "{record}");
        let target = record["target"]["ticks"].as_i64().expect("the record's target");
        let end = clock.instant(target + record["samples"].as_i64().expect("the record's samples"));
        assert!(end <= stop + window, "the record ends {} ms after the stop", (end - stop) / core.ticks(1_000_000));
        // The device's own end is the record's plus the two zero samples in the EOB; its ACK is one tick later.
        device_at(&core, &time, &device, end + 2 * clock.n + 1);
        tx.reports(Wall::ZERO);
        let ack = lock(&core.rec).device_async.iter().find(|row| row["code"] == "BurstAck").expect("the end-of-burst's acknowledgement")["tick"].as_i64().unwrap();
        assert_eq!(ack, end + 2 * clock.n + 1, "{:?}", lock(&core.rec).device_async);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played() {
        // T2: A closes with its held tail and one zero; B starts at the close's exclusive end
        // plus three samples, so the device sees A, zero, a gap of three, then B.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_004, 1_000);
        let second = b.samples[0].clone();
        give(&mut tx, b);
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "A's device burst is not ended at c");
        assert_eq!(tx.chain.front(), Some((clock.domain, 70_004)), "the safe sample follows the two-sample close");
        let before_unsafe = sends(&device);
        give(&mut tx, continuation(&clock, 70_003, 1_000));
        assert!(tx.step(), "the start inside A's EOB was not judged");
        assert_eq!(sends(&device), before_unsafe, "the sample before the safe front was not handed over");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"] == "UR-21: the late policy dropped the burst"));
        assert!(tx.step(), "B is not dispatched at the safe sample");
        device_at(&core, &time, &device, core.ticks(69_000_000) + 40_000);
        assert!(tx.step(), "B's held sample is not ended");
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && tx.chain.tail().is_none());
        let mut whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        whole.extend([Some([0.0, 0.0]), None, None, None]);
        whole.extend(second.iter().copied().map(Some));
        assert_eq!(device.transmitted(clock.instant(40_000), 31_004), whole, "{:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, sob, _)| *sob).count(), 2, "{sends:?}");
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 2, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "TimeError" && row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned() {
        // T4: A's held sample's send ahead of B takes nothing, or fails: B is abandoned, and
        // the device burst ends with two zero samples after A's 29 999 accepted samples.
        for (fault, reason) in [(crate::device::FakeFault::StalledSend(13), "UR-22: a send did not complete"), (crate::device::FakeFault::FailSend(13), "fake: the send failed")] {
            let what = format!("{fault:?}");
            let (core, device, time, mut tx, _frozen) = at_a_held_tail(vec![fault]);
            let clock = tx.clock.expect("a transmit clock");
            give(&mut tx, continuation(&clock, 70_000, 1_000));
            device_at(&core, &time, &device, core.ticks(62_000_000));
            assert!(tx.step(), "{what}: the held sample's send was not entered");
            let sent = device.transmitted(clock.instant(40_000), 30_001);
            let whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
            assert_eq!(sent[..29_999], whole[..29_999], "{what}: {:?}", device.calls());
            assert_eq!(sent[29_999], Some([0.0, 0.0]), "{what}: B played in A's last sample's place: {:?}", device.calls());
            assert_eq!(sent[30_000], Some([0.0, 0.0]), "{what}: second EOB sample {:?}", device.calls());
            let sends = sends(&device);
            assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{what}: {sends:?}");
            assert_eq!(device.unended_bursts(), 0, "{what}: {sends:?}");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains(reason)), "{what}: {:?} {sends:?}", lock(&core.rec).rejected);
            assert!(device.calls().iter().any(|call| call.contains("stalled") || call.contains("failed")), "{what}: the case was not reached: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst() {
        // T5: B continues A's device burst; B's first payload send takes nothing. The device
        // burst is still open and ends with a two-zero EOB after A's last sample.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(vec![crate::device::FakeFault::StalledSend(14)]);
        let clock = tx.clock.expect("a transmit clock");
        give(&mut tx, continuation(&clock, 70_000, 1_000));
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step(), "B's first send was not entered");
        assert!(device.calls().iter().any(|call| call.contains("stalled")), "the case was not reached: {:?}", device.calls());
        let sent = device.transmitted(clock.instant(40_000), 30_002);
        let whole: Vec<Option<[f32; 2]>> = (0..30_000).map(|_| Some([0.25, 0.0])).collect();
        assert_eq!(sent[..30_000], whole[..], "{:?}", device.calls());
        assert_eq!(sent[30_000], Some([0.0, 0.0]), "{:?}", device.calls());
        assert_eq!(sent[30_001], Some([0.0, 0.0]), "the second EOB sample {:?}", device.calls());
        let sends = sends(&device);
        assert_eq!(sends.iter().filter(|(_, _, eob)| *eob).count(), 1, "{sends:?}");
        assert_eq!(device.unended_bursts(), 0, "{sends:?}");
        tx.reports(Wall::ZERO);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", lock(&core.rec).device_async);
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("UR-22")), "{:?}", lock(&core.rec).rejected);
    }

    #[test]
    fn ur_23_bursts_go_out_in_target_order() {
        // RM-15: held bursts go out in target order, whatever order they were booked in. The
        // Spec sorts its entries by instant, so its two bursts arrive in target order; the
        // reverse booking is pinned here, on the controlled clock.
        let config = FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() };
        let (core, device, time, _) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        // 200 ms first, then 100 ms: the later booking has the nearer target.
        for k in [200_000, 100_000] {
            give(&mut tx, held(&clock, k, 2_000, LatePolicy::DropAndFlag));
        }
        device_at(&core, &time, &device, core.ticks(91_000_000));
        assert!(tx.step(), "the nearer burst is not dispatched");
        let sends = sends(&device);
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert!(sends[0].0 == 1_000 && sends[0].1 && !sends[0].2, "the nearer burst did not go first within the strict window: {sends:?}");
        assert!(device.calls().iter().any(|call| call.contains("at=20000000 sob=true")), "the 100 ms burst's timed start: {:?}", device.calls());
        assert!(held_keys(&tx).contains(&200_000), "the farther burst is still held");
    }

    #[test]
    fn ur_22_a_first_send_cut_short_keeps_the_reports_paired() {
        // Review Q, TG-Q1: A's acknowledgement stays paired ahead of B's late-at-device
        // report after a short send or an error that may have accepted samples.
        for (fault, cursor) in [(crate::device::FakeFault::ShortSend(0), 40_499), (crate::device::FakeFault::FailSend(0), 40_999)] {
            let what = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, faults: vec![fault], ..FakeConfig::default() };
            let (core, device, time, events) = crate::provider::test_support::rig_with(config, Vec::new());
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let a: Vec<[f32; 2]> = (0..1_000).map(|i| [0.25 + i as f32 / 4_000.0, -0.125]).collect();
            let mut burst = held(&clock, 40_000, 1_000, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![a.clone()]);
            give(&mut tx, burst);
            device_at(&core, &time, &device, core.ticks(31_000_000));
            assert!(tx.step(), "{what}: A's first send");
            assert!(tx.open_next().is_none(), "{what}: A was not abandoned: {:?}", device.calls());
            assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [TimePoint::new(clock.domain, 40_000)], "{what}: the attempted first burst stays correlated");
            let sent = device.transmitted(clock.instant(40_000), 501);
            assert_eq!(sent[..499], a[..499].iter().copied().map(Some).collect::<Vec<_>>(), "{what}: {:?}", device.calls());
            assert_eq!(sent[499], Some([0.0, 0.0]), "{what}: after what A took: {:?}", device.calls());
            assert_eq!(sent[500], Some([0.0, 0.0]), "{what}: the second closing sample {:?}", device.calls());
            // The accepted (or conservatively attempted) cursor plus the two zeros and the gap is the first safe start.
            let safe = cursor + 2 + GAP_1MSPS;
            give(&mut tx, continuation(&clock, safe, 1_000));
            // The window reaches 32 samples past B's first send (FS-3: a first send never goes short). B's front is now 15 samples past the zeros, so the window moves with it.
            device_at(&core, &time, &device, core.ticks(31_048_000));
            // The device gets past B's safe target while the owner's manual clock stays fixed.
            device.set_time(core.ticks(70_000_000));
            assert!(tx.step(), "{what}: B's timed send");
            device_at(&core, &time, &device, core.ticks(70_000_000));
            tx.reports(Wall::ZERO);
            assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [TimePoint::new(clock.domain, safe)], "{what}: A's close ack retires A, not B");
            let late: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert_eq!(late.len(), 1, "{what}: {:?}", lock(&core.rec).device_async);
            assert_eq!(late[0].payload["outcome"], "late_at_device", "{what}: {late:?}");
            assert_eq!(late[0].payload["target"]["ticks"], safe, "{what}: the report names B: {late:?}");
            assert!(
                lock(&core.rec)
                    .rejected
                    .iter()
                    .any(|row| row["reason"].as_str().unwrap_or("").contains("fake: the send failed") || row["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete")),
                "{what}: {:?}",
                lock(&core.rec).rejected
            );
        }
    }

    #[test]
    fn ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took() {
        // Review P, NB-1: a send that comes up short abandons the burst. The device burst is
        // closed with a two-zero EOB straight after the samples the
        // device took; the burst's held last sample is not sent there. The 6 000-sample case
        // cuts the final data buffer (the 4th device send); the last sample is held inside the
        // in-flight window (D3). A send that fails after taking half is ended the same way
        // (Review Q, NB-Q1). The steps are placed by hand on the manual clock, so the first
        // buffer's size does not depend on when a thread ran (design note 37).
        let cases = [
            (crate::device::FakeFault::ShortSend(0), 1_000usize),
            (crate::device::FakeFault::ShortSend(1), 3_000),
            (crate::device::FakeFault::ShortSend(3), 6_000),
            (crate::device::FakeFault::FailSend(0), 1_000),
        ];
        for (fault, len) in cases {
            let nth = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, faults: vec![fault], ..FakeConfig::default() };
            let (core, device, time, _events) = crate::provider::test_support::rig_with(config, Vec::new());
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let clock = core.register(Dir::Tx, 200, 0).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            std::mem::forget(_to_tx);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            let wave: Vec<[f32; 2]> = (0..len).map(|i| [0.25 + i as f32 / 40_000.0, -0.125]).collect();
            let mut burst = held(&clock, 40_000, len, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            give(&mut tx, burst);
            // The first buffer is 100 samples (the window reaches 100 samples past the start),
            // then a whole 2 000-sample buffer once the window takes it (UR-23), one per ten 200 us steps:
            // the 4th device send is the final data buffer of the 6 000-sample case, whose last sample is held.
            let mut at = 30_100_000;
            for _ in 0..80 {
                device_at(&core, &time, &device, core.ticks(at));
                tx.step();
                at += 200_000;
                if tx.open_next().is_none() {
                    break;
                }
            }
            let sends = sends(&device);
            assert!(tx.open_next().is_none(), "{nth}: the device burst was not ended: {:?}", device.calls());
            let rejected = lock(&core.rec).rejected.clone();
            assert!(
                rejected.iter().any(|r| r["reason"].as_str().unwrap_or("").contains("UR-22: a send did not complete") || r["reason"].as_str().unwrap_or("").contains("fake: the send failed")),
                "{nth}: {rejected:?} {sends:?}"
            );
            let sent = device.transmitted(clock.instant(40_000), len + 2);
            let took = sent.iter().zip(&wave).take_while(|(s, w)| **s == Some(**w)).count();
            assert!(took < len - 1, "nth {nth}: {took} of {len} {sends:?}");
            assert_eq!(sent[took], Some([0.0, 0.0]), "nth {nth}: after {took} {sends:?}");
            assert_eq!(sent[took + 1], Some([0.0, 0.0]), "nth {nth}: the second closing zero");
            assert!(sent[took + 2..].iter().all(Option::is_none), "nth {nth}: {sends:?}");
            assert_eq!(sends.iter().filter(|s| s.2).count(), 1, "nth {nth}: {sends:?}");
            if len == 6_000 {
                assert!(took > 5_000, "nth {nth}: the cut was not in the final buffer: {took} {sends:?}");
            }
        }

        for fault in [crate::device::FakeFault::ShortSend(1), crate::device::FakeFault::FailSend(1)] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, vec![fault.clone()], BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let wave: Vec<Iq> = (0..997).map(|i| [i as f32 / 1_000.0, -(i as f32) / 2_000.0]).collect();
            let mut burst = txp_burst(&clock, start, wave.len(), true, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            txp_give(&mut tx, burst);
            assert!(tx.step(), "{fault:?}: first tiled send");
            assert!(tx.step(), "{fault:?}: the second tiled send is cut");
            assert!(tx.open_next().is_none(), "{fault:?}: a partial/error send is abandoned");
            let handed = if matches!(&fault, FakeFault::ShortSend(_)) { 10_000 } else { 20_000 };
            assert_eq!(tx.chain.front(), Some((clock.domain, start + 20_000 + handed + 2 + GAP_200MSPS)), "{fault:?}: recovery uses the accepted or conservatively attempted cursor");
            let sent = device.transmitted(clock.instant(start), 30_002);
            let expected: Vec<_> = (0..30_000).map(|i| Some(wave[i % wave.len()])).collect();
            assert_eq!(&sent[..30_000], expected, "{fault:?}: the accepted prefix keeps the original repeat phase");
            assert_eq!(sent[30_000], Some([0.0, 0.0]), "{fault:?}: EOB follows what the device accepted");
            assert_eq!(sent[30_001], Some([0.0, 0.0]), "{fault:?}: the second EOB zero follows the first");
            assert!(sent[30_002..].iter().all(Option::is_none), "{fault:?}: the unsent tiled suffix is never replayed");
            assert!(
                lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("send") || reason.contains("failed"))),
                "{fault:?}: {:?}",
                lock(&core.rec).rejected
            );
        }
    }

    #[test]
    fn ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst() {
        for fault in [FakeFault::ShortSend(1), FakeFault::StalledSend(1), FakeFault::FailSend(1)] {
            let faults = vec![fault.clone(), FakeFault::TxReport(Wall::ZERO, TxCode::BurstAck)];
            let config = FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() };
            let (core, device, time, _events) = super::super::test_support::rig_with(config, Vec::new());
            let frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(200e6), ..Settings::default() }).unwrap();
            let clock = core.register(Dir::Tx, 1, 0).unwrap();
            let sample = [[0.25, 0.0]];
            device.tx_send(0, &[&sample], Some(clock.instant(500_000)), true, false, Wall::ZERO).unwrap();
            let (_to_tx, cmds) = std::sync::mpsc::channel();
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            tx.chain.advance_front(clock.domain, 200);
            tx.remember_burst(TimePoint::new(clock.domain, 500_000));
            tx.chain.open_device_burst(0, 500_001, Some(Tail { samples: [[0.25, 0.0]; TX_CHANNELS_MAX], channels: 1 }));

            tx.end_open(true);
            assert!(tx.chain.needs_reopen(), "{fault:?}: a partial or failed EOB did not invalidate the streamer");
            assert_eq!(tx.chain.next(), None, "{fault:?}: failed end_open does not retain the device cursor");
            assert!(tx.chain.tail().is_none(), "{fault:?}: failed end_open abandons the tail");
            assert_eq!(tx.chain.front(), Some((clock.domain, 200)), "{fault:?}: failed close advanced the safe front");
            assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "tx_close"), "{fault:?}: {:?}", lock(&core.rec).rejected);
            assert_eq!(tx.unacked.len(), 1, "{fault:?}: the old experiment burst stays correlated");

            txp_give(&mut tx, txp_burst(&clock, 1_000_000, 1, false, LatePolicy::DropAndFlag));
            // The recovery flush waits for its ack on the device's clock: it runs as a participant, and the clock is held again before it ends.
            let ((mut tx, passed), frozen) = crate::provider::test_support::released(frozen, &device.host(), move || {
                let passed = tx.pass();
                (tx, passed)
            });
            assert!(passed, "{fault:?}: the owner did not reopen the uncertain streamer");
            let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
            assert!(sends[2].contains("n=4 at=- sob=true eob=true"), "{fault:?}: recovery flush was not the first send after the failed close: {sends:?}");
            assert_eq!(sends.len(), 3, "{fault:?}: timed experiment data crossed streamer replacement: {sends:?}");
            assert!(!tx.chain.needs_reopen());
            assert!(tx.unacked.is_empty(), "{fault:?}: replacing the streamer retained old-generation correlations");

            // Both clocks, the Core's and the device's, are put at one instant, with the device's clock held, before the burst is dispatched.
            assert!(device.time_now().unwrap() <= TX_RECOVERY_RESUME, "the held clock stands before the resume instant");
            device_at(&core, &time, &device, TX_RECOVERY_RESUME);
            assert!(tx.pass(), "{fault:?}: the following timed burst was not dispatched");
            let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
            assert_eq!(sends.len(), 4, "{fault:?}: {sends:?}");
            assert!(sends[3].contains("at=1000000 sob=true"), "{fault:?}: the timed burst did not follow the recovery flush: {sends:?}");
            drop(frozen);
        }
    }

    #[test]
    fn ur_28_ack_ticks_separate_a_lost_flush_ack_from_later_bursts() {
        let config = FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::DropBurstAck], ..FakeConfig::default() };
        let (core, device, _time, events) = super::super::test_support::rig_with(config, Vec::new());
        device.tx_open(0, &[0]).unwrap();
        let result = core.tx_flush(0, &[0]);
        assert!(!result.acknowledged, "the suppressed flush ack was reported as received");

        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        tx.remember_burst(TimePoint::new(clock.domain, 20_000));
        tx.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(20_000) + 1), channel: 0 });
        assert!(tx.unacked.is_empty(), "the experiment ack is not swallowed as the missing flush ack");

        let next = TimePoint::new(clock.domain, 30_000);
        tx.remember_burst(next);
        core.stash_tx_report(0, TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(29_999)), channel: 0 });
        tx.reports(Wall::ZERO);
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [next], "an earlier flush ack does not retire a later experiment correlation");
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty());
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "BurstAck").count(), 2, "the late ack is still visible in the raw async record");
    }

    /// An instant after the device's time at a recovery flush's ack (a few device ticks past the failed send) and before the next burst's start.
    const TX_RECOVERY_RESUME: i64 = 600_000;

    #[test]
    fn ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery() {
        let (core, device, time, _events, clock, mut tx, frozen) = txp_high_rate_rig(1, vec![FakeFault::ShortSend(0)], BTreeMap::new());
        let start = 500_000;
        txp_give(&mut tx, txp_burst(&clock, start, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.step(), "the one-sample payload did not reach the padded EOB send");
        assert!(tx.chain.needs_reopen());
        assert_eq!(tx.chain.front(), None, "an attempted two-sample EOB is not a safe front");
        let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends.len(), 1, "the uncertain EOB is not followed by a close on the same streamer: {sends:?}");
        assert!(sends[0].contains("n=2 at=500000 sob=true eob=false"), "the short padded EOB transferred two of its four samples: {sends:?}");
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().is_some_and(|reason| reason.contains("did not complete"))));
        assert!(lock(&core.rec).rejected.iter().any(|row| row["call"] == "tx_close"), "the failed EOB is identified as an uncertain close");

        device_at(&core, &time, &device, start + 3);
        // The recovery flush waits for its ack on the device's clock: it runs as a participant, and the clock is held again before it ends.
        let ((mut tx, recovered), frozen) = crate::provider::test_support::released(frozen, &device.host(), move || {
            let recovered = tx.pass();
            (tx, recovered)
        });
        assert!(recovered, "streamer replacement and recovery flush were not attempted");
        assert!(!tx.chain.needs_reopen());
        let flush_tick =
            lock(&core.rec).device_async.iter().find(|row| row["tx_flush"] == true && row["code"] == "BurstAck").and_then(|row| row["tick"].as_i64()).expect("the recovery flush ack was recorded");
        assert_eq!(tx.chain.front(), Some((clock.domain, clock.at_or_after(flush_tick - 1) + GAP_200MSPS)), "the recovered flush ack establishes the safe front the gap past the flush");
        let after_flush: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert!(after_flush[1].contains("n=4 at=- sob=true eob=true"), "the replacement flush precedes any next timed burst: {after_flush:?}");

        // Both clocks, the Core's and the device's, are put at one instant before the next burst is booked and dispatched, with the device's clock held.
        assert!(device.time_now().unwrap() <= TX_RECOVERY_RESUME, "the held clock stands before the resume instant");
        device_at(&core, &time, &device, TX_RECOVERY_RESUME);
        txp_give(&mut tx, txp_burst(&clock, 1_000_000, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.pass(), "the burst after a recovered EOB was not sent");
        let sends: Vec<_> = device.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends.len(), 3, "the next burst follows the flush: {sends:?}");
        assert!(sends[2].contains("at=1000000 sob=true"), "the first later data packet remains timed: {sends:?}");
        drop(frozen);
    }

    /// A repeat whose send is abandoned keeps its accepted cursor as the recovery front (UR-23), and its span stays open
    /// until a Stop: a booking inside it is refused at receipt and never reaches the device (S5).
    #[test]
    fn ur_23_abandoned_send_preserves_the_accepted_cursor() {
        let a_samples = vec![[0.25, 0.0]; 2_000];
        let (a_wave, a_bytes) = waveform_input("abandoned_repeat", &a_samples);
        let (b_wave, b_bytes) = waveform_input("after_abandon", &[[0.75, -0.25]]);
        for (fault, cursor) in [(crate::device::FakeFault::ShortSend(0), 40_500), (crate::device::FakeFault::FailSend(0), 41_000)] {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, _events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 40_000, &a_wave, true, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            device_at(&core, &time, &device, core.ticks(31_000_000));
            assert!(tx.step(), "{case}: A was not sent");
            assert_eq!(tx.chain.front(), Some((clock.domain, cursor + 2 + GAP_1MSPS)), "{case}: recovery starts after the samples accepted or conservatively counted");

            let calls = device.calls().len();
            book_burst(&mut control, &core, 2, clock, 40_499, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            let rejected = lock(&core.rec).rejected.clone();
            assert!(rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst overlaps an accepted booking")), "{case}: {rejected:?}");
            assert_eq!(device.calls().len(), calls, "{case}: the refused B reached the device");
        }
    }

    #[test]
    fn ur_23_a_continuation_tail_is_counted_at_recovery() {
        let a_samples = vec![[0.25, 0.0]; 6_000];
        let b_samples = vec![[0.75, -0.25]; 1_000];
        let (a_wave, a_bytes) = waveform_input("tail_a", &a_samples);
        let (b_wave, b_bytes) = waveform_input("tail_b", &b_samples);
        for (fault, cursor, tail_taken, payload_taken) in
            [(crate::device::FakeFault::ShortSend(4), 8_499, true, 499), (crate::device::FakeFault::FailSend(4), 8_999, true, 499), (crate::device::FakeFault::FailSend(3), 8_000, false, 0)]
        {
            let case = format!("{fault:?}");
            let config = FakeConfig { clock: ClockMode::Manual, faults: vec![fault], ..FakeConfig::default() };
            let device = Arc::new(FakeDevice::new(config));
            let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes.clone())), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes.clone()))]);
            let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
            let _frozen = crate::provider::test_support::Frozen::of(&device);
            device_at(&core, &time, &device, 0);
            device.tx_open(0, &[0]).unwrap();
            device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (mut control, cmds) = booking_control(&core);
            let clock = tx_clock(&core);
            let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
            book_burst(&mut control, &core, 1, clock, 2_000, &a_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            for _ in 0..3 {
                assert!(tx.step(), "{case}: A's chunks");
            }
            assert!(tx.open_next().is_none() && tx.chain.next() == Some(8_000) && tx.chain.tail().as_ref().is_some_and(|tail| tail.channels == 1), "{case}: A's held tail");
            assert_eq!(sends(&device).len(), 3, "{case}: the tail must still be held");

            // B at A's end joins the held tail's chain (S2, received on time), so it continues A's device burst.
            book_burst(&mut control, &core, 2, clock, 8_000, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            assert!(held_keys(&tx).contains(&8_000), "{case}: B did not join A's end");
            assert!(tx.step(), "{case}: B's continuation decision");
            let moved =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert!(moved.is_empty(), "{case}: B joins on time: {moved:?}");
            let safe = cursor + 2 + GAP_1MSPS;
            assert!(tx.open_next().is_none(), "{case}: the incomplete continuation was resumed");
            let mut expected = vec![Some([0.25, 0.0]); 5_999];
            if tail_taken {
                expected.push(Some([0.25, 0.0]));
            }
            expected.extend(b_samples[..payload_taken].iter().copied().map(Some));
            expected.extend([Some([0.0, 0.0]), Some([0.0, 0.0])]);
            expected.resize((safe - 2_000) as usize, None);
            assert_eq!(device.transmitted(clock.instant(2_000), expected.len()), expected, "{case}: FakeDevice's accepted samples and padding may precede the conservative recovery cursor");
            assert_eq!(tx.chain.front(), Some((clock.domain, safe)), "{case}: the recovery front is the safe sample");

            // The padded end of the failed close lies inside B's accepted span, so a booking there is refused at receipt and never reaches the device.
            let calls = device.calls().len();
            book_burst(&mut control, &core, 3, clock, cursor + 2, &b_wave, false, LatePolicy::DropAndFlag);
            drain_tx(&mut tx);
            let rejected = lock(&core.rec).rejected.clone();
            let overlaps = rejected.iter().filter(|row| row["reason"].as_str().unwrap_or("").starts_with("UR-21: the burst overlaps")).count();
            assert_eq!(overlaps, 1, "{case}: the booking inside B's span is refused: {rejected:?}");
            let late =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect::<Vec<_>>();
            assert!(late.is_empty(), "{case}: the refused booking was also late: {late:?}");
            assert_eq!(device.calls().len(), calls, "{case}: the refused booking reached the device");
        }
    }

    #[test]
    fn ur_26_shutdown_after_loss_cancels_held_bursts_record_only() {
        let (core, device, _, events) = super::super::test_support::rig();
        let current = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(current), Channels::first(1));
        hold(&tx, held(&current, 20, 1, LatePolicy::DropAndFlag));
        core.device_lost("fake loss before the burst start");

        tx.shutdown();

        assert!(held_keys(&tx).is_empty());
        let rejected = lock(&core.rec).rejected.clone();
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected.iter().all(|row| row["reason"] == "UR-26: cancelled by Provider::stop"), "{rejected:?}");
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().any(|event| event.kind.as_str() == ezsdr_kernel::event::EventKind::DEVICE_LOST));
        assert!(!events.iter().any(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED));
        assert!(!device.calls().iter().any(|call| call.starts_with("tx_send ")));
    }

    // UR-23: the window's last stretch waits for a whole buffer (stage TXP). The unit scale: 1 MS/s on a manual clock,
    // 200 root ticks a sample, window 10 000 samples, chunk 2 500 (window/4), device lead 2 000 samples.

    /// A transmit owner on a frozen manual clock: the window `window_ns` (the default 10 ms when `None`) and RX `block_len`.
    #[allow(clippy::type_complexity)]
    fn txp_rig(
        window_ns: Option<i64>,
        block_len: u32,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        txp_rig_rate(window_ns, block_len, 1e6, 200)
    }

    #[allow(clippy::type_complexity)]
    fn txp_rig_rate(
        window_ns: Option<i64>,
        block_len: u32,
        rate: f64,
        ticks_per_sample: i64,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        txp_rig_faults(window_ns, block_len, rate, ticks_per_sample, Vec::new())
    }

    #[allow(clippy::type_complexity)]
    fn txp_rig_faults(
        window_ns: Option<i64>,
        block_len: u32,
        rate: f64,
        ticks_per_sample: i64,
        faults: Vec<FakeFault>,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults, ..FakeConfig::default() }));
        let mut description = super::super::test_support::default_description(block_len);
        if let Some(window) = window_ns {
            description.timing.tx_in_flight_ns = window;
        }
        let (core, time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), description);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(rate), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, ticks_per_sample, 0).unwrap();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(to_tx);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, device, time, events, clock, tx, frozen)
    }

    #[allow(clippy::type_complexity)]
    fn txp_high_rate_rig(
        channels: usize,
        faults: Vec<FakeFault>,
        inputs: BTreeMap<ContentHash, Arc<[u8]>>,
    ) -> (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Arc<EventCollector>, Clock, Tx, crate::provider::test_support::Frozen) {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, faults, ..FakeConfig::default() }));
        let (core, time, events) = super::super::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        let channel_ids: Vec<_> = (0..channels).collect();
        device.tx_open(0, &channel_ids).unwrap();
        for &channel in &channel_ids {
            device.apply(Dir::Tx, channel, &Settings { rate: Some(200e6), ..Settings::default() }).unwrap();
        }
        let clock = core.register(Dir::Tx, 1, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels as u32));
        (core, device, time, events, clock, tx, frozen)
    }

    /// Sample `i` of burst `k` is `[(i + 1) / 100 000, k / 10 000]`, so a played-out sample names its place.
    fn txp_burst(clock: &Clock, k: i64, len: usize, repeat: bool, policy: LatePolicy) -> Held {
        let mut burst = held(clock, k, len, policy);
        burst.samples = Arc::new(vec![(0..len).map(|i| [(i + 1) as f32 / 100_000.0, k as f32 / 10_000.0]).collect()]);
        burst.open.waveform_len = u32::try_from(len).ok();
        burst.repeat = repeat;
        burst
    }

    fn txp_give(tx: &mut Tx, burst: Held) {
        give(tx, burst);
    }

    /// The booking `control.rs` makes, without the hand-over: the record holds the burst's chain, so the owner's close can see
    /// a continuation that is still joinable (UR-23, RM-16).
    fn txp_record(core: &Core, clock: Clock, burst: &Held) {
        let mut streams = lock(&core.streams);
        let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
        let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
        busy.accept(burst.k, (!burst.repeat).then_some(burst.waveform_len as i64), core.now(), |k| core.join_by(clock, k), horizon).expect("the record takes the test's booking");
        busy.book((!burst.repeat).then(|| clock.instant(burst.k + burst.waveform_len as i64)));
    }

    /// The receipt (`txp_record`: the record accepts it) and the list entry, then the owner's read.
    fn txp_book(tx: &mut Tx, core: &Core, clock: Clock, burst: Held) {
        txp_record(core, clock, &burst);
        txp_give(tx, burst);
    }

    /// The owner's count of closes past the record's front (Q-6, §1.4).
    fn past_front(core: &Core) -> i64 {
        lock(&core.rec).stats.get("tx_close_past_record_front").copied().unwrap_or(0)
    }

    /// Steps until the owner has nothing to do, failing instead of looping if it never stops: a wait that reports itself as work
    /// would otherwise hang the test (design note 37).
    fn txp_drain(tx: &mut Tx) {
        for _ in 0..100_000 {
            if !tx.step() {
                return;
            }
        }
        panic!("the owner still reports work after 100 000 steps");
    }

    /// Joins `thread` once it has ended, failing if it has not within 10 s of real time.
    fn txp_join(thread: std::thread::JoinHandle<()>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !thread.is_finished() {
            assert!(std::time::Instant::now() < deadline, "a transmit owner did not end within 10 s of its Shutdown");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        thread.join().expect("a transmit owner");
    }

    /// Root ticks of `samples` samples.
    fn txp_ticks(samples: i64) -> i64 {
        samples * 200
    }

    #[test]
    fn ur_23_txs_tiles_a_short_repeat_across_original_wraps_at_high_rate() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let wave: Vec<Iq> = (0..997).map(|i| [(i + 1) as f32 / 997.0, -(i as f32) / 1_000.0]).collect();
        let start = 2_000_000 - 996;
        let mut burst = txp_burst(&clock, start, wave.len(), true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![wave.clone()]);
        txp_give(&mut tx, burst);
        txp_drain(&mut tx);

        let held = tx.open_booking(|held| held.samples.clone()).expect("the repeat remains open");
        let tiled = &held[0];
        assert_eq!(sends(&device), [(996, true, false)], "the first timed SOB takes the available prefix");
        assert_eq!(tiled.len() % wave.len(), 0, "the tile contains whole original periods");
        assert!(tiled.len() >= 20_000 + wave.len() - 1 && tiled.len() < 60_000, "tile bound: {} samples", tiled.len());
        assert!(tiled.iter().enumerate().all(|(i, sample)| *sample == wave[i % wave.len()]));
        assert_eq!(tx.open_booking(|held| held.open.waveform_len).unwrap(), Some(997));

        device_at(&core, &time, &device, clock.instant(20_000));
        assert!(tx.step(), "the continuation now has a full 20,000-sample window");
        assert_eq!(*sends(&device).last().unwrap(), (20_000, false, false));
        assert!(Arc::ptr_eq(&held, &tx.open_booking(|held| held.samples.clone()).unwrap()), "the burst is not re-tiled between sends");
        device_at(&core, &time, &device, clock.instant(40_000));
        assert!(tx.step(), "a second continuation crosses another original-waveform wrap");
        assert_eq!(*sends(&device).last().unwrap(), (20_000, false, false));
        assert!(Arc::ptr_eq(&held, &tx.open_booking(|held| held.samples.clone()).unwrap()), "the burst is not re-tiled between sends");
        let expected: Vec<_> = (0..40_996).map(|i| Some(wave[i % wave.len()])).collect();
        assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "continuations keep the original 997-sample phase");

        halt(&mut tx);
        let record = lock(&core.rec).bursts.last().cloned().expect("the stopped repeat record");
        assert_eq!(record["samples"], 40_996);
        assert_eq!(record["wraps"], 41, "wrap accounting uses the original 997-sample waveform");
    }

    #[test]
    fn ur_22_repeat_wrap_is_not_a_new_burst_start() {
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 500, true, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert_eq!(sends(&device), [(2_500, true, false)]);
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&1));
        assert!(tx.step(), "the next chunk starts at an original-waveform wrap");
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false));
        assert_eq!(tx.open_next().unwrap(), 8_000);
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&1));
    }

    #[test]
    fn ur_23_txs_a_held_cut_splits_a_high_rate_repeat_at_the_target() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 50_000, true, LatePolicy::DropAndFlag);
        let a = repeat.samples[0].clone();
        txp_give(&mut tx, repeat);
        assert!(tx.step());
        assert_eq!(sends(&device), [(20_000, true, false)]);

        let next = txp_burst(&clock, start + 30_000, 1_000, false, LatePolicy::DropAndFlag);
        let b = next.samples[0].clone();
        txp_give(&mut tx, next);
        assert!(tx.step(), "the repeat sends only to the held start");
        assert_eq!(*sends(&device).last().unwrap(), (10_000, false, false));
        assert_eq!(tx.chain.next(), Some(start + 30_000));
        assert!(tx.step(), "the adjacent burst continues on the device burst");
        assert_eq!(*sends(&device).last().unwrap(), (999, false, false));
        device_at(&core, &time, &device, clock.instant(start + 31_000 - 400_000));
        assert!(tx.step(), "the last held sample closes at the burst end");
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true));

        let expected: Vec<_> = a.into_iter().take(30_000).chain(b).map(Some).chain([Some([0.0, 0.0])]).collect();
        assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "no old sample crosses the cut and the next burst begins at it");
        let records = lock(&core.rec).bursts.clone();
        assert_eq!(records.len(), 2);
        assert_eq!((records[0]["samples"].as_u64(), records[0]["target"]["ticks"].as_i64()), (Some(30_000), Some(start)));
        assert_eq!((records[1]["samples"].as_u64(), records[1]["target"]["ticks"].as_i64()), (Some(1_000), Some(start + 30_000)));
    }

    #[test]
    fn ur_23_s3_dispatch_move_into_a_later_booking_is_refused() {
        // UR-21, UR-23 (Sol 3, M4): a head that host lateness moves at dispatch takes its moved region through the booking record. A
        // collision with a later booking, or with a joined piece of its own chain, refuses the burst (TIME_ERROR refused, its Action's
        // id), and the booking it meets is sent as booked. A move into free space is sent at the moved start.
        let action = |burst: &mut Held, id: u64| burst.settings = Arc::new(super::ColdConfig::of_tx(BTreeMap::new(), ActionId(id), 0));
        let drained = |events: &ezsdr_kernel::event::EventCollector| events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let of_kind = |events: &[ezsdr_kernel::event::Event], kind: &str| events.iter().filter(|event| event.kind.as_str() == kind).cloned().collect::<Vec<_>>();

        // Sol 3: A [20 000, 40 000) is moved to 42 000 by a dispatch at 40 000; B at 50 000 is booked.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        let mut a = txp_burst(&clock, 20_000, 20_000, false, LatePolicy::SendAsapAndFlag);
        action(&mut a, 2);
        txp_book(&mut tx, &core, clock, a);
        let b = txp_burst(&clock, 50_000, 1_000, false, LatePolicy::DropAndFlag);
        let b_samples = b.samples[0].clone();
        txp_book(&mut tx, &core, clock, b);
        device_at(&core, &time, &device, txp_ticks(40_000));
        txp_drain(&mut tx);
        let events = drained(&events);
        let errors = of_kind(&events, ezsdr_radio::kinds::TIME_ERROR);
        assert_eq!(errors.len(), 1, "{events:?}");
        assert_eq!(errors[0].payload["outcome"], "refused");
        assert_eq!(of_kind(&events, ezsdr_radio::kinds::COMMAND_REJECTED).iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(2)]);
        device_at(&core, &time, &device, txp_ticks(48_000));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(49_000));
        txp_drain(&mut tx);
        let sent = sends(&device);
        assert_eq!(sent.iter().map(|send| send.0).sum::<usize>(), 1_000, "only B is sent: {:?}", device.calls());
        assert!(sent.first().is_some_and(|send| send.1) && sent.last().is_some_and(|send| send.2), "{sent:?}");
        let expected: Vec<_> = b_samples.iter().copied().map(Some).collect();
        assert_eq!(device.transmitted(clock.instant(50_000), expected.len()), expected, "B plays as booked");

        // M4: A(30 000) at 3 000 with B(10) joined at 33 000. A's dispatch at 1 001 moves it to 3 001, whose region reaches B's piece.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        let mut a = txp_burst(&clock, 3_000, 30_000, false, LatePolicy::SendAsapAndFlag);
        action(&mut a, 2);
        txp_book(&mut tx, &core, clock, a);
        let b = txp_burst(&clock, 33_000, 10, false, LatePolicy::DropAndFlag);
        let b_samples = b.samples[0].clone();
        txp_book(&mut tx, &core, clock, b);
        device_at(&core, &time, &device, txp_ticks(1_001));
        txp_drain(&mut tx);
        let events = drained(&events);
        let errors = of_kind(&events, ezsdr_radio::kinds::TIME_ERROR);
        assert_eq!(errors.len(), 1, "{events:?}");
        assert_eq!(errors[0].payload["outcome"], "refused");
        assert_eq!(of_kind(&events, ezsdr_radio::kinds::COMMAND_REJECTED).len(), 1);
        assert!(sends(&device).is_empty(), "the refused move sent nothing: {:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(31_000));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(34_000));
        txp_drain(&mut tx);
        let sent = sends(&device);
        assert_eq!(sent.iter().map(|send| send.0).sum::<usize>(), 10, "only B is sent: {:?}", device.calls());
        assert!(sent.first().is_some_and(|send| send.1) && sent.last().is_some_and(|send| send.2), "{sent:?}");
        let expected: Vec<_> = b_samples.iter().copied().map(Some).collect();
        assert_eq!(device.transmitted(clock.instant(33_000), expected.len()), expected, "B plays as booked");

        // Free space: the same move with no booking after A is sent at the moved start, one SendAsap error.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::SendAsapAndFlag));
        device_at(&core, &time, &device, txp_ticks(1_001));
        txp_drain(&mut tx);
        let events = drained(&events);
        let errors = of_kind(&events, ezsdr_radio::kinds::TIME_ERROR);
        assert_eq!(errors.len(), 1, "{events:?}");
        assert_eq!(errors[0].payload["outcome"], "send_asap");
        assert!(sends(&device).first().is_some_and(|send| send.1), "the moved first send opens a device burst: {:?}", device.calls());
    }

    #[test]
    fn ur_23_s3_refused_move_frees_its_front() {
        // UR-23 (Sol 3): a refused move leaves the overlap rule. The refused head held [20 000, 40 002): a booking there is accepted.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 20_000, false, LatePolicy::SendAsapAndFlag));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 50_000, 1_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(40_000));
        txp_drain(&mut tx);
        assert!(!held_keys(&tx).contains(&20_000), "the refused burst is gone");
        txp_record(&core, clock, &txp_burst(&clock, 30_000, 4, false, LatePolicy::DropAndFlag));

        // C6's value half: a refused move keeps the burst's booked end in the value rule. A [20 000, 60 002) is refused at 40 000 by
        // B [70 000, 70 100): a booking that meets A's moved region starts after A's front, so its end is the later one, and no value
        // before A's end is taken.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 20_000, 40_000, false, LatePolicy::SendAsapAndFlag));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 70_000, 100, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(40_000));
        txp_drain(&mut tx);
        assert!(!held_keys(&tx).contains(&20_000), "the refused burst is gone");
        let streams = lock(&core.streams);
        let busy = &streams.stream(stream_key(Dir::Tx, 0)).book.record;
        assert!(busy.stageable(clock.instant(50_000)).is_err(), "A's booked end (60 000) still counts for a value");
        assert!(busy.stageable(clock.instant(70_100)).is_ok(), "at the last booked end the value is taken");
    }

    #[test]
    fn ur_23_s3_a_pruned_head_moved_into_a_later_booking_is_refused() {
        // F10 (5b-3): B's receipt at 61 000 prunes A's span (its front, 60 002, is behind it) while A is still held. A dispatched late
        // at 63 000 is tested against B like any other head: it is refused, and B plays as booked.
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (a_wave, a_bytes) = waveform_input("f10_a", &vec![[0.25, 0.0]; 40_000]);
        let (b_wave, b_bytes) = waveform_input("f10_b", &[[0.75, -0.25]; 10]);
        let inputs = BTreeMap::from([(a_wave.hash.clone(), Arc::<[u8]>::from(a_bytes)), (b_wave.hash.clone(), Arc::<[u8]>::from(b_bytes))]);
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let (mut control, cmds) = booking_control(&core);
        let clock = tx_clock(&core);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_burst(&mut control, &core, 1, clock, 20_000, &a_wave, false, LatePolicy::SendAsapAndFlag);
        device_at(&core, &time, &device, clock.instant(61_000));
        book_burst(&mut control, &core, 2, clock, 70_000, &b_wave, false, LatePolicy::DropAndFlag);
        device_at(&core, &time, &device, clock.instant(63_000));
        drain_tx(&mut tx);
        txp_drain(&mut tx);
        let emitted = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let errors: Vec<_> = emitted.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(errors.len(), 1, "{emitted:?}");
        assert_eq!(errors[0].payload["outcome"], "refused");
        let rejected: Vec<_> = emitted.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect();
        assert_eq!(rejected.iter().map(|e| e.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(1)], "{emitted:?}");
        device_at(&core, &time, &device, clock.instant(71_000));
        txp_drain(&mut tx);
        assert_eq!(sends(&device).iter().map(|send| send.0).sum::<usize>(), 10, "only B is sent: {:?}", device.calls());
        let expected = vec![Some([0.75, -0.25]); 10];
        assert_eq!(device.transmitted(clock.instant(70_000), expected.len()), expected, "B plays as booked");
    }

    #[test]
    fn ur_23_a_burst_at_the_handed_frontier_continues_the_open_burst() {
        // UR-23 (P1166): a held burst that starts at the open burst's first unhanded sample continues it on time, with no end-of-burst between.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let a = repeat.samples[0].clone();
        txp_give(&mut tx, repeat);
        assert!(tx.step(), "first send");
        assert_eq!(tx.open_next().unwrap(), 520_000, "first unhanded sample");
        let next = txp_burst(&clock, 520_000, 4, false, LatePolicy::DropAndFlag);
        let b = next.samples[0].clone();
        txp_give(&mut tx, next);
        assert!(tx.step(), "the frontier burst is decided");
        assert_eq!(held_keys(&tx), [520_000]);
        assert!(tx.step(), "adjacent continuation");
        assert_eq!(*sends(&device).last().unwrap(), (3, false, false));
        halt(&mut tx);
        let errors: Vec<_> =
            events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert!(errors.is_empty(), "{errors:?}");
        let expected: Vec<_> = a[..20_000].iter().copied().chain(b).map(Some).collect();
        assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected, "the frontier sample is neither late nor skipped");
        let records = lock(&core.rec).bursts.clone();
        assert_eq!(records.len(), 2, "both logical bursts are recorded");
        assert_eq!(records[1]["target"]["ticks"], 520_000);
    }

    #[test]
    fn ur_23_txs_stop_does_not_send_past_a_high_rate_bound() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let repeat = txp_burst(&clock, start, 30_000, true, LatePolicy::DropAndFlag);
        let expected: Vec<_> = repeat.samples[0][..20_000].iter().copied().map(Some).collect();
        txp_give(&mut tx, repeat);
        assert!(tx.step());
        halt(&mut tx);
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "Stop sends only the EOB closure after the handed chunk");
        assert_eq!(device.transmitted(clock.instant(start), 20_002), expected.into_iter().chain([Some([0.0, 0.0]), Some([0.0, 0.0])]).collect::<Vec<_>>());
        assert_eq!(lock(&core.rec).bursts.last().unwrap()["samples"], 20_000);
    }

    #[test]
    fn ur_23_txs_one_shots_keep_their_payload_and_end_on_the_last_sample() {
        for len in [1_000, 25_000] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
            let start = 10_000_000;
            device_at(&core, &time, &device, clock.instant(start + len as i64 - 1_200_000));
            let wave: Vec<Iq> = (0..len).map(|i| [0.1 + i as f32 / 100_000.0, -(i as f32) / 200_000.0]).collect();
            let mut burst = txp_burst(&clock, start, len, false, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![wave.clone()]);
            txp_book(&mut tx, &core, clock, burst);
            txp_drain(&mut tx);
            assert!(tx.chain.tail().is_some());
            let expected_sizes = if len < 20_000 { vec![len - 1] } else { vec![20_000, len - 20_001] };
            assert_eq!(sends(&device).iter().map(|send| send.0).collect::<Vec<_>>(), expected_sizes, "payload before the held tail for {len}");
            device_at(&core, &time, &device, clock.instant(start + len as i64 - 400_000));
            assert!(tx.step(), "{len}: the held tail ends at the final payload sample");
            assert_eq!(*sends(&device).last().unwrap(), (2, false, true));
            assert_eq!(device.transmitted(clock.instant(start), len), wave.iter().copied().map(Some).collect::<Vec<_>>(), "{len}: no tiled, missing, or duplicate data");
            let record = lock(&core.rec).bursts.last().cloned().unwrap();
            assert_eq!(record["samples"], len);
            assert_eq!(record["end"], "eob");
        }
    }

    #[test]
    fn ur_23_txs_grouped_short_repeats_keep_channel_phase_aligned() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(2, Vec::new(), BTreeMap::new());
        let start = 500_000;
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        for channel in 0..2 {
            device.apply(Dir::Rx, channel, &Settings { rate: Some(200e6), ..Settings::default() }).unwrap();
        }
        device.rx_open(1, &[0, 1]).unwrap();
        device.rx_start(1, clock.instant(start)).unwrap();
        let waves: Vec<Vec<Iq>> = (0..2).map(|channel| (0..997).map(|i| [((channel * 997 + i + 1) as f32) / 2_000.0, -((channel * 997 + i) as f32) / 3_000.0]).collect()).collect();
        let mut burst = txp_burst(&clock, start, 997, true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(waves.clone());
        txp_give(&mut tx, burst);
        assert!(tx.step());
        assert!(tx.step());
        assert_eq!(sends(&device), [(20_000, true, false), (20_000, false, false)]);
        device_at(&core, &time, &device, clock.instant(start + 40_000));
        let crate::device::RxRecv::Samples { samples, first_tick } = device.rx_recv(1, 40_000, Wall::from_millis(10), false) else { panic!("both channels loop back") };
        assert_eq!(first_tick, clock.instant(start));
        assert_eq!(samples.len(), 2);
        for channel in 0..2 {
            let expected: Vec<_> = (0..40_000).map(|i| waves[channel][i % 997]).collect();
            assert_eq!(samples[channel], expected, "channel {channel} keeps its own tiled phase and shared cursor");
        }
        halt(&mut tx);
    }

    #[test]
    fn ur_23_txs_does_not_copy_repeats_at_or_above_the_send_cap() {
        for len in [20_000, 25_000] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(1, Vec::new(), BTreeMap::new());
            let start = 500_000;
            device_at(&core, &time, &device, clock.instant(start - 400_000));
            let mut burst = txp_burst(&clock, start, len, true, LatePolicy::DropAndFlag);
            burst.samples = Arc::new(vec![(0..len).map(|i| [i as f32 / 100_000.0, 0.25]).collect()]);
            let input = burst.samples.clone();
            txp_give(&mut tx, burst);
            assert!(Arc::ptr_eq(&input, &tx.with_held(|map| map[&start].samples.clone())), "holding length {len} does not copy");
            assert!(tx.step());
            assert!(Arc::ptr_eq(&input, &tx.open_booking(|held| held.samples.clone()).unwrap()), "sending length {len} does not copy");
            assert_eq!(*sends(&device).last().unwrap(), (20_000, true, false));
            let expected: Vec<_> = input[0][..20_000].iter().copied().map(Some).collect();
            assert_eq!(device.transmitted(clock.instant(start), expected.len()), expected);
        }
    }

    #[test]
    fn ur_23_m1_a_repeat_stops_short_of_a_window_edge_it_cannot_fill() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        for _ in 0..10 {
            if !tx.step() {
                break;
            }
            let next = tx.open_next().unwrap();
            assert!(next <= 10_000 && device.transmitted(clock.instant(next), 1) == vec![None], "nothing at or past the window edge after a step: {next}");
        }
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open_next().unwrap(), 8_000, "2 000 samples are left before the 10 000 edge: no short buffer");
        let calls = device.calls().len();
        assert!(!tx.step(), "waiting is not work");
        assert_eq!(device.calls().len(), calls, "a wait makes no device call");
        // M1b: the buffer goes the moment the whole of it fits, and not before.
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(sends(&device).len(), 3);
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false));
        assert_eq!(tx.open_next().unwrap(), 10_500, "the window edge itself takes a buffer: `<`, not `<=`");
        // M1c: nothing past the edge was handed over.
        assert_eq!(device.transmitted(clock.instant(10_500), 1), vec![None]);
        assert!(!tx.step());
    }

    #[test]
    fn ur_23_m2_the_last_part_of_a_burst_is_not_split_by_the_window() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_500, 6_800, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open_next().unwrap(), 8_500, "1 500 samples fit, the 1 800 left do not: wait");
        device_at(&core, &time, &device, txp_ticks(300));
        assert!(tx.step());
        // All 800 samples go, the last one held back for the end-of-burst.
        assert_eq!(*sends(&device).last().unwrap(), (1_799, false, false));
        assert!(tx.chain.tail().is_some());
        assert_eq!(tx.chain.next(), Some(10_300));
    }

    #[test]
    fn ur_23_m3_a_tiled_repeat_waits_for_a_full_chunk_across_wraps() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 9_000, 1_500, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(1_000, true, false)], "the first timed buffer takes what fits");
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(!tx.step(), "a tiled continuation waits for the full chunk");
        device_at(&core, &time, &device, txp_ticks(2_500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false), "the full chunk crosses original waveform wraps");
        assert_eq!(tx.open_next().unwrap(), 12_500);
        let wave: Vec<_> = (0..3_500).map(|i| Some([((i % 1_500) + 1) as f32 / 100_000.0, 0.9])).collect();
        assert_eq!(device.transmitted(clock.instant(9_000), 3_500), wave, "no gap, no phase reset at the wrap");
    }

    #[test]
    fn ur_23_m4_a_continuation_waits_without_a_device_call() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(tx.open_next().unwrap(), 8_000);
        txp_give(&mut tx, txp_burst(&clock, 9_600, 4_000, false, LatePolicy::DropAndFlag));
        assert!(tx.step(), "A's last 1 600 before B, a cut-sized buffer");
        assert_eq!(*sends(&device).last().unwrap(), (1_600, false, false));
        assert_eq!(tx.chain.next(), Some(9_600));
        let calls = device.calls().len();
        assert!(!tx.step(), "B's first buffer, a continuation, waits for a whole buffer of room");
        assert_eq!(device.calls().len(), calls, "the wait reached the device (an entry that skips the rule)");
        assert!(tx.open_next() == Some(9_600) && tx.open_first() == Some(true));
        assert_eq!(tx.chain.next(), Some(9_600));
        assert!(tx.chain.tail().is_none());
        device_at(&core, &time, &device, txp_ticks(2_100));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_500, false, false), "B continues A's device burst");
        assert_eq!(sends(&device).iter().filter(|s| s.1).count(), 1, "one device start");
        assert_eq!(lock(&core.rec).stats.get("tx_bursts"), Some(&2));
    }

    #[test]
    fn ur_22_a_short_first_send_is_padded_to_four() {
        // UR-22: a first send of one to three samples goes at once as one timed SOB+EOB of four, its zeros after its samples.
        for len in 1..=3 {
            let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
            txp_give(&mut tx, txp_burst(&clock, 3_000, len, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            assert_eq!(sends(&device), [(4, true, true)], "len {len}: {:?}", device.calls());
            let mut expected: Vec<Option<[f32; 2]>> = (0..len).map(|i| Some([(i + 1) as f32 / 100_000.0, 3_000f32 / 10_000.0])).collect();
            expected.resize(4, Some([0.0, 0.0]));
            assert_eq!(device.transmitted(clock.instant(3_000), 4), expected, "len {len}");
            assert_eq!(tx.chain.front(), Some((clock.domain, 3_019)), "len {len}: the padded end + 15 (the gap at 1 MS/s)");
        }
    }

    #[test]
    fn ur_23_a_closed_short_burst_s_owner_front_is_gap_past_its_end() {
        // UR-22 (F8): a closed device burst of at most 32 transmitted samples fronts at its padded end plus `max(3, ceil(15 µs x rate))`, and
        // the owner's front is the record's: the record refuses a booking one sample before that front and takes one at it. The gaps are written
        // out from the arithmetic: 15, 30, 43 (N = 70), 150, 1 500 and 3 000 samples at 1, 2, 2.857, 10, 100 and 200 MS/s.
        for (rate, ticks_per_sample, gap) in [(1e6, 200, 15i64), (2e6, 100, 30), (200e6 / 70.0, 70, 43), (10e6, 20, 150), (100e6, 2, 1_500), (200e6, 1, 3_000)] {
            for len in [1usize, 3, 4, 10, 31, 32] {
                let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, ticks_per_sample);
                // The burst starts 5 ms after the device's time zero at every rate, as the 1 MS/s rigs book theirs.
                let start = (5_000.0 * rate / 1e6).round() as i64;
                txp_book(&mut tx, &core, clock, txp_burst(&clock, start, len, false, LatePolicy::DropAndFlag));
                txp_drain(&mut tx);
                let front = start + len.max(4) as i64 + gap;
                assert_eq!(sends(&device), [(len.max(4), true, true)], "{rate} S/s, len {len}: {:?} {:?}", device.calls(), lock(&core.rec).rejected);
                assert_eq!(tx.chain.front(), Some((clock.domain, front)), "{rate} S/s, len {len}: the owner's front");
                assert_eq!(past_front(&core), 0, "{rate} S/s, len {len}");
                let mut streams = lock(&core.streams);
                let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
                let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
                let mut book = |start| busy.accept(start, Some(1), core.now(), |k| core.join_by(clock, k), horizon);
                assert!(book(front - 1).is_err(), "{rate} S/s, len {len}: the record refuses one sample before the front");
                assert!(book(front).is_ok(), "{rate} S/s, len {len}: and takes the front");
            }
        }
    }

    #[test]
    fn ur_21_a_short_head_front_follows_the_stream_s_rate() {
        // RM-15: the record's front after a one-sample head (padded to four) is the gap of the stream's own rate past its padded end: 15 samples
        // at 1 MS/s and 30 at 2 on the 200 MHz root. The rig arms its record where it registers the clock (`register_in`).
        for (rate, ticks_per_sample, gap) in [(1e6, 200, 15), (2e6, 100, 30)] {
            let (core, _device, _time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, ticks_per_sample);
            txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 1, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            let mut streams = lock(&core.streams);
            let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
            let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
            let mut book = |start| busy.accept(start, Some(1), core.now(), |k| core.join_by(clock, k), horizon);
            assert!(book(5_000 + 4 + gap - 1).is_err(), "{rate} S/s: the record refuses one sample before the front");
            assert!(book(5_000 + 4 + gap).is_ok(), "{rate} S/s: and takes the front");
        }
    }

    #[test]
    fn ur_23_every_close_fronts_at_its_payload_end_plus_four_at_each_rate() {
        // UR-22 (UR-23): a 33-sample burst closes with its held tail and one zero, so its front is the payload end plus four whether a Stop
        // closes it (a two-zero close from the held tail, `close_start + 5`) or its own end does (`close_unjoined`), at every rate. The owner's front is
        // checked against that formula in both cases and no close is past the record's front; it is compared with the record's own front only
        // for the natural close, since this rig's `halt` does not run the record's Stop (the Stop's record side is pinned in `staging.rs` and `o1.rs`).
        for (rate, ticks_per_sample) in [(1e6f64, 200), (2e6, 100), (200e6 / 70.0, 70), (10e6, 20), (100e6, 2), (200e6, 1)] {
            let start = (6_000.0 * rate / 1e6).round() as i64;
            for stop in [true, false] {
                let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig_rate(None, 2_000, rate, ticks_per_sample);
                txp_book(&mut tx, &core, clock, txp_burst(&clock, start, 33, false, LatePolicy::DropAndFlag));
                txp_drain(&mut tx);
                assert_eq!(sends(&device), [(32, true, false)], "{rate} S/s: {:?}", device.calls());
                if stop {
                    halt(&mut tx);
                } else {
                    device_at(&core, &time, &device, core.join_by(clock, start + 33) + 1);
                    txp_drain(&mut tx);
                }
                assert_eq!(sends(&device), [(32, true, false), (2, false, true)], "{rate} S/s, stop {stop}: {:?}", device.calls());
                let front = start + 33 + 4;
                assert_eq!(tx.chain.front(), Some((clock.domain, front)), "{rate} S/s, stop {stop}: the owner's front");
                assert_eq!(past_front(&core), 0, "{rate} S/s, stop {stop}");
                let mut streams = lock(&core.streams);
                let busy = &mut streams.stream_mut(stream_key(Dir::Tx, 0)).book.record;
                let horizon = ezsdr_radio::staging::handover_horizon(core.now(), core.tx_in_flight(), |t| clock.at_or_after(t));
                let mut book = |at| busy.accept(at, Some(1), core.now(), |k| core.join_by(clock, k), horizon);
                if !stop {
                    assert!(book(front - 1).is_err(), "{rate} S/s: the record refuses one sample before the front");
                    assert!(book(front).is_ok(), "{rate} S/s: and takes the front");
                }
            }
        }
    }

    #[test]
    fn ur_23_the_abandon_front_follows_the_rate() {
        // UR-22 (D1): an abandoned open first send closes with two zeros, so its device burst is the accepted samples plus two. The owner's front
        // is the abandon's cursor plus two plus the gap of the stream's rate, which is where the fake's short guard ends for a device burst of at
        // most 32 samples (16 + 2 and 30 + 2) and beyond its three-sample guard for 33 (31 + 2). The fake accepts exactly half of a short send
        // (`FakeFault::ShortSend`), so open first sends of 32, 60 and 62 samples (the window holds that many; a first send never goes short, FS-3)
        // take 16, 30 and 31. The device clock stays put, so nothing runs dry (UR-33).
        for (rate, ticks_per_sample, gap) in [(2e6f64, 100, 30i64), (200e6, 1, 3_000)] {
            let start = (6_000.0 * rate / 1e6).round() as i64;
            for (first_send, accepted) in [(32i64, 16i64), (60, 30), (62, 31)] {
                let case = format!("{rate} S/s, first send {first_send}");
                let window_ns = ((start + first_send) as f64 * 1e9 / rate).round() as i64;
                let front = start + accepted + 2 + gap;
                // The device's own verdict, on a rig of its own for each probe: a timed SOB one tick before the front is refused only while the
                // fake's guard reaches it (the short bursts); at the front it is never refused for the guard.
                for (delta, refused) in [(-1i64, accepted + 2 <= 32), (0, false)] {
                    let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig_faults(Some(window_ns), 2_000, rate, ticks_per_sample, vec![FakeFault::ShortSend(0)]);
                    txp_book(&mut tx, &core, clock, txp_burst(&clock, start, 200, false, LatePolicy::DropAndFlag));
                    txp_drain(&mut tx);
                    assert_eq!(sends(&device).first(), Some(&(accepted as usize, true, false)), "{case}: the first send is cut to half: {:?}", device.calls());
                    assert_eq!(tx.chain.front(), Some((clock.domain, front)), "{case}: the abandon fronts at the cursor + 2 + the gap: {:?} {:?}", device.calls(), lock(&core.rec).rejected);
                    device.tx_send(0, &[&[[0.2f32, 0.0]; 4]], Some(clock.instant(front) + delta), true, true, Wall::from_secs(1)).unwrap();
                    device_at(&core, &time, &device, clock.instant(front) + 1_000 * ticks_per_sample);
                    let codes: Vec<TxCode> = std::iter::from_fn(|| device.tx_async(0, Wall::ZERO).unwrap()).map(|report| report.code).collect();
                    assert_eq!(codes.contains(&TxCode::TimeError), refused, "{case}, SOB at the front {delta:+} ticks: the fake's guard after {} transmitted samples: {codes:?}", accepted + 2);
                }
            }
        }
    }

    #[test]
    fn ur_23_a_continuation_that_closes_a_burst_keeps_the_front_three_past_its_end() {
        // UR-22 (Q5): a 40-sample burst whose window lets 32 samples go first, then 8 that end it once the device lead already reaches
        // their end, so they are sent with end-of-burst at once rather than held. The 8 are a continuation (`device_sob` false), not the
        // device burst's first send, so the close fronts three samples past its padded end, not the gap: the short closed burst's rule does not apply.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, 40, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(2_932));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(32, true, false)], "{:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(3_040));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(32, true, false), (8, false, true)], "{:?}", device.calls());
        assert_eq!(tx.chain.front(), Some((clock.domain, 5_043)), "the continuation's close fronts three samples past its padded end");
        assert_eq!(past_front(&core), 0);
    }

    #[test]
    fn ur_23_a_one_sample_continuation_eob_is_padded_to_two() {
        // UR-22 (N2): a one-sample continuation that closes the device burst is padded to two, not to four.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(tx.open_next().unwrap(), 8_000);
        txp_give(&mut tx, txp_burst(&clock, 9_600, 1, false, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (1_600, false, false));
        device_at(&core, &time, &device, txp_ticks(7_601));
        txp_drain(&mut tx);
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "{:?}", device.calls());
        assert_eq!(tx.chain.front(), Some((clock.domain, 9_605)));
    }

    #[test]
    fn ur_23_a_first_send_waits_for_32_window_samples() {
        // UR-22 (S3, #78): an open first send waits for 32 samples of window space, or for its whole length when that is shorter. The window
        // is 2 100 samples and the burst starts at 3 000: 28, then 31, samples of room at 928 and 931, nothing goes; at 932, 32. The device
        // lead (2 000 samples) still holds at 932, so the burst is not late.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 10_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(928));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "28 samples of window space were handed over: {:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(931));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "31 samples of window space were handed over: {:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(932));
        while sends(&device).is_empty() && tx.step() {}
        assert_eq!(sends(&device), [(32, true, false)], "{:?}", device.calls());
    }

    #[test]
    fn ur_23_a_burst_whose_window_first_takes_28_samples_plays_clean_to_its_end() {
        // UR-22 (S3, #78, the bench's shape): a 10 000-sample burst at 1 MS/s whose window first admits 28 samples (at 928). The first send waits
        // for 32 (at 932), which still leaves the device lead before the start at 1 000, so FakeDevice sees no open first send under 30 samples:
        // no Underflow, no TimeError, one BurstAck, the whole waveform played.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_100_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 10_000, false, LatePolicy::DropAndFlag));
        for samples in [928, 932].into_iter().chain((1_432..14_000).step_by(500)) {
            device_at(&core, &time, &device, txp_ticks(samples));
            txp_drain(&mut tx);
        }
        tx.reports(Wall::ZERO);
        let rows = lock(&core.rec).device_async.clone();
        assert!(rows.iter().all(|row| row["code"] != "Underflow" && row["code"] != "TimeError"), "{rows:?}");
        assert_eq!(rows.iter().filter(|row| row["code"] == "BurstAck").count(), 1, "{rows:?} {:?}", device.calls());
        let played: Vec<_> = (0..10_000).map(|i| Some([(i + 1) as f32 / 100_000.0, 3_000f32 / 10_000.0])).collect();
        assert_eq!(device.transmitted(clock.instant(3_000), 10_000), played, "{:?}", device.calls());
    }

    #[test]
    fn ur_22_a_repeat_of_one_to_five_samples_tiles_its_first_send() {
        // UR-22 (FS-6, S3): a repeat's first send is its waveform tiled to 32 payload samples; the window holds exactly 32.
        for len in [1usize, 3, 5] {
            let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(Some(3_032_000), 2_000);
            let mut burst = txp_burst(&clock, 3_000, len, true, LatePolicy::DropAndFlag);
            burst.tile_repeat();
            txp_give(&mut tx, burst);
            assert!(tx.step());
            assert_eq!(sends(&device), [(32, true, false)], "len {len}: {:?}", device.calls());
            let expected: Vec<Option<[f32; 2]>> = (0..32).map(|i| Some([((i % len) + 1) as f32 / 100_000.0, 3_000f32 / 10_000.0])).collect();
            assert_eq!(device.transmitted(clock.instant(3_000), 32), expected, "len {len}");
        }
    }

    #[test]
    fn ur_23_a_short_first_send_closes_before_a_booking_at_its_end() {
        // UR-22 (N3): a padded short first send is closed; a Kernel burst booked at its payload end is not continued into the padding.
        let (_, device, _time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 2, false, LatePolicy::DropAndFlag));
        txp_give(&mut tx, txp_burst(&clock, 3_002, 5, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device)[0], (4, true, true), "{:?}", device.calls());
        assert!(
            !device.calls().iter().any(|call| call.starts_with("tx_send") && call.contains(&format!("at={} ", clock.instant(3_002)))),
            "the booking at the padded end was continued: {:?}",
            device.calls()
        );
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "B at 3 002 is before the padded front 3 005 and is judged late: {events:?}");
        assert_eq!(late[0].payload["outcome"], "drop");
    }

    #[test]
    fn ur_23_a_four_sample_first_send_is_not_held() {
        // UR-22 (FS-1, N1): a first send of four samples goes at once, its last sample is not held back.
        let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 4, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(4, true, true)], "{:?}", device.calls());
    }

    #[test]
    fn ur_23_a_burst_of_31_or_32_is_sent_whole_without_a_held_tail() {
        // UR-22 (S3, #78): a finite burst of at most 32 samples is one closed device burst, with no sample held back.
        for len in [31, 32] {
            let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
            txp_give(&mut tx, txp_burst(&clock, 3_000, len, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            assert_eq!(sends(&device), [(len, true, true)], "len {len}: {:?}", device.calls());
            let played: Vec<_> = (0..len).map(|i| Some([(i + 1) as f32 / 100_000.0, 3_000f32 / 10_000.0])).collect();
            assert_eq!(device.transmitted(clock.instant(3_000), len), played, "len {len}");
        }
    }

    #[test]
    fn ur_23_a_33_sample_burst_sends_32_then_holds_one() {
        // UR-23 (S3, #78): a finite burst of 33 samples is not short: its first send is 32, and its last sample is held back to its end.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 6_000, 33, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(32, true, false)], "{:?}", device.calls());
        halt(&mut tx);
        assert_eq!(sends(&device), [(32, true, false), (2, false, true)], "{:?}", device.calls());
        let mut played: Vec<Option<[f32; 2]>> = (0..33).map(|i| Some([(i + 1) as f32 / 100_000.0, 6_000f32 / 10_000.0])).collect();
        played.push(Some([0.0, 0.0]));
        assert_eq!(device.transmitted(clock.instant(6_000), 34), played, "{:?}", device.calls());
    }

    #[test]
    fn ur_23_a_lone_burst_plays_every_sample_and_its_end_packet_is_its_last_sample() {
        // D74: a burst with nothing booked behind it plays all its samples; the held last sample goes with the end-of-burst packet.
        for len in [32usize, 33, 40] {
            let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
            txp_book(&mut tx, &core, clock, txp_burst(&clock, 5_000, len, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
            device_at(&core, &time, &device, core.join_by(clock, 5_000 + len as i64) + 1);
            txp_drain(&mut tx);
            let expected = if len == 32 { vec![(32, true, true)] } else { vec![(len - 1, true, false), (2, false, true)] };
            assert_eq!(sends(&device), expected, "len {len}: {:?}", device.calls());
            let eob_at = clock.instant(if len == 32 { 5_000 } else { 5_000 + len as i64 - 1 });
            assert!(device.calls().iter().any(|call| call.contains(&format!("at={eob_at} sob=")) && call.ends_with("eob=true")), "len {len}: {:?}", device.calls());
            assert_eq!(device.transmitted(clock.instant(5_000), len), played(5_000, len), "len {len}: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_23_a_burst_at_a_closed_short_burst_s_end_is_late_at_dispatch() {
        // UR-22 (S3, #78): a closed short burst's padded end plus the gap (15 samples at 1 MS/s) is its front, so a burst the owner is handed before it is late at
        // dispatch and one handed at it is on time. The record refuses such a booking at receipt (S2, pinned by `s3_parity_booking_after_a_closed_short_burst_follows_d5`);
        // this test gives the bursts without the record, so only the dispatch's verdict is tested.
        for len in [4usize, 10, 31, 32] {
            for (offset, late) in [(0i64, true), (14, true), (15, false)] {
                let (_, device, _time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
                txp_give(&mut tx, txp_burst(&clock, 3_000, len, false, LatePolicy::DropAndFlag));
                txp_give(&mut tx, txp_burst(&clock, 3_000 + len as i64 + offset, 4, false, LatePolicy::DropAndFlag));
                txp_drain(&mut tx);
                let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
                let errors: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
                if late {
                    assert_eq!(sends(&device), [(len, true, true)], "len {len}: {:?}", device.calls());
                    assert_eq!(errors.len(), 1, "len {len}: {events:?}");
                    assert_eq!(errors[0].payload["outcome"], "drop", "len {len}");
                } else {
                    assert_eq!(sends(&device), [(len, true, true), (4, true, true)], "len {len}: {:?}", device.calls());
                    assert!(errors.is_empty(), "len {len}: {events:?}");
                }
            }
        }
    }

    #[test]
    fn ur_23_a_31_sample_burst_waits_for_all_31() {
        // UR-22 (S3, #78): a 31-sample burst's first send waits for all 31 of its samples, not for 32. The window holds 30 at once, 31 at 1 sample.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(3_030_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 31, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "{:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(1));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(31, true, true)], "{:?}", device.calls());
    }

    #[test]
    fn ur_23_a_new_sob_after_an_abandon_waits_for_32_window_samples() {
        // UR-22 (S3, #78): after a failed first send, the next burst's first send waits for 32 samples of window space like any first send.
        let config = FakeConfig { clock: crate::ClockMode::Manual, faults: vec![crate::device::FakeFault::FailSend(0)], ..FakeConfig::default() };
        let (core, device, time, _events) = crate::provider::test_support::rig_with(config, Vec::new());
        let _frozen = crate::provider::test_support::Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        std::mem::forget(_to_tx);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        txp_give(&mut tx, txp_burst(&clock, 40_000, 1_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(32_031));
        txp_drain(&mut tx);
        let failed = sends(&device).len();
        txp_give(&mut tx, txp_burst(&clock, 42_000, 1_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device).len(), failed, "31 samples of window space were handed over: {:?}", device.calls());
        device_at(&core, &time, &device, txp_ticks(32_032));
        txp_drain(&mut tx);
        assert_eq!(sends(&device)[failed..], [(32, true, false)], "{:?}", device.calls());
    }

    #[test]
    fn ur_22_a_padded_send_counts_payload_only() {
        // UR-22 (FS-T4): the two samples of a padded first send are payload; its four device samples are not counted as payload.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 2, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(4, true, true)], "{:?}", device.calls());
        let rec = lock(&core.rec);
        assert_eq!(rec.stats.get("tx_samples"), Some(&2), "{:?}", rec.stats);
        assert_eq!(rec.bursts.len(), 1, "{:?}", rec.bursts);
        assert_eq!(rec.bursts[0]["samples"], 2, "{:?}", rec.bursts);
    }

    #[test]
    fn ur_22_a_two_channel_one_sample_burst_is_padded_on_both_channels() {
        // UR-22 (D6): a one-sample burst on a two-channel stream is one timed SOB+EOB of four per call, padded on each channel; its front is the padded end + the gap (3 000 samples at 200 MS/s).
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(2, Vec::new(), BTreeMap::new());
        let start = 500_000;
        for channel in 0..2 {
            device.apply(Dir::Rx, channel, &Settings { rate: Some(200e6), ..Settings::default() }).unwrap();
        }
        device.rx_open(0, &[0, 1]).unwrap();
        device.rx_start(0, clock.instant(start)).unwrap();
        device_at(&core, &time, &device, clock.instant(start - 400_000));
        let mut burst = txp_burst(&clock, start, 1, false, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![vec![[0.1, 0.0]], vec![[0.2, 0.0]]]);
        txp_give(&mut tx, burst);
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(4, true, true)], "{:?}", device.calls());
        assert_eq!(tx.chain.front(), Some((clock.domain, start + 4 + GAP_200MSPS)), "the padded end + the gap");
        device_at(&core, &time, &device, clock.instant(start + 4));
        let crate::device::RxRecv::Samples { samples, .. } = device.rx_recv(0, 4, Wall::from_millis(10), false) else { panic!("the loopback contains both channels") };
        assert_eq!(samples, [vec![[0.1, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], vec![[0.2, 0.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]], "both channels are padded with zeros after their sample");
    }

    #[test]
    fn ur_23_a_continuation_of_two_samples_is_sent_at_once_with_no_minimum() {
        // UR-23 (FS-7, TBD-b4): a continuation has no four-sample minimum: the two samples the window holds go at once. The window ends 2 samples after A's 7 000.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(Some(7_002_000), 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 3_000, 4_000, false, LatePolicy::DropAndFlag));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 7_000, 2, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 1, "one device start: {:?}", device.calls());
        assert_eq!(*sends(&device).last().unwrap(), (1, false, false), "B's first sample goes at once; its last is the held tail: {:?}", device.calls());
        assert_eq!(device.transmitted(clock.instant(7_000), 1), vec![Some([1.0 / 100_000.0, 7_000f32 / 10_000.0])]);
    }

    #[test]
    fn ur_23_a_32_sample_window_takes_a_first_send_of_32() {
        // UR-22 (N5, S3): a window that holds exactly 32 samples of the burst admits a first send of 32; the next chunk waits for the window to empty.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(1_000));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(32, true, false)], "{:?}", device.calls());
        assert!(device.transmitted(clock.instant(3_000), 32).iter().all(Option::is_some));
    }

    #[test]
    fn ur_28_grouped_channels_retire_per_position() {
        // UR-28 (F8-b): one stream with two channel positions: the ACK of position 0 retires only its own pending entry, and a duplicate ACK of 0 leaves position 1's.
        let (core, _, _, _events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(2));
        let burst = TimePoint::new(clock.domain, 10_000);
        tx.remember_burst(burst);
        let ack = |channel| TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(10_000) + 1), channel };
        tx.report(ack(0));
        assert_eq!(tx.unacked.iter().map(|pending| pending.channels.iter().copied().collect::<Vec<_>>()).collect::<Vec<_>>(), [vec![1]]);
        tx.report(ack(0));
        assert_eq!(tx.unacked.iter().map(|pending| pending.channels.iter().copied().collect::<Vec<_>>()).collect::<Vec<_>>(), [vec![1]], "a duplicate ACK retired position 1");
        tx.report(ack(1));
        assert!(tx.unacked.is_empty());
    }

    #[test]
    fn ur_28_split_streams_retire_their_own_pending() {
        // UR-28 (F8-c): two streams, one channel each: each stream's ACK retires its own pending entry and not the other's.
        let (core, _, _, _events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_a, cmds_a) = std::sync::mpsc::channel();
        let (_to_b, cmds_b) = std::sync::mpsc::channel();
        let mut a = Tx::new(core.clone(), 0, cmds_a, Some(clock), Channels::first(1));
        let mut b = Tx::new(core, 1, cmds_b, Some(clock), Channels::first(1));
        let first = TimePoint::new(clock.domain, 10_000);
        let second = TimePoint::new(clock.domain, 20_000);
        a.remember_burst(first);
        b.remember_burst(second);
        let ack = TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(10_000) + 1), channel: 0 };
        a.report(ack);
        assert!(a.unacked.is_empty(), "stream 0's own ACK retires its pending entry");
        assert_eq!(b.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [second], "stream 1 keeps its entry");
        b.report(ack);
        assert_eq!(b.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [second], "stream 0's ACK at 10 000 does not retire stream 1's burst at 20 000");
        b.report(TxReport { code: TxCode::BurstAck, tick: Some(clock.instant(20_000) + 1), channel: 0 });
        assert!(b.unacked.is_empty(), "stream 1's own ACK retires its entry");
    }

    #[test]
    fn ur_28_a_device_burst_over_two_kernel_bursts_keeps_both_records() {
        // UR-28, UR-23 (F8-d): B booked at A's end continues A's device burst: one pending correlation for the device burst's start, and both Kernel bursts in the record.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let b = continuation(&clock, 70_000, 1_000);
        give(&mut tx, b);
        device_at(&core, &time, &device, core.ticks(62_000_000));
        assert!(tx.step());
        device_at(&core, &time, &device, core.ticks(69_000_000));
        assert!(tx.step());
        assert_eq!(tx.unacked.len(), 1, "{:?}", tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>());
        let rec = lock(&core.rec);
        assert_eq!(rec.bursts.len(), 2, "{:?}", rec.bursts);
    }

    #[test]
    fn ur_22_a_first_send_is_never_undersized() {
        // UR-22 (F6-c, S3): every first send of one to nine samples, each its own closed device burst, reaches the device padded to four samples per channel; an open first send carries at least 32.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        for (index, len) in [1usize, 2, 3, 4, 5, 9].into_iter().enumerate() {
            let start = 3_000 + 10_000 * index as i64;
            device_at(&core, &time, &device, txp_ticks(start - 2_000));
            txp_give(&mut tx, txp_burst(&clock, start, len, false, LatePolicy::DropAndFlag));
            txp_drain(&mut tx);
        }
        let sent = sends(&device);
        assert_eq!(sent.iter().filter(|send| send.1).count(), 6, "{sent:?}");
        assert!(sent.iter().filter(|send| send.1).all(|send| (send.2 && send.0 >= 4) || send.0 >= 32), "{sent:?}");
    }

    #[test]
    fn ur_23_a_held_tail_is_timed_at_its_instant() {
        // UR-23 (F5-e): the held tail goes out at its own sample instant, not untimed.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        device_at(&core, &time, &device, core.ticks(68_000_000));
        txp_drain(&mut tx);
        assert!(sends(&device).ends_with(&[(2, false, true)]), "{:?}", device.calls());
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send") && call.contains(&format!("at={} ", clock.instant(69_999)))), "{:?}", device.calls());
    }

    #[test]
    fn ur_23_a_burst_at_the_end_of_a_closed_burst_is_late() {
        // UR-23 (F4-late): a burst booked at the closed device burst's exclusive end is late (gap 0), not continued.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        device_at(&core, &time, &device, core.ticks(68_000_000));
        txp_drain(&mut tx);
        assert!(tx.open_next().is_none() && tx.chain.tail().is_none(), "A's held sample did not go out at the deadline: {:?}", device.calls());
        let before = sends(&device);
        give(&mut tx, continuation(&clock, 70_000, 1_000));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), before, "a burst at the closed end was sent: {:?}", device.calls());
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"] == "UR-21: the late policy dropped the burst"), "{:?}", lock(&core.rec).rejected);
    }

    #[test]
    fn ur_23_a_short_burst_then_an_adjacent_burst_booked_after_dispatch_follows_the_late_policy() {
        // UR-22 (N3): a short first send is padded and closed at once; an adjacent burst booked after that dispatch is judged against the padded front.
        let (_, device, _time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 2, false, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert_eq!(sends(&device), [(4, true, true)], "{:?}", device.calls());
        txp_give(&mut tx, txp_burst(&clock, 3_002, 5, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(4, true, true)], "the booking at the padded end was continued: {:?}", device.calls());
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{events:?}");
        assert_eq!(late[0].payload["outcome"], "drop");
    }

    #[test]
    fn ur_23_stop_while_the_first_send_waits_sends_nothing() {
        // UR-22 (FS-3): a first send waiting for four samples of window space sends nothing once Stop cancels it.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_004_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(999));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "{:?}", device.calls());
        halt(&mut tx);
        assert_eq!(e1::notes(&core), ["UR-26: cancelled by Stop"], "#85: the open booking issued nothing");
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "a first send went after Stop: {:?}", device.calls());
    }

    #[test]
    fn ur_23_loss_while_the_first_send_waits_sends_nothing() {
        // UR-22 (FS-3): a device loss while a first send waits for window space sends nothing more.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_004_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(999));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "{:?}", device.calls());
        core.device_lost("test loss while the first send waits");
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "a first send went after the loss: {:?}", device.calls());
    }

    #[test]
    fn ur_23_a_wait_past_the_start_lead_follows_the_late_policy() {
        // UR-22 (FS-5): a first send that waited past its start is judged by its late policy before dispatch, not sent.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_004_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::DropAndFlag));
        device_at(&core, &time, &device, txp_ticks(999));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(3_001));
        txp_drain(&mut tx);
        assert!(sends(&device).is_empty(), "a late first send was handed over: {:?}", device.calls());
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{events:?}");
        assert_eq!(late[0].payload["outcome"], "drop");
    }

    #[test]
    fn ur_23_a_wait_past_the_start_lead_moves_a_send_asap_first_send() {
        // UR-21, UR-22 (FS-5, SendAsapAndFlag): a first send that waited past its start is moved to the cursor, not dropped; the moved send opens a device burst.
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, LatePolicy::SendAsapAndFlag));
        device_at(&core, &time, &device, txp_ticks(999));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(3_001));
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let late: Vec<_> = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
        assert_eq!(late.len(), 1, "{events:?}");
        assert_eq!(late[0].payload["outcome"], "send_asap", "{events:?}");
        assert!(sends(&device).first().is_some_and(|send| send.1), "the moved first send opens a device burst: {:?}", device.calls());
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
    }

    /// UR-21, UR-22 (C2): a first send that waits for window space is judged at its device lead. Window space for 32 samples
    /// appears at 1 000 samples; the start at 3 000 is then a device lead (2 000 samples) away, so `now_ticks` is the lead boundary
    /// or one tick past it. Returns the sends and the TIME_ERROR outcomes.
    fn first_send_at_its_lead(policy: LatePolicy, now_ticks: i64) -> (Vec<(usize, bool, bool)>, Vec<String>) {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(Some(2_032_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, false, policy));
        device_at(&core, &time, &device, txp_ticks(999));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, now_ticks);
        txp_drain(&mut tx);
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        let outcomes = events.iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).map(|event| event.payload["outcome"].as_str().unwrap().to_owned()).collect();
        (sends(&device), outcomes)
    }

    #[test]
    fn ur_23_a_first_send_one_tick_inside_its_lead_follows_drop_and_flag() {
        let (sends, outcomes) = first_send_at_its_lead(LatePolicy::DropAndFlag, txp_ticks(1_000) + 1);
        assert!(sends.is_empty(), "a first send inside its lead was handed over: {sends:?}");
        assert_eq!(outcomes, ["drop"]);
    }

    #[test]
    fn ur_23_a_first_send_at_its_lead_boundary_is_sent_by_drop_and_flag() {
        let (sends, outcomes) = first_send_at_its_lead(LatePolicy::DropAndFlag, txp_ticks(1_000));
        assert!(sends.first().is_some_and(|send| send.1), "{sends:?}");
        assert!(outcomes.is_empty(), "{outcomes:?}");
    }

    #[test]
    fn ur_23_a_first_send_one_tick_inside_its_lead_follows_send_asap_and_flag() {
        let (sends, outcomes) = first_send_at_its_lead(LatePolicy::SendAsapAndFlag, txp_ticks(1_000) + 1);
        assert_eq!(outcomes, ["send_asap"]);
        assert!(sends.first().is_some_and(|send| send.1), "the moved first send opens a device burst: {sends:?}");
    }

    #[test]
    fn ur_23_a_first_send_at_its_lead_boundary_is_sent_by_send_asap_and_flag() {
        let (sends, outcomes) = first_send_at_its_lead(LatePolicy::SendAsapAndFlag, txp_ticks(1_000));
        assert!(sends.first().is_some_and(|send| send.1), "{sends:?}");
        assert!(outcomes.is_empty(), "{outcomes:?}");
    }

    #[test]
    fn ur_23_a_long_burst_in_many_chunks_plays_contiguously() {
        // UR-23 (F5-d): a burst sent in many window-sized chunks plays every sample once, in place, as one device burst.
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_200_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 3_000, false, LatePolicy::DropAndFlag));
        for step in 0..600 {
            device_at(&core, &time, &device, txp_ticks(100 * step));
            txp_drain(&mut tx);
        }
        assert!(sends(&device).len() > 3, "the burst was not sent in chunks: {:?}", device.calls());
        assert_eq!(sends(&device).iter().filter(|send| send.1).count(), 1, "{:?}", device.calls());
        let expected: Vec<Option<[f32; 2]>> = (0..3_000).map(|i| Some([(i + 1) as f32 / 100_000.0, 3_000f32 / 10_000.0])).collect();
        assert_eq!(device.transmitted(clock.instant(3_000), 3_000), expected, "{:?}", device.calls());
    }

    #[test]
    fn ur_28_an_ack_at_a_bursts_end_plus_one_tick_keeps_the_burst_at_the_next_sample() {
        // UR-28 (N4): at 200 MS/s the BurstAck tick (exclusive end + 1 tick) is the next sample's instant; a burst starting there is not yet acknowledged.
        let (core, _, _, _events) = super::super::test_support::rig();
        let clock = core.register(Dir::Tx, 1, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut tx = Tx::new(core, 0, cmds, Some(clock), Channels::first(1));
        let first = TimePoint::new(clock.domain, 10_000);
        let next = TimePoint::new(clock.domain, 10_011);
        tx.remember_burst(first);
        tx.remember_burst(next);
        tx.report(crate::device::TxReport { code: crate::device::TxCode::BurstAck, tick: Some(clock.instant(10_010) + 1), channel: 0 });
        assert_eq!(tx.unacked.iter().map(|pending| pending.target).collect::<Vec<_>>(), [next]);
    }

    #[test]
    fn ur_23_m4c_a_stop_during_a_continuation_s_wait_ends_the_device_burst_once() {
        // The waiting state (an open burst still `first`, its device burst continued) meets Stop, with and without a held tail.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, txp_burst(&clock, 9_600, 4_000, false, LatePolicy::DropAndFlag));
        assert!(tx.step());
        assert!(!tx.step());
        let sent = sends(&device).len();
        halt(&mut tx);
        assert_eq!(sends(&device)[sent..], [(2, false, true)], "one two-zero end-of-burst and no more payload");
        assert!(tx.open_next().is_none() && tx.chain.next().is_none() && held_keys(&tx).is_empty());
        assert_eq!(e1::notes(&core), ["UR-26: cancelled by Stop"], "#85: the continuation issued nothing");
        // With a held tail (M9's state): the tail goes out as the device burst's last sample, with the end-of-burst.
        let (core, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 3_000, 6_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        assert!(!tx.step());
        let sent = sends(&device).len();
        halt(&mut tx);
        assert_eq!(sends(&device)[sent..], [(2, false, true)], "the held sample and one zero, with the end-of-burst");
        assert_eq!(device.transmitted(clock.instant(8_999), 2), vec![Some([6_000f32 / 100_000.0, 0.3]), Some([0.0, 0.0])]);
        assert!(tx.open_next().is_none() && tx.chain.tail().is_none() && held_keys(&tx).is_empty());
        assert_eq!(e1::notes(&core), ["UR-26: cancelled by Stop"], "#85: the continuation behind the held tail issued nothing");
    }

    #[test]
    fn ur_23_m4b_the_buffer_before_a_held_start_is_not_split() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        txp_give(&mut tx, txp_burst(&clock, 10_200, 4_000, false, LatePolicy::DropAndFlag));
        let calls = device.calls().len();
        assert!(!tx.step(), "2 200 samples to B's start, 2 000 of room");
        assert_eq!(device.calls().len(), calls);
        device_at(&core, &time, &device, txp_ticks(500));
        assert!(tx.step());
        assert_eq!(*sends(&device).last().unwrap(), (2_200, false, false), "A's last buffer ends at B's start, the device burst staying open");
        assert_eq!(tx.chain.next(), Some(10_200));
    }

    #[test]
    fn ur_23_m5_the_first_timed_buffer_takes_what_fits() {
        let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(Some(2_200_000), 2_000);
        txp_give(&mut tx, txp_burst(&clock, 2_000, 30_000, true, LatePolicy::DropAndFlag));
        assert!(tx.step(), "not a wait: the burst's start is its own lead away");
        assert_eq!(sends(&device), [(200, true, false)], "chunk 550, 200 samples of window");
        assert!(device.calls().iter().any(|call| call.contains(&format!("at={}", clock.instant(2_000)))));
        assert!(!tx.step(), "from here a whole buffer is waited for");
    }

    #[test]
    fn ur_23_m8_a_long_repeat_sends_full_chunks_as_the_window_advances() {
        let (core, device, time, events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        for ms in 1..=10 {
            device_at(&core, &time, &device, txp_ticks(ms * 1_000));
            txp_drain(&mut tx);
        }
        let sends = sends(&device);
        assert_eq!(sends.len(), 6, "two initial chunks, then one whenever a full 2 500-sample chunk fits");
        assert!(sends.iter().all(|s| s.0 == 2_500), "{sends:?}");
        assert!(lock(&core.rec).rejected.is_empty());
        assert!(events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TIME_ERROR));
    }

    #[test]
    fn ur_23_m9_a_waiting_continuation_leaves_the_held_tail_alone() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 3_000, 6_000, false, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false), (999, false, false)]);
        assert!(tx.chain.tail().is_some() && tx.chain.next() == Some(9_000));
        txp_book(&mut tx, &core, clock, txp_burst(&clock, 9_000, 4_000, false, LatePolicy::DropAndFlag));
        let calls = device.calls().len();
        assert!(!tx.step());
        assert_eq!(device.calls().len(), calls, "the tail went out ahead of a buffer that had to wait");
        assert!(tx.chain.tail().is_some());
        device_at(&core, &time, &device, txp_ticks(1_500));
        assert!(tx.step());
        let sends = sends(&device);
        assert_eq!(sends[3..], [(1, false, false), (2_500, false, false)], "the tail, then B, in order");
        assert_eq!(device.transmitted(clock.instant(8_999), 2), vec![Some([6_000f32 / 100_000.0, 0.3]), Some([1f32 / 100_000.0, 0.9])], "A's held last sample, then B's first");
    }

    #[test]
    fn ur_23_a_held_tails_keep_each_channel_sample() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_high_rate_rig(2, Vec::new(), BTreeMap::new());
        let start = 500_000;
        for channel in 0..2 {
            device.apply(Dir::Rx, channel, &Settings { rate: Some(200e6), ..Settings::default() }).unwrap();
        }
        device.rx_open(0, &[0, 1]).unwrap();
        device.rx_start(0, clock.instant(start)).unwrap();
        device_at(&core, &time, &device, clock.instant(start - 400_000));

        let a: Vec<Vec<[f32; 2]>> = (0..2).map(|c| (0..33).map(|i| [(i + 1) as f32 / 64.0, (c + 1) as f32 / 16.0]).collect()).collect();
        let mut first = one_shot(&clock, start, 33, a[0].clone());
        first.samples = Arc::new(a.clone());
        txp_give(&mut tx, first);
        assert!(tx.step());
        assert_eq!(sends(&device), [(32, true, false)]);

        let b: Vec<Vec<[f32; 2]>> = (0..2).map(|c| (0..2).map(|i| [(40 + i) as f32 / 64.0, (c + 3) as f32 / 16.0]).collect()).collect();
        let mut next = one_shot(&clock, start + 33, 2, b[0].clone());
        next.samples = Arc::new(b.clone());
        txp_give(&mut tx, next);
        assert!(tx.step());
        assert_eq!(sends(&device)[1..], [(1, false, false), (1, false, false)], "the first tail precedes the next burst payload");
        assert!(tx.chain.tail().is_some(), "the next burst's distinct per-channel tail is retained");
        halt(&mut tx);
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "Stop ends on the next burst's held tail and one zero");

        device_at(&core, &time, &device, clock.instant(start + 36));
        let crate::device::RxRecv::Samples { samples, .. } = device.rx_recv(0, 36, Wall::from_millis(10), false) else { panic!("the loopback contains both channel tails") };
        let expected: Vec<Vec<[f32; 2]>> = (0..2).map(|c| a[c].iter().chain(b[c].iter()).copied().chain([[0.0, 0.0]]).collect()).collect();
        assert_eq!(samples, expected, "continuation and EOB preserve each channel's held sample");
    }

    #[test]
    fn ur_23_m10_a_waiting_burst_whose_timeline_passed_is_abandoned() {
        let (core, device, time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        device_at(&core, &time, &device, txp_ticks(9_001));
        assert!(tx.step());
        assert!(lock(&core.rec).rejected.iter().any(|row| row["reason"].as_str().unwrap_or("").contains("UR-22: the transmit timeline expired")));
        assert_eq!(*sends(&device).last().unwrap(), (2, false, true), "a two-zero timed end-of-burst, not a late payload");
        assert_eq!(device.calls().last().map(String::as_str), Some(format!("tx_send 0 n=2 at={} sob=false eob=true", clock.instant(8_000)).as_str()));
        assert_eq!(device.transmitted(clock.instant(8_000), 2), vec![None, None], "a starved device does not play the late close");
    }

    #[test]
    fn ur_23_m11_tx_sizing_is_independent_of_receive_block_len() {
        let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 1);
        txp_give(&mut tx, txp_burst(&clock, 3_000, 5, true, LatePolicy::DropAndFlag));
        txp_drain(&mut tx);
        assert_eq!(tx.open_next().unwrap(), 8_000);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)], "RX block_len=1 does not cap TX");
    }

    #[test]
    fn ur_23_m12_the_count_is_samples_per_channel() {
        let (_, device, _time, _events, clock, mut tx, _frozen) = txp_rig(None, 2_000);
        tx.channels = Channels::first(2);
        device.tx_open(0, &[0, 1]).unwrap();
        let mut burst = txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag);
        burst.samples = Arc::new(vec![burst.samples[0].clone(), burst.samples[0].clone()]);
        txp_give(&mut tx, burst);
        txp_drain(&mut tx);
        assert_eq!(sends(&device), [(2_500, true, false), (2_500, false, false)]);
        assert_eq!(tx.open_next().unwrap(), 8_000);
    }

    // The priority of the transmit owner (UR-23) and the idle wait, on `Tx::run` threads over a `TxSpy`.

    /// `owners` transmit owners of the default layout on a `TxSpy`, each with a repeat burst (start 3 000) in its channel;
    /// `setup` sees the spy before they run. The owners are joined after each has sent its first buffer (a wait for an event,
    /// not a time bound), and the Provider's thread names are `uhd-tx-s<k>`.
    fn txp_owners(owners: usize, setup: impl FnOnce(&super::super::test_support::TxSpy)) -> (Arc<Core>, Arc<FakeDevice>, Arc<super::super::test_support::TxSpy>, Arc<EventCollector>) {
        let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let spy = Arc::new(super::super::test_support::TxSpy::new(inner.clone()));
        setup(&spy);
        let (core, time, events) = super::super::test_support::rig_on_description(spy.clone(), Vec::new(), BTreeMap::new(), super::super::test_support::default_description(2_000));
        let frozen = crate::provider::test_support::Frozen::of(&inner);
        device_at(&core, &time, &inner, 0);
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let host = core.device.host();
        let mut senders = Vec::new();
        let mut threads = Vec::new();
        for k in 0..owners {
            inner.tx_open(k, &[k]).unwrap();
            inner.apply(Dir::Tx, k, &Settings { rate: Some(1e6), ..Settings::default() }).unwrap();
            let (to_tx, cmds) = std::sync::mpsc::channel();
            let burst = txp_burst(&clock, 3_000, 30_000, true, LatePolicy::DropAndFlag);
            lock(&core.streams).stream_mut(stream_key(Dir::Tx, k)).book.list.book(burst);
            let tx = Tx::new(core.clone(), k, cmds, Some(clock), Channels::first(1));
            senders.push(to_tx);
            threads.push(host.spawn(&super::super::tx_name(k), move || tx.run()).expect("a participant thread"));
        }
        drop(frozen);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        for k in 0..owners {
            let name = super::super::tx_name(k);
            while !spy.recs().iter().any(|rec| rec.what == "tx_send" && rec.thread == name) {
                assert!(std::time::Instant::now() < deadline, "{name} sent nothing within 10 s: {:?}", spy.recs());
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        for to_tx in &senders {
            to_tx.send(TxCmd::Shutdown).unwrap();
        }
        for thread in threads {
            txp_join(thread);
        }
        (core, inner, spy, events)
    }

    fn txp_names(spy: &super::super::test_support::TxSpy, what: &str) -> Vec<String> {
        let mut names: Vec<String> = spy.recs().into_iter().filter(|rec| rec.what == what).map(|rec| rec.thread).collect();
        names.sort();
        names
    }

    #[test]
    fn ur_23_p1_p2_each_owner_raises_its_own_thread_once_before_its_first_send() {
        let (_core, _device, spy, _events) = txp_owners(2, |_| {});
        assert_eq!(txp_names(&spy, "raise_thread_priority"), ["uhd-tx-s0", "uhd-tx-s1"], "once per owner thread, from no other thread");
        let recs = spy.recs();
        for name in ["uhd-tx-s0", "uhd-tx-s1"] {
            let raised = recs.iter().position(|rec| rec.what == "raise_thread_priority" && rec.thread == name).expect("raised");
            let sent = recs.iter().position(|rec| rec.what == "tx_send" && rec.thread == name).expect("sent");
            assert!(raised < sent, "{name}: the priority goes before the first send: {recs:?}");
        }
    }

    #[test]
    fn ur_23_p3_every_start_raises_a_new_thread() {
        let (core, _device, spy, _events) = txp_owners(1, |_| {});
        // The same Core and spy, a second start: a new owner, a new thread, a second call.
        let (to_tx, cmds) = std::sync::mpsc::channel();
        to_tx.send(TxCmd::Shutdown).unwrap();
        let tx = Tx::new(core.clone(), 0, cmds, None, Channels::first(0));
        txp_join(core.device.host().spawn(&super::super::tx_name(0), move || tx.run()).unwrap());
        assert_eq!(txp_names(&spy, "raise_thread_priority"), ["uhd-tx-s0", "uhd-tx-s0"]);
    }

    #[test]
    fn ur_23_p4_a_refused_priority_is_recorded_and_the_burst_still_goes_out() {
        let (core, device, spy, events) = txp_owners(1, |spy| spy.refuse_priority(crate::DeviceError { lost: true, message: "uhd_set_thread_priority: UHD error 31: EPERM".to_owned() }, None));
        assert!(!core.is_lost(), "a refused priority is not a lost device");
        let rejected = lock(&core.rec).rejected.clone();
        let rows: Vec<_> = rejected.iter().filter(|row| row["call"] == "set_thread_priority").collect();
        assert_eq!(rows.len(), 1, "{rejected:?}");
        assert_eq!(rows[0]["stream"], 0);
        assert!(rows[0]["error"].as_str().unwrap().contains("EPERM"), "{rows:?}");
        assert!(!lock(&core.rec).stats.contains_key("tx_realtime"));
        assert!(device.calls().iter().any(|call| call.starts_with("tx_send ") && call.contains("sob=true")), "the burst's payload reached the device");
        assert!(txp_names(&spy, "tx_send").iter().all(|name| name == "uhd-tx-s0"), "the payload went out on the owner's thread");
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(
            events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED && event.kind.as_str() != EventKind::DEVICE_LOST),
            "a refused priority is a row, not an event: {events:?}"
        );
    }

    #[test]
    fn ur_23_p5_p6_the_success_is_counted_per_owner() {
        let (core, _device, _spy, _events) = txp_owners(2, |_| {});
        assert_eq!(lock(&core.rec).stats.get("tx_realtime"), Some(&2));
        assert!(lock(&core.rec).rejected.iter().all(|row| row["call"] != "set_thread_priority"));
        let (core, _device, _spy, _events) = txp_owners(2, |spy| spy.refuse_priority(crate::DeviceError::failed("only the first"), Some("uhd-tx-s0")));
        assert_eq!(lock(&core.rec).stats.get("tx_realtime"), Some(&1));
        let rejected = lock(&core.rec).rejected.clone();
        let rows: Vec<_> = rejected.iter().filter(|row| row["call"] == "set_thread_priority").collect();
        assert_eq!(rows.len(), 1, "{rejected:?}");
        assert_eq!(rows[0]["stream"], 0);
    }

    /// An owner of the 100 kS/s transmit stream on a `TxSpy`: a 50 ms window (5 000 samples, buffers of 1 250) and a repeat
    /// burst at sample 1 250 in its channel, which fills the window with three buffers: the next one is 12.5 ms away.
    #[allow(clippy::type_complexity)]
    fn txp_idle_owner() -> (Arc<Core>, Arc<FakeDevice>, Arc<super::super::test_support::TxSpy>, std::sync::mpsc::Sender<TxCmd>, Tx, crate::provider::test_support::Frozen) {
        let inner = Arc::new(FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let spy = Arc::new(super::super::test_support::TxSpy::new(inner.clone()));
        let mut description = super::super::test_support::default_description(2_000);
        description.timing.tx_in_flight_ns = 50_000_000;
        let (core, time, _events) = super::super::test_support::rig_on_description(spy.clone(), Vec::new(), BTreeMap::new(), description);
        let frozen = crate::provider::test_support::Frozen::of(&inner);
        device_at(&core, &time, &inner, 0);
        inner.tx_open(0, &[0]).unwrap();
        inner.apply(Dir::Tx, 0, &Settings { rate: Some(1e5), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 2_000, 0).unwrap();
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let burst = txp_burst(&clock, 1_250, 5_000, true, LatePolicy::DropAndFlag);
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.book(burst);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        (core, inner, spy, to_tx, tx, frozen)
    }

    const TXP_IDLE: std::time::Duration = std::time::Duration::from_micros(200);

    #[test]
    fn ur_23_m8b_a_waiting_owner_sleeps_an_idle_period_between_looks() {
        let (core, _device, spy, to_tx, tx, frozen) = txp_idle_owner();
        let (snapshots, release) = spy.trap_idle(3, 5);
        let owner = core.device.host().spawn("tx-idle", move || tx.run()).expect("a participant thread");
        drop(frozen);
        // The owner's own thread takes the snapshot at its fifth idle look and waits on a channel (the clock stands still);
        // this thread only receives it, queues the Shutdown and lets it go.
        let snapshot = snapshots.recv_timeout(std::time::Duration::from_secs(10)).expect("five idle looks");
        to_tx.send(TxCmd::Shutdown).unwrap();
        release.send(()).unwrap();
        txp_join(owner);
        let sends = snapshot.iter().filter(|rec| rec.what == "tx_send").count();
        assert_eq!(sends, 3, "no payload while it waits: {snapshot:?}");
        let idle: Vec<_> =
            snapshot.iter().enumerate().filter(|(i, rec)| rec.what == "tx_async" && snapshot[..*i].iter().filter(|r| r.what == "tx_send").count() >= 3).map(|(_, rec)| rec.host).collect();
        assert_eq!(idle.len(), 5, "{snapshot:?}");
        // Elapsed since the third payload send, not an absolute time: the test thread may stand still before the owner starts.
        let third = snapshot.iter().filter(|rec| rec.what == "tx_send").nth(2).expect("the third send").host;
        assert!(*idle.last().unwrap() - third < std::time::Duration::from_micros(12_500), "the snapshot is long before a buffer fits (12.5 ms): {idle:?} after {third:?}");
        for pair in idle.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= TXP_IDLE && gap <= 5 * TXP_IDLE, "a sleeping wait looks again an idle period later, not at once (a spin) and not a window later: {gap:?} {idle:?}");
        }
    }

    #[test]
    fn ur_23_m6_a_waiting_owner_wakes_for_a_stop_within_an_idle_period() {
        let (core, _device, spy, to_tx, tx, frozen) = txp_idle_owner();
        let host = core.device.host();
        let (woken, woken_rx) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let owner = host
            .spawn("tx-stop", move || {
                crate::host::pause_next_wake(woken, released);
                tx.run();
            })
            .expect("a participant thread");
        drop(frozen);
        // The owner's first wake, held with its deadline: the clock reads that deadline, however late this thread runs.
        woken_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the owner's first idle wake");
        let woke = host.now();
        let recs = spy.recs();
        assert_eq!(recs.iter().filter(|rec| rec.what == "tx_send").count(), 3, "{recs:?}");
        let looked = recs.last().expect("a look");
        assert_eq!(looked.what, "tx_async", "the owner went to sleep after a look: {recs:?}");
        let slept = woke - looked.host;
        assert!(slept >= TXP_IDLE && slept < 3 * TXP_IDLE, "the wait sleeps an idle period, not until a buffer fits: {slept:?}");
        lock(&core.streams).stream_mut(stream_key(Dir::Tx, 0)).book.list.halt(ezsdr_radio::CancelReason::Stop);
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let sends = loop {
            let sends: Vec<_> = spy.recs().into_iter().filter(|rec| rec.what == "tx_send").collect();
            if sends.len() > 3 {
                break sends;
            }
            assert!(std::time::Instant::now() < deadline, "the Stop was not acted on within 10 s: {:?}", spy.recs());
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!((sends[3].n, sends[3].eob), (2, true), "the Stop ends the device burst with two zeros before any more payload: {sends:?}");
        to_tx.send(TxCmd::Shutdown).unwrap();
        txp_join(owner);
        assert_eq!(spy.recs().iter().filter(|rec| rec.what == "tx_send").count(), 4, "no payload after the Stop");
    }

    const F1: f64 = 1.0e9;
    const F2: f64 = 2.0e9;
    const F3: f64 = 3.0e9;

    /// A burst of `len` samples at sample `k` on each of `channels`, whose values are `frequency` on every channel at
    /// `generation`: the snapshot the Control takes at booking (R3).
    fn valued(clock: &Clock, k: i64, len: usize, id: u64, generation: u64, frequency: f64, channels: usize) -> Held {
        let base = (0..channels).map(|chan| (crate::provider::core::key(&format!("radio.tx.c{chan}.frequency_hz")), ezsdr_kernel::spec::Value::num(frequency).unwrap())).collect();
        let mut burst = one_shot(clock, k, channels, vec![[0.25, 0.0]; len]);
        burst.settings = Arc::new(super::ColdConfig::of_tx(base, ActionId(id), generation));
        burst
    }

    /// The owner on transmit stream 0's clock with `channels` channels, as `arm` leaves it: the connect values (F1) are on the
    /// device and the stream's mark is generation 0 (M1). The sender is the stream's command channel.
    type OwnerRig = (Arc<Core>, Arc<FakeDevice>, Arc<ezsdr_kernel::time::ManualTimeAuthority>, Tx, Frozen, Arc<ezsdr_kernel::event::EventCollector>, std::sync::mpsc::Sender<TxCmd>);

    fn owner(channels: usize) -> OwnerRig {
        owner_with(channels, Vec::new())
    }

    fn owner_with(channels: usize, faults: Vec<FakeFault>) -> OwnerRig {
        let config = FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() };
        let (core, device, time, events) = super::super::test_support::rig_with(config, Vec::new());
        let frozen = Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &(0..channels).collect::<Vec<_>>()).unwrap();
        for chan in 0..channels {
            device.apply(Dir::Tx, chan, &Settings { rate: Some(1e6), freq: Some(F1), ..Settings::default() }).unwrap();
        }
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        lock(&core.streams).record_configured(stream_key(Dir::Tx, 0), Some(0));
        let (to_tx, cmds) = std::sync::mpsc::channel();
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(channels as u32));
        (core, device, time, tx, frozen, events, to_tx)
    }

    /// The device time of each transmit apply the owner makes, read by the `apply` probe.
    fn record_applies(core: &Core, device: &Arc<FakeDevice>) -> Arc<std::sync::Mutex<Vec<i64>>> {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (device, log) = (device.clone(), seen.clone());
        super::super::test_support::probe(core, "apply", move |_| log.lock().unwrap().push(device.time_now().unwrap()));
        seen
    }

    /// The owner's turns from `from_ms` to `until_ms` in `step_ms` steps of device time. Each pass drains the commands first, as
    /// the run loop does.
    fn turns(core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, device: &FakeDevice, tx: &mut Tx, from_ms: i64, until_ms: i64, step_ms: i64) {
        for ms in (from_ms..=until_ms).step_by(step_ms as usize) {
            device_at(core, time, device, core.ticks(ms * 1_000_000));
            for _ in 0..8 {
                drain_tx(tx);
                if !tx.step() {
                    break;
                }
            }
        }
    }

    /// The refusals the Provider has reported since the last call.
    fn rejected(events: &ezsdr_kernel::event::EventCollector) -> Vec<ezsdr_kernel::event::Event> {
        events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).collect()
    }

    /// Books a value of the transmit frequency on channel 0 (R2): the Control's receipt path.
    fn book_frequency(control: &mut Control, core: &Core, id: u64, frequency: f64) {
        let action = Action::UpdateParameter {
            target: super::super::test_support::target_of(&core.id),
            key: crate::provider::core::key("radio.tx.c0.frequency_hz"),
            value: ezsdr_kernel::spec::Value::num(frequency).unwrap(),
            class: ezsdr_kernel::module_api::UpdateClass::Cold,
            at: None,
        };
        control.book(Dispatched::new(ActionId(id), action, Some(core.id.clone())));
    }

    /// The calls after the test's setup: `tx_open` and one connect apply per channel.
    fn traffic(device: &FakeDevice, channels: usize) -> Vec<String> {
        device.calls().into_iter().skip(1 + channels).collect()
    }

    fn sent_before(calls: &[String], index: usize) -> bool {
        calls[..index].iter().any(|call| call.starts_with("tx_send "))
    }

    #[test]
    fn ur_23_apply_is_due_only_between_bursts_and_only_for_a_new_generation() {
        // S9 (R4): each input of the guard changes the answer alone.
        let due = |mark, device_open, now, end, recovering| super::apply_due(mark, 1, device_open, now, end, recovering);
        assert!(due(Some(0), false, 100, Some(100), false), "at the owner's last end, for a new generation");
        assert!(due(Some(0), false, 101, None, false), "no earlier device burst to wait for");
        assert!(due(None, false, 100, Some(100), false), "a mark of None is a generation the device does not hold");
        assert!(!due(Some(1), false, 100, Some(100), false), "the device holds the generation");
        assert!(!due(Some(0), true, 100, Some(100), false), "a device burst is open");
        assert!(!due(Some(0), false, 99, Some(100), false), "before the owner's last end");
        assert!(!due(Some(0), false, 100, Some(100), true), "during a recovery");
    }

    #[test]
    fn ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply() {
        // M1: the connect values are generation 0, which arm publishes, so the first burst with no new value applies nothing.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 4, 1, 0, F1, 1));
        turns(&core, &time, &device, &mut tx, 1, 40, 1);
        assert!(applies.lock().unwrap().is_empty(), "no apply for an unchanged generation: {:?}", traffic(&device, 1));
        assert!(sends(&device).iter().any(|(_, _, eob)| *eob), "the burst is sent: {:?}", traffic(&device, 1));
    }

    #[test]
    fn ur_25_a_value_after_a_burst_ends_is_applied_before_the_next_burst() {
        // S1 (R4): A plays the connect values; B's new generation is applied after A's last sample and before B's first send.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 4, 1, 0, F1, 1));
        give(&mut tx, valued(&clock, 40_000, 4, 2, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 60, 1);
        let calls = traffic(&device, 1);
        let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("B's value is applied");
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply tx")).count(), 1, "{calls:?}");
        assert!(sent_before(&calls, apply), "A is sent before the apply: {calls:?}");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send ")), "B is sent after the apply: {calls:?}");
        assert!(applies.lock().unwrap()[0] >= clock.instant(20_004), "the apply waits for A's end: {:?}", applies.lock().unwrap());
        assert_eq!(device.read_back(Dir::Tx, 0).freq, Some(F2));
    }

    #[test]
    fn ur_23_a_burst_at_a_held_tail_with_a_new_generation_is_configured_before_it() {
        // S1 (R4): a burst booked at a held tail's end with a new generation is not a continuation of the device burst, which
        // would play the old values: the device burst ends, the configure runs, and only then does the burst play.
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let applies = record_applies(&core, &device);
        give(&mut tx, valued(&clock, 70_000, 4, 2, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 62, 90, 1);
        let applies = applies.lock().unwrap().clone();
        assert_eq!(applies.len(), 1, "the new generation is configured: {:?}", device.calls());
        assert!(applies[0] >= clock.instant(70_000), "the configure waits for the held tail's end: {applies:?}");
        assert_eq!(device.read_back(Dir::Tx, 0).freq, Some(F2));
    }

    #[test]
    fn ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call() {
        // S9 (F-24): a contiguous burst of the same generation continues the device burst; no apply is made mid-burst.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 40, 1, 1, F2, 1));
        give(&mut tx, valued(&clock, 20_040, 40, 2, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 60, 1);
        let calls = traffic(&device, 1);
        assert_eq!(applies.lock().unwrap().len(), 1, "one configure, before the first burst: {calls:?}");
        let sent = sends(&device);
        assert_eq!((sent.iter().filter(|(_, sob, _)| *sob).count(), sent.iter().filter(|(_, _, eob)| *eob).count()), (1, 1), "one device burst: {sent:?}");
        assert!(sent.iter().map(|(n, _, _)| *n).sum::<usize>() >= 80, "both bursts are sent: {sent:?}");
    }

    #[test]
    fn ur_25_an_equal_value_after_a_new_one_configures_once() {
        // Sol 7: a new generation with an equal value configures; a later burst of that generation configures nothing.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F1, 1));
        turns(&core, &time, &device, &mut tx, 1, 30, 1);
        assert_eq!(applies.lock().unwrap().len(), 1, "the new generation is configured: {:?}", traffic(&device, 1));
        give(&mut tx, valued(&clock, 60_000, 4, 2, 1, F1, 1));
        turns(&core, &time, &device, &mut tx, 31, 80, 1);
        assert_eq!(applies.lock().unwrap().len(), 1, "the same generation configures nothing: {:?}", traffic(&device, 1));
        assert_eq!(sends(&device).iter().filter(|(_, _, eob)| *eob).count(), 2, "{:?}", traffic(&device, 1));
    }

    #[test]
    fn ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock() {
        // S7 (R5, M3): a refused configure refuses its burst with its Action id; nothing is sent, the stream is not halted, the mark
        // is None, and the next burst configures again while the refused value stays; a later value (F3) ends the refusal.
        let (core, device, time, mut tx, _frozen, events, _to_tx) = owner_with(1, vec![FakeFault::TxApplyRefused(0, F2)]);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 15, 1);
        let drained = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert_eq!(drained.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(), [ezsdr_radio::kinds::COMMAND_REJECTED], "no other event: the stream is not halted");
        assert_eq!(drained[0].payload["kind"], "tx_burst");
        assert_eq!(drained[0].payload["action"], 1);
        assert!(!traffic(&device, 1).iter().any(|call| call.starts_with("tx_send ")), "nothing is sent: {:?}", traffic(&device, 1));
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, None);
        assert!(!core.is_lost());
        give(&mut tx, valued(&clock, 40_000, 4, 2, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 16, 35, 1);
        assert_eq!(rejected(&events).iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(2)]);
        assert_eq!(traffic(&device, 1).iter().filter(|call| call.starts_with("apply tx")).count(), 2, "the retry configures again: {:?}", traffic(&device, 1));
        give(&mut tx, valued(&clock, 60_000, 4, 3, 2, F3, 1));
        turns(&core, &time, &device, &mut tx, 36, 80, 1);
        assert!(rejected(&events).is_empty(), "{:?}", traffic(&device, 1));
        assert_eq!(device.read_back(Dir::Tx, 0).freq, Some(F3));
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, Some(2));
    }

    #[test]
    fn ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both() {
        // S7 partial (R4, R5): channel 0 applies and channel 1 is refused: the mark is None, nothing is sent, and the retry
        // configures both channels with a corrected value.
        let (core, device, time, mut tx, _frozen, events, _to_tx) = owner_with(2, vec![FakeFault::TxApplyRefused(1, F2)]);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 2));
        turns(&core, &time, &device, &mut tx, 1, 15, 1);
        assert_eq!(rejected(&events).len(), 1, "{:?}", traffic(&device, 2));
        assert_eq!(traffic(&device, 2).iter().filter(|call| call.starts_with("apply tx")).count(), 2, "{:?}", traffic(&device, 2));
        assert!(!traffic(&device, 2).iter().any(|call| call.starts_with("tx_send ")), "{:?}", traffic(&device, 2));
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, None);
        give(&mut tx, valued(&clock, 40_000, 4, 2, 1, F3, 2));
        turns(&core, &time, &device, &mut tx, 16, 40, 1);
        assert_eq!(traffic(&device, 2).iter().filter(|call| call.starts_with("apply tx")).count(), 4, "the retry applies both channels: {:?}", traffic(&device, 2));
        assert_eq!(device.read_back(Dir::Tx, 1).freq, Some(F3));
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, Some(1));
    }

    #[test]
    fn ur_21_a_refused_burst_stays_counted_until_its_booked_end() {
        // F9 (P4): a burst refused at its configure stays counted in the stream's Bookings to its booked end: a value before the end is
        // refused, and a value at it is taken.
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults: vec![FakeFault::TxApplyRefused(0, F2)], ..FakeConfig::default() }));
        let (wave, bytes) = waveform_input("a", &[[0.25, 0.0]; 4]);
        let (one, one_bytes) = waveform_input("a1", &[[0.5, 0.0]; 1]);
        let inputs = BTreeMap::from([(wave.hash.clone(), Arc::<[u8]>::from(bytes)), (one.hash.clone(), Arc::<[u8]>::from(one_bytes))]);
        let (core, time, events) = super::super::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let _frozen = Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), freq: Some(F1), ..Settings::default() }).unwrap();
        let clock = core.register(Dir::Tx, 200, 0).unwrap();
        let (_to_tx, cmds) = std::sync::mpsc::channel();
        let mut config = core.description.defaults();
        config.insert(crate::provider::core::key("radio.tx.c0.stream"), ezsdr_kernel::spec::Value::from(0));
        let mut start = super::super::control::Start { t0: clock.origin, rx: vec![None; core.description.stream_count(Dir::Rx)], tx: vec![None; core.description.stream_count(Dir::Tx)] };
        start.tx[0] = Some((clock, Channels::first(1)));
        let mut control = Control::new(core.clone(), Arc::new(NoActions), config, false, start);
        let mut tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        book_frequency(&mut control, &core, 1, F2);
        book_burst_on(&mut control, &core, 2, 0, clock, 20_000, &wave, false, LatePolicy::SendAsapAndFlag);
        turns(&core, &time, &device, &mut tx, 1, 15, 1);
        assert_eq!(rejected(&events).iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(2)], "{:?}", lock(&core.rec).rejected);
        book_burst_on(&mut control, &core, 5, 0, clock, 20_002, &one, false, LatePolicy::DropAndFlag);
        assert!(rejected(&events).is_empty(), "the refused burst's span is free: {:?}", lock(&core.rec).rejected);
        device_at(&core, &time, &device, core.ticks(19_000_000));
        book_frequency(&mut control, &core, 3, F3);
        assert_eq!(rejected(&events).iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(), [serde_json::json!(3)], "before the booked end");
        device_at(&core, &time, &device, clock.instant(20_003));
        book_frequency(&mut control, &core, 6, F3);
        assert_eq!(
            rejected(&events).iter().map(|event| event.payload["action"].clone()).collect::<Vec<_>>(),
            [serde_json::json!(6)],
            "the refused burst's end (20 004) is past the accepted burst's (20 003)"
        );
        device_at(&core, &time, &device, clock.instant(20_004));
        book_frequency(&mut control, &core, 4, F3);
        assert!(rejected(&events).is_empty(), "at the booked end the value is taken");
    }

    #[test]
    fn ur_22_a_stop_during_the_configure_leaves_no_payload_after_it() {
        // M2: a Stop that arrives while the configure is paused does not abort it; the configure completes and publishes its
        // generation, and the next turn drains the Stop and cancels the burst before any send.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let clock = tx.clock.expect("a clock");
        let stopper = core.clone();
        super::super::test_support::probe(&core, "tx_configure_before_mark", move |_| {
            stopper.streams.lock().unwrap().stream_mut(stream_key(Dir::Tx, 0)).book.list.halt(ezsdr_radio::CancelReason::Stop)
        });
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 30, 1);
        assert!(!traffic(&device, 1).iter().any(|call| call.starts_with("tx_send ")), "no payload after the Stop: {:?}", traffic(&device, 1));
        assert_eq!(device.read_back(Dir::Tx, 0).freq, Some(F2), "the configure completed");
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, Some(1));
        assert!(lock(&core.rec).rejected.iter().any(|rejection| rejection["reason"] == "UR-26: cancelled by Stop"), "{:?}", lock(&core.rec).rejected);
        assert!(held_keys(&tx).is_empty());
    }

    #[test]
    fn ur_22_a_loss_during_the_configure_clears_the_mark() {
        // S10 (Sol 4): a loss during the configure keeps the mark None: the configure's publication is not made after the loss.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let clock = tx.clock.expect("a clock");
        let lost = core.clone();
        super::super::test_support::probe(&core, "tx_configure_before_mark", move |_| lost.device_lost("test: the device is lost during the configure"));
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 15, 1);
        assert!(core.is_lost());
        assert_eq!(lock(&core.streams).stream(stream_key(Dir::Tx, 0)).configured, None);
        assert!(!traffic(&device, 1).iter().any(|call| call.starts_with("tx_send ")), "{:?}", traffic(&device, 1));
    }

    /// The owner's turns at each device instant of `ns` (nanoseconds of device time), each with its steps.
    fn turns_at(core: &Core, time: &ezsdr_kernel::time::ManualTimeAuthority, device: &FakeDevice, tx: &mut Tx, ns: &[i64]) {
        for &at in ns {
            device_at(core, time, device, core.ticks(at));
            for _ in 0..8 {
                drain_tx(tx);
                if !tx.step() {
                    break;
                }
            }
        }
    }

    #[test]
    fn ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end() {
        // Owner guard (a): A is booked to end at 20 004 and is dispatched late, moved to 21 000. B's value waits for A's moved
        // front (its device burst closes at 21 004 and fronts at 21 019, UR-22), not for the booked end, though A's device burst has already closed at 19 ms.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        let mut moved = valued(&clock, 20_000, 4, 1, 0, F1, 1);
        moved.policy = LatePolicy::SendAsapAndFlag;
        give(&mut tx, moved);
        give(&mut tx, valued(&clock, 25_000, 4, 2, 1, F2, 1));
        turns_at(&core, &time, &device, &mut tx, &[19_000_000, 19_500_000, 20_500_000, 21_004_000, 21_005_000, 21_006_000, 21_018_000, 21_020_000, 21_021_000]);
        let calls = traffic(&device, 1);
        let applied = applies.lock().unwrap().clone();
        assert_eq!(applied.len(), 1, "{calls:?}");
        assert!(applied[0] >= clock.instant(21_019), "the value waits for A's moved front: {applied:?} {calls:?}");
        assert_eq!(sends(&device).first(), Some(&(4, true, true)), "{calls:?}");
        let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("B's value is applied");
        assert!(sent_before(&calls, apply), "A is sent before the apply: {calls:?}");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send ")), "B is sent after the apply: {calls:?}");
    }

    #[test]
    fn ur_23_a_value_waits_for_a_padded_one_sample_burst() {
        // Owner guard (b): a one-sample burst at 20 000 is sent padded to four samples, a device burst that ends at 20 004 and fronts
        // at 20 019 (UR-22); B's value waits for that front, not for the payload's end at 20 001.
        let (core, device, time, mut tx, _frozen, _events, _to_tx) = owner(1);
        let applies = record_applies(&core, &device);
        let clock = tx.clock.expect("a clock");
        give(&mut tx, valued(&clock, 20_000, 1, 1, 0, F1, 1));
        give(&mut tx, valued(&clock, 25_000, 4, 2, 1, F2, 1));
        turns_at(&core, &time, &device, &mut tx, &[17_000_000, 17_500_000, 19_500_000, 20_004_000, 20_005_000, 20_006_000, 20_018_000, 20_020_000]);
        let calls = traffic(&device, 1);
        let applied = applies.lock().unwrap().clone();
        assert_eq!(applied.len(), 1, "{calls:?}");
        assert!(applied[0] >= clock.instant(20_019), "the value waits for the padded front: {applied:?} {calls:?}");
        assert_eq!(sends(&device).first(), Some(&(4, true, true)), "A is padded to four: {calls:?}");
        let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("B's value is applied");
        assert!(sent_before(&calls, apply), "A is sent before the apply: {calls:?}");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send ")), "B is sent after the apply: {calls:?}");
    }

    #[test]
    fn ur_23_a_value_waits_for_a_stop_s_held_tail() {
        // Owner guard (c) (m9): after a Stop, A's held sample and the two-sample close end the device burst at 70 001 and it fronts at 70 004
        // (close_start + 5). A value received at 70 000 (the Stop's bound) waits for that front: no apply in [70 000, 70 004).
        let (core, device, time, mut tx, _frozen) = at_a_held_tail(Vec::new());
        let clock = tx.clock.expect("a transmit clock");
        let applies = record_applies(&core, &device);
        device_at(&core, &time, &device, core.ticks(63_000_000));
        halt(&mut tx);
        give(&mut tx, valued(&clock, 75_000, 4, 2, 1, F2, 1));
        turns_at(&core, &time, &device, &mut tx, &[70_000_000, 70_001_000, 70_003_000, 70_004_000, 70_005_000]);
        let calls = traffic(&device, 2);
        let applied = applies.lock().unwrap().clone();
        assert_eq!(applied.len(), 1, "{calls:?}");
        assert!(applied[0] >= clock.instant(70_004), "the value waits for the close's front: {applied:?} {calls:?}");
        let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("the value is applied");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send ")), "the burst is sent after the apply: {calls:?}");
    }

    #[test]
    fn ur_25_a_slow_configure_makes_a_burst_late_by_its_policy() {
        // S8 (R4): a configure that runs until 23 ms makes the burst booked at 20 ms late when its turn comes after the configure.
        // The late policy decides then: SendAsap moves it and sends it after the apply; Drop drops it, reported, with no send.
        for (policy, outcome) in [(LatePolicy::SendAsapAndFlag, "send_asap"), (LatePolicy::DropAndFlag, "drop")] {
            let (core, device, time, mut tx, _frozen, events, _to_tx) = owner(1);
            let clock = tx.clock.expect("a clock");
            let (slow_core, slow_time, slow_device) = (core.clone(), time.clone(), device.clone());
            super::super::test_support::probe(&core, "apply", move |_| device_at(&slow_core, &slow_time, &slow_device, slow_core.ticks(23_000_000)));
            let mut burst = valued(&clock, 20_000, 4, 1, 1, F2, 1);
            burst.policy = policy;
            give(&mut tx, burst);
            turns_at(&core, &time, &device, &mut tx, &[19_000_000]);
            let calls = traffic(&device, 1);
            let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("the configure runs");
            assert!(!sent_before(&calls, apply), "{policy:?}: no send before the configure: {calls:?}");
            let timed: Vec<_> =
                events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).collect();
            assert!(timed.iter().any(|event| event.payload["outcome"] == outcome), "{policy:?}: {timed:?} {calls:?}");
            // The lateness is read after the configure ended at 23 ms: at least 3 ms past the booked 20 ms, not the 19 ms decision.
            let late_by_ns = timed[0].payload["late_by_ns"].as_i64().expect("a lateness");
            assert!(late_by_ns >= 3_000_000, "{policy:?}: the decision after the configure: {late_by_ns} ns {timed:?}");
            assert_eq!(calls.iter().any(|call| call.starts_with("tx_send ")), outcome == "send_asap", "{policy:?}: {calls:?}");
        }
    }

    #[test]
    fn ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst() {
        // M2 (UR-25, UR-26): Provider::stop between the two channels' applies. Both are applied, so the mark is Some(1); the
        // claim is back; the burst is cancelled with ProviderStop; no payload follows.
        let (core, device, time, mut tx, frozen, events, to_tx) = owner(2);
        let clock = tx.clock.expect("a clock");
        let key = stream_key(Dir::Tx, 0);
        let stopper = core.clone();
        let applies_seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        super::super::test_support::probe(&core, "apply", move |_| {
            if applies_seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                let at = stopper.now();
                let mut streams = lock(&stopper.streams);
                streams.stop = Some((at, StopMode::Orderly));
                streams.end(&stopper, at, false, false);
                stopper.release_streams(streams);
                to_tx.send(TxCmd::Shutdown).unwrap();
            }
        });
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 2));
        device_at(&core, &time, &device, core.ticks(15_000_000));
        assert!(tx.step(), "the configure turn");
        assert!(matches!(tx.cmds.try_recv(), Ok(TxCmd::Shutdown)), "Provider::stop sent its Shutdown during the configure");
        assert_eq!(lock(&core.streams).stream(key).configured, Some(1), "both channels are applied, so the mark is published");
        {
            let mut streams = lock(&core.streams);
            let claim = streams.claim_owner(key).expect("the configure's claim is released");
            streams.give_back(claim);
        }
        drop(frozen);
        let host = core.device.host();
        let driver_host = host.clone();
        let driver = host.spawn("tx-shutdown-deadline", move || driver_host.sleep(Wall::from_millis(150))).expect("a manual-clock participant");
        tx.shutdown();
        driver.join().unwrap();
        assert!(!traffic(&device, 2).iter().any(|call| call.starts_with("tx_send ")), "no payload after Provider::stop: {:?}", traffic(&device, 2));
        assert!(held_keys(&tx).is_empty(), "the burst is cancelled");
        let reasons: Vec<_> = lock(&core.rec).rejected.iter().map(|row| row["reason"].clone()).collect();
        assert_eq!(reasons, [serde_json::json!("UR-26: cancelled by Provider::stop")], "{:?}", rejected(&events));
    }

    #[test]
    fn ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it() {
        // F5: a reopen of the streamer keeps the values the device holds; a loss clears the mark.
        let (core, device, _time, mut tx, frozen, _events, _to_tx) = owner(1);
        let key = stream_key(Dir::Tx, 0);
        drop(frozen);
        lock(&core.streams).record_configured(key, Some(1));
        tx.chain.mark_reopen_needed();
        assert!(tx.pass(), "the reopen runs");
        assert!(!tx.chain.needs_reopen() && device.calls().iter().any(|call| call.starts_with("tx_open")), "{:?}", device.calls());
        assert_eq!(lock(&core.streams).stream(key).configured, Some(1), "a reopen keeps the mark");
        core.device_lost("test: the device is lost");
        assert_eq!(lock(&core.streams).stream(key).configured, None, "a loss clears the mark");
    }

    #[test]
    fn ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read() {
        // F5, the reopen's own race: the device is lost after the reopen returned Ok and before its mark is written back.
        let (core, device, _time, mut tx, frozen, _events, _to_tx) = owner(1);
        let key = stream_key(Dir::Tx, 0);
        drop(frozen);
        lock(&core.streams).record_configured(key, Some(1));
        tx.chain.mark_reopen_needed();
        let lost = core.clone();
        super::super::test_support::probe(&core, "tx_reopen_done", move |_| lost.device_lost("test: the device is lost during the reopen"));
        tx.pass();
        assert!(device.calls().iter().any(|call| call.starts_with("tx_open")), "the reopen ran: {:?}", device.calls());
        assert_eq!(lock(&core.streams).stream(key).configured, None, "a loss during the reopen clears the mark the reopen read");
    }

    #[test]
    fn ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery() {
        // S10 (R4): a value due while a recovery is pending waits for the reopen; the configure comes after it.
        let (core, device, time, mut tx, frozen, _events, _to_tx) = owner(1);
        let clock = tx.clock.expect("a clock");
        let applies = record_applies(&core, &device);
        tx.chain.mark_reopen_needed();
        give(&mut tx, valued(&clock, 20_000, 4, 1, 1, F2, 1));
        turns(&core, &time, &device, &mut tx, 1, 15, 1);
        assert!(applies.lock().unwrap().is_empty(), "no configure during the recovery: {:?}", traffic(&device, 1));
        let before = device.calls().len();
        // The reopen needs the device's clock to run, so the frozen participant goes first.
        drop(frozen);
        assert!(tx.pass(), "the reopen runs first");
        let _frozen = Frozen::of(&device);
        turns(&core, &time, &device, &mut tx, 16, 35, 1);
        let calls = device.calls();
        let apply = calls.iter().skip(before).position(|call| call.starts_with("apply tx")).map(|index| index + before).expect("the value is applied after the reopen");
        assert!(calls[before..apply].iter().any(|call| call.starts_with("tx_open")), "the reopen precedes the apply: {calls:?}");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send ")), "the burst is sent after the apply: {calls:?}");
    }

    #[test]
    fn ur_23_two_transmit_streams_busy_and_empty() {
        // S6 (P9): stream 0 streams a 30 000-sample burst; stream 1 is empty and takes a new value. Its configure touches channel 1
        // alone, and stream 0's sends stay contiguous through it.
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, ..FakeConfig::default() }));
        let (core, time, events) = super::super::test_support::rig_on_description(device.clone(), Vec::new(), BTreeMap::new(), two_tx_description());
        let _frozen = Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.tx_open(1, &[1]).unwrap();
        for chan in 0..2 {
            device.apply(Dir::Tx, chan, &Settings { rate: Some(1e6), freq: Some(F1), ..Settings::default() }).unwrap();
        }
        let c0 = core.register(Dir::Tx, 200, 0).unwrap();
        let c1 = core.register(Dir::Tx, 200, 0).unwrap();
        for k in 0..2 {
            lock(&core.streams).record_configured(stream_key(Dir::Tx, k), Some(0));
        }
        let (_to0, cmds0) = std::sync::mpsc::channel();
        let (_to1, cmds1) = std::sync::mpsc::channel();
        let mut s0 = Tx::new(core.clone(), 0, cmds0, Some(c0), Channels::first(1));
        let mut s1 = Tx::new(core.clone(), 1, cmds1, Some(c1), Channels::first(0).with(1));
        let start = device.calls().len();
        give(&mut s0, valued(&c0, 20_000, 30_000, 1, 0, F1, 1));
        let mut value = one_shot(&c1, 25_000, 1, vec![[0.25, 0.0]; 4]);
        value.settings = Arc::new(super::ColdConfig::of_tx(BTreeMap::from([(crate::provider::core::key("radio.tx.c1.frequency_hz"), ezsdr_kernel::spec::Value::num(F2).unwrap())]), ActionId(2), 1));
        give(&mut s1, value);
        for ms in 1..=60 {
            device_at(&core, &time, &device, core.ticks(ms * 1_000_000));
            for _ in 0..8 {
                drain_tx(&mut s0);
                drain_tx(&mut s1);
                let (busy, empty) = (s0.step(), s1.step());
                if !busy && !empty {
                    break;
                }
            }
        }
        let calls = device.calls()[start..].to_vec();
        assert_eq!(calls.iter().filter(|call| call.starts_with("apply tx")).cloned().collect::<Vec<_>>(), ["apply tx 1 rate=- freq=2000000000 gain=- antenna=-"], "{calls:?}");
        let apply = calls.iter().position(|call| call.starts_with("apply tx")).expect("stream 1's value is applied");
        assert!(calls[apply..].iter().any(|call| call.starts_with("tx_send 0 ")), "stream 0 is still sending: {calls:?}");
        assert!(calls.iter().any(|call| call.starts_with("tx_send 1 n=4")), "stream 1 sends its burst: {calls:?}");
        let sends: Vec<(i64, i64)> = calls
            .iter()
            .filter_map(|call| call.strip_prefix("tx_send 0 n="))
            .map(|rest| (rest.split(' ').next().unwrap().parse().unwrap(), rest.split("at=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap()))
            .collect();
        assert!(sends.len() >= 2 && sends.windows(2).all(|pair| pair[1].1 == pair[0].1 + 200 * pair[0].0), "stream 0's sends are contiguous: {sends:?}");
        assert!(rejected(&events).is_empty(), "{:?}", lock(&core.rec).rejected);
    }

    /// A transmit owner and uhd-control on stream 0 at 1 MS/s (200 ticks a sample, a 10 ms window, a 2 ms lead), with one ingested
    /// input per `waves` entry: `rig.waves[i]`.
    struct SwitchRig {
        core: Arc<Core>,
        device: Arc<FakeDevice>,
        time: Arc<ezsdr_kernel::time::ManualTimeAuthority>,
        events: Arc<EventCollector>,
        control: Control,
        tx: Tx,
        clock: Clock,
        waves: Vec<ezsdr_kernel::manifest::ArtifactRef>,
        _frozen: Frozen,
    }

    fn switch_rig(waves: Vec<Vec<Iq>>) -> SwitchRig {
        switch_rig_with(waves, Vec::new())
    }

    fn switch_rig_with(waves: Vec<Vec<Iq>>, faults: Vec<FakeFault>) -> SwitchRig {
        switch_rig_on(waves, faults, 200)
    }

    /// A rig whose transmit clock has `n` root ticks a sample on the 200 MHz root: 1 MS/s at 200, 200 MS/s at 1.
    fn switch_rig_on(waves: Vec<Vec<Iq>>, faults: Vec<FakeFault>, n: i64) -> SwitchRig {
        let device = Arc::new(FakeDevice::new(FakeConfig { clock: ClockMode::Manual, faults, ..FakeConfig::default() }));
        let mut inputs = BTreeMap::new();
        let mut refs = Vec::new();
        for (i, samples) in waves.iter().enumerate() {
            let (wave, bytes) = waveform_input(&format!("switch_{i}"), samples);
            inputs.insert(wave.hash.clone(), Arc::<[u8]>::from(bytes));
            refs.push(wave);
        }
        let (core, time, events) = crate::provider::test_support::rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
        let frozen = Frozen::of(&device);
        device_at(&core, &time, &device, 0);
        device.tx_open(0, &[0]).unwrap();
        device.apply(Dir::Tx, 0, &Settings { rate: Some(200e6 / n as f64), ..Settings::default() }).unwrap();
        let (control, cmds) = booking_control_on(&core, core.register(Dir::Tx, n, 0).unwrap());
        let clock = tx_clock(&core);
        let tx = Tx::new(core.clone(), 0, cmds, Some(clock), Channels::first(1));
        SwitchRig { core, device, time, events, control, tx, clock, waves: refs, _frozen: frozen }
    }

    /// The switch mark (RM-37): `radio.repeat_operation = "switch"`.
    fn switch_mark() -> BTreeMap<ezsdr_kernel::spec::Key, ezsdr_kernel::spec::Value> {
        BTreeMap::from([(ezsdr_kernel::spec::Key::parse("radio.repeat_operation").unwrap(), ezsdr_kernel::spec::Value::Scalar(ezsdr_kernel::spec::Scalar::from("switch")))])
    }

    /// Books the repeat of `rig.waves[wave]` at sample `k` (the first repeat books at its origin, 2 000).
    fn book_repeat(rig: &mut SwitchRig, id: u64, k: i64, wave: usize) {
        book_with(&mut rig.control, &rig.core, id, 0, rig.clock, k, &rig.waves[wave], true, LatePolicy::DropAndFlag, BTreeMap::new());
    }

    /// Books the switch to `rig.waves[wave]` at sample `k`: a marked repeat under `start_repeat`'s SendAsapAndFlag (RM-37).
    fn book_switch(rig: &mut SwitchRig, id: u64, k: i64, wave: usize) {
        book_with(&mut rig.control, &rig.core, id, 0, rig.clock, k, &rig.waves[wave], true, LatePolicy::SendAsapAndFlag, switch_mark());
    }

    /// uhd-control's Stop of transmit stream 0 (UR-26, which books the record's Stop), then the owner's delivery of it.
    fn stop(rig: &mut SwitchRig) {
        rig.control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(super::super::test_support::target_of(&rig.core.tx_id)) }, Some(rig.core.tx_id.clone())));
        drain_tx(&mut rig.tx);
    }

    /// The owner and control run from device time `from` to `until` milliseconds, one millisecond a step.
    fn run(rig: &mut SwitchRig, from: i64, until: i64) {
        turns(&rig.core, &rig.time, &rig.device, &mut rig.tx, from, until, 1);
    }

    /// The reasons of the refusals the Provider has reported since the last call.
    fn refusals(events: &EventCollector) -> Vec<String> {
        rejected(events).iter().map(|event| event.payload["reason"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// The `switches` row of the Provider's record, settled first.
    fn switch_row(core: &Core, index: usize) -> serde_json::Value {
        core.settle_switches();
        lock(&core.rec).switches[index].clone()
    }

    const OLD: Iq = [0.25, 0.0];
    const NEW: Iq = [0.75, -0.25];

    #[test]
    fn ur_21_s8_switch_installed_before_the_boundary_sample() {
        // S8 (UR-36, RM-37): a repeat booked at 2 000 and switched at 3 ms takes effect at its boundary, 15 000 (the receipt plus
        // the window and the lead, on the 2 000 origin). The old repeat ends at 14 999, the new waveform starts at 15 000.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        drain_tx(&mut rig.tx);
        assert_eq!(held_keys(&rig.tx), [15_000], "taken at receipt: the owner holds the new waveform at its boundary once it reads it");
        run(&mut rig, 3, 20);
        assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
        assert_eq!(rig.device.transmitted(rig.clock.instant(14_999), 2), vec![Some(OLD), Some(NEW)], "{:?}", rig.device.calls());
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    }

    #[test]
    fn ur_23_switch_cut_reuses_held_cut_path() {
        // m9: the switch at its boundary continues the device burst; no end-of-burst is sent there and no underflow is recorded.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 20);
        assert!(sends(&rig.device).iter().all(|send| !send.2), "{:?}", sends(&rig.device));
        assert!(rig.events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TX_UNDERFLOW));
    }

    #[test]
    fn ur_21_s4_an_underflow_ends_the_repeat_so_a_later_switch_is_refused_at_receipt() {
        // S4 (note 47 stage 5, sol-code): an underflow abandons the repeat, and the record ends it as a Stop at that point would. A
        // switch received after it is refused at receipt, so no switch is accepted and left pending for a repeat that is gone.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(2_500)), channel: 0 });
        drain_tx(&mut rig.tx);
        let reason = "UR-21: a repeat switch needs a playing repeat and no pending switch";
        book_switch(&mut rig, 2, 3_000, 1);
        assert_eq!(refusals(&rig.events), [reason], "{:?}", lock(&rig.core.rec).rejected);
        book_switch(&mut rig, 3, 20_100, 2);
        assert_eq!(refusals(&rig.events), [reason], "{:?}", lock(&rig.core.rec).rejected);
        book_with(&mut rig.control, &rig.core, 4, 0, rig.clock, 300_000, &rig.waves[2], false, LatePolicy::DropAndFlag, BTreeMap::new());
        assert_eq!(refusals(&rig.events), Vec::<String>::new(), "the abandoned repeat ends at its cursor, so a burst past its stop is accepted: {:?}", lock(&rig.core.rec).rejected);
    }

    #[test]
    fn ur_21_s4_a_burst_past_the_abandoned_repeat_s_cursor_is_accepted_on_time() {
        // S4 (note 47 stage 5, Opus re-check): an underflow ends the repeat in the record at its handed cursor c, so a Stop there would
        // leave the stream free from c + 5. A burst on time at c + 5 000 is past that stop and is accepted; a stop one in-flight window
        // later (c + 10 005) refused it as overlapping.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        let cursor = rig.tx.open_next().expect("the repeat is handed");
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(2_500)), channel: 0 });
        drain_tx(&mut rig.tx);
        book_with(&mut rig.control, &rig.core, 4, 0, rig.clock, cursor + 5_000, &rig.waves[2], false, LatePolicy::DropAndFlag, BTreeMap::new());
        assert_eq!(refusals(&rig.events), Vec::<String>::new(), "{:?}", lock(&rig.core.rec).rejected);
    }

    #[test]
    fn ur_21_s4_a_switch_taken_past_the_abandoned_cursor_is_cancelled_not_applied() {
        // S4 (note 47 stage 5, Opus re-check): the owner reads a switch whose boundary is 15 000, then an underflow abandons the repeat at
        // its handed cursor c, below 15 000. No sample of the new waveform played, so the record settles the switch cancelled.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 4);
        let cursor = rig.tx.open_next().expect("the repeat is handed");
        assert!(cursor < 15_000, "the cursor {cursor} is before the boundary 15 000");
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(cursor)), channel: 0 });
        drain_tx(&mut rig.tx);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{:?}", lock(&rig.core.rec).rejected);
    }

    #[test]
    fn ur_21_s4_a_switch_at_the_abandoned_cursor_is_cancelled_not_applied() {
        // S4 (Opus re-check): the switch's boundary is 15 000 and the repeat is abandoned at that same handed cursor. No sample of the new
        // waveform is sent, so the record settles the switch cancelled, not applied by the Stop the abandon records.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(15_000)), channel: 0 });
        drain_tx(&mut rig.tx);
        assert!(held_keys(&rig.tx).is_empty(), "the switch's held burst goes with the switch");
        run(&mut rig, 3, 20);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{:?}", lock(&rig.core.rec).rejected);
        assert!(!rig.device.transmitted(rig.clock.instant(15_000), 1_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_s4_a_switch_past_the_abandoned_cursor_sends_none_of_its_waveform() {
        // S4 (Opus re-check): the handed cursor 14 000 is below the switch's boundary 15 000, so the switch is cancelled. Its held burst is
        // not blocked by the abandoned repeat's front (14 005), so it is sent unless the abandon removes it.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 14_000);
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(14_000)), channel: 0 });
        drain_tx(&mut rig.tx);
        run(&mut rig, 3, 20);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{:?}", lock(&rig.core.rec).rejected);
        assert!(!rig.device.transmitted(rig.clock.instant(14_000), 2_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_s4_a_report_between_the_join_and_the_switch_s_open_cancels_it() {
        // S4 (Opus re-check, the neighbour): the switch joins the open burst at its boundary, so no repeat is open when an underflow
        // abandons the device burst there. The switch the owner read is cancelled, and nothing of the new waveform is sent later.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
        assert!(rig.tx.step(), "the held switch joins the open burst at its boundary");
        assert!(rig.tx.open_next().is_none());
        rig.tx.report(crate::device::TxReport { code: crate::device::TxCode::Underflow, tick: Some(rig.clock.instant(15_000)), channel: 0 });
        drain_tx(&mut rig.tx);
        assert!(held_keys(&rig.tx).is_empty(), "the switch's held burst goes with the switch");
        book_with(&mut rig.control, &rig.core, 4, 0, rig.clock, 20_000, &rig.waves[0], false, LatePolicy::DropAndFlag, BTreeMap::new());
        assert_eq!(refusals(&rig.events), Vec::<String>::new(), "the abandoned repeat ends at its cursor, so a burst on time at 20 000 is accepted: {:?}", lock(&rig.core.rec).rejected);
        book_switch(&mut rig, 5, 20_100, 1);
        assert_eq!(refusals(&rig.events), ["UR-21: a repeat switch needs a playing repeat and no pending switch"], "{:?}", lock(&rig.core.rec).rejected);
        run(&mut rig, 3, 20);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{:?}", lock(&rig.core.rec).rejected);
        assert!(!rig.device.transmitted(rig.clock.instant(15_000), 1_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_s4_a_switch_whose_repeat_the_host_dispatched_late_is_refused_at_receipt() {
        // S4, UR-38 (#84): the host is late for the repeat the switch replaces: the device has passed its start and the owner has issued
        // none of it. The repeat has not started on the device, so the switch is refused at receipt, with no row; nothing else changes.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        device_at(&rig.core, &rig.time, &rig.device, rig.core.ticks(3_000_000));
        book_switch(&mut rig, 2, 3_000, 1);
        assert_eq!(refusals(&rig.events), ["UR-38: the repeat the switch replaces has not started on the device"], "{:?}", lock(&rig.core.rec).rejected);
        assert!(lock(&rig.core.rec).switches.is_empty(), "a refused switch writes no row");
        drain_tx(&mut rig.tx);
        // RM-37: the refused switch is not the stream's playing repeat; the repeat it would have replaced still is.
        let playing = lock(&rig.core.streams).stream(stream_key(Dir::Tx, 0)).book.playing;
        assert_eq!(playing.map(|playing| playing.id), Some(ActionId(1)), "the refused switch (id 2) must not be the playing repeat");
    }

    #[test]
    fn ur_21_s4_a_switch_joined_then_abandoned_late_is_cancelled_not_applied() {
        // S4 (note 47 stage 5, Opus re-check of round 7, Minor A): the switch joins the open repeat at its boundary, and the host is late past
        // that boundary. The record applies the switch, then the timeline's expiry abandons the repeat at the joined cursor. No sample of the
        // new waveform is sent, so the switch is settled cancelled and its held burst is not sent late.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        drain_tx(&mut rig.tx);
        rig.tx.with_book(|book| book.open.as_mut().expect("the repeat is handed").next = 15_000);
        assert!(rig.tx.step(), "the held switch joins the open burst at its boundary");
        device_at(&rig.core, &rig.time, &rig.device, rig.core.ticks(16_000_000));
        assert!(rig.tx.step());
        drain_tx(&mut rig.tx);
        assert!(held_keys(&rig.tx).is_empty(), "the switch's held burst goes with the switch");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "{:?}", lock(&rig.core.rec).rejected);
        run(&mut rig, 16, 25);
        assert!(!rig.device.transmitted(rig.clock.instant(15_000), 1_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_s4_a_late_repeat_dropped_at_dispatch_leaves_the_record() {
        // UR-21, RM-15 (fix round 8, the unverified item): the repeat is on time at receipt and the host dispatches it late, so DropAndFlag
        // drops it at dispatch and it never plays. Its endless region leaves the record, so a finite burst on time after it is accepted.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        drain_tx(&mut rig.tx);
        device_at(&rig.core, &rig.time, &rig.device, rig.core.ticks(3_000_000));
        for _ in 0..8 {
            rig.tx.step();
        }
        assert!(lock(&rig.core.rec).rejected.iter().any(|r| r["reason"] == "UR-21: the late policy dropped the burst"), "{:?}", lock(&rig.core.rec).rejected);
        book_with(&mut rig.control, &rig.core, 2, 0, rig.clock, 20_000, &rig.waves[1], false, LatePolicy::DropAndFlag, BTreeMap::new());
        assert!(refusals(&rig.events).is_empty(), "the dropped repeat holds no region: {:?}", lock(&rig.core.rec).rejected);
        run(&mut rig, 3, 30);
        assert!(rig.device.transmitted(rig.clock.instant(20_000), 2).iter().all(|sample| *sample == Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_s5_second_switch_refused_while_device_time_is_before_the_boundary() {
        // Sol 5: the owner hands new-waveform samples past the boundary while device time is before it; a second switch is refused
        // until device time reaches the first's boundary.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000], vec![NEW; 300]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 6);
        assert!(rig.device.transmitted(rig.clock.instant(15_000), 1)[0].is_none(), "device time is before the boundary");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "pending", "the switch stays pending until device time reaches its boundary");
        book_switch(&mut rig, 3, 6_000, 2);
        assert_eq!(refusals(&rig.events).len(), 1, "{:?}", lock(&rig.core.rec).rejected);
        run(&mut rig, 6, 20);
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    }

    #[test]
    fn ur_23_stop_before_the_boundary_cancels_the_pending_switch() {
        // Q-2 (D42): a Stop at 4 ms bounds the stream at 14 003, before the boundary 15 000: the switch is cancelled, and no sample of
        // the new waveform is sent.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 3);
        device_at(&rig.core, &rig.time, &rig.device, rig.core.ticks(4_000_000));
        stop(&mut rig);
        assert!(!rig.device.transmitted(rig.clock.instant(0), 20_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
    }

    #[test]
    fn ur_23_stop_after_handover_before_the_boundary_takes_effect() {
        // Q-2 (D42), m4: the switch is taken and handed, and the owner's cursor is past the boundary before the Stop. A Stop at 8 ms
        // bounds the stream at 18 003, past the boundary 15 000: the switch has taken effect, and the new repeat is cut at the bound.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 8);
        assert!(rig.tx.open_next().is_some_and(|next| next > 15_000), "the cursor is past the boundary before the Stop");
        stop(&mut rig);
        assert_eq!(rig.device.transmitted(rig.clock.instant(15_000), 1), vec![Some(NEW)]);
        assert!(!rig.device.transmitted(rig.clock.instant(18_100), 900).contains(&Some(NEW)), "{:?}", rig.device.calls());
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "applied");
    }

    #[test]
    fn ur_23_provider_stop_and_loss_cancel_the_pending_switch() {
        // RM-37: a Provider stop or a loss before the boundary (the record's `end`) cancels the switch the owner has not reached.
        for lost in [false, true] {
            let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 1_000]]);
            book_repeat(&mut rig, 1, 2_000, 0);
            run(&mut rig, 0, 3);
            book_switch(&mut rig, 2, 3_000, 1);
            run(&mut rig, 3, 3);
            let core = rig.core.clone();
            crate::provider::test_support::end_streams(&core, core.ticks(4_000_000), false, lost);
            assert!(!rig.device.transmitted(rig.clock.instant(0), 20_000).contains(&Some(NEW)), "lost {lost}: {:?}", rig.device.calls());
            assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled", "lost {lost}");
        }
    }

    #[test]
    fn ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch() {
        // RM-37, UR-22 (F8): the reopen that ends the repeat cancels the switch with it.
        let mut rig = switch_rig_with(vec![vec![OLD; 1_000], vec![NEW; 1_000]], vec![FakeFault::TxOpenFails(1)]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 3);
        drop(rig._frozen);
        rig.tx.chain.mark_reopen_needed();
        assert!(rig.tx.pass(), "the reopen runs");
        assert_eq!(switch_row(&rig.core, 0)["outcome"], "cancelled");
        assert!(!rig.device.transmitted(rig.clock.instant(0), 20_000).contains(&Some(NEW)), "{:?}", rig.device.calls());
    }

    #[test]
    fn ur_21_metadata_is_empty_or_the_mark_only() {
        // Row 45: a repeat takes no metadata but the switch mark, and the mark alone on a repeat under SendAsapAndFlag is a switch.
        let mut rig = switch_rig(vec![vec![OLD; 1_000]]);
        let other = BTreeMap::from([(ezsdr_kernel::spec::Key::parse("radio.repeat_operation").unwrap(), ezsdr_kernel::spec::Value::Scalar(ezsdr_kernel::spec::Scalar::from("start")))]);
        book_with(&mut rig.control, &rig.core, 1, 0, rig.clock, 2_000, &rig.waves[0], true, LatePolicy::DropAndFlag, other);
        assert_eq!(refusals(&rig.events), ["UR-21: TxBurst.metadata must be empty or the repeat-switch mark"]);
        book_with(&mut rig.control, &rig.core, 4, 0, rig.clock, 2_000, &rig.waves[0], false, LatePolicy::DropAndFlag, switch_mark());
        assert_eq!(refusals(&rig.events), ["UR-21: TxBurst.metadata must be empty or the repeat-switch mark"], "the mark alone on a finite burst is not a switch");
        book_with(&mut rig.control, &rig.core, 6, 0, rig.clock, 2_000, &rig.waves[0], false, LatePolicy::SendAsapAndFlag, switch_mark());
        assert_eq!(refusals(&rig.events), ["UR-21: TxBurst.metadata must be empty or the repeat-switch mark"], "nor under SendAsapAndFlag");
        book_switch(&mut rig, 2, 2_000, 0);
        assert_eq!(refusals(&rig.events), ["UR-21: a repeat switch needs a playing repeat and no pending switch"]);
        book_repeat(&mut rig, 3, 2_000, 0);
        assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
        // RM-13: the mark on a repeat under another late policy is refused at receipt as any misplaced mark, and the repeat plays on.
        for policy in [LatePolicy::DropAndFlag, LatePolicy::RejectAtPlan] {
            book_with(&mut rig.control, &rig.core, 5, 0, rig.clock, 3_000, &rig.waves[0], true, policy, switch_mark());
            assert_eq!(refusals(&rig.events), ["UR-21: TxBurst.metadata must be empty or the repeat-switch mark"], "{policy:?}");
        }
        assert!(time_errors(&rig.events).is_empty() && lock(&rig.core.rec).switches.is_empty());
    }

    #[test]
    fn ur_21_s9_switch_different_length_sets_the_new_period() {
        // S9 (UHD): the switch to a 500-sample waveform makes the new period 500; the next switch's record names it as the old one.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        run(&mut rig, 0, 3);
        book_switch(&mut rig, 2, 3_000, 1);
        run(&mut rig, 3, 16);
        assert_eq!(rig.device.transmitted(rig.clock.instant(15_000), 500), vec![Some(NEW); 500]);
        book_switch(&mut rig, 3, 16_000, 2);
        run(&mut rig, 16, 17);
        assert!(refusals(&rig.events).is_empty(), "{:?}", lock(&rig.core.rec).rejected);
        let second = switch_row(&rig.core, 1);
        assert_eq!((second["old_period"].as_i64(), second["new_period"].as_i64()), (Some(500), Some(300)), "{second}");
    }

    #[test]
    fn ur_21_s10_a_repeat_after_a_stop_keeps_its_own_period() {
        // A repeat cancelled by a Stop before it opens does not clear the playing record; the repeat booked after the Stop takes its own period 500.
        let mut rig = switch_rig(vec![vec![OLD; 1_000], vec![NEW; 500], vec![NEW; 300]]);
        book_repeat(&mut rig, 1, 2_000, 0);
        stop(&mut rig);
        run(&mut rig, 0, 1);
        book_repeat(&mut rig, 2, 40_000, 1);
        run(&mut rig, 1, 42);
        book_switch(&mut rig, 3, 60_000, 2);
        run(&mut rig, 42, 62);
        assert!(refusals(&rig.events).iter().all(|reason| reason.contains("cancelled by Stop")), "{:?}", lock(&rig.core.rec).rejected);
        let row = switch_row(&rig.core, 0);
        assert_eq!(row["old_period"].as_i64(), Some(500), "{row}");
    }
}
