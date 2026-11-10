//! RM-21, RM-27 computed once (note 47, stage 3): which receive value a Provider accepts, and
//! the configuration a `start_rx` takes. Pure, like [`crate::timeline`]: MockRadio and the UHD
//! Module each hold one and only wire it (event emission, snapshot storage), so the rule is
//! written once and the differential harness (VH) proves the wiring.
//!
//! "Streaming" is not kept here. It is the stream's line after replay ([`crate::timeline::Line::streaming`]),
//! which each Provider passes in as a closure, so the fact has one representation.

use std::collections::{BTreeMap, VecDeque};

use ezsdr_kernel::event::ActionId;
use ezsdr_kernel::spec::{Key, Scalar, Value};
use ezsdr_kernel::stream::LatePolicy;
use ezsdr_kernel::time::TimePoint;

use crate::{CancelReason, Concrete, concrete};

/// Why a receive value is not staged (RM-21). Each cause has its own text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The update carries an instant: a staged value takes effect at the next start, never at an instant.
    Instant,
    /// The key is not a receive channel's frequency, gain or antenna.
    NotStaged,
    /// The channel is in no stream: nothing would ever use the value.
    NoStream(u32),
    /// The channel's own stream is streaming.
    Streaming {
        /// The channel's number.
        channel: u32,
        /// Its stream.
        stream: u32,
    },
    /// A `start_rx` for a stream that is streaming: a booked start not yet begun counts, and it is never revised to an earlier origin.
    StartWhileStreaming(u32),
    /// A transmit stream that its bookings or a repeat keep busy (see [`Bookings`]).
    TxBusy {
        /// Until when the stream is busy.
        until: Until,
    },
}

impl Refusal {
    /// The reason, without a rule prefix: the rule and the cause (RM-21).
    pub fn reason(&self) -> String {
        match self {
            Refusal::Instant => "RM-21: a staged value takes effect at the next start, never at an instant".to_owned(),
            Refusal::NotStaged => "RM-21: only a channel's frequency, gain or antenna is staged".to_owned(),
            Refusal::NoStream(channel) => format!("RM-21: receive channel {channel} is in no stream, so no start would use the value"),
            Refusal::Streaming { channel, stream } => format!("RM-21: receive channel {channel}'s stream {stream} is streaming; send Stop first, then the value, then start_rx"),
            Refusal::StartWhileStreaming(stream) => format!("RM-21: stream {stream} is streaming (a booked start counts); send Stop first, then start_rx"),
            Refusal::TxBusy { until: Until::At(end) } => format!("RM-21: the transmit stream is busy until tick {end}; a value is taken once its last booked burst has ended"),
            Refusal::TxBusy { until: Until::Open } => "RM-21: the transmit stream is repeating; send Stop, then the value once the in-flight window has passed".to_owned(),
        }
    }
}

/// When a transmit stream stays busy (RM-21): at an exclusive end in root ticks, or with no end
/// until a `Stop` cuts a repeat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Until {
    /// The exclusive end, in root ticks.
    At(i64),
    /// A repeat that no `Stop` has cut yet.
    Open,
}

/// Why a booking is refused at receipt (RM-15): its region meets one already accepted, or it starts before one the
/// owner may already have handed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlap {
    /// Its region meets an accepted booking's region, or the Stop's region (RM-16).
    Booking,
    /// It starts before an accepted chain that may already be handed to the device ([`Bookings::may_be_handed`]).
    Handed,
}

/// RM-15: the longest standalone head that is one closed device burst (its front is then its padded end plus [`closed_short_gap`]).
pub const CLOSED_SHORT_MAX: i64 = 32;

/// RM-15, UR-22: the samples past any closing send's exclusive end that the next device burst keeps (every close but a short head's), and
/// the floor of [`closed_short_gap`].
pub const CLOSE_GAP: i64 = 3;

/// RM-15: a head longer than 32 samples, or a joined piece, closes with its held tail and at most one zero, so its closing end is its payload
/// end plus one and its front that plus [`CLOSE_GAP`].
pub const HELD_TAIL_FRONT: i64 = 1 + CLOSE_GAP;

/// RM-16: a Stop's two-zero close starts at the cursor and ends two samples later; the front is that end plus [`CLOSE_GAP`].
pub const STOP_FRONT: i64 = 2 + CLOSE_GAP;

/// RM-15: the samples past a closed short device burst's padded end that the next device burst keeps: `max(3, ceil(15 µs × rate))`, the
/// stream's sample rate being `root_hz × ticks_den / ticks_num` (`ticks_num / ticks_den` root ticks per sample). Integer arithmetic.
pub fn closed_short_gap(root_hz: u64, ticks_num: u64, ticks_den: u64) -> i64 {
    let (num, den) = (u128::from(root_hz) * u128::from(ticks_den) * 15, u128::from(ticks_num.max(1)) * 1_000_000);
    i64::try_from(num.div_ceil(den)).unwrap_or(i64::MAX).max(CLOSE_GAP)
}

/// One accepted booking's span on the stream's sample clock, and what may continue it (RM-15).
#[derive(Clone, PartialEq, Eq, Debug)]
struct Chain {
    /// The head's first sample.
    start: i64,
    /// The last piece's exclusive end; `i64::MAX` for a repeat. A Stop caps `front` only: `end` is read
    /// only with `join`, which a Stop clears.
    end: i64,
    /// The first samples of the joined pieces, in order: each is a join point the owner must not close at (RM-16). Empty
    /// for a head alone.
    joined: Vec<i64>,
    /// Exclusive: no other booking may start in `[start, front)`.
    front: i64,
    /// A booking at exactly `end` continues the chain.
    join: bool,
}

