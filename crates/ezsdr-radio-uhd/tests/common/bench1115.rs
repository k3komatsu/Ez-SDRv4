//! The bench probes of 2026-11-15 (plan/phase8/bench.md): the receive-restart zeros sweep (item 11, direction D), the #75 probes of
//! the recovery flush (T3), and the open-flush sweep. Each probe drives raw `Device` calls (no Session) and prints `RXZ`, `P75` or
//! `OFS` lines; the cell lists, the summaries, the verdicts and the decision rule are pure functions here, so a wrong comparison fails
//! a unit test without a USRP and the probes run once on the FakeDevice (`tests/fake.rs`).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::{Duration as Wall, Instant};

use ezsdr_radio::x3xx::Board;
use ezsdr_radio_uhd::{Device, Dir, Iq, Settings, TxCode, TxReport};
use serde_json::{Value as Json, json};

use super::iii::{self, RestartTrial};
use super::n47;
use super::observations;
use super::raw::{Read, TICKS_PER_MS, align_to, d9_channel, drain, iq, pin_slot_a, raw_rx, read, read_with, send_burst, tone_wave};
use super::tone::{Verdict, first_ms_frequency};
use super::{MCR, SAFE_TX_HZ, SAFE_TX_MAX_RATE};

/// min, median and max of `values`, `None` when empty.
fn spread<T: Copy + Ord>(mut values: Vec<T>) -> Option<(T, T, T)> {
    values.sort();
    Some((*values.first()?, values[values.len() / 2], *values.last()?))
}

fn wait_device_time(device: &dyn Device, tick: i64) {
    while device.time_now().is_ok_and(|now| now < tick) {
        std::thread::sleep(Wall::from_millis(1));
    }
}

/// The probes transmit on slot A's channel 0 only (the two-channel cells name their channels themselves, and slot B's attenuator is confirmed apart): a
/// probe aborts before any device call when `channel` (`EZSDR_D9_CHANNEL`) points elsewhere, because only slot A's loopback is checked before a transmit.
pub fn assert_slot_a_only(channel: usize) {
    assert!(channel == 0, "bench.md RF safety: EZSDR_D9_CHANNEL={channel}: the 2026-11-15 probes transmit on channel 0 (slot A) only; unset it");
}

/// The receive outcomes of a read other than a timeout and samples (an overflow, a late command...).
fn rx_outcomes(got: &Read) -> Vec<String> {
    got.other.iter().filter(|(_, other)| other != "Timeout").map(|(_, other)| other.clone()).collect()
}

fn error_text(what: &str, error: impl std::fmt::Debug) -> String {
    format!("{what}: {error:?}")
}

// ============================================================================================================================
// Probe 1: the receive-restart zeros sweep (`hw_rxzeros_sweep`). Plan: ~/.cache/ezsdr-fixes/2026-11-13-rxzeros/options.md §4.
// ============================================================================================================================

/// The decision rule of §4 (owner's direction D, 2026-11-13), printed before the first trial and again in the summary.
pub const RXZ_RULE: &str = "RULE (written before the data): per half-band class n_hb = min(v2(D), 3) the swept zero-count range of the gain-0 restart rate cells must stay within +-1 of the class's measured range (n_hb 3: 13-15, 2: 9-11, 1: 3-4, 0: 0); \
all four classes hold -> D: the table 0/4/11/15 stands; a class wider than +-1 -> B: a table over the swept rates, unmeasured elsewhere. \
A fresh open (rx_close, rx_open, rx_start) outside +-1 of the same D's restart range -> the fill applies to the case measured only (restarts), else to every start. \
A gain whose range moves a limit by more than 2 -> KNOWN_LIMITATIONS: the zero count is exact for the stated level only; the Mock value stays a bound. \
A precursor (non-zero samples before the 10 % point) whose peak reaches 5 % of the steady magnitude -> signal-like: the recommendation is revisited before any code. \
The candidate bound of a class is the max zero count over every counted cell of the class (every gain, every start kind), apart from the structure comparison; a level effect adds the KNOWN_LIMITATIONS line and never lowers the bound. \
A cell counts only when every planned trial (10, 15 where planned) is complete; a decision needs every planned cell counted and is otherwise INCONCLUSIVE, naming the missing cells. The warm-up trial of each cell is dumped and not counted.";

/// The largest transmit gain of the sweep's tone source, with the attenuator in the loopback (owner, 2026-10-27, bench.md T5: gain up to the
/// maximum is allowed with the attenuator in the loopback; the tone's amplitude stays at 0.4).
pub const MAX_TONE_GAIN_DB: f64 = 20.0;

/// The tone's offset from the carrier that a retune of the receiver leaves: receive at carrier + 50 kHz, so the 100 kHz tone is at 50 kHz.
pub const RETUNE_HZ: f64 = 50e3;

/// How far trial `index` of a `Start::Retune` cell moves the receive frequency from the carrier: up on even trials, back on odd ones, so every trial retunes.
pub fn retune_shift(index: usize) -> f64 {
    if index % 2 == 0 { RETUNE_HZ } else { 0.0 }
}

/// Where the 100 kHz tone lies when the receiver sits `shift` Hz above the carrier.
pub fn expected_tone_hz(shift: f64) -> f64 {
    iii::RESTART_TONE_HZ - shift
}

/// Heads of the dump: samples of each trial that are printed (fc32).
pub const HEAD_SAMPLES: usize = 40;

/// A precursor peak, as a fraction of the steady magnitude, from which it is signal-like (half of the 10 % rise threshold; fixed before the data).
pub const PRECURSOR_SIGNAL_LIKE: f64 = 0.05;

/// What the probe does to the receiver between the previous segment and the trial's timed start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// `rx_stop`, drain, a timed `rx_start` on the open streamer with the same values (`hw_iii_raw_rx_restart_fill`'s shape).
    Restart,
    /// `rx_stop`, drain, `rx_close`, the rate applied, `rx_open`, a timed `rx_start`: a freshly opened streamer's first start.
    Fresh,
    /// A restart after the receive frequency changed (carrier + 50 kHz and back, alternately): the Module's restart-with-new-values shape.
    Retune,
    /// A restart after a segment at another rate (the companion decimation) and the rate applied back.
    AfterRate,
}

impl Start {
    pub fn name(self) -> &'static str {
        match self {
            Start::Restart => "restart",
            Start::Fresh => "fresh",
            Start::Retune => "retune",
            Start::AfterRate => "after_rate",
        }
    }
}

/// One cell of the sweep: a decimation of the 200 MHz master clock, how the receiver is started, the tone source's transmit gain, the trials.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZCell {
    pub decim: u64,
    pub start: Start,
    pub gain_db: f64,
    pub trials: usize,
}

/// The DDC's half-band count for decimation `decim`: `min(v2(decim), 3)` (UHD `ddc_block_control.cpp` `set_decim`); 0 at decimation 1.
pub fn half_bands(decim: u64) -> u32 {
    decim.trailing_zeros().min(3)
}

impl ZCell {
    pub fn rate(&self) -> f64 {
        MCR as f64 / self.decim as f64
    }

    pub fn half_bands(&self) -> u32 {
        half_bands(self.decim)
    }

    pub fn name(&self) -> String {
        format!("D{} n_hb{} {} gain {:+.0} dB ({:.3} MS/s)", self.decim, self.half_bands(), self.start.name(), self.gain_db, self.rate() / 1e6)
    }

    /// Trials a cell needs complete to count: every planned one.
    pub fn needed(&self) -> usize {
        self.trials
    }
}

/// The rate cells' decimations, 20: every half-band class gets odd parts beyond {1, 5, 25} and the 8 rates of `hw_iii_raw_rx_restart_fill` that
/// the class table was read from (D = 200, 100, 40, 20, 8, 4, 2, 1). The slowest rate is 200/255 = 0.784 MS/s (>= 0.4: the PLL failed to lock at 0.098).
pub const RATE_DECIMATIONS: [u64; 20] = [1, 2, 3, 4, 6, 8, 10, 12, 20, 24, 25, 40, 50, 60, 100, 125, 200, 248, 250, 255];

/// The class-boundary cells (options.md §4): the classes that have a single or no odd-part check today get 15 trials.
pub const BOUNDARY_DECIMATIONS: [u64; 7] = [3, 125, 255, 6, 10, 50, 250];

/// The decimations of the level and start cells: 1, 2 and 100 MS/s (n_hb 3, 2, 1).
pub const LEVEL_DECIMATIONS: [u64; 3] = [200, 100, 2];

/// The transmit gains of the level cells besides 0 dB (the rate cells' level): steady about 0.01, 0.03, 0.1 at the earlier bench's attenuation.
pub const LEVEL_GAINS_DB: [f64; 2] = [10.0, 20.0];

pub const ZERO_TRIALS: usize = 10;
pub const BOUNDARY_TRIALS: usize = 15;

/// The decimation of the segment before a trial of `Start::AfterRate`.
pub fn companion(decim: u64) -> u64 {
    match decim {
        200 => 100,
        100 => 200,
        2 => 4,
        other => other * 2,
    }
}

/// The sweep's cells in the order they run: the gain-0 cells first (20 rate cells, then the three start kinds at the three level rates), then
/// the level cells by gain, so that the transmit gain changes twice.
pub fn zeros_cells() -> Vec<ZCell> {
    let mut cells: Vec<ZCell> =
        RATE_DECIMATIONS.iter().map(|&decim| ZCell { decim, start: Start::Restart, gain_db: 0.0, trials: if BOUNDARY_DECIMATIONS.contains(&decim) { BOUNDARY_TRIALS } else { ZERO_TRIALS } }).collect();
    for start in [Start::Fresh, Start::Retune, Start::AfterRate] {
        cells.extend(LEVEL_DECIMATIONS.iter().map(|&decim| ZCell { decim, start, gain_db: 0.0, trials: ZERO_TRIALS }));
    }
    for gain_db in LEVEL_GAINS_DB {
        cells.extend(LEVEL_DECIMATIONS.iter().map(|&decim| ZCell { decim, start: Start::Restart, gain_db, trials: ZERO_TRIALS }));
    }
    cells
}

/// The magnitude of the steady part: the median over the last half.
fn steady_of(magnitudes: &[f64]) -> f64 {
    let mut half = magnitudes[magnitudes.len() / 2..].to_vec();
    half.sort_by(f64::total_cmp);
    half[half.len() / 2]
}

/// The largest magnitude among the samples from the first non-zero one up to (not including) the first at 10 % of the steady magnitude, as a
/// fraction of it. 0.0 when the 10 % point is the first non-zero sample. None when the span is too short or has no steady magnitude.
pub fn precursor_peak(samples: &[(i64, Iq)]) -> Option<f64> {
    if samples.len() < 4 {
        return None;
    }
    let magnitudes: Vec<f64> = samples.iter().map(|(_, s)| f64::from(s[0]).hypot(f64::from(s[1]))).collect();
    let steady = steady_of(&magnitudes);
    if steady <= 0.0 {
        return None;
    }
    let first = magnitudes.iter().position(|m| *m > 0.0)?;
    let ten = magnitudes.iter().enumerate().skip(first).find(|(_, m)| **m >= 0.1 * steady)?.0;
    Some(magnitudes[first..ten].iter().copied().fold(0.0, f64::max) / steady)
}

/// One trial of a cell as the probe saw it.
#[derive(Debug, Clone)]
pub struct ZTrial {
    pub warmup: bool,
    pub trial: RestartTrial,
    pub expected_tone_hz: f64,
    pub precursor_peak: Option<f64>,
    /// A failed device call; the trial is then incomplete.
    pub error: Option<String>,
    /// The receive outcomes other than a timeout (an overflow, a late command...): a read that lost samples says nothing about where the first block is.
    pub rx_other: Vec<String>,
}

impl ZTrial {
    /// Why the trial does not count (`iii::incomplete_reason`, or the failed call), None when it does.
    pub fn reason(&self) -> Option<String> {
        self.error.clone().or_else(|| iii::incomplete_reason(&self.trial, self.expected_tone_hz, n47::TONE_TOLERANCE_HZ))
    }

    /// The first block is off the planned tick in a read with no overflow: after one, the first block's tick is not the start's.
    fn misplaced(&self) -> bool {
        self.rx_other.is_empty() && self.trial.first_tick.is_some_and(|tick| tick != self.trial.planned)
    }
}

/// What a cell's trials add up to (warm-ups excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct CellStats {
    pub trials: usize,
    pub complete: usize,
    /// Trials whose first block is not at the planned tick, in a read with no overflow (an overflow makes the trial INCOMPLETE, not a FAIL).
    pub misplaced: usize,
    pub errors: usize,
    pub zeros: Option<(usize, usize, usize)>,
    pub ramp10: Option<(usize, usize, usize)>,
    pub ramp90: Option<(usize, usize, usize)>,
    pub precursor_peak: Option<f64>,
}

pub fn cell_stats(trials: &[ZTrial]) -> CellStats {
    let counted: Vec<&ZTrial> = trials.iter().filter(|trial| !trial.warmup).collect();
    let complete: Vec<&&ZTrial> = counted.iter().filter(|trial| trial.reason().is_none()).collect();
    let fills = || complete.iter().filter_map(|trial| trial.trial.fill);
    CellStats {
        trials: counted.len(),
        complete: complete.len(),
        misplaced: counted.iter().filter(|trial| trial.misplaced()).count(),
        errors: counted.iter().filter(|trial| trial.error.is_some()).count(),
        zeros: spread(fills().map(|fill| fill.zeros).collect()),
        ramp10: spread(fills().map(|fill| fill.ramp10).collect()),
        ramp90: spread(fills().map(|fill| fill.ramp90).collect()),
        precursor_peak: complete.iter().filter_map(|trial| trial.precursor_peak).max_by(f64::total_cmp),
    }
}

/// FAIL when a call failed or a first block is not at its planned tick in a read with no overflow; INCONCLUSIVE below the cell's planned trials complete; else PASS, with the
/// measured spread. (The cell's meaning is in the decision, `decide`.)
pub fn cell_verdict(cell: &ZCell, stats: &CellStats) -> Verdict {
    if stats.errors > 0 {
        return Verdict::Fail(format!("{} of {} trials had a failed device call", stats.errors, stats.trials));
    }
    if stats.misplaced > 0 {
        return Verdict::Fail(format!("{} of {} trials have the first block off the planned tick", stats.misplaced, stats.trials));
    }
    if stats.complete < cell.needed() {
        return Verdict::Inconclusive(format!("{} of {} trials complete ({} needed)", stats.complete, stats.trials, cell.needed()));
    }
    Verdict::Pass(format!("{} of {} complete; zeros {:?}; ramp10 {:?}; ramp90 {:?}", stats.complete, stats.trials, stats.zeros, stats.ramp10, stats.ramp90))
}

/// The summary line of one cell.
pub fn cell_line(cell: &ZCell, stats: &CellStats) -> String {
    format!(
        "RXZ cell {}: complete {} of {} (needed {}); zeros min/med/max {:?}; ramp10 {:?}; ramp90 {:?}; precursor peak {:?}",
        cell.name(),
        stats.complete,
        stats.trials,
        cell.needed(),
        stats.zeros,
        stats.ramp10,
        stats.ramp90,
        stats.precursor_peak
    )
}

/// The classes' measured zero ranges before the sweep (options.md §2.1, 5 trials a rate, two boards): (n_hb, min, max).
pub const CURRENT_ZEROS: [(u32, usize, usize); 4] = [(0, 0, 0), (1, 3, 4), (2, 9, 11), (3, 13, 15)];

/// One half-band class after the sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassResult {
    pub half_bands: u32,
    /// The structure comparison: min and max zeros over the counted gain-0 restart rate cells of the class, None without data.
    pub observed: Option<(usize, usize)>,
    pub cells: Vec<String>,
    /// `observed` within +-1 of the class's measured range.
    pub holds: Option<bool>,
    /// The candidate bound: the max zeros over every counted cell of the class (every gain, every start kind), and how many cells that is.
    pub bound: Option<usize>,
    pub bound_cells: usize,
    /// Planned cells of the class (every gain, every start kind): `bound` is a bound only when `bound_cells` reaches this.
    pub planned_cells: usize,
}

/// The sweep's verdicts on the plan's questions.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Planned cells that are not counted (absent, a failed call, a misplaced block or fewer complete trials than planned); any makes every decision INCONCLUSIVE.
    pub missing: Vec<String>,
    pub classes: Vec<ClassResult>,
    /// D holds: every planned cell counted and all four classes hold. None while a planned cell is missing.
    pub d_holds: Option<bool>,
    /// (decimation, start kind, outside +-1 of the same decimation's restart range).
    pub start_differs: Vec<(u64, Start, bool)>,
    /// (decimation, gain, the largest shift of the min or max against the gain-0 restart cell).
    pub amplitude_shift: Vec<(u64, f64, usize)>,
    /// The largest precursor peak over every counted trial, as a fraction of the steady magnitude.
    pub precursor_peak: Option<f64>,
}

/// The zero range of `planned`'s cell among `runs`, when it counts: no failed call, no misplaced block, and every planned trial complete.
fn counted_range(planned: &ZCell, runs: &[(ZCell, CellStats)]) -> Option<(usize, usize)> {
    let (_, stats) = runs.iter().find(|(cell, _)| cell.decim == planned.decim && cell.start == planned.start && cell.gain_db == planned.gain_db)?;
    (stats.errors == 0 && stats.misplaced == 0 && stats.complete >= planned.needed()).then_some(()).and(stats.zeros).map(|(min, _, max)| (min, max))
}

fn outside_by_one(range: (usize, usize), base: (usize, usize)) -> bool {
    range.0 + 1 < base.0 || range.1 > base.1 + 1
}

/// §4's decision rule (`RXZ_RULE`) over every cell run on one board, against the planned cell set (`zeros_cells`).
pub fn decide(runs: &[(ZCell, CellStats)]) -> Decision {
    let planned = zeros_cells();
    let counted: Vec<(&ZCell, (usize, usize))> = planned.iter().filter_map(|cell| counted_range(cell, runs).map(|range| (cell, range))).collect();
    let missing: Vec<String> = planned.iter().filter(|cell| !counted.iter().any(|(c, _)| c == cell)).map(ZCell::name).collect();
    let classes: Vec<ClassResult> = CURRENT_ZEROS
        .iter()
        .map(|&(class, lo, hi)| {
            let of_class: Vec<&(&ZCell, (usize, usize))> = counted.iter().filter(|(cell, _)| cell.half_bands() == class).collect();
            let structure: Vec<&&(&ZCell, (usize, usize))> = of_class.iter().filter(|(cell, _)| cell.start == Start::Restart && cell.gain_db == 0.0).collect();
            let observed = structure.iter().map(|(_, range)| *range).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
            ClassResult {
                half_bands: class,
                observed,
                cells: structure.iter().map(|(cell, _)| cell.name()).collect(),
                holds: observed.map(|range| !outside_by_one(range, (lo, hi))),
                bound: of_class.iter().map(|(_, range)| range.1).max(),
                bound_cells: of_class.len(),
                planned_cells: planned.iter().filter(|cell| cell.half_bands() == class).count(),
            }
        })
        .collect();
    let d_holds = missing.is_empty().then(|| classes.iter().all(|class| class.holds == Some(true)));
    let range_of = |decim: u64, start: Start, gain_db: f64| counted.iter().find(|(cell, _)| cell.decim == decim && cell.start == start && cell.gain_db == gain_db).map(|(_, range)| *range);
    let mut start_differs = Vec::new();
    let mut amplitude_shift = Vec::new();
    for decim in LEVEL_DECIMATIONS {
        let Some(base) = range_of(decim, Start::Restart, 0.0) else { continue };
        for start in [Start::Fresh, Start::Retune, Start::AfterRate] {
            if let Some(range) = range_of(decim, start, 0.0) {
                start_differs.push((decim, start, outside_by_one(range, base)));
            }
        }
        for gain in LEVEL_GAINS_DB {
            if let Some(range) = range_of(decim, Start::Restart, gain) {
                amplitude_shift.push((decim, gain, base.0.abs_diff(range.0).max(base.1.abs_diff(range.1))));
            }
        }
    }
    let precursor_peak = runs.iter().filter_map(|(_, stats)| stats.precursor_peak).max_by(f64::total_cmp);
    Decision { missing, classes, d_holds, start_differs, amplitude_shift, precursor_peak }
}

/// The summary lines of a decision: the observations (one line per class, the start kinds, the levels and the precursor), then the four decisions of the
/// rule, each INCONCLUSIVE while a planned cell is missing.
pub fn decision_lines(decision: &Decision) -> Vec<String> {
    let mut lines: Vec<String> = decision
        .classes
        .iter()
        .map(|class| {
            let current = CURRENT_ZEROS.iter().find(|(n, _, _)| *n == class.half_bands).map(|(_, lo, hi)| (*lo, *hi));
            let incomplete = if class.bound_cells < class.planned_cells { " (INCOMPLETE: a planned cell of the class is not counted, so this is no bound)" } else { "" };
            format!(
                "RXZ class n_hb {}: structure (gain-0 restart rate cells, {} counted) zeros {:?} (measured before {:?}, +-1): holds {:?}; this board's candidate bound {:?}{incomplete} = max over {} of {} planned cell(s) of the class counted (every gain, every start kind)",
                class.half_bands,
                class.cells.len(),
                class.observed,
                current,
                class.holds,
                class.bound,
                class.bound_cells,
                class.planned_cells
            )
        })
        .collect();
    lines.push(format!(
        "RXZ observed: start kinds against restart (decimation, kind, outside +-1) {:?}; level shifts (decimation, gain, shift) {:?}; largest precursor peak {:?}",
        decision.start_differs, decision.amplitude_shift, decision.precursor_peak
    ));
    let inconclusive = |number: &str| {
        format!("RXZ decision {number}: INCONCLUSIVE: {} planned cell(s) not counted (every planned trial complete, no failed call): {}", decision.missing.len(), decision.missing.join("; "))
    };
    lines.push(match decision.d_holds {
        _ if !decision.missing.is_empty() => inconclusive("1"),
        Some(true) => "RXZ decision 1: D holds on this board: the table by half-band count stands; the class value is the max over cells and boards".to_owned(),
        _ => "RXZ decision 1: D does NOT hold on this board (a class is wider than +-1): B, a table over the swept rates, unmeasured elsewhere".to_owned(),
    });
    let fresh: Vec<&(u64, Start, bool)> = decision.start_differs.iter().filter(|(_, start, _)| *start == Start::Fresh).collect();
    lines.push(if !decision.missing.is_empty() {
        inconclusive("2 (every start or restarts only)")
    } else if fresh.iter().any(|(_, _, differs)| *differs) {
        format!("RXZ decision 2: a fresh open differs from a restart ({fresh:?}): the fill applies to the case measured (restarts) only")
    } else {
        "RXZ decision 2: a fresh open is within +-1 of a restart at every level rate: the fill applies to every start".to_owned()
    });
    let moved: Vec<&(u64, f64, usize)> = decision.amplitude_shift.iter().filter(|(_, _, shift)| *shift > 2).collect();
    lines.push(if !decision.missing.is_empty() {
        inconclusive("3 (amplitude)")
    } else if moved.is_empty() {
        format!("RXZ decision 3: the gain moves a limit by at most 2 samples ({:?}): no KNOWN_LIMITATIONS entry for the level", decision.amplitude_shift)
    } else {
        format!("RXZ decision 3: the gain moves a limit by more than 2 ({moved:?}): KNOWN_LIMITATIONS says the zero count is exact for the stated level only; the candidate bounds above already include every level")
    });
    lines.push(match decision.precursor_peak {
        _ if !decision.missing.is_empty() => inconclusive("4 (shape)"),
        Some(peak) if peak >= PRECURSOR_SIGNAL_LIKE => {
            format!("RXZ decision 4 (shape): a precursor reaches {peak:.3} of the steady magnitude: signal-like; revisit the recommendation (read the head dump)")
        }
        Some(peak) => format!("RXZ decision 4 (shape): the precursor peaks at {peak:.3} of the steady magnitude, below {PRECURSOR_SIGNAL_LIKE}: near zero; confirm with the head dump"),
        None => "RXZ decision 4 (shape): INCONCLUSIVE: no precursor measured".to_owned(),
    });
    lines
}

/// The last line of the sweep: the rule, repeated, and the verdict of the whole run.
pub fn summary_line(decision: &Decision) -> String {
    format!("RXZ summary: d_holds {:?}; {RXZ_RULE}", decision.d_holds)
}

/// Opens the tone source: the transmit settings and streamer, at `gain_db` within `MAX_TONE_GAIN_DB` and the bench's frequency and rate bounds.
fn tone_tx(device: &dyn Device, rate: f64, freq: f64, gain_db: f64) {
    assert!(SAFE_TX_HZ.contains(&freq) && rate <= SAFE_TX_MAX_RATE && (0.0..=MAX_TONE_GAIN_DB).contains(&gain_db), "bench.md RF safety: {freq} Hz at {rate} S/s, gain {gain_db} dB");
    let settings = Settings { rate: Some(rate), freq: Some(freq), gain: Some(gain_db), antenna: Some("TX/RX".to_owned()) };
    let applied = device.apply(Dir::Tx, d9_channel(), &settings).unwrap();
    assert!((applied.gain - gain_db).abs() < 0.5 && SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
    device.tx_open(0, &[d9_channel()]).unwrap();
}

fn set_tx_gain(device: &dyn Device, gain_db: f64) {
    assert!((0.0..=MAX_TONE_GAIN_DB).contains(&gain_db), "bench.md RF safety: gain {gain_db} dB");
    let applied = device.apply(Dir::Tx, d9_channel(), &Settings { gain: Some(gain_db), ..Settings::default() }).unwrap();
    assert!((applied.gain - gain_db).abs() < 0.5, "asked {gain_db} dB, applied {applied:?}");
}

/// Puts the transmit gain back to 0 dB when the probe ends, by return or by a panic's unwind.
struct GainReset<'a>(&'a dyn Device);

