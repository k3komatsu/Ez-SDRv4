# #106 — Sol's code review (openai/gpt-6.1-sol, read-only; the final answer, tool transcript left out)

What this change does: Removes the repeat’s second-drop termination and adds exact resume counters (`crates/ezsdr-radio-uhd/src/provider/tx.rs:1136–1161`). It bounds Manifest records by kind and validates the selector before opening the device (`crates/ezsdr-radio-uhd/src/provider/core/records.rs:45–59`; `crates/ezsdr-server/src/catalogue.rs:138–144`).

Read-only review; no files changed, builds run, or mutation results independently reproduced.

## Findings

1. **Major — A joined switch duplicates `resumes_at` for one resume.**

   **Sequence:** Start repeat A, hand switch B’s first send as a continuation of A’s device burst, then receive a `TimeError` reaching that burst. Production registers B in the same correlation (`crates/ezsdr-radio-uhd/src/provider/tx.rs:801–804`), and retirement returns both targets (`crates/ezsdr-radio-uhd/src/provider/tx.rs:407–412`).

   **Wrong observable outcome:** One resume produces two `TIME_ERROR` events carrying the same non-null `resumes_at`. The handler computes the plan once, then passes it unchanged to **every** target (`crates/ezsdr-radio-uhd/src/provider/tx.rs:1463–1475`). This contradicts the step’s new “at most once per resume” promise (`design/18-uhd-radio.md:357`) and can make consumers observing that field count one resume twice.

   **Why tests miss it:** The existing joined-part test exercises precisely the multiple-target case, but checks only target names—not the plans carried by their events (`crates/ezsdr-radio-uhd/src/provider/tx/tests/mt3d.rs:381–396`).

   **Fix:** Add the joined-target payload assertion. Resolve which event carries the single plan while preserving the required event for every target. If carrying the plan on every target is intentional, bring that user-visible contract decision back to the owner; the current implementation and new promise cannot both stand.

**Verdict: Fix finding 1 before landing.**
