//! A controlled Provider clock for command-order tests without host deadlines.
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration as Wall;
use ezsdr_kernel::event::{Event, EventCollector, EventHandle, EventKind, EventSink, EventSource, Severity, Target};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::ResourceId;
use ezsdr_kernel::module_api::{Fidelity, ModuleRegistry, Pacing};
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Ident, Scalar, Value};
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
    rig_on_description(device, links, inputs, default_description(block_len))
}

pub(super) fn default_description(block_len: u32) -> ezsdr_radio::channels::Description {
    let selector = BTreeMap::from([
        (Ident::parse("rx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
        (Ident::parse("tx").unwrap(), Value::List(vec![Scalar::from("0:A:ubx:0"), Scalar::from("0:B:ubx:0")])),
    ]);
    let layout = ezsdr_radio::channels::from_selector(&selector).unwrap();
    ezsdr_radio::channels::Description::new(&layout, block_len, &crate::profile::terms())
}

/// `rig_on_inputs` with a specific layout description.
pub(super) fn rig_on_description(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, description: ezsdr_radio::channels::Description)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    rig_on_description_links(device, vec![links], inputs, description)
}

/// `rig_on_description` with each receive stream's links (RM-29): index `k` is stream `k`'s.
pub(super) fn rig_on_description_links(device: Arc<dyn Device>, links: Vec<Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, description: ezsdr_radio::channels::Description)
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
    let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"].iter().flat_map(|s|
        kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
    let core = core_on(device, clocks, time.clone(), events.clone(), inputs, links, description);
    (core, time, events)
}

/// A layout rig with `links` on receive stream `k` alone.
pub(super) fn rig_with_links_on(k: usize, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, description: ezsdr_radio::channels::Description)
    -> (Arc<Core>, Arc<FakeDevice>, Arc<ManualTimeAuthority>, Arc<EventCollector>) {
    let device = Arc::new(FakeDevice::new(FakeConfig::default()));
    let mut per_stream: Vec<Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>> = Vec::new();
    per_stream.resize_with(k + 1, Vec::new);
    per_stream[k] = links;
    let (core, time, events) = rig_on_description_links(device.clone(), per_stream, BTreeMap::new(), description);
    (core, device, time, events)
}

/// `rig_on_description` through a sink that witnesses which events are emitted while the line lock
/// is held (UR-25): the test keeps the witness and reads it.
pub(super) fn rig_on_description_witness(device: Arc<dyn Device>, links: Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, description: ezsdr_radio::channels::Description)
    -> (Arc<Core>, Arc<ManualTimeAuthority>, Arc<SectionWitness>) {
    let clocks = Arc::new(ClockRegistry::new());
    let root = clocks.allocate_id().unwrap();
    clocks.register(ClockDomain::root(root, Rational::new(200_000_000, 1).unwrap(),
        EpochRef::Arbitrary { set_by: "uhd.test".to_owned() })).unwrap();
    let time = Arc::new(ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning).unwrap());
    let mut kinds = EventKindRegistry::with_kernel_kinds();
    ezsdr_radio::register(&mut ModuleRegistry::new(),
        &mut ezsdr_kernel::binding::AdmissionCheckRegistry::new(), &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0", "usrp/rx/s1", "usrp/tx/s1", "usrp/rx/s2", "usrp/tx/s2", "usrp/rx/s3", "usrp/tx/s3"].iter().flat_map(|s|
        kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let witness = Arc::new(SectionWitness::new(Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()))));
    let core = core_on(device, clocks, time.clone(), witness.clone(), inputs, vec![links], description);
    witness.watch(&core);
    (core, time, witness)
}

/// The rig's `Core` on `sink`.
fn core_on(device: Arc<dyn Device>, clocks: Arc<ClockRegistry>, time: Arc<ManualTimeAuthority>, events: Arc<dyn EventSink>, inputs: BTreeMap<ContentHash, Arc<[u8]>>, links: Vec<Vec<Arc<dyn ezsdr_kernel::stream::DataLink>>>, description: ezsdr_radio::channels::Description) -> Arc<Core> {
    let root = time.primary_root();
    Arc::new(Core::new(device, ResourceId::parse("usrp").unwrap(),
        root, time, clocks, events, Arc::new(inputs), links, description)
        .expect("the rx node is inside the instance root"))
}