impl Drop for GainReset<'_> {
    fn drop(&mut self) {
        let _ = self.0.apply(Dir::Tx, d9_channel(), &Settings { gain: Some(0.0), ..Settings::default() });
    }
}

/// `fc32` samples as the dump writes them: `re,im re,im ...`.
fn head_text(samples: &[(i64, Iq)]) -> String {
    samples.iter().take(HEAD_SAMPLES).map(|(_, s)| format!("{},{}", s[0], s[1])).collect::<Vec<_>>().join(" ")
}

/// The analysed span of a trial at `per` ticks a sample: 4 000 samples, at least 2 ms, rounded up to a whole number of the tone analysis's
/// 25-sample windows, so that it holds whole samples and ends on a window (D = 3, 6, 12, 24, 60 do not divide 2 ms).
pub fn span_ticks(per: i64) -> i64 {
    let window = 25 * per;
    ((4_000 * per).max(2 * TICKS_PER_MS) + window - 1) / window * window
}

/// What a trial's capture says over its analysed span: the samples in it, whether it is covered, the tone of its last millisecond and the fill.
pub struct SpanAnalysis {
    pub samples: Vec<(i64, Iq)>,
    pub covered: bool,
    pub tone_hz: Option<f64>,
    pub fill: Option<iii::Fill>,
}

/// Every sample is one spacing from the next: a gap or an overlap (an overflow's time jump) makes it false.
pub fn contiguous(samples: &[(i64, Iq)], per: i64) -> bool {
    samples.windows(2).all(|pair| pair[1].0 - pair[0].0 == per)
}

/// The analysis of `captured` (tick, sample) from `planned` over `span_ticks(per)`: covered means the first block exists, every sample is one spacing
/// from the next, the span holds all `span / per` of them and the read saw nothing but timeouts besides samples.
pub fn analyse_span(captured: &[(i64, Iq)], first_tick: Option<i64>, only_timeouts: bool, planned: i64, per: i64) -> SpanAnalysis {
    let span = span_ticks(per);
    let samples: Vec<(i64, Iq)> = captured.iter().copied().filter(|(tick, _)| *tick >= planned && *tick < planned + span).collect();
    let covered = first_tick.is_some() && contiguous(&samples, per) && samples.len() as i64 == span / per && only_timeouts;
    let tone_hz = first_ms_frequency(&samples, planned + span - TICKS_PER_MS, per);
    let fill = iii::restart_fill(&samples, per);
    SpanAnalysis { samples, covered, tone_hz, fill }
}

/// The measured part of one trial: a tone burst 20 ms before a timed start 50 ms ahead (the shape of `iii::restart_fill_trials`), the span read, the
/// receiver stopped and drained. The trial's printed trace line is returned with it.
fn measured_trial(device: &dyn Device, cell: &ZCell, applied: f64, expected_tone_hz: f64, tone: &[Iq], warmup: bool, index: usize) -> ZTrial {
    let per = cell.decim as i64;
    let span = span_ticks(per);
    let outcome = (|| -> Result<_, String> {
        let planned = align_to(device.time_now().map_err(|e| error_text("time_now", e))? + 50 * TICKS_PER_MS, per);
        let source_at = align_to(device.time_now().map_err(|e| error_text("time_now", e))? + 20 * TICKS_PER_MS, 200);
        let got = std::thread::scope(|scope| {
            let source = scope.spawn(|| send_burst(device, tone, Some(source_at)));
            let started = device.rx_start(0, planned).map_err(|e| error_text("rx_start", e));
            let got = started.map(|()| read(device, Wall::from_secs(2), Some(per), |r| r.end(per).is_some_and(|end| end >= planned + span)));
            let stopped = device.rx_stop(0).map_err(|e| error_text("rx_stop", e));
            source.join().expect("the tone source's send");
            got.and_then(|got| stopped.map(|()| got))
        })?;
        Ok((planned, source_at, got))
    })();
    let (planned, source_at, got) = match outcome {
        Ok(done) => done,
        Err(text) => return failed_trial(cell, warmup, index, text),
    };
    let (ack, _) = super::raw::wait_burst_ack(device, Wall::from_secs(3));
    let first_tick = got.blocks.first().map(|(tick, _)| *tick);
    let SpanAnalysis { samples: span_samples, covered, tone_hz, fill } = analyse_span(&got.samples, first_tick, got.other.iter().all(|(_, other)| other == "Timeout"), planned, per);
    let trial = RestartTrial { drained: true, asked: cell.rate(), applied, planned, first_tick, covered, tone_hz, fill };
    let done = ZTrial { warmup, trial, expected_tone_hz, precursor_peak: precursor_peak(&span_samples), error: None, rx_other: rx_outcomes(&got) };
    println!(
        "RXZ trial {}",
        json!({
            "cell": cell.name(), "decim": cell.decim, "n_hb": cell.half_bands(), "start": cell.start.name(), "gain_db": cell.gain_db, "rate": cell.rate(), "index": index, "warmup": warmup,
            "planned": planned, "first_tick": first_tick, "source_at": source_at, "source_ack": ack, "applied": applied, "covered": covered, "tone_hz": tone_hz, "expected_tone_hz": expected_tone_hz,
            "other_outcomes": done.rx_other, "contiguous": contiguous(&span_samples, per),
            "fill": fill.map(|fill| json!({ "zeros": fill.zeros, "ramp10": fill.ramp10, "ramp90": fill.ramp90, "steady": fill.steady })),
            "precursor_peak": done.precursor_peak, "complete": done.reason().is_none(), "reason": done.reason(), "head": head_text(&span_samples),
        })
    );
    done
}

/// Stops the receiver and reads until it is quiet; false when it did not go quiet.
fn quiet_rx(device: &dyn Device) -> bool {
    device.rx_stop(0).ok();
    drain(device).quiet
}

fn rx_settings(rate: Option<f64>, freq: Option<f64>) -> Settings {
    Settings { rate, freq, ..Settings::default() }
}

/// A short segment at `decim`'s rate, discarded: the segment before an `AfterRate` trial.
fn companion_segment(device: &dyn Device, decim: u64) -> Result<(), String> {
    device.apply(Dir::Rx, d9_channel(), &rx_settings(Some(MCR as f64 / decim as f64), None)).map_err(|e| error_text("apply companion rate", e))?;
    let at = align_to(device.time_now().map_err(|e| error_text("time_now", e))? + 20 * TICKS_PER_MS, decim as i64);
    device.rx_start(0, at).map_err(|e| error_text("rx_start companion", e))?;
    read(device, Wall::from_secs(2), None, |r| r.blocks.len() >= 3);
    if quiet_rx(device) { Ok(()) } else { Err("the companion segment did not go quiet".to_owned()) }
}

/// One cell: a warm-up trial then `cell.trials`. Every trial first stops and drains the receiver, then does what `cell.start` says.
fn run_cell(device: &dyn Device, hz: f64, cell: &ZCell) -> Vec<ZTrial> {
    let rate = cell.rate();
    let tone = iq(&tone_wave(1e6, 60_000, iii::RESTART_TONE_HZ));
    let mut trials = Vec::new();
    if !quiet_rx(device) {
        return (0..cell.trials).map(|index| failed_trial(cell, false, index, "the old stream did not drain quietly".to_owned())).collect();
    }
    let mut applied = match device.apply(Dir::Rx, d9_channel(), &rx_settings(Some(rate), Some(hz))) {
        Ok(applied) => applied.rate,
        Err(error) => return (0..cell.trials).map(|index| failed_trial(cell, false, index, error_text("apply rate", error.clone()))).collect(),
    };
    for index in 0..=cell.trials {
        let warmup = index == 0;
        let mut expected = iii::RESTART_TONE_HZ;
        let prepared = (|| -> Result<(), String> {
            if !quiet_rx(device) {
                return Err("the previous segment did not go quiet".to_owned());
            }
            match cell.start {
                Start::Restart => {}
                Start::Fresh => {
                    device.rx_close(0);
                    applied = device.apply(Dir::Rx, d9_channel(), &rx_settings(Some(rate), None)).map_err(|e| error_text("apply rate", e))?.rate;
                    device.rx_open(0, &[d9_channel()]).map_err(|e| error_text("rx_open", e))?;
                }
                Start::Retune => {
                    let shift = retune_shift(index);
                    device.apply(Dir::Rx, d9_channel(), &rx_settings(None, Some(hz + shift))).map_err(|e| error_text("apply frequency", e))?;
                    expected = expected_tone_hz(shift);
                }
                Start::AfterRate => {
                    companion_segment(device, companion(cell.decim))?;
                    applied = device.apply(Dir::Rx, d9_channel(), &rx_settings(Some(rate), None)).map_err(|e| error_text("apply rate", e))?.rate;
                }
            }
            Ok(())
        })();
        trials.push(match prepared {
            Ok(()) => measured_trial(device, cell, applied, expected, &tone, warmup, index),
            Err(text) => failed_trial(cell, warmup, index, text),
        });
    }
    if cell.start == Start::Retune {
        device.apply(Dir::Rx, d9_channel(), &rx_settings(None, Some(hz))).ok();
    }
    trials
}

/// A trial that ended on a failed device call (printed, so the log keeps it).
fn failed_trial(cell: &ZCell, warmup: bool, index: usize, error: String) -> ZTrial {
    println!("RXZ trial {}", json!({ "cell": cell.name(), "index": index, "warmup": warmup, "complete": false, "error": error }));
    let trial = RestartTrial { drained: true, asked: cell.rate(), applied: f64::NAN, planned: 0, first_tick: None, covered: false, tone_hz: None, fill: None };
    ZTrial { warmup, trial, expected_tone_hz: iii::RESTART_TONE_HZ, precursor_peak: None, error: Some(error), rx_other: Vec::new() }
}

/// The sweep on `device`: every cell in order; each cell's trials print `RXZ trial` lines and its summary `RXZ cell`. Returns the cells with their
/// trials. The tone source transmits at the cell's gain (0 dB back at the end, also by an unwind).
pub fn zeros_sweep(device: &dyn Device, cells: &[ZCell]) -> Vec<(ZCell, Vec<ZTrial>)> {
    assert_slot_a_only(d9_channel());
    println!("{RXZ_RULE}");
    let hz = pin_slot_a(device);
    tone_tx(device, 1e6, hz, 0.0);
    raw_rx(device, 1e6, hz);
    let _reset = GainReset(device);
    let mut gain = 0.0;
    let mut runs = Vec::new();
    for cell in cells {
        if cell.gain_db != gain {
            set_tx_gain(device, cell.gain_db);
            gain = cell.gain_db;
        }
        let trials = run_cell(device, hz, cell);
        println!("{}", cell_line(cell, &cell_stats(&trials)));
        runs.push((*cell, trials));
    }
    device.rx_stop(0).ok();
    let _ = drain(device);
    runs
}

/// The per-cell verdicts of a sweep and its printed decision: `(claim, verdict)` for each cell, then the decision's lines and the summary are printed.
pub fn zeros_report(runs: &[(ZCell, Vec<ZTrial>)]) -> Vec<(String, Verdict)> {
    let stats: Vec<(ZCell, CellStats)> = runs.iter().map(|(cell, trials)| (*cell, cell_stats(trials))).collect();
    let decision = decide(&stats);
    for line in decision_lines(&decision) {
        println!("{line}");
    }
    println!("{}", summary_line(&decision));
    stats.iter().map(|(cell, stats)| (format!("rxz {}", cell.name()), cell_verdict(cell, stats))).collect()
}

// ============================================================================================================================
// Probe 2: the #75 probes (`hw_75_t3_probes`). Plan: ~/.cache/ezsdr-fixes/2026-11-13-n75/options.md §2.4 and the Fable review's bench items.
// ============================================================================================================================

/// One report read from the transmit streamer: the code, its tick, the channel position, and the wall milliseconds since the probe's reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub code: TxCode,
    pub tick: Option<i64>,
    pub channel: usize,
    pub at_ms: u64,
}

impl Seen {
    fn json(&self) -> Json {
        json!({ "code": format!("{:?}", self.code), "tick": self.tick, "channel": self.channel, "at_ms": self.at_ms })
    }
}

/// Reads `tx_async` until `until` or until `done(reports so far)` holds.
fn collect_reports(device: &dyn Device, since: Instant, until: Instant, done: impl Fn(&[Seen]) -> bool) -> Result<Vec<Seen>, String> {
    let mut out = Vec::new();
    while Instant::now() < until && !done(&out) {
        let slice = until.saturating_duration_since(Instant::now()).min(Wall::from_millis(20));
        match device.tx_async(0, slice) {
            Ok(Some(TxReport { code, tick, channel })) => out.push(Seen { code, tick, channel, at_ms: since.elapsed().as_millis() as u64 }),
            Ok(None) => {}
            Err(error) => return Err(error_text("tx_async", error)),
        }
    }
    Ok(out)
}

/// The four-zero untimed SOB+EOB burst of the recovery flush (UR-22) on every channel position; the samples accepted per channel.
fn send_t3(device: &dyn Device, positions: usize, timeout: Wall) -> Result<usize, String> {
    let zeros = [Iq::default(); 4];
    let buffers: Vec<&[Iq]> = (0..positions).map(|_| &zeros[..]).collect();
    device.tx_send(0, &buffers, None, true, true, timeout).map_err(|e| error_text("tx_send (T3)", e))
}

fn burst_acks(reports: &[Seen]) -> Vec<&Seen> {
    reports.iter().filter(|seen| seen.code == TxCode::BurstAck).collect()
}

/// ACK counts by channel position.
pub fn acks_by_position(reports: &[Seen]) -> BTreeMap<usize, usize> {
    let mut counts = BTreeMap::new();
    for seen in burst_acks(reports) {
        *counts.entry(seen.channel).or_default() += 1;
    }
    counts
}

/// Why the BurstAcks among `reports` cannot be attributed to a T3 sent at device tick `sent_at`: one has no tick, or one is older than the send.
fn unattributable(reports: &[Seen], sent_at: i64) -> Option<String> {
    let acks = burst_acks(reports);
    if acks.iter().any(|seen| seen.tick.is_none()) {
        return Some("an ACK without a tick cannot be attributed to this T3".to_owned());
    }
    acks.iter().filter_map(|seen| seen.tick).find(|tick| *tick < sent_at).map(|tick| format!("an ACK (tick {tick}) is before T3's send {sent_at}: it cannot be attributed to this T3"))
}

fn other_codes(reports: &[Seen]) -> Vec<String> {
    reports.iter().filter(|seen| seen.code != TxCode::BurstAck).map(|seen| format!("{:?}", seen.code)).collect()
}

/// Opens one raw transmit and one raw receive streamer at `rate` on the bench frequency, gain 0 (the closed streamers first).
fn open_loopback(device: &dyn Device, rate: f64, hz: f64, with_rx: bool) {
    device.tx_close(0);
    device.rx_close(0);
    super::raw::raw_tx(device, rate, hz);
    if with_rx {
        raw_rx(device, rate, hz);
    }
}

// ---- (a) skip T3 and compare the next burst (options.md §2.4)

/// What precedes burst B: an unclosed burst A and a reopen with or without T3, or a closed burst A and a reopen without T3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipKind {
    /// Control: A is left open (no EOB), the streamer is closed and reopened, T3 is sent and acknowledged.
    OpenThenFlush,
    /// A is left open, the streamer is reopened and T3 is skipped.
    OpenThenSkip,
    /// A is closed by its EOB, the streamer is reopened and T3 is skipped (does skipping alone cost anything?).
    ClosedThenSkip,
}

impl SkipKind {
    pub fn name(self) -> &'static str {
        match self {
            SkipKind::OpenThenFlush => "open_then_flush",
            SkipKind::OpenThenSkip => "open_then_skip",
            SkipKind::ClosedThenSkip => "closed_then_skip",
        }
    }
}

pub const SKIP_RATES: [f64; 2] = [1e6, 200e6];
pub const P75_TRIALS: usize = 10;

/// The tolerance on B's onset against its bracketing control, in device ticks (2 us): the check is for a stale header's ms-scale mistiming, the
/// onset offset itself is in every trace.
pub const SKIP_TOLERANCE_TICKS: i64 = 400;

/// The stability rule of the control: the control trials' onset offsets of one rate (max - min), and the two controls around one skipping trial, must
/// spread at most this (half the tolerance: a skipping trial that fails lies more than that beyond every control trial).
pub const CONTROL_SPREAD_TICKS: i64 = SKIP_TOLERANCE_TICKS / 2;

/// The prefix of a verdict that is a failed device call, not an observation about the device's behaviour.
pub const TRANSPORT: &str = "transport/setup failure";

/// Burst B's length in samples at `rate`: 1 ms, at most 20 000 samples (one send chunk: 0.1 ms at 200 MS/s, where the host cannot feed more in real time).
pub fn b_samples(rate: f64) -> usize {
    ((rate / 1000.0) as usize).min(20_000)
}

/// One trial of (a). `onset` and `length` are B's first sample above half the capture's steady magnitude and its duration in ticks; `t3_ack` the
/// control's T3 ACK tick; `reports` every report read after B was sent; `rx_other` the receive outcomes other than a timeout and `contiguous` whether the
/// capture has no gap (an overflow loses samples and moves the onset); `spans_b` whether the capture runs from before B's window to past its end (a read
/// cut short by its 2 s limit or full of timeouts does not). `round` is the trial's place in the interleaved order: a skipping trial of
/// round r lies between the controls r and r + 1.
#[derive(Debug, Clone)]
pub struct SkipTrial {
    pub kind: SkipKind,
    pub round: usize,
    pub rate: f64,
    pub b_start: i64,
    pub b_end: i64,
    pub per: i64,
    pub onset: Option<i64>,
    pub length: Option<i64>,
    pub t3_ack: Option<i64>,
    pub t3_sent: Option<usize>,
    pub reports: Vec<Seen>,
    pub rx_other: Vec<String>,
    pub contiguous: bool,
    pub spans_b: bool,
    pub error: Option<String>,
}

impl SkipTrial {
    /// No receive outcome besides samples and timeouts, no gap, and the capture spans B's window: its onset and length are B's to read.
    pub fn clean(&self) -> bool {
        self.rx_other.is_empty() && self.contiguous && self.spans_b
    }

    pub fn offset(&self) -> Option<i64> {
        self.onset.map(|onset| onset - self.b_start)
    }

    /// Takes what B's read says: the outcomes besides samples and timeouts, whether the samples are contiguous and span B's window, B's onset and length.
    pub fn record_capture(&mut self, got: &Read) {
        self.rx_other = rx_outcomes(got);
        self.contiguous = contiguous(&got.samples, self.per);
        self.spans_b = spans_b(&got.samples, self.per, self.b_start, self.b_end);
        (self.onset, self.length) = match burst_onset(&got.samples, self.per) {
            Some((onset, length)) => (Some(onset), Some(length)),
            None => (None, None),
        };
    }

    pub fn trace(&self) -> Json {
        json!({
            "kind": self.kind.name(), "round": self.round, "rate": self.rate, "b_start": self.b_start, "b_end": self.b_end, "onset": self.onset, "onset_offset": self.offset(), "length_ticks": self.length,
            "expected_length_ticks": self.b_end - self.b_start, "t3_sent": self.t3_sent, "t3_ack": self.t3_ack, "reports": self.reports.iter().map(Seen::json).collect::<Vec<_>>(),
            "b_acks": acks_by_position(&self.reports), "old_acks": self.old_acks(), "other": other_codes(&self.reports), "rx_other": self.rx_other, "contiguous": self.contiguous, "spans_b": self.spans_b, "error": self.error,
        })
    }

    /// Ticks of the ACKs that precede B's start: an earlier burst's, not B's.
    pub fn old_acks(&self) -> Vec<i64> {
        burst_acks(&self.reports).iter().filter_map(|seen| seen.tick).filter(|tick| *tick < self.b_start).collect()
    }
}

/// B's first sample at or above half of the capture's 90th-percentile magnitude, and the burst's duration from it to the last such sample (plus a sample).
/// None when the capture holds no burst: the 90th percentile must reach eight times the noise floor (the median, at least one sc16 LSB: a quantised noise
/// whose median is zero has a floor too), and the samples at or above half of it must fill 90 % of the stretch from the first to the last (a burst is
/// contiguous; sparse noise spikes are not).
pub fn burst_onset(samples: &[(i64, Iq)], per: i64) -> Option<(i64, i64)> {
    if samples.len() < 16 {
        return None;
    }
    let magnitudes: Vec<f64> = samples.iter().map(|(_, s)| f64::from(s[0]).hypot(f64::from(s[1]))).collect();
    let mut sorted = magnitudes.clone();
    sorted.sort_by(f64::total_cmp);
    let (floor, top) = (sorted[sorted.len() / 2].max(SC16_LSB), sorted[sorted.len() * 9 / 10]);
    if top < 8.0 * floor {
        return None;
    }
    let level = 0.5 * top;
    let first = magnitudes.iter().position(|m| *m >= level)?;
    let last = magnitudes.iter().rposition(|m| *m >= level)?;
    let dense = magnitudes[first..=last].iter().filter(|m| **m >= level).count() as f64 >= 0.9 * (last - first + 1) as f64;
    dense.then(|| (samples[first].0, samples[last].0 - samples[first].0 + per))
}

/// Whether `samples` (tick-ordered) start at or before `b_start - (b_end - b_start)` and end at or after `b_end + 1.5 (b_end - b_start)`: the window the read
/// of (a) aims at. Without it a missing burst is a short capture, not a burst that did not play.
pub fn spans_b(samples: &[(i64, Iq)], per: i64, b_start: i64, b_end: i64) -> bool {
    let len = b_end - b_start;
    match (samples.first(), samples.last()) {
        (Some((first, _)), Some((last, _))) => *first <= b_start - len && *last + per >= b_end + len * 3 / 2,
        _ => false,
    }
}

/// One sc16 least significant bit as an fc32 magnitude.
pub const SC16_LSB: f64 = 1.0 / 32768.0;

/// Whether one rate's controls are stable: the controls whose capture is clean (`SkipTrial::clean`; a dirty one is not read) each have a measured onset, two
/// at least, and the offsets spread at most `CONTROL_SPREAD_TICKS`.
pub fn control_stable(controls: &[SkipTrial]) -> Result<(), String> {
    let clean: Vec<&SkipTrial> = controls.iter().filter(|control| control.clean()).collect();
    let offsets: Vec<i64> = clean.iter().filter_map(|control| control.offset()).collect();
    if offsets.len() < 2 || offsets.len() < clean.len() {
        return Err(format!("{} of {} control trials are clean and have a measured onset", offsets.len(), controls.len()));
    }
    let (min, _, max) = spread(offsets).expect("two offsets");
    if max - min > CONTROL_SPREAD_TICKS { Err(format!("the control's onset offsets spread {} ticks (min {min}, max {max}), more than {CONTROL_SPREAD_TICKS}", max - min)) } else { Ok(()) }
}

/// The reference of the skipping trial of `round`: the midpoint of the two controls around it, when both are clean, have an onset and lie within
/// `CONTROL_SPREAD_TICKS` of each other.
pub fn bracket_reference(controls: &[SkipTrial], round: usize) -> Result<i64, String> {
    let offset = |index: usize| controls.get(index).filter(|control| control.clean()).and_then(SkipTrial::offset).ok_or_else(|| format!("control {index} has no clean, measured onset"));
    let (before, after) = (offset(round)?, offset(round + 1)?);
    if (before - after).abs() > CONTROL_SPREAD_TICKS {
        return Err(format!("the controls around round {round} are {before:+} and {after:+} ticks, more than {CONTROL_SPREAD_TICKS} apart"));
    }
    Ok(before.min(after) + (before - after).abs() / 2)
}

