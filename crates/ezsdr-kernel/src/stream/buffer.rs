//! Memory domains and buffer references — `02-stream-contract.md` SC-6…SC-9 (Vision §31).

use super::BlockRef;
use crate::id::MemoryDomainId;

/// A memory domain, an opaque handle and a length in bytes. The handle is
/// meaningful only to the owner of that memory domain — the producing pool, or the
/// link that carries the domain. No Kernel interface dereferences a handle.
///
/// Rule: SC-7.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferRef {
    /// Which memory domain the bytes live in (SC-7).
    pub memory_domain: MemoryDomainId,
    /// Opaque to the Kernel; meaningful only to the domain's owner (SC-7).
    pub handle: u64,
    /// Size of the buffer in bytes; SC-10a checks it against the block shape (SC-7).
    pub len_bytes: u64,
}

/// Host bytes are obtained only through this interface, implemented by the
/// [`DataLink`](super::DataLink), which delegates to the owning pool and yields
/// nothing for a domain that is not host-reachable.
///
/// The returned slice borrows the **block**, not the link: a pool may recycle the
/// buffer as soon as the last [`BlockRef`] is dropped, so a slice tied to the
/// link's lifetime would outlive its own bytes (decision S5).
///
/// Rule: SC-8. Vision invariant 34.
pub trait HostMemoryAccess {
    /// The block's bytes, or `None` when its domain is not host-reachable (SC-8).
    fn map_host<'a>(&self, b: &'a BlockRef) -> Option<&'a [u8]>;
}
