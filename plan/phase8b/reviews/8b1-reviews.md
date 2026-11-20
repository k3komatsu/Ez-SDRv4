# 8b-1 — reviews

Test plan: Sol (v1), then the one re-check by Terra and Opus side by side (v2). Code: Sol, then a re-check of the fix by Terra and Opus.

## Sol — test plan v1

**FAIL.** The current per-thread bound would fail S1 and S5’s elapsed-time assertions, but S4 does not reliably kill it. Several planned tests can also fail on correct code under load.

### Major

1. **The timing margins allow false failures.** In S1, hold all TX workers and deschedule the stop thread for more than a second near expiry. Correct code can return after the `< 2 s` ceiling. S5, S7b, and S8 have the same problem. In S3, delay the healthy worker after Shutdown and it can be detached on correct code. Delay the joiner instead and a carried deadline can still join that worker. S2, S6, S9–S11 also expect workers to finish inside a real deadline, despite being labeled timing-independent. Use a controllable monotonic deadline and handshakes for the budget and completion verdicts. Keep the frozen-manual-clock test for the real-time wait, with its infrastructure timeout. Twenty repeats are useful validation, but cannot establish scheduler guarantees. Test plan §§2–5 (`test-plan.md:28`) · Timing rules §3 (`plan/phase8/51-test-timing.md:92`)

2. **S2 and S6 observe receipt, while the contract is about enqueue.** Queue RX Shutdown while TX is still held, then schedule RX only after the TX detach note. RX reports `notes == 1` and S6 passes the early-send mutant. With correct code, TX1 can receive its already-queued Shutdown after TX0’s note and fail S2. Gate the workers and inspect the actual command queues at the join-stage boundary. Also check that the control plan row exists before the TX sends. The current row counts only establish that it exists when stop returns. Test plan S2 and S6 (`test-plan.md:36`) · Provider stop (`crates/ezsdr-radio-uhd/src/provider/mod.rs:749`)

3. **S4 races the joiner, and D1 promises a decision at an instant it cannot observe.** Hold TX0 and TX1 through expiry, publish TX0’s note, then let a per-handle recheck reach TX1 before the test releases it. Both workers are recorded detached and R-S4snap survives. Old code can also pass if TX1 misses its fresh second. Use one completion sweep after the poll observes expiry and capture every result before publishing any note. Specify that sweep as the decision point. In S4, pause the joiner after the first note, release TX1, verify its handle has finished, then resume. That distinguishes the captured decision from a later recheck. Test plan D1 and S4 (`test-plan.md:7`) · Current join (`crates/ezsdr-radio-uhd/src/provider/mod.rs:419`)

### Minor

- **Tighten the mutation rows.** R-S1 and R-S4tx are the same mutation. Keep one once its killer is deterministic. R-S2 should change send ordering while preserving the shared deadline. N51A-15 also has an `old2` anchor, which D4 does not preserve explicitly. P277 removes the detach note and provides no coverage for unwrapping a panicked join result. S10’s worker panic is caught by `spawn`, so scope the test to the panic record, loss, and peer outcomes. Use `kill: bound` for intentional infrastructure expiry and retain the required declarations on real-clock FakeDevice filters. Use ordinary assertion kills for deterministic decisions. Test plan §4 (`test-plan.md:78`) · Existing N51A-15 row (`plan/maintenance/tools/mutations.json:17804`) · Spawn wrapper (`crates/ezsdr-radio-uhd/src/provider/mod.rs:394`)
- **Spec edits need one more pass.** Put the shared one-second wait and completion sweep in UR-16. In UR-26, say that **all** Shutdowns for a direction are enqueued before waiting on that direction, and cite UR-16 for the bound. In KC-46b, describe about three seconds of join waiting, with polling and scheduling overhead. RS-8a’s five seconds is the Kernel’s separate step deadline. Update the indented `*Test:*` lines to match what the tests observe and add a *Changes* row in both edited specs. UR-16 (`design/18-uhd-radio.md:223`) · UR-26 (`design/18-uhd-radio.md:352`) · KC-46b (`design/06-kernel-coordinator.md:445`)

