//! The pure segmentation decisions of transmit (note 49 §5.4, E3a): how large the next send is, where it is cut, when the window lets it go,
//! whether the device burst ends with it, and what MR-25 counts as handed over at a `Stop`. Integers and plain structs only; no clock is
//! read here, so each decision is a table of inputs and results. The UHD Module's real-time executor and MockRadio's stepped model of it
//! (MR-15, MR-25) both call these, with the packet rules of their profile ([`PacketRules`]). The front arithmetic of every close is here too
//! ([`close_front`]), and the receipt's prediction of it ([`predicted_front`]) and of the hand-over ([`may_have_handed`]).

use crate::x3xx::PacketRules;

/// RM-15, UR-22: the samples past any closing send's exclusive end that the next device burst keeps (every close but a closed short device burst's),
/// and the floor of [`crate::staging::closed_short_gap`].
pub const CLOSE_GAP: i64 = 3;

/// UR-22, UR-23: the samples of the close that ends a device burst with a held tail (the tail and one zero) or with no payload (two zeros).
pub const CLOSE_SAMPLES: i64 = 2;

/// UR-22: the front a close leaves, the first sample a next device burst may start at: the close's first sample `start`, the `samples` it
/// carries and the `gap` the device keeps after them. Every front in the tree is an instance of this sum. It saturates: an executor's front near
/// the end of the range stays a bound ([`checked_close_front`] is the receipt's, which refuses instead).
pub const fn close_front(start: i64, samples: i64, gap: i64) -> i64 {
    start.saturating_add(samples).saturating_add(gap)
}

/// [`close_front`] that is `None` when the sum does not fit an `i64`: a receipt refuses such a booking, it never saturates one into an accepted
/// footprint (RM-15).
pub const fn checked_close_front(start: i64, samples: i64, gap: i64) -> Option<i64> {
    match start.checked_add(samples) {
        Some(end) => end.checked_add(gap),
        None => None,
    }
}

/// RM-15: a head longer than 32 samples, or a joined piece, closes with its held tail and a zero, which start at its last payload sample: the front
/// is its payload end plus this.
pub const HELD_TAIL_FRONT: i64 = close_front(-1, CLOSE_SAMPLES, CLOSE_GAP);

/// RM-16: a Stop's two-zero close starts at the cursor: the front is the cursor plus this.
pub const STOP_FRONT: i64 = close_front(0, CLOSE_SAMPLES, CLOSE_GAP);

/// UR-23: the largest send, in samples.
pub const SEND_CAP_MAX: i64 = 20_000;

/// UR-23: the samples one send carries at most, a quarter of the in-flight window (`window_ticks` root ticks, `ticks_per_sample` root ticks a
/// sample), never fewer than one first packet and never more than [`SEND_CAP_MAX`].
pub fn send_cap(window_ticks: i64, ticks_per_sample: i64, rules: &PacketRules) -> i64 {
    (window_ticks / ticks_per_sample.max(1) / 4).clamp(rules.min_open_first_send, SEND_CAP_MAX)
}

/// One booking's waveform on the stream's sample clock. `len` is at least one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waveform {
    /// The first sample of the booking.
    pub start: i64,
    /// The waveform's own length in samples.
    pub len: i64,
    /// The waveform repeats.
    pub repeat: bool,
    /// The repeat's buffer is the waveform tiled to at least a send, so one send crosses its wraps (UHD only).
    pub tiled: bool,
}

/// What one send may take from a booking at `next`, before the window: the offset into the waveform, the most samples it carries, and what
/// ends at those samples (UR-23, MR-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// The send's first sample.
    pub next: i64,
    /// Where `next` is in the waveform.
    pub offset: i64,
    /// The most samples the send carries; at least one.
    pub wanted: i64,
    repeat: bool,
    len: i64,
    held_cut: Option<i64>,
}

