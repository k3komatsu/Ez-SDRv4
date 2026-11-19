# Stage 4b — briefs

## design-brief

## Design brief: stage 4b — the overflow restart gap (RM-17) and #108 (a receive restart booked just after an overflow starts late)

You draft the design (AGENTS.md §9 step 0) for Phase 8 stage 4b in the Ez-SDR v4 repository at `/home/komatsu/local_work/Ez-SDRv4` (branch `main`, commit ce631495). READ-ONLY: edit nothing in the repository, run no cargo build or test, no git writes. Write your draft to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/design-draft.md` (that file only) and finish with a short summary (the owner questions, one line each, with your recommendation).

### Read first

- AGENTS.md §3, §6, §9 step 0; `plan/phase8/50-remaining-work.md` §2 (where 4b sits).
- Issue #108 with both comments: `gh issue view 108 -R k3komatsu/Ez-SDRv4 --comments`.
- Rules: `design/07-radio-model.md` RM-17 (~:190), RM-18, RM-20, RM-25, and the LowerBound table (~:347); `design/09-mock-radio.md` MR-19, MR-21, MR-22, MR-37, the `x3xx` head paragraph (~:211), the profile table (~:72) and the test table rows `mr_21_overrun_shape`, `mr_20a_*`; `design/18-uhd-radio.md` UR-17, UR-18 and the receive-path rules (the cut, the device lead, segments); SC-18 / SC-20a in `design/03-stream-contract.md` (or wherever SC-18 lives).
- Decisions: `plan/phase8/reviews/stage4-owner-decisions.md` (D0, D4, D5 and the owner's replies), `plan/phase8/reviews/stage4-proposal.md` K3/K4 (~:90–125, the restart-after-resume measurement and the proposed RM-17 rewording), `plan/phase8/41-measurement-records.md` §8 (the owner's stability criterion and the "after the consumer reads again" reading), `plan/phase8/reviews/s4a-decisions.md` and `s4a-dispositions.md` (#108's origin, the narrowed differential exemption).
- Bench: `bench/2026-11-16-stage4/README.md` (S4: 1–100 MS/s 20/20 per rate, restart 69–78 ms (#2) and 69–87 ms (#1) after resume; 200 MS/s blocked by the capture sink's cleanup past RS-8a's 5 s; stall-length sweep not run) and its `results/`; `plan/phase8/bench.md` ~:1499–1510 (the S4 procedure); `bench/2026-11-18-mt3/README.md` (receive start lead 10 ms at 100 MS/s).
- KNOWN_LIMITATIONS.md: the `overflow_restart_gap_ns` row (~:229), KL-7, KL-8, KL-15.
- Code: `crates/ezsdr-radio-uhd/src/provider/rx.rs` (the receive call's timeout ~:113/:232/:454, `rx_recv` ~:246, the cut ~:518–527, overflow vs `expected` ~:479–481, the stop-plus-device-lead drop ~:292/:543, the pending overrun dropped with a trimmed packet ~:318/:595/:618–628, the new stream's empty pending list ~:77), `provider/control.rs` (how a stop/start reaches uhd-rx), `crates/ezsdr-mock-radio/src/lib.rs` (the loss window ~:580, ~:735), `crates/ezsdr-radio/src/x3xx*` (timing values), the fake (`crates/ezsdr-radio-uhd/tests/fake.rs` ~:159, :505–517) and the differential `rm_26_the_timeline_against_uhd_rx` with its #108 exemption.

### What is decided — do not reopen

1. **Value** (owner D5, 2026-11-13; reply "140msでよいです"): `overflow_restart_gap_ns` = 140 ms for both Providers' `x3xx` profile (max 139.1 ms under H4, rounded up). Profile values are envelope bounds for the worst supported host state; time-type values come from H4, not a load simulation. Today 50 ms and the row `unmeasured`.
2. **Meaning** (same decision): RM-17 is re-worded so the loss ends `gap` after the consumer reads again (resumes), not `gap` after `f`. The owner was told this matters only for MR-19 (a link `Full`, i.e. the consumer stalled); MR-21 (an injected fault) changes only by the value. Check that claim against the code and say if it is wrong.
3. **Receive start lead stays 10 ms** (owner 2026-11-17); #108 is fixed in 4b, not by raising the lead.
4. The design step comes after the S4 sweep (done 2026-11-16 at 1–100 MS/s). Whether 200 MS/s and stall-length dependence must be measured before the value is final, or in the one bench after 4b (50 §2 lane 1 step 5), is for you to recommend with AGENTS §9 "Hardware tests and bench values" in mind (a value applied beyond its measured conditions needs a sweep plan).

### Questions the draft must answer

A. **RM-17's new wording** for both Providers: what "the consumer reads again" is on the USRP (uhd-rx's receive call returning? the next accepted publish?), in MockRadio's MR-19 (first accepted publish after `Full`), and for MR-21 (no stall: is resume = `f`?). The flags and `lost` of the restart block, the event's timing, and RM-17's report aggregation under the new wording. Is the new start instant observable by a user and by the differential test?

B. **#108**: a receive stop/start (or a cut) booked while uhd-rx is blocked in `rx_recv` during an overflow's gap starts late on the USRP while MockRadio starts on time; the late segment's first block is flagged `GAP_BEFORE | SEQ_DISCONTINUITY` (not `RESTARTED`), and the `RX_OVERFLOW` report is lost altogether. At least two alternatives, each with why it is kept or rejected. Candidates: (i) uhd-rx waits in short slices (or a timeout derived from the next booked cut) so a cut is applied on time — what UHD allows during an overflow restart, VERIFIED from UHD 4.10 source if you can (`/root/tmp/uhd-4.10.0.0` in the docker image `ezsdr-v4-dev:uhd4.10`, or the evidence already in the repo); (ii) a Vocabulary rule both Providers obey: a receive start whose origin falls inside an overflow's gap starts at the gap's end (or is reported as a late start with `RESTARTED`), so Mock reproduces what the device does (§3: Mock enforces the hardware envelope); (iii) refuse such a booking at admission — check against EA-16 and KL-7's workaround; (iv) others you find. Whatever is chosen: the overflow report must not be lost, and the restart block's flags must say what happened (RM-17/RM-18/SC-18).

C. Which tests, `design/` texts, KL entries and Python docs the choice makes false (the #108 exemption in the differential must go; `vh_8_*`, `mr_19_*`, `mr_21_*`, `rm_17_*`, fake values, Python tests that sleep across a gap). Which hardware tests compute expectations from RM-17 (AGENTS §9: each needs a FakeDevice rehearsal).

D. **"queue の深さ 7／8"** is listed under 4b in handoff §2 and plan 50 §2, but `radio.timing.command_queue_depth` left the envelope in note 48 S2b-i (design/07 Changes 2026-11-06). Confirm with one search whether anything about the timed command queue depth is still open (plan/phase8/00-overview.md:22 says Mock 16 vs device 7, "may be loose"); if it is obsolete, say so in one line with the search.

### What the draft must contain (§9 step 0)

1. The user-facing contract: what a user sees after an overflow (events, flags, `lost`, Manifest rows, when a restart they book after an overflow really starts), what they can rely on, and what the Python client shows.
2. For B (and A if there is a real choice), at least two alternatives with reasons.
3. The principles that decide cases.
4. A check against existing promises (EA-*, RM-*, MR-*, UR-*, SC-*, RS-*, KL-*): every text and test made false.
5. Hardware/UHD evidence, VERIFIED or INFERRED, and what is not measured (200 MS/s, stall length, two channels).
6. What the choice makes impossible later.
7. The questions the owner must decide, one at a time, each with a recommendation — only what a user can see (AGENTS §9 "What the owner decides"); decide internal shape yourself and say so.
8. The risky step's operation sequences for the brief (AGENTS §9 step 1): short sequences with expected results, and the hardware behaviour each relies on (VERIFIED/INFERRED).

Keep it proportionate (a research project: lean rules, no frameworks). Mark every claim about code with path:line.

## brief-plan

## Stage 4b — test plan run (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (from main ce631495). This run writes the **test plan only** (AGENTS.md §9 step 2): no code, no edits in the tree, no cargo. You do **not** commit, stash, push or switch branches. Never touch other worktrees (`/home/komatsu/local_work/Ez-SDRv4-n105` is another lane). **Risky step**: it changes a deadline (uhd-rx's receive wait), a lifecycle (overrun → stop → start, a second stop), and the boundary between the FakeDevice/rig and the hardware they stand for. Sol reviews the plan next; later runs implement it in sub-steps.

### What is decided (all binding)
Read all of these in `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/`: `decisions.md` (owner Q1–Q4 and M1, and the orchestrator's dispositions of the Fable review — where they differ from the draft, they win), `design-draft.md` (the design, Opus, revised once), `fable-review.md` (the adversarial review: M2 and the minors m0–m9 and §4's omissions were adopted). Issue #108 with comments (`gh issue view 108 -R k3komatsu/Ez-SDRv4 --comments`).

In short:
1. **uhd-rx's receive wait is bounded** with no cut pending: `clamp(quiet_span, 1 ms, 100 ms)` (draft §3 (i)); the VH-8 rig's `set_now` advances the FakeDevice's host clock with device time, and a rig self-check pins it (draft S4c). The #108 exemption in `rm_26_the_timeline_against_uhd_rx` is deleted.
2. **A second `rx_stop`** when a stream ends with an overrun reported since its last block (Fable M2: ordering from UHD's SYNC post, not timing), with a FakeDevice/rig case of a restart START that survives one stop and is cleared by the second (S13).
3. **RM-17 reworded** (draft §2, with Fable m8's two clauses): the loss belongs to the segment running at `f` and resumes at the latest `overflow_restart_gap_ns` after `r`, the instant its reader takes samples again; a cut ends the segment; the next segment starts at its own origin.
4. **The tail event** (Q2): an overrun/sequence loss whose segment a stop or the Run's end cuts before it resumes produces one `RX_OVERFLOW` at the segment's end, `lost` = cut − first sample not delivered (not the reports' sum, m8), cause from the shared helper; emitted where each Provider now drops its pending reports (uhd-rx `finish_locked`, Mock `rx_settle`). m0: a Stop's cut followed by an abort within `span` keeps the tail for the Stop's cut on both Providers. m2: only *reported* losses make a tail; a late Overflow arriving after the stop must not land on the next segment (S16). m3: one time form and Manifest clocks on both Providers for a tail on a segment that published nothing (S14) — propose it. m4: emission before the clock ends; the order against `DEVICE_LOST`; with a capture open, the artifact marks equal on both Providers. m6: say whether a pending `Alignment` at the end is covered.
5. **The Mock's abort floor** inside a gap is the first sample not delivered (`k_f`), as uhd-rx (draft §2, S6).
6. **MR-19 (b)** (Q3): a refused block's loss is an ordinary RM-17 report — one event per lost stretch, with the first accepted block or at the end; RM-17's back-pressure exception and MR-37's direct emit go; resumption `r = a`; SC-20a gets m7's clause.
7. **140 ms** on both Providers, provisional (Q4): every test expectation computes from `ezsdr_radio::x3xx::timing()` (no `140_000`/`140 000 000` literals; old `50`-ms gap literals replaced the same way); the FakeDevice's two overflow paths take the profile's gap (scripted at `f`, behind at the call that found it behind); S4's bound constant reads the profile; the overflow measurement group emits the `LowerBound` value row (draft §8).
8. **M1** (owner): the promise is "on time, except a restart booked while the Module is still reading the samples buffered before the overflow, which is a reported missed start"; bench S9 split per trial by trace into S9a (booked after the overflow result: must be on time) and S9b (before it: a reported missed start is expected); say which cell S10 rehearses and whether the FakeDevice gains a backlog drain to reach S9b (recommend; default: S9b hardware-only, stated).
9. **Texts**: draft §5 plus Fable §4's five (MR-27 row vs the tail's `lost`, SC-20a, RM-11 if m3 needs it, UR-18 with the tail and the second stop, `plan/phase8/bench.md` S4/S9); KNOWN_LIMITATIONS KL-7 (the M1 line, not "a host stall" only), KL-8, KL-15 (measured conditions incl. block length, m1; provisional; H4 5 trials), the `overflow_restart_gap_ns` row; handoff and plan 50 are the orchestrator's.

### Suggested staging (you may refine; each sub-step leaves the tree green)
4b-1: the wait bound, the rig host clock + self-check, the exemption deleted, UR-17/UR-25, KL-7. 4b-2: RM-17 and the tail, the second stop, MR-19 (b), the Mock abort floor, 140 ms everywhere, the fake's gap, every value test, SC-18/SC-20a/UR-18/UR-33/MR-*, KL-8/KL-15. 4b-3: the measurement value row, S4's bound, the #108 hardware test (S9a/S9b) and its rehearsal (S10), bench.md.

### The plan to write
`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan.md` (the only file you write). For each sequence: operations, expected result (both Providers where both apply), the test (name, file, rig: unit / FakeDevice manual clock / VH-8 parity / layer-3 differential / hardware + rehearsal), what it asserts and how it observes it, the wrong behaviour that fails it, and its mutation row (Rust token anchors, smallest unique text; `plan/maintenance/tools/mutations.json` and `plan/phase*/tools/mutations.json` conventions; name the killing test). Cover at least the draft's S1–S12 (as amended), Fable's S13–S16, S9a/S9b, the S2/S5 capture-open variant (m4), S11 with a pending fault report and with an abort (m7), S8 with a sequence and an overrun before one cut, and the second-overrun-in-a-gap case (m8). Then:
- **Tests and rows the change makes false**: draft §5 plus Fable §4's eight sites; for each: rewrite, retarget, retire or keep, with the reason. Grep every symbol you plan to remove (the excuse helpers and tests in `differential.rs`) in the mutation lists.
- **The audit table** (AGENTS §9 step 1): every test whose expectation comes from RM-17, the gap value, the receive wait or the overflow restart — hardware tests included with their FakeDevice rehearsals — test, expectation, still right / fixed / cannot tell.
- **Texts**: each `design/` and KNOWN_LIMITATIONS text of item 9 with the sentence that changes (substance only).
- Check every refusal or new failure mode you add against EA-16 (`design/16-easy-api.md`); there should be none.
Read: AGENTS.md §6, §7, §9; `design/07-radio-model.md` RM-11/16/17/18/25/35; `design/09-mock-radio.md` MR-19/21/22/27/37 and the x3xx head paragraph; `design/18-uhd-radio.md` UR-12/17/18/25/29/33; `design/02-stream-contract.md` SC-18/SC-20a; `crates/ezsdr-radio-uhd/src/provider/{rx,control,core}.rs`, `provider/rx/differential.rs`, `src/device.rs`, `tests/fake.rs`, `tests/common/{measurement,s4bench}.rs`; `crates/ezsdr-mock-radio/src/lib.rs` and its tests; `crates/ezsdr-acceptance/tests/{parity,v58}.rs`; `crates/ezsdr-radio/src/{x3xx,timeline,lib}.rs`.

### Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never `mutate.py` locally, never a mutant in the tree. (This run writes no rows into the lists, only into the plan.)
- Files under `~/.cache/ezsdr-fixes/` (except the plan file), `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only.
- No tests that judge by wall-clock time (use the manual clock / device time). No mechanisms the decisions do not name.
- Do not run cargo in this run (reading only). If a claim needs a run to settle, mark it INFERRED and say which test in the plan settles it.

