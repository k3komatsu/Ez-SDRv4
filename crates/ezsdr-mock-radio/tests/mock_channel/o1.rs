//! O1b (design note 49 §5.4.11, plan `2026-11-09-o1/test-plan.md`): the spec oracle against MockRadio's own stepped harness, with the samples
//! read through the SimulationChannel loopback (MR-31, MR-32): a transmit stream coupled to a receive stream hears exactly the radiated plan.
//! `Radio::received(0)` is the sample view; no production accessor is used.
//!
//! The Mock is deterministic: a receipt is the tick a test steps to, never a thread's timing. On `ideal` (no window, no lead, deterministic
//! phase) the loopback sample equals the transmitted one; on `x3xx` (window 10 ms, lead 2 ms, allowance 0, 45 samples of transmit path delay,
//! LO phases drawn from `sim.seed`) the oracle's sample is identified by undoing the existing `phases` rotation, as `mr_32_the_transmit_path_
//! delay_shifts_a_loopback` does. All samples are one channel. Sample indices are absolute on the transmit clock (origin 0, 1 MS/s, 1 000 root
//! ticks a sample); the receive stream starts at `T0`, 2 000 000 samples in.

use super::*;
use crate::oracle as o;
use crate::oracle::Iq;

const X: i64 = 2_000_000;
const T0: i64 = X * 1_000;
const SEED: u64 = 3;

struct M {
    world: World,
    radio: Radio,
    p: o::Params,
    base: i64,
    delay: i64,
    rot: (f64, f64),
    waves: Vec<ArtifactRef>,
    now: i64,
    events: Vec<Event>,
    rx: Option<BTreeMap<i64, (f32, f32)>>,
}

fn build(world: World, radio: Radio, p: o::Params, base: i64, delay: i64, rot: (f64, f64), waves: &[(u8, usize)]) -> M {
    let waves = waves.iter().map(|(tag, len)| world.waveform(&o::ramp(*tag, *len).iter().map(|s| (s[0], s[1])).collect::<Vec<_>>())).collect();
    M { world, radio, p, base, delay, rot, waves, now: base * 1_000, events: Vec::new(), rx: None }
}

/// The `x3xx` Mock on a loopback, started at `T0`, with the waveforms `ramp(tag, len)` in order.
fn x3xx(waves: &[(u8, usize)]) -> M {
    let world = World::new(json!({ "sim.seed": SEED, "sim.channel": { "couplings": [{ "tx": "radio", "tx_channel": 0, "rx": "radio", "rx_channel": 0, "gain_db": 0.0 }] } }));
    let constraints = [("radio.tx.c0.stream", eq(Scalar::from(0))), ("radio.tx.c0.gain_db", eq(Scalar::try_from(0.0).unwrap()))];
    let mut radio = prepare(&world, Options { profile: "x3xx", constraints: &constraints, ..Options::default() }).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, T0))).unwrap();
    step(&world, &mut [&mut radio], T0);
    let Phases { rx, tx, .. } = phases(SEED, "mock");
    let (sin, cos) = (tx[0] - rx[0]).sin_cos();
    // The Mock's profile in the oracle's terms: window 10 ms, device lead 2 ms (`min_timed_command_lead_ns`), and the emulated UHD allowance
    // (3 ms, MR-3) that RM-15's join deadline uses.
    let p = o::Params { clock: o::Clock { origin: 0, n: 1_000 }, root_hz: 1_000_000_000, window: 10_000_000, lead: 2_000_000, allowance: 3_000_000 };
    let mut m = build(world, radio, p, X, 45, (sin, cos), waves);
    m.now = T0;
    m
}

/// The `ideal` Mock (zero window, zero lead, deterministic phase) on a loopback, started at 0.
fn ideal(waves: &[(u8, usize)]) -> M {
    let world = World::new(loopback(None));
    let mut radio = prepare(&world, Options::default()).unwrap();
    radio.mock.arm().unwrap();
    radio.mock.start(Some(TimePoint::new(ROOT, 0))).unwrap();
    step(&world, &mut [&mut radio], 0);
    let p = o::Params { clock: o::Clock { origin: 0, n: 1_000 }, root_hz: 1_000_000_000, window: 0, lead: 0, allowance: 0 };
    build(world, radio, p, 0, 0, (0.0, 1.0), waves)
}

impl M {
    /// The tick of sample `k` (absolute).
    fn tick(&self, k: i64) -> i64 {
        k * 1_000
    }

    /// Steps the Mock to root tick `tick`; the Actions queued before it are received there.
    fn go_tick(&mut self, tick: i64) {
        assert!(self.rx.is_none(), "samples are read once, after the last step");
        step(&self.world, &mut [&mut self.radio], tick);
        self.now = tick;
    }

    /// Steps to the instant of sample `k`.
    fn go(&mut self, k: i64) {
        self.go_tick(self.tick(k));
    }

    fn push(&mut self, action: Action) {
        self.radio.push(action);
        self.go_tick(self.now);
    }

    fn action(&self, wave: usize, at: i64, repeat: bool, policy: LatePolicy, switch: bool) -> Action {
        let metadata = if switch { BTreeMap::from([(key("radio.repeat_operation"), Value::Scalar(Scalar::from("switch")))]) } else { BTreeMap::new() };
        Action::TxBurst {
            target: tgt(&format!("{}/tx/s0", self.radio.id)),
            waveform: self.waves[wave].clone(),
            repeat,
            at: AbsoluteDeadline::new(TimePoint::new(self.radio.tx_domain(&self.world), at)),
            requested_at: None,
            late_policy: policy,
            metadata,
        }
    }

    /// A timed booking at sample `at`, received at the current tick.
    fn book(&mut self, wave: usize, at: i64, repeat: bool, policy: LatePolicy) {
        let action = self.action(wave, at, repeat, policy, false);
        self.push(action);
    }

    fn book_switch(&mut self, wave: usize, at: i64) {
        let action = self.action(wave, at, true, LatePolicy::SendAsapAndFlag, true);
        self.push(action);
    }

    /// An untimed booking (RS-19) stamped `at`, received at the current tick.
    fn book_untimed(&mut self, wave: usize, at: i64) {
        let action = self.action(wave, at, false, LatePolicy::SendAsapAndFlag, false);
        self.radio.actions.1.store(true, Ordering::SeqCst);
        self.push(action);
    }

    fn stop(&mut self) {
        self.push(Action::Stop { target: Some(tgt(&format!("{}/tx/s0", self.radio.id))) });
    }

    fn take(&mut self) -> Vec<Event> {
        let new = self.radio.drain_events();
        self.events.extend(new.clone());
        new
    }