### Notes on the open questions

- **Snapshot:** Use the completion sweep described above.
- **Three streams:** Acceptable in `prepared_rig`, which constructs stream slots directly. Two per direction suffice once the deadline test is deterministic.
- **S9:** Keep the unit rig and existing start-failure tests. Use a reachable partial spawn, such as RX0 present and RX1, TX, and control absent. Assert zero wait calls for an empty stage. The current set assertions cannot observe a wasted wait.
- **R-S3 `OnceLock`:** Acceptable as a mutant only with one stop per filtered process. Split the control→TX and TX→RX cases. The current two-run test reuses the first run’s expired deadline. Test plan §7 (`test-plan.md:124`)

Reviewed read-only. No files changed and no builds or tests ran.
## Terra — test plan v2 re-check

**FAIL.** The completion-sweep test addresses Sol’s third major, but the plan still has an ordering blind spot and a test that can fail on correct code.

**Major findings**

1. **The enqueue hooks do not establish the claimed ordering.** S6 (`test-plan.md:83`) observes queues only *after* all sends for a direction. Minimal counterexample: enqueue all TX shutdowns, invoke `Enqueued Tx`, enqueue RX shutdowns **before** the TX stage, then run the TX stage and invoke `Enqueued Rx`. Both observations pass although RX was sent too early. Likewise, recording the plan after the first TX send but before `Enqueued Tx` passes. Observe the plan at the first TX send and RX queues at the TX decision boundary, before RX sends can occur.

2. **S9 can fail on a correct scheduler.** S9 (`test-plan.md:98`) requires no `Polled` call, but RX0 may simply not have run after receiving Shutdown when the joiner first checks it. A correct joiner then polls until RX0 finishes. The stated `reads == 1` also contradicts its own “deadline plus expiry read” explanation. Assert that empty stages make no clock reads or waits using **per-stage observations**, and permit any number of RX polls; avoid deriving an expected count from the implementation.

**Minor findings and recommendations**

- The virtual clock exercises decision logic but does not itself prove the production clock and polling path match it. Keep S7b with the seam unset, inspect the actual production deadline and wait in code review, and use failing-first tests to validate the seam. The stop-thread hooks must never wait for work that itself needs the stop thread or a lock it holds; S7a’s frozen clock intentionally turns a broken idle wait into an infrastructure timeout. Seam (`test-plan.md:20`) · S7 (`test-plan.md:88`)
- Keep the seam as small as possible and make hook data and calls compile out of production; `Stage` must still exist wherever a production function signature uses it. Verify the compiled shape and anchor matches in the first code review rather than assuming empty calls have no cost. Open point 2 (`test-plan.md:145`)
- The split `OnceLock` mutants are reasonable **if each filter selects exactly one test in a fresh process**. Verify that during mutation execution; failing-first tests can check both mutants. Open point 3 (`test-plan.md:119`)
- UR-16’s proposed wording (`test-plan.md:135`) says the three-second wait is within the five-second deadline without accounting for scheduling and polling overhead. Preserve the distinction made in KC-46b.
- The P277 disposition keeps its existing coverage, but calling an unwrapped panicked join *equivalent in every reachable case* is stronger than the evidence: the catch handler itself may panic. Describe that path as untested without fault injection instead. S10 (`test-plan.md:101`)

Sol’s timing major is substantially addressed apart from S9; the enqueue major remains open as above. The sweep and S4 handshake close the third major. The other mutation and spec corrections are incorporated, subject to the qualifications above.

This is the plan’s one re-check. Correct the two major test designs before implementation; validate hook progress, real-clock wiring, mutation filters and production footprint through failing-first tests and the first code review. No files were changed or tests run.
## Opus — test plan v2 re-check

