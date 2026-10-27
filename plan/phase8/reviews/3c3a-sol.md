# 段階 3c-3a：Sol 6.1 のレビュー

## 事前確認（指示書）
## Verdict: CHANGE before implementation

The measurements are worthwhile, but the brief currently overstates capacity evidence, misses mixed-board prerequisites, and requests H4 timestamps that the probe does not record.

### §2.1 — CHANGE: selector propagation is necessary, but insufficient

**VERIFIED:** `bench_profile` overwrites supplied RX/TX layouts (`tests/common/mod.rs:140–145`). Preserve explicit overrides, validate their JSON shape before opening, and propagate them through every Provider group and raw probe.

**VERIFIED:** mixed-board support also requires frequency and safety changes:
- `board_of` accepts only UBX/OBX/CBX; Basic/LF fails (`tests/common/mod.rs:83–85`).
- `bench_hz` chooses slot A’s frequency for the experiment (`:127–135`).
- CBX starts at **1.2 GHz**, outside the present approximately 1 GHz TX safety band (`crates/ezsdr-radio/src/x3xx.rs:144`; `tests/common/mod.rs:46`).

**Concrete change:** use channel-specific settings from the selected Description. Keep mixed-board TX and Basic/LF procedures disabled until the owner approves appropriate frequency, wiring and RF-envelope procedures. A selector override alone does not make those benches executable.

### §2.2 — CHANGE: distinguish link allocation from stream grouping

