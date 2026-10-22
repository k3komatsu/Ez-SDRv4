//! What the Provider's three threads share (UR-14…UR-30).

use std::collections::{BTreeMap, BTreeSet};
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
use ezsdr_radio::payloads::CommandRejectedPayload;
use ezsdr_radio::timeline::{self, Channels, Config, Item, Kind, Line, StreamKey, NO_RELEASE};
use ezsdr_radio::kinds;
use serde_json::{Value as Json, json};

use super::control::{ColdConfig, Planned};
use super::mode::{DescriptionExt, channel_list};
use crate::device::{Device, DeviceError, Dir, Settings, decimation};

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
    /// The segment this stream's owner is still processing, through its cut (A2).
    pub active: Option<(i64, Config, Option<u64>)>,
    /// Segments its owner finished processing; a segment with a finite cut remains needed
    /// until its owner records completion (A2).
    pub completed: Vec<(i64, Config, Option<u64>)>,
    /// uhd-control's enable from no stream owns the stream's streamer from the section that
    /// decides it until the section that books the change or ends the enable without one: an
    /// owner that finds it set makes no device call and moves neither its cursor nor its idle
    /// mark (UR-25).
    pub busy: bool,
    /// The owner configuration whose device outcome is still being recorded, if any (RM-34).
    pub busy_seq: Option<u64>,
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

    /// Another stream's recorded streamer owns any channel in `set` (D2, D3).
    pub fn held_elsewhere(&self, stream: StreamKey, set: Channels) -> Option<StreamKey> {
        (0..self.count(dir_of(stream))).map(|k| stream_key(dir_of(stream), k)).find(|other| {
            *other != stream && match self.stream(*other).set {
                DeviceSet::Open(open) => open.iter().any(|channel| set.contains(channel)),
                DeviceSet::Unknown => false,
            }
        })
    }

    /// An earlier, unprocessed segment owns the channel before `origin` (A2's finite chain).
    pub fn needed_before(&self, stream: StreamKey, set: Channels, origin: i64) -> bool {
        (0..self.count(dir_of(stream))).map(|k| stream_key(dir_of(stream), k)).any(|other| {
            if other == stream { return false; }
            let state = self.stream(other);
            self.planned(other).iter().any(|planned| {
                let identity = (planned.segment.origin, planned.segment.config, planned.segment.by);
                planned.segment.origin <= origin && !planned.refused && !state.completed.contains(&identity)
                    && !planned.segment.cut.is_some_and(|cut| planned.segment.instant(cut).is_ok_and(|end| end <= planned.segment.origin))
                    && planned.segment.config.channels.iter().any(|channel| set.contains(channel))
            })
        })
    }

    /// A move destination waits only for its source's configuration of that same move.
    pub fn move_source_busy(&self, stream: StreamKey, seq: Option<u64>) -> bool {
        let Some(seq) = seq else { return false };
        self.lines.move_lines(seq).is_some_and(|(from, to)| {
            to == stream && self.stream(from).busy && self.stream(from).busy_seq == Some(seq)
        })
    }

    /// Records a segment its owner finished carrying out.
    pub fn complete(&mut self, stream: StreamKey, segment: (i64, Config, Option<u64>)) {
        let completed = &mut self.stream_mut(stream).completed;
        if !completed.contains(&segment) { completed.push(segment); }
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
        let planned: Vec<_> = self.lines.line(stream).into_iter().flat_map(|line| &line.plan)
            .map(|segment| (segment.origin, segment.config, segment.by)).collect();
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
                rows.extend(line.plan.iter().map(|segment| json!({
                    "stream": k,
                    "sequence": segment.by,
                    "origin": segment.origin,
                    "end": segment.cut.and_then(|cut| segment.instant(cut).ok()),
                    "channels": segment.config.channels.iter().collect::<Vec<u32>>(),
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
    /// Serializes streamer open/close admission with publishing device loss (UR-29).
    device_gate: Mutex<()>,
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
            for (name, claimed, read) in [
                ("sample_rate_hz", Some(rate), applied.rate),
                ("frequency_hz", settings.freq, applied.freq),
                ("gain_db", settings.gain, applied.gain),
            ] {
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
        decimation(self.mcr, rate)
            .map(|n| n as i64)
            .ok_or_else(|| DeviceError::failed(format!("UR-12: {rate} S/s is no decimation of {} Hz", self.mcr)))
    }

    /// UR-13, UR-36: the device's open of `k`'s streamer of `dir` on `set`; the record is the
    /// caller's (see `record_opened`), so that an enable from no stream records its open in the
    /// line-lock section it already takes (UR-25).
    pub fn open_device(&self, dir: Dir, k: usize, set: Channels) -> Result<(), DeviceError> {
        self.probe("open", None);
        let device_gate = self.lock_device_gate();
        if self.is_lost() {
            return Err(DeviceError { lost: true, message: "UR-29: the device is lost before opening a streamer".to_owned() });
        }
        let list = channel_list(set);
        let opened = match dir {
            Dir::Rx => self.device.rx_open(k, &list),
            Dir::Tx => self.device.tx_open(k, &list),
        };
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
        let opened = self.open_device(dir, k, set);
        let checked = if opened.is_ok() { self.check_opened(dir, set) } else { Ok(()) };
        if opened.is_ok() && checked.is_err() && !self.is_lost() {
            self.close_device(dir, k);
        }
        lock(&self.streams).record_opened(stream, set, &opened, &checked);
        opened.and(checked)
    }

    pub fn close_device(&self, dir: Dir, k: usize) {
        let _device_gate = self.lock_device_gate();
        if self.is_lost() { return; }
        match dir {
            Dir::Rx => self.device.rx_close(k),
            Dir::Tx => self.device.tx_close(k),
        }
    }

    /// Releases an idle stream's streamer only when another line still has an unprocessed
    /// segment that needs one of its channels (D1, A2).
    pub fn release_unneeded(&self, dir: Dir, k: usize) -> bool {
        let stream = stream_key(dir, k);
        {
            let mut streams = lock(&self.streams);
            if self.is_lost() { return false; }
            let state = streams.stream(stream);
            if state.busy || !state.idle || state.active.is_some() { return false; }
            let DeviceSet::Open(set) = state.set else { return false };
            if !streams.needed_before(stream, set, i64::MAX) { return false; }
            let state = streams.stream_mut(stream);
            state.busy = true;
            state.busy_seq = None;
        }
        if self.is_lost() {
            let mut streams = lock(&self.streams);
            let state = streams.stream_mut(stream);
            state.busy = false;
            state.busy_seq = None;
            return false;
        }
        self.close_device(dir, k);
        let mut streams = lock(&self.streams);
        streams.record_open(stream, None);
        let state = streams.stream_mut(stream);
        state.busy = false;
        state.busy_seq = None;
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
            if self.is_lost() || streams.stop.is_some() { return None; }
            let current = streams.planned(stream).into_iter().find(|planned| planned.segment == expected.segment)?;
            if current.refused {
                return None;
            }
            if let Some(since) = waiting_since {
                if dir == Dir::Tx && streams.line(stream).is_some_and(|line| line.items.iter().any(|item| item.seq > since && item.kind == Kind::Stop)) {
                    return None;
                }
            }
            if streams.held_elsewhere(stream, current.set()).is_none()
                && !streams.needed_before(stream, current.set(), current.segment.origin)
                && !streams.move_source_busy(stream, current.segment.by)
            {
                return Some(current);
            }
            let first_wait = waiting_since.is_none();
            waiting_since.get_or_insert(streams.seq);
            drop(streams);
            if first_wait { self.probe("wait_release", None); }
            self.device.host().sleep(POLL);
        }
    }

    /// The owner's device calls for a segment it begins (UR-13, UR-36): the record decides what
    /// is opened and configured. Each device call is made with `streams` released, and its
    /// outcome is recorded after it; a successful open moves the record's generation on, which
    /// is how a thread holding correlations against the replaced streamer learns of it (UR-28).
    /// On error, the caller books the refusal before releasing the busy mark.
    pub fn owner_configure(&self, dir: Dir, k: usize, next: &Planned) -> Result<bool, DeviceError> {
        let stream = stream_key(dir, k);
        let (mut current, mut step, release_first) = {
            let mut streams = lock(&self.streams);
            if self.is_lost() { return Ok(false); }
            if streams.stream(stream).busy { return Ok(false); }
            let Some(current) = streams.planned(stream).into_iter().find(|planned| planned.segment == next.segment) else { return Ok(false) };
            if current.refused { return Ok(false); }
            let step = streams.owner_step(stream, current.set(), current.segment.by);
            if step == Owner::Nothing { return Ok(true); }
            let release_first = step == Owner::Open && (streams.held_elsewhere(stream, current.set()).is_some()
                || streams.needed_before(stream, current.set(), current.segment.origin)
                || streams.move_source_busy(stream, current.segment.by));
            let state = streams.stream_mut(stream);
            state.busy = true;
            state.busy_seq = current.segment.by;
            (current, step, release_first)
        };
        if release_first {
            if self.is_lost() {
                let mut streams = lock(&self.streams);
                let state = streams.stream_mut(stream);
                state.busy = false;
                state.busy_seq = None;
                return Ok(false);
            }
            let open = lock(&self.streams).stream(stream).set;
            if matches!(open, DeviceSet::Open(_)) {
                self.close_device(dir, k);
                lock(&self.streams).record_open(stream, None);
            }
            let Some(latest) = self.await_release(dir, k, &current) else {
                let mut streams = lock(&self.streams);
                let state = streams.stream_mut(stream);
                state.busy = false;
                state.busy_seq = None;
                return Ok(false);
            };
            current = latest;
            step = Owner::Open;
        }
        let opened = if step == Owner::Open { self.open_streamer(dir, k, current.set()) } else { Ok(()) };
        if let Err(error) = opened {
            let mut streams = lock(&self.streams);
            streams.record_configured(stream, None);
            return Err(error);
        }
        match current.configure(self, dir) {
            Ok(_) => {
                // UR-36: the record's mark is published only once the configuration returned.
                self.probe("configured", None);
                let mut streams = lock(&self.streams);
                streams.record_configured(stream, current.segment.by);
                let state = streams.stream_mut(stream);
                state.busy = false;
                state.busy_seq = None;
                Ok(true)
            }
            Err(error) => {
                let mut streams = lock(&self.streams);
                streams.record_configured(stream, None);
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
        let stream = self.description.stream_node(&self.id, dir, k);
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
        let rx_line = |channels| Line::new(timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
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

    #[test]
    fn ur_34_a_device_loss_waits_for_an_admitted_streamer_open_and_close() {
        for (call, opening) in [("rx_open", true), ("rx_close", false)] {
            let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
                clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default()
            }));
            let (core, _time, _events) = super::super::test_support::rig_on(spy.clone(), Vec::new(), 2_000);
            spy.watch(&core);
            let gate = spy.stop_before(call);
            let call_core = core.clone();
            let call_thread = core.device.host().spawn("streamer-lifecycle", move || {
                if opening {
                    call_core.open_device(Dir::Rx, 0, Channels::first(1)).unwrap();
                } else {
                    call_core.close_device(Dir::Rx, 0);
                }
            }).unwrap();
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
            let call = log.iter().position(|entry| entry == if opening { "rx_open 0 [0]" } else { "rx_close 0" })
                .unwrap_or_else(|| panic!("{call}: {log:?}"));
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
        let spy = Arc::new(super::super::test_support::LockSpy::new(crate::device::FakeConfig {
            clock: crate::ClockMode::Manual, ..crate::device::FakeConfig::default()
        }));
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
        let reader = core.device.host().spawn("rate-read", move || {
            let _ = done.send(worker.check_rates(Dir::Rx, 0));
        }).unwrap();
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
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
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
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-finite-chain", move || {
            let _ = done.send(worker.await_release(Dir::Rx, c.1 as usize, &destination));
        }).unwrap();
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
        {
            let mut streams = lock(&core.streams);
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            let source = streams.stream_mut(from);
            source.set = DeviceSet::Unknown;
            source.busy = true;
            source.busy_seq = Some(seq);
        }
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-move-source", move || {
            let _ = done.send(worker.await_release(Dir::Rx, to.1 as usize, &next));
        }).unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination reached its source wait");
        assert!(!device.calls().iter().any(|call| call == "rx_open 0 [0, 1]"), "it did not open during the source configure: {:?}", device.calls());
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, from, seq), "the source's refusal refuses the move");
            let source = streams.stream_mut(from);
            source.busy = false;
            source.busy_seq = None;
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
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
        device.rx_open(0, &[0]).unwrap();
        let next = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(2)));
            streams.lines.insert(to, line(Channels::first(0)));
            streams.lines.book_move(from, to, 1, Some(10_000), 0, 1, 0).unwrap();
            streams.changed(&core, from);
            streams.changed(&core, to);
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            let source = streams.stream_mut(from);
            source.set = DeviceSet::Open(Channels::first(1));
            source.busy = true;
            source.busy_seq = Some(1);
            streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(1)).unwrap()
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-move-destination", move || {
            let _ = done.send(worker.owner_configure(Dir::Rx, to.1 as usize, &next));
        }).unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination waits when the source released the moved channel but still configures the move");
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [1]"), "the destination stayed unopened: {:?}", device.calls());
        {
            let mut streams = lock(&core.streams);
            assert!(streams.refuse(&core, from, 1), "source refusal records the move on both lines");
            let source = streams.stream_mut(from);
            source.busy = false;
            source.busy_seq = None;
        }
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)), "the refused destination segment was interrupted");
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [1]"), "the refused destination never opened: {:?}", device.calls());
    }

    #[test]
    fn ur_34_a_release_skips_a_stream_held_busy_by_from_none() {
        let (core, device) = two_by_two_with_device();
        let stream = stream_key(Dir::Rx, 0);
        {
            let mut streams = lock(&core.streams);
            let state = streams.stream_mut(stream);
            state.set = DeviceSet::Open(Channels::first(1));
            state.busy = true;
            state.idle = true;
        }
        assert!(!core.release_unneeded(Dir::Rx, 0), "a from-none owner still holds the stream");
        assert!(!device.calls().iter().any(|call| call == "rx_close 0"), "the owner's close overlapped from-none: {:?}", device.calls());
        assert!(lock(&core.streams).stream(stream).busy);
    }

    #[test]
    fn ur_34_a_release_closes_an_idle_streamer_needed_by_a_move() {
        let (core, device, _next, from, _to, _seq) = waiting_move();
        lock(&core.streams).stream_mut(from).idle = true;
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
            state.idle = true;
        }
        assert!(!core.release_unneeded(Dir::Rx, 0));
        assert!(!device.calls().iter().any(|call| call == "rx_close 0"), "{:?}", device.calls());
        assert_eq!(lock(&core.streams).stream(stream).set, DeviceSet::Open(Channels::first(0).with(1)));
    }

    #[test]
    fn ur_34_a_waited_open_reloads_a_move_refused_while_it_waits() {
        let (core, device, next, from, to, seq) = waiting_move();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-test", move || { let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next)); }).unwrap();
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
        assert!(!lock(&core.streams).stream(to).busy, "the interrupted owner released its busy mark");
    }

    #[test]
    fn ur_34_a_owner_does_not_open_a_move_plan_removed_before_its_turn() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Rx, 0), stream_key(Dir::Rx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
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
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Rx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 0,
            call: 0,
            allowance: 0,
        });
        device.rx_open(0, &[0]).unwrap();
        let next = {
            let mut streams = lock(&core.streams);
            streams.lines.insert(from, line(Channels::first(1)));
            streams.lines.insert(to, line(Channels::first(0)));
            let seq = streams.next_seq();
            assert!(streams.book(&core, to, Item {
                e: 10_000, seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false,
                kind: Kind::Cold(Config { channels: Channels::first(1), ratio }),
            }));
            streams.stream_mut(from).set = DeviceSet::Open(Channels::first(1));
            let initial = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).unwrap();
            streams.complete(from, (initial.segment.origin, initial.segment.config, initial.segment.by));
            streams.stream_mut(to).configs.insert(seq, Arc::new(ColdConfig::new(BTreeMap::new())));
            streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(seq)).unwrap()
        };
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let expected = next.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-replan", move || {
            let _ = done.send(worker.owner_configure(Dir::Rx, to.1 as usize, &expected));
        }).unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the destination reaches its source wait");
        let changed_by = {
            let mut streams = lock(&core.streams);
            let seq = streams.next_seq();
            assert!(streams.book(&core, to, Item {
                e: next.segment.origin + 5_000,
                seq,
                ready: core.now(),
                release: NO_RELEASE,
                delivered: None,
                refused: false,
                kind: Kind::Cold(Config {
                    channels: next.segment.config.channels,
                    ratio: Rational::new(100, 1).unwrap(),
                }),
            }), "the destination plan changes without a refusal");
            let new = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(seq)).expect("the accepted replacement segment");
            assert!(new.segment.origin > next.segment.origin && new.segment.config.ratio == Rational::new(100, 1).unwrap() && !new.refused);
            streams.record_open(from, None);
            seq
        };
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)));
        assert!(!device.calls().iter().any(|call| call == "rx_open 1 [0]"), "the stale segment did not open: {:?}", device.calls());
        assert!(!lock(&core.streams).stream(to).busy, "the interrupted owner gave back its busy mark after replan {changed_by}");
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
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-loss-wait", move || { let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next)); }).unwrap();
        assert!(reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the owner reached its wait");
        core.device_lost("test: lost while waiting for a streamer");
        owner.join().unwrap();
        assert!(matches!(result_rx.recv_timeout(Wall::from_secs(10)).unwrap(), Ok(false)));
        let log = device.calls();
        let marked = log.iter().position(|call| call == "mark_lost").unwrap();
        assert!(!log[marked + 1..].iter().any(|call| call.starts_with("rx_close") || call.starts_with("rx_open")), "a lost device is never closed or reopened: {log:?}");
        assert!(!lock(&core.streams).stream(stream_key(Dir::Rx, 0)).busy);
    }

    #[test]
    fn ur_34_a_waited_open_ends_at_provider_stop_without_opening() {
        let (core, device, next, _from, _to, _seq) = waiting_move();
        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("rx-stop-wait", move || { let _ = done.send(worker.owner_configure(Dir::Rx, 0, &next)); }).unwrap();
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
        assert!(!lock(&core.streams).stream(stream_key(Dir::Rx, 0)).busy);
    }

    #[test]
    fn ur_34_a_waited_transmit_open_observes_provider_stop_with_a_later_cold_reserved() {
        let (core, device) = two_by_two_with_device();
        let (from, to) = (stream_key(Dir::Tx, 1), stream_key(Dir::Tx, 0));
        let ratio = Rational::new(200, 1).unwrap();
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Tx,
            origin: 0,
            config: Config { channels, ratio },
            lead: 1_000,
            call: 0,
            allowance: 0,
        });
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
            assert!(streams.book(&core, to, Item { e: 20_000, seq: cold_seq, ready: 0, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(Config { channels: Channels::first(1), ratio }) }));
            streams.changed(&core, to);
            let source = streams.planned(from).into_iter().find(|planned| planned.segment.by.is_none()).expect("the old source segment");
            let next = streams.planned(to).into_iter().find(|planned| planned.segment.by == Some(move_seq)).expect("the moved transmit segment");
            assert!(next.segment.cut.is_some_and(|cut| next.segment.instant(cut).unwrap() > next.segment.origin));
            (source, next)
        };

        let (entered, reached) = std::sync::mpsc::channel();
        super::super::test_support::probe(&core, "wait_release", move |_| { let _ = entered.send(()); });
        let worker = core.clone();
        let next_for_owner = next.clone();
        let (done, result_rx) = std::sync::mpsc::channel();
        let owner = core.device.host().spawn("tx-stop-with-later-cold", move || { let _ = done.send(worker.owner_configure(Dir::Tx, 0, &next_for_owner)); }).unwrap();
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
        assert!(!lock(&core.streams).stream(to).busy);
    }

    #[test]
    fn ur_34_a_refused_move_reconciles_both_transmit_clocks() {
        let core = two_by_two();
        let (from, to) = (stream_key(Dir::Tx, 0), stream_key(Dir::Tx, 1));
        let ratio = Rational::new(200, 1).unwrap();
        let config = |channels| Config { channels, ratio };
        let line = |channels| Line::new(timeline::Stream {
            direction: Direction::Tx,
            origin: 0,
            config: config(channels),
            lead: core.ticks(50_000_000),
            call: 0,
            allowance: 0,
        });
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
