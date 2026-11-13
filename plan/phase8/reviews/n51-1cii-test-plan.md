# Note 51 stage 1c-ii (stage 4): the `Frozen` / `released` classification table

Tree: `main` 1750373 (clean). Written 2026-11-14 by the stage's table implementer (Claude Code Sonnet subagent). Plan only: nothing in the tree was edited. Probes ran in a copy (`/tmp/n51-1cii-probe`, built into `~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`, deleted at the end). V = run, I = inferred (not run).

## 0. The answer in one page

**Scope.** `grep -rniE 'frozen|handed_over|released\(' crates/ezsdr-radio-uhd/tests crates/ezsdr-acceptance crates/ezsdr-server` finds no `Frozen`, `released` or `handed_over` (only the word "frozen" in comments, and `tests/README.md:94`). The stage is the lib tests of `crates/ezsdr-radio-uhd/src/provider` (V). Measured at run time (a `#[track_caller]` log in a probe copy, 667 lib tests): **350 tests build a held clock**, through **79** `Frozen::of`/`on` sites (the plan's "about 75"), **39** `released` calls, **13** `handed_over` calls (all outside the helper itself), and **18** test-level `drop(frozen)` (the plan's 17 on `bfd59d3` counted the `drop` inside `released` as its H site; two `drop`s sit inside the helpers). Five rig builders carry most of the tests: `switch_rig_on` 114, `txp_rig_faults` 70, `owner_with` 29, `txp_high_rate_rig` 15, `at_a_held_tail` 10.

**Recommendation (one).** Replace every site with the participant guard and delete `Frozen`, `released` and `handed_over`. **No site needs `Frozen`**; the plan's keep-case ("time must stay stopped while the test waits for an owner's reaction; a non-clock wait under the guard hangs") does not occur. Reason: M2 says a participant is idle only while it waits on the clock; a guard-holding test thread blocked in a channel wait is busy and stops the clock exactly as the frozen participant did. A guard *kept* across a non-clock wait hangs only where the awaited owner needs the clock (W1); at those five sites the hold is dropped before the wait today, and the guard is dropped at the same line.

**Treatment counts** (by site; the same site can serve tests of several shapes, see §5):

