# #120 + #121 — reviews

## Sol 6.2, test plan v1

Read-only review of `work/n120` at `cefd067c`. I read both issues through the GitHub connector after `gh` failed, reran the inventory searches, and checked the proposed tests and mutation rows against the current code. The worktree is clean.

1. **Major — An abandoned step 6 can still put counts in the Manifest.** Let step 6 time out while the second `drops()` is blocked. Release it after `run_cleanup` records the timeout but before `assemble_manifest` reads `link_drops`. The proposed worker writes the full vector, so the Manifest can report both a `FlushEvents` timeout and non-null counts, contrary to binding decision Q3. The test holds the gate until the Manifest arrives, which excludes this sequence. Make acceptance of the snapshot depend on successful completion of step 6 and test release after timeout but before assembly. The cleanup-step worker is detached, so the test also needs its own acknowledgement to observe its late return. Decisions:7 (`~/.cache/ezsdr-fixes/2026-11-20-n120/decisions.md:7`) · Test plan:120 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:120`) · Test plan:164 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:164`) · run.rs:565 (`crates/ezsdr-kernel/src/run.rs:565`) · ending.rs:117 (`crates/ezsdr-kernel/src/coordinator/ending.rs:117`)

2. **Major — T17 waits for the state M2 removes.** Publish the ordinary wake, then let the disconnect wake replace it under M2. The list stays at length one, so T17 waits for `wake_handles == 2` until its infrastructure bound. It never asserts that the ordinary handle received zero cancels. A skipped disconnect publication fails the same way. Use return from `disconnect_signal()` as the second wake’s synchronization point and assert both per-handle cancel counts after `finish`. T17 also names M11, which has no row. Test plan:148 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:148`) · Test plan:186 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:186`) · mod.rs:516 (`crates/ezsdr-kernel/src/coordinator/mod.rs:516`)

3. **Major — M7 survives its named killer.** In `kc_24a_freeze_cancels_the_wake_handle_exactly_once`, the early freeze takes the horizon, then the data thread publishes its unconditional wake, and the final freeze cancels that wake once. Cloning the wake list in the final freeze still produces exactly one cancel. The stale entry has no later drain. Publish an unfired ordinary wake before the early freeze, then assert its cancel count across both freezes. M7 would produce a second cancel attempt for that same handle. Test plan:196 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:196`) · device_paced.rs:2167 (`crates/ezsdr-kernel/tests/device_paced.rs:2167`) · paced.rs:133 (`crates/ezsdr-kernel/src/coordinator/paced.rs:133`)

4. **Major — T18 can time out on correct code.** Let wake 1 fire and clear `wake_pending`. Release mark 2 while the control thread is still in the round following wake 1. That round can drain the mark before the data thread sees it, so there is no second data-thread `Schedule`. T18 times out on a correct tree. Wait for a fresh control-loop park before each release, as the existing wake test does. Also observe publication of the final handle before checking the list length. A logged `Schedule` result and a cleared generation can both precede publication. Test plan:149 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:149`) · stepping.rs:146 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:146`) · stepping.rs:346 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:346`) · device_paced.rs:370 (`crates/ezsdr-kernel/tests/device_paced.rs:370`)

5. **Major — The test misses the new ownership boundary, and the proposed KC-46a text overpromises.** Let the Authority accept a wake and return its handle, then pause the publisher just before it adds the handle to the list. Run the final freeze and resume the publisher. The proposed code leaves the callback pending even though the coordinator already had the handle on its stack. RS-6 still says to cancel every callback for which the coordinator holds a handle. The test parks before the inner `schedule`, so it never exercises this sequence. Define the cutoff consistently in RS-6 and KC-46a and test it on both sides of publication. Also narrow the latency claim. `disconnect_signal()` calls its own `schedule` synchronously, and an Authority may serialize `schedule` and `cancel` internally. Test plan:112 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:112`) · Test plan:155 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:155`) · RS-6 (`design/04-run-and-session.md:134`) · start_window.rs:547 (`crates/ezsdr-kernel/tests/support/start_window.rs:547`) · mod.rs:516 (`crates/ezsdr-kernel/src/coordinator/mod.rs:516`)

6. **Major — The widened sweep can pass a forbidden Authority call.** During validation, hold a recorded state lock across `Authority::descriptor()` and start a Run. The wrapper forwards the descriptor call without logging it, so the rest of the sweep can pass. `Authority::time()` and `TimeAuthority::{primary_root,pacing,governs}` have the same gap. Record the covered methods and assert that fixtures enter the paths they cite. Keep the binding Q1 exceptions explicit. An exact lock-state assertion for the coordinator’s `DEVICE_LOST` stamp needs to identify that call separately from a Module’s use of its own time handle. Test plan:128 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:128`) · Test plan:169 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:169`) · start_window.rs:479 (`crates/ezsdr-kernel/tests/support/start_window.rs:479`) · start_window.rs:511 (`crates/ezsdr-kernel/tests/support/start_window.rs:511`)

