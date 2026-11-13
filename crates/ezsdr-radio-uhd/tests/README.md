# UHD software tests and timing prerequisites

Run the default regression suite without a USRP or libuhd:

```sh
CARGO_TARGET_DIR="$HOME/.cache/cargo-target/Ez-SDRv4" cargo test --workspace
```

The library's controlled-clock tests prove booking order, the exclusion of a
configuration from a burst's device call,
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

`provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`
also runs the Phase 8 stage 3c-1 TX start-lead, overflow-gap and TX-throughput groups.
It checks their observations, classifications and fake-record round trip; overflow records
the full gap and inferred restart-after-resume estimate as observations without supplying a
comparable value. The rehearsal does not assert that a timing candidate passes or that a
measured value has a particular magnitude. Their hardware evidence is collected by
`hw_measurements_record`.

No test's verdict may depend on when the test thread runs. The device's time moves while
every Provider thread is idle (M3, M5), so a test thread that reads a time or observes
state before its next operation or assertion can be descheduled while the device's time
advances. Such a sequence is one of: (1) book it in the Spec, before the Run starts;
(2) assert something that holds whatever the clock did, never a read time as the verdict's
basis; (3) where the delay cannot change the verdict, book a burst that may be moved with
the `send_asap_and_flag` policy (`start_repeat`); or (4) where the timing window is the
property itself, pin it in a controlled-clock unit test. Booking "well ahead" only lowers
the chance and is not (2). There is no exception: the two driving rigs (`UhdRig` in the
acceptance crate and `Direct` in `fake.rs`) hold their device clock while the test thread
works, so a delay of the test thread cannot move a receipt (below).