### Report (short)
The plan's path, the count of sequences and rows (new, retired, retargeted, re-anchored), the staging, open questions each with a recommendation, anything INFERRED.

## brief-4b1

## Brief — stage 4b, sub-step 4b-1 (the receive wait bound, the rig's host clock, the #108 exemption deleted)

**This step continues a trial of a new way of working (owner 2026-11-18), now on a hard, risky step.** You, a Claude Sonnet subagent, own the sub-step end to end and are responsible for its quality: you turn the plan into precise tasks, Go DeepSeek writes the code, you review DeepSeek's work and send it back until it is right, and only then you report to the orchestrator. You do not hand-write production code or tests yourself (a one-line touch-up after review is fine; say so). Your review replaces the self-review; a fresh Opus sub-step review follows your report and Sol reviews the whole of 4b at its end, so do not stop at "looks fine" — the trial is judged by what those reviews still find. This sub-step touches a receive thread's deadline and the boundary between the FakeDevice/rig and the hardware: the area where DeepSeek most often misreads things. Read its diff with that in mind.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (base 272bc6ff). Work only there. Never touch `/home/komatsu/local_work/Ez-SDRv4-n102` or other worktrees. No commit, stash, push or branch switch (you and DeepSeek): the orchestrator commits each sub-step. Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement: exactly the plan's 4b-1
The reviewed test plan is binding: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-1 row, §3's "4b-1" sequences, §4's rows that belong to 4b-1 (new, retired, re-anchored; §4.3's uniqueness list), §5 and §6 for the tests and texts 4b-1 makes false, §7 for UR-17's wait wording, UR-25's sentence and KL-7 (the owner's M1 line). Decisions: `decisions.md` in the same folder (binding where it differs from the plan or the design). Design context: `design-draft.md`, `fable-review.md`. Do nothing that belongs to 4b-2a and later (the value stays 50 ms in 4b-1; no tail, no second stop, no MR-19 change).