/// The verdict of one (a) trial against its reference (`reference`: None while judging a control; else the bracketing controls' midpoint, or why there is
/// none). FAIL: a failed call, a report other than B's ACK, no ACK for B, an ACK outside `n47::burst_ack_holds`, no burst in the capture, a length more
/// than 2 us off the nominal, or an onset more than `SKIP_TOLERANCE_TICKS` from the reference. B's ACKs are those at or after B's start by tick: an
/// earlier ACK (burst A's) is reported (`old_acks`) and not counted. INCONCLUSIVE: the control's T3 was not acknowledged, a skipping trial has no stable
/// bracketing control (a difference is attributed to T3 only against one), an ACK has no tick, or the capture holds a receive overflow or a gap (the
/// onset and length are then not B's), or the capture is too short to span B's window (`spans_b`).
pub fn skip_trial_verdict(trial: &SkipTrial, reference: Option<Result<i64, String>>) -> Verdict {
    if let Some(error) = &trial.error {
        return Verdict::Fail(format!("{TRANSPORT}: {error}"));
    }
    if trial.kind == SkipKind::OpenThenFlush && trial.t3_ack.is_none() {
        return Verdict::Inconclusive(format!("the control's T3 was not acknowledged (sent {:?})", trial.t3_sent));
    }
    if let Some(Err(why)) = &reference {
        return Verdict::Inconclusive(format!("no stable bracketing control for this trial: {why}"));
    }
    let other = other_codes(&trial.reports);
    if !other.is_empty() {
        return Verdict::Fail(format!("the device reported {other:?} after the reopen (T3 or B)"));
    }
    if burst_acks(&trial.reports).iter().any(|seen| seen.tick.is_none()) {
        return Verdict::Inconclusive("an ACK without a tick cannot be attributed to B".to_owned());
    }
    let old = trial.old_acks();
    let acks: Vec<i64> = burst_acks(&trial.reports).iter().filter_map(|seen| seen.tick).filter(|tick| *tick >= trial.b_start).collect();
    match acks.as_slice() {
        [] => return Verdict::Fail("no BurstAck for burst B".to_owned()),
        [ack] if n47::burst_ack_holds(*ack, trial.b_end, trial.per) => {}
        [ack] => return Verdict::Fail(format!("B's BurstAck is at {ack}, {:+} ticks from its end {}", ack - trial.b_end, trial.b_end)),
        many => return Verdict::Fail(format!("{} BurstAcks for burst B: {many:?}", many.len())),
    }
    if !trial.clean() {
        return Verdict::Inconclusive(format!(
            "the capture is not clean (receive outcomes {:?}, contiguous {}, spans B's window {}): B's onset and length are not reliable",
            trial.rx_other, trial.contiguous, trial.spans_b
        ));
    }
    let (Some(offset), Some(length)) = (trial.offset(), trial.length) else {
        return Verdict::Fail("burst B is not in the capture".to_owned());
    };
    let nominal = trial.b_end - trial.b_start;
    if (length - nominal).abs() > SKIP_TOLERANCE_TICKS {
        return Verdict::Fail(format!("B lasts {length} ticks in the capture, {nominal} sent"));
    }
    let old = if old.is_empty() { String::new() } else { format!("; {} earlier ACK(s) {old:?} not counted", old.len()) };
    match reference {
        None => Verdict::Pass(format!("control: B on time (onset {offset:+} ticks from its start), ACKed{old}")),
        Some(Ok(reference)) if (offset - reference).abs() > SKIP_TOLERANCE_TICKS => {
            Verdict::Fail(format!("B's onset is {offset:+} ticks from its start, the bracketing controls' midpoint is {reference:+}"))
        }
        Some(Ok(reference)) => Verdict::Pass(format!("B's onset {offset:+} ticks, the bracketing controls' {reference:+}; ACKed{old}")),
        Some(Err(_)) => unreachable!("returned above"),
    }
}

fn skip_trial(device: &dyn Device, kind: SkipKind, rate: f64, round: usize) -> SkipTrial {
    let per = (MCR as f64 / rate) as i64;
    let n_b = b_samples(rate);
    let mut trial = SkipTrial {
        kind,
        round,
        rate,
        b_start: 0,
        b_end: 0,
        per,
        onset: None,
        length: None,
        t3_ack: None,
        t3_sent: None,
        reports: Vec::new(),
        rx_other: Vec::new(),
        contiguous: true,
        spans_b: true,
        error: None,
    };
    let result = (|| -> Result<(), String> {
        let now = || device.time_now().map_err(|e| error_text("time_now", e));
        let a = iq(&tone_wave(rate, 1_000, 100e3));
        let b = iq(&tone_wave(rate, n_b, 100e3));
        device.tx_open(0, &[d9_channel()]).map_err(|e| error_text("tx_open", e))?;
        let a_start = align_to(now()? + 30 * TICKS_PER_MS, per);
        if kind == SkipKind::ClosedThenSkip {
            send_burst(device, &a, Some(a_start));
        } else {
            device.tx_send(0, &[&a[..]], Some(a_start), true, false, Wall::from_secs(1)).map_err(|e| error_text("tx_send (A)", e))?;
        }
        wait_device_time(device, a_start + 2 * TICKS_PER_MS);
        device.tx_close(0);
        device.tx_open(0, &[d9_channel()]).map_err(|e| error_text("tx_open (reopen)", e))?;
        let since = Instant::now();
        if kind == SkipKind::OpenThenFlush {
            trial.t3_sent = Some(send_t3(device, 1, Wall::from_millis(100))?);
            let acks = collect_reports(device, since, since + Wall::from_millis(100), |seen| !burst_acks(seen).is_empty())?;
            trial.t3_ack = burst_acks(&acks).first().and_then(|seen| seen.tick);
            trial.reports.extend(acks.into_iter().filter(|seen| seen.code != TxCode::BurstAck));
        }
        trial.b_start = align_to(now()? + 40 * TICKS_PER_MS, per);
        trial.b_end = trial.b_start + n_b as i64 * per;
        let (n, len) = (((rate / 1000.0) as usize).clamp(2_000, 100_000), trial.b_end - trial.b_start);
        device.rx_start(0, trial.b_start - len).map_err(|e| error_text("rx_start", e))?;
        send_burst(device, &b, Some(trial.b_start));
        let got = read_with(device, 0, n, Wall::from_secs(2), Some(per), |r| r.end(per).is_some_and(|end| end >= trial.b_end + len * 3 / 2));
        device.rx_stop(0).map_err(|e| error_text("rx_stop", e))?;
        let _ = drain(device);
        trial.record_capture(&got);
        let reports = collect_reports(device, since, Instant::now() + Wall::from_millis(300), |_| false)?;
        trial.reports.extend(reports);
        Ok(())
    })();
    trial.error = result.err();
    trial
}

/// (a): per rate the trials are interleaved, control, open-then-skip, closed-then-skip, control, ... and a control last, so that every skipping trial lies
/// between two controls: `trials` skipping trials of each kind and `trials + 1` controls. Prints a trace per trial; returns the trials per cell.
pub fn skip_probe(device: &dyn Device, rates: &[f64], trials: usize) -> Vec<(f64, SkipKind, Vec<SkipTrial>)> {
    assert_slot_a_only(d9_channel());
    let hz = pin_slot_a(device);
    let mut out = Vec::new();
    for &rate in rates {
        open_loopback(device, rate, hz, true);
        device.tx_close(0);
        let run = |kind: SkipKind, round: usize| {
            let done = skip_trial(device, kind, rate, round);
            println!("P75 a trace {} #{round} {}", kind.name(), done.trace());
            done
        };
        let mut controls = vec![run(SkipKind::OpenThenFlush, 0)];
        let (mut open, mut closed) = (Vec::new(), Vec::new());
        for round in 0..trials {
            open.push(run(SkipKind::OpenThenSkip, round));
            closed.push(run(SkipKind::ClosedThenSkip, round));
            controls.push(run(SkipKind::OpenThenFlush, round + 1));
        }
        out.extend([(rate, SkipKind::OpenThenFlush, controls), (rate, SkipKind::OpenThenSkip, open), (rate, SkipKind::ClosedThenSkip, closed)]);
    }
    device.tx_close(0);
    out
}

/// Every (a) trial judged against its bracket, and each rate's control stability.
struct SkipJudged<'a> {
    judged: Vec<(f64, &'a SkipTrial, Verdict)>,
    stability: Vec<(f64, Result<(), String>)>,
}

fn judge_skips(cells: &[(f64, SkipKind, Vec<SkipTrial>)]) -> SkipJudged<'_> {
    let mut out = SkipJudged { judged: Vec::new(), stability: Vec::new() };
    for (rate, kind, trials) in cells {
        let controls: &[SkipTrial] = cells.iter().find(|(r, k, _)| r == rate && *k == SkipKind::OpenThenFlush).map_or(&[], |(_, _, controls)| controls);
        let stable = control_stable(controls);
        if *kind == SkipKind::OpenThenFlush {
            out.stability.push((*rate, stable.clone()));
        }
        for trial in trials {
            let reference = (*kind != SkipKind::OpenThenFlush).then(|| stable.clone().and_then(|()| bracket_reference(controls, trial.round)));
            out.judged.push((*rate, trial, skip_trial_verdict(trial, reference)));
        }
    }
    out
}

/// (a)'s verdict lines: every trial against its bracketing controls, and a claim for each rate's control stability; the cell lines are printed.
pub fn skip_report(cells: &[(f64, SkipKind, Vec<SkipTrial>)]) -> Vec<(String, Verdict)> {
    let SkipJudged { judged, stability } = judge_skips(cells);
    for (rate, kind, trials) in cells {
        let mine = judged.iter().filter(|(r, trial, _)| r == rate && trial.kind == *kind).map(|(_, _, verdict)| verdict);
        let offsets: Vec<i64> = trials.iter().filter_map(SkipTrial::offset).collect();
        let acks: Vec<i64> =
            trials.iter().filter_map(|trial| burst_acks(&trial.reports).iter().filter_map(|seen| seen.tick).find(|tick| *tick >= trial.b_start).map(|tick| tick - trial.b_end)).collect();
        println!(
            "P75 a cell {} {} MS/s: {} of {}; onset offset min/med/max {:?}; B ACK minus B end min/med/max {:?}",
            kind.name(),
            rate / 1e6,
            cell_counts(mine),
            trials.len(),
            spread(offsets),
            spread(acks)
        );
    }
    let mut verdicts: Vec<(String, Verdict)> = judged.into_iter().map(|(rate, trial, verdict)| (format!("n75 a {} {} MS/s trial {}", trial.kind.name(), rate / 1e6, trial.round), verdict)).collect();
    for (rate, stable) in &stability {
        let text = match stable {
            Ok(()) => format!("the control's onset offsets spread at most {CONTROL_SPREAD_TICKS} ticks"),
            Err(why) => why.clone(),
        };
        println!("P75 a control stability {} MS/s: {text}", rate / 1e6);
        verdicts.push((format!("n75 a control stability {} MS/s", rate / 1e6), if stable.is_ok() { Verdict::Pass(text) } else { Verdict::Inconclusive(text) }));
    }
    verdicts
}

/// What (a) decides for #75's (3), from the trials against their bracketing controls, over the planned cells (`rates`, `trials` skipping trials of each
/// kind and `trials + 1` controls each): INCONCLUSIVE unless the evidence is complete, the controls clean and stable and no call failed.
pub fn skip_decision(cells: &[(f64, SkipKind, Vec<SkipTrial>)], rates: &[f64], trials: usize) -> String {
    let planned = |rate: f64, kind: SkipKind| cells.iter().find(|(r, k, _)| *r == rate && *k == kind).map_or(0, |(_, _, found)| found.len());
    let incomplete: Vec<String> = rates
        .iter()
        .flat_map(|&rate| [SkipKind::OpenThenFlush, SkipKind::OpenThenSkip, SkipKind::ClosedThenSkip].map(move |kind| (rate, kind)))
        .filter(|&(rate, kind)| planned(rate, kind) != trials + usize::from(kind == SkipKind::OpenThenFlush))
        .map(|(rate, kind)| format!("{} {} MS/s has {} of {} trials", kind.name(), rate / 1e6, planned(rate, kind), trials + usize::from(kind == SkipKind::OpenThenFlush)))
        .collect();
    if !incomplete.is_empty() {
        return format!("P75 a decision: INCONCLUSIVE: the planned cells are incomplete ({})", incomplete.join("; "));
    }
    let SkipJudged { judged, stability } = judge_skips(cells);
    let verdict_of = |trial: &SkipTrial| judged.iter().find(|(_, other, _)| std::ptr::eq(*other, trial)).map(|(_, _, verdict)| verdict).expect("judged");
    let unstable: Vec<String> = stability.iter().filter_map(|(rate, stable)| stable.as_ref().err().map(|why| format!("{} MS/s: {why}", rate / 1e6))).collect();
    let all = || cells.iter().flat_map(|(_, kind, trials)| trials.iter().map(move |trial| (*kind, trial)));
    if all().any(|(kind, trial)| kind == SkipKind::OpenThenFlush && !matches!(verdict_of(trial), Verdict::Pass(_))) {
        "P75 a decision: INCONCLUSIVE: the control (T3 sent) is not clean in every trial".to_owned()
    } else if !unstable.is_empty() {
        format!("P75 a decision: INCONCLUSIVE: the control is not stable, so no difference can be attributed to T3 ({})", unstable.join("; "))
    } else if all().any(|(_, trial)| trial.error.is_some()) {
        format!("P75 a decision: INCONCLUSIVE: a trial had a failed device call ({TRANSPORT}), which says nothing about T3")
    } else {
        let failed = |kind: SkipKind| all().any(|(k, trial)| k == kind && matches!(verdict_of(trial), Verdict::Fail(_)));
        let clean = |kind: SkipKind| all().filter(|(k, _)| *k == kind).all(|(_, trial)| matches!(verdict_of(trial), Verdict::Pass(_)));
        match (failed(SkipKind::OpenThenSkip), failed(SkipKind::ClosedThenSkip)) {
            (true, true) => "P75 a decision: both skipping cells differ from their stable bracketing controls (see the FAIL lines): skipping T3 changes the next burst, also after a closed burst: T3 is needed, the retry rule stands".to_owned(),
            (true, false) => "P75 a decision: skipping T3 after an unclosed burst changes what the next burst does against the stable bracketing controls (see the FAIL lines): T3 is needed, the retry rule stands".to_owned(),
            (false, true) => "P75 a decision: skipping T3 changes the next burst even after a closed burst, so an unclosed burst is not the cause (see the FAIL lines): not attributable to #75's hazard, read the lines".to_owned(),
            (false, false) if clean(SkipKind::OpenThenSkip) && clean(SkipKind::ClosedThenSkip) => {
                "P75 a decision: skipping T3 left the next burst intact in every trial at these rates against a stable control: the hazard is not shown here (it stays INFERRED)".to_owned()
            }
            (false, false) => "P75 a decision: INCONCLUSIVE: a skipping cell has an INCONCLUSIVE trial".to_owned(),
        }
    }
}

// ---- (b) an old burst's ACK on the new streamer; (c) the T3 ACK read late

/// When burst A of (b) ends against the reopen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AEnds {
    /// A has played and its ACK is unread when the streamer is closed.
    BeforeClose,
    /// A is still waiting for its start when the streamer is closed and reopened and T3 is sent.
    AfterReopen,
}

impl AEnds {
    pub fn name(self) -> &'static str {
        match self {
            AEnds::BeforeClose => "a_ends_before_close",
            AEnds::AfterReopen => "a_ends_after_reopen",
        }
    }
}

/// The prefix of a (b) FAIL that observed an ACK at A's end on the new streamer.
pub const AT_A_END: &str = "ACK at A's end";

pub const ROUTING_RATES: [f64; 2] = [1e6, 10e6];

/// One trial of (b). `reports` are everything read on the new streamer from T3's send until 150 ms past A's end (read continuously: (c) is the late read).
#[derive(Debug, Clone)]
pub struct RoutingTrial {
    pub when: AEnds,
    pub rate: f64,
    pub per: i64,
    pub a_end: i64,
    pub closed_at: i64,
    pub reopened_at: i64,
    pub t3_at: i64,
    pub t3_sent: Option<usize>,
    pub reports: Vec<Seen>,
    pub error: Option<String>,
}

impl RoutingTrial {
    pub fn trace(&self) -> Json {
        json!({
            "when": self.when.name(), "rate": self.rate, "a_end": self.a_end, "closed_at": self.closed_at, "reopened_at": self.reopened_at, "t3_at": self.t3_at, "t3_sent": self.t3_sent, "ack_minus_a_end": burst_acks(&self.reports).iter().filter_map(|s| s.tick).map(|t| t - self.a_end).collect::<Vec<_>>(), "ack_minus_t3": burst_acks(&self.reports).iter().filter_map(|s| s.tick).map(|t| t - self.t3_at).collect::<Vec<_>>(),
            "reports": self.reports.iter().map(Seen::json).collect::<Vec<_>>(), "acks_by_position": acks_by_position(&self.reports), "other": other_codes(&self.reports), "error": self.error,
        })
    }
}

/// (b)'s verdict. FAIL: an old burst's ACK on the new streamer (an ACK at A's end, or more than one ACK for the one T3), or a failed call (`TRANSPORT`).
/// INCONCLUSIVE: T3 sent short, no ACK at all, or one ACK that cannot be told from A's or from T3's by its tick (none, or before T3 was sent and not at
/// A's end). PASS: exactly one ACK, at or after T3's send and not at A's end: no foreign ACK observed (not a proof that ACKs belong to their streamer).
pub fn routing_verdict(trial: &RoutingTrial) -> Verdict {
    if let Some(error) = &trial.error {
        return Verdict::Fail(format!("{TRANSPORT}: {error}"));
    }
    if trial.t3_sent != Some(4) {
        return Verdict::Inconclusive(format!("T3 sent {:?} samples of 4", trial.t3_sent));
    }
    let acks = burst_acks(&trial.reports);
    if acks.iter().any(|seen| seen.tick.is_none()) {
        return Verdict::Inconclusive("an ACK without a tick cannot be told from A's or from T3's".to_owned());
    }
    let ticks: Vec<i64> = acks.iter().filter_map(|seen| seen.tick).collect();
    let at_a = ticks.iter().filter(|tick| n47::burst_ack_holds(**tick, trial.a_end, trial.per)).count();
    match (ticks.as_slice(), at_a) {
        ([], _) => Verdict::Inconclusive("no BurstAck on the new streamer in the window: T3's own was not seen either".to_owned()),
        ([tick], 1) => Verdict::Fail(format!("{AT_A_END}: the only ACK on the new streamer is at A's end {} (tick {tick}): T3's own is missing", trial.a_end)),
        ([tick], _) if *tick >= trial.t3_at => {
            Verdict::Pass(format!("one ACK on the new streamer (tick {tick}), at or after T3's send {} and not at A's end {}: no foreign ACK observed", trial.t3_at, trial.a_end))
        }
        ([tick], _) => Verdict::Inconclusive(format!("the one ACK (tick {tick}) is before T3's send {} and not at A's end {}: it cannot be attributed", trial.t3_at, trial.a_end)),
        (many, 0) => Verdict::Fail(format!("{} ACKs on the new streamer {many:?} for one T3 (A ends at {})", many.len(), trial.a_end)),
        (many, _) => Verdict::Fail(format!("{AT_A_END}: {} ACKs on the new streamer {many:?} for one T3 (A ends at {}): an old burst's ACK reached it", many.len(), trial.a_end)),
    }
}

fn routing_trial(device: &dyn Device, when: AEnds, rate: f64) -> RoutingTrial {
    let per = (MCR as f64 / rate) as i64;
    let mut trial = RoutingTrial { when, rate, per, a_end: 0, closed_at: 0, reopened_at: 0, t3_at: 0, t3_sent: None, reports: Vec::new(), error: None };
    let result = (|| -> Result<(), String> {
        let now = || device.time_now().map_err(|e| error_text("time_now", e));
        let a = iq(&tone_wave(rate, b_samples(rate), 100e3));
        device.tx_open(0, &[d9_channel()]).map_err(|e| error_text("tx_open", e))?;
        let lead = if when == AEnds::BeforeClose { 20 } else { 120 };
        let a_start = align_to(now()? + lead * TICKS_PER_MS, per);
        trial.a_end = a_start + a.len() as i64 * per;
        send_burst(device, &a, Some(a_start));
        if when == AEnds::BeforeClose {
            wait_device_time(device, trial.a_end + 5 * TICKS_PER_MS);
        }
        device.tx_close(0);
        trial.closed_at = now()?;
        device.tx_open(0, &[d9_channel()]).map_err(|e| error_text("tx_open (reopen)", e))?;
        trial.reopened_at = now()?;
        let since = Instant::now();
        trial.t3_at = now()?;
        trial.t3_sent = Some(send_t3(device, 1, Wall::from_millis(100))?);
        let remaining = (trial.a_end + 150 * TICKS_PER_MS - now()?).max(0);
        trial.reports = collect_reports(device, since, Instant::now() + Wall::from_micros(remaining as u64 / 200), |_| false)?;
        Ok(())
    })();
    trial.error = result.err();
    trial
}

/// One trial of (c): T3 sent, the ACK read after `LATE_MS`.
#[derive(Debug, Clone)]
pub struct LateTrial {
    pub rate: f64,
    pub t3_sent: Option<usize>,
    pub t3_at_device: i64,
    pub reports: Vec<Seen>,
    pub error: Option<String>,
}

pub const LATE_MS: u64 = 150;

impl LateTrial {
    pub fn trace(&self) -> Json {
        json!({
            "rate": self.rate, "t3_sent": self.t3_sent, "t3_at_device": self.t3_at_device, "late_ms": LATE_MS, "reports": self.reports.iter().map(Seen::json).collect::<Vec<_>>(),
            "acks_by_position": acks_by_position(&self.reports), "ack_minus_t3": burst_acks(&self.reports).first().and_then(|s| s.tick).map(|tick| tick - self.t3_at_device), "other": other_codes(&self.reports), "error": self.error,
        })
    }
}

/// (c)'s verdict. PASS: one ACK, read after 150 ms: retention demonstrated. FAIL: more than one ACK for the one T3, or a failed call (`TRANSPORT`).
/// INCONCLUSIVE: T3 sent short, an ACK without a tick or older than the send, or no ACK read: it was never generated, not seen, or expired, and absence
/// alone does not say which.
pub fn late_verdict(trial: &LateTrial) -> Verdict {
    if let Some(error) = &trial.error {
        return Verdict::Fail(format!("{TRANSPORT}: {error}"));
    }
    if trial.t3_sent != Some(4) {
        return Verdict::Inconclusive(format!("T3 sent {:?} samples of 4", trial.t3_sent));
    }
    if let Some(why) = unattributable(&trial.reports, trial.t3_at_device) {
        return Verdict::Inconclusive(why);
    }
    match burst_acks(&trial.reports).as_slice() {
        [] => Verdict::Inconclusive(format!("no BurstAck read {LATE_MS} ms after T3 (read for 100 ms): never generated, not seen or expired; absence alone does not prove expiry")),
        [ack] => Verdict::Pass(format!("the ACK (tick {:?}, read at {} ms) was still there {LATE_MS} ms after T3: retained", ack.tick, ack.at_ms)),
        many => Verdict::Fail(format!("{} ACKs for one T3", many.len())),
    }
}

fn late_trial(device: &dyn Device, rate: f64) -> LateTrial {
    let mut trial = LateTrial { rate, t3_sent: None, t3_at_device: 0, reports: Vec::new(), error: None };
    let result = (|| -> Result<(), String> {
        device.tx_open(0, &[d9_channel()]).map_err(|e| error_text("tx_open", e))?;
        let since = Instant::now();
        trial.t3_at_device = device.time_now().map_err(|e| error_text("time_now", e))?;
        trial.t3_sent = Some(send_t3(device, 1, Wall::from_millis(100))?);
        std::thread::sleep(Wall::from_millis(LATE_MS));
        trial.reports = collect_reports(device, since, Instant::now() + Wall::from_millis(100), |seen| !burst_acks(seen).is_empty())?;
        device.tx_close(0);
        Ok(())
    })();
    trial.error = result.err();
    trial
}

/// (b) and (c) on `rates`: per rate, (b)'s two cells then (c)'s, `trials` each; every trial prints a trace; returns the `(claim, verdict)` lines, cell lines printed.
pub fn routing_and_late_probe(device: &dyn Device, rates: &[f64], trials: usize) -> Vec<(String, Verdict)> {
    assert_slot_a_only(d9_channel());
    let hz = pin_slot_a(device);
    let mut verdicts = Vec::new();
    for &rate in rates {
        open_loopback(device, rate, hz, false);
        device.tx_close(0);
        for when in [AEnds::BeforeClose, AEnds::AfterReopen] {
            let judged: Vec<(Verdict, RoutingTrial)> = (0..trials)
                .map(|index| {
                    let done = routing_trial(device, when, rate);
                    println!("P75 b trace {} {} MS/s #{index} {}", when.name(), rate / 1e6, done.trace());
                    device.tx_close(0);
                    (routing_verdict(&done), done)
                })
                .collect();
            println!("P75 b cell {} {} MS/s: {}", when.name(), rate / 1e6, cell_counts(judged.iter().map(|(v, _)| v)));
            verdicts.extend(judged.into_iter().enumerate().map(|(index, (verdict, _))| (format!("n75 b {} {} MS/s trial {index}", when.name(), rate / 1e6), verdict)));
        }
        let judged: Vec<(Verdict, LateTrial)> = (0..trials)
            .map(|index| {
                let done = late_trial(device, rate);
                println!("P75 c trace {} MS/s #{index} {}", rate / 1e6, done.trace());
                (late_verdict(&done), done)
            })
            .collect();
        println!("P75 c cell {} MS/s: {}", rate / 1e6, cell_counts(judged.iter().map(|(v, _)| v)));
        verdicts.extend(judged.into_iter().enumerate().map(|(index, (verdict, _))| (format!("n75 c {} MS/s trial {index}", rate / 1e6), verdict)));
    }
    device.tx_close(0);
    verdicts
}

/// `"n PASS, n FAIL, n INCONCLUSIVE of n"`.
pub fn cell_counts<'a>(verdicts: impl Iterator<Item = &'a Verdict>) -> String {
    let all: Vec<&Verdict> = verdicts.collect();
    let count = |f: fn(&Verdict) -> bool| all.iter().filter(|v| f(v)).count();
    format!("{} PASS, {} FAIL, {} INCONCLUSIVE of {}", count(|v| matches!(v, Verdict::Pass(_))), count(|v| matches!(v, Verdict::Fail(_))), count(|v| matches!(v, Verdict::Inconclusive(_))), all.len())
}

