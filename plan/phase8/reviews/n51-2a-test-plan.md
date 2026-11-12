# Note 51 stage 2a — test plan (no code in the tree)

## Changes after Sol's review (sol-plan.md: FAIL, 3 majors, 3 minors; resolutions in plan-decisions.md) — 2026-11-12

Where this section and a later one differ from the sections below, this section wins. Sections 0–12 are the plan as Sol reviewed it; only the points named here changed. Evidence marks as above (V = run, I = inferred).

### M1 — join is a bound everywhere; the exemption `now >= t0` is dropped (affects §0 rec. 5, §1.3, §7, §8 rows 01/02, §10 item 1)

- `judge_end` has no `started` input. A Provider join (`did not join within`, from the `rejected` notes) is a bound for every Run, before or after T0. Two declarations may exempt it, and nothing else: `join_is_subject` (the 3 tests whose subject is the join) and **`rx_join_issue_67`**, which exempts the join of a `uhd-rx-*` thread only (notes carry the thread name, `provider/mod.rs:405`) and says so in one comment citing #67. It is removed, with its declarations, when #67 is fixed. It exempts no other bound (RS-8a, KC-12a, KC-21a), no `uhd-tx-*` or `uhd-control` join, and declares no end. Its API on the wrapper: `.rx_join_issue_67()`.
- The tests that hit #67 are the five healthy tests that the unconditional-join prototype failed (V: `probe/failed-with-wrapper.txt`, the whole `fake` binary, 22 failures; the marker line is `probe/fake-suite-with-wrapper.log:238,315`). Each is a Spec Run on the real clock that is `Running` when it ends: the receive segment is begun ahead of its origin and the test finishes it before T0. Whether each of the five records a `uhd-rx` thread is **I** (the marker names no thread; V only for a plain Session in `plan.log:5197-5213`): the wrapper's marker now prints the `rejected` notes with their threads, so sub-step 2's first run shows it, and a test whose note names another thread is re-examined, not declared.

