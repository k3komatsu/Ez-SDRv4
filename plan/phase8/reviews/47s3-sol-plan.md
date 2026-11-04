## PLAN NEEDS CHANGES

Reviewed at `d8f23af`, clean tree. Independent scenarios were written before reading S1–S11.

References below:
- **P** = `~/.cache/ezsdr-fixes/2026-11-04-n46/test-plan-s3.md`
- **R** = `~/.cache/ezsdr-fixes/2026-11-04-n46/rulings-s3-plan.md`

### Major 1 — Scheduled updates cannot follow the proposed successful sequence
**P:13,137,281; R:6**

The planned schedule `Stop(+5 ms) → Set(+6 ms) → Start(+7 ms)` cannot stage successfully:
- Scheduled updates acquire `at: Some(...)` (`crates/ezsdr-kernel/src/event.rs:929–933`), which D3 rejects.
- Timed schedule entries are dispatched together at startup; Stop instead enters the runtime agenda (`coordinator/pipeline.rs:337–344,747–787`). Distinct offsets do **not** establish booking order.

**Minimal sequence:** submit that Spec → update reaches the Provider before the scheduled Stop, carrying `at`.
**Wrong outcome:** update is refused; the later start uses the initial value, contrary to B3’s assertions.

**Fix:** keep the Kernel boundary unchanged and test/document scheduled-value refusal for this stage. If successful scheduled staging is required, resolve its dispatch semantics explicitly before implementation; removing only the `at` check is insufficient. Ruling D3 can stand, but B3 must change.

### Major 2 — D4 silently loses lead whenever configure finishes before the booked origin
**P:14,98,186,266; R:7**

I would overturn ruling D4 **as written**. UHD lateness is not an assertion that the configured start lead remains available.

**Minimal sequence:** stop has settled → book origin at `b + 50 ms` → configure completes at `b + 25 ms` → issue the timed start.
**Wrong outcome:** only 25 ms remains, with neither origin ≥ configure completion + 50 ms nor a lateness report. P:98 explicitly expects this case to succeed silently.

`rx.rs:472–489` handles a device-reported missed instant, not exhausted configuration-relative lead.

**Fix:** add this partial-consumption case. At owner completion, enforce completion + lead or explicitly report/reschedule the lead violation while preserving the clock-origin contract. This need not move device configuration back into uhd-control. Merely measuring a larger margin changes the requirement rather than testing it.

### Major 3 — Composite Python calls never test stopping on a Provider refusal
**P:110–113,162–167,255**

S9 exercises a **Kernel** rejection of the second value. S2 exercises a channel setter. Neither proves that `Rx.start` handles an **admitted entry plus Provider refusal**.

**Minimal sequence:** stream is streaming → `rx.start(frequency={0: f2, 1: f3})` → first update is Kernel-admitted but Provider-refused.
**Wrong outcome:** `start` checks only the admitted outcome and sends the second update and `start_rx`. The proposed S9 and setter tests can both pass.

**Fix:** add this real-Mock sequence, asserting `Rejected.event`, the first Action’s id, and no subsequent value or command. Keep the existing second-value Kernel-rejection case; it tests retained staging separately.

### Minor 1 — The proposed configure-once observation bypasses `Owner::Nothing`
**P:156,211,248**

`RxRecv::LateCommand` retries through `device.rx_start` directly (`rx.rs:486–489`), not `owner_configure`. Therefore “no extra apply during missed-start restart” cannot kill N3-34’s always-configure mutation.

**Fix:** explicitly invoke `owner_configure` twice for the same current segment and assert one configure, then use a new command/snapshot and assert another. Retain existing killing coverage until that replacement demonstrably exercises `owner_step`.

### Minor 2 — The O2 early-return fallback does not test an interrupted wait
**P:164**

An already-ended Run can return `Ended` without exercising the new post-submit check or `wait_finished`’s early exit.

**Fix:** require a gated Provider: dispatch → pause before finishing booking → trigger freeze/end → release. Assert no successful `Submitted` response. For normal H6 timing, also assert submit cannot complete before the booking gate is released.

H6’s normal-path reasoning is otherwise sound: `state.rs:70–80` acknowledges completion on the **next** `recv`; `pipeline.rs:1034–1048` waits and drains, or steps MockRadio and drains.

### Minor 3 — Some mutation descriptions are impossible or have unproven killers
**P:227,240,251–255**

- N3-10’s live-reference snapshot is not a compiling replacement for an owned `BTreeMap`.
- Applying Mock values at booking rather than origin need not change S1’s samples: the interval is stopped.
- Removing the event-kind filter is not necessarily killed by two Action-id-filtered replies.

**Fix:** specify compiling, single-behaviour replacements and distinct killing observations; drop ownership-guaranteed or observationally equivalent mutants rather than adding anchor scaffolding.

### Minor 4 — Hardware evidence and acceptance scope need correction
**P:35–40**

H2 cites note 39’s superseded INFERRED label; its header correction marks stale overwrite VERIFIED (`plan/phase8/39-dsp-timed-commands.md:3`). UHD 4.10’s DDC writer uses command time (`ddc_block_control.cpp:650–660`); the FPGA bypass evidence distinguishes untimed writes from sample-driven timed entries.

H1 correctly leaves held-open retuning INFERRED, but note 47’s explicit held-open acceptance gate is **rate-specific**, not a completed retune gate.

**Fix:** correct H2’s citation and state separately what retune bench verification remains. Readback must not imply RF settling.

D1’s shared decision type, D2’s booking-time rule, D8’s antenna-envelope substitution, and D10’s independent oracle are appropriate. Preserve the named cancellation, partial-configure retry, peer-isolation and history cases.

No files changed. No builds, mutation execution or hardware checks performed; proposed mutation kills remain unverified.
