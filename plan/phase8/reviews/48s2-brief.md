# Note 48 S2 — briefs

---

## `brief.md`

## Brief: design note 48 S2 — delete the timed-setting paths and other mechanisms whose reason is gone (TEST PLAN FIRST)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `c58cb32`, clean tree. Nobody else edits the tree. This is a **risky** step (AGENTS.md §9): it changes the `Device` trait and the boundary between FakeDevice and the hardware, a profile key and its schema, and the receive restart's deadline. **This task is the test plan only**: write it to `~/.cache/ezsdr-fixes/2026-11-05-n48s2/test-plan.md` and stop. Do not change the tree.

Read:
- `AGENTS.md`: Safety core, §2 (spec format, KNOWN_LIMITATIONS), §6, §7, and §9 with its Maintainability and Counterexamples parts.
- `plan/phase8/48-simplification-inventory.md`: all of it. **§2 and §3 are the decisions. Do not reopen them.**
- `plan/phase8/research/48-inventory-facts.md`: the facts, with path:line from `9dab08a`. Re-check every line against `c58cb32`, because S1 moved code in `tx.rs`, `core.rs`, `device.rs` and the Mock.
- `plan/phase8/reviews/48-sol.md`.
- `plan/phase8/47-no-change-while-streaming.md` §2.1, item 4 (the release window).
- The specs the items cite: `design/07-radio-model.md`, `design/09-mock-radio.md`, `design/18-uhd-radio.md`, `schemas/`.

### The step

S2 covers these items of note 48 §3:
- A1: `Device::apply`'s `at`, the timed command queue in FakeDevice, the DSP FIFO and its flow parts.
- A1′: `rx_stop`'s `at`.
- A2: `command_queue_depth` and `COMMAND_QUEUE_FULL`, with the payload, schema and measurement group (owner: delete).
- A3: `Line::halted`.
- A4: `tx_dsp_gates`.
- A5: `Channels::without`, `rx_start_now`.
- A5′: `Recovery::Halted`.
- A6: stale text.
- A7: the Mock's empty `applied` section.
- B3: the receive restart's release window, and `release_window_ns` (owner: delete). **The profile start lead is not lowered.**
- B5: rename `CancelReason::ColdChange` to say what creates it.
- B7: trace `owner_step`'s two branches. Delete each one that is unreachable.

Each row's "残すもの・注意" column is binding.

**Split.** Plan S2 as two sub-steps. Each one leaves the tree green and is gated on its own:
- **S2a**: A1, A1′, A4, A5, A5′. These are the timed paths of the UHD crate's device and core.
- **S2b**: A2, A3, A6, A7, B3, B5, B7. These are the profile and schema, the timeline, the Mock and the spec text.

If you find a dependency that forces another order, say so with the evidence.

### Sequences that matter (each needs a test that stays or moves; expected result)

- G1. An untimed setting on a stopped receive stream, then `start_rx` with staged values. Expected: coercion, read-back, refusal and `apply_delay` behave as today. The frequency history (`device.rs` ~1464–1519, 1148–1155 at 9dab08a) is unchanged.
- G2. Open a transmit stream, then flush, then a timed burst, then close. Expected: the stale-header state and the P5/D70 packet-state tests behave as today. The four-zero T3 flush and its attribution from S1 are unchanged.
- G3. The receive restart's origin (B3). Expected: the ready term without the release window is old-rate call time + delivery allowance + start lead, rounded to the grid. The configure-complete warning, missed start and `START_LEAD_SHORT` are unchanged. MockRadio and UHD agree on the origin in the differential (VH). Retarget `rm_25_a_receive_successor_adds_release_window_before_call_and_lead` to pin exactly the remaining terms.
- G4. A transmit reopen that fails (A5′). Expected: neither the held payload nor a new payload is sent. The clock ends, and the cancellation reports are as today under the new `CancelReason` name (B5).
- G5. A loss, a Stop and `Provider::stop` while receiving, and while transmitting. Expected: the order and outcomes are unchanged after `rx_stop`'s `at`, the gates and `Line::halted` are gone. The halt rule stays: plan, `streaming()`, refusal exclusion and prune equivalence (A3's column).
- G6. A profile or Binding document that still carries `command_queue_depth` or `release_window_ns`. Expected: name what happens: refused, or migrated with a reason (AGENTS §3: never silently reinterpreted). The schemas are regenerated and `schema_freeze` is green. There is no version bump and no `SCHEMA_CHANGELOG` entry (AGENTS §6).
- G7. B7: for each `owner_step` branch, give the production path that reaches it (path:line), or the evidence that none does. Delete it only on the second.
- Add your own failure scenarios beyond G1–G7.

