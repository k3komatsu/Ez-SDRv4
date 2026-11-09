//! The O1 oracle's own checks (design note 49 §5.4.11): hand-computed numbers taken from the spec text, not from any Provider's code. They
//! guard the oracle against its own typos; whether it reads the spec correctly is the reviewer's check of its rule table.

mod oracle;

use oracle::*;

/// The UHD rig's clock and profile: 1 MS/s on a 200 MHz root (200 ticks a sample), window 10 ms, device lead 2 ms, allowance 3 ms.
fn uhd() -> Params {
    Params { clock: Clock { origin: 0, n: 200 }, window: 2_000_000, lead: 400_000, allowance: 600_000 }
}

fn ms(n: i64) -> i64 {
    n * 200_000
}

#[test]
fn o1_oracle_rm_15_fronts() {
    // RM-15: 1 to 3 samples pad to four, a closed head of 1 to 32 fronts at its padded end plus ten, longer at its end plus two.
    assert_eq!(front_offset(1, false), 14);
    assert_eq!(front_offset(3, false), 14);
    assert_eq!(front_offset(4, false), 14);
    assert_eq!(front_offset(20, false), 30);
    assert_eq!(front_offset(32, false), 42);
    assert_eq!(front_offset(33, false), 35);
    assert_eq!(front_offset(3_000, false), 3_002);
    assert_eq!(front_offset(10, true), 12, "a joined piece fronts at its end plus two");
    assert_eq!(region(100, Some(1)).front, 114);
    assert_eq!(region(100, None).front, i64::MAX);
}

#[test]
fn o1_oracle_rm_16_stop_bound_and_region() {
    let p = uhd();
    // A Stop at 5 ms: t_k = 5000, B = the first sample at or after 5 ms + 10 ms, plus three.
    assert_eq!(p.stop_bound(ms(5)), (5_000, 15_003));
    assert_eq!(p.stop_bound(ms(5) + 1), (5_001, 15_004), "an instant inside a sample rounds up");
    // A cancelled booking starting in [t_k, B) leaves the region; one starting at B leaves none.
    assert_eq!(stop_region(&p, ms(5), &[9_000]), Some(Region { start: 5_000, end: 15_003, front: 15_003, len: 10_003 }));
    assert_eq!(stop_region(&p, ms(5), &[15_003]), None);
    assert_eq!(stop_region(&p, ms(5), &[4_999]), None, "a booking before the Stop is kept, not cancelled");
}

#[test]
fn o1_oracle_rm_15_join_is_inclusive_at_its_deadline() {
    let p = uhd();
    let a = region(20_000, Some(3_000));
    let e = 23_000;
    assert_eq!(p.join_by(e), ms(23) - ms(5), "the deadline is the start less the lead and the allowance");
    for (recv, want) in [(p.join_by(e) - 1, Verdict::Accept), (p.join_by(e), Verdict::Accept), (p.join_by(e) + 1, Verdict::Refuse)] {
        assert_eq!(admit(&p, &[a], recv, e, Some(500)), want, "recv {recv}");
    }
    // At the front (end + 2) a booking is accepted, one sample before it refused.
    assert_eq!(admit(&p, &[a], 0, 23_002, Some(500)), Verdict::Accept);
    assert_eq!(admit(&p, &[a], 0, 23_001, Some(500)), Verdict::Refuse);
}

#[test]
fn o1_oracle_rm_14_placement() {
    let p = uhd();
    let a = region(10_000, Some(3_000));
    // Received at 10 ms: ordinary = 10 000 + 2 000 = 12 000, inside A's region, so past its front 13 002.
    assert_eq!(p.ordinary(ms(10)), 12_000);
    assert_eq!(place(&p, &[a], ms(10), Some(11_500), Some(200)), 13_002);
    // A late moved booking whose lead dominates: received at 12 ms, A (300 samples, front 10 302) long done.
    let short = region(10_000, Some(300));
    assert_eq!(place(&p, &[short], ms(12), Some(10_100), Some(200)), 14_000);
    // A receipt inside a sample decides at the next sample.
    assert_eq!(p.ordinary(ms(10) + 1), 12_001);
    assert!(p.is_late(ms(10), 11_999) && !p.is_late(ms(10), 12_000));
}

#[test]
fn o1_oracle_rm_37_switch_boundary() {
    let p = uhd();
    // Origin 2000, period 1000, received at 3 ms: bound = 3 000 + 10 000 + 2 000 = 15 000, on the grid.
    assert_eq!(switch_boundary(&p, 2_000, 1_000, ms(3), None), 15_000);
    // A period that divides nothing: the grid is 2000 + 700 j, so 15 300 (j = 19) and 16 000 (j = 20); a bound of 15 995 takes 16 000.
    assert_eq!(2_000 + 700 * 19, 15_300);
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4) - 1_000, None), 16_000);
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4), None), 16_000, "bound 16 000 is on the grid");
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4) + 200, None), 16_700, "bound 16 001 moves to the next grid point");
    // An explicit `at` later than the receipt's bound takes the grid point at or after it.
    assert_eq!(switch_boundary(&p, 2_000, 700, ms(4), Some(16_100)), 16_700);
    // UR-38: the first grid point at or after the cursor.
    assert_eq!(moved_boundary(2_000, 1_000, 17_000), 17_000);
    assert_eq!(moved_boundary(2_000, 1_000, 17_001), 18_000);
}

#[test]
fn o1_oracle_mr_25_cut() {
    let p = Params { clock: Clock { origin: 0, n: 1_000 }, window: 10_000_000, lead: 2_000_000, allowance: 0 };
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
    assert_eq!(outcome_after_stop(15_000, 14_003), Outcome::Cancelled, "B before b");
    assert_eq!(outcome_after_stop(15_000, 15_000), Outcome::Cancelled, "B at b: `at or before`");
    assert_eq!(outcome_after_stop(15_000, 15_001), Outcome::Applied);
    assert_eq!(outcome_after_abandon(14_000, 12_000), Outcome::Cancelled);
    assert_eq!(outcome_after_abandon(14_000, 14_000), Outcome::Cancelled, "a cursor at b has not handed the sample at b");
    assert_eq!(outcome_after_abandon(14_000, 16_500), Outcome::Applied);
    assert_eq!(outcome_after_repeat_dropped().as_str(), "cancelled");
}

#[test]
fn o1_oracle_an_unhanded_booking_at_a_stop_is_cancelled_with_one_note() {
    assert_eq!(unhanded_booking_at_a_stop(), Cancelled { notes: 1, records: 0, samples: false });
}

#[test]
fn o1_oracle_a_moved_switch_records_its_asked_instant_as_its_requested_target() {
    assert_eq!(requested_target_of_a_moved_switch(3_000), Some(3_000));
}
