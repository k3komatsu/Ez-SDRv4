//! Conformance of `ModelDevice` to `FakeDevice` (stage E2a-2, S9; test plan §8, §14, §15): the scripts (a)–(h) run through the
//! `Device` trait on both and are compared with the expected results stated here. Both devices run at ratio one (1 tick = 1
//! sample = 1 ms), with packets of four samples; every recv asks for four samples with a timeout of three ticks.
//!
//! The clock. `FakeDevice` runs on its manual clock (`ClockMode::Manual`) from a participant thread: the clock stands while the
//! script runs and advances only while the script waits on it, so `advance_to` stops exactly at its deadline and a blocking
//! call returns at its natural instant (M3, M4). `ModelDevice` has virtual time: `advance_to` ticks it, and a call that blocks
//! runs on a helper thread while the script thread ticks one at a time until the call returns. No verdict reads the wall clock.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use super::cell::{InFlight, Outcome, Phase, Terminal, ValueState};
use super::fixtures::{cell, seg, value};
use super::force::{Actor, Sched};
use super::model::Device as Model;
use super::model_device::ModelDevice;
use super::rules;
use crate::device::{Device, Dir, FakeConfig, FakeDevice, RxRecv, Settings};
use crate::host::ClockMode;

const TIMEOUT: Duration = Duration::from_millis(3);

/// One device under a script: the trait object the script calls, the clock it advances, and the DDC's log it reads.
enum Kind {
    Fake(Arc<FakeDevice>),
    Model(Arc<ModelDevice>),
}

pub(super) struct Rig {
    kind: Kind,
    /// The ticks of every sample a recv returned, in order (the probe's input).
    seen: RefCell<Vec<i64>>,
    /// The number of channels every recv returned (each with the same length): the probe's coverage comes from this, not from a literal.
    returned: std::cell::Cell<Option<usize>>,
}

impl Rig {
    fn dev(&self) -> &dyn Device {
        match &self.kind {
            Kind::Fake(device) => &**device,
            Kind::Model(device) => &**device,
        }
    }

    fn now(&self) -> i64 {
        self.dev().time_now().unwrap()
    }

    fn advance_to(&self, t: i64) {
        let now = self.now();
        assert!(t >= now, "time does not go back: {now} -> {t}");
        match &self.kind {
            Kind::Fake(device) => device.host().sleep(Duration::from_millis((t - now) as u64)),
            Kind::Model(device) => device.advance((t - now) as usize),
        }
    }

    /// A call that may wait for time to pass. On the fake the clock runs while the caller waits; on the model the script thread
    /// ticks, one tick whenever the call is blocked, until it returns.
    fn call<T: Send>(&self, f: impl FnOnce(&dyn Device) -> T + Send) -> T {
        match &self.kind {
            Kind::Fake(device) => f(&**device),
            Kind::Model(device) => std::thread::scope(|scope| {
                let helper = scope.spawn(|| f(&**device));
                let started = std::time::Instant::now();
                while !helper.is_finished() {
                    assert!(started.elapsed() < Duration::from_secs(10), "a model call neither returned nor blocked");
                    if device.sched().is_blocked(Actor::Main) {
                        device.advance(1);
                    }
                    std::thread::yield_now();
                }
                helper.join().unwrap()
            }),
        }
    }

    /// `(tick, value id)` of each timed write the DDC of channel 0 applied, in order.
    fn applications(&self) -> Vec<(i64, u64)> {
        self.applications_on(0)
    }

    fn applications_on(&self, chan: usize) -> Vec<(i64, u64)> {
        match &self.kind {
            Kind::Fake(device) => device.dsp_log(Dir::Rx, chan).into_iter().filter(|(_, _, timed)| *timed).map(|(tick, freq, _)| (tick, freq as u64)).collect(),
            Kind::Model(device) => device.snapshot().0.dsp.get(&chan).map_or_else(Vec::new, |chain| chain.log.iter().map(|&(tick, id)| (tick, id)).collect()),
        }
    }

    /// Opens stream 0 on channel 0 at time zero.
    fn open(&self) {
        self.open_on(&[0]);
    }

    fn open_on(&self, channels: &[usize]) {
        let d = self.dev();
        d.set_time_zero(false).unwrap();
        d.apply(Dir::Rx, 0, &Settings { rate: Some(1e3), ..Settings::default() }, None).unwrap();
        d.rx_open(0, channels).unwrap();
    }

    /// The channels of the samples this script received (what a probe may claim to cover).
    fn returned_channels(&self) -> u32 {
        self.returned.get().expect("the script received samples") as u32
    }

