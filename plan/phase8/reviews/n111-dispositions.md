# #111 + #119 — dispositions and counts

#111: RS-6 step 1's freeze cancelled Provider handles while holding the admission lock (a Provider whose cancel needs a lock it holds while submitting deadlocks). #119: cleanup read the TimeAuthority and the host clock under the machine lock. Third strike of `mech:supplied-callback-under-lock` (#103's F1, #111, #119): design note [n111-note.md](n111-note.md) — one KC-24a rule, pinned by lock records and a sweep. Way of working: a Sonnet subagent supervised Go DeepSeek; Sol 6.2 reviewed the test plan and the code; Opus and GPT-6.1 Terra reviewed side by side (plan re-checks, the sub-step code review, the fix re-check). Briefs [n111-briefs.md](n111-briefs.md), test plan [n111-test-plan.md](n111-test-plan.md) (v3), reviews [n111-reviews.md](n111-reviews.md), reports [n111-reports.md](n111-reports.md). Gate: `n111-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, test plan v1 | 8 | 2 | 1 | test-plan gaps (none counted) |
| Opus + Terra, plan v2 and v3 re-checks | see the sections below | | | none counted |
| Opus, code 531c73a3 | 0 | 6 | 4 | none |
| Terra, code 531c73a3 | 2 | 1 | 0 | none (tests did not pin correct lock placement) |
| Sol 6.2, code c31a2138..099fd26c | 2 | 2 | 0 | F1 (Provider stop overlapping a late cancel): `mech:supplied-callback-under-lock`'s fix shape, not counted as a fourth instance (found in the fix, before landing) |
| Opus, re-check of fix round 2 | 0 | 0 | 4 | none |
| Terra, re-check of fix round 2 | 1 | 2 | 0 | none (closed by argument, below) |

Follow-ups: the Phase 8-end review gets G10/G29's wall-clock killers and `RunStateMachine::{move_to, begin_stopping, finish}` still reading the host clock internally; 8b gets FreezeDispatch's 5 s deadline now covering the data thread's whole stop sequence (several devices).

## Decisions (orchestrator unless marked)


