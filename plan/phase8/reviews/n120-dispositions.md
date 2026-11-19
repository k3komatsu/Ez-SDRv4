# #120 + #121 — dispositions and counts

#120: a disconnect wake could overwrite a pending ordinary wake, leaving its handle outside the freeze's drains (`wake_handle` held across the TimeAuthority's `schedule`). #121: cleanup could hang on the `links` mutex when a supplied `DataLink::drops()` wedged while holding it. Instances 4 and 5 of `mech:supplied-callback-under-lock` after the n111 design note, whose KC-24a rule covered only the admission and machine locks; this step widens the rule to every recorded coordinator lock (`Locked<T>`, per-thread held counts in every build, a sweep over every Authority method). Sol's code review found two more sites of the mechanism the sweep could not see (a raw fault mutex; re-entry from a supplied `cancel` through `disconnect_signal`), both closed. Way of working: a Sonnet subagent (with Go DeepSeek for the inventory sweep); Sol 6.2 reviewed the test plan and the code; Opus and GPT-6.1 Terra reviewed each sub-step side by side; plan v2 went to code without a paper re-check. Briefs [n120-briefs.md](n120-briefs.md), test plan [n120-test-plan.md](n120-test-plan.md) (v2), reviews [n120-reviews.md](n120-reviews.md), reports [n120-reports.md](n120-reports.md). Gate: `n120-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, test plan v1 | 7 | 1 | 0 | test-plan gaps (none counted) |
| Terra, B1 | 0 | 1 | 0 | none |
| Opus, B2+B3 | 2 (spec text) | 2 | 5 | none |
| Terra, B2+B3 | 1 | 0 | 1 | `supplied-callback-under-lock` (a lock-free cancel against Provider stop; design changed before landing) |
| Sol 6.2, code | 2 | 3 | 0 | F1, F2: `supplied-callback-under-lock` (review findings, counted: instances 6 and 7) |
| Opus, fix-2 re-check | 0 | 2 | 2 | none |
| Terra, fix-2 re-check | 2 | 1 | 0 | none (closed by wording: the skipped wake's limit) |
| Terra, text checks | 2 | 1 | 0 | none |

## Decisions (orchestrator unless marked)


- Triage (Phase 8 cut-off, owner 2026-11-19): kept in Phase 8 (possible deadlock; finish hanging past RS-8a); `mech:supplied-callback-under-lock` instances 4 and 5 — the n111 note's rule (admission and machine only) was too narrow; widen it rather than patch two sites.
- Plan (test-plan.md, Sonnet) open questions, decided as recommended:
  - Q1 the `DEVICE_LOST` `now()` under the slot lock and (on the early stop) the freeze gate stays a named exception in KC-24a's Ceiling: no supplied thread can wait on those there; recording slots needs a Module-call marker, too large for two sites.
  - Q2 `FreezePoint::BeforeWakeTake` removed (with the take it observed).
  - Q3 a `drops()` abandoned by RS-8a in step 6 leaves every link's `drops` null in the Manifest (all or nothing); before the fix that Run never finished, so no existing Manifest changes. Decided by the orchestrator (failure-only path); reported at landing.
  - Q4 `Locked<T>` enforced by type; the mechanical `lock(&x)` → `x.lock()` rewrite lands first as its own sub-step commit.

## Sol 6.2 test-plan review (sol-plan.md): FAIL, 7 majors, 1 minor — dispositions (orchestrator)
All adopted. Plan v2, then implementation without a paper re-check (bt2's lesson: three paper rounds did not converge; AGENTS §9 step 3 asks for adjudication and amendment); Opus + Terra review each sub-step's diff, Sol the code at the end.
- F1: `link_drops` is accepted only when step 6 completed (the snapshot written by the step on success, or a completion flag checked by assembly); test: release the wedged `drops()` after the timeout is recorded and before assembly → every `drops` null; the test acknowledges the detached worker's late return.
- F2: T17 synchronises on `disconnect_signal()`'s return, asserts per-handle cancel counts (ordinary handle cancelled once) after `finish`; name a real row.
- F3: M7's killer publishes an unfired ordinary wake before the early freeze and asserts its cancel count across both freezes.
- F4: T18 waits for a fresh control-loop park before each release, and observes the final handle's publication before checking the list.
- F5: the ownership cutoff is defined once (a handle the coordinator holds = published to the list) in RS-6 and KC-46a, with tests on both sides of publication (accepted by the Authority but not yet published vs published) — and the coordinator must not leave an accepted-but-unpublished handle uncancelled when the freeze has passed: the publisher, finding dispatch frozen after its `schedule`, hands the handle to the freeze gate (or cancels under it) — design it; narrow KC-46a's latency claim (`disconnect_signal` schedules synchronously; an Authority may serialise schedule and cancel).
- F6: the watched Authority records every method (`descriptor`, `time`, `primary_root`, `pacing`, `governs`, …) with the held locks; fixtures assert they reach the paths they cite; the Q1 exception identified apart from a Module's use of its own time handle.
- F7: `kc_24a_freeze_runs_cancels_outside_the_admission_lock` updated for the removed `BeforeWakeTake` (its trace and its direct lock assertions kept).
- F8: inventory adds `Provider::instance()` / `Sink::descriptor()` under `with_inputs`, corrects prepare's `instance()` and Module-origin `coerce`, and the `links` clear that can drop a supplied `DataLink` under the guard (drop outside); KC-44 keeps the Module-slot `try_lock` exception explicitly.

## Plan v2 + B1 (orchestrator)
- F5's design (Sonnet): `WakeList{handles, closed}`; only the cleanup's freeze sets `closed`; a publisher that finds `closed` cancels its own handle without a lock; `wake` checks `closed` first and schedules nothing. Accepted for implementation, with the cost named for the B2/B3 reviews: this one path cancels outside the freeze gate (a wake's callback calls no Provider; the reviews must check RS-6 step 2's "no outstanding cancel" text and any deadlock with a supplied Authority that serialises cancel and Provider stop).
- B1 committed (no behaviour change); reviewed by Terra alone (mechanical), in parallel with B2.

## B2+B3 review (fbfae8b3): Opus (opus-b23.md) FAIL 2 majors (spec text) 2 minors 5 notes; Terra (terra-b23.md) FAIL 1 major — dispositions (orchestrator)
- Terra 1 (major: the publisher's lock-free cancel can deadlock with Provider stop under an Authority that serialises cancel against something the stop needs) + Opus 6 (accepted it, verified only for UHD) + Opus 3 (the cancel's panic swallowed): **decided by the orchestrator: the publisher that finds the list closed cancels nothing; it drops its handle, which stays scheduled as a no-op (its callback only touches two atomics), as KC-46a already allows for a wake after the last freeze.** Every coordinator cancel is again under the freeze gate; no new path for a supplied Authority to meet Provider stop. T20 asserts the handle is not cancelled and the wake stays a no-op; rows follow.
- Opus 2 adopted (spec): the cutoff is "a wake handle is held once it is in the wake list"; one whose publisher finds the list closed is not held and stays scheduled as a no-op — RS-6 step 1 and KC-46a, same words.
- Opus 1 adopted (spec): "no wake that *starts* after that performance schedules anything".
- Opus 4 adopted: `#[cfg(feature = "testing")]` on `PendingWake.generation` (a warning in the shipped build).
- Opus 5 adopted: KC-44's reason reworded so it does not contradict KC-24a's exceptions.
- Opus 7, 9 adopted: name the hidden RS-8a bound in T20/T22's Deschedule comments; one helper for the duplicated `on_schedule_return`; the source scan rejects any field whose type contains a lock not wrapped in `Locked` (e.g. `OnceLock<Mutex<_>>`).
- Opus 8 / Terra 2: N120-12/14/15 confirmed in the full run and the gate.

## Sol 6.2 code review of cefd067c..465e5a9a (sol-code.md): FAIL, 2 majors, 3 minors — dispositions (orchestrator)
Both majors are further sites of the same mechanism the widened rule names; they are found by review, not by the sweep, so the fix also closes the sweep's two blind spots (an unrecorded lock; re-entry from a supplied call).
- F1 adopted: the Simulation step-fault path holds the fault-vector mutex (a temporary guard in `apply_faults`'s argument, `stepping.rs:179`) across the `DEVICE_LOST` `now()`. Take the vector out in its own statement; the fault mutex becomes a `Locked` (no raw coordinator mutex is excluded from the scan except `store`/`freeze_hook`, each with its reason); a sweep fixture drives a Simulation `DeviceLost` with the watched Authority.
- F2 adopted (re-entry): a supplied `cancel` that synchronously calls `disconnect_signal()` makes `wake` run `now()`/`schedule()` on a thread that holds the freeze gate. Decided by the orchestrator: `Locked<T>` keeps a per-thread count of held coordinator locks in **all** builds (not only `testing`); `wake` called on a thread that holds any coordinator lock schedules nothing — the holder is the control loop (withdrawal, cleanup freeze) or the data thread's early stop, each of which re-reads the end request after release, and the early stop makes its unconditional wake after `drop(gate)`. A test with a supplied `cancel` that calls `disconnect_signal()` (no deadlock; no `now`/`schedule` under the gate; the end request still served). KC-24a/KC-46a say so in one clause.
- F3 adopted: a second #121 sequence keeps `drops()` parked until the sealed Manifest arrives (Manifest assembly never waits on the abandoned call).
- F4 adopted: the three 10 s waits before N120-8/9's list-length assertion are infra failures (`EZSDR-INFRA:`).
- F5 adopted: the RS-6 Changes row and the comments at `state.rs:206` and `ending.rs:274` describe the no-cancel cutoff.
- Re-check after the fix: Opus + Terra on the fix diff and its neighbours (F2 changes production in every build).

## Re-check of fix round 2 (cbd90187): Opus (opus-fix2.md) PASS, 2 minors 2 notes; Terra (terra-fix2.md) FAIL, 2 majors 1 minor — dispositions (orchestrator)
- Terra 1 + 2 (majors: a wake skipped on a Module thread holding the admission lock inside `coerce`, or on a thread holding another Run's lock, is never re-read by that thread) + Opus 1 (same sequence, rated minor): **decided by the orchestrator: wording, not a new mechanism.** The paths need an embedder that wires `disconnect_signal` into a supplied `coerce`/check/`cancel`; no Module can reach the signal, and the one real caller (the server's input-watch thread) holds no coordinator lock. KC-24a and the `wake` doc now say the end request is seen at the control loop's next wakeup, as for a refused wake (KC-46a). A deferred wake run on the last lock release (supplied code in a guard's drop) was rejected as more mechanism than the reach justifies.
- Terra 3 + Opus 4: N120-19 dropped (byte-identical to N120-1, bound-only kill).
- Opus 3: the vacuous `assert_eq!(manifest, sealed)` removed.
- Opus 2 (an Authority deadlocking on its own lock with no coordinator lock involved): outside KC-24a; noted.
- Text re-check: Terra alone.
