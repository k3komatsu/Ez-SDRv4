//! Ez-SDR v4 Module ezsdr.radio.mock 2.0.0 (design/09-mock-radio.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod channel;
mod device;
mod profile;
mod time;

use std::collections::BTreeMap;

use ezsdr_hostmem::{HOST_MEMORY, HostPool, write_cf32};
use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::event::{Action, ActionId, Event, EventHandle, EventKind, EventSink, EventSource, Severity};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId};
use ezsdr_kernel::module_api::InputStore;
use ezsdr_kernel::module_api::{
    ActionReceiver, CoerceReport, Dispatched, Driving, Endpoint, ExecutionClass, KERNEL_API, ModuleDescriptor, ModuleError, ModuleRef, PrepareContext, Provider, ProviderInstance, Requested, Role,
    StepOutcome, StopMode, Version, VersionReq, VocabularyRequirement,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BurstOpen, BurstRecord, BurstStep, BurstTracker, ChannelMask, DataLink, Direction, LateOutcome, LatePolicy, PublishOutcome, SampleBlock,
};
use ezsdr_kernel::time::{ClockRegistry, Duration, Rational, SampleClockHandle, ScheduleHandle, TimeAuthority, TimePoint};
use ezsdr_radio::segment::{self, Extent, Pending, Waveform};
use ezsdr_radio::staging::{
    Bookings, DeviceEnd, Outcome, Overlap, Playing, Read, Staging, SwitchRecord, SwitchRefusal, TxBook, TxSend, handover_horizon, is_switch, ordinary_k, stop_bound, stop_fits, stop_sample,
    write_settled,
};
use ezsdr_radio::timeline::{self, Item, Kind, Line, Segment, StreamKey};
use ezsdr_radio::{Addressed, channels, x3xx};
use ezsdr_sim::channel::{Medium, RootInstant};
use ezsdr_sim::{FaultEntry, FaultKind, SimRng};

pub use device::{DeviceModel, DeviceTimeError};
pub use profile::{BLOCK_LEN, Profile, ProfileKind};

/// The reason a lost device gives for everything it cancels or refuses (MR-20).
const DEVICE_LOST: &str = "MR-20: device lost";

fn module_ref() -> ModuleRef {
    ModuleRef { id: ModuleId::parse("ezsdr.radio.mock").expect("valid module id"), version: Version::new(2, 0, 0) }
}

/// The Module descriptor for `ezsdr.radio.mock` 2.0.0 (MR-1).
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: ModuleId::parse("ezsdr.radio.mock").expect("valid module id"),
        version: Version::new(2, 0, 0),
        kernel_api: KERNEL_API,
        roles: vec![Role::Provider],
        vocabularies: [("radio", Version::new(2, 0, 0)), ("sim", Version::new(1, 1, 0))]
            .into_iter()
            .map(|(id, version)| VocabularyRequirement { id: Namespace::parse(id).expect("valid vocabulary"), req: VersionReq(version) })
            .collect(),
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.mock 2.0.0")),
    }
}

/// Deterministic in-process radio Provider (MR-1…MR-30).
///
/// An instance owns its streams: receive and transmit state lives in [`RxStream`] and
/// [`TxStream`], one entry per stream number `k` (RM-29), and their timelines live together
/// in [`timeline::Lines`]. Logical-channel RF timelines
/// and the instance's one SimulationChannel stay here.
pub struct MockRadio {
    instance: ProviderInstance,
    /// The root node of this instance's event sources, as `prepare`'s context names it
    /// (KC-8); sub-nodes are named below it.
    source_root: Option<ResourceId>,
    profile: Profile,
    config: BTreeMap<ezsdr_kernel::spec::Key, Value>,
    /// RM-21, MR-18: the receive values a stopped stream accepted, on top of the Run's configuration.
    staging: Staging,
    prepare_called: bool,
    prepared: bool,
    armed: bool,
    started: bool,
    selector: Selector,
    actions: Option<std::sync::Arc<dyn ActionReceiver>>,
    clocks: Option<std::sync::Arc<ClockRegistry>>,
    time: Option<std::sync::Arc<dyn TimeAuthority>>,
    events: Option<std::sync::Arc<dyn EventSink>>,
    root: Option<ClockDomainId>,
    /// The instance's streams' timelines, one line per stream, keyed by `(direction, k)`
    /// (RM-25); a stream's line is absent until `start` and `arm` book it.
    lines: timeline::Lines,
    /// The instance's receive streams; `k` is a stream's number (RM-29, MR-10).
    rx: Vec<RxStream>,
    /// The instance's transmit streams; `k` is a stream's number (RM-29, MR-10).
    tx: Vec<TxStream>,
    faults: Vec<ScheduledFault>,
    start_tick: Option<i64>,
    sync_end: Option<i64>,
    wakeup: Option<ScheduleHandle>,
    device_lost_reported: bool,
    next_order: u64,
    medium: Option<std::sync::Arc<Medium>>,
    channel: Option<ChannelMode>,
}

/// The channel-coupled state of a Mock whose environment declares `sim.channel` (MR-31).
/// What an individual stream models of the channel lives on the stream (MR-33…MR-35).
struct ChannelMode {
    medium: std::sync::Arc<Medium>,
    fragment: Ident,
    inputs: std::sync::Arc<dyn InputStore>,
    /// MR-34's fresh phases for a `random` board (design 33 D13), drawn in the order the
    /// applied frequency updates reach the Mock, so its output stays a function of its seed
    /// (MR-30).
    lo_retune: SimRng,
    /// Receive channels keep their own gain, frequency and phase timelines.
    rx_channels: Vec<channel::RxChannelModel>,
}

#[derive(Clone)]
struct Selector {
    block_len_jitter: bool,
    rx_test_pattern: String,
}

/// The receive segment being delivered (RM-21).
struct Rx {
    handle: SampleClockHandle,
    /// Its SampleClock, registered at the first block published (RM-25).
    domain: Option<ClockDomainId>,
    origin: i64,
    ratio: Rational,
    /// The set of channels it holds, in the stream's own set (RM-3, RM-21): a layout
    /// stream's set need not be the first `n` channels.
    channels: timeline::Channels,
    by: Option<u64>,
    next: i64,
    planned: Option<i64>,
    flags: BlockFlags,
    lost: Option<u64>,
    /// Each fault report's first lost sample, cause and loss, for the block that carries them (RM-17, MR-37).
    reports: Vec<(i64, ezsdr_radio::payloads::RxOverflowCause, u64)>,
    /// Its SampleClock has ended.
    ended: bool,
}

impl Rx {
    fn config(&self) -> timeline::Config {
        timeline::Config { channels: self.channels, ratio: self.ratio }
    }
}

/// One receive stream of an instance (RM-29, MR-10): its node and port, the SampleClock it
/// was declared with, the segment it is delivering, the links its port publishes to, and
/// its channel-mode state (MR-7, MR-12, MR-25, MR-33…MR-35).
struct RxStream {
    /// Its node, `<device>/rx/s<k>` (MR-10, RM-29): the stream its SampleClock is declared
    /// for (RM-7) and the node a `Stop` or `start_rx` names (RM-21).
    node: ResourceId,
    /// The stream's node below the device, `rx/s<k>`: the path the context's source root
    /// gives it (MR-28).
    path: String,
    /// The name of the port its links attach to (MR-7, RM-29).
    port: String,
    /// `radio.RX_OVERFLOW`'s handle on its event node, resolved in `prepare` (MR-37).
    overflow: Option<EventHandle>,
    /// The receive SampleClock declared in `prepare`, registered at the first block
    /// published (MR-7, MR-25).
    clock: Option<SampleClockHandle>,
    /// The earliest origin of a receive segment not yet begun (RM-25).
    from: i64,
    /// The segment being delivered (RM-21).
    running: Option<Rx>,
    /// The links this stream's port is attached to (MR-7).
    links: Vec<std::sync::Arc<dyn DataLink>>,
    /// The pool its blocks come from (MR-13).
    pool: Option<HostPool>,
    /// `block_len_jitter`'s generator, `<device>/rx/s<k>` (MR-12).
    rng: Option<SimRng>,
    /// The last segment whose untimed tunes were taken, by its origin and arrival order (MR-34).
    tuned: Option<(i64, Option<u64>)>,
    /// The configuration each `start_rx` took at its booking, by arrival order: what the segment it begins starts with (RM-21, MR-33).
    snapshots: BTreeMap<u64, BTreeMap<ezsdr_kernel::spec::Key, Value>>,
}

impl RxStream {
    fn new(node: ResourceId, path: String, port: String) -> RxStream {
        RxStream { node, path, port, overflow: None, clock: None, from: i64::MIN, running: None, links: Vec::new(), pool: None, rng: None, tuned: None, snapshots: BTreeMap::new() }
    }
}

/// One transmit stream of an instance (RM-29, MR-10): its node, the clock its plan registered,
/// the bursts booked on it, and the tracker and device model of that clock (MR-15, MR-18, MR-25).
struct TxStream {
    /// Its node, `<device>/tx/s<k>` (MR-10,
    /// RM-29): the stream its SampleClock is declared for (RM-7) and the node a `TxBurst`
    /// names (RM-15).
    node: ResourceId,
    /// The stream's node below the device, `tx/s<k>`: the path the context's
    /// source root gives it (MR-28).
    path: String,
    /// The transmit SampleClock declared in `prepare`, registered at `arm` (MR-9).
    clock: Option<SampleClockHandle>,
    /// The stream's one transmit clock, registered for the Run (MR-18, RM-25): none or one.
    registered: Option<TxClock>,
    /// The clock's tracker (MR-15).
    tracker: Option<BurstTracker>,
    /// The current clock's device framing model (MR-15, MR-24).
    device: DeviceModel,
    /// The stream's book (RM-13, RM-15, RM-21): the record of the bookings and Stops, their receipts in order (`book.list.held` is the bursts
    /// booked on the current clock, by their first sample), the repeat booked last, which a switch replaces, and the burst being
    /// transmitted (MR-15).
    book: TxBook<HeldBurst>,
    /// The payload cut made by the last Stop on this SampleClock (MR-25).
    stop_cut: Option<TxFront>,
    /// The instance-wide transmitter, read by logical channel and clock segment (MR-32).
    plan: Option<std::sync::Arc<channel::TxShared>>,
}

impl TxStream {
    fn new(node: ResourceId, path: String) -> TxStream {
        TxStream { node, path, clock: None, registered: None, tracker: None, device: DeviceModel::new(), book: TxBook::new(ezsdr_radio::staging::Started::AtAcceptance), stop_cut: None, plan: None }
    }
}

/// A registered transmit clock.
struct TxClock {
    handle: SampleClockHandle,
    domain: ClockDomainId,
    origin: i64,
    by: Option<u64>,
    ended: bool,
}

#[derive(Clone)]
struct ScheduledFault {
    tick: i64,
    entry: FaultEntry,
    applied: bool,
    resolved: bool,
    order: u64,
}

impl ScheduledFault {
    /// MR-20: whether the fault fires in the step's ordered loop at its instant, cutting the
    /// block in progress and waking the step.
    fn fires(&self) -> bool {
        !self.resolved
    }
}

struct HeldBurst {
    start: i64,
    len: i64,
    repeat: bool,
    /// A repeat switch's new waveform (RM-37), and the Action that booked the burst: what the stream's book reads of it.
    switch: bool,
    id: ActionId,
    open: BurstOpen,
    order: u64,
    /// The stream's channel values the burst takes at its start (RM-21, MR-33).
    values: Vec<ChannelValues>,
}

impl ezsdr_radio::staging::Booked for HeldBurst {
    fn id(&self) -> Option<ActionId> {
        Some(self.id)
    }

    fn is_switch(&self) -> bool {
        self.switch
    }
}

/// One logical transmit channel's frequency and gain, as a burst's snapshot holds them.
#[derive(Clone, Copy)]
struct ChannelValues {
    channel: usize,
    frequency_hz: f64,
    gain_db: f64,
}

#[derive(Clone, Copy)]
struct TxFront {
    domain: ClockDomainId,
    sample: i64,
}

/// RM-15: the latest end the bookings received before receipt `before` contribute to the hand-over front: each as `segment::handed_end` counts it
/// for the hand-over horizon `window_end`.
fn handed_front(bookings: impl IntoIterator<Item = (u64, Pending)>, before: u64, window_end: i64) -> Option<i64> {
    bookings.into_iter().filter(|(order, _)| *order < before).fold(None, |front, (_, pending)| merge_front(front, segment::handed_end(&x3xx::PACKETS, pending, window_end)))
}

fn merge_front(front: Option<i64>, candidate: Option<i64>) -> Option<i64> {
    match (front, candidate) {
        (Some(old), Some(end)) => Some(old.max(end)),
        (old, end) => old.or(end),
    }
}

#[derive(Clone, Copy)]
enum WorkKind {
    Fault { index: usize },
    HeldBurst { stream: usize, start: i64 },
    RxBegin { stream: usize, origin: i64 },
    Tune { stream: usize, key: (i64, Option<u64>) },
}

impl MockRadio {
    /// The instance's streams: one receive and one transmit stream per channel in its
    /// layout (RM-29, MR-10). A stream whose direction has no channel holds no channel,
    /// declares no SampleClock and publishes no block (MR-3, MR-5).
    fn streams(profile: &Profile, id: &ResourceId) -> (Vec<RxStream>, Vec<TxStream>) {
        let rx = (0..profile.rx_streams())
            .map(|k| {
                let path = profile.stream_path(Direction::Rx, k);
                let node = id.child(&path).expect("a valid stream id");
                RxStream::new(node, path, profile.rx_port(k))
            })
            .collect();
        let tx = (0..profile.tx_streams())
            .map(|k| {
                let path = profile.stream_path(Direction::Tx, k);
                let node = id.child(&path).expect("a valid stream id");
                TxStream::new(node, path)
            })
            .collect();
        (rx, tx)
    }

    /// The key the instance's lines hold receive stream `k` under (RM-29).
    fn rx_key(k: usize) -> StreamKey {
        stream_key(Direction::Rx, k)
    }

    /// The key the instance's lines hold transmit stream `k` under (RM-29).
    fn tx_key(k: usize) -> StreamKey {
        stream_key(Direction::Tx, k)
    }

