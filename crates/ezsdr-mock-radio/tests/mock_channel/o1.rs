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
    assert_eq!(m.switch_rows()[0]["outcome"], o::outcome_after_stop(b, bound).as_str());
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

fn stopped_after_switch(after: i64) -> (M, i64, i64, i64, i64) {
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
    (m, s, b, bound, cut)
}

#[test]
fn o1_mock_t4d_a_stop_with_b_before_its_bound_leaves_the_row_applied() {
    // RM-37, MR-25, MR-42 (seq 4): the Mock installed the switch at receipt, so with b < B it is applied and its new repeat is cut at MR-25's
    // point: the window edge is b + 600 and the new repeat's first 32 samples fit, so [b, b + 600) of it radiates. C2 (note 49 §11.10)
    // changes the ROW; the emission stays.
    let (mut m, s, b, bound, cut) = stopped_after_switch(600);
    assert_eq!(cut, b + 600);
    m.check(&o::Slots { segs: vec![o::Seg::repeating(8, s, 1_000, b), o::Seg::repeating(9, b, 1_000, cut)], cut: None }, s - 10, cut + 100);
    assert_eq!(m.switch_rows()[0]["outcome"], o::outcome_after_stop(b, bound).as_str(), "C2: this outcome changes with note 49 §11.10");
    assert_eq!(m.switch_rows()[0]["outcome"], "applied");
    m.assert_switch_ids(0);
    assert_eq!(m.pairs(), [(s, (b - s) as u64), (b, (cut - b) as u64)]);
    assert_eq!(m.full_rec(1), (b, (cut - b) as u64, ((cut - b) / 1_000) as u32, pushed(&m, s), false, "stop"), "the successor's record");
}

#[test]
fn o1_mock_t4e_a_stop_after_the_new_waveform_sounded_leaves_it_applied() {
    // RM-37 (seq 4): as T4d with the window edge at b + 3 000: 3 000 samples of the new waveform radiate; the row is applied.
    let (mut m, s, b, bound, cut) = stopped_after_switch(3_000);
    assert_eq!(cut, b + 3_000);
    m.check(&o::Slots { segs: vec![o::Seg::repeating(8, s, 1_000, b), o::Seg::repeating(9, b, 1_000, cut)], cut: None }, s - 10, cut + 100);
    assert_eq!(m.switch_rows()[0]["outcome"], o::outcome_after_stop(b, bound).as_str());
    m.assert_switch_ids(0);
    assert_eq!(m.full_rec(0), (s, (b - s) as u64, ((b - s) / 1_000) as u32, None, false, "eob"), "the predecessor's record");
    assert_eq!(m.full_rec(1), (b, (cut - b) as u64, ((cut - b) / 1_000) as u32, pushed(&m, s), false, "stop"), "the successor's record");
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
