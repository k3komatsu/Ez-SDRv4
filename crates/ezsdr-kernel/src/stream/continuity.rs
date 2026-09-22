//! Continuity derivation — `02-stream-contract.md` SC-30…SC-31d (Vision §23, §28).
//!
//! `ContinuityMap` and the per-channel validity it contains are derived by this
//! builder from block headers alone, and never assembled by hand (SC-30). Sink-side
//! derivation would let every Sink derive it differently, which is what Vision §28
//! forbids.

use serde::{Deserialize, Serialize};

use super::{BlockFlags, BlockHeader, DropCarry, StreamError};
use crate::id::ClockDomainId;
use crate::time::{TimeError, TimePoint};

/// What the **stream** lost. A closed, derived set (SC-31, decision S16).
///
/// What the **link** lost is the separate `link_dropped` count, not a cause, so a
/// device overflow inside an interval that also lost a block to the link is still
/// reported as `OverflowRestart` with its sample count, beside a link-drop count of
/// one. Collapsing the two into a single cause loses the device's own diagnosis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GapCause {
    /// `GAP_BEFORE` alone (SC-31).
    Stream,
    /// `RESTARTED`: a stream restart, as a UHD overflow produces (SC-31, OV-23a).
    OverflowRestart,
    /// `SEQ_DISCONTINUITY`: transport sequence loss (SC-31).
    SequenceError,
    /// A per-channel break on a block carrying `ALIGNMENT` (SC-31a).
    Alignment,
    /// A jump with no gap flag on a lossy path (SC-31).
    LinkDrop,
    /// `GAP_BEFORE` with `lost` less than the jump on a lossy path and no carry to
    /// explain the difference (SC-31).
    Mixed {
        /// What the stream itself reported losing.
        stream_lost: u64,
    },
    /// `GAP_BEFORE` with `lost` absent (SC-31).
    Unknown,
}

/// An interval of missing samples in the stream (SC-30, Vision §28).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Gap {
    /// Where the missing interval begins.
    pub start: TimePoint,
    /// Its extent in samples.
    pub len: u64,
    /// What the stream reported losing, when known (SC-13).
    pub lost: Option<u64>,
    /// What the stream lost (SC-31).
    pub cause: GapCause,
    /// How many blocks the link discarded inside the same interval. A block count,
    /// not a sample count: the samples the link lost are `len − lost`, and are
    /// unrecoverable when `lost` is absent. That is a real limit of what a dropped
    /// header can tell us, not an omission (SC-20b, SC-31).
    pub link_dropped: u32,
}

/// A channel that lost validity while the stream continued (SC-31a).
///
/// Vision §28 requires a capture to describe a multi-channel alignment failure, and
/// a hole between two segments alone is indistinguishable from a channel that was
/// never enabled over that interval.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChannelGap {
    /// Which channel (SC-31a).
    pub channel: u16,
    /// Where the break begins (SC-31d).
    pub start: TimePoint,
    /// Its extent in samples (SC-31d).
    pub len: u64,
    /// `Alignment` when the block that dropped the channel carried `ALIGNMENT`,
    /// `Stream` otherwise (SC-31a).
    pub cause: GapCause,
}

/// A contiguous run of valid samples on one channel (SC-30).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Segment {
    /// First sample of the run.
    pub start: TimePoint,
    /// Its extent in samples.
    pub len: u64,
}

/// What a stream carried, per SampleClock. There is one map per SampleClock: a
/// block in another domain ends the map, and the Sink starts a new builder (SC-30).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContinuityMap {
    /// The SampleClock this map describes (SC-30).
    pub domain: ClockDomainId,
    /// The channel count, constant for the map (SC-30a).
    pub channels: u16,
    /// Per channel, the runs of valid samples (SC-14, SC-30).
    pub valid: Vec<Vec<Segment>>,
    /// Stream gaps, in order (SC-31).
    pub gaps: Vec<Gap>,
    /// Per-channel breaks, in order (SC-31a).
    pub channel_gaps: Vec<ChannelGap>,
    /// First sample the map accounts for (SC-30).
    pub first: TimePoint,
    /// Just past the last sample the map accounts for (SC-30).
    pub end: TimePoint,
}

