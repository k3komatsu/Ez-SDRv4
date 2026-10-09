//! Node-qualified identifiers (`00-overview.md` X7, Vision §49).
//!
//! Every Kernel identifier that could ever name something on another host is a
//! `{node, local}` pair, and `ResourceId` is a `{node, path}` pair because a resource
//! is a composite tree (`03-spec-and-binding.md` SB-33). v4.0 refuses any id whose
//! `node` is not [`NodeId::LOCAL`]: [`NodeId`]'s deserialiser refuses it in every
//! document, and `validate()` refuses it in the ids handed in as Rust values (D91).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A node in the (future) multi-host deployment. `0` (the local node) is the only legal value in v4.0.
///
/// Rule: X7 (`00-overview.md`); it qualifies every id of TM-11, SC-6, SB-3 and MA-38.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// X7 at the document boundary (D91): every id in every document embeds a `NodeId`,
/// so refusing a non-local node here covers them all. The schema stays `uint32`, so
/// lifting X7 later is a code loosening, not a schema change.
impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = u32::deserialize(d)?;
        if n != NodeId::LOCAL.0 {
            return Err(serde::de::Error::custom(format!(
                "X7: v4.0 accepts only node 0 (LOCAL), not node {n}"
            )));
        }
        Ok(NodeId(n))
    }
}

/// SB-1 at the document boundary (D95): a name type deserialises through its own
/// `parse`, so a document cannot carry a name a Rust caller could not have built.
/// `what` names the grammar the refusal reports.
pub(crate) fn parsed<'de, D, T, E>(
    d: D,
    what: &str,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    parse(&s).map_err(|_| serde::de::Error::custom(format!("SB-1: {s:?} is not {what}")))
}

impl<'de> Deserialize<'de> for ResourceId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // The node is checked by `NodeId`'s own deserialiser (X7), the path by
        // `ResourceId::parse` (SB-1, D95); the closed field set is SB-9's.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            node: NodeId,
            path: String,
        }
        let raw = Raw::deserialize(d)?;
        let mut id = ResourceId::parse(&raw.path).map_err(|e| {
            serde::de::Error::custom(format!("SB-1: resource path {:?}: {e}", raw.path))
        })?;
        id.node = raw.node;
        Ok(id)
    }
}

impl<'de> Deserialize<'de> for ModuleId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        parsed(d, "a ModuleId", ModuleId::parse)
    }
}

impl NodeId {
    /// The only node id v4.0 accepts (X7).
    pub const LOCAL: NodeId = NodeId(0);

    /// True when this id is one v4.0 accepts (X7).
    pub fn is_local(self) -> bool {
        self == NodeId::LOCAL
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_local() { f.write_str("local") } else { write!(f, "node{}", self.0) }
    }
}

/// Identifies a `ClockDomain`.
///
/// Rule: TM-11 (`01-time-model.md`), X7.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ClockDomainId {
    /// Owning node; `0`, the local node, throughout v4.0 (X7).
    pub node: NodeId,
    /// Node-local ordinal, allocated by the owning registry.
    pub local: u32,
}

/// Identifies a memory domain a sample block lives in.
///
/// Rule: SC-6 (`02-stream-contract.md`), X7.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct MemoryDomainId {
    /// Owning node; `0`, the local node, throughout v4.0 (X7).
    pub node: NodeId,
    /// Node-local ordinal, allocated by the owning registry.
    pub local: u32,
}

/// Identifies a scheduling island (`05-module-api.md` MA-27).
///
/// Rule: MA-27, X7.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct IslandId {
    /// Owning node; `0`, the local node, throughout v4.0 (X7).
    pub node: NodeId,
    /// Node-local ordinal, allocated by the owning registry.
    pub local: u32,
}

/// Identifies a data link instance (`DataLinkDecl`).
///
/// Rule: SC-18 (`02-stream-contract.md`), X7.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct DataLinkId {
    /// Owning node; `0`, the local node, throughout v4.0 (X7).
    pub node: NodeId,
    /// Node-local ordinal, allocated by the owning registry.
    pub local: u32,
}

