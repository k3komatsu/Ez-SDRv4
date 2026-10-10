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
    ActionReceiver, ActionSubmitter, AttachedPort, Endpoint, ExecutionClass,
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
use ezsdr_sink::{CAPTURE_ARTIFACT_KIND, CAPTURE_SAMPLES, CAPTURE_WRITTEN, REQUEST_REJECTED};
use ezsdr_sink_capture::{descriptor, sigmf_meta, sink_descriptor, CaptureSink};
use serde_json::json;

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

/// SC-20b's link, as HostLink implements it (this crate may not depend on that Module).
struct TestLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<(VecDeque<(BlockRef, DropCarry)>, DropCarry)>,
    drops: AtomicU64,
}

impl TestLink {
    fn new(policy: BackPressure, capacity: usize) -> TestLink {
        TestLink {
            policy,
            capacity: capacity.max(1),
            queue: Mutex::new((VecDeque::new(), DropCarry::default())),
            drops: AtomicU64::new(0),
        }
    }
}

impl DataLink for TestLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut guard = self.queue.lock().expect("queue lock");
        let (queue, tail) = &mut *guard;
        let outcome = match (queue.len() < self.capacity, self.policy) {
            (true, _) => PublishOutcome::Accepted,
            (false, BackPressure::Block) => return PublishOutcome::Full,
            (false, BackPressure::DropOldest) => {
                let (evicted, mut carry) = queue.pop_front().expect("queue is full");
                carry.absorb(&evicted);
                queue.front_mut().map_or(&mut *tail, |(_, next)| next).merge(carry);
                PublishOutcome::DroppedOldest
            }
            (false, BackPressure::DropNewest) => {
                tail.absorb(&block);
                PublishOutcome::DroppedNewest
            }
        };
        if outcome != PublishOutcome::Accepted {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
        if outcome != PublishOutcome::DroppedNewest {
            let carry = std::mem::take(tail);
            queue.push_back((block, carry));
        }
        outcome
    }

    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.queue.lock().expect("queue lock").0.pop_front()
    }

    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    fn take_drop_carry(&self) -> DropCarry {
        let mut guard = self.queue.lock().expect("queue lock");
        let (queue, tail) = &mut *guard;
        let mut carry = std::mem::take(tail);
        for (_, queued) in queue.iter_mut() {
            carry.merge(std::mem::take(queued));
        }
        carry
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
        let root = clocks.allocate_id().unwrap();
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
        let unrelated_root = clocks.allocate_id().unwrap();
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
        Value::from(dir.to_string_lossy().into_owned()),
    )]);
    if extra_selector {
        selector.insert(ident("unused"), Value::from(1));
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
    BTreeMap::from([(key(CAPTURE_SAMPLES), Value::from(count))])
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
    assert_eq!(module.version, Version::new(1, 2, 0));
    assert_eq!(module.kernel_api, KERNEL_API);
    assert_eq!(module.roles, vec![ezsdr_kernel::module_api::Role::Sink]);
    assert_eq!(
        module.impl_hash,
        Some(ContentHash::of_bytes(b"ezsdr.sink.capture 1.2.0"))
    );
    assert_eq!(
        module.vocabularies,
        vec![VocabularyRequirement {
            id: Namespace::parse("sink").expect("namespace"),
            req: VersionReq(Version::new(1, 1, 0)),
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
            &fragment(BTreeMap::from([(key("other.key"), Value::from(1))])),
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
    rig.env.actions.push(request(Value::from(5_000), Some(at)));
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
    assert_eq!(artifacts[0].continuity[0].valid[0][0].start.ticks_in(rig.env.sample_clock).unwrap(), 123);
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
        Value::from(100),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 100))),
    ));
    rig.env.actions.push(request(
        Value::from(100),
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
        Value::from(20),
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
fn hd_10_a_capture_keeps_of_a_leading_gap_only_what_follows_its_start() {
    // SC-13: a first block's GAP_BEFORE counts back from its first sample; a capture
    // keeps of it only the samples at or after its own start instant.
    let gap = |rig: &mut Rig| {
        let flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
        ramp_block(&mut rig.pool, rig.env.sample_clock, 40, 40.0, 10, flags, Some(40))
    };
    let mut rig = Rig::new("leading-gap-at", BTreeMap::new());
    let at = AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 30));
    rig.env.actions.push(request(Value::from(10), Some(at)));
    let block = gap(&mut rig);
    rig.link.publish(block);
    rig.step().expect("one block");
    let artifacts = rig.stop(StopMode::Orderly);
    let map = &artifacts[0].continuity[0];
    assert_eq!(map.first.ticks_in(rig.env.sample_clock).unwrap(), 30);
    assert_eq!(map.gaps.len(), 1);
    assert_eq!((map.gaps[0].start.ticks_in(rig.env.sample_clock).unwrap(), map.gaps[0].len, map.gaps[0].lost), (30, 10, Some(10)));
    assert_eq!(map.gaps[0].cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});

    // With no start instant the capture begins at its first delivered sample.
    let mut rig = Rig::new("leading-gap-own", sample_count(10));
    let block = gap(&mut rig);
    rig.link.publish(block);
    rig.step().expect("one block");
    let artifacts = rig.stop(StopMode::Orderly);
    let map = &artifacts[0].continuity[0];
    assert_eq!((map.first.ticks_in(rig.env.sample_clock).unwrap(), map.gaps.len()), (40, 0));

    // A start instant at the block's first sample keeps none of the gap either.
    let mut rig = Rig::new("leading-gap-at-start", BTreeMap::new());
    let at = AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 40));
    rig.env.actions.push(request(Value::from(10), Some(at)));
    let block = gap(&mut rig);
    rig.link.publish(block);
    rig.step().expect("one block");
    let map = &rig.stop(StopMode::Orderly)[0].continuity[0];
    assert_eq!((map.first.ticks_in(rig.env.sample_clock).unwrap(), map.gaps.len()), (40, 0));
    let at = AbsoluteDeadline::new(TimePoint::new(rig.env.sample_clock, 30));

    // An unknown `lost` is clipped the same way: kept as a zero-extent gap at a block
    // after the start instant, cleared with no start instant.
    let unknown = |rig: &mut Rig| {
        let flags = BlockFlags::GAP_BEFORE;
        ramp_block(&mut rig.pool, rig.env.sample_clock, 40, 40.0, 10, flags, None)
    };
    let mut rig = Rig::new("leading-gap-unknown-at", BTreeMap::new());
    rig.env.actions.push(request(Value::from(10), Some(at)));
    let block = unknown(&mut rig);
    rig.link.publish(block);
    rig.step().expect("one block");
    let map = &rig.stop(StopMode::Orderly)[0].continuity[0];
    assert_eq!((map.first.ticks_in(rig.env.sample_clock).unwrap(), map.gaps.len(), map.gaps[0].len, map.gaps[0].lost), (40, 1, 0, None));
    let mut rig = Rig::new("leading-gap-unknown-own", sample_count(10));
    let block = unknown(&mut rig);
    rig.link.publish(block);
    rig.step().expect("one block");
    let map = &rig.stop(StopMode::Orderly)[0].continuity[0];
    assert_eq!((map.first.ticks_in(rig.env.sample_clock).unwrap(), map.gaps.len()), (40, 0));
}

