# Review G — the reviewer's report

Re-review of Review F's fixes (commit `c70b472`) by Claude Opus (AGENTS.md §8), brief [`../prompts/review-g.txt`](../prompts/review-g.txt), 2026-09-26. Every build and probe ran in scratch copies outside the repository. Reproduced as delivered, lightly shortened; the triage is in [`../implementation-notes.md`](../implementation-notes.md), "Review G".

**Verdict: CHANGES_REQUIRED.** No P0, 4 P1, 9 P2. Every fix is small; two new defects are in normative text. The code is correct: 644 tests on both toolchains, Clippy clean, all 27 listed mutations killed.

## Review F's findings

P0-1 closed (a new defect in the same rule came with P2-3's insertion: G-2). P0-2 partly closed: MA-24 exempts an Executor that applies no Action and spec 14 §2's corrected claim is VERIFIED (the producer rows of the Phase 1 exit table are MA-3, MA-8, MA-13, MA-17, MA-19b, MA-24, MA-28a; only MA-17 and MA-24's applying half have no producer), but UC-2's wording and UC-1/UC-3…UC-6 remain (G-1). P1-1 closed (VERIFIED at all four `round(` call sites; the drain runs after `frozen` is set, and `admit_with` checks `frozen` first). P1-2 closed in substance, with two text slips (P2-1, P2-2 below). P1-3 closed (E21 fails `v58_12` three times out of three at the straddle assertion). P1-4 not closed (G-4). P2-1, P2-2, P2-4, P2-6, P2-7, P2-8, P2-9, P2-10 closed. P2-3 implemented, see G-2 and G-3: it refuses no valid Spec in the repository and no Session (KC-28 builds its references with id `input_<k>`, `partial: false`, empty marks and continuity); the id rule is right (INFERRED). P2-5 closed for `ezsdr_` paths; an inline `crate::` path still passes (P2-4 below). P2-11 added, with a wrong sentence (P2-3 below).

## New findings

**G-1 (P1) — KE-5 scopes the UC rules in the wrong place.** UC-2's marker "every Provider, Executor and Sink that applies updates" restricts all three, so a Provider or Sink that applies no updates falls outside UC-2; and UC-1 still ends "UC-2…UC-6 bind the target" while UC-3…UC-6 keep unqualified markers, so UC-3 still binds `ezsdr.exec.native` for the responder's `cold` parameters. *Fix:* UC-2's marker "Producer obligation of every Provider and Sink, and of every Executor that applies Actions; an Executor that applies none refuses the update instead (MA-24; Phase 5, KE-5)"; UC-1's last clause "UC-2…UC-6 bind the target, except an Executor that applies no Action, which refuses it (MA-24, KE-5)".

**G-2 (P1) — KC-9's new sentence was spliced in and inverts "Anything else is refused".** The antecedent of "Anything else" became the list of refused inputs, so read literally every other input is refused; the stage and reason prefix no longer attach to the new refusals; a capital "An" after a semicolon; and it cites "Phase 5 Review F" rather than KE-1 (GX-2). *Fix:* make the new conditions part of the list of requirements, citing KE-1, before "anything else is refused".

**G-3 (P1) — two of KC-9's new refusals are untested.** G01 (drop the `continuity` check) and G02 (the id rule for listed inputs only) together survive the whole workspace (644 passed). *Fix:* test an input with one `ContinuityMap`, and a listed input and a scheduled waveform sharing id `waveform` with two hashes; add both mutations.

**G-4 (P1) — a fresh-timestamp copy still reuses a later build.** The copy was made at 19:32:45; the mutation tool then built E27's mutated kernel last (19:38:39) and restored the file without rebuilding; the unmutated copy's next `cargo test` finished in 0.05 s with nothing compiled and `ke_02` failed. Cargo reuses any artifact newer than the copy's files, whichever copy built it; the failure is sequential interleaving, not concurrency. *Fix:* refresh the copy's timestamps (`find <copy> -type f -exec touch {} +`) immediately before each build session; say "interleaved" rather than "at the same time".

## P2

1. `00-overview.md` §3's event-edge row lost a cell separator. 2. It cites "§65's reactive packet radio"; the chain is in §66. 3. `responder.rs`: a gap *lengthens* the wait in time, it does not shorten it; a PING wholly inside a gap is not heard. 4. `v58_10` misses an inline `crate::` path (a probe `let _ = crate::rig::assemble;` in `responder.rs` passes). 5. MA-24's first sentence is still unconditional, and its marker names no carrier for the Executor's refusal. 6. Spec 15's `Amends` row omits design/06's UC-2 and both schema descriptions. 7. The brief's `rsync` copies `.cargo/config.toml`, whose `target-dir` is the main tree's; add `--exclude .cargo`. 8. KC-9's continuity refusal makes a capture's `ArtifactRef` unusable verbatim as an input — say how a produced artifact is reused. 9. Vision §27's "a key with no class cannot change during a Run" is not expressible (`ParamDecl.update_class` is mandatory): record it as an owner decision before the freeze.

## Mutations and tests

The list: E01–E27 killed. Review F's survivors F07, F10, F18, F19, F20, F21 re-run: all killed. The reviewer's own: G01 SURVIVED, G02 SURVIVED, G03 killed, G04 killed, G05 (`crate::rig` in the responder) SURVIVED, G06 killed. Stable 644, 1.85.0 644, Clippy clean, `check_links.py` 397 links, `ov_23b` 116 NEW / 292.
