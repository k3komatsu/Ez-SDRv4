# Note 51 stage 2b — test plan (read-only run, 2026-11-13)

Tree: `main` at `829b7b0` (stage 2a landed). Nothing in the tree was edited. Probes ran in a copy under `/tmp` (deleted), building into `~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`. Mutants were applied only to that copy, never through `mutate.py`; the rows still have to run through `remote_mutate.py`. Line numbers are of `829b7b0`. Ev: **V** = read or run and observed, **I** = inferred.

Not touched: `staging.rs`, `provider/tx.rs` and its tests of the `~/local_work/Ez-SDRv4-e3` lane (#99). The plan edits none of those files. (Two rows below, J317/J318, have `provider/tx.rs` as their *mutated* file; that is the row's source anchor, not an edit.)

## 0. Summary

| | Count | Tests |
|---|---|---|
| 2b, a test moves or loses a non-subject assertion | 5 | `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (split: window half to `Direct`), `ur_21_s18_…`, `ur_23_a_burst_after_a_stop_targets_the_one_clock`, `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window`, `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` (`uhd.rs`) |
| 2b, an allow-listed time-reading Run test loses its time read | 2 | `ur_29_a_silent_stream_is_a_lost_device`, `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` |
| 2b, tooling | 1 | the scan does not see `support/mod.rs::session_run_with` (2a re-check note) |
| stays as it is (stage 3) | 1 | `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` |
| to 2c (need `receipts`) | 13 | `n47s5_front/long_close/repeat_switch/untimed/held_tail_and_stop/one_sample_after_a_burst_refusal/stop_end_rehearsal_on_the_fake` (7), `s1_short_first_sends_rehearsal_on_the_fake`, `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `rx_start_lead_group_runs_on_the_fake`, `rehearsal_b7_refusal_holds_on_a_two_board_layout`, `rehearsal_b7_session_loopback`, `rehearsal_b7_session_loopback_on_one_obx` |

**Two corrections to the 2a record (V).** (1) The sweep's changed set is **14** tests (13 `fake` + 1 `uhd`), not 15: `n51-2a-sweep.md` lists 13 distinct `fake` names (union of the "Failing sets by config" table = the table of changed verdicts = 13; the `rx_start_lead_group…` row carries its module path in one table and not in the other), and `n51-2a-dispositions.md` / note 51 "段 2a の後" say "14 `fake`". (2) Rows N47B-19 and N47B-36 (`rx_start_lead_group_runs_on_the_fake`) mutate **test code** (`measurement.rs`: `trial.late = false`; the sign in `configure_lead_left`), not the Provider. They are harness rows like N47S5-177, not "lateness killers" in the sense of tables §6.1.

## 1. The set

### 1.1 Tests that move in 2b

Evidence for "why": the 2a sweep (`n51-2a-sweep.md`, injections `now`/`advance`/`submit` × 5/20/60/200 ms) and, for the moves, the same sweep re-run on the probe version of each new test (V; only 20 and 200 ms were run for `now`/`advance`/`submit`, the full 12 configs run at the step).

**T1 `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (fake.rs:1930). Class R2 (+R1-tx), needs a window. Fails today in all 12 sweep configs (V).**
- Why it depends on when the test thread acts: the first burst is `send` (`drop_and_flag`) at `tx_at(+10 ms)` read from `run.now()`; a delay before the submit makes it late at receipt, it is dropped (no configure, no refusal) and the `rejected` pair at `:1963` differs. The early value after `wait(12 ms)` must also land inside the burst's 10–20 ms span, the corrected value after it, which a delay between the waits moves.
- Planned move: split into two tests.
  - **T1a (stays in the Run, same name, so row N47S4-56's filter is unchanged):** `value(REFUSED)` admitted; the first burst is **untimed** `send` (no lead, no `now` read); `wait(60 ms)` (past any untimed placement + length, so the corrected value is always after the burst's booked end, whatever the delay: a receipt is before `submit` returns, KC-21a); `value(CORRECTED)` admitted; next burst untimed; `wait(60 ms)`; assert `rejected == [(tx_burst, first_id)]` (the Kernel's dispatched id equals the Provider's id: the half only the Run can show), nothing sent before the corrected value, `apply tx … freq=3000000000` before the next `tx_send`, same tx clock. The early-value / window parts go.
  - **T1b (new, `Direct`, name must not contain T1a's name because row filters are substrings):** `ur_21_a_refused_configure_keeps_its_burst_s_span_until_its_booked_end_on_the_direct_rig`. `Direct::build_with(FakeConfig{faults:[TxApplyRefused(0, 2e9)]}, &[("radio.tx.c0.stream", 0)], Vec::new(), {"rx":[],"tx":["0:A:ubx:0"]}, inputs)`, `settle(50 ms)`, `update("radio.tx.c0.frequency_hz", REFUSED, Cold, None)`, `burst_in(…, ms(10))` of 10 000 samples, `settle(12 ms)`, `update(CORRECTED)` (early), `settle(1.5 ms)`, `settle(9 ms)`, `update(CORRECTED)` (after the end), `settle(1 ms)`, `burst_in(…, ms(10))` of 2 000 samples, `settle(30 ms)`. Assert: `COMMAND_REJECTED` kinds/ids `[(tx_burst, id of the burst), (update_parameter, id of the early value)]` (the `Pushed` queue numbers Actions from 1, so ids are exact), nothing sent before the second burst, `apply tx 0 rate=- freq=3000000000` before the next `tx_send`, one tx SampleClock.
- Probe (V): T1b passes 3/3 with no delay and 3/3 with a 200 ms `std::thread::sleep` after **every** push and settle (the test thread is a participant; the clock stands). T1a passes 3/3 and in all 12 sweep configs `{now,advance,submit}×{5,20,60,200}` (V).
- What the pair still proves: (a) Kernel id = Provider id on a refused configure (wrong: the Provider reports `None`/another id → T1a's `rejected` assert fails; probe on mutant N47S4-56: T1a dies at the `rejected` assert, left `[(tx_burst, Null)]`, V; T1b does **not** die on it, it does not assert ids against the Kernel); (b) a refused burst's span stays counted to its booked end: an early value is refused, a value at the end is taken, then the corrected value configures the next burst (wrong: the span is dropped at the refusal → the early value is accepted → `rejected` has 1 entry; probe on the N47S4-13b mutant: T1b dies at the `rejected` assert, V; the unit tests `ur_21_a_refused_burst_stays_counted_until_its_booked_end` (tx.rs) and `ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` keep the same rules on the controlled clock). What is lost: the window half no longer passes through the Kernel's `update_parameter` dispatch, only through the Provider's `UhdRadio` (INFERRED not to matter: the Kernel does not read the span).

**T2 `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error` (fake.rs:2082). Class R1-tx (non-subject). Fails at `now-20/60/200` today (V).**
- Why: the first `start_repeat` is timed at `tx_at(+10 ms)` (`send_asap`); a late receipt moves it and adds a `TIME_ERROR{send_asap}`, which the subject assert `time_errors().is_empty()` (`:2090`) rejects.
- Move: book the first repeat **untimed** (`send(&mut run, "start_repeat", None, …)`), as the tables §3.2 say. The second (untimed) repeat is refused as an overlap exactly as before: at its receipt (a few ms after the first's) the first is booked and placed ~12 ms ahead, as the timed one was.
- Still proves: RM-15/16: an untimed repeat received on a playing/booked repeat is refused with the overlap reason, with no `TIME_ERROR` at all (the assert is stronger now: no timed booking exists), one `sob=true` on the device. Wrong behaviour that fails it: the second repeat is accepted (2 `sob`) or placed with a `TIME_ERROR`. Probe: passes at `now/advance/submit × 20/200` (V). No mutation row selects it (V: no row's filter matches).

**T3 `ur_23_a_burst_after_a_stop_targets_the_one_clock` (fake.rs:2268). Class R1-tx (non-subject). Fails at `now-20/60/200` today (V).**
- Why: `send` (`drop_and_flag`) at `tx_at(+10 ms)` after `wait(5 ms)`; a delayed read drops it late (`"UR-21: the late policy dropped the burst"` is in `rejections()`, `:2282`).
- Move (tables §3.2): book the burst after the Stop as `start_repeat` (`send_asap_and_flag`, same `tx_at(+10 ms)`): a late burst is moved (a `TIME_ERROR`, not a `rejected` row). **Keep `assert!(rejections().is_empty())`** and the clock count.
- Why `:2282` must stay (re-check C2 of tables §1): it is the only assert that kills **N47S4-41** (`stream.clock = None` at the Stop). Probe on the mutant, in the copy (V): the current test, the `start_repeat` version **and** an untimed `send` version all die at `rejections().is_empty()` with `["UR-21: no transmit channel is configured"]`; the clock count stays 1.
- Passes at `now/advance/submit × 20/200` (V). Alternative kept in reserve: untimed `send` (also passes and kills N47S4-41), but the timed repeat keeps the "target on the stream's clock" subject.
- Still proves: UR-23/RM-25: a burst booked after a Stop is on the stream's one registered clock (one `usrp/tx/s0` clock in the Manifest) and is not refused; wrong behaviour: the Stop drops the clock (N47S4-41) or registers a second.

**T4 `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` (fake.rs:3803). Class R1-tx (premise). Fails at `now-60/200` today (V; `now-20` passed in my re-run).**
- Why: the premise is a timed `tx_send` (`LostAtShutdown.timed_send`); the burst `send` at `tx_at(+20 ms)` (`drop_and_flag`) is dropped late when `now()` is delayed, no timed send, `assert!(device.lost)` fails at `:3814`.
- Move: `send(&mut run, "send", None, …)` — **untimed**, no `now` read, the premise still holds because an untimed burst is also sent timed (V: passes, `device.lost` true). (The tables say `start_repeat`; a timed `start_repeat` also passes at all 6 configs — V — but still reads `now`. The untimed `send` keeps the one-shot burst the test had; recommended.)
- Still proves: UR-26/29/33: a loss found by the shutdown's first report read ends its 100 ms ACK window; uhd-tx joins; no device call after the loss. Wrong behaviour = J318's mutant (`self.core.is_lost()` dropped from the window loop): uhd-tx never leaves the loop → the join detaches. Probe on J318 in the copy (V): the new test dies with `EZSDR-INFRA: … the Run met join … "uhd-tx-s0", "did not join within 1 s; left detached"`, i.e. `killed (bound)`, as J318 declares.

**T5 `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` (`ezsdr-acceptance/tests/uhd.rs:479`). Class R1 (test-side read). Fails today at `now-200`, `submit-5/20/60/200` (V; the 2a sweep listed 5 of 12).**
- Why: `stop_root = run.now()` is read **after** the Stop's `submit` returned (`:491-492`); a delayed thread reads a later device time, the computed `stop` sample is too late, `gap = rows[1].target − stop` falls under `9_900` (`:503`). P1: a read time is not an observation.
- Move: take the Stop's own `LogEntry.time` (`let stopped = run.submit(Stop…)`, `stop_root = stopped.time`), as `fake.rs:2228` does for `ur_23_a_stop_ends_the_burst_within_the_in_flight_window`. The log stamp is taken first in `submit` (`pipeline.rs:939`), so it is a lower bound of the Provider's receipt: the asserted gap can only grow with a delay. The `admitted()` helper there returns `()`; the move needs an entry-returning variant (or `matches!` on `stopped.outcome` inline).
- Probe (V): passes at `now/advance/submit × 20/200`; baseline `gap` = 10 895 samples (was nominal 9 988): **the lower bound `≥ 9_900` now has ~1 ms of slack** (stamp→receipt), so a mutant shrinking the Stop region's 10 ms window by < 1 ms survives this test. No row selects it (V: the `uhd` bin has only `P1341` and `N51-2A-25`); the window is pinned by `control.rs` unit tests and `ur_21_s19…`. 2c can tighten it with the exact `receipts` row for the Stop. Say so in the step's record; do not add a row now.
- Still proves: RM-15/16 on UHD: an untimed repeat after a Stop starts past the Stop region, reports no lateness, with no `TIME_ERROR`/`COMMAND_REJECTED`; wrong behaviour: it starts inside the region (gap < 9 900) or is refused/late.

### 1.2 `rx_start_lead_group_runs_on_the_fake` (`common/measurement.rs:4127`) — classified: **R2, shared with the bench, to 2c** (V)

- Sweep: fails only under `submit` injection (≥ 20 ms), at the `a SHORT candidate` assert (`:4141`).
- Trace (V, probe copy): `restart_trial` submits `Stop`, two value `set`s, then an untimed `start_rx` (`measurement.rs:2156-2170`). With `submit-20` the 3 ms candidate comes back `ProviderFailure`/`late`, `smallest_lead_left_ns = 92 677 005` instead of a short lead. A single `std::thread::sleep(30 ms)` **between the last value `set` and the `start_rx` submit** reproduces the failure (probe `PROBE_BEFORE_START`: FAILED at the SHORT assert); the same sleep **after** the `start_rx` submit passes (probe `PROBE_AFTER_START`); `now` and `advance` injections pass. So the verdict needs `start_rx` to be received **within the configure's apply window after the value sets**; a delayed submit lets the apply finish first and the SHORT/LATE candidates the test insists on (`:4140-4141`) vanish.
- Class: R2 ("phase": an arrival between two device times), and the L→fault direction is "a late arrival *hides* a result", not "creates a fault". The body is `measure_rx_start_lead`, which `hw_measurements_record` runs on the USRP: it cannot move to `Direct` (not a rehearsal any more). The Provider-addressed Action is `start_rx` (its receipt is read at `control.rs:369`), so the witness is `receipts` → **2c**.
- Rows: N47B-19, N47B-36 (above: test-code mutants whose kill is the same delay-sensitive assertion block). Until 2c, a loaded host can produce a false kill on them in the assertion direction. 2c decides: pin them on a pure test of `restart-trial` marking (needs the post-`finish` loop `measurement.rs:2308-2320` factored into a function over a Manifest), or keep with the `receipts` witness. They do not need 2b work.

### 1.3 To 2c, with the reason

| Test (line) | Class | Why not 2b |
|---|---|---|
| `n47s5_front_rehearsal_on_the_fake` (fake.rs:7907) | R1-tx ×3 + R1-k | the trial is `common::n47s5::front_trial`, run by `hardware.rs` too (AGENTS §9 rehearsal rule): a move to `Direct` stops it being a rehearsal; the asserts are `!Fail`, and the fault comes from a late drop → needs `receipt_class` INCONCLUSIVE(timing). Weakening `!Fail` removes the test. |
| `n47s5_long_close_rehearsal_on_the_fake` (7923) | R1-tx + R2 | same; `Fail("1 radio.TIME_ERROR, 0 expected")` is the late B |
| `n47s5_repeat_switch_rehearsal_on_the_fake` (7939) | R1-tx + R1-k + switch | same; shared `repeat_switch_trial` |
| `n47s5_untimed_rehearsal_on_the_fake` (7961) | R1-k + R2 | same; the `untimed_burst_outcome` loop waits for A's start + 5 ms (`n47s5.rs:381-384`) |
| `n47s5_held_tail_and_stop_rehearsal_on_the_fake` (7979) | R2 (J(e)) + R1-tx + Stop; two deliberately late cases (`:8004`, `:8011`) | same; rows N47S5-192/-193/-194 retarget (M-R) in 2c |
| `n47s5_stop_end_rehearsal_on_the_fake` (8028) | R1-tx + R1-k | not in the sweep; tone INCONCLUSIVE anyway; 2c with the rest |
| `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` (8041) | R2 (`late_booking`) + R1-tx; strict `Pass` | same; fails at `now-5…200`, `advance-20/60/200`, `submit-200`; row N47S5-177 is a test-code row (keep) |
| `s1_short_first_sends_rehearsal_on_the_fake` (8061) | R1-tx; strict `Pass` | shared `common::e3::short_first_case` (`hardware.rs`), timed `a = tx_at(+40 ms)` read before two submits; `now-60/200`, `submit-200` |
| `p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake` (8091) | R1-tx; asserts a send at A's instant | shared `common::e3::one_sample_pair`; `now-60/200`, `submit-60/200`; `no send at A's instant` = A dropped late |
| `rx_start_lead_group_runs_on_the_fake` (measurement.rs:4127) | R2 | §1.2 |
| `rehearsal_b7_refusal_holds_on_a_two_board_layout` (fake.rs:2915), `rehearsal_b7_session_loopback` (2927), `…_on_one_obx` (2955) | R1 (untimed repeat) + R1-k | not in the sweep; `late_by_ns ≤ lead` bound (`common/mod.rs:998-999`) is the shared helper's; 2c decides receipts or relax |

`s1_short…` and `p70…` were not in the tables' R table (they are `common::e3`, added after stage 0); the sweep found them. Their doc comment says "no assertion here depends on when a thread runs" (`fake.rs:8053-8056`): the sweep refutes it; 2c fixes the comment with the move.

## 2. The three allow-listed time-reading Run tests, and `session_run_with`

| Item | Decision | Reason (V unless marked) |
|---|---|---|
| `ur_29_a_silent_stream_is_a_lost_device` (fake.rs:2506) | **2b: delete the time reads** (`begun`, `device.host().now() - begun < 7 s`, `Instant::now()`, `wall.elapsed() < 30 s`) and its `TIME_ALLOWED` row | The property is device time. `run_until_end(after(8 s))` returning `Err(Ended)` already says "ended before the 8 s horizon"; `after_silence ∈ [1, 2] s` is read from the Provider's `DEVICE_LOST` event time. The 7 s bound is 1 s tighter than the horizon and the wall bound is a 30 s plain assert (`T9`: an unmarked real-time bound, a false kill under load). Hangs are the watchdog's (120 s) and the wrapper's. Probe on R10 (silence bound 1 s → 3 s): the stripped test dies at `after_silence`: `3100 ms` (V). |
| `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` (fake.rs:3653) | **2b: delete `begun` / `begun.elapsed() < 10 s`** and its `TIME_ALLOWED` row | The property "`finish` returns, uhd-tx joins" is asserted by `rejections` (no `did not join`) and, in the wrapper, by the join bound's marker; the 10 s wall assert is a second, unmarked copy of the 1 s join bound. Probe on J317 (V): the stripped test dies with `EZSDR-INFRA … the Run met join … did not join within 1 s`, so J317 stays `killed (bound)`. |
| `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` (fake.rs:3574) | **stays** (allow-listed, S2 → stage 3) | The Real half measures real-time lateness by design (J295 declares `bound`; ⟂real). The Manual half is deterministic: uhd-tx is descheduled inside `tx_send`, so the manual clock stands (M3); a test-thread delay only moves the `send_asap` booking. The `recv_timeout(5 s)` is an observed-entry handshake. Not changed in the 2a sweep. |
| `ezsdr-acceptance/tests/support/mod.rs::session_run_with` (raw `coordinator::connect` → `RunHandle`) | **2b: scan hardening, no test change** | Only Mock Runs use it today (`v58.rs` ×6, `support::{spec_run,loopback_run,session_run}`: `rig::assemble` = Mock), but its `assembly` argument can be a FakeDevice assembly (`rig::assemble_on`), which would bypass the wrapper. The only users of `assemble_on` are `uhd.rs:56, :70, :268` (all wrapped; V). Rule: the token `assemble_on` in `crates/ezsdr-acceptance/tests/**` outside `uhd.rs` is a violation (covers `support/` and the exempt files). It is a syntactic fact, in the existing `scan_used`. Fixtures in `governance.rs` (a source using `assemble_on` in `support/mod.rs` and in `v58.rs` is found; `uhd.rs` and a string/comment mentioning it are not) and a row. |

Observation, not in 2b's set (flag for the orchestrator's decision, an internal call): tables §4.3 gave `bounded()` (T5, fake.rs:1835) and `entered.recv_timeout(5 s)` (T6, :3584) a marker entry ("`bounded` itself: `infra()`", "inline (2a)"). Neither has one in the tree: `bounded` expiry is a plain `panic!` (`:1846`). A mutant that hangs a Run inside `bounded` is therefore a plain `killed`, not `ERROR`. T4 and the ur_33 test use `bounded`; adding `infra(&format!("{what} did not finish within {bound:?}"))` before the panic is a 2-line mechanical change that belongs to sub-step A. J317/J318 keep their `bound` rows either way (the join marker comes first).

