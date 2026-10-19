//! A controlled Provider clock for command-order tests without host deadlines.
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use ezsdr_kernel::event::{Event, EventCollector, EventHandle, EventKind, EventSink, EventSource, Severity, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{Fidelity, ModuleRegistry, Pacing};
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::time::{ClockDomain, ClockRegistry, EpochRef, ManualTimeAuthority, Rational, TimeAuthority, TimePoint};
use ezsdr_kernel::run::RunError;
use crate::device::{Applied, Device, DeviceError, Dir, FakeConfig, FakeDevice, Iq, RxRecv, Settings, TxReport};
use crate::host::HostClock;
use super::core::Core;

pub(super) fn rig() -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    rig_with_links(Vec::new())
}

/// `rig` with the receive stream's links.
pub(super) fn rig_with_links(links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>)
    -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    rig_with(FakeConfig::default(), links)
}

/// `rig_with_links` on a FakeDevice of `config`.
pub(super) fn rig_with(config: FakeConfig, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>)
    -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    let device = Arc::new(FakeDevice::new(config));
    let (core, time, events) = rig_on(device.clone(), links, 2_000);
    (core, device, time, events)
}

/// `rig` with the Run's Command inputs, for a test that books a waveform (UR-21: a burst's
/// bytes must be an input of its Run).
pub(super) fn rig_with_inputs(inputs: BTreeMap<ContentHash, Arc<[u8]>>)
    -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    let device = Arc::new(FakeDevice::new(FakeConfig::default()));
    let (core, time, events) = rig_on_inputs(device.clone(), Vec::new(), 2_000, inputs);
    (core, device, time, events)
}

/// The rig on any device, with `block_len`.
pub(super) fn rig_on(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, block_len: u32)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    rig_on_inputs(device, links, block_len, BTreeMap::new())
}

/// `rig_on` with the Run's Command inputs.
pub(super) fn rig_on_inputs(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, block_len: u32, inputs: BTreeMap<ContentHash, Arc<[u8]>>)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    rig_on_mode(device, links, inputs, super::mode::Mode::Current(Box::new(crate::profile::ubx_description(block_len))))
}

/// `rig_on_inputs` with the key set the instance presents (RM-27): the current keys, or
/// the layout builder's, whose stream nodes are `<id>/rx/s<k>` and `<id>/tx/s<k>`.
pub(super) fn rig_on_mode(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, mode: super::mode::Mode)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    let clocks = Arc::new(ClockRegistry::new());
    let root = clocks.allocate_id().unwrap();
    clocks.register(ClockDomain::root(root, Rational::new(200_000_000, 1).unwrap(),
        EpochRef::Arbitrary { set_by: "uhd.test".to_owned() })).unwrap();
    let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut ModuleRegistry::new(),
        &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
    // The instance's own nodes, under either key set (RM-29): a layout's events come from
    // its stream nodes.
    let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1"].iter().flat_map(|s|
        kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
    let core = core_on(device, clocks, time.clone(), events.clone(), inputs, links, mode);
    (core, time, events)
}

/// `rig_on_mode` through a sink that witnesses which events are emitted while the line lock
/// is held (UR-25): the test keeps the witness and reads it.
pub(super) fn rig_on_mode_witness(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, mode: super::mode::Mode)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<SectionWitness>) {
    let clocks = Arc::new(ClockRegistry::new());
    let root = clocks.allocate_id().unwrap();
    clocks.register(ClockDomain::root(root, Rational::new(200_000_000, 1).unwrap(),
        EpochRef::Arbitrary { set_by: "uhd.test".to_owned() })).unwrap();
    let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut ModuleRegistry::new(),
        &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx", "usrp/tx", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1"].iter().flat_map(|s|
        kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let witness = Arc::new(SectionWitness::new(Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()))));
    let core = core_on(device, clocks, time.clone(), witness.clone(), inputs, links, mode);
    witness.watch(&core);
    (core, time, witness)
}

/// The rig's `Core` on `sink`.
fn core_on(device: Arc<dyn Device>, clocks: Arc<ClockRegistry>, time: Arc<ManualTimeAuthority>, events: Arc<dyn EventSink>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, mode: super::mode::Mode) -> Arc<Core> {
    let root = time.primary_root();
    Arc::new(Core::new(device, ResourceId::parse("usrp").unwrap(),
        root, time, clocks, events, Arc::new(inputs), links, mode)
        .expect("the rx node is inside the instance root"))
}

