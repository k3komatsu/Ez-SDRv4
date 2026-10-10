//! Phase 1 tests for `02-stream-contract.md`. Each name begins with the rule it proves (OV-19).

mod support;

use std::sync::Arc;

use ezsdr_kernel::contract::{
    ContractRegistry, DataContract, DataContractId, PortRef, Scalar, standard_contracts,
};
use ezsdr_kernel::id::{ClockDomainId, DataLinkId, MemoryDomainId, NodeId, ResourceId};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BufferRef, BurstEnd, BurstOpen, BurstState, BurstStep,
    BurstTracker, ChannelGap, ChannelMask, ContinuityBuilder, DataLink, DataLinkDecl, Direction,
    DropCarry, Gap, GapCause, LateOutcome, LatePolicy, PublishOutcome, SampleBlock, Segment, StreamError,
    admit_burst_target, check_sink_link,
};
use ezsdr_kernel::time::{
    ClockDomain, ClockRegistry, Duration, EpochRef, Rational, TimeError, TimePoint,
};
use support::{CF32_BPS, GPU_MEM, MemLink, RetryingProducer, block, cf32, header, host_buffer, id};

const MCLK: u64 = 200_000_000;

fn rat(n: u64, d: u64) -> Rational {
    Rational::new(n, d).expect("valid rational")
}

fn arbitrary(s: &str) -> EpochRef {
    EpochRef::Arbitrary {
        set_by: s.to_owned(),
    }
}

/// A registry with a 200 MHz root and one 20 Msps SampleClock at the given origin.
fn sample_clock(origin: i64) -> (Arc<ClockRegistry>, ClockDomainId, ClockDomainId) {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test")))
        .expect("root");
    let sc = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(sc, root, rat(10, 1), origin))
        .expect("derived");
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
fn sc_02_scalar_equality_is_exact_across_int_and_float() {
    use ezsdr_kernel::contract::{DataContract, DataContractId, Scalar};
    // OV-15a accepts that `20` and `20.0` are one value with one canonical form and
    // one hash. It does **not** accept that two values with *different* canonical
    // forms compare equal: comparing through `as f64` made 2^53+1 equal to 2^53, so
    // SC-2's "registering a different definition under an existing id fails" took a
    // genuinely different definition for an idempotent re-registration and discarded
    // it with no diagnostic. Equality was also not transitive.
    assert_eq!(
        Scalar::Int(20),
        Scalar::Float(20.0),
        "OV-15a's accepted case"
    );
    assert_ne!(Scalar::Int(20), Scalar::Float(20.5));
    assert_ne!(
        Scalar::Int(9_007_199_254_740_993),
        Scalar::Float(9_007_199_254_740_992.0),
        "2^53+1 is not 2^53"
    );
    assert_eq!(
        Scalar::Int(9_007_199_254_740_992),
        Scalar::Float(9_007_199_254_740_992.0)
    );
    // Nothing panics at the edges.
    assert_ne!(Scalar::Int(1), Scalar::Float(1e30));
    assert_ne!(Scalar::Int(0), Scalar::Float(f64::NAN));
    assert_ne!(Scalar::Int(0), Scalar::Float(f64::INFINITY));

    // The rule, not a list of cases: two `Scalar`s are equal **iff** they are the
    // same number, compared exactly. Canonical form is *document* identity and is a
    // different question — `i64::MIN` and -2^63 are one number written two ways
    // (`-9223372036854775808` and `-9223372036854776000`), and
    // `1152921504606847000` and 2^60 are two numbers written one way, so deciding
    // equality by the form accepted a re-registration of a genuinely different
    // definition, which is the harm above moved from 2^53 to 2^60.
    let vectors: [(i64, f64, bool); 10] = [
        (0, 0.0, true),
        (20, 20.0, true),
        (20, 20.5, false),
        (-1, -1.0, true),
        (9_007_199_254_740_992, 9_007_199_254_740_992.0, true),
        (9_007_199_254_740_993, 9_007_199_254_740_992.0, false),
        (i64::MAX, 9_223_372_036_854_775_808.0, false),
        (i64::MIN, -9_223_372_036_854_775_808.0, true),
        (1_152_921_504_606_846_976, 1_152_921_504_606_846_976.0, true),
        (
            1_152_921_504_606_847_000,
            1_152_921_504_606_846_976.0,
            false,
        ),
    ];
    for (a, b, want) in vectors {
        let (si, sf) = (Scalar::Int(a), Scalar::Float(b));
        assert_eq!(si == sf, want, "Int({a}) vs Float({b})");
        // OV-15a's coincidence, which holds while |v| <= 2^53 and not above it.
        let one_form = ezsdr_kernel::hash::ContentHash::of(&si).ok()
            == ezsdr_kernel::hash::ContentHash::of(&sf).ok();
        if a.unsigned_abs() <= 9_007_199_254_740_992 {
            assert_eq!(want, one_form, "at or below 2^53 one value is one document");
        }
    }

    // SC-2 through the registry: the two definitions differ, so the second is refused
    // rather than silently discarded.
    let id = DataContractId::parse("test.exact").expect("id");
    let of = |v: Scalar| DataContract {
        id: id.clone(),
        attributes: [("full_scale".to_owned(), v)].into_iter().collect(),
        compatible_from: Default::default(),
    };
    let reg = ezsdr_kernel::contract::ContractRegistry::new();
    reg.register(of(Scalar::Int(9_007_199_254_740_993)))
        .expect("first");
    assert!(
        reg.register(of(Scalar::Float(9_007_199_254_740_992.0)))
            .is_err(),
        "a different definition under an existing id fails"
    );
    // The genuinely identical one is still a no-op.
    reg.register(of(Scalar::Int(9_007_199_254_740_993)))
        .expect("idempotent");
}

#[test]
fn sc_04_standard_contracts_fixture() {
    let reg = ContractRegistry::with_standard_contracts();
    let cf32 = reg.get(&cf32()).expect("cf32 registered");
    assert_eq!(
        cf32.attributes.get("bytes_per_sample"),
        Some(&Scalar::Int(8))
    );
    assert_eq!(cf32.attributes.get("full_scale"), Some(&Scalar::Float(1.0)));
    assert_eq!(
        cf32.attributes.get("layout"),
        Some(&Scalar::Str("planar".to_owned()))
    );
    assert!(cf32.compatible_from.is_empty());
    assert_eq!(cf32.bytes_per_sample(), Some(CF32_BPS));

    let sc16 = reg
        .get(&DataContractId::parse("ezsdr.stream.sc16").unwrap())
        .expect("sc16");
    assert_eq!(
        sc16.attributes.get("bytes_per_sample"),
        Some(&Scalar::Int(4))
    );
    assert_eq!(
        sc16.attributes.get("full_scale"),
        Some(&Scalar::Float(32767.0))
    );
    assert!(sc16.compatible_from.is_empty());
}

