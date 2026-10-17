//! Ez-SDR v4 Module ezsdr.radio.mock 2.0.0 (design/09-mock-radio.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod channel;
mod profile;
mod time;
mod device;

use std::collections::BTreeMap;

use ezsdr_kernel::binding::Binding;
use ezsdr_kernel::event::{Action, ActionId, Event, EventHandle, EventKind, EventSink, EventSource, Severity};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId};
use ezsdr_kernel::module_api::{
    ActionReceiver, CoerceReport, Dispatched, Driving, Endpoint, ExecutionClass, KERNEL_API, ModuleDescriptor,
    ModuleError, ModuleRef, PrepareContext, Provider, ProviderInstance, Role, StepOutcome,
    Requested, StopMode, Version, VersionReq, VocabularyRequirement,
};
use ezsdr_kernel::plan::{Fragment, PrepareReport};
use ezsdr_kernel::spec::{Ident, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, BlockFlags, BlockHeader, BurstOpen, BurstRecord, BurstStep, BurstTracker, ChannelMask, DataLink, Direction, LateOutcome, PublishOutcome, SampleBlock};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockRegistry, Duration, Rational, SampleClockHandle, ScheduleHandle, TimeAuthority, TimePoint};
use ezsdr_kernel::module_api::UpdateClass;
use ezsdr_hostmem::{HostPool, HOST_MEMORY, write_cf32};
use ezsdr_sim::{FaultEntry, FaultKind, SimRng};
use ezsdr_sim::channel::{Medium, RootInstant};
use ezsdr_kernel::module_api::InputStore;
use ezsdr_radio::timeline::{self, Item, Kind, Line, Segment, StreamKey, NO_RELEASE};
use ezsdr_radio::x3xx;

pub use device::{DeviceModel, DeviceTimeError};
pub use profile::{Profile, ProfileKind};

/// The reason a lost device gives for everything it cancels or refuses (MR-20).
const DEVICE_LOST: &str = "MR-20: device lost";

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.radio.mock").expect("valid module id"),
        version: Version::new(2, 0, 0),
    }
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
            .map(|(id, version)| VocabularyRequirement {
                id: Namespace::parse(id).expect("valid vocabulary"),
                req: VersionReq(version),
            })
            .collect(),
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.radio.mock 2.0.0")),
    }
}

/// Deterministic in-process radio Provider (MR-1…MR-30).
///
/// An instance owns its streams: the receive and the transmit state that belongs to a
/// stream lives in [`RxStream`] and [`TxStream`], one entry per stream number `k` (RM-29),
/// and their timelines live together in [`timeline::Lines`] so that a move can hold two of
/// them at once (RM-34). What belongs to the instance — its profile, configuration,
/// updates, fault schedule, wakeup and its one SimulationChannel — stays here.
pub struct MockRadio {
    instance: ProviderInstance,
    /// The root node of this instance's event sources, as `prepare`'s context names it
    /// (KC-8); sub-nodes are named below it.
    source_root: Option<ResourceId>,
    profile: Profile,
    config: BTreeMap<ezsdr_kernel::spec::Key, Value>,
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
    /// (RM-26, RM-34); a stream's line is absent until `start` and `arm` book it.
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
    updates: BTreeMap<(i64, u64), PendingUpdate>,
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
    channels: u16,
    next: i64,
    planned: Option<i64>,
    flags: BlockFlags,
    lost: Option<u64>,
    /// Each loss's `RX_OVERFLOW`, at `k_f`, for the block that carries it (RM-17, MR-37).
    overflows: Vec<(i64, ezsdr_radio::payloads::RxOverflowPayload)>,
    /// Its SampleClock has ended.
    ended: bool,
}

impl Rx {
    fn config(&self) -> timeline::Config {
        timeline::Config { channels: timeline::Channels::first(u32::from(self.channels)), ratio: self.ratio }
    }
}

/// One receive stream of an instance (RM-29, MR-10): its node and port, the SampleClock it
/// was declared with, the segment it is delivering, the links its port publishes to, and —
/// in channel mode — what it models of the channel (MR-7, MR-12, MR-25, MR-33…MR-35).
struct RxStream {
    /// Its node, `<device>/rx` (MR-10, RM-29): the stream its SampleClock is declared for
    /// (RM-7) and the node a `Stop` or `start_rx` names (RM-21).
    node: ResourceId,
    /// The name of the port its links attach to (MR-7).
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
    /// `block_len_jitter`'s generator, `<device>/rx` (MR-12).
    rng: Option<SimRng>,
    /// Channel mode's receive parameters (MR-33…MR-35).
    model: Option<channel::RxModel>,
}

impl RxStream {
    fn new(node: ResourceId) -> RxStream {
        RxStream {
            node,
            port: "rx".to_owned(),
            overflow: None,
            clock: None,
            from: i64::MIN,
            running: None,
            links: Vec::new(),
            pool: None,
            rng: None,
            model: None,
        }
    }
}

/// One transmit stream of an instance (RM-29, MR-10): its node, the clocks its plan has
/// registered, the bursts booked on them, and the tracker and device model of the clock
/// that is current (MR-15, MR-18, MR-25).
struct TxStream {
    /// Its node, `<device>/tx` (MR-10, RM-29): the stream its SampleClock is declared for
    /// (RM-7) and the node a `TxBurst` names (RM-15).
    node: ResourceId,
    /// The transmit SampleClock declared in `prepare`, registered at `arm` (MR-9).
    clock: Option<SampleClockHandle>,
    /// Every transmit clock registered, in the order of the transmit plan's segments.
    clocks: Vec<TxClock>,
    /// The index in `clocks` of the current transmit clock.
    cur: Option<usize>,
    /// Bursts booked on a transmit clock a pending change starts, by its index (KC-21a).
    later: Vec<(usize, HeldBurst)>,
    /// The current clock's tracker (MR-15).
    tracker: Option<BurstTracker>,
    /// The current clock's device framing model (MR-15, MR-24).
    device: DeviceModel,
    /// The bursts booked on the current clock, by their first sample (RM-15).
    held: BTreeMap<i64, HeldBurst>,
    /// The burst being transmitted (MR-15).
    open: Option<OpenBurst>,
    /// Channel mode's transmit plan, which the medium reads (MR-32).
    plan: Option<std::sync::Arc<channel::TxShared>>,
}

impl TxStream {
    fn new(node: ResourceId) -> TxStream {
        TxStream {
            node,
            clock: None,
            clocks: Vec::new(),
            cur: None,
            later: Vec::new(),
            tracker: None,
            device: DeviceModel::new(),
            held: BTreeMap::new(),
            open: None,
            plan: None,
        }
    }
}

/// A registered transmit clock.
struct TxClock {
    handle: SampleClockHandle,
    domain: ClockDomainId,
    origin: i64,
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

struct HeldBurst {
    start: i64,
    len: i64,
    repeat: bool,
    open: BurstOpen,
    order: u64,
}

struct OpenBurst {
    start: i64,
    len: i64,
    repeat: bool,
    next: i64,
    first: bool,
    open: BurstOpen,
}

struct PendingUpdate {
    key: ezsdr_kernel::spec::Key,
    /// The value `coerce` applied when it was booked (MR-18).
    value: Value,
    class: UpdateClass,
    /// A transmit `cold` change: the transmit clock it starts, and how many clocks the
    /// transmit plan held when it was booked (RM-25).
    tx: Option<(Option<usize>, usize)>,
}

#[derive(Clone, Copy)]
enum WorkKind {
    Fault { index: usize },
    HeldBurst { stream: usize, start: i64 },
    Update { effective: i64 },
    RxBegin { stream: usize, origin: i64 },
}

impl MockRadio {
    /// The instance's streams: a receive and a transmit stream each, `<device>/rx` and
    /// `<device>/tx` (RM-29, MR-10). Every direction has one stream; a stream whose
    /// direction has no channel holds no channel, declares no SampleClock and publishes no
    /// block (MR-3, MR-5).
    fn streams(id: &ResourceId) -> (Vec<RxStream>, Vec<TxStream>) {
        let node = |name: &str| id.child(name).expect("a valid stream id");
        (vec![RxStream::new(node("rx"))], vec![TxStream::new(node("tx"))])
    }

    /// The key the instance's lines hold receive stream `k` under (RM-29, RM-34).
    fn rx_key(k: usize) -> StreamKey {
        (Direction::Rx, k as u32)
    }