    /// The reasons of the `COMMAND_REJECTED` events since the last call: refusals at receipt.
    fn refusals(&mut self) -> Vec<String> {
        self.take().iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).map(|e| e.payload["reason"].as_str().unwrap_or_default().to_owned()).collect()
    }

    fn all_refusals(&mut self) -> usize {
        self.take();
        self.events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::COMMAND_REJECTED).count()
    }

    fn time_errors(&mut self) -> Vec<String> {
        self.take();
        self.events.iter().filter(|e| e.kind.as_str() == ezsdr_radio::kinds::TIME_ERROR).map(|e| e.payload["outcome"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// The `rejected` notes of kind `tx_burst` that are not refusals at receipt (a refusal is also a `COMMAND_REJECTED` event): the bookings
    /// a Stop cancelled.
    fn cancellations(&mut self) -> usize {
        let rows = self.radio.section("rejected").as_array().unwrap().iter().filter(|row| row["kind"] == "tx_burst").count();
        rows - self.all_refusals()
    }

    /// The Kernel burst records as `(target sample, payload samples, requested target, late, end)`.
    fn recs(&self) -> Vec<(i64, u64, Option<i64>, bool, BurstEnd)> {
        records(&self.radio)
            .into_iter()
            .map(|r| (r.target.ticks_in(r.target.domain()).unwrap(), r.samples, r.requested_target.map(|t| t.ticks_in(t.domain()).unwrap()), r.late_by.is_some(), r.end))
            .collect()
    }

    fn pairs(&self) -> Vec<(i64, u64)> {
        self.recs().iter().map(|(t, n, ..)| (*t, *n)).collect()
    }

    fn rec_of(&self, samples: u64) -> (i64, Option<i64>, bool) {
        let rows: Vec<_> = self.recs().into_iter().filter(|r| r.1 == samples).collect();
        assert_eq!(rows.len(), 1, "one record of {samples} samples: {:?}", self.recs());
        (rows[0].0, rows[0].2, rows[0].3)
    }

    fn switch_rows(&self) -> Vec<serde_json::Value> {
        self.radio.section("switches").as_array().unwrap().clone()
    }

    /// The switch row's identities: the switch's action id and the repeat it replaces (the queue gives ids in order: the repeat is 1, the switch 2).
    fn assert_switch_ids(&self, index: usize) {
        let row = &self.switch_rows()[index];
        assert_eq!((row["action_id"].as_u64(), row["predecessor"].as_u64()), (Some(2), Some(1)), "the switch row's identities: {row}");
    }

    /// The successor's record in full: `(target, samples, wraps, requested target, late, end)` of record `index`.
    fn full_rec(&self, index: usize) -> (i64, u64, u32, Option<i64>, bool, &'static str) {
        let r = &records(&self.radio)[index];
        (r.target.ticks_in(r.target.domain()).unwrap(), r.samples, r.wraps, r.requested_target.map(|t| t.ticks_in(t.domain()).unwrap()), r.late_by.is_some(), end_of(r.end))
    }

    /// The transmit sample `j` as the loopback delivers it, identified against the oracle's waveforms: undo the LO rotation, then take the
    /// index and tag the sample encodes. A sample that is not one of the oracle's is returned as NaN, which matches nothing.
    fn observed(&mut self, j: i64) -> Option<Iq> {
        let radio = &self.radio;
        let rx = self.rx.get_or_insert_with(|| radio.received(0).into_iter().map(|(k, re, im)| (k, (re, im))).collect());
        let k = j - self.base + self.delay;
        let (re, im) = *rx.get(&k).unwrap_or_else(|| panic!("receive sample {k} (transmit {j}) was not delivered: step further"));
        let (re, im) = (f64::from(re), f64::from(im));
        if re.abs() < 1e-9 && im.abs() < 1e-9 {
            return None;
        }
        let (sin, cos) = self.rot;
        let (tre, tim) = (re * cos + im * sin, -re * sin + im * cos);
        let (idx, tag) = ((tre * 32_768.0).round() - 1.0, (tim * 64.0).round());
        let want = o::sample(tag as u8, idx.max(0.0) as usize);
        let close = (f64::from(want[0]) - tre).abs() < 1e-6 && (f64::from(want[1]) - tim).abs() < 1e-6;
        Some(if close { want } else { [f32::NAN, f32::NAN] })
    }

    fn check(&mut self, slots: &o::Slots, from: i64, to: i64) {
        let seen: Vec<Option<Iq>> = (from..to).map(|j| self.observed(j)).collect();
        if let Err(error) = slots.check(from, to, &|k| seen[(k - from) as usize]) {
            panic!("{error}");
        }
    }

    fn heard(&mut self, tag: u8, from: i64, to: i64) -> bool {
        (from..to).any(|j| self.observed(j).is_some_and(|s| (s[1] * 64.0).round() as u8 == tag))
    }
}

fn end_of(end: BurstEnd) -> &'static str {
    match end {
        BurstEnd::Eob => "eob",
        BurstEnd::Stop => "stop",
        BurstEnd::Discontinuity => "discontinuity",
    }
}

fn region_of(start: i64, len: i64) -> o::Region {
    // Both Mock rigs run at 1 MS/s, so the closed short gap is 15 samples (RM-15); only the gap of the params is read.
    o::region(&o::Params { clock: o::Clock { origin: 0, n: 1_000 }, root_hz: 1_000_000_000, window: 0, lead: 0, allowance: 0 }, start, Some(len))
}

// ------------------------------------------------------------------------------------------------------------ T1

#[test]
fn o1_mock_t1_short_and_long_heads_front_table() {
    // RM-15 (seq 1): the same table as the UHD's: a head of L samples fronts at its padded end plus ten (L <= 32) or its end plus two; a
    // booking at the front is accepted and its samples are exact, one at front - 1 is refused. The loopback shows every sample, so a wrong
    // position or index fails here (mutation row O1-M7a).
    let s = X + 10_000;
    for len in [1usize, 3, 4, 31, 32, 33, 3_000] {
        for accept in [true, false] {
            let mut m = x3xx(&[(1, len), (2, 40)]);
            let a = region_of(s, len as i64);
            let b_start = if accept { a.front } else { a.front - 1 };
            m.book(0, s, false, LatePolicy::DropAndFlag);
            m.book(1, b_start, false, LatePolicy::DropAndFlag);
            assert_eq!(o::admit(&m.p, &[a], T0, b_start, Some(40)), if accept { o::Verdict::Accept } else { o::Verdict::Refuse }, "L={len} accept={accept}");
            m.go(X + 60_000);
            let reasons = m.refusals();
            assert_eq!(reasons.len(), usize::from(!accept), "L={len} accept={accept}: {reasons:?}");
            let mut slots = o::Slots { segs: vec![o::Seg::once(1, s, len as i64)], cut: None };
            if accept {
                slots.segs.push(o::Seg::once(2, b_start, 40));
            }
            m.check(&slots, s - 3, b_start + 60);
            let want: Vec<(i64, u64)> = if accept { vec![(s, len as u64), (b_start, 40)] } else { vec![(s, len as u64)] };
            assert_eq!(m.pairs(), want, "L={len} accept={accept}");
            assert!(m.time_errors().is_empty());
        }
    }
}

// ------------------------------------------------------------------------------------------------------------ T2

#[test]
fn o1_mock_t2_a_join_is_inclusive_at_its_deadline() {
    // RM-15, MR-3 (seq 2): B at the exact end of a 3 000-sample head joins it when received no later than J(e) = start - (lead + allowance), the
    // instant itself included, and is refused one root tick after. The Mock's allowance in the deadline is the emulated UHD Module's, 3 ms
    // (`Params.allowance` below), so the Mock is never looser than the UHD Module.
    let (s, e) = (X + 20_000, X + 23_000);
    let a = region_of(s, 3_000);
    let j = x3xx(&[]).p.join_by(e);
    assert_eq!(j, e * 1_000 - 5_000_000);
    for (case, recv) in [("J - 1 tick", j - 1), ("J", j), ("J + 1 tick", j + 1)] {
        let mut m = x3xx(&[(5, 3_000), (6, 500)]);
        m.book(0, s, false, LatePolicy::DropAndFlag);
        m.go_tick(recv);
        m.book(1, e, false, LatePolicy::DropAndFlag);
        let accepted = o::admit(&m.p, &[a], recv, e, Some(500)) == o::Verdict::Accept;
        assert_eq!(accepted, recv <= j, "{case}");
        m.go(X + 70_000);
        let refusals = m.refusals();
        assert_eq!(refusals.len(), usize::from(!accepted), "{case}: {refusals:?}");
        let mut slots = o::Slots { segs: vec![o::Seg::once(5, s, 3_000)], cut: None };
        if accepted {
            slots.segs.push(o::Seg::once(6, e, 500));
        }
        m.check(&slots, s - 2, e + 520);
        let want: Vec<(i64, u64)> = if accepted { vec![(s, 3_000), (e, 500)] } else { vec![(s, 3_000)] };
        assert_eq!(m.pairs(), want, "{case}");
    }
}

#[test]
fn o1_mock_t2c_a_booking_joined_at_a_heads_end_with_a_later_one_booked_early_plays_on_time() {
    // RM-15 (seq 2; the exact-timing property of the parity test `s15_parity_s4b_...`, S4b R4-5): A (40 samples) at X + 20 000 booked at 6 ms,
    // C (10) 100 samples past it at 11 ms, then B (10) joining A at its end X + 20 040 at 12 ms, before the join deadline: all three play, no
    // refusal, no TIME_ERROR, three records, the samples contiguous from A into B.
    let s = X + 20_000;
    let mut m = x3xx(&[(1, 40), (2, 10), (3, 10)]);
    m.go(X + 6_000);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(X + 11_000);
    m.book(2, s + 100, false, LatePolicy::DropAndFlag);
    m.go(X + 12_000);
    m.book(1, s + 40, false, LatePolicy::DropAndFlag);
    m.go(X + 40_000);
    assert_eq!(m.all_refusals(), 0);
    assert!(m.time_errors().is_empty());
    m.check(&o::Slots { segs: vec![o::Seg::once(1, s, 40), o::Seg::once(2, s + 40, 10), o::Seg::once(3, s + 100, 10)], cut: None }, s - 10, s + 200);
    assert_eq!(m.pairs(), [(s, 40), (s + 40, 10), (s + 100, 10)]);
}

// ------------------------------------------------------------------------------------------------------------ T3

#[test]
fn o1_mock_t3_a_repeat_keeps_its_phase_across_wraps_and_a_stop_cuts_it_at_mr_25s_point() {
    // RM-13, MR-25 (seq 3): a 1 000-sample repeat from X + 2 000 is `R[(k - s) mod 1000]` for as many wraps as run; a Stop at 4.5 ms cuts it at
    // the exact MR-25 point, the first sample at or after t + W (the repeat is open, so it counts to the window's edge). (Mutation rows
    // O1-M4 and O1-M7b.)
    let s = X + 2_000;
    let mut m = x3xx(&[(7, 1_000)]);
    m.book(0, s, true, LatePolicy::DropAndFlag);
    m.go(X + 4_500);
    let t = m.tick(X + 4_500);
    m.stop();
    m.go(X + 40_000);
    let cut = o::mock_cut(&m.p, t, &[o::MBurst { start: s, len: None, started: true, before_stop: true }], None);
    assert_eq!(cut, X + 14_500);
    m.check(&o::Slots { segs: vec![o::Seg::repeating(7, s, 1_000, cut)], cut: None }, s - 10, cut + 60);
    let recs = m.recs();
    assert_eq!(recs.len(), 1);
    assert_eq!((recs[0].0, recs[0].1, end_of(recs[0].4)), (s, (cut - s) as u64, "stop"));
    assert_eq!(records(&m.radio)[0].wraps as i64, (cut - s) / 1_000);
}

// ------------------------------------------------------------------------------------------------------------ T4

/// R (`period` long) from `X + 2 000` and a switch to waveform 1 received at sample `recv` for `at`, on the `x3xx` Mock.
/// The `ideal` Mock's twin of [`switched`]: R from 2 000, the switch received at `recv`.
fn ideal_switched(period: usize, new_period: usize, recv: i64, at: i64) -> M {
    let mut m = ideal(&[(8, period), (9, new_period)]);
    m.book(0, 2_000, true, LatePolicy::DropAndFlag);
    m.go(recv);
    m.book_switch(1, at);
    m
}

fn switched(period: usize, new_period: usize, recv: i64, at: i64) -> M {
    let mut m = x3xx(&[(8, period), (9, new_period)]);
    m.book(0, X + 2_000, true, LatePolicy::DropAndFlag);
    m.go(recv);
    m.book_switch(1, at);
    m
}

#[test]
fn o1_mock_t4a_a_switch_takes_the_first_grid_point_at_or_after_its_bound() {
    // RM-37, MR-42 (seq 4): R has period 700 (grid X + 2000 + 700 j); the switch is received so that its bound (receipt sample + window +
    // lead) is one before a grid point, on it and one after it, and once with an explicit `at` between grid points. The boundary is the
    // oracle's; the new waveform has its own period from it. Nothing moves a switch (its row has no `moved_to`); the successor's requested
    // target is the oracle's (#86): the switch's `at` when the receipt pushed it, none for an `at` past the bound. (Mutation row O1-M3b.)
    let s = X + 2_000;
    let g = s + 14_000;
    let cases: [(&str, i64, Option<i64>); 4] = [("g - 1", g - 1, None), ("g", g, None), ("g + 1", g + 1, None), ("at between grid points", g, Some(g + 100))];
    for (case, bound, at) in cases {
        let recv = if at.is_some() { s + 1_000 } else { bound - 12_000 };
        let asked = at.unwrap_or(recv);
        let mut m = switched(700, 400, recv, asked);
        let b = o::switch_boundary(&m.p, s, 700, m.tick(recv), at).unwrap();
        let requested = o::switch_requested_target(&m.p, s, 700, m.tick(recv), asked, false);
        assert_eq!(requested.is_some(), at.is_none(), "{case}: an `at` before the bound is pushed");
        assert_eq!(b, if at.is_none() && bound <= g { g } else { g + 700 }, "{case}");
        // A Stop at X + 31 000 closes the new repeat: its record is the successor's (the Mock's cut is MR-25's point).
        m.go(X + 31_000);
        let t = m.tick(X + 31_000);
        m.stop();
        m.go(X + 60_000);
        let cut = o::mock_cut(&m.p, t, &[o::MBurst { start: b, len: None, started: true, before_stop: true }], None);
        let rows = m.switch_rows();
        assert_eq!(rows.len(), 1, "{case}");
        assert_eq!((rows[0]["outcome"].as_str(), rows[0]["boundary"].as_i64(), rows[0].get("moved_to")), (Some("applied"), Some(b), None), "{case}: {rows:?}");
        assert!(m.all_refusals() == 0 && m.time_errors().is_empty(), "{case}: {:?}", m.events);
        let slots = o::Slots { segs: vec![o::Seg::repeating(8, s, 700, b), o::Seg::repeating(9, b, 400, cut)], cut: None };
        m.check(&slots, s - 10, cut + 100);
        assert_eq!(m.pairs(), [(s, (b - s) as u64), (b, (cut - b) as u64)], "{case}: the old repeat's record ends at the boundary");
        m.assert_switch_ids(0);
        assert_eq!(m.full_rec(1), (b, (cut - b) as u64, ((cut - b) / 400) as u32, requested, false, "stop"), "{case}: the successor's record");
    }
}

#[test]
fn o1_mock_t4c_a_stop_before_the_boundary_cancels_the_switch() {
    // RM-37 (seq 4): R from X + 2 000, a switch received at X + 4 000 (boundary X + 16 000... grid step 1 000), a Stop whose bound is before it:
    // cancelled, the Mock cuts R at its exact MR-25 point and no new sample is radiated.
    let s = X + 2_000;
    let mut m = switched(1_000, 1_000, s + 2_000, s + 2_000);
    let b = o::switch_boundary(&m.p, s, 1_000, m.tick(s + 2_000), Some(s + 2_000)).unwrap();
    assert_eq!(b, s + 14_000);
    m.go(s + 3_000);
    let t = m.tick(s + 3_000);
    let (_, bound) = m.p.stop_bound(t);
    assert!(bound <= b);
    m.stop();
    m.go(X + 60_000);
    let cut = o::mock_cut(&m.p, t, &[o::MBurst { start: s, len: None, started: true, before_stop: true }, o::MBurst { start: b, len: None, started: false, before_stop: true }], None);
    assert_eq!(m.switch_rows()[0]["outcome"], o::switch_outcome_after_mock_stop(b, cut).as_str());
    m.assert_switch_ids(0);
    assert!(!m.heard(9, s, X + 50_000));
    m.check(&o::Slots { segs: vec![o::Seg::repeating(8, s, 1_000, cut)], cut: None }, s - 10, X + 40_000);
    assert_eq!(m.pairs(), [(s, (cut - s) as u64)]);
}

/// A switch received at s + 2 000 (boundary s + 14 000), then a Stop whose window edge is `b + after`: `(m, s, b, cut)`.
/// The successor's requested target (#86): the switch asked for `s + 2 000` and the receipt pushed it to `s + 14 000`.
fn pushed(m: &M, s: i64) -> Option<i64> {
    let requested = o::switch_requested_target(&m.p, s, 1_000, m.tick(s + 2_000), s + 2_000, false);
    assert_eq!(requested, Some(s + 2_000));
    requested
}

fn stopped_after_switch(after: i64) -> (M, i64, i64, i64) {
    let s = X + 2_000;
    let mut m = switched(1_000, 1_000, s + 2_000, s + 2_000);
    let b = s + 14_000;
    let t_sample = b + after - 10_000;
    m.go(t_sample);
    let t = m.tick(t_sample);
    let (_, bound) = m.p.stop_bound(t);
    assert!(b < bound, "the case needs b < B");
    m.stop();
    m.go(X + 60_000);
    let bursts = [o::MBurst { start: s, len: None, started: true, before_stop: true }, o::MBurst { start: b, len: None, started: false, before_stop: true }];
    let cut = o::mock_cut(&m.p, t, &bursts, None);
    (m, s, b, cut)
}

#[test]
fn c2_mr_25_a_stop_below_b_settles_the_row_at_the_stop() {
    // RM-37, MR-42: the Mock's payload cut is known at the Stop, and no loss can follow it, so the row is settled there and does not wait
    // for device time to reach `b` (as UHD's does: `c2_s3_…`): a Stop whose window edge is `b + 600` hands the new waveform's sample at `b`.
    let (s, recv) = (X + 2_000, X + 4_000);
    let mut m = switched(1_000, 1_000, recv, recv);
    let b = o::switch_boundary(&m.p, s, 1_000, m.tick(recv), Some(recv)).unwrap();
    let t_sample = b + 600 - 10_000;
    m.go(t_sample);
    let t = m.tick(t_sample);
    let (_, bound) = m.p.stop_bound(t);
    m.stop();
    let bursts = [o::MBurst { start: s, len: None, started: true, before_stop: true }, o::MBurst { start: b, len: None, started: false, before_stop: true }];
    let cut = o::mock_cut(&m.p, t, &bursts, None);
    assert!(t_sample < b && b < cut && b < bound, "the Stop is below b and hands it: b {b}, B {bound}, cut {cut}");
    assert_eq!(m.switch_rows()[0]["outcome"], "applied", "b {b}, B {bound}, cut {cut}");
}

#[test]
fn o1_mock_t4d_a_stop_with_b_before_its_bound_leaves_the_row_applied() {
    // RM-37, MR-25, MR-42 (seq 4): the Mock installed the switch at receipt, so with b < B it is applied and its new repeat is cut at MR-25's
    // point: the window edge is b + 600 and the new repeat's first 32 samples fit, so [b, b + 600) of it radiates, and the row says so (C2:
    // applied iff b is below the payload cut).
    let (mut m, s, b, cut) = stopped_after_switch(600);
    assert_eq!(cut, b + 600);
    m.check(&o::Slots { segs: vec![o::Seg::repeating(8, s, 1_000, b), o::Seg::repeating(9, b, 1_000, cut)], cut: None }, s - 10, cut + 100);
    assert_eq!(m.switch_rows()[0]["outcome"], o::switch_outcome_after_mock_stop(b, cut).as_str());
    assert_eq!(m.switch_rows()[0]["outcome"], "applied");
    m.assert_switch_ids(0);
    assert_eq!(m.pairs(), [(s, (b - s) as u64), (b, (cut - b) as u64)]);
    assert_eq!(m.full_rec(1), (b, (cut - b) as u64, ((cut - b) / 1_000) as u32, pushed(&m, s), false, "stop"), "the successor's record");
}

#[test]
fn o1_mock_t4e_a_stop_after_the_new_waveform_sounded_leaves_it_applied() {
    // RM-37 (seq 4): as T4d with the window edge at b + 3 000: 3 000 samples of the new waveform radiate; the row is applied.
    let (mut m, s, b, cut) = stopped_after_switch(3_000);
    assert_eq!(cut, b + 3_000);
    m.check(&o::Slots { segs: vec![o::Seg::repeating(8, s, 1_000, b), o::Seg::repeating(9, b, 1_000, cut)], cut: None }, s - 10, cut + 100);
    assert_eq!(m.switch_rows()[0]["outcome"], o::switch_outcome_after_mock_stop(b, cut).as_str());
    m.assert_switch_ids(0);
    assert_eq!(m.full_rec(0), (s, (b - s) as u64, ((b - s) / 1_000) as u32, None, false, "eob"), "the predecessor's record");
    assert_eq!(m.full_rec(1), (b, (cut - b) as u64, ((cut - b) / 1_000) as u32, pushed(&m, s), false, "stop"), "the successor's record");
}

#[test]
fn c2_mr_25_a_stop_settles_a_switch_by_the_payload_cut() {
    // RM-37, MR-25, MR-42 (note 49 stage C2, KL-42): a Stop settles a switch `applied` only when `b` is below the payload cut, the first sample
    // the Mock does not emit; `B` at or before `b` cancels it, and so does a Stop whose bound is past `b` but whose cut is not (a switch the
    // window did not hand: the row said `applied` for a sample never radiated). The row follows the oracle, and agrees with what radiated:
    // the new waveform exists exactly when it is applied.
    for (profile, afters) in [("x3xx", vec![-10, -3, -2, 0, 31, 32, 600]), ("ideal", vec![-5, -3, -2, -1, 1])] {
        for after in afters {
            let (s, recv) = if profile == "x3xx" { (X + 2_000, X + 4_000) } else { (2_000, 3_500) };
            let mut m = if profile == "x3xx" { switched(1_000, 1_000, recv, recv) } else { ideal_switched(1_000, 1_000, recv, recv) };
            let b = o::switch_boundary(&m.p, s, 1_000, m.tick(recv), Some(recv)).unwrap();
            // On x3xx the Stop's window edge is b + after; on ideal (no window) its sample is.
            let t_sample = if profile == "x3xx" { b + after - 10_000 } else { b + after };
            m.go(t_sample);
            let t = m.tick(t_sample);
            let (_, bound) = m.p.stop_bound(t);
            m.stop();
            m.go(b + 60_000);
            let bursts = [o::MBurst { start: s, len: None, started: true, before_stop: true }, o::MBurst { start: b, len: None, started: false, before_stop: true }];
            let cut = o::mock_cut(&m.p, t, &bursts, None);
            let expected = if bound <= b { o::Outcome::Cancelled } else { o::switch_outcome_after_mock_stop(b, cut) };
            let case = format!("{profile}, after {after}: b {b}, B {bound}, cut {cut}");
            assert_eq!(m.switch_rows()[0]["outcome"], expected.as_str(), "{case}");
            let successor = m.pairs().iter().any(|(start, _)| *start == b);
            assert_eq!(successor, expected == o::Outcome::Applied, "{case}: the new waveform radiates exactly when the row says applied: {:?}", m.pairs());
        }
    }
}

#[test]
fn mr_42_a_timed_switch_pushed_past_its_boundary_records_its_at() {
    // #86, MR-42 (M3; `e2a_86_a_timed_switch_pushed_past_its_boundary_records_its_at` is the UHD twin): R (period 1 000 from s) and a
    // switch received at s + 2 000, whose bound is s + 14 000. A timed switch for s + 2 000 is pushed to s + 14 000 and its new repeat's
    // record carries s + 2 000 as its requested target; one for s + 16 000 is on its own grid point and carries none; an untimed one none.
    let s = X + 2_000;
    let recv = s + 2_000;
    for (case, at, untimed) in [("pushed", s + 2_000, false), ("on the rule", s + 16_000, false), ("untimed", s + 2_000, true)] {
        let mut m = x3xx(&[(8, 1_000), (9, 1_000)]);
        m.book(0, s, true, LatePolicy::DropAndFlag);
        m.go(recv);
        let action = m.action(1, at, true, LatePolicy::SendAsapAndFlag, true);
        if untimed {
            m.radio.actions.1.store(true, Ordering::SeqCst);
        }
        m.push(action);
        let b = o::switch_boundary(&m.p, s, 1_000, m.tick(recv), Some(at)).unwrap();
        m.go(X + 31_000);
        m.stop();
        m.go(X + 60_000);
        let rows = m.switch_rows();
        assert_eq!((rows.len(), rows[0]["outcome"].as_str(), rows[0]["boundary"].as_i64(), rows[0].get("moved_to")), (1, Some("applied"), Some(b), None), "{case}: {rows:?}");
        let want = o::switch_requested_target(&m.p, s, 1_000, m.tick(recv), at, untimed);
        assert_eq!(want.is_some(), case == "pushed", "{case}");
        let successor = m.full_rec(1);
        assert_eq!((successor.0, successor.3, successor.4), (b, want, false), "{case}: the successor's record");
    }
}

// ------------------------------------------------------------------------------------------------------------ T5

#[test]
fn o1_mock_t5_a_late_send_asap_and_an_untimed_booking_are_placed_exactly() {
    // RM-14, RM-15 (seq 5): the placement is the oracle's, compared by equality with where the record says the booking started.
    let s = X + 10_000;
    let a = region_of(s, 3_000);
    // (a) late SendAsap inside A's region, whose front dominates the ordinary bound.
    let mut m = x3xx(&[(10, 3_000), (11, 200)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(s);
    m.book(1, X + 11_500, false, LatePolicy::SendAsapAndFlag);
    let want = o::place(&m.p, &[a], m.tick(s), Some(X + 11_500), Some(200));
    assert_eq!(want, X + 13_004);
    m.go(X + 40_000);
    assert_eq!(m.rec_of(200), (want, Some(X + 11_500), true));
    assert_eq!(m.time_errors(), ["send_asap"]);
    m.check(&o::Slots { segs: vec![o::Seg::once(10, s, 3_000), o::Seg::once(11, want, 200)], cut: None }, s - 10, want + 300);
    // (b) the same where the lead dominates: A (300 samples, front s + 304) is long done at 12 ms.
    let mut m = x3xx(&[(10, 300), (12, 200)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(X + 12_000);
    m.book(1, X + 10_100, false, LatePolicy::SendAsapAndFlag);
    let want = o::place(&m.p, &[region_of(s, 300)], m.tick(X + 12_000), Some(X + 10_100), Some(200));
    assert_eq!(want, X + 14_000);
    m.go(X + 40_000);
    assert_eq!(m.rec_of(200), (want, Some(X + 10_100), true));
    m.check(&o::Slots { segs: vec![o::Seg::once(10, s, 300), o::Seg::once(12, want, 200)], cut: None }, s - 10, want + 300);
    // (c) a late DropAndFlag booking in A's region is dropped (lateness is decided before the footprint), not refused.
    let mut m = x3xx(&[(10, 3_000), (13, 200)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(s);
    m.book(1, X + 11_500, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&m.p, &[a], m.tick(s), X + 11_500, Some(200)), o::Verdict::Late);
    m.go(X + 40_000);
    assert_eq!(m.time_errors(), ["drop"]);
    assert_eq!(m.all_refusals(), 0);
    assert!(!m.heard(13, s, X + 35_000));
    // (d) an untimed booking while A plays: the first free sample, A's front, no TIME_ERROR, no lateness.
    let mut m = x3xx(&[(10, 3_000), (14, 200)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(s);
    m.book_untimed(1, X + 10_500);
    let want = o::place(&m.p, &[a], m.tick(s), Some(X + 10_500), Some(200));
    assert_eq!(want, X + 13_004);
    m.go(X + 40_000);
    assert_eq!(m.rec_of(200), (want, None, false));
    assert!(m.time_errors().is_empty());
    // (e) an untimed booking after A ended: ordinary dominates.
    let mut m = x3xx(&[(10, 300), (15, 200)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.go(X + 12_000);
    m.book_untimed(1, X + 12_000);
    let want = o::place(&m.p, &[region_of(s, 300)], m.tick(X + 12_000), Some(X + 12_000), Some(200));
    assert_eq!(want, X + 14_000);
    m.go(X + 40_000);
    assert_eq!(m.rec_of(200), (want, None, false));
}

// ------------------------------------------------------------------------------------------------------------ T6

/// A short A (ramp 100 at X + 10 000, long over by the Stop at X + 20 000) and the bookings `extra` made before it; the Stop is received at
/// X + 20 000.
fn before_stop(waves: &[(u8, usize)], extra: &[(usize, i64)]) -> (M, i64) {
    let mut m = x3xx(waves);
    m.book(0, X + 10_000, false, LatePolicy::DropAndFlag);
    for (wave, start) in extra {
        m.book(*wave, *start, false, LatePolicy::DropAndFlag);
    }
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    (m, t)
}

#[test]
fn o1_mock_t6a_a_cancellation_that_starts_at_or_after_the_bound_leaves_no_region() {
    // RM-16 as amended (a Stop that cancels a booking starting before B leaves [t, B)) (seq 6): H starts at X + 50 000, after B = X + 30 005,
    // so it is cancelled (a `rejected` note, not an event) and leaves no region: D, received after the Stop with start X + 26 000, is accepted.
    let (mut m, t) = before_stop(&[(1, 100), (3, 300), (4, 200)], &[(1, X + 50_000)]);
    assert_eq!(m.p.stop_bound(t), (X + 20_000, X + 30_005));
    assert_eq!(o::stop_region(&m.p, t, &[X + 50_000]), None);
    m.book(2, X + 26_000, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&m.p, &[], t, X + 26_000, Some(200)), o::Verdict::Accept);
    m.go(X + 60_000);
    assert_eq!(m.cancellations(), 1, "H is cancelled by a rejected note");
    assert_eq!(m.all_refusals(), 0, "a cancellation by Stop is no COMMAND_REJECTED event, and D is accepted");
    assert_eq!(m.recs().iter().map(|r| (r.0, r.1, end_of(r.4))).collect::<Vec<_>>(), [(X + 10_000, 100, "eob"), (X + 26_000, 200, "eob")]);
    m.check(&o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100), o::Seg::once(4, X + 26_000, 200)], cut: None }, X + 9_990, X + 50_400);
}

#[test]
fn o1_mock_t6b_a_cancellation_that_starts_before_the_bound_leaves_the_region() {
    // RM-16 (seq 6): H2 is booked ahead and starts in [window_end, B) (X + 30 001) or at window_end - 16 (X + 29 984, the case where the UHD
    // drops it with no note, issue #85: the Mock records the cancellation, as the spec says). Either way it is unhanded at the Stop, is
    // cancelled (a note), starts in [t, B) and so leaves [X + 20 000, X + 30 005); every later booking is judged by it.
    for h2 in [X + 30_001, X + 29_984] {
        for case in ["drop inside", "send_asap inside", "untimed", "drop at B"] {
            let (mut m, t) = before_stop(&[(1, 100), (3, 300), (4, 200)], &[(1, h2)]);
            let region = o::stop_region(&m.p, t, &[h2]).expect("H2 starts in [t_k, B)");
            assert_eq!((region.start, region.front), (X + 20_000, X + 30_005));
            let want_start = match case {
                "drop inside" => {
                    m.book(2, X + 29_000, false, LatePolicy::DropAndFlag);
                    assert_eq!(o::admit(&m.p, &[region], t, X + 29_000, Some(200)), o::Verdict::Refuse);
                    None
                }
                "send_asap inside" => {
                    m.book(2, X + 29_000, false, LatePolicy::SendAsapAndFlag);
                    Some(o::place(&m.p, &[region], t, Some(X + 29_000), Some(200)))
                }
                "untimed" => {
                    m.book_untimed(2, X + 29_000);
                    Some(o::place(&m.p, &[region], t, Some(X + 29_000), Some(200)))
                }
                _ => {
                    m.book(2, X + 30_005, false, LatePolicy::DropAndFlag);
                    assert_eq!(o::admit(&m.p, &[region], t, X + 30_005, Some(200)), o::Verdict::Accept);
                    Some(X + 30_005)
                }
            };
            m.go(X + 60_000);
            let refusals = m.refusals();
            assert_eq!(m.cancellations(), 1, "{case} (H2 at {h2}): H2 is cancelled with one note");
            assert!(m.recs().iter().all(|r| r.1 != 300), "{case}: H2 never played: {:?}", m.recs());
            match want_start {
                None => assert_eq!(refusals.len(), 1, "{case}: {refusals:?}"),
                Some(start) => {
                    assert!(refusals.is_empty(), "{case}: {refusals:?}");
                    assert_eq!(m.rec_of(200), (start, (case == "send_asap inside").then_some(X + 29_000), case == "send_asap inside"), "{case}");
                    assert_eq!(start, X + 30_005, "{case}: the first sample past the region");
                    m.check(&o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100), o::Seg::once(4, start, 200)], cut: None }, X + 9_990, start + 300);
                }
            }
            assert_eq!(m.time_errors(), if case == "send_asap inside" { vec!["send_asap"] } else { vec![] }, "{case}");
        }
    }
}

#[test]
fn o1_mock_t6c_a_booking_after_a_stop_on_a_playing_burst_survives() {
    // RM-16, MR-25 (seq 6): A is 12 000 samples from X + 10 000 (longer than the window). The Stop at X + 11 000 cuts it at MR-25's point, the
    // window's first sample X + 21 000; the held H3 (start X + 40 000) is cancelled; E, received after the Stop with start X + 30 000, plays.
    let s = X + 10_000;
    let mut m = x3xx(&[(5, 12_000), (6, 200), (3, 300)]);
    m.book(0, s, false, LatePolicy::DropAndFlag);
    m.book(2, X + 40_000, false, LatePolicy::DropAndFlag);
    m.go(X + 11_000);
    let t = m.tick(X + 11_000);
    m.stop();
    let (_, bound) = m.p.stop_bound(t);
    let cut_region = o::Region { start: s, end: bound, front: bound, len: bound - s };
    m.book(1, X + 30_000, false, LatePolicy::DropAndFlag);
    assert_eq!(o::admit(&m.p, &[cut_region], t, X + 30_000, Some(200)), o::Verdict::Accept);
    m.go(X + 60_000);
    let cut =
        o::mock_cut(&m.p, t, &[o::MBurst { start: s, len: Some(12_000), started: true, before_stop: true }, o::MBurst { start: X + 40_000, len: Some(300), started: false, before_stop: true }], None);
    assert_eq!(cut, X + 21_000);
    m.check(&o::Slots { segs: vec![o::Seg { end: cut, ..o::Seg::once(5, s, 12_000) }, o::Seg::once(6, X + 30_000, 200)], cut: None }, s - 10, X + 30_300);
    assert_eq!(m.cancellations(), 1, "H3");
    assert_eq!(m.all_refusals(), 0);
    let recs = m.recs();
    assert_eq!((recs[0].0, recs[0].1, end_of(recs[0].4)), (s, (cut - s) as u64, "stop"));
    assert_eq!((recs[1].0, recs[1].1), (X + 30_000, 200));
}

// ------------------------------------------------------------------------------------------------------------ T9

#[test]
fn o1_mock_t9_the_stop_cut_follows_mr_25_in_full() {
    // MR-25 (seq 3, 6): the cut is the later of the Stop sample, the payload handed within the window (a burst not yet started counts only if
    // its first send, 32 samples or the whole burst if shorter, fits before the window's first sample) and a cut retained from an earlier Stop.
    // The Stop is at X + 20 000: t_k = X + 20 000, window_end = X + 30 000. (Mutation row O1-M4.)
    let (tk, we) = (X + 20_000, X + 30_000);
    // (label, start, len (None: repeat), started)
    let rows: [(&str, i64, Option<i64>, bool); 5] = [
        ("an open repeat", X + 15_000, None, true),
        ("a repeat whose first 32 samples fit", we - 32, None, false),
        ("a repeat whose first 32 do not", we - 31, None, false),
        ("a finite burst of 20 that fits whole", we - 20, Some(20), false),
        ("a finite burst of 20 that does not", we - 19, Some(20), false),
    ];
    for (label, start, len, started) in rows {
        let mut m = x3xx(&[(1, len.unwrap_or(1_000) as usize)]);
        m.book(0, start, len.is_none(), LatePolicy::DropAndFlag);
        m.go(tk);
        let t = m.tick(tk);
        m.stop();
        m.go(X + 60_000);
        let cut = o::mock_cut(&m.p, t, &[o::MBurst { start, len, started, before_stop: true }], None);
        let handed = cut > start;
        assert_eq!(handed, label.contains("open") || label.contains("fit") && !label.contains("not"), "{label}");
        let mut slots = o::Slots::default();
        if handed {
            slots.segs.push(match len {
                None => o::Seg::repeating(1, start, 1_000, cut),
                Some(l) => o::Seg { end: cut, ..o::Seg::once(1, start, l) },
            });
        }
        m.check(&slots, start - 10, cut.max(start) + 100);
        let want: Vec<(i64, u64)> = if handed { vec![(start, (len.map_or(cut, |l| (start + l).min(cut)) - start) as u64)] } else { vec![] };
        assert_eq!(m.pairs(), want, "{label}");
        assert_eq!(m.cancellations(), usize::from(!handed), "{label}: an unhanded burst is cancelled with a note");
    }
    // A cut retained from an earlier Stop is never retracted: the second Stop, whose instant is before the first Stop's cut, leaves R radiating
    // to the first cut.
    let s = X + 15_000;
    let mut m = x3xx(&[(7, 1_000)]);
    m.book(0, s, true, LatePolicy::DropAndFlag);
    m.go(tk);
    let t1 = m.tick(tk);
    m.stop();
    m.go(tk + 2_000);
    let t2 = m.tick(tk + 2_000);
    m.stop();
    m.go(X + 60_000);
    let cut1 = o::mock_cut(&m.p, t1, &[o::MBurst { start: s, len: None, started: true, before_stop: true }], None);
    let cut2 = o::mock_cut(&m.p, t2, &[], Some(cut1));
    assert_eq!((cut1, cut2), (we, we), "the retained cut stands");
    m.check(&o::Slots { segs: vec![o::Seg::repeating(7, s, 1_000, cut2)], cut: None }, s - 10, cut2 + 100);
    assert_eq!(m.pairs(), [(s, (cut2 - s) as u64)]);
}

#[test]
fn o1_mock_t9f_with_a_zero_window_the_stop_cuts_at_its_own_sample() {
    // RM-15 ("a zero window preserves the ordinary behavior"), MR-25 (seq 3): on `ideal` (window 0) a Stop at sample 5 000 cuts A (100 samples
    // from 4 990) at the Stop sample: 10 samples radiate, the record holds 10.
    let mut m = ideal(&[(1, 100)]);
    m.book(0, 4_990, false, LatePolicy::DropAndFlag);
    m.go(5_000);
    let t = m.tick(5_000);
    m.stop();
    m.go(9_000);
    let cut = o::mock_cut(&m.p, t, &[o::MBurst { start: 4_990, len: Some(100), started: true, before_stop: true }], None);
    assert_eq!(cut, 5_000);
    m.check(&o::Slots { segs: vec![o::Seg { end: cut, ..o::Seg::once(1, 4_990, 100) }], cut: None }, 4_980, 5_100);
    assert_eq!(m.pairs(), [(4_990, 10)]);
}

#[test]
fn e1_mock_q10_a_booking_a_stop_and_a_booking_in_one_step() {
    // RM-15, RM-16, MR-25 (note 49 E1, Q10): A booked, a Stop, then R booked, all queued and received in one step. The list is read in receipt
    // order, so the Stop cancels A (one note, no event) and not R, which plays from its start.
    let mut m = ideal(&[(1, 100), (2, 100)]);
    m.go(1_000);
    let (a, r) = (m.action(0, 10_000, false, LatePolicy::DropAndFlag, false), m.action(1, 20_000, false, LatePolicy::DropAndFlag, false));
    m.radio.push(a);
    m.radio.push(Action::Stop { target: Some(tgt(&format!("{}/tx/s0", m.radio.id))) });
    m.radio.push(r);
    m.go(1_000);
    m.go(30_000);
    assert_eq!(m.all_refusals(), 0, "no refusal event");
    assert_eq!(m.cancellations(), 1, "A is cancelled by the Stop with one note");
    assert_eq!(m.pairs(), [(20_000, 100)], "R plays from its start");
    m.check(&o::Slots { segs: vec![o::Seg::once(2, 20_000, 100)], cut: None }, 9_990, 20_100);
}

// ------------------------------------------------------------------------------------------------------------ E3a-1 (design note 49 E3a)

/// The Mock's column of table Q: the payload samples the Kernel record of a booking holds after a Stop at `X + 20 000`, `None` when the
/// booking is cancelled (MR-25: a note, no record, no sample). Written from MR-25 and read against the oracle's `mock_cut`, which computes
/// the same cut independently; the UHD column of the same rows is `e3_u1_…` (`provider/tx/tests/o1.rs`).
fn mock_column(label: &str) -> Option<i64> {
    match label {
        "finite 1 at we-1" => Some(1),
        "finite 20 at we-20" => Some(20),
        "finite 32 at we-32" | "finite 33 at we-32" | "repeat at we-32" => Some(32),
        "finite 100 inside the window" => Some(100),
        "open repeat at t_k-5000" | "long finite at t_k-5000" => Some(15_000),
        "open repeat at t_k-4010" => Some(14_010),
        "finite 1 at we" | "finite 20 at we-19" | "finite 32 at we-31" | "finite 33 at we-31" | "repeat at we-31" => None,
        other => panic!("table Q has no Mock column for {other}"),
    }
}

#[test]
fn e3_mc1_the_stop_cut_by_burst_shape() {
    // MR-25 (seq 3, 6): the Stop at X + 20 000 cuts at the later of its own sample and the payload handed within the window (a burst not yet
    // started counts only when its first send, 32 samples or the whole burst if shorter, fits before the window's first sample `we`; a
    // repeat's first send is 32, an open one's is one sample). Every row of `o::STOP_SHAPES` is read against the oracle's `mock_cut`, the
    // literal column above, the radiated samples and the Kernel records.
    let (tk, we) = (X + 20_000, X + 30_000);
    for shape in o::STOP_SHAPES {
        let (len, repeat) = (shape.len.unwrap_or(1_000), shape.len.is_none());
        let start = if shape.from_window_edge { we } else { tk } + shape.offset;
        let mut m = x3xx(&[(1, 100), (3, len as usize)]);
        m.book(0, X + 10_000, false, LatePolicy::DropAndFlag);
        m.book(1, start, repeat, LatePolicy::DropAndFlag);
        m.go(X + 20_000);
        let t = m.tick(X + 20_000);
        m.stop();
        m.go(X + 60_000);
        let bursts = [o::MBurst { start: X + 10_000, len: Some(100), started: true, before_stop: true }, o::MBurst { start, len: shape.len, started: shape.started, before_stop: true }];
        let cut = o::mock_cut(&m.p, t, &bursts, None);
        let label = shape.label;
        let recorded = mock_column(label);
        let from_oracle = (cut > start).then(|| shape.len.map_or(cut, |l| (start + l).min(cut)) - start);
        assert_eq!(from_oracle, recorded, "{label}: the oracle and the literal column agree (cut {cut}, we {we})");
        let mut slots = o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100)], cut: None };
        let mut pairs = vec![(X + 10_000, 100)];
        if let Some(samples) = recorded {
            slots.segs.push(match shape.len {
                None => o::Seg::repeating(3, start, 1_000, cut),
                Some(l) => o::Seg { end: (start + l).min(cut), ..o::Seg::once(3, start, l) },
            });
            pairs.push((start, samples as u64));
        }
        m.check(&slots, X + 9_990, cut.max(start) + 100);
        assert_eq!(m.pairs(), pairs, "{label}");
        assert_eq!(m.cancellations(), usize::from(recorded.is_none()), "{label}: an unhanded booking is cancelled with a note");
        assert_eq!(m.all_refusals(), 0, "{label}");
    }
}

