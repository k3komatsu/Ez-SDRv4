# Stage 4a — briefs (orchestrator → implementer and review runs)

## brief-plan

# Stage 4a — test plan run (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

You are the implementer (Claude Code Sonnet subagent). Main tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/s4a` (from main 8d59be00). You write only the plan file below in this run; you do **not** edit the tree, commit, stash, push or switch branches. **Risky step** (it changes deadlines and refusal values that MockRadio and the UHD Module both read, a refusal at `arm`, and the boundary between the fakes and the hardware). This run writes the **test plan only** (AGENTS.md §9 step 2): no code. Sol reviews the plan next.

## What is decided (all binding)
Read all of these; they win over anything older:
- `plan/phase8/reviews/stage4-proposal.md` (the per-key rows K1–K14, §2's policies, §4, §6's outline) and `plan/phase8/reviews/stage4-owner-decisions.md` (D0–D13, owner 2026-11-13).
- `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/decisions.md` (owner 2026-11-17: 1-a, 1-a(b), 1-b, 1-c).
- The 2026-11-16 bench: `bench/2026-11-16-stage4/README.md`, `results/decisions.tsv`, and `plan/phase8/bench-results.md` "2026-11-16".

The step has three parts (plan each; the implementation will run them as sub-steps in this order):
- **A — the profile values (K1, K4, K6, K7 and the `unmeasured` rows D6–D12).** `min_timed_command_lead_ns` = 5 ms for both Providers (Mock 2 → 5 ms; `ProviderTerms` loses the lead; the UHD 2 + 3 ms split stays internal, pinned by a test) (D3); `rx_start_lead_ns` = 10 ms (D4, 1-b); `rx_bytes_per_s` = `tx_bytes_per_s` = 8e8 B/s per 10 GbE link (D1 — confirmed 10/10 at 200 MS/s receive on both boards 2026-11-16 — and D2; sustained transmit underflow is KL-5, not a refusal); the rows of D6–D12 stay as they are and are marked `unmeasured` with their reasons. Each value's conditions (boards, rates, trials) go where the decision is recorded (AGENTS §9 "Hardware tests and bench values"). XG two links (D1's note) and K3 (`overflow_restart_gap_ns`, RM-17: stage 4b, its own design step) are **out of scope**; so is placing measurement records (D13: a bench run after this step).
- **B — the receive start head (1-a, 1-a(b)).** One shared function in `ezsdr_radio` (next to `x3xx::decimation` / `clip_decimation`; `rx_fill_samples(decim)` in `~/.cache/ezsdr-fixes/2026-11-13-rxzeros/options.md` §3 D shows the shape) computes n_hb = min(v2(D), 3) and returns the head **2 / 13 / 18 / 24** samples (n_hb 0 / 1 / 2 / 3). MockRadio (`publish_rx`, signal paths — channel mode, the segment head) and FakeDevice (`sample_at`'s loopback branch, the stream origin) both call it, at **every** receive start (fresh open, restart, retune, after a rate change), and deliver that many exact-zero samples; the samples after it are on time (not shifted). Not in the Manifest. The options note (§1, §5) names the Mock and Fake sites and the specs: MR-13 untouched, MR-35 and UR-33's Fake clause gain the rule, KNOWN_LIMITATIONS gets the table and its conditions (2026-11-16, both boards, 20 rate cells, gain 0/+10/+20 dB, the 90 % threshold) and that the exact-zero count on hardware falls with the received level.
- **C — #75 (3) on (1-c).** `OPEN_FLUSH_CONFIRMED` (`crates/ezsdr-radio-uhd/src/provider/core.rs:846`) becomes the behaviour (remove the switch rather than flip it, if that keeps the code smaller — say which), with its tests (`provider/core/t3.rs` `ur_75_*`), the texts (UR-22, UR-13 if the refusal text appears there, KL-12 rewritten for the new refusal at `arm`: three unconfirmed attempts refuse Connect), and the refusal checked against EA-16 (how it surfaces to a Python user). Record: `plan/phase8/reviews/n75-*`, `~/.cache/ezsdr-fixes/2026-11-13-n75/options.md` §3.7 and the decisions at its end.

## The plan to write
`/home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/test-plan.md` (the only file you write). For each sequence: operations, expected result, the test (name, file, rig), what it asserts and how it observes it, the wrong behaviour that fails it, and its mutation row (Rust token anchors, smallest unique text; `plan/maintenance/tools/mutations.json` conventions). At least:
- **A:** the value pins (proposal K1, K4, K6, K7), the 2 + 3 ms identity test, a differential test that Mock and UHD Descriptions agree on every comparable key except the repeat pair (proposal §6.3, the `d3_…` test extended), the admission boundary at the new byte rates (one channel 200 MS/s admitted on one 10 GbE link, 200 + 100 refused) on both Providers, the Python tests that read the receive start lead (proposal §6.3, `sleep(0.062)` and the like), the two acceptance fixture tests rebuilt on another key; KL-4/5/7/14/15/16 texts and design 07/09/18 tables.
- **B:** the pure function at every class boundary and the odd factors (D = 1, 2, 3, 4, 6, 8, 12, 24, 25, 248, 250, 255); Mock and Fake at each start kind (fresh open, restart, retune, after a rate change) deliver exactly the head of zeros and then the on-time signal (the first non-zero sample is the one the rate's clock puts there, not shifted); a start with no signal path (MR-13's cases) is untouched; both call the one function (a mutation of the table is killed through each caller).
- **C:** three unconfirmed open flushes refuse `arm`/Connect with the refusal text, one confirmed attempt passes, a recovered attempt leaves no rejection (F2), Stop or loss during an attempt (as the recovery path's tests), the server/Python surface of the refusal (one test).
- **Tests and hardware tests the change makes false:** every test whose expectation comes from a changed value or rule, including the hardware tests and their FakeDevice rehearsals (`hardware.rs` readers of the profile — proposal §6.3 names `:713`, `:2464`, `:2653`, `:2708`, the n47/n47s5 groups — and every receive test that reads the first samples: the rxzeros, PN, restart-fill and lead probes, the Python ones): an **audit table** (test, expectation, still right / fixed / cannot tell), AGENTS §9 step 1.
- **Mutation rows** to retire, retarget, re-anchor or add (grep every symbol you plan to remove, e.g. `OPEN_FLUSH_CONFIRMED`, the Mock's 2 ms lead).
- **Texts:** each design/ and KNOWN_LIMITATIONS sentence that changes (substance; the wording is the implementing diff's), with no history in a rule (AGENTS §2 DA-1: rationale goes in the Changes rows).
Read: AGENTS.md §2, §3, §6, §7, §9; design/07 (RM-14, RM-15, RM-20, RM-32), design/09 (MR-3, MR-13, MR-35), design/18 (UR-9, UR-13, UR-22, UR-33); `crates/ezsdr-radio/src/x3xx.rs`; `crates/ezsdr-mock-radio/src/{lib,profile}.rs`; `crates/ezsdr-radio-uhd/src/{profile.rs,device.rs,provider/core.rs,provider/core/t3.rs}`.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. (This run writes no rows into the lists, only into the plan.)
- Files under `~/.cache/ezsdr-fixes/` (except the plan file above), `handoff.md`, `AGENTS.md` and plan notes are the orchestrator's: read only.
- Do not add tests that judge by wall-clock time. Do not plan mechanisms the decisions do not name.
- No build output inside the tree; test temp files on /tmp. Do not run cargo in this run (reading only).

## Report (short)
The plan's path, the count of sequences and rows (new, retired, retargeted, re-anchored), open questions each with a recommendation, anything INFERRED.

## brief-impl

# Stage 4a — implementation runs (orchestrator → Claude Code Sonnet 5.5 subagent, effort high)

You are the implementer (Claude Code Sonnet subagent), the same role as in `brief-plan.md` in this folder (read it: tree `/home/komatsu/local_work/Ez-SDRv4`, branch `work/s4a`, decisions, scope, rules). You edit the tree; you do **not** commit, stash, push or switch branches (the orchestrator commits each sub-step after you exit). The decisions file `decisions.md` in this folder is binding and now has the owner's Q1 (option B) and the orchestrator's answers to the plan's other questions.

Four sub-steps, each its own run, each reviewed before the next: **A** (profile values), **B** (receive start head), **C** (open flush, #75 (3)), **D** (the probe fix and the receive-only start-lead bench test). The run's message names the sub-step.

## First (sub-step A only): amend the plan
Copy `test-plan.md` → `test-plan-v2.md`, one "v2 changes" line at the top, and apply the decisions and Sol's plan review (`sol-plan.md`, findings at its end; all adopted):
1. **Major 1 / Q1 (owner): option B throughout.** Both Providers admit with the 5 ms declared lead and both move a late SendAsap send to receipt + the 2 ms device lead; the Mock no longer uses the declared lead as its device lead; one shared VH-8 oracle; RM-14 amended to name the two quantities. Tests: admission just below and at 5 ms; placement at receipt + 2 ms on both; the existing join and untimed-placement expectations kept (`crates/ezsdr-mock-radio/src/lib.rs` ~:1630-1657).
2. **Major 2 / Q2:** a head after an overflow restart (both the injected overflow and the Mock's back-pressure path, `lib.rs` ~:580-593, ~:720-755), with the SampleClock origin and the gap accounting unchanged; the texts and KL entry say so (INFERRED on hardware).
3. **Major 3:** FakeDevice delivers the head at every receive start, whichever source supplies the later samples (the loopback or the ramp fallback, `crates/ezsdr-radio-uhd/src/device.rs` ~:1275-1294): add receive-only and mismatched rate/frequency cases.
4. **Minors 4–8:** a time-varying fixture for the on-time check (compare several post-head samples against their own ticks); a head split across reads (e.g. 7 + 7 + 11 for 24) and a Stop/start with an unread old tail (the old tail unchanged, the successor gets one whole head, `device.rs` ~:473-494, ~:1687-1700); drop or fix S4A-12 (asymmetric Description), S4A-35 (no behavioural kill: check by review), S4A-64 (pin an independent budget or drop), fix the ledger's names (A3's killer file, C's row numbers); migrated flush tests hold the manual clock while the floor and the reports are set (`provider/core/t3.rs` ~:39-45); the FakeDevice rehearsal of the rxzeros verdict books an over-bound case through the same verdict, or the audit says only the pure decision is boundary-tested.

## Exit checks (each sub-step)
- `cargo fmt --all`.
- Build and tests on sim03, not on this host (the bench host is shared): `python3 plan/maintenance/tools/remote/rdev.py --host sim03 -- cargo clippy --workspace --all-targets -- -D warnings` and `… -- cargo test --workspace` after your last edit (foreground, long timeout). The Python suites when a sub-step changes what they read (A: the receive start lead): run them as `gate.py --only py314,py39` would (read `plan/maintenance/tools/gate.py`), or through rdev. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base 8d59be00` (with `--renames <file>` if you rename or retire a test that a spec cites; keep the file at `/home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/renames.txt`).
- New tests whose verdict can depend on timing: 20 serial runs through rdev.
- Mutation: the sub-step's new, retargeted and re-anchored rows through `REMOTE_SSH="ssh -o BatchMode=yes" ~/.cache/ezsdr-fixes/tools/simq python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:7,sim03:7 <ids>`: all killed, none `"kill": "bound"`; retired rows removed. Log: `~/.cache/ezsdr-fixes/2026-11-17-stage4a/rows-<A|B|C|D>.log`.
- The audit table for the sub-step's tests (AGENTS §9 step 1) in the report.