    fn recv(&self, log: &mut Vec<String>) {
        let result = self.call(|d| d.rx_recv(0, 4, TIMEOUT, true));
        let text = match result {
            RxRecv::Samples { first_tick, samples } => {
                let len = samples[0].len();
                assert!(samples.iter().all(|channel| channel.len() == len), "every returned channel has the same length: {:?}", samples.iter().map(Vec::len).collect::<Vec<_>>());
                let seen_channels = self.returned.get().unwrap_or(samples.len());
                self.returned.set(Some(seen_channels));
                assert_eq!(samples.len(), seen_channels, "every recv returns the same number of channels");
                self.seen.borrow_mut().extend((0..len as i64).map(|i| first_tick + i));
                format!("Samples(first {first_tick}, {len})")
            }
            RxRecv::Timeout => "Timeout".to_string(),
            RxRecv::LateCommand => "LateCommand".to_string(),
            _ => "other".to_string(),
        };
        log.push(format!("recv {text} T={}", self.now()));
    }

    fn apply(&self, log: &mut Vec<String>, id: u64, at: i64) -> bool {
        self.apply_on(log, 0, id, at)
    }

    fn apply_on(&self, log: &mut Vec<String>, chan: usize, id: u64, at: i64) -> bool {
        let result = self.call(|d| d.apply(Dir::Rx, chan, &Settings { freq: Some(id as f64), ..Settings::default() }, Some(at)));
        log.push(format!("apply {id} at {at}: {}", if result.is_ok() { "Ok" } else { "Err" }));
        result.is_ok()
    }
}

// ------------------------------------------------------------------------------------------------ the two devices

/// Runs `script` on a `FakeDevice` whose manual clock the script's participant thread drives.
pub(super) fn on_fake(queue: usize, script: fn(&Rig) -> Vec<String>) -> Vec<String> {
    let device =
        Arc::new(FakeDevice::new(FakeConfig { master_clock_rate: 1000, rx_packet: Some(4), rx_latency: Duration::ZERO, command_queue: queue, clock: ClockMode::Manual, ..FakeConfig::default() }));
    let (done, result) = std::sync::mpsc::channel();
    let rig = Rig { kind: Kind::Fake(device.clone()), seen: RefCell::new(Vec::new()), returned: std::cell::Cell::new(None) };
    let participant = device
        .host()
        .spawn("conformance", move || {
            let _ = done.send(script(&rig));
        })
        .unwrap();
    let log = result.recv_timeout(Duration::from_secs(10)).expect("the script ends");
    participant.join().unwrap();
    log
}

/// Runs `script` on a `ModelDevice` (packet four, FIFO 32), time driven by the script.
pub(super) fn on_model(queue: usize, script: fn(&Rig) -> Vec<String>) -> Vec<String> {
    let device = ModelDevice::new(Sched::new(false), Model::new(queue, 32), 4);
    script(&Rig { kind: Kind::Model(device), seen: RefCell::new(Vec::new()), returned: std::cell::Cell::new(None) })
}

// -------------------------------------------------------------------------------------------------------- the probe

/// The consumption the production rule gives the samples a script received, for values it applied: `(id, instant, accepted)`.
/// A value is offered only once its acceptance is recorded (`accepted` is `None` before).
fn consumption(values: &[(u64, i64, Option<i64>)], channels: u32, samples: &[i64]) -> Vec<(u64, i64)> {
    consumption_on(values.iter().map(|&(id, e, accepted)| (id, 0, e, accepted)).collect::<Vec<_>>().as_slice(), channels, samples)
}

/// As [`consumption`], for a streamer of `channels` channels and values `(id, channel, instant, accepted)`.
fn consumption_on(values: &[(u64, usize, i64, Option<i64>)], channels: u32, samples: &[i64]) -> Vec<(u64, i64)> {
    let mut c = cell(Phase::Streaming(seg(0)), 0, None, 0);
    for &(id, channel, e, accepted) in values {
        let mut entry = value(channel, e, id);
        entry.accepted_at = accepted;
        c.in_flight.push(InFlight { entry, state: ValueState::Done(Terminal::Call(Outcome::Ok)), left: false });
    }
    rules::consume_samples(&c, (1..channels).fold(ezsdr_radio::timeline::Channels::first(1), |all, channel| all.with(channel)), &Default::default(), samples)
}

// ------------------------------------------------------------------------------------------------------- the scripts

pub(super) fn script_a(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 2).unwrap();
    rig.advance_to(6);
    rig.recv(&mut log);
    rig.advance_to(7);
    rig.dev().rx_stop(0, None).unwrap();
    rig.recv(&mut log);
    rig.recv(&mut log);
    log
}

