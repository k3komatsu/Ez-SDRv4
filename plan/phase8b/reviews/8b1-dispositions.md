# 8b-1 — dispositions

Step: one shared 1 s join bound per stop stage (D-6, `plan/phase8b/01-device-bundle.md` §3.5); closes debt #20 (`plan/phase8/52-phase8-end-review.md`). Risky (a deadline): test plan first (AGENTS §9 steps 2–3). Implementer: a Claude Sonnet subagent alone (plan, implementation, fix round).

Findings: test plan v1 — Sol 3 majors + minors; plan v2 re-check — Terra 2 majors + minors, Opus 2 majors + 6 minors; code — Sol 1 major + 3 minors; fix re-check — Terra PASS, Opus PASS (two stale comments fixed by the orchestrator). Mechanism: the plan's majors were tests deciding by wall time or racing the joiner (`wall-clock-test-oracle`, design note 51) and observing receipt instead of enqueue; the code major a missing test. No production defect was found after the plan.

All adopted, decided by the orchestrator:
- Plan v1 (Sol): budget and completion verdicts decided through a test-driven deadline seam (`join_now`; hook points and types test-only), no elapsed-time upper bounds; ordering observed at enqueue and at each stage's decision point; the decision point specified as one completion sweep after expiry; mutation-row and spec corrections (`sol-dispositions`, in 8b1-brief's work dir).
- Plan v2 re-check (Terra, Opus): S9 rebuilt on a unit rig with the thread already finished; `finished(name)` asserts on a name the stage was not given; S6 observes at the decision points (a `Sending` hook point added); production keeps only `join_now` and `join_stage(names)`; R-S7a-wall dropped; UR-16 keeps KC-46b's distinction from RS-8a's 5 s.
- Code (Sol): the mixed-stage test releases TX1 at the last poll, so a sweep reusing the previous poll's results fails (row P1415); UR-30 states when uhd-control's plan row is written and UR-26 cites it; handshake timeouts report `EZSDR-INFRA`; P442/P1404/P1405 anchors narrowed with `old2`; P245's description.

Not changed: the workspace test `t200_bare_uhd_script_judges_a_run_and_bounds_it` failed once in a full local run (its signal-trap self-check) and passes alone; unrelated to this step, watched at the gate.