## 3. Mutation rows (filters per tables §6; `remote_mutate.py` later)

A row's filter is a substring of the libtest name including the module path: a new test's name must not contain an existing row's filter.

| Test | Rows selecting it today | New filter | Same mutant still dies on |
|---|---|---|---|
| T1a (Run half, same name) | N47S4-56 (`device_failed(None, …)`) | unchanged (full name of T1a) | `rejected == [(tx_burst, first_id)]` — assertion (V on the probe copy). The Direct test T1b does not contain T1a's name. |
| T1b (new, Direct) | none | new row **N51-2B-01**: N47S4-13b's mutant (`self.record.drop_piece(k, len);` → `self.record = Bookings::unarmed();` in `crates/ezsdr-radio/src/staging.rs`), filter = T1b's full name | the `rejected` pair assert (one entry instead of two) — assertion (V on the probe copy). Not N47S5-89 (its mutant leaves the next burst clear of the old span here: V, T1b passes it). |
| T2 | none | — | — |
| T3 | N47S4-41 | unchanged | `rejections().is_empty()` — assertion, `["UR-21: no transmit channel is configured"]` (V; for the `start_repeat` version and the untimed one) |
| T4 | J318 (`bound`) | unchanged | the join marker (`killed (bound)`), V on the probe copy |
| T5 (`ea_16`, `uhd`) | none | no row (see §1.1 T5: looser lower bound; 2c tightens) | — |
| `ur_29_a_silent…` | phase7:R10 | unchanged | `after_silence` assertion (V) |
| `ur_33_a_shutdown…` | J317 (`bound`) | unchanged | the join marker (V) |
| `ur_21_a_descheduled…` | J295 (`bound`) | unchanged | untouched in 2b |
| scan: `assemble_on` rule | — | new rows **N51-2B-02…** (one per branch of the new rule: the token test, the path exemption for `uhd.rs`, the file scope that includes `support/`), filter the new governance unit test; each row's `old` anchors the exact substring in `governance.rs` | the new fixtures' asserts (written failing first) |

