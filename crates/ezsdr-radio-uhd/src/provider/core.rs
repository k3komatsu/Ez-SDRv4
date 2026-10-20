//! What the Provider's three threads share (UR-14…UR-30).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_kernel::event::{ActionId, Event, EventHandle, EventKind, EventSink, EventSource, Severity};
use ezsdr_kernel::id::{ClockDomainId, ResourceId};
use ezsdr_kernel::module_api::{InputStore, ModuleError, StopMode};
use ezsdr_kernel::spec::{Key, Scalar, Value};
use ezsdr_kernel::stream::{DataLink, Direction};
use ezsdr_kernel::time::{ClockRegistry, Rational, TimeAuthority, TimePoint};
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, StreamKey, NO_RELEASE};
use ezsdr_radio::{keys, kinds};
use serde_json::{Value as Json, json};

use super::control::{ColdConfig, Planned};
use super::mode::{channel_list, Mode};
use crate::device::{Device, DeviceError, Dir, Settings, decimation};

/// A guard of the instance's state (UR-25): a `MutexGuard`, except that the test build counts
/// how many `core::lock` guards this thread holds — `streams`, `rec`, `held` and `rates` all
/// count — so that [`assert_line_free`] sees a lock order a `try_lock` cannot: another
/// thread's line is not this caller's.
pub(crate) struct Locked<'a, T> {
    guard: MutexGuard<'a, T>,
}

impl<T> std::ops::Deref for Locked<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { &self.guard }
}

impl<T> std::ops::DerefMut for Locked<'_, T> {
    fn deref_mut(&mut self) -> &mut T { &mut self.guard }
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

impl Clock {
    /// The root tick of sample `k`.
    pub fn instant(&self, k: i64) -> i64 {
        i64::try_from(i128::from(self.origin) + i128::from(k) * i128::from(self.n))
            .expect("a representable stream instant")
    }

    /// The first sample at or after root tick `t`.
    pub fn at_or_after(&self, t: i64) -> i64 {
        i64::try_from((i128::from(t) - i128::from(self.origin) + i128::from(self.n) - 1)
            .div_euclid(i128::from(self.n))).expect("a representable sample index")
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
    /// The instance's lines, one per stream, keyed by `(direction, k)` (RM-26, RM-29,
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
    /// Counts the changes of the plans, for uhd-tx to read its own again.
    pub version: u64,
}

impl Default for Streams {
    /// One stream per direction: the current keys' shape (design 33 D4).
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
    /// uhd-control's enable from no stream owns the stream's streamer from the section that
    /// decides it until the section that books the change or ends the enable without one: an
    /// owner that finds it set makes no device call and moves neither its cursor nor its idle
    /// mark (UR-25).
    pub busy: bool,
    /// uhd-tx's report read is in flight: an enable from no stream waits for it before its
    /// first device call (UR-25).
    pub reading: bool,
    /// uhd-rx's segment, as its origin and configuration, and the first sample of it not yet
    /// delivered: RM-16's floor, which reaches the plan as `Item.delivered` (UR-17).
    pub delivered: Option<(i64, Config, i64)>,
    /// The owner has no stream and nothing planned to begin (UR-25).
    pub idle: bool,
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

/// The stream a command under the current keys names (RM-29): stream 0 of its direction.
/// Under the layout the code names its stream with [`stream_key`].
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

