# Note 51 stage 1b — briefs (2026-11-12)

## brief-plan

## Brief — note 51 stage 1b, phase 1: the test plan (no code)

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1b` from `main` `2c0493c`. Read only; write only the plan file below. You are the implementer (OpenCode Sonnet 5.5 #high); stage 1b is **risky**: the test plan comes first and Sol reviews it.

Read first: AGENTS.md §3, §7, §9; plan/phase8/51-test-timing.md (§3, §4.1–§4.2, §7.1, §7.2 row 1b, the appendix decisions); plan/phase8/51-tables.md §2 (the W table: `UhdRig`'s rig-level sites and §2.3) and §4; stage 1a's records plan/phase8/reviews/n51-1a-* (the `HostClock::drive()` / `Driver` API that 1b is the first to use); plan/phase8/reviews/n96-dispositions.md (the 14 receipt-window tests #96 left to this design); crates/ezsdr-acceptance/tests/parity.rs (`UhdRig`, its constructor, `wait_until`, `settle`, `wait_for_events`, `TxBlocks`, Drop), crates/ezsdr-radio-uhd/src/authority.rs (`DeviceAuthority::new`, Drop), crates/ezsdr-radio-uhd/src/host.rs.

### Stage 1b (note 51 §7.2)

`UhdRig` drives its manual clock: the `Driver` guard is the rig's first field (so it drops before the Authority), registration happens after the Authority is built (its 5 s first-read wait stays outside), and the `TxBlocks` assertion named in §7.2. The failing-test-first set is the **14 receipt-window tests** #96 left here (§7.1; n96-dispositions.md lists examples): each fails today under an injected delay of the test thread (the probe's `submit` injection) and passes after 1b. Tests #96 already fixed by sequence are not in the set.

### The plan (write it to /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1b/test-plan.md)

1. The 14 tests by name (from n96's records and note 51 §7.1), the injection that makes each fail today (where, how long; diagnostic only — note 51 §5.4: delay injection is not a gate or a mutation filter), and why registering the driver closes each one.
2. The rig changes, each with its test: the guard order (a rig dropped normally and during unwind leaves the clock free and the Authority's Drop does not wait on a participant that is gone), registration after `DeviceAuthority::new`, every rig method that waits doing so as a participant (W table §2.3), the `TxBlocks` assertion, the 10 s `wait_for_events` bound (keep real time; it must write the infra marker on expiry, as stage M's `infra` does).
3. Mutation rows (Rust token anchors) for the rig decisions, each naming its killer (a test that fails on an assertion, not a bound); existing parity rows whose filter or killer changes.
4. Tests in `parity.rs` that change, and any that 1b makes redundant (keep coverage unless its failure case is covered elsewhere — say where).
5. `design/` or plan text 1b makes false (UR-33, note 51, the tables).

Rules: no code edits, no builds in the tree (probes only in a copy outside the repository via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground; the injection runs only in such a copy); no commit. Final message: a summary, and anything in note 51, the tables or the #96 records the code contradicts.

## brief-impl

## Brief — note 51 stage 1b: amend the plan, then implement

Main tree /home/komatsu/local_work/Ez-SDRv4, branch `work/n51-1b` from `2c0493c` (the step's start). You are the implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1b/: brief-plan.md, test-plan.md, sol-plan.md, plan-decisions.md (all decisions, including "After Sol's plan review"); your probe sketch is a starting point.

1. Amend test-plan.md for Sol's findings (a "Changes after Sol's review" section).
2. Implement stage 1b as the amended plan states: `UhdRig`'s `_driver` first field, registration as the constructor's last step, the `TxBlocks` assert, `wait_for_events`'s infra marker with the 10 s real-time bound kept, the two skip branches as assertions, the new `uhd_rig_*` tests (N4/N5 as amended), rows N51B-01…08 (+ any new), the comment and text fixes of plan §5 (code comments and the tests README; list note 51 / tables lines for the orchestrator).
3. The 14 receipt-window tests must pass; show the diagnostic injection result in an out-of-tree copy (not in the tree, not a gate).

Rules: tests must not depend on when a thread runs (note 51 §3); new timing-dependent tests 20 runs; rows through `remote_mutate.py <list> sim03:4 <ids>` (if the other lane runs the full lists on both hosts, wait), each killed on an assertion unless declared bound with a marker. Builds via `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- <cmd>`, foreground. `cargo fmt --all`; `rdev -- cargo test --workspace` green; clippy clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 2c0493c` = 0. Never a mutant in the tree; temp files on /tmp; kill by PID. No commit.

Report: `git diff --stat`; per test what it observes; rows and outcomes (with `why`); the injection result; deviations.

## brief-fix1

## Brief — note 51 stage 1b, fix round 1

