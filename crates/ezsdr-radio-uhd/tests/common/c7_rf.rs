//! bt2's RF detector for cell C7: the per-period correlation of a capture against the repeat's PN (plan `test-plan-v3.md` §4.3, as amended by the B3
//! specification). A capture is the receive samples of the trial, `(root tick, sample)`, in runs of consecutive samples (`per` ticks a sample) separated by holes.
//!
//! The control's loop delay is the lag of the correlation's peak over its first 20 periods, anchored to a power onset (a periodic PN ties peaks a whole period
//! apart, so the peak alone cannot resolve the delay); the floor is the largest gain of the quiet windows before the control. A part is read by its first period
//! (against the control's first, which shares the DUC's transient) and 19 steady periods (against the control's median gain and circular phase).

#![allow(dead_code)]

use ezsdr_radio_uhd::Iq;

use super::tone::angle_difference_deg;

/// Root ticks in a millisecond.
const MS: i64 = super::MCR / 1_000;

/// The silence scan's guard after `region_from + d`, in microseconds: a time, not a number of periods, so a long PN period cannot eat the gap. 200 us is two
/// periods at the fake's 100 us, so the fake's numbers do not move.
pub const SILENCE_GUARD_US: i64 = 200;

/// The silence scan's guard in root ticks.
const GUARD_TICKS: i64 = SILENCE_GUARD_US * super::MCR / 1_000_000;

/// The repeat's waveform and its energy `sum |w|^2`.
#[derive(Clone)]
pub struct Wave {
    pub w: Vec<Iq>,
    pub norm: f64,
}

impl Wave {
    pub fn new(samples: &[(f32, f32)]) -> Wave {
        let w: Vec<Iq> = samples.iter().map(|&(re, im)| [re, im]).collect();
        let norm = w.iter().map(|s| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum();
        Wave { w, norm }
    }
}

/// The receive samples of one trial: root ticks, runs of consecutive samples `per` ticks apart, holes between them.
#[derive(Clone)]
pub struct Capture {
    samples: Vec<(i64, Iq)>,
    pub per: i64,
    /// Samples of one period of the repeat's PN.
    pub n: usize,
}

impl Capture {
    /// `fake_ramp`: the FakeDevice's idle sample at tick `t` is `[((t / per).rem_euclid(65_536)) as f32 / 65_536.0, 0.0]` (`device.rs`'s `ramp_at`): a sample
    /// equal to it is replaced by `[0, 0]`; a transmitted sample (which replaces the ramp) is kept. Hardware passes `false`.
    pub fn new(samples: Vec<(i64, Iq)>, per: i64, n: usize, fake_ramp: bool) -> Capture {
        let samples = if fake_ramp {
            samples
                .into_iter()
                .map(|(tick, sample)| {
                    let ramp = ((tick / per).rem_euclid(65_536)) as f32 / 65_536.0;
                    (tick, if sample == [ramp, 0.0] { [0.0, 0.0] } else { sample })
                })
                .collect()
        } else {
            samples
        };
        Capture { samples, per, n }
    }

    /// The index of the sample nearest `tick` within half a sample; `None` for an empty capture or a tick between runs.
    fn near(&self, tick: i64) -> Option<usize> {
        let after = self.samples.partition_point(|(t, _)| *t < tick);
        let before = after.checked_sub(1);
        let after = (after < self.samples.len()).then_some(after);
        let distance = |index: usize| (self.samples[index].0 - tick).abs();
        let index = match (before, after) {
            (Some(before), Some(after)) => {
                if distance(before) <= distance(after) {
                    before
                } else {
                    after
                }
            }
            (Some(index), None) | (None, Some(index)) => index,
            (None, None) => return None,
        };
        (distance(index) * 2 <= self.per).then_some(index)
    }

    /// `len` samples from `index`, when they are one run of consecutive samples.
    fn slice(&self, index: usize, len: usize) -> Option<&[(i64, Iq)]> {
        let window = self.samples.get(index..index.checked_add(len)?)?;
        window.windows(2).all(|pair| pair[1].0 - pair[0].0 == self.per).then_some(window)
    }

    /// `n` consecutive samples (tick step `per`) starting at the sample nearest `tick` (`|diff| <= per/2`); `None` if they are not all in one run.
    pub fn window(&self, tick: i64) -> Option<&[(i64, Iq)]> {
        self.slice(self.near(tick)?, self.n)
    }

    /// Whether one run of consecutive samples spans `[from, to)` (a sample at tick `t` covers `[t, t + per)`).
    pub fn covers(&self, from: i64, to: i64) -> bool {
        let Some(index) = self.samples.partition_point(|(t, _)| *t <= from).checked_sub(1) else { return false };
        let mut last = self.samples[index].0;
        if last + self.per <= from {
            return false;
        }
        for (tick, _) in &self.samples[index + 1..] {
            if *tick != last + self.per {
                break;
            }
            last = *tick;
        }
        last + self.per >= to
    }

    /// The tick after the last sample.
    pub fn end(&self) -> i64 {
        self.samples.last().map_or(0, |(tick, _)| tick + self.per)
    }

    /// The tick of the first sample; 0 for an empty capture.
    pub fn start(&self) -> i64 {
        self.samples.first().map_or(0, |(tick, _)| *tick)
    }

    /// `Σ conj(w_k) s_k` over the window at `tick`.
    fn correlation(&self, wave: &Wave, tick: i64) -> Option<(f64, f64)> {
        let window = self.window(tick)?;
        let (mut re, mut im) = (0.0, 0.0);
        for (w, (_, s)) in wave.w.iter().zip(window) {
            let (wr, wi, sr, si) = (f64::from(w[0]), f64::from(w[1]), f64::from(s[0]), f64::from(s[1]));
            re += wr * sr + wi * si;
            im += wr * si - wi * sr;
        }
        Some((re, im))
    }

