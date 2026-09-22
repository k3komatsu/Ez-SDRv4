//! Phase 1 tests for `02-stream-contract.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::sync::Arc;

use ezsdr_kernel::contract::{
    ContractRegistry, DataContract, DataContractId, PortRef, Scalar, standard_contracts,
};
use ezsdr_kernel::id::{ClockDomainId, DataLinkId, ResourceId};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BufferRef, BurstEnd, BurstOpen, BurstState, BurstStep,
    BurstTracker, ChannelMask, ContinuityBuilder, DataLink, DataLinkDecl, Direction, DropCarry,
    GapCause, HostMemoryAccess, LateOutcome, LatePolicy, PublishOutcome, SampleBlock, Segment,
    StreamError, admit_burst_target, check_sink_link,
};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Duration, EpochRef, Rational, TimeError, TimePoint,
};
use support::{
    CF32_BPS, GPU_MEM, MemLink, RetryingProducer, block, cf32, header, host_buffer,
};

const MCLK: u64 = 200_000_000;

fn rat(n: u64, d: u64) -> Rational {
    Rational::new(n, d).expect("valid rational")
}

fn arbitrary(s: &str) -> EpochRef {
    EpochRef::Arbitrary { set_by: s.to_owned() }
}

/// A registry with a 200 MHz root and one 20 Msps SampleClock at the given origin.
fn sample_clock(origin: i64) -> (Arc<ClockRegistry>, ClockDomainId, ClockDomainId) {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test"))).expect("root");
    let sc = reg.allocate_id();
    reg.register(ClockDomain::derived(sc, root, rat(10, 1), origin)).expect("derived");
    (reg, root, sc)
}

fn dom() -> ClockDomainId {
    ClockDomainId::local(7)
}

fn t(ticks: i64) -> TimePoint {
    TimePoint::new(dom(), ticks)
}

// ---------------------------------------------------------------- contracts

#[test]
fn sc_04_standard_contracts_fixture() {
    let reg = ContractRegistry::with_standard_contracts();
    let cf32 = reg.get(&cf32()).expect("cf32 registered");
    assert_eq!(cf32.attributes.get("bytes_per_sample"), Some(&Scalar::Int(8)));
    assert_eq!(cf32.attributes.get("full_scale"), Some(&Scalar::Float(1.0)));
    assert_eq!(cf32.attributes.get("layout"), Some(&Scalar::Str("planar".to_owned())));
    assert!(cf32.compatible_from.is_empty());
    assert_eq!(cf32.bytes_per_sample(), Some(CF32_BPS));

    let sc16 = reg.get(&DataContractId::parse("ezsdr.stream.sc16").unwrap()).expect("sc16");
    assert_eq!(sc16.attributes.get("bytes_per_sample"), Some(&Scalar::Int(4)));
    assert_eq!(sc16.attributes.get("full_scale"), Some(&Scalar::Float(32767.0)));
    assert!(sc16.compatible_from.is_empty());
}

#[test]
fn sc_02_contract_registry_conflict() {
    let reg = ContractRegistry::new();
    let mut c = standard_contracts().into_iter().next().expect("cf32 first");
    assert!(reg.register(c.clone()).is_ok());
    assert!(reg.register(c.clone()).is_ok(), "an identical re-registration is a no-op");
    c.attributes.insert("full_scale".to_owned(), Scalar::Float(2.0));
    assert!(reg.register(c).is_err());
}

#[test]
fn sc_03_contract_identity_or_compat() {
    let reg = ContractRegistry::with_standard_contracts();
    let cf32 = cf32();
    let sc16 = DataContractId::parse("ezsdr.stream.sc16").unwrap();
    assert!(reg.check_link(&cf32, &cf32).is_ok());
    assert!(matches!(reg.check_link(&cf32, &sc16), Err(StreamError::Incompatible { .. })));

    // A fixture whose `compatible_from` holds Y, checked in both directions.
    let x = DataContractId::parse("test.x").unwrap();
    let y = DataContractId::parse("test.y").unwrap();
    reg.register(DataContract {
        id: y.clone(),
        attributes: Default::default(),
        compatible_from: Default::default(),
    })
    .unwrap();
    reg.register(DataContract {
        id: x.clone(),
        attributes: Default::default(),
        compatible_from: [y.clone()].into_iter().collect(),
    })
    .unwrap();
    assert!(reg.check_link(&y, &x).is_ok(), "the check is directional");
    assert!(matches!(reg.check_link(&x, &y), Err(StreamError::Incompatible { .. })));
}

// ---------------------------------------------------------------- blocks

#[test]
fn sc_10_block_rejects_invalid_shape() {
    let buf = host_buffer(4, 2000);
    let bad = |h: BlockHeader| SampleBlock::new(h, buf, CF32_BPS);

    let mut h = header(t(0), 0, 1);
    assert!(matches!(bad(h), Err(StreamError::InvalidBlock { .. })), "len 0");

    h = header(t(0), 10, 1);
    h.channels = 0;
    assert!(matches!(bad(h), Err(StreamError::InvalidBlock { .. })), "channels 0");

    h = header(t(0), 10, 1);
    h.channels = 65;
    assert!(matches!(bad(h), Err(StreamError::InvalidBlock { .. })), "channels 65");

    h = header(t(0), 10, 2);
    h.valid = ChannelMask(0b100);
    assert!(matches!(bad(h), Err(StreamError::InvalidBlock { .. })), "valid bit above channels");

    h = header(t(0), 10, 2);
    h.flags = BlockFlags(0x0100);
    assert!(matches!(bad(h), Err(StreamError::InvalidBlock { .. })), "reserved flag bit");
}

