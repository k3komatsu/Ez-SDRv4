//! `SampleBlock`, its flags and its construction invariants —
//! `02-stream-contract.md` SC-10…SC-18 (Vision §23).

use std::fmt;
use std::ops::BitOr;

use serde::{Deserialize, Serialize};

use super::{BufferRef, StreamError};
use crate::contract::DataContractId;
use crate::id::MemoryDomainId;
use crate::time::{Duration, TimeError, TimePoint};

/// Which side of the radio produced the block, taken from the producing Port at
/// construction. Without the field nothing can detect a receive Provider that sets
/// a start-of-burst flag by copying a transmit code path (SC-16).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A receive stream.
    Rx,
    /// A transmit stream.
    Tx,
}

/// Bit `c` set means channel `c` is valid in this block.
///
/// Ceiling: 64 channels per stream, a deliberate limit (decision S1). The inner
/// field is crate-private so that widening it stays an in-memory change with no
/// schema impact and no public-API break, which decision S1 promises.
///
/// Rule: SC-14.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ChannelMask(pub(crate) u64);

impl ChannelMask {
    /// The mask whose set bits are those of `bits` (SC-14).
    pub fn from_bits(bits: u64) -> ChannelMask {
        ChannelMask(bits)
    }

    /// The set bits, for a producer writing a wire header (SC-14).
    pub fn bits(self) -> u64 {
        self.0
    }

    /// The mask with every channel below `channels` set (SC-14).
    pub fn full(channels: u16) -> ChannelMask {
        if channels >= 64 {
            ChannelMask(u64::MAX)
        } else {
            ChannelMask((1u64 << channels) - 1)
        }
    }

    /// Whether channel `c` is valid (SC-14).
    pub fn is_set(self, c: u16) -> bool {
        c < 64 && self.0 & (1u64 << c) != 0
    }
}

/// The block flag set. Bit positions are fixed by `02-stream-contract.md` so that
/// an Executor mapping input blocks to output blocks one-to-one can propagate
/// flags unchanged by default (SC-17, decision S17).
///
/// The inner field is crate-private: the bit positions are public through the
/// constants, and the width is not, so widening it is not a public-API break
/// (decision S1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct BlockFlags(pub(crate) u16);

impl BlockFlags {
    /// The flag set whose bits are those of `bits`, for a producer reading a wire
    /// header (SC-10 rejects a block whose reserved bits are set).
    pub fn from_bits(bits: u16) -> BlockFlags {
        BlockFlags(bits)
    }

    /// The set bits, for a producer writing a wire header (SC-17).
    pub fn bits(self) -> u16 {
        self.0
    }

    /// Samples are missing between the previous block's end and this first sample (SC-13).
    pub const GAP_BEFORE: BlockFlags = BlockFlags(0x0001);
    /// The gap is transport sequence loss (SC-13, SC-18).
    pub const SEQ_DISCONTINUITY: BlockFlags = BlockFlags(0x0002);
    /// The gap is a stream restart, as a UHD overflow produces (SC-13, SC-18, OV-23a).
    pub const RESTARTED: BlockFlags = BlockFlags(0x0004);
    /// On receive: the stream started later than the requested time; set on the
    /// first block of the stream only (SC-16a, decision S19).
    pub const LATE: BlockFlags = BlockFlags(0x0008);
    /// Derived by the constructor: `valid` is not the full mask (SC-10, decision S3).
    pub const PARTIAL_CHANNELS: BlockFlags = BlockFlags(0x0010);
    /// Opens a transmit burst (SC-24).
    pub const START_OF_BURST: BlockFlags = BlockFlags(0x0020);
    /// Closes a transmit burst (SC-24).
    pub const END_OF_BURST: BlockFlags = BlockFlags(0x0040);
    /// A multi-channel alignment failure; the cause of a per-channel break (SC-31a).
    pub const ALIGNMENT: BlockFlags = BlockFlags(0x0080);
    /// Bits 8–15, which must be zero (SC-10).
    pub const RESERVED: BlockFlags = BlockFlags(0xFF00);
    /// No flags set: the fast path one-to-one propagation relies on (SC-17).
    pub const NONE: BlockFlags = BlockFlags(0);

