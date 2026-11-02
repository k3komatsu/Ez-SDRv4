//! Today's owner for E2b: the Provider (uhd-control, uhd-rx, uhd-clock) over the fake behind the timing decorator, driven without the
//! Kernel as `tests/fake.rs::Direct` does: an `UpdateParameter` Action pushed into the Provider's own queue.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use ezsdr_kernel::binding::{AdmissionCheckRegistry, Binding, Violation};
use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventKind, EventSource, Target};
use ezsdr_kernel::id::{ClockDomainId, ResourceId, RunId};
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Authority, Dispatched, Endpoint, ExecutionClass, Link, ModuleRegistry, PrepareContext, Provider, Requested, Role, StopMode, UpdateClass,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::{EventKindRegistry, Policy};
use ezsdr_kernel::spec::{Constraint, Ident, Key, Namespace, Scalar, Value};
use ezsdr_kernel::stream::{BackPressure, DataLinkDecl};
use ezsdr_kernel::time::{AbsoluteDeadline, ClockRegistry, Duration, RelativeBudget, TimePoint};
use serde_json::{Value as Json, json};

use super::runs::{Cond, Opts, Rig};
use super::{FREQS, Offset, Tap, Timing, Upd};
use crate::device::{Device, FakeConfig, FakeDevice, FakeFault};
use crate::{DeviceAuthority, UhdRadio};

const MCR: u64 = 200_000_000;
const ARGS: &str = "type=fake-rig";

#[derive(Default)]
struct Pushed(Mutex<VecDeque<Action>>, AtomicU64);

impl ActionReceiver for Pushed {
    fn recv(&self) -> Option<Dispatched> {
        let id = || ActionId(self.1.fetch_add(1, SeqCst) + 1);
        self.0.lock().unwrap().pop_front().map(|action| {
            let node = action.target().and_then(|target| match target {
                Target::Resource { resource, path } if path.is_empty() => ResourceId::parse(resource.as_str()).ok(),
                _ => None,
            });
            Dispatched::new(id(), action, node)
        })
    }
}

struct NoSubmit;
impl ActionSubmitter for NoSubmit {
    fn submit(&self, _: Action) -> Result<ActionId, Vec<Violation>> {
        Ok(ActionId(1))
    }
}

pub(super) struct Today {
    radio: Option<UhdRadio>,
    queue: Arc<Pushed>,
    events: Arc<EventCollector>,
    root: ClockDomainId,
    off: Offset,
    fake: Arc<FakeDevice>,
    timing: Arc<Timing>,
    link: Arc<Tap>,
    rate_applied: Option<String>,
    _authority: DeviceAuthority,
}

fn request(pairs: &[(&str, Scalar)]) -> Requested {
    Requested { resource: ResourceId::parse("usrp").unwrap(), constraints: pairs.iter().map(|(k, v)| (Key::parse(k).unwrap(), Constraint::Eq { value: v.clone() })).collect() }
}

pub(super) fn start(cond: &Cond, opts: &Opts) -> Box<dyn Rig> {
    let faults = opts.fault_ms.map(|ms| vec![FakeFault::Lost(std::time::Duration::from_millis(ms))]).unwrap_or_default();
    let fake = Arc::new(FakeDevice::new(FakeConfig { faults, ..FakeConfig::default() }));
    let timing = Timing::new(fake.clone(), cond.count + 1024, opts.counters);
    timing.counts.tps.store((MCR as f64 / cond.rate_hz).round() as i64, SeqCst);
    let tested: Arc<dyn Device> = timing.clone();
    let clocks = Arc::new(ClockRegistry::new());
    let authority = DeviceAuthority::new(tested.clone(), clocks.clone(), "internal", "internal", ARGS).unwrap();
    let (time, root) = (authority.time(), authority.root());
    let mut registry = ModuleRegistry::new();
    let (mut checks, mut kinds) = (AdmissionCheckRegistry::new(), EventKindRegistry::with_kernel_kinds());
    ezsdr_radio::register(&mut registry, &mut checks, &mut kinds).unwrap();
    let pairs: Vec<_> = ["usrp", "usrp/rx/s0", "usrp/tx/s0"].iter().flat_map(|s| kinds.kinds().into_iter().map(move |k| (EventSource::Node { node: ResourceId::parse(s).unwrap() }, k))).collect();
    let events = Arc::new(EventCollector::new(&pairs, &kinds.kinds(), 4096, &Policy::default()));
    let queue = Arc::new(Pushed::default());
    let decl = DataLinkDecl {
        id: ezsdr_kernel::id::DataLinkId::local(0),
        from: ezsdr_kernel::contract::PortRef { component: Ident::parse("radio").unwrap(), port: Ident::parse("rx0").unwrap() },
        to: ezsdr_kernel::contract::PortRef { component: Ident::parse("rec").unwrap(), port: Ident::parse("in").unwrap() },
        contract: ezsdr_kernel::contract::DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy: BackPressure::DropOldest,
        capacity: super::LINK_CAPACITY,
    };
    let link = Tap::new(ezsdr_link_host::HostLinkModule::new().create(&decl).unwrap());
    let port = AttachedPort::new(Ident::parse("radio").unwrap(), Ident::parse("rx0").unwrap(), Endpoint::StreamOut(link.clone()));
    let ctx = {
        let mut ctx = PrepareContext::testing(EventSource::Node { node: ResourceId::parse("usrp").unwrap() }, time.clone(), clocks.clone(), events.clone(), queue.clone(), Arc::new(NoSubmit));
        ctx.run = RunId::from_string("e2b".to_owned());
        ctx.class = ExecutionClass::HardwareInLoop;
        ctx.environment = Arc::new(BTreeMap::new());
        ctx.inputs = Arc::new(BTreeMap::new());
        ctx.links = vec![port];
        ctx.host_budget = RelativeBudget::new(Duration::new(ClockDomainId::HOST_MONOTONIC, 5_000_000_000)).unwrap();
        ctx
    };
    let constraints = [("radio.rx.s0.sample_rate_hz", Scalar::try_from(cond.rate_hz).unwrap()), ("radio.rx.c0.frequency_hz", Scalar::try_from(FREQS[0]).unwrap())];
    let fragment =
        Fragment { id: Ident::parse("radio").unwrap(), instance: crate::module_ref(), role: Role::Provider, content: json!({ "selector": {}, "requested": request(&constraints) }), after: Vec::new() };
    let selector = json!({ "args": ARGS, "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"], "block_len": cond.block_len, "link_gbps": [10] });
    let binding: Binding = serde_json::from_value(json!({
        "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 4, "patch": 0 } },
        "selector": selector,
        "profile": { "name": "x3xx", "version": { "major": 0, "minor": 1, "patch": 0 } },
    }))
    .unwrap();
    let mut radio = UhdRadio::from_binding(&binding, tested).unwrap();
    radio.prepare(&fragment, ctx).unwrap();
    radio.arm().unwrap();
    // `Direct::start`: T0 two seconds ahead, then a millisecond past it.
    let ms = |n: i64| n * MCR as i64 / 1_000;
    let t0 = time.now(root).unwrap().ticks_in(root).unwrap() + ms(2_000) + ms(1);
    radio.start(Some(TimePoint::new(root, t0))).unwrap();
    time.wait_until(TimePoint::new(root, t0 + ms(1))).unwrap();
    let off = Offset::establish(&fake, MCR);
    let rate_applied = fake.calls().into_iter().find(|c| c.starts_with("apply rx 0 rate=")).and_then(|c| c.split(' ').nth(3).map(str::to_owned));
    Box::new(Today { radio: Some(radio), queue, events, root, off, fake, timing, link, rate_applied, _authority: authority })
}