In short (the plan rules): `recv_timeout` with no cut pending is `clamp(quiet_span, 1 ms, 100 ms)`; the cut-pending wait keeps its bound; the VH-8 rig's `set_now` advances the FakeDevice's host clock by the device-time step, pinned by S4c (the `Instant` bracket, no wall-clock bound); the rig's `GAP` is read from `ezsdr_radio::x3xx::timing()`; the #108 exemption in `rm_26_the_timeline_against_uhd_rx` and its three tests are deleted, with the twelve S4A rows the plan names; S2/S3 on `LockSpy` with one-shot hooks (`crates/ezsdr-radio-uhd/src/test_support.rs`), S2a's 5 ms a literal; S12's explicit ratio fixture; S20's coverage assertion.

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b1-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b1-<n>.log 2>&1 < /dev/null & echo $!`. Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-1 sequences, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature build/clippy if the plan's files need it (`python3 plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`).
- `rm_26_the_timeline_against_uhd_rx` with all 1 000 seeds clean and no exemption; S4c.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 272bc6ff` (about 5 minutes).
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b1.log`. Every new row `killed`.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b1-* files, rows-4b1.log and report-4b1.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b1.md`; your final message is its summary)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed result;
- the plan's 4b-1 sequences → test names, any you could not implement as planned and why;
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each of your review rounds (findings, what DeepSeek did with each, any touch-up you made), the number of send-backs, the total time;
- deviations, and anything INFERRED.

### From the plan's last re-check (Opus, 2026-11-18; apply where they fall in 4b-1)
- LockSpy one-shot hooks: take each hook out of its mutex before running it — an `rx_stop` hook that calls `device_lost` re-enters LockSpy through `mark_lost` and would deadlock if the hook's mutex were still held (`test_support.rs:341-455`, `core.rs:1067-1088`).
- S4c: the bracket (`Δ ≤ after − before ≤ Δ + r`) is what tests the seam; a "recorded step equals Δ" assertion on a direct call is a tautology — keep it only as a secondary check.
- (For later sub-steps, not 4b-1: N108-48 split or the reverse-order killer named; S16h/N108-76 a defensive pin with a comment; S9 pure cases without rows.)

## brief-4b2a

## Brief — stage 4b, sub-step 4b-2a (the value: 140 ms everywhere, computed from the profile; plus 4b-1's review minors)

**The trial continues (owner 2026-11-18).** You, a Claude Sonnet subagent, own the sub-step end to end and are responsible for its quality: you turn the plan into precise tasks, Go DeepSeek writes the code, you review DeepSeek's work and send it back until it is right, and only then you report. You do not hand-write production code or tests (a one-line touch-up after review is fine; say so). A fresh Opus sub-step review follows your report and Sol reviews the whole of 4b at its end. 4b-1 went through this way with 0 send-backs; its Opus review still found six minors (below), mostly tests that pass without exercising the shape their name claims — read DeepSeek's tests for that.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (stage base **a4ad013a**; 4b-1 is the local commit 4b579533 on top). Work only there. Never touch `/home/komatsu/local_work/Ez-SDRv4-n103` or other worktrees. No commit, stash, push or branch switch (you and DeepSeek). Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement
1. **The plan's 4b-2a**, exactly: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-2a row, §3's 4b-2a sequences (S23, S24, S1 and whatever else §3 places in 4b-2a), §4's rows for 4b-2a (new, retired, retargeted, re-anchored; §4.3's uniqueness list), §5 and §6 (the audit table: every value test and horizon it marks for 4b-2a), §7 for the texts 4b-2a owns (SC-18 value part, spec 07 §2/§6 rows, spec 18 UR-33 rows, KL-8, KL-15 value part, the `overflow_restart_gap_ns` row). `overflow_restart_gap_ns` becomes 140 ms (owner Q4, provisional) on both Providers; **every** expectation computes from `ezsdr_radio::x3xx::timing()` — no `140_000`/`140_000_000` literals and no remaining 50 ms gap literals (the plan's single pin `s4a_x3xx_values_are_the_measured_ones` is the one place the number is written). No behaviour change besides the value: no tail, no second stop, no MR-19 change, no RM-17 rewording (4b-2b and later).
2. **4b-1's review minors** (dispositions in `decisions.md`, section "4b-1 sub-step review"; the review is `review-4b1.md`): m1 UR-17 and the `rx.rs` doc-comment wording; m2 a row on the rig's host-clock seam (`differential.rs`, `ticks as u64 * 5` → `* 10`) killed by `rm_26_the_rig_s_host_clock_moves_with_its_device_time`; m3 S20 counts only a cut followed by another segment; m4 S2b (`tests/fake.rs`) books its Stop inside the overflow's gap at the profile's value and asserts the overflow; m5 the Mock test asserts the overflow fired and the cut is inside the gap; m6 the spec 09 §6 row's cell count; notes: KL-7 "until past", the always-false `timeout < 1 ms` check in `rx.rs` removed.

Decisions: `decisions.md` (binding where it differs from the plan or the design). Design context: `design-draft.md`, `fable-review.md`. Watch the horizons: a test whose horizon is too short for 1 ms + 140 ms + its blocks goes green without seeing the restart — the plan says each such test asserts the restarted block exists before its fields.

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2a-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2a-<n>.log 2>&1 < /dev/null & echo $!`. Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-2a sequences and the audit table, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature clippy (`python3 plan/maintenance/tools/remote/rdev.py -- cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`) and the hardware test build (`... rdev.py -- cargo test -p ezsdr-radio-uhd --features uhd --test hardware --no-run`) — hardware tests compute from the profile too. The Python suites if any Python test or fixture names the gap (search `python/`).
- `rm_26_the_timeline_against_uhd_rx` with all 1 000 seeds clean at 140 ms (the plan's horizon formula); S4c.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a4ad013a`.
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b2a.log`. Every new row `killed`. (A full-list run of 4b-1 may still hold the sim lock when you start; simq waits for it.)

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b2a-* files, rows-4b2a.log and report-4b2a.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b2a.md`; your final message is its summary; if the write is refused, put the whole report in your final message)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed result;
- the plan's 4b-2a sequences and the audit table's 4b-2a sites → test names and what changed in each; any you could not implement as planned and why; each minor m1–m6 and note → what was done;
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each of your review rounds (findings, what DeepSeek did with each, any touch-up), the number of send-backs, the total time;
- deviations, and anything INFERRED.

