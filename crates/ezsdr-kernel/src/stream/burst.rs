//! The transmit burst state machine and the late policy —
//! `02-stream-contract.md` SC-23…SC-29a (Vision §22, §23 TX rules 1–5).

use serde::{Deserialize, Serialize};

use super::{BlockFlags, BlockHeader, StreamError};
use crate::id::ClockDomainId;
use crate::time::{ClockRegistry, Converted, Duration, TimeError, TimePoint};

/// What a transmit target was admitted as (SC-23a).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AdmittedTarget {
    /// The target on the transmit stream's sample grid (SC-23a).
    pub target: TimePoint,
    /// What the caller asked for, recorded only when the applied target differs —
    /// that is, when the conversion was inexact and the target was advanced (SC-23a).
    pub requested_target: Option<TimePoint>,
}

/// Converts a burst target onto the transmit stream's SampleClock.
///
/// A target expressed in another **exactly related** domain is converted with
/// `apply` (TM-4); an `Inexact` result is advanced to the next transmit sample
/// instant, `floor + 1`, and both times are recorded. Refusal would be wrong, not
/// strict: TM-13b makes each stream's origin its own first sample, so a Reactor
/// computing `target = rx_time + turnaround` in the receive domain — the natural
/// computation, and the one Vision §56 and §58 #9 require — would otherwise be
/// refused on almost every burst. Advancing is the physically correct answer,
/// because a radio cannot transmit between samples.
///
/// A target in an **unrelated** domain is refused: TM-5 makes such a conversion
/// uncertain, and an uncertain transmit instant is not a transmit instant.
///
/// Rule: SC-23a, SC-23b.
pub fn admit_burst_target(
    registry: &ClockRegistry,
    target: TimePoint,
    tx_domain: ClockDomainId,
) -> Result<AdmittedTarget, StreamError> {
    if target.domain == tx_domain {
        return Ok(AdmittedTarget { target, requested_target: None,
        });
    }
    // SC-23b: `Unrelated` propagates out of `conversion`, which is the refusal.
    match registry.conversion(target.domain, tx_domain)?.apply(target)? {
        Converted::Exact { point } => Ok(AdmittedTarget { target: point, requested_target: None,
        }),
        Converted::Inexact { floor, .. } => {
            let next = floor.checked_add(Duration::new(tx_domain, 1))?;
            Ok(AdmittedTarget { target: next, requested_target: Some(target),
            })
        }
    }
}

/// What to do about a burst whose lead is short (SC-27, Vision §22).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum LatePolicy {
    /// Legal only for a burst whose target is statically known; `validate()`
    /// rejects it on a runtime-decided burst (SC-27).
    RejectAtPlan,
    /// Transmit as soon as possible and flag it (SC-27).
    SendAsapAndFlag,
    /// Drop the burst and flag it (SC-27).
    DropAndFlag,
}

/// The verdict on a burst's lead (SC-27).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LateOutcome {
    /// `target − now ≥ min_lead`.
    OnTime {},
    /// Transmit immediately; the burst is this late.
    SendAsap {
        /// How far short of `min_lead` the lead fell, in `host.monotonic`.
        late_by: Duration,
    },
    /// Drop the burst; it is this late.
    Drop {
        /// How far short of `min_lead` the lead fell, in `host.monotonic`.
        late_by: Duration,
    },
    /// A `RejectAtPlan` burst reached the runtime late. The Provider must not
    /// transmit it (SC-27).
    PlanViolation {
        /// How far short of `min_lead` the lead fell, in `host.monotonic`.
        late_by: Duration,
    },
}

