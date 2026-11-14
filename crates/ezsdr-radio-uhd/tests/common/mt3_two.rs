//! Maintenance table 3, cell C5 of `plan/phase8/bench.md` (Fable m1): two channels, a late start. Can one channel of a two-channel burst play alone? The trial is C1's
//! (`mt3::late_chunked_cell`, a two-channel `Shape`): a two-channel stream on slots A and B, an on-time control, then a burst whose first sample is timed at the host's
//! reference instant plus a delta, and a receive on both channels; each channel has its own evidence and its own `late_chunked_verdict`. The DUC chains of the two channels are
//! separate (`rfnoc_block_duc.v:242-244, 310-323`), so a burst that is on the edge of late can reach one chain in time and the other not (RM-11's "nothing was transmitted" would then need
//! "on at least one channel"). A split is therefore the finding the cell looks for, not a tool failure: `two_channel_reading` reports it as a FAIL so that the operator brings it to
//! the orchestrator, and says it is a device fact.
//!
//! The cell transmits on slot B, so it runs only through `bench1115::two_channel_gate` (an OBX or UBX in both slots and the owner's confirmation of slot B's attenuator).

#![allow(dead_code)]

use ezsdr_radio_uhd::Device;

use super::mt3::{Evidence, LATE_MS, Shape, Trial, cell_verdict, late_chunked_cell, late_chunked_verdict};
use super::mt3_cascade::{Outcome, US};
use super::raw::TICKS_PER_MS;
use super::tone::Verdict;
use ezsdr_radio_uhd::TxCode;

/// The marginal grid: the first sample timed at the reference instant plus each of these, in microseconds. The host's control-call latency decides where the transition lies, so the grid
/// is wide (INFERRED: the latency is not measured) and the reading is INCONCLUSIVE unless it straddles the transition.
pub const DELTAS_US: [i64; 9] = [-200, -100, -50, -20, 0, 20, 50, 100, 200];
/// Trials a delta, and the clear cell's.
pub const C5_TRIALS: usize = 10;
/// Decided trials (both channels played or dropped) the cells next to the transition need.
pub const MIN_DECIDED: usize = 10;

/// The text of the outcome of a channel that dropped the late burst (TimeError, no ACK) but on whose receive a tone was heard: a leak from the other slot, or a partial play.
pub const LEAK: &str = "tone heard without an ACK: leak suspected (or a partial play)";

/// One channel's outcome of a marginal trial, read from its own reports first (a tone on the channel's receive can be slot A's leaking in, and must not decide a split): **played**
/// when a BurstAck came at or after the late burst's first sample and no TimeError did; **dropped** when a TimeError did and no ACK, and `late_chunked_verdict` is PASS (nothing heard,
/// the receive complete, the control heard...); INCONCLUSIVE when the reports have a tick missing, say both or neither, the verdict is INCONCLUSIVE, or a tone was heard without an
/// ACK (`late_chunked_verdict` FAIL with no ACK: a leak is suspected, or a partial play; either way the channel is not known dropped).
pub fn channel_outcome(e: &Evidence) -> Outcome {
    let from = e.late.0;
    if e.reports.iter().any(|r| r.tick.is_none() && matches!(r.code, TxCode::BurstAck | TxCode::TimeError)) {
        return Outcome::Inconclusive("a report without a tick cannot be attributed".to_owned());
    }
    let at_or_after = |code: TxCode| e.reports.iter().any(|r| r.code == code && r.tick.is_some_and(|tick| tick >= from));
    match (at_or_after(TxCode::BurstAck), at_or_after(TxCode::TimeError)) {
        (true, false) => Outcome::Played,
        (true, true) => Outcome::Inconclusive("a BurstAck and a TimeError both for the late burst on one channel".to_owned()),
        (false, false) => Outcome::Inconclusive("neither a BurstAck nor a TimeError for the late burst".to_owned()),
        (false, true) => match late_chunked_verdict(e) {
            Verdict::Pass(_) => Outcome::Dropped,
            Verdict::Fail(_) => Outcome::Inconclusive(LEAK.to_owned()),
            Verdict::Inconclusive(text) => Outcome::Inconclusive(text),
        },
    }
}

