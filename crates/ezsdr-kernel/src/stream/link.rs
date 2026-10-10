//! DataLink identity, back-pressure policy and drop attribution —
//! `02-stream-contract.md` SC-19…SC-22 (Vision §23 rule 8, §30).

use serde::{Deserialize, Serialize};

use super::{BlockFlags, BlockRef, StreamError};
use crate::contract::{DataContractId, PortRef};
use crate::id::DataLinkId;

/// What a link does when its queue is at capacity. There is no default (SC-19).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum BackPressure {
    /// Publishing is refused as full and nothing is ever dropped.
    /// The name is the Vision's; the policy produces back-pressure through a
    /// refusal the producer must honour, not through a parked thread (SC-20a).
    Block,
    /// The oldest queued block is evicted (SC-20).
    DropOldest,
    /// The new block is refused (SC-20).
    DropNewest,
}

impl BackPressure {
    /// True for the two policies that discard blocks. A drop-class link never
    /// returns `Full` (SC-20).
    pub fn is_drop_class(self) -> bool {
        matches!(self, BackPressure::DropOldest | BackPressure::DropNewest)
    }
}

/// A declared link: its endpoints, the contract it carries, its policy and its
/// capacity in blocks (SC-19).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataLinkDecl {
    /// Node-qualified identity (SC-19, X7).
    pub id: DataLinkId,
    /// The producing port (SC-19).
    pub from: PortRef,
    /// The consuming port (SC-19).
    pub to: PortRef,
    /// The contract carried; SC-3 checks it against both ports (SC-19).
    pub contract: DataContractId,
    /// What happens at capacity; there is no default (SC-19).
    pub policy: BackPressure,
    /// Queue depth in blocks, at least 1 (SC-19).
    pub capacity: u32,
}

/// What a `publish` did (SC-20).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PublishOutcome {
    /// Queued.
    Accepted,
    /// Queued after evicting the oldest queued block (`DropOldest`).
    DroppedOldest,
    /// Refused and discarded by the link (`DropNewest`).
    DroppedNewest,
    /// Refused and **not** discarded: the producer still owns the block (`Block`).
    /// A producer that receives this must not discard it silently — it retries on
    /// its next turn, or stops and emits `LINK_BACKPRESSURE` (SC-20a).
    Full,
}

/// What a drop-class link accumulated from the blocks it discarded between two
/// delivered blocks (SC-20b).
///
/// Without the carry, a hardware overflow whose block is then evicted by the lossy
/// link in front of a recorder is re-derived as a plain link drop, and the Manifest
/// attributes a device overflow to host-side loss with the lost count thrown away
/// although it was known — v3's failure reappearing one layer up.
///
/// Rule: SC-20b.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DropCarry {
    /// The union of the dropped blocks' `GAP_BEFORE`, `RESTARTED`,
    /// `SEQ_DISCONTINUITY` and `ALIGNMENT` flags (SC-20b).
    pub flags: BlockFlags,
    /// The sum of the `lost` counts that were present (SC-20b).
    pub lost: Option<u64>,
    /// How many blocks were dropped (SC-20b).
    pub blocks: u32,
}

impl DropCarry {
    /// The flags a carry keeps from a dropped block (SC-20b).
    pub const CARRIED_FLAGS: BlockFlags = BlockFlags(
        BlockFlags::GAP_BEFORE.0
            | BlockFlags::RESTARTED.0
            | BlockFlags::SEQ_DISCONTINUITY.0
            | BlockFlags::ALIGNMENT.0,
    );

    /// True when nothing has been dropped (SC-20b).
    pub fn is_empty(self) -> bool {
        self.blocks == 0 && self.flags.is_empty() && self.lost.is_none()
    }

    /// Folds one dropped block into the carry (SC-20b).
    pub fn absorb(&mut self, dropped: &BlockRef) {
        let h = dropped.header();
        self.merge(DropCarry {
            flags: BlockFlags(h.flags.0 & DropCarry::CARRIED_FLAGS.0),
            lost: h.lost,
            blocks: 1,
        });
    }

    /// Folds another carry in: flags united, `lost` counts summed where present,
    /// block counts added (SC-20b).
    pub fn merge(&mut self, other: DropCarry) {
        self.flags = self.flags | other.flags;
        if let Some(n) = other.lost {
            self.lost = Some(self.lost.unwrap_or(0).saturating_add(n));
        }
        self.blocks = self.blocks.saturating_add(other.blocks);
    }
}

/// One end-to-end block queue. `publish` never parks: a step-driven Island cannot
/// afford a parking publish, and a [`PublishOutcome::Full`] that could never be
/// observed would be dead (SC-20a, decision S9).
///
/// Rule: SC-19, SC-20, SC-20a, SC-20b.
pub trait DataLink: Send + Sync {
    /// Queues a block, reporting what happened. Never parks (SC-20, SC-20a).
    fn publish(&self, b: BlockRef) -> PublishOutcome;
    /// The next queued block, if any, with what was dropped immediately before it in
    /// stream order (SC-20, SC-20b).
    fn receive(&self) -> Option<(BlockRef, DropCarry)>;
    /// A never-dropping counter, incremented on every drop and read by the event
    /// collector (SC-20, Vision §29).
    fn drops(&self) -> u64;
    /// Everything dropped after the last block `receive` returned, queued blocks'
    /// carries included, which a recording that ends now records as its trailing
    /// carry; cleared by this call (SC-20b, SC-30c).
    fn take_drop_carry(&self) -> DropCarry;
    /// The declared policy (SC-19).
    fn policy(&self) -> BackPressure;
}

/// A link whose consumer port belongs to a Sink-role Module must be drop-class,
/// because observation must never stall the real-time path. The planner calls this
/// from `validate()` (specs 03 and 05).
///
/// Rule: SC-21. Vision §30, invariant 21.
pub fn check_sink_link(decl: &DataLinkDecl, consumer_is_sink: bool) -> Result<(), StreamError> {
    if consumer_is_sink && !decl.policy.is_drop_class() {
        return Err(StreamError::InvalidBlock {
            reason: format!(
                "link {} feeds a Sink and must be drop-class, not {:?} (SC-21)",
                decl.id, decl.policy
            ),
        });
    }
    Ok(())
}
