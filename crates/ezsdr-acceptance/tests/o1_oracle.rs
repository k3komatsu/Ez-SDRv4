//! The O1 oracle's own checks (design note 49 §5.4.11): hand-computed numbers taken from the spec text, not from any Provider's code. They
//! guard the oracle against its own typos; whether it reads the spec correctly is the reviewer's check of its rule table.

mod oracle;

use oracle::*;

/// The UHD rig's clock and profile: 1 MS/s on a 200 MHz root (200 ticks a sample), window 10 ms, device lead 2 ms, allowance 3 ms.
fn uhd() -> Params {
    Params { clock: Clock { origin: 0, n: 200 }, root_hz: 200_000_000, window: 2_000_000, lead: 400_000, allowance: 600_000 }
}

fn ms(n: i64) -> i64 {
    n * 200_000
}

#[test]
fn o1_oracle_rm_15_fronts() {
    // RM-15: 1 to 3 samples pad to four, a closed head of 1 to 32 fronts at its padded end plus the gap (15 at 1 MS/s), longer at its end plus four.
    assert_eq!(uhd().gap(), 15);
    assert_eq!(front_offset(1, false, 15), 19);
    assert_eq!(front_offset(3, false, 15), 19);
    assert_eq!(front_offset(4, false, 15), 19);
    assert_eq!(front_offset(20, false, 15), 35);
    assert_eq!(front_offset(32, false, 15), 47);
    assert_eq!(front_offset(33, false, 15), 37);
    assert_eq!(front_offset(3_000, false, 15), 3_004);
    assert_eq!(front_offset(10, true, 15), 14, "a joined piece fronts at its end plus four");
    assert_eq!(region(&uhd(), 100, Some(1)).front, 119);
    assert_eq!(region(&uhd(), 100, None).front, i64::MAX);
    // The gap follows the rate: 2 MS/s 30, 200 MS/s 3000, 3 MS/s on a 1 GHz root (ticks 1000/3 is n = 333.3: not integral here) is the Provider tests'.
    assert_eq!(Params { clock: Clock { origin: 0, n: 100 }, ..uhd() }.gap(), 30);
    assert_eq!(Params { clock: Clock { origin: 0, n: 1 }, ..uhd() }.gap(), 3_000);
    assert_eq!(Params { clock: Clock { origin: 0, n: 10_000 }, root_hz: 1_000_000_000, ..uhd() }.gap(), 3, "100 kS/s: ceil(1.5) = 2, floored to three");
}

#[test]
fn o1_oracle_rm_16_stop_bound_and_region() {
    let p = uhd();
    // A Stop at 5 ms: t_k = 5000, B = the first sample at or after 5 ms + 10 ms, plus five.
    assert_eq!(p.stop_bound(ms(5)), (5_000, 15_005));
    assert_eq!(p.stop_bound(ms(5) + 1), (5_001, 15_006), "an instant inside a sample rounds up");
    // A cancelled booking starting in [t_k, B) leaves the region; one starting at B leaves none.
    assert_eq!(stop_region(&p, ms(5), &[9_000]), Some(Region { start: 5_000, end: 15_005, front: 15_005, len: 10_005 }));
    assert_eq!(stop_region(&p, ms(5), &[15_005]), None);
    assert_eq!(stop_region(&p, ms(5), &[4_999]), None, "a booking before the Stop is kept, not cancelled");
}

#[test]
fn o1_oracle_rm_15_join_is_inclusive_at_its_deadline() {
    let p = uhd();
    let a = region(&p, 20_000, Some(3_000));
    let e = 23_000;
    assert_eq!(p.join_by(e), ms(23) - ms(5), "the deadline is the start less the lead and the allowance");
    for (recv, want) in [(p.join_by(e) - 1, Verdict::Accept), (p.join_by(e), Verdict::Accept), (p.join_by(e) + 1, Verdict::Refuse)] {
        assert_eq!(admit(&p, &[a], recv, e, Some(500)), want, "recv {recv}");
    }
    // At the front (end + 4) a booking is accepted, one sample before it refused.
    assert_eq!(admit(&p, &[a], 0, 23_004, Some(500)), Verdict::Accept);
    assert_eq!(admit(&p, &[a], 0, 23_003, Some(500)), Verdict::Refuse);
}