#[test]
fn sc_10a_block_rejects_undersized_buffer() {
    let h = header(t(0), 2000, 4);
    let small = BufferRef { memory_domain: support::HOST_MEM, handle: 1, len_bytes: 32_000 };
    assert!(matches!(
        SampleBlock::new(h.clone(), small, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
    let exact = BufferRef { len_bytes: 64_000, ..small };
    assert!(SampleBlock::new(h, exact, CF32_BPS).is_ok());
}

#[test]
fn sc_16_direction_flags_rejected() {
    let buf = host_buffer(1, 10);
    let mut rx = header(t(0), 10, 1);
    rx.flags = BlockFlags::START_OF_BURST;
    assert!(matches!(
        SampleBlock::new(rx, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));

    let mut tx = header(t(0), 10, 1);
    tx.direction = Direction::Tx;
    tx.flags = BlockFlags::GAP_BEFORE;
    assert!(matches!(
        SampleBlock::new(tx, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
}

#[test]
fn sc_10_block_partial_channels_derived() {
    let buf = host_buffer(2, 10);
    let mut h = header(t(0), 10, 2);
    h.valid = ChannelMask(0b01);
    let b = SampleBlock::new(h, buf, CF32_BPS).expect("valid");
    assert!(b.header().flags.contains(BlockFlags::PARTIAL_CHANNELS));

    let h = header(t(0), 10, 2); // mask 0b11
    let b = SampleBlock::new(h, buf, CF32_BPS).expect("valid");
    assert!(!b.header().flags.contains(BlockFlags::PARTIAL_CHANNELS));

    let mut h = header(t(0), 10, 2);
    h.flags = BlockFlags::PARTIAL_CHANNELS;
    assert!(matches!(
        SampleBlock::new(h, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
}

#[test]
fn sc_10_block_flag_implications() {
    let buf = host_buffer(1, 10);
    let mut h = header(t(0), 10, 1);
    h.flags = BlockFlags::RESTARTED;
    assert!(matches!(SampleBlock::new(h, buf, CF32_BPS), Err(StreamError::InvalidBlock { .. })));

    let mut h = header(t(0), 10, 1);
    h.lost = Some(5);
    assert!(matches!(SampleBlock::new(h, buf, CF32_BPS), Err(StreamError::InvalidBlock { .. })));

    let mut h = header(t(0), 10, 1);
    h.direction = Direction::Tx;
    h.flags = BlockFlags::START_OF_BURST | BlockFlags::GAP_BEFORE;
    assert!(matches!(SampleBlock::new(h, buf, CF32_BPS), Err(StreamError::InvalidBlock { .. })));

    let mut h = header(t(0), 10, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(0);
    assert!(matches!(SampleBlock::new(h, buf, CF32_BPS), Err(StreamError::InvalidBlock { .. })));
}

#[test]
fn sc_17_flag_bit_positions_are_fixed_by_the_document() {
    // Decision S17 rejects "an ordering that exists only in the Rust source", and
    // SC-17's purpose — an Executor propagating flags unchanged one-to-one — is a
    // numeric contract.
    assert_eq!(BlockFlags::GAP_BEFORE, BlockFlags(0x0001));
    assert_eq!(BlockFlags::SEQ_DISCONTINUITY, BlockFlags(0x0002));
    assert_eq!(BlockFlags::RESTARTED, BlockFlags(0x0004));
    assert_eq!(BlockFlags::LATE, BlockFlags(0x0008));
    assert_eq!(BlockFlags::PARTIAL_CHANNELS, BlockFlags(0x0010));
    assert_eq!(BlockFlags::START_OF_BURST, BlockFlags(0x0020));
    assert_eq!(BlockFlags::END_OF_BURST, BlockFlags(0x0040));
    assert_eq!(BlockFlags::ALIGNMENT, BlockFlags(0x0080));
    assert_eq!(BlockFlags::RESERVED, BlockFlags(0xFF00), "bits 8-15 are reserved");
    assert_eq!(BlockFlags::NONE, BlockFlags(0));
}

#[test]
fn sc_19_link_declares_a_policy_and_a_capacity() {
    // There is no default policy, and both fields are mandatory in the type.
    let decl = DataLinkDecl {
        id: DataLinkId::local(1),
        from: PortRef { component: "a".into(), port: "out".into() },
        to: PortRef { component: "b".into(), port: "in".into() },
        contract: cf32(),
        policy: BackPressure::DropOldest,
        capacity: 4,
    };
    let json = serde_json::to_value(&decl).expect("serialises");
    assert!(json.get("policy").is_some() && json.get("capacity").is_some());
    let mut without = json.clone();
    without.as_object_mut().expect("object").remove("policy");
    assert!(
        serde_json::from_value::<DataLinkDecl>(without).is_err(),
        "SC-19: there is no default policy"
    );
    assert!(BackPressure::DropOldest.is_drop_class());
    assert!(!BackPressure::Block.is_drop_class(), "a drop-class link never returns Full");
}

#[test]
fn sc_11_block_fanout_shares_reference() {
    let b = block(header(t(0), 100, 1));
    let a = MemLink::new(BackPressure::Block, 4);
    let c = MemLink::new(BackPressure::Block, 4);
    assert_eq!(a.publish(b.clone()), PublishOutcome::Accepted);
    assert_eq!(c.publish(b.clone()), PublishOutcome::Accepted);
    let ra = a.receive().expect("queued");
    let rc = c.receive().expect("queued");
    assert!(Arc::ptr_eq(&ra, &rc), "fan-out shares one reference; nothing is copied");
    assert_eq!(Arc::strong_count(&b), 3);
}

#[test]
fn sc_08_buffer_map_host_none_for_gpu_domain() {
    let link = MemLink::new(BackPressure::Block, 2);
    let host = block(header(t(0), 10, 1));
    assert!(link.map_host(&host).is_some());

    let h = header(t(0), 10, 1);
    let gpu = BufferRef { memory_domain: GPU_MEM, handle: 0xdead, len_bytes: 1 << 20 };
    let gpu = Arc::new(SampleBlock::new(h, gpu, CF32_BPS).expect("valid"));
    assert!(link.map_host(&gpu).is_none());
}

// ---------------------------------------------------------------- links

#[test]
fn sc_20_link_block_policy_full() {
    let link = MemLink::new(BackPressure::Block, 2);
    assert_eq!(link.publish(block(header(t(0), 10, 1))), PublishOutcome::Accepted);
    assert_eq!(link.publish(block(header(t(10), 10, 1))), PublishOutcome::Accepted);
    assert_eq!(link.publish(block(header(t(20), 10, 1))), PublishOutcome::Full);
    assert_eq!(link.drops(), 0, "nothing is ever dropped under Block");
    link.receive().expect("queued");
    assert_eq!(link.publish(block(header(t(20), 10, 1))), PublishOutcome::Accepted);
}

#[test]
fn sc_20_link_drop_oldest() {
    let link = MemLink::new(BackPressure::DropOldest, 2);
    for i in 0..3 {
        link.publish(block(header(t(i * 10), 10, 1)));
    }
    assert_eq!(link.receive().expect("queued").first_sample_time(), t(10));
    assert_eq!(link.receive().expect("queued").first_sample_time(), t(20));
    assert_eq!(link.drops(), 1);
}

#[test]
fn sc_20_link_drop_newest() {
    let link = MemLink::new(BackPressure::DropNewest, 2);
    for i in 0..3 {
        link.publish(block(header(t(i * 10), 10, 1)));
    }
    assert_eq!(link.receive().expect("queued").first_sample_time(), t(0));
    assert_eq!(link.receive().expect("queued").first_sample_time(), t(10));
    assert_eq!(link.drops(), 1);
}

#[test]
fn sc_20a_full_is_not_a_silent_drop() {
    let link = MemLink::new(BackPressure::Block, 1);
    let mut producer = RetryingProducer::new();
    assert_eq!(producer.offer(&link, block(header(t(0), 10, 1))), PublishOutcome::Accepted);
    assert_eq!(producer.offer(&link, block(header(t(10), 10, 1))), PublishOutcome::Full);
    // The producer still owns it; it must not discard it.
    link.receive().expect("the first block");
    assert_eq!(producer.retry(&link), Some(PublishOutcome::Accepted));
    assert_eq!(link.receive().expect("the retried block").first_sample_time(), t(10));
    assert_eq!(link.drops(), 0);
    assert_eq!(producer.delivered, 2);
    assert_eq!(producer.refusals, 1);
}

#[test]
fn sc_20b_drop_carry_preserves_attribution() {
    let link = MemLink::new(BackPressure::DropOldest, 1);
    let mut h = header(t(100), 10, 1);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(150);
    link.publish(block(h));
    link.publish(block(header(t(200), 10, 1)));

    let carry = link.take_drop_carry();
    assert!(carry.flags.contains(BlockFlags::GAP_BEFORE));
    assert!(carry.flags.contains(BlockFlags::RESTARTED));
    assert_eq!(carry.lost, Some(150));
    assert_eq!(carry.blocks, 1);
    assert!(link.take_drop_carry().is_empty(), "the carry is cleared when read");
    assert_eq!(link.drops(), 1, "the drop counter never resets");
}

#[test]
fn sc_21_sink_links_must_be_drop_class() {
    let decl = |policy| DataLinkDecl {
        id: DataLinkId::local(3),
        from: PortRef { component: "rx".into(), port: "out".into() },
        to: PortRef { component: "rec".into(), port: "in".into() },
        contract: cf32(),
        policy,
        capacity: 8,
    };
    assert!(check_sink_link(&decl(BackPressure::Block), true).is_err());
    assert!(check_sink_link(&decl(BackPressure::DropOldest), true).is_ok());
    assert!(check_sink_link(&decl(BackPressure::Block), false).is_ok());
}

// ---------------------------------------------------------------- burst tracker

fn tx(at: i64, len: u32, flags: BlockFlags) -> BlockHeader {
    let mut h = header(t(at), len, 1);
    h.direction = Direction::Tx;
    h.flags = flags;
    h
}

fn open() -> Option<BurstOpen> {
    Some(BurstOpen::default())
}

#[test]
fn sc_24_burst_sob_eob_basic() {
    let mut tr = BurstTracker::new(dom());
    assert_eq!(tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()), Ok(BurstStep::Started));
    assert_eq!(tr.on_block(&tx(1100, 50, BlockFlags::NONE), None), Ok(BurstStep::Continued));
    let step = tr.on_block(&tx(1150, 10, BlockFlags::END_OF_BURST), None).expect("ends");
    let BurstStep::Ended { record } = step else { panic!("expected Ended, got {step:?}") };
    assert_eq!(record.samples, 160);
    assert_eq!(record.blocks, 3);
    assert_eq!(record.target, t(1000));
    assert_eq!(record.end, BurstEnd::Eob);
    assert_eq!(tr.state(), BurstState::Idle);
}

#[test]
fn sc_24_burst_single_block() {
    let mut tr = BurstTracker::new(dom());
    let step = tr
        .on_block(&tx(1000, 100, BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST), open())
        .expect("one-block burst");
    let BurstStep::Ended { record } = step else { panic!("expected Ended") };
    assert_eq!(record.samples, 100);
    assert_eq!(record.blocks, 1);
    assert_eq!(tr.state(), BurstState::Idle);
}

#[test]
fn sc_24_burst_missing_sob() {
    let mut tr = BurstTracker::new(dom());
    assert_eq!(
        tr.on_block(&tx(1000, 100, BlockFlags::NONE), None),
        Err(StreamError::MissingStartOfBurst)
    );
}

#[test]
fn sc_24_burst_forward_jump_is_discontinuity() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()).expect("start");
    let step = tr.on_block(&tx(1300, 100, BlockFlags::NONE), None).expect("recovers");
    let BurstStep::Discontinuity { expected, got, closed } = step else { panic!("expected a discontinuity") };
    assert_eq!((expected, got), (t(1100), t(1300)));
    assert_eq!(closed.end, BurstEnd::Discontinuity);
    assert_eq!(closed.samples, 100);
    assert!(matches!(tr.state(), BurstState::InBurst { target, .. } if target == t(1300)));
}

#[test]
fn sc_24_burst_backward_time_is_discontinuity() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()).expect("start");
    let step = tr.on_block(&tx(1050, 100, BlockFlags::NONE), None).expect("recovers");
    assert!(matches!(step, BurstStep::Discontinuity { got, .. } if got == t(1050)));
}

#[test]
fn sc_24_burst_sob_inside_burst() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()).expect("start");
    let step = tr.on_block(&tx(1100, 100, BlockFlags::START_OF_BURST), open()).expect("recovers");
    let BurstStep::Discontinuity { expected, got, closed } = step else { panic!("expected a discontinuity") };
    assert_eq!((expected, got), (t(1100), t(1100)));
    assert_eq!(closed.samples, 100);
    assert!(matches!(tr.state(), BurstState::InBurst { target, .. } if target == t(1100)));
}

#[test]
fn sc_25_burst_continuous_until_stop() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(0, 100, BlockFlags::START_OF_BURST), open()).expect("start");
    for i in 1..1000 {
        assert_eq!(
            tr.on_block(&tx(i * 100, 100, BlockFlags::NONE), None),
            Ok(BurstStep::Continued),
            "block {i}"
        );
    }
    let record = tr.stop().expect("an open burst");
    assert_eq!(record.end, BurstEnd::Stop);
    assert_eq!(record.samples, 100_000);
    assert_eq!(tr.state(), BurstState::Idle);
    assert!(tr.stop().is_none());
}

#[test]
fn sc_26_burst_repeat_wrap_contiguous() {
    // The v3 tail pattern: a waveform of 1 000 sent as 300, 300, 300, 100, then 300.
    let mut tr = BurstTracker::new(dom());
    let open = Some(BurstOpen { waveform_len: Some(1000), ..BurstOpen::default() });
    tr.on_block(&tx(1000, 300, BlockFlags::START_OF_BURST), open).expect("start");
    for (at, len) in [(1300, 300), (1600, 300), (1900, 100), (2000, 300)] {
        assert_eq!(
            tr.on_block(&tx(at, len, BlockFlags::NONE), None),
            Ok(BurstStep::Continued),
            "at {at}"
        );
    }
}

#[test]
fn sc_26_burst_repeat_wrap_off_by_one() {
    let mut tr = BurstTracker::new(dom());
    let open = Some(BurstOpen { waveform_len: Some(1000), ..BurstOpen::default() });
    tr.on_block(&tx(1000, 300, BlockFlags::START_OF_BURST), open).expect("start");
    for (at, len) in [(1300, 300), (1600, 300), (1900, 100)] {
        tr.on_block(&tx(at, len, BlockFlags::NONE), None).expect("contiguous");
    }
    let step = tr.on_block(&tx(2001, 300, BlockFlags::NONE), None).expect("recovers");
    assert!(matches!(step, BurstStep::Discontinuity { expected, got, .. } if expected == t(2000) && got == t(2001)));
}

#[test]
fn sc_29a_wraps_counted_from_burst_open() {
    let mut tr = BurstTracker::new(dom());
    let open = Some(BurstOpen { waveform_len: Some(1000), ..BurstOpen::default() });
    tr.on_block(&tx(0, 500, BlockFlags::START_OF_BURST), open).expect("start");
    for i in 1..7 {
        tr.on_block(&tx(i * 500, 500, BlockFlags::NONE), None).expect("contiguous");
    }
    let record = tr.stop().expect("open"); // 3 500 samples of a 1 000-sample waveform
    assert_eq!(record.samples, 3500);
    assert_eq!(record.wraps, 3);

    // Without a BurstOpen the tracker refuses rather than reporting a constant zero.
    let mut tr = BurstTracker::new(dom());
    assert!(matches!(
        tr.on_block(&tx(0, 500, BlockFlags::START_OF_BURST), None),
        Err(StreamError::InvalidBlock { .. })
    ));
}

#[test]
fn sc_29a_late_and_actual_start_reach_the_record() {
    // SC-29a's whole argument is that these ship as constants if the Provider does
    // not supply them, so the record must show them when it does.
    let mut tr = BurstTracker::new(dom());
    let late_by = Duration::new(ClockDomainId::HOST_MONOTONIC, 42);
    let open = Some(BurstOpen {
        waveform_len: Some(200),
        late: Some(LateOutcome::SendAsap { late_by }),
        requested_target: None,
    });
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open).expect("start");
    tr.set_actual_start(t(1003));
    let step = tr.on_block(&tx(1100, 100, BlockFlags::END_OF_BURST), None).expect("ends");
    let BurstStep::Ended { record } = step else { panic!("expected Ended") };
    assert_eq!(record.late_by, Some(late_by));
    assert_eq!(record.actual_start, Some(t(1003)));
    assert_eq!(record.wraps, 1);
}

#[test]
fn sc_23a_requested_target_reaches_the_record() {
    // SC-23a: "both times are recorded: the BurstRecord keeps `requested_target`
    // alongside the applied `target`". The admission result travels on the block
    // that opens the burst, like the other fields no header can carry (SC-29a).
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test"))).expect("root");
    let txdom = reg.allocate_id();
    reg.register(ClockDomain::derived(txdom, root, rat(8, 1), 0)).expect("derived");

    let admitted = admit_burst_target(&reg, TimePoint::new(root, 70), txdom).expect("admitted");
    assert_eq!(admitted.target, TimePoint::new(txdom, 9));

    let mut tr = BurstTracker::new(txdom);
    let mut h = header(admitted.target, 100, 1);
    h.direction = Direction::Tx;
    h.flags = BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST;
    let open = Some(BurstOpen {
        waveform_len: None,
        late: None,
        requested_target: admitted.requested_target,
    });
    let BurstStep::Ended { record } = tr.on_block(&h, open).expect("one-block burst") else {
        panic!("expected Ended")
    };
    assert_eq!(record.target, TimePoint::new(txdom, 9));
    assert_eq!(record.requested_target, Some(TimePoint::new(root, 70)));
}

#[test]
fn sc_24_burst_domain_mismatch() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()).expect("start");
    let mut h = tx(1100, 100, BlockFlags::NONE);
    h.first_sample_time = TimePoint::new(ClockDomainId::local(8), 1100);
    assert!(matches!(
        tr.on_block(&h, None),
        Err(StreamError::Time(TimeError::DomainMismatch { .. }))
    ));
}

