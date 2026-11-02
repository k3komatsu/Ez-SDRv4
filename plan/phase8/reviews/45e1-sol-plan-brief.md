# Sol: test-plan review for the design-note-45 receive-slice experiment (E1a + E1b)

You are the independent reviewer (AGENTS.md §9 step 3). Read-only: do not edit any file. Repository: ~/local_work/Ez-SDRv4 (main). Do not cd into v3/. Do not read anything under `~/.cache/ezsdr-fixes/2026-11-01-n45/holdout/`.

Context: design note 45 (`plan/phase8/45-uhd-decision-core.md`) proposes a pure decision core, per-stream acceptance cells and executors for the UHD Module, and a throwaway experiment (§7) on the receive slice before any migration. The experiment's shared plan is `~/.cache/ezsdr-fixes/2026-11-01-n45/45-experiment.md` (draft 2: stages, actors, event granularity, search, seven cases, oracles I1–I13 and P1–P5, pure-level sequences U1–U18). The implementer's test plan is `~/.cache/ezsdr-fixes/2026-11-01-n45/test-plan.md`. Note 45 §7 requires that **before implementation** the event lists and granularity of the exhaustive cases are fixed and approved in your plan review; this review is that approval (or not).

Method (in this order):
1. From note 45's purpose, the existing code it replaces (`crates/ezsdr-radio-uhd/src/provider/core.rs` RxChain, `provider/rx.rs`, `provider/control.rs` handoff, `src/device.rs` FakeDevice RX/DDC, `src/uhd.rs` native locks) and the primary sources it cites (UHD 4.10 docs/source), write your own failure scenarios for the receive slice and for the experiment itself (ways the experiment could pass without proving C1–C3: a model too coarse to express a real interleaving, an oracle that cannot fail, a harness that misses states, a device model that diverges from FakeDevice/UHD, cases too small to contain the races of note 45 §7's past-bug table).
2. Only then compare with the experiment plan and the test plan.

Review:
- Event granularity (§3.2): is each atomic event really atomic in the proposed design, and is every interleaving that matters representable? Name any event that must be split or merged.
- Search (§3.3): does DFS with whole-state hashing plus terminal-state progress checks soundly cover the safety and progress claims; is cycle detection right; is the budget rule sound.
- The device model (§3.1, G6–G8): is it faithful enough to FakeDevice and to UHD for the cases (head-of-line DSP FIFO, in-order radio queue effective time, host buffer, native lock).
- Cases 1–7 and the seeded configuration: does each contain the interleavings it claims; are any of note 45 §7's six cases weakened; what is missing.
- Oracles I1–I13, P1–P5: can each fail; does each observe the behaviour, not the implementation's own bookkeeping; any oracle that compares the core with itself.
- Pure-level tests U1–U18 and harness self-tests: each observes what it claims and fails under the named wrong behaviour.
- Mutation rows (section F of the test plan): each breaks a test that observes the behaviour.
- Scope and size (E1a ≤ 1,500, E1b ≤ 1,500 lines): realistic?
- Maintainability (AGENTS.md §9, items 1–5) of the planned structure.

Severity: major / minor / note. A major gives a minimal operation sequence and the observable wrong outcome (AGENTS.md §9 Counterexamples). End with VERDICT: PASS (approve the granularity and cases as written), REVISE (list what must change before code), or FAIL. Output in English.