Rows to *keep unchanged* are filters on tests whose names do not change; `rules.py --base 829b7b0` must still find each row's `old` exactly once. The full lists run at each sub-step's commit per AGENTS §7 (the 2b rows: N47S4-56, N47S4-41, J318, J317, R10, J295, the new ones, plus every N51-2A row if governance.rs changes).

## 4. Risk per scene and sub-steps

No production code changes. By AGENTS §9 step 1 a step is risky when it changes a queue, deadline, identity, lifecycle, FFI ownership, or **the boundary between a fake and the hardware it stands for**. 2b changes only which rig a verdict comes from. My reading: **not risky as a step** (no test plan review by Sol needed beyond this plan; the code review at the end stays), with **T1 and T5 flagged as "changes what a test proves"** and so given the audit below.

| Scene | Risk | Why |
|---|---|---|
| T2, T3, T4: book untimed / `send_asap` | mechanical | one booking line each; the mutant that the test exists for (N47S4-41, J318) was run on the new version in the copy and still dies |
| `ur_29_silent`, `ur_33`: delete time reads | mechanical | the property is asserted elsewhere in the test; R10, J317 run on the stripped tests (copy) |
| scan hardening | mechanical (tooling) | failing-first unit tests |
| T1 split | **changes what a test proves** | proof moves from Run to `Direct` + unit; audit table below |
| T5 | **changes what a test proves** | lower bound loosens by ~0.9 ms; no row; 2c tightens |