The clock rules are pinned by the unit tests in `src/host.rs` (one per rule M1–M7 and
§4.7, each with a wall-clock bound), and the Authority's manual-clock behaviour by the
tests in `src/authority.rs` (the first anchor exact, a read wider than 1 ms discarded, a
`host.monotonic` wait and schedule judged on real host time, the root and host timelines
kept apart). Every manual-clock test arms one shared
wall watchdog (`common/watchdog.rs`, plan 30 §7): a clock a mutant freezes aborts the
process with the test's name once `EZSDR_TEST_WATCHDOG_SECS` (default 120 s) passes,
instead of hanging the suite, after writing an `EZSDR-INFRA:` marker line straight to stderr
(`common/infra.rs`: libtest's capture would lose a printed line to the abort); a watchdog that
cannot start fails its test at once. The same marker is written, once, before the one panic of a
rig that ends in a wait on a real-time bound (the parity rig's constructor and its `wait_for_events`, the `Direct` rig's Authority and its
`until`) and of a Run that meets one (the Run wrapper, below), and `EZSDR-TIMING:` names a test that a serial re-run
may pass. Each driving rig's thread is a participant of its device clock (`HostClock::drive`, held
for the rig's life; the guard drops before the Authority, so the clock runs while `uhd-clock` is joined), so a delay of the test thread does not move a receipt: between its waits the
clock stands. Two rules make a receipt a function of the poll grid alone (`common/rig.rs`; note 51 stage 1c): a wait that ends on an instant the
test chose (`UhdRig::settle`, `wait_until`; `Direct::settle`, `wait_until`, `start`) ends with a one-microsecond wait, so every participant due at that instant has run
before the test thread continues; and the rig's Action queue withholds an Action until the device clock is strictly later than its push
(`rig::Stamped`), so an Action is received at the first poll strictly after its push, whichever order the test thread and a poll
woke in at a shared instant, even one the microsecond itself lands on (`uhd_rig_a_wait_that_ends_on_a_poll_instant_…`, `direct_rig_a_wait_that_ends_on_a_poll_instant_…`). A participant
due one microsecond *after* the wait may or may not have run: a test reads an event or a call by its own device time, or waits for it
with `wait_for_events` / `until`, which end on an observed state (no microsecond), whose horizon is device time (a plain failure) and whose real-time bound writes the marker. A `Direct` test on a
real clock (the two `Gated` ones) is outside this guarantee: the rig registers nothing there. The mutation runner reads both from the output (`mutate.py`'s docstring): a marker makes a row `ERROR` unless
the row declares `"kill": "bound"`. `infra_fixtures/` holds the tests `mutate.py --e2e` runs to
pin that; they return at once unless `EZSDR_INFRA_FIXTURE` names them. The real clock stays where a real host deadline
is the point: the Authority's own diagnostics, `ur_07_now_is_monotonic_across_a_reanchor` (a diagnostic, since the guard is
pinned by a unit test in `src/authority.rs`; the re-anchor's bracket error is real
time's), the device's own time against the process's clock
(`ur_33_a_real_fake_s_time_starts_at_its_own_creation`), and the intentionally
wedged thread (`FakeFault::TxBlocks`). Such a test builds a `ClockMode::Real` fake, and the mutation runner marks it: with `EZSDR_REAL_CLOCK_PROBE` set
(the baseline of every row), `FakeDevice::new` on the real clock writes `EZSDR-CLASS: real-clock`. A mutation row whose baseline is marked must declare
`"kill": "bound"` (the row's filter selects a test whose verdict is a real-time quantity, or one a held thread would hang on a manual clock): otherwise `mutate.py` makes it `ERROR`
whatever its outcome (killed, survived or recorded as equivalent) and `gate.py` rejects it again from the row line (`real-clock (undeclared)`). A test whose subject is not real time
takes the manual clock instead: the `rig*` helpers of `src/provider/test_support.rs` and `common::fake()` do. `ur_21_late_policies_on_the_device_lead` now runs
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
a participant holds the clock until the test releases it after its checks (a unit test that
runs the owner's code on its own thread holds the clock with `test_support::clock_guard`, which
registers that thread as a participant for as long as the guard lives), and
`ur_24_the_device_queue_order_is_the_effective_order` builds its late command on the
controlled clock of `src/provider/control.rs`. The only short wall verdicts left are the
medians of the real-clock latency diagnostics `ur_07_schedule_wakes_a_waiting_next_wakeup` and
`ur_07_cancel_wakes_a_waiting_next_wakeup`, whose wakes are pinned on a held clock by unit
tests; the remaining wall bounds, of seconds, only guard that something ends (a stop past an
intentionally wedged thread, a shutdown, a Run).

**The Run wrapper** (`common/run.rs`; note 51 stage 2a). Every Run a test drives on a FakeDevice is a `Run`, never a raw `RunHandle`: it
is created with `Run::connect` / `Run::start_spec` (`ezsdr-acceptance`'s `uhd.rs` includes the same file with `#[path]`), and the scan in
`crates/ezsdr-acceptance/tests/governance.rs` fails any reference to `connect` / `start_spec_run` (a call, an import, a function value) or a `RunHandle` outside `run.rs`, a time read
(`thread::sleep`, `Instant::now`, `.elapsed()`, `recv_timeout(`, `.time_now(`, `.host()`) inside a `#[test]` fn that is not on its per-fn
allow list (a helper fn may read time; the scan sees tokens, not time sources reached through a helper), `Expect::Any` outside its three
named loops, a `.handle(` in `protocol.rs` outside `fn handle`, and `assemble_on` (a FakeDevice assembly) in any `ezsdr-acceptance` test file but `uhd.rs`, so
`support::session_run_with`'s raw `connect` cannot carry one. The wrapper checks how the Run ended at its creation, when `submit`,
`advance_to`, `wait_for` or `run_until_end` returns, at `finish`, and at `Drop`. **A read-only call (`state`, `now`, `events`, ...) checks
nothing**: the Run reaches `CleanedUp` only inside a call that settles it, so a bound that cuts in the background (a Policy end's
`stop_early` on the data thread) shows at the next of those calls, at `finish` or at `Drop`. A real-time bound (KC-12a, KC-21a, a cleanup
step cut at RS-8a, a Provider join that timed out) writes the `EZSDR-INFRA:` marker once and then panics once, so a late assertion cannot
hide it; an end the test did not declare is a panic with no marker (a failure the code under test caused). A test declares what it
expects: `Expect::Refused` (refused at validate, prepare or arm), `Expect::Policy("<event>")` (ended by that Policy event) or, for a named
bench loop that only records how its Run ended, `Expect::Any`; none of them exempts a bound. `join_is_subject()` exempts the Provider join
and nothing else, for a test whose subject is the join. A Run a checked call found ended is kept as its
Manifest, and answers from it (`Err(Ended)`, `state`, `events`, `finish`). A Run dropped without `finish()` is finished and judged like
`finish`; while the thread is unwinding it only writes the marker of a bound and never panics again (a second panic aborts). A bound is
read from the primary end and from every later end the Kernel kept in `termination.also`. A test that provokes a bound on purpose wraps the call
in `capture_markers(&["join"], ...)` naming that bound, because the marker of a passing test would read as a bound that expired; the marker of
any other bound is still written, and a test that expects no bound captures nothing.
`EZSDR_RIG_DELAY_MS` (milliseconds; unset means no delay) with `EZSDR_RIG_DELAY_AT` (a comma list of `now`, `advance`, `submit`, `server`,
or `all`) makes the calling thread sleep after the wrapper's `now`/`start_instant`, `submit` and waiting calls, and after `protocol.rs`'s
`handle`, as a descheduled test thread would; it is a diagnostic for finding tests whose verdict depends on the test thread's timing, it
gates and kills nothing, and no gate or bench run sets it. The wrapper does not see when the test thread acts: a test whose verdict depends on it fails by its own
assertion under the delay (stage 2b moved those it could: a booking made untimed or `send_asap`, the Stop's own log stamp, the window on the `Direct` rig). Nine rehearsals judge their
trials through `common::Rehearsal` (note 51 stage 2c, §5.3b): a trial that carries evidence of host lateness (a `radio.TIME_ERROR` with cause `late`, or a Kernel log
stamp past its deadline; `iii::late_evidence`) is not judged, and `finish()` writes the `EZSDR-TIMING:` marker and fails once, so `mutate.py` and `gate.py` re-run the test
alone, up to twice (three marked runs: a red gate step, a mutation row `ERROR`). A trial without evidence is judged as before, and a Kernel refusal beside evidence is a plain
failure. The three `rehearsal_b7_*` tests carry their own tolerance and are unchanged. `rx_start_lead_group_runs_on_the_fake` reads `radio.LATE_COMMAND` /
`START_LEAD_SHORT`, not `radio.TIME_ERROR`, and the Kernel log bounds its restart gap only from above, so a delayed test thread can turn a SHORT stage into a LATE one: the Run rehearsal
therefore does not assert that a SHORT candidate exists (note 51 stage 3d), and `a_restart_stage_reports_what_the_events_say` covers the events-to-stage decoding (SHORT, LATE, association, the reported row) on synthetic events with no timing.

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
(120 s) is the mutation shards' bound, below the runner's 180 s timeout.

This is a host timing/load probe, not a portable requirement that every developer
machine service 128 concurrent radios within the UHD deadlines. Keep its complete
failure output. Rerun a failing case alone on an idle host, and compare it with
the controlled-clock regressions. Persistent failures, and failures without
measured missed prerequisites, still require investigation. Passing an isolated
rerun does not erase the stress failure. Hardware deadline measurements remain
Phase 8 bench work.
