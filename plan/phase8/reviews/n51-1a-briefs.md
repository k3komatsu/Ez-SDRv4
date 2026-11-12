# ===== brief-plan.md

# Brief — note 51 stage 1a, phase 1: the test plan (no code)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1a` from `main` `1231462`. Read only; write only the plan file below. You are the implementer (OpenCode Sonnet 5.5 #high); stage 1a is **risky** (participant identity, a lifecycle): the test plan comes first and Sol reviews it.

Read first: AGENTS.md §3, §7, §9; plan/phase8/51-test-timing.md in full (the design; §4.1 and §4.2 are 1a's scope; §7.2 row 1a; the owner decisions Q1, Q2, Q7 in the appendix); plan/phase8/51-tables.md §2 (the W table) and §4 (bounds); plan/maintenance/30-fakedevice-manual-time.md (the M1–M7 participant model); the code: `crates/ezsdr-radio-uhd/src/host.rs` (thread-local id ~:40-43, `register` ~:399-402, `park` ~:234, `acquire_lock` ~:431, `ParticipantGuard` ~:517-525), `crates/ezsdr-radio-uhd/src/provider/mod.rs` (`Provider::join` ~:395-404), `crates/ezsdr-radio-uhd/src/authority.rs`, and design/18-uhd-radio.md UR-33 (its M1 sentence).

## Stage 1a (note 51 §7.2)

`host.rs`: per-clock registration (the thread-local holds `(clock, id)` pairs; `park`, `acquire_lock`, `ParticipantGuard` look up "my id on this clock"); `HostClock::drive(&self) -> Driver` (`!Send`, unregisters on drop including unwind; a second registration of the same thread on the same clock panics before allocating anything, Q7; a real clock gives a no-op guard); `Provider::join`'s 1 ms wait becomes `host.wait(None, real 1 ms, seen)` with `JOIN_BOUND` still real time (Q2); UR-33's M1 sentence ("the threads the uhd crate starts, and a rig's driver registered with `drive()`"). No rig uses `drive()` yet (that is 1b/1c).

## The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1a/test-plan.md)

1. The seven tests of note 51 §4.1 ((1)–(7)), each: what it asserts, how it observes it (the clock's participant states, its time, a frozen/manual rig — never a wall-clock sleep as the oracle except (1)'s "the clock does not move while the driver sleeps"), and the wrong behaviour that fails it; plus `Provider::join`'s change (a test that the joining thread is idle on the manual clock while it waits, and that `JOIN_BOUND` still cuts in real time).
2. Sequences the design implies beyond the seven (e.g. registration while another participant is busy; drop of a `Driver` on a thread that also holds a `ParticipantGuard`; a uhd-crate thread that calls `drive()`), each with its expected result.
3. Mutation rows (Rust token anchors) for each decision: the per-clock lookup, the double-registration check and its order before `register`, the drop path, the real-clock no-op, join's wait; each naming its killer.
4. Existing tests and rows that touch `host.rs` and `Provider::join` and whether they change (grep the anchors).
5. `design/` text: UR-33's M1 and any other sentence 1a makes false.

Rules: no code edits, no builds in the tree (probes only in a copy outside the repository via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground); no commit. Final message: a summary, and anything in note 51 or the tables the code contradicts.

# ===== brief-plan2.md

# Brief — stage 1a, phase 1b: amend the test plan after Sol's review (no code)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1a`. You are the implementer (OpenCode Sonnet 5.5 #high). Your first brief: /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1a/brief-plan.md; your plan: test-plan.md there. Read sol-plan.md and plan-adjudication.md there. Amend test-plan.md in place so every adopted finding is closed, with a short "Changes after Sol's review" section at the top (per finding: what changed, where). You may re-run your out-of-tree probe (a copy outside the repository via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`) to check the new cases kill their mutants; report only what its log shows. No edits in the tree; no commit. Final message: per finding how it is closed.

# ===== brief-impl.md

# Brief — note 51 stage 1a: implementation

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1a` from `1231462` (the step's start). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1a/: brief-plan.md (scope), test-plan.md (reviewed; "Changes after Sol's review"), sol-plan.md, plan-adjudication.md; your probe diff probe/probe-1a-amended.diff is a starting point, not the answer.

Implement stage 1a as the plan states: per-clock registration in `host.rs`, `HostClock::drive()` / `Driver`, the double-registration check (manual clocks, before allocating), the real-clock no-op, `UhdRadio::join`'s clock wait with `JOIN_BOUND` in real time, every planned test (incl. E9a–E9c, J1, J2 as amended), the mutation rows (rows 14–15 with `"kill": "bound"`; re-anchor J283; check J284, J278, J313), the `design/` text (UR-33 M1 and M6 example list, UR-16 as adjudicated, the Changes rows, the over-long line split for `rules.py`) and the `host.rs` module doc. No rig uses `drive()` yet.

Run the rows you add or touch through `remote_mutate.py maintenance sim03:4 <ids>` (or the list they belong to); every row killed (rows 14–15 `killed (bound)`). New timing-dependent tests: 20 runs. `cargo fmt --all`; `rdev -- cargo test --workspace` green; `rdev -- cargo clippy --workspace --all-targets` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 1231462` = 0. Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground. Never a mutant in the tree; temp files on /tmp; kill by PID only. No commit.

Report: `git diff --stat`; per planned test: green and what it observes; rows and outcomes; design text changed; deviations from the plan with reasons.

# ===== brief-fix1.md

# Brief — note 51 stage 1a, fix round 1: Sol's code review

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1a` (local commit `13f4e54`; step start `1231462`). Implementer: OpenCode Sonnet 5.5 #high; brief-impl.md and the plan in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1a/ still bind. Read sol-code.md there. Adopted (decided by the orchestrator):

Major 1 — hang bounds counted as ordinary kills (N51A-02, 03, 04, 11, 14, 15 die only when a lib-test helper `within`/`joined`/`bounded` or J2's 10 s/30 s bound expires, with no infra marker, so `mutate.py` reports plain `killed`):
- Retarget N51A-02/03/04 to tests whose **state assertion** fails under the mutant (the E9 cases' participant-state checks, or add one) and N51A-11 to the duplicate-registration test's immediate post-drop state check — each must die on an assertion, not a bound.
- N51A-14/15 are genuinely bound-only: keep `"kill": "bound"`, and make the bound that kills them write an uncaptured `EZSDR-INFRA: <test>: <msg>` marker (one `std::io::stderr().write_all`, as `crates/ezsdr-radio-uhd/tests/common/infra.rs` does; the lib tests cannot import it — a small `#[cfg(test)]` helper in the crate is fine, or extend the existing lib helpers `within`/`bounded` so **every** lib-test bound writes the marker before panicking, if that is the smaller change). Then they come back `killed (bound)`.
- Run the touched rows through `remote_mutate.py maintenance sim03:4 <ids>` and report each row's outcome and `why`.

`cargo fmt --all`; `rdev -- cargo test --workspace` green; clippy clean; `rules.py --base 1231462` = 0. No commit. Report per row.