**VERIFIED, UHD 4.10:** link selection occurs per channel connection, separately balancing historical RX/TX allocation counts—not byte rates. Counters increment without decrementing. See [graph_stream_manager.cpp:188–219,252–284](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/graph_stream_manager.cpp#L188).

Consequently:
- HG with one connected 10 GbE link puts all channels on it.
- XG plus `second_addr` can distribute **one two-channel streamer** across both links; two streamers are not required.
- Opening/closing earlier streamers can change subsequent assignments. Record opening order and per-NIC traffic; do not equate stream number with physical link.

**VERIFIED:** different DDC rates on distinct channels are legal: rates/decimations are per port ([ddc_block_control.cpp:169–177,214 onward](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ddc_block_control.cpp#L169)). One `multi_usrp` streamer harmonizes its channels’ rates, so condition (c) correctly needs separate streamers ([multi_usrp_rfnoc.cpp:357–404](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L357)).

**VERIFIED:** the Module supports multiple streams and port-specific links (`src/provider/mod.rs:489–554`). Existing measurement helpers do not: `receive_spec` asserts one channel and profile placements feed only `rx0` (`tests/common/mod.rs:181–215`).

**Concrete change:** explicitly require per-stream specs, sinks/placements on every active `rx<k>`, TX actions/waveforms per stream, and per-stream completion evidence. Aggregate counters must not hide a starved channel.

**Capacity rule: CHANGE.** “No failure observed below T” permits untested/admission-rejected conditions to inflate m. That is incompatible with `p ≤ m` and P3’s “sustained in all trials” (`41-measurement-records.md:31–34`; overview:84).

Require successful, complete evidence for every applicable condition family. Missing coverage yields `unmeasured`, not a pass. Preserve rejection observations without treating them as hardware failures. Define handling of unequal candidate grids explicitly; do not use the first failed T as a measured capacity.

### §2.3 — OK to add the builder; CHANGE its interpretation

**VERIFIED:** the existing timing builder changes timing only (`src/provider/mod.rs:367–373`). Assembly’s adjustment closure can call public builders but cannot modify private Description state. Selector/profile edits cannot override performance, which `Description::new` derives from `x3xx::performance` (`channels.rs:348–364`).

RM-32 is enforced through Description coercion/violations, including prepare—not merely an editable Kernel capability (`channels.rs:599–605,721–724,817–848`; `provider/mod.rs:470–472`). Therefore a small performance builder updating the internal Description and published surfaces is justified.

**VERIFIED:** wire format is sc16, four bytes/sample (`src/uhd.rs:948–953`; `x3xx.rs:407–412`).

**Concrete change:** describe 1,250 MB/s as an **admission probe ceiling**, not achievable sample payload. Ethernet/protocol overhead reduces usable bandwidth. For two channels on the 200e6/n grid, 200+100 MS/s gives 1,200 MB/s; 200+200 gives 1,600 and is excluded. Thus the proposed ceiling exposes a useful 1,200 MB/s candidate, not a precise link-capacity measurement. Keep the accepted one-link aggregate restriction.

### §2.4 — OK for OBX duplex; CHANGE success evidence

**VERIFIED:** RM-31 requires RX2 when the same duplex board’s TX channel belongs to a stream (`channels.rs:768–812`). RM-32 budgets directions independently; 200 MS/s sc16 is 800 MB/s **each direction**, not 1,600 MB/s competing for one unidirectional budget.

**INFERRED:** simultaneous 200/200 MS/s is a legitimate trial, not a guaranteed sustained operating point. The current bench already observed TX underflows at 100 and 200 MS/s (`bench-results.md:111`).

**VERIFIED:** cold configure uses untimed settings (`provider/core.rs:1079–1087`); gain updates touch radio rather than DDC/DUC (note 39:13–15; `uhd.rs:875–894`). These procedures need not trigger F1–F5 if they avoid timed frequency updates. Radio-queue interference such as F6 remains measurable.

**Concrete change:** prove the companion direction stays streaming throughout each trial. Explicitly retain the existing TX-start limitation: it observes configuration completion, **not an on-time transmitted burst** (`measurement.rs:2489–2522`). Do not rename that evidence into a stronger guarantee.

### §2.5 — OK: retain genuine in-process reopen

**VERIFIED:** UHD has a weak device cache, releases X300 claims on destruction, and shuts down the RFNoC graph ([device.cpp:226–233](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/device.cpp#L226), [x300_impl.cpp:409–424](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_impl.cpp#L409), [rfnoc_graph.cpp:122–129](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_graph.cpp#L122)).

Phase 7 demonstrated same-process reopen, including recovery after loss (`plan/phase7/bench-results.md:1602–1616`). No blanket prohibition is supported.

**Concrete change:** drop every Session, Authority, probe and device clone before reopening; require completed teardown and no operational/control failure or detached owner. `DeviceLease` currently exposes no clean query (`measurement.rs:1843–1884`). Distinguish expected candidate failures from unsafe handle failures. A second process is a useful separate baseline, **not** equivalent to in-process reopen.

### §2.6 — CHANGE: load is useful; observability is missing

**VERIFIED:** Module threads use ordinary Rust spawning without priority elevation (`src/host.rs:383–389`). Relevant UHD control workers also spawn normally; X300 defaults disable RX/TX offload ([chdr_ctrl_endpoint.cpp:38–42](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/chdr_ctrl_endpoint.cpp#L38), [x300_mb_iface.cpp:20–27](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_mb_iface.cpp#L20)). SYS_NICE/rtprio permissions do not themselves activate realtime scheduling.

**Concrete change:** retain nice-0 CPU/memory contention, but record actual thread policies, affinities, CPU utilization and bounded memory allocation. Do not add realtime busy loops.

**VERIFIED:** `ProbeDevice::apply` records no timestamped row; `time_now` records no value/bounds; continuation TX sends may have `at=None` (`probe_device.rs:233–276,339–351`; `provider/tx.rs:442`).

Simplest observable: add test-only timed-apply entry rows carrying `e`, and compute release-to-call lateness using the known Description release window and bounded host/device anchors. Reconstruct TX sample headroom from burst timestamps and accepted per-channel counts. Label these **dispatch/headroom proxies**, not exact scheduler wake-up latency; exact wake-up latency needs scheduler tracing.

## Required brief additions

- Close one streamer before regrouping its channels; overlapping ownership can corrupt UHD’s graph and abort (`bench-results.md:71–76`).
- Cover companion failure, partial sends, missing ACKs, one-channel starvation, dirty-reopen skip and selector/layout mismatch.
- Require verified attenuation on **every TX board**; slot-B CBX was uncabled (`bench-results.md:15`).
- Stop after control-buffer timeout; never continue reset/coercion on the wedged handle (`bench.md:93`).

No files changed; no tests, Docker or hardware run. Sustained duplex rates and actual scheduling policies remain bench checks.

## コードレビュー
VERDICT: FAIL

## Failure scenarios checked

- A selector reaches Provider measurements but not raw probes.
- One stream starves while aggregate traffic appears sufficient.
- Failed, rejected, missing or unequal-grid stages inflate capacity.
- A candidate ceiling changes ordinary construction.
- A previous Run leaves streamers open before the next condition.
- Companion traffic stops during trials but Run-wide counters still pass.
- A control timeout or failed `apply` is followed by further device access.
- Mixed-board RF settings, attenuation confirmation and fake-record provenance disagree with the recorded configuration.

## Findings

### 1. Major — Companion success does not prove traffic throughout the trials

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:594–639`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2653–2654`.

`companion_verdict` compares **Run-wide** samples against 90% of a narrower window. This includes samples before the window and during shutdown. It also checks only underflow/overflow events, not complete transmission evidence or other async failures.

The reported limitation is not confined to short rehearsals. For a two-second window, 1 M pre-window samples plus 17 M in-window samples passes the 18 M threshold despite 300 ms without companion reception. An aggregate threshold also cannot establish that every trial had companion traffic.

These verdicts can qualify lead candidates as successful without the required concurrent evidence. The shortcut comment does not make that measurement valid.

**Fix:** Collect window-attributed, per-stream evidence covering each trial, including continuity and relevant async failures. Otherwise retain the companion result as an observation and leave the affected value unmeasured. Add deterministic cases for traffic only before the window and a companion stopping during one trial.

### 2. Major — The “nothing more reaches a wedged handle” guarantee is incomplete

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:425–442, 497–503`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2347–2348, 2393–2394, 3046–3054`.

The detector recognizes only the literal `Control operation timed out`, and normally runs **after** `run.finish()`. Non-timeout `apply` errors do not latch it.

The first two Provider groups never call `check_control` after finishing their Runs. The one-channel throughput rejection branch also continues without inspecting the failed Run for a timeout. These paths can start another candidate or group on the same handle.

Open-stream detection exists only in `run_stage`; the preceding one-channel sweeps and companion Sessions do not use its counting wrapper. Their detached-worker/open-stream failures therefore do not get the same protection.

**Fix:** Centralize failure tracking at the measurement Device boundary across all Provider groups and completion/error exits. Prevent subsequent forwarded calls once the handle is unsafe. Test the first-group timeout, prepare-time `apply` failure, and leftover-streamer sequences using a call log—not merely the wedge flag.

### 3. Major — Raw probes still ignore the requested layout

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:320–340, 2155–2172`; `plan/phase8/bench.md:51`.

Installing the thread-local selector affects profiles, not `raw_rx`/`raw_tx`: those still operate device channel 0 at fixed frequencies. The documentation now requires slot A to be UBX or OBX instead of implementing the brief’s selector-aware raw path and Basic/LF support.

A selector naming slot B alone still probes—and can transmit through—slot A. A requested CBX- or Basic/LF-first configuration cannot complete this suite as specified. Adding slot A to the attenuation confirmation mitigates accidental unconfirmed transmission, but does not make the evidence match the requested layout.

**Fix:** Resolve raw probe channels/settings from the selected layout. Where a probe genuinely cannot apply, record it explicitly as inapplicable rather than silently probing another board.

### 4. Major — Default runs omit an applicable capacity condition

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:721, 826–830`; `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:245–252`.

Duplex conditions are disabled by default. A normal record can therefore publish measured throughput without testing the applicable simultaneous RX/TX condition required by §2.4 of the brief.

For example, simplex 200 MS/s can succeed while concurrent operation underflows; without the flag, that limiting condition never participates in capacity.

**Fix:** Include duplex conditions by default where both directions exist. If simplex-only measurement is deliberately supported, its result must not stand in for the requested all-condition capacity.

### 5. Minor — New accepted-spec prose is not DA-1 shaped

**Location:** `design/18-uhd-radio.md:370–373`.

The added text starts with implementation history (“Stage 3c-3a”) and combines selector, RF settings, traffic evidence, lifecycle, capacity and NIC observations into one block. “Running throughout” and the post-wedge guarantee are also stronger than the implementation.

**Fix:** Keep concise normative topics ending in their test lines; move stage history and rationale to the Changes/decision record, and correct the guarantees alongside findings 1–2.

## Tests and mutation review

The per-stream starvation, partial-send and missing-ACK tests inspect useful evidence. Capacity’s pure tests cover successful prefixes, unequal grids, rejection and explicit missing stages. Fake record rehearsals correctly retain `Source::Fake` and verify its rejection.

P913–P947 target real branches or data transformations. However, they do not establish completeness of the measurement boundary. Missing mutation coverage includes hardware-level wedge skipping, `check_control` call sites, actual companion-window extraction, TX companion-condition insertion, `tx_at_channel_zero_frequency`, and passing the candidate ceiling into `run_stage`.

P931–P933 test companion arithmetic and propagation, not whether the samples belong to the claimed window. The companion integration test’s unconditional `Success` assertions do not exercise a companion stopping during that window.

No new test-thread deadline dependency was established by inspection; that is not confirmation from execution.

## Disposition of the seven deviations

1. **Ascending stage order — accept.** Capacity sorts by totals; ascending order can preserve lower-stage evidence before a wedge.
2. **Opt-in duplex — reject the default omission.** Separate RX/TX executions are acceptable, though redundant; missing applicable conditions is not.
3. **RX tolerance of one request/block — accept.** Appropriate for whole-request counting at the cut; retain explicit recorded tolerance.
4. **One ACK per channel — accept.** Consistent with Provider bookkeeping at `src/provider/tx.rs:266–285`; missing-channel coverage is present.
5. **Wedge latch and capacity truncation — accept the policy, reject completeness claims.** Finding 2 remains.
6. **Candidate ceiling only for added stages — accept.** Preserves the required existing one-channel path. Ordinary construction is unchanged.
7. **Fixed slot-A raw probes — reject.** This removes explicitly requested configurations rather than implementing selector propagation.

Scope otherwise stays controlled: the only production change is the measurement performance builder; no profile values, H4/reopen implementation or new FakeDevice fault kinds were added.

**Not checked:** Builds, tests, mutation execution and hardware, as instructed. The measurement results must not be accepted as complete concurrent-capacity evidence until the major findings are resolved.

## 見直し 1
VERDICT: FAIL

### 1. Major — The guard does not cover opening, teardown or `describe()`

**Locations:** `crates/ezsdr-radio-uhd/tests/hardware.rs:27,2174–2196`; `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:83–87,128,189–192`.

- `usrp()` evaluates `open()` **before** `guard_if_recording` checks the latch. Later probes therefore still open hardware after an earlier probe latches.
- Record setup independently calls `open()` without checking the latch. Because setup follows the raw probes, a raw-probe failure can also make `serial()` fail and prevent the final record from being assembled.
- Raw-probe guards have no failed-handle retention. When a probe unwinds after an apply error or timeout, dropping its inner `UhdDevice` can read device time and perform teardown (`crates/ezsdr-radio-uhd/src/uhd.rs:642–658`).
- `describe()` is not device-free: UHD reads its printable description and channel counts (`crates/ezsdr-radio-uhd/src/uhd.rs:663–683`).

**Fix:** Check the latch before invoking an opener; retain unsafe probe handles without teardown; cache description metadata. Capture the record’s identity before probes so a probe failure cannot prevent record writing. Test actual opener and destructor calls, including unwind.

### 2. Major — Leftover streamers are detected too late

**Locations:** `crates/ezsdr-radio-uhd/tests/hardware.rs:2202–2209`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2453–2454,3042–3047`.

`latch_if_streamers_open` runs only after an entire Provider group. Each lead or one-channel throughput group opens several successive Runs.

If one Run finishes with a detached worker and an open streamer, the next candidate can configure/open the same front end before the group-level check happens. The extra-throughput path stops on its own leftover-streamer evidence, but these earlier loops do not.

**Fix:** Check streamer ownership after every Run, including rejection, error and unwind exits, before another candidate begins. Add a two-candidate test proving that a first Run leaving a streamer open prevents the second from reaching the device.

### 3. Major — Observation-only companions still erase solo measurements

**Locations:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2454–2456,2509–2515,2606–2607,2959–2971`; `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2087–2094`.

The reported open item is **not acceptable**. Solo results remain local until all companion conditions finish. A companion setup error, trial error or panic causes `GroupRun` to discard the group and replace its lead value with `unmeasured`.

For example, failure to admit `start_companion_tx` loses already-completed receive-lead measurements. Overflow companion failure likewise discards the solo observations.

**Fix:** Isolate companion execution from the completed solo group. Record companion failures as observations while preserving solo results; still honor the safety latch for subsequent access. Skip unavailable opposite directions explicitly. Test companion failure after successful solo measurement.

### 4. Major — Attenuator confirmation misses preflight transmitters

**Locations:** `plan/phase8/tools/run-record.sh:57–73,128–131,192–193`.

With a selector naming only slot B, the prompt names slot B and omits slot A because the record’s raw probes are inapplicable. Optional preflight tests, however, run in `CONTAINER`, without that selector. A transmitting preflight such as `hw_b8_raw_device_lead` still drives slot A.

Thus the script can transmit through a board the user was not asked to confirm.

**Fix:** Include preflight transmitters in the confirmation. The smallest safe policy is to include slot A whenever preflight tests are requested, alongside the selector’s transmit channels. Test the slot-B-selector plus slot-A-preflight case.

### 5. Major — Newly mandatory duplex stages exceed the documented waveform amplitude

**Locations:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:580–581`; `crates/ezsdr-radio-uhd/tests/common/mod.rs:569–575`; `plan/phase8/bench.md:8`.

Duplex stages transmit `pn()`, whose I and Q components are each ±0.4. Its complex magnitude is approximately **0.566**, exceeding the bench limit of 0.5. This transmission now happens by default even in the receive-throughput group.

**Fix:** Use a waveform whose complex magnitude is at most 0.5 and assert that bound before transmission. Apply the same correction to the added companion transmissions.

### 6. Minor — Deleted RX mutation rows leave a testable guard path uncovered

**Locations:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:151–156,832–845`; `crates/ezsdr-radio-uhd/tests/fake.rs:7924–7931`.

The timeout test calls `guard.note()` directly. It does not prove that a forwarded `rx_recv` timeout/loss latches. Deleting P952/P953 because FakeDevice cannot produce those outcomes is insufficient: `ProbeDevice::script_rx` already supports scripted receive results.

**Fix:** Wrap a scripted ProbeDevice, return `RxRecv::Failed` with timeout and loss cases, verify subsequent calls are blocked, and restore mutation coverage.

**Resolved:** Companion counters no longer qualify values; raw-probe applicability is explicit; duplex stages are included by default. RX2 defaults satisfy RM-31. UR-5’s corrected default matches the parser, and the added UR-34 topics are substantially DA-1 shaped, but its lifecycle guarantees remain false.

**Not checked:** Builds, tests, mutations or hardware execution, as instructed. No files changed.

## 見直し 2
VERDICT: FAIL

### 1. Major — The new reopen guard stops a normal raw probe and skips all Provider measurements

**Locations:** `crates/ezsdr-radio-uhd/tests/hardware.rs:470–491`; `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:122–126`.

`hw_b8_raw_restart_lead` opens RX stream 0, then deliberately calls `rx_open(0, …)` again in its `reopen` cases without first calling `rx_close(0)`. The new guard treats this normal probe sequence as a leftover streamer and latches.

Consequently, an otherwise healthy, applicable record run fails during its second raw probe. Subsequent probes cannot open devices, and the post-probe reopen refuses every Provider group. UHD previously supported this replacement by removing the old stream before creating its successor (`crates/ezsdr-radio-uhd/src/uhd.rs:954–958`).

**Fix:** Keep the guard’s refusal policy, but explicitly close the stopped, drained streamer before this probe reopens it. Add a deterministic guarded rehearsal of the actual stop → drain → close → reopen sequence, proving that it does not latch while an unclosed reopen still does.

### 2. Major — The attenuation prompt still misses inherited transmitters

**Locations:** `plan/phase8/tools/run-record.sh:64–72`; `crates/ezsdr-radio-uhd/tests/common/mod.rs:152–160`.

The preflight-specific omission is fixed, but an incomplete selector still bypasses confirmation for a transmitting board.

For example, with `{"rx":["0:B:cbx:0"]}` and no preflight tests, the script defaults `tx` to an empty list, marks raw probes inapplicable, and prints transmitting channels as **“none.”** The Rust layout instead inherits the device’s default TX layout when `tx` is omitted. With OBX/UBX in slot A, later TX measurements can therefore transmit through slot A without its path having been named for confirmation.

**Fix:** Include inherited transmitters in the confirmation, or reject an incomplete selector before device access when the script cannot determine them safely. Cover the RX-slot-B / omitted-TX / no-preflight case.

### Resolution of the previous six findings

- **Opening, teardown and description:** Resolved by latch-checking openers, cached metadata, latched-handle retention, and identity capture before probes.
- **Leftover-stream protection:** The requested refusal exists, but breaks the legitimate probe sequence above.
- **Companion failures erasing solo values:** Resolved. Companions are separate, value-less groups after the value-producing groups.
- **Preflight attenuation confirmation:** The named preflight case is resolved; inherited TX remains unsafe as above.
- **Added waveform magnitude:** Resolved. Added duplex/companion waveforms are scaled and checked against 0.5.
- **Forwarded receive failure coverage:** Resolved by scripted timeout/loss cases and mutation rows P960/P961.

**Not checked:** Builds, tests, mutation execution or hardware, as instructed. No files changed; the healthy raw-probe path and omitted-TX confirmation remain blockers.

## 見直し 3
VERDICT: PASS

No findings within the two fixes and their neighbourhood.

- **Channel guard:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:124–137` refuses overlapping channels held by a **different stream in the same direction**, before forwarding. Same-stream replacement, disjoint channel sets and opposite directions remain allowed. Empty lists map to channel 0 for both RX and TX, matching `src/uhd.rs:948–968,1044–1060`.

- **Legitimate sequences:** Reviewed every `rx_open`/`tx_open` occurrence in `tests/hardware.rs` and the measurement code. The restart probe’s same-stream reopen (`tests/hardware.rs:470–491`), close-before-transfer probes (`:1401–1402,1446–1447`), queue trials and Provider grouped/split/duplex conditions remain compatible. The different-stream overlap refusal matches the abort case recorded in `plan/phase8/bench-results.md:73`.

- **Incomplete selectors:** `tests/common/mod.rs:126–127` rejects either direction supplied alone; metadata validation runs before hardware setup (`tests/hardware.rs:2147,2177`). `plan/phase8/tools/run-record.sh:68–79` likewise exits 2 before the attenuation prompt, Docker or device access.

- **Tests and documentation:** Regression assertions cover overlap refusal, empty-list aliasing, same-stream replacement and disjoint channels (`tests/fake.rs:8104–8141`), plus incomplete-selector rejection (`:7796–7813`). UR-34 (`design/18-uhd-radio.md:370,382`) and BP4b (`plan/phase8/bench.md:51`) accurately describe these fixes.

Not checked: builds, tests, mutations or hardware execution, as instructed. No files changed.
