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
use ezsdr_kernel::time::{ClockRegistry, TimeAuthority, TimePoint};
use ezsdr_radio::kinds;
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::staging::Busy;
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, StreamKey};
use serde_json::{Value as Json, json};

use super::control::{ColdConfig, Planned};
use super::mode::{DescriptionExt, channel_list};
use crate::device::{Device, DeviceError, Dir, Settings, decimation};
use crate::device::{Iq, TxCode, TxReport};
use crate::host::{ClockGuard, ClockMutex};

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
/// `rates` —, so a publication that takes them may begin. The whole of the check: the
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
    Uncertain,
    Lost,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct RxIntent {
    cut: Option<i64>,
    end_now: bool,
    terminate: Option<StopMode>,
}

/// One receive stream's device state and the owner's intent.
#[derive(Default)]
pub(crate) struct RxChain {
    device: RxDevice,
    intent: RxIntent,
    segment: Option<SegmentKey>,
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
    pub fn adopt_start(&mut self, segment: SegmentKey, at: i64) {
        assert!(matches!(self.device, RxDevice::Off));
        self.segment = Some(segment);
        self.device = RxDevice::Armed { at };
    }

    pub fn begin_result(&mut self, segment: SegmentKey, at: i64, result: Result<(), DeviceError>) -> Result<(), DeviceError> {
        assert!(matches!(self.device, RxDevice::Off | RxDevice::Uncertain));
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

    pub fn lose(&mut self) {
        self.device = RxDevice::Lost;
    }

    pub fn finish(&mut self) {
        assert!(matches!(self.device, RxDevice::Off | RxDevice::Uncertain | RxDevice::Lost));
        self.segment = None;
        self.intent.cut = None;
        self.intent.end_now = false;
    }

    pub fn abandon_unowned(&mut self) {
        if self.device != RxDevice::Lost {
            self.device = RxDevice::Uncertain;
        }
        self.segment = None;
        self.intent.end_now = true;
        self.intent.terminate.get_or_insert(StopMode::Abort);
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
/// [`Streams`] (`seq`, `ended`, `stop`); what belongs to a stream is in its
/// [`StreamState`] (RM-29, design 33 D15). Lock order: this, then `rec`; never held across a
/// device call.
pub(crate) struct Streams {
    /// The instance's lines, one per stream, keyed by `(direction, k)` (RM-25, RM-29); none for
    /// a receive side with no link.
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

/// What a segment's owner does before it begins, from the record (UR-25): configure the open
/// streamer, or open it again and configure.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Owner {
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
    /// Its owner: a receive or transmit segment's device calls (UR-25).
    Owner,
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

/// One stream's own state (RM-29, design 33 D15): stream `k` of its direction, and not the
/// instance's other streams.
#[derive(Default)]
pub(crate) struct StreamState {
    /// The configuration each segment begins with, by the command that began it (UR-25).
    pub configs: BTreeMap<u64, Arc<ColdConfig>>,
    /// The device's record of the stream's streamer (UR-13, UR-36): its channel set.
    pub set: DeviceSet,
    /// The streamer's open generation: every successful open of the stream is a new streamer with a
    /// new report queue, which the correlations of the bursts in flight are told from (UR-36,
    /// UR-28).
    pub generation: u64,
    /// The command of the segment whose configuration the device was last given, `seq`; none
    /// after a failed configure or open. The owner configures again unless this is its own
    /// segment's command and the record holds its set open (UR-36). For a transmit stream it is
    /// the generation of the values the device holds (RM-21): arm publishes 0, a loss clears it.
    pub configured: Option<u64>,
    /// Who may call the stream's device and whether uhd-tx's report read is in flight
    /// (UR-25).
    claims: Claims,
    /// uhd-rx's segment: its origin and configuration, and the first sample of it not yet
    /// delivered (UR-17, RM-16); none while it delivers nothing.
    delivered: Option<(i64, Config, i64)>,
    /// The transmit clock registered at `arm` (RM-25): none or one.
    pub clock: Option<Clock>,
    /// The transmit value rule's record of the stream's bookings and Stops (RM-21): the one place
    /// a receipt is judged by (never the hand-over state).
    pub busy: Busy,
}

/// The key the instance's lines hold `dir`'s stream `k` under (RM-29).
pub(crate) fn stream_key(dir: Dir, k: usize) -> StreamKey {
    match dir {
        Dir::Rx => (Direction::Rx, k as u32),
        Dir::Tx => (Direction::Tx, k as u32),
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

    /// UR-25: takes `stream` for its owner's device calls; `None`, with nothing changed, when
    /// another holder has it.
    #[must_use]
    pub fn claim_owner(&mut self, stream: StreamKey) -> Option<Claim> {
        let claims = &mut self.stream_mut(stream).claims;
        if claims.holder != Holder::Free {
            return None;
        }
        claims.holder = Holder::Owner;
        Some(Claim { stream, holder: Holder::Owner })
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
    /// still holding the stream cannot block it (UR-16).
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

    /// UR-17, UR-25: uhd-rx begins a segment; the first sample not delivered is zero, named by the
    /// clock and configuration uhd-rx's stream has.
    pub fn started(&mut self, stream: StreamKey, origin: i64, config: Config) {
        self.stream_mut(stream).delivered = Some((origin, config, 0));
    }

    /// UR-17: uhd-rx delivered its segment up to `k`.
    pub fn delivered(&mut self, stream: StreamKey, origin: i64, config: Config, k: i64) {
        if let Some(delivered) = self.stream_mut(stream).delivered.as_mut() {
            *delivered = (origin, config, k);
        }
    }

    /// UR-17: the owner finishes its segment.
    pub fn finished(&mut self, stream: StreamKey) {
        self.stream_mut(stream).delivered = None;
    }

    /// UR-17, RM-16: the first sample of the segment uhd-rx is delivering that it has not yet
    /// delivered.
    pub fn first_undelivered(&self, stream: StreamKey) -> Option<(i64, Config, i64)> {
        self.stream(stream).delivered
    }

    /// Whether uhd-tx's report read is in flight (UR-25), as a test reads it.
    #[cfg(test)]
    pub fn reading(&self, stream: StreamKey) -> bool {
        self.stream(stream).claims.reading
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
    pub fn book(&mut self, core: &Core, stream: StreamKey, mut item: Item, id: Option<ActionId>) -> bool {
        let delivered = self.first_undelivered(stream);
        let Some(line) = self.lines.line(stream) else { return false };
        if stream.0 == Direction::Rx && matches!(item.kind, Kind::End { .. }) {
            let cuts = |origin, config| line.made(origin, config, item.e, item.seq).ok().flatten().is_some_and(|s| s.cut.is_none());
            item.delivered = item.delivered.or(delivered.filter(|(origin, config, _)| cuts(*origin, *config)).map(|(_, _, k)| k));
        }
        if let Err(error) = self.lines.book(stream, item) {
            // The Action being booked names the refusal: a `start_rx` is a command, a Stop or an end a stop.
            core.command_rejected(id, if item.kind == Kind::Start { "command" } else { "stop" }, &format!("RM-25: {error}"));
            return false;
        }
        self.changed(core, stream, id);
        true
    }

    /// RM-25, VH-2: the device refused the configuration of the segment command `seq` began;
    /// the command is booked as refused, which halts the stream. False when it is no longer
    /// booked, or when the stream has no line.
    pub fn refuse(&mut self, core: &Core, stream: StreamKey, seq: u64) -> bool {
        if self.lines.line(stream).is_none() {
            return false;
        }
        match self.lines.refuse(stream, seq) {
            Ok(refused) => {
                self.changed(core, stream, None);
                refused
            }
            Err(error) => {
                // Only a `start_rx` is refused at its instant (RM-25).
                core.command_rejected(None, "command", &format!("RM-25: {error}"));
                false
            }
        }
    }

    /// RM-16, VH-2: a loss or `Provider::stop` at `at` ends every stream, once, each line with
    /// the same instant and arrival order; a loss counts as received with the faults, at the
    /// Run's start (RM-25).
    pub fn end(&mut self, core: &Core, at: i64, abort: bool, lost: bool) {
        // A loss clears every transmit mark, even when a Stop or `Provider::stop` ended the Run first (RM-21).
        if lost {
            for stream in &mut self.tx {
                stream.configured = None;
            }
        }
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        let seq = if lost { 0 } else { self.next_seq() };
        for stream in &mut self.tx {
            stream.busy.stop(at, core.tx_in_flight());
        }
        for dir in [Dir::Rx, Dir::Tx] {
            for k in 0..self.count(dir) {
                self.book(core, stream_key(dir, k), Item { e: at, seq, ready: at, delivered: None, refused: false, kind: Kind::End { abort } }, None);
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
        let Some(e) = by.and_then(|by| self.lines.line(stream).and_then(|held| held.items.iter().find(|item| item.seq == by).map(|item| item.e))) else { return };
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
            .map(|segment| Planned {
                segment: *segment,
                k: stream.1 as usize,
                settings: segment.by.and_then(|by| state.configs.get(&by)).cloned(),
                refused: segment.by.is_some_and(|by| line.items.iter().any(|item| item.seq == by && item.refused)),
            })
            .collect()
    }

    /// What the owner of `stream` does before it begins a segment that holds `set` (UR-36): a
    /// configure when the record has that set open; an open otherwise.
    pub fn owner_step(&self, stream: StreamKey, set: Channels) -> Owner {
        match self.stream(stream).set {
            DeviceSet::Open(open) if open == set => Owner::Configure,
            _ => Owner::Open,
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

    /// The record of a configure of `stream`: its segment's command, or a transmit value generation,
    /// once it succeeded; none when it failed (UR-36).
    pub fn record_configured(&mut self, stream: StreamKey, mark: Option<u64>) {
        self.stream_mut(stream).configured = mark;
    }

    /// A plan changed: a transmit clock whose segment the plan now cuts ends at the cut (RM-25,
    /// VH-2). uhd-tx reads the cut on its next turn (UR-25).
    pub fn changed(&mut self, core: &Core, stream: StreamKey, id: Option<ActionId>) {
        // A loss or `Provider::stop` at or before the clock's origin cuts its segment there; a
        // clock the registry already ended is not ended again. Only a transmit stream holds a
        // clock here and a transmit line is booked only with Stop and end, so the failure is
        // always a `stop`'s.
        let cut = self.stream(stream).clock.zip(self.lines.line(stream).and_then(|line| line.plan.first()).and_then(|segment| segment.cut));
        if let Some((clock, cut)) = cut.filter(|(clock, _)| !core.clocks.get(clock.domain).is_ok_and(|domain| domain.ended_at.is_some())) {
            if let Err(error) = core.clocks.end(clock.domain, core.at(clock.instant(cut))) {
                core.command_rejected(id, "stop", &format!("UR-25: {error}"));
            }
        }
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
    /// Each receive chain's owner transitions share a clock lock (L1).
    pub rx_chain: Vec<ClockMutex<RxChain>>,
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

    /// The in-flight window of a transmit stream, in root ticks (RM-16): what a Stop leaves in the device.
    pub fn tx_in_flight(&self) -> i64 {
        self.ticks(self.description.timing.tx_in_flight_ns)
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
            let applied = self.device.apply(dir, *chan, settings)?;
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

    /// The open-time four-zero burst (UR-22) clears stale TX header state. One device-clock deadline
    /// bounds both report waits and immediate-error/unrelated-report progress.
    #[cfg(test)]
    pub fn tx_flush(&self, stream: usize, channels: &[usize]) -> TxFlushResult {
        if channels.is_empty() || self.is_lost() {
            return TxFlushResult { acknowledged: channels.is_empty(), ack_tick: None, stopped: self.is_lost() };
        }
        let _reports = self.lock_tx_reports(stream);
        self.tx_flush_locked(stream, channels)
    }

    fn tx_flush_locked(&self, stream: usize, channels: &[usize]) -> TxFlushResult {
        let expected: BTreeSet<usize> = (0..channels.len()).collect();
        let mut acknowledged = BTreeSet::new();
        let mut ack_tick: Option<i64> = None;
        let mut problem = None;
        let zeros = [Iq::default(); 4];
        let buffers: Vec<&[Iq]> = channels.iter().map(|_| &zeros[..]).collect();
        let host = self.device.host();
        let until = host.now() + Wall::from_millis(100);
        let send_timeout = until.saturating_sub(host.now());
        match self.device.tx_send(stream, &buffers, None, true, true, send_timeout) {
            Ok(4) => {}
            Ok(n) => problem = Some(format!("flush send accepted {n} of 4 samples")),
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
                    } else if matches!(report.code, TxCode::Underflow | TxCode::TimeError) && expected.contains(&report.channel) && !acknowledged.contains(&report.channel) {
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

    /// Opens TX and completes its flush while the report gate remains held, so no other
    /// async reader can take the flush acknowledgement.
    pub fn open_tx_device(&self, k: usize, set: Channels) -> Result<TxFlushResult, DeviceError> {
        let channels = channel_list(set);
        let _reports = self.lock_tx_reports(k);
        self.open_tx_device_locked(k, set, &channels)
    }

    /// Replaces an uncertain TX streamer and runs its open-time flush without allowing a
    /// concurrent reader to take reports from the old generation.
    pub fn reopen_tx_streamer(&self, k: usize, set: Channels) -> Result<TxFlushResult, DeviceError> {
        let stream = stream_key(Dir::Tx, k);
        let channels = channel_list(set);
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

    /// UR-13, UR-36: the device's open of `k`'s streamer of `dir` on `set`; the record is the
    /// caller's (see `record_opened`).
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
    /// and `arm` call this with `streams` released (UR-25).
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
    }

    #[cfg(not(test))]
    fn assert_rx_chain_reusable(&self, _k: usize, _operation: &str) {}

    #[cfg(test)]
    pub fn assert_rx_chains_reusable(&self) {
        for k in 0..self.rx_chain.len() {
            self.assert_rx_chain_reusable(k, "close_streams");
        }
    }

    /// The owner's device calls for a segment it begins (UR-13, UR-36): the record decides what
    /// is opened and configured. Each device call is made with `streams` released, and its
    /// outcome is recorded after it; a successful open moves the record's generation on, which
    /// is how a thread holding correlations against the replaced streamer learns of it (UR-28).
    /// On error, the caller books the refusal before giving the claim back (UR-25).
    pub fn owner_configure(&self, dir: Dir, k: usize, next: &Planned) -> Result<bool, (DeviceError, Claim)> {
        let stream = stream_key(dir, k);
        let (current, step, claim) = {
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
            let step = streams.owner_step(stream, current.set());
            let Some(claim) = streams.claim_owner(stream) else { return Ok(false) };
            (current, step, claim)
        };
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

    /// Declares and registers stream `k`'s clock with origin `origin` (UR-12, RM-25), for the
    /// unit rigs.
    #[cfg(test)]
    pub fn register_in(&self, dir: Dir, k: usize, n: i64, origin: i64) -> Result<Clock, String> {
        let stream = self.description.stream_node(&self.id, dir, k);
        let ratio = ezsdr_kernel::time::Rational::new(n as u64, 1).map_err(|e| e.to_string())?;
        let handle = self.clocks.declare_sample_clock(stream, self.root, ratio).map_err(|e| e.to_string())?;
        let domain = self.clocks.register_sample_clock(&handle, origin).map_err(|e| e.to_string())?;
        Ok(Clock { domain, origin, n })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezsdr_kernel::time::Rational;

    fn rx_segment() -> SegmentKey {
        SegmentKey { by: Some(7), origin: 100 }
    }

    #[test]
    fn rx_chain_adopt_start_transitions() {
        let mut chain = RxChain::default();
        let segment = rx_segment();
        chain.adopt_start(segment, 90);
        assert_eq!(chain.device(), RxDevice::Armed { at: 90 });
        assert_eq!(chain.segment(), Some(segment));
        assert_eq!(chain.cut(), None);
        assert!(!chain.end_now());
        assert_eq!(chain.terminate(), None);
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
    fn rx_chain_finish_transitions() {
        for device in [RxDevice::Off, RxDevice::Uncertain, RxDevice::Lost] {
            let mut chain = RxChain { device, intent: RxIntent { cut: Some(80), end_now: true, terminate: Some(StopMode::Abort) }, segment: Some(rx_segment()) };
            chain.finish();
            assert_eq!(chain.device(), device);
            assert_eq!(chain.segment(), None);
            assert_eq!(chain.cut(), None);
            assert!(!chain.end_now());
            assert_eq!(chain.terminate(), Some(StopMode::Abort));
        }
    }

    #[test]
    fn rx_chain_lose_is_terminal() {
        for prior in [RxDevice::Off, RxDevice::Armed { at: 90 }, RxDevice::Streaming, RxDevice::Uncertain, RxDevice::Lost] {
            let mut chain = RxChain { device: prior, segment: Some(rx_segment()), ..RxChain::default() };
            chain.lose();
            assert_eq!(chain.device(), RxDevice::Lost);
            chain.finish();
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
    }

    #[test]
    fn rx_chain_abandon_unowned_preserves_loss() {
        let mut chain = RxChain::default();
        chain.lose();
        chain.abandon_unowned();
        assert_eq!(chain.device(), RxDevice::Lost);
        assert_eq!(chain.segment(), None);
        assert!(chain.end_now());
        assert_eq!(chain.terminate(), Some(StopMode::Abort));
    }

    #[test]
    fn ur_22_t3_flush_ack_is_not_taken_by_shutdown() {
        use crate::device::Dir;
        use crate::provider::test_support::LockSpy;
        use std::sync::Arc;

        for reopen in [false, true] {
            let spy = Arc::new(LockSpy::new(crate::device::FakeConfig { clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default() }));
            let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            let gate = spy.stop_before("tx_async");
            let (opened, flush_result) = std::sync::mpsc::channel();
            let opener = core
                .device
                .host()
                .spawn("tx-flush-open", {
                    let core = core.clone();
                    move || {
                        let flush = if reopen { core.reopen_tx_streamer(0, Channels::first(1)) } else { core.open_tx_device(0, Channels::first(1)) };
                        let _ = opened.send(flush.unwrap());
                    }
                })
                .unwrap();
            gate.wait();
            // The flush's acknowledgement is due now: a reader that got past the report gate
            // would take it at once (the host clock stays put, so the flush's deadline does not move).
            spy.set_time(spy.time_now().unwrap() + 1_000_000);

            let shutdown_stream = stream_key(Dir::Tx, 0);
            lock(&core.streams).begin_shutdown_read(shutdown_stream);
            let (shutdown_done, shutdown_report) = std::sync::mpsc::channel();
            let shutdown = core
                .device
                .host()
                .spawn("shutdown-tx-report", {
                    let core = core.clone();
                    move || {
                        let _reports = core.lock_tx_reports(0);
                        let report = core.device.tx_async(0, Wall::ZERO).unwrap();
                        lock(&core.streams).end_read(shutdown_stream);
                        let _ = shutdown_done.send(report);
                    }
                })
                .unwrap();
            let deadline = std::time::Instant::now() + Wall::from_secs(2);
            while core.tx_report_gates[0].queued() == 0 && spy.seen("tx_async") == 1 && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            let queued_at_release = core.tx_report_gates[0].queued();
            let reads_at_release = spy.seen("tx_async");

            gate.release();
            opener.join().unwrap();
            shutdown.join().unwrap();
            let flush = flush_result.recv().unwrap();
            let shutdown_got = shutdown_report.recv().unwrap();
            assert!(flush.acknowledged && !flush.stopped, "the opener's flush was not acknowledged (reopen: {reopen})");
            assert!(shutdown_got.is_none(), "shutdown's reader took a report from the opener's flush (reopen: {reopen})");
            let flush_acks = lock(&core.rec).device_async.iter().filter(|row| row["code"] == "BurstAck" && row["channel"] == 0 && row["tx_flush"] == true).count();
            assert_eq!(flush_acks, 1, "the flush's BurstAck is recorded once (reopen: {reopen})");
            assert!(lock(&core.rec).rejected.iter().all(|row| row["call"] != "tx_flush"), "no BurstAck was missing (reopen: {reopen})");
            let order = spy.order();
            let send = order.iter().position(|call| *call == "tx_send").unwrap();
            let ack = order.iter().position(|call| *call == "tx_async").unwrap();
            assert!(send < ack, "flush -> ack read: {order:?} (reopen: {reopen})");
            assert_eq!(spy.log().iter().filter(|call| call.starts_with("tx_send")).count(), 1);
            assert_eq!(spy.seen("tx_async"), 2, "shutdown owns the next read after the flush (reopen: {reopen})");
            assert_eq!((queued_at_release, reads_at_release), (1, 1), "shutdown was not queued on the report lock behind the opener (reopen: {reopen})");
        }
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
    fn ur_22_a_flush_underflows_are_recorded_as_the_flushs() {
        // UR-28 (T3): an Underflow of an unacknowledged flush channel, read before the flush ends, is the flush's; after its channel's ACK it is a Run report.
        use crate::device::{FakeConfig, TxCode, TxReport};

        let inner = Arc::new(crate::device::FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let feed = Arc::new(super::super::test_support::ReportFeed::new(inner));
        let (core, _time, _events) = super::super::test_support::rig_on(feed.clone(), Vec::new(), 2_000);
        feed.tx_open(0, &[0, 1]).unwrap();
        let report = |code, channel| TxReport { code, tick: Some(0), channel };
        feed.give(report(TxCode::Underflow, 0));
        feed.give(report(TxCode::TimeError, 0));
        feed.give(report(TxCode::BurstAck, 1));
        feed.give(report(TxCode::Underflow, 1));
        feed.give(report(TxCode::BurstAck, 0));
        let result = core.tx_flush(0, &[0, 1]);
        assert!(result.acknowledged, "{:?}", lock(&core.rec).device_async);
        let flush_rows: Vec<_> = lock(&core.rec).device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| (row["code"].as_str().unwrap().to_owned(), row["channel"].as_u64())).collect();
        assert_eq!(flush_rows, [("Underflow".to_owned(), Some(0)), ("TimeError".to_owned(), Some(0)), ("BurstAck".to_owned(), Some(1)), ("BurstAck".to_owned(), Some(0))]);
        assert_eq!(core.take_stashed_tx_report(0).map(|report| (report.code, report.channel)), Some((TxCode::Underflow, 1)), "the Underflow after channel 1's ACK is a Run report");
        assert!(core.take_stashed_tx_report(0).is_none());
    }

    #[test]
    fn ur_22_a_recovery_reopen_flush_records_its_underflows_as_its_own() {
        // UR-28 (T3, recovery): the reopen that replaces an uncertain streamer runs the same flush attribution as the open flush.
        use crate::device::{FakeConfig, TxCode, TxReport};

        let inner = Arc::new(crate::device::FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let feed = Arc::new(super::super::test_support::ReportFeed::new(inner));
        let (core, _time, _events) = super::super::test_support::rig_on(feed.clone(), Vec::new(), 2_000);
        feed.tx_open(0, &[0, 1]).unwrap();
        let report = |code, channel| TxReport { code, tick: Some(0), channel };
        feed.give(report(TxCode::Underflow, 0));
        feed.give(report(TxCode::TimeError, 0));
        feed.give(report(TxCode::BurstAck, 1));
        feed.give(report(TxCode::Underflow, 1));
        feed.give(report(TxCode::BurstAck, 0));
        let result = core.reopen_tx_streamer(0, Channels::first(2)).unwrap();
        assert!(result.acknowledged, "{:?}", lock(&core.rec).device_async);
        let flush_rows: Vec<_> = lock(&core.rec).device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| (row["code"].as_str().unwrap().to_owned(), row["channel"].as_u64())).collect();
        assert_eq!(flush_rows, [("Underflow".to_owned(), Some(0)), ("TimeError".to_owned(), Some(0)), ("BurstAck".to_owned(), Some(1)), ("BurstAck".to_owned(), Some(0))]);
        assert_eq!(core.take_stashed_tx_report(0).map(|report| (report.code, report.channel)), Some((TxCode::Underflow, 1)), "the Underflow after channel 1's ACK is a Run report");
    }

    #[test]
    fn ur_22_a_flush_is_four_zeros_and_draws_no_underflow() {
        // UR-22 (F1-4): the flush is one untimed SOB+EOB of four zeros per channel; its ACK is a flush row, and no Underflow is drawn.
        use crate::device::FakeConfig;

        let inner = Arc::new(crate::device::FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let (core, _time, events) = super::super::test_support::rig_on(inner.clone(), Vec::new(), 2_000);
        inner.tx_open(0, &[0]).unwrap();
        let result = core.tx_flush(0, &[0]);
        assert!(result.acknowledged, "{:?}", lock(&core.rec).device_async);
        let sends: Vec<_> = inner.calls().into_iter().filter(|call| call.starts_with("tx_send")).collect();
        assert_eq!(sends, ["tx_send 0 n=4 at=- sob=true eob=true"], "{:?}", inner.calls());
        let events = events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        assert!(events.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TX_UNDERFLOW), "{events:?}");
        let rec = lock(&core.rec);
        assert!(rec.device_async.iter().all(|row| row["code"] != "Underflow"), "{:?}", rec.device_async);
        assert_eq!(rec.device_async.iter().filter(|row| row["tx_flush"] == true && row["code"] == "BurstAck").count(), 1, "{:?}", rec.device_async);
    }

    #[test]
    fn ur_22_t3_flush_attribution_is_per_channel_and_code_bounded() {
        // UR-28 (T3): a duplicate ACK does not complete another channel, and a report of another code is never the flush's.
        use crate::device::{FakeConfig, TxCode, TxReport};

        let inner = Arc::new(crate::device::FakeDevice::new(FakeConfig { clock: crate::ClockMode::Manual, ..FakeConfig::default() }));
        let feed = Arc::new(super::super::test_support::ReportFeed::new(inner));
        let (core, _time, _events) = super::super::test_support::rig_on(feed.clone(), Vec::new(), 2_000);
        feed.tx_open(0, &[0, 1]).unwrap();
        let report = |code, channel| TxReport { code, tick: Some(0), channel };
        feed.give(report(TxCode::BurstAck, 0));
        feed.give(report(TxCode::BurstAck, 0));
        feed.give(report(TxCode::Other(77), 1));
        feed.give(report(TxCode::BurstAck, 1));
        let result = core.tx_flush(0, &[0, 1]);
        assert!(result.acknowledged, "{:?}", lock(&core.rec).device_async);
        let flush_rows: Vec<_> = lock(&core.rec).device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| (row["code"].as_str().unwrap().to_owned(), row["channel"].as_u64())).collect();
        assert_eq!(flush_rows, [("BurstAck".to_owned(), Some(0)), ("BurstAck".to_owned(), Some(1))]);
        let stashed: Vec<_> = std::iter::from_fn(|| core.take_stashed_tx_report(0)).map(|report| (report.code, report.channel)).collect();
        assert_eq!(stashed, [(TxCode::BurstAck, 0), (TxCode::Other(77), 1)]);
    }

    #[test]
    fn ur_22_t3_flush_reports_after_the_deadline_stay_the_runs() {
        // UR-28 (T3): an Underflow read after the flush's deadline is not the flush's; it is stashed for the Run.
        use crate::device::{FakeConfig, FakeFault, TxCode};
        use crate::provider::test_support::LockSpy;
        use std::sync::Arc;

        let faults = vec![FakeFault::DropBurstAck, FakeFault::TxReport(Wall::ZERO, TxCode::Underflow)];
        let spy = Arc::new(LockSpy::new(FakeConfig { clock: crate::ClockMode::Real, faults, ..FakeConfig::default() }));
        let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
        spy.watch(&core);
        let gate = spy.stop_after("tx_async");
        let (done, finished) = std::sync::mpsc::channel();
        let opener = core
            .device
            .host()
            .spawn("late-tx-flush-underflow", {
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
        assert!(!flush.acknowledged);
        assert!(core.take_stashed_tx_report(0).is_some_and(|report| report.code == TxCode::Underflow), "the late Underflow remains for the Run");
        assert!(lock(&core.rec).device_async.iter().all(|row| row["tx_flush"] != true), "a late report is not recorded as the flush's: {:?}", lock(&core.rec).device_async);
    }

    #[test]
    fn ur_22_t3_loss_during_flush_records_its_reports() {
        // UR-28, UR-29 (T3): an Underflow read before a device loss during the flush wait is the flush's; the loss is one DEVICE_LOST and no TX_UNDERFLOW.
        use crate::device::{FakeConfig, FakeFault, TxCode};

        let config = FakeConfig {
            clock: crate::ClockMode::Manual,
            faults: vec![FakeFault::DropBurstAck, FakeFault::TxReport(Wall::ZERO, TxCode::Underflow), FakeFault::TxAsyncFails(Wall::from_millis(50), true)],
            ..FakeConfig::default()
        };
        let (core, device, _time, events) = super::super::test_support::rig_with(config, Vec::new());
        device.tx_open(0, &[0]).unwrap();
        let result = core.tx_flush(0, &[0]);
        assert!(result.stopped && core.is_lost());
        let flush_rows: Vec<_> = lock(&core.rec).device_async.iter().filter(|row| row["tx_flush"] == true).map(|row| row["code"].as_str().unwrap().to_owned()).collect();
        assert_eq!(flush_rows, ["Underflow"], "{:?}", lock(&core.rec).device_async);
        let emitted = events.drain(core.at(core.now()));
        assert_eq!(emitted.iter().filter(|event| event.kind.as_str() == EventKind::DEVICE_LOST).count(), 1, "{emitted:?}");
        assert!(emitted.iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::TX_UNDERFLOW), "{emitted:?}");
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

    fn cfg(channels: u32) -> Config {
        Config { channels: Channels::first(channels), ratio: Rational::new(200, 1).unwrap() }
    }

    fn line(direction: Direction) -> Line {
        Line::new(timeline::Stream { direction, origin: 0, config: cfg(1), lead: 0, call: 2_000, allowance: 0 })
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

    fn active_rx_chain(core: &Core, k: usize) {
        core.rx_chain[k].lock().adopt_start(SegmentKey { by: Some(1), origin: 0 }, 0);
    }

    #[test]
    fn rx_chain_reuse_guard_rejects_open_and_close_of_an_active_chain() {
        let (core, device) = two_by_two_with_device();
        active_rx_chain(&core, 0);
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = core.open_device(Dir::Rx, 0, Channels::first(1));
        }));
        assert!(opened.is_err(), "an Armed chain cannot be reopened");
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_open")), "the guard runs before the device call: {:?}", device.calls());

        let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.close_device(Dir::Rx, 0)));
        assert!(closed.is_err(), "an Armed chain cannot be closed");
        assert!(!device.calls().iter().any(|call| call.starts_with("rx_close")), "the guard runs before the device call: {:?}", device.calls());
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
    fn ur_36_the_owner_step_follows_the_record() {
        // UR-36: a configure when the record has the set open; an open when nothing is open or the set is not the open one.
        let mut streams = Streams::with_streams(2, 2);
        let (k, set) = (stream_key(Dir::Rx, 0), Channels::first(1));
        assert_eq!(streams.owner_step(k, set), Owner::Open, "nothing is open");
        streams.record_open(k, Some(set));
        assert_eq!(streams.owner_step(k, set), Owner::Configure, "open");
        assert_eq!(streams.owner_step(k, Channels::first(2)), Owner::Open, "another set");
    }

    #[test]
    fn ur_26_an_end_takes_delivered_from_each_segment_s_own_stream() {
        // UR-26, RM-16 (S12b): an end cuts each receive line at its own first sample not
        // delivered; an Orderly end floors it at its own instant, per line.
        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            for (stream, k) in [(stream_key(Dir::Rx, 0), 1_000), (stream_key(Dir::Rx, 1), 500)] {
                streams.started(stream, 0, cfg(1));
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
                streams.started(stream, 0, cfg(1));
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

    /// The two-by-two rig with transmit stream 0's clock registered at root tick `origin`, as `arm`
    /// does, and its line planned from there.
    fn tx_clock_at(origin: i64) -> (Arc<Core>, Clock) {
        let core = two_by_two();
        let clock = core.register_in(Dir::Tx, 0, 200, origin).unwrap();
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Tx, 0)).clock = Some(clock);
            streams.lines.insert(stream_key(Dir::Tx, 0), Line::new(timeline::Stream { direction: Direction::Tx, origin, config: cfg(1), lead: 0, call: 2_000, allowance: 0 }));
        }
        (core, clock)
    }

    /// The `ended_at` root tick of the transmit clock `clock`'s registry record.
    fn tx_clock_end(core: &Core, clock: Clock) -> Option<i64> {
        let record = core.clocks.sample_clock_records().into_iter().find(|record| record.domain == clock.domain).expect("the clock is registered");
        record.ended_at.map(|end| end.ticks_in(record.root).unwrap())
    }

    #[test]
    fn ur_23_a_transmit_clock_cut_at_its_origin_ends_at_the_booking() {
        // RM-25, VH-2: a Provider::stop at or before the clock's origin (S <= O) cuts the
        // segment at sample 0 and ends the clock at O; after O the plan has no cut and the
        // clock stays open. A booking after the end does not end it again.
        const O: i64 = 10_000;
        for (stop, ended) in [(O - 4_000, Some(O)), (O, Some(O)), (O + 400, None)] {
            let (core, clock) = tx_clock_at(O);
            lock(&core.streams).end(&core, stop, false, false);
            assert_eq!(lock(&core.streams).planned(stream_key(Dir::Tx, 0))[0].segment.cut, ended.map(|_| 0), "S {stop}");
            assert_eq!(tx_clock_end(&core, clock), ended, "S {stop}");
            let mut streams = lock(&core.streams);
            streams.changed(&core, stream_key(Dir::Tx, 0), None);
            assert!(lock(&core.rec).rejected.is_empty(), "S {stop}: {:?}", lock(&core.rec).rejected);
        }
        // A loss: the same end through the thread that finds it, which bypasses the owner's work.
        let (core, clock) = tx_clock_at(O);
        core.device_lost("scripted loss");
        assert_eq!(tx_clock_end(&core, clock), Some(O));
        assert!(lock(&core.rec).rejected.is_empty(), "{:?}", lock(&core.rec).rejected);
    }

    // ------------------------------------------------------------ 1b-6a: the holder and the read

    #[test]
    fn ur_25_a_claim_owner_takes_only_a_free_stream() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        assert_eq!(streams.holder(k), Holder::Free);
        let claim = streams.claim_owner(k).expect("a free stream");
        assert_eq!(streams.holder(k), Holder::Owner);
        assert!(streams.claim_owner(k).is_none());
        assert_eq!(streams.holder(k), Holder::Owner);
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
        let claim = streams.claim_owner(k).expect("free again");
        assert_eq!(streams.holder(k), Holder::Owner);
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
    }

    // ------------------------------------------------------------ 1b-6b: the owner's delivered floor

    #[test]
    fn ur_26_a_finished_stream_clears_only_its_own_floor() {
        // UR-26, RM-16: finishing stream 1 clears its own floor and leaves stream 0's; an
        // abort's end after both delivered floors cuts stream 0 at its own floor and stream 1,
        // no longer running, at the end's instant instead.
        let core = two_by_two();
        let (rx0, rx1) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        {
            let mut streams = lock(&core.streams);
            for (stream, k) in [(rx0, 3_000), (rx1, 500)] {
                streams.started(stream, 0, cfg(1));
                streams.delivered(stream, 0, cfg(1), k);
            }
            streams.finished(rx1);
            assert_eq!(streams.first_undelivered(rx0), Some((0, cfg(1), 3_000)), "stream 0's floor stands");
            assert_eq!(streams.first_undelivered(rx1), None, "stream 1's floor is cleared");
            streams.end(&core, 800_000, true, false);
        }
        let streams = lock(&core.streams);
        assert_eq!(streams.planned(rx0)[0].segment.cut, Some(3_000), "stream 0's own delivered floor");
        assert_eq!(streams.planned(rx1)[0].segment.cut, Some(4_000), "stream 1 is cut at the end's instant, not its old floor");
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

        let claim = streams.claim_owner(k).expect("free");
        assert!(!streams.begin_read(k));
        assert!(!streams.reading(k));
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

        let claim = streams.claim_owner(k).expect("free");
        streams.begin_shutdown_read(k);
        assert!(streams.reading(k));
        assert_eq!(streams.holder(k), Holder::Owner);
        streams.end_read(k);
        streams.give_back(claim);

        let claim = streams.claim_owner(k).expect("free");
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
    #[should_panic(expected = "UR-25: a claim of Owner was not given back")]
    fn ur_25_a_claim_dropped_without_give_back_panics_in_the_test_build() {
        let mut streams = Streams::with_streams(1, 1);
        let claim = streams.claim_owner(stream_key(Dir::Rx, 0)).expect("free");
        drop(claim);
    }

    #[test]
    fn ur_25_a_give_back_of_another_holder_panics_in_the_test_build_and_changes_nothing() {
        let mut streams = Streams::with_streams(1, 1);
        let k = stream_key(Dir::Rx, 0);
        let claim = streams.claim_owner(k).expect("free");
        let wrong = Claim { stream: k, holder: Holder::Free };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            streams.give_back(wrong);
        }));
        assert!(caught.is_err(), "a claim the stream does not hold is refused");
        assert_eq!(streams.holder(k), Holder::Owner, "the real holder is untouched");
        streams.give_back(claim);
        assert_eq!(streams.holder(k), Holder::Free);
    }
}
