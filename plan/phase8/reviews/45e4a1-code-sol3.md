**REVISE — no P1; one P2 remains.**

**P2 — The interleaving exemption is not exact or safe.**  
`crates/ezsdr-radio-uhd/src/brain/oracle.rs:429–439`; `crates/ezsdr-radio-uhd/src/brain/world.rs:1485–1488`.

Any executor event disables the entire step-end check, including events that change neither cells nor capacity and send no input.

Minimal counterexample:
```text
K=1; A completed past its instant; eligible B held.
inbox=[Done(A, now≥e_A), Shutdown]; R is ready to receive.
BStep(Done); RCheck schedules Snap/Begin/End/Publish.
BOp(Done), incorrectly omitting its retry; operations finish.
BStep(Shutdown); finish shutdown.
```
`RCheck` only schedules receive work (`world.rs:1048–1065`), but sets `interleaved=true`. P6 therefore misses the omitted retry. State-level P6 is suppressed by queued Shutdown; Shutdown cancels B (`core.rs:609–616`). **Observable wrong outcome:** B is cancelled without the reservation owed by the Done step. No later step owes that reservation; RCheck generated no input.

Track the retry’s actual decision state/obligation, rather than exempting every executor interleaving. Add this negative regression. The current test instead explicitly accepts the omitted-retry state after merely setting the flag (`oracle.rs:1003–1012`); E4-50 protects that acceptance.

- **Prior P2: not closed.** Queue exemptions are gone, but the same escape survives through unrelated executor activity.
- **Prior P3: closed.** Case 12a correctly describes the cut above both instants (`cases.rs:420–427`).
- **800,000-state budget: acceptable.** `oracle_happy_path_passes` still requires exhaustive completion (`oracle.rs:1322–1326`); 414,041 states was not independently reproduced.

Not checked: test/mutation execution, hardware, E4a2, or excluded holdout material. No files or git state changed.