#[test]
fn sc_02_contract_registry_conflict() {
    let reg = ContractRegistry::new();
    let mut c = standard_contracts().into_iter().next().expect("cf32 first");
    assert!(reg.register(c.clone()).is_ok());
    assert!(
        reg.register(c.clone()).is_ok(),
        "an identical re-registration is a no-op"
    );
    c.attributes
        .insert("full_scale".to_owned(), Scalar::Float(2.0));
    assert!(reg.register(c).is_err());
}

#[test]
fn sc_03_contract_identity_or_compat() {
    let reg = ContractRegistry::with_standard_contracts();
    let cf32 = cf32();
    let sc16 = DataContractId::parse("ezsdr.stream.sc16").unwrap();
    assert!(reg.check_link(&cf32, &cf32).is_ok());
    assert!(matches!(
        reg.check_link(&cf32, &sc16),
        Err(StreamError::Incompatible { .. })
    ));

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
    assert!(matches!(
        reg.check_link(&x, &y),
        Err(StreamError::Incompatible { .. })
    ));
}

// ---------------------------------------------------------------- blocks

#[test]
fn sc_10_block_rejects_invalid_shape() {
    let buf = host_buffer(4, 2000);
    let bad = |h: BlockHeader| SampleBlock::new(h, buf, CF32_BPS);

    let mut h = header(t(0), 0, 1);
    assert!(
        matches!(bad(h), Err(StreamError::InvalidBlock { .. })),
        "len 0"
    );

    h = header(t(0), 10, 1);
    h.channels = 0;
    assert!(
        matches!(bad(h), Err(StreamError::InvalidBlock { .. })),
        "channels 0"
    );

    h = header(t(0), 10, 1);
    h.channels = 65;
    assert!(
        matches!(bad(h), Err(StreamError::InvalidBlock { .. })),
        "channels 65"
    );

    h = header(t(0), 10, 2);
    h.valid = ChannelMask::from_bits(0b100);
    assert!(
        matches!(bad(h), Err(StreamError::InvalidBlock { .. })),
        "valid bit above channels"
    );

    h = header(t(0), 10, 2);
    h.flags = BlockFlags::from_bits(0x0100);
    assert!(
        matches!(bad(h), Err(StreamError::InvalidBlock { .. })),
        "reserved flag bit"
    );
}

