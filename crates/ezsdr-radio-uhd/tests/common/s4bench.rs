//! Stage 4's bench tools for 2026-11-15 (`plan/phase8/bench.md`): S2 throughput repeats, T200 (the record suite's 200 MS/s transmit stage run
//! alone), S5 lead rungs and S4 overflow gap. Each runs the record's own measurement code with other plan values, prints one row per trial and
//! decision lines that state what was observed; a decision line says INCONCLUSIVE when the planned evidence is incomplete. A device failure is
//! an observation, not a test failure: a run fails only when the harness cannot go on (a group's error, a latched handle, a streamer left open).
//! The pure judgements are unit-tested at the bottom; the runners have FakeDevice rehearsals in `fake.rs`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ezsdr_radio_uhd::Device;
use serde_json::{Value as Json, json};

use super::measurement::{self, MeasurementPlan, OverflowTrialEvidence, Stage, StageResult, StreamPlan, TrialClass, env_count, trial_class_name};

fn msps(rate_hz: u64) -> String {
    if rate_hz % 1_000_000 == 0 { format!("{}msps", rate_hz / 1_000_000) } else { format!("{rate_hz}hz") }
}

/// Seconds from nanoseconds, to a tenth.
fn secs(ns: i64) -> String {
    format!("{:.1}", ns as f64 / 1e9)
}

fn ms(ns: u64) -> String {
    format!("{:.3}", ns as f64 / 1e6)
}

fn secs_list(values: impl Iterator<Item = i64>) -> String {
    let list: Vec<String> = values.map(secs).collect();
    if list.is_empty() { "none".to_owned() } else { format!("[{}] s", list.join(", ")) }
}

fn is_failure(class: TrialClass) -> bool {
    matches!(class, TrialClass::DeviceFailure | TrialClass::ProviderFailure)
}

/// A run's observations are a map; the decision code reads them as JSON.
fn result_json(row: &ezsdr_radio::measurement::Observation) -> Json {
    Json::Object(row.result.clone().into_iter().collect())
}

// ---------------------------------------------------------------- S2: throughput repeats (decides D1 and D2)

/// The rates S2 runs at: the rung below the grid's top and the top (RX and TX each, and the duplex pairs).
pub const S2_RATES: (u64, u64) = (100_000_000, 200_000_000);
/// The window of the amended TX reading of D2 (owner, 2026-11-13): "the device accepts and plays" over this long; how long it holds is a second figure.
pub const S2_TX_WINDOW_NS: i64 = 10_000_000_000;

/// The owner's rule a cell's result is read against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum S2Rule {
    None,
    /// D1: a failure of the top RX rate on a board → `rx_bytes_per_s` 4e8 (none → 8e8 stands for the board).
    D1,
    /// D2 (amended): the top TX rate is admitted when the device accepts and plays it over the window; how long it holds is an observation.
    D2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct S2Cell {
    pub id: String,
    pub rx_hz: Option<u64>,
    pub tx_hz: Option<u64>,
    pub trials: usize,
    pub rule: S2Rule,
}

impl S2Cell {
    pub fn stage(&self) -> Stage {
        let one = |rate_hz| vec![StreamPlan { channels: vec![0], rate_hz }];
        Stage { id: self.id.clone(), family: "s2", rx: self.rx_hz.map(one).unwrap_or_default(), tx: self.tx_hz.map(one).unwrap_or_default() }
    }
}

/// RX at the low and the high rate, TX at both, then the duplex pairs (RX high + TX low, RX low + TX low): `trials` Runs each, `duplex_trials` for a pair.
pub fn s2_cells(rates: (u64, u64), trials: usize, duplex_trials: usize) -> Vec<S2Cell> {
    let (low, high) = rates;
    let cell = |id: String, rx_hz, tx_hz, trials, rule| S2Cell { id, rx_hz, tx_hz, trials, rule };
    vec![
        cell(format!("rx-{}", msps(low)), Some(low), None, trials, S2Rule::None),
        cell(format!("rx-{}", msps(high)), Some(high), None, trials, S2Rule::D1),
        cell(format!("tx-{}", msps(low)), None, Some(low), trials, S2Rule::None),
        cell(format!("tx-{}", msps(high)), None, Some(high), trials, S2Rule::D2),
        cell(format!("duplex-rx{}-tx{}", msps(high), msps(low)), Some(high), Some(low), duplex_trials, S2Rule::None),
        cell(format!("duplex-rx{}-tx{}", msps(low), msps(low)), Some(low), Some(low), duplex_trials, S2Rule::None),
    ]
}

/// The cells named in `filter` (comma-separated ids), all of them when it is `None`; an unknown id is an error before any device call.
pub fn s2_select(cells: Vec<S2Cell>, filter: Option<&str>) -> Result<Vec<S2Cell>, String> {
    let Some(filter) = filter else { return Ok(cells) };
    let wanted: Vec<&str> = filter.split(',').map(str::trim).collect();
    if let Some(unknown) = wanted.iter().find(|id| !cells.iter().any(|cell| cell.id == **id)) {
        return Err(format!("EZSDR_S2_CELLS names an unknown cell {unknown:?}: the cells are {:?}", cells.iter().map(|cell| cell.id.as_str()).collect::<Vec<_>>()));
    }
    Ok(cells.into_iter().filter(|cell| wanted.contains(&cell.id.as_str())).collect())
}

/// The wall time the cells plan for their Runs.
pub fn s2_planned_seconds(cells: &[S2Cell], seconds: u64) -> u64 {
    cells.iter().map(|cell| cell.trials as u64 * (seconds + 10)).sum()
}

/// What one S2 Run showed, read from `run_stage`'s observation.
#[derive(Clone, Debug)]
pub struct S2Trial {
    pub class: TrialClass,
    pub rx_samples: Option<u64>,
    pub rx_expected: Option<u64>,
    pub tx_samples: Option<u64>,
    pub tx_minimum: Option<u64>,
    pub tx_rate_hz: Option<u64>,
    pub rx_overflow_events: u64,
    pub first_rx_overflow_ns: Option<i64>,
    pub tx_underflow_events: u64,
    pub first_tx_underflow_ns: Option<i64>,
    pub tx_underflow_untimed: u64,
    pub async_failures: Vec<String>,
    pub run_termination: Option<String>,
}

pub fn s2_trial(result: &StageResult) -> S2Trial {
    let observation = &result.observation;
    let streams: Vec<&Json> = observation["streams"].as_array().into_iter().flatten().collect();
    let stream = |direction: &str| streams.iter().find(|row| row["direction"] == direction).copied();
    let (rx, tx) = (stream("rx"), stream("tx"));
    S2Trial {
        class: result.class,
        rx_samples: rx.and_then(|row| row["actual_samples"].as_u64()),
        rx_expected: rx.and_then(|row| row["expected_samples"].as_u64()),
        tx_samples: tx.and_then(|row| row["actual_samples"].as_u64()),
        tx_minimum: tx.and_then(|row| row["minimum_samples"].as_u64()),
        tx_rate_hz: tx.and_then(|row| row["rate_hz"].as_u64()),
        rx_overflow_events: observation["rx_overflow_events"].as_u64().unwrap_or(0),
        first_rx_overflow_ns: observation["first_rx_overflow_ns_after_t0"].as_i64(),
        tx_underflow_events: observation["tx_underflow_events"].as_u64().unwrap_or(0),
        first_tx_underflow_ns: observation["first_tx_underflow_ns_after_t0"].as_i64(),
        tx_underflow_untimed: observation["tx_underflow_untimed_reports"].as_u64().unwrap_or(0),
        async_failures: tx.and_then(|row| row["async_failures"].as_array()).into_iter().flatten().filter_map(|code| code.as_str().map(str::to_owned)).collect(),
        run_termination: observation["run_termination"].as_str().map(str::to_owned),
    }
}

