Independent scenarios, before reading the test plan:

- A rate read returns `lost=true` after a successful open. Loss must be published before `open_streamer` closes anything—not merely when `arm` receives the error.
- Prepare/configure, external-reference checking, and RX/TX arm opens each distinguish loss from ordinary refusal. Failure must prevent subsequent channel/stream calls.
- A lost rate read must neither lose its type in the cache nor allow a later device call. Ordinary cached mismatches must remain refusals; successful comparisons remain reusable.
- An RX-linked Run and a TX-only Run must both publish exactly one loss, mark the device, and avoid teardown calls, including after repeated cleanup.
- Mid-run owner opens/configuration must retain their existing loss behavior when the conversion is shared.
- Prepare-time loss must survive the failed-prepare lifecycle into the actual Manifest/event log.
- Error 47 must become lost at both native `get_*_stream` calls, but not at receive/send/async or unrelated control calls; existing error classifications must remain intact.
- Fault injection must reach the intended call: loss before rates is not a rates-read test, and internal-reference arm cannot exercise `ref_locked`. Native tests must exercise call-site wiring, not just a classifier helper.
## Verdict: FAIL

Core coverage is strong, but the owner-reopen test cannot reach its intended failure as written.

Plan references below mean `/home/komatsu/.cache/ezsdr-fixes/2026-11-14-n101/test-plan.md`.

### Findings

1. **Major — T1 trips during setup, not recovery** — **VERIFIED**
   - Plan: `test-plan.md:68,115`. The proposed fault fails the **first** `tx_open`, but `owner_with` opens TX during setup (`crates/ezsdr-radio-uhd/src/provider/tx.rs:6277–6282`).
   - Minimal intended sequence: establish TX → claim recovery → book Stop → recovery `tx_open` returns lost → cleanup.
   - Wrong outcome currently reachable: the Stop suppresses loss publication (`tx.rs:855–858`), allowing cleanup to call `close_streams` on the lost device. The proposed test instead fails at setup’s `unwrap`, before exercising this sequence.
   - **Fix:** use a fault armed after setup, an nth-open fault, or a scripted spy. Assert setup succeeded and the recovery open was reached. Retain the non-lost twin. Adopt Q9 rather than leaving this teardown hole optional.

2. **Minor — Do not waive the Manifest event assertion** — **VERIFIED static path; runtime untested**
   - Plan: `test-plan.md:134,224` permits dropping the event column if the baseline contains no event.
   - The coordinator explicitly drains events during cleanup (`crates/ezsdr-kernel/src/coordinator/ending.rs:263,414–419`) and puts delivered events into the Manifest (`ending.rs:144–147`). This is not merely an unknown output format.
   - **Fix:** require exactly one delivered `DEVICE_LOST` for every row. A missing baseline event should trigger investigation, not weaken the oracle. Also assert the actual Manifest’s `rates_checked` value for Q3; A4 currently checks only Core state.

### §9 questions

- **Q1:** F1 is reasonable for prepare/arm, but insufficient for T1 without delayed/nth-call injection.
- **Q2:** Approve shared `publish_loss` plus `refuse`; existing owner/start tests appropriately protect rewiring.
- **Q3:** Recommend **false**, pending owner confirmation. Assert it in the Manifest.
- **Q4:** Approve the call-name match, protected by K1’s negative streaming cases and K2’s real call-site wiring.
- **Q5:** Reject the event-assertion fallback; see finding 2.
- **Q6:** Accept the explicitly documented fake divergences for this step; K1/K2 must remain mandatory.
- **Q7:** Decide KL-2 wording from the fixed, explicit user-visible result—not a weakened baseline.
- **Q8:** Agree publication remains with the caller, after owner refusal bookkeeping.
- **Q9:** Adopt the loss-preserving condition and repair T1 as above.

No files changed, formatting performed, tests run, or mutation tools invoked. Hardware outcomes and proposed mutation killers remain unverified.