/// A target the Kernel would resolve to `node` (KC-23): its first segment as the resource
/// and the rest as the path. A Provider matches on the node it is handed, not on this.
pub(super) fn target_of(node: &ResourceId) -> Target {
    let (resource, path) = node.path().split_once('/').unwrap_or((node.path(), ""));
    Target::Resource { resource: ezsdr_kernel::spec::Ident::parse(resource).unwrap(), path: path.to_owned() }
}

/// A device's host clock held still while this lives (M3): a participant of its own that stays
/// busy, so that a test reads the device's settings at an instant of its own and no real time
/// moves them. On a manual clock alone; the drop gives the clock back.
pub(super) struct Frozen {
    release: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Frozen {
    pub fn of(device: &FakeDevice) -> Frozen {
        Frozen::on(device.host())
    }

    pub fn on(clock: HostClock) -> Frozen {
        assert!(clock.is_manual(), "a clock is frozen on a manual clock alone");
        let (release, wait) = channel();
        let thread = clock.spawn("frozen", move || { let _ = wait.recv(); }).expect("a participant thread");
        Frozen { release: Some(release), thread: Some(thread) }
    }
}

impl Drop for Frozen {
    fn drop(&mut self) {
        let _ = self.release.take().expect("a held clock").send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The device's clock moved to `ticks`, with its queue's commands due by then applied, and
/// asserted to be there: the two clocks a test reads are the Core's, which it advances, and
/// the device's, which it fixes.
pub(super) fn device_at(core: &Core, time: &ManualTimeAuthority, device: &FakeDevice, ticks: i64) {
    if core.now() < ticks {
        time.advance_to(core.at(ticks)).expect("the Core's clock reaches the device's instant");
    }
    device.set_time(ticks);
    assert_eq!((core.now(), device.time_now().unwrap()), (ticks, ticks), "the Core's clock and the device's are not at the same instant");
}

thread_local! {
    static LOCKS: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
    /// The line guards this thread holds (UR-25): what `LockSpy` and `assert_line_free` see,
    /// where a `try_lock` can only see some other thread holding the line.
    static LINE_DEPTH: Cell<usize> = const { Cell::new(0) };
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

/// One line guard taken by this thread.
pub(super) fn entered_lock() {
    LINE_DEPTH.with(|depth| depth.set(depth.get() + 1));
}

/// One line guard dropped by this thread.
pub(super) fn left_lock() {
    LINE_DEPTH.with(|depth| depth.set(depth.get() - 1));
}

/// UR-25's order: the caller holds no line guard, so no `updates` may be taken.
pub(super) fn assert_line_free(what: &str) {
    assert_eq!(LINE_DEPTH.with(Cell::get), 0, "{what} was made under the line lock (UR-25)");
}

/// Registers the probe `name` on `core`'s test hooks (UR-25, UR-29): a section a test holds
/// open, or reads while the section's own state is as the rule expects.
pub(super) fn probe(core: &Core, name: &'static str, f: impl Fn(Option<&super::core::Streams>) + Send + Sync + 'static) {
    core.probes.lock().unwrap().insert(name, Arc::new(f));
}

/// A gated device call's handshake: the call is held until the test releases it.
pub(super) struct Gate {
    reached: Receiver<()>,
    release: Sender<()>,
}

impl Gate {
    /// Waits until the gated call is reached.
    pub fn wait(&self) {
        assert!(self.reached.recv_timeout(Wall::from_secs(10)).is_ok(), "the gated call was not reached within 10 s");
    }

    /// Lets the gated call go on.
    pub fn release(self) {
        let _ = self.release.send(());
    }
}

/// A gate the spy holds a device call at.
struct Hold {
    reached: Sender<()>,
    release: Receiver<()>,
}

/// The direction a device call belongs to, for the gate's "no other call of this direction
/// while one is held" check.
fn family(name: &str) -> Option<Dir> {
    match name {
        "rx_open" | "rx_close" | "rx_start" | "rx_start_now" | "rx_recv" | "rx_packet_samples" | "rx_stop" => Some(Dir::Rx),
        "tx_open" | "tx_close" | "tx_send" | "tx_async" => Some(Dir::Tx),
        _ => None,
    }
}

/// A `FakeDevice` that fails its test when a device call is made under the Provider's line
/// lock (UR-25, design 33 D15): the test build counts the caller's own guards, so a line
/// another thread holds is not mistaken for the caller's. It counts each call, so a test can
/// prove its sequence passed one, and it can hold a chosen call until the test lets it go.
type RxApplyPending = (usize, Option<i64>, f64, usize);

pub(super) struct LockSpy {
    inner: FakeDevice,
    core: Mutex<Weak<Core>>,
    calls: Mutex<BTreeMap<&'static str, usize>>,
    order: Mutex<Vec<&'static str>>,
    rx_requests: Mutex<Vec<usize>>,
    rx_one_packet: Mutex<Vec<bool>>,
    rx_packet_max: Mutex<Option<usize>>,
    rx_apply_pending: Mutex<Vec<RxApplyPending>>,
    hosts: Mutex<Vec<bool>>,
    before: Mutex<BTreeMap<&'static str, Hold>>,
    after: Mutex<BTreeMap<&'static str, Hold>>,
    rx_apply_error: Mutex<Option<DeviceError>>,
    rx_start_error: Mutex<Option<DeviceError>>,
    rx_stop_error: Mutex<Option<DeviceError>>,
    /// The call a gate holds now: no other call of its direction may enter meanwhile.
    holding: Mutex<Option<&'static str>>,
}

impl LockSpy {
    pub fn new(config: FakeConfig) -> LockSpy {
        LockSpy {
            inner: FakeDevice::new(config),
            core: Mutex::new(Weak::new()),
            calls: Mutex::new(BTreeMap::new()),
            order: Mutex::new(Vec::new()),
            rx_requests: Mutex::new(Vec::new()),
            rx_one_packet: Mutex::new(Vec::new()),
            rx_packet_max: Mutex::new(None),
            rx_apply_pending: Mutex::new(Vec::new()),
            hosts: Mutex::new(Vec::new()),
            before: Mutex::new(BTreeMap::new()),
            after: Mutex::new(BTreeMap::new()),
            rx_apply_error: Mutex::new(None),
            rx_start_error: Mutex::new(None),
            rx_stop_error: Mutex::new(None),
            holding: Mutex::new(None),
        }
    }

    /// The Core this device serves, taken once the rig has built it.
    pub fn watch(&self, core: &Arc<Core>) {
        *self.core.lock().unwrap() = Arc::downgrade(core);
    }

    /// How many calls of `name` the spy saw.
    pub fn seen(&self, name: &str) -> usize {
        self.calls.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    /// Device call order, including calls the underlying fake does not record.
    pub fn order(&self) -> Vec<&'static str> {
        self.order.lock().unwrap().clone()
    }

    /// Sample count requested by each receive call, in order.
    pub fn rx_requests(&self) -> Vec<usize> {
        self.rx_requests.lock().unwrap().clone()
    }

    pub fn rx_packet_modes(&self) -> Vec<bool> {
        self.rx_one_packet.lock().unwrap().clone()
    }

    pub fn report_rx_packet_max(&self, samples: usize) {
        *self.rx_packet_max.lock().unwrap() = Some(samples);
    }

    pub fn rx_apply_pending(&self) -> Vec<RxApplyPending> {
        self.rx_apply_pending.lock().unwrap().clone()
    }

    /// Every call the fake behind the spy saw, in order.
    pub fn log(&self) -> Vec<String> {
        self.inner.calls()
    }

    pub fn dsp_pending(&self, dir: Dir, chan: usize) -> usize {
        self.inner.dsp_pending(dir, chan)
    }

    pub fn dsp_log(&self, dir: Dir, chan: usize) -> Vec<(i64, f64, bool)> {
        self.inner.dsp_log(dir, chan)
    }

    pub fn set_time(&self, ticks: i64) {
        self.inner.set_time(ticks);
    }

    pub fn time_now(&self) -> Result<i64, DeviceError> {
        self.inner.time_now()
    }

    /// Whether the line lock was held at each host clock read since `watch`, in order (UR-25).
    pub fn host_held(&self) -> Vec<bool> {
        self.hosts.lock().unwrap().clone()
    }

    /// Holds the next call of `name` before it reaches the device (UR-25).
    pub fn stop_before(&self, name: &'static str) -> Gate {
        let (gate, hold) = gate();
        self.before.lock().unwrap().insert(name, hold);
        gate
    }

    /// Holds the next call of `name` after the device returned (UR-29).
    pub fn stop_after(&self, name: &'static str) -> Gate {
        let (gate, hold) = gate();
        self.after.lock().unwrap().insert(name, hold);
        gate
    }

    pub fn refuse_rx_stop(&self, error: DeviceError) {
        *self.rx_stop_error.lock().unwrap() = Some(error);
    }

    pub fn refuse_rx_apply(&self, error: DeviceError) {
        *self.rx_apply_error.lock().unwrap() = Some(error);
    }

    pub fn refuse_rx_start(&self, error: DeviceError) {
        *self.rx_start_error.lock().unwrap() = Some(error);
    }

    /// Records the call, fails when its caller holds the line, and holds it at its gate.
    fn enter(&self, name: &'static str) {
        *self.calls.lock().unwrap().entry(name).or_default() += 1;
        self.order.lock().unwrap().push(name);
        assert_line_free(name);
        if let Some(other) = *self.holding.lock().unwrap() {
            if other != name && family(other).is_some() && family(other) == family(name) {
                panic!("{name} entered while {other} of its direction is held (UR-25)");
            }
        }
        let hold = self.before.lock().unwrap().remove(name);
        if let Some(hold) = hold {
            *self.holding.lock().unwrap() = Some(name);
            let _ = hold.reached.send(());
            let _ = hold.release.recv();
            *self.holding.lock().unwrap() = None;
        }
    }

    /// Holds the call after the device returned.
    fn leave(&self, name: &'static str) {
        let hold = self.after.lock().unwrap().remove(name);
        if let Some(hold) = hold {
            let _ = hold.reached.send(());
            let _ = hold.release.recv();
        }
    }

    fn call<T>(&self, name: &'static str, f: impl FnOnce() -> T) -> T {
        self.enter(name);
        let value = f();
        self.leave(name);
        value
    }
}

/// A gate and its hold.
fn gate() -> (Gate, Hold) {
    let (reached, reached_rx) = channel();
    let (release, release_rx) = channel();
    (Gate { reached: reached_rx, release }, Hold { reached, release: release_rx })
}

impl Device for LockSpy {
    fn describe(&self) -> serde_json::Value { self.inner.describe() }
    fn fidelity(&self) -> Fidelity { self.inner.fidelity() }
    fn master_clock_rate(&self) -> u64 { self.inner.master_clock_rate() }
    fn channels(&self, dir: Dir) -> usize { self.inner.channels(dir) }
    fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError> { self.inner.front_end(dir, chan) }
    fn serial(&self) -> Result<String, DeviceError> { self.inner.serial() }
    fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError> { self.inner.set_subdev_spec(dir, markup) }
    fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError> {
        self.call("rates", || self.inner.rates(dir, chan))
    }
    fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError> { self.inner.set_sources(clock, time) }
    fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError> { self.inner.set_time_zero(at_next_pps) }
    fn time_now(&self) -> Result<i64, DeviceError> { self.inner.time_now() }
    fn ref_locked(&self) -> Result<Option<bool>, DeviceError> { self.inner.ref_locked() }
    fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError> {
        self.call("apply", || {
            if dir == Dir::Rx {
                if let Some(freq) = s.freq { self.rx_apply_pending.lock().unwrap().push((chan, at, freq, self.inner.dsp_pending(dir, chan))); }
                if let Some(error) = self.rx_apply_error.lock().unwrap().take() { return Err(error); }
            }
            self.inner.apply(dir, chan, s, at)
        })
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.call("rx_open", || self.inner.rx_open(stream, channels)) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> {
        self.call("rx_start", || match self.rx_start_error.lock().unwrap().take() {
            Some(error) => Err(error),
            None => self.inner.rx_start(stream, at),
        })
    }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.call("rx_start_now", || self.inner.rx_start_now(stream)) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.call("rx_packet_samples", || self.rx_packet_max.lock().unwrap().unwrap_or_else(|| self.inner.rx_packet_samples(stream))) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> {
        self.call("rx_stop", || match self.rx_stop_error.lock().unwrap().take() {
            Some(error) => Err(error),
            None => self.inner.rx_stop(stream, at),
        })
    }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration, one_packet: bool) -> RxRecv {
        self.rx_requests.lock().unwrap().push(n);
        self.rx_one_packet.lock().unwrap().push(one_packet);
        self.call("rx_recv", || self.inner.rx_recv(stream, n, timeout, one_packet))
    }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.call("tx_open", || self.inner.tx_open(stream, channels)) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.call("tx_send", || self.inner.tx_send(stream, samples, at, sob, eob, timeout))
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> { self.call("tx_async", || self.inner.tx_async(stream, timeout)) }
    fn rx_close(&self, stream: usize) { self.call("rx_close", || self.inner.rx_close(stream)) }
    fn tx_close(&self, stream: usize) { self.call("tx_close", || self.inner.tx_close(stream)) }
    fn close_streams(&self) { self.call("close_streams", || self.inner.close_streams()) }
    fn mark_lost(&self) { self.call("mark_lost", || self.inner.mark_lost()) }
    fn raise_thread_priority(&self) -> Result<(), DeviceError> { self.inner.raise_thread_priority() }
    fn host(&self) -> HostClock {
        // Only once the rig has watched: `Control::new` reads the clock before the test does.
        if self.core.lock().unwrap().upgrade().is_some() {
            self.hosts.lock().unwrap().push(LINE_DEPTH.with(Cell::get) > 0);
        }
        self.inner.host()
    }
}

/// A `FakeDevice` with one scripted refusal `FakeConfig::faults` does not carry (UR-33's
/// vocabulary keeps to what a device does): the first `apply` of `dir` of the class named — of
/// the device channel `chan`, when one is named — fails before anything is applied, which a set
/// whose channels are applied one by one and a configuration of a streamer already open need
/// (UR-13). The fake behind it is unchanged by the refused call. Every `apply` it was given is
/// kept, in order, so a test sees an attempt a refusal cut short.
pub(super) struct ApplyFails {
    inner: Arc<FakeDevice>,
    dir: Dir,
    chan: Option<usize>,
    /// The class it refuses: `Some(true)` a timed `apply`, `Some(false)` an untimed one.
    timed: Option<bool>,
    /// Whether the refusal is a lost device (`DeviceError::lost`).
    lost: bool,
    fired: AtomicBool,
    attempts: Mutex<Vec<(Dir, usize, Option<i64>)>>,
}

impl ApplyFails {
    pub fn new(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> ApplyFails {
        ApplyFails::of(inner, dir, chan, false, false)
    }

    /// The first untimed `apply` of `dir` — of `chan`, when one is named — fails as a lost device.
    pub fn lost(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> ApplyFails {
        ApplyFails::of(inner, dir, chan, false, true)
    }

    /// The first timed `apply` of `dir` — of `chan`, when one is named — fails.
    pub fn timed(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> ApplyFails {
        ApplyFails::of(inner, dir, chan, true, false)
    }

    /// The first timed `apply` of `dir` — of `chan`, when one is named — fails as a lost device.
    pub fn timed_lost(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> ApplyFails {
        ApplyFails::of(inner, dir, chan, true, true)
    }

    fn of(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>, timed: bool, lost: bool) -> ApplyFails {
        ApplyFails { inner, dir, chan, timed: Some(timed), lost, fired: AtomicBool::new(false), attempts: Mutex::new(Vec::new()) }
    }

    /// Every `apply` the wrapper was given, in order, as the test sees an attempt: a refusal
    /// and an apply the device never took alike.
    pub fn attempts(&self) -> Vec<(Dir, usize, Option<i64>)> {
        self.attempts.lock().unwrap().clone()
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
        self.attempts.lock().unwrap().push((dir, chan, at));
        if dir == self.dir && Some(at.is_some()) == self.timed && self.chan.is_none_or(|want| want == chan) && !self.fired.swap(true, Ordering::SeqCst) {
            return if self.lost {
                Err(DeviceError { lost: true, message: "test: scripted loss on an apply".to_owned() })
            } else {
                Err(DeviceError::failed("test: scripted refusal of an apply"))
            };
        }
        self.inner.apply(dir, chan, s, at)
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.inner.rx_start_now(stream) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration, one_packet: bool) -> RxRecv { self.inner.rx_recv(stream, n, timeout, one_packet) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> { self.inner.tx_async(stream, timeout) }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn raise_thread_priority(&self) -> Result<(), crate::DeviceError> { self.inner.raise_thread_priority() }
    fn host(&self) -> HostClock { self.inner.host() }
}

/// A `FakeDevice` whose untimed `apply` of one direction and channel is held after the
/// device returned (L1, L2, S16a in the unit rigs): the test publishes a loss while the
/// owner is between its configuration and its start.
pub(super) struct HoldApply {
    inner: Arc<FakeDevice>,
    dir: Dir,
    chan: Option<usize>,
    reached: Mutex<Option<Sender<()>>>,
    release: Mutex<Option<Receiver<()>>>,
    pending: Mutex<bool>,
}

impl HoldApply {
    pub fn new(inner: Arc<FakeDevice>, dir: Dir, chan: Option<usize>) -> HoldApply {
        HoldApply { inner, dir, chan, reached: Mutex::new(None), release: Mutex::new(None), pending: Mutex::new(false) }
    }

    /// Arms the gate for the next untimed apply of its direction and channel.
    pub fn arm(&self) -> (Receiver<()>, Sender<()>) {
        let (reached, reached_rx) = channel();
        let (release, release_rx) = channel();
        *self.reached.lock().unwrap() = Some(reached);
        *self.release.lock().unwrap() = Some(release_rx);
        *self.pending.lock().unwrap() = true;
        (reached_rx, release)
    }
}

impl Device for HoldApply {
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
        let applied = self.inner.apply(dir, chan, s, at)?;
        let held = at.is_none() && dir == self.dir && self.chan.is_none_or(|want| want == chan) && std::mem::replace(&mut *self.pending.lock().unwrap(), false);
        if held {
            if let Some(reached) = self.reached.lock().unwrap().take() {
                let _ = reached.send(());
            }
            if let Some(release) = self.release.lock().unwrap().take() {
                let _ = release.recv_timeout(Wall::from_secs(10));
            }
        }
        Ok(applied)
    }
    fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.rx_open(stream, channels) }
    fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError> { self.inner.rx_start(stream, at) }
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.inner.rx_start_now(stream) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration, one_packet: bool) -> RxRecv { self.inner.rx_recv(stream, n, timeout, one_packet) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> { self.inner.tx_async(stream, timeout) }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn raise_thread_priority(&self) -> Result<(), crate::DeviceError> { self.inner.raise_thread_priority() }
    fn host(&self) -> HostClock { self.inner.host() }
}

/// A `FakeDevice` whose asynchronous reports the test writes: `tx_async` returns the reports
/// given to it before any the fake holds, so a test puts a report on a streamer's queue at the
/// point its owner reads it (UR-28). Everything else is the fake's.
pub(super) struct ReportFeed {
    inner: Arc<FakeDevice>,
    given: Mutex<VecDeque<TxReport>>,
    reads: Mutex<[usize; 2]>,
}

impl ReportFeed {
    pub fn new(inner: Arc<FakeDevice>) -> ReportFeed {
        ReportFeed { inner, given: Mutex::new(VecDeque::new()), reads: Mutex::new([0, 0]) }
    }

    /// Queues `report` for the next `tx_async`.
    pub fn give(&self, report: TxReport) {
        self.given.lock().unwrap().push_back(report);
    }

    /// How many `tx_async` reads stream `k` was given (T3).
    pub fn reads(&self, stream: usize) -> usize {
        self.reads.lock().unwrap().get(stream).copied().unwrap_or(0)
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
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.inner.rx_start_now(stream) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: std::time::Duration, one_packet: bool) -> RxRecv { self.inner.rx_recv(stream, n, timeout, one_packet) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> { self.inner.tx_open(stream, channels) }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: std::time::Duration) -> Result<usize, DeviceError> {
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: std::time::Duration) -> Result<Option<TxReport>, DeviceError> {
        *self.reads.lock().unwrap().get_mut(stream).unwrap_or(&mut 0) += 1;
        let given = self.given.lock().unwrap().pop_front();
        match given {
            Some(report) => Ok(Some(report)),
            None => self.inner.tx_async(stream, timeout),
        }
    }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn raise_thread_priority(&self) -> Result<(), crate::DeviceError> { self.inner.raise_thread_priority() }
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

/// One call `TxSpy` saw: its place in the common order, the host clock when it was made and the calling thread's name.
#[derive(Clone, Debug)]
pub(super) struct TxRec {
    pub what: &'static str,
    pub host: Wall,
    pub thread: String,
    pub n: usize,
    pub eob: bool,
}

/// Where `TxSpy` holds an owner: its `nth` `tx_async` after `after_sends` sends (an idle one: nothing was sent since). The
/// snapshot is taken on the owner's own thread and the owner waits on `release` — an ordinary channel wait, so the manual clock
/// stands still — until the test lets it go (design note 37).
struct IdleTrap {
    after_sends: usize,
    nth: usize,
    snapshot: Sender<Vec<TxRec>>,
    release: Receiver<()>,
}

/// A `FakeDevice` behind a recorder of the transmit calls and of `raise_thread_priority`, whose answer a test sets (UR-23).
pub(super) struct TxSpy {
    inner: Arc<FakeDevice>,
    refusal: Mutex<Option<(DeviceError, Option<String>)>>,
    log: Mutex<Vec<TxRec>>,
    idle: Mutex<usize>,
    trap: Mutex<Option<IdleTrap>>,
}

impl TxSpy {
    pub fn new(inner: Arc<FakeDevice>) -> TxSpy {
        TxSpy { inner, refusal: Mutex::new(None), log: Mutex::new(Vec::new()), idle: Mutex::new(0), trap: Mutex::new(None) }
    }

    /// `raise_thread_priority` answers `Err(refusal)` from now on, to the thread named `only` alone when given.
    pub fn refuse_priority(&self, refusal: DeviceError, only: Option<&str>) {
        *self.refusal.lock().unwrap() = Some((refusal, only.map(str::to_owned)));
    }

    pub fn recs(&self) -> Vec<TxRec> {
        self.log.lock().unwrap().clone()
    }

    /// Holds the owner at its `nth` idle `tx_async` after `after_sends` sends; the snapshot comes on the returned receiver, the
    /// owner goes on when the sender is used or dropped.
    pub fn trap_idle(&self, after_sends: usize, nth: usize) -> (Receiver<Vec<TxRec>>, Sender<()>) {
        let (snapshot, snapshots) = channel();
        let (release, released) = channel();
        *self.trap.lock().unwrap() = Some(IdleTrap { after_sends, nth, snapshot, release: released });
        (snapshots, release)
    }

    fn record(&self, what: &'static str, n: usize, eob: bool) {
        let rec = TxRec { what, host: self.inner.host().now(), thread: std::thread::current().name().unwrap_or("").to_owned(), n, eob };
        self.log.lock().unwrap().push(rec);
    }
}

impl Device for TxSpy {
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
    fn rx_start_now(&self, stream: usize) -> Result<(), DeviceError> { self.inner.rx_start_now(stream) }
    fn rx_packet_samples(&self, stream: usize) -> usize { self.inner.rx_packet_samples(stream) }
    fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError> { self.inner.rx_stop(stream, at) }
    fn rx_recv(&self, stream: usize, n: usize, timeout: Wall, one_packet: bool) -> RxRecv { self.inner.rx_recv(stream, n, timeout, one_packet) }
    fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError> {
        self.record("tx_open", 0, false);
        self.inner.tx_open(stream, channels)
    }
    fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Wall) -> Result<usize, DeviceError> {
        self.record("tx_send", samples.first().map_or(0, |ch| ch.len()), eob);
        self.inner.tx_send(stream, samples, at, sob, eob, timeout)
    }
    fn tx_async(&self, stream: usize, timeout: Wall) -> Result<Option<TxReport>, DeviceError> {
        self.record("tx_async", 0, false);
        let sends = self.log.lock().unwrap().iter().filter(|rec| rec.what == "tx_send").count();
        // No lock is held across the owner's waits: a thread blocked on one would hold the manual clock still.
        let held = {
            let mut trap = self.trap.lock().unwrap();
            let fire = trap.as_ref().is_some_and(|held| {
                if sends < held.after_sends { return false; }
                let mut idle = self.idle.lock().unwrap();
                *idle += 1;
                *idle == held.nth
            });
            if fire { trap.take() } else { None }
        };
        if let Some(held) = held {
            let _ = held.snapshot.send(self.recs());
            let _ = held.release.recv();
        }
        self.inner.tx_async(stream, timeout)
    }
    fn rx_close(&self, stream: usize) { self.inner.rx_close(stream) }
    fn tx_close(&self, stream: usize) { self.inner.tx_close(stream) }
    fn close_streams(&self) { self.inner.close_streams() }
    fn mark_lost(&self) { self.inner.mark_lost() }
    fn host(&self) -> HostClock { self.inner.host() }
    fn raise_thread_priority(&self) -> Result<(), DeviceError> {
        self.record("raise_thread_priority", 0, false);
        let me = std::thread::current().name().unwrap_or("").to_owned();
        match self.refusal.lock().unwrap().clone() {
            Some((refusal, only)) if only.as_ref().is_none_or(|name| *name == me) => Err(refusal),
            _ => Ok(()),
        }
    }
}