impl S2Trial {
    /// Whether the transmit stream played its first `window_ns` without a `TX_UNDERFLOW`: `Some(false)` when one came within it, `Some(true)` when none did and
    /// the device accepted the window's samples, `None` when that cannot be told (an underflow with no time, or too few samples accepted). An underflow the device
    /// reported without a time is dated at the instant the Provider read it, which is never before the true one: within the window it still shows a failure,
    /// after it the window is unknown.
    pub fn window_held(&self, window_ns: i64) -> Option<bool> {
        let rate = self.tx_rate_hz?;
        if self.first_tx_underflow_ns.is_some_and(|ns| ns <= window_ns) {
            return Some(false);
        }
        if self.tx_underflow_untimed > 0 || (self.tx_underflow_events > 0 && self.first_tx_underflow_ns.is_none()) {
            return None;
        }
        let needed = u128::from(rate) * window_ns as u128 / 1_000_000_000;
        self.tx_samples.filter(|accepted| u128::from(*accepted) >= needed).map(|_| true)
    }

    pub fn row(&self, cell: &str, index: usize) -> String {
        let json = json!({
            "cell": cell, "trial": index, "outcome": trial_class_name(self.class),
            "rx_samples": self.rx_samples, "rx_expected_samples": self.rx_expected, "tx_samples": self.tx_samples, "tx_minimum_samples": self.tx_minimum,
            "rx_overflow_events": self.rx_overflow_events, "first_rx_overflow_ns": self.first_rx_overflow_ns,
            "tx_underflow_events": self.tx_underflow_events, "first_tx_underflow_ns": self.first_tx_underflow_ns, "tx_underflow_untimed_reports": self.tx_underflow_untimed,
            "async_failures": self.async_failures, "run_termination": self.run_termination,
        });
        format!("S2 trial {json}")
    }
}

/// What S2 ran: each cell with its trials, and why it stopped early.
#[derive(Debug, Default)]
pub struct S2Run {
    pub cells: Vec<(S2Cell, Vec<S2Trial>)>,
    pub stopped: Option<String>,
}

/// Runs the cells' stages in order through `run_stage` (the record's own stage runner), printing a row per trial. Stops at the first harness problem: an `Err`
/// from the stage, a streamer left open, or `latched` naming a reason. The cells after it are left with fewer trials than planned.
pub fn s2_run(cells: &[S2Cell], mut run_stage: impl FnMut(&Stage) -> Result<StageResult, String>, mut latched: impl FnMut() -> Option<String>) -> S2Run {
    let mut run = S2Run::default();
    for cell in cells {
        let stage = cell.stage();
        let mut trials = Vec::new();
        for index in 0..cell.trials {
            if run.stopped.is_some() {
                break;
            }
            match run_stage(&stage) {
                Err(error) => run.stopped = Some(format!("{}: run_stage returned an error: {error}", cell.id)),
                Ok(result) => {
                    let trial = s2_trial(&result);
                    println!("{}", trial.row(&cell.id, index));
                    trials.push(trial);
                    if result.left_open {
                        run.stopped = Some(format!("{}: a streamer was left open", cell.id));
                    } else if let Some(reason) = latched() {
                        run.stopped = Some(format!("{}: the device handle is latched ({reason})", cell.id));
                    }
                }
            }
        }
        run.cells.push((cell.clone(), trials));
    }
    if let Some(reason) = &run.stopped {
        println!("S2 stop: {reason}; no later trial runs on this device: power-cycle the board and tell the owner");
    }
    run
}

fn first_times(trials: &[S2Trial], pick: impl Fn(&S2Trial) -> Option<i64>) -> String {
    secs_list(trials.iter().filter_map(pick))
}

/// The decision line of one cell. `seconds` is the Run length the cell was planned for.
pub fn s2_decision(cell: &S2Cell, trials: &[S2Trial], seconds: u64, stopped: Option<&str>) -> String {
    let (pass, fail) = (trials.iter().filter(|trial| trial.class == TrialClass::Success).count(), trials.iter().filter(|trial| is_failure(trial.class)).count());
    let inconclusive = trials.len() - pass - fail;
    let counts = format!("{pass} Success, {fail} failed, {inconclusive} inconclusive of {} run (planned {})", trials.len(), cell.trials);
    let mut facts = Vec::new();
    if cell.rx_hz.is_some() {
        let overflowed = trials.iter().filter(|trial| trial.rx_overflow_events > 0).count();
        facts.push(format!("RX_OVERFLOW in {overflowed} of {} trials (first at {})", trials.len(), first_times(trials, |trial| trial.first_rx_overflow_ns)));
    }
    if cell.tx_hz.is_some() {
        let held: Vec<Option<bool>> = trials.iter().map(|trial| trial.window_held(S2_TX_WINDOW_NS)).collect();
        let (yes, no) = (held.iter().filter(|held| **held == Some(true)).count(), held.iter().filter(|held| **held == Some(false)).count());
        facts.push(format!(
            "no TX_UNDERFLOW in the first {} s: {yes} of {} (underflow within it: {no}, not established: {}); whole {seconds} s Success: {pass} of {}; first TX_UNDERFLOW at {}",
            S2_TX_WINDOW_NS / 1_000_000_000,
            trials.len(),
            held.len() - yes - no,
            trials.len(),
            first_times(trials, |trial| trial.first_tx_underflow_ns)
        ));
        let untimed: u64 = trials.iter().map(|trial| trial.tx_underflow_untimed).sum();
        if untimed > 0 {
            facts.push(format!("{untimed} underflow reports had no device time (their events are dated when the Provider read them, so those first-underflow times are late)"));
        }
    }
    let facts = facts.join("; ");
    // A transmit trial whose window cannot be told (a Run shorter than the window, an underflow with no time) is not the planned evidence either.
    let window_unknown = cell.tx_hz.is_some() && trials.iter().any(|trial| trial.window_held(S2_TX_WINDOW_NS).is_none());
    let complete = trials.len() == cell.trials && inconclusive == 0 && !window_unknown;
    if !complete {
        let stop = stopped.map(|reason| format!("; stopped: {reason}")).unwrap_or_default();
        return format!("S2 decision {}: INCONCLUSIVE: {counts}{stop}; observed: {facts}", cell.id);
    }
    let rule = match cell.rule {
        S2Rule::None => String::new(),
        S2Rule::D1 if fail > 0 => "; D1 rule (a failure of the top RX rate on a board -> rx_bytes_per_s 4e8): fired on this board".to_owned(),
        S2Rule::D1 => "; D1 rule (a failure of the top RX rate on a board -> rx_bytes_per_s 4e8): not fired on this board (every board must be clean for 8e8)".to_owned(),
        S2Rule::D2 => "; D2 reading (admitted when the device accepts and plays the rate over the window; how long it holds is observed): see the first two figures".to_owned(),
    };
    format!("S2 decision {}: {counts}; {facts}{rule}", cell.id)
}

