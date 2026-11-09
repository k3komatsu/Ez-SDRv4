//! The O1 spec oracle for transmit (design note 49 §5.4.11): what a transmit stream's device should emit and its records should
//! hold, written from the spec text (RM-13…RM-16, RM-37, UR-22, UR-38, MR-25) and independent of every Provider. It names no Kernel,
//! Radio, Mock or UHD type and calls no shared decision (`ezsdr_radio::staging`): it works on sample indices of one transmit
//! SampleClock, root ticks and `[f32; 2]` samples, so any crate can include it by `#[path]`.
//!
//! Units: instants are root ticks, samples are indices on the stream's clock (`Clock`: sample `k` is at `origin + k * n`).
#![allow(dead_code)]

pub type Iq = [f32; 2];

/// RM-15's bound between a short head and a long one, UR-22's `MIN_OPEN_FIRST_SEND`.
pub const SHORT: i64 = 32;

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub origin: i64,
    /// Root ticks per sample.
    pub n: i64,
}

impl Clock {
    pub fn instant(&self, k: i64) -> i64 {
        self.origin + k * self.n
    }

    /// The first sample at or after root tick `t`.
    pub fn at_or_after(&self, t: i64) -> i64 {
        (t - self.origin + self.n - 1).div_euclid(self.n)
    }
}

/// A Provider's profile values in root ticks (never read from Provider output): `window` is `radio.timing.tx_in_flight_ns`, `lead`
/// the device lead (RM-14), `allowance` the delivery allowance (zero for a stepped Provider).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub clock: Clock,
    pub window: i64,
    pub lead: i64,
    pub allowance: i64,
}

impl Params {
    /// RM-14's decision instant: the first transmit sample at or after the receipt, as an instant.
    pub fn decision(&self, recv: i64) -> i64 {
        self.clock.instant(self.clock.at_or_after(recv))
    }

    /// RM-14: the first sample at or after the decision instant plus the device lead.
    pub fn ordinary(&self, recv: i64) -> i64 {
        self.clock.at_or_after(self.decision(recv) + self.lead)
    }

    /// RM-15 `J(e)`: the latest receipt instant (inclusive) that still joins a booking at sample `e` to the chain ending there.
    pub fn join_by(&self, e: i64) -> i64 {
        self.clock.instant(e) - (self.lead + self.allowance)
    }

    /// RM-16: a Stop at instant `t` closes at the first sample at or after `t` (`t_k`) and no later than `B`, the first sample at or
    /// after `t + W`, plus three.
    pub fn stop_bound(&self, t: i64) -> (i64, i64) {
        (self.clock.at_or_after(t), self.clock.at_or_after(t + self.window) + 3)
    }

    /// RM-14: whether a booking at `start`, received at `recv`, is late (its start is less than the device lead after the decision).
    pub fn is_late(&self, recv: i64, start: i64) -> bool {
        self.clock.instant(start) < self.decision(recv) + self.lead
    }
}

/// A sample of the oracle's waveforms: `[(i + 1) / 32768, tag / 64]`, exact in `f32`, distinct per index and per waveform and never
/// zero (zero is silence or padding). Indices up to 32 766 stay inside the full scale.
pub fn sample(tag: u8, index: usize) -> Iq {
    [(index + 1) as f32 / 32_768.0, f32::from(tag) / 64.0]
}

pub fn ramp(tag: u8, len: usize) -> Vec<Iq> {
    (0..len).map(|i| sample(tag, i)).collect()
}

/// RM-15: the samples past a booking's start that its region holds: a standalone closed device burst of 1 to 32 samples (1 to 3
/// padded to four) its padded end plus ten; a head longer than 32 samples, or a joined piece, its end plus two.
pub fn front_offset(len: i64, joined: bool) -> i64 {
    if len <= SHORT && !joined { len.max(4) + 10 } else { len + 2 }
}

/// An accepted booking's footprint `[start, front)` (RM-15); `end` is its payload end, both `i64::MAX` for a repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: i64,
    pub end: i64,
    pub front: i64,
    pub len: i64,
}

/// The region of a booking of `len` samples at `start` (`None`: a repeat).
pub fn region(start: i64, len: Option<i64>) -> Region {
    match len {
        Some(len) => Region { start, end: start + len, front: start + front_offset(len, false), len },
        None => Region { start, end: i64::MAX, front: i64::MAX, len: i64::MAX },
    }
}

/// RM-16: the region `[t_k, B)` a Stop leaves where it cancelled a booking that starts in `[t_k, B)`; none when every cancelled booking
/// starts at or after `B` (it was never handed).
pub fn stop_region(p: &Params, stop_recv: i64, cancelled_starts: &[i64]) -> Option<Region> {
    let (t_k, b) = p.stop_bound(stop_recv);
    cancelled_starts.iter().any(|s| (t_k..b).contains(s)).then_some(Region { start: t_k, end: b, front: b, len: b - t_k })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    /// RM-14: the start is late; the late policy decides (the footprint is judged after it).
    Late,
    /// RM-15: the region meets an accepted region (a join that is not in time included).
    Refuse,
}

