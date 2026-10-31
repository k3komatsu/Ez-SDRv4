//! What the Provider's three threads share (UR-14…UR-30).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration as Wall;

use ezsdr_kernel::event::{ActionId, Event, EventHandle, EventKind, EventSink, EventSource, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{InputStore, ModuleError, StopMode};
#[cfg(test)]
use ezsdr_kernel::spec::Key;
use ezsdr_kernel::stream::{DataLink, Direction};
use ezsdr_kernel::time::{ClockRegistry, Rational, TimeAuthority, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, NO_RELEASE, StreamKey};
use serde_json::{Value as Json, json};

use super::control::{ColdConfig, Planned};
use super::mode::{DescriptionExt, channel_list};
use crate::device::{Device, DeviceError, Dir, Settings, decimation};
use crate::device::{Iq, TxCode, TxReport};
use crate::host::{ClockGuard, ClockMutex};

const POLL: Wall = Wall::from_millis(1);

/// A guard of the instance's state (UR-25): a `MutexGuard`, except that the test build counts
/// how many `core::lock` guards this thread holds — `streams`, `rec`, `held` and `rates` all
/// count — so that [`assert_line_free`] sees a lock order a `try_lock` cannot: another
/// thread's line is not this caller's.
pub(crate) struct Locked<'a, T> {
    guard: MutexGuard<'a, T>,
}

impl<T> std::ops::Deref for Locked<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for Locked<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Locked<'_, T> {
    fn drop(&mut self) {
        #[cfg(test)]
        super::test_support::left_lock();
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> Locked<'_, T> {
    #[cfg(test)]
    {
        super::test_support::note_lock(std::ptr::from_ref(m) as usize);
        super::test_support::entered_lock();
    }
    Locked { guard: m.lock().unwrap_or_else(|e| e.into_inner()) }
}

/// UR-25: the caller holds no guard of the instance's state — `streams`, `rec`, `held` or
/// `rates` —, so the configuration's `updates` may be taken. The whole of the check: the
/// other order (`rec` taken before `streams`) is not detected.
pub(crate) fn assert_line_free(what: &str) {
    #[cfg(test)]
    super::test_support::assert_line_free(what);
    #[cfg(not(test))]
    let _ = what;
}

/// A test's probe of a section (no effect outside the test build), by name.
#[cfg(test)]
pub(crate) type Probe = Arc<dyn Fn(Option<&Streams>) + Send + Sync>;

#[cfg(test)]
pub(crate) fn key(name: &str) -> Key {
    Key::parse(name).expect("a declared radio key")
}

/// A registered stream clock: `N` root ticks per sample from `origin` (UR-12, RM-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Clock {
    pub domain: ClockDomainId,
    pub origin: i64,
    pub n: i64,
}

/// A timed receive DDC write not yet proven consumed by a sample from this stream.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RxEntry {
    pub order: (i64, u64),
    pub channel: usize,
    pub value: f64,
    pub accepted_at: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SegmentKey {
    pub by: Option<u64>,
    pub origin: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum RxDevice {
    #[default]
    Off,
    Armed {
        at: i64,
    },
    Streaming,
    Flushing,
    Uncertain,
    Lost,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct RxIntent {
    cut: Option<i64>,
    end_now: bool,
    terminate: Option<StopMode>,
}

#[derive(Default)]
pub(crate) enum Restore {
    #[default]
    NotAsked,
    Planned(BTreeMap<usize, f64>),
    Done,
}

/// One receive stream's device state and the owner intent serialized with control's handoff.
#[derive(Default)]
pub(crate) struct RxChain {
    device: RxDevice,
    intent: RxIntent,
    segment: Option<SegmentKey>,
    debt: Vec<RxEntry>,
    left: Vec<RxEntry>,
    latest: BTreeMap<usize, RxEntry>,
    restore: Restore,
}

impl RxChain {
    pub fn device(&self) -> RxDevice {
        self.device
    }
    pub fn segment(&self) -> Option<SegmentKey> {
        self.segment
    }
    pub fn cut(&self) -> Option<i64> {
        self.intent.cut
    }
    pub fn end_now(&self) -> bool {
        self.intent.end_now
    }
    pub fn terminate(&self) -> Option<StopMode> {
        self.intent.terminate
    }
    pub fn debt(&self) -> &[RxEntry] {
        &self.debt
    }
    pub fn left(&self) -> &[RxEntry] {
        &self.left
    }
    pub fn latest(&self) -> &BTreeMap<usize, RxEntry> {
        &self.latest
    }
    pub fn restore(&self) -> &Restore {
        &self.restore
    }

    pub fn eligible(&self, segment: SegmentKey, instant: i64) -> bool {
        self.segment == Some(segment) && matches!(self.device, RxDevice::Armed { .. } | RxDevice::Streaming) && instant >= segment.origin
    }

    pub fn adopt_start(&mut self, segment: SegmentKey, at: i64) {
        assert!(matches!(self.device, RxDevice::Off));
        assert!(self.debt.is_empty());
        self.segment = Some(segment);
        self.device = RxDevice::Armed { at };
    }

    pub fn begin_result(&mut self, segment: SegmentKey, at: i64, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Off | RxDevice::Uncertain));
        assert!(self.debt.is_empty());
        match result {
            Ok(()) => {
                self.segment = Some(segment);
                self.device = RxDevice::Armed { at };
                Ok(())
            }
            Err(error) => {
                if error.lost {
                    self.device = RxDevice::Lost;
                }
                Err(error)
            }
        }
    }

    pub fn restart_result(&mut self, at: i64, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Armed { .. } | RxDevice::Streaming));
        match result {
            Ok(()) => {
                self.device = RxDevice::Armed { at };
                Ok(())
            }
            Err(error) => {
                if error.lost {
                    self.device = RxDevice::Lost;
                }
                Err(error)
            }
        }
    }

    pub fn first_sample(&mut self, segment: SegmentKey) {
        if self.segment == Some(segment) && matches!(self.device, RxDevice::Armed { .. }) {
            self.device = RxDevice::Streaming;
        }
    }

    pub fn request_cut(&mut self, cut: i64) {
        self.intent.cut = Some(self.intent.cut.map_or(cut, |old| old.min(cut)));
    }

    pub fn request_end_now(&mut self) {
        self.intent.end_now = true;
    }

    pub fn request_terminate(&mut self, mode: StopMode) {
        self.intent.terminate.get_or_insert(mode);
    }

    /// Provisional debt is visible while `apply` runs; refusal removes it, loss is terminal.
    pub fn hand(&mut self, entry: RxEntry, apply: impl FnOnce() -> Result<(), DeviceError>, accepted_at: impl FnOnce() -> i64) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Armed { .. } | RxDevice::Streaming));
        self.debt.push(entry.clone());
        match apply() {
            Ok(()) => {
                let accepted_at = accepted_at();
                let accepted = RxEntry { accepted_at: Some(accepted_at), ..entry };
                if let Some(debt) = self.debt.iter_mut().find(|debt| debt.order == accepted.order && debt.channel == accepted.channel) {
                    debt.accepted_at = Some(accepted_at);
                }
                self.latest.insert(accepted.channel, accepted);
                Ok(())
            }
            Err(error) => {
                self.debt.retain(|debt| debt.order != entry.order || debt.channel != entry.channel);
                if error.lost {
                    self.device = RxDevice::Lost;
                }
                Err(error)
            }
        }
    }

    pub fn stop_result(&mut self, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Armed { .. } | RxDevice::Streaming));
        match result {
            Ok(()) => {
                self.device = RxDevice::Off;
                Ok(())
            }
            Err(error) => {
                self.device = if error.lost { RxDevice::Lost } else { RxDevice::Uncertain };
                Err(error)
            }
        }
    }

    pub fn flush_start_result(&mut self, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Off));
        match result {
            Ok(()) => {
                self.device = RxDevice::Flushing;
                Ok(())
            }
            Err(error) => {
                if error.lost {
                    self.device = RxDevice::Lost;
                }
                Err(error)
            }
        }
    }

    pub fn flush_stop_result(&mut self, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Flushing));
        match result {
            Ok(()) => {
                self.device = RxDevice::Off;
                Ok(())
            }
            Err(error) => {
                self.device = if error.lost { RxDevice::Lost } else { RxDevice::Uncertain };
                Err(error)
            }
        }
    }

    pub fn consume(&mut self, sample_ticks: i64, channels: Channels, first_tick: i64, samples: &[Vec<Iq>]) {
        for (index, channel) in channels.iter().enumerate() {
            let Some(samples) = samples.get(index) else { continue };
            for sample in 0..samples.len() {
                let tick = i128::from(first_tick) + sample as i128 * i128::from(sample_ticks);
                let channel = channel as usize;
                let left = self.left.iter().position(|entry| entry.channel == channel);
                if let Some(position) = left {
                    let entry = &self.left[position];
                    if tick >= i128::from(entry.order.0) && tick >= i128::from(entry.accepted_at.unwrap_or(i64::MIN)) {
                        self.left.remove(position);
                    }
                } else if let Some(position) = self.debt.iter().position(|entry| entry.channel == channel) {
                    let entry = &self.debt[position];
                    if tick >= i128::from(entry.order.0) && tick >= i128::from(entry.accepted_at.unwrap_or(i64::MIN)) {
                        self.debt.remove(position);
                    }
                }
            }
        }
    }

    pub fn lose(&mut self) -> Vec<RxEntry> {
        self.device = RxDevice::Lost;
        self.report_debt()
    }

    /// Transfers this segment's remaining obligations to persistent, already-reported `left`.
    pub fn finish(&mut self) -> Vec<RxEntry> {
        assert!(matches!(self.device, RxDevice::Off | RxDevice::Uncertain | RxDevice::Lost));
        let reported = self.report_debt();
        self.segment = None;
        self.intent.cut = None;
        self.intent.end_now = false;
        reported
    }

    pub fn abandon_unowned(&mut self) {
        if self.device != RxDevice::Lost {
            self.device = RxDevice::Uncertain;
        }
        self.segment = None;
        self.intent.end_now = true;
        self.intent.terminate.get_or_insert(StopMode::Abort);
    }

    pub fn plan_restore(&mut self, values: BTreeMap<usize, f64>) {
        self.restore = Restore::Planned(values);
    }

    pub fn take_restore(&mut self) -> Option<BTreeMap<usize, f64>> {
        match std::mem::replace(&mut self.restore, Restore::Done) {
            Restore::Planned(values) => Some(values),
            other => {
                self.restore = other;
                None
            }
        }
    }

    fn report_debt(&mut self) -> Vec<RxEntry> {
        let entries = std::mem::take(&mut self.debt);
        self.left.extend(entries.iter().cloned());
        entries
    }
}

impl Clock {
    /// The root tick of sample `k`.
    pub fn instant(&self, k: i64) -> i64 {
        i64::try_from(i128::from(self.origin) + i128::from(k) * i128::from(self.n)).expect("a representable stream instant")
    }

    /// The first sample at or after root tick `t`.
    pub fn at_or_after(&self, t: i64) -> i64 {
        i64::try_from((i128::from(t) - i128::from(self.origin) + i128::from(self.n) - 1).div_euclid(i128::from(self.n))).expect("a representable sample index")
    }
}

/// Each stream's planned state, once, shared by uhd-control, uhd-rx and uhd-tx and booked by
/// whichever learns an event (maintenance note 23): uhd-control the commands, uhd-rx and
/// uhd-tx a configuration their device refused, the thread that finds the device lost the
/// loss, and `Provider::stop` the streams' end. What belongs to the instance stays on
/// [`Streams`] (`seq`, `ended`, `stop`, `version`); what belongs to a stream is in its
/// [`StreamState`] (RM-29, design 33 D15). Lock order: a segment configuration's `updates`,
/// then this, then `rec`; never held across a device call.
pub(crate) struct Streams {
    /// The instance's lines, one per stream, keyed by `(direction, k)` (RM-25, RM-29,
    /// RM-34); none for a receive side with no link.
    pub lines: timeline::Lines,
    /// The instance's receive streams; `k` is a stream's number (RM-29).
    pub rx: Vec<StreamState>,
    /// The instance's transmit streams; `k` is a stream's number (RM-29).
    pub tx: Vec<StreamState>,
    /// The arrival order of the last command booked (RM-25).
    pub seq: u64,
    /// The streams' end — a loss or `Provider::stop` — has been booked, once; a command booked
    /// after it takes no effect, by the timeline's `End` rule.
    pub ended: bool,
    /// `Provider::stop`'s instant and mode, once it has begun (UR-26).
    pub stop: Option<(i64, StopMode)>,
    /// The instant of the device's loss, once a thread found it (UR-29): the end's `S` when
    /// the loss comes first (design 38 D2 (4)).
    pub lost_at: Option<i64>,
    /// Counts the changes of the plans, for uhd-tx to read its own again.
    pub version: u64,
}

impl Default for Streams {
    /// One stream per direction in the default test state (design 33 D4).
    fn default() -> Streams {
        Streams::with_streams(1, 1)
    }
}

impl Streams {
    /// One state and one line slot per declared stream of each direction (RM-29).
    pub fn with_streams(rx: usize, tx: usize) -> Streams {
        Streams {
            lines: timeline::Lines::new(),
            rx: (0..rx.max(1)).map(|_| StreamState::default()).collect(),
            tx: (0..tx.max(1)).map(|_| StreamState::default()).collect(),
            seq: 0,
            ended: false,
            stop: None,
            lost_at: None,
            version: 0,
        }
    }
}

/// What the device holds open for a stream, as the Provider recorded it (UR-13): the channel
/// set of its streamer, or nothing known. Written after each device call, under `streams`.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) enum DeviceSet {
    /// Unknown: a failed open or a refused rate comparison, or no open yet.
    #[default]
    Unknown,
    /// The streamer is open on this set, and its rates compared.
    Open(Channels),
}

/// What a segment's owner does before it begins, from the record (UR-25): nothing, configure
/// the open streamer, or open it again and configure.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Owner {
    Nothing,
    Configure,
    Open,
}

/// One stream's device-call holder and report read (UR-25). Only [`Streams`]'s transitions
/// change them; a [`Claim`] holds a taken holder until the caller gives it back.
#[derive(Default)]
pub(crate) struct Claims {
    holder: Holder,
    reading: bool,
}

/// Who may call a stream's device now (UR-25).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Holder {
    /// Nobody: an owner or uhd-control may take the stream.
    #[default]
    Free,
    /// Its owner: a receive or transmit segment's device calls, with the move's `seq` while
    /// that command is still unrecorded or refused (RM-34, UR-25).
    Owner { seq: Option<u64> },
    /// The idle stream's own owner closing its streamer, which another line needs (UR-25, RM-34).
    Release,
    /// uhd-control's enable from no stream, from the section that decides it until the
    /// section that books it or ends it without one (UR-25).
    FromNone,
}

/// A holder taken from a stream, to be given back once the section that booked its outcome is
/// done (UR-25). The test build panics when one is dropped without [`Streams::give_back`];
/// the production build does not, and takes no lock.
#[must_use]
#[derive(Debug)]
pub(crate) struct Claim {
    stream: StreamKey,
    holder: Holder,
}

impl Drop for Claim {
    fn drop(&mut self) {
        // Not while another panic unwinds: a second panic would abort the test binary.
        #[cfg(test)]
        if !std::thread::panicking() {
            panic!("UR-25: a claim of {:?} was not given back", self.holder);
        }
    }
}

/// A planned segment, as a stream's phase and its owner name it: origin, configuration and
/// the command that began it (UR-17, UR-25).
pub(crate) type Segment = (i64, Config, Option<u64>);

/// What a stream's owner is doing with it (UR-17, UR-25). Only [`Streams`]'s transitions
/// change it; the phase is one value, so a stream the owner has not begun cannot be idle and
/// active at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Phase {
    /// The owner has nothing to begin.
    Idle,
    /// The owner has a segment or a switch to begin, or finished one and has not looked for
    /// the next.
    Pending,
    /// The owner is processing a segment; `delivered` is uhd-rx's, the first sample of the
    /// segment not yet delivered, and `segment` is absent when the plan has none where the
    /// owner is (a loss during `start`, or a transmit clock past the plan).
    Active { segment: Option<Segment>, delivered: Option<(i64, Config, i64)> },
}

impl Default for Phase {
    /// `StreamState::default` is the state before `start`: nothing begun, but not idle.
    fn default() -> Phase {
        Phase::Pending
    }
}

impl Phase {
    pub(crate) fn is_idle(self) -> bool {
        matches!(self, Phase::Idle)
    }
    pub(crate) fn active(self) -> Option<Segment> {
        match self {
            Phase::Active { segment, .. } => segment,
            _ => None,
        }
    }
    pub(crate) fn delivered(self) -> Option<(i64, Config, i64)> {
        match self {
            Phase::Active { delivered, .. } => delivered,
            _ => None,
        }
    }
}

