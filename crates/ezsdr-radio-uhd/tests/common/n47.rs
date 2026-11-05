//! Note 47's bench probes (2026-11-05): the pure PASS/FAIL decisions of the receive-restart (T2), the raw
//! rate on a held-open streamer (T4), the transmit setting after a burst's end (T5) and the control-call and
//! receive-buffer probes (T6). A probe prints `Verdict` lines; the decision itself is here, so a wrong
//! comparison fails a unit test without a USRP.

#![allow(dead_code)]

use serde_json::Value as Json;

use super::tone::{Verdict, contiguous_windows_at, first_ms_frequency, first_window_verdict, median_freq, window_tone_at};

/// The receive tone's tolerance, in hertz (the probes shift tones by 50 kHz or more).
pub const TONE_TOLERANCE_HZ: f64 = 10e3;

/// The setting-after-the-burst-end probe's bound on A's tail power: ±1 dB on power, so the ratio lies in 10^(±1/10).
pub const TAIL_POWER_DB: f64 = 1.0;

/// Whether a burst ACK at `ack` confirms a burst whose exclusive end is `e`, at `per` ticks a sample for that burst's
/// rate. The radio acknowledges when it consumes the EOB sample (`radio_tx_core.v:359-379`), so its tick is up to one
/// sample before `e`; the host reads it no later than a tick past `e`. The window is `[e − 2 × per, e + 1]`.
pub fn burst_ack_holds(ack: i64, e: i64, per: i64) -> bool {
    let delta = ack - e;
    delta >= -2 * per && delta <= 1
}

/// How many burst ACKs among `reports` fall in `[from, to]`, the window of one burst: one confirms the burst there, and
/// a duplicate is a second.
pub fn burst_acks_in(reports: &[ezsdr_radio_uhd::TxReport], from: i64, to: i64) -> usize {
    reports.iter().filter(|report| report.code == ezsdr_radio_uhd::TxCode::BurstAck && report.tick.is_some_and(|tick| (from..=to).contains(&tick))).count()
}

/// One receive rate change of `hw_n47_raw_rx_rate_on_an_open_streamer`: `drained` says the old stream reached a
/// quiet interval before the change (a failed drain makes the trial inconclusive, and no rate is applied); then the
/// device's read-back, the blocks after the planned start as (first tick, samples), the ticks a sample at the new
/// rate, the tone found in the read window, and how often the whole probe opened the receive streamer.
#[allow(clippy::too_many_arguments)]
pub fn rx_rate_verdict(asked: f64, applied: f64, planned: i64, blocks: &[(i64, usize)], per: i64, tone: Option<f64>, tone_hz: f64, opens: usize, drained: bool) -> Verdict {
    if !drained {
        return Verdict::Inconclusive("the old stream did not reach a quiet interval before the change: no rate was applied".to_owned());
    }
    if opens != 1 {
        return Verdict::Fail(format!("the probe opened the receive streamer {opens} times, not once"));
    }
    if (applied - asked).abs() > asked.abs() * 1e-9 + 1e-6 {
        return Verdict::Fail(format!("the device read back {applied} S/s for the asked {asked} S/s"));
    }
    let new: Vec<&(i64, usize)> = blocks.iter().filter(|(tick, _)| *tick >= planned).collect();
    let Some(first) = new.first() else {
        return Verdict::Inconclusive("no block after the planned start was read".to_owned());
    };
    if first.0 != planned {
        return Verdict::Fail(format!("the first block is at {} for the planned start {planned}", first.0));
    }
    for pair in new.windows(2) {
        let expected = pair[0].0 + pair[0].1 as i64 * per;
        if pair[1].0 != expected {
            return Verdict::Fail(format!("a block begins at {} where the block before it ends at {expected}", pair[1].0));
        }
    }
    match tone {
        Some(hz) if (hz - tone_hz).abs() <= TONE_TOLERANCE_HZ => Verdict::Pass(format!("{applied} S/s, blocks at {per} ticks a sample, the tone at {hz:.0} Hz")),
        Some(hz) => Verdict::Fail(format!("the tone is at {hz:.0} Hz, not {tone_hz:.0} Hz")),
        None => Verdict::Inconclusive("no coherent tone in the window after the start".to_owned()),
    }
}