// ---------------------------------------------------------------- targets and lateness

#[test]
fn sc_23a_tx_target_advances_to_next_sample() {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test"))).expect("root");
    let tx = reg.allocate_id();
    reg.register(ClockDomain::derived(tx, root, rat(8, 1), 0)).expect("derived");

    let got = admit_burst_target(&reg, TimePoint::new(root, 70), tx).expect("admitted");
    assert_eq!(got.target, TimePoint::new(tx, 9), "70/8 is between samples 8 and 9");
    assert_eq!(got.requested_target, Some(TimePoint::new(root, 70)));

    let got = admit_burst_target(&reg, TimePoint::new(root, 40), tx).expect("admitted");
    assert_eq!(got.target, TimePoint::new(tx, 5));
    assert_eq!(got.requested_target, None, "nothing was advanced");
}

#[test]
fn sc_23a_reactive_target_across_disjoint_grids() {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test"))).expect("root");
    // TM-13b makes each stream's origin its own first sample, so the grids are disjoint.
    let rx = reg.allocate_id();
    reg.register(ClockDomain::derived(rx, root, rat(10, 1), 1_000_000_000)).expect("rx");
    let tx = reg.allocate_id();
    reg.register(ClockDomain::derived(tx, root, rat(10, 1), 1_000_000_003)).expect("tx");

    // The natural reactive computation: rx_time + turnaround, in the receive domain.
    let target = TimePoint::new(rx, 100);
    let got = admit_burst_target(&reg, target, tx).expect("admitted, not refused");
    assert_eq!(got.target, TimePoint::new(tx, 100), "advanced by at most one sample");
    assert_eq!(got.requested_target, Some(target), "both times are recorded");
}