/// A target the Kernel would resolve to `node` (KC-23): its first segment as the resource
/// and the rest as the path. A Provider matches on the node it is handed, not on this.
pub(super) fn target_of(node: &ResourceId) -> Target {
    let (resource, path) = node.path().split_once('/').unwrap_or((node.path(), ""));
    Target::Resource { resource: ezsdr_kernel::spec::Ident::parse(resource).unwrap(), path: path.to_owned() }
}

thread_local! {
    static LOCKS: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
}

/// Starts recording the addresses of the line locks this thread takes (UR-25).
pub(super) fn record_locks() {
    LOCKS.with(|locks| *locks.borrow_mut() = Some(Vec::new()));
}

/// The addresses of the locks taken since `record_locks`, in order, and stops recording.
pub(super) fn taken_locks() -> Vec<usize> {
    LOCKS.with(|locks| locks.borrow_mut().take().unwrap_or_default())
}

/// `core::lock`'s record of one acquisition, by the mutex's address (UR-25, #63).
pub(super) fn note_lock(address: usize) {
    LOCKS.with(|locks| {
        if let Some(taken) = locks.borrow_mut().as_mut() {
            taken.push(address);
        }
    });
}

/// A `FakeDevice` that fails its test when a device call is made under the Provider's line
/// lock (UR-25, design 33 D15): the rigs are single-threaded, so a lock the spy cannot take
/// is its own caller's. It counts each call, so a test can prove its sequence passed one.
pub(super) struct LockSpy {
    inner: FakeDevice,
    core: Mutex<Weak<Core>>,
    calls: Mutex<BTreeMap<&'static str, usize>>,
    hosts: Mutex<Vec<bool>>,
}

impl LockSpy {
    pub fn new(config: FakeConfig) -> LockSpy {
        LockSpy { inner: FakeDevice::new(config), core: Mutex::new(Weak::new()), calls: Mutex::new(BTreeMap::new()), hosts: Mutex::new(Vec::new()) }
    }

    /// The Core this device serves, taken once the rig has built it.
    pub fn watch(&self, core: &Arc<Core>) {
        *self.core.lock().unwrap() = Arc::downgrade(core);
    }