/// One stream's own state (RM-29, design 33 D15): stream `k` of its direction, and not the
/// instance's other streams.
#[derive(Default)]
pub(crate) struct StreamState {
    /// The configuration each segment begins with, by the command that began it (UR-25).
    pub configs: BTreeMap<u64, Arc<ColdConfig>>,
    /// The device's record of the stream's streamer (UR-13, UR-36): its channel set.
    pub set: DeviceSet,
    /// The streamer's open generation: every successful open of the stream — whoever opened it,
    /// uhd-control's enable from no stream included — is a new streamer with a new report queue,
    /// which the correlations of the bursts in flight are told from (UR-36, UR-28).
    pub generation: u64,
    /// The command of the segment whose configuration the device was last given, `seq`; none
    /// after a failed configure or open. The owner configures again unless this is its own
    /// segment's command and the record holds its set open (UR-36).
    pub configured: Option<u64>,
    /// Segments its owner finished processing; a segment with a finite cut remains needed
    /// until its owner records completion (A2).
    pub completed: Vec<Segment>,
    /// Who may call the stream's device and whether uhd-tx's report read is in flight
    /// (UR-25).
    claims: Claims,
    /// What the owner is doing with the stream (UR-17, UR-25).
    phase: Phase,
    /// The transmit clocks registered, in the order of the transmit plan's segments, and
    /// whether each has ended (RM-25).
    pub clocks: Vec<(Clock, bool)>,
}

/// The key the instance's lines hold `dir`'s stream `k` under (RM-29).
pub(crate) fn stream_key(dir: Dir, k: usize) -> StreamKey {
    match dir {
        Dir::Rx => (Direction::Rx, k as u32),
        Dir::Tx => (Direction::Tx, k as u32),
    }
}

/// The direction a stream's key names (RM-29).
pub(crate) fn dir_of(stream: StreamKey) -> Dir {
    match stream.0 {
        Direction::Rx => Dir::Rx,
        Direction::Tx => Dir::Tx,
    }
}

/// The stream the command names (RM-29).
#[cfg(test)]
pub(crate) fn booked_stream(dir: Dir) -> StreamKey {
    stream_key(dir, 0)
}

impl Streams {
    /// A stream's state, by its key (RM-29).
    pub fn stream(&self, stream: StreamKey) -> &StreamState {
        match stream.0 {
            Direction::Rx => &self.rx[stream.1 as usize],
            Direction::Tx => &self.tx[stream.1 as usize],
        }
    }

    /// A stream's state, to change (RM-29).
    pub fn stream_mut(&mut self, stream: StreamKey) -> &mut StreamState {
        match stream.0 {
            Direction::Rx => &mut self.rx[stream.1 as usize],
            Direction::Tx => &mut self.tx[stream.1 as usize],
        }
    }

    /// Who may call the stream's device now (UR-25).
    pub fn holder(&self, stream: StreamKey) -> Holder {
        self.stream(stream).claims.holder
    }

    /// Whether uhd-tx's report read is in flight (UR-25).
    pub fn reading(&self, stream: StreamKey) -> bool {
        self.stream(stream).claims.reading
    }

    /// UR-25: takes `stream` for its owner's device calls; `None`, with nothing changed, when
    /// another holder has it.
    #[must_use]
    pub fn claim_owner(&mut self, stream: StreamKey, seq: Option<u64>) -> Option<Claim> {
        let claims = &mut self.stream_mut(stream).claims;
        if claims.holder != Holder::Free {
            return None;
        }
        claims.holder = Holder::Owner { seq };
        Some(Claim { stream, holder: Holder::Owner { seq } })
    }

    /// UR-25, RM-34: takes an idle, active-less `stream` for its owner's close of a
    /// streamer another line needs; `None` when it is held or not idle.
    #[must_use]
    pub fn claim_release(&mut self, stream: StreamKey) -> Option<Claim> {
        let idle = self.is_idle(stream);
        let active = self.active(stream).is_some();
        let state = self.stream_mut(stream);
        if state.claims.holder != Holder::Free || !idle || active {
            return None;
        }
        state.claims.holder = Holder::Release;
        Some(Claim { stream, holder: Holder::Release })
    }

    /// UR-25: takes `stream` for uhd-control's enable from no stream. A report read in flight
    /// does not refuse it: the enable waits for that read before its first device call.
    #[must_use]
    pub fn claim_from_none(&mut self, stream: StreamKey) -> Option<Claim> {
        let claims = &mut self.stream_mut(stream).claims;
        if claims.holder != Holder::Free {
            return None;
        }
        claims.holder = Holder::FromNone;
        Some(Claim { stream, holder: Holder::FromNone })
    }

    /// UR-25: the from-none enable holds the stream and no report read is in flight, so its
    /// first device call may be made.
    pub fn may_call_from_none(&self, stream: StreamKey) -> bool {
        let claims = &self.stream(stream).claims;
        claims.holder == Holder::FromNone && !claims.reading
    }

    /// UR-25: uhd-tx takes the stream for a report read; `false`, with nothing changed, when
    /// its holder is not `Free` or a read is already in flight.
    pub fn begin_read(&mut self, stream: StreamKey) -> bool {
        let claims = &mut self.stream_mut(stream).claims;
        if claims.holder != Holder::Free || claims.reading {
            return false;
        }
        claims.reading = true;
        true
    }

    /// UR-25: the shutdown's report read takes no holder check, so that a detached uhd-control
    /// left in `FromNone` cannot hold it (UR-16).
    pub fn begin_shutdown_read(&mut self, stream: StreamKey) {
        self.stream_mut(stream).claims.reading = true;
    }

    /// UR-25: the report read in flight ends; in the test build, one that never began panics.
    pub fn end_read(&mut self, stream: StreamKey) {
        let claims = &mut self.stream_mut(stream).claims;
        if !claims.reading {
            #[cfg(test)]
            panic!("UR-25: end_read without a read in flight");
            #[cfg(not(test))]
            return;
        }
        claims.reading = false;
    }

    /// UR-25: gives a taken holder back. The test build panics if the stream no longer holds
    /// what the claim took; the production build leaves the stream as it is.
    pub fn give_back(&mut self, claim: Claim) {
        let (stream, holder) = (claim.stream, claim.holder);
        let current = self.holder(stream);
        if current != holder {
            std::mem::forget(claim);
            #[cfg(test)]
            panic!("UR-25: a claim of {holder:?} was given back to {current:?}");
            #[cfg(not(test))]
            return;
        }
        self.stream_mut(stream).claims.holder = Holder::Free;
        std::mem::forget(claim);
    }

    /// How many streams of `dir` the instance holds (RM-29).
    pub fn count(&self, dir: Dir) -> usize {
        match dir {
            Dir::Rx => self.rx.len(),
            Dir::Tx => self.tx.len(),
        }
    }

    /// UR-25: `start` gives the stream `held` (it has a segment from T0, or a clock) or
    /// nothing to begin.
    pub fn starting(&mut self, stream: StreamKey, held: bool) {
        self.stream_mut(stream).phase = if held { Phase::Pending } else { Phase::Idle };
    }

    /// UR-25: uhd-rx's search for the next planned segment found one or none.
    pub fn search(&mut self, stream: StreamKey, found: bool) {
        self.stream_mut(stream).phase = if found { Phase::Pending } else { Phase::Idle };
    }

    /// UR-17, UR-25: uhd-rx begins a segment; `delivered` is the first sample not delivered
    /// (zero), named by the clock and configuration uhd-rx's stream has.
    pub fn started(&mut self, stream: StreamKey, segment: Option<Segment>, origin: i64, config: Config) {
        self.stream_mut(stream).phase = Phase::Active { segment, delivered: Some((origin, config, 0)) };
    }

    /// UR-17: uhd-rx delivered its segment up to `k`.
    pub fn delivered(&mut self, stream: StreamKey, origin: i64, config: Config, k: i64) {
        let phase = &mut self.stream_mut(stream).phase;
        if let Phase::Active { delivered, .. } = phase {
            *delivered = Some((origin, config, k));
        }
    }

    /// UR-17, UR-25: the owner finishes its segment: its phase clears and the segment is
    /// returned for `complete`, by the caller that records it.
    pub fn finished(&mut self, stream: StreamKey) -> Option<Segment> {
        let phase = &mut self.stream_mut(stream).phase;
        let active = std::mem::replace(phase, Phase::Pending);
        active.active()
    }

    /// UR-25: uhd-tx reads its plan: `clock` is its current clock, `switch` a change it has
    /// not made, `segment` the planned segment at its clock's index.
    pub fn plan_read(&mut self, stream: StreamKey, clock: bool, switch: bool, segment: Option<Segment>) {
        self.stream_mut(stream).phase = if clock {
            Phase::Active { segment, delivered: None }
        } else if switch {
            Phase::Pending
        } else {
            Phase::Idle
        };
    }

    /// The stream's phase, as the transitions left it (UR-25).
    pub fn phase(&self, stream: StreamKey) -> Phase {
        self.stream(stream).phase
    }

    /// UR-25: the owner has nothing to begin.
    pub fn is_idle(&self, stream: StreamKey) -> bool {
        self.phase(stream).is_idle()
    }

    /// The segment the stream's owner is processing (A2).
    pub fn active(&self, stream: StreamKey) -> Option<Segment> {
        self.phase(stream).active()
    }

    /// UR-17, RM-16: the first sample of the segment uhd-rx is delivering that it has not yet
    /// delivered.
    pub fn first_undelivered(&self, stream: StreamKey) -> Option<(i64, Config, i64)> {
        self.phase(stream).delivered()
    }

    /// The line of `stream`, if it has one.
    pub fn line(&self, stream: StreamKey) -> Option<&Line> {
        self.lines.line(stream)
    }

    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Books a command or fault on `stream`'s timeline; the end of a receive stream with the
    /// first sample uhd-rx has not delivered of the segment it cuts (RM-16). False when the
    /// stream has no line, or when its plan cannot be represented, which refuses it.
    pub fn book(&mut self, core: &Core, stream: StreamKey, mut item: Item) -> bool {
        let delivered = self.first_undelivered(stream);
        let Some(line) = self.lines.line(stream) else { return false };
        if stream.0 == Direction::Rx && matches!(item.kind, Kind::End { .. }) {
            let cuts = |origin, config| line.made(origin, config, item.e, item.seq).ok().flatten().is_some_and(|s| s.cut.is_none());
            item.delivered = item.delivered.or(delivered.filter(|(origin, config, _)| cuts(*origin, *config)).map(|(_, _, k)| k));
        }
        if let Err(error) = self.lines.book(stream, item) {
            core.command_rejected(None, "update_parameter", &format!("RM-25: {error}"));
            return false;
        }
        self.changed(core, stream);
        true
    }

    /// RM-25, VH-2: the device refused the configuration of the segment command `seq` began;
    /// the command is booked as refused, which halts the stream. False when it is no longer
    /// booked, or when the stream has no line.
    pub fn refuse(&mut self, core: &Core, stream: StreamKey, seq: u64) -> bool {
        if self.lines.line(stream).is_none() {
            return false;
        }
        let moved = self.lines.move_lines(seq);
        match self.lines.refuse(stream, seq) {
            Ok(refused) => {
                if let Some((from, to)) = moved.filter(|_| refused) {
                    self.changed(core, from);
                    self.changed(core, to);
                } else {
                    self.changed(core, stream);
                }
                refused
            }
            Err(error) => {
                core.command_rejected(None, "update_parameter", &format!("RM-25: {error}"));
                false
            }
        }
    }

    /// RM-16, VH-2: a loss or `Provider::stop` at `at` ends every stream, once, each line with
    /// the same instant and arrival order; a loss counts as received with the faults, at the
    /// Run's start (RM-25).
    pub fn end(&mut self, core: &Core, at: i64, abort: bool, lost: bool) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        let seq = if lost { 0 } else { self.next_seq() };
        for dir in [Dir::Rx, Dir::Tx] {
            for k in 0..self.count(dir) {
                self.book(core, stream_key(dir, k), Item { e: at, seq, ready: at, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::End { abort } });
            }
        }
    }

    /// UR-26, design 38 D2 (4): `S`, the instant the Run's end was booked at, and whether a
    /// loss was its cause — the stop's or the loss's, whichever came first; a loss at the same
    /// instant counts as first, as MockRadio's order does.
    pub fn ended_at(&self) -> Option<(i64, bool)> {
        match (self.stop.map(|(at, _)| at), self.lost_at) {
            (Some(stop), Some(lost)) if lost <= stop => Some((lost, true)),
            (Some(stop), _) => Some((stop, false)),
            (None, Some(lost)) => Some((lost, true)),
            (None, None) => None,
        }
    }

    /// #63: once its owner has ended the segment command `by` began, the items before that
    /// command are pruned: nothing booked later takes effect before it, and no command before
    /// it can be refused any more.
    pub fn prune(&mut self, stream: StreamKey, by: Option<u64>) {
        let Some(e) = by.and_then(|by| self.lines.line(stream).and_then(|held| held.items.iter().find(|item| item.seq == by).map(|item| timeline::effective(&held.items, item)))) else { return };
        self.lines.prune_stream(stream, e);
        let kept: Vec<u64> = self.lines.line(stream).map(|line| line.items.iter().map(|item| item.seq).collect()).unwrap_or_default();
        self.stream_mut(stream).configs.retain(|seq, _| kept.contains(seq));
    }

    /// The plan of `stream`, each segment with its configuration and its transmit clock.
    pub fn planned(&self, stream: StreamKey) -> Vec<Planned> {
        let Some(line) = self.lines.line(stream) else { return Vec::new() };
        let state = self.stream(stream);
        line.plan
            .iter()
            .enumerate()
            .map(|(index, segment)| Planned {
                segment: *segment,
                k: stream.1 as usize,
                settings: segment.by.and_then(|by| state.configs.get(&by)).cloned(),
                refused: segment.by.is_some_and(|by| line.items.iter().any(|item| item.seq == by && item.refused)),
                clock: (stream.0 == Direction::Tx).then(|| state.clocks.get(index).map(|(clock, _)| *clock)).flatten(),
            })
            .collect()
    }

    /// What the owner of `stream` does before it begins a segment that holds `set`, begun by
    /// the command `by` (UR-36): nothing when the record has that set open and configured for
    /// that command; a configure when only the command differs; an open otherwise.
    pub fn owner_step(&self, stream: StreamKey, set: Channels, by: Option<u64>) -> Owner {
        let state = self.stream(stream);
        match state.set {
            DeviceSet::Open(open) if open == set => {
                if state.configured.is_some() && state.configured == by {
                    Owner::Nothing
                } else {
                    Owner::Configure
                }
            }
            _ => Owner::Open,
        }
    }

    /// Another stream's recorded streamer owns any channel in `set` (D2, D3).
    pub fn held_elsewhere(&self, stream: StreamKey, set: Channels) -> Option<StreamKey> {
        (0..self.count(dir_of(stream))).map(|k| stream_key(dir_of(stream), k)).find(|other| {
            *other != stream
                && match self.stream(*other).set {
                    DeviceSet::Open(open) => open.iter().any(|channel| set.contains(channel)),
                    DeviceSet::Unknown => false,
                }
        })
    }

    /// An earlier, unprocessed segment owns the channel before `origin` (A2's finite chain).
    pub fn needed_before(&self, stream: StreamKey, set: Channels, origin: i64) -> bool {
        (0..self.count(dir_of(stream))).map(|k| stream_key(dir_of(stream), k)).any(|other| {
            if other == stream {
                return false;
            }
            let state = self.stream(other);
            self.planned(other).iter().any(|planned| {
                let identity = (planned.segment.origin, planned.segment.config, planned.segment.by);
                planned.segment.origin <= origin
                    && !planned.refused
                    && !state.completed.contains(&identity)
                    && !planned.segment.cut.is_some_and(|cut| planned.segment.instant(cut).is_ok_and(|end| end <= planned.segment.origin))
                    && planned.segment.config.channels.iter().any(|channel| set.contains(channel))
            })
        })
    }

    /// A move destination waits only for its source's configuration of that same move.
    pub fn move_source_busy(&self, stream: StreamKey, seq: Option<u64>) -> bool {
        let Some(seq) = seq else { return false };
        self.lines.move_lines(seq).is_some_and(|(from, to)| to == stream && self.holder(from) == Holder::Owner { seq: Some(seq) })
    }

    /// Records a segment its owner finished carrying out.
    pub fn complete(&mut self, stream: StreamKey, segment: (i64, Config, Option<u64>)) {
        let completed = &mut self.stream_mut(stream).completed;
        if !completed.contains(&segment) {
            completed.push(segment);
        }
    }

    /// The record of an open of `stream`'s streamer: the set it is open on, or unknown after a
    /// failed open; the generation moves on for each successful open, so that a correlation
    /// against the replaced streamer's reports is told from the new one's (UR-28, UR-36). Any
    /// configuration it had is no longer known (UR-13).
    pub fn record_open(&mut self, stream: StreamKey, set: Option<Channels>) {
        let state = self.stream_mut(stream);
        state.generation += u64::from(set.is_some());
        state.set = set.map_or(DeviceSet::Unknown, DeviceSet::Open);
        state.configured = None;
    }

    /// The record of one open and its rate comparison: a device refusal leaves the set
    /// unknown with the generation as it was; a successful open moves it on, and a refused
    /// comparison after it leaves the set and the configuration unknown (UR-13, UR-36).
    pub fn record_opened(&mut self, stream: StreamKey, set: Channels, opened: &Result<(), DeviceError>, checked: &Result<(), DeviceError>) {
        match (opened, checked) {
            (Err(_), _) => self.record_open(stream, None),
            (Ok(()), Ok(())) => self.record_open(stream, Some(set)),
            (Ok(()), Err(_)) => {
                self.record_open(stream, Some(set));
                self.record_refused(stream);
            }
        }
    }

    /// A rate comparison refused after a successful open: the streamer is the new one all the
    /// same, so the generation stands, and its set and configuration are unknown (UR-13, UR-36).
    pub fn record_refused(&mut self, stream: StreamKey) {
        let state = self.stream_mut(stream);
        state.set = DeviceSet::Unknown;
        state.configured = None;
    }

    /// The record of a configure of `stream`: its segment's command once it succeeded, none
    /// when it failed (UR-36).
    pub fn record_configured(&mut self, stream: StreamKey, mark: Option<u64>) {
        self.stream_mut(stream).configured = mark;
    }

    /// A plan changed: the transmit clocks follow it (RM-25, VH-2).
    pub fn changed(&mut self, core: &Core, stream: StreamKey) {
        self.version += 1;
        if stream.0 == Direction::Tx {
            if let Err(error) = self.reconcile(core, stream) {
                core.command_rejected(None, "update_parameter", &format!("UR-25: {error}"));
            }
        }
        let planned: Vec<_> = self.lines.line(stream).into_iter().flat_map(|line| &line.plan).map(|segment| (segment.origin, segment.config, segment.by)).collect();
        self.stream_mut(stream).completed.retain(|done| planned.contains(done));
    }

    /// Each transmit segment's clock, registered when its change is booked and ended at its
    /// cut once the plan has one; a clock registered for a change the plan no longer has — one
    /// booked after `Provider::stop`'s instant — ends at its origin (KC-21a, VH-2).
    fn reconcile(&mut self, core: &Core, stream: StreamKey) -> Result<(), String> {
        let plan = self.lines.line(stream).map_or_else(Vec::new, |line| line.plan.clone());
        for (index, segment) in plan.iter().enumerate() {
            if self.stream(stream).clocks.len() == index {
                let clock = core.register_in(dir_of(stream), stream.1 as usize, segment.config.ratio.num() as i64, segment.origin)?;
                self.stream_mut(stream).clocks.push((clock, false));
            }
            let (clock, ended) = &mut self.stream_mut(stream).clocks[index];
            if let Some(cut) = segment.cut.filter(|_| !*ended) {
                core.clocks.end(clock.domain, core.at(clock.instant(cut))).map_err(|e| e.to_string())?;
                *ended = true;
            }
        }
        let state = self.stream_mut(stream);
        for (clock, ended) in state.clocks.iter_mut().skip(plan.len()).filter(|(_, ended)| !*ended) {
            if let Err(error) = core.clocks.end(clock.domain, core.at(clock.origin)) {
                core.reject_note(json!({ "clock_not_ended": error.to_string() }));
            }
            *ended = true;
        }
        Ok(())
    }

    /// UR-30: the plans the streams were carried out by, each segment's stream, the command
    /// that began it, its origin, end, logical channels and root ticks per sample.
    pub fn record(&self) -> Json {
        let plan = |dir: Dir| -> Vec<Json> {
            let mut rows = Vec::new();
            for k in 0..self.count(dir) {
                let Some(line) = self.lines.line(stream_key(dir, k)) else { continue };
                rows.extend(line.plan.iter().map(|segment| {
                    json!({
                        "stream": k,
                        "sequence": segment.by,
                        "origin": segment.origin,
                        "end": segment.cut.and_then(|cut| segment.instant(cut).ok()),
                        "channels": segment.config.channels.iter().collect::<Vec<u32>>(),
                        "ticks_per_sample": [segment.config.ratio.num(), segment.config.ratio.den()],
                    })
                }));
            }
            rows
        };
        json!({ "what": "plan", "rx": plan(Dir::Rx), "tx": plan(Dir::Tx) })
    }
}