    /// `Σ conj(w_k) s_k` over the wave's first `len` samples against `len` capture samples at `tick`: the lag search's cheaper correlation.
    fn correlation_prefix(&self, wave: &Wave, tick: i64, len: usize) -> Option<(f64, f64)> {
        let window = self.slice(self.near(tick)?, len)?;
        let (mut re, mut im) = (0.0, 0.0);
        for (w, (_, s)) in wave.w.iter().zip(window) {
            let (wr, wi, sr, si) = (f64::from(w[0]), f64::from(w[1]), f64::from(s[0]), f64::from(s[1]));
            re += wr * sr + wi * si;
            im += wr * si - wi * sr;
        }
        Some((re, im))
    }

    /// The gain `|Σ conj(w_k) s_k| / norm` and phase (degrees) of the window at `tick`.
    pub fn gain_phase(&self, wave: &Wave, tick: i64) -> Option<(f64, f64)> {
        let (re, im) = self.correlation(wave, tick)?;
        Some((re.hypot(im) / wave.norm, im.atan2(re).to_degrees()))
    }

    /// The mean `|s|^2` of `len_samples` consecutive samples starting nearest `from`.
    pub fn power(&self, from: i64, len_samples: usize) -> Option<f64> {
        let window = self.slice(self.near(from)?, len_samples)?;
        Some(window.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / len_samples as f64)
    }
}

/// What the control repeat gives the detector: the loop delay, its steady gain and phase, its first period's gain and phase (which shares the DUC's
/// transient), the steady power, the quiet floor and the heard threshold.
#[derive(Debug)]
pub struct Control {
    /// The lag found, in ticks from the control's accepted start.
    pub d: i64,
    pub g_c: f64,
    pub phase_c: f64,
    pub g_c1: f64,
    pub phase_c1: f64,
    pub p_c: f64,
    pub q: f64,
    pub p_q: f64,
    pub t: f64,
    /// The mean-power threshold of a heard quarter-period window: `(4 p_q).max(0.1 p_c)`.
    pub tp: f64,
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

/// The control's delay, gain, phase and floor statistics; `Err` is an INCONCLUSIVE reason (`c7_verdict` prints it).
pub fn analyse_control(cap: &Capture, wave: &Wave, c0: i64, control_at: i64) -> Result<Control, String> {
    let (n, per) = (cap.n as i64, cap.per);
    let span = n * per;
    let quarter = cap.n / 4;
    let stride = quarter as i64 * per;
    let quiet_end = c0 + 20 * MS;

    // 1. The quiet floor: the largest gain of the n-sample windows and the largest power of the n/4-sample windows in [c0, c0 + 20 ms).
    let mut gains = Vec::new();
    let mut at = c0;
    while at + span <= quiet_end {
        if let Some((gain, _)) = cap.gain_phase(wave, at) {
            gains.push(gain);
        }
        at += stride;
    }
    let Some(q) = gains.iter().copied().reduce(f64::max) else { return Err("no quiet floor in the capture".to_owned()) };
    let mut powers = Vec::new();
    let mut at = c0;
    while at + quarter as i64 * per <= quiet_end {
        if let Some(power) = cap.power(at, quarter) {
            powers.push(power);
        }
        at += stride;
    }
    let p_q = powers.iter().copied().reduce(f64::max).unwrap_or(0.0);

    // 2. The loop delay: the lag whose 4-period correlation of the PN's first `l` samples is the largest (0..n samples; tick lag = k * per). The prefix keeps
    //    the search cheap at a 1 ms PN (a full period at every lag would be n^2 per period).
    let l = cap.n.min(1_000);
    let mut best: Option<(f64, i64)> = None;
    for k in 0..cap.n {
        let (mut re, mut im) = (0.0, 0.0);
        for p in 1..=4 {
            let (r, i) = cap.correlation_prefix(wave, control_at + k as i64 * per + p as i64 * span, l).ok_or_else(|| "the control has a hole".to_owned())?;
            re += r;
            im += i;
        }
        let magnitude = re.hypot(im);
        if best.is_none_or(|(largest, _)| magnitude > largest) {
            best = Some((magnitude, k as i64 * per));
        }
    }
    let d = best.map(|(_, d)| d).unwrap_or(0);

    // 3. The onset: the first n/4-sample window from c0 + 20 ms whose mean power reaches the power threshold.
    let mut period_powers = Vec::new();
    for p in 1..=19 {
        period_powers.push(cap.power(control_at + d + p as i64 * span, cap.n).ok_or_else(|| "the control has a hole".to_owned())?);
    }
    let p_c = median(&period_powers);
    let tp = (4.0 * p_q).max(0.1 * p_c);
    let mut onset = None;
    let mut at = c0 + 20 * MS;
    while at + quarter as i64 * per <= cap.end() {
        if let Some(power) = cap.power(at, quarter) {
            if power >= tp {
                onset = Some(at);
                break;
            }
        }
        at += stride;
    }
    let onset = onset.ok_or_else(|| "the loop delay is not resolved against the control's onset".to_owned())?;
    if (onset - (control_at + d)).abs() > span / 2 {
        return Err("the loop delay is not resolved against the control's onset".to_owned());
    }

    // 4. The control statistics.
    let mut gains = Vec::new();
    let (mut phase_re, mut phase_im) = (0.0, 0.0);
    for p in 1..=19 {
        let (gain, phase) = cap.gain_phase(wave, control_at + d + p as i64 * span).ok_or_else(|| "the control has a hole".to_owned())?;
        gains.push(gain);
        let radians = phase.to_radians();
        phase_re += radians.cos();
        phase_im += radians.sin();
    }
    let g_c = median(&gains);
    let phase_c = phase_im.atan2(phase_re).to_degrees();
    let (g_c1, phase_c1) = cap.gain_phase(wave, control_at + d).ok_or_else(|| "the control has a hole".to_owned())?;
    let t = (2.0 * q).max(0.05 * g_c);
    if g_c1 < 4.0 * t || g_c <= 0.0 {
        return Err("the control is not separable from the floor".to_owned());
    }
    Ok(Control { d, g_c, phase_c, g_c1, phase_c1, p_c, q, p_q, t, tp })
}

/// Whether the window at `tick` is heard (`gain >= ctl.t`); `None` for a hole.
pub fn heard(cap: &Capture, wave: &Wave, ctl: &Control, tick: i64) -> Option<bool> {
    cap.gain_phase(wave, tick).map(|(gain, _)| gain >= ctl.t)
}

/// What a part whose first sample is at `a` reads: its first period, its 19 steady periods, the worst gain ratio and phase difference, and how many
/// periods late its PN starts when its first period is not heard.
#[derive(Debug)]
pub struct PartReading {
    pub w0_gain: f64,
    pub w0_phase: f64,
    pub w0_heard: bool,
    pub steady_ok: bool,
    pub worst_gain_ratio: f64,
    pub worst_phase_deg: f64,
    pub late_by_periods: Option<usize>,
}

/// Reads the part at `a`: the first period at `a + d`, the steady periods 1..=19 after it (1..=9 when the first period is silent: its coverage is ten periods), and the
/// first heard of the 9 periods after a silent first one.
pub fn read_part(cap: &Capture, wave: &Wave, ctl: &Control, a: i64) -> Result<PartReading, String> {
    let (n, per, d) = (cap.n as i64, cap.per, ctl.d);
    let first = a + d;
    let w0 = cap.gain_phase(wave, first);
    let w0_heard = w0.is_some_and(|(gain, _)| gain >= ctl.t);
    let (from, to) = if w0_heard { (first - n * per, first + 20 * n * per) } else { (first, first + 10 * n * per) };
    if !cap.covers(from, to) {
        return Err("hole in the coverage".to_owned());
    }
    let (w0_gain, w0_phase) = w0.ok_or_else(|| "hole in the coverage".to_owned())?;
    let mut steady_ok = true;
    let mut worst_gain_ratio = f64::INFINITY;
    let mut worst_phase_deg = 0.0f64;
    // A heard part is read to its 19th steady period (covered above); a silent first period promises ten periods, so only the nine steady periods inside them are read.
    for p in 1..=if w0_heard { 19 } else { 9 } {
        let (gain, phase) = cap.gain_phase(wave, first + p as i64 * n * per).ok_or_else(|| "hole in the coverage".to_owned())?;
        let ratio = gain / ctl.g_c;
        let difference = angle_difference_deg(phase, ctl.phase_c);
        steady_ok &= gain >= 0.5 * ctl.g_c && difference <= 15.0;
        worst_gain_ratio = worst_gain_ratio.min(ratio);
        worst_phase_deg = worst_phase_deg.max(difference);
    }
    if w0_heard {
        steady_ok &= w0_gain >= 0.5 * ctl.g_c1 && angle_difference_deg(w0_phase, ctl.phase_c1) <= 15.0;
    }
    worst_gain_ratio = worst_gain_ratio.min(w0_gain / ctl.g_c1);
    worst_phase_deg = worst_phase_deg.max(angle_difference_deg(w0_phase, ctl.phase_c1));
    let late_by_periods = if w0_heard { None } else { (1..=9).find(|k| heard(cap, wave, ctl, first + *k as i64 * n * per) == Some(true)).map(|k| k as usize) };
    Ok(PartReading { w0_gain, w0_phase, w0_heard, steady_ok, worst_gain_ratio, worst_phase_deg, late_by_periods })
}

/// The first start tick of a heard window in `[region_from, region_end)`, or `None`. The windows are one whole period on the repeat's lattice, shifted by the
/// loop delay: `[w, w + n*per)` with `w = lattice_tick + ctl.d + j*n*per`, `j` an integer. A window is examined when it starts at or after
/// `region_from + ctl.d + GUARD_TICKS` (a fixed 200 us guard, so a long period cannot eat the gap) and ends at or before `region_end`. The grid must be the
/// lattice's, not one starting at `region_from`: a window off the PN's phase correlates at the PN's off-peak autocorrelation (about `1/sqrt(n)`), which a long
/// period pushes under the threshold even though a leak is there. A region no window fits is an error, not silence.
pub fn silence(cap: &Capture, wave: &Wave, ctl: &Control, region_from: i64, region_end: i64, lattice_tick: i64) -> Result<Option<i64>, String> {
    let (n, per) = (cap.n as i64, cap.per);
    if !cap.covers(region_from + ctl.d - n * per, region_end) {
        return Err("hole in the coverage".to_owned());
    }
    let span = n * per;
    let first = region_from + ctl.d + GUARD_TICKS;
    let j = (first - (lattice_tick + ctl.d) + span - 1).div_euclid(span);
    let mut at = lattice_tick + ctl.d + j * span;
    let mut examined = false;
    while at + span <= region_end {
        examined = true;
        if heard(cap, wave, ctl, at) == Some(true) {
            return Ok(Some(at));
        }
        at += span;
    }
    if examined { Ok(None) } else { Err("no silence window fits the region".to_owned()) }
}

/// The first window whose mean power reaches `ctl.tp`, or `None`. The windows are `n/4` samples (the quiet floor's and the onset's window), stride `n/4`
/// samples, the first at `region_from + ctl.d + GUARD_TICKS` (the same guard as `silence`), and a window is examined only if it ends at or before `region_end`;
/// a window with no coverage is a hole, not silence, and a region no window fits is an error. The window's own power depends on no PN phase: a leak a quarter
/// period off the repeat's lattice, which the correlation reads at the PN's off-peak floor, is heard here. It is the blunter test (threshold `tp`, about -10 dB of
/// the steady power), so it runs second and a leak is whichever finds RF first. INFERRED on the hardware: a burst edge's transient carries power that the steady
/// PN does not.
pub fn quiet(cap: &Capture, ctl: &Control, region_from: i64, region_end: i64) -> Result<Option<(i64, f64)>, String> {
    let quarter = cap.n / 4;
    let stride = quarter as i64 * cap.per;
    let first = region_from + ctl.d + GUARD_TICKS;
    if !cap.covers(first, region_end) {
        return Err("hole in the coverage".to_owned());
    }
    let mut at = first;
    let mut examined = false;
    while at + stride <= region_end {
        examined = true;
        let power = cap.power(at, quarter).ok_or_else(|| "hole in the coverage".to_owned())?;
        if power >= ctl.tp {
            return Ok(Some((at, power)));
        }
        at += stride;
    }
    if examined { Ok(None) } else { Err("no silence window fits the region".to_owned()) }
}

/// Whether at least 5 consecutive period windows heard before `before_tick` show RF in the capture's last 100 ms before it. The phase of the period
/// windows is the one whose window ending last before `before_tick` has the largest gain (the part's lattice is not known here); the run is then
/// counted backwards from it.
pub fn prestall_rf(cap: &Capture, wave: &Wave, ctl: &Control, before_tick: i64) -> bool {
    let (n, per) = (cap.n as i64, cap.per);
    let span = n * per;
    let base = cap.samples.partition_point(|(tick, _)| *tick <= before_tick - span);
    let Some(base) = base.checked_sub(1).map(|index| cap.samples[index].0) else { return false };
    let mut best: Option<(f64, i64)> = None;
    for k in 0..cap.n {
        let start = base - k as i64 * per;
        if let Some((gain, _)) = cap.gain_phase(wave, start) {
            if best.is_none_or(|(largest, _)| gain > largest) {
                best = Some((gain, start));
            }
        }
    }
    let Some((_, last)) = best else { return false };
    let oldest = before_tick - 100 * MS;
    let mut run = 0usize;
    let mut at = last;
    while at >= oldest {
        if heard(cap, wave, ctl, at) == Some(true) {
            run += 1;
            if run >= 5 {
                return true;
            }
        } else {
            return false;
        }
        at -= span;
    }
    false
}

/// The synthetic captures the detector's tests and `c7_verdict`'s fixtures are built from: a dense run of zeros from `c0` to `end_tick`, with a floor,
/// played periods, a DC offset, the fake's idle ramp, and holes.
#[cfg(test)]
pub mod synth {
    use ezsdr_radio_uhd::Iq;