/// RM-14, RM-15: the receipt-time verdict on a timed booking of `len` samples at `start` received at `recv` against the accepted
/// regions. A booking starting at the exact end of an accepted chain of more than 32 samples joins it when received no later than
/// `J(e)` (inclusive) and is refused after.
pub fn admit(p: &Params, regions: &[Region], recv: i64, start: i64, len: Option<i64>) -> Verdict {
    if p.is_late(recv, start) {
        return Verdict::Late;
    }
    let mine = region(start, len);
    for r in regions {
        let joins = r.end == start && r.len > SHORT;
        if joins {
            if recv > p.join_by(start) {
                return Verdict::Refuse;
            }
            continue;
        }
        if start < r.front && r.start < mine.front {
            return Verdict::Refuse;
        }
    }
    Verdict::Accept
}

/// RM-14, RM-15: where a late `SendAsap` booking (`at` its asked start) or an untimed one (`at` its stamp) starts: the first sample at or
/// after `max(at, ordinary)` from which its region meets no accepted region.
pub fn place(p: &Params, regions: &[Region], recv: i64, at: Option<i64>, len: Option<i64>) -> i64 {
    let mut start = p.ordinary(recv).max(at.unwrap_or(i64::MIN));
    loop {
        let mine = region(start, len);
        match regions.iter().find(|r| start < r.front && r.start < mine.front) {
            Some(r) => start = r.front,
            None => return start,
        }
    }
}

/// RM-37: a switch's boundary is the first sample `origin + j * period` (`j >= 0`) at or after the later of its `at` and the receipt's
/// sample plus the window and the device lead.
pub fn switch_boundary(p: &Params, origin: i64, period: i64, recv: i64, at: Option<i64>) -> i64 {
    grid(origin, period, p.clock.at_or_after(recv + p.window + p.lead).max(at.unwrap_or(i64::MIN)))
}

/// The first `origin + j * period`, `j >= 0`, at or after `bound`.
fn grid(origin: i64, period: i64, bound: i64) -> i64 {
    let d = (bound - origin).max(0);
    origin + (d + period - 1).div_euclid(period) * period
}

/// UR-38: where the owner moves a switch whose handed cursor passed its boundary: the first grid point at or after the cursor.
pub fn moved_boundary(origin: i64, period: i64, cursor: i64) -> i64 {
    grid(origin, period, cursor)
}

/// A burst the oracle expects on the device: waveform `tag` of `len` samples from `start`, repeating or not, until `end` (exclusive).
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub tag: u8,
    pub start: i64,
    pub len: i64,
    pub repeat: bool,
    pub end: i64,
    /// Whether a Stop's cut range (`Slots::cut`) applies to this burst's samples.
    pub cuttable: bool,
}

impl Seg {
    pub fn once(tag: u8, start: i64, len: i64) -> Seg {
        Seg { tag, start, len, repeat: false, end: start + len, cuttable: false }
    }

    pub fn repeating(tag: u8, start: i64, len: i64, end: i64) -> Seg {
        Seg { tag, start, len, repeat: true, end, cuttable: false }
    }

    /// This burst is the one a Stop cuts.
    pub fn cuttable(self) -> Seg {
        Seg { cuttable: true, ..self }
    }

    fn at(&self, k: i64) -> Option<Iq> {
        let natural = if self.repeat { i64::MAX } else { self.start + self.len };
        (k >= self.start && k < self.end.min(natural)).then(|| sample(self.tag, ((k - self.start) % self.len) as usize))
    }
}

/// The expected device output: the segments (a later start replaces an earlier burst, as a switch boundary or a next burst does) and,
/// for a Stop on a Provider that hands ahead, the cut's range `[lo, hi]` (RM-16): every sample before `lo` is as expected, a sample
/// from `lo` on is the expected one until the first silent sample and silent after it, and every sample from `hi` on is silent.
#[derive(Clone, Debug, Default)]
pub struct Slots {
    pub segs: Vec<Seg>,
    pub cut: Option<(i64, i64)>,
}

fn silent(sample: Option<Iq>) -> bool {
    sample.is_none_or(|s| s == [0.0, 0.0])
}

fn name(sample: Option<Iq>) -> String {
    match sample {
        None => "nothing".to_owned(),
        Some([0.0, 0.0]) => "zero".to_owned(),
        Some([re, im]) => format!("waveform {} index {}", (im * 64.0).round(), (re * 32_768.0).round() - 1.0),
    }
}

impl Slots {
    fn seg_at(&self, k: i64) -> Option<&Seg> {
        self.segs.iter().filter(|s| s.start <= k).max_by_key(|s| s.start)
    }

    /// What the oracle expects at `k` before any cut: the latest-starting segment that covers it.
    pub fn expected(&self, k: i64) -> Option<Iq> {
        self.seg_at(k).and_then(|s| s.at(k))
    }