impl Chain {
    /// The last piece's first sample: the head's, or the newest joined piece's.
    fn last(&self) -> i64 {
        self.joined.last().copied().unwrap_or(self.start)
    }

    /// The head's exclusive end: the first joined piece's start, or the head's own end when no piece is joined.
    fn head_end(&self) -> i64 {
        self.joined.first().copied().unwrap_or(self.end)
    }

    /// A head of at most 32 samples before its first joined piece (RM-15). A repeat is never one (its end is `i64::MAX`). Pieces
    /// joined behind it do not lengthen it: the owner may have sent it as one closed device burst.
    fn short_head(&self) -> bool {
        self.head_end().checked_sub(self.start).is_some_and(|len| len <= CLOSED_SHORT_MAX)
    }

    /// The front a short head's region keeps at a Stop (RM-16): each piece of at most 32 samples that starts below `handed` fronts at its
    /// own standalone front, and a longer piece behind the head, which the owner never sends, does not extend it.
    fn own_fronts(&self, handed: i64, gap: Option<i64>) -> i64 {
        let starts = std::iter::once(self.start).chain(self.joined.iter().copied());
        let ends = self.joined.iter().copied().chain(std::iter::once(self.end));
        starts.zip(ends).filter(|&(s, e)| s < handed && e.checked_sub(s).is_some_and(|len| len <= CLOSED_SHORT_MAX)).filter_map(|(s, e)| front(gap, s, Some(e - s), false)).max().unwrap_or(i64::MIN)
    }

    /// A standalone short head: one closed device burst (RM-15), with no piece joined behind it.
    fn closed_short(&self) -> bool {
        self.joined.is_empty() && self.short_head()
    }
}

/// A repeat switch accepted and not yet applied (RM-37, note 47 §1.2): the Action that asked for it and
/// the sample at which the new waveform starts. The boundary is chosen at receipt; the owner moves it
/// when its cursor has passed it (UR-38).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Switch {
    /// The Action that asked for the switch.
    pub id: ActionId,
    /// The origin and period of the repeat it replaces: the grid a moved boundary stays on.
    pub grid: (i64, i64),
    /// The first sample of the new waveform, on the stream's clock.
    pub boundary: i64,
    /// Whether the owner has taken the new waveform for the device (UR-36): a switch the owner has not taken is not applied.
    pub installed: bool,
    /// The boundary the owner moved the switch to under its late policy (UR-38), or `None`.
    pub moved_to: Option<i64>,
}

/// A switch the record settled (RM-37): its outcome, and the boundary it was moved to, if it was (UR-38). The Provider writes
/// both into the switch's row in one place ([`write_settled`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settled {
    /// The Action that asked for the switch.
    pub id: ActionId,
    /// How it was settled.
    pub outcome: SwitchOutcome,
    /// The boundary the owner moved it to under its late policy, or `None`.
    pub moved_to: Option<i64>,
}

/// How a switch was settled: applied at its boundary, or cancelled by a Stop before it, or refused because the owner had no handed
/// cursor to take it at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchOutcome {
    /// The old repeat ended at the boundary and the new one began there.
    Applied,
    /// A Stop reached the boundary first, so the new waveform never started.
    Cancelled,
    /// The owner had no handed cursor to take the switch at (the repeat it replaces left the record first): the old repeat keeps playing.
    Refused,
}

impl SwitchOutcome {
    /// The outcome's name in the `switches` section.
    pub fn label(self) -> &'static str {
        match self {
            SwitchOutcome::Applied => "applied",
            SwitchOutcome::Cancelled => "cancelled",
            SwitchOutcome::Refused => "refused_past_boundary",
        }
    }
}

/// One row of a Provider's `switches` section (RM-37): a repeat switch and how it was settled.
#[derive(serde::Serialize)]
pub struct SwitchRecord {
    /// The id the switch was dispatched with.
    pub action_id: ActionId,
    /// The id of the repeat it replaces.
    pub predecessor: ActionId,
    /// The receipt's instant.
    pub selected_at: TimePoint,
    /// The period of the repeat it replaces.
    pub old_period: i64,
    /// The period of the new waveform.
    pub new_period: i64,
    /// The boundary chosen at receipt, on the stream's clock (RM-37).
    pub boundary: i64,
    /// The boundary the owner moved the switch to under its late policy (UR-38), or `None`.
    pub moved_to: Option<i64>,
    /// `pending` until the record settles it: [`SwitchOutcome::label`].
    pub outcome: &'static str,
}

/// One receipt on a transmit stream's list, not yet read by the executor.
enum Arrival<P> {
    Booking(P),
    Halt(CancelReason),
}

/// What the executor reads next from a [`TxList`].
pub enum Read<P> {
    /// An accepted booking: the executor holds it in [`TxList::held`], or refuses it.
    Booking(P),
    /// A `Stop`, `Provider::stop` or a loss: `cancelled` is what `held` held when the halt was read, taken whole.
    Halt {
        /// Why the bookings are cancelled.
        reason: CancelReason,
        /// The bookings read before the halt and not yet taken, by first sample.
        cancelled: BTreeMap<i64, P>,
    },
}

/// The accepted bookings and halts of one transmit stream in receipt order (RM-15, RM-16), beside
/// [`Bookings`] and under the same lock. Receipt appends in the critical section that decides; the
/// executor reads one entry at a time, so a halt cancels exactly the bookings read before it and a booking
/// received after it survives.
pub struct TxList<P> {
    unread: VecDeque<Arrival<P>>,
    /// Bookings the executor has read and not yet taken, by first sample. Only the executor writes it.
    pub held: BTreeMap<i64, P>,
}

