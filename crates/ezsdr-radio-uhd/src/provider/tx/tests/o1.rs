//! O1a (design note 49 §5.4.11, plan `2026-11-09-o1/test-plan.md`): the spec oracle against the UHD owner on a frozen single-threaded
//! rig. Every receipt is "at the tick the test sets", never "when the test thread runs": the device clock is manual and moves only
//! through `at`. The expected values come from `oracle` (written from the spec text, no `ezsdr_radio::staging`); the observations are
//! `FakeDevice::transmitted` (channel 0), `calls()`, the `bursts`, `switches` and `rejected` rows and the events. All streams are
//! one channel; the multi-channel limit is not covered.
//!
//! Units: one sample is 1 us (the rig's 1 MS/s), 200 root ticks; `MS` is a millisecond in root ticks.

use super::*;
use crate::provider::tx::oracle as o;

const MS: i64 = 200_000;

/// The UHD profile in the oracle's terms: the window 10 ms, the device lead 2 ms, the delivery allowance 3 ms (profile values:
/// `x3xx::timing`, `DEVICE_LEAD_NS`, `DELIVERY_ALLOWANCE_NS`), on the rig's clock (origin 0, 200 ticks a sample).
fn params() -> o::Params {
    o::Params { clock: o::Clock { origin: 0, n: 200 }, window: 10 * MS, lead: 2 * MS, allowance: 3 * MS }
}

struct Rig {
    rig: SwitchRig,
    p: o::Params,
    events: Vec<ezsdr_kernel::event::Event>,
}

/// One rig per case: waves are `ramp(tag, len)`, in the order given.
fn rig(waves: &[(u8, usize)], faults: Vec<FakeFault>) -> Rig {
    let rig = switch_rig_with(waves.iter().map(|(tag, len)| o::ramp(*tag, *len)).collect(), faults);
    Rig { rig, p: params(), events: Vec::new() }
}

type Row = serde_json::Value;

impl Rig {
    /// The device (and the Core) at root tick `ticks`.
    fn at(&mut self, ticks: i64) {
        device_at(&self.rig.core, &self.rig.time, &self.rig.device, ticks);
    }

    /// One owner pass as the run loop makes it: drain the commands, step until there is nothing to do.
    fn turn(&mut self) {
        for _ in 0..8 {
            drain_tx(&mut self.rig.tx);
            if !self.rig.tx.step() {
                break;
            }
        }
    }

    fn turns(&mut self, from_ms: i64, to_ms: i64) {
        for ms in from_ms..=to_ms {
            self.at(ms * MS);
            self.turn();
        }
    }

    fn book(&mut self, id: u64, wave: usize, k: i64, repeat: bool, policy: LatePolicy) {
        book_with(&mut self.rig.control, &self.rig.core, id, 0, self.rig.clock, k, &self.rig.waves[wave], repeat, policy, BTreeMap::new());
    }

    fn book_untimed(&mut self, id: u64, wave: usize, k: i64) {
        book_untimed(&mut self.rig.control, &self.rig.core, id, self.rig.clock, k, &self.rig.waves[wave], false, LatePolicy::SendAsapAndFlag);
    }

    /// The events since the last call.
    fn take(&mut self) -> Vec<ezsdr_kernel::event::Event> {
        let new = self.rig.events.drain(TimePoint::new(ezsdr_kernel::id::ClockDomainId::HOST_MONOTONIC, 0));
        self.events.extend(new.clone());
        new
    }