    /// True when every bit of `other` is set here (SC-17).
    pub fn contains(self, other: BlockFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when any bit of `other` is set here (SC-10).
    pub fn intersects(self, other: BlockFlags) -> bool {
        self.0 & other.0 != 0
    }

    /// True when no flag is set (SC-17).
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for BlockFlags {
    type Output = BlockFlags;
    fn bitor(self, rhs: BlockFlags) -> BlockFlags {
        BlockFlags(self.0 | rhs.0)
    }
}

impl fmt::Display for BlockFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:04x}", self.0)
    }
}

/// Everything about a block except its bytes. The continuity builder and the burst
/// tracker read headers alone (SC-29, SC-30).
///
/// Rule: SC-10.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlockHeader {
    /// Time of the first sample, in the stream's SampleClock (TM-10, SC-12).
    pub first_sample_time: TimePoint,
    /// Samples per channel; at least 1, because zero-length blocks are never
    /// published (SC-10, decision S14).
    pub len: u32,
    /// Channel count, 1..=64 (SC-10, decision S1).
    pub channels: u16,
    /// Which side produced the block (SC-16).
    pub direction: Direction,
    /// Per-channel validity, constant within the block (SC-14).
    pub valid: ChannelMask,
    /// The flag set; `PARTIAL_CHANNELS` is derived, never supplied (SC-10).
    pub flags: BlockFlags,
    /// Samples lost before this block, when known. Present only with
    /// `GAP_BEFORE`, and at least 1 (SC-13).
    pub lost: Option<u64>,
    /// The contract the payload satisfies (SC-1).
    pub contract: DataContractId,
}

impl BlockHeader {
    /// Time just past the block's last sample: `first_sample_time + len`, checked (SC-12).
    pub fn end_time(&self) -> Result<TimePoint, TimeError> {
        self.first_sample_time.checked_add(Duration::new(
            self.first_sample_time.domain,
            self.len.into(),
        ))
    }
}

/// An immutable block of samples: a header plus a [`BufferRef`].
///
/// Immutable once published — the type has no mutation interface — and shared by
/// reference, so fan-out to N consumers is N links carrying the same reference and
/// nothing is copied (SC-11).
///
/// Not a document: no schema and no version, because it never leaves the process
/// (decision S18, X8).
///
/// Rule: SC-10, SC-10a, SC-11.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SampleBlock {
    header: BlockHeader,
    buffer: BufferRef,
    host: Option<HostBytes>,
}

/// Host bytes a producer attached to a block; they live as long as the block does
/// (SC-8, SC-9). Compared by content and printed by length only.
#[derive(Clone)]
struct HostBytes(std::sync::Arc<[u8]>);

impl PartialEq for HostBytes {
    fn eq(&self, other: &Self) -> bool {
        self.0[..] == other.0[..]
    }
}

impl Eq for HostBytes {}

impl fmt::Debug for HostBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HostBytes({} bytes)", self.0.len())
    }
}

/// A shared, immutable reference to a [`SampleBlock`]; fan-out shares it (SC-11).
pub type BlockRef = std::sync::Arc<SampleBlock>;