pub fn s2_decisions(run: &S2Run, seconds: u64) -> Vec<String> {
    run.cells.iter().map(|(cell, trials)| s2_decision(cell, trials, seconds, run.stopped.as_deref())).collect()
}

// ---------------------------------------------------------------- T200: the record suite's 200 MS/s transmit stage, run alone

/// T200's transmit rate on the bench.
pub const T200_RATE_HZ: u64 = 200_000_000;

#[derive(Clone, Debug)]
pub struct T200Run {
    pub class: TrialClass,
    pub actual_samples: Option<u64>,
    pub minimum_samples: Option<u64>,
    pub underflow_events: u64,
    pub first_underflow_ns: Option<i64>,
    pub async_failures: Vec<String>,
}

pub fn t200_run(observation: &Json) -> T200Run {
    let class = match observation["stage"]["trials"][0]["class"].as_str() {
        Some("Success") => TrialClass::Success,
        Some("DeviceFailure") => TrialClass::DeviceFailure,
        Some("ProviderFailure") => TrialClass::ProviderFailure,
        Some("AdmissionRejection") => TrialClass::AdmissionRejection,
        Some("Unattributable") => TrialClass::Unattributable,
        _ => TrialClass::Incomplete,
    };
    T200Run {
        class,
        actual_samples: observation["actual_samples"].as_u64(),
        minimum_samples: observation["minimum_samples"].as_u64(),
        underflow_events: observation["tx_underflow_events"].as_u64().unwrap_or(0),
        first_underflow_ns: observation["first_underflow_ns_after_burst_start"].as_i64(),
        async_failures: observation["async_failures"].as_array().into_iter().flatten().filter_map(|code| code.as_str().map(str::to_owned)).collect(),
    }
}

/// `runs` times `measure_tx_throughput` on `plan` (the record suite's TX stage: its Spec, its waveform, its judgement) at the plan's one transmit rate (200 MS/s on the
/// bench), a row per run. Stops at the first `Err`, or when `latched` names a reason. Returns the runs and why it stopped.
pub fn t200_runs(device: &Arc<dyn Device>, plan: &MeasurementPlan, runs: usize, mut latched: impl FnMut() -> Option<String>) -> (Vec<T200Run>, Option<String>) {
    let [rate_hz] = plan.tx_throughput_rates_hz[..] else {
        return (Vec::new(), Some(format!("T200 runs one transmit rate, the plan has {:?}", plan.tx_throughput_rates_hz)));
    };
    let name = format!("provider_tx_throughput_{rate_hz}");
    let mut done = Vec::new();
    for index in 0..runs {
        let group = match measurement::measure_tx_throughput(device.clone(), plan) {
            Ok(group) => group,
            Err(error) => return (done, Some(format!("run {index}: measure_tx_throughput returned an error: {error}"))),
        };
        let Some(row) = group.observations.iter().find(|row| row.name == name) else {
            return (done, Some(format!("run {index}: the group has no observation `{name}`")));
        };
        let run = t200_run(&result_json(row));
        println!(
            "T200 run {}",
            json!({ "run": index, "outcome": trial_class_name(run.class), "tx_samples": run.actual_samples, "minimum_samples": run.minimum_samples,
                "tx_underflow_events": run.underflow_events, "first_underflow_ns_after_burst_start": run.first_underflow_ns, "async_failures": run.async_failures })
        );
        done.push(run);
        if let Some(reason) = latched() {
            return (done, Some(format!("run {index}: the device handle is latched ({reason})")));
        }
    }
    (done, None)
}

pub fn t200_decision(runs: &[T200Run], planned: usize, plan: &MeasurementPlan, stopped: Option<&str>) -> String {
    let success = runs.iter().filter(|run| run.class == TrialClass::Success).count();
    let failed = runs.iter().filter(|run| is_failure(run.class)).count();
    let inconclusive = runs.len() - success - failed;
    let shape = format!(
        "record-suite shape, {} MS/s, {} s, waveform {} samples",
        plan.tx_throughput_rates_hz.first().map_or(0.0, |rate| *rate as f64 / 1e6),
        plan.throughput_seconds,
        plan.tx_waveform_samples
    );
    let underflows: Vec<u64> = runs.iter().map(|run| run.underflow_events).collect();
    let facts = format!(
        "{success} Success, {failed} failed, {inconclusive} inconclusive of {} run (planned {planned}); TX_UNDERFLOW events per run {underflows:?}; first at {}",
        runs.len(),
        secs_list(runs.iter().filter_map(|run| run.first_underflow_ns))
    );
    if runs.len() < planned || inconclusive > 0 {
        let stop = stopped.map(|reason| format!("; stopped: {reason}")).unwrap_or_default();
        return format!("T200 decision ({shape}): INCONCLUSIVE: {facts}{stop}");
    }
    format!("T200 decision ({shape}): {facts}")
}

// ---------------------------------------------------------------- S5: lead rungs

/// S5's plan: the record's own plan with the S5 rungs and rates where the environment names none.
pub fn s5_plan(values: &BTreeMap<String, String>) -> Result<MeasurementPlan, String> {
    let mut values = values.clone();
    for (name, default) in [
        ("EZSDR_MEASURE_RX_START_LEADS_NS", "3000000,4000000,5000000,7500000,10000000"),
        ("EZSDR_MEASURE_RX_START_RATES_HZ", "10000000,100000000"),
        ("EZSDR_MEASURE_TIMED_LEADS_NS", "3500000,4000000,4500000,5000000"),
        ("EZSDR_MEASURE_TIMED_LEAD_TX_RATES_HZ", "1000000,10000000,100000000"),
    ] {
        values.entry(name.to_owned()).or_insert_with(|| default.to_owned());
    }
    MeasurementPlan::from_env(&values)
}

/// One candidate lead of one condition: its trials by outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct LeadStage {
    pub condition: String,
    pub rung_ns: u64,
    pub success: usize,
    pub failed: usize,
    pub total: usize,
}

pub fn lead_stages(result: &Json) -> Vec<LeadStage> {
    result["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|stage| {
            let count = |class: &str| stage["classes"][class].as_u64().unwrap_or(0) as usize;
            LeadStage {
                condition: stage["condition"].as_str().unwrap_or("?").to_owned(),
                rung_ns: stage["candidate"].as_u64().unwrap_or(0),
                success: count("Success"),
                failed: count("ProviderFailure") + count("DeviceFailure"),
                total: stage["trials"].as_array().map_or(0, Vec::len),
            }
        })
        .collect()
}