## Rules (every brief)
- Mutation rows run only through `remote_mutate.py`; never run `mutate.py` locally, never apply a mutant to the tree. A row that must change with your code may be retargeted in `plan/maintenance/tools/**/mutations*.json` and `plan/phase7/tools/mutations.json`; say so.
- Files under `~/.cache/ezsdr-fixes/` (except `test-plan-v2.md`, `renames.txt` and `rows-*.log` in this folder), `handoff.md`, `AGENTS.md` and plan notes other than `plan/phase8/bench.md` are the orchestrator's: read only. KNOWN_LIMITATIONS.md and `design/` are yours for this step's texts.
- Do not add tests that judge by wall-clock time. Do not add mechanisms the decisions do not name. No history in a rule (AGENTS §2 DA-1: reasons go in the Changes rows).
- No build output inside the tree; test temp files on /tmp. New files mode 644.
- Wait in the foreground for every long command. Kill processes by PID, never by a name pattern. Do not delete a test of a reachable sequence without a replacement; grep mutation rows for any symbol you rename or remove. After `cargo fmt`, `rules.py` must still find every row exactly once.

Offloading to DeepSeek: Go DeepSeek (`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash#max --auto "<task>"`, `~/.cache/ezsdr-fixes/tools/deepseek-offload.txt`) for precisely described repetitive edits; read its diff and run the tests; foreground, one at a time, do not touch the tree while it runs; list what you offloaded.

