# 段階 3c-2a：Sol 6.1 のレビュー

## コードのレビュー（`sol-3c2a.log`）

VERDICT: FAIL

### Failure scenarios checked

- Commands execute before counting finishes; a host pause or slow call is mistaken for FIFO back-pressure.
- Tune and gain operations consume different queue space; RX and TX share the radio block’s queue.
- Commands left by one trial affect the next.
- Read-backs echo requests rather than coerced settings; gain-step sampling or stream indexing produces a misleading value.
- TX leaves the safety band or streams during raw measurement.
- FakeDevice records claim hardware measurements; rehearsal assertions depend on scheduling.
- Hardware measurements cannot reach their success conditions; record shapes, mutation rows or UR-34 misstate coverage.

### Findings

#### 1. Major — Queue classification accepts commands that have already executed

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:816–833`, `:934–958`

**Problem:** The classifier checks earlier calls’ durations but ignores their recorded device return times. Only the final slow call must return after `first_at`. Consequently, a host pause **between** calls can let the queue drain, and a later host pause **inside** an `apply` can supply the supposed blocking evidence. The resulting depth need not be a count of simultaneously queued calls.

**Concrete case:** On a seven-tune device, submit three short calls, pause between calls until after their scheduled times, then submit more short, now-late commands. If the ninth call experiences a host pause exceeding 1.5 seconds, the classifier accepts depth eight: all preceding calls individually took under 100 ms, and the ninth returned after `first_at`. No eight commands were queued together. The stored timestamps expose this case, but classification ignores them.

This also accepts an arbitrarily late return as evidence of queue blocking. Taking the minimum and requiring five successful trials does not make an individual false success valid.

**Evidence:** UHD 4.10’s [`ctrlport_endpoint.cpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ctrlport_endpoint.cpp), `send_request_packet()`, verifies that insufficient downstream credits cause a wait; it does **not** establish that every sufficiently slow host call was waiting for credits. The existing pure test at `measurement.rs:2872` never supplies an earlier short call whose return time is at or beyond the first command time.

**Fix:** Reject trials unless every counted call demonstrably returned before the first command could execute. Bound the submission/return window so delays outside the expected release interval cannot become successful evidence. Add synthetic cases for elapsed command times and excessively late returns, with mutations for those guards. Describe wall-time-based attribution as inferred rather than proof of the blocking cause.

#### 2. Minor — UR-34 adds implementation history to normative text

**Location:** `design/18-uhd-radio.md:351`

**Problem:** “and, **from stage 3c-2a**, raw queue-depth…” puts stage history into the rule. AGENTS.md §2 requires normative rules without history; the new Changes row already records the stage.

**Fix:** Remove “from stage 3c-2a” from the rule. Keep the history in Changes.

### Other review results

- **UHD behavior, VERIFIED:** Full control buffers wait for credits, with the longer timed-command timeout. An OBX timed tune is **not one FIFO entry**: its frequency experts commit LO registers and issue synchronization writes. See [`obx_expert.cpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_expert.cpp) and [`obx_cpld_ctrl.cpp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_cpld_ctrl.cpp). RX/TX daughterboard operations use the same radio-side register interface. Thus the probe measures queued **high-level operations under its stated conditions**, not literal FIFO entries.
- **Read-backs:** `src/uhd.rs:912–944` calls UHD getters for rate, frequency, gain and the explicitly changed untimed antenna. The unnamed gain path passes through UHD’s gain-group step allocation; it is not simply the unquantized request echoed by this Rust code. The current bench’s channel-indexed rate probes match its stream-key enumeration. Empty stream conditions are appropriate for these unstreamed raw probes and pass the existing condition rules.
- **Safety and records:** The added raw groups open no streamers. TX frequency requests remain inside `SAFE_TX_HZ`; frequency grids remain unmeasured. Fake-source merging replaces supplied values with `unmeasured`. Restoration attempts every channel even after an apply error.
- **Tests and mutations:** P732 and P807–P839 each match their source exactly once. Their cited assertions target the described changes, but none closes finding 1. The rehearsal’s relaxed call-count assertion avoids a scheduling-dependent verdict. UR-34’s newly cited tests exist.
- **Scope and cost:** No production `src/` changes or excluded measurements were added. The reported queue budget—25 trials, approximately 75–77 seconds plus overhead—and roughly 2,109 coercion calls plus antennas agree with the code. Actual coercion wall time remains unknown.

Read-only review; no builds, tests, mutation runs or hardware operations were performed. The implementation report’s execution results were not independently rerun.

## 修正の見直し（`sol-fix-3c2a.log`）

VERDICT: FAIL

#### Adopted findings

- **Sol 1 — major: CLOSED at classifier level.** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:830–841` rejects missing earlier timestamps, earlier returns at/after `first_at`, and stalled returns beyond `first_at + 100 ms`. However, the hardware observation path has the major problem below.
- **Sol 2 — minor: CLOSED.** `design/18-uhd-radio.md:351` no longer includes “from stage 3c-2a”; history remains in Changes.
- **O1 — minor: CLOSED under the authorized fallback.** `crates/ezsdr-radio/src/channels.rs:97–122` has no stream-membership mapping. `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1337–1338` retains channel-indexed rate keys with the required `shortcut:` comment. Multi-channel stream-key correctness remains deferred, not implemented.

#### New finding — major: OPEN — Time observations drain the queue being measured

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:945–948`, `:830–841`; `crates/ezsdr-radio-uhd/src/uhd.rs:826–831`.