#[test]
fn o1_oracle_rm_14_placement() {
    let p = uhd();
    let a = region(&p, 10_000, Some(3_000));
    // Received at 10 ms: ordinary = 10 000 + 2 000 = 12 000, inside A's region, so past its front 13 004.
    assert_eq!(p.ordinary(ms(10)), 12_000);
    assert_eq!(place(&p, &[a], ms(10), Some(11_500), Some(200)), 13_004);
    // A late moved booking whose lead dominates: received at 12 ms, A (300 samples, front 10 304) long done.
    let short = region(&p, 10_000, Some(300));
    assert_eq!(place(&p, &[short], ms(12), Some(10_100), Some(200)), 14_000);
    // A receipt inside a sample decides at the next sample.
    assert_eq!(p.ordinary(ms(10) + 1), 12_001);
    assert!(p.is_late(ms(10), 11_999) && !p.is_late(ms(10), 12_000));
}

#[test]
fn o1_oracle_rm_37_switch_boundary() {
    let p = uhd();
    // Origin 2000, period 1000, received at 3 ms: bound = 3 000 + 10 000 + 2 000 = 15 000, on the grid.
    assert_eq!(switch_boundary(&p, 2_000, 1_000, ms(3), None), Some(15_000));
    // A period that divides nothing: the grid is 2000 + 700 j, so 15 300 (j = 19) and 16 000 (j = 20); a bound of 15 995 takes 16 000.
    assert_eq!(2_000 + 700 * 19, 15_300);
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4) - 1_000, None), Some(16_000));
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4), None), Some(16_000), "bound 16 000 is on the grid");
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4) + 200, None), Some(16_700), "bound 16 001 moves to the next grid point");
    // An explicit `at` later than the receipt's bound takes the grid point at or after it.
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4), Some(16_100)), Some(16_700));
}

#[test]
fn o1_oracle_rm_37_a_switch_boundary_that_does_not_fit_is_none_never_a_wrapped_one() {
    // RM-37 (#94), from the rule: the boundary is origin + j * period at or after the bound, and an i64 that does not hold it is no boundary.
    let p = uhd();
    // Origin 0, period 10: the last grid point that fits is 9_223_372_036_854_775_800, its own boundary; a bound past it has none.
    assert_eq!(switch_boundary(&p, 0, 10, ms(3), Some(9_223_372_036_854_775_800)), Some(9_223_372_036_854_775_800));
    assert_eq!(switch_boundary(&p, 0, 10, ms(3), Some(9_223_372_036_854_775_801)), None);
    assert_eq!(switch_boundary(&p, 0, 10, ms(3), Some(i64::MAX)), None);
    // A period of one has a boundary at every sample, i64::MAX included.
    assert_eq!(switch_boundary(&p, 0, 1, ms(3), Some(i64::MAX)), Some(i64::MAX));
    // A bound far below a negative origin takes the receipt's bound (15 000): -5 + 10 j, j = 1 501.
    assert_eq!(switch_boundary(&p, -5, 10, ms(3), Some(i64::MIN)), Some(15_005));
    // The span from the origin to the bound passes i64::MAX and does not wrap.
    assert_eq!(switch_boundary(&p, i64::MIN, 10, ms(3), Some(i64::MAX)), None);
    // A period that is not positive has no grid.
    assert_eq!((switch_boundary(&p, 0, 0, ms(3), None), switch_boundary(&p, 0, -1, ms(3), None)), (None, None));
    // A switch with no boundary carries no requested target.
    assert_eq!(switch_requested_target(&p, 0, 10, ms(3), i64::MAX, false), None);
}

#[test]
fn o1_oracle_mr_25_cut() {
    let p = Params { clock: Clock { origin: 0, n: 1_000 }, root_hz: 1_000_000_000, window: 10_000_000, lead: 2_000_000, allowance: 0 };
    let t = 5_000_000;
    let we = 15_000; // at_or_after(t + W)
    let burst = |start, len, started| MBurst { start, len, started, before_stop: true };
    assert_eq!(mock_cut(&p, t, &[burst(5_000, None, true)], None), we, "an open repeat is handed to the window's first sample");
    assert_eq!(mock_cut(&p, t, &[burst(we - 32, None, false)], None), we, "a repeat whose first 32 samples fit");
    assert_eq!(mock_cut(&p, t, &[burst(we - 31, None, false)], None), 5_000, "one that does not fit is not handed");
    assert_eq!(mock_cut(&p, t, &[burst(we - 20, Some(20), false)], None), we, "a finite burst of 20 whose whole fits");
    assert_eq!(mock_cut(&p, t, &[burst(we - 19, Some(20), false)], None), 5_000);
    assert_eq!(mock_cut(&p, t, &[burst(6_000, Some(100), false)], None), 6_100, "a finite burst contributes its end");
    assert_eq!(mock_cut(&p, t, &[MBurst { start: 5_500, len: None, started: true, before_stop: false }], None), 5_000, "received after the Stop: not counted");
    assert_eq!(mock_cut(&p, t, &[], Some(12_000)), 12_000, "a retained earlier cut is never retracted");
    let ideal = Params { window: 0, ..p };
    assert_eq!(mock_cut(&ideal, t, &[burst(5_000, Some(100), true)], None), 5_000, "a zero window cuts at the Stop sample");
}

