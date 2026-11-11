My independent failure scenarios, before consulting the plans:

- **Framing:** a dropped first payload must not open or close a device burst; a short/failed send must close exactly once. A held tail must not be replayed after failure.
- **Receipt races:** insert a switch or Stop between the initial unread check, send planning, device call, and completion. The already-issued send may finish, but later sends must observe the receipt and respect its boundary.
- **One send in flight:** a receipt must not remove or replace the open booking before completion; short, failed, and tail-short outcomes must use the right cursor.
- **Scoped abandon:** abandon the old repeat after Stop and a later booking, including after a switch was applied in device time. Preserve the later reservation, busy end (`until`), and playing identity; distinguish the old footprint from the Stop region.
- **Cancellation:** halt an open booking before its first issue. Cancel a switch with its payload unread, held, or open, with exactly one cancellation and no later transmission.
- **Switch chains:** accept a switch, cross its boundary without issuing its payload, then request another. Check Provider-specific started rules, refusal side effects, timed-pushed-only `requested_target`, and the repeat grid after dispatch moves.
- **Reports:** deliver an old underflow after opening a new device burst; compare its tick with that burst’s first tick, not its cursor. Equality, no tick, and sequence errors must retain their distinct behavior.
- **External promises:** compare Mock/UHD outcomes, deferred refusal delivery outside the lock, EA-16/docstring/KL-41/KL-42, and hardware-test expectations with the same fake-device boundaries.
The send/receipt ordering and cancellation paths match the amended plan so far; I have not found a major counterexample. Two points need attention: KL-41 says the refused switch’s predecessor “plays on,” but the #84 test deliberately drops that predecessor; and both Providers recompute the switch boundary instead of using `SwitchAccepted.boundary`, leaving the booking and record vulnerable to diverging.
## Verdict: PASS WITH MINORS

E2a centralizes transmit booking/results and decides switches at receipt. **No major counterexample found; no settled owner decision is refuted.**

### Minor findings

1. **KL-41 promises playback that may never happen — VERIFIED.**  
   `KNOWN_LIMITATIONS.md:170–172` says the predecessor “plays on” and recommends retrying once it sounds. But the #84 regression explicitly drops that repeat at dispatch and verifies neither waveform sounds (`crates/ezsdr-radio-uhd/src/provider/tx/tests/o1.rs:898–915`).  
   **Fix:** say refusal leaves the predecessor unchanged; playback still depends on its late policy. A dropped repeat needs restarting.

2. **KL-42 confuses the reservation bound with the payload cut — VERIFIED.**  
   `KNOWN_LIMITATIONS.md:177` says Mock transmits `[b, B)`. Mock computes a separate payload cut (`crates/ezsdr-mock-radio/src/lib.rs:1167–1180`). Its corresponding test expects `[b, b+600)`, not through the five-sample-later bound (`crates/ezsdr-mock-radio/tests/mock_channel/o1.rs:428–439`).  
   **Fix:** describe transmission through the MR-25 payload cut, subject to its first-send rule.

3. **The accepted boundary is computed twice and its returned value ignored — VERIFIED maintainability issue; future divergence INFERRED.**  
   Both Providers calculate `start` before calling `receive_switch`, which independently calculates and returns `SwitchAccepted.boundary`:
   - UHD: `crates/ezsdr-radio-uhd/src/provider/control.rs:554–570`
   - Mock: `crates/ezsdr-mock-radio/src/lib.rs:1687–1705`
   - Shared transition: `crates/ezsdr-radio/src/staging.rs:501–508`

   Today the arguments agree. A one-sided change can give the payload/row and pending switch different boundaries—the mismatch explicitly modeled by mutation `E2A3-17`.  
   **Fix:** use the accepted boundary for the booking and row; retain preview computation only where validation requires it.

### Coverage assessment

Static tracing found the intended ordering intact: completion precedes halt cancellation; unread receipts gate subsequent sends; abandon removes held/unread switch payloads while preserving later bookings. The #93 equality/no-tick/sequence-error cases and hardware-switch rehearsal have explicit assertions. No ineffective mutation killer was established.

**Not checked:** no builds, tests, mutation runs, or hardware measurements were rerun. Repository files were unchanged.