#[test]
fn sc_10a_block_rejects_undersized_buffer() {
    let h = header(t(0), 2000, 4);
    let small = BufferRef {
        memory_domain: support::HOST_MEM,
        handle: 1,
        len_bytes: 32_000,
    };
    assert!(matches!(
        SampleBlock::new(h.clone(), small, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
    let exact = BufferRef {
        len_bytes: 64_000,
        ..small
    };
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
        SampleBlock::new(tx.clone(), buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));

    // SC-16a: LATE is transmit-only.
    tx.flags = BlockFlags::LATE;
    SampleBlock::new(tx, buf, CF32_BPS).expect("a transmit block may carry LATE");
    let mut rx = header(t(0), 10, 1);
    rx.flags = BlockFlags::LATE;
    assert!(matches!(
        SampleBlock::new(rx, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
}

#[test]
fn sc_10_block_partial_channels_derived() {
    let buf = host_buffer(2, 10);
    let mut h = header(t(0), 10, 2);
    h.valid = ChannelMask::from_bits(0b01);
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
    assert!(matches!(
        SampleBlock::new(h, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));

    // ALIGNMENT qualifies a gap, so it implies GAP_BEFORE.
    let mut h = header(t(0), 10, 1);
    h.flags = BlockFlags::ALIGNMENT;
    assert!(matches!(
        SampleBlock::new(h.clone(), buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
    h.flags = BlockFlags::ALIGNMENT | BlockFlags::GAP_BEFORE;
    SampleBlock::new(h, buf, CF32_BPS).expect("an alignment gap");

    let mut h = header(t(0), 10, 1);
    h.lost = Some(5);
    assert!(matches!(
        SampleBlock::new(h, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));

    let mut h = header(t(0), 10, 1);
    h.direction = Direction::Tx;
    h.flags = BlockFlags::START_OF_BURST | BlockFlags::GAP_BEFORE;
    assert!(matches!(
        SampleBlock::new(h, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));

    let mut h = header(t(0), 10, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(0);
    assert!(matches!(
        SampleBlock::new(h, buf, CF32_BPS),
        Err(StreamError::InvalidBlock { .. })
    ));
}

#[test]
fn sc_17_flag_bit_positions_are_fixed_by_the_document() {
    // Decision S17 rejects "an ordering that exists only in the Rust source", and
    // SC-17's purpose — an Executor propagating flags unchanged one-to-one — is a
    // numeric contract.
    assert_eq!(BlockFlags::GAP_BEFORE, BlockFlags::from_bits(0x0001));
    assert_eq!(BlockFlags::SEQ_DISCONTINUITY, BlockFlags::from_bits(0x0002));
    assert_eq!(BlockFlags::RESTARTED, BlockFlags::from_bits(0x0004));
    assert_eq!(BlockFlags::LATE, BlockFlags::from_bits(0x0008));
    assert_eq!(BlockFlags::PARTIAL_CHANNELS, BlockFlags::from_bits(0x0010));
    assert_eq!(BlockFlags::START_OF_BURST, BlockFlags::from_bits(0x0020));
    assert_eq!(BlockFlags::END_OF_BURST, BlockFlags::from_bits(0x0040));
    assert_eq!(BlockFlags::ALIGNMENT, BlockFlags::from_bits(0x0080));
    assert_eq!(
        BlockFlags::RESERVED,
        BlockFlags::from_bits(0xFF00),
        "bits 8-15 are reserved"
    );
    assert_eq!(BlockFlags::NONE, BlockFlags::from_bits(0));
    assert_eq!(BlockFlags::GAP_BEFORE.bits(), 0x0001);
    assert_eq!(ChannelMask::from_bits(0b101).bits(), 0b101);
}

#[test]
fn sc_06_memory_domain_id_is_node_qualified() {
    let id = MemoryDomainId::local(7);
    assert_eq!(id.node, NodeId::LOCAL);
    assert_eq!(id.local, 7);
    assert_eq!(id.to_string(), "local:mem#7");
}

#[test]
fn sc_19_link_declares_a_policy_and_a_capacity() {
    // There is no default policy, and both fields are mandatory in the type.
    let decl = DataLinkDecl {
        id: DataLinkId::local(1),
        from: PortRef {
            component: id("a"),
            port: id("out"),
        },
        to: PortRef {
            component: id("b"),
            port: id("in"),
        },
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
    assert!(
        !BackPressure::Block.is_drop_class(),
        "a drop-class link never returns Full"
    );
    assert_eq!(
        MemLink::new(BackPressure::DropNewest, 4).policy(),
        BackPressure::DropNewest
    );
}

#[test]
fn sc_11_block_fanout_shares_reference() {
    let b = block(header(t(0), 100, 1));
    let a = MemLink::new(BackPressure::Block, 4);
    let c = MemLink::new(BackPressure::Block, 4);
    assert_eq!(a.publish(b.clone()), PublishOutcome::Accepted);
    assert_eq!(c.publish(b.clone()), PublishOutcome::Accepted);
    let ra = a.receive().expect("queued").0;
    let rc = c.receive().expect("queued").0;
    assert!(
        Arc::ptr_eq(&ra, &rc),
        "fan-out shares one reference; nothing is copied"
    );
    assert_eq!(Arc::strong_count(&b), 3);
}

#[test]
fn sc_08_host_bytes_only_for_a_block_that_carries_them() {
    // SC-8 (KA-3): a block built with `new` carries no host bytes.
    let h = header(t(0), 10, 1);
    let gpu = BufferRef {
        memory_domain: GPU_MEM,
        handle: 0xdead,
        len_bytes: 1 << 20,
    };
    let gpu = SampleBlock::new(h, gpu, CF32_BPS).expect("valid");
    assert!(gpu.host_bytes().is_none());
    // One built with `new_host` returns exactly the bytes it was given.
    let bytes: Arc<[u8]> = vec![7u8; 4 * 1000 * 8].into();
    let host = SampleBlock::new_host(header(t(0), 1000, 4), support::HOST_MEM, bytes, CF32_BPS)
        .expect("valid");
    assert_eq!(host.host_bytes().map(|b| b.len()), Some(32_000));
    assert_eq!(host.buffer().len_bytes, 32_000);
    // SC-10a refuses too few bytes exactly as `new` does.
    let short: Arc<[u8]> = vec![0u8; 31_999].into();
    assert!(
        SampleBlock::new_host(header(t(0), 1000, 4), support::HOST_MEM, short, CF32_BPS).is_err()
    );
}

// ---------------------------------------------------------------- links

#[test]
fn sc_20_link_block_policy_full() {
    let link = MemLink::new(BackPressure::Block, 2);
    assert_eq!(
        link.publish(block(header(t(0), 10, 1))),
        PublishOutcome::Accepted
    );
    assert_eq!(
        link.publish(block(header(t(10), 10, 1))),
        PublishOutcome::Accepted
    );
    assert_eq!(
        link.publish(block(header(t(20), 10, 1))),
        PublishOutcome::Full
    );
    assert_eq!(link.drops(), 0, "nothing is ever dropped under Block");
    link.receive().expect("queued");
    assert_eq!(
        link.publish(block(header(t(20), 10, 1))),
        PublishOutcome::Accepted
    );
}

#[test]
fn sc_20_link_drop_oldest() {
    let link = MemLink::new(BackPressure::DropOldest, 2);
    // The outcome of each publish says what happened, and is never `Full`.
    let outcomes: Vec<_> = (0..3)
        .map(|i| link.publish(block(header(t(i * 10), 10, 1))))
        .collect();
    assert_eq!(
        outcomes,
        [
            PublishOutcome::Accepted,
            PublishOutcome::Accepted,
            PublishOutcome::DroppedOldest
        ]
    );
    assert_eq!(link.receive().expect("queued").0.first_sample_time(), t(10));
    assert_eq!(link.receive().expect("queued").0.first_sample_time(), t(20));
    assert_eq!(link.drops(), 1);
}

#[test]
fn sc_20_link_drop_newest() {
    let link = MemLink::new(BackPressure::DropNewest, 2);
    let outcomes: Vec<_> = (0..3)
        .map(|i| link.publish(block(header(t(i * 10), 10, 1))))
        .collect();
    assert_eq!(
        outcomes,
        [
            PublishOutcome::Accepted,
            PublishOutcome::Accepted,
            PublishOutcome::DroppedNewest
        ]
    );
    assert_eq!(link.receive().expect("queued").0.first_sample_time(), t(0));
    assert_eq!(link.receive().expect("queued").0.first_sample_time(), t(10));
    assert_eq!(link.drops(), 1);
}

#[test]
fn sc_20a_full_is_not_a_silent_drop() {
    let link = MemLink::new(BackPressure::Block, 1);
    let mut producer = RetryingProducer::new();
    assert_eq!(
        producer.offer(&link, block(header(t(0), 10, 1))),
        PublishOutcome::Accepted
    );
    assert_eq!(
        producer.offer(&link, block(header(t(10), 10, 1))),
        PublishOutcome::Full
    );
    // The producer still owns it; it must not discard it.
    link.receive().expect("the first block");
    assert_eq!(producer.retry(&link), Some(PublishOutcome::Accepted));
    assert_eq!(
        link.receive()
            .expect("the retried block").0
            .first_sample_time(),
        t(10)
    );
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

    // The carry comes with the block the drop preceded.
    let (_, carry) = link.receive().expect("the block the first drop kept");
    assert!(carry.flags.contains(BlockFlags::GAP_BEFORE));
    assert!(carry.flags.contains(BlockFlags::RESTARTED));
    assert_eq!(carry.lost, Some(150));
    assert_eq!(carry.blocks, 1);
    assert!(link.take_drop_carry().is_empty(), "no carry trails the last block");
    assert_eq!(link.drops(), 1, "the drop counter never resets");

    // Two real drops into one carry: the flags are the union, the lost counts the sum.
    let mut first = header(t(300), 10, 1);
    first.flags = BlockFlags::GAP_BEFORE;
    first.lost = Some(7);
    let mut second = header(t(400), 10, 1);
    // `lost` travels with a gap (SC-13), so the second drop carries one too.
    second.flags = BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY;
    second.lost = Some(5);
    for h in [first, second, header(t(500), 10, 1)] {
        link.publish(block(h));
    }
    let (_, carry) = link.receive().expect("the last block");
    assert_eq!(
        carry.flags,
        BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY
    );
    assert_eq!(carry.lost, Some(12));
    assert_eq!(carry.blocks, 2);
    assert_eq!(link.drops(), 3);
}

#[test]
fn sc_21_sink_links_must_be_drop_class() {
    let decl = |policy| DataLinkDecl {
        id: DataLinkId::local(3),
        from: PortRef {
            component: id("rx"),
            port: id("out"),
        },
        to: PortRef {
            component: id("rec"),
            port: id("in"),
        },
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
    assert_eq!(
        tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open()),
        Ok(BurstStep::Started)
    );
    assert_eq!(
        tr.on_block(&tx(1100, 50, BlockFlags::NONE), None),
        Ok(BurstStep::Continued)
    );
    let step = tr
        .on_block(&tx(1150, 10, BlockFlags::END_OF_BURST), None)
        .expect("ends");
    let BurstStep::Ended { record } = step else {
        panic!("expected Ended, got {step:?}")
    };
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
        .on_block(
            &tx(
                1000,
                100,
                BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST,
            ),
            open(),
        )
        .expect("one-block burst");
    let BurstStep::Ended { record } = step else {
        panic!("expected Ended")
    };
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
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1300, 100, BlockFlags::NONE), None)
        .expect("recovers");
    let BurstStep::Discontinuity {
        expected,
        got,
        closed,
        ..
    } = step
    else {
        panic!("expected a discontinuity")
    };
    assert_eq!((expected, got), (t(1100), t(1300)));
    assert_eq!(closed.end, BurstEnd::Discontinuity);
    assert_eq!(closed.samples, 100);
    assert!(matches!(tr.state(), BurstState::InBurst { target, .. } if target == t(1300)));
}

#[test]
fn sc_24_burst_backward_time_is_discontinuity() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1050, 100, BlockFlags::NONE), None)
        .expect("recovers");
    assert!(matches!(step, BurstStep::Discontinuity { got, .. } if got == t(1050)));
}

#[test]
fn sc_24_burst_sob_inside_burst() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1100, 100, BlockFlags::START_OF_BURST), open())
        .expect("recovers");
    let BurstStep::Discontinuity {
        expected,
        got,
        closed,
        ..
    } = step
    else {
        panic!("expected a discontinuity")
    };
    assert_eq!((expected, got), (t(1100), t(1100)));
    assert_eq!(closed.samples, 100);
    assert!(matches!(tr.state(), BurstState::InBurst { target, .. } if target == t(1100)));
}

#[test]
fn sc_25_burst_continuous_until_stop() {
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(0, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
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
    let open = Some(BurstOpen {
        waveform_len: Some(1000),
        ..BurstOpen::default()
    });
    tr.on_block(&tx(1000, 300, BlockFlags::START_OF_BURST), open)
        .expect("start");
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
    let open = Some(BurstOpen {
        waveform_len: Some(1000),
        ..BurstOpen::default()
    });
    tr.on_block(&tx(1000, 300, BlockFlags::START_OF_BURST), open)
        .expect("start");
    for (at, len) in [(1300, 300), (1600, 300), (1900, 100)] {
        tr.on_block(&tx(at, len, BlockFlags::NONE), None)
            .expect("contiguous");
    }
    let step = tr
        .on_block(&tx(2001, 300, BlockFlags::NONE), None)
        .expect("recovers");
    assert!(
        matches!(step, BurstStep::Discontinuity { expected, got, .. } if expected == t(2000) && got == t(2001))
    );
}

#[test]
fn sc_29a_wraps_counted_from_burst_open() {
    let mut tr = BurstTracker::new(dom());
    let open = Some(BurstOpen {
        waveform_len: Some(1000),
        ..BurstOpen::default()
    });
    tr.on_block(&tx(0, 500, BlockFlags::START_OF_BURST), open)
        .expect("start");
    for i in 1..7 {
        tr.on_block(&tx(i * 500, 500, BlockFlags::NONE), None)
            .expect("contiguous");
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
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open)
        .expect("start");
    tr.set_actual_start(t(1003));
    let step = tr
        .on_block(&tx(1100, 100, BlockFlags::END_OF_BURST), None)
        .expect("ends");
    let BurstStep::Ended { record } = step else {
        panic!("expected Ended")
    };
    assert_eq!(record.late_by, Some(late_by));
    assert_eq!(record.actual_start, Some(t(1003)));
    assert_eq!(record.wraps, 1);
}

#[test]
fn sc_24_a_discontinuity_that_also_ends_reports_both_records() {
    // A block whose time jumped **and** which carried END_OF_BURST does two things:
    // it closes the open burst and opens-and-ends another. The return value used to
    // name only the first, so a caller tracking state from it believed a burst was
    // open while `state()` said `Idle` (finding D36).
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1300, 100, BlockFlags::END_OF_BURST), None)
        .expect("jumps and ends");
    let BurstStep::Discontinuity {
        expected,
        got,
        closed,
        then_ended,
    } = step
    else {
        panic!("expected a discontinuity")
    };
    assert_eq!((expected, got), (t(1100), t(1300)));
    assert_eq!(closed.end, BurstEnd::Discontinuity);
    let ended = then_ended.expect("the same block ended the burst it opened");
    assert_eq!(ended.end, BurstEnd::Eob);
    assert_eq!(ended.target, t(1300));
    // And the return value agrees with the tracker.
    assert!(matches!(tr.state(), BurstState::Idle));

    // A plain discontinuity still reports no second record.
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1300, 100, BlockFlags::NONE), None)
        .expect("jumps");
    assert!(matches!(
        step,
        BurstStep::Discontinuity {
            then_ended: None,
            ..
        }
    ));
}

#[test]
fn sc_29a_set_late_carries_a_discontinuity_opened_burst() {
    // SC-24a requires the late policy to be evaluated for the burst a discontinuity
    // opened, and that burst carries no START_OF_BURST and so no BurstOpen. Without
    // `set_late` its `late_by` would be permanently None — the one case SC-24a is
    // about (finding D8).
    let mut tr = BurstTracker::new(dom());
    let late_by = Duration::new(ClockDomainId::HOST_MONOTONIC, 7);
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    let step = tr
        .on_block(&tx(1300, 100, BlockFlags::NONE), None)
        .expect("recovers");
    assert!(
        matches!(step, BurstStep::Discontinuity { .. }),
        "the new burst took no BurstOpen"
    );

    tr.set_late(LateOutcome::SendAsap { late_by });
    let step = tr
        .on_block(&tx(1400, 100, BlockFlags::END_OF_BURST), None)
        .expect("ends");
    let BurstStep::Ended { record } = step else {
        panic!("expected Ended")
    };
    assert_eq!(
        record.late_by,
        Some(late_by),
        "the outcome reached the record"
    );
    assert_eq!(
        record.target,
        t(1300),
        "and it is the discontinuity-opened burst's record"
    );

    // OnTime clears it rather than recording a zero, so a caller cannot turn an
    // on-time burst into a late one by reporting the evaluation it made (SC-27).
    let mut tr = BurstTracker::new(dom());
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
    tr.on_block(&tx(1300, 100, BlockFlags::NONE), None)
        .expect("recovers");
    tr.set_late(LateOutcome::OnTime {});
    let step = tr
        .on_block(&tx(1400, 100, BlockFlags::END_OF_BURST), None)
        .expect("ends");
    let BurstStep::Ended { record } = step else {
        panic!("expected Ended")
    };
    assert_eq!(record.late_by, None);
}

#[test]
fn sc_23a_requested_target_reaches_the_record() {
    // SC-23a: "both times are recorded: the BurstRecord keeps `requested_target`
    // alongside the applied `target`". The admission result travels on the block
    // that opens the burst, like the other fields no header can carry (SC-29a).
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test")))
        .expect("root");
    let txdom = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(txdom, root, rat(8, 1), 0))
        .expect("derived");

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
    tr.on_block(&tx(1000, 100, BlockFlags::START_OF_BURST), open())
        .expect("start");
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
    let root = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test")))
        .expect("root");
    let tx = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(tx, root, rat(8, 1), 0))
        .expect("derived");

    let got = admit_burst_target(&reg, TimePoint::new(root, 70), tx).expect("admitted");
    assert_eq!(
        got.target,
        TimePoint::new(tx, 9),
        "70/8 is between samples 8 and 9"
    );
    assert_eq!(got.requested_target, Some(TimePoint::new(root, 70)));

    let got = admit_burst_target(&reg, TimePoint::new(root, 40), tx).expect("admitted");
    assert_eq!(got.target, TimePoint::new(tx, 5));
    assert_eq!(got.requested_target, None, "nothing was advanced");
}

