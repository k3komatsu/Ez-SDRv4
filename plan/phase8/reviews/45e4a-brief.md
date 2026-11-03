# Brief: note-45 experiment E4a — the two bounds, the DST holes, four model concepts, the replay order

You are the implementer (AGENTS.md §9). Repository `~/local_work/Ez-SDRv4`, branch `main`. Edit in place: you are the only editor of the tree. Read AGENTS.md first. Do not cd into `v3/`. **Never read, list or search `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/` or `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout-e4/`** (the second is E4's hidden holdout; the evaluation depends on your not seeing it).

**This step is risky** (a queue and its bound, deadlines, lifecycle, the boundary between the model and the device). It runs in two phases. **Phase 1 (now): write the test plan only, report, and stop. No code.** Phase 2 starts only after Sol has reviewed the plan and the orchestrator says so.

## Context (read these)
- Design: `plan/phase8/45-uhd-decision-core.md` (note 45, sixth draft), all of §4, especially §4.4, §4.5 (the new paragraph 「上限は二つ」), §4.7, §4.9 (the new paragraph 「実行役の生成の失敗」), and §11's last entries.
- Plan: `plan/phase8/45-experiment.md`, all of it; **§11 is this round**: stages E4a1 and E4a2, the fixed K = 4 (device-wide) and D = 64 (per chain), the criteria, and sequences B1–B15.
- Why: `plan/phase8/45-report.md` §3 and §5; the design reviews `plan/phase8/reviews/45e4-design-sol*.md`; the replay diagnosis `plan/phase8/reviews/45e3-sol-s1c-diagnosis.md`; E3's results `plan/phase8/reviews/45e3-holdout-results.md` and `45e3-past-bugs.md` (the first holdout is spent and may be read).
- Code: `crates/ezsdr-radio-uhd/src/brain/` (`core.rs`, `cell.rs`, `rules.rs`, `model.rs`, `world.rs`, `dst.rs`, `cases.rs`, `threads.rs`, `shared.rs`, `force.rs`, `observed.rs`, `thread_tests.rs`, `speed/`). Today's owner for the semantics the four new concepts copy: `src/provider/mod.rs` (spawn, stop order, cleanup, around `:630-740`), `src/provider/core.rs` (`abandon_unowned`, `device_lost`), `src/uhd.rs:342-365,1174-1175` (the native destructor and the loss mark).

## What to build (under `src/brain/`, test-only)
**E4a1**
1. **Two bounds** as note 45 §4.5 「上限は二つ」 says, counted from the value's §4.4 state, its instant and its obligations, with no separately updated counter or flag. K is shared by all chains; D is per chain and counts every resident value from reservation, including K's and reported `left`. Replace the single `in_flight_bound` (the E2b diagnostic's override becomes two fields, default K = 4, D = 64).
2. **Device time by input.** The brain learns device time only from inputs that carry it (today's `Input::Timer` carries none). R's progress must not be used. A capacity timer is a kind distinct from the drain timer; a stale, duplicate or post-`F` capacity timer only re-evaluates.
3. **Held re-evaluation** on `Done`, cancel/`Dropped`, consumption, and time input; when waiting on time, one timer at the earliest future instant among the counted values.
4. **DST holes:** I11′ (replaces I11: K over the device and D per chain, each computed independently from the monitor's facts); I17 (Stopping reopens only on the executor's confirmation of a start the brain issued for that epoch); new cases for B1–B10, each with witnesses. Use micro variants within the existing budgets where a case is exhaustive.
5. **H03's double guard** (`cell.rs` reserve's `progress >= cut` next to `publish_planned`'s `stop_accepting`): search every transition that changes progress, cut or phase (publish, Stop booking, plan cancel, LateCommand). If each one that makes progress ≥ cut closes acceptance in the same section, remove the redundant guard and add the invariant "accepting ⇒ progress < plan cut" to the oracles; otherwise add the case that separates them. The plan states which, with the search.
6. **Replay order (B15):** make the world and the threads send R's end-of-recv inputs (`LateCommand`, publication boundaries) in one order, from one shared function (Maintainability 1), choosing the order the design implies and saying why; fix the shrink display for divergences after the first three (`thread_tests.rs:643-653`); then classify every divergence of `s1c_whole_walks_wide` at seeds 1–10,000.
7. Threads, `speed/` and conformance follow the core changes; nothing else in them changes.

**E4a2**
8. **Loss recording** (procedure 4): model the loss record as an event; I18: F before the record and before `mark_lost`.
9. **Native handle destruction:** model the handle's destroy at cleanup and the destructor's retention when the loss mark is set; I19 (B12).
10. **Join timeout and detach** (§4.7, §4.9): a join limit in virtual time; shutdown detaches after it, retains handles, closes nothing; I20 (B13).
11. **Executor spawn failure** (§4.9, receive streams only; TX is outside the slice): an injectable spawn failure per stream, before and after its `rx_start`; I21 (B14).
Each gets a case (13–15 or as the plan numbers them) and is in the seeded configuration where it fits.

Out of scope: FakeDevice changes, production code outside `src/brain/` (allowed only where E1/E2 already opened visibility, named in the plan), `design/`, the Kernel, schemas, Python, the speed measurement runs (E4b).

## Phase 1 deliverable: `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan-e4a.md`
- For each sequence B1–B15 and each oracle I11′, I17–I21: the test, what it asserts, how it observes the behaviour, and the wrong behaviour that fails it (AGENTS.md §9 step 2).
- The input and timer changes (types, who sends device time on the DST and on threads).
- How K and D are computed (the function, the facts it reads) and why nothing else stores them.
- H03: the search and the decision.
- B15: the chosen order and the reason.
- What the four model concepts add to `model.rs`/`world.rs`/`threads.rs`, and which existing case or the seeded configuration exercises each.
- The file layout and a size estimate (body and tests, per stage). Say if you expect more than 3,000 lines in all.
- Mutation rows you plan, one line each, from E4-01. Token anchors for Rust (AGENTS.md §7).
- Open questions.

Phase 1 reads code but changes nothing in the tree.

## Self-review checklist (phase 2; one round, AGENTS.md §9 step 5)
The orchestrator will give the reviewer command in phase 2. Its checklist will include: every planned sequence has its test, the test observes what the plan says, and a row breaks it; scope; every "every/only/all" claim searched; design text made false; maintainability; and **for each test time read or wait you add: the verdict does not depend on when the test thread runs (note 37's rule, `plan/phase8/37-test-thread-timing.md`)**.

## Build and test
- Local cargo is allowed. Build output stays in `~/.cache/cargo-target/Ez-SDRv4`. Test temp files on `/tmp`. Heavy sweeps may run on sim02/sim03 through `plan/maintenance/tools/remote/rdev.py`.
- Never run `mutate.py` or `remote_mutate.py`. Never git commit, push, stash, reset or checkout.

## Report (phase 1, at most 15 lines)
The plan's path, the size estimate, open questions, and anything in this brief you think is wrong, with the reason.