// Written out rather than generated: `kernel_surface` does not read macro bodies,
// so a macro here hid four Kernel document types from the allow-list that is the
// Kernel's own review checklist. Four copies of one impl pair is the price of a
// gate that cannot be walked past (OV-23, OV-23b, X11).
impl ClockDomainId {
    /// A local id with the given ordinal (X7).
    pub const fn local(local: u32) -> Self {
        Self { node: NodeId::LOCAL, local }
    }
}

impl fmt::Display for ClockDomainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:clk#{}", self.node, self.local)
    }
}

impl MemoryDomainId {
    /// A local id with the given ordinal (X7).
    pub const fn local(local: u32) -> Self {
        Self { node: NodeId::LOCAL, local }
    }
}

impl fmt::Display for MemoryDomainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:mem#{}", self.node, self.local)
    }
}

impl IslandId {
    /// A local id with the given ordinal (X7).
    pub const fn local(local: u32) -> Self {
        Self { node: NodeId::LOCAL, local }
    }
}

impl fmt::Display for IslandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:island#{}", self.node, self.local)
    }
}

impl DataLinkId {
    /// A local id with the given ordinal (X7).
    pub const fn local(local: u32) -> Self {
        Self { node: NodeId::LOCAL, local }
    }
}

impl fmt::Display for DataLinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:link#{}", self.node, self.local)
    }
}

impl ClockDomainId {
    /// Reserved: UTC, `Root`, 1 GHz, epoch 1970 (TM-11).
    pub const UTC: ClockDomainId = ClockDomainId::local(0);
    /// Reserved: the host monotonic clock, `Root`, 1 GHz, arbitrary epoch (TM-11).
    pub const HOST_MONOTONIC: ClockDomainId = ClockDomainId::local(1);
    /// The first id a registry may allocate; 0 and 1 are reserved (TM-11).
    pub const FIRST_ALLOCATABLE: u32 = 2;
}

/// Names a resource, or a sub-resource, inside the composite tree a Provider declares.
///
/// The path is a non-empty sequence of segments matching
/// `^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+){0,15}$`, at most 256 bytes; `dev0/rx/0` names
/// channel 0 of the receive sub-tree of `dev0`. Vision §49 asks only for the node qualification; the
/// path replaces §49's flat `local` because a channel or a timekeeper must be
/// addressable (`00-overview.md` "Where these specs depart from the Vision").
///
/// Rule: SB-1, SB-3, SB-33, SB-34 (`03-spec-and-binding.md`), X7.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceId {
    /// Owning node; `0`, the local node, throughout v4.0 (X7).
    pub node: NodeId,
    /// Slash-joined path through the composite resource tree (SB-34).
    pub path: String,
}

/// Why a [`ResourceId`] path was refused (SB-34).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResourceIdError {
    /// The path was empty, or a segment between two separators was.
    EmptySegment,
    /// A segment held a character outside `[A-Za-z0-9_.-]` (SB-34).
    BadCharacter(char),
    /// The path exceeded [`ResourceId::MAX_PATH_LEN`] bytes or [`ResourceId::MAX_DEPTH`] segments.
    TooLong,
}

impl fmt::Display for ResourceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceIdError::EmptySegment => f.write_str("resource path has an empty segment"),
            ResourceIdError::BadCharacter(c) => write!(f, "resource path holds {c:?}"),
            ResourceIdError::TooLong => f.write_str("resource path is too long"),
        }
    }
}

impl std::error::Error for ResourceIdError {}

impl ResourceId {
    /// Longest accepted path, in bytes (SB-34).
    pub const MAX_PATH_LEN: usize = 256;
    /// Deepest accepted path, in segments (SB-34).
    pub const MAX_DEPTH: usize = 16;

