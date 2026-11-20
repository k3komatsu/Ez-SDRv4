# #117 — dispositions

Three receive-path device calls entered UHD without the UR-29 ordering after `DEVICE_LOST` (issue #117, `phase:8b`, done in the idle time before the 2026-11-27 bench). Risk: not risky (an existing, reviewed mechanism applied to three more call sites). A Sonnet subagent implemented; GPT-6.1 Terra and an Opus re-check reviewer reviewed side by side, Terra re-checked ([n117-reviews.md](n117-reviews.md)). Briefs: [n117-brief.md](n117-brief.md). Gate: `n117-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Terra, review | 2 | 1 | 0 | 1 `stream-lifecycle-scattered` (the site-2 panic); others none |
| Opus, review | 1 | 2 | 1 | the same site-2 panic (counted once); others none |
| Terra, re-check | 1 | 2 | 0 | none |

## Dispositions (all adopted, decided by the orchestrator)
- Site 2's "lost, no call" path finished an `Armed` chain without the `Lost` transition and panicked uhd-rx (both reviewers): it now takes `chain.lose()` first, as the ordinary stop does. The implementer's first claim that the path needed a production hook to test was wrong (Terra): the test exhausts clock ids to make `declare_sample_clock` fail, and fails with the panic on the unfixed code.
- The new rows removed the loss injection together with the admitted call (Terra): each row's replacement now runs the site's probe first, so the mutant meets the loss and the test fails on its post-loss-call assertion; the site-3 test checks the call log before the refusal (Terra's re-check).
- Site-3 test: two RX streams; stream 0's owner is seen running before the loss and making no turn after the refused start returns.
- Site-1 test asserts the chain ends `Lost` (Opus).
- `design/18`: the six admitted receive calls are listed once, in UR-29, with what a loss found at each does; UR-17 cites it; the new tests are in the *Test:* lines.
- The re-check's major (an assertion order) and minors were fixed without another review; the orchestrator read the reordered test (`provider/mod.rs`: the call-log assertions precede `unwrap_err`).
- Gate run 1 (orchestrator): rows P477 and P1291 survived, since the admitted calls re-check the loss after their early checks. Both early checks stay. P477's still has its own effect — a stream with no first segment never reaches an admitted call, so without the check its owner is spawned after the loss and `start` succeeds — and a new test (`ur_15_a_loss_published_by_an_owner_stops_a_next_stream_without_a_first_segment`) now kills the row. P1291's only remaining effect is that `begin` does not reach the segment start's admission at all; its test asserts that the site's probe is never passed after the loss (a test-hook observation, accepted as the cheapest one that tells the paths apart).
