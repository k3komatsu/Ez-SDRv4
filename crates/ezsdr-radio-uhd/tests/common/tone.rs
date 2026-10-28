//! The probes' tone classifier (design note 42): where a received 100 kHz tone first leaves its
//! frequency, judged by coherence and by power against the noise before the burst, so that a tone the
//! loopback attenuates to about 0.008 is seen and noise alone is not taken for one.

#![allow(dead_code)]

/// Ticks of the master clock in a millisecond (200 MHz).
const MS: i64 = 200_000;

/// What a capture says about a tone that may shift at `e` (design note 42's probes): the windows
/// of 25 samples (a tone's coherence, not its magnitude: the loopback attenuates a 0.4 tone to
/// about 0.008), the power of the noise before the burst, how many windows carry the tone on
/// each side of `e`, and where the tone first leaves `base_hz`.
#[derive(Debug, PartialEq)]
pub struct ToneReport {
    /// Mean power of the windows before the burst: what noise looks like.
    pub floor: Option<f64>,
    /// The power a window must exceed (with coherence over 0.5) to carry the tone: 4 times the floor.
    pub threshold: f64,
    /// Windows carrying the tone that end by `e`, and that begin 5 ms or more after it.
    pub before: usize,
    pub after: usize,
    /// How many windows lie on each side, tone or not: a side is seen when most of them carry it.
    pub before_of: usize,
    pub after_of: usize,
    /// The first of three consecutive tone windows more than 25 kHz from `base_hz`, in ticks from `e`.
    pub moves_at: Option<i64>,
    /// The tone is seen to have moved, but samples are missing between the last window at the
    /// base frequency and the first shifted one (or there is no base window): the time of the
    /// move is not known, only that it lies in that interval.
    pub gapped: bool,
    /// The first three shifted windows begin the burst's first inspected window: `moves_at` is then only an upper bound
    /// (the tone moved at or before it, D9 F7).
    pub at_or_before: bool,
}

impl ToneReport {
    /// Whether the capture can say anything: a noise floor was measured and the tone is seen on
    /// both sides of `e`.
    pub fn usable(&self) -> bool {
        !self.gapped && self.floor.is_some() && self.before_of >= 10 && self.after_of >= 10 && self.before * 2 >= self.before_of && self.after * 2 >= self.after_of
    }
}

/// A window's coherence (the length of its summed lag-1 products over the sum of their lengths:
/// 1 for a pure tone, about 1/√n for noise) and its frequency at 1 Msps.
pub fn window_tone(w: &[(i64, ezsdr_radio_uhd::Iq)]) -> (f64, f64) {
    let (mut re, mut im, mut norm) = (0.0f64, 0.0f64, 0.0f64);
    for pair in w.windows(2) {
        let (a, b) = (pair[0].1, pair[1].1);
        let (re_k, im_k) = (f64::from(b[0] * a[0] + b[1] * a[1]), f64::from(b[1] * a[0] - b[0] * a[1]));
        re += re_k;
        im += im_k;
        norm += re_k.hypot(im_k);
    }
    (if norm > 0.0 { re.hypot(im) / norm } else { 0.0 }, im.atan2(re) / (2.0 * std::f64::consts::PI) * 1e6)
}

