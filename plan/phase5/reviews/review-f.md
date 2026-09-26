# Review F — the reviewer's report

Adversarial dynamic review of Phase 5 by Claude Opus (AGENTS.md §8), brief [`../prompts/review-f.txt`](../prompts/review-f.txt), run on 2026-09-26 against commits `e13348c`…`35cdeaf`. Every build and probe ran in scratch copies outside the repository. The report is reproduced as delivered, lightly shortened; the triage and the fixes are in [`../implementation-notes.md`](../implementation-notes.md), "Review F".

**Verdict: CHANGES_REQUIRED.** 2 P0, 4 P1, 11 P2. Both P0s are contradictions in the normative text. The code works: 644 tests pass on stable and on 1.85.0, Clippy is clean, and the 20 listed mutations are all killed. All three holes of `00-overview.md` §2 reproduce at `0627839`.

## P0

**P0-1 — KC-9's middle sentence still contradicts KE-1.** `design/06-kernel-coordinator.md` KC-9 and spec 15's amendment ("its first and last sentences") left "an `Assembly.inputs` entry no schedule entry references is not kept, so the store holds only verified bytes". A listed input is referenced by no schedule entry, yet SB-20a says its bytes join the store and the code keeps every verified input. Read literally, the sentence puts hole 1 back. *Fix:* "an `Assembly.inputs` entry that the Spec's `inputs` does not list and no schedule entry carries is not kept", in design/06 and spec 15.

**P0-2 — NX-7 contradicts MA-24 and UC-2, and neither spec amends them.** MA-24: an admitted `UpdateParameter` reaching an Executor "is applied under the parameter's declared update class"; UC-2: "Producer obligation of every Provider, Executor and Sink". Spec 14 §2's "MA-19b is the only Phase 1 Module-API obligation still marked producer with no producer" is false: MA-24's applying half and MA-17 have none. Probe: the responder pushes `UpdateParameter { target: responder, key: …ping.threshold, class: Cold }`; the Kernel admits it; the Run ends `Failed { run }`, "KC-30: island_0: NX-7: ezsdr.exec.native 1.0.0 applies no Action, and UpdateParameter for local:responder reached it". *Fix:* a text amendment limiting MA-24's and UC-2's Executor obligation to Executors that apply Actions, with `ezsdr.exec.native` 1.0.0's refusal as the ceiling until Phase 10; correct spec 14 §2.

## P1

**P1-1 — KE-3's MA-14a sentence is too broad.** A probe Executor submitting from `start()`, before `Running`, gets `ezsdr.run_state` ("RS-18: the Run is not Running") and the Run then proceeds to `Stopped { client }`: MA-14a would have a Module swallow that as "the Run is ending", and "the termination already records why" is false there. The native Executor is unaffected (it submits only from `step`, and rounds run only while `Running` or in the drain, where dispatch is frozen first). *Fix:* restrict the sentence to Actions submitted from `step`.

**P1-2 — §58 #9 marked "Remaining: —" on evidence that is not the Vision's definition of a Reactor.** "Reacts to samples by emitting Actions" is `ComponentKind::Reactor`'s doc comment; Vision §19 defines a Reactor as "events / messages → state machine → zero or more actions". *Fix:* base #9 on §58's own minimal reactive test (A sends PING, B receives, Reactor, timed PONG); set Remaining to a Reactor fed by an event or message edge (Phase 10, the pending owner decision); correct the doc comment.

**P1-3 — The responder's state across a block boundary is tested only by one seed's luck.** A mutation that resets the rearm state at each block (a PING split across two blocks answered twice) survives `v58_12` and every carrier without jitter; only `v58_03`'s seed 8 happens to split the PING. Deterministic probe: a PING at 11 500 with no jitter spans 11 546–12 545 across the boundary at 12 000; HEAD gives one burst `(2018546, None, None, 500, 1)`, the mutation two. *Fix:* add the case to `v58_12` and the mutation to Appendix A.