## brief-4b2b

## Brief — stage 4b, sub-step 4b-2b (RM-17 re-worded, the tail `RX_OVERFLOW` on both Providers, the Mock's abort floor)

**The trial continues (owner 2026-11-18), on 4b's heaviest sub-step.** You, a Claude Sonnet subagent, own the sub-step end to end: you turn the plan into precise tasks, Go DeepSeek writes the code (you may write the production code's key parts into a task — say so in the report), you review each run adversarially (below) and send it back until it is right, and only then you report. A fresh Opus sub-step review follows; Sol reviews the whole of 4b at its end. So far the later reviews found no wrong production logic from this way of working, but they did find **tests that do not reach the shape their name claims** and **orderings in a test's harness that turn a kill into a harness failure**; Sol's #103 code review also found a refusal-order change in production code written into a task. This sub-step adds an event on two Providers, at a lifecycle boundary (a stop, an abort, the Run's end, a device loss), with ordering against the block stream and the capture's marks: read every test for what it really reaches.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (stage base **a4ad013a**; 4b-1 = 4b579533 and 4b-2a = f6f706b9 are local commits on top). Work only there. Never touch `/home/komatsu/local_work/Ez-SDRv4-n103` or other worktrees. No commit, stash, push or branch switch (you and DeepSeek). Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement
1. **The plan's 4b-2b**, exactly: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-2b row; §3's 4b-2b sequences (S2c, S5, S5b, S6, S6m, S7, S8, S14, S15 (P5), S16 (P6), P10, and whatever else §3 places in 4b-2b); §4's rows for 4b-2b (new, retired — **F32 and D38 retired here** —, retargeted — `vh_8_a_report_without_a_following_block_emits_no_overflow` —, re-anchored; §4.3's uniqueness list); §5, §6 (audit table) and §7 for the texts 4b-2b owns (RM-17 re-worded with Fable m8's two clauses, UR-18/UR-19, the RM-17-dependent rows). The binding decisions are in `decisions.md` (they win over the plan and the design where they differ): read all of it, in particular Q2 (the tail), M2 and the Fable dispositions, F3/P6's accepted residual, and the 2026-11-18 owner line for KL-8 below. Design context: `design-draft.md`, `fable-review.md`. Nothing that belongs to 4b-2c and later: no second `rx_stop` (4b-2c), no MR-19 (b) change (4b-2d), no value row or bench tool (4b-3).
2. **Items the orchestrator placed in 4b-2b** (`decisions.md`):
   - The owner's KL-8 line (2026-11-18, "足してください"): a segment cut by a stop (or the Run's end) after an overflow and before it resumes gets its tail `RX_OVERFLOW` at the first lost sample, the capture's exclusive end, so that capture carries no overflow mark (RS-30); **UR-18 says the same in one sentence.**
   - F3 (P6): the early gate stays; its residual (a stale overflow of A arriving at or after B's origin can be charged to B; hardware arrival INFERRED) is stated in UR-18 and KL-8; tests for the boundary, the failed-stop (`Uncertain`) path and the three gated metadata classes.
   - Re-check minors: row N108-48 split, or its reverse-order killer named; S16h / N108-76 kept as a defensive pin with a one-line comment saying so.
   - Deferred from 4b-1's review: KL-7's "unreported" wording (the tail now reports it).
   - 4b-2a's review (`review-4b2a.md`; dispositions in `decisions.md`, section "4b-2a sub-step review"): m1 the rig's per-hit "fired on this segment" flag (and n2's comment wording); m2 the UHD source citation for the 0.05 s restart timer (or INFERRED) and KL-8's "from the overflow result after the drain"; m3 KL-8's and KL-15's evidence citations; m4 the two Vision lines (§23 and part 03) with an index revision-table row; n5 N108-12's `old2`.
   - The Changes-table rows and `*Test:*` lines of specs 07 and 02 if 4b-2b is the last sub-step to touch them (spec 09 and 18 rows wait for 4b-2d / 4b-3).

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2b-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2b-<n>.log 2>&1 < /dev/null & echo $!` — `$!` is setsid's PID, which exits at once; take the run's real PID afterwards in a separate command with `pgrep -n -f 'deepseek-v4.1-flash#max'` and check its log path in `/proc/<PID>/cmdline` or its start time (another lane may run DeepSeek too). Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-2b sequences, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.


### Your review of each DeepSeek run must be adversarial, not only a match against the plan (owner, 2026-11-18)
After the plan check, for **every test the run added or changed**:
1. Write down one smallest wrong change (to production code or to the fixture) that this test's assertions should catch, and check — by reading the old code, a scratch-copy run, or the test's mutation row — that the test really fails on it, and fails on the assertion its name claims (not on a timeout, a harness wait, or an earlier unrelated assertion).
2. Check that the test actually reaches the state or interleaving its name says (the overflow fired, the cut is inside the gap, the lock is held at that point, the end was requested before the dispatch): name the assertion that proves it. If none does, it is a finding.
3. For a test that uses threads, hooks or gates: for each thread, the worst place it can be descheduled (between a log line and the call it announces, between a marker and the payload, after a gate opens) and what the test then does; a test whose verdict changes with that schedule is a finding.
Also apply 1–2 to production code you wrote into a DeepSeek task yourself. Record these per test in your report (one line each).

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature clippy and the hardware test build through `rdev.py` (as in 4b-2a); the Python suites if a Python test or fixture sees the new event (search `python/` for `RX_OVERFLOW`/`overflow`).
- `rm_26_the_timeline_against_uhd_rx` and `rm_26_the_timeline_against_mockradio` with all 1 000 seeds clean; the parity tests (`crates/ezsdr-acceptance/tests/parity.rs`) for the tail on both Providers.
- Tests whose verdict depends on threads or the manual clock's interleaving: 20 runs each on the final tree.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a4ad013a`.
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b2b.log`. Every new row `killed`; retired rows deleted by id and counted.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b2b-* files, rows-4b2b.log and report-4b2b.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b2b.md`; your final message is its summary; if the write is refused, put the whole report in your final message)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed results;
- the plan's 4b-2b sequences and audit sites → test names; any you could not implement as planned and why; each item of "Items placed in 4b-2b" → what was done;
- the adversarial review, one line per test (the smallest wrong change, does it fail on the named assertion, the assertion that proves the shape is reached, the worst descheduling for thread tests);
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each review round (findings, what DeepSeek did, touch-ups), send-backs, total time, and which production code you wrote into tasks;
- deviations, oracle or rig changes you decided (each with its reason — the orchestrator adjudicates them), and anything INFERRED.

## brief-4b2c

## Brief — stage 4b, sub-step 4b-2c (the second `rx_stop`; and 4b-2b's review items)

**The trial continues (owner 2026-11-18).** You, a Claude Sonnet subagent, own the sub-step end to end: you turn the plan into precise tasks, Go DeepSeek writes the code (you may write the production code's key parts into a task — say so in the report), you review each run adversarially (below) and send it back until it is right, and only then you report. A fresh Opus sub-step review follows; Sol reviews the whole of 4b at its end. This sub-step changes the boundary between the FakeDevice and UHD (a restart START that survives one stop and is cleared by the second): the fake's new knob must model UHD's SYNC post ordering, not invent a timing — check every claim about UHD against the source in the image `ezsdr-v4-dev:uhd4.10` (`/root/tmp/uhd-4.10.0.0`; `docker run --rm ezsdr-v4-dev:uhd4.10 cat <path>`) and mark INFERRED what you cannot.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (stage base **a4ad013a**; 4b-1 4b579533, 4b-2a f6f706b9, 4b-2b 0fd5c539 are local commits on top). Work only there. Never touch `/home/komatsu/local_work/Ez-SDRv4-n107` or other worktrees. No commit, stash, push or branch switch (you and DeepSeek). Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement
1. **The plan's 4b-2c**, exactly: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-2c row (the second stop: S13, S18, with the FakeDevice's overtaking knob, the rig's survivor model and its `compare` change; UR-17/UR-18 text), §3's 4b-2c sequences, §4's rows for 4b-2c, §5/§6/§7 where they fall in 4b-2c. Binding: `decisions.md` (Fable M2 — ordering from UHD's SYNC post, not timing; the re-check's N1 on S13c's knob; everything after it). Context: `design-draft.md`, `fable-review.md`. Nothing that belongs to 4b-2d (MR-19 (b)) or 4b-3 (value row, bench tools).
2. **4b-2b's review** (`review-4b2b.md`; dispositions in `decisions.md`, section "4b-2b sub-step review"): m1 one helper for the end cut that the `applied` rx_stop row, the clock and the tail all use, and UR-17's "the end takes that sample" fixed; m2 the wall-clock upper bound in `uhd.rs` (~:627); m3 the rig's P5 clause reached by a seed and asserted, or deleted with its plumbing; m4 KL-8's residual and UR-18's DEVICE_LOST-order sentence; m5 RM-17's "the first sample not delivered"; n1 one owner for `successor` (say which); n3 a helper for the quiet-end sequence in the `rx.rs` tests; n9 P6's second residual (a late overrun after a single stop) stated in UR-18/KL-8.
3. The Changes-table rows and `*Test:*` lines of specs 02 and 07 if 4b-2c is the last sub-step to touch them (09 waits for 4b-2d, 18 for 4b-3).

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2c-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2c-<n>.log 2>&1 < /dev/null & echo $!` — `$!` is setsid's PID, which exits at once; take the run's real PID afterwards in a separate command with `pgrep -n -f 'deepseek-v4.1-flash#max'` and check its log path in `/proc/<PID>/cmdline` or its start time (another lane may run DeepSeek too). Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-2c sequences, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.


### Your review of each DeepSeek run must be adversarial, not only a match against the plan (owner, 2026-11-18)
After the plan check, for **every test the run added or changed**:
1. Write down one smallest wrong change (to production code or to the fixture) that this test's assertions should catch, and check — by reading the old code, a scratch-copy run, or the test's mutation row — that the test really fails on it, and fails on the assertion its name claims (not on a timeout, a harness wait, or an earlier unrelated assertion).
2. Check that the test actually reaches the state or interleaving its name says (the overflow fired, the cut is inside the gap, the lock is held at that point, the end was requested before the dispatch): name the assertion that proves it. If none does, it is a finding.
3. For a test that uses threads, hooks or gates: for each thread, the worst place it can be descheduled (between a log line and the call it announces, between a marker and the payload, after a gate opens) and what the test then does; a test whose verdict changes with that schedule is a finding.
Also apply 1–2 to production code you wrote into a DeepSeek task yourself. Record these per test in your report (one line each).

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature clippy and the hardware test build through `rdev.py` (as in 4b-2a); the Python suites if a Python test or fixture sees the new event (search `python/` for `RX_OVERFLOW`/`overflow`).
- `rm_26_the_timeline_against_uhd_rx` and `rm_26_the_timeline_against_mockradio` with all 1 000 seeds clean, with the rig's survivor model (S13c: the knob fires only for a stop issued while no sample at or after kg was returned; the oracle permits, not requires, a second stop); S20's `cut_in_gap` coverage still ≥ 3 (raise the seeds' reach rather than lower it); the parity tests.
- Tests whose verdict depends on threads or the manual clock's interleaving: 20 runs each on the final tree.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a4ad013a --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt` (append any rename of yours to that file, `old -> new`, one per line).
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b2c.log`. Every new row `killed`; retired rows deleted by id and counted.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b2c-* files, rows-4b2c.log and report-4b2c.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b2c.md`; your final message is its summary; if the write is refused, put the whole report in your final message)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed results;
- the plan's 4b-2c sequences and audit sites → test names; any you could not implement as planned and why; each item of "Items placed in 4b-2b" → what was done;
- the adversarial review, one line per test (the smallest wrong change, does it fail on the named assertion, the assertion that proves the shape is reached, the worst descheduling for thread tests);
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each review round (findings, what DeepSeek did, touch-ups), send-backs, total time, and which production code you wrote into tasks;
- deviations, oracle or rig changes you decided (each with its reason — the orchestrator adjudicates them), and anything INFERRED.