/// What (b) and (c) decide for the recovery's ACK handling: from their claims' verdicts (`n75 b ...`, `n75 c ...`) over the planned trials (`rates`,
/// (b)'s two cells and (c)'s one, `trials` each). INCONCLUSIVE unless every planned trial has a verdict, none is a failed call and none is without one.
pub fn routing_decision(verdicts: &[(String, Verdict)], rates: &[f64], trials: usize) -> Vec<String> {
    let of = |prefix: &str| verdicts.iter().filter(|(claim, _)| claim.starts_with(prefix)).map(|(_, verdict)| verdict).collect::<Vec<_>>();
    let line = |name: &str, prefix: &str, planned: usize, all_pass: &str, any_fail: &str, none_pass: &str| {
        let group = of(prefix);
        let count = |f: fn(&Verdict) -> bool| group.iter().filter(|v| f(v)).count();
        let transport = count(|v| matches!(v, Verdict::Fail(text) if text.starts_with(TRANSPORT)));
        let text = if group.len() != planned {
            format!("INCONCLUSIVE: {} of {planned} planned trials have a verdict", group.len())
        } else if transport > 0 {
            format!("INCONCLUSIVE: {transport} trial(s) had a failed device call ({TRANSPORT}), which says nothing about the ACKs")
        } else if group.iter().any(|v| matches!(v, Verdict::Fail(_))) {
            any_fail.to_owned()
        } else if count(|v| matches!(v, Verdict::Pass(_))) == planned {
            all_pass.to_owned()
        } else {
            format!(
                "INCONCLUSIVE: {} of {planned} trials passed, {} without a verdict (read the INCONCLUSIVE lines): {none_pass}",
                count(|v| matches!(v, Verdict::Pass(_))),
                count(|v| matches!(v, Verdict::Inconclusive(_)))
            )
        };
        format!("P75 {name} decision: {text}")
    };
    let at_a_end = of("n75 b").iter().any(|v| matches!(v, Verdict::Fail(text) if text.starts_with(AT_A_END)));
    let b_failed = if at_a_end {
        "an ACK at an old burst's end shows on the new streamer: F1's routing is real; the flush must match its ACK by tick, not by count"
    } else {
        "a trial FAILed on the new streamer without an ACK at A's end (more than one ACK for one T3, with none at A's end): no cause is observed; read the FAIL lines"
    };
    vec![
        line(
            "b",
            "n75 b",
            rates.len() * 2 * trials,
            "no foreign ACK observed on the new streamer in any trial: F1's ACK-routing hazard is not shown at these rates and A endings (this does not prove that ACKs belong to their streamer: UHD posts them on the radio's edge)",
            b_failed,
            "no recommendation",
        ),
        line(
            "c",
            "n75 c",
            rates.len() * trials,
            "retention demonstrated: the T3 ACK was still readable 150 ms after T3 in every trial: a missed 100 ms window is not a lost report at these rates; the retry's wait may extend",
            "more than one ACK for one T3: read the FAIL lines",
            "an ACK that is not read late was never generated, not seen or expired and absence alone does not prove expiry: no recommendation about waiting longer",
        ),
    ]
}

// ============================================================================================================================
// Probe 3: the open-flush sweep (`hw_open_flush_sweep`): the T3 at `arm` (`core.rs` `open_tx_device_locked`), 1, 2, 10 MS/s x one and two channels.
// ============================================================================================================================

pub const FLUSH_RATES: [f64; 3] = [1e6, 2e6, 10e6];
pub const FLUSH_TRIALS: usize = 10;

/// The environment variable that waives the two-channel cells of the decision: set to `1` only on the owner's word (he gated (3) on the one- and
/// two-channel sweep).
pub const TWO_CHANNEL_WAIVER: &str = "EZSDR_OFS_TWO_CHANNEL_WAIVED";

/// The environment variable that lets the two-channel cells transmit on slot B: set to `1` only after the owner confirmed, for the board now in slot B,
/// TX/RX → attenuator (30 dB or more, 18 GHz rated) → the same board's RX2 (a board swap leaves slot B without that check).
pub const SLOT_B_CONFIRMED: &str = "EZSDR_SLOT_B_ATTENUATOR_CONFIRMED";

/// The claim of one flush trial (`index` Some) or of a two-channel cell that cannot run (`index` None, `positions` 2).
pub fn flush_claim(rate: f64, positions: usize, index: Option<usize>) -> String {
    match index {
        Some(index) => format!("n75 ofs {} MS/s x {positions} channel(s) trial {index}", rate / 1e6),
        None => format!("n75 ofs {} MS/s x {positions} channel(s) not runnable", rate / 1e6),
    }
}

fn not_runnable_claim(claim: &str) -> bool {
    claim.ends_with("not runnable")
}

/// The Module's flush deadline: one 100 ms bound over the send and the ACK reads.
pub const FLUSH_DEADLINE: Wall = Wall::from_millis(100);

/// How long after the deadline a missing ACK is still waited for (to tell a late ACK from none).
pub const FLUSH_LATE_WAIT: Wall = Wall::from_millis(400);

/// Whether the device can open a two-channel transmit streamer within the bench's RF bounds, from the front end names of its first two transmit
/// channels (`None`: no such channel). Both must be an OBX or UBX: they tune to 1 GHz (`SAFE_TX_HZ`) and a CBX (1.2 GHz and above) cannot.
pub fn two_channel_runnable(front_ends: &[Option<String>]) -> Result<(), String> {
    let Some(first) = front_ends.first().and_then(Option::as_deref) else {
        return Err("the device has no transmit channel".to_owned());
    };
    let Some(second) = front_ends.get(1).and_then(Option::as_deref) else {
        return Err(format!("a one-channel device ({first:?} only)"));
    };
    for (position, name) in [first, second].into_iter().enumerate() {
        match Board::of_front_end(name) {
            Some(Board::Ubx | Board::Obx) => {}
            Some(Board::Cbx) => return Err(format!("channel {position} is a {name:?}: it tunes from 1.2 GHz, outside SAFE_TX_HZ (999.5-1000.5 MHz)")),
            None => return Err(format!("channel {position} is {name:?}, not an OBX or UBX (an empty slot reads as an unknown board)")),
        }
    }
    Ok(())
}

/// `two_channel_runnable`, and slot B's attenuator confirmed by the owner (`SLOT_B_CONFIRMED`): the two-channel cells transmit on both slots.
pub fn two_channel_gate(front_ends: &[Option<String>], slot_b_confirmed: bool) -> Result<(), String> {
    two_channel_runnable(front_ends)?;
    if slot_b_confirmed { Ok(()) } else { Err(format!("slot B's attenuator is not confirmed: set {SLOT_B_CONFIRMED}=1 only after the owner confirmed TX/RX -> attenuator -> RX2 on slot B's board")) }
}

/// One open-flush trial: the open's and the flush's wall time, the T3 send, every report with its wall time since the flush began, the positions acknowledged
/// within the deadline and the ones that came late.
#[derive(Debug, Clone)]
pub struct FlushTrial {
    pub positions: usize,
    pub open_ms: f64,
    pub flush_ms: f64,
    pub sent: Option<usize>,
    pub sent_at_device: i64,
    pub reports: Vec<Seen>,
    pub within: Vec<usize>,
    pub late: Vec<usize>,
    pub error: Option<String>,
}

impl FlushTrial {
    pub fn ack_ticks(&self) -> Vec<(usize, i64)> {
        burst_acks(&self.reports).iter().filter_map(|seen| seen.tick.map(|tick| (seen.channel, tick))).collect()
    }

    pub fn trace(&self) -> Json {
        json!({
            "positions": self.positions, "open_ms": self.open_ms, "flush_ms": self.flush_ms, "sent": self.sent, "sent_at_device": self.sent_at_device,
            "acked_within_100ms": self.within, "acked_late": self.late, "ack_ticks": self.ack_ticks(), "ack_minus_send": self.ack_ticks().iter().map(|(c, t)| (*c, t - self.sent_at_device)).collect::<Vec<_>>(),
            "reports": self.reports.iter().map(Seen::json).collect::<Vec<_>>(), "other": other_codes(&self.reports), "error": self.error,
        })
    }
}

/// PASS: T3 sent whole (4 of 4) and every position acknowledged within the 100 ms deadline. FAIL: a failed call, a short send, or a position with no ACK
/// by the deadline (the text says which came late and which never). INCONCLUSIVE: an ACK without a tick or older than the send (not this flush's).
pub fn flush_trial_verdict(trial: &FlushTrial) -> Verdict {
    if let Some(error) = &trial.error {
        return Verdict::Fail(error.clone());
    }
    if trial.sent != Some(4) {
        return Verdict::Fail(format!("the flush send accepted {:?} of 4 samples", trial.sent));
    }
    if let Some(why) = unattributable(&trial.reports, trial.sent_at_device) {
        return Verdict::Inconclusive(why);
    }
    let missing: Vec<usize> = (0..trial.positions).filter(|position| !trial.within.contains(position)).collect();
    if missing.is_empty() {
        Verdict::Pass(format!("every position acknowledged; open {:.1} ms, flush {:.1} ms", trial.open_ms, trial.flush_ms))
    } else {
        let late: Vec<usize> = missing.iter().copied().filter(|position| trial.late.contains(position)).collect();
        Verdict::Fail(format!("BurstAck missing on positions {missing:?} before the 100 ms deadline ({late:?} of them came late, within {} ms more)", FLUSH_LATE_WAIT.as_millis()))
    }
}

fn open_flush_trial(device: &dyn Device, channels: &[usize]) -> FlushTrial {
    let positions = channels.len();
    let mut trial = FlushTrial { positions, open_ms: 0.0, flush_ms: 0.0, sent: None, sent_at_device: 0, reports: Vec::new(), within: Vec::new(), late: Vec::new(), error: None };
    let result = (|| -> Result<(), String> {
        device.tx_close(0);
        let begun = Instant::now();
        device.tx_open(0, channels).map_err(|e| error_text("tx_open", e))?;
        trial.open_ms = begun.elapsed().as_secs_f64() * 1e3;
        let flush = Instant::now();
        let until = flush + FLUSH_DEADLINE;
        trial.sent_at_device = device.time_now().map_err(|e| error_text("time_now", e))?;
        trial.sent = Some(send_t3(device, positions, until.saturating_duration_since(Instant::now()))?);
        let all = |seen: &[Seen]| (0..positions).all(|position| seen.iter().any(|s| s.code == TxCode::BurstAck && s.channel == position));
        trial.reports = collect_reports(device, flush, until, all)?;
        trial.flush_ms = flush.elapsed().as_secs_f64() * 1e3;
        trial.within = (0..positions).filter(|position| trial.reports.iter().any(|s| s.code == TxCode::BurstAck && s.channel == *position)).collect();
        if trial.within.len() < positions {
            let more = collect_reports(device, flush, Instant::now() + FLUSH_LATE_WAIT, all)?;
            trial.reports.extend(more);
            trial.late = (0..positions).filter(|position| !trial.within.contains(position) && trial.reports.iter().any(|s| s.code == TxCode::BurstAck && s.channel == *position)).collect();
        }
        Ok(())
    })();
    trial.error = result.err();
    trial
}

/// One cell's summary: the ACK counts, the ACK ticks relative to the send, the open's and the flush's wall time.
pub fn flush_cell_line(rate: f64, positions: usize, trials: &[FlushTrial]) -> String {
    let all_acked = trials.iter().filter(|trial| trial.error.is_none() && trial.sent == Some(4) && trial.within.len() == positions).count();
    let ack_offsets: Vec<i64> = trials.iter().flat_map(|trial| trial.ack_ticks().into_iter().map(move |(_, tick)| tick - trial.sent_at_device)).collect();
    let (open, flush): (Vec<u64>, Vec<u64>) = (trials.iter().map(|t| (t.open_ms * 1e3) as u64).collect(), trials.iter().map(|t| (t.flush_ms * 1e3) as u64).collect());
    let ms = |spread: Option<(u64, u64, u64)>| spread.map(|(a, b, c)| (a as f64 / 1e3, b as f64 / 1e3, c as f64 / 1e3));
    format!(
        "OFS cell {} MS/s x {positions} channel(s): all positions ACKed {all_acked} of {}; ACK tick minus device time at the send min/med/max {:?}; open ms {:?}; flush ms {:?}",
        rate / 1e6,
        trials.len(),
        spread(ack_offsets),
        ms(spread(open)),
        ms(spread(flush))
    )
}

/// The sweep: every rate x one channel, then x two channels where the device can and slot B's attenuator is confirmed (`two_channel_gate`), `trials` each. A
/// two-channel cell that cannot run prints `OFS cell ... NOT RUNNABLE: <reason>`, transmits nothing on slot B and returns an INCONCLUSIVE claim.
pub fn open_flush_sweep(device: &dyn Device, rates: &[f64], trials: usize, slot_b_confirmed: bool) -> Vec<(String, Verdict)> {
    assert_slot_a_only(d9_channel());
    let hz = pin_slot_a(device);
    let front_ends: Vec<Option<String>> = (0..device.channels(Dir::Tx).min(2)).map(|channel| device.front_end(Dir::Tx, channel).ok()).collect();
    let two = two_channel_gate(&front_ends, slot_b_confirmed);
    let mut verdicts = Vec::new();
    for positions in [1usize, 2] {
        for &rate in rates {
            if positions == 2 {
                if let Err(reason) = &two {
                    println!("OFS cell {} MS/s x 2 channel(s): NOT RUNNABLE: {reason}", rate / 1e6);
                    verdicts.push((flush_claim(rate, positions, None), Verdict::Inconclusive(format!("not runnable: {reason}"))));
                    continue;
                }
            }
            let channels: Vec<usize> = if positions == 1 { vec![d9_channel()] } else { vec![0, 1] };
            device.tx_close(0);
            for &channel in &channels {
                let settings = Settings { rate: Some(rate), freq: Some(hz), gain: Some(0.0), antenna: Some("TX/RX".to_owned()) };
                assert!(SAFE_TX_HZ.contains(&hz) && rate <= SAFE_TX_MAX_RATE, "bench.md RF safety: {hz} Hz at {rate} S/s");
                let applied = device.apply(Dir::Tx, channel, &settings).unwrap();
                assert!(applied.gain == 0.0 && SAFE_TX_HZ.contains(&applied.freq), "{applied:?}");
            }
            let cell: Vec<FlushTrial> = (0..trials)
                .map(|index| {
                    let done = open_flush_trial(device, &channels);
                    println!("OFS trace {} MS/s x {positions} #{index} {}", rate / 1e6, done.trace());
                    done
                })
                .collect();
            println!("{}", flush_cell_line(rate, positions, &cell));
            verdicts.extend(cell.iter().enumerate().map(|(index, trial)| (flush_claim(rate, positions, Some(index)), flush_trial_verdict(trial))));
        }
    }
    device.tx_close(0);
    verdicts
}

/// What the open-flush sweep decides for #75's (3), from its claims' verdicts over the planned cells (`rates` x one and two channels, `trials` each). A miss
/// stops it ("ask the owner again"). Without one: INCONCLUSIVE while a planned runnable trial has no verdict, and while a two-channel cell is NOT RUNNABLE
/// unless the owner `waived` it (he gated (3) on the one- and two-channel cells); else enabled.
pub fn open_flush_decision(verdicts: &[(String, Verdict)], rates: &[f64], trials: usize, waived: bool) -> String {
    let has = |claim: &str| verdicts.iter().any(|(found, _)| found == claim);
    let not_runnable: Vec<String> = rates.iter().filter(|&&rate| has(&flush_claim(rate, 2, None))).map(|&rate| format!("{} MS/s x 2 channels", rate / 1e6)).collect();
    let absent: Vec<String> = [1usize, 2]
        .iter()
        .flat_map(|&positions| rates.iter().map(move |&rate| (positions, rate)))
        .filter(|&(positions, rate)| !(positions == 2 && has(&flush_claim(rate, 2, None))) && (0..trials).any(|index| !has(&flush_claim(rate, positions, Some(index)))))
        .map(|(positions, rate)| format!("{} MS/s x {positions} channel(s)", rate / 1e6))
        .collect();
    let unclear: Vec<&str> = verdicts.iter().filter(|(claim, verdict)| matches!(verdict, Verdict::Inconclusive(_)) && !not_runnable_claim(claim)).map(|(claim, _)| claim.as_str()).collect();
    if verdicts.iter().any(|(_, verdict)| matches!(verdict, Verdict::Fail(_))) {
        "OFS decision: an open-time flush went unacknowledged within 100 ms in a trial (or a call failed): ask the owner again before (3) is enabled".to_owned()
    } else if !absent.is_empty() {
        format!("OFS decision: INCONCLUSIVE: planned cells without all their trials: {}", absent.join("; "))
    } else if !unclear.is_empty() {
        format!("OFS decision: INCONCLUSIVE: runnable trial(s) without a PASS or FAIL (read the INCONCLUSIVE lines): {}", unclear.join("; "))
    } else if !not_runnable.is_empty() && !waived {
        format!(
            "OFS decision: INCONCLUSIVE: the two-channel cell(s) {} are NOT RUNNABLE and the owner gated (3) on the one- and two-channel cells; set {TWO_CHANNEL_WAIVER}=1 only on the owner's waiver",
            not_runnable.join(", ")
        )
    } else {
        let waiver = if not_runnable.is_empty() { String::new() } else { format!(" (the owner waived the not-runnable two-channel cell(s) {})", not_runnable.join(", ")) };
        format!("OFS decision: every planned cell's flushes were acknowledged within 100 ms in every trial: enable (3){waiver}, with the cells' conditions (rates, channel counts, boards) stated")
    }
}

// ============================================================================================================================
// Probe 4: the PN cross-correlation cell for the receive-restart zeros (`hw_rxzeros_pn`). Brief: ~/.cache/ezsdr-fixes/2026-11-13-bench1115b/brief.md §2.
// ============================================================================================================================

/// The decision rule, printed before the first trial and again with the decisions.
pub const PNZ_RULE: &str = "RULE (written before the data): z = the leading exact zeros of a restart. d_cont and d_restart = the delay in receive samples of the transmitted PN, found by cross-correlation \
(1 024-sample reference, +-2 048 samples searched) in a continuous capture of one PN period taken after 1 s of settled stream and in the restart's samples after the zeros; delta = d_restart - d_cont, tolerance +-1 sample. \
res_gd and res_shift = the relative rms error of the first 256 post-zero samples against the continuous capture (one whole PN period, the same channel and filters) at the same PN phase and at the phase z samples earlier, each with its own best complex gain. \
z < 3: INSEPARABLE (the two readings differ by no more than the tolerance). |delta| <= 1 and res_gd <= 0.35: GROUP DELAY (the zeros replace samples that would have been there: no time shift). \
|delta - z| <= 1: SHIFTED (the signal arrives z samples late). Otherwise NEITHER. A rate is decided only when all 10 trials are complete and agree. \
A FAIL is infrastructure only (a failed call, a first block off its tick, a complete capture without a correlation peak); the outcome itself is a fact, not a defect.";

/// The PN's period in samples: the correlation is unique within half of it.
pub const PN_PERIOD: usize = 16_384;
/// The reference's length in samples.
pub const PN_REF: usize = 1_024;
/// How far either side of the expected position the correlation looks, in samples (below half the period).
pub const PN_RADIUS: i64 = 2_048;
/// The samples of each capture (the restart's, and the continuous capture's correlation): the search range (2 * radius) and a reference, with room for the zeros.
pub const PN_SPAN: usize = 6_000;
/// The continuous capture's samples: a whole PN period, so that every PN phase of the restart has its continuous sample.
pub const PN_CONT_SPAN: usize = PN_PERIOD;
/// The receive stream's settled time before the continuous capture (brief: at least 1 s).
pub const PN_SETTLE_MS: u64 = 1_000;
/// The tolerance on the alignment, in samples.
pub const PN_TOL: i64 = 1;
/// The fewest zeros that tell the two readings apart: more than twice the tolerance.
pub const PN_MIN_ZEROS: usize = 2 * PN_TOL as usize + 1;
/// The largest relative residual of the post-zero samples against the continuous prediction that still matches it.
pub const PN_RES_MATCH: f64 = 0.35;
/// The post-zero samples the residual is taken over.
pub const PN_RES_SAMPLES: usize = 256;
pub const PN_TRIALS: usize = 10;
/// The decimations of the master clock for the 8 rates of `hw_iii_raw_rx_restart_fill` (1, 2, 5, 10, 25, 50, 100, 200 MS/s), slowest first.
pub const PN_DECIMATIONS: [u64; 8] = [200, 100, 40, 20, 8, 4, 2, 1];
const PN_CHUNK: usize = 4 * PN_PERIOD;
const _: () = assert!(PN_SPAN >= 2 * PN_RADIUS as usize + PN_REF + 512 && (PN_RADIUS as usize) * 2 < PN_PERIOD && PN_REF < PN_PERIOD);

/// One rate of the cell: transmit and receive at the same rate (a sample of the PN is a sample of the capture, so a one-sample shift shows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PnCell {
    pub decim: u64,
    pub trials: usize,
    pub settle_ms: u64,
}

impl PnCell {
    pub fn rate(&self) -> f64 {
        MCR as f64 / self.decim as f64
    }

    pub fn name(&self) -> String {
        format!("D{} ({:.3} MS/s)", self.decim, self.rate() / 1e6)
    }
}

pub fn pn_cells() -> Vec<PnCell> {
    PN_DECIMATIONS.iter().map(|&decim| PnCell { decim, trials: PN_TRIALS, settle_ms: PN_SETTLE_MS }).collect()
}

/// The PN, one period: the bench's QPSK pseudo-noise at 0.7 of its amplitude (magnitude 0.4, inside the 0.5 bound).
pub fn pn_period() -> Vec<Iq> {
    super::pn(PN_PERIOD).into_iter().map(|(re, im)| [re * 0.7, im * 0.7]).collect()
}

fn pn_at(period: &[Iq], index: i64) -> Iq {
    period[index.rem_euclid(period.len() as i64) as usize]
}

/// What the correlation of one capture found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PnCorr {
    pub status: observations::CorrelationStatus,
    /// The delay in samples from the PN's transmit tick (`t0 + n * per` for PN sample `n`), when the peak is found.
    pub d: Option<i64>,
    /// The peak over the median of the searched correlations.
    pub ratio: f64,
}

/// The PN's delay in `samples` (a capture at `per` ticks a sample), the PN's sample `n` having been transmitted at `t0 + n * per`: a reference
/// segment is correlated over the first `2 * PN_RADIUS` start positions of the capture.
pub fn locate_pn(samples: &[(i64, Iq)], period: &[Iq], t0: i64, per: i64) -> PnCorr {
    let Some(&(first, _)) = samples.first() else { return PnCorr { status: observations::CorrelationStatus::NoSignal, d: None, ratio: 0.0 } };
    let expected = first + PN_RADIUS * per;
    let from = (expected - t0).div_euclid(per);
    let reference: Vec<Iq> = (0..PN_REF as i64).map(|k| pn_at(period, from + k)).collect();
    let found = observations::correlation_offset(samples, &reference, expected, per, PN_RADIUS);
    PnCorr { status: found.status, d: found.sample_offset, ratio: if found.median > 0.0 { found.peak / found.median } else { 0.0 } }
}

/// The relative rms error of the first `PN_RES_SAMPLES` of `samples` against the continuous capture `cont` read `shift` samples later, with the best
/// complex gain fitted: `cont` is one whole PN period of the settled stream through the same channel (its filters included), so a sample at PN phase
/// `p` is predicted by the continuous sample of phase `p - shift`. None with fewer than 16 samples, a `cont` shorter than a period or no energy.
pub fn pn_residual(samples: &[(i64, Iq)], cont: &[(i64, Iq)], t0: i64, per: i64, shift: i64) -> Option<f64> {
    let window = &samples[..samples.len().min(PN_RES_SAMPLES)];
    if window.len() < 16 || cont.len() < PN_PERIOD {
        return None;
    }
    let phase = |tick: i64| (tick - t0).div_euclid(per);
    let first = phase(cont[0].0);
    let predicted: Vec<(f64, f64)> = window.iter().map(|(tick, _)| cont[(phase(*tick) - shift - first).rem_euclid(PN_PERIOD as i64) as usize].1).map(|p| (f64::from(p[0]), f64::from(p[1]))).collect();
    let (mut re, mut im, mut power, mut energy) = (0.0, 0.0, 0.0, 0.0);
    for ((_, sample), (pr, pi)) in window.iter().zip(&predicted) {
        let (rr, ri) = (f64::from(sample[0]), f64::from(sample[1]));
        re += pr * rr + pi * ri;
        im += pr * ri - pi * rr;
        power += pr * pr + pi * pi;
        energy += rr * rr + ri * ri;
    }
    if power <= 0.0 || energy <= 0.0 {
        return None;
    }
    let (gr, gi) = (re / power, im / power);
    let error: f64 = window.iter().zip(&predicted).map(|((_, s), (pr, pi))| (f64::from(s[0]) - (gr * pr - gi * pi)).powi(2) + (f64::from(s[1]) - (gr * pi + gi * pr)).powi(2)).sum();
    Some((error / energy).sqrt())
}

/// What one trial measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PnMeasure {
    pub z: usize,
    pub cont: PnCorr,
    pub restart: PnCorr,
    pub res_gd: Option<f64>,
    pub res_shift: Option<f64>,
}

impl PnMeasure {
    /// `d_restart - d_cont`, when both are found.
    pub fn delta(&self) -> Option<i64> {
        Some(self.restart.d? - self.cont.d?)
    }
}

