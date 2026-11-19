# #111 + #119 — reviews

## Sol 6.2, test plan v1

I reviewed `work/n111` at `c31a2138` by reading the code and specs before opening the plan, note, and decisions. The conclusions below come from static analysis.

1. **Major — T6 rewards stopping Providers before step 1 finishes.** The data thread requests an end, claims the scheduled handle, and parks inside `cancel`. The control cleanup finds the handle list empty, completes its own freeze, and calls `Provider::stop` while the first cancellation is still pending. The inverse ordering is possible when the control worker claims the handle first. RS-6 and RS-7 put cancellation before Provider stop. RS-8a permits progress after a timeout, while `stop_early` itself has no deadline. Preserve that ordering while keeping `admission` free. Make T6 assert that cancellation completes before the single Provider stop, and keep N111-1 on T1’s direct lock assertion. With T6 as written, even the mutant can reach `radio:stop` while the gate is closed once the second freeze times out. N111-3 can still be caught by counting calls for the same handle across two completed freezes. Test plan (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:130`) · `crates/ezsdr-kernel/src/coordinator/paced.rs:133` · `crates/ezsdr-kernel/src/coordinator/ending.rs:291` · RS-6–RS-8a (`design/04-run-and-session.md:120`)

2. **Major — S4 misses a second owner of the same horizon handle.** `wait_for` returns from its loop early and `advance_to_with` samples `live = true`. Park its next `now()` call, let the data thread request an end and drain and cancel the horizon, then return `now < horizon`. The control path runs `retain` and calls `cancel(handle)` again even though freeze already removed it. The race predates the proposed change and invalidates S4’s once-per-handle claim. Make early withdrawal cancel only when it successfully removes the handle, and pin that sequence with a per-handle count. Also cover the other side of publication: if horizon `schedule` returns after the early drain, the control path can still append its handle, and the later control freeze must claim it. S4 and N111-3 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:59`) · `crates/ezsdr-kernel/src/coordinator/stepping.rs:394` · `crates/ezsdr-kernel/src/coordinator/ending.rs:307`

3. **Major — T2 can fail on the correct code.** `WallTime::cancel(horizon)` wakes the parked control loop. The control loop can observe an empty Authority queue and finish its wake-handle drain before the data thread publishes its unconditional wake. In that order, the wake survives, as KC-46a permits. Gate `next_wakeup` so the data thread has entered the unconditional `schedule` with `wake_handle` held before the control loop can return. Then assert the exact cancel count for each handle. That makes the missing-wake mutation N111-6 an assertion kill. Use the inverse order to pin the permitted late wake if the spec text claims both orders. T2 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:83`) · WallTime::cancel (`crates/ezsdr-kernel/tests/support/paced.rs:133`) · WallAuthority::wait_and_fire (`crates/ezsdr-kernel/tests/support/paced.rs:262`) · KC-46a (`design/06-kernel-coordinator.md:437`)

4. **Major — T4 can kill N111-2 by assertion.** With the disconnect wake parked in `schedule`, the mutant holds `admission` while waiting for `wake_handle`. The late Module submission waits too. A descheduled submitter produces the same timeout on the fixed code, and the five-second cleanup deadline can let later steps run while the freeze worker is still waiting. Add one `testing`-gated observation on the freeze worker immediately before it attempts `wake_handle`, with other admission contenders gated out. Capture the lock state there, open the schedule gate on either result, and assert the captured state on the test thread. Keep the frozen refusal as a second assertion. That gives N111-2 a named failure and makes the receive timeout a harness backstop. A separate assertion on that worker just before dropping `admission` can also catch a mutant that moves `frozen.store` below the drop. The existing thread-local start hook alone cannot observe either boundary. T4 and proposed survivor (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:111`) · start probe (`crates/ezsdr-kernel/src/coordinator/mod.rs:52`) · cleanup worker (`crates/ezsdr-kernel/src/run.rs:575`) · mutation verdict (`plan/maintenance/tools/mutate.py:179`)

5. **Major — #119 has no targeted test or mutation.** Before the freeze, a Module worker can hold a mutex needed by `utc_nanos()` and enter ordinary submission. Submission takes `admission` and waits for `machine` while the `Stopping` transition holds `machine` and waits for that mutex. At `CleanedUp`, an ordinary submission normally returns at the frozen check, so that same submission sequence would miss the second site. Watch each transition’s clock reads on the caller, assert that both `now` and UTC are sampled outside `machine`, and check that the exact samples reach the two transition records and the final termination fields. Give each UTC site its own assertion mutation. Reuse `sample_transition` and the existing pre-sampled publication path, preserving the public state machine’s validation-before-clock behavior. Include an abort during the unlocked cleanup window and assert the original cause, the `also` entry, and the actual stop mode under RS-10. Decisions (`~/.cache/ezsdr-fixes/2026-11-19-n111/decisions.md:3`) · proposed rows (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:143`) · `crates/ezsdr-kernel/src/coordinator/ending.rs:21` · `crates/ezsdr-kernel/src/coordinator/admission.rs:38` · `crates/ezsdr-kernel/src/coordinator/state.rs:387` · `crates/ezsdr-kernel/src/run.rs:628` · KC-39 and KC-43 (`design/06-kernel-coordinator.md:413`)

6. **Major — The proposed sweep can report both false failures and false passes.** During paced start, the data thread can enter `now()` while the control thread legitimately holds `admission` for the start batch. The current probe reports that *someone* holds the lock and has no machine reading. The lock and frozen readings are also separate observations. The captured weak probe works across threads, but the hook used to obtain it is thread-local, and calls before `Entered` slip past the current wrappers. `now()` is logged after successful forwarding, and several methods just forward. The same wrapped time handle is passed to Modules, so the log can include Module-originated calls. Use caller-owned observations at callback entry, or isolate the callers for each targeted assertion. Require a positive observation of each claimed phase and use separate Runs for orderly finish, explicit abort, and device loss. A single trace per execution class cannot guarantee that a fourth call site will fail the sweep. Design note (`~/.cache/ezsdr-fixes/2026-11-19-n111/note.md:13`) · probe (`crates/ezsdr-kernel/src/coordinator/mod.rs:37`) · watched doubles (`crates/ezsdr-kernel/tests/support/start_window.rs:288`) · assembly (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:110`) · paced start (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:331`) · PrepareContext (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:572`)

7. **Major — S3’s P2 sequence is reachable, and the fix changes when it can escalate.** Park the control thread after the Authority selects a due targeted `Stop` and before `next_wakeup` returns. Let the data thread request an orderly Policy stop and reach freeze, then release the return. The control loop enters `dispatch_agenda`, turns the frozen refusal into `Failed { run }`, adds it to `also`, and escalates the mode. After the proposed drop, that can happen during the first cancellation. Previously the agenda had to wait for that freeze to release `admission`, absent a timeout or another path. Record P2 as reachable and exercise it with a gated due item and an orderly first request. The admission check still prevents the Action from being dispatched. S3 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:49`) · `crates/ezsdr-kernel/src/coordinator/stepping.rs:311` · ending::request (`crates/ezsdr-kernel/src/coordinator/ending.rs:210`) · KC-20 (`design/06-kernel-coordinator.md:259`)

8. **Major — The callback inventory misses two remaining lock paths.** First, a worker holding a mutex needed by an ordinary wake’s `schedule` can call the exported `disconnect_signal` first. Its unconditional wake waits for the same `wake_handle` held by the ordinary wake. A pending ordinary wake can also be overwritten by the unconditional wake before either fires, leaving the earlier handle outside both freeze drains. These paths predate the patch. Second, if `DataLink::drops()` wedges while holding `links`, RS-8a abandons step 6, then Manifest assembly waits on the same `links` mutex and can hang. The existing panicking-link test covers unwinding, which releases the guard. Keep the clock rule narrow and record these as separate follow-ups, with explicit ownership of the permitted late wake. Disconnect signal (`crates/ezsdr-kernel/src/coordinator/mod.rs:356`) · paced::wake (`crates/ezsdr-kernel/src/coordinator/paced.rs:153`) · step 6 (`crates/ezsdr-kernel/src/coordinator/ending.rs:414`) · Manifest assembly (`crates/ezsdr-kernel/src/coordinator/ending.rs:118`) · link panic test (`crates/ezsdr-kernel/tests/coordinator.rs:2525`)

9. **Minor — T3’s trace races, and S6’s empty checks miss lost records.** The cancel worker can acquire M and log `cancel:done` between W dropping M and logging `w:released_m`. Record `releasing_m` while W holds it, then join W before asserting. The `held` branch can kill N111-1 by assertion, provided it captures the reading before releasing W. The real blocking branch still runs under the RS-8a deadline. For overlap coverage, queue a known Module Action on a gated receiver and assert its exact ID once in `undelivered`, plus its absence from the Session log. A uniqueness check on an empty list also passes if the record disappears. Keep the `frozen` store, queue clears, and ID append together under `admission`. T3 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:90`) · T6 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:130`) · queue clear and receive (`crates/ezsdr-kernel/src/coordinator/state.rs:52`) · RS-16 (`design/04-run-and-session.md:173`)

10. **Minor — Reconcile the spec text with the adopted decision.** State the rule once in KC-24a for coordinator calls to supplied `TimeAuthority` and `HostClock` methods. Say explicitly that freeze releases `admission` before taking `wake_handle` or `scheduled`, and that their guards are released before cancellation. RS-6 should cite that rule, limit its refusal wording to Actions subject to admission, and preserve `Abort` escalation and KC-46a’s late wake. Keep KC-20’s panic aggregation and KC-30’s Module panic containment distinct from the HostClock change. The existing Ceiling’s specific `stop`/slot cycle is stale for the current paths: early `stop` follows freeze, and a later admission sees `frozen` before reaching `coerce`. A live `coerce` can still wait on a Provider-owned mutex held by a worker trying to submit. Runtime registered checks also run under `admission` and remain subject to SB-31 purity. Provider `stop` itself runs under its slot lock. Cite tests for the specific properties they assert, including the sampled UTC in KC-43. Plan wording (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:166`) · Design note (`~/.cache/ezsdr-fixes/2026-11-19-n111/note.md:12`) · KC-24a and KC-26 (`design/06-kernel-coordinator.md:329`) · KC-30 (`design/06-kernel-coordinator.md:372`) · SB-31 (`design/03-spec-and-binding.md:288`) · Provider stop (`crates/ezsdr-kernel/src/coordinator/ending.rs:359`) · Module Abort (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1206`)

11. **Note — T1 and T5 have direct assertion kills.** With the Simulation fixture quiescent, T1 can attribute the held admission lock to its cancel caller and kill N111-1. T5 can deterministically fire the first scheduled callback through the manual Authority and kill N111-4 through the resulting `CleanupFailure`. Assert the manual advance fired the expected callback and match it to the cancel event, so `false` pins the intended sequence. J95 and J111 already assert the exact panic totals. G29’s existing killer still uses a 200 ms wait to arrange the interleaving, so its re-anchor alone leaves wall-clock dependence. T1 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:76`) · T5 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan.md:123`) · manual Authority (`crates/ezsdr-kernel/src/time/authority.rs:194`) · panic-count test (`crates/ezsdr-kernel/tests/coordinator.rs:2639`) · G29’s test gate (`crates/ezsdr-kernel/tests/device_paced.rs:1096`)

**Verdict — FAIL before implementation. Keep cancellation outside `admission`, preserve the freeze-before-stop order, and revise the plan to prove handle ownership and both #119 transitions with assertions.**
## Opus, re-check of plan v2

# #111 + #119 test plan v2: re-check (Opus, read-only, 2026-11-19)

Worktree `~/local_work/Ez-SDRv4-n111` at `c31a2138`. Static reading only: no build, no `rules.py`, nothing run.

Order followed: I read test-plan-v2.md and the code first and wrote my scenarios, then read `sol-plan.md`, `note.md` and `decisions.md`. v2's section 0 already summarises Sol's findings, so my scenarios were not fully independent of them.

## 1. Own scenarios against the planned code

