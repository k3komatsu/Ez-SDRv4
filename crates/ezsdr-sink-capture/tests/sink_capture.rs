use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ezsdr_hostmem::{write_cf32, HostPool, HOST_MEMORY};
use ezsdr_kernel::binding::{Binding, Violation};
use ezsdr_kernel::contract::{DataContractId, PortRef};
use ezsdr_kernel::event::{Action, ActionId, EventCollector, EventKind};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, ModuleId, ResourceId, RunId};
use ezsdr_kernel::manifest::ArtifactRef;
use ezsdr_kernel::module_api::{
    ActionReceiver, ActionSubmitter, AttachedPort, Deployment, Endpoint, ExecutionClass,
    ModuleError, ModuleRef, Pacing, Sink, StopMode, UpdateClass, Version, VersionReq,
    VocabularyRequirement, KERNEL_API,
};
use ezsdr_kernel::plan::Fragment;
use ezsdr_kernel::policy::Policy;
use ezsdr_kernel::spec::{Ident, Key, Namespace, OutputReq, SinkFeed, Value};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BlockRef, ChannelMask, DataLink, Direction, DropCarry,
    PublishOutcome, SampleBlock,
};
use ezsdr_kernel::time::{
    AbsoluteDeadline, ClockDomain, ClockRegistry, EpochRef, ManualTimeAuthority, Rational,
    RelativeBudget, TimePoint,
};
use ezsdr_sink::{CAPTURE_ARTIFACT_KIND, CAPTURE_SAMPLES, REQUEST_REJECTED};
use ezsdr_sink_capture::{descriptor, sink_descriptor, CaptureSink};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "ezsdr-capture-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TestLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
    carry: Mutex<DropCarry>,
}

impl TestLink {
    fn new(policy: BackPressure, capacity: usize) -> TestLink {
        TestLink {
            policy,
            capacity: capacity.max(1),
            queue: Mutex::new(VecDeque::new()),
            drops: AtomicU64::new(0),
            carry: Mutex::new(DropCarry::default()),
        }
    }

    fn record_drop(&self, block: &BlockRef) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        self.carry.lock().expect("carry lock").absorb(block);
    }
}

impl DataLink for TestLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut queue = self.queue.lock().expect("queue lock");
        if queue.len() < self.capacity {
            queue.push_back(block);
            return PublishOutcome::Accepted;
        }
        match self.policy {
            BackPressure::Block => PublishOutcome::Full,
            BackPressure::DropOldest => {
                self.record_drop(&queue.pop_front().expect("queue is full"));
                queue.push_back(block);
                PublishOutcome::DroppedOldest
            }
            BackPressure::DropNewest => {
                self.record_drop(&block);
                PublishOutcome::DroppedNewest
            }
        }
    }

    fn receive(&self) -> Option<BlockRef> {
        self.queue.lock().expect("queue lock").pop_front()
    }

    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    fn take_drop_carry(&self) -> DropCarry {
        std::mem::take(&mut *self.carry.lock().expect("carry lock"))
    }

    fn policy(&self) -> BackPressure {
        self.policy
    }
}

#[derive(Default)]
struct ActionQueue(Mutex<VecDeque<Action>>);

impl ActionQueue {
    fn push(&self, action: Action) {
        self.0.lock().expect("action lock").push_back(action);
    }
}

impl ActionReceiver for ActionQueue {
    fn recv(&self) -> Option<Action> {
        self.0.lock().expect("action lock").pop_front()
    }
}

struct NoopSubmitter;

impl ActionSubmitter for NoopSubmitter {
    fn submit(&self, _action: Action) -> Result<ActionId, Vec<Violation>> {
        Err(Vec::new())
    }
}

struct Environment {
    clocks: Arc<ClockRegistry>,
    root: ClockDomainId,
    sample_clock: ClockDomainId,
    changed_clock: ClockDomainId,
    unrelated_root: ClockDomainId,
    time: Arc<ManualTimeAuthority>,
    events: Arc<EventCollector>,
    actions: Arc<ActionQueue>,
    submitter: Arc<NoopSubmitter>,
}