### The test plan (AGENTS §9 step 2; note 48 §2 rule 4)

1. **The retained/moved/deleted map.** List every test that touches a deleted item. For each one: kept, moved (which assertion goes where), or deleted. A deletion needs evidence that its failure case is unreachable or covered elsewhere, and names the covering test. Never delete a test of a reachable sequence without a replacement. Group the tests by what they check, not by file.
2. **Mutation rows.** These are in `plan/maintenance/tools/mutations.json` and `plan/*/tools/mutations.json`. Give:
   - every row whose `old` or `old2` lies in deleted code: delete it or retarget it, with the reason;
   - every row whose `filter` names a deleted or renamed test: find them with grep;
   - every row whose `new` contains a deleted name: find them with grep;
   - the new rows, with Rust token anchors and their killing tests.
3. **Spec and docs.** Give the exact rule IDs and sentences that change in `design/07`, `09`, `18`, the Vision `Normative:` lines if any, `KNOWN_LIMITATIONS.md`, the measurement docs and `schemas/`. For each one say: deleted, reworded, or stubbed (a withdrawn rule keeps a one-line stub). Add a Changes row for each. A UR/RM/MR line about the DSP FIFO becomes the one line note 48 A2 asks for: the receive-command FIFO and the transmit data path.
4. **Tests that stay as they are** and pin the behaviour that must not change (G1, G2 and G5 especially): name them.
5. **Size.** Estimate the lines changed per sub-step, production and tests counted apart. A sub-step is about 1,500 lines. Deletions count, but less. Propose a further split if one is far over.

### Rules

- No tree changes in this task. Use `timeout 300` on any test you run. Your tool stops a command at 120 s, so run longer ones with a longer timeout in the foreground.
- **Write what happened, never what was not done.**
- Lessons from S1, where these went wrong:
  - Every planned test is part of the step. None is a "gap" to report.
  - PID of your own `opencode run` client is not another agent.
  - Read the orchestrator's files only.
  - Add no test whose verdict depends on the wall clock.
- Do not add mechanisms beyond the step. Report a gap instead.

### Report (at most 15 lines)

- Where the plan is.
- The S2a/S2b split and its sizes.
- The G6 and G7 answers.
- Every deletion whose coverage you could not show.
- Open questions for the orchestrator, each with your recommendation.
- Claims you could not verify.

---

## `brief-sol-plan.md`

You are the independent reviewer (Sol), read-only, in ~/local_work/Ez-SDRv4 (HEAD c58cb32, clean). Do not edit, format or build in the tree.

AGENTS.md §9 step 3: review the test plan of a risky step before any code: ~/.cache/ezsdr-fixes/2026-11-05-n48s2/test-plan.md, written for the brief ~/.cache/ezsdr-fixes/2026-11-05-n48s2/brief.md (design note 48 S2: delete the timed-setting paths and other mechanisms whose reason is gone). The decisions are plan/phase8/48-simplification-inventory.md §3 (do not reopen them); the facts plan/phase8/research/48-inventory-facts.md (cited at 9dab08a; S1 at c58cb32 moved code since). You reviewed note 48 itself earlier (plan/phase8/reviews/48-sol.md).

