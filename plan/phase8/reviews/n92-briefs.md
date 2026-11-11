# #92 — briefs

## Brief — #92 phase 1: the test plan only (no code)

Work tree: **/home/komatsu/local_work/Ez-SDRv4-n92** (git worktree, branch `work/n92` from `main` `1371ffa`). Read only there; write only the plan file below. Read AGENTS.md (§3, §9: this is a **risky** step — it changes how a Run ends — so the test plan comes before any code, and Sol reviews it), `gh issue view 92`, design/18-uhd-radio.md UR-16 (it limits `DEVICE_LOST` on a panic to uhd-rx/uhd-tx), UR-29, UR-33, the MA-9a rule it cites (search design/), KC-21a and KC-30 (design/06), `crates/ezsdr-radio-uhd/src/provider/mod.rs` (`spawn` ~:362–388, `stop`, `join`), `core.rs` (`device_lost` ~:1051–1073, `reject_note`), `control.rs` (the uhd-control loop), and the existing panic/loss tests (`grep -rn "panicked\|device_lost\|DEVICE_LOST" crates/ezsdr-radio-uhd`).

## Direction to plan for (owner to confirm; do not treat it as decided)

A uhd-control panic is handled like a uhd-rx/uhd-tx panic: recorded in `rejected`, and `device_lost("a Provider thread panicked")` — one fatal `DEVICE_LOST`, the streams ended, the device marked lost — so the Run ends at once instead of after KC-21a's 5 s, and no repeat keeps playing. UR-16's text is amended accordingly. If you find a reason this is wrong (e.g. a lock the panic can leave held so `device_lost` deadlocks, or a stream uhd-control owns at that moment that `device_lost` cannot end, or a better event kind), say so with evidence; do not silently plan another direction.

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/test-plan.md)

