//! Phase 1 test doubles (OV-20). Never compiled into `src/`.
//!
//! Four doubles: an in-memory `DataLink` implementing all three policies, a
//! test-double Provider, a recording `TestExecutor` and a fake `HostClock`.
//! `ManualTimeAuthority` is the exception and ships in `src/` behind the `testing`
//! feature, because Phase 2's Simulation Engine builds on it (OV-20, TM-17a).

#![allow(dead_code)]

pub mod doubles;
#[allow(unused_imports)]
pub use doubles::*;

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::id::MemoryDomainId;
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BlockRef, BufferRef, ChannelMask, DataLink, Direction,
    DropCarry, HostMemoryAccess, PublishOutcome, SampleBlock,
};
use ezsdr_kernel::time::TimePoint;

/// The host memory domain the doubles allocate from.
pub const HOST_MEM: MemoryDomainId = MemoryDomainId::local(0);
/// A memory domain that is deliberately not host-reachable (SC-8).
pub const GPU_MEM: MemoryDomainId = MemoryDomainId::local(1);

/// `ezsdr.stream.cf32`, the contract the doubles publish under (SC-4).
pub fn cf32() -> DataContractId {
    DataContractId::parse("ezsdr.stream.cf32").expect("SC-4 id")
}

/// Bytes per sample of [`cf32`] (SC-4).
pub const CF32_BPS: u32 = 8;

/// A buffer big enough for `channels · len` cf32 samples, in the host domain.
///
/// ponytail: the bytes are leaked so `map_host` can hand out a slice that outlives
/// the link, which is exactly what SC-8 and decision S5 require of a real pool.
/// The Phase 2 pool recycles slots instead.
pub fn host_buffer(channels: u16, len: u32) -> BufferRef {
    let n = channels as usize * len as usize * CF32_BPS as usize;
    let bytes: &'static [u8] = Box::leak(vec![0u8; n].into_boxed_slice());
    BufferRef {
        memory_domain: HOST_MEM,
        handle: bytes.as_ptr() as u64,
        len_bytes: bytes.len() as u64,
    }
}

/// A header with the common defaults; callers override what they are testing.
pub fn header(t: TimePoint, len: u32, channels: u16) -> BlockHeader {
    BlockHeader {
        first_sample_time: t,
        len,
        channels,
        direction: Direction::Rx,
        valid: ChannelMask::full(channels),
        flags: BlockFlags::NONE,
        lost: None,
        contract: cf32(),
    }
}

/// A block from a header, with a buffer sized to fit (SC-10a).
pub fn block(h: BlockHeader) -> BlockRef {
    let buffer = host_buffer(h.channels, h.len);
    BlockRef::new(SampleBlock::new(h, buffer, CF32_BPS).expect("a well-formed test block"))
}

/// An in-memory link implementing all three back-pressure policies.
///
/// ponytail: a mutex around a `VecDeque`, not lock-free (decision S9). Real links
/// are Phase 2 Link Modules; this one exists to prove the policy semantics.
pub struct MemLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
    carry: Mutex<DropCarry>,
}

impl MemLink {
    /// A link with the given policy and capacity in blocks (SC-19).
    pub fn new(policy: BackPressure, capacity: u32) -> MemLink {
        MemLink {
            policy,
            // SC-19 puts a capacity of at least 1 on every declared link; the double
            // clamps rather than panicking if a test hands it 0.
            capacity: (capacity as usize).max(1),
            queue: Mutex::new(VecDeque::new()),
            drops: AtomicU64::new(0),
            carry: Mutex::new(DropCarry::default()),
        }
    }

    fn record_drop(&self, dropped: &BlockRef) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        self.carry.lock().expect("lock").absorb(dropped);
    }
}

impl DataLink for MemLink {
    fn publish(&self, b: BlockRef) -> PublishOutcome {
        let mut q = self.queue.lock().expect("lock");
        if q.len() < self.capacity {
            q.push_back(b);
            return PublishOutcome::Accepted;
        }
        match self.policy {
            // SC-20a: the producer still owns the block; nothing is discarded.
            BackPressure::Block => PublishOutcome::Full,
            BackPressure::DropOldest => {
                let evicted = q.pop_front().expect("at capacity");
                self.record_drop(&evicted);
                q.push_back(b);
                PublishOutcome::DroppedOldest
            }
            BackPressure::DropNewest => {
                self.record_drop(&b);
                PublishOutcome::DroppedNewest
            }
        }
    }

    fn receive(&self) -> Option<BlockRef> {
        self.queue.lock().expect("lock").pop_front()
    }

    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    fn take_drop_carry(&self) -> DropCarry {
        std::mem::take(&mut *self.carry.lock().expect("lock"))
    }

    fn policy(&self) -> BackPressure {
        self.policy
    }
}

impl HostMemoryAccess for MemLink {
    fn map_host<'a>(&self, b: &'a BlockRef) -> Option<&'a [u8]> {
        let buf = b.buffer();
        if buf.memory_domain != HOST_MEM {
            return None; // SC-8: nothing for a domain that is not host-reachable.
        }
        // Safety is not at stake here: the double leaks its buffers (see `host_buffer`),
        // so the slice is genuinely `'static`.
        Some(unsafe { std::slice::from_raw_parts(buf.handle as *const u8, buf.len_bytes as usize) })
    }
}

/// A producer that honours SC-20a: on `Full` it keeps the block and retries,
/// never discarding it silently. It panics if it is ever asked to drop one.
pub struct RetryingProducer {
    pending: Option<BlockRef>,
    pub delivered: u32,
    pub refusals: u32,
}

impl RetryingProducer {
    /// A producer with nothing pending (SC-20a).
    pub fn new() -> RetryingProducer {
        RetryingProducer { pending: None, delivered: 0, refusals: 0 }
    }

    /// Offers `b` — or the block held over from a previous refusal — to the link.
    /// Returns what the link said.
    pub fn offer(&mut self, link: &dyn DataLink, b: BlockRef) -> PublishOutcome {
        assert!(self.pending.is_none(), "SC-20a: a refused block must be retried, never dropped");
        match link.publish(b.clone()) {
            PublishOutcome::Full => {
                self.pending = Some(b);
                self.refusals += 1;
                PublishOutcome::Full
            }
            other => {
                self.delivered += 1;
                other
            }
        }
    }

    /// Retries whatever is held over (SC-20a).
    pub fn retry(&mut self, link: &dyn DataLink) -> Option<PublishOutcome> {
        let b = self.pending.take()?;
        match link.publish(b.clone()) {
            PublishOutcome::Full => {
                self.pending = Some(b);
                self.refusals += 1;
                Some(PublishOutcome::Full)
            }
            other => {
                self.delivered += 1;
                Some(other)
            }
        }
    }
}

impl Default for RetryingProducer {
    fn default() -> Self {
        RetryingProducer::new()
    }
}