#[test]
fn hd_11_an_unexpected_action_is_rejected() {
    let mut rig = Rig::new("unexpected-action", BTreeMap::new());
    rig.env.actions.push(tx_burst());
    assert!(rig.step().expect("unexpected Action is an event"));
    let events = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, ResourceId::parse("sink/rec").expect("event source"));
    assert_eq!(events[0].kind.as_str(), REQUEST_REJECTED);
    assert_eq!(events[0].payload["action"], "tx_burst");
    assert!(events[0].payload["reason"].as_str().unwrap().starts_with("HD-14"));
    rig.env.actions.push(Action::Command {
        target: ResourceId::parse("sink/rec").expect("command target"),
        verb: ident("start_rx"),
        params: BTreeMap::new(),
        at: None,
    });
    assert!(rig.step().expect("unexpected Command is an event"));
    let events = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(events[0].payload["action"], "command");
}

#[test]
fn hd_14_a_bad_capture_value_is_an_event_not_a_failure() {
    let mut rig = Rig::new("bad-capture-values", BTreeMap::new());
    rig.env.actions.push(request(Value::from(0), None));
    rig.env.actions.push(request(Value::num(5.0).unwrap(), None));
    assert!(rig.step().expect("bad values are rejected events"));
    rig.env.actions.push(request(
        Value::from(5),
        Some(AbsoluteDeadline::new(TimePoint::new(rig.env.unrelated_root, 0))),
    ));
    rig.env.actions.push(request(Value::from(10), None));
    rig.push_ramp(0, 10);
    assert!(rig.step().expect("unrelated time is rejected and valid request runs"));
    // Phase 6, VD-1: the served request is announced too (HD-16); the three refusals remain.
    let events: Vec<_> = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).into_iter().filter(|event| event.kind.as_str() != CAPTURE_WRITTEN).collect();
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
    aborted.env.actions.push(request(Value::from(100), None));
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
fn hd_13_stop_preserves_trailing_drop_carry() {
    for mode in [StopMode::Abort, StopMode::Orderly] {
        let mut rig = Rig::with_link("trailing-carry", sample_count(100), BackPressure::DropOldest, 1);
        rig.push_ramp(0, 2);
        rig.step().expect("begin capture");
        let overflow = ramp_block(&mut rig.pool, rig.env.sample_clock, 52, 52.0, 2,
            BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(50));
        assert_eq!(rig.link.publish(overflow), PublishOutcome::Accepted);
        assert_eq!(rig.push_ramp(54, 2), PublishOutcome::DroppedOldest);
        let artifacts = rig.stop(mode);
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].partial);
        assert_eq!(artifacts[0].size_bytes, 16);
        let map = &artifacts[0].continuity[0];
        assert_eq!(map.end.ticks_in(rig.env.sample_clock).unwrap(), 2);
        assert_eq!(map.gaps.len(), 1);
        assert_eq!(map.gaps[0].start.ticks_in(rig.env.sample_clock).unwrap(), 2);
        assert_eq!(map.gaps[0].len, 0);
        assert_eq!(map.gaps[0].lost, Some(50));
        assert_eq!(map.gaps[0].link_dropped, 1);
        assert_eq!(map.gaps[0].cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
        assert_eq!(rig.link.take_drop_carry().blocks, 0);
        assert_eq!(meta_of(&artifacts[0]).unwrap()["global"]["ezsdr:gaps"], json!([
            {"sample_start": 2, "global_index": 2, "len": 0, "lost": 50,
             "cause": {"kind": "overflow_restart"}, "link_dropped": 1}
        ]));
    }
}

