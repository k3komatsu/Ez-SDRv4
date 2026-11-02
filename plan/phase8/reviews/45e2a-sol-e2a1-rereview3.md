## Verdict: PASS
Ready to close **after the orchestrator’s remote row run passes**. No new P1/P2 blockers.

Paths below are relative to `crates/ezsdr-radio-uhd/src/`, unless stated otherwise. Mutation killers were assessed by inspection.

| Item | Assessment and evidence |
|---|---|
| Previous P1: I14 | **Closed.** Recovery sets `F` before releasing `G` (`brain/threads.rs:347–356`). R rechecks `F` after acquiring `G` (`561–568`). |
| Previous P2: E2-44 | **Closed by inspection.** S10b no longer builds its trace through the mutated core. It drains the runtime’s operations, then checks routed commands before any acknowledgement (`brain/thread_tests.rs:817–831`). E2-44’s premature Start reaches that first assertion. |
| Guard order | **Matches DST:** Gate → Set → Ungate → Mark (`brain/world.rs:63–70`; `brain/threads.rs:349–358`). This satisfies note 45 §4.5 procedure 4: `F` precedes `mark_lost` and is published under `G` (`plan/phase8/45-uhd-decision-core.md:136`). |
| Panic holding cells | **Safe for the reviewed unwind path.** `Swap::drop` restores scratch cells before cell guards leave scope (`brain/shared.rs:56–67,195–199`). S5b checks restored identity and a subsequent device call with no cell held (`brain/thread_tests.rs:592–612`). |
| Panic holding `G` | **Safe.** Recovery recognizes brain ownership, skips reacquisition, and releases the gate after publishing `F` (`brain/threads.rs:347–355`). S5c covers all four guard-operation panic positions (`brain/thread_tests.rs:873–879`). |
| Indefinite wait | **Possible if `L` never becomes available:** R holds `G` while waiting for its device call, and recovery waits for `G` (`brain/threads.rs:563–572`; `brain/model_device.rs:87–105`). This is the existing native-lock qualification, not a new circular dependency: recovery retains no cell lock. Note 45 §4.7 explicitly excludes this case from bounded shutdown (`:155`). |
| Old recovery leftovers | **None found.** The three deleted recovery names have no matches in `src/brain/`. Unwind invokes only `guard_routine` (`brain/threads.rs:399–405`). The ordinary DST `Input::Panic` operation sequence remains intentionally (`brain/core.rs:335–338`). |
| S5c / S5d | **Observe recovery completion.** They inject inside operations, verify flag/mark order, gate release, termination routing and actual brain-thread exit (`brain/thread_tests.rs:848–899`). |
| S5e / S5f | **Claims supported.** S5e deterministically observes R blocked, `F` published with brain-owned `G`, then refused close and retained handle (`brain/thread_tests.rs:913–923`). S5f supplements this with unrestricted scheduling; it is not the deterministic killer (`929–945`). |
| Dedupe reversal | **Confirmed.** Every publication records its step and actually calls `mark_lost`; shared publication no longer suppresses repeats (`brain/shared.rs:279–293`). |
| Mutation rows | All eight reviewed anchors match exactly once. E2-10/11/39/42/43/44/47 have matching regressions and deterministic failure paths. E2-48 deterministically exposes recovery to the second injected panic, with the wording caveat below. |
| Closure | **No blocking code finding.** Remote mutation results remain required; inspection is not an executed kill result. |

### New findings
- **P3 — E2-48 overstates its mechanism.** It inserts the operation-panic counter/check directly into recovery; it does **not** restore execution through `run_ops` (`plan/maintenance/tools/mutations.json:18535–18539`). Rename it to “the guard routine is reachable by operation-panic injection.” Nonblocking; its killer is valid.

Not checked: test/mutation execution, full gate, hardware, or the prohibited holdout directory. No files, formatting, or git state changed.