Audit table (test, what it asserted → what it asserts):

| Test | Before | After | Verdict |
|---|---|---|---|
| T1 | Run: id pair + window + corrected configure | T1a: id + corrected configure in the Run; T1b: window pair (ids, `kind`) on `Direct`; unit tests keep the rule | fixed (window half moved; id half kept) |
| T2 | no `TIME_ERROR`, overlap refusal | same, stronger (no timed booking) | still right |
| T3 | one clock, no refusal | same | still right (the refusal assert is the N47S4-41 killer: kept) |
| T4 | loss ends the ack window | same | still right (J318 dies by the join bound) |
| T5 | gap ≥ 9 900 from a read `now()` | gap ≥ 9 900 from the Stop's stamp (lower bound) | fixed, weaker by the stamp→receipt slack |
| ur_29_silent, ur_33 | + wall/device-time asserts | − them | fixed (redundant) |

Sub-steps (each leaves the tree green; the implementer reports after each; the orchestrator commits on `work/n51-2b`):
- **A (mechanical):** the scan's `assemble_on` rule + fixtures + rows; delete the time reads of `ur_29_silent` / `ur_33` and their two `TIME_ALLOWED` rows (the real-tree test fails if a row is left unused); optionally the `bounded` marker (orchestrator's call, §2). Rows: N51-2B-02…, R10, J317, J318 retested.
- **B (R1 non-subject):** T2, T3, T4, T5. Rows: N47S4-41, J318 retested; the `rules.py` anchor check.
- **C (T1 split):** T1a + T1b + row N51-2B-01; N47S4-56 and N47S4-13b-derived row retested.
- **D (text):** README line in `crates/ezsdr-radio-uhd/tests/README.md` (what the Run wrapper no longer catches: the 13 tests above go to 2c; the allow list is shorter by two); note 51 "段 2b の後"; tables §3.2 "Planned move" column and §6.1 rows (N47B-19/-36 are test-code rows); fix the "14 `fake` / 15 tests" count in the 2a record's text (it is `n51-2a-dispositions.md`, which is a closed record: correct it in the new 2b record instead). No `design/` text is touched; no KNOWN_LIMITATIONS entry.