#[test]
fn hd_11_targeted_stop_keeps_carry_on_the_finished_capture() {
    let mut rig = Rig::with_link("stop-carry", sample_count(100), BackPressure::DropOldest, 1);
    rig.push_ramp(0, 2);
    rig.step().expect("begin capture");
    let overflow = ramp_block(&mut rig.pool, rig.env.sample_clock, 52, 52.0, 2,
        BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(50));
    rig.link.publish(overflow);
    assert_eq!(rig.push_ramp(54, 2), PublishOutcome::DroppedOldest);
    rig.env.actions.push(Action::Stop { target: Some(ResourceId::parse("sink/rec").unwrap()) });
    rig.env.actions.push(request(Value::from(2), None));
    rig.step().expect("Stop then a new capture request");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 2);
    assert!(artifacts[0].partial);
    assert_eq!(artifacts[0].continuity[0].gaps.len(), 1);
    assert_eq!(artifacts[0].continuity[0].gaps[0].lost, Some(50));
    assert_eq!(artifacts[0].continuity[0].gaps[0].link_dropped, 1);
    assert!(!artifacts[1].partial);
    assert!(artifacts[1].continuity[0].gaps.is_empty());
    assert_ramp(&artifacts[1], 54, 2);
    assert_eq!(rig.link.take_drop_carry().blocks, 0);
}

#[test]
fn hd_11_stop_for_another_target_is_rejected() {
    let mut rig = Rig::new("stop-other-target", sample_count(8));
    rig.push_ramp(0, 4);
    rig.step().expect("start partial capture");
    rig.env.actions.push(Action::Stop { target: Some(ResourceId::parse("radio").expect("radio target")) });
    assert!(rig.step().expect("wrong-target Stop is rejected as an event"));
    let events = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
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
    // Phase 6, VD-1: the only event is the partial capture's announcement (HD-16).
    let events = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(events.iter().map(|event| event.kind.as_str()).collect::<Vec<_>>(), [CAPTURE_WRITTEN]);

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
    rig.env.actions.push(request(Value::from(3), None));
    rig.env.actions.push(Action::Stop {
        target: Some(ResourceId::parse("sink/rec").expect("Sink target")),
    });
    rig.step().expect("the Stop finishes the own capture");
    rig.env.actions.push(request(Value::from(2), None));
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

/// A block of `channels` channels whose sample `i` of channel `c` is `(first + i, c)`,
/// with the valid mask and flags given (HD-15's per-channel cases).
fn channel_block(pool: &mut HostPool, domain: ClockDomainId, first: i64, len: u32, channels: u16, valid: ChannelMask, flags: BlockFlags) -> BlockRef {
    let len_usize = len as usize;
    let bytes = pool.fill(len_usize * channels as usize * ezsdr_hostmem::CF32_BYTES, |buf| {
        for c in 0..channels as usize {
            for index in 0..len_usize {
                write_cf32(buf, len_usize, c, index, (first + index as i64) as f32, c as f32);
            }
        }
    });
    let header = BlockHeader {
        first_sample_time: TimePoint::new(domain, first),
        len,
        channels,
        direction: Direction::Rx,
        valid,
        flags,
        lost: None,
        contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract"),
    };
    BlockRef::new(
        SampleBlock::new_host(header, HOST_MEMORY, bytes, ezsdr_hostmem::CF32_BYTES as u32)
            .expect("well-formed CF32 block"),
    )
}

fn meta_of(artifact: &ArtifactRef) -> Option<serde_json::Value> {
    let data = artifact.uri.strip_prefix("file://").expect("file URI");
    let meta = Path::new(data).with_extension("sigmf-meta");
    fs::read_to_string(meta).ok().map(|text| serde_json::from_str(&text).expect("the metadata is JSON"))
}

#[test]
fn hd_15_a_capture_is_a_sigmf_recording() {
    let mut rig = Rig::new("sigmf-recording", sample_count(300));
    let before = ramp_block(&mut rig.pool, rig.env.sample_clock, 100, 100.0, 100, BlockFlags::NONE, None);
    rig.link.publish(before);
    let after = ramp_block(&mut rig.pool, rig.env.sample_clock, 250, 250.0, 200, BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(50));
    rig.link.publish(after);
    rig.step().expect("both blocks");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts.len(), 1);
    let artifact = &artifacts[0];
    assert!(!artifact.partial);
    assert!(artifact.uri.ends_with("_rec.sigmf-data"), "{}", artifact.uri);
    // The Dataset is HD-10's bytes: the delivered samples, the gap not filled.
    let samples = artifact_samples(artifact);
    assert_eq!(samples.len(), 300);
    assert_eq!((samples[0], samples[99], samples[100], samples[299]), (100.0, 199.0, 250.0, 449.0));
    assert_eq!(
        meta_of(artifact).expect("a one-map capture has metadata"),
        json!({
            "global": {
                "core:datatype": "cf32_le",
                "core:version": "1.2.6",
                "core:num_channels": 1,
                "core:sample_rate": 1_000_000.0,
                "core:recorder": "ezsdr.sink.capture 1.2.0",
                "core:extensions": [{ "name": "ezsdr", "version": "1.0.0", "optional": true }],
                "ezsdr:sample_rate": { "num": 1_000_000, "den": 1 },
                "ezsdr:partial": false,
                "ezsdr:gaps": [{
                    "sample_start": 100, "global_index": 200, "len": 50, "lost": 50,
                    "cause": { "kind": "overflow_restart" }, "link_dropped": 0
                }],
                "ezsdr:valid": [[
                    { "sample_start": 0, "sample_count": 100 },
                    { "sample_start": 100, "sample_count": 200 }
                ]]
            },
            "captures": [
                { "core:sample_start": 0, "core:global_index": 100 },
                { "core:sample_start": 100, "core:global_index": 250 }
            ],
            "annotations": []
        })
    );
}