/// A row per trial, and a line per stage, of a lead group's observation.
pub fn lead_rows(label: &str, result: &Json) -> Vec<String> {
    let mut lines = Vec::new();
    for stage in result["candidates"].as_array().into_iter().flatten() {
        for trial in stage["trials"].as_array().into_iter().flatten() {
            let mut row = trial.clone();
            row["condition"] = stage["condition"].clone();
            row["rung_ns"] = stage["candidate"].clone();
            lines.push(format!("S5 {label} trial {row}"));
        }
    }
    for stage in lead_stages(result) {
        lines.push(format!(
            "S5 {label} stage {} {} ms: {}/{} Success, {} failed, {} not counted",
            stage.condition,
            ms(stage.rung_ns),
            stage.success,
            stage.total,
            stage.failed,
            stage.total - stage.success - stage.failed
        ));
    }
    lines
}

/// The decision line of a lead group: per condition the smallest rung that is `planned_trials`/`planned_trials` Success there and at every larger rung, and
/// whether the rung `claim_ns` (the owner's decided value) is 100 % Success in every condition. INCONCLUSIVE when a planned stage or trial is missing.
pub fn lead_decision(label: &str, claim_ns: u64, stages: &[LeadStage], planned_stages: usize, planned_trials: usize) -> String {
    let clean = |stage: &LeadStage| stage.total >= planned_trials && stage.success == stage.total;
    let mut by_condition: BTreeMap<&str, Vec<&LeadStage>> = BTreeMap::new();
    for stage in stages {
        by_condition.entry(stage.condition.as_str()).or_default().push(stage);
    }
    let mut edges = Vec::new();
    let (mut not_clean, mut missing) = (Vec::new(), Vec::new());
    for (condition, rungs) in &mut by_condition {
        rungs.sort_by_key(|stage| stage.rung_ns);
        let edge = (0..rungs.len()).find(|index| rungs[*index..].iter().all(|stage| clean(stage))).map(|index| rungs[index].rung_ns);
        edges.push(format!("{condition}: {}", edge.map_or("no rung".to_owned(), |ns| format!("{} ms", ms(ns)))));
        match rungs.iter().find(|stage| stage.rung_ns == claim_ns) {
            Some(stage) if clean(stage) => {}
            Some(_) => not_clean.push(*condition),
            None => missing.push(*condition),
        }
    }
    let incomplete = stages.len() < planned_stages || stages.iter().any(|stage| stage.total < planned_trials);
    let facts = format!(
        "{} ms: every trial Success in {} of {} conditions{}; smallest rung clean from there up: {}",
        ms(claim_ns),
        by_condition.len() - not_clean.len() - missing.len(),
        by_condition.len(),
        if not_clean.is_empty() { String::new() } else { format!(" (not in: {})", not_clean.join(", ")) },
        edges.join("; ")
    );
    if incomplete || !missing.is_empty() || stages.is_empty() {
        return format!(
            "S5 decision {label}: INCONCLUSIVE: {} of {planned_stages} planned stages ran{}; observed: {facts}",
            stages.len(),
            if missing.is_empty() { String::new() } else { format!(" (no {} ms rung in: {})", ms(claim_ns), missing.join(", ")) }
        );
    }
    format!("S5 decision {label}: {facts}")
}

/// A generous wall bound for a whole S5 run (the command-lead pass and the receive start-lead pass at one and at two channels): a quarter second a trial.
pub fn s5_planned_seconds(plan: &MeasurementPlan) -> u64 {
    let command = plan.timed_command_leads_ns.len() * plan.timed_command_tx_rates_hz.len();
    let receive = plan.rx_start_leads_ns.len() * 2 * plan.rx_start_rates_hz.len();
    ((command + 2 * receive) * plan.step_trials) as u64 / 4
}

/// The command lead (S5, owner's D3: 5 ms) and the receive start lead (D4: 10 ms) the decision lines read.
pub const D3_LEAD_NS: u64 = 5_000_000;
pub const D4_LEAD_NS: u64 = 10_000_000;

/// The lines of one lead group (a row per trial, a line per stage, the decision line) and the group's own error, if it had one.
pub type LeadLines = (Vec<String>, Option<String>);

fn lead_pass(label: &str, claim_ns: u64, planned_stages: usize, plan: &MeasurementPlan, observation: &str, group: Result<measurement::MeasurementGroup, String>) -> LeadLines {
    let group = match group {
        Ok(group) => group,
        Err(error) => return (vec![format!("S5 decision {label}: INCONCLUSIVE: the group returned an error: {error}")], Some(format!("{label}: {error}"))),
    };
    let Some(row) = group.observations.iter().find(|row| row.name == observation) else {
        return (vec![format!("S5 decision {label}: INCONCLUSIVE: the group has no observation `{observation}`")], Some(format!("{label}: no observation `{observation}`")));
    };
    let result = result_json(row);
    if let Some(reason) = result["reason"].as_str().filter(|_| result["status"].as_str().is_some_and(|status| status.starts_with("skipped"))) {
        return (vec![format!("S5 decision {label}: INCONCLUSIVE: not runnable: {reason}")], None);
    }
    let mut lines = lead_rows(label, &result);
    lines.push(lead_decision(label, claim_ns, &lead_stages(&result), planned_stages, plan.step_trials));
    (lines, None)
}

/// S5's command-lead pass: `plan.timed_command_leads_ns` at each of `plan.timed_command_tx_rates_hz`, one TX channel.
pub fn s5_command_lead(device: &Arc<dyn Device>, plan: &MeasurementPlan) -> LeadLines {
    let planned = plan.timed_command_leads_ns.len() * plan.timed_command_tx_rates_hz.len();
    lead_pass("command-lead", D3_LEAD_NS, planned, plan, "provider_min_timed_command_lead", measurement::measure_min_timed_command_lead(device.clone(), plan))
}

/// S5's receive start-lead pass at `plan.rx_start_channels` channels: both conditions at each of `plan.rx_start_rates_hz`.
pub fn s5_rx_start_lead(device: &Arc<dyn Device>, plan: &MeasurementPlan) -> LeadLines {
    let planned = plan.rx_start_leads_ns.len() * 2 * plan.rx_start_rates_hz.len();
    let label = format!("rx-start-lead-{}ch", plan.rx_start_channels);
    lead_pass(&label, D4_LEAD_NS, planned, plan, "provider_rx_start_lead", measurement::measure_rx_start_lead(device.clone(), plan))
}

// ---------------------------------------------------------------- S4: overflow gap

/// The reader stall each rate's cell uses (the record's own inference from the 50 MB receive socket buffer: a stall longer than the buffer's fill time).
pub const S4_RATE_STALLS_MS: [(u64, u64); 6] = [(1_000_000, 20_000), (10_000_000, 2_000), (25_000_000, 1_000), (50_000_000, 500), (100_000_000, 500), (200_000_000, 500)];
/// The stalls of the stall-length sweep, at one rate.
pub const S4_STALL_SWEEP_MS: [u64; 3] = [100, 500, 2_000];
pub const S4_STALL_SWEEP_RATE_HZ: u64 = 100_000_000;
/// The owner's stability criterion (41 §8, 2026-10-27): (max - min) / max of the restart-after-resume times at a rate is at most this (here 1/2).
pub const S4_STABILITY_NUM_DEN: (u64, u64) = (1, 2);
/// D5's decided value for the overflow restart gap (owner, 2026-11-13): the maximum of the 2026-11-13 loaded run, 139.111 ms, rounded up.
pub const S4_D5_VALUE_NS: u64 = 140_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct S4Cell {
    pub rate_hz: u64,
    pub stall_ms: u64,
    pub trials: usize,
    /// Part of the stall-length sweep (a cell that is also a rate cell counts as a rate cell).
    pub sweep: bool,
}