#[test]
fn sc_23a_reactive_target_across_disjoint_grids() {
    let reg = Arc::new(ClockRegistry::new());
    let root = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(root, rat(MCLK, 1), arbitrary("test")))
        .expect("root");
    // TM-13b makes each stream's origin its own first sample, so the grids are disjoint.
    let rx = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(rx, root, rat(10, 1), 1_000_000_000))
        .expect("rx");
    let tx = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(tx, root, rat(10, 1), 1_000_000_003))
        .expect("tx");

    // The natural reactive computation: rx_time + turnaround, in the receive domain.
    let target = TimePoint::new(rx, 100);
    let got = admit_burst_target(&reg, target, tx).expect("admitted, not refused");
    assert_eq!(
        got.target,
        TimePoint::new(tx, 100),
        "advanced by at most one sample"
    );
    assert_eq!(
        got.requested_target,
        Some(target),
        "both times are recorded"
    );
}

#[test]
fn sc_23b_tx_target_unrelated_domain_refused() {
    let (reg, _root, sc) = sample_clock(0);
    let other = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(other, rat(MCLK, 1), arbitrary("other")))
        .expect("root");
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
        Ok(LateOutcome::OnTime {})
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
        LatePolicy::DropAndFlag.decide(
            &reg,
            TimePoint::new(sc, 10),
            TimePoint::new(root, 0),
            min_lead
        ),
        Err(TimeError::DomainMismatch { .. })
    ));
}