/// Everything the threads record; written to the sections at `cleanup` (UR-30).
pub(crate) struct Records {
    pub applied: Vec<Json>,
    pub bursts: Vec<Json>,
    pub device_async: Vec<Json>,
    pub rejected: Vec<Json>,
    pub timing: Vec<Json>,
    pub stats: BTreeMap<&'static str, i64>,
}

impl Default for Records {
    fn default() -> Self {
        let stats = ["rx_blocks", "rx_samples", "rx_overflows", "rx_errors", "rx_off_lattice", "rx_overlapping", "rx_before_origin", "link_drops_seen", "tx_bursts", "tx_samples", "tx_errors"]
            .into_iter()
            .map(|name| (name, 0))
            .collect();
        Records { applied: Vec::new(), bursts: Vec::new(), device_async: Vec::new(), rejected: Vec::new(), timing: Vec::new(), stats }
    }
}

/// The context `prepare` gathered, shared by uhd-control, uhd-rx and uhd-tx.
pub(crate) struct Core {
    pub device: Arc<dyn Device>,
    /// Serializes streamer open/close admission with publishing device loss (UR-29).
    device_gate: Mutex<()>,
    /// Each receive chain's owner transitions and control handovers share a clock lock (L1).
    pub rx_chain: Vec<ClockMutex<RxChain>>,
    /// A timed TX DSP write and the open-time flush share each channel's gate (D9b-1a).
    tx_dsp_gates: Vec<ClockMutex<()>>,
    /// Every tx_async reader, including shutdown, shares one per-stream gate.
    tx_report_gates: Vec<ClockMutex<()>>,
    tx_report_stash: Vec<Mutex<VecDeque<TxReport>>>,
    pub id: ResourceId,
    /// Stream 0's node of each direction (tests).
    #[cfg(test)]
    pub rx_id: ResourceId,
    #[cfg(test)]
    pub tx_id: ResourceId,
    pub mcr: u64,
    pub root: ClockDomainId,
    pub time: Arc<dyn TimeAuthority>,
    pub clocks: Arc<ClockRegistry>,
    pub events: Arc<dyn EventSink>,
    /// One hot-path handle per receive stream (UR-20, RM-29).
    pub overflow: Vec<EventHandle>,
    pub inputs: Arc<dyn InputStore>,
    /// Each receive stream's links, by its number (RM-29): the port `rx<k>`'s.
    pub links: Vec<Vec<Arc<dyn DataLink>>>,
    /// The description built from the binding's layout (RM-29).
    pub description: ezsdr_radio::channels::Description,
    pub block_len: usize,
    pub rec: Mutex<Records>,
    pub streams: Mutex<Streams>,
    /// Held burst starts per transmit clock, for UR-21's duplicate refusal.
    pub held: Mutex<BTreeSet<(ClockDomainId, i64)>>,
    /// UR-13's rate-set comparison, once per direction: `None` until the direction's
    /// first streamer opened, then the check's outcome (design 33 D5).
    rates: Mutex<[Option<Result<(), String>>; 2]>,
    /// `S`, the end of the device's start-up (UR-13).
    pub start_up: AtomicI64,
    lost: AtomicBool,
    /// The test build's probes, by section name (UR-25, UR-29).
    #[cfg(test)]
    pub probes: Mutex<BTreeMap<&'static str, Probe>>,
}