| test (`fake.rs`) | why it ends before T0 | handling in sub-step 2 |
|---|---|---|
| `ur_13_arm_cases` (`:1681`, second Run) | the subject is the refusal at arm; the last lines only check `Running` and `finish()` | `.rx_join_issue_67()`. Advancing past T0 means waiting out the 1 s start lead of a real-clock Spec Run and adds a wall-clock dependence the test never had (note 51 §3) |
| `ur_13_arm_compares_the_rate_set_after_the_streamer_opens` (`:4868`) | subject: the order of `rates` and `*_open` calls, read from the call log at `Running` | `.rx_join_issue_67()` |
| `ur_13_arm_takes_the_rate_set_in_any_order` (`:4949`) | subject: a reversed rate set is accepted at arm | `.rx_join_issue_67()` |
| `ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid` (`:4964`, first Run) | subject: the whole grid is accepted at arm; the second Run is a refusal (`Expect::Refused`) | `.rx_join_issue_67()` on the first Run |
| `the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses` (`:6782`) | subject: admission; `admitted` is dropped at scope end | `.rx_join_issue_67()`, and the end made explicit with `admitted.finish()` (the wrapper's `Drop` would judge it too) |

  Every `#[test]` that an unconditional join fails is in this table or in the three-test `join_is_subject` class of §2; a test that fails on a join only after sub-step 2 (the acceptance `uhd.rs` and `hardware.rs` were not run with the prototype) is handled the same way, one by one, in the step's record — never by exempting the join globally. On the bench (`hardware.rs`) a test that finishes a receive Run before its origin would show the same marker; **cannot tell** which, and the audit table row for "any bench Run that reaches … a post-T0 join detach" now reads "…a join detach, before or after T0".
- Rows (§8): 02 (join before T0 counted) is replaced; see "Changes in sub-step 1" for the new row list.

### M2 — a method-return test is not satisfied by `Drop` (affects §7)

Each of the four method tests (`submit`, `advance_to`, `wait_for`, `run_until_end`) keeps the Run **outside** the closure and catches the panic of the one call: `let mut run = …declared…; let caught = catch_unwind(AssertUnwindSafe(|| run.advance_to(t)));` and asserts `caught.is_err()` with the panic message `ended unexpectedly`, then that the call's marker count is as declared (the undeclared Policy end writes no marker; the bound variant — a `Gated` Run past T0 whose cleanup join times out, one per method that can reach it — asserts exactly one `EZSDR-INFRA` line, counted by a `thread_local` that `judge` increments next to `infra()`: a small test-only hook that sub-step 2 adds to `run.rs`, because the marker goes straight to stderr and cannot be captured). Only then may the test drop the Run: the wrapper keeps the Manifest before it judges (§1.4 stored first), so `run.finish()` after the catch returns it without a second panic, and a later `Drop` has nothing to judge. A `#[should_panic]` on the whole test is **not** used for these four. Rows 06–09 (`check` removed from one method) die on `assert!(caught.is_err())`. These tests need the switch, so they stay in sub-step 2.

### M3 — a bound cut in the background (affects §1.2 table row "read-only forwards", §1.5, §7, §10 item 5)

Scenario added (**V by code reading**, not run; the interleaving is I): a Policy ends the Run, `stop_early` runs `StopProviders` on the data thread (`paced.rs:133-148`) and the Provider's join times out; the test calls only `events()`/`state()` and then an assertion fails before any settling call.

Observation boundary, chosen with the evidence:
1. **A read-only method cannot see a new end** (V: `CleanedUp` is entered only by `settle()`, `ending.rs:15-47`, whose callers are `connect` `mod.rs:160,177`, `disconnect`/`check_lease` `:256,:288`, `submit`/`run_child` `pipeline.rs:1048,1094`, and advance/`wait_for`/`run_until_end` `stepping.rs:408,423,458`, plus `finish`; the data thread's `stop_early` only requests the end and records cleanup failures, it does not settle). So `state()`/`events()`/`now()` can add no information a checked call has not already judged, and a read-only check would be dead code. The Manifest of a Run that has not settled cannot be had without `finish`, which consumes the handle and settles.
2. **What is guaranteed, said plainly** (to go into note 51 §6.2 and the tests README in sub-step 8): every call that settles (`connect`, `start_spec`, `submit`, `advance_to`, `wait_for`, `run_until_end`), `finish`, and `Drop` check; a read-only return checks nothing; a bound cut in the background shows at the next of those.
3. **The unwinding case is closed by `Drop`, not by narrowing**: the test's assertion fails after `events()`, the thread unwinds, and `Drop` finishes the live Run — what the Kernel's own `Drop` already does for a device-paced Run (`mod.rs:315-320`) — judges it, and writes the marker **without panicking** (`judge` panics only when `!thread::panicking()`). This changes note 51 §6.2's "unwind の規則" (`Drop` neither panics nor calls `finish()` while panicking): now it calls `finish()`, writes a marker for a bound, and never panics. A `Drop` that finds a Run already ended (a checked call took its Manifest) does nothing. Residual, accepted: a bound that cuts inside that unwinding cleanup only after an assertion has already failed is marked too (the Manifest is complete by then); a `finish()` that does not return is the watchdog's. Test (sub-step 2): `run_unwinding_marks_a_bound_and_never_panics_again` — a wedged Run (J317 form) alive while the test panics with "boom": the panic message is "boom" (`should_panic(expected = "boom")` is fine here: the point is that the process does not abort and the message is not masked), and the marker count is 1 (the `thread_local` counter, read by a `Drop` guard declared before the Run). Row 12 (`Drop` judges while panicking → second panic) now kills on the abort-free "boom" message; a new row removes the marker while unwinding.
4. The wrapper's read-only forwards therefore stay plain forwards, as in §1.2.

### Minors

- **Scan** (§5): it forbids a raw `coordinator::connect` / `start_spec_run` call or a `use … connect as …` / `start_spec_run as …` import (a renamed import is tracked: the scan collects the alias after `as` and forbids it too) in any file but `run.rs`, and allows `Run::connect(` / `Run::start_spec(`; `disconnect(` and `RunHandleError` stay allowed. Self-tests (strings): the allowed `Run::connect(`; `use ezsdr_kernel::coordinator::connect as open; open(…)`; `connect (` and `connect\n(` with whitespace/line breaks; `coordinator::\nstart_spec_run(`. Cells are added to rows 13–18's killers; sub-step 6.
- **Fixture setup** (§6): no `wait(700 ms)`. Each fixture's `Stalls` device sets an `entered: AtomicBool` (or a channel) when `ref_locked`/`close_streams` is entered; the test waits for it with an infrastructure-marked setup bound (`bounded(5 s)` that writes `EZSDR-INFRA: fixture setup …` and panics) — an observed-entry handshake. **Fixture 8 (join at `finish`) is kept** as part of sub-step 5.
- **`protocol.rs`** (§4): the injection is **after** `server.handle` returns, consistently, and `rig_delay` is called on the returned value's path; the test in sub-step 4: with `EZSDR_RIG_DELAY_MS` set in the process's own `rig_delay_for(…)` parser test (the parser takes the two strings as arguments, so no environment is touched), the point `server` selects the call-return injection and `now`/`advance` do not.

### `Expect::Any` (§1.3, §2, §3 audit table)

`Expect::Any` is used only by individually named observation-only bench loops: `hw_perf_rx_rates` and the B8 lead loops (the `let _ = run.submit/advance_to` ones, named one by one at sub-step 2 from `grep -n 'let _ = run\.' hardware.rs`), never in a shared helper, and a shared trial gets no `Any` parameter. It rejects every bound (`judge_end` names the bound before the end: `run_judge_end_any_is_not_a_bound_pass`, row 05) and the loop still **reports** the termination it saw. FakeDevice checks (sub-step 2, with the loops' bodies factored enough to run on the Fake — their expectation is "a non-deadline end is reported and the loop continues; a bound aborts"): the rehearsal of the loop's body on a Fake Run with `FakeFault::Lost` continues past `Err(Ended)` and prints the termination; the same body on a Run whose join times out panics with the marker. B9 (`hw_b9_unplug*`) keeps its named `Policy("ezsdr.DEVICE_LOST")` expectation, not `Any`.

### §11 sub-steps after the changes

Sub-step 1 now contains the whole wrapper in `run.rs` (the types, `judge_end`, the methods, `check`, `judge`, `Drop`), compiled and dead until sub-step 2 switches callers, plus the **pure** `judge_end` tests and their rows. The method tests with `catch_unwind` (M2), the Ended-mode, `Drop` and unwinding tests (M3) and rows for them stay in **sub-step 2** because they need `Run::connect` on a Fake Session through the switched helpers. Sub-step 2 is atomic as before and additionally carries the five `rx_join_issue_67` declarations, the `Expect::Any` declarations and the Any rehearsals; sub-step 4's `protocol.rs` injection is after the return; sub-step 5's fixtures use the handshake and add fixture 8; sub-step 6's scan has the alias/whitespace cells. The sub-step list stays at eight.


Branch `work/n51-2a` from `main` `42f5031`. Author: the implementer (OpenCode Sonnet 5.5 #high). For Sol's test-plan review (AGENTS §9 step 3). Scope: the tests of `ezsdr-radio-uhd` (`tests/common/**`, `fake.rs`, `hardware.rs`, `infra_fixtures`), `ezsdr-acceptance/tests/uhd.rs` and `governance.rs`, `ezsdr-server/tests/protocol.rs`, `plan/maintenance/tools/mutate.py`/`mutations.json`, and the text 2a makes false. **No production code and no Kernel change.** The main tree is untouched (`git status` clean).

**Evidence.** V = run on 2026-11-12 in throw-away copies outside the repository, built **locally** into `~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0` (no sim02/sim03, no `mutate.py`). `/tmp/opencode/n51-2a-base` = `main` + probes 4/4b/6/7 as they fail today (no wrapper); `/tmp/opencode/n51-2a-probe` = `main` + a prototype of the wrapper (`~/.cache/ezsdr-fixes/2026-11-12-n51-2a/probe/run.rs`, whole diff `wrapper-vs-main.diff`, suite log `fake-suite-with-wrapper.log`). I = inferred, not run. Hazard met (AGENTS §7): both copies build `fake-1500dc241d1ae98c` in the shared review directory; I touched every file before each build and checked the binary contains the copy's own probe string; I never built the base while the probe's suite ran.

## 0. Recommendations (one line each)

1. **One wrapper `Run`** in a new file `tests/common/run.rs`, shared with the acceptance crate by `#[path]` (the way `infra.rs`, `watchdog.rs`, `rig.rs` already are). Raw `RunHandle` is not visible outside that file. Eleven methods, no `Deref`.
2. **Eager check at every return** (note 51 §6.2): when a method returns and `state()` is `CleanedUp`, the wrapper takes the Manifest itself (`finish()`), judges it, and keeps it. A Run in that state answers from the stored Manifest (§1.4). I prototyped it: over the 432 `fake` tests **no test reads an accessor after its Run ended** (instrumented, 0 hits), so the post-end representation costs ~25 lines and no behaviour.
3. **Two declarations**, no more: `Declared { end: Option<Expect>, join_is_subject: bool }`. `Expect` = `Refused` | `Policy(event kind)` | `Any`. Only `join` is ever the subject of a test in the UHD Run tests (3 tests), so the other limits get no enum variant until a test needs one.
4. **A test that forgets `finish()` is caught by `Drop`, not by a scan or `#[must_use]`**: a live Run dropped while the thread is not panicking is finished and judged exactly like `finish()` (1 test does this today). Rejected: `#[must_use]` (catches only a discarded value, not a binding that goes out of scope); a scan for `.finish(` (cannot see helpers that move the Run; false positives on Runs the test ends by `captured`/`finish_spec`).
5. **`join` is a limit only for a Run that reached its T0.** A healthy Run finished before T0 records `did not join within 1 s; left detached` for `uhd-rx-s0` today (V, §6.3: 1.1 s `finish`; after T0: 114 ms and no row). Judging join everywhere failed 5 healthy tests in the prototype. This contradicts note 51 §6.2 item 2 and tables §4.2 U1 (§10). The cause may be a product defect (the receive owner ignores `Shutdown` while it waits for its start); not mine to fix, flagged for the orchestrator.
6. **The scan is a Rust test in `ezsdr-acceptance/tests/governance.rs`** (beside `po_11`, `gz_08`), with a pure scanning function and a string self-test. Rejected: `rules.py` (specs, not Rust), clippy `disallowed_*` (`.host()`/`.time_now()` are legitimate in rigs).
7. **Fixtures 4, 4b, 6, 7 are built and work** on one test-local wrapper device `Stalls` (§6). Both of note 51's "INFERRED" device forms were wrong in detail; the forms that work are in §6.
8. Sub-steps: 8, each green (§11).

## 1. The wrapper

### 1.1 Types and where they live

`crates/ezsdr-radio-uhd/tests/common/run.rs` (new, ~230 lines): `pub struct Run`, `pub enum Expect`, `pub struct Declared`. `common/mod.rs` declares `pub mod run; pub use run::{Declared, Expect, Run};`. It needs `infra()` and `creation_limit()`, which `common` re-exports; `run.rs` imports them with `use super::{creation_limit, infra};` — the same trick as `rig.rs` (`use super::infra`), so the including test file only has to have those two names at its root.

**Acceptance crate.** `ezsdr-acceptance/tests/uhd.rs` adds `#[path = "../../ezsdr-radio-uhd/tests/common/run.rs"] mod run;` and `use watchdog::infra::{creation_limit, infra};`. V: the prototype compiles and the 19 tests of `uhd` pass (16 + the shared files' own tests). Rejected alternative: a new test-support crate (would also have to move `watchdog.rs`/`infra.rs`/`rig.rs`, which already share by path; crate count is not a goal, §4). Rejected: a second copy of the rule in the acceptance crate (a rule implemented twice; AGENTS §9 maintainability 1).

`parity.rs` (`UhdRig`) does not create Runs; it is unchanged by 2a.

### 1.2 Methods (only those used today)

Search: in a copy, every test file's `RunHandle` replaced by an empty newtype `Run` and `cargo check` — the compiler lists every method a test calls (V; `probe/method-uses.txt`, plus `hardware.rs` with its `#![cfg(feature = "uhd")]` removed in the copy: its only other errors are the three feature-only symbols `UhdDevice`, `uhd_kept_count` (×2), which do not touch the Run). Uses: `submit` 61 (+70 in `hardware.rs`), `finish` 96 (+40), `state` 25 (+12), `now` 17 (+9), `advance_to` 16 (+9), `start_instant` 16 (+6), `sample_clocks` 14 (+13), `run_until_end` 14, `wait_for` 13 (+12), `events` 5 (+3), `effective` 1 (`uhd.rs`). Not used anywhere: `id`, `kind`, `run_child`, `disconnect`, `disconnect_signal`, `check_lease` — not provided.

| method | forwards | after the forward |
|---|---|---|
| `Run::connect(profile, initial, assembly, lease, Declared)` / `Run::start_spec(spec, profile, assembly, Declared)` | the Kernel's `connect` / `start_spec_run`; the `SpecError` is returned unchanged | `check` (creation) |
| `submit`, `advance_to`, `wait_for`, `run_until_end` | the same-named `RunHandle` method; the result is returned unchanged | `check` |
| `state`, `now`, `start_instant`, `sample_clocks`, `effective`, `events` | read-only forwards | none (they cannot end a Run: the state moves to `CleanedUp` only inside a call that settles — `settle()` is reached from `connect`/`submit`/advance, `ending.rs:15-47`, `stepping.rs`) |
| `finish(self) -> Manifest` | `RunHandle::finish` | `judge` |
| `expecting(self, Expect) -> Run`, `subject_join(self) -> Run` | none | set the declaration |

### 1.3 What `check` does (note 51 §6.2 steps 1–3)

`check` runs when a method returned: if the inner handle is `Live` and `state()` is not `CleanedUp`, return. Else: read `now`, `start_instant`, `sample_clocks`, `effective` (they cannot be read after `finish`), call `finish()` (the ordinary path; never while unwinding), store `Ended`, then `judge(&manifest)`:

1. `creation_limit(reason, cleanup timed_out, rejected notes)` (already in `infra.rs`; the prefixes `KC-12a:`/`KC-21a:`, a `timed_out` cleanup row (RS-8a), `did not join within`) names a limit → `infra(msg)` (one `write_all`) and **one** panic. Join counts only if `started` (T0 reached: `start_instant` is `Some(t0)` and `now >= t0`); `join_is_subject` exempts join and nothing else.
2. Otherwise the termination must be `Completed`, `Stopped{Client}` (the test's own `finish`), or what `expecting` named: `Refused` = `Failed` at `Validate|Prepare|Arm`; `Policy(kind)` = `Stopped{Policy{event == kind}}`; `Any` = every end that is not a limit (for a test that records how the Run ended: the bench's B8/B9 loops). Anything else → a panic **without** a marker ("the Run ended unexpectedly: …"): a failure the code under test caused, which is the kill evidence.

The pure part — `fn judge_end(termination, limit: Option<&str>, started: bool, declared) -> Judged { Fine | Limit(name) | Unexpected }` — is what the unit tests and the mutation rows hit (§7).

### 1.4 A Run that has ended

`Core::Ended(Box<Ended{manifest, now, t0, clocks, effective}>)`. `state()` = `CleanedUp{termination: manifest.termination.reason}`; `submit`/`advance_to`/`wait_for`/`run_until_end` = `Err(RunHandleError::Ended{termination})` (the Kernel answers the same after the end: `ensure_live`, `stepping.rs`, `mod.rs:214`); `events(from)` = `manifest.events.delivered` from `from` (the Manifest copies `shared.delivered`, `ending.rs:146`); `now`, `start_instant`, `sample_clocks`, `effective` = the values read just before `finish`; `finish` = the stored Manifest (already judged). The Kernel's `finish(self)` consuming the handle is why the Manifest is stored here: this is the "折り合い" note 51 §11 left to 2a. Lazy alternative rejected (judge only the reason prefix at a return and the Manifest at `finish`): it would leave a test that reads device records between a Stop's end and its `finish` unprotected against an RS-8a cut (N2); the eager form costs one stored struct.

### 1.5 `finish`, `Drop`, unwind

- `finish(self)`: `Live` → `RunHandle::finish`, `judge`, return; `Ended` → the stored Manifest.
- `Drop`: `thread::panicking()` → nothing (the Kernel's own `RunHandle` Drop cleans a device-paced Run, KC-46c); else a `Live` inner is finished and judged like `finish` (the Manifest is discarded, as the Kernel's Drop discards it). A panic here is a first panic. This is the answer to "a test that forgets `finish()`": no scan. V: one test in the suite does it (`the_candidate_ceiling_admits_what_the_profiles_link_limit_refuses` drops `admitted`, `fake.rs:6808`); the prototype judged it.
- A `finish()` that does not return is the watchdog's (marker + abort, `watchdog.rs`).

## 2. Run creation sites and expected-end tests (re-found on `42f5031`)

**Sites.** Search: `grep -rnE '\bconnect\(|\bstart_spec_run\(' crates/ezsdr-radio-uhd/tests crates/ezsdr-acceptance/tests/uhd.rs | grep -v disconnect` (not cut), FakeDevice files only.

| # | site | becomes |
|---|---|---|
| — | `common/mod.rs:548` `session_on` (so `session`, `session_with`, `tx_session*`, `transmit_run`, `rehearse_session_loopback`, `ma1_run`) | `Run::connect`; the `assert Running` stays |
| — | `common/mod.rs:493` `start_spec` (so `spec_run`, `spec_run_ended`, `captured` callers) | `Run::start_spec`; `spec_run_ended`/`start_spec` pass `Expect::Refused` |
| 1 | `common/mod.rs:843` `rehearse_overflow` | `Run::start_spec` |
| 2 | `common/measurement.rs:1904` `candidate_session_asking` (returns `Err(String)` on a `SpecError`) | `Run::connect` |
| 3 | `common/measurement.rs:2636` (`rejected_run_reason`: a Run refused at validate/prepare is a result) | `Run::start_spec`, `Expect::Refused` |
| 4 | `common/measurement/multichannel.rs:791` `run_stage` | `Run::start_spec`, `Expect::Refused` |
| 5–8 | `fake.rs:223` (`ur_06_…`: a refused root, `Prepare`), `:537`, `:3831`, `:6808` | `Run::start_spec`; `:223` `Refused` |
| 9–11 | `ezsdr-acceptance/tests/uhd.rs:51` `uhd_spec_run`, `:66` `uhd_session_asking`, `:264` `session_with_layout` | `Run::start_spec`/`connect`; `:264` gets a `Declared` parameter, `Refused` from the three tests that expect a refusal (`:187`, `:379`, `:395-397`) |

That is the twelve of note 51 §5.4 (the numbers' lines have moved: `mod.rs:460`→`:493`, `:785`→`:843`, `fake.rs` `+2…+60`, `measurement.rs:2628`→`:2636`) plus `session_on`/`start_spec`. `measurements.rs`, `x3xx.rs`, `parity.rs` create no FakeDevice Run (no hit). Kernel's/`v58.rs`/`reactive.rs` Runs have no wrapper (out of scope, note 51 §8).

**Type-only edits.** `RunHandle` appears 84 times in the five radio-uhd test files and `uhd.rs`; all become `Run` (compiler-driven; no signature changes otherwise).

**Tests that expect an end that is not a deadline.** The note's grep (`Termination::Failed|StopCause::Policy|RunState::CleanedUp|RunHandleError::Ended`) re-run on this tree gives 20 `fake.rs` fns (21 before), 3 `uhd.rs`, 1 `hardware.rs`, 2 helpers; but that grep over-counts (the creation-time refusals already go through `spec_run_ended`). The exact set is what the prototype **failed**: 22 of 432 `fake` tests (V, `failed-with-wrapper.txt`), which I classified:

| class | tests | declaration |
|---|---|---|
| Policy end (DEVICE_LOST / CLOCK_LOST) — 11 | `ur_16_a_panicking_thread_is_a_lost_device`, `…_ends_a_two_stream_run_with_its_message`, `ur_16_a_uhd_control_panic_ends_the_run_with_a_lost_device`, `…_while_a_submit_waits_ends_it_with_the_loss`, `…_with_no_stream_sending_ends_the_run`, `ur_27_a_lost_reference_is_clock_lost`, `ur_28_a_failed_report_read_is_not_an_empty_one`, `ur_29_a_lost_device_aborts_the_run`, `ur_29_a_silent_stream_is_a_lost_device`, `ur_29_a_dead_link_that_fails_without_lost_is_a_lost_device`, `ur_29_a_lost_device_is_found_while_idle` | `.expecting(Expect::Policy("ezsdr.DEVICE_LOST"))` / `("radio.CLOCK_LOST")` at the `spec_run`/`tx_session` call |
| refusal at creation through the raw `start_spec_run` — 3 | `ur_06_the_provider_refuses_another_root`, `a_stage_above_the_admission_ceiling_…`, `rejected_runs_are_admission_rejections_…` | `Expect::Refused` (sites 3, 5) |
| healthy Run finished before T0, join judged — 5 | `ur_13_arm_cases`, `ur_13_arm_compares_the_rate_set_after_the_streamer_opens`, `ur_13_arm_takes_the_rate_set_in_any_order`, `ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid`, `the_candidate_ceiling_…` (the dropped Run) | **none**: fixed by recommendation 5 (join only after T0); they must pass undeclared |
| join is the subject — 3 | `ur_16_a_wedged_thread_does_not_wedge_stop`, `ur_16_cleanup_keeps_the_streamer_of_a_detached_thread`, `ur_26_the_sending_threads_join_before_the_receiving_ones` | `.subject_join()` |

(11 + 3 + 5 + 3 = 22.) No test failed on KC-12a, KC-21a or RS-8a, i.e. no UHD Run test has a deadline as its subject. `uhd.rs`: three tests expect `Validate` refusals (`Refused`, above). `hardware.rs`: `hw_b9_unplug*` ×3 `Expect::Policy("ezsdr.DEVICE_LOST")`; `hw_perf_rx_rates` records how each rate's Run ended: `Expect::Any`; the `until_not_running` helper takes the declared Run.

## 3. Shared trials and `hardware.rs`

Type only: `front_trial`, `long_close_trial`, `repeat_switch_trial`, `untimed_*_outcome`, `one_sample_after_a_burst`, `held_tail_trial`, `stop_next_trial`, `stop_end_trial`, `transmit_run` (`n47s5.rs`), `short_first_case`, `repeat_sizes_case`, `one_sample_pair` (`e3.rs`), the `measurement.rs` helpers, `ma1_run`, `rehearse_*` (`mod.rs`) take/return `Run`. V: with the wrapper, `cargo check --tests` of the two crates passes except `hardware.rs`'s three feature-only symbols (no wrapper involvement). The Run-creating `hardware.rs` helpers (`tx_session`, `tx_only_session`, `tx_session_at`, `until_not_running`) change their return/argument type. Gate: `hw-build` (`cargo test --features uhd --test hardware --no-run`) and `uhd-clippy` compile it; behaviour at the next bench.

**Audit table (AGENTS §9: hardware tests whose expectation comes from a rule).** 2a changes no expected value, bound, front or rate grid; what it changes is *what a Run that ends or hits a bound does*:

| hardware test / helper | expectation | after 2a | verdict |
|---|---|---|---|
| every caller of the shared trials (`hw_t5_*`, `hw_n47s5_*`, `hw_b7*`, MA-1 …) | trial verdicts of `iii`/`n47s5` | unchanged: forwards only | still right |
| `hw_b9_unplug`, `…_transmit_only`, `…_and_reopen` | the Run ends by DEVICE_LOST when the cable is pulled; prints the termination | declared `Policy(DEVICE_LOST)`; no unplug → still Running → `finish` = Client, as before | still right |
| `hw_perf_rx_rates`, B8 lead loop (`let _ = run.submit/advance_to`) | record how/whether a Run ended; ignore `Err(Ended)` | `Expect::Any` for `hw_perf_rx_rates` (an end is recorded, not asserted); the B8 loop and any helper that ignores `Err(Ended)` now **panic** at the method that saw an undeclared end, before their evidence is printed | **cannot tell**: whether a bench Run ends inside those loops (loss, Policy on overflow) — owner/orchestrator to decide `Any` for each such test; found by `grep -n 'let _ = run\.\|let _ = .*advance_to\|let _ = .*submit' hardware.rs` (3 lines) plus `past_t0`/`past_tx_origin`/`wait` (they `let _ =` the result too) |
| any bench Run that reaches KC-21a/RS-8a/a post-T0 join detach | a late assertion fails | `EZSDR-INFRA:` marker and one panic at the call; the bench log shows the limit | fixed (an improvement, not a verdict change) |
| env `EZSDR_RIG_DELAY_MS` | none | unset on the bench; injection is read by `run.rs` only if set | still right |

**Default-suite rehearsals that pin the same boundaries.** The wrapper changes no boundary a bench test computes from a rule, so no new rehearsal is owed; the existing rehearsals (`n47s5_*_rehearsal_on_the_fake`, `ur_*` Run tests) go through the same wrapper type, so a type/forwarding mistake turns them red without hardware. The wrapper's own unit tests (§7) pin its rules.

## 4. `protocol.rs`

Search: `grep -c '\.handle(' crates/ezsdr-server/tests/protocol.rs` → **82** (7 `let handled = server.handle`, 2 `Server::new(..).handle`, the rest inside `ok(…)`/`err(…)`). The plan: one fn

```rust
fn handle(server: &mut Server, request: Request, body: Vec<u8>) -> Handled { rig_delay("server"); server.handle(request, body) }
```

and the 82 call sites become `handle(&mut server, …)` (mechanical; `Server::new(..).handle(..)` becomes `handle(&mut Server::new(..), ..)`). Its injection point: `rig_delay` sleeps `EZSDR_RIG_DELAY_MS` ms after the call returns when `EZSDR_RIG_DELAY_AT` contains `server` or `all` (default off). No classification here (note 51 §8: out of scope; tables §1 #5: only `ea_15` uses another entry, `serve_watched`; `serve`/`serve_bytes`/the child process stay). Rejected: reading `Ended`+`termination` from the reply to classify K1–K3 here — possible in the same fn, but it is a new rule for 18 FakeDevice tests, outside §7.2's row.

## 5. The scan (`ezsdr-acceptance/tests/governance.rs`)

A pure `fn scan(path: &str, source: &str) -> Vec<String>` plus `#[test]`s that run it over the files and over strings. Lines are cut at `//` (as `po_11` does); the enclosing `fn` is tracked with `^\s*(pub )?(const )?fn (\w+)`, and a fn is a test if an attribute run above it contains `#[test]`.

- **Forbidden outside `tests/common/run.rs`**: `connect(` (not `disconnect(`), `start_spec_run(`, `RunHandle` (but `RunHandleError` allowed). Scope: every `.rs` under `crates/ezsdr-radio-uhd/tests/` (recursive, so a new file is covered) and every `.rs` under `crates/ezsdr-acceptance/tests/` **except** the file-level allow list `{v58.rs, v61.rs, reactive.rs, parity.rs, measurements.rs, x3xx.rs, o1_oracle.rs, repeat_switch.rs, governance.rs, support/, oracle/}` (each with its reason in the table: "no FakeDevice Run: Mock/stepped Runs, a Provider-direct rig, or this scan"). `hardware.rs` is scanned for these three tokens (it must use the wrapper).
- **Forbidden inside a `#[test]` fn** (files: `fake.rs`, `tests/common/**`, `uhd.rs`; not `hardware.rs`, whose raw probes read device time by design): `thread::sleep`, `Instant::now`, `.elapsed()`, `recv_timeout(`, `.time_now(`, `.host()`. Helpers (non-test fns) are not scanned for these (note 51 §5.4: "helper の中だけ"). Today's hits inside `#[test]` fns: **24 fns** (23 in `fake.rs`, 1 in `probe_device.rs`; none is a Run-based test: they are Authority, `Direct`-rig and raw-device tests) — V by script over the current tree. They form the **per-fn allow list** (`(file, fn, rig)` rows in the test, each with its rig as the reason). A new test that reads time itself fails the scan until it is either moved into a helper or listed with a reason in review.
- **`protocol.rs`**: `.handle(` forbidden outside `fn handle`.
- **Self-test** (strings): each forbidden token in a test fn is found; the same token in a helper fn is not; an allow-listed `(file, fn)` passes and another fn of that file does not; `disconnect(` and `RunHandleError` do not trip; a `//` comment does not trip; a file-level exempt file is skipped; the real tree gives no violation.

What the scan cannot do (said plainly, note 51 §8): it sees tokens, not time sources reached through a helper, so it stops the new bypass, not every bypass.

## 6. End-to-end fixtures 4, 4b, 6, 7 (`mutate.py --e2e`)

All four share one test-local device in `infra_fixtures/mod.rs`:

```rust
struct Stalls { inner: Arc<FakeDevice>, ref_locked: AtomicBool, close: AtomicBool }   // forwards the 25 Device methods;
// ref_locked(): when armed, std::thread::sleep(8 s); close_streams(): when armed, std::thread::sleep(7 s)
```

(a forwarding wrapper like the eight already in `fake.rs`; ~60 lines). **Both of note 51's INFERRED forms changed** (tables §1 #2, #6): a Spec `tx_burst` cannot reach KC-21a #3 (right), and `Gated` + a Manual clock does not reach it either — the held participant freezes the clock before the round. What works (V in `n51-2a-base`, today's behaviour first, then in the prototype):

| # | sequence | today (no wrapper): V | with the wrapper: V |
|---|---|---|---|
| 4 | `fake_real` device in `Stalls`; `session(external clock)`; arm `ref_locked`; `wait(700 ms)` (uhd-control's 500 ms check now blocks); `submit(set gain)`; **then** `assert!(false, "FIXTURE-4-AFTER")` | `submit` returns `Ok` after 6.1 s with the Run `CleanedUp{Failed KC-21a}`; the assert fails; **no marker**, `test result: FAILED` → `mutate.py` says `killed` | `EZSDR-INFRA: … a submit met KC-21a …`, one `panicked at`, the `FIXTURE-4-AFTER` text never printed |
| 4b | `fake_real`/`Stalls`; a Spec with `schedule: [stop at offset 50 000 ticks]` (the untimed-path template: only a Spec `stop` with a target reaches the agenda, tables §1 #2); arm; `advance_to(+3 s)` (crosses T0, dispatches the stop) | `advance_to` returns after 8.2 s, `CleanedUp{Failed KC-21a}`, `FAILED`, no marker | marker `an advance met KC-21a`, one panic, no later text |
| 6 | manual-clock `Stalls`; `session`; `past_t0`; arm `close`; `finish()`; read `section(.., "FIXTURE-6-AFTER-bursts")` | `finish` returns after 5.1 s; `cleanup_failures` has `RS-8a: abandoned after 5000 ms, timed_out: true` and `KC-44`; `section` panics `no section …` — `FAILED`, no marker | marker `the Run met RS-8a …`, one panic **inside `finish`**, the section read not reached |
| 7 | manual-clock `Stalls` with `FakeFault::Unlocked(2.3 s)`; Spec with `clock_source: external`; `.expecting(Expect::Policy("radio.CLOCK_LOST"))`; arm `close`; `run_until_end(+5 s)`; later `section(..)` | Run ends `Stopped{Policy CLOCK_LOST}` (as ur_27), cleanup 5 s, `RS-8a timed_out`, `no section` panic, no marker | marker `a wait met RS-8a …` **at the `run_until_end` return** (the eager take); the declaration did not exempt the limit |

Note: fixture 7's end is CLOCK_LOST, not DEVICE_LOST (note 51 §6.4): a lost device skips `close_streams` (`src/provider/mod.rs`, `cleanup`: `else if !core.is_lost() { …close_streams() }`), so no cleanup step stalls. Each fixture is `infra_fixture_<name>` (returns at once unless `EZSDR_INFRA_FIXTURE` names it), **not** a mutation filter. `mutate.py` `E2E` gets four cases — `{"id": "4-submit-limit", "fixture": "submit_limit", "bound": False, "expect": "ERROR", "markers": 1, "panics": 1, "has": [r"EZSDR-INFRA: [^\n]*KC-21a", r"test result: FAILED"], "lacks": [r"FIXTURE-4-AFTER"]}` and likewise `4b-agenda-limit` (`KC-21a`), `6-cleanup-limit` (`RS-8a`, lacks `no section`), `7-declared-end-cleanup-limit` (`RS-8a`) — and `e2e_case` learns `lacks` (patterns that must **not** match; today only `has`). The self-test (`mutate.py:439`, "every case names a fixture") covers the new names; `--self-test` gets a string cell for `lacks`. The gate's `e2e` step counts `N/N`: 9 → 13. Each case adds 6–9 s to the e2e step.

An eighth fixture is worth adding but is not in the brief: **8, join at `finish`**: `Gated` (real clock) holds a transmit call, undeclared → marker `met join` (the J317/J318 mechanism; today the only coverage of join-at-finish is the two `bound` rows). Recommended as part of sub-step 5; Sol may drop it.

## 7. Tests for the wrapper and the failing-first evidence

Default-suite tests (all in `run.rs`/`fake.rs`; none needs the env var):

| test | asserts / observes | wrong behaviour that fails it |
|---|---|---|
| `run_judge_end_*` (pure, ~9 cells) | `Completed`, `Stopped{Client}` fine; `Failed{Run,..}` unexpected; `Failed{Arm}` fine only with `Refused`, not `Failed{Run}`; `Policy(k)` fine only for event `k`; `Any` accepts every end but a limit; a limit name → `Limit(name)`; `join` with `started == false` → not a limit; `join_is_subject` exempts join but not RS-8a/KC-21a | a mutant that exempts every limit under a declaration; one that counts join before T0 (the 5 healthy tests); one that lets `Refused` accept a Run-stage failure; one that swaps `Policy` equality |
| `run_a_policy_end_is_returned_by_every_method_and_panics_when_undeclared` ×4 (`submit`, `advance_to`, `wait_for`, `run_until_end`; `#[should_panic(expected = "ended unexpectedly")]`; a `FakeFault::Lost` Manual-clock Session, ~0.5 s each) | an undeclared Policy end panics **in the method that returned it**, not later | `check` missing from one method (4 rows) |
| `run_a_declared_end_answers_after_the_end` | declared Run: `run_until_end` → `Err(Ended)`; then `state()` is `CleanedUp{that termination}`, `submit`/`advance_to` → `Err(Ended)`, `events(0)` = the Manifest's, `finish()` = the Manifest, the device saw `close_streams` once | Ended-mode `submit` returning `Ok`; `events` from the wrong list; `finish` calling the handle twice |
| `run_a_dropped_live_run_is_judged` (`should_panic(expected = "met join")`, ~3 s: a `Gated`-held real-clock Session past T0 dropped undeclared) | `Drop` runs `judge` | a `Drop` that only drops |
| `run_unwinding_never_panics_again` (`should_panic(expected = "boom")`: the same wedged Run alive while the test panics) | the process neither aborts nor masks "boom" | `Drop` judging while panicking (double panic = abort) |

**Failing-first evidence** (V above): today's false `killed` for each of 4, 4b, 6, 7 (§6, "today" column). The unit tests have no sequence today (the wrapper does not exist); their failing-first form is the mutation rows (§8): each row's mutant must fail the named test by an assertion, never by a timeout or marker.

## 8. Mutation rows (Rust token anchors; Python exact substrings)

New (in `plan/maintenance/tools/mutations.json`, list `maintenance`, ids `N51-2A-nn`), each with its killer; rows run through `remote_mutate.py` only. All target `tests/common/run.rs` (and `governance.rs`) — test code, as stage 1's rows did:

| row | mutant | killer |
|---|---|---|
| 01 | `judge_end`: a declared `join_is_subject` exempts any limit (`name == "join" &&` removed) | `run_judge_end_a_subject_exempts_only_join` |
| 02 | `judge_end`: join counted when `!started` | `…_join_before_t0_is_not_a_limit` |
| 03 | `Refused` accepts `Failed{Run}` | `…_refused_is_validate_prepare_arm_only` |
| 04 | `Policy(kind)` accepts any Policy event | `…_policy_names_its_event` |
| 05 | `Any` accepts a limit | `…_any_is_not_a_limit_pass` |
| 06–09 | `check` removed from `submit` / `advance_to` / `wait_for` / `run_until_end` | the matching `run_a_policy_end_is_returned_by_every_method…` |
| 10 | Ended-mode `submit` returns `Ok` | `run_a_declared_end_answers_after_the_end` |
| 11 | `Drop` does not judge | `run_a_dropped_live_run_is_judged` |
| 12 | `Drop` judges while panicking | `run_unwinding_never_panics_again` |
| 13–18 | scan: a token removed from the forbidden list / the `#[test]` detector ignores `#[test]` / the helper exemption removed / allow-list match by file only / `RunHandleError` trips / file-level exempt list ignored | the scan's self-test cells |
| 19 | `mutate.py` `e2e_case`: `lacks` ignored (`"lacks"` loop removed) | `mutate.py --self-test` cell (Python: killed by the self-test, which `gate.py` already runs) — if the runner does not support non-Rust rows, this one is covered by the e2e step only; I will check `anchor.py` and say so |

Rows that exist and are touched:

- **Filter names a test whose declaration changes** (27 rows, computed by script over the 2 287 rows of the six lists, substring match as `cargo test <filter>` does): J44, U03 (`ur_06_…`); P246, P273, T34, N92-1 (selects the three `ur_16_a_uhd_control_panic_*`), U27, U29, B33, U43, R10, B35 (the Policy tests); P878–P882 (`rejected_runs_are_…`); P294, P296, P919, N49F-14 (healthy-before-T0, must stay `killed` by their own assertions); P274, P275, P277, U42 (join subject; their killers are the `rejected` thread-name asserts, unchanged). Their killers do not change; they run with the full lists at each sub-step's commit (§7 AGENTS) and the first full run may show an `ERROR` (as M's did: 5 rows) — settled one by one.
- **J317, J318** (`ur_33_a_shutdown_with_a_report_read…`, `ur_29_a_loss_found_by_the_shutdown_s_report_read…`): killed today by `assert!(!… "did not join")`; both Runs pass T0, so with the wrapper the join marker comes first → `ERROR`. Both get `"kill": "bound"` in the sub-step that switches the Run type (tables §6.1 already says so; M declared only G16: `grep -c '"kill"'` shows G16 alone).
- Every other row of the `fake`, `uhd`, `protocol` lists runs through the changed plumbing (type-only): the full lists run at each sub-step's commit, and Sol's review follows a green full run (§9 step 6).

## 9. Diagnostic delay injection (report only; last sub-step)

`run.rs` (and the `protocol.rs` `handle`) read `EZSDR_RIG_DELAY_MS` (default unset = nothing) and `EZSDR_RIG_DELAY_AT` (`now`, `advance`, `submit`, `server`, comma list, `all`), once into a `OnceLock`, and `std::thread::sleep` that long: after `now()`/`start_instant()` returns (`now`), after `advance_to`/`wait_for`/`run_until_end` (`advance`), after `submit` (`submit`). It is runtime, not `cfg(feature = "uhd")`: the acceptance crate has no such feature, and `unexpected_cfgs` would fail clippy there. The bench never sets it. Sweep, run by a small script outside the gate: `{now, advance, submit} × {5, 20, 60, 200} ms` over `cargo test -p ezsdr-radio-uhd --test fake` (one baseline run takes 134 s here at `--test-threads=8`, so ≈ 27 min) plus `uhd` and `protocol`; report "passes without injection, fails with it" per test, with note 51 §5.2's table as the comparison (probe 6: 10/2/5 failures at 60 ms). It gates nothing and kills nothing.

## 10. Text 2a makes false, and what the code contradicts

Contradictions found (V unless marked):

1. **note 51 §6.2 item 2 and tables §4.2 U1 ("classify `did not join within`")**: a healthy Run finished before T0 records it (1.1 s `finish` vs 114 ms after T0; plain `session(...)` + `wait(20 ms)` + `finish`, both clocks). 5 of 22 prototype failures. 2a classifies join only after T0. **Orchestrator: file an issue** (`uhd-rx` seems not to honour `Shutdown` while it waits for its start; a user-visible 1 s stop delay if so — INFERRED cause; the 1.1 s is V).
2. **note 51 §6.4 / tables §1 #6 (fixtures 4, 4b)**: `Gated`+Manual cannot reach KC-21a #3; a Spec `tx_burst` cannot either; the forms in §6 work. Fixture 7's end is CLOCK_LOST (a lost device skips `close_streams`).
3. **note 51 §6.2 item 1 / §11 ("包みが Manifest を取る … 折り合いは 2a で決める")**: decided in §1.4 (store it; no test uses a post-end accessor).
4. **note 51 §6.2 "宣言は 2 種類"** names four limits; 2a has one (join) and an `Expect` of three forms, because no UHD Run test has KC-12a/KC-21a/RS-8a as its subject (0 of 432 failed on them).
5. **note 51 §6.2 "残るもの"** ("scan or `#[must_use]`"): neither; `Drop` (§1.5). And "包みの `Drop` は panic しない": it panics only when not unwinding.
6. **tables §3.1 line numbers** (`mod.rs:460`, `:785`, `fake.rs:221/535/3786/6749`, `measurement.rs:2628`): now `mod.rs:493/843`, `fake.rs:223/537/3831/6808`, `measurement.rs:2636`. **tables §3.2 / §6.1**: no change in classes; J317/J318 get `bound`.
7. **tables §3.4 / note 51 §5.4 "`server.handle(` を 1 fn に"**: 82 call sites (not "one").
8. `tests/README.md:58`: "`session_with`, `spec_run`, the parity rig's constructor …" — now: the wrapper's constructors and every method return; add the `Drop` and `EZSDR_RIG_DELAY_*` lines. note 51 §7.2 row 2a, §8 (the two "検出（2a の後）" rows), §10 Q-text for 2a, the paragraph after stage 1d (add "段 2a の後"). `plan/maintenance/failure-mechanisms.md` (`mech:wall-clock-test-oracle` row: one sentence after the gate). `handoff.md` §2 item 1 (the order line) and its 負荷に弱いテスト row (one sentence). `KNOWN_LIMITATIONS.md`: nothing (no user-visible Ceiling changes). `AGENTS.md`: nothing.
9. Not contradictions but unknown: the bench behaviour of the three B8/`past_*` helpers (§3 audit: cannot tell); the cause of the pre-T0 join; that the join rule (`now >= t0`) holds for all 3 subject tests and J317/J318 (I = to be run in sub-step 2).

## 11. Sub-steps (each leaves the tree green; a local commit each; full mutation lists at each commit)

| # | content | size |
|---|---|---|
| 1 | `run.rs` + its unit tests (pure `judge_end`, the Ended-mode and Drop tests need the switch, so they come in 2); `Stalls` not yet. Dead code at this point (`common/mod.rs` already allows it); the acceptance include is not yet added. Rows 01–05 | ~230 lines new, 1 file + `mod.rs` 2 lines |
| 2 | **The switch in `ezsdr-radio-uhd`**: `session_on`, `start_spec` (and its two wrappers) through `Run`; all 84 `RunHandle` → `Run` in `common/{mod,n47s5,e3,measurement}.rs`, `fake.rs`, `hardware.rs`; the 8 creation sites 1–8 of §2; the 11 Policy + 3 refusal + 3 subject declarations; B9 ×3 and `hw_perf_rx_rates` declarations; J317/J318 `bound`; the method/Ended/Drop tests and rows 06–12. This cannot be split (the type change is one compilation unit) | ~200 edited lines in 6 files (mostly type names), ~120 new test lines. Verified locally: `cargo test -p ezsdr-radio-uhd --test fake` (134 s) and the cfg-removed `hardware` check; the gate does `hw-build` |
| 3 | **`ezsdr-acceptance/tests/uhd.rs`**: `#[path]` include, sites 9–11, `Declared` parameter on `session_with_layout`, `effective`/`state` as methods | ~25 lines |
| 4 | **`protocol.rs`**: the `handle` fn, 82 call-site edits, the injection point (`protocol.rs` cannot include `common`, so it gets its own 6-line `rig_delay` reading the same env vars: one more copy of a 6-line reader, accepted; the alternative is another `#[path]` include of a file that pulls in Kernel types) | ~90 edited lines, 1 file |
| 5 | **Fixtures** 4, 4b, 6, 7 (+8 if kept): `Stalls`, the `infra_fixtures` tests, `mutate.py` `E2E` cases and `lacks`, the self-test cells | ~150 lines Rust, ~40 lines Python. The e2e step is run remotely (`remote_mutate.py --e2e`); locally I run each fixture by hand |
| 6 | **The scan** in `governance.rs`: `scan`, the allow lists, the self-test, rows 13–18 | ~140 lines |
| 7 | **Delay injection** (`rig_delay` in `run.rs`, env parsing, 3 tiny tests: unset = no sleep; the point list parses) and the sweep script + one report in the step's record | ~60 lines + a script outside the repo |
| 8 | Text of §10 (note 51, tables, README, handoff, failure-mechanisms), the review record skeleton | docs |

The order differs from the brief's suggestion (the brief put the 12 sites, then the shared trials, then `hardware.rs`, as separate steps): they cannot be, because the sites return `Run` and the trials take it; one switch is the smallest green change. Sub-steps 3–7 are independent of each other after 2.

## 12. Summary of what I did not do

- No Kernel, production, `mutations.json`, `mutate.py` or test-file edits in the tree (all in `/tmp/opencode/n51-2a-*`); no `mutate.py`/`remote_mutate.py`; no sim02/sim03; no commit.
- Not run: the `uhd` feature build (no libuhd here: V `build.rs:19` fails; the gate's Docker image has it), `protocol.rs`, `watch.rs`, the Python suites, the mutation rows, the sweep.
- Prototype caveat: the `fake` suite ran once at `--test-threads=8`; the 22 failures are classified from their messages; the join-after-T0 rule is not prototyped (the five tests are the evidence that a rule is needed, and `start_instant`/`now` are the inputs it needs).

## Changes in sub-step 1 (implemented 2026-11-12; no commit)

Files: `crates/ezsdr-radio-uhd/tests/common/run.rs` (new, 9 default-suite tests), `common/mod.rs` (`pub mod run;`, one line), `plan/maintenance/tools/mutations.json` (rows `N51-2A-01…07`, list `maintenance`). Nothing switches a caller; `Run` is dead code (`#![allow(dead_code)]`, as `infra.rs`). The acceptance crate does not include it yet. `mod.rs` has no `pub use run::{…}`: the switch adds it.

**What `run.rs` has.** `Expect {Refused, Policy(kind), Any}`; `Declared {end, join_is_subject, rx_join_issue_67}`; `Judged {Fine, Limit(name), Unexpected}`; the pure `judge_end(termination, cleanup_timed_out, rejected: &[(thread, reason)], declared)`; `Run` with `connect`/`start_spec` (check at creation), the builders `expecting`/`join_is_subject`/`rx_join_issue_67`, the plain forwards `state/now/start_instant/sample_clocks/effective/events`, the checking calls `submit/advance_to/wait_for/run_until_end`, `finish(self)`, and `Drop`. `check` stores the Manifest in `Core::Ended` **before** it judges (M2). `judge` writes the marker and panics for a bound, panics for an unexpected end, and only writes the marker while `thread::panicking()` (M3). `Drop` finishes and judges a live Run. The marker text carries the `rejected` notes with their thread names (M1's check of the five tests).

**Differences from the plan's §1:** no `started`/`now >= t0` in `judge_end`, and `Ended` no longer needs `t0` for it (it keeps `now`, `t0`, `clocks`, `effective` as accessors' answers); `Run::join_is_subject()` and `rx_join_issue_67()` replace `subject_join()`; one `#[allow(clippy::large_enum_variant)]` on `Core` (a `RunHandle` is 1.6 KB; boxing it would touch every method for a type that exists once per test).

**Tests (pure, run once; `cargo test -p ezsdr-radio-uhd --test fake run_judge_end`: 9 passed):**

| test | asserts |
|---|---|
| `run_judge_end_a_completed_or_client_stopped_run_needs_no_declaration` | `Completed`, `Stopped{Client}` fine undeclared; `Abort` and `Failed{Run}` unexpected |
| `run_judge_end_refused_is_validate_prepare_arm_only` | `Refused` accepts the three stages, not `Run`; an undeclared refusal is unexpected |
| `run_judge_end_policy_names_its_event` | `Policy(k)` accepts event `k` only, undeclared Policy is unexpected, the ordinary ends stay fine |
| `run_judge_end_a_bound_is_named_before_any_end` | KC-21a, KC-12a (under `Refused`), RS-8a, a `uhd-tx` join (under a Policy declaration) are `Limit` |
| `run_judge_end_any_is_not_a_bound_pass` | `Any` accepts non-bound ends, still `Limit` for KC-21a, RS-8a and join |
| `run_judge_end_a_join_is_a_bound_unless_it_is_the_subject` | an undeclared `uhd-rx` join (a join before T0 included) is `Limit("join")`; `join_is_subject` makes it fine; another `rejected` note is not a bound |
| `run_judge_end_a_subject_exempts_only_join` | with `join_is_subject`, RS-8a, KC-21a, KC-12a stay `Limit` |
| `run_judge_end_issue_67_exempts_the_join_of_a_receive_thread_only` | `uhd-rx-s0`/`s1` joins fine; `uhd-tx-s0`, `uhd-control`, or a tx join beside an rx one are `Limit("join")`; undeclared rx join is `Limit` |
| `run_judge_end_issue_67_exempts_no_other_bound` | RS-8a, KC-21a, KC-12a stay `Limit`; a Failed Run end stays `Unexpected` (it declares no end) |

**Rows** (`maintenance`, `crate ezsdr-radio-uhd`, `test_bin fake`; run on sim03 with `remote_mutate.py maintenance sim03:4 N51-2A-01…07`, log `impl1-mut.log`: **7/7 killed**, none `ERROR`, no timeouts). The failing assertion of each is derived from the test and the mutant, not observed: a mutant is never applied to a tree here (AGENTS §7), and the runner prints only the outcome.

| row | mutant | killer and the assertion it fails |
|---|---|---|
| N51-2A-01 | `join_is_subject` filters every limit | `…_a_subject_exempts_only_join`: `assert_eq!(…Completed, true, [tx join], subject, Limit("RS-8a"))` gets `Fine` |
| N51-2A-02 | `#67` skips the thread test (`declared.rx_join_issue_67` alone) | `…_issue_67_exempts_the_join_of_a_receive_thread_only`: the `uhd-tx-s0` line expects `Limit("join")`, gets `Fine` |
| N51-2A-03 | the rx exemption ignores its declaration | `…_a_join_is_a_bound_unless_it_is_the_subject`: the undeclared `uhd-rx-s0` join expects `Limit("join")`, gets `Fine` |
| N51-2A-04 | `#67` also drops `cleanup_timed_out` | `…_issue_67_exempts_no_other_bound`: `Completed`, `true`, rx join expects `Limit("RS-8a")`, gets `Fine` |
| N51-2A-05 | `Refused` accepts any stage | `…_refused_is_validate_prepare_arm_only`: `Failed{Run}` under `Refused` expects `Unexpected`, gets `Fine` |
| N51-2A-06 | `Policy(k)` accepts any Policy event | `…_policy_names_its_event`: `CLOCK_LOST` under `Policy(DEVICE_LOST)` expects `Unexpected`, gets `Fine` |
| N51-2A-07 | `Any` skips the bound | `…_any_is_not_a_bound_pass`: `Failed{Run,"KC-21a: late"}` under `Any` expects `Limit("KC-21a")`, gets `Fine` |

Rows with no killer yet (they need the switch, sub-step 2): `check` removed from one method (4), Ended-mode `submit`, `Drop` not judging, `Drop` judging while panicking, `judge` not writing the marker while unwinding. Existing rows whose filter names a test the plan changes are untouched in sub-step 1 (no test changed).

**Gates run:** `cargo fmt --all` (clean); `cargo test --workspace` (every binary `ok`; `fake` 441 = 432 + 9 new); `cargo clippy --workspace --all-targets -- -D warnings` (clean after the one `allow`); `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 42f5031` (0 problems, every row anchors exactly once). Not run: the `uhd` feature build (no libuhd here); `run.rs` is compiled in `hardware.rs`'s binary only at the gate's `hw-build`.

## Changes in sub-step 2 (implemented 2026-11-12; no commit)

Files: `common/{mod,e3,n47s5,measurement,run}.rs`, `common/measurement/multichannel.rs`, `fake.rs`, `hardware.rs` (all `crates/ezsdr-radio-uhd/tests/`), `plan/maintenance/tools/mutations.json`, `plan/phase7/tools/mutations.json` (R15 `"kill": "bound"`).

**The switch.** `session_on`, `start_spec`, `spec_run` and `spec_run_ended` go through `Run`; every `RunHandle` in `tests/` is `Run` (the 74 sites found by grep, type-only). Raw creation sites left in this crate: none (`grep -rnE '\bconnect\(|\bstart_spec_run\(' tests | grep -v disconnect` gives only `run.rs`). Sites: 1 `rehearse_overflow` (`Run::start_spec`, undeclared), 2 `candidate_session_asking` (`Run::connect`, undeclared), 3 `measurement.rs` overflow Run and 4 `multichannel.rs` `run_stage` (`Refused`), 5 `ur_06_…` (now `spec_run_ended`, so no raw site), 6 and 7 plain `Run::start_spec`, 8 `the_candidate_ceiling_…` (`rx_join_issue_67`, explicit `admitted.finish()`).

**Declarations.** Policy: 11 tests (`DEVICE_LOST`; `ur_27_…` `radio.CLOCK_LOST`), at the creation of the Run, with `.expecting(…)`; `ur_28_a_failed_report_read…` only on its second Run. Refusal: `start_spec`/`spec_run_ended` (`Refused`). Join subject: `wedged()` (two tests) and `ur_26_…` (`join_is_subject()`). #67 (`rx_join_issue_67()`): five tests. Bench: `hw_b9_unplug`, `hw_b9_unplug_transmit_only`, `hw_b9_unplug_and_reopen` (`Policy(DEVICE_LOST)`); `Any` only in `perf_rx_rate_trial` (`hw_perf_rx_rates`) and `b8_lead_trial` (`hw_b8_leads`: the only `let _ = run.submit/advance_to` loop in `hardware.rs`; `grep -n 'let _ = run\.' hardware.rs` gives that line only). Both loop bodies moved into `common/mod.rs` so the FakeDevice can run them; `hardware.rs` calls them. `b8_lead_trial` prints one added line, `termination`.

**Run.rs.** `capture_markers(body) -> (T, Vec<String>)` (a thread-local: `judge` records its `EZSDR-INFRA` text there instead of writing it, while a test captures); `effects(judged, panicking) -> (marker, panic)` (pure); `Drop` while unwinding wraps `finish()` in `catch_unwind`. Why capture: a deliberate bound in a default-suite test leaves an `EZSDR-INFRA:` line in the output of a passing test (libtest does not capture the direct stderr write), which `mutate.py`/`gate.py` read as a bound in every row or step that selects the test (`verdict`: `INFRA in output`). The repository's convention is the same one `infra_fixtures` follows: no marker from the default suite.

**Point 1: can `finish()` panic?** Yes, but only through a broken Kernel invariant: `RunHandle::finish` is `check_lease()`, `ending::finish` and `self.manifest.take().expect("cleanup writes a Manifest")` (`coordinator/mod.rs:293-297`); `cleanup` has two `debug_assert!`s on the state machine (`ending.rs:31,43`); Module calls run under `contain` (KC-30) and `lock` tolerates a poisoned mutex (`state.rs:215-217`). The Kernel's own `Drop for RunHandle` (`mod.rs:315-320`) runs the same `ending::finish` for a device-paced Run while unwinding, so the wrapper adds no abort source for these Runs. `catch_unwind` in the wrapper's `Drop` covers the `expect` (the cleanup ran, so the inner `Drop` has nothing left to do); it cannot cover a `debug_assert!`, because the inner `Drop` would run the same cleanup again and panic while unwinding. Not testable without a Kernel change, so no test pins the `catch_unwind`. Test: `run_unwinding_marks_a_bound_and_never_panics_again` (a healthy live Run and a wedged one, each alive while the test panics with "boom"): the panic is "boom" in both, no abort, no marker for the healthy Run, exactly one for the wedged one.

**Point 2: #67.** Run on the tree with the declarations absent, the first run printed `rejected [("uhd-rx-s0", "did not join within 1 s; left detached")]` and nothing else for `ur_13_arm_cases`, `ur_13_arm_compares_the_rate_set_after_the_streamer_opens`, `ur_13_arm_takes_the_rate_set_in_any_order`, `ur_13_a_transmit_stream_is_compared_with_the_whole_device_grid` (`impl2` run, `/tmp/opencode/n51/fake1.log`); the fifth (`the_candidate_ceiling_…`) I declared before the first run, and ran once more without the declaration in a copy: the same single `uhd-rx-s0` note. All five are `uhd-rx-s0`. Nothing else failed on the join. The first run also failed 11 Policy and 3 subject tests (14), so 18 failures in all, exactly the plan's 22 less the 3 refusals and the declared fifth.

**Point 3.** No test's expectation changed except by the declarations above.

**New default-suite tests (14; `fake` 442 → 456).** All marker-free. Method tests keep the Run outside the closure and catch the call's panic (M2): `run_a_policy_end_is_judged_by_{submit,advance_to,wait_for,run_until_end}` (the end is a uhd-control panic, `DEVICE_LOST`; `submit` uses `PanicsInBook`, because only a Stop that waits on uhd-control reaches a Policy end without moving the clock) assert the call panicked with `ended unexpectedly`, no marker, and that the Run still hands out its Manifest after. Bound at a method return: `run_a_bound_is_marked_by_{advance_to,wait_for,run_until_end}` (real clock, transmit thread blocked by `TxBlocks`, uhd-control armed after 300 ms as `wedged()` does: the declared Policy end, and the Provider's join is the bound): one captured marker naming the join. `run_a_declared_end_answers_after_the_end` (CLOCK_LOST; every method answers `Ended` with that termination, `state`, `events(0)` = the Manifest's, `events(1)` empty, `finish` = the Manifest, one `close_streams`). `run_a_dropped_live_run_is_judged` (Gated, real clock). `run_unwinding_…` (above). `run_effects_…` (pure, in `run.rs`). `Any`: `run_any_a_perf_trial_reports_a_lost_device_and_the_next_trial_goes_on`, `run_any_a_lead_trial_…` (a Lost Fake: the line reports the termination, and the next healthy trial reports `Client`), `run_any_a_perf_trial_still_fails_on_a_bound` (a held Gated: one captured marker, a `met join` panic). Timing: the three bound tests, the dropped Run, the unwinding test and the held perf trial, plus the five Policy tests, ran 20 times in a row (`/tmp/opencode/n51/rep-*.log`): 20/20, no marker line in any.

**Rows (maintenance, `N51-2A-08…21`).** 08–11 `check` removed from `submit`/`advance_to`/`wait_for`/`run_until_end` (killers `run_a_policy_end_is_judged_by_*`: `expect_err` of the call that returned the end); 12 an ended Run answers another termination (`run_a_declared_end_answers_after_the_end`: the `Ended` termination equals the Manifest's); 13 an ended Run's `events` ignores `from` (same test: `events(1)` empty); 14 `Drop` does not judge (`run_a_dropped_live_run_is_judged`: no panic); 15 `Drop` does not finish while unwinding (`run_unwinding_…`: 0 markers for the wedged Run); 16 `effects` panics while unwinding (`run_effects_…`: `(true, false)` expected); 17 `mark(&message)` removed (`run_a_bound_is_marked_by_advance_to`: `markers.len()` 0); 18/19 `Any` removed from `b8_lead_trial`/`perf_rx_rate_trial` (the Lost trial panics `ended unexpectedly`); 20 `start_spec` without `Refused` (`ur_15_start_cases` panics); 21 N92-1's mutant on the submit-waits test, `"kill": "bound"` (below). Full-list run and the failing assertion of each: not observed by me (the runner prints the outcome only; the assertions above are derived from test and mutant), except for the rows I reproduced in a copy (below).

**Full lists** (`impl2-mut.log`, 2347 rows, 22:13–22:40): 2334 passed; not passed: `phase7/G09` (the known survivor) and 12 `ERROR … infra marker`: J44, N92-1, P292, P293, P334, P344, P346, P348, P880, R15, U03, U10. Cause, reproduced one by one in a copy outside the repository (`/tmp/opencode/n51/trymut.sh`, never the tree): the mutant makes a Run start that the test expects refused, or hang a thread, and the test fails by its assertion with a **live** Run, which the wrapper's `Drop` finishes during the unwind (M3) and meets #67 (a receive segment finished before T0) or another bound: a marker next to a failure the assertion explains. Settled:
- P292, P293, P334, P344, P346, P348, U10: the Run is the one `spec_run_ended` found not refused; its failure path now tears that Run down with `rx_join_issue_67()` before the panic (a path no green run reaches). They are `killed` with no marker.
- J44, U03: `ur_06_…` used a raw `Run::start_spec` and its own assertion; it now calls `spec_run_ended`, which also removes a raw site. `killed`.
- N92-1: its filter selected the test whose only kill under the mutant is KC-21a (a `submit` that waits 5 s on a uhd-control that never reports); it now names `ur_16_a_uhd_control_panic_ends_the_run_with_a_lost_device` (`killed`, no marker), and the submit test keeps its coverage as **N51-2A-21**, the same mutant, `"kill": "bound"`: `killed (bound)`, the marker is `a submit met KC-21a`.
- R15 (phase7), `"kill": "bound"`: without the stream's end the receive owner never stops, so every Run of the test fails its Provider join (1 s) before the plan assertion: `killed (bound)`.
- P880, `"kill": "bound"`: the mutant makes the measurement code report a started Run as refused, and the code drops it before T0, so the teardown meets #67. The two loops that drop it (`measurement.rs:2801,3036`) and `run_stage` could each tear it down with `rx_join_issue_67()`, three more declarations for one mutant; I chose the bound declaration, which goes away with #67. **Orchestrator to decide.**
- J317, J318: `"kill": "bound"` as the plan said (both ended by the join marker, `killed (bound)` as predicted).

Re-run of the changed rows (`spec_run_ended` and `ur_06_…` change the tests of 14 rows, plus the 4 above and the new 21): `impl2-mut2.log`, **18/18 passed** (`R15`, `P880`, `N51-2A-21` `killed (bound)`; the rest `killed`). The tests (`cargo test --workspace`: green, `fake` 456), `clippy -D warnings`, `cargo fmt --all --check`, `rules.py --base e449397` (0 problems) were re-run after these edits.

**Deviations and findings.**
1. **`run_stage` (`multichannel.rs`) declares `Refused`, as the plan's site 4 says, but the code beside it records an early Policy end** (`ended_early`/`run_ended_early`: "a device lost, say, still has its manifest to judge"). With only `Refused`, a Policy end in a throughput stage on the bench now panics `ended unexpectedly` at `advance_root` instead of being recorded. Not exercised by any default test. It is a change of a bench behaviour the plan did not name; a one-token change (`Expect::Any`, an observation-only loop that reports the termination) restores it. **Orchestrator/owner to decide**; I kept the plan's declaration.
2. Method tests use a uhd-control panic, not `FakeFault::Lost`, for the Policy end (the plan's text), because `submit` cannot reach a Policy end on a manual clock otherwise; all four are marker-free.
3. `capture_markers`, `effects` and the `catch_unwind` guard in `Drop` are additions to `run.rs` beyond sub-step 1's list; `rx_join_issue_67()` on the failure path of `spec_run_ended` is a declaration the plan did not name (above).
4. The `hardware` binary: not built with libuhd (none here). A copy with `#![cfg(feature = "uhd")]` removed and `UhdDevice`/`uhd_kept_count` stubbed type-checks except for 29 errors, all `tune_auto`/`tune_dsp` on the stub `FakeDevice` (`E0599`), none about `Run`, `Declared`, `Expect`, the trials or the helpers; the gate's `hw-build` is the real check. The B9/B8/perf declarations are therefore compile-checked only, as the plan said.

## Changes in sub-steps 3 and 4 (implemented 2026-11-12; no commit)

**Sub-step 3 — `crates/ezsdr-acceptance/tests/uhd.rs`** (only this file). `#[path = "../../ezsdr-radio-uhd/tests/common/run.rs"] mod run;` and `use watchdog::infra::{creation_limit, infra};` (the two names `run.rs` takes from its root); `use run::{Declared, Expect, Run}`; `RunHandle` and the raw `coordinator::` import are gone from the file. Sites 9–11: `uhd_spec_run` → `Run::start_spec`, `uhd_session_asking` → `Run::connect` (both undeclared), and `session_with_layout` → `Run::connect` with a new last parameter `declared: Declared`; `session_on` takes it too and passes it on (its 9 callers: the refusals pass `refused()`, a new 3-line helper returning `Declared { end: Some(Expect::Refused), .. }`; the others `Declared::default()`). The three refusal tests are `uhd_59_a_value_outside_the_rf_envelope…` (first `session_on`), `uhd_59_a_refused_initial_request_configures_nothing`, and the `refused` closure of `uhd_59_the_rf_envelope_judges_an_assigned_transmit_channel_at_validate`. `effective`/`state` are already methods of `Run` (sub-step 1). The Mock Run of `uhd_59_one_spec_mock_and_uhd_profiles` still goes through `support::spec_run`/`finish_at` (no FakeDevice, out of scope). No expectation changed. As the plan noted, the included `run.rs` brings its 10 pure tests into the `uhd` binary (19 → 29 tests). `cargo test -p ezsdr-acceptance --test uhd`: 29 passed (the 16 Run tests pass undeclared except the three refusals).

**Sub-step 4 — `crates/ezsdr-server/tests/protocol.rs`** (only this file). `fn handle(server: &mut Server, request, body) -> Handled` calls `server.handle` and then `rig_delay("server")` on the way out (after the reply is back, as the plan's M-section says). All 82 `.handle(` sites go through it (`Server::new(..).handle(..)` → `handle(&mut Server::new(..), ..)`; where the variable is already a `&mut Server`, `handle(server, ..)`, 12 sites in 11 helper fns); the only `.handle(` left in the file is inside `fn handle`. `rig_delay(point)` reads `EZSDR_RIG_DELAY_MS` and `EZSDR_RIG_DELAY_AT` once into a `OnceLock` and sleeps when the pure `rig_delay_for(ms, at, point)` returns a delay: unset ms → none; `at` is a comma list (blanks trimmed) that names the point or `all`; a non-numeric ms panics (a typo must not look like "no delay"). Tests: `rig_delay_names_its_points_and_reads_nothing_from_the_environment` (8 cells: `server` selects `server`, not `now`/`advance`; `all`; comma list with blanks; unset ms; empty list) and `rig_delay_refuses_a_milliseconds_value_that_is_not_a_number` (`should_panic`). Both pure, run once. Checked by hand: `EZSDR_RIG_DELAY_MS=300 EZSDR_RIG_DELAY_AT=server` makes `ea_10_a_failed_connect_writes_its_manifest` take 0.61 s instead of 0.01 s, and `AT=now` leaves it at 0.01 s. `protocol`: 79 passed (77 + 2).

**Rows** (`maintenance`, `plan/maintenance/tools/mutations.json`): N51-2A-22 (`name == point || name == "all"` → `name == point`: the `all` cell expects `Some(20 ms)`, gets `None`), N51-2A-23 (→ `true`: the `now,advance` cell expects `None`, gets `Some`), N51-2A-24 (the unset-ms early return removed: the unset cell panics parsing `""`, so the test fails by that panic), N51-2A-25 (`refused()` declares nothing: `uhd_59_a_refused_initial_request_configures_nothing` panics `ended unexpectedly` at `Run::connect`). As before, the failing assertion is derived from test and mutant, not observed. No row pins that the injection comes **after** `server.handle` returns: its order is observable only through timing, so it is not tested (reported to the orchestrator).

**Gates run:** `cargo fmt --all` (clean), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test --workspace` (all ok), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` (0 problems, every row anchors once).

**Full lists** (`impl34-mut.log`, 2352 rows, 23:08–23:23): 2351 passed; the only row not passed is the known `phase7/G09`. No `ERROR`, no row needed settling, no re-run (`impl34-mut2.log` not made). N51-2A-22…25 all `killed`.

## Changes in sub-step 5 (implemented 2026-11-12; no commit)

Files: `crates/ezsdr-radio-uhd/tests/infra_fixtures/mod.rs` (the fixtures, `Stalls`), `plan/maintenance/tools/mutate.py` (five `E2E` cases, `lacks`, the self-test cells). No production code, no `mutations.json` change.

**`Stalls`** (test-local, in `infra_fixtures/mod.rs`; a forwarding `Device` over a `FakeDevice` like the eight in `fake.rs`): `ref_locked` (read by uhd-control every 500 ms) sleeps 8 s once its flag is armed, `close_streams` sleeps 7 s once its flag is armed. Each stalled call first sends its name on a channel (`entered`), which the test reads. **Handshake (Sol's minor):** `await_entry(&entries, "ref_locked")` does `recv_timeout(5 s)`; a miss writes `EZSDR-INFRA: fixture setup: ref_locked was not entered within 5 s` and panics. No `wait(700 ms)` remains. Fixtures 6 and 7 stall inside the very call under test (`finish`, `run_until_end`), so they need no handshake and drop the receiver; if the stall were not reached, the case's `has` pattern would not match. `setup(what, step)` (fixture 8): a panic in the setup closure writes `EZSDR-INFRA: fixture setup: … did not complete` and resumes the panic, so a setup that did not reach the state the fixture needs is a marked setup failure, not a sentinel assertion that reads as a kill.

**Fixtures** (each returns at once unless `EZSDR_INFRA_FIXTURE` names it; the default suite counts them as no-ops, `fake` 456 → 461):

| fixture | sequence | observed marker (V, by hand, 20/20, `EZSDR_INFRA_FIXTURE=<name> /tmp/…/fake infra_fixture_<name>`) |
|---|---|---|
| 4 `submit_limit` | `fake_real` in `Stalls`, external clock, `session`; arm `ref_locked`; await entry; `submit(set gain)`; `panic!("FIXTURE-4-AFTER …")` | `a submit met KC-21a`, 1 marker, 1 `panicked at`, no `FIXTURE-4-AFTER`; 6.6 s |
| 4b `agenda_limit` | Spec with `schedule: [stop at offset 50 000]`, external clock, `Stalls`; arm; await entry; `advance_to(+3 s)`; `panic!("FIXTURE-4B-AFTER …")` | `an advance met KC-21a`, 1/1, no sentinel; 8.2 s |
| 6 `cleanup_limit` | manual-clock `Stalls`; `session`; `past_t0`; arm `close`; `finish()`; `section(.., "FIXTURE-6-AFTER-bursts")` | `the Run met RS-8a` (inside `finish`), 1/1, no `no section`; 7.7 s |
| 7 `declared_end_cleanup_limit` | manual-clock `Stalls` with `Unlocked(2.3 s)`, external clock; `.expecting(Policy("radio.CLOCK_LOST"))`; arm `close`; `run_until_end(+5 s)`; `finish`; `section(..)` | `a wait met RS-8a`, 1/1, no `no section`; 8.4 s |
| 8 `join_at_finish` | `Gated` (real clock), `held_session` (both Provider threads held in a device call) inside `setup`; undeclared; `finish()` | `the Run met join` (`uhd-tx-s0`, `uhd-rx-s0`), 1/1, no `unreachable!`; 4.1 s |

Each run printed 1 marker and 1 panic and none of the sentinel strings: 5 × 20 runs, 100/100 (`/tmp/opencode/n51-5-rep.log`; the binary was built before `assert!(false, …)` became `panic!(…)` for clippy's `assertions_on_constants`; the e2e run below used the final source). The fixtures are the failing-first evidence's other half: before the wrapper each of 4, 4b, 6, 7 gave `FAILED` with no marker (plan §6, V in `n51-2a-base`), i.e. a false `killed`.

**`mutate.py`.** `E2E` has five more cases (`4-submit-limit`, `4b-agenda-limit`, `6-cleanup-limit`, `7-declared-end-cleanup-limit`, `8-join-at-finish`; 9 → 14): `expect: ERROR`, `markers: 1`, `panics: 1`, `has` = the marker line naming the call and the bound plus `test result: FAILED`, `lacks` = the sentinel. `e2e_case`'s checks moved into a pure `e2e_seen(case, ran, outcome, reruns)` that also reads `lacks` (a pattern that matches fails the case). `--self-test` gains three cells on strings: `lacks` absent passes; `lacks` present fails; `has`, the outcome and the counts still decide. The existing self-test cell "every case names a fixture" covers the new names. `gate.py` derives the e2e count from `E2E`, so nothing else changed.

**No mutation row for `lacks`.** A row runs a cargo test or a Python `unittest`; `mutate.py` is not under one (no row targets it today). Its protection is `mutate.py --self-test`, which `gate.py` and the e2e step run. Checked by hand in a copy outside the repository: removing `and not seen["lacks"]` from `e2e_seen` fails the self-test cell "an e2e case fails when a `lacks` pattern is present". The fixtures themselves have no row either: the e2e step is their check.

**e2e** (`remote_mutate.py --e2e sim02:8` through `simq`, `impl5-e2e.log`): **`e2e: 14/14 ok; failed: []; self-test: ok`**; the five new cases `ok` with `markers 1, panics 1, lacks []`.

**Gates run:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test --workspace` (all ok; `fake` 461), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` (0 problems), `mutate.py --self-test` ok.

**Full lists** (`impl5-mut.log`, 2352 rows, 23:51–00:13): 2351 passed; the only row not passed is the known `phase7/G09`. No `ERROR`, no row needed settling, no re-run (`impl5-mut2.log` not made). The fixtures and `mutate.py` are read by no row's test, so the run only confirms that the changes broke none.

## Changes in sub-steps 6 and 7 (implemented 2026-11-13; no commit)

Sub-step 6 files: `crates/ezsdr-acceptance/tests/governance.rs`, `plan/maintenance/tools/mutations.json` (rows `N51-2A-26…39`). Sub-step 7 files: `crates/ezsdr-radio-uhd/tests/common/{infra,run,mod}.rs`, `crates/ezsdr-acceptance/tests/uhd.rs`, `crates/ezsdr-server/tests/protocol.rs`, `mutations.json` (rows 22–24 retargeted to `infra.rs`); outside the repository: `sweep.py`, `sweep/` (raw logs, `results.json`, `report.md`). No production code.

### Sub-step 6 — the scan (`governance.rs`, +410 lines)

`scan(path, source) -> Vec<String>` (path relative to the workspace, `/`-separated; `scan_used` also reports the allow-list rows it relied on). Comments, string, raw-string and char literals are blanked first (`blank`), then the source is cut into tokens (`tokens`; `::` is two `:` tokens), so whitespace and line breaks inside a path or before `(` change nothing. The enclosing `fn` is tracked by brace depth (`Frame`); a fn is a test when `#[test]` is among the attributes before it; a helper nested in a test is a helper.

Rules and scope:
- **Raw Run** (every `.rs` under `radio-uhd/tests/` — `hardware.rs`, `infra_fixtures/`, `common/**` included — except `common/run.rs`; every `.rs` under `ezsdr-acceptance/tests/` except `{v58, v61, reactive, parity, measurements, x3xx, o1_oracle, repeat_switch, governance}.rs`, `support/`, `oracle/`): the identifier `RunHandle` (`RunHandleError` is another token); `connect(` / `start_spec_run(`, whatever precedes them (`coordinator::connect(`, a line break before `(`), unless preceded by `Run::` or `fn`; a `use … connect as X` / `start_spec_run as X` makes `X(` a raw call from there on (Sol's minor: renamed and multi-line forms). `disconnect(` is another identifier.
- **`Expect::Any`** (same scope): only inside `perf_rx_rate_trial` and `b8_lead_trial` (`common/mod.rs`) and `run_stage` (`common/measurement/multichannel.rs`) — the orchestrator's decision; `run.rs` defines it. Matched by `(file, fn)`.
- **Time read by a `#[test]` fn** (`fake.rs`, `common/**`, acceptance `uhd.rs`; not `hardware.rs`): `thread::sleep`, `Instant::now`, `.elapsed(`, `recv_timeout(`, `.time_now(`, `.host(`. A helper fn is not scanned (note 51 §5.4). The per-fn allow list `TIME_ALLOWED` has **24** rows `(file, fn, reason)`, the same number the plan found by script.
- **`.handle(`** in `protocol.rs`: only inside `fn handle`.

**Stale allowances fail:** the real-tree test `run_scan_the_tree_is_clean_and_every_allowance_is_used` also requires every `TIME_ALLOWED` and `ANY_ALLOWED` row to have been used.

Tests (pure, run once; `cargo test -p ezsdr-acceptance --test governance`: 12 passed, 5 old + 7 new): `run_scan_the_tree_is_clean_and_every_allowance_is_used` (the three crates' real files), `run_scan_finds_a_raw_run_creation_however_it_is_written` (11 cells: raw, `coordinator::`, `start_spec_run`, `connect (`, `connect\n(`, `coordinator::\nstart_spec_run(`, a renamed import and one inside a group, `RunHandle`, two on one line, the line number and fn in the message), `run_scan_allows_the_wrapper_and_what_only_looks_like_a_raw_call` (`Run::connect(` / `Run::start_spec(`, spaced `Run :: connect (`, `disconnect(`, `RunHandleError`, `fn connect(`, comments and strings and a raw string, `run.rs` itself), `run_scan_reads_time_only_in_test_fns_and_lets_the_listed_ones_pass` (each of the six sources in a test and in a helper; a closure in a test; another attribute between; a nested helper; a helper after a test; a `;` in the header; `hardware.rs` skipped; `uhd.rs` and `common/` scanned; `v58.rs` outside; a listed fn passes, another fn of that file and the same name in another file do not), `run_scan_covers_the_files_it_names_and_skips_the_others`, `run_scan_allows_expect_any_in_its_named_loops_only`, `run_scan_the_diagnostic_door_is_the_only_handle_call`.

**Finding — the plan's allow list description was wrong in one respect.** §5 said none of the 24 fns is Run-based. Three are: `ur_29_a_silent_stream_is_a_lost_device` (device time measured with `host()`, a 30 s wall watchdog; plan 30 §4.4), `ur_21_a_descheduled_timed_send_is_on_time_on_the_manual_clock_and_late_on_the_real_one` (a 5 s `recv_timeout` handshake on the descheduled uhd-tx) and `ur_33_a_shutdown_with_a_report_read_that_fails_at_once_still_joins_uhd_tx` (`Instant`/`.elapsed()` for a 10 s bound on `finish`). They are listed with that as their reason (the other 21 are Authority, `Direct`-rig and device tests with no Run). Whether they should be rewritten to a rig or an observed handshake is the orchestrator's call; not done here.

**Rows** (`maintenance`, `crate ezsdr-acceptance`, `test_bin governance`; the failing assertion is derived from the test and the mutant — a mutant is never applied to a tree here):

| row | mutant | killer · what fails |
|---|---|---|
| N51-2A-26 | `connect` dropped from the raw names | `…finds_a_raw_run_creation…`: the first cell expects 1 hit, gets 0 |
| 27 | alias after `as` not recorded | same test, the renamed-import cells get 0 |
| 28 | `Run::` prefix not recognised | `…allows_the_wrapper…`: `Run::connect(` gets 2 hits |
| 29 | `RunHandleError` counts as `RunHandle` | same test, the error-type cell gets 2 |
| 30 | `run.rs` scanned like the others | same test, the last cell gets 3 |
| 31 | comments read as code | same test, the comment/string cell gets hits |
| 32 | `#[test]` not recognised | `…reads_time_only_in_test_fns…`: the first test-fn cell gets 0 |
| 33 | every fn is a test | same test, the helper cell gets 1 |
| 34 | `;` anywhere ends a fn header | same test, the `[u8; 4]` cell gets 0 |
| 35 | time allow-list by file only | same test, "another fn of that file" gets 0 |
| 36 | time rule skips `common/**` | same test, the `COMMON` cell gets 0 |
| 37 | acceptance file-level exempt ignored | `…covers_the_files_it_names…`: `v58.rs` gets 1 |
| 38 | `Expect::Any` allow-list by file only | `…expect_any…`: "another fn of that file" gets 0 |
| 39 | `.handle(` allowed in any fn | `…diagnostic_door…`: `fn t` gets 0 |

All 14 `killed` in the full run (below). The stale-allowance check has no row: a mutant that skipped it would be caught only by a stale row, which the tree does not have.

### Sub-step 7 — the delay injection

**Where.** `rig_delay(point)` and its pure `rig_delay_for(ms, at, point)` now live in `common/infra.rs`, the file `watchdog.rs` already shares by `#[path]` with `ezsdr-acceptance`'s `uhd.rs` and `ezsdr-server`'s `protocol.rs`. That is a **deviation**: sub-step 4 had given `protocol.rs` its own copy, and the plan had accepted a second copy of the 6-line reader because `protocol.rs` cannot include `common`. It can reach `infra.rs` through `watchdog::infra`, so the copy is gone (a rule implemented once; AGENTS §9 maintainability 1). Changed with it: `protocol.rs` calls `watchdog::infra::rig_delay("server")` in its `handle` and drops its copy and its two tests (the tests moved with the function: `rig_delay_names_its_points_and_reads_nothing_from_the_environment`, now also with the `now`/`advance`/`submit` cells, and `rig_delay_refuses_a_milliseconds_value_that_is_not_a_number`); `common/mod.rs` re-exports `rig_delay` beside `infra`; `uhd.rs` imports it for the included `run.rs`; rows N51-2A-22…24 only changed `file` to `infra.rs` (same anchors, same `ezsdr-server`/`protocol` test: the tests run there through the include).

**Semantics.** `EZSDR_RIG_DELAY_MS` unset or empty: nothing. Set: a non-number panics (a typo must not read as "no delay"); `EZSDR_RIG_DELAY_AT` is a comma list (blanks trimmed) of `now`, `advance`, `submit`, `server` or `all`, and the calling thread sleeps after the call returns: `Run::now` and `Run::start_instant` (`now`), `submit` (`submit`), `advance_to`/`wait_for`/`run_until_end` (`advance`) — each after the wrapper's `check`, so a panic from `check` is not delayed. `finish`, `Drop`, `state`, `events` have no point. Both variables are read once into a `OnceLock`. It is runtime, not a `cfg`: the acceptance crate has no `uhd` feature. Not set on any gate or bench run (nothing in the repository sets it). The calls inside `run.rs` are not unit-tested (they read the environment through the `OnceLock`, so a test cannot set it); the parser is pinned by the pure tests and rows 22–24. Pure tests run once.

### The sweep (`sweep.py`, `sweep/report.md`, `results.json`, raw logs in `sweep/<binary>/`)

`sweep.py` runs three prebuilt binaries (`fake`, `uhd`, `protocol`, built from this tree and copied to `/tmp/opencode/sweep-bins`) with `--test-threads=8`, in the crate's directory, once per config: a baseline and `{now, advance, submit} × {5, 20, 60, 200} ms` (`protocol`: `server` only), `EZSDR_TEST_WATCHDOG_SECS=1800` so a slow test is slow, not an abort. A binary is killed by PID at its timeout; none timed out, none aborted, no `EZSDR-INFRA` line in any of the 31 runs. **One run per config**, so a count is one sample.

Baseline: `fake` 463 tests, 0 failed (143 s); `uhd` 31, 0 failed; `protocol` 79, 0 failed. Failures per config (the failing sets grow monotonically with the delay at each point):

| point | 5 ms | 20 ms | 60 ms | 200 ms |
|---|---|---|---|---|
| `fake` `now` | 3 | 7 | 12 | 12 |
| `fake` `advance` | 1 | 2 | 2 | 2 |
| `fake` `submit` | 2 | 4 | 6 | 8 |
| `uhd` `now` / `advance` / `submit` | 0/0/1 | 0/0/1 | 0/0/1 | 1/0/1 |
| `protocol` `server` | 0 | 0 | 0 | 0 |

Against note 51 §5.2's table (probe 6, `fake` 398 tests, 3 points × 4 delays): `now` 2/7/10/10 there, `advance_to` 0/2/2/2, `submit` —/4/5/—; `protocol` 0 throughout; the two `submit`-only tests there (`rx_start_lead_group_runs_on_the_fake`, `uhd.rs`'s `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end`) are the two here. The sets are the old ones plus two at `now` (`p70_one_sample_burst_then_timed_burst_rehearsal_on_the_fake`, `s1_short_first_sends_rehearsal_on_the_fake`, both rehearsals on the n47s5 shape; whether they existed at probe 6 is not checked: the binary has grown from 398 tests).

Tests whose verdict changes (14 in `fake`, 1 in `uhd`; first delay at which each fails): `n47s5_one_sample_after_a_burst_refusal_rehearsal_on_the_fake` (`advance` 20, `now` 5), `ur_21_a_refused_configure_refuses_its_burst_through_the_kernel` (all three points from 5 ms), `n47s5_front_rehearsal_on_the_fake` (`now` 5, `submit` 5), `n47s5_held_tail_and_stop_rehearsal_on_the_fake` (`now` 20, `submit` 20), `n47s5_long_close_rehearsal_on_the_fake` (`now` 60, `submit` 60), `n47s5_repeat_switch_rehearsal_on_the_fake` (`now` 20), `n47s5_untimed_rehearsal_on_the_fake` (`now` 60), `ur_21_s18_an_untimed_repeat_on_a_playing_repeat_is_refused_without_time_error` (`now` 20), `ur_23_a_burst_after_a_stop_targets_the_one_clock` (`now` 20), `ur_29_a_loss_found_by_the_shutdown_s_report_read_ends_its_ack_window` (`now` 60), `p70_…`, `s1_…` (`now`/`submit` 60), `common::measurement::tests::rx_start_lead_group_runs_on_the_fake` (`submit` 20), and `uhd.rs`'s `ea_16_untimed_repeat_after_stop_starts_at_the_stop_region_end` (`submit` 5, `now` 200). Every one fails by its own assertion (the first panic text per test is in `report.md`), none by a marker. **Findings, not fixed here:** these are the tests whose verdict depends on a delay between a call's return and the next call: the stage-2a wrapper detects none of them, because the wrapper judges how a Run ended, not when the test thread acts; they are the "R1/R2" class of note 51 §5.2 (`plan/phase8/51-test-timing.md`), whose handling (a direct rig, or a window that does not depend on the delay) that note leaves to a later stage. The 14 are known to the note except the two named above.

### Gates and full lists

`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo test --workspace` (74 test binaries, 2564 passed, 0 failed, run before the last edit; `governance` re-run after it: 12 passed), `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base e449397` (0 problems, every new row anchors once).

**Full lists** (`impl67-mut.log`, 2366 rows, 00:27–00:45; the other lane held the sim lock from 00:18 to 00:27): **2365 passed**; the one not passed is the known `phase7/G09`. No row is `ERROR`; `J58`, `J156`, `G03` print `equivalent (survived)` and are in `known-survivors.txt` as before; N51-2A-01…39 all `killed` except N51-2A-21 `killed (bound)`. Nothing needed settling, so there is no `impl67-mut2.log`.

### Deviations and decisions for the orchestrator

1. `rig_delay` lives in `infra.rs`, not as a second copy in `run.rs` (above). Sub-step 4's `protocol.rs` text changed (its reader and 2 tests are gone, its call goes through `watchdog::infra`).
2. The scan is 410 lines, not ~140: the lexer (`blank`, `tokens`, ~100), the 24-row allow list with reasons, and 7 tests.
3. Three of the 24 time-reading `#[test]` fns drive a Run (above); listed, not rewritten.
4. The `Run` methods' delay calls have no test of their own (the environment is read once per process).
5. No test's expectation changed. Nothing needs a decision beyond (3).
