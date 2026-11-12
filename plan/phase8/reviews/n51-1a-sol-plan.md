Independent failure scenarios, before reading the test plan:

- A thread registered on A and B waits or queues for a lock on A: only A’s registration may become idle; B must remain busy. Clock-local IDs can collide.
- Dropping A before B, or unwinding through either guard, must remove exactly that registration. A rejected duplicate—including `drive()` inside a same-clock `spawn`—must neither allocate nor disturb the original participant.
- Clock-lock handoff and notify must mark the correct clock’s participant busy before it resumes; otherwise time can escape while it is descheduled.
- Joining under a driver must release its busy status during each wait, yet retain a real deadline even if another participant freezes manual time. Real-deadline expiry and notify have different clock-stopping semantics; the tests must distinguish them.
- Later `UhdRig`/`Direct` adoption needs an owned, thread-bound guard, registration after Authority construction, and driver removal before Authority teardown, including unwind. Two manual rigs must remain independently registered; real-clock guards must not disturb either.

I’ll now compare these with the brief, plan, and probe.
## Verdict: FAIL

Two major test gaps need correction before implementation. Review was read-only at `1231462`.

Paths below abbreviate `/home/komatsu/.cache/ezsdr-fixes/2026-11-12-n51-1a/` as `$review/`.

### Major

**M1 — No test waits on the earlier clock while both registrations remain live.**  
**VERIFIED coverage gap; INFERRED counterexample.**  
`$review/test-plan.md:30–33,49`; `$review/probe/probe-1a.diff:306–335,383–399`.

Minimal sequence:
```text
da = A.drive()
db = B.drive()
A.sleep(d)                 // both guards still live
```
A wrong lookup using `entries.last().filter(clock_matches)` passes the described cases: test 2 has only one registration; test 4 drops A before waiting on B; E4 drops B before waiting on A. In this sequence, however, A is treated as a non-participant while its registered ID remains busy: **A’s sleep hangs**.

Add cases that wait on A and B with both guards live, asserting that only the selected clock becomes idle. Include the clock-lock queue path and add the last-entry-only lookup mutant.

**M2 — Setup races can hide a broken join or fail a correct implementation.**  
**VERIFIED missing synchronization; INFERRED scheduling outcomes.**  
`$review/probe/probe-1a.diff:171–189,361–379,468–510`; `$review/test-plan.md:36–40,47`.

Minimal sequences:

- **J1 false pass:** worker finishes its 1 ms sleep → joiner registers → broken `std::thread::sleep` join sees an already-finished handle. The intended mutant survives.
- **E2/test 7 false failure:** sleeper completes → driver registers → `try_recv` finds completion and fails a correct implementation.
- **J2 false failure:** controller is descheduled throughout the join’s approximately 1 s lifetime → controller misses every transient `blocked()` state → fails after 10 s although join worked.

Register the driver before spawning sleepers; handshake J1’s pending sleep before joining. Observe J2’s wait through a held entry hook rather than transient polling. Anchor its elapsed-time check inside the joiner immediately before `join`, not before spawning it. Remove E2’s 100 ms wall sleep; only test (1) has that stimulus exception.

### Minor

- **Probe evidence needs reconciliation — VERIFIED.**  
  `$review/test-plan.md:9,46,65,78` claims results not established by the supplied log: `probe/mut.log:3` lists only J1 for the missing-wait-ID mutant; its 15 lines contain no no-registration mutant. Correct the claims and verify the final exact filters through `remote_mutate.py`. The current custom runner is not gate evidence.

- **UR-16 wording overpromises — VERIFIED.**  
  `$review/test-plan.md:111` says manual participants “run while it waits.” Another busy participant can still freeze them. Say **the joining participant is idle during the clock wait; the join bound remains real time**. Real-deadline expiry does not cap manual time (`crates/ezsdr-radio-uhd/src/host.rs:331–335`); do not imply that guarantee.

### Accepted

- Rows **14–15 already declare `"kill": "bound"`**, as required.
- Real-clock nested `drive()` remaining undetected is consistent with the explicit no-op choice: no second registration occurs. Scope duplicate-registration wording to manual clocks.
- UR-33’s M1 addition and M6 example correction are appropriate. Later 1b/1c still require registration after Authority construction and guard-first teardown, including unwind.

No edits, builds, or new probes performed. Counterexample execution and final mutation verification remain unchecked.