pub(crate) struct TxFlushResult {
    pub acknowledged: bool,
    pub ack_tick: Option<i64>,
    pub stopped: bool,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Arc<dyn Device>,
        id: ResourceId,
        root: ClockDomainId,
        time: Arc<dyn TimeAuthority>,
        clocks: Arc<ClockRegistry>,
        events: Arc<dyn EventSink>,
        inputs: Arc<dyn InputStore>,
        mut links: Vec<Vec<Arc<dyn DataLink>>>,
        description: ezsdr_radio::channels::Description,
    ) -> Result<Core, ModuleError> {
        #[cfg(test)]
        let rx_id = description.stream_node(&id, Dir::Rx, 0);
        #[cfg(test)]
        let tx_id = description.stream_node(&id, Dir::Tx, 0);
        let rx_count = description.stream_count(Dir::Rx);
        let tx_count = description.stream_count(Dir::Tx);
        let tx_channel_count = device.channels(Dir::Tx);
        let host = device.host();
        links.resize_with(rx_count, Vec::new);
        // UR-12, UR-20: every receive stream's node is below the instance root the Kernel
        // handed this instance, so the bound view resolves it; a refusal is a Module error,
        // not a Kernel event (item 3).
        let mut overflow = Vec::with_capacity(rx_count);
        for k in 0..rx_count {
            let node = description.stream_node(&id, Dir::Rx, k);
            overflow.push(
                events
                    .resolve(&EventSource::Node { node }, &EventKind::parse(kinds::RX_OVERFLOW).expect("a radio kind"))
                    .map_err(|error| ModuleError::rejected(format!("UR-12: the rx stream's event handle: {error}")))?,
            );
        }
        Ok(Core {
            mcr: device.master_clock_rate(),
            block_len: description.block_len as usize,
            device,
            device_gate: Mutex::new(()),
            rx_chain: (0..rx_count).map(|_| host.clock_mutex(RxChain::default())).collect(),
            tx_dsp_gates: (0..tx_channel_count).map(|_| host.clock_mutex(())).collect(),
            tx_report_gates: (0..tx_count).map(|_| host.clock_mutex(())).collect(),
            tx_report_stash: (0..tx_count).map(|_| Mutex::new(VecDeque::new())).collect(),
            id,
            #[cfg(test)]
            rx_id,
            #[cfg(test)]
            tx_id,
            root,
            time,
            clocks,
            events,
            overflow,
            inputs,
            links,
            description,
            rec: Mutex::new(Records::default()),
            streams: Mutex::new(Streams::with_streams(rx_count, tx_count)),
            held: Mutex::new(BTreeSet::new()),
            rates: Mutex::new([None, None]),
            start_up: AtomicI64::new(i64::MIN),
            lost: AtomicBool::new(false),
            #[cfg(test)]
            probes: Mutex::new(BTreeMap::new()),
        })
    }

    /// Receive stream `k`'s links (RM-29): the port `rx<k>`'s.
    pub fn links(&self, k: usize) -> &[Arc<dyn DataLink>] {
        self.links.get(k).map_or(&[], Vec::as_slice)
    }

    /// Receive stream `k`'s RX_OVERFLOW handle (UR-20).
    pub fn overflow(&self, k: usize) -> EventHandle {
        self.overflow[k]
    }

    /// A test's synchronisation point, by section name: what a wrong ownership or lock order
    /// would pass through unseen. Nothing outside the test build.
    pub(crate) fn probe(&self, name: &'static str, streams: Option<&Streams>) {
        #[cfg(test)]
        {
            let probe = self.probes.lock().unwrap().get(name).cloned();
            if let Some(probe) = probe {
                probe(streams);
            }
        }
        #[cfg(not(test))]
        let _ = (name, streams);
    }

    /// Opens and closes of streamers cannot cross the publication of device loss.
    fn lock_device_gate(&self) -> MutexGuard<'_, ()> {
        match self.device_gate.try_lock() {
            Ok(gate) => gate,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.probe("device_gate_waiting", None);
                self.device_gate.lock().unwrap_or_else(|error| error.into_inner())
            }
        }
    }

    /// The current instant on the primary root.
    pub fn now(&self) -> i64 {
        self.time.now(self.root).and_then(|t| t.ticks_in(self.root)).unwrap_or(i64::MIN)
    }

    pub fn at(&self, ticks: i64) -> TimePoint {
        TimePoint::new(self.root, ticks)
    }

    /// Nanoseconds as root ticks, rounded up.
    pub fn ticks(&self, ns: i64) -> i64 {
        ((ns as i128 * self.mcr as i128 + 999_999_999) / 1_000_000_000) as i64
    }

    /// Root ticks as nanoseconds.
    pub fn ns(&self, ticks: i64) -> i64 {
        (ticks as i128 * 1_000_000_000 / self.mcr as i128) as i64
    }

    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    pub fn stat(&self, name: &'static str, n: i64) {
        *lock(&self.rec).stats.entry(name).or_default() += n;
    }

    pub fn timing(&self, row: Json) {
        lock(&self.rec).timing.push(row);
    }

    pub fn reject_note(&self, row: Json) {
        lock(&self.rec).rejected.push(row);
    }

    /// A control-path event from `source` at `time` (RM-11, UR-20).
    pub fn emit_at(&self, source: &ResourceId, kind: &str, severity: Severity, payload: Json, time: TimePoint) {
        let event = Event { source: EventSource::Node { node: source.clone() }, time, severity, kind: EventKind::parse(kind).expect("a radio kind"), payload };
        if let Err(error) = self.events.emit_control(event) {
            self.reject_note(json!({ "event_not_emitted": kind, "error": error.to_string() }));
        }
    }

    pub fn emit(&self, source: &ResourceId, kind: &str, severity: Severity, payload: Json) {
        self.emit_at(source, kind, severity, payload, self.at(self.now()));
    }

    /// `radio.COMMAND_REJECTED` from the device, naming the Action by `id` when uhd-control
    /// refuses it as it takes it, and its reason recorded (UR-21, UR-30, RM-22).
    pub fn command_rejected(&self, id: Option<ActionId>, kind: &str, reason: &str) {
        self.reject_note(json!({ "kind": kind, "reason": reason, "at": self.at(self.now()) }));
        let payload = serde_json::to_value(CommandRejectedPayload { action: id, kind: kind.to_owned(), reason: reason.to_owned() }).expect("a payload");
        self.emit(&self.id, kinds::COMMAND_REJECTED, Severity::Error, payload);
    }

    /// MA-9a: `DEVICE_LOST`, once, from this Provider; then no thread uses the device. The
    /// thread that finds the loss books it on both streams, before any thread can see the
    /// device lost (RM-16).
    pub fn device_lost(&self, message: &str) {
        assert_line_free("device_lost");
        let _device_gate = self.lock_device_gate();
        self.device_lost_locked(message);
    }

    /// Publishes loss while the caller owns `device_gate`.
    fn device_lost_locked(&self, message: &str) {
        let mut streams = lock(&self.streams);
        if self.lost.load(Ordering::Acquire) {
            return;
        }
        // D2 (4): the loss's own instant, read once, is the end's `S`.
        let at = self.now();
        streams.lost_at = streams.lost_at.or(Some(at));
        streams.end(self, at, false, true);
        self.lost.store(true, Ordering::Release);
        drop(streams);
        // Before anything can free it: a lost device is never freed (F4).
        self.device.mark_lost();
        self.timing(json!({ "what": "device_lost", "message": message, "at": at }));
        self.emit(&self.id, EventKind::DEVICE_LOST, Severity::Fatal, json!({ "message": message }));
    }

    /// A device failure: `DEVICE_LOST` when the device is gone, else the reason.
    pub fn device_failed(&self, id: Option<ActionId>, action: &str, error: &DeviceError) {
        if error.lost {
            self.device_lost(&error.message);
        } else {
            self.command_rejected(id, action, &error.message);
        }
    }

    /// The node stream `k` of `dir` reports from (RM-29, UR-17, UR-20): `<id>/rx` and
    /// `<id>/tx/s<k>`. The one place a Provider reads a stream's event source from.
    pub fn stream_node(&self, dir: Dir, k: usize) -> ResourceId {
        self.description.stream_node(&self.id, dir, k)
    }

    /// UR-13, design 33 D5: the rate set the device reports for one channel of `dir` is the
    /// profile's, compared by set once — after the direction's first streamer opens — and
    /// the outcome held for every later open. A mismatch is the refusal `UR-13:`.
    pub fn check_rates(&self, dir: Dir, chan: usize) -> Result<(), DeviceError> {
        self.probe("rates", None);
        if self.is_lost() {
            return Err(DeviceError { lost: true, message: "UR-29: the device is lost before the rate comparison".to_owned() });
        }
        {
            let checks = lock(&self.rates);
            if let Some(outcome) = &checks[dir as usize] {
                return outcome.clone().map_err(DeviceError::failed);
            }
        }
        // The device is read outside the lock: no device call is made under one (UR-25).
        // Two directions never share a slot and a repeated read of one answers the same
        // set, so the first outcome stored is the one every later open sees.
        self.probe("rates_before_gate", None);
        let device_gate = self.lock_device_gate();
        if self.is_lost() {
            return Err(DeviceError { lost: true, message: "UR-29: the device is lost before the rate comparison".to_owned() });
        }
        let outcome = match self.device.rates(dir, chan) {
            Ok(ranges) if ezsdr_radio::device::rate_set_matches(&self.description.rates, &ranges) => Ok(()),
            Ok(_) => Err(format!("UR-13: the {} rates the device reports are not the profile's rate set", dir.name())),
            Err(error) => Err(format!("UR-13: {}", error.message)),
        };
        drop(device_gate);
        lock(&self.rates)[dir as usize] = Some(outcome.clone());
        self.probe("rates_returned", None);
        outcome.map_err(DeviceError::failed)
    }

    /// UR-13, UR-30: whether a direction's rate set has been compared yet; the Manifest's
    /// `device` section records it.
    pub fn rates_checked(&self, dir: Dir) -> bool {
        lock(&self.rates)[dir as usize].is_some()
    }

    /// **Configuring a direction** (UR-12): each given channel's own settings, untimed,
    /// read back, the applied rows under the mode's keys — the rate's is stream `k`'s key,
    /// a channel's setting its own —; the stream's rate must be exactly
    /// the claim on every channel, a decimation of the master clock
    /// ([`crate::device::decimation`]). Returns that decimation `N`.
    pub fn configure(&self, dir: Dir, channels: &[(usize, Settings)], rate: f64, k: usize) -> Result<i64, DeviceError> {
        for (chan, settings) in channels {
            self.probe("apply", None);
            // UR-29: the device was lost between two channels of one configuration; the rest
            // is not applied (the owner's configure comes through here too).
            if self.is_lost() {
                return Err(DeviceError { lost: true, message: "UR-29: the device is lost before the next channel's apply".to_owned() });
            }
            let applied = self.device.apply(dir, *chan, settings, None)?;
            let at = self.at(self.now());
            let mut rec = lock(&self.rec);
            for (name, claimed, read) in [("sample_rate_hz", Some(rate), applied.rate), ("frequency_hz", settings.freq, applied.freq), ("gain_db", settings.gain, applied.gain)] {
                rec.applied.push(json!({
                    "key": self.description.applied_key(dir, *chan, k, name), "channel": chan,
                    "claimed": claimed, "read_back": read, "difference": claimed.map(|c| read - c), "at": at,
                }));
            }
            if let Some(antenna) = &settings.antenna {
                rec.applied.push(json!({
                    "key": self.description.applied_key(dir, *chan, k, "antenna"), "channel": chan,
                    "claimed": antenna, "read_back": applied.antenna, "at": at,
                }));
            }
            drop(rec);
            if applied.rate != rate {
                return Err(DeviceError::failed(format!("UR-12: the device applied {} S/s for the claimed {rate}", applied.rate)));
            }
            if let (Some(antenna), read) = (settings.antenna.as_ref(), applied.antenna.as_deref()) {
                if read != Some(antenna.as_str()) {
                    return Err(DeviceError::failed(format!(
                        "UR-12: the device reads back {} for the claimed {antenna} antenna of {}",
                        read.unwrap_or("no antenna"),
                        self.description.applied_key(dir, *chan, k, "antenna")
                    )));
                }
            }
        }
        decimation(self.mcr, rate).map(|n| n as i64).ok_or_else(|| DeviceError::failed(format!("UR-12: {rate} S/s is no decimation of {} Hz", self.mcr)))
    }

    /// UR-13, UR-36: the device's open of `k`'s streamer of `dir` on `set`; the record is the
    /// caller's (see `record_opened`), so that an enable from no stream records its open in the
    /// line-lock section it already takes (UR-25).
    pub fn lock_tx_dsp(&self, channels: &[usize]) -> Vec<ClockGuard<'_, ()>> {
        let mut channels = channels.to_vec();
        channels.sort_unstable();
        channels.dedup();
        channels.into_iter().filter_map(|channel| self.tx_dsp_gates.get(channel)).map(ClockMutex::lock).collect()
    }

    #[cfg(test)]
    pub(super) fn tx_dsp_waiters(&self, channel: usize) -> usize {
        self.tx_dsp_gates.get(channel).map_or(0, ClockMutex::waiters)
    }

    pub fn lock_tx_reports(&self, stream: usize) -> ClockGuard<'_, ()> {
        self.tx_report_gates[stream].lock()
    }

    pub fn take_stashed_tx_report(&self, stream: usize) -> Option<TxReport> {
        lock(&self.tx_report_stash[stream]).pop_front()
    }

    pub fn stash_tx_report(&self, stream: usize, report: TxReport) {
        lock(&self.tx_report_stash[stream]).push_back(report);
    }

    fn record_tx_async(&self, stream: usize, report: TxReport) {
        lock(&self.rec).device_async.push(json!({
            "code": format!("{:?}", report.code), "tick": report.tick, "channel": report.channel, "stream": stream, "tx_flush": true,
        }));
    }

    /// The open-time two-zero burst clears stale TX header state. One device-clock deadline
    /// bounds both report waits and immediate-error/unrelated-report progress.
    #[cfg(test)]
    pub fn tx_flush(&self, stream: usize, channels: &[usize]) -> TxFlushResult {
        if channels.is_empty() || self.is_lost() {
            return TxFlushResult { acknowledged: channels.is_empty(), ack_tick: None, stopped: self.is_lost() };
        }
        let _dsp = self.lock_tx_dsp(channels);
        let _reports = self.lock_tx_reports(stream);
        self.tx_flush_locked(stream, channels)
    }

    fn tx_flush_locked(&self, stream: usize, channels: &[usize]) -> TxFlushResult {
        let expected: BTreeSet<usize> = (0..channels.len()).collect();
        let mut acknowledged = BTreeSet::new();
        let mut ack_tick: Option<i64> = None;
        let mut problem = None;
        let zeros = [Iq::default(); 2];
        let buffers: Vec<&[Iq]> = channels.iter().map(|_| &zeros[..]).collect();
        let host = self.device.host();
        let until = host.now() + Wall::from_millis(100);
        let send_timeout = until.saturating_sub(host.now());
        match self.device.tx_send(stream, &buffers, None, true, true, send_timeout) {
            Ok(2) => {}
            Ok(n) => problem = Some(format!("flush send accepted {n} of 2 samples")),
            Err(error) => {
                self.device_failed(None, "tx_flush", &error);
                problem = Some(format!("flush send failed: {}", error.message));
                if error.lost {
                    self.reject_note(json!({ "call": "tx_flush", "stream": stream, "error": problem }));
                    return TxFlushResult { acknowledged: false, ack_tick: None, stopped: true };
                }
            }
        }

        let mut stopped = false;
        while acknowledged != expected {
            if self.is_lost() || lock(&self.streams).stop.is_some() {
                stopped = true;
                break;
            }
            let remaining = until.saturating_sub(host.now());
            if remaining.is_zero() {
                break;
            }
            match self.device.tx_async(stream, remaining) {
                Ok(Some(report)) => {
                    if host.now() >= until {
                        self.stash_tx_report(stream, report);
                        break;
                    }
                    if report.code == TxCode::BurstAck && expected.contains(&report.channel) && acknowledged.insert(report.channel) {
                        ack_tick = match (ack_tick, report.tick) {
                            (Some(old), Some(tick)) => Some(old.max(tick)),
                            (None, tick) => tick,
                            (old, None) => old,
                        };
                        self.record_tx_async(stream, report);
                    } else {
                        self.stash_tx_report(stream, report);
                        host.sleep(until.saturating_sub(host.now()).min(Wall::from_millis(1)));
                    }
                }
                Ok(None) => {}
                Err(error) if error.lost => {
                    self.device_lost(&error.message);
                    stopped = true;
                    break;
                }
                Err(error) => {
                    let first = lock(&self.rec).stats.get("tx_errors").is_none_or(|n| *n == 0);
                    self.stat("tx_errors", 1);
                    if first {
                        self.reject_note(json!({ "call": "tx_async", "error": error.message }));
                    }
                    host.sleep(until.saturating_sub(host.now()).min(Wall::from_millis(1)));
                }
            }
        }
        // A Stop may be published while the final blocking read is in flight. Do not let
        // that last ack turn an already-stopped open into a successful initialization.
        stopped |= !self.is_lost() && lock(&self.streams).stop.is_some();

        let missing: Vec<usize> = expected.difference(&acknowledged).copied().collect();
        if !missing.is_empty() && !stopped {
            let detail = format!("BurstAck missing on channel positions {missing:?} before the 100 ms deadline");
            problem = Some(problem.map_or(detail.clone(), |old| format!("{old}; {detail}")));
        }
        if let Some(error) = problem {
            self.reject_note(json!({ "call": "tx_flush", "stream": stream, "error": error }));
        }
        TxFlushResult { acknowledged: acknowledged == expected, ack_tick, stopped }
    }

    /// Opens TX and completes its flush while the same DSP and report gates remain held, so
    /// neither a timed tune nor another async reader can cross the open/flush boundary.
    pub fn open_tx_device(&self, k: usize, set: Channels) -> Result<TxFlushResult, DeviceError> {
        let channels = channel_list(set);
        let _dsp = self.lock_tx_dsp(&channels);
        let _reports = self.lock_tx_reports(k);
        self.open_tx_device_locked(k, set, &channels)
    }

    /// Replaces an uncertain TX streamer and runs its open-time flush without allowing a
    /// concurrent reader to take reports from the old generation.
    pub fn reopen_tx_streamer(&self, k: usize, set: Channels) -> Result<TxFlushResult, DeviceError> {
        let stream = stream_key(Dir::Tx, k);
        let channels = channel_list(set);
        let _dsp = self.lock_tx_dsp(&channels);
        let _reports = self.lock_tx_reports(k);
        self.close_device(Dir::Tx, k);
        lock(&self.streams).record_open(stream, None);
        let opened = self.open_tx_device_locked(k, set, &channels);
        let checked = if opened.is_ok() { self.check_opened(Dir::Tx, set) } else { Ok(()) };
        if opened.is_ok() && checked.is_err() && !self.is_lost() {
            self.close_device(Dir::Tx, k);
        }
        let opened_for_record: Result<(), DeviceError> = opened.as_ref().map(|_| ()).map_err(Clone::clone);
        lock(&self.streams).record_opened(stream, set, &opened_for_record, &checked);
        match opened {
            Ok(flush) => checked.map(|()| flush),
            Err(error) => Err(error),
        }
    }

    fn open_tx_device_locked(&self, k: usize, set: Channels, channels: &[usize]) -> Result<TxFlushResult, DeviceError> {
        self.open_device_locked(Dir::Tx, k, set)?;
        let flush = self.tx_flush_locked(k, channels);
        if self.is_lost() {
            return Err(DeviceError { lost: true, message: "UR-29: device lost during TX open flush".to_owned() });
        }
        if flush.stopped {
            return Err(DeviceError::failed("UR-25: Provider::stop during TX open flush"));
        }
        Ok(flush)
    }

    pub fn open_device(&self, dir: Dir, k: usize, set: Channels) -> Result<(), DeviceError> {
        let _reports = (dir == Dir::Tx).then(|| self.lock_tx_reports(k));
        self.open_device_locked(dir, k, set)
    }

    fn open_device_locked(&self, dir: Dir, k: usize, set: Channels) -> Result<(), DeviceError> {
        self.probe("open", None);
        if dir == Dir::Rx {
            self.assert_rx_chain_reusable(k, "open");
        }
        let device_gate = self.lock_device_gate();
        if self.is_lost() {
            return Err(DeviceError { lost: true, message: "UR-29: the device is lost before opening a streamer".to_owned() });
        }
        let list = channel_list(set);
        let opened = match dir {
            Dir::Rx => self.device.rx_open(k, &list),
            Dir::Tx => self.device.tx_open(k, &list),
        };
        if dir == Dir::Tx && opened.is_ok() {
            lock(&self.tx_report_stash[k]).clear();
        }
        drop(device_gate);
        self.probe("open_returned", None);
        opened
    }

    /// UR-13, UR-36: the rate comparison after an open on `set`, the direction's first one or
    /// a later one's stored outcome (`check_rates`).
    pub fn check_opened(&self, dir: Dir, set: Channels) -> Result<(), DeviceError> {
        self.check_rates(dir, channel_list(set)[0])
    }

    /// UR-13, UR-36: opens `k`'s streamer of `dir` on `set` and compares its rates. The owner
    /// and `arm` call this with `streams` released (UR-25); uhd-control's enable from no stream
    /// uses `open_device` and `check_opened` itself, and records the outcome in its own section.
    pub fn open_streamer(&self, dir: Dir, k: usize, set: Channels) -> Result<(), DeviceError> {
        let stream = stream_key(dir, k);
        let opened = if dir == Dir::Tx { self.open_tx_device(k, set).map(|_| ()) } else { self.open_device(dir, k, set) };
        let checked = if opened.is_ok() { self.check_opened(dir, set) } else { Ok(()) };
        if opened.is_ok() && checked.is_err() && !self.is_lost() {
            self.close_device(dir, k);
        }
        lock(&self.streams).record_opened(stream, set, &opened, &checked);
        opened.and(checked)
    }

    pub fn close_device(&self, dir: Dir, k: usize) {
        if dir == Dir::Rx {
            self.assert_rx_chain_reusable(k, "close");
        }
        let _device_gate = self.lock_device_gate();
        if self.is_lost() {
            return;
        }
        match dir {
            Dir::Rx => self.device.rx_close(k),
            Dir::Tx => self.device.tx_close(k),
        }
    }

    #[cfg(test)]
    fn assert_rx_chain_reusable(&self, k: usize, operation: &str) {
        let chain = self.rx_chain[k].lock();
        assert!(matches!(chain.device, RxDevice::Off | RxDevice::Uncertain | RxDevice::Lost), "L1: RX {operation} requires an idle device chain, got {:?}", chain.device);
        assert!(chain.debt.is_empty(), "L1: RX {operation} requires empty debt, got {:?}", chain.debt.iter().map(|entry| (entry.order, entry.channel)).collect::<Vec<_>>());
    }

    #[cfg(not(test))]
    fn assert_rx_chain_reusable(&self, _k: usize, _operation: &str) {}

    #[cfg(test)]
    pub fn assert_rx_chains_reusable(&self) {
        for k in 0..self.rx_chain.len() {
            self.assert_rx_chain_reusable(k, "close_streams");
        }
    }

    /// Releases an idle stream's streamer only when another line still has an unprocessed
    /// segment that needs one of its channels (D1, A2).
    pub fn release_unneeded(&self, dir: Dir, k: usize) -> bool {
        let stream = stream_key(dir, k);
        let claim = {
            let mut streams = lock(&self.streams);
            if self.is_lost() {
                return false;
            }
            let state = streams.stream(stream);
            if !streams.is_idle(stream) || streams.active(stream).is_some() {
                return false;
            }
            let DeviceSet::Open(set) = state.set else { return false };
            if !streams.needed_before(stream, set, i64::MAX) {
                return false;
            }
            let Some(claim) = streams.claim_release(stream) else { return false };
            claim
        };
        // UR-25, UR-29: the claim is taken; a loss found now gives it back before the close.
        self.probe("release_claimed", None);
        if self.is_lost() {
            lock(&self.streams).give_back(claim);
            return false;
        }
        self.close_device(dir, k);
        let mut streams = lock(&self.streams);
        streams.record_open(stream, None);
        streams.give_back(claim);
        true
    }

    /// Waits for another stream's recorded streamer to release a target segment's channels.
    /// The segment is read again after each wait; an absent, refused, stopped or changed
    /// segment is an abort, not a refusal (A4).
    fn await_release(&self, dir: Dir, k: usize, expected: &Planned) -> Option<Planned> {
        let stream = stream_key(dir, k);
        let mut waiting_since = None;
        loop {
            let streams = lock(&self.streams);
            if self.is_lost() || streams.stop.is_some() {
                return None;
            }
            let current = streams.planned(stream).into_iter().find(|planned| planned.segment == expected.segment)?;
            if current.refused {
                return None;
            }
            if let Some(since) = waiting_since {
                if dir == Dir::Tx && streams.line(stream).is_some_and(|line| line.items.iter().any(|item| item.seq > since && item.kind == Kind::Stop)) {
                    return None;
                }
            }
            if streams.held_elsewhere(stream, current.set()).is_none() && !streams.needed_before(stream, current.set(), current.segment.origin) && !streams.move_source_busy(stream, current.segment.by)
            {
                return Some(current);
            }
            let first_wait = waiting_since.is_none();
            waiting_since.get_or_insert(streams.seq);
            drop(streams);
            if first_wait {
                self.probe("wait_release", None);
            }
            self.device.host().sleep(POLL);
        }
    }

    /// The owner's device calls for a segment it begins (UR-13, UR-36): the record decides what
    /// is opened and configured. Each device call is made with `streams` released, and its
    /// outcome is recorded after it; a successful open moves the record's generation on, which
    /// is how a thread holding correlations against the replaced streamer learns of it (UR-28).
    /// On error, the caller books the refusal before giving the claim back (UR-25).
    pub fn owner_configure(&self, dir: Dir, k: usize, next: &Planned) -> Result<bool, (DeviceError, Claim)> {
        let stream = stream_key(dir, k);
        let (mut current, mut step, release_first, claim) = {
            let mut streams = lock(&self.streams);
            if self.is_lost() {
                return Ok(false);
            }
            if streams.holder(stream) != Holder::Free {
                return Ok(false);
            }
            let Some(current) = streams.planned(stream).into_iter().find(|planned| planned.segment == next.segment) else { return Ok(false) };
            if current.refused {
                return Ok(false);
            }
            let step = streams.owner_step(stream, current.set(), current.segment.by);
            if step == Owner::Nothing {
                return Ok(true);
            }
            let release_first = step == Owner::Open
                && (streams.held_elsewhere(stream, current.set()).is_some()
                    || streams.needed_before(stream, current.set(), current.segment.origin)
                    || streams.move_source_busy(stream, current.segment.by));
            let Some(claim) = streams.claim_owner(stream, current.segment.by) else { return Ok(false) };
            (current, step, release_first, claim)
        };
        if release_first {
            if self.is_lost() {
                lock(&self.streams).give_back(claim);
                return Ok(false);
            }
            let open = lock(&self.streams).stream(stream).set;
            if matches!(open, DeviceSet::Open(_)) {
                self.close_device(dir, k);
                lock(&self.streams).record_open(stream, None);
            }
            let Some(latest) = self.await_release(dir, k, &current) else {
                lock(&self.streams).give_back(claim);
                return Ok(false);
            };
            current = latest;
            step = Owner::Open;
        }
        let opened = if step == Owner::Open { self.open_streamer(dir, k, current.set()) } else { Ok(()) };
        if let Err(error) = opened {
            lock(&self.streams).record_configured(stream, None);
            return Err((error, claim));
        }
        match current.configure(self, dir) {
            Ok(_) => {
                // UR-36: the record's mark is published only once the configuration returned.
                self.probe("configured", None);
                let mut streams = lock(&self.streams);
                streams.record_configured(stream, current.segment.by);
                streams.give_back(claim);
                Ok(true)
            }
            Err(error) => {
                lock(&self.streams).record_configured(stream, None);
                Err((error, claim))
            }
        }
    }

    /// Declares and registers stream 0's clock with origin `origin` (UR-12, RM-25), for the
    /// unit rigs.
    #[cfg(test)]
    pub fn register(&self, dir: Dir, n: i64, origin: i64) -> Result<Clock, String> {
        self.register_in(dir, 0, n, origin)
    }

    /// Declares and registers stream `k`'s clock with origin `origin` (UR-12, RM-25).
    pub fn register_in(&self, dir: Dir, k: usize, n: i64, origin: i64) -> Result<Clock, String> {
        let stream = self.description.stream_node(&self.id, dir, k);
        let ratio = Rational::new(n as u64, 1).map_err(|e| e.to_string())?;
        let handle = self.clocks.declare_sample_clock(stream, self.root, ratio).map_err(|e| e.to_string())?;
        let domain = self.clocks.register_sample_clock(&handle, origin).map_err(|e| e.to_string())?;
        Ok(Clock { domain, origin, n })
    }
}