7. **Major — Removing `BeforeWakeTake` breaks a second test.** Remove the variant as decided, retarget the paced test, then compile the `coordinator` test binary. `kc_24a_freeze_runs_cancels_outside_the_admission_lock` still names the variant in its expected trace and its lock-state assertion. The binary fails to compile. Update that test’s trace and retain its direct assertions on the locks held during cancellation. Test plan:184 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:184`) · coordinator.rs:4636 (`crates/ezsdr-kernel/tests/coordinator.rs:4636`)

8. **Minor — The inventory misses supplied calls and overstates an exception.** Plan compilation calls `Provider::instance()` and `Sink::descriptor()` through `with_inputs` while all Provider and Sink slots are held. Executor slots are outside that hold, and prepare collection calls `instance()` rather than `coerce()`. The raw inventory also lists runtime `coerce` for Module-origin submissions, which explicitly skip it. Finally, clearing `links` under its guard can invoke a supplied `DataLink` destructor if the coordinator owns the last `Arc`. Account for that site in the inventory. The proposed KC-44 sentence should explicitly preserve the Module-slot `try_lock` exception. Test plan:57 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:57`) · Test plan:85 (`~/.cache/ezsdr-fixes/2026-11-20-n120/test-plan.md:85`) · Raw inventory:58 (`~/.cache/ezsdr-fixes/2026-11-20-n120/ds-1-result.md:58`) · pipeline.rs:872 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:872`) · compile.rs:63 (`crates/ezsdr-kernel/src/plan/compile.rs:63`) · compile.rs:119 (`crates/ezsdr-kernel/src/plan/compile.rs:119`) · prepare.rs:179 (`crates/ezsdr-kernel/src/plan/prepare.rs:179`) · admission.rs:146 (`crates/ezsdr-kernel/src/coordinator/admission.rs:146`) · ending.rs:44 (`crates/ezsdr-kernel/src/coordinator/ending.rs:44`)

Verdict: FAIL
## Terra, B1 (mechanical)

A mechanical rewrite can change lock order or scope, guard drop timing, poison handling, or which acquisitions are recorded. Mutation re-anchors can target a different occurrence or produce a different mutant. A source scan can also pass while an unrecorded lock remains.

1. **P2 — The source scan can miss an unrecorded `Shared` lock** — coordinator.rs (`crates/ezsdr-kernel/tests/coordinator.rs:5110`). It looks only for the literal text `Mutex<` on field declaration lines. A field declared using a mutex type alias, for example, would pass the scan while its acquisitions remain unrecorded. The self-check exercises only the spelling the scan already recognizes. The scan also does not cover coordinator-owned locks outside `Shared`; those need an explicit scope or exclusion if the claimed coverage is broader.

The rewritten acquisition expressions preserve their surrounding scopes and order on inspection, and the wrapper retains the existing poison recovery. I found no changed guard lifetime or incorrect record assignment in the converted `Shared` fields. All 11 changed mutation rows—including two in Phase 6—match exactly once in both revisions, and their replacements are equivalent after normalizing the lock syntax.

No tests or mutation runs were executed in this read-only review; anchor matching and `git diff --check` passed.

**Verdict: FAIL**
## Opus, B2+B3

# Opus sub-step review — #120/#121 B2+B3 (fbfae8b3, diff HEAD~1..HEAD on work/n120)

Read-only. I built and ran in a /tmp copy (`/tmp/n120-review`, the shared review target).
- `cargo check -p ezsdr-kernel` (no `testing` feature).
- `cargo test -p ezsdr-kernel --test device_paced -- kc_46a kc_24a_a_wake kc_24a_sweep kc_24a_a_device_lost kc_24a_freeze_cancels`: 12/12 pass.

## Step 1 — my scenarios

How independent this is: I formed these scenarios from the prompt's list, the diff and the source (`paced.rs`, `ending.rs`, `state.rs`, `stepping.rs`, `pipeline.rs`, `run.rs`) before I opened test-plan-v2.md or decisions.md. I wrote them down only after reading both, so the wording below may be coloured by them. S12 is my own inventory, rerun from my own greps.

| # | Scenario | Result |
|---|---|---|
| S1 | Ordinary wake h1 pending, then `disconnect_signal` wake h2: h1 lost | Closed. Both go into the list (`paced.rs:183-186`). T17 asserts one cancel each. |
| S2 | A wake passes the `closed` pre-check (`paced.rs:164`). The cleanup's freeze then takes and closes the list (`ending.rs:277`). The wake then calls `now()`/`schedule` (`paced.rs:176-177`). | The code is right: the publisher sees `closed` under the list lock and cancels (`paced.rs:179-182`). **KC-46a's text is wrong here** (finding 1). |
| S3 | `schedule` has returned (`paced.rs:177`) and the publisher is descheduled before `wake_list.lock()` (`paced.rs:179`). A freeze runs in that gap. | The code is right: the decision is made under the list lock. RS-6/KC-46a's "held from the moment `schedule` returns it" does not match (finding 2). |
| S4 | A handle cancelled twice (early freeze plus cleanup freeze, or freeze plus publisher) | Not reachable. Each performance `mem::take`s the list (`ending.rs:276-279`). The publisher cancels only handles it never pushed. Pinned by N120-15 and `kc_24a_a_wake_published_before_the_early_freeze_is_cancelled_once_across_both_freezes`. |
| S5 | A handle left pending after the last freeze | Only when RS-8a abandons FreezeDispatch (the RS-8a exception), or when the publisher's `cancel` panics (finding 3). |
| S6 | `closed` racing the data thread's early freeze | Only `freeze_held(true)` closes the list, and only the cleanup's freeze calls it with `true` (`ending.rs:337`; the data thread passes `false`, `paced.rs:145`). The data thread skips its own step 1 when `frozen` is set under the gate (`paced.rs:144`), so it never freezes after the cleanup. Its end-wake (`paced.rs:155`) published after its own freeze sits in the open list. The cleanup's freeze cancels it under the gate, after the data thread's stops. Pinned by T21. |
| S7 | The publisher's lock-free cancel overlaps the cleanup's step-2 `Provider::stop`, or lands after CleanedUp (a `disconnect_signal` caller holds `Arc<Shared>`) | Reachable. Judged acceptable (note 6). |
| S8 | An Authority that serialises `schedule`, `cancel` and something `Provider::stop` needs | No cycle. The publisher holds no coordinator lock when it calls `schedule` or `cancel`, and its cancel adds no wait beyond the `schedule` it already made. The freeze's cancels only wait for that `schedule` to return. INFERRED for third-party Authorities. VERIFIED for UHD: `DeviceAuthority::cancel` only removes a queue entry under its own `schedule` mutex (`ezsdr-radio-uhd/src/authority.rs:312-322`). |
| S9 | An abandoned step 6's late `drops()` reaches the Manifest | Closed. `drops_known` (`ending.rs:120`, `:123`) blanks every link's count when the outcome has a FlushEvents failure. Pinned by N120-3 and `kc_24a_cleanup_calls_drops_with_no_lock_and_a_wedged_drops_does_not_hang_the_manifest`. |
| S10 | The Manifest of a normal Run changes | No. `FlushEvents` always returns `Ok` (`ending.rs:436-446`). A panic gave null before and still does. Only the timeout path changes: it used to block on `links`, now it gives null (decision Q3). |
| S11 | The `links` clear drops a supplied `DataLink` under the guard | Fixed. The `mem::take` guard is a temporary in a `let` and is dropped at the end of that statement (`ending.rs:45-46`). An abandoned step-6 worker drops its own `Arc` clones on its own thread, with no lock held. |
| S12 | Lock × supplied-call inventory, rerun | Searched `coordinator/*.rs` and `run.rs` for `.time.`, `host_clock.`, `authority.`, `.drops()`, `now()`, `sample_transition`, `schedule(`, `cancel(`, `emit_device_lost`, `RunStateMachine::move_to`/`new`, `Lease::*`, and the other `DataLink` methods. Nothing outside the named exceptions.<br>• Freeze gate: `cancel` (`ending.rs:285`, `stepping.rs:440`); `Provider::stop` on the data thread (`paced.rs:151`); its DEVICE_LOST `now()` (`ending.rs:398` → `pipeline.rs:1293`).<br>• Slot locks: `with_inputs` → `Authority::descriptor()` (`pipeline.rs:885`); the DEVICE_LOST `now()` under a Sink/Executor slot (`ending.rs:363`, `:430`).<br>• `admission`: `coerce` and the registered checks.<br>• `paced.rs:68-70` reads `lease_deadline` before `monotonic_millis`; the guard is a temporary.<br>• `RunStateMachine::move_to`, which calls `utc_nanos`, has no coordinator caller; only `move_to_at` is used (`state.rs:489-497`). |
| S13 | A site the sweep cannot see | As documented in `start_window.rs:376-381`: slot locks (so `descriptor()` under `with_inputs` passes unseen), `DataLink` destructors, raw locks (`store`, `Queue.state`). I found no site of production concern hidden there.<br>Every method of `Authority` (`descriptor`, `time`, `next_wakeup`, `relations`; `module_api.rs:1163-1180`), `TimeAuthority` (7 methods, `time/authority.rs:34-62`) and `HostClock` (`run.rs:252-255`) is logged by the `Watched*` impls in `start_window.rs`. VERIFIED: every impl method calls `push_call`. |
| S14 | Tests pass on a wrong implementation; rows killed only by a bound | Each N120 killer I traced dies by an assertion before any infra wait:<br>• N120-6 at `parked_calls[0].held` (`state` true);<br>• N120-12 at `!after_freeze.wake_closed`;<br>• N120-14 at `after.wake_next == 0`;<br>• N120-15 at `published.wake_handles == 1` (2 under the mutant).<br>The logged history is in note 8. |

## Findings

1. **major (spec text only) — KC-46a says something the step's own test contradicts.**
   - Where: `design/06-kernel-coordinator.md:442`: "A wake whose `schedule` returns after the cleanup's performance of step 1 took the list is cancelled by the thread that scheduled it, and after that performance no wake schedules anything".
   - Counterexample. It is reachable, and it is exactly the sequence that `kc_46a_a_wake_scheduled_across_the_cleanups_freeze_is_cancelled_by_its_publisher` (`device_paced.rs:2262`) drives. That test is named in the same Test line (`design/06:444`).
     ```
     data thread: wake(true): wake_list.lock().closed == false (paced.rs:164) → lock released → shared.now() (paced.rs:176) [parked]
     cleanup:     FreezeDispatch → freeze_held(true): closed = true (ending.rs:277), take list, cancels, Done
     data thread: now() returns → shared.schedule(at, clear) (paced.rs:177)   ← a schedule after that performance
                  → wake_list.lock(): closed → drop → cancel(h) (paced.rs:182)
     ```
   - Observable wrong outcome: the normative text promises that no `schedule` arrives after the cleanup's step 1, and the Kernel then makes one, which its own test records (`by_data[parked].kind == Schedule` after the freeze's `Done`). An Authority or Provider author who reads KC-46a may rely on that promise, for example by refusing or asserting on a `schedule` after the freeze, and would break a conforming Run.
   - Fix: "…and no wake that starts after that performance schedules anything". Plan v2 §3.1 says "a wake that starts after".
   - Why major rather than minor: this repo rates a normative sentence that is false in a reachable, tested interleaving as P1/FAIL. Precedent: n111's text re-checks failed rounds on KC-46a's "stays scheduled" and RS-7 (`plan/phase8/reviews/n111-dispositions.md:79`, `n111-reviews.md:679`, `:702`). No code change is needed.

2. **major (spec text only) — binding decision F5 is not met in RS-6 or KC-46a: the ownership cutoff is "returned", where F5 requires "published".**
   - decisions.md F5: "the ownership cutoff is defined once (a handle the coordinator holds = published to the list) in RS-6 and KC-46a".
   - RS-6 step 1 (`design/04-run-and-session.md:134`): "a handle is held from the moment `schedule` returns it; a wake handle returned after the cleanup's performance took the wake list is cancelled by its own scheduler". KC-46a (`design/06:441-442`) also keys on "returns".
   - The code decides at the list lock (`paced.rs:179-186`): published means owned by the freezes, and finding the list closed means owned by the publisher.
   - Where the text and the code disagree: the window after `shared.schedule` returns (`paced.rs:177`) and before `wake_list.lock()` (`paced.rs:179`).
     ```
     publisher: h = schedule(...) returns   [descheduled here]
     cleanup:   freeze_held(true): closed = true, take list (h not in it), cancel the rest
     publisher: lock list → closed → cancel(h) itself
     ```
     The text says h was "held" before the take and was not "returned after" it. So RS-6's main clause makes the cleanup's step 1 responsible for cancelling h, but the publisher cancels it.
     Likewise, when the data thread's freeze runs in that window, that performance does not cancel a handle RS-6 calls held; the cleanup's later performance does.
   - Origin: the implementer copied plan v2 §3.1 word for word ("the coordinator *holds* a wake's handle from the moment `schedule` returned it"; the plan's KC-46a text says the same). The conflict is between plan v2 and decision F5, not drift in implementation.
   - Fix (no code change): "a wake handle is held once it is in the wake list; one whose publisher finds the list closed is cancelled by that publisher". Use the same wording in KC-46a.
   - Why major: same precedent as finding 1, and it is a binding disposition left unmet.

3. **minor — a panic in the publisher's `cancel` is swallowed with no record.**
   - Where: `paced.rs:182` `let _ = shared.cancel(handle);`.
   - The same panic elsewhere is recorded: inside a freeze it becomes a step-1 `CleanupFailure` "KC-30: a Module panicked during cleanup: Authority cancel()" (`ending.rs:285-289`), and in a withdrawal it fails the Run (`stepping.rs:440-442`).
   - Sequence: an Authority whose `cancel` panics, and a wake whose `schedule` straddles the cleanup's performance. Nothing is recorded, and the no-op callback stays scheduled past cleanup.
   - Pushing a `CleanupFailure` would not reliably help: the cancel can happen after the Manifest is assembled, so the failure would never reach it.
   - KC-46a already leaves a panic in the wake's own `schedule` unrecorded ("clears its generation and is not retried").
   - Proportionate fix: one clause in KC-46a, for example "a publisher's cancel that panics is not recorded; its callback is a no-op".

4. **minor — the shipped build gets a new dead-code warning.**
   - `PendingWake.generation` (`state.rs:199`) is read only by `start_probe` (`mod.rs:116`), which is `cfg(feature = "testing")`.
   - VERIFIED: `cargo check -p ezsdr-kernel` prints `warning: field generation is never read`.
   - The gate's `cargo clippy --workspace --all-targets -- -D warnings` (`gate.py:332`) turns `testing` on through the kernel's self dev-dependency, so the gate never sees it.
   - INFERRED from the dependency chain (resolver 3, `Cargo.toml:2`; `testing` appears only under `[dev-dependencies]` in every crate's `Cargo.toml`; `ezsdr-server`'s normal dependency is plain, `crates/ezsdr-server/Cargo.toml:15`), not built here: `cargo build -p ezsdr-server`, the py314 step, compiles the kernel without `testing` and prints the warning.
   - Fix: `#[cfg(feature = "testing")]` on the field (the probe is its only reader), or `#[cfg_attr(not(feature = "testing"), allow(dead_code))]`.

5. **note — KC-24a and KC-44 wording.**
   - KC-24a's first sentence (`design/06:330`) bans every supplied call under "a lock of its own other than a Module's slot lock". The gate's exception comes two sentences later, and the Ceiling (`:331`) adds `admission` held across `coerce` and the checks.
   - KC-44's new reason (`design/06:427`, "the coordinator holds no lock of its own across such a call (KC-24a)") is therefore false as written. The conclusion holds anyway, because assembly takes neither `admission` nor the gate.
   - Suggest: "…other than a Module's slot lock and the exceptions below" in KC-24a. In KC-44: "the assembly takes neither the admission lock nor the freeze gate, holds no other lock across a supplied call (KC-24a), and takes a Module's slot only with `try_lock`".

6. **note — the one cancel outside the gate, which decisions.md asked reviewers to check.**
   - It can overlap the cleanup's step-2 `Provider::stop`. It can also land after `CleanedUp` and the Manifest, when a `disconnect_signal` caller's `schedule` returns late; that caller holds `Arc<Shared>`, so the Authority stays alive.
   - RS-6's "step 2 does not start while a cancel that step 1 took, or a withdrawal's cancel, is outstanding" excludes this cancel by its wording.
   - RS-7 orders only "a later performance's cancel", and its last clause holds: the wake's callback touches two atomics only (`paced.rs:171-175`).
   - Deadlock: see S8.
   - The design is acceptable. Consider adding to KC-46a: "that cancel may overlap step 2 or follow cleanup".

7. **note — two new tests have a hidden 5 s RS-8a wall-clock bound on their verdict path.**
   - T20 (`device_paced.rs:3237`): the data thread is parked in `on_schedule_return` while cleanup step 3 joins it, under the default 5 s deadline. If more than 5 s pass before `open(&park)`, step 3 is abandoned, and the final `cleanup_failures.is_empty()` fails as a plain assertion, not as an infra error.
   - T22: FreezeDispatch, parked at `Done`, runs under the same deadline, but there an overrun surfaces as the 10 s infra error instead.
   - The window is milliseconds in practice. Name the bound in the Deschedule comments.

8. **note — mutation history.**
   - In the first B3 run (b3-mut.log, 02:05, 14 shards), N120-12 and N120-15 ended ERROR (infra marker) and N120-14 SURVIVED.
   - All three were killed in the reruns (b3-mut2.log, b3-mut3.log, on 3–4 shards) after the tests changed. From my reading of the committed killers (S14), each now dies by an assertion.
   - The full run (full.log, started 02:23, 16 shards) had no result when I reviewed. Confirm those three rows there before the gate.

9. **note — maintainability.**
   - (a) The `on_schedule_return` closure is the same in T20 and T21 (`device_paced.rs:3253`, `:3303`). A small helper next to `on_data_thread()` (`:3104`) would remove the copy.
   - (b) The source scan's prefix allow-list (`NON_LOCK_FIELDS`, `coordinator.rs:5112`, used by `unrecorded_fields` at `:5118`) accepts any type that starts with `"Context"` or `"OnceLock<"`, for example `OnceLock<Mutex<_>>`. Listing the exact `OnceLock` types would close that.
   - (c) Good reuse: `two_link_docs()`, `Threads::wait_reading`, `wake_session`.
   - (d) The N111-23 row and `FreezePoint::BeforeWakeTake` were removed together with the take they observed. My grep for `wake_handle\b|BeforeWakeTake|…_is_not_cancelled|…_with_the_admission_lock_free` finds them only in historical records.

## Against the binding decisions

- D1–D5 are implemented as stated.
- F1, F2, F3, F4, F6 and F7 each have a test that observes what the plan names. F6's "every Authority method logged" is VERIFIED (S13).
- F5 holds in the code but not in the spec text (finding 2).
- F8's `links` clear now runs outside the guard.
- Q1–Q4 are as decided.
- Nothing beyond the plan's scope was added.

**Verdict: FAIL.** The cause is findings 1 and 2: normative spec text only, with no code change needed. A narrow text re-check after the fix is enough. The code and tests pass my scenarios.

## Terra, B2+B3

1. **Major — A late wake cancellation can deadlock with Provider stop.** The publisher cancels outside the freeze gate at paced.rs (`crates/ezsdr-kernel/src/coordinator/paced.rs:179`), while cleanup can already be calling `Provider::stop` at ending.rs (`crates/ezsdr-kernel/src/coordinator/ending.rs:379`).

   Minimal sequence: a Provider thread schedules a disconnect wake and pauses after `schedule` returns; cleanup closes the list and starts `stop`; `stop` holds a Provider mutex and needs the Authority; the publisher resumes, enters `cancel`, holds the Authority’s serialization mutex and waits for that Provider mutex. Stop and cancel now wait on each other. The callback’s lack of Provider calls does not prevent this cycle. RS-8a can abandon cleanup steps, but the Provider and publisher remain wedged, and their resources may remain live. The test at device_paced.rs (`crates/ezsdr-kernel/tests/device_paced.rs:3237`) establishes the late cancellation but does not exercise this overlap with a serializing Authority and Provider stop.

2. **Note — Mutation evidence needs the final gate’s reconciliation.** The recorded first run reported N120-12 and N120-15 as infrastructure errors and N120-14 as survived; later targeted runs reported all three killed. I did not rerun mutations or tests. The final gate should account for the current source and all rows; the earlier errors are not kills.

I reran the coordinator lock and supplied-call searches and traced the changed wake and Manifest paths. The `drops()` snapshot is copied outside `links`, and assembly rejects it when step 6 failed. The sweep records calls it reaches, but, as the plan acknowledges, cannot prove unexercised paths or observe supplied destructors.

Verdict: FAIL
## Sol 6.2, code (cefd067c..465e5a9a)

Reviewed `work/n120` at `465e5a9a`, using `cefd067c..HEAD`. The range contains three commits after the base. I traced the lock and callback paths, both freezes, and step 6 through Manifest assembly. All 16 `N120-*` anchors matched once. The review was read-only and static, so test outcomes and mutation kills remain unverified.

1. **Major — A Simulation step fault calls the Authority with an unrecorded coordinator lock held.** Start a Simulation Run whose stepped Provider returns `DeviceLost`. `round` takes the fault vector in the argument to `apply_faults`, so the temporary `MutexGuard` lives through the call. The resulting `DEVICE_LOST` stamp enters supplied `now()` with the fault mutex still held, contrary to KC-24a. The sweep would report this call as unlocked because the fault mutex is raw and the source scan excludes it. Move the fault vector out in a separate statement and drive this path with a watched Authority. stepping.rs:179 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:179`) · pipeline.rs:1293 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1293`) · coordinator.rs:5135 (`crates/ezsdr-kernel/tests/coordinator.rs:5135`) · KC-24a (`design/06-kernel-coordinator.md:330`)

2. **Major — Horizon withdrawal can re-enter wake scheduling under the freeze gate.** In a live Attached paced Session, satisfy `wait_for` before its horizon. Have the supplied `cancel` synchronously deliver the exported disconnect signal while holding its timer mutex. The signal wins the end request and calls `wake` on that same thread, which still holds the freeze gate. If `schedule` takes the same timer mutex, `wait_for` deadlocks outside RS-8a. Even if the supplied mutex is released before the signal, the nested `now()` and `schedule()` still run under the gate, outside KC-24a’s exceptions. This sequence uses an embedder-supplied Authority. stepping.rs:431 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:431`) · mod.rs:549 (`crates/ezsdr-kernel/src/coordinator/mod.rs:549`) · paced.rs:164 (`crates/ezsdr-kernel/src/coordinator/paced.rs:164`)

3. **Minor — The #121 test releases the wedged link before it tests Manifest assembly.** The `CleanedUp` clock hook opens the wedge and waits for the late snapshot before `assemble_manifest` runs. This exercises rejection of late counts. A Manifest that waits for the abandoned call through an unrecorded lock or completion signal could still pass. Add a separate sequence that keeps `drops()` parked until the sealed Manifest arrives. coordinator.rs:5215 (`crates/ezsdr-kernel/tests/coordinator.rs:5215`) · ending.rs:41 (`crates/ezsdr-kernel/src/coordinator/ending.rs:41`)

4. **Minor — N120-8 and N120-9 can get credit for an unrelated timeout.** Their test has a direct list-length assertion, but its three preceding 10-second waits fail without `EZSDR-INFRA:`. A worker delayed at one of those waits can produce an ordinary test failure that the runner counts as a kill before the list length is read. Mark those waits as infrastructure failures. device_paced.rs:3224 (`crates/ezsdr-kernel/tests/device_paced.rs:3224`) · mutations.json:1887 (`plan/phase7/tools/mutations.json:1887`) · mutate.py:179 (`plan/maintenance/tools/mutate.py:179`)

5. **Minor — Three descriptions still prescribe cancelling a late wake.** The RS-6 changes table and both implementation comments describe publisher-side cancellation. The normative text and implementation leave a wake whose publisher finds the list closed scheduled as a no-op. Align the three descriptions with that cutoff. 04-run-and-session.md:480 (`design/04-run-and-session.md:480`) · state.rs:206 (`crates/ezsdr-kernel/src/coordinator/state.rs:206`) · ending.rs:274 (`crates/ezsdr-kernel/src/coordinator/ending.rs:274`)

Verdict: FAIL
## Opus, re-check of fix round 2

# Opus re-check of #120/#121 fix round 2 (cbd90187 on work/n120; step cefd067c..HEAD)

Read-only. I wrote my own scenarios before I read sol-code.md and decisions.md; they are listed below. I built and ran tests in a /tmp copy (`rsync --no-times`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`):
- `cargo check -p ezsdr-kernel` with no features: clean.
- `cargo clippy -p ezsdr-kernel --all-targets --features testing`: no warning or error lines.
- The new and changed tests all pass: `kc_24a_a_cancel_that_calls_disconnect_signal_schedules_nothing_under_the_gate`, `kc_24a_sweep_a_simulation_device_lost_stamps_its_event_under_no_lock`, both wedged-`drops()` tests and `kc_24a_every_shared_lock_is_a_recording_lock`.
- Mutation results are from the recorded `fix2-mut.log` and `fix2-mut-p4.log`. I did not run them again.

## Step 1: my own scenarios (written before reading the review)

- **S1. A skipped wake with nobody to re-read the end request.** `wake` returns at once when the thread holds any recorded lock (`paced.rs:168`). These threads can hold one:
  - **Freeze gate, control loop, during the horizon withdrawal.** After the gate is released, `settle()` re-reads `end` (`stepping.rs:433-446`). OK.
  - **Freeze gate, the cleanup's `FreezeDispatch` on a per-step worker.** `run.rs:579` spawns the worker, and the worker takes the gate itself (`ending.rs:336`), so the count sees it. An end is already set, so `request` returns false and no wake is tried. OK.
  - **Freeze gate, the data thread's early stop.** An end is already set here too, and the unconditional wake comes after `drop(gate)` (`paced.rs:155-156`). OK.
  - **Admission lock, a Module thread's submission** (`Submitter::submit`, `pipeline.rs:1223`) or an Executor's submission from inside its step on the data thread. `Provider::coerce` (`admission.rs:202-204`) and the registered checks run under that lock. If one of them fires the signal, `request` returns true, the wake is skipped, and that thread never reads `end`. **This becomes finding 1.**
- **S2. A held count that leaks.**
  - A panic unwinds through `Recorded::drop`, so the count comes back down.
  - `Recorded` wraps a `MutexGuard`, so it is `!Send` and cannot cross threads.
  - The coordinator calls `mem::forget` nowhere (`grep -n forget crates/ezsdr-kernel/src/coordinator/*.rs`: no hit).
  - `try_lock` is recorded the same way as `lock` (`state.rs:186-188`).
  - The counts are per kind, so the order in which guards drop does not matter.
  - The thread-local is a `const` `Cell` with no destructor, so `with` cannot fail during thread teardown.
  - No leak.
- **S3. Hot-path cost.** Each `Locked::lock` adds one const-TLS read and write, and each `wake` adds one read. Negligible next to the mutex itself.
- **S4. Slot locks are not counted (Q1).** A signal fired under only a slot lock still schedules. The Ceiling's slot-lock exception covers this. Taking `round`'s slot guards in their own scope keeps them out of `apply_faults` (`stepping.rs:160-182`).
- **S5. Other ways a supplied call can re-enter.**
  - An Authority's `cancel` that hands the signal to another thread and waits for it: that thread's count is 0, so it schedules while the gate holder waits. This is the Authority waiting on itself.
  - A supplied `schedule()` or `now()` called with no coordinator lock held (for example `advance_to`'s `schedule`, `stepping.rs:409`) that fires the signal re-enters itself.
  - Neither involves a coordinator lock, so KC-24a does not cover them (note 2).
  - `drops()` is copied out before it is called (`ending.rs:426-428`). The links' destructor runs after the guard (`ending.rs:44-45`).
- **S6. Normal wakes.** The data thread's two call sites hold no recorded lock: the `lease_deadline` guard is a temporary in a `let` (`paced.rs:68`), and `drop(gate)` comes before `wake` (`paced.rs:155`). So no wake that was needed before is skipped now.
- **S7. Tests and rows.**
  - **N120-18:** the mutant makes `Now`/`Schedule` run on W_c with `freeze` held, and the test's loop over every call asserts against that.
  - **N120-17:** `Now` runs under `Fault` (`Which::State`), which the sweep flags.
  - **N120-19:** this is N120-1's mutant (finding 4).
  - **D25:** re-anchored to the same program.
  - The new parked test has one vacuous assertion (note 3).

## Findings

1. **minor — The rule's promise that "the holder re-reads the end request" is not true for every thread that can hold a coordinator lock.**
   - **Where the claim is made:** `paced.rs:164-166`, KC-24a's new sentence (`design/06-kernel-coordinator.md:330`: "…schedules nothing, and that thread reads the end request after it releases the lock"), and the F2 disposition in decisions.md ("the holder is the control loop … or the data thread's early stop").
   - **The case it misses:** the admission lock is also held by a Module thread's submission (`pipeline.rs:1223`) and by the data thread when an Executor submits inside its step. `Provider::coerce` (`admission.rs:202-204`) and the registered checks run under it.
   - **Sequence:**
     1. A paced Run. The control loop is parked in `next_wakeup` for a horizon one hour out.
     2. A Provider thread submits an `UpdateParameter`, which takes the admission lock.
     3. The Provider's `coerce` calls a `disconnect_signal()` closure that the embedder wired in.
     4. `request` returns true and `wake` returns at `any_held()`.
     5. The Module thread returns without reading `end`. If the data thread delivers nothing, the control loop sleeps until its horizon, and the client disconnect is not served until then.
   - **Why minor, not major:** no Module or Vocabulary can obtain the closure; only the embedder can wire it in. The one real caller, the server's watch on the input thread (`ezsdr-server/src/transport.rs:59`), holds no coordinator lock.
   - **Lean fix:** wording only. Say that a wake on such a thread is skipped and the control loop sees the end at its next wakeup, as KC-46a already says for a refused wake. Name the holders that do re-read: the withdrawal through `settle()`; the two freezes set no new end. Do not add a "wake owed" mechanism.

2. **note — Re-entry outside the coordinator's locks.** Two cases are outside KC-24a:
   - An Authority whose `cancel` fires the signal on another thread and waits for that thread.
   - A supplied `schedule()` or `now()` that fires the signal synchronously while it holds its own timer mutex (`advance_to`, `stepping.rs:409`, no coordinator lock).

   Either can still wait on itself. That is the supplier's own re-entrancy. No action; mention it only if the Ceiling should list it.

3. **note — A vacuous assertion.** `coordinator.rs:5337-5340` compares `assert_eq!(manifest, sealed)` against a clone of an owned, already-returned value, so nothing can make it fail. The real check is `still_parked` together with the `[None, None]` assertion. Remove the comparison, or say it documents intent only.

4. **minor — N120-19 repeats N120-1's mutant and is killed only by the bound.**
   - Its `old`/`new` are byte-identical to N120-1 (`plan/phase7/tools/mutations.json`, ids N120-1 and N120-19). N120-1 already kills that mutant by assertion, and so does the early-release test's no-lock check on `drops()`.
   - The new parked test checks liveness, so a bound kill is honest for it. But brief-fix2 said "Every new or retargeted row dies by assertion", and this deviation is not recorded.
   - Either drop N120-19 as a duplicate (maintainability rule 5) or keep it and record the deviation in the dispositions. Orchestrator's call.

Checked and found clean:
- F1's fix: the `let` releases the temporary guard before `apply_faults`.
- The per-thread count's symmetry (S2).
- The hot-path cost (S3).
- The no-`testing` build.
- The KC-46a text, which agrees with `paced.rs`.
- The `struct Fault(Locked<` scan, which closes the fault-vector blind spot.

## Sol's findings

| Sol | Status | Evidence |
|---|---|---|
| F1 Simulation `DEVICE_LOST` `now()` under the fault mutex | **CLOSED** | `stepping.rs:19` (`Fault(Locked<…>)`), `stepping.rs:182-184` (taken out in its own statement); `kc_24a_sweep_a_simulation_device_lost_stamps_its_event_under_no_lock` (`coordinator.rs:5112`) asserts the event was delivered, so the sweep cannot pass vacuously; the scan check at `coordinator.rs:5177-5178`; N120-17 killed |
| F2 a `cancel` that calls `disconnect_signal` under the gate | **CLOSED** for the sequence Sol gave; the residual is finding 1 (wording) and note 2 | `paced.rs:168-170`; `record.rs` counts in every build; `device_paced.rs:2844` asserts the first cancel ran under the gate, that no other call ran under any lock, and that the end was served as a client disconnect; N120-18 killed |
| F3 Manifest assembly with `drops()` parked | **CLOSED** | `coordinator.rs:5323` (the Manifest arrives while the call is still parked, `FlushEvents` timed out, every `drops` null); N120-19 bound kill (finding 4) |
| F4 infra markers before N120-8/9's assertion | **CLOSED** | `device_paced.rs:3293`, `3295`, and the `returned` wait beside them now go through `infra(…)` |
| F5 three descriptions of a late wake | **CLOSED** | `design/04-run-and-session.md:480`, `state.rs:206`, `ending.rs:274`; `grep -rn "cancels itself\|cancels its own handle\|cancelled by its scheduler" crates design`: no hit |

No major found. Finding 1 asks for a one-clause wording change, and finding 4 asks for a recorded decision.

Verdict: PASS

## Terra, re-check of fix round 2

1. **Major — A Module-thread callback can suppress the only wake for its own Run.** A Module thread can submit an update while the control loop waits in `next_wakeup`. Submission holds the admission lock while calling the Provider’s `coerce`; if that supplied method synchronously calls the Run’s disconnect signal, the signal records the end request, but `wake` returns because the thread holds a recorded lock. The submitting thread then returns without settling the Run, and the data thread does not treat an already-recorded disconnect as its own new end request. With no other delivery, the control loop can remain asleep until its horizon instead of promptly cleaning up. The assumption that every lock holder rereads the end request is therefore false. See pipeline.rs:1223 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1223`), admission.rs:202 (`crates/ezsdr-kernel/src/coordinator/admission.rs:202`), mod.rs:511 (`crates/ezsdr-kernel/src/coordinator/mod.rs:511`), and paced.rs:167 (`crates/ezsdr-kernel/src/coordinator/paced.rs:167`).

2. **Major — A callback holding one Run’s lock can lose another Run’s wake.** The held count is per thread, with no Run identity. For example, while Run A withdraws a horizon under its freeze gate, its supplied `cancel` can synchronously invoke Run B’s disconnect signal. B records the request, but the thread sees A’s held lock and suppresses B’s wake. A rereads **A’s** end request; it does not service B. If B’s control loop is parked and no other wake arrives, B remains running until a later wake or horizon. The same issue can arise on a cleanup worker, including a worker abandoned under RS-8a that later invokes a signal. See record.rs:9 (`crates/ezsdr-kernel/src/coordinator/record.rs:9`), stepping.rs:433 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:433`), and design/06:330 (`design/06-kernel-coordinator.md:330`).

3. **Minor — N120-19 can credit an unrelated infrastructure timeout as a kill.** The row declares `kill: bound`, while its test helper uses the same infrastructure marker for a 30-second Manifest wait. The runner treats *any* infrastructure marker on a bound row as `killed (bound)`, so an unrelated delay can count without showing that the mutated lock blocked Manifest assembly. See mutations.json:1997 (`plan/phase7/tools/mutations.json:1997`), coordinator.rs:5276 (`crates/ezsdr-kernel/tests/coordinator.rs:5276`), and mutate.py:186 (`plan/maintenance/tools/mutate.py:186`).

4. **Note — Sol’s findings:** **F1 CLOSED**: the fault vector is taken in a separate statement, recorded, and exercised by a watched Simulation fault test. **F2 PARTLY**: the reported same-Run horizon-withdrawal sequence is fixed and tested, but the broader suppression introduces findings 1–2. **F3 CLOSED**: a second test keeps `drops()` parked through Manifest arrival. **F4 CLOSED**: the three waits carry infrastructure markers. **F5 CLOSED**: the cited changes row and comments now describe the no-cancel cutoff.

5. **Note — Remaining review limits.** Guard destruction balances the count during ordinary unwinding; `MutexGuard` prevents moving the recorded guard between threads, and successful `try_lock` records it. The slot-lock exception is deliberately unrecorded. The source scan covers `Shared` and explicitly checks `Fault`, rather than proving that every future coordinator-local mutex is recorded; the sweep also does not observe supplied destructors. N120-17/18 target the two reported paths, and D25 retains its fault-order mutation; their execution and the hot-path cost of recording were not independently measured in this static re-check. No tests or mutation rows were run.

Verdict: FAIL
## Terra, text check 1

1. **Major — design/06-kernel-coordinator.md:330 (`design/06-kernel-coordinator.md:330`), paced.rs:164 (`crates/ezsdr-kernel/src/coordinator/paced.rs:164`).** The new sentence is not true for every lock holder. The code establishes these cases:

   - `disconnect_signal` calls `wake` only when its orderly disconnect request **sets** the end on a device-paced Run (mod.rs:503 (`crates/ezsdr-kernel/src/coordinator/mod.rs:503`)). `wake` then schedules nothing if the **calling thread** holds a recorded coordinator lock (record.rs:19 (`crates/ezsdr-kernel/src/coordinator/record.rs:19`), paced.rs:167 (`crates/ezsdr-kernel/src/coordinator/paced.rs:167`)).
   - On the **control thread**, a supplied call under the admission lock can set the request during agenda dispatch. The loop may see it at its next end check **before calling** `next_wakeup` again (stepping.rs:313 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:313`), stepping.rs:376 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:376`)). A withdrawal cancel can likewise signal under the freeze gate, after which `settle` sees the request without another wakeup (stepping.rs:426 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:426`)).
   - A **Module thread**, including a submission made on the data thread, can call supplied admission logic under the admission lock (pipeline.rs:1223 (`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1223`)). If that logic signals, no replacement wake is scheduled. A control loop already waiting can observe the end only when some other wakeup occurs; this path does not guarantee one.
   - During **cleanup**, an end is already set before cleanup begins (ending.rs:22 (`crates/ezsdr-kernel/src/coordinator/ending.rs:22`)). A disconnect from a cleanup worker’s cancel therefore does not set it or call `wake`; the same applies to the data thread’s early freeze and Provider stops, which follow an existing end request (paced.rs:133 (`crates/ezsdr-kernel/src/coordinator/paced.rs:133`)). There need be no subsequent control-loop wakeup.

   The wording should distinguish observation at an existing control-loop check from observation **if and when** a waiting loop next wakes.

2. **Major — design/06-kernel-coordinator.md:330 (`design/06-kernel-coordinator.md:330`).** KC-46a does say that an Authority-refused wake clears its generation, is not retried, and leaves the control loop to see state “at its next wakeup” (design/06:443 (`design/06-kernel-coordinator.md:443`)). That describes a failed schedule; it does not establish that every lock-held signal reaches another wakeup. KC-36 also says a signal that sets the end wakes the loop and that the in-progress call’s wait returns (design/06:400 (`design/06-kernel-coordinator.md:400`)). The newly explicit skipped-wake case qualifies that promise without reconciling it. I found no contradiction with RS-6’s freeze and cancellation ordering; the issue is the claimed observation and wake guarantee.

**Verdict: FAIL**
## Terra, text check 2

1. **Minor — KC-46a describes a refused wake too narrowly** (design/06-kernel-coordinator.md:443 (`design/06-kernel-coordinator.md:443`)). It says the control loop sees the state “at its next wakeup.” If the Authority refuses a wake while the control thread is already running, the thread can reach an end check and settle without another `next_wakeup` call. Conversely, if it is waiting, another wake is needed and is not guaranteed. Describe a refused wake using the same next-end-check distinction as KC-24a.

The code basis for the changed sentences is:

- `disconnect_signal` requests the end and calls `wake` only when that request sets the end and the Run is device-paced (mod.rs:503 (`crates/ezsdr-kernel/src/coordinator/mod.rs:503`)). `wake` returns without scheduling if the **calling thread** holds a recorded coordinator lock; the counts are thread-local (paced.rs:168 (`crates/ezsdr-kernel/src/coordinator/paced.rs:168`), record.rs:9 (`crates/ezsdr-kernel/src/coordinator/record.rs:9`)).
- The control loop checks for an end before `next_wakeup`; after a wake it can run a round before the next check, or leave the loop and check while settling (stepping.rs:313 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:313`), stepping.rs:423 (`crates/ezsdr-kernel/src/coordinator/stepping.rs:423`)). A signal issued while the loop waits does not itself release that wait when scheduling is skipped.

That distinction holds regardless of whether the lock holder is the control thread, data thread, a Module thread in admission, or a cleanup worker. The new KC-24a and `wake` wording correctly says what happens **if the next end check is reached**, without guaranteeing a wake. KC-36’s new exception agrees with the guard, and RS-6’s cancellation and freeze rules do not contradict it. KC-46a’s “unconditionally” describes bypassing the pending-wake suppression, subject to its preceding lock and closed-list exceptions.

**Verdict: FAIL**