    use super::super::{MCR, c7};
    use super::Capture;

    pub struct Synth {
        samples: Vec<Option<Iq>>,
        per: i64,
        n: usize,
        c0: i64,
        w: Vec<(f32, f32)>,
    }

    impl Synth {
        /// A dense capture of exact zeros from `c0` to `end_tick`, `MCR/rate` ticks a sample, carrying the fake's 100 us PN.
        pub fn new(rate: f64, c0: i64, end_tick: i64) -> Synth {
            Synth::with_period(rate, c7::FAKE_PERIOD_US, c0, end_tick)
        }

        /// A dense capture of exact zeros from `c0` to `end_tick`, `MCR/rate` ticks a sample, carrying a PN of `period_us`.
        pub fn with_period(rate: f64, period_us: i64, c0: i64, end_tick: i64) -> Synth {
            let per = (MCR as f64 / rate) as i64;
            let w = c7::wave(rate, period_us);
            let count = ((end_tick - c0) / per) as usize;
            Synth { samples: vec![Some([0.0, 0.0]); count], per, n: w.len(), c0, w }
        }

        fn range(&self, from: i64, to: i64) -> std::ops::Range<usize> {
            assert!((from - self.c0) % self.per == 0 && (to - self.c0) % self.per == 0, "the fixture's bounds are sample ticks");
            let index = |tick: i64| ((tick - self.c0) / self.per) as usize;
            index(from)..index(to)
        }