/// The six rate cells, then the stall sweep's cells that are not already rate cells.
pub fn s4_cells(trials: usize, sweep_rate_hz: u64, sweep_stalls_ms: &[u64]) -> Vec<S4Cell> {
    let mut cells: Vec<S4Cell> = S4_RATE_STALLS_MS.iter().map(|(rate_hz, stall_ms)| S4Cell { rate_hz: *rate_hz, stall_ms: *stall_ms, trials, sweep: false }).collect();
    for stall_ms in sweep_stalls_ms {
        if !cells.iter().any(|cell| (cell.rate_hz, cell.stall_ms) == (sweep_rate_hz, *stall_ms)) {
            cells.push(S4Cell { rate_hz: sweep_rate_hz, stall_ms: *stall_ms, trials, sweep: true });
        }
    }
    cells
}

/// The wall time the cells plan for their Runs.
pub fn s4_planned_seconds(cells: &[S4Cell], after_ms: u64) -> u64 {
    cells.iter().map(|cell| cell.trials as u64 * (measurement::overflow_wait_ms(after_ms, cell.stall_ms).unwrap_or(0) / 1_000 + 5)).sum()
}

#[derive(Clone, Debug)]
pub struct S4Trial {
    pub class: TrialClass,
    pub overflow_count: usize,
    pub gap_ns: Option<u64>,
    pub restart_ns: Option<u64>,
    pub reason: Option<String>,
    pub run_termination: Option<String>,
}

impl S4Trial {
    pub fn of(evidence: &OverflowTrialEvidence, rejection: Option<String>) -> S4Trial {
        S4Trial {
            class: evidence.class,
            overflow_count: evidence.overflow_count,
            gap_ns: evidence.gap_ns,
            restart_ns: evidence.restart_after_resume.nanoseconds,
            reason: evidence.restart_after_resume.reason.clone(),
            run_termination: rejection,
        }
    }

    /// A trial that counts: one overflow, the whole gap read, and a restart-after-resume time.
    pub fn complete(&self) -> bool {
        self.class == TrialClass::Success && self.restart_ns.is_some()
    }

    pub fn row(&self, cell: &S4Cell, index: usize) -> String {
        let json = json!({
            "rate_hz": cell.rate_hz, "stall_ms": cell.stall_ms, "trial": index, "outcome": trial_class_name(self.class), "overflow_count": self.overflow_count,
            "gap_ns": self.gap_ns, "restart_after_resume_ns": self.restart_ns, "restart_after_resume_reason": self.reason, "run_termination": self.run_termination,
        });
        format!("S4 trial {json}")
    }
}

/// The smallest, the largest and the spread of a cell's restart-after-resume times.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stability {
    pub min: u64,
    pub max: u64,
}

impl Stability {
    pub fn of(values: &[u64]) -> Option<Stability> {
        Some(Stability { min: *values.iter().min()?, max: *values.iter().max()? })
    }

    /// (max - min) / max <= 1/2, in integers (a spread of exactly half meets it).
    pub fn met(&self) -> bool {
        (self.max - self.min) * S4_STABILITY_NUM_DEN.1 <= self.max * S4_STABILITY_NUM_DEN.0
    }

    pub fn ratio(&self) -> f64 {
        (self.max - self.min) as f64 / self.max as f64
    }
}

fn median(values: &[u64]) -> Option<u64> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied()
}

#[derive(Debug, Default)]
pub struct S4Run {
    pub cells: Vec<(S4Cell, Vec<S4Trial>)>,
    pub stopped: Option<String>,
}

/// Runs the cells' Runs in order (`overflow_trial`: the record's own overflow Run), printing a row per trial. Stops at the first `Err`, or when `latched` names a reason.
pub fn s4_run(device: &Arc<dyn Device>, cells: &[S4Cell], after_ms: u64, mut latched: impl FnMut() -> Option<String>) -> S4Run {
    let mut run = S4Run::default();
    for cell in cells {
        let mut trials = Vec::new();
        for index in 0..cell.trials {
            if run.stopped.is_some() {
                break;
            }
            match measurement::overflow_trial(device, cell.rate_hz, cell.stall_ms, after_ms) {
                Err(error) => run.stopped = Some(format!("{} MS/s stall {} ms: {error}", cell.rate_hz / 1_000_000, cell.stall_ms)),
                Ok((evidence, rejection)) => {
                    let trial = S4Trial::of(&evidence, rejection);
                    println!("{}", trial.row(cell, index));
                    trials.push(trial);
                    if let Some(reason) = latched() {
                        run.stopped = Some(format!("the device handle is latched ({reason})"));
                    }
                }
            }
        }
        run.cells.push((cell.clone(), trials));
    }
    if let Some(reason) = &run.stopped {
        println!("S4 stop: {reason}; no later trial runs on this device: power-cycle the board and tell the owner");
    }
    run
}

fn s4_cell_name(cell: &S4Cell) -> String {
    format!("{} MS/s stall {} ms", cell.rate_hz as f64 / 1e6, cell.stall_ms)
}

/// The decision line of one cell: the restart-after-resume spread against the owner's criterion. INCONCLUSIVE unless every planned trial is complete.
pub fn s4_cell_decision(cell: &S4Cell, trials: &[S4Trial], stopped: Option<&str>) -> String {
    let times: Vec<u64> = trials.iter().filter(|trial| trial.complete()).map(|trial| trial.restart_ns.unwrap()).collect();
    let stability = Stability::of(&times);
    let observed = stability.map_or("no complete trial".to_owned(), |s| {
        format!("restart-after-resume min {} ms, median {} ms, max {} ms, (max - min)/max = {:.3}", ms(s.min), ms(median(&times).unwrap()), ms(s.max), s.ratio())
    });
    let name = s4_cell_name(cell);
    if times.len() < cell.trials {
        let stop = stopped.map(|reason| format!("; stopped: {reason}")).unwrap_or_default();
        return format!("S4 decision {name}: INCONCLUSIVE: {} complete of {} planned trials{stop}; observed: {observed}", times.len(), cell.trials);
    }
    let met = if stability.is_some_and(|s| s.met()) { "met" } else { "not met" };
    format!("S4 decision {name}: {observed}; owner's criterion (max - min)/max <= 0.5: {met} ({} of {} trials)", times.len(), cell.trials)
}

