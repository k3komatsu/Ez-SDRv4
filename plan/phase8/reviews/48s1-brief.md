# Note 48 S1 — briefs

---

## `brief.md`

## Brief: design note 48 S1 — fix #78 and time only the first send of a device burst (TEST PLAN FIRST)

You are the implementer (`anthropic/claude-haiku-5-5#high`, OpenCode2) in `~/local_work/Ez-SDRv4`, branch `main`, HEAD `6ccc1c5`, clean tree. This is a **risky** step (AGENTS.md §9: it changes the transmit order, the report correlation and the fake/hardware boundary). **This task is the test plan only**: write it to `~/.cache/ezsdr-fixes/2026-11-05-n48s1/test-plan.md` and stop. Do not change the tree.

Read: `AGENTS.md` (Safety core, §6, §7, §9 with Maintainability and Counterexamples); `plan/phase8/48-simplification-inventory.md` (§3.2 B1, B2, §3.3 C2/C3, §4 S1); issue #78 with its analysis comment (`gh issue view 78 --comments`); `plan/phase8/bench-results.md` §「2026-11-05」; `plan/phase8/39-dsp-timed-commands.md` §6.6 (D9b-1a: why every send was timed, the two-sample close, the safe start); spec `design/18-uhd-radio.md` UR-22, UR-23, UR-28, UR-33 and `design/07-radio-model.md` RM-14/RM-15; the code `crates/ezsdr-radio-uhd/src/provider/tx.rs` (`send` ~510-700, `close_device_burst` ~740, `front_late_by`/`dispatch_start` ~897-945, report correlation ~355-409), `provider/core.rs` (`tx_flush_locked` ~1102-1181 and the report stash), `src/device.rs` (FakeDevice `Duc`/`Radio` ~602-735, `tx_send` ~1856); and the existing tests that pin these (`tests/fake.rs`, `tx.rs`/`core.rs` unit tests).

### The step (four parts, in this order)

**(a) The T3 flush's Underflow reports leak into the Run (VERIFIED on the bench).** On X300 (UBX and OBX), the two-zero untimed SOB/EOB flush at open draws about 9 `Underflow` async reports before its `BurstAck` (`tx_flush: true`). Evidence: `hw_b6_txrx_and_repeat` on #1: async `[BurstAck 672710 tx_flush, Underflow 672320, 672367, …, 672702, BurstAck 404809601]`. The Underflows are at device ticks before the flush's ACK. The burst itself played to its ACK. The raw P5 probe shows the same. The Module turned them into `radio.TX_UNDERFLOW { cause: starved }`. Required: reports of the flush's channels between the flush send and its ACK (or its deadline) belong to the flush. They become no Run event and no Manifest async row counted as the Run's; record them as the flush's (say where). A real underflow of a later burst is still reported. FakeDevice must model the flush's Underflows (UR-33), so the fake reproduces the bench.

**(b) A burst at the previous burst's end + 1 sample is dropped as late.** Bench (#1): `hw_b8_burst_one_sample_after_a_burst` → `radio.TIME_ERROR { cause: late, late_by_ns: 1000, outcome: drop }` for B at A's end + 1 sample. The device reported no TimeError. The Module decided it. UR-22 says: "After a successful close, the next safe start is the closing send's exclusive end plus one sample". The front advances at `tx.rs:691,727,802,869`. Find the reason for the "+ one sample" in design note 39 §6.6 and the bench records (VERIFIED or INFERRED). Then state what B at `e + 1` and B at `e` should do, and why. Test expectations: the existing hardware test was written for B to play (Review O, O-B1). Propose the rule. The orchestrator and Sol decide in the plan review; do not change UR-22 on your own.

**(c) `hw_p70_one_sample_burst_then_timed_burst` reports TIME_ERROR 1 and UHD prints about 32 `L` per trial**, the two-sample control included. Unexplained. In the plan: what the trial sends (a 1- or 2-sample burst at `a`, then a 10 000-sample tone at `a + 25 µs`), which of (a), (b), (d) can explain an `L`, and a FakeDevice sequence that would reproduce it if one does. If none does, plan only the diagnostic for the next bench: print the async section and the per-send rows in `hw_p70_…`.