**P1-4 — The brief's copy recipe and `mutate.py` give wrong results with the shared target directory.** `rsync -a` and `copytree` keep timestamps, so Cargo reuses whatever was last built for the crate from any copy, mutated builds included (a fresh copy failed `ke_02` with E05's mutated admission; 11 list mutations reported `BASELINE FAILED` until the files were touched, then all 20 were killed). *Fix:* copy without keeping timestamps (`rsync -a --no-times`; `copy_function=shutil.copy` in `mutate.py`), and say so in AGENTS.md §7. Residual: two reviewers building at once can still overwrite each other's binaries.

## P2

1. KE-4/R8's reason ("the stepping loop holds every Module") holds for stepped Modules only; a hardware Provider is not held. The conclusion stands on MA-14 and parity.
2. NX-3's "nothing is built" is true per component only: in an Island `[a, b]` with `b` unknown, `a` is built and prepared first (stop and cleanup still reach it).
3. KE-1 lets a Spec write Run-owned provenance into `Manifest.inputs`: `partial: true`, a made-up `DEVICE_LOST` mark, two inputs with one id — recorded verbatim (already possible for scheduled waveforms). Refuse them at KC-9.
4. KE-2's length check is tested one way only (`!=` → `<` survives).
5. `v58_10` checks only `use` lines: an inline `ezsdr_radio::kinds::TIME_ERROR` and a `pub use ezsdr_sim` survive; the forbidden list misspells the Mock crate `ezsdr_radio_mock`.
6. `ke_03` never checks that a decision was made; a drain that never steps an Executor survives it.
7. NX-8's "drops every kept handle" is untested.
8. The responder's rounding of a turnaround between samples is untested.
9. The test tables differ from the tests (`ke_02` has no listed → `Ok` case; `ke_01`'s Provider reads the store at the burst, not in `prepare`; `nx_03` checks only the descriptor's id).
10. `design/05` names `plan/phase5/14-native-executor.md` while spec 15 says `design/14-…`; add the path change to Step X.
11. The responder reads channel 0 of planar cf32 correctly; it ignores per-channel validity and counts its rearm in samples, not time, across a `GAP_BEFORE`; note it in `responder.rs`.

## The holes, reproduced at `0627839`

Hole 1: confirmed, `Failed { prepare }` with the PONG "not an input of this Run". Hole 2: confirmed, a probe Executor's burst naming absent bytes is admitted (`x:submit:ok:1`) and the Provider reports `burst_input:missing`. Hole 3: confirmed with the prototype's error-returning refusal; with NX-6 as implemented `also` is empty even on the parent Kernel, so KE-3 needs no Kernel code.

KE-2: no lock cycle (admission and Providers take the store lock after slot locks; nothing takes a slot lock holding the store lock); redundant for the schedule and Session origins; nothing previously allowed is now refused. No scope creep found. Reading "dynamic" as the decision with the waveform by reference is defensible (INFERRED). §22's "too slow … fails in simulation" is not met as written; the owner has to decide Vision issue 2.

## Decisions

R1 supported for §58's minimal reactive test, evidence cited wrong (P1-2). R2 supported; state that a Reactor sends only waveforms declared before the Run. R3 supported. R4 supported for §58 #11; §22 needs the owner (INFERRED). R5, R6, R9 supported. R7 sound, but MA-24/UC-2 unamended (P0-2). R8 conclusion supported, reason incomplete (P2-1). R10 supported, check narrower than GX-6 (P2-5). R11 supported, subject to build hygiene (P1-4). N1 supported (the hash is an author-chosen label: NX-3 catches a Spec/registration mismatch, not changed code under one label; INFERRED). N2, N4, N5 supported. N3 needs P0-2's amendment. N6 supported.

## Mutations and tests

The list: 20 of 20 killed after refreshing timestamps. The reviewer's own 22: killed F01–F06, F08, F09 (Executor), F12–F16 (KC-9), F17 (only by seed 8), F22; survived F07 (NX-8 handles), F10 (KE-2 one-way), F11 (equivalent), F18 (rounding), F19, F20 (`v58_10`), F21 (drain). Stable 644, 1.85.0 644, Clippy clean, `check_links.py` 393 links, 22 `v3/` paths present, `kernel_surface` 116 NEW / 292, no `#[ignore]`, `Cargo.lock` one workspace member added, Vision unedited.