For each sequence: the test, what it asserts, how it observes it (events, Manifest sections, `device.calls()`, FakeDevice state), and the wrong behaviour that would fail it. At least:
1. uhd-control panics while a repeat plays → one `DEVICE_LOST` (fatal), the transmit stream ended (EOB/close as a loss does today), `rejected` names `uhd-control` panicked, the Run ends promptly (not via KC-21a's 5 s), no further send.
2. uhd-control panics while booking a Stop / with Actions queued → those Actions' outcome (undelivered/ended), no hang.
3. A panic while uhd-control holds the streams lock or the device gate (if reachable) → `device_lost` still completes (poisoned locks are recovered by `lock`): no deadlock; bounded by a test watchdog.
4. A uhd-tx panic and then a uhd-control panic (and the reverse) → exactly one `DEVICE_LOST` (the second finds `lost` set).
5. uhd-control panics during `Provider::stop`/`cleanup` (already stopping) → no second end, `stop`'s joins still bounded, cleanup's `close_streams` rule (UR-16) unchanged.
6. No streams running → `DEVICE_LOST` and the Run ends.
How a panic is induced: a test seam behind `#[cfg(test)]` or the existing `test-support` mechanisms (name it; no production-only hook). Every test must not depend on when a thread runs (plan/phase8/37-test-thread-timing.md): use the manual clock and observed records.
Also list: the mutation rows to add (Rust token anchors, killers), the design text to amend (UR-16 and any other rule that says only uhd-rx/uhd-tx), hardware tests that rely on the current rule (AGENTS §9 audit table), and the Mock's counterpart (MockRadio has no threads: say whether anything there must change — likely not).

Rules: no code edits, no builds in the tree (if you must probe, a copy outside the repository via `rdev.py --host sim03`; sim02 may be used by another task), no commit. Final message: a summary of the plan and any reason the direction looks wrong.

## Brief — #92 phase 1b: revise the test plan after Sol's review (no code)

Same tree and rules as brief-plan.md (/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/; read it again). Read Sol's review sol-plan.md and the orchestrator's adjudication plan-adjudication.md in the same directory. Revise test-plan.md in place (keep a "Changes after Sol's review" section at its end listing each change):
- Sol majors 1–5 exactly as Sol states the changes (no device-time waits while a participant blocks on a std primitive; the external `bounded` watchdog around every deliberately blocking scenario including teardown; the Policy `continue` repeat-safety case; a full-coordinator gated booking-panic test of KC-21a's pending-submit path; an RF oracle that separates in-flight from newly initiated work and accounts for buffered samples, with no promise of immediate silence).
- The adjudicated answers to A, B, C and Policy `continue`.
- Sol's smaller corrections (T2a instants, N92-3's killer, N92-8, the "Authority-panic injection" label).
Final message: a summary of the changes and anything you disagree with (with evidence).

## Brief — #92 phase 1c: fold the Opus re-check into the test plan (no code)

Same tree and rules as brief-plan.md (/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/). Read plan-recheck.md (Opus: pass with one major and eight minors). Amend test-plan.md in place, adding a section "Changes after the Opus re-check":
- Major M1: T4a/T4b wait (inside `bounded`) until every running thread has finished before `stop`; T5 books the end as `stop` does (`test_support::end_streams`), releases the body, waits for it to finish, then calls `stop`; T9b uses the direct-end variant. No test may race `stop`'s 1 s wall-time join bound.
- Section 2 of the re-check: write the plan row in `Provider::stop` after `join("uhd-control")`; delete `Control::finish`'s write (and `control/differential.rs:241-243`'s caller handling); decide the failed-start case (recommend: only if uhd-control was spawned) and name the `ur_15_*` tests to check; drop T9c; T9b is the killer of "finish still writes"; re-anchor N92-7/7b; list the stale texts (`mod.rs:692-693`, design/18:443, design/18:56).
- Section 3: UR-16 wording fixes (streams ended and no new device call under every Policy; under `continue` the next submit to that radio fails after KC-21a's budget; uhd-control in the detached-thread list). KL-2 is NOT amended (it describes a dead link): plan for "a separate KNOWN_LIMITATIONS entry or none — owner's call", with a draft of the separate entry.
- Minors m1–m8 as the re-check states (N92-8's killers and a row of T8's own; T2c as one Session entry and its return value; Oracle Q from `device.calls()`; S-recv before the `Pushed` lock and keyed on the call after A1; T2a with a clocked transmit stream; the unit-test wiring of `Running` and the two fakes; N92-1's real killers; S-dev before `FakeDevice::time_now`).
Final message: a short summary.

## Brief — #92 implementation

Work tree /home/komatsu/local_work/Ez-SDRv4-n92 (branch `work/n92` from `main` `1371ffa`). Read: brief-plan.md (its context), test-plan.md (final, with "Changes after Sol's review" and "Changes after the Opus re-check" — it binds), plan-adjudication.md, plan-recheck.md and decisions.md, all in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/. The owner adopted the direction (2026-11-12): a uhd-control panic is handled like uhd-rx/uhd-tx — the `rejected` row and `device_lost("a Provider thread panicked")`, one fatal DEVICE_LOST; UR-16 amended as the plan says.

Implement exactly the plan: the code change in `provider/mod.rs` `spawn`, the UR-30 plan row moved to `Provider::stop` after `join("uhd-control")` (and `Control::finish`'s write removed, `control/differential.rs` adjusted, the failed-start case as the plan decides), the tests T1–T9 as amended with their seams behind `#[cfg(test)]`/test support only, the mutation rows N92-* (and P276 removed), the design text (UR-16, UR-30, design/18:56, :443, `mod.rs:692-693` comments). **Do not edit KNOWN_LIMITATIONS.md**: the owner is deciding whether to add an entry; put your draft entry in the report instead.

Rules: builds and tests only via `python3 plan/maintenance/tools/remote/rdev.py --host sim02 -- <cmd>` from the worktree (long explicit shell timeouts; foreground; nothing left running). Every blocking scenario under the external `bounded` watchdog; no test depends on when a thread runs (plan/phase8/37-test-thread-timing.md) and none races `stop`'s 1 s wall-time join bound (plan-recheck M1). Never run mutate.py/remote_mutate.py or apply a mutant in the tree; check each new row's killer in a copy outside the repository. `cargo fmt --all` then `--check`; `rules.py --base 1371ffa` 0 problems; `rdev --host sim02 -- cargo test --workspace` once at the end; new files 644; no commit.

Report to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/report.md (final message: its summary): the changes (file:line), each test and what it observes, rows and checked killers, design text changed, the KL draft, every behaviour change, and anything in the plan that proved wrong (if the plan proves wrong on a point that changes the user-visible contract, stop and report instead of improvising).

## Brief — #92 minor fixes after Sol's code review

Work tree /home/komatsu/local_work/Ez-SDRv4-n92 (branch `work/n92`, your change committed). brief-impl.md's rules bind (rdev `--host sim02` only; long shell timeouts; no commit; no mutants in the tree — apply mutants only in a copy outside the repository and run their killers there through rdev; do not use any script that runs rows in place of `remote_mutate.py`). Read Sol's review /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n92/sol-code.md and fix exactly its three minors:
1. `crates/ezsdr-radio-uhd/tests/fake.rs` ~:8170 and ~:8275: the "repeat playing" precondition accepts any `tx_send`, including arm's four-zero flush (~:8212). Require an experiment payload send (a timed send of the repeat's samples, `n>0` with an instant) before the panic is armed.
2. `design/18-uhd-radio.md:220` (UR-16): the next-Action-times-out-after-KC-21a consequence under Policy `continue` holds only when uhd-control has died (after a uhd-rx/uhd-tx panic, uhd-control still drains and acknowledges Actions); qualify it.
3. `design/18-uhd-radio.md:309`: remove the stale sentence that uhd-control reads `S` from the stop/loss booking (that reader went with `Control::finish`'s write).
Run the two changed tests and `rules.py --base 1371ffa` (0 problems), `cargo fmt --all --check`. Append "Fix round after Sol" to report.md; final message: its summary.
