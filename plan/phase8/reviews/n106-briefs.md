# #106 — briefs (orchestrator → design, review and implementer runs)

## design-brief

# Design brief: #106 — a repeat resumes after every drop, and the UHD Module's per-Run records are bounded

You draft the design (AGENTS.md §9 step 0) for issue #106 in the Ez-SDR v4 repository at `/home/komatsu/local_work/Ez-SDRv4` (branch `main`). READ-ONLY: edit nothing in the repository, run no cargo build or test, no git writes. Write your draft to `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/design-draft.md` (that file only) and finish with a short summary.

Read first: AGENTS.md (§3, §6, §9 step 0), issue #106 (`gh issue view 106 -R k3komatsu/Ez-SDRv4`), `design/18-uhd-radio.md` UR-22 (lines ~255–285: "A repeat resumes", the stall rule at line ~266), UR-23, UR-28, UR-30; `design/07-radio-model.md` (the RM rules about repeats, RM-11, RM-22, RM-37, and its test lines that name the stutter bound); `design/04-run-and-session.md` RS-33 and `crates/ezsdr-kernel/src/event.rs` (the bounded event ring and `EVENTS_DROPPED`: the pattern to follow); `crates/ezsdr-radio-uhd/src/provider/core.rs` around `Records` (lines ~780–805) and `provider/mod.rs` around line 426 (how the records reach the Manifest); `crates/ezsdr-radio-uhd/src/provider/tx.rs` `dropped` (~1122) and the report handling (~1400–1470); `plan/phase8/reviews/mt3-decisions.md` (M4: why b′ was chosen) and `mt3-fable-design.md` (~line 430–480: the (a)/(b) discussion); KNOWN_LIMITATIONS.md KL-5, KL-13, KL-45; `design/16-easy-api.md` EA-16 (what a user is promised about errors); the Manifest schema under `schemas/` for the Provider records. MockRadio (`crates/ezsdr-mock-radio`): does it have a repeat-resume or stutter rule, or unbounded per-Run records, of its own?

## What the owner decided (2026-11-16) — do not reopen

1. **Drop the stutter bound b′.** A host-fed repeat resumes after every underflow and every late report that reaches it, on its lattice at `R` as UR-22 says now; it never ends because it stutters. Each resume stays a recorded gap (event, `timing` row, split burst record). What still ends a repeat (a sequence-loss report, a failed or short send, a failed reopen, a lost device, an `R` or switch boundary that does not fit the clock) is out of scope unless dropping b′ forces a change.
2. **Bound the Module's own per-Run records** the way the Kernel bounds events (RS-33): a Run whose host cannot keep up must not grow memory or the Manifest without bound. Today `bursts`, `device_async`, `rejected`, `switches`, `timing` (and `applied`) are unbounded `Vec<Json>` held until the Run ends; one resume adds a burst record, a timing row and about 10–30 raw async rows (the packets in flight reported late, `TimeError`).
3. A burst that is not a repeat keeps ending at its first underflow (not in scope). v3's untimed continuation (samples played at a time no record says) is rejected: it breaks UR-22's honesty about time.

## Evidence (bench 2026-11-16 and evening; logs in `~/.cache/ezsdr-bench/2026-11-16/` and `2026-11-17/`)

- X300 #1 (OBX) and #2 (UBX), UHD 4.10 in docker, host usrp-lnx03 (i40e, MTU 9000, governor performance, C-states up to C3 at 350 µs). 200 MS/s, 300 s, slot A: bare UHD underflows 19–34 times (1996 or 1 M samples per `send`), v3 (UHD 4.10, 1 M per send) about 49–63 times; the Module's repeat resumed through 13–14 underflows on #1 (5.997e10 of 6.0e10 samples) and stopped after 0.27e9 and 18.8e9 samples on #2 right after two underflows (b′ INFERRED: the Manifests were not kept). 100 MS/s: 2 of 10 60 s trials underflowed on both boards. X300 HG image: stream endpoint ingress buffer 32768 × 64-bit words = 256 KB, about 0.33 ms at 200 MS/s (`x300_HG_rfnoc_image_core.yml`, VERIFIED from the UHD 4.10 tree in `ezsdr-v4-dev:uhd4.10` at `/root/tmp/uhd-4.10.0.0`).