/// The measurement of a continuous capture and a restart capture of the same PN (`t0`, `per` as in `locate_pn`): the zeros, both delays and the residuals.
pub fn pn_analyse(period: &[Iq], t0: i64, per: i64, cont: &[(i64, Iq)], restart: &[(i64, Iq)]) -> PnMeasure {
    let z = restart.iter().take_while(|(_, sample)| sample[0] == 0.0 && sample[1] == 0.0).count();
    let after = &restart[z..];
    let cont_corr = locate_pn(cont, period, t0, per);
    PnMeasure { z, cont: cont_corr, restart: locate_pn(after, period, t0, per), res_gd: pn_residual(after, cont, t0, per, 0), res_shift: pn_residual(after, cont, t0, per, z as i64) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PnOutcome {
    GroupDelay,
    Shifted,
    Neither,
    Inseparable,
    /// A delay was not found: no PN in the capture.
    Unmeasured,
}

impl PnOutcome {
    pub fn name(self) -> &'static str {
        match self {
            PnOutcome::GroupDelay => "group_delay",
            PnOutcome::Shifted => "shifted",
            PnOutcome::Neither => "neither",
            PnOutcome::Inseparable => "inseparable",
            PnOutcome::Unmeasured => "unmeasured",
        }
    }
}

/// The rule (`PNZ_RULE`) on one trial.
pub fn pn_outcome(measure: &PnMeasure) -> PnOutcome {
    let Some(delta) = measure.delta() else { return PnOutcome::Unmeasured };
    if measure.z < PN_MIN_ZEROS {
        PnOutcome::Inseparable
    } else if delta.abs() <= PN_TOL && measure.res_gd.is_some_and(|residual| residual <= PN_RES_MATCH) {
        PnOutcome::GroupDelay
    } else if (delta - measure.z as i64).abs() <= PN_TOL {
        PnOutcome::Shifted
    } else {
        PnOutcome::Neither
    }
}

/// One trial of a cell.
#[derive(Debug, Clone)]
pub struct PnTrial {
    pub warmup: bool,
    /// A failed device call.
    pub error: Option<String>,
    /// Why the captures do not count (a gap, a receive or transmit outcome, no burst ACK): not a fact about the zeros.
    pub incomplete: Option<String>,
    /// The restart's first block is not at its planned tick in a read with no other outcome.
    pub misplaced: bool,
    pub measure: Option<PnMeasure>,
}

impl PnTrial {
    fn usable(&self) -> bool {
        self.error.is_none() && self.incomplete.is_none() && !self.misplaced
    }

    pub fn outcome(&self) -> Option<PnOutcome> {
        self.measure.filter(|_| self.usable()).map(|measure| pn_outcome(&measure))
    }

    /// Usable captures with no correlation peak: infrastructure.
    fn no_peak(&self) -> bool {
        self.outcome() == Some(PnOutcome::Unmeasured)
    }

    fn counted(&self) -> bool {
        self.outcome().is_some_and(|outcome| outcome != PnOutcome::Unmeasured)
    }
}

/// What a cell's trials add up to (warm-up excluded).
#[derive(Debug, Clone, PartialEq)]
pub struct PnStats {
    pub trials: usize,
    pub complete: usize,
    pub errors: usize,
    pub misplaced: usize,
    pub no_peak: usize,
    pub counts: BTreeMap<&'static str, usize>,
    pub z: Option<(usize, usize, usize)>,
    pub d_cont: Option<(i64, i64, i64)>,
    pub delta: Option<(i64, i64, i64)>,
}

pub fn pn_stats(trials: &[PnTrial]) -> PnStats {
    let counted: Vec<&PnTrial> = trials.iter().filter(|trial| !trial.warmup).collect();
    let complete: Vec<&&PnTrial> = counted.iter().filter(|trial| trial.counted()).collect();
    let mut counts = BTreeMap::new();
    for outcome in complete.iter().filter_map(|trial| trial.outcome()) {
        *counts.entry(outcome.name()).or_default() += 1;
    }
    let measures = || complete.iter().filter_map(|trial| trial.measure);
    PnStats {
        trials: counted.len(),
        complete: complete.len(),
        errors: counted.iter().filter(|trial| trial.error.is_some()).count(),
        misplaced: counted.iter().filter(|trial| trial.misplaced).count(),
        no_peak: counted.iter().filter(|trial| trial.no_peak()).count(),
        counts,
        z: spread(measures().map(|m| m.z).collect()),
        d_cont: spread(measures().filter_map(|m| m.cont.d).collect()),
        delta: spread(measures().filter_map(|m| m.delta()).collect()),
    }
}

/// FAIL for infrastructure only: a failed call, a first block off its tick, a usable capture without a correlation peak. INCONCLUSIVE below the cell's
/// planned complete trials. Otherwise PASS, whatever the outcome (`pn_cell_decision` says what it means).
pub fn pn_cell_verdict(cell: &PnCell, stats: &PnStats) -> Verdict {
    if stats.errors > 0 {
        return Verdict::Fail(format!("{} of {} trials had a failed device call", stats.errors, stats.trials));
    }
    if stats.misplaced > 0 {
        return Verdict::Fail(format!("{} of {} trials have the restart's first block off the planned tick", stats.misplaced, stats.trials));
    }
    if stats.no_peak > 0 {
        return Verdict::Fail(format!("{} of {} trials have a usable capture without a correlation peak (no PN in it)", stats.no_peak, stats.trials));
    }
    if stats.complete < cell.trials {
        return Verdict::Inconclusive(format!("{} of {} trials complete ({} needed)", stats.complete, stats.trials, cell.trials));
    }
    Verdict::Pass(format!("{} of {} complete; {:?}; z {:?}; d_cont {:?}; delta {:?}", stats.complete, stats.trials, stats.counts, stats.z, stats.d_cont, stats.delta))
}

/// What a cell decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PnKind {
    Undecided,
    Mixed,
    Outcome(PnOutcome),
}

pub fn pn_cell_kind(cell: &PnCell, stats: &PnStats) -> PnKind {
    if stats.errors > 0 || stats.misplaced > 0 || stats.no_peak > 0 || stats.complete < cell.trials {
        return PnKind::Undecided;
    }
    let outcomes: Vec<PnOutcome> =
        [PnOutcome::GroupDelay, PnOutcome::Shifted, PnOutcome::Neither, PnOutcome::Inseparable].into_iter().filter(|outcome| stats.counts.contains_key(outcome.name())).collect();
    match outcomes[..] {
        [one] => PnKind::Outcome(one),
        _ => PnKind::Mixed,
    }
}

pub fn pn_cell_line(cell: &PnCell, stats: &PnStats) -> String {
    let what = match pn_cell_kind(cell, stats) {
        PnKind::Undecided => format!("INCONCLUSIVE ({} of {} complete, {} failed call(s), {} misplaced, {} without a peak)", stats.complete, cell.trials, stats.errors, stats.misplaced, stats.no_peak),
        PnKind::Mixed => format!("MIXED {:?}", stats.counts),
        PnKind::Outcome(PnOutcome::GroupDelay) => "zeros = group delay: the signal is on time, the zeros replace samples that would have been there".to_owned(),
        PnKind::Outcome(PnOutcome::Shifted) => "shifted: the signal arrives z samples late".to_owned(),
        PnKind::Outcome(PnOutcome::Neither) => "neither (read the trial lines)".to_owned(),
        PnKind::Outcome(_) => format!("inseparable: z < {PN_MIN_ZEROS}, the two readings cannot be told apart"),
    };
    format!("RXZPN cell {}: z min/med/max {:?}; d_cont {:?}; d_restart - d_cont {:?}; {what}", cell.name(), stats.z, stats.d_cont, stats.delta)
}

/// The decision over every rate: each rate's line, then the whole.
pub fn pn_decision_lines(runs: &[(PnCell, PnStats)]) -> Vec<String> {
    let mut lines: Vec<String> = runs.iter().map(|(cell, stats)| pn_cell_line(cell, stats)).collect();
    let kinds: Vec<PnKind> = runs.iter().map(|(cell, stats)| pn_cell_kind(cell, stats)).collect();
    let separable: Vec<PnKind> = kinds.iter().copied().filter(|kind| *kind != PnKind::Outcome(PnOutcome::Inseparable)).collect();
    lines.push(if runs.is_empty() || kinds.contains(&PnKind::Undecided) {
        "RXZPN decision: INCONCLUSIVE: a rate is not decided (read the cell lines)".to_owned()
    } else if separable.is_empty() {
        "RXZPN decision: inseparable at every rate: no rate has z >= 3".to_owned()
    } else if separable.iter().all(|kind| *kind == PnKind::Outcome(PnOutcome::GroupDelay)) {
        format!("RXZPN decision: zeros = group delay at the {} rate(s) with z >= 3 (the signal is on time; the zeros replace samples that would have been there)", separable.len())
    } else if separable.iter().all(|kind| *kind == PnKind::Outcome(PnOutcome::Shifted)) {
        format!("RXZPN decision: shifted at the {} rate(s) with z >= 3 (the signal arrives z samples late)", separable.len())
    } else {
        "RXZPN decision: neither, or different at different rates: read the cell lines".to_owned()
    });
    lines
}

/// What a read over a tick window kept.
pub struct Window {
    pub samples: Vec<(i64, Iq)>,
    first_block: Option<i64>,
    /// The end tick of the last block when an outcome other than samples and a timeout arrived, with the outcome.
    other: Vec<(i64, String)>,
}

/// Reads until the stream reaches `from + span * per` or `limit` passes, keeping the samples of `[from, from + span * per)`: the settled stream before
/// the window is read and dropped, so that 200 MS/s does not fill the memory. A failed call ends it with that error.
pub fn read_window(device: &dyn Device, n: usize, per: i64, from: i64, span: usize, limit: Wall) -> Result<Window, String> {
    let to = from + span as i64 * per;
    let mut out = Window { samples: Vec::with_capacity(span), first_block: None, other: Vec::new() };
    let mut end = i64::MIN;
    let begun = Instant::now();
    while begun.elapsed() < limit && end < to {
        match device.rx_recv(0, n, Wall::from_millis(100), false) {
            ezsdr_radio_uhd::RxRecv::Samples { first_tick, samples } => {
                out.first_block.get_or_insert(first_tick);
                let block = &samples[0];
                end = first_tick + block.len() as i64 * per;
                let begin = ((from - first_tick).max(0) + per - 1) / per;
                let finish = ((to - first_tick).max(0) + per - 1) / per;
                out.samples.extend(block.iter().enumerate().take(finish as usize).skip(begin as usize).map(|(index, sample)| (first_tick + index as i64 * per, *sample)));
            }
            ezsdr_radio_uhd::RxRecv::Timeout => {}
            ezsdr_radio_uhd::RxRecv::Failed(error) => return Err(error_text("rx_recv", error)),
            other => out.other.push((end, format!("{other:?}"))),
        }
    }
    Ok(out)
}

/// The samples per `rx_recv` call: about a millisecond at 1 MS/s and above, 2 000 to 50 000 (100 MS/s needs calls of tens of thousands to keep up).
fn pn_read_len(rate: f64) -> usize {
    ((rate / 1e3) as usize).clamp(2_000, 50_000)
}

fn window_covered(window: &Window, from: i64, span: usize, per: i64) -> bool {
    window.samples.len() == span && window.samples[0].0 == from && contiguous(&window.samples, per)
}

/// Sends the PN, repeated, as one burst from `t0` (timed) until `stop`, then closes it with two samples and an end of burst; the samples sent.
/// The device's flow control paces it. PN sample `n` is sent at `t0 + n * per`, partial sends included.
fn feed_pn(device: &dyn Device, ring: &[Iq], t0: i64, stop: &std::sync::atomic::AtomicBool) -> Result<usize, String> {
    use std::sync::atomic::Ordering::Relaxed;
    let mut sent = 0usize;
    while !stop.load(Relaxed) {
        let at = (sent == 0).then_some(t0);
        let from = sent % PN_PERIOD;
        let accepted = device.tx_send(0, &[&ring[from..from + PN_CHUNK]], at, sent == 0, false, Wall::from_millis(500)).map_err(|e| error_text("tx_send", e))?;
        sent += accepted;
        if accepted == 0 {
            std::thread::sleep(Wall::from_millis(1));
        }
    }
    let from = sent % PN_PERIOD;
    device.tx_send(0, &[&ring[from..from + 2]], (sent == 0).then_some(t0), sent == 0, true, Wall::from_millis(500)).map_err(|e| error_text("tx_send (end)", e))?;
    Ok(sent)
}

/// One trial: the PN is transmitted from `t0`; a receive started at `ta` is read for `settle_ms`, then `PN_CONT_SPAN` samples are kept (the continuous
/// capture); the receive is stopped, drained and started timed at `t1` inside the running PN, and its first `PN_SPAN` samples are kept (the restart).
fn pn_trial(device: &dyn Device, cell: &PnCell, period: &[Iq], ring: &[Iq], warmup: bool, index: usize) -> PnTrial {
    let per = cell.decim as i64;
    let n = pn_read_len(cell.rate());
    if !quiet_rx(device) {
        return pn_failed(cell, warmup, index, "the previous segment did not go quiet".to_owned());
    }
    let outcome = (|| -> Result<_, String> {
        let now = device.time_now().map_err(|e| error_text("time_now", e))?;
        let ta = align_to(now + 50 * TICKS_PER_MS, per);
        let t0 = align_to(ta + 20 * TICKS_PER_MS, per);
        let wa = align_to(ta + cell.settle_ms as i64 * TICKS_PER_MS, per);
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let feeder = scope.spawn(|| feed_pn(device, ring, t0, &stop));
            let captured = (|| -> Result<_, String> {
                device.rx_start(0, ta).map_err(|e| error_text("rx_start", e))?;
                let cont = read_window(device, n, per, wa, PN_CONT_SPAN, Wall::from_millis(cell.settle_ms + 4_000))?;
                if !quiet_rx(device) {
                    return Err("the continuous segment did not go quiet".to_owned());
                }
                let t1 = align_to(device.time_now().map_err(|e| error_text("time_now", e))? + 50 * TICKS_PER_MS, per);
                device.rx_start(0, t1).map_err(|e| error_text("rx_start (restart)", e))?;
                let restart = read_window(device, n, per, t1, PN_SPAN, Wall::from_secs(2))?;
                device.rx_stop(0).map_err(|e| error_text("rx_stop", e))?;
                Ok((cont, t1, restart))
            })();
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let fed = feeder.join().expect("the PN feeder");
            let (cont, t1, restart) = captured?;
            Ok((t0, wa, t1, cont, restart, fed?))
        })
    })();
    let (t0, wa, t1, cont, restart, fed) = match outcome {
        Ok(done) => done,
        Err(text) => {
            let _ = quiet_rx(device);
            return pn_failed(cell, warmup, index, text);
        }
    };
    let (ack, reports) = super::raw::wait_burst_ack(device, Wall::from_secs(3));
    let bad: Vec<String> = reports.iter().filter(|report| report.code != TxCode::BurstAck).map(|report| format!("{:?}", report.code)).collect();
    let guard = 5 * TICKS_PER_MS;
    let misplaced = restart.other.is_empty() && restart.first_block.is_some_and(|tick| tick != t1);
    let incomplete = if !window_covered(&cont, wa, PN_CONT_SPAN, per) {
        Some(format!("the continuous window is not covered ({} of {PN_CONT_SPAN} samples)", cont.samples.len()))
    } else if !window_covered(&restart, t1, PN_SPAN, per) {
        Some(format!("the restart window is not covered ({} of {PN_SPAN} samples)", restart.samples.len()))
    } else if let Some((_, text)) = cont.other.iter().find(|(end, _)| *end >= wa - guard).or(restart.other.first()) {
        Some(format!("a receive outcome in or just before a window: {text}"))
    } else if !bad.is_empty() {
        Some(format!("the transmitter reported {bad:?}"))
    } else if ack.is_none() {
        Some("no burst ACK for the PN".to_owned())
    } else {
        None
    };
    let measure = (incomplete.is_none() && !misplaced).then(|| pn_analyse(period, t0, per, &cont.samples, &restart.samples));
    let done = PnTrial { warmup, error: None, incomplete, misplaced, measure };
    println!(
        "RXZPN trial {}",
        json!({
            "cell": cell.name(), "decim": cell.decim, "rate": cell.rate(), "index": index, "warmup": warmup,
            "t0": t0, "window_a": wa, "planned": t1, "first_block": restart.first_block, "fed_samples": fed, "tx_reports": reports.len(), "tx_bad": bad,
            "z": measure.map(|m| m.z), "d_cont": measure.and_then(|m| m.cont.d), "d_restart": measure.and_then(|m| m.restart.d), "delta": measure.and_then(|m| m.delta()),
            "status_cont": measure.map(|m| format!("{:?}", m.cont.status)), "status_restart": measure.map(|m| format!("{:?}", m.restart.status)),
            "ratio_cont": measure.map(|m| m.cont.ratio), "ratio_restart": measure.map(|m| m.restart.ratio), "res_gd": measure.and_then(|m| m.res_gd), "res_shift": measure.and_then(|m| m.res_shift),
            "outcome": done.outcome().map(PnOutcome::name), "complete": done.counted(), "incomplete": done.incomplete, "misplaced": misplaced,
            "rx_other": cont.other.iter().chain(&restart.other).map(|(_, text)| text.clone()).collect::<Vec<_>>(), "head": head_text(&restart.samples),
        })
    );
    done
}

fn pn_failed(cell: &PnCell, warmup: bool, index: usize, error: String) -> PnTrial {
    println!("RXZPN trial {}", json!({ "cell": cell.name(), "index": index, "warmup": warmup, "complete": false, "error": error }));
    PnTrial { warmup, error: Some(error), incomplete: None, misplaced: false, measure: None }
}

/// One cell: the transmitter reopened at the cell's rate (gain 0 dB), the receiver at the same rate, a warm-up trial and `cell.trials`.
fn run_pn_cell(device: &dyn Device, hz: f64, cell: &PnCell, period: &[Iq], ring: &[Iq]) -> Vec<PnTrial> {
    let failed_all = |text: String| (0..cell.trials).map(|index| pn_failed(cell, false, index, text.clone())).collect::<Vec<_>>();
    let rate = cell.rate();
    let _ = quiet_rx(device);
    device.tx_close(0);
    tone_tx(device, rate, hz, 0.0);
    match device.apply(Dir::Rx, d9_channel(), &rx_settings(Some(rate), Some(hz))) {
        Ok(applied) if (applied.rate - rate).abs() <= rate * 1e-9 + 1e-6 => {}
        Ok(applied) => return failed_all(format!("rate mismatch (asked {rate}, read back {})", applied.rate)),
        Err(error) => return failed_all(error_text("apply rate", error)),
    }
    (0..=cell.trials).map(|index| pn_trial(device, cell, period, ring, index == 0, index)).collect()
}

/// The PN cell on `device`: every cell in order, the transmitter at gain 0 dB, the first trial of each cell a warm-up (printed, not counted).
pub fn pn_sweep(device: &dyn Device, cells: &[PnCell]) -> Vec<(PnCell, Vec<PnTrial>)> {
    assert_slot_a_only(d9_channel());
    println!("{PNZ_RULE}");
    let hz = pin_slot_a(device);
    let period = pn_period();
    let ring: Vec<Iq> = (0..PN_PERIOD + PN_CHUNK).map(|i| period[i % PN_PERIOD]).collect();
    assert!(ring.iter().all(|sample| f64::from(sample[0]).hypot(f64::from(sample[1])) <= 0.5), "bench RF safety: TX amplitude exceeds 0.5");
    raw_rx(device, 1e6, hz);
    let mut runs = Vec::new();
    for cell in cells {
        let trials = run_pn_cell(device, hz, cell, &period, &ring);
        println!("{}", pn_cell_line(cell, &pn_stats(&trials)));
        runs.push((*cell, trials));
    }
    device.rx_stop(0).ok();
    let _ = drain(device);
    device.tx_close(0);
    runs
}