impl Extent {
    /// Whether a send of `count` samples reaches a finite waveform's end. A repeat's wrap is not an end.
    pub fn ends_waveform(&self, count: i64) -> bool {
        !self.repeat && self.offset + count == self.len
    }

    /// Whether a send of `count` samples reaches the next held booking's start: the Kernel's burst ends there.
    pub fn ends_at_held(&self, count: i64) -> bool {
        self.held_cut == Some(self.next.saturating_add(count))
    }
}

/// UR-23, MR-15: the extent of the send at `next`: the remainder of the waveform (a repeat's remainder to its wrap, unless it is tiled), at most
/// `chunk`, cut at the next held booking's start (`held_cut`, a Kernel burst boundary) and at `hard_cut` (a clock's or a Stop's cut, which ends no
/// Kernel burst). `None` when nothing is left before a cut or the waveform has no samples.
pub fn extent(waveform: Waveform, next: i64, chunk: i64, held_cut: Option<i64>, hard_cut: Option<i64>) -> Option<Extent> {
    if waveform.len <= 0 {
        return None;
    }
    let since = next.saturating_sub(waveform.start);
    let offset = if waveform.repeat { since.rem_euclid(waveform.len) } else { since };
    let mut wanted = if waveform.tiled { chunk } else { chunk.min(waveform.len - offset) };
    if let Some(cut) = [held_cut, hard_cut].into_iter().flatten().min() {
        wanted = wanted.min(cut.saturating_sub(next));
    }
    (wanted > 0).then_some(Extent { next, offset, wanted, repeat: waveform.repeat, len: waveform.len, held_cut })
}

/// The real-time executor's decision for one send (UR-22, UR-23). Counts are samples per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The samples taken from the booking.
    pub count: i64,
    /// The Kernel burst ends with this send.
    pub eob: bool,
    /// A device burst's first send of at most the open first send (UR-22): the size class only. It closes the device burst only when the
    /// Kernel burst ends with it, even at a held start; `device_eob` says whether it closes.
    pub short_first: bool,
    /// The last sample is held back and sent later with the device's end of burst (UR-23).
    pub hold: bool,
    /// This send ends the device burst.
    pub device_eob: bool,
    /// The samples of the booking this send carries (`count`, less the held one).
    pub sending: i64,
    /// The zero samples appended to a closing packet (UR-22).
    pub pad: i64,
    /// The samples the device receives: `sending + pad`.
    pub device_count: i64,
    /// A closing send that is also the device burst's first send of at most the open first send: its front keeps the stream's short gap, not
    /// the long one (RM-15). False for a send that does not close.
    pub closes_short: bool,
}

/// UR-22, UR-23: the next send of an open booking, or `None` while the window cannot yet take what it must: a device burst's first send needs
/// `min(wanted, min_open_first_send)` samples of window, a continuation all of `wanted`. `device_sob` is whether this send starts a device
/// burst; `window_cut` is the first sample the window does not reach. The last sample is held back (`hold`) when it ends the Kernel burst, no
/// booking follows, and its instant is more than `lead` root ticks after `now`; `instant` is the stream's clock.
pub fn plan_send(rules: &PacketRules, extent: &Extent, device_sob: bool, window_cut: i64, now: i64, lead: i64, instant: impl Fn(i64) -> i64) -> Option<Plan> {
    let avail = window_cut.saturating_sub(extent.next);
    let wanted = extent.wanted;
    if avail < if device_sob { wanted.min(rules.min_open_first_send) } else { wanted } {
        return None;
    }
    let count = wanted.min(avail);
    let ends_waveform = extent.ends_waveform(count);
    let ends_at_held = extent.ends_at_held(count);
    let short_first = device_sob && count <= rules.min_open_first_send;
    let eob = ends_waveform || ends_at_held;
    let hold = eob && !ends_at_held && !short_first && instant(extent.next.saturating_add(count)).saturating_sub(now) > lead;
    let device_eob = eob && (!ends_at_held || short_first) && !hold;
    let sending = if hold { count - 1 } else { count };
    let pad = if device_sob && device_eob { (rules.min_first_send - sending).max(0) } else { i64::from(device_eob && sending == 1) };
    let device_count = sending + pad;
    let closes_short = device_eob && device_sob && device_count <= rules.min_open_first_send;
    Some(Plan { count, eob, short_first, hold, device_eob, sending, pad, device_count, closes_short })
}