#[test]
fn o1_oracle_slots_name_a_shifted_sample_and_a_wrong_phase() {
    let slots = Slots { segs: vec![Seg::repeating(7, 100, 10, 140)], cut: None };
    let good = |k: i64| slots.expected(k);
    assert_eq!(slots.check(90, 150, &good), Ok(None));
    let shifted = |k: i64| slots.expected(k + 1);
    let error = slots.check(90, 150, &shifted).unwrap_err();
    assert!(error.contains("k=99") || error.contains("k=100"), "{error}");
    let drift = |k: i64| if k >= 110 { slots.expected(k - 1) } else { slots.expected(k) };
    assert!(slots.check(90, 150, &drift).unwrap_err().contains("k=110"));
    // A cut in [lo, hi]: silent from 125 is inside [120, 130]; from 131 it is outside.
    let cut = Slots { segs: vec![Seg::repeating(7, 100, 10, i64::MAX).cuttable()], cut: Some((120, 130)) };
    let at = |c: i64| move |k: i64| if k >= c { None } else { cut_expected(k) };
    fn cut_expected(k: i64) -> Option<Iq> {
        (k >= 100).then(|| sample(7, ((k - 100) % 10) as usize))
    }
    assert_eq!(cut.check(100, 200, &at(125)), Ok(Some(125)));
    assert!(cut.check(100, 200, &at(119)).is_err(), "a cut below lo is not accepted");
    assert!(cut.check(100, 200, &at(131)).is_err(), "samples at or after hi must be silent");
    assert_eq!(cut.check(100, 200, &at(130)), Ok(Some(130)), "silence beginning exactly at hi is the cut hi");
    assert_eq!(cut.check(100, 200, &at(120)), Ok(Some(120)), "and exactly at lo is the cut lo");
}

#[test]
fn o1_oracle_ramp_is_distinct_and_never_zero() {
    let a = ramp(1, 3_000);
    let b = ramp(2, 3_000);
    assert!(a.iter().all(|s| *s != [0.0, 0.0]) && a.iter().zip(&b).all(|(x, y)| x != y));
    let mut sorted: Vec<u32> = a.iter().map(|s| s[0].to_bits()).collect();
    sorted.dedup();
    assert_eq!(sorted.len(), 3_000);
    assert!(a.iter().all(|s| s[0] < 1.0));
}

#[test]
fn o1_oracle_rm_37_outcomes() {
    assert!(switch_refused_unstarted(), "#84: a switch on a repeat not started on the device is refused at receipt");
}

#[test]
fn o1_oracle_c2_a_switch_applies_at_device_time_only_when_its_boundary_send_was_taken() {
    let b = 15_000;
    assert_eq!(switch_outcome_at_device_time(b, b, Sent::Whole), Outcome::Applied, "S1: device time reaches b");
    assert_eq!(switch_outcome_at_device_time(b, b - 1, Sent::Whole), Outcome::Pending, "one sample short of b");
    assert_eq!(switch_outcome_at_device_time(b, b + 5_000, Sent::Short(1)), Outcome::Applied, "S8: one sample is enough");
    assert_eq!(switch_outcome_at_device_time(b, b + 5_000, Sent::Short(0)), Outcome::Pending, "S8: none taken");
    assert_eq!(switch_outcome_at_device_time(b, b + 5_000, Sent::NotIssued), Outcome::Pending, "S11: a late host has not sent b");
    assert_eq!(switch_outcome_at_device_time(b, b + 5_000, Sent::Failed), Outcome::Pending, "S9: an exception settles at the abandon");
}