#[test]
fn sc_23b_tx_target_unrelated_domain_refused() {
    let (reg, _root, sc) = sample_clock(0);
    let other = reg.allocate_id();
    reg.register(ClockDomain::root(other, rat(MCLK, 1), arbitrary("other"))).expect("root");
    assert!(matches!(
        admit_burst_target(&reg, TimePoint::new(other, 400), sc),
        Err(StreamError::Time(TimeError::Unrelated { .. }))
    ));
}

#[test]
fn sc_27_late_policy_decisions() {
    let (reg, _root, sc) = sample_clock(0);
    let host = ClockDomainId::HOST_MONOTONIC;
    let min_lead = Duration::new(host, 1_000_000); // 1 ms
    let now = TimePoint::new(sc, 0);
    let far = TimePoint::new(sc, 40_000); // 2 ms at 20 Msps
    let near = TimePoint::new(sc, 10_000); // 0.5 ms

    assert_eq!(
        LatePolicy::SendAsapAndFlag.decide(&reg, far, now, min_lead),
        Ok(LateOutcome::OnTime)
    );
    let late_by = Duration::new(host, 500_000);
    assert_eq!(
        LatePolicy::SendAsapAndFlag.decide(&reg, near, now, min_lead),
        Ok(LateOutcome::SendAsap { late_by })
    );
    assert_eq!(
        LatePolicy::DropAndFlag.decide(&reg, near, now, min_lead),
        Ok(LateOutcome::Drop { late_by })
    );
    assert_eq!(
        LatePolicy::RejectAtPlan.decide(&reg, near, now, min_lead),
        Ok(LateOutcome::PlanViolation { late_by })
    );
}