## brief-4b2d

## Brief — stage 4b, sub-step 4b-2d (MR-19 (b): a refused block's loss is an ordinary RM-17 report; and the reviews' carried items)

**The trial continues (owner 2026-11-18).** You, a Claude Sonnet subagent, own the sub-step end to end: you turn the plan into precise tasks, Go DeepSeek writes the code (you may write the production code's key parts into a task — say so in the report), you review each run adversarially (below) and send it back until it is right, and only then you report. A fresh Opus sub-step review follows; Sol reviews the whole of 4b at its end. This sub-step is Mock-only (UR-12 refuses `Block` links on the USRP) but it changes a back-pressure path's reporting, which a capture and the Manifest see: read every test for the stretch it really refuses and what it really reports. DeepSeek's scratch builds use `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0` (AGENTS §7), never their own target dirs; logs only in `ds-4b2d-*` files.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (stage base **a4ad013a**; local commits 4b-1 4b579533, 4b-2a f6f706b9, 4b-2b 0fd5c539, 4b-2c d890fef5 + 05647cdc on top). Work only there. Never touch other worktrees. No commit, stash, push or branch switch (you and DeepSeek). Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement
1. **The plan's 4b-2d**, exactly: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-2d row (MR-19 (b): S11, S19; MR-19/MR-21/MR-27/MR-37/SC-20a texts; **rows F45, F46, D10, D34 retired here, with the conversion; F44 re-anchored**), §3's 4b-2d sequences (S11a–S11d, S19), §4's rows (N108-50…53 and the rest the plan names), §5–§7 where they fall in 4b-2d. Binding: `decisions.md` (owner Q3: MR-19's refused-block loss becomes an ordinary RM-17 report, resumption `r = a`; RM-17's back-pressure exception and MR-37's direct emit go; SC-20a gets Fable m7's clause; and every later section). Context: `design-draft.md`, `fable-review.md`. Nothing that belongs to 4b-3 (value row, bench tools, S9/S10, bench.md).
2. **Carried items** (`decisions.md`):
   - 4b-2b review m6, **this exact case as a test**: a Block link refuses a block at `a` and MR-19 emits its event; a fault then fires inside that gap; a Stop cuts the segment → today `rx_tail` emits a second `{overrun, c − a}` at `a`, so [a, c) is counted twice. After MR-19 (b) the stretch is counted once; the test fails on 05647cdc.
   - 4b-2b deviation 7's interim (an abort inside a back-pressure gap ended the Mock's clock at the refused block's first sample after MR-19's event already reported the gap): closed by S11d; say how.
   - MR-19's direct emit bypassed the zero filter (4b-2b deviation 6): goes with the direct emit; confirm nothing else bypasses the filter.
   - 4b-2c's re-check (`recheck-4b2c.md`; dispositions in `decisions.md`, last section): m1′ — UR-18's consequence sentence rewritten in normative form as the re-check proposes (its "UR-18 and KL-8" section), and KL-8's "Also" tail replaced by the re-check's proposed text (provisional: the owner approves it at landing); a unit-level case that a gated `Alignment` and a gated `BadPacket` arm nothing (today only the sequence-error class is pinned), with a row; if small, the rig's `World::restart: Option<i64>` → bool and the restart formula in one helper.
   - The Changes-table rows and `*Test:*` lines of specs 02, 07 and 09 (4b-2d is the last sub-step to touch them; spec 18's wait for 4b-3).

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2d-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b2d-<n>.log 2>&1 < /dev/null & echo $!` — `$!` is setsid's PID, which exits at once; take the run's real PID afterwards in a separate command with `pgrep -n -f 'deepseek-v4.1-flash#max'` and check its log path in `/proc/<PID>/cmdline` or its start time (another lane may run DeepSeek too). Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-2d sequences, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.


### Your review of each DeepSeek run must be adversarial, not only a match against the plan (owner, 2026-11-18)
After the plan check, for **every test the run added or changed**:
1. Write down one smallest wrong change (to production code or to the fixture) that this test's assertions should catch, and check — by reading the old code, a scratch-copy run, or the test's mutation row — that the test really fails on it, and fails on the assertion its name claims (not on a timeout, a harness wait, or an earlier unrelated assertion).
2. Check that the test actually reaches the state or interleaving its name says (the overflow fired, the cut is inside the gap, the lock is held at that point, the end was requested before the dispatch): name the assertion that proves it. If none does, it is a finding.
3. For a test that uses threads, hooks or gates: for each thread, the worst place it can be descheduled (between a log line and the call it announces, between a marker and the payload, after a gate opens) and what the test then does; a test whose verdict changes with that schedule is a finding.
Also apply 1–2 to production code you wrote into a DeepSeek task yourself. Record these per test in your report (one line each).

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature clippy and the hardware test build through `rdev.py` (as in 4b-2a); the Python suites if a Python test or fixture sees the new event (search `python/` for `RX_OVERFLOW`/`overflow`).
- `rm_26_the_timeline_against_uhd_rx` and `rm_26_the_timeline_against_mockradio` with all 1 000 seeds clean, S20's coverage counts still at or above their thresholds; the parity tests.
- Tests whose verdict depends on threads or the manual clock's interleaving: 20 runs each on the final tree.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a4ad013a --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt` (append any rename of yours to that file, `old -> new`, one per line).
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b2d.log`. Every new row `killed`; retired rows deleted by id and counted.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b2d-* files, rows-4b2d.log and report-4b2d.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b2d.md`; your final message is its summary; if the write is refused, put the whole report in your final message)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed results;
- the plan's 4b-2d sequences and audit sites → test names; any you could not implement as planned and why; each item of "Items placed in 4b-2b" → what was done;
- the adversarial review, one line per test (the smallest wrong change, does it fail on the named assertion, the assertion that proves the shape is reached, the worst descheduling for thread tests);
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each review round (findings, what DeepSeek did, touch-ups), send-backs, total time, and which production code you wrote into tasks;
- deviations, oracle or rig changes you decided (each with its reason — the orchestrator adjudicates them), and anything INFERRED.

