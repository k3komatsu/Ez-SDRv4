# Bench 2026-11-01 preparation — review record (2026-10-31)

Step: production-path hardware tests for D9b-1a, D9b-2, L1, L2 and issues #70/#72 (`crates/ezsdr-radio-uhd/tests/hardware.rs`: `hw_p72_timed_tx_chunks_are_timed_and_flushed`, `hw_p70_one_sample_burst_then_timed_burst`, `hw_p4_rx_timed_tune_then_early_stop`, `hw_l1_held_rx_tune_applies_when_its_segment_streams`, `hw_noise_floor_receive`; fixes to `hw_txp_b0_send_sizes`, `hw_tx200_hot_path_probe`, `hw_b9_transmit_only_session`), the shared tone helpers in `tests/common/tone.rs` with a pure test, the runbook `plan/phase8/bench.md` §「2026-11-01 の手順」, and the ssh-drop experiment `plan/phase8/tools/ssh-drop-tx.sh` + `ssh-drop-driver.py`. Measurement tooling (not risky): no test plan; Sol pre-checked the brief before code (owner rule for hardware briefs). Implementer Haiku 5.5 (`anthropic/claude-haiku-5-5#high`), no self-review. Brief and fix briefs: [bench-prep-brief.md](bench-prep-brief.md). Gate: [bench-prep-gate.json](bench-prep-gate.json) — full run FAIL only on two mutation rows the tone-helper refactor broke (P995 matched twice, P998 not at all) and the matching `rules` problems; both re-anchored to the same mutant (`scope`), then [bench-prep-gate-fix.json](bench-prep-gate-fix.json): rules 0, both killed. Otherwise workspace 2164, uhd 593, Python 57+57, mutation 2004/2007 with G09 known.

## Findings and dispositions

| Round | Source | Sev. | Finding | Disposition |
|---|---|---|---|---|
| pre-check | Sol | P2 | Production tuning is AUTO/AUTO: a frequency update mostly moves the LO, so a "DUC shift inside a burst" test can pass without the DUC | #72 rests on production T1/T2/T3 evidence (every payload send timed, closing send ≥ 2 samples, flush ACK) plus raw P6; the optional DUC check was later removed |
| pre-check | Sol | P2 | P4 cannot see post-cut samples through a capture; a fresh Run resets the blocks | Probe-retained device samples, effective cut and stop booking observed; INCONCLUSIVE otherwise |
| pre-check | Sol | P2 | L1 early booking does not exercise the held path | Two-segment wiring smoke; held path printed "not exercised" |
| pre-check | Sol | P2 | The ssh test bypassed `ezsdr-server`'s disconnect path and could manufacture silence | Real client over `ssh -T`, Attached Lease, independent receiver, verdicts before any reopen |
| pre-check | Sol | P2 | `tone_start` resolution vs "± a few samples" | One-sample vs two-sample control comparison |
| pre-check | Sol | P2 | `hw_txp_b0_send_sizes` stale (T2 closing send) | Fixed |
| pre-check | Sol | P3 | H1/H2 raw; TX200 verdict weak; wideband TX200 not bounded by `SAFE_TX_HZ`; T3 needs positive evidence | Fixed; TX200 marked for owner confirmation |
| code | Sol | P2 ×6 | Capture deleted before read; obsolete ssh profile; P4 early cut not established; forced cleanup could pass C2; 1a vacuous T2/T3; runbook loops continued after failure | Fixed (fix round 1) |
| code | Sol | P3 | #72 1b prerequisite from another handle | 1b removed |
| re-review 1 | Sol | P2 ×2 | C2 deadline not anchored at the drop; unbounded receiver command | Fixed (fix round 2), mocked-clock scenarios |
| re-review 1 | Sol | P3 | Same-Run successor reachable (stop + `start_rx`) | L1 two-segment smoke built; P4 keeps a labelled second Run (finish-driven evidence; Sol: reason holds as a scoped deferral) |
| re-review 2 | Sol | P2 ×2 | Receiver background child could block cleanup; L1 transition dated across missing samples | Fixed (fix round 3): `setsid` + output file + group kill; contiguity rule shared with `classify_tone`, pure test |
| re-review 2 | Sol | P3 | C1 unobserved during a slow receiver call | C1 from `docker inspect` FinishedAt |
| re-review 3 | Sol | P3 | An interrupted `rx_state` could leave receiver descendants | Orchestrator: exit cleanup kills the recorded receiver group; `rx_state` clears the record after its own kill (no reused group id signalled) |

Sol re-review 3: PASS. Counts: Sol P2 ×16, P3 ×5; mechanisms: none (test tooling). Not verified before the bench: the live ssh/server chain of the drop test, the L1 held path, P4's same-Run successor.

Process notes: resuming an idle OpenCode session with `-s` hung (18:07); follow-up rounds ran as fresh service runs.
