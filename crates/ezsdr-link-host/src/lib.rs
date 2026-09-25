//! Ez-SDR v4 Module ezsdr.link.host 1.0.0 (plan/phase2/10-host-data-path.md).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_kernel::hash::ContentHash;
use ezsdr_kernel::id::{MemoryDomainId, ModuleId};
use ezsdr_kernel::module_api::{
    Deployment, KERNEL_API, Link, LinkDescriptor, ModuleDescriptor, ModuleError, ModuleRef,
    Role, Version,
};
use ezsdr_kernel::stream::{
    BackPressure, BlockRef, DataLink, DataLinkDecl, DropCarry, PublishOutcome,
};

fn module_ref() -> ModuleRef {
    ModuleRef {
        id: ModuleId::parse("ezsdr.link.host").expect("a valid Module id"),
        version: Version::new(1, 0, 0),
    }
}

/// The Module descriptor for `ezsdr.link.host` 1.0.0 (HD-4).
pub fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: ModuleId::parse("ezsdr.link.host").expect("a valid Module id"),
        version: Version::new(1, 0, 0),
        kernel_api: KERNEL_API,
        roles: vec![Role::Link],
        vocabularies: Vec::new(),
        deployment: Deployment::InProcess {},
        impl_hash: Some(ContentHash::of_bytes(b"ezsdr.link.host 1.0.0")),
    }
}

/// The memory domains and back-pressure policies supported by this Link (HD-4).
pub fn link_descriptor() -> LinkDescriptor {
    // HD-1's HOST_MEMORY; this crate may not depend on ezsdr-hostmem (00-overview.md §5).
    let host_memory = MemoryDomainId::local(0);
    LinkDescriptor {
        module: module_ref(),
        kind: ezsdr_kernel::spec::Namespace::parse("ezsdr.link.host")
            .expect("a valid Link kind"),
        connects: vec![(host_memory, host_memory)],
        policies: vec![
            BackPressure::Block,
            BackPressure::DropOldest,
            BackPressure::DropNewest,
        ],
        cross_process: false,
    }
}

/// A Link Module factory for host-to-host block queues (HD-4).
pub struct HostLinkModule {
    descriptor: LinkDescriptor,
}

impl HostLinkModule {
    /// Creates a factory with the Phase 2 host Link descriptor (HD-4).
    pub fn new() -> HostLinkModule {
        HostLinkModule {
            descriptor: link_descriptor(),
        }
    }
}

impl Default for HostLinkModule {
    fn default() -> HostLinkModule {
        HostLinkModule::new()
    }
}

impl Link for HostLinkModule {
    fn descriptor(&self) -> &LinkDescriptor {
        &self.descriptor
    }

    fn create(&self, decl: &DataLinkDecl) -> Result<Arc<dyn DataLink>, ModuleError> {
        if decl.capacity == 0 {
            return Err(ModuleError::rejected(
                "HD-4: a link needs a capacity of at least 1 (SC-19)",
            ));
        }
        Ok(Arc::new(HostLink::new(decl.policy, decl.capacity as usize)))
    }
}

/// A bounded in-process host-memory block queue implementing the declared policy (HD-5).
pub struct HostLink {
    policy: BackPressure,
    capacity: usize,
    queue: Mutex<VecDeque<BlockRef>>,
    drops: AtomicU64,
    carry: Mutex<DropCarry>,
}

impl HostLink {
    fn new(policy: BackPressure, capacity: usize) -> HostLink {
        HostLink {
            policy,
            capacity,
            queue: Mutex::new(VecDeque::new()),
            drops: AtomicU64::new(0),
            carry: Mutex::new(DropCarry::default()),
        }
    }

    fn record_drop(&self, dropped: &BlockRef) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        lock_unpoisoned(&self.carry).absorb(dropped);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl DataLink for HostLink {
    fn publish(&self, block: BlockRef) -> PublishOutcome {
        let mut queue = lock_unpoisoned(&self.queue);
        if queue.len() < self.capacity {
            queue.push_back(block);
            return PublishOutcome::Accepted;
        }
        match self.policy {
            BackPressure::Block => PublishOutcome::Full,
            BackPressure::DropOldest => {
                let evicted = queue.pop_front().expect("the queue is at capacity");
                self.record_drop(&evicted);
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
        lock_unpoisoned(&self.queue).pop_front()
    }

    fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    fn take_drop_carry(&self) -> DropCarry {
        std::mem::take(&mut *lock_unpoisoned(&self.carry))
    }

    fn policy(&self) -> BackPressure {
        self.policy
    }
}
