//! Memory domains and buffer references — `02-stream-contract.md` SC-6…SC-9 (Vision §31).

use serde::{Deserialize, Serialize};

use super::BlockRef;
use crate::id::MemoryDomainId;

/// A memory domain: an id and a namespaced `kind`. The Kernel compares ids and
/// never interprets kinds, because the Kernel is frozen while Vision §31's list of
/// kinds will grow (decision S6).
///
/// Kinds named at v4.0: `ezsdr.mem.host`, `pinned_host`, `huge_pages`,
/// `wasm_linear`, `gpu`, `device`, `remote`.
///
/// Rule: SC-6.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MemoryDomain {
    /// Node-qualified identity (SC-6, X7).
    pub id: MemoryDomainId,
    /// Namespaced kind string, uninterpreted by the Kernel (SC-6).
    pub kind: String,
}

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
