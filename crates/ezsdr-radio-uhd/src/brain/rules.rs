//! The executors' pure rules (note 45 §4.6): what a publish takes from a recv, the consumption
//! of debt and left values by a packet (through the production `RxChain::consume`), and the
//! loop checks that discard an executor's FIFO when the failure flag is set (I1).

use std::collections::VecDeque;

use ezsdr_radio::timeline::Channels;

use crate::device::Iq;
use crate::provider::core::{RxChain, RxEntry, SegmentKey};

/// What a publish reaches: the progress after it and the samples it discards.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) progress: i64,
    pub(crate) discard: u64,
}

/// The samples from `origin` up to `bound` are published, and the progress passes them; the rest are discarded.
pub(crate) fn plan_publish(origin: i64, bound: Option<i64>, samples: &[i64], progress: i64) -> Plan {
    let mut plan = Plan { progress, discard: 0 };
    for &tick in samples {
        if tick >= origin && bound.is_none_or(|bound| tick < bound) {
            plan.progress = plan.progress.max(tick + 1);
        } else {
            plan.discard += 1;
        }
    }
    plan
}

/// The consumption evidence of one packet: `(value id, sample tick)` for each entry the production
/// rule removes from `left` or `debt`. The state is rebuilt in a scratch chain, so the rule is
/// `RxChain::consume` itself (no copy of it), and each sample is one call.
pub(crate) fn consume_packet(left: &[RxEntry], debt: &[RxEntry], sample_ticks: i64, channels: Channels, first_tick: i64, samples: &[Vec<Iq>]) -> Vec<(u64, i64)> {
    let segment = SegmentKey { by: None, origin: i64::MIN };
    let mut chain = RxChain::default();
    chain.adopt_start(segment, i64::MIN);
    for entry in left {
        hand(&mut chain, entry);
    }
    chain.stop_result(Ok(())).expect("the scratch chain stops");
    chain.finish();
    chain.adopt_start(segment, i64::MIN);
    for entry in debt {
        hand(&mut chain, entry);
    }
    let mut consumed = Vec::new();
    let count = samples.first().map_or(0, Vec::len);
    for index in 0..count {
        let tick = first_tick + index as i64 * sample_ticks;
        let before = orders(&chain);
        let one: Vec<Vec<Iq>> = samples.iter().map(|channel| vec![channel[index]]).collect();
        chain.consume(sample_ticks, channels, tick, &one);
        let after = orders(&chain);
        consumed.extend(before.into_iter().filter(|order| !after.contains(order)).map(|order| (order.1, tick)));
    }
    consumed
}

/// Puts `entry` into the scratch chain's debt, keeping its acceptance instant.
fn hand(chain: &mut RxChain, entry: &RxEntry) {
    chain.hand(entry.clone(), || Ok(()), || entry.accepted_at.unwrap_or(i64::MIN)).expect("the scratch chain applies");
}

fn orders(chain: &RxChain) -> Vec<(i64, u64)> {
    chain.left().iter().chain(chain.debt()).map(|entry| entry.order).collect()
}

/// The outcome of an executor's loop check.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Check {
    Proceed,
    Exit,
}

/// The loop check of a receive executor (I1): a set failure flag discards the FIFO and exits
/// before any call.
pub(crate) fn loop_check<T>(failure: bool, fifo: &mut VecDeque<T>) -> Check {
    if failure {
        fifo.clear();
        Check::Exit
    } else {
        Check::Proceed
    }
}

/// The loop check of the settings executor (I1): a set failure flag abandons the command in hand
/// and every queued one, each through `abandon` (one cell section per command), and exits.
pub(crate) fn settings_check(failure: bool, in_hand: Option<u64>, fifo: &mut VecDeque<u64>, mut abandon: impl FnMut(u64)) -> Check {
    if !failure {
        return Check::Proceed;
    }
    for id in in_hand.into_iter().chain(fifo.drain(..)) {
        abandon(id);
    }
    Check::Exit
}

#[cfg(test)]
mod tests {
    use super::super::cell::Terminal;
    use super::super::fixtures::value;
    use super::*;

    fn zeros(channels: usize, samples: usize) -> Vec<Vec<Iq>> {
        vec![vec![[0.0, 0.0]; samples]; channels]
    }

    fn with_accepted(mut entry: RxEntry, accepted: i64) -> RxEntry {
        entry.accepted_at = Some(accepted);
        entry
    }