/// One transmit rate change of `hw_n47_raw_tx_rate_on_an_open_streamer`: `a_ack` is A's correlated burst ACK and
/// `a_end` its exclusive end, `per` ticks a sample at A's rate; `a_tail` the tone of A's last millisecond; `b_onset`
/// the receive tick where B's tone starts and `b_expected_onset` the same after the control burst's loopback delay.
#[allow(clippy::too_many_arguments)]
pub fn tx_rate_verdict(
    asked: f64,
    applied: f64,
    a_ack: Option<i64>,
    a_end: i64,
    per: i64,
    a_tail: Option<f64>,
    a_tone: f64,
    b_onset: Option<i64>,
    b_expected_onset: Option<i64>,
    onset_tolerance_ticks: i64,
    b_tone: Option<f64>,
    b_tone_expected: f64,
    a_acks: usize,
    b_acks: usize,
    time_errors: usize,
    underflows: usize,
    opens: usize,
) -> Verdict {
    if opens != 1 {
        return Verdict::Fail(format!("the probe opened the transmit streamer {opens} times, not once"));
    }
    if (applied - asked).abs() > asked.abs() * 1e-9 + 1e-6 {
        return Verdict::Fail(format!("the device read back {applied} S/s for the asked {asked} S/s"));
    }
    if time_errors > 0 || underflows > 0 {
        return Verdict::Fail(format!("{time_errors} TIME_ERROR and {underflows} underflow report(s)"));
    }
    let Some(a_ack) = a_ack else {
        return Verdict::Inconclusive("A's burst ACK was not read: A's end is not confirmed".to_owned());
    };
    let ack_delta = a_ack - a_end;
    if !burst_ack_holds(a_ack, a_end, per) {
        return Verdict::Inconclusive(format!("A's burst ACK is at {a_ack} ({ack_delta:+} from its end {a_end}), outside [e − 2 × {per}, e + 1]: A's end is not confirmed"));
    }
    if b_acks == 0 {
        return Verdict::Inconclusive("B's burst ACK was not read: B's end is not confirmed".to_owned());
    }
    if a_acks != 1 || b_acks != 1 {
        return Verdict::Fail(format!("{a_acks} and {b_acks} burst ACKs for A and B, not one each"));
    }
    match a_tail {
        Some(hz) if (hz - a_tone).abs() <= TONE_TOLERANCE_HZ => {}
        Some(hz) => return Verdict::Fail(format!("A's last millisecond is at {hz:.0} Hz, not {a_tone:.0} Hz")),
        None => return Verdict::Inconclusive("A's last millisecond holds no coherent tone".to_owned()),
    }
    match b_tone {
        Some(hz) if (hz - b_tone_expected).abs() <= TONE_TOLERANCE_HZ => {}
        Some(hz) => return Verdict::Fail(format!("B's first window is at {hz:.0} Hz, not {b_tone_expected:.0} Hz")),
        None => return Verdict::Inconclusive("B's first window holds no coherent tone".to_owned()),
    }
    // The rise's threshold crossing depends on the rate and frequency (filter fill): a few microseconds off the control's
    // delay is onset precision, not the claim (bench 2026-11-05: +9 us at 2 MS/s); reported, never a FAIL.
    match (b_onset, b_expected_onset) {
        (Some(onset), Some(expected)) if (onset - expected).abs() <= onset_tolerance_ticks => {}
        (Some(onset), Some(expected)) => return Verdict::Inconclusive(format!("the rest holds; B starts {:+} ticks off the expected {expected}: onset precision", onset - expected)),
        (Some(_), None) => return Verdict::Inconclusive("no control burst measured the loopback's delay".to_owned()),
        (None, _) => return Verdict::Inconclusive("no B onset was read".to_owned()),
    }
    Verdict::Pass(format!("{applied} S/s read back; A's ACK at {a_ack} ({ack_delta:+} from its end {a_end}); B at its tick; the tones hold"))
}

/// One trial of `hw_n47_raw_tx_setting_after_a_burst_end`: `m_ms` is the requested offset of the untimed setting from
/// A's end (`e`), `per` ticks a sample at the probe's rate, and `issued`/`returned` its actual device-time bracket;
/// `tail` is the tone of A's last millisecond and its power against the steady burst; `move_at` where the tone first
/// leaves A's frequency. `power` is `(tail, steady)`; a ratio outside `[min_ratio, max_ratio]` is a changed tail (a
/// truncation or a gain move). `b_power` and `b_gain_db` carry the gain trial: B's first-window power must be the
/// steady power raised by that gain, ±1.5 dB.
#[allow(clippy::too_many_arguments)]
pub fn tail_verdict(
    m_ms: i64,
    e: i64,
    per: i64,
    issued: i64,
    returned: i64,
    a_ack: Option<i64>,
    a_tone: f64,
    tail: Option<f64>,
    power: Option<(f64, f64)>,
    min_ratio: f64,
    max_ratio: f64,
    move_at: Option<i64>,
    b_tone: Option<f64>,
    b_tone_expected: f64,
    b_onset: Option<i64>,
    b_expected_onset: Option<i64>,
    onset_tolerance_ticks: i64,
    b_power: Option<f64>,
    b_gain_db: f64,
) -> Verdict {
    let Some(a_ack) = a_ack else {
        return Verdict::Inconclusive("A's burst ACK was not read: A's end is not confirmed".to_owned());
    };
    let ack_delta = a_ack - e;
    if !burst_ack_holds(a_ack, e, per) {
        return Verdict::Inconclusive(format!("A's burst ACK is at {a_ack} ({ack_delta:+} from its end {e}), outside [e − 2 × {per}, e + 1]: A's end is not confirmed"));
    }
    let ratio = power.map(|(tail, steady)| if steady > 0.0 { tail / steady } else { f64::INFINITY });
    let intact = tail.is_some_and(|hz| (hz - a_tone).abs() <= TONE_TOLERANCE_HZ) && ratio.is_some_and(|ratio| (min_ratio..=max_ratio).contains(&ratio)) && move_at.is_none_or(|at| at >= e);
    if m_ms < 0 {
        // The control shows the hazard only when the setting really went out before A ended; a bracket at or after
        // `e` is not the −5 ms control and says nothing.
        if issued >= e || returned >= e {
            return Verdict::Inconclusive(format!("the {m_ms} ms control was issued at {issued} and returned at {returned}, not both before A's end {e}"));
        }
        return if intact {
            Verdict::Inconclusive(format!("the {m_ms} ms control left A's tail intact: the hazard is not shown"))
        } else {
            Verdict::Pass(format!("the {m_ms} ms control disturbed A's tail (tone {tail:?}, ratio {ratio:?}, move {move_at:?} for e {e})"))
        };
    }
    if issued < e {
        return Verdict::Inconclusive(format!("the {m_ms} ms trial's setting was issued at {issued}, before A's end {e}: it is not the measured offset"));
    }
    if !intact {
        return Verdict::Fail(format!("A's tail is not intact (tone {tail:?}, ratio {ratio:?}, move {move_at:?} for e {e})"));
    }
    match b_tone {
        Some(hz) if (hz - b_tone_expected).abs() <= TONE_TOLERANCE_HZ => {}
        Some(hz) => return Verdict::Fail(format!("B's first window is at {hz:.0} Hz, not {b_tone_expected:.0} Hz")),
        None => return Verdict::Inconclusive("B's first window holds no coherent tone".to_owned()),
    }
    // B's onset is judged last and only reported: its threshold crossing depends on the frequency (filter fill), so a few
    // microseconds off the control's delay is onset precision, not the claim (bench 2026-11-05: +6 us).
    let onset_off = match (b_onset, b_expected_onset) {
        (Some(onset), Some(expected)) if (onset - expected).abs() <= onset_tolerance_ticks => None,
        (Some(onset), Some(expected)) => Some(format!("B starts at {onset}, {:+} ticks from the expected {expected}", onset - expected)),
        (None, Some(_)) => return Verdict::Inconclusive("no B onset was read".to_owned()),
        (_, None) => None,
    };
    if b_gain_db != 0.0 {
        let (low, high) = (10f64.powf((b_gain_db - 1.5) / 10.0), 10f64.powf((b_gain_db + 1.5) / 10.0));
        match (b_power, power) {
            (Some(b_power), Some((_, steady))) if steady > 0.0 && (low..=high).contains(&(b_power / steady)) => {}
            (Some(b_power), Some((_, steady))) if steady > 0.0 => {
                return Verdict::Fail(format!("B's power {b_power:.3e} is {:.1} dB above A's steady {steady:.3e}, not {b_gain_db} ± 1.5 dB", 10.0 * (b_power / steady).log10()));
            }
            _ => return Verdict::Inconclusive("B's or A's power was not measured: the gain change is not shown".to_owned()),
        }
    }
    if let Some(off) = onset_off {
        return Verdict::Inconclusive(format!(
            "A's tail holds at {a_tone:.0} Hz ({ratio:?} of its power, ACK {ack_delta:+} from its end {e}) and B is at its tone, but {off}: onset precision, not the claim"
        ));
    }
    Verdict::Pass(format!("A's tail holds at {a_tone:.0} Hz ({ratio:?} of its power, ACK {ack_delta:+} from its end {e}) and B starts at its tick"))
}