impl LatePolicy {
    /// Yields `OnTime` when `target − now ≥ min_lead`, and otherwise the outcome
    /// this policy names.
    ///
    /// `target` and `now` are in one domain and their difference is a `Duration`
    /// there (TM-7). `min_lead` comes from the bound Provider's TimingEnvelope, a
    /// static Vocabulary document that cannot name a domain created at `prepare`,
    /// so like a `RelativeBudget` it is declared in `host.monotonic`, and TM-21
    /// compares the two exactly by cross-multiplication with no rescale and no
    /// rounding. Exactness matters in both directions: a check looser than the
    /// hardware is a defect, and one needlessly stricter rejects bursts the device
    /// would have sent.
    ///
    /// `late_by` is reported in `host.monotonic`, the domain `min_lead` is declared
    /// in. The lead is rescaled there with TM-9 and floored, so `late_by` errs
    /// towards reporting slightly more lateness, never less.
    ///
    /// Rule: SC-27, TM-21.
    pub fn decide(
        self,
        registry: &ClockRegistry,
        target: TimePoint,
        now: TimePoint,
        min_lead: Duration,
    ) -> Result<LateOutcome, TimeError> {
        let host = ClockDomainId::HOST_MONOTONIC;
        // SC-27 declares `min_lead` in `host.monotonic`; checked up front so that
        // the same misuse is not silently tolerated on the on-time path and
        // rejected on the late one.
        let min_lead_ticks = min_lead.ticks_in(host)?;
        let lead = target.checked_sub(now)?;
        if registry.compare_durations(lead, min_lead)?.is_ge() {
            return Ok(LateOutcome::OnTime {});
        }
        let lead_host = registry.rescale(lead, host)?.floor();
        let late_by = Duration::new(
            host,
            min_lead_ticks.checked_sub(lead_host.ticks).ok_or(TimeError::Overflow)?,
        );
        Ok(match self {
            LatePolicy::RejectAtPlan => LateOutcome::PlanViolation { late_by },
            LatePolicy::SendAsapAndFlag => LateOutcome::SendAsap { late_by },
            LatePolicy::DropAndFlag => LateOutcome::Drop { late_by },
        })
    }
}

/// The three `BurstRecord` fields no block header carries, supplied by the Provider
/// on the block that opens a burst. Left unsupplied, `wraps` would ship as a
/// constant zero and the v3 wrap-continuity regression of Vision §61 and §58 #15
/// would have nothing to assert against.
///
/// Rule: SC-29a.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct BurstOpen {
    /// The repeated waveform's length in samples, without which `wraps` cannot be
    /// counted (SC-26, SC-29a).
    pub waveform_len: Option<u32>,
    /// What [`LatePolicy::decide`] returned for this burst (SC-27, SC-29a).
    pub late: Option<LateOutcome>,
    /// What the caller asked for, when [`admit_burst_target`] advanced the target
    /// onto the sample grid. SC-23a requires **both** times in the record, and the
    /// tracker cannot derive this one from a header any more than it can derive
    /// `waveform_len`, so it travels on the same block (SC-23a, SC-29a).
    pub requested_target: Option<TimePoint>,
}

/// How a burst ended (SC-28).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum BurstEnd {
    /// A block carrying `END_OF_BURST` (SC-24).
    Eob,
    /// The Run stopped (SC-25).
    Stop,
    /// A time jump or a nested start of burst closed it (SC-24).
    Discontinuity,
}

/// What was transmitted, one per burst. The Provider puts these in its Manifest
/// section (SC-28).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BurstRecord {
    /// The applied target on the sample grid (SC-23a).
    pub target: TimePoint,
    /// What the caller asked for, when it differed (SC-23a).
    pub requested_target: Option<TimePoint>,
    /// Device feedback, where the Provider can supply it (SC-28, SC-29a).
    pub actual_start: Option<TimePoint>,
    /// Blocks transmitted in this burst (SC-28).
    pub blocks: u32,
    /// Samples transmitted in this burst (SC-28).
    pub samples: u64,
    /// Complete repetitions of the waveform, `samples / waveform_len` (SC-26, SC-29a).
    pub wraps: u32,
    /// Lateness, as the burst's late policy decided it (SC-27, SC-29a).
    pub late_by: Option<Duration>,
    /// How it ended (SC-28).
    pub end: BurstEnd,
}

/// The tracker's public state (SC-24).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BurstState {
    /// No burst is open.
    Idle,
    /// A burst is open.
    InBurst {
        /// The burst's target (SC-23).
        target: TimePoint,
        /// Where the next block must begin (SC-24).
        expected_next: TimePoint,
        /// Blocks so far (SC-28).
        blocks: u32,
        /// Samples so far (SC-28).
        samples: u64,
    },
}