// By hand: a derive would require `P: Default`.
impl<P> Default for TxList<P> {
    fn default() -> Self {
        TxList { unread: VecDeque::new(), held: BTreeMap::new() }
    }
}

impl<P> TxList<P> {
    /// Receipt: an accepted booking, in the critical section that accepted it.
    pub fn book(&mut self, booking: P) {
        self.unread.push_back(Arrival::Booking(booking));
    }

    /// Receipt: a halt, in the critical section that decided it.
    pub fn halt(&mut self, reason: CancelReason) {
        self.unread.push_back(Arrival::Halt(reason));
    }

    /// The executor: the oldest receipt it has not read.
    pub fn read(&mut self) -> Option<Read<P>> {
        Some(match self.unread.pop_front()? {
            Arrival::Booking(booking) => Read::Booking(booking),
            Arrival::Halt(reason) => Read::Halt { reason, cancelled: std::mem::take(&mut self.held) },
        })
    }

    /// Whether a receipt waits to be read.
    pub fn unread(&self) -> bool {
        !self.unread.is_empty()
    }

    /// Whether a halt waits to be read: the executor must read it before it hands anything further.
    pub fn halt_unread(&self) -> bool {
        self.unread.iter().any(|arrival| matches!(arrival, Arrival::Halt(_)))
    }
}

/// The transmit bookings of one stream (RM-15, RM-16, RM-21), written once here; each Provider
/// keeps one per transmit stream. Two representations of the same accepted bookings: `until`
/// is the value rule in root ticks (today's `Busy`, unchanged), and the chains and the stop
/// region are the overlap rule in sample indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookings {
    /// The samples a closed short head's front keeps past its padded end ([`closed_short_gap`]); none until the stream's clock is known
    /// ([`Bookings::arm`]): an unarmed record refuses a finite short head rather than book it with no guard.
    gap: Option<i64>,
    until: Option<Until>,
    chains: Vec<Chain>,
    region: Option<(i64, i64)>,
    pending: Option<Switch>,
    settled: Vec<Settled>,
}

impl Bookings {
    /// A record whose stream's rate is not known yet: it refuses a short head until [`Bookings::arm`]. The value for a stream that books nothing.
    pub fn unarmed() -> Bookings {
        Bookings { gap: None, until: None, chains: Vec::new(), region: None, pending: None, settled: Vec::new() }
    }

    /// Sets the samples a closed short head's front keeps past its padded end, [`closed_short_gap`] of the stream's clock. The rate is fixed
    /// for a Run (RM-27), so this is called where the stream's clock is registered; a later call replaces the gap.
    pub fn arm(&mut self, gap: i64) {
        self.gap = Some(gap);
    }

    /// A booking accepted, after any booking-time move: its exclusive end in root ticks, or `None`
    /// for a repeat. The later end counts, and an open repeat stays open.
    pub fn book(&mut self, end: Option<i64>) {
        self.until = match (self.until, end) {
            (Some(Until::Open), _) | (_, None) => Some(Until::Open),
            (Some(Until::At(old)), Some(end)) => Some(Until::At(old.max(end))),
            (None, Some(end)) => Some(Until::At(end)),
        };
    }

    /// A `Stop`, a loss or `Provider::stop` taking effect, with `bound` its [`stop_bound`] in root
    /// ticks (RM-16): the stream is busy until `bound` at the latest, and no later than a booked end
    /// already there. An idle stream stays idle.
    pub fn stop(&mut self, bound: i64) {
        self.until = match self.until {
            None => None,
            Some(Until::Open) => Some(Until::At(bound)),
            Some(Until::At(end)) => Some(Until::At(end.min(bound))),
        };
    }

    /// RM-21: a value received at `receipt` is staged only when the stream is not busy, that is
    /// when its last booked end is at or before the receipt.
    pub fn stageable(&self, receipt: i64) -> Result<(), Refusal> {
        match self.until {
            None => Ok(()),
            Some(Until::At(end)) if end <= receipt => Ok(()),
            Some(until) => Err(Refusal::TxBusy { until }),
        }
    }