/// The furthest a receive restart's tone may rise after its booked origin, in samples. The device delivers about 13 exact
/// zeros and about 10 rising samples at each restart (DDC filter fill, INFERRED, bench 2026-11-05: onset 20 samples at
/// 1 MS/s on UBX and OBX), so 40 leaves margin without hiding a start that slipped by more.
pub const RESTART_ONSET_MAX_SAMPLES: i64 = 40;

/// T2's tone at a receive restart: the samples from the booked `origin` (at `per` ticks a sample) must rise above 4 x the
/// quiet `floor` within `RESTART_ONSET_MAX_SAMPLES`, and hold `new_hz` over every window of the first millisecond from five samples after the rise (a window at
/// the old frequency there fails that check). Returns the verdict and the
/// fill it saw (exact-zero samples, onset sample), both counted from `origin`.
pub fn restart_tone_verdict(samples: &[(i64, ezsdr_radio_uhd::Iq)], origin: i64, per: i64, floor: f64, new_hz: f64) -> (Verdict, Option<(i64, i64)>) {
    let restart: Vec<_> = samples.iter().filter(|(tick, _)| *tick >= origin).copied().collect();
    let power = |s: &ezsdr_radio_uhd::Iq| f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2);
    let Some(index) = restart.iter().position(|(_, s)| power(s) > 4.0 * floor) else {
        return (Verdict::Inconclusive("no sample of the new segment rises above 4 x the noise".to_owned()), None);
    };
    let onset = (restart[index].0 - origin) / per;
    let zeros = restart.iter().take_while(|(_, s)| s[0] == 0.0 && s[1] == 0.0).count() as i64;
    let fill = Some((zeros, onset));
    if onset > RESTART_ONSET_MAX_SAMPLES {
        return (Verdict::Fail(format!("the tone rises {onset} samples after the booked origin, more than {RESTART_ONSET_MAX_SAMPLES}")), fill);
    }
    let from = restart[index].0 + 5 * per;
    let settled: Vec<_> = restart.iter().filter(|(tick, _)| *tick >= from).copied().collect();
    (first_window_verdict(&settled, from, per, new_hz, TONE_TOLERANCE_HZ, floor), fill)
}

/// A call's wall-clock durations, as the control-call probe prints them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallStats {
    pub min_ns: u64,
    pub median_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

/// `CallStats` of `durations`; `None` for none.
pub fn call_stats(durations: &[u64]) -> Option<CallStats> {
    let mut sorted = durations.to_vec();
    sorted.sort_unstable();
    let last = sorted.len().checked_sub(1)?;
    Some(CallStats { min_ns: sorted[0], median_ns: sorted[sorted.len() / 2], p99_ns: sorted[last * 99 / 100], max_ns: sorted[last] })
}

/// Where a receive buffer's overflow was found between the probe's two ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallBracket {
    /// The first trial (100 ms) already overflowed.
    BelowFirst { first_ms: u64 },
    /// No trial through the last (5 s) overflowed.
    ThroughLast { longest_ms: u64 },
    /// The longest stall without an overflow and the shortest with one.
    Between { longest_ok_ms: u64, shortest_overflow_ms: u64 },
}

/// Classifies the buffer probe's trials `(stall ms, overflowed)`; `None` for no trial.
pub fn overflow_bracket(trials: &[(u64, bool)]) -> Option<StallBracket> {
    let mut sorted = trials.to_vec();
    sorted.sort_unstable();
    let (first_ms, _) = *sorted.first()?;
    let overflowed: Vec<u64> = sorted.iter().filter(|(_, overflowed)| *overflowed).map(|(ms, _)| *ms).collect();
    let Some(&shortest_overflow_ms) = overflowed.first() else {
        return Some(StallBracket::ThroughLast { longest_ms: sorted.last().map(|(ms, _)| *ms)? });
    };
    let longest_ok_ms = sorted.iter().filter(|(ms, overflowed)| !*overflowed && *ms < shortest_overflow_ms).map(|(ms, _)| *ms).max();
    Some(match longest_ok_ms {
        Some(longest_ok_ms) => StallBracket::Between { longest_ok_ms, shortest_overflow_ms },
        None => StallBracket::BelowFirst { first_ms },
    })
}