#[test]
fn e3_mc1b_a_booking_joined_to_a_handed_one_at_a_stop() {
    // Table Q row 10 (F3b): A (40) ends at `we - 50` and B (100) joins it at its end. The Mock judges B as a fresh first send of 32, which fits,
    // so the Stop hands B to `we`: 50 samples of it, its record ends at the Stop. UHD hands less (B continues A's device burst and needs all of
    // its 100): `e3_u1b_…`.
    let we = X + 30_000;
    let [(a_at, a_len), (b_at, b_len)] = o::JOINED_AT_A_STOP;
    let (a_start, b_start) = (we + a_at, we + b_at);
    assert_eq!(a_start + a_len, b_start);
    let mut m = x3xx(&[(1, 100), (2, a_len as usize), (3, b_len as usize)]);
    m.book(0, X + 10_000, false, LatePolicy::DropAndFlag);
    m.book(1, a_start, false, LatePolicy::DropAndFlag);
    m.book(2, b_start, false, LatePolicy::DropAndFlag);
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    m.go(X + 60_000);
    let bursts = [
        o::MBurst { start: X + 10_000, len: Some(100), started: true, before_stop: true },
        o::MBurst { start: a_start, len: Some(a_len), started: false, before_stop: true },
        o::MBurst { start: b_start, len: Some(b_len), started: false, before_stop: true },
    ];
    let cut = o::mock_cut(&m.p, t, &bursts, None);
    assert_eq!(cut, we, "B's first send of 32 fits, so the cut is the window's edge");
    let slots = o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100), o::Seg::once(2, a_start, a_len), o::Seg { end: cut, ..o::Seg::once(3, b_start, b_len) }], cut: None };
    m.check(&slots, X + 9_990, we + 100);
    assert_eq!(m.pairs(), [(X + 10_000, 100), (a_start, 40), (b_start, 50)]);
    assert_eq!((m.recs()[2].4, m.cancellations(), m.all_refusals()), (BurstEnd::Stop, 0, 0));
}

