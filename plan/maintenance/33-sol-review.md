# 33 — Sol のレビュー記録（2026-10-25）

openai/gpt-6.1-sol（OpenCode2，--agent plan，read-only）．brief は orchestrator が書いた．各回の最終報告をそのまま写す．

## sol

VERDICT: FAIL

## Independent contract check

**Method caveat:** my first read inadvertently included §5–§6, so this was not a blind first pass. The following list comes from checking the contracts and device evidence directly.

- **Untimed tuning/gain:** these devices cannot honor existing `hardware_timed` keys under UC-6. Changing their class per device requires Kernel changes; representing untimed configuration does not inherently require that particular solution.
- **HackRF timing:** an ungoverned sample-count Root does not satisfy the current receive-origin, T0 and schedule contracts (TM-13b, KC-15/KC-16, SB-16, RM-25).
- **Unplaced losses:** fatal termination alone does not establish that previously published samples remain correctly timestamped.
- **Underflow reporting:** missing occurrence timestamps conflict with SC-25, not merely radio event conventions.
- **Command granularity:** X440’s command grid is coarser than its sample grid; UC-6’s existing rounding exception does not cover that.
- **Rate changes:** immutable Roots are required, but neither integer-multiple sample rates nor a Run-wide fixed sample rate follows from that.

## Findings

### S1 — major — G4 / 判断1: Kernel classification is conditional, not unavoidable

**Location:** `plan/maintenance/33-radio-generality.md:83,105,120–126`.

The note equates preserving the existing keys’ global `hardware_timed` declarations with representing these devices. Those are different requirements. A Vocabulary-only alternative is to declare common frequency/gain updates `cold` for every Provider, with optional timed controls handled separately if required. This trades away uninterrupted timed updates through the existing keys, but does not require device-specific Vocabularies.

**Evidence:** update classes belong to Vocabulary `KeyDecl`s under RS-52 (`design/04-run-and-session.md:261`); radio assigns the classes in RM-27 (`design/07-radio-model.md:267–279`). UC-3 already supplies cold semantics (`design/05-module-api.md:229`).

**Fix:** distinguish “Kernel change needed to retain the existing same-key timed API” from “Kernel change needed to support the device.” Evaluate the globally-cold alternative explicitly; retain the override recommendation only if its portability benefit justifies the additional Kernel mechanism.

### S2 — major — 判断1: target-node fallback can give one parameter two classes

**Location:** `plan/maintenance/33-radio-generality.md:120–122`.

Reading the target node’s override and otherwise falling back to the Vocabulary is insufficient. Radio permits the device **and stream nodes** as update targets; the key identifies what changes, while the target can merely select the schedule clock. An override placed only on the device can therefore make the same frequency key `cold` through one target and `hardware_timed` through another.

The proposed scope also omits validation’s shared class lookup and the rule that declarations cannot change between validation and arm. Scheduled updates receive their final class at admission—not compilation during validation.

**Evidence:** RM-12 (`design/07-radio-model.md:144`); SB-16 (`design/03-spec-and-binding.md:187`); KC-17 and KC-24 step 5 (`design/06-kernel-coordinator.md:250,291`). Validation and admission currently share `declared_class` (`crates/ezsdr-kernel/src/spec.rs:1204–1228`).

**Fix:** require one immutable effective declaration for each bound parameter, consistently resolved through every legal target and origin. List SB-16, UC-1, RS-52, KC-17/KC-24, the shared lookup and public declaration schemas as affected—not just RS-52 and a tree field. RM-21/RM-25 must use that effective class.

### S3 — major — 判断2: the proposed escape does not keep Kernel timing unchanged

**Location:** `plan/maintenance/33-radio-generality.md:131–138`.

Making the sample-count Root ungoverned avoids scheduling on it; it does not explain how radio’s first receive segment begins at T0. With a host primary Root, HackRF’s SampleClock is off-primary and cannot supply SB-16 schedule times. Making the count Root primary instead requires governance and an account of `now`, pacing and the advance before samples arrive. Calling an actively advanced Provider counter “ungoverned” also needs reconciliation with TM-16b’s definition.

Fatal-on-loss addresses neither this initial-origin problem nor delayed discovery: already-published post-loss samples may have been assigned false contiguous indices.