    /// The reasons of the `COMMAND_REJECTED` events since the last call: a refusal at receipt.
    fn refusals(&mut self) -> Vec<String> {
        self.take().iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|e| e.payload["reason"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// Every refusal so far.
    fn all_refusals(&mut self) -> usize {
        self.take();
        self.events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count()
    }

    /// Every `TIME_ERROR` so far as its outcome.
    fn time_errors(&mut self) -> Vec<String> {
        self.take();
        self.events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).map(|e| e.payload["outcome"].as_str().unwrap_or_default().to_owned()).collect()
    }

    fn rows(&self) -> Vec<Row> {
        lock(&self.rig.core.rec).bursts.clone()
    }

    /// The burst record with `samples` payload samples, as `(target, requested target, late)`.
    fn burst_of(&self, samples: u64) -> (i64, Option<i64>, bool) {
        let rows: Vec<Row> = self.rows().into_iter().filter(|row| row["samples"] == samples).collect();
        assert_eq!(rows.len(), 1, "one record of {samples} samples: {:?}", self.rows());
        let row = &rows[0];
        (row["target"]["ticks"].as_i64().unwrap(), row["requested_target"]["ticks"].as_i64(), !row["late_by"].is_null())
    }

    /// The Provider's device calls that carried payload or zeros with a time spec: `(first sample, n, eob)`. A failing `FailSend`
    /// is logged with the accepted half (`device.rs:1820` before `:1840`).
    fn sends(&self) -> Vec<(i64, i64, bool)> {
        self.rig
            .device
            .calls()
            .iter()
            .filter_map(|call| {
                let parts: Vec<&str> = call.split_whitespace().collect();
                if parts.first() != Some(&"tx_send") || parts.get(1).and_then(|s| s.parse::<usize>().ok()).is_none() {
                    return None;
                }
                let field = |name: &str| parts.iter().find_map(|p| p.strip_prefix(name)).map(str::to_owned);
                let at = field("at=").filter(|at| at != "-")?.parse::<i64>().ok()?;
                Some((self.p.clock.at_or_after(at), field("n=")?.parse().ok()?, field("eob=")? == "true"))
            })
            .collect()
    }

    /// The device's samples `[from, to)` on channel 0.
    fn observed(&self, from: i64, to: i64) -> Vec<Option<Iq>> {
        self.rig.device.transmitted(self.rig.clock.instant(from), (to - from) as usize)
    }

    /// Checks `slots` over `[from, to)`; returns the first silent sample from the cut range's `lo`, when there is one.
    fn check(&self, slots: &o::Slots, from: i64, to: i64) -> Option<i64> {
        let seen = self.observed(from, to);
        slots.check(from, to, &|k| seen[(k - from) as usize]).unwrap_or_else(|error| panic!("{error}: {:?}", self.rig.device.calls()))
    }

    /// Whether any sample of waveform `tag` sounded in `[from, to)`.
    fn heard(&self, tag: u8, from: i64, to: i64) -> bool {
        self.observed(from, to).iter().flatten().any(|s| (s[1] * 64.0).round() as u8 == tag && *s != [0.0, 0.0])
    }

    fn switch_row(&self, index: usize) -> Row {
        switch_row(&self.rig.core, index)
    }

    /// The `rejected` notes that say a booking was cancelled by a Stop.
    fn cancellations(&self) -> usize {
        lock(&self.rig.core.rec).rejected.iter().filter(|row| row["kind"] == "tx_burst" && row["reason"].as_str().is_some_and(|reason| reason.contains("cancelled by Stop"))).count()
    }

    fn stop(&mut self) {
        stop(&mut self.rig);
    }
}

fn region_of(start: i64, len: i64) -> o::Region {
    o::region(start, Some(len))
}

/// A Kernel burst record in full: `(target, samples, wraps, requested target, late_by in ns, end)`.
type Rec = (i64, i64, i64, Option<i64>, Option<i64>, String);

impl Rig {
    fn full_recs(&self) -> Vec<Rec> {
        self.rows()
            .iter()
            .map(|row| {
                (
                    row["target"]["ticks"].as_i64().unwrap(),
                    row["samples"].as_i64().unwrap(),
                    row["wraps"].as_i64().unwrap(),
                    row["requested_target"]["ticks"].as_i64(),
                    row["late_by"]["ticks"].as_i64(),
                    row["end"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// The switch row's identities: the switch's action id and the repeat it replaces (the rig books the repeat as 1 and the switch as 2).
    fn assert_switch_ids(&self, index: usize) {
        let row = self.switch_row(index);
        assert_eq!((row["action_id"].as_u64(), row["predecessor"].as_u64()), (Some(2), Some(1)), "the switch row's identities: {row}");
    }
}

fn ids(rows: &[(i64, u64)], got: &Rig) {
    let seen: Vec<(i64, u64)> = got.rows().iter().map(|row| (row["target"]["ticks"].as_i64().unwrap(), row["samples"].as_u64().unwrap())).collect();
    assert_eq!(seen, rows, "the Kernel burst records: {:?}", got.rows());
}

// ------------------------------------------------------------------------------------------------------------ T1

#[test]
fn o1_uhd_t1_short_and_long_heads_front_table() {
    // RM-15 (seq 1): a finite head of L samples fronts at its padded end plus ten (L <= 32, 1 to 3 padded to four) or its end plus two; a
    // booking at the front is accepted and plays exactly, one at front - 1 is refused. The 3 000-sample row spans two sends, which is
    // what a shifted waveform in the second send needs (mutation row O1-M1).
    let s = 10_000;
    for len in [1usize, 3, 4, 31, 32, 33, 3_000] {
        for accept in [true, false] {
            let mut r = rig(&[(1, len), (2, 40)], Vec::new());
            let a = region_of(s, len as i64);
            let b_start = if accept { a.front } else { a.front - 1 };
            r.book(1, 0, s, false, LatePolicy::DropAndFlag);
            r.book(2, 1, b_start, false, LatePolicy::DropAndFlag);
            r.turns(0, 30);
            let verdict = o::admit(&r.p, &[a], 0, b_start, Some(40));
            assert_eq!(verdict, if accept { o::Verdict::Accept } else { o::Verdict::Refuse }, "L={len} accept={accept}");
            let reasons = r.refusals();
            assert_eq!(reasons.len(), usize::from(!accept), "L={len} accept={accept}: {reasons:?}");
            assert!(reasons.iter().all(|reason| reason.contains("overlaps an accepted booking")), "{reasons:?}");
            let mut slots = o::Slots { segs: vec![o::Seg::once(1, s, len as i64)], cut: None };
            if accept {
                slots.segs.push(o::Seg::once(2, b_start, 40));
            }
            r.check(&slots, s - 3, b_start + 60);
            let want: Vec<(i64, u64)> = if accept { vec![(s, len as u64), (b_start, 40)] } else { vec![(s, len as u64)] };
            ids(&want, &r);
            assert!(r.time_errors().is_empty(), "L={len}: no TIME_ERROR");
            if len == 3_000 && accept {
                // The nonzero-offset precondition of O1-M1: a payload send that starts inside the burst.
                let sends = r.sends();
                assert!(sends.iter().filter(|(_, n, _)| *n > 2).count() >= 2 && sends.iter().any(|(k, _, _)| *k > s && *k < s + 3_000), "{sends:?}");
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------------ T2

#[test]
fn o1_uhd_t2_a_join_is_inclusive_at_its_deadline() {
    // RM-15 (seq 2): B at the exact end of a 3 000-sample head joins it when received no later than J(e) = start - (lead + allowance),
    // the instant itself included, and is refused one root tick after. A joined B is continuous with A (no silent sample) and a
    // separate Kernel record.
    let (s, e) = (20_000, 23_000);
    let a = region_of(s, 3_000);
    for (case, recv) in [("J - 1 tick", params().join_by(e) - 1), ("J", params().join_by(e)), ("J + 1 tick", params().join_by(e) + 1)] {
        let mut r = rig(&[(5, 3_000), (6, 500)], Vec::new());
        r.book(1, 0, s, false, LatePolicy::DropAndFlag);
        r.turns(0, 17);
        r.at(recv);
        r.book(2, 1, e, false, LatePolicy::DropAndFlag);
        r.turn();
        let verdict = o::admit(&r.p, &[a], recv, e, Some(500));
        let accepted = verdict == o::Verdict::Accept;
        assert_eq!(accepted, recv <= params().join_by(e), "{case}");
        r.turns(19, 40);
        assert_eq!(r.refusals().len(), usize::from(!accepted), "{case}: {:?}", r.events);
        let mut slots = o::Slots { segs: vec![o::Seg::once(5, s, 3_000)], cut: None };
        if accepted {
            slots.segs.push(o::Seg::once(6, e, 500));
        }
        r.check(&slots, s - 2, e + 520);
        let want: Vec<(i64, u64)> = if accepted { vec![(s, 3_000), (e, 500)] } else { vec![(s, 3_000)] };
        ids(&want, &r);
    }
}

#[test]
fn o1_uhd_t2c_a_booking_joined_at_a_heads_end_with_a_later_one_booked_early_plays_on_time() {
    // RM-15 (seq 2; the exact-timing property of the parity test `s15_parity_s4b_three_bookings_b_continues_a_and_c_plays_on_time`, S4b R4-5):
    // A (40 samples) at 20 000 is booked at 6 ms, C (10) at 100 samples past it at 11 ms, then B (10) joining A at its exact end 20 040 at 12 ms,
    // before the join deadline (20 040 - 5 ms = 15.04 ms). B continues A with no end-of-burst between, C plays on time: no refusal, no
    // TIME_ERROR, three records, two device bursts.
    let mut r = rig(&[(1, 40), (2, 10), (3, 10)], Vec::new());
    r.at(6 * MS);
    r.book(1, 0, 20_000, false, LatePolicy::DropAndFlag);
    r.turn();
    r.at(11 * MS);
    r.book(2, 2, 20_100, false, LatePolicy::DropAndFlag);
    r.turn();
    r.at(12 * MS);
    r.book(3, 1, 20_040, false, LatePolicy::DropAndFlag);
    r.turns(13, 30);
    assert!(r.refusals().is_empty() && r.time_errors().is_empty(), "{:?}", r.events);
    r.check(&o::Slots { segs: vec![o::Seg::once(1, 20_000, 40), o::Seg::once(2, 20_040, 10), o::Seg::once(3, 20_100, 10)], cut: None }, 19_990, 20_200);
    ids(&[(20_000, 40), (20_040, 10), (20_100, 10)], &r);
    let device_bursts = r.rig.device.calls().iter().filter(|call| call.starts_with("tx_send ") && call.contains("sob=true") && !call.contains("at=-")).count();
    assert_eq!(device_bursts, 2, "A and B are one device burst, C another: {:?}", r.rig.device.calls());
}

// ------------------------------------------------------------------------------------------------------------ T3

/// The payload samples the device took in sends that carry no end-of-burst (a repeat's payload, a Stop's two zeros excluded).
fn open_payload(r: &Rig) -> i64 {
    r.sends().iter().filter(|(_, _, eob)| !eob).map(|(_, n, _)| n).sum()
}

#[test]
fn o1_uhd_t3_a_repeat_keeps_its_phase_across_wraps_and_a_stop_cuts_it_in_its_range() {
    // RM-13, RM-16 (seq 3): a 1 000-sample repeat from 2 000 is `R[(k - 2000) mod 1000]` everywhere; a Stop at 4.5 ms cuts it at the
    // handed cursor, in [t_k, B]. The record holds what the device took (samples, wraps, end stop). (Mutation row O1-M2.)
    let s = 2_000;
    let mut r = rig(&[(7, 1_000)], Vec::new());
    r.book(1, 0, s, true, LatePolicy::DropAndFlag);
    r.turns(0, 4);
    r.at(4 * MS + MS / 2);
    r.turn();
    let t = 4 * MS + MS / 2;
    let (t_k, b) = r.p.stop_bound(t);
    r.stop();
    r.turns(5, 20);
    // The precondition of O1-M2: a send that starts after the first wrap, in the middle of a period.
    assert!(r.sends().iter().any(|(k, _, _)| *k >= s + 1_000 && (*k - s) % 1_000 != 0), "{:?}", r.sends());
    let slots = o::Slots { segs: vec![o::Seg::repeating(7, s, 1_000, i64::MAX).cuttable()], cut: Some((t_k, b)) };
    let cut = r.check(&slots, s - 10, b + 60).expect("a Stop cuts the repeat");
    assert!((t_k..=b).contains(&cut), "the cut {cut} is outside [{t_k}, {b}]");
    let rows = r.rows();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0]["samples"].as_i64(), rows[0]["wraps"].as_i64(), rows[0]["end"].as_str()), (Some(cut - s), Some((cut - s) / 1_000), Some("stop")));
    assert_eq!(open_payload(&r), cut - s, "the device took exactly what the record counts: {:?}", r.sends());
}

#[test]
fn o1_uhd_t3b_a_stop_cannot_find_payload_past_its_bound() {
    // RM-16 (seq 3, mutation row O1-M5): the owner hands the repeat in 2 500-sample chunks from 20 000. At the Stop the window's edge is
    // four samples short of the chunk boundary 25 000: `avail = window_cut - next` is 2 496, below the 2 500 the chunk needs, so the
    // chunk [22 500, 25 000) is not handed and the cut is 22 500, inside [t_k, B = 24 999]. A window four samples wider would hand it and
    // cut at B + 1.
    let s = 20_000;
    let mut r = rig(&[(7, 1_000)], Vec::new());
    r.book(1, 0, s, true, LatePolicy::DropAndFlag);
    r.at(125 * MS / 10);
    r.turn();
    // The chunk precondition, before the window-edge turn: the one handed chunk ends at s + 2 500.
    assert_eq!(r.sends().last().map(|(k, n, _)| k + n), Some(s + 2_500), "the first chunk is [s, s + 2 500): {:?}", r.sends());
    let edge = s + 5_000 - 4;
    let t = r.p.clock.instant(edge) - r.p.window;
    r.at(t);
    r.turn();
    let (t_k, b) = r.p.stop_bound(t);
    assert_eq!(b, s + 5_000 - 1, "B is the window edge plus three");
    r.stop();
    r.turns(15, 40);
    let slots = o::Slots { segs: vec![o::Seg::repeating(7, s, 1_000, i64::MAX).cuttable()], cut: Some((t_k, b)) };
    let cut = r.check(&slots, s - 10, b + 100).expect("cut");
    assert_eq!(cut, s + 2_500);
}

// ------------------------------------------------------------------------------------------------------------ T4

/// A repeat of waveform 0 (`period` long) from 2 000, run to 3 ms, with a switch to waveform 1 received at `recv` for `at`.
fn switched(period: usize, new_period: usize, recv: i64, at: i64) -> Rig {
    let mut r = rig(&[(8, period), (9, new_period)], Vec::new());
    r.book(1, 0, 2_000, true, LatePolicy::DropAndFlag);
    r.turns(0, 3);
    r.at(recv);
    book_switch(&mut r.rig, 2, at, 1);
    r.turn();
    r
}

#[test]
fn o1_uhd_t4a_a_switch_takes_the_first_grid_point_at_or_after_its_bound() {
    // RM-37 (seq 4): R has period 700 (a period that divides neither the window nor the lead, grid 2000 + 700 j). The switch is received
    // so that its bound (receipt's sample + window + lead) is one before a grid point, on it and one after it, and once with an explicit
    // `at` between grid points. The boundary is the oracle's, not the observed one; the new waveform has its own period from there.
    // (Mutation row O1-M3a.)
    let cases: [(&str, i64, Option<i64>); 4] = [("g - 1", 15_999, None), ("g", 16_000, None), ("g + 1", 16_001, None), ("at between grid points", 16_000, Some(16_100))];
    for (case, bound, at) in cases {
        let recv = if at.is_some() { 4 * MS } else { r_tick(bound - 12_000) };
        let mut r = switched(700, 400, recv, at.unwrap_or(bound - 12_000));
        let b = o::switch_boundary(&r.p, 2_000, 700, recv, at);
        assert_eq!(b, if bound == 15_999 || bound == 16_000 && at.is_none() { 16_000 } else { 16_700 }, "{case}: the grid is 2000 + 700 j");
        r.turns(5, 30);
        let row = r.switch_row(0);
        assert_eq!((row["outcome"].as_str(), row["boundary"].as_i64(), row["moved_to"].is_null()), (Some("applied"), Some(b), true), "{case}: {row}");
        assert!(r.refusals().is_empty() && r.time_errors().is_empty(), "{case}: {:?}", r.events);
        let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 700, b), o::Seg::repeating(9, b, 400, i64::MAX)], cut: None };
        r.check(&slots, 1_990, b + 1_200);
        assert_eq!(
            r.rows().first().map(|row| (row["target"]["ticks"].as_i64(), row["samples"].as_i64())),
            Some((Some(2_000), Some(b - 2_000))),
            "{case}: the old repeat's record ends at the boundary"
        );
        r.assert_switch_ids(0);
        // A Stop at 31 ms closes the new repeat: its record is the successor's, with its own target, count, wraps and end, not late, not moved.
        r.at(31 * MS);
        r.turn();
        let (t_k, bound) = r.p.stop_bound(31 * MS);
        r.stop();
        r.turns(32, 45);
        let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 700, b).cuttable(), o::Seg::repeating(9, b, 400, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
        let cut = r.check(&slots, 1_990, bound + 100).expect("the Stop cuts the new repeat");
        let recs = r.full_recs();
        assert_eq!(recs.len(), 2, "{case}: {recs:?}");
        assert_eq!(recs[1], (b, cut - b, (cut - b) / 400, None, None, "stop".to_owned()), "{case}: the successor's record");
    }
}

/// The root tick of sample `k` on the rig's clock.
fn r_tick(k: i64) -> i64 {
    params().clock.instant(k)
}

#[test]
fn o1_uhd_t4b_a_switch_taken_after_the_cursor_passed_its_boundary_moves_to_the_next_grid_point() {
    // RM-37, UR-38 (seq 4, UHD only; E2a deletes this path, note 49 §5.4.5): the switch is booked at 3 ms (boundary 15 000) and sits in the
    // owner's channel while the owner hands the repeat past 15 000 (no state is poked: `Tx::step` hands without draining). When the owner
    // takes it, its cursor is where the send trace says; the boundary is the first grid point at or after that cursor.
    let mut r = rig(&[(8, 1_000), (9, 1_000)], Vec::new());
    r.book(1, 0, 2_000, true, LatePolicy::DropAndFlag);
    r.turns(0, 3);
    book_switch(&mut r.rig, 2, 3_000, 1);
    for ms in 4..=7 {
        r.at(ms * MS);
        for _ in 0..8 {
            if !r.rig.tx.step() {
                break;
            }
        }
    }
    let cursor = r.sends().iter().map(|(k, n, _)| k + n).max().unwrap();
    assert!(cursor > 15_000, "the owner's cursor {cursor} has passed the boundary 15 000 without draining the switch");
    let to = o::moved_boundary(2_000, 1_000, cursor);
    drain_tx(&mut r.rig.tx);
    r.turns(8, 30);
    let row = r.switch_row(0);
    assert_eq!((row["outcome"].as_str(), row["moved_to"].as_i64()), (Some("applied"), Some(to)), "{row}");
    assert_eq!(r.time_errors(), ["send_asap"], "the move is reported once");
    let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 1_000, to), o::Seg::repeating(9, to, 1_000, i64::MAX)], cut: None };
    r.check(&slots, 1_990, to + 1_500);
    r.assert_switch_ids(0);
    // A Stop at 30 ms closes the moved successor. Its record reads the move like any late burst's (UR-38): the target is where it started,
    // and `late_by` the amount the cursor passed the boundary chosen at receipt (15 000).
    r.at(30 * MS);
    r.turn();
    let (t_k, bound) = r.p.stop_bound(30 * MS);
    r.stop();
    r.turns(31, 45);
    let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 1_000, to).cuttable(), o::Seg::repeating(9, to, 1_000, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
    let cut = r.check(&slots, 1_990, bound + 100).expect("the Stop cuts the new repeat");
    let recs = r.full_recs();
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_eq!(recs[0].1, to - 2_000, "the old repeat's record ends at the moved boundary");
    let b0 = o::switch_boundary(&r.p, 2_000, 1_000, 3 * MS, Some(3_000));
    assert_eq!(b0, 15_000);
    assert_eq!((recs[1].0, recs[1].1, recs[1].2, recs[1].4, recs[1].5.as_str()), (to, cut - to, (cut - to) / 1_000, Some((cursor - b0) * 1_000), "stop"), "the moved successor's record");
    // KNOWN DIVERGENCE #86: the spec records the switch's asked instant (`at` = 3 000) as the moved successor's requested target (RM-14, UR-38,
    // SC-23a); today's record leaves it None. The fix deletes only this block and asserts `recs[1].3 == spec`.
    let spec = o::requested_target_of_a_moved_switch(3_000);
    assert_eq!(recs[1].3, None, "#86 fixed? delete this block and assert the requested target equals the spec's");
    assert_ne!(recs[1].3, spec, "today's value differs from the oracle's");
}

#[test]
fn o1_uhd_t4c_a_stop_before_the_boundary_cancels_the_switch() {
    // RM-37 (seq 4): R from 2 000, a switch received at 3 ms (boundary 15 000), a Stop at 4 ms whose bound 14 003 is before it: the switch
    // is cancelled and no sample of the new waveform sounds.
    let mut r = switched(1_000, 1_000, 3 * MS, 3_000);
    let b = o::switch_boundary(&r.p, 2_000, 1_000, 3 * MS, Some(3_000));
    r.turns(4, 4);
    let (t_k, bound) = r.p.stop_bound(4 * MS);
    assert!(bound <= b, "the case needs B <= b");
    r.stop();
    r.turns(5, 30);
    assert_eq!(r.switch_row(0)["outcome"], o::outcome_after_stop(b, bound).as_str());
    r.assert_switch_ids(0);
    assert!(!r.heard(9, 2_000, 30_000));
    let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 1_000, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
    r.check(&slots, 1_990, 30_000);
    assert_eq!(r.rows().len(), 1);
}

/// A switch received at 3 ms (boundary 15 000), drained at once, and a Stop at `stop_ticks` after the owner's turns up to there.
fn stopped_after_switch(stop_ticks: i64) -> (Rig, i64, i64, i64) {
    let mut r = switched(1_000, 1_000, 3 * MS, 3_000);
    let b = o::switch_boundary(&r.p, 2_000, 1_000, 3 * MS, Some(3_000));
    assert_eq!(b, 15_000);
    r.turns(4, stop_ticks / MS);
    r.at(stop_ticks);
    r.turn();
    let (t_k, bound) = r.p.stop_bound(stop_ticks);
    assert!(b < bound, "the case needs b < B");
    r.stop();
    r.turns(stop_ticks / MS + 1, 30);
    (r, b, t_k, bound)
}

#[test]
fn o1_uhd_t4d_a_stop_with_b_before_its_bound_leaves_the_row_applied_with_no_new_sample_handed() {
    // RM-37 (seq 4): the owner drained the switch before the Stop (otherwise today's UHD answers `cancelled`, note 49 §5.4.5). The Stop at
    // 5.6 ms has its window edge at b + 600, below the 2 500 samples the new waveform's first chunk needs, so none of it is handed.
    // C2 (note 49 §11.10) changes the ROW below; the emission assertion stays.
    let (r, b, t_k, bound) = stopped_after_switch(5 * MS + 3 * MS / 5);
    let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 1_000, b).cuttable(), o::Seg::repeating(9, b, 1_000, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
    let cut = r.check(&slots, 1_990, 30_000).expect("cut");
    assert!(cut <= b, "no sample of the new waveform was handed: cut {cut}, boundary {b}");
    assert!(!r.heard(9, 2_000, 30_000));
    // The row, pinned apart from the emission (C2 changes it):
    assert_eq!(r.switch_row(0)["outcome"], o::outcome_after_stop(b, bound).as_str(), "C2: this outcome changes with note 49 §11.10");
    assert_eq!(r.switch_row(0)["outcome"], "applied");
    r.assert_switch_ids(0);
    // Applied with no new payload handed leaves no successor record today (C2 changes the row, not this): only the predecessor's.
    assert_eq!(r.full_recs().len(), 1, "{:?}", r.full_recs());
}

#[test]
fn o1_uhd_t4e_a_stop_after_the_new_waveform_sounded_leaves_it_applied() {
    // RM-37 (seq 4): as T4d, but the Stop at 7.6 ms comes after the new waveform's first chunk [15 000, 17 500) was handed: its samples
    // sound up to the cut, and the row is applied.
    let (r, b, t_k, bound) = stopped_after_switch(7 * MS + 3 * MS / 5);
    let slots = o::Slots { segs: vec![o::Seg::repeating(8, 2_000, 1_000, b).cuttable(), o::Seg::repeating(9, b, 1_000, i64::MAX).cuttable()], cut: Some((t_k, bound)) };
    let cut = r.check(&slots, 1_990, 30_000).expect("cut");
    assert!(cut > b + 1_000, "the new waveform sounded: cut {cut}, boundary {b}");
    assert!(r.heard(9, b, b + 100));
    assert_eq!(r.switch_row(0)["outcome"], o::outcome_after_stop(b, bound).as_str(), "C2 (note 49 §11.10) may change the unhanded case, not this one");
    r.assert_switch_ids(0);
    // The successor's record: its own target, the samples the device played of it, wraps, end stop; not late, not moved.
    let recs = r.full_recs();
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_eq!(recs[0], (2_000, b - 2_000, (b - 2_000) / 1_000, None, None, "eob".to_owned()), "the predecessor's record");
    assert_eq!(recs[1], (b, cut - b, (cut - b) / 1_000, None, None, "stop".to_owned()), "the successor's record");
}

// ------------------------------------------------------------------------------------------------------------ T5

#[test]
fn o1_uhd_t5_a_late_send_asap_and_an_untimed_booking_are_placed_exactly() {
    // RM-14, RM-15 (seq 5): the placement is the oracle's, compared by equality with where the record says the booking started.
    let s = 10_000;
    // (a) late SendAsap inside A's region, whose front dominates the ordinary bound.
    let mut r = rig(&[(10, 3_000), (11, 200)], Vec::new());
    r.book(1, 0, s, false, LatePolicy::DropAndFlag);
    r.turns(0, 10);
    let a = region_of(s, 3_000);
    r.book(2, 1, 11_500, false, LatePolicy::SendAsapAndFlag);
    r.turn();
    assert_eq!(o::admit(&r.p, &[a], 10 * MS, 11_500, Some(200)), o::Verdict::Late);
    let want = o::place(&r.p, &[a], 10 * MS, Some(11_500), Some(200));
    assert_eq!(want, 13_002);
    r.turns(11, 30);
    assert_eq!(r.burst_of(200), (want, Some(11_500), true));
    assert_eq!(r.time_errors(), ["send_asap"]);
    r.check(&o::Slots { segs: vec![o::Seg::once(10, s, 3_000), o::Seg::once(11, want, 200)], cut: None }, s - 10, want + 300);
    // (b) the same where the lead dominates: A (300 samples, front 10 302) is long done at 12 ms.
    let mut r = rig(&[(10, 300), (12, 200)], Vec::new());
    r.book(1, 0, s, false, LatePolicy::DropAndFlag);
    r.turns(0, 12);
    r.book(2, 1, 10_100, false, LatePolicy::SendAsapAndFlag);
    r.turn();
    let want = o::place(&r.p, &[region_of(s, 300)], 12 * MS, Some(10_100), Some(200));
    assert_eq!(want, 14_000);
    r.turns(13, 30);
    assert_eq!(r.burst_of(200), (want, Some(10_100), true));
    r.check(&o::Slots { segs: vec![o::Seg::once(10, s, 300), o::Seg::once(12, want, 200)], cut: None }, s - 10, want + 300);
    // (c) a late DropAndFlag booking in A's region is dropped (lateness is decided before the footprint), not refused.
    let mut r = rig(&[(10, 3_000), (13, 200)], Vec::new());
    r.book(1, 0, s, false, LatePolicy::DropAndFlag);
    r.turns(0, 10);
    r.book(2, 1, 11_500, false, LatePolicy::DropAndFlag);
    r.turn();
    assert_eq!(o::admit(&r.p, &[a], 10 * MS, 11_500, Some(200)), o::Verdict::Late);
    r.turns(11, 30);
    assert_eq!(r.time_errors(), ["drop"]);
    assert_eq!(r.all_refusals(), 0);
    assert!(!r.heard(13, s, 30_000));
    // (d) an untimed booking while A plays: the first free sample, A's front, no TIME_ERROR, no lateness.
    let mut r = rig(&[(10, 3_000), (14, 200)], Vec::new());
    r.book(1, 0, s, false, LatePolicy::DropAndFlag);
    r.turns(0, 10);
    r.book_untimed(2, 1, 10_500);
    r.turn();
    let want = o::place(&r.p, &[a], 10 * MS, Some(10_500), Some(200));
    assert_eq!(want, 13_002);
    r.turns(11, 30);
    assert_eq!(r.burst_of(200), (want, None, false));
    assert!(r.time_errors().is_empty());
    // (e) an untimed booking after A ended: ordinary dominates.
    let mut r = rig(&[(10, 300), (15, 200)], Vec::new());
    r.book(1, 0, s, false, LatePolicy::DropAndFlag);
    r.turns(0, 12);
    r.book_untimed(2, 1, 12_000);
    r.turn();
    let want = o::place(&r.p, &[region_of(s, 300)], 12 * MS, Some(12_000), Some(200));
    assert_eq!(want, 14_000);
    r.turns(13, 30);
    assert_eq!(r.burst_of(200), (want, None, false));
}

// ------------------------------------------------------------------------------------------------------------ T6

/// A short A (ramp 100 at 10 000, long over by the Stop at 20 ms) and the bookings `extra` (`wave`, start) made before it; the owner has
/// taken its last turn at the Stop's instant.
fn before_stop(waves: &[(u8, usize)], extra: &[(usize, i64)]) -> (Rig, i64) {
    let mut r = rig(waves, Vec::new());
    r.book(1, 0, 10_000, false, LatePolicy::DropAndFlag);
    for (i, (wave, start)) in extra.iter().enumerate() {
        r.book(10 + i as u64, *wave, *start, false, LatePolicy::DropAndFlag);
    }
    r.turns(0, 19);
    r.at(20 * MS);
    r.turn();
    r.stop();
    (r, 20 * MS)
}

#[test]
fn o1_uhd_t6a_a_cancellation_that_starts_at_or_after_the_bound_leaves_no_region() {
    // RM-16 as amended (a Stop that cancels a booking starting before B leaves [t, B)) (seq 6): H starts at 50 000, after B = 30 003, so
    // it is cancelled (a `rejected` note, not an event) and leaves no region: D, received after the Stop with start 26 000, is accepted.
    let (mut r, t) = before_stop(&[(1, 100), (3, 300), (4, 200)], &[(1, 50_000)]);
    assert_eq!(r.p.stop_bound(t), (20_000, 30_003));
    assert_eq!(o::stop_region(&r.p, t, &[50_000]), None);
    r.book(2, 2, 26_000, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&r.p, &[], t, 26_000, Some(200)), o::Verdict::Accept);
    r.turns(21, 60);
    assert_eq!(r.cancellations(), 1, "H is cancelled by a rejected note: {:?}", lock(&r.rig.core.rec).rejected);
    assert!(r.refusals().is_empty(), "a cancellation by Stop is no COMMAND_REJECTED event, and D is accepted: {:?}", r.events);
    assert_eq!(
        r.rows().iter().map(|row| (row["target"]["ticks"].as_i64().unwrap(), row["samples"].as_i64().unwrap(), row["end"].as_str().unwrap().to_owned())).collect::<Vec<_>>(),
        [(10_000, 100, "eob".to_owned()), (26_000, 200, "eob".to_owned())]
    );
    r.check(&o::Slots { segs: vec![o::Seg::once(1, 10_000, 100), o::Seg::once(4, 26_000, 200)], cut: None }, 9_990, 50_400);
}