        /// Adds deterministic pseudo-noise of rms `amp` a component (a seeded xorshift, uniform over ±amp·√3).
        pub fn floor(&mut self, amp: f64) -> &mut Synth {
            let mut state = 0x9E37_79B9_7F4A_7C15u64;
            for sample in self.samples.iter_mut().flatten() {
                let mut draw = || {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    ((state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * amp * 3.0f64.sqrt()
                };
                sample[0] += draw() as f32;
                sample[1] += draw() as f32;
            }
            self
        }

        fn play_into(&mut self, start: i64, periods: usize, gain: f64, phase_deg: f64, add: bool) {
            let phase = phase_deg.to_radians();
            let (gain_re, gain_im) = (gain * phase.cos(), gain * phase.sin());
            let first = self.range(start, start + (periods * self.n) as i64 * self.per).start;
            for k in 0..periods * self.n {
                let index = first + k;
                let (wr, wi) = self.w[k % self.n];
                let (wr, wi) = (f64::from(wr), f64::from(wi));
                let (re, im) = (gain_re * wr - gain_im * wi, gain_re * wi + gain_im * wr);
                if let Some(sample) = self.samples[index].as_mut() {
                    if add {
                        sample[0] += re as f32;
                        sample[1] += im as f32;
                    } else {
                        *sample = [re as f32, im as f32];
                    }
                }
            }
        }

        /// Adds `gain * e^{j phase} * w[k % n]` at `start_tick + k*per`.
        pub fn play(&mut self, start_tick: i64, periods: usize, gain: f64, phase_deg: f64) -> &mut Synth {
            self.play_into(start_tick, periods, gain, phase_deg, true);
            self
        }

        /// Sets (does not add) `gain * e^{j phase} * w[k % n]` at `start_tick + k*per`: the fake's loopback returns the transmitted sample instead of
        /// the idle ramp, so a played period over the ramp replaces it.
        pub fn play_over_ramp(&mut self, start_tick: i64, periods: usize, gain: f64, phase_deg: f64) -> &mut Synth {
            self.play_into(start_tick, periods, gain, phase_deg, false);
            self
        }

        /// Adds `[value, 0]` to every sample in `[from, to)`.
        pub fn dc(&mut self, from: i64, to: i64, value: f64) -> &mut Synth {
            let range = self.range(from, to);
            for sample in self.samples[range].iter_mut().flatten() {
                sample[0] += value as f32;
            }
            self
        }

        /// Adds the fake's idle ramp, `[((t / per).rem_euclid(65_536)) as f32 / 65_536.0, 0]`, in `[from, to)`.
        pub fn ramp(&mut self, from: i64, to: i64) -> &mut Synth {
            let range = self.range(from, to);
            for (index, sample) in self.samples[range].iter_mut().enumerate() {
                let tick = from + index as i64 * self.per;
                if let Some(sample) = sample {
                    sample[0] += ((tick / self.per).rem_euclid(65_536)) as f32 / 65_536.0;
                }
            }
            self
        }

        /// Removes the samples in `[from, to)`.
        pub fn hole(&mut self, from: i64, to: i64) -> &mut Synth {
            let range = self.range(from, to);
            for sample in &mut self.samples[range] {
                *sample = None;
            }
            self
        }

        /// The `Capture` of the samples still present.
        pub fn capture(&self, fake_ramp: bool) -> Capture {
            let samples = self.samples.iter().enumerate().filter_map(|(index, sample)| sample.map(|sample| (self.c0 + index as i64 * self.per, sample))).collect();
            Capture::new(samples, self.per, self.n, fake_ramp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{MCR, c7};
    use super::synth::Synth;
    use super::*;

    const RATE: f64 = 1e6;
    const PER: i64 = 200;
    const N: usize = 100;
    const DELAY: i64 = 44;
    /// The loop delay in ticks (44 samples).
    const D: i64 = DELAY * PER;
    /// One period in ticks.
    const SPAN: i64 = N as i64 * PER;
    const CONTROL_AT: i64 = 25 * MCR / 1_000;
    const CAPTURE_END: i64 = 150 * MCR / 1_000;

    fn wave() -> Wave {
        Wave::new(&c7::wave(RATE, c7::FAKE_PERIOD_US))
    }

    /// A capture with the control repeat (550 periods from +25 ms, which the control's delay shifts) and nothing else.
    fn control(delay: i64) -> (Capture, Wave, Control) {
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + delay, 550, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        (cap, wave, ctl)
    }

    #[test]
    fn c7_rf_the_standard_control_is_analysed() {
        for delay in [D, 0] {
            let (_, _, ctl) = control(delay);
            assert_eq!(ctl.d, delay, "the loop delay");
            assert_eq!((ctl.g_c, ctl.g_c1), (1.0, 1.0), "the control's steady and first-period gains are the synthesised one");
            assert_eq!((ctl.phase_c, ctl.phase_c1), (0.0, 0.0), "and its phase");
            assert_eq!((ctl.q, ctl.p_q), (0.0, 0.0), "a noise-free zero floor");
            assert_eq!(ctl.t, 0.05 * ctl.g_c, "a zero floor leaves t = 0.05 g_c");
        }

        // A floor large enough that 2 q is the threshold.
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.floor(0.04).play(CONTROL_AT + D, 550, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control over the floor");
        assert!(2.0 * ctl.q > 0.05 * ctl.g_c, "the fixture makes 2 q the larger term: q {}, g_c {}", ctl.q, ctl.g_c);
        assert_eq!(ctl.t, 2.0 * ctl.q, "t = 2 q over the floor");
    }

    #[test]
    fn c7_rf_the_heard_threshold_is_twice_the_floor() {
        // The quiet floor's largest gain q, from the same deterministic noise field the fixture below carries.
        let mut probe = Synth::new(RATE, 0, CAPTURE_END);
        probe.floor(0.04).play(CONTROL_AT + D, 550, 1.0, 0.0);
        let cap = probe.capture(false);
        let wave = wave();
        let q = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control over the floor").q;
        assert!(2.0 * q > 0.05, "the fixture puts the threshold at 2 q: q {q}");

        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.floor(0.04);
        synth.play_over_ramp(100 * MS, 1, 1.5 * q, 0.0);
        synth.play_over_ramp(110 * MS, 1, 2.5 * q, 0.0);
        synth.play(CONTROL_AT + D, 550, 1.0, 0.0);
        let cap = synth.capture(false);
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control over the floor");
        assert_eq!(ctl.q, q, "the floor is the one q was read from");
        assert_eq!(ctl.t, 2.0 * q, "the threshold is twice the floor");
        assert!(1.5 * q > 0.05 * ctl.g_c, "the fixture puts 1.5 q over the 0.05 g_c term: q {q}, g_c {}", ctl.g_c);
        assert_eq!(heard(&cap, &wave, &ctl, 100 * MS), Some(false), "1.5 q is under twice the floor");
        assert_eq!(heard(&cap, &wave, &ctl, 110 * MS), Some(true), "2.5 q is over it");
    }

    #[test]
    fn c7_rf_the_delay_is_anchored_to_the_controls_onset() {
        // A true delay of d + n samples: the PN's periodicity ties the correlation peak at d, a whole period from the onset.
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + (DELAY + N as i64) * PER, 550, 1.0, 0.0);
        let cap = synth.capture(false);
        let error = analyse_control(&cap, &wave(), 0, CONTROL_AT).unwrap_err();
        assert_eq!(error, "the loop delay is not resolved against the control's onset");
    }

    #[test]
    fn c7_rf_the_onset_is_a_power_test_within_a_quarter_period() {
        // The first n/4 window after the quiet interval overlaps 8 of its 25 samples with the PN (32 %): the power test fires there, within a quarter
        // period of the true onset, while a gain test on the full window fires a whole 0.67 period earlier (a sidelobe of the 30 %-overlapped window).
        let control_at = CONTROL_AT - 2 * PER;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(control_at + D, 550, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, control_at).expect("the power test places the onset");
        assert_eq!(ctl.d, D);

        let tp = (4.0 * ctl.p_q).max(0.1 * ctl.p_c);
        let quarter = (N / 4) as i64;
        let mut at = 20 * MS;
        let mut onset = None;
        while at + quarter * PER <= cap.end() {
            if cap.power(at, N / 4).is_some_and(|power| power >= tp) {
                onset = Some(at);
                break;
            }
            at += quarter * PER;
        }
        let onset = onset.expect("a power onset");
        assert!((onset - (control_at + D)).abs() <= quarter * PER, "the onset {onset} is within n/4 of {}", control_at + D);
    }

    #[test]
    fn c7_rf_the_fake_ramp_is_silence_and_a_pn_over_it_is_heard() {
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.ramp(0, CAPTURE_END);
        synth.play_over_ramp(CONTROL_AT + D, 200, 1.0, 0.0);
        let cap = synth.capture(true);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control over the ramp");
        assert_eq!((ctl.q, ctl.p_q), (0.0, 0.0), "the fake's ramp is the baseline: the quiet floor is exactly zero");
        assert_eq!((ctl.g_c, ctl.g_c1), (1.0, 1.0), "the PN replaces the ramp and keeps its gain");
        // The ramp's wrap at sample 65 536 falls inside this window; with the baseline it is silence.
        let wrap = (65_536 - N as i64 / 2) * PER;
        assert_eq!(heard(&cap, &wave, &ctl, wrap), Some(false), "a window over the ramp's wrap");
        for period in 0..20 {
            assert_eq!(heard(&cap, &wave, &ctl, period * SPAN), Some(false), "a ramp window before the control");
        }
    }

    #[test]
    fn c7_rf_the_wave_is_zero_mean_and_a_dc_offset_is_not_heard() {
        for rate in [1e6, 10e6] {
            let wave = Wave::new(&c7::wave(rate, c7::FAKE_PERIOD_US));
            let sum = wave.w.iter().fold((0.0, 0.0), |(re, im), s| (re + f64::from(s[0]), im + f64::from(s[1])));
            assert!(sum.0.abs() < 1e-5 && sum.1.abs() < 1e-5, "{rate}: the wave's components sum to zero (in f32: {sum:?})");
        }
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.dc(100 * MS, 110 * MS, 0.3);
        let cap = synth.capture(false);
        let (gain, _) = cap.gain_phase(&wave(), 100 * MS).expect("a silent window");
        assert!(gain < 1e-7, "a 0.3 DC offset is not heard: gain {gain}");
    }

    #[test]
    fn c7_rf_coverage_and_windows_across_a_hole() {
        let samples: Vec<(i64, Iq)> = [0, 200, 400, 800, 1000].into_iter().map(|tick| (tick, [0.0, 0.0])).collect();
        let cap = Capture::new(samples, PER, 2, false);
        assert!(cap.covers(0, 600) && cap.covers(100, 600), "one run spans the hole's left side");
        assert!(!cap.covers(0, 1200) && !cap.covers(600, 800), "the run ends at 600 and no sample covers 600");
        assert!(cap.window(0).is_some() && cap.window(800).is_some(), "windows inside the runs");
        assert!(cap.window(400).is_none() && cap.window(600).is_none(), "no window across or inside the hole");
        assert_eq!(cap.end(), 1200);
    }

    /// A capture with the control and a part whose PN starts at `a + delay` for `periods` periods.
    fn part(a: i64, delay: i64, periods: usize, gain: f64, phase_deg: f64) -> (Capture, Wave, Control) {
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 550, 1.0, 0.0);
        synth.play(a + delay, periods, gain, phase_deg);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        (cap, wave, ctl)
    }

    #[test]
    fn c7_rf_read_part_at_the_right_place_is_steady() {
        let a = 100 * MS;
        let (cap, wave, ctl) = part(a, D, 20, 1.0, 0.0);
        let part = read_part(&cap, &wave, &ctl, a).expect("the part is covered");
        assert!(part.w0_heard && part.steady_ok);
        assert!(part.worst_gain_ratio > 0.999, "the gain is the control's: {}", part.worst_gain_ratio);
        assert!(part.worst_phase_deg < 1e-6, "the phase is the control's: {}", part.worst_phase_deg);
        assert_eq!(part.late_by_periods, None);
    }

    #[test]
    fn c7_rf_read_part_three_samples_off_the_lattice_is_not_steady() {
        let a = 100 * MS;
        let (cap, wave, ctl) = part(a, D + 3 * PER, 20, 1.0, 0.0);
        let part = read_part(&cap, &wave, &ctl, a).expect("the part is covered");
        assert!(part.w0_gain < 0.5 * ctl.g_c1, "the first period is off the PN: {}", part.w0_gain);
        assert!(!part.steady_ok, "the steady periods are off the PN: {part:?}");
    }

    #[test]
    fn c7_rf_read_part_forty_degrees_off_is_not_steady() {
        let a = 100 * MS;
        let (cap, wave, ctl) = part(a, D, 20, 1.0, 40.0);
        let part = read_part(&cap, &wave, &ctl, a).expect("the part is covered");
        assert!(part.w0_heard, "the gain is the control's");
        assert!(!part.steady_ok, "the phase is 40 degrees off: {}", part.worst_phase_deg);
        assert!(part.worst_phase_deg > 30.0, "the worst phase difference is the part's: {}", part.worst_phase_deg);
    }

    #[test]
    fn c7_rf_read_part_a_silent_period_then_the_pn_is_one_period_late() {
        let a = 100 * MS;
        let (cap, wave, ctl) = part(a, D + SPAN, 20, 1.0, 0.0);
        let part = read_part(&cap, &wave, &ctl, a).expect("the part is covered");
        assert!(!part.w0_heard, "the period at a + d is silent");
        assert!(part.steady_ok, "the periods from a + d + n*per are the PN");
        assert_eq!(part.late_by_periods, Some(1), "the smallest heard period is the first");
    }

    #[test]
    fn c7_rf_read_part_a_silent_part_with_its_ten_promised_periods_is_read_not_a_coverage_error() {
        // bt2 fix round 2, item 7: a silent first period promises ten periods of coverage, and the steady loop reads only the nine inside them. A capture of exactly those
        // ten periods gives the silent-part reading; one sample fewer is still a coverage error.
        let a = 100 * MS;
        let first = a + D;
        let ten = |end: i64| {
            let mut synth = Synth::new(RATE, 0, end);
            synth.play(CONTROL_AT + D, 550, 1.0, 0.0);
            let cap = synth.capture(false);
            let wave = wave();
            let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
            (read_part(&cap, &wave, &ctl, a), cap.covers(first, first + 10 * SPAN))
        };
        let (reading, covered) = ten(first + 10 * SPAN);
        assert!(covered);
        let part = reading.expect("ten periods cover a silent part");
        assert!(!part.w0_heard, "{part:?}");
        assert_eq!(part.late_by_periods, None, "nothing is heard in the ten periods");
        let (reading, covered) = ten(first + 10 * SPAN - PER);
        assert!(!covered);
        assert_eq!(reading.unwrap_err(), "hole in the coverage");
    }

    #[test]
    fn c7_rf_the_noise_floor_term_of_the_power_threshold_binds_over_a_tenth_of_the_steady_power() {
        // bt2 fix round 2, item 8 (N4): `tp = max(4 p_q, 0.1 p_c)`. With a noise floor of rms 0.09 a component the floor term is the larger: a leak at gain 0.4 (power
        // 0.16 of the PN's, over the floor) is above a tenth of the steady power and below four times the floor, so it is not heard; one at gain 1.0 is.
        let a = 100 * MS;
        let region_from = a + SPAN / 2;
        let region_end = a + 10 * SPAN;
        let leak = a + D + SPAN;
        let analyse = |gain: f64| {
            let mut synth = Synth::new(RATE, 0, CAPTURE_END);
            synth.floor(0.09).play(CONTROL_AT + D, 550, 1.0, 0.0);
            synth.play(leak, 3, gain, 0.0);
            let cap = synth.capture(false);
            let wave = wave();
            let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
            let power = cap.power(leak, N / 4).expect("the leak is covered");
            (ctl.tp, ctl.p_q, ctl.p_c, power, quiet(&cap, &ctl, region_from, region_end))
        };
        let (tp, p_q, p_c, power, heard) = analyse(0.4);
        println!("C7 quiet floor term: tp={tp:.4e} 4pq={:.4e} 0.1pc={:.4e} leak(0.4)={power:.4e}", 4.0 * p_q, 0.1 * p_c);
        assert!(4.0 * p_q > 0.1 * p_c, "the floor term binds: 4 p_q {} against 0.1 p_c {}", 4.0 * p_q, 0.1 * p_c);
        assert_eq!(tp, 4.0 * p_q);
        assert!(0.1 * p_c < power && power < tp, "the leak is between the two terms: {} < {power} < {tp}", 0.1 * p_c);
        assert_eq!(heard, Ok(None), "below the floor term it is not heard, though it is above a tenth of the steady power");
        let (tp_loud, _, _, power_loud, heard_loud) = analyse(1.0);
        assert!(power_loud >= tp_loud, "{power_loud} >= {tp_loud}");
        assert!(heard_loud.expect("covered").is_some_and(|(_, power)| power >= tp_loud), "a leak at the PN's own gain is heard");
    }

    #[test]
    fn c7_rf_silence_finds_pn_leakage_inside_the_region() {
        let region_from = 100 * MS;
        let region_end = 110 * MS;
        let leak = region_from + D + 2 * SPAN;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.play(leak, 2, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Ok(Some(leak)), "the PN leakage is heard");
    }

    #[test]
    fn c7_rf_silence_catches_a_part_heard_in_the_window_before_its_start() {
        let a = 100 * MS;
        let region_from = a - 3 * SPAN;
        let region_end = a + D;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.play(a + D - SPAN, 1, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, a), Ok(Some(a + D - SPAN)), "the last window before a + d, on the lattice");
    }

    #[test]
    fn c7_rf_silence_ignores_the_fake_ramp_and_the_guard() {
        // The fake's ramp with its wrap inside the region, baseline on: silent.
        let (region_from, region_end) = (60 * MS, 80 * MS);
        assert!(region_from <= 65_536 * PER && 65_536 * PER < region_end, "the ramp wraps inside the region");
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.ramp(0, CAPTURE_END);
        synth.play_over_ramp(CONTROL_AT + D, 200, 1.0, 0.0);
        let cap = synth.capture(true);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control over the ramp");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Ok(None), "the ramp is silence");

        // A heard window inside the 200 us guard after region_from + d (one period of the fake's 100 us) is not examined.
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.play(region_from + D + SPAN, 1, 1.0, 0.0);
        let cap = synth.capture(false);
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Ok(None), "the 200 us guard is not examined");
    }

    #[test]
    fn c7_rf_silence_stops_at_the_region_end() {
        let region_from = 100 * MS;
        let region_end = region_from + D + 2 * SPAN + 390 * (N as i64 / 4) * PER;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.play(region_end, 20, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Ok(None), "the PN starts exactly at the region's end");
    }

    #[test]
    fn c7_rf_silence_refuses_a_hole() {
        let (region_from, region_end) = (100 * MS, 110 * MS);
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.hole(105 * MS, 105 * MS + PER);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Err("hole in the coverage".to_owned()));
    }

