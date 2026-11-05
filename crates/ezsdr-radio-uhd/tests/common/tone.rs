//! The probes' tone classifier (design note 42): where a received 100 kHz tone first leaves its
//! frequency, judged by coherence and by power against the noise before the burst, so that a tone the
//! loopback attenuates to about 0.008 is seen and noise alone is not taken for one.

#![allow(dead_code)]

/// Ticks of the master clock in a millisecond (200 MHz).
const MS: i64 = 200_000;

/// The master clock, in hertz.
pub const MCR_HZ: f64 = 200_000_000.0;

/// Ticks a sample at 1 Msps.
pub const PER_1MSPS: i64 = 200;

/// Ticks of a 25-sample window at 1 Msps.
const SPAN: i64 = 25 * 200;

/// The outcome of a claim about the tone: PASS and INCONCLUSIVE pass the test; FAIL is evidence against the claim.
#[derive(Debug)]
pub enum Verdict {
    Pass(String),
    Inconclusive(String),
    Fail(String),
}

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
    window_tone_at(w, PER_1MSPS)
}

/// `window_tone` at `ticks_per_sample` root ticks a sample (T4: the receive rate changes while the
/// transmit tone's hertz stays the same).
pub fn window_tone_at(w: &[(i64, ezsdr_radio_uhd::Iq)], ticks_per_sample: i64) -> (f64, f64) {
    let (mut re, mut im, mut norm) = (0.0f64, 0.0f64, 0.0f64);
    for pair in w.windows(2) {
        let (a, b) = (pair[0].1, pair[1].1);
        let (re_k, im_k) = (f64::from(b[0] * a[0] + b[1] * a[1]), f64::from(b[1] * a[0] - b[0] * a[1]));
        re += re_k;
        im += im_k;
        norm += re_k.hypot(im_k);
    }
    let hz = im.atan2(re) / (2.0 * std::f64::consts::PI) * (MCR_HZ / ticks_per_sample as f64);
    (if norm > 0.0 { re.hypot(im) / norm } else { 0.0 }, hz)
}