#[test]
fn sc_27_min_lead_cross_multiplied() {
    let reg = ClockRegistry::new();
    let host = ClockDomainId::HOST_MONOTONIC;
    let min_lead = Duration::new(host, 1_000_000); // 1 ms
    let hz3 = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(hz3, rat(3, 1), arbitrary("test")))
        .expect("root");
    let lte = reg.allocate_id().unwrap();
    reg.register(ClockDomain::root(
        lte,
        rat(30_720_000, 1),
        arbitrary("test"),
    ))
    .expect("root");

    // 3 Hz: one tick is 333 ms, comfortably over; zero ticks is under. A
    // rescale-and-round implementation reports the first case 333 times too strict.
    let p = LatePolicy::DropAndFlag;
    assert_eq!(
        p.decide(
            &reg,
            TimePoint::new(hz3, 1),
            TimePoint::new(hz3, 0),
            min_lead
        ),
        Ok(LateOutcome::OnTime {})
    );
    assert_eq!(
        p.decide(
            &reg,
            TimePoint::new(hz3, 0),
            TimePoint::new(hz3, 0),
            min_lead
        ),
        Ok(LateOutcome::Drop {
            late_by: Duration::new(host, 1_000_000)
        })
    );
    // 30.72 Msps: 1 ms is exactly 30 720 ticks, and one tick is 3125/96 ns.
    assert_eq!(
        p.decide(
            &reg,
            TimePoint::new(lte, 30_720),
            TimePoint::new(lte, 0),
            min_lead
        ),
        Ok(LateOutcome::OnTime {})
    );
    assert_eq!(
        p.decide(
            &reg,
            TimePoint::new(lte, 30_719),
            TimePoint::new(lte, 0),
            min_lead
        ),
        Ok(LateOutcome::Drop {
            late_by: Duration::new(host, 33)
        })
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
    assert_eq!(
        map.valid[0],
        vec![Segment {
            start: t(0),
            len: 200
        }]
    );
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
    assert_eq!(map.gaps[0].cause, GapCause::Stream {});
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
        Err(StreamError::GapFlagWithoutJump)
    ));
}