/// The two channels' outcomes of every trial of a marginal trial set.
pub fn trial_outcomes(trials: &[Trial]) -> Vec<[Outcome; 2]> {
    trials.iter().map(|trial| [channel_outcome(&trial.evidence[0]), channel_outcome(&trial.evidence[1])]).collect()
}

/// One delta's trials, each the outcomes of channel 0 and channel 1.
#[derive(Clone, Debug)]
pub struct MarginalCell {
    /// The first sample's time less the reference instant, in root ticks.
    pub delta: i64,
    pub trials: Vec<[Outcome; 2]>,
}

fn decided(outcome: &Outcome) -> bool {
    !matches!(outcome, Outcome::Inconclusive(_))
}

impl MarginalCell {
    /// The trials whose two channels are both decided.
    fn decided(&self) -> impl Iterator<Item = &[Outcome; 2]> {
        self.trials.iter().filter(|pair| pair.iter().all(decided))
    }

    fn count(&self, want: &Outcome) -> usize {
        self.decided().filter(|pair| pair.iter().all(|outcome| outcome == want)).count()
    }

    /// Both channels played in every decided trial, and there is one.
    fn all_played(&self) -> bool {
        self.decided().next().is_some() && self.count(&Outcome::Played) == self.decided().count()
    }

    /// Both channels dropped in every decided trial, and there is one.
    fn all_dropped(&self) -> bool {
        self.decided().next().is_some() && self.count(&Outcome::Dropped) == self.decided().count()
    }

    /// The trials in which one channel played and the other was dropped.
    fn splits(&self) -> Vec<usize> {
        let split = |pair: &[Outcome; 2]| (pair[0] == Outcome::Played && pair[1] == Outcome::Dropped) || (pair[0] == Outcome::Dropped && pair[1] == Outcome::Played);
        self.trials.iter().enumerate().filter(|(_, pair)| split(pair)).map(|(index, _)| index).collect()
    }

    /// The trials in which one channel played and the other is undecided only because a tone was heard without an ACK: a split a leak may hide.
    fn hidden_splits(&self) -> Vec<usize> {
        let leak = |outcome: &Outcome| matches!(outcome, Outcome::Inconclusive(text) if text == LEAK);
        let hidden = |pair: &[Outcome; 2]| (pair[0] == Outcome::Played && leak(&pair[1])) || (leak(&pair[0]) && pair[1] == Outcome::Played);
        self.trials.iter().enumerate().filter(|(_, pair)| hidden(pair)).map(|(index, _)| index).collect()
    }

    fn line(&self) -> String {
        format!(
            "{} us: {} both played, {} both dropped, {} undecided of {}",
            self.delta as f64 / US as f64,
            self.count(&Outcome::Played),
            self.count(&Outcome::Dropped),
            self.trials.len() - self.decided().count(),
            self.trials.len()
        )
    }
}