    /// Constructs a Mock Provider from a binding, rejecting unsupported content (MR-2).
    pub fn from_binding(binding: &Binding) -> Result<MockRadio, ModuleError> {
        let reject = |why: &str| ModuleError::rejected(format!("MR-2: {why}"));
        if binding.module != module_ref() {
            return Err(reject("binding names a different Module version"));
        }
        let kind = match binding.profile.as_ref() {
            Some(p) if p == &x3xx::profile_ref() => ProfileKind::X3xx,
            Some(p) if p.version == Version::new(2, 0, 0) && p.name == "ideal" => ProfileKind::Ideal,
            _ => return Err(reject("profile must be x3xx 0.1.0 or ideal 2.0.0")),
        };
        if binding.feed.is_some() {
            return Err(reject("Provider binding cannot carry a feed"));
        }
        let allowed: Vec<&str> =
            ["id", "block_len_jitter", "rx_test_pattern", "arm_after"].into_iter().chain(x3xx::SELECTOR_KEYS.into_iter().filter(|_| kind == ProfileKind::X3xx)).chain(["rx", "tx"]).collect();
        if let Some(name) = binding.selector.keys().find(|name| !allowed.contains(&name.as_str())) {
            return Err(reject(&format!("unsupported selector key `{name}`")));
        }
        let selector_value = |name: &str| binding.selector.get(&Ident::parse(name).expect("selector key"));
        let id = match selector_value("id") {
            None => ResourceId::parse("mock").expect("default resource id"),
            Some(Value::Scalar(Scalar::Str(raw))) => ResourceId::parse(raw).ok().filter(|id| id.segments().count() == 1).ok_or_else(|| reject("selector `id` must be one resource path segment"))?,
            Some(_) => return Err(reject("selector `id` must be a string")),
        };
        let block_len_jitter = match selector_value("block_len_jitter") {
            None => false,
            Some(Value::Scalar(Scalar::Bool(value))) => *value,
            _ => return Err(reject("selector `block_len_jitter` must be a Bool")),
        };
        let rx_test_pattern = match selector_value("rx_test_pattern") {
            None => "zero".to_owned(),
            Some(Value::Scalar(Scalar::Str(value))) if value == "zero" || value == "ramp" => value.clone(),
            _ => return Err(reject("selector `rx_test_pattern` must be `zero` or `ramp`")),
        };
        let arm_after = match selector_value("arm_after") {
            None => Vec::new(),
            Some(Value::List(values)) => values
                .iter()
                .map(|value| match value {
                    Scalar::Str(raw) => ResourceId::parse(raw).map_err(|_| reject("selector `arm_after` contains an invalid resource id")),
                    _ => Err(reject("selector `arm_after` must contain resource path strings")),
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(reject("selector `arm_after` must be a list")),
        };
        let layout = match kind {
            ProfileKind::X3xx => channels::from_selector(&binding.selector).map_err(|why| reject(&why))?,
            ProfileKind::Ideal => channels::ideal_from_selector(&binding.selector).map_err(|why| reject(&why))?,
        };
        let profile = Profile::new(kind, layout);
        let timing = profile.timing();
        let mut sections = BTreeMap::new();
        sections.insert(section("envelope"), serde_json::to_value(profile.envelope()).expect("serializable envelope"));
        sections.insert(section("bursts"), serde_json::json!([]));
        sections.insert(section("faults"), serde_json::json!([]));
        sections.insert(section("rejected"), serde_json::json!([]));
        sections.insert(section("stats"), serde_json::json!({"rx_blocks": 0, "rx_samples": 0, "tx_blocks": 0, "rx_clipped": 0, "tx_clipped": 0}));
        sections.insert(section("segments"), serde_json::json!([]));
        sections.insert(section("switches"), serde_json::json!([]));
        let instance = ProviderInstance {
            id: id.clone(),
            module: module_ref(),
            profile: binding.profile.clone(),
            tree: profile.tree(&id, HOST_MEMORY),
            fidelity: profile.fidelity(),
            driving: Driving { stepped: true },
            arm_after,
            min_command_lead: (timing.min_timed_command_lead_ns > 0).then(|| Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, timing.min_timed_command_lead_ns)),
            sections,
        };
        let (rx, tx) = MockRadio::streams(&profile, &id);
        let defaults = profile.defaults().clone();
        Ok(MockRadio {
            instance,
            source_root: None,
            profile,
            staging: Staging::new(defaults.clone()),
            config: defaults,
            prepare_called: false,
            prepared: false,
            armed: false,
            started: false,
            selector: Selector { block_len_jitter, rx_test_pattern },
            actions: None,
            clocks: None,
            time: None,
            events: None,
            root: None,
            lines: timeline::Lines::new(),
            rx,
            tx,
            faults: Vec::new(),
            start_tick: None,
            sync_end: None,
            wakeup: None,
            device_lost_reported: false,
            next_order: 0,
            medium: None,
            channel: None,
        })
    }

    /// Hands this instance the Run's shared SimulationChannel medium; a Mock whose
    /// environment declares `sim.channel` refuses `prepare` without one (MR-31, CH-6).
    pub fn with_medium(mut self, medium: std::sync::Arc<Medium>) -> MockRadio {
        self.medium = Some(medium);
        self
    }

    fn reject(&self, reason: impl Into<String>) -> ModuleError {
        ModuleError::rejected(reason)
    }

    fn selector_id(&self) -> &str {
        self.instance.id.path()
    }

    /// What an Action's resolved node names (KC-23, RM-12, RM-15, RM-21): this instance,
    /// one of its streams, or something else. The judgement is `ezsdr_radio::addressed`'s
    /// one function, shared with the UHD Module.
    fn addressed(&self, node: Option<&ResourceId>) -> ezsdr_radio::Addressed {
        ezsdr_radio::addressed(&self.instance.id, node, |direction| match direction {
            Direction::Rx => self.rx.len(),
            Direction::Tx => self.tx.len(),
        })
    }

    /// Whether `key` is a configuration key of this instance's key set (RM-5, RM-27).
    fn is_configuration(&self, key: &ezsdr_kernel::spec::Key) -> bool {
        ezsdr_radio::channels::is_configuration(key)
    }

    /// RM-3, RM-30: the channel set and sample rate stream `k` of `direction` has in
    /// `configuration`.
    fn stream_set(&self, direction: Direction, k: usize, configuration: &BTreeMap<ezsdr_kernel::spec::Key, Value>) -> (timeline::Channels, f64) {
        let side = if direction == Direction::Rx { "rx" } else { "tx" };
        let description = self.profile.channels();
        (description.stream_channels(configuration, direction == Direction::Rx, k as u32), num_config(configuration, &ezsdr_radio::keys::stream(side, k as u32)))
    }

    /// RM-3, RM-30: how many channels stream `k` of `direction` holds in the configuration
    /// in force (MR-16's channel count).
    fn effective_stream_channels(&self, direction: Direction, k: usize) -> i64 {
        i64::from(self.stream_set(direction, k, &self.config).0.count())
    }

    /// The node the context's source root gives `path` (MR-28): the root itself for an
    /// empty path, and `path`'s segments below it otherwise.
    fn source_node(&self, path: &str) -> ResourceId {
        let root = self.source_root.clone().expect("prepared source root");
        let mut node = root.clone();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            match node.child(segment) {
                Ok(child) => node = child,
                Err(_) => return root,
            }
        }
        node
    }

    /// MR-28: an event whose source is the instance's root, or one of its streams, named
    /// by the stream's own path below the root.
    fn emit_event(&self, path: &str, kind: &str, severity: Severity, payload: serde_json::Value, at: TimePoint) -> Result<(), ModuleError> {
        let (Some(events), Some(_root), Some(_source_root)) = (&self.events, self.root, &self.source_root) else {
            return Ok(());
        };
        let node = self.source_node(path);
        let event = Event { source: EventSource::Node { node }, time: at, severity, kind: EventKind::parse(kind).expect("declared radio event kind"), payload };
        events.emit_control(event).map_err(|error| ModuleError {
            kind: ezsdr_kernel::module_api::ModuleErrorKind::Internal,
            message: format!("MR-10: registered Radio event could not be emitted: {error:?}"),
            detail: serde_json::Value::Null,
        })?;
        Ok(())
    }

    /// Emits receive stream `k`'s `radio.RX_OVERFLOW` on the hot path with RM-24's bytes, as
    /// a device's sample path must (MR-37, RS-32).
    fn emit_overflow(&self, k: usize, payload: ezsdr_radio::payloads::RxOverflowPayload, at: TimePoint) -> Result<(), ModuleError> {
        let (Some(events), Some(handle)) = (&self.events, self.rx[k].overflow) else {
            return Ok(());
        };
        events.emit(handle, at, Severity::Warning, &payload.to_hot()).map_err(|error| ModuleError {
            kind: ezsdr_kernel::module_api::ModuleErrorKind::Internal,
            message: format!("MR-37: registered Radio event could not be emitted: {error:?}"),
            detail: serde_json::Value::Null,
        })
    }

    fn plan_rx_block(&mut self, k: usize) {
        if self.rx[k].running.as_ref().is_none_or(|rx| rx.planned.is_some()) {
            return;
        }
        if let Some(rx) = self.rx[k].running.as_ref() {
            debug_assert_eq!(rx.handle.root_ticks_per_tick(), rx.ratio);
        }
        let len = if self.selector.block_len_jitter {
            let jitter = self.rx[k].rng.as_mut().map(|rng| rng.below(u64::from(self.profile.block_len() * 2))).unwrap_or(0);
            1 + jitter as i64
        } else {
            i64::from(self.profile.block_len())
        };
        if let Some(rx) = self.rx[k].running.as_mut() {
            rx.planned = Some(len);
        }
    }

