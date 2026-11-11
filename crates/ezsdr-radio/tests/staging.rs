//! RM-21: `ezsdr_radio::staging`, which receive value a Provider stages and what a `start_rx` takes (note 47 stage 3).

use std::collections::BTreeMap;

use ezsdr_kernel::event::ActionId;
use ezsdr_kernel::spec::{Key, Value};
use ezsdr_radio::CancelReason;
use ezsdr_radio::staging::{
    Booked, Bookings, Outcome, Overlap, Playing, Read, Refusal, STOP_FRONT, Settle, Settled, Staging, Started, SwitchAccepted, SwitchOutcome, SwitchRefusal, TxBook, TxList, TxSend, Until,
    closed_short_gap, ordinary_k, repeat_boundary, stop_bound,
};

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn num(x: f64) -> Value {
    Value::num(x).unwrap()
}

/// The transmit side of a Run: channel 0 in stream 0, channel 1 in stream 1, channel 2 in none.
fn transmit_staging() -> Staging {
    let config: BTreeMap<Key, Value> = [
        ("radio.tx.c0.stream", Value::from(0)),
        ("radio.tx.c1.stream", Value::from(1)),
        ("radio.tx.c2.stream", Value::from(-1)),
        ("radio.tx.c0.frequency_hz", num(1.0e9)),
        ("radio.tx.c0.gain_db", num(0.0)),
    ]
    .into_iter()
    .map(|(name, value)| (key(name), value))
    .collect();
    Staging::new(config)
}

/// The Run's configuration: receive channel 0 in stream 0, channel 1 in stream 1, channel 2 in none.
fn staging() -> Staging {
    let config: BTreeMap<Key, Value> = [
        ("radio.rx.c0.stream", Value::from(0)),
        ("radio.rx.c1.stream", Value::from(1)),
        ("radio.rx.c2.stream", Value::from(-1)),
        ("radio.rx.c0.frequency_hz", num(1.0e9)),
        ("radio.rx.c1.frequency_hz", num(1.0e9)),
        ("radio.rx.c0.gain_db", num(0.0)),
    ]
    .into_iter()
    .map(|(name, value)| (key(name), value))
    .collect();
    Staging::new(config)
}

const NEVER: fn(u32) -> bool = |_| false;
const IDLE: fn(u32) -> Bookings = |_| Bookings::unarmed();

/// RM-15: the samples a closed short burst's front keeps past its padded end at 1 MS/s on a 200 MHz root: ceil(15 µs × 1 MS/s).
const GAP_1M: i64 = 15;

/// A record armed for a stream at 1 MS/s on a 200 MHz root (`GAP_1M`).
fn armed() -> Bookings {
    armed_with(GAP_1M)
}

fn armed_with(gap: i64) -> Bookings {
    let mut bookings = Bookings::unarmed();
    bookings.arm(gap);
    bookings
}

/// The settled switches as (id, outcome), for the assertions that do not read the boundary.
fn settled(bookings: &mut Bookings) -> Vec<(ActionId, SwitchOutcome)> {
    bookings.take_settled().into_iter().map(|settled| (settled.id, settled.outcome)).collect()
}

#[test]
fn rm_21_a_value_for_a_streaming_stream_is_refused_and_a_stopped_one_stages_it() {
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    assert_eq!(staging.set(&frequency, num(2.0e9), false, 0, |_| true, IDLE), Err(Refusal::Streaming { channel: 0, stream: 0 }));
    assert_eq!(staging.snapshot()[&frequency], num(1.0e9), "a refused value is not kept");
    assert_eq!(staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE), Ok(()));
    assert_eq!(staging.snapshot()[&frequency], num(2.0e9));
    // The closure is asked about the channel's own stream, and only that one.
    let asked = std::cell::RefCell::new(Vec::new());
    staging
        .set(
            &key("radio.rx.c1.frequency_hz"),
            num(3.0e9),
            false,
            0,
            |stream| {
                asked.borrow_mut().push(stream);
                false
            },
            IDLE,
        )
        .unwrap();
    assert_eq!(*asked.borrow(), [1]);
}

#[test]
fn rm_21_the_last_staged_value_wins() {
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE).unwrap();
    staging.set(&frequency, num(3.0e9), false, 0, NEVER, IDLE).unwrap();
    assert_eq!(staging.snapshot()[&frequency], num(3.0e9));
    staging.set(&frequency, num(4.0e9), false, 0, |_| true, IDLE).unwrap_err();
    assert_eq!(staging.snapshot()[&frequency], num(3.0e9), "a refused value between accepted ones changes nothing");
    // Gain and antenna are staged like frequency, each under its own key.
    staging.set(&key("radio.rx.c0.gain_db"), num(7.0), false, 0, NEVER, IDLE).unwrap();
    staging.set(&key("radio.rx.c0.antenna"), Value::from("TX/RX"), false, 0, NEVER, IDLE).unwrap();
    let snapshot = staging.snapshot();
    assert_eq!((&snapshot[&key("radio.rx.c0.gain_db")], &snapshot[&key("radio.rx.c0.antenna")]), (&num(7.0), &Value::from("TX/RX")));
}

#[test]
fn rm_21_a_peer_stream_s_state_does_not_matter() {
    // Transmit stream 0 is busy until tick 100, stream 1 is idle: a value for stream 1's channel is taken at receipt 50.
    let mut staging = transmit_staging();
    let mut busy = armed();
    busy.book(Some(100));
    let stream_0_busy = |stream: u32| if stream == 0 { busy.clone() } else { armed() };
    assert_eq!(staging.set(&key("radio.tx.c0.frequency_hz"), num(2.0e9), false, 50, NEVER, stream_0_busy), Err(Refusal::TxBusy { until: Until::At(100) }));
    assert_eq!(staging.set(&key("radio.tx.c1.frequency_hz"), num(2.0e9), false, 50, NEVER, stream_0_busy), Ok(()));
    assert_eq!(staging.tx_snapshot(1).0, 1, "stream 1 counts its own value");
}

#[test]
fn rm_21_a_channel_in_no_stream_an_instant_or_another_key_is_refused() {
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    assert_eq!(staging.set(&key("radio.rx.c2.frequency_hz"), num(2.0e9), false, 0, NEVER, IDLE), Err(Refusal::NoStream(2)));
    assert_eq!(staging.set(&key("radio.rx.c5.frequency_hz"), num(2.0e9), false, 0, NEVER, IDLE), Err(Refusal::NoStream(5)), "a channel the configuration does not hold is in no stream");
    assert_eq!(staging.set(&frequency, num(2.0e9), true, 0, NEVER, IDLE), Err(Refusal::Instant));
    for other in ["radio.rx.s0.sample_rate_hz", "radio.rx.c0.stream", "radio.rx.c0.frequency_step_hz", "radio.full_duplex", "sim.seed"] {
        assert_eq!(staging.set(&key(other), num(1.0), false, 0, NEVER, IDLE), Err(Refusal::NotStaged), "{other}");
    }
    // A distinct reason per cause, each naming the rule.
    let reasons: Vec<String> = [Refusal::Instant, Refusal::NotStaged, Refusal::NoStream(2), Refusal::Streaming { channel: 0, stream: 0 }].iter().map(Refusal::reason).collect();
    assert!(reasons.iter().all(|reason| reason.starts_with("RM-21: ")), "{reasons:?}");
    assert_eq!(reasons.iter().collect::<std::collections::BTreeSet<_>>().len(), 4);
    assert!(reasons[0].contains("instant") && reasons[1].contains("only") && reasons[2].contains("no stream") && reasons[3].contains("send Stop first"));
}

#[test]
fn rm_21_a_snapshot_is_not_changed_by_a_later_value() {
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE).unwrap();
    let snapshot = staging.snapshot();
    staging.set(&frequency, num(3.0e9), false, 0, NEVER, IDLE).unwrap();
    assert_eq!(snapshot[&frequency], num(2.0e9));
}

#[test]
fn rm_21_the_line_decides_streaming_not_staging() {
    // `Staging` keeps no state of "streaming": the same value is refused for one answer and accepted for the next.
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    staging.set(&frequency, num(2.0e9), false, 0, |_| true, IDLE).unwrap_err();
    assert_eq!(staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE), Ok(()));
    assert_eq!(staging.set(&frequency, num(3.0e9), false, 0, |_| true, IDLE), Err(Refusal::Streaming { channel: 0, stream: 0 }));
    assert_eq!(staging.set(&frequency, num(3.0e9), false, 0, NEVER, IDLE), Ok(()));
}

#[test]
fn rm_21_a_start_of_a_streaming_stream_is_refused_and_a_stopped_one_takes_the_snapshot() {
    // The refusal names the stream, says what to do, keeps the values staged, and the accepted start holds the last staged value.
    let mut staging = staging();
    let frequency = key("radio.rx.c0.frequency_hz");
    staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE).unwrap();
    let refusal = staging.start(1, true).unwrap_err();
    assert_eq!(refusal, Refusal::StartWhileStreaming(1));
    let reason = refusal.reason();
    assert!(reason.starts_with("RM-21:") && reason.contains("stream 1") && reason.contains("streaming") && reason.contains("Stop"), "{reason}");
    assert_eq!(staging.start(0, false).unwrap()[&frequency], num(2.0e9), "a refused start left the staged value alone");
}

// RM-21, transmit form (note 47 stage 4): `Bookings` is the value rule of one transmit stream, in root ticks.

#[test]
fn rm_21_a_transmit_value_at_the_exclusive_end_is_accepted_and_one_sample_before_refused() {
    let mut busy = armed();
    busy.book(Some(100));
    assert_eq!(busy.stageable(100), Ok(()));
    assert_eq!(busy.stageable(99), Err(Refusal::TxBusy { until: Until::At(100) }));
}

#[test]
fn rm_21_a_transmit_value_after_the_end_is_accepted() {
    let mut busy = armed();
    busy.book(Some(100));
    assert_eq!(busy.stageable(1_100), Ok(()));
    assert_eq!(armed().stageable(0), Ok(()), "an idle stream takes a value");
}

#[test]
fn rm_21_a_transmit_value_is_refused_while_a_booked_burst_has_not_ended() {
    let mut busy = armed();
    busy.book(Some(100));
    assert_eq!(busy.stageable(50), Err(Refusal::TxBusy { until: Until::At(100) }));
}

#[test]
fn rm_21_the_last_booked_burst_decides() {
    let mut busy = armed();
    busy.book(Some(10));
    busy.book(Some(20));
    assert_eq!(busy.stageable(15), Err(Refusal::TxBusy { until: Until::At(20) }), "the later end counts");
    busy.book(Some(5));
    assert_eq!(busy.stageable(15), Err(Refusal::TxBusy { until: Until::At(20) }), "an earlier end does not shorten it");
}

#[test]
fn rm_21_a_transmit_value_is_refused_while_a_repeat_plays_and_taken_after_its_stop() {
    let mut busy = armed();
    busy.book(None);
    assert_eq!(busy.stageable(1_000), Err(Refusal::TxBusy { until: Until::Open }));
    busy.book(Some(50));
    assert_eq!(busy.stageable(1_000), Err(Refusal::TxBusy { until: Until::Open }), "a later booking does not close a repeat");
    busy.stop(110);
    assert_eq!(busy.stageable(109), Err(Refusal::TxBusy { until: Until::At(110) }));
    assert_eq!(busy.stageable(110), Ok(()));
}

#[test]
fn rm_21_a_stop_bounds_an_open_repeat_by_the_in_flight_window() {
    // The identity clock: the bound is the first sample at or after 110, plus five, so one sample past the old bound is refused.
    let mut busy = armed();
    busy.book(None);
    busy.stop(stop_bound(100, 10, |t| t, |k| k));
    assert_eq!(busy.stageable(114), Err(Refusal::TxBusy { until: Until::At(115) }));
    assert_eq!(busy.stageable(115), Ok(()));
}

#[test]
fn rm_16_stop_bound_is_at_or_after_plus_five() {
    // Grid 200, origin 7: the first sample at or after 2000 is 10 (at 2007), so the bound is sample 15 (root 3007).
    let (origin, n) = (7, 200);
    let at_or_after = |t: i64| (t - origin + n - 1).div_euclid(n);
    assert_eq!(stop_bound(1_000, 1_000, at_or_after, |k| origin + k * n), 3_007);
}

