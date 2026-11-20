//! RM-38: the rule both Providers call for a `Block` link on a radio receive port — its admission,
//! what a discard adds to the next block, the once-per-segment event and the discard count — and
//! the merge rule of the device loss reports a block carries (RM-17).

use ezsdr_kernel::stream::{BackPressure, BlockFlags};

use crate::payloads::{RxOverflowCause, RxOverflowPayload, rx_overflow_payload};

/// The refusal text of a port whose links RM-38 does not admit.
pub const REASON: &str = "RM-38: a Block link must be its receive port's only link";

/// The stats key of the samples a Provider read and discarded while a `Block` link answered `Full` (RM-38).
pub const DISCARDED_STAT: &str = "rx_link_discarded_samples";

/// The flags a discard adds to the next block a link accepts: the gap, and nothing a device report adds (RM-38).
pub const GAP_FLAGS: BlockFlags = BlockFlags::GAP_BEFORE;

/// RM-38: `Some(`[`REASON`]`)` when a port's links hold a `Block` link beside another link, else `None`.
pub fn refusal(policies: &[BackPressure]) -> Option<&'static str> {
    (policies.len() > 1 && policies.contains(&BackPressure::Block)).then_some(REASON)
}

/// The payload of RM-38's `ezsdr.LINK_BACKPRESSURE`: none; the stream node that emits it names the port.
pub fn event_payload() -> serde_json::Value {
    serde_json::json!({})
}

/// What one refused block adds (RM-38).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refused {
    /// The samples to add to [`DISCARDED_STAT`]: the block's length, clipped at a cut.
    pub samples: u64,
    /// Whether this block is the segment's first discard, which owes the segment's one `LINK_BACKPRESSURE`.
    pub raise_event: bool,
}

/// One receive segment's memory of RM-38's event: owed once, at its first discard.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Discards {
    raised: bool,
}

impl Discards {
    /// A block of `len` samples was refused.
    pub fn refuse(&mut self, len: u64) -> Refused {
        Refused { samples: len, raise_event: !std::mem::replace(&mut self.raised, true) }
    }
}

/// RM-17: the device loss the reports before a block yield, merged: the smallest first lost sample, the summed loss, `Overrun` over `Sequence`. A discard is no report and is not in it (RM-38).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LossReport {
    /// The first sample the device lost.
    pub first: i64,
    /// The samples the device lost.
    pub lost: u64,
    /// The report's cause.
    pub cause: RxOverflowCause,
}

impl LossReport {
    /// Takes `other` into this report.
    pub fn merge(&mut self, other: &LossReport) {
        self.first = self.first.min(other.first);
        self.lost = self.lost.saturating_add(other.lost);
        if other.cause == RxOverflowCause::Overrun {
            self.cause = RxOverflowCause::Overrun;
        }
    }

    /// The `RX_OVERFLOW` the report owes; none for a loss of 0.
    pub fn payload(&self) -> Option<RxOverflowPayload> {
        rx_overflow_payload(&[self.cause], Some(self.lost))
    }
}

/// The samples the device lost before `cut` among the half-open sample intervals `gaps`; an overlap is counted once.
pub fn lost_before(gaps: &[(i64, i64)], cut: i64) -> u64 {
    let mut gaps: Vec<(i64, i64)> = gaps.iter().map(|&(from, to)| (from, to.min(cut))).filter(|(from, to)| from < to).collect();
    gaps.sort_unstable();
    let mut total = 0u64;
    let mut covered = i64::MIN;
    for (from, to) in gaps {
        let from = from.max(covered);
        if from < to {
            total = total.saturating_add((to - from) as u64);
            covered = to;
        }
    }
    total
}