/// Derives a [`ContinuityMap`] from block headers, incrementally (SC-30, decision S15).
///
/// The builder is told whether its path is lossless; on a lossless path a jump
/// without `GAP_BEFORE` is a contract violation, and on a drop-class path it is a
/// link-drop gap.
pub struct ContinuityBuilder {
    domain: ClockDomainId,
    channels: u16,
    lossless: bool,
    expected_next: Option<TimePoint>,
    first: Option<TimePoint>,
    end: Option<TimePoint>,
    /// Per channel: the segment currently being extended.
    open: Vec<Option<Segment>>,
    /// Per channel: closed segments.
    valid: Vec<Vec<Segment>>,
    /// Per channel: a break remembered but not yet emitted, and why (SC-31d).
    pending: Vec<Option<(TimePoint, GapCause)>>,
    /// The previous pushed block's mask; empty before the first push (SC-31d).
    was_valid: super::ChannelMask,
    /// What a drop-class link refused that no Gap has recorded yet (SC-30b).
    ///
    /// Under `DropNewest` the refused block is newer than everything queued, so the
    /// block that follows the drop is **contiguous** with what came before: the
    /// consumer reads a non-empty `DropCarry` and hands it to a push that opens no
    /// gap. SC-30b gives the carry three jobs — its flags derive the cause, its
    /// `lost` counts sum, its block count reaches `link_dropped` — and a contiguous
    /// push has nowhere to put any of them, so the whole carry is held for the next
    /// Gap rather than any part of it being dropped. Holding only the block count
    /// wrote the next Gap as a plain `LinkDrop` with no `lost`, which positively
    /// attributes a device overflow to host-side link loss.
    pending_carry: DropCarry,
    gaps: Vec<Gap>,
    channel_gaps: Vec<ChannelGap>,
}

/// The cause a carry's flags alone imply (SC-30c, SC-31).
fn cause_from_carry(carry: &DropCarry) -> GapCause {
    if carry.flags.contains(BlockFlags::RESTARTED) {
        GapCause::OverflowRestart
    } else if carry.flags.contains(BlockFlags::SEQ_DISCONTINUITY) {
        GapCause::SequenceError
    } else if carry.flags.contains(BlockFlags::GAP_BEFORE) {
        if carry.lost.is_some() { GapCause::Stream } else { GapCause::Unknown }
    } else {
        GapCause::LinkDrop
    }
}

impl ContinuityBuilder {
    /// A builder for one SampleClock. `lossless` is false behind a drop-class link
    /// (SC-30).
    pub fn new(domain: ClockDomainId, channels: u16, lossless: bool) -> ContinuityBuilder {
        ContinuityBuilder {
            domain,
            channels,
            lossless,
            expected_next: None,
            first: None,
            end: None,
            open: vec![None; channels as usize],
            valid: vec![Vec::new(); channels as usize],
            pending: vec![None; channels as usize],
            was_valid: super::ChannelMask(0),
            pending_carry: DropCarry::default(),
            gaps: Vec::new(),
            channel_gaps: Vec::new(),
        }
    }

    /// One carry from two, by SC-20b's own rule: flags union, `lost` counts summed
    /// where present, block counts added (SC-20b, SC-30b).
    fn merge_carries(a: &DropCarry, b: &DropCarry) -> DropCarry {
        DropCarry {
            flags: a.flags | b.flags,
            lost: match (a.lost, b.lost) {
                (None, None) => None,
                (x, y) => Some(x.unwrap_or(0).saturating_add(y.unwrap_or(0))),
            },
            blocks: a.blocks.saturating_add(b.blocks),
        }
    }

    /// Whether this path can lose blocks (SC-30).
    pub fn is_lossless(&self) -> bool {
        self.lossless
    }