## Report (short)
`git diff --stat`, each check's counts, rows' outcomes, the audit table, deviations from the plan and why, what you offloaded, anything INFERRED.

## sol-plan-brief

You review a TEST PLAN before any code is written (AGENTS.md §9 step 3) for the Ez-SDR v4 repository at the current directory (branch work/s4a). READ-ONLY: edit nothing, write no files, no git writes, never run cargo or any build. Use grep/rg, sed, cat, git.

The step (Phase 8 stage 4a, risky): A — profile values from the bench: `min_timed_command_lead_ns` 5 ms for MockRadio and the UHD Module (both admit at 5 ms; both move a late SendAsap send to receipt + the 2 ms device lead; RM-14 names the two quantities), `rx_start_lead_ns` 10 ms, `rx_bytes_per_s` = `tx_bytes_per_s` = 8e8 B/s per 10 GbE link, the other keys `unmeasured`; B — a receive start head: one function in `ezsdr_radio` returns 2/13/18/24 samples by DDC half-band class, and MockRadio and FakeDevice deliver that many zeros at every receive start, the signal after it on time; C — the open-time flush at `arm` retries up to 3 attempts and refuses Connect when none is confirmed (#75 (3)), the single-attempt path removed; D — the bench probe's row limit fix (`tests/common/probe_device.rs`, `measurement.rs` `restart_trial`) and a receive-only start-lead bench test.

FIRST, before reading the plan: from the step's purpose, the code (`crates/ezsdr-radio/src/x3xx.rs`, `crates/ezsdr-mock-radio/src/{lib,profile}.rs`, `crates/ezsdr-radio-uhd/src/{profile.rs,device.rs,provider/core.rs,provider/core/t3.rs}`, `crates/ezsdr-radio-uhd/tests/common/{probe_device.rs,measurement.rs}`) and the specs (design/07 RM-14, RM-15, RM-20, RM-32; design/09 MR-3, MR-13, MR-35; design/18 UR-9, UR-13, UR-22, UR-33), write your own list of failure scenarios: operation sequences where an implementation could go wrong, each with the wrong observable outcome. Only THEN read the decisions and the plan:
- decisions: /home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/decisions.md, plan/phase8/reviews/stage4-owner-decisions.md, plan/phase8/reviews/stage4-proposal.md
- the plan under review: /home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/test-plan.md
Compare: which of your scenarios the plan misses; whether each test observes what it claims and whether its mutation row would be killed by it; wall-clock-dependent tests; equivalent mutants; retired or retargeted rows whose behaviour is still reachable; the audit of tests whose expectations come from a changed value (hardware tests and their FakeDevice rehearsals included); texts made false and not listed.

Severity: major (a minimal operation sequence and the wrong outcome), minor, note. Every claim with path:line. Finish with a numbered list of findings and a one-line verdict.

## sol-code-brief

You review the CODE of one implementation step (AGENTS.md §9 step 7) in the Ez-SDR v4 repository at the current directory (branch work/s4a). READ-ONLY: edit nothing, write no files, no git writes, never run cargo or any build. Use git, grep/rg, sed, cat. The step is the diff `git diff 8d59be00..HEAD` (sub-step commits: A 60dce09f, B e9d2aa96 + fix f9fd145c, C 1cfa23d2 + fix 11b3ed6e, D a62962d1 + fix ce05a195).

The step (stage 4a of Phase 8; decisions binding: /home/komatsu/.cache/ezsdr-fixes/2026-11-17-stage4a/decisions.md; you reviewed the test plan earlier: sol-plan.md; final plan test-plan-v2.md with deviations §12–§14, same folder):
- A — profile values from the 2026-11-16 bench (bench/2026-11-16-stage4/): min_timed_command_lead_ns 5 ms for both Providers (MockRadio 2 → 5), and both place a late SendAsap send at receipt + the 2 ms device lead (option B; RM-14 amended); rx_start_lead_ns 10 ms; rx_bytes_per_s = tx_bytes_per_s = 8e8 per 10 GbE link; one shared x3xx timing/performance table; KL-4/5/7/14/15/16, design 05/07/09/16/18. Also a narrowed test exemption for issue #108 in crates/ezsdr-radio-uhd/src/provider/rx/differential.rs (gh issue view 108 -R k3komatsu/Ez-SDRv4 --comments), tightened in the fix commits.
- B — the receive start head: x3xx::rx_fill_samples(D) (n_hb = min(v2(D),3) → 2/13/18/24 exact zeros) applied by MockRadio and FakeDevice at every receive start (fresh, restart, retune, timed, after an overrun restart incl. the Mock's back-pressure path, a start behind an unread tail, a reopened streamer), samples after it on time; not on MR-13's paths nor after a sequence error; bench1115 zeros verdict bound; MR-35, UR-33, KL-47.
- C — #75 (3): the open-time transmit flush is retried up to 3 times, then refuses arm/Connect with `UR-13: …` (OPEN_FLUSH_CONFIRMED and the single-attempt path removed); UR-13, UR-22, KL-12, KL-44.
- D — ProbeDevice (tests/common/probe_device.rs) clears rows per trial, searches under the lock, fails a trial on dropped rows; a FakeDevice rehearsal at 100 MS/s; receive-only #[ignore] hw_s5_rx_start_lead; plan/phase8/bench.md.
Each sub-step was reviewed by an Opus subagent; their findings are fixed. The full mutation lists passed 3051/3052 (the known survivor phase7/G09).

Review independently, writing your own failure scenarios first from the step's purpose, the code and the primary sources: correctness of the values and their every reader (admission at the boundaries, the 2 ms placement on both Providers, Python tests reading the start lead), the head at every start site and at none other (off-by-one across reads/blocks, interaction with gaps, SampleClock origin, two channels), the open flush loop (every transmit open path, counting, state after refusal, Stop/loss during an attempt, the user-visible refusal per EA-16), the probe fix (races between clear and new rows), the #108 exemption (can it hide another defect?), the texts (design/, KNOWN_LIMITATIONS) against the code and DA-1 (no history in a rule), hardware tests whose expectations come from a changed rule and their FakeDevice rehearsals (AGENTS §9 "Hardware tests and bench values"), tests that cannot fail, and maintainability (duplicated decisions, state in two places, leftovers, rows anchored on scaffolding). Severity: major (a minimal operation sequence and the wrong observable outcome), minor, note. Every claim with path:line. Finish with a numbered findings list and a one-line verdict.