#[test]
fn rm_16_stop_bound_rounds_up_on_a_fractional_window() {
    // Grid 200, origin 0: `at + window` on a sample (2000) gives sample 15, and one tick past it (2001) gives sample 16.
    let (origin, n) = (0, 200);
    let at_or_after = |t: i64| (t - origin + n - 1).div_euclid(n);
    assert_eq!(stop_bound(1_000, 1_000, at_or_after, |k| origin + k * n), 3_000);
    assert_eq!(stop_bound(1_000, 1_001, at_or_after, |k| origin + k * n), 3_200);
}

#[test]
fn rm_21_a_stop_never_extends_a_booked_end() {
    let mut busy = armed();
    busy.book(Some(5));
    busy.stop(13);
    assert_eq!(busy.stageable(4), Err(Refusal::TxBusy { until: Until::At(5) }));
    assert_eq!(busy.stageable(5), Ok(()));
}

#[test]
fn rm_21_a_stop_leaves_an_idle_stream_idle() {
    let mut busy = armed();
    busy.stop(15);
    assert_eq!(busy.stageable(0), Ok(()));
}

#[test]
fn rm_21_a_transmit_refusal_reasons_name_the_busy_end_or_the_repeat() {
    let at = Refusal::TxBusy { until: Until::At(110) }.reason();
    let open = Refusal::TxBusy { until: Until::Open }.reason();
    assert!(at.starts_with("RM-21:") && at.contains("110"), "{at}");
    assert!(open.starts_with("RM-21:") && open.contains("repeating") && open.contains("Stop"), "{open}");
}

// RM-21, transmit form (note 47 stage 4): a transmit value is staged when its stream's `Bookings` allows it at the receipt.

#[test]
fn rm_21_an_accepted_transmit_value_bumps_its_stream_s_generation() {
    let mut staging = transmit_staging();
    let frequency = key("radio.tx.c0.frequency_hz");
    assert_eq!(staging.tx_snapshot(0).0, 0, "the Run's values are generation 0");
    assert_eq!(staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE), Ok(()));
    let (generation, values) = staging.tx_snapshot(0);
    assert_eq!((generation, &values[&frequency]), (1, &num(2.0e9)));
    // An equal value is a new generation as well (Sol 7): the device is configured again.
    assert_eq!(staging.set(&frequency, num(2.0e9), false, 0, NEVER, IDLE), Ok(()));
    assert_eq!(staging.tx_snapshot(0).0, 2);
    assert_eq!(staging.set(&key("radio.tx.c1.gain_db"), num(3.0), false, 0, NEVER, IDLE), Ok(()));
    assert_eq!((staging.tx_snapshot(0).0, staging.tx_snapshot(1).0), (2, 1), "a value bumps its own stream only");
}

#[test]
fn rm_21_a_transmit_value_is_refused_by_the_booked_end_and_a_refusal_keeps_the_generation() {
    let mut staging = transmit_staging();
    let mut busy = armed();
    busy.book(Some(100));
    let booked = |_: u32| busy.clone();
    assert_eq!(staging.set(&key("radio.tx.c0.frequency_hz"), num(2.0e9), false, 50, NEVER, booked), Err(Refusal::TxBusy { until: Until::At(100) }));
    assert_eq!(staging.tx_snapshot(0).0, 0, "a refused value is not counted");
    assert_eq!(staging.set(&key("radio.tx.c0.frequency_hz"), num(2.0e9), false, 100, NEVER, booked), Ok(()));
}

#[test]
fn rm_21_a_transmit_value_with_an_instant_or_a_rate_or_no_stream_is_refused() {
    let mut staging = transmit_staging();
    let frequency = key("radio.tx.c0.frequency_hz");
    assert_eq!(staging.set(&frequency, num(2.0e9), true, 0, NEVER, IDLE), Err(Refusal::Instant));
    assert_eq!(staging.set(&key("radio.tx.s0.sample_rate_hz"), num(2.0e6), false, 0, NEVER, IDLE), Err(Refusal::NotStaged));
    assert_eq!(staging.set(&key("radio.tx.c2.frequency_hz"), num(2.0e9), false, 0, NEVER, IDLE), Err(Refusal::NoStream(2)));
    assert_eq!(staging.tx_snapshot(0), (0, staging.snapshot()), "nothing was staged");
    assert_eq!(staging.snapshot()[&frequency], num(1.0e9));
}

#[test]
fn rm_21_a_transmit_value_is_not_judged_by_the_receive_stream_s_state() {
    // The receive streams' state is for receive keys only: a transmit value on an idle stream is taken although the receive side streams.
    let mut staging = transmit_staging();
    assert_eq!(staging.set(&key("radio.tx.c0.frequency_hz"), num(2.0e9), false, 0, |_| true, IDLE), Ok(()));
}

#[test]
fn rm_21_the_last_transmit_value_wins_on_an_empty_stream() {
    let mut staging = transmit_staging();
    let frequency = key("radio.tx.c0.frequency_hz");
    for value in [2.0e9, 3.0e9, 4.0e9] {
        assert_eq!(staging.set(&frequency, num(value), false, 0, NEVER, IDLE), Ok(()));
    }
    assert_eq!(staging.tx_snapshot(0).1[&frequency], num(4.0e9));
}

// RM-15, RM-16 (note 47 stage 5, 5a): the overlap rule of one transmit stream, in sample indices. The Providers do not call it yet.

const ON_TIME: fn(i64) -> i64 = |_| i64::MAX;
/// The horizon of a receipt before any booking is handed (RM-15): no start is refused for an earlier-starting booking.
const UNHANDED: i64 = i64::MIN;

#[test]
fn rm_21_busy_is_max_accepted_end() {
    let mut busy = armed();
    busy.book(Some(100));
    assert_eq!(busy.stageable(100), Ok(()), "a finite end equal to the receipt is accepted");
    busy.book(None);
    assert_eq!(busy.stageable(1_000), Err(Refusal::TxBusy { until: Until::Open }), "a playing repeat refuses");
    let mut cancelled = armed();
    cancelled.book(Some(100));
    cancelled.stop(60);
    assert_eq!(cancelled.stageable(59), Err(Refusal::TxBusy { until: Until::At(60) }), "a stop caps the end, and the end still counts");
}