**Evidence:** TM-13b (`design/01-time-model.md:161`), TM-16a/a1/b (`:193–197`), SB-16 (`design/03-spec-and-binding.md:187`), RM-25 (`design/07-radio-model.md:252`). HackRF loss is polled and unplaced (`plan/maintenance/33-facts-hackrf.md:110–124`); the [pinned firmware](https://github.com/greatscottgadgets/hackrf/blob/v2026.01.3/firmware/hackrf_usb/sgpio_m0.s#L57-L68) confirms discarded samples do not advance the byte counter.

**Fix:** keep acceptance as an owner decision, but mark “Kernel unchanged” unproven. The follow-up must settle origin/T0, governed domains, permitted schedules and a publication boundary that cannot include unnoticed loss; fatal termination must have explicit first-loss-stop or equivalent safeguards.

### S4 — major — G7: a Provider spec cannot waive SC-25

**Location:** `plan/maintenance/33-radio-generality.md:86`.

“No asynchronous report” and “no report at all” are conflated. HackRF exposes polled underrun statistics, but not occurrence timestamps. More importantly, SC-25 requires an underflow event carrying **the time it occurred**. Listing a never-emitted kind in a Provider spec cannot override that Kernel obligation. MockRadio’s list is not a general exemption.

**Evidence:** SC-25 (`design/02-stream-contract.md:250`); HackRF counters and polling (`plan/maintenance/33-facts-hackrf.md:127–136`); bladeRF’s unexposed FPGA count (`plan/maintenance/33-facts-bladerf.md:138–148`).

**Fix:** classify this as a Kernel-contract question unless the supported operating envelope demonstrably excludes underflows. Decide whether unknown occurrence time is representable before freeze; do not silently substitute detection time.

### S5 — major — G12 / 判断3: fixed sample rate is a policy, not a consequence of TM-3

**Location:** `plan/maintenance/33-radio-generality.md:91,145–148`.

The rejection of a fixed-rate logical Root because arbitrary integer sample rates lack a common Root rate is mathematically wrong: Derived ratios are rational, not necessarily integers. For example, a 20 MHz Root represents 10,000,002 samples/s with ratio `10,000,000 / 5,000,001`, within TM-3’s bounds. Whether hardware counter transitions can be mapped exactly is a separate evidence question.

Also, TM-16a declares **governance**, not an immutable set of every Root a Run may register. HackRF has no sample-rate time counter in the first place.

**Evidence:** TM-3/TM-10 (`design/01-time-model.md:134,150`), domain immutability in TM-12 (`:157`), governance in TM-16a (`:193`); HackRF facts (`plan/maintenance/33-facts-hackrf.md:48–59`).

**Fix:** describe binding-fixed rate as a conservative restriction, not a forced Kernel rule. Reassess logical-root mapping and new-root alternatives against counter resets, exact origins, governance and RM-25/SB-16; do not reject them using nonexistent integer-ratio or fixed-registry requirements.

### S6 — major — checklist G4/G20 misses command-time quantization

**Location:** §3; G20 at `plan/maintenance/33-radio-generality.md:99`.

The facts contain a freeze-relevant mismatch that the classification drops: X440 at 500 MHz compares command times only every 16 ns, despite a 2 ns sample period. UC-6 promises exact application, allowing advancement only to the next **sample** instant—not the next eight-sample command boundary.

**Evidence:** `plan/maintenance/33-facts-n3xx.md:379–386`; [UHD v4.10.0.0 documentation, lines 140–144](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/timed_cmd.dox#L140-L144); UC-6 (`design/05-module-api.md:236`).

**Fix:** add command-grid granularity to the checklist. Decide whether unsupported instants are explicitly refused/restricted without Kernel changes or UC-6’s timing promise must change; envelope lead and queue values alone do not resolve it.

### S7 — minor — “合う” sometimes upgrades conditional evidence to established support

**Location:** G1–G3, G6, G9.

bladeRF RX/TX counter equality is explicitly INFERRED; trigger gating does not reset those counters. Its loss-count reconstruction also rests on an inferred interpretation of a fetcher summary. N3xx/X4xx timestamp carry-over is marked INFERRED in the facts but stated unconditionally in the classification.

**Evidence:** `33-facts-bladerf.md:35–36,126–131,189`; `33-facts-n3xx.md:57–63,340–343` (all under `plan/maintenance/`).

**Fix:** retain conditional classifications and name the remaining proof obligations. For bladeRF late TX, a host check plus margin is not proof against post-check transport delay; recheck the Module-only claim against RM-14’s exact-start promise.

No implementation design expansion is needed to repair this note—only corrected classifications, conditions and decision boundaries.

Skipped: builds, tests and hardware execution; no files or Git state changed. Vendor-source verification was targeted, not exhaustive.

## sol-fix1

VERDICT: FAIL

## S1–S7

All locations below refer to `plan/maintenance/33-radio-generality.md`.

- **S1 — fixed** (`:86,109,128–135`). Kernel changes are now conditional on preserving the same-key timed API. Alternative (b) is viable and fairly compared: it avoids Kernel changes but needs rules for two keys controlling one value. Recommending (a) is reasonable.
- **S2 — fixed** (`:129`). One immutable effective class per bound parameter covers both device and stream targets. The affected validation/admission lookup, schemas and rules are listed. This agrees with `spec.rs:1204–1228` and its callers.
- **S3 — fixed** (`:111–118,139–143`). The note no longer claims an ungoverned counter solves HackRF timing. Origin/T0, governance, schedules and unnoticed loss are explicitly assigned to the follow-up.
- **S4 — fixed** (`:89,116–118,143`). SC-25 is correctly treated as a Kernel obligation. The distinction between HackRF’s untimestamped observations and bladeRF’s unobservable underflows is retained.
- **S5 — fixed** (`:94,150–153`). Binding-fixed rate is now a conservative policy, not a mathematical necessity. Rational mappings and additional Roots are no longer incorrectly excluded.
- **S6 — fixed**, but the proposed remedy needs correction below (`:63,105,120,155–160`). G23 and K3 now expose the command-grid mismatch.
- **S7 — fixed** (`:79,83–85,88,91,164`). Relevant conclusions retain conditions, inferred evidence and follow-up obligations. BladeRF’s host-side late check is no longer presented as proof of exact transmission.

## New finding

### R1 — major — Decision 4 understates the change and misidentifies X3xx’s grid

**Location:** `:157–159`.

**Problem:** Generalizing UC-6’s rounding exception is sound in principle, and needs no new Kernel class. However, changing that sentence and declaring a capability is not sufficient: radio’s effective-time rules and implementation must use the rounded command instant consistently.

The claim that X3xx’s command grid equals its sample grid is also false for decimated streams. At 200 MHz MCR and 1 Msps, their periods are 5 ns and 1 μs respectively.

**Evidence:**
- [UHD v4.10.0.0 `timed_cmd.dox`, “Command Time Resolution”](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/timed_cmd.dox) explicitly distinguishes radio command timing at MCR from X440’s additional physical quantization.
- `design/18-uhd-radio.md:60,203` distinguishes master-clock Root ticks from decimated SampleClocks.
- `design/07-radio-model.md:233–235` defines booking, queue occupancy and application using the effective instant.
- `crates/ezsdr-radio-uhd/src/provider/control.rs:561–592` currently converts to Root ticks, then books and projects the unquantized `e`. On X440, an `e` between command boundaries would therefore become the recorded configuration instant before the device can apply it.

**Fix:** Keep the UC-6 direction, but list the corresponding radio/Provider amendment: determine the actual supported command instant before booking and use it consistently for ordering, queue accounting, configuration projections and applied-time records. Define the grid’s clock/anchor and aligned-time rounding. Correct the X3xx sentence to distinguish MCR and stream rate; do not assume one instance-wide grid covers every command path without verification.

Decision 1’s comparison is acceptable. Decision 4 is not yet complete enough to accept as “wording only.”

Not checked: builds, tests or hardware; vendor verification was targeted. No files or Git state changed.

## sol-fix2

VERDICT: PASS

**R1 — fixed.** `plan/maintenance/33-radio-generality.md:120,157–163` now:
- Distinguishes X3xx’s master-clock command grid (200 MHz → 5 ns) from a decimated stream’s sample grid (1 Msps → 1 µs), consistent with `design/18-uhd-radio.md:60,203` and UHD v4.10.0.0’s “Command Time Resolution.”
- Requires determining the supported command instant before booking, then using it consistently for ordering, queue accounting, configuration projections and applied-time records.
- Requires specifying the grid’s clock, anchor and rounding behavior, without assuming one instance-wide grid covers every command path.
- Explicitly includes radio-rule and Provider implementation changes, rather than treating this as UC-6 wording alone.

The implementation citation is accurate: `control.rs:561–592` converts the deadline to Root ticks, computes `e`, and uses it for bookings and projections without additional command-grid quantization. RM-21 (`design/07-radio-model.md:233–235`) confirms why the effective-time rule must change alongside that implementation.

**New findings:** None in the rewritten text or its immediate context.

Not checked: builds, tests or hardware, as instructed. No files or Git state changed.