## brief-4b3

## Brief — stage 4b, sub-step 4b-3 (the bench's tools: the overflow value row, S4's bound and sweep line, S9a/S9b's classifier and rehearsal, bench.md)

**The trial continues (owner 2026-11-18).** You, a Claude Sonnet subagent, own the last sub-step of 4b end to end: you turn the plan into precise tasks, Go DeepSeek writes the code (you may write key parts into a task — say so in the report), you review each run adversarially (below) and send it back until it is right, and only then you report. A fresh Opus sub-step review follows, then Sol 6.2 reviews the whole of 4b. This sub-step builds what the next bench runs on hardware: every `#[ignore]` hardware test whose expectation comes from a rule needs its FakeDevice rehearsal in the default suite that books the same boundary (AGENTS §9 "Hardware tests and bench values"); a value fixed from bench data states the conditions it was measured under. DeepSeek's scratch builds use `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` with `CARGO_INCREMENTAL=0`; logs only in `ds-4b3-*` files.

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b` (stage base **a4ad013a**; local commits 4b-1 … 4b-2d and its fix round on top; `git log --oneline a4ad013a..HEAD`). Work only there. Never touch other worktrees (another lane works in `/home/komatsu/local_work/Ez-SDRv4-nsb9a`: its DeepSeek run may be active — check `/proc/<PID>/cwd` before trusting a PID). No commit, stash, push or branch switch (you and DeepSeek). Read `AGENTS.md` §3, §6, §7, §9 first.

### What to implement
1. **The plan's 4b-3**, exactly: `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/test-plan-v3.md` — §1's 4b-3 row; §3's 4b-3 sequences S21 (the `LowerBound` value row from complete evidence only; one shared completeness predicate), S22 (S4's bound from the profile and the sweep-cell line), S9 (the hardware test split by trace into S9a/S9b; the pure classifier `s9_classify` and verdict `s9_verdict`), S10a (S9's body on the FakeDevice: cell A and the INCONCLUSIVE path), S10b (the Module's response to cell B at unit level); §4's rows for 4b-3 (N108-60, 61, 62, 63, 70, 72, 73, 74, 75 and the rest it names; **P800 retargeted**, P780/P797–P799 following the renamed summary test); §5–§7 where they fall in 4b-3 (`plan/phase8/bench.md` S4/S9, the KL-15 rows' bench plan). Binding: `decisions.md` (all of it; owner Q4 — 140 ms provisional, bench after 4b; M1 — S9a/S9b; Q10–Q13; the re-check minors placed in 4b-3: **S9's pure cases given rows, and incomplete rows as an input to `s9_classify`**).
2. **Spec 18's Changes-table rows and `*Test:*` lines** for the whole of 4b (4b-3 is the last sub-step to touch spec 18): every UR rule 4b changed (UR-17, UR-18, UR-19, UR-25, UR-33 and any other), with the tests that pin them.
3. The 4b-2d re-check's two text minors (`recheck-4b2d.md`; `decisions.md`, last section): RM-17's "begins at `max(b, a + gap)` (MR-19's `k_g`)", and RM-25's clause that a block a Mock `Block` link refused counts as published.
4. Nothing beyond the plan: no new measurement mechanism, no change to the overflow gap's value (it stays 140 ms provisional until the bench).

### Driving DeepSeek
- Write each task to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b3-<n>-task.md` first: files, exact behaviour, tests with names and expected values, rows (Rust token anchors, smallest uniquely matching text, killing test named), exit checks, and what it must not do (mechanisms you did not ask for, files outside the task, `mutate.py` locally, a mutant in the tree, a commit, wall-clock tests). Its shell tool cuts commands at 120 s: tell it to give long commands a timeout of at least 1800 s.
- Launch it detached and note the PID: `cd /home/komatsu/local_work/Ez-SDRv4 && setsid nohup opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --auto "$(cat <task file>)" > /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/ds-4b3-<n>.log 2>&1 < /dev/null & echo $!` — `$!` is setsid's PID, which exits at once; take the run's real PID afterwards in a separate command with `pgrep -n -f 'deepseek-v4.1-flash#max'` and check its log path in `/proc/<PID>/cmdline` or its start time (another lane may run DeepSeek too). Wait with `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done`, repeated until `done`; glance at the log between waits. Do not end your turn while it runs. One run at a time; do not edit the tree while it runs. If the log has not grown for 15 minutes, check `~/.local/share/opencode/log/opencode.log` before judging it hung; kill a hung run by its process group (`kill -- -<PID>`).
- **Run `remote_mutate.py` in the foreground, never detached** (a detached launch from a subagent died silently on 2026-11-18).
- Split only where the work splits naturally (e.g. production + tests, then rows and texts), not by time.
- Review each run yourself: read the whole diff against the plan's 4b-3 sequences, run the exit checks, and check that each new test fails on the old code (in a scratch copy outside the repo under /tmp with fresh timestamps, `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`, `CARGO_INCREMENTAL=0`; or reason from the mutation rows). Send it back (a fresh run whose task names the previous task, the diff and your findings) **at most twice**; if it is still wrong after that, stop and report what is left — do not write it yourself.