pub(super) fn script_b(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 1).unwrap();
    rig.advance_to(6);
    rig.dev().rx_stop(0, None).unwrap();
    rig.dev().rx_start(0, 12).unwrap();
    rig.recv(&mut log);
    rig.recv(&mut log);
    rig.recv(&mut log);
    rig.advance_to(16);
    rig.recv(&mut log);
    log
}

/// (b2): the start behind the tail has passed by the time the next recv enters.
pub(super) fn script_b2(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 1).unwrap();
    rig.advance_to(6);
    rig.dev().rx_stop(0, None).unwrap();
    rig.dev().rx_start(0, 8).unwrap();
    rig.recv(&mut log);
    rig.recv(&mut log);
    rig.advance_to(10);
    rig.recv(&mut log);
    rig.recv(&mut log);
    log
}

pub(super) fn script_c(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.advance_to(10);
    rig.dev().rx_start(0, 8).unwrap();
    rig.recv(&mut log);
    rig.recv(&mut log);
    let t0 = rig.now();
    rig.dev().rx_start_now(0).unwrap();
    rig.advance_to(t0 + 4);
    rig.recv(&mut log);
    log
}

/// (i): a recv whose packet is not complete at its deadline times out, although part of it is buffered; the next recv, entered
/// after the packet is complete, returns it whole.
pub(super) fn script_i(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 0).unwrap();
    rig.recv(&mut log);
    rig.recv(&mut log);
    log
}

pub(super) fn script_d(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 0).unwrap();
    rig.advance_to(2);
    rig.apply(&mut log, 1, 5);
    rig.apply(&mut log, 2, 5);
    rig.advance_to(9);
    log.push(format!("applied {:?}", rig.applications()));
    let accepted = (10..=42).take_while(|&id| rig.apply(&mut Vec::new(), id, 1000)).count();
    log.push(format!("accepted {accepted} timed writes before the first error"));
    log
}

pub(super) fn script_e(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 0).unwrap();
    rig.apply(&mut log, 1, 8);
    rig.apply(&mut log, 2, 9);
    rig.apply(&mut log, 3, 10);
    log.push(format!("returned at T={}", rig.now()));
    rig.advance_to(12);
    log.push(format!("applied {:?}", rig.applications()));
    log
}

pub(super) fn script_f(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 2).unwrap();
    rig.advance_to(3);
    rig.apply(&mut log, 7, 10);
    let accepted = rig.now();
    rig.advance_to(6);
    rig.dev().rx_stop(0, None).unwrap();
    rig.dev().rx_close(0);
    rig.dev().rx_open(0, &[0]).unwrap();
    rig.dev().rx_start(0, 12).unwrap();
    rig.advance_to(16);
    rig.recv(&mut log);
    log.push(format!("applied {:?}", rig.applications()));
    log.push(format!("probe {:?}", consumption(&[(7, 10, Some(accepted))], rig.returned_channels(), &rig.seen.borrow())));
    log
}

pub(super) fn script_g(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 0).unwrap();
    rig.advance_to(1);
    rig.apply(&mut log, 1, 20);
    let a1 = rig.now();
    rig.advance_to(2);
    rig.apply(&mut log, 2, 8);
    let a2 = rig.now();
    rig.advance_to(24);
    for _ in 0..6 {
        rig.recv(&mut log);
    }
    log.push(format!("applied {:?}", rig.applications()));
    log.push(format!("probe {:?}", consumption(&[(1, 20, Some(a1)), (2, 8, Some(a2))], rig.returned_channels(), &rig.seen.borrow())));
    log
}

/// (g2): the same two commands on two channels of one stream. The radio's command queue is shared, so the later instant holds
/// back the earlier one's effective instant (20), but each channel's consumption rule looks only at its own debt.
pub(super) fn script_g2(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open_on(&[0, 1]);
    rig.dev().rx_start(0, 0).unwrap();
    rig.advance_to(1);
    rig.apply_on(&mut log, 0, 1, 20);
    let a1 = rig.now();
    rig.advance_to(2);
    rig.apply_on(&mut log, 1, 2, 8);
    let a2 = rig.now();
    rig.advance_to(24);
    for _ in 0..6 {
        rig.recv(&mut log);
    }
    log.push(format!("applied channel 0 {:?}, channel 1 {:?}", rig.applications_on(0), rig.applications_on(1)));
    log.push(format!("probe {:?}", consumption_on(&[(1, 0, 20, Some(a1)), (2, 1, 8, Some(a2))], rig.returned_channels(), &rig.seen.borrow())));
    log
}

