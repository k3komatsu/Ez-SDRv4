You are the independent code reviewer (AGENTS.md §9 step 7), read-only: do not edit, format, build, test, run mutate.py or remote_mutate.py, or run any git command that changes state. Repository: ~/local_work/Ez-SDRv4 (current directory). Never read ~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/ (it holds held-out evaluation data; reading it spoils stage E3).

## What to review
Stage E1 of design note 45's experiment: a deterministic simulation of the RX slice of note 45's decision core, built beside production code. All of it is uncommitted:
- crates/ezsdr-radio-uhd/src/brain/: core.rs (the brain, `step`/`op`, `decide`), cell.rs (the acceptance cell and its procedures), rules.rs (executor rules; consumption through production `RxChain::consume`), model.rs (the device model, mirroring FakeDevice), dst.rs (the generic explorer: exhaustive DFS with an exact key, bounded searches, walks, replay, shrink), toy.rs, world.rs (the simulated world: brain, settings executor S, RX executors R_k, locks L and G, flag F, device, environment, monitor), oracle.rs (I1–I16, P1–P4), cases.rs (cases 1–9, micro variants, seeded configuration, walks), mod.rs.
- crates/ezsdr-radio-uhd/src/lib.rs, crates/ezsdr-radio-uhd/src/provider/mod.rs, crates/ezsdr-radio/tests/generator/mod.rs (visibility changes only).
- plan/maintenance/tools/mutations.json: rows E1-01 to E1-130, appended. `git diff` shows them.

## Sources
- The experiment plan: ~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md. Read all of it, and especially §10's last rows (drafts 5–8 and two owner decisions). They record every design decision taken during implementation, and they are part of what you review.
- The test plan Sol approved before the code: ~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md.
- The design: plan/phase8/45-uhd-decision-core.md §4. Semantics: plan/phase8/44-*.md §3 (note 44) and the production code it models: crates/ezsdr-radio-uhd/src/provider/core.rs (RxChain), provider/rx.rs, device.rs (FakeDevice).

## How to review (AGENTS.md §9 step 7)
1. **First**, from the plan's purpose, note 45 §4, the production code and FakeDevice, write your own failure scenarios for this simulation, before reading the brief's list or the tests. Cover:
   - (a) places where the brain's decisions would be wrong in a real concurrent system yet look right in the model;
   - (b) places where the world, device model or oracles diverge from production or FakeDevice, so that the experiment would miss real bugs or report false ones;
   - (c) unsound merges in `World::key`: two states the key makes equal whose futures or verdicts differ. Read every canonicalization in `key` and its comment;
   - (d) oracles that are weaker than the plan's text, or fire on legal behaviour;
   - (e) tests or mutation rows that do not observe what they claim;
   - (f) maintainability as AGENTS.md §9 defines it: duplicated decisions, lifecycle facts kept outside their owner, leftovers, test bloat.
2. **Then** compare your scenarios with the code, the tests and the §10 decisions. Judge each decision on its merits: production-faithful, sound, and not a weakening of an oracle or a case.
3. Every major finding gives a minimal operation sequence (pseudocode is fine) and the observable wrong outcome it leads to (AGENTS.md §9 "Counterexamples"). Give `path:line` for each finding.

## Output
A verdict, PASS or REVISE. Then findings ranked P1 (major), P2 (minor) and note. Each finding gives:
- its category, (a)–(f);
- `path:line`;
- the counterexample or evidence;
- the fix you propose.

End with what you did not check.