impl Environment {
    fn new() -> Environment {
        let clocks = Arc::new(ClockRegistry::new());
        let root = clocks.allocate_id();
        clocks
            .register(ClockDomain::root(
                root,
                Rational::new(1_000_000_000, 1).expect("root rate"),
                EpochRef::Arbitrary {
                    set_by: "sink_capture_test".to_owned(),
                },
            ))
            .expect("root registration");
        let time = Arc::new(
            ManualTimeAuthority::new(clocks.clone(), root, &[], Pacing::FreeRunning)
                .expect("manual time"),
        );
        let stream = ResourceId::parse("dev/rx").expect("stream id");
        let sample_handle = clocks
            .declare_sample_clock(
                stream.clone(),
                root,
                Rational::new(1_000, 1).expect("sample rate ratio"),
            )
            .expect("sample clock declaration");
        let sample_clock = clocks
            .register_sample_clock(&sample_handle, 0)
            .expect("sample clock registration");
        let changed_handle = clocks
            .declare_sample_clock(
                stream,
                root,
                Rational::new(2_000, 1).expect("changed sample rate ratio"),
            )
            .expect("changed sample clock declaration");
        clocks
            .end(sample_clock, TimePoint::new(root, 58_000))
            .expect("old sample clock end");
        let changed_clock = clocks
            .register_sample_clock(&changed_handle, 58_000)
            .expect("changed sample clock registration");
        let unrelated_root = clocks.allocate_id();
        clocks
            .register(ClockDomain::root(
                unrelated_root,
                Rational::new(1_000_000_000, 1).expect("unrelated rate"),
                EpochRef::Arbitrary {
                    set_by: "unrelated_test_root".to_owned(),
                },
            ))
            .expect("unrelated root registration");
        let source = ResourceId::parse("sink/rec").expect("Sink event source");
        let kind = EventKind::parse(REQUEST_REJECTED).expect("Sink event kind");
        let events = Arc::new(EventCollector::new(
            &[(source, kind)],
            &[],
            32,
            &Policy::default(),
        ));
        Environment {
            clocks,
            root,
            sample_clock,
            changed_clock,
            unrelated_root,
            time,
            events,
            actions: Arc::new(ActionQueue::default()),
            submitter: Arc::new(NoopSubmitter),
        }
    }

    fn context(&self, links: Vec<AttachedPort>) -> ezsdr_kernel::module_api::PrepareContext {
        ezsdr_kernel::module_api::PrepareContext {
            run: RunId::from_string("run:test".to_owned()),
            class: ExecutionClass::Simulation,
            time: self.time.clone(),
            clocks: self.clocks.clone(),
            events: self.events.clone(),
            actions: self.actions.clone(),
            actions_out: self.submitter.clone(),
            environment: Arc::new(BTreeMap::new()),
            inputs: Arc::new(BTreeMap::<ezsdr_kernel::hash::ContentHash, Arc<[u8]>>::new()),
            links,
            components: BTreeMap::new(),
            host_budget: RelativeBudget::new(ezsdr_kernel::time::Duration::new(
                ClockDomainId::HOST_MONOTONIC,
                1_000_000,
            ))
            .expect("host budget"),
        }
    }
}

struct Rig {
    _temp: TempDir,
    env: Environment,
    link: Arc<TestLink>,
    sink: CaptureSink,
    pool: HostPool,
}

impl Rig {
    fn new(name: &str, params: BTreeMap<Key, Value>) -> Rig {
        Rig::with_link(name, params, BackPressure::DropOldest, 64)
    }

    fn with_link(
        name: &str,
        params: BTreeMap<Key, Value>,
        policy: BackPressure,
        capacity: usize,
    ) -> Rig {
        let temp = TempDir::new(name);
        let env = Environment::new();
        let link = Arc::new(TestLink::new(policy, capacity));
        let mut sink = CaptureSink::from_binding(&binding(temp.path(), false, None))
            .expect("valid binding");
        let attached = attached("rec", "in", Endpoint::StreamIn(link.clone()));
        sink.prepare(&fragment(params), env.context(vec![attached]))
            .expect("valid prepare");
        Rig {
            _temp: temp,
            env,
            link,
            sink,
            pool: HostPool::new(80_000),
        }
    }