    #[test]
    fn c7_rf_silence_windows_follow_the_lattice_not_the_region_start() {
        // The region starts half a period off the repeat's lattice; the windows stay on the lattice, so the leak is heard at a lattice tick shifted by d.
        let a = 100 * MS;
        let region_from = a + SPAN / 2;
        let region_end = a + 10 * SPAN;
        let leak = a + D + 3 * SPAN;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        synth.play(leak, 5, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, a), Ok(Some(leak)), "the leak is found on the lattice, not on a grid from region_from");
    }

    #[test]
    fn c7_rf_silence_at_a_one_millisecond_period_hears_an_aligned_leak_and_not_an_unaligned_one() {
        // A window off the PN's phase reads the off-peak autocorrelation, about 1/sqrt(n): 0.1 at n = 100 (above t), 0.03 at n = 1 000 (below it). So the
        // scan must use the repeat's lattice: a leak on it is heard, and the same leak a quarter period off it is not heard at all. The power test reads
        // both, because its window is not the PN's.
        let rate = 1e6;
        let period_us = c7::HW_PERIOD_US;
        let n = c7::period(rate, period_us) as i64;
        let span = n * PER;
        let a = 100 * MS;
        let region_from = a + span / 2;
        let region_end = a + 10 * span;
        let leak = a + D + span;
        let analyse = |leak: i64| {
            let mut synth = Synth::with_period(rate, period_us, 0, 120 * MS);
            synth.floor(1e-5).play(CONTROL_AT + D, 30, 1.0, 0.0);
            synth.play(leak, 3, 1.0, 0.0);
            let cap = synth.capture(false);
            let wave = Wave::new(&c7::wave(rate, period_us));
            let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
            (silence(&cap, &wave, &ctl, region_from, region_end, a), quiet(&cap, &ctl, region_from, region_end), ctl.tp)
        };
        let (on_lattice, quiet_on_lattice, tp) = analyse(leak);
        let (off_lattice, quiet_off_lattice, _) = analyse(leak + span / 4);
        assert_eq!(on_lattice, Ok(Some(leak)), "the leak on the lattice is heard");
        assert_eq!(off_lattice, Ok(None), "the same leak a quarter period off the lattice is not heard");
        let quarter_window = (n / 4) * PER;
        for (at, power_scan) in [(leak, quiet_on_lattice), (leak + span / 4, quiet_off_lattice)] {
            let (tick, power) = power_scan.expect("the power scan reads the region").expect("the power scan hears the leak whatever its phase");
            assert!(tick >= at - quarter_window && tick < at + 3 * span, "the window at {tick} starts around the leak at {at}");
            assert!(power >= tp, "the leak's power {power} reaches the threshold {tp}");
        }
    }