/// Whether the transmit rate probe's tone is still at `expected_hz` in `[from, to)` at `per` ticks a sample.
pub fn tone_hold_verdict(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, to: i64, per: i64, expected_hz: f64) -> Verdict {
    let freqs: Vec<(i64, f64)> = contiguous_windows_at(samples, per)
        .filter(|w| w[0].0 >= from && w[0].0 + 25 * per <= to)
        .filter_map(|w| {
            let (coherence, hz) = window_tone_at(w, per);
            (coherence > 0.5).then_some((w[0].0, hz))
        })
        .collect();
    match median_freq(&freqs, from, to) {
        Some(hz) if (hz - expected_hz).abs() <= TONE_TOLERANCE_HZ => Verdict::Pass(format!("the tone holds at {hz:.0} Hz")),
        Some(hz) => Verdict::Fail(format!("the tone is at {hz:.0} Hz, not {expected_hz:.0} Hz")),
        None => Verdict::Inconclusive("no coherent tone in the window".to_owned()),
    }
}

/// Whether a late-command count is zero: the restart kept its planned origin.
pub fn restart_late_verdict(late_commands: usize) -> Verdict {
    if late_commands == 0 { Verdict::Pass("no radio.LATE_COMMAND".to_owned()) } else { Verdict::Fail(format!("{late_commands} radio.LATE_COMMAND event(s)")) }
}

/// A receive stream's blocks, `(first tick, samples)`, advance without a hole between `from` and `to`: the value
/// submitted while it streamed must not restart it.
pub fn continuity_verdict(blocks: &[(i64, usize)], per: i64, from: i64, to: i64) -> Verdict {
    let inside: Vec<&(i64, usize)> = blocks.iter().filter(|(tick, _)| *tick >= from && *tick < to).collect();
    for pair in inside.windows(2) {
        let expected = pair[0].0 + pair[0].1 as i64 * per;
        if pair[1].0 != expected {
            return Verdict::Fail(format!("a block begins at {} where the block before it ends at {expected}", pair[1].0));
        }
    }
    if inside.is_empty() { Verdict::Inconclusive("no blocks in the window".to_owned()) } else { Verdict::Pass(format!("{} blocks advance without a hole", inside.len())) }
}

/// The stage-3 receive value submitted while the stream runs: the Kernel admitted it and the Provider refused it with
/// one `radio.COMMAND_REJECTED` naming its Action.
pub fn streaming_value_verdict(kernel_admitted: bool, refusals_with_id: usize) -> Verdict {
    if !kernel_admitted {
        return Verdict::Fail("the Kernel refused the value instead of admitting it and letting the Provider refuse it".to_owned());
    }
    match refusals_with_id {
        1 => Verdict::Pass("one radio.COMMAND_REJECTED names the Action".to_owned()),
        0 => Verdict::Fail("no radio.COMMAND_REJECTED names the Action".to_owned()),
        count => Verdict::Fail(format!("{count} radio.COMMAND_REJECTED events name the Action")),
    }
}

/// A restarted segment's origin taken from the booked segment (T2's planned `start_rx` instant, T3's registered
/// SampleClock), not from the device call. The device start, the first published block and the Sink's `GAP_BEFORE`
/// (a first block later than the booked origin) are all judged against it.
pub fn restart_origin_verdict(expected: Option<i64>, device_start: Option<i64>, first_block: Option<i64>, gap_before: bool) -> Verdict {
    let Some(expected) = expected else {
        return Verdict::Inconclusive("no booked segment records the restart's origin".to_owned());
    };
    match device_start {
        Some(at) if at == expected => {}
        Some(at) => return Verdict::Fail(format!("the device start is at {at} for the booked origin {expected}")),
        None => return Verdict::Inconclusive("no rx_start row records the restart".to_owned()),
    }
    match first_block {
        Some(first) if first == expected => {}
        Some(first) => return Verdict::Fail(format!("the first published block is at {first} for the booked origin {expected}")),
        None => return Verdict::Inconclusive("no block after the restart was read".to_owned()),
    }
    if gap_before {
        return Verdict::Fail(format!("the restart's first block carries GAP_BEFORE for the booked origin {expected}"));
    }
    Verdict::Pass(format!("the device start and the first block are at the booked origin {expected}, without GAP_BEFORE"))
}

/// The configure's read-back of one value: `claimed` was asked, `read_back` is what the device reported and
/// `difference` their difference (the Manifest's `applied` row).
pub fn readback_verdict(asked: f64, claimed: Option<f64>, read_back: Option<f64>, difference: Option<f64>) -> Verdict {
    match (claimed, read_back, difference) {
        (Some(claimed), Some(read_back), Some(difference)) if claimed == asked && read_back == asked && difference == 0.0 => {
            Verdict::Pass(format!("the configure claimed {claimed} and the device read back {read_back}"))
        }
        (Some(claimed), Some(read_back), Some(difference)) => Verdict::Fail(format!("the configure claimed {claimed}, read back {read_back}, difference {difference} for the asked {asked}")),
        _ => Verdict::Inconclusive("the restarted segment has no applied row for this value".to_owned()),
    }
}

/// The receive start-lead group's value: the smallest candidate lead at which every stage's restart stayed at its
/// origin, or `reason` why no candidate did.
pub fn start_lead_value_verdict(worst: Option<f64>, reason: Option<&str>) -> Verdict {
    match worst {
        Some(ns) => Verdict::Pass(format!("the smallest candidate lead is {ns} ns")),
        None => Verdict::Inconclusive(format!("no candidate lead passed: {}", reason.unwrap_or("no reason recorded"))),
    }
}

/// First sample of `blocks` at or after `from`, `None` when none.
pub fn first_tick_at_or_after(blocks: &[(i64, usize)], from: i64) -> Option<i64> {
    blocks.iter().filter(|(tick, _)| *tick >= from).map(|(tick, _)| *tick).min()
}

/// A `harness` note for the report: the tone of `blocks` in `[from, to)` (JSON), or why none.
pub fn tone_row(samples: &[(i64, ezsdr_radio_uhd::Iq)], from: i64, to: i64, per: i64) -> Json {
    serde_json::json!({ "from": from, "to": to, "first_ms_hz": first_ms_frequency(samples, from, per), "samples": samples.iter().filter(|(tick, _)| *tick >= from && *tick < to).count() })
}