#[test]
fn sc_27_late_policy_domain_check() {
    let (reg, root, sc) = sample_clock(0);
    let min_lead = Duration::new(ClockDomainId::HOST_MONOTONIC, 1_000_000);
    assert!(matches!(
        LatePolicy::DropAndFlag.decide(&reg, TimePoint::new(sc, 10), TimePoint::new(root, 0), min_lead),
        Err(TimeError::DomainMismatch { .. })
    ));
}

#[test]
fn sc_27_min_lead_cross_multiplied() {
    let reg = ClockRegistry::new();
    let host = ClockDomainId::HOST_MONOTONIC;
    let min_lead = Duration::new(host, 1_000_000); // 1 ms
    let hz3 = reg.allocate_id();
    reg.register(ClockDomain::root(hz3, rat(3, 1), arbitrary("test"))).expect("root");
    let lte = reg.allocate_id();
    reg.register(ClockDomain::root(lte, rat(30_720_000, 1), arbitrary("test"))).expect("root");

    // 3 Hz: one tick is 333 ms, comfortably over; zero ticks is under. A
    // rescale-and-round implementation reports the first case 333 times too strict.
    let p = LatePolicy::DropAndFlag;
    assert_eq!(p.decide(&reg, TimePoint::new(hz3, 1), TimePoint::new(hz3, 0), min_lead), Ok(LateOutcome::OnTime));
    assert_eq!(
        p.decide(&reg, TimePoint::new(hz3, 0), TimePoint::new(hz3, 0), min_lead),
        Ok(LateOutcome::Drop { late_by: Duration::new(host, 1_000_000) })
    );
    // 30.72 Msps: 1 ms is exactly 30 720 ticks, and one tick is 3125/96 ns.
    assert_eq!(
        p.decide(&reg, TimePoint::new(lte, 30_720), TimePoint::new(lte, 0), min_lead),
        Ok(LateOutcome::OnTime)
    );
    assert_eq!(
        p.decide(&reg, TimePoint::new(lte, 30_719), TimePoint::new(lte, 0), min_lead),
        Ok(LateOutcome::Drop { late_by: Duration::new(host, 33) })
    );
}