## What the draft must contain (§9 step 0)

1. **The user-facing contract**: what a user sees during and after a Run whose repeat stutters (events, Manifest rows, the burst records, the summary counts), what they can and cannot rely on, and what the Python client shows (`python/ezsdr`).
2. **The bound**: at least two alternatives, each with the reasons it is rejected or kept. Candidates: (i) per-kind first-N rows kept plus an exact count of the rest, as RS-33 does; (ii) first-N plus last-M; (iii) a count-only summary for the repetitive kinds (one aggregated row per gap cause) with raw rows only up to N; (iv) streaming the records to disk during the Run instead of holding them. Say which records are bounded (`device_async`, `timing`, `rejected`, `bursts`, `switches`, `applied`; and the receive side's records) and which must stay complete because another rule reads them (UR-28 correlation, SC-24 burst records, the continuity/record of a stream, tests that read rows). The Kernel's own records (stream records, gaps) too: does a stuttering repeat grow anything there without bound?
3. **The principles** that decide cases (what is never dropped, how a reader learns rows were left out, how counts stay exact).
4. **A check against existing promises**: EA-*, RM-*, UR-*, SC-*, RS-*, KL entries; list every `design/` text and test that the change makes false (including the b′ tests: `mt3d_a_part_that_plays_less_than_the_stall_part_ends_the_repeat`, `mt3d_a_delayed_underflow_that_retires_two_entries_applies_the_stutter_rule`, `mt3d_a_switch_part_keeps_the_stutter_bound_of_the_gap_it_replaced`, and others you find).
5. **Hardware evidence**, VERIFIED or INFERRED, and what is not measured.
6. **What the choice makes impossible later.**
7. **The questions the owner must decide**, one at a time, each with a recommendation: at least the row limit N (and whether it is a profile value or a constant), whether burst records are bounded, and how the Manifest says what was left out. Keep to what a user can see; decide internal shape yourself and say so.

Keep it proportionate (this is a research project: lean rules, no frameworks). Mark every claim about code with path:line.

## fable-brief

# Adversarial design review: #106 (Fable, AGENTS.md §9 step 0)

You review a design adversarially for the Ez-SDR v4 repository at `/home/komatsu/local_work/Ez-SDRv4` (branch `main`). READ-ONLY: edit nothing in the repository, run no build or test, no git writes. Write your review to `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/fable-review.md` (that file only).

Inputs:
- the draft: `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/design-draft.md` (Opus, read against main at 081fedeb);
- the owner's decisions so far: `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/decisions.md` (they supersede the draft where they differ: the env override of N, the `record_bound` section with `rows_per_kind`, the word "stutter" kept out of user-facing texts);
- issue #106 (`gh issue view 106 -R k3komatsu/Ez-SDRv4`), the bench record `bench/2026-11-16-stage4/README.md`, and the code and specs the draft cites (`design/18-uhd-radio.md` UR-22, UR-28, UR-30; `design/02-stream-contract.md` SC-24/25/28; `design/04-run-and-session.md` RS-33..36; `crates/ezsdr-radio-uhd/src/provider/{core,tx,mod,rx,control}.rs`; `crates/ezsdr-radio/src/staging.rs`; `crates/ezsdr-kernel/src/event.rs` and `coordinator/`); the original b′ decision `plan/phase8/reviews/mt3-decisions.md` and `mt3-fable-design.md`.

The owner settled the direction: a repeat resumes after every underflow or late report (b′ removed), and the Module's per-Run records are bounded per kind (first N kept, the rest counted). Do not argue to keep b′ unless you show a concrete harm the owner has not weighed. Your job:
1. Attack the chosen design: a minimal operation sequence (pseudocode is fine) and the observable wrong outcome for each problem you find. Look especially at: what removing b′ and its zero case does to the lifecycle (a late report reaching a part, two channels, a switch across the gap, a Stop or Run end in the gap, the expired-timeline path, the T3 flush, `busy`, the 100/s rate-bound argument); whether "count then keep per kind" can lose or double-count anything (locks, the clone at `write_sections`, rows rewritten in place, a kind derived from a field that changes); whether the "intermediate part" predicate can drop a record a rule or reader needs (SC-24/SC-28, UR-28's `target` join, the Python client, tests); whether any growth remains unbounded besides the Kernel's events (receive side, control side, `unacked`, chains, anything the draft missed); the env override (where it is read, when it is refused, how it reaches the Manifest).
2. Propose better alternatives where you see them, with the reason they are better.
3. Check the draft's lists (design texts made false, tests, mutation rows, obsolete code) for omissions.
Every claim with path:line. Mark what you could not verify as INFERRED. Severity per finding: major (a wrong outcome a user or a rule would see), minor, or note.

Finish with a summary of at most 20 lines: the majors (one line each with the sequence), the alternatives you recommend, and the omissions.

## brief-plan

# #106 — test plan run (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n106` (from main 5b8059c7). You edit nothing in this run except the plan file below; you do **not** commit, stash, push or switch branches. **Risky step** (it changes a lifecycle — when a repeat ends — and the Manifest's Provider records). This run writes the **test plan only** (AGENTS.md §9 step 2): no code. Sol reviews the plan next; a later run implements it.

## What is decided (all binding)
Read all of: `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/decisions.md` (the owner's decisions and the orchestrator's; where they differ from the draft, they win), `design-draft.md` (the design, Opus) and `fable-review.md` (the adversarial review whose majors were decided and whose minors were adopted) in the same folder; issue #106 (`gh issue view 106 -R k3komatsu/Ez-SDRv4`).

In short:
1. **b′ goes, its zero case included**: a repeat resumes after every underflow or late report that reaches it, on its lattice at R (UR-22). What still ends a repeat is unchanged (sequence loss, also while paused; a failed or short send; a failed reopen; a lost device; an R or switch boundary that does not fit the clock; Stop; the Run's end). A burst that is not a repeat still ends at its first underflow. Code made obsolete: draft §4.5 (and `remember_burst`/`remember_part` merge, Fable §4).
2. **Promise and counters** (F-M1): every resume is counted exactly; a drop the device reports is told by an event; `resumes_at` once per resume. New fixed keys in `ezsdr.radio.uhd.stats`: `tx_resumes`, `tx_gap_samples` (Σ planned R − H, written where the `tx_resume` row is). No new event for the expired-check resume.
3. **Bounded records** (Q2, F-M2, minors m1–m5): `async`, `timing`, `rejected` keep the first N rows **per kind** and count the rest; kinds: `async` by `code` (a flush's `BurstAck` its own key; a `BurstAck` that acknowledges a **booked** burst's end is never bounded, one of a resume's close or unmatched is), `timing` by `what`, `rejected` by `call`, else `kind` plus the rule id at the start of `reason` ("UR-22", …), else the identifying field (draft §2.4); at most 64 kinds per section, the rest under `other`. Exempt by name (never bounded; a new kind is bounded unless exempted on purpose): `tx_stop_applied`, `rx_segment`, `first_rx_block`, `rx_stop_untimed`, `rx_stop_late`. `bursts`: only intermediate parts (a part's record that a later resume closed) are bounded, kind `resumed_part`; booked bursts' records and every repeat's closing record are kept. `applied` and `switches` are complete. Count first, then keep, under the same `rec` lock; one push-only bounded type that derefs to `[Json]` so every writer goes through it (m5). Per Run, not per stream (m6).
4. **`ezsdr.radio.uhd.record_bound`** (Q4), always written, under the same `rec` guard as the other sections' clones: `{ "rows_per_kind": N, "omitted": { "<section>": { "<kind>": count } } }`, `omitted` `{}` when nothing was left out.
5. **N** (F-M3): binding selector key `record_rows_per_kind`, default 1000, `Int` in `1..=100000`, read in `UhdRadio::from_binding` beside `block_len` (`provider/mod.rs` ~:219-245), refused through the same UR-5 selector path before any device open; tests that need a small N use a builder (like `with_measurement_timing`), never `std::env::set_var`. No environment variable.
6. **Texts**: UR-22, UR-28, UR-30 (and its `*Test:*` line), SC-25, SC-28 (one clause each), RM-37's test list, the design/18 test table rows that name removed tests, a Changes row in each changed spec; KNOWN_LIMITATIONS KL-13 (no b′; "keeps dropping and resuming until it is stopped" — never the word "stutter" in user-facing text; m7's sentence: a repeat whose every part the device drops late transmits nothing and keeps resuming; m8's sentence), KL-39's note, KL-5 (line 48), KL-45 (one line), and one new entry (the record bound, the selector key, the `TimeError` figure of m3, per Run, and the Kernel's event list that still grows — Q1); `plan/phase8/bench.md` C7's "cap に達しない". The orchestrator adds the freeze-review item to handoff (not you). Check every refusal you add against EA-16 (`design/16-easy-api.md`) and say how it surfaces (`EA-7: …` through the catalogue, as UR-5's).

Out of scope: the Kernel's event list (Q1), the receive side's resume rules, MockRadio (it has no resume rule and no growing rows: draft §2.6 — confirm with the grep the draft names), anything of #102–#105.

## The plan to write
`/home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/test-plan.md` (the only file you write). For each sequence: operations, expected result, the test (name, file, rig), what it asserts and how it observes it, the wrong behaviour that fails it, and its mutation row (Rust token anchors, smallest unique text; `plan/maintenance/tools/mutations.json` conventions). At least:
- **Lifecycle** (FakeDevice, manual clock): an underflow on a part before it played the old stall length resumes again; a late report reaching a part (nothing played) resumes again; the delayed underflow that retires two entries (both resumes kept; the event as UR-28 says); two channels late in either order; a switch part's former P − 1 case; a stale report of a stopped repeat; each ending that stays (sequence loss in the gap, failed send, failed reopen, lost device, R past the clock) still ends — reuse the existing tests where they pin it and say which; **every part dropped late** (m7: the fake's `SlowSend` or a manual-clock equivalent; the existing `ur_28_the_device_reports_a_late_burst` (`fake.rs:2576`) pins the zero case and becomes wall-clock dependent — rewrite it on the manual clock): the repeat keeps resuming, each cycle one `TIME_ERROR { late_at_device }`, successive resume points at least m + 32 samples apart; **many resumes past N** with a small N: the Run goes on, `tx_resumes` equals kept `tx_resume` rows plus omitted, `tx_gap_samples` the sum of the planned spans.
- **The bound** (pure, at the type's level): 1 001 rows of one kind → 1 000 kept, omitted 1; two kinds independent; a one-off after a flood kept; an exempt kind never bounded; the 65th kind goes to `other`; count before keep (kept + omitted = pushed); the key functions for `async` (booked-burst `BurstAck` vs resume close vs flush vs unmatched), `timing`, `rejected` (kind + rule id; a reason with device text does not make a new kind).
- **Wiring** (FakeDevice, small N): a clean Run with more than N booked bursts keeps every booked `BurstAck` and every `tx_stop_applied`; a long resume sequence keeps the booked record and the closing record and counts `resumed_part`; `record_bound` is written on a clean Run with `omitted: {}` and on a bounded one with the exact counts; written under the same guard as the clones (a detached thread still writing, UR-16: say how the test observes it, or why it cannot).
- **Selector** (`from_binding`): default 1000; 1 and 100000 accepted; 0, 100001, a Float, a String refused with the UR-5 text, before any device call (assert no open happened); the server path surfaces it as `EA-7: …` (one server or catalogue test).
- **Manifest and schema**: `ur_30_the_sections_are_written` gains `record_bound`; the stats keys; whether `schemas/manifest.v1.json` constrains Provider sections (if it does, the regeneration and `schema_freeze`).
- **Tests and rows the change makes false**: draft §4.3 and §4.4 plus Fable §4's omissions (MT3D-81 retire; MT3D-15, -38, -55 retarget; `hw_h5_held_last_sample`'s row counts, `hardware.rs:2090-2121`); for each: rewrite, retarget, retire, or keep, with the reason. Grep every symbol you plan to remove in the mutation lists.
- **Readers that count rows** (draft §4.3 table, Fable §4): the audit table AGENTS §9 step 1 asks for (test, expectation, still right / fixed / cannot tell), including the hardware tests and their FakeDevice rehearsals.
- **Texts**: each `design/` and KNOWN_LIMITATIONS text of item 6 with the sentence that changes (substance; the wording is the implementing diff's).
Read: AGENTS.md §6, §7, §9; design/18-uhd-radio.md UR-5, UR-22, UR-23, UR-28, UR-30; design/02-stream-contract.md SC-24/25/28; `provider/{mod,core,tx,rx,control}.rs`; `crates/ezsdr-radio/src/staging.rs` (`Paused`); `src/provider/tx/tests/mt3d.rs`; `tests/fake.rs`.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. (This run writes no rows into the lists, only into the plan.)
- Files under `~/.cache/ezsdr-fixes/` (except the plan file above), `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only.
- Do not add tests that judge by wall-clock time. Do not plan mechanisms the decisions do not name.
- No build output inside the tree; test temp files on /tmp. Do not run cargo in this run (reading only).

## Report (short)
The plan's path, the count of sequences and rows (new, retired, retargeted, re-anchored), open questions each with a recommendation, anything INFERRED.

## sol-plan-brief

You review a TEST PLAN before any code is written (AGENTS.md §9 step 3) for the Ez-SDR v4 repository at the current directory (branch work/n106, a git checkout). READ-ONLY: edit nothing, write no files, no git writes, never run cargo or any build. Use grep/rg, sed, cat, git.

The step (issue #106, risky: a lifecycle and the Manifest's Provider records): (1) the UHD Module's host-fed repeat no longer ends when a resumed part underflows or is dropped late again (the stutter bound b′ and its zero case are removed); it resumes after every drop on its lattice (design/18-uhd-radio.md UR-22). (2) The Module's per-Run records `async`, `timing`, `rejected` keep the first N rows per kind and count the rest, `bursts` bounds only intermediate parts, `applied` and `switches` stay complete, Action-only kinds are exempt, a booked burst's BurstAck is never bounded; a new section `ezsdr.radio.uhd.record_bound` says what was left out; N is a binding selector key `record_rows_per_kind`; two new stats counters `tx_resumes`, `tx_gap_samples`.

FIRST, before reading the plan: from the step's purpose, the code (`crates/ezsdr-radio-uhd/src/provider/{tx,core,mod,rx,control}.rs`, `crates/ezsdr-radio/src/staging.rs`, `crates/ezsdr-server/src/catalogue.rs`, `crates/ezsdr-kernel/src/event.rs`) and the specs (design/18 UR-5, UR-22, UR-28, UR-30; design/02 SC-24, SC-25, SC-28), write your own list of failure scenarios: operation sequences where an implementation of this step could go wrong, each with the wrong observable outcome. Only THEN read the decisions and the plan:
- decisions: /home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/decisions.md (binding; the design is design-draft.md and the adversarial review fable-review.md in the same folder)
- the plan under review: /home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/test-plan.md
Compare: which of your scenarios the plan misses; for each planned test, whether it observes what it claims and whether its mutation row would be killed by it (and not by luck); tests that would judge by wall-clock time; rows that are equivalent mutants; retired or retargeted rows whose behaviour is still reachable; the audit of readers that count rows; texts the change makes false that the plan does not list.

Severity per finding: major (a wrong behaviour the plan would let through, or a test that cannot fail), minor, note. A major states a minimal operation sequence and the wrong outcome. Every claim with path:line. Finish with a numbered list of findings and a one-line verdict.

## brief-impl

# #106 — implementation runs (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

You are the implementer (Claude Code Sonnet subagent), the same role as in `brief-plan.md` in this folder (read it: tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/n106`, decisions, scope, rules). You edit the tree; you do **not** commit, stash, push or switch branches (the orchestrator commits each sub-step after you exit).

The step is implemented in **two sub-steps**, each its own run, each reviewed before the next:
- **Sub-step A** (this run unless the message says B): the lifecycle and the counters — test-plan §1 (the parts of the production shape A needs), §2 (L), §3 (E), §4 (C), the rows of §9 that A's code touches (retire, retarget, re-anchor, new rows of L/C), §10.1, and the texts of §11 for b′ (UR-22, UR-28's stutter sentence, RM-37's list, design/18 test-table rows and :571, KL-13, KL-39's note, KL-5 line 48, bench.md C7). Not the bound.
- **Sub-step B**: the bound, the keys, the wiring, the selector and the Manifest — §1's `Bounded`/`Records`, §5 (B), §6 (K), §7 (W), §8 (S, M), the rest of §9, §10.2, the rest of §11 (UR-30, SC-25, SC-28, UR-28's "every report in `async`", KL-45, the new KL entry, the test-helper guard of OQ-6 and the bench profiles' `record_rows_per_kind`).

## First (sub-step A only): amend the plan
Copy `test-plan.md` → `test-plan-v2.md` in this folder, one "v2 changes" line at the top, and apply Sol's plan review (`sol-plan.md`, findings at its end; all adopted) and the orchestrator's decisions on the open questions (`decisions.md`, last entries):
1. **Major 1:** a booked switch joined onto a resumed part's open device burst makes that correlation acknowledge a booked burst: its `BurstAck` is never bounded (for each channel). Add the saturated-budget test for: exhaust the budget → resume into part P → book switch S → S joins P's device burst → close → S's ACK kept. The exemption must look at the correlation's targets (any booked target → booked), not only how it was opened. (Sub-step B implements it; plan it now.)
2. **Major 2:** extend W4's saturated sequence (N = 1, `resumed_part` exhausted) to the two other final-record writers — an abandon (sequence loss or send failure, `tx.rs` ~:1177) and a failed reopen (~:973) — each keeps its closing record; one mutation row per writer.
3. **Major 3:** N106-27's killer is W5 (eight finite bookings, N = 3), not W4a.
4. **Minors 4–8:** fix L2 (advance to the new part's send window), C3 (liveness before Stop), L10 (recover at the next planned point; `clock.instant(...)` for ticks); MT3D-55 is retargeted to `crates/ezsdr-radio/tests/staging.rs` ~:3179-3190, never retired for a survivor; B6 asserts conservation per effective bucket (an independent first-64 mapping) and globally; keep the throughput readers (`measurement.rs` ~:3218, `multichannel.rs` ~:874) audited or give them the same completeness check, and add a synthetic omitted-Manifest test of the guard and its opt-out; T5 uses the accepted wording (every resume counted; a device-reported drop told by an event), and §11 lists `design/18-uhd-radio.md:571`.

## Exit checks (each sub-step)
- `cargo fmt --all`.
- Build and tests on sim03, not on this host (the bench host is shared with the owner's other work): `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- cargo clippy --workspace --all-targets -- -D warnings` and `… -- cargo test --workspace` (foreground; give the command a long timeout). `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 5b8059c7` locally (about 5 minutes; foreground).
- New tests whose verdict can depend on timing (threads, the owner loop): 20 serial runs through rdev.
- Mutation: the sub-step's new, retargeted and re-anchored rows through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all killed, none `"kill": "bound"`; retired rows removed from the lists. Save the log as `~/.cache/ezsdr-fixes/2026-11-17-n106/rows-A.log` (or `rows-B.log`).
- The audit table (sub-step B: §10.2; sub-step A: the tests of §10.1) in the report.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. A row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json` and `plan/phase7/tools/mutations.json`; say so.
- Files under `~/.cache/ezsdr-fixes/` (except `test-plan-v2.md` and `rows-*.log` in this folder), `handoff.md`, `AGENTS.md` and plan notes other than `plan/phase8/bench.md` are the orchestrator's: read only.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the decisions do not name. The word "stutter" stays out of user-facing texts (KNOWN_LIMITATIONS, docstrings the Python client shows); code identifiers that already use it may stay or be renamed.
- No build output inside the tree; test temp files on /tmp.
- Wait in the foreground for every long command. Kill processes by PID, never by a name pattern. Do not delete a test of a reachable sequence without a replacement; grep mutation rows for any symbol you rename or remove. After `cargo fmt`, `rules.py` must still find every row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`, details in `~/.cache/ezsdr-fixes/tools/deepseek-offload.txt`) is cheap for precisely described repetitive edits (golden `stats` lines in e2b.rs/e3.rs, say); read its diff and run the tests; foreground, one at a time, do not touch the tree while it runs; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, rows' outcomes, the audit table, deviations from the plan and why, what you offloaded, anything INFERRED.

## sol-code-brief

You review the CODE of one implementation step (AGENTS.md §9 step 7) in the Ez-SDR v4 repository at the current directory (branch work/n106). READ-ONLY: edit nothing, write no files, no git writes, never run cargo or any build. Use git, grep/rg, sed, cat. The step is the diff `git diff 5b8059c7..HEAD` (two sub-step commits 50e60fee and 4e76bd55, plus a handoff line b8d5cf20 you may ignore).

The step (issue #106; `gh issue view 106 -R k3komatsu/Ez-SDRv4`): (1) the UHD Module's host-fed repeat no longer ends when a resumed part underflows or is dropped late again (the stutter bound b′ and its zero case removed); it resumes after every drop (design/18-uhd-radio.md UR-22, UR-28); new stats counters `tx_resumes`, `tx_gap_samples`. (2) The Module's per-Run records are bounded per kind (`crates/ezsdr-radio-uhd/src/provider/core/records.rs`): `async` by code (a flush BurstAck apart; a BurstAck that matches a pending booked burst never bounded), `timing` by `what` with Action-only kinds exempt, `rejected` by call / kind+rule id / identifying field, at most 64 kinds per section, `bursts` bounds only a repeat's intermediate parts; `applied`, `switches` complete; a new always-written section `ezsdr.radio.uhd.record_bound`; the binding selector key `record_rows_per_kind` (default 1000, 1..=100000) parsed before the device opens. Decisions (binding, owner and orchestrator): /home/komatsu/.cache/ezsdr-fixes/2026-11-17-n106/decisions.md. The test plan was reviewed by you earlier (sol-plan.md); the final plan is test-plan-v2.md in the same folder. The full mutation lists passed (2996/2997, the known survivor phase7/G09).

Review independently: correctness of the lifecycle without b′ (two channels, expired check, switch across the gap, Stop or loss in the gap, the T3 flush and reopen, busy and the rate of resumes), the bound (count before keep, every writer through the bounded type, keys, the booked-ACK exemption for every way a correlation can contain a booked target, the intermediate-part predicate at every record writer, the one-borrow `record_bound`), the selector (parse, range, refusal path before the open on the server and in `from_binding`), the texts (design/18 UR-5/22/28/30, design/02 SC-25/28, design/16 EA-7, KNOWN_LIMITATIONS KL-5/13/39/45/46) against the code, scenarios the test plan missed, tests that cannot fail, and maintainability (duplicated decisions, state in two places, leftovers of b′, rows anchored on scaffolding). Severity: major (a minimal operation sequence and the wrong observable outcome), minor, note. Every claim with path:line. Finish with a numbered findings list and a one-line verdict.
