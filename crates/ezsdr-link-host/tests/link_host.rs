use std::sync::Arc;

use ezsdr_kernel::contract::{DataContractId, PortRef};
use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{ClockDomainId, DataLinkId, MemoryDomainId, ModuleId};
use ezsdr_kernel::module_api::{
    Deployment, Factories, KERNEL_API, Link, ModuleRef, ModuleRegistry, Role, Version,
};
use ezsdr_kernel::spec::{Ident, Namespace};
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BlockRef, ChannelMask, DataLinkDecl, Direction,
    PublishOutcome, SampleBlock,
};
use ezsdr_kernel::time::TimePoint;
use ezsdr_link_host::{HostLinkModule, descriptor, link_descriptor};

fn id(name: &str) -> Ident {
    Ident::parse(name).unwrap()
}

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.link.host").unwrap(),
        version: Version::new(1, 0, 0),
    }
}

fn declaration(policy: BackPressure, capacity: u32) -> DataLinkDecl {
    DataLinkDecl {
        id: DataLinkId::local(3),
        from: PortRef {
            component: id("radio"),
            port: id("rx"),
        },
        to: PortRef {
            component: id("rec"),
            port: id("in"),
        },
        contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
        policy,
        capacity,
    }
}

fn block(ticks: i64, flags: BlockFlags, lost: Option<u64>) -> BlockRef {
    let channels = 1;
    let len = 10;
    let header = BlockHeader {
        first_sample_time: TimePoint::new(ClockDomainId::local(7), ticks),
        len,
        channels,
        direction: Direction::Rx,
        valid: ChannelMask::full(channels),
        flags,
        lost,
        contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
    };
    let bytes: Arc<[u8]> = vec![0; len as usize * 8].into();
    Arc::new(SampleBlock::new_host(header, MemoryDomainId::local(0), bytes, 8).unwrap())
}

#[test]
fn hd_04_descriptor_and_create() {
    let module_descriptor = descriptor();
    assert_eq!(module_descriptor.id, ModuleId::parse("ezsdr.link.host").unwrap());
    assert_eq!(module_descriptor.version, Version::new(1, 0, 0));
    assert_eq!(module_descriptor.kernel_api, KERNEL_API);
    assert_eq!(module_descriptor.roles, [Role::Link]);
    assert!(module_descriptor.vocabularies.is_empty());
    assert_eq!(module_descriptor.deployment, Deployment::InProcess {});
    assert_eq!(
        module_descriptor.impl_hash,
        Some(ContentHash::of_bytes(b"ezsdr.link.host 1.0.0"))
    );

    let expected = link_descriptor();
    assert_eq!(expected.module, module_ref());
    assert_eq!(expected.kind, Namespace::parse("ezsdr.link.host").unwrap());
    assert_eq!(
        expected.connects,
        [(MemoryDomainId::local(0), MemoryDomainId::local(0))]
    );
    assert_eq!(
        expected.policies,
        [
            BackPressure::Block,
            BackPressure::DropOldest,
            BackPressure::DropNewest
        ]
    );
    assert!(!expected.cross_process);

    let mut registry = ModuleRegistry::new();
    registry
        .register(module_descriptor, Factories { link: true, ..Factories::default() })
        .unwrap();
    registry.register_link_descriptor(expected.clone()).unwrap();
    assert_eq!(registry.link_descriptor(&module_ref()), Some(&expected));

    let module = HostLinkModule::new();
    assert_eq!(module.descriptor(), &expected);
    let error = match module.create(&declaration(BackPressure::DropOldest, 0)) {
        Err(error) => error,
        Ok(_) => panic!("HD-4: a zero-capacity Link was created"),
    };
    assert!(error.message.starts_with("HD-4: "));
    let link = module
        .create(&declaration(BackPressure::DropNewest, 3))
        .unwrap();
    assert_eq!(link.policy(), BackPressure::DropNewest);
}