#[test]
fn e3_mc1c_a_repeat_shorter_than_32_samples_has_a_first_send_of_32() {
    // Table Q row 9 (F3a) as far as the Mock reaches it: R (20 samples a period) is held at `we - 20`. A repeat's first send is 32 samples
    // whatever its period, so it does not fit before `we` and is not handed: the cut is the Stop sample, R is cancelled and no sample radiates
    // (a finite burst of 20 at the same place is handed whole, row `finite 20 at we-20`). The row the plan names for this, a switch 20 samples
    // after R's start, cannot be booked: the Mock refuses a switch while R is not playing (MR-16), as UHD does (KL-41), so no run reaches a
    // repeat cut by a held start before 32 samples on either Provider; `e3_c4_…` pins the UHD owner's cut on a direct give.
    let (tk, we) = (X + 20_000, X + 30_000);
    let r_start = we - 20;
    let mut m = x3xx(&[(1, 100), (2, 20), (3, 500)]);
    m.book(0, X + 10_000, false, LatePolicy::DropAndFlag);
    m.book(1, r_start, true, LatePolicy::DropAndFlag);
    m.go(X + 17_990);
    m.book_switch(2, X + 17_990);
    assert_eq!(m.refusals(), ["MR-16: a repeat switch needs a playing repeat and no pending switch"], "the switch on the held repeat is refused at receipt");
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    m.go(X + 60_000);
    let bursts = [o::MBurst { start: X + 10_000, len: Some(100), started: true, before_stop: true }, o::MBurst { start: r_start, len: None, started: false, before_stop: true }];
    assert_eq!(o::mock_cut(&m.p, t, &bursts, None), tk, "nothing is handed: the cut is the Stop sample");
    m.check(&o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100)], cut: None }, X + 9_990, we + 600);
    assert_eq!(m.pairs(), [(X + 10_000, 100)], "R has no record");
    assert_eq!(m.cancellations(), 1, "R is cancelled with one note");
    assert!(m.switch_rows().is_empty(), "a refused switch writes no row");
}