// ---------------------------------------------------------------- continuity

fn builder(channels: u16, lossless: bool) -> ContinuityBuilder {
    ContinuityBuilder::new(dom(), channels, lossless)
}

fn push(b: &mut ContinuityBuilder, h: BlockHeader) {
    b.push(&h, DropCarry::default()).expect("accepted");
}

#[test]
fn sc_30_continuity_contiguous() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 100, 1));
    push(&mut b, header(t(100), 100, 1));
    let map = b.finish(DropCarry::default());
    assert_eq!(map.valid[0], vec![Segment { start: t(0), len: 200 }]);
    assert!(map.gaps.is_empty());
    assert_eq!((map.first, map.end), (t(0), t(200)));
}

#[test]
fn sc_13_continuity_gap_before_known_lost() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 100, 1));
    let mut h = header(t(250), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(150);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].len, 150);
    assert_eq!(map.gaps[0].lost, Some(150));
    assert_eq!(map.gaps[0].cause, GapCause::Stream);
}

#[test]
fn sc_13_continuity_gap_flag_without_jump() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 100, 1));
    let mut h = header(t(100), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(1);
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err((StreamError::GapFlagWithoutJump, _))
    ));
}

#[test]
fn sc_30_continuity_jump_without_flag() {
    let mut lossless = builder(1, true);
    push(&mut lossless, header(t(0), 100, 1));
    assert!(matches!(
        lossless.push(&header(t(500), 100, 1), DropCarry::default()),
        Err((StreamError::JumpWithoutGapFlag, _))
    ));

    let mut lossy = builder(1, false);
    push(&mut lossy, header(t(0), 100, 1));
    push(&mut lossy, header(t(500), 100, 1));
    let map = lossy.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!((map.gaps[0].start, map.gaps[0].len), (t(100), 400));
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop);
}