#[test]
fn hd_15_channel_validity() {
    let mut rig = Rig::new("sigmf-channels", sample_count(40));
    let full = ChannelMask::full(2);
    let only_first = ChannelMask::from_bits(0b01);
    let domain = rig.env.sample_clock;
    for (first, valid, flags) in [
        (0, full, BlockFlags::NONE),
        (10, only_first, BlockFlags::NONE),
        (20, full, BlockFlags::NONE),
        (30, only_first, BlockFlags::NONE),
    ] {
        let block = channel_block(&mut rig.pool, domain, first, 10, 2, valid, flags);
        rig.link.publish(block);
    }
    rig.step().expect("four blocks");
    let artifacts = rig.stop(StopMode::Orderly);
    let meta = meta_of(&artifacts[0]).expect("metadata");
    assert_eq!(meta["global"]["core:num_channels"], 2);
    assert_eq!(meta["global"]["ezsdr:valid"], json!([
        [{ "sample_start": 0, "sample_count": 40 }],
        [{ "sample_start": 0, "sample_count": 10 }, { "sample_start": 20, "sample_count": 10 }]
    ]));
    // SC-31d: one annotation per break; SC-31c: a channel still invalid at the end has
    // a break running to the end.
    assert_eq!(meta["annotations"], json!([
        { "core:sample_start": 10, "core:sample_count": 10, "core:label": "invalid channel",
          "ezsdr:channel": 1 },
        { "core:sample_start": 30, "core:sample_count": 10, "core:label": "invalid channel",
          "ezsdr:channel": 1 }
    ]));
    assert_eq!(meta["captures"], json!([{ "core:sample_start": 0, "core:global_index": 0 }]));
}

#[test]
fn hd_15_a_capture_across_a_clock_change_has_no_metadata() {
    let mut rig = Rig::new("sigmf-clock-change", sample_count(8));
    let first = ramp_block(&mut rig.pool, rig.env.sample_clock, 0, 0.0, 4, BlockFlags::NONE, None);
    rig.link.publish(first);
    let changed = ramp_block(&mut rig.pool, rig.env.changed_clock, 0, 4.0, 4, BlockFlags::NONE, None);
    rig.link.publish(changed);
    rig.step().expect("two clocks");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts[0].continuity.len(), 2);
    assert!(artifacts[0].uri.ends_with(".sigmf-data"));
    assert!(Path::new(artifacts[0].uri.strip_prefix("file://").unwrap()).exists());
    assert_eq!(meta_of(&artifacts[0]), None, "one Recording has one sample rate");

    // The other way to two maps: a channel-count change (SC-30a).
    let mut rig = Rig::new("sigmf-channels-change", sample_count(8));
    let one = channel_block(&mut rig.pool, rig.env.sample_clock, 0, 4, 1, ChannelMask::full(1), BlockFlags::NONE);
    rig.link.publish(one);
    let two = channel_block(&mut rig.pool, rig.env.sample_clock, 4, 4, 2, ChannelMask::full(2), BlockFlags::NONE);
    rig.link.publish(two);
    rig.step().expect("two channel counts");
    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts[0].continuity.len(), 2);
    assert_eq!(meta_of(&artifacts[0]), None, "one Recording has one channel count");
}