#[test]
fn e3_mc1d_a_booking_after_a_stop_is_not_counted_in_its_cut() {
    // MR-25: only a burst received before the Stop contributes. An open repeat is cut at `we`; E, received after the Stop with its start at
    // `we + 100`, is accepted and plays whole, and a second Stop 2 000 samples later, whose own sample is before the first cut, leaves it as it was (a
    // retained cut is never retracted).
    let (tk, we) = (X + 20_000, X + 30_000);
    let s = X + 15_000;
    let mut m = x3xx(&[(7, 1_000), (4, 100)]);
    m.book(0, s, true, LatePolicy::DropAndFlag);
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    m.book(1, we + 100, false, LatePolicy::DropAndFlag);
    m.go(tk + 2_000);
    let t2 = m.tick(tk + 2_000);
    m.stop();
    m.go(X + 60_000);
    let cut1 = o::mock_cut(&m.p, t, &[o::MBurst { start: s, len: None, started: true, before_stop: true }], None);
    let cut2 = o::mock_cut(&m.p, t2, &[], Some(cut1));
    assert_eq!((cut1, cut2), (we, we), "the second Stop's own sample is before the retained cut, which stands");
    m.check(&o::Slots { segs: vec![o::Seg::repeating(7, s, 1_000, cut1), o::Seg::once(4, we + 100, 100)], cut: None }, s - 10, we + 300);
    assert_eq!(m.pairs(), [(s, (cut1 - s) as u64), (we + 100, 100)]);
    assert_eq!((m.cancellations(), m.all_refusals()), (0, 0));
}