#[test]
fn sc_30_continuity_jump_without_flag() {
    let mut lossless = builder(1, true);
    push(&mut lossless, header(t(0), 100, 1));
    assert!(matches!(
        lossless.push(&header(t(500), 100, 1), DropCarry::default()),
        Err(StreamError::JumpWithoutGapFlag)
    ));

    let mut lossy = builder(1, false);
    push(&mut lossy, header(t(0), 100, 1));
    push(&mut lossy, header(t(500), 100, 1));
    let map = lossy.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!((map.gaps[0].start, map.gaps[0].len), (t(100), 400));
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop {});
}

#[test]
fn sc_12_continuity_overlap_is_error() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 200, 1));
    assert!(matches!(
        b.push(&header(t(150), 100, 1), DropCarry::default()),
        Err(StreamError::TimeOverlap { .. })
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
        GapCause::OverflowRestart {}
    );
    assert_eq!(
        case(
            BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY,
            Some(200)
        ),
        GapCause::SequenceError {}
    );
    assert_eq!(case(BlockFlags::GAP_BEFORE, None), GapCause::Unknown {});
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
    h.valid = ChannelMask::from_bits(0b01);
    push(&mut b, h);
    push(&mut b, header(t(200), 100, 2));
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.valid[0],
        vec![Segment {
            start: t(0),
            len: 300
        }]
    );
    assert_eq!(
        map.valid[1],
        vec![
            Segment {
                start: t(0),
                len: 100
            },
            Segment {
                start: t(200),
                len: 100
            }
        ]
    );
}

#[test]
fn sc_30_continuity_domain_change_ends_map() {
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 100, 1));
    let h = header(TimePoint::new(ClockDomainId::local(8), 100), 100, 1);
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err(StreamError::DomainChanged { .. })
    ));
}

#[test]
fn sc_30a_channel_count_change_ends_map() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    assert!(matches!(
        b.push(&header(t(100), 100, 2), DropCarry::default()),
        Err(StreamError::ChannelsChanged { from: 4, to: 2 })
    ));
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channels, 4);
    for c in 0..4 {
        assert_eq!(
            map.valid[c],
            vec![Segment {
                start: t(0),
                len: 100
            }],
            "channel {c}"
        );
    }
}

#[test]
fn sc_31d_a_channel_break_is_a_channel_gap() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(100), 100, 4);
    h.valid = ChannelMask::from_bits(0b1011);
    push(&mut b, h);
    push(&mut b, header(t(200), 100, 4));
    let gaps = b.finish(DropCarry::default()).channel_gaps;
    assert_eq!(gaps, [ChannelGap { channel: 2, start: t(100), len: 100 }]);
}

#[test]
fn sc_31_alignment_qualifies_a_stream_gap() {
    // ALIGNMENT is a stream-gap qualifier like RESTARTED and SEQ_DISCONTINUITY: UHD
    // discards whole packets on every channel to keep them aligned (UR-19).
    let mut b = builder(2, true);
    push(&mut b, header(t(0), 100, 2));
    let mut h = header(t(250), 100, 2);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::ALIGNMENT;
    h.lost = Some(150);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].cause, GapCause::Alignment {});
    assert_eq!((map.gaps[0].start, map.gaps[0].len, map.gaps[0].lost), (t(100), 150, Some(150)));
    assert!(map.channel_gaps.is_empty(), "a stream gap, not a per-channel break");

    // Precedence: RESTARTED over SEQ_DISCONTINUITY over ALIGNMENT (UR-19 keeps every
    // report kind before one block).
    for (qualifier, cause) in [
        (BlockFlags::SEQ_DISCONTINUITY, GapCause::SequenceError {}),
        (BlockFlags::RESTARTED, GapCause::OverflowRestart {}),
    ] {
        let mut b = builder(2, true);
        push(&mut b, header(t(0), 100, 2));
        let mut h = header(t(250), 100, 2);
        h.flags = BlockFlags::GAP_BEFORE | BlockFlags::ALIGNMENT | qualifier;
        h.lost = Some(150);
        push(&mut b, h);
        assert_eq!(b.finish(DropCarry::default()).gaps[0].cause, cause);
    }
}

#[test]
fn sc_13_a_first_block_gap_is_recorded() {
    // A first block's GAP_BEFORE counts its `lost` back from its first sample, as a
    // late receive start (RM-25) has it: the map begins at the gap's start.
    let mut b = builder(1, true);
    let mut h = header(t(40), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(40);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps,
        [Gap { start: t(0), len: 40, lost: Some(40), cause: GapCause::Stream {}, link_dropped: 0 }]
    );
    assert_eq!((map.first, map.end), (t(0), t(140)));
    assert_eq!(map.valid[0], [Segment { start: t(40), len: 100 }]);

    // With `lost` unknown, the gap is zero-extent at the block, cause Unknown.
    let mut b = builder(1, true);
    let mut h = header(t(40), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps,
        [Gap { start: t(40), len: 0, lost: None, cause: GapCause::Unknown {}, link_dropped: 0 }]
    );
    assert_eq!(map.first, t(40));
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
    assert!(
        map.channel_gaps.is_empty(),
        "one overflow must not become four channel gaps"
    );
}

#[test]
fn sc_31c_channel_that_never_returns() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(100), 100, 4);
    h.valid = ChannelMask::from_bits(0b1011);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(map.channel_gaps[0].channel, 2);
    assert_eq!(
        (map.channel_gaps[0].start, map.channel_gaps[0].len),
        (t(100), 100)
    );
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
    h.valid = ChannelMask::from_bits(0b1011);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!((map.gaps[0].start, map.gaps[0].len), (t(200), 150));
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(map.channel_gaps[0].channel, 2);
    assert_eq!(
        (map.channel_gaps[0].start, map.channel_gaps[0].len),
        (t(350), 100)
    );
}

#[test]
fn sc_31d_break_across_a_stream_gap_is_split() {
    let mut b = builder(4, true);
    push(&mut b, header(t(0), 100, 4));
    let mut h = header(t(100), 100, 4);
    h.valid = ChannelMask::from_bits(0b1011);
    push(&mut b, h);
    let mut h = header(t(350), 100, 4);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(150);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(map.channel_gaps.len(), 1);
    assert_eq!(
        (
            map.channel_gaps[0].channel,
            map.channel_gaps[0].start,
            map.channel_gaps[0].len
        ),
        (2, t(100), 100),
        "the gap's 150 samples are not charged to channel 2"
    );
}

