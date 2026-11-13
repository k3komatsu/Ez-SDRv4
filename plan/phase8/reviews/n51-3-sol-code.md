# Note 51 stage 3 — Sol's code review (8d96743..ba9ff7d, 2026-11-14)

## Verdict: PASS

No major findings. The stage closes the six plan-review findings. One minor scan gap remains.

### Findings

1. **Minor — ordinary suffixed decimal literals bypass the scan.**  
   `crates/ezsdr-acceptance/tests/governance.rs:736–742`

   `short_wall_at` removes underscores and parses the whole token as `u64`. Valid Rust such as:
   ```rust
   rx.recv_timeout(Wall::from_secs(1_u64)).unwrap();
   ```
   therefore passes: `"1u64"` does not parse. This is an ordinary authoring pattern, not one of the documented non-decimal/computed-literal limits.

   **Fix:** recognize integer suffixes and add a fixture. This is nonblocking because it does not invalidate an existing rewritten test. **VERIFIED parsing gap; INFERRED future bypass.**

2. **Note — the TTL exemptions retain genuine load-sensitive verdicts.**  
   `crates/ezsdr-kernel/tests/device_paced.rs:506–536`

   Disconnect → deschedule the caller beyond the TTL → Renew/Adopt can encounter an already-expired lease. Pinning these tests does not make them deterministic. This is pre-existing behavior explicitly retained by P3a, so it does not block this stage; the broader claim that *no verdict depends on wall time* remains qualified. **VERIFIED structure; INFERRED scheduling outcome.**

3. **Note — function-name-only pins do not currently admit additional matching calls.**  
   `crates/ezsdr-acceptance/tests/governance.rs:780–789`

   Exact call text and aggregate occurrence counts still reject an extra matching sleep, even in a same-named function. Names alone cannot distinguish a relocated approved occurrence, but the documented pin already excludes checking the surrounding judgment. No current incorrect exemption found. **VERIFIED.**

### Plan-review closure

- **M1:** emission follows observation of the parked horizon; the callback-before-return count is checked after joining the data thread (`device_paced.rs:367–400`).
- **M2:** `restart_stage` covers event decoding, association, classification and reported JSON, not merely formatting (`measurement.rs:2287–2315`, `4352–4418`).
- **M3:** undeclared marked rows are rejected independently of outcome; cached baselines retain row-specific declarations (`mutate.py:286–317`; `gate.py:224–236`).
- **m4–m6:** J58 remains equivalent; retained bounds agree with the scan; allowances pin calls and counts rather than whole functions.

The untested `main()` forwarding of `declared` is simple wiring between separately tested components, not a blocking coverage gap. No stage-introduced duplicated decision or competing lifecycle state warranted another finding.

**Checks:** `gate.py --self-test` passed. Read-only accounting of the supplied mutation log confirms 2,746 rows, 27 marked rows all declared, and only the reported G09 survivor.

**Not checked:** no builds, hardware runs or mutation tools were run; no files changed. Seconds-long backstops and the documented scan limits remain risks, not proofs of load independence.