/// T6b's four bookings after the Stop at 20 ms, one rig each: the oracle's answer to each is compared with the Provider's.
#[test]
fn o1_uhd_t6b_a_cancellation_that_starts_before_the_bound_leaves_the_region() {
    // RM-16 (seq 6): H2 starts at window_end + 1 = 30 001, inside [window_end, B): the owner's window edge is 30 000, so it is not
    // dispatched and is still held at the Stop, where it is cancelled (a `rejected` note); it starts in [t, B), so [20 000, 30 003) is a
    // Stop region. Every later booking is judged by it. (The plan's `window_end - 16` is dispatched by the owner into its open slot and
    // dropped by the Stop with no note and no record, where MR-25 records a cancellation: reported to the orchestrator, not tested.)
    for case in ["drop inside", "send_asap inside", "untimed", "drop at B"] {
        let (mut r, t) = before_stop(&[(1, 100), (3, 300), (4, 200)], &[(1, 30_001)]);
        let region = o::stop_region(&r.p, t, &[30_001]).expect("H2 starts in [t_k, B)");
        assert_eq!((region.start, region.front), (20_000, 30_003));
        let want_start = match case {
            "drop inside" => {
                r.book(2, 2, 29_000, false, LatePolicy::DropAndFlag);
                assert_eq!(o::admit(&r.p, &[region], t, 29_000, Some(200)), o::Verdict::Refuse);
                None
            }
            "send_asap inside" => {
                r.book(2, 2, 29_000, false, LatePolicy::SendAsapAndFlag);
                Some(o::place(&r.p, &[region], t, Some(29_000), Some(200)))
            }
            "untimed" => {
                r.book_untimed(2, 2, 29_000);
                Some(o::place(&r.p, &[region], t, Some(29_000), Some(200)))
            }
            _ => {
                r.book(2, 2, 30_003, false, LatePolicy::DropAndFlag);
                assert_eq!(o::admit(&r.p, &[region], t, 30_003, Some(200)), o::Verdict::Accept);
                Some(30_003)
            }
        };
        r.turns(21, 60);
        let notes = lock(&r.rig.core.rec).rejected.clone();
        assert_eq!(r.cancellations(), 1, "{case}: H2 is cancelled: {notes:?} / {:?}", r.rows());
        assert!(r.rows().iter().all(|row| row["samples"] != 300), "{case}: H2 never played: {:?}", r.rows());
        let refusals = r.refusals();
        match want_start {
            None => assert_eq!(refusals.len(), 1, "{case}: {refusals:?}"),
            Some(start) => {
                assert!(refusals.is_empty(), "{case}: {refusals:?}");
                assert_eq!(r.burst_of(200), (start, (case == "send_asap inside").then_some(29_000), case == "send_asap inside"), "{case}");
                assert_eq!(start, 30_003, "{case}: the first sample past the region");
                r.check(&o::Slots { segs: vec![o::Seg::once(1, 10_000, 100), o::Seg::once(4, start, 200)], cut: None }, 9_990, start + 300);
            }
        }
        assert_eq!(r.time_errors(), if case == "send_asap inside" { vec!["send_asap"] } else { vec![] }, "{case}");
    }
}