#[test]
fn o1_oracle_c2_a_stop_settles_a_switch_by_its_bound_and_its_boundary_send() {
    let (b, after, before) = (15_000, 15_003, 14_003);
    for sent in [Sent::Whole, Sent::Short(1), Sent::Short(0), Sent::Failed, Sent::NotIssued] {
        assert_eq!(switch_outcome_after_stop(b, before, sent), Outcome::Cancelled, "S2: B before b, {sent:?}");
        assert_eq!(switch_outcome_after_stop(b, b, sent), Outcome::Cancelled, "S2: B at b, {sent:?}");
    }
    assert_eq!(switch_outcome_after_stop(b, after, Sent::Whole), Outcome::Applied, "S3: sent before the Stop (H1)");
    assert_eq!(switch_outcome_after_stop(b, after, Sent::NotIssued), Outcome::Cancelled, "S4 (KL-42): never sent");
    assert_eq!(switch_outcome_after_stop(b, after, Sent::Short(1)), Outcome::Applied, "S5/S8: one sample taken");
    assert_eq!(switch_outcome_after_stop(b, after, Sent::Short(0)), Outcome::Cancelled, "S5/S8: none taken");
    assert_eq!(switch_outcome_after_stop(b, after, Sent::Failed), Outcome::Uncertain, "S5/S9: an exception");
}

#[test]
fn o1_oracle_c2_an_abandon_settles_a_switch_by_the_cursor_and_its_boundary_send() {
    let b = 14_000;
    for sent in [Sent::Whole, Sent::Failed, Sent::Short(1), Sent::NotIssued] {
        assert_eq!(switch_outcome_after_abandon(b, 12_000, sent), Outcome::Cancelled, "S6: the cursor before b, {sent:?}");
        assert_eq!(switch_outcome_after_abandon(b, b, sent), Outcome::Cancelled, "S7: a cursor at b has not handed the sample at b, {sent:?}");
    }
    assert_eq!(switch_outcome_after_abandon(b, 16_500, Sent::Whole), Outcome::Applied, "S10: an underflow after a whole boundary send");
    assert_eq!(switch_outcome_after_abandon(b, 16_500, Sent::Failed), Outcome::Uncertain, "S9");
    assert_eq!(switch_outcome_after_abandon(b, 16_500, Sent::Short(0)), Outcome::Cancelled, "S8");
    // An abandon halts at device time: a loss at a sample at or before b after it cancels a whole boundary send, one past b leaves it applied.
    assert_eq!(switch_outcome_after_abandon_and_loss(b, 16_500, 9_000, Sent::Whole), Outcome::Cancelled, "a loss before b");
    assert_eq!(switch_outcome_after_abandon_and_loss(b, 16_500, b, Sent::Whole), Outcome::Cancelled, "a loss at b");
    assert_eq!(switch_outcome_after_abandon_and_loss(b, 16_500, b + 1, Sent::Whole), Outcome::Applied, "a loss past b");
    assert_eq!(switch_outcome_after_abandon_and_loss(b, 16_500, 9_000, Sent::Failed), Outcome::Uncertain, "the exception wins");
    assert_eq!(switch_outcome_after_abandon_and_loss(b, b, b + 1, Sent::Whole), Outcome::Cancelled, "a cursor at b");
}

#[test]
fn o1_oracle_c2_a_loss_settles_a_switch_by_device_time_at_the_loss() {
    let (b, t) = (15_000, 15_000);
    // The loss's own bound is past b (the window is longer than the distance to b), except where the case says otherwise.
    let bound = b + 60;
    assert_eq!(switch_outcome_after_loss(b, bound, 12_000, Sent::Whole), Outcome::Cancelled, "S13: lost before b, whatever was sent");
    assert_eq!(switch_outcome_after_loss(b, bound, t, Sent::Whole), Outcome::Cancelled, "the sample at b is not before the loss");
    assert_eq!(switch_outcome_after_loss(b, bound, t + 1, Sent::Whole), Outcome::Applied, "b was played");
    assert_eq!(switch_outcome_after_loss(b, bound, t + 1, Sent::Short(1)), Outcome::Applied);
    assert_eq!(switch_outcome_after_loss(b, bound, t + 1, Sent::Short(0)), Outcome::Cancelled);
    assert_eq!(switch_outcome_after_loss(b, bound, t + 1, Sent::NotIssued), Outcome::Cancelled, "never sent, so never played");
    assert_eq!(switch_outcome_after_loss(b, bound, 12_000, Sent::Failed), Outcome::Uncertain, "Q1: a failed boundary send is uncertain at any loss time");
    assert_eq!(switch_outcome_after_loss(b, bound, t + 1, Sent::Failed), Outcome::Uncertain);
    assert_eq!(switch_outcome_after_loss(b, 12_060, 12_000, Sent::Failed), Outcome::Cancelled, "a loss whose bound is before b cancels at once, before the exception matters");
}