impl SampleBlock {
    /// Builds a block, enforcing every shape invariant of SC-10 and SC-10a.
    ///
    /// `bytes_per_sample` comes from the producing Port's DataContract, resolved at
    /// `prepare`. Without SC-10a's check the one invariant that prevents an
    /// out-of-bounds read is unchecked: SC-4 fixes the planar layout, so a Provider
    /// publishing four channels of 2 000 samples over a buffer sized for two
    /// produces a consumer-side panic or a garbage read rather than a Kernel error.
    ///
    /// Rule: SC-10, SC-10a, SC-14, SC-16.
    pub fn new(
        header: BlockHeader,
        buffer: BufferRef,
        bytes_per_sample: u32,
    ) -> Result<SampleBlock, StreamError> {
        let bad = |reason: &str| StreamError::InvalidBlock {
            reason: reason.to_owned(),
        };
        let f = header.flags;

        if header.len < 1 {
            return Err(bad(
                "len must be at least 1; zero-length blocks are never published",
            ));
        }
        if header.channels < 1 || header.channels > 64 {
            return Err(bad("channels must be in 1..=64"));
        }
        if header.valid.0 & !ChannelMask::full(header.channels).0 != 0 {
            return Err(bad("a valid bit is set at or above `channels`"));
        }
        if f.intersects(BlockFlags::RESERVED) {
            return Err(bad("a reserved flag bit (8..15) is set"));
        }
        if f.contains(BlockFlags::PARTIAL_CHANNELS) {
            return Err(bad(
                "PARTIAL_CHANNELS is derived by the constructor, never supplied",
            ));
        }
        match header.lost {
            Some(0) => return Err(bad("`lost` must be at least 1 when present")),
            Some(_) if !f.contains(BlockFlags::GAP_BEFORE) => {
                return Err(bad("`lost` is present only together with GAP_BEFORE"));
            }
            _ => {}
        }
        if f.intersects(BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY)
            && !f.contains(BlockFlags::GAP_BEFORE)
        {
            return Err(bad("RESTARTED or SEQ_DISCONTINUITY implies GAP_BEFORE"));
        }
        if f.intersects(BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST)
            && f.intersects(
                BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY,
            )
        {
            return Err(bad(
                "a burst flag never accompanies GAP_BEFORE, RESTARTED or SEQ_DISCONTINUITY",
            ));
        }
        // SC-16: the direction rules.
        match header.direction {
            Direction::Rx
                if f.intersects(BlockFlags::START_OF_BURST | BlockFlags::END_OF_BURST) =>
            {
                return Err(bad(
                    "a receive block must not carry START_OF_BURST or END_OF_BURST",
                ));
            }
            Direction::Tx
                if f.intersects(
                    BlockFlags::GAP_BEFORE | BlockFlags::RESTARTED | BlockFlags::SEQ_DISCONTINUITY,
                ) =>
            {
                // A jump in transmit time is a discontinuity (SC-24), never a flagged gap.
                return Err(bad(
                    "a transmit block must not carry GAP_BEFORE, RESTARTED or SEQ_DISCONTINUITY",
                ));
            }
            _ => {}
        }
        // SC-10a: the planar layout of SC-4 makes this the bound on every read.
        let needed = (header.channels as u64)
            .checked_mul(header.len as u64)
            .and_then(|v| v.checked_mul(bytes_per_sample as u64))
            .ok_or_else(|| bad("the block's byte size overflows"))?;
        if buffer.len_bytes < needed {
            return Err(bad(
                "buffer.len_bytes is smaller than channels · len · bytes_per_sample",
            ));
        }

        let mut header = header;
        // SC-10, SC-14, decision S3: derived here so the flag and the mask cannot drift apart.
        if header.valid != ChannelMask::full(header.channels) {
            header.flags = header.flags | BlockFlags::PARTIAL_CHANNELS;
        }
        Ok(SampleBlock {
            header,
            buffer,
            host: None,
        })
    }

    /// Builds a block whose bytes are host memory the producer attaches, with every
    /// check of [`SampleBlock::new`]; the buffer reference names `memory_domain` with
    /// the bytes' length (SC-8, SC-10a, KA-3).
    pub fn new_host(
        header: BlockHeader,
        memory_domain: MemoryDomainId,
        bytes: std::sync::Arc<[u8]>,
        bytes_per_sample: u32,
    ) -> Result<SampleBlock, StreamError> {
        let buffer = BufferRef {
            memory_domain,
            handle: 0,
            len_bytes: bytes.len() as u64,
        };
        let mut block = SampleBlock::new(header, buffer, bytes_per_sample)?;
        block.host = Some(HostBytes(bytes));
        Ok(block)
    }

    /// The host bytes a producer attached with [`SampleBlock::new_host`], or `None`
    /// (SC-8).
    pub fn host_bytes(&self) -> Option<&[u8]> {
        self.host.as_ref().map(|h| &h.0[..])
    }

    /// The validated header, with `PARTIAL_CHANNELS` derived (SC-10).
    pub fn header(&self) -> &BlockHeader {
        &self.header
    }

    /// Time of the first sample (SC-12).
    pub fn first_sample_time(&self) -> TimePoint {
        self.header.first_sample_time
    }

    /// Time just past the last sample, checked (SC-12).
    pub fn end_time(&self) -> Result<TimePoint, TimeError> {
        self.header.end_time()
    }

    /// The payload reference; opaque to the Kernel (SC-7).
    pub fn buffer(&self) -> BufferRef {
        self.buffer
    }
}