#[test]
fn o1_uhd_t6b2_an_unhanded_open_booking_at_a_stop_is_cancelled_known_divergence_85() {
    // RM-16, MR-25 (seq 6): H2 starts at window_end - 16 = 29 984, inside (window_end - 32, B): the owner dispatches it into its open slot
    // but cannot hand its first 32 samples, so at the Stop it is unhanded. The spec: cancelled, one `rejected` cancellation note, no
    // sample, no burst record. Today's UHD drops it in `end_open` with no note (#85).
    let (mut r, t) = before_stop(&[(1, 100), (3, 300), (4, 200)], &[(1, 29_984)]);
    assert_eq!(o::stop_region(&r.p, t, &[29_984]).map(|region| region.front), Some(30_003));
    r.turns(21, 60);
    let spec = o::unhanded_booking_at_a_stop();
    assert_eq!(spec, o::Cancelled { notes: 1, records: 0, samples: false }, "the oracle's own answer, independent of the code");
    // The normal assertions hold today: no sample of H2 and no burst record of 300 samples.
    assert_eq!(r.heard(3, 0, 60_000), spec.samples);
    assert_eq!(r.rows().iter().filter(|row| row["samples"] == 300).count(), spec.records);
    // KNOWN DIVERGENCE #85: today's UHD leaves no cancellation note. E1/E2a delete only this block and assert `notes == spec.notes`.
    let notes = r.cancellations();
    assert_eq!(notes, 0, "#85 fixed? delete this block and assert notes == spec.notes");
    assert_ne!(notes, spec.notes);
}