#[test]
fn hd_15_the_datatype_follows_the_contract() {
    let clock = ClockDomainId::local(9);
    let header = BlockHeader {
        first_sample_time: TimePoint::new(clock, 0),
        len: 4,
        channels: 1,
        direction: Direction::Rx,
        valid: ChannelMask::full(1),
        flags: BlockFlags::NONE,
        lost: None,
        contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract"),
    };
    let mut builder = ezsdr_kernel::stream::ContinuityBuilder::new(clock, 1, false);
    builder.push(&header, DropCarry::default()).expect("one block");
    let map = builder.finish(DropCarry::default());
    let rate = Rational::new(3, 2).expect("rate");
    let datatype = |contract: &str| {
        sigmf_meta(&map, rate, &DataContractId::parse(contract).expect("contract id"), false)
            .map(|meta| meta["global"]["core:datatype"].clone())
    };
    assert_eq!(datatype("ezsdr.stream.cf32"), Ok(json!("cf32_le")));
    assert_eq!(datatype("ezsdr.stream.sc16"), Ok(json!("ci16_le")));
    assert!(datatype("ezsdr.stream.cs8").unwrap_err().starts_with("HD-15"));
    let meta = sigmf_meta(&map, rate, &DataContractId::parse("ezsdr.stream.sc16").unwrap(), false).unwrap();
    assert_eq!(meta["global"]["core:sample_rate"], json!(1.5));
    assert_eq!(meta["global"]["ezsdr:sample_rate"], json!({ "num": 3, "den": 2 }));
}

/// A one-channel map built from the headers given, then finished with `trailing`.
fn map_of(headers: &[(i64, u32, BlockFlags, Option<u64>, DropCarry)], trailing: DropCarry) -> ezsdr_kernel::stream::ContinuityMap {
    let clock = ClockDomainId::local(9);
    let mut builder = ezsdr_kernel::stream::ContinuityBuilder::new(clock, 1, false);
    for (first, len, flags, lost, carry) in headers {
        let header = BlockHeader {
            first_sample_time: TimePoint::new(clock, *first),
            len: *len,
            channels: 1,
            direction: Direction::Rx,
            valid: ChannelMask::full(1),
            flags: *flags,
            lost: *lost,
            contract: DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract"),
        };
        builder.push(&header, *carry).expect("a well-formed header");
    }
    builder.finish(trailing)
}

fn cf32() -> DataContractId {
    DataContractId::parse("ezsdr.stream.cf32").expect("cf32 contract")
}

#[test]
fn hd_15_gap_fields_carry_the_continuity_causes() {
    let none = DropCarry::default();
    let dropped = DropCarry { flags: BlockFlags::NONE, lost: None, blocks: 1 };
    let evicted_overflow = DropCarry { flags: BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, lost: Some(50), blocks: 1 };
    // [0, 10); a link drop of 5 (SC-31 LinkDrop); [15, 25); a stream gap of 50 that
    // reported 30 lost (Mixed); [75, 85); then a carry no block follows (SC-30c).
    let map = map_of(&[
        (0, 10, BlockFlags::NONE, None, none),
        (15, 10, BlockFlags::NONE, None, dropped),
        (75, 10, BlockFlags::GAP_BEFORE, Some(30), none),
    ], evicted_overflow);
    let meta = sigmf_meta(&map, Rational::integer(1).unwrap(), &cf32(), true).unwrap();
    assert_eq!(meta["global"]["ezsdr:partial"], true);
    assert_eq!(meta["global"]["ezsdr:gaps"], json!([
        { "sample_start": 10, "global_index": 10, "len": 5, "lost": null, "cause": { "kind": "link_drop" }, "link_dropped": 1 },
        { "sample_start": 20, "global_index": 25, "len": 50, "lost": 30, "cause": { "kind": "mixed", "stream_lost": 30 }, "link_dropped": 0 },
        { "sample_start": 30, "global_index": 85, "len": 0, "lost": 50, "cause": { "kind": "overflow_restart" }, "link_dropped": 1 }
    ]));
    // The zero-extent gap at the end opens no empty capture segment.
    assert_eq!(meta["captures"], json!([
        { "core:sample_start": 0, "core:global_index": 0 },
        { "core:sample_start": 10, "core:global_index": 15 },
        { "core:sample_start": 20, "core:global_index": 75 }
    ]));
}