pub(super) fn script_h(rig: &Rig) -> Vec<String> {
    let mut log = Vec::new();
    rig.open();
    rig.dev().rx_start(0, 0).unwrap();
    rig.advance_to(6);
    rig.apply(&mut log, 5, 4);
    let accepted = rig.now();
    rig.advance_to(9);
    log.push(format!("applied {:?}", rig.applications()));
    rig.recv(&mut log);
    rig.recv(&mut log);
    let seen = rig.seen.borrow().clone();
    let before: Vec<i64> = seen.iter().copied().filter(|t| (4..6).contains(t)).collect();
    let after: Vec<i64> = seen.iter().copied().filter(|t| (6..8).contains(t)).collect();
    log.push(format!("probe before acceptance {:?}", consumption(&[(5, 4, None)], rig.returned_channels(), &before)));
    log.push(format!("probe after acceptance {:?}", consumption(&[(5, 4, Some(accepted))], rig.returned_channels(), &after)));
    log
}

// ----------------------------------------------------------------------------------------------------------- tests

/// Runs `script` on both devices; each must equal `expected`. A difference names the device that differs.
fn conforms(name: &str, queue: usize, script: fn(&Rig) -> Vec<String>, expected: &[&str]) {
    let fake = on_fake(queue, script);
    let model = on_model(queue, script);
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    println!("{name}\n  fake : {fake:?}\n  model: {model:?}");
    assert_eq!(fake, expected, "{name}: FakeDevice differs from the expected result");
    assert_eq!(model, expected, "{name}: ModelDevice differs from the expected result");
}

#[test]
fn s9a_timed_start_receive_stop_tail() {
    conforms("a", 64, script_a, &["recv Samples(first 2, 4) T=6", "recv Samples(first 6, 1) T=7", "recv Timeout T=10"]);
}

#[test]
fn s9b_start_behind_a_tail_is_promoted_at_recv_entry() {
    conforms("b", 64, script_b, &["recv Samples(first 1, 4) T=6", "recv Samples(first 5, 1) T=6", "recv Timeout T=9", "recv Samples(first 12, 4) T=16"]);
    conforms("b2", 64, script_b2, &["recv Samples(first 1, 4) T=6", "recv Samples(first 5, 1) T=6", "recv LateCommand T=10", "recv Timeout T=13"]);
}

#[test]
fn s9c_start_in_the_past_is_missed_and_start_now_is_never_late() {
    conforms("c", 64, script_c, &["recv LateCommand T=10", "recv Timeout T=13", "recv Samples(first 13, 4) T=17"]);
}

#[test]
fn s9i_a_packet_not_complete_at_the_deadline_is_a_timeout() {
    conforms("i", 64, script_i, &["recv Timeout T=3", "recv Samples(first 0, 4) T=4"]);
}

#[test]
fn s9d_timed_applies_and_the_dsp_fifo_bound() {
    conforms("d", 64, script_d, &["apply 1 at 5: Ok", "apply 2 at 5: Ok", "applied [(5, 1), (6, 2)]", "accepted 32 timed writes before the first error"]);
}

#[test]
fn s9e_full_queue_blocks_until_the_head_leaves() {
    conforms("e", 2, script_e, &["apply 1 at 8: Ok", "apply 2 at 9: Ok", "apply 3 at 10: Ok", "returned at T=8", "applied [(8, 1), (9, 2), (10, 3)]"]);
}

#[test]
fn s9f_a_queued_value_persists_across_close_and_reopen() {
    conforms("f", 64, script_f, &["apply 7 at 10: Ok", "recv Samples(first 12, 4) T=16", "applied [(12, 7)]", "probe [(7, 12)]"]);
}

#[test]
fn s9g_a_later_instant_then_an_earlier_one() {
    conforms(
        "g",
        64,
        script_g,
        &[
            "apply 1 at 20: Ok",
            "apply 2 at 8: Ok",
            "recv Samples(first 0, 4) T=24",
            "recv Samples(first 4, 4) T=24",
            "recv Samples(first 8, 4) T=24",
            "recv Samples(first 12, 4) T=24",
            "recv Samples(first 16, 4) T=24",
            "recv Samples(first 20, 4) T=24",
            "applied [(20, 1), (21, 2)]",
            "probe [(1, 20), (2, 21)]",
        ],
    );
}

