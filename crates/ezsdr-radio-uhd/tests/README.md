# UHD software tests and timing prerequisites

Run the default regression suite without a USRP or libuhd:

```sh
CARGO_TARGET_DIR="$HOME/.cache/cargo-target/Ez-SDRv4" cargo test --workspace
```

The library's controlled-clock tests prove booking order, cold-change exclusion,
first-dispatch late policies, underflow recovery and RX stop idempotence without
requiring millisecond host wake-ups. The deliberate starvation test primes an
untimed fake burst before starving it, so its setup has no timed-start deadline.
Spec 22's VH-8 layers 2 and 3 (`rm_26_the_timeline_against_uhd_control`,
`rm_26_the_timeline_against_uhd_rx`) compare uhd-control's booking and uhd-rx's
carrying out of a plan with `ezsdr_radio::timeline` on 1 000 seeded sequences
each, on the same controlled clock.

`fake.rs` is a smoke set: it exercises the complete coordinator and the bench
rehearsals with a wall-clock FakeDevice. Its clock advances while the host is descheduled, as a
device's does. Exact capture indices and uninterrupted loopback require on-time
stream starts and no overflow/underflow. A loaded host may miss the 3 ms delivery
allowance, 2 ms device lead or held-tail deadline. The fake's clock and hardware
envelope must not be slowed or relaxed to make these tests pass.

Before the B3/B4/B6/B7 waveform/index assertions and the single-burst assertion,
`require_stream_timing` checks measured stream events and expired-timeline
rejections. `TIMING_PRECONDITION_UNMET` reports the events, timestamps, payloads
and raw device async reports. It means the uninterrupted/on-time prerequisites
failed; it **does not prove host scheduling was the cause** or exclude a runtime
regression. The test still fails. Successful prerequisites are followed by the
original strict assertions. No test is ignored or retried by this diagnostic.

An intentional stress probe is:

```sh
CARGO_TARGET_DIR="$HOME/.cache/cargo-target/Ez-SDRv4" \
  cargo test -p ezsdr-radio-uhd --test fake -- --test-threads=128
```

This is a host timing/load probe, not a portable requirement that every developer
machine service 128 concurrent radios within the UHD deadlines. Keep its complete
failure output. Rerun a failing case alone on an idle host, and compare it with
the controlled-clock regressions. Persistent failures, and failures without
measured missed prerequisites, still require investigation. Passing an isolated
rerun does not erase the stress failure. Hardware deadline measurements remain
Phase 8 bench work.