## 5. Failing-test-first

The sweep injection that fails today and the version that passes after the move (V = run; the command is `EZSDR_RIG_DELAY_AT=<point> EZSDR_RIG_DELAY_MS=<ms> cargo test -p <crate> --test <bin> <name> -- --test-threads=1`):

| Test | Fails today | Passes after (probe) |
|---|---|---|
| T1 / T1a | all 12 of `{now,advance,submit} × {5,20,60,200}` | T1a: 12/12 (V). T1b: not reachable by the injection (Direct): `std::thread::sleep(200 ms)` after every push and settle: passes 3/3 (V); today's Run test is not comparable |
| T2 | `now` 20, 60, 200 (V: `now-20`, `now-200` re-run) | `now/advance/submit × 20, 200` (V) |
| T3 | `now` 20, 60, 200 (V) | `now/advance/submit × 20, 200` (V) |
| T4 | `now` 60, 200 (V: `now-200` re-run; `now-20` passes today) | `now/advance/submit × 20, 200` (V) |
| T5 | `now-200`, `submit` 5, 20, 60, 200 (V: `now-20` passes today) | `now/advance/submit × 20, 200` (V) |
| `ur_29_silent`, `ur_33` | none (not in the sweep) | no failing-first: mechanical; the kill is what the rows show |
| scan rule | a fixture source with `assemble_on` in `support/mod.rs` is not reported today | red first, then green |