    fn emit_rx_until(&mut self, k: usize, until: i64) -> Result<bool, ModuleError> {
        let mut progressed = false;
        while let Some(rx) = self.rx[k].running.as_ref() {
            let (next, origin, ratio) = (rx.next, rx.origin, rx.ratio);
            let cut = self.rx_cut(k);
            if cut.is_some_and(|cut| cut <= next) {
                self.rx_settle(k, until)?;
                break;
            }
            // Every fault cuts the block in progress at its instant (MR-20).
            let fault = self.faults.iter().filter(|fault| fault.fires()).map(|fault| fault.tick).min();
            let fault_cut = fault.map(|tick| time::k_at_or_after(origin, ratio, tick).ok_or_else(|| self.reject("MR-12: receive cut overflow"))).transpose()?;
            if fault_cut.is_some_and(|fault_cut| fault_cut <= next) {
                break;
            }
            self.plan_rx_block(k);
            let Some(planned) = self.rx[k].running.as_ref().and_then(|rx| rx.planned) else {
                break;
            };
            let mut stop = next.checked_add(planned).ok_or_else(|| self.reject("MR-12: sample index overflow"))?;
            for bound in [fault_cut, cut].into_iter().flatten() {
                stop = stop.min(bound);
            }
            let after = time::v_after(origin, ratio, stop - 1).ok_or_else(|| self.reject("MR-14: block time overflow"))?;
            if after > until {
                break;
            }
            let (flags, lost, channels) = self.rx[k].running.as_ref().map(|rx| (rx.flags, rx.lost, rx.channels.count() as u16)).expect("a receive stream");
            let full = self.publish_rx(k, next, stop, channels, flags, lost)?;
            progressed = true;
            if full {
                let gap = time::ns_to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), self.profile.timing().overflow_restart_gap_ns)
                    .map_err(|error| self.reject(format!("MR-19: {error}")))?;
                let gap_samples = time::k_at_or_after(0, ratio, gap).ok_or_else(|| self.reject("MR-19: restart sample overflow"))?;
                let after_gap = next.checked_add(gap_samples).ok_or_else(|| self.reject("MR-19: restart sample overflow"))?;
                let next_sample = stop.max(after_gap);
                let lost_total = lost.unwrap_or(0).saturating_add((next_sample - next) as u64);
                let domain = self.rx_register(k)?;
                if let Some(rx) = self.rx[k].running.as_mut() {
                    rx.next = next_sample;
                    rx.planned = None;
                    rx.flags = rx.flags | BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
                    rx.lost = Some(lost_total);
                }
                self.emit_rx_overflow(k, domain, next, next_sample - next)?;
            } else if let Some(rx) = self.rx[k].running.as_mut() {
                rx.next = stop;
                rx.planned = None;
                rx.flags = BlockFlags::NONE;
                rx.lost = None;
                // `stop > next` was checked above, so both counts are at least one.
                let samples = (stop - next) as u64;
                self.add_stat("rx_blocks", 1);
                self.add_stat("rx_samples", samples);
            }
        }
        Ok(progressed)
    }

    fn publish_rx(&mut self, k: usize, first: i64, stop: i64, channels: u16, flags: BlockFlags, lost: Option<u64>) -> Result<bool, ModuleError> {
        let domain = self.rx_register(k)?;
        let len = u32::try_from(stop - first).map_err(|_| self.reject("MR-13: block length overflow"))?;
        let bytes_len = channels as usize * len as usize * 8;
        let pattern = self.selector.rx_test_pattern.clone();
        if self.rx[k].pool.is_none() {
            return Err(self.reject("MR-13: receive pool is absent"));
        }
        let (origin, ratio) = self.rx[k].running.as_ref().map(|rx| (rx.origin, rx.ratio)).expect("a receive stream publishes");
        let mut clipped = 0u64;
        let received: Option<Vec<(f32, f32)>> = if self.channel.is_some() {
            let Some((mode, models, channel_set)) = self.channel.as_ref().and_then(|mode| Some((mode, &mode.rx_channels, self.rx[k].running.as_ref()?.channels))) else {
                return Err(self.reject("MR-35: layout receive channel model is absent"));
            };
            let length = len as usize;
            let mut out = vec![(0.0, 0.0); channels as usize * length];
            for (position, logical) in channel_set.iter().enumerate() {
                let model = &models[logical as usize];
                for sample in first..stop {
                    let stamp = RootInstant::of_sample(origin, ratio, sample);
                    let antenna = RootInstant::of_sample(origin, ratio, sample.saturating_sub(model.path_delay_samples));
                    let (fr, fi) = mode.medium.field(&mode.fragment, logical as u16, model.frequency_hz.at(stamp), antenna);
                    let amplitude = 10f64.powf(model.gain_db.at(stamp) / 20.0);
                    let (sin, cos) = model.phase.at(stamp).sin_cos();
                    let re = amplitude * (fr * cos + fi * sin);
                    let im = amplitude * (fi * cos - fr * sin);
                    let (cre, cim) = (re.clamp(-1.0, 1.0), im.clamp(-1.0, 1.0));
                    if cre != re || cim != im {
                        clipped += 1;
                    }
                    out[position * length + (sample - first) as usize] = (cre as f32, cim as f32);
                }
            }
            Some(out)
        } else {
            None
        };
        let pool = self.rx[k].pool.as_mut().expect("pool was checked");
        let bytes = pool.fill(bytes_len, |buffer| {
            buffer[..bytes_len].fill(0);
            if let Some(received) = &received {
                for (index, (re, im)) in received.iter().enumerate() {
                    write_cf32(buffer, len as usize, index / len as usize, index % len as usize, *re, *im);
                }
            } else if pattern == "ramp" {
                for channel in 0..channels as usize {
                    for index in 0..len as usize {
                        let sample = (first as u64 + index as u64) % 65_536;
                        write_cf32(buffer, len as usize, channel, index, sample as f32 / 65_536.0, channel as f32 / 64.0);
                    }
                }
            }
        });
        let header = BlockHeader {
            first_sample_time: TimePoint::new(domain, first),
            len,
            channels,
            direction: Direction::Rx,
            valid: ChannelMask::full(channels),
            flags,
            lost,
            contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract"),
        };
        let block = std::sync::Arc::new(SampleBlock::new_host(header, HOST_MEMORY, bytes, 8).map_err(|error| self.reject(format!("MR-13: {error}")))?);
        self.add_stat("rx_clipped", clipped);
        let mut full = false;
        for link in &self.rx[k].links {
            if link.publish(block.clone()) == PublishOutcome::Full {
                full = true;
            }
        }
        // RM-17, MR-37: one event for the block's reported loss; MR-19 reported its own drops already.
        let reports = self.rx[k].running.as_mut().map(|rx| std::mem::take(&mut rx.reports)).unwrap_or_default();
        let causes: Vec<_> = reports.iter().map(|(_, cause, _)| *cause).collect();
        let reported: Option<u64> = lost.map(|_| reports.iter().map(|(_, _, lost)| lost).sum());
        if let Some(payload) = ezsdr_radio::payloads::rx_overflow_payload(&causes, reported) {
            let at = reports.iter().map(|(kf, _, _)| *kf).min().expect("a reported loss");
            self.emit_overflow(k, payload, TimePoint::new(domain, at))?;
        }
        Ok(full)
    }

    fn emit_rx_overflow(&mut self, k: usize, domain: ClockDomainId, at: i64, lost: i64) -> Result<(), ModuleError> {
        let payload = ezsdr_radio::payloads::RxOverflowPayload { cause: ezsdr_radio::payloads::RxOverflowCause::Overrun, lost: lost.max(0) as u64 };
        self.emit_overflow(k, payload, TimePoint::new(domain, at))
    }

    /// MR-21: whether receive stream `k` runs its segment at the fault instant `f` received
    /// with `seq`.
    fn rx_runs(&self, k: usize, f: i64, seq: u64) -> Result<bool, ModuleError> {
        Ok(match (self.rx[k].running.as_ref(), self.lines.line(Self::rx_key(k))) {
            (Some(rx), Some(line)) => f >= rx.origin && line.made(rx.origin, rx.config(), f, seq).map_err(rm25)?.is_some_and(|segment| segment.cut.is_none()),
            _ => false,
        })
    }

    /// MR-21, MR-22, D5: the lowest-numbered receive stream the fault at `index` cuts — one
    /// running at the fault's instant (RM-29); none when no stream runs then, which leaves
    /// the fault unapplied (MR-20).
    fn faulted_rx_stream(&self, index: usize) -> Result<Option<usize>, ModuleError> {
        let (f, seq) = (self.faults[index].tick, self.faults[index].order);
        for k in 0..self.rx.len() {
            if self.rx_runs(k, f, seq)? {
                return Ok(Some(k));
            }
        }
        Ok(None)
    }

    /// MR-21, MR-22: an overrun or a sequence error at `f`, on the receive stream `k`'s
    /// segment running then; its row is written now and never revised, its event comes with
    /// the block that carries its loss, if any (MR-20; spec 22, VH-5).
    fn apply_rx_fault(&mut self, k: usize, index: usize) -> Result<(), ModuleError> {
        if self.faults[index].resolved {
            return Ok(());
        }
        self.faults[index].resolved = true;
        let (f, seq) = (self.faults[index].tick, self.faults[index].order);
        let runs = self.rx_runs(k, f, seq)?;
        let Some(rx) = self.rx[k].running.as_ref().filter(|_| runs) else {
            self.record_fault(index, 0);
            return Ok(());
        };
        let (origin, ratio, next) = (rx.origin, rx.ratio, rx.next);
        let kf = time::k_at_or_after(origin, ratio, f).unwrap_or(next).max(next);
        let (kg, cause) = match self.faults[index].entry.fault {
            FaultKind::RxOverflow => {
                let gap_ns = self.profile.timing().overflow_restart_gap_ns;
                let gap = time::ns_to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), gap_ns).map_err(|error| self.reject(format!("MR-21: {error}")))?;
                let target = f.checked_add(gap).ok_or_else(|| self.reject("MR-21: restart time overflow"))?;
                (time::k_at_or_after(origin, ratio, target).unwrap_or(kf).max(kf), ezsdr_radio::payloads::RxOverflowCause::Overrun)
            }
            FaultKind::RxSequenceError => (kf.saturating_add(i64::from(self.profile.block_len())), ezsdr_radio::payloads::RxOverflowCause::Sequence),
            FaultKind::DeviceLost => unreachable!("a loss ends the device (MR-20)"),
        };
        let lost = kg - kf;
        if let Some(rx) = self.rx[k].running.as_mut() {
            rx.next = kg;
            rx.planned = None;
            if lost > 0 {
                rx.flags = rx.flags
                    | BlockFlags::GAP_BEFORE
                    | match cause {
                        ezsdr_radio::payloads::RxOverflowCause::Overrun => BlockFlags::RESTARTED,
                        ezsdr_radio::payloads::RxOverflowCause::Sequence => BlockFlags::SEQ_DISCONTINUITY,
                    };
                rx.lost = Some(rx.lost.unwrap_or(0).saturating_add(lost as u64));
                rx.reports.push((kf, cause, lost as u64));
            }
        }
        self.faults[index].applied = true;
        self.record_fault(index, lost as u64);
        Ok(())
    }

    fn record_fault(&mut self, index: usize, lost: u64) {
        #[derive(serde::Serialize)]
        struct Record {
            at: TimePoint,
            fault: FaultKind,
            applied: bool,
            lost: u64,
        }
        let fault = &self.faults[index];
        let record = Record { at: TimePoint::new(self.root.expect("prepared root"), fault.tick), fault: fault.entry.fault, applied: fault.applied, lost };
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("faults")) {
            rows.push(serde_json::to_value(record).expect("fault section row"));
        }
    }

    /// MR-20: the faults not yet fired are recorded in the order they would have fired (SE-4): by instant, document order (`self.faults`' own) breaking ties.
    fn record_pending_faults(&mut self) {
        let mut pending: Vec<usize> = (0..self.faults.len()).filter(|&index| !self.faults[index].resolved).collect();
        pending.sort_by_key(|&index| self.faults[index].tick);
        for index in pending {
            self.faults[index].resolved = true;
            self.record_fault(index, 0);
        }
    }

    fn schedule_wakeup(&mut self) -> Result<(), ModuleError> {
        let (Some(time), Some(root)) = (self.time.clone(), self.root) else {
            return Ok(());
        };
        let now = time.now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-14: {error}")))?;
        let mut next: Option<i64> = None;
        let mut wake = |tick: i64| next = Some(next.map_or(tick, |next| next.min(tick)));
        if self.started {
            self.faults.iter().filter(|fault| fault.fires()).for_each(|fault| wake(fault.tick));
            for k in 0..self.rx.len() {
                if let Some(segment) = self.rx_next_segment(k) {
                    wake(segment.origin);
                }
            }
        }
        for k in 0..self.rx.len() {
            let cut = self.rx_cut(k);
            if self.rx[k].running.as_ref().is_some_and(|rx| cut.is_none_or(|cut| rx.next < cut)) {
                let fault = self.faults.iter().filter(|fault| fault.fires()).map(|fault| fault.tick).min();
                let rx = self.rx[k].running.as_ref().expect("a receive stream");
                let fault_cut = fault.and_then(|tick| time::k_at_or_after(rx.origin, rx.ratio, tick));
                if fault_cut.is_none_or(|fault_cut| fault_cut > rx.next) {
                    self.plan_rx_block(k);
                }
                if let Some(rx) = self.rx[k].running.as_ref() {
                    if let Some(planned) = rx.planned {
                        let stop = [fault_cut, cut].into_iter().flatten().fold(rx.next.saturating_add(planned), i64::min);
                        if stop > rx.next {
                            if let Some(tick) = time::v_after(rx.origin, rx.ratio, stop - 1) {
                                wake(tick);
                            }
                        }
                    }
                }
            }
        }
        for k in 0..self.tx.len() {
            if let (Some(block), Some(clock)) = (self.tx_block(k, None), self.tx_now(k)) {
                if let Some(tick) = time::v_after(clock.origin, clock.handle.root_ticks_per_tick(), block.next.saturating_add(block.wanted) - 1) {
                    wake(tick);
                }
            }
            if let Some(clock) = self.tx_now(k) {
                let (handle, origin) = (&clock.handle, clock.origin);
                if let Some(start) = self.tx[k].book.list.held.keys().next().copied() {
                    if let Some(tick) = time::v_floor(origin, handle.root_ticks_per_tick(), start) {
                        wake(tick);
                    }
                }
            }
        }
        let wanted = next.filter(|tick| *tick > now);
        if self.wakeup.is_some_and(|handle| wanted == Some(handle.ticks)) {
            return Ok(());
        }
        if let Some(handle) = self.wakeup.take() {
            time.cancel(handle);
        }
        if let Some(tick) = wanted {
            self.wakeup = Some(time.schedule(TimePoint::new(root, tick), Box::new(|_| {})).map_err(|error| self.reject(format!("MR-14: {error}")))?);
        }
        Ok(())
    }

    /// MR-20: refuses every Action queued for a lost device, recording each in `rejected`
    /// with the loss's reason and emitting no event: the step's `DeviceLost` reports it.
    fn refuse_actions(&mut self, at: i64) {
        let Some(actions) = self.actions.clone() else { return };
        while let Some(dispatched) = actions.recv() {
            self.record_rejected_action(Self::action_name(&dispatched.action), DEVICE_LOST, at);
        }
    }

    fn record_rejected_action(&mut self, kind: &str, reason: &str, at: i64) {
        #[derive(serde::Serialize)]
        struct Rejected {
            kind: String,
            reason: String,
            at: TimePoint,
        }
        let time = TimePoint::new(self.root.expect("prepared root"), at);
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("rejected")) {
            rows.push(serde_json::to_value(Rejected { kind: kind.to_owned(), reason: reason.to_owned(), at: time }).expect("rejection section row"));
        }
    }

    /// RM-22: `COMMAND_REJECTED`, naming the Action by `id` when it is refused at receipt.
    fn reject_action_at(&mut self, id: Option<ActionId>, kind: &str, reason: &str, at: i64) -> Result<(), ModuleError> {
        self.record_rejected_action(kind, reason, at);
        let time = TimePoint::new(self.root.expect("prepared root"), at);
        let payload = serde_json::to_value(ezsdr_radio::payloads::CommandRejectedPayload { action: id, kind: kind.to_owned(), reason: reason.to_owned() }).expect("command rejection payload");
        self.emit_event("", ezsdr_radio::kinds::COMMAND_REJECTED, Severity::Error, payload, time)
    }

    fn record_burst(&mut self, record: BurstRecord) {
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("bursts")) {
            rows.push(serde_json::to_value(record).expect("burst record"));
        }
    }

    fn add_stat(&mut self, name: &str, n: u64) {
        if n == 0 {
            return;
        }
        if let Some(serde_json::Value::Object(stats)) = self.instance.sections.get_mut(&section("stats")) {
            let total = stats.get(name).and_then(serde_json::Value::as_u64).unwrap_or(0);
            stats.insert(name.to_owned(), serde_json::json!(total + n));
        }
    }

    /// Cuts every burst of transmit stream `k`'s current clock at sample `k_cut` and
    /// withdraws the bursts that start at or after it, as a stop does
    /// (MR-32).
    fn cut_segments(&mut self, k: usize, k_cut: i64) {
        let Some(shared) = self.tx[k].plan.clone() else { return };
        let mut plan = shared.plan();
        plan.segments.retain(|(_, _, stream, start), _| *stream != k || *start < k_cut);
        for ((_, _, stream, _), segment) in plan.segments.iter_mut() {
            if *stream == k {
                segment.end = Some(segment.end.map_or(k_cut, |end| end.min(k_cut)));
            }
        }
    }

    /// Ends the open burst's radiation at transmit stream `k`'s sample `stop` (MR-32).
    fn end_open_segment(&mut self, k: usize, stop: i64) {
        let Some(shared) = self.tx[k].plan.clone() else { return };
        let Some(open) = self.tx[k].book.open.as_ref() else { return };
        if let Some(clock) = self.tx_now(k) {
            let key = (clock.origin, clock.by, k, open.booking.start);
            if let Some(segment) = shared.plan().segments.get_mut(&key) {
                segment.end = Some(segment.end.map_or(stop, |end| end.min(stop)));
            }
        }
    }

    /// Transmits every sample of stream `k` before `cut`: the open burst up to it, and every
    /// held burst that starts before it, opened in turn (MR-25, MR-32).
    fn transmit_until_cut(&mut self, k: usize, now: i64, cut: i64) -> Result<(), ModuleError> {
        loop {
            if self.tx[k].book.open.is_some() {
                self.emit_tx_until(k, now, Some(cut))?;
            }
            if self.tx[k].book.open.is_some() {
                break;
            }
            let Some(start) = self.tx[k].book.list.held.keys().next().copied().filter(|start| *start < cut) else { break };
            let held = self.tx[k].book.list.held.remove(&start).expect("the first held burst");
            self.open_held(k, held);
        }
        Ok(())
    }

    /// Opens a held burst for transmission and sets the values it takes at its start (RM-21,
    /// MR-33). Called in instant order, which [`channel::Timeline::at`] relies on.
    fn open_held(&mut self, k: usize, held: HeldBurst) {
        let at = self.tx_now(k).map(|clock| RootInstant::of_sample(clock.origin, clock.handle.root_ticks_per_tick(), held.start));
        if let (Some(at), Some(plan)) = (at, self.tx[k].plan.clone()) {
            let mut plan = plan.plan();
            for value in &held.values {
                let model = &mut plan.channel_models[value.channel];
                model.frequency_hz.set_at(at, value.frequency_hz);
                model.gain_db.set_at(at, value.gain_db);
            }
        }
        let next = held.start;
        // MR-42: a switch at the open repeat's next sample ends it there (UHD's `join_open` ends the booking too but keeps the device burst; the Mock closes it, MR-15).
        if let Some(open) = self.tx[k].book.open.as_ref() {
            debug_assert_eq!(open.next, next, "only a switch at the open booking's next sample meets it here");
            if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) {
                self.record_burst(record);
            }
            self.tx[k].device.close();
            self.tx[k].book.end_open(DeviceEnd::Keep);
        }
        self.tx[k].book.take(held, next);
    }

    fn action_name(action: &Action) -> &'static str {
        match action {
            Action::TxBurst { .. } => "tx_burst",
            Action::SetTimer { .. } => "set_timer",
            Action::UpdateParameter { .. } => "update_parameter",
            Action::Command { .. } => "command",
            Action::Emit { .. } => "emit",
            Action::Stop { .. } => "stop",
            Action::Abort { .. } => "abort",
        }
    }

    fn emit_late_burst(&self, k: usize, late_by: Duration, target: TimePoint, now_tx: TimePoint, outcome: ezsdr_radio::payloads::TimeErrorOutcome) -> Result<(), ModuleError> {
        let late_by_ns = time::to_ns(self.clocks.as_ref().expect("prepared clocks"), late_by).unwrap_or(i64::MAX);
        let payload = serde_json::to_value(ezsdr_radio::payloads::TimeErrorPayload { cause: ezsdr_radio::payloads::TimeErrorCause::Late, outcome, late_by_ns, target, action: None })
            .expect("time error payload");
        self.emit_event(&self.tx[k].path, ezsdr_radio::kinds::TIME_ERROR, Severity::Error, payload, now_tx)
    }

    /// MR-34: the first segment of receive stream `k`'s line after the one it last tuned, by
    /// origin and arrival order.
    fn tune_next(&self, k: usize) -> Option<(i64, Option<u64>)> {
        let line = self.lines.line(stream_key(Direction::Rx, k))?;
        let tuned = self.rx[k].tuned;
        line.plan.iter().map(|segment| (segment.origin, segment.by)).find(|key| tuned.is_none_or(|tuned| *key > tuned))
    }

    /// MR-34: receive stream `k`'s segment `key` begins at `tick` with an untimed tune of every
    /// channel it carries. The stream's first segment has no arrival order and takes none of
    /// these: its phases are drawn at `prepare`.
    fn tune(&mut self, k: usize, key: (i64, Option<u64>), tick: i64) {
        self.rx[k].tuned = Some(key);
        if self.rx[k].links.is_empty() {
            return;
        }
        let Some(line) = self.lines.line(stream_key(Direction::Rx, k)) else { return };
        let Some(index) = line.plan.iter().position(|segment| (segment.origin, segment.by) == key) else { return };
        let segment = line.plan[index];
        if segment.by.is_none() {
            return;
        }
        let channels: Vec<u32> = segment.config.channels.iter().collect();
        // A `start_rx` that restarts the stream at the origin of its first segment (a `Stop` at T0 cut that one with no sample) is the segment that
        // runs: its snapshot, not the Run's configuration, is what the row of that origin states (RM-25, MR-27).
        if let Some(seq) = segment.by {
            let values = self.rx_values(k, Some(seq), segment.config.channels);
            if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("segments")) {
                if let Some(row) = rows.iter_mut().find(|row| row["direction"] == "rx" && row["stream"].as_u64() == Some(k as u64) && row["origin"]["ticks"].as_i64() == Some(segment.origin)) {
                    row["values"] = values;
                }
            }
        }
        // MR-33: the snapshot's frequency and gain are the channel's from this origin on.
        if let (Some(snapshot), Some(mode)) = (segment.by.and_then(|seq| self.rx[k].snapshots.get(&seq)), self.channel.as_mut()) {
            for number in &channels {
                if let Some(model) = mode.rx_channels.get_mut(*number as usize) {
                    model.frequency_hz.set(tick, num_config(snapshot, &ezsdr_radio::keys::channel("rx", *number, "frequency_hz")));
                    model.gain_db.set(tick, num_config(snapshot, &ezsdr_radio::keys::channel("rx", *number, "gain_db")));
                }
            }
        }
        self.untimed_tune(&channels, tick);
    }

    /// MR-34: each named receive channel takes a fresh LO phase at `tick`; a `deterministic`
    /// channel keeps its zero phase.
    fn untimed_tune(&mut self, channels: &[u32], tick: i64) {
        for number in channels {
            if let Some(mode) = self.channel.as_mut() {
                if let Some(model) = mode.rx_channels.get_mut(*number as usize) {
                    if model.phase_behavior != "deterministic" {
                        model.phase.set(tick, channel::lo_phase(&mut mode.lo_retune));
                    }
                }
            }
        }
    }

    /// MR-27: records the logical channel order of a layout SampleClock segment.
    fn layout_phase_basis(&self, direction: Direction, channels: &[u32]) -> (String, bool) {
        if self.channel.is_none() {
            return ("unmodelled".to_owned(), false);
        }
        let mut fixed = true;
        for number in channels {
            let behavior = match direction {
                Direction::Rx => {
                    let Some(model) = self.channel.as_ref().and_then(|mode| mode.rx_channels.get(*number as usize)) else {
                        return ("unmodelled".to_owned(), false);
                    };
                    model.phase_behavior.clone()
                }
                Direction::Tx => {
                    let Some(shared) = self.tx.iter().find_map(|stream| stream.plan.clone()) else {
                        return ("unmodelled".to_owned(), false);
                    };
                    let plan = shared.plan();
                    let Some(model) = plan.channel_models.get(*number as usize) else {
                        return ("unmodelled".to_owned(), false);
                    };
                    model.phase_behavior.clone()
                }
            };
            if behavior != "deterministic" {
                fixed = false;
            }
        }
        if fixed { ("deterministic".to_owned(), true) } else { ("none".to_owned(), false) }
    }

    /// MR-27, MR-33: the frequency, gain and antenna the receive segment begun by `sequence` starts with, per channel:
    /// its `start_rx`'s snapshot, or the Run's configuration for the stream's first segment.
    fn rx_values(&self, k: usize, sequence: Option<u64>, channels: timeline::Channels) -> serde_json::Value {
        let snapshot = sequence.and_then(|seq| self.rx[k].snapshots.get(&seq)).unwrap_or(&self.config);
        let mut values = serde_json::Map::new();
        for number in channels.iter() {
            let mut channel = serde_json::Map::new();
            for leaf in ["frequency_hz", "gain_db", "antenna"] {
                let key = ezsdr_kernel::spec::Key::parse(&ezsdr_radio::keys::channel("rx", number, leaf)).expect("a declared radio key");
                channel.insert(leaf.to_owned(), snapshot.get(&key).map_or(serde_json::Value::Null, |value| serde_json::to_value(value).expect("a value is JSON")));
            }
            values.insert(format!("c{number}"), serde_json::Value::Object(channel));
        }
        serde_json::Value::Object(values)
    }

    fn end_layout_segment(&mut self, direction: Direction, k: usize, origin: i64, sequence: Option<u64>, at: i64) {
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("segments")) {
            if let Some(row) = rows.iter_mut().find(|row| {
                row["direction"] == if direction == Direction::Rx { "rx" } else { "tx" }
                    && row["stream"].as_u64() == Some(k as u64)
                    && row["sequence"] == serde_json::to_value(sequence).expect("optional sequence")
                    && row["origin"]["ticks"].as_i64() == Some(origin)
            }) {
                row["ended_at"] = serde_json::to_value(TimePoint::new(self.root.expect("prepared root"), at)).expect("segment end time");
            }
        }
    }

    /// MR-27: records the logical channel order and D13 phase basis of a layout segment.
    fn record_layout_segment(&mut self, direction: Direction, k: usize, origin: i64, sequence: Option<u64>, channels: timeline::Channels) {
        let values = (direction == Direction::Rx).then(|| self.rx_values(k, sequence, channels));
        let channel_numbers: Vec<u32> = channels.iter().collect();
        let phase_behavior: Vec<&str> = channel_numbers.iter().map(|number| self.profile.channel_model_terms(direction, *number as usize).0).collect();
        let (basis, valid) = self.layout_phase_basis(direction, &channel_numbers);
        let mut row = serde_json::json!({
            "direction": if direction == Direction::Rx { "rx" } else { "tx" },
            "stream": k,
            "sequence": sequence,
            "origin": TimePoint::new(self.root.expect("prepared root"), origin),
            "channels": channel_numbers,
            "phase_behavior": phase_behavior,
            "coherence": [{ "at": TimePoint::new(self.root.expect("prepared root"), origin), "basis": basis, "valid": valid }],
            "ended_at": serde_json::Value::Null,
        });
        if let Some(values) = values {
            row["values"] = values;
        }
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("segments")) {
            rows.push(row);
        }
    }

    fn record_cancelled_burst(&mut self, reason: &str, now: i64, cancel_reason: ezsdr_radio::CancelReason) -> Result<(), ModuleError> {
        if ezsdr_radio::cancellation_reports(cancel_reason) {
            self.reject_action_at(None, "tx_burst", reason, now)
        } else {
            self.record_rejected_action("tx_burst", reason, now);
            Ok(())
        }
    }

    /// Reads the stream's list in receipt order: a booking is held by its start, a halt cancels what was read before it, with `reason`.
    fn read_tx(&mut self, k: usize, now: i64, reason: &str) -> Result<(), ModuleError> {
        while let Some(read) = self.tx[k].book.read() {
            match read {
                Read::Booking(held) => {
                    self.tx[k].book.list.held.insert(held.start, held);
                }
                Read::Halt { reason: cancel_reason, cancelled } => {
                    for _ in cancelled {
                        self.record_cancelled_burst(reason, now, cancel_reason)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn stop_tx(&mut self, k: usize, now: i64, reason: &str, cancel_reason: ezsdr_radio::CancelReason) -> Result<(), ModuleError> {
        self.tx[k].book.list.halt(cancel_reason);
        if let Some(clock) = self.tx_now(k) {
            let (ratio, origin, domain) = (clock.handle.root_ticks_per_tick(), clock.origin, clock.domain);
            let window = self.tx_window()?;
            let bound = stop_bound(now, window, |t| time::k_at_or_after(origin, ratio, t).unwrap_or(i64::MAX), |k| time::v_of(origin, ratio, k).unwrap_or(i64::MAX));
            let region = time::k_at_or_after(origin, ratio, now).map(|at_stop| (at_stop, stop_sample(now, window, |t| time::k_at_or_after(origin, ratio, t).unwrap_or(i64::MAX))));
            // MR-25's payload cut and MR-17's later-booking margin stay separate.
            let handover = matches!(cancel_reason, ezsdr_radio::CancelReason::Stop | ezsdr_radio::CancelReason::ProviderStop);
            let mut planned = None;
            if let Some((at_stop, _)) = region {
                planned = Some(if handover {
                    let payload = self.tx_handed_end(k, now, self.next_order)?;
                    let previous = self.tx[k].stop_cut.filter(|front| front.domain == domain).map(|front| front.sample);
                    (now.saturating_add(window), timeline::later_bound(at_stop, merge_front(payload, previous)))
                } else {
                    (now, at_stop)
                });
            }
            // RM-37, MR-42: a switch is applied iff its boundary is below the payload cut, the first sample the Mock does not emit.
            self.tx[k].book.stop_played((bound, region), planned.map_or(i64::MAX, |(_, cut)| cut));
            if let Some((until, cut)) = planned {
                // The cut is known: no loss can follow it, so the record does not hold a sent switch back for device time (RM-37).
                self.tx[k].book.record.settle_unresolved();
                self.settle_switches(k);
                self.transmit_until_cut(k, until, cut)?;
                self.cut_segments(k, cut);
                if handover {
                    self.tx[k].stop_cut = Some(TxFront { domain, sample: cut });
                }
            }
        }
        if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) {
            self.record_burst(record);
        }
        self.tx[k].device.close();
        // The halt pushed above cancels the held bookings and an open one that has emitted no block (#85, the book's one rule).
        self.read_tx(k, now, reason)?;
        self.tx[k].book.end_open(DeviceEnd::Keep);
        Ok(())
    }

    /// MR-25's `stop` at `now`, its cancellations recorded with `reason`; a loss runs its
    /// `abort` body at the loss's instant with `DEVICE_LOST` (MR-20), as an item received
    /// with the faults (RM-25). Every stream of the instance is stopped.
    fn stop_at(&mut self, mode: StopMode, now: i64, reason: &str, seq: u64, cancel_reason: ezsdr_radio::CancelReason) -> Result<(), ModuleError> {
        for k in 0..self.tx.len() {
            self.stop_tx(k, now, reason, cancel_reason)?;
        }
        self.record_pending_faults();
        if let Some(handle) = self.wakeup.take() {
            if let Some(time) = &self.time {
                time.cancel(handle);
            }
        }
        // RM-16: under `abort` the receive cut is the first sample not yet delivered; a loss
        // cuts at its instant, having delivered the block in progress (MR-20).
        let abort = mode == StopMode::Abort && cancel_reason != ezsdr_radio::CancelReason::Loss;
        let delivered: Vec<Option<i64>> = self.rx.iter().map(|stream| stream.running.as_ref().filter(|_| abort).map(|rx| rx.next)).collect();
        for (k, delivered) in delivered.into_iter().enumerate() {
            self.book_rx(k, Item { e: now, seq, ready: now, delivered, refused: false, kind: Kind::End { abort } })?;
        }
        for k in 0..self.tx.len() {
            if self.lines.line(Self::tx_key(k)).is_none() {
                continue;
            }
            self.book_tx(k, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::End { abort } })?;
            self.end_tx_clock(k)?;
        }
        // An orderly stop delivers the block in progress up to its cut; a loss has (MR-20).
        if mode == StopMode::Orderly {
            for k in 0..self.rx.len() {
                self.emit_rx_until(k, now)?;
            }
        }
        for k in 0..self.rx.len() {
            self.rx_finish(k, now)?;
        }
        if mode == StopMode::Abort {
            self.started = false;
        } else if let (Some(time), Some(root)) = (self.time.clone(), self.root) {
            // MA-7: one drain round at the stop instant hands the last block to the Executors
            // and Sinks that read it.
            self.wakeup = Some(time.schedule(TimePoint::new(root, now), Box::new(|_| {})).map_err(|error| self.reject(format!("MR-25: {error}")))?);
        }
        Ok(())
    }

    /// The cut of the receive segment stream `k` is delivering, as a sample index of it:
    /// the plan's, or 0 when the plan has dropped the segment, cut with no sample (RM-16,
    /// RM-25).
    fn rx_cut(&self, k: usize) -> Option<i64> {
        let (rx, line) = (self.rx[k].running.as_ref()?, self.lines.line(Self::rx_key(k))?);
        line.segment(rx.origin, rx.config()).map_or(Some(0), |segment| segment.cut)
    }

    /// RM-25: receive stream `k`'s segment's SampleClock, registered at the first block
    /// published.
    fn rx_register(&mut self, k: usize) -> Result<ClockDomainId, ModuleError> {
        let clocks = self.clocks.clone().expect("prepared clocks");
        let rx = self.rx[k].running.as_mut().expect("a receive stream");
        if let Some(domain) = rx.domain {
            return Ok(domain);
        }
        let domain = clocks.register_sample_clock(&rx.handle, rx.origin).map_err(|error| ModuleError::rejected(format!("MR-12: {error}")))?;
        rx.domain = Some(domain);
        Ok(domain)
    }

    /// RM-16, RM-25: once the items in effect by `t` cut receive stream `k`'s segment, its
    /// SampleClock, if it has published a block, ends at the cut's instant; a segment cut at
    /// its origin has none, and another may begin there in its place.
    fn rx_settle(&mut self, k: usize, t: i64) -> Result<(), ModuleError> {
        let Some((origin, config, by, ended)) = self.rx[k].running.as_ref().map(|rx| (rx.origin, rx.config(), rx.by, rx.ended)) else { return Ok(()) };
        let Some(line) = self.lines.line(Self::rx_key(k)) else { return Ok(()) };
        match line.made(origin, config, t, u64::MAX).map_err(rm25)? {
            None => {
                self.end_layout_segment(Direction::Rx, k, origin, by, t);
                self.rx[k].running = None;
                self.rx[k].from = origin;
            }
            Some(Segment { cut: Some(cut), .. }) if !ended => {
                let at = time::v_of(origin, config.ratio, cut).ok_or_else(|| self.reject("MR-12: receive cut overflow"))?;
                if let Some(domain) = self.rx[k].running.as_ref().and_then(|rx| rx.domain) {
                    self.clocks
                        .as_ref()
                        .expect("prepared clocks")
                        .end(domain, TimePoint::new(self.root.expect("prepared root"), at))
                        .map_err(|error| ModuleError::rejected(format!("MR-12: {error}")))?;
                }
                self.end_layout_segment(Direction::Rx, k, origin, by, at);
                if let Some(rx) = self.rx[k].running.as_mut() {
                    rx.ended = true;
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// The receive segment stream `k` is delivering ends, its cut made by `t` (RM-16).
    fn rx_finish(&mut self, k: usize, t: i64) -> Result<(), ModuleError> {
        self.rx_settle(k, t)?;
        if self.rx[k].running.as_ref().is_some_and(|rx| rx.ended) {
            self.rx[k].running = None;
        }
        Ok(())
    }

    /// The next receive segment of stream `k`'s plan, after the last one begun (RM-25).
    fn rx_next_segment(&self, k: usize) -> Option<Segment> {
        if self.rx[k].links.is_empty() {
            return None;
        }
        let line = self.lines.line(Self::rx_key(k))?;
        line.plan.iter().find(|segment| segment.origin >= self.rx[k].from).copied()
    }

    /// RM-25: a receive segment of stream `k` begins at its origin, with the configuration
    /// the plan gives it.
    fn rx_begin(&mut self, k: usize, segment: Segment, handle: Option<SampleClockHandle>) -> Result<(), ModuleError> {
        self.rx_finish(k, segment.origin)?;
        self.rx[k].running = None;
        let handle = match handle {
            Some(handle) => handle,
            None => {
                let stream = self.rx[k].node.clone();
                self.clocks
                    .as_ref()
                    .expect("prepared clocks")
                    .declare_sample_clock(stream, self.root.expect("prepared root"), segment.config.ratio)
                    .map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?
            }
        };
        let channels = segment.config.channels;
        self.rx[k].running = Some(Rx {
            handle,
            domain: None,
            origin: segment.origin,
            ratio: segment.config.ratio,
            channels,
            by: segment.by,
            next: 0,
            planned: None,
            flags: BlockFlags::NONE,
            lost: None,
            reports: Vec::new(),
            ended: false,
        });
        self.rx[k].pool = Some(HostPool::new(self.profile.block_len() as usize * 2 * channels.count() as usize * 8));
        self.rx[k].from = segment.origin.saturating_add(1);
        self.record_layout_segment(Direction::Rx, k, segment.origin, segment.by, channels);
        Ok(())
    }

    /// Books a receive command or fault on receive stream `k`'s timeline (RM-25); returns
    /// its effective instant, or the item's own when the stream has no line yet (a change
    /// before `start`).
    fn book_rx(&mut self, k: usize, item: Item) -> Result<i64, ModuleError> {
        if self.lines.line(Self::rx_key(k)).is_none() {
            return Ok(item.e);
        }
        self.lines.book(Self::rx_key(k), item).map_err(rm25)
    }

    /// Books a transmit command on transmit stream `k`'s timeline (RM-25); nothing when the
    /// stream has no line (no transmit clock).
    fn book_tx(&mut self, k: usize, item: Item) -> Result<(), ModuleError> {
        if self.lines.line(Self::tx_key(k)).is_none() {
            return Ok(());
        }
        self.lines.book(Self::tx_key(k), item).map_err(rm25)?;
        Ok(())
    }

    /// Transmit stream `k`'s clock (MR-18).
    fn tx_now(&self, k: usize) -> Option<&TxClock> {
        self.tx[k].registered.as_ref()
    }

    /// The cut of transmit stream `k`'s clock, from the plan (RM-25).
    fn tx_cut(&self, k: usize) -> Option<i64> {
        self.tx_now(k)?;
        self.lines.line(Self::tx_key(k))?.plan.first()?.cut
    }

    /// RM-16: the in-flight window of a transmit stream, in root ticks.
    fn tx_window(&self) -> Result<i64, ModuleError> {
        time::ns_to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), self.profile.timing().tx_in_flight_ns)
            .map_err(|error| ModuleError::rejected(format!("MR-25: {error}")))
    }

    /// RM-16: whether a `Stop` at `now` has a bound on transmit stream `k`'s clock; a stream with no clock has nothing to bound.
    fn tx_stop_fits(&self, k: usize, now: i64) -> Result<bool, ModuleError> {
        let Some(clock) = self.tx_now(k) else { return Ok(true) };
        let (ratio, origin) = (clock.handle.root_ticks_per_tick(), clock.origin);
        Ok(stop_fits(now, self.tx_window()?, |t| time::k_at_or_after(origin, ratio, t), |k| time::v_of(origin, ratio, k)))
    }

    /// MR-15: the next block of stream `k`'s open booking before `extra_cut` (a Stop's cut, which ends no Kernel burst): at most one block, cut at the
    /// booking's wrap or end, the next held booking's start and the plan's cut.
    fn tx_block(&self, k: usize, extra_cut: Option<i64>) -> Option<Extent> {
        let open = self.tx[k].book.open.as_ref()?;
        let waveform = Waveform { start: open.booking.start, len: open.booking.len, repeat: open.booking.repeat, tiled: false };
        let held_cut = self.tx[k].book.list.held.keys().next().copied();
        let hard_cut = self.tx_cut(k).into_iter().chain(extra_cut).min();
        segment::extent(waveform, open.next, i64::from(self.profile.block_len()), held_cut, hard_cut)
    }

    /// RM-15: the handed payload end of stream `k`'s clock (MR-25's payload cut): the latest end of the bursts whose first send fits the hand-over window.
    fn tx_handed_end(&self, k: usize, now: i64, before: u64) -> Result<Option<i64>, ModuleError> {
        let Some(clock) = self.tx_now(k) else { return Ok(None) };
        let window_end = handover_horizon(now, self.tx_window()?, |t| time::k_at_or_after(clock.origin, clock.handle.root_ticks_per_tick(), t).unwrap_or(i64::MAX));
        let pending = |booking: &HeldBurst, open: bool| (booking.order, Pending { start: booking.start, len: booking.len, repeat: booking.repeat, open });
        let book = &self.tx[k].book;
        Ok(handed_front(book.open.iter().map(|open| pending(&open.booking, true)).chain(book.list.held.values().map(|held| pending(held, false))), before, window_end))
    }

    /// RM-25, VH-2: the transmit clock of stream `k` ends at its origin when a stop is booked at
    /// or before it, as `Streams::changed` ends it (`core.rs:666–672`). A stream with no declared
    /// clock registers its own here, once.
    fn end_tx_clock(&mut self, k: usize) -> Result<(), ModuleError> {
        let Some(segment) = self.lines.line(Self::tx_key(k)).and_then(|line| line.plan.first()).copied() else { return Ok(()) };
        let clocks = self.clocks.clone().expect("prepared clocks");
        let root = self.root.expect("prepared root");
        if self.tx[k].registered.is_none() {
            let stream = self.tx[k].node.clone();
            let handle = clocks.declare_sample_clock(stream, root, segment.config.ratio).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
            let domain = clocks.register_sample_clock(&handle, segment.origin).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
            self.tx[k].registered = Some(TxClock { handle, domain, origin: segment.origin, by: segment.by, ended: false });
            self.record_layout_segment(Direction::Tx, k, segment.origin, segment.by, segment.config.channels);
        }
        let ended_at = {
            let clock = self.tx[k].registered.as_mut().expect("registered above");
            if let Some(cut) = segment.cut.filter(|_| !clock.ended) {
                let at = time::v_of(clock.origin, clock.handle.root_ticks_per_tick(), cut).ok_or_else(|| ModuleError::rejected("MR-18: transmit cut overflow"))?;
                clocks.end(clock.domain, TimePoint::new(root, at)).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
                clock.ended = true;
                Some(at)
            } else {
                None
            }
        };
        if let Some(at) = ended_at {
            self.end_layout_segment(Direction::Tx, k, segment.origin, segment.by, at);
        }
        Ok(())
    }

    /// RM-25, M3: root ticks per sample at `rate` on this profile, from the decimation
    /// integer on `x3xx` and the integer rate on `ideal`, never from the `f64` rate
    /// (TM-20). `rule` is the rule a refusal is reported under.
    fn rate_ratio(&self, root_rate: Rational, rate: f64, rule: &str) -> Result<Rational, ModuleError> {
        match self.profile.kind {
            ProfileKind::X3xx => {
                let n = self.profile.decimation(rate).ok_or_else(|| self.reject(format!("{rule}: rate {rate} is not an X3xx decimation")))?;
                Rational::new(root_rate.num().checked_mul(n).ok_or_else(|| self.reject(format!("{rule}: ratio overflow")))?, x3xx::MASTER_CLOCK_HZ)
            }
            ProfileKind::Ideal => Rational::new(root_rate.num(), rate as u64),
        }
        .map_err(|error| self.reject(format!("{rule}: {error}")))
    }

    /// RM-25: root ticks per sample at `rate` on this profile.
    fn ratio(&self, rate: f64) -> Result<Rational, ModuleError> {
        let root = self.root.expect("root");
        let clocks = self.clocks.as_ref().expect("clocks");
        let root_rate = clocks.nominal_rate(root).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
        self.rate_ratio(root_rate, rate, "MR-18")
    }

    /// A stream's timeline terms on this profile (RM-25, MR-18, RM-29).
    fn line(&self, direction: Direction, k: usize, origin: i64, config: &BTreeMap<ezsdr_kernel::spec::Key, Value>) -> Result<Line, ModuleError> {
        let (channels, rate) = self.stream_set(direction, k, config);
        let ratio = self.ratio(rate)?;
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        let root = self.root.expect("prepared root");
        let ticks = |ns: i64| time::ns_to_v(clocks, root, ns).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")));
        Ok(Line::new(timeline::Stream {
            direction,
            origin,
            config: timeline::Config { channels, ratio },
            lead: match direction {
                Direction::Rx => ticks(timeline::start_lead_ns(&self.profile.timing()))?,
                Direction::Tx => 0,
            },
            call: self.profile.rx_call_samples(),
            allowance: ticks(self.profile.delivery_allowance_ns())?,
        }))
    }

    /// `node` is what the Kernel resolved the target to (KC-23), which is what a
    /// Provider matches on: the instance's own node or one of its stream nodes (RM-12, D14).
    fn handle_action(&mut self, dispatched: Dispatched, now: i64) -> Result<(), ModuleError> {
        let id = dispatched.id;
        let node = dispatched.node.as_ref();
        let untimed = dispatched.untimed;
        match dispatched.action {
            // MR-13, MR-16: a burst names its transmit stream (RM-13, RM-29).
            action @ Action::TxBurst { .. } => match self.addressed(node) {
                Addressed::Stream { direction: Direction::Tx, k } => self.handle_tx_burst(k, id, action, node, untimed, now),
                _ => {
                    self.refuse_tx_burst(id, now)?;
                    Ok(())
                }
            },
            Action::UpdateParameter { key, value, at, .. } => self.handle_update(id, node, key, value, at.is_some(), now),
            // RM-16, MR-25: a `Stop` of the device or of a stream cancels no update; on
            // receive it is a cut at its instant, on transmit it ends the bursts. A `Stop`
            // of the device stops every stream of the instance; one of a stream node names
            // that stream alone (RM-12, D14).
            Action::Stop { target } => {
                let device = target.is_none() || node == Some(&self.instance.id);
                let stream = match self.addressed(node) {
                    Addressed::Stream { direction, k } => Some((direction, k)),
                    Addressed::Instance | Addressed::Other => None,
                };
                if !device && stream.is_none() {
                    return self.reject_action_at(Some(id), "stop", "MR-25: Stop target is not this device or its stream", now);
                }
                let streams: Vec<(Direction, usize)> =
                    if device { (0..self.tx.len()).map(|k| (Direction::Tx, k)).chain((0..self.rx.len()).map(|k| (Direction::Rx, k))).collect() } else { stream.into_iter().collect() };
                // RM-16, MR-25: a stream whose Stop has no bound that fits its clock is refused with the Action's id, nothing changed; the others go on.
                for (direction, k) in streams {
                    if direction == Direction::Tx {
                        if !self.tx_stop_fits(k, now)? {
                            self.reject_action_at(Some(id), "stop", "MR-25: the Stop's bound overflows the transmit clock", now)?;
                            continue;
                        }
                        self.stop_tx(k, now, "MR-25: cancelled by stop", ezsdr_radio::CancelReason::Stop)?;
                    } else {
                        let seq = self.arrival();
                        self.book_rx(k, Item { e: now, seq, ready: now, delivered: None, refused: false, kind: Kind::Stop })?;
                    }
                }
                Ok(())
            }
            // RM-12, RM-21: `start_rx` turns the receive stream on at its `at`, or at its
            // receipt if that is later, never before T0; it is never late. Only a receive
            // stream's own node, with no params, is that command (MR-29).
            Action::Command { verb, params, at, .. } if verb.as_str() == ezsdr_radio::START_RX => {
                let (Addressed::Stream { direction: Direction::Rx, k }, true) = (self.addressed(node), params.is_empty()) else {
                    return self.reject_action_at(Some(id), "command", "MR-29: Action `command` is not supported by MockRadio", now);
                };
                let requested = match at.map(|at| time::to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), at.time_point)).transpose() {
                    Ok(requested) => requested,
                    Err(error) => return self.reject_action_at(Some(id), "command", &format!("MR-29: start_rx's instant cannot be converted: {error}"), now),
                };
                // RM-21: a stream that is streaming (a booked start not yet begun counts) takes no start; the line is unchanged.
                let line = self.lines.line(Self::rx_key(k));
                let snapshot = match self.staging.start(k as u32, line.is_some_and(Line::streaming)) {
                    Ok(snapshot) => snapshot,
                    Err(refusal) => return self.reject_action_at(Some(id), "command", &format!("MR-29: {}", refusal.reason()), now),
                };
                let e = timeline::command_instant(line, requested, now);
                let seq = self.arrival();
                // RM-25: a booking the line cannot represent (an instant that overflows) is a refusal with the Action's id, never a failed Run.
                if let Err(error) = self.book_rx(k, Item { e, seq, ready: now, delivered: None, refused: false, kind: Kind::Start }) {
                    return self.reject_action_at(Some(id), "command", &error.message, now);
                }
                self.rx[k].snapshots.insert(seq, snapshot);
                Ok(())
            }
            other => {
                let name = Self::action_name(&other);
                self.reject_action_at(Some(id), name, &format!("MR-29: Action `{name}` is not supported by MockRadio"), now)?;
                Ok(())
            }
        }
    }

    /// The next arrival order (RM-25): the faults have the first.
    fn arrival(&mut self) -> u64 {
        let order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        order
    }

    /// MR-16: the refusal of a burst whose target, clock, waveform size, channel count or
    /// metadata this Provider does not accept, named by the id it was dispatched with.
    fn refuse_tx_burst(&mut self, id: ActionId, now: i64) -> Result<(), ModuleError> {
        self.reject_action_at(Some(id), "tx_burst", "MR-16: target, clock, waveform size, channel count or metadata is invalid", now)
    }

    fn handle_tx_burst(&mut self, k: usize, id: ActionId, action: Action, node: Option<&ResourceId>, untimed: bool, now: i64) -> Result<(), ModuleError> {
        let Action::TxBurst { waveform, repeat, at, requested_at, late_policy, metadata, .. } = action else { unreachable!("only TxBurst is dispatched to handle_tx_burst") };
        let size_bytes = waveform.size_bytes;
        let switch = is_switch(repeat, late_policy, &metadata);
        let metadata_ok = metadata.is_empty() || switch;
        let root = self.root.expect("prepared root");
        let tx_id = self.tx[k].node.clone();
        // The burst's clock: the stream's one (KC-21a, RM-25).
        let Some((ratio, origin, tx_domain, clock_order)) =
            self.tx_now(k).filter(|clock| clock.domain == at.time_point.domain()).map(|clock| (clock.handle.root_ticks_per_tick(), clock.origin, clock.domain, clock.by))
        else {
            let reason = if self.tx_now(k).is_none() { "MR-16: no transmit channel is configured" } else { "MR-16: target, clock, waveform size, channel count or metadata is invalid" };
            self.reject_action_at(Some(id), "tx_burst", reason, now)?;
            return Ok(());
        };
        let channels = self.effective_stream_channels(Direction::Tx, k);
        let unit = channels.max(0) as u64 * 8;
        if node != Some(&tx_id) || channels <= 0 || size_bytes == 0 || unit == 0 || size_bytes % unit != 0 || !metadata_ok {
            self.refuse_tx_burst(id, now)?;
            return Ok(());
        }
        let Ok(waveform_len) = u32::try_from(size_bytes / unit) else {
            self.reject_action_at(Some(id), "tx_burst", "MR-16: the waveform has more samples than a burst can hold", now)?;
            return Ok(());
        };
        let length = i64::from(waveform_len);
        if repeat && (length as u64 > self.profile.repeat_max_samples() || length as u64 % self.profile.repeat_align_samples() != 0) {
            self.reject_action_at(Some(id), "tx_burst", "MR-16: repeated waveform violates RM-13's length or alignment limits", now)?;
            return Ok(());
        }
        let decoded = match self.channel.as_ref().map(|mode| mode.inputs.get(&waveform.hash)) {
            None => None,
            Some(None) => {
                self.reject_action_at(Some(id), "tx_burst", "MR-32: the waveform's bytes are not an input of this Run", now)?;
                return Ok(());
            }
            Some(Some(bytes)) if bytes.len() as u64 != size_bytes => {
                self.reject_action_at(Some(id), "tx_burst", "MR-32: the waveform's bytes do not have its size_bytes", now)?;
                return Ok(());
            }
            Some(Some(bytes)) => match channel::decode_waveform(&bytes) {
                Ok(decoded) => Some(decoded),
                Err(reason) => {
                    self.reject_action_at(Some(id), "tx_burst", reason, now)?;
                    return Ok(());
                }
            },
        };
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        // RM-22: a time that does not fit a clock is a refusal of the Action with its id, never a failed Run.
        let instants = || -> Result<_, String> {
            let lead = time::ns_to_v(clocks, root, self.profile.timing().min_timed_command_lead_ns).map_err(|error| format!("MR-17: {error}"))?;
            let join_lead = lead + time::ns_to_v(clocks, root, self.profile.delivery_allowance_ns()).map_err(|error| format!("MR-17: {error}"))?;
            let window = time::ns_to_v(clocks, root, self.profile.timing().tx_in_flight_ns).map_err(|error| format!("MR-25: {error}"))?;
            // RM-15: decided at the clock's origin less the lead if that is later.
            let decided = timeline::decided_at(now, origin, lead);
            let now_tx = clocks.convert(TimePoint::new(root, now), tx_domain).map_err(|error| format!("MR-17: {error}"))?.floor();
            let decided_tx = match clocks.convert(TimePoint::new(root, decided), tx_domain).map_err(|error| format!("MR-17: {error}"))? {
                ezsdr_kernel::time::Converted::Exact { point } => point,
                ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.checked_add(Duration::new(tx_domain, 1)).map_err(|error| format!("MR-17: {error}"))?,
            };
            let target = at.time_point.ticks_in(tx_domain).map_err(|error| format!("MR-17: {error}"))?;
            Ok((lead, join_lead, window, decided, now_tx, decided_tx, target))
        };
        let (lead, join_lead, window, decided, now_tx, decided_tx, target) = match instants() {
            Ok(instants) => instants,
            Err(reason) => {
                self.reject_action_at(Some(id), "tx_burst", &reason, now)?;
                return Ok(());
            }
        };
        let ordinary = ordinary_k(decided, lead, |t| time::k_at_or_after(origin, ratio, t).unwrap_or(i64::MAX), |k| time::v_of(origin, ratio, k).unwrap_or(i64::MAX));
        let now_k = time::k_at_or_after(origin, ratio, now).unwrap_or(i64::MAX);
        let horizon = handover_horizon(now, window, |t| time::k_at_or_after(origin, ratio, t).unwrap_or(i64::MAX));
        let join_by = |e: i64| time::v_of(origin, ratio, e).map_or(i64::MIN, |instant| instant.saturating_sub(join_lead));
        let decision = if switch || untimed {
            Ok(LateOutcome::OnTime {})
        } else {
            late_policy.decide(clocks, at.time_point, decided_tx, Duration::new(ClockDomainId::HOST_MONOTONIC, self.profile.timing().min_timed_command_lead_ns))
        };
        let mut policy = match decision {
            Ok(policy) => policy,
            Err(error) => {
                self.reject_action_at(Some(id), "tx_burst", &format!("MR-17: {error}"), now)?;
                return Ok(());
            }
        };
        self.tx[k].book.record.prune(now_k);
        self.settle_switches(k);
        let mut start = target;
        let mut requested = requested_at.map(|deadline| deadline.time_point);
        let mut send_asap_late_by = None;
        let mut unplaced = false;
        let mut switched_from = None;
        let sync_k = self.sync_end.and_then(|sync| time::k_at_or_after(origin, ratio, sync)).unwrap_or(i64::MIN);
        let overlap_text = |overlap: Overlap| match overlap {
            Overlap::Booking => "MR-16: burst overlaps an accepted booking",
            Overlap::Handed => "MR-16: burst starts before a booking that may already be handed",
        };
        if untimed && !switch {
            match self.tx[k].book.record.place_untimed(target, ordinary.max(sync_k), (!repeat).then_some(length), now, join_by, horizon) {
                Some(free) => start = free,
                None => unplaced = true,
            }
        }
        match policy {
            LateOutcome::OnTime {} if late_policy == LatePolicy::SendAsapAndFlag && !switch && !untimed => {
                match self.tx[k].book.record.region_move(target, (!repeat).then_some(length), now, join_by, horizon) {
                    Ok(moved) if moved != target => {
                        let late_ns = match time::to_ns(self.clocks.as_ref().expect("prepared clocks"), Duration::new(tx_domain, moved - target)) {
                            Ok(late_ns) => late_ns,
                            Err(error) => {
                                self.reject_action_at(Some(id), "tx_burst", &format!("MR-17: {error}"), now)?;
                                return Ok(());
                            }
                        };
                        let late_by = Duration::new(ClockDomainId::HOST_MONOTONIC, late_ns);
                        start = moved;
                        policy = LateOutcome::SendAsap { late_by };
                        requested = requested.or(Some(at.time_point));
                        send_asap_late_by = Some(late_by);
                    }
                    Ok(_) => {}
                    Err(_) => unplaced = true,
                }
            }
            LateOutcome::OnTime {} => {}
            LateOutcome::SendAsap { late_by } => {
                match self.tx[k].book.record.place(ordinary, (!repeat).then_some(length), now, join_by, horizon) {
                    Some(free) => start = free,
                    None => unplaced = true,
                }
                requested = requested.or(Some(at.time_point));
                send_asap_late_by = Some(late_by);
            }
            LateOutcome::Drop { late_by } => {
                self.emit_late_burst(k, late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::Drop)?;
                self.record_rejected_action("tx_burst", "MR-17: the late policy dropped the burst", now);
                return Ok(());
            }
            LateOutcome::PlanViolation { late_by } => {
                self.emit_late_burst(k, late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::PlanViolation)?;
                self.record_rejected_action("tx_burst", "MR-17: the planned burst arrived late", now);
                return Ok(());
            }
        }
        let switch_bound = time::k_at_or_after(origin, ratio, now.saturating_add(window).saturating_add(lead)).unwrap_or(i64::MAX);
        // A switch's boundary previewed for the overflow and synchronization checks, which refuse before the book changes; the booking
        // takes the one `receive_switch` returns.
        if switch {
            switched_from = self.tx[k].book.playing;
            start = self.tx[k].book.switch_boundary(target, switch_bound).unwrap_or(start);
        }
        // RM-22: the instants are computed before anything is booked: one that does not fit the clock is a refusal, not a booked chain.
        let start_v = time::v_of(origin, ratio, start);
        let end = (!repeat).then(|| time::v_of(origin, ratio, start.saturating_add(length)));
        let overflow = "MR-16: the burst time overflows the transmit clock";
        let refusal = if start_v.is_none() || end.is_some_and(|end| end.is_none()) {
            Some(overflow)
        } else if start_v.is_some_and(|start_v| start_v < self.sync_end.unwrap_or(i64::MIN)) {
            Some("MR-16: burst begins before synchronization")
        } else if switch {
            match self.tx[k].book.receive_switch(id, length, target, switch_bound, untimed, now_k) {
                Ok(accepted) => {
                    debug_assert_eq!(start, accepted.boundary, "the previewed switch boundary is the book's");
                    start = accepted.boundary;
                    requested = requested.or(accepted.requested_target.map(|_| at.time_point));
                    None
                }
                Err(SwitchRefusal::TimeOverflow) => Some(overflow),
                Err(_) => Some("MR-16: a repeat switch needs a playing repeat and no pending switch"),
            }
        } else if unplaced {
            Some("MR-16: burst overlaps an accepted booking")
        } else {
            self.tx[k].book.record.accept(start, (!repeat).then_some(length), now, join_by, horizon).err().map(overlap_text)
        };
        self.settle_switches(k);
        if let Some(reason) = refusal {
            if let Some(late_by) = send_asap_late_by {
                self.emit_late_burst(k, late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::Refused)?;
            }
            self.reject_action_at(Some(id), "tx_burst", reason, now)?;
            return Ok(());
        }
        if let Some(late_by) = send_asap_late_by {
            self.emit_late_burst(k, late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::SendAsap)?;
        }
        if let (Some(shared), Some((samples, clipped))) = (self.tx[k].plan.clone(), decoded) {
            let channel_numbers: Vec<u16> =
                self.lines.line(Self::tx_key(k)).and_then(|line| line.plan.first()).map(|segment| segment.config.channels.iter().map(|number| number as u16).collect()).unwrap_or_default();
            shared
                .plan()
                .segments
                .insert((origin, clock_order, k, start), channel::Segment { origin, ratio, start, len: length, repeat, channels: channels as u16, channel_numbers, samples, end: None });
            self.add_stat("tx_clipped", clipped);
        }
        let (_, values) = self.staging.tx_snapshot(k as u32);
        let values: Vec<ChannelValues> = self
            .stream_set(Direction::Tx, k, &values)
            .0
            .iter()
            .map(|number| {
                let channel = number as usize;
                ChannelValues {
                    channel,
                    frequency_hz: num_config(&values, &ezsdr_radio::keys::channel("tx", number, "frequency_hz")),
                    gain_db: num_config(&values, &ezsdr_radio::keys::channel("tx", number, "gain_db")),
                }
            })
            .collect();
        let held = HeldBurst {
            start,
            len: length,
            repeat,
            switch,
            id,
            open: BurstOpen { waveform_len: Some(waveform_len), late: (!matches!(policy, LateOutcome::OnTime {})).then_some(policy), requested_target: requested },
            order: self.next_order,
            values,
        };
        self.tx[k].book.book(held, end.flatten());
        self.read_tx(k, now, "")?;
        self.next_order = self.next_order.saturating_add(1);
        if let Some(from) = switched_from {
            self.record_switch(from, id, length, start, now);
        }
        self.tx[k].book.play(repeat, id, start, length);
        Ok(())
    }

    /// RM-13: a switch accepted at `now` is a row of the `switches` section, `pending` until it is settled.
    fn record_switch(&mut self, from: Playing, id: ActionId, new_period: i64, boundary: i64, now: i64) {
        let row = SwitchRecord {
            action_id: id,
            predecessor: from.id,
            selected_at: TimePoint::new(self.root.expect("prepared root"), now),
            old_period: from.period,
            new_period,
            boundary,
            outcome: "pending",
        };
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("switches")) {
            rows.push(serde_json::to_value(row).expect("switch section row"));
        }
    }

    /// RM-13: writes the outcome of every switch the stream's record settled since the last call into its row.
    fn settle_switches(&mut self, k: usize) {
        let settled = self.tx[k].book.record.take_settled();
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("switches")) {
            write_settled(rows, settled);
        }
    }

    /// RM-37: applies each taken switch whose boundary device time `u` has reached, in every transmit stream, and settles its row.
    /// `advance` calls it after the step's faults and bursts, so a loss or Stop before a boundary cancels the switch first (`halt`).
    fn settle_switches_to(&mut self, u: i64) {
        for k in 0..self.tx.len() {
            if let Some((ratio, origin)) = self.tx_now(k).map(|clock| (clock.handle.root_ticks_per_tick(), clock.origin)) {
                if let Some(now_k) = time::k_at_or_after(origin, ratio, u) {
                    self.tx[k].book.advance(now_k);
                }
            }
            self.settle_switches(k);
        }
    }

    fn emit_tx_until(&mut self, k: usize, until: i64, extra_cut: Option<i64>) -> Result<bool, ModuleError> {
        let mut progressed = false;
        while let Some(open) = self.tx[k].book.open.as_ref() {
            let Some((ratio, origin, domain)) = self.tx_now(k).map(|clock| (clock.handle.root_ticks_per_tick(), clock.origin, clock.domain)) else {
                break;
            };
            let Some(block) = self.tx_block(k, extra_cut) else { break };
            let stop = block.next.saturating_add(block.wanted);
            let last = time::v_after(origin, ratio, stop - 1).ok_or_else(|| ModuleError::rejected("MR-15: transmit time overflow"))?;
            if last > until {
                break;
            }
            let next = open.next;
            let first = open.first;
            let burst_open = open.booking.open;
            let len = u32::try_from(stop - next).map_err(|_| ModuleError::rejected("MR-15: transmit block length overflow"))?;
            let channels = self.effective_stream_channels(Direction::Tx, k) as u16;
            let ends = block.ends_waveform(block.wanted) || block.ends_at_held(block.wanted);
            let flags = (if first { BlockFlags::START_OF_BURST } else { BlockFlags::NONE }) | if ends { BlockFlags::END_OF_BURST } else { BlockFlags::NONE };
            let header = BlockHeader {
                first_sample_time: TimePoint::new(domain, next),
                len,
                channels,
                direction: Direction::Tx,
                valid: ChannelMask::full(channels),
                flags,
                lost: None,
                contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").expect("cf32"),
            };
            let tracker_result = self.tx[k].tracker.as_mut().expect("tracker").on_block(&header, first.then_some(burst_open));
            match tracker_result {
                Ok(BurstStep::Ended { record }) => self.record_burst(record),
                Ok(BurstStep::Discontinuity { closed, then_ended, .. }) => {
                    self.record_burst(closed);
                    if let Some(record) = then_ended {
                        self.record_burst(record);
                    }
                    self.reject_action_at(None, "tx_burst", "MR-15: the burst's blocks were not contiguous", until)?;
                }
                Ok(BurstStep::Started | BurstStep::Continued) => {}
                Err(error) => {
                    self.reject_action_at(None, "tx_burst", &format!("MR-15: {error}"), until)?;
                    self.end_open_segment(k, next);
                    if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) {
                        self.record_burst(record);
                    }
                    self.tx[k].device.close();
                    self.tx[k].book.end_open(DeviceEnd::Keep);
                    break;
                }
            }
            if self.tx[k].device.on_tx_block(&header).is_err() {
                let payload = serde_json::to_value(ezsdr_radio::payloads::TimeErrorPayload {
                    cause: ezsdr_radio::payloads::TimeErrorCause::UnclosedBurst,
                    outcome: ezsdr_radio::payloads::TimeErrorOutcome::Refused,
                    late_by_ns: 0,
                    target: header.first_sample_time,
                    action: None,
                })
                .expect("device error payload");
                self.emit_event(&self.tx[k].path, ezsdr_radio::kinds::TIME_ERROR, Severity::Error, payload, header.first_sample_time)?;
                self.record_rejected_action("tx_burst", "MR-24: the device rejected an unclosed burst", until);
                self.end_open_segment(k, next);
                if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) {
                    self.record_burst(record);
                }
                self.tx[k].device.close();
                self.tx[k].book.end_open(DeviceEnd::Keep);
                progressed = true;
                break;
            }
            self.add_stat("tx_blocks", 1);
            progressed = true;
            // MR-15: the block is the device's whole result; the book advances the burst, and its last block ends it.
            let block = i64::from(len);
            let id = self.tx[k].book.issue(TxSend { count: block, device_count: block, ends });
            self.tx[k].book.complete(id, Outcome::Accepted(block), None);
            if ends {
                self.tx[k].device.close();
            }
        }
        Ok(progressed)
    }

    /// MR-18, RM-21: a receive channel's frequency, gain or antenna is staged for the next `start_rx` of its
    /// stream when the stream is not streaming, and refused with the cause otherwise (the Kernel refuses a
    /// classless key first; the refusal of one here is the Provider's own guard for a harness that calls it directly).
    fn handle_update(&mut self, id: ActionId, node: Option<&ResourceId>, key: ezsdr_kernel::spec::Key, value: Value, has_at: bool, now: i64) -> Result<(), ModuleError> {
        // RM-12, D14: the node names the instance or one of its streams; what changes is what `key` says.
        if matches!(self.addressed(node), Addressed::Other) {
            return self.reject_action_at(Some(id), "update_parameter", "MR-18: the target is not this device or its streams", now);
        }
        let lines = &self.lines;
        let tx = &self.tx;
        match self.staging.set(
            &key,
            value,
            has_at,
            now,
            |k| lines.line(Self::rx_key(k as usize)).is_some_and(Line::streaming),
            |k| tx.get(k as usize).map_or(Bookings::unarmed(), |stream| stream.book.record.clone()),
        ) {
            Ok(()) => Ok(()),
            Err(refusal) => self.reject_action_at(Some(id), "update_parameter", &format!("MR-18: {}", refusal.reason()), now),
        }
    }

    /// The step's ordered loop up to `u` (RM-25, MR-20): every stream's receive segment
    /// beginnings, then faults and held bursts by instant and arrival, then the untimed tunes of
    /// the instant (MR-34), the streams emitted up to each.
    fn advance(&mut self, u: i64) -> Result<bool, ModuleError> {
        debug_assert!(self.tx.iter().all(|stream| !stream.book.list.unread()), "advance runs with every receipt read");
        let mut progressed = false;
        loop {
            let mut candidates: Vec<(i64, u8, u64, WorkKind)> = Vec::new();
            if self.started {
                candidates.extend(self.faults.iter().enumerate().filter(|(_, fault)| fault.fires()).map(|(index, fault)| (fault.tick, 1, fault.order, WorkKind::Fault { index })));
                for k in 0..self.rx.len() {
                    if let Some(segment) = self.rx_next_segment(k) {
                        candidates.push((segment.origin, 0, 0, WorkKind::RxBegin { stream: k, origin: segment.origin }));
                    }
                }
            }
            // MR-34: an untimed tune comes after the instant's faults and held bursts (rank 2). The
            // pushes into each `channel::Timeline` are then in instant order, which
            // `channel::Timeline::at` relies on. On a deterministic channel an untimed tune leaves its
            // stream's phases as they are; otherwise it redraws them (MR-34). MA-30 does not order an instant's Actions before the loop passes that
            // instant (design 09 MR-34's ceiling).
            if self.started {
                for k in 0..self.rx.len() {
                    if let Some(key) = self.tune_next(k) {
                        candidates.push((key.0, 2, tune_order(k), WorkKind::Tune { stream: k, key }));
                    }
                }
            }
            for k in 0..self.tx.len() {
                candidates.extend(self.tx[k].book.list.held.iter().filter_map(|(start, burst)| {
                    let clock = self.tx_now(k)?;
                    // shortcut: a held burst opens at the last root tick at or before its first sample, which comes after the previous sample only while a sample spans a whole root tick (true on the 1 GHz root for every rate the Mock accepts), upgrade by opening on the sample's rational instant when a slower root is declared.
                    let tick = time::v_floor(clock.origin, clock.handle.root_ticks_per_tick(), *start)?;
                    Some((tick, 1, burst.order, WorkKind::HeldBurst { stream: k, start: *start }))
                }));
            }
            candidates.sort_by_key(|(tick, rank, order, _)| (*tick, *rank, *order));
            let Some((tick, _, _, kind)) = candidates.first().copied() else {
                break;
            };
            if tick > u {
                break;
            }
            if self.started {
                for k in 0..self.rx.len() {
                    self.emit_rx_until(k, tick)?;
                }
                for k in 0..self.tx.len() {
                    self.emit_tx_until(k, tick, None)?;
                }
            }
            match kind {
                WorkKind::Fault { index } => match self.faults[index].entry.fault {
                    // The instance's faults cut the lowest-numbered receive stream running
                    // at their instant (RM-29, MR-21).
                    FaultKind::RxOverflow | FaultKind::RxSequenceError => match self.faulted_rx_stream(index)? {
                        Some(k) => self.apply_rx_fault(k, index)?,
                        None => {
                            self.faults[index].resolved = true;
                            self.record_fault(index, 0);
                        }
                    },
                    FaultKind::DeviceLost => {
                        // MR-20: the blocks before `f` are out; the device stops at `f` as an
                        // `abort` would there, the receive stream cut at `f`, and the step fails.
                        self.faults[index].applied = true;
                        self.faults[index].resolved = true;
                        self.record_fault(index, 0);
                        self.device_lost_reported = true;
                        let seq = self.faults[index].order;
                        self.stop_at(StopMode::Abort, tick, DEVICE_LOST, seq, ezsdr_radio::CancelReason::Loss)?;
                        return Err(ModuleError { kind: ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost, message: DEVICE_LOST.to_owned(), detail: serde_json::Value::Null });
                    }
                },
                WorkKind::HeldBurst { stream, start } => {
                    if let Some(held) = self.tx[stream].book.list.held.remove(&start) {
                        self.open_held(stream, held);
                    }
                }
                WorkKind::RxBegin { stream, origin } => match self.rx_next_segment(stream).filter(|segment| segment.origin == origin) {
                    Some(segment) => self.rx_begin(stream, segment, None)?,
                    None => self.rx[stream].from = origin.saturating_add(1),
                },
                WorkKind::Tune { stream, key } => {
                    self.tune(stream, key, tick);
                    continue;
                }
            }
            progressed = true;
        }
        self.settle_switches_to(u);
        if self.started {
            for k in 0..self.rx.len() {
                progressed |= self.emit_rx_until(k, u)?;
            }
            for k in 0..self.tx.len() {
                progressed |= self.emit_tx_until(k, u, None)?;
            }
            for k in 0..self.rx.len() {
                self.rx_settle(k, u)?;
            }
        }
        self.schedule_wakeup()?;
        Ok(progressed)
    }
}