impl Today {
    fn kinds(&self) -> Json {
        let drained = self.events.drain(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 0));
        let count = |name: &str| drained.iter().filter(|e| e.kind == EventKind::parse(name).unwrap()).count();
        json!({ "LATE_COMMAND": count("radio.LATE_COMMAND"), "COMMAND_QUEUE_FULL": count("radio.COMMAND_QUEUE_FULL"), "COMMAND_REJECTED": count("radio.COMMAND_REJECTED"), "RX_OVERFLOW": count("radio.RX_OVERFLOW") })
    }
}

impl Rig for Today {
    fn name(&self) -> &'static str {
        "today"
    }
    fn fake(&self) -> &Arc<FakeDevice> {
        &self.fake
    }
    fn timing(&self) -> &Arc<Timing> {
        &self.timing
    }
    fn link(&self) -> &Arc<Tap> {
        &self.link
    }
    fn offset(&self) -> Offset {
        self.off
    }
    fn ingress(&self, u: &Upd) {
        self.queue.0.lock().unwrap().push_back(Action::UpdateParameter {
            target: Target::Resource { resource: Ident::parse("usrp").unwrap(), path: String::new() },
            key: Key::parse("radio.rx.c0.frequency_hz").unwrap(),
            value: Value::num(u.freq).unwrap(),
            class: UpdateClass::HardwareTimed,
            at: Some(AbsoluteDeadline::new(TimePoint::new(self.root, u.e))),
        });
    }
    fn extra(&self) -> Json {
        // CALC from the code (facts §1-2), not measured: per block `uhd-rx` takes the chain lock about 6 times and the `streams` lock about 3 times; each
        // update takes the chain lock in `give`/`hand` (held across the device call) and the `streams` lock several times. No counter exists.
        json!({ "rate_applied": self.rate_applied, "chain_lock_per_block": "CALC ~6 (+1 per update in give/hand)", "events": self.kinds() })
    }
    fn finish(mut self: Box<Self>) -> Json {
        let mut radio = self.radio.take().expect("a radio");
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let stop = radio.stop(StopMode::Orderly).map(|_| ()).map_err(|e| e.message);
            radio.cleanup();
            let instance = radio.instance().clone();
            let _ = tx.send((stop, instance));
        });
        match rx.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok((stop, instance)) => {
                let _ = handle.join();
                let stats = instance.sections.get(&Namespace::parse("ezsdr.radio.uhd.stats").unwrap()).cloned().unwrap_or(Json::Null);
                let stat = |name: &str| stats.get(name).and_then(Json::as_u64).unwrap_or(0);
                // The provider's counters are blocks (`rx_before_origin`, `rx_off_lattice`, `rx_overlapping` each count a discarded block), and `rx_samples` the
                // samples it published; the discarded samples are not counted.
                let endpoint = json!({ "published_blocks": stat("rx_blocks"), "published_samples": stat("rx_samples"),
                    "protocol_discarded_blocks": { "before_origin": stat("rx_before_origin"), "off_lattice": stat("rx_off_lattice"), "overlapping": stat("rx_overlapping") },
                    "protocol_discarded_samples": null });
                json!({ "endpoint": endpoint, "joined": true, "stop": stop.map_or_else(|e| format!("error: {e}"), |_| "ok".into()), "events_after": self.kinds(), "stats": stats })
            }
            Err(_) => json!({ "joined": false, "stop": "stop/cleanup did not return within 60 s" }),
        }
    }
}
