Before reading the diff, my failure scenarios are:

- Duplicate or re-entrant registration panics after allocating an id, leaving the clock frozen; unwind or TLS destruction panics again while removing it.
- Dropping guards out of order removes another clock’s entry; a stale pointer identity later matches a new clock.
- A spawned UHD participant calls `drive()` or waits/locks on another clock and uses the wrong clock-local id.
- Notify races with entering a wait, or unregistering the last busy participant fails to wake stalled waiters.
- Join waits on manual time while a worker is busy and never reaches `JOIN_BOUND`; alternatively, its real-time wait lets manual time advance past the joining participant before it resumes.

I’ll check these against the implementation first, then compare the tests and mutation evidence with the reviewed plan.
The registration and join paths look consistent with the agreed contract. I found a mutation-evidence gap: rows 02, 03, 04 and 11 also appear to die only on unmarked hang bounds, but unlike 14/15 they do not declare `kill: bound`. The runner reports all six as ordinary `killed`, so the full-list total does not distinguish those bound kills.
## Verdict: FAIL

**What this change does:** Adds per-clock, thread-bound driver registration and makes join polling a clock wait while retaining its real-time deadline. No production correctness defect found; one major mutation-evidence issue blocks approval.

### Major

**1. Hang bounds are counted as ordinary assertion kills — VERIFIED.**

Locations:
- `plan/maintenance/tools/mutations.json:18205–18235,18297–18305,18327–18349`
- `crates/ezsdr-radio-uhd/src/host.rs:1371–1391,1418–1445,1549–1550`
- `crates/ezsdr-radio-uhd/src/provider/mod.rs:1306–1308`

Minimal sequence:
```text
N51A-02: drive clock → sleep without participant id
clock stays frozen → within expires after 5 s
unmarked panic → runner reports ordinary “killed”
```

The exact-duration assertion is never reached. The same problem affects:

| Rows | Actual killing observation |
|---|---|
| N51A-02/03 | `within` expires |
| N51A-04 | `joined` expires |
| N51A-11 | Dropped driver remains busy; completion wait expires |
| N51A-14/15 | J2’s 10 s / 30 s bound expires |

Rows **14/15 declare `kill: bound`, but that alone is insufficient**: without an infra marker, `mutate.py:178–182` reports plain `killed`. `/tmp/opencode/mut-1.log:29–46` confirms these outcomes. Rows **02/03/04/11 do not even declare the bound dependence**.

**Wrong outcome:** bound-only failures enter the ordinary-kill tally and bypass bound accounting. A load-induced timeout could likewise count as a kill (**INFERRED scheduling outcome**).

**Fix:** Retarget 02/03/04 to the existing E9 state assertions and 11 to the duplicate-registration test’s immediate post-drop state assertion. For genuinely bound-only 14/15, emit an uncaptured `EZSDR-INFRA:` marker when their bounds expire; retain `kill: bound`. Verify the exact filters and classifications again. Do not merely remove their declarations.

### Otherwise checked

**VERIFIED by inspection:** duplicate refusal precedes allocation; TLS borrows do not span user callbacks; guards retain clock identity and remove their own pair; unwind and TLS-exit removal are handled; lookup covers sleep, wait and lock acquisition. The amended setup handshakes address the prior races. UR-33 M1/M6 and UR-16 match the agreed semantics, including **no manual-time cap from a real deadline**. No new maintainability finding.

Not checked: no new builds, probes or mutation runs; review remained read-only. The supplied full-list total does not resolve the classification defect.
