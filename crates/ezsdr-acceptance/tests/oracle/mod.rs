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
    /// The primary root's rate in hertz, so that one sample is `n / root_hz` seconds (RM-15's closed short gap is 15 µs of samples).
    pub root_hz: i64,
    pub window: i64,
    pub lead: i64,
    pub allowance: i64,
}

impl Params {
    /// RM-15: the samples past a closed short burst's padded end that its front keeps: `max(3, ceil(15 µs × rate))`, the rate being
    /// `root_hz / n`. The oracle's own integer arithmetic, not `ezsdr_radio::staging`'s.
    pub fn gap(&self) -> i64 {
        let (num, den) = (15 * self.root_hz as i128, self.clock.n as i128 * 1_000_000);
        (((num + den - 1) / den) as i64).max(3)
    }

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
    /// after `t + W`, plus five (the two-zero close ends at the cursor plus two, and the next burst keeps three more).
    pub fn stop_bound(&self, t: i64) -> (i64, i64) {
        (self.clock.at_or_after(t), self.clock.at_or_after(t + self.window) + 5)
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
/// padded to four) its padded end plus `gap` (`Params::gap`); a head longer than 32 samples, or a joined piece, its end plus four.
pub fn front_offset(len: i64, joined: bool, gap: i64) -> i64 {
    if len <= SHORT && !joined { len.max(4) + gap } else { len + 4 }
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
pub fn region(p: &Params, start: i64, len: Option<i64>) -> Region {
    match len {
        Some(len) => Region { start, end: start + len, front: start + front_offset(len, false, p.gap()), len },
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
    let mine = region(p, start, len);
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
        let mine = region(p, start, len);
        match regions.iter().find(|r| start < r.front && r.start < mine.front) {
            Some(r) => start = r.front,
            None => return start,
        }
    }
}

/// RM-37: a switch's boundary is the first sample `origin + j * period` (`j >= 0`) at or after the later of its `at` and the receipt's
/// sample plus the window and the device lead. None when no such sample fits an `i64` (or `period` is not positive): the switch has no
/// boundary and is refused at receipt.
pub fn switch_boundary(p: &Params, origin: i64, period: i64, recv: i64, at: Option<i64>) -> Option<i64> {
    let bound = recv.checked_add(p.window)?.checked_add(p.lead)?;
    grid(origin, period, p.clock.at_or_after(bound).max(at.unwrap_or(i64::MIN)))
}

/// The first `origin + j * period`, `j >= 0`, at or after `bound`; None when it does not fit an `i64`. Wide integers, so that no step wraps.
fn grid(origin: i64, period: i64, bound: i64) -> Option<i64> {
    if period <= 0 {
        return None;
    }
    let (origin, period) = (i128::from(origin), i128::from(period));
    let d = (i128::from(bound) - origin).max(0);
    i64::try_from(origin + (d + period - 1).div_euclid(period) * period).ok()
}

/// RM-37 (#86): the requested target a switch's burst record carries: its `at` when it is timed and the receipt pushed its boundary past
/// the grid point `at` alone gives; none for an untimed switch (RS-19), one on that grid point, or one that has no boundary.
pub fn switch_requested_target(p: &Params, origin: i64, period: i64, recv: i64, at: i64, untimed: bool) -> Option<i64> {
    let (boundary, own) = (switch_boundary(p, origin, period, recv, Some(at))?, grid(origin, period, at)?);
    (!untimed && boundary != own).then_some(at)
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

/// UR-38 (#84): a switch on a repeat that has not started on the device is refused at receipt: no row, and the repeat it would replace
/// is left as it is (a Provider whose repeats start at acceptance never refuses it).
pub fn switch_refused_unstarted() -> bool {
    true
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

/// A booking as it stands at a Stop (design note 49 E3a, table Q): the one literal both Providers' tests read, so that their two columns
/// of expectations are about the same bookings. Its start is `offset` samples from the Stop sample `t_k` (`from_window_edge` false) or from
/// the window's first sample `we` (true); `len` is `None` for a repeat; `started` when the booking's start is before the Stop.
#[derive(Clone, Copy, Debug)]
pub struct StopShape {
    pub label: &'static str,
    pub from_window_edge: bool,
    pub offset: i64,
    pub len: Option<i64>,
    pub started: bool,
}

const fn shape(label: &'static str, from_window_edge: bool, offset: i64, len: Option<i64>, started: bool) -> StopShape {
    StopShape { label, from_window_edge, offset, len, started }
}

/// Table Q's single-booking rows. Rows 4 and 6 are where the Providers' first-send rules differ in what they hand (UR-23 / MR-25); the
/// rows with `started` are bursts the device already plays at the Stop. The long finite one is 30 000 samples: the oracle's ramps stay
/// inside the full scale below 32 767.
pub const STOP_SHAPES: [StopShape; 14] = [
    shape("finite 1 at we-1", true, -1, Some(1), false),
    shape("finite 1 at we", true, 0, Some(1), false),
    shape("finite 20 at we-20", true, -20, Some(20), false),
    shape("finite 20 at we-19", true, -19, Some(20), false),
    shape("finite 32 at we-32", true, -32, Some(32), false),
    shape("finite 32 at we-31", true, -31, Some(32), false),
    shape("finite 33 at we-32", true, -32, Some(33), false),
    shape("finite 33 at we-31", true, -31, Some(33), false),
    shape("finite 100 inside the window", false, 1_000, Some(100), false),
    shape("repeat at we-32", true, -32, None, false),
    shape("repeat at we-31", true, -31, None, false),
    shape("open repeat at t_k-5000", false, -5_000, None, true),
    shape("open repeat at t_k-4010", false, -4_010, None, true),
    shape("long finite at t_k-5000", false, -5_000, Some(30_000), true),
];

/// Table Q's joined pair (row 10, F3b): `(offset from we, len)` of A and of B, B booked at A's exact end.
pub const JOINED_AT_A_STOP: [(i64, i64); 2] = [(-90, 40), (-50, 100)];

/// The sample counts of what completed sends carried (a failed send, not counted) as `(start, n)` pairs, summed: the Kernel record's
/// `samples` for an abandoned burst (UR-22; a withheld tail is counted with its buffer, which the caller includes in `n`).
pub fn recorded(completed: &[(i64, i64)]) -> i64 {
    completed.iter().map(|(_, n)| n).sum()
}

/// A receipt row (design note 49 E3b-1, table C5): bookings accepted in order, an optional Stop, then one probe booking received last on a
/// fresh stream. Offsets are in samples from the stream's receipt sample `t_k` (`near`) or from `t_k + FAR` (the bookings are then far outside
/// the hand-over window, so RM-15's hand-over rule plays no part): a start never late. The verdict column is typed from RM-15 and RM-16, and each
/// Provider's test reads it against the oracle's `admit` and its own receipt.
#[derive(Clone, Copy, Debug)]
pub struct FrontReceipt {
    pub label: &'static str,
    /// `(offset, len)` of the bookings accepted first (`len` `None`: a repeat).
    pub bookings: &'static [(i64, Option<i64>)],
    /// A Stop received after the bookings.
    pub stop: bool,
    /// The offsets are from `t_k` itself, not from `t_k + FAR`.
    pub near: bool,
    /// `(offset, len, verdict)` of the probe; each probe runs on its own fresh stream.
    pub probes: &'static [(i64, i64, Verdict)],
}

/// The distance, in samples, of table C5's far bookings from the receipt sample.
pub const FAR: i64 = 50_000;

pub const FRONT_RECEIPTS: [FrontReceipt; 11] = [
    FrontReceipt { label: "short head 1", bookings: &[(0, Some(1))], stop: false, near: false, probes: &[(18, 1, Verdict::Refuse), (19, 1, Verdict::Accept)] },
    FrontReceipt { label: "short head 3", bookings: &[(0, Some(3))], stop: false, near: false, probes: &[(18, 1, Verdict::Refuse), (19, 1, Verdict::Accept)] },
    FrontReceipt { label: "short head 4", bookings: &[(0, Some(4))], stop: false, near: false, probes: &[(18, 1, Verdict::Refuse), (19, 1, Verdict::Accept)] },
    FrontReceipt { label: "short head 32", bookings: &[(0, Some(32))], stop: false, near: false, probes: &[(46, 1, Verdict::Refuse), (47, 1, Verdict::Accept)] },
    FrontReceipt {
        label: "long head 33: a booking at its end joins it, then end + 4",
        bookings: &[(0, Some(33))],
        stop: false,
        near: false,
        probes: &[(33, 1, Verdict::Accept), (34, 1, Verdict::Refuse), (36, 1, Verdict::Refuse), (37, 1, Verdict::Accept)],
    },
    FrontReceipt { label: "long head 100", bookings: &[(0, Some(100))], stop: false, near: false, probes: &[(100, 1, Verdict::Accept), (103, 1, Verdict::Refuse), (104, 1, Verdict::Accept)] },
    FrontReceipt {
        label: "a piece joined to a long head: end + 4 of the piece",
        bookings: &[(0, Some(40)), (40, Some(10))],
        stop: false,
        near: false,
        probes: &[(50, 1, Verdict::Accept), (53, 1, Verdict::Refuse), (54, 1, Verdict::Accept)],
    },
    FrontReceipt {
        label: "a one-sample piece joined to a long head",
        bookings: &[(0, Some(40)), (40, Some(1))],
        stop: false,
        near: false,
        probes: &[(44, 1, Verdict::Refuse), (45, 1, Verdict::Accept)],
    },
    FrontReceipt { label: "a repeat holds the rest of the stream", bookings: &[(0, None)], stop: false, near: false, probes: &[(10, 1, Verdict::Refuse), (1_000_000, 1, Verdict::Refuse)] },
    FrontReceipt {
        label: "a Stop that cancelled a booking inside its window leaves the region to B",
        bookings: &[(5_000, Some(20))],
        stop: true,
        near: true,
        probes: &[(5_000, 1, Verdict::Refuse), (10_004, 1, Verdict::Refuse), (10_005, 1, Verdict::Accept)],
    },
    FrontReceipt {
        label: "a Stop that cancelled nothing leaves no region",
        bookings: &[(FAR, Some(20))],
        stop: true,
        near: true,
        probes: &[(5_000, 1, Verdict::Accept), (10_004, 1, Verdict::Accept)],
    },
];

impl FrontReceipt {
    /// The first sample of an offset of this row, for the stream whose receipt sample is `t_k`.
    pub fn start(&self, t_k: i64, offset: i64) -> i64 {
        t_k + if self.near { 0 } else { FAR } + offset
    }

    /// The oracle's verdict on the probe `(offset, len)` of this row, every booking and the Stop received at root tick `recv` (stream sample `t_k`):
    /// the regions of RM-15 (a piece that starts at the end of a head of more than 32 samples extends that head's region to its own end plus four)
    /// and the Stop's region of RM-16.
    pub fn verdict(&self, p: &Params, t_k: i64, recv: i64, probe: (i64, i64)) -> Verdict {
        let mut regions: Vec<Region> = Vec::new();
        for &(offset, len) in self.bookings {
            let start = self.start(t_k, offset);
            let joined = len.and_then(|len| regions.iter().position(|r| r.end == start && r.len > SHORT).map(|i| (i, len)));
            match joined {
                Some((i, len)) => regions[i] = Region { end: start + len, front: start + front_offset(len, true, p.gap()), ..regions[i] },
                None => regions.push(region(p, start, len)),
            }
        }
        if self.stop {
            let cancelled: Vec<i64> = self.bookings.iter().map(|&(offset, _)| self.start(t_k, offset)).collect();
            regions.extend(stop_region(p, recv, &cancelled));
        }
        admit(p, &regions, recv, self.start(t_k, probe.0), Some(probe.1))
    }
}
