# #101 — review record and dispositions (2026-11-14)

A lost device found at `prepare` or `arm` is published (`DEVICE_LOST`, `mark_lost` before any close) instead of being reduced to a refusal: the rates check, `prepare`'s configure, the transmit open at `arm` and the `ref_locked` read at `arm` go through one conversion, `Core::publish_loss`, which `device_failed` and `start` use too. The transmit owner's reopen publishes a loss even with a `Provider::stop` booked. A lost rates read is not cached (`rates_checked` stays false). At `uhd_usrp_get_{rx,tx}_stream` only, `UHD_ERROR_EXCEPT` (47) also counts as lost.

Risky (a lifecycle, the device's loss, and the classification at the fake/hardware boundary): test plan, Sol's plan review, implementation, Sol's code review. Branch `work/n101` from `89dc141`, squashed. Implementer: Claude Code Sonnet 5.5 subagent, effort high ([n101-briefs.md](n101-briefs.md)). Test plan: [n101-test-plan.md](n101-test-plan.md) (v2).

## Owner decisions

- 2026-11-13 (on the issue): the rates check and `prepare`'s configure publish the loss; the transmit open at `arm` first needs the UHD source checked for when `get_*_stream` returns `UHD_ERROR_RUNTIME`.
- 2026-11-14, after the source check: RUNTIME stays lost at the stream opens (no narrowing); 47 also counts as lost there.
- 2026-11-14: `rates_checked` is false for a direction whose rates read was lost (Manifest content).
- 2026-11-14: KL-2 names a loss during `connect` (`Failed { prepare | arm }`, one `DEVICE_LOST`), and its Status names the stream opens as checked in the UHD source.

## Evidence (UHD 4.10.0.0, VERIFIED in the raw source unless marked)

- `error_c.cpp:20-35` maps exceptions by `dynamic_cast` in a fixed order; anything outside the listed classes is `UHD_ERROR_EXCEPT` (47). `rfnoc_error` and its `op_timeout`, `resolve_error` and `routing_error` derive from `uhd::exception` directly (`exception.hpp:229-290`), so they are 47. `usrp_c.cpp:257-288` wraps both stream opens in `UHD_SAFE_C_SAVE_ERROR`.
- At the stream opens, RUNTIME (44) is thrown for refusals ("Graph edge list is empty", `multi_usrp_rfnoc.cpp:361`, `:472`; "Invalid channel", `rfnoc_rx_streamer.cpp:156`, `rfnoc_tx_streamer.cpp:124`; "Graph topology is not valid", `graph.cpp:974`) and for a dead link's transport timeouts (`chdr_rx_data_xport.cpp:203`, `chdr_tx_data_xport.cpp:171`, `:193`). 47 is thrown for refusals (reconnecting a held port, `graph.cpp:184`, `:197`; `graph.cpp:557`) and for a dead link (`ctrlport_endpoint.cpp:646`; `mgmt_portal.cpp:984`).
- The Module's stream arguments are fixed (`fc32`, `sc16`, channels read back at bind), so the RUNTIME refusals cannot be reached (VERIFIED by reading the Module). The reconnect refusal is designed out, and on the bench it ended the process anyway (`plan/phase8/bench-results.md:73`). A dead link's device-time read returned 47 and 30 on the bench (`plan/phase7/bench-results.md:1446`, `:1535`). The message text cannot separate the cases (an empty `last_error` with 47, `bench-results.md:73`).
- uhd-control starts in `start`, after `arm`, so UR-29's time-read rule cannot find a loss at `prepare` or `arm`.

## Findings by source and severity

| Source | Major | Minor | Mechanism |
|---|---|---|---|
| Sol, test plan v1 ([n101-sol-plan.md](n101-sol-plan.md)) | 1 | 1 | — (M1: T1's fault fired at the rig's setup, so the recovery open was never reached; m: a fallback that dropped the Manifest event assertion): plan defects, no code yet |
| Sol, code ([n101-sol-code.md](n101-sol-code.md)) | 0 | 0 | — |

## Dispositions (decided by the orchestrator)

- **Sol M1:** an nth-call fault (`FakeFault::LostOnCall(call, nth)`); T1 asserts that the setup succeeded and the recovery `tx_open` was reached. Q9 adopted: a booked `Provider::stop` no longer suppresses the loss at the transmit reopen.
- **Sol minor:** every coordinator-level row asserts exactly one delivered `DEVICE_LOST` in the Manifest, and `rates_checked` is asserted in the Manifest.
- **Plan questions:** one `publish_loss` with three callers (Q2); the 47 rule matches the call name inside `check_with`, with a `shortcut:` comment (Q4); the fake's reconnect refusals stay non-lost and `Unreachable` does not touch the opens (Q6); publication stays with the caller (Q8).
- **Bind-time sites** (the serial read, the subdev spec, `front_end`, the authority's source and time calls, `uhd_usrp_make`) stay refusals: no `Core` exists yet to publish a loss.
- **Feature `uhd`:** the classification tests (K1, K2) run only with libuhd, outside the mutation gate; K2 was shown red without the 47 clause and green with it.

## Mechanism

`mech:failure-collapsed-into-value`, the ninth bug ([failure-mechanisms.md](../../maintenance/failure-mechanisms.md)). The design note's answer is #75's, applied to the remaining paths: a failure at the Device boundary keeps its type until the one function that decides what it means (`publish_loss`, then the refusal), and no outcome is cached in a type that cannot carry `lost`.
