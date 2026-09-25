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
        self.changes
            .iter()
            .rev()
            .find(|(tick, _)| t.is_at_or_after(*tick))
            .map_or(self.initial, |(_, value)| *value)
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
    /// `(n · channels + c) · 2` is sample `n`'s real part on channel `c`, `+ 1` its
    /// imaginary part, each already clipped to the contract's full scale.
    pub samples: Arc<[f32]>,
    /// The first sample not radiated, once a stop or a cold change cut the burst.
    pub end: Option<i64>,
}

/// The transmit side's plan: every admitted burst keyed by `(clock generation, start)`,
/// and the parameters that shape what it radiates (MR-32, MR-33, MR-34).
pub(crate) struct TxPlan {
    pub segments: BTreeMap<(u32, i64), Segment>,
    pub gain_db: Timeline,
    pub frequency_hz: Timeline,
    pub phase: Vec<Timeline>,
    /// The phase each channel's LO takes after every timed tune (MR-34).
    pub timed_phase: Vec<f64>,
    pub path_delay: i64,
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
            let j = at.sample_at_or_before(segment.origin, segment.ratio).saturating_sub(plan.path_delay);
            if j < segment.start {
                continue;
            }
            let natural_end = if segment.repeat { i64::MAX } else { segment.start.saturating_add(segment.len) };
            if j >= segment.end.unwrap_or(i64::MAX).min(natural_end) || channel >= segment.channels {
                return None;
            }
            let n = usize::try_from((j - segment.start) % segment.len).ok()?;
            let index = (n * usize::from(segment.channels) + usize::from(channel)) * 2;
            let (re, im) = (f64::from(segment.samples[index]), f64::from(segment.samples[index + 1]));
            let stamp = RootInstant::of_sample(segment.origin, segment.ratio, j);
            let amplitude = 10f64.powf(plan.gain_db.at(stamp) / 20.0);
            let (sin, cos) = plan.phase.get(usize::from(channel)).map_or(0.0, |phase| phase.at(stamp)).sin_cos();
            return Some(Radiated {
                re: amplitude * (re * cos - im * sin),
                im: amplitude * (re * sin + im * cos),
                frequency_hz: plan.frequency_hz.at(stamp),
            });
        }
        None
    }
}

/// The receive side's parameters (MR-33, MR-34, MR-35). Its antenna instant is the
/// sample's own: both profiles declare `radio.rx.path_delay_samples` 0 (MR-3).
pub(crate) struct RxModel {
    pub gain_db: Timeline,
    pub frequency_hz: Timeline,
    pub phase: Vec<Timeline>,
    /// The phase each channel's LO takes after every timed tune (MR-34).
    pub timed_phase: Vec<f64>,
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