First write your own failure scenarios from the step's purpose and the code (crates/ezsdr-radio-uhd/src/device.rs Device trait, UhdDevice and FakeDevice apply/rx_stop/command queue/DSP FIFO; provider/core.rs, rx.rs, tx.rs; crates/ezsdr-radio/src timeline and TimingEnvelope; crates/ezsdr-mock-radio; schemas/; tests including tests/common/measurement.rs and the parity/differential suites), then compare with the brief's sequences G1–G7 and the plan. Check in particular:
1. The retained/moved/deleted test map: is any deleted test the only pin of a reachable behaviour (untimed apply, coercion/read-back/refusal, apply_delay, frequency history, stale TX header and P5/D70 packet state, T3 flush, receive restart origin, loss/Stop/Provider::stop order, the halt rule)? Is every claimed replacement real?
2. B3: is the restart origin's remaining formula right in both UHD and Mock, and does the plan keep the differential agreement? Is the profile start lead untouched?
3. G6: removed keys in old profile/Binding/measurement documents — refused, and is that consistent with AGENTS §3 / invariant 39 and the schema_freeze tooling?
4. G7/B7: the `owner_step` reachability evidence (the plan says `Nothing` is unreachable for `by: None`, `Open` reached via `DeviceSet::Unknown`).
5. Mutation rows: every row in deleted code, every filter naming a deleted test, every `new` naming a deleted item — accounted for; new rows real.
6. The four-way split (S2a-i, S2a-ii, S2b-i, S2b-ii) and the forced order; spec/doc changes complete (design/05, 06, 07, 09, 18, Vision Normative lines, KNOWN_LIMITATIONS, measurement docs, schemas); the B5 rename (the plan proposes `Refusal`; note 48 suggests `ClockEnded`-like: which names what creates it?); the bench probes to delete.
For each major finding give a minimal operation sequence and the observable wrong outcome. Output: your scenario list (short), findings P1/P2/P3 with path:line or plan section, verdict PLAN OK / PLAN NEEDS CHANGES. At most 150 lines.

---

## `brief-plan2.md`

## Brief: design note 48 S2 — test plan rev 2

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `c58cb32`, clean tree. Nobody else edits the tree. Read `brief.md` (the task, its rules and report format still apply), your test plan `test-plan.md` (rev 1), Sol's review at the end of `sol-plan.log`, and the rulings `rulings-plan.md` (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s2/`).

Write rev 2 of the test plan: apply R1–R10 exactly. Keep rev 1 as `test-plan.v1.md`. Start rev 2 with a table mapping each ruling to the sections that changed. For R3 and R7, show the commands you ran (grep, anchor checks) so the orchestrator can rerun them. Do not change the tree.

Report (at most 12 lines): per ruling what changed; the final split with sizes; the R7 trace outcome; deletions whose coverage you still could not show; open questions with your recommendation.

---

## `brief-impl-a1.md`

## Brief: design note 48 S2 — test plan rev 3, then implement sub-step S2a-i

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `c58cb32`, clean tree. Nobody else edits the tree. The `opencode run` client's PID belongs to you; it is not another agent.

Read:
- `AGENTS.md`: Safety core, §6, §7, and §9 with Maintainability.
- From `~/.cache/ezsdr-fixes/2026-11-05-n48s2/`: `brief.md`, your `test-plan.md` (rev 2), and `rulings-plan.md` (R1–R10 and Round 2, S1–S12).

### Task

1. **Test plan rev 3.** Apply S1–S12 exactly.
   - Keep rev 2 as `test-plan.v2.md`.
   - Start rev 3 with a table that maps each ruling to the sections it changed.
2. **Implement sub-step S2a-i only.** S2a-i is A2's queue measurement group, A1, A1′ and A5, with its tests, mutation rows and spec text as rev 3 assigns them.
   - Follow the plan.
   - Extend it (rev 3 §17, "implementation record") for every state or failure path the implementation adds.
   - **Every planned test of S2a-i is written. None is a "gap" to report.** If a planned test cannot be written as planned, say why with evidence, and give what replaces it.
   - Do not start S2a-ii.

### Rules