**(d) B1: time only the first send of each device burst.** Continuations, held-tail sends, sends of adjacent Kernel bursts that continue the device burst, and closing sends go untimed. A closing send keeps at least 2 samples (#70). Note 48 B1 lists the facts and what each depends on: chunking uses cursors, correlation uses device-burst starts, T3 is independent, the underflow abandonment stays. FakeDevice's DUC already models untimed continuations (`Duc::packet`). Close #72 as obsolete (the orchestrator does that at the gate).

### Sequences that matter (each needs a test; expected result)

- F1 open a transmit stream → T3 flush; the fake emits N `Underflow` then the flush's `BurstAck`. Expected: no `radio.TX_UNDERFLOW` in the Run. The flush is acknowledged. The reports are recorded as the flush's.
- F2 F1 then a burst that underflows in the middle (fake fault or a late host send). Expected: exactly that underflow is reported (UR-28 correlation unchanged).
- F3 a recovery reopen (uncertain close → reopen → T3) during a Run. Expected: the second flush's Underflows are not the Run's either. Generations keep reports of the old streamer apart.
- F4 A ends at `e`, B booked at `e` (adjacent: continues the device burst), at `e + 1`, at `e + 2`, and one far later. Expected per (b)'s rule. Each played or reported exactly as the rule says.
- F5 a long burst sent in many chunks, a repeat across its wrap, two adjacent Kernel bursts, a held tail. Expected: only the device burst's first send carries `at`. The fake's DUC plays the samples contiguously at the planned instants. Every sample is in place.
- F6 a send accepted short or failed mid-burst, then the close. Expected: today's abandonment/recovery (C2) unchanged. A closing send of ≥ 2 samples.
- F7 an underflow mid-burst with untimed continuations. Expected: the Module abandons the burst. Later untimed samples do not resume on a shifted timeline (`tx.rs:1131-1139`).
- F8 per-channel ACK correlation with two channels (grouped and split), and a device burst that spans two Kernel bursts. Expected: each Kernel burst retired by the right ACK.
- F9 MockRadio parity: whatever (b) changes in the safe-start rule, MockRadio's front (MR-17, MR-25) changes the same way, and the timeline differential agrees.
- Your own failure scenarios beyond F1–F9.

Hardware/UHD facts these rely on: mark each VERIFIED (path:line or URL, or bench log) or INFERRED. Note 48 B1 and `reviews/48-sol.md` cite UHD 4.10 sources for untimed continuations; the flush's Underflows are VERIFIED by the bench logs (`~/.cache/ezsdr-bench/2026-11-05/*.log`, outside the repo; read them).

### The test plan

Per AGENTS §9 step 2. For each sequence and each part (a)–(d), give:
- the test (file, name), what it asserts, how the assertion observes the behaviour, and the wrong behaviour that fails it;
- the existing tests and mutation rows that change, are retargeted or are deleted, with the reason;
- the mutation rows you will add (Rust token anchors; `plan/maintenance/tools/mutations.json`, text edits only), with their killing tests;
- the spec text each part changes (UR-22/23/28/33, RM-15, MR-17/MR-25 if (b) changes the rule; KNOWN_LIMITATIONS if any).

Keep it to what the step needs. **Do not add mechanisms beyond the step**: report a gap instead.

### Rules

No tree changes in this task. Write what happened, never what was not done. If you run a test, use `timeout 300`. Your tool stops a command at 120 s: run longer commands with a longer timeout in the foreground.

### Report (at most 15 lines)

Where the plan is. Your proposed rule for (b), with evidence. What (c) can and cannot explain. Open questions for the orchestrator, each with your recommendation. Claims you could not verify.

---

## `brief-impl.md`

## Brief: design note 48 S1 — implementation (rev 4 of the test plan)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `1f120dc`, clean tree. Read `AGENTS.md` (Safety core, §6, §7, §9 with Maintainability), `brief.md`, `rulings-plan.md`, `rulings-plan2.md`, `hardware-facts-cpp.md` and the test plan `test-plan.md` rev 4 (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s1/`).

Orchestrator's answers to rev 4 §15: Q1 keep the cut refusal (FS-4), no window refusal; Q2 approved: reopen front = flush end + 1 sample (`at_or_after(tick)`, no `+ 2`), TBD-d2 becomes unconditional; Q3 the bench print `hw_s1_short_first_sends` is written now and run at the next bench; Q4 labels are decided at the orchestrator's review, not by you; Q5 accepted as written (FS-9); Q6 accepted (wait for window space, never send short).

### Task

Implement rev 4 in the tree: parts (a) four-zero T3 flush and bounded flush-report attribution, (b) R1 retargets, (c) the first-send rule FS-1..FS-9 (`MIN_FIRST_SEND`), FakeDevice's undersized-first-packet model and ACK at end + 1 tick (UR-33), N4's strict ACK correlation, the reopen front, MockRadio parity (M1, MR-17, MR-25), the hardware-test changes (§13, including the small `hw_p70` diagnostic print and `hw_s1_short_first_sends`), the specs and KNOWN_LIMITATIONS rows (§12) with Changes rows, and the mutation rows (§11) with their killing tests. Part (d)/B1 is OUT: every send stays timed; only UR-22 T1's Changes row records the new reason. Extend the plan (rev 5, keep rev 4 as `test-plan.v4.md`) for every state or failure path the implementation adds (AGENTS §9 step 4). Do not add mechanisms beyond the plan — report a gap instead.

### Rules

- Mutation rows: text edits only in `plan/maintenance/tools/mutations.json` (never re-serialize); Rust token anchors; each matches exactly once; each names its killing test. Confirm each new or retargeted row in a throwaway copy outside the repository (`rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, `find <copy> -type f -exec touch {} +` before building, one copy at a time); a compile failure is not a kill; logs under `~/.cache/ezsdr-fixes/2026-11-05-n48s1/probes/`. Never run `mutate.py`, `remote_mutate.py` or `gate.py`; never apply a mutant to the tree.
- Existing rows whose `old` text lies in a function you touch: re-run them in the copy too (stage 1b-4d-2 lesson: shape changes surface only at the gate otherwise).
- Single test runs with `timeout 300`. Your tool stops a command at 120 s: run long commands with a longer timeout, in the foreground. Leave no process running. Test temp files on `/tmp`.
- No git state changes, no version bumps, no `SCHEMA_CHANGELOG`.
- Write what happened, never what was not done.
- End checks: `cargo fmt --all`; clippy `-D warnings` for the workspace and `-p ezsdr-radio-uhd --features uhd --all-targets` (via `python3 plan/maintenance/tools/remote/rdev.py -- <cmd>`); workspace tests; the Python 3.14 suite (handoff §5); `git ls-files -m -o --exclude-standard | xargs chmod go+r`; `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` → 0 (if you rename or drop tests, list them with a reason in `plan/phase8/reviews/48s1-renames.txt` in the format of `47s3-renames.txt` and pass `--renames`).

### Self-review (one round, AGENTS §9 step 5)

Before reporting: `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"` in the foreground (if the free tier refuses, `opencode-go/deepseek-v4.1-flash#max`). Checklist: (1) every planned sequence has its test, the test observes what the plan says, and its mutation row breaks it; (2) scope against the plan, missing and extra (part (d) must be absent); (3) every "every/only/all" claim (every first send >= 4 per channel, only the flush's own channels' reports attributed, every ACK correlation strict) checked against all call sites; (4) every `design/` text the change makes false is amended; (5) maintainability (AGENTS §9): no duplicated decision between UHD and Mock beyond the parity the plan names, no new flag that duplicates a lifecycle fact. Fix or reject (with reason) every finding.

### Report (at most 20 lines)

What is implemented per plan part; tests added/changed; rows added/retargeted/deleted with probe outcomes; suites; `rules.py`; self-review findings and their disposition; deviations and why; open questions.

---

## `brief-impl2.md`

## Brief: design note 48 S1 — continuation: write the planned tests that are missing

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `1f120dc`. The tree holds the previous run's uncommitted S1 implementation; keep it. Read `AGENTS.md` (Safety core, §7, §9 with Maintainability), the prior brief `brief-impl.md` (its Rules and End checks apply unchanged), and `test-plan.md` rev 5 (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s1/`), especially §17.

**About the previous run's stop:** it reported a "concurrent agent" PID 79585. That process was the previous run itself (the `opencode run` client of its own session). There was only one implementation session; every change in the tree is that run's. Nobody else edits the tree now. The watchdog line "mutant P1116 looks applied" is a false positive: FS-3 rewrote the text P1116 anchors; the row was retargeted.

### Task

§17.3 lists planned tests as "gaps, not added". They are not new mechanisms: they are rev 4's planned tests, and the step is not done without them. Write them now, each with what rev 4 says it asserts, and give each new row (or a retargeted one) its killing test:

1. **Parity (`crates/ezsdr-acceptance/tests/parity.rs`, §9.3):** `s1_parity_short_burst_front_s_s3_s5`, `s1_parity_stop_during_padded_device_burst` (also the killer of TBD-m2, the Stop front's floor), `s1_parity_grouped_and_split_fronts`, `s1_parity_payload_cut_separate_from_padding`, `s1_parity_short_burst_then_adjacent_burst_both_orders`. **Skip** `s1_parity_one_sample_continuation_is_padded_to_two`: the Mock treats each Kernel burst as its own device burst (record the reason in §17).
2. **Fake (`crates/ezsdr-radio-uhd/tests/fake.rs`):** every fake test §17.3 lists (F6-c, F4-late, F4-adj, F5-d, F5-e, §8.2 ACK tick, §8.4 reopen front, the loss/stop while the first send waits, the wait past the start lead, the two N3 adjacent variants, FS-7 / TBD-b4). Where a test already exists in `device.rs` or `tx.rs` that observes the same thing through the same path, name it in §17 instead of writing a copy (AGENTS §9 Maintainability 3).
3. **`tx.rs`:** FS-T4 `ur_22_a_padded_send_counts_payload_only`; the repeat tiling for lengths 1 and 3 (extend the existing two-sample test to 1–3 rather than adding a test); the four T3 flush tests (§17.3); F8-b, F8-c, F8-d.
4. **Acceptance:** `ur_23_a_four_sample_window_takes_a_first_send_of_four`, `ur_23_a_preempted_first_send_restarts_its_count` (rev 4 §7).
5. **Bench (`hardware.rs`; compile-checked only, run at the next bench):** `hw_s1_short_first_sends` (print-only, Q3 of the prior brief: Module bursts of 1, 2, 3, 4 samples at 1 MS/s, each followed by a 10 000-sample burst; print every async report and per-send row); retarget `hw_p70_one_sample_burst_then_timed_burst` to the four-sample control with the small diagnostic print (§5.4); the payload-count comparison at `hardware.rs:2032–2033` (F-28); the `e + 1` trial of `hw_b8_burst_one_sample_after_a_burst` (§13: B at `e + 1` gives one late TimeError, B at `e + 2` plays).

A test that cannot be written as planned (the behaviour is unreachable, or the planned observation cannot see it): say why in §17 with evidence, and propose what replaces it. Do not change production code except to fix a defect a new test exposes; report every such fix.

### Rules

As in `brief-impl.md` (probes in a throwaway copy outside the repository, logs in `probes2/`; never `mutate.py`, `remote_mutate.py`, `gate.py`; no git state changes; `timeout 300` per test run). New tests whose verdict depends on timing (threads, wall clock) are repeated 20 times; pure ones are not.

### Self-review

One round, as in `brief-impl.md`, on the diff of this run only (the checklist's item (1) against §17.3's list). Fix or reject with a reason every finding.

### Report (at most 15 lines)

Tests written per item; tests skipped or merged with the reason; rows added or retargeted with probe outcomes; production fixes, if any; suites; `rules.py`; self-review findings and disposition. Update `test-plan.md` §17 (rev 6; keep rev 5 as `test-plan.v5.md`).

---

## `brief-fix1.md`

## Brief: design note 48 S1 — fix round 1

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `1f120dc`, with the S1 implementation uncommitted in the tree (yours, from `brief-impl.md` and `brief-impl2.md`). Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9), `brief-impl.md` (its Rules and End checks apply), `test-plan.md` rev 6 §17, Sol's review at the end of `sol-code.log` and the rulings `rulings-code.md` (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s1/`).

### Task

Fix C1–C5 exactly as `rulings-code.md` rules. For C1 and C2 write the failing test first, confirm it fails on the current code, then fix. Add or retarget the mutation rows: TBD-m2 (C1) and a row for C2's lead condition, each with its killing test, probed in a throwaway copy (logs in `probes3/`). Re-probe the existing rows whose `old` text lies in code you change (N48S1-14 at least).

Do not change anything else. If a fix needs a production change the rulings do not name, stop and report it.

### Self-review

One round on this round's diff: `opencode2 run --standalone -m opencode/deepseek-v4.1-flash-free#max --agent plan --auto "<checklist>"` (Go `opencode-go/deepseek-v4.1-flash#max` only if free refuses), in the foreground, `timeout 2400`. Checklist: each of C1–C5 done as ruled; each new test fails without its fix; the C2 change reuses `dispatch_start`'s lateness test; no other behaviour changed.

### Report (at most 12 lines)

Per C1–C5: what changed, the test, the row and probe outcome. Suites, `rules.py`, self-review findings and disposition. Update `test-plan.md` §17 (rev 7; keep rev 6 as `test-plan.v6.md`).

---

## `brief-fix2.md`

## Brief: design note 48 S1 — fix round 2

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `1f120dc`, with the S1 implementation and fix round 1 uncommitted in the tree (yours). Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9), `brief-impl.md` (its Rules and End checks apply), `test-plan.md` rev 7 §17, and `rulings-code.md` "Round 2" (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s1/`).

### Task

Do D1, D2, D3, D4 and D6 exactly as ruled (D5 and D7 need nothing). For D1 and D4 write the failing tests first and confirm they fail on the current code (record the failure lines in §17). D1 changes MockRadio's receipt decision (`crates/ezsdr-mock-radio/src/lib.rs`, the `tx_handed_front` / late-policy decision around 1353–1640): reuse the existing front, floor and late-policy helpers; do not add a second decision path. Mutation rows for D1, D4 and D6 with their killers, probed in a throwaway copy (logs in `probes4/`); re-probe N48S1-16 after D2 and every existing row whose `old` lies in code you change.

If D1 cannot be done without moving the Mock's decision point from receipt to hand-over, stop and report why with the sequence that needs it; do not do that move.

### Self-review

One round on this round's diff, free DeepSeek first (`opencode/deepseek-v4.1-flash-free#max`), Go (`opencode-go/deepseek-v4.1-flash#max`) only if free refuses; foreground, `timeout 2400`. Checklist: each of D1–D6 done as ruled; each new test fails without its fix; D1 reuses one decision path; no other behaviour changed.

### Report (at most 12 lines)

Per item: what changed, the tests, rows and probe outcomes; suites (parity, `ezsdr-mock-radio`, `ezsdr-radio-uhd` lib and fake, remote workspace tests after your last edit, clippy, fmt, Python, `rules.py --renames`); self-review findings and disposition. `test-plan.md` §17 → rev 8 (keep rev 7 as `test-plan.v7.md`).

---

## `brief-fix3.md`

## Brief: design note 48 S1 — fix round 3 (revert D1)

You are the implementer (`anthropic/claude-haiku-5-5#high`) in `~/local_work/Ez-SDRv4`, HEAD `1f120dc`, with S1 and fix rounds 1–2 uncommitted in the tree (yours). Nobody else edits the tree. Read `AGENTS.md` (Safety core, §7, §9), `brief-impl.md` (its Rules and End checks apply), `test-plan.md` rev 8 §17, and `rulings-code.md` "Round 3" (all in `~/.cache/ezsdr-fixes/2026-11-05-n48s1/`).

### Task

Do E1 exactly: remove D1 and everything that exists only for it (code, tests, rows, spec sentence, record lines), keeping D2, D3, D4, D6 and their tests and rows. The Mock's receipt decision must be what it was after fix round 1 plus D4 (compare with `test-plan.v7.md` §17 and your round-2 diff). List in §17 each item removed and each round-2 item kept, with the reason. Rows that remain must still match exactly once; re-probe in a throwaway copy (logs in `probes5/`) every kept row whose `old` lies in the Mock functions you change, and confirm the round-1 Mock parity tests (C1) still pass.

Change nothing else.

### Self-review

One round on this round's diff, free DeepSeek first (`opencode/deepseek-v4.1-flash-free#max`), Go (`opencode-go/deepseek-v4.1-flash#max`) only if free refuses; foreground, `timeout 2400`. Checklist: nothing of D1 left (search the names it added); D2/D3/D4/D6 intact; no other behaviour changed.

### Report (at most 10 lines)

Removed, kept, rows and probes, suites (parity, `ezsdr-mock-radio`, `ezsdr-radio-uhd` lib and fake, remote workspace after your last edit, clippy, fmt, Python, `rules.py --renames`), self-review. `test-plan.md` §17 → rev 9 (keep rev 8 as `test-plan.v8.md`).

---

## `brief-sol-plan.md`

You are the independent reviewer (Sol), read-only, in ~/local_work/Ez-SDRv4 (HEAD 6ccc1c5, clean). Do not edit, format or build in the tree.

Review the test plan of a risky step before any code: ~/.cache/ezsdr-fixes/2026-11-05-n48s1/test-plan.md, written for the brief ~/.cache/ezsdr-fixes/2026-11-05-n48s1/brief.md (design note 48 S1: fix issue #78 and time only the first send of a device burst; read plan/phase8/48-simplification-inventory.md §3.2 B1, §4 S1 and `gh issue view 78 --comments`).

Hardware facts measured TODAY with a minimal UHD 4.10 C++ program on both X300s (no Ez-SDR code): ~/.cache/ezsdr-fixes/2026-11-05-n48s1/hardware-facts-cpp.md — read all of it including both addenda. Addendum 2 (split sends) was measured AFTER the plan was written and overrides the plan where they differ: a device burst's FIRST send must carry >= 4 samples per channel; padding only the closing send (the plan's (c)) is not enough (1+3 and 2+3 splits still fail intermittently; 4+2 is clean). The program is uhd-repro/tx_repro.cpp beside it.

AGENTS.md §9 step 3: first write your own failure scenarios from the step's purpose, the code (crates/ezsdr-radio-uhd/src/provider/tx.rs, core.rs flush, src/device.rs FakeDevice Duc/Radio, the Mock's mirror in crates/ezsdr-mock-radio/src/lib.rs MR-17/MR-25) and the facts; only then compare with the brief's sequences F1–F9 and the plan. Check in particular:
1. (a) a four-zero T3 flush plus attributing flush-window reports: is the attribution window right (reports of other channels/streams, a flush with no ACK by its deadline, a recovery flush during a Run with a burst's own underflow nearby)?
2. (b) R1 (keep tail+zero close, retarget the hardware test to e+2) vs holding back two real samples: which is right given the facts, the Mock parity (MR-17) and the phase-7 records the plan cites (F-5..F-8)?
3. (c) with addendum 2: what must the Module guarantee about a device burst's first send (>= 4 samples per channel) for every path — a Kernel burst shorter than 4 samples, an open-ended burst whose first buffer is short (window limits, `avail < wanted`), a held tail, a continuation of a device burst (does it matter there?), repeat; what the Mock must mirror; whether FakeDevice must model "first packet < 4 samples is dropped/underflows" (UR-33) so tests fail without the fix.
4. (d) B1: the plan's handling of untimed continuations, the underflow abandonment, ACK correlation.
5. Every sequence has a test that observes it and a mutation row that breaks it; spec rows (UR-22/23/28/33, MR-17/MR-25, KNOWN_LIMITATIONS) complete; anything missing or extra (no mechanisms beyond the step).
For each major finding give a minimal operation sequence and the observable wrong outcome. Output: your scenario list (short), findings P1/P2/P3 with path:line or plan section, answers to the plan's §9 decisions, verdict PLAN OK / PLAN NEEDS CHANGES. At most 150 lines.

---

## `brief-sol-code.md`

You are the independent reviewer (Sol), read-only, in ~/local_work/Ez-SDRv4 (HEAD 1f120dc; the step's changes are uncommitted: `git diff HEAD` plus `git status`). Do not edit, format, build in the tree or commit. If you need to build or run a test, use a throwaway copy outside the repository (`rsync -a --no-times --exclude .cargo --exclude target --exclude .git --exclude v3 ~/local_work/Ez-SDRv4/ /tmp/sol-s1/`, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0`, `timeout 300` per test run). Never run mutate.py or remote_mutate.py.

AGENTS.md §9 step 7: review the CODE of a risky step independently — design note 48 S1 (issue #78). Inputs: the brief ~/.cache/ezsdr-fixes/2026-11-05-n48s1/brief.md (but its part (d)/B1 is OUT by the owner: every send stays timed), the rulings rulings-plan.md and rulings-plan2.md, the hardware facts hardware-facts-cpp.md (with both addenda; a device burst's first send must carry >= 4 samples per channel on X3x0; ACK at burst end + 1 tick), and the final test plan test-plan.md rev 6 (§17 is the implementation record: deviations, what was not written and why, self-review dispositions). All in ~/.cache/ezsdr-fixes/2026-11-05-n48s1/. You reviewed rev 2 of this plan earlier (sol-plan-final.md there).

First write your own failure scenarios from the step's purpose and the changed code, then check the code and tests against them and against the plan. Code: crates/ezsdr-radio-uhd/src/provider/tx.rs (`step`, `send`, `refuse_first_send`, the report correlation, the reopen front), provider/core.rs (`tx_flush_locked` attribution), src/device.rs (FakeDevice `Radio::packet` undersized model, ACK tick), crates/ezsdr-mock-radio/src/lib.rs (the Mock floor), tests in tx.rs/core.rs/device.rs, tests/fake.rs, crates/ezsdr-acceptance/tests/parity.rs, tests/hardware.rs (compile-only bench tests), design/07, 09, 18, KNOWN_LIMITATIONS.md, and the mutation rows N48S1-* plus the retargeted rows in plan/maintenance/tools/mutations.json and plan/phase7/tools/mutations.json.

Check in particular:
1. Every path that can start a device burst sends >= 4 samples per channel in its first send, or refuses/pads as the plan says (short Kernel bursts, window-limited first buffers, holds, cuts, repeats, adjacency to a previous burst, a preemption).
2. The `step()` change (an open first send whose start passed is put back to `held` and judged by its late policy again): can it loop, double-report, lose a booking record or ACK correlation, or change behaviour for bursts that used to resume?
3. Flush attribution: reports of other channels/streams/generations, a flush with no ACK by its deadline, a recovery reopen during a Run, a real Run underflow next to a flush.
4. MockRadio vs Module front parity (MR-17/MR-25) after the floor; whether the skipped and unwritten tests in §17 leave a reachable behaviour unpinned, and whether each "not written" reason holds.
5. Each new or retargeted mutation row breaks the behaviour it names and its filter test kills it; rows that test nothing; leftovers the step made obsolete.
6. design/ text and KNOWN_LIMITATIONS made false or missing; maintainability per AGENTS §9 (duplication between UHD and Mock beyond named parity, state kept twice, test quality).
7. §17's open item: in a probe the Module path judged bookings submitted at 97–98 ms (for a burst near 100 ms) late by 1–2 ms while the Mock did not; the parity tests moved to 91 ms. Is this a Module/Mock divergence the step introduced or exposes, and does moving the tests hide it?

For each major finding give a minimal operation sequence and the observable wrong outcome (AGENTS §9 Counterexamples). Output: your scenario list (short); findings P1/P2/P3 with path:line; verdict CODE OK / CODE NEEDS CHANGES. At most 150 lines.