#[test]
fn o1_uhd_t6c_a_booking_after_a_stop_on_a_playing_burst_survives() {
    // RM-16 (seq 6): A is 12 000 samples from 10 000 (longer than the window, so the Stop cuts it). The Stop at 11 ms cuts it in
    // [t_k, B]; the held booking H3 (start 40 000, after B) is cancelled; E, received after the Stop with start 30 000, plays exactly.
    let mut r = rig(&[(5, 12_000), (6, 200), (3, 300)], Vec::new());
    r.book(1, 0, 10_000, false, LatePolicy::DropAndFlag);
    r.book(2, 2, 40_000, false, LatePolicy::DropAndFlag);
    r.turns(0, 10);
    r.at(11 * MS);
    r.turn();
    let (t_k, bound) = r.p.stop_bound(11 * MS);
    r.stop();
    r.book(3, 1, 30_000, false, LatePolicy::DropAndFlag);
    let cut_region = o::Region { start: 10_000, end: bound, front: bound, len: bound - 10_000 };
    assert_eq!(o::admit(&r.p, &[cut_region], 11 * MS, 30_000, Some(200)), o::Verdict::Accept);
    r.turns(12, 60);
    let slots = o::Slots { segs: vec![o::Seg::once(5, 10_000, 12_000).cuttable(), o::Seg::once(6, 30_000, 200)], cut: Some((t_k, bound)) };
    let cut = r.check(&slots, 9_990, 30_300).expect("A is cut");
    assert!(cut < 22_000);
    assert_eq!(r.cancellations(), 1, "H3: {:?}", lock(&r.rig.core.rec).rejected);
    assert!(r.refusals().is_empty());
    let rows = r.rows();
    assert_eq!((rows[0]["samples"].as_i64(), rows[0]["end"].as_str()), (Some(cut - 10_000), Some("stop")), "{rows:?}");
    assert_eq!((rows[1]["target"]["ticks"].as_i64(), rows[1]["samples"].as_i64()), (Some(30_000), Some(200)));
}