/// Whether a drain's outcomes end with two consecutive timeouts: the stream has stopped. `events` holds every
/// outcome in order, samples included (`"samples"`), so a packet between two timeouts resets the quiet interval.
pub fn drain_quiet(events: &[String]) -> bool {
    events.len() >= 2 && events[events.len() - 2..].iter().all(|event| event == "Timeout")
}

/// What one buffer-stall trial shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallOutcome {
    /// The device reported an overflow.
    Overflow,
    /// No overflow, and the drain's samples span the stall: ample buffering.
    NoOverflow,
    /// Neither: the trial says nothing about the buffer and is not used.
    Incomplete,
}

/// Classifies a buffer stall from the drain's first sample tick and end tick, the device tick the stall began at,
/// its length and the tolerance: no overflow counts only when the drained samples start by the stall and reach its
/// end, so a receiver that stopped streaming is not read as ample buffering.
pub fn stall_outcome(overflow: bool, first: Option<i64>, end: Option<i64>, stall_start: i64, stall_ms: u64, ticks_per_ms: i64, slack_ticks: i64) -> StallOutcome {
    if overflow {
        return StallOutcome::Overflow;
    }
    let (Some(first), Some(end)) = (first, end) else {
        return StallOutcome::Incomplete;
    };
    let stall_end = stall_start.saturating_add(i64::try_from(stall_ms).unwrap_or(i64::MAX).saturating_mul(ticks_per_ms));
    if first > stall_start.saturating_add(slack_ticks) || end.saturating_add(slack_ticks) < stall_end {
        return StallOutcome::Incomplete;
    }
    StallOutcome::NoOverflow
}

/// Whether a receive stream restarted at `start` streamed from it without a hole through `end` (blocks are
/// `(first tick, samples)`): a trial's continuity and progress before a buffer stall.
pub fn restart_streamed(blocks: &[(i64, usize)], per: i64, start: i64, end: i64) -> bool {
    let Some(first) = blocks.first() else { return false };
    if first.0 != start {
        return false;
    }
    if blocks.windows(2).any(|pair| pair[1].0 != pair[0].0 + pair[0].1 as i64 * per) {
        return false;
    }
    blocks.last().is_some_and(|(tick, samples)| tick + *samples as i64 * per >= end)
}

/// One send of a burst: the samples from `offset` to offer and whether this send closes the burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BurstSend {
    pub offset: usize,
    pub len: usize,
    pub eob: bool,
    /// The closing send carries a real sample plus one zero: a short acceptance left a one-sample tail.
    pub padded: bool,
}