    #[test]
    fn c7_rf_quiet_threshold_both_sides() {
        // The threshold is a tenth of the PN's steady power: a leak at gain 0.30 reads 0.09 of it (below), one at 0.33 reads 0.109 (above). The measured
        // threshold and leak power are asserted, not assumed.
        let rate = 1e6;
        let period_us = c7::HW_PERIOD_US;
        let n = c7::period(rate, period_us) as i64;
        let span = n * PER;
        let a = 100 * MS;
        let region_from = a + span / 2;
        let region_end = a + 10 * span;
        let leak = a + D + span;
        let analyse = |gain: f64| {
            let mut synth = Synth::with_period(rate, period_us, 0, 120 * MS);
            synth.floor(1e-5).play(CONTROL_AT + D, 30, 1.0, 0.0);
            synth.play(leak, 3, gain, 0.0);
            let cap = synth.capture(false);
            let wave = Wave::new(&c7::wave(rate, period_us));
            let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
            (ctl.tp, cap.power(leak, n as usize / 4).expect("the leak is covered"), quiet(&cap, &ctl, region_from, region_end))
        };
        let (tp_below, power_below, heard_below) = analyse(0.30);
        let (tp_above, power_above, heard_above) = analyse(0.33);
        println!("C7 quiet: tp={tp_below:.4e} 0.30={power_below:.4e} 0.33={power_above:.4e} tp(0.33)={tp_above:.4e}");
        assert!(power_below < tp_below && tp_below < power_above, "the threshold lies between the two leaks: {power_below} < {tp_below} < {power_above}");
        assert_eq!(heard_below, Ok(None), "a leak at 0.30 is under the threshold");
        assert!(heard_above.expect("the region is covered").is_some_and(|(_, power)| power >= tp_above), "a leak at 0.33 is heard");
    }