#[test]
fn hd_15_annotations_are_sorted_by_index_then_channel() {
    // The builder emits a break when it closes: channel 2's [20, 30) closes before
    // channel 1's [10, 40), so the map lists channel 2 first (SC-31d).
    let clock = ClockDomainId::local(9);
    let mut builder = ezsdr_kernel::stream::ContinuityBuilder::new(clock, 3, false);
    for (first, valid) in [(0, 0b111), (10, 0b101), (20, 0b001), (30, 0b101), (40, 0b111)] {
        let header = BlockHeader {
            first_sample_time: TimePoint::new(clock, first),
            len: 10,
            channels: 3,
            direction: Direction::Rx,
            valid: ChannelMask::from_bits(valid),
            flags: BlockFlags::NONE,
            lost: None,
            contract: cf32(),
        };
        builder.push(&header, DropCarry::default()).expect("a well-formed header");
    }
    let map = builder.finish(DropCarry::default());
    assert_eq!(map.channel_gaps.iter().map(|g| g.channel).collect::<Vec<_>>(), [2, 1]);
    let meta = sigmf_meta(&map, Rational::integer(1).unwrap(), &cf32(), false).unwrap();
    let order: Vec<_> = meta["annotations"].as_array().unwrap().iter()
        .map(|a| (a["core:sample_start"].as_u64().unwrap(), a["ezsdr:channel"].as_u64().unwrap(), a["core:sample_count"].as_u64().unwrap()))
        .collect();
    assert_eq!(order, [(10, 1, 30), (20, 2, 10)]);
}

#[test]
fn hd_15_a_map_it_cannot_describe_is_refused() {
    // `sigmf_meta` is public and so are a map's fields: a hand-built map whose file
    // indices would be negative or out of range is refused, not written as invalid
    // SigMF (a `core:sample_start` below 0).
    let mut before_first = map_of(&[(100, 10, BlockFlags::NONE, None, DropCarry::default())], DropCarry::default());
    before_first.gaps.push(ezsdr_kernel::stream::Gap {
        start: TimePoint::new(before_first.domain, 0),
        len: 5,
        lost: None,
        cause: ezsdr_kernel::stream::GapCause::Stream {},
        link_dropped: 0,
    });
    assert!(sigmf_meta(&before_first, Rational::integer(1).unwrap(), &cf32(), false).unwrap_err().starts_with("HD-15"));
    let mut huge = map_of(&[(0, 10, BlockFlags::NONE, None, DropCarry::default())], DropCarry::default());
    huge.gaps.push(ezsdr_kernel::stream::Gap {
        start: TimePoint::new(huge.domain, 10),
        len: u64::MAX,
        lost: None,
        cause: ezsdr_kernel::stream::GapCause::Stream {},
        link_dropped: 0,
    });
    assert!(sigmf_meta(&huge, Rational::integer(1).unwrap(), &cf32(), false).unwrap_err().starts_with("HD-15"));
}

#[test]
fn hd_15_a_partial_capture_has_metadata() {
    let mut rig = Rig::new("sigmf-partial", sample_count(5_000));
    let block = ramp_block(&mut rig.pool, rig.env.sample_clock, 0, 0.0, 1_000, BlockFlags::NONE, None);
    rig.link.publish(block);
    rig.step().expect("one block");
    let artifacts = rig.stop(StopMode::Abort);
    assert!(artifacts[0].partial);
    let meta = meta_of(&artifacts[0]).expect("a partial capture is a Recording too");
    assert_eq!(meta["global"]["ezsdr:partial"], true);
    assert_eq!(meta["captures"], json!([{ "core:sample_start": 0, "core:global_index": 0 }]));
    let data = artifacts[0].uri.strip_prefix("file://").unwrap();
    assert!(!Path::new(data).with_extension("sigmf-meta.partial").exists(), "no staging file is left behind");
}

#[test]
fn hd_15_an_sc16_capture_is_ci16_le() {
    let mut rig = Rig::new("sigmf-sc16", sample_count(16));
    let len = 16usize;
    let bytes = rig.pool.fill(len * 2 * 4, |buf| {
        for c in 0..2 {
            for i in 0..len {
                let at = (c * len + i) * 4;
                buf[at..at + 2].copy_from_slice(&(i as i16).to_le_bytes());
                buf[at + 2..at + 4].copy_from_slice(&(c as i16).to_le_bytes());
            }
        }
    });
    let header = BlockHeader {
        first_sample_time: TimePoint::new(rig.env.sample_clock, 0),
        len: len as u32,
        channels: 2,
        direction: Direction::Rx,
        valid: ChannelMask::full(2),
        flags: BlockFlags::NONE,
        lost: None,
        contract: DataContractId::parse("ezsdr.stream.sc16").expect("sc16 contract"),
    };
    rig.link.publish(BlockRef::new(SampleBlock::new_host(header, HOST_MEMORY, bytes, 4).expect("sc16 block")));
    rig.step().expect("one block");
    let artifacts = rig.stop(StopMode::Orderly);
    let data = fs::read(artifacts[0].uri.strip_prefix("file://").unwrap()).unwrap();
    assert_eq!(data.len(), 16 * 2 * 4);
    // SigMF's interleave: sample 1 of channel 0, then of channel 1 (I then Q, LE).
    assert_eq!(&data[8..16], &[1, 0, 0, 0, 1, 0, 1, 0]);
    let meta = meta_of(&artifacts[0]).expect("metadata");
    assert_eq!(meta["global"]["core:datatype"], "ci16_le");
    assert_eq!(meta["global"]["core:num_channels"], 2);
}

