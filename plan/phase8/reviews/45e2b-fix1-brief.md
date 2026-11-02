# Brief: note-45 experiment E2b, fix round — a core liveness fix, the receive adapter's hot path, and a fair measurement

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`** and do not search for it.

## Where things are
- The original brief is `~/.cache/ezsdr-fixes/2026-11-01-n45/brief-e2b.md`, including its Amendments section. Its definitions and verdict stay binding unless this brief changes them below.
- The first implementation is uncommitted in the tree: `crates/ezsdr-radio-uhd/src/brain/speed/` plus small edits to `brain/threads.rs`, `brain/shared.rs` and `brain/mod.rs`, and rows E2-62 to E2-65. Its results are in `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results/` (start with `README.txt`).
- Sol's review is `~/.cache/ezsdr-fixes/2026-11-01-n45/sol-e2b-code.md`: the `# REVISE` section near the end, 5 × P1, 2 × P2, 1 × P3, and a ranked list of hot-path costs. **All findings are adopted.** The decisions are below.
- E2a (committed e144da8) is the thread runtime. Its sequences, tests and rows (E2-01 to E2-61) must keep passing. E1's core, cell, rules, world and cases (`brain/core.rs`, `cell.rs`, `rules.rs`, `world.rs`, `cases.rs`) and their rows (E1-*) must keep passing too.

Work in two parts. **Report after part A and stop.** Part B starts only when the orchestrator says so.

## Part A — the decision core and the receive adapter (changes E1/E2a code: no decision may change except A1)

### A1. A held value at the in-flight bound is never reserved later (Sol finding 4; a core liveness defect)
- `Brain::reserve` (`core.rs:~518-530`) leaves a released value held when its chain is at `IN_FLIGHT_BOUND`. `recheck` (`core.rs:~724-739`) retries only deferred requests, so the value is never reserved again, even after a slot frees and while its instant is still ahead.
- Note 45 §4.5 says the value stays in the held queue "as today's held queue does". A held value waits for capacity; it is not dropped.
- **First, write a failing test.** Fill the bound, release one more value, then consume an earlier value before the new value's instant. Expected: the held value is reserved, sent and applied. A unit test in `core.rs` is enough. If the DST cases can reach the bound cheaply, add a progress check to the oracle as well: at the end, no released value stays held while its chain had a free slot before its instant. If they cannot, say why.
- **Then fix it in the core:** when a slot frees, the released held values of that chain are reserved in their order. Decide where this fits (`recheck`, or the consumption and retire paths), using the same rules as `reserve`. A value whose instant has passed is reported as `reserve` would report it today. Do not invent a new disposition.
- Add a row that removes the retry. Run the E1 suites (cases, sweeps, witnesses) and say whether any verdict or witness changed. If one changed, explain why.

### A2. The receive executor's hot path (Sol finding 3 and its cost list)
These costs are in the test adapter. Note 45 does not require them:
1. per-sample scratch-chain reconstruction and payload allocation in `rules.rs:~37-59,380-387`;
2. the growing `consumed` history, a `BTreeSet<u64>` that is never pruned and is cloned two or three times per cycle (`threads.rs:~589-591,627,757,794`);
3. per-block tick vectors that measured runs discard (`rules.rs:~257,423-427,463-465`);
4. per-cycle cell clones and predicate allocation (`threads.rs`, `force.rs:~133-140,224-242`).

Remove each cost without changing a decision:
- prune or bound `consumed`, keeping what a later decision reads (find every reader first);
- read state by reference instead of cloning it;
- compute the per-sample consumption in one pass over the block's channel/tick ranges, not per sample;
- build tick vectors only when the observation sink needs them.

The cell protocol of note 45 §4.5 stays: compute outside the cell from a view, then recheck and write inside it. If the view must stay a copy, make it a small one, holding what the computation reads, not the whole cell. In `rules.rs`, change a function only where both `world.rs` and the threads call it. The DST and the threads must keep calling the same rule (AGENTS.md §9 Maintainability 1).

**Equivalence is the check:**
- the whole brain suite;
- S1 forced replays, where `Observed` must equal the world's;
- the E1 2,000-seed sweeps, unchanged;
- the conformance suite.

The orchestrator runs every E1-* and E2-* row remotely afterwards. A row whose anchor you change must be re-anchored so that it produces the same mutant program.