#[test]
fn o1_uhd_t6d_a_booking_after_a_stop_survives_the_failing_close_of_the_burst_it_cut() {
    // RM-16, UR-22 (seq 6, the `A -> Stop -> B` gap of the retained-test map): A (12 000 samples from 10 000) is cut by a Stop at 11 ms whose
    // closing send fails (`FailSend` of exactly that send, the fault the fake already scripts; its index is counted on a fault-free dry run).
    // The failed close leaves the streamer uncertain, so the owner reopens it (a flush, run as a participant of the device's clock) before any
    // further payload. B, booked after the Stop with start 30 000 (after the Stop's bound), is accepted and plays exactly from its start:
    // its reservation is not erased by A's failure.
    let waves = [(5u8, 12_000usize), (6, 200)];
    let scenario = |faults: Vec<FakeFault>| {
        let mut r = rig(&waves, faults);
        r.book(1, 0, 10_000, false, LatePolicy::DropAndFlag);
        r.turns(0, 10);
        r.at(11 * MS);
        r.turn();
        r
    };
    let payload_sends = |r: &Rig| r.rig.device.calls().iter().filter(|call| call.starts_with("tx_send ") && !call.contains("n=0 ")).count();
    let before_stop = payload_sends(&scenario(Vec::new()));
    let mut r = scenario(vec![FakeFault::FailSend(before_stop)]);
    let (t_k, bound) = r.p.stop_bound(11 * MS);
    // The Stop is booked by the control and not yet drained; B is booked after it; then the owner takes both in order: the Stop (whose close
    // fails) and then B, so B's booking is received before the owner has recovered from A's failed close.
    let node = r.rig.core.tx_id.clone();
    r.rig.control.book(Dispatched::new(ActionId(9), Action::Stop { target: Some(crate::provider::test_support::target_of(&node)) }, Some(node)));
    r.book(2, 1, 30_000, false, LatePolicy::DropAndFlag);
    drain_tx(&mut r.rig.tx);
    assert!(r.rig.tx.chain.needs_reopen(), "the Stop's close failed, so the streamer is uncertain");
    assert!(lock(&r.rig.core.rec).rejected.iter().any(|row| row["call"] == "tx_close"), "the failed close is noted: {:?}", lock(&r.rig.core.rec).rejected);
    // A's samples are read before the streamer is replaced (the fake drops the old streamer's history on the reopen).
    let a_slots = o::Slots { segs: vec![o::Seg::once(5, 10_000, 12_000).cuttable()], cut: Some((t_k, bound)) };
    let cut = r.check(&a_slots, 9_990, 21_100).expect("A is cut by the Stop");
    assert!((t_k..=bound).contains(&cut), "the cut {cut} is outside [{t_k}, {bound}]");
    r.take();
    let refused: Vec<_> = r.events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED && e.payload["kind"] == "tx_burst").collect();
    assert!(refused.is_empty(), "B is accepted at receipt (the one rejection is the failed close): {:?}", r.events);
    let Rig { rig, .. } = r;
    let SwitchRig { core, device, time, mut tx, clock, _frozen: frozen, .. } = rig;
    let ((mut tx, passed), frozen) = crate::provider::test_support::released(frozen, &device.host(), move || {
        let passed = tx.pass();
        (tx, passed)
    });
    assert!(passed, "the owner reopened the uncertain streamer");
    assert!(!tx.chain.needs_reopen());
    for ms in 21..=40 {
        device_at(&core, &time, &device, ms * MS);
        for _ in 0..8 {
            drain_tx(&mut tx);
            if !tx.pass() {
                break;
            }
        }
    }
    let seen = device.transmitted(clock.instant(29_990), 310);
    o::Slots { segs: vec![o::Seg::once(6, 30_000, 200)], cut: None }.check(29_990, 30_300, &|k| seen[(k - 29_990) as usize]).unwrap_or_else(|error| panic!("{error}: {:?}", device.calls()));
    let rows = lock(&core.rec).bursts.clone();
    let ends: Vec<_> = rows.iter().map(|row| (row["target"]["ticks"].as_i64().unwrap(), row["end"].as_str().unwrap().to_owned())).collect();
    assert_eq!(ends, [(10_000, "stop".to_owned()), (30_000, "eob".to_owned())], "{rows:?}");
    assert_eq!(rows[1]["samples"], 200, "B played whole");
    drop(frozen);
}