    /// Folds one block header in, together with the [`DropCarry`] the consumer read
    /// from its link (empty on a lossless path).
    ///
    /// A rejected push **returns the carry** to the consumer, which gives it to the
    /// *outgoing* builder's `finish` rather than to the new one: those samples
    /// belong to the domain and channel count that just ended, and a first push has
    /// no previous block and therefore no gap to merge into (SC-30c).
    ///
    /// Rule: SC-12, SC-13, SC-14, SC-30, SC-30a, SC-30b, SC-31, SC-31a, SC-31d.
    pub fn push(
        &mut self,
        h: &BlockHeader,
        carry: DropCarry,
    ) -> Result<(), (StreamError, DropCarry)> {
        if h.first_sample_time.domain != self.domain {
            // SC-30: a rate change starts a new SampleClock (TM-13c), not a gap.
            return Err((
                StreamError::DomainChanged { from: self.domain, to: h.first_sample_time.domain },
                carry,
            ));
        }
        if h.channels != self.channels {
            return Err((
                StreamError::ChannelsChanged { from: self.channels, to: h.channels },
                carry,
            ));
        }
        // Anything a previous contiguous push had to hold belongs to the next Gap,
        // so it is part of the carry this push is judged against (SC-30b).
        let carried = ContinuityBuilder::merge_carries(&self.pending_carry, &carry);
        // SC-30b: the carry's flags and counts merge into this block's.
        let flags = h.flags | carried.flags;
        let lost = match (h.lost, carried.lost) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
        };
        let t = h.first_sample_time;
        let end = match h.end_time() {
            Ok(e) => e,
            Err(e) => return Err((StreamError::Time(e), carry)),
        };

        if let Some(expected) = self.expected_next {
            let order = match t.try_cmp(expected) {
                Ok(o) => o,
                Err(e) => return Err((StreamError::Time(e), carry)),
            };
            match order {
                std::cmp::Ordering::Less => {
                    return Err((StreamError::TimeOverlap { expected, got: t }, carry));
                }
                // `h.flags`, not the merged set: the claim being checked is the
                // **delivered block's**. A `GAP_BEFORE` the carry brought describes a
                // block the link dropped, which is exactly the case where the
                // delivered block is contiguous and correct, so judging the merged
                // flags refused a well-formed push.
                std::cmp::Ordering::Equal if h.flags.contains(BlockFlags::GAP_BEFORE) => {
                    return Err((StreamError::GapFlagWithoutJump, carry));
                }
                // Contiguous, so there is no Gap to record the carry in; it is held
                // whole for the next one rather than discarded (SC-30b).
                std::cmp::Ordering::Equal => self.pending_carry = carried,
                std::cmp::Ordering::Greater => {
                    let jump = match t.checked_sub(expected) {
                        Ok(d) => d.ticks as u64,
                        Err(e) => return Err((StreamError::Time(e), carry)),
                    };
                    let dropped = carried.blocks;
                    if !flags.contains(BlockFlags::GAP_BEFORE) {
                        if self.lossless {
                            return Err((StreamError::JumpWithoutGapFlag, carry));
                        }
                        self.gaps.push(Gap {
                            start: expected,
                            len: jump,
                            lost: None,
                            cause: GapCause::LinkDrop,
                            link_dropped: dropped,
                        });
                    } else {
                        let cause = if flags.contains(BlockFlags::RESTARTED) {
                            GapCause::OverflowRestart
                        } else if flags.contains(BlockFlags::SEQ_DISCONTINUITY) {
                            GapCause::SequenceError
                        } else {
                            match lost {
                                None => GapCause::Unknown,
                                Some(n) if n < jump && carried.blocks == 0 && !self.lossless => {
                                    GapCause::Mixed { stream_lost: n }
                                }
                                Some(_) => GapCause::Stream,
                            }
                        };
                        self.gaps.push(Gap {
                            start: expected,
                            len: jump,
                            lost,
                            cause,
                            link_dropped: dropped,
                        });
                    }
                    // Cleared only here, where the push has committed to a Gap that
                    // carries it. Clearing before the `JumpWithoutGapFlag` return
                    // above destroyed the held carry on that one path, while every
                    // other rejected push leaves it for `finish` to fold — a
                    // rejected push returns the **caller's** carry, never the held
                    // one (SC-30b, SC-30c).
                    self.pending_carry = DropCarry::default();
                    // SC-31d: a pending break ends where the stream gap begins; the
                    // samples after that belong to the stream's own Gap.
                    self.flush_pending_at(expected);
                    // A stream gap breaks every open segment, and emits no
                    // ChannelGap for the channels it interrupts (SC-31b).
                    self.close_open_segments();
                }
            }
        }

