use std::collections::BTreeMap;

use ezsdr_radio_uhd::Iq;
use serde_json::Value as Json;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeardInterval {
    pub first_tick: i64,
    pub last_tick: i64,
    pub samples: usize,
}

/// Contiguous samples above an amplitude threshold, shifted back by a known loopback delay.
pub fn heard_intervals(samples: &[(i64, Iq)], ticks_per_sample: i64, amplitude_threshold: f64, delay_samples: i64) -> Vec<HeardInterval> {
    let mut intervals = Vec::new();
    let mut current: Option<HeardInterval> = None;
    let delay = delay_samples.saturating_mul(ticks_per_sample);
    for (tick, sample) in samples {
        let heard = f64::from(sample[0]).hypot(f64::from(sample[1])) >= amplitude_threshold;
        let contiguous = current.is_some_and(|interval| interval.last_tick.checked_add(ticks_per_sample) == Some(*tick));
        if !heard || (current.is_some() && !contiguous) {
            if let Some(mut interval) = current.take() {
                interval.first_tick = interval.first_tick.saturating_sub(delay);
                interval.last_tick = interval.last_tick.saturating_sub(delay);
                intervals.push(interval);
            }
        }
        if heard {
            match current.as_mut() {
                Some(interval) => {
                    interval.last_tick = *tick;
                    interval.samples += 1;
                }
                None => current = Some(HeardInterval { first_tick: *tick, last_tick: *tick, samples: 1 }),
            }
        }
    }
    if let Some(mut interval) = current {
        interval.first_tick = interval.first_tick.saturating_sub(delay);
        interval.last_tick = interval.last_tick.saturating_sub(delay);
        intervals.push(interval);
    }
    intervals
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrelationStatus {
    Found,
    NoSignal,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorrelationOffset {
    pub status: CorrelationStatus,
    pub matched_tick: Option<i64>,
    pub sample_offset: Option<i64>,
    pub peak: f64,
    pub second_peak: f64,
    pub median: f64,
}

/// Locate a reference waveform near its expected receive tick, rejecting weak or tied peaks.
pub fn correlation_offset(samples: &[(i64, Iq)], reference: &[Iq], expected_tick: i64, ticks_per_sample: i64, search_radius_samples: i64) -> CorrelationOffset {
    let empty = CorrelationOffset { status: CorrelationStatus::NoSignal, matched_tick: None, sample_offset: None, peak: 0.0, second_peak: 0.0, median: 0.0 };
    if reference.is_empty() || ticks_per_sample <= 0 || search_radius_samples < 0 || samples.len() < reference.len() {
        return empty;
    }
    let radius = i128::from(search_radius_samples) * i128::from(ticks_per_sample);
    let lower = i128::from(expected_tick) - radius;
    let upper = i128::from(expected_tick) + radius;
    let mut magnitudes = Vec::new();
    let mut best: Option<(i64, f64)> = None;
    let mut second = 0.0f64;
    let mut run_start = 0usize;
    while run_start < samples.len() {
        let mut run_end = run_start + 1;
        while run_end < samples.len() && samples[run_end - 1].0.checked_add(ticks_per_sample) == Some(samples[run_end].0) {
            run_end += 1;
        }
        if run_end - run_start >= reference.len() {
            for start in run_start..=run_end - reference.len() {
                let tick = samples[start].0;
                if i128::from(tick) < lower || i128::from(tick) > upper {
                    continue;
                }
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (received, reference) in samples[start..start + reference.len()].iter().zip(reference) {
                    re += f64::from(received.1[0]) * f64::from(reference[0]) + f64::from(received.1[1]) * f64::from(reference[1]);
                    im += f64::from(received.1[1]) * f64::from(reference[0]) - f64::from(received.1[0]) * f64::from(reference[1]);
                }
                let magnitude = re.hypot(im);
                magnitudes.push(magnitude);
                match best {
                    None => best = Some((tick, magnitude)),
                    Some((_, peak)) if magnitude > peak => {
                        second = peak;
                        best = Some((tick, magnitude));
                    }
                    Some(_) if magnitude > second => second = magnitude,
                    Some(_) => {}
                }
            }
        }
        run_start = run_end;
    }
    let Some((tick, peak)) = best else { return empty };
    magnitudes.sort_by(f64::total_cmp);
    let median = magnitudes[magnitudes.len() / 2];
    let offset = ((i128::from(tick) - i128::from(expected_tick)) / i128::from(ticks_per_sample)) as i64;
    let status = if peak <= median * 8.0 || peak <= 0.0 {
        CorrelationStatus::NoSignal
    } else if second >= peak * 0.8 {
        CorrelationStatus::Ambiguous
    } else {
        CorrelationStatus::Found
    };
    CorrelationOffset {
        status,
        matched_tick: (status == CorrelationStatus::Found).then_some(tick),
        sample_offset: (status == CorrelationStatus::Found).then_some(offset),
        peak,
        second_peak: second,
        median,
    }
}

/// Counts every report code; malformed rows are retained under `<missing>`.
pub fn count_report_codes(reports: &[Json]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for report in reports {
        let code = report.get("code").and_then(Json::as_str).unwrap_or("<missing>");
        *counts.entry(code.to_owned()).or_default() += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pn(n: usize) -> Vec<Iq> {
        let mut state = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                [if state >> 30 & 1 == 1 { 0.25 } else { -0.25 }, if state >> 29 & 1 == 1 { 0.25 } else { -0.25 }]
            })
            .collect()
    }

    fn received(reference: &[Iq], at: usize, len: usize) -> Vec<(i64, Iq)> {
        let mut samples = vec![(0.0, [0.0, 0.0]); len];
        for (index, sample) in reference.iter().enumerate() {
            if at + index < len {
                samples[at + index].1 = *sample;
            }
        }
        samples.into_iter().enumerate().map(|(index, (_, sample))| (index as i64 * 200, sample)).collect()
    }

    #[test]
    fn correlation_offset_finds_zero_positive_negative_and_noisy_offsets() {
        let reference = pn(64);
        for offset in [0, 5, -7] {
            let expected = 20i64 * 200;
            let received_at = (20 + offset) as usize;
            let mut input = received(&reference, received_at, 120);
            for (index, (_, sample)) in input.iter_mut().enumerate() {
                sample[0] += (index % 3) as f32 * 0.0001;
                sample[1] -= (index % 5) as f32 * 0.0001;
            }
            let found = correlation_offset(&input, &reference, expected, 200, 10);
            assert_eq!(found.status, CorrelationStatus::Found, "{found:?}");
            assert_eq!(found.sample_offset, Some(offset));
        }
        let silence = vec![(0, [0.0, 0.0]); 100];
        assert_eq!(correlation_offset(&silence, &reference, 10_000, 200, 20).status, CorrelationStatus::NoSignal);
    }

    #[test]
    fn correlation_offset_marks_equal_peaks_ambiguous() {
        let reference = pn(64);
        let mut input = received(&reference, 20, 160);
        for (index, sample) in reference.iter().enumerate() {
            input[80 + index].1 = *sample;
        }
        let found = correlation_offset(&input, &reference, 60 * 200, 200, 50);
        assert_eq!(found.status, CorrelationStatus::Ambiguous, "{found:?}");
        assert_eq!(found.sample_offset, None);
    }

    #[test]
    fn heard_intervals_cover_silence_midstream_and_a_truncated_tail() {
        let samples: Vec<_> = (0..8).map(|index| (index * 200, if (2..=3).contains(&index) || index >= 6 { [0.1, 0.0] } else { [0.0, 0.0] })).collect();
        assert!(heard_intervals(&samples[..2], 200, 0.01, 0).is_empty());
        assert_eq!(heard_intervals(&samples, 200, 0.01, 1), vec![HeardInterval { first_tick: 200, last_tick: 400, samples: 2 }, HeardInterval { first_tick: 1_000, last_tick: 1_200, samples: 2 },]);
    }

    #[test]
    fn report_counts_include_all_codes_and_malformed_rows() {
        let reports = vec![json!({"code":"Underflow"}), json!({"code":"BurstAck"}), json!({"code":"Underflow"}), json!({"tick":12})];
        assert_eq!(count_report_codes(&reports), BTreeMap::from([("<missing>".to_owned(), 1), ("BurstAck".to_owned(), 1), ("Underflow".to_owned(), 2),]));
    }
}
