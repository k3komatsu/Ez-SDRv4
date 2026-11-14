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

/// The outcome of a claim about the tone: PASS and INCONCLUSIVE pass the test; FAIL is evidence against the claim. A verdict that is computed and
/// not asserted would be silent, so it must be used.
#[derive(Debug)]
#[must_use]
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

/// One receive sample as the monitor keeps it: its tick and value, or `None` where the receive lost time (an overflow, a timeout,
/// a failed call). A gap is never filled.
pub type Slot = Option<(i64, ezsdr_radio_uhd::Iq)>;

/// The ssh drop's tone at the receiver: 100 kHz at 1 MS/s (ssh-drop-driver.py's wave). A window further off is another signal.
const DROP_TONE_HZ: f64 = 100e3;

/// The mean power of one window.
pub fn mean_power(w: &[(i64, ezsdr_radio_uhd::Iq)]) -> f64 {
    w.iter().map(|(_, s)| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2)).sum::<f64>() / w.len() as f64
}

/// A window's coherence the monitor takes for a tone. 0.9, not `tone.rs`'s 0.5: white noise is over 0.5 in about 1.7 % of its
/// 25-sample windows (simulated, 2026-11-08), which is over the 1 % bar; a 100 kHz tone at the loopback's level is over 0.9999.
const MONITOR_COHERENCE: f64 = 0.9;

/// Whether a window has the drop's shape: coherent over `MONITOR_COHERENCE` and at 100 kHz within 20 kHz, as the monitor reads a tone. Sub-LSB
/// quantized noise has the shape in under 1 % of its windows (simulated, 2026-11-08), and its coherent windows pile up at 125 kHz, where a lag
/// product lands on a multiple of 45° (the lattice); the bound stays under that point. C3 (`hw_noise_floor_receive`) and the monitor both use this.
pub fn is_drop_tone_window(w: &[(i64, ezsdr_radio_uhd::Iq)]) -> bool {
    let (coherence, hz) = window_tone(w);
    coherence > MONITOR_COHERENCE && (hz - DROP_TONE_HZ).abs() <= 20e3
}

/// C3's amplitude floor, 4 LSB as a mean power (sc16 counts are 1/32768 of full scale). The loopback tone is about −49 dBFS, roughly 30 dB
/// above it (INFERRED from bench-results.md:27-28); noise at 1 LSB has about 2 LSB² of window power.
pub const QUIET_CHECK_MIN_POWER: f64 = (4.0 / 32768.0) * (4.0 / 32768.0);

/// What C3 (`hw_noise_floor_receive`) reads of a quiet receive: its 25-sample windows, how many carry the drop's tone above the amplitude floor,
/// and the largest amplitude in LSB of a window with the drop's shape (0 when there is none).
#[derive(Debug, PartialEq)]
pub struct QuietCheck {
    pub windows: usize,
    pub tone: usize,
    pub largest_lsb: f64,
}

/// C3's decision over `samples`: a window counts as tone only when it has the drop's shape and its mean power exceeds `QUIET_CHECK_MIN_POWER`,
/// so sub-LSB quantized noise cannot reach the 1 % bar through its lattice windows.
pub fn quiet_check(samples: &[(i64, ezsdr_radio_uhd::Iq)]) -> QuietCheck {
    let windows: Vec<&[(i64, ezsdr_radio_uhd::Iq)]> = samples.chunks_exact(25).collect();
    QuietCheck {
        windows: windows.len(),
        tone: windows.iter().filter(|w| is_drop_tone_window(w) && mean_power(w) > QUIET_CHECK_MIN_POWER).count(),
        largest_lsb: windows.iter().filter(|w| is_drop_tone_window(w)).map(|w| mean_power(w).sqrt() * 32768.0).fold(0.0, f64::max),
    }
}

/// Whether a window carries the drop's tone at a power above `4 × floor`: the monitor's test for one window.
fn carries_drop_tone(w: &[(i64, ezsdr_radio_uhd::Iq)], floor: f64) -> bool {
    is_drop_tone_window(w) && mean_power(w) > 4.0 * floor
}

