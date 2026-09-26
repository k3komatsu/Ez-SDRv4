# Phase 3 — Review C: findings, verdicts and the open items for Gate X

| Field | Value |
|---|---|
| What | Review C of `20-implementation-plan.md`: one adversarial review (Opus, AGENTS.md §8) of the applied series against specs 11 and 12, then sticky re-reviews of each fix delta |
| Reviewed | `5eb61dd` (Steps 0–6), then the fix deltas that became `857a1c2` |
| Reviewer's limits | No shell in any pass: every claim is from reading; the implementer ran the tests, mutations and probes the reviewer asked for |
| Raw results | `tmp/luna-primary-engineer/reviews/20260926-review-c-phase3-state/` (`attempt-1/`, `rereview-1/` … `rereview-3/`; `tmp/` is gitignored, so this file is the record) |
| Fix record | [`../implementation-notes.md`](../implementation-notes.md), `## Fixes after Review C` |
| Verdicts | [`../00-overview.md`](../00-overview.md) §11 |

## 1. Passes

| Pass | Reviewed | Verdict | Result |
|---|---|---|---|
| 1 | `5eb61dd` | `PASS_WITH_RISK` | no P0; P1-1, P1-2; five P2 test gaps; P2-1 … P2-9 |
| 2 | the eleven test fixes (592 → 600) | **`CHANGES_REQUIRED`** | P1-1, P1-2 and four test gaps closed; new **P0 B1**, exposed by the P2-1 probe |
| 3 | B1's fix (600 → 602) | `PASS_WITH_RISK` | B1 closed; two P2 text inconsistencies from VB-9's partial update |
| 4 | the text fixes and the P2-1 measurement | `PASS_WITH_RISK` | nothing left the implementer can close; P2-1 measured; P2-2 … P2-9 the owner's |

The loop stopped after pass 4 because every remaining item was an owner decision.

## 2. Closed findings

| ID | Finding | Closed by |
|---|---|---|
| B1 (P0) | In a Run with two MockRadios the Manifest kept only one Mock's `ezsdr.radio.mock.*` sections: `Manifest::write_section` inserts, so the last writer won silently (KC-45, RS-39) | VB-9 (owner-approved): MR-27's section names carry the instance id, MR-2 refuses an id that is not a section-name segment; `v58_08_b`, `mr_03`; Kernel unchanged |
| B1, consequence | `v58_12`'s jitter guard compared the transmitter's `stats` with the receiver's, so it could not fail | the guard reads the receiver's `rx_samples` |
| P1-1 | CH-4: two couplings with the same ends, and the document-order `f64` sum, were untested | `ch_04_b`, `ch_04_c` |
| P1-2 | MR-16/MR-32: the claimed carrier could not observe where the waveform refusals sit | `mr_32_a_the_waveform_refusals_hold_their_place_in_handle_tx_burst` |
| Test gaps | MR-32 phase and frequency at the sample's own instant; MR-34 on every transmit channel; MR-17 `now_up` in channel mode; CH-6 "a refusal changes nothing"; MR-35 the receive gain on the noise | `mr_32_b`, `mr_32_c`, `mr_34_a_a`, `mr_17_a_a`, `ch_06`'s last case, `mr_35_the_receive_gain_scales_the_noise_with_the_signal` |
| P2 nits | comments and wording in `ch_04_c`, `mr_32_b`, `mr_32_c`, `mr_34_a_a`, `v58_12`; stale section names and VB-9 status rows in `design/09` and spec 12 | the fix deltas |

## 3. Open at Gate X: P2-1 … P2-9, with recommendations

The reviewer's text is quoted. Each recommendation was checked against the code on
`3cbc103`. It is Claude's recommendation (Opus). **The owner accepted all nine as
recommended at Gate X (2026-09-26)**; they are applied as KB-1's KC-9 amendment
(P2-2) and spec 12 VB-10 (the rest), with `kb_01_b_the_store_keeps_no_unverified_entry`
and `mr_25_a_stream_stop_keeps_the_pending_commands`, each shown to fail under the
mutation that undoes it.