    /// The first mismatch in `[from, to)` against `observed(k)`, or `Ok`. The first silent sample from `lo` on is the cut, returned.
    pub fn check(&self, from: i64, to: i64, observed: &dyn Fn(i64) -> Option<Iq>) -> Result<Option<i64>, String> {
        let mut cut = None;
        for k in from..to {
            let (want, got) = (self.expected(k), observed(k));
            let cuttable = self.seg_at(k).is_some_and(|s| s.cuttable);
            match self.cut {
                Some((lo, hi)) if k >= lo && cuttable => {
                    if cut.is_some() || k >= hi {
                        if !silent(got) {
                            return Err(format!("k={k}: expected silence after the cut, observed {}", name(got)));
                        }
                        // Silence first beginning exactly at `hi` is a cut the range allows.
                        if cut.is_none() && k == hi {
                            cut = Some(hi);
                        }
                    } else if silent(got) {
                        cut = Some(k);
                    } else if got != want {
                        return Err(format!("k={k}: expected {}, observed {}", name(want), name(got)));
                    }
                }
                _ => {
                    let ok = if silent(want) { silent(got) } else { got == want };
                    if !ok {
                        return Err(format!("k={k}: expected {}, observed {}", name(want), name(got)));
                    }
                }
            }
        }
        if let Some((lo, hi)) = self.cut {
            if let Some(c) = cut {
                if c < lo || c > hi {
                    return Err(format!("cut {c} outside [{lo}, {hi}]"));
                }
            }
        }
        Ok(cut)
    }
}

/// The outcome of a switch's `switches` row as RM-37 states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Cancelled => "cancelled",
        }
    }
}

/// RM-37: a Stop whose bound `B` is at or before the boundary `b` cancels the switch; one with `b < B` leaves it applied (the text
/// design note 49's C2 will change: a switch whose new payload was never handed is then not `applied`).
pub fn outcome_after_stop(b: i64, bound: i64) -> Outcome {
    if bound <= b { Outcome::Cancelled } else { Outcome::Applied }
}

/// RM-37: a Provider that ends the repeat on its own (an abandon) before handing the sample at `b`, with its cursor `c` at or before
/// `b`, cancels the switch.
pub fn outcome_after_abandon(b: i64, cursor: i64) -> Outcome {
    if b >= cursor { Outcome::Cancelled } else { Outcome::Applied }
}

/// RM-37: a Provider that drops the repeat at dispatch under its late policy cancels the switch.
pub fn outcome_after_repeat_dropped() -> Outcome {
    Outcome::Cancelled
}

/// RM-14, UR-38, SC-29a: a switch moved under SendAsap (the owner's cursor passed its boundary) records the instant it asked for (`at`) as
/// its requested target, not the boundary chosen at receipt.
pub fn requested_target_of_a_moved_switch(at: i64) -> Option<i64> {
    Some(at)
}

/// What a Stop leaves of a booking that was not handed to the device (RM-16 "cancel held bursts not handed", MR-25 "cancelled into
/// `rejected`"): one cancellation note in `rejected`, no Kernel burst record, no sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled {
    pub notes: usize,
    pub records: usize,
    pub samples: bool,
}

pub fn unhanded_booking_at_a_stop() -> Cancelled {
    Cancelled { notes: 1, records: 0, samples: false }
}

/// A burst for MR-25's payload cut: a repeat when `len` is `None`; `started` when the Mock has already opened it (it counts with a
/// first send of one sample); `before_stop` when it was received before the Stop.
#[derive(Clone, Copy, Debug)]
pub struct MBurst {
    pub start: i64,
    pub len: Option<i64>,
    pub started: bool,
    pub before_stop: bool,
}

/// MR-25: the Mock's transmit payload cut at a Stop received at `t`: the later of the Stop sample, the payload handed within the
/// in-flight window (each burst received before the Stop whose start is before the window's first sample and whose first send fits
/// before it contributes the earlier of its exclusive end and that sample, a repeat that sample) and a cut retained from an earlier
/// Stop on the same clock. A zero window gives the Stop sample.
pub fn mock_cut(p: &Params, t: i64, bursts: &[MBurst], previous: Option<i64>) -> i64 {
    let (t_k, _) = p.stop_bound(t);
    let window_end = p.clock.at_or_after(t + p.window);
    let mut cut = t_k.max(previous.unwrap_or(i64::MIN));
    for b in bursts.iter().filter(|b| b.before_stop) {
        let first = if b.started { 1 } else { b.len.map_or(SHORT, |l| l.min(SHORT)) };
        if b.start < window_end && b.start + first <= window_end {
            cut = cut.max(b.len.map_or(window_end, |l| (b.start + l).min(window_end)));
        }
    }
    cut
}

/// The sample counts of what completed sends carried (a failed send, not counted) as `(start, n)` pairs, summed: the Kernel record's
/// `samples` for an abandoned burst (UR-22; a withheld tail is counted with its buffer, which the caller includes in `n`).
pub fn recorded(completed: &[(i64, i64)]) -> i64 {
    completed.iter().map(|(_, n)| n).sum()
}
