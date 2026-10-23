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
rehearsals through the real coordinator. Its `FakeDevice`s run on the **manual**
host clock (plan 30 §4.4): the device's time stops while one of the Provider's
threads — `uhd-rx`, `uhd-tx`, `uhd-control`, `uhd-clock` — is busy, and advances at
real speed only while every one of them is idle (M1–M7). Host descheduling therefore
cannot move the device's time past a deadline, and the smoke set no longer proves that
the Provider catches up with real time: that is Phase 8's bench work. The fake's clock
and the hardware envelope are still not slowed or relaxed to make a test pass.

No test's verdict may depend on when the test thread runs. The device's time moves while
every Provider thread is idle (M3, M5), so a test thread that reads a time or observes
state before its next operation or assertion can be descheduled while the device's time
advances. Such a sequence is one of: (1) book it in the Spec, before the Run starts;
(2) assert something that holds whatever the clock did, never a read time as the verdict's
basis; (3) where the delay cannot change the verdict, book a burst that may be moved with
the `send_asap_and_flag` policy (`start_repeat`); or (4) where the timing window is the
property itself, pin it in a controlled-clock unit test. Booking "well ahead" only lowers
the chance and is not (2).

The clock rules are pinned by the unit tests in `src/host.rs` (one per rule M1–M7 and
§4.7, each with a wall-clock bound), and the Authority's manual-clock behaviour by the
tests in `src/authority.rs` (the first anchor exact, a read wider than 1 ms discarded, a
`host.monotonic` wait and schedule judged on real host time, the root and host timelines
kept apart). Every manual-clock test arms one shared
wall watchdog (`common/watchdog.rs`, plan 30 §7): a clock a mutant freezes aborts the
process with the test's name once `EZSDR_TEST_WATCHDOG_SECS` (default 120 s) passes,
instead of hanging the suite; a watchdog that cannot start fails its test at once. The
real clock stays where a real host deadline is the point: the Authority's own
diagnostics, `ur_07_now_is_monotonic_across_a_reanchor` (a diagnostic, since the guard is
pinned by a unit test in `src/authority.rs`; the re-anchor's bracket error is real
time's), the device's own time against the process's clock
(`ur_33_a_real_fake_s_time_starts_at_its_own_creation`), and the intentionally
wedged thread (`FakeFault::TxBlocks`). `ur_21_late_policies_on_the_device_lead` now runs
on the manual clock and asserts the booking's verdicts only; the repeat's re-decision at
uhd-tx's first hand-over — a real interval — is pinned deterministically by the unit test
`ur_21_first_dispatch_rechecks_all_late_policies` (plan 30 stage A). The tests a loaded host
could fail went the same way (plan 30 D2). The fake's raw cases — a timed stop, a stopped
stream's tail and its whole timeout, a request cut short by a stop, and the transmit and report
cases (timed starts, an empty end-of-burst, an underflow and its recovery, and per-stream
reports) — are unit tests in `src/device.rs` whose setup and device calls all run as
participants of the fake's clock. `ur_07_no_device_read_follows_the_authority_s_drop` asserts only that no device read
follows the Authority's drop, with no 50 ms bound: the drop's promptness is pinned by
`ur_07_dropping_the_authority_ends_uhd_clock_while_the_manual_clock_is_stopped` in
`src/authority.rs`. No short wall interval decides a functional property: where one needs the
device's clock to stand still — the Authority's wakes, its drop, its `host.monotonic` waits —
a participant holds the clock until the test releases it after its checks, and
`ur_24_the_device_queue_order_is_the_effective_order` builds its late command on the
controlled clock of `src/provider/control.rs`. The only short wall verdicts left are the
medians of the real-clock latency diagnostics `ur_07_schedule_wakes_a_waiting_next_wakeup` and
`ur_07_cancel_wakes_a_waiting_next_wakeup`, whose wakes are pinned on a held clock by unit
tests; the remaining wall bounds, of seconds, only guard that something ends (a stop past an
intentionally wedged thread, a shutdown, a Run).

Before the B3/B4/B6/B7 waveform/index assertions and the single-burst assertion,
`require_stream_timing` checks measured stream events and expired-timeline
rejections. `TIMING_PRECONDITION_UNMET` reports the events, timestamps, payloads
and raw device async reports. It means the uninterrupted/on-time prerequisites
failed; it **does not prove host scheduling was the cause** or exclude a runtime
regression. The test still fails. Successful prerequisites are followed by the
original strict assertions. No test is ignored or retried by this diagnostic.

An intentional stress probe is:

```sh
CARGO_TARGET_DIR="$HOME/.cache/cargo-target/Ez-SDRv4" EZSDR_TEST_WATCHDOG_SECS=900 \
  cargo test -p ezsdr-radio-uhd --test fake -- --test-threads=128
```

The stress probe raises the watchdog's bound on purpose: under heavy
oversubscription a single smoke test legitimately takes far longer than its
normal ~50 s, and the probe is measuring load failures, not hangs. The default
(120 s) is the mutation shards' bound, below `mutate.py`'s 180 s timeout.

This is a host timing/load probe, not a portable requirement that every developer
machine service 128 concurrent radios within the UHD deadlines. Keep its complete
failure output. Rerun a failing case alone on an idle host, and compare it with
the controlled-clock regressions. Persistent failures, and failures without
measured missed prerequisites, still require investigation. Passing an isolated
rerun does not erase the stress failure. Hardware deadline measurements remain
Phase 8 bench work.
