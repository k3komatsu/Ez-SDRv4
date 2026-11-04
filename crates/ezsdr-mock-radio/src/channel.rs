//! MockRadio's radio-side signal model on the SimulationChannel: what its transmit side
//! radiates and how its receive side turns the field into samples (MR-31…MR-36).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use ezsdr_kernel::time::Rational;
use ezsdr_sim::channel::{Radiated, RootInstant, Transmitter};

/// A parameter's value over time: `initial` until its first change, and each change from
/// its effective root tick on; changes are appended in application order (MR-33, UC-2).
#[derive(Clone, Debug)]
pub(crate) struct Timeline {
    initial: f64,
    changes: Vec<(i64, f64)>,
}

impl Timeline {
    pub(crate) fn new(initial: f64) -> Timeline {
        Timeline { initial, changes: Vec::new() }
    }

    pub(crate) fn set(&mut self, tick: i64, value: f64) {
        self.changes.push((tick, value));
    }

    /// The value in force at `t`: the last change whose tick is at or before `t`.
    pub(crate) fn at(&self, t: RootInstant) -> f64 {
        self.changes.iter().rev().find(|(tick, _)| t.is_at_or_after(*tick)).map_or(self.initial, |(_, value)| *value)
    }
}

/// One admitted burst as the medium sees it (MR-32).
pub(crate) struct Segment {
    pub origin: i64,
    pub ratio: Rational,
    pub start: i64,
    pub len: i64,
    pub repeat: bool,
    pub channels: u16,
    pub channel_numbers: Vec<u16>,
    /// `(n · channels + c) · 2` is sample `n`'s real part on channel `c`, `+ 1` its
    /// imaginary part, each already clipped to the contract's full scale.
    pub samples: Arc<[f32]>,
    /// The first sample not radiated, once a stop cut the burst.
    pub end: Option<i64>,
}

/// The transmit side's plan: every admitted burst and its logical channels' radio model
/// (MR-32, MR-33, MR-34).
pub(crate) struct TxPlan {
    pub segments: BTreeMap<(i64, Option<u64>, usize, u32, i64), Segment>,
    pub channel_models: Vec<ChannelModel>,
}

/// One layout channel's parameters and phase history (RM-29, MR-33, MR-34).
pub(crate) struct ChannelModel {
    pub gain_db: Timeline,
    pub frequency_hz: Timeline,
    pub phase: Timeline,
    pub phase_behavior: String,
    pub path_delay_samples: i64,
}

/// The shared handle the medium reads the transmit side through (MR-32, CH-8).
pub(crate) struct TxShared(Mutex<TxPlan>);

impl TxShared {
    pub(crate) fn new(plan: TxPlan) -> Arc<TxShared> {
        Arc::new(TxShared(Mutex::new(plan)))
    }

    pub(crate) fn plan(&self) -> MutexGuard<'_, TxPlan> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Transmitter for TxShared {
    fn radiated(&self, channel: u16, at: RootInstant) -> Option<Radiated> {
        let plan = self.plan();
        for segment in plan.segments.values().rev() {
            let Some(position) = segment.channel_numbers.iter().position(|number| *number == channel) else {
                continue;
            };
            let Some(model) = plan.channel_models.get(usize::from(channel)) else {
                continue;
            };
            let j = at.sample_at_or_before(segment.origin, segment.ratio).saturating_sub(model.path_delay_samples);
            if j < segment.start {
                continue;
            }
            let natural_end = if segment.repeat { i64::MAX } else { segment.start.saturating_add(segment.len) };
            if j >= segment.end.unwrap_or(i64::MAX).min(natural_end) {
                return None;
            }
            let n = usize::try_from((j - segment.start) % segment.len).ok()?;
            let index = (n * usize::from(segment.channels) + position) * 2;
            let (re, im) = (f64::from(segment.samples[index]), f64::from(segment.samples[index + 1]));
            let stamp = RootInstant::of_sample(segment.origin, segment.ratio, j);
            let gain = model.gain_db.at(stamp);
            let amplitude = 10f64.powf(gain / 20.0);
            let phase = model.phase.at(stamp);
            let (sin, cos) = phase.sin_cos();
            let frequency = model.frequency_hz.at(stamp);
            return Some(Radiated { re: amplitude * (re * cos - im * sin), im: amplitude * (re * sin + im * cos), frequency_hz: frequency });
        }
        None
    }
}

