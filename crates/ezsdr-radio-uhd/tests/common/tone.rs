//! The probes' tone classifier (design note 42): where a received 100 kHz tone first leaves its
//! frequency, judged by coherence against the noise before the burst, so that a tone the
//! loopback attenuates to about 0.008 is seen and noise alone is not taken for one.

#![allow(dead_code)]

/// Ticks of the master clock in a millisecond (200 MHz).
const MS: i64 = 200_000;

/// What a capture says about a tone that may shift at `e` (design note 42's probes): the windows
/// of 25 samples (a tone's coherence, not its magnitude: the loopback attenuates a 0.4 tone to
/// about 0.008), the coherence of the noise before the burst, how many windows carry the tone on
/// each side of `e`, and where the tone first leaves `base_hz`.
#[derive(Debug, PartialEq)]
pub struct ToneReport {
    /// Mean coherence of the windows before the burst: what noise looks like.
    pub floor: Option<f64>,
    /// The coherence a window must exceed to carry the tone.
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
/// it above `max(0.5, 2 × floor)`, and the first run of three off `base_hz`.
pub fn classify_tone(samples: &[(i64, ezsdr_radio_uhd::Iq)], burst_start: i64, burst_end: i64, e: i64, base_hz: f64) -> ToneReport {
    const SPAN: i64 = 25 * 200;
    // A window is 25 samples one tick spacing apart: one that straddles a hole in the capture
    // (the packets' timestamps jump across missing samples) is dropped.
    let windows: Vec<(i64, f64, f64)> = samples.chunks_exact(25).filter(|w| w[24].0 - w[0].0 == 24 * 200).map(|w| {
        let (coherence, hz) = window_tone(w);
        (w[0].0, coherence, hz)
    }).collect();
    let quiet: Vec<f64> = windows.iter().filter(|(t, ..)| t + SPAN <= burst_start - MS / 2).map(|(_, c, _)| *c).collect();
    let floor = (quiet.len() >= 10).then(|| quiet.iter().sum::<f64>() / quiet.len() as f64);
    let threshold = floor.map_or(0.5, |floor| (2.0 * floor).max(0.5));
    let inside: Vec<&(i64, f64, f64)> = windows.iter().filter(|(t, ..)| *t >= burst_start + MS && t + SPAN <= burst_end).collect();
    let signal: Vec<&(i64, f64, f64)> = inside.iter().copied().filter(|(_, c, _)| *c > threshold).collect();
    let first_shifted = signal.windows(3).position(|w| w.iter().all(|(_, _, hz)| (hz - base_hz).abs() > 25e3) && w[2].0 - w[0].0 == 2 * SPAN);
    // The move is dated only when the window before the first shifted one is a base window that
    // ends where the shifted one begins; a hole or a run of windows that carry no tone is a gap.
    let gapped = first_shifted.is_some_and(|i| i == 0 || signal[i].0 - (signal[i - 1].0 + SPAN) > 0);
    ToneReport {
        floor,
        gapped,
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
        format!("INCONCLUSIVE: the tone moved, but samples are missing between the last window at the base frequency and the first shifted one (noise coherence {:?}, threshold {:.2})", report.floor, report.threshold)
    } else if report.usable() {
        format!("the tone leaves 100 kHz at {:?} µs from e (signal windows {} of {} before e, {} of {} after; noise coherence {:?}, threshold {:.2})", report.moves_at.map(|t| t as f64 / 200.0), report.before, report.before_of, report.after, report.after_of, report.floor, report.threshold)
    } else {
        format!("INCONCLUSIVE: no usable signal windows (before e {} of {}, after e {} of {}, noise coherence {:?}, threshold {:.2})", report.before, report.before_of, report.after, report.after_of, report.floor, report.threshold)
    }
}
