//! RM-21: `ezsdr_radio::staging`, which receive value a Provider stages and what a `start_rx` takes (note 47 stage 3).

use std::collections::BTreeMap;

use ezsdr_kernel::spec::{Key, Value};
use ezsdr_radio::staging::{Busy, Refusal, Staging, Until};

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
const IDLE: fn(u32) -> Busy = |_| Busy::default();

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
    let mut busy = Busy::default();
    busy.book(Some(100));
    let stream_0_busy = |stream: u32| if stream == 0 { busy } else { Busy::default() };
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

// RM-21, transmit form (note 47 stage 4): `Busy` is the value rule of one transmit stream, in root ticks.

#[test]
fn rm_21_a_transmit_value_at_the_exclusive_end_is_accepted_and_one_sample_before_refused() {
    let mut busy = Busy::default();
    busy.book(Some(100));
    assert_eq!(busy.stageable(100), Ok(()));
    assert_eq!(busy.stageable(99), Err(Refusal::TxBusy { until: Until::At(100) }));
}

#[test]
fn rm_21_a_transmit_value_after_the_end_is_accepted() {
    let mut busy = Busy::default();
    busy.book(Some(100));
    assert_eq!(busy.stageable(1_100), Ok(()));
    assert_eq!(Busy::default().stageable(0), Ok(()), "an idle stream takes a value");
}

#[test]
fn rm_21_a_transmit_value_is_refused_while_a_booked_burst_has_not_ended() {
    let mut busy = Busy::default();
    busy.book(Some(100));
    assert_eq!(busy.stageable(50), Err(Refusal::TxBusy { until: Until::At(100) }));
}

#[test]
fn rm_21_the_last_booked_burst_decides() {
    let mut busy = Busy::default();
    busy.book(Some(10));
    busy.book(Some(20));
    assert_eq!(busy.stageable(15), Err(Refusal::TxBusy { until: Until::At(20) }), "the later end counts");
    busy.book(Some(5));
    assert_eq!(busy.stageable(15), Err(Refusal::TxBusy { until: Until::At(20) }), "an earlier end does not shorten it");
}

#[test]
fn rm_21_a_transmit_value_is_refused_while_a_repeat_plays_and_taken_after_its_stop() {
    let mut busy = Busy::default();
    busy.book(None);
    assert_eq!(busy.stageable(1_000), Err(Refusal::TxBusy { until: Until::Open }));
    busy.book(Some(50));
    assert_eq!(busy.stageable(1_000), Err(Refusal::TxBusy { until: Until::Open }), "a later booking does not close a repeat");
    busy.stop(100, 10);
    assert_eq!(busy.stageable(109), Err(Refusal::TxBusy { until: Until::At(110) }));
    assert_eq!(busy.stageable(110), Ok(()));
}

#[test]
fn rm_21_a_stop_bounds_an_open_repeat_by_the_in_flight_window() {
    let mut busy = Busy::default();
    busy.book(None);
    busy.stop(100, 10);
    assert_eq!(busy.stageable(109), Err(Refusal::TxBusy { until: Until::At(110) }));
}

#[test]
fn rm_21_a_stop_never_extends_a_booked_end() {
    let mut busy = Busy::default();
    busy.book(Some(5));
    busy.stop(3, 10);
    assert_eq!(busy.stageable(4), Err(Refusal::TxBusy { until: Until::At(5) }));
    assert_eq!(busy.stageable(5), Ok(()));
}

#[test]
fn rm_21_a_stop_leaves_an_idle_stream_idle() {
    let mut busy = Busy::default();
    busy.stop(10, 5);
    assert_eq!(busy.stageable(0), Ok(()));
}

#[test]
fn rm_21_a_transmit_refusal_reasons_name_the_busy_end_or_the_repeat() {
    let at = Refusal::TxBusy { until: Until::At(110) }.reason();
    let open = Refusal::TxBusy { until: Until::Open }.reason();
    assert!(at.starts_with("RM-21:") && at.contains("110"), "{at}");
    assert!(open.starts_with("RM-21:") && open.contains("repeating") && open.contains("Stop"), "{open}");
}

// RM-21, transmit form (note 47 stage 4): a transmit value is staged when its stream's `Busy` allows it at the receipt.

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
    let mut busy = Busy::default();
    busy.book(Some(100));
    let booked = |_: u32| busy;
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
