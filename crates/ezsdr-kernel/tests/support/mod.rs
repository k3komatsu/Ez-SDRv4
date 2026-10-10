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
pub mod run_doubles;
#[allow(unused_imports)]
pub use run_doubles::*;
pub mod paced;
#[allow(unused_imports)]
pub use paced::*;

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ezsdr_kernel::contract::DataContractId;
use ezsdr_kernel::id::MemoryDomainId;
use ezsdr_kernel::stream::{
    BackPressure, BlockFlags, BlockHeader, BlockRef, BufferRef, ChannelMask, DataLink, Direction,
    DropCarry, PublishOutcome, SampleBlock,
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

/// A buffer reference big enough for `channels · len` cf32 samples, in the host
/// domain, for a block built with `SampleBlock::new` (no host bytes attached).
pub fn host_buffer(channels: u16, len: u32) -> BufferRef {
    let n = channels as u64 * len as u64 * CF32_BPS as u64;
    BufferRef {
        memory_domain: HOST_MEM,
        handle: 0,
        len_bytes: n,
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
    let n = h.channels as usize * h.len as usize * CF32_BPS as usize;
    let bytes: std::sync::Arc<[u8]> = vec![0u8; n].into();
    BlockRef::new(
        SampleBlock::new_host(h, HOST_MEM, bytes, CF32_BPS).expect("a well-formed test block"),
    )
}

/// An in-memory link implementing all three back-pressure policies.
///
/// ponytail: a mutex around a `VecDeque`, not lock-free (decision S9). Real links
/// are Phase 2 Link Modules; this one exists to prove the policy semantics.
pub struct MemLink {
    policy: BackPressure,
    capacity: usize,
    /// Each block with what was dropped just before it, and what was dropped after
    /// the last queued block (SC-20b).
    queue: Mutex<(VecDeque<(BlockRef, DropCarry)>, DropCarry)>,
    drops: AtomicU64,
}

impl MemLink {
    /// A link with the given policy and capacity in blocks (SC-19).
    pub fn new(policy: BackPressure, capacity: u32) -> MemLink {
        MemLink {
            policy,
            // SC-19 puts a capacity of at least 1 on every declared link; the double
            // clamps rather than panicking if a test hands it 0.
            capacity: (capacity as usize).max(1),
            queue: Mutex::new((VecDeque::new(), DropCarry::default())),
            drops: AtomicU64::new(0),
        }
    }
}

impl DataLink for MemLink {
    fn publish(&self, b: BlockRef) -> PublishOutcome {
        let mut guard = self.queue.lock().expect("lock");
        let (q, tail) = &mut *guard;
        let outcome = match (q.len() < self.capacity, self.policy) {
            (true, _) => PublishOutcome::Accepted,
            // SC-20a: the producer still owns the block; nothing is discarded.
            (false, BackPressure::Block) => return PublishOutcome::Full,
            (false, BackPressure::DropOldest) => {
                let (evicted, mut carry) = q.pop_front().expect("at capacity");
                carry.absorb(&evicted);
                q.front_mut().map_or(&mut *tail, |(_, next)| next).merge(carry);
                PublishOutcome::DroppedOldest
            }
            (false, BackPressure::DropNewest) => {
                tail.absorb(&b);
                PublishOutcome::DroppedNewest
            }
        };
        if outcome != PublishOutcome::Accepted {
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
        if outcome != PublishOutcome::DroppedNewest {
            let carry = std::mem::take(tail);
            q.push_back((b, carry));
        }
        outcome
    }

    fn receive(&self) -> Option<(BlockRef, DropCarry)> {
        self.queue.lock().expect("lock").0.pop_front()
    }

    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    fn take_drop_carry(&self) -> DropCarry {
        let mut guard = self.queue.lock().expect("lock");
        let (q, tail) = &mut *guard;
        let mut carry = std::mem::take(tail);
        for (_, queued) in q.iter_mut() {
            carry.merge(std::mem::take(queued));
        }
        carry
    }

    fn policy(&self) -> BackPressure {
        self.policy
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
        RetryingProducer {
            pending: None,
            delivered: 0,
            refusals: 0,
        }
    }

    /// Offers `b` — or the block held over from a previous refusal — to the link.
    /// Returns what the link said.
    pub fn offer(&mut self, link: &dyn DataLink, b: BlockRef) -> PublishOutcome {
        assert!(
            self.pending.is_none(),
            "SC-20a: a refused block must be retried, never dropped"
        );
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