/// Classifies `samples` (ticks of 200 a sample) around a burst `[burst_start, burst_end)` and a
/// possible shift at `e`: the noise floor from the windows before the burst, signal windows inside
/// it with coherence over 0.5 and power over 4 × floor, and the first run of three off `base_hz`.
pub fn classify_tone(samples: &[(i64, ezsdr_radio_uhd::Iq)], burst_start: i64, burst_end: i64, e: i64, base_hz: f64) -> ToneReport {
    const SPAN: i64 = 25 * 200;
    // A window is 25 samples one tick spacing apart: one that straddles a hole in the capture
    // (the packets' timestamps jump across missing samples) is dropped.
    let windows: Vec<(i64, f64, f64, f64)> = samples.chunks_exact(25).filter(|w| w[24].0 - w[0].0 == 24 * 200).map(|w| {
        let (coherence, hz) = window_tone(w);
        (w[0].0, coherence, hz, w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 25.0)
    }).collect();
    // Noise at the sc16 LSB (the bench's 0 dB receive gain) reaches any coherence up to 1: the floor is a power.
    let quiet: Vec<f64> = windows.iter().filter(|(t, ..)| t + SPAN <= burst_start - MS / 2).map(|(.., power)| *power).collect();
    let floor = (quiet.len() >= 10).then(|| quiet.iter().sum::<f64>() / quiet.len() as f64);
    let threshold = floor.map_or(0.0, |floor| 4.0 * floor);
    let inside: Vec<&(i64, f64, f64, f64)> = windows.iter().filter(|(t, ..)| *t >= burst_start + MS && t + SPAN <= burst_end).collect();
    let signal: Vec<&(i64, f64, f64, f64)> = inside.iter().copied().filter(|(_, c, _, power)| *c > 0.5 && *power > threshold).collect();
    let first_shifted = signal.windows(3).position(|w| w.iter().all(|(_, _, hz, _)| (hz - base_hz).abs() > 25e3) && w[2].0 - w[0].0 == 2 * SPAN);
    // The move is dated only when the window before the first shifted one is a base window that
    // ends where the shifted one begins; a hole or a run of windows that carry no tone is a gap.
    // Off the base already in the burst's first window is a move at or before it (D9 F7), not a gap.
    let gapped = first_shifted.is_some_and(|i| if i == 0 { signal[0].0 != inside[0].0 } else { signal[i].0 - (signal[i - 1].0 + SPAN) > 0 });
    ToneReport {
        floor,
        gapped,
        at_or_before: !gapped && first_shifted == Some(0),
        threshold,
        before: signal.iter().filter(|(t, ..)| t + SPAN <= e).count(),
        after: signal.iter().filter(|(t, ..)| *t >= e + 5 * MS).count(),
        before_of: inside.iter().filter(|(t, ..)| t + SPAN <= e).count(),
        after_of: inside.iter().filter(|(t, ..)| *t >= e + 5 * MS).count(),
        // Three windows in a row more than 25 kHz off (the probes' shifts are 50 kHz): one noisy
        // window is not a move.
        moves_at: first_shifted.filter(|_| !gapped).map(|i| signal[i].0 - e),
    }
}

/// `classify_tone`'s verdict for a probe's line: the move, or why there is none to report.
pub fn tone_verdict(report: &ToneReport) -> String {
    if report.gapped {
        format!("INCONCLUSIVE: the tone moved, but samples are missing between the last window at the base frequency and the first shifted one (noise power {:?}, threshold {:.1e})", report.floor, report.threshold)
    } else if report.usable() {
        format!("the tone leaves 100 kHz {} {:?} µs from e (signal windows {} of {} before e, {} of {} after; noise power {:?}, threshold {:.1e})", if report.at_or_before { "at or before" } else { "at" }, report.moves_at.map(|t| t as f64 / 200.0), report.before, report.before_of, report.after, report.after_of, report.floor, report.threshold)
    } else {
        format!("INCONCLUSIVE: no usable signal windows (before e {} of {}, after e {} of {}, noise power {:?}, threshold {:.1e})", report.before, report.before_of, report.after, report.after_of, report.floor, report.threshold)
    }
}

/// The loop's phase for a burst of `wave` that starts at `offset` in `samples` (a cabled loop turns it by its two LOs' relative
/// phase): the degrees over the whole burst and over each half of it, and the loop's gain as a fraction of the waveform's
/// energy. `None` when the capture does not hold the burst or the waveform has no energy.
#[derive(Debug, PartialEq)]
pub struct BurstPhase {
    pub deg: f64,
    pub halves_deg: [f64; 2],
    pub gain: f64,
}