#[test]
fn e3_mc2_a_zero_window() {
    // MR-25 on `ideal` (window 0, `we` = the Stop sample): a burst counts only if it starts before the Stop sample and its first send fits
    // before it, so nothing is handed ahead and the cut is the Stop sample, whatever the shape: the open repeat radiates to the Stop sample, a
    // held repeat and a held finite burst are cancelled, and a finite burst booked at the Stop sample itself is open when the Stop is read but
    // has no sample before it, so it is cancelled too.
    let cases: [(&str, i64, Option<i64>, bool); 4] = [
        ("open repeat from 4000", 4_000, None, true),
        ("held repeat at 7000", 7_000, None, false),
        ("held finite 100 at 7000", 7_000, Some(100), false),
        ("finite 100 at the Stop sample", 5_000, Some(100), true),
    ];
    for (label, start, len, started) in cases {
        let mut m = ideal(&[(1, 1_000)]);
        m.book(0, start, len.is_none(), LatePolicy::DropAndFlag);
        m.go(5_000);
        let t = m.tick(5_000);
        m.stop();
        m.go(9_000);
        let cut = o::mock_cut(&m.p, t, &[o::MBurst { start, len, started, before_stop: true }], None);
        assert_eq!(cut, 5_000, "{label}: the cut is the Stop sample");
        let handed = label.starts_with("open");
        let slots = if handed { o::Slots { segs: vec![o::Seg::repeating(1, start, 1_000, cut)], cut: None } } else { o::Slots::default() };
        m.check(&slots, 3_990, 6_000);
        assert_eq!(m.pairs(), if handed { vec![(start, (cut - start) as u64)] } else { vec![] }, "{label}");
        assert_eq!(m.cancellations(), usize::from(!handed), "{label}");
    }
}