Tree /home/komatsu/local_work/Ez-SDRv4 (main tree), branch `work/n51-1b`, last commit `7924df2` (your implementation, committed by the orchestrator). You are the stage 1b implementer (OpenCode Sonnet 5.5 #high). Read in /home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1b/: sol-code.md (Sol's code review: FAIL, two majors) and recheck.md (the Opus re-check: Sol's plan majors closed, the T/T+1 race, minors m1–m4). All findings below are adopted (decided by the orchestrator).

### 1. Receipts at a fixed instant (the T/T+1 race; recheck.md)
In `UhdRig::wait_until` (parity.rs ~452), after the clock wait, `self.device.host().sleep(Wall::from_micros(1))` (or the clock's own smallest sleep), so every participant due at the instant (uhd-control's poll) runs before the test thread continues: a receipt is then exactly the first poll strictly after max(T, now). Check the same for any other rig wait that can end on a poll instant (`settle`, `uhd_barrier`'s polls). Add a pin test: repeated `wait_until(uhd_t0 + n ms)` + a submit + a barrier asserting the exact receipt tick each time. **Restore `s14_…` S3's value to 71 ms** (the 14 bodies unchanged) if it is green with the fix; if not, report why. Update crates/ezsdr-radio-uhd/tests/README.md (m4) to state the rule exactly. A mutation row for the 1 µs sleep only if its killer kills it in 20 of 20 runs; otherwise no row, and say so.

### 2. N4/N5's baseline (Sol M1, Opus m1)
The baseline `idle` must not depend on when the test thread reaches `drive()`: make the unreachable fault time late enough that construction cannot reach it in normal runs, and at the first reading assert the device time is still before the fault — if it is not, write the `EZSDR-INFRA:` marker (`watchdog::infra::infra` or the crate's marker helper) and panic (a host stall, not a verdict); take `idle` from a state where uhd-clock provably holds no extra `Arc` (or a running minimum), not from one reading.

### 3. N51B-03 by an ordering fact (Sol M2, Opus m2, m3)
The probe also records the device clock (`host.now()`) when it runs; the test reads the clock just before the rig drops. Correct order: the driver drops first, uhd-clock's read completes, the clock advanced across the drop and the count is 0. Mutant (driver last): the clock did not advance across the drop — assert that as a plain ordering assertion (it holds for any `CLOCK_JOIN` value, so N51B-03 stays a plain kill without `"kill": "bound"`). Correct code that detached only because of a host stall (count 1 while the clock advanced) writes the infra marker. Put the Authority and the probe in one wrapper field so no stall between two adjacent field drops can reorder them (m3); keep N51B-09 meaningful or retarget it.

### Rules
Tests must not depend on when a thread runs (plan/phase8/51-test-timing.md §3). **Build and test locally** (the main tree's `.cargo/config.toml` target), not on sim02/sim03. New or changed timing-dependent tests 20 runs each, and the 14 receipt-window tests 20 runs under load (e.g. `--test-threads` high, or concurrent runs). Run the touched and new mutation rows through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <ids>` (`simq` waits for the sim lock). `cargo fmt --all`; `cargo test --workspace` green; `cargo clippy --workspace --all-targets -- -D warnings` clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 2c0493c` = 0. No commit. Note 51, the tables and #96's report are the orchestrator's; list text your change makes false.

### Report
`git diff --stat`; per finding: what changed, the test, runs; rows and outcomes; anything not done and why.

## Orchestrator decisions on the test plan

## Stage 1b — orchestrator decisions on the plan's questions (2026-11-12, decided by the orchestrator)
- Registration position: accepted as the plan proposes — the driver registers as the constructor's last step (after `DeviceAuthority::new` and `prepare`), so no error or unwind path holds a live driver. Note 51 §4.2's "after the Authority" is satisfied.
- Skip branches: the two "not established" branches in `s15_parity_s4b_…` and `s15_parity_provider_stop_…` become assertions in 1b (the probe never took them; with a registered driver the window is deterministic).
- The contradictions 1–5 are recorded in note 51 / the tables at landing (§7.1/§7.2's counts, #96's "15", §4.2's reason, the tables' marker site).

### After Sol's plan review (sol-plan.md: FAIL, 2 majors, 1 minor) — all adopted (decided by the orchestrator)
- M1: N4/N5 establish the pending clock sleep explicitly (a hook or a gate that holds uhd-clock inside the unreachable read until the test releases it), and observe join/detach before releasing the condition that preserves the evidence; no verdict from transient `Arc` counts alone.
- M2: N51B-03 gets an ordering assertion that does not depend on `CLOCK_JOIN` expiring; if none is possible, it declares `"kill": "bound"` and the bound writes the infra marker.
- m3: the comment describes the residual poll phase, not host scheduling.
- Process: the amended plan and the implementation go in one run; the Opus re-check covers the majors on the code together with Sol's code review (decided by the orchestrator, to save a round).