**Problem:** After every timed apply, the probe calls `time_now()`. On UHD 4.10’s current radio implementation, this is a synchronous register read through the same radio control endpoint, not necessarily an independent motherboard-clock read.

**Evidence — VERIFIED from UHD 4.10 source:**

- [`multi_usrp_rfnoc.cpp:2701–2705`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L2701-L2705) delegates `get_time_now()` to the radio.
- [`radio_control_impl.cpp:388–404`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L388-L404) reads `REG_TIME_LO` through `regs().peek64()`; only old compatibility versions use the motherboard timekeeper.
- [`axis_ctrl_slave.v:201–242`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/core/axis_ctrl_slave.v#L201-L242) waits for each control operation’s acknowledgment before processing the next.

Thus, when the first timed apply returns quickly, its following clock read waits behind its pending timed writes. That read returns at/after the first command time. Subsequent commands are no longer accumulating ahead of execution: either no apply stalls, yielding `Incomplete`, or a later stall is rejected by the new earlier-return guard. A first apply that itself stalls can instead produce depth zero.

**Fix:** Obtain conservative pre-execution timing evidence without inserting synchronous reads into the measured FIFO—for example, an independently verified clock path or conservatively bounded host-clock observations. Add a deterministic device-path test where clock reads wait behind timed writes. Do not loosen the guards to accommodate this interference.

#### Adjacent sequences and mutations

- **Note — boundaries correct:** `measurement.rs:2895–2908` covers rejection of an earlier return exactly at `first_at`, acceptance of a stalled return exactly at `first_at` and exactly at `first_at + 100 ms`, and rejection beyond that bound.
- **Note — no stall:** `:820–823`, tested at `:2900`, leaves all-short/fast trials incomplete.
- **Note — cleanup:** `:970–981` waits through the last accepted target plus 10 ms, then attempts the untimed reset even if the wait fails. Wait/reset errors invalidate success.
- **Note — 100 ms plausibility:** Credit release after execution is source-supported; completion of apply plus clock read within 100 ms is **INFERRED**, not guaranteed. The bound alone does not inherently reject normal blocked returns; the interposed clock reads are the blocker.
- **Note — mutations:** P840–P843 target timing guards/boundaries; P844–P847 target interpretation labels. Each matches once and has a corresponding assertion. They do not cover the hardware observation-path problem.

Read-only review; no builds, tests, mutations, or hardware operations run. Reported execution results were not independently verified.

## 修正 2 の見直し（`sol-fix2-3c2a.log`）

VERDICT: PASS

- **Note — previous major: CLOSED.** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:934–945` brackets one initial device-time read with host timestamps. The measured loop (`:979–1003`) records host time only; no per-call `time_now()` remains. No further fix required.

- **Note — timing guards use conservative directions.** `measurement.rs:823–828` rounds the lower bound down and upper bound up. Earlier returns require **upper < first_at** (`:853–855`); the stalled return requires **lower ≥ first_at** and **upper ≤ first_at + 100 ms** (`:860–865`). An initial read exceeding 100 ms is incomplete (`:837–838`). Boundary assertions are at `:2941–2957`. These bounds remain conditional on the explicitly disclosed equal-clock-rate assumption (`:1096`), not hardware timing guarantees.

- **Note — remaining control operations do not establish another queue-draining defect.** The untimed setup apply remains after clock calibration (`measurement.rs:966`), but before any trial commands are queued. Timed `apply` includes command-time setup/clear and rate/frequency/gain getters (`crates/ezsdr-radio-uhd/src/uhd.rs:864–922`). **VERIFIED for the X300/OBX path:** UHD 4.10’s frequency/gain getters use properties or cached gain groups ([X300 source, lines 347–377 and 471–488](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L347-L377)); DDC/DUC frequency/rate getters likewise return stored properties. They are not equivalent to the removed synchronous clock-register read. I found no source-verified reason the classifier must reject every hardware trial.

- **Note — normal cleanup drains before reset.** `measurement.rs:1007–1018` waits through the last accepted target plus 10 ms, then performs the untimed reset; wait/reset errors invalidate success. UHD’s OBX phase-sync commands extend the target by roughly 400 μs ([source, lines 21–88](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_expert.cpp#L21-L88)), within that margin. Normal cleanup does not leave those commands queued into the next trial. Recovery after a device/transport failure is not a verified queue-flush guarantee.

- **Note — mutations match the revised code.** `plan/maintenance/tools/mutations.json:11505–11512,11745–11752,11825–11902`: all ten requested rows match their target exactly once. P808 covers missing host timestamps; P840–P843 cover the new timing guards; P832 and P844–P847 cover observation fields and interpretation labels, not timing guards. Corresponding assertions exist at `measurement.rs:2941–2957` and `tests/fake.rs:696–711`.

Read-only review. No builds, tests, mutations, or hardware operations run; reported execution results were not independently verified.