    #[test]
    fn c7_rf_quiet_refuses_a_hole_and_a_region_no_window_fits() {
        let rate = 1e6;
        let period_us = c7::HW_PERIOD_US;
        let n = c7::period(rate, period_us) as i64;
        let (region_from, region_end) = (100 * MS, 120 * MS);
        let mut synth = Synth::with_period(rate, period_us, 0, CAPTURE_END);
        synth.floor(1e-5).play(CONTROL_AT + D, 30, 1.0, 0.0);
        synth.hole(110 * MS, 110 * MS + PER);
        let cap = synth.capture(false);
        let wave = Wave::new(&c7::wave(rate, period_us));
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(quiet(&cap, &ctl, region_from, region_end), Err("hole in the coverage".to_owned()));

        // Shorter than the guard plus one window: nothing was looked at, so it is an error, not silence.
        let mut synth = Synth::with_period(rate, period_us, 0, CAPTURE_END);
        synth.floor(1e-5).play(CONTROL_AT + D, 30, 1.0, 0.0);
        let cap = synth.capture(false);
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        let region_end = region_from + D + GUARD_TICKS + (n / 4) * PER / 2;
        assert_eq!(quiet(&cap, &ctl, region_from, region_end), Err("no silence window fits the region".to_owned()));
    }

