# Stage 4a — dispositions of every finding

Step: the profile values from the 2026-11-16 bench (command lead 5 ms on both Providers, device lead 2 ms, receive start lead 10 ms, 8e8 B/s per 10 GbE link), the receive start head (2/13/18/24 zeros at every receive start on MockRadio and FakeDevice), #75 (3) (the open flush confirmed or refused at `arm`), and the bench probe's row fix with a receive-only start-lead bench test. Risky (deadlines, a refusal at `arm`, the boundary between the fakes and the hardware). Decisions: [s4a-decisions.md](s4a-decisions.md) (owner 2026-11-13 D0–D13 in [stage4-owner-decisions.md](stage4-owner-decisions.md), owner 2026-11-17 1-a, 1-a(b), 1-b, 1-c, Q1 option B). No step-0 design run: the values and the head were decided from the bench with the owner ([stage4-proposal.md](stage4-proposal.md)). Test plan: [s4a-test-plan.md](s4a-test-plan.md) (Sonnet; v2 after Sol's review [s4a-sol-plan.md](s4a-sol-plan.md); deviations in its §12–§14). Implementer: Claude Code Sonnet 5.5 subagent (effort high), sub-steps A, B, C, D, each reviewed by a fresh Opus subagent (effort high); narrow Opus re-checks after each major fix; code reviewed by Sol ([s4a-sol-code.md](s4a-sol-code.md)). Briefs: [s4a-briefs.md](s4a-briefs.md).

| Source | Finding | Severity | Disposition |
|---|---|---|---|
| Sol plan review | 1: the plan still placed a late SendAsap at 5 ms on the Mock | major | owner Q1: option B throughout (admit at 5 ms, place at receipt + 2 ms on both), RM-14 amended |
| Sol plan review | 2: no head after an overflow restart | major | owner Q2: a head after every overrun restart on Mock and Fake, INFERRED on hardware (KL-47) |
| Sol plan review | 3: the Fake's head only on the loopback source | major | adopted: the head is applied outside `sample_at`, every source |
| Sol plan review | 4–8 (time-varying fixture, split heads, unread tail, S4A-12/35/64, held clock, rehearsal through the verdict) | minor | adopted (decided by the orchestrator) |
| Opus review A | 1: the #108 test exemption too broad, its comment unexplained | major | narrowed (Overrun only, the cut in the gap, that segment only, seeds asserted); [#108](https://github.com/k3komatsu/Ez-SDRv4/issues/108) filed for stage 4b |
| Opus review A | 2: a late receive start after an overflow with the 10 ms lead | major | brought to the owner; KL-7 "Also", fix in stage 4b (#108) |
| Opus review A | 3–7 (RM-14 vs MA-10, 1 MS/s evidence overstated, INFERRED above 10 MS/s, UR-25 pointer, A6 cases) | minor | fixed in A (MA-10's own sentence fixed after Sol's code review) |
| Opus review A | 8–10 (rs_19 instant, spec 16 Changes row, leftovers) | note | fixed or left (pre-existing) |
| Opus re-check of the exemption (60dce09f) | F1: the late segment is `GAP_BEFORE \| SEQ_DISCONTINUITY`, not `RESTARTED` | major | confirmed by running seeds 243/766/875; comment, KL-7, UR-25 (pointer only) fixed |
| Opus re-check (60dce09f) | F2: UR-25 stated the rig's behaviour as a rule | major | UR-25 keeps only a pointer to KL-7 |
| Opus re-check (60dce09f) | minors (KL-7 "can", USRP outcome, tie to the previous segment, structural check) | minor | fixed (`gap_excuses`, `excused_blocks`) |
| Opus review B | 1–6 (head across blocks on the Mock, no head after a sequence error unpinned, KL-47 rate wording, bound check twice, leftovers, duplicate case) | minor | fixed |
| Opus review B | 7–11 | note | accepted; head after noise confirmed by the orchestrator |
| Opus re-check (f9fd145c) | 1: the excused segment's start not pinned (a 3 ms guard delay passed 1 000 seeds) | major | fixed: pinned to the stop + device-lead guard and the plan's blocks; row S4A-64 |
| Opus re-check (f9fd145c) | 3: KL-7's workaround cannot be followed (no `RX_OVERFLOW` reaches the user in this sequence) | major | owner 2026-11-18: KL-7 says no `RX_OVERFLOW` arrives, how a capture detects the case, and the workaround `block_len` 65 536 |
| Opus re-check (f9fd145c) | 2, 4, 6 (prefix tolerance, stated reason, wiring unpinned) | minor | fixed (reason, forged-Observed test); 2 is the existing per-segment tolerance |
| Opus review C | 1–5 (recovery text, rows outside the run, KC-30 prefix, two spec sentences, stale comment) | minor | fixed; a Changes row edit reverted |
| Opus review C | notes (test tools unweakened, arm budget, Python surface) | note | accepted; no Python test (the generic `RunEnded` path is pinned) |
| Opus re-check (11b3ed6e) | 1: the guard pinned only to the block | minor | fixed: direct uhd-rx tests at the tick, rows S4A-75..77 |
| Opus re-check (11b3ed6e) | 2: a zero guard inverts the check; 3: two stops do not set `last_stop` | note | 2 fixed (S4A-78); 3 a false failure only, not reached, left |
| Opus review D | 1–4 (`restart_trial` drop check unpinned, a wrong comment, bench.md not runnable, rehearsal margin by a literal) | minor | fixed |
| Opus review D | 5: a dropped row discards the whole pass | minor | rejected by the orchestrator: reachable only after about 1.3 s at 100 MS/s |
| Sol code review | 1: MA-10 still said RM-14 uses the Kernel allowance | minor | fixed by the orchestrator (MA-10 sentence and Changes row) |
| Sol code review | 2: KL-47's workaround (start the source first) does not avoid the head | minor | fixed by the orchestrator (start the receive a head earlier) |

Counts: majors 10 (Sol plan 3, Opus A 2, Opus re-checks 4, owner decisions included); minors and notes as listed. Mechanisms (`plan/maintenance/failure-mechanisms.md`): none counted — the majors are a stale plan branch, a test exemption whose premise was read rather than run, and test power; the late receive start after an overflow is an old path the lower lead exposed, tracked as #108 for stage 4b.

Not measured: the head after an overrun restart on the USRP; the 10 ms receive start lead above 10 MS/s (the bench rerun with `hw_s5_rx_start_lead`); the open flush's three attempts on hardware.

Gate (2026-11-18): every step passed ([s4a-gate.json](s4a-gate.json)); tests 3025 + UHD 819 passed, Python 121 on 3.14 and 3.9, mutation 3051/3052 (the known survivor phase7/G09).