### Your review of each DeepSeek run must be adversarial, not only a match against the plan (owner, 2026-11-18)
After the plan check, for **every test the run added or changed**:
1. Write down one smallest wrong change (to production code or to the fixture) that this test's assertions should catch, and check — by reading the old code, a scratch-copy run, or the test's mutation row — that the test really fails on it, and fails on the assertion its name claims (not on a timeout, a harness wait, or an earlier unrelated assertion).
2. Check that the test actually reaches the state or interleaving its name says (the overflow fired, the cut is inside the gap, the lock is held at that point, the end was requested before the dispatch): name the assertion that proves it. If none does, it is a finding.
3. For a test that uses threads, hooks or gates: for each thread, the worst place it can be descheduled (between a log line and the call it announces, between a marker and the payload, after a gate opens) and what the test then does; a test whose verdict changes with that schedule is a finding.
Also apply 1–2 to production code you wrote into a DeepSeek task yourself. Record these per test in your report (one line each).

### Exit checks (in the main tree, foreground or waited for)
- `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; the UHD feature clippy and the hardware test build through `rdev.py` (as in 4b-2a); the Python suites if a Python test or fixture sees the new event (search `python/` for `RX_OVERFLOW`/`overflow`).
- `rm_26_the_timeline_against_uhd_rx` and `rm_26_the_timeline_against_mockradio` with all 1 000 seeds clean, S20's coverage counts still at or above their thresholds; the parity tests.
- Tests whose verdict depends on threads or the manual clock's interleaving: 20 runs each on the final tree.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a4ad013a --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt` (append any rename of yours to that file, `old -> new`, one per line).
- Rows: the new rows and every existing row whose filter selects a test that changed or whose anchor moved, through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>` in the foreground; log to `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/rows-4b3.log`. Every new row `killed`; retired rows deleted by id and counted.

### Rules
Mutation rows only through `remote_mutate.py`; never `mutate.py` locally; never a mutant in the tree. Kill by PID. Temp files on /tmp. No build output in the tree. `~/.cache/ezsdr-fixes/` (except this step's ds-4b3-* files, rows-4b3.log and report-4b3.md), `handoff.md`, `AGENTS.md`, plan notes: read only; `design/` and `KNOWN_LIMITATIONS.md` texts the plan names may be edited. No wall-clock tests. No version bumps or SCHEMA_CHANGELOG entries. `cargo fmt --all` before reporting; after it, `rules.py` must still find every row exactly once.

### Report (`/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/report-4b3.md`; your final message is its summary; if the write is refused, put the whole report in your final message)
- `git diff --stat`; each check's counts; each row's outcome; the 1 000-seed results;
- the plan's 4b-3 sequences and audit sites → test names; any you could not implement as planned and why; each item of "Items placed in 4b-2b" → what was done;
- the adversarial review, one line per test (the smallest wrong change, does it fail on the named assertion, the assertion that proves the shape is reached, the worst descheduling for thread tests);
- **the trial's record**: each DeepSeek run (task in one line, duration, outcome), each review round (findings, what DeepSeek did, touch-ups), send-backs, total time, and which production code you wrote into tasks;
- deviations, oracle or rig changes you decided (each with its reason — the orchestrator adjudicates them), and anything INFERRED.

## brief-4b3-fix1

## Stage 4b, sub-step 4b-3 fix round 1: Sonnet drives Go DeepSeek

You supervise Go DeepSeek fixing the findings adopted from the Opus sub-step review of 4b-3. Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b`, HEAD `1cf0b296` (4b-3, committed locally by the orchestrator; stage base `a4ad013a`). Work only there; never touch `/home/komatsu/local_work/Ez-SDRv4-nsb9a` or other worktrees. Do not commit, stash, push, reset or switch branch.

Read first, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/`: `review-4b3.md` (the review, with path:line), the section "4b-3 sub-step review" at the end of `decisions.md` (binding dispositions), `report-4b3.md` (what 4b-3 did), and `brief-4b3.md` (the sub-step brief: its process sections — driving DeepSeek, the common rules for DeepSeek tasks, adversarial review, exit checks, rules — bind this round too, with the changes below). Also `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

### What to fix
M1, M2, M3, m1–m7 and the notes n6, n8, n10, n11 exactly as `decisions.md` disposes them. The majors change verdicts the next hardware bench reports, so design the M1/M3/m2 rules yourself and write their key production lines into DeepSeek's task; DeepSeek writes the tests and rows. Each major gets a pure classifier/verdict case that fails on the 4b-3 code at the named assertion and a mutation row killed by it. For M3, take UHD's restart delay from the UHD 4.10 source (the review cites it; the image `ezsdr-v4-dev:uhd4.10` has the source) and state the citation in the constant's doc comment. For m1, judge the size first: if adding the block length is more than a small wrapper, stop at qualifying KL-15's clause (the orchestrator shows the owner the wording) and say which you chose.