// ------------------------------------------------------------------------------------------------------------ T7

/// A repeat of waveform 0 from 2 000 handed at 1 ms (three 2 500-sample sends), a switch to waveform 1 received at 2 ms (boundary 14 000),
/// and the owner's turns to 8 ms. `FailSend(k)` fails the k-th send that carries samples (a tail and a close included).
fn failing_switch(k: usize) -> Rig {
    let mut r = rig(&[(17, 1_000), (18, 400)], vec![FakeFault::FailSend(k)]);
    r.book(1, 0, 2_000, true, LatePolicy::DropAndFlag);
    r.turns(0, 1);
    r.at(2 * MS);
    book_switch(&mut r.rig, 2, 2_000, 1);
    r.turn();
    r.turns(3, 8);
    r
}

/// The failing send's `(first sample, attempted n)`: `calls()` logs the accepted half, so the attempt is twice the logged `n`, and
/// the attempt is even by construction.
fn failed_attempt(r: &Rig, index: usize, expect_at: i64) -> (i64, i64) {
    let sends = r.sends();
    let (a, half, _) = sends[index];
    assert_eq!(a, expect_at, "the failing entry is the intended send: {sends:?}");
    (a, 2 * half)
}

#[test]
fn o1_uhd_t7a_a_failed_send_ends_a_repeat_at_its_attempted_end_and_cancels_the_switch() {
    // UR-22, RM-37 (seq 7): FailSend(3) fails the fourth payload send, [9 500, 12 000), before the switch's boundary 14 000. The fake takes
    // half (1 250): [2 000, 10 750) sounds. The owner's abandon cursor is the ATTEMPTED end 12 000: the repeat's region runs to 12 003, the
    // switch is cancelled and no new sample ever sounds; the record counts the three sends that completed. (Mutation row O1-M6.)
    let r = failing_switch(3);
    let (a, attempt) = failed_attempt(&r, 3, 9_500);
    assert_eq!(attempt, 2_500);
    let cursor = a + attempt;
    let sends = r.sends();
    let completed: Vec<(i64, i64)> = sends[..3].iter().map(|(k, n, _)| (*k, *n)).collect();
    assert_eq!(o::recorded(&completed), 7_500);
    r.check(&o::Slots { segs: vec![o::Seg::repeating(17, 2_000, 1_000, a + attempt / 2)], cut: None }, 1_990, cursor);
    assert!(!r.heard(18, 2_000, 30_000), "no sample of the new waveform ever sounds");
    assert_eq!(r.switch_row(0)["outcome"], o::outcome_after_abandon(14_000, cursor).as_str());
    let rows = r.rows();
    assert_eq!((rows[0]["samples"].as_i64(), rows[0]["end"].as_str()), (Some(7_500), Some("stop")), "{rows:?}");
}