fn rm25(error: ezsdr_kernel::time::TimeError) -> ModuleError {
    ModuleError::rejected(format!("RM-25: {error}"))
}

/// The key the instance's lines hold a stream under (RM-29).
fn stream_key(direction: Direction, k: usize) -> StreamKey {
    (direction, k as u32)
}

/// MR-34's order of the untimed tunes at one instant, among themselves: by stream number.
/// `advance` takes them after the receive segment beginnings, the faults and the held bursts of
/// the same instant (rank 2).
fn tune_order(k: usize) -> u64 {
    k as u64 + 1
}

fn num_config(config: &BTreeMap<ezsdr_kernel::spec::Key, Value>, name: &str) -> f64 {
    let key = ezsdr_kernel::spec::Key::parse(name).expect("radio key");
    match config.get(&key).and_then(Value::as_scalar) {
        Some(Scalar::Num(value)) => value.get(),
        Some(Scalar::Int(value)) => *value as f64,
        _ => 0.0,
    }
}

/// One of a MockRadio instance's seven Manifest sections, `ezsdr.radio.mock.<suffix>`; the
/// Kernel files it under the instance's source root (MR-27, RS-39).
fn section(suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.mock.{suffix}")).expect("module section")
}

impl Provider for MockRadio {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.profile.coerce(&self.instance.id, request)
    }

    fn prepare(&mut self, f: &Fragment, ctx: PrepareContext) -> Result<PrepareReport, ModuleError> {
        if self.prepare_called {
            return Err(self.reject("MR-7: prepare was already called"));
        }
        self.prepare_called = true;
        if ctx.class != ExecutionClass::Simulation {
            return Err(ModuleError {
                kind: ezsdr_kernel::module_api::ModuleErrorKind::Unsupported,
                message: "MR-7: MockRadio requires the Simulation execution class".to_owned(),
                detail: serde_json::Value::Null,
            });
        }
        let requested = f.content.get("requested").cloned().ok_or_else(|| self.reject("MR-7: fragment has no requested configuration"))?;
        let requested: Requested = serde_json::from_value(requested).map_err(|error| self.reject(format!("MR-7: invalid Requested: {error}")))?;
        let report = self.coerce(&requested)?;
        if !report.rejected.is_empty() {
            return Err(self.reject(format!("MR-7: rejected requests: {:?}", report.rejected)));
        }
        let mut config = self.profile.defaults().clone();
        for (key, value) in report.applied {
            if self.is_configuration(&key) {
                config.insert(key, value);
            }
        }
        if f.instance != module_ref() || f.role != Role::Provider {
            return Err(self.reject("MR-7: fragment identity or role does not match this Provider"));
        }
        let root = ctx.time.primary_root();
        let root_rate = ctx.clocks.nominal_rate(root).map_err(|error| self.reject(format!("MR-7: {error}")))?;
        if root_rate.den() != 1 {
            return Err(ModuleError { kind: ezsdr_kernel::module_api::ModuleErrorKind::Unsupported, message: "MR-7: primary root rate is not an integer".to_owned(), detail: serde_json::Value::Null });
        }
        // MR-7, RM-29: one SampleClock per stream that holds a channel, declared on that
        // stream's own node.
        for k in 0..self.rx.len() {
            let (channels, rate) = self.stream_set(Direction::Rx, k, &config);
            if channels.is_empty() {
                continue;
            }
            let ratio = self.rate_ratio(root_rate, rate, "MR-7")?;
            let node = self.rx[k].node.clone();
            self.rx[k].clock = Some(ctx.clocks.declare_sample_clock(node, root, ratio).map_err(|error| self.reject(format!("MR-7: {error}")))?);
        }
        for k in 0..self.tx.len() {
            let (channels, rate) = self.stream_set(Direction::Tx, k, &config);
            if channels.is_empty() {
                continue;
            }
            let ratio = self.rate_ratio(root_rate, rate, "MR-7")?;
            let node = self.tx[k].node.clone();
            // RM-15: the stream's record learns its closed short burst gap with its clock (the rate is fixed for the Run, RM-27).
            self.tx[k].book.record.arm(ezsdr_radio::staging::closed_short_gap(root_rate.num(), ratio.num(), ratio.den()));
            self.tx[k].clock = Some(ctx.clocks.declare_sample_clock(node, root, ratio).map_err(|error| self.reject(format!("MR-7: {error}")))?);
        }
        // MR-7, RM-29: every attached link is this fragment's, on one of its receive
        // streams' ports; a port's links are its own, so the one-Block-link rule holds for
        // each port.
        let mut links: Vec<Vec<std::sync::Arc<dyn DataLink>>> = vec![Vec::new(); self.rx.len()];
        for attached in &ctx.links {
            let Some(port) = self.rx.iter().position(|stream| stream.port == attached.port.as_str()) else {
                return Err(self.reject("MR-7: only this fragment's rx port may be attached"));
            };
            if attached.component != f.id {
                return Err(self.reject("MR-7: only this fragment's rx port may be attached"));
            }
            match &attached.endpoint {
                Endpoint::StreamOut(link) => links[port].push(link.clone()),
                _ => return Err(self.reject("MR-7: rx must attach to a StreamOut link")),
            }
        }
        for stream in &links {
            if stream.len() > 1 && stream.iter().any(|link| link.policy() == BackPressure::Block) {
                return Err(self.reject("MR-7: multiple rx links cannot use Block back-pressure"));
            }
        }
        let seed = ezsdr_sim::seed(&ctx.environment).map_err(|error| self.reject(format!("MR-7: {error}")))?;
        let mut faults = Vec::new();
        for (order, entry) in ezsdr_sim::faults(&ctx.environment).map_err(|error| self.reject(format!("MR-7: {error}")))?.into_iter().filter(|fault| fault.target == f.id).enumerate() {
            let offset = i64::try_from(entry.at_ns).map_err(|_| self.reject("MR-20: fault offset overflow"))?;
            let tick = time::ns_to_v(&ctx.clocks, root, offset).map_err(|error| self.reject(format!("MR-20: {error}")))?;
            faults.push(ScheduledFault { tick, entry, applied: false, resolved: false, order: order as u64 });
        }
        let channel_mode = match ezsdr_sim::channel::read(&ctx.environment).map_err(|error| self.reject(format!("MR-7: {error}")))? {
            None => None,
            Some(spec) => {
                let medium = self.medium.clone().ok_or_else(|| self.reject("MR-31: the environment declares sim.channel, but this instance has no medium"))?;
                if self.selector.rx_test_pattern != "zero" {
                    return Err(self.reject("MR-31: a channel-coupled Mock takes no receive test pattern"));
                }
                let (rx_max, tx_max) = (self.profile.channel_count(Direction::Rx) as i64, self.profile.channel_count(Direction::Tx) as i64);
                if let Some(coupling) = spec.couplings.iter().find(|c| (c.tx == f.id && i64::from(c.tx_channel) >= tx_max) || (c.rx == f.id && i64::from(c.rx_channel) >= rx_max)) {
                    let max = if coupling.rx == f.id { rx_max } else { tx_max };
                    return Err(self.reject(format!("MR-31: a coupling {} -> {} names a channel beyond this profile's {max}", coupling.tx, coupling.rx)));
                }
                let mut lo = SimRng::new(seed, &format!("{}/lo", self.selector_id()));
                let mut phases = |direction, count: usize| {
                    (0..count)
                        .map(|number| {
                            let (behavior, _) = self.profile.channel_model_terms(direction, number);
                            if behavior != "deterministic" { channel::lo_phase(&mut lo) } else { 0.0 }
                        })
                        .collect::<Vec<_>>()
                };
                let rx_phase = phases(Direction::Rx, rx_max as usize);
                let tx_phase = phases(Direction::Tx, tx_max as usize);
                let rx_channels = (0..rx_max as usize)
                    .map(|number| {
                        let side = "rx";
                        let (behavior, delay) = self.profile.channel_model_terms(Direction::Rx, number);
                        channel::RxChannelModel {
                            gain_db: channel::Timeline::new(num_config(&config, &ezsdr_radio::keys::channel(side, number as u32, "gain_db"))),
                            frequency_hz: channel::Timeline::new(num_config(&config, &ezsdr_radio::keys::channel(side, number as u32, "frequency_hz"))),
                            phase: channel::Timeline::new(rx_phase[number]),
                            phase_behavior: behavior.to_owned(),
                            path_delay_samples: delay,
                        }
                    })
                    .collect();
                let tx_channels = (0..tx_max as usize)
                    .map(|number| {
                        let side = "tx";
                        let (behavior, delay) = self.profile.channel_model_terms(Direction::Tx, number);
                        channel::ChannelModel {
                            gain_db: channel::Timeline::new(num_config(&config, &ezsdr_radio::keys::channel(side, number as u32, "gain_db"))),
                            frequency_hz: channel::Timeline::new(num_config(&config, &ezsdr_radio::keys::channel(side, number as u32, "frequency_hz"))),
                            phase: channel::Timeline::new(tx_phase[number]),
                            phase_behavior: behavior.to_owned(),
                            path_delay_samples: delay,
                        }
                    })
                    .collect();
                let tx = channel::TxShared::new(channel::TxPlan { segments: BTreeMap::new(), channel_models: tx_channels });
                let lo_retune = SimRng::new(seed, &format!("{}/lo/retune", self.selector_id()));
                // One transmitter covers the instance's logical channels; segment membership
                // selects the owning stream at each instant (MR-32).
                medium.join(&ctx.run, &f.id, &spec, seed, root_rate.num(), tx.clone()).map_err(|error| self.reject(format!("MR-31: {error}")))?;
                for stream in &mut self.tx {
                    stream.plan = Some(tx.clone());
                }
                Some(ChannelMode { medium, fragment: f.id.clone(), inputs: ctx.inputs.clone(), lo_retune, rx_channels })
            }
        };
        if channel_mode.is_some() {
            self.instance.fidelity.rf = ezsdr_kernel::module_api::RfFidelity::ImpairmentModel;
        }
        self.channel = channel_mode;
        self.root = Some(root);
        self.clocks = Some(ctx.clocks);
        self.time = Some(ctx.time);
        // MR-37: `resolve` is control-path only (RS-33), so the handles are taken here;
        // the view refuses a source outside this instance's root (item 3).
        let EventSource::Node { node: source_root } = ctx.source else {
            return Err(self.reject("KC-8: a Provider's event source root is a node"));
        };
        let overflow = EventKind::parse(ezsdr_radio::kinds::RX_OVERFLOW).expect("declared radio event kind");
        for (k, stream) in links.into_iter().enumerate() {
            let path = self.rx[k].path.clone();
            let rx_node = source_root.child(&path).map_err(|error| self.reject(format!("MR-37: {error}")))?;
            self.rx[k].overflow = Some(ctx.events.resolve(&EventSource::Node { node: rx_node }, &overflow).map_err(|error| self.reject(format!("MR-37: {error}")))?);
            self.rx[k].links = stream;
            self.rx[k].rng = Some(SimRng::new(seed, &format!("{}/{path}", self.selector_id())));
        }
        self.source_root = Some(source_root);
        self.events = Some(ctx.events);
        self.actions = Some(ctx.actions);
        self.faults = faults;
        self.next_order = self.faults.len() as u64;
        self.config = config.clone();
        self.staging = Staging::new(config.clone());
        self.prepared = true;
        Ok(PrepareReport { effective: config, coercions: report.coercions, warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        if !self.prepared {
            return Err(self.reject("MR-9: prepare must precede arm"));
        }
        if let Some(mode) = &self.channel {
            let missing = mode.medium.missing();
            if !missing.is_empty() {
                let names: Vec<&str> = missing.iter().map(Ident::as_str).collect();
                return Err(self.reject(format!("MR-31: the channel names {}, which did not join its medium", names.join(", "))));
            }
        }
        let root = self.root.expect("prepared root");
        let time = self.time.as_ref().expect("prepared time");
        let clocks = self.clocks.clone().expect("prepared clocks");
        let now = time.now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-9: {error}")))?;
        let latency = self.profile.timing().startup_latency_ns;
        self.sync_end =
            Some(now.checked_add(time::ns_to_v(&clocks, root, latency).map_err(|error| self.reject(format!("MR-9: {error}")))?).ok_or_else(|| self.reject("MR-9: synchronisation time overflow"))?);
        // RM-25, MR-9: each transmit stream's clock starts at the first lattice instant at
        // or after the arm instant (VE-4), and its line is booked.
        for k in 0..self.tx.len() {
            let stream = Self::tx_key(k);
            let (_, rate) = self.stream_set(Direction::Tx, k, &self.config);
            let ratio = self.ratio(rate)?;
            let origin = timeline::lattice(now, ratio).map_err(|error| self.reject(format!("MR-9: {error}")))?;
            let line = self.line(Direction::Tx, k, origin, &self.config)?;
            let by = line.plan.first().and_then(|segment| segment.by);
            if let Some(handle) = self.tx[k].clock.clone() {
                let domain = clocks.register_sample_clock(&handle, origin).map_err(|error| self.reject(format!("MR-9: {error}")))?;
                self.tx[k].registered = Some(TxClock { handle, domain, origin, by, ended: false });
            }
            let current = self.tx[k].registered.is_some();
            self.lines.insert(stream, line);
            if let Some(segment) = self.lines.line(stream).and_then(|line| line.plan.first()).copied() {
                if current {
                    self.record_layout_segment(Direction::Tx, k, segment.origin, segment.by, segment.config.channels);
                }
            }
            self.tx[k].tracker = self.tx_now(k).map(|clock| BurstTracker::new(clock.domain));
            self.tx[k].device = DeviceModel::new();
        }
        self.armed = true;
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        if !self.armed {
            return Err(self.reject("MR-11: arm must precede start"));
        }
        let root = self.root.expect("prepared root");
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        let time = self.time.as_ref().expect("prepared time").clone();
        let now = time.now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-11: {error}")))?;
        let target = match at {
            Some(at) => time::to_v(clocks, root, at).map_err(|error| self.reject(format!("MR-11: {error}")))?,
            None => now,
        };
        let sync_end = self.sync_end.unwrap_or(now);
        if target < sync_end {
            self.emit_event(
                "",
                ezsdr_radio::kinds::LATE_COMMAND,
                Severity::Warning,
                serde_json::to_value(ezsdr_radio::payloads::LateCommandPayload { key: None, requested: TimePoint::new(root, target), applied: TimePoint::new(root, sync_end) })
                    .expect("late command payload"),
                TimePoint::new(root, now),
            )?;
            return Err(
                self.reject(format!("MR-11: the start at {target} precedes synchronisation at {sync_end}; the profile needs ezsdr.time.start_lead_ns ≥ {}", self.profile.timing().startup_latency_ns))
            );
        }
        let fault_offsets: Vec<i64> = self.faults.iter().map(|fault| fault.tick).collect();
        let fault_ticks: Vec<i64> = fault_offsets.iter().map(|offset| target.checked_add(*offset).ok_or_else(|| ModuleError::rejected("MR-20: fault time overflow"))).collect::<Result<_, _>>()?;
        let segment_rows_before = self.instance.sections.get(&section("segments")).and_then(serde_json::Value::as_array).map(Vec::len);
        // RM-21, RM-25: every receive stream with a link runs from T0, its first segment's
        // clock registered at its first block.
        for k in 0..self.rx.len() {
            if self.rx[k].links.is_empty() {
                let line = self.line(Direction::Rx, k, target, &self.config)?;
                self.lines.insert(Self::rx_key(k), line);
                let seq = self.arrival();
                self.lines.book(Self::rx_key(k), Item { e: target, seq, ready: target, delivered: None, refused: false, kind: Kind::Stop }).map_err(rm25)?;
                continue;
            }
            let line = self.line(Direction::Rx, k, target, &self.config)?;
            let first = line.plan.first().copied();
            self.lines.insert(Self::rx_key(k), line);
            if let Some(segment) = first {
                self.rx_begin(k, segment, self.rx[k].clock.clone())?;
            }
        }
        for (fault, tick) in self.faults.iter_mut().zip(fault_ticks) {
            fault.tick = tick;
        }
        self.start_tick = Some(target);
        self.started = true;
        if let Err(error) = self.schedule_wakeup() {
            if let Some(handle) = self.wakeup.take() {
                time.cancel(handle);
            }
            self.started = false;
            self.start_tick = None;
            // A start that failed leaves no receive stream behind: the instance's lines are
            // rebuilt without the receive ones, and every transmit stream's line — inserted at
            // `arm`, before `start` — is put back. Nothing else is booked between `arm` and
            // `start`, so the rebuild drops nothing: a Session or Module Action is
            // refused unless the Run is Running (RS-18), and a scheduled UpdateParameter is
            // dispatched only after every `start` returns (KC-14, KC-17, KC-18).
            let tx_lines: Vec<(usize, Line)> = (0..self.tx.len()).filter_map(|k| self.lines.line(Self::tx_key(k)).cloned().map(|line| (k, line))).collect();
            self.lines = timeline::Lines::new();
            for (k, line) in tx_lines {
                self.lines.insert(Self::tx_key(k), line);
            }
            for k in 0..self.rx.len() {
                self.rx[k].running = None;
                self.rx[k].from = i64::MIN;
                self.rx[k].pool = None;
            }
            if let (Some(length), Some(serde_json::Value::Array(rows))) = (segment_rows_before, self.instance.sections.get_mut(&section("segments"))) {
                rows.truncate(length);
            }
            for (fault, offset) in self.faults.iter_mut().zip(fault_offsets) {
                fault.tick = offset;
            }
            return Err(error);
        }
        Ok(())
    }

    fn stop(&mut self, mode: StopMode) -> Result<(), ModuleError> {
        let root = self.root.ok_or_else(|| self.reject("MR-25: not prepared"))?;
        let now = self.time.as_ref().expect("prepared time").now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-25: {error}")))?;
        // What is due by now first, the stop then at now (RM-16, RM-25).
        if self.started && !self.device_lost_reported {
            match self.advance(now) {
                Err(error) if error.kind == ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost => {}
                other => {
                    other?;
                }
            }
        }
        if self.device_lost_reported {
            // MR-20: the device stopped at its loss, as an `abort` there, and `step` is gated
            // from then on, so nothing is published after it whatever `stop` does; `stop`
            // only refuses an Action still queued, having nothing left to stop.
            self.refuse_actions(now);
            return Ok(());
        }
        let seq = self.arrival();
        self.stop_at(mode, now, "MR-25: cancelled by stop", seq, ezsdr_radio::CancelReason::ProviderStop)
    }

    fn cleanup(&mut self) {
        if self.root.is_some() {
            self.record_pending_faults();
        }
        if let (Some(time), Some(handle)) = (&self.time, self.wakeup.take()) {
            time.cancel(handle);
        }
        self.started = false;
        self.armed = false;
        self.prepared = false;
        self.actions = None;
        self.lines = timeline::Lines::new();
        let (rx, tx) = Self::streams(&self.profile, &self.instance.id);
        self.rx = rx;
        self.tx = tx;
        self.wakeup = None;
        self.faults.clear();
        self.clocks = None;
        self.time = None;
        self.events = None;
        self.source_root = None;
        self.root = None;
        self.channel = None;
        self.medium = None;
    }

    fn step(&mut self, until: TimePoint) -> Result<StepOutcome, ModuleError> {
        if !self.prepared {
            return Ok(StepOutcome { progressed: false });
        }
        let root = self.root.ok_or_else(|| self.reject("MR-12: not prepared"))?;
        let u = time::to_v(self.clocks.as_ref().expect("prepared clocks"), root, until).map_err(|error| self.reject(format!("MR-12: {error}")))?;
        if self.device_lost_reported {
            // MR-20: a lost device carries out nothing; what reaches it is refused.
            self.refuse_actions(u);
            return Ok(StepOutcome { progressed: false });
        }
        let mut progressed = false;
        // MR-20: in the step that applies a loss, the Actions reach the device at `u`, at or
        // after the loss, so none is carried out; the loss itself is applied below, in order.
        let loss_due = self.started && self.faults.iter().any(|fault| !fault.resolved && fault.entry.fault == FaultKind::DeviceLost && fault.tick <= u);
        if loss_due {
            self.refuse_actions(u);
        } else if let Some(actions) = self.actions.clone() {
            while let Some(dispatched) = actions.recv() {
                self.handle_action(dispatched, u)?;
                progressed = true;
            }
        }
        progressed |= self.advance(u)?;
        Ok(StepOutcome { progressed })
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    fn held(start: i64) -> HeldBurst {
        HeldBurst { start, len: 1, repeat: false, switch: false, id: ActionId(start as u64), open: BurstOpen::default(), order: start as u64, values: Vec::new() }
    }

    fn radio_with_reservations() -> (MockRadio, std::sync::Arc<channel::TxShared>, std::sync::Arc<ezsdr_kernel::event::EventCollector>) {
        let binding = Binding {
            module: module_ref(),
            selector: BTreeMap::from([
                (Ident::parse("id").unwrap(), Value::from("radio")),
                (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
                (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0")])),
            ]),
            profile: Some(x3xx::profile_ref()),
            feed: None,
        };
        let mut radio = MockRadio::from_binding(&binding).unwrap();
        let root = ClockDomainId::local(80);
        let source_root = ResourceId::parse("radio").unwrap();
        let clocks = std::sync::Arc::new(ClockRegistry::new());
        clocks
            .register(ezsdr_kernel::time::ClockDomain::root(root, Rational::new(200_000_000, 1).unwrap(), ezsdr_kernel::time::EpochRef::Arbitrary { set_by: "cancellation-test".to_owned() }))
            .unwrap();
        let stream = ResourceId::parse("radio/tx/s0").unwrap();
        let mut segments = BTreeMap::new();
        let (origin, by, start) = (0, None, 20);
        let handle = clocks.declare_sample_clock(stream.clone(), root, Rational::new(200, 1).unwrap()).unwrap();
        let domain = clocks.register_sample_clock(&handle, origin).unwrap();
        let registered = TxClock { handle, domain, origin, by, ended: false };
        segments.insert(
            (origin, by, 0, start),
            channel::Segment { origin, ratio: Rational::new(200, 1).unwrap(), start, len: 1, repeat: false, channels: 1, channel_numbers: vec![0], samples: vec![0.0f32, 0.0].into(), end: None },
        );
        let shared = channel::TxShared::new(channel::TxPlan { segments, channel_models: Vec::new() });
        let (mut registry, mut admission, mut kinds) =
            (ezsdr_kernel::module_api::ModuleRegistry::new(), ezsdr_kernel::binding::AdmissionCheckRegistry::new(), ezsdr_kernel::policy::EventKindRegistry::with_kernel_kinds());
        ezsdr_radio::register(&mut registry, &mut admission, &mut kinds).unwrap();
        let event_kinds = kinds.kinds();
        let source = EventSource::Node { node: source_root.clone() };
        let pairs: Vec<_> = event_kinds.iter().cloned().map(|kind| (source.clone(), kind)).collect();
        let events = std::sync::Arc::new(ezsdr_kernel::event::EventCollector::new(&pairs, &event_kinds, 4096, &ezsdr_kernel::policy::Policy::default()));
        radio.root = Some(root);
        radio.source_root = Some(source_root);
        radio.clocks = Some(clocks);
        radio.events = Some(events.clone());
        radio.tx[0].clock = Some(registered.handle.clone());
        radio.tx[0].tracker = Some(BurstTracker::new(registered.domain));
        // As production arms a stream's record where its clock is registered: 1 MS/s on this 200 MHz root (RM-15).
        radio.tx[0].book.record.arm(ezsdr_radio::staging::closed_short_gap(200_000_000, 200, 1));
        radio.tx[0].registered = Some(registered);
        radio.tx[0].plan = Some(shared.clone());
        radio.tx[0].book.list.held.insert(20, held(20));
        (radio, shared, events)
    }

    #[test]
    fn a_cancelled_held_burst_releases_the_shared_reservation() {
        let (mut radio, shared, events) = radio_with_reservations();
        radio.stop_tx(0, 0, "MR-25: cancelled by stop", ezsdr_radio::CancelReason::Stop).unwrap();
        assert!(radio.tx[0].book.list.held.is_empty());
        assert!(shared.plan().segments.is_empty());
        let rows = radio.instance.sections[&section("rejected")].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows.iter().all(|row| row["reason"] == "MR-25: cancelled by stop"), "{rows:?}");
        assert!(events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0)).iter().all(|event| event.kind.as_str() != ezsdr_radio::kinds::COMMAND_REJECTED));
    }

    #[test]
    fn a_loss_cuts_the_open_burst_at_its_instant() {
        let (mut radio, _, _) = radio_with_reservations();
        radio.tx[0].book.open =
            Some(ezsdr_radio::staging::Open::new(HeldBurst { start: 20, len: 1, repeat: true, switch: false, id: ActionId(1), open: BurstOpen::default(), order: 1, values: Vec::new() }, 25, false));
        radio.stop_tx(0, 0, "test termination", ezsdr_radio::CancelReason::Loss).unwrap();
        assert!(radio.tx[0].book.open.is_none(), "a loss must cut at its instant");
    }
}

#[cfg(test)]
mod payload_cut_tests {
    use super::handed_front;
    use ezsdr_radio::segment::Pending;

    #[test]
    fn mr_17_handed_front_reduction_cases() {
        let finite = |start, len| Pending { start, len, repeat: false, open: false };
        let open_repeat = Pending { start: 9, len: 1, repeat: true, open: true };
        assert_eq!(handed_front([(1, finite(0, 3)), (2, finite(0, 8))], 3, 10), Some(8), "the front is the greatest candidate");
        assert_eq!(handed_front([(1, finite(0, 8)), (2, finite(0, 3))], 3, 10), Some(8), "whatever the order of the candidates");
        assert_eq!(handed_front([(1, finite(0, 3)), (2, open_repeat)], 3, 10), Some(10), "a handed repeat reaches the horizon");
        assert_eq!(handed_front([(4, open_repeat)], 5, 10), Some(10), "an earlier same-instant receipt counts");
        assert_eq!(handed_front([(5, open_repeat)], 5, 10), None, "the current receipt is excluded");
        assert_eq!(handed_front([(6, open_repeat), (1, finite(20, 5))], 5, 10), None, "a later receipt is excluded, and a burst past the horizon is not handed");
    }
}
