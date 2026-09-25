//! Stream Contract — `02-stream-contract.md` (rules `SC-n`).
//!
//! Vision §23 makes the stream normative rather than conventional, because v3
//! proved what happens otherwise: its UHD bridge (evidence only, OV-23a) discarded
//! the receive error code
//! and the receive timestamp, so an overflow produced a silently gapped, untimed
//! array and the user was left to detect drift by hand.

mod block;
mod buffer;
mod burst;
mod continuity;
mod link;

use std::fmt;

pub use block::{BlockFlags, BlockHeader, BlockRef, ChannelMask, Direction, SampleBlock};
pub use buffer::BufferRef;
pub use burst::{
    AdmittedTarget, BurstEnd, BurstOpen, BurstRecord, BurstState, BurstStep, BurstTracker,
    LateOutcome, LatePolicy, admit_burst_target,
};
pub use continuity::{ChannelGap, ContinuityBuilder, ContinuityMap, Gap, GapCause, Segment};
pub use link::{BackPressure, DataLink, DataLinkDecl, DropCarry, PublishOutcome, check_sink_link};

use crate::contract::DataContractId;
use crate::id::ClockDomainId;
use crate::time::{TimeError, TimePoint};

/// Why a stream operation was refused (SC-10, SC-12, SC-24, SC-30).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StreamError {
    /// A time operation failed; carries `DomainMismatch` and `Overflow` from
    /// spec 01 (TM-8).
    Time(TimeError),
    /// A transmit block was published while idle without `START_OF_BURST`; it must
    /// not be transmitted (SC-24).
    MissingStartOfBurst,
    /// A block's first sample precedes the previous block's end (SC-12).
    TimeOverlap {
        /// Where the block should have started.
        expected: TimePoint,
        /// Where it did start.
        got: TimePoint,
    },
    /// `GAP_BEFORE` on a block that is contiguous with its predecessor (SC-13).
    GapFlagWithoutJump,
    /// A jump with no `GAP_BEFORE` on a lossless path (SC-30).
    JumpWithoutGapFlag,
    /// A block, contract id or link declaration failed a shape check (SC-10, SC-10a).
    InvalidBlock {
        /// What was wrong.
        reason: String,
    },
    /// A block arrived in a different SampleClock: a rate change, which ends the
    /// map (SC-30, TM-13c).
    DomainChanged {
        /// The map's domain.
        from: ClockDomainId,
        /// The block's domain.
        to: ClockDomainId,
    },
    /// A block arrived with a different channel count, which ends the map (SC-30a).
    ChannelsChanged {
        /// The map's channel count.
        from: u16,
        /// The block's channel count.
        to: u16,
    },
    /// The producer contract is neither equal to the consumer's nor in its
    /// `compatible_from` (SC-3).
    Incompatible {
        /// The producer contract.
        from: DataContractId,
        /// The consumer contract.
        to: DataContractId,
    },
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamError::Time(e) => write!(f, "{e}"),
            StreamError::MissingStartOfBurst => {
                f.write_str("a transmit block was published while idle without START_OF_BURST")
            }
            StreamError::TimeOverlap { expected, got } => {
                write!(
                    f,
                    "block time {got} precedes the previous block's end {expected}"
                )
            }
            StreamError::GapFlagWithoutJump => f.write_str("GAP_BEFORE on a contiguous block"),
            StreamError::JumpWithoutGapFlag => {
                f.write_str("a time jump with no GAP_BEFORE on a lossless path")
            }
            StreamError::InvalidBlock { reason } => write!(f, "invalid block: {reason}"),
            StreamError::DomainChanged { from, to } => {
                write!(f, "the SampleClock changed from {from} to {to}")
            }
            StreamError::ChannelsChanged { from, to } => {
                write!(f, "the channel count changed from {from} to {to}")
            }
            StreamError::Incompatible { from, to } => {
                write!(f, "contract {from} is not accepted by {to}")
            }
        }
    }
}

impl std::error::Error for StreamError {}
