# 2026-11-15 bench probes — briefs (2026-11-13)

## brief


Worktree /home/komatsu/local_work/Ez-SDRv4-e3: `git checkout -b work/bench1115 main` (main `89fd3f7`). You are the implementer (OpenCode Sonnet 5.5 #high). Another lane will edit the main tree (`~/local_work/Ez-SDRv4`, note 51 stage 2c: `tests/common/{iii,n47s5}.rs`, the shared trials and their hardware tests, `src/provider/control.rs`): add your probes as new functions (prefer a new module under `crates/ezsdr-radio-uhd/tests/common/` and new `#[test] #[ignore]` fns appended to `hardware.rs`), touch shared code only where needed, and say where. Measurement tooling: not risky (owner 2026-10-26), no test plan; Sol reviews the code.

Three probes, each runnable alone with `hw <name>` with its result in the log (VERDICT/trace lines, per-cell summary), bounded by one deadline over the device's whole life (as `hw_67_stop_before_origin` does in hardware.rs: thread + deadline + `EZSDR-INFRA:` marker + abort), and with its non-device logic (cell list, summary, verdict from traces) in pure tests in the default suite:

1. **Receive-restart zeros sweep** — /home/komatsu/.cache/ezsdr-fixes/2026-11-13-rxzeros/options.md §4 (the sweep plan) and the owner's decision at its end (direction D: zeros by DDC half-band count as a bound 0/4/11/15; the sweep checks the half-band structure and settles (a) every start vs restarts only). Extend or reuse `hw_iii_raw_rx_restart_fill`: the decimation cells of §4 (≈ 20, incl. odd factors), 10 trials per cell (15 where §4 says), three tone levels (transmit gain; the tone source transmits), fresh open vs restart vs retune, and a dump of the first 40 samples per trial. Write the decision rule from §4 into the probe's summary line before data exists.
2. **#75 probes** — /home/komatsu/.cache/ezsdr-fixes/2026-11-13-n75/options.md §2.4 (skip T3 and compare the next burst) and the Fable review's bench items (fable-review in the orchestrator's notes, summarised here): (b) close a burst properly, reopen, send T3 at once, and count the ACKs on the new streamer (does an old burst's ACK reach the new streamer?); (c) read the T3 ACK 150 ms late (is it still there?). Transmit; record ACK ticks, counts per channel position, fronts.
3. **Open-flush sweep** — the open-time T3 flush at `arm` (`core.rs` `open_tx_device_locked`/`tx_flush_locked`): 1, 2, 10 MS/s × one-channel and two-channel transmit streamers (#2 UBX has slot A only; a two-channel streamer needs both slots — say which board/layout can do it, and if none in the current wiring, make the cell report "not runnable" rather than fail), ≥ 10 trials per cell, recording ACK received (all positions), ACK tick, the open's and the flush's wall time.

Add one dated section to plan/phase8/bench.md for 2026-11-15 (the three probes plus the existing `hw_67_stop_before_origin`): commands per board, which transmit (attenuator check first), minutes each, what each result decides (D's table and (a); #75's (3) enable/ask-again; F1's ACK routing), and the total time. Local build and test; no libuhd here: type-check the `hardware` binary as the earlier probe did (a copy with the `uhd` cfg removed and stubs, or the dev container `ezsdr-v4-dev:uhd4.10` with `--features uhd`); `cargo fmt --all`; `cargo test --workspace` green; clippy clean; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 89fd3f7` = 0. Mutation rows for the pure logic; run them through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115/mut.log 2>&1` **in the foreground** and read its result before you end. No commit.

Report: `git diff --stat`; per probe the sequence, the cells, the verdict/decision rule, the time estimate, transmit or not; the bench.md section; rows and their killing assertions.

## brief-fix1


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/bench1115` at `c959310`. You are the implementer (OpenCode Sonnet 5.5 #high). Read /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115/sol-code.md (Sol: FAIL, seven majors, one minor — all adopted) and your report at the end of impl.log.

Principle for every decision line: a probe states what it observed; it recommends a change only when the complete planned evidence is present, and otherwise prints INCONCLUSIVE with what is missing.
1. Odd decimations: align the analysed span to whole samples and to the tone-analysis window so D = 3, 6, 12, 24, 60 can be complete; a pure test that an odd-decimation cell is covered.
2. RXZ decisions need the complete planned cell set and ≥ 10 valid observations per cell (no 8-of-10 threshold; 15 where planned); otherwise INCONCLUSIVE naming the missing cells.
3. The candidate bound is the maximum over every measured condition that applies (levels and start kinds), computed separately from the half-band structure comparison; a level effect still adds the KL line but never lowers the bound.
4. Skip-T3: reject an unstable control (state the stability rule: spread of the control offsets), bracket/interleave control trials with the skip trials, and attribute a difference to T3 only against a stable bracketing control; Sol's `[0;5]+[10000;5]` case as a pure test.
5. ACK routing: an ACK with no tick cannot pass routing; the decision says only "no foreign ACK observed" (never "ACKs are streamer-owned"). Late ACK: separate a transport/setup failure, a missing ACK (never generated or not seen) and demonstrated retention; absence alone never proves expiry.
6. Open-flush: the global decision stays INCONCLUSIVE while any two-channel cell is NOT RUNNABLE (the owner gated (3) on one- and two-channel cells), unless the owner waives it; say so in the line.
7. bench.md: one command per transmitting probe, each preceded by the owner's attenuator confirmation; keep the per-board minutes and the total.
8. Onset noise guard: add a positive noise-floor / coherent-tone requirement so a zero-median quantised noise does not pass; a quantised-noise pure test.

Rules as before (local build and test, the `hardware` binary built with `--features uhd` in the dev container as you did, `cargo fmt --all`, `cargo test --workspace` green, clippy clean, `rules.py --base 89fd3f7` = 0, no commit, no mutant in the tree). Run the new and touched rows through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115/fix1-mut.log 2>&1` **in the foreground** and read the result before ending.

Report: `git diff --stat`; each fix with path:line; new pure tests; rows; the bench.md changes.

## brief-fix2


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/bench1115` at `1bc8daa`. You are the implementer (OpenCode Sonnet 5.5 #high). Sol's eight findings are closed. The re-check found (all adopted):
1. Major: probe (a) cannot see receive overflows: `skip_trial` (tests/common/bench1115.rs ~886-895) takes the onset only from `got.samples`, never reads `got.other` (the receive outcomes `raw.rs:81-83` collects) nor checks contiguity; `SkipTrial` and its trace (~763-768) keep neither. Record the non-Timeout receive outcomes and contiguity in every receive-reading trial of the three probes (and their traces); a trial with an overflow or a gap is INCONCLUSIVE (named), never FAIL; a pure test: an overflow in an `open_then_skip` trial against stable controls → the trial INCONCLUSIVE and the skip decision not "T3 is needed".
2. Minor: the `RXZ class` line (~368-377) prints a "candidate bound" even with missing cells: mark it incomplete (and bench.md's table derives a class value only from a complete bound).
3. Minor: `late_verdict` (~1154) and `flush_trial_verdict` (~1342-1344) accept an ACK with no tick or one before the send: refuse them as (b) does (INCONCLUSIVE), with tests.
4. Minor: the routing FAIL text (~1246) names a cause for the `(many, 0)` case it did not observe: say only what was observed.
5. Note → fix: a `closed_then_skip` trial that sees A's old ACK counts it as a T3 effect (~835-841): count only ACKs at or after the trial's own burst B's send by tick, or report the old ACK separately.
6. Note → fix: bench.md's grep for the rule line (`RXZ rule`) does not match the printed `RULE (` line: make them agree.
7. Safety (orchestrator): the two-channel open-flush cells would transmit on slot B (~1410) after a board swap with no attenuator check of slot B. Transmit on slot B only when an explicit environment variable naming the confirmation is set (e.g. `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1`), otherwise the cell is NOT RUNNABLE; bench.md says the owner confirms slot B's attenuator before setting it.

Rules as before (local build and test, the `hardware` binary built with `--features uhd` in the dev container, `cargo fmt --all`, `cargo test --workspace` green, clippy clean, `rules.py --base 89fd3f7` = 0, no commit, no mutant in the tree). Run new and touched rows **in the foreground** via `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115/fix2-mut.log 2>&1` and read the result before ending.

Report: `git diff --stat`; each fix with path:line; tests; rows.

## brief-fix3


Worktree /home/komatsu/local_work/Ez-SDRv4-e3, branch `work/bench1115` at `6dcba5f`. You are the implementer (OpenCode Sonnet 5.5 #high). The overflow major and the slot B gate are closed; adopt these (tests/common/bench1115.rs line numbers as of 6dcba5f):
1. N1: probe (a) must check that the capture spans B's window: require the capture to start at or before `b_start - len` and end at or after `b_end + len*3/2`; a short capture (the read's 2 s limit, or only Timeouts) is INCONCLUSIVE like a gap, never "burst B is not in the capture". Pure test: a timeout-only capture → INCONCLUSIVE, the decision not "T3 is needed".
2. Pin the recording side: a fake rehearsal of `skip_probe` with `FakeConfig { faults: vec![FakeFault::Overflow(Duration::ZERO)], .. }` (device.rs:271-272, :1580-1595) asserting a trial with non-empty `rx_other` and `contiguous == false`; restore a row with the original mutant `trial.contiguous = true` and one dropping the `rx_other` assignment, both killed by it.
3. N2: in RXZ, the completeness / overflow check runs before `misplaced()` / `cell_verdict`'s off-tick FAIL, so an overflow ahead of the first block is INCONCLUSIVE.
4. N3: `control_stable` and `bracket_reference` ignore a control whose capture is dirty (overflow, gap, short); with too few clean controls the control is not stable (INCONCLUSIVE).
5. N4: assert `d9_channel() == 0` in every bench1115 transmit path (a probe aborts before any device call if `EZSDR_D9_CHANNEL` points elsewhere); bench.md shows the one-shot form `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 hw hw_75_open_flush_sweep` and warns against `export`.

Rules as before (local build and test, the `hardware` binary built with `--features uhd` in the dev container, `cargo fmt --all`, `cargo test --workspace` green, clippy clean, `rules.py --base 89fd3f7` = 0, no commit, no mutant in the tree). Run the new and touched rows **in the foreground** via `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim03:4 <id> ... > /home/komatsu/.cache/ezsdr-fixes/2026-11-13-bench1115/fix3-mut.log 2>&1` and read the result before ending.

Report: `git diff --stat`; each fix with path:line; tests; rows.