    fn push_ramp(&mut self, first: i64, len: u32) -> PublishOutcome {
        let block = ramp_block(
            &mut self.pool,
            self.env.sample_clock,
            first,
            first as f32,
            len,
            BlockFlags::NONE,
            None,
        );
        self.link.publish(block)
    }

    fn step(&mut self) -> Result<bool, ModuleError> {
        self.sink
            .step(TimePoint::new(self.env.root, 0))
            .map(|outcome| outcome.progressed)
    }

    fn stop(&mut self, mode: StopMode) -> Vec<ArtifactRef> {
        self.sink.stop(mode).expect("Sink stop")
    }
}

fn ident(value: &str) -> Ident {
    Ident::parse(value).expect("valid identifier")
}

fn key(value: &str) -> Key {
    Key::parse(value).expect("valid key")
}

fn binding(dir: &Path, extra_selector: bool, profile: Option<ezsdr_kernel::module_api::ProfileRef>) -> Binding {
    let descriptor = descriptor();
    let mut selector = BTreeMap::from([(
        ident("dir"),
        Value::Str(dir.to_string_lossy().into_owned()),
    )]);
    if extra_selector {
        selector.insert(ident("unused"), Value::Int(1));
    }
    Binding {
        module: ModuleRef {
            id: descriptor.id,
            version: descriptor.version,
        },
        selector,
        profile,
        feed: None,
    }
}

fn fragment(params: BTreeMap<Key, Value>) -> Fragment {
    let descriptor = descriptor();
    let request = OutputReq {
        id: ident("rec"),
        kind: Namespace::parse(CAPTURE_ARTIFACT_KIND).expect("artifact kind"),
        feed: SinkFeed {
            port: PortRef {
                component: ident("source"),
                port: ident("out"),
            },
            policy: BackPressure::DropOldest,
            capacity: 8,
        },
        params,
    };
    Fragment {
        id: ident("rec"),
        instance: ModuleRef {
            id: descriptor.id,
            version: descriptor.version,
        },
        role: ezsdr_kernel::module_api::Role::Sink,
        content: serde_json::to_value(request).expect("OutputReq JSON"),
        after: Vec::new(),
    }
}

fn attached(component: &str, port: &str, endpoint: Endpoint) -> AttachedPort {
    AttachedPort {
        component: ident(component),
        port: ident(port),
        endpoint,
    }
}

fn sample_count(count: i64) -> BTreeMap<Key, Value> {
    BTreeMap::from([(key(CAPTURE_SAMPLES), Value::Int(count))])
}

fn request(n: Value, at: Option<AbsoluteDeadline>) -> Action {
    Action::UpdateParameter {
        target: ResourceId::parse("sink/rec").expect("target"),
        key: key(CAPTURE_SAMPLES),
        value: n,
        class: UpdateClass::BlockBoundary,
        at,
    }
}

fn ramp_block(
    pool: &mut HostPool,
    domain: ClockDomainId,
    first: i64,
    value_first: f32,
    len: u32,
    flags: BlockFlags,
    lost: Option<u64>,
) -> BlockRef {
    let len_usize = len as usize;
    let bytes = pool.fill(len_usize * ezsdr_hostmem::CF32_BYTES, |buf| {
        for index in 0..len_usize {
            let value = value_first + index as f32;
            write_cf32(buf, len_usize, 0, index, value, -value);
        }
    });
    let header = BlockHeader {
        first_sample_time: TimePoint::new(domain, first),
        len,
        channels: 1,
        direction: Direction::Rx,
        valid: ChannelMask::full(1),
        flags,
        lost,
        contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract"),
    };
    BlockRef::new(
        SampleBlock::new_host(header, HOST_MEMORY, bytes, ezsdr_hostmem::CF32_BYTES as u32)
            .expect("well-formed CF32 block"),
    )
}