/// Classifies `samples` (ticks of 200 a sample) around a burst `[burst_start, burst_end)` and a
/// possible shift at `e`: the noise floor from the windows before the burst, signal windows inside
/// it with coherence over 0.5 and power over 4 × floor, and the first run of three off `base_hz`.
pub fn classify_tone(samples: &[(i64, ezsdr_radio_uhd::Iq)], burst_start: i64, burst_end: i64, e: i64, base_hz: f64) -> ToneReport {
    let windows: Vec<(i64, f64, f64, f64)> = contiguous_windows(samples)
        .map(|w| {
            let (coherence, hz) = window_tone(w);
            (w[0].0, coherence, hz, w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 25.0)
        })
        .collect();
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
    let gapped = first_shifted.is_some_and(|i| if i == 0 { signal[0].0 != inside[0].0 } else { !move_is_dated(signal[i - 1].0, signal[i].0) });
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

/// The 25-sample windows of `samples` whose ticks are one spacing apart: a window that straddles a hole in the capture (the
/// packets' timestamps jump across missing samples) is dropped.
pub fn contiguous_windows(samples: &[(i64, ezsdr_radio_uhd::Iq)]) -> impl Iterator<Item = &[(i64, ezsdr_radio_uhd::Iq)]> {
    contiguous_windows_at(samples, PER_1MSPS)
}

/// `contiguous_windows` at `ticks_per_sample` root ticks a sample (T4: a receive rate change).
pub fn contiguous_windows_at(samples: &[(i64, ezsdr_radio_uhd::Iq)], ticks_per_sample: i64) -> impl Iterator<Item = &[(i64, ezsdr_radio_uhd::Iq)]> {
    samples.chunks_exact(25).filter(move |w| w[24].0 - w[0].0 == 24 * ticks_per_sample)
}

/// The mean power of the 25-sample windows in `[from, to)`, at `ticks_per_sample` root ticks a sample: the noise a tone
/// is looked for against, as `classify_tone` measures it; `None` when the stretch holds fewer than ten windows.
pub fn window_power_at(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, to: i64, ticks_per_sample: i64) -> Option<f64> {
    let span = 25 * ticks_per_sample;
    let powers: Vec<f64> = contiguous_windows_at(samples, ticks_per_sample)
        .filter(|w| w[0].0 >= from && w[0].0 + span <= to)
        .map(|w| w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 25.0)
        .collect();
    (powers.len() >= 10).then(|| powers.iter().sum::<f64>() / powers.len() as f64)
}

/// `window_power_at` at 1 Msps.
pub fn window_power(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, to: i64) -> Option<f64> {
    window_power_at(samples, from, to, PER_1MSPS)
}

/// One millisecond of a segment's tone: the median window frequency, the window furthest from that median (a wrong
/// initial tone shows here even when a later majority is right) and the mean power.
#[derive(Debug, PartialEq)]
pub struct FirstMillisecond {
    pub hz: f64,
    pub worst_hz: f64,
    pub power: f64,
}

/// The first millisecond of `samples` from `from`, at `ticks_per_sample` root ticks a sample: every 25-sample window
/// that covers `[from, from + 1 ms)` from the window containing `from` through the window ending at or after
/// `from + 1 ms` must be present, consecutive and coherent (over 0.5); `None` when the stretch is not fully covered
/// this way (a missing beginning or ending is then not read as a tone).
pub fn first_ms_tone(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, ticks_per_sample: i64) -> Option<FirstMillisecond> {
    let span = 25 * ticks_per_sample;
    let mut windows: Vec<(i64, f64, f64)> = contiguous_windows_at(samples, ticks_per_sample)
        .filter(|w| w[0].0 + span > from && w[0].0 < from + MS)
        .map(|w| {
            let (coherence, hz) = window_tone_at(w, ticks_per_sample);
            let power = w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / 25.0;
            (w[0].0, coherence, hz, power)
        })
        .filter(|(_, coherence, _, _)| *coherence > 0.5)
        .map(|(start, _, hz, power)| (start, hz, power))
        .collect();
    // The first window must already cover `from` (else the beginning is missing), the windows must be consecutive,
    // and the last must reach `from + 1 ms` (else the end is missing).
    if windows.first().is_none_or(|(start, ..)| *start > from) || windows.last().is_none_or(|(start, ..)| start + span < from + MS) || windows.windows(2).any(|pair| pair[1].0 != pair[0].0 + span) {
        return None;
    }
    windows.sort_by_key(|(start, ..)| *start);
    let mut freqs: Vec<f64> = windows.iter().map(|(_, hz, _)| *hz).collect();
    freqs.sort_by(f64::total_cmp);
    let hz = freqs[freqs.len() / 2];
    let worst_hz = freqs.iter().fold(0.0f64, |worst, f| worst.max((f - hz).abs()));
    let power = windows.iter().map(|(_, _, power)| *power).sum::<f64>() / windows.len() as f64;
    Some(FirstMillisecond { hz, worst_hz, power })
}

/// The median frequency of `first_ms_tone`, or why there is none.
pub fn first_ms_frequency(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, ticks_per_sample: i64) -> Option<f64> {
    first_ms_tone(samples, from, ticks_per_sample).map(|first| first.hz)
}

/// What a receive segment's first millisecond says of a tone `expected_hz` from `origin`, at `ticks_per_sample` a sample:
/// the stage-3 restart's evidence at the segment's first window (the tone comes with the segment, so only the first
/// millisecond tells whether the configure settled before it). `floor` is the quiet power measured separately, never
/// from an interval in which the receive was stopped; the loopback's delay is not read into this verdict.
pub fn first_window_verdict(blocks: &[(i64, ezsdr_radio_uhd::Iq)], origin: i64, ticks_per_sample: i64, expected_hz: f64, tolerance_hz: f64, floor: f64) -> Verdict {
    match first_ms_tone(blocks, origin, ticks_per_sample) {
        Some(first) if first.power <= 4.0 * floor => Verdict::Fail(format!("the segment's first window power {:.3e} is not above 4 x the noise {floor:.3e}", first.power)),
        Some(first) if (first.hz - expected_hz).abs() > tolerance_hz => Verdict::Fail(format!("the segment's first millisecond holds {:.0} Hz, not {expected_hz:.0} Hz", first.hz)),
        Some(first) if first.worst_hz > tolerance_hz => {
            Verdict::Fail(format!("a window of the first millisecond is {:.0} Hz from its median {:.0} Hz: the tone was not settled at the segment's beginning", first.worst_hz, first.hz))
        }
        Some(first) => Verdict::Pass(format!("every window of the segment's first millisecond holds the tone at {:.0} Hz, above 4 x the noise", first.hz)),
        None => Verdict::Inconclusive("the segment's first millisecond is not covered by consecutive coherent windows from its beginning through its end".to_owned()),
    }
}

/// Whether a move whose first shifted window starts at `at` is dated by the window before it, which starts at `before`: that
/// window must end where `at` begins. A hole or a window without the tone between them leaves the move undated.
fn move_is_dated(before: i64, at: i64) -> bool {
    at == before + SPAN
}

/// The median window frequency in `[from, to)`.
pub fn median_freq(freqs: &[(i64, f64)], from: i64, to: i64) -> Option<f64> {
    let mut f: Vec<f64> = freqs.iter().filter(|(t, _)| *t >= from && *t < to).map(|(_, f)| *f).collect();
    f.sort_by(f64::total_cmp);
    f.get(f.len() / 2).copied()
}

/// `classify_tone`'s verdict for a probe's line: the move, or why there is none to report.
pub fn tone_verdict(report: &ToneReport) -> String {
    if report.gapped {
        format!(
            "INCONCLUSIVE: the tone moved, but samples are missing between the last window at the base frequency and the first shifted one (noise power {:?}, threshold {:.1e})",
            report.floor, report.threshold
        )
    } else if report.usable() {
        format!(
            "the tone leaves 100 kHz {} {:?} µs from e (signal windows {} of {} before e, {} of {} after; noise power {:?}, threshold {:.1e})",
            if report.at_or_before { "at or before" } else { "at" },
            report.moves_at.map(|t| t as f64 / 200.0),
            report.before,
            report.before_of,
            report.after,
            report.after_of,
            report.floor,
            report.threshold
        )
    } else {
        format!(
            "INCONCLUSIVE: no usable signal windows (before e {} of {}, after e {} of {}, noise power {:?}, threshold {:.1e})",
            report.before, report.before_of, report.after, report.after_of, report.floor, report.threshold
        )
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
    if energy <= 0.0 || half == 0 {
        return None;
    }
    let (first, second) = (correlate(0..half), correlate(half..wave.len()));
    let deg = |(re, im, _): (f64, f64, f64)| im.atan2(re).to_degrees();
    Some(BurstPhase { deg: im.atan2(re).to_degrees(), halves_deg: [deg(first), deg(second)], gain: re.hypot(im) / energy })
}

/// The probe's verdict on a burst's phase: the degrees, or why none can be read (INFERRED INCONCLUSIVE): a loop too weak
/// (gain under 0.002 of the waveform's energy; the bench's 30 dB loop gives about 0.015) or a phase that moves across the burst.
pub fn phase_verdict(phase: &BurstPhase) -> Result<f64, String> {
    if phase.gain < 0.002 {
        return Err(format!("the loop's gain {:.4} is too weak to read a phase", phase.gain));
    }
    if !phase_is_steady(phase) {
        return Err(format!("the phase is not steady across the burst ({:.1} and {:.1} degrees)", phase.halves_deg[0], phase.halves_deg[1]));
    }
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
    if phases.len() < 2 {
        return None;
    }
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
    if queued.len() < 10_000 || median <= 0.0 {
        return Err(format!("INCONCLUSIVE: no baseline from the queued part ({} samples, median {median})", queued.len()));
    }
    let covered = samples.iter().filter(|(t, _)| *t >= next + 100 * 200 && *t < next + 2_000 * 200).count();
    if covered < 1_500 {
        return Err(format!("INCONCLUSIVE: the continuation's interval was captured in only {covered} samples"));
    }
    Ok(samples.iter().filter(|(t, s)| *t >= next + 100 * 200 && *t < next + 2_000 * 200 && s[0].abs() + s[1].abs() > median / 2.0).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone at `hz` sampled at `MCR_HZ / per` samples a second, from tick 0.
    fn tone(per: i64, hz: f64, n: usize) -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        let rate = MCR_HZ / per as f64;
        (0..n).map(|i| (i as i64 * per, [(0.4 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate).cos()) as f32, (0.4 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate).sin()) as f32])).collect()
    }

    #[test]
    fn a_window_reads_its_hz_at_any_sample_rate() {
        for per in [PER_1MSPS, 100, 20] {
            let samples = tone(per, 100e3, 100);
            let (coherence, hz) = window_tone_at(&samples[..25], per);
            assert!(coherence > 0.999, "{per}: {coherence}");
            assert!((hz - 100e3).abs() < 1.0, "{per}: {hz}");
        }
    }

    #[test]
    fn the_first_millisecond_verdict_reads_only_that_window() {
        let origin = 20 * MS;
        let quiet: Vec<(i64, ezsdr_radio_uhd::Iq)> = (0..origin / PER_1MSPS).map(|i| (i * PER_1MSPS, [0.0, 0.0])).collect();
        let mut samples = quiet.clone();
        samples.extend((0..20_000).map(|i| {
            let phase = 2.0 * std::f64::consts::PI * 50e3 * i as f64 / 1e6;
            (origin + i as i64 * PER_1MSPS, [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32])
        }));
        let floor = window_power_at(&samples, origin - 10 * MS, origin - 2 * MS, PER_1MSPS).expect("a quiet floor before the segment");
        assert!(first_ms_frequency(&samples, origin, PER_1MSPS).is_some_and(|hz| (hz - 50e3).abs() < 1.0));
        assert!(matches!(first_window_verdict(&samples, origin, PER_1MSPS, 50e3, 10e3, floor), Verdict::Pass(_)));
        assert!(matches!(first_window_verdict(&samples, origin, PER_1MSPS, 100e3, 10e3, floor), Verdict::Fail(_)), "a wrong tone");
        assert!(matches!(first_window_verdict(&samples, origin, PER_1MSPS, 50e3, 10e3, 1.0), Verdict::Fail(_)), "below the noise");
        // The requested beginning missing: the capture starts 200 µs into the millisecond.
        let missing_prefix: Vec<(i64, ezsdr_radio_uhd::Iq)> = samples.iter().copied().filter(|(tick, _)| *tick >= origin + 200 * PER_1MSPS).collect();
        assert!(matches!(first_window_verdict(&missing_prefix, origin, PER_1MSPS, 50e3, 10e3, floor), Verdict::Inconclusive(_)), "a missing prefix");
        // The requested end missing: only 900 µs of the millisecond were captured.
        let missing_suffix: Vec<(i64, ezsdr_radio_uhd::Iq)> = samples.iter().copied().filter(|(tick, _)| *tick < origin + 900 * PER_1MSPS).collect();
        assert!(matches!(first_window_verdict(&missing_suffix, origin, PER_1MSPS, 50e3, 10e3, floor), Verdict::Inconclusive(_)), "a missing suffix");
        // The initial tone wrong for 200 µs while the rest is right: the majority's median is not enough.
        let wrong_start: Vec<(i64, ezsdr_radio_uhd::Iq)> = samples
            .iter()
            .copied()
            .map(|(tick, sample)| {
                if (origin..origin + 200 * PER_1MSPS).contains(&tick) {
                    let i = (tick - origin) / PER_1MSPS;
                    let phase = 2.0 * std::f64::consts::PI * 100e3 * i as f64 / 1e6;
                    (tick, [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32])
                } else {
                    (tick, sample)
                }
            })
            .collect();
        assert!(matches!(first_window_verdict(&wrong_start, origin, PER_1MSPS, 50e3, 10e3, floor), Verdict::Fail(_)), "a wrong initial tone");
    }
}
