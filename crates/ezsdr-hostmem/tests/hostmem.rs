use std::sync::Arc;

use ezsdr_hostmem::{CF32_BYTES, HOST_MEMORY, HostPool, interleave, read_cf32, write_cf32};
use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::id::{ClockDomainId, MemoryDomainId};
use ezsdr_kernel::stream::{
    BlockHeader, BlockRef, BufferRef, ChannelMask, Direction, SampleBlock,
};
use ezsdr_kernel::time::TimePoint;

fn header(ticks: i64, len: u32, channels: u16) -> BlockHeader {
    BlockHeader {
        first_sample_time: TimePoint::new(ClockDomainId::local(7), ticks),
        len,
        channels,
        direction: Direction::Rx,
        valid: ChannelMask::full(channels),
        flags: Default::default(),
        lost: None,
        contract: DataContractId::parse("ezsdr.stream.cf32").unwrap(),
    }
}

fn hosted_block(ticks: i64, len: u32, channels: u16, bytes: Arc<[u8]>) -> BlockRef {
    Arc::new(
        SampleBlock::new_host(header(ticks, len, channels), HOST_MEMORY, bytes, CF32_BYTES as u32)
            .unwrap(),
    )
}

#[test]
fn hd_02_a_released_slot_is_reused_and_a_held_one_is_not() {
    assert_eq!(HOST_MEMORY, MemoryDomainId::local(0));
    assert_eq!(CF32_BYTES, 8);

    let mut pool = HostPool::new(32);
    let first = pool.fill(16, |bytes| bytes.fill(1));
    let first_block = hosted_block(0, 2, 1, first);
    assert_eq!(pool.slots(), 1);

    let second = pool.fill(16, |bytes| bytes.fill(2));
    assert_eq!(pool.slots(), 2, "a held block keeps its buffer immutable");
    assert_eq!(&first_block.host_bytes().unwrap()[..16], &[1; 16]);
    drop(first_block);
    drop(second);

    let reused = pool.fill(8, |bytes| bytes.fill(3));
    assert_eq!(pool.slots(), 2, "the unreferenced first slot is reused");
    assert_eq!(&reused[..8], &[3; 8]);

    let unpooled = pool.fill(33, |bytes| bytes.fill(4));
    assert_eq!(unpooled.len(), 33);
    assert_eq!(pool.slots(), 2, "an oversized buffer is not added to the pool");
}

#[test]
fn hd_03_layout_round_trip() {
    let (channels, len) = (3_u16, 3_usize);
    let mut pool = HostPool::new(channels as usize * len * CF32_BYTES);
    let bytes = pool.fill(channels as usize * len * CF32_BYTES, |bytes| {
        for channel in 0..channels as usize {
            for index in 0..len {
                write_cf32(
                    bytes,
                    len,
                    channel,
                    index,
                    channel as f32 * 10.0 + index as f32,
                    -(channel as f32 * 10.0 + index as f32),
                );
            }
        }
    });
    let block = SampleBlock::new_host(header(100, len as u32, channels), HOST_MEMORY, bytes, 8)
        .unwrap();

    for channel in 0..channels as usize {
        for index in 0..len {
            let value = channel as f32 * 10.0 + index as f32;
            assert_eq!(
                read_cf32(block.host_bytes().unwrap(), len, channel, index),
                (value, -value)
            );
        }
    }

    let mut expected = Vec::new();
    for index in 1..3 {
        for channel in 0..channels as usize {
            let value = channel as f32 * 10.0 + index as f32;
            expected.extend_from_slice(&value.to_le_bytes());
            expected.extend_from_slice(&(-value).to_le_bytes());
        }
    }
    assert_eq!(interleave(&block, CF32_BYTES, 1, 3), expected);
    assert!(interleave(&block, CF32_BYTES, 3, 2).is_empty());
    assert!(interleave(&block, CF32_BYTES, 0, 4).is_empty());

    let no_host_bytes = SampleBlock::new(
        header(200, 1, 1),
        BufferRef {
            memory_domain: HOST_MEMORY,
            handle: 0,
            len_bytes: CF32_BYTES as u64,
        },
        CF32_BYTES as u32,
    )
    .unwrap();
    assert!(interleave(&no_host_bytes, CF32_BYTES, 0, 1).is_empty());
}