#[test]
fn sc_31c_never_valid_channel_has_no_gap() {
    let mut b = builder(4, true);
    let mut h = header(t(0), 100, 4);
    h.valid = ChannelMask::from_bits(0b0111);
    push(&mut b, h.clone());
    h.first_sample_time = t(100);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert!(
        map.channel_gaps.iter().all(|g| g.channel != 3),
        "never enabled is not a gap"
    );
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
        Err(StreamError::Time(TimeError::Overflow))
    ));

    // A header whose own end overflows is refused too, by the same error.
    let mut b = builder(1, true);
    push(&mut b, header(t(0), 1, 1));
    let mut h = header(t(i64::MAX), 1, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(1);
    assert!(matches!(
        b.push(&h, DropCarry::default()),
        Err(StreamError::Time(TimeError::Overflow))
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
    assert_eq!(
        map.gaps[0].cause,
        GapCause::OverflowRestart {},
        "not a plain LinkDrop"
    );
    assert_eq!(map.gaps[0].lost, Some(150));
    assert_eq!(map.gaps[0].link_dropped, 1);
}

#[test]
fn sc_30c_carry_survives_a_rejected_push() {
    let mut b = builder(4, false);
    push(&mut b, header(t(0), 100, 4));
    let carry = DropCarry {
        flags: BlockFlags::GAP_BEFORE,
        lost: Some(7),
        blocks: 2,
    };
    let err = b
        .push(&header(t(100), 100, 2), carry)
        .expect_err("channel count changed");
    assert!(matches!(err, StreamError::ChannelsChanged { .. }));

    // The builder kept it, so it lands in the OUTGOING map's finish.
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].link_dropped, 2);
    assert_eq!(map.gaps[0].lost, Some(7));
}

#[test]
fn sc_30c_trailing_carry_is_zero_extent() {
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let map = b.finish(DropCarry {
        flags: BlockFlags::NONE,
        lost: None,
        blocks: 3,
    });
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].start, t(100));
    assert_eq!(map.gaps[0].len, 0);
    assert_eq!(map.gaps[0].lost, None);
    assert_eq!(map.gaps[0].link_dropped, 3);
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop {});
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
    assert_eq!(map.gaps[0].cause, GapCause::OverflowRestart {});
    assert_eq!(map.end, t(100), "end does not move");

    // Without a qualifier, the known count is what makes the cause `Stream`.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let map = b.finish(DropCarry {
        flags: BlockFlags::GAP_BEFORE,
        lost: Some(150),
        blocks: 1,
    });
    assert_eq!(map.gaps[0].cause, GapCause::Stream {});
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
    let bdom = reg.allocate_id().unwrap();
    reg.register(ClockDomain::derived(bdom, root, rat(8, 1), 500))
        .expect("derived");
    let _ = ResourceId::parse("dev0/rx/0").expect("path");

    let mut b = ContinuityBuilder::new(a, 1, true);
    b.push(&header(TimePoint::new(a, 0), 50, 1), DropCarry::default())
        .expect("accepted");
    let err = b
        .push(
            &header(TimePoint::new(bdom, 0), 50, 1),
            DropCarry::default(),
        )
        .expect_err("a rate change ends the map");
    assert!(matches!(err, StreamError::DomainChanged { .. }));
}

#[test]
fn sc_30b_a_carry_on_a_contiguous_block_is_not_discarded() {
    // SC-30b's obligation is that the builder "records the dropped block count in the
    // gap's `link_dropped`", and the rule says in its own words why the counts merge:
    // a rule that dropped the count would "discard exactly the number it was written
    // to preserve". Under `DropNewest` the refused block is newer than everything
    // queued, so the block delivered after the drop is **contiguous** — the consumer
    // reads a non-empty carry and hands it to a push that opens no gap. That count had
    // nowhere to go and was dropped, so the next real jump was written as a `LinkDrop`
    // with `link_dropped: 0`: the link dropped blocks and the map said none.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    // Two blocks refused, and the next delivered one is contiguous at 100.
    b.push(
        &header(t(100), 100, 1),
        DropCarry {
            blocks: 2,
            ..DropCarry::default()
        },
    )
    .expect("a contiguous block is accepted");
    // A later jump, with one more block refused at that point.
    b.push(
        &header(t(400), 100, 1),
        DropCarry {
            blocks: 1,
            ..DropCarry::default()
        },
    )
    .expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1, "one jump, one gap");
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop {});
    assert_eq!(
        map.gaps[0].link_dropped, 3,
        "two carried forward plus the one at the jump"
    );
}

#[test]
fn sc_30b_a_carry_held_across_a_contiguous_block_reaches_the_trailing_gap() {
    // Holding the count for "the next Gap" is only half of SC-30b if the stream ends
    // before there is one. Under `DropNewest` the last drop is followed by a
    // contiguous block, so a map that ends there had no Gap to record it in and the
    // count was discarded at `finish` — the same loss the mid-stream fix closed,
    // moved to the end of the stream.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    b.push(
        &header(t(100), 100, 1),
        DropCarry {
            blocks: 2,
            ..DropCarry::default()
        },
    )
    .expect("a contiguous block is accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps.len(),
        1,
        "the drop is recorded even with no later jump"
    );
    assert_eq!(map.gaps[0].cause, GapCause::LinkDrop {});
    assert_eq!(
        map.gaps[0].len, 0,
        "SC-30c: zero-extent, no sample is accounted for"
    );
    assert_eq!(map.gaps[0].link_dropped, 2);

    // And the trailing carry sums with what was held, rather than replacing it.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    b.push(
        &header(t(100), 100, 1),
        DropCarry {
            blocks: 2,
            ..DropCarry::default()
        },
    )
    .expect("accepted");
    let map = b.finish(DropCarry {
        blocks: 1,
        ..DropCarry::default()
    });
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].link_dropped, 3);
}

