# A1 — Sol's code review (2026-11-12)

**Verdict: FAIL — two majors.**

What this change does: adds a binary-only timer that exits with status 3 after input ends, without cleanup.

### Major

1. **Thread-creation failure defeats the backstop** — `crates/ezsdr-server/src/main.rs:30–33`  
   **VERIFIED:** Injecting `pthread_create → EAGAIN` made `ezsdr-input` panic; the stdout-blocked server remained alive beyond its configured deadline. Without injection, it exited 3. The panic also prevents `forward()` from reaching `watch.fire()` (`transport.rs:47–59`).

   Minimal sequence: start repeat → enter a blocking request/reply → stdin ends → backstop spawn fails. Neither the timer nor disconnect notification runs; the server can remain transmitting.  
   **Fix:** use fallible thread creation and fail-stop immediately if it fails.

2. **60 seconds is not longer than RS-8a’s worst cleanup** — `design/16-easy-api.md:85`; `crates/ezsdr-server/src/main.rs:16`  
   **VERIFIED:** cleanup deadlines apply separately to each fragment in three passes, not once to seven aggregate steps (`crates/ezsdr-kernel/src/run.rs:501–505, 545, 575–585`). There is no global 35-second bound.

   Counterexample: EOF → 13 Provider stops each return after 4.9 seconds, within their individual deadlines → the backstop exits at 60 seconds before cleanup finishes, losing the Manifest. **INFERRED operational occurrence; not reproduced on hardware.**  
   **Fix:** reopen the owner’s timing premise: either establish a supported total bound or explicitly adopt 60 seconds as a cutoff that can interrupt valid cleanup. Merely increasing the constant does not establish a general bound.

### Minor

3. **A1-3 can survive through scheduling alone** — `crates/ezsdr-server/tests/protocol.rs:2256–2266`  
   **VERIFIED by inspection:** elapsed time is measured when the parent observes exit, not when the child exits. Remove the sleep, then deschedule the parent for 2–10 seconds: status 3 and the lower-bound assertion both pass. Thus the mutation’s reported kill is scheduling-sensitive. Pin the delay independently; retain this subprocess test for status and the upper bound.

### Other checks

- EOF/non-`Interrupted` errors arm the timer before watch callbacks; ordinary reads and `Interrupted` do not. Detached refusal remains intact.
- The 10-second expiry writes the infrastructure marker; A1-1/A1-2 appropriately declare bound kills.
- The new subprocess test passed on sim03. Governance exemption follows the existing pattern; Python retains its 30-second fallback.

Not checked: full suites, mutation reruns, or E9 raw bench records. No repository files changed.