    /// The key the instance's lines hold transmit stream `k` under (RM-29, RM-34).
    fn tx_key(k: usize) -> StreamKey {
        (Direction::Tx, k as u32)
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
        // The shared keys of design 33 D5 are the `x3xx` layout and its companions; `ideal`
        // has no layout at this stage, so its binding may not carry them (design 33 D5).
        let allowed: Vec<&str> = ["id", "block_len_jitter", "rx_test_pattern", "arm_after"]
            .into_iter()
            .chain(x3xx::SELECTOR_KEYS.into_iter().filter(|_| kind == ProfileKind::X3xx))
            .collect();
        if let Some(name) = binding
            .selector
            .keys()
            .find(|name| !allowed.contains(&name.as_str()))
        {
            return Err(reject(&format!("unsupported selector key `{name}`")));
        }
        let selector_value = |name: &str| binding.selector.get(&Ident::parse(name).expect("selector key"));
        let id = match selector_value("id") {
            None => ResourceId::parse("mock").expect("default resource id"),
            Some(Value::Scalar(Scalar::Str(raw))) => ResourceId::parse(raw)
                .ok()
                .filter(|id| id.segments().count() == 1)
                .ok_or_else(|| reject("selector `id` must be one resource path segment"))?,
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
                    Scalar::Str(raw) => ResourceId::parse(raw)
                        .map_err(|_| reject("selector `arm_after` contains an invalid resource id")),
                    _ => Err(reject("selector `arm_after` must contain resource path strings")),
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(reject("selector `arm_after` must be a list")),
        };
        let selection = match kind {
            ProfileKind::X3xx => Some(x3xx::from_selector(&binding.selector).map_err(|why| reject(&why))?),
            ProfileKind::Ideal => None,
        };
        let profile = Profile::new(kind, selection);
        let timing = profile.timing();
        let mut sections = BTreeMap::new();
        sections.insert(section("envelope"), serde_json::to_value(profile.envelope()).expect("serializable envelope"));
        sections.insert(section("bursts"), serde_json::json!([]));
        sections.insert(section("faults"), serde_json::json!([]));
        sections.insert(section("rejected"), serde_json::json!([]));
        sections.insert(section("stats"), serde_json::json!({"rx_blocks": 0, "rx_samples": 0, "tx_blocks": 0, "rx_clipped": 0, "tx_clipped": 0}));
        sections.insert(section("applied"), serde_json::json!([]));
        let instance = ProviderInstance {
            id: id.clone(),
            module: module_ref(),
            profile: binding.profile.clone(),
            tree: profile.description().tree(&id, HOST_MEMORY),
            fidelity: profile.fidelity(),
            driving: Driving { stepped: true },
            arm_after,
            min_command_lead: (timing.min_timed_command_lead_ns > 0).then(|| {
                Duration::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, timing.min_timed_command_lead_ns)
            }),
            sections,
        };
        let (rx, tx) = MockRadio::streams(&id);
        let defaults = profile.description().defaults.clone();
        Ok(MockRadio {
            instance,
            source_root: None,
            profile,
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
            updates: BTreeMap::new(),
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

    fn effective_channels(&self, key: &str) -> i64 {
        int_config(&self.config, key)
    }

    fn emit_event(&self, suffix: &str, kind: &str, severity: Severity, payload: serde_json::Value, at: TimePoint) -> Result<(), ModuleError> {
        let (Some(events), Some(_root), Some(root)) = (&self.events, self.root, &self.source_root) else { return Ok(()); };
        let node = if suffix.is_empty() { root.clone() } else { root.child(suffix).unwrap_or_else(|_| root.clone()) };
        let event = Event {
            source: EventSource::Node { node },
            time: at,
            severity,
            kind: EventKind::parse(kind).expect("declared radio event kind"),
            payload,
        };
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
        let (Some(events), Some(handle)) = (&self.events, self.rx[k].overflow) else { return Ok(()); };
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
        if let Some(rx) = self.rx[k].running.as_mut() { rx.planned = Some(len); }
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
            let fault = self.faults.iter().filter(|fault| !fault.resolved).map(|fault| fault.tick).min();
            let fault_cut = fault.map(|tick| time::k_at_or_after(origin, ratio, tick)
                .ok_or_else(|| self.reject("MR-12: receive cut overflow"))).transpose()?;
            if fault_cut.is_some_and(|fault_cut| fault_cut <= next) { break; }
            self.plan_rx_block(k);
            let Some(planned) = self.rx[k].running.as_ref().and_then(|rx| rx.planned) else { break; };
            let mut stop = next.checked_add(planned).ok_or_else(|| self.reject("MR-12: sample index overflow"))?;
            for bound in [fault_cut, cut].into_iter().flatten() { stop = stop.min(bound); }
            let after = time::v_after(origin, ratio, stop - 1).ok_or_else(|| self.reject("MR-14: block time overflow"))?;
            if after > until { break; }
            let (flags, lost, channels) = self.rx[k].running.as_ref().map(|rx| (rx.flags, rx.lost, rx.channels)).expect("a receive stream");
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
                self.emit_rx_overflow(k, domain, next, next_sample - next, self.profile.timing().overflow_restart_gap_ns)?;
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
        if self.rx[k].pool.is_none() { return Err(self.reject("MR-13: receive pool is absent")); }
        let (origin, ratio) = self.rx[k].running.as_ref().map(|rx| (rx.origin, rx.ratio)).expect("a receive stream publishes");
        let mut clipped = 0u64;
        let received: Option<Vec<(f32, f32)>> = self.rx[k].model.as_ref().zip(self.channel.as_ref()).map(|(model, mode)| {
            let mut out = Vec::with_capacity(channels as usize * len as usize);
            for c in 0..channels {
                let phase = model.phase.get(usize::from(c));
                for sample in first..stop {
                    let stamp = RootInstant::of_sample(origin, ratio, sample);
                    let (fr, fi) = mode.medium.field(&mode.fragment, c, model.frequency_hz.at(stamp), stamp);
                    let amplitude = 10f64.powf(model.gain_db.at(stamp) / 20.0);
                    let (sin, cos) = phase.map_or(0.0, |p| p.at(stamp)).sin_cos();
                    let re = amplitude * (fr * cos + fi * sin);
                    let im = amplitude * (fi * cos - fr * sin);
                    let (cre, cim) = (re.clamp(-1.0, 1.0), im.clamp(-1.0, 1.0));
                    if cre != re || cim != im { clipped += 1; }
                    out.push((cre as f32, cim as f32));
                }
            }
            out
        });
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
            if link.publish(block.clone()) == PublishOutcome::Full { full = true; }
        }
        // RM-17, MR-37: each loss's RX_OVERFLOW comes with the block that carries it.
        let overflows = self.rx[k].running.as_mut().map(|rx| std::mem::take(&mut rx.overflows)).unwrap_or_default();
        for (kf, payload) in overflows {
            self.emit_overflow(k, payload, TimePoint::new(domain, kf))?;
        }
        Ok(full)
    }

    fn emit_rx_overflow(&mut self, k: usize, domain: ClockDomainId, at: i64, lost: i64, gap_ns: i64) -> Result<(), ModuleError> {
        let payload = ezsdr_radio::payloads::RxOverflowPayload {
            cause: ezsdr_radio::payloads::RxOverflowCause::Overrun,
            lost: lost.max(0) as u64,
            restart_gap_ns: gap_ns,
        };
        self.emit_overflow(k, payload, TimePoint::new(domain, at))
    }

    /// MR-21, MR-22: an overrun or a sequence error at `f`, on the receive stream `k`'s
    /// segment running then; its row is written now and never revised, its event comes with
    /// the block that carries its loss, if any (MR-20; spec 22, VH-5).
    fn apply_rx_fault(&mut self, k: usize, index: usize) -> Result<(), ModuleError> {
        if self.faults[index].resolved { return Ok(()); }
        self.faults[index].resolved = true;
        let (f, seq) = (self.faults[index].tick, self.faults[index].order);
        let runs = match (self.rx[k].running.as_ref(), self.lines.line(Self::rx_key(k))) {
            (Some(rx), Some(line)) => f >= rx.origin && line.made(rx.origin, rx.config(), f, seq).map_err(rm26)?.is_some_and(|segment| segment.cut.is_none()),
            _ => false,
        };
        let Some(rx) = self.rx[k].running.as_ref().filter(|_| runs) else {
            self.record_fault(index, 0);
            return Ok(());
        };
        let (origin, ratio, next) = (rx.origin, rx.ratio, rx.next);
        let kf = time::k_at_or_after(origin, ratio, f).unwrap_or(next).max(next);
        let (kg, cause, gap_ns) = match self.faults[index].entry.fault {
            FaultKind::RxOverflow => {
                let gap_ns = self.profile.timing().overflow_restart_gap_ns;
                let gap = time::ns_to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), gap_ns)
                    .map_err(|error| self.reject(format!("MR-21: {error}")))?;
                let target = f.checked_add(gap).ok_or_else(|| self.reject("MR-21: restart time overflow"))?;
                (time::k_at_or_after(origin, ratio, target).unwrap_or(kf).max(kf), ezsdr_radio::payloads::RxOverflowCause::Overrun, gap_ns)
            }
            FaultKind::RxSequenceError => (kf.saturating_add(i64::from(self.profile.block_len())), ezsdr_radio::payloads::RxOverflowCause::Sequence, 0),
            FaultKind::DeviceLost => unreachable!("a loss ends the device (MR-20)"),
        };
        let lost = kg - kf;
        if let Some(rx) = self.rx[k].running.as_mut() {
            rx.next = kg;
            rx.planned = None;
            if lost > 0 {
                rx.flags = rx.flags | BlockFlags::GAP_BEFORE | match cause {
                    ezsdr_radio::payloads::RxOverflowCause::Overrun => BlockFlags::RESTARTED,
                    ezsdr_radio::payloads::RxOverflowCause::Sequence => BlockFlags::SEQ_DISCONTINUITY,
                };
                rx.lost = Some(rx.lost.unwrap_or(0).saturating_add(lost as u64));
                rx.overflows.push((kf, ezsdr_radio::payloads::RxOverflowPayload { cause, lost: lost as u64, restart_gap_ns: gap_ns }));
            }
        }
        self.faults[index].applied = true;
        self.record_fault(index, lost as u64);
        Ok(())
    }