#[test]
fn e3_mc3_the_stop_sample_and_the_handed_payload() {
    // MR-25's cut is the later of the Stop sample and the handed payload. (a) Nothing is handed (the booking's start is past the window's edge,
    // and A completed long ago): the cut is the Stop sample, A's record is whole and the booking is cancelled. (b) A short open finite burst
    // whose end is past the Stop sample: its end is the cut, so all 100 samples radiate and its record ends at its own end.
    let (tk, we) = (X + 20_000, X + 30_000);
    let mut m = x3xx(&[(1, 100), (2, 100)]);
    m.book(0, X + 10_000, false, LatePolicy::DropAndFlag);
    m.book(1, we + 500, false, LatePolicy::DropAndFlag);
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    m.go(X + 60_000);
    let a = o::MBurst { start: X + 10_000, len: Some(100), started: true, before_stop: true };
    assert_eq!(o::mock_cut(&m.p, t, &[a, o::MBurst { start: we + 500, len: Some(100), started: false, before_stop: true }], None), tk, "(a) the Stop sample");
    m.check(&o::Slots { segs: vec![o::Seg::once(1, X + 10_000, 100)], cut: None }, X + 9_990, we + 700);
    assert_eq!((m.pairs(), m.cancellations()), (vec![(X + 10_000, 100)], 1));
    let mut m = x3xx(&[(2, 100)]);
    let b_start = tk - 25;
    m.book(0, b_start, false, LatePolicy::DropAndFlag);
    m.go(X + 20_000);
    let t = m.tick(X + 20_000);
    m.stop();
    m.go(X + 60_000);
    let cut = o::mock_cut(&m.p, t, &[o::MBurst { start: b_start, len: Some(100), started: true, before_stop: true }], None);
    assert_eq!(cut, tk + 75, "(b) the burst's own end, past the Stop sample");
    m.check(&o::Slots { segs: vec![o::Seg::once(2, b_start, 100)], cut: None }, b_start - 10, cut + 100);
    assert_eq!(m.recs().iter().map(|r| (r.0, r.1, end_of(r.4))).collect::<Vec<_>>(), [(b_start, 100, "eob")]);
}