/// The noise floor of a receive stretch from its first second: the mean power of its windows without the drop's shape. Refused (with
/// the reason) when the second has a gap, or when the drop's tone fills half its windows: a sustained tone, not the sparse coherent
/// windows of quantized noise. The 1 % bar of `monitor_reading` applies to the readings after the start.
pub fn monitor_floor(second: &[Slot]) -> Result<f64, String> {
    if second.iter().any(Option::is_none) {
        return Err("the first second has a gap: the receive did not start clean".to_owned());
    }
    let samples: Vec<(i64, ezsdr_radio_uhd::Iq)> = second.iter().flatten().copied().collect();
    let windows: Vec<&[(i64, ezsdr_radio_uhd::Iq)]> = contiguous_windows(&samples).collect();
    if windows.is_empty() {
        return Err("the first second holds no 25-sample window".to_owned());
    }
    let tone = windows.iter().filter(|w| is_drop_tone_window(w)).count();
    if tone * 2 >= windows.len() {
        return Err(format!("the drop's tone fills {tone} of the first second's {} windows: the transmitter must be silent for the first second", windows.len()));
    }
    let quiet: Vec<f64> = windows.iter().filter(|w| !is_drop_tone_window(w)).map(|w| mean_power(w)).collect();
    Ok(quiet.iter().sum::<f64>() / quiet.len() as f64)
}

/// The newest a reading may be: a block whose last sample is more than 300 ms behind the device's clock when its line is written
/// describes the air too late (the receive fell behind).
pub const MONITOR_MAX_LAG_TICKS: i64 = 300 * MS;