- Test plan (Sonnet, test-plan.md) open questions: Q1 the wake handle is taken after `admission` is dropped (the three-party cycle with `paced::wake`), adopted; Q2 KC-24a's Ceiling names `coerce` and the registered checks, adopted; Q3 C filed as #119 and included in this step (third strike, below), P2 (recheck `end` after `next_wakeup`) filed later only if the Sol review finds it reachable; Q4 `wake()` holding `wake_handle` across `schedule` stays (Review L); Q5 rows in plan/phase7/tools/mutations.json.
- Three strikes: `mech:supplied-callback-under-lock` (#103 F1, #111, #119). Design note: note.md — one KC-24a rule (no supplied TimeAuthority/HostClock call under `admission` or `machine`), #111 and #119 fixed with #103's read-before/act-after shape, and a sweep test over the watched doubles that pins the rule for every path.

## Sol 6.2 plan review (sol-plan.md): FAIL, 8 majors, 2 minors, 1 note — dispositions (orchestrator)
- F1 adopted: freeze-before-stop order kept with `admission` free — a second freeze waits for the first's cancellations to complete on a cleanup-only serialisation (not a lock any Module/Provider thread takes); T6 asserts cancellation completes before the single Provider stop; N111-1 killed by T1's direct lock assertion; N111-3 by per-handle counts across two completed freezes.
- F2 adopted (pre-existing race, same handles): an early withdrawal of the horizon cancels only when it removed the handle; a handle appended after the early drain is claimed by the control freeze; pinned with per-handle counts.
- F3 adopted: T2 gated so the unconditional wake is inside `schedule` with `wake_handle` held before the control loop returns; exact per-handle cancel counts (N111-6 an assertion kill); the inverse order pins KC-46a's permitted late wake if the text claims both.
- F4 adopted: a `testing`-gated observation on the freeze worker immediately before it takes `wake_handle` (and one before it drops `admission`), captured and asserted on the test thread; N111-2 and "frozen stored after the drop" become assertion kills.
- F5 adopted: #119 gets targeted tests and one row per UTC site: both `now` and UTC sampled outside `machine`, the exact samples reaching the transition records and termination; an abort during the unlocked cleanup window keeps RS-10's cause, `also` and mode.
- F6 adopted: the sweep uses caller-owned observations — the Kernel's own lock helpers keep, in `testing` builds, a per-thread record of `admission`/`machine` held, and the watched doubles read the calling thread's record at callback entry (no global probe, no read after forwarding); each phase positively observed; separate Runs for orderly finish, abort and device loss; the note says the sweep pins the coordinator's own threads, not every future path.
- F7 adopted: P2 is reachable and the change widens it: after `next_wakeup` returns, a requested end makes the control loop return without dispatching the agenda (no frozen-refusal escalation); a gated test with an orderly first request asserts no `also` entry and the orderly mode.
- F8: both paths pre-existing and outside the clock rule — filed as issues (the ordinary wake overwritten by the unconditional one / disconnect_signal waiting on wake_handle; `DataLink::drops()` wedging while holding `links`), not fixed here.
- F9 adopted (T3's trace order; a known Module Action's exact id once in `undelivered` and absent from the log). F10 adopted (spec texts as Sol states, KC-24a Ceiling updated, KC-43's sampled UTC cited). F11 noted (G29's 200 ms gate stays; not this step's).
- Plan v2 (test-plan-v2.md) open questions 1–5: adopted as recommended (freeze_gate via a third helper `lock_freeze`; T2b kept; #103's test migrates to the one `Held` representation; #120/#121 stay filed; kernel_surface treatment as start_probe). Re-check of v2: Opus (high) and Terra (high) side by side.

## Re-check of plan v2: Opus (recheck-plan.md) and Terra (terra-recheck-plan.md) — dispositions (orchestrator)
- T6 (both): assert `cancel:released < radio:stop` and `cancel:released < second Serialised`; identify the second freeze by the freeze hook's event, not by W_f (finish runs it on a deadline worker).
- T13 (both): gate on the data thread's wake `Schedule` return before `finish`.
- Escalation race (both; Opus N1's fix adopted): `dispatch_agenda` checks `frozen` right after taking the admission lock and returns without dispatching or escalating; the post-`next_wakeup` recheck is dropped. T9 places the freeze between the old recheck point and `dispatch_agenda`'s lock; DEVICE_LOST's default is Abort (`policy.rs:79`), so T9's Spec overrides it with a Stop policy (Opus N7).
- Withdrawal across Provider stop (Terra 10): the early withdrawal's remove-and-cancel runs under `freeze_gate`, so a freeze (and hence StopProviders) waits for an in-flight withdrawal cancel; lock order stays freeze_gate → scheduled, and the control thread holds no admission there. A test with a gate after removal and before cancel.
- T4 (both): `disconnect_signal()` on a worker; fixed and mutant release paths specified so the reading is asserted before G_s opens on the fixed path.
- RS-6 text (both): state RS-8a's deadline exception; "the second performance" labelled correctly; no unconditional "never".
- Minors adopted: T5 one handle (or "1 of N" asserted correctly) and no assertion inside the hook; T3/T11/T13/T14 each name the scheduled handle they cancel (a far-future untimed `stop`); T3's `action_log` assertion dropped (proves nothing in a Spec Run); T2b's wording (the second freeze's Done, not cleanup's end); T9 parks only on the entry instant's return; KC-46a not repeated, T2b cited in its *Test:* line.
- Side-by-side tally: both found the T6, T13, escalation and RS-6-text problems and T4's sequence; Terra only: the withdrawal-across-stop major and T5's count; Opus only: the concrete N1 fix, DEVICE_LOST's Abort default (T9 fails on correct code), the missing scheduled handles (N5), N8, N9. Time: Terra 1.8 min, 91 k tokens, 2%; Opus 16.5 min, 251 k tokens.

## Narrow re-check of plan v3: Terra (terra-recheck-v3.md) and Opus (recheck-v3.md) — dispositions (orchestrator); the plan is final with these amendments, no further re-check
- Opus M1: T10c asserts only that the horizon's cancel precedes every `Serialised` and `radio:stop` (the cleanup's own freeze may take the gate before the data thread).
- Opus m1: the withdrawal takes `freeze_gate` with `try_lock` and, if a freeze holds it, skips (the freeze drains and cancels the handle); the plan's line on RS-8a covering these waits is corrected; KC-24a's Ceiling adds: a supplied `cancel` that never returns holds the freeze (and the data thread's early stop) with no deadline of its own outside RS-8a's cleanup steps.
- RS-6 text (Terra 1, Opus m2): the exception covers RS-8a abandoning either performance (the waiting one or the one cancelling), and step 1 also waits for a withdrawal in progress (Opus's replacement sentence in recheck-v3.md, with the abandonment clause broadened).
- Opus m3: T9 records the Provider's stop mode on a separate line; the existing `radio:stop` line is unchanged.
- Notes n1, n2 (removal under the gate; `frozen` checked under the lock) are code-review items for Sol; n3–n5 noted; v58.rs:1232's resource-targeted untimed `stop` noted (cannot reach the break).
- Tally, plan v3: Terra (1.2 min, 2%): the RS-6 deadline wording; Opus (15 min): the T10c ordering major, three minors, five notes.

## Sub-step review of 531c73a3: Opus (opus-code.md) PASS 6 minors 4 notes; Terra (terra-code.md) FAIL 2 majors 1 minor — dispositions (orchestrator)
Production code: no defect found by either. All findings are test strength, spec text or maintainability.
- Terra 1 + Opus 7 adopted (major): T9 must fail if the `frozen` check moves before the admission lock: a controlled window between the check and the lock (e.g. a hook after the frozen read, before the lock) in which the freeze completes, so a misplaced check lets the item through to admission → KC-17 escalation; plus a row that moves the check before the lock (not only one deleting it).
- Terra 2 + Opus 4 adopted (major): T10c pins removal under the gate and cancel *return* before stop: a controlled window before the withdrawal takes the gate; a row that moves the horizon removal before the gate; the order asserted on the cancel's return (`Call.returned` or an exit line), in T6 too.
- Terra 3 + Opus 3 adopted: T10d asserts on its wait's timeout (open `gate_c`, join, assert `withdrawn:parked` absent) so N111-22 dies by assertion; drop `"kill": "bound"`.
- Opus 2 adopted: T4 waits for `BeforeDrop`, then requires the wake waiter's refusal before the gate opens, so a take moved above `drop(admission)` fails by assertion; N111-2 retargeted to move only the take.
- Opus 1, 9, 10 adopted (spec text): design/06 Changes row names KC-29b, KC-29b's Test line names T10a–d; KC-2 lists KC-24a as applying in every class; RS-6 step 1 says "refuse every further submission (spec 06 KC-24 step 1) and empty … under the admission lock" instead of an undefined `frozen`.
- Opus 5 (duplicated test setup) adopted where cheap: one helper for the ~25-line device-paced setup of T2a/T2b/T10a–d, `freeze_trace` reused in T6/T13; the coordinator SimAuthority setup only if a helper already fits. Not a refactor beyond the step's own tests.
- Opus 6 adopted: one doc line on `Reading.admission_held` (any thread) vs `Held.admission` (calling thread).
- Opus 8 notes: rename N103-10/11's names to cover the cleanup transitions (text only); `RunStateMachine::{move_to, begin_stopping, finish}` still reading the host clock internally → to the Phase 8-end review list (not in this step).

## Sol 6.2 code review of c31a2138..099fd26c (sol-code.md): FAIL, 2 majors, 2 minors — dispositions (orchestrator)
- F1 major adopted (production): Provider stop can start while a second freeze's cancel of a late-published horizon handle is outstanding (data-thread freeze releases freeze_gate → control publishes the handle → cleanup's freeze parks in cancel → data thread proceeds to StopProviders). Fix so that no Provider stop starts while any freeze's or withdrawal's cancel is outstanding, except where RS-8a abandons the performance doing the cancel (the exception Terra asked for in the plan re-check: covers abandonment of the cancelling performance itself). Implementer proposes the smallest mechanism (e.g. StopProviders serialised behind freeze_gate, plus whatever stops a handle published after that point from being cancelled after stop — or a proof it cannot be), written test-first: Sol's sequence as a failing test, then the fix; a row. RS-6 / KC-29b text if the rule changes.
- F2 major adopted: N111-22 and N111-23 must die by assertion on the operation itself, not via a 10 s receive timeout that a slow correct worker also hits: observe the gate / wake-lock attempt and the lock state (e.g. a hook at the attempt, `held` records), so a correct-but-slow run cannot take the failure path.
- F3 minor: a frozen read between the `BeforeAgendaLock` hook and `lock_admission()` is untestable black-box without moving the hook; the implementer either places the hook so no statement can sit between it and the lock (e.g. inside the locking helper for this call) and adds a row, or documents the residual in the test — decided by the orchestrator: try the first, fall back to the second.
- F4 minor adopted: raw `admission` / `machine` mutexes reachable only through the recording helpers (private fields, an explicit accessor for the probe's non-blocking observation), so an alias cannot bypass the recorder.

## Re-check of fix round 2 (a68d8123): Opus (opus-fix2.md) PASS, 4 notes; Terra (terra-fix2.md) FAIL, 1 major 2 minors — dispositions (orchestrator)
- Terra 1 (major: a late-published horizon is still cancelled after Provider stop): **closed by argument, decided by the orchestrator.** F1's defect was a stop *overlapping* an outstanding cancel; that is gone (both reviewers). A cancel after the stops is of a handle whose callback calls no Provider (agenda placeholders `pipeline.rs:805`, the horizon no-op `stepping.rs:419`, the wake that only clears the Kernel's wake flag `paced.rs:167`) — verified by the orchestrator — so its order against stop has no Provider-visible effect. RS-6 states it; RS-7 now says so across performances (Opus 3).
- Terra 2 + Opus 1 adopted (text, by the orchestrator): KC-29b's withdrawal reason and the gate's doc (KC-24a, `state.rs:220`) name the early stop's hold.
- Terra 3 (G10's killer asserts a 10 s stop bound): pre-existing row and test, re-run only because its hunk moved; not this step's defect. To the Phase 8-end review with G29's (same shape).
- Opus 2 note (FreezeDispatch's 5 s RS-8a deadline now also covers the data thread's whole stop sequence; with several Providers whose stops add up past 5 s, the `timed_out` entry moves from step 3 to FreezeDispatch): recorded; no one-device Run reaches it (one UHD stop ≈ 3 s). Revisit with 8b's several devices (handoff / 8b list).
- Opus 4: the `uhd_59` timing flake under load is settled by the gate's serial run.
- Text fix re-check: Terra alone (text-only).
- Text re-checks by Terra alone (text-only): round 1 FAIL (RS-6's withdrawal reason, KC-46a's "stays scheduled"), round 2 FAIL (RS-7 "within each performance" vs the skip branch; KC-46a vs a replaced wake slot), round 3 FAIL (RS-7 vs RS-8a abandonment), round 4 PASS. All fixed by the orchestrator in the spec text; no code change.