/// One step of the Authority that emitted a transmit block: `(root tick, blocks emitted so far, burst records so far, the last record as
/// (target, samples, blocks, end))`.
type BlockRow = (i64, u64, usize, Option<(i64, u64, u32, &'static str)>);

impl M {
    fn tx_blocks(&self) -> u64 {
        self.radio.section("stats")["tx_blocks"].as_u64().expect("the tx_blocks stat")
    }

    fn block_row(&self, tick: i64) -> BlockRow {
        let records = records(&self.radio);
        (tick, self.tx_blocks(), records.len(), records.last().map(|r| (r.target.ticks_in(r.target.domain()).unwrap(), r.samples, r.blocks, end_of(r.end))))
    }

    /// Steps the Mock to each wakeup the Authority schedules up to `until`, one at a time (the receive stream's wakeups too), and returns the
    /// row of every step that emitted a transmit block.
    fn block_rows(&mut self, until: i64) -> Vec<BlockRow> {
        let mut rows = Vec::new();
        while let Some(due) = self.world.auth.next_due() {
            let due = due.ticks_in(ROOT).unwrap();
            if due > until {
                break;
            }
            let before = self.tx_blocks();
            self.go_tick(due);
            if self.tx_blocks() > before {
                rows.push(self.block_row(due));
            }
        }
        rows
    }
}

/// The tick a block ending at sample `end` is due: the last instant of its last sample plus one tick (`v_after(end - 1)`), 1 000 root ticks a
/// sample on the 1 GHz root at 1 MS/s.
fn due_of(end: i64) -> i64 {
    (end - 1) * 1_000 + 1
}

#[test]
fn e3_mc4_blocks_by_shape() {
    // MR-15: a transmit block is emitted when its last sample is past, and it ends at the first of the block length (2 000 samples), the
    // waveform's end or a repeat's wrap, a held burst's start and a cut. One row for each emitting step of the Authority's own wakeups
    // (`ideal`, so no window or lead blocks a booking); the first sample of a block is the previous one's end (the first, the booking's start),
    // its length the difference, its SOB the first and its EOB the one at which a record appears. Typed literals, never read from the Mock.
    assert_eq!(ezsdr_mock_radio::BLOCK_LEN, 2_000);
    // (a) a finite burst of 5 000 from 100: 2 000, 2 000, then the 1 000 that end it (the record appears with the third).
    let mut m = ideal(&[(1, 5_000)]);
    m.book(0, 100, false, LatePolicy::DropAndFlag);
    assert_eq!(
        m.block_rows(8_000_000),
        [(due_of(2_100), 1, 0, None), (due_of(4_100), 2, 0, None), (due_of(5_100), 3, 1, Some((100, 5_000, 3, "eob")))],
        "(a) a finite burst: block ends 2 100, 4 100, 5 100"
    );
    // (b) a repeat of 3 000 from 100: a block stops at each wrap (3 100, 6 100) as well as at the block length, and no record appears.
    let mut m = ideal(&[(2, 3_000)]);
    m.book(0, 100, true, LatePolicy::DropAndFlag);
    assert_eq!(
        m.block_rows(9_000_000),
        [(due_of(2_100), 1, 0, None), (due_of(3_100), 2, 0, None), (due_of(5_100), 3, 0, None), (due_of(6_100), 4, 0, None), (due_of(8_100), 5, 0, None)],
        "(b) a repeat across its wrap: block ends 2 100, 3 100, 5 100, 6 100, 8 100"
    );
    // (c) a repeat of 5 400 from 100 and a switch received at sample 150, whose boundary is the repeat's second start 5 500: the old repeat's last
    // block ends at the boundary with the end of burst (its record), and the successor (period 1 000) starts there with its own blocks.
    let mut m = ideal(&[(3, 5_400), (4, 1_000)]);
    m.book(0, 100, true, LatePolicy::DropAndFlag);
    m.go(150);
    m.book_switch(1, 150);
    assert_eq!(o::switch_boundary(&m.p, 100, 5_400, m.tick(150), Some(150)), Some(5_500));
    assert_eq!(
        m.block_rows(8_000_000),
        [
            (due_of(2_100), 1, 0, None),
            (due_of(4_100), 2, 0, None),
            (due_of(5_500), 3, 1, Some((100, 5_400, 3, "eob"))),
            (due_of(6_500), 4, 1, Some((100, 5_400, 3, "eob"))),
            (due_of(7_500), 5, 1, Some((100, 5_400, 3, "eob"))),
        ],
        "(c) a repeat cut at a switch boundary: block ends 2 100, 4 100, 5 500 (END_OF_BURST), then the successor's 6 500, 7 500"
    );
    // (d) a Stop at sample 3 000 on the repeat of (b): the step that reads it emits the block up to the cut, with no end of burst, and the
    // tracker's stop records the burst (end stop) with what was transmitted.
    let mut m = ideal(&[(2, 3_000)]);
    m.book(0, 100, true, LatePolicy::DropAndFlag);
    let before = m.block_rows(2_999_999);
    assert_eq!(before, [(due_of(2_100), 1, 0, None)], "(d) the block before the Stop");
    m.go_tick(3_000_000);
    m.stop();
    assert_eq!(m.block_row(3_000_000), (3_000_000, 2, 1, Some((100, 2_900, 2, "stop"))), "(d) the Stop's cut block [2 100, 3 000) and the record");
    // (e) the Run's end (`Provider::stop`, orderly) at the same instant cuts the same way.
    let mut m = ideal(&[(2, 3_000)]);
    m.book(0, 100, true, LatePolicy::DropAndFlag);
    m.block_rows(2_999_999);
    m.go_tick(3_000_000);
    m.radio.mock.stop(ezsdr_kernel::module_api::StopMode::Orderly).unwrap();
    assert_eq!(m.block_row(3_000_000), (3_000_000, 2, 1, Some((100, 2_900, 2, "stop"))), "(e) the Run's end");
    assert!(m.all_refusals() == 0 && m.radio.section("rejected").as_array().unwrap().is_empty(), "a mis-flagged block would be refused by the tracker or the device model (MR-24)");
}

/// One case of `e3_mc5_…`: its label, whether it repeats, the waveform's length, the receipt sample of a switch (if any), the booking's start, and
/// the block ends whose deadlines are asserted.
type Wake = (&'static str, bool, i64, Option<i64>, i64, &'static [i64]);

#[test]
fn e3_mc5_the_wake_follows_the_block() {
    // MR-14: the Mock schedules its next wakeup at the instant its next transmit block is complete, `v_after(stop - 1)`: the last instant of the
    // block's last sample plus one tick, where `stop` is the first of the block length, the waveform's end or a repeat's wrap, a held start and a
    // cut. The Authority is asked, not the Mock: `next_due` is the scheduled deadline, the step one tick before it emits nothing and the step
    // at it emits the block. The receive stream's wakeups sit at `v_after(1 999 + 2 000 j)`: the chosen block ends are never one of them, so the
    // transmit wake is the minimum at each position (precondition: `next_due` equals the transmit deadline once every earlier wake has run).
    // The wake for a held start (`v_floor` of its first sample) is a later tick than the block deadline before it, so it cannot stand in for
    // that. A held start before the block's wrap or waveform end is not a reachable state (the record refuses a booking inside an open burst's
    // span, and a switch's boundary is a wrap), so the cut at a held start in the wake equals the waveform end whenever it matters (INFERRED
    // from the booking rules, not run): no test here can tell a wake that ignores the held start from one that reads it.
    assert!(due_of(5_500) < 5_500 * 1_000, "the wake for the successor's start (the sample's own instant) is a later tick than the old repeat's last block");
    let cases: [Wake; 3] = [
        ("a finite burst of 5 000 from 100 (block length, waveform end)", false, 5_000, None, 100, &[2_100, 4_100, 5_100]),
        ("a repeat of 3 000 from 100 (block length, a wrap)", true, 3_000, None, 100, &[2_100, 3_100, 5_100, 6_100]),
        ("a repeat of 5 400 with a switch boundary at 5 500 (block length, the held start that is the wrap)", true, 5_400, Some(150), 100, &[2_100, 4_100, 5_500]),
    ];
    for (label, repeat, period, switch_at, start, ends) in cases {
        let mut m = if switch_at.is_some() { ideal(&[(1, period as usize), (2, 1_000)]) } else { ideal(&[(1, period as usize)]) };
        m.book(0, start, repeat, LatePolicy::DropAndFlag);
        if let Some(at) = switch_at {
            m.go(at);
            m.book_switch(1, at);
        }
        let mut blocks = 0;
        for end in ends.iter().copied() {
            let deadline = due_of(end);
            run_to(&m.world, &mut [&mut m.radio], deadline - 2);
            let due = m.world.auth.next_due().expect("a wakeup").ticks_in(ROOT).unwrap();
            assert_eq!(due, deadline, "{label}: the scheduled deadline of the block ending at {end}");
            m.go_tick(deadline - 1);
            assert_eq!(m.tx_blocks(), blocks, "{label}: nothing is emitted one tick before the deadline of the block ending at {end}");
            m.go_tick(deadline);
            blocks += 1;
            assert_eq!(m.tx_blocks(), blocks, "{label}: the block ending at {end} is emitted at its deadline");
        }
    }
}

// ------------------------------------------------------------------------------------------------------------ E3b-1 (design note 49 E3b), table C5

#[test]
fn e3b_c5_receipts_by_provider_mock_column() {
    // Table C5 (`o::FRONT_RECEIPTS`, the literal the UHD owner's `e3b_c5_receipts_by_provider_uhd_column` reads too): a booking received after the
    // row's bookings (and Stop) is accepted or refused as the table says, as the independent oracle says and as the Mock's receipt says. Each
    // probe runs on a fresh stepped Mock; a receipt is the tick the test steps to, never a thread's timing.
    for row in o::FRONT_RECEIPTS {
        for &(offset, len, want) in row.probes {
            let mut waves: Vec<(u8, usize)> = row.bookings.iter().enumerate().map(|(i, &(_, len))| (i as u8 + 1, len.unwrap_or(100) as usize)).collect();
            waves.push((60, len as usize));
            let mut m = x3xx(&waves);
            let t_k = X;
            for (i, &(at, booked)) in row.bookings.iter().enumerate() {
                m.book(i, row.start(t_k, at), booked.is_none(), LatePolicy::DropAndFlag);
            }
            assert_eq!(m.all_refusals(), 0, "{}: the row's own bookings are accepted", row.label);
            if row.stop {
                m.stop();
            }
            m.all_refusals();
            m.book(row.bookings.len(), row.start(t_k, offset), false, LatePolicy::DropAndFlag);
            let reasons = m.refusals();
            let got = if reasons.is_empty() { o::Verdict::Accept } else { o::Verdict::Refuse };
            assert!(reasons.iter().all(|reason| reason == "MR-16: burst overlaps an accepted booking"), "{}: a refusal is an overlap: {reasons:?}", row.label);
            assert_eq!(row.verdict(&m.p, t_k, T0, (offset, len)), want, "{}: probe {offset}: the oracle reads the literal", row.label);
            assert_eq!(got, want, "{}: probe {offset} of {len}: the Mock's receipt", row.label);
        }
    }
}
