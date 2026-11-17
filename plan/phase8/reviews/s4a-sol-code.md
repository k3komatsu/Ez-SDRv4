# Stage 4a — Sol's code review (openai/gpt-6.1-sol, read-only; the final answer, tool transcript left out)

What this change does: it shares the measured profile values, adds receive-start heads, confirms open flushes with retries, and repairs the bench probe. **No major code finding identified.**

The failure scenarios checked included wrong late-send placement, heads repeated across reads or omitted at restarts, stale or partial flush acknowledgements, probe-clear races, and unrelated defects hidden by the #108 exemption.

1. **Minor — MA-10 still contradicts the adopted receipt-lead rule.**  
   `design/05-module-api.md:161–162` says the declared lead minus the actual delivery allowance is what RM-14 uses. For stepped MockRadio that implies **5 − 0 = 5 ms**. However, amended RM-14 explicitly uses the emulated allowance, giving **5 − 3 = 2 ms** (`design/07-radio-model.md:160`), as implemented in `crates/ezsdr-mock-radio/src/profile.rs:160–161`.

   **Fix:** distinguish Kernel dispatch allowance from the profile’s emulated device-placement margin in MA-10, and repair its Test line. Keep the adopted code behaviour.

2. **Minor — KL-47’s last workaround does not prevent the head.**  
   `KNOWN_LIMITATIONS.md:237` suggests starting the source before receiving when capture needs signal from its first sample. Starting the source earlier does not remove a receive-start head: FakeDevice sets it unconditionally at start and replaces samples before consulting the source (`crates/ezsdr-radio-uhd/src/device.rs:500–504,1856`). The Mock’s restart tests explicitly expect zeros with the source already transmitting (`crates/ezsdr-mock-radio/tests/mock_channel/s4a.rs:102–110,148–178`).

   **Fix:** say to start the **receiver** early and place the capture after its settling head, with the source already active. Source ordering alone is insufficient.

Not checked: builds, tests, mutation runs or new hardware measurements; none were run. Hardware behaviour beyond the recorded bench conditions remains inferred.

**Verdict: PASS WITH MINORS — correct documentation findings 1–2; no major code blocker found.**