# 8b-1 test plan v2 — Opus re-check (read-only, 2026-11-20)

**Verdict: FAIL, narrowly.** v2 closes Sol's three majors in substance, but it has two new majors. Each has a one-sentence fix that can go into the phase-B brief without another plan re-check. Nothing was built or run. The code reads are against `/home/komatsu/local_work/Ez-SDRv4-n111` at e178a2e5.

## Own failure scenarios for the seam (written before the checklist)

1. **Can a test pass on wrong code?** In production `join_now()` is `Instant::now()`. Any mix of `join_now` and `Instant::now()` in the stage is therefore identical in production, and the seam cannot hide a defect there. A deadline re-read every round would hang S1 into the infra bound, so it still fails. A busy spin with no real 1 ms wait is invisible to the virtual tests, but N51A-13 and N51A-14 still pin the `host.wait` line. No hidden real-time defect found.
2. **Can a test fail on correct code?** A thread that should end by itself while the stage clock is frozen ends only if it needs nothing from the joiner. The `prepared_rig` core is a **manual-clock** FakeDevice (`test_support::rig_with` forces `ClockMode::Manual`, test_support.rs:30). That clock stops while any participant is busy and otherwise runs at real rate (host.rs:123-166). Stub workers that wait on std channels are fine. A real `Rx::run` that has gone to sleep is not: it sleeps 1 ms on the host clock (rx.rs:207), and that clock advances only while the joiner waits. This breaks S9 (Major A).
3. **Can a hook deadlock with a worker?** Hooks run on the joiner, which is a clock participant. A hook that spins on `finished()` of a thread that needs the manual clock deadlocks. No planned hook does this, but S9's rig is the case where it could. Phase B should state the rule: a hook waits only on stub workers.
4. **Can a hook spin forever under a mutant?** Yes. This is Major B.
5. **Production cost.** If the points are `#[cfg(test)]` statements, production gains only `join_now`. See open point 2.

## Major