**Mutation rows**
- Make text edits only, in `plan/maintenance/tools/mutations.json` and `plan/*/tools/mutations.json`. Never re-serialize the file.
- Use Rust token anchors. Each row matches exactly once and names its killing test.
- The mechanical sweep (S3): after the code change, a script lists every row that is `NOT FOUND`. Save its output as `sweep-a1.txt`. Delete only the listed rows whose behaviour is gone. Each row on retained code is decided as rev 3 says.
- Confirm each new or retargeted row in a throwaway copy outside the repository:
  - copy: `rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3`;
  - build with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`;
  - run `find <copy> -type f -exec touch {} +` before building;
  - use one copy at a time;
  - a compile failure is not a kill;
  - logs go in `probes-a1/`.
- Never run `mutate.py`, `remote_mutate.py` or `gate.py`. Never apply a mutant to the tree.
- Existing rows whose `old` lies in a function you change: re-run them in the copy too.

**Runs and state**
- Run single tests with `timeout 300`. Your tool stops a command at 120 s, so run longer commands with a longer timeout, in the foreground. Leave no process running.
- Test temp files go on `/tmp`.
- Make no git state changes, no version bumps, and no `SCHEMA_CHANGELOG` entry. Regenerate any changed schema, so `schema_freeze` and `rm_20_schema_freeze` are green.
- Write what happened, never what was not done.

**End checks**
- `cargo fmt --all`.
- clippy `-D warnings` for the workspace, and for `-p ezsdr-radio-uhd --features uhd --all-targets`, both via `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`.
- The remote workspace tests, after your last edit.
- The Python 3.14 suite (handoff §5).
- `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` must give 0. If you rename or drop tests, list each with its reason in `plan/phase8/reviews/48s2a1-renames.txt`, in the format of `48s1-renames.txt`, and pass `--renames`.

### Self-review (one round)

Run it in the foreground with `timeout 2400`. Use free DeepSeek first:

`opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"`

Use Go (`opencode-go/deepseek-v4.1-flash#max`) only if free refuses.

Checklist:
1. Every S2a-i test in rev 3 is written, observes what the plan says, and its row breaks it.
2. Scope against rev 3: missing and extra (nothing of S2a-ii/S2b).
3. Every "no caller / unreachable" claim behind a deletion: grep the callers.
4. Every `design/` text this sub-step makes false is amended in this sub-step.
5. Maintainability: no leftovers of the deleted paths. No retained behaviour lost its only test.

Fix every finding, or reject it with a reason.

### Report (at most 15 lines)

- Rev 3: per ruling, what changed.
- S2a-i: what was deleted and what was kept.
- Tests: added, moved and deleted, with counts.
- Rows: deleted, retargeted and added, with probe outcomes and the sweep.
- Suites and `rules.py`.
- Self-review findings and their disposition.
- Deviations, and open questions with your recommendation.

---

## `brief-impl-a1a.md`

## Brief: design note 48 S2 — implement sub-step S2a-i-α (the queue measurement group)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `c58cb32`, clean tree. Nobody else edits the tree. The `opencode run` client's PID belongs to you; it is not another agent.

Read:
- `AGENTS.md`: Safety core, §6, §7, and §9 with Maintainability.
- From `~/.cache/ezsdr-fixes/2026-11-05-n48s2/`: `brief.md`, your `test-plan.md` (rev 2), and `rulings-plan.md` (R1–R10 and Round 2, S1–S12).

### Task

1. Rev 3 of the test plan is written (`test-plan.md`); read `rulings-plan.md` Round 3 (T1–T3) and record them in rev 3 §0 (keep rev 3 as `test-plan.v3.md` first).
   - Keep rev 2 as `test-plan.v2.md`.
   - Start rev 3 with a table that maps each ruling to the sections it changed.
2. **Implement S2a-i-α only** (T1): the queue measurement group of A2, its plan fields, env parsing and validation, fixtures, tests, mutation rows and hardware queue probes, and the spec text rev 3 assigns to it. The `Device` trait keeps `at`. Do not start β. This session has no time limit: do the whole of α, keeping the tree green; do not stop because the work is large.
   - Follow the plan.
   - Extend it (rev 3 §17, "implementation record") for every state or failure path the implementation adds.
   - **Every planned test of S2a-i is written. None is a "gap" to report.** If a planned test cannot be written as planned, say why with evidence, and give what replaces it.
   - Do not start S2a-ii.

### Rules