#[test]
fn hd_16_a_written_capture_is_announced() {
    let mut rig = Rig::new("capture-written", BTreeMap::new());
    rig.env.actions.push(request(Value::from(1_000), None));
    rig.push_ramp(0, 600);
    rig.step().expect("the capture starts");
    assert!(rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).is_empty(), "nothing is announced before the capture is recorded");
    rig.push_ramp(600, 600);
    rig.step().expect("the capture completes");
    let first = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].kind.as_str(), CAPTURE_WRITTEN);
    assert_eq!(first[0].source, ResourceId::parse("sink/rec").expect("event source"));
    let announced: ezsdr_sink::CaptureWrittenPayload = serde_json::from_value(first[0].payload.clone()).expect("the payload is a CaptureWrittenPayload");
    assert_eq!(announced.request, Some(0), "the first capture request the Sink received");
    let path = PathBuf::from(announced.artifact.uri.strip_prefix("file://").expect("a file URI"));
    assert!(path.with_extension("sigmf-meta").exists(), "the metadata is written before the announcement");

    // A second request, finished partial by a Stop for the Sink (HD-11).
    rig.env.actions.push(request(Value::from(1_000), None));
    rig.push_ramp(1_200, 300);
    rig.step().expect("the second capture starts");
    rig.env.actions.push(Action::Stop { target: Some(ResourceId::parse("sink/rec").expect("Sink target")) });
    rig.step().expect("the Stop finishes it");
    let second = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].kind.as_str(), CAPTURE_WRITTEN);
    let partial: ezsdr_sink::CaptureWrittenPayload = serde_json::from_value(second[0].payload.clone()).expect("payload");
    assert!(partial.artifact.partial);
    assert_eq!(partial.request, Some(1));

    let artifacts = rig.stop(StopMode::Orderly);
    assert_eq!(artifacts, vec![announced.artifact, partial.artifact], "each announcement is the artifact stop returns");
}

#[test]
fn hd_16_every_capture_request_is_numbered() {
    // Refused or served, each capture request takes the next number, and the output's own
    // capture has none; a client waits for its own (Phase 6 Review H, P0-2).
    let mut rig = Rig::new("numbered", sample_count(4));
    rig.push_ramp(0, 4);
    rig.step().expect("the own capture completes");
    rig.env.actions.push(request(Value::from(0), None));
    rig.env.actions.push(request(Value::num(5.0).unwrap(), None));
    rig.env.actions.push(request(Value::from(5), Some(AbsoluteDeadline::new(TimePoint::new(rig.env.unrelated_root, 0)))));
    rig.env.actions.push(request(Value::from(3), None));
    rig.env.actions.push(tx_burst());
    rig.push_ramp(4, 10);
    rig.step().expect("the requests are handled");
    let numbers: Vec<(String, serde_json::Value)> = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().map(|event| (event.kind.as_str().to_owned(), event.payload["request"].clone())).collect();
    assert_eq!(numbers, vec![
        (CAPTURE_WRITTEN.to_owned(), json!(null)),
        (REQUEST_REJECTED.to_owned(), json!(0)),
        (REQUEST_REJECTED.to_owned(), json!(1)),
        (REQUEST_REJECTED.to_owned(), json!(null)),
        (REQUEST_REJECTED.to_owned(), json!(2)),
        (CAPTURE_WRITTEN.to_owned(), json!(3)),
    ]);
}

#[test]
fn hd_16_a_discarded_request_is_answered() {
    // A Stop for the Sink discards the requests that have not started; each is answered
    // with its number, so a client does not wait out its timeout (Review I, P2-1).
    let mut rig = Rig::new("discarded", BTreeMap::new());
    rig.env.actions.push(request(Value::from(8), None));
    rig.env.actions.push(request(Value::from(3), None));
    rig.push_ramp(0, 4);
    rig.step().expect("the first request starts");
    rig.env.actions.push(Action::Stop { target: Some(ResourceId::parse("sink/rec").expect("Sink target")) });
    rig.step().expect("the Stop");
    let answers: Vec<(String, serde_json::Value)> = rig.env.events.drain(ezsdr_kernel::time::TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0)).iter().map(|event| (event.kind.as_str().to_owned(), event.payload["request"].clone())).collect();
    assert_eq!(answers, vec![(CAPTURE_WRITTEN.to_owned(), json!(0)), (REQUEST_REJECTED.to_owned(), json!(1))]);
}