The implementer repeats the new `Direct` test (T1b) 20 times (its verdict depends on threads and the clock, AGENTS §9 step 8); T1a, T2–T5 and the scan are pure with respect to the test thread (the sweep covers them) and need the one full 12-config sweep at the end of the step: `fake` and `uhd`, expecting exactly the 13 tests of §1.3 (counting `rx_start_lead…`) and nothing else.

## 6. What was not verified

- No mutation was run through `remote_mutate.py`; the mutant probes (N47S4-56, N47S4-41, N47S4-13b, N47S5-89, J317, J318, R10) are single runs in the copy.
- T1b's behaviour under injected delays is by sleeps in the test body, not by `EZSDR_RIG_DELAY_*` (`Direct` has no injection point, by design).
- Only 20 and 200 ms were swept for the moved versions; 5 and 60 are expected, not run.
- The cause of the `smallest_lead_left_ns = 92 677 005` in `rx_start_lead…` under `submit-20` is not explained beyond "the delay sits between the value sets and `start_rx`" (the apply/receipt arithmetic of `restart_trial` was not worked out).
- The `hardware` binary is not built here (`hw-build` is the gate's); 2b does not touch `hardware.rs`, but T5 and the scan change nothing it calls.
- The stamp→receipt slack in T5 (~0.9 ms) is one baseline run.

## Changes in implementation (2026-11-13)

Deviations from the plan, and the audit table.

- A: the `bounded()` expiry and the `entered.recv_timeout(5 s)` of `ur_21_a_descheduled…` write `infra(...)` before they panic. The scan rule is one token test (`assemble_on`) with `fake_assembly` = any `ezsdr-acceptance/tests/**` file except `uhd.rs`; the early return of `scan_used` includes it. Failing first: `run_scan_a_fake_device_assembly_is_made_only_in_uhd_rs` failed on `support/mod.rs` before the rule. Rows N51-2B-01…05 (token, `uhd.rs` exemption, exempt files covered, early return, acceptance-only scope). TIME_ALLOWED 24 → 22.
- B: T2, T3, T4, T5 as planned (T3: `start_repeat` at `+10 ms`, the `rejections().is_empty()` assert kept; T4: untimed `send`; T5: `stopped.time`, the Stop's own log stamp).
- C: T1a waits 60 ms twice (not 12/1.5/9 ms); T1b as planned, row N51-2B-06 (N47S4-13b's mutant, filter T1b's full name; T1a's name is not contained in it).
- D: README, note 51 "段 2b の後", the "14 / 15" corrections in note 51 and `n51-2a-dispositions.md`. Not done: the tables §3.2 "Planned move" column and §6.1 rows (tables are the stage-0 record; the note's paragraph carries the result).

Audit (test, expectation, verdict):

| Test | Expectation | Verdict |
|---|---|---|
| `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (T1a) | `rejected == [(tx_burst, first_id)]` (Kernel id = Provider id); nothing sent before the corrected value; corrected configure before the next `tx_send`; one clock | fixed (the early-value pair and its window moved to T1b) |
| `ur_21_a_refused_configure_keeps_its_burst_s_span_until_its_booked_end_on_the_direct_rig` (T1b, new) | `rejected == [(tx_burst, 2), (update_parameter, 3)]`; nothing sent before the second burst; configure of 3 GHz before `tx_send`; one clock | new (passes 20/20; passes with a 200 ms sleep after every push and settle, in a probe copy; dies on N47S4-13b's mutant at the `rejected` assert (plan probe)) |
| `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` | `rows == 2`; `gap ≥ 9 900` from the Stop's stamp; no `late_by`; no TIME_ERROR / COMMAND_REJECTED | fixed (gap's lower bound loosened by the stamp→receipt slack, ~1 ms; no row selects it; 2c may tighten with `receipts`) |
| `ur_21_s18…`, `ur_23_a_burst_after_a_stop…`, `ur_29_a_loss_found…` | as before | still right |
| `ur_29_a_silent_stream…`, `ur_33_a_shutdown…` | as before minus the time asserts | fixed (redundant: `Err(Ended)`, `after_silence`, the join marker) |

Injection (`EZSDR_RIG_DELAY_AT`/`_MS`, 12 configs `{now,advance,submit}×{5,20,60,200}`, `--test-threads=1`): `ur_21_a_refused_configure*` (both), `ur_21_s18`, `ur_23_a_burst_after`, `ur_29_a_loss_found`, `ur_29_a_silent`, `ur_33_a_shutdown`, `ur_21_a_descheduled` (8 tests) pass in all 12; `ea_16_…` passes in all 12. T1a, `ur_21_s18`, `ur_23_a_burst_after` and `ur_29_a_loss_found` and `ea_16` failed in the 2a sweep at the configs listed there; the others were not in it. 20 runs without injection: all pass (T1b included).