/// The verdicts of a PN sweep, with the decision lines and the rule printed.
pub fn pn_report(runs: &[(PnCell, Vec<PnTrial>)]) -> Vec<(String, Verdict)> {
    let stats: Vec<(PnCell, PnStats)> = runs.iter().map(|(cell, trials)| (*cell, pn_stats(trials))).collect();
    for line in pn_decision_lines(&stats) {
        println!("{line}");
    }
    println!("RXZPN summary: {PNZ_RULE}");
    stats.iter().map(|(cell, stats)| (format!("rxzpn {}", cell.name()), pn_cell_verdict(cell, stats))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_trial(zeros: usize, warmup: bool) -> ZTrial {
        let trial = RestartTrial {
            drained: true,
            asked: 1e6,
            applied: 1e6,
            planned: 100,
            first_tick: Some(100),
            covered: true,
            tone_hz: Some(100e3),
            fill: Some(iii::Fill { zeros, ramp10: 6, ramp90: 7, steady: 0.01 }),
        };
        ZTrial { warmup, trial, expected_tone_hz: 100e3, precursor_peak: Some(0.02), error: None, rx_other: Vec::new() }
    }

    fn cell(decim: u64, start: Start, gain_db: f64, trials: usize) -> ZCell {
        ZCell { decim, start, gain_db, trials }
    }

    fn stats_of(zeros: &[usize]) -> CellStats {
        cell_stats(&zeros.iter().map(|zeros| fill_trial(*zeros, false)).collect::<Vec<_>>())
    }

    /// Every class at its measured range, every level and start cell like its restart cell.
    fn plain_runs() -> Vec<(ZCell, CellStats)> {
        zeros_cells()
            .into_iter()
            .map(|cell| {
                let zeros = match cell.half_bands() {
                    0 => 0,
                    1 => 4,
                    2 => 11,
                    _ => 14,
                };
                let stats = stats_of(&vec![zeros; cell.trials]);
                (cell, stats)
            })
            .collect()
    }

    fn with(runs: &mut [(ZCell, CellStats)], decim: u64, start: Start, gain: f64) -> &mut CellStats {
        &mut runs.iter_mut().find(|(c, _)| c.decim == decim && c.start == start && c.gain_db == gain).expect("the cell exists").1
    }

    #[test]
    fn the_half_band_count_is_the_twos_in_the_decimation_capped_at_three() {
        let classes: Vec<(u64, u32)> = vec![(1, 0), (3, 0), (255, 0), (2, 1), (6, 1), (250, 1), (4, 2), (12, 2), (100, 2), (8, 3), (24, 3), (200, 3), (248, 3), (16, 3), (64, 3)];
        for (decim, class) in classes {
            assert_eq!(half_bands(decim), class, "D {decim}");
        }
    }

    #[test]
    fn the_sweeps_cells_are_the_plan() {
        let cells = zeros_cells();
        assert_eq!(cells.len(), 20 + 9 + 6);
        for cell in &cells {
            assert!(ezsdr_radio::x3xx::is_decimation(cell.decim), "D {} is not a decimation of the X3xx", cell.decim);
            assert!(cell.rate() >= 0.4e6 && cell.rate() <= 200e6, "{}", cell.name());
            assert!(cell.gain_db <= MAX_TONE_GAIN_DB);
        }
        for class in 0..4 {
            let rate_cells: Vec<u64> = cells.iter().filter(|c| c.start == Start::Restart && c.gain_db == 0.0 && c.half_bands() == class).map(|c| c.decim).collect();
            assert_eq!(rate_cells.len(), 5, "class {class}: {rate_cells:?}");
        }
        let trials = |decim: u64| cells.iter().find(|c| c.decim == decim && c.start == Start::Restart && c.gain_db == 0.0).unwrap().trials;
        assert_eq!([trials(3), trials(125), trials(255), trials(6), trials(10), trials(50), trials(250)], [15; 7]);
        assert_eq!([trials(1), trials(2), trials(4), trials(100), trials(200)], [10; 5]);
        assert_eq!(cells.iter().filter(|c| c.start != Start::Restart).count(), 9);
        assert_eq!(cells.iter().filter(|c| c.gain_db > 0.0).map(|c| c.gain_db as u32).collect::<Vec<_>>(), [10, 10, 10, 20, 20, 20]);
        assert!(cells.windows(2).all(|pair| pair[0].gain_db <= pair[1].gain_db), "the gain changes twice, upwards");
    }

    #[test]
    fn a_cell_needs_every_planned_trial() {
        assert_eq!([cell(1, Start::Restart, 0.0, 10).needed(), cell(1, Start::Restart, 0.0, 15).needed(), cell(1, Start::Restart, 0.0, 7).needed()], [10, 15, 7]);
        assert!(zeros_cells().iter().all(|cell| cell.trials >= 10), "at least ten valid observations a cell");
    }

    #[test]
    fn a_retune_cell_alternates_the_receiver_and_the_expected_tone_follows_it() {
        assert_eq!([retune_shift(0), retune_shift(1), retune_shift(2)], [RETUNE_HZ, 0.0, RETUNE_HZ]);
        assert_eq!([expected_tone_hz(RETUNE_HZ), expected_tone_hz(0.0)], [50e3, 100e3]);
    }

    #[test]
    fn the_companion_decimation_differs_from_the_cells_own() {
        assert_eq!([companion(200), companion(100), companion(2), companion(8)], [100, 200, 4, 16]);
    }

    #[test]
    fn the_stats_leave_the_warm_up_out_and_count_complete_trials() {
        let mut trials = vec![fill_trial(99, true)];
        trials.extend([13, 14, 15].map(|zeros| fill_trial(zeros, false)));
        let mut bad = fill_trial(0, false);
        bad.trial.covered = false;
        trials.push(bad);
        let stats = cell_stats(&trials);
        assert_eq!((stats.trials, stats.complete, stats.misplaced, stats.errors), (4, 3, 0, 0));
        assert_eq!(stats.zeros, Some((13, 14, 15)));
        assert_eq!(stats.precursor_peak, Some(0.02));
    }

    #[test]
    fn a_failed_call_or_a_misplaced_first_block_fails_the_cell_and_too_few_trials_are_inconclusive() {
        let c = cell(200, Start::Restart, 0.0, 10);
        let mut trials: Vec<ZTrial> = (0..10).map(|_| fill_trial(14, false)).collect();
        assert!(matches!(cell_verdict(&c, &cell_stats(&trials)), Verdict::Pass(_)));
        trials[3].trial.first_tick = Some(101);
        assert!(matches!(cell_verdict(&c, &cell_stats(&trials)), Verdict::Fail(text) if text.contains("planned tick")));
        trials[3] = ZTrial { error: Some("rx_start: boom".to_owned()), ..fill_trial(14, false) };
        assert!(matches!(cell_verdict(&c, &cell_stats(&trials)), Verdict::Fail(text) if text.contains("failed device call")));
        let few: Vec<ZTrial> = (0..10)
            .map(|i| {
                let mut t = fill_trial(14, false);
                t.trial.covered = i < 3;
                t
            })
            .collect();
        assert!(matches!(cell_verdict(&c, &cell_stats(&few)), Verdict::Inconclusive(text) if text.contains("3 of 10")));
        let nine: Vec<ZTrial> = (0..10)
            .map(|i| {
                let mut t = fill_trial(14, false);
                t.trial.covered = i < 9;
                t
            })
            .collect();
        assert!(matches!(cell_verdict(&c, &cell_stats(&nine)), Verdict::Inconclusive(text) if text.contains("9 of 10") && text.contains("10 needed")), "nine of ten is below the ten planned");
    }

    #[test]
    fn an_overflow_ahead_of_the_first_block_is_inconclusive_never_an_off_tick_fail() {
        let c = cell(200, Start::Restart, 0.0, 10);
        let trials = |dirty: &dyn Fn(&mut ZTrial)| -> Vec<ZTrial> {
            (0..10)
                .map(|i| {
                    let mut t = fill_trial(14, false);
                    if i == 3 {
                        t.trial.first_tick = Some(101);
                        t.trial.covered = false;
                        dirty(&mut t);
                    }
                    t
                })
                .collect()
        };
        let verdict = |trials: &[ZTrial]| cell_verdict(&c, &cell_stats(trials));
        let clean = trials(&|_| {});
        assert!(matches!(verdict(&clean), Verdict::Fail(text) if text.contains("planned tick")), "a first block off the tick in a clean read is a FAIL");
        assert_eq!(cell_stats(&clean).misplaced, 1);
        let overflow = trials(&|t| t.rx_other = vec!["Overflow { out_of_sequence: false }".to_owned()]);
        assert!(matches!(verdict(&overflow), Verdict::Inconclusive(text) if text.contains("9 of 10")), "{:?}", verdict(&overflow));
        assert_eq!(cell_stats(&overflow).misplaced, 0);
        let failed = trials(&|t| {
            t.rx_other = vec!["Overflow".to_owned()];
            t.error = Some("rx_start: boom".to_owned());
        });
        assert!(matches!(verdict(&failed), Verdict::Fail(text) if text.contains("failed device call")), "a failed call still fails");
    }

    #[test]
    fn a_trial_with_a_failed_call_does_not_count() {
        let trials = vec![ZTrial { error: Some("apply rate: boom".to_owned()), ..fill_trial(14, false) }, fill_trial(14, false)];
        let stats = cell_stats(&trials);
        assert_eq!((stats.trials, stats.complete, stats.errors), (2, 1, 1));
    }

    #[test]
    fn the_precursor_peak_is_the_largest_sample_before_the_ten_percent_point() {
        let per = 200;
        let mut samples: Vec<(i64, Iq)> = vec![(0, [0.0, 0.0]); 5];
        samples.extend([0.01f32, 0.04, 0.02].map(|m| (0, [m, 0.0])));
        samples.extend([0.5f32, 1.0].map(|m| (0, [m, 0.0])));
        samples.extend(vec![(0, [1.0, 0.0]); 20]);
        let samples: Vec<(i64, Iq)> = samples.into_iter().enumerate().map(|(i, (_, s))| (i as i64 * per, s)).collect();
        let peak = precursor_peak(&samples).expect("a precursor");
        assert!((peak - 0.04).abs() < 1e-6, "{peak}");
        let steep: Vec<(i64, Iq)> = (0..20).map(|i| (i * per, if i < 4 { [0.0, 0.0] } else { [1.0, 0.0] })).collect();
        assert_eq!(precursor_peak(&steep), Some(0.0));
        assert_eq!(precursor_peak(&[(0, [0.0, 0.0]); 10]), None);
    }

    fn short_stats(zeros: usize, complete: usize, of: usize) -> CellStats {
        let trials: Vec<ZTrial> = (0..of)
            .map(|i| {
                let mut t = fill_trial(zeros, false);
                t.trial.covered = i < complete;
                t
            })
            .collect();
        cell_stats(&trials)
    }

    #[test]
    fn the_measured_classes_hold_and_the_plan_says_d() {
        let decision = decide(&plain_runs());
        assert_eq!(decision.d_holds, Some(true));
        assert!(decision.missing.is_empty());
        assert_eq!(decision.classes.iter().map(|c| c.observed).collect::<Vec<_>>(), vec![Some((0, 0)), Some((4, 4)), Some((11, 11)), Some((14, 14))]);
        assert_eq!(decision.classes.iter().map(|c| (c.bound, c.bound_cells)).collect::<Vec<_>>(), vec![(Some(0), 5), (Some(4), 10), (Some(11), 10), (Some(14), 10)]);
        let lines = decision_lines(&decision);
        assert!(lines.iter().any(|l| l.contains("decision 1: D holds")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("decision 2") && l.contains("every start")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("decision 3") && l.contains("no KNOWN_LIMITATIONS")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("decision 4") && l.contains("near zero")), "{lines:?}");
    }

    #[test]
    fn a_class_line_marks_an_incomplete_bound() {
        let line_of = |runs: &[(ZCell, CellStats)], class: u32| decision_lines(&decide(runs)).into_iter().find(|l| l.starts_with(&format!("RXZ class n_hb {class}"))).unwrap();
        let line = line_of(&plain_runs(), 3);
        assert!(!line.contains("INCOMPLETE") && line.contains("candidate bound Some(14)") && line.contains("max over 10 of 10 planned"), "{line}");
        let mut runs = plain_runs();
        runs.retain(|(c, _)| !(c.decim == 100 && c.start == Start::Fresh));
        let line = line_of(&runs, 2);
        assert!(line.contains("candidate bound Some(11) (INCOMPLETE") && line.contains("max over 9 of 10 planned"), "{line}");
        assert!(!line_of(&runs, 3).contains("INCOMPLETE"), "the other classes stay complete");
        let decision = decide(&runs);
        assert_eq!((decision.classes[2].bound_cells, decision.classes[2].planned_cells), (9, 10));
    }

    #[test]
    fn a_class_wider_than_one_either_way_is_b_and_one_inside_is_d() {
        let mut runs = plain_runs();
        *with(&mut runs, 6, Start::Restart, 0.0) = stats_of(&[6; 15]);
        assert_eq!(decide(&runs).d_holds, Some(false), "n_hb 1 reaching 6 against 3-4");
        let mut runs = plain_runs();
        *with(&mut runs, 6, Start::Restart, 0.0) = stats_of(&[5; 15]);
        assert_eq!(decide(&runs).d_holds, Some(true), "5 is 4 + 1");
        *with(&mut runs, 6, Start::Restart, 0.0) = stats_of(&[1; 15]);
        assert_eq!(decide(&runs).d_holds, Some(false), "1 is 3 - 2");
        *with(&mut runs, 6, Start::Restart, 0.0) = stats_of(&[2; 15]);
        assert_eq!(decide(&runs).d_holds, Some(true), "2 is 3 - 1");
        let mut runs = plain_runs();
        *with(&mut runs, 255, Start::Restart, 0.0) = stats_of(&[2; 15]);
        assert_eq!(decide(&runs).d_holds, Some(false), "n_hb 0 reaching 2 against 0");
        let lines = decision_lines(&decide(&runs));
        assert!(lines.iter().any(|l| l.contains("decision 1") && l.contains("B, a table over the swept rates")), "{lines:?}");
    }

    #[test]
    fn the_structure_reads_the_gain_zero_restart_cells_and_the_bound_every_counted_cell() {
        let mut runs = plain_runs();
        *with(&mut runs, 200, Start::Restart, 20.0) = stats_of(&[20; 10]);
        let decision = decide(&runs);
        assert_eq!(decision.d_holds, Some(true), "a level cell is the amplitude rule's, not the structure's");
        assert_eq!(decision.classes[3].observed, Some((14, 14)));
        assert_eq!(decision.classes[3].bound, Some(20), "the candidate bound is the maximum over every measured level");
        let lines = decision_lines(&decision);
        assert!(lines.iter().any(|l| l.contains("class n_hb 3") && l.contains("candidate bound Some(20)")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("decision 1: D holds")) && lines.iter().any(|l| l.contains("decision 3") && l.contains("exact for the stated level only")), "{lines:?}");
        *with(&mut runs, 200, Start::Restart, 20.0) = stats_of(&[3; 10]);
        assert_eq!(decide(&runs).classes[3].bound, Some(14), "a level that lowers the count never lowers the bound");
        *with(&mut runs, 100, Start::Fresh, 0.0) = stats_of(&[13; 10]);
        let decision = decide(&runs);
        assert_eq!((decision.classes[2].observed, decision.classes[2].bound), (Some((11, 11)), Some(13)), "a start kind with more zeros raises the bound, not the structure");
        assert_eq!(decision.d_holds, Some(true));
        assert!(decision.start_differs.contains(&(100, Start::Fresh, true)));
    }

    #[test]
    fn a_missing_or_short_cell_makes_every_decision_inconclusive_and_names_it() {
        let mut runs = plain_runs();
        runs.retain(|(c, _)| !(c.decim == 3 && c.start == Start::Restart && c.gain_db == 0.0));
        let decision = decide(&runs);
        assert_eq!(decision.d_holds, None, "an odd-decimation cell is missing");
        assert_eq!(decision.missing.len(), 1);
        let lines = decision_lines(&decision);
        let inconclusive: Vec<&String> = lines.iter().filter(|l| l.starts_with("RXZ decision") && l.contains("INCONCLUSIVE") && l.contains("D3 n_hb0")).collect();
        assert_eq!(inconclusive.len(), 4, "{lines:?}");
        assert!(lines.iter().all(|l| !l.contains("D holds") && !l.contains("applies to every start") && !l.contains("no KNOWN_LIMITATIONS")), "{lines:?}");
        let mut runs = plain_runs();
        *with(&mut runs, 8, Start::Restart, 0.0) = short_stats(14, 9, 10);
        assert_eq!(decide(&runs).d_holds, None, "nine of ten valid observations do not count");
        *with(&mut runs, 8, Start::Restart, 0.0) = short_stats(14, 10, 10);
        assert_eq!(decide(&runs).d_holds, Some(true));
        *with(&mut runs, 255, Start::Restart, 0.0) = short_stats(0, 14, 15);
        assert_eq!(decide(&runs).d_holds, None, "fourteen of the fifteen planned");
        let mut runs = plain_runs();
        with(&mut runs, 200, Start::Fresh, 0.0).errors = 1;
        assert_eq!(decide(&runs).missing.len(), 1, "a cell with a failed call is not counted");
        let mut runs = plain_runs();
        with(&mut runs, 200, Start::Fresh, 0.0).misplaced = 1;
        assert_eq!(decide(&runs).missing.len(), 1, "a cell with a misplaced first block is not counted");
        let mut runs = plain_runs();
        *with(&mut runs, 100, Start::Restart, 20.0) = short_stats(11, 7, 10);
        let lines = decision_lines(&decide(&runs));
        assert!(lines.iter().any(|l| l.contains("decision 3") && l.contains("INCONCLUSIVE") && l.contains("D100 n_hb2 restart gain +20 dB")), "{lines:?}");
        assert_eq!(decide(&[]).missing.len(), zeros_cells().len());
    }

    #[test]
    fn a_fresh_open_outside_one_of_the_restart_range_limits_the_fill_to_restarts() {
        let mut runs = plain_runs();
        *with(&mut runs, 200, Start::Fresh, 0.0) = stats_of(&[14; 10]);
        assert!(decision_lines(&decide(&runs)).iter().any(|l| l.contains("decision 2") && l.contains("every start")));
        *with(&mut runs, 200, Start::Fresh, 0.0) = stats_of(&[0; 10]);
        let decision = decide(&runs);
        assert!(decision.start_differs.contains(&(200, Start::Fresh, true)));
        assert!(decision.start_differs.contains(&(100, Start::Fresh, false)));
        assert!(decision_lines(&decision).iter().any(|l| l.contains("decision 2") && l.contains("restarts) only")));
        *with(&mut runs, 200, Start::Fresh, 0.0) = stats_of(&[15; 10]);
        assert!(!decide(&runs).start_differs.contains(&(200, Start::Fresh, true)), "15 is 14 + 1");
        *with(&mut runs, 200, Start::Fresh, 0.0) = stats_of(&[16; 10]);
        assert!(decide(&runs).start_differs.contains(&(200, Start::Fresh, true)));
    }

    #[test]
    fn a_gain_that_moves_a_limit_by_more_than_two_gets_a_known_limitation() {
        let mut runs = plain_runs();
        *with(&mut runs, 100, Start::Restart, 10.0) = stats_of(&[9; 10]);
        let decision = decide(&runs);
        assert!(decision.amplitude_shift.contains(&(100, 10.0, 2)));
        assert!(decision_lines(&decision).iter().any(|l| l.contains("decision 3") && l.contains("no KNOWN_LIMITATIONS")), "a shift of 2 is within the rule");
        *with(&mut runs, 100, Start::Restart, 10.0) = stats_of(&[8; 10]);
        let decision = decide(&runs);
        assert!(decision.amplitude_shift.contains(&(100, 10.0, 3)));
        assert!(decision_lines(&decision).iter().any(|l| l.contains("decision 3") && l.contains("exact for the stated level only")));
        *with(&mut runs, 100, Start::Restart, 10.0) = stats_of(&[11; 10]);
        *with(&mut runs, 100, Start::Restart, 20.0) = stats_of(&[14; 10]);
        assert!(decision_lines(&decide(&runs)).iter().any(|l| l.contains("exact for the stated level only")), "a gain that raises the bound moves it too");
    }

    #[test]
    fn a_signal_like_precursor_reopens_the_recommendation() {
        let mut runs = plain_runs();
        with(&mut runs, 8, Start::Restart, 0.0).precursor_peak = Some(0.049);
        assert!(decision_lines(&decide(&runs)).iter().any(|l| l.contains("decision 4") && l.contains("near zero")));
        with(&mut runs, 8, Start::Restart, 0.0).precursor_peak = Some(0.05);
        assert!(decision_lines(&decide(&runs)).iter().any(|l| l.contains("decision 4") && l.contains("signal-like")));
    }

    #[test]
    fn the_rule_is_in_the_summary_before_any_data() {
        assert!(summary_line(&decide(&[])).contains("RULE (written before the data)"));
        assert!(RXZ_RULE.contains("13-15") && RXZ_RULE.contains("every planned trial") && RXZ_RULE.contains("never lowers the bound"));
    }

    fn synthetic_capture(per: i64, planned: i64, zeros: usize) -> Vec<(i64, Iq)> {
        let n = (span_ticks(per) / per) as usize + 50;
        (0..n)
            .map(|i| {
                let tick = planned + i as i64 * per;
                let phase = 2.0 * std::f64::consts::PI * iii::RESTART_TONE_HZ * (tick as f64 / MCR as f64);
                let amplitude = if i < zeros { 0.0 } else { 0.01 };
                (tick, [(amplitude * phase.cos()) as f32, (amplitude * phase.sin()) as f32])
            })
            .collect()
    }

    #[test]
    fn the_analysed_span_holds_whole_samples_and_whole_tone_windows_so_odd_decimations_can_be_complete() {
        for per in [1i64, 2, 3, 6, 12, 24, 60, 125, 255] {
            let span = span_ticks(per);
            assert!(span % per == 0 && span % (25 * per) == 0 && span >= 2 * TICKS_PER_MS && span >= 4_000 * per, "D {per}: {span}");
        }
        assert_eq!(span_ticks(3), 400_050, "2 ms is not a whole number of samples at D = 3");
        assert_eq!(span_ticks(200), 800_000);
        for decim in RATE_DECIMATIONS {
            let per = decim as i64;
            let planned = 123_456 * per;
            let capture = synthetic_capture(per, planned, 5);
            let analysis = analyse_span(&capture, Some(planned), true, planned, per);
            assert!(analysis.covered, "D {decim}: a complete capture is covered ({} samples of {})", analysis.samples.len(), span_ticks(per) / per);
            assert!(analysis.tone_hz.is_some_and(|hz| (hz - iii::RESTART_TONE_HZ).abs() < n47::TONE_TOLERANCE_HZ), "D {decim}: {:?}", analysis.tone_hz);
            assert_eq!(analysis.fill.map(|fill| fill.zeros), Some(5), "D {decim}");
            let trial = RestartTrial {
                drained: true,
                asked: MCR as f64 / per as f64,
                applied: MCR as f64 / per as f64,
                planned,
                first_tick: Some(planned),
                covered: analysis.covered,
                tone_hz: analysis.tone_hz,
                fill: analysis.fill,
            };
            assert_eq!(iii::incomplete_reason(&trial, iii::RESTART_TONE_HZ, n47::TONE_TOLERANCE_HZ), None, "D {decim}");
        }
        let per = 3;
        let mut capture = synthetic_capture(per, 300, 5);
        assert!(!analyse_span(&capture, Some(300), false, 300, per).covered, "a read that saw an overflow is not covered");
        assert!(!analyse_span(&capture, None, true, 300, per).covered, "no first block");
        let whole = capture.clone();
        capture.remove(1_000);
        assert!(!analyse_span(&capture, Some(300), true, 300, per).covered, "a missing sample");
        let short = &whole[..(span_ticks(per) / per) as usize - 10];
        assert!(!analyse_span(short, Some(300), true, 300, per).covered, "a contiguous capture that ends before the span does");
    }

    fn report(code: TxCode, tick: Option<i64>, channel: usize) -> Seen {
        Seen { code, tick, channel, at_ms: 1 }
    }

    fn skip_at(kind: SkipKind, round: usize, onset_offset: i64, ack_offset: i64) -> SkipTrial {
        SkipTrial {
            kind,
            round,
            rate: 1e6,
            b_start: 10_000,
            b_end: 10_000 + 1_000 * 200,
            per: 200,
            onset: Some(10_000 + onset_offset),
            length: Some(200_000),
            t3_ack: (kind == SkipKind::OpenThenFlush).then_some(9_000),
            t3_sent: (kind == SkipKind::OpenThenFlush).then_some(4),
            reports: vec![report(TxCode::BurstAck, Some(10_000 + 200_000 + ack_offset), 0)],
            rx_other: Vec::new(),
            contiguous: true,
            spans_b: true,
            error: None,
        }
    }

    fn skip(kind: SkipKind, onset_offset: i64, ack_offset: i64) -> SkipTrial {
        skip_at(kind, 0, onset_offset, ack_offset)
    }

    /// One rate's cells in the probe's shape: `controls` (one more than the others), then the two skipping cells, onset offsets given.
    fn skip_cells(controls: &[i64], open: &[i64], closed: &[i64]) -> Vec<(f64, SkipKind, Vec<SkipTrial>)> {
        let make = |kind, offsets: &[i64]| (1e6, kind, offsets.iter().enumerate().map(|(round, offset)| skip_at(kind, round, *offset, 0)).collect::<Vec<_>>());
        vec![make(SkipKind::OpenThenFlush, controls), make(SkipKind::OpenThenSkip, open), make(SkipKind::ClosedThenSkip, closed)]
    }

    fn sc16(units: &[f64]) -> Vec<(i64, Iq)> {
        units.iter().enumerate().map(|(i, m)| (i as i64 * 200, [(*m * SC16_LSB) as f32, 0.0])).collect()
    }

    #[test]
    fn the_onset_is_the_first_sample_at_half_the_top_and_needs_a_burst_well_above_the_noise() {
        let per = 200;
        let mut samples: Vec<(i64, Iq)> = (0..100).map(|i| (i * per, [0.0005, 0.0])).collect();
        samples.extend((100..130).map(|i| (i * per, [0.01, 0.0])));
        samples.extend((130..200).map(|i| (i * per, [0.0005, 0.0])));
        assert_eq!(burst_onset(&samples, per), Some((100 * per, 30 * per)));
        let flat: Vec<(i64, Iq)> = (0..200).map(|i| (i * per, [0.0005, 0.0])).collect();
        assert_eq!(burst_onset(&flat, per), None);
        let weak: Vec<(i64, Iq)> = (0..200).map(|i| (i * per, if (100..130).contains(&i) { [0.002, 0.0] } else { [0.0005, 0.0] })).collect();
        assert_eq!(burst_onset(&weak, per), None, "a burst only 4 times the noise is not one");
        let mut edge = samples.clone();
        edge[99].1 = [0.006, 0.0];
        edge[100].1 = [0.004, 0.0];
        assert_eq!(burst_onset(&edge, per), Some((99 * per, 31 * per)), "the first sample at half the top (0.005) is the 0.006 one, not the 0.004");
        assert_eq!(burst_onset(&samples[..10], per), None);
        let quiet: Vec<(i64, Iq)> = (0..200).map(|i| (i * per, if (100..130).contains(&i) { [0.01, 0.0] } else { [0.0, 0.0] })).collect();
        assert_eq!(burst_onset(&quiet, per), Some((100 * per, 30 * per)), "a burst over an exactly quiet capture is one");
    }

    #[test]
    fn a_quantised_noise_with_a_zero_median_is_not_a_burst() {
        let per = 200;
        let sparse_lsb: Vec<f64> = (0..200).map(|i| if i % 8 == 3 { 1.0 } else { 0.0 }).collect();
        assert_eq!(burst_onset(&sc16(&sparse_lsb), per), None, "median 0, 90th percentile one LSB: below eight times the floor");
        let spikes: Vec<f64> = (0..200).map(|i| if i % 7 == 3 { 20.0 } else { 0.0 }).collect();
        assert_eq!(burst_onset(&sc16(&spikes), per), None, "sparse spikes above the floor do not fill the stretch they span");
        let step: Vec<f64> = (0..200).map(|i| if i >= 120 { 1.0 } else { 0.0 }).collect();
        assert_eq!(burst_onset(&sc16(&step), per), None, "a one-LSB step over an exactly quiet capture has a zero median but not a floor below one LSB");
        let burst: Vec<f64> = (0..200)
            .map(|i| {
                if (100..130).contains(&i) {
                    40.0
                } else if i % 8 == 3 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        assert_eq!(burst_onset(&sc16(&burst), per), Some((100 * per, 30 * per)), "a burst of 40 LSB over the same noise is one");
    }

    #[test]
    fn b_on_time_with_its_ack_passes_and_a_skipping_trial_needs_a_reference() {
        assert!(matches!(skip_trial_verdict(&skip(SkipKind::OpenThenFlush, 3_000, 0), None), Verdict::Pass(_)));
        assert!(matches!(skip_trial_verdict(&skip(SkipKind::OpenThenSkip, 3_100, 1), Some(Ok(3_000))), Verdict::Pass(_)));
        let unusable = skip_trial_verdict(&skip(SkipKind::OpenThenSkip, 3_100, 1), Some(Err("control 1 has no measured onset".to_owned())));
        assert!(matches!(unusable, Verdict::Inconclusive(text) if text.contains("no stable bracketing control")));
        let mut no_ack = skip(SkipKind::OpenThenSkip, 3_000, 0);
        no_ack.reports.clear();
        assert!(matches!(skip_trial_verdict(&no_ack, Some(Err(String::new()))), Verdict::Inconclusive(_)), "a difference is not attributed without a stable control");
    }

    #[test]
    fn a_mistimed_missing_or_extra_b_fails_a_skipping_trial() {
        let c = || Some(Ok(3_000));
        assert!(matches!(skip_trial_verdict(&skip(SkipKind::OpenThenSkip, 3_000 + SKIP_TOLERANCE_TICKS, 0), c()), Verdict::Pass(_)), "the tolerance is inclusive");
        assert!(matches!(skip_trial_verdict(&skip(SkipKind::OpenThenSkip, 3_000 + SKIP_TOLERANCE_TICKS + 1, 0), c()), Verdict::Fail(text) if text.contains("onset")));
        assert!(matches!(skip_trial_verdict(&skip(SkipKind::OpenThenSkip, 3_000 - SKIP_TOLERANCE_TICKS - 1, 0), c()), Verdict::Fail(_)));
        let mut no_ack = skip(SkipKind::OpenThenSkip, 3_000, 0);
        no_ack.reports.clear();
        assert!(matches!(skip_trial_verdict(&no_ack, c()), Verdict::Fail(text) if text.contains("no BurstAck")));
        let mut late_ack = skip(SkipKind::OpenThenSkip, 3_000, 5);
        assert!(matches!(skip_trial_verdict(&late_ack, c()), Verdict::Fail(text) if text.contains("+5 ticks")));
        late_ack.reports = vec![report(TxCode::BurstAck, Some(10_000 + 200_000), 0), report(TxCode::BurstAck, Some(10_000 + 200_000), 0)];
        assert!(matches!(skip_trial_verdict(&late_ack, c()), Verdict::Fail(text) if text.contains("2 BurstAcks")));
        let mut error = skip(SkipKind::OpenThenSkip, 3_000, 0);
        error.reports.push(report(TxCode::TimeError, Some(1), 0));
        assert!(matches!(skip_trial_verdict(&error, c()), Verdict::Fail(text) if text.contains("TimeError")));
        let mut gone = skip(SkipKind::OpenThenSkip, 3_000, 0);
        (gone.onset, gone.length) = (None, None);
        assert!(matches!(skip_trial_verdict(&gone, c()), Verdict::Fail(text) if text.contains("not in the capture")));
        let mut edge = skip(SkipKind::OpenThenSkip, 3_000, 0);
        edge.length = Some(200_000 - SKIP_TOLERANCE_TICKS);
        assert!(matches!(skip_trial_verdict(&edge, c()), Verdict::Pass(_)), "the length tolerance is inclusive");
        let mut short = skip(SkipKind::OpenThenSkip, 3_000, 0);
        short.length = Some(100_000);
        assert!(matches!(skip_trial_verdict(&short, c()), Verdict::Fail(text) if text.contains("lasts")));
        let mut failed = skip(SkipKind::OpenThenSkip, 3_000, 0);
        failed.error = Some("tx_send (A): boom".to_owned());
        assert!(matches!(skip_trial_verdict(&failed, c()), Verdict::Fail(text) if text.starts_with(TRANSPORT) && text.contains("boom")));
        let mut control = skip(SkipKind::OpenThenFlush, 3_000, 0);
        control.t3_ack = None;
        assert!(matches!(skip_trial_verdict(&control, None), Verdict::Inconclusive(text) if text.contains("T3 was not acknowledged")));
    }

    #[test]
    fn a_receive_overflow_or_a_gap_makes_a_trial_inconclusive_never_a_t3_effect() {
        let c = || Some(Ok(3_000));
        let mut overflow = skip(SkipKind::OpenThenSkip, 90_000, 0);
        overflow.rx_other = vec!["Overflow { first_tick: 1 }".to_owned()];
        assert!(matches!(skip_trial_verdict(&overflow, c()), Verdict::Inconclusive(text) if text.contains("not clean") && text.contains("Overflow")));
        let mut gap = skip(SkipKind::OpenThenSkip, 90_000, 0);
        gap.contiguous = false;
        (gap.onset, gap.length) = (None, None);
        assert!(matches!(skip_trial_verdict(&gap, c()), Verdict::Inconclusive(text) if text.contains("contiguous false")), "a B lost to a gap is not 'not in the capture'");
        let mut dropped_ack = skip(SkipKind::OpenThenSkip, 3_000, 0);
        dropped_ack.rx_other = vec!["Overflow".to_owned()];
        dropped_ack.reports.clear();
        assert!(matches!(skip_trial_verdict(&dropped_ack, c()), Verdict::Fail(text) if text.contains("no BurstAck")), "the ACK check does not read the capture");
        let rates = [1e6];
        let mut cells = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        cells[1].2[1] = {
            let mut trial = skip_at(SkipKind::OpenThenSkip, 1, 90_000, 0);
            trial.rx_other = vec!["Overflow".to_owned()];
            trial
        };
        let line = skip_decision(&cells, &rates, 3);
        assert!(line.contains("INCONCLUSIVE") && !line.contains("T3 is needed"), "{line}");
        assert!(skip_report(&cells).iter().any(|(claim, v)| claim == "n75 a open_then_skip 1 MS/s trial 1" && matches!(v, Verdict::Inconclusive(_))));
        let mut control = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        control[0].2[2].contiguous = false;
        assert!(skip_decision(&control, &rates, 3).contains("not clean"));
        let trace = overflow.trace();
        assert_eq!((trace["rx_other"][0].as_str(), trace["contiguous"].as_bool()), (Some("Overflow { first_tick: 1 }"), Some(true)));
    }

    #[test]
    fn a_capture_that_does_not_span_b_s_window_is_inconclusive_never_a_burst_that_did_not_play() {
        let (per, b_start, b_end) = (200, 100_000, 300_000);
        let len = b_end - b_start;
        let from_to = |first: i64, last: i64| vec![(first, [0.0, 0.0]), (last, [0.0, 0.0])];
        assert!(spans_b(&from_to(b_start - len, b_end + len * 3 / 2 - per), per, b_start, b_end), "both limits are inclusive");
        assert!(!spans_b(&from_to(b_start - len + per, b_end + len * 3 / 2), per, b_start, b_end), "it starts one sample late");
        assert!(!spans_b(&from_to(b_start - len, b_end + len * 3 / 2 - 2 * per), per, b_start, b_end), "it ends one sample early");
        assert!(!spans_b(&[], per, b_start, b_end), "a capture of timeouts only");
        let c = || Some(Ok(3_000));
        let mut short = skip(SkipKind::OpenThenSkip, 3_000, 0);
        short.spans_b = false;
        (short.onset, short.length) = (None, None);
        assert!(matches!(skip_trial_verdict(&short, c()), Verdict::Inconclusive(text) if text.contains("not clean") && text.contains("spans B's window false")), "not 'burst B is not in the capture'");
        let mut control = skip(SkipKind::OpenThenFlush, 3_000, 0);
        control.spans_b = false;
        assert!(matches!(skip_trial_verdict(&control, None), Verdict::Inconclusive(_)));
        let rates = [1e6];
        let mut cells = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        cells[1].2[1] = {
            let mut trial = skip_at(SkipKind::OpenThenSkip, 1, 3_000, 0);
            (trial.onset, trial.length, trial.spans_b) = (None, None, false);
            trial
        };
        let line = skip_decision(&cells, &rates, 3);
        assert!(line.contains("INCONCLUSIVE") && !line.contains("T3 is needed"), "{line}");
        assert!(skip_report(&cells).iter().any(|(claim, v)| claim == "n75 a open_then_skip 1 MS/s trial 1" && matches!(v, Verdict::Inconclusive(_))));
        assert_eq!(short.trace()["spans_b"].as_bool(), Some(false));
    }

    #[test]
    fn a_trial_records_the_outcomes_the_gap_and_the_span_of_its_read() {
        let per = 200;
        let (b_start, b_end) = (100_000, 140_000);
        let ticks = |range: std::ops::Range<i64>| -> Vec<(i64, Iq)> { range.map(|i| (b_start - 40_000 + i * per, if (200..400).contains(&i) { [0.4, 0.0] } else { [0.0005, 0.0] })).collect() };
        let mut trial = skip(SkipKind::OpenThenSkip, 0, 0);
        (trial.b_start, trial.b_end, trial.per) = (b_start, b_end, per);
        trial.rx_other = vec!["stale".to_owned()];
        trial.contiguous = false;
        trial.spans_b = false;
        let clean = Read { samples: ticks(0..800), other: vec![(1, "Timeout".to_owned())], ..Read::default() };
        trial.record_capture(&clean);
        assert!(trial.rx_other.is_empty() && trial.contiguous && trial.spans_b && trial.clean(), "{:?}", trial.trace());
        assert_eq!(trial.onset, Some(b_start - 40_000 + 200 * per), "the onset is the capture's first sample at half the top");
        let mut gap = ticks(0..800);
        gap.remove(300);
        let dirty = Read { samples: gap, other: vec![(1, "Timeout".to_owned()), (2, "Overflow { out_of_sequence: false }".to_owned())], ..Read::default() };
        trial.record_capture(&dirty);
        assert_eq!(trial.rx_other, ["Overflow { out_of_sequence: false }"], "a timeout is not an outcome");
        assert!(!trial.contiguous && trial.spans_b && !trial.clean());
        trial.record_capture(&Read { samples: ticks(0..400), ..Read::default() });
        assert!(trial.contiguous && !trial.spans_b && !trial.clean(), "a capture that stops before B's window ends");
        trial.record_capture(&Read::default());
        assert!(trial.contiguous && !trial.spans_b && trial.onset.is_none() && trial.length.is_none());
    }

    #[test]
    fn a_dirty_control_is_not_read_for_the_stability_or_the_bracket() {
        let controls = |offsets: &[i64]| skip_cells(offsets, &[], &[]).remove(0).2;
        let mut c = controls(&[3_000, 3_000, 3_000, 3_000]);
        c[1].onset = Some(10_000 + 90_000);
        c[1].rx_other = vec!["Overflow".to_owned()];
        assert_eq!(control_stable(&c), Ok(()), "an overflowed control's wild onset does not spread the rest");
        c[2].contiguous = false;
        assert_eq!(control_stable(&c), Ok(()), "two clean controls are enough to read");
        c[3].spans_b = false;
        assert!(control_stable(&c).unwrap_err().contains("1 of 4"), "one clean control is no spread");
        let mut c = controls(&[3_000, 3_100, 3_200]);
        c[1].rx_other = vec!["Overflow".to_owned()];
        assert!(bracket_reference(&c, 0).unwrap_err().contains("control 1 has no clean"), "the bracket's later control is dirty");
        assert!(bracket_reference(&c, 1).unwrap_err().contains("control 1 has no clean"), "the bracket's earlier control is dirty");
        c[1].rx_other.clear();
        c[1].spans_b = false;
        assert!(bracket_reference(&c, 0).is_err());
        c[1].spans_b = true;
        assert_eq!(bracket_reference(&c, 0), Ok(3_050));
        let rates = [1e6];
        let mut cells = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        cells[0].2[1].rx_other = vec!["Overflow".to_owned()];
        cells[1].2[0] = skip_at(SkipKind::OpenThenSkip, 0, 90_000, 0);
        let line = skip_decision(&cells, &rates, 3);
        assert!(line.contains("INCONCLUSIVE") && !line.contains("T3 is needed"), "{line}");
        assert!(skip_report(&cells).iter().any(|(claim, v)| claim == "n75 a open_then_skip 1 MS/s trial 0" && matches!(v, Verdict::Inconclusive(_))), "a bracket with a dirty control judges nothing");
    }

    #[test]
    fn the_probes_refuse_a_channel_other_than_slot_a() {
        assert_slot_a_only(0);
        for channel in [1, 2] {
            let refused = std::panic::catch_unwind(|| assert_slot_a_only(channel));
            let payload = refused.expect_err("another channel is refused");
            let text = payload.downcast_ref::<String>().cloned().unwrap_or_default();
            assert!(text.contains("RF safety") && text.contains(&format!("EZSDR_D9_CHANNEL={channel}")), "{text}");
        }
    }

    #[test]
    fn the_capture_is_contiguous_when_every_sample_is_one_spacing_from_the_next() {
        let at = |ticks: &[i64]| ticks.iter().map(|tick| (*tick, [0.0, 0.0])).collect::<Vec<(i64, Iq)>>();
        assert!(contiguous(&at(&[0, 200, 400]), 200) && contiguous(&[], 200));
        assert!(!contiguous(&at(&[0, 200, 600]), 200), "a gap");
        assert!(!contiguous(&at(&[0, 200, 200]), 200), "an overlap");
    }

    #[test]
    fn only_acks_at_or_after_b_are_b_s_and_an_ack_without_a_tick_is_nobody_s() {
        let c = || Some(Ok(3_000));
        let mut old_and_b = skip(SkipKind::ClosedThenSkip, 3_000, 0);
        old_and_b.reports.insert(0, report(TxCode::BurstAck, Some(10_000 - 1), 0));
        assert!(matches!(skip_trial_verdict(&old_and_b, c()), Verdict::Pass(text) if text.contains("1 earlier ACK") && text.contains("not counted")), "A's old ACK is not a second ACK for B");
        assert_eq!(old_and_b.old_acks(), vec![9_999]);
        assert_eq!(old_and_b.trace()["old_acks"][0].as_i64(), Some(9_999));
        let mut only_old = skip(SkipKind::ClosedThenSkip, 3_000, 0);
        only_old.reports = vec![report(TxCode::BurstAck, Some(10_000 - 1), 0)];
        assert!(matches!(skip_trial_verdict(&only_old, c()), Verdict::Fail(text) if text.contains("no BurstAck for burst B")), "an old ACK is not B's");
        let mut at_start = skip(SkipKind::ClosedThenSkip, 3_000, 0);
        at_start.reports.insert(0, report(TxCode::BurstAck, Some(10_000), 0));
        assert!(matches!(skip_trial_verdict(&at_start, c()), Verdict::Fail(text) if text.contains("2 BurstAcks")), "the boundary tick is B's");
        let mut no_tick = skip(SkipKind::OpenThenSkip, 3_000, 0);
        no_tick.reports.push(report(TxCode::BurstAck, None, 0));
        assert!(matches!(skip_trial_verdict(&no_tick, c()), Verdict::Inconclusive(text) if text.contains("without a tick")));
    }

    #[test]
    fn a_control_is_stable_when_its_offsets_spread_at_most_half_the_tolerance_and_a_bracket_gives_the_midpoint() {
        let controls = |offsets: &[i64]| skip_cells(offsets, &[], &[]).remove(0).2;
        assert_eq!(control_stable(&controls(&[3_000, 3_100, 3_200])), Ok(()));
        assert!(control_stable(&controls(&[3_000, 3_100, 3_000 + CONTROL_SPREAD_TICKS + 1])).unwrap_err().contains("spread 201 ticks"));
        assert!(control_stable(&controls(&[3_000])).is_err(), "one control is no spread");
        let mut missing = controls(&[3_000, 3_000, 3_000]);
        missing[1].onset = None;
        assert!(control_stable(&missing).unwrap_err().contains("2 of 3"));
        let c = controls(&[3_000, 3_100, 3_000 + CONTROL_SPREAD_TICKS + 101]);
        assert_eq!(bracket_reference(&c, 0), Ok(3_050));
        assert!(bracket_reference(&c, 1).unwrap_err().contains("apart"));
        assert!(bracket_reference(&c, 2).unwrap_err().contains("control 3"), "the last round has no later control");
        assert_eq!(bracket_reference(&controls(&[3_000, 3_000 + CONTROL_SPREAD_TICKS]), 0), Ok(3_100), "the spread limit is inclusive");
    }

    #[test]
    fn the_skip_decision_reads_the_control_first_and_a_failed_skip_against_a_stable_control_means_t3_is_needed() {
        let rates = [1e6];
        let clean = || skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        assert!(skip_decision(&clean(), &rates, 3).contains("hazard is not shown"));
        let mut cells = clean();
        cells[1].2[1] = skip_at(SkipKind::OpenThenSkip, 1, 90_000, 0);
        assert!(skip_decision(&cells, &rates, 3).contains("after an unclosed burst") && skip_decision(&cells, &rates, 3).contains("T3 is needed"));
        cells[2].2[0] = skip_at(SkipKind::ClosedThenSkip, 0, 90_000, 0);
        assert!(skip_decision(&cells, &rates, 3).contains("also after a closed burst"));
        let mut only_closed = clean();
        only_closed[2].2[2] = skip_at(SkipKind::ClosedThenSkip, 2, 90_000, 0);
        let line = skip_decision(&only_closed, &rates, 3);
        assert!(line.contains("even after a closed burst") && !line.contains("T3 is needed"), "{line}");
        let mut unclean = clean();
        unclean[0].2[0].reports.clear();
        assert!(skip_decision(&unclean, &rates, 3).contains("not clean"), "{}", skip_decision(&unclean, &rates, 3));
        assert!(matches!(skip_report(&unclean)[0].1, Verdict::Fail(_)));
        assert_eq!(skip_report(&clean()).len(), 4 + 3 + 3 + 1, "every trial and the control's stability");
    }

    #[test]
    fn an_unstable_control_cannot_establish_the_skip_hazard() {
        let rates = [1e6];
        let mut controls = vec![0; 5];
        controls.extend([10_000; 5]);
        let cells = skip_cells(&controls, &[0; 9], &[0; 9]);
        let line = skip_decision(&cells, &rates, 9);
        assert!(line.contains("INCONCLUSIVE") && line.contains("not stable") && line.contains("spread 10000 ticks") && !line.contains("T3 is needed"), "{line}");
        let verdicts = skip_report(&cells);
        assert!(
            verdicts.iter().filter(|(claim, _)| claim.contains("open_then_skip")).all(|(_, v)| matches!(v, Verdict::Inconclusive(_))),
            "no skipping trial is judged against an unstable control: {verdicts:?}"
        );
        assert!(verdicts.iter().any(|(claim, v)| claim.contains("control stability") && matches!(v, Verdict::Inconclusive(_))));
        let drifting = skip_cells(&[0, 100, 200, 300], &[0, 0, 0], &[0; 3]);
        assert!(skip_decision(&drifting, &rates, 3).contains("not stable"), "a drift past the spread rule is unstable too");
    }

    #[test]
    fn a_skipping_trial_is_judged_against_the_controls_around_its_own_round() {
        let rates = [1e6];
        let line = skip_decision(&skip_cells(&[3_000, 3_100, 3_200], &[3_050, 3_550], &[3_050, 3_150]), &rates, 2);
        assert!(line.contains("hazard is not shown"), "round 1 lies between the controls at 3100 and 3200 (midpoint 3150): {line}");
        let line = skip_decision(&skip_cells(&[3_000, 3_100, 3_200], &[3_050, 3_551], &[3_050, 3_150]), &rates, 2);
        assert!(line.contains("T3 is needed"), "one tick past the tolerance from that midpoint: {line}");
    }

    #[test]
    fn the_skip_decision_needs_the_planned_trials_and_no_failed_call() {
        let rates = [1e6];
        let line = skip_decision(&skip_cells(&[3_000; 3], &[3_000; 3], &[3_000; 3]), &rates, 3);
        assert!(line.contains("incomplete") && line.contains("open_then_flush 1 MS/s has 3 of 4"), "{line}");
        assert!(skip_decision(&skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]), &[1e6, 200e6], 3).contains("incomplete"), "a rate that was not run");
        let mut cells = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        cells[1].2[0].error = Some("tx_open: boom".to_owned());
        let line = skip_decision(&cells, &rates, 3);
        assert!(line.contains("INCONCLUSIVE") && line.contains("failed device call") && !line.contains("T3 is needed"), "{line}");
        let mut cells = skip_cells(&[3_000; 4], &[3_000; 3], &[3_000; 3]);
        cells[1].2[0].onset = None;
        assert!(skip_decision(&cells, &rates, 3).contains("T3 is needed"), "a B that is missing after a skip, against a stable control, is a difference");
    }

    fn routing(when: AEnds, reports: Vec<Seen>) -> RoutingTrial {
        RoutingTrial { when, rate: 1e6, per: 200, a_end: 5_000_000, closed_at: 1, reopened_at: 2, t3_at: 3, t3_sent: Some(4), reports, error: None }
    }

    #[test]
    fn the_new_streamer_sees_one_ack_that_is_t3s_and_not_a_s() {
        assert!(matches!(routing_verdict(&routing(AEnds::AfterReopen, vec![report(TxCode::BurstAck, Some(5_001_000), 0)])), Verdict::Pass(text) if text.contains("no foreign ACK observed")));
        assert!(matches!(routing_verdict(&routing(AEnds::AfterReopen, vec![])), Verdict::Inconclusive(_)));
        let only_a = routing_verdict(&routing(AEnds::AfterReopen, vec![report(TxCode::BurstAck, Some(5_000_001), 0)]));
        assert!(matches!(only_a, Verdict::Fail(text) if text.contains("T3's own is missing") && text.starts_with(AT_A_END)));
        let both = routing_verdict(&routing(AEnds::AfterReopen, vec![report(TxCode::BurstAck, Some(5_000_001), 0), report(TxCode::BurstAck, Some(5_001_000), 0)]));
        assert!(matches!(both, Verdict::Fail(text) if text.contains("an old burst's ACK reached it") && text.starts_with(AT_A_END)));
        let twice = routing_verdict(&routing(AEnds::AfterReopen, vec![report(TxCode::BurstAck, Some(10), 0), report(TxCode::BurstAck, Some(11), 0)]));
        assert!(matches!(twice, Verdict::Fail(text) if text.contains("2 ACKs") && !text.contains("old burst") && !text.starts_with(AT_A_END)));
        let no_tick = routing_verdict(&routing(AEnds::AfterReopen, vec![report(TxCode::BurstAck, None, 0)]));
        assert!(matches!(no_tick, Verdict::Inconclusive(text) if text.contains("without a tick")), "an ACK with no tick cannot pass");
        let before_t3 = routing_verdict(&routing(AEnds::BeforeClose, vec![report(TxCode::BurstAck, Some(2), 0)]));
        assert!(matches!(before_t3, Verdict::Inconclusive(text) if text.contains("cannot be attributed")), "an ACK older than T3 and not at A's end is nobody's");
        let mut short = routing(AEnds::BeforeClose, vec![report(TxCode::BurstAck, Some(10), 0)]);
        short.t3_sent = Some(0);
        assert!(matches!(routing_verdict(&short), Verdict::Inconclusive(_)));
        short.t3_sent = Some(4);
        short.error = Some("boom".to_owned());
        assert!(matches!(routing_verdict(&short), Verdict::Fail(text) if text.starts_with(TRANSPORT)));
    }

    #[test]
    fn the_acks_are_counted_by_channel_position() {
        let reports = vec![report(TxCode::BurstAck, Some(1), 0), report(TxCode::Underflow, None, 0), report(TxCode::BurstAck, Some(2), 1), report(TxCode::BurstAck, Some(3), 1)];
        assert_eq!(acks_by_position(&reports), BTreeMap::from([(0, 1), (1, 2)]));
        assert_eq!(other_codes(&reports), ["Underflow"]);
    }

    fn late(reports: Vec<Seen>) -> LateTrial {
        LateTrial { rate: 1e6, t3_sent: Some(4), t3_at_device: 10, reports, error: None }
    }

    #[test]
    fn a_late_ack_still_there_is_retention_and_an_absent_one_is_not_a_loss() {
        assert!(matches!(late_verdict(&late(vec![report(TxCode::BurstAck, Some(11), 0)])), Verdict::Pass(text) if text.contains("retained")));
        assert!(matches!(late_verdict(&late(vec![])), Verdict::Inconclusive(text) if text.contains("never generated") && text.contains("absence alone does not prove expiry")));
        assert!(matches!(late_verdict(&late(vec![report(TxCode::BurstAck, Some(11), 0), report(TxCode::BurstAck, Some(12), 0)])), Verdict::Fail(text) if text.contains("2 ACKs")));
        let mut short = late(vec![report(TxCode::BurstAck, Some(11), 0)]);
        short.t3_sent = Some(2);
        assert!(matches!(late_verdict(&short), Verdict::Inconclusive(_)));
        assert!(matches!(late_verdict(&late(vec![report(TxCode::BurstAck, None, 0)])), Verdict::Inconclusive(text) if text.contains("without a tick")), "an ACK with no tick is not T3's");
        assert!(matches!(late_verdict(&late(vec![report(TxCode::BurstAck, Some(9), 0)])), Verdict::Inconclusive(text) if text.contains("before T3's send")), "an ACK older than the send is not T3's");
        assert!(matches!(late_verdict(&late(vec![report(TxCode::BurstAck, Some(10), 0)])), Verdict::Pass(_)), "an ACK at the send tick counts");
        let mut failed = late(vec![]);
        failed.error = Some("tx_open: boom".to_owned());
        assert!(matches!(late_verdict(&failed), Verdict::Fail(text) if text.starts_with(TRANSPORT)), "a failed call is a transport failure, not a lost ACK");
    }

    #[test]
    fn the_routing_decision_reads_every_planned_trial_and_never_says_acks_are_streamer_owned() {
        let pass = || Verdict::Pass(String::new());
        let group = |prefix: &str, n: usize, verdict: &dyn Fn() -> Verdict| (0..n).map(|i| (format!("n75 {prefix} t{i}"), verdict())).collect::<Vec<_>>();
        let rates = [1e6];
        let all = |b: Vec<(String, Verdict)>, c: Vec<(String, Verdict)>| b.into_iter().chain(c).collect::<Vec<_>>();
        let lines = routing_decision(&all(group("b", 4, &pass), group("c", 2, &pass)), &rates, 2);
        assert!(lines[0].contains("no foreign ACK observed") && lines[0].contains("does not prove") && !lines[0].contains("are streamer-owned"), "{lines:?}");
        assert!(lines[1].contains("retention demonstrated"), "{lines:?}");
        let failed = |v: Verdict| {
            let mut b = group("b", 4, &pass);
            b[1].1 = v;
            routing_decision(&all(b, group("c", 2, &pass)), &rates, 2)
        };
        assert!(failed(Verdict::Fail(format!("{AT_A_END}: only ACK")))[0].contains("F1's routing is real"));
        let extra = failed(Verdict::Fail("2 ACKs on the new streamer [10, 11] for one T3 (A ends at 5)".to_owned()))[0].clone();
        assert!(extra.contains("no cause is observed") && !extra.contains("routing is real") && !extra.contains("old burst"), "{extra}");
        assert!(failed(Verdict::Fail(format!("{TRANSPORT}: boom")))[0].contains("INCONCLUSIVE") && failed(Verdict::Fail(format!("{TRANSPORT}: boom")))[0].contains("failed device call"));
        assert!(failed(Verdict::Inconclusive(String::new()))[0].contains("INCONCLUSIVE: 3 of 4 trials passed"));
        let missing_acks = routing_decision(&all(group("b", 4, &pass), group("c", 2, &|| Verdict::Inconclusive("none".to_owned()))), &rates, 2);
        assert!(missing_acks[1].contains("INCONCLUSIVE") && missing_acks[1].contains("absence alone does not prove expiry") && !missing_acks[1].contains("lost"), "{missing_acks:?}");
        let short = routing_decision(&all(group("b", 3, &pass), group("c", 2, &pass)), &rates, 2);
        assert!(short[0].contains("INCONCLUSIVE: 3 of 4 planned"), "{short:?}");
        assert!(routing_decision(&[], &rates, 2)[1].contains("0 of 2 planned"));
    }

    #[test]
    fn two_channels_need_an_obx_or_ubx_in_both_slots() {
        let names = |a: &str, b: Option<&str>| vec![Some(a.to_owned()), b.map(str::to_owned)];
        assert!(two_channel_runnable(&names("UBX TX", Some("UBX TX"))).is_ok());
        assert!(two_channel_runnable(&names("OBX TX", Some("UBX TX"))).is_ok());
        assert!(two_channel_runnable(&names("UBX TX", Some("CBX-120 TX"))).unwrap_err().contains("SAFE_TX_HZ"), "the bench's #2: a CBX in slot B");
        assert!(two_channel_runnable(&names("OBX TX", Some("Unknown (0xffff) - 0"))).unwrap_err().contains("empty slot"), "the bench's #1: slot B empty");
        assert!(two_channel_runnable(&names("OBX TX", None)).unwrap_err().contains("one-channel"));
        assert!(two_channel_runnable(&[]).unwrap_err().contains("no transmit channel"));
        assert!(two_channel_runnable(&[None, None]).is_err());
    }

    #[test]
    fn the_two_channel_cells_transmit_on_slot_b_only_with_the_owner_s_attenuator_confirmation() {
        let names = |a: &str, b: &str| vec![Some(a.to_owned()), Some(b.to_owned())];
        assert!(two_channel_gate(&names("UBX TX", "OBX TX"), true).is_ok());
        let unconfirmed = two_channel_gate(&names("UBX TX", "OBX TX"), false).unwrap_err();
        assert!(unconfirmed.contains(SLOT_B_CONFIRMED) && unconfirmed.contains("not confirmed"), "{unconfirmed}");
        assert!(two_channel_gate(&names("UBX TX", "CBX-120 TX"), true).unwrap_err().contains("SAFE_TX_HZ"), "a confirmation does not make a CBX runnable");
        assert!(two_channel_gate(&names("UBX TX", "CBX-120 TX"), false).unwrap_err().contains("SAFE_TX_HZ"), "the board reason comes first");
    }

    fn flush(positions: usize, sent: Option<usize>, within: &[usize], late: &[usize]) -> FlushTrial {
        FlushTrial {
            positions,
            open_ms: 3.0,
            flush_ms: 0.5,
            sent,
            sent_at_device: 1_000,
            reports: within.iter().map(|p| report(TxCode::BurstAck, Some(1_001), *p)).collect(),
            within: within.to_vec(),
            late: late.to_vec(),
            error: None,
        }
    }

    #[test]
    fn a_flush_passes_only_when_whole_and_every_position_acked_by_the_deadline() {
        assert!(matches!(flush_trial_verdict(&flush(1, Some(4), &[0], &[])), Verdict::Pass(_)));
        assert!(matches!(flush_trial_verdict(&flush(2, Some(4), &[0, 1], &[])), Verdict::Pass(_)));
        assert!(matches!(flush_trial_verdict(&flush(2, Some(4), &[0], &[])), Verdict::Fail(text) if text.contains("missing on positions [1]")));
        assert!(matches!(flush_trial_verdict(&flush(2, Some(4), &[0], &[1])), Verdict::Fail(text) if text.contains("[1] of them came late")));
        assert!(matches!(flush_trial_verdict(&flush(1, Some(2), &[0], &[])), Verdict::Fail(text) if text.contains("Some(2) of 4")));
        assert!(matches!(flush_trial_verdict(&flush(1, None, &[0], &[])), Verdict::Fail(_)));
        let mut no_tick = flush(1, Some(4), &[0], &[]);
        no_tick.reports = vec![report(TxCode::BurstAck, None, 0)];
        assert!(matches!(flush_trial_verdict(&no_tick), Verdict::Inconclusive(text) if text.contains("without a tick")), "an ACK with no tick is not this flush's");
        let mut old = flush(2, Some(4), &[0, 1], &[]);
        old.reports[1] = report(TxCode::BurstAck, Some(999), 1);
        assert!(matches!(flush_trial_verdict(&old), Verdict::Inconclusive(text) if text.contains("before T3's send")), "an ACK older than the send is not this flush's");
        old.reports[1] = report(TxCode::BurstAck, Some(1_000), 1);
        assert!(matches!(flush_trial_verdict(&old), Verdict::Pass(_)));
        let mut error = flush(1, Some(4), &[0], &[]);
        error.error = Some("tx_open: boom".to_owned());
        assert!(matches!(flush_trial_verdict(&error), Verdict::Fail(text) if text.contains("boom")));
    }

    #[test]
    fn the_cell_line_counts_trials_with_every_position_acked_and_gives_the_tick_offsets() {
        let trials = vec![flush(2, Some(4), &[0, 1], &[]), flush(2, Some(4), &[0], &[]), flush(2, Some(4), &[0, 1], &[])];
        let line = flush_cell_line(1e6, 2, &trials);
        assert!(line.contains("ACKed 2 of 3") && line.contains("Some((1, 1, 1))") && line.contains("open ms Some((3.0, 3.0, 3.0))"), "{line}");
    }

    /// Every claim the sweep makes on a device that can run the cells: `runnable` positions get their trials, a two-channel cell that cannot run its marker.
    fn flush_claims(rates: &[f64], trials: usize, two_channels: bool) -> Vec<(String, Verdict)> {
        let mut claims = Vec::new();
        for positions in [1usize, 2] {
            for &rate in rates {
                if positions == 2 && !two_channels {
                    claims.push((flush_claim(rate, 2, None), Verdict::Inconclusive("not runnable: a CBX".to_owned())));
                } else {
                    claims.extend((0..trials).map(|index| (flush_claim(rate, positions, Some(index)), Verdict::Pass(String::new()))));
                }
            }
        }
        claims
    }

    #[test]
    fn the_open_flush_decision_enables_only_with_every_planned_cell_and_the_two_channel_cells_run_or_waived() {
        let rates = [1e6, 2e6];
        let full = || flush_claims(&rates, 3, true);
        assert!(open_flush_decision(&full(), &rates, 3, false).contains("enable (3)"));
        let one_channel_only = flush_claims(&rates, 3, false);
        let line = open_flush_decision(&one_channel_only, &rates, 3, false);
        assert!(line.contains("INCONCLUSIVE") && line.contains("NOT RUNNABLE") && line.contains("1 MS/s x 2 channels") && line.contains(TWO_CHANNEL_WAIVER) && !line.contains("enable (3)"), "{line}");
        let waived = open_flush_decision(&one_channel_only, &rates, 3, true);
        assert!(waived.contains("enable (3)") && waived.contains("the owner waived"), "{waived}");
        let mut missed = full();
        missed[1].1 = Verdict::Fail(String::new());
        assert!(open_flush_decision(&missed, &rates, 3, false).contains("ask the owner again") && open_flush_decision(&missed, &rates, 3, true).contains("ask the owner again"));
        let mut short = full();
        short.retain(|(claim, _)| claim != &flush_claim(2e6, 1, Some(2)));
        let line = open_flush_decision(&short, &rates, 3, true);
        assert!(line.contains("INCONCLUSIVE") && line.contains("2 MS/s x 1 channel(s)"), "{line}");
        let mut unclear = full();
        unclear[1].1 = Verdict::Inconclusive("an ACK without a tick".to_owned());
        let line = open_flush_decision(&unclear, &rates, 3, true);
        assert!(line.contains("INCONCLUSIVE") && line.contains("n75 ofs 1 MS/s x 1 channel(s) trial 1") && !line.contains("enable (3)"), "a trial without a verdict does not enable: {line}");
        assert!(open_flush_decision(&[], &rates, 3, true).contains("INCONCLUSIVE"));
        assert!(open_flush_decision(&flush_claims(&rates, 3, false)[..3], &rates, 3, true).contains("INCONCLUSIVE"), "a cell that never ran is not waived");
    }

    #[test]
    fn the_counts_line_and_the_spread_helpers() {
        let verdicts = [Verdict::Pass(String::new()), Verdict::Fail(String::new()), Verdict::Pass(String::new()), Verdict::Inconclusive(String::new())];
        assert_eq!(cell_counts(verdicts.iter()), "2 PASS, 1 FAIL, 1 INCONCLUSIVE of 4");
        assert_eq!(spread(vec![3, 1, 2]), Some((1, 2, 3)));
        assert_eq!(spread::<i64>(vec![]), None);
    }

    // ---- the PN cell (`hw_rxzeros_pn`)

    const T0: i64 = 777 * 5;

    /// The tick PN sample 0 is transmitted at, a whole number of samples.
    fn t0_of(per: i64) -> i64 {
        777 * per
    }

    fn gain_of(sample: Iq, gain: (f32, f32)) -> Iq {
        [sample[0] * gain.0 - sample[1] * gain.1, sample[0] * gain.1 + sample[1] * gain.0]
    }

    /// `n` samples from tick `from`, each the PN delayed by `delay` samples times `gain`, the first `zeros` of them exact zeros.
    fn pn_capture(period: &[Iq], per: i64, from: i64, n: usize, delay: i64, gain: (f32, f32), zeros: usize) -> Vec<(i64, Iq)> {
        let t0 = t0_of(per);
        (0..n)
            .map(|k| {
                let tick = from + k as i64 * per;
                (tick, if k < zeros { [0.0, 0.0] } else { gain_of(pn_at(period, (tick - t0) / per - delay), gain) })
            })
            .collect()
    }

    /// Like `pn_capture`, through a channel that is the sum of `taps` (delay in samples, real gain): a filter, not one delay.
    fn pn_filtered(period: &[Iq], per: i64, from: i64, n: usize, taps: &[(i64, f32)], zeros: usize) -> Vec<(i64, Iq)> {
        let t0 = t0_of(per);
        (0..n)
            .map(|k| {
                let tick = from + k as i64 * per;
                let mut sample = [0.0f32, 0.0];
                for (delay, gain) in taps {
                    let p = pn_at(period, (tick - t0) / per - delay);
                    sample = [sample[0] + gain * p[0], sample[1] + gain * p[1]];
                }
                (tick, if k < zeros { [0.0, 0.0] } else { sample })
            })
            .collect()
    }

    #[test]
    fn a_channel_that_filters_the_pn_does_not_turn_an_unshifted_restart_into_neither() {
        // y[n] = 0.6 PN[n-44] + 0.4 PN[n-45]: both captures see the same filter, the restart only loses its first 8 samples. The residual is taken
        // against the continuous capture of that same channel, so the post-zero samples match it and the restart is on time.
        let period = pn_period();
        let per = 5;
        let taps = [(44, 0.6), (45, 0.4)];
        let (wa, t1) = (T0 + 50_000 * per, T0 + 90_000 * per + 7 * per);
        let cont = pn_filtered(&period, per, wa, PN_CONT_SPAN, &taps, 0);
        let measure = pn_analyse(&period, T0, per, &cont, &pn_filtered(&period, per, t1, PN_SPAN, &taps, 8));
        assert_eq!((measure.z, measure.cont.d, measure.restart.d), (8, Some(44), Some(44)), "{measure:?}");
        assert!(measure.res_gd.is_some_and(|r| r < 1e-3) && measure.res_shift.is_some_and(|r| r > PN_RES_MATCH), "{measure:?}");
        assert_eq!(pn_outcome(&measure), PnOutcome::GroupDelay);
        let late = [(52, 0.6), (53, 0.4)];
        let measure = pn_analyse(&period, T0, per, &cont, &pn_filtered(&period, per, t1, PN_SPAN, &late, 8));
        assert!(measure.res_shift.is_some_and(|r| r < 1e-3) && measure.res_gd.is_some_and(|r| r > PN_RES_MATCH), "{measure:?}");
        assert_eq!(pn_outcome(&measure), PnOutcome::Shifted);
    }

    fn corr(d: Option<i64>) -> PnCorr {
        PnCorr { status: if d.is_some() { observations::CorrelationStatus::Found } else { observations::CorrelationStatus::NoSignal }, d, ratio: 30.0 }
    }

    fn measure(z: usize, d_cont: Option<i64>, d_restart: Option<i64>, res_gd: Option<f64>) -> PnMeasure {
        PnMeasure { z, cont: corr(d_cont), restart: corr(d_restart), res_gd, res_shift: None }
    }

    #[test]
    fn the_pn_locator_reads_the_delay_of_a_capture_whatever_its_gain_and_tick_grid() {
        let period = pn_period();
        for per in [1i64, 5, 200] {
            for delay in [-30, 0, 44, 700, 2_000] {
                let capture = pn_capture(&period, per, t0_of(per) + 90_000 * per, PN_SPAN, delay, (0.004, -0.003), 0);
                let found = locate_pn(&capture, &period, t0_of(per), per);
                assert_eq!((found.status, found.d), (observations::CorrelationStatus::Found, Some(delay)), "per {per} delay {delay}: {found:?}");
            }
        }
        let silence: Vec<(i64, Iq)> = (0..PN_SPAN).map(|k| (T0 + k as i64 * 5, [0.0, 0.0])).collect();
        assert_eq!(locate_pn(&silence, &period, T0, 5).d, None);
        assert_eq!(locate_pn(&[], &period, T0, 5).d, None);
    }

    #[test]
    fn a_restart_whose_zeros_replace_samples_is_group_delay_and_one_that_arrives_late_is_shifted() {
        let period = pn_period();
        let per = 5;
        let (wa, t1) = (T0 + 50_000 * per, T0 + 90_000 * per);
        let cont = pn_capture(&period, per, wa, PN_CONT_SPAN, 44, (0.004, -0.003), 0);
        let analyse = |z: usize, restart_delay: i64| pn_analyse(&period, T0, per, &cont, &pn_capture(&period, per, t1, PN_SPAN, restart_delay, (-0.003, 0.002), z));
        let on_time = analyse(8, 44);
        assert_eq!((on_time.z, on_time.cont.d, on_time.restart.d, on_time.delta()), (8, Some(44), Some(44), Some(0)));
        assert!(on_time.res_gd.is_some_and(|r| r < 1e-3) && on_time.res_shift.is_some_and(|r| r > PN_RES_MATCH), "{on_time:?}");
        assert_eq!(pn_outcome(&on_time), PnOutcome::GroupDelay);
        let late = analyse(8, 52);
        assert_eq!((late.restart.d, late.delta()), (Some(52), Some(8)));
        assert!(late.res_shift.is_some_and(|r| r < 1e-3) && late.res_gd.is_some_and(|r| r > PN_RES_MATCH), "{late:?}");
        assert_eq!(pn_outcome(&late), PnOutcome::Shifted);
        assert_eq!(pn_outcome(&analyse(8, 47)), PnOutcome::Neither, "3 samples late with 8 zeros is neither reading");
        assert_eq!(pn_outcome(&analyse(2, 44)), PnOutcome::Inseparable);
        assert_eq!(pn_outcome(&analyse(0, 44)), PnOutcome::Inseparable);
        let silent = pn_analyse(&period, T0, per, &cont, &pn_capture(&period, per, t1, PN_SPAN, 44, (1.0, 0.0), PN_SPAN));
        assert_eq!((silent.z, silent.restart.d, pn_outcome(&silent)), (PN_SPAN, None, PnOutcome::Unmeasured));
        assert_eq!(pn_outcome(&pn_analyse(&period, T0, per, &[], &cont)), PnOutcome::Unmeasured, "no continuous capture");
    }

    #[test]
    fn the_residual_fits_a_complex_gain_to_the_continuous_capture_at_the_same_phase_and_a_wrong_shift_does_not_fit() {
        let period = pn_period();
        let cont = pn_capture(&period, 5, T0 + 4_000 * 5, PN_CONT_SPAN, 44, (0.004, -0.003), 0);
        // A restart capture of the same channel at another phase of the period than the continuous one's start.
        let restart = pn_capture(&period, 5, T0 + 30_000 * 5, 400, 44, (-0.002, 0.001), 0);
        assert!(pn_residual(&restart, &cont, T0, 5, 0).is_some_and(|r| r < 1e-4));
        assert!(pn_residual(&restart, &cont, T0, 5, 1).is_some_and(|r| r > PN_RES_MATCH));
        let late = pn_capture(&period, 5, T0 + 30_000 * 5, 400, 47, (-0.002, 0.001), 0);
        assert!(pn_residual(&late, &cont, T0, 5, 3).is_some_and(|r| r < 1e-4), "shifted by 3 fits the continuous capture 3 samples earlier");
        assert!(pn_residual(&late, &cont, T0, 5, 0).is_some_and(|r| r > PN_RES_MATCH));
        assert_eq!(pn_residual(&restart[..15], &cont, T0, 5, 0), None, "too few samples");
        assert_eq!(pn_residual(&restart, &cont[..PN_PERIOD - 1], T0, 5, 0), None, "less than one period of the continuous capture");
        let zeros = pn_capture(&period, 5, T0, 400, 44, (1.0, 0.0), 400);
        assert_eq!(pn_residual(&zeros, &cont, T0, 5, 0), None, "no energy");
        let long = pn_capture(&period, 5, T0 + 30_000 * 5, PN_RES_SAMPLES + 100, 44, (1.0, 0.0), 0);
        let mut broken = long.clone();
        broken[PN_RES_SAMPLES + 50].1 = [9.0, 9.0];
        assert_eq!(pn_residual(&long, &cont, T0, 5, 0), pn_residual(&broken, &cont, T0, 5, 0), "only the first {PN_RES_SAMPLES} samples count");
    }

    #[test]
    fn the_outcome_rule_has_its_boundaries_at_the_tolerance_the_zero_count_and_the_residual_limit() {
        let outcome = |z, delta: i64, res: Option<f64>| pn_outcome(&measure(z, Some(44), Some(44 + delta), res));
        assert_eq!(outcome(PN_MIN_ZEROS - 1, 0, Some(0.0)), PnOutcome::Inseparable);
        assert_eq!(outcome(PN_MIN_ZEROS, 0, Some(0.0)), PnOutcome::GroupDelay);
        assert_eq!((PN_MIN_ZEROS, PN_TOL), (3, 1));
        for delta in [-1, 0, 1] {
            assert_eq!(outcome(8, delta, Some(0.1)), PnOutcome::GroupDelay, "delta {delta}");
        }
        for delta in [-2, 2] {
            assert_eq!(outcome(8, delta, Some(0.1)), PnOutcome::Neither, "delta {delta}");
        }
        assert_eq!(outcome(8, 0, Some(PN_RES_MATCH)), PnOutcome::GroupDelay);
        assert_eq!(outcome(8, 0, Some(PN_RES_MATCH + 0.01)), PnOutcome::Neither, "the samples do not match the prediction");
        assert_eq!(outcome(8, 0, None), PnOutcome::Neither, "no residual is no match");
        for delta in [7, 8, 9] {
            assert_eq!(outcome(8, delta, Some(0.9)), PnOutcome::Shifted, "delta {delta}");
        }
        for delta in [6, 10] {
            assert_eq!(outcome(8, delta, Some(0.9)), PnOutcome::Neither, "delta {delta}");
        }
        assert_eq!(outcome(3, 2, Some(0.9)), PnOutcome::Shifted);
        assert_eq!(pn_outcome(&measure(8, None, Some(44), Some(0.0))), PnOutcome::Unmeasured);
        assert_eq!(pn_outcome(&measure(8, Some(44), None, Some(0.0))), PnOutcome::Unmeasured);
    }

    fn pn_trial_of(z: usize, delta: i64) -> PnTrial {
        PnTrial { warmup: false, error: None, incomplete: None, misplaced: false, measure: Some(measure(z, Some(44), Some(44 + delta), Some(if delta.abs() <= 1 { 0.05 } else { 0.9 }))) }
    }

    fn pn_cell() -> PnCell {
        PnCell { decim: 200, trials: 10, settle_ms: PN_SETTLE_MS }
    }

    #[test]
    fn a_cell_fails_only_for_infrastructure_and_is_inconclusive_below_its_planned_trials() {
        let cell = pn_cell();
        let ten = || -> Vec<PnTrial> { (0..10).map(|_| pn_trial_of(14, 0)).collect() };
        let verdict = |trials: &[PnTrial]| pn_cell_verdict(&cell, &pn_stats(trials));
        assert!(matches!(verdict(&ten()), Verdict::Pass(text) if text.contains("10 of 10") && text.contains("group_delay")));
        let mut shifted = ten();
        shifted.iter_mut().for_each(|t| *t = pn_trial_of(14, 14));
        assert!(matches!(verdict(&shifted), Verdict::Pass(_)), "an outcome is a fact, not a defect");
        let mut trials = ten();
        trials[2].error = Some("rx_start: boom".to_owned());
        assert!(matches!(verdict(&trials), Verdict::Fail(text) if text.contains("failed device call")));
        let mut trials = ten();
        trials[2].misplaced = true;
        assert!(matches!(verdict(&trials), Verdict::Fail(text) if text.contains("planned tick")));
        let mut trials = ten();
        trials[2].measure = Some(measure(14, None, None, None));
        assert!(matches!(verdict(&trials), Verdict::Fail(text) if text.contains("without a correlation peak")));
        let mut trials = ten();
        trials[2].measure = Some(measure(14, None, None, None));
        trials[2].incomplete = Some("a gap".to_owned());
        assert!(matches!(verdict(&trials), Verdict::Inconclusive(text) if text.contains("9 of 10") && text.contains("10 needed")), "a capture that does not count has no peak to blame");
        let mut trials = ten();
        trials[0].warmup = true;
        trials[0].error = Some("the warm-up fails".to_owned());
        assert!(matches!(verdict(&trials), Verdict::Inconclusive(text) if text.contains("9 of 9")), "a warm-up is not counted: nine trials remain");
        assert_eq!(pn_stats(&trials).errors, 0);
    }

    #[test]
    fn a_cell_decides_only_when_every_trial_is_complete_and_they_agree() {
        let cell = pn_cell();
        let kind = |trials: &[PnTrial]| pn_cell_kind(&cell, &pn_stats(trials));
        let of = |z, delta| -> Vec<PnTrial> { (0..10).map(|_| pn_trial_of(z, delta)).collect() };
        assert_eq!(kind(&of(14, 0)), PnKind::Outcome(PnOutcome::GroupDelay));
        assert_eq!(kind(&of(14, 14)), PnKind::Outcome(PnOutcome::Shifted));
        assert_eq!(kind(&of(14, 5)), PnKind::Outcome(PnOutcome::Neither));
        assert_eq!(kind(&of(0, 0)), PnKind::Outcome(PnOutcome::Inseparable));
        let mut mixed = of(14, 0);
        mixed[4] = pn_trial_of(14, 14);
        assert_eq!(kind(&mixed), PnKind::Mixed);
        assert!(pn_cell_line(&cell, &pn_stats(&mixed)).contains("MIXED"));
        let mut short = of(14, 0);
        short[4].incomplete = Some("a gap".to_owned());
        assert_eq!(kind(&short), PnKind::Undecided);
        assert!(pn_cell_line(&cell, &pn_stats(&short)).contains("INCONCLUSIVE (9 of 10 complete"));
        assert!(pn_cell_line(&cell, &pn_stats(&of(14, 0))).contains("zeros = group delay"));
        assert!(pn_cell_line(&cell, &pn_stats(&of(14, 14))).contains("shifted"));
        assert!(pn_cell_line(&cell, &pn_stats(&of(0, 0))).contains("inseparable"));
    }

    #[test]
    fn the_decision_over_the_rates_ignores_the_inseparable_ones_and_needs_all_of_them_decided() {
        let runs = |kinds: &[(usize, i64)]| -> Vec<(PnCell, PnStats)> { kinds.iter().map(|&(z, delta)| (pn_cell(), pn_stats(&(0..10).map(|_| pn_trial_of(z, delta)).collect::<Vec<_>>()))).collect() };
        let last = |kinds: &[(usize, i64)]| pn_decision_lines(&runs(kinds)).pop().unwrap();
        assert!(last(&[(14, 0), (10, 0), (0, 0)]).contains("zeros = group delay at the 2 rate(s)"), "{}", last(&[(14, 0), (10, 0), (0, 0)]));
        assert!(last(&[(14, 14), (10, 10), (0, 0)]).contains("shifted at the 2 rate(s)"));
        assert!(last(&[(14, 0), (10, 10)]).contains("neither, or different"));
        assert!(last(&[(14, 5), (10, 5)]).contains("neither, or different"));
        assert!(last(&[(0, 0), (1, 0)]).contains("inseparable at every rate"));
        assert!(last(&[]).contains("INCONCLUSIVE"));
        let mut undecided = runs(&[(14, 0), (10, 0)]);
        undecided[1].1.complete = 9;
        assert!(pn_decision_lines(&undecided).pop().unwrap().contains("INCONCLUSIVE"));
        assert_eq!(pn_decision_lines(&runs(&[(14, 0), (10, 0)])).len(), 3, "a line a rate and the decision");
    }

    #[test]
    fn the_pn_rule_names_its_thresholds_and_the_cell_covers_the_eight_rates_of_the_restart_fill_probe() {
        for text in ["+-1", "0.35", "256", "z < 3", "10 trials", "1 s", "INSEPARABLE", "GROUP DELAY", "SHIFTED", "NEITHER"] {
            assert!(PNZ_RULE.contains(text), "{text}");
        }
        let cells = pn_cells();
        assert_eq!(cells.iter().map(|cell| cell.rate() / 1e6).collect::<Vec<_>>(), vec![1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0]);
        assert!(cells.iter().all(|cell| cell.trials == 10 && cell.settle_ms == 1_000));
        assert_eq!(cells.iter().map(|cell| iii::RESTART_RATES.contains(&cell.rate())).collect::<Vec<_>>(), vec![true; 8]);
        assert_eq!(pn_read_len(1e6), 2_000);
        assert_eq!(pn_read_len(25e6), 25_000);
        assert_eq!(pn_read_len(200e6), 50_000);
        let period = pn_period();
        assert!(period.len() == PN_PERIOD && period.iter().all(|s| f64::from(s[0]).hypot(f64::from(s[1])) <= 0.5));
    }
}
