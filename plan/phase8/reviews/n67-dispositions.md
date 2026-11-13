# #67 — review record and dispositions (2026-11-13)

Issue #67: a receive segment begun well ahead of its origin has its timed `rx_start` queued in the device. When `Provider::stop`'s End (or a Stop) cut it at its origin, uhd-rx waited for the origin before stopping (VH-6, written while #56's hardware answer was unknown). So with an origin more than 1 s away, `UhdRadio::stop` left uhd-rx detached and cleanup skipped `close_streams`, which leaked the streamers. The bench of 2026-11-13 (`bench/2026-11-13-n67-records`, both boards, 1/10/100 MS/s × 10) showed that an untimed `rx_stop` before the origin cancels the queued start, and that a close and reopen does not.

The change:
1. **Stop at once.** A cut at or before the first undelivered sample stops untimed and ends the stream at once, origin passed or not. One rule for `Provider::stop` and an ordinary Stop: the `done` conjunct in `rx.rs`.
2. **Stop on open.** Every receive streamer the Module opens gets one untimed `rx_stop` before its first `rx_start` (UR-16). A start orphaned by any earlier close (a failed stop, a failed owner spawn, cleanup, a killed process) is then cancelled. A failure refuses the segment (`UR-16: the untimed stop on open: …`). A lost result publishes the loss (UR-29), at `arm` too.
3. **FakeDevice.** Per-channel orphaned queued starts survive a close and reopen; an untimed stop clears them; time zero, a spec change or a passed origin drops them.
4. **The differential.** The scripted device and the compare allow an early stop only for a segment the plan took away.
5. **Hardware test.** `hw_67_stop_before_origin` has four cells (`Control`, `StopBefore`, `CloseBefore` FAIL expected, `CloseThenStop`), and a FakeDevice rehearsal books the same table.

Risk: risky (a lifecycle, and the boundary between a fake and the hardware). Branch `work/n67` from `79f55e1`: sub-steps A, B, C and two fix rounds, squashed and rebased. Implementer: OpenCode Sonnet 5.5 #high for the plan and A–C; Claude Code Sonnet subagents for the fix rounds (owner, 2026-11-13 evening). Records: [n67-briefs.md](n67-briefs.md), test plan [n67-test-plan.md](n67-test-plan.md).

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan | 1 | 3 + audit correction | `stream-lifecycle-scattered` (M: a failed cleanup stop still closes a queued start) |
| Sol, code | 1 | 3 | `failure-collapsed-into-value` (M1: a lost stop on open at `arm` lost its `lost` flag in `open_refusal`) |
| Opus, re-check of fix round 1 | 0 | 2 + nits | — (the transmit open routed through `refuse_open` beyond scope; no test for it) |

## Dispositions

**Sol, test plan (FAIL).** All adopted.
- The major is answered by a different rule, which replaces the plan's D3 (cleanup stops an Uncertain chain): **stop on open**. Reason: a stop that fails cannot be repaired at cleanup, and a process death never reaches cleanup; the next open is the one place that sees every case. That a stop on a reopened streamer cancels the orphan is INFERRED; `hw_67`'s `CloseThenStop` cell measures it on 2026-11-15.
- Minors: the C1 fixture uses a start seconds ahead; orphans are kept per channel; the rehearsal is budgeted at real time; the `hw_56` audit row corrected.

**Sol, code (FAIL).** All adopted, fix round 1.
- M1: `refuse_open` publishes the loss before the `arm` refusal.
- m2: the stash includes a start queued behind an unread tail.
- m3: `calls()` drops only the stop that directly follows its own stream's open.
- m4: UR-17's sentence is qualified.
- Rows N67F-27…30.

**Opus, re-check of fix round 1.** M1 is closed for receive; m2–m4 are closed. Fix round 2 is minor-only, with no re-review.
- The transmit open at `arm` goes back to the plain `UR-13` refusal: `refuse_open` serves the receive open only. A `UHD_ERROR_RUNTIME` from `get_tx_stream` counts as `lost`, so routing it would make a failed transmit open a `DEVICE_LOST`, which is user-visible.
- The remaining lost-flag drops are filed as #101 (`mech:failure-collapsed-into-value`): the rates check, `prepare`'s configure, and the transmit open at `arm`.
- The nits are fixed.

## Decided by the orchestrator

- One rule for `Provider::stop` and an ordinary Stop (D1). No MockRadio or timeline change (D2): the user-visible result is identical on both Providers.
- Stop on open replaces D3. Cleanup adds no stop.
- The refusal text `UR-16: the untimed stop on open: …` is a new cause on an existing refusal path. It is listed for the owner at landing.
- The `applied`/`timing` rows of such a stop can carry an instant earlier than the cut (D4). The fields keep their meaning, so this is not an owner question; it is listed at landing.
- Row ids are `N67F-nn` (`N67-nn` belongs to the probe step). G60 is retargeted. P880 keeps `"kill": "bound"`. The gate passes `renames.txt` for the three renamed cited tests.

## Not verified on hardware

The following run on 2026-11-15 (`hw_67`, every receive bench):
- `CloseThenStop` PASS;
- a stop on a fresh streamer being harmless;
- the per-channel keying of orphans;
- a close and reopen across an origin or a rate change.

## Gate

PASS ([n67-gate.json](n67-gate.json)) on sim02:8 and sim03:8: 2775 tests, 686 UHD, `hw-build`, Python 121 × 2, e2e 16/16, mutation 2672/2673 (the known survivor `phase7/G09`), no `ERROR`, no timing re-runs.