impl Plan {
    /// UR-22: the front this send's close leaves, or `None` when the send does not end the device burst. `start` is the send's first sample;
    /// `short_gap` is the stream's [`crate::staging::closed_short_gap`], kept past a closing first send of at most the open first send
    /// ([`Plan::closes_short`]) and not past any other close ([`CLOSE_GAP`]). Saturating, as [`close_front`].
    pub fn front(&self, start: i64, short_gap: i64) -> Option<i64> {
        self.device_eob.then(|| close_front(start, self.device_count, if self.closes_short { short_gap } else { CLOSE_GAP }))
    }
}

/// A booking the Mock counts for MR-25's Stop cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The booking's first sample.
    pub start: i64,
    /// Its length in samples, at least one.
    pub len: i64,
    /// A repeat has no end.
    pub repeat: bool,
    /// The booking is the stream's open burst, already being sent.
    pub open: bool,
}

/// MR-25, RM-15: the end a real-time executor would have handed over for `pending` by the hand-over horizon `window_end`, or `None` when it
/// would not have handed it: its first send, which needs `min_open_first_send` samples (a finite burst's whole length when it is shorter, a
/// repeat's always), must fit the window. The open burst has begun, so one sample of window is enough; a continuation takes what fits. This is the
/// window-only mode of the segmentation: it ignores the next held start and a repeat's wrap, and has no send-size cap.
pub fn handed_end(rules: &PacketRules, pending: Pending, window_end: i64) -> Option<i64> {
    let first = if pending.open {
        1
    } else if pending.repeat {
        rules.min_open_first_send
    } else {
        pending.len.min(rules.min_open_first_send)
    };
    (pending.start.saturating_add(first) <= window_end).then(|| if pending.repeat { window_end } else { pending.start.saturating_add(pending.len).min(window_end) })
}

/// RM-15: the exclusive front of a booking's region at receipt, before any send exists: the envelope of the fronts the executor's closes can leave
/// it. A standalone head of at most `min_open_first_send` samples is one closed device burst, padded to `min_first_send` and kept `gap`
/// ([`crate::staging::closed_short_gap`]) past that; a longer head and a joined piece end in a held tail, [`HELD_TAIL_FRONT`] past their end (a close
/// without the hold, [`Plan::front`], is one sample less: the receipt keeps the later front); a repeat never ends. Checked: `None` when the front
/// does not fit an `i64` (the booking is refused, not wrapped), or when a short head's `gap` is not known.
pub fn predicted_front(rules: &PacketRules, gap: Option<i64>, start: i64, len: Option<i64>, joined: bool) -> Option<i64> {
    match len {
        None => Some(i64::MAX),
        Some(len) if joined || len > rules.min_open_first_send => start.checked_add(len)?.checked_add(HELD_TAIL_FRONT),
        Some(len) => checked_close_front(start, len.max(rules.min_first_send), gap?),
    }
}

/// RM-15 (note 47 stage 5 §1.3): whether the owner may already have handed a chain that starts at `start` when a booking is received before the
/// hand-over `horizon` ([`crate::staging::handover_horizon`]): it starts below it, and no earlier chain is still open to a join (`behind_open_join`),
/// for the owner holds that device burst until its join deadline. The window rule alone: a chain may be handed without the owner having sent a sample
/// of it, which is what [`handed_end`] asks.
pub fn may_have_handed(start: i64, horizon: i64, behind_open_join: impl FnOnce() -> bool) -> bool {
    start < horizon && !behind_open_join()
}