**Freeze gate and lock order. No new cycle found.**
- In the planned code only FreezeDispatch takes `freeze_gate`. Its two performers are the data thread's `stop_early` (`paced.rs:134-149`) and the `run_cleanup` step thread (`run.rs:576-582`, a fresh thread per step). Each takes the gate first, then `admission`, then leaf locks.
- The start holds `admission` (`pipeline.rs:333-352`) and spawns the data thread under it (`:341`), but it never takes the gate. A data-thread freeze there waits on `admission` while holding the gate, and no thread holds `admission` while waiting for the gate.
- Neither performer holds `admission`, `wake_handle` or `end` when it enters the freeze. `wake()` holds `wake_handle` across `schedule` (`paced.rs:166-172`) but never enters a freeze.
- A hung `cancel` makes the other performer wait on the gate rather than on `admission`. That is the same exposure as today: the control performer is abandoned at RS-8a, and `stop_early` has no deadline, as before. Module submissions no longer wait, which is the #111 fix.

**No dispatch after the freeze.**
- `frozen` is still stored under `admission`, and every dispatch path checks it under `admission`: `admission.rs:39` on the Session (`pipeline.rs:1011`), Module (`:1220`) and agenda (`stepping.rs:372`) paths.
- The start batch dispatches without re-admitting (`pipeline.rs:346-351`), but no freeze can precede it: the data thread starts under that same lock.
- G29's re-anchor keeps its killer.

**Lost or double cancel.** Every handle is cancelled once in these cases:
- the early withdrawal versus the freeze (F2, `removed` test);
- a push after the early drain (T10b);
- the wake taken after the drop (T2a).

Three paths remain:
- a wake issued after step 1 (KC-46a, permitted);
- an ordinary wake overwritten by `disconnect_signal`'s (#120);
- the **control-path withdrawal cancel** (`stepping.rs:416-421`). It runs without the gate, so a data-thread step 2 can start while it is outstanding. Its callback is a no-op (`stepping.rs:401`). See N3.

**Lost `undelivered` record.**
- None. Queue clears and the `undelivered` append stay under `admission`.
- A second freeze appends an empty list.
- An abandoned control freeze that later wakes appends nothing, because the queues are already empty.

**Agenda race after `next_wakeup`.** The planned recheck is a check-then-act gap (major N1 below).

**Tests that fail on correct code.** T6 (major N2) and T13 (major N4).

**Rows killed only by a timeout.** None found:
- N111-1 compiles with an unused binding, and mutation builds do not use `-D warnings` (grep of `plan/maintenance/tools`, `.cargo`, `Cargo.toml`).
- N111-2's T4 hang is broken by opening G_s.
- N111-13 leaves a handle pending, but nothing waits on it.

## 2. Sol's findings: CLOSED / NOT CLOSED by v2

| Finding | Verdict | Reason |
|---|---|---|
| F1 freeze-before-stop | **NOT CLOSED** (one line in T6) | The gate design closes it: no submitter takes the gate, and a second freeze waits for the first freeze's cancels. But T6's order assertion `second Serialised < radio:stop` (v2:146) fails on correct code (N2). The forced orders are `cancel:released < radio:stop` and `cancel:released < second Serialised`. |
| F2 horizon owned twice | CLOSED | `removed`-gated withdrawal (v2:57). T10a pins the single cancel and T10b the late push. Both are ordered by gates. N111-16 and N111-13 die on counts. |
| F3 T2 gating | CLOSED | T2a's order is forced. The data thread holds `wake_handle` from before its `schedule` entry (`paced.rs:166-167`), so freeze #2's take must wait for the stored handle. A cancelled wake never clears `wake_pending`, so no second wake follows (`paced.rs:154`). N111-6 dies on a count of 0. T2b pins KC-46a's late wake (see N8). |
| F4 freeze-worker observation | CLOSED (minor N6) | The freeze hook lives in `Shared` and runs on the performing thread. Its `BeforeDrop` and `BeforeWakeTake` readings give assertion kills for N111-14 and N111-2. T4 needs its thread and gate order stated (N6). |
| F5 #119 targeted | CLOSED | T7 checks both clocks at both transitions, with exact samples reaching the records and `termination`. Each site has its own row (N111-7…-10). T8 pins RS-10 in the unlocked window. The only `utc_nanos` sites are `state.rs:388`, `run.rs:615` and `run.rs:631` (grep `utc_nanos()`), so T7's count assertion is sound. |
| F6 sweep | **NOT CLOSED** (T13; the mechanism itself is closed) | Per-thread records answer the false-failure and false-pass cases: data-thread calls during the start's hold, calls before `Entered`, and Module-originated calls under no Kernel lock. T15 adds a source scan. T13's phase requirements can fail on correct code (N4). T11, T13 and T14 do not name the handle their `Cancel` requirements need (N5). |
| F7 agenda after `next_wakeup` | **NOT CLOSED** | The recheck narrows the window but leaves a gap between the check and the admission lock (N1). Move the check under the lock. T9's orderly premise also needs a Policy override (N7). |
| F9 T3 trace and known id | CLOSED (minor N5) | `releasing_m` is recorded while M is held, and W is joined before the asserts. k is in `undelivered` exactly once. `action_log` is empty for any Spec Run, so it proves nothing about k (note N9). T3 needs a scheduled handle, or no `cancel` occurs (N5). |
| F10 spec text | CLOSED (minor N3, N8) | The rule is stated once in KC-24a, and the stale Ceiling is replaced. RS-6 limits the refusal to admitted Actions and keeps the `Abort` escalation. KC-43 is cited. N3 covers two wording overclaims in RS-6 and N8 one duplicate in KC-46a. |

## 3. New findings

### N1 (major): the post-`next_wakeup` recheck leaves the escalation reachable

v2:58, v2:261, `stepping.rs:313-373`.

Sequence:
1. The control thread is in `run_loop`. `next_wakeup` returns the due instant of a targeted untimed `stop`, and the planned recheck reads `end` as `None`.
2. The control thread is descheduled.
3. The data thread receives DEVICE_LOST under a Policy `stop` (orderly). `request` sets the end, and `stop_early` runs freeze #1: the gate, then `admission`, then `frozen = true`, then the drop. It is now in its cancels.
4. The control thread enters `dispatch_agenda` (`stepping.rs:372`) and locks `admission`. `admit` refuses with "RS-6: dispatch is frozen" (`admission.rs:39`), and `dispatch_agenda` calls `request(Failed{KC-17 agenda entry}, Abort)` (`stepping.rs:378`).

Wrong outcome: `also == [Failed{KC-17 …}]`, and the mode escalates to Abort, so the Providers stop in `Abort`. This is exactly what T9 forbids. T9 parks before the recheck (v2:176), so it never exercises this window. The KC-20 sentence at v2:261 is literally true, but it does not deliver the property the step claims.

Fix (smaller, and closed by construction): in `dispatch_agenda`, right after taking the admission lock, `if frozen.load(Acquire) { break; }`. Then drop the post-`next_wakeup` recheck.
- `frozen` is stored under that same lock, so there is no gap.
- In the Simulation class no freeze runs during `run_loop`: its only freeze performer is the control thread's cleanup, which never overlaps `run_loop`. Simulation behaviour is therefore untouched.
- The recheck, by contrast, also skips that instant's agenda and round whenever any end arrives during `next_wakeup`, for example a Module `Abort` or `disconnect_signal` (`mod.rs:366-376`). That is an unlisted behaviour change: the audit list at v2:80 names no such test, for example `kc_36_a_disconnect_signal_ends_a_waiting_advance`. INFERRED: I did not check which existing tests reach this path; a cancelled horizon makes `next_wakeup` return `None` (`paced.rs:266`), where the recheck changes nothing.
- T9 as written kills removal of the check. N111-15 retargets to "delete the frozen break".
- KC-20's sentence becomes: "An agenda item reached once dispatch is frozen (RS-6 step 1) is not admitted and requests nothing."

### N2 (major): T6 fails on correct code

v2:146, `paced.rs:142-147`.

The data thread runs freeze #1 and then, on the same thread, `StopProviders` (`paced.rs:142-146`). It records `radio:stop` as soon as its freeze returns. The control freeze #2 sits on a step thread blocked on the gate, and it records the second `Serialised` only after it is scheduled again.

Sequence:
1. The test opens G_c.
2. On the data thread, `cancel:released` is recorded, then freeze #1 reaches `Done` and releases the gate, then `StopProviders` runs and `radio:stop` is recorded. `done` makes it the single stop (`ending.rs:366-370`).
3. Only then does the step thread wake and record the second `Serialised`.

Result: `second Serialised < radio:stop` is false on correct code, and usually so.

Fix: assert `cancel:released < radio:stop` and `cancel:released < second Serialised`. Both are forced, the first on the data thread and the second by the gate. Keep the `held.freeze` readings as the N111-11 and N111-12 kills; T1's `Cancel` reading also kills both.

### N3 (minor): RS-6's "step 2 never starts with a cancel outstanding" overclaims

v2:269. The sentence is false in two cases:
- RS-8a abandons a control freeze that waits on the gate. v2:55 itself allows this.
- The control path's early withdrawal cancel (`stepping.rs:419`) is outstanding when the data thread's step 2 runs. Its callback is a no-op, so there is no harm.

Fix: append "unless RS-8a abandons the waiting performance", and scope the claim to the handles step 1 takes. Also replace "(the data thread's, KC-46b)": in T6 the data thread's freeze is the first one, so write "a second performance, by either thread (KC-46b)".

### N4 (major): T13's phase requirements fail on correct code

v2:207, `paced.rs:142-148`, `paced.rs:666-673`.

The test waits only for freeze #1's `Done` and then calls `finish`. But `wake(true)` comes after `StopProviders`, and `ThreadedProvider::stop` polls its worker with 1 ms sleeps. Under the default Policy (`policy.rs:79`, DEVICE_LOST → Abort), nothing else issues a wake in the Running window.

Sequence:
1. Freeze #1 reaches `Done`.
2. The test calls `finish`, and the Stopping `Utc` is recorded.
3. Freeze #2 takes `wake_handle` and finds `None`.
4. The data thread then `Schedule`s its wake.

Result: there is no wake `Schedule` in the Running window and no `Cancel` in the Stopping window.

Fix: gate `finish` on the data thread's wake `Schedule` *return* (its handle filled), as T2a does.

### N5 (minor): unstated handle sources

T3 (v2:109) and T11, T13 and T14 (v2:205, v2:207, v2:208) do not say which scheduled handle produces their `Cancel`.
- **T3:** a Simulation Spec Run with no untimed entry and no `advance_to` has nothing in `scheduled`, so `cancel` never runs. G_w is then never opened, and the test ends at the infra backstop. Give it a far-future untimed `stop`, as in T1.
- **T13:** its Running-window `Cancel` by the data thread needs something for freeze #1 to cancel. Without an untimed `stop` or a pending wake, it fails on correct code, and N4's gating does not supply a handle. Give it a far-future untimed `stop`.
- **T11 and T14:** satisfied today only because a fired `advance_to` horizon stays in `scheduled` (`stepping.rs:412`; only the withdrawal removes it). Name it, or use a far-future untimed `stop`.

### N6 (minor): T4's thread and gate order

v2:126-129.
- The `disconnect_signal()` closure runs `wake(true)` on the calling thread (`mod.rs:366-376`). Called on the test thread, it parks the test thread itself. Call it on a worker W_d.
- v2:128 opens G_s "as soon as it has received the reading, on either result", but v2:129 needs G_s still closed. Order it this way:
  - On a passing reading, submit on W_s, assert `Err`, then open G_s.
  - On a failing reading, open G_s and fail.

  Under N111-2, a submit before G_s opens would block on `admission`.

### N7 (minor): T9's "orderly" premise is false under the default Policy

v2:174-182, `policy.rs:79`. DEVICE_LOST defaults to `Abort`, so on correct code the Stopping mode and the Provider stop mode are `Abort`, and T9's `Orderly` assertions fail. The `also` assertion still kills N111-15, because an overriding `Failed` is pushed even when the mode is already Abort (`ending.rs:220-224`). Sol's F7 also asks for an orderly first request.

Fix: set the Spec's `policies.failure."ezsdr.DEVICE_LOST"` to the Stop reaction (as `device_paced.rs:339` overrides it to `continue`). The same applies to any other v2 test that asserts an orderly mode after a loss. T2a, T6, T10 and T13 assert none.

