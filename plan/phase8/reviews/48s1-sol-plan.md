> plan · gpt-6.1-sol
# Verdict: PLAN NEEDS CHANGES

**Keep R1 and B1, but replace (c)’s total-length rule with a first-send minimum.** Addendum 2 invalidates several planned tests and their expected results.

References below: **Plan** = `~/.cache/ezsdr-fixes/2026-11-05-n48s1/test-plan.md`; **Facts** = adjacent `hardware-facts-cpp.md`.

## Independent failure scenarios

Written before comparing the brief and plan:
- A device burst totals four samples, but its first send contains only one, two or three.
- Holding the tail turns an otherwise sufficient four-sample first buffer into three.
- Window availability, chunk size or an upcoming cut limits the first send below four.
- A short Kernel burst continues an already-started device burst: padding it would shift later payload.
- Flush reports interleave across channels/streams; one ACK or all ACKs never arrive.
- Recovery replaces a streamer while another stream has a real underflow.
- A late timed first packet is followed by multiple untimed packets.
- Grouped-channel ACKs arrive separately; one device burst covers multiple Kernel records.
- Padding changes the safe front, but Stop or Mock retains the old front.

## Findings

### P1 — Closing-send padding does not fix the measured failure
**Plan §4.5, §6.2 L2b/L3b/L4b, §6.5; Facts:34–39.**

Minimal sequence: book a two-sample Kernel burst sufficiently ahead → send timed SOB with one sample → close untimed with three samples. The plan expects a clean ACK; addendum 2 measured intermittent drops, underflows and incorrect ACK timing for exactly **1+3**.

Likewise, the planned **2+2** and **3+2** cases are not valid successes.

Required rule and tests:
- A normal device-burst first send must request **at least four samples per channel**, after tail withholding.
- A finite Kernel burst shorter than four, when starting a device burst, goes in one timed SOB/EOB send padded to four.
- Do not pad or impose this minimum anew on a Kernel burst continuing an established device burst.
- Keep Kernel records/statistics payload-only; calculate the safe front from the actual padded device interval.
- Test payload values and zero positions, not only `n`, ACK count or total length.

### P1 — First-send window/cut paths are missing
**Plan §5.1:220, §6; `crates/ezsdr-radio-uhd/src/provider/tx.rs:516–554`.**

Minimal sequence: a long burst enters the window with `avail = 2` → current first-buffer exception sends two samples → hardware can drop it despite a long eventual total. Another: four-sample finite burst → hold its final sample → first send has three.

The declaration that chunking is unchanged is therefore unsafe.

Add deterministic cases for:
- `avail` of 1–3, then 4; no premature send or window violation.
- `wanted`/chunk below four, including configurations whose entire window cannot hold four.
- A four-sample candidate reduced to three by holding its tail.
- Held-start/preemption and clock cuts before four samples can be sent.
- Repeat waveforms of lengths 1–3 and repeat wrap.
- Stop/loss while waiting before the first send.

Specify progress versus explicit refusal when constraints make a legal first send impossible; never silently spin or send undersized. If waiting expires the start lead, reapply the existing late-policy rules before dispatch.

### P2 — Flush attribution is broader than its ownership boundary
**Plan §2.1:63–65, §2.3; `provider/core.rs:1103–1150`; brief:9.**

Minimal sequence: flush expects channel position 0 → an unrelated report for position 1 arrives → proposed unconditional non-ACK branch marks it `tx_flush` and removes it from normal processing. The brief limits attribution to the flush’s channels.

Define attribution using **streamer generation, stream, channel positions and the bounded flush window**, not merely “whatever this loop reads.” Preserve unrelated reports and their normal handling.

Required assertions:
- Grouped channels with interleaved reports/ACKs; duplicate ACKs do not complete another channel.
- Another stream’s underflow remains a Run event during recovery.
- Old-generation reports cannot become new-flush or new-burst reports.
- Missing ACK, exact-deadline return and post-timeout reports retain explicitly chosen behavior.
- A real underflow after recovery is emitted and abandons the burst exactly once.

Keeping the existing deadline boundary is reasonable; an after-deadline report is **not proven** to originate from experiment payload merely because it was read late. Test the conservative fallback without claiming otherwise. Preserve `Other(77)` handling (`core.rs:1649–1669`).

### P2 — FakeDevice still permits the newly measured broken split
**Plan §2.2, §6.3; `src/device.rs:611–652,685–732`.**

Minimal sequence: raw timed SOB of two samples → untimed EOB of three → proposed fake accepts both and ACKs cleanly; measured hardware sometimes fails.

UR-33 needs a deterministic adverse model for **an undersized first packet**, including non-EOB first packets—not only single SOB/EOB totals below four. Pin raw failing splits and a clean **4+2** control; Module tests must fail when first-send enforcement is removed.