#[test]
fn o1_oracle_c2_a_stop_and_a_loss_together_settle_a_switch_in_either_order() {
    let (b, stop) = (15_000, 15_003);
    // A loss before b cancels what the Stop would have applied, whichever came first; B at or before b still cancels; a failure is uncertain.
    assert_eq!(switch_outcome_after_loss(b, stop, 12_000, Sent::Whole), Outcome::Cancelled);
    assert_eq!(switch_outcome_after_stop(b, stop, Sent::Whole), Outcome::Applied, "the Stop alone: applied, until a loss at or before b follows");
    assert_eq!(switch_outcome_after_loss(b, stop, 15_002, Sent::Whole), Outcome::Applied, "the loss after b leaves the Stop's answer");
    assert_eq!(switch_outcome_after_loss(b, stop, 15_002, Sent::Short(1)), Outcome::Applied);
    assert_eq!(switch_outcome_after_loss(b, stop, 12_000, Sent::Short(0)), Outcome::Cancelled);
    assert_eq!(switch_outcome_after_loss(b, stop, 12_000, Sent::Failed), Outcome::Uncertain);
    assert_eq!(switch_outcome_after_loss(b, b, 20_000, Sent::Whole), Outcome::Cancelled, "B at b");
    assert_eq!(switch_outcome_after_loss(b, b, 20_000, Sent::Failed), Outcome::Cancelled, "B at b, before the exception matters");
}

#[test]
fn o1_oracle_c2_an_unresolved_boundary_send_is_uncertain_when_the_manifest_is_written() {
    let b = 15_000;
    assert_eq!(switch_outcome_at_close(b, None), Outcome::Uncertain, "no halt: the owner left with the send in flight");
    assert_eq!(switch_outcome_at_close(b, Some(b + 3)), Outcome::Uncertain, "a halt past b waits for a result that never comes");
    assert_eq!(switch_outcome_at_close(b, Some(b)), Outcome::Cancelled, "B at b cancels first");
    assert_eq!(switch_outcome_at_close(b, Some(b - 1)), Outcome::Cancelled);
    assert_eq!(switch_outcome_at_close(b, Some(b + 3)), switch_outcome_after_stop(b, b + 3, Sent::Failed), "the same as a send that ended in an exception");
}

#[test]
fn o1_oracle_c2_the_mock_settles_a_switch_by_the_payload_cut() {
    assert_eq!(switch_outcome_after_mock_stop(15_000, 15_001), Outcome::Applied, "b below the cut");
    assert_eq!(switch_outcome_after_mock_stop(15_000, 15_000), Outcome::Cancelled, "the cut at b emits nothing of it");
    assert_eq!(switch_outcome_after_mock_stop(15_000, 12_000), Outcome::Cancelled);
}

#[test]
fn o1_oracle_c2_outcome_labels() {
    let labels = [Outcome::Applied, Outcome::Cancelled, Outcome::Uncertain, Outcome::Pending].map(Outcome::as_str);
    assert_eq!(labels, ["applied", "cancelled", "uncertain", "pending"]);
}

#[test]
fn o1_oracle_an_unhanded_booking_at_a_stop_is_cancelled_with_one_note() {
    assert_eq!(unhanded_booking_at_a_stop(), Cancelled { notes: 1, records: 0, samples: false });
}

#[test]
fn o1_oracle_a_pushed_timed_switch_records_its_at_and_an_untimed_one_none() {
    // #86: received at 3 ms, the bound is 15 000. An `at` of 3 000 alone gives grid point 3 000, so the receipt pushed it: `at` is recorded.
    // An `at` of 16 000 is past the bound and on its own grid point: none. An untimed switch records none whatever its instant.
    let p = uhd();
    assert_eq!(switch_requested_target(&p, 2_000, 1_000, ms(3), 3_000, false), Some(3_000));
    assert_eq!(switch_requested_target(&p, 2_000, 1_000, ms(3), 16_000, false), None);
    assert_eq!(switch_requested_target(&p, 2_000, 1_000, ms(3), 15_500, false), None, "between grid points past the bound: on its own grid point");
    assert_eq!(switch_requested_target(&p, 2_000, 1_000, ms(3), 3_000, true), None);
}