fn artifact_samples(artifact: &ArtifactRef) -> Vec<f32> {
    let path = artifact.uri.strip_prefix("file://").expect("file URI");
    let bytes = fs::read(path).expect("capture file");
    assert_eq!(bytes.len() as u64, artifact.size_bytes);
    assert_eq!(ContentHash::of_bytes(&bytes), artifact.hash);
    bytes
        .chunks_exact(8)
        .map(|sample| f32::from_le_bytes(sample[..4].try_into().expect("real sample")))
        .collect()
}

fn assert_ramp(artifact: &ArtifactRef, first: usize, count: usize) {
    let values = artifact_samples(artifact);
    assert_eq!(values.len(), count);
    for (offset, value) in values.into_iter().enumerate() {
        assert_eq!(value, (first + offset) as f32);
    }
}

fn tx_burst() -> Action {
    Action::TxBurst {
        target: ResourceId::parse("dev/tx").expect("TX target"),
        waveform: ArtifactRef {
            id: ident("wave"),
            kind: Namespace::parse("test.waveform").expect("waveform kind"),
            uri: "memory://waveform".to_owned(),
            hash: ContentHash::of_bytes(&[]),
            size_bytes: 0,
            partial: false,
            marks: Vec::new(),
            continuity: Vec::new(),
        },
        repeat: false,
        at: AbsoluteDeadline::new(TimePoint::new(ClockDomainId::HOST_MONOTONIC, 1)),
        requested_at: None,
        late_policy: ezsdr_kernel::stream::LatePolicy::DropAndFlag,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn hd_07_descriptor() {
    let module = descriptor();
    assert_eq!(module.id, ModuleId::parse("ezsdr.sink.capture").expect("module id"));
    assert_eq!(module.version, Version::new(1, 0, 0));
    assert_eq!(module.kernel_api, KERNEL_API);
    assert_eq!(module.roles, vec![ezsdr_kernel::module_api::Role::Sink]);
    assert_eq!(module.deployment, Deployment::InProcess {});
    assert_eq!(
        module.impl_hash,
        Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.0.0"))
    );
    assert_eq!(
        module.vocabularies,
        vec![VocabularyRequirement {
            id: Namespace::parse("sink").expect("namespace"),
            req: VersionReq(Version::new(1, 0, 0)),
        }]
    );
    let sink = sink_descriptor();
    assert_eq!(sink.module.id, module.id);
    assert_eq!(sink.kind, Namespace::parse("ezsdr.sink.capture").expect("sink kind"));
    assert_eq!(sink.memory_domains, vec![HOST_MEMORY]);
    assert_eq!(
        sink.contracts.iter().map(DataContractId::as_str).collect::<Vec<_>>(),
        vec!["ezsdr.stream.cf32", "ezsdr.stream.sc16"]
    );
    assert_eq!(
        sink.artifact_kinds,
        vec![Namespace::parse(CAPTURE_ARTIFACT_KIND).expect("artifact kind")]
    );
}

#[test]
fn hd_09_prepare_cases() {
    let temp = TempDir::new("prepare-cases");
    let env = Environment::new();
    let link_a = Arc::new(TestLink::new(BackPressure::DropOldest, 4));
    let link_b = Arc::new(TestLink::new(BackPressure::DropOldest, 4));
    let one = attached("rec", "in", Endpoint::StreamIn(link_a.clone()));
    let two = attached("rec", "in", Endpoint::StreamIn(link_b.clone()));

    let mut no_link = CaptureSink::from_binding(&binding(temp.path(), false, None))
        .expect("valid binding");
    assert!(no_link.prepare(&fragment(BTreeMap::new()), env.context(Vec::new())).is_err());

    let mut two_links = CaptureSink::from_binding(&binding(temp.path(), false, None))
        .expect("valid binding");
    assert!(two_links
        .prepare(&fragment(BTreeMap::new()), env.context(vec![one.clone(), two]))
        .is_err());

    let mut wrong_port = CaptureSink::from_binding(&binding(temp.path(), false, None))
        .expect("valid binding");
    assert!(wrong_port
        .prepare(
            &fragment(BTreeMap::new()),
            env.context(vec![attached("rec", "out", Endpoint::StreamIn(link_a.clone()))]),
        )
        .is_err());

    assert!(CaptureSink::from_binding(&binding(temp.path(), true, None)).is_err());

    let mut zero = CaptureSink::from_binding(&binding(temp.path(), false, None))
        .expect("valid binding");
    assert!(zero
        .prepare(&fragment(sample_count(0)), env.context(vec![one.clone()]))
        .is_err());

    let mut unknown = CaptureSink::from_binding(&binding(temp.path(), false, None))
        .expect("valid binding");
    assert!(unknown
        .prepare(
            &fragment(BTreeMap::from([(key("other.key"), Value::Int(1))])),
            env.context(vec![one.clone()]),
        )
        .is_err());

    let file_dir = temp.path().join("regular-file");
    fs::create_dir_all(temp.path()).expect("create temporary parent");
    fs::write(&file_dir, b"not a directory").expect("create regular file");
    let mut blocked_dir = CaptureSink::from_binding(&binding(&file_dir, false, None))
        .expect("binding path need not exist yet");
    assert!(blocked_dir
        .prepare(&fragment(BTreeMap::new()), env.context(vec![one]))
        .is_err());
}

#[test]
fn hd_10_capture_of_n_samples_across_jittered_blocks() {
    let mut rig = Rig::new("jittered", BTreeMap::new());
    let at = AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 123));
    rig.env.actions.push(request(Value::Int(5_000), Some(at)));
    let lengths = [1_u32, 7, 1_999, 3, 1_024, 67, 3_000, 3_899];
    let mut first = 0_i64;
    for len in lengths {
        assert_eq!(rig.push_ramp(first, len), PublishOutcome::Accepted);
        assert!(rig.step().expect("step succeeds"));
        first += i64::from(len);
    }
    assert_eq!(first, 10_000);
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].id, ident("rec_0"));
    assert!(!artifacts[0].partial);
    assert_ramp(&artifacts[0], 123, 5_000);
    assert_eq!(artifacts[0].continuity.len(), 1);
    assert_eq!(artifacts[0].continuity[0].valid[0].len(), 1);
    assert_eq!(artifacts[0].continuity[0].valid[0][0].start.ticks, 123);
    assert_eq!(artifacts[0].continuity[0].valid[0][0].len, 5_000);
}