        let was_valid = self.was_valid;
        for c in 0..h.channels {
            let i = c as usize;
            if h.valid.is_set(c) {
                if let Some((start, cause)) = self.pending[i].take() {
                    // SC-31a: the break's extent is known now that the channel is back.
                    let len = t.ticks.saturating_sub(start.ticks).max(0) as u64;
                    self.channel_gaps.push(ChannelGap { channel: c, start, len, cause });
                }
                match self.open[i].as_mut() {
                    Some(seg) => seg.len = end.ticks.saturating_sub(seg.start.ticks).max(0) as u64,
                    None => self.open[i] = Some(Segment { start: t, len: h.len as u64 }),
                }
            } else {
                // SC-31a: the flag on the block that DROPS the channel.
                let cause = if h.flags.contains(BlockFlags::ALIGNMENT) {
                    GapCause::Alignment
                } else {
                    GapCause::Stream
                };
                if let Some(seg) = self.open[i].take() {
                    let at = TimePoint::new(self.domain, seg.start.ticks.saturating_add(seg.len as i64));
                    self.valid[i].push(seg);
                    self.pending[i] = Some((at, cause));
                } else if was_valid.is_set(c) {
                    // SC-31d: the stream gap already closed this channel's segment,
                    // so the break starts here, not at the last valid sample.
                    self.pending[i] = Some((t, cause));
                }
            }
        }

        self.was_valid = h.valid;
        self.first.get_or_insert(t);
        self.end = Some(end);
        self.expected_next = Some(end);
        Ok(())
    }

    fn close_open_segments(&mut self) {
        for i in 0..self.channels as usize {
            if let Some(seg) = self.open[i].take() {
                self.valid[i].push(seg);
            }
        }
    }

    fn flush_pending_at(&mut self, at: TimePoint) {
        for c in 0..self.channels {
            let i = c as usize;
            if let Some((start, cause)) = self.pending[i].take() {
                let len = at.ticks.saturating_sub(start.ticks).max(0) as u64;
                self.channel_gaps.push(ChannelGap { channel: c, start, len, cause });
            }
        }
    }

    /// Closes the map. A carry that no delivered block follows is recorded as a
    /// zero-extent `Gap` at `expected_next`, carrying the cause derived from its
    /// flags, its `lost` where present, and its block count; the map's `end` does
    /// not move, because no sample after the last delivered one is accounted for.
    ///
    /// A channel that was valid somewhere in the map and is invalid at its end
    /// produces a `ChannelGap` running to the map's end; a channel that was never
    /// valid produces nothing, because never enabled is not a gap.
    ///
    /// Rule: SC-30c, SC-31c.
    pub fn finish(mut self, carry: DropCarry) -> ContinuityMap {
        let zero = TimePoint::new(self.domain, 0);
        let end = self.end.unwrap_or(zero);
        // What a contiguous push had to hold is carried forward, so the trailing Gap
        // takes it too — a stream whose last drop is followed by a contiguous block
        // and then ends has no other Gap to record it in.
        let carry = ContinuityBuilder::merge_carries(&self.pending_carry, &carry);
        if carry.blocks > 0 || !carry.flags.is_empty() || carry.lost.is_some() {
            self.gaps.push(Gap {
                // SC-30c: **zero-extent**, and `end` does not move, because no
                // sample after the last delivered one is accounted for. §6's
                // pseudocode writes `carry.lost or 0` for this length, which would
                // claim `lost` samples beyond the map's own end and contradict the
                // rule's own justification; the rule carries the ID, so it wins.
                // Raised for the owner as finding D15.
                start: self.expected_next.unwrap_or(end),
                len: 0,
                lost: carry.lost,
                cause: cause_from_carry(&carry),
                link_dropped: carry.blocks,
            });
        }
        for c in 0..self.channels {
            let i = c as usize;
            if let Some((start, cause)) = self.pending[i].take() {
                let len = end.ticks.saturating_sub(start.ticks).max(0) as u64;
                self.channel_gaps.push(ChannelGap { channel: c, start, len, cause });
            }
            if let Some(seg) = self.open[i].take() {
                self.valid[i].push(seg);
            }
        }
        ContinuityMap {
            domain: self.domain,
            channels: self.channels,
            valid: self.valid,
            gaps: self.gaps,
            channel_gaps: self.channel_gaps,
            first: self.first.unwrap_or(zero),
            end,
        }
    }
}

/// A jump computation that leaves the representable range is a time error, never a
/// panic (SC-30, TM-8).
impl From<TimeError> for StreamError {
    fn from(e: TimeError) -> StreamError {
        StreamError::Time(e)
    }
}