pub fn burst_phase(samples: &[(f32, f32)], wave: &[(f32, f32)], offset: usize) -> Option<BurstPhase> {
    let held = samples.get(offset..offset.checked_add(wave.len())?)?;
    let correlate = |range: std::ops::Range<usize>| {
        let (mut re, mut im, mut energy) = (0.0f64, 0.0f64, 0.0f64);
        for ((wr, wi), (sr, si)) in wave[range.clone()].iter().zip(&held[range]) {
            let (wr, wi, sr, si) = (f64::from(*wr), f64::from(*wi), f64::from(*sr), f64::from(*si));
            re += wr * sr + wi * si;
            im += wr * si - wi * sr;
            energy += wr * wr + wi * wi;
        }
        (re, im, energy)
    };
    let half = wave.len() / 2;
    let (re, im, energy) = correlate(0..wave.len());
    if energy <= 0.0 || half == 0 { return None; }
    let (first, second) = (correlate(0..half), correlate(half..wave.len()));
    let deg = |(re, im, _): (f64, f64, f64)| im.atan2(re).to_degrees();
    Some(BurstPhase { deg: im.atan2(re).to_degrees(), halves_deg: [deg(first), deg(second)], gain: re.hypot(im) / energy })
}

/// The probe's verdict on a burst's phase: the degrees, or why none can be read (INFERRED INCONCLUSIVE): a loop too weak
/// (gain under 0.002 of the waveform's energy; the bench's 30 dB loop gives about 0.015) or a phase that moves across the burst.
pub fn phase_verdict(phase: &BurstPhase) -> Result<f64, String> {
    if phase.gain < 0.002 { return Err(format!("the loop's gain {:.4} is too weak to read a phase", phase.gain)); }
    if !phase_is_steady(phase) { return Err(format!("the phase is not steady across the burst ({:.1} and {:.1} degrees)", phase.halves_deg[0], phase.halves_deg[1])); }
    Ok(phase.deg)
}

/// Whether the phase holds across the burst: its two halves within 15 degrees of each other.
pub fn phase_is_steady(phase: &BurstPhase) -> bool {
    angle_difference_deg(phase.halves_deg[0], phase.halves_deg[1]) <= 15.0
}

/// The difference of two angles in degrees, 0 to 180.
pub fn angle_difference_deg(a: f64, b: f64) -> f64 {
    ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs()
}

/// The largest difference between any two of `phases` (degrees); `None` for fewer than two.
pub fn circular_spread_deg(phases: &[f64]) -> Option<f64> {
    if phases.len() < 2 { return None; }
    Some(phases.iter().flat_map(|a| phases.iter().map(move |b| angle_difference_deg(*a, *b))).fold(0.0, f64::max))
}

/// How many of a starved burst's continuation samples reached the loopback (P6): those from 100 samples after `next` for
/// 1 900 samples (the loop's delay is 44 samples) whose |I| + |Q| exceeds half the median over the queued part
/// `[start + 1 ms, queued_end)` (the loop's tone is about 0.002, far under a fixed threshold). `Err` when the queued
/// part gives no baseline (under 10 000 samples, or a zero median) or the continuation's interval is not captured
/// (under 1 500 of its samples): a count of 0 then says nothing about the loopback.
pub fn continuation_in_loopback(samples: &[(i64, ezsdr_radio_uhd::Iq)], start: i64, queued_end: i64, next: i64) -> Result<usize, String> {
    let mut queued: Vec<f32> = samples.iter().filter(|(t, _)| *t >= start + MS && *t < queued_end).map(|(_, s)| s[0].abs() + s[1].abs()).collect();
    queued.sort_by(f32::total_cmp);
    let median = queued.get(queued.len() / 2).copied().unwrap_or(0.0);
    if queued.len() < 10_000 || median <= 0.0 { return Err(format!("INCONCLUSIVE: no baseline from the queued part ({} samples, median {median})", queued.len())); }
    let covered = samples.iter().filter(|(t, _)| *t >= next + 100 * 200 && *t < next + 2_000 * 200).count();
    if covered < 1_500 { return Err(format!("INCONCLUSIVE: the continuation's interval was captured in only {covered} samples")); }
    Ok(samples.iter().filter(|(t, s)| *t >= next + 100 * 200 && *t < next + 2_000 * 200 && s[0].abs() + s[1].abs() > median / 2.0).count())
}