/// What one block did to the state machine (SC-24).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BurstStep {
    /// The block opened a burst.
    Started,
    /// The block continued the open burst contiguously.
    Continued,
    /// The block closed the burst.
    Ended {
        /// The completed record (SC-28).
        record: BurstRecord,
    },
    /// The block's time did not match, or it opened a burst inside one: the open
    /// burst is closed and reported, and this block becomes the first block of a
    /// new one. Nothing is padded. The Provider emits `TX_DISCONTINUITY` and
    /// evaluates SC-27 for the new burst (SC-24, SC-24a).
    Discontinuity {
        /// Where the block should have started.
        expected: TimePoint,
        /// Where it did start.
        got: TimePoint,
        /// The burst that was closed (SC-28).
        closed: BurstRecord,
        /// The burst this block **opened**, when the same block also carried
        /// `END_OF_BURST` and so ended it at once. Without it a block that did two
        /// things reported one, and a caller tracking state from the return value
        /// believed a burst was open while `state()` said `Idle` (SC-24, SC-28).
        then_ended: Option<BurstRecord>,
    },
}

struct Open {
    target: TimePoint,
    requested_target: Option<TimePoint>,
    expected_next: TimePoint,
    blocks: u32,
    samples: u64,
    waveform_len: Option<u32>,
    late_by: Option<Duration>,
    actual_start: Option<TimePoint>,
}

impl Open {
    fn close(self, end: BurstEnd) -> BurstRecord {
        BurstRecord {
            target: self.target,
            requested_target: self.requested_target,
            actual_start: self.actual_start,
            blocks: self.blocks,
            samples: self.samples,
            // SC-26, SC-29a: complete repetitions of the declared waveform.
            wraps: self
                .waveform_len
                .filter(|l| *l > 0)
                .map_or(0, |l| (self.samples / l as u64).min(u32::MAX as u64) as u32),
            late_by: self.late_by,
            end,
        }
    }
}

/// The single implementation of the transmit burst state of SC-24 to SC-28. Every
/// Provider — Mock and UHD alike (evidence, not a dependency: OV-23a) — routes
/// its transmit blocks through it, so that
/// burst semantics cannot diverge between them.
///
/// The tracker owns the state, the contiguity arithmetic, and the record fields it
/// can derive from block headers alone. The Provider owns device input and output,
/// including the zero-length end-of-burst send that closes a burst on UHD (OV-23a) as v3
/// did, the `now` and `min_lead` inputs, executing the late policy, and emitting
/// events.
///
/// Rule: SC-29, SC-29a.
pub struct BurstTracker {
    domain: ClockDomainId,
    open: Option<Open>,
}

impl BurstTracker {
    /// A tracker for one transmit SampleClock (SC-24, SC-29).
    pub fn new(domain: ClockDomainId) -> BurstTracker {
        BurstTracker { domain, open: None }
    }

    /// The current state (SC-24).
    pub fn state(&self) -> BurstState {
        match &self.open {
            None => BurstState::Idle,
            Some(o) => BurstState::InBurst {
                target: o.target,
                expected_next: o.expected_next,
                blocks: o.blocks,
                samples: o.samples,
            },
        }
    }