**Mutation rows**
- Make text edits only, in `plan/maintenance/tools/mutations.json` and `plan/*/tools/mutations.json`. Never re-serialize the file.
- Use Rust token anchors. Each row matches exactly once and names its killing test.
- The mechanical sweep (S3): after the code change, a script lists every row that is `NOT FOUND`. Save its output as `sweep-a1a.txt`. Delete only the listed rows whose behaviour is gone. Each row on retained code is decided as rev 3 says.
- Confirm each new or retargeted row in a throwaway copy outside the repository:
  - copy: `rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3`;
  - build with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`;
  - run `find <copy> -type f -exec touch {} +` before building;
  - use one copy at a time;
  - a compile failure is not a kill;
  - logs go in `probes-a1a/`.
- Never run `mutate.py`, `remote_mutate.py` or `gate.py`. Never apply a mutant to the tree.
- Existing rows whose `old` lies in a function you change: re-run them in the copy too.

**Runs and state**
- Run single tests with `timeout 300`. Your tool stops a command at 120 s, so run longer commands with a longer timeout, in the foreground. Leave no process running.
- Test temp files go on `/tmp`.
- Make no git state changes, no version bumps, and no `SCHEMA_CHANGELOG` entry. Regenerate any changed schema, so `schema_freeze` and `rm_20_schema_freeze` are green.
- Write what happened, never what was not done.

**End checks**
- `cargo fmt --all`.
- clippy `-D warnings` for the workspace, and for `-p ezsdr-radio-uhd --features uhd --all-targets`, both via `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`.
- The remote workspace tests, after your last edit.
- The Python 3.14 suite (handoff §5).
- `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` must give 0. If you rename or drop tests, list each with its reason in `plan/phase8/reviews/48s2a1a-renames.txt`, in the format of `48s1-renames.txt`, and pass `--renames`.

### Self-review (one round)

Run it in the foreground with `timeout 2400`. Use free DeepSeek first:

`opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"`

Use Go (`opencode-go/deepseek-v4.1-flash#max`) only if free refuses.

Checklist:
1. Every S2a-i-α test in rev 3 is written, observes what the plan says, and its row breaks it.
2. Scope against rev 3: missing and extra (nothing of β, S2a-ii or S2b).
3. Every "no caller / unreachable" claim behind a deletion: grep the callers.
4. Every `design/` text this sub-step makes false is amended in this sub-step.
5. Maintainability: no leftovers of the deleted paths. No retained behaviour lost its only test.

Fix every finding, or reject it with a reason.

### Report (at most 15 lines)

- Rev 3: per ruling, what changed.
- S2a-i: what was deleted and what was kept.
- Tests: added, moved and deleted, with counts.
- Rows: deleted, retargeted and added, with probe outcomes and the sweep.
- Suites and `rules.py`.
- Self-review findings and their disposition.
- Deviations, and open questions with your recommendation.

---

## `brief-sol-code-a1a.md`