    fn record_fault(&mut self, index: usize, lost: u64) {
        #[derive(serde::Serialize)]
        struct Record { at: TimePoint, fault: FaultKind, applied: bool, lost: u64 }
        let fault = &self.faults[index];
        let record = Record { at: TimePoint::new(self.root.expect("prepared root"), fault.tick), fault: fault.entry.fault, applied: fault.applied, lost };
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("faults")) {
            rows.push(serde_json::to_value(record).expect("fault section row"));
        }
    }

    fn record_pending_faults(&mut self) {
        for index in 0..self.faults.len() {
            if !self.faults[index].resolved {
                self.faults[index].resolved = true;
                self.record_fault(index, 0);
            }
        }
    }

    fn schedule_wakeup(&mut self) -> Result<(), ModuleError> {
        let (Some(time), Some(root)) = (self.time.clone(), self.root) else { return Ok(()); };
        let now = time.now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-14: {error}")))?;
        let mut next: Option<i64> = None;
        let mut wake = |tick: i64| next = Some(next.map_or(tick, |next| next.min(tick)));
        for (tick, _) in self.updates.keys() { wake(*tick); }
        if self.started {
            self.faults.iter().filter(|fault| !fault.resolved).for_each(|fault| wake(fault.tick));
            for k in 0..self.rx.len() {
                if let Some(segment) = self.rx_next_segment(k) { wake(segment.origin); }
            }
        }
        for k in 0..self.rx.len() {
            let cut = self.rx_cut(k);
            if self.rx[k].running.as_ref().is_some_and(|rx| cut.is_none_or(|cut| rx.next < cut)) {
                let fault = self.faults.iter().filter(|fault| !fault.resolved).map(|fault| fault.tick).min();
                let rx = self.rx[k].running.as_ref().expect("a receive stream");
                let fault_cut = fault.and_then(|tick| time::k_at_or_after(rx.origin, rx.ratio, tick));
                if fault_cut.is_none_or(|fault_cut| fault_cut > rx.next) { self.plan_rx_block(k); }
                if let Some(rx) = self.rx[k].running.as_ref() {
                    if let Some(planned) = rx.planned {
                        let stop = [fault_cut, cut].into_iter().flatten().fold(rx.next.saturating_add(planned), i64::min);
                        if stop > rx.next {
                            if let Some(tick) = time::v_after(rx.origin, rx.ratio, stop - 1) { wake(tick); }
                        }
                    }
                }
            }
        }
        for k in 0..self.tx.len() {
            if let (Some(open), Some(clock)) = (self.tx[k].open.as_ref(), self.tx_now(k)) {
                let (handle, origin) = (&clock.handle, clock.origin);
                let block_end = open.next.saturating_add(i64::from(self.profile.block_len()));
                let waveform_end = if open.repeat {
                    open.start.saturating_add(((open.next - open.start) / open.len + 1).saturating_mul(open.len))
                } else { open.start.saturating_add(open.len) };
                let held_cut = self.tx[k].held.keys().next().copied().unwrap_or(i64::MAX);
                let cut = held_cut.min(self.tx_cut(k).unwrap_or(i64::MAX));
                let stop = block_end.min(waveform_end).min(cut);
                if stop > open.next {
                    if let Some(tick) = time::v_after(origin, handle.root_ticks_per_tick(), stop - 1) { wake(tick); }
                }
            }
            if let Some(clock) = self.tx_now(k) {
                let (handle, origin) = (&clock.handle, clock.origin);
                if let Some(start) = self.tx[k].held.keys().next().copied() {
                    if let Some(tick) = time::v_of(origin, handle.root_ticks_per_tick(), start) { wake(tick); }
                }
            }
        }
        let wanted = next.filter(|tick| *tick > now);
        if self.wakeup.is_some_and(|handle| wanted == Some(handle.ticks)) { return Ok(()); }
        if let Some(handle) = self.wakeup.take() { time.cancel(handle); }
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
        struct Rejected { kind: String, reason: String, at: TimePoint }
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
        if n == 0 { return; }
        if let Some(serde_json::Value::Object(stats)) = self.instance.sections.get_mut(&section("stats")) {
            let total = stats.get(name).and_then(serde_json::Value::as_u64).unwrap_or(0);
            stats.insert(name.to_owned(), serde_json::json!(total + n));
        }
    }

    /// Cuts every burst of transmit stream `k`'s current clock at sample `k_cut` and
    /// withdraws the bursts that start at or after it, as a stop or a cold change does
    /// (MR-32).
    fn cut_segments(&mut self, k: usize, k_cut: i64) {
        let generation = self.tx_gen(k);
        let Some(shared) = self.tx[k].plan.clone() else { return };
        let mut plan = shared.plan();
        plan.segments.retain(|(g, start), _| *g != generation || *start < k_cut);
        for ((g, _), segment) in plan.segments.iter_mut() {
            if *g == generation {
                segment.end = Some(segment.end.map_or(k_cut, |end| end.min(k_cut)));
            }
        }
    }

    /// Ends the open burst's radiation at transmit stream `k`'s sample `stop` (MR-32).
    fn end_open_segment(&mut self, k: usize, stop: i64) {
        let Some(shared) = self.tx[k].plan.clone() else { return };
        let Some(open) = self.tx[k].open.as_ref() else { return };
        if let Some(segment) = shared.plan().segments.get_mut(&(self.tx_gen(k), open.start)) {
            segment.end = Some(segment.end.map_or(stop, |end| end.min(stop)));
        }
    }

    /// Transmits every sample of stream `k` before `cut`: the open burst up to it, and every
    /// held burst that starts before it, opened in turn (MR-25, MR-32).
    fn transmit_until_cut(&mut self, k: usize, now: i64, cut: i64) -> Result<(), ModuleError> {
        loop {
            if self.tx[k].open.is_some() {
                self.emit_tx_until(k, now, Some(cut))?;
            }
            if self.tx[k].open.is_some() { break; }
            let Some(start) = self.tx[k].held.keys().next().copied().filter(|start| *start < cut) else { break };
            let held = self.tx[k].held.remove(&start).expect("the first held burst");
            self.tx[k].open = Some(OpenBurst { start: held.start, len: held.len, repeat: held.repeat, next: held.start, first: true, open: held.open });
        }
        Ok(())
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

    fn emit_late_burst(&self, late_by: Duration, target: TimePoint, now_tx: TimePoint, outcome: ezsdr_radio::payloads::TimeErrorOutcome) -> Result<(), ModuleError> {
        let late_by_ns = time::to_ns(self.clocks.as_ref().expect("prepared clocks"), late_by).unwrap_or(i64::MAX);
        let payload = serde_json::to_value(ezsdr_radio::payloads::TimeErrorPayload { cause: ezsdr_radio::payloads::TimeErrorCause::Late, outcome, late_by_ns, target }).expect("time error payload");
        self.emit_event("tx", ezsdr_radio::kinds::TIME_ERROR, Severity::Error, payload, now_tx)
    }

    fn record_update(&mut self, key: &ezsdr_kernel::spec::Key, value: &Value, at: i64) {
        #[derive(serde::Serialize)]
        struct Applied<'a> { key: &'a ezsdr_kernel::spec::Key, value: &'a Value, at: TimePoint }
        let row = Applied { key, value, at: TimePoint::new(self.root.expect("root"), at) };
        if let Some(serde_json::Value::Array(rows)) = self.instance.sections.get_mut(&section("applied")) {
            rows.push(serde_json::to_value(row).expect("applied row"));
        }
    }

    fn stop_tx(&mut self, k: usize, now: i64, reason: &str, emit_command_rejected: bool) -> Result<(), ModuleError> {
        if let Some((ratio, origin)) = self.tx_now(k).map(|clock| (clock.handle.root_ticks_per_tick(), clock.origin)) {
            if let Some(cut) = time::k_at_or_after(origin, ratio, now) {
                self.transmit_until_cut(k, now, cut)?;
                self.cut_segments(k, cut);
            }
        }
        let tracked = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop);
        let open_burst_without_record = self.tx[k].open.is_some() && tracked.is_none();
        if let Some(record) = tracked { self.record_burst(record); }
        self.tx[k].device.close();
        if open_burst_without_record {
            if emit_command_rejected { self.reject_action_at(None, "tx_burst", reason, now)?; }
            else { self.record_rejected_action("tx_burst", reason, now); }
        }
        self.tx[k].open = None;
        let held: Vec<i64> = self.tx[k].held.keys().copied().collect();
        for start in held {
            self.tx[k].held.remove(&start);
            if emit_command_rejected { self.reject_action_at(None, "tx_burst", reason, now)?; }
            else { self.record_rejected_action("tx_burst", reason, now); }
        }
        Ok(())
    }

    /// MR-25's `stop` at `now`, its cancellations recorded with `reason`; a loss runs its
    /// `abort` body at the loss's instant with `DEVICE_LOST` (MR-20), as an item received
    /// with the faults (RM-25). Every stream of the instance is stopped.
    fn stop_at(&mut self, mode: StopMode, now: i64, reason: &str, seq: u64, lost: bool) -> Result<(), ModuleError> {
        for k in 0..self.tx.len() {
            self.stop_tx(k, now, reason, false)?;
        }
        for k in 0..self.tx.len() {
            self.cancel_later(k, reason, false)?;
        }
        for _ in std::mem::take(&mut self.updates) {
            self.record_rejected_action("update_parameter", reason, now);
        }
        self.record_pending_faults();
        if let Some(handle) = self.wakeup.take() { if let Some(time) = &self.time { time.cancel(handle); } }
        // RM-16: under `abort` the receive cut is the first sample not yet delivered; a loss
        // cuts at its instant, having delivered the block in progress (MR-20).
        let abort = mode == StopMode::Abort && !lost;
        let delivered: Vec<Option<i64>> = self.rx.iter()
            .map(|stream| stream.running.as_ref().filter(|_| abort).map(|rx| rx.next))
            .collect();
        for (k, delivered) in delivered.into_iter().enumerate() {
            self.book_rx(k, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered, refused: false, kind: Kind::End { abort } })?;
        }
        for k in 0..self.tx.len() {
            if self.lines.line(Self::tx_key(k)).is_none() { continue; }
            self.book_tx(k, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::End { abort } })?;
            self.tx_reconcile(k)?;
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
        if let Some(domain) = rx.domain { return Ok(domain); }
        let domain = clocks.register_sample_clock(&rx.handle, rx.origin).map_err(|error| ModuleError::rejected(format!("MR-12: {error}")))?;
        rx.domain = Some(domain);
        Ok(domain)
    }

    /// RM-16, RM-25: once the items in effect by `t` cut receive stream `k`'s segment, its
    /// SampleClock, if it has published a block, ends at the cut's instant; a segment cut at
    /// its origin has none, and another may begin there in its place.
    fn rx_settle(&mut self, k: usize, t: i64) -> Result<(), ModuleError> {
        let Some((origin, config, ended)) = self.rx[k].running.as_ref().map(|rx| (rx.origin, rx.config(), rx.ended)) else { return Ok(()) };
        let Some(line) = self.lines.line(Self::rx_key(k)) else { return Ok(()) };
        match line.made(origin, config, t, u64::MAX).map_err(rm26)? {
            None => {
                self.rx[k].running = None;
                self.rx[k].from = origin;
            }
            Some(Segment { cut: Some(cut), .. }) if !ended => {
                if let Some(domain) = self.rx[k].running.as_ref().and_then(|rx| rx.domain) {
                    let at = time::v_of(origin, config.ratio, cut).ok_or_else(|| self.reject("MR-12: receive cut overflow"))?;
                    self.clocks.as_ref().expect("prepared clocks").end(domain, TimePoint::new(self.root.expect("prepared root"), at))
                        .map_err(|error| ModuleError::rejected(format!("MR-12: {error}")))?;
                }
                if let Some(rx) = self.rx[k].running.as_mut() { rx.ended = true; }
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// The receive segment stream `k` is delivering ends, its cut made by `t` (RM-16).
    fn rx_finish(&mut self, k: usize, t: i64) -> Result<(), ModuleError> {
        self.rx_settle(k, t)?;
        if self.rx[k].running.as_ref().is_some_and(|rx| rx.ended) { self.rx[k].running = None; }
        Ok(())
    }

    /// The next receive segment of stream `k`'s plan, after the last one begun (RM-25).
    fn rx_next_segment(&self, k: usize) -> Option<Segment> {
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
                self.clocks.as_ref().expect("prepared clocks").declare_sample_clock(stream, self.root.expect("prepared root"), segment.config.ratio)
                    .map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?
            }
        };
        let channels = segment.config.channels.count() as u16;
        self.rx[k].running = Some(Rx { handle, domain: None, origin: segment.origin, ratio: segment.config.ratio, channels, next: 0, planned: None, flags: BlockFlags::NONE, lost: None, overflows: Vec::new(), ended: false });
        self.rx[k].pool = Some(HostPool::new(self.profile.block_len() as usize * 2 * channels as usize * 8));
        self.rx[k].from = segment.origin.saturating_add(1);
        Ok(())
    }

    /// Books a receive command or fault on receive stream `k`'s timeline (RM-26); returns
    /// its effective instant, or the item's own when the stream has no line yet (a change
    /// before `start`).
    fn book_rx(&mut self, k: usize, item: Item) -> Result<i64, ModuleError> {
        if self.lines.line(Self::rx_key(k)).is_none() {
            return Ok(item.e);
        }
        self.lines.book(Self::rx_key(k), item).map_err(rm26)
    }

    /// Books a transmit command on transmit stream `k`'s timeline (RM-26); nothing when the
    /// stream has no line (no transmit clock).
    fn book_tx(&mut self, k: usize, item: Item) -> Result<(), ModuleError> {
        if self.lines.line(Self::tx_key(k)).is_none() {
            return Ok(());
        }
        self.lines.book(Self::tx_key(k), item).map_err(rm26)?;
        Ok(())
    }

    /// Transmit stream `k`'s current clock (MR-18).
    fn tx_now(&self, k: usize) -> Option<&TxClock> {
        self.tx[k].clocks.get(self.tx[k].cur?)
    }

    /// MR-32's segment generation of transmit stream `k`'s current clock.
    fn tx_gen(&self, k: usize) -> u32 {
        self.tx[k].cur.map_or(0, |index| index as u32 + 1)
    }

    /// The cut of transmit stream `k`'s current clock, from the plan (RM-25).
    fn tx_cut(&self, k: usize) -> Option<i64> {
        self.lines.line(Self::tx_key(k))?.plan.get(self.tx[k].cur?)?.cut
    }

    /// RM-25, VH-2: a transmit clock of stream `k` is registered, and its predecessor ended,
    /// when its change is booked, so the registry follows the transmit plan.
    fn tx_reconcile(&mut self, k: usize) -> Result<(), ModuleError> {
        let Some(plan) = self.lines.line(Self::tx_key(k)).map(|line| line.plan.clone()) else { return Ok(()) };
        let clocks = self.clocks.clone().expect("prepared clocks");
        let root = self.root.expect("prepared root");
        for (index, segment) in plan.iter().enumerate() {
            if index == self.tx[k].clocks.len() {
                let stream = self.tx[k].node.clone();
                let handle = clocks.declare_sample_clock(stream, root, segment.config.ratio).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
                let domain = clocks.register_sample_clock(&handle, segment.origin).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
                self.tx[k].clocks.push(TxClock { handle, domain, origin: segment.origin, ended: false });
            }
            let clock = &mut self.tx[k].clocks[index];
            if let Some(cut) = segment.cut.filter(|_| !clock.ended) {
                let at = time::v_of(clock.origin, clock.handle.root_ticks_per_tick(), cut).ok_or_else(|| ModuleError::rejected("MR-18: transmit cut overflow"))?;
                clocks.end(clock.domain, TimePoint::new(root, at)).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
                clock.ended = true;
            }
        }
        Ok(())
    }

    /// MR-18: transmit stream `k`'s clock `to` is current from a change's instant, with a
    /// new `BurstTracker` and `DeviceModel`; the bursts booked on it are held, and those on a
    /// clock the change passed (one of the first `passed`) are cancelled.
    fn tx_switch(&mut self, k: usize, to: Option<usize>, passed: usize) -> Result<(), ModuleError> {
        self.tx[k].cur = to;
        let tracker = self.tx_now(k).map(|clock| BurstTracker::new(clock.domain));
        self.tx[k].tracker = tracker;
        self.tx[k].device = DeviceModel::new();
        let later = std::mem::take(&mut self.tx[k].later);
        let now = self.root.zip(self.time.as_ref()).and_then(|(root, time)| time.now(root).and_then(|t| t.ticks_in(root)).ok()).unwrap_or(0);
        for (index, held) in later {
            if Some(index) == to {
                self.tx[k].held.insert(held.start, held);
            } else if index < passed {
                self.forget_later(k, index, held.start);
                self.reject_action_at(None, "tx_burst", "MR-18: cancelled by a cold change", now)?;
            } else {
                self.tx[k].later.push((index, held));
            }
        }
        Ok(())
    }

    /// Cancels the bursts booked on transmit stream `k`'s clocks not yet current (RM-16,
    /// MR-25).
    fn cancel_later(&mut self, k: usize, reason: &str, emit: bool) -> Result<(), ModuleError> {
        let now = self.root.zip(self.time.as_ref()).and_then(|(root, time)| time.now(root).and_then(|t| t.ticks_in(root)).ok()).unwrap_or(0);
        for (index, held) in std::mem::take(&mut self.tx[k].later) {
            self.forget_later(k, index, held.start);
            if emit { self.reject_action_at(None, "tx_burst", reason, now)?; } else { self.record_rejected_action("tx_burst", reason, now); }
        }
        Ok(())
    }

    /// Withdraws a cancelled burst of transmit stream `k`'s clock `index` from the channel
    /// (MR-32).
    fn forget_later(&mut self, k: usize, index: usize, start: i64) {
        let Some(shared) = self.tx[k].plan.clone() else { return };
        shared.plan().segments.remove(&(index as u32 + 1, start));
    }

    /// The configuration in force at `e` for an update received with `order`: the applied
    /// one and every pending update before it in UC-2's order (MR-18).
    fn projected(&self, e: i64, order: u64) -> BTreeMap<ezsdr_kernel::spec::Key, Value> {
        let mut config = self.config.clone();
        for update in self.updates.range(..(e, order)).map(|(_, update)| update) {
            config.insert(update.key.clone(), update.value.clone());
        }
        config
    }

    /// RM-25: root ticks per sample at `rate` on this profile.
    fn ratio(&self, rate: f64) -> Result<Rational, ModuleError> {
        let root = self.root.expect("root");
        let clocks = self.clocks.as_ref().expect("clocks");
        let root_rate = clocks.nominal_rate(root).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
        match self.profile.kind {
            ProfileKind::X3xx => {
                let n = self.profile.decimation(rate).ok_or_else(|| ModuleError::rejected("MR-18: rate is not an X3xx decimation"))?;
                Rational::new(root_rate.num().checked_mul(n).ok_or_else(|| ModuleError::rejected("MR-18: ratio overflow"))?, x3xx::MASTER_CLOCK_HZ)
            }
            ProfileKind::Ideal => Rational::new(root_rate.num(), rate as u64),
        }.map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))
    }

    /// RM-26: this instance's description, from its profile (design 33 D5).
    fn description(&self) -> &ezsdr_radio::device::DeviceDescription {
        self.profile.description()
    }

    /// A stream's timeline terms on this profile (RM-25, MR-18).
    fn line(&self, direction: Direction, origin: i64, config: &BTreeMap<ezsdr_kernel::spec::Key, Value>) -> Result<Line, ModuleError> {
        let side = if direction == Direction::Rx { "rx" } else { "tx" };
        let channels = timeline::Channels::first(int_config(config, &format!("radio.{side}.channels")).max(0) as u32);
        let ratio = self.ratio(num_config(config, &format!("radio.{side}.sample_rate_hz")))?;
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        let root = self.root.expect("prepared root");
        let ticks = |ns: i64| time::ns_to_v(clocks, root, ns).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")));
        Ok(Line::new(timeline::Stream {
            direction,
            origin,
            config: timeline::Config { channels, ratio },
            lead: ticks(timeline::start_lead_ns(&self.profile.timing(), direction))?,
            call: self.profile.rx_call_samples(),
            allowance: ticks(self.profile.delivery_allowance_ns())?,
        }))
    }

    /// `node` is what the Kernel resolved the target to (KC-23), which is what a
    /// Provider matches on.
    fn handle_action(&mut self, dispatched: Dispatched, now: i64) -> Result<(), ModuleError> {
        let (rx, tx) = (self.rx[0].node.clone(), self.tx[0].node.clone());
        let id = dispatched.id;
        let node = dispatched.node.as_ref();
        match dispatched.action {
            action @ Action::TxBurst { .. } => self.handle_tx_burst(0, id, action, node, now),
            Action::UpdateParameter { key, value, class, at, .. } => self.handle_update(id, node, key, value, class, at, now),
            // RM-16, MR-25: a `Stop` of the device or of a stream cancels no update; on
            // receive it is a cut at its instant, on transmit it ends the bursts.
            Action::Stop { target } => {
                let device = target.is_none() || node == Some(&self.instance.id);
                if !device && node != Some(&rx) && node != Some(&tx) {
                    return self.reject_action_at(Some(id), "stop", "MR-25: Stop target is not this device or its stream", now);
                }
                if device || node == Some(&tx) {
                    self.stop_tx(0, now, "MR-25: cancelled by stop", false)?;
                    self.cancel_later(0, "MR-25: cancelled by stop", false)?;
                }
                if device || node == Some(&rx) {
                    let seq = self.arrival();
                    self.book_rx(0, Item { e: now, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Stop })?;
                }
                Ok(())
            }
            // RM-12, RM-21: `start_rx` turns the receive stream on at its `at`, or at its
            // receipt if that is later, never before T0; it is never late.
            Action::Command { verb, params, at, .. } if verb.as_str() == ezsdr_radio::START_RX && node == Some(&rx) && params.is_empty() => {
                let requested = match at.map(|at| time::to_v(self.clocks.as_ref().expect("prepared clocks"), self.root.expect("prepared root"), at.time_point)).transpose() {
                    Ok(requested) => requested,
                    Err(error) => return self.reject_action_at(Some(id), "command", &format!("MR-29: start_rx's instant cannot be converted: {error}"), now),
                };
                let seq = self.arrival();
                let (e, _) = timeline::command_instant(self.lines.line(Self::rx_key(0)), requested, now, seq, false);
                self.book_rx(0, Item { e, seq, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Start })?;
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

    fn handle_tx_burst(&mut self, k: usize, id: ActionId, action: Action, node: Option<&ResourceId>, now: i64) -> Result<(), ModuleError> {
        let Action::TxBurst { waveform, repeat, at, requested_at, late_policy, metadata, .. } = action else {
            unreachable!("only TxBurst is dispatched to handle_tx_burst")
        };
        let size_bytes = waveform.size_bytes;
        let metadata_empty = metadata.is_empty();
        let root = self.root.expect("prepared root");
        let tx_id = self.tx[k].node.clone();
        // The burst's clock: the stream's current one, or one a booked change starts
        // (KC-21a, RM-25).
        let clock = self.tx[k].clocks.iter().position(|clock| clock.domain == at.time_point.domain())
            .filter(|index| self.tx[k].cur.is_none_or(|current| *index >= current))
            .filter(|index| Some(*index) == self.tx[k].cur || self.lines.line(Self::tx_key(k)).and_then(|line| line.plan.get(*index)).is_some_and(|segment| segment.cut.is_none_or(|cut| cut > 0)));
        let Some(index) = clock else {
            let reason = if self.tx[k].cur.is_none() && self.tx_later_clock(k).is_none() { "MR-16: no transmit channel is configured" } else { "MR-16: target, clock, waveform size, channel count or metadata is invalid" };
            self.reject_action_at(Some(id), "tx_burst", reason, now)?;
            return Ok(());
        };
        let current = Some(index) == self.tx[k].cur;
        let channels = if current {
            self.effective_channels(ezsdr_radio::keys::TX_CHANNELS)
        } else {
            self.lines.line(Self::tx_key(k)).map_or(0, |line| i64::from(line.plan[index].config.channels.count()))
        };
        let unit = channels.max(0) as u64 * 8;
        if node != Some(&tx_id) || channels <= 0 || size_bytes == 0 || unit == 0 || size_bytes % unit != 0 || !metadata_empty {
            self.reject_action_at(Some(id), "tx_burst", "MR-16: target, clock, waveform size, channel count or metadata is invalid", now)?;
            return Ok(());
        }
        let length = i64::try_from(size_bytes / unit).map_err(|_| ModuleError::rejected("MR-16: waveform is too large"))?;
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
        let (ratio, origin, tx_domain) = {
            let clock = &self.tx[k].clocks[index];
            (clock.handle.root_ticks_per_tick(), clock.origin, clock.domain)
        };
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        let lead = time::ns_to_v(clocks, root, self.profile.timing().min_timed_command_lead_ns).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))?;
        // RM-15: decided at the clock's origin less the lead if that is later.
        let decided = timeline::decided_at(now, origin, lead);
        let now_tx = clocks.convert(TimePoint::new(root, now), tx_domain).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))?.floor();
        let decided_tx = match clocks.convert(TimePoint::new(root, decided), tx_domain).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))? {
            ezsdr_kernel::time::Converted::Exact { point } => point,
            ezsdr_kernel::time::Converted::Inexact { floor, .. } => floor.checked_add(Duration::new(tx_domain, 1)).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))?,
        };
        let policy = late_policy.decide(
            clocks,
            at.time_point,
            decided_tx,
            Duration::new(ClockDomainId::HOST_MONOTONIC, self.profile.timing().min_timed_command_lead_ns),
        ).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))?;
        let mut start = at.time_point.ticks_in(tx_domain).map_err(|error| ModuleError::rejected(format!("MR-17: {error}")))?;
        let mut requested = requested_at.map(|deadline| deadline.time_point);
        let mut send_asap_late_by = None;
        match policy {
            LateOutcome::OnTime {} => {}
            LateOutcome::SendAsap { late_by } => {
                let earliest = decided.checked_add(lead).ok_or_else(|| ModuleError::rejected("MR-17: asap instant overflow"))?;
                start = time::k_at_or_after(origin, ratio, earliest).ok_or_else(|| ModuleError::rejected("MR-17: asap sample overflow"))?;
                requested = requested.or(Some(at.time_point));
                send_asap_late_by = Some(late_by);
            }
            LateOutcome::Drop { late_by } => {
                self.emit_late_burst(late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::Drop)?;
                self.record_rejected_action("tx_burst", "MR-17: the late policy dropped the burst", now);
                return Ok(());
            }
            LateOutcome::PlanViolation { late_by } => {
                self.emit_late_burst(late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::PlanViolation)?;
                self.record_rejected_action("tx_burst", "MR-17: the planned burst arrived late", now);
                return Ok(());
            }
        }
        let start_v = time::v_of(origin, ratio, start).ok_or_else(|| ModuleError::rejected("MR-16: burst time overflow"))?;
        let refusal = if start_v < self.sync_end.unwrap_or(i64::MIN) {
            Some("MR-16: burst begins before synchronization")
        } else if (current && self.tx[k].held.contains_key(&start)) || self.tx[k].later.iter().any(|(later, held)| *later == index && held.start == start) {
            Some("MR-16: a held burst already has this start")
        } else if current && self.tx[k].open.as_ref().is_some_and(|open| start <= open.next) {
            Some("MR-16: burst begins at or before the open burst's next sample")
        } else {
            None
        };
        if let Some(reason) = refusal {
            if let Some(late_by) = send_asap_late_by {
                self.emit_late_burst(late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::Refused)?;
            }
            self.reject_action_at(Some(id), "tx_burst", reason, now)?;
            return Ok(());
        }
        if let Some(late_by) = send_asap_late_by {
            self.emit_late_burst(late_by, at.time_point, now_tx, ezsdr_radio::payloads::TimeErrorOutcome::SendAsap)?;
        }
        let waveform_len = u32::try_from(length).map_err(|_| ModuleError::rejected("MR-16: waveform length exceeds u32"))?;
        if let (Some(shared), Some((samples, clipped))) = (self.tx[k].plan.clone(), decoded) {
            shared.plan().segments.insert((index as u32 + 1, start), channel::Segment {
                origin,
                ratio,
                start,
                len: length,
                repeat,
                channels: channels as u16,
                samples,
                end: None,
            });
            self.add_stat("tx_clipped", clipped);
        }
        let held = HeldBurst {
            start,
            len: length,
            repeat,
            open: BurstOpen {
                waveform_len: Some(waveform_len),
                late: (!matches!(policy, LateOutcome::OnTime {})).then_some(policy),
                requested_target: requested,
            },
            order: self.next_order,
        };
        if current { self.tx[k].held.insert(start, held); } else { self.tx[k].later.push((index, held)); }
        self.next_order = self.next_order.saturating_add(1);
        Ok(())
    }

    /// Whether a transmit clock stream `k`'s booked change starts has been registered.
    fn tx_later_clock(&self, k: usize) -> Option<usize> {
        let first = self.tx[k].cur.map_or(0, |current| current + 1);
        (first < self.tx[k].clocks.len()).then_some(first)
    }

    fn emit_tx_until(&mut self, k: usize, until: i64, extra_cut: Option<i64>) -> Result<bool, ModuleError> {
        let mut progressed = false;
        while let Some(open) = self.tx[k].open.as_ref() {
            let Some((ratio, origin, domain)) = self.tx_now(k).map(|clock| (clock.handle.root_ticks_per_tick(), clock.origin, clock.domain)) else { break; };
            let mut stop = open.next.saturating_add(i64::from(self.profile.block_len()));
            let repeat_end = if open.repeat {
                open.start.saturating_add(((open.next - open.start) / open.len + 1).saturating_mul(open.len))
            } else { open.start.saturating_add(open.len) };
            let held_cut = self.tx[k].held.keys().next().copied().unwrap_or(i64::MAX);
            let cut = held_cut.min(self.tx_cut(k).unwrap_or(i64::MAX)).min(extra_cut.unwrap_or(i64::MAX));
            stop = stop.min(repeat_end).min(cut);
            if stop <= open.next { break; }
            let last = time::v_after(origin, ratio, stop - 1).ok_or_else(|| ModuleError::rejected("MR-15: transmit time overflow"))?;
            if last > until { break; }
            let start = open.start;
            let next = open.next;
            let first = open.first;
            let burst_open = open.open;
            let len = u32::try_from(stop - next).map_err(|_| ModuleError::rejected("MR-15: transmit block length overflow"))?;
            let channels = self.effective_channels(ezsdr_radio::keys::TX_CHANNELS) as u16;
            let ends = (!open.repeat && stop >= start.saturating_add(open.len)) || held_cut == stop;
            let flags = (if first { BlockFlags::START_OF_BURST } else { BlockFlags::NONE })
                | if ends { BlockFlags::END_OF_BURST } else { BlockFlags::NONE };
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
                    if let Some(record) = then_ended { self.record_burst(record); }
                    self.reject_action_at(None, "tx_burst", "MR-15: the burst's blocks were not contiguous", until)?;
                }
                Ok(BurstStep::Started | BurstStep::Continued) => {}
                Err(error) => {
                    self.reject_action_at(None, "tx_burst", &format!("MR-15: {error}"), until)?;
                    self.end_open_segment(k, next);
                    if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) { self.record_burst(record); }
                    self.tx[k].device.close();
                    self.tx[k].open = None;
                    break;
                }
            }
            if self.tx[k].device.on_tx_block(&header).is_err() {
                let payload = serde_json::to_value(ezsdr_radio::payloads::TimeErrorPayload {
                    cause: ezsdr_radio::payloads::TimeErrorCause::UnclosedBurst,
                    outcome: ezsdr_radio::payloads::TimeErrorOutcome::Refused,
                    late_by_ns: 0,
                    target: header.first_sample_time,
                }).expect("device error payload");
                self.emit_event("tx", ezsdr_radio::kinds::TIME_ERROR, Severity::Error, payload, header.first_sample_time)?;
                self.record_rejected_action("tx_burst", "MR-24: the device rejected an unclosed burst", until);
                self.end_open_segment(k, next);
                if let Some(record) = self.tx[k].tracker.as_mut().and_then(BurstTracker::stop) { self.record_burst(record); }
                self.tx[k].device.close();
                self.tx[k].open = None;
                progressed = true;
                break;
            }
            self.add_stat("tx_blocks", 1);
            progressed = true;
            if ends {
                self.tx[k].open = None;
                self.tx[k].device.close();
            } else if let Some(open) = self.tx[k].open.as_mut() {
                open.next = stop;
                open.first = false;
            }
        }
        Ok(progressed)
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_update(&mut self, id: ActionId, node: Option<&ResourceId>, key: ezsdr_kernel::spec::Key, value: Value, class: UpdateClass, at: Option<AbsoluteDeadline>, now: i64) -> Result<(), ModuleError> {
        if node != Some(&self.instance.id) || !ezsdr_radio::keys::CONFIGURATION.contains(&key.as_str()) || key.as_str().ends_with(".antenna") {
            self.reject_action_at(Some(id), "update_parameter", "MR-18: target or configuration key is not updateable", now)?;
            return Ok(());
        }
        let expected = if is_hardware_key(key.as_str()) { UpdateClass::HardwareTimed } else { UpdateClass::Cold };
        if class != expected {
            self.reject_action_at(Some(id), "update_parameter", "MR-18: update class does not match the Radio Model key", now)?;
            return Ok(());
        }
        let root = self.root.expect("root");
        let clocks = self.clocks.as_ref().expect("clocks");
        let requested_tick = match at {
            Some(deadline) => match time::to_v(clocks, root, deadline.time_point) {
                Ok(tick) => Some(tick),
                Err(error) => {
                    self.reject_action_at(Some(id), "update_parameter", &format!("MR-18: timed instant cannot be converted: {error}"), now)?;
                    return Ok(());
                }
            },
            None => None,
        };
        let late = |requested: i64, applied: i64| serde_json::to_value(ezsdr_radio::payloads::LateCommandPayload {
            key: Some(key.clone()), requested: TimePoint::new(root, requested), applied: TimePoint::new(root, applied),
        }).expect("late command payload");
        let order = self.next_order;
        let tick = if expected == UpdateClass::HardwareTimed {
            let lead = time::ns_to_v(clocks, root, self.profile.timing().min_timed_command_lead_ns).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
            let earliest = now.checked_add(lead).ok_or_else(|| ModuleError::rejected("MR-18: timed command instant overflow"))?;
            let requested = requested_tick.unwrap_or(earliest);
            let depth = self.profile.timing().command_queue_depth as usize;
            if self.updates.values().filter(|update| update.class == UpdateClass::HardwareTimed).count() >= depth {
                let payload = serde_json::to_value(ezsdr_radio::payloads::CommandQueueFullPayload { key: key.clone(), depth: depth as i64 }).expect("queue full payload");
                self.emit_event("", ezsdr_radio::kinds::COMMAND_QUEUE_FULL, Severity::Error, payload, TimePoint::new(root, now))?;
                self.record_rejected_action("update_parameter", "MR-18: hardware-timed command queue is full", now);
                return Ok(());
            }
            if requested < earliest {
                self.emit_event("", ezsdr_radio::kinds::LATE_COMMAND, Severity::Warning, late(requested, earliest), TimePoint::new(root, now))?;
            }
            requested.max(earliest)
        } else {
            // A `cold` change of a channel of this instance belongs to the stream that holds
            // the channel; one direction is one stream today (RM-29).
            let line = if is_rx_cold_key(key.as_str()) { self.lines.line(Self::rx_key(0)) } else { self.lines.line(Self::tx_key(0)) };
            let (e, is_late) = timeline::command_instant(line, requested_tick, now, order, true);
            if is_late {
                self.emit_event("", ezsdr_radio::kinds::LATE_COMMAND, Severity::Warning, late(requested_tick.unwrap_or(now), e), TimePoint::new(root, now))?;
            }
            e
        };
        // MR-18, RM-7: `coerce` over the configuration projected to `e`; a refusal changes nothing.
        let mut candidate = self.projected(tick, order);
        candidate.insert(key.clone(), value.clone());
        let constraints = candidate.iter().filter_map(|(key, value)| Some((key.clone(), ezsdr_kernel::spec::Constraint::Eq { value: value.as_scalar()?.clone() }))).collect();
        let requested = Requested { resource: self.instance.id.clone(), constraints };
        let report = self.profile.description().coerce(&self.instance.id, &requested).map_err(|error| ModuleError::rejected(format!("MR-18: {error}")))?;
        if let Some(rejected) = report.rejected.first() {
            self.reject_action_at(Some(id), "update_parameter", &format!("MR-18: {}", rejected.reason), now)?;
            return Ok(());
        }
        let applied = report.applied.get(&key).cloned().unwrap_or(value);
        candidate.insert(key.clone(), applied.clone());
        self.next_order = self.next_order.saturating_add(1);
        // RM-25, RM-26: a `cold` change is planned on its stream's timeline when booked; a
        // transmit one registers its clock and ends its predecessor now (VH-2).
        let mut tx = None;
        let direction = if is_rx_cold_key(key.as_str()) { Some(Direction::Rx) } else if is_tx_cold_key(key.as_str()) { Some(Direction::Tx) } else { None };
        if let Some(direction) = direction {
            let side = if direction == Direction::Rx { "rx" } else { "tx" };
            let channels = int_config(&candidate, &format!("radio.{side}.channels")).max(0) as u16;
            let rate = num_config(&candidate, &format!("radio.{side}.sample_rate_hz"));
            let stream = if direction == Direction::Rx { Self::rx_key(0) } else { Self::tx_key(0) };
            if let Some(old) = self.lines.line(stream).map(|line| line.stream.config) {
                let ratio = if channels > 0 { self.ratio(rate)? } else { self.ratio(rate).unwrap_or(old.ratio) };
                let item = Item { e: tick, seq: order, ready: now, release: NO_RELEASE, delivered: None, refused: false, kind: Kind::Cold(timeline::Config { channels: timeline::Channels::first(u32::from(channels)), ratio }) };
                if direction == Direction::Rx {
                    self.book_rx(0, item)?;
                } else {
                    self.book_tx(0, item)?;
                    let clocks = self.lines.line(stream).map_or(0, |line| line.plan.len());
                    self.tx_reconcile(0)?;
                    tx = Some(((channels > 0).then(|| clocks - 1), clocks));
                }
            }
        }
        self.updates.insert((tick, order), PendingUpdate { key, value: applied, class, tx });
        Ok(())
    }

    /// MR-18: at `e` the value applies and is recorded; a transmit `cold` change ends the old
    /// clock's bursts at its cut and makes its own clock current.
    fn apply_update(&mut self, tick: i64, effective: i64, order: u64) -> Result<(), ModuleError> {
        let Some(update) = self.updates.remove(&(effective, order)) else { return Ok(()); };
        self.config.insert(update.key.clone(), update.value.clone());
        self.record_update(&update.key, &update.value, tick);
        if let Some(mode) = self.channel.as_mut() {
            if let Some(value) = match update.value.as_scalar() { Some(Scalar::Num(v)) => Some(v.get()), Some(Scalar::Int(v)) => Some(*v as f64), _ => None } {
                match update.key.as_str() {
                    ezsdr_radio::keys::TX_GAIN_DB => {
                        if let Some(shared) = self.tx[0].plan.clone() { shared.plan().gain_db.set(tick, value); }
                    }
                    ezsdr_radio::keys::TX_FREQUENCY_HZ => {
                        if let Some(shared) = self.tx[0].plan.clone() {
                            let mut plan = shared.plan();
                            plan.frequency_hz.set(tick, value);
                            if self.profile.timed_tune_fixes_phase() {
                                let timed = plan.timed_phase.clone();
                                for (phase, value) in plan.phase.iter_mut().zip(timed) { phase.set(tick, value); }
                            } else {
                                // MR-34: a `random` channel redraws its phase at every applied tune,
                                // every channel of the direction in draw order (design 33 D13).
                                let count = plan.phase.len();
                                let values: Vec<f64> = (0..count).map(|_| channel::lo_phase(&mut mode.lo_retune)).collect();
                                for (phase, value) in plan.phase.iter_mut().zip(values) { phase.set(tick, value); }
                            }
                        }
                    }
                    ezsdr_radio::keys::RX_GAIN_DB => {
                        if let Some(model) = self.rx[0].model.as_mut() { model.gain_db.set(tick, value); }
                    }
                    ezsdr_radio::keys::RX_FREQUENCY_HZ => {
                        if let Some(model) = self.rx[0].model.as_mut() {
                            model.frequency_hz.set(tick, value);
                            if self.profile.timed_tune_fixes_phase() {
                                for (phase, value) in model.phase.iter_mut().zip(model.timed_phase.iter()) { phase.set(tick, *value); }
                            } else {
                                let values: Vec<f64> = (0..model.phase.len()).map(|_| channel::lo_phase(&mut mode.lo_retune)).collect();
                                for (phase, value) in model.phase.iter_mut().zip(values) { phase.set(tick, value); }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some((to, passed)) = update.tx {
            self.stop_tx(0, tick, "MR-18: cancelled by a cold change", true)?;
            self.tx_switch(0, to, passed)?;
        }
        Ok(())
    }

    /// The step's ordered loop up to `u` (RM-25, MR-20): every stream's receive segment
    /// beginnings, then faults, held bursts and updates by instant and arrival, the streams
    /// emitted up to each.
    fn advance(&mut self, u: i64) -> Result<bool, ModuleError> {
        let mut progressed = false;
        loop {
            let mut candidates: Vec<(i64, u8, u64, WorkKind)> = Vec::new();
            if self.started {
                candidates.extend(self.faults.iter().enumerate().filter(|(_, fault)| !fault.resolved)
                    .map(|(index, fault)| (fault.tick, 1, fault.order, WorkKind::Fault { index })));
                for k in 0..self.rx.len() {
                    if let Some(segment) = self.rx_next_segment(k) {
                        candidates.push((segment.origin, 0, 0, WorkKind::RxBegin { stream: k, origin: segment.origin }));
                    }
                }
            }
            for k in 0..self.tx.len() {
                candidates.extend(self.tx[k].held.iter().filter_map(|(start, burst)| {
                    let clock = self.tx_now(k)?;
                    let tick = time::v_of(clock.origin, clock.handle.root_ticks_per_tick(), *start)?;
                    Some((tick, 1, burst.order, WorkKind::HeldBurst { stream: k, start: *start }))
                }));
            }
            candidates.extend(self.updates.keys().map(|(effective, order)| (*effective, 1, *order, WorkKind::Update { effective: *effective })));
            candidates.sort_by_key(|(tick, rank, order, _)| (*tick, *rank, *order));
            let Some((tick, _, order, kind)) = candidates.first().copied() else { break; };
            if tick > u { break; }
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
                    // The instance's faults cut its one receive stream (RM-29, MR-21).
                    FaultKind::RxOverflow | FaultKind::RxSequenceError => self.apply_rx_fault(0, index)?,
                    FaultKind::DeviceLost => {
                        // MR-20: the blocks before `f` are out; the device stops at `f` as an
                        // `abort` would there, the receive stream cut at `f`, and the step fails.
                        self.faults[index].applied = true;
                        self.faults[index].resolved = true;
                        self.record_fault(index, 0);
                        self.device_lost_reported = true;
                        let seq = self.faults[index].order;
                        self.stop_at(StopMode::Abort, tick, DEVICE_LOST, seq, true)?;
                        return Err(ModuleError { kind: ezsdr_kernel::module_api::ModuleErrorKind::DeviceLost, message: DEVICE_LOST.to_owned(), detail: serde_json::Value::Null });
                    }
                },
                WorkKind::HeldBurst { stream, start } => {
                    if let Some(held) = self.tx[stream].held.remove(&start) {
                        self.tx[stream].open = Some(OpenBurst {
                            start: held.start,
                            len: held.len,
                            repeat: held.repeat,
                            next: held.start,
                            first: true,
                            open: held.open,
                        });
                    }
                }
                WorkKind::Update { effective } => self.apply_update(tick, effective, order)?,
                WorkKind::RxBegin { stream, origin } => match self.rx_next_segment(stream).filter(|segment| segment.origin == origin) {
                    Some(segment) => self.rx_begin(stream, segment, None)?,
                    None => self.rx[stream].from = origin.saturating_add(1),
                },
            }
            progressed = true;
        }
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

fn rm26(error: ezsdr_kernel::time::TimeError) -> ModuleError {
    ModuleError::rejected(format!("RM-26: {error}"))
}

fn int_config(config: &BTreeMap<ezsdr_kernel::spec::Key, Value>, name: &str) -> i64 {
    let key = ezsdr_kernel::spec::Key::parse(name).expect("radio key");
    match config.get(&key).and_then(Value::as_scalar) { Some(Scalar::Int(value)) => *value, _ => 0 }
}

fn num_config(config: &BTreeMap<ezsdr_kernel::spec::Key, Value>, name: &str) -> f64 {
    let key = ezsdr_kernel::spec::Key::parse(name).expect("radio key");
    match config.get(&key).and_then(Value::as_scalar) { Some(Scalar::Num(value)) => value.get(), Some(Scalar::Int(value)) => *value as f64, _ => 0.0 }
}

fn is_rx_cold_key(key: &str) -> bool {
    matches!(key, ezsdr_radio::keys::RX_CHANNELS | ezsdr_radio::keys::RX_SAMPLE_RATE_HZ)
}

fn is_tx_cold_key(key: &str) -> bool {
    matches!(key, ezsdr_radio::keys::TX_CHANNELS | ezsdr_radio::keys::TX_SAMPLE_RATE_HZ)
}

fn is_hardware_key(key: &str) -> bool {
    matches!(key,
        ezsdr_radio::keys::RX_FREQUENCY_HZ | ezsdr_radio::keys::TX_FREQUENCY_HZ
        | ezsdr_radio::keys::RX_GAIN_DB | ezsdr_radio::keys::TX_GAIN_DB)
}

/// One of a MockRadio instance's six Manifest sections, `ezsdr.radio.mock.<suffix>`; the
/// Kernel files it under the instance's source root (MR-27, RS-39).
fn section(suffix: &str) -> Namespace {
    Namespace::parse(&format!("ezsdr.radio.mock.{suffix}")).expect("module section")
}

impl Provider for MockRadio {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn coerce(&self, request: &Requested) -> Result<CoerceReport, ModuleError> {
        self.profile.description().coerce(&self.instance.id, request)
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
        let mut config = self.description().defaults.clone();
        for (key, value) in report.applied {
            if ezsdr_radio::keys::CONFIGURATION.contains(&key.as_str()) {
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
        let mut rx_handle = None;
        let mut tx_handle = None;
        for (direction, dest) in [("rx", &mut rx_handle), ("tx", &mut tx_handle)] {
            let channels = int_config(&config, &format!("radio.{direction}.channels"));
            if channels <= 0 { continue; }
            let rate = num_config(&config, &format!("radio.{direction}.sample_rate_hz"));
            let ratio = match self.profile.kind {
                ProfileKind::X3xx => {
                    let n = self.profile.decimation(rate).ok_or_else(|| self.reject(format!("MR-7: rate {rate} is not an X3xx decimation")))?;
                    Rational::new(root_rate.num().checked_mul(n).ok_or_else(|| self.reject("MR-7: clock ratio overflow"))?, x3xx::MASTER_CLOCK_HZ)
                }
                ProfileKind::Ideal => Rational::new(root_rate.num(), rate as u64),
            }.map_err(|error| self.reject(format!("MR-7: {error}")))?;
            let stream = if direction == "rx" { self.rx[0].node.clone() } else { self.tx[0].node.clone() };
            *dest = Some(ctx.clocks.declare_sample_clock(stream, root, ratio).map_err(|error| self.reject(format!("MR-7: {error}")))?);
        }
        let mut links = Vec::new();
        for attached in &ctx.links {
            if attached.component != f.id || attached.port.as_str() != self.rx[0].port {
                return Err(self.reject("MR-7: only this fragment's rx port may be attached"));
            }
            match &attached.endpoint {
                Endpoint::StreamOut(link) => links.push(link.clone()),
                _ => return Err(self.reject("MR-7: rx must attach to a StreamOut link")),
            }
        }
        if links.len() > 1 && links.iter().any(|link| link.policy() == BackPressure::Block) {
            return Err(self.reject("MR-7: multiple rx links cannot use Block back-pressure"));
        }
        let seed = ezsdr_sim::seed(&ctx.environment).map_err(|error| self.reject(format!("MR-7: {error}")))?;
        let mut faults = Vec::new();
        for (order, entry) in ezsdr_sim::faults(&ctx.environment).map_err(|error| self.reject(format!("MR-7: {error}")))?
            .into_iter().filter(|fault| fault.target == f.id).enumerate()
        {
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
                let (rx_max, tx_max) = (self.description().rx_max_channels, self.description().tx_max_channels);
                if let Some(coupling) = spec.couplings.iter().find(|c| {
                    (c.tx == f.id && i64::from(c.tx_channel) >= tx_max)
                        || (c.rx == f.id && i64::from(c.rx_channel) >= rx_max)
                }) {
                    let max = if coupling.rx == f.id { rx_max } else { tx_max };
                    return Err(self.reject(format!("MR-31: a coupling {} -> {} names a channel beyond this profile's {max}", coupling.tx, coupling.rx)));
                }
                let random = self.profile.random_phase_on_untimed_tune();
                let mut lo = SimRng::new(seed, &format!("{}/lo", self.selector_id()));
                // MR-34: the initial phases, and the timed-tune constants of a profile whose
                // timed tunes fix them; a `random` board redraws instead (design 33 D13).
                let fixes = random && self.profile.timed_tune_fixes_phase();
                let draw = |count: i64, rng: &mut SimRng, live: bool| (0..count).map(|_| if live { channel::lo_phase(rng) } else { 0.0 }).collect::<Vec<f64>>();
                let rx_phase: Vec<channel::Timeline> = draw(rx_max, &mut lo, random).into_iter().map(channel::Timeline::new).collect();
                let tx_phase: Vec<channel::Timeline> = draw(tx_max, &mut lo, random).into_iter().map(channel::Timeline::new).collect();
                let rx_timed = draw(rx_max, &mut lo, fixes);
                let tx_timed = draw(tx_max, &mut lo, fixes);
                let lo_retune = SimRng::new(seed, &format!("{}/lo/retune", self.selector_id()));
                let tx = channel::TxShared::new(channel::TxPlan {
                    segments: BTreeMap::new(),
                    gain_db: channel::Timeline::new(num_config(&config, ezsdr_radio::keys::TX_GAIN_DB)),
                    frequency_hz: channel::Timeline::new(num_config(&config, ezsdr_radio::keys::TX_FREQUENCY_HZ)),
                    phase: tx_phase,
                    timed_phase: tx_timed,
                    path_delay: self.description().tx_path_delay_samples,
                });
                let rx = channel::RxModel {
                    gain_db: channel::Timeline::new(num_config(&config, ezsdr_radio::keys::RX_GAIN_DB)),
                    frequency_hz: channel::Timeline::new(num_config(&config, ezsdr_radio::keys::RX_FREQUENCY_HZ)),
                    phase: rx_phase,
                    timed_phase: rx_timed,
                };
                // The channel models belong to the streams they describe: the transmit
                // plan to the transmit stream, the receive parameters to the receive one
                // (MR-32, MR-35). The medium takes this instance's transmit side first, so
                // a refusal from `join` leaves neither behind; `cleanup` rebuilds the
                // streams, so a stream's model is present exactly when the instance is in
                // channel mode and neither outlives the other.
                medium.join(&ctx.run, &f.id, &spec, seed, root_rate.num(), tx.clone()).map_err(|error| self.reject(format!("MR-31: {error}")))?;
                self.tx[0].plan = Some(tx.clone());
                self.rx[0].model = Some(rx);
                Some(ChannelMode { medium, fragment: f.id.clone(), inputs: ctx.inputs.clone(), lo_retune })
            }
        };
        if channel_mode.is_some() {
            self.instance.fidelity.rf = ezsdr_kernel::module_api::RfFidelity::ImpairmentModel;
        }
        self.channel = channel_mode;
        self.root = Some(root);
        self.clocks = Some(ctx.clocks);
        self.time = Some(ctx.time);
        // MR-37: `resolve` is control-path only (RS-33), so the handle is taken here;
        // the view refuses a source outside this instance's root (item 3).
        let EventSource::Node { node: source_root } = ctx.source else {
            return Err(self.reject("KC-8: a Provider's event source root is a node"));
        };
        let rx_node = source_root.child("rx").map_err(|error| self.reject(format!("MR-37: {error}")))?;
        let overflow = EventKind::parse(ezsdr_radio::kinds::RX_OVERFLOW).expect("declared radio event kind");
        self.rx[0].overflow = Some(ctx.events
            .resolve(&EventSource::Node { node: rx_node }, &overflow)
            .map_err(|error| self.reject(format!("MR-37: {error}")))?);
        self.source_root = Some(source_root);
        self.events = Some(ctx.events);
        self.actions = Some(ctx.actions);
        self.rx[0].clock = rx_handle;
        self.tx[0].clock = tx_handle;
        self.rx[0].links = links;
        self.rx[0].rng = Some(SimRng::new(seed, &format!("{}/rx", self.selector_id())));
        self.faults = faults;
        self.next_order = self.faults.len() as u64;
        self.config = config.clone();
        self.prepared = true;
        Ok(PrepareReport { effective: config, coercions: report.coercions, warnings: Vec::new() })
    }

    fn arm(&mut self) -> Result<(), ModuleError> {
        if !self.prepared { return Err(self.reject("MR-9: prepare must precede arm")); }
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
        self.sync_end = Some(now.checked_add(time::ns_to_v(&clocks, root, latency).map_err(|error| self.reject(format!("MR-9: {error}")))?).ok_or_else(|| self.reject("MR-9: synchronisation time overflow"))?);
        // RM-25: the transmit stream from the first lattice instant at or after the arm
        // instant (VE-4).
        let ratio = self.ratio(num_config(&self.config, ezsdr_radio::keys::TX_SAMPLE_RATE_HZ))?;
        let origin = timeline::lattice(now, ratio).map_err(|error| self.reject(format!("MR-9: {error}")))?;
        let line = self.line(Direction::Tx, origin, &self.config)?;
        if let Some(handle) = self.tx[0].clock.clone() {
            let domain = clocks.register_sample_clock(&handle, origin).map_err(|error| self.reject(format!("MR-9: {error}")))?;
            self.tx[0].clocks.push(TxClock { handle, domain, origin, ended: false });
        }
        let current = (!self.tx[0].clocks.is_empty()).then_some(0);
        self.lines.insert(Self::tx_key(0), line);
        self.tx_switch(0, current, 0)?;
        self.armed = true;
        Ok(())
    }

    fn start(&mut self, at: Option<TimePoint>) -> Result<(), ModuleError> {
        if !self.armed { return Err(self.reject("MR-11: arm must precede start")); }
        let root = self.root.expect("prepared root");
        let clocks = self.clocks.as_ref().expect("prepared clocks");
        let time = self.time.as_ref().expect("prepared time").clone();
        let now = time.now(root).and_then(|t| t.ticks_in(root)).map_err(|error| self.reject(format!("MR-11: {error}")))?;
        let target = match at { Some(at) => time::to_v(clocks, root, at).map_err(|error| self.reject(format!("MR-11: {error}")))?, None => now };
        let sync_end = self.sync_end.unwrap_or(now);
        if target < sync_end {
            self.emit_event("", ezsdr_radio::kinds::LATE_COMMAND, Severity::Warning, serde_json::to_value(ezsdr_radio::payloads::LateCommandPayload { key: None, requested: TimePoint::new(root, target), applied: TimePoint::new(root, sync_end) }).expect("late command payload"), TimePoint::new(root, now))?;
            return Err(self.reject(format!("MR-11: the start at {target} precedes synchronisation at {sync_end}; the profile needs ezsdr.time.start_lead_ns ≥ {}", self.profile.timing().startup_latency_ns)));
        }
        let fault_offsets: Vec<i64> = self.faults.iter().map(|fault| fault.tick).collect();
        let fault_ticks: Vec<i64> = fault_offsets.iter().map(|offset| {
            target.checked_add(*offset).ok_or_else(|| ModuleError::rejected("MR-20: fault time overflow"))
        }).collect::<Result<_, _>>()?;
        // RM-21, RM-25: a receive stream with a link runs from T0, its first segment's clock
        // registered at its first block.
        if !self.rx[0].links.is_empty() {
            let line = self.line(Direction::Rx, target, &self.config)?;
            let first = line.plan.first().copied();
            self.lines.insert(Self::rx_key(0), line);
            if let Some(segment) = first {
                self.rx_begin(0, segment, self.rx[0].clock.clone())?;
            }
        }
        for (fault, tick) in self.faults.iter_mut().zip(fault_ticks) { fault.tick = tick; }
        self.start_tick = Some(target);
        self.started = true;
        if let Err(error) = self.schedule_wakeup() {
            if let Some(handle) = self.wakeup.take() { time.cancel(handle); }
            self.started = false;
            self.start_tick = None;
            // A start that failed leaves no receive stream behind: the instance's lines are
            // rebuilt without the receive one, and the transmit stream's line — booked at
            // `arm`, before `start` — is put back. Setup only, and this stage books no move,
            // so the rebuild drops nothing else (RM-34).
            let tx_line = self.lines.line(Self::tx_key(0)).cloned();
            self.lines = timeline::Lines::new();
            if let Some(line) = tx_line { self.lines.insert(Self::tx_key(0), line); }
            self.rx[0].running = None;
            self.rx[0].from = i64::MIN;
            self.rx[0].pool = None;
            for (fault, offset) in self.faults.iter_mut().zip(fault_offsets) { fault.tick = offset; }
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
                other => { other?; }
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
        self.stop_at(mode, now, "MR-25: cancelled by stop", seq, false)
    }

    fn cleanup(&mut self) {
        if self.root.is_some() {
            self.record_pending_faults();
        }
        if let (Some(time), Some(handle)) = (&self.time, self.wakeup.take()) { time.cancel(handle); }
        self.started = false;
        self.armed = false;
        self.prepared = false;
        self.actions = None;
        self.lines = timeline::Lines::new();
        let (rx, tx) = Self::streams(&self.instance.id);
        self.rx = rx;
        self.tx = tx;
        self.wakeup = None;
        self.updates.clear();
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
        if !self.prepared { return Ok(StepOutcome { progressed: false }); }
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
        let loss_due = self.started
            && self.faults.iter().any(|fault| !fault.resolved && fault.entry.fault == FaultKind::DeviceLost && fault.tick <= u);
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