    /// Feeds one transmit block through the state machine of `02-stream-contract.md` §6.
    ///
    /// `open` must be `Some` exactly when the block carries `START_OF_BURST`: that
    /// is the block on which the Provider supplies `waveform_len` and the late
    /// outcome (SC-29a). A burst opened by a *discontinuity* carries neither, so
    /// its `wraps` is 0 — a discontinuity is an error path the Provider did not
    /// plan for.
    ///
    /// Rule: SC-24, SC-25, SC-26, SC-29, SC-29a.
    pub fn on_block(
        &mut self,
        h: &BlockHeader,
        open: Option<BurstOpen>,
    ) -> Result<BurstStep, StreamError> {
        if h.first_sample_time.domain != self.domain {
            return Err(StreamError::Time(TimeError::DomainMismatch {
                expected: self.domain,
                found: h.first_sample_time.domain,
            }));
        }
        let sob = h.flags.contains(BlockFlags::START_OF_BURST);
        let eob = h.flags.contains(BlockFlags::END_OF_BURST);
        if sob != open.is_some() {
            return Err(StreamError::InvalidBlock {
                reason: "a BurstOpen accompanies exactly the block that carries START_OF_BURST"
                    .to_owned(),
            });
        }
        let t = h.first_sample_time;
        let end = h.end_time()?;

        match self.open.take() {
            None => {
                if !sob {
                    return Err(StreamError::MissingStartOfBurst);
                }
                self.begin(t, end, h.len, open.unwrap_or_default());
                if eob {
                    let record = self.open.take().expect("just opened").close(BurstEnd::Eob);
                    return Ok(BurstStep::Ended { record });
                }
                Ok(BurstStep::Started)
            }
            Some(mut o) => {
                let contiguous = t == o.expected_next;
                if contiguous && !sob {
                    // ponytail: saturating rather than checked. SC-25 makes
                    // continuous transmission one burst, so `blocks` is unbounded by
                    // design; at 20 Msps and ~2 000 samples a block that is about
                    // five days before it matters, and a saturated count is a
                    // wrong number where a wrapped one is a wrong number that looks
                    // right. Widen to u64 if a Run ever runs that long.
                    o.blocks = o.blocks.saturating_add(1);
                    o.samples += h.len as u64;
                    o.expected_next = end;
                    self.open = Some(o);
                    if eob {
                        let record = self.open.take().expect("open").close(BurstEnd::Eob);
                        return Ok(BurstStep::Ended { record });
                    }
                    return Ok(BurstStep::Continued);
                }
                // SC-24: a time jump in either direction, or a start of burst while
                // a burst is open, closes the burst and opens a new one here.
                let expected = o.expected_next;
                let closed = o.close(BurstEnd::Discontinuity);
                self.begin(t, end, h.len, open.unwrap_or_default());
                let then_ended = if eob {
                    let record = self.open.take().expect("just opened").close(BurstEnd::Eob);
                    Some(record)
                } else {
                    None
                };
                Ok(BurstStep::Discontinuity { expected, got: t, closed, then_ended,
                })
            }
        }
    }

    fn begin(&mut self, t: TimePoint, end: TimePoint, len: u32, open: BurstOpen) {
        let late_by = match open.late {
            Some(LateOutcome::SendAsap { late_by })
            | Some(LateOutcome::Drop { late_by })
            | Some(LateOutcome::PlanViolation { late_by }) => Some(late_by),
            _ => None,
        };
        self.open = Some(Open {
            target: t,
            requested_target: open.requested_target,
            expected_next: end,
            blocks: 1,
            samples: len as u64,
            waveform_len: open.waveform_len,
            late_by,
            actual_start: None,
        });
    }

    /// The late outcome of the open burst, for a burst a **discontinuity** opened.
    /// Such a burst carries no `START_OF_BURST` and so no [`BurstOpen`], but SC-24a
    /// still requires the Provider to evaluate SC-27's policy for it; this is the
    /// second path SC-29a names for that one field (SC-24a, SC-27, SC-29a).
    pub fn set_late(&mut self, late: LateOutcome) {
        if let Some(o) = self.open.as_mut() {
            o.late_by = match late {
                LateOutcome::SendAsap { late_by }
                | LateOutcome::Drop { late_by }
                | LateOutcome::PlanViolation { late_by } => Some(late_by),
                LateOutcome::OnTime {} => None,
            };
        }
    }

    /// Device feedback on where the burst actually started, when the Provider has
    /// it (SC-28, SC-29a).
    pub fn set_actual_start(&mut self, t: TimePoint) {
        if let Some(o) = self.open.as_mut() {
            o.actual_start = Some(t);
        }
    }

    /// Closes an open burst because the Run stopped. Continuous transmission is one
    /// burst, ended by the Run's stop or by an explicit `END_OF_BURST` (SC-25).
    pub fn stop(&mut self) -> Option<BurstRecord> {
        let record = self.open.take()?.close(BurstEnd::Stop);
        Some(record)
    }
}