    /// How many calls of `name` the spy saw.
    pub fn seen(&self, name: &str) -> usize {
        self.calls.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    /// Whether the line lock was held at each host clock read since `watch`, in order (UR-25).
    pub fn host_held(&self) -> Vec<bool> {
        self.hosts.lock().unwrap().clone()
    }

    /// Records the call and fails when its caller holds the streams' lock.
    fn enter(&self, name: &'static str) {
        *self.calls.lock().unwrap().entry(name).or_default() += 1;
        if let Some(core) = self.core.lock().unwrap().upgrade() {
            assert!(core.streams.try_lock().is_ok(), "{name} was made under the streams' lock (UR-25)");
        }
    }
}

impl Device for LockSpy {
    fn describe(&self) -> serde_json::Value { self.inner.describe() }
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
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> { self.enter("apply"); self.inner.apply(dir, chan, s, at) }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.enter("rx_open"); self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.enter("rx_start"); self.inner.rx_start(stream, at) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.enter("rx_packet_samples"); self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.enter("rx_stop"); self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration) -> RxRecv { self.enter("rx_recv"); self.inner.rx_recv(stream, n, timeout) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.enter("tx_open"); self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.enter("tx_send");
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> { self.enter("tx_async"); self.inner.tx_async(stream, timeout) }
    fn close_streams(&self) { self.enter("close_streams"); self.inner.close_streams() }
    fn mark_lost(&self) { self.enter("mark_lost"); self.inner.mark_lost() }
    fn host(&self) -> HostClock {
        if let Some(core) = self.core.lock().unwrap().upgrade() {
            self.hosts.lock().unwrap().push(core.streams.try_lock().is_err());
        }
        self.inner.host()
    }
}

/// A `FakeDevice` with one scripted refusal `FakeConfig::faults` does not carry (UR-33's
/// vocabulary keeps to what a device does): the first untimed `apply` of `dir` — of the
/// device channel `chan`, when one is named — fails before anything is applied, which a set
/// whose channels are applied one by one and a configuration of a streamer already open need
/// (UR-13). The fake behind it is unchanged by the refused call.
pub(super) struct ApplyFails {
    inner: Arc<FakeDevice>,
    dir: Dir,
    chan: Option<usize>,
    fired: AtomicBool,
}

impl ApplyFails {
    pub fn new(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> ApplyFails {
        ApplyFails { inner, dir, chan, fired: AtomicBool::new(false) }
    }
}

impl Device for ApplyFails {
    fn describe(&self) -> serde_json::Value { self.inner.describe() }
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
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        if dir == self.dir && at.is_none() && self.chan.is_none_or(|want| want == chan) && !self.fired.swap(true, Ordering::SeqCst) {
            return Err(DeviceError::failed("test: scripted refusal of an apply"));
        }
        self.inner.apply(dir, chan, s, at)
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration) -> RxRecv { self.inner.rx_recv(stream, n, timeout) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> { self.inner.tx_async(stream, timeout) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> HostClock { self.inner.host() }
}

/// A `FakeDevice` whose asynchronous reports the test writes: `tx_async` returns the reports
/// given to it before any the fake holds, so a test puts a report on a streamer's queue at the
/// point its owner reads it (UR-28). Everything else is the fake's.
pub(super) struct ReportFeed {
    inner: Arc<FakeDevice>,
    given: Mutex<VecDeque<TxReport>>,
}

impl ReportFeed {
    pub fn new(inner: Arc<FakeDevice>) -> ReportFeed {
        ReportFeed { inner, given: Mutex::new(VecDeque::new()) }
    }

    /// Queues `report` for the next `tx_async`.
    pub fn give(&self, report: TxReport) {
        self.given.lock().unwrap().push_back(report);
    }
}

impl Device for ReportFeed {
    fn describe(&self) -> serde_json::Value { self.inner.describe() }
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
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> { self.inner.apply(dir, chan, s, at) }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration) -> RxRecv { self.inner.rx_recv(stream, n, timeout) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> {
        let given = self.given.lock().unwrap().pop_front();
        match given {
            Some(report) => Ok(Some(report)),
            None => self.inner.tx_async(stream, timeout),
        }
    }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> HostClock { self.inner.host() }
}

/// An event sink that records whether each control-path event was emitted while the
/// instance's line lock was held (UR-25): the witness for a command whose instant and
/// booking are one critical section — no other section of the Provider emits inside it, and
/// the rigs are single-threaded, so the lock it meets is its own caller's.
pub(super) struct SectionWitness {
    inner: Arc<EventCollector>,
    core: Mutex<Weak<Core>>,
    seen: Mutex<Vec<(String, bool)>>,
}

impl SectionWitness {
    pub fn new(inner: Arc<EventCollector>) -> SectionWitness {
        SectionWitness { inner, core: Mutex::new(Weak::new()), seen: Mutex::new(Vec::new()) }
    }

    /// The Core this sink serves, taken once the rig has built it.
    pub fn watch(&self, core: &Arc<Core>) {
        *self.core.lock().unwrap() = Arc::downgrade(core);
    }

    /// The events emitted since the last call, each with whether the line lock was held.
    pub fn events(&self) -> Vec<(String, bool)> {
        std::mem::take(&mut *self.seen.lock().unwrap())
    }

    /// Records the event's kind and whether the line lock is held now.
    fn observe(&self, kind: &EventKind) {
        let held = self.core.lock().unwrap().upgrade().is_some_and(|core| core.streams.try_lock().is_err());
        self.seen.lock().unwrap().push((kind.as_str().to_owned(), held));
    }
}

impl EventSink for SectionWitness {
    fn resolve(&self, source: &EventSource, kind: &EventKind) -> Result<EventHandle, RunError> {
        self.inner.resolve(source, kind)
    }

    fn emit(&self, handle: EventHandle, time: TimePoint, severity: Severity, payload: &[u8]) -> Result<(), RunError> {
        self.inner.emit(handle, time, severity, payload)
    }

    fn emit_control(&self, event: Event) -> Result<(), RunError> {
        self.observe(&event.kind);
        self.inner.emit_control(event)
    }
}