You are the independent reviewer (Sol), read-only, in ~/local_work/Ez-SDRv4 (HEAD c58cb32; the sub-step's changes are uncommitted: `git diff HEAD`, `git status`). Do not edit, format, build in the tree or commit; never run mutate.py or remote_mutate.py. To run a test, use a throwaway copy (`rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3 ~/local_work/Ez-SDRv4/ /tmp/sol-s2a/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, `timeout 300`).

AGENTS.md §9 step 7: review the code of sub-step S2a-i-α of design note 48 S2 — removing the raw command-queue measurement group (A2's measurement part; the `Device` trait keeps `at` until sub-step β). Inputs in ~/.cache/ezsdr-fixes/2026-11-05-n48s2/: brief.md, rulings-plan.md (R1–R10, S1–S12, T1–T3), test-plan.md rev 3 (§9 is the α implementation record, including two deletions and one retarget the plan did not name), sweep-a1a.txt if present, and the self-review dispositions in impl-a1a.log's end. You reviewed rev 1 of this plan (sol-plan.log).

First write your own failure scenarios for this deletion, then check:
1. No retained behaviour lost its only test: the coercion group, the raw group latch (`raw_group_*`), lease/restore, record assembly, `ur_7_retries_only_the_reference_clock_not_locking`, the probe array, the restart-lead probe, P5/D70 packet state. Check every deleted or rewritten test (18 deleted, 5 rewritten) against rev 3's map and the rulings (S3, S12), especially the three the plan did not name and `FakeFault::TimeReadFails` (variant deleted).
2. The 52 deleted mutation rows: each one's `old`/`old2` lay only in deleted code, or its behaviour is gone; P732's retarget is real; no row on retained code was dropped (compare `git diff HEAD -- plan/maintenance/tools/mutations.json` with the code).
3. Leftovers: remaining references to the queue group (names, env vars `EZSDR_MEASURE_QUEUE_*`, fixtures, docs, bench.md, run-record.sh, Python), and nothing of β/S2a-ii/S2b done early.
4. design/18 changes (line 30, UR-34) correct and in the right sub-step; nothing else in design/ made false by α.
5. Maintainability per AGENTS §9.
For each major finding give a minimal sequence and the observable wrong outcome. Output: scenario list (short), findings P1/P2/P3 with path:line, verdict CODE OK / CODE NEEDS CHANGES. At most 100 lines.

---

## `brief-impl-a1b.md`

## Brief: design note 48 S2 — implement sub-step S2a-i-β (the timed device paths)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD = the commit that closed S2a-i-α (`git log -1`), clean tree. Nobody else edits the tree. The `opencode run` client's PID belongs to you; it is not another agent.

Read:
- `AGENTS.md`: Safety core, §6, §7, and §9 with Maintainability.
- From `~/.cache/ezsdr-fixes/2026-11-05-n48s2/`: `brief.md`, your `test-plan.md` (rev 2), and `rulings-plan.md` (R1–R10 and Round 2, S1–S12).

### Task

1. The test plan is rev 3 (`test-plan.md`, with §0 Round 3 and §9 the α record). Keep it as `test-plan.v3a.md`, then record β in a new §10 (rev 4).
2. **Implement S2a-i-β only** (T1): `Device::apply`'s and `rx_stop`'s `at` leave the trait (A1, A1′); FakeDevice's timed command queue, `settle`, `FakeConfig::command_queue`, the DSP FIFO and its flow parts go, keeping `Dsp.log` (frequency history), `tx_header` and `tx_header_stale` (S12); `UhdDevice::tune_dsp`/`tune_auto` keep `at` as UhdDevice-only bench methods (S10, T2); A5 per rev 3 (`Channels::without` has a caller at rx.rs:1707: decide by rev 3, report); N1, N2, N10, N11 and every β test rev 3 names; the D9 table (S4); the rows rev 3 decides for β (P215, P219, P369 only if B7 is β — otherwise later, P985, P990) plus the mechanical sweep; the spec text rev 3 assigns to β (UR-2, UR-33, UR-22 at design/18:249). Do not start S2a-ii or S2b. This session has no time limit: do the whole of β, keeping the tree green; do not stop because the work is large. Work in passes as you need, but report only when all of β is done and the tree is green.
   - Follow the plan.
   - Extend it (§10, the β implementation record) for every state or failure path the implementation adds.
   - **Every planned test of S2a-i is written. None is a "gap" to report.** If a planned test cannot be written as planned, say why with evidence, and give what replaces it.
   - Do not start S2a-ii.

### Rules

**Mutation rows**
- Make text edits only, in `plan/maintenance/tools/mutations.json` and `plan/*/tools/mutations.json`. Never re-serialize the file.
- Use Rust token anchors. Each row matches exactly once and names its killing test.
- The mechanical sweep (S3): after the code change, a script lists every row that is `NOT FOUND`. Save its output as `sweep-a1b.txt`. Delete only the listed rows whose behaviour is gone. Each row on retained code is decided as rev 3 says.
- Confirm each new or retargeted row in a throwaway copy outside the repository:
  - copy: `rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3`;
  - build with `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`;
  - run `find <copy> -type f -exec touch {} +` before building;
  - use one copy at a time;
  - a compile failure is not a kill;
  - logs go in `probes-a1b/`.
- Never run `mutate.py`, `remote_mutate.py` or `gate.py`. Never apply a mutant to the tree.
- Existing rows whose `old` lies in a function you change: re-run them in the copy too.

**Runs and state**
- Run single tests with `timeout 300`. Your tool stops a command at 120 s, so run longer commands with a longer timeout, in the foreground. Leave no process running.
- Test temp files go on `/tmp`.
- Make no git state changes, no version bumps, and no `SCHEMA_CHANGELOG` entry. Regenerate any changed schema, so `schema_freeze` and `rm_20_schema_freeze` are green.
- Write what happened, never what was not done.

**End checks**
- `cargo fmt --all`.
- clippy `-D warnings` for the workspace, and for `-p ezsdr-radio-uhd --features uhd --all-targets`, both via `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`.
- The remote workspace tests, after your last edit.
- The Python 3.14 suite (handoff §5).
- `git ls-files -m -o --exclude-standard | xargs chmod go+r`.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` must give 0. If you rename or drop tests, list each with its reason in `plan/phase8/reviews/48s2a1b-renames.txt`, in the format of `48s1-renames.txt`, and pass `--renames`.

### Self-review (one round)

Run it in the foreground with `timeout 2400`. Use free DeepSeek first:

`opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"`

Use Go (`opencode-go/deepseek-v4.1-flash#max`) only if free refuses.

Checklist:
1. Every S2a-i-β test in rev 3 is written, observes what the plan says, and its row breaks it.
2. Scope against rev 3: missing and extra (nothing of S2a-ii or S2b).
3. Every "no caller / unreachable" claim behind a deletion: grep the callers.
4. Every `design/` text this sub-step makes false is amended in this sub-step.
5. Maintainability: no leftovers of the deleted paths. No retained behaviour lost its only test.

Fix every finding, or reject it with a reason.

### Report (at most 15 lines)

- Rev 3: per ruling, what changed.
- S2a-i: what was deleted and what was kept.
- Tests: added, moved and deleted, with counts.
- Rows: deleted, retargeted and added, with probe outcomes and the sweep.
- Suites and `rules.py`.
- Self-review findings and their disposition.
- Deviations, and open questions with your recommendation.

---

## `brief-impl-a1b2.md`

## Brief: design note 48 S2 — finish sub-step S2a-i-β (continuation)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`. Nobody else edits the tree.

The previous run of `brief-impl-a1b.md` stopped before it reported. It had started a probe batch in the background and then ended its turn. That batch has since finished. Its logs are in `~/.cache/ezsdr-fixes/2026-11-05-n48s2/probes-a1b/` (`summary.txt` and one log per row). The tree holds that run's uncommitted β changes: `git diff HEAD`. Keep them.

### Task

Read `brief-impl-a1b.md` in full. Its task, rules, self-review and report format all apply. Then finish β from where the tree is:
1. Read the probe results (P985, P986, P1201, P1210 and the rest).
2. Fix or retarget any row that is not killed.
3. Make the spec edits.
4. Do the mechanical sweep (`sweep-a1b.txt`).
5. Update the test plan's §10, the β implementation record (rev 4; keep the current file as `test-plan.v3a.md` if that is not done already).
6. Run the end checks.
7. Run the self-review round.
8. Report.

**Never leave a command running in the background when you end your turn.** Run every probe batch, test and self-review in the foreground with a long enough `timeout`, and wait for it. A run that ends while its own background job is still going stops without a report.

---

## `brief-sol-code-a1b.md`

You are the independent reviewer (Sol), read-only, in ~/local_work/Ez-SDRv4 (HEAD 991d52a; the sub-step's changes are uncommitted: `git diff HEAD`, `git status`). Do not edit, format, build in the tree or commit; never run mutate.py or remote_mutate.py. To run a test, use a throwaway copy (`rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3 ~/local_work/Ez-SDRv4/ /tmp/sol-s2a/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, `timeout 300`).

AGENTS.md §9 step 7: review the code of sub-step S2a-i-β of design note 48 S2 — removing the timed device paths: `Device::apply`'s and `rx_stop`'s `at`, `rx_start_now`, FakeDevice's timed command queue, DDC/DUC FIFOs and flows (keeping `Dsp.log` frequency history and the TX header state), with `UhdDevice::tune_dsp`/`tune_auto` kept as bench-only methods. α (the queue measurement group) is committed at HEAD. Inputs in ~/.cache/ezsdr-fixes/2026-11-05-n48s2/: brief.md, rulings-plan.md (R1–R10, S1–S12, T1–T3), test-plan.md (rev 4: §10 is the β implementation record and deviations), sweep-a1b.txt, probes-a1b/ and probes-a1b2/ (row probe logs), selfreview-a1b2.out and the end of impl-a1b2.log (the implementer's report: P1224 deleted, design/18:249 left to S2a-ii, `hw_n47_raw_control_call_durations` now timing `tune_auto`). You reviewed rev 1 of this plan (sol-plan.log).

First write your own failure scenarios for this change, then check:
1. G1/G2 hold: untimed apply, coercion, read-back, refusal, `apply_delay` (N1/N2), sample-time frequency history (the loopback split test), front-end preservation, stale TX header and P5/D70 packet state, the T3 flush and its report attribution from S1. Did any deleted or rewritten test (19 deleted, 8 renamed/retargeted) take the only pin of a reachable behaviour? Check against rev 3's map and rulings S1–S12.
2. FakeDevice after the FIFO/flow removal still models what the Module relies on (receive start/stop, stale header, DDC restart, sample production) and is not looser than the hardware it stands for.
3. Every production caller of `apply`/`rx_stop` passed `None` before (the deletion changes no production behaviour); `UhdDevice` lost no untimed path; the bench methods keep `at`.
4. Mutation rows: 18 deleted, 4 added, 4 retargeted, P1224 deleted (its behaviour only in a hardware probe): each disposition right; no row on retained code dropped; the sweep complete.
5. design/ text: UR-2, UR-33 and any other text β makes false amended now; design/18:249 (UR-22's timed TX DSP sentences) correctly left to S2a-ii (A4); nothing of S2a-ii/S2b done early.
6. Maintainability per AGENTS §9 (leftovers of the deleted paths, write-only state, stale comments).
For each major finding give a minimal sequence and the observable wrong outcome. Output: scenario list (short), findings P1/P2/P3 with path:line, verdict CODE OK / CODE NEEDS CHANGES. At most 120 lines.

---

## `brief-fix-a1b.md`

## Brief: design note 48 S2a-i-β — fix round 1

You are the implementer (`anthropic/claude-haiku-5-5#high`). You work in `~/local_work/Ez-SDRv4`, HEAD `991d52a`. The tree holds your uncommitted β changes; keep them. Nobody else edits the tree.

Read these files in `~/.cache/ezsdr-fixes/2026-11-05-n48s2/`:
- `brief-impl-a1b.md`: its rules and end checks apply.
- `test-plan.md` §10.
- Sol's review at the end of `sol-code-a1b.log`.

### Task (Sol P2, adopted)

The deleted P4 test also checked that case C really closes and reopens the streamer and that case A does not. This check does not depend on the FIFO. Restore only that check:
- Write a small FakeDevice sequence test of `p4_dsp_only` (`tests/common/d9.rs` ~225–229) with an untimed or no-op tune closure.
- Assert `rx_close`/`rx_open` for `DspOnlyReopen`, and assert they are absent for case A.
- Retarget P1224 to this test. Do not delete P1224. Confirm the kill in a throwaway copy (`probes-a1b3/`).
- Record this in §10 and in the renames file if needed.

Change nothing else. Also do the orchestrator's two notes:
1. P1224 is no longer deleted (it is retargeted now).
2. Leave `design/18:249` and the `hw_n47_raw_control_call_durations` rename for S2a-ii. Record both in §10 as carried to S2a-ii.

Run all of the end checks in the foreground. No self-review round is needed for this fix.

### Report (at most 6 lines)

Give the test, the row and its probe, and the suites. Run `rules.py`.

---

## `brief-fix-a1b2.md`

## Brief: design note 48 S2a-i-β — gate fix (mutation P1187 survives)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`. HEAD is `991d52a`. Your β changes are uncommitted in the tree; keep them. Nobody else edits the tree. The rules and end checks of `brief-impl-a1b.md` apply.

### What failed

The orchestrator's gate (`gate-a1b/mutation.log`) reports that row **P1187** now SURVIVES:
- name: "a one-sample EOB drops the next timed header (D9b-1a)";
- file: `device.rs`;
- change: `self.eob_lost = true` → `false`;
- filter: `ur_33_a_one_sample_eob_loses_the_next_timed_header`.

The same row was killed at HEAD `991d52a`. So β changed either the FakeDevice DUC model or this test's path, and the killing test passes without the lost header.

### Task

1. In a throwaway copy, find why. Compare the DUC/`Duc::packet`/`has_time`/`eob_lost` logic and the test at HEAD and in the tree. Check whether the flow or FIFO removal dropped a step that consumed `eob_lost`, or whether something else now makes the next packet start early.
2. If β lost behaviour that the hardware has (a one-sample EOB loses the next timed header, D9b-1a, hardware-verified), restore it in the model. If instead the test now passes for a wrong reason, make it observe the header loss again.
3. Show that P1187 is killed again in the copy (logs in `probes-a1b4/`).
4. Re-probe the other rows in the functions you touch.
5. Record what you found in `test-plan.md` §10.

Change nothing else. Run the end checks in the foreground. No self-review round.

### Report (at most 6 lines)

The cause with path:line, the fix, the probes, the suites, `rules.py`.