    /// RM-15 (the receipt rule): a booking of `len` samples at `start` (`None` for a repeat),
    /// received at root tick `now`. `join_by(e)` is the root tick by which a booking at `e` must be
    /// received to continue a chain that ends at `e`. A booking whose region meets an accepted one
    /// is refused, so the one received second loses; a contiguous booking in time joins its chain.
    /// `horizon` is [`handover_horizon`] at `now`: a booking that starts before an accepted chain
    /// the owner may already have handed ([`Bookings::may_be_handed`]) is refused too. The caller has run
    /// [`Bookings::prune`] at this receipt: a switch due by `now` is applied by then, and `accept` reads only the chains and the region.
    pub fn accept(&mut self, start: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Result<(), Overlap> {
        let join = self.joinable(start, len, now, &join_by);
        let front = front(self.gap, start, len, join.is_some()).ok_or(Overlap::Booking)?;
        let early = self.chains.iter().any(|c| start < c.start && self.may_be_handed(c, now, horizon, &join_by));
        if self.meets(start, front, join) {
            return Err(Overlap::Booking);
        }
        if early {
            return Err(Overlap::Handed);
        }
        let end = len.map_or(i64::MAX, |len| start + len);
        match join {
            Some(i) => {
                let chain = &mut self.chains[i];
                chain.end = end;
                chain.front = front;
                chain.joined.push(start);
            }
            None => self.chains.push(Chain { start, end, joined: Vec::new(), front, join: len.is_some_and(|len| len > CLOSED_SHORT_MAX) }),
        }
        Ok(())
    }

    /// RM-16 (the owner's close, UR-23): whether a booking can still continue the chain at sample `e`, read at root tick `now`:
    /// a piece joined at `e` is accepted and not yet sent (the head's own start counts while no piece is joined), or a chain with
    /// `join` ends at `e` and `now` is at or before `join_by(e)`.
    pub fn continuation_open(&self, e: i64, now: i64, join_by: impl Fn(i64) -> i64) -> bool {
        self.chains.iter().any(|c| c.join && ((c.joined.is_empty() && c.start == e) || c.joined.contains(&e) || (c.end == e && now <= join_by(e))))
    }

    /// RM-16 (the owner's close at `e`): the chain that ends at `e` with `join` no longer takes a booking, so one received later is refused by [`Bookings::accept`]. Returns its front.
    pub fn close(&mut self, e: i64) -> Option<i64> {
        let chain = self.chains.iter_mut().find(|c| c.join && c.end == e)?;
        chain.join = false;
        Some(chain.front)
    }

    /// RM-15, RM-16: the first sample at or after `k` that no accepted region holds, except a join
    /// point: a booking at the end of a chain with `join` may continue it if received at `now`,
    /// that is `now <= join_by(k)`. Only a booking with a length (`joins`) can continue a chain.
    /// A sample before a chain the owner may already have handed (see [`Bookings::accept`]) is not
    /// free either, so the scan moves past that chain. `None` when an open repeat holds every sample from there on.
    pub fn first_free(&self, mut k: i64, joins: bool, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
        while let Some(front) = self.held_until(k, joins, now, &join_by, horizon) {
            if front == i64::MAX {
                return None;
            }
            k = front;
        }
        Some(k)
    }

    /// RM-15 (note 47 stage 5, §2): the first sample at or after `from` at which a booking of `len` fits: [`Bookings::first_free`], then
    /// its full front tested against the accepted regions (a joined end excepted), jumping past each region the front meets, until none
    /// is met. The one placement rule. `None` only when an open repeat holds the rest of the stream or the front does not fit the clock.
    pub fn place(&self, mut from: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
        loop {
            let k = self.first_free(from, len.is_some(), now, &join_by, horizon)?;
            let join = self.joinable(k, len, now, &join_by);
            match self.met_front(k, front(self.gap, k, len, join.is_some())?, join) {
                Some(i64::MAX) => return None,
                Some(next) => from = next,
                None => return Some(k),
            }
        }
    }

    /// RM-15 (note 47 stage 5, §2): where an untimed booking received at `now` starts: from `max(at, ordinary)`, where `at` is the
    /// Kernel's instant and `ordinary` the earliest sample the device can take at receipt, placed by [`Bookings::place`]. The one
    /// untimed decision for both Providers, which book it with [`Bookings::accept`] after their own checks; the late policy is not
    /// consulted. `None` only when an open repeat holds the rest of the stream or the front does not fit the clock.
    pub fn place_untimed(&self, at: i64, ordinary: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
        self.place(at.max(ordinary), len, now, join_by, horizon)
    }

    /// RM-16 (F36, note 47 stage 5, §2): a timed booking at `at` whose region meets only the Stop region moves to the first sample
    /// [`Bookings::place`] finds past it. One that meets an accepted chain stays at `at`, so [`Bookings::accept`] refuses it. `Err`
    /// when no sample is free.
    pub fn region_move(&self, at: i64, len: Option<i64>, now: i64, join_by: impl Fn(i64) -> i64, horizon: i64) -> Result<i64, Overlap> {
        let join = self.joinable(at, len, now, &join_by);
        let front = front(self.gap, at, len, join.is_some()).ok_or(Overlap::Booking)?;
        if self.chain_front(at, front, join).is_some() || self.region_met(at, front).is_none() {
            return Ok(at);
        }
        self.place(at, len, now, join_by, horizon).ok_or(Overlap::Booking)
    }

    /// RM-15, RM-16 (note 47 stage 5, §2): moves the chain head `old` for an untimed burst at dispatch, to the first sample
    /// [`Bookings::place`] finds from `from`. The head's own span is removed before placing, so it does not block itself, and no
    /// hand-over horizon or join applies (the owner hands in start order, and nothing is received at dispatch). A joined piece that
    /// is not a head is refused (D24). Returns the new start, or `None` with the record unchanged.
    pub fn move_untimed(&mut self, old: i64, from: i64, len: Option<i64>) -> Option<i64> {
        self.move_head(old, len, |bookings| bookings.place(from, len, i64::MAX, |_| i64::MIN, i64::MIN))
    }

    /// Advances the record to the receipt's sample `now_k` (RM-13's switch, note 47 m3), then drops the
    /// chains and the region whose fronts are at or before `now_k`, the first sample at or
    /// after the receipt. A booking received from then on starts at or after `now_k` (an on-time
    /// start is at least the lead after its receipt, a late one moves to the ordinary bound), and
    /// its join deadline has passed for any chain that ends before `now_k`, so none of them blocks it.
    /// Every receipt calls it first, so a booking never sees a switch that its boundary has passed.
    pub fn prune(&mut self, now_k: i64) {
        self.advance(now_k);
        self.chains.retain(|c| c.front > now_k);
        // A join point the device has passed can no longer be continued; the newest stays, since `last` reads it.
        for c in &mut self.chains {
            let last = c.joined.last().copied();
            c.joined.retain(|&j| j > now_k || Some(j) == last);
        }
        if self.region.is_some_and(|(_, front)| front <= now_k) {
            self.region = None;
        }
    }

    /// RM-16 (a Stop taking effect at the sample `t`, with `stop` the first sample a booking may
    /// start at after it): every kept chain's front is capped at `stop` unless it is a short head
    /// (at most 32 samples before its first joined piece), which is capped at `stop` or its pieces' own
    /// fronts ([`Chain::own_fronts`]), whichever is later; no chain joins
    /// any more, and bookings starting at or after `t` are cancelled. A cancelled booking may already
    /// be handed, so a Stop that cancels one starting before `stop` adds the region `[t, stop)` (hulled
    /// with an earlier region), and a cancelled short head that starts below the handed cursor's bound
    /// (`stop - STOP_FRONT`) reaches the own fronts of its short pieces that start below that bound. A Stop that cancels nothing adds no region. A booking starting at or after
    /// `stop` was never handed (the handed cursor is at most `stop - STOP_FRONT`), so it adds nothing.
    pub fn halt(&mut self, t: i64, stop: i64) {
        let pending = self.pending.take();
        // The handed cursor is at most `handed` (RM-16): a cancelled short head that starts below it may have been handed
        // as one send, so the region reaches the own fronts of its short pieces below it (`own_fronts`), not only `stop`.
        let handed = stop - STOP_FRONT;
        let cut = self.chains.iter().filter(|c| c.start >= t && c.start < stop).map(|c| if c.start < handed && c.short_head() { c.own_fronts(handed, self.gap).max(stop) } else { stop }).max();
        self.chains.retain(|c| c.start < t);
        let gap = self.gap;
        for c in &mut self.chains {
            // A kept short head was handed as one send, which a Stop does not recall: it keeps its own front, not a longer piece's.
            c.front = c.front.min(if c.short_head() { stop.max(c.own_fronts(handed, gap)) } else { stop });
            c.join = false;
        }
        match pending {
            // A switch before the bound is applied, and its new repeat is cut at the bound like any other; the samples
            // handed within the in-flight window are not recalled (RM-16). One at or after the bound was never handed,
            // and one the owner has not taken is never handed.
            Some(switch) if switch.installed && switch.boundary < stop => self.apply(switch, stop),
            Some(switch) => self.settled.push(Settled { id: switch.id, outcome: SwitchOutcome::Cancelled, moved_to: switch.moved_to }),
            None => {}
        }
        if let Some(front) = cut {
            self.region = Some(match self.region {
                Some((start, region_front)) => (start.min(t), region_front.max(front)),
                None => (t, front),
            });
        }
    }

    /// RM-37 (the repeat switch): accepts, for the action `id`, a switch whose new waveform starts at `boundary`, asked for at the
    /// receipt's sample `now_k`, on the replaced repeat's `grid` (origin, period). Refused, and nothing is pending, unless a repeat
    /// is playing (its chain has no end and no front yet, and it started at or before `now_k`) and no other switch is pending. The
    /// caller chooses `boundary` with [`repeat_boundary`]. Returns whether the switch was accepted; the owner then takes it with
    /// [`Bookings::install`].
    pub fn switch(&mut self, id: ActionId, grid: (i64, i64), boundary: i64, now_k: i64) -> bool {
        self.advance(now_k);
        let playing = self.chains.iter().any(|c| c.end == i64::MAX && c.front == i64::MAX && c.start <= now_k);
        if self.pending.is_some() || !playing {
            return false;
        }
        self.pending = Some(Switch { id, grid, boundary, installed: false, moved_to: None });
        true
    }

    /// UR-36, UR-38 (RM-37): the owner takes the pending switch `id` at its handed cursor `cursor_k`. A cursor at or before the
    /// boundary installs it there. A later cursor moves it to the first boundary of its grid at or after the cursor (a switch is
    /// always SendAsapAndFlag, RM-13). Returns the boundary it is installed at, or `None` when `id` is not pending.
    pub fn install(&mut self, id: ActionId, cursor_k: i64) -> Option<i64> {
        let switch = self.pending.filter(|s| s.id == id && !s.installed)?;
        let (boundary, moved_to) = if cursor_k <= switch.boundary {
            (switch.boundary, None)
        } else {
            let moved = repeat_boundary(switch.grid.0, switch.grid.1, cursor_k);
            (moved, Some(moved))
        };
        self.pending = Some(Switch { boundary, installed: true, moved_to, ..switch });
        Some(boundary)
    }

    /// UR-36: the owner refuses the pending switch `id` that it cannot take, so the old repeat keeps playing and the row is settled
    /// as refused. Every owner-side refusal of a switch goes through here (S4).
    pub fn refuse(&mut self, id: ActionId) {
        if self.pending.is_some_and(|s| s.id == id && !s.installed) {
            self.pending = None;
            self.settled.push(Settled { id, outcome: SwitchOutcome::Refused, moved_to: None });
        }
    }

    /// RM-37: a Provider's loss or F8 recovery that ends the repeat cancels the pending switch. Returns whether one was pending.
    pub fn cancel_switch(&mut self) -> bool {
        match self.pending.take() {
            Some(switch) => {
                self.settled.push(Settled { id: switch.id, outcome: SwitchOutcome::Cancelled, moved_to: switch.moved_to });
                true
            }
            None => false,
        }
    }

    /// RM-37 (S4): the owner abandons its repeat at the handed cursor `cursor_k`. A pending switch whose boundary is at or after the cursor
    /// never starts, so it is cancelled rather than applied by the Stop that ends the repeat. Returns whether it cancelled one.
    pub fn cancel_switch_from(&mut self, cursor_k: i64) -> bool {
        let cancel = self.pending.is_some_and(|switch| switch.boundary >= cursor_k);
        if cancel {
            self.cancel_switch();
        }
        cancel
    }

    /// RM-37 (S4): the switch `id` the record applied, whose new waveform the owner never sent, is settled cancelled. The entry is
    /// changed only while it is still unwritten to the switch rows.
    pub fn cancel_applied(&mut self, id: ActionId) {
        if let Some(settled) = self.settled.iter_mut().find(|s| s.id == id && s.outcome == SwitchOutcome::Applied) {
            settled.outcome = SwitchOutcome::Cancelled;
        }
    }

    /// RM-37: at the sample `now_k`, applies a pending switch that the owner has taken and whose boundary has been reached (the old
    /// repeat ends there and the new one begins). [`Bookings::prune`] and [`Bookings::switch`] run it first; [`Bookings::halt`]
    /// settles a pending switch itself.
    pub fn advance(&mut self, now_k: i64) {
        if let Some(switch) = self.pending.filter(|s| s.installed && s.boundary <= now_k) {
            self.pending = None;
            self.apply(switch, i64::MAX);
        }
    }

    /// The switches settled since the last call, in the order they were settled. The Provider writes them to its record.
    pub fn take_settled(&mut self) -> Vec<Settled> {
        std::mem::take(&mut self.settled)
    }

    /// The old repeat's region ends at the switch's boundary, and the new one begins there with the given front.
    fn apply(&mut self, switch: Switch, front: i64) {
        for c in self.chains.iter_mut().filter(|c| c.end == i64::MAX && c.start < switch.boundary) {
            c.front = c.front.min(switch.boundary);
        }
        self.chains.push(Chain { start: switch.boundary, end: i64::MAX, joined: Vec::new(), front, join: false });
        self.settled.push(Settled { id: switch.id, outcome: SwitchOutcome::Applied, moved_to: switch.moved_to });
    }

    /// RM-14, UR-23: a chain head that host lateness moves at dispatch, from `old` to `new` with its length `len` (`None` for a
    /// repeat). The moved region is tested against every other chain, the Stop region and the head's joined pieces (M4), and
    /// leaves `old`; a refused move changes nothing (the caller then calls [`Bookings::drop_piece`]). A head that
    /// [`Bookings::prune`] has already dropped is tested and inserted the same way. A joined piece that is
    /// not a chain's head is not moved here: the record does not reframe its chain (host lateness, RM-16).
    pub fn move_start(&mut self, old: i64, new: i64, len: Option<i64>) -> Result<(), Overlap> {
        // The hand-over horizon of `accept` does not apply: the owner hands in start order, so no chain after the moved head was handed before it.
        self.move_head(old, len, |bookings| (!bookings.meets(new, front(bookings.gap, new, len, false)?, None)).then_some(new)).map(|_| ()).ok_or(Overlap::Booking)
    }

    /// RM-14, RM-15, D24: takes the chain head `old` out of the record and puts it back at the start that `place` returns, with its
    /// length and join point: the one rewrite both dispatch moves share. `place` sees the record without the head, and `None` refuses
    /// the move, which leaves the record unchanged. A joined piece that is not a head cannot move, so it is refused.
    fn move_head(&mut self, old: i64, len: Option<i64>, place: impl FnOnce(&Bookings) -> Option<i64>) -> Option<i64> {
        let head = self.chains.iter().position(|c| c.start == old);
        if head.is_none() && self.chains.iter().any(|c| c.start < old && old <= c.last()) {
            return None;
        }
        let saved = self.chains.clone();
        let join = head.is_some_and(|i| self.chains[i].join && self.chains[i].last() == old);
        if let Some(i) = head {
            self.remove_head(i, old, len);
        }
        let gap = self.gap;
        let placed = place(self).and_then(|start| Some((start, front(gap, start, len, false)?)));
        let Some((start, region_front)) = placed else {
            self.chains = saved;
            return None;
        };
        self.chains.push(Chain { start, end: len.map_or(i64::MAX, |len| start + len), joined: Vec::new(), front: region_front, join });
        Some(start)
    }

    /// RM-15 (C6): a booking refused at dispatch leaves the overlap rule: its head's region goes, its joined pieces keep
    /// theirs, and its end stays in the value rule (`book` was not undone).
    pub fn drop_piece(&mut self, start: i64, len: Option<i64>) {
        if let Some(i) = self.chains.iter().position(|c| c.start == start) {
            self.remove_head(i, start, len);
        }
    }

    /// The chain at `i` without its head of length `len` at `old`: its joined pieces keep their span from the first piece on. A
    /// surviving head of at most 32 samples with no piece behind it is one closed device burst (RM-15), so no later booking joins it
    /// (S2, UR-23). Pieces left behind keep their regions: the front stays, and each piece's own front is kept as well.
    fn remove_head(&mut self, i: usize, old: i64, len: Option<i64>) {
        let gap = self.gap;
        match len {
            Some(len) if self.chains[i].last() != old => {
                let c = &mut self.chains[i];
                c.start = old + len;
                c.joined.retain(|&j| j > c.start);
                if c.closed_short() {
                    c.join = false;
                    c.front = front(gap, c.start, Some(c.end - c.start), false).unwrap_or(c.front);
                } else if !c.joined.is_empty() {
                    c.join = false;
                    let starts = std::iter::once(c.start).chain(c.joined.iter().copied());
                    let ends = c.joined.iter().copied().chain(std::iter::once(c.end));
                    let pieces = starts.zip(ends).filter_map(|(s, e)| front(gap, s, Some(e - s), false)).max().unwrap_or(c.front);
                    c.front = c.front.max(pieces);
                }
            }
            _ => {
                self.chains.remove(i);
            }
        }
    }

    /// RM-15, RM-16: whether `[start, front)` meets an accepted chain other than `skip`, or the Stop's region. The receipt rule
    /// ([`Bookings::accept`]) and a dispatch-time move ([`Bookings::move_start`]) both judge a region by it.
    fn meets(&self, start: i64, front: i64, skip: Option<usize>) -> bool {
        self.met_front(start, front, skip).is_some()
    }

    /// The largest front of the accepted regions that `[start, front)` meets (the chains other than `skip`, and the Stop's region), if any.
    fn met_front(&self, start: i64, front: i64, skip: Option<usize>) -> Option<i64> {
        self.chain_front(start, front, skip).into_iter().chain(self.region_met(start, front)).max()
    }

    /// The largest front of the chains other than `skip` that `[start, front)` meets, if any.
    fn chain_front(&self, start: i64, front: i64, skip: Option<usize>) -> Option<i64> {
        self.chains.iter().enumerate().filter(|(i, c)| Some(*i) != skip && c.start < front && start < c.front).map(|(_, c)| c.front).max()
    }

    /// The front of the Stop's region, if `[start, front)` meets it.
    fn region_met(&self, start: i64, front: i64) -> Option<i64> {
        self.region.filter(|&(s, f)| s < front && start < f).map(|(_, f)| f)
    }

    /// The chain that a booking of `len` at `start`, received at `now`, continues: one with `join` that ends at `start`, while
    /// `now <= join_by(start)` (RM-16). Only a booking with a length can continue a chain.
    fn joinable(&self, start: i64, len: Option<i64>, now: i64, join_by: &impl Fn(i64) -> i64) -> Option<usize> {
        len.and_then(|_| self.chains.iter().position(|c| c.join && c.end == start && now <= join_by(start)))
    }

    /// RM-15 (note 47 stage 5 §1.3): whether the owner may already have handed chain `c` to the device when a booking is received at
    /// `now`. Its start must lie below `horizon`. It cannot have reached the device while a joinable chain that ends before it is
    /// still open to a join (`now <= join_by(end)`): the owner holds that device burst until its join deadline (`tx.rs`, `continuation_open`).
    fn may_be_handed(&self, c: &Chain, now: i64, horizon: i64, join_by: &impl Fn(i64) -> i64) -> bool {
        c.start < horizon && !self.chains.iter().any(|x| x.join && x.end <= c.start && now <= join_by(x.end))
    }

    /// The front of the accepted region that holds `k`, if any; a join point (the end of a chain
    /// with `join`, received at `now` on time) is not held.
    fn held_until(&self, k: i64, joins: bool, now: i64, join_by: &impl Fn(i64) -> i64, horizon: i64) -> Option<i64> {
        self.chains
            .iter()
            .filter(|c| c.start <= k && k < c.front && !(joins && c.join && c.end == k && now <= join_by(k)))
            .map(|c| c.front)
            .chain(self.chains.iter().filter(|c| k < c.start && self.may_be_handed(c, now, horizon, join_by)).map(|c| c.front))
            .chain(self.region.filter(|&(start, front)| start <= k && k < front).map(|(_, front)| front))
            .max()
    }
}

/// RM-15: the exclusive front of a booking's region. A standalone head of at most 32 samples is
/// one closed device burst, and its front is its transmitted end (padded to four) plus `gap`
/// ([`closed_short_gap`]), the safe start after it (UR-22); a head longer than 32 and a joined piece end in a held
/// tail, `end + HELD_TAIL_FRONT`; a repeat never ends. `None` when the front does not fit in an `i64`
/// (the booking is refused, not wrapped), or when a short head's `gap` is not known ([`Bookings::unarmed`]).
fn front(gap: Option<i64>, start: i64, len: Option<i64>, joined: bool) -> Option<i64> {
    match len {
        None => Some(i64::MAX),
        Some(len) if joined || len > CLOSED_SHORT_MAX => start.checked_add(len)?.checked_add(HELD_TAIL_FRONT),
        Some(len) => start.checked_add(len.max(4))?.checked_add(gap?),
    }
}

/// RM-15: the first sample a booking received at decision instant `decided` may start at without
/// being late (note 47 stage 5, §0): one sample past the decision, plus the device lead.
/// `at_or_after` and `instant` are the stream's clock, which each Provider passes in.
pub fn ordinary_k(decided: i64, lead: i64, at_or_after: impl Fn(i64) -> i64, instant: impl Fn(i64) -> i64) -> i64 {
    at_or_after(instant(at_or_after(decided)).saturating_add(lead))
}

/// RM-16: the first sample a booking may start at after a `Stop` at `at` (note 47 stage 5, §0):
/// the first sample at or after `at + in_flight`, plus [`STOP_FRONT`]. The window `in_flight` is the
/// profile's; `at_or_after` is the stream's clock, which each Provider passes in.
pub fn stop_sample(at: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64) -> i64 {
    at_or_after(at.saturating_add(in_flight)).saturating_add(STOP_FRONT)
}

/// RM-16: the root tick a `Stop` at `at` bounds a transmit stream by: [`stop_sample`] in root ticks.
pub fn stop_bound(at: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64, instant: impl Fn(i64) -> i64) -> i64 {
    instant(stop_sample(at, in_flight, at_or_after))
}

/// RM-15 (note 47 stage 5 §1.3): the first sample a receipt at root tick `now` cannot yet be sure the owner has not handed: the
/// owner hands only below `at_or_after(now + in_flight)` (`tx.rs:443-444`), so a booking that starts below it may be on the device.
pub fn handover_horizon(now: i64, in_flight: i64, at_or_after: impl Fn(i64) -> i64) -> i64 {
    at_or_after(now.saturating_add(in_flight))
}

/// RM-37, UR-38: writes each settled switch into its row of the `switches` section: its outcome and the boundary it was moved to.
/// Both Providers call it, so the row has one writer.
pub fn write_settled(rows: &mut [serde_json::Value], settled: impl IntoIterator<Item = Settled>) {
    for settled in settled {
        let id = serde_json::to_value(settled.id).expect("an action id");
        if let Some(row) = rows.iter_mut().rev().find(|row| row["action_id"] == id) {
            row["outcome"] = serde_json::json!(settled.outcome.label());
            row["moved_to"] = serde_json::json!(settled.moved_to);
        }
    }
}

/// The first `origin + j·period` at or after `bound`, `j ≥ 0`: a repeat switch's boundary (note 47
/// stage 5, §0), with `bound` the first sample the switch may take effect at.
pub fn repeat_boundary(origin: i64, period: i64, bound: i64) -> i64 {
    origin + ((bound - origin).max(0) + period - 1) / period * period
}

/// RM-13: whether a burst is a repeat switch: a repeat under `send_asap_and_flag` whose metadata is exactly the mark `radio.repeat_operation = "switch"`.
pub fn is_switch(repeat: bool, late_policy: LatePolicy, metadata: &BTreeMap<Key, Value>) -> bool {
    repeat
        && late_policy == LatePolicy::SendAsapAndFlag
        && metadata.len() == 1
        && metadata.get(&Key::parse("radio.repeat_operation").expect("a declared metadata key")) == Some(&Value::Scalar(Scalar::from("switch")))
}

/// The values of record: the Run's configuration, changed by the values a stopped receive stream
/// or an idle transmit stream accepted (RM-21). A `start_rx` or a burst takes a copy of them at
/// its booking.
#[derive(Clone, Debug)]
pub struct Staging {
    values: BTreeMap<Key, Value>,
    /// Per transmit stream, the number of values it has accepted (RM-21): a burst's snapshot
    /// carries it, and the device's configuration is told by it (UR-36).
    tx_generation: BTreeMap<u32, u64>,
}

impl Staging {
    /// The values at the Run's start.
    pub fn new(configuration: BTreeMap<Key, Value>) -> Staging {
        Staging { values: configuration, tx_generation: BTreeMap::new() }
    }