| ID | Finding (reviewer) | Checked | Recommendation |
|---|---|---|---|
| P2-1 | `step_until_quiescent` returns at the first `?`, so a Module error leaves every instance later in `(step_rank, fragment id)` order unstepped in that round; which blocks a receiver published before the abort depends on fragment names. CH-9's precondition excludes this only implicitly | **VERIFIED by measurement** (implementation notes, "P2-1, measured after B1"): receiver `rx_blocks` 0 when the faulted transmitter sorts first, 1 when the receiver does | **Ceiling.** Add to CH-9 and MR-30: "a round that a Module error ends early does not step the remaining instances, so which blocks a receiver published before the abort can depend on fragment order; the published values do not", and note it on Z10. Reproducibility (§58 #3) still holds: the same Spec with the same names reproduces bit for bit; only renaming fragments changes an error-terminated Run. *Rejected:* finishing the round before propagating the error — a coordinator behaviour change whose semantics (what else runs after a failure) belong to Phase 4's failure work |
| P2-2 | MA-5a says the store holds "every input KC-9 verified and every waveform KC-28 ingested"; it holds every `Assembly.inputs` entry, unverified, and KC-28's `or_insert_with` keeps an existing mis-keyed entry, so a Provider could transmit bytes the Manifest's hash does not describe | **VERIFIED by reading**: `pipeline.rs` builds the store from all of `assembly.inputs`; `check_inputs` verifies only the entries a schedule `TxBurst` references; KC-28 uses `entry(hash).or_insert_with` | **Fix in Phase 3.** KB-1 made the store Provider-readable, so the hole is Phase 3's. After KC-9 passes, keep only the entries a schedule entry references, so every entry is verified and recorded in `Manifest.inputs`, and MA-5a's sentence becomes true. One test: an Assembly holding a mis-keyed, unreferenced entry under hash `H`, then a Session waveform whose hash is `H` — the Provider must read the Session's bytes. A mutation back to the old store must fail it. *Rejected:* KC-28 overwriting only — it leaves unverified bytes readable by hash and absent from the Manifest |
| P2-3 | A cold `radio.tx.channels` change writes `config` before `stop_tx`, so a held burst opened inside `stop_tx` gets a header whose `channels` disagrees with its waveform; no effect today | **VERIFIED by reading**: `apply_update` inserts into `config` before the cold branch calls `stop_tx`; the header reaches only `BurstTracker`, which does not check `channels` | **Ceiling, no code change.** Nothing can observe the header today, so a fix would ship untested — the Phase 1 lesson against unkilled code. Record it under MR-18 as a ceiling for the first consumer of transmit headers (a real TX port), which must fix and test it |
| P2-4 | RM-16 and MR-25 put "cancel … the pending commands" in the transmit half; a `Stop` for `<device>/tx` cancels none, only `Provider::stop` does | **VERIFIED by reading**: `Stop(<device>/tx)` calls `stop_tx`; only `stop()` clears `updates` | **Fix the text, not the code.** Pending timed commands include receive keys, so a transmit-stream stop must not cancel them. Amend RM-16 and MR-25 (spec 12, a new VB-10, text only): the pending commands are cancelled by `Provider::stop` and `Stop(<device>)`, not by a stream `Stop`. Pin it with one assertion that a pending receive update still applies after `Stop(<device>/tx)` |
| P2-5 | `end_open_segment(next)` on MR-15's unreachable emission-refusal branches would cut radiation at a sample that may be past, breaking CH-9; a `debug_assert` would expose it | **VERIFIED by reading** (the two call sites); reachability stays INFERRED, as VB-8 records | **No code change; extend the recorded ceiling** with "reaching it would break CH-9". A `debug_assert` in a branch no test reaches is code no mutation can kill |
| P2-6 | `schemas/sim/channel.v1.json` carries none of CH-1's ranges; a non-Rust validator accepts what the reader refuses (as `FaultEntry`) | **VERIFIED**: no `minimum`/`maximum` beyond the integer types', but each description states the range | **Reject for Phase 3.** Phase 1's D95 put grammar in the description and no `pattern` in schemas; `FaultEntry` follows the same rule. Machine-checkable ranges are a schema-policy change for every Vocabulary at once, not for one file |
| P2-7 | M13: srsRAN's evidence supports only `n_tx + n_rx = 45`; the 45/0 split is as invented as the rejected "invented split" | **VERIFIED**: M13's rejected option is "an invented transmit/receive split" | **Fix the text.** Chosen: "follow srsRAN, which applies the whole 45 samples as a transmit advance; the evidence gives only the sum". Rejected: "any other split, for which no source exists". Change spec 12 M13 and `design/09` §5's M13 row |
| P2-8 | Stale package descriptions in `ezsdr-sim` and `ezsdr-mock-radio` | **VERIFIED, and wider**: eight crate descriptions cite `plan/phase2/07…10-*.md`, which Phase 2's Step X moved to `design/`; `radio`, `sim` and `ezsdr.radio.mock` are 1.1.0 but say 1.0.0; `ezsdr-acceptance` says "Phase 2 acceptance tests" | **Fix now**: point each description at `design/…`, state the current versions, and drop "Phase 2" from the acceptance crate's. Text only; `Cargo.lock` does not change |
| P2-9 | Per sample: two Medium locks, one `Vec` of `Arc` clones, two `powf` | Not measured again; C11 records the cost | **No action.** C11 accepts the cost (INFERRED timings), and Phase 3 has no real-time requirement on the simulation path. Revisit with RealtimeEmulation |

## 4. The reviewer's reading of the decisions

- Z1–Z11 supported; Z10 holds except for P2-1's error-terminated round.
- C1–C11 supported; C11's timings are INFERRED.
- M11, M12, M14, M15 supported (M15's conflict with Vision §13 and §16 is Vision issue 4); **M13 partly supported** (P2-7).