Do not encode exact nine-report counts/spacing as universal hardware behavior. Keep the existing one-sample stale-header regression separate unless evidence justifies replacing it.

Also, the proposed `<4`-only `FlushUnderflows` cannot exercise attribution through the corrected four-zero production flush. Use a test-local scripted report source for that defence, without adding production shape-selection options.

### P2 — Mock parity remains a proposal, not an executable test plan
**Plan §3.5, §7 F9; `crates/ezsdr-mock-radio/src/lib.rs:1131,1369,1751`.**

Minimal sequence: one-sample burst at `s` → Stop → book another at `s+3`. UHD’s padded four-sample burst requires front `s+5`; the unchanged Stop frontier can retain `s+3`. Mock then admits a burst UHD refuses.

Name the differential tests, exact action ordering, observations and killing mutations. Cover normal completion **and Stop**, grouped/split streams, and short bursts that continue an existing device burst.

Update MR-17 **and MR-25** where required. Preserve the payload cut separately from padding and the safe-start margin; do not extend payload records to cover zeros.

### P2 — The late-first-send oracle misstates untimed DUC behavior
**Plan §5.3 F-own-1, §5.4; `src/device.rs:614–642`; `provider/tx.rs:1141–1161`.**

Minimal sequence: timed first packet arrives late → first untimed continuation also has a past inherited timestamp and is dropped → second untimed continuation arrives. The first untimed continuation has already cleared `has_time`; the next does **not** necessarily continue a timestamped rejection train. It can start untimed on a shifted timeline.

Replace the “train until the counter catches up” oracle with explicit assertions over at least three sends, playback positions and report availability. Distinguish:
- a report observed before the next send;
- reports delayed until several sends have occurred;
- existing cursor-expiry abandonment.

Do not automatically add abandon-on-TimeError: correlation can be ambiguous. First establish the reachable behavior and whether existing abandonment closes it.

F8 also needs corrected terminology: `remember_burst` tracks **device-burst starts**, not every Kernel burst (`tx.rs:355–362,664–665`). Assert channel-specific pending retirement and separately assert both Kernel records survive.

### P2 — Mutation and specification coverage is incomplete
**Plan §2.5, §5.5, §3.7, §5.7.**

- `TBD-a1/a2` describe fix-direction replacements, not regressions against the proposed final source.
- P1170/P1195–P1197 likewise retain today’s old→new fix direction; reverse/retarget them to introduce timed continuations into B1.
- Held-tail/close timing rows have the same direction problem.
- No explicit killing rows cover first-send minimum, premature window send, tail withholding, padding/front arithmetic, Mock/Stop parity or the new fake rule.
- F4-far and several length rows leave observations/incorrect behavior blank; F9 has no named test.
- UR-22/23 must describe first-send buffering, short-burst atomic padding and adjacency consequences—not merely closing padding.
- UR-28 must constrain flush attribution; UR-33 must cover undersized first packets.
- MR-17/MR-25 and `KNOWN_LIMITATIONS.md:289` need the revised rule: **first send ≥4**, not “four samples per device burst.”
- Reconcile the new ACK assertions with FakeDevice’s current last-sample ACK (`device.rs:724`); measured hardware ACK is exclusive end **plus one master-clock tick**, not one sample.

### P3 — Hardware assertions would lose unrelated protection
**Plan §2.4:89.**

Replacing `code != TimeError && code != Underflow` with `code != Underflow || tx_flush` removes the TimeError check. Preserve rejection of non-flush TimeError/Underflow reports. Four-zero clean-flush checks should additionally require no flush underflows.

## Answers to Plan §9

1. **R1.** Keep tail+zero closing and test `e+2` accepted / `e+1` late. Phase-7 records support the older one-real-tail behavior (`bench-results.md:882–904,934–944`), not weakening today’s front. Holding two real samples is possible but unnecessary here and complicates first-send/tail constraints.
2. **P-split is already answered by addendum 2.** Reject closing-only padding; enforce the first-send minimum. No duplicate probe is needed to decide that rule.
3. **Diagnostic optional**, useful for Module integration confirmation, not a prerequisite already settled by the independent C++ measurements.
4. **M1 in principle**, revised for the actual first-send policy and backed by named differential/Stop tests.
5. **Four-zero flush plus bounded attribution**, with the ownership restrictions above.
6. **Correct F-own-1 first; no automatic new TimeError-abandon mechanism.**
7. **Defer mechanism labels to adjudication.** R1 is correction of a stale expectation, not evidence that today’s front is defective.
8. **Four-sample control, yes—but its first send must actually contain four.** A total-four split is not a clean control.

Read-only review; no edits, formatting, builds or hardware runs. Multi-channel hardware behavior remains unmeasured, and #78 must remain open until Module-path bench confirmation.
