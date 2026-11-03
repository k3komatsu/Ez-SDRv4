**REVISE — one P2; no P1.**

- **Original counterexample: caught.** `BStep(Done) → RCheck → operations omitting retry`, with Shutdown queued, now fails P6b (`oracle.rs:449–455`). Neither interleaving nor inbox contents exempts it.
- **P2 — Scheduling a retry can still substitute for actually attempting it.**  
  `crates/ezsdr-radio-uhd/src/brain/world.rs:782–783`; `crates/ezsdr-radio-uhd/src/brain/oracle.rs:433–435`; `crates/ezsdr-radio-uhd/src/brain/core.rs:519–523`.
  Minimal negative sequence:
  ```text
  K=1; A finished past its instant; eligible B held; inbox=[Done(A), Shutdown].
  BStep(Done); RCheck; BOp(Done); BOp(ReserveAll).
  Fault: ReserveAll omits queuing Reserve(B); operations end.
  BStep(Shutdown); B is cancelled.
  ```
  P6b passes because ReserveAll set `step_retried`; P6a never runs because no Reserve event exists. State P6 remains suppressed by queued Shutdown. **Observable wrong outcome:** B is cancelled without the owed reservation attempt, undetected.
  Smallest fix: credit an actual `Reserve`, or independently assert that ReserveAll with a held head queues its matching Reserve. Add this negative regression.
- **P6a evaluation point:** Reserve is correct: executor activity between ReserveAll and Reserve can change eligibility. Do not require reservation against the earlier ReserveAll state.
- **Freeing set:** matches the current input dispatch and retry-producing sections; no missing input variant found.
- **Rows:** E4-41/42/49/51–54 target their stated changes coherently by inspection; E4-48/50 are removed. None covers the scheduling gap above. Runtime kills were not independently verified.

Not checked: test/mutation execution, hardware, E4a2, or excluded holdout material. No files or git state changed.