/// The reading of the cell from the clear cell's verdict (C1's, per channel) and the marginal cells, in this order. **FAIL** with `SPLIT observed` when any trial had one channel play and
/// the other drop: positive evidence for the contract review, a device fact and not a tool defect. FAIL when the clear cell fails (a clearly late burst played). Otherwise
/// **INCONCLUSIVE** when any trial in any cell paired a played channel with one that heard a tone without an ACK (a possible split hidden by a leak), when the clear cell is not PASS, when the grid did not straddle the transition (a delta at which both channels played in every decided trial, and a lower one at which both
/// dropped in every decided trial), or when a cell from the last all-dropped one to the first all-played one above it has fewer than `MIN_DECIDED` decided trials. **PASS**
/// otherwise, worded "no split in N marginal trials at these conditions".
pub fn two_channel_reading(clear: &Verdict, cells: &[MarginalCell]) -> Verdict {
    let split: Vec<String> = cells.iter().flat_map(|cell| cell.splits().into_iter().map(move |index| format!("{} us trial {index}", cell.delta as f64 / US as f64))).collect();
    if !split.is_empty() {
        return Verdict::Fail(format!(
            "SPLIT observed ({split:?}): one channel of a two-channel burst played and the other was dropped. This is a device fact for the contract review (RM-11's \"nothing was transmitted\" would need \"on at least one channel\"), not a tool defect: bring it to the orchestrator"
        ));
    }
    if let Verdict::Fail(text) = clear {
        return Verdict::Fail(format!("the clearly late burst played on a channel: {text}"));
    }
    let mut reasons = Vec::new();
    if let Verdict::Inconclusive(text) = clear {
        reasons.push(format!("the clear cell is not PASS: {text}"));
    }
    let hidden: Vec<String> = cells.iter().flat_map(|cell| cell.hidden_splits().into_iter().map(move |index| format!("{} us trial {index}", cell.delta as f64 / US as f64))).collect();
    if !hidden.is_empty() {
        reasons.push(format!("possible split hidden by a leak ({hidden:?}): one channel played and the other heard a tone without an ACK"));
    }
    let upper = cells.iter().position(MarginalCell::all_played);
    let lower = upper.and_then(|upper| cells[..upper].iter().rposition(MarginalCell::all_dropped));
    let bracket = lower.zip(upper);
    match bracket {
        Some((lower, upper)) => {
            let thin: Vec<String> = cells[lower..=upper].iter().filter(|cell| cell.decided().count() < MIN_DECIDED).map(MarginalCell::line).collect();
            if !thin.is_empty() {
                reasons.push(format!("the cells from the last all-dropped to the first all-played delta need {MIN_DECIDED} decided trials each: {thin:?}"));
            }
        }
        None => reasons.push("the grid did not straddle the transition: no delta with both channels dropped in every decided trial below one with both played in every decided trial".to_owned()),
    }
    match bracket {
        Some((lower, upper)) if reasons.is_empty() => {
            let decided: usize = cells.iter().map(|cell| cell.decided().count()).sum();
            let near: usize = cells[lower..=upper].iter().map(|cell| cell.decided().count()).sum();
            Verdict::Pass(format!(
                "no split in {decided} decided marginal trials ({} run) at these conditions: 1 MS/s, a 20 ms burst, 1 GHz, slots A and B; the transition lies between {} us and {} us, where {near} decided trials in the {} cells from the last all-dropped to the first all-played delta saw no split",
                cells.iter().map(|cell| cell.trials.len()).sum::<usize>(),
                cells[lower].delta as f64 / US as f64,
                cells[upper].delta as f64 / US as f64,
                upper - lower + 1
            ))
        }
        _ => Verdict::Inconclusive(reasons.join("; ")),
    }
}

/// Runs the cell and prints its lines: the clear cell (a burst timed `LATE_MS` before the reference instant: `MT3 C5 clear trace` and `VERDICT mt3 c5 clear` a trial, `MT3 C5 clear cell`), then
/// each marginal delta (`MT3 C5 trace` and `MT3 C5 trial <delta> us #<i>` the outcome of each channel, `MT3 C5 marginal` the cell) and `MT3 C5 reading`. `trials` a cell; returns the
/// marginal cells and the reading. The caller has passed `two_channel_gate`: this transmits on both slots.
pub fn report_c5(device: &dyn Device, deltas_us: &[i64], trials: usize) -> (Vec<MarginalCell>, Verdict) {
    let clear = late_chunked_cell(device, Shape::c5(-LATE_MS * TICKS_PER_MS), trials);
    for trial in &clear {
        println!("MT3 C5 clear trace {}", trial.trace());
        println!("VERDICT mt3 c5 clear trial {}: {:?}", trial.trial, trial.verdict());
    }
    let clear = cell_verdict(&clear);
    println!("MT3 C5 clear cell: {clear:?}");
    let cells: Vec<MarginalCell> = deltas_us
        .iter()
        .map(|&us| {
            let run = late_chunked_cell(device, Shape::c5(us * US), trials);
            let cell = MarginalCell { delta: us * US, trials: trial_outcomes(&run) };
            for (trial, outcomes) in run.iter().zip(&cell.trials) {
                println!("MT3 C5 trace {}", trial.trace());
                let first_send = trial.evidence[0].late_first.map(|(handed, at)| handed - at);
                println!(
                    "MT3 C5 trial {us} us #{}: channel 0 {:?}, channel 1 {:?}; the first send was handed {first_send:?} ticks after its first sample's time (device time read after it)",
                    trial.trial, outcomes[0], outcomes[1]
                );
            }
            println!("MT3 C5 marginal {}", cell.line());
            cell
        })
        .collect();
    let reading = two_channel_reading(&clear, &cells);
    println!("MT3 C5 reading: {reading:?}");
    (cells, reading)
}

