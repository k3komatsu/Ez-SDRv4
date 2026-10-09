# Maintenance note 25 — Faults and marks at the end of a Run (three-strikes design notes for #65 and #66)

| Field | Value |
|---|---|
| Status | Design notes required by AGENTS.md §6 before fixing #65 and #66, written by the orchestrator (Claude Opus) on 2026-10-09. The owner approved fixing both ahead of the other pre-freeze items (audit, [24-prefreeze-audit.md](24-prefreeze-audit.md), F19 and F21). |
| Mechanisms | #65: `failure-collapsed-into-value` (sixth bug). #66: `pending-provenance-collapsed` (fifth bug). Registry: [failure-mechanisms.md](failure-mechanisms.md). |

## #65 — a fault after a DeviceLost in the same round is dropped

**What failed.** The fault loop exists twice, in `coordinator/stepping.rs` and in `coordinator/paced.rs`. Each copy decides with `index == 0` whether a fault may still end the Run. Once the first fault is a DeviceLost, which goes through the Policy, every later fault in the same list is dropped. If the Policy does not abort, the Run continues with an instance that failed and left no record.

**The design questions.**
- *Is a rule implemented twice?* Yes: the two coordinator drivers each carry KC-30's fault rule. This is the same shape as spec 22's MockRadio/UHD duplication, inside the Kernel.
- *Is the concept needed?* Yes: KC-30 is a rule, not an accident of the loop.
- *Is the spec text too complex?* No. The code drifted from a simple rule.

**Outcome.**
- One function applies KC-30 to a round's faults, and both drivers call it.
- Every fault is handled on its own terms: a DeviceLost goes through the Policy; any other error or a panic requests `Failed { run }`.
- A test per driver puts a DeviceLost before a panic.
- Recording every later failure, not only the first (`Termination` with a reason and `also`), is audit item 4's record shape. It lands with spec 24, not with this fix.

## #66 — marks delivered during cleanup never reach an artifact

**What failed.** Cleanup step 6 applies the recorded marks to the Sinks' artifacts. Step 7 then drains the event queues one last time, and the marks that drain delivers go into `shared.marks`, which nothing reads any more. A warning still in flight when the Run stops never taints the artifact it overlaps (RS-30).

**The design questions.**
- *Is provenance held apart from its record?* Yes: marks wait in one shared list. Finalisation reads that list at a fixed step instead of after the last event that can add to it.
- *Can the concept be removed?* No: RS-30's taint is the point.
- *Is the spec text too complex?* The RS-6 cleanup table orders "finalise artifacts" before "flush events". That order is the defect, in text and code alike.

**Outcome.**
- The last drain runs before artifacts are finalised, under `abort` as well as `orderly`.
- Finalisation then reads every mark delivered.
- The RS-6 table and KC-42 change their order to match.
- A test emits a warning that is still queued when cleanup starts, and expects the artifact to carry the mark.
