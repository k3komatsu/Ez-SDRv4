# Brief: E4a2 (continuation of brief-e4a.md)

E4a1 is closed (Sol PASS after five rounds; committed by the orchestrator). Implement **E4a2** per `brief-e4a.md` items 8–11 and your amended test plan `test-plan-e4a.md` (B11–B14, I18–I21, cases 13–15, plus the dispositions in `dispositions-sol-e4a-plan.md` that concern them: finding 6 — destructor entry modelled separately from cleanup close, retention decided by the native mark not F, the missing-mark path, a loss between cleanup's eligibility check and destruction; and from Sol's coverage table — late execution after detach; spawn failure of the first stream, suppression of later spawns, a prior loss; P2/P3 hold).

Lessons from E4a1, now rules for this stage:
- Oracles get no blanket exemptions. If a sweep fails a new oracle, report the trace as a finding; an exemption must name its exact condition and why no obligation is lost, and the orchestrator rules on it.
- Every behavioural row must be killed by a reachable DST or unit test that observes behaviour; oracle self-tests are listed separately.
- Save every sweep, s1c and probe run's output to `/tmp/e4a2-*.log` and name the logs in the report.

Scope, rows (next free E4 id), checks (brain suite, walk sweeps and `s1c_whole_walks_wide` at seeds 1–10,000, timing-dependent thread tests 20× with the final binary, clippy `-D warnings`, `cargo fmt --all`, chmod, `rules.py --base HEAD`, out-of-tree probes), and the self-review (one round, DeepSeek free, the checklist of brief-e4a.md including note 37's rule; `/tmp/e4a2-selfreview.log`) are as in E4a1. Never read the holdout directories; no git state changes; never run mutate.py/remote_mutate.py. Plan §16 records the stage.

Report (at most 25 lines): per B11–B14 and I18–I21 the test and result; rows; self-review findings and dispositions; suite, sweeps, s1c with log paths; size; anything wrong.