#[cfg(test)]
mod tests {
    use super::super::mt3::tests::{passing, report};
    use super::super::observations::HeardInterval;
    use super::*;

    fn cell(delta_us: i64, played: usize, dropped: usize, undecided: usize) -> MarginalCell {
        let pair = |outcome: Outcome| [outcome.clone(), outcome];
        let trials = std::iter::repeat_with(|| pair(Outcome::Played))
            .take(played)
            .chain(std::iter::repeat_with(|| pair(Outcome::Dropped)).take(dropped))
            .chain(std::iter::repeat_with(|| [Outcome::Played, Outcome::Inconclusive("x".to_owned())]).take(undecided))
            .collect();
        MarginalCell { delta: delta_us * US, trials }
    }

    /// Dropped below the transition, played above it, one mixed cell (both channels together) between, each cell ten trials.
    fn straddling() -> Vec<MarginalCell> {
        vec![cell(-200, 0, 10, 0), cell(-100, 0, 10, 0), cell(-50, 0, 10, 0), cell(-20, 0, 10, 0), cell(0, 3, 7, 0), cell(20, 10, 0, 0), cell(50, 10, 0, 0), cell(100, 10, 0, 0), cell(200, 10, 0, 0)]
    }

    fn pass() -> Verdict {
        Verdict::Pass("clear".to_owned())
    }

    fn assert_kind(verdict: Verdict, want: &str, what: &str) {
        verdict.assert_kind(want, what);
    }

    #[test]
    fn mt3_c5_reading_passes_a_straddled_transition_without_a_split() {
        let verdict = two_channel_reading(&pass(), &straddling());
        assert!(
            matches!(&verdict, Verdict::Pass(text) if text.contains("no split in 90 decided marginal trials (90 run) at these conditions") && text.contains("between -20 us and 20 us, where 30 decided trials in the 3 cells")),
            "{verdict:?}"
        );
    }

    #[test]
    fn mt3_c5_reading_a_split_fails_and_says_it_is_a_device_fact() {
        for pair in [[Outcome::Played, Outcome::Dropped], [Outcome::Dropped, Outcome::Played]] {
            let mut cells = straddling();
            cells[4].trials[0] = pair;
            let verdict = two_channel_reading(&pass(), &cells);
            assert!(matches!(&verdict, Verdict::Fail(text) if text.contains("SPLIT observed") && text.contains("0 us trial 0") && text.contains("device fact")), "{verdict:?}");
        }
        // A split is positive evidence whatever else is missing: a clear cell that is INCONCLUSIVE, a grid that does not straddle.
        let mut cells = vec![cell(-200, 0, 10, 0)];
        cells[0].trials[3] = [Outcome::Dropped, Outcome::Played];
        assert_kind(two_channel_reading(&Verdict::Inconclusive("x".to_owned()), &cells), "FAIL", "a split in an otherwise useless run");
        // A pair with an undecided channel is not a split.
        let mut undecided = straddling();
        undecided[0].trials[0] = [Outcome::Played, Outcome::Inconclusive("x".to_owned())];
        assert_kind(two_channel_reading(&pass(), &undecided), "PASS", "one channel played, the other undecided");
    }

    #[test]
    fn mt3_c5_reading_a_clear_cell_that_is_not_pass_is_not_a_pass() {
        assert_kind(two_channel_reading(&Verdict::Fail("played".to_owned()), &straddling()), "FAIL", "a clearly late burst played");
        assert_kind(two_channel_reading(&Verdict::Inconclusive("no control".to_owned()), &straddling()), "INCONCLUSIVE", "the clear cell is INCONCLUSIVE");
    }