/// The decision lines of the run: every cell, then D5's bound over the rate cells, then the stall-length figures of the cells at `sweep_rate_hz` (the rate the sweep ran at).
pub fn s4_decisions(run: &S4Run, sweep_rate_hz: u64) -> Vec<String> {
    let mut lines: Vec<String> = run.cells.iter().map(|(cell, trials)| s4_cell_decision(cell, trials, run.stopped.as_deref())).collect();
    let rate_cells: Vec<_> = run.cells.iter().filter(|(cell, _)| !cell.sweep).collect();
    let max_of = |cells: &[&(S4Cell, Vec<S4Trial>)]| cells.iter().flat_map(|(_, trials)| trials.iter().filter(|trial| trial.complete()).filter_map(|trial| trial.restart_ns)).max();
    let complete = rate_cells.iter().all(|(cell, trials)| trials.iter().filter(|trial| trial.complete()).count() == cell.trials);
    if let Some(max) = max_of(&rate_cells) {
        let (rounded, side) = (max.div_ceil(1_000_000), if max <= S4_D5_VALUE_NS { "at or below" } else { "above" });
        let text = format!("max restart-after-resume over the rate cells {} ms (whole ms: {rounded} ms), {side} D5's {} ms", ms(max), S4_D5_VALUE_NS / 1_000_000);
        lines.push(if complete { format!("S4 decision D5 bound: {text}") } else { format!("S4 decision D5 bound: INCONCLUSIVE: a rate cell is incomplete; observed: {text}") });
    } else {
        lines.push("S4 decision D5 bound: INCONCLUSIVE: no complete trial".to_owned());
    }
    let sweep: Vec<_> = run.cells.iter().filter(|(cell, _)| cell.rate_hz == sweep_rate_hz).collect();
    if sweep.len() > 1 {
        let figures: Vec<String> = sweep
            .iter()
            .map(|(cell, trials)| {
                let times: Vec<u64> = trials.iter().filter(|trial| trial.complete()).filter_map(|trial| trial.restart_ns).collect();
                format!("stall {} ms: median {} ms, max {} ms (n={})", cell.stall_ms, median(&times).map_or("-".to_owned(), ms), times.iter().max().map_or("-".to_owned(), |max| ms(*max)), times.len())
            })
            .collect();
        let incomplete = sweep.iter().any(|(cell, trials)| trials.iter().filter(|trial| trial.complete()).count() < cell.trials);
        lines.push(format!(
            "S4 decision stall length at {} MS/s: {}{}",
            sweep_rate_hz as f64 / 1e6,
            if incomplete { "INCONCLUSIVE: a cell is incomplete; observed: " } else { "" },
            figures.join("; ")
        ));
    }
    lines
}