    /// The last sample of a packet is consumption evidence too (E1-49).
    #[test]
    fn u12e_last_sample_of_packet_is_evidence() {
        let consumed = consume_packet(&[], &[value(0, 8, 1)], 1, Channels::first(1), 8, &zeros(1, 1));
        assert_eq!(consumed, vec![(1, 8)]);
    }

    #[test]
    fn u12a_equal_time_values_take_distinct_samples() {
        let debt = [value(0, 8, 1), value(0, 8, 2)];
        let consumed = consume_packet(&[], &debt, 1, Channels::first(1), 8, &zeros(1, 3));
        assert_eq!(consumed, vec![(1, 8), (2, 9)]);
    }

    #[test]
    fn u12b_value_not_consumed_before_acceptance() {
        let debt = [with_accepted(value(0, 8, 1), 10)];
        let consumed = consume_packet(&[], &debt, 1, Channels::first(1), 8, &zeros(1, 3));
        assert_eq!(consumed, vec![(1, 10)]);
    }

    #[test]
    fn u12c_unconsumed_left_entry_holds_later_debt() {
        let left = [value(0, 100, 9)];
        let debt = [value(0, 20, 1)];
        let consumed = consume_packet(&left, &debt, 1, Channels::first(1), 20, &zeros(1, 4));
        assert!(consumed.is_empty());
    }

    /// The reference: the same state consumed by one `RxChain::consume` over the whole packet.
    fn reference(left: &[RxEntry], debt: &[RxEntry], channels: Channels, first_tick: i64, samples: &[Vec<Iq>]) -> Vec<u64> {
        let segment = SegmentKey { by: None, origin: i64::MIN };
        let mut chain = RxChain::default();
        chain.adopt_start(segment, i64::MIN);
        for entry in left {
            hand(&mut chain, entry);
        }
        chain.stop_result(Ok(())).expect("the reference chain stops");
        chain.finish();
        chain.adopt_start(segment, i64::MIN);
        for entry in debt {
            hand(&mut chain, entry);
        }
        let before = orders(&chain);
        chain.consume(1, channels, first_tick, samples);
        let after = orders(&chain);
        let mut removed: Vec<u64> = before.into_iter().filter(|order| !after.contains(order)).map(|order| order.1).collect();
        removed.sort_unstable();
        removed
    }

    #[test]
    fn u12d_scratch_adapter_matches_rxchain_consume() {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: u64| {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        for _ in 0..200 {
            let channels = Channels::first(2);
            let mut id = 0;
            let mut entries = |count: u64, next: &mut dyn FnMut(u64) -> u64| -> Vec<RxEntry> {
                (0..count)
                    .map(|_| {
                        id += 1;
                        let accepted = (next(2) == 1).then(|| next(20) as i64);
                        let entry = value(next(2) as usize, next(20) as i64, id);
                        RxEntry { accepted_at: accepted, ..entry }
                    })
                    .collect()
            };
            let left = entries(next(3), &mut next);
            let debt = entries(next(3), &mut next);
            let first_tick = next(10) as i64;
            let count = 1 + next(6) as usize;
            let samples = zeros(2, count);
            let mut adapter: Vec<u64> = consume_packet(&left, &debt, 1, channels, first_tick, &samples).into_iter().map(|(id, _)| id).collect();
            adapter.sort_unstable();
            assert_eq!(adapter, reference(&left, &debt, channels, first_tick, &samples));
        }
    }

    #[test]
    fn u13a_loop_check_stops_before_call() {
        let mut fifo: VecDeque<u32> = VecDeque::from([1, 2]);
        assert_eq!(loop_check(true, &mut fifo), Check::Exit);
        assert!(fifo.is_empty());
        let mut kept: VecDeque<u32> = VecDeque::from([1]);
        assert_eq!(loop_check(false, &mut kept), Check::Proceed);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn u13c_settings_discard_writes_abandoned_per_command() {
        let mut fifo = VecDeque::from([2, 3]);
        let mut terminals: Vec<(u64, Terminal)> = Vec::new();
        let check = settings_check(true, Some(1), &mut fifo, |id| terminals.push((id, Terminal::Abandoned)));
        assert_eq!(check, Check::Exit);
        assert_eq!(terminals, vec![(1, Terminal::Abandoned), (2, Terminal::Abandoned), (3, Terminal::Abandoned)]);
        assert!(fifo.is_empty());
    }
}