#[cfg(test)]
pub(super) fn assert_tx_open_flush_barrier(core: Arc<Core>, spy: Arc<super::test_support::LockSpy>, open: impl FnOnce() + Send + 'static) {
    let gate = spy.stop_after("tx_send");
    let host = core.device.host();
    let opener = host.spawn("tx-open-route", open).unwrap();
    gate.wait();
    assert!(core.tx_dsp_gates[0].is_held(), "the opener must hold the timed-DSP gate through its flush acknowledgement");

    let core_ = core.clone();
    let timed = host
        .spawn("timed-tx-dsp", move || {
            let _dsp = core_.lock_tx_dsp(&[0]);
            core_.device.apply(Dir::Tx, 0, &Settings { freq: Some(1.1e9), ..Settings::default() }, Some(10_000)).unwrap();
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Wall::from_secs(10);
    while core.tx_dsp_gates[0].waiters() == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(core.tx_dsp_gates[0].waiters() > 0, "timed DSP did not reach the flush's channel gate");
    assert_eq!(spy.seen("apply"), 0, "timed TX DSP crossed the flush-ack barrier");

    gate.release();
    opener.join().unwrap();
    timed.join().unwrap();
    let order = spy.order();
    let flush = order.iter().position(|call| *call == "tx_send").unwrap();
    let ack = order.iter().enumerate().skip(flush + 1).find_map(|(i, call)| (*call == "tx_async").then_some(i)).unwrap();
    let apply = order.iter().position(|call| *call == "apply").unwrap();
    assert!(flush < ack && ack < apply, "flush -> ack read -> timed DSP: {order:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezsdr_kernel::time::Rational;

    fn rx_segment() -> SegmentKey {
        SegmentKey { by: Some(7), origin: 100 }
    }

    fn rx_entry(seq: u64) -> RxEntry {
        RxEntry { order: (100, seq), channel: 0, value: 1.001e9, accepted_at: Some(101) }
    }

    #[test]
    fn rx_chain_adopt_start_transitions() {
        let mut chain = RxChain::default();
        let segment = rx_segment();
        chain.adopt_start(segment, 90);
        assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
        assert_eq!(chain.segment(), Some(segment));
        assert!(chain.debt().is_empty());
        assert!(chain.left().is_empty());
        assert!(chain.latest().is_empty());
        assert_eq!(chain.cut(), None);
        assert!(!chain.end_now());
        assert_eq!(chain.terminate(), None);
        assert!(matches!(chain.restore(), Restore::NotAsked));
    }

    #[test]
    fn rx_chain_begin_transitions() {
        for prior in [RxDevice::Off, RxDevice::Uncertain] {
            for outcome in [0, 1, 2] {
                let mut chain = RxChain { device: prior, ..RxChain::default() };
                let result = match outcome {
                    0 => Ok(()),
                    1 => Err(DeviceError::failed("scripted refusal")),
                    _ => Err(DeviceError { lost: true, message: "scripted loss".to_owned() }),
                };
                let actual = chain.begin_result(rx_segment(), 90, result);
                match outcome {
                    0 => {
                        assert!(actual.is_ok());
                        assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
                        assert_eq!(chain.segment(), Some(rx_segment()));
                    }
                    1 => {
                        assert!(actual.is_err());
                        assert_eq!(chain.device(), prior);
                        assert_eq!(chain.segment(), None);
                    }
                    _ => {
                        assert!(actual.unwrap_err().lost);
                        assert_eq!(chain.device(), RxDevice::Lost);
                        assert_eq!(chain.segment(), None);
                    }
                }
                assert!(chain.debt().is_empty());
            }
        }
    }

    #[test]
    fn rx_chain_restart_transitions() {
        for prior in [RxDevice::Armed { at: 90 }, RxDevice::Streaming] {
            for outcome in [0, 1, 2] {
                let mut chain = RxChain { device: prior, segment: Some(rx_segment()), ..RxChain::default() };
                let actual = chain.restart_result(
                    120,
                    match outcome {
                        0 => Ok(()),
                        1 => Err(DeviceError::failed("scripted refusal")),
                        _ => Err(DeviceError { lost: true, message: "scripted loss".to_owned() }),
                    },
                );
                match outcome {
                    0 => {
                        assert!(actual.is_ok());
                        assert_eq!(chain.device(), RxDevice::Armed { at: 120 });
                    }
                    1 => {
                        assert!(actual.is_err());
                        assert_eq!(chain.device(), prior);
                    }
                    _ => {
                        assert!(actual.unwrap_err().lost);
                        assert_eq!(chain.device(), RxDevice::Lost);
                    }
                }
                assert_eq!(chain.segment(), Some(rx_segment()));
            }
        }
    }

    #[test]
    fn rx_chain_first_sample_transition() {
        let mut chain = RxChain::default();
        chain.adopt_start(rx_segment(), 90);
        chain.first_sample(SegmentKey { by: Some(8), origin: 100 });
        assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
        chain.first_sample(rx_segment());
        assert_eq!(chain.device(), RxDevice::Streaming);
        assert_eq!(chain.segment(), Some(rx_segment()));
    }

    #[test]
    fn rx_chain_hand_transitions() {
        for outcome in [0, 1, 2] {
            let mut chain = RxChain::default();
            chain.adopt_start(rx_segment(), 90);
            let result = chain.hand(
                rx_entry(1),
                || match outcome {
                    0 => Ok(()),
                    1 => Err(DeviceError::failed("scripted refusal")),
                    _ => Err(DeviceError { lost: true, message: "scripted loss".to_owned() }),
                },
                || 105,
            );
            match outcome {
                0 => {
                    assert!(result.is_ok());
                    assert_eq!(chain.debt().len(), 1);
                    assert_eq!(chain.debt()[0].accepted_at, Some(105));
                    assert_eq!(chain.latest().get(&0).unwrap().accepted_at, Some(105));
                    assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
                }
                1 => {
                    assert!(!result.unwrap_err().lost);
                    assert!(chain.debt().is_empty());
                    assert!(chain.latest().is_empty());
                    assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
                }
                _ => {
                    assert!(result.unwrap_err().lost);
                    assert!(chain.debt().is_empty());
                    assert!(chain.latest().is_empty());
                    assert_eq!(chain.device(), RxDevice::Lost);
                }
            }
        }
    }

    #[test]
    fn rx_chain_stop_transitions() {
        for outcome in [0, 1, 2] {
            let mut chain = RxChain::default();
            chain.adopt_start(rx_segment(), 90);
            let result = chain.stop_result(match outcome {
                0 => Ok(()),
                1 => Err(DeviceError::failed("scripted refusal")),
                _ => Err(DeviceError { lost: true, message: "scripted loss".to_owned() }),
            });
            let expected = match outcome {
                0 => RxDevice::Off,
                1 => RxDevice::Uncertain,
                _ => RxDevice::Lost,
            };
            assert_eq!(chain.device(), expected);
            assert_eq!(chain.segment(), Some(rx_segment()));
            assert_eq!(result.is_ok(), outcome == 0);
        }
    }

    #[test]
    fn rx_chain_flush_transitions() {
        let mut chain = RxChain::default();
        assert!(chain.flush_start_result(Ok(())).is_ok());
        assert_eq!(chain.device(), RxDevice::Flushing);
        assert!(chain.flush_stop_result(Ok(())).is_ok());
        assert_eq!(chain.device(), RxDevice::Off);

        let mut chain = RxChain::default();
        assert!(chain.flush_start_result(Err(DeviceError::failed("start refusal"))).is_err());
        assert_eq!(chain.device(), RxDevice::Off);
        let mut chain = RxChain::default();
        assert!(chain.flush_start_result(Err(DeviceError { lost: true, message: "start loss".to_owned() })).is_err());
        assert_eq!(chain.device(), RxDevice::Lost);
        for (error, expected) in [(DeviceError::failed("stop refusal"), RxDevice::Uncertain), (DeviceError { lost: true, message: "stop loss".to_owned() }, RxDevice::Lost)] {
            let mut chain = RxChain::default();
            chain.flush_start_result(Ok(())).unwrap();
            assert!(chain.flush_stop_result(Err(error)).is_err());
            assert_eq!(chain.device(), expected);
        }
    }

    #[test]
    fn rx_chain_finish_transitions() {
        for device in [RxDevice::Off, RxDevice::Uncertain, RxDevice::Lost] {
            let entry = rx_entry(1);
            let mut chain = RxChain {
                device,
                intent: RxIntent { cut: Some(80), end_now: true, terminate: Some(StopMode::Abort) },
                segment: Some(rx_segment()),
                debt: vec![entry.clone()],
                latest: BTreeMap::from([(0, entry.clone())]),
                restore: Restore::Planned(BTreeMap::from([(0, 1e9)])),
                left: Vec::new(),
            };
            assert_eq!(chain.finish().as_slice(), std::slice::from_ref(&entry));
            assert_eq!(chain.device(), device);
            assert_eq!(chain.segment(), None);
            assert!(chain.debt().is_empty());
            assert_eq!(chain.left(), std::slice::from_ref(&entry));
            assert_eq!(chain.latest().get(&0).unwrap().value, entry.value);
            assert_eq!(chain.cut(), None);
            assert!(!chain.end_now());
            assert_eq!(chain.terminate(), Some(StopMode::Abort));
            assert!(matches!(chain.restore(), Restore::Planned(values) if values.get(&0) == Some(&1e9)));
        }
    }

    #[test]
    fn rx_chain_lose_is_terminal() {
        for prior in [RxDevice::Off, RxDevice::Armed { at: 90 }, RxDevice::Streaming, RxDevice::Flushing, RxDevice::Uncertain, RxDevice::Lost] {
            let entry = rx_entry(1);
            let mut chain = RxChain { device: prior, segment: Some(rx_segment()), debt: vec![entry.clone()], latest: BTreeMap::from([(0, entry.clone())]), ..RxChain::default() };
            assert_eq!(chain.lose().as_slice(), std::slice::from_ref(&entry));
            assert_eq!(chain.device(), RxDevice::Lost);
            assert_eq!(chain.debt().len(), 0);
            assert_eq!(chain.left(), std::slice::from_ref(&entry));
            assert!(chain.lose().is_empty(), "loss reports each entry once");
            assert_eq!(chain.latest().get(&0).unwrap().value, entry.value);
            assert_eq!(chain.finish(), []);
            assert_eq!(chain.device(), RxDevice::Lost);
            assert_eq!(chain.segment(), None);
        }
    }

    #[test]
    fn rx_chain_abandon_unowned_records_cleanup_state() {
        let mut chain = RxChain::default();
        chain.adopt_start(rx_segment(), 90);
        chain.request_cut(150);
        chain.abandon_unowned();
        assert_eq!(chain.device(), RxDevice::Uncertain);
        assert_eq!(chain.segment(), None);
        assert_eq!(chain.cut(), Some(150));
        assert!(chain.end_now());
        assert_eq!(chain.terminate(), Some(StopMode::Abort));
        assert!(chain.debt().is_empty());
        assert!(chain.latest().is_empty());
    }

    #[test]
    fn rx_chain_abandon_unowned_preserves_loss() {
        let entry = rx_entry(1);
        let mut chain = RxChain { debt: vec![entry.clone()], ..RxChain::default() };
        chain.lose();
        chain.abandon_unowned();
        assert_eq!(chain.device(), RxDevice::Lost);
        assert_eq!(chain.segment(), None);
        assert_eq!(chain.left(), &[entry]);
        assert!(chain.end_now());
        assert_eq!(chain.terminate(), Some(StopMode::Abort));
        assert!(chain.finish().is_empty(), "loss debt is reported only once");
    }

    #[test]
    fn ur_22_t3_flush_ack_is_a_timed_dsp_barrier() {
        use crate::device::{Dir, FakeConfig, Settings};
        use crate::provider::test_support::LockSpy;
        use std::sync::Arc;

        let spy = Arc::new(LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let gate = spy.stop_before("tx_async");
        let opener = core
            .device
            .host()
            .spawn("tx-flush-open", {
                let core = core.clone();
                move || {
                    core.open_tx_device(0, Channels::first(1)).unwrap();
                }
            })
            .unwrap();
        gate.wait();
        assert!(core.tx_report_gates[0].is_held(), "the opener must own async reports through its flush acknowledgement");

        let (done, finished) = std::sync::mpsc::channel();
        let timed = core
            .device
            .host()
            .spawn("timed-tx-dsp", {
                let core = core.clone();
                move || {
                    let _dsp = core.lock_tx_dsp(&[0]);
                    core.device.apply(Dir::Tx, 0, &Settings { freq: Some(1.1e9), ..Settings::default() }, Some(10_000)).unwrap();
                    let _ = done.send(());
                }
            })
            .unwrap();
        let shutdown_stream = stream_key(Dir::Tx, 0);
        let (shutdown_started, shutdown_start) = std::sync::mpsc::channel();
        lock(&core.streams).begin_shutdown_read(shutdown_stream);
        let (shutdown_done, shutdown_report) = std::sync::mpsc::channel();
        let shutdown = core
            .device
            .host()
            .spawn("shutdown-tx-report", {
                let core = core.clone();
                move || {
                    let _ = shutdown_started.send(());
                    let _reports = core.lock_tx_reports(0);
                    let report = core.device.tx_async(0, Wall::ZERO).unwrap();
                    lock(&core.streams).end_read(shutdown_stream);
                    let _ = shutdown_done.send(report);
                }
            })
            .unwrap();
        shutdown_start.recv_timeout(Wall::from_secs(2)).unwrap();
        let deadline = std::time::Instant::now() + Wall::from_secs(2);
        while core.tx_report_gates[0].waiters() == 0 && spy.seen("tx_async") == 1 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let shutdown_waited_for_report = core.tx_report_gates[0].waiters() > 0;
        let reads_before_flush_ack = spy.seen("tx_async");
        assert!(finished.try_recv().is_err(), "timed TX DSP crossed the open-time flush wait");
        assert_eq!(spy.seen("apply"), 0, "no timed write reached the device before the flush ack");

        gate.release();
        opener.join().unwrap();
        timed.join().unwrap();
        shutdown.join().unwrap();
        assert!(shutdown_waited_for_report, "shutdown's reader did not wait behind the opener's report ownership");
        assert_eq!(reads_before_flush_ack, 1, "shutdown read an async report before the opener consumed the flush ack");
        assert!(shutdown_report.recv().unwrap().is_none());
        assert!(finished.try_recv().is_ok());
        let order = spy.order();
        let flush = order.iter().position(|call| *call == "tx_send").unwrap();
        let ack = order.iter().position(|call| *call == "tx_async").unwrap();
        let apply = order.iter().position(|call| *call == "apply").unwrap();
        assert!(flush < ack && ack < apply, "flush -> ack read -> timed DSP: {order:?}");
        assert_eq!(spy.log().iter().filter(|call| call.starts_with("tx_send")).count(), 1);
        assert_eq!(spy.seen("tx_async"), 2, "shutdown owns the next read after the flush");
    }

    #[test]
    fn ur_22_t3_missing_ack_is_bounded_and_read_faults_keep_progressing() {
        use crate::device::{FakeConfig, FakeFault, TxCode};

        for (faults, expect_read_error, expect_other) in [
            (vec![FakeFault::DropBurstAck], false, false),
            (vec![FakeFault::TxAsyncFails(Wall::ZERO, false)], true, false),
            (vec![FakeFault::DropBurstAck, FakeFault::TxReport(Wall::ZERO, TxCode::Other(77))], false, true),
        ] {
            let config = FakeConfig { clock: crate::ClockMode::Manual, faults, ..FakeConfig::default() };
            let (core, device, _time, _events) = super::super::test_support::rig_with(config, Vec::new());
            device.tx_open(0, &[0]).unwrap();
            let before = device.host().now();
            let result = core.tx_flush(0, &[0]);
            let elapsed = device.host().now().saturating_sub(before);
            let rejected: Vec<_> = lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").cloned().collect();
            assert!(!result.acknowledged, "a missing flush ack was treated as success");
            assert!(elapsed >= Wall::from_millis(100), "flush did not use its total device-clock deadline: {elapsed:?}");
            assert_eq!(rejected.len(), 1, "{rejected:?}");
            assert!(!core.is_lost(), "a missing acknowledgement is not device loss");
            assert_eq!(lock(&core.rec).stats.get("tx_errors").copied().unwrap_or_default() > 0, expect_read_error);
            assert_eq!(core.take_stashed_tx_report(0).is_some_and(|report| report.code == TxCode::Other(77)), expect_other);
        }
    }

    #[test]
    fn ur_22_t3_flush_send_faults_record_a_missing_ack() {
        use crate::device::{FakeConfig, FakeFault};

        for fault in [FakeFault::ShortSend(0), FakeFault::StalledSend(0), FakeFault::FailSend(0)] {
            let config = FakeConfig { clock: crate::ClockMode::Manual, faults: vec![fault.clone()], ..FakeConfig::default() };
            let (core, device, _time, _events) = super::super::test_support::rig_with(config, Vec::new());
            device.tx_open(0, &[0]).unwrap();
            let result = core.tx_flush(0, &[0]);
            assert!(!result.acknowledged, "{fault:?}: incomplete flush was treated as success");
            assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1, "{fault:?}");
            assert!(device.calls().iter().any(|call| call.starts_with("tx_send")), "{fault:?}: {:?}", device.calls());
        }
    }

    #[test]
    fn ur_22_t3_flush_requires_each_channel_ack_and_device_loss_stays_distinct() {
        use crate::device::{FakeConfig, FakeFault};

        let (core, device, _time, _events) = super::super::test_support::rig_with(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }, Vec::new());
        device.tx_open(0, &[0, 1]).unwrap();
        let result = core.tx_flush(0, &[0, 1]);
        assert!(result.acknowledged, "both channel-position acks are required: {:?}", lock(&core.rec).device_async);
        assert_eq!(lock(&core.rec).device_async.iter().filter(|row| row["code"] == "BurstAck" && row["tx_flush"] == true).map(|row| row["channel"].as_u64()).collect::<Vec<_>>(), [Some(0), Some(1)]);

        let config = FakeConfig { clock: crate::ClockMode::Manual, faults: vec![FakeFault::TxAsyncFails(Wall::ZERO, true)], ..FakeConfig::default() };
        let (lost_core, lost_device, _time, events) = super::super::test_support::rig_with(config, Vec::new());
        lost_device.tx_open(0, &[0]).unwrap();
        let result = lost_core.tx_flush(0, &[0]);
        assert!(result.stopped && lost_core.is_lost());
        assert_eq!(events.drain(lost_core.at(lost_core.now())).iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1);
        assert!(lock(&lost_core.rec).rejected.iter().all(|row| row["call"] != "tx_flush"), "loss is not a missing ack: {:?}", lock(&lost_core.rec).rejected);
    }

    #[test]
    fn ur_22_t3_ack_read_returned_after_deadline_is_not_a_flush_success() {
        use crate::device::FakeConfig;
        use crate::provider::test_support::LockSpy;
        use std::sync::Arc;

        let spy = Arc::new(LockSpy::new(FakeConfig { clock: crate::ClockMode::Real, ..FakeConfig::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let gate = spy.stop_after("tx_async");
        let (done, finished) = std::sync::mpsc::channel();
        let opener = core
            .device
            .host()
            .spawn("late-tx-flush-ack", {
                let core = core.clone();
                move || {
                    let _ = done.send(core.open_tx_device(0, Channels::first(1)));
                }
            })
            .unwrap();
        gate.wait();
        std::thread::sleep(Wall::from_millis(120));
        gate.release();
        opener.join().unwrap();

        let flush = finished.recv().unwrap().unwrap();
        assert!(!flush.acknowledged, "an ack read returned after the total deadline completed T3");
        assert!(core.take_stashed_tx_report(0).is_some_and(|report| report.code == TxCode::BurstAck), "the late ack remains available to the report owner");
        assert_eq!(lock(&core.rec).rejected.iter().filter(|row| row["call"] == "tx_flush").count(), 1);
        assert!(lock(&core.rec).device_async.iter().all(|row| row["tx_flush"] != true), "a late ack is not recorded as a timely flush ack");
    }

    #[test]
    fn ur_22_t3_stop_interrupts_an_open_flush_wait() {
        use crate::device::FakeConfig;
        use crate::provider::test_support::LockSpy;
        use std::sync::Arc;

        let spy = Arc::new(LockSpy::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let gate = spy.stop_before("tx_async");
        let (done, result_rx) = std::sync::mpsc::channel();
        let opener = core
            .device
            .host()
            .spawn("tx-flush-stop", {
                let core = core.clone();
                move || {
                    let _ = done.send(core.open_tx_device(0, Channels::first(1)));
                }
            })
            .unwrap();
        gate.wait();
        lock(&core.streams).stop = Some((core.now(), StopMode::Orderly));
        gate.release();
        opener.join().unwrap();
        let error = match result_rx.recv().unwrap() {
            Err(error) => error,
            Ok(_) => panic!("Provider::stop let streamer open complete"),
        };
        assert!(!error.lost && error.message.contains("Provider::stop"), "{error:?}");
        assert!(!core.is_lost());
        assert!(lock(&core.rec).rejected.iter().all(|row| row["call"] != "tx_flush"), "Stop is distinct from a missing ack");
    }

    #[test]
    fn ur_22_t3_flush_clears_the_header_but_does_not_drain_timed_dsp_fifo() {
        use crate::device::{Dir, FakeConfig, Settings};
        use ezsdr_radio::timeline::Channels;

        let config = FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() };
        let (core, device, _time, _events) = super::super::test_support::rig_with(config, Vec::new());
        device.apply(Dir::Tx, 0, &Settings { rate: Some(1e6), freq: Some(1e9), ..Settings::default() }, None).unwrap();
        let old_start = 4_000_000;
        device.apply(Dir::Tx, 0, &Settings { freq: Some(1.00005e9), ..Settings::default() }, Some(old_start + 600)).unwrap();
        let old_burst = [[0.25, 0.0]; 2];
        device.tx_open(0, &[0]).unwrap();
        device.tx_send(0, &[&old_burst], Some(old_start), true, true, Wall::ZERO).unwrap();
        device.tx_close(0);
        assert_eq!(device.dsp_pending(Dir::Tx, 0), 1, "the command lies beyond the old burst");

        let flush = core.open_tx_device(0, Channels::first(1)).unwrap();
        assert!(flush.acknowledged, "T3 consumed its flush BurstAck");
        assert_eq!(device.dsp_pending(Dir::Tx, 0), 1, "an untimed flush is not a DUC FIFO drain");
        assert!(device.dsp_log(Dir::Tx, 0).iter().all(|(_, _, timed)| !timed), "the queued timed value is not taken by the flush");

        let next_start = 6_000_000;
        device.tx_send(0, &[&old_burst], Some(next_start), true, true, Wall::ZERO).unwrap();
        device.set_time(next_start);
        assert_eq!(device.dsp_log(Dir::Tx, 0).last(), Some(&(next_start, 1.00005e9, true)), "the old timed value later overrides the new configuration on the next timed burst");
    }

    fn cfg(channels: u32) -> Config {
        Config { channels: Channels::first(channels), ratio: Rational::new(200, 1).unwrap() }
    }

    fn line(direction: Direction) -> Line {
        Line::new(timeline::Stream { direction, origin: 0, config: cfg(1), lead: 0, call: 2_000, release_window: 0, allowance: 0 })
    }

    fn two_by_two_with_device() -> (Arc<Core>, Arc<crate::device::FakeDevice>) {
        let (core, device, _, _) = super::super::test_support::rig();
        {
            let mut streams = lock(&core.streams);
            *streams = Streams::with_streams(2, 2);
            for dir in [Direction::Rx, Direction::Tx] {
                for k in 0..2 {
                    streams.lines.insert((dir, k), line(dir));
                }
            }
        }
        (core, device)
    }

    fn two_by_two() -> Arc<Core> {
        two_by_two_with_device().0
    }

    fn waiting_move() -> (Arc<Core>, Arc<crate::device::FakeDevice>, Planned, StreamKey, StreamKey, u64) {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Rx, 1), stream_key(Dir::Rx, 0));
        let ratio = Rational::new(200, 1).unwrap();
        let rx_line = |channels| {
            Line::new(timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels, ratio },
                lead: 0,
                call: 0,
                release_window: core.ticks(core.description.timing.release_window_ns),
                allowance: 0,
            })
        };
        device.rx_open(0, &[0]).unwrap();
        device.rx_open(1, &[1]).unwrap();
        let mut streams = lock(&core.streams);
        streams.lines.insert(from, rx_line(Channels::first(2).without(0)));
        streams.lines.insert(to, rx_line(Channels::first(1)));
        streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
        streams.stream_mut(to).set = DeviceSet::Open(Channels::first(1));
        let seq = 1;
        streams.lines.book_move(from, to, 1, Some(10_000), 0, seq, 0).unwrap();
        streams.changed(&core, from);
        streams.changed(&core, to);
        let next = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(seq)).expect("the destination move segment");
        drop(streams);
        (core, device, next, from, to, seq)
    }

    fn active_rx_chain(core: &Core, k: usize) {
        core.rx_chain[k].lock().adopt_start(SegmentKey { by: Some(1), origin: 0 }, 0);
    }

    fn indebted_off_rx_chain(core: &Core, k: usize) {
        let mut chain = core.rx_chain[k].lock();
        chain.adopt_start(SegmentKey { by: Some(1), origin: 0 }, 0);
        chain.hand(RxEntry { order: (1, 1), channel: 0, value: 1e9, accepted_at: Some(0) }, || Ok(()), || 0).unwrap();
        chain.stop_result(Ok(())).unwrap();
    }

    #[test]
    fn rx_chain_reuse_guard_rejects_active_open_and_debt_close() {
        let (core, device) = two_by_two_with_device();
        active_rx_chain(&core, 0);
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = core.open_device(Dir::Rx, 0, Channels::first(1));
        }));
        assert!(opened.is_err(), "an Armed chain cannot be reopened");
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_open")), "the guard runs before the device call: {:?}", device.calls());

        indebted_off_rx_chain(&core, 1);
        let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.close_device(Dir::Rx, 1)));
        assert!(closed.is_err(), "an Off chain with outstanding debt cannot be closed");
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_close")), "the guard runs before the device call: {:?}", device.calls());
    }

    #[test]
    fn rx_chain_reconfigure_close_requires_idle_empty_debt() {
        let (core, device, next, from, to, seq) = waiting_move();
        let source = lock(&core.streams).claim_owner(from, Some(seq)).expect("the move source is busy");
        active_rx_chain(&core, to.1 as usize);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = core.owner_configure(Dir::Rx, to.1 as usize, &next);
        }));
        assert!(result.is_err(), "the owner's release-before-open close is guarded");
        assert!(!device.calls().iter().any(|call| call == "rx_close 0"), "the reconfigure did not close an active chain: {:?}", device.calls());
        lock(&core.streams).give_back(source);
    }

    #[test]
    fn rx_chain_release_unneeded_close_requires_idle_empty_debt() {
        let (core, device, _next, from, _to, _seq) = waiting_move();
        lock(&core.streams).search(from, false);
        indebted_off_rx_chain(&core, from.1 as usize);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.release_unneeded(Dir::Rx, from.1 as usize)));
        assert!(result.is_err(), "release_unneeded cannot close a chain with debt");
        assert!(!device.calls().iter().any(|call| call == "rx_close 1"), "the debt-bearing chain was not closed: {:?}", device.calls());
    }

    #[test]
    fn rx_chain_open_failure_close_requires_idle_empty_debt() {
        let (core, device) = two_by_two_with_device();
        let rates_core = core.clone();
        super::super::test_support::probe(&core, "rates", move |_| active_rx_chain(&rates_core, 0));
        lock(&core.rates)[Dir::Rx as usize] = Some(Err("scripted rate-check refusal".to_owned()));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = core.open_streamer(Dir::Rx, 0, Channels::first(1));
        }));
        assert!(result.is_err(), "open_streamer cannot clean up by closing an active chain");
        let calls = device.calls();
        assert!(calls.iter().any(|call| call == "rx_open 0 [0]"), "the open preceded its rate check: {calls:?}");
        assert!(!calls.iter().any(|call| call == "rx_close 0"), "the invalid chain prevented cleanup close: {calls:?}");
    }

    #[test]
    fn ur_34_a_device_loss_waits_for_an_admitted_streamer_open_and_close() {
        for (call, opening) in [("rx_open", true), ("rx_close", false)] {
            let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
            let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            let gate = spy.stop_before(call);
            let call_core = core.clone();
            let call_thread = core
                .device
                .host()
                .spawn("streamer-lifecycle", move || {
                    if opening {
                        call_core.open_device(Dir::Rx, 0, Channels::first(1)).unwrap();
                    } else {
                        call_core.close_device(Dir::Rx, 0);
                    }
                })
                .unwrap();
            gate.wait();

            let (waiting, reached) = std::sync::mpsc::channel();
            let (continue_loss, continue_rx) = std::sync::mpsc::channel();
            let continue_rx = Mutex::new(continue_rx);
            super::super::test_support::probe(&core, "device_gate_waiting", move |_| {
                let _ = waiting.send(());
                let _ = continue_rx.lock().unwrap().recv();
            });
            let loss_core = core.clone();
            let loss_thread = core.device.host().spawn("device-loss", move || loss_core.device_lost("test loss")).unwrap();
            assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "loss waits for the admitted {call}");
            let _ = continue_loss.send(());
            gate.release();
            call_thread.join().unwrap();
            loss_thread.join().unwrap();

            let log = spy.log();
            let call = log.iter().position(|entry| entry == if opening { "rx_open 0 [0]" } else { "rx_close 0" }).unwrap_or_else(|| panic!("{call}: {log:?}"));
            let marked_lost = log.iter().position(|entry| entry == "mark_lost").unwrap_or_else(|| panic!("no mark_lost: {log:?}"));
            assert!(call < marked_lost, "the admitted call completed before loss publication: {log:?}");
            let before = log.len();
            assert!(core.open_device(Dir::Rx, 0, Channels::first(1)).unwrap_err().lost);
            core.close_device(Dir::Rx, 0);
            assert_eq!(spy.log().len(), before, "no open or close reaches a lost device: {:?}", spy.log());
        }
    }

    #[test]
    fn ur_34_a_rate_read_waiting_for_device_gate_does_not_call_after_loss() {
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let (entered, reached) = std::sync::mpsc::channel();
        let (resume, waiting) = std::sync::mpsc::channel();
        let waiting = Mutex::new(waiting);
        super::super::test_support::probe(&core, "rates_before_gate", move |_| {
            let _ = entered.send(());
            let _ = waiting.lock().unwrap().recv();
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let reader = core
            .device
            .host()
            .spawn("rate-read", move || {
                let _ = done.send(worker.check_rates(Dir::Rx, 0));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the rate read passed its first loss check before waiting for the gate");
        core.device_lost("test loss");
        let _ = resume.send(());
        reader.join().unwrap();
        assert!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap().unwrap_err().lost);
        assert!(!spy.log().iter().any(|call| call.starts_with("rates ")), "the rate read called a lost device: {:?}", spy.log());
    }

    #[test]
    fn ur_34_a_waited_open_waits_for_a_finite_unprocessed_move_segment() {
        let (core, _device) = two_by_two_with_device();
        let (a, b, c) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1), stream_key(Dir::Rx, 2));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(timeline::Stream { direction: Direction::Rx, origin: 0, config: Config { channels, ratio }, lead: 0, call: 0, release_window: 0, allowance: 0 });
        let (middle, destination) = {
            let mut streams = lock(&core.streams);
            *streams = Streams::with_streams(3, 1);
            streams.lines.insert(a, line(Channels::first(1)));
            streams.lines.insert(b, line(Channels::first(0).with(1)));
            streams.lines.insert(c, line(Channels::first(0)));
            streams.lines.book_move(a, b, 0, Some(1_000), 0, 1, 0).unwrap();
            streams.changed(&core, a);
            streams.changed(&core, b);
            streams.lines.book_move(b, c, 0, Some(3_000), 0, 2, 0).unwrap();
            streams.changed(&core, b);
            streams.changed(&core, c);
            let a_initial = streams.planned(a).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(a, (a_initial.segment.origin, a_initial.segment.config, a_initial.segment.by));
            let middle = streams.planned(b).into_iter().find(|planned| planned.segment.by == Some(1)).unwrap();
            assert!(middle.segment.cut.is_some_and(|cut| middle.segment.instant(cut).is_ok_and(|end| end > middle.segment.origin)), "B has a finite segment with samples");
            let destination = streams.planned(c).into_iter().find(|planned| planned.segment.by == Some(2)).unwrap();
            streams.stream_mut(b).set = DeviceSet::Unknown;
            (middle, destination)
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-finite-chain", move || {
                let _ = done.send(worker.await_release(Dir::Rx, c.1 as usize, &destination));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "C waits for B's finite segment despite B's Unknown device set");
        assert!(result_rx.try_recv().is_err(), "the destination did not pass B's unfinished segment");
        {
            let mut streams = lock(&core.streams);
            streams.complete(b, (middle.segment.origin, middle.segment.config, middle.segment.by));
        }
        owner.join().unwrap();
        assert!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap().is_some(), "the owner continued after B completed");
    }

    #[test]
    fn ur_34_a_waited_open_waits_for_same_move_source_configuration() {
        let (core, device, next, from, to, seq) = waiting_move();
        let claim = {
            let mut streams = lock(&core.streams);
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            streams.stream_mut(from).set = DeviceSet::Unknown;
            streams.claim_owner(from, Some(seq)).expect("the source is free")
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-move-source", move || {
                let _ = done.send(worker.await_release(Dir::Rx, to.1 as usize, &next));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination reached its source wait");
        assert!(!device.calls().iter().any(|call| call == "rx_open 0 [0, 1]"), "it did not open during the source configure: {:?}", device.calls());
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, from, seq), "the source's refusal refuses the move");
            streams.give_back(claim);
        }
        let interrupted_on_refusal = result_rx.recv_timeout(Wall::from_secs(10)).is_ok_and(|result| result.is_none());
        if !interrupted_on_refusal {
            core.device_lost("test: release wait did not re-read the refused plan");
        }
        owner.join().unwrap();
        assert!(interrupted_on_refusal, "the refused plan was not re-read before the watchdog");
        assert!(!device.calls().iter().any(|call| call == "rx_open 0 [0, 1]"), "the destination stayed unopened: {:?}", device.calls());
    }

    #[test]
    fn ur_34_a_move_destination_waits_for_source_busy_after_channel_release() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| {
            Line::new(timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels, ratio },
                lead: 0,
                call: 0,
                release_window: core.ticks(core.description.timing.release_window_ns),
                allowance: 0,
            })
        };
        device.rx_open(0, &[0]).unwrap();
        let (next, claim) = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(2)));
            streams.lines.insert(to, line(Channels::first(0)));
            streams.lines.book_move(from, to, 1, Some(10_000), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
            let claim = streams.claim_owner(from, Some(1)).expect("the source is free");
            let next = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(1)).unwrap();
            (next, claim)
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-move-destination", move || {
                let _ = done.send(worker.owner_configure(Dir::Rx, to.1 as usize, &next));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination waits when the source released the moved channel but still configures the move");
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [1]"), "the destination stayed unopened: {:?}", device.calls());
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, from, 1), "source refusal records the move on both lines");
            streams.give_back(claim);
        }
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)), "the refused destination segment was interrupted");
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [1]"), "the refused destination never opened: {:?}", device.calls());
    }

    #[test]
    fn ur_34_a_release_skips_a_stream_held_busy_by_from_none() {
        let (core, device) = two_by_two_with_device();
        let stream = stream_key(Dir::Rx, 0);
        let claim = {
            let mut streams = lock(&core.streams);
            let state = streams.stream_mut(stream);
            state.set = DeviceSet::Open(Channels::first(1));
            streams.search(stream, false);
            streams.claim_from_none(stream).expect("the stream is free")
        };
        assert!(!core.release_unneeded(Dir::Rx, 0), "a from-none owner still holds the stream");
        assert!(!device.calls().iter().any(|call| call == "rx_close 0"), "the owner's close overlapped from-none: {:?}", device.calls());
        {
            let streams = lock(&core.streams);
            assert_eq!(streams.holder(stream), Holder::FromNone);
        }
        lock(&core.streams).give_back(claim);
    }

    #[test]
    fn ur_34_a_release_closes_an_idle_streamer_needed_by_a_move() {
        let (core, device, _next, from, _to, _seq) = waiting_move();
        lock(&core.streams).search(from, false);
        assert!(core.release_unneeded(Dir::Rx, from.1 as usize));
        assert!(device.calls().iter().any(|call| call == "rx_close 1"), "{:?}", device.calls());
        assert_eq!(lock(&core.streams).stream(from).set, DeviceSet::Unknown);
    }

    #[test]
    fn ur_34_a_release_keeps_an_idle_streamer_with_no_peer_need() {
        let (core, device) = two_by_two_with_device();
        let stream = stream_key(Dir::Rx, 0);
        device.rx_open(0, &[1]).unwrap();
        {
            let mut streams = lock(&core.streams);
            let state = streams.stream_mut(stream);
            state.set = DeviceSet::Open(Channels::first(0).with(1));
            streams.search(stream, false);
        }
        assert!(!core.release_unneeded(Dir::Rx, 0));
        assert!(!device.calls().iter().any(|call| call == "rx_close 0"), "{:?}", device.calls());
        assert_eq!(lock(&core.streams).stream(stream).set, DeviceSet::Open(Channels::first(0).with(1)));
    }

    #[test]
    fn ur_34_a_waited_open_reloads_a_move_refused_while_it_waits() {
        let (core, device, next, from, to, seq) = waiting_move();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-test", move || {
                let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner reached its device wait");
        let mark = device.calls().len();
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, from, seq), "the source's device refusal refuses the move");
        }
        owner.join().unwrap();
        let result = result_rx.recv_timeout(Wall::from_secs(10)).unwrap();
        assert!(matches!(result, Ok(false)), "the stale segment was interrupted: {result:?}");
        assert!(lock(&core.streams).line(to).unwrap().halted(10_000));
        assert!(!device.calls()[mark..].iter().any(|call| call == "rx_open 0 [0, 1]"), "the stale destination was not opened: {:?}", device.calls());
        assert_eq!(lock(&core.streams).holder(to), Holder::Free, "the interrupted owner released its claim");
    }

    #[test]
    fn ur_34_a_owner_does_not_open_a_move_plan_removed_before_its_turn() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| {
            Line::new(timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels, ratio },
                lead: 0,
                call: 0,
                release_window: core.ticks(core.description.timing.release_window_ns),
                allowance: 0,
            })
        };
        device.rx_open(0, &[0]).unwrap();
        let stale = {
            let mut streams = lock(&core.streams);
            *streams = Streams::with_streams(2, 2);
            streams.lines.insert(from, line(Channels::first(2)));
            streams.lines.insert(to, line(Channels::first(0)));
            streams.lines.book_move(from, to, 1, Some(10_000), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
            let stale = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(1)).unwrap();
            assert!(streams.refuse(&core, from, 1), "the source refusal removes the destination plan");
            stale
        };

        assert!(matches!(core.owner_configure(Dir::Rx, to.1 as usize, &stale), Ok(false)));
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [1]"), "the removed plan never opens: {:?}", device.calls());
    }

    #[test]
    fn ur_34_a_waited_open_aborts_when_the_planned_segment_changes() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| {
            Line::new(timeline::Stream {
                direction: Direction::Rx,
                origin: 0,
                config: Config { channels, ratio },
                lead: 0,
                call: 0,
                release_window: core.ticks(core.description.timing.release_window_ns),
                allowance: 0,
            })
        };
        device.rx_open(0, &[0]).unwrap();
        let next = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(1)));
            streams.lines.insert(to, line(Channels::first(0)));
            let seq = streams.next_seq();
            assert!(streams.book(&core, to, Item { e: 10_000, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(Config { channels: Channels::first(1), ratio }) }));
            streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            streams.stream_mut(to).configs.insert(seq, Arc::new(ColdConfig::new(BTreeMap::new())));
            streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(seq)).unwrap()
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let expected = next.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-replan", move || {
                let _ = done.send(worker.owner_configure(Dir::Rx, to.1 as usize, &expected));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination reaches its source wait");
        let changed_by = {
            let mut streams = lock(&core.streams);
            let seq = streams.next_seq();
            assert!(
                streams.book(
                    &core,
                    to,
                    Item {
                        e: next.segment.origin + 5_000,
                        seq,
                        ready: core.now(),
                        release: NO_RELEASE,
                        delivered: None,
                        refused: false,
                        kind: Kind::Cold(Config { channels: next.segment.config.channels, ratio: Rational::new(100, 1).unwrap() }),
                    }
                ),
                "the destination plan changes without a refusal"
            );
            let new = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(seq)).expect("the accepted replacement segment");
            assert!(new.segment.origin > next.segment.origin && new.segment.config.ratio == Rational::new(100, 1).unwrap() && !new.refused);
            streams.record_open(from, None);
            seq
        };
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)));
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [0]"), "the stale segment did not open: {:?}", device.calls());
        assert_eq!(lock(&core.streams).holder(to), Holder::Free, "the interrupted owner gave back its claim after replan {changed_by}");
    }

    #[test]
    fn ur_34_a_rate_refusal_leaves_the_stream_record_unknown() {
        let (core, device) = two_by_two_with_device();
        lock(&core.rates)[Dir::Rx as usize] = Some(Err("UR-13: test rate-set refusal".to_owned()));
        assert!(core.open_streamer(Dir::Rx, 0, Channels::first(1)).is_err());
        let streams = lock(&core.streams);
        let state = streams.stream(stream_key(Dir::Rx, 0));
        assert_eq!(state.set, DeviceSet::Unknown);
        assert_eq!(state.configured, None);
        let calls = device.calls();
        let open = calls.iter().position(|call| call == "rx_open 0 [0]").unwrap();
        let close = calls.iter().position(|call| call == "rx_close 0").unwrap();
        assert!(open < close, "the opened streamer was closed before the record was refused: {calls:?}");
    }

    #[test]
    fn ur_34_a_waited_open_ends_at_device_loss_without_closing_after_loss() {
        let (core, device, next, _from, _to, _seq) = waiting_move();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-loss-wait", move || {
                let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner reached its wait");
        core.device_lost("test: lost while waiting for a streamer");
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)));
        let log = device.calls();
        let marked = log.iter().position(|call| call == "mark_lost").unwrap();
        assert!(!log[marked + 1..].iter().any(|call| call.starts_with("rx_close") || call.starts_with("rx_open")), "a lost device is never closed or reopened: {log:?}");
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Rx, 0)), Holder::Free);
    }

    #[test]
    fn ur_34_a_waited_open_ends_at_provider_stop_without_opening() {
        let (core, device, next, _from, _to, _seq) = waiting_move();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("rx-stop-wait", move || {
                let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner reached its wait");
        {
            let mut streams = lock(&core.streams);
            let at = core.now();
            streams.stop = Some((at, StopMode::Orderly));
            streams.end(&core, at, false, false);
        }
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)));
        assert!(!device.calls().iter().any(|call| call == "rx_open 0 [0, 1]"), "{:?}", device.calls());
        assert_eq!(lock(&core.streams).holder(stream_key(Dir::Rx, 0)), Holder::Free);
    }

    #[test]
    fn ur_34_a_waited_transmit_open_observes_provider_stop_with_a_later_cold_reserved() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Tx, 1), stream_key(Dir::Tx, 0));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(timeline::Stream { direction: Direction::Tx, origin: 0, config: Config { channels, ratio }, lead: 1_000, call: 0, release_window: 0, allowance: 0 });
        device.tx_open(1, &[0]).unwrap();
        let (source, next) = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(1)));
            streams.lines.insert(to, line(Channels::first(0)));
            streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
            let move_seq = streams.next_seq();
            streams.lines.book_move(from, to, 0, Some(10_000), 0, move_seq, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            streams.stream_mut(to).configs.insert(move_seq, Arc::new(super::super::control::ColdConfig::new(BTreeMap::new())));
            let cold_seq = streams.next_seq();
            assert!(streams.book(
                &core,
                to,
                Item { e: 20_000, seq: cold_seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(Config { channels: Channels::first(1), ratio }) }
            ));
            streams.changed(&core, to);
            let source = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).expect("the old source segment");
            let next = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(move_seq)).expect("the moved transmit segment");
            assert!(next.segment.cut.is_some_and(|cut| next.segment.instant(cut).unwrap() > next.segment.origin));
            (source, next)
        };

        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| {
            let _ = entered.send(());
        });
        let worker = core.clone();
        let next_for_owner = next.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core
            .device
            .host()
            .spawn("tx-stop-with-later-cold", move || {
                let _ = done.send(worker.owner_configure(Dir::Tx, 0, &next_for_owner));
            })
            .unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the transmit owner reached its release wait");
        let still_planned = {
            let mut streams = lock(&core.streams);
            streams.stop = Some((5_000, StopMode::Orderly));
            streams.end(&core, 5_000, false, false);
            streams.planned(to).iter().any(|planned| planned.segment == next.segment)
        };
        core.close_device(Dir::Tx, 1);
        {
            let mut streams = lock(&core.streams);
            streams.record_open(from, None);
            streams.complete(from, (source.segment.origin, source.segment.config, source.segment.by));
        }
        owner.join().unwrap();
        let result = result_rx.recv_timeout(Wall::from_secs(10)).unwrap();
        let calls = device.calls();
        assert!(still_planned, "TX End before the origin leaves the reserved move segment in the plan");
        assert!(!result.unwrap(), "Provider stop ends the wait");
        assert!(!calls.iter().any(|call| call == "tx_open 0 [0]"), "no target streamer opens after stop: {calls:?}");
        assert_eq!(lock(&core.streams).holder(to), Holder::Free);
    }

    #[test]
    fn ur_25_a_transmit_move_after_a_refused_source_change_begins_the_destination_as_if_not_refused() {
        // RM-25, RM-34 (design 43): the source's change C (2 MS/s, due 100 ms, origin 150 ms) is
        // refused, and a move of channel 0 into the empty destination, due at 300 ms, is booked at
        // 100 ms. The destination's clock is registered for 350 ms (a lead after the cut C would
        // have had), whether or not C is refused; C's own clock ends at its origin, 150 ms.
        let run = |refuse: bool| {
            let core = two_by_two();
            let (from, to) = (stream_key(Dir::Tx, 0), stream_key(Dir::Tx, 1));
            let ms = |n: i64| core.ticks(n * 1_000_000);
            let line = |channels| {
                Line::new(timeline::Stream {
                    direction: Direction::Tx,
                    origin: 0,
                    config: Config { channels, ratio: Rational::new(200, 1).unwrap() },
                    lead: ms(50),
                    call: 0,
                    release_window: 0,
                    allowance: 0,
                })
            };
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(1)));
            streams.lines.insert(to, line(Channels::first(0)));
            let seq = streams.next_seq();
            assert!(streams.book(
                &core,
                from,
                Item {
                    e: ms(100),
                    seq,
                    ready: 0,
                    release: NO_RELEASE,
                    delivered: None,
                    refused: false,
                    kind: Kind::Cold(Config { channels: Channels::first(1), ratio: Rational::new(100, 1).unwrap() })
                }
            ));
            if refuse {
                assert!(streams.refuse(&core, from, seq));
            }
            let moved = streams.next_seq();
            streams.lines.book_move(from, to, 0, Some(ms(300)), ms(100), moved, ms(100)).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            let destination = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(moved)).expect("the destination's segment");
            let source = streams.planned(from);
            drop(streams);
            let records = core.clocks.sample_clock_records();
            let ended = |clock: Clock| records.iter().find(|record| record.domain == clock.domain).and_then(|record| record.ended_at.map(|at| at.ticks_in(record.root).unwrap()));
            let (clock, c_clock) = (destination.clock.expect("the destination's clock"), source[1].clock.expect("C's clock"));
            (destination.segment.origin, clock.origin, ended(c_clock), ms(350), ms(150), ms(300))
        };
        let (plain, plain_clock, plain_end, expected, origin, cut) = run(false);
        let (refused, refused_clock, refused_end, ..) = run(true);
        assert_eq!((plain, plain_clock), (expected, expected));
        assert_eq!((refused, refused_clock), (expected, expected), "the destination begins a lead after the cut C would have had");
        assert_eq!(plain_end, Some(cut));
        assert_eq!(refused_end, Some(origin), "the refused clock's record ends at its origin");
    }

    #[test]
    fn ur_34_a_refused_move_reconciles_both_transmit_clocks() {
        let core = two_by_two();
        let (from, to) = (stream_key(Dir::Tx, 0), stream_key(Dir::Tx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let config = |channels| Config { channels, ratio };
        let line = |channels| Line::new(timeline::Stream { direction: Direction::Tx, origin: 0, config: config(channels), lead: core.ticks(50_000_000), call: 0, release_window: 0, allowance: 0 });
        {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(2)));
            streams.lines.insert(to, line(Channels::first(3).without(0).without(1)));
            let e = 10_000 * 200;
            streams.lines.book_move(from, to, 1, Some(e), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            assert!(streams.refuse(&core, from, 1), "the move is refused on both lines");
            assert!(streams.line(from).unwrap().halted(e));
            assert!(streams.line(to).unwrap().halted(e));
        }
        let streams = lock(&core.streams);
        let source = streams.stream(from).clocks.clone();
        let destination = streams.stream(to).clocks.clone();
        drop(streams);
        let records = core.clocks.sample_clock_records();
        let record = |clock: &Clock| records.iter().find(|record| record.domain == clock.domain).expect("a clock's record");
        assert_eq!(source.len(), 2, "the source's refused segment clock was registered: {source:?}");
        assert_eq!(destination.len(), 2, "the destination's refused segment clock was registered: {destination:?}");
        for (_, ended) in [&source, &destination].into_iter().flat_map(|clocks| clocks.iter()) {
            assert!(*ended, "both lines' clocks were reconciled after refusal: {source:?} {destination:?}");
        }
        let old_source = record(&source[0].0);
        assert_eq!(old_source.ended_at.unwrap().ticks_in(old_source.root).unwrap(), 10_000 * 200, "the old source clock ends at the move cut");
        let old_destination = record(&destination[0].0);
        assert_eq!(old_destination.ended_at.unwrap().ticks_in(old_destination.root).unwrap(), 10_000 * 200, "the old destination clock ends at the move cut");
        for (clock, _) in [&source[1], &destination[1]] {
            let refused = record(clock);
            assert_eq!(refused.ended_at.unwrap().ticks_in(refused.root).unwrap(), refused.origin.ticks_in(refused.root).unwrap(), "a refused move clock ends at its own origin");
        }
    }

    #[test]
    fn ur_26_an_end_takes_delivered_from_each_segment_s_own_stream() {
        // UR-26, RM-16 (S12b): an end cuts each receive line at its own first sample not
        // delivered; an Orderly end floors it at its own instant, per line.
        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            for (stream, k) in [(stream_key(Dir::Rx, 0), 1_000), (stream_key(Dir::Rx, 1), 500)] {
                streams.started(stream, Some((0, cfg(1), None)), 0, cfg(1));
                streams.delivered(stream, 0, cfg(1), k);
            }
            streams.end(&core, 5_000, true, false);
        }
        let streams = lock(&core.streams);
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0))[0].segment.cut, Some(1_000));
        assert_eq!(streams.planned(stream_key(Dir::Rx, 1))[0].segment.cut, Some(500));

        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            for (stream, k) in [(stream_key(Dir::Rx, 0), 3_000), (stream_key(Dir::Rx, 1), 500)] {
                streams.started(stream, Some((0, cfg(1), None)), 0, cfg(1));
                streams.delivered(stream, 0, cfg(1), k);
            }
            streams.end(&core, 400_000, false, false);
        }
        let streams = lock(&core.streams);
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0))[0].segment.cut, Some(3_000), "its own floor");
        assert_eq!(streams.planned(stream_key(Dir::Rx, 1))[0].segment.cut, Some(2_000), "its own instant, its own floor");

        // One end, one item per line, the same instant and arrival order.
        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            streams.end(&core, 5_000, false, false);
            streams.end(&core, 6_000, false, false);
        }
        let streams = lock(&core.streams);
        for dir in [Direction::Rx, Direction::Tx] {
            for k in 0..2 {
                let ends: Vec<_> = streams.line((dir, k)).unwrap().items.iter().filter(|item| matches!(item.kind, Kind::End { .. })).collect();
                assert_eq!(ends.len(), 1, "{dir:?} {k}");
                assert_eq!((ends[0].e, ends[0].seq), (5_000, 1), "{dir:?} {k}");
            }
        }
    }

    // ------------------------------------------------------------ 1b-6a: the holder and the read

    #[test]
    fn ur_25_a_claim_owner_takes_only_a_free_stream() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        assert_eq!(streams.holder(k), Holder::Free);
        let claim = streams.claim_owner(k, None).expect("a free stream");
        assert_eq!(streams.holder(k), Holder::Owner { seq: None });
        assert!(streams.claim_owner(k, Some(7)).is_none());
        assert!(streams.claim_from_none(k).is_none());
        assert!(streams.claim_release(k).is_none());
        assert_eq!(streams.holder(k), Holder::Owner { seq: None });
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
        let claim = streams.claim_owner(k, Some(9)).expect("free again");
        assert_eq!(streams.holder(k), Holder::Owner { seq: Some(9) });
        assert!(!streams.move_source_busy(k, Some(9)), "no move names this stream");
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
    }

    #[test]
    fn ur_25_a_claim_release_needs_a_free_idle_stream_with_no_active_segment() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        streams.search(k, false);
        let claim = streams.claim_release(k).expect("idle and free");
        assert_eq!(streams.holder(k), Holder::Release);
        assert!(streams.claim_owner(k, None).is_none());
        assert!(streams.claim_from_none(k).is_none());
        assert!(streams.claim_release(k).is_none());
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);

        streams.search(k, true);
        assert!(streams.claim_release(k).is_none(), "not idle");
        assert_eq!(streams.holder(k), Holder::Free);
        streams.started(k, Some((0, cfg(1), None)), 0, cfg(1));
        assert!(streams.claim_release(k).is_none(), "an active segment");
        let _ = streams.finished(k);

        let claim = streams.claim_owner(k, None).expect("free");
        streams.search(k, false);
        assert!(streams.claim_release(k).is_none(), "the holder alone refuses");
        assert_eq!(streams.holder(k), Holder::Owner { seq: None });
        streams.give_back(claim);
        let claim = streams.claim_release(k).expect("idle, free, no active segment");
        streams.give_back(claim);
    }

    // ------------------------------------------------------------ 1b-6b: the owner's phase

    #[test]
    fn ur_25_a_stream_starts_idle_or_waiting() {
        // UR-25: `start` gives a stream with a segment from T0 (or a transmit clock) a
        // `Pending` phase and one without anything to begin `Idle`; the default state before
        // `start` is not idle, so a release does not close its streamer.
        let mut streams = Streams::with_streams(1, 1);
        let (rx, tx) = (stream_key(Dir::Rx, 0), stream_key(Dir::Tx, 0));
        assert_eq!(streams.phase(rx), Phase::Pending, "the default before start");
        assert_eq!(streams.phase(tx), Phase::Pending);
        assert!(!streams.is_idle(rx));
        assert!(streams.active(rx).is_none());
        assert!(streams.claim_release(rx).is_none(), "a stream before start is not idle");

        streams.starting(rx, false);
        assert_eq!(streams.phase(rx), Phase::Idle);
        let claim = streams.claim_release(rx).expect("start gave the stream nothing to begin");
        streams.give_back(claim);

        streams.starting(rx, true);
        assert_eq!(streams.phase(rx), Phase::Pending);
        assert!(!streams.is_idle(rx));
        assert!(streams.active(rx).is_none());

        streams.starting(tx, true);
        assert_eq!(streams.phase(tx), Phase::Pending, "a transmit clock waits for its first plan read");
        streams.starting(tx, false);
        assert_eq!(streams.phase(tx), Phase::Idle);
    }

    #[test]
    fn ur_26_a_receive_start_without_a_segment_stays_active() {
        // RM-16: uhd-rx begins its T0 stream even when the plan no longer has its segment (a
        // loss during `start`): the phase is active with no segment, and the delivered tuple
        // stays; finishing it records nothing and leaves the stream pending.
        let mut streams = Streams::with_streams(1, 1);
        let rx = stream_key(Dir::Rx, 0);
        let config = cfg(1);
        streams.started(rx, None, 0, config);
        assert_eq!(streams.phase(rx), Phase::Active { segment: None, delivered: Some((0, config, 0)) });
        assert!(!streams.is_idle(rx));
        streams.delivered(rx, 0, config, 500);
        assert_eq!(streams.first_undelivered(rx), Some((0, config, 500)), "the tuple is kept");

        assert_eq!(streams.finished(rx), None);
        assert_eq!(streams.phase(rx), Phase::Pending);
        assert_eq!(streams.first_undelivered(rx), None);
        assert!(streams.stream(rx).completed.is_empty(), "no segment to record");
    }

    #[test]
    fn ur_25_a_search_leaves_the_stream_pending_or_idle() {
        // UR-25: the search for the next planned segment moves the stream between `Pending`
        // and `Idle` only, and a found segment is not yet active.
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        streams.search(k, true);
        assert_eq!(streams.phase(k), Phase::Pending);
        assert!(!streams.is_idle(k));
        assert!(streams.active(k).is_none(), "a planned segment is not begun");
        assert!(streams.claim_release(k).is_none(), "the owner found a segment to begin");

        streams.search(k, false);
        assert_eq!(streams.phase(k), Phase::Idle);
        let claim = streams.claim_release(k).expect("nothing left to begin");
        streams.give_back(claim);

        // Unreachable in production (`begin` runs only with no stream), but the phase is one
        // value: a later search replaces an active segment rather than leaving it beside Idle.
        streams.started(k, Some((0, cfg(1), None)), 0, cfg(1));
        streams.search(k, false);
        assert_eq!(streams.phase(k), Phase::Idle);
        assert!(streams.active(k).is_none());
    }

    #[test]
    fn ur_17_a_receive_segment_moves_from_pending_to_active_to_pending() {
        // UR-17: a found segment becomes active with its floor at zero, each block moves the
        // floor, and finishing returns the segment for the caller's record. A floor written
        // while the stream is not active is not kept.
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        let config = cfg(1);
        let segment = (0, config, None);
        streams.search(k, true);
        streams.delivered(k, 0, config, 500);
        assert_eq!(streams.phase(k), Phase::Pending);
        assert_eq!(streams.first_undelivered(k), None, "nothing is being delivered");

        streams.started(k, Some(segment), 0, config);
        assert_eq!(streams.phase(k), Phase::Active { segment: Some(segment), delivered: Some((0, config, 0)) });
        streams.delivered(k, 0, config, 3_000);
        assert_eq!(streams.first_undelivered(k), Some((0, config, 3_000)));

        assert_eq!(streams.finished(k), Some(segment));
        assert_eq!(streams.phase(k), Phase::Pending);
        assert_eq!(streams.first_undelivered(k), None);
        streams.delivered(k, 0, config, 4_000);
        assert_eq!(streams.phase(k), Phase::Pending, "a finished segment's floor cannot reappear");
        assert!(streams.stream(k).completed.is_empty(), "the caller records completion");

        streams.complete(k, segment);
        assert_eq!(streams.stream(k).completed, [segment]);
    }

    #[test]
    fn ur_26_a_finished_stream_clears_only_its_own_floor() {
        // UR-26, RM-16: finishing stream 1 clears its own floor and leaves stream 0's; an
        // abort's end after both delivered floors cuts stream 0 at its own floor and stream 1,
        // no longer running, at the end's instant instead.
        let core = two_by_two();
        let (rx0, rx1) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let segment = (0, cfg(1), None);
        {
            let mut streams = lock(&core.streams);
            for (stream, k) in [(rx0, 3_000), (rx1, 500)] {
                streams.started(stream, Some(segment), 0, cfg(1));
                streams.delivered(stream, 0, cfg(1), k);
            }
            assert_eq!(streams.finished(rx1), Some(segment));
            assert_eq!(streams.first_undelivered(rx0), Some((0, cfg(1), 3_000)), "stream 0's floor stands");
            assert_eq!(streams.first_undelivered(rx1), None, "stream 1's floor is cleared");
            streams.end(&core, 800_000, true, false);
        }
        let streams = lock(&core.streams);
        assert_eq!(streams.planned(rx0)[0].segment.cut, Some(3_000), "stream 0's own delivered floor");
        assert_eq!(streams.planned(rx1)[0].segment.cut, Some(4_000), "stream 1 is cut at the end's instant, not its old floor");
    }

    #[test]
    fn ur_25_a_claim_from_none_admits_a_read_in_flight() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        assert!(streams.begin_read(k));
        let claim = streams.claim_from_none(k).expect("a read does not refuse from-none");
        assert_eq!(streams.holder(k), Holder::FromNone);
        assert!(streams.claim_from_none(k).is_none(), "a from-none claim is not taken twice");
        assert!(!streams.may_call_from_none(k), "the read is in flight");
        streams.end_read(k);
        assert!(streams.may_call_from_none(k));
        assert!(!streams.reading(k));
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);

        let claim = streams.claim_from_none(k).expect("free again");
        assert!(!streams.begin_read(k), "a from-none holder owns the stream");
        assert!(!streams.reading(k));
        streams.give_back(claim);
        let claim = streams.claim_owner(k, Some(1)).expect("free");
        assert!(!streams.may_call_from_none(k));
        streams.give_back(claim);
        streams.search(k, false);
        let claim = streams.claim_release(k).expect("free, idle");
        assert!(!streams.may_call_from_none(k));
        streams.give_back(claim);
        assert!(!streams.may_call_from_none(k), "a free stream is not a from-none holder");
    }

    #[test]
    fn ur_25_a_begin_read_needs_a_free_stream_and_no_read() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        assert!(streams.begin_read(k));
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::Free);
        assert!(!streams.begin_read(k), "a read is already in flight");
        assert!(streams.reading(k));
        streams.end_read(k);
        assert!(!streams.reading(k));

        let claim = streams.claim_owner(k, None).expect("free");
        assert!(!streams.begin_read(k));
        assert!(!streams.reading(k));
        streams.give_back(claim);
        let claim = streams.claim_from_none(k).expect("free");
        assert!(!streams.begin_read(k), "a from-none enable owns the stream");
        streams.give_back(claim);
        streams.search(k, false);
        let claim = streams.claim_release(k).expect("free, idle");
        assert!(!streams.begin_read(k));
        streams.give_back(claim);
    }

    #[test]
    fn ur_25_a_begin_shutdown_read_ignores_the_holder() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);

        streams.begin_shutdown_read(k);
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::Free);
        streams.end_read(k);

        let claim = streams.claim_owner(k, Some(3)).expect("free");
        streams.begin_shutdown_read(k);
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::Owner { seq: Some(3) });
        streams.end_read(k);
        streams.give_back(claim);

        let claim = streams.claim_from_none(k).expect("free");
        streams.begin_shutdown_read(k);
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::FromNone);
        streams.end_read(k);
        streams.give_back(claim);

        streams.search(k, false);
        let claim = streams.claim_release(k).expect("free, idle");
        streams.begin_shutdown_read(k);
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::Release);
        streams.end_read(k);
        streams.give_back(claim);

        let claim = streams.claim_from_none(k).expect("free");
        streams.begin_shutdown_read(k);
        streams.begin_shutdown_read(k);
        assert!(streams.reading(k), "a second shutdown read leaves it set");
        streams.end_read(k);
        assert!(!streams.reading(k));
        streams.give_back(claim);
    }

    #[test]
    #[should_panic(expected = "UR-25: end_read without a read in flight")]
    fn ur_25_an_end_read_without_a_read_panics_in_the_test_build() {
        let mut streams = Streams::with_streams(1, 1);
        streams.end_read(stream_key(Dir::Rx, 0));
    }

    #[test]
    #[should_panic(expected = "UR-25: a claim of Owner { seq: None } was not given back")]
    fn ur_25_a_claim_dropped_without_give_back_panics_in_the_test_build() {
        let mut streams = Streams::with_streams(1, 1);
        let claim = streams.claim_owner(stream_key(Dir::Rx, 0), None).expect("free");
        drop(claim);
    }

    #[test]
    fn ur_25_a_give_back_of_another_holder_panics_in_the_test_build_and_changes_nothing() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        let claim = streams.claim_owner(k, None).expect("free");
        let wrong = Claim { stream: k, holder: Holder::Free };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            streams.give_back(wrong);
        }));
        assert!(caught.is_err(), "a claim the stream does not hold is refused");
        assert_eq!(streams.holder(k), Holder::Owner { seq: None }, "the real holder is untouched");
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
    }

    #[test]
    fn ur_25_move_source_busy_reads_the_owner_of_the_move() {
        let mut streams = Streams::with_streams(2, 1);
        let (from, to) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let rx_line = |channels| Line::new(timeline::Stream { direction: Direction::Rx, origin: 0, config: Config { channels, ratio }, lead: 0, call: 0, release_window: 0, allowance: 0 });
        streams.lines.insert(from, rx_line(Channels::first(2)));
        streams.lines.insert(to, rx_line(Channels::first(1)));
        streams.lines.book_move(from, to, 1, Some(10_000), 0, 7, 0).unwrap();

        let claim = streams.claim_owner(from, Some(7)).expect("free");
        assert!(streams.move_source_busy(to, Some(7)));
        assert!(!streams.move_source_busy(to, Some(8)), "another command's move");
        assert!(!streams.move_source_busy(to, None));
        streams.give_back(claim);

        let claim = streams.claim_owner(from, Some(8)).expect("free");
        assert!(!streams.move_source_busy(to, Some(7)), "a different seq");
        streams.give_back(claim);
        let claim = streams.claim_from_none(from).expect("free");
        assert!(!streams.move_source_busy(to, Some(7)), "from-none is not the owner");
        streams.give_back(claim);
        streams.search(from, false);
        let claim = streams.claim_release(from).expect("free, idle");
        assert!(!streams.move_source_busy(to, Some(7)), "release is not the owner");
        streams.give_back(claim);
        assert!(!streams.move_source_busy(to, Some(7)), "free is not the owner");
    }

    #[test]
    fn ur_34_a_release_gives_back_a_claim_when_loss_arrives_after_the_claim() {
        let (core, device, _next, from, _to, _seq) = waiting_move();
        lock(&core.streams).search(from, false);
        let before = device.calls().len();
        let (entered, reached) = std::sync::mpsc::channel();
        let (resume, wait) = std::sync::mpsc::channel();
        let wait = std::sync::Mutex::new(wait);
        super::super::test_support::probe(&core, "release_claimed", move |_| {
            let _ = entered.send(());
            let _ = wait.lock().unwrap().recv();
        });
        let worker = core.clone();
        let release = std::thread::spawn(move || worker.release_unneeded(Dir::Rx, from.1 as usize));
        reached.recv_timeout(Wall::from_secs(10)).expect("the release took its claim");
        assert_eq!(lock(&core.streams).holder(from), Holder::Release);
        core.device_lost("test: a loss between the release claim and its close");
        let _ = resume.send(());
        assert!(!release.join().expect("the release thread"), "the lost stream is not closed");
        assert_eq!(lock(&core.streams).holder(from), Holder::Free, "the claim is given back");
        assert_eq!(lock(&core.streams).stream(from).set, DeviceSet::Open(Channels::first(1)), "no close is recorded");
        assert!(!device.calls()[before..].iter().any(|call| call == "rx_close 1"), "{:?}", device.calls());
    }
}