    #[test]
    fn mt3_c5_reading_a_grid_that_does_not_straddle_the_transition_is_inconclusive() {
        // Every offset arrived clearly late: both channels dropped everywhere. The tool saw no transition, so the absence of a split proves nothing.
        let all_late: Vec<MarginalCell> = DELTAS_US.iter().map(|us| cell(*us, 0, 10, 0)).collect();
        let verdict = two_channel_reading(&pass(), &all_late);
        assert!(matches!(&verdict, Verdict::Inconclusive(text) if text.contains("did not straddle the transition")), "both channels dropped at every delta: {verdict:?}");
        let all_on_time: Vec<MarginalCell> = DELTAS_US.iter().map(|us| cell(*us, 10, 0, 0)).collect();
        assert_kind(two_channel_reading(&pass(), &all_on_time), "INCONCLUSIVE", "both channels played at every delta");
        assert_kind(two_channel_reading(&pass(), &[]), "INCONCLUSIVE", "no marginal cell");
        // An all-played cell below the all-dropped ones is not the transition either.
        let reversed = vec![cell(-200, 10, 0, 0), cell(0, 0, 10, 0)];
        assert_kind(two_channel_reading(&pass(), &reversed), "INCONCLUSIVE", "played before dropped");
    }

    #[test]
    fn mt3_c5_reading_the_cells_next_to_the_transition_need_ten_decided_trials() {
        let mut thin_lower = straddling();
        thin_lower[3] = cell(-20, 0, 9, 1);
        assert_kind(two_channel_reading(&pass(), &thin_lower), "INCONCLUSIVE", "9 decided in the last all-dropped cell");
        let mut thin_upper = straddling();
        thin_upper[5] = cell(20, 8, 0, 2);
        assert_kind(two_channel_reading(&pass(), &thin_upper), "INCONCLUSIVE", "8 decided in the first all-played cell");
        let mut thin_mixed = straddling();
        thin_mixed[4] = cell(0, 2, 7, 1);
        assert_kind(two_channel_reading(&pass(), &thin_mixed), "INCONCLUSIVE", "9 decided in the mixed cell between");
        let mut thin_far = straddling();
        thin_far[0] = cell(-200, 0, 2, 8);
        thin_far[8] = cell(200, 2, 0, 8);
        let far = two_channel_reading(&pass(), &thin_far);
        assert!(
            matches!(&far, Verdict::Pass(text) if text.contains("no split in 74 decided marginal trials (90 run)")),
            "thin cells far from the transition do not matter, and the headline counts decided trials: {far:?}"
        );
    }

    /// A tone heard inside the late interval (the loopback delay of `passing`'s control taken into account).
    fn tone_in_late_interval() -> HeardInterval {
        HeardInterval { first_tick: 30_000_000 + 44 * 200 + 500_000, last_tick: 30_000_000 + 44 * 200 + 500_000 + 2_499 * 200, samples: 2_500 }
    }

    #[test]
    fn mt3_c5_a_channel_is_read_from_its_reports_first() {
        // `passing`: the control's ACK before the late burst, two TimeErrors for it, nothing heard in its interval.
        assert_eq!(channel_outcome(&passing()), Outcome::Dropped);
        let mut played = passing();
        played.reports = vec![report(TxCode::BurstAck, Some(14_000_000)), report(TxCode::BurstAck, Some(34_000_001))];
        assert_eq!(channel_outcome(&played), Outcome::Played, "an ACK for the late burst and no TimeError");
        let mut both = passing();
        both.reports.push(report(TxCode::BurstAck, Some(34_000_001)));
        assert!(matches!(channel_outcome(&both), Outcome::Inconclusive(text) if text.contains("both")));
        let mut neither = passing();
        neither.reports = vec![report(TxCode::BurstAck, Some(14_000_000))];
        assert!(matches!(channel_outcome(&neither), Outcome::Inconclusive(text) if text.contains("neither")));
        let mut tickless = passing();
        tickless.reports.push(report(TxCode::TimeError, None));
        assert!(matches!(channel_outcome(&tickless), Outcome::Inconclusive(text) if text.contains("tick")));
        let mut played_tickless = played.clone();
        played_tickless.reports.push(report(TxCode::TimeError, None));
        assert!(matches!(channel_outcome(&played_tickless), Outcome::Inconclusive(text) if text.contains("tick")), "an ACK and a TimeError that cannot be placed: the ACK alone is not read");
        let mut uncovered = passing();
        uncovered.blocks.clear();
        assert!(matches!(channel_outcome(&uncovered), Outcome::Inconclusive(text) if text.contains("cover")), "a TimeError and no ACK, but the receive cannot say nothing was heard");
    }