#[test]
fn sc_30b_a_held_carry_keeps_its_flags_and_lost_count() {
    // SC-30b gives the carry three jobs: its flags "derive a cause", its `lost`
    // counts "sum", and its block count reaches `link_dropped`. A contiguous push has
    // nowhere to put any of them, so holding only the third wrote the next Gap as a
    // plain `LinkDrop` with no `lost` — which positively attributes a device overflow
    // to host-side link loss, the failure `DropCarry` and SC-20b exist to prevent.
    let held = DropCarry {
        flags: BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED,
        lost: Some(150),
        blocks: 1,
    };
    // The delivered block is contiguous and correct; the carried `GAP_BEFORE`
    // describes a block the **link** dropped, so the push is well formed.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    b.push(&header(t(100), 100, 1), held)
        .expect("a contiguous block is accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(
        map.gaps[0].cause,
        GapCause::OverflowRestart {},
        "the carried flags derive it"
    );
    assert_eq!(
        map.gaps[0].lost,
        Some(150),
        "and the carried count is not discarded"
    );
    assert_eq!(map.gaps[0].link_dropped, 1);

    // Same carry, and a later jump: it reaches that Gap instead of the trailing one.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    b.push(&header(t(100), 100, 1), held).expect("accepted");
    b.push(&header(t(400), 100, 1), DropCarry::default())
        .expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].cause, GapCause::OverflowRestart {});
    assert_eq!(map.gaps[0].lost, Some(150));
    assert_eq!(map.gaps[0].link_dropped, 1);

    // A held carry and the jump's own carry: the counts sum (SC-20b).
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    b.push(&header(t(100), 100, 1), held).expect("accepted");
    b.push(
        &header(t(400), 100, 1),
        DropCarry {
            flags: BlockFlags::GAP_BEFORE,
            lost: Some(12),
            blocks: 1,
        },
    )
    .expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps[0].lost, Some(162));
    assert_eq!(map.gaps[0].link_dropped, 2);
}

#[test]
fn sc_31_mixed_requires_that_no_carry_explains_the_shortfall() {
    // SC-31's `Mixed { stream_lost }` says the stream's own `lost` accounts for part
    // of the jump and nothing else explains the rest — so it requires that no carry
    // explains it. The guard and the Gap's own `link_dropped` must therefore count
    // the same blocks: reading the incoming carry alone while `link_dropped` reports
    // the carried total produced a Gap claiming nothing explains the shortfall and
    // reporting a link drop in the same breath.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    // One block dropped, and the delivered block is contiguous — so the count is held.
    b.push(
        &header(t(100), 100, 1),
        DropCarry {
            blocks: 1,
            ..DropCarry::default()
        },
    )
    .expect("accepted");
    // A jump the stream's own `lost` only partly accounts for, with an empty carry.
    let mut h = header(t(600), 100, 1);
    h.flags = h.flags | BlockFlags::GAP_BEFORE;
    h.lost = Some(150);
    b.push(&h, DropCarry::default()).expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(
        map.gaps[0].link_dropped, 1,
        "the held block reaches this Gap"
    );
    assert_eq!(
        map.gaps[0].cause,
        GapCause::Stream {},
        "a Gap that reports a link drop cannot also claim nothing explains the shortfall"
    );

    // And with nothing held, the same shape *is* `Mixed`.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let mut h = header(t(500), 100, 1);
    h.flags = h.flags | BlockFlags::GAP_BEFORE;
    h.lost = Some(150);
    b.push(&h, DropCarry::default()).expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps[0].link_dropped, 0);
    assert_eq!(map.gaps[0].cause, GapCause::Mixed { stream_lost: 150 });
}

#[test]
fn sc_30c_a_rejected_push_keeps_both_carries() {
    // A rejected push keeps the carry it was given, beside what the builder already
    // held, for `finish` (SC-30c).
    let mut b = builder(1, true); // lossless: the path that returns JumpWithoutGapFlag
    push(&mut b, header(t(0), 100, 1));
    b.push(
        &header(t(100), 100, 1),
        DropCarry {
            blocks: 2,
            ..DropCarry::default()
        },
    )
    .expect("a contiguous block is accepted");
    let err = b
        .push(
            &header(t(500), 100, 1),
            DropCarry { blocks: 1, ..DropCarry::default() },
        )
        .expect_err("a jump with no GAP_BEFORE on a lossless path is refused");
    assert!(matches!(err, StreamError::JumpWithoutGapFlag));
    let map = b.finish(DropCarry::default());
    assert_eq!(map.gaps.len(), 1);
    assert_eq!(map.gaps[0].link_dropped, 3, "the held 2 and the rejected push's 1");

    // An overlapping block is refused the same way and keeps its carry too.
    let mut b = builder(1, false);
    push(&mut b, header(t(0), 100, 1));
    let err = b
        .push(&header(t(50), 100, 1), DropCarry { blocks: 1, ..DropCarry::default() })
        .expect_err("an overlapping block is refused");
    assert!(matches!(err, StreamError::TimeOverlap { .. }));
    let map = b.finish(DropCarry::default());
    assert_eq!((map.gaps.len(), map.gaps[0].link_dropped), (1, 1));
}

#[test]
fn sc_30b_a_first_block_s_carry_is_recorded_at_it() {
    // Blocks dropped before a stream's first delivered block are recorded at that
    // block, never in a later, unrelated gap (SC-30b).
    let mut b = builder(1, false);
    b.push(&header(t(100), 100, 1), DropCarry { blocks: 3, ..DropCarry::default() })
        .expect("a first block with a carry is accepted");
    push(&mut b, header(t(200), 100, 1));
    let mut h = header(t(400), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    h.lost = Some(100);
    push(&mut b, h);
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps,
        [
            Gap { start: t(100), len: 0, lost: None, cause: GapCause::LinkDrop {}, link_dropped: 3 },
            Gap { start: t(300), len: 100, lost: Some(100), cause: GapCause::OverflowRestart {}, link_dropped: 0 },
        ]
    );
    assert_eq!(map.first, t(100));

    // With a first-block gap too: the dropped blocks leave the origin unknown, so the
    // gap is zero-extent at the block and keeps what is known, as a trailing carry's
    // does (SC-30c).
    let mut b = builder(1, false);
    let mut h = header(t(40), 100, 1);
    h.flags = BlockFlags::GAP_BEFORE;
    h.lost = Some(10);
    b.push(&h, DropCarry { blocks: 2, lost: Some(5), ..DropCarry::default() })
        .expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps,
        [Gap { start: t(40), len: 0, lost: Some(15), cause: GapCause::Stream {}, link_dropped: 2 }]
    );
    assert_eq!(map.first, t(40));

    // Without a first-block gap, a carry with a known count keeps it the same way.
    let mut b = builder(1, false);
    b.push(
        &header(t(40), 100, 1),
        DropCarry { flags: BlockFlags::GAP_BEFORE, lost: Some(5), blocks: 1 },
    )
    .expect("accepted");
    let map = b.finish(DropCarry::default());
    assert_eq!(
        map.gaps,
        [Gap { start: t(40), len: 0, lost: Some(5), cause: GapCause::Stream {}, link_dropped: 1 }]
    );
}