#[test]
fn s9h_consumption_before_and_after_acceptance() {
    conforms(
        "h",
        64,
        script_h,
        &["apply 5 at 4: Ok", "applied [(6, 5)]", "recv Samples(first 0, 4) T=9", "recv Samples(first 4, 4) T=9", "probe before acceptance []", "probe after acceptance [(5, 6)]"],
    );
}

/// FINDING (Q1 at the device, test plan §14 (g); recorded for the E3 report). Between the raw `Device` and the consumption rule, not
/// proven reachable in production. On one channel the rule agrees with the devices: the DDC applies the later-instant value at 20
/// and the earlier-instant one at 21, and the debt's order (the first entry of a channel gates the rest) consumes them at 20 and 21
/// (`s9g`). The radio's command queue is shared by the channels: with the values on two channels, the second command's effective
/// instant is 20 (`max(8, 20, now)`), the channel-1 DDC applies it at tick 20 on both devices, and the production rule
/// `RxChain::consume`, which reads `max(instant, accepted_at)` with `accepted_at` the clock read after the call (here 2), takes the
/// sample at tick 8 as consumption evidence, twelve ticks before the application. The raw sequence is legal at `Device`, but the
/// production booking cannot hand instant 8 after instant 20 as written (`q1_g2_is_not_reachable_through_the_booking_floor`), so
/// production reachability stays unproven: the E1/E2 brain releases values at arbitrary instants, which production's booking
/// floor does not allow. Not waived; no effective-instant API change is recommended on this evidence alone. When the finding is
/// resolved this test is replaced.
#[test]
fn finding_q1_two_channels_evidence_precedes_application() {
    let expected = [
        "apply 1 at 20: Ok",
        "apply 2 at 8: Ok",
        "recv Samples(first 0, 4) T=24",
        "recv Samples(first 4, 4) T=24",
        "recv Samples(first 8, 4) T=24",
        "recv Samples(first 12, 4) T=24",
        "recv Samples(first 16, 4) T=24",
        "recv Samples(first 20, 4) T=24",
        "applied channel 0 [(20, 1)], channel 1 [(20, 2)]",
        "probe [(2, 8), (1, 20)]",
    ];
    for (device, run) in [("FakeDevice", on_fake(64, script_g2)), ("ModelDevice", on_model(64, script_g2))] {
        assert_eq!(run, expected, "{device}");
    }
}

/// Why g2's order is not a production order (`provider/control.rs:648-711`, `timeline::timed_instant`): a command booked while an
/// earlier one lies inside the release window is floored to the earlier one's instant, so the second booking's instant is 20, not 8,
/// and the cell's entry carries it; consumption is then at 20, the application tick. An earlier booking beyond the window does not
/// floor the second, but release goes in instant order, so the device meets 8 before 20 and its effective instant never passes the
/// booked one. Unit level only: the Provider is not run (INFERRED for the release order; `control.rs:690-711`).
#[test]
fn q1_g2_is_not_reachable_through_the_booking_floor() {
    use ezsdr_radio::timeline::timed_instant;
    let (lead, window) = (1, 100);
    assert_eq!(timed_instant(Some(20), 1, lead, window, []), (20, false), "v1 books at its instant");
    assert_eq!(timed_instant(Some(8), 2, lead, window, [20]), (20, true), "v2 is floored behind v1 and reported late");
    // The entry for v2 carries the floored instant 20: consumption on channel 1 is at 20, equal to the device's application tick.
    assert_eq!(consumption_on(&[(1, 0, 20, Some(1)), (2, 1, 20, Some(2))], 2, &(0..24).collect::<Vec<_>>()), vec![(1, 20), (2, 20)]);
    // Beyond the window the floor does not apply; release in instant order puts 8 first, so the device never sees 20 before 8.
    assert_eq!(timed_instant(Some(8), 2, lead, 10, [20]), (8, false));
}

/// The window's boundary, exactly (Sol: the floor is sound, the boundary was not pinned): with a 3 ms window an earlier booking at
/// `now + 3` is inside it (the filter is `<= now + window`) and floors the next booking; one at `now + 3` plus one tick is outside
/// and does not. Release then takes the outside one in instant order.
#[test]
fn q1_booking_floor_window_boundary_is_inclusive_at_exactly_the_window() {
    use ezsdr_radio::timeline::timed_instant;
    let (now, lead, window) = (2, 1, 3);
    assert_eq!(timed_instant(Some(4), now, lead, window, [now + window]), (now + window, true), "an earlier booking at exactly now + window floors");
    assert_eq!(timed_instant(Some(4), now, lead, window, [now + window + 1]), (4, false), "one tick beyond the window does not");
}