#[test]
fn hd_10_a_capture_spans_a_gap_and_a_clock_change() {
    let mut rig = Rig::new("gap-clock", sample_count(12));
    let first = ramp_block(
        &mut rig.pool,
        rig.env.sample_clock,
        0,
        0.0,
        4,
        BlockFlags::NONE,
        None,
    );
    rig.link.publish(first);
    rig.step().expect("first block");
    let overflow = ramp_block(
        &mut rig.pool,
        rig.env.sample_clock,
        54,
        54.0,
        4,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        Some(50),
    );
    rig.link.publish(overflow);
    rig.step().expect("overflow block");
    let changed = ramp_block(
        &mut rig.pool,
        rig.env.changed_clock,
        0,
        58.0,
        4,
        BlockFlags::NONE,
        None,
    );
    rig.link.publish(changed);
    rig.step().expect("new SampleClock block");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    let maps = &artifacts[0].continuity;
    assert_eq!(maps.len(), 2);
    assert_eq!(maps[0].domain, rig.env.sample_clock);
    assert_eq!(maps[1].domain, rig.env.changed_clock);
    assert_eq!(maps[0].gaps.len(), 1);
    assert_eq!(maps[0].gaps[0].lost, Some(50));
    assert_eq!(
        maps[0].gaps[0].cause,
        ezsdr_kernel::stream::GapCause::OverflowRestart {}
    );
}