    #[test]
    fn mt3_c5_a_tone_heard_without_an_ack_is_a_suspected_leak_and_a_played_channel_beside_it_is_a_possible_hidden_split() {
        // Channel 1 dropped the late burst (TimeErrors, no ACK) but slot A's tone leaks into its receiver: the tone is heard. That is not "played" and not "dropped".
        let mut leaky = passing();
        leaky.heard.push(tone_in_late_interval());
        assert_eq!(channel_outcome(&leaky), Outcome::Inconclusive(LEAK.to_owned()));
        // A real split (channel 0 played by its ACK, channel 1 dropped by its reports) is seen even though channel 1 hears the leak: the pair is undecided, not both played.
        let mut played = passing();
        played.reports = vec![report(TxCode::BurstAck, Some(14_000_000)), report(TxCode::BurstAck, Some(34_000_001))];
        let trial = Trial { rate: 1e6, trial: 0, evidence: vec![played.clone(), leaky.clone()] };
        let outcomes = trial_outcomes(&[trial]);
        assert_eq!(outcomes[0][0], Outcome::Played);
        assert!(matches!(outcomes[0][1], Outcome::Inconclusive(_)), "the leaking channel is undecided: {:?}", outcomes[0]);
        // And a clean split is a split.
        let clean = Trial { rate: 1e6, trial: 0, evidence: vec![played.clone(), passing()] };
        let split = MarginalCell { delta: 0, trials: trial_outcomes(&[clean]) };
        assert_eq!(split.splits(), vec![0]);
        assert!(split.hidden_splits().is_empty());
        // The pair of a played channel and a leaking one is a possible split hidden by the leak, in either order.
        let hidden = MarginalCell {
            delta: 0,
            trials: trial_outcomes(&[Trial { rate: 1e6, trial: 0, evidence: vec![played.clone(), leaky.clone()] }, Trial { rate: 1e6, trial: 1, evidence: vec![leaky.clone(), played.clone()] }]),
        };
        assert_eq!(hidden.hidden_splits(), vec![0, 1]);
        assert!(hidden.splits().is_empty());
    }

    #[test]
    fn mt3_c5_reading_a_played_channel_beside_a_leaking_one_outside_the_bracket_is_not_a_pass() {
        // -20 us all dropped, 0 us all played: the bracket is those two cells. One trial at 20 us (outside it) pairs a played channel with one that heard a tone without an ACK.
        let sequence = |last: [Outcome; 2]| {
            let mut late = cell(20, 10, 0, 0);
            late.trials[9] = last;
            vec![cell(-20, 0, 10, 0), cell(0, 10, 0, 0), late]
        };
        let leak = || Outcome::Inconclusive(LEAK.to_owned());
        assert_kind(two_channel_reading(&pass(), &[cell(-20, 0, 10, 0), cell(0, 10, 0, 0), cell(20, 10, 0, 0)]), "PASS", "the same sequence without the leak");
        for pair in [[Outcome::Played, leak()], [leak(), Outcome::Played]] {
            let verdict = two_channel_reading(&pass(), &sequence(pair));
            assert!(matches!(&verdict, Verdict::Inconclusive(text) if text.contains("possible split hidden by a leak") && text.contains("20 us trial 9")), "{verdict:?}");
        }
        // A leak beside a dropped channel, or two leaking channels, hides nothing about a split.
        assert_kind(two_channel_reading(&pass(), &sequence([Outcome::Dropped, leak()])), "PASS", "a dropped channel beside a leaking one");
        assert_kind(two_channel_reading(&pass(), &sequence([leak(), leak()])), "PASS", "two leaking channels");
        // A real split elsewhere is still the finding.
        let mut with_split = sequence([Outcome::Played, leak()]);
        with_split[0].trials[0] = [Outcome::Played, Outcome::Dropped];
        assert_kind(two_channel_reading(&pass(), &with_split), "FAIL", "a split beats a possible hidden one");
    }
}