    /// RM-21: stages `value` for `key` when it is a receive channel's frequency, gain or antenna,
    /// or a transmit channel's frequency, gain or antenna, carries no instant, and the channel is
    /// in a stream that takes it: a receive stream that is not streaming (`streaming(k)`, asked
    /// for the channel's own stream only), or a transmit stream whose [`Bookings`] allows it at
    /// `receipt`. A later value for the same key replaces an earlier one; an accepted transmit
    /// value bumps its stream's generation.
    pub fn set(&mut self, key: &Key, value: Value, has_at: bool, receipt: i64, streaming: impl Fn(u32) -> bool, busy: impl Fn(u32) -> Bookings) -> Result<(), Refusal> {
        let Some(Concrete::Channel { rx, number, leaf: "frequency_hz" | "gain_db" | "antenna" }) = concrete(key) else {
            return Err(Refusal::NotStaged);
        };
        if has_at {
            return Err(Refusal::Instant);
        }
        let stream_key = Key::parse(&crate::keys::channel(if rx { "rx" } else { "tx" }, number, "stream")).expect("a declared radio key");
        let stream = match self.values.get(&stream_key).and_then(Value::as_scalar) {
            Some(ezsdr_kernel::spec::Scalar::Int(stream)) if *stream >= 0 => *stream as u32,
            _ => return Err(Refusal::NoStream(number)),
        };
        if rx && streaming(stream) {
            return Err(Refusal::Streaming { channel: number, stream });
        }
        if !rx {
            busy(stream).stageable(receipt)?;
            *self.tx_generation.entry(stream).or_insert(0) += 1;
        }
        self.values.insert(key.clone(), value);
        Ok(())
    }

    /// RM-21: a `start_rx` of `stream` booked now. Refused when the stream is streaming (its
    /// line, replayed, ends on: a booked start not yet begun counts), else the configuration its
    /// segment begins with.
    pub fn start(&self, stream: u32, streaming: bool) -> Result<BTreeMap<Key, Value>, Refusal> {
        if streaming { Err(Refusal::StartWhileStreaming(stream)) } else { Ok(self.snapshot()) }
    }

    /// The configuration a `start_rx` booked now begins with (RM-21); a copy, so a later value
    /// does not change it.
    pub fn snapshot(&self) -> BTreeMap<Key, Value> {
        self.values.clone()
    }

    /// The configuration a transmit burst of `stream` booked now begins with, and the stream's
    /// generation at that time (RM-21, UR-36): a copy, so a later value does not change it.
    pub fn tx_snapshot(&self, stream: u32) -> (u64, BTreeMap<Key, Value>) {
        (self.tx_generation.get(&stream).copied().unwrap_or(0), self.snapshot())
    }
}