#[test]
fn sc_12_continuity_overlap_is_error() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 200, 1));
    assert!(matches!(
        b.push(&header(t(150), 100, 1), DropCarry::default()),
        Err((StreamError::TimeOverlap { .. }, _))
    ));
}

#[test]
fn sc_31_continuity_causes() {
    let case = |flags: BlockFlags, lost: Option<u64>| {
        let mut b = builder(1, true);
        push(&mut b, header(t(0), 100, 1));
        let mut h = header(t(300), 100, 1);
        h.flags = flags;
        h.lost = lost;
        push(&mut b, h);
        b.finish(DropCarry::default()).gaps[0].cause
    };
    assert_eq!(
        case(BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED, Some(200)),
        GapCause::OverflowRestart
    );
    assert_eq!(
        case(BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY, Some(200)),
        GapCause::SequenceError
    );
    assert_eq!(case(BlockFlags::GAP_BEFORE, None), GapCause::Unknown);
}

#[test]
fn sc_31_continuity_mixed_on_lossy() {
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let mut h = header(t(500), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(150);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps[0].len, 400);
    assert_eq!(map.gaps[0].cause, GapCause::Mixed { stream_lost: 150 });
}

#[test]
fn sc_14_continuity_per_channel_segments() {
    let mut b = builder(2, true);
    push(&mut b, header(t(0), 100, 2));
    let mut h = header(t(100), 100, 2);
    h.valid = ChannelMask(0b01);
    push(&mut b, h);
    push(&mut b, header(t(200), 100, 2));
    let map = b.finish(DropCarry::default());
    assert_eq!(map.valid[0], vec![Segment { start: t(0), len: 300 }]);
    assert_eq!(
        map.valid[1],
        vec![Segment { start: t(0), len: 100 }, Segment { start: t(200), len: 100 }]
    );
}

#[test]
fn sc_30_continuity_domain_change_ends_map() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 100, 1));
    let h = header(TimePoint::new(ClockDomainId::local(8), 100), 100, 1);
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err((StreamError::DomainChanged { .. }, _))
    ));
}

#[test]
fn sc_30a_channel_count_change_ends_map() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    assert!(matches!(
        b.push(&header(t(100), 100, 2), DropCarry::default()),
        Err((StreamError::ChannelsChanged { from: 4, to: 2 }, _))
    ));
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channels, 4);
    for c in 0..4 {
        assert_eq!(map.valid[c], vec![Segment { start: t(0), len: 100 }], "channel {c}");
    }
}

#[test]
fn sc_31a_channel_gap_carries_a_cause() {
    let run = |flags: BlockFlags| {
        let mut b = builder(4, true);
        push(&mut b, header(t(0), 100, 4));
        let mut h = header(t(100), 100, 4);
        h.valid = ChannelMask(0b1011);
        h.flags = flags;
        push(&mut b, h);
        push(&mut b, header(t(200), 100, 4));
        b.finish(DropCarry::default()).channel_gaps
    };
    let gaps = run(BlockFlags::ALIGNMENT);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].channel, 2);
    assert_eq!((gaps[0].start, gaps[0].len), (t(100), 100));
    assert_eq!(gaps[0].cause, GapCause::Alignment);

    let gaps = run(BlockFlags::NONE);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].cause, GapCause::Stream);
}

#[test]
fn sc_31b_stream_gap_emits_no_channel_gaps() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(350), 100, 4);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(250);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert!(map.channel_gaps.is_empty(), "one overflow must not become four channel gaps");
}

#[test]
fn sc_31c_channel_that_never_returns() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(100), 100, 4);
    h.valid = ChannelMask(0b1011);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(map.channel_gaps[0].channel, 2);
    assert_eq!((map.channel_gaps[0].start, map.channel_gaps[0].len), (t(100), 100));
    assert_eq!(map.end, t(200));
}

#[test]
fn sc_31c_channel_lost_at_the_overflow() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    push(&mut b, header(t(100), 100, 4));
    let mut h = header(t(350), 100, 4);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(150);
    h.valid = ChannelMask(0b1011);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!((map.gaps[0].start, map.gaps[0].len), (t(200), 150));
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(map.channel_gaps[0].channel, 2);
    assert_eq!((map.channel_gaps[0].start, map.channel_gaps[0].len), (t(350), 100));
}