| Treatment | Sites | Meaning |
|---|---|---|
| G: `Frozen::of/on` -> `clock_guard(clock)` guard, held to where the Frozen was dropped | 79 (57 plain G, 10 G/R, 3 G/H, 4 G/W, 2 G/N, 3 G/R+N) | the constructor |
| inline: `released(f, clock, work)` -> `work` on the guarded thread | 39 | §5.1 |
| collapse: `handed_over(f, clock)` deleted (the guard is already the test thread's) | 13 | §5.2 |
| `drop(driver)` kept at the same line (W1: the test thread must stop being a participant before it waits off the clock) | 5 (`rx.rs:902, :947`, `tx.rs:6124, :6231, :6264`) | §5.3 |
| delete the `drop(frozen)` statement (N: last statement or assertions only) | 12 | §5.3 |
| `drop` then owner code on the test thread (G shape the 1d recount missed) -> hold to the end | 1 (`c2.rs:581`) | §5.3 |
| **keep `Frozen`** | **0** | |
| delete a hold as redundant | **0** (23 tests pass with no hold in 3 of 3 runs; that is not a proof, §6.4) | |

**`Frozen` can be removed** (V on the unmutated suite, I on the rows): §2 and §7.

**What the evidence is.** A shim in the probe copy turns `Frozen` into a `Driver` wrapper (`Frozen::on` = `clock.drive()`), `released` into `(work(), frozen)` and `handed_over` into "give back the held `Driver`": **667/667 lib tests pass**, 3 rounds x 8 concurrent runs on 16 cores pass 24/24, and a slow test thread (5 ms after every `device_at`, 20 ms before and after every inlined work) passes. It does **not** show the mutation rows still kill; that is I until the rows are run (§6).

**Sub-steps** (§8): 4a the semantic swap in `test_support.rs` (about 80 lines, risky), 4b `core/t3.rs`, `core.rs`, `rx.rs`, `mod.rs`, `control.rs` (48 tests, about 250 lines), 4c `tx.rs` and `tx/tests/*` (301 tests, about 450 lines), 4d delete the helpers, retire rows, texts (about 150 lines). All under 1 500 lines; each leaves the tree green.

**Where the plan's rule does not fit** (§9): (1) the keep-case premise; (2) "about 75" is 79 sites in 350 tests; (3) `c2.rs:581` is a G shape not in 1d's table; (4) a stray clock wait by the test thread hung under `Frozen` and advances time silently under the guard (0 occurrences in 287 hold-only tests, V; a design cost, §9 Q1); (5) the one 1d-C′ site left is `tx.rs:1852`, with no `Frozen`.

**Not decided / recommendations:** §9.

## 1. How the table was made

- Sites: `grep -rnE 'Frozen::(of|on)\(' crates/ezsdr-radio-uhd/src` (83 lines: 79 test or rig sites, `of` -> `on` forwarding at `test_support.rs:147`, the `Frozen::on(again)` inside `released` at `:180`, two helper tests at `:1163`, `:1173`); `grep -rnE '\breleased\(' ...` (39); `grep -rnE '\bhanded_over\(' ...` (16: 13 sites, the definition `:192`, two helper tests); `grep -rnE 'drop\((_?frozen|rig\._frozen)\)' ...` (20: 18 tests, 2 helpers).
- Which tests use which site: **dynamic**. In a probe copy `Frozen::on`, `released` and `handed_over` carry `#[track_caller]` and append `kind file:line thread-name` (libtest names the thread after the test) to a file; one lib run gave 1 006 lines, 350 test names, 80 F sites (79 + the helper test), 39 R, 14 H (13 + the helper test). Static call-graph guessing was dropped: it under-counted (247 tests) because rig builders call rig builders.
- Wait kinds: read in full for every site that spawns a participant, waits on a channel, spins, joins or has a `released`/`handed_over`/`drop` (§5.3's W1 sites, `rx.rs:849-1113`, `:1475`, `:1767`, `:2299-2340`, `:2457`, `mod.rs:1086-1145`, `tx.rs:1593-1802`, `:4518-4640`, `:6103-6290`, `:6700-6776`, the `c2.rs`, `n75.rs`, `o1.rs`, `e2b.rs`, `e3b.rs` hold and release sites, `core.rs:1994-2020`, `core/t3.rs:30-135`); for the rig builders (`txp_rig*`, `switch_rig_on`, `owner_with`, `retry_rig*`, `at_a_held_tail`, `untimed_rig`, `hold`) I read the builder and the users that release, hand over or drop, and rely on the measurement for the rest. Then measured: a probe in `host.rs::park` and the clock-lock wait logs every time a thread named `provider::...` (a test thread) goes idle on a manual clock: 11 764 events; **0 in the 287 tests without an R or H site; 52 of the 63 tests with one make at least one clock wait** (the other 11 reach none on their path, e.g. loss tests).
- Mutation rows: `plan/**/mutations.json` (2 747 rows; 572 are radio-uhd lib rows). A row selects a test when its `filter` is a substring of the test's full path (`mutate.py:141`, `cargo test -q -p .. --lib <filter>`). **249** rows select at least one of the 350 tests excluding `J58` (232 maintenance, 17 phase7); 176 tests have a row, 174 have none (their hold is by construction). Two rows are broad: `J58` (`provider::`, 538 tests, `equivalent`, declared `bound`) and `J53` (`ur_29_a_`, 13 tests, 4 of them users). The only row anchors that lie in a test region of a file this stage edits are `N51D-03` (`tx.rs`) and, in `test_support.rs`, `N51D-01`, `N51D-02` (anchor scan with `anchor.find` over every row; 1 hit in a test region).
- **Offloaded: nothing.** The brief allows Go DeepSeek for listing sites; a `#[track_caller]` log is deterministic and exact, so no model was used for the listing and nothing needed to be checked against it.

## 2. Evidence that the guard fits everywhere (V unless marked)

Probe copy of `1750373`, lib tests only (`cargo test -p ezsdr-radio-uhd --lib`, baseline 668 passed, 10.9 s).

1. **Shim** (`test_support.rs` only): `Frozen { driver: Option<Driver> }`, `Frozen::on(clock)` = assert manual + `clock.drive()`; `released(frozen, _clock, work)` = `(work(), frozen)`; `handed_over(mut frozen, _clock)` = `frozen.driver.take()`. The helper test that moves a `Frozen` into a thread was deleted (`Driver` is `!Send`; **no other site needs `Frozen: Send`**: the crate compiles). Result: 667 passed, 0 failed. 24/24 green in 3 rounds x 8 concurrent copies of the test binary (16 cores; each run 14 s against 11 s alone).
2. **Negative control** (`Frozen::on` registers nothing, `released` inline, `handed_over` drives): 326 of 667 fail in 3 of 3 runs. So the hold is observed by 326 of the 349 non-helper users. 23 pass without it (listed in §6.4); the other 326 prove the shim's hold is real, not vacuous.
3. **Slow test thread**: `thread::sleep(5 ms)` after every `device_at`'s `set_time`, 20 ms before and after every inlined `released` work: 667 passed. The clock does not move while a guard-holding thread is slow.
4. **Idle probe** (§1): no test without an R/H site makes its thread wait on the clock under the guard. Under `Frozen` such a wait hangs the test; none exists today, so for the 287 hold-only tests guard and `Frozen` hold the same time. The 63 R/H tests wait on the clock; that it is inside the inlined work follows by argument (the probe counts events per test, not where they happen).
5. **Non-clock waits under a kept hold** (`rx.rs:1113`, `:1475`: the plan's keep-case): both pass with the hold as a guard. The awaited participant (an owner parked in a probe, or queued on the chain clock lock) needs no clock time, and the test thread, blocked on `recv_timeout`/a spin, is busy and stops the clock as `Frozen` did.

**Limits.** One tree, one machine. The rows were not run (I). Thread-local hooks (`on_next_lock`, `record_locks`, `LINE_HELD`) armed inside a `released` closure used to die with the worker; inline they live on the test thread: the suite is green, but a hook armed and not consumed would now fire at the test thread's next `core::lock` (risk a, §3).

## 3. Legend

**P (what the freeze protects).**
- **D**: device time exact between the `device_at`/`set_time` steps the test sets, for owner code the test thread runs itself (`Tx::step`/`pass`, `Control::book`, `Core::*`) and for assertions that read `time_now`. Without a hold the clock runs at real speed (M5) under a non-participant test thread (M6): `device_at`'s equality assertion fails (the negative control).
- **F**: D, and the owner's call waits on the clock (recovery flush ack, 100 ms retry attempts, reopen, shutdown window): the wait must run at clock speed and the clock must stay where the test left it before and after.
- **Q**: a start queued far in the future on a held clock; the test thread later waits on the clock itself (`rx_recv` with 100 ms timeouts).
- **O**: owner participants (`host.spawn`) exist while the clock is held; the hold ends or stays while the test thread waits by channel, spin or join.

**W (the wait under it).**
- **W0**: none; every call is synchronous on the test thread.
- **Wc**: a clock wait, made by the code the release or hand-over wraps (flush, reopen, shutdown, `rx_recv` timeouts).
- **Wn**: a non-clock wait (channel, spin with `thread::sleep(1 ms)`, `join`) after the hold is dropped, for owners that are participants and need the clock.
- **Wh**: a non-clock wait while the hold stays; the awaited participant needs no clock time.

**Treatment.** G = guard from the `Frozen::of/on` line; /R = every `released` of the site inlined; /H = `handed_over` deleted; /W = the guard is dropped where the Frozen is dropped today (W1: the test thread must stop being a participant before it waits off the clock); /N = the `drop(frozen)` statements of the site are deleted (N: last statement or assertions only).

**Risk.**
- **-**: none beyond the shared ones below.
- **a**: thread-local hooks armed inside inlined work now live on the test thread (assert `next_lock_armed()` is false after the work where a hook was armed); `SLACK_MS`-bounded elapsed assertions (`core/t3.rs`) keep their bounds unchanged.
- **b**: the thread is a participant from the construction, not from the hand-over; nothing between them waits on the clock (idle probe: 0).
- **c**: a hold that is not dropped before the non-clock wait would hang the test (deterministically, every run, no marker unless the wait is bounded: `recv_timeout(10 s)` and the 10 s `Instant` deadlines are).
- **d**: the `drop` is the last statement or only assertions follow; no behaviour.
- **e**: the handshake under the guard hangs if the awaited participant ever needed clock time; it needs none today (V: passes; the owners are parked in a probe / on a lock).

**Shared risks.** (1) A hang under the guard (a non-clock wait while the guard is held for a participant that needs the clock): only at the W1 sites; the exit checks (§8) run every moved test serially and in parallel. (2) A vacuous pass: the assertions are unchanged, so a test that passed with a held clock and passes with a guard cannot become weaker unless the guard registers nothing; the 4a row on `Frozen::on` (§6) kills exactly that, with 326 hold-sensitive killers. (3) Hold-only tests lose `Frozen`'s hang-on-stray-wait detection (§9 Q1).

## 4. Table F: the 79 constructor sites

A row without a note (`D/W0`) is a stepped single-thread test: it builds the rig, books bursts, calls `Tx::step`/`pass`/`read_list` or `Control::book` itself and asserts at instants it sets with `device_at`; the "reaction" it waits for is the return value of its own call, and no wait (clock or other) happens under the hold. The `ur_21_*`/`ur_23_*` rows are all of this kind.

(`Tests` = tests that reach the site at run time; names are in Appendix A. `Rows` = rows whose filter selects those tests, killers' count first; `-` = no row selects them. `J58` and `J53` omitted.)

| # | Site | In | Tests | Freeze protects (P) / wait under it (W) | Hold ends | Treatment | Rows | Risk |
|---|---|---|---|---|---|---|---|---|
| F1 | `control.rs:767` | `hold` | 9 | D/W0 — `hold()` = Frozen + `device_at(0)`; 2 tests call it, 7 through `staging_rig` | returned to the test | G | 5: N47S3-38, N47S3-16, .. | - |
| F2 | `core.rs:2001` | `ur_22_a_recovery_reopen_flush_records_its_underflows_as_its_own` | this one | F/Wc — Core::reopen_tx_streamer flush after `feed.give` reports | end of test | G/R | - | a |
| F3 | `core/t3.rs:42` | `held_rig` | 9 | F/Wc — `held_rig`: Frozen built before the Core (fault instants count from the device start) | end of test | G/R | 17: N75-17, N75-20, .. | a |
| F4 | `core/t3.rs:80` | `strict` | 2 | F/Wc — `strict`: one strict flush attempt `tx_flush_attempt` | end of test | G/R | 5: N75-08a, N75-08b, .. | a |
| F5 | `core/t3.rs:136` | `ur_75_a_send_that_is_not_whole_ends_its_attempt_at_once` | this one | F/Wc — strict flush on a LockSpy device | end of test | G/R | N75-09, N75-10 | a |
| F6 | `core/t3.rs:257` | `ur_75_the_confirmed_tick_is_the_largest_ack_tick_across_positions` | this one | F/Wc — largest ack tick across positions | end of test | G/R | N75-07 | a |
| F7 | `core/t3.rs:325` | `ur_75_a_stale_ack_stays_for_the_report_owner_across_the_loop` | this one | F/Wc — stale ack across the retry loop | end of test | G/R | - | a |
| F8 | `core/t3.rs:535` | `ur_75_stop_and_loss_at_each_boundary_of_the_retry` | this one | F/Wc — stop/loss at each boundary of the retry | end of test | G/R | N75-05a, N75-05c | a |
| F9 | `core/t3.rs:712` | `ur_75_with_the_switch_off_the_open_flush_is_todays` | this one | F/Wc — switch-off open flush | end of test | G/R | N75-13, N75-14 | a |
| F10 | `mod.rs:1098` | `ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup` | this one | Q/Wc — a queued rx start at 5 s on a held clock; then `rx_recv(100 ms)` x6 by the test thread | handed_over at mod.rs:1140 | G/H | P1292, N92-7c | b |
| F11 | `rx.rs:849` | `ur_24_fake_host_silence_delivers_no_block` | this one | D/W0 — `rx_recv(Wall::ZERO)` polls; time set with `set_time` | end of test | G | - | - |
| F12 | `rx.rs:871` | `ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead` | this one | O/Wn — owner `rx.turn()` on a spawned participant (rx_recv with 6.5 ms latency = clock wait); test waits on `gate.wait()`, `finished.recv_timeout`, `join` | drop at rx.rs:902 | G/W | P1238 | c |
| F13 | `rx.rs:923` | `ur_24_a_far_future_cut_keeps_pre_cut_receives_multi_packet` | this one | O/Wn — owner participant stepped by channel turns; test waits on `first_turn`/`second_turn` `recv_timeout`, `join` | drop at rx.rs:947 | G/W | P1242 | c |
| F14 | `rx.rs:985` | `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` | this one | D/W0 — `try_end()` on the test thread; assertions after the drop read calls/events only | drop at rx.rs:1001 (N) | G/N | N47S3-21 | d |
| F15 | `rx.rs:1019` | `spied_open_rig` | 2 | D/W0 — `spied_open_rig`: `rx.begin()` then assertions on spy calls/events (2 tests) | drop at rx.rs:1060, :1077 (N) | G/N | N67F-22, J53, N67F-23 | d |
| F16 | `rx.rs:1032` | `ur_16_an_open_stops_the_streamer_before_its_first_start_and_cancels_an_orphaned_start` | this one | Q/Wc — orphaned queued start; then `rx_recv(100 ms)` x6 on the test thread | handed_over at rx.rs:1044 | G/H | N67F-21 | b |
| F17 | `rx.rs:1092` | `ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls` | this one | D/W0 — RxRecv::Failed fed by hand; no device time read | end of test | G | P1256 | - |
| F18 | `rx.rs:1113` | `ur_24_a_loss_published_before_rx_recv_skips_the_device_call` | this one | O/Wh — spawned owner parked in a probe (`waiting.recv()`); test `recv_timeout` handshakes, `device_lost`, `join`; owner needs no clock time | end of test | G | P1295 | e |
| F19 | `rx.rs:1475` | `ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` | this one | O/Wh — owner `begin` queued on the chain clock lock; test spins on `waiters()`, `device_lost`, drops the chain guard, `recv_timeout`, `join` | end of test | G | J53, P1291 | e |
| F20 | `rx.rs:1767` | `ur_17_a_finite_segment_delivers_through_its_cut` | this one | D/W0 — `rx_recv(.., 100 ms)` returns at once (data present) | end of test | G | P1396, P1397 | - |
| F21 | `rx.rs:2308` | `queued` | 5 | Q/Wc — `queued()` rig for 5 tests; one reads the device through the origin with 100 ms timeouts | handed_over at rx.rs:2331 (`assert_silent_through`, 1 of 5 tests) | G/H | 6: N67F-02, N67F-01, .. | b |
| F22 | `rx.rs:2457` | `ur_26_a_start_after_a_cancelled_queued_start_delivers_at_its_own_origin` | this one | D/W0 — `rx_recv(.., 100 ms)` returns at once | end of test | G | N67F-05 | - |
| F23 | `rx.rs:2673` | `ur_24_a_missed_start_after_a_loss_makes_no_restart_call` | this one | D/W0 — loss then LateCommand; call counts | end of test | G | P1290 | - |
| F24 | `rx.rs:2735` | `begin_with_configure_ending_at` | 2 | D/W0 — `begin_with_configure_ending_at`: freeze lives inside the helper | end of helper | G | N47S3-13, N47S3-14, N47S3-12 | - |
| F25 | `rx.rs:2788` | `begin_and_recover_from_a_missed_start` | 2 | D/W0 — `begin_and_recover_from_a_missed_start`: freeze lives inside the helper | end of helper | G | - | - |
| F26 | `tx.rs:1608` | `retry_rig_without_burst` | 9 | F/Wc — `retry_rig_without_burst` for 9 tests; `pass_on` = `released(.. tx.pass())` (3 x 100 ms flush attempts) | 5 tests drop at the end (N): tx.rs:1686, :1706, :1764, n75.rs:79, :217 | G/R+N | 9: N75-26, N75-27, .. | a |
| F27 | `tx.rs:1773` | `ur_22_a_partial_ack_on_every_attempt_is_unconfirmed` | this one | F/Wc — `two_channels` closure; `pass_on` | end of test | G/R | - | a |
| F28 | `tx.rs:2220` | `ur_21_s1_overlap_refused_at_receipt` | this one | D/W0 | end of test | G | N47S5-19 | - |
| F29 | `tx.rs:2247` | `ur_21_s6_send_asap_moves_past_an_accepted_span` | this one | D/W0 | end of test | G | N47S5-21 | - |
| F30 | `tx.rs:2273` | `ur_21_s6_a_late_drop_and_flag_booking_is_dropped_before_the_overlap_is_judged` | this one | D/W0 | end of test | G | - | - |
| F31 | `tx.rs:2350` | `ur_21_s6_send_asap_past_a_later_booking_is_placed_after_it` | this one | D/W0 | end of test | G | - | - |
| F32 | `tx.rs:2408` | `untimed_rig` | 3 | D/W0 | end of test | G | 5: N47S5-123, N47S5-130, .. | - |
| F33 | `tx.rs:2431` | `ur_21_s5_receipt_order_decides_the_overlap` | this one | D/W0 | end of test | G | - | - |
| F34 | `tx.rs:2465` | `ur_21_s5_c_a_booking_before_a_chain_beyond_the_horizon_is_accepted` | this one | D/W0 | end of test | G | N47S5-106 | - |
| F35 | `tx.rs:2491` | `ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt` | this one | D/W0 | end of test | G | N47S5-102, N47S5-105 | - |
| F36 | `tx.rs:2525` | `ur_21_a_send_asap_move_is_reported_once_across_handovers` | this one | D/W0 | end of test | G | - | - |
| F37 | `tx.rs:2567` | `ur_21_a_send_asap_judged_again_at_dispatch_is_not_reported_again` | this one | D/W0 | end of test | G | P667 | - |
| F38 | `tx.rs:2600` | `ur_21_tx_front_is_per_stream` | this one | D/W0 | end of test | G | - | - |
| F39 | `tx.rs:2641` | `ur_21_a_target_after_a_completed_burst_is_on_time` | this one | D/W0 | end of test | G | - | - |
| F40 | `tx.rs:2673` | `ur_21_send_does_not_cross_the_in_flight_window` | this one | D/W0 | end of test | G | P638 | - |
| F41 | `tx.rs:2710` | `ur_21_tx_reads_in_flight_window_from_description` | this one | D/W0 | end of test | G | P628 | - |
| F42 | `tx.rs:2727` | `ur_21_tx_reads_in_flight_window_from_description` | this one | D/W0 | end of test | G | P628 | - |
| F43 | `tx.rs:2749` | `measurement_candidate_in_flight_window_sets_the_tx_chunk_size` | this one | D/W0 | end of test | G | P1160 | - |
| F44 | `tx.rs:2790` | `ur_21_a_new_burst_without_an_open_predecessor_is_on_time` | this one | D/W0 | end of test | G | - | - |
| F45 | `tx.rs:2815` | `ur_21_s2_a_finite_front_stops_at_its_end` | this one | D/W0 | end of test | G | - | - |
| F46 | `tx.rs:2846` | `ur_21_a_finite_burst_inside_the_window_is_refused` | this one | D/W0 | end of test | G | - | - |
| F47 | `tx.rs:2882` | `ur_21_s7_a_handed_repeat_refuses_a_booking_inside_its_span` | this one | D/W0 | end of test | G | - | - |
| F48 | `tx.rs:2919` | `ur_21_a_late_move_into_a_handed_repeat_is_refused` | this one | D/W0 | end of test | G | - | - |
| F49 | `tx.rs:2961` | `ur_21_s4_a_booking_at_the_finite_front_is_on_time` | this one | D/W0 | end of test | G | - | - |
| F50 | `tx.rs:3198` | `ur_23_stop_preserves_handed_cursor_after_stop` | this one | D/W0 | end of test | G | N47S5-24, N47S5-30 | - |
| F51 | `tx.rs:3253` | `ur_23_stop_does_not_send_unhanded_window_edge_payload` | this one | D/W0 | end of test | G | - | - |
| F52 | `tx.rs:3291` | `dropped_head_rig` | 3 | D/W0 | end of test | G | - | - |
| F53 | `tx.rs:3363` | `ur_23_stop_closes_a_running_repeat_at_its_handed_cursor` | this one | D/W0 | end of test | G | - | - |
| F54 | `tx.rs:3396` | `ur_21_a_finite_burst_with_a_held_tail_keeps_its_front` | this one | D/W0 | end of test | G | E3A-U04 | - |
| F55 | `tx.rs:3431` | `ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst` | this one | D/W0 | end of test | G | N48S1-08, E3B-41 | - |
| F56 | `tx.rs:3471` | `ur_21_a_post_eob_restart_uses_the_safe_sample` | this one | D/W0 | end of test | G | P640, P641, P642 | - |
| F57 | `tx.rs:3542` | `ur_21_a_burst_inside_the_in_flight_window_is_refused` | this one | D/W0 | end of test | G | - | - |
| F58 | `tx.rs:4108` | `at_a_held_tail` | 10 | D/W0 — `at_a_held_tail`: 10 tests | returned to the test | G | 14: N47S4-50, P658, .. | - |
| F59 | `tx.rs:4232` | `ur_23_a_stop_ends_the_burst_within_the_in_flight_window` | this one | D/W0 | end of test | G | - | - |
| F60 | `tx.rs:4352` | `ur_23_bursts_go_out_in_target_order` | this one | D/W0 | end of test | G | U18 | - |
| F61 | `tx.rs:4381` | `ur_22_a_first_send_cut_short_keeps_the_reports_paired` | this one | D/W0 | end of test | G | B26, B27 | - |
| F62 | `tx.rs:4447` | `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` | this one | D/W0 | end of test | G | 5: P1168, E3B-45, .. | - |
| F63 | `tx.rs:4523` | `ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst` | this one | F/Wc — failed EOB close, then `released(.. tx.pass())` (flush) in a loop over 3 faults | drop at tx.rs:4567 (N) | G/R+N | P1172, P1191 | a |
| F64 | `tx.rs:4651` | `ur_23_abandoned_send_preserves_the_accepted_cursor` | this one | D/W0 | end of test | G | P657, P663, P664 | - |
| F65 | `tx.rs:4687` | `ur_23_a_continuation_tail_is_counted_at_recovery` | this one | D/W0 | end of test | G | P665, P666 | - |
| F66 | `tx.rs:4793` | `txp_rig_faults` | 70 | D/W0 — `txp_rig_faults` <- `txp_rig`, `txp_rig_rate`: 70 tests; 4 call `pass_on`/`released` | returned to the test | G | 28: N47S5-57, E2A2B-10, .. | a |
| F67 | `tx.rs:4812` | `txp_high_rate_rig` | 15 | D/W0 — `txp_high_rate_rig`: 15 tests; 5 call `released`; the `recv` hits are `rx_recv(.., 10 ms)` loopback reads that return at once | returned; 1 test drops at tx.rs:4635 (N) | G | 18: E3B-42, N48S1-02, .. | a |
| F68 | `tx.rs:5072` | `ur_23_s3_a_pruned_head_moved_into_a_later_booking_is_refused` | this one | D/W0 | end of test | G | N47S5-88 | - |
| F69 | `tx.rs:5616` | `ur_23_a_new_sob_after_an_abandon_waits_for_32_window_samples` | this one | D/W0 | end of test | G | - | - |
| F70 | `tx.rs:6108` | `txp_owners` | 4 | O/Wn — `txp_owners`: owners are `host.spawn` participants; test spins with `thread::sleep(1 ms)` under a 10 s `Instant`, then `txp_join` (4 tests) | drop at tx.rs:6124 | G/W | 8: P1100, P1101, .. | c |
| F71 | `tx.rs:6212` | `txp_idle_owner` | 2 | O/Wn — `txp_idle_owner`: m8b and m6; owner participant takes an idle look every 200 us (clock wait); test `recv_timeout`s its snapshot/wake | drop at tx.rs:6231, :6264 | G/W | P388, P1118 | c |
| F72 | `tx.rs:6315` | `owner_with` | 29 | D/W0 — `owner_with` <- `owner`: 29 tests; 10 end in `shutdown_owner`, 3 in `released`; the `recv` hits are non-blocking `cmds.try_recv()` and the one 50 ms `thread::sleep` of `ur_28_…` | returned to the test | G | 19: E2B-10, E2B-13, .. | b |
| F73 | `tx.rs:6519` | `ur_21_a_refused_burst_stays_counted_until_its_booked_end` | this one | D/W0 | end of test | G | N47S4-13b, N47S5-89 | - |
| F74 | `tx.rs:6784` | `ur_23_two_transmit_streams_busy_and_empty` | this one | D/W0 | end of test | G | - | - |
| F75 | `tx.rs:6863` | `switch_rig_on` | 114 | D/W0 — `switch_rig_on` <- `switch_rig`, `switch_rig_with`: 114 tests (e1, e2a, e2b, e3, o1, c2, n99 families) | returned (`SwitchRig._frozen`); c2.rs:581 drops it (G); o1.rs:753 drops it (N) | G | 55: C2-19, C2-23, .. | b |
| F76 | `tx/tests/e1.rs:75` | `rig_at` | this one | D/W0 — `rig_at` | `shutdown` (e1.rs:88) | G | E1-U1, E1-U2 | b |
| F77 | `tx/tests/e2b.rs:119` | `control_rig_with` | 5 | D/W0 — `control_rig_with` <- `control_rig`: 5 tests | `shutdown_owner` (e2b.rs:88) in 1 test | G | E2B-16, N98-04, N98-05 | b |
| F78 | `tx/tests/e2b.rs:742` | `e2b_r2b_the_line_lock_is_free_inside_the_reopen` | this one | F/Wc — `Frozen::on` then `released(.. tx.pass())`: the freeze never holds anything the test reads; asserts are call counts | Frozen is released at once | G/R | - | a |
| F79 | `tx/tests/n75.rs:128` | `ur_75_a_stop_with_a_pending_recovery_then_three_unconfirmed_t3s` | this one | F/Wc — Stop with pending recovery, 3 unconfirmed T3s; `pass_on` | drop at n75.rs:154 (N) | G/R+N | - | a |

## 5. Tables R, H and the `drop` sites

### 5.1 R: the 39 `released` calls (all clock waits, all inline)

Each closure moves a value in and returns it; inline, the work runs on the guarded test thread and the closure, the `Send + 'static` ownership and the tuple threading disappear. The helpers `pass_on` (`tx.rs:1593`, 54 dynamic calls in 11 tests) and `pass_released` (`e3b.rs:410`) become `tx.pass()` and are deleted.

| # | Site | In | Tests | The work run as a participant | Treatment | Rows |
|---|---|---|---|---|---|---|
| R1 | `core.rs:2012` | `ur_22_a_recovery_reopen_flush_records_its_underflows_as_its_own` | 1 | `let (reopened, _frozen) = released(frozen, &inner.host(), { let core = core.clone(); move \|\| core.reopen_tx_streamer(0, Channels::first(2), i64::...` | inline | - |
| R2 | `core/t3.rs:88` | `strict` | 2 | `let (result, _frozen) = released(frozen, &inner.host(), { let core = core.clone(); move \|\| core.tx_flush_attempt(0, &channels, Some(attempt)) });` | inline | 5: N75-08a, N75-08b, .. |
| R3 | `core/t3.rs:105` | `ur_75_t3_ends_on_the_first_confirmed_attempt_on_a_fresh_streamer` | 1 | `let (confirmed, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| if reopen { core.reopen_tx_streamer(0, Channels::f...` | inline | N75-02, N75-16 |
| R4 | `core/t3.rs:138` | `ur_75_a_send_that_is_not_whole_ends_its_attempt_at_once` | 1 | `let (result, _frozen) = released(frozen, &spy.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MIN)....` | inline | N75-09, N75-10 |
| R5 | `core/t3.rs:173` | `ur_75_three_unconfirmed_attempts_fail_and_close_their_streamer` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| if reopen { core.reopen_tx_streamer(0, Channels::firs...` | inline | N75-01, N75-03, N75-04 |
| R6 | `core/t3.rs:230` | `ur_75_a_whole_send_is_confirmed_only_by_every_channel_position` | 1 | `let (confirmed, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(2), i64:...` | inline | N75-06 |
| R7 | `core/t3.rs:241` | `ur_75_a_whole_send_is_confirmed_only_by_every_channel_position` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(2), i64::MI...` | inline | N75-06 |
| R8 | `core/t3.rs:261` | `ur_75_the_confirmed_tick_is_the_largest_ack_tick_across_positions` | 1 | `let (confirmed, _frozen) = released(frozen, &inner.host(), { let core = core.clone(); move \|\| core.reopen_tx_streamer(0, Channels::first(2), i64:...` | inline | N75-07 |
| R9 | `core/t3.rs:328` | `ur_75_a_stale_ack_stays_for_the_report_owner_across_the_loop` | 1 | `let (result, _frozen) = released(frozen, &inner.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MIN...` | inline | - |
| R10 | `core/t3.rs:362` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (confirmed, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64:...` | inline | 6: N75-17, N75-20, .. |
| R11 | `core/t3.rs:376` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (confirmed, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64:...` | inline | 6: N75-17, N75-20, .. |
| R12 | `core/t3.rs:388` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R13 | `core/t3.rs:398` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R14 | `core/t3.rs:412` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R15 | `core/t3.rs:425` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R16 | `core/t3.rs:446` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R17 | `core/t3.rs:464` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R18 | `core/t3.rs:473` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | 6: N75-17, N75-20, .. |
| R19 | `core/t3.rs:493` | `ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | 1 | `let (confirmed, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64:...` | inline | 6: N75-17, N75-20, .. |
| R20 | `core/t3.rs:536` | `ur_75_stop_and_loss_at_each_boundary_of_the_retry` | 1 | `let (result, _frozen) = released(frozen, &spy.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MIN)....` | inline | N75-05a, N75-05c |
| R21 | `core/t3.rs:574` | `ur_75_a_stop_or_loss_published_around_a_successful_pre_send_time_read_sends_no_t3` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | N75-29 |
| R22 | `core/t3.rs:626` | `ur_75_an_open_that_fails_between_attempts_ends_the_loop` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.reopen_tx_streamer(0, Channels::first(1), i64::M...` | inline | - |
| R23 | `core/t3.rs:638` | `ur_75_an_open_that_fails_between_attempts_ends_the_loop` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.reopen_tx_streamer(0, Channels::first(1), i64::M...` | inline | - |
| R24 | `core/t3.rs:651` | `ur_75_reports_stashed_in_an_attempt_survive_the_next_open_for_the_report_owner` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | N75-30, N75-31 |
| R25 | `core/t3.rs:695` | `ur_75_after_three_failed_attempts_every_stash_stays_the_runs` | 1 | `let (result, _frozen) = released(frozen, &device.host(), { let core = core.clone(); move \|\| core.open_tx_confirmed(0, Channels::first(1), i64::MI...` | inline | - |
| R26 | `core/t3.rs:714` | `ur_75_with_the_switch_off_the_open_flush_is_todays` | 1 | `let (result, _frozen) = released(frozen, &spy.host(), { let core = core.clone(); move \|\| core.open_streamer(Dir::Tx, 0, Channels::first(1)) });` | inline | N75-13, N75-14 |
| R27 | `core/t3.rs:742` | `ur_75_with_the_switch_off_the_open_flush_is_todays` | 1 | `let (result, _frozen) = released(frozen, &device.host(), move \|\| core.reopen_tx_streamer(0, Channels::first(1), i64::MIN).map(\|_\| ()));` | inline | N75-13, N75-14 |
| R28 | `tx.rs:1594` | `pass_on` | 11 | `let ((tx, worked), frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; let worked = tx.pass(); (tx, worked) });` | inline | 9: N75-26, N75-27, .. |
| R29 | `tx.rs:4548` | `ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst` | 1 | `let ((mut tx, passed), frozen) = released(frozen, &device.host(), move \|\| { let passed = tx.pass(); (tx, passed) });` | inline | P1172, P1191 |
| R30 | `tx.rs:4614` | `ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery` | 1 | `let ((mut tx, recovered), frozen) = released(frozen, &device.host(), move \|\| { let recovered = tx.pass(); (tx, recovered) });` | inline | N48S1-02, E3B-38, E3B-39 |
| R31 | `tx/tests/e2b.rs:648` | `e2b_c7_a_stop_during_a_reopen_is_read_after_its_result` | 1 | `let (tx, _frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; assert!(tx.pass(), "the reopen turn"); tx.pass(); tx });` | inline | - |
| R32 | `tx/tests/e2b.rs:677` | `e2b_c9a_a_loss_just_before_the_reopen_s_result_section_leaves_the_mark_cleared` | 1 | `let ((tx, armed), _frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; tx.pass(); (tx, next_lock_armed()) });` | inline | E2B-11 |
| R33 | `tx/tests/e2b.rs:694` | `e2b_c9c_a_loss_just_before_a_failed_reopen_s_result_section_ends_no_clock` | 1 | `let ((tx, armed), _frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; tx.pass(); (tx, next_lock_armed()) });` | inline | E2B-19 |
| R34 | `tx/tests/e2b.rs:742` | `e2b_r2b_the_line_lock_is_free_inside_the_reopen` | 1 | `let (worked, _frozen) = released(Frozen::on(host.clone()), &host, move \|\| tx.pass());` | inline | - |
| R35 | `tx/tests/e3b.rs:186` | `e3b_c1_the_front_after_each_close` | 1 | `let ((tx, recovered), _frozen) = released(frozen, &device.host(), move \|\| { let recovered = tx.pass(); (tx, recovered) });` | inline | E3B-42 |
| R36 | `tx/tests/e3b.rs:411` | `pass_released` | 2 | `let ((tx, worked), frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; let worked = tx.pass(); (tx, worked) });` | inline | - |
| R37 | `tx/tests/e3b.rs:504` | `e3b_c3_the_chain_through_its_lifecycles` | 1 | `let ((tx, ()), _frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; tx.shutdown(); (tx, ()) });` | inline | - |
| R38 | `tx/tests/e3b.rs:699` | `e3b_c4_the_streams_lock_sections` | 1 | `let ((tx, counted), _frozen) = released(frozen, &device.host(), move \|\| { let mut tx = tx; let mut worked = false; let sections = streams_section...` | inline | - |
| R39 | `tx/tests/o1.rs:732` | `o1_uhd_t6d_a_booking_after_a_stop_survives_the_failing_close_of_the_burst_it_cut` | 1 | `let ((mut tx, passed), frozen) = released(frozen, &device.host(), move \|\| { let passed = tx.pass(); (tx, passed) });` | inline | - |

### 5.2 H: the 13 `handed_over` calls

The guard is held from F, so the hand-over (register the thread, then release the frozen participant) is a no-op and goes. Order is no longer a rule (the thread is registered first by construction); `N51D-02`'s pin goes with it.

| # | Site | In | Tests | After the hand-over (what the test thread runs) | Treatment | Rows |
|---|---|---|---|---|---|---|
| H1 | `mod.rs:1140` | `ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup` | 1 | `rx_recv(.., 100 ms)` x6 (clock waits), then a time assertion | hand-over line deleted (the guard is held from F) | P1292, N92-7c |
| H2 | `rx.rs:1044` | `ur_16_an_open_stops_the_streamer_before_its_first_start_and_cancels_an_orphaned_start` | 1 | `rx_recv(.., 100 ms)` x6 (clock waits), then a time assertion | hand-over line deleted (the guard is held from F) | N67F-21 |
| H3 | `rx.rs:2331` | `assert_silent_through` | 1 | `rx_recv(.., 100 ms)` x6 (clock waits) through the origin (1 of the 5 `queued` tests calls it) | hand-over line deleted (the guard is held from F) | N67F-01 |
| H4 | `tx.rs:6714` | `ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst` | 1 | `tx.shutdown()` (100 ms acknowledgement window = clock wait) | hand-over line deleted (the guard is held from F) | - |
| H5 | `tx.rs:6727` | `ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` | 1 | `tx.pass()` (reopen: flush waits on the clock) | hand-over line deleted (the guard is held from F) | N47S4-20 |
| H6 | `tx.rs:6743` | `ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` | 1 | `tx.pass()` (reopen) | hand-over line deleted (the guard is held from F) | N47S4-51 |
| H7 | `tx.rs:6766` | `ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` | 1 | 50 ms `thread::sleep` + clock-equality assertion (N51D-03), then `tx.pass()` (reopen) and `turns` | hand-over line deleted (the guard is held from F) | N51D-03 |
| H8 | `tx.rs:7178` | `ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch` | 1 | `mark_reopen_needed()`, `rig.tx.pass()` | hand-over line deleted (the guard is held from F) | N47S5-44 |
| H9 | `tx/tests/c2.rs:622` | `c2_s15b_a_retried_recovery_settles_a_pending_switch_after_the_window_and_a_failed_one_cancels_it` | 1 | `tx.pass()` (reopen), `device_at`, `owner_turn` | hand-over line deleted (the guard is held from F) | - |
| H10 | `tx/tests/c2.rs:644` | `c2_s15b_a_retried_recovery_settles_a_pending_switch_after_the_window_and_a_failed_one_cancels_it` | 1 | `tx.pass()` with a `t3_between` probe, in a loop of 3 cases | hand-over line deleted (the guard is held from F) | - |
| H11 | `tx/tests/e1.rs:88` | `shutdown` | 6 | `tx.shutdown()` (clock wait); `shutdown(rig)` helper for 6 tests | hand-over line deleted (the guard is held from F) | 4: E1-U1, E1-U2, .. |
| H12 | `tx/tests/e2b.rs:88` | `shutdown_owner` | 7 | `tx.shutdown()`; `shutdown_owner` helper for 7 tests | hand-over line deleted (the guard is held from F) | 5: E2B-13, E2B-14, .. |
| H13 | `tx/tests/n75.rs:296` | `abandoned` | 2 | `abandoned()` helper returns the `Driver`; 2 tests | hand-over line deleted (the guard is held from F) | - |

### 5.3 The 18 test-level `drop(frozen)`

(1d's types: N assertions only, G owner code on the test thread, W1 waits off the clock, W2 sleeping helper + join. No W2 is left; 1d rewrote them.)

| Site | Test | Type (1d §4.4 names) | What follows the drop | Treatment |
|---|---|---|---|---|
| `tx/tests/c2.rs:581` | `c2_s15_a_failed_reopen_cancels_even_a_sent_switch` | G (owner code on the test thread after the drop) | added by e4493972 (note 49 C2), after 1d's recount; `rig.tx.pass()` runs unguarded; the reopen fails at its open (`TxOpenFails(1)`) so no flush wait is reached | hold to the end (the drop goes); not a 1d `handed_over` shape, no pin needed (the verdict does not read device time) |
| `tx/tests/n75.rs:79` | `ur_75_a_retry_longer_window_leaves_held_bookings_to_their_late_policy` | N | last statement of the loop body (keeps `frozen` alive through `pass_on` threading) | delete the drop |
| `tx/tests/n75.rs:154` | `ur_75_a_stop_with_a_pending_recovery_then_three_unconfirmed_t3s` | N | last statement of the failing branch | delete the drop |
| `tx/tests/n75.rs:217` | `ur_75_the_owner_s_recovery_waits_out_failing_time_reads_and_ends_failed_only_at_the_cap` | N | last statement of the loop body | delete the drop |
| `tx/tests/o1.rs:753` | `o1_uhd_t6d_a_booking_after_a_stop_survives_the_failing_close_of_the_burst_it_cut` | N | last statement of the test | delete the drop |
| `tx.rs:1686` | `ur_22_an_unconfirmed_t3_is_retried_and_recovery_ends_only_on_a_confirmed_one` | N | last statement of the loop body | delete the drop |
| `tx.rs:1706` | `ur_22_a_flush_ack_below_the_old_burst_s_padded_end_costs_an_attempt` | N | last statement of the test | delete the drop |
| `tx.rs:1764` | `ur_22_three_unconfirmed_t3s_end_the_transmit_stream` | N | last statement of the loop body | delete the drop |
| `tx.rs:4567` | `ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst` | N | last statement of the loop body | delete the drop |
| `tx.rs:4635` | `ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery` | N | last statement of the test | delete the drop |
| `rx.rs:1001` | `ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` | N | assertions on `spy` calls and events only follow | delete the drop |
| `rx.rs:1060` | `ur_16_a_failed_stop_on_open_refuses_the_segment_and_starts_nothing` | N | assertions on `spy`/`events`/`core` only follow | delete the drop |
| `rx.rs:1077` | `ur_29_a_loss_found_by_the_stop_on_open_is_published_once_and_nothing_follows` | N | assertions on `spy`/`events`/`core` only follow | delete the drop |
| `rx.rs:902` | `ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead` | W1 | owner participant already spawned; the test then waits on `gate.wait()`, `finished.recv_timeout(10 s)`, `join` while the owner does `rx_recv` with 6.5 ms latency | `drop(driver)` at the same line (a guard kept across these waits would stop the clock the owner needs: hang) |
| `rx.rs:947` | `ur_24_a_far_future_cut_keeps_pre_cut_receives_multi_packet` | W1 | owner participant stepped by channel turns; test waits on `recv_timeout`, `join` | `drop(driver)` at the same line |
| `tx.rs:6124` | helper `txp_owners` (4 tests: `ur_23_p1_p2_…`, `ur_23_p3_…`, `ur_23_p4_…`, `ur_23_p5_p6_…`) | W1 | owners are `host.spawn` participants; test spins `thread::sleep(1 ms)` under a 10 s `Instant`, then `txp_join` | `drop(driver)` at the same line |
| `tx.rs:6231` | `ur_23_m8b_a_waiting_owner_sleeps_an_idle_period_between_looks` | W1 | `snapshots.recv_timeout(10 s)` while the owner takes idle looks (clock waits) | `drop(driver)` at the same line |
| `tx.rs:6264` | `ur_23_m6_a_waiting_owner_wakes_for_a_stop_within_an_idle_period` | W1 | `woken_rx.recv_timeout(10 s)`, a 1 ms real-time poll under a 10 s deadline, `txp_join` | `drop(driver)` at the same line |

Result by type: **5 W1, 12 N, 1 G**, 0 W2.

## 6. Mutation rows

### 6.1 What can change

Assertions are not edited, so a row killed by an assertion in a converted test is killed by the same assertion afterwards (I; V only by running the rows). The rows to run are the 249 selected by the 350 tests (Appendix B). Specific points:

- **`N51D-01`** (`handed_over` registers on a real clock: killer `handed_over_holds_the_clock_while_the_test_thread_is_slow`): its anchor `let driver = clock.drive();` disappears in 4a. Replace by a row on `Frozen::on`'s registration (old: `clock.drive()` inside `Frozen::on`; new: `HostClock::real().drive()`); killers: the rewritten helper test (hold, 100 ms real sleep, equality) plus `ur_28_…` and the 326 hold-sensitive tests. In 4c the row moves to the `clock_guard()` helper that replaces `Frozen::on`.
- **`N51D-02`** (release before registration): obsolete in 4a (nothing is released; the helper test that pinned the order needs `Send` and is deleted). Retire it. The property it pinned (no instant where the clock can run between the hold and the driver) becomes true by construction.
- **`N51D-03`** (`ur_28_…`: `handed_over` replaced by `drop(frozen)`): its anchor text `let _driver = handed_over(frozen, &device.host()); let held = device.host().now();` is the only row anchor inside a converted test. It still matches in 4a (the line stays) and still kills (`drop(frozen)` drops the driver, the 50 ms sleep moves the clock, the equality assertion fails). In 4c retarget it to the converted text (drop the guard before the slow step) and keep the equality assertion as its killer.
- **`J58`** (`provider::`, equivalent, declared `bound`): selects every converted test; it must keep surviving as equivalent, and its declaration is unchanged because the set of real-clock tests is unchanged. Run it once in the full lists, not per sub-step.
- **`J53`** (`ur_29_a_`, 13 tests, 4 users): run with the rows of 4b/4c.
- The 174 users with no row are guarded by construction: nothing kills a missing guard except the shared row in 4a and the hundreds of hold-sensitive tests; that is the "no row, said plainly" of 1d for the same reason.
- No row has `"kill": "bound"` or `equivalent` among the 249 (checked); no kill depends on a wall-clock bound.

### 6.2 Row sets per sub-step

4a: the 248 of Appendix B, the replacement of `N51D-01`, and `E2A2A-10` (its file is `test_support.rs`; 1d re-ran it for the same reason; expected `killed (bound)`). 4b: 56. 4c: 192 (`N51D-03` retargeted in it). 4d: the full lists (`all`) at the gate, `E2A2A-10` included.

### 6.3 Row anchors

`rules.py --base 1750373` must find every row exactly once after each sub-step; `N51D-01/-02/-03` are the only rows to touch.

### 6.4 Tests that pass with no hold (3 of 3 runs), not "redundant"

- `core::t3::ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap`
- `core::t3::ur_75_a_send_that_is_not_whole_ends_its_attempt_at_once`
- `core::t3::ur_75_a_stale_ack_stays_for_the_report_owner_across_the_loop`
- `core::t3::ur_75_a_stop_or_loss_published_around_a_successful_pre_send_time_read_sends_no_t3`
- `core::t3::ur_75_a_whole_send_is_confirmed_only_by_every_channel_position`
- `core::t3::ur_75_after_three_failed_attempts_every_stash_stays_the_runs`
- `core::t3::ur_75_an_open_that_fails_between_attempts_ends_the_loop`
- `core::t3::ur_75_reports_stashed_in_an_attempt_survive_the_next_open_for_the_report_owner`
- `core::t3::ur_75_stop_and_loss_at_each_boundary_of_the_retry`
- `core::t3::ur_75_t3_ends_on_the_first_confirmed_attempt_on_a_fresh_streamer`
- `core::t3::ur_75_the_open_flush_at_arm_keeps_todays_ack_rule`
- `core::t3::ur_75_three_unconfirmed_attempts_fail_and_close_their_streamer`
- `core::t3::ur_75_with_the_switch_off_the_open_flush_is_todays`
- `rx::tests::ur_16_a_failed_stop_on_open_refuses_the_segment_and_starts_nothing`
- `rx::tests::ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls`
- `rx::tests::ur_24_a_loss_published_before_rx_recv_skips_the_device_call`
- `rx::tests::ur_24_a_missed_start_after_a_loss_makes_no_restart_call`
- `rx::tests::ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain`
- `rx::tests::ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start`
- `rx::tests::ur_29_a_loss_found_by_the_stop_on_open_is_published_once_and_nothing_follows`
- `tx::tests::e2b::e2b_r2b_the_line_lock_is_free_inside_the_reopen`
- `tx::tests::n75::ur_75_a_stop_with_a_pending_recovery_then_three_unconfirmed_t3s`
- `tx::tests::ur_22_a_partial_ack_on_every_attempt_is_unconfirmed`

Three idle runs show only that the other 326 observe the hold. The 13 `core/t3.rs` tests compare device-time elapsed with `SLACK_MS = 40` for exact waits: the hold is what makes that slack sufficient on a loaded host. The 7 `rx.rs` ones (`ur_16_a_failed_stop…`, `ur_24_a_loss_during_rx_receive…`, `ur_24_a_loss_published_before…`, `ur_24_a_missed_start…`, `ur_24_nonloss_rx_stop…`, `ur_29_a_begin_rechecks…`, `ur_29_a_loss_found_by…`) and `e2b_r2b`, `n75`'s `ur_75_a_stop_with_a_pending_recovery…`, `tx.rs`'s `ur_22_a_partial_ack…` assert on call counts, events and records (read for `rx.rs:1092`, `:1113`, `:1475`, `:985`, `:1019`, `:2673` and `e2b_r2b`), so deleting their hold is plausible, but it is not needed and each deletion would be a separate argument. Recommendation: keep one guard line in every one of them (the uniform rule is cheaper than 23 proofs).

## 7. Can `Frozen` be removed?

Yes. After 4d: `test_support.rs` loses `Frozen` (about 30 lines), `released` (14), `handed_over` (5), the two helper tests and their rows; it gains `clock_guard(clock) -> Driver` (manual assertion kept: a bare `drive()` on a real clock registers nothing silently, which the old `Frozen::on` refused). Nothing in the 350 tests needs `Send` for the hold, a thread other than the test thread, or a hold that outlives a `Driver`. What stays: `HostClock::spawn` participants for the owner threads the tests start (W1), `drive()` in `tests/common` (`Direct`, `UhdRig`: untouched).

## 8. Sub-steps (each leaves the tree green)

Risk: 4a is **risky** (an identity: which thread is a participant; a test plan first, Sol on the plan and code, AGENTS §9 steps 2-3, 7). 4b-4d are mechanical against 4a's proof (not risky; a per-sub-step Opus review, Sol's code review at the end).

| Step | Change | About | Moved tests | Rows |
|---|---|---|---|---|
| **4a** | `test_support.rs`: `clock_guard(clock)`; `Frozen` becomes a `Driver` wrapper; `released` inline; `handed_over` gives the driver back; rewrite helper test 1; delete helper test 2; rows `N51D-01` replaced, `N51D-02` retired | 80 lines | all 350 | the 248 + the new `N51D-01` |
| **4b** | `core/t3.rs` (7 F sites, 26 R), `core.rs` (1, 1), `rx.rs` (15 F, 2 H, 5 drops), `mod.rs` (1 F, 1 H), `control.rs` (`hold`, `staging_rig` types): direct `clock_guard()`/`Driver`, inline work, delete hand-overs and N drops | 250 | 48 (+ the helper test in 4a) | 56 |
| **4c** | `tx.rs` (50 F, 3 R + `pass_on`, 5 H, 8 drops) and `tx/tests/{c2,e1,e2b,e3b,n75,o1}.rs`: `SwitchRig._frozen` and `OwnerRig` hold a `Driver`; the same edits; `c2.rs:581`; `tx.rs:1852` (below); `N51D-03` retargeted | 450 | 301 | 192 |
| **4d** | delete `Frozen`, `released`, `handed_over`; unused imports; the texts of §10; `N51D-01` moves to `clock_guard` | 150 | none new | full lists |

`tx.rs:1852` (**other**, no `Frozen`): the test `ur_26_provider_stop_ends_recovery_after_stop_and_keeps_shutdown_report_reads` (row `P1302`) is the one 1d-C′ site left (1d found three; the other two are no longer in the tree: INFERRED that #75's rewrite of the retry tests removed them, not traced): a `tx-shutdown-deadline` participant sleeps 150 ms so `tx.shutdown()`'s clock wait ends, then `join`. Same rewrite as 1d's W2: `let _driver = host.drive(); tx.shutdown();`, no helper, no join. In 4c. The other no-`Frozen` participant tests (`tx.rs:1432`, `:3663`, 1d's §1.3 list of six) are not `Frozen` uses and stay out of this stage.

**Exit checks per sub-step**, each on the sub-step's moved tests (4a: all 667 lib tests):
1. `cargo test -p ezsdr-radio-uhd --lib` green, `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `rules.py --base 1750373` = 0.
2. 20 serial runs: `python3 plan/maintenance/tools/remote/rdev.py -- bash -c 'for i in $(seq 20); do cargo test -p ezsdr-radio-uhd --lib -q -- --test-threads=1 <the sub-step's filters> || exit 1; done'` (filters: 4b `provider::core provider::rx provider::tests provider::control`; 4c `provider::tx`).
3. 3 x 8 parallel, through `rdev.py -- bash -c '...'` (the brief's rule): `cargo test -p ezsdr-radio-uhd --lib --no-run`, then run the built test binary 8 times at once, three rounds, failing if any run exits non-zero (24 runs, 0 failures expected; this is how the shim was run here, locally: 24/24, 14 s each).
4. Rows through `remote_mutate.py`: `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:7,sim03:7 <the sub-step's ids>` (Appendix B). A row that was `killed` must stay `killed`; no `ERROR`.
5. 4d only, the orchestrator's gate (not the implementer's): `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/gate.py --out <dir outside the checkout> --mutation all --base 1750373` (the full lists, J58 included).
For 4a the 20 serial and the 3 x 8 are on all 667 tests; 4a's delay injection (5 ms after `device_at`, 20 ms around inlined work) is the extra diagnostic, not a gate.

## 9. Not decided, plan rules that do not fit, and my recommendation

- **Q1. The stepped rigs lose a loud failure.** A test that waits on the clock under `Frozen` hangs (detected only by the test timeout); under the guard the same stray wait advances time silently. `device_at`'s equality assertion catches a run of the clock *between* `set_time` and the read, and a drift past the next set target, but not a smaller drift between steps (I). Today: 0 occurrences in 287 hold-only tests (V); in the stepped stretches of the 63 R/H tests the same silent advance is possible and unmeasured (I). Recommendation: accept, and add no mechanism (a check in `host.rs` would be production code for a test property; the three-strikes rule is not at stake: no bug of this kind exists). Say it in `tests/README.md` where "device time moves only through `device_at`" is stated: it holds because the test thread never waits on the clock.
- **Q2. The plan's keep-case.** Not present: see §0 and §2.5. `rx.rs:1113` and `:1475` are the sites that match its description (a non-clock wait for an owner's reaction under a kept hold); they pass with the guard. Recommendation: say so in §4.4, strike "keep `Frozen`" from Q6, and keep the W1 rule as "the guard must be dropped where the Frozen is dropped".
- **Q3. Helper shape.** `clock_guard(clock: &HostClock) -> Driver` with the manual assertion (recommended) against a bare `clock.drive()` at 79 sites (silent no-op on a real clock). The name is free (`held(&clock, target, ..)` is a burst helper in `tx.rs`, `hold` exists in `control.rs` and `tx.rs`); the implementer may pick another. Decided by the orchestrator; I recommend `clock_guard`, one line per site and one place for the row.
- **Q4. The 23 no-hold tests** (§6.4): keep the guard (recommended) or delete; no site is classified "delete as redundant".
- **Q5. `SLACK_MS = 40`** (`core/t3.rs`): left alone in this stage. After inlining, the work still runs on a participant and M4 stops the clock at a deadline until it runs; tightening it is a different change.
- **Q6. `c2.rs:581`** (G shape, not in 1d's table): hold to the end. INFERRED that no clock wait is reached (the reopen fails at its open, `TxOpenFails(1)` = the second open); the 4c exit check runs it 20 x serial and 3 x 8.
- **Q7. Stop after 4a?** Possible (tree green, 350 tests on the guard, 15 lines of wrapper and an identity `released` remain). Not recommended: 39 closure sites and two helper shapes are the leftover AGENTS §9 "Leftovers" asks the step to remove; `50-remaining-work.md:37` would otherwise carry the rest to the Phase-8-end review.
- **I (inferred):** (a) the 249 rows keep their kills; (b) hooks armed inside inlined work do not leak (green suite, not a proof); (c) `c2.rs:581` reaches no clock wait; (d) no hang under load in the converted tree (V only for the shim).

**Plan text that does not fit:** §4.4 "W1 ... 変えない (guard を足すと hang)" is right only where the Frozen is dropped before the wait; Q6 (§11 appendix) "keep Frozen where time must stay stopped while the test waits for an owner's reaction (the guard cannot do that)"; §7.1 and §7.2 row 4 "`Frozen` 約 75"; §8 S1 row; `51-tables.md` §2.5 title "17" and T13's "10 `drop(frozen)` sites left after 1d" (18 test-level now: 5 W1, 12 N, 1 G).

## 10. Texts the change makes false (find with `grep -rnE 'Frozen|frozen|released|handed_over' <path>`)

- `plan/phase8/51-test-timing.md`: line 57, §3 P5 (line 99), §4.4 (table, lines 149-157), §7.1 (line 383), §7.2 row 4 (404), §8 S1 row, Q6 (514), the appendix paragraphs "段 1d の後"; `plan/phase8/51-tables.md` §2.5 (115-137), line 113, T13 (349); `plan/phase8/00-overview.md` (21) and `37-test-thread-timing.md` lines 53, 63, 87 (history, leave); `handoff.md`; `crates/ezsdr-radio-uhd/tests/README.md:94`.
- Source comments: `core/t3.rs:3` ("held still (`Frozen`) ... run as a participant (`released`)"), `tx/tests/e2b.rs:5` (`released` and "Probes and `on_next_lock` hooks run on the owner's thread": after the change the owner's thread is the test thread), `tx/tests/e3.rs:3` ("the frozen single-threaded ones"), `control.rs:761-765`, `device.rs:1115` (`test_support::Frozen`), `tx.rs:1592` and `e3b.rs:408` (helper docs), and the `Frozen`/`released`/`handed_over` doc comments in `test_support.rs`.

## 11. Probe conditions (so the numbers can be rerun)

Probe copy of `1750373` made with `rsync -a --no-times --exclude .cargo --exclude target --exclude v3 --exclude .git`, files touched, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, local PC, 16 cores. Shim = §2.1; negative control = §2.2; idle probe in `host.rs::park` and the clock-lock wait; injection = §2.3. The shim is a probe, not a design: the real 4a puts the `clock_guard()` helper first and keeps `Frozen` only as the wrapper until 4d.

## Appendix A: the 349 tests (the 350th is `test_support`'s own helper test)

Shape: `hold` = held to the end of the test or helper; `R` = reaches a `released`; `H` = reaches a `handed_over`; `drop` = a `drop(frozen)` in the test body (`txp_owners`' W1 `drop` is in its helper). F = the constructor row of §4.

| Test (`provider::` omitted) | F | Shape | Test-thread clock waits under the guard | Passes with no hold (3/3) | Rows |
|---|---|---|---|---|---|
| `control::tests::ur_25_a_booking_the_line_cannot_represent_is_refused_with_the_start_s_id` | F1 | hold | 0 |  | N47S3-38 |
| `control::tests::ur_26_a_peer_stream_s_state_does_not_matter_and_an_instance_stop_stops_both` | F1 | hold | 0 |  | - |
| `control::tests::ur_26_a_staged_value_with_an_instant_or_another_target_is_refused_and_the_instance_and_its_streams_are_accepted` | F1 | hold | 0 |  | N47S3-16 |
| `control::tests::ur_26_a_start_of_a_streaming_stream_is_refused_with_the_start_s_id_and_leaves_the_line_unchanged` | F1 | hold | 0 |  | N47S3-36 |
| `control::tests::ur_26_a_stop_after_a_booked_future_start_does_not_make_the_stream_stopped` | F1 | hold | 0 |  | - |
| `control::tests::ur_26_a_value_after_a_booked_start_is_refused_even_before_its_instant` | F1 | hold | 0 |  | - |
| `control::tests::ur_26_a_value_for_a_streaming_stream_is_refused_without_a_device_call` | F1 | hold | 0 |  | P1392, N47S3-15 |
| `control::tests::ur_26_d14_a_layout_instance_names_its_streams` | F1 | hold | 0 |  | - |
| `control::tests::ur_26_the_last_value_before_a_start_is_the_one_it_snapshots_and_a_second_stop_leaves_it` | F1 | hold | 0 |  | - |
| `core::tests::ur_22_a_recovery_reopen_flush_records_its_underflows_as_its_own` | F2 | R | 1 |  | - |
| `core::t3::ur_75_a_failing_pre_send_time_read_is_retried_until_it_succeeds_a_stop_a_loss_or_the_cap` | F3 | R | 8380 | yes | N75-17, N75-20, N75-28, N75-32, N75-33, N75-34 |
| `core::t3::ur_75_a_send_that_is_not_whole_ends_its_attempt_at_once` | F5 | R | 3 | yes | N75-09, N75-10 |
| `core::t3::ur_75_a_stale_ack_stays_for_the_report_owner_across_the_loop` | F7 | R | 3 | yes | - |
| `core::t3::ur_75_a_stop_or_loss_published_around_a_successful_pre_send_time_read_sends_no_t3` | F3 | R | 2 | yes | N75-29 |
| `core::t3::ur_75_a_whole_send_is_confirmed_only_by_every_channel_position` | F3 | R | 5 | yes | N75-06 |
| `core::t3::ur_75_after_three_failed_attempts_every_stash_stays_the_runs` | F3 | R | 4 | yes | - |
| `core::t3::ur_75_an_ack_older_than_the_floor_is_not_this_attempts` | F4 | R | 19 |  | N75-08a, N75-08b, N75-08c, N75-18 |
| `core::t3::ur_75_an_open_that_fails_between_attempts_ends_the_loop` | F3 | R | 3 | yes | - |
| `core::t3::ur_75_reports_stashed_in_an_attempt_survive_the_next_open_for_the_report_owner` | F3 | R | 4 | yes | N75-30, N75-31 |
| `core::t3::ur_75_stop_and_loss_at_each_boundary_of_the_retry` | F8 | R | 6 | yes | N75-05a, N75-05c |
| `core::t3::ur_75_t3_ends_on_the_first_confirmed_attempt_on_a_fresh_streamer` | F3 | R | 12 | yes | N75-02, N75-16 |
| `core::t3::ur_75_the_confirmed_tick_is_the_largest_ack_tick_across_positions` | F6 | R | 0 |  | N75-07 |
| `core::t3::ur_75_the_open_flush_at_arm_keeps_todays_ack_rule` | F4 | R | 4 | yes | N75-19 |
| `core::t3::ur_75_three_unconfirmed_attempts_fail_and_close_their_streamer` | F3 | R | 8 | yes | N75-01, N75-03, N75-04 |
| `core::t3::ur_75_with_the_switch_off_the_open_flush_is_todays` | F3,F9 | R | 7 | yes | N75-13, N75-14 |
| `tests::ur_15_a_failed_rx_owner_spawn_abandons_its_started_chain_before_cleanup` | F10 | H | 6 |  | P1292, N92-7c |
| `rx::tests::ur_16_a_failed_stop_on_open_refuses_the_segment_and_starts_nothing` | F15 | N drop | 0 | yes | N67F-22 |
| `rx::tests::ur_16_an_open_stops_the_streamer_before_its_first_start_and_cancels_an_orphaned_start` | F16 | H | 6 |  | N67F-21 |
| `rx::tests::ur_17_a_finite_segment_delivers_through_its_cut` | F20 | hold | 0 |  | P1396, P1397 |
| `rx::tests::ur_24_a_cut_receive_uses_one_short_packet_and_preserves_successor_lead` | F12 | W1 (drop in body) | 0 |  | P1238 |
| `rx::tests::ur_24_a_far_future_cut_keeps_pre_cut_receives_multi_packet` | F13 | W1 (drop in body) | 0 |  | P1242 |
| `rx::tests::ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls` | F17 | hold | 0 | yes | P1256 |
| `rx::tests::ur_24_a_loss_published_before_rx_recv_skips_the_device_call` | F18 | hold | 0 | yes | P1295 |
| `rx::tests::ur_24_a_missed_start_after_a_loss_makes_no_restart_call` | F23 | hold | 0 | yes | P1290 |
| `rx::tests::ur_24_fake_host_silence_delivers_no_block` | F11 | hold | 0 |  | - |
| `rx::tests::ur_24_nonloss_rx_stop_failure_leaves_the_chain_uncertain` | F14 | N drop | 0 | yes | N47S3-21 |
| `rx::tests::ur_25_a_configure_that_ends_after_the_origin_is_a_missed_start_reported_once` | F25 | hold | 0 |  | - |
| `rx::tests::ur_25_a_configure_that_leaves_less_than_the_declared_lead_reports_it_and_starts_at_the_origin` | F24 | hold | 0 |  | N47S3-13, N47S3-14 |
| `rx::tests::ur_25_a_configure_that_leaves_the_declared_lead_whole_reports_nothing` | F24 | hold | 0 |  | N47S3-12 |
| `rx::tests::ur_25_a_short_lead_and_a_missed_start_are_each_reported_once` | F25 | hold | 0 |  | - |
| `rx::tests::ur_26_a_cut_after_a_queued_origin_keeps_the_stream` | F21 | hold | 0 |  | N67F-02 |
| `rx::tests::ur_26_a_cut_at_a_queued_origin_stops_the_stream_at_once` | F21 | H | 72 |  | N67F-01 |
| `rx::tests::ur_26_a_start_after_a_cancelled_queued_start_delivers_at_its_own_origin` | F22 | hold | 0 |  | N67F-05 |
| `rx::tests::ur_26_a_stop_and_provider_stop_in_one_turn_stop_a_queued_start_once` | F21 | hold | 0 |  | N67F-03 |
| `rx::tests::ur_29_a_begin_rechecks_loss_after_acquiring_the_chain_before_rx_start` | F19 | hold | 0 | yes | J53, P1291 |
| `rx::tests::ur_29_a_loss_found_before_the_stop_of_a_queued_start_makes_no_stop_call` | F21 | hold | 0 |  | J53, N67F-10 |
| `rx::tests::ur_29_a_loss_found_by_the_stop_on_open_is_published_once_and_nothing_follows` | F15 | N drop | 0 | yes | J53, N67F-23 |
| `rx::tests::ur_29_a_stop_of_a_queued_start_that_finds_the_loss_publishes_it_once` | F21 | hold | 0 |  | J53, N67F-11 |
| `tx::tests::measurement_candidate_in_flight_window_sets_the_tx_chunk_size` | F43 | hold | 0 |  | P1160 |
| `tx::tests::ur_21_a_burst_inside_the_in_flight_window_is_refused` | F57 | hold | 0 |  | - |
| `tx::tests::ur_21_a_finite_burst_inside_the_window_is_refused` | F46 | hold | 0 |  | - |
| `tx::tests::ur_21_a_finite_burst_with_a_held_tail_keeps_its_front` | F54 | hold | 0 |  | E3A-U04 |
| `tx::tests::ur_21_a_late_joined_burst_cannot_move_and_is_refused` | F66 | hold | 0 |  | N47S5-61 |
| `tx::tests::ur_21_a_late_move_into_a_handed_repeat_is_refused` | F48 | hold | 0 |  | - |
| `tx::tests::ur_21_a_new_burst_without_an_open_predecessor_is_on_time` | F44 | hold | 0 |  | - |
| `tx::tests::ur_21_a_partial_configure_refusal_sends_nothing_and_the_retry_configures_both` | F72 | hold | 0 |  | - |
| `tx::tests::ur_21_a_post_eob_restart_uses_the_safe_sample` | F56 | hold | 0 |  | P640, P641, P642 |
| `tx::tests::ur_21_a_refused_burst_stays_counted_until_its_booked_end` | F73 | hold | 0 |  | N47S4-13b, N47S5-89 |
| `tx::tests::ur_21_a_refused_configure_refuses_the_burst_and_the_fixed_value_plays_on_the_same_clock` | F72 | hold | 0 |  | N47S4-12, N47S4-16, N47S4-14, N98-03 |
| `tx::tests::ur_21_a_send_asap_judged_again_at_dispatch_is_not_reported_again` | F37 | hold | 0 |  | P667 |
| `tx::tests::ur_21_a_send_asap_move_is_reported_once_across_handovers` | F36 | hold | 0 |  | - |
| `tx::tests::ur_21_a_short_head_front_follows_the_stream_s_rate` | F66 | hold | 0 |  | N49F-27 |
| `tx::tests::ur_21_a_target_after_a_completed_burst_is_on_time` | F39 | hold | 0 |  | - |
| `tx::tests::ur_21_m1_148us_booked_early_at_the_chain_front_plays_on_time` | F66 | hold | 0 |  | N47S5-56 |
| `tx::tests::ur_21_m1_joins_booked_early_continue_the_device_burst` | F66 | hold | 0 |  | - |
| `tx::tests::ur_21_m1_receipt_at_the_deadline_is_decided_by_the_record` | F66 | hold | 0 |  | N47S5-58 |
| `tx::tests::ur_21_metadata_is_empty_or_the_mark_only` | F75 | hold | 0 |  | N47S5-45, N47S5-92, N0-1 |
| `tx::tests::ur_21_s10_a_repeat_after_a_stop_keeps_its_own_period` | F75 | hold | 0 |  | N47S5-53 |
| `tx::tests::ur_21_s15_c_untimed_head_moved_past_its_joined_piece_goes_after_the_piece` | F32 | hold | 0 |  | N47S5-123 |
| `tx::tests::ur_21_s15_d_untimed_head_moved_before_a_later_booking_keeps_the_bound` | F32 | hold | 0 |  | N47S5-130 |
| `tx::tests::ur_21_s15_host_lateness_moves_an_untimed_burst_silently` | F32 | hold | 0 |  | N47S5-124, N47S5-125, N47S5-129 |
| `tx::tests::ur_21_s1_overlap_refused_at_receipt` | F28 | hold | 0 |  | N47S5-19 |
| `tx::tests::ur_21_s2_a_finite_front_stops_at_its_end` | F45 | hold | 0 |  | - |
| `tx::tests::ur_21_s2_a_short_surviving_head_is_closed_and_a_piece_joined_behind_it_is_late` | F72 | hold | 0 |  | - |
| `tx::tests::ur_21_s2_c_and_d_joined_behind_a_refused_head_are_late_at_dispatch` | F72 | hold | 0 |  | - |
| `tx::tests::ur_21_s4_a_booking_at_the_finite_front_is_on_time` | F49 | hold | 0 |  | - |
| `tx::tests::ur_21_s4_a_burst_past_the_abandoned_repeat_s_cursor_is_accepted_on_time` | F75 | hold | 0 |  | N47S5-139 |
| `tx::tests::ur_21_s4_a_late_repeat_dropped_at_dispatch_leaves_the_record` | F75 | hold | 0 |  | N47S5-156 |
| `tx::tests::ur_21_s4_a_report_between_the_join_and_the_switch_s_open_cancels_it` | F75 | hold | 0 |  | N47S5-151 |
| `tx::tests::ur_21_s4_a_switch_at_the_abandoned_cursor_is_cancelled_not_applied` | F75 | hold | 0 |  | - |
| `tx::tests::ur_21_s4_a_switch_joined_then_abandoned_late_is_cancelled_not_applied` | F75 | hold | 0 |  | N47S5-154 |
| `tx::tests::ur_21_s4_a_switch_past_the_abandoned_cursor_sends_none_of_its_waveform` | F75 | hold | 0 |  | O1-M6 |
| `tx::tests::ur_21_s4_a_switch_taken_past_the_abandoned_cursor_is_cancelled_not_applied` | F75 | hold | 0 |  | - |
| `tx::tests::ur_21_s4_a_switch_whose_repeat_the_host_dispatched_late_is_refused_at_receipt` | F75 | hold | 0 |  | - |
| `tx::tests::ur_21_s4_an_underflow_ends_the_repeat_so_a_later_switch_is_refused_at_receipt` | F75 | hold | 0 |  | N47S5-136 |
| `tx::tests::ur_21_s4b_booking_exactly_at_the_deadline_continues` | F66 | hold | 0 |  | - |
| `tx::tests::ur_21_s4b_three_bookings_b_continues_a_and_c_plays_on_time` | F66 | hold | 0 |  | N47S5-25 |
| `tx::tests::ur_21_s5_b_handed_before_an_earlier_a_is_refused_at_receipt` | F35 | hold | 0 |  | N47S5-102, N47S5-105 |
| `tx::tests::ur_21_s5_c_a_booking_before_a_chain_beyond_the_horizon_is_accepted` | F34 | hold | 0 |  | N47S5-106 |
| `tx::tests::ur_21_s5_receipt_order_decides_the_overlap` | F33 | hold | 0 |  | - |
| `tx::tests::ur_21_s5_second_switch_refused_while_device_time_is_before_the_boundary` | F75 | hold | 0 |  | - |
| `tx::tests::ur_21_s6_a_late_drop_and_flag_booking_is_dropped_before_the_overlap_is_judged` | F30 | hold | 0 |  | - |
| `tx::tests::ur_21_s6_send_asap_moves_past_an_accepted_span` | F29 | hold | 0 |  | N47S5-21 |
| `tx::tests::ur_21_s6_send_asap_past_a_later_booking_is_placed_after_it` | F31 | hold | 0 |  | - |
| `tx::tests::ur_21_s7_a_handed_repeat_refuses_a_booking_inside_its_span` | F47 | hold | 0 |  | - |
| `tx::tests::ur_21_s8_switch_installed_before_the_boundary_sample` | F75 | hold | 0 |  | - |
| `tx::tests::ur_21_s9_switch_different_length_sets_the_new_period` | F75 | hold | 0 |  | E2A3-03 |
| `tx::tests::ur_21_send_does_not_cross_the_in_flight_window` | F40 | hold | 0 |  | P638 |
| `tx::tests::ur_21_stat_counts_owner_closes_past_the_record_front` | F66 | hold | 0 |  | N47S5-60 |
| `tx::tests::ur_21_tx_front_is_per_stream` | F38 | hold | 0 |  | - |
| `tx::tests::ur_21_tx_reads_in_flight_window_from_description` | F41,F42 | hold | 0 |  | P628 |
| `tx::tests::ur_22_a_failed_eob_close_recovers_with_flush_before_the_next_timed_burst` | F63 | R+N drop | 3 |  | P1172, P1191 |
| `tx::tests::ur_22_a_failed_one_sample_padded_eob_is_not_safe_until_flush_recovery` | F67 | R+N drop | 1 |  | N48S1-02, E3B-38, E3B-39 |
| `tx::tests::ur_22_a_first_send_cut_short_keeps_the_reports_paired` | F61 | hold | 0 |  | B26, B27 |
| `tx::tests::ur_22_a_first_send_is_never_undersized` | F66 | hold | 0 |  | - |
| `tx::tests::ur_22_a_flush_ack_below_the_old_burst_s_padded_end_costs_an_attempt` | F26 | R+N drop | 4 |  | N75-23 |
| `tx::tests::ur_22_a_loss_during_the_configure_clears_the_mark` | F72 | hold | 0 |  | N47S4-12b, N47S4-20b |
| `tx::tests::ur_22_a_loss_during_the_reopen_clears_the_mark_the_reopen_read` | F72 | H | 1 |  | N47S4-51 |
| `tx::tests::ur_22_a_padded_send_counts_payload_only` | F66 | hold | 0 |  | E3A-U05 |
| `tx::tests::ur_22_a_partial_ack_on_every_attempt_is_unconfirmed` | F27 | R | 5 | yes | - |
| `tx::tests::ur_22_a_provider_stop_during_the_configure_keeps_the_mark_and_cancels_the_burst` | F72 | H | 10 |  | - |
| `tx::tests::ur_22_a_reopen_keeps_the_mark_and_a_loss_clears_it` | F72 | H | 1 |  | N47S4-20 |
| `tx::tests::ur_22_a_repeat_of_one_to_five_samples_tiles_its_first_send` | F66 | hold | 0 |  | N48S1-06 |
| `tx::tests::ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` | F62,F67 | hold | 0 |  | P1168, E3B-45, B18, B19, B25 |
| `tx::tests::ur_22_a_short_first_send_is_padded_to_four` | F66 | hold | 0 |  | - |
| `tx::tests::ur_22_a_stop_during_the_configure_leaves_no_payload_after_it` | F72 | hold | 0 |  | - |
| `tx::tests::ur_22_a_two_channel_one_sample_burst_is_padded_on_both_channels` | F67 | hold | 0 |  | N48S1-24 |
| `tx::tests::ur_22_an_unconfirmed_t3_is_retried_and_recovery_ends_only_on_a_confirmed_one` | F26 | R+N drop | 63 |  | P1298, P1306, N75-15 |
| `tx::tests::ur_22_f8_recovery_that_ends_the_repeat_cancels_the_pending_switch` | F75 | H | 0 |  | N47S5-44 |
| `tx::tests::ur_22_repeat_wrap_is_not_a_new_burst_start` | F66 | hold | 0 |  | U16 |
| `tx::tests::ur_22_the_floor_of_a_flush_ack_is_the_old_burst_s_padded_end_plus_two_ticks` | F26 | R | 5 |  | N75-21, N75-22 |
| `tx::tests::ur_22_three_unconfirmed_t3s_end_the_transmit_stream` | F26 | R+N drop | 3 |  | - |
| `tx::tests::ur_23_a_31_sample_burst_waits_for_all_31` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_32_sample_window_takes_a_first_send_of_32` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_33_sample_burst_sends_32_then_holds_one` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_booking_at_a_dropped_head_s_front_is_admitted` | F52 | hold | 0 |  | - |
| `tx::tests::ur_23_a_booking_in_a_dropped_head_s_region_follows_its_late_policy` | F52 | hold | 0 |  | - |
| `tx::tests::ur_23_a_burst_at_a_closed_short_burst_s_end_is_late_at_dispatch` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_burst_at_a_held_tail_with_a_new_generation_is_configured_before_it` | F58 | hold | 0 |  | N47S4-50 |
| `tx::tests::ur_23_a_burst_at_the_end_of_a_closed_burst_is_late` | F58 | hold | 0 |  | P658 |
| `tx::tests::ur_23_a_burst_at_the_first_safe_sample_after_a_held_tail_close_is_played` | F58 | hold | 0 |  | P509, P1186 |
| `tx::tests::ur_23_a_burst_at_the_handed_frontier_continues_the_open_burst` | F67 | hold | 0 |  | R06, B04 |
| `tx::tests::ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` | F58 | hold | 0 |  | E3B-40, B08, B16, B20 |
| `tx::tests::ur_23_a_burst_of_31_or_32_is_sent_whole_without_a_held_tail` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_burst_whose_window_first_takes_28_samples_plays_clean_to_its_end` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_closed_short_burst_s_owner_front_is_gap_past_its_end` | F66 | hold | 0 |  | N49B-05, N49F-21, E3B-13 |
| `tx::tests::ur_23_a_contiguous_burst_with_the_same_generation_makes_no_configure_call` | F72 | hold | 0 |  | - |
| `tx::tests::ur_23_a_continuation_of_two_samples_is_sent_at_once_with_no_minimum` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst` | F58 | hold | 0 |  | B29 |
| `tx::tests::ur_23_a_continuation_tail_is_counted_at_recovery` | F65 | hold | 0 |  | P665, P666 |
| `tx::tests::ur_23_a_continuation_that_closes_a_burst_keeps_the_front_three_past_its_end` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned` | F58 | hold | 0 |  | B28, B31, B32 |
| `tx::tests::ur_23_a_first_send_at_its_lead_boundary_is_sent_by_drop_and_flag` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_first_send_at_its_lead_boundary_is_sent_by_send_asap_and_flag` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_first_send_one_tick_inside_its_lead_follows_drop_and_flag` | F66 | hold | 0 |  | N48S1-17 |
| `tx::tests::ur_23_a_first_send_one_tick_inside_its_lead_follows_send_asap_and_flag` | F66 | hold | 0 |  | N48S1-17 |
| `tx::tests::ur_23_a_first_send_waits_for_32_window_samples` | F66 | hold | 0 |  | E3A-U08 |
| `tx::tests::ur_23_a_four_sample_first_send_is_not_held` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_held_tail_is_timed_at_its_instant` | F58 | hold | 0 |  | - |
| `tx::tests::ur_23_a_held_tails_keep_each_channel_sample` | F67 | hold | 0 |  | P1167 |
| `tx::tests::ur_23_a_lone_burst_plays_every_sample_and_its_end_packet_is_its_last_sample` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_long_burst_in_many_chunks_plays_contiguously` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_new_sob_after_an_abandon_waits_for_32_window_samples` | F69 | hold | 0 |  | - |
| `tx::tests::ur_23_a_one_sample_burst_restarts_after_its_padded_device_burst` | F55 | hold | 0 |  | N48S1-08, E3B-41 |
| `tx::tests::ur_23_a_one_sample_continuation_eob_is_padded_to_two` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_short_burst_then_an_adjacent_burst_booked_after_dispatch_follows_the_late_policy` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_short_first_send_closes_before_a_booking_at_its_end` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_a_stop_ends_the_burst_within_the_in_flight_window` | F59 | hold | 0 |  | - |
| `tx::tests::ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it` | F58 | hold | 0 |  | B10 |
| `tx::tests::ur_23_a_value_waits_for_a_burst_moved_at_dispatch_past_its_booked_end` | F72 | hold | 0 |  | N47S4-09a |
| `tx::tests::ur_23_a_value_waits_for_a_padded_one_sample_burst` | F72 | hold | 0 |  | N47S4-09b |
| `tx::tests::ur_23_a_value_waits_for_a_stop_s_held_tail` | F58 | hold | 0 |  | N47S4-09c |
| `tx::tests::ur_23_a_wait_past_the_start_lead_follows_the_late_policy` | F66 | hold | 0 |  | N48S1-14 |
| `tx::tests::ur_23_a_wait_past_the_start_lead_moves_a_send_asap_first_send` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_abandoned_send_preserves_the_accepted_cursor` | F64 | hold | 0 |  | P657, P663, P664 |
| `tx::tests::ur_23_an_untimed_booking_after_a_dropped_head_is_placed_at_its_front` | F52 | hold | 0 |  | - |
| `tx::tests::ur_23_bursts_go_out_in_target_order` | F60 | hold | 0 |  | U18 |
| `tx::tests::ur_23_every_close_fronts_at_its_payload_end_plus_four_at_each_rate` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_loss_while_the_first_send_waits_sends_nothing` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m10_a_waiting_burst_whose_timeline_passed_is_abandoned` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m11_tx_sizing_is_independent_of_receive_block_len` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m12_the_count_is_samples_per_channel` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m1_a_repeat_stops_short_of_a_window_edge_it_cannot_fill` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m2_the_last_part_of_a_burst_is_not_split_by_the_window` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m3_a_tiled_repeat_waits_for_a_full_chunk_across_wraps` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m4_a_continuation_waits_without_a_device_call` | F66 | hold | 0 |  | P1117, E3A-U10 |
| `tx::tests::ur_23_m4b_the_buffer_before_a_held_start_is_not_split` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m4c_a_stop_during_a_continuation_s_wait_ends_the_device_burst_once` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m5_the_first_timed_buffer_takes_what_fits` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m6_a_waiting_owner_wakes_for_a_stop_within_an_idle_period` | F71 | W1 (drop in body) | 0 |  | P388, P1118 |
| `tx::tests::ur_23_m8_a_long_repeat_sends_full_chunks_as_the_window_advances` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_m8b_a_waiting_owner_sleeps_an_idle_period_between_looks` | F71 | W1 (drop in body) | 0 |  | - |
| `tx::tests::ur_23_m9_a_waiting_continuation_leaves_the_held_tail_alone` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_p1_p2_each_owner_raises_its_own_thread_once_before_its_first_send` | F70 | W1 (drop in `txp_owners`) | 0 |  | P1100, P1101, P1102 |
| `tx::tests::ur_23_p3_every_start_raises_a_new_thread` | F70 | W1 (drop in `txp_owners`) | 0 |  | - |
| `tx::tests::ur_23_p4_a_refused_priority_is_recorded_and_the_burst_still_goes_out` | F70 | W1 (drop in `txp_owners`) | 0 |  | P1103, P1104, P1105 |
| `tx::tests::ur_23_p5_p6_the_success_is_counted_per_owner` | F70 | W1 (drop in `txp_owners`) | 0 |  | P1106, P1107 |
| `tx::tests::ur_23_provider_stop_and_loss_cancel_the_pending_switch` | F75 | hold | 0 |  | - |
| `tx::tests::ur_23_s3_a_pruned_head_moved_into_a_later_booking_is_refused` | F68 | hold | 0 |  | N47S5-88 |
| `tx::tests::ur_23_s3_dispatch_move_into_a_later_booking_is_refused` | F66 | hold | 0 |  | N47S5-31, N47S5-59 |
| `tx::tests::ur_23_s3_refused_move_frees_its_front` | F66 | hold | 0 |  | N47S5-32 |
| `tx::tests::ur_23_stop_after_handover_before_the_boundary_takes_effect` | F75 | hold | 0 |  | - |
| `tx::tests::ur_23_stop_before_the_boundary_cancels_the_pending_switch` | F75 | hold | 0 |  | - |
| `tx::tests::ur_23_stop_closes_a_running_repeat_at_its_handed_cursor` | F53 | hold | 0 |  | - |
| `tx::tests::ur_23_stop_does_not_send_unhanded_window_edge_payload` | F51 | hold | 0 |  | - |
| `tx::tests::ur_23_stop_preserves_handed_cursor_after_stop` | F50 | hold | 0 |  | N47S5-24, N47S5-30 |
| `tx::tests::ur_23_stop_while_the_first_send_waits_sends_nothing` | F66 | hold | 0 |  | - |
| `tx::tests::ur_23_switch_cut_reuses_held_cut_path` | F75 | hold | 0 |  | - |
| `tx::tests::ur_23_the_abandon_front_follows_the_rate` | F66 | hold | 0 |  | N49F-26 |
| `tx::tests::ur_23_two_transmit_streams_busy_and_empty` | F74 | hold | 0 |  | - |
| `tx::tests::ur_23_txs_a_held_cut_splits_a_high_rate_repeat_at_the_target` | F67 | hold | 0 |  | E3A-U01 |
| `tx::tests::ur_23_txs_does_not_copy_repeats_at_or_above_the_send_cap` | F67 | hold | 0 |  | - |
| `tx::tests::ur_23_txs_grouped_short_repeats_keep_channel_phase_aligned` | F67 | hold | 0 |  | P1164 |
| `tx::tests::ur_23_txs_one_shots_keep_their_payload_and_end_on_the_last_sample` | F67 | hold | 0 |  | - |
| `tx::tests::ur_23_txs_stop_does_not_send_past_a_high_rate_bound` | F67 | hold | 0 |  | - |
| `tx::tests::ur_23_txs_tiles_a_short_repeat_across_original_wraps_at_high_rate` | F67 | hold | 0 |  | P1159, P1161, P1163 |
| `tx::tests::ur_25_a_burst_whose_snapshot_equals_the_device_s_makes_no_apply` | F72 | hold | 0 |  | - |
| `tx::tests::ur_25_a_slow_configure_makes_a_burst_late_by_its_policy` | F72 | hold | 0 |  | N47S4-15b |
| `tx::tests::ur_25_a_value_after_a_burst_ends_is_applied_before_the_next_burst` | F72 | hold | 0 |  | - |
| `tx::tests::ur_25_an_equal_value_after_a_new_one_configures_once` | F72 | hold | 0 |  | N47S4-11c |
| `tx::tests::ur_28_a_device_burst_over_two_kernel_bursts_keeps_both_records` | F58 | hold | 0 |  | - |
| `tx::tests::ur_28_a_value_during_an_underflow_recovery_is_applied_after_the_recovery` | F72 | H | 1 |  | N51D-03 |
| `tx::tests::c2::c2_no_row_is_left_pending_when_the_run_ends` | F75 | H | 32 |  | - |
| `tx::tests::c2::c2_s10_an_underflow_after_the_boundary_send_does_not_unsend_it` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s11_a_host_late_switch_is_pending_until_the_turn_then_cancelled` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s13_a_loss_before_the_boundary_is_decided_by_device_time` | F75 | hold | 0 |  | C2-19 |
| `tx::tests::c2::c2_s14_provider_stop_is_decided_like_a_stop` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s15_a_failed_reopen_cancels_even_a_sent_switch` | F75 | G (drop, then owner code) | 0 |  | - |
| `tx::tests::c2::c2_s15b_a_retried_recovery_settles_a_pending_switch_after_the_window_and_a_failed_one_cancels_it` | F75 | H | 9 |  | - |
| `tx::tests::c2::c2_s17b_two_accepted_switches_settle_each_on_its_own_boundary_send` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s19_a_loss_while_the_boundary_send_is_in_flight_is_decided_by_the_result_and_the_loss_sample` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s20_a_loss_after_the_boundary_send_before_device_time_reaches_b` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s21_a_stop_and_a_loss_in_flight_either_order` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s22_a_loss_after_provider_stop_in_flight_keeps_the_loss_sample` | F75 | hold | 0 |  | C2-23 |
| `tx::tests::c2::c2_s23_a_worker_that_panics_in_the_boundary_send_leaves_the_row_uncertain_at_the_close` | F75 | hold | 0 |  | C2-28 |
| `tx::tests::c2::c2_s24_a_loss_after_a_stop_below_b_overturns_a_whole_boundary_send_s_applied` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s25_a_loss_after_the_result_of_a_send_in_flight_at_a_stop_overturns_applied` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s26_an_abandon_after_a_whole_boundary_send_halts_at_device_time` | F75 | hold | 0 |  | C2-40 |
| `tx::tests::c2::c2_s3_a_stop_after_the_boundary_send_leaves_it_applied` | F75 | hold | 0 |  | N47S5-43 |
| `tx::tests::c2::c2_s4_a_stop_before_the_boundary_send_cancels_the_switch` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s5_a_stop_in_flight_is_decided_by_the_result` | F75 | hold | 0 |  | - |
| `tx::tests::c2::c2_s8_a_short_boundary_send_is_applied_when_it_took_a_sample_and_cancelled_when_it_took_none` | F75 | hold | 0 |  | - |
| `tx::tests::e1::e1_q12_a_booking_racing_a_provider_stop_cut_is_refused_at_receipt` | F75 | hold | 0 |  | E1-U5 |
| `tx::tests::e1::e1_q13_a_refused_booking_is_not_on_the_list` | F75 | hold | 0 |  | E1-U10 |
| `tx::tests::e1::e1_q14_a_continuation_accepted_after_the_unread_check_keeps_the_burst_open` | F66 | hold | 0 |  | N47S5-57 |
| `tx::tests::e1::e1_q1_a_stop_is_read_between_the_booking_before_it_and_the_one_after_it` | F75 | hold | 0 |  | E1-U7 |
| `tx::tests::e1::e1_q2_the_step_reads_a_stop_it_has_not_read_before_it_hands` | F75 | hold | 0 |  | E1-U3 |
| `tx::tests::e1::e1_q2b_now_is_read_under_the_lock_that_checked_the_list` | F75 | hold | 0 |  | E1-U4 |
| `tx::tests::e1::e1_q3_a_pre_origin_provider_stop_cancels_a_booking_with_its_own_note` | F76 | H | 10 |  | E1-U1, E1-U2 |
| `tx::tests::e1::e1_q4b_a_switch_read_after_the_stop_that_took_it_is_cancelled_with_it` | F75 | hold | 0 |  | - |
| `tx::tests::e1::e1_q5_a_booking_waiting_when_the_repeat_is_abandoned_still_plays` | F75 | hold | 0 |  | E1-U13 |
| `tx::tests::e1::e1_q6_a_stop_a_booking_and_the_run_end_all_unread` | F75 | H | 10 |  | - |
| `tx::tests::e1::e1_q6b_shutdown_reads_the_list_before_it_cancels` | F75 | H | 10 |  | E1-U11 |
| `tx::tests::e1::e1_q7_a_stop_received_while_a_configure_refuses_its_burst` | F72 | hold | 0 |  | - |
| `tx::tests::e1::e1_q8_provider_stop_closes_the_open_burst_at_the_next_turn` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::core_stat_max_keeps_the_largest` | F75 | hold | 0 |  | E2A2A-07 |
| `tx::tests::e2a::e2a_80_a_failed_send_after_a_stop_leaves_the_booking_accepted_after_it` | F75 | hold | 0 |  | E2A2B-01 |
| `tx::tests::e2a::e2a_80_an_abandon_at_the_repeat_s_own_start_keeps_today_s_answers` | F75 | hold | 0 |  | E2A2B-05 |
| `tx::tests::e2a::e2a_80_the_booking_accepted_after_the_stop_is_the_playing_repeat` | F75 | hold | 0 |  | E2A2B-03 |
| `tx::tests::e2a::e2a_84_a_second_switch_on_a_switch_the_host_has_not_issued_is_refused` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_85_provider_stop_cancels_an_unsent_open_booking_with_a_note` | F75 | H | 10 |  | E2A2B-09 |
| `tx::tests::e2a::e2a_86_a_timed_switch_pushed_past_its_boundary_records_its_at` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_88_a_switch_after_a_repeat_moved_at_dispatch_takes_a_boundary_on_the_moved_grid` | F75 | hold | 0 |  | E2A3-14 |
| `tx::tests::e2a::e2a_93_a_late_underflow_of_the_burst_before_does_not_cut_the_next_device_burst` | F75 | hold | 0 |  | E2A4-03 |
| `tx::tests::e2a::e2a_93_a_sequence_error_before_the_open_burst_s_first_tick_still_abandons_it` | F75 | hold | 0 |  | E2A4-05 |
| `tx::tests::e2a::e2a_93_an_underflow_at_the_open_burst_s_first_tick_abandons_it` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_a_send_records_the_longest_lock_wait` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_a_stop_received_between_the_plan_and_the_device_call_changes_nothing_in_the_send` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_b1_a_switch_behind_an_unstarted_finite_burst_is_refused` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_emit_under_the_streams_lock_panics` | F75 | hold | 0 |  | E2A2A-09 |
| `tx::tests::e2a::e2a_f1_a_failed_send_with_a_switch_received_in_flight_cancels_it` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_f1_an_underflow_cancels_a_switch_whose_payload_is_unread` | F75 | hold | 0 |  | E2A3-11, E2A3-12 |
| `tx::tests::e2a::e2a_f3_the_pending_twin_answers_the_same` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_f4_a_late_underflow_with_nothing_issued_sends_nothing_and_keeps_the_booking` | F66 | hold | 0 |  | E2A2B-10 |
| `tx::tests::e2a::e2a_kl39_a_finite_burst_abandoned_by_a_short_send_keeps_its_whole_region` | F75 | hold | 0 |  | E2A2B-13 |
| `tx::tests::e2a::e2a_loss_between_the_plan_and_the_device_call_completes_once_and_shutdown_notes_the_held` | F75 | H | 0 |  | - |
| `tx::tests::e2a::e2a_s11_a_stop_after_a_switch_whose_boundary_is_before_the_bound_cancels_it_and_its_payload` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::e2a_s6_a_short_send_with_a_stop_unread_closes_once_then_cancels_held` | F75 | hold | 0 |  | - |
| `tx::tests::e2a::ur_21_a_burst_time_that_does_not_fit_the_clock_is_refused_with_its_id_and_the_next_booking_is_served` | F75 | hold | 0 |  | N95-10, N95-11 |
| `tx::tests::e2a::ur_26_a_stop_whose_bound_does_not_fit_the_clock_is_refused_with_its_id_and_changes_nothing` | F75 | hold | 0 |  | N95-19, N95-20 |
| `tx::tests::e2a::ur_38_a_switch_at_the_end_of_the_range_is_refused_with_its_id_and_the_next_one_is_taken` | F75 | hold | 0 |  | N95-12 |
| `tx::tests::e2a::ur_38_a_switch_whose_boundary_does_not_fit_a_clock_of_one_tick_a_sample_is_refused_by_the_book` | F75 | hold | 0 |  | N95-14 |
| `tx::tests::e2b::e2b_c0_a_configure_records_its_applied_rows_before_the_burst_s_first_send` | F72 | hold | 0 |  | - |
| `tx::tests::e2b::e2b_c1_a_configure_waits_for_the_stream_claim` | F72 | hold | 0 |  | E2B-10 |
| `tx::tests::e2b::e2b_c2_a_full_stop_during_the_configure_settles_x_and_keeps_y` | F77 | hold | 0 |  | - |
| `tx::tests::e2b::e2b_c3_a_stop_after_the_configure_result_cancels_the_burst_at_the_next_turn` | F77 | hold | 0 |  | - |
| `tx::tests::e2b::e2b_c4_a_loss_around_the_configure` | F72 | H | 0 |  | E2B-13 |
| `tx::tests::e2b::e2b_c5_a_switch_during_its_repeat_s_configure_is_refused_until_the_repeat_sends` | F77 | hold | 0 |  | - |
| `tx::tests::e2b::e2b_c6_provider_stop_during_a_refused_configure_reports_the_refusal_once` | F72 | H | 10 |  | - |
| `tx::tests::e2b::e2b_c7_a_stop_during_a_reopen_is_read_after_its_result` | F72 | R | 1 |  | - |
| `tx::tests::e2b::e2b_c8_a_receipt_between_a_refused_configure_s_mark_and_its_drop_is_refused` | F77 | hold | 0 |  | E2B-16 |
| `tx::tests::e2b::e2b_c9a_a_loss_just_before_the_reopen_s_result_section_leaves_the_mark_cleared` | F72 | R | 1 |  | E2B-11 |
| `tx::tests::e2b::e2b_c9b_a_loss_just_before_the_configure_s_mark_section_leaves_the_mark_cleared` | F72 | H | 0 |  | E2B-14 |
| `tx::tests::e2b::e2b_c9c_a_loss_just_before_a_failed_reopen_s_result_section_ends_no_clock` | F72 | R | 0 |  | E2B-19 |
| `tx::tests::e2b::e2b_r2b_the_line_lock_is_free_inside_the_reopen` | F78 | R | 1 | yes | - |
| `tx::tests::e2b::n98_a_loss_in_the_first_bookings_configure_gives_both_held_bookings_the_note` | F72 | H | 0 |  | - |
| `tx::tests::e2b::n98_a_loss_landing_during_a_refusal_leaves_the_refusal` | F72 | H | 0 |  | N98-01 |
| `tx::tests::e2b::n98_a_lost_configure_keeps_its_booking_as_a_held_one_is_kept` | F77 | H | 0 |  | N98-04, N98-05 |
| `tx::tests::e2b::n98_a_provider_stop_before_a_lost_configure_completes_cancels_the_booking_once` | F72 | H | 0 |  | - |
| `tx::tests::e3::e3_c1_a_lone_finite_burst_by_length` | F66 | hold | 0 |  | - |
| `tx::tests::e3::e3_c2_the_first_send_window_edge` | F66 | hold | 0 |  | - |
| `tx::tests::e3::e3_c3_joined_pieces` | F66 | hold | 0 |  | - |
| `tx::tests::e3::e3_c4_the_cut_at_a_held_start` | F66 | hold | 0 |  | - |
| `tx::tests::e3::e3_c5_the_hold_deadline` | F66 | hold | 0 |  | - |
| `tx::tests::e3::e3_c6_the_send_cap_by_window_and_rate` | F66 | hold | 0 |  | - |
| `tx::tests::e3b::e3b_b8_a_new_owner_starts_with_a_closed_chain` | F66 | hold | 0 |  | E3B-15 |
| `tx::tests::e3b::e3b_c1_the_front_after_each_close` | F66,F67 | R | 1 |  | E3B-42 |
| `tx::tests::e3b::e3b_c2_may_be_handed_is_the_window_and_not_the_owner` | F66 | hold | 0 |  | - |
| `tx::tests::e3b::e3b_c3_the_chain_through_its_lifecycles` | F66,F67 | R | 18 |  | - |
| `tx::tests::e3b::e3b_c3b_the_seams_between_the_sections` | F66,F67 | R | 1 |  | - |
| `tx::tests::e3b::e3b_c3c_a_loss_found_while_the_open_burst_ends` | F66 | hold | 0 |  | E3B-37 |
| `tx::tests::e3b::e3b_c4_the_streams_lock_sections` | F66,F67 | R | 1 |  | - |
| `tx::tests::e3b::e3b_c4c_the_stat_reads_the_front_the_close_leaves` | F66 | hold | 0 |  | E3B-43, E3B-44 |
| `tx::tests::e3b::e3b_c5_receipts_by_provider_uhd_column` | F75 | hold | 0 |  | - |
| `tx::tests::e3b::e3b_c6_f1_as_it_is` | F66 | hold | 0 |  | - |
| `tx::tests::n75::ur_22_stop_and_loss_at_the_retry_boundary_release_the_claim` | F26 | R | 2 |  | N75-26, N75-27 |
| `tx::tests::n75::ur_75_a_loss_or_a_stop_while_the_time_reads_fail_is_not_a_failed_recovery` | F26 | R | 505 |  | - |
| `tx::tests::n75::ur_75_a_report_an_unconfirmed_attempt_stashed_makes_its_event_after_the_recovery` | F26 | R | 9 |  | N75-11 |
| `tx::tests::n75::ur_75_a_retry_longer_window_leaves_held_bookings_to_their_late_policy` | F26 | R+N drop | 12 |  | - |
| `tx::tests::n75::ur_75_a_stop_with_a_pending_recovery_then_three_unconfirmed_t3s` | F79 | R+N drop | 6 | yes | - |
| `tx::tests::n75::ur_75_an_ordinary_tx_stop_during_the_recovery` | F75 | H | 5 |  | - |
| `tx::tests::n75::ur_75_the_owner_s_recovery_waits_out_failing_time_reads_and_ends_failed_only_at_the_cap` | F26 | R+N drop | 2453 |  | - |
| `tx::tests::n75::ur_75_the_record_across_a_retried_recovery_is_the_one_attempt_record` | F75 | H | 12 |  | - |
| `tx::tests::n99::n99_u10_a_receipt_at_the_seam_is_decided_by_the_record_it_finds` | F75 | hold | 0 |  | N99-15 |
| `tx::tests::n99::n99_u11_what_the_abandon_leaves_to_a_later_booking_and_a_later_stop` | F75 | hold | 0 |  | N99-15 |
| `tx::tests::n99::n99_u1_a_booking_at_the_end_of_an_abandoned_burst_is_refused` | F75 | hold | 0 |  | N99-15 |
| `tx::tests::n99::n99_u2_every_way_a_finite_burst_is_abandoned_ends_its_chain` | F75 | hold | 0 |  | - |
| `tx::tests::n99::n99_u3_a_piece_accepted_before_the_abandon_is_a_chain_of_its_own` | F75 | hold | 0 |  | - |
| `tx::tests::n99::n99_u4_the_abandoned_middle_piece_leaves_the_pieces_before_and_behind_it` | F75 | hold | 0 |  | - |
| `tx::tests::n99::n99_u5_a_tail_that_comes_up_short_abandons_the_piece_that_waits_for_it` | F75 | hold | 0 |  | - |
| `tx::tests::n99::n99_u6_the_waiting_device_burst_that_expires_re_frames_the_piece_held_behind_it` | F75 | hold | 0 |  | N99-16 |
| `tx::tests::n99::n99_u7_a_stop_or_a_loss_that_comes_during_the_send_is_not_undone_by_its_abandon` | F75 | hold | 0 |  | N99-17 |
| `tx::tests::n99::n99_u8_a_halt_the_owner_has_not_read_is_not_undone_by_a_report_driven_abandon` | F75 | hold | 0 |  | - |
| `tx::tests::n99::n99_u9_a_receipt_between_the_sends_and_the_abandon_does_not_hide_the_abandoned_chain` | F75 | hold | 0 |  | - |
| `tx::tests::o1::e3_u1_the_stop_by_burst_shape` | F75 | hold | 0 |  | - |
| `tx::tests::o1::e3_u1b_a_booking_joined_to_a_handed_one_at_a_stop` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t1_short_and_long_heads_front_table` | F75 | hold | 0 |  | O1-M1 |
| `tx::tests::o1::o1_uhd_t2_a_join_is_inclusive_at_its_deadline` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t2c_a_booking_joined_at_a_heads_end_with_a_later_one_booked_early_plays_on_time` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t3_a_repeat_keeps_its_phase_across_wraps_and_a_stop_cuts_it_in_its_range` | F75 | hold | 0 |  | E3A-U03 |
| `tx::tests::o1::o1_uhd_t3b_a_stop_cannot_find_payload_past_its_bound` | F75 | hold | 0 |  | O1-M5 |
| `tx::tests::o1::o1_uhd_t4a_a_switch_takes_the_first_grid_point_at_or_after_its_bound` | F75 | hold | 0 |  | O1-M3a, O1-M8, E2A3-17 |
| `tx::tests::o1::o1_uhd_t4b_an_owner_that_has_not_read_a_switch_never_hands_past_its_boundary` | F75 | hold | 0 |  | E2A3-18 |
| `tx::tests::o1::o1_uhd_t4c_a_stop_before_the_boundary_cancels_the_switch` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t4d_a_stop_with_b_before_its_bound_cancels_a_switch_whose_first_sample_was_not_handed` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t4e_a_stop_after_the_new_waveform_sounded_leaves_it_applied` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t5_a_late_send_asap_and_an_untimed_booking_are_placed_exactly` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t6a_a_cancellation_that_starts_at_or_after_the_bound_leaves_no_region` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t6b2_an_unhanded_open_booking_at_a_stop_is_cancelled` | F75 | hold | 0 |  | E2A2B-08 |
| `tx::tests::o1::o1_uhd_t6b_a_cancellation_that_starts_before_the_bound_leaves_the_region` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t6c_a_booking_after_a_stop_on_a_playing_burst_survives` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t6d_a_booking_after_a_stop_survives_the_failing_close_of_the_burst_it_cut` | F75 | R+N drop | 1 |  | - |
| `tx::tests::o1::o1_uhd_t7a2_the_region_of_an_abandoned_repeat_is_its_cursor_plus_five` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t7a3_a_failed_send_that_ends_at_the_boundary_still_cancels_the_switch` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t7a_a_failed_send_ends_a_repeat_at_its_attempted_end_and_cancels_the_switch` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t7b_an_abandoned_finite_booking_keeps_its_whole_region` | F75 | hold | 0 |  | - |
| `tx::tests::o1::o1_uhd_t7c_a_failed_send_that_carries_the_boundary_leaves_the_switch_uncertain` | F75 | hold | 0 |  | C2-14 |
| `tx::tests::o1::o1_uhd_t8_a_switch_whose_repeat_the_host_has_not_issued_is_refused_at_receipt` | F75 | hold | 0 |  | E2A3-01 |
| `tx::tests::o1::ur_23_a_booking_at_the_records_abandon_front_waits_for_the_owner_front` | F75 | hold | 0 |  | - |

## Appendix B: rows to run

**4a (the 248 below; `N51D-01` is replaced and `N51D-02` retired as §6.1 says; `N51D-03` is in 4c's list):**
B04 B08 B10 B16 B18 B19 B20 B25 B26 B27 B28 B29 B31 B32 C2-14 C2-19 C2-23 C2-28 C2-40 E1-U1 E1-U10 E1-U11 E1-U13 E1-U2 E1-U3 E1-U4 E1-U5 E1-U7 E2A2A-07 E2A2A-09 E2A2B-01 E2A2B-03 E2A2B-05 E2A2B-08 E2A2B-09 E2A2B-10 E2A2B-13 E2A3-01 E2A3-03 E2A3-11 E2A3-12 E2A3-14 E2A3-17 E2A3-18 E2A4-03 E2A4-05 E2B-10 E2B-11 E2B-13 E2B-14 E2B-16 E2B-19 E3A-U01 E3A-U03 E3A-U04 E3A-U05 E3A-U08 E3A-U10 E3B-13 E3B-15 E3B-37 E3B-38 E3B-39 E3B-40 E3B-41 E3B-42 E3B-43 E3B-44 E3B-45 J53 N0-1 N47S3-12 N47S3-13 N47S3-14 N47S3-15 N47S3-16 N47S3-21 N47S3-36 N47S3-38 N47S4-09a N47S4-09b N47S4-09c N47S4-11c N47S4-12 N47S4-12b N47S4-13b N47S4-14 N47S4-15b N47S4-16 N47S4-20 N47S4-20b N47S4-50 N47S4-51 N47S5-102 N47S5-105 N47S5-106 N47S5-123 N47S5-124 N47S5-125 N47S5-129 N47S5-130 N47S5-136 N47S5-139 N47S5-151 N47S5-154 N47S5-156 N47S5-19 N47S5-21 N47S5-24 N47S5-25 N47S5-30 N47S5-31 N47S5-32 N47S5-43 N47S5-44 N47S5-45 N47S5-53 N47S5-56 N47S5-57 N47S5-58 N47S5-59 N47S5-60 N47S5-61 N47S5-88 N47S5-89 N47S5-92 N48S1-02 N48S1-06 N48S1-08 N48S1-14 N48S1-17 N48S1-24 N49B-05 N49F-21 N49F-26 N49F-27 N51D-03 N67F-01 N67F-02 N67F-03 N67F-05 N67F-10 N67F-11 N67F-21 N67F-22 N67F-23 N75-01 N75-02 N75-03 N75-04 N75-05a N75-05c N75-06 N75-07 N75-08a N75-08b N75-08c N75-09 N75-10 N75-11 N75-13 N75-14 N75-15 N75-16 N75-17 N75-18 N75-19 N75-20 N75-21 N75-22 N75-23 N75-26 N75-27 N75-28 N75-29 N75-30 N75-31 N75-32 N75-33 N75-34 N92-7c N95-10 N95-11 N95-12 N95-14 N95-19 N95-20 N98-01 N98-03 N98-04 N98-05 N99-15 N99-16 N99-17 O1-M1 O1-M3a O1-M5 O1-M6 O1-M8 P1100 P1101 P1102 P1103 P1104 P1105 P1106 P1107 P1117 P1118 P1159 P1160 P1161 P1163 P1164 P1167 P1168 P1172 P1186 P1191 P1238 P1242 P1256 P1290 P1291 P1292 P1295 P1298 P1306 P1392 P1396 P1397 P388 P509 P628 P638 P640 P641 P642 P657 P658 P663 P664 P665 P666 P667 R06 U16 U18

**4b (56):**
J53 N47S3-12 N47S3-13 N47S3-14 N47S3-15 N47S3-16 N47S3-21 N47S3-36 N47S3-38 N67F-01 N67F-02 N67F-03 N67F-05 N67F-10 N67F-11 N67F-21 N67F-22 N67F-23 N75-01 N75-02 N75-03 N75-04 N75-05a N75-05c N75-06 N75-07 N75-08a N75-08b N75-08c N75-09 N75-10 N75-13 N75-14 N75-16 N75-17 N75-18 N75-19 N75-20 N75-28 N75-29 N75-30 N75-31 N75-32 N75-33 N75-34 N92-7c P1238 P1242 P1256 P1290 P1291 P1292 P1295 P1392 P1396 P1397

**4c (192):**
B04 B08 B10 B16 B18 B19 B20 B25 B26 B27 B28 B29 B31 B32 C2-14 C2-19 C2-23 C2-28 C2-40 E1-U1 E1-U10 E1-U11 E1-U13 E1-U2 E1-U3 E1-U4 E1-U5 E1-U7 E2A2A-07 E2A2A-09 E2A2B-01 E2A2B-03 E2A2B-05 E2A2B-08 E2A2B-09 E2A2B-10 E2A2B-13 E2A3-01 E2A3-03 E2A3-11 E2A3-12 E2A3-14 E2A3-17 E2A3-18 E2A4-03 E2A4-05 E2B-10 E2B-11 E2B-13 E2B-14 E2B-16 E2B-19 E3A-U01 E3A-U03 E3A-U04 E3A-U05 E3A-U08 E3A-U10 E3B-13 E3B-15 E3B-37 E3B-38 E3B-39 E3B-40 E3B-41 E3B-42 E3B-43 E3B-44 E3B-45 N0-1 N47S4-09a N47S4-09b N47S4-09c N47S4-11c N47S4-12 N47S4-12b N47S4-13b N47S4-14 N47S4-15b N47S4-16 N47S4-20 N47S4-20b N47S4-50 N47S4-51 N47S5-102 N47S5-105 N47S5-106 N47S5-123 N47S5-124 N47S5-125 N47S5-129 N47S5-130 N47S5-136 N47S5-139 N47S5-151 N47S5-154 N47S5-156 N47S5-19 N47S5-21 N47S5-24 N47S5-25 N47S5-30 N47S5-31 N47S5-32 N47S5-43 N47S5-44 N47S5-45 N47S5-53 N47S5-56 N47S5-57 N47S5-58 N47S5-59 N47S5-60 N47S5-61 N47S5-88 N47S5-89 N47S5-92 N48S1-02 N48S1-06 N48S1-08 N48S1-14 N48S1-17 N48S1-24 N49B-05 N49F-21 N49F-26 N49F-27 N51D-03 N75-11 N75-15 N75-21 N75-22 N75-23 N75-26 N75-27 N95-10 N95-11 N95-12 N95-14 N95-19 N95-20 N98-01 N98-03 N98-04 N98-05 N99-15 N99-16 N99-17 O1-M1 O1-M3a O1-M5 O1-M6 O1-M8 P1100 P1101 P1102 P1103 P1104 P1105 P1106 P1107 P1117 P1118 P1159 P1160 P1161 P1163 P1164 P1167 P1168 P1172 P1186 P1191 P1298 P1306 P388 P509 P628 P638 P640 P641 P642 P657 P658 P663 P664 P665 P666 P667 R06 U16 U18