/// The next send of a `total`-sample burst whose first `offset` samples the device accepted, one send at most
/// `chunk` samples. The last two samples are kept for the closing send, which always has at least two samples and
/// carries the EOB (a one-sample tail is padded with a zero, as the Module's own end-of-burst path does). `None`
/// when the burst is complete.
pub fn burst_send(offset: usize, total: usize, chunk: usize) -> Option<BurstSend> {
    if offset >= total || chunk == 0 {
        return None;
    }
    let remaining = total - offset;
    if remaining <= 2 {
        return Some(BurstSend { offset, len: 2, eob: true, padded: remaining < 2 });
    }
    Some(BurstSend { offset, len: chunk.min(remaining - 2), eob: false, padded: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(first: i64, lengths: &[usize], per: i64) -> Vec<(i64, usize)> {
        let mut out = Vec::new();
        let mut tick = first;
        for length in lengths {
            out.push((tick, *length));
            tick += *length as i64 * per;
        }
        out
    }

    #[test]
    fn a_receive_rate_change_reads_back_starts_at_its_tick_and_steps_by_the_new_rate() {
        let blocks = blocks(1_000, &[2_000, 2_000, 500], 100);
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 100, Some(100e3), 100e3, 1, true), Verdict::Pass(_)));
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 100, Some(100e3), 100e3, 1, false), Verdict::Inconclusive(_)), "a failed drain");
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_200, &blocks, 100, Some(100e3), 100e3, 1, true), Verdict::Fail(_)), "a first block elsewhere");
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 101, Some(100e3), 100e3, 1, true), Verdict::Fail(_)), "a wrong tick spacing");
        assert!(matches!(rx_rate_verdict(2e6, 1e6, 1_000, &blocks, 100, Some(100e3), 100e3, 1, true), Verdict::Fail(_)), "a wrong read-back");
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 100, Some(70e3), 100e3, 1, true), Verdict::Fail(_)), "a moved tone");
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 100, Some(100e3), 100e3, 2, true), Verdict::Fail(_)), "two opens");
        assert!(matches!(rx_rate_verdict(2e6, 2e6, 1_000, &blocks, 100, None, 100e3, 1, true), Verdict::Inconclusive(_)), "no tone");
    }

    #[test]
    fn a_drain_is_quiet_only_after_two_timeouts_with_no_packet_between_them() {
        let events = |items: &[&str]| items.iter().map(|item| (*item).to_owned()).collect::<Vec<String>>();
        assert!(drain_quiet(&events(&["Timeout", "Timeout"])));
        assert!(drain_quiet(&events(&["samples", "Timeout", "Timeout"])));
        assert!(!drain_quiet(&events(&["Timeout", "samples", "Timeout"])), "a packet between the timeouts resets the quiet interval");
        assert!(!drain_quiet(&events(&["Timeout"])), "one timeout is not yet quiet");
        assert!(!drain_quiet(&events(&["samples"])));
    }

    #[test]
    fn a_buffer_stall_counts_only_when_it_overflowed_or_the_drain_spans_the_stall() {
        let per_ms = 200_000;
        let start = 1_000_000;
        assert_eq!(stall_outcome(true, Some(start + 5_000), Some(start + 50 * per_ms), start, 50, per_ms, 10_000), StallOutcome::Overflow);
        assert_eq!(stall_outcome(false, Some(start), Some(start + 50 * per_ms), start, 50, per_ms, 10_000), StallOutcome::NoOverflow);
        assert_eq!(stall_outcome(false, None, None, start, 50, per_ms, 10_000), StallOutcome::Incomplete, "a drain that yielded nothing");
        assert_eq!(stall_outcome(false, Some(start), Some(start + 20 * per_ms), start, 50, per_ms, 10_000), StallOutcome::Incomplete, "a drain that stopped inside the stall");
        assert_eq!(stall_outcome(false, Some(start + 40 * per_ms), Some(start + 50 * per_ms), start, 50, per_ms, 10_000), StallOutcome::Incomplete, "a drain that starts after the stall");
    }

    #[test]
    fn a_restarted_stream_needs_its_start_and_an_unbroken_run_through_the_progress_target() {
        let blocks = blocks(1_000, &[2_000, 2_000], 100);
        assert!(restart_streamed(&blocks, 100, 1_000, 400_000));
        assert!(!restart_streamed(&blocks, 100, 1_000, 500_000), "the run stops before the target");
        assert!(!restart_streamed(&blocks, 100, 1_200, 400_000), "the first block is not at the start");
        assert!(!restart_streamed(&blocks, 101, 1_000, 400_000), "a hole between the blocks");
        assert!(!restart_streamed(&[], 100, 1_000, 400_000));
    }

    #[test]
    fn a_burst_send_keeps_a_two_sample_closing_send_after_a_short_acceptance() {
        // 20 001 samples: the payload send never takes the last two, and the close is two samples with the EOB.
        let first = burst_send(0, 20_001, 20_000).expect("a first send");
        assert_eq!((first.len, first.eob, first.padded), (19_999, false, false));
        let short = 19_998; // the device accepted one sample less than offered
        let retried = burst_send(short, 20_001, 20_000).expect("the retried send");
        assert_eq!((retried.offset, retried.len, retried.eob), (short, 1, false), "the remainder goes out without the EOB");
        let close = burst_send(short + retried.len, 20_001, 20_000).expect("the closing send");
        assert_eq!((close.len, close.eob, close.padded), (2, true, false));
        assert_eq!(burst_send(20_001, 20_001, 20_000), None);
        let padded = burst_send(20_000, 20_001, 20_000).expect("a one-sample tail");
        assert_eq!((padded.len, padded.eob, padded.padded), (2, true, true), "a one-sample tail is padded to two samples");
    }

    #[test]
    fn a_transmit_rate_change_needs_the_ack_the_two_tones_and_b_the_loop_delay() {
        let pass = |a_ack, a_tail, b_onset, b_tone, time_errors, underflows, opens| {
            tx_rate_verdict(2e6, 2e6, a_ack, 9_000, 100, a_tail, 100e3, b_onset, Some(20_044), 10, b_tone, 100e3, 1, 1, time_errors, underflows, opens)
        };
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Pass(_)));
        // The ACK's window is [e − 2 × per, e + 1] at 2 MS/s: one sample early is the radio's own ACK tick, and the
        // two edges still confirm the end.
        assert!(matches!(pass(Some(8_900), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Pass(_)), "one sample early");
        assert!(matches!(pass(Some(8_800), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Pass(_)), "the window's early edge");
        assert!(matches!(pass(Some(9_001), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Pass(_)), "the window's late edge");
        assert!(matches!(pass(Some(8_799), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Inconclusive(_)), "an ACK before the window");
        assert!(matches!(pass(Some(9_002), Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Inconclusive(_)), "an ACK past the window");
        assert!(matches!(pass(Some(9_000), Some(50e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Fail(_)), "A's tail moved");
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_500), Some(100e3), 0, 0, 1), Verdict::Inconclusive(_)), "B late beyond the loop delay");
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_040), Some(50e3), 0, 0, 1), Verdict::Fail(_)), "B's tone moved");
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_040), Some(100e3), 1, 0, 1), Verdict::Fail(_)), "a TIME_ERROR");
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_040), Some(100e3), 0, 1, 1), Verdict::Fail(_)), "an underflow");
        assert!(matches!(pass(Some(9_000), Some(100e3), Some(20_040), Some(100e3), 0, 0, 2), Verdict::Fail(_)), "two opens");
        assert!(matches!(pass(None, Some(100e3), Some(20_040), Some(100e3), 0, 0, 1), Verdict::Inconclusive(_)), "no ACK");
        // A duplicate ACK for A is a second in A's window, not the one that confirms the burst.
        let duplicate = tx_rate_verdict(2e6, 2e6, Some(9_000), 9_000, 100, Some(100e3), 100e3, Some(20_040), Some(20_044), 10, Some(100e3), 100e3, 2, 1, 0, 0, 1);
        assert!(matches!(duplicate, Verdict::Fail(_)), "two ACKs for A");
        let no_b_ack = tx_rate_verdict(2e6, 2e6, Some(9_000), 9_000, 100, Some(100e3), 100e3, Some(20_040), Some(20_044), 10, Some(100e3), 100e3, 1, 0, 0, 0, 1);
        assert!(matches!(no_b_ack, Verdict::Inconclusive(_)), "B's ACK not read");
    }

    #[test]
    fn a_burst_ack_confirms_an_end_only_inside_its_window_and_once_per_burst() {
        let ack = |tick| ezsdr_radio_uhd::TxReport { code: ezsdr_radio_uhd::TxCode::BurstAck, tick: Some(tick), channel: 0 };
        let reports = [ack(9_900), ezsdr_radio_uhd::TxReport { code: ezsdr_radio_uhd::TxCode::TimeError, tick: Some(9_950), channel: 0 }, ack(9_950), ack(10_000), ack(5_000)];
        assert_eq!(burst_acks_in(&reports, 9_000, 10_001), 3, "two ACKs in the window are both counted; the earlier burst's is not");
        assert_eq!(burst_acks_in(&reports, 12_000, 13_000), 0, "no ACK in the window");
    }

    #[test]
    fn a_setting_after_the_burst_end_leaves_the_tail_or_the_control_shows_the_hazard() {
        let bounds = (10f64.powf(-TAIL_POWER_DB / 10.0), 10f64.powf(TAIL_POWER_DB / 10.0));
        let pass = |m, ack, tail, tail_power, move_at, b_tone, b_onset| {
            let (issued, returned) = if m < 0 { (9_000, 9_500) } else { (10_100, 10_200) };
            tail_verdict(m, 10_000, 200, issued, returned, ack, 100e3, tail, Some((tail_power, 1.0)), bounds.0, bounds.1, move_at, b_tone, 200e3, b_onset, Some(20_040), 10, None, 0.0)
        };
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Pass(_)));
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.85, None, Some(200e3), Some(20_040)), Verdict::Pass(_)), "−0.7 dB, inside ±1 dB");
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.75, None, Some(200e3), Some(20_040)), Verdict::Fail(_)), "−1.2 dB, outside ±1 dB");
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.9, Some(9_000), Some(200e3), Some(20_040)), Verdict::Fail(_)), "a move before e");
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.1, None, Some(200e3), Some(20_040)), Verdict::Fail(_)), "a truncated tail");
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.9, None, Some(50e3), Some(20_040)), Verdict::Fail(_)), "B's tone");
        assert!(matches!(pass(5, Some(10_000), Some(100e3), 0.9, None, Some(200e3), Some(21_000)), Verdict::Inconclusive(_)), "B late");
        assert!(matches!(pass(-5, Some(10_000), Some(50e3), 0.1, Some(9_000), Some(200e3), Some(20_040)), Verdict::Pass(_)), "the control shows the hazard");
        assert!(matches!(pass(-5, Some(10_000), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Inconclusive(_)), "the control did not show it");
        // The ACK's window at 1 MS/s is [e − 400, e + 1]: both edges confirm A's end, and an ACK outside it is not
        // evidence of a different failure (the burst may simply have been cut short).
        assert!(matches!(pass(5, Some(9_800), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Pass(_)), "one sample early");
        assert!(matches!(pass(5, Some(9_600), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Pass(_)), "the window's early edge");
        assert!(matches!(pass(5, Some(9_599), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Inconclusive(_)), "an ACK before the window");
        assert!(matches!(pass(5, Some(10_001), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Pass(_)), "the window's late edge");
        assert!(matches!(pass(5, Some(10_002), Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Inconclusive(_)), "an ACK past the window");
        assert!(matches!(pass(5, None, Some(100e3), 0.9, None, Some(200e3), Some(20_040)), Verdict::Inconclusive(_)), "no ACK");
        // The control's bracket decides whether it was the control at all: issued or returned at/after A's end is
        // not a −5 ms setting and must not pass, whether or not the tail moved.
        let late_control = |issued, returned| {
            tail_verdict(
                -5,
                10_000,
                200,
                issued,
                returned,
                Some(10_000),
                100e3,
                Some(50e3),
                Some((0.1, 1.0)),
                bounds.0,
                bounds.1,
                Some(9_000),
                Some(200e3),
                200e3,
                Some(20_040),
                Some(20_040),
                10,
                None,
                0.0,
            )
        };
        assert!(matches!(late_control(9_000, 9_500), Verdict::Pass(_)), "a bracket wholly before e");
        assert!(matches!(late_control(9_500, 10_000), Verdict::Inconclusive(_)), "returned at e");
        assert!(matches!(late_control(10_100, 10_200), Verdict::Inconclusive(_)), "issued after e");
        // A non-negative trial whose setting really went out before A's end is not the measured offset.
        let early =
            tail_verdict(5, 10_000, 200, 9_000, 9_500, Some(10_000), 100e3, Some(100e3), Some((0.9, 1.0)), bounds.0, bounds.1, None, Some(200e3), 200e3, Some(20_040), Some(20_040), 10, None, 0.0);
        assert!(matches!(early, Verdict::Inconclusive(_)), "an issue before e");
        // The gain trial: A's tail unchanged within 1 dB and B's power +6 dB ± 1.5 dB above A's steady.
        let gain = |b_power| {
            tail_verdict(
                5,
                10_000,
                200,
                10_100,
                10_200,
                Some(10_000),
                100e3,
                Some(100e3),
                Some((0.9, 1.0)),
                bounds.0,
                bounds.1,
                None,
                Some(100e3),
                100e3,
                Some(20_040),
                Some(20_040),
                10,
                Some(b_power),
                6.0,
            )
        };
        assert!(matches!(gain(4.0 / 1.0), Verdict::Pass(_)), "+6 dB");
        assert!(matches!(gain(1.0), Verdict::Fail(_)), "unchanged");
        assert!(matches!(gain(8.0), Verdict::Fail(_)), "+9 dB, outside 1.5 dB");
        assert!(matches!(gain(f64::NAN), Verdict::Fail(_)), "no power");
        // B's onset is judged after the gain: an onset off the loop delay never hides a wrong gain.
        let off = tail_verdict(
            5,
            10_000,
            200,
            10_100,
            10_200,
            Some(10_000),
            100e3,
            Some(100e3),
            Some((0.9, 1.0)),
            bounds.0,
            bounds.1,
            None,
            Some(100e3),
            100e3,
            Some(21_000),
            Some(20_040),
            10,
            Some(1.0),
            6.0,
        );
        assert!(matches!(off, Verdict::Fail(_)), "a late onset and an unchanged gain");
    }

    /// Samples from `origin` at 1 MS/s: `zeros` exact zeros, ten rising samples, then a tone at `hz` (amplitude 0.02).
    fn restart_samples(origin: i64, zeros: usize, hz: f64) -> Vec<(i64, ezsdr_radio_uhd::Iq)> {
        (0..3_000usize)
            .map(|i| {
                let amplitude = if i < zeros { 0.0 } else { 0.02 * (((i - zeros + 1) as f64) / 10.0).min(1.0) };
                let phase = 2.0 * std::f64::consts::PI * hz * i as f64 / 1e6;
                (origin + i as i64 * 200, [(amplitude * phase.cos()) as f32, (amplitude * phase.sin()) as f32])
            })
            .collect()
    }

    #[test]
    fn a_receive_restart_rises_at_the_new_tone_after_the_device_fill() {
        let origin = 1_000_000;
        let (pass, fill) = restart_tone_verdict(&restart_samples(origin, 13, 50e3), origin, 200, 1e-8, 50e3);
        assert!(matches!(pass, Verdict::Pass(_)), "zeros, a rise, then 50 kHz: {pass:?}");
        assert_eq!(fill, Some((13, 13)), "13 exact zeros, onset at the first rising sample");
        let (old, _) = restart_tone_verdict(&restart_samples(origin, 13, 100e3), origin, 200, 1e-8, 50e3);
        assert!(matches!(old, Verdict::Fail(_)), "the old 100 kHz right after the fill");
        let (late, _) = restart_tone_verdict(&restart_samples(origin, 60, 50e3), origin, 200, 1e-8, 50e3);
        assert!(matches!(late, Verdict::Fail(_)), "an onset 60 samples after the booked origin");
        let (edge, _) = restart_tone_verdict(&restart_samples(origin, RESTART_ONSET_MAX_SAMPLES as usize, 50e3), origin, 200, 1e-8, 50e3);
        assert!(matches!(edge, Verdict::Pass(_)), "an onset exactly at the bound: {edge:?}");
        let (none, fill) = restart_tone_verdict(&restart_samples(origin, 3_000, 50e3), origin, 200, 1e-8, 50e3);
        assert!(matches!(none, Verdict::Inconclusive(_)) && fill.is_none(), "no onset");
    }

    #[test]
    fn call_stats_order_and_percentile() {
        assert_eq!(call_stats(&[]), None);
        let stats = call_stats(&[3, 1, 2, 100]).unwrap();
        assert_eq!((stats.min_ns, stats.median_ns, stats.p99_ns, stats.max_ns), (1, 3, 3, 100));
        let stats = call_stats(&(1..=100).collect::<Vec<u64>>()).unwrap();
        assert_eq!((stats.min_ns, stats.median_ns, stats.p99_ns, stats.max_ns), (1, 51, 99, 100));
    }

    #[test]
    fn the_buffer_bracket_reports_its_two_ends_or_the_trial_that_saw_an_overflow() {
        assert_eq!(overflow_bracket(&[]), None);
        assert_eq!(overflow_bracket(&[(100, false), (5_000, false)]), Some(StallBracket::ThroughLast { longest_ms: 5_000 }));
        assert_eq!(overflow_bracket(&[(100, true)]), Some(StallBracket::BelowFirst { first_ms: 100 }));
        assert_eq!(overflow_bracket(&[(100, false), (300, false), (600, true), (5_000, true)]), Some(StallBracket::Between { longest_ok_ms: 300, shortest_overflow_ms: 600 }));
    }

    #[test]
    fn the_stage_three_verdicts_name_their_failure() {
        assert!(matches!(streaming_value_verdict(true, 1), Verdict::Pass(_)));
        assert!(matches!(streaming_value_verdict(false, 1), Verdict::Fail(_)));
        assert!(matches!(streaming_value_verdict(true, 0), Verdict::Fail(_)));
        assert!(matches!(restart_origin_verdict(Some(10), Some(10), Some(10), false), Verdict::Pass(_)));
        assert!(matches!(restart_origin_verdict(Some(10), Some(10), Some(10), true), Verdict::Fail(_)), "a GAP_BEFORE");
        assert!(matches!(restart_origin_verdict(Some(10), Some(11), Some(11), false), Verdict::Fail(_)), "the device start 1 tick off");
        assert!(matches!(restart_origin_verdict(Some(10), Some(11), Some(10), false), Verdict::Fail(_)), "the device start 1 tick off, the first block at the origin");
        assert!(matches!(restart_origin_verdict(Some(10), Some(10), Some(11), false), Verdict::Fail(_)), "the first block 1 tick off");
        assert!(matches!(restart_origin_verdict(None, Some(10), Some(10), false), Verdict::Inconclusive(_)));
        assert!(matches!(restart_origin_verdict(Some(10), None, Some(10), false), Verdict::Inconclusive(_)));
        assert!(matches!(restart_origin_verdict(Some(10), Some(10), None, false), Verdict::Inconclusive(_)));
        assert!(matches!(readback_verdict(1e9, Some(1e9), Some(1e9), Some(0.0)), Verdict::Pass(_)));
        assert!(matches!(readback_verdict(1e9, Some(1e9), Some(1.1e9), Some(1e8)), Verdict::Fail(_)));
        assert!(matches!(readback_verdict(1e9, None, None, None), Verdict::Inconclusive(_)));
        assert!(matches!(restart_late_verdict(0), Verdict::Pass(_)));
        assert!(matches!(restart_late_verdict(1), Verdict::Fail(_)));
        assert!(matches!(start_lead_value_verdict(Some(5e7), None), Verdict::Pass(_)));
        assert!(matches!(start_lead_value_verdict(None, Some("every stage failed")), Verdict::Inconclusive(_)));
        assert_eq!(first_tick_at_or_after(&[(1, 3), (7, 3)], 5), Some(7));
    }

    #[test]
    fn continuity_and_the_hold_verdict_read_a_hole_and_a_moved_tone() {
        let blocks = blocks(1_000, &[100, 100], 200);
        assert!(matches!(continuity_verdict(&blocks, 200, 1_000, 42_000), Verdict::Pass(_)));
        assert!(matches!(continuity_verdict(&blocks, 201, 1_000, 42_000), Verdict::Fail(_)), "a hole");
        assert!(matches!(continuity_verdict(&blocks, 200, 50_000, 60_000), Verdict::Inconclusive(_)), "no block");
        let tone: Vec<(i64, ezsdr_radio_uhd::Iq)> = (0..4_000)
            .map(|i| (1_000 + i * 200, [(0.4 * (2.0 * std::f64::consts::PI * 100e3 * i as f64 / 1e6).cos()) as f32, (0.4 * (2.0 * std::f64::consts::PI * 100e3 * i as f64 / 1e6).sin()) as f32]))
            .collect();
        assert!(matches!(tone_hold_verdict(&tone, 1_000, 800_000, 200, 100e3), Verdict::Pass(_)));
        assert!(matches!(tone_hold_verdict(&tone, 1_000, 800_000, 200, 50e3), Verdict::Fail(_)));
    }
}