#[test]
fn o1_uhd_t7a2_the_region_of_an_abandoned_repeat_is_its_cursor_plus_three() {
    // RM-15 (seq 7): the same abandon as T7a, with the bookings made at 2 ms, right after it (the failing send is the owner's turn at 2 ms).
    let mut r = rig(&[(17, 1_000), (18, 400), (19, 100)], vec![FakeFault::FailSend(3)]);
    r.book(1, 0, 2_000, true, LatePolicy::DropAndFlag);
    r.turns(0, 1);
    r.at(2 * MS);
    book_switch(&mut r.rig, 2, 2_000, 1);
    r.turn();
    let (a, attempt) = failed_attempt(&r, 3, 9_500);
    let cursor = a + attempt;
    let region = o::Region { start: 2_000, end: cursor, front: cursor + 3, len: cursor - 2_000 };
    r.take();
    r.book(5, 2, cursor + 2, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&r.p, &[region], 2 * MS, cursor + 2, Some(100)), o::Verdict::Refuse);
    assert_eq!(r.refusals().len(), 1);
    r.book(6, 2, cursor + 3, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&r.p, &[region], 2 * MS, cursor + 3, Some(100)), o::Verdict::Accept);
    assert!(r.refusals().is_empty());
}

#[test]
fn o1_uhd_t7a3_a_failed_send_that_ends_at_the_boundary_still_cancels_the_switch() {
    // RM-37 (seq 7): FailSend(4) fails [12 000, 14 000), whose attempted end is the boundary 14 000: the cursor has not handed the sample at
    // b, so the switch is cancelled.
    let r = failing_switch(4);
    let (a, attempt) = failed_attempt(&r, 4, 12_000);
    assert_eq!(a + attempt, 14_000);
    assert_eq!(r.switch_row(0)["outcome"], o::outcome_after_abandon(14_000, a + attempt).as_str());
    assert!(!r.heard(18, 2_000, 30_000));
    r.check(&o::Slots { segs: vec![o::Seg::repeating(17, 2_000, 1_000, a + attempt / 2)], cut: None }, 1_990, a + attempt);
}

#[test]
fn o1_uhd_t7c_a_failed_send_that_carries_the_boundary_leaves_the_switch_applied_today() {
    // RM-37 (seq 7): FailSend(5) fails the send that carries b, [14 000, 16 500). The sample at b was attempted, so the row is `applied`
    // today and half of it sounds. C2 (note 49 §5.4.5: an `uncertain` outcome for a failed send at b) will change the ROW below.
    let r = failing_switch(5);
    let (a, attempt) = failed_attempt(&r, 5, 14_000);
    assert_eq!(a + attempt, 16_500);
    assert_eq!(o::outcome_after_abandon(14_000, a + attempt), o::Outcome::Applied);
    assert_eq!(r.switch_row(0)["outcome"], "applied", "C2 changes this row");
    r.check(&o::Slots { segs: vec![o::Seg::repeating(17, 2_000, 1_000, 14_000), o::Seg::repeating(18, 14_000, 400, a + attempt / 2)], cut: None }, 1_990, a + attempt);
}

#[test]
fn o1_uhd_t7b_an_abandoned_finite_booking_keeps_its_whole_region() {
    // UR-22, RM-15, KL-39 (seq 7): F is 3 001 samples from 2 000: a 2 500-sample send, then a 501-sample send whose last sample is held,
    // so the failing send carries 500 (FailSend(1); the fake takes 250). The record counts the first send only; F keeps [2 000, 5 003):
    // a booking at 4 900 is refused, one at the front 5 003 is accepted.
    let mut r = rig(&[(19, 3_001), (20, 100)], vec![FakeFault::FailSend(1)]);
    r.book(1, 0, 2_000, false, LatePolicy::DropAndFlag);
    r.turn();
    let sends = r.sends();
    assert!(sends.len() >= 2, "{sends:?}");
    assert_eq!((sends[1].0, 2 * sends[1].1), (4_500, 500), "the failing send is the second: {sends:?}");
    r.check(&o::Slots { segs: vec![o::Seg { end: 4_500 + 250, ..o::Seg::once(19, 2_000, 3_001) }], cut: None }, 1_990, 5_003);
    let rows = r.rows();
    assert_eq!((rows[0]["samples"].as_i64(), rows[0]["end"].as_str()), (Some(o::recorded(&[(2_000, 2_500)])), Some("stop")), "{rows:?}");
    let f = region_of(2_000, 3_001);
    assert_eq!(f.front, 5_003);
    r.take();
    r.book(2, 1, 4_900, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&r.p, &[f], 0, 4_900, Some(100)), o::Verdict::Refuse);
    assert_eq!(r.refusals().len(), 1);
    r.book(3, 1, f.front, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&r.p, &[f], 0, f.front, Some(100)), o::Verdict::Accept);
    assert!(r.refusals().is_empty());
}

// ------------------------------------------------------------------------------------------------------------ T8

#[test]
fn o1_uhd_t8_a_switch_whose_repeat_was_dropped_at_dispatch_is_cancelled_known_divergence_84() {
    // RM-37, UR-38 (seq 8): the host dispatches the repeat after the device passed its start (3 ms, start 2 000), so DropAndFlag drops it and
    // the switch that replaces it has nothing to replace: the spec says cancelled. Today's code settles `refused_past_boundary` (#84).
    let mut r = rig(&[(8, 1_000), (9, 1_000)], Vec::new());
    r.book(1, 0, 2_000, true, LatePolicy::DropAndFlag);
    r.at(3 * MS);
    book_switch(&mut r.rig, 2, 3_000, 1);
    drain_tx(&mut r.rig.tx);
    let spec = o::outcome_after_repeat_dropped();
    assert_eq!(spec, o::Outcome::Cancelled, "the oracle's own answer, independent of the code");
    let today = r.switch_row(0)["outcome"].as_str().unwrap().to_owned();
    // KNOWN DIVERGENCE #84: today's code settles refused_past_boundary. E2a deletes only this block and asserts `today == spec`.
    assert_eq!(today, "refused_past_boundary", "#84 fixed? delete this block and assert today == spec");
    assert_ne!(today, spec.as_str());
    // The normal assertions hold today: the repeat never played and no new sample ever sounds, one `switches` row.
    r.turns(4, 30);
    assert!(!r.heard(8, 0, 30_000) && !r.heard(9, 0, 30_000));
    r.check(&o::Slots::default(), 1_990, 20_000);
    assert_eq!(lock(&r.rig.core.rec).switches.len(), 1);
}
