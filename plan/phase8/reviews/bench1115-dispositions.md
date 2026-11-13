# 2026-11-15 bench probes — review record and dispositions (2026-11-13)

Three bench probes for the 2026-11-15 X310 session (crates/ezsdr-radio-uhd/tests/common/bench1115.rs, `hardware.rs`, FakeDevice rehearsals in `fake.rs`, plan/phase8/bench.md's 2026-11-15 section): `hw_rxzeros_sweep` (receive-restart leading zeros by DDC half-band class, levels and start kinds, for the owner's direction D: the Mock and Fake fill as a bound by class), `hw_75_t3_probes` ((a) skip the recovery flush T3 and compare the next burst against interleaved controls, (b) whether an old burst's ACK reaches a new streamer, (c) whether a T3 ACK read 150 ms late is still there; for #75), `hw_75_open_flush_sweep` (the open-time flush at 1, 2, 10 MS/s on one- and two-channel streamers, for #75's (3), which the owner gated on this sweep). Each probe prints only what it observed; a decision line recommends a change only on the complete planned evidence and otherwise prints INCONCLUSIVE naming what is missing. Transmit only at 1 GHz on slot A (an `EZSDR_D9_CHANNEL` other than 0 aborts before any device call); gain ≤ 20 dB, back to 0 on return and unwind; slot B transmits only with `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1` set for one command after the owner confirms slot B's attenuator; one whole-life bound per probe. Risk: not risky (measurement tooling). Branch `work/bench1115` from `89fd3f7`, one squashed commit. Implementer: OpenCode Sonnet 5.5 #high ([bench1115-briefs.md](bench1115-briefs.md)).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, code | 7 | 1 | — (decision lines recommending a change on incomplete or ambiguous evidence; three transmitting probes behind one confirmation) |
| Opus, re-check 1 | 1 | 3 + 3 notes | — (an overflow read as a T3 effect) |
| Opus, re-check 2 | 0 | 4 + 1 note | — |

## Dispositions

**Sol, code (FAIL)** — all adopted, fix round 1, on one principle: a probe states what it observed and recommends a change only on the complete planned evidence ([bench1115-sol-code.md](bench1115-sol-code.md)).

**Opus, re-check 1** — the major (receive outcomes and contiguity recorded; an overflow or gap makes a trial INCONCLUSIVE) and the minors adopted in fix round 2; the notes fixed too (old ACKs reported apart; the grep; slot B behind a named confirmation, decided by the orchestrator for RF safety).

**Opus, re-check 2** — all adopted in fix round 3 (a capture must span B's window; the recording side pinned with `FakeFault::Overflow`; RXZ checks completeness before an off-tick FAIL; dirty controls ignored; slot A only) ([bench1115-recheck.md](bench1115-recheck.md)).

## Decided by the orchestrator

- The slot B confirmation variable and the slot-A-only assertion (RF safety).
- Not risky; no test plan; minor-only fix round 3 without a further re-check.
- Open for the owner before the bench: whether a two-channel streamer can be wired (board and slot B attenuator), and the tone at up to +20 dB transmit gain (the owner's 2026-10-27 allowance of transmit gain up to the maximum with the attenuator in the loopback).

## Gate

PASS ([bench1115-gate.json](bench1115-gate.json)) on sim02:8 and sim03:8: 2664 tests, 634 UHD, `hw-build` (the probes compiled with `--features uhd`), Python 121 × 2, e2e 14/14, mutation 2514/2515 (the known survivor `phase7/G09`), `killed (bound)` A1-1, A1-2, E2A2A-10, J317, J318, N51-2A-21, N51A-14, N51A-15, N51B-08, N51C-06, P880, `phase7/G16`, `phase7/R15`, no `ERROR`, no timing re-runs.