/// One layout receive channel's parameters and phase history (RM-29, MR-33…MR-35).
pub(crate) struct RxChannelModel {
    pub gain_db: Timeline,
    pub frequency_hz: Timeline,
    pub phase: Timeline,
    pub phase_behavior: String,
    pub path_delay_samples: i64,
}

/// Decodes a `cf32` waveform, refusing a non-finite component and clipping each
/// component to `[−1, 1]`; returns the samples and how many sample-channel pairs had a
/// component clipped (RM-13, MR-32, MR-36).
pub(crate) fn decode_waveform(bytes: &[u8]) -> Result<(Arc<[f32]>, u64), &'static str> {
    let mut samples = Vec::with_capacity(bytes.len() / 4);
    let mut clipped = 0;
    for pair in bytes.chunks_exact(8) {
        let re = f32::from_le_bytes(pair[0..4].try_into().expect("four bytes"));
        let im = f32::from_le_bytes(pair[4..8].try_into().expect("four bytes"));
        if !re.is_finite() || !im.is_finite() {
            return Err("RM-13: a waveform sample is not finite");
        }
        let (cre, cim) = (re.clamp(-1.0, 1.0), im.clamp(-1.0, 1.0));
        if cre != re || cim != im {
            clipped += 1;
        }
        samples.push(cre);
        samples.push(cim);
    }
    Ok((samples.into(), clipped))
}

/// A random LO phase in `[0, 2π)` from one draw: `2π · ⌊x / 2^11⌋ · 2^−53` (MR-34).
pub(crate) fn lo_phase(rng: &mut ezsdr_sim::SimRng) -> f64 {
    const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
    2.0 * std::f64::consts::PI * ((rng.next_u64() >> 11) as f64 * SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mr_33_a_timeline_keeps_the_value_in_force_at_each_instant() {
        // The per-channel value history a delayed sample reads (MR-33): the stage that brings Run-time
        // changes back needs it, so it stays although no Run-time change reaches it now.
        let mut timeline = Timeline::new(1.0);
        timeline.set(100, 2.0);
        timeline.set(200, 3.0);
        for (tick, expected) in [(i64::MIN, 1.0), (99, 1.0), (100, 2.0), (199, 2.0), (200, 3.0), (i64::MAX, 3.0)] {
            assert_eq!(timeline.at(RootInstant::tick(tick)), expected, "tick {tick}");
        }
    }

    #[test]
    fn mr_32_a_radiated_sample_takes_the_values_in_force_at_its_own_instant() {
        // The transmit side reads gain, frequency and phase at the instant of the sample it radiates (its sample
        // clock's, `path_delay_samples` before the antenna instant), not at the antenna instant (MR-32, MR-33).
        let (mut gain_db, mut frequency_hz, mut phase) = (Timeline::new(0.0), Timeline::new(100.0), Timeline::new(0.0));
        gain_db.set(3_000, 20.0);
        frequency_hz.set(3_000, 200.0);
        phase.set(3_000, std::f64::consts::FRAC_PI_2);
        let ratio = Rational::new(1_000, 1).unwrap();
        let segment = Segment { origin: 0, ratio, start: 0, len: 4, repeat: false, channels: 1, channel_numbers: vec![0], samples: [1.0f32, 0.0].repeat(4).into(), end: None };
        let model = ChannelModel { gain_db, frequency_hz, phase, phase_behavior: "deterministic".to_owned(), path_delay_samples: 2 };
        let shared = TxShared::new(TxPlan { segments: BTreeMap::from([((0, None, 0, 0, 0), segment)]), channel_models: vec![model] });
        // Antenna instant 4 900 (sample 4): the radiated sample is 2, at tick 2 000, before the change at 3 000.
        let before = shared.radiated(0, RootInstant::tick(4_900)).unwrap();
        assert_eq!((before.re, before.im, before.frequency_hz), (1.0, 0.0, 100.0), "the sample's own instant precedes the change");
        // Antenna instant 5 100 (sample 5): the radiated sample is 3, at tick 3 000, which the change takes at or after.
        let after = shared.radiated(0, RootInstant::tick(5_100)).unwrap();
        assert!(after.re.abs() < 1e-9 && (after.im - 10.0).abs() < 1e-9 && after.frequency_hz == 200.0, "{after:?}");
    }
}
