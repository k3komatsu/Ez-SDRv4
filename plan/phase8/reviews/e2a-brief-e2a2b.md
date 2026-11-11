# Brief — E2a-2b: the scoped abandon (#80), a halt cancels an unsent open booking (#85), a late underflow with no open device burst changes nothing (A5/F4)

Repository /home/komatsu/local_work/Ez-SDRv4, branch `work/e2a`. The head is the E2a-2a commit (the orchestrator names the hash when launching); the step's start commit is `0ccb267`.

Read first:
- /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/brief.md, whose rules bind;
- test-plan.md §2.3 and §3.3, both with the "Changes after Sol's review" and "Changes after the re-check" sections, which override the older text;
- plan-dispositions.md;
- `git log -3`, for E2a-1 and E2a-2a, which built `TxBook`, `Open`, `issue` and `complete`.

This is the first sub-step that changes behaviour. **Red first** (plan §2.5): before each production change, run each test that pins it on the previous commit, in a /tmp copy via rdev or locally with a target outside the tree, and put its failure text in the report. Tests the plan marks green on the base say so.

## Scope

1. **The scoped abandon** (§2.3, rules 1–7, as amended).
   - It is one pure function on `Bookings`, used through `TxBook::abandon`, and it replaces today's `cancel_switch_from` + `stop` + `halt` + `playing = None`.
   - It has exactly three named differences from today:
     - (a) #80: later chains (start ≥ s) are kept;
     - (b) F3: R + applied S with c ≤ b < s gives a SendAsap in [b, s) a receipt refusal (footprint), where today it is moved;
     - (c) an applied switch whose payload is the unsent `open` is cancelled.
   - The `c == r` cell keeps today's answer (R's chain removed, Stop region [r, s)).
   - A pending switch with b < c is applied at s.
   - `until` is capped only when no chain with start ≥ s remains.
   - `playing` is cleared only for the abandoned repeat or its switch.
2. **#85.** The book's `cancel_all(reason)` returns `held` plus `open` when `open.first`.
   - `read()` uses it for a `Halt`, and `shutdown()`'s `stop_now` uses it too.
   - The Mock's `stop_tx` uses the same rule. Its `open_burst_without_record` is the same predicate: one rule, one copy.
3. **A5/F4.** `report`'s underflow branch acts only when `chain.burst_is_open()`; `broken(None)` is a no-op.
4. The E2a-1 leftover m1: `Tx::abandon` reads only what the new function needs. The case where the `Tx` has a clock and the stream does not exists only in test fixtures; make the fixtures set the stream clock, or explain why not.

## Tests (plan §3.3, amended)

- Pure tests: P10 (P3 extended), P11, P12, P13, P14, P15.
- Rig tests in `e2a.rs`: R7, R8, R9 (the re-check recipe: start from `o1_uhd_t4d`'s state; a Stop applies S in the record; inject `report(Underflow)` before the owner reads the halt; SendAsap at b+1 with ≥ 2 ms lead), its pending twin (b == c, no Stop), R10, R11 (the `c == r` sequence) and R12.
- Changed tests: `o1_uhd_t6b2` (flips; renamed; add it to the renames file the orchestrator keeps at /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/renames.txt, one `old -> new` per line, or `old -> -` for a test dropped with its recorded reason), `e1_q5`, `o1_uhd_t4d`, `ur_23_stop_while_the_first_send_waits_sends_nothing` and `ur_23_m4c_…`.
- Use the step-F constants (`STOP_FRONT`, `CLOSE_GAP`, `closed_short_gap`), never literal numbers.

## Trace comparison

Compare against E2a-2a's head, not `0ccb267`. Every differing file must be explained by a named change: the stray two-zero EOB of F4 that disappears, and the extra close that is avoided. List each one.

## Mutation rows

Follow plan §3.3, amended:
- retarget N47S5-136, N47S5-139 and N47S5-153 (killer P13);
- re-anchor what breaks;
- add the plan's rows (10 after the re-check), including the `c == r` row and the applied-switch-with-open-payload row. Each names its killer in `filter`, and that killer must run the mutated path.

The rules from E2a-1 and E2a-2a hold:
- never move a UHD-only or Mock-only row into shared code;
- never make two rows identical;
- check that every `new` compiles.

You may check killers by applying mutants in a /tmp copy outside the repository. Never use mutate.py or remote_mutate.py, and never apply a mutant inside the tree.

## Text (plan §5)

- RM-16's `*Test:*` line names the renamed `o1_uhd_t6b2` and R10.
- The `*Test:*` lines of RM-15 and RM-21 (`design/07-radio-model.md`) name R7 and R8.
- UR-22 (`design/18-uhd-radio.md`): change a sentence only if it becomes false.
- F3's change (b) is RM-15's footprint, which is already written. Check that no sentence in RM-15, RM-16 or UR-22 says the abandoned repeat's region after an applied switch is a Stop region.
- List every `design/` sentence you changed, and every one you checked and left.

## Before reporting

- `cargo fmt --all`.
- rdev workspace tests and clippy, in the foreground.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 0ccb267 --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-10-e2a/renames.txt` = 0.

## Report

- `git diff --stat`.
- The function and its rules, as implemented.
- The red-first results, test by test.
- Every existing test changed, and how.
- The trace differences and the named change that explains each.
- The rows retargeted, re-anchored and added, with their killers.
- The `design/` text changed and checked.
- Anything in the plan that proved wrong against the code, with evidence.

Do not commit, push or `git add`. Do not edit AGENTS.md, handoff.md, plan/phase8/49-*.md, bench-results.md or bench/.
