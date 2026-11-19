# Phase 8-end review — reviews

## GPT-6.1 Terra, 6cf43263

**Verdict: FAIL**

- **Major — the planned 8b remeasurement cannot measure 8b.** commands.sh (`plan/phase8/52-measure/commands.sh:7`) hard-codes `4679abc1` as the endpoint for churn, totals and test counts, while the owner’s decision calls for rerunning the measurement at the end of 8b to decide whether the test-to-production ratio is still growing (decision D3 (`plan/phase8/52-phase8-end-review.md:478`)). Minimal sequence: finish 8b, run the referenced script, and compare its “now” totals with the Phase 8 results. The script reports the same Phase 8 commit for both, so growth during 8b is invisible.

- **Minor — the generator trial has conflicting timing.** 50-remaining-work.md:65 (`plan/phase8/50-remaining-work.md:65`) schedules the trial before Gate X, while decision D1 (`plan/phase8/52-phase8-end-review.md:476`), its implementation note at line 484, and AGENTS §7 place it after Gate X. The working schedule should reflect the owner’s decision.

- **Minor — an inventory still describes the removed stat as present.** 49-time-and-booking-inventory.md:58 (`plan/phase8/49-time-and-booking-inventory.md:58`) says the stat counts discrepancies, and line 138 says tests pin it to zero. These are historical design notes, but neither passage marks that statement as superseded by D7.

For the two TTL tests, the counter is a count of **time reads**, not passes: the normal pass reads at its start and again in the drain. In this rig, three additional reads still require a lease check after the clock advance, even if a prior check was already in progress. An expired deadline requests termination before the next pass; the assertions and final termination reason detect it. The ten-second bound can still fail under extreme scheduling delay, but bounds the wait rather than making elapsed wall time the TTL oracle.

The TX production diff preserves the close and its ordering; the removed return value served the stat. The three re-anchored mutation rows retain their previous substitutions. I found no remaining active code, schema, Python, or bench-tool reader of the stat, and the checked relative Markdown links resolve. This was a read-only review; no builds, tests, or mutation runs were performed.

Verdict: FAIL
## GPT-6.1 Terra, re-check 7d32c187

**Major — the rerun mixes Phase 8b changes into the Phase 8 cohort.** steps.py:107 (`plan/phase8/52-measure/steps.py:107`) assigns every commit dated November 7 or later to `P8b`, the report’s label for the latter part of Phase 8. At the end of Phase 8b, commands.sh:13 (`plan/phase8/52-measure/commands.sh:13`) will therefore produce a combined cohort ratio rather than a separate Phase 8b churn ratio. The endpoint totals do allow a comparison of the overall test-to-production ratio, but the cohort report is misleading.

The shell syntax is sound on inspection. The script passes the selected endpoint to churn, totals, mutation-row counts and test counts; it retains `4679abc1` as a separate Phase 8 reference. Endpoint-specific totals and row outputs include the revision in their names. The D1 timing is reconciled by the recorded owner-approved move before Gate X, and both historical stat descriptions are marked as removed.

This was a read-only re-check; I did not run the script.

Verdict: FAIL