**Measure the effect before reporting:**
- run today's owner and the new runtime once each at the high-rate condition, 50 MS/s × 500, for 10 s, on sim03 through `rdev.py`;
- give the achieved blocks/s and the overflow counts.

This is a diagnostic, not a verdict.

### A3. One FakeDevice configuration for both owners (Sol finding 1)
The new runtime must run on the same `FakeConfig` as today's owner: the same master clock, decimation 4 at 50 MS/s and 200 at 1 MS/s. Today E2a's R assumes one tick per sample. Map at the device boundary, with device ticks = sample index × decimation from the stream's origin, where the receive executor turns a block into ticks. Do not do it by changing the device.
- Check where the core and cell read ticks: consumption ticks, cuts, origins and the instants of timed values are device ticks.
- Add one test with decimation > 1: a timed DDC value is consumed by the sample at its instant. Add it to the conformance suite or as an S7 variant.

**Report for part A, at most 20 lines:**
- A1: the test, the fix, the row, the E1 effect;
- A2: each cost removed, the equivalence results, the 10 s blocks/s and overflow numbers;
- A3: the change and its test;
- suite and clippy results;
- rows added or re-anchored;
- lines changed.

## Part B — the measurement (after the orchestrator's go)
- **B1. Ingress (Sol finding 2).** The original brief is contradictory: a 50 ms lookahead holds more than 8 pending updates for these tables. The rule now is:
  - update i enters its owner at `max(e_i − 50 ms, the moment fewer than 8 earlier updates are outstanding)`;
  - an update is outstanding until its owner's `apply` returns, or, if it never does, until `e_i + 10 ms`, when it counts as a failure and frees its slot;
  - the same rule applies to both owners.

  Record each update's requested instant, ingress instant, lateness against `e − 50 ms`, its effective instant as booked, and whether the booking floor moved it (observed, not predicted). An update whose ingress comes later than `e − 10 ms` is a failure, reported with its cause.
- **B2. Fail closed (Sol finding 5).** The verdict is PASS only if all of these hold:
  - every update of **both** owners is applied at `e`;
  - no unexpected apply and no extra DSP entry;
  - no floor movement;
  - the latency and loss checks hold **per repetition** as well as pooled.

  Keep the per-repetition verdicts in the output.
- **B3. Loss at a common endpoint (Sol finding 6).** Stop and drain before reading the counters.
  - Reconcile, per owner: blocks produced by the device, samples the protocol discarded (with its reason: before origin, after cut), blocks queued, and blocks delivered to the reader.
  - The new runtime publishes to the sink **after** the cell accepts the publication, and only what it publishes. A gap at the last block is detected as well.
  - Report the three loss counts from these.
- **B4. Diagnostics (Sol finding 7).** Either collect per-block R hold and wait distributions in R itself, recording outside the held mutex, or label the current numbers as acquisition-level diagnostics over all actors. Prefer per-block R.
- **B5. Attribution (Sol finding 4).** Record bound hits: per update, whether it was held at the bound, when it was reserved, and why it was not applied.
- **B6. P3s (Sol finding 8).** DSP matching stays strictly one-to-one, with no frequency fallback for a label. The bracket uncertainty includes tick quantization. Rename E2-64 to what it mutates.
- **B7. Rerun everything:**
  - the rate trial, if A2 or A3 changed the fake setup;
  - the four conditions with ABBA and BAAB;
  - the diagnostics;
  - the fault runs.

  Run them on sim03 through `rdev.py`, one at a time, and record `uptime`. Write the new results to `~/.cache/ezsdr-fixes/2026-11-01-n45/e2b-results-2/`, and keep the first results.

Part B report: as the original brief's report (at most 30 lines), plus the ingress lateness distribution and the bound-hit counts.

## Rules for both parts
- Run `cargo fmt --all` and `cargo clippy -p ezsdr-radio-uhd --all-targets -- -D warnings`, both clean.
- Run the brain suite. Run each timing-dependent test you add or change 20 times.
- Append rows as text and never re-serialize `mutations.json`. Then run `git ls-files -m -o --exclude-standard | xargs chmod go+r` and `python3 plan/maintenance/tools/rules.py --base HEAD` (0 problems).
- Never run mutate.py or remote_mutate.py. Never git commit, push, stash, reset or checkout. Kill processes by PID only.
- Out of scope: production code (`src/provider/`, `src/device.rs`, `src/uhd.rs`), FakeDevice behaviour, `design/`, the Kernel, schemas and Python. If a fix seems to need one of them, stop and report it.