    #[test]
    fn c7_rf_silence_with_no_window_is_an_error_not_silence() {
        // Shorter than the guard plus one period: nothing was looked at, so the scan cannot call it silence.
        let region_from = 100 * MS;
        let region_end = region_from + D + GUARD_TICKS + SPAN / 2;
        let mut synth = Synth::new(RATE, 0, CAPTURE_END);
        synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = wave();
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(silence(&cap, &wave, &ctl, region_from, region_end, region_from), Err("no silence window fits the region".to_owned()));
    }

    #[test]
    fn c7_rf_the_lag_search_finds_the_loop_delay_at_a_long_period() {
        // The 4-period prefix search at a 1 ms PN: n = 1 000 at 1 MS/s, and n = 10 000 at 10 MS/s.
        let mut synth = Synth::with_period(RATE, c7::HW_PERIOD_US, 0, 60 * MS);
        synth.play(CONTROL_AT + D, 30, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = Wave::new(&c7::wave(RATE, c7::HW_PERIOD_US));
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(ctl.d, D, "the loop delay at n = {}", c7::period(RATE, c7::HW_PERIOD_US));

        let rate = 10e6;
        let per = (MCR as f64 / rate) as i64;
        let mut synth = Synth::with_period(rate, c7::HW_PERIOD_US, 0, 60 * MS);
        synth.play(CONTROL_AT + 44 * per, 30, 1.0, 0.0);
        let cap = synth.capture(false);
        let wave = Wave::new(&c7::wave(rate, c7::HW_PERIOD_US));
        let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
        assert_eq!(ctl.d, 44 * per, "the loop delay at n = 10 000");
    }

    #[test]
    fn c7_rf_prestall_rf_counts_five_heard_periods_before_the_stall() {
        let a = 100 * MS;
        let prestall = |periods: usize, before: i64| {
            let mut synth = Synth::new(RATE, 0, CAPTURE_END);
            synth.play(CONTROL_AT + D, 200, 1.0, 0.0);
            synth.play(a, periods, 1.0, 0.0);
            let cap = synth.capture(false);
            let wave = wave();
            let ctl = analyse_control(&cap, &wave, 0, CONTROL_AT).expect("the control is analysed");
            prestall_rf(&cap, &wave, &ctl, before)
        };
        assert!(prestall(5, a + 5 * SPAN), "5 heard periods before the stall");
        assert!(!prestall(4, a + 4 * SPAN), "4 are not 5");
        assert!(!prestall(5, a + 4 * SPAN), "a period ending after before_tick does not count");
    }
}