/// `trials` for S4 from the environment-style map, 20 by default (the owner's plan).
pub fn s4_trials(values: &BTreeMap<String, String>) -> Result<usize, String> {
    usize::try_from(env_count(values, "EZSDR_S4_TRIALS", 20)?).map_err(|_| "EZSDR_S4_TRIALS is too large".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(class: TrialClass) -> S2Trial {
        S2Trial {
            class,
            rx_samples: None,
            rx_expected: None,
            tx_samples: Some(2_000_000_000 * 5),
            tx_minimum: None,
            tx_rate_hz: Some(200_000_000),
            rx_overflow_events: 0,
            first_rx_overflow_ns: None,
            tx_underflow_events: 0,
            first_tx_underflow_ns: None,
            tx_underflow_untimed: 0,
            async_failures: vec![],
            run_termination: None,
        }
    }

    fn cell(id: &str, rx: Option<u64>, tx: Option<u64>, trials: usize, rule: S2Rule) -> S2Cell {
        S2Cell { id: id.to_owned(), rx_hz: rx, tx_hz: tx, trials, rule }
    }

    #[test]
    fn the_cells_are_the_briefs_and_a_filter_names_them() {
        let cells = s2_cells(S2_RATES, 10, 5);
        let ids: Vec<_> = cells.iter().map(|cell| (cell.id.as_str(), cell.trials)).collect();
        assert_eq!(ids, [("rx-100msps", 10), ("rx-200msps", 10), ("tx-100msps", 10), ("tx-200msps", 10), ("duplex-rx200msps-tx100msps", 5), ("duplex-rx100msps-tx100msps", 5)]);
        let tx200 = s2_select(cells.clone(), Some("tx-200msps")).unwrap();
        assert_eq!(tx200.len(), 1);
        assert_eq!(tx200[0].stage().tx, [StreamPlan { channels: vec![0], rate_hz: 200_000_000 }]);
        assert!(tx200[0].stage().rx.is_empty());
        assert!(s2_select(cells.clone(), Some("tx-201msps")).unwrap_err().contains("unknown cell"));
        let stage = |index: usize| (cells[index].stage().rx.iter().map(|s| s.rate_hz).collect::<Vec<_>>(), cells[index].stage().tx.iter().map(|s| s.rate_hz).collect::<Vec<_>>());
        assert_eq!(stage(4), (vec![200_000_000], vec![100_000_000]), "the first duplex pair is RX 200 + TX 100");
        assert_eq!(stage(5), (vec![100_000_000], vec![100_000_000]), "the second is RX 100 + TX 100");
    }

    #[test]
    fn only_a_report_that_raises_tx_underflow_without_a_device_time_is_untimed() {
        let row = |code: &str, tick: Json| json!({ "code": code, "tick": tick, "channel": 0, "stream": 0 });
        let rows = json!([
            row("Underflow", Json::Null),
            row("UnderflowInPacket", Json::Null),
            row("SeqError", Json::Null),
            row("SeqErrorInBurst", Json::Null),
            row("Underflow", json!(100)),
            row("BurstAck", Json::Null),
            row("TimeError", Json::Null),
            row("Other(7)", Json::Null),
        ]);
        assert_eq!(measurement::untimed_tx_underflow_reports(&rows), 4);
        assert_eq!(measurement::untimed_tx_underflow_reports(&Json::Null), 0);
    }

    #[test]
    fn the_transmit_window_is_judged_on_both_sides_of_ten_seconds() {
        let at = |ns: Option<i64>| {
            let mut t = trial(TrialClass::DeviceFailure);
            t.tx_underflow_events = ns.map_or(0, |_| 1);
            t.first_tx_underflow_ns = ns;
            t.window_held(S2_TX_WINDOW_NS)
        };
        assert_eq!(at(Some(S2_TX_WINDOW_NS - 1)), Some(false));
        assert_eq!(at(Some(S2_TX_WINDOW_NS)), Some(false), "an underflow at the window's end is within it");
        assert_eq!(at(Some(S2_TX_WINDOW_NS + 1)), Some(true), "one just after the window: the first 10 s played");
        assert_eq!(at(None), Some(true));
        let mut unknown = trial(TrialClass::DeviceFailure);
        unknown.tx_underflow_events = 1;
        assert_eq!(unknown.window_held(S2_TX_WINDOW_NS), None, "an underflow with no time cannot be placed");
        let untimed = |ns: Option<i64>| {
            let mut t = trial(TrialClass::DeviceFailure);
            t.tx_underflow_events = 1;
            t.tx_underflow_untimed = 1;
            t.first_tx_underflow_ns = ns;
            t.window_held(S2_TX_WINDOW_NS)
        };
        assert_eq!(untimed(Some(S2_TX_WINDOW_NS + 1)), None, "an untimestamped underflow read after the window may have happened inside it");
        assert_eq!(untimed(None), None);
        assert_eq!(untimed(Some(S2_TX_WINDOW_NS - 1)), Some(false), "read within the window, so it happened within it");
        let mut short = trial(TrialClass::Incomplete);
        short.tx_samples = Some(200_000_000 * 10 - 1);
        assert_eq!(short.window_held(S2_TX_WINDOW_NS), None, "fewer samples than the window's accepted");
        short.tx_samples = Some(200_000_000 * 10);
        assert_eq!(short.window_held(S2_TX_WINDOW_NS), Some(true));
    }

    #[test]
    fn an_s2_decision_reports_what_was_observed_and_is_inconclusive_when_trials_are_missing() {
        let rx = cell("rx-200msps", Some(200_000_000), None, 10, S2Rule::D1);
        let clean: Vec<S2Trial> = (0..10).map(|_| trial(TrialClass::Success)).collect();
        let line = s2_decision(&rx, &clean, 60, None);
        assert!(line.contains("10 Success, 0 failed") && line.contains("RX_OVERFLOW in 0 of 10") && line.contains("not fired"), "{line}");
        let mut one_overflow = clean.clone();
        one_overflow[3].class = TrialClass::DeviceFailure;
        one_overflow[3].rx_overflow_events = 1;
        one_overflow[3].first_rx_overflow_ns = Some(12_300_000_000);
        let line = s2_decision(&rx, &one_overflow, 60, None);
        assert!(line.contains("9 Success, 1 failed") && line.contains("first at [12.3] s") && line.contains("fired on this board") && !line.contains("not fired"), "{line}");
        let line = s2_decision(&rx, &clean[..9], 60, Some("handle latched"));
        assert!(line.contains("INCONCLUSIVE") && line.contains("9 Success") && line.contains("planned 10") && line.contains("handle latched"), "{line}");
        let mut incomplete = clean.clone();
        incomplete[0].class = TrialClass::Incomplete;
        assert!(s2_decision(&rx, &incomplete, 60, None).contains("INCONCLUSIVE"), "an Incomplete trial is not evidence");
        let tx_cell = cell("tx-200msps", None, Some(200_000_000), 1, S2Rule::D2);
        let mut short_run = trial(TrialClass::Success);
        short_run.tx_samples = Some(1_000);
        assert!(s2_decision(&tx_cell, &[short_run], 5, None).contains("INCONCLUSIVE"), "a Success whose Run was too short to show the 10 s window leaves the TX figure open");

        let tx = cell("tx-200msps", None, Some(200_000_000), 2, S2Rule::D2);
        let mut early = trial(TrialClass::DeviceFailure);
        early.tx_underflow_events = 1;
        early.first_tx_underflow_ns = Some(30_000_000_000);
        let mut soon = early.clone();
        soon.first_tx_underflow_ns = Some(4_000_000_000);
        let line = s2_decision(&tx, &[early, soon], 60, None);
        assert!(line.contains("first 10 s: 1 of 2 (underflow within it: 1") && line.contains("whole 60 s Success: 0 of 2") && line.contains("first TX_UNDERFLOW at [30.0, 4.0] s"), "{line}");
    }

    #[test]
    fn s2_runs_stop_at_a_harness_problem_and_leave_the_rest_unrun() {
        let cells = s2_cells((1_000_000, 2_000_000), 2, 1);
        let mut calls = 0;
        let result = |class| StageResult { class, left_open: false, observation: json!({ "streams": [] }) };
        let run = s2_run(
            &cells,
            |_| {
                calls += 1;
                if calls == 3 { Err("boom".to_owned()) } else { Ok(result(TrialClass::Success)) }
            },
            || None,
        );
        assert_eq!(run.cells.iter().map(|(_, trials)| trials.len()).collect::<Vec<_>>(), [2, 0, 0, 0, 0, 0]);
        assert!(run.stopped.as_deref().is_some_and(|reason| reason.contains("boom")), "{:?}", run.stopped);
        assert!(s2_decisions(&run, 60).iter().skip(1).all(|line| line.contains("INCONCLUSIVE")));
    }

    #[test]
    fn s5_decisions_name_the_claimed_rung_in_every_condition() {
        let stage = |condition: &str, rung_ms: u64, success: usize| LeadStage { condition: condition.to_owned(), rung_ns: rung_ms * 1_000_000, success, failed: 100 - success, total: 100 };
        let all = |five: usize| vec![stage("a", 4, 90), stage("a", 5, five), stage("a", 10, 100), stage("b", 4, 100), stage("b", 5, 100), stage("b", 10, 100)];
        let line = lead_decision("x", D3_LEAD_NS, &all(100), 6, 100);
        assert!(line.contains("5.000 ms: every trial Success in 2 of 2 conditions") && line.contains("a: 5.000 ms; b: 4.000 ms") && !line.contains("INCONCLUSIVE"), "{line}");
        let line = lead_decision("x", D3_LEAD_NS, &all(99), 6, 100);
        assert!(line.contains("1 of 2 conditions (not in: a)") && line.contains("a: 10.000 ms"), "{line}");
        assert!(lead_decision("x", D3_LEAD_NS, &all(100), 7, 100).contains("INCONCLUSIVE"), "a planned stage is missing");
        let mut short = all(100);
        short[1].total = 99;
        short[1].success = 99;
        assert!(lead_decision("x", D3_LEAD_NS, &short, 6, 100).contains("INCONCLUSIVE"), "99 trials of 100 planned");
        let line = lead_decision("x", D3_LEAD_NS, &all(100)[..3].to_vec().iter().filter(|s| s.rung_ns != D3_LEAD_NS).cloned().collect::<Vec<_>>(), 2, 100);
        assert!(line.contains("INCONCLUSIVE") && line.contains("no 5.000 ms rung in: a"), "{line}");
    }

    #[test]
    fn s5_plan_fills_the_rungs_and_a_named_value_wins() {
        let plan = s5_plan(&BTreeMap::new()).unwrap();
        assert_eq!(plan.rx_start_leads_ns, [3_000_000, 4_000_000, 5_000_000, 7_500_000, 10_000_000]);
        assert_eq!(plan.rx_start_rates_hz, [10_000_000, 100_000_000]);
        assert_eq!(plan.timed_command_leads_ns, [3_500_000, 4_000_000, 4_500_000, 5_000_000]);
        assert_eq!(plan.timed_command_tx_rates_hz, [1_000_000, 10_000_000, 100_000_000]);
        assert_eq!(plan.step_trials, 100);
        let named = s5_plan(&BTreeMap::from([("EZSDR_MEASURE_RX_START_RATES_HZ".to_owned(), "1000000".to_owned()), ("EZSDR_MEASURE_STEP_TRIALS".to_owned(), "7".to_owned())])).unwrap();
        assert_eq!((named.rx_start_rates_hz, named.step_trials), (vec![1_000_000], 7));
        assert!(s5_plan(&BTreeMap::from([("EZSDR_MEASURE_TIMED_LEADS_NS".to_owned(), "5000000,4000000".to_owned())])).is_err(), "unsorted rungs are refused before a device opens");
    }

    #[test]
    fn the_stability_criterion_is_met_at_exactly_half_and_not_above_it() {
        assert!(Stability::of(&[50, 100]).unwrap().met(), "(100 - 50)/100 = 0.5 meets <= 0.5");
        assert!(!Stability::of(&[49, 100]).unwrap().met());
        assert!(Stability::of(&[100, 100]).unwrap().met());
        assert_eq!(Stability::of(&[]), None);
    }

    #[test]
    fn an_s4_trial_counts_only_with_one_overflow_and_a_restart_time() {
        let mut trial = s4(Some(70));
        assert!(trial.complete());
        trial.restart_ns = None;
        assert!(!trial.complete(), "a Success whose restart-after-resume is unavailable is not evidence");
        let cell = S4Cell { rate_hz: 1_000_000, stall_ms: 20_000, trials: 1, sweep: false };
        assert!(s4_cell_decision(&cell, &[trial], None).contains("INCONCLUSIVE"));
    }

    #[test]
    fn lead_stages_and_rows_are_read_from_a_groups_observation() {
        let result = json!({ "candidates": [
            { "condition": "restart-same", "candidate": 5_000_000, "classes": { "Success": 1, "ProviderFailure": 1, "DeviceFailure": 0, "Incomplete": 0 },
              "trials": [{ "trial": 0, "class": "Success" }, { "trial": 1, "class": "ProviderFailure", "late_command": true }] },
        ] });
        let stages = lead_stages(&result);
        assert_eq!(stages, [LeadStage { condition: "restart-same".to_owned(), rung_ns: 5_000_000, success: 1, failed: 1, total: 2 }]);
        let rows = lead_rows("x", &result);
        assert_eq!(rows.len(), 3, "a row per trial and a line for the stage: {rows:?}");
        assert!(
            rows[1].starts_with("S5 x trial ") && rows[1].contains("\"late_command\":true") && rows[1].contains("\"condition\":\"restart-same\"") && rows[1].contains("\"rung_ns\":5000000"),
            "{}",
            rows[1]
        );
        assert!(rows[2].contains("5.000 ms: 1/2 Success, 1 failed, 0 not counted"), "{}", rows[2]);
        let failed_too = json!({ "candidates": [{ "condition": "c", "candidate": 1, "classes": { "Success": 0, "ProviderFailure": 1, "DeviceFailure": 3 }, "trials": [{}, {}, {}, {}] }] });
        assert_eq!(lead_stages(&failed_too)[0].failed, 4, "both failure classes count");
    }

    #[test]
    fn d5s_bound_is_inconclusive_while_a_rate_cell_is_incomplete() {
        let cell = |rate_hz| S4Cell { rate_hz, stall_ms: 500, trials: 2, sweep: false };
        let run = S4Run { cells: vec![(cell(1_000_000), vec![s4(Some(70)), s4(Some(80))]), (cell(10_000_000), vec![s4(Some(70)), s4(None)])], stopped: None };
        let lines = s4_decisions(&run, S4_STALL_SWEEP_RATE_HZ);
        assert!(lines.last().unwrap().contains("D5 bound: INCONCLUSIVE: a rate cell is incomplete") && lines.last().unwrap().contains("80.000 ms"), "{lines:?}");
    }

    fn s4(restart_ms: Option<u64>) -> S4Trial {
        S4Trial {
            class: if restart_ms.is_some() { TrialClass::Success } else { TrialClass::Incomplete },
            overflow_count: 1,
            gap_ns: Some(1),
            restart_ns: restart_ms.map(|ms| ms * 1_000_000),
            reason: None,
            run_termination: None,
        }
    }

    #[test]
    fn s4_decisions_state_the_spread_the_bound_and_the_stall_figures() {
        let cell = |rate_hz, stall_ms, sweep| S4Cell { rate_hz, stall_ms, trials: 2, sweep };
        let run = S4Run {
            cells: vec![
                (cell(1_000_000, 20_000, false), vec![s4(Some(70)), s4(Some(140))]),
                (cell(100_000_000, 500, false), vec![s4(Some(70)), s4(Some(141))]),
                (cell(100_000_000, 100, true), vec![s4(Some(80)), s4(None)]),
            ],
            stopped: None,
        };
        let lines = s4_decisions(&run, S4_STALL_SWEEP_RATE_HZ);
        assert!(lines[0].contains("max 140.000 ms") && lines[0].contains("<= 0.5: met"), "{}", lines[0]);
        assert!(lines[1].contains("max 141.000 ms") && lines[1].contains("not met"), "{}", lines[1]);
        assert!(lines[2].contains("INCONCLUSIVE") && lines[2].contains("1 complete of 2"), "{}", lines[2]);
        assert!(lines[3].contains("max restart-after-resume over the rate cells 141.000 ms (whole ms: 141 ms), above D5's 140 ms") && !lines[3].contains("INCONCLUSIVE"), "{}", lines[3]);
        assert!(lines[4].contains("stall length at 100 MS/s: INCONCLUSIVE") && lines[4].contains("stall 100 ms") && lines[4].contains("stall 500 ms"), "{}", lines[4]);
        let at = S4Run { cells: vec![(cell(1_000_000, 20_000, false), vec![s4(Some(140)), s4(Some(140))])], stopped: None };
        assert!(s4_decisions(&at, S4_STALL_SWEEP_RATE_HZ)[1].contains("at or below D5's 140 ms"), "the bound is inclusive");
        let at_50 = S4Run {
            cells: vec![
                (cell(50_000_000, 500, false), vec![s4(Some(70)), s4(Some(90))]),
                (cell(50_000_000, 2_000, true), vec![s4(Some(80)), s4(Some(95))]),
                (cell(100_000_000, 500, false), vec![s4(Some(70)), s4(Some(90))]),
            ],
            stopped: None,
        };
        let sweep = s4_decisions(&at_50, 50_000_000).pop().unwrap();
        assert!(sweep.contains("stall length at 50 MS/s: stall 500 ms") && sweep.contains("stall 2000 ms") && !sweep.contains("INCONCLUSIVE"), "the summary follows the sweep's own rate: {sweep}");
    }

    #[test]
    fn s4_cells_do_not_repeat_the_rate_cell_in_the_sweep() {
        let cells = s4_cells(20, S4_STALL_SWEEP_RATE_HZ, &S4_STALL_SWEEP_MS);
        assert_eq!(cells.len(), 6 + 2, "the 0.5 s sweep cell at 100 MS/s is the rate cell");
        assert_eq!(cells.iter().filter(|cell| cell.rate_hz == 100_000_000).map(|cell| (cell.stall_ms, cell.sweep)).collect::<Vec<_>>(), [(500, false), (100, true), (2_000, true)]);
        assert!(cells.iter().all(|cell| cell.trials == 20));
    }

    #[test]
    fn a_t200_decision_counts_runs_and_is_inconclusive_when_one_is_missing() {
        let run = |underflows: u64| T200Run {
            class: if underflows == 0 { TrialClass::Success } else { TrialClass::DeviceFailure },
            actual_samples: None,
            minimum_samples: None,
            underflow_events: underflows,
            first_underflow_ns: (underflows > 0).then_some(61_500_000_000),
            async_failures: vec![],
        };
        let plan = MeasurementPlan::default();
        let line = t200_decision(&[run(0), run(1), run(0)], 3, &plan, None);
        assert!(line.contains("2 Success, 1 failed") && line.contains("[0, 1, 0]") && line.contains("[61.5] s") && !line.contains("INCONCLUSIVE"), "{line}");
        assert!(t200_decision(&[run(0), run(0)], 3, &plan, Some("latched")).contains("INCONCLUSIVE"));
    }
}