#[test]
fn hd_10_session_requests_are_sequential() {
    let mut rig = Rig::new("sequential", BTreeMap::new());
    rig.env.actions.push(request(
        Value::Int(100),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 100))),
    ));
    rig.env.actions.push(request(
        Value::Int(100),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 150))),
    ));
    rig.push_ramp(0, 500);
    rig.step().expect("step succeeds");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["rec_0", "rec_1"]);
    assert_ramp(&artifacts[0], 100, 100);
    assert_ramp(&artifacts[1], 200, 100);
}

#[test]
fn hd_10_the_own_capture_comes_first() {
    let mut rig = Rig::new("own-first", sample_count(50));
    rig.env.actions.push(request(
        Value::Int(20),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 0))),
    ));
    rig.push_ramp(0, 100);
    rig.step().expect("step succeeds");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["rec", "rec_0"]);
    assert_ramp(&artifacts[0], 0, 50);
    assert_ramp(&artifacts[1], 50, 20);
}

#[test]
fn hd_10_no_capture_no_artifact() {
    let mut rig = Rig::new("no-capture", BTreeMap::new());
    rig.push_ramp(0, 1_000);
    rig.step().expect("step succeeds");
    assert!(rig.stop(StopMode::Orderly).is_empty());
}

#[test]
fn hd_10_the_carry_attributes_a_dropped_overflow() {
    let mut rig = Rig::with_link(
        "drop-carry",
        sample_count(100),
        BackPressure::DropOldest,
        1,
    );
    rig.push_ramp(0, 10);
    rig.step().expect("start capture before link drop");
    let overflow = ramp_block(
        &mut rig.pool,
        rig.env.sample_clock,
        10,
        10.0,
        10,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        Some(50),
    );
    assert_eq!(rig.link.publish(overflow), PublishOutcome::Accepted);
    let after_drop = ramp_block(
        &mut rig.pool,
        rig.env.sample_clock,
        70,
        70.0,
        10,
        BlockFlags::NONE,
        None,
    );
    assert_eq!(rig.link.publish(after_drop), PublishOutcome::DroppedOldest);
    rig.step().expect("carry reaches continuity builder");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    let gaps = &artifacts[0].continuity[0].gaps;
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
    assert_eq!(gaps[0].lost, Some(50));
    assert_eq!(gaps[0].link_dropped, 1);
}

#[test]
fn hd_11_an_unexpected_action_is_rejected() {
    let mut rig = Rig::new("unexpected-action", BTreeMap::new());
    rig.env.actions.push(tx_burst());
    assert!(rig.step().expect("unexpected Action is an event"));
    let events = rig.env.events.drain();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, ResourceId::parse("sink/rec").expect("event source"));
    assert_eq!(events[0].kind.as_str(), REQUEST_REJECTED);
    assert_eq!(events[0].payload["action"], "tx_burst");
    assert!(events[0].payload["reason"].as_str().unwrap().starts_with("HD-14"));
}

#[test]
fn hd_14_a_bad_capture_value_is_an_event_not_a_failure() {
    let mut rig = Rig::new("bad-capture-values", BTreeMap::new());
    rig.env.actions.push(request(Value::Int(0), None));
    rig.env.actions.push(request(Value::Num(5.0), None));
    assert!(rig.step().expect("bad values are rejected events"));
    rig.env.actions.push(request(
        Value::Int(5),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.unrelated_root, 0))),
    ));
    rig.env.actions.push(request(Value::Int(10), None));
    rig.push_ramp(0, 10);
    assert!(rig.step().expect("unrelated time is rejected and valid request runs"));
    let events = rig.env.events.drain();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| event.kind.as_str() == REQUEST_REJECTED));
    assert!(events.iter().all(|event| {
        event.payload["action"] == "update_parameter"
            && event.payload["reason"].as_str().unwrap().starts_with("HD-14")
    }));
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].id, ident("rec_0"));
    assert_ramp(&artifacts[0], 0, 10);
}