### Changes to the sub-step brief's process
- Task files `ds-4b3f1-<n>-task.md`, logs `ds-4b3f1-<n>.log`, rows log `rows-4b3f1.log`, in the folder above.
- **Waiting for DeepSeek or a long command: call the Bash tool with its `timeout` parameter set to 600000** and `timeout 590 tail --pid=<PID> -f /dev/null; kill -0 <PID> 2>/dev/null && echo running || echo done` — one wait of up to ~10 minutes per call. `remote_mutate.py` the same way (or `run_in_background` if it may exceed 10 minutes).
- Every DeepSeek task repeats: never apply a mutant or a wrong change in the main tree — wrong-change probes only in a /tmp scratch copy (4b-3's run 4 broke this).
- Rows: the rows whose code or killing test this round changes plus the new ones (not the full lists).
- At most two send-backs in total. No wall-clock verdicts; repeat a test 20 times only if its verdict can depend on timing (S10a: yes).

### Report
Your final message is the report: `git diff --stat` against `1cf0b296`; each finding: what changed, its test and row and outcome; checks with counts (fmt, clippy workspace, the UHD-feature clippy and `hardware` build via `rdev.py`, `cargo test --workspace`, S10a ×20, rules.py `--base a4ad013a --renames …/renames.txt`); the adversarial review one line per test added or changed; m1's choice and KL-15's exact wording if changed; the trial record; deviations and anything INFERRED.

## brief-4b3-fix2

## Stage 4b, sub-step 4b-3 fix round 2: Sonnet drives Go DeepSeek

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b`, HEAD `61f53f27` (4b rebased onto main `6cc8026d`; the stage base for rules.py is now `6cc8026d`). Work only there. Do not commit, stash, push, reset or switch branch.

Read first, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/`: `recheck-4b3.md` (the re-check: new finding 1 major, 2 minor, notes 3–6), the last bullet of `decisions.md` (binding dispositions: R1, R2, note 4, note 5), `report-4b3-fix1.md`, and `brief-4b3-fix1.md` (its process sections bind this round, including the 10-minute Bash waits and "never a mutant in the main tree"; logs `ds-4b3f2-<n>.log`, tasks `ds-4b3f2-<n>-task.md`, rows `rows-4b3f2.log`). Also `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

### What to fix
- R1 (major): design the rule yourself and write its production lines into the task: the sequence-cause FAIL counts only sequence events in the successor's clock domain at its first block (tick 0); a pure case with a sequence-error overflow before the stall, one in the backlog and one on a later successor block (each not FAIL), and one at the successor's first block (FAIL); a row killed by it.
- R2: `S9_RESTART_MARGIN_NS` = 2 ms with its reason in the doc comment; re-derive the window test's expected figures by hand; the boundary rows follow.
- Note 4: an assertion that an S9 trial's initial requests carry no frequency.
- Note 5: if it is small, one fake run of the S4 overflow trial with a 65 536-sample block reaching a complete trial; otherwise say why not.
- rules.py: `--base 6cc8026d --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt`.

### Report
Final message: `git diff --stat` against `61f53f27`; each item with its test, row and outcome; checks with counts (fmt, clippy workspace, `cargo test --workspace`, rules.py); the adversarial review one line per test added or changed; the trial record; deviations and INFERRED.

## brief-4b-solfix

## Stage 4b, fix round after Sol's code review: Sonnet drives Go DeepSeek

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b`, HEAD `74104080` (the whole of 4b on main `6cc8026d`; rules.py `--base 6cc8026d --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt`). Work only there. Do not commit, stash, push, reset or switch branch.

Read first, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/`: `sol-code.md` (Sol 6.2's review of the whole stage: S1–S6 majors, S7–S10 minors, path:line), the section "Sol 6.2 code review of the whole of 4b" at the end of `decisions.md` (binding dispositions), `test-plan-v3.md` (the stage's plan: P1 and the UHD facts it records), the reports `report-4b1.md`, `report-4b2c*.md`, `report-4b3*.md` for the code these findings touch, and `brief-4b3-fix1.md` (its process sections bind this round: 10-minute Bash waits, never a mutant or wrong change in the main tree, tasks `ds-solfix-<n>-task.md`, logs `ds-solfix-<n>.log`, rows `rows-solfix.log`). Also `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

### What to fix
S1–S10 as `decisions.md` disposes them. S1 and S2 are production changes in the UHD Module's receive and stop paths (a deadline and a lifecycle across the FFI): design them yourself, read the UHD 4.10 source in the image `ezsdr-v4-dev:uhd4.10` for S1 (how `recv` applies its timeout across packets; one-packet mode), write their production lines into DeepSeek's task, and give each a deterministic test (manual clock, FakeDevice knobs or hooks; no wall-clock verdict) that fails on the current code at its named assertion, plus a row. If S1's fix changes the VH-8 rig's expectations or the differential test's coverage counts, say how. S3–S6 change bench verdicts: each gets a pure classifier/verdict/extractor case that fails on the current code and a row. S7–S9 as disposed. S10: change KNOWN_LIMITATIONS KL-7, KL-8, KL-15 only as disposed, with the bench evidence cited (`bench/2026-11-18-mt3/README.md`), and report the exact old and new sentences.

Order: S1 and S2 first (production; one DeepSeek run), then S3–S6 (bench tools), then S7–S10. At most two send-backs in total. Repeat a test 20 times only if its verdict can depend on timing.

### Report
Final message: `git diff --stat` against `74104080`; each finding with what changed, its test, its row and the row's outcome; checks with counts (fmt, clippy workspace, UHD-feature clippy and `hardware` build via `rdev.py`, `cargo test --workspace`, the 1 000-seed `rm_26_the_timeline_against_uhd_rx` if S1 touches the receive path, rules.py); KL-7/8/15 old and new sentences; the adversarial review one line per test added or changed; the trial record; deviations and INFERRED.

## brief-4b-solfix2

## Stage 4b, fix round 2 after Sol's code review: Sonnet drives Go DeepSeek

Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/4b`, HEAD `ec404e26`. Work only there. Do not commit, stash, push, reset or switch branch. rules.py `--base 6cc8026d --renames /home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/renames.txt`.

Read first, in `/home/komatsu/.cache/ezsdr-fixes/2026-11-18-4b/`: `recheck-solfix.md` (Opus) and `terra-recheck-solfix.md` (Terra), the last bullet block of `decisions.md` (binding: T1–T4, N1, N2, N8, the duplicated assertion), `report-4b-solfix.md` (your previous round), `sol-code.md` (S3–S5 as Sol stated them), and `brief-4b-solfix.md` (its process sections bind: 10-minute Bash waits, never a mutant or wrong change in the main tree; tasks `ds-solfix2-<n>-task.md`, logs `ds-solfix2-<n>.log`, rows `rows-solfix2.log`). Also `~/.cache/ezsdr-fixes/tools/sonnet-review-adversarial.md`.

### What to fix
- T1: first find out, from the code, whether an S9 trial can observe the successor's first PUBLISHED block (index and flags) without new production surface — e.g. a capture Sink in the trial's profile, a test-only link, a Manifest row. If yes, the cell-A PASS requires that block at index 0 with no GAP_BEFORE/SEQ_DISCONTINUITY, with a test and a row. If no, stop on T1 and report what it would take; do not add production surface.
- T2, T3, T4, N1, N2, N8, the duplicated assertion: as disposed. N2 and N1 touch production (`rx.rs`, `core.rs`): write those lines yourself; each gets a deterministic test failing on the current code at its named assertion and a row.
- At most two send-backs. Repeat timing-dependent tests 20 times.

### Report
Final message: `git diff --stat` against `ec404e26`; each item with what changed, test, row, outcome (T1: the finding about observability, with path:line); checks with counts (fmt, clippy workspace, UHD-feature clippy and `hardware` build via `rdev.py`, `cargo test --workspace`, 1 000-seed `rm_26_the_timeline_against_uhd_rx` if the receive path changed, rules.py); the adversarial review one line per test; the trial record; deviations and INFERRED.