    /// How many streams of `dir` the instance holds (RM-29).
    pub fn count(&self, dir: Dir) -> usize {
        match dir {
            Dir::Rx => self.rx.len(),
            Dir::Tx => self.tx.len(),
        }
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
        let delivered = self.stream(stream).delivered;
        let Some(line) = self.lines.line(stream) else { return false };
        if stream.0 == Direction::Rx && matches!(item.kind, Kind::End { .. }) {
            let cuts = |origin, config| line.made(origin, config, item.e, item.seq).ok().flatten().is_some_and(|s| s.cut.is_none());
            item.delivered = item.delivered.or(delivered.filter(|(origin, config, _)| cuts(*origin, *config)).map(|(_, _, k)| k));
        }
        if let Err(error) = self.lines.book(stream, item) {
            core.command_rejected(None, "update_parameter", &format!("RM-26: {error}"));
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
        match self.lines.refuse(stream, seq) {
            Ok(refused) => {
                self.changed(core, stream);
                refused
            }
            Err(error) => {
                core.command_rejected(None, "update_parameter", &format!("RM-26: {error}"));
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
        line.plan.iter().enumerate().map(|(index, segment)| {
            Planned {
                segment: *segment,
                k: stream.1 as usize,
                settings: segment.by.and_then(|by| state.configs.get(&by)).cloned(),
                refused: segment.by.is_some_and(|by| line.items.iter().any(|item| item.seq == by && item.refused)),
                clock: (stream.0 == Direction::Tx).then(|| state.clocks.get(index).map(|(clock, _)| *clock)).flatten(),
            }
        }).collect()
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
    fn changed(&mut self, core: &Core, stream: StreamKey) {
        self.version += 1;
        if stream.0 == Direction::Tx {
            if let Err(error) = self.reconcile(core, stream) {
                core.command_rejected(None, "update_parameter", &format!("UR-25: {error}"));
            }
        }
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

    /// UR-30: the plans the streams were carried out by, each segment's origin and end.
    pub fn record(&self) -> Json {
        let plan = |dir: Dir| -> Vec<Json> {
            let mut rows = Vec::new();
            for k in 0..self.count(dir) {
                let Some(line) = self.lines.line(stream_key(dir, k)) else { continue };
                rows.extend(line.plan.iter().map(|segment| json!({
                    "origin": segment.origin,
                    "end": segment.cut.and_then(|cut| segment.instant(cut).ok()),
                    "channels": segment.config.channels.count(),
                    "ticks_per_sample": [segment.config.ratio.num(), segment.config.ratio.den()],
                })));
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
        let stats = [
            "rx_blocks",
            "rx_samples",
            "rx_overflows",
            "rx_errors",
            "rx_off_lattice",
            "rx_overlapping",
            "rx_before_origin",
            "link_drops_seen",
            "tx_bursts",
            "tx_samples",
            "tx_errors",
        ]
        .into_iter()
        .map(|name| (name, 0))
        .collect();
        Records {
            applied: Vec::new(),
            bursts: Vec::new(),
            device_async: Vec::new(),
            rejected: Vec::new(),
            timing: Vec::new(),
            stats,
        }
    }
}

/// The context `prepare` gathered, shared by uhd-control, uhd-rx and uhd-tx.
pub(crate) struct Core {
    pub device: Arc<dyn Device>,
    pub id: ResourceId,
    /// Stream 0's node of each direction (RM-29): `<id>/rx` and `<id>/tx` under the
    /// current keys, `<id>/rx/s0` and `<id>/tx/s0` under the layout. Every other stream
    /// names its own with [`Core::stream_node`].
    pub rx_id: ResourceId,
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
    /// The key set this instance presents (RM-26, RM-27).
    pub mode: Mode,
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
        mode: Mode,
    ) -> Result<Core, ModuleError> {
        let rx_id = mode.stream_node(&id, Dir::Rx, 0);
        let tx_id = mode.stream_node(&id, Dir::Tx, 0);
        let rx_count = mode.stream_count(Dir::Rx);
        let tx_count = mode.stream_count(Dir::Tx);
        links.resize_with(rx_count, Vec::new);
        // UR-12, UR-20: every receive stream's node is below the instance root the Kernel
        // handed this instance, so the bound view resolves it; a refusal is a Module error,
        // not a Kernel event (item 3).
        let mut overflow = Vec::with_capacity(rx_count);
        for k in 0..rx_count {
            let node = mode.stream_node(&id, Dir::Rx, k);
            overflow.push(
                events
                    .resolve(&EventSource::Node { node }, &EventKind::parse(kinds::RX_OVERFLOW).expect("a radio kind"))
                    .map_err(|error| ModuleError::rejected(format!("UR-12: the rx stream's event handle: {error}")))?,
            );
        }
        Ok(Core {
            mcr: device.master_clock_rate(),
            block_len: mode.block_len() as usize,
            device,
            id,
            rx_id,
            tx_id,
            root,
            time,
            clocks,
            events,
            overflow,
            inputs,
            links,
            mode,
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
        let event = Event {
            source: EventSource::Node { node: source.clone() },
            time,
            severity,
            kind: EventKind::parse(kind).expect("a radio kind"),
            payload,
        };
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
        let payload = serde_json::to_value(CommandRejectedPayload { action: id, kind: kind.to_owned(), reason: reason.to_owned() })
            .expect("a payload");
        self.emit(&self.id, kinds::COMMAND_REJECTED, Severity::Error, payload);
    }

    /// MA-9a: `DEVICE_LOST`, once, from this Provider; then no thread uses the device. The
    /// thread that finds the loss books it on both streams, before any thread can see the
    /// device lost (RM-16).
    pub fn device_lost(&self, message: &str) {
        let mut streams = lock(&self.streams);
        if self.lost.load(Ordering::Acquire) {
            return;
        }
        streams.end(self, self.now(), false, true);
        self.lost.store(true, Ordering::Release);
        drop(streams);
        // Before anything can free it: a lost device is never freed (F4).
        self.device.mark_lost();
        self.timing(json!({ "what": "device_lost", "message": message, "at": self.now() }));
        self.emit(
            &self.id,
            EventKind::DEVICE_LOST,
            Severity::Fatal,
            json!({ "message": message }),
        );
    }

    /// A device failure: `DEVICE_LOST` when the device is gone, else the reason.
    pub fn device_failed(&self, id: Option<ActionId>, action: &str, error: &DeviceError) {
        if error.lost {
            self.device_lost(&error.message);
        } else {
            self.command_rejected(id, action, &error.message);
        }
    }

    /// A direction's full configuration from a configuration map (UR-12).
    pub fn settings(config: &BTreeMap<Key, Value>, dir: Dir) -> Settings {
        let number = |name: &str| config.get(&key(&format!("radio.{}.{name}", dir.name()))).and_then(Value::as_scalar).and_then(super::mode::scalar_number);
        Settings {
            rate: number("sample_rate_hz"),
            freq: number("frequency_hz"),
            gain: number("gain_db"),
            antenna: match config.get(&key(&format!("radio.{}.antenna", dir.name()))) {
                Some(Value::Scalar(Scalar::Str(s))) => Some(s.clone()),
                _ => None,
            },
        }
    }

    /// The node stream `k` of `dir` reports from (RM-29, UR-17, UR-20): `<id>/rx` and
    /// `<id>/tx` under the current keys, `<id>/rx/s<k>` and `<id>/tx/s<k>` under the layout.
    /// The one place a Provider reads a stream's event source from.
    pub fn stream_node(&self, dir: Dir, k: usize) -> ResourceId {
        self.mode.stream_node(&self.id, dir, k)
    }

    pub fn channels(config: &BTreeMap<Key, Value>, dir: Dir) -> usize {
        let name = match dir {
            Dir::Rx => keys::RX_CHANNELS,
            Dir::Tx => keys::TX_CHANNELS,
        };
        match config.get(&key(name)) {
            Some(Value::Scalar(Scalar::Int(n))) if *n > 0 => *n as usize,
            _ => 0,
        }
    }

    /// UR-13, design 33 D5: the rate set the device reports for one channel of `dir` is the
    /// profile's, compared by set once — after the direction's first streamer opens — and
    /// the outcome held for every later open. A mismatch is the refusal `UR-13:`.
    pub fn check_rates(&self, dir: Dir, chan: usize) -> Result<(), DeviceError> {
        self.probe("rates", None);
        {
            let checks = lock(&self.rates);
            if let Some(outcome) = &checks[dir as usize] {
                return outcome.clone().map_err(DeviceError::failed);
            }
        }
        // The device is read outside the lock: no device call is made under one (UR-25).
        // Two directions never share a slot and a repeated read of one answers the same
        // set, so the first outcome stored is the one every later open sees.
        let outcome = match self.device.rates(dir, chan) {
            Ok(ranges) if ezsdr_radio::device::rate_set_matches(self.mode.rates(), &ranges) => Ok(()),
            Ok(_) => Err(format!("UR-13: the {} rates the device reports are not the profile's rate set", dir.name())),
            Err(error) => Err(format!("UR-13: {}", error.message)),
        };
        lock(&self.rates)[dir as usize] = Some(outcome.clone());
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
            for (name, claimed, read) in [
                ("sample_rate_hz", Some(rate), applied.rate),
                ("frequency_hz", settings.freq, applied.freq),
                ("gain_db", settings.gain, applied.gain),
            ] {
                rec.applied.push(json!({
                    "key": self.mode.applied_key(dir, *chan, k, name), "channel": chan,
                    "claimed": claimed, "read_back": read, "difference": claimed.map(|c| read - c), "at": at,
                }));
            }
            if let Some(antenna) = &settings.antenna {
                if self.mode.is_layout() {
                    // design 33 D16: the read-back is recorded and compared; a front end
                    // whose getter disagrees with the claimed antenna is refused.
                    rec.applied.push(json!({
                        "key": self.mode.applied_key(dir, *chan, k, "antenna"), "channel": chan,
                        "claimed": antenna, "read_back": applied.antenna, "at": at,
                    }));
                } else {
                    rec.applied.push(json!({ "key": self.mode.applied_key(dir, *chan, k, "antenna"), "channel": chan, "claimed": antenna, "at": at }));
                }
            }
            drop(rec);
            if applied.rate != rate {
                return Err(DeviceError::failed(format!("UR-12: the device applied {} S/s for the claimed {rate}", applied.rate)));
            }
            if self.mode.is_layout() {
                if let (Some(antenna), read) = (settings.antenna.as_ref(), applied.antenna.as_deref()) {
                    if read != Some(antenna.as_str()) {
                        return Err(DeviceError::failed(format!(
                            "UR-12: the device reads back {} for the claimed {antenna} antenna of {}",
                            read.unwrap_or("no antenna"),
                            self.mode.applied_key(dir, *chan, k, "antenna")
                        )));
                    }
                }
            }
        }
        decimation(self.mcr, rate)
            .map(|n| n as i64)
            .ok_or_else(|| DeviceError::failed(format!("UR-12: {rate} S/s is no decimation of {} Hz", self.mcr)))
    }

    /// UR-13, UR-36: the device's open of `k`'s streamer of `dir` on `set`; the record is the
    /// caller's (see `record_opened`), so that an enable from no stream records its open in the
    /// line-lock section it already takes (UR-25).
    pub fn open_device(&self, dir: Dir, k: usize, set: Channels) -> Result<(), DeviceError> {
        self.probe("open", None);
        let list = channel_list(set);
        match dir {
            Dir::Rx => self.device.rx_open(k, &list),
            Dir::Tx => self.device.tx_open(k, &list),
        }
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
        let opened = self.open_device(dir, k, set);
        let checked = if opened.is_ok() { self.check_opened(dir, set) } else { Ok(()) };
        lock(&self.streams).record_opened(stream, set, &opened, &checked);
        opened.and(checked)
    }

    /// The owner's device calls for a segment it begins (UR-13, UR-36): the record decides what
    /// is opened and configured. Each device call is made with `streams` released, and its
    /// outcome is recorded after it; a successful open moves the record's generation on, which
    /// is how a thread holding correlations against the replaced streamer learns of it (UR-28).
    pub fn owner_configure(&self, dir: Dir, k: usize, next: &Planned) -> Result<(), DeviceError> {
        let stream = stream_key(dir, k);
        let set = next.set();
        let step = lock(&self.streams).owner_step(stream, set, next.segment.by);
        if step == Owner::Nothing {
            return Ok(());
        }
        if step == Owner::Open {
            self.open_streamer(dir, k, set)?;
        }
        match next.configure(self, dir) {
            Ok(_) => {
                // UR-36: the record's mark is published only once the configuration returned.
                self.probe("configured", None);
                lock(&self.streams).record_configured(stream, next.segment.by);
                Ok(())
            }
            Err(error) => {
                lock(&self.streams).record_configured(stream, None);
                Err(error)
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
        let stream = self.mode.stream_node(&self.id, dir, k);
        let ratio = Rational::new(n as u64, 1).map_err(|e| e.to_string())?;
        let handle = self.clocks.declare_sample_clock(stream, self.root, ratio).map_err(|e| e.to_string())?;
        let domain = self.clocks.register_sample_clock(&handle, origin).map_err(|e| e.to_string())?;
        Ok(Clock { domain, origin, n })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ezsdr_kernel::time::Rational;

    fn cfg(channels: u32) -> Config {
        Config { channels: Channels::first(channels), ratio: Rational::new(200, 1).unwrap() }
    }

    fn line(direction: Direction) -> Line {
        Line::new(timeline::Stream {
            direction,
            origin: 0,
            config: cfg(1),
            lead: 0,
            call: 2_000,
            allowance: 0,
        })
    }

    fn two_by_two() -> Arc<Core> {
        let (core, _, _, _) = super::super::test_support::rig();
        {
            let mut streams = lock(&core.streams);
            *streams = Streams::with_streams(2, 2);
            for dir in [Direction::Rx, Direction::Tx] {
                for k in 0..2 {
                    streams.lines.insert((dir, k), line(dir));
                }
            }
        }
        core
    }

    #[test]
    fn ur_26_an_end_takes_delivered_from_each_segment_s_own_stream() {
        // UR-26, RM-16 (S12b): an end cuts each receive line at its own first sample not
        // delivered; an Orderly end floors it at its own instant, per line.
        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Rx, 0)).delivered = Some((0, cfg(1), 1_000));
            streams.stream_mut(stream_key(Dir::Rx, 1)).delivered = Some((0, cfg(1), 500));
            streams.end(&core, 5_000, true, false);
        }
        let streams = lock(&core.streams);
        assert_eq!(streams.planned(stream_key(Dir::Rx, 0))[0].segment.cut, Some(1_000));
        assert_eq!(streams.planned(stream_key(Dir::Rx, 1))[0].segment.cut, Some(500));

        let core = two_by_two();
        {
            let mut streams = lock(&core.streams);
            streams.stream_mut(stream_key(Dir::Rx, 0)).delivered = Some((0, cfg(1), 3_000));
            streams.stream_mut(stream_key(Dir::Rx, 1)).delivered = Some((0, cfg(1), 500));
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

}