/// A block's reading as its line gives it: `reading` when its last sample (`last`) is within `MONITOR_MAX_LAG_TICKS` of the device's
/// clock `now`, `unknown` otherwise.
pub fn dated_reading(reading: &'static str, last: Option<i64>, now: i64) -> &'static str {
    match last {
        Some(tick) if now - tick <= MONITOR_MAX_LAG_TICKS => reading,
        _ => "unknown",
    }
}

/// The monitor's reading of one stretch of receive, given the noise floor: `"tone"` when at least 1 % of its windows carry the
/// drop's tone (`carries_drop_tone`), `"quiet"` otherwise, and `"unknown"` when a sample is missing, the ticks break, or no
/// window is left.
pub fn monitor_reading(block: &[Slot], floor: f64) -> &'static str {
    if block.iter().any(Option::is_none) {
        return "unknown";
    }
    let samples: Vec<(i64, ezsdr_radio_uhd::Iq)> = block.iter().flatten().copied().collect();
    if samples.windows(2).any(|p| p[1].0 - p[0].0 != PER_1MSPS) {
        return "unknown";
    }
    let windows: Vec<&[(i64, ezsdr_radio_uhd::Iq)]> = contiguous_windows(&samples).collect();
    if windows.is_empty() {
        return "unknown";
    }
    let tone = windows.iter().filter(|w| carries_drop_tone(w, floor)).count();
    if tone * 100 >= windows.len() { "tone" } else { "quiet" }
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

    /// `n` samples from tick 0 at 1 MS/s, sample `i` = `signal(i)`, in the form the monitor keeps.
    fn stretch(n: usize, signal: impl Fn(usize) -> [f32; 2]) -> Vec<Slot> {
        (0..n).map(|i| Some((i as i64 * PER_1MSPS, signal(i)))).collect()
    }

    /// A tone at `hz`, amplitude 0.4, at 1 MS/s.
    fn tone_at(hz: f64) -> impl Fn(usize) -> [f32; 2] {
        move |i| {
            let phase = 2.0 * std::f64::consts::PI * hz * i as f64 / 1e6;
            [(0.4 * phase.cos()) as f32, (0.4 * phase.sin()) as f32]
        }
    }

    /// White noise of amplitude about 1e-4 from a hash of the sample index: no coherent window.
    fn hiss(i: usize) -> [f32; 2] {
        let uniform = |k: u64| {
            let mut z = (i as u64 * 2 + k).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        [uniform(0) * 2e-4, uniform(1) * 2e-4]
    }

    #[test]
    fn the_monitor_reads_the_drop_tone_and_quiet_and_says_unknown_for_a_gap() {
        let floor = monitor_floor(&stretch(50_000, hiss)).expect("a quiet second");
        let block = 200_000;
        assert_eq!(monitor_reading(&stretch(block, tone_at(100e3)), floor), "tone");
        assert_eq!(monitor_reading(&stretch(block, hiss), floor), "quiet");
        let mut gap = stretch(block, tone_at(100e3));
        gap[1000] = None;
        assert_eq!(monitor_reading(&gap, floor), "unknown", "a gap inside the block");
        let mut first = stretch(block, tone_at(100e3));
        first[0] = None;
        assert_eq!(monitor_reading(&first, floor), "unknown", "a gap at the block's first sample");
        let mut skipped = stretch(block, tone_at(100e3));
        skipped.remove(1000);
        assert_eq!(monitor_reading(&skipped, floor), "unknown", "a sample taken away: the ticks skip one");
    }

    #[test]
    fn a_tone_in_one_window_of_a_hundred_is_the_bar_and_noise_is_quiet() {
        let floor = 1e-8;
        let tone = tone_at(100e3);
        assert_eq!(monitor_reading(&stretch(2_500, move |i| if i < 25 { tone(i) } else { hiss(i) }), floor), "tone", "1 % of the windows");
        assert_eq!(monitor_reading(&stretch(2_500, hiss), floor), "quiet");
    }

    #[test]
    fn loud_noise_with_no_coherent_window_reads_quiet() {
        let floor = monitor_floor(&stretch(50_000, hiss)).expect("a quiet second");
        assert_eq!(monitor_reading(&stretch(20_000, |i| hiss(i).map(|x| 100.0 * x)), floor), "quiet");
    }

    #[test]
    fn a_tone_under_four_times_the_floor_reads_quiet() {
        assert_eq!(monitor_reading(&stretch(2_500, tone_at(100e3)), 1.0), "quiet");
    }

    #[test]
    fn a_coherent_signal_off_the_drop_frequency_reads_quiet() {
        assert_eq!(monitor_reading(&stretch(2_500, tone_at(300e3)), 1e-8), "quiet");
    }

    /// The drop's tone in the first `samples` samples of a 5 000-sample stretch (5 ms; the monitor takes 1 000 000 samples, and its rule
    /// is a share of the windows, so the length only sets the resolution), hiss after it: `samples / 25` of the 200 windows.
    fn tone_for(samples: usize) -> Vec<Slot> {
        let tone = tone_at(100e3);
        stretch(5_000, move |i| if i < samples { tone(i) } else { hiss(i) })
    }

    #[test]
    fn the_start_refuses_a_sustained_tone_or_a_gap_and_takes_the_floor_of_a_quiet_second() {
        let whole = monitor_floor(&stretch(50_000, tone_at(100e3))).unwrap_err();
        assert!(whole.contains("fills"), "{whole}");
        let half = monitor_floor(&tone_for(2_500)).unwrap_err();
        assert!(half.contains("fills"), "100 of 200 windows is half: {half}");
        let mut gapped = stretch(50_000, hiss);
        gapped[7] = None;
        assert!(monitor_floor(&gapped).unwrap_err().contains("gap"));
        let floor = monitor_floor(&stretch(50_000, hiss)).expect("a quiet second");
        assert!(floor > 5e-9 && floor < 9e-9, "the mean power of the hiss: {floor}");
    }

    #[test]
    fn sparse_tone_windows_do_not_refuse_the_start() {
        assert!(monitor_floor(&tone_for(50)).is_ok(), "2 of 200 windows");
        assert!(monitor_floor(&tone_for(100)).is_ok(), "4 of 200 windows");
        assert!(monitor_floor(&tone_for(2_400)).is_ok(), "96 of 200 windows is under half");
    }

    /// Standard normal draws for sample `i`: Box–Muller over a hash of the index, so the tests are deterministic.
    fn gaussian(i: usize) -> (f64, f64) {
        let uniform = |k: u64| {
            let mut z = (i as u64 * 2 + k).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (((z ^ (z >> 31)) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let (radius, angle) = ((-2.0 * uniform(0).ln()).sqrt(), 2.0 * std::f64::consts::PI * uniform(1));
        (radius * angle.cos(), radius * angle.sin())
    }

    /// Ordinary noise of about 3 LSB of standard deviation (1e-4 of full scale).
    fn ordinary(i: usize) -> [f32; 2] {
        let (re, im) = gaussian(i);
        [(re * 1e-4) as f32, (im * 1e-4) as f32]
    }

    /// Gaussian noise of `sigma_lsb` standard deviation, rounded to whole sc16 counts (1 LSB = 2^-15): at 0.5 most samples are 0 or ±1.
    fn quantized(i: usize, sigma_lsb: f64) -> [f32; 2] {
        let (re, im) = gaussian(i);
        [((re * sigma_lsb).round() / 32768.0) as f32, ((im * sigma_lsb).round() / 32768.0) as f32]
    }

    #[test]
    fn ordinary_and_quantized_noise_start_the_monitor_and_read_quiet() {
        assert!(monitor_floor(&stretch(50_000, ordinary)).is_ok(), "ordinary noise starts the monitor");
        let quantized_half = |i: usize| quantized(i, 0.5);
        let floor = monitor_floor(&stretch(50_000, quantized_half)).expect("noise at 0.5 LSB starts the monitor");
        assert_eq!(monitor_reading(&stretch(200_000, quantized_half), floor), "quiet", "noise at 0.5 LSB reads quiet");
        assert_eq!(monitor_reading(&stretch(200_000, ordinary), monitor_floor(&stretch(50_000, ordinary)).unwrap()), "quiet", "ordinary noise reads quiet");
    }

    #[test]
    fn c3_quiet_noise_stays_under_its_bar_at_its_sample_size() {
        // hw_noise_floor_receive's capture: 20 000 samples, 800 windows, fewer than 8 (1 %) counted as tone. Before the amplitude floor and the
        // 20 kHz bound, seed 0 at 0.3 LSB and seed 4 at 0.4 LSB reach 8 windows (of 200 seeds, 54 and 59 reach it: simulated 2026-11-08).
        let counted = |signal: &dyn Fn(usize) -> [f32; 2]| quiet_check(&stretch(20_000, signal).into_iter().flatten().collect::<Vec<_>>()).tone;
        for (sigma, seed) in [(0.3, 0), (0.4, 4), (0.5, 0)] {
            let noise = counted(&|i: usize| quantized(i + seed * 1_000_000, sigma));
            assert!(noise * 100 < 800, "noise at {sigma} LSB, seed {seed}: {noise} of 800 windows count as tone");
        }
        assert_eq!(counted(&tone_at(100e3)), 800, "a tone has the drop's shape in every window");
        let loopback = |i: usize| {
            let phase = 2.0 * std::f64::consts::PI * 100e3 * i as f64 / 1e6;
            [(120.0 / 32768.0 * phase.cos()) as f32, (120.0 / 32768.0 * phase.sin()) as f32]
        };
        assert_eq!(counted(&loopback), 800, "a tone at the loopback's 120 LSB counts in every window");
    }

    #[test]
    fn a_window_on_the_45_degree_lattice_is_not_the_drop_tone() {
        // Each lag product is at 45°, so the window reads coherence 1 at exactly 125 kHz, outside the 20 kHz bound.
        const CYCLE: [[f32; 2]; 8] = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0], [1.0, -1.0]];
        let window: Vec<(i64, ezsdr_radio_uhd::Iq)> = (0..25).map(|i| (i as i64 * PER_1MSPS, CYCLE[i % 8])).collect();
        let (coherence, hz) = window_tone(&window);
        assert!(coherence > 0.99 && (hz - 125e3).abs() < 1.0, "coherence {coherence}, {hz} Hz");
        assert!(!is_drop_tone_window(&window));
    }

    /// A 25-sample window at 100 kHz of `amplitude`, whose phase steps alternate between 0.2π + θ and 0.2π − θ, so its coherence is exactly cos θ.
    fn window_at_coherence(coherence: f64, amplitude: f64) -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        let (phi, theta) = (2.0 * std::f64::consts::PI * 0.1, coherence.acos());
        let mut psi = 0.0f64;
        (0..25)
            .map(|k| {
                let sample = [(amplitude * psi.cos()) as f32, (amplitude * psi.sin()) as f32];
                psi += if k % 2 == 0 { phi + theta } else { phi - theta };
                (k as i64 * PER_1MSPS, sample)
            })
            .collect()
    }

    #[test]
    fn a_coherent_window_below_0_9_is_not_the_drop_tone() {
        let (coherence, hz) = window_tone(&window_at_coherence(0.7, 1.0));
        assert!((coherence - 0.7).abs() < 1e-3 && (hz - 100e3).abs() < 1.0, "coherence {coherence}, {hz} Hz");
        assert!(!is_drop_tone_window(&window_at_coherence(0.7, 1.0)), "coherence 0.7 at 100 kHz is under the monitor's 0.9");
        assert!(is_drop_tone_window(&window_at_coherence(0.95, 1.0)), "coherence 0.95 at 100 kHz");
    }

    #[test]
    fn c3_counts_a_drop_shape_window_only_above_4_lsb() {
        let lsb = 1.0 / 32768.0;
        let below = quiet_check(&window_at_coherence(0.95, 3.0 * lsb));
        assert_eq!(below.tone, 0, "3 LSB of the drop's shape is under the floor");
        let above = quiet_check(&window_at_coherence(0.95, 5.0 * lsb));
        assert_eq!(above.tone, 1, "5 LSB counts");
        assert!((above.largest_lsb - 5.0).abs() < 1e-3, "largest {}", above.largest_lsb);
    }

    #[test]
    fn a_reading_is_dated_by_its_last_sample_against_the_device_clock() {
        let now = 1_000 * MS;
        assert_eq!(dated_reading("tone", Some(now - 10 * MS), now), "tone", "on time");
        assert_eq!(dated_reading("tone", Some(now - 300 * MS), now), "tone", "300 ms is the bound");
        assert_eq!(dated_reading("tone", Some(now - 400 * MS), now), "unknown", "a block read 400 ms late is not fresh");
        assert_eq!(dated_reading("quiet", None, now), "unknown", "a block with no sample");
    }
}