#[test]
fn rm_15_1_to_3_sample_burst_front_is_transmitted_end_plus_gap() {
    // Sequence 1: a head of 1 to 3 samples is padded to four and fronts at 4 plus the gap, at 1, 2 and 200 MS/s on a 200 MHz root.
    for (ratio, gap) in [(200, 15), (100, 30), (1, 3000)] {
        assert_eq!(closed_short_gap(200_000_000, ratio, 1), gap);
        for len in 1..=3 {
            let mut bookings = armed_with(gap);
            bookings.accept(0, Some(len), 0, ON_TIME, UNHANDED).unwrap();
            for k in 4..4 + gap {
                assert_eq!(bookings.accept(k, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "ratio {ratio}, len {len}: {k} is inside the padded close and its gap");
            }
            assert_eq!(bookings.accept(4 + gap, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "ratio {ratio}, len {len}: the padded end 4 plus the gap is the front");
        }
    }
}

#[test]
fn rm_15_closed_burst_front_is_end_plus_gap() {
    // Sequence 2: a head of 4 to 32 samples fronts at its end plus the gap, at 1 and 2 MS/s.
    for gap in [15, 30] {
        for len in 4..=32 {
            let mut bookings = armed_with(gap);
            bookings.accept(0, Some(len), 0, ON_TIME, UNHANDED).unwrap();
            for k in len..len + gap {
                assert_eq!(bookings.accept(k, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "gap {gap}, len {len}: {k} is inside the close and its gap");
            }
            assert_eq!(bookings.accept(len + gap, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "gap {gap}, len {len}: end + gap is the front");
        }
    }
    let mut at_33 = armed();
    at_33.accept(0, Some(33), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(at_33.accept(33, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "33 samples hold a tail: the booking at its end joins it");
}

#[test]
fn rm_15_held_tail_front_is_end_plus_four() {
    let mut head = armed();
    head.accept(0, Some(33), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(head.accept(36, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "end + 3 is inside the tail's close");
    assert_eq!(head.accept(37, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "end + 4 is the front");
    let mut joined = armed();
    joined.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    joined.accept(40, Some(5), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(joined.accept(48, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "a joined piece of 5 keeps the chain's front at its end + 4");
    assert_eq!(joined.accept(49, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
}

#[test]
fn rm_15_contiguous_booking_joins_a_long_head() {
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(40, Some(5), 0, ON_TIME, UNHANDED), Ok(()), "one chain: the booking at the end joins it");
    assert_eq!(bookings.accept(48, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the joined chain ends at 45 and fronts at 49");
    assert_eq!(bookings.accept(49, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
}

#[test]
fn rm_15_short_burst_joins_a_long_chain_with_the_chain_front() {
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(53, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "B's end + 3 is inside the device burst's close");
    assert_eq!(bookings.accept(54, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "B's end + 4 is the chain's front");
}

#[test]
fn rm_15_candidate_running_into_a_later_booking_is_refused() {
    let mut bookings = armed();
    bookings.accept(50, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(0, Some(55), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "a candidate that runs into the later booking is refused");
    assert_eq!(bookings.accept(0, Some(10), 0, ON_TIME, UNHANDED), Ok(()), "one that ends before it is accepted");
}

#[test]
fn rm_15_back_to_back_bookings_are_accepted() {
    let mut later = armed();
    later.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(later.accept(81, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "its front is 100, the chain's start");
    let mut earlier = armed();
    earlier.accept(81, Some(1), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(earlier.accept(100, Some(40), 0, ON_TIME, UNHANDED), Ok(()), "the front 100 is the next start");
}

#[test]
fn rm_15_join_is_accepted_up_to_the_deadline_and_refused_after() {
    let deadline = |e: i64| 100 + e;
    let mut at_deadline = armed();
    at_deadline.accept(0, Some(40), 0, deadline, UNHANDED).unwrap();
    assert_eq!(at_deadline.accept(40, Some(5), 140, deadline, UNHANDED), Ok(()), "received exactly at J(e)");
    let mut after = armed();
    after.accept(0, Some(40), 0, deadline, UNHANDED).unwrap();
    assert_eq!(after.accept(40, Some(5), 141, deadline, UNHANDED), Err(Overlap::Booking), "one tick after J(e) it is refused");
}

#[test]
fn rm_16_every_joined_piece_keeps_its_join_point_open() {
    // S1 (note 47 stage 5, sol-code): A(40), B(40) joined at 40, C(40) joined at 80, all received on time; the owner holds
    // A's tail at 40 before any of them is drained. The join at 40 stays open, though C is the newest piece.
    let mut head = armed();
    head.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert!(head.continuation_open(0, 0, ON_TIME), "a head alone is open at its own start");
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(80, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.continuation_open(40, 0, ON_TIME), "B is accepted at 40 and not yet sent");
    assert!(bookings.continuation_open(80, 0, ON_TIME), "C is accepted at 80");
    assert!(!bookings.continuation_open(0, 0, ON_TIME), "no piece starts at the head's own start");
}

#[test]
fn rm_15_a_short_head_left_alone_is_closed_and_takes_no_join() {
    // S2 (UR-23, a consequence of a refused configure): A(40) at 100 with B(10) joined at 140. A is refused and leaves, so B is
    // closed at its standalone front (165). A booking at B's end (150) received before the join deadline is refused, not joined.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(140, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(100, Some(40));
    assert!(!bookings.continuation_open(150, 0, ON_TIME), "B is closed, so nothing continues it");
    assert_eq!(bookings.accept(150, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert_eq!(bookings.accept(164, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "B's front is 165, so 164 is inside it");
    assert_eq!(bookings.accept(165, Some(10), 0, ON_TIME, UNHANDED), Ok(()), "B's front is 165");
}

#[test]
fn rm_15_a_head_left_alone_with_joined_pieces_keeps_their_regions() {
    // S2 (Opus re-check): A(40) at 100 000 is refused and leaves. B(10) at 100 040 is the head, but C(4) at 100 050 and D(10) at
    // 100 054 remain: the chain keeps its front (D's, 100 066) and takes no join, so E(10) at 100 056, received before D is dispatched, is refused.
    let mut bookings = armed();
    bookings.accept(100000, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(100040, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(100050, Some(4), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(100054, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(100000, Some(40));
    assert_eq!(bookings.accept(100056, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert!(!bookings.continuation_open(100064, 0, ON_TIME), "the chain takes no join once its head has left");
}

#[test]
fn rm_15_a_short_piece_left_behind_keeps_its_own_front() {
    // S2 (Opus re-check): as above, with C one sample long. As a standalone head C fronts at 100 069 (its padded end plus the gap, 15), so a booking at 100 068 is refused.
    let mut bookings = armed();
    bookings.accept(100000, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(100040, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(100050, Some(1), 0, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(100000, Some(40));
    assert_eq!(bookings.accept(100053, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert_eq!(bookings.accept(100068, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "C's own front is 100 069");
    assert_eq!(bookings.accept(100069, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
}

#[test]
fn rm_16_a_passed_join_point_is_dropped_and_the_newest_is_kept() {
    // S1 (bound): B(10) at 140 and C(10) at 150 join A(40) at 100. Device time has passed 145, so B's join point is dropped; C,
    // the newest, stays, and a booking at C's end still joins the chain.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(140, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(150, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.continuation_open(140, 0, ON_TIME));
    bookings.prune(145);
    assert!(!bookings.continuation_open(140, 0, ON_TIME), "B's join point has passed");
    assert!(bookings.continuation_open(150, 0, ON_TIME), "C's join point is kept");
    bookings.drop_piece(100, Some(40));
    assert!(!bookings.continuation_open(150, 0, ON_TIME), "A's removal leaves B, then C, closed: the head is at most 32 samples (S2)");
}

#[test]
fn rm_16_continuation_is_open_to_its_deadline_and_close_ends_it() {
    let deadline = |e: i64| 100 + e;
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, deadline, UNHANDED).unwrap();
    assert!(bookings.continuation_open(40, 140, deadline), "a chain ending at 40 is open at its deadline");
    assert!(!bookings.continuation_open(40, 141, deadline), "one tick after the deadline it is not");
    assert!(!bookings.continuation_open(39, 0, deadline), "only the chain's end continues it");
    assert_eq!(bookings.close(40), Some(44), "close returns the chain's front");
    assert!(!bookings.continuation_open(40, 0, deadline), "a closed chain takes no join");
    assert_eq!(bookings.accept(40, Some(5), 0, deadline, UNHANDED), Err(Overlap::Booking), "so a booking at 40 is refused");
    assert_eq!(bookings.close(40), None, "a second close finds no open chain");
}

#[test]
fn rm_16_second_stop_keeps_the_earlier_bound() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(100, 110);
    bookings.halt(105, 200);
    assert_eq!(bookings.accept(109, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the tail still runs to 110");
    assert_eq!(bookings.accept(110, Some(5), 0, ON_TIME, UNHANDED), Ok(()), "the earlier bound stands");
}

#[test]
fn rm_16_stop_cancels_bookings_starting_at_or_after_the_stop() {
    let mut cancelled = armed();
    cancelled.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    cancelled.halt(100, 110);
    assert_eq!(cancelled.accept(105, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the cancelled span is refused by the Stop's region");
    assert_eq!(cancelled.accept(115, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the booking at the Stop's instant is gone");
    let mut kept = armed();
    kept.accept(99, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    kept.halt(100, 110);
    assert_eq!(kept.accept(109, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "a booking starting before the Stop keeps its front, capped at the bound");
    assert_eq!(kept.accept(110, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
}

#[test]
fn rm_16_stop_region_refuses_the_span_the_stop_cut() {
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(100, 110);
    assert_eq!(bookings.accept(105, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the region [100, 110) holds the span the Stop cut");
    assert_eq!(bookings.first_free(105, true, 0, ON_TIME, UNHANDED), Some(110), "first_free skips the region");
    assert_eq!(bookings.accept(110, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the bound is free");
}

#[test]
fn rm_16_a_later_booking_after_a_stop_on_a_long_burst_is_accepted() {
    let mut bookings = armed();
    bookings.accept(0, Some(1_000_000), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(100_000, 110_003);
    assert_eq!(bookings.accept(110_003, Some(5), 0, ON_TIME, UNHANDED), Ok(()), "the kept burst's front is capped at the Stop's bound, not its end");
}

#[test]
fn rm_16_second_stop_that_drops_a_booking_extends_the_region_to_the_hull() {
    let mut repeat = armed();
    repeat.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    repeat.halt(100, 110);
    repeat.accept(110, Some(1000), 0, ON_TIME, UNHANDED).unwrap();
    repeat.halt(105, 200);
    assert_eq!(repeat.accept(111, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the booking the second Stop dropped leaves its span to the region");
    // The first head (10 at 50, front 75) is cut by the first Stop, so its region is [50, 75); the second head (5 at 75, front 95) is
    // dropped by the second Stop, and the hull is [50, 95): the second head's own front, not its bound 85, is the hull's end.
    let mut hull = armed();
    hull.accept(50, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    hull.halt(50, 60);
    hull.accept(75, Some(5), 0, ON_TIME, UNHANDED).unwrap();
    hull.halt(55, 85);
    assert_eq!(hull.accept(40, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "its own front 59 reaches the first cut at 50: the hull keeps that start");
    assert_eq!(hull.accept(36, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "its own front 55 reaches 50, the hull's start, not the second Stop's 55");
    assert_eq!(hull.accept(70, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside the hull");
    assert_eq!(hull.accept(80, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside the hull's later half [75, 95), up to the dropped head's own front");
    assert_eq!(hull.accept(95, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the dropped head's own front");
}

#[test]
fn rm_16_a_stop_that_drops_nothing_adds_no_region() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(10, 20);
    bookings.accept(40, Some(5), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(40, 60);
    assert_eq!(bookings.accept(20, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the second Stop's region starts at 40, not at the first Stop's 10");
}

#[test]
fn rm_16_a_stop_that_cancels_a_booking_past_its_bound_adds_no_region() {
    let mut far = armed();
    far.accept(1_000_000, Some(100), 0, ON_TIME, UNHANDED).unwrap();
    far.halt(100, 10_103);
    assert_eq!(far.accept(5_000, Some(10), 0, ON_TIME, UNHANDED), Ok(()), "the far booking was never handed: its cancel adds no region");
    let mut head = armed();
    head.accept(50, Some(1), 0, ON_TIME, UNHANDED).unwrap();
    head.halt(10, 20);
    assert_eq!(head.accept(15, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "a booking at or after the bound cancels no span before it");
}

#[test]
fn rm_16_repeat_occupies_until_a_stop() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(1_000, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert_eq!(bookings.accept(-20, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "a booking whose region ends before the repeat's start");
    bookings.halt(500, 510);
    assert_eq!(bookings.accept(509, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert_eq!(bookings.accept(510, Some(5), 0, ON_TIME, UNHANDED), Ok(()), "a booking at the bound is accepted");
}

#[test]
fn rm_16_first_free_skips_an_accepted_span() {
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.first_free(110, true, 0, ON_TIME, UNHANDED), Some(144), "inside [100, 144) it returns the front");
    assert_eq!(bookings.first_free(99, true, 0, ON_TIME, UNHANDED), Some(99), "before it, the instant is free");
    assert_eq!(bookings.first_free(140, true, 0, ON_TIME, UNHANDED), Some(140), "the chain's end is a join point");
    assert_eq!(bookings.first_free(141, true, 0, ON_TIME, UNHANDED), Some(144));
    let deadline = |e: i64| e + 100;
    assert_eq!(bookings.first_free(140, true, 240, deadline, UNHANDED), Some(140), "received at J(e) it may still join");
    assert_eq!(bookings.first_free(140, true, 241, deadline, UNHANDED), Some(144), "received after J(e) the join point is held: the front");
    bookings.halt(120, 130);
    assert_eq!(bookings.first_free(125, true, 0, ON_TIME, UNHANDED), Some(130), "after a Stop the span is skipped to the capped front");
    let mut open = armed();
    open.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(open.first_free(5, true, 0, ON_TIME, UNHANDED), None, "an open repeat holds every later sample: no start is free");
}

#[test]
fn rm_16_a_repeat_at_a_join_point_skips_it_to_the_front() {
    // The ideal profile (no lead): 40 samples at 0, then a repeat targeted at 39 and decided at 40 is placed at the front, 44.
    let at_its_deadline = |e: i64| e;
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.first_free(40, true, 40, at_its_deadline, UNHANDED), Some(40), "a booking with a length may join at its deadline");
    assert_eq!(bookings.first_free(40, false, 40, at_its_deadline, UNHANDED), Some(44), "a repeat may not join: its first free sample is the front");
    assert_eq!(bookings.accept(44, None, 40, at_its_deadline, UNHANDED), Ok(()), "and is accepted there");
}

#[test]
fn rm_16_prune_keeps_a_chain_whose_front_is_past_the_receipt() {
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.prune(43);
    assert_eq!(bookings.accept(43, Some(1), 43, ON_TIME, UNHANDED), Err(Overlap::Booking), "a chain whose front is one sample past the receipt still holds its span");
    bookings.prune(44);
    assert_eq!(bookings.accept(44, Some(1), 44, ON_TIME, UNHANDED), Ok(()), "once the receipt reaches its front the chain is dropped");
}

#[test]
fn rm_16_ordinary_k_is_decide_s_threshold() {
    // 390 625 S/s on a 200 MHz root clock: 512 root ticks a sample, a 2 ms lead (400 000 ticks). The decision falls on sample 1 000;
    // a booking may start at the first sample whose instant is at least that instant plus the lead, 1 782, not at 1 781.
    let at_or_after = |t: i64| (t + 511).div_euclid(512);
    let instant = |k: i64| k * 512;
    let (decided, lead) = (511_489, 400_000);
    assert_eq!(ordinary_k(decided, lead, at_or_after, instant), 1_782);
    assert_eq!(at_or_after(decided + lead), 1_781, "the form that places a booking one sample before decide's own threshold");
}

#[test]
fn rm_15_a_booking_before_a_chain_inside_the_hand_over_horizon_is_refused() {
    let mut inside = armed();
    inside.accept(9_999, Some(1), 0, ON_TIME, 10_000).unwrap();
    assert_eq!(inside.accept(2_000, Some(40), 0, ON_TIME, 10_000), Err(Overlap::Handed), "a chain one sample inside the horizon may already be on the device");
    let mut repeat = armed();
    repeat.accept(5_000, None, 0, ON_TIME, 10_000).unwrap();
    assert_eq!(repeat.accept(2_000, Some(40), 0, ON_TIME, 10_000), Err(Overlap::Handed), "a handable repeat is refused the same way");
    let mut beyond = armed();
    beyond.accept(10_000, Some(1), 0, ON_TIME, 10_000).unwrap();
    assert_eq!(beyond.accept(2_000, Some(40), 0, ON_TIME, 10_000), Ok(()), "one sample beyond the horizon is not handed: judged as before");
    let mut after = armed();
    after.accept(5_000, Some(40), 0, ON_TIME, 10_000).unwrap();
    assert_eq!(after.accept(6_000, Some(10), 0, ON_TIME, 10_000), Ok(()), "a booking after the chain is not before it: the region rule alone decides");
    let mut cancelled = armed();
    cancelled.accept(5_000, Some(40), 0, ON_TIME, 10_000).unwrap();
    cancelled.halt(4_000, 4_010);
    assert_eq!(cancelled.accept(2_000, Some(40), 0, ON_TIME, 10_000), Ok(()), "a Stop cancelled the chain, so nothing is on the device there");
}

#[test]
fn rm_15_a_chain_behind_an_open_join_is_not_yet_handed() {
    let deadline = |e: i64| e + 100;
    // The chain at 70 (front 114) is inside the horizon 80; a booking at 45 fronts at 64, which meets neither chain.
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, deadline, 0).unwrap();
    bookings.accept(70, Some(40), 0, deadline, 80).unwrap();
    assert_eq!(bookings.accept(45, Some(1), 0, deadline, 80), Ok(()), "the chain at 70 is held while the one ending at 40 may still be joined");
    let mut expired = armed();
    expired.accept(0, Some(40), 0, deadline, 0).unwrap();
    expired.accept(70, Some(40), 0, deadline, 80).unwrap();
    assert_eq!(expired.accept(45, Some(1), 141, deadline, 80), Err(Overlap::Handed), "past the join deadline the chain at 70 may be handed");
}

#[test]
fn rm_16_first_free_moves_past_a_chain_inside_the_hand_over_horizon() {
    let mut bookings = armed();
    bookings.accept(5_000, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.first_free(2_000, true, 0, ON_TIME, 10_000), Some(5_044), "a start before the handable chain is refused, so the first free one is its front");
    assert_eq!(bookings.first_free(2_000, true, 0, ON_TIME, 5_000), Some(2_000), "a chain at the horizon is not handable: the instant is free");
    assert_eq!(bookings.first_free(5_044, true, 0, ON_TIME, 10_000), Some(5_044), "from the front on, the instant is free");
    let mut repeat = armed();
    repeat.accept(5_000, None, 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(repeat.first_free(2_000, true, 0, ON_TIME, 10_000), None, "a handable repeat holds every later sample");
}

#[test]
fn rm_16_stop_truncates_the_front_and_ends_the_join() {
    let mut repeat = armed();
    repeat.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    repeat.halt(50, 60);
    assert_eq!(repeat.accept(59, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    assert_eq!(repeat.accept(60, Some(5), 0, ON_TIME, UNHANDED), Ok(()));
    let mut finite = armed();
    finite.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    finite.halt(101, 110);
    assert_eq!(finite.accept(110, Some(5), 0, ON_TIME, UNHANDED), Ok(()), "the finite front is capped at the bound: 110 is free");
    assert_eq!(finite.accept(140, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the Stop cleared the join: the booking at the old end is a new chain");
    assert_eq!(finite.accept(143, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "its front is 159, not the joined 143");
    assert_eq!(finite.accept(146, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the booking at 140 is a standalone head (front 159), not a joined piece (front 145)");
}

#[test]
fn rm_15_a_booking_at_the_end_of_a_long_chain_joins_it_before_its_deadline() {
    // A booking at `e` received no later than `J(e)` joins the chain (its front is then the piece's end plus four), and one received after
    // `J(e)` is refused (RM-15).
    let deadline = |e: i64| e + 100;
    let mut on_time = armed();
    on_time.accept(0, Some(40), 0, deadline, UNHANDED).unwrap();
    assert_eq!(on_time.accept(40, Some(5), 140, deadline, UNHANDED), Ok(()), "received at J(e) = 140: it joins");
    assert_eq!(on_time.accept(48, Some(1), 140, deadline, UNHANDED), Err(Overlap::Booking), "the joined chain fronts at 49");
    assert_eq!(on_time.accept(49, Some(1), 140, deadline, UNHANDED), Ok(()));
    let mut late = armed();
    late.accept(0, Some(40), 0, deadline, UNHANDED).unwrap();
    assert_eq!(late.accept(40, Some(5), 141, deadline, UNHANDED), Err(Overlap::Booking), "received after J(e): refused");
}

#[test]
fn rm_16_a_stop_that_cuts_nothing_adds_no_region() {
    let mut idle = armed();
    idle.halt(10, 20);
    assert_eq!(idle.accept(10, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
    // A closed short head the Stop keeps keeps its own front (19), so the span up to it is held.
    let mut ended = armed();
    ended.accept(0, Some(1), 0, ON_TIME, UNHANDED).unwrap();
    ended.halt(10, 20);
    assert_eq!(ended.accept(10, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "its own front 19 is kept");
    assert_eq!(ended.accept(19, Some(1), 0, ON_TIME, UNHANDED), Ok(()));
    let mut cut = armed();
    cut.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    cut.halt(10, 20);
    assert_eq!(cut.accept(19, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "a Stop that cut the repeat leaves [10, 20)");
}

#[test]
fn rm_16_a_stop_lowers_a_finite_front_to_its_bound() {
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(10, 20);
    assert_eq!(bookings.accept(19, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the span before the bound is kept");
    assert_eq!(bookings.accept(20, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the span after the bound is not reserved: the Stop cut it");
}

#[test]
fn rm_16_a_stop_keeps_an_uncut_short_head_s_front() {
    // Sequence 4a (#82): a head of 1 to 3 samples starts at c - 1, so the Stop at c (start < c, kept) cuts nothing of it: its front stays
    // c + 3 + gap (c + 18 at 1 MS/s), not the bound c + 5.
    let c = 100;
    for gap in [15, 30] {
        for len in 1..=3 {
            let mut bookings = armed_with(gap);
            bookings.accept(c - 1, Some(len), 0, ON_TIME, UNHANDED).unwrap();
            bookings.halt(c, c + 5);
            for k in c + 3..c + 3 + gap {
                assert_eq!(bookings.accept(k, Some(3), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "gap {gap}, len {len}: {k} is inside the head's span");
            }
            assert_eq!(bookings.accept(c + 3 + gap, Some(3), 0, ON_TIME, UNHANDED), Ok(()), "gap {gap}, len {len}: c + 3 + gap is the head's own front, not the bound c + 5");
        }
    }
}

#[test]
fn rm_16_a_stop_that_cuts_a_short_head_adds_its_front_to_the_region() {
    // Sequence 4b (#82, Q1): the Stop at c - 1 drops the head of 1 at c - 1, which may have been handed (c - 1 < the handed bound c, with
    // the bound c + 5). Its own front c + 3 + gap reaches past the Stop's bound: the region is [c - 1, c + 3 + gap).
    let c = 100;
    for gap in [15, 30] {
        let mut bookings = armed_with(gap);
        bookings.accept(c - 1, Some(1), 0, ON_TIME, UNHANDED).unwrap();
        bookings.halt(c - 1, c + 5);
        for k in c - 1..c + 5 {
            assert_eq!(bookings.accept(k, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "gap {gap}: {k} is inside the dropped head's region");
        }
        for k in c + 5..c + 3 + gap {
            assert_eq!(bookings.accept(k, Some(3), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "gap {gap}: {k} is inside the region, not only before the bound");
        }
        assert_eq!(bookings.accept(c + 3 + gap, Some(3), 0, ON_TIME, UNHANDED), Ok(()), "gap {gap}: the dropped head's own front");
    }
}

#[test]
fn rm_16_a_stop_inside_a_32_sample_head_keeps_its_front() {
    // Sequence 5c (F1, Sol's counterexample): a closed head of 32 starts at 100 and the Stop at 101 (bound 106) keeps it: its payload was handed as
    // one send, so its front is 147, not the bound.
    let mut bookings = armed();
    bookings.accept(100, Some(32), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(101, 106);
    assert_eq!(bookings.accept(106, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "106 is inside the kept head's front 147");
    assert_eq!(bookings.accept(147, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the kept head's own front");
}

#[test]
fn rm_16_a_stop_caps_a_33_sample_head_at_the_bound() {
    // The length bound of a closed short head (m3): a head of 33 is not one, so the Stop caps its front (35) at its bound 20.
    let mut bookings = armed();
    bookings.accept(0, Some(33), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(1, 20);
    assert_eq!(bookings.accept(19, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the span before the bound is kept");
    assert_eq!(bookings.accept(20, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "capped at the bound, not its own front 35");
}

#[test]
fn rm_16_a_stop_keeps_a_short_head_s_front_with_joined_pieces() {
    // A head of at most 32 samples keeps its own front through a Stop, with pieces behind it (S2, Sol finding 1): the owner closes the
    // head at its end plus the gap, 165. C(150, 4) starts above the Stop's handed bound (141), so the owner has not handed it and the region
    // ends at B's own front, 165, not C's 169. The Stop at 141 does not cap it at 146.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(140, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(150, Some(4), 0, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(100, Some(40));
    bookings.halt(141, 146);
    assert_eq!(bookings.accept(145, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the span before the bound is kept");
    assert_eq!(bookings.accept(146, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "not capped at the bound: the head keeps its front 165");
    assert_eq!(bookings.accept(164, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the span up to B's own front is kept");
    assert_eq!(bookings.accept(165, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "C, joined behind B and above the handed bound, does not extend it to 169");
}

#[test]
fn rm_16_a_stop_keeps_a_short_head_s_own_front_past_its_joined_pieces() {
    // S2 (Sol finding 1): A(39 950, 40) is refused and leaves. B(39 990, 10) is the head and C(40 000, 4) joined behind it. The owner sends B as
    // a standalone closed short burst fronting at 40 015 (its end plus the gap, 15 samples at 1 MS/s, UR-22), and C fronts at 40 019. A Stop at 30 000 with bound 40 003
    // cancels B and C: the region reaches their front, so D(40 003, 40) received after the Stop is refused at receipt.
    let mut bookings = armed();
    bookings.accept(39950, Some(40), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(39990, Some(10), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40000, Some(4), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(39950, Some(40));
    bookings.halt(30000, 40003);
    assert_eq!(bookings.accept(40003, Some(40), 30000, ON_TIME, UNHANDED), Err(Overlap::Booking), "B's own front 40 015 is inside the region the Stop kept");
}

#[test]
fn rm_16_a_stop_cutting_a_dropped_short_head_stops_at_its_own_front_not_a_long_piece() {
    // Sol fix round, item 1: A(39 950, 40) is refused and leaves; B(39 990, 10) heads the chain with C(40 000, 100 000) joined behind it,
    // whose front is 140 004. The owner sends B standalone, fronting at 40 015, and C is late at dispatch. A Stop at 30 000 with bound 40 005
    // cancels B and C: the region ends at B's own front, so a booking inside it is refused and one after it is admitted as asked.
    let mut bookings = armed();
    bookings.accept(39950, Some(40), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(39990, Some(10), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40000, Some(100_000), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(39950, Some(40));
    bookings.halt(30000, 40005);
    assert_eq!(bookings.accept(40014, Some(40), 30000, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside B's own front 40 015");
    assert_eq!(bookings.accept(40015, Some(40), 30000, ON_TIME, UNHANDED), Ok(()), "B's own front ends the region");
    assert_eq!(bookings.accept(50000, Some(40), 30000, ON_TIME, UNHANDED), Ok(()), "C's front 140 004 does not hold the stream");
}

#[test]
fn rm_16_a_stop_keeping_a_short_head_keeps_its_own_front_not_a_long_piece() {
    // Sol fix round, item 1: the sequence of the test above, with a Stop at 39 995. B started before it and is kept, and the owner is idle
    // from B's front 40 010: a booking inside it is refused, and one at 50 000 is admitted as asked.
    let mut bookings = armed();
    bookings.accept(39950, Some(40), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(39990, Some(10), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40000, Some(100_000), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(39950, Some(40));
    bookings.halt(39995, 40003);
    assert_eq!(bookings.accept(40005, Some(40), 30000, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside B's own front 40 010, kept");
    assert_eq!(bookings.accept(50000, Some(40), 30000, ON_TIME, UNHANDED), Ok(()), "C's front 140 002 does not hold the kept head");
}

#[test]
fn rm_16_a_long_piece_handed_before_the_bound_does_not_extend_a_short_head() {
    // Sol fix round, item 1: the sequence of the dropped case, with a Stop at 30 000 and bound 40 012, so C(40 000, 100 000) starts below
    // the handed cursor (40 007). C is longer than a closed burst, so it is not a standalone short piece: the region ends at B's own front.
    let mut bookings = armed();
    bookings.accept(39950, Some(40), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(39990, Some(10), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.accept(40000, Some(100_000), 30000, ON_TIME, UNHANDED).unwrap();
    bookings.drop_piece(39950, Some(40));
    bookings.halt(30000, 40012);
    assert_eq!(bookings.accept(40014, Some(1), 30000, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside B's own front 40 015");
    assert_eq!(bookings.accept(40015, Some(40), 30000, ON_TIME, UNHANDED), Ok(()), "C does not extend the region to 140 004");
}

#[test]
fn rm_16_a_stop_caps_a_33_sample_head_with_a_short_piece_at_the_bound() {
    // Sol fix round, item 1: a head of 33 is not a short head, so a short piece behind it does not give the Stop a front past the bound. The
    // chain's front is 41 (its joined end 37 plus four) and the piece's own front 52 (33 + 4 + the gap 15), which starts below the handed
    // cursor 35 (the bound 40 less five): the front is capped at 40. Counted as a short head the chain would keep 41.
    let mut bookings = armed();
    bookings.accept(0, Some(33), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(33, Some(4), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(1, 40);
    assert_eq!(bookings.accept(39, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the span before the bound is kept");
    assert_eq!(bookings.accept(40, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "capped at the bound, not the joined end's front 41 or the short piece's 52");
}

#[test]
fn rm_13_repeat_boundary_is_the_first_boundary_at_or_after_the_bound() {
    assert_eq!(repeat_boundary(0, 10, 20), 20, "an equal bound is chosen");
    assert_eq!(repeat_boundary(0, 10, 21), 30, "one past the boundary takes the next");
    assert_eq!(repeat_boundary(100, 10, 50), 100, "an origin later than the bound");
    assert_eq!(repeat_boundary(0, 1, 21), 21);
    assert_eq!(repeat_boundary(0, 7, 15), 21, "a period that does not divide the distance");
}

#[test]
fn rm_37_a_switch_taken_at_the_abandoned_cursor_is_cancelled() {
    // S4 (Opus re-check): the owner takes the switch at boundary 100, and the repeat is abandoned at the same handed cursor. The new
    // waveform never starts, so the switch is cancelled, not applied.
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.switch(ActionId(2), 100, 10, true).is_ok());
    assert_eq!(bookings.abandon(100, 105 * 200), [Settled { id: ActionId(2), outcome: SwitchOutcome::Cancelled, boundary: 100 }]);
    assert_eq!(settled(&mut bookings), [(ActionId(2), SwitchOutcome::Cancelled)]);
}

#[test]
fn rm_16_switch_pending_until_the_boundary_is_reached() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.switch(ActionId(2), 100, 10, true).is_ok());
    assert!(bookings.switch(ActionId(3), 200, 99, true).is_err(), "pending before its boundary: a second switch is refused");
    bookings.advance(99);
    assert_eq!(settled(&mut bookings), [], "not applied before its boundary");
    bookings.advance(100);
    assert_eq!(settled(&mut bookings), [(ActionId(2), SwitchOutcome::Applied)], "applied at its boundary");
    assert_eq!(bookings.accept(100, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the new repeat holds its boundary");
    assert_eq!(bookings.accept(-20, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the old repeat ends at the boundary; its front is -6, before it");
}

#[test]
fn rm_16_advance_runs_inside_every_call() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.switch(ActionId(2), 100, 10, true).is_ok());
    bookings.prune(101);
    assert_eq!(settled(&mut bookings), [(ActionId(2), SwitchOutcome::Applied)], "a receipt at 101 applies the switch due at 100");
    assert!(bookings.switch(ActionId(3), 200, 101, true).is_ok(), "a second switch after the first boundary is not refused by a stale pending");
    bookings.halt(150, 160);
    assert_eq!(settled(&mut bookings), [(ActionId(3), SwitchOutcome::Cancelled)], "a Stop before its boundary cancels it");

    let mut direct = armed();
    direct.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(direct.switch(ActionId(2), 100, 10, true).is_ok());
    assert!(direct.switch(ActionId(3), 200, 101, true).is_ok(), "the switch itself applies the first switch due at 100");
    assert_eq!(settled(&mut direct), [(ActionId(2), SwitchOutcome::Applied)]);
}

#[test]
fn rm_16_a_stop_after_a_boundary_cuts_the_new_repeat_at_its_bound() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(bookings.switch(ActionId(2), 100, 10, true).is_ok());
    bookings.halt(150, 160);
    assert_eq!(settled(&mut bookings), [(ActionId(2), SwitchOutcome::Applied)], "a switch whose boundary the Stop has passed is applied");
    assert_eq!(bookings.accept(159, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the new repeat holds up to the bound");
    assert_eq!(bookings.accept(160, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "and no further");
}

#[test]
fn rm_16_switch_refused_unless_a_repeat_is_playing() {
    let mut finite = armed();
    finite.accept(0, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert!(finite.switch(ActionId(2), 100, 5, true).is_err(), "a finite burst");
    let mut booked = armed();
    booked.accept(50, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(booked.switch(ActionId(2), 100, 10, true).is_err(), "a repeat booked but not started");
    let mut stopped = armed();
    stopped.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    stopped.halt(20, 30);
    assert!(stopped.switch(ActionId(2), 100, 25, true).is_err(), "a stopped repeat");
    let mut playing = armed();
    playing.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert!(playing.switch(ActionId(2), 100, 0, true).is_ok(), "a repeat that has started");
}

/// What both Providers do for an untimed booking: place it, then accept it at the placed start.
fn book_untimed(bookings: &mut Bookings, at: i64, ordinary: i64, len: Option<i64>, now: i64) -> Result<i64, Overlap> {
    let start = bookings.place_untimed(at, ordinary, len, now, ON_TIME, UNHANDED).ok_or(Overlap::Booking)?;
    bookings.accept(start, len, now, ON_TIME, UNHANDED)?;
    Ok(start)
}

#[test]
fn rm_15_an_untimed_booking_is_placed_after_the_booking_it_meets() {
    // Sequence 3: B (len 10) at 100 is accepted, its front 125; an untimed A whose earliest start is 103 queues behind it, at 125.
    let mut bookings = armed();
    bookings.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(book_untimed(&mut bookings, 103, 0, Some(10), 0), Ok(125));
    assert_eq!(bookings.accept(125, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "the untimed booking is accepted at 125");
    assert_eq!(bookings.accept(110, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "B's span reaches 125");
}

#[test]
fn rm_15_place_tests_the_full_front() {
    // A candidate of len 40 at 100 has front 144, which a booking at 120 (front 145) meets though 100 is free: place jumps past its front, 145.
    let mut bookings = armed();
    bookings.accept(120, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.first_free(100, true, 0, ON_TIME, UNHANDED), Some(100));
    assert_eq!(bookings.place(100, Some(40), 0, ON_TIME, UNHANDED), Some(145));
}

#[test]
fn rm_15_an_untimed_booking_meeting_a_booking_beyond_the_horizon_is_placed_after_it() {
    // Re-check (a): B (10 samples, front 145) at 120 is beyond the hand-over horizon (110), so first_free passes 100; A (30 samples) at 100 has
    // front 145, which B's region meets. place moves A to B's front, 145; accepted at 100 it would be refused.
    let mut bookings = armed();
    bookings.accept(120, Some(10), 0, ON_TIME, 110).unwrap();
    assert_eq!(bookings.first_free(100, true, 0, ON_TIME, 110), Some(100));
    assert_eq!(bookings.place_untimed(100, 100, Some(30), 0, ON_TIME, 110), Some(145));
    assert_eq!(bookings.clone().accept(100, Some(30), 0, ON_TIME, 110), Err(Overlap::Booking));
    assert_eq!(bookings.accept(131, Some(30), 0, ON_TIME, 110), Err(Overlap::Booking), "131 is inside B's span");
    assert_eq!(bookings.accept(145, Some(30), 0, ON_TIME, 110), Ok(()));
}

#[test]
fn rm_16_untimed_booking_skips_the_stop_region() {
    // Shape 1: a repeat at 0 cut by a Stop at 10 with bound 20 keeps [0, 20); an untimed repeat at 12 starts at 20.
    let mut repeat = armed();
    repeat.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    repeat.halt(10, 20);
    assert_eq!(book_untimed(&mut repeat, 12, 12, None, 12), Ok(20));
    // Shape 2: a repeat at 15, cancelled by the Stop, adds the region [10, 20); an untimed burst at 12 starts at 20 (first_free sees it too).
    let mut region = armed();
    region.accept(15, None, 0, ON_TIME, UNHANDED).unwrap();
    region.halt(10, 20);
    assert_eq!(region.first_free(12, false, 12, ON_TIME, UNHANDED), Some(20));
    assert_eq!(book_untimed(&mut region, 12, 12, Some(5), 12), Ok(20));
}

#[test]
fn rm_15_an_untimed_repeat_on_a_playing_repeat_is_refused() {
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(book_untimed(&mut bookings, 5, 5, None, 5), Err(Overlap::Booking));
}

#[test]
fn rm_15_an_untimed_repeat_meeting_a_later_booking_starts_after_it() {
    // Re-check (b): a repeat from 50 would meet the booking at 100 (front 125), so it starts at 125, after that booking.
    let mut bookings = armed();
    bookings.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(book_untimed(&mut bookings, 50, 50, None, 0), Ok(125));
}

#[test]
fn rm_15_an_untimed_booking_after_a_padded_head_is_placed_at_end_plus_gap() {
    // Sequence 3: a head of 1 or 3 samples at 0 is padded to four and fronts at 19; an untimed burst of 5 samples at 2 is placed at 19.
    for len in [1, 3] {
        let mut bookings = armed();
        bookings.accept(0, Some(len), 0, ON_TIME, UNHANDED).unwrap();
        assert_eq!(book_untimed(&mut bookings, 2, 2, Some(5), 2), Ok(19), "len {len}: the padded end plus the gap");
    }
    // After a 33-sample head (a held tail, join deadline passed) it is placed at the end plus four.
    let mut long = armed();
    long.accept(0, Some(33), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(long.place_untimed(2, 2, Some(5), 0, |_| i64::MIN, UNHANDED), Some(37), "a head of 33 fronts at its end plus four");
}

#[test]
fn rm_16_an_untimed_booking_meeting_a_stop_region_from_before_the_stop_is_placed_after_it() {
    // Re-check (c): a Stop at 10 with bound 22 cuts the head of 3 at 15, whose own front is 34 (its padded end plus the gap): the region is
    // [10, 34). A burst at 8 of 5 samples has its front (28) in it and is placed at 34.
    let mut bookings = armed();
    bookings.accept(15, Some(3), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(10, 22);
    assert_eq!(book_untimed(&mut bookings, 8, 8, Some(5), 8), Ok(34));
}

#[test]
fn rm_16_a_timed_send_asap_moves_past_only_the_stop_region() {
    // A Stop at 10 with bound 22 cancels the head of 3 at 15, leaving the region [10, 34) (its own front): a timed booking at 12 moves to 34.
    let mut bookings = armed();
    bookings.accept(15, Some(3), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(10, 22);
    assert_eq!(bookings.region_move(12, Some(5), 0, ON_TIME, UNHANDED), Ok(34));
}

#[test]
fn rm_16_a_timed_send_asap_meeting_a_chain_is_refused() {
    // A chain at 0 of length 4 (front 19) lies over 12, and the region [10, 38) is there too: the booking stays at 12 and accept refuses it.
    let mut bookings = armed();
    bookings.accept(0, Some(4), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(19, Some(3), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(10, 25);
    assert_eq!(bookings.region_move(12, Some(5), 0, ON_TIME, UNHANDED), Ok(12));
    assert_eq!(bookings.accept(12, Some(5), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
}

#[test]
fn rm_16_a_moved_untimed_head_does_not_block_itself() {
    // At dispatch the head of 40 samples at 100 is moved to 100: its own span is removed before placing it.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.move_untimed(100, 100, Some(40)), Some(100));
    assert_eq!(bookings.move_untimed(100, 110, Some(40)), Some(110), "the head's old span is gone, so it moves to 110");
}

#[test]
fn rm_16_a_moved_untimed_head_goes_after_its_joined_pieces() {
    // Head H (40 at 100) with a joined piece P (10 at 140, joined front 152). Once H leaves, P is the only piece and at most 32
    // samples, so it is closed at its standalone front (165, S2): H, moved to a bound of 120, lands after that.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    bookings.accept(140, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.move_untimed(100, 120, Some(40)), Some(165));
}

#[test]
fn rm_15_a_front_past_the_clock_is_refused_without_a_panic_or_a_change() {
    // A start near i64::MAX has a front that overflows: place and region_move find no sample, and a move is refused with the record unchanged.
    let near = i64::MAX - 10;
    let mut bookings = armed();
    assert_eq!(bookings.place(near, Some(40), 0, ON_TIME, UNHANDED), None);
    assert_eq!(bookings.region_move(near, Some(40), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    bookings.accept(10, Some(5), 0, ON_TIME, UNHANDED).unwrap();
    let before = bookings.clone();
    assert_eq!(bookings.move_untimed(10, near, Some(40)), None);
    assert_eq!(bookings.move_start(10, near, Some(40)), Err(Overlap::Booking));
    assert_eq!(bookings, before);
}

#[test]
fn rm_15_place_moves_a_booking_past_a_later_booking_its_region_meets() {
    // A late SendAsap is placed with place: a candidate of 20 at 90 meets the booking at 100 (front 144) and moves to 144.
    let mut bookings = armed();
    bookings.accept(100, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.place(90, Some(20), 0, ON_TIME, UNHANDED), Some(144));
}

#[test]
fn rm_15_place_honours_a_join_point() {
    // A chain of 40 at 0 (joinable, end 40) takes a booking of 5 at 40: the join is not a conflict, so place keeps 40.
    let mut bookings = armed();
    bookings.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.place(40, Some(5), 0, ON_TIME, UNHANDED), Some(40));
}

fn txlist_read(list: &mut TxList<u32>) -> String {
    match list.read() {
        None => "none".into(),
        Some(Read::Booking(p)) => format!("book {p}"),
        Some(Read::Halt { reason, cancelled }) => format!("halt {reason:?} {:?}", cancelled.keys().collect::<Vec<_>>()),
    }
}

#[test]
fn txlist_reads_in_receipt_order() {
    let mut list = TxList::<u32>::default();
    list.book(1);
    list.halt(CancelReason::Stop);
    list.book(2);
    assert_eq!([txlist_read(&mut list), txlist_read(&mut list), txlist_read(&mut list), txlist_read(&mut list)], ["book 1", "halt Stop []", "book 2", "none"]);
}

#[test]
fn txlist_a_halt_cancels_exactly_what_was_read_before_it() {
    let mut list = TxList::<u32>::default();
    list.book(1);
    list.book(2);
    for _ in 0..2 {
        let Some(Read::Booking(p)) = list.read() else { panic!("a booking") };
        list.held.insert(i64::from(p), p);
    }
    list.halt(CancelReason::Stop);
    list.book(3);
    let Some(Read::Halt { cancelled, .. }) = list.read() else { panic!("a halt") };
    assert_eq!(cancelled.keys().copied().collect::<Vec<_>>(), [1, 2]);
    assert!(list.held.is_empty());
    assert!(matches!(list.read(), Some(Read::Booking(3))));
    assert!(list.held.is_empty());
}

#[test]
fn txlist_a_booking_read_after_a_halt_survives_it() {
    let mut list = TxList::<u32>::default();
    list.halt(CancelReason::Stop);
    list.book(9);
    assert!(matches!(list.read(), Some(Read::Halt { .. })));
    let Some(Read::Booking(p)) = list.read() else { panic!("a booking") };
    list.held.insert(9, p);
    assert_eq!(list.held.keys().copied().collect::<Vec<_>>(), [9]);
}

#[test]
fn txlist_unread_tells_what_is_waiting() {
    let mut list = TxList::<u32>::default();
    assert!(!list.unread());
    list.book(1);
    assert!(list.unread());
    list.read();
    assert!(!list.unread());
    list.halt(CancelReason::Loss);
    assert!(list.unread());
    list.read();
    assert!(!list.unread());
}

#[test]
fn txlist_carries_the_reason() {
    let mut list = TxList::<u32>::default();
    for reason in [CancelReason::Stop, CancelReason::ProviderStop, CancelReason::Loss] {
        list.halt(reason);
    }
    assert_eq!([txlist_read(&mut list), txlist_read(&mut list), txlist_read(&mut list)], ["halt Stop []", "halt ProviderStop []", "halt Loss []"]);
}

#[test]
fn txlist_default_needs_no_payload_default() {
    struct NoDefault;
    let list = TxList::<NoDefault>::default();
    assert!(!list.unread() && list.held.is_empty());
}

#[test]
fn txlist_held_is_by_first_sample() {
    let mut list = TxList::<u32>::default();
    for k in [30, 10, 20] {
        list.held.insert(k, 0);
    }
    assert_eq!(list.held.keys().copied().collect::<Vec<_>>(), [10, 20, 30]);
}

#[test]
fn rm_15_closed_short_gap_is_fifteen_microseconds_or_three_samples() {
    // On a 200 MHz root, root ticks per sample N: 1, 2, 2.857 (N = 70), 10, 100 and 200 MS/s.
    for (ratio, gap) in [(200, 15), (100, 30), (70, 43), (20, 150), (2, 1500), (1, 3000)] {
        assert_eq!(closed_short_gap(200_000_000, ratio, 1), gap, "ratio {ratio}");
    }
    // `ideal` at 100 kS/s on a 1 GHz root: 1.5 samples, ceil 2, floored to three.
    assert_eq!(closed_short_gap(1_000_000_000, 10_000, 1), 3);
    // A Mock-shaped ratio: root 10^9, N = 70 gives 350 root ticks a sample.
    assert_eq!(closed_short_gap(1_000_000_000, 350, 1), 43);
    // A non-integral ratio: `ideal` at 3 MS/s on a 1 GHz root is 1000/3 ticks a sample; a dropped denominator would give 15.
    assert_eq!(closed_short_gap(1_000_000_000, 1000, 3), 45);
}

#[test]
fn rm_15_an_unarmed_record_refuses_a_closed_short_head() {
    let mut bookings = Bookings::unarmed();
    assert_eq!(bookings.accept(0, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "no rate, no guard: a short head is refused, not booked with none");
    assert_eq!(bookings.accept(0, Some(33), 0, ON_TIME, UNHANDED), Ok(()), "a head of 33 needs no gap");
    assert_eq!(Bookings::unarmed().accept(100, None, 0, ON_TIME, UNHANDED), Ok(()), "nor does a repeat");
    bookings.arm(15);
    assert_eq!(bookings.accept(200, Some(10), 0, ON_TIME, UNHANDED), Ok(()), "armed, the same head is accepted");
}

#[test]
fn rm_15_arming_again_replaces_the_gap() {
    let mut bookings = armed_with(15);
    bookings.accept(0, Some(4), 0, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(18, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "armed at 15: the front is 4 + 15");
    bookings.prune(19);
    bookings.arm(30);
    bookings.accept(100, Some(4), 100, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(133, Some(1), 100, ON_TIME, UNHANDED), Err(Overlap::Booking), "re-armed at 30: the front is 104 + 30");
    assert_eq!(bookings.accept(134, Some(1), 100, ON_TIME, UNHANDED), Ok(()));
    bookings.prune(135);
    bookings.arm(15);
    bookings.accept(200, Some(4), 200, ON_TIME, UNHANDED).unwrap();
    assert_eq!(bookings.accept(218, Some(1), 200, ON_TIME, UNHANDED), Err(Overlap::Booking), "back at 15: the front is 204 + 15");
    assert_eq!(bookings.accept(219, Some(1), 200, ON_TIME, UNHANDED), Ok(()));
}

#[test]
fn rm_16_a_dropped_short_head_inside_five_of_the_bound_was_not_handed() {
    // The Stop at 50 has bound 100, so the handed cursor is at most 95: a dropped head starting at 96 was never handed and leaves only
    // [50, 100), not its own front 115 (a bound read as `stop - 3` would call it handed).
    let mut bookings = armed();
    bookings.accept(96, Some(1), 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(50, 100);
    assert_eq!(bookings.accept(99, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "inside the Stop's region");
    assert_eq!(bookings.accept(100, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "the region ends at the bound");
}

#[test]
fn rm_16_a_stop_cutting_a_repeat_fronts_at_the_cursor_plus_five() {
    // Sequence 5: a Stop with no window at cursor c cuts a repeat; its two-zero close ends at c + 2 and the front is c + 5.
    let c = 100;
    let mut bookings = armed();
    bookings.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    bookings.halt(c, stop_sample_with_no_window(c));
    assert_eq!(bookings.accept(c + 4, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "c + 4 is inside the close and its gap");
    assert_eq!(bookings.accept(c + 5, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "c + 5 is the front");
}

/// The Stop bound at sample `c` with no in-flight window, on the identity clock.
fn stop_sample_with_no_window(c: i64) -> i64 {
    ezsdr_radio::staging::stop_sample(c, 0, |t| t)
}

// TxBook (note 49 stage E2a): the book writes the record, the list, `playing` and the open booking.

const PLAYING: Playing = Playing { id: ActionId(1), origin: 0, period: 10, started: true };

/// A booking as the book reads it: a payload number, the Action that booked it and whether it is a switch's new waveform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Pl {
    n: u32,
    id: Option<ActionId>,
    switch: bool,
}

impl Booked for Pl {
    fn id(&self) -> Option<ActionId> {
        self.id
    }

    fn is_switch(&self) -> bool {
        self.switch
    }
}

/// Payload `n`, booked by no Action the book knows.
fn pl(n: u32) -> Pl {
    Pl { n, id: None, switch: false }
}

/// The new waveform of switch `id`.
fn switch_pl(n: u32, id: u64) -> Pl {
    Pl { n, id: Some(ActionId(id)), switch: true }
}

/// An empty book of a Provider whose repeats start at their first issue (UHD).
fn new_book() -> TxBook<Pl> {
    TxBook::new(Started::AtFirstIssue)
}

/// The `TxBook` reach of a Stop at sample `c`: a bound of 200 root ticks a sample, and the Stop region `[c, c + STOP_FRONT)`.
fn reach(c: i64) -> (i64, Option<(i64, i64)>) {
    (c * 200 + STOP_FRONT * 200, Some((c, c + STOP_FRONT)))
}

#[test]
fn txbook_a_repeat_booking_sets_playing_and_a_finite_one_does_not() {
    let mut book = new_book();
    book.record.arm(GAP_1M);
    book.record.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    book.play(false, ActionId(1), 0, 10);
    book.book(pl(7), Some(20_000));
    assert_eq!(book.playing, None, "a finite booking is not a repeat");
    book.record.accept(400, None, 0, ON_TIME, UNHANDED).unwrap();
    book.play(true, ActionId(1), 0, 10);
    book.book(pl(8), None);
    assert_eq!(book.playing, Some(Playing { started: false, ..PLAYING }), "a repeat becomes the playing one, not started before its first issue");
    book.play(false, ActionId(3), 0, 10);
    book.book(pl(9), Some(30_000));
    assert_eq!(book.playing, Some(Playing { started: false, ..PLAYING }), "a later finite booking leaves it");
    let mut mock = TxBook::<Pl>::new(Started::AtAcceptance);
    mock.play(true, ActionId(1), 0, 10);
    assert_eq!(mock.playing, Some(PLAYING), "a Provider whose repeats start at acceptance");
    let mut manual = armed();
    manual.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    manual.book(Some(20_000));
    manual.accept(400, None, 0, ON_TIME, UNHANDED).unwrap();
    manual.book(None);
    manual.book(Some(30_000));
    assert_eq!(book.record, manual, "the record's end is booked with the booking");
    let read: Vec<u32> = std::iter::from_fn(|| book.list.read()).filter_map(|read| if let Read::Booking(b) = read { Some(b.n) } else { None }).collect();
    assert_eq!(read, [7, 8, 9], "the list receives the bookings in order");
}

#[test]
fn txbook_stop_applies_the_record_stop_and_its_region() {
    let mut book = new_book();
    book.record.arm(GAP_1M);
    book.record.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    book.record.book(None);
    let mut manual = book.record.clone();
    book.stop(reach(20));
    manual.stop(20 * 200 + STOP_FRONT * 200);
    manual.halt(20, 20 + STOP_FRONT);
    assert_eq!(book.record, manual, "stop is the record's stop and its halt");
    let mut no_region = new_book();
    no_region.record.arm(GAP_1M);
    no_region.record.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    no_region.record.book(None);
    let mut manual = no_region.record.clone();
    no_region.stop((4_000, None));
    manual.stop(4_000);
    assert_eq!(no_region.record, manual, "no region without a first sample");
}

/// The root tick bound of an abandon at sample `c` on the identity clock of 200 root ticks a sample: `c + STOP_FRONT`.
fn abandon_bound(c: i64) -> i64 {
    (c + STOP_FRONT) * 200
}

/// A repeat R (id 1) at `start`, booked.
fn repeat_at(start: i64) -> Bookings {
    let mut r = armed();
    r.accept(start, None, 0, ON_TIME, UNHANDED).unwrap();
    r.book(None);
    r
}

/// R at 0 with switch 2 at boundary 10 on its grid, taken at receipt.
fn switch_pending() -> Bookings {
    let mut r = repeat_at(0);
    assert!(r.switch(ActionId(2), 10, 0, true).is_ok());
    r
}

/// The same switch applied by the record (device time reached 10).
fn switch_applied() -> Bookings {
    let mut r = switch_pending();
    r.advance(10);
    r
}

/// Records whose abandon at the given cursors equals today's (`cancel_switch_from`, then a Stop at the cursor with no window), each by
/// name, with whether a pending switch is cancelled first (today's `cancel_switch_from`: boundary 10 at or after the cursor).
fn abandon_rows() -> Vec<(&'static str, Bookings, Vec<i64>)> {
    let mut stopped = repeat_at(0);
    stopped.stop(1_000);
    stopped.halt(30, 30 + STOP_FRONT);
    let mut finite = armed();
    finite.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    finite.book(Some(8_000));
    let mut before = armed();
    before.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    before.book(Some(8_000));
    before.accept(100, None, 0, ON_TIME, UNHANDED).unwrap();
    before.book(None);
    vec![
        ("R alone", repeat_at(0), vec![5, 10, 15]),
        ("R open after a Stop", stopped, vec![5, 10, 15]),
        ("finite alone", finite, vec![5, 10, 15]),
        ("pending S", switch_pending(), vec![5, 10, 15]),
        ("applied S, abandoned past its boundary", switch_applied(), vec![15]),
        // c == r: the repeat's own first send took nothing, so its chain goes and the Stop region [r, r + 5) stays (today's answer).
        ("R abandoned at its own start", repeat_at(10), vec![10]),
        ("R behind a finite booking, at its own start", before, vec![100]),
    ]
}

/// Today's abandon of `record` at `c` (E2a-1): `cancel_switch_from(c)`, then a Stop at `c` with no window.
fn abandon_today(record: &Bookings, pending_at: Option<i64>, c: i64) -> Bookings {
    let mut today = record.clone();
    if pending_at.is_some_and(|b| b >= c) {
        assert!(today.cancel_switch());
    }
    today.stop(abandon_bound(c));
    today.halt(c, c + STOP_FRONT);
    today
}

#[test]
fn txbook_abandon_equals_a_stop_at_the_cursor_except_the_named_differences() {
    // P10: with no chain after the abandoned repeat, the scoped abandon is today's except in the cells below.
    for (name, record, cursors) in abandon_rows() {
        let pending_at = name.contains("pending S");
        for c in cursors {
            let mut book = TxBook { record: record.clone(), ..new_book() };
            book.abandon(true, Some(ActionId(1)), c, abandon_bound(c));
            let mut today = abandon_today(&record, pending_at.then_some(10), c);
            assert_eq!(book.record, today, "{name}, cursor {c}");
            assert_eq!(book.record.take_settled(), today.take_settled(), "{name}, cursor {c}: the settled switches");
        }
    }
    // (c) An applied switch at or after the cursor never started: settled cancelled, whether its payload was held or is the unsent open
    // booking (the record does not know which; today only a held payload's row was cancelled, by uhd-tx). The record is today's.
    let mut book = TxBook { record: switch_applied(), ..new_book() };
    book.abandon(true, Some(ActionId(1)), 5, abandon_bound(5));
    let mut today = abandon_today(&switch_applied(), None, 5);
    assert_eq!(settled(&mut today), [(ActionId(2), SwitchOutcome::Applied)], "today's record leaves it applied");
    assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Cancelled)], "(c): cancelled");
    assert_eq!(book.record, today, "past the cursor's footprint: the record is today's");
    // (b) R + applied S with c <= b < c + 5: R's front is set to its footprint c + 5, where today a separate Stop region [c, c + 5) is
    // left with R's front at b, so a SendAsap booking in [b, c + 5) is refused at receipt instead of moved. P12 has the cell.
    for c in [7, 10] {
        let mut book = TxBook { record: switch_applied(), ..new_book() };
        book.abandon(true, Some(ActionId(2)), c, abandon_bound(c));
        let mut today = abandon_today(&switch_applied(), None, c);
        assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Cancelled)], "cursor {c}");
        assert_eq!(settled(&mut today), [(ActionId(2), SwitchOutcome::Applied)], "cursor {c}");
        // Today's record with R's front (its first chain's, at b = 10) set to c + STOP_FRONT; the Stop region [c, c + 5) stays.
        let want = format!("{today:?}").replacen("front: 10,", &format!("front: {},", c + STOP_FRONT), 1);
        assert_ne!(want, format!("{today:?}"), "cursor {c}: today's R fronts at b");
        assert_eq!(format!("{:?}", book.record), want, "cursor {c}: the named difference (b)");
    }
}

#[test]
fn txbook_abandon_keeps_a_later_booking_its_chain_playing_and_busy_end() {
    // P11 (#80): R plays from 0 and a Stop at 30 cuts it (front 35, the value rule at the Stop's bound); B is accepted after the Stop.
    // The owner's send fails at cursor 20: R ends at its footprint 25, and B keeps its chain, its busy end and, as a repeat, `playing`.
    let stopped = || {
        let mut r = repeat_at(0);
        r.stop(35 * 200);
        r.halt(30, 30 + STOP_FRONT);
        r
    };
    let b_repeat = Playing { id: ActionId(3), origin: 300, period: 10, started: true };
    for (case, len) in [("B finite", Some(10)), ("B repeat", None)] {
        let mut record = stopped();
        record.accept(300, len, 0, ON_TIME, UNHANDED).unwrap();
        record.book(len.map(|len| (300 + len) * 200));
        let playing = if len.is_none() { b_repeat } else { PLAYING };
        let mut book = TxBook { record, playing: Some(playing), ..new_book() };
        book.abandon(true, Some(ActionId(1)), 20, abandon_bound(20));
        assert_eq!(book.record.accept(300, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "{case}: B's chain stays");
        assert_eq!(book.record.accept(24, Some(1), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "{case}: R's footprint is [0, 25)");
        assert_eq!(book.record.clone().accept(25, Some(1), 0, ON_TIME, UNHANDED), Ok(()), "{case}: 25 is free");
        match len {
            Some(_) => {
                assert_eq!(book.record.stageable(310 * 200 - 1), Err(Refusal::TxBusy { until: Until::At(310 * 200) }), "{case}: busy until B's end");
                assert_eq!(book.playing, None, "{case}: R was the playing repeat");
            }
            None => {
                assert_eq!(book.record.stageable(i64::MAX), Err(Refusal::TxBusy { until: Until::Open }), "{case}: B is open");
                assert_eq!(book.playing, Some(b_repeat), "{case}: B stays the playing repeat");
            }
        }
    }
    // With no later booking, the value rule is capped at the abandon's bound (today's).
    let mut book = TxBook { record: stopped(), playing: Some(PLAYING), ..new_book() };
    book.abandon(true, Some(ActionId(1)), 20, abandon_bound(20));
    assert_eq!(book.record.stageable(abandon_bound(20)), Ok(()));
    assert_eq!(book.record.stageable(abandon_bound(20) - 1), Err(Refusal::TxBusy { until: Until::At(abandon_bound(20)) }));
    assert_eq!(book.playing, None);
}

#[test]
fn txbook_abandon_at_a_switch_boundary_extends_the_old_repeat_to_its_footprint() {
    // P12 (F3, RM-15): R plays from 0 and its switch S at 10 is applied in the record; the owner abandons at c (S never started). The
    // footprint of R is [0, c + 5): a SendAsap booking at b + 1 meets R's chain and stays where it asked, so it is refused (today it met
    // only the Stop region and moved past it). The pending twin (S taken, not applied) answers the same, as today.
    for (case, record, c) in [("applied, c == b", switch_applied(), 10), ("applied, c < b < c + 5", switch_applied(), 7), ("pending, c == b", switch_pending(), 10)] {
        let mut book = TxBook { record, ..new_book() };
        book.abandon(true, Some(ActionId(1)), c, abandon_bound(c));
        assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Cancelled)], "{case}");
        assert_eq!(book.record.region_move(11, Some(100), 0, ON_TIME, UNHANDED), Ok(11), "{case}: not moved");
        assert_eq!(book.record.accept(11, Some(100), 0, ON_TIME, UNHANDED), Err(Overlap::Booking), "{case}: refused at receipt");
        assert_eq!(book.record.accept(c + STOP_FRONT, Some(100), 0, ON_TIME, UNHANDED), Ok(()), "{case}: the footprint ends at c + 5");
    }
}

#[test]
fn txbook_a_finite_abandon_changes_nothing() {
    // P13 (KL-39, RM-15): a finite booking abandoned at its cursor keeps its whole region, and a switch pending on a repeat behind it is
    // left as it is: the record, its settled switches and `playing` are unchanged.
    let mut record = armed();
    record.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    record.book(Some(40 * 200));
    record.accept(100, None, 0, ON_TIME, UNHANDED).unwrap();
    record.book(None);
    assert!(record.switch(ActionId(2), 120, 100, true).is_ok());
    let mut book = TxBook { record: record.clone(), playing: Some(PLAYING), ..new_book() };
    book.abandon(false, Some(ActionId(5)), 20, abandon_bound(20));
    assert_eq!((&book.record, book.playing), (&record, Some(PLAYING)));
    assert!(settled(&mut book.record).is_empty());
}

#[test]
fn txbook_halt_cancels_an_open_booking_that_issued_nothing_and_keeps_one_that_did() {
    // P14 (#85, RM-16): a halt read from the list cancels the held bookings and an open booking with nothing issued; an open booking that
    // has sent keeps what it handed (nothing handed is recalled), so it is not cancelled and stays open for the executor to end.
    for (case, sent) in [("nothing issued", false), ("a send completed", true)] {
        let mut book = open_book();
        if sent {
            book.issue(TxSend { count: 10, device_count: 10, ends: false });
            assert_eq!(book.complete(Outcome::Accepted(10)), Settle::Sent);
        }
        book.list.held.insert(300, pl(9));
        book.list.halt(CancelReason::Stop);
        let Some(Read::Halt { reason, cancelled }) = book.read() else { panic!("{case}: the halt") };
        assert_eq!(reason, CancelReason::Stop);
        let want: BTreeMap<i64, Pl> = if sent { [(300, pl(9))].into() } else { [(100, pl(7)), (300, pl(9))].into() };
        assert_eq!(cancelled, want, "{case}");
        assert_eq!(book.open.is_some(), sent, "{case}: only a booking that sent stays open");
    }
    let mut book = open_book();
    assert_eq!(book.cancel_all(), [(100, pl(7))].into(), "Provider::stop's cancellation is the same rule");
}

#[test]
fn txbook_dispatched_verdicts_equal_the_record_calls() {
    let mut base = armed();
    for (at, len) in [(100, Some(10)), (130, Some(10)), (400, None)] {
        base.accept(at, len, 0, ON_TIME, UNHANDED).unwrap();
        base.book(len.map(|len| (at + len) * 200));
    }
    // A move that fits, untimed and timed; and a refused move, which drops the piece.
    let mut book = TxBook { record: base.clone(), ..new_book() };
    let mut manual = base.clone();
    assert_eq!(book.move_untimed(None, 130, 160, Some(10)), manual.move_untimed(130, 160, Some(10)));
    assert_eq!(book.record, manual, "untimed move");
    assert_eq!(book.move_start(None, 130, 104, Some(10)).is_ok(), manual.move_start(130, 104, Some(10)).is_ok(), "the first move");
    let mut book = TxBook { record: base.clone(), ..new_book() };
    let mut manual = base.clone();
    assert_eq!(book.move_start(None, 130, 170, Some(10)), manual.move_start(130, 170, Some(10)));
    assert_eq!(book.record, manual, "timed move");
    let mut book = TxBook { record: base.clone(), ..new_book() };
    let mut manual = base.clone();
    let refused = manual.move_start(130, 105, Some(10));
    assert!(refused.is_err());
    manual.drop_piece(130, Some(10));
    assert_eq!(book.move_start(None, 130, 105, Some(10)), refused);
    assert_eq!(book.record, manual, "a refused move drops the piece");
    // Drop: the piece leaves the record (no switch waits on a repeat dropped at dispatch: UR-38).
    let mut book = TxBook { record: base.clone(), ..new_book() };
    let mut manual = base.clone();
    manual.drop_piece(100, Some(10));
    book.drop_piece(100, Some(10));
    assert_eq!(book.record, manual, "drop");
}

#[test]
fn txbook_clock_ended_cancels_the_pending_switch_and_configure_refused_drops_the_piece() {
    let mut repeat = armed();
    repeat.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    repeat.book(None);
    repeat.switch(ActionId(2), 10, 0, true).unwrap();
    let mut book = TxBook { record: repeat, ..new_book() };
    book.clock_ended();
    assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Cancelled)], "W7: the pending switch is cancelled");
    let mut book = new_book();
    book.record.arm(GAP_1M);
    book.record.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    book.record.book(Some(110 * 200));
    assert_eq!(book.record.accept(100, Some(10), 0, ON_TIME, UNHANDED), Err(Overlap::Booking));
    book.drop_piece(100, Some(10));
    assert_eq!(book.record.accept(100, Some(10), 0, ON_TIME, UNHANDED), Ok(()), "W6: the refused configure's region is gone");
}

/// A book whose record holds a repeat with a pending switch, `playing` set, and booking 7 open at sample 100.
fn open_book() -> TxBook<Pl> {
    let mut record = armed();
    record.accept(0, None, 0, ON_TIME, UNHANDED).unwrap();
    record.book(None);
    record.switch(ActionId(2), 10, 0, true).unwrap();
    let mut book = TxBook { record, playing: Some(PLAYING), ..new_book() };
    book.take(pl(7), 100);
    book
}

#[test]
#[should_panic(expected = "send in flight")]
fn txbook_issue_twice_panics() {
    let mut book = open_book();
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
}

#[test]
#[should_panic(expected = "no send in flight")]
fn txbook_complete_without_issue_panics() {
    // The second `complete` of one send is a `complete` with none issued.
    let mut book = open_book();
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
    assert_eq!(book.complete(Outcome::Accepted(10)), Settle::Sent);
    book.complete(Outcome::Accepted(10));
}

#[test]
fn txbook_outcomes_give_the_cursors_the_executor_gives_today() {
    // UR-22: 10 samples from 100, handed as 12 (padding); a held-tail send hands 0. The cursor is what the device has nothing past.
    let send = TxSend { count: 10, device_count: 12, ends: false };
    for (case, send, outcome, settle, next) in [
        ("whole", send, Outcome::Accepted(12), Settle::Sent, Some((110, false))),
        ("whole, its last", TxSend { ends: true, ..send }, Outcome::Accepted(12), Settle::Sent, None),
        ("a held tail only", TxSend { count: 1, device_count: 0, ends: false }, Outcome::Accepted(0), Settle::Sent, Some((101, false))),
        ("short", send, Outcome::Accepted(5), Settle::Abandon { cursor: 105 }, Some((100, true))),
        ("failed", send, Outcome::Failed, Settle::Abandon { cursor: 112 }, Some((100, true))),
        ("the tail before it failed", send, Outcome::TailShort, Settle::Abandon { cursor: 100 }, Some((100, true))),
    ] {
        let mut book = open_book();
        book.issue(send);
        assert_eq!(book.complete(outcome), settle, "{case}");
        assert_eq!(book.open.as_ref().map(|open| (open.next, open.first)), next, "{case}");
        if next.is_some() {
            book.issue(send);
        }
    }
}

#[test]
fn txbook_take_untake_and_end_open_keep_one_open_booking_and_change_no_record() {
    let mut book = open_book();
    let (record, playing) = (book.record.clone(), book.playing);
    book.untake();
    assert!(book.open.is_none());
    assert_eq!(book.list.held.get(&100), Some(&pl(7)), "the booking is held again at its first sample");
    let held = book.list.held.remove(&100).unwrap();
    book.take(held, 100);
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
    assert_eq!(book.end_open(), Some(pl(7)));
    assert!(book.open.is_none() && book.end_open().is_none());
    assert_eq!((&book.record, book.playing), (&record, playing), "ending the open booking with no outcome changes no record");
    assert!(settled(&mut book.record).is_empty(), "and settles no switch");
    book.take(pl(8), 200);
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
}

// E2a-3: switches decided at receipt (RM-37, UR-38).

/// A book whose record plays R (id 1, period 10, from 0), `playing` R with `started`.
fn playing_book(started: bool) -> TxBook<Pl> {
    TxBook { record: repeat_at(0), playing: Some(Playing { started, ..PLAYING }), ..new_book() }
}

#[test]
fn txbook_receive_switch_takes_the_boundary_at_receipt() {
    // P16: the boundary is the first of R's grid at or after both `at` and the bound; the switch is pending (not applied before device
    // time reaches it, applied once it does) and is the playing repeat from receipt, on its own grid, not started before its own issue.
    let mut book = playing_book(true);
    assert_eq!(book.switch_boundary(3, 15), Some(20));
    assert_eq!(book.receive_switch(ActionId(2), 7, 3, 15, false, 0), Ok(SwitchAccepted { boundary: 20, requested_target: Some(3) }));
    assert_eq!(book.playing, Some(Playing { id: ActionId(2), origin: 20, period: 7, started: false }));
    book.advance(19);
    assert_eq!(settled(&mut book.record), [], "pending before its boundary");
    book.advance(20);
    assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Applied)], "applied at its boundary");
    let mut later = playing_book(true);
    assert_eq!(later.receive_switch(ActionId(2), 7, 33, 15, false, 0).map(|accepted| accepted.boundary), Ok(40), "an `at` past the bound decides");
    assert_eq!(new_book().switch_boundary(3, 15), None, "no repeat plays");
}

#[test]
fn txbook_a_refused_switch_changes_nothing() {
    // P17: each refusal, in its order (no playing repeat, then a pending switch, then not started), leaves the record, its settled
    // switches, `playing` and the list as they were.
    let mut stopped = repeat_at(0);
    stopped.halt(20, 25);
    let mut finite = armed();
    finite.accept(0, Some(40), 0, ON_TIME, UNHANDED).unwrap();
    let mut pending = playing_book(true);
    pending.receive_switch(ActionId(2), 10, 0, 15, false, 0).unwrap();
    let cases = [
        ("none playing", new_book(), SwitchRefusal::NoPlayingRepeat),
        ("a finite burst", TxBook { record: finite, playing: Some(PLAYING), ..new_book() }, SwitchRefusal::NoPlayingRepeat),
        ("a stopped repeat", TxBook { record: stopped, playing: Some(PLAYING), ..new_book() }, SwitchRefusal::NoPlayingRepeat),
        ("a pending switch, itself not started", pending, SwitchRefusal::PendingSwitch),
        ("not started", playing_book(false), SwitchRefusal::NotStarted),
    ];
    for (case, mut book, refusal) in cases {
        book.list.held.insert(500, pl(5));
        book.list.book(pl(6));
        let (record, playing) = (book.record.clone(), book.playing);
        assert_eq!(book.receive_switch(ActionId(3), 10, 0, 15, false, 1), Err(refusal), "{case}");
        assert_eq!((&book.record, book.playing), (&record, playing), "{case}");
        assert_eq!(book.list.held, [(500, pl(5))].into(), "{case}");
        assert!(matches!(book.list.read(), Some(Read::Booking(p)) if p == pl(6)) && !book.list.unread(), "{case}");
    }
}

#[test]
fn txbook_receive_switch_requested_target_rule() {
    // P18 (#86): a timed switch the bound pushed past the grid point its `at` alone gives records its `at`; one on that grid point, or
    // untimed, records none.
    let requested = |at: i64, untimed: bool| playing_book(true).receive_switch(ActionId(2), 10, at, 15, untimed, 0).unwrap().requested_target;
    assert_eq!(requested(3, false), Some(3), "pushed from 10 to 20");
    assert_eq!(requested(10, false), Some(10), "on a grid point, pushed to the next");
    assert_eq!(requested(20, false), None, "at the bound's own grid point");
    assert_eq!(requested(12, false), None, "before the bound, but the bound's grid point is its own");
    assert_eq!(requested(33, false), None, "past the bound, between grid points: the grid point of `at`");
    assert_eq!(requested(-5, false), Some(-5), "an `at` before the receipt");
    assert_eq!(requested(3, true), None, "untimed: no requested instant");
}

#[test]
fn txbook_cancel_switch_removes_its_payload_from_held_and_unread_and_clears_playing_once() {
    // P19 (A1, F1): once the record has settled a switch cancelled (here as `Bookings::abandon` does), the book's cancellation ends it as
    // the playing repeat and removes its payload, held or unread; the other entries keep their order. A second call changes nothing.
    for unread in [false, true] {
        let mut book = playing_book(true);
        book.receive_switch(ActionId(2), 10, 0, 15, false, 0).unwrap();
        book.list.held.insert(50, pl(5));
        book.list.book(pl(6));
        if unread {
            book.list.book(switch_pl(1, 2));
        } else {
            book.list.held.insert(20, switch_pl(1, 2));
        }
        book.list.halt(CancelReason::Stop);
        book.list.book(pl(7));
        assert!(book.record.cancel_switch(), "unread {unread}: the record settles it");
        assert!(book.cancel_switch(ActionId(2)), "unread {unread}");
        assert_eq!(settled(&mut book.record), [(ActionId(2), SwitchOutcome::Cancelled)], "unread {unread}");
        assert_eq!(book.playing, None, "unread {unread}");
        assert_eq!(book.list.held, [(50, pl(5))].into(), "unread {unread}");
        assert!(!book.cancel_switch(ActionId(2)), "unread {unread}: nothing left to cancel");
        assert_eq!(settled(&mut book.record), [], "unread {unread}");
        let read: Vec<String> = std::iter::from_fn(|| book.list.read())
            .map(|read| match read {
                Read::Booking(p) => format!("book {}", p.n),
                Read::Halt { reason, .. } => format!("halt {reason:?}"),
            })
            .collect();
        assert_eq!(read, ["book 6", "halt Stop", "book 7"], "unread {unread}");
    }
}

#[test]
fn txbook_dispatched_move_moves_the_playing_origin_of_that_repeat_only() {
    // P20 (#88): a repeat moved at dispatch repeats from its new start; a move of another booking, or a refused move, leaves the grid.
    let mut base = armed();
    base.accept(100, Some(10), 0, ON_TIME, UNHANDED).unwrap();
    base.accept(400, None, 0, ON_TIME, UNHANDED).unwrap();
    let r = Playing { id: ActionId(3), origin: 400, period: 10, started: false };
    let mut book = TxBook { record: base.clone(), playing: Some(r), ..new_book() };
    assert_eq!(book.move_start(Some(ActionId(1)), 100, 130, Some(10)), Ok(()));
    assert_eq!(book.playing, Some(r), "another booking moved");
    assert!(book.move_start(Some(ActionId(3)), 400, 105, None).is_err());
    assert_eq!(book.playing, Some(r), "a refused move");
    let mut book = TxBook { record: base.clone(), playing: Some(r), ..new_book() };
    assert_eq!(book.move_start(Some(ActionId(3)), 400, 455, None), Ok(()));
    assert_eq!(book.playing, Some(Playing { origin: 455, ..r }), "a timed move");
    let mut book = TxBook { record: base, playing: Some(r), ..new_book() };
    assert_eq!(book.move_untimed(Some(ActionId(3)), 400, 461, None), Some(461));
    assert_eq!(book.playing, Some(Playing { origin: 461, ..r }), "an untimed move");
}

#[test]
fn txbook_issue_starts_the_playing_repeat_and_a_switch_s_own_payload() {
    // P21 (D3, UR-38): the first issue of the playing repeat starts it, and a switch is the playing repeat from receipt, started only by
    // the first issue of its own payload; another booking's issue starts neither.
    let mut book = TxBook { record: repeat_at(0), ..new_book() };
    book.play(true, ActionId(1), 0, 10);
    book.take(Pl { n: 0, id: Some(ActionId(5)), switch: false }, 0);
    book.issue(TxSend { count: 10, device_count: 10, ends: true });
    assert_eq!(book.playing.map(|playing| playing.started), Some(false), "another booking's issue");
    book.complete(Outcome::Accepted(10));
    assert_eq!(book.receive_switch(ActionId(2), 10, 0, 15, false, 0), Err(SwitchRefusal::NotStarted));
    book.take(Pl { n: 1, id: Some(ActionId(1)), switch: false }, 10);
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
    assert_eq!(book.playing.map(|playing| playing.started), Some(true), "R's first issue");
    assert_eq!(book.complete(Outcome::Accepted(10)), Settle::Sent);
    assert!(book.receive_switch(ActionId(2), 10, 0, 15, false, 0).is_ok());
    assert_eq!(book.playing.map(|playing| (playing.id, playing.started)), Some((ActionId(2), false)));
    book.advance(20);
    assert_eq!(book.receive_switch(ActionId(3), 10, 0, 35, false, 21), Err(SwitchRefusal::NotStarted), "S applied in the record, not issued");
    book.end_open();
    book.take(switch_pl(2, 2), 20);
    book.issue(TxSend { count: 10, device_count: 10, ends: false });
    assert_eq!(book.playing.map(|playing| (playing.id, playing.started)), Some((ActionId(2), true)), "S's own first issue");
    assert!(book.receive_switch(ActionId(3), 10, 0, 35, false, 21).is_ok());
}