#[test]
fn hd_13_partial_on_abort_and_on_an_unfinished_capture() {
    let mut aborted = Rig::new("partial-abort", sample_count(5_000));
    aborted.push_ramp(0, 4_000);
    aborted.env.actions.push(request(Value::Int(100), None));
    aborted.step().expect("partial samples");
    let artifacts = aborted.stop(StopMode::Abort);
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].partial);
    assert_eq!(artifacts[0].size_bytes, 4_000 * 8);

    let mut orderly = Rig::new("partial-orderly", sample_count(5_000));
    orderly.push_ramp(0, 4_000);
    orderly.step().expect("partial samples");
    let artifacts = orderly.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].partial);
    assert_eq!(artifacts[0].size_bytes, 4_000 * 8);
}

#[test]
fn hd_11_stop_for_another_target_is_rejected() {
    let mut rig = Rig::new("stop-other-target", sample_count(8));
    rig.push_ramp(0, 4);
    rig.step().expect("start partial capture");
    rig.env.actions.push(Action::Stop { target: Some(ResourceId::parse("radio").expect("radio target")) });
    assert!(rig.step().expect("wrong-target Stop is rejected as an event"));
    let events = rig.env.events.drain();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind.as_str(), REQUEST_REJECTED);
    assert_eq!(events[0].payload["action"], "stop");
    assert!(events[0].payload["reason"].as_str().unwrap().starts_with("HD-14"));

    rig.push_ramp(4, 4);
    rig.step().expect("capture continues after unrelated Stop");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    assert!(!artifacts[0].partial);
    assert_eq!(artifacts[0].size_bytes, 8 * 8);
    assert_ramp(&artifacts[0], 0, 8);
}

#[test]
fn hd_11_stop_for_own_target_finishes_the_capture() {
    let mut rig = Rig::new("stop-own-target", sample_count(8));
    rig.push_ramp(0, 4);
    rig.step().expect("start partial capture");
    rig.env.actions.push(Action::Stop {
        target: Some(ResourceId::parse("sink/rec").expect("Sink target")),
    });
    assert!(rig.step().expect("own Stop finishes the capture"));
    assert!(rig.env.events.drain().is_empty());

    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].partial);
    assert_eq!(artifacts[0].size_bytes, 4 * 8);
    assert_ramp(&artifacts[0], 0, 4);
}

#[test]
fn hd_11_a_stop_discards_unstarted_requests_and_later_ones_are_served() {
    let mut rig = Rig::new("stop-discards-queue", sample_count(8));
    rig.push_ramp(0, 4);
    rig.step().expect("start the own capture");
    rig.env.actions.push(request(Value::Int(3), None));
    rig.env.actions.push(Action::Stop {
        target: Some(ResourceId::parse("sink/rec").expect("Sink target")),
    });
    rig.step().expect("the Stop finishes the own capture");
    rig.env.actions.push(request(Value::Int(2), None));
    rig.push_ramp(4, 6);
    rig.step().expect("a request after the Stop is served");

    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["rec", "rec_0"]);
    assert!(artifacts[0].partial);
    assert_ramp(&artifacts[0], 0, 4);
    assert!(!artifacts[1].partial);
    assert_ramp(&artifacts[1], 4, 2);
}

#[test]
fn hd_10_unknown_contract_is_rejected() {
    let mut rig = Rig::new("unknown-contract", sample_count(4));
    let header = BlockHeader {
        first_sample_time: TimePoint::new(rig.env.sample_clock, 0),
        len: 1,
        channels: 1,
        direction: Direction::Rx,
        valid: ChannelMask::full(1),
        flags: BlockFlags::NONE,
        lost: None,
        contract: DataContractId::parse("test.unknown").expect("test contract"),
    };
    let bytes: Arc<[u8]> = vec![0; 8].into();
    rig.link.publish(BlockRef::new(
        SampleBlock::new_host(header, HOST_MEMORY, bytes, 8).expect("host block"),
    ));
    let error = rig.step().expect_err("unknown contract is a Module defect");
    assert!(error.message.starts_with("HD-10: contract test.unknown"));
}