#[test]
fn hd_05_policies() {
    let block_policy = HostLinkModule::new()
        .create(&declaration(BackPressure::Block, 1))
        .unwrap();
    assert_eq!(block_policy.policy(), BackPressure::Block);
    assert_eq!(block_policy.publish(block(0, BlockFlags::NONE, None)), PublishOutcome::Accepted);
    let retry = block(10, BlockFlags::NONE, None);
    assert_eq!(
        block_policy.publish(Arc::clone(&retry)),
        PublishOutcome::Full
    );
    assert_eq!(block_policy.drops(), 0, "Block never discards or records a drop");
    assert_eq!(block_policy.receive().unwrap().0.first_sample_time().ticks, 0);
    assert_eq!(block_policy.publish(retry), PublishOutcome::Accepted);
    assert_eq!(block_policy.receive().unwrap().0.first_sample_time().ticks, 10);

    let oldest = HostLinkModule::new()
        .create(&declaration(BackPressure::DropOldest, 1))
        .unwrap();
    let carried_flags = BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED;
    assert_eq!(
        oldest.publish(block(100, carried_flags, Some(150))),
        PublishOutcome::Accepted
    );
    assert_eq!(
        oldest.publish(block(200, BlockFlags::NONE, None)),
        PublishOutcome::DroppedOldest
    );
    let (kept, carry) = oldest.receive().unwrap();
    assert_eq!(kept.first_sample_time().ticks, 200);
    assert!(carry.flags.contains(BlockFlags::GAP_BEFORE));
    assert!(carry.flags.contains(BlockFlags::RESTARTED));
    assert_eq!(carry.lost, Some(150));
    assert_eq!(carry.blocks, 1);
    assert!(oldest.take_drop_carry().is_empty(), "the carry went with its block");
    assert_eq!(oldest.drops(), 1, "the drop counter does not reset with the carry");

    let first_gap = BlockFlags::GAP_BEFORE;
    let second_gap = BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY;
    assert_eq!(oldest.publish(block(300, first_gap, Some(7))), PublishOutcome::Accepted);
    assert_eq!(oldest.publish(block(400, second_gap, Some(5))), PublishOutcome::DroppedOldest);
    assert_eq!(oldest.publish(block(500, BlockFlags::NONE, None)), PublishOutcome::DroppedOldest);
    let (_, carry) = oldest.receive().unwrap();
    assert_eq!(
        carry.flags,
        BlockFlags::GAP_BEFORE | BlockFlags::SEQ_DISCONTINUITY
    );
    assert_eq!(carry.lost, Some(12));
    assert_eq!(carry.blocks, 2);
    assert_eq!(oldest.drops(), 3);

    let newest = HostLinkModule::new()
        .create(&declaration(BackPressure::DropNewest, 2))
        .unwrap();
    assert_eq!(
        [0, 10, 20].map(|at| newest.publish(block(at, BlockFlags::NONE, None))),
        [
            PublishOutcome::Accepted,
            PublishOutcome::Accepted,
            PublishOutcome::DroppedNewest
        ]
    );
    assert_eq!(newest.receive().unwrap().0.first_sample_time().ticks, 0);
    assert_eq!(newest.receive().unwrap().0.first_sample_time().ticks, 10);
    assert_eq!(newest.drops(), 1);
    assert_eq!(newest.take_drop_carry().blocks, 1);

}

#[test]
fn hd_05_a_carry_belongs_to_the_block_after_the_drop() {
    // SC-20b: `receive` returns each block with what was dropped immediately before
    // it. DropNewest: the queued head keeps its own gap, and the blocks refused after
    // it go with the next accepted block.
    let newest = HostLinkModule::new()
        .create(&declaration(BackPressure::DropNewest, 1))
        .unwrap();
    newest.publish(block(300, BlockFlags::GAP_BEFORE, Some(100)));
    newest.publish(block(400, BlockFlags::NONE, None));
    newest.publish(block(500, BlockFlags::NONE, None));
    let (head, carry) = newest.receive().unwrap();
    assert_eq!((head.first_sample_time().ticks, carry.is_empty()), (300, true));
    newest.publish(block(600, BlockFlags::NONE, None));
    let (next, carry) = newest.receive().unwrap();
    assert_eq!((next.first_sample_time().ticks, carry.blocks), (600, 2));
    assert!(newest.take_drop_carry().is_empty());

    // DropOldest: the evicted block's carry goes to the block after it, not the newest.
    let oldest = HostLinkModule::new()
        .create(&declaration(BackPressure::DropOldest, 2))
        .unwrap();
    for at in [0, 10, 20] {
        oldest.publish(block(at, BlockFlags::NONE, None));
    }
    let (first, carry) = oldest.receive().unwrap();
    assert_eq!((first.first_sample_time().ticks, carry.blocks), (10, 1));
    let (second, carry) = oldest.receive().unwrap();
    assert_eq!((second.first_sample_time().ticks, carry.blocks), (20, 0));

    // A recording that ends takes everything dropped after the last received block,
    // queued blocks' carries included, and they are not delivered again.
    for at in [30, 40, 50] {
        oldest.publish(block(at, BlockFlags::NONE, None));
    }
    assert_eq!(oldest.take_drop_carry().blocks, 1);
    assert!(oldest.receive().unwrap().1.is_empty());
}