### N8 (minor): the KC-46a append duplicates its own last clause

v2:265 against `design/06-kernel-coordinator.md:437`: "…a wake scheduled after step 1 has passed stays scheduled…". DA-1 writes each topic once. Add T2b to KC-46a's `*Test:*` line instead of a sentence.

### N9 (note): small accuracy points in v2's text

- T3's `action_log` assertion (v2:120) is vacuous for a Spec Run, which has no Session log. Drop it, or move T3 to a Session.
- T5 asserts inside the `on_cancel` hook (v2:134), against v2's own rule that hooks only record (section 2). Record the fired count and assert it on the test thread. A hook panic would only show up as a KC-30 cleanup failure.
- T2b says "Control cleanup runs to its end" (v2:105), but step 3 joins the parked data thread. The test's release after freeze #2's `Done` is what lets it finish. Say so, so the implementer does not wait on the join.
- T9 should park only on a `next_wakeup` return equal to the entry's instant (v2:176), not on the first return.

## 4. Verdict

**Not ready as written. Small plan edits make it ready:**
- N1: move the end check under the admission lock in `dispatch_agenda` and drop the recheck.
- N2: one assertion change in T6.
- N4: gate T13 on the wake's `Schedule` return.

The minors (N5–N8) belong in the same edit. Neither N2 nor N4 changes the design; N1 replaces one mechanism with a smaller one.

## Terra, re-check of plan v2

1. **F1 — NOT CLOSED.** The gate serializes the two freeze performers, and T6 now asserts the intended order. But T6 waits for W_f’s `Serialise` event; `finish()` runs that freeze on a separate deadline worker, so W_f cannot emit it. Identify the second freeze worker by the hook event instead. Also qualify the proposed RS-6 wording: a timed-out freeze can remain in `cancel` while step 2 proceeds. Plan, T6 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:139`) · deadline worker (`crates/ezsdr-kernel/src/run.rs:575`)

2. **F2 — CLOSED for the reported double-cancel and late-publication sequences.** T10a checks ownership before cancellation; T10b exercises publication after the first drain. There is a separate cancellation-order gap in finding 10 below. Plan, T10a–b (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:182`)

3. **F3 — CLOSED.** T2a gates the control return until the data-thread wake has entered `schedule` while holding `wake_handle`; T2b covers the inverse order. T2b should say it waits for the second *freeze* to finish, not for cleanup to finish: cleanup later joins the parked data thread. Plan, T2a–b (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:95`) · data-thread join (`crates/ezsdr-kernel/src/coordinator/paced.rs:46`)

4. **F4 — NOT CLOSED.** The shared freeze hook supplies the needed caller-owned readings, but T4’s sequence cannot run as written: the test thread calls `disconnect_signal()` synchronously and parks inside its `schedule` hook before it can start `finish()`. It also says to open G_s immediately on receiving the reading and to submit while G_s remains closed. Run the disconnect on a worker and specify separate fixed/mutant release paths so the assertion is made before opening the gate on the fixed path. Plan, T4 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:125`) · disconnect signal (`crates/ezsdr-kernel/src/coordinator/mod.rs:356`)

5. **F5 — CLOSED.** T7 watches both samples at each ending transition and checks the recorded values; T8 covers an abort in the unlocked window. Separate mutations target each read site. Plan, T7–T8 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:155`)

6. **F6 — NOT CLOSED.** Per-thread entry records address the original false attribution and missed-entry problems. T13, however, waits only for the first freeze’s `Done` before calling `finish()`. The data thread can be descheduled there; the Stopping transition can then precede its wake `Schedule`, and the second freeze can find no handle to cancel. Correct code can fail the required Running-window schedule or Stopping-window cancel. Gate and positively establish those events and their ownership. Plan, sweep (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:195`) · early stop and wake (`crates/ezsdr-kernel/src/coordinator/paced.rs:134`)

7. **F7 — CLOSED for Sol’s specified window.** T9 parks the return, completes the freeze, and checks the cause, `also`, and mode; the proposed recheck covers that sequence. A later window remains in finding 11. Plan, T9 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:174`)

8. **F9 — CLOSED.** T3 records the release while W still holds M, joins W, and checks the known ID exactly once. T6 also checks that ID across overlapping freezes, subject to finding 1. Plan, T3 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:109`)

9. **F10 — NOT CLOSED.** The proposed text incorporates the lock extent, exceptions, Abort, and late wake. Its unconditional “step 2 never starts with a cancel outstanding” conflicts with RS-8a’s timeout and with the withdrawal sequence below. State the deadline exception and make the promised ordering match the actual ownership protocol. Plan, RS-6 text (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:270`) · cleanup deadline (`crates/ezsdr-kernel/src/run.rs:538`)

10. **New — major: withdrawal can leave a cancel outstanding across Provider stop.** The control thread removes the horizon from `scheduled`, then pauses before `cancel`. The data thread requests an end, freezes an empty list, and stops the Provider; the control thread then cancels. The freeze gate serializes freezes only, so it does not cover this owner. Define and test how withdrawal’s removal and cancellation participate in the freeze-before-stop ordering, including a gate after removal and before cancellation. Plan, withdrawal (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:55`) · current withdrawal (`crates/ezsdr-kernel/src/coordinator/stepping.rs:416`)

11. **New — major: the end recheck still leaves an escalation race.** The control thread can pass the new end check, then the data thread requests an orderly end and completes freeze before the control thread enters `dispatch_agenda`. The targeted agenda action is refused as frozen and converted to a failure, escalating cleanup. T9 places the end before the new check and cannot detect this sequence. Coordinate the end/freeze decision with agenda admission, or explicitly decide and test the remaining behavior. Plan, recheck (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:58`) · agenda refusal (`crates/ezsdr-kernel/src/coordinator/stepping.rs:360`)

12. **New — minor: T5’s stated mutation result conflicts with its fixture.** T5 says the Run is like T1, which has two scheduled handles, but expects the aggregate panic message to say “1 of 1.” The current aggregation reports the total drained handles, so that fixture yields “1 of 2” if one cancel is counted as failed. Specify one handle for T5 or assert the correct total. Plan, T1 and T5 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:86`) · aggregation (`crates/ezsdr-kernel/src/coordinator/ending.rs:309`)