    /// Parses a slash-separated path. Segments are non-empty and drawn from
    /// `[A-Za-z0-9_.-]`, which keeps ids ASCII so that canonical-JSON key order is
    /// byte order (`00-overview.md` §7).
    ///
    /// Rule: SB-34.
    pub fn parse(path: &str) -> Result<ResourceId, ResourceIdError> {
        if path.len() > Self::MAX_PATH_LEN {
            return Err(ResourceIdError::TooLong);
        }
        let mut depth = 0;
        for seg in path.split('/') {
            depth += 1;
            if seg.is_empty() {
                return Err(ResourceIdError::EmptySegment);
            }
            if let Some(c) = seg.chars().find(|c| !matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '.' | '-')) {
                return Err(ResourceIdError::BadCharacter(c));
            }
        }
        if depth > Self::MAX_DEPTH {
            return Err(ResourceIdError::TooLong);
        }
        Ok(ResourceId { node: NodeId::LOCAL, path: path.to_owned() })
    }

    /// The path split into its segments (SB-34).
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.path.split('/')
    }

    /// The id of the enclosing resource, or `None` for a root (SB-33).
    pub fn parent(&self) -> Option<ResourceId> {
        let cut = self.path.rfind('/')?;
        Some(ResourceId { node: self.node, path: self.path[..cut].to_owned() })
    }

    /// True when `self` is `other` or lies underneath it in the composite tree (SB-33).
    pub fn is_within(&self, other: &ResourceId) -> bool {
        self.node == other.node
            && self
                .path
                .strip_prefix(&other.path)
                .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
    }

    /// Appends one segment, validating it (SB-34).
    pub fn child(&self, segment: &str) -> Result<ResourceId, ResourceIdError> {
        ResourceId::parse(&format!("{}/{}", self.path, segment))
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.node, self.path)
    }
}

/// Names a Module implementation in the registry, matching
/// `^[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*$`, at most 256 bytes.
///
/// The reverse-DNS-ish namespaced form (`ezsdr.test.provider`) is what keeps two
/// vendors' Modules from colliding; the Kernel checks the shape, not the authority.
///
/// Rule: SB-1, MA-31.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ModuleId(String);

/// One segment of a `ModuleId`: non-empty and `[A-Za-z0-9_-]`. Shared with `Key`, so
/// that an `ext.<module-id>.<path>` key is parseable for every Module id SB-1 admits
/// and not only for the ones that also fit `Ident`'s grammar (SB-1, SB-2, MA-34).
pub(crate) fn is_module_segment(seg: &str) -> bool {
    !seg.is_empty()
        && !seg.chars().any(|c| !matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-'))
}

impl ModuleId {
    /// Parses a dotted namespaced name; segments are non-empty and `[A-Za-z0-9_-]`.
    ///
    /// Rule: SB-1, MA-31.
    pub fn parse(name: &str) -> Result<ModuleId, ResourceIdError> {
        if name.len() > ResourceId::MAX_PATH_LEN {
            return Err(ResourceIdError::TooLong);
        }
        for seg in name.split('.') {
            if seg.is_empty() {
                return Err(ResourceIdError::EmptySegment);
            }
            if let Some(c) = seg.chars().find(|c| !matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-')) {
                return Err(ResourceIdError::BadCharacter(c));
            }
            debug_assert!(is_module_segment(seg));
        }
        Ok(ModuleId(name.to_owned()))
    }

    /// The name as written (MA-31).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies one Run, unique per host (`04-run-and-session.md` RS-1, X6).
///
/// The form is `<node>:<pid-hex>:<unix-nanos-hex>-<counter>`. The process id is
/// required: `node` is fixed at `LOCAL` for all of v4.0 and the counter is
/// per-process, so without it two `ezsdr` processes on one host produce identical
/// ids within one nanosecond. Ceiling: unique per host, not across hosts until
/// `NodeId` is real (`00-overview.md` §8).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Mints a fresh id from the wall clock, the process id and a per-process counter (RS-1).
    pub fn generate() -> RunId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        RunId(format!("{}:{:x}:{:x}-{}", NodeId::LOCAL, std::process::id(), nanos, n))
    }

    /// Wraps an id read back from a Manifest (RS-1).
    pub fn from_string(s: String) -> RunId {
        RunId(s)
    }

    /// The id as written (RS-1).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