#[test]
fn hd_10_a_colliding_request_preserves_the_completed_recording() {
    let mut rig = Rig::new("collision", BTreeMap::new());
    let mut other = CaptureSink::from_binding(&binding(rig._temp.path(), false, None)).unwrap();
    let other_link = Arc::new(TestLink::new(BackPressure::DropOldest, 8));
    let mut own = fragment(sample_count(2));
    own.id = ident("rec_0");
    own.content["id"] = json!("rec_0");
    other.prepare(&own, rig.env.context(vec![attached(
        "rec_0", "in", Endpoint::StreamIn(other_link.clone()),
    )])).unwrap();
    other_link.publish(ramp_block(&mut rig.pool, rig.env.sample_clock,
        0, 0.0, 2, BlockFlags::NONE, None));
    other.step(TimePoint::new(rig.env.root, 0)).unwrap();
    let original = other.stop(StopMode::Orderly).unwrap().remove(0);
    let path = Path::new(original.uri.strip_prefix("file://").unwrap());
    let data = fs::read(path).unwrap();
    let meta_path = path.with_extension("sigmf-meta");
    let meta = fs::read(&meta_path).unwrap();

    rig.env.actions.push(request(Value::from(3), None));
    rig.push_ramp(2, 3);
    assert!(rig.step().unwrap_err().message.contains("cannot create"));
    assert_eq!(fs::read(path).unwrap(), data);
    assert_eq!(ContentHash::of_bytes(&data), original.hash);
    assert_eq!(fs::read(meta_path).unwrap(), meta);
    assert!(rig.stop(StopMode::Abort).is_empty());
}

#[test]
fn hd_10_contract_changes_keep_drop_carry_on_the_outgoing_capture() {
    fn publish(rig: &mut Rig, contract: &str, domain: ClockDomainId,
        first: i64, flags: BlockFlags, lost: Option<u64>) -> PublishOutcome {
        let bps = if contract == "ezsdr.stream.cf32" { 8 } else { 4 };
        let bytes = rig.pool.fill(2 * bps, |buf| buf.fill(0));
        let header = BlockHeader { first_sample_time: TimePoint::new(domain, first), len: 2,
            channels: 1, direction: Direction::Rx, valid: ChannelMask::full(1), flags, lost,
            contract: DataContractId::parse(contract).unwrap() };
        rig.link.publish(BlockRef::new(SampleBlock::new_host(header, HOST_MEMORY, bytes, bps as u32).unwrap()))
    }
    for (old, new) in [("ezsdr.stream.cf32", "ezsdr.stream.sc16"), ("ezsdr.stream.sc16", "ezsdr.stream.cf32")] {
        for queued in [false, true] {
            let mut rig = Rig::with_link("contract-carry", sample_count(100), BackPressure::DropOldest, 1);
            let old_clock = rig.env.sample_clock;
            publish(&mut rig, old, old_clock, 0, BlockFlags::NONE, None);
            rig.step().unwrap();
            if queued { rig.env.actions.push(request(Value::from(2), None)); }
            assert_eq!(publish(&mut rig, old, old_clock, 12,
                BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(10)), PublishOutcome::Accepted);
            let new_clock = rig.env.changed_clock;
            assert_eq!(publish(&mut rig, new, new_clock, 0, BlockFlags::NONE, None), PublishOutcome::DroppedOldest);
            rig.step().unwrap();
            let artifacts = rig.stop(StopMode::Orderly);
            assert_eq!(artifacts.len(), if queued { 2 } else { 1 });
            assert!(artifacts[0].partial);
            assert_eq!(artifacts[0].size_bytes, if old == "ezsdr.stream.cf32" { 16 } else { 8 });
            let map = &artifacts[0].continuity[0];
            assert_eq!(map.end.ticks_in(old_clock).unwrap(), 2); assert_eq!(map.gaps.len(), 1);
            assert_eq!(map.gaps[0].start.ticks_in(old_clock).unwrap(), 2); assert_eq!(map.gaps[0].len, 0);
            assert_eq!(map.gaps[0].lost, Some(10)); assert_eq!(map.gaps[0].link_dropped, 1);
            assert_eq!(map.gaps[0].cause, ezsdr_kernel::stream::GapCause::OverflowRestart {});
            if queued {
                assert!(!artifacts[1].partial);
                assert_eq!(artifacts[1].continuity[0].domain, new_clock);
                assert!(artifacts[1].continuity[0].gaps.is_empty());
                assert_eq!(artifacts[1].size_bytes, if new == "ezsdr.stream.cf32" { 16 } else { 8 });
            }
            assert_eq!(rig.link.take_drop_carry().blocks, 0);
        }
    }
}