13. **New — note: the proposed tests have not yet established assertion kills.** In particular, the blocked sequences above can reach the `infra()` backstop on correct code; that is not evidence that a mutation was killed by its intended assertion. This is a static review; no builds, tests, mutations, or repository writes were run. Plan, backstops (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v2.md:249`)

**Verdict: not ready for implementation; revise the ordering protocol and affected test sequences first.**
## Opus, re-check of plan v3

# #111 + #119 test plan v3 — narrow re-check (Opus, read-only, 2026-11-19)

Worktree `~/local_work/Ez-SDRv4-n111` at `c31a2138`. Static reading only: nothing built, run or written except this file. Paths below are relative to `crates/ezsdr-kernel/` unless they say otherwise.

## 1. Own scenarios for v3's two new shapes

I read v3 in full before writing these, so they are not fully independent of its section 1. I derived them from the code, before checking T9 and T10a–c against them.

### Shape A — the withdrawal's remove-and-cancel under `freeze_gate` (v3:61-70)

**Deadlock: none found.**
- Three paths take the gate: the data thread's `stop_early` (`src/coordinator/paced.rs:142`), the cleanup step thread (`src/run.rs:575-579`), and the control thread's withdrawal (`src/coordinator/stepping.rs:416-421`, planned).
- None of them holds a lock when it waits on the gate:
  - `stop_early` is entered from `run` with nothing held (`paced.rs:73-74`).
  - The step thread is fresh.
  - The withdrawal drops its `scheduled` guard at the end of its block, and `now()` is read before the gate.
- Under the gate:
  - The withdrawal takes `scheduled`, then calls `cancel`, then on a failure `request`, which takes `end` → `also` (`ending.rs:210-224`).
  - The freeze takes `admission`, then leaf locks.
- No thread holds `admission`, `scheduled`, `end` or `also` while it takes the gate, so the order gate → admission → end holds everywhere. `dispatch_agenda` already takes admission → end at `stepping.rs:372`, `:378`.
- #111's cycle is not re-formed. A Module thread that holds M and submits takes only `admission`, and the withdrawal holds the gate, not `admission`, while `cancel` waits on M.
- Both shipped `cancel`s are short and non-blocking: `crates/ezsdr-radio-uhd/src/authority.rs:312-322` and `crates/ezsdr-sim-engine/src/lib.rs:101-106`.

**Lost or double cancel: none.**
- The withdrawal's `removed` and the freeze's `drain(..)` both run under the gate, so exactly one of them owns the horizon handle.
- A handle pushed after freeze #1's drain (`stepping.rs:412`) is pushed after the end was set, because the data thread requests the end before `stop_early` (`paced.rs:68-74`). `live` is then false, there is no withdrawal, and the control freeze claims the handle (T10b).
- If `now()` before the gate requests an end (`state.rs` `now()` failure path), `live` is stale, but the handle is still removed and cancelled once.

**Dispatch after the freeze:** the withdrawal dispatches nothing.

**New waits with no deadline (INFERRED reachable only with a `cancel` that hangs; finding m1 below).**
1. **Control side.** The data thread's freeze #1 hangs in `cancel` while it holds the gate. The control thread's wait returns early with `live` true and blocks in the withdrawal's `lock_freeze()`.
   - Before v3 it went on to `settle()`. The control freeze then waited on the gate inside `perform_with_deadline`, and RS-8a abandoned it after 5 s (`run.rs:575-585`), so a Manifest was written.
   - Now the client call never returns.
2. **Data side.** The withdrawal's `cancel` hangs while it holds the gate. `stop_early` waits on the gate and never reaches `StopProviders` (`paced.rs:142-146`).
   - Before v3 the data thread stopped the Providers on its own.
   - The control thread is stuck in the same hung call either way.

### Shape B — `dispatch_agenda` breaks on `frozen` right after taking the admission lock (v3:72)

**Deadlock: none.** The check is an atomic load under a lock the loop already takes, and the hook point (`BeforeAgendaLock`) is before the lock.

**Dispatch after the freeze: none.**
- `frozen` is stored under `admission` (`ending.rs:293-294`) and read under it, so there is no gap.
- The items before the break were dispatched before the freeze. The items after it are dropped unadmitted.
- Those dropped items are not in `undelivered`, and they were not before either: the old path also broke (`stepping.rs:379`). RS-16 records admitted Actions only.

**No silent hang.** `frozen` is set only by FreezeDispatch, which runs only from `run_cleanup` (`ending.rs:22-33`, `end` is `Some`) or from `stop_early` after a request (`paced.rs:73`). So `frozen` implies an end, and `run_loop`'s top check returns on the next iteration (`stepping.rs:313`).

**Reach.**
- The break is reachable only through a Resource-targeted untimed `stop`. A `Stop{target: None}` takes the branch before the lock (`stepping.rs:368-371`), and RS-10 drops its `Completed` (`ending.rs:216-225`).
- v3:27 says no existing test schedules a targeted `stop`. That is true of the kernel tests, but `crates/ezsdr-acceptance/tests/v58.rs:1231-1232` has one. It is in the control Spec of `mr_34_…`: a Simulation (mock) Run finished right after the start, with no freeze during `run_loop`, so it does not reach the break. It is also a precedent for the spelling. No existing test's outcome changes (search in section 4).
- In the Simulation class no freeze overlaps `run_loop`. Its only freeze is the control thread's own cleanup, and `cleanup` needs `&mut RunHandle`.
- The paced post-break `round` only drains (`stepping.rs:146-149`), so continuing orderly after the break is harmless.

**Lost or double cancel:** Shape B touches no handle.

## 2. Dispositions of the v2 re-checks (decisions.md, last section)

| Disposition | Verdict | Evidence |
|---|---|---|
| T6: `cancel:released < radio:stop` and `< second Serialised`; second freeze identified by the hook event, not W_f | **CLOSED** | v3:153-164. Both orders are forced: the first on the data thread (`paced.rs:142-146`), the second by the gate. The second `Serialise` comes from the step thread (`run.rs:575`). |
| T13: gate `finish` on the data thread's wake `Schedule` return | **CLOSED** | v3:217. The wake is scheduled after `StopProviders` (`paced.rs:148`). Gating on its return fills `wake_handle` (`paced.rs:166-168`) before the Stopping sample. |
| Escalation race (Opus N1, Terra 11): `frozen` check under the lock, recheck dropped | **CLOSED** | v3:72-77. See Shape B. N111-15 is retargeted to the break (v3:247), and its kill is `also` and mode, not a timeout. |
| T9: DEVICE_LOST overridden to Stop (Opus N7) | **CLOSED** | v3:178. The spelling matches `tests/coordinator.rs:1748`, and the default is Abort (`src/policy.rs:79`). |
| T9 parks only on the entry instant's `next_wakeup` return (Opus N9) | **SUPERSEDED, closed by replacement** | `Point::BeforeAgendaLock{index:0}` fires only for a due item inside the loop (v3:93, v3:179), so wake rounds with no due item never reach it. |
| Withdrawal outstanding across Provider stop (Terra 10): remove-and-cancel under `freeze_gate` | **NOT CLOSED (test, M1)**: the code shape is sound | The shape is sound (section 1A). T10c's third line-order assertion fails on correct code: major **M1** below. |
| T4: `disconnect_signal()` on W_d; explicit fixed and mutant release paths | **CLOSED** | v3:137-145. W_d parks inside `schedule` holding `wake_handle` (`paced.rs:166-167`, `src/coordinator/mod.rs:366-376`). The fixed path asserts before G_s opens. On the mutant path G_s opens first, so the freeze can complete. |
| RS-6 text: RS-8a exception, "second performance by either thread", no unconditional "never" | **NOT CLOSED (minor m2)** | The RS-8a exception and the label are in. But the sentence (v3:288) says only that the withdrawal waits for step 1. The conclusion "step 2 does not start while … a withdrawal is outstanding" needs the converse, that step 1 waits for an in-flight withdrawal, which is what T10c pins. "therefore" also reads as unconditional right after the exception. |
| T5: one handle; fired count asserted on the test thread | **CLOSED** | v3:147-151. N111-4 yields "1 of 1" (`ending.rs:312-315`). |
| T3, T11, T13, T14 name their scheduled handle (far-future untimed `stop`) | **CLOSED** | v3:99, :123, :215-218. |
| T3: `action_log` assertion dropped | **CLOSED** | v3:128-133 has no `action_log`. |
| T2b: waits for the second freeze's `Done`, then releases the data thread | **CLOSED** | v3:117-121. |
| KC-46a: no new sentence; T2b cited in its `*Test:*` line | **CLOSED** | v3:284. |
| Terra 13 (note): assertion kills not established | **Addressed statically** | v3:260-268 names an assertion for every row. M1 is the one test whose verdict on correct code depends on scheduling. |

## 3. New findings

### M1 (major) — T10c's `Serialised < radio:stop` fails on correct code

v3:196-201; `src/coordinator/stepping.rs:423`, `src/coordinator/ending.rs:15-33`, `:359-375`; `src/run.rs:471-473`, `:575-579`.

Sequence on correct code:
1. W_c holds the gate at `Point::Withdrawn`. The data thread's freeze records `Serialise` and blocks on the gate. The test releases W_c.
2. W_c cancels, records `cancel:released(horizon)`, drops the gate, and goes straight into `settle()` → `cleanup()` → `run_cleanup`. That spawns the step thread for FreezeDispatch.
3. `std::sync::Mutex` does not hand the lock to the waiter it wakes. The step thread can take the gate before the data thread is scheduled. It records `Serialised(step)`, freezes, reaches `Done` and drops the gate.
4. The control path's next step, `StopProviders` (`run.rs:473`), finds no `done` entry, because the data thread is still waiting. It stops the radio and records `radio:stop`.
5. Only then does the data thread take the gate and record `Serialised(data)`. Its own `StopProviders` is skipped by `done` (`ending.rs:367-368`).

Observable result:
- If `Serialised` means the data thread's (as v3:197-198 implies), `Serialised(data) < radio:stop` is false and the test fails.
- `cancel:released < radio:stop`, a single `radio:stop` and a count of one still hold.
- This is the same class as v2's N2: rare, but a verdict that depends on scheduling.

Fix: assert only the forced orders.
- `cancel:released(horizon)` < every `Serialised` (any thread).
- `cancel:released(horizon)` < `radio:stop`.

N111-21's kill is unaffected: it rests on `held.freeze == false` at the `Cancel`, which is a reading.

### m1 (minor) — Shape A adds two waits that RS-8a does not bound, and v3:79 claims otherwise

Section 1A gives the sequences. It is INFERRED reachable only with a `cancel` that hangs; both shipped Authorities' `cancel`s are non-blocking (paths in 1A). The waits are not a lock cycle: no waiter on the gate holds a lock.

- v3:79 says "a control freeze waiting on a gate held by a hung `cancel` is abandoned at RS-8a". That is true of the cleanup step thread but not of the withdrawal's wait, which runs outside `run_cleanup`.
- RS-6's "unless RS-8a abandons the waiting performance" does not cover the withdrawal either, and nothing bounds the data thread's `stop_early` wait.

Minimal fix: correct v3:79 to say that these two waits have no deadline.

Whether to also state it in KC-24a's Ceiling is the orchestrator's decision, because it is new spec text. A possible sentence: "the withdrawal's and the data thread's waits on the freeze serialisation are not under RS-8a; a supplied `cancel` that does not return holds the Run there."

Option (orchestrator's call; not required): the withdrawal could use `try_lock` and skip when the gate is held.
- This is correct: the holder can only be a freeze that has not yet released the gate, and it drains `scheduled` under the gate, so it owns the handle.
- It removes the control-side wait. The data-side wait stays.
- T10a would then assert the skip, not a wait.

### m2 (minor) — RS-6 step 1 text states the wrong direction (v3:288)

Replace the second sentence with:

> The performances of step 1, by either thread (spec 06 KC-46b), and the control loop's withdrawal of its own horizon (KC-29b) exclude one another: each waits until the other has finished its cancels. Except where RS-8a abandons a waiting performance, step 2 does not start while a cancel that step 1 took, or a withdrawal's cancel, is outstanding.

Also add T10a to the *Test:* list (v3:290), next to T10c.

### m3 (minor) — `ThreadedProvider::stop` records no mode, and extending the line breaks existing tests

- `tests/support/paced.rs:651-652` records the bare `"stop"`, with `_mode` unused.
- v3:304 says T9 "extend[s] it if not".
- `Probe::wait_for` matches exactly (`tests/support/run_doubles.rs:64-71`). `grep -rn '"radio:stop"' crates/ezsdr-kernel/tests/*.rs` gives 20 hits, among them the exact sequence at `device_paced.rs:184-185` and kg_04's gate at `device_paced.rs:1105`.

So T9's mode needs a separate line, for example `radio:stop_mode:Orderly`, recorded next to the unchanged `radio:stop`. T8 is fine: its stepped Provider already records `stop:{mode:?}` (`tests/support/doubles.rs:756-757`, `run_doubles.rs:772-773`).

### n1 (note) — no row pins "the removal is under the gate"

A mutant that removes the handle before `lock_freeze()` and keeps the gate only around `cancel` survives:
- **T10c:** W_c takes the gate before the loss.
- **T10a:** the freeze drained first, so `removed` is false.

It reopens Terra 10's cancel-after-stop order. A deterministic test needs a positive observation of W_c between the removal and the gate, which only the mutant has. Leave it to the code review, or name it in the brief's checklist.

### n2 (note) — a check moved before the lock survives T9

T9's seam parks before the lock, and the freeze completes before the release. A mutant that checks `frozen` between the hook and `lock_admission()` therefore passes, but it reopens N1's gap. As in n1, a code-review item.

### n3 (note) — `StartTrace::assert_extent` matches `Point` exhaustively

The match is at `tests/support/start_window.rs:82-90`. Adding `Withdrawal`, `Withdrawn`, `Scheduled` and `BeforeAgendaLock` needs an arm there, which is a compile error, not a runtime one.

No existing hook sees the new points:
- Hooks are installed only for the start, by `start_on_worker` (`start_window.rs:111-118`). Its callers are `coordinator.rs:4404`, `:4451`, `:4509`, `:4554` and `device_paced.rs:1569`, `:1625`, `:1699`, `:1753` (`grep -n "start_on_worker(" tests/*.rs`).
- The start never enters `dispatch_agenda` or `advance_to_with`: it ends in one `round` (`src/coordinator/pipeline.rs:300-358`, read).

### n4 (note) — T4 and T6 hold a cleanup step thread on a gate the test opens

T4 holds it on G_s and T6 behind G_c. Their "`cleanup_failures` empty" assertion therefore depends on the test opening within RS-8a's 5 s (`src/run.rs:525`). Normal; this is only the stall budget. T10c's waiter is the data thread, which has no deadline, and T9 parks no step thread.

### n5 (note) — T11 and T12's "Armed window" checked

H's `Schedule` comes in `resolve_schedule` (`src/coordinator/pipeline.rs:801`). That is called after `arm_instances` publishes `Armed` (`:660`, `:314-316`) and before `Running` (`:331`). So H's `Schedule` lies in the Armed window, and the requirement holds on correct code.

## 4. Searches behind the "not found" claims

- Existing tests asserting the old agenda escalation: `grep -rn "agenda entry\|dispatch is frozen" --include=*.rs crates python` found only `stepping.rs:378`, `start_window.rs:86` (a start-point assertion) and `crates/ezsdr-exec-native/tests/native_executor.rs:366` (a submit refusal). None of these is an agenda escalation. `grep -rn "KC-17" crates/*/tests` found only arm failures (`coordinator.rs:1051`, `:4290`).
- Other `frozen` readers: `grep -rn "frozen" crates/ezsdr-kernel/src` → `ending.rs:294`, `paced.rs:186`, `admission.rs:39`, `mod.rs:58`, `pipeline.rs:159`.
- Resource-targeted untimed `stop`: `grep -rn '"stop"' crates/*/tests python/tests`, filtered of `"target": null`, found one: `crates/ezsdr-acceptance/tests/v58.rs:1232` (see Shape B).
- `cancel` implementations: `grep -rn "fn cancel(&self" crates --include=*.rs` → `state.rs:321`, `:433`, `sim-engine/src/lib.rs:101`, `time/authority.rs:62`, `:365`, `radio-uhd/src/authority.rs:312`.

## 5. Verdict

**Not ready as written, one edit away.**
- M1: T10c's order assertions.
- m2: RS-6 sentence.
- m1: v3:79 claim and the Ceiling sentence.
- m3: T9's mode line.

M1, m2 and m3 are plan-text edits. m1's minimal fix is correcting v3:79; adding a Ceiling sentence is the orchestrator's decision. None of them changes the design.

Neither new shape adds a deadlock, a lost or double cancel, or a dispatch after the freeze, and no row is killed only by a timeout. After those edits the plan is ready for implementation without another re-check.

Not checked: nothing was built or run, and `rules.py` did not run.

## Terra, re-check of plan v3

1. **Minor — the RS-6 timeout exception is still incomplete; Opus N3 and Terra 1/9 are not fully closed.** A cleanup worker can enter its first freeze, take a handle, and hang in `cancel`. RS-8a abandons that worker after its deadline and cleanup proceeds to Provider stop while the cancel remains outstanding. No *waiting second performance* need exist, but v3’s exception only says “unless RS-8a abandons the waiting performance.” Broaden it to cover abandonment of the performance doing the cancel as well. The code’s deadline behavior is explicit in `crates/ezsdr-kernel/src/run.rs:538`; the proposed wording is in `~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:286`.

2. **Note — withdrawal shape: closed for the reported race.** If withdrawal wins the gate, it removes and cancels the horizon before a freeze can proceed to Provider stop. If freeze wins, it drains the handle and withdrawal finds nothing to cancel. The stated lock order introduces no cycle with admission in the paths reviewed. T10a, T10b and T10c cover ownership, late publication and ordering respectively; N111-21 has a direct lock-reading assertion. This closes Terra 2 and 10, subject to the deadline exception above. Plan (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:61`) · tests (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:186`)

3. **Note — agenda shape: closed.** An agenda action waiting for admission either takes the lock before freeze and dispatches before it, or takes it afterward, sees `frozen`, and breaks without the KC-17 escalation. T9 forces the latter ordering and uses the required orderly policy. This closes Opus N1/N7 and Terra 7/11. Plan (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:72`) · T9 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:177`)

4. **Note — T6 corrections: closed.** The second freeze is identified by its hook rather than the `finish` caller; the two asserted orders follow from the gate without ordering Provider stop against the second worker’s scheduling. This closes Opus N2 and Terra 1’s test finding. T6 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:153`)

5. **Note — T13 and handle sources: closed.** Waiting for the data thread’s wake schedule to return with its handle filled establishes the required phase ordering; T3, T11, T13 and T14 now name their cancellation handles. This closes Opus N4/N5 and Terra 6. Sweep (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:205`)

6. **Note — T4 sequencing: closed.** The disconnect runs on a worker, and the fixed and mutant release paths are distinguished. This closes Opus N6 and Terra 4. T4 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:137`)

7. **Note — remaining prior findings: closed.** T2b waits for freeze completion rather than cleanup completion; T5 uses one handle and moves assertions out of the hook; T3 drops the vacuous log assertion; KC-46a avoids duplicate text. These close Opus N8/N9 and Terra 3/12. The already-closed F2–F6 and F9 portions retain their v3 coverage. T2b (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:117`) · T3–T5 (`~/.cache/ezsdr-fixes/2026-11-19-n111/test-plan-v3.md:123`)

8. **Note — verification limit.** I found no additional deadlock, lost or double cancellation, dispatch after freeze, correct-code test failure, or mutation row that must die only by timeout in the two new shapes. This is a static assessment: no tests, mutations, builds, or repository writes were run, so assertion kills remain to be demonstrated, as Terra 13 noted.

**Verdict: not ready for implementation until the RS-6 deadline exception is corrected.**
## Opus, code (531c73a3)

# #111 + #119 code review (Opus, read-only) — commit 531c73a3 on work/n111, diff c31a2138..531c73a3

Reviewed: the production diff in `crates/ezsdr-kernel/src/coordinator/` (admission, ending, mod, pipeline, state, stepping), the new tests in `tests/coordinator.rs`, `tests/device_paced.rs`, `tests/support/{start_window,paced}.rs`, the N111-* / G29 / N103-10/11 rows in `plan/phase7/tools/mutations.json`, and the edits to `design/04` and `design/06`. I ran `rules.py --base c31a2138` (0 problems). I ran no cargo build and no mutation rows.

## Step 1 — my own failure scenarios (written before reading the plan) and what the code does

| # | Scenario | Result (evidence) |
|---|---|---|
| S1 | Lock order. A cycle between `freeze_gate`, `admission`, `machine`, `wake_handle`, `scheduled` and a Provider lock M that `cancel` needs while a Provider thread holding M submits. | No cycle. The freeze takes gate → admission (released) → wake_handle / scheduled, then cancels under the gate alone (`ending.rs:293-323`). The withdrawal takes gate → scheduled, then cancel (`stepping.rs:428-441`). Submitters take admission only and never the gate (`pipeline.rs:1220`, `:1011`, `stepping.rs:372`). The gate is never retaken on the same thread: `_gate` in the withdrawal ends with its `if let` block, before `settle()`. Nothing that holds `wake_handle` or `scheduled` waits on the gate. |
| S2 | The withdrawal skips (`try_lock` fails), but the freeze holding the gate drained `scheduled` before the horizon was pushed, so nobody cancels the handle. | Unreachable. A freeze runs only after `request()` has set `end` (`paced.rs:75-77`, `ending.rs:21-37`). `live` needs `end.is_none()`, and the push precedes `run_loop`. So a freeze that holds the gate while `live` was true began its drain after the push. If `end` was set before the push, `live` is false and the control freeze claims the handle (T10b). |
| S3 | A handle cancelled twice, by the withdrawal and the freeze. | No. Removal (`retain` + `removed`) and `drain(..)` are exclusive under `scheduled`, and the cancel runs only `if removed` (`stepping.rs:431-438`). |
| S4 | A step-2 Provider stop while a withdrawal cancel is still outstanding. | Excluded: the data thread's freeze waits for the gate the withdrawal holds across its cancel (T10c). |
| S5 | An agenda item dispatched after the freeze, or a frozen refusal escalating an orderly end to Failed/Abort. | Fixed. The `frozen` check comes right after `lock_admission()` (`stepping.rs:372-376`), and `frozen` is stored under that lock (`ending.rs:297-298`), so there is no gap. The items already drained from `agenda` are dropped, as the `Stop{None}` and `Err` branches did before. |
| S6 | A wake after the freeze. | It stays scheduled, as KC-46a says (T2b). Before the change, the freeze took `wake_handle` under the admission lock, so a `schedule` that blocks made admission wait. Now it waits under the gate only (T4). |
| S7 | A clock sample published out of order with its transition. | No. Only the control thread publishes transitions. The cleanup's `sample_transition()` and `publish` are adjacent (`ending.rs:29-30`, `:40-41`). A `now()` failure inside the sample escalates through `time_failure`, as it did before. |
| S8 | An abort in the new unlocked windows. Between `drop(admission)` and the cancels, it only escalates `end`. Between the sample and the publication, the Stopping mode stays the one read first. | Pinned by T8. |
| S9 | A cleanup that hangs past RS-8a. | The withdrawal never waits (it uses `try_lock`). The only new holder of the gate across a supplied call is the withdrawal's `cancel` on the control thread, and if that call hangs, the control thread is hung anyway. The gate takes over exactly the extent the admission lock had in the freeze before. A cancel that hangs inside a freeze stalls the data thread's `stop_early` with no deadline. That was true before with the admission lock, and the Ceiling now states it. |
| S10 | Tests that pass on a wrong implementation, for example a hook that records `held` but is not called on the path. | See findings 2, 3 and 7. Every assertion I traced reads `held` captured at the supplied call's entry, on the calling thread (`start_window.rs` `WatchedTime::{now,schedule,cancel}`, `WatchedClock::utc_nanos`). Every test also asserts that the call happened (counts or exact handles). |

I found no correctness defect in the production change.

## Step 2 — against the plan v3, the decisions and the report

- **Plan coverage.** Every planned sequence has its test: T1–T15, T2a/b, T10a–c, plus T10d, added with row N111-22 under decisions m1. The tests assert on the planned observations. Deviations: N111-20 is folded into N103-11, and N111-22 is a `bound` row; both are discussed below.
- **The every/only/all claim (KC-24a: no `TimeAuthority` or `HostClock` call under `admission` or `machine`).** I listed every holder.
  - Admission holders: the freeze, `dispatch_agenda`, the start batch, `submit`, and `Submitter::submit`. Inside these sections the only supplied calls are `coerce` and the registered checks, which the Ceiling names. `start_data_thread` under the start's lock spawns a thread whose `now()` calls hold no lock of their own, and the per-thread record covers that.
  - Machine holders: `check_running`, `state()`, `transitions()`, `time_failure` and `publish`. Each is statement-scoped, with no callback inside.
  - Every other `now()`, `schedule` and `cancel` site (`grep -n "now()\|now_ticks\|schedule(\|cancel(" src/coordinator/*.rs`) runs outside both locks.
  - The claim holds for the code as it stands.
- **design/ text.** RS-6, KC-20, KC-24a (rule, Ceiling, Test), KC-29b, KC-43 and KC-46a match the code; findings 1, 9 and 10 are the exceptions.
  - The old KC-24a Ceiling (control in `coerce` waiting for a Provider slot that KC-46b's `stop` holds while it joins a Provider thread blocked on `admission`) is dropped, and that is correct.
  - The data thread's step 2 follows its own step 1, and step 1 takes `admission`, so the freeze waits for any holder from before it.
  - A control-path admission that begins after the freeze is refused at KC-24 step 1 (`admission.rs:39-41`), before step 10's `coerce`, so it never waits for the slot.
  - I searched `design/` and `KNOWN_LIMITATIONS.md` for `KC-24a`, `admission lock`, `machine lock`, `bounded join` and `submit(s) none`. Nothing else cites the old Ceiling.
- **Test deletions.** I read every `^-` line of `git diff c31a2138..531c73a3 -- crates/ezsdr-kernel/tests`. The only removed assertions are in #103's `kc_24a_start_reads_the_transition_clocks_outside_the_lock`, along with the `ClockEvent` doubles. The migrated test (`coordinator.rs:4434`) asserts `!held.admission && !held.machine` on every call the start makes, where the old one asserted `!admission_held` on `now` and `utc` only. It keeps the Now→Utc→`BeforePublish` order and both sample equalities. The old test's `finish_checked()` before reading events added nothing: its probe was dead by then. T7 now covers the cleanup's reads. The migrated test is equivalent or stronger. The other removals are imports and `ThreadedProvider::stop`'s signature (`_mode` becomes `mode`, with behaviour unchanged unless `recording_stop_mode`).
- **Row evidence.** I read `rows-{a,b,c1,c2,c3,d,e}.log`. Every N111 id in the json has a line:
  - Killed: N111-1, -2, -3, -4, -6 to -19, and -21.
  - `killed (bound)` with `"why": "infra marker"`: N111-22.
  - Killed again in `rows-e.log` (21:30): G29, C10 and N103-1/2/3/5/6/7/10/11. The maintenance list's J95, J111, J187, J193 and J198 are killed too.
  - N111-1, -2 and -14 ran in stage c1 (19:36), before stages c2 to e edited `ending.rs`, `stepping.rs` and the tests. The full-list run before the Sol review (AGENTS §9 step 6) re-establishes those outcomes on the final tree. No row log shows a survivor.

## Findings

1. **minor — the Changes row omits KC-29b.** `design/06-kernel-coordinator.md:369` (KC-29b) gains a new normative clause: the cancel runs under the freeze gate, and the withdrawal does nothing when a freeze holds the gate. But the step's Changes row at `:627` lists only "KC-20, KC-24a, KC-43, KC-46a". KC-29b's own `*Test:*` line also names none of the withdrawal tests (T10a–d). They are cited only under KC-24a and RS-6. Fix: add KC-29b to the row, and either cite `kc_24a_an_early_withdrawal_*` / `kc_24a_a_withdrawal_cancel_precedes_the_provider_stop` in KC-29b's Test line or point to KC-24a. `rules.py` does not catch this.

2. **minor — "wake handle taken under the admission lock" is pinned through the hook's position, not the take's.**
   - N111-2 moves the `BeforeWakeTake` hook together with `lock(&self.shared.wake_handle).take()` (`mutations.json:1641`, against `ending.rs:314-317`), so T1 and T4 read the mutant's `held.admission` at the hook.
   - A regression that moves only the `take` above `drop(admission)`, and leaves the hook after it, is invisible to T1. T1 is single-threaded, so the take never blocks, and the hook then reads `admission == false`.
   - In T4 that regression makes the freeze block on `wake_handle` with the admission lock held before `BeforeWakeTake`, so T4 ends at its `infra("the freeze did not reach BeforeWakeTake …")` backstop (`device_paced.rs:1871`). That is an ERROR, not an assertion kill. It surfaces rather than survives, so this is not a major.
   - Sequence: `wake(true)` parks in `schedule` holding `wake_handle`. A regressed freeze takes the admission lock and blocks on `wake_handle`. W_s's `submit` would block on `admission`. The test's only observation is the missing hook.
   - Suggestion: in T4, wait for `BeforeDrop`, then start W_s and require its refusal before G_s opens. Use a bounded wait followed by the mutant-path open, as T4 already does, so that the kill is the refusal-order assertion and does not depend on where the hook sits.

3. **minor — N111-22 is a `bound` kill where an assertion kill is available.**
   - T10d waits for the control freeze's `Serialise` and panics as infra after 10 s (`device_paced.rs` T10d, the `control_freeze` wait; row `mutations.json:1755`).
   - Under the mutant the withdrawal blocks on the gate. Once `gate_c` opens it acquires the gate, finds the handle drained and reaches `Point::Withdrawn`, which records `withdrawn:parked` (the trail). The test already asserts that this line is absent.
   - Suggestion: follow T4's mutant path. On the wait's timeout, open `gate_c`, join, and let the final `!withdrawn:parked` assertion fail. The fixed path is unchanged, and on a slow host it still passes.
   - The row could then drop `"kill": "bound"`.

4. **note — "cancel:released" is recorded at the cancel's entry, not at its return.**
   - In T6 (`device_paced.rs:1950`) the hook records "cancel:released" after its own gate opens, but before the inner Authority's `cancel` is called. In T10c (`:2558`) it is recorded at entry.
   - The orders asserted ("cancel before `radio:stop`", "cancel before every `Serialised`") therefore observe the cancel's entry and its hook's release, not its return.
   - With the gate held across the whole call this is equivalent for the code under test. A future change that dropped the gate inside the call would not be seen.
   - The `Call.returned` slot is filled after the return (`start_window.rs` `WatchedTime::cancel`). Comparing that call's log index against the `Serialised` and `stop` events would observe the return itself. Optional.

5. **minor (maintainability, tests) — duplicated rigs.**
   - Six device-paced tests repeat a block of about 25 lines: T2a `:2120`, T2b `:2214`, T10a `:2411`, T10b `:2472`, T10c `:2537`, T10d `:2613`. The block is: `paced_parked` → `WatchedAuthority` with hooks → `ThreadedProvider…losing_device_on` → `on_control_worker` with a tick channel → `wait_parked_until`.
   - T6 (`:1919`) and T13 (`:2701`) each inline their own freeze-recording closure instead of `freeze_trace`.
   - In `coordinator.rs`, T1 `:4615`, T3 `:4773`, T5 `:4903`, T7 `:4538`, T11 `:4978` and T12 `:5022` repeat the SimAuthority + WatchedAuthority + WatchedClock + SteppedProvider setup.
   - What would have to change together: any new hook field or Provider builder flag.
   - Suggestion: one `watched_paced_wait(…)` helper returning (worker, tick, log, threads), and one `watched_sim_rig(log)`. Keep each test's gates and assertions visible. The change adds about 1,650 test lines for about 120 production lines. (AGENTS §9 maintainability 3; not a correctness issue.)

6. **note — two representations of "admission held".** `start_probe::Reading.admission_held` (`mod.rs:52`, `:96`) uses `try_lock`, so it means "any thread holds it". `lock_record::Held.admission` means "this thread holds it". Both stay in use: `StartTrace::assert_extent` reads the first, and `Call.held` reads the second. Plan Q3 asked for one representation for #103's test. The `ClockEvent` arms migrated, but `Reading.admission_held` remains, with different semantics that are not stated where it is defined. A one-line doc on `Reading.admission_held` ("any thread; see `lock_record::held` for the caller's own") would stop a later test from mixing them up.

7. **note — T9 does not pin the check's place relative to the lock.** Only a `frozen` check moved between `reach(BeforeAgendaLock)` and `lock_admission()` survives T9; a check moved before the hook is killed. The code is right, with the check after the lock (`stepping.rs:372-376`), and the report records this for review. No action is needed beyond keeping the code comment that says why.

8. **note — stale row names and an unguarded HostClock API.**
   - Rows N103-10/11 (`mutations.json:1571`, `:1581`) mutate `Shared::publish`, which the cleanup's Stopping and CleanedUp transitions now share. Their `name` still says "the start's lock", and their filter runs only the start test, although T7 and the sweeps also kill them. A rename is cosmetic.
   - Separately, `RunStateMachine::{move_to, begin_stopping, finish}` (`run.rs:629`, `:657`, `:663`) still read the `HostClock` inside `&mut self`. They are kept as public API, which the plan decided.
   - A future coordinator call to them through `lock_machine()` would reintroduce #119. Only a runtime sweep on a driven path would see it, because T15 scans raw lock text and not these calls.
   - Optional: extend T15's scan to flag `begin_stopping(`, `.finish(…host_clock` or `move_to(…host_clock` under `src/coordinator/`.

9. **minor — KC-2 now confines KC-24a's new rule to the device-paced classes.**
   - KC-2 (`design/06-kernel-coordinator.md:206`) lists KC-24a among the rules that apply to the device-paced classes.
   - The lock is class-independent in the code, and so is the freeze's lock extent. The rule this change adds to KC-24a ("no `TimeAuthority` / `HostClock` call under the admission or machine lock") also applies in every class.
   - Its tests include Simulation Runs: T1 `coordinator.rs:4615`, T7 `:4538`, T11 `:4978` and T12 `:5022`.
   - The listing was already loose before this step, but the step makes it plainly false for KC-24a's new text.
   - Fix: drop KC-24a from KC-2's list, or narrow it to "KC-24a's start-batch clause".

10. **minor — RS-6 step 1 now names an internal flag instead of the user-facing behaviour.**
    - The old text, "**1** refuse every further submission, empty …", became "**1** set `frozen` and empty … under the admission lock" (`design/04-run-and-session.md:134`).
    - Spec 04 never defines `frozen`. What it means for a user, that every further submission is refused, now appears only in spec 06's KC-24 step 1 table (`:287`, `:309`).
    - Plan v3's text also carried "An `Abort` is not an Action subject to admission … accepted throughout and escalates the cleanup (RS-10)". It was dropped. That is harmless, since KC-24 and KC-24a state it.
    - Fix: "**1** refuse every further submission (spec 06 KC-24 step 1: dispatch is frozen) and empty every undelivered Action queue under the admission lock …".

## Mutation rows

- **N111-1, -14, -11, -12, -21:** each dies on a captured `held` or `dispatch_frozen` reading.
- **N111-2:** dies on the hook reading (see finding 2).
- **N111-3, -6, -13, -16:** each dies on a per-handle count.
- **N111-4:** dies on the KC-30 cleanup failure.
- **N111-15:** dies on `also` and the mode.
- **N111-7 to -10, -17, -18:** each dies on per-call `held`.
- **N111-19:** dies on the T15 scan.
- **N111-22:** `bound` (finding 3).
- **G29:** re-anchored to delete the lock and the drop together; its killer is unchanged.

The anchors are unique (`rules.py` 0 problems).

## Not checked

- I ran no cargo build or tests. The 20× repeats and the green suite are the implementer's claims.
- I applied no mutant, including the variants in findings 2 and 3. Their behaviour is reasoned from the code.
- I did not run the full mutation lists. The row outcomes come from the stage logs only.
- I did not read `terra-code.md` (independent review).

## Verdict

The production change is correct. My deadlock, double-cancel, lost-handle, post-freeze-dispatch and clock-order scenarios are all excluded by the gate, `removed`, the lock-then-`frozen` check and sample-before-publish. The spec text matches the code apart from findings 1, 9 and 10, which are bookkeeping and wording. The findings are minors and notes: spec bookkeeping, how strongly the tests observe the order, and test duplication.

Verdict: PASS

## Terra, code (531c73a3)

1. **Major — the agenda test does not pin the check inside the admission lock.** In `crates/ezsdr-kernel/tests/device_paced.rs:2356`, the control thread parks before taking the lock; the test completes the freeze before releasing it. The test also passes if the `frozen` check moves immediately before the lock. N111-15 only deletes the check (`plan/phase7/tools/mutations.json:1725`).

   Minimal wrong sequence: the control thread reads `frozen == false` outside the lock; the data thread takes the lock and freezes; the control thread then takes the lock and attempts admission. Admission refuses the item, causing a KC-17 failure and abort escalation instead of preserving the orderly policy stop. The current implementation places the check correctly (`crates/ezsdr-kernel/src/coordinator/stepping.rs:374`), but the test and row do not protect that placement.

2. **Major — the withdrawal test does not pin handle removal under the gate or completion of cancellation before stop.** T10c parks only at `Withdrawn`, after both removal and gate acquisition (`crates/ezsdr-kernel/tests/device_paced.rs:2569`). N111-21 removes the gate entirely; it does not move removal before it (`plan/phase7/tools/mutations.json:1745`). Also, `cancel:released` is recorded by the entry hook before the underlying `cancel` runs (`crates/ezsdr-kernel/tests/device_paced.rs:2553`); the ordering assertion therefore establishes entry, not return.

   Minimal wrong sequence: withdrawal removes the horizon before acquiring the gate; a freeze acquires the gate, finds no handle, completes and stops the Provider; withdrawal then acquires the gate and cancels after the stop. The current code removes and cancels under the gate (`crates/ezsdr-kernel/src/coordinator/stepping.rs:429`), but the tests would not reliably reject that misplaced removal. Add a controlled window before gate acquisition and observe cancel return before allowing stop.

3. **Minor — N111-22 is not an assertion kill.** The row explicitly declares a bound kill (`plan/phase7/tools/mutations.json:1755`); its test fails on an `infra` timeout waiting for the control freeze (`crates/ezsdr-kernel/tests/device_paced.rs:2681`). That distinguishes the blocking mutant only by elapsed time and can also count an infrastructure stall as a kill. The test does check the successful skip path, but this does not meet the requested assertion-based mutation evidence.

I did not run tests or mutation rows. The implementer reports the targeted rows killed and one workspace test failing in a background run but passing in the foreground; I did not independently verify those results. The findings above concern missing regression evidence for currently correct lock placement, rather than a demonstrated production failure in this commit.

Verdict: FAIL
## Sol 6.2, code (c31a2138..099fd26c)

1. **Major — Provider stop can start while the second freeze is cancelling a handle.** In a device-paced Run, pause `advance_to` after `schedule` returns a horizon handle but before it publishes the handle. Let device loss trigger the data thread’s freeze. Pause that thread after it releases `freeze_gate` and before it calls `StopProviders`. Resume the control thread so it publishes the handle, enters cleanup, and parks inside the second freeze’s `cancel`. Now resume the data thread. `Provider::stop` starts while `cancel` is still outstanding, even when neither step has reached the RS-8a deadline. That violates RS-6. The overlap test parks the **first** cancel, and the late-horizon test checks who cancels the handle without checking when Provider stop starts. stepping.rs:407 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), paced.rs:142 (`crates/ezsdr-kernel/src/coordinator/paced.rs:142`), ending.rs:317 (`crates/ezsdr-kernel/src/coordinator/ending.rs:317`), RS-6 (`design/04-run-and-session.md:134`), device_paced.rs:2521 (`crates/ezsdr-kernel/tests/device_paced.rs:2521`)

2. **Major — N111-22 and N111-23 rely on timeouts to expose the regressions.** For N111-22, park the freeze in `cancel` and release the withdrawal worker. The harness opens the cancel gate after 10 seconds if no control freeze appears. A correct withdrawal worker delayed until just after that release can acquire the now-free gate, reach `Withdrawn`, and fail the same assertion as the blocking mutant. For N111-23, park the wake in `schedule` and start the submitter after `BeforeDrop`. The mutant reaches the failing assertion only when the receive times out and opens the wake gate. A correct submitter delayed past that bound takes the same failure path. Both rows omit `kill: bound`. Observe the actual gate or wake-lock attempt and its lock state so the verdict follows from the operation being tested. device_paced.rs:1879 (`crates/ezsdr-kernel/tests/device_paced.rs:1879`), device_paced.rs:2649 (`crates/ezsdr-kernel/tests/device_paced.rs:2649`), mutations.json:1755 (`plan/phase7/tools/mutations.json:1755`)

3. **Minor — T9 leaves a placement of the frozen check untested.** Moving the `frozen` read just after `BeforeAgendaLock` and just before `lock_admission()` passes T9 because the test completes the freeze while the worker is parked at the hook. In a real Run, a freeze between that misplaced read and acquisition lets admission turn the frozen refusal into a KC-17 failure and escalate an orderly stop to abort. N111-24 moves the read ahead of the hook and catches that placement only. stepping.rs:372 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:372`), device_paced.rs:2375 (`crates/ezsdr-kernel/tests/device_paced.rs:2375`), mutations.json:1775 (`plan/phase7/tools/mutations.json:1775`)

4. **Minor — The lock-recording source check misses acquisitions through an alias.** The scan requires the field name and lock call on the same line. `let machine = &shared.machine;` followed by `lock(machine)` bypasses both the recorder and its source check. Make the raw mutexes accessible only through the recording helpers, with an explicit accessor for the probe’s nonblocking observation. state.rs:177 (`crates/ezsdr-kernel/src/coordinator/state.rs:177`), coordinator.rs:4523 (`crates/ezsdr-kernel/tests/coordinator.rs:4523`)

Verification was static. All 23 N111 rows have unique anchors and resolve to named tests. The diff passed `git diff --check`.

Verdict: FAIL
## Opus, re-check of fix round 2

# Re-check of #111 fix round 2 (a68d8123, work/n111): Opus, read-only

Scope: HEAD~1..HEAD (a68d8123) against the whole step c31a2138..HEAD. Static review, plus `rules.py --base c31a2138` (0 problems, so every row anchors exactly once) and the orchestrator's logs (`rows-fix2.log`, `rows-fix2b.log`, `ws-fix2d.log`).

Not checked:
- I did not run cargo, the tests or the mutation rows myself.
- I did not re-read the killer tests of C06, G10 and G29. Those three tests are unchanged in this commit; only C06's anchor moved.

## Step 1: my own scenarios, written before I read sol-code.md and decisions.md

| # | Sequence | Result on a68d8123 |
|---|---|---|
| S1 | (Sol's F1) The control thread's horizon H is scheduled but not yet pushed. The data thread's freeze drains `scheduled` without H. The control thread pushes H and enters cleanup, and its freeze reaches `cancel(H)` while the data thread starts `Provider::stop`. | Cannot happen. `stop_early` holds the gate from `lock_freeze` (paced.rs:143) to `drop(gate)` (paced.rs:155). `FreezeDispatch` blocks in `lock_freeze` (ending.rs:337) until every stop has run, so `cancel(H)` runs strictly after them. That is the residual RS-6 now states. |
| S2 | Deadlock: `Provider::stop` now runs under the gate. Could a thread that the stop waits for hold or wait for the gate? | No cycle. Only three places take the gate: the cleanup's step thread, `stop_early`, and the withdrawal's `try_lock` (stepping.rs:431, which never blocks). No Module thread takes it (state.rs:220). The data thread holds neither the admission lock nor the machine lock while it stops, so a Provider thread that submits while being joined gets through. No cancel is outstanding while the data thread holds the gate in step 2, so a Provider thread that calls the Authority does not meet a parked cancel. |
| S3 | A wedged data-thread stop. | The cleanup's `FreezeDispatch` parks at the gate and is abandoned after 5 s (RS-8a, run.rs:525). Step 3's thread takes `shared.data` and is abandoned in `join` (stepping.rs:264). The control thread's later `stop_data_thread` finds `data` empty and returns, so the RunHandle does not hang. Handles pushed after the data thread's freeze are never cancelled. The KC-24a Ceiling states this (design/06:331). |
| S4 | The skip-if-frozen branch loses a freeze. | `frozen` is stored only inside `freeze_held` (ending.rs:256), which runs only under the gate. `stop_early` reads `frozen` after it takes the gate. If it reads true, an earlier performance has finished its drain and its cancels. That earlier performance was the cleanup's, on the control path, so the control thread is already in cleanup and publishes no horizon afterwards. The data thread itself goes straight from `pass` into `stop_early` and schedules no wake in between. Nothing is lost. A wake from a disconnect signal after step 1 stays scheduled, as KC-46a already says. |
| S5 | RS-8a abandons the cleanup's freeze while the data thread is still stopping Providers, so the cleanup's StopProviders (which takes no gate) overlaps them. | The overlap happens, but `done` dedups it, and the only cancel performance left is the one waiting at the gate, so no cancel is outstanding. This is RS-6's stated RS-8a exception. Before the fix this overlap happened on every run; now it happens only after abandonment. |
| S6 | The withdrawal meets the gate while the data thread holds it in its stop phase. | `live` was read before the data thread set the end. H was pushed before `run_loop` (stepping.rs:420), so the data thread's freeze drained and cancelled H under the same hold. The skip is therefore correct. |
| S7 | A Simulation Run. | No data thread, so `stop_early` never runs. The gate is taken only by `FreezeDispatch` and the withdrawal, both on the control thread one after the other. Unchanged. |
| S8 | Tests that pass on a wrong implementation: (a) a gate released between freeze and stops and taken again for the stops; (b) `if !frozen` turned into `if false`. | (a) The new test would pass, but that variant also satisfies RS-6's "no stop during an outstanding cancel", so it is not a defect. (b) has no row, but every data-thread-freeze test expects that freeze (for example the `overlapping_freezes` test), and G10 covers the whole call. Not required. |

## Step 2: findings

No major findings.

1. **note** (wording only):
   - KC-29b says "when a freeze holds the gate, the withdrawal does nothing, because the freeze cancels the handle" (design/06-kernel-coordinator.md:369). The gate can now also be held by the data thread during its Provider stops. The conclusion still holds, because that thread's freeze took H under the same hold (S6). RS-6's wording ("when a performance holds it", design/04-run-and-session.md:134) is the exact one, and KC-29b could reuse it.
   - KC-24a (design/06:330) and the `freeze_gate` doc comment (state.rs:220) say the gate covers "the data thread's Provider stops that follow its own performance". In the skip branch (paced.rs:144) those stops follow no performance of the data thread's own, yet they still run under the gate. KC-46b (design/06:442) is exact here.

2. **note** (Manifest-visible, so the owner decides whether it needs text):
   - `FreezeDispatch`'s RS-8a deadline (`DEFAULT_CLEANUP_DEADLINE_MS` = 5000, run.rs:525; `Ops` does not override `deadline_millis`) now covers the data thread's whole stop sequence. Before the fix, step 3's join (stepping.rs:264) did that wait under its own 5 s deadline.
   - Take a device-loss Run with several Providers where every stop returns but the stops add up to more than 5 s, for example two UHD Providers at the ~3 s KC-46b cites. `termination.cleanup_failures` gains a `timed_out: true` entry for `FreezeDispatch`. Before the fix, the same Run recorded the timeout under step 3.
   - The abandoned freeze thread later runs `freeze_held` (queue clear, cancels) beside steps 3 and later. Nothing is lost or reordered: the stops are done and `done` dedups them.
   - The KC-24a Ceiling covers only a stop that "does not return". This is worth one clause when 8b brings Runs with several devices; nothing is needed now.

3. **note**: RS-7, "Step 1 precedes step 2" (design/04:146), still holds within each performance. Across performances, RS-6 now says that a later performance's cancel of a late-published handle follows the data thread's Provider stops. Every handle in `scheduled` or `wake_handle` is a no-op (the agenda placeholders at pipeline.rs:806, the horizons at stepping.rs:419, the wakes), so cancelling one after a stop has no effect. A pointer from RS-7 to RS-6 would stop a reader from seeing a contradiction. Optional.

4. **note**: `uhd_59_a_loss_after_an_overrun_stops_the_run_and_keeps_the_tail` failed in `ws-fix2c.log` and `ws-fix2d.log` with "the UHD Run's T0 drifted … from the probe's". It carries the `EZSDR-TIMING` marker (crates/ezsdr-acceptance/tests/uhd.rs:138-143). That is a start-time precondition under load, not the stop path this fix changes, and it passed in `ws-fix2.log`. The gate's serial re-run settles it. `ws-fix2d.log` (23:52) predates the commit (23:56), so the gate should run on the committed tree.

## Sol's findings

- **F1: CLOSED.** `stop_early` holds the gate from its freeze through its last `StopProviders` (paced.rs:134-157). `FreezeDispatch` takes the gate (ending.rs:335-339). The cleanup's StopProviders takes none (ending.rs:380-399), and S5 shows no cancel can be outstanding there except under RS-8a.
  - The new test `kc_24a_the_data_threads_provider_stop_does_not_overlap_a_late_horizon_cancel` (device_paced.rs:2551) replays Sol's exact sequence. H is held unpublished at `Scheduled`, the data thread is parked at `BeforeStop`, and the late cancel is parked on G_c.
  - On the unfixed shape (N111-26), its gates force `cancel:entered < radio:stop < cancel:returned`, so the test fails on an assertion (`!(entered < stop && stop < returned)`, and `stop_reading.held.freeze`), not on a timeout.
  - The skip branch is pinned by `kc_24a_an_early_stop_after_the_cleanups_freeze_performs_no_second_freeze` (device_paced.rs:2678, N111-27).
  - N111-28 (the data thread takes no gate, with its Serialise hook kept) is killed by the overlapping-freezes test.
- **F2: CLOSED.**
  - N111-22: the blocking mutant records `freeze:Serialise` on W_c, because the hook is now inside `lock_freeze` (state.rs:326-330) and `try_lock_freeze` has none. On the fixed tree, the cleanup's freeze runs on `run_cleanup`'s spawned step thread (run.rs:579), so no Serialise line names W_c. The verdict is that line; the 10 s waits are only the infra backstop (device_paced.rs:2824-2908).
  - N111-23: `BeforeWakeTake` is inside `take_wake_handle` (ending.rs:288-292). The mutant reads `held.admission == true` at the take before it blocks on the parked schedule, and fails by assertion (device_paced.rs:1788-1915).
- **F3: CLOSED** by the disposition's fallback. The residual, a read placed between `BeforeAgendaLock` and `lock_admission()`, is documented in the test (device_paced.rs:2350-2356). N111-24 kills the reachable placement.
- **F4: CLOSED.** `Serial<T>(Mutex<T>)` has a private field (state.rs:167). Outside state.rs the only access is the probe's `is_held()` (mod.rs:100); a grep of `\.admission\b|\.machine\b|\.freeze_gate\b` outside state.rs finds no raw lock. The source-scan test and N111-19 are removed, and KC-24a now says this is a compile check (design/06:332).

## Rows

- **Killers of N111-11 and N111-28**, read in `kc_24a_overlapping_freezes_serialise_and_cancel_before_the_single_stop` (device_paced.rs:1918-2030). Both fail on an assertion with no timeout on their path:
  - N111-11 (the cleanup's freeze takes no gate): the second performance's own `Serialised` reading has `held.freeze == false`, so `assert!(second_serialised.1.held.freeze …)` fires. That reading belongs to the performing thread, so no schedule changes it.
  - N111-28 (`stop_early` takes no gate): the data thread cancels the far handle without the gate, so `assert!(cancels[0].held.freeze …)` fires. A schedule-dependent backup is `returned < radio:stop`, because the cleanup stops `radio` while the data thread's cancel is parked. N111-28 also fails the new test deterministically, through `stop_reading.held.freeze`.
  - The waits on those paths (the second `Serialise`, the Manifest) are reached by both mutants, because the row's `new` text keeps the `Serialise` hook.
- **How N111-11 and N111-28 went from ERROR to killed:**
  - A plain removal of the `lock_freeze` call also removes the hook, which now sits inside `lock_freeze`. The test then never sees a second `Serialise`, and its 10 s infra backstop fires. That was the first run's ERROR in `rows-fix2.log`.
  - The re-written rows keep the hook and remove only the lock. They mutate the intended behaviour (the lock) and keep the observation. The kept hook is not an anchor added to production code, so AGENTS §9 maintainability rule 5 is not breached. The cost is that each row's `new` text names the hook; if the hook moves, the rows move with it.
- **Tests deleted or changed in device_paced.rs** (`git diff HEAD~1..HEAD | grep '^-'`). No assertion was loosened to let the fix pass:
  - The N111-23 test's `refused_while_parked` verdict became a lock-state assertion at `BeforeWakeTake`, which is deterministic. A refusal that does not arrive while G_s is closed now ends in the infra panic instead of passing.
  - `does_not_cancel_a_handle_the_freeze_took` now releases W_c on `wake:after_stop` instead of `radio:stop`. Under the new gate, `radio:stop` no longer implies the gate is free, while the wake follows `drop(gate)`.
  - The N111-22 test's unasserted wait became an asserted wait plus the W_c-`Serialise` assertion.
  - The source hunks in state.rs, mod.rs and pipeline.rs are only the `Serial` wrapper, the hook moved into `lock_freeze`, and the `BeforeStop` point.

- **Killed by assertion (verified):** N111-26 and N111-27, read in the new tests (`stop_reading.held.freeze`, the overlap order, and the data thread's `freeze:*` lines), and N111-11 and N111-28 (above). All four are logged "killed" in `rows-fix2.log` and `rows-fix2b.log`.
- **Logged killed, killer tests not re-read:** C06, G10, G29 and the other N111 rows. "Killed" is the tool's outcome for a test failure with no infra marker, signal or timeout.
- **Anchors:** C06 re-anchors on `let gate = shared.lock_freeze();`, so its program is the same as before: Stopping is published by the data thread before its freeze. G29's anchor and N111-1/N111-14's `drop(admission);` are unique in ending.rs (`rules.py` passes).
- **Removed rows:** N111-2 (a duplicate of N111-23), N111-12 (now N111-26/N111-28) and N111-19 (no longer compiles under `Serial`). Nothing else references them (grep over design, plan and crates).

## Spec text against the code

- RS-6 (design/04:134) states the hold through step 2, the residual for "a handle published after the data thread's step 1" (its cancel follows the stops), and the skip branch.
- KC-46b (design/06:442) states the gate from step 1 through the last stop and the skip.
- The KC-24a Ceiling (design/06:331) states that `Provider::stop` runs under the gate on the early stop, and the wedged-stop case: a control freeze waits behind it and its cancels are not made, and RS-8a bounds only the cleanup's steps.
- KC-46a (design/06:439) already covers the wake left scheduled after step 1, including the unconditional wake `stop_early` makes after `drop(gate)`.
- All of these match the code. The *Test:* lines and the Changes rows name the two new tests.

Verdict: PASS

## Terra, re-check of fix round 2

1. **Major — F1 is PARTLY closed: a late horizon is still canceled after Provider stop.** The original overlap is fixed: the data thread holds `freeze_gate` through its Provider stops, so another freeze cannot cancel concurrently with them. But the adopted disposition also requires preventing cancellation **after** stop, or proving it impossible.

   Minimal sequence: the control thread receives a horizon handle but pauses before publishing it → the data thread freezes and drains the handles then present → the control thread publishes the horizon → the data thread stops the Provider and releases the gate → cleanup freezes and cancels that horizon. The observable outcome is `Provider::stop` followed by `TimeAuthority::cancel`, without an RS-8a timeout. The new test deliberately permits this order. RS-6 now describes it, but that changes the stated disposition rather than satisfying it. See stepping.rs:407 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), paced.rs:143 (`crates/ezsdr-kernel/src/coordinator/paced.rs:143`), device_paced.rs:2551 (`crates/ezsdr-kernel/tests/device_paced.rs:2551`), RS-6 (`design/04-run-and-session.md:134`), and the F1 disposition (`~/.cache/ezsdr-fixes/2026-11-19-n111/decisions.md:48`).

2. **Minor — The withdrawal explanation in RS-6 and KC-29b is too strong.** They say that when the withdrawal finds the gate held, *that performance* cancels its handle. The gate can instead be held across the data thread’s Provider stops, after its freeze has drained the list. A horizon published after that drain is left for the **cleanup’s later freeze**, not the performance holding the gate. The code’s eventual ownership is consistent with the late-horizon test, but the stated reason for skipping withdrawal is inaccurate. See stepping.rs:420 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:420`), RS-6 (`design/04-run-and-session.md:134`), and KC-29b (`design/06-kernel-coordinator.md:369`).

3. **Minor — G10’s reported assertion kill still depends on a wall-clock bound.** Its test asserts that the Provider stops within ten seconds; a sufficiently delayed correct data thread produces the same assertion failure as the mutant. The recorded run reports `killed`, but that alone does not establish an operation-based assertion kill. G29’s existing test also uses timed behavior; the prior review already identified that limitation. See device_paced.rs:717 (`crates/ezsdr-kernel/tests/device_paced.rs:717`) and mutations.json:95 (`plan/phase7/tools/mutations.json:95`).

4. **Note — F2, F3 and F4 are CLOSED against their dispositions.** N111-22 observes the blocking lock helper’s entry, and N111-23 reads the admission-lock state at the wake-handle take; their verdicts no longer require a receive timeout. F3 takes the explicitly permitted fallback: the remaining placement gap is documented in the test. `Serial<T>` makes the underlying mutex private to the state module, closing F4’s alias bypass from other coordinator modules. See device_paced.rs:2887 (`crates/ezsdr-kernel/tests/device_paced.rs:2887`), device_paced.rs:1876 (`crates/ezsdr-kernel/tests/device_paced.rs:1876`), device_paced.rs:2354 (`crates/ezsdr-kernel/tests/device_paced.rs:2354`), and state.rs:166 (`crates/ezsdr-kernel/src/coordinator/state.rs:166`).

5. **Note — Remaining concurrency and verification limits.** The skip-if-frozen check occurs under the gate, so a completed cleanup freeze cannot be mistaken for one still canceling. Simulation uses cleanup’s freeze rather than the device-paced early-stop path. The spec records the wedged-stop consequence: a control freeze can wait behind the stop and its cancels may not occur; RS-8a bounds control cleanup’s steps, not the data thread’s stop. A wake published after the last freeze can remain scheduled as a no-op. See paced.rs:143 (`crates/ezsdr-kernel/src/coordinator/paced.rs:143`), KC-24a (`design/06-kernel-coordinator.md:331`), and KC-46a (`design/06-kernel-coordinator.md:439`).

   This was a static re-check; I did not run tests or mutations. The supplied mutation log (`~/.cache/ezsdr-fixes/2026-11-19-n111/rows-fix2.log`) reports kills for 23 of the 26 requested rows, including C06, G10, G29, N111-22, N111-23, N111-26 and N111-27; the follow-up log (`~/.cache/ezsdr-fixes/2026-11-19-n111/rows-fix2b.log`) reports kills for the three initially unsuccessful rows. These summaries do not show each underlying assertion, so they do not independently establish that **every** row died by an operation-based assertion.

Verdict: FAIL
## Terra, text check round 1

1. **P2 — RS-6 (`design/04-run-and-session.md:134`):** The statement that a withdrawal finding the gate held does nothing “because that performance cancels the handle” contradicts the revised KC-29b. A horizon handle can be published after the data thread’s freeze has drained handles, while the data thread still holds the gate for Provider stops. The withdrawal skips it; the cleanup’s later freeze takes it. stepping.rs (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:143`), ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:273`).

2. **P2 — KC-46a (`design/06-kernel-coordinator.md:439`):** “A wake scheduled after step 1 has passed stays scheduled” is too broad. A wake published after the data thread’s freeze can be taken by the cleanup’s later freeze, including the unconditional wake after the early stops if it is published before that freeze takes the wake slot. This conflicts with the revised RS-7’s later-performance ordering. paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:154`), ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:273`).

The changed sentences themselves match the traced behavior, subject to cancellation being an **attempt**: a callback may already have fired or `cancel` may panic. The data thread holds the gate through its early Provider stops; a withdrawal that cannot acquire it does nothing; and the later cleanup freeze takes a late-published horizon handle. The coordinator’s three schedule sites use either no-op callbacks or a wake callback that clears an atomic generation, so none calls a Provider. The changed gate description also matches KC-46b, including the case where cleanup froze first and the data thread skips its own freeze.

**Verdict: FAIL**
## Terra, text check round 2

The changed sentences need to match these code paths:

- The data thread takes `freeze_gate`, performs its freeze only if dispatch is not already frozen, and keeps the gate through its Provider stops. It then releases the gate and schedules an unconditional wake. paced.rs:133 (`crates/ezsdr-kernel/src/coordinator/paced.rs:133`)
- An early horizon withdrawal uses `try_lock_freeze`; if the gate is held, it does nothing. If it acquires the gate, it cancels only if it successfully removes the handle from `scheduled`. stepping.rs:420 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:420`)
- A horizon handle published after the data thread drains `scheduled` remains available for the cleanup’s later freeze. That freeze cannot acquire the gate until the data thread’s stops finish. ending.rs:273 (`crates/ezsdr-kernel/src/coordinator/ending.rs:273`)
- The coordinator’s scheduled agenda and horizon callbacks are no-ops; wake callbacks only clear a pending generation. None calls a Provider. pipeline.rs:805 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:805`), stepping.rs:407 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), paced.rs:164 (`crates/ezsdr-kernel/src/coordinator/paced.rs:164`)

1. **Medium — design/04-run-and-session.md:146 (`design/04-run-and-session.md:146`):** “Step 1 precedes step 2 within each performance” is false when cleanup has already frozen dispatch: the data thread skips its own step 1 and still performs its early Provider stops. It also conflicts with RS-6 and KC-46b, which explicitly state that exception. The ordering between a later freeze’s cancellation and the data thread’s stops is otherwise consistent with the gate.

2. **Medium — design/06-kernel-coordinator.md:439 (`design/06-kernel-coordinator.md:439`):** KC-46a says a wake scheduled before cleanup’s later performance is cancelled by it. The wake slot holds only one handle, and an unconditional wake can replace that handle before cleanup takes the slot; the earlier handle is then no longer held for cancellation. A wake can also fire before cancellation. The sentence needs to qualify the claim to the handle still held when the slot is taken. paced.rs:160 (`crates/ezsdr-kernel/src/coordinator/paced.rs:160`)

The revised KC-24a and state comment correctly describe the gate’s scope. The revised RS-6 and KC-29b withdrawal wording describes the late horizon-handle case, subject to RS-6’s stated abandonment exception.

**Verdict: FAIL**
## Terra, text check round 3

1. **P1 — RS-7 (`design/04-run-and-session.md:146`): “Every Provider stop … follows a completed step 1” is false.** Cleanup can time out while its step 1 is waiting for the gate or blocked in `cancel`, then start step 2 while that step 1 remains unfinished. The cleanup runner explicitly continues after a timeout (run.rs (`crates/ezsdr-kernel/src/run.rs:552`)); the control cleanup’s Provider stop does not take the freeze gate (ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:380`)). This also contradicts RS-6’s explicit RS-8a exception. Qualify the claim for an abandoned step 1.

The remaining changed statements match these code paths, subject to the documented cleanup deadline and supplied-call failures:

- **RS-6 and KC-29b:** The data thread holds the gate from its freeze through its Provider stops; a withdrawal that finds it held does nothing. A horizon handle published after that freeze is left for cleanup’s later freeze, which takes the gate after the stops (paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:133`), stepping.rs (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:250`)).
- **RS-7’s callback claim:** The coordinator’s scheduled agenda and horizon callbacks are no-ops; the wake callback only clears an atomic generation. None calls a Provider (pipeline.rs (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:805`), paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:160`)).
- **KC-24a and the state comment:** Both describe the gate covering the data thread’s early Provider stops, including when cleanup froze dispatch first. This agrees with KC-46b.
- **KC-46a:** The wake slot can be replaced before a later freeze takes it; that freeze takes the handle currently in the slot. This is consistent with the changed RS-6 wording for a horizon withdrawal.

**Verdict: FAIL**
## Terra, text check round 4

The changed sentences must match these code paths:

1. **Freeze and Provider stops.** The data thread takes `freeze_gate` before checking whether dispatch is frozen, performs its freeze if needed, and holds the gate through its Provider stops. Cleanup’s freeze takes the same gate. A cleanup freeze abandoned under RS-8a is the stated exception to waiting for it to complete. See paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:133`) and ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:250`).

2. **Withdrawal and late handles.** A withdrawal that cannot take the gate does nothing. A horizon handle published after the data thread’s drain remains in `scheduled` for cleanup’s later freeze; that freeze cannot acquire the gate until the data thread’s stops finish. A wake handle can instead be replaced in its slot before cleanup takes it, as KC-46a now says. See stepping.rs (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`) and paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:160`).

3. **Callbacks.** The coordinator’s scheduled agenda and horizon callbacks are empty; its wake callback only updates the pending generation. None calls a Provider. See pipeline.rs (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:805`), stepping.rs (`crates/ezsdr-kernel/src/coordinator/stepping.rs:407`), and paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:166`).

**Findings:** None. The changed universal claims match these paths, including the RS-8a exception and the wake-slot replacement qualification. I found no contradiction among RS-6, RS-7, KC-24a, KC-29b, KC-46a and KC-46b.

**Verdict: PASS**