**A. S9's "no `Polled`" depends on a race, and is lost in most schedules (§3 S9, §7.1).** S9 asserts "no `Polled`" and `reads == 1`. Only a stage that finds rx0 already finished at its first check can satisfy that. Nothing guarantees it, which contradicts §5's claim that "the deterministic tests depend on no scheduler outcome".
- Sequence: `start` spawns the real `uhd-rx-s0`. If its first turn comes before the `Shutdown(Abort)` is enqueued, it goes `Idle` and calls `host.sleep(1 ms)` on the inner manual clock (rx.rs:207).
- That clock stops while any participant is busy (host.rs:123-131, M3). The test thread is a participant: it holds `clock_guard` (mod.rs:1346) and is also the joiner. So rx0 cannot wake until the joiner reaches `host.wait`, which comes only after `Polled{Rx,0}`.
- Wrong outcome: the test fails on correct code. It passes only if rx0 receives its `Shutdown` at its very first `poll` and exits before the stage's first check. The plan's own count also disagrees with itself ("`reads == 1`" against "the `until` … plus the one expiry read").
- Fix (Sol's own suggestion, and less code): use the unit rig with only `uhd-rx-s0` in `running.threads`, finished before `stop` (`all_finished`). `to_tx` and `to_rx` each hold two real channels. Call `stop(StopMode::Abort)` directly.
  - Assert exactly one `Begun`, `{Rx, ["uhd-rx-s0"]}`, and no `Polled`.
  - In the `Begun{Rx}` hook, assert `reads == 1`. This is deterministic because the check short-circuits on "all finished", so it reads the clock only for `until`.
  - Assert that rx0 is joined and `detached` is empty.
  - Drop the `fail_next_rx_spawn` → `fail_rx_spawn_of` change. Coverage that the `start`-failure path joins an owner it already spawned stays in `ur_29_a_loss_published_just_before_a_later_stream_s_first_start_gate_makes_no_call`, which asserts that "stream 0's owner was joined by the refused start". Note that `ur_15_a_failed_rx_owner_spawn_…` spawns no thread.
- This closes open point 1.

**B. `finished(name)` is undefined for a name the stage does not hold, so the per-thread mutants become ERROR or survive under load (§0.1 `finished`, S2, S4, §4 R-PT-tx and R-PT-rx).**
- Under R-PT-rx, each per-name stage holds one handle.
  - S2rx's `Begun{Rx}` hook, in rx0's stage, spins on `finished("uhd-rx-s1")`.
  - S4rx's `Noted{Rx,"uhd-rx-s0"}` hook does the same.
- If `finished` returns false for a handle it does not hold, the spin lasts until the 30 s `bounded` limit writes `EZSDR-INFRA:`. mutate.py then reports the row as ERROR (mutate.py:193, no `kill: bound`).
- If it returns true, the S4 hook returns at once. rx1's own stage then races: rx1 must finish within 4 polls of about 1 ms real time each, or it is detached and S4 passes. The kill depends on load, and S2rx does not kill at all.
- The same applies to R-PT-tx. Its filter `a_transmit_stage_` also selects S2tx (`ur_16_a_transmit_stage_joins_…`), which §2 S1 and §4 overlook.
- Fix: `finished(name)` panics with an assertion on a name that the current stage was not given. Joined names stay "true". The hook panic fails the test without a marker, so the mutant is a plain kill. `UhdRadio` has no `Drop`, so there is no double panic. With this fix, S4's "old code fails" line holds deterministically.

## Sol's findings, as sol-dispositions.md asks

| Item | Closed? |
|---|---|
| Major 1 (wall margins) | Yes, through §0.1–0.3 and §5, except S9 (Major A). Virtual time advances only at `Polled` in stuck stages, so S1's 4 polls, S5's 12 and S8's 0 are exact. |
| Major 2 (enqueue, not receipt) | Yes. The test owns the receivers. `Enqueued` comes after all sends and before `Begun`. S6 checks the plan row at `Enqueued{Tx}`. |
| Major 3 (sweep) | Yes, specified in §0.3. S4 is deterministic once Major B is fixed. The `R-sweep` kill is sound: at `Noted{tx0}` the hook makes tx1 finished, so a re-read joins it. |
| Minor: merge R-S1 and R-S4tx | Yes (R-PT-tx). Fix the filter text: it selects S1, S2tx and S4tx. |
| Minor: R-S2 changes order only | Yes. |
| Minor: N51A-15 `old2` | Yes, explicit. |
| Minor: P277 and the panicked join result | Yes. P277 is re-anchored, and the unreachability is stated (the `spawn` wrapper catches every body panic, mod.rs:404). This is accepted. |
| Minor: S10 scope | Yes. |
| Minor: `kill` rule | Yes. The unit rig is manual-clock, so its baselines print no `EZSDR-CLASS: real-clock` and plain kills are valid. Existing P466 runs undeclared on the same rig. |
| Spec edits | Yes in substance (§6). The proposed UR-16 sentence is long for AGENTS §2's "a few lines". Settle it in the code review with `rules.py`. |

## Minor

1. **§0.1 `Begun` carries no `names`**, but S1 and S9 assert the names. Add `names: &[String]`.
2. **§7.2 contradicts itself.** It says `Stage`, `JoinPoint` and `JoinSeam` are test-only, yet production calls `join_point(JoinPoint{…})` and `join_stage(…, stage: Stage, …)`.
3. **R-S7a-wall duplicates N51A-13.** Both make the same mutation. N51A-13 already kills it with a plain assertion on the existing seam-unset test, so the new row only adds a 30 s infra expiry. Drop it.
   **R-S7a (`all` → `any`) on S7a depends on load.** The manual clock runs only while the joiner idles (host.rs:123-131). If the joiner returns from its 1 ms wait more than about 49 ms late, the 50 ms sleeper ends before the next check, the sweep finds both threads finished, and the mutant survives. Filter R-S7a on S3a instead: tx1 is finished and tx0 is held on a channel, so `any` exits before `Polled{Tx,0}` every time.
4. **S4 does not need the test thread.** The `Noted` hook can own tx1's sender and release tx1 itself, so `stop` can run on the test thread.
5. **S6's `event_log` order omits `Polled`.** Say that the log records only `Begun`, `Noted` and `Enqueued`.
6. **The `running_radio` doc comment (mod.rs:1722) says "the rig's clock is the real one"**, but the rig is manual. The step edits these helpers, so correct the comment.

## Open points: recommendations

1. **S9 read counts:** follow Major A. Use the unit rig with rx0 finished, and assert `reads == 1` at `Begun{Rx}`, not as a total after the stage.
2. **The seam's footprint:**
   - Production keeps one item, `fn join_now(&self) -> Instant`. With `cfg(test)` it reads `join_seam.now`. Without it, it is `Instant::now()`.
   - The four `join_point` calls become `#[cfg(test)]` statements, so `JoinPoint` and `JoinSeam` are truly test-only.
   - Drop `Stage` from the production signature: `join_stage(&mut self, core, names: &[String])`, with hooks reading the stage from `names[0]`'s prefix.
   - The mutation rows anchor on production lines and are unaffected. This is minimal under AGENTS §9 maintainability (no new production state, no anchor scaffolding).
3. **The `OnceLock` split:** accept. S3a and S3b each make one `stop` per filtered process. Their names do not contain `a_transmit_stage_` or `a_receive_stage_` (checked), so no other row's filter picks them up. Exact re-runs are separate processes. Phase B confirms with `--list` that each filter selects exactly one test.

## What can go to failing-first tests and the first code review

Majors A and B: write each fix into the phase-B brief as a sentence. A's correction shows up when the corrected S9 is written failing-first. B shows up when R-PT-tx and R-PT-rx are first run. All the minors, the UR-16 wording and the confirmation that every anchor still matches exactly once go to the first code review. No further plan re-check is needed.

## Not read, not run

- I read `Rx::begin`, the `fake.rs` tests and the KC and UR spec text only through the plan's citations.
- No `anchor.py` or `rules.py` run, no build, no tests.
- The host-clock semantics come from reading host.rs:123-166 and 233-247 (the manual clock stops while any participant is busy and otherwise runs at real rate).

## Sol — code

**FAIL.** The staged join logic matches D-6 in the reviewed code. The blocking finding is a gap in the tests for the expiry sweep.

**Major**

- **The tests miss a stale completion result at expiry.** Hold TX0 through the deadline. Let TX1 finish during the final poll, before advancing the test clock to expiry. A joiner that reuses the previous poll’s completion results would wrongly record TX1 as detached in the Manifest. The mixed-stage test releases TX1 before the first poll, and P1408 checks only for a recheck *after* the sweep. Move the mixed-stage release to the last poll, confirm TX1 has finished, then expire the stage and assert that only TX0 is detached. Mixed-stage test (`crates/ezsdr-radio-uhd/src/provider/mod.rs:2404`) · P1408 (`plan/maintenance/tools/mutations.json:31381`)

**Minor**

- **The plan-record rules disagree on when the row is written.** UR-26 omits the condition that uhd-control was started. UR-30 says the thread must have been joined, although the code also writes the row after a control timeout. State the condition in UR-30 and have UR-26 cite it for the enqueue order. UR-26 (`design/18-uhd-radio.md:355`) · UR-30 (`design/18-uhd-radio.md:391`) · Stop implementation (`crates/ezsdr-radio-uhd/src/provider/mod.rs:831`)
- **New handshake timeouts can be counted as assertion kills.** If a released worker is descheduled for 10 seconds, `wait_ended` panics without `EZSDR-INFRA`. Mark that timeout and the two-worker manual-clock setup timeout as infrastructure failures. Worker handshake (`crates/ezsdr-radio-uhd/src/provider/mod.rs:2232`) · Manual-clock setup (`crates/ezsdr-radio-uhd/src/provider/mod.rs:2607`)
- **Some mutation anchors include unrelated hooks.** P442 removes RX hooks while reordering the stage calls. P1404 and P1405 include those hooks in their anchors too. Use `old2` to target the affected sends and stage calls directly. Rename P245 to say it skips TX owners after stream 0. P442 (`plan/maintenance/tools/mutations.json:5622`) · P1404–P1405 (`plan/maintenance/tools/mutations.json:31341`) · P245 (`plan/maintenance/tools/mutations.json:5038`)

`rules.py --base e178a2e5` and `git diff --check` passed. The recorded targeted mutation run passed 24/24. The recorded workspace run had a previously documented T200 signal-trap self-check failure. I reviewed read-only and did not rerun builds or tests. Mutation log (`mut1.log:56`, outside the repository) · Workspace log (`test-ws.log:3584`, outside the repository)
## Terra — fix re-check

**PASS.** No major or minor findings in the requested re-check.

The last-poll test covers the missed sequence: TX0 stays stuck, TX1 finishes after the last poll’s completion check, and the expiry sweep must detach only TX0. P1415 retains the previous poll’s results, so it would incorrectly detach TX1 and fail the test. The released worker waits only on a channel, not the manual clock; a 10-second scheduling delay produces an `EZSDR-INFRA` marker. Test and hook (`crates/ezsdr-radio-uhd/src/provider/mod.rs:2403`) · P1415 (`plan/maintenance/tools/mutations.json:31463`)

The UR-26 and UR-30 wording matches the stop implementation, both identified handshake timeouts are marked, and P245’s description matches its mutation. The `old` and `old2` anchors for P442, P1404 and P1405 each match once in sequence.

This was a read-only review; I did not run builds, tests, or mutation rows. `git diff --check` passed.
## Opus — fix re-check

**PASS.** The major and the four minors are closed. No new major. Correct code was observed to pass once, not only traced. `mutate.py` runs each row's filter on the unmutated copy first, and a failure there reports `BASELINE FAILED`, not `killed` (`plan/maintenance/tools/mutate.py:5`, `:304-309`). In `mut2.log`, which includes P1415 and so ran on the fixed tree, P1415 (the tx twin) and P1405 (the rx twin) are both `killed`, so both baselines passed.

**My own failure scenarios (written before the checklist)**

1. **Stale sweep (Sol's major).** The joiner sweeps at the bound with completion results from the previous poll. `beside` ends during the last poll, so the stale results still show it running, and it is recorded as detached.
2. **The test cannot tell old and new results apart.** If `beside` ends before an earlier poll, any staleness of one round gives the right answer, and the mutant survives.
3. **The hook never reaches LAST_POLL.** If the round counter or the clock step changes, the release never happens. The test would still fail, but for the wrong reason.
4. **The test fails on correct code.** This happens if the hook waits on a thread that needs the clock while the joiner is blocked, or if descheduling changes the verdict.
5. **The mutant is killed for the wrong reason.** A compile error, or the mutant's `static` carrying state across stages, would fail the test without testing the sweep.

**Checked against the code**

- **Correct code passes** (`crates/ezsdr-radio-uhd/src/provider/mod.rs:474-521`). The hook advances the virtual clock by 250 ms at each tx poll, so rounds 0–3 see 0, 250, 500 and 750 ms. In round 3, which is LAST_POLL = 1000/250 − 1 (`:2398`), the hook moves the clock to 1000 ms, opens `beside` and waits until its handle has ended (`:2416-2419`). Round 4 then breaks on expiry (`:496`). The sweep reads fresh results (`:505`): `beside` is joined and only `stuck` is noted.
- **No dependence on timing.** The joiner is blocked inside the hook until `beside` has ended, and the clock moves only in hooks. `stuck` is held by `_stuck` until the closure ends. `beside` is a stub that only does `recv()` (`:2323-2333`): it needs no clock and no lock. Two other tests release a worker inside a hook and wait for it in the same way: the earlier version of this test at `Begun`, and `a_stage_has_its_own_bound` at `Polled` 0. Both passed in `test-ws.log`. The only wall-clock bound is `wait_ended`'s 10 s, which is now marked as infrastructure.
- **Scenario 3 is covered.** The test asserts that its LAST_POLL hook ran (`:2424`).
- **P1415 is killed by the intended assertion** (traced by hand). During the control stage, PREV becomes [true]. In tx rounds 0–3, PREV is [F,F]. In round 4 the clock is at the bound and prev is still [F,F], so the sweep detaches both s0 and s1. `joined_notes == [stuck]` then fails. The run records it as `killed`, not ERROR, so it compiled (`mut2.log`).
- **The old test missed this mutant.** It released `beside` at `Begun`, so the prev snapshot at round 4 was already [F,T] and the mutant survived. That confirms the gap Sol reported.
- **The rows I asked about (P245, P442, P1404, P1405, P1415) were all killed.** The targeted run was 15/15 killed (`mut2.log`). `rules.py --base e178a2e5` reports 0 problems, and `git diff --check HEAD` is clean. I reran both myself.

**Minors from Sol's review**

- **UR-26 and UR-30 are closed.** UR-26 (`design/18-uhd-radio.md:355`) now says "when uhd-control was started, … after the uhd-control stage and before the first transmit `Shutdown`". UR-30 (`:391`) says the record is written after uhd-control's stage, whether the thread was joined or the bound ended with it left detached. The code matches both: `control_ran` (`mod.rs:831`) is true when uhd-control was in `threads`, whatever the outcome of the join. `ur_16_the_transmit_stage_has_its_own_bound_after_a_stuck_control_stage` (`mod.rs:2473`, through `a_stage_has_its_own_bound`, `:2463-2465`) asserts one plan row after a stuck control thread.
- **The EZSDR-INFRA marks are closed.** `wait_ended` (`mod.rs:2235-2238`) and the two-thread manual-clock setup (`:2618-2621`) now call `test_support::infra` before they panic. Both run on the test's named `bounded` thread, so the marker names the test.
- **The anchors are closed.** P442, P1404 and P1405 now anchor on the sends and the `join_stage` calls only, using `old`/`old2`. The `#[cfg(test)]` join-point hooks stay in the mutated program. P442's mutant still joins the receive stage before the transmit one.
- **P245's description is closed.** The new name, "skips the transmit owners after stream 0", matches the mutant `&[tx_name(0)]`.

**Notes (no change required)**

- `mod.rs:822`: the comment still says the plans are recorded "once uhd-control is joined". That is also the case after the bound ends with uhd-control left detached. One wording fix would bring it in line with UR-30.
- `mod.rs:2400-2402`: the doc comment says the hook confirms the handle has ended and then the clock moves to the bound. The code moves the clock first (`advance`, `:2412`), then runs `open` and `wait_ended`. The joiner sees the same thing either way, so only the wording is off.
- `mod.rs:1761`: the one-thread manual-clock setup still has a plain `assert!` on a 10 s wall-clock deadline, with no infra mark. It is the twin of the one fixed at `:2618`. This code was there before the step, so it is outside this round's scope.

I did not build or run tests. P1415's kill path is traced by hand; whether correct code passes comes from the baselines in `mut2.log`, a single run each. Repeated runs on unmutated code are owed at the gate if the orchestrator wants them, though the test does not depend on timing. The Terra re-check was still running (its output file was empty) when I read the directory.