#[test]
fn sc_31d_break_across_a_stream_gap_is_split() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(100), 100, 4);
    h.valid = ChannelMask(0b1011);
    h.flags = BlockFlags::ALIGNMENT;
    push(&mut b, h);
    let mut h = header(t(350), 100, 4);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(150);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(
        (map.channel_gaps[0].channel, map.channel_gaps[0].start, map.channel_gaps[0].len),
        (2, t(100), 100),
        "the gap's 150 samples are not charged to channel 2"
    );
    assert_eq!(map.channel_gaps[0].cause, GapCause::Alignment);
}

#[test]
fn sc_31a_never_valid_channel_has_no_gap() {
    let mut b = builder(4, true);
    let mut h = header(t(0), 100, 4);
    h.valid = ChannelMask(0b0111);
    push(&mut b, h.clone());
    h.first_sample_time = t(100);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert!(map.channel_gaps.iter().all(|g| g.channel != 3), "never enabled is not a gap");
    assert!(map.valid[3].is_empty());
}

#[test]
fn sc_30_continuity_jump_overflow_is_time_error() {
    // The jump itself must overflow, not the block's own `end_time`: the second
    // header is placed so that `first + len` still fits and only `t - expected`
    // does not.
    let mut b = builder(1, true);
    push(&mut b, header(t(i64::MIN), 1, 1));
    let mut h = header(t(i64::MAX - 1), 1, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(1);
    assert!(h.end_time().is_ok(), "the block's own end is representable");
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err((StreamError::Time(TimeError::Overflow), _))
    ));

    // A header whose own end overflows is refused too, by the same error.
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 1, 1));
    let mut h = header(t(i64::MAX), 1, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(1);
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err((StreamError::Time(TimeError::Overflow), _))
    ));
}

#[test]
fn sc_30b_carry_merges_into_next_gap() {
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let carry = DropCarry {
        flags: BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        lost: Some(150),
        blocks: 1,
    };
    b.push(&header(t(350), 100, 1), carry).expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].cause, GapCause::OverflowRestart, "not a plain LinkDrop");
    assert_eq!(map.gaps[0].lost, Some(150));
    assert_eq!(map.gaps[0].link_dropped, 1);
}

#[test]
fn sc_30c_carry_survives_a_rejected_push() {
    let mut b = builder(4, false);
    push(&mut b, header(t(0), 100, 4));
    let carry = DropCarry { flags: BlockFlags::GAP_BEFORE, lost: Some(7), blocks: 2 };
    let (err, returned) = b.push(&header(t(100), 100, 2), carry).expect_err("channel count changed");
    assert!(matches!(err, StreamError::ChannelsChanged { .. }));
    assert_eq!(returned, carry, "the carry comes back to the consumer");

    // It lands in the OUTGOING map's finish, not the new builder's.
    let map = b.finish(returned);
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].link_dropped, 2);
    assert_eq!(map.gaps[0].lost, Some(7));
}

#[test]
fn sc_30c_trailing_carry_is_zero_extent() {
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let map = b.finish(DropCarry { flags: BlockFlags::NONE, lost: None, blocks: 3 });
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].len, 0);
    assert_eq!(map.gaps[0].lost, None);
    assert_eq!(map.gaps[0].link_dropped, 3);
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop);
    assert_eq!(map.end, t(100), "end does not move");
}

#[test]
fn sc_30c_trailing_carry_with_a_known_lost_stays_zero_extent() {
    // SC-30c: zero-extent, and `end` does not move, "because no sample after the
    // last delivered one is accounted for". A length of `lost` would claim samples
    // beyond the map's own end.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let map = b.finish(DropCarry {
        flags: BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        lost: Some(150),
        blocks: 1,
    });
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].len, 0, "zero-extent even when `lost` is known");
    assert_eq!(map.gaps[0].lost, Some(150), "the count is still carried");
    assert_eq!(map.gaps[0].cause, GapCause::OverflowRestart);
    assert_eq!(map.end, t(100), "end does not move");
}

#[test]
fn sc_27_min_lead_domain_is_checked_before_the_verdict() {
    // The same misuse must not be tolerated on the on-time path and rejected on the
    // late one (SC-27).
    let (reg, _root, sc) = sample_clock(0);
    let bad_lead = Duration::new(sc, 1_000);
    let now = TimePoint::new(sc, 0);
    for target in [TimePoint::new(sc, 40_000), TimePoint::new(sc, 1)] {
        assert!(matches!(
            LatePolicy::DropAndFlag.decide(&reg, target, now, bad_lead),
            Err(TimeError::DomainMismatch { .. })
        ));
    }
}

#[test]
fn sc_13_tm_13c_rate_change_is_a_new_domain_not_a_gap() {
    // A sample-rate change ends the map and the Sink starts a new builder (TM-13c).
    let (reg, root, a) = sample_clock(0);
    let bdom = reg.allocate_id();
    reg.register(ClockDomain::derived(bdom, root, rat(8, 1), 500)).expect("derived");
    let _ = ResourceId::parse("dev0/rx/0").expect("path");

    let mut b = ContinuityBuilder::new(a, 1, true);
    b.push(&header(TimePoint::new(a, 0), 50, 1), DropCarry::default()).expect("accepted");
    let (err, _) = b
        .push(&header(TimePoint::new(bdom, 0), 50, 1), DropCarry::default())
        .expect_err("a rate change ends the map");
    assert!(matches!(err, StreamError::DomainChanged { .. }));
}
