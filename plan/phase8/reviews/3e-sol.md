# 段階 3e：Sol のレビュー

## 事前確認（指示書）

[0m
> plan · gpt-6.1-sol
[0m→ [0mRead /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/sol-pre-3e-brief.md 
> plan · gpt-6.1-sol
[0m→ [0mRead handoff.md 
[0m→ [0mSkill "ponytail-review" 
[0m→ [0mRead /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/impl-3e-brief.md 
> plan · gpt-6.1-sol
[0m→ [0mRead plan/phase8/reviews/bench-dry-run-1027.md 
[0m→ [0mRead plan/phase8/reviews/flaky-tests-1027.md 
[0m→ [0mRead plan/phase8/reviews/d9a-opus.md 
[0m✱ [0mGrep "classify_queue_trial|queue_max_calls|install_record_latch|set_sources|command_queue" [90min crates/ezsdr-radio-uhd/tests/common/measurement.rs · 22 matches[0m
> plan · gpt-6.1-sol
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/measurement.rs [90m[offset=840, limit=95][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/measurement.rs [90m[offset=1120, limit=180][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/mod.rs [90m[offset=290, limit=115][0m
[0m✱ [0mGrep "RM-32|UR-24|UR-34" [90min design · 73 matches[0m
[0m✱ [0mGrep "fn raw\(|install_record_latch|set_sources|check_rx|check_tx|get_rx_stream|get_tx_stream" [90min crates/ezsdr-radio-uhd · 53 matches[0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs [90m[offset=365, limit=125][0m
> plan · gpt-6.1-sol
[0m→ [0mRead plan/phase8/tools/run-record.sh [90m[offset=120, limit=90][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/mod.rs [90m[offset=398, limit=30][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs [90m[offset=1, limit=165][0m
[0m✱ [0mGrep "phase|timed|gain|FIFO|queue" [90min plan/phase8/research/d9-production-investigation.md · 55 matches[0m
[0m→ [0mRead crates/ezsdr-radio-uhd/src/uhd.rs [90m[offset=830, limit=42][0m
[0m% [0mWebFetch https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/host/lib/usrp/usrp_c.cpp 
> plan · gpt-6.1-sol
[0m⚙ [0mexecute {"code":"const base='https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/'; const files=['host/lib/usrp/dboard/db_ubx.cpp','host/lib/usrp/dboard/db_obx.cpp','fpga/usrp3/top/x300/rfnoc_image_core.yml','host/docs/sync.dox']; return await Promise.all(files.map(async p=>{const r=await fetch(base+p);const t=await r.text();const ls=t.split('\\n');let indexes=[];for(let i=0;i<ls.length;i++)if(/set_gain|CMD_FIFO|command.*queue|timed.*tune|phase.*align|phase.*sync|synchroniz/i.test(ls[i]))indexes.push(i);return {p,status:r.status,excerpts:indexes.slice(0,45).map(i=>ls.slice(Math.max(0,i-4),i+15).map((l,j)=>`${Math.max(0,i-4)+j+1}: ${l}`).join('\\n'))};}));"} 
> plan · gpt-6.1-sol
[0m⚙ [0mexecute {"code":"const base='https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/'; const files=['host/lib/usrp/dboard/db_ubx.cpp','host/lib/usrp/dboard/db_obx.cpp','fpga/usrp3/top/x300/x300_rfnoc_image_core.yml','host/docs/sync.dox.in','host/lib/usrp/dboard/obx/obx_expert.cpp']; return await Promise.all(files.map(async p=>{const r=await fetch(base+p);const ls=(await r.text()).split('\\n'); const ids=[];for(let i=0;i<ls.length;i++)if(/gain|include.*obx|fifo|sync_phase/.test(ls[i]))ids.push(i); return {p,status:r.status,excerpts:ids.slice(0,50).map(i=>ls.slice(Math.max(0,i-2),i+9).map((l,j)=>`${Math.max(0,i-2)+j+1}: ${l}`).join('\\n'))};}));"} 
> plan · gpt-6.1-sol
[0m→ [0mRead crates/ezsdr-radio-uhd/src/uhd.rs [90m[offset=885, limit=65][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/tests/hardware.rs [90m[offset=2410, limit=110][0m
[0m→ [0mRead crates/ezsdr-radio-uhd/src/authority.rs [90m[offset=350, limit=32][0m
[0m✱ [0mGrep "CMD_FIFO|32.*radio|radio.*32|queue.*depth|FIFO.*depth" [90min plan/phase8/research · 9 matches[0m
> plan · gpt-6.1-sol
[0m⚙ [0mexecute {"code":"const b='https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/';const ps=['host/lib/usrp/dboard/obx/obx_gpio_ctrl.cpp','fpga/usrp3/top/x300/rfnoc/blocks/radio.yml','host/include/uhd/rfnoc/blocks/radio.yml','host/lib/usrp/dboard/db_ubx.cpp','host/lib/rfnoc/ctrlport_endpoint.cpp'];return await Promise.all(ps.map(async p=>{let r=await fetch(b+p), ls=(await r.text()).split('\\n');let ids=[];ls.forEach((l,i)=>{if(/void.*write|write_gpio\\(|fifo_depth|payload|CMD_FIFO|set_gpio_out/.test(l))ids.push(i)});return {p,status:r.status,excerpts:ids.slice(0,15).map(i=>ls.slice(Math.max(i-2,0),i+20).map((l,j)=>`${Math.max(i-2,0)+j+1}: ${l}`).join('\\n'))}}));"} 
> plan · gpt-6.1-sol
[0m⚙ [0mexecute {"code":"const b='https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/'; const ps=['host/lib/usrp/x300/x300_impl.cpp','host/lib/usrp/x300/x300_dboard_iface.cpp'];return await Promise.all(ps.map(async p=>{let r=await fetch(b+p),ls=(await r.text()).split('\\n');let ids=[];ls.forEach((l,i)=>{if(/Reference Clock PLL failed|update_clock_source|set_gpio_out/.test(l))ids.push(i)});return {p,status:r.status,excerpts:ids.slice(0,10).map(i=>ls.slice(Math.max(0,i-5),i+45).map((l,j)=>`${Math.max(i-5,0)+j+1}: ${l}`).join('\\n'))}}));"} 
> plan · gpt-6.1-sol
**Verdict: CHANGE before implementation.** The main problems are queue-depth evidence, duplex coverage, and the phase probe.

Below, `tests/…` and `src/…` mean paths under `crates/ezsdr-radio-uhd/`.

### §2.1 — CHANGE: fix the scope contradiction

The proposed flaky-test fixes follow the investigation. Removing unrelated timing assertions is sound; moving the short-send test onto a controlled clock preserves its actual property.

However, §0 prohibits editing `src/` except §2.7, while §2.1 explicitly requires a unit test in `src/provider/tx.rs`. Allow **test-only edits** there and, if needed, in `control.rs`. Do not replace an on-time property with merely “something transmitted” without retaining its controlled-clock test.

**VERIFIED:** `plan/phase8/reviews/flaky-tests-1027.md:154–165,194–197`.

### §2.2 — CHANGE: an overlapping bound is not proof of queue blocking

The upper-bound rule establishes only that the return **might** have followed the target. It can accept a call that actually returned before the target. A slow host path or descheduling can also cross the stall threshold and return near the target without exhausting the queue.

**VERIFIED:** `tests/common/measurement.rs:865–909,1256–1281`: the observations are total host-call duration and an extrapolated device-time interval, not the reason for waiting.

Keep overlapping intervals as **ambiguous**, rather than promoting them directly to successful depth measurements. Repeated trials with different target delays and a consistent blocked-call index would strengthen the back-pressure inference; synthetic tests must include a slow/non-queue call and an interval straddling the target. Do not require the twelve historical samples to become successes.

**512 calls is reasonable, but not a measured guarantee.** UHD’s [radio.yml](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/rfnoc/blocks/radio.yml#L38) declares 512 control words; a timed one-word request occupies six payload words ([ctrlport_endpoint.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ctrlport_endpoint.cpp#L475-L481)). OBX gain changes write GPIO ([obx_expert.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_expert.cpp#L428-L447)); UBX does likewise. **INFERRED:** alternating gains should fill the queue well within 512 calls, provided they enqueue faster than the three-second deadline. Retain `unmeasured` otherwise.

### §2.3 — CHANGE: specify the layouts, not just “the rate”

Separating directional capacity measurements from duplex observations is reasonable. But RM-32 admits **both directional limits simultaneously**, including multi-stream/channel arrangements.

**VERIFIED:** `design/07-radio-model.md:314`; current duplex stages exercise channel 0 alone (`tests/common/measurement/multichannel.rs:421–426`).

A byte-rate limit does not identify one rate or layout. Construct and record a concrete simultaneous configuration that reaches each measured aggregate limit. Do not silently clamp a multi-channel capacity to channel 0’s maximum rate.

One 60-second pair is an acceptable **smoke observation**, not evidence that every admitted layout or longer run works. Record both directions’ per-stream failures and actual overlapping activity; an unsuccessful pair must explicitly flag the proposed limits as not jointly demonstrated.

### §2.4 / §2.6 — OK, with precise bookkeeping

Preserve earlier stages on a later error. The capacity’s condition should name a **successful condition at the reported capacity**, with the failed condition separately recorded as the blocker—not attach a higher-rate failed condition to a lower-rate value.

**VERIFIED:** `tests/common/measurement/multichannel.rs:451–481`.

### §2.5 — CHANGE: share the matcher, and test the whole failure path

Latching after PLL-only retries are exhausted is safe and appropriately conservative for this bench. Existing guards refuse later calls and leak the failed handle, avoiding teardown access.

**VERIFIED:** `tests/common/measurement/multichannel.rs:117–152`; `tests/hardware.rs:2490–2515`.

But raw `set_sources` returns UHD’s text, whereas `DeviceAuthority::new` translates it to `REFERENCE_DID_NOT_LOCK`. Passing raw errors unchanged to the existing retry helper will not match reliably.

**VERIFIED:** `src/authority.rs:363–369`; `tests/common/mod.rs:401–410`.

Normalize that error in test support. Test retry-success without latching, exhausted retries with latching, unrelated errors without retries, refusal before another opener, and preservation of the failed group plus later “not run” records.

### §2.7 — OK

UHD explicitly uses `UHD_SAFE_C_SAVE_ERROR(h_s, …)` for **both** stream-creation functions. The streamer handle is correct.

**VERIFIED:** UHD v4.10.0.0 [usrp_c.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/usrp_c.cpp), `uhd_usrp_get_rx_stream` / `uhd_usrp_get_tx_stream`. Add a stub-based regression assertion; clippy alone cannot verify the error source.

### §2.8 — CHANGE: control both LOs

Untimed TX plus untimed RX loopback measures their **relative** phase; it cannot attribute variation to RX or validate RX’s timed-tune guarantee.

For the timed comparison, retune **both directions at one future root instant**, with RX streaming. Ensure real LO programming occurs, allow settling, and estimate phase against the known waveform at timestamp-aligned sample positions. Reject gaps, weak signal and unstable phase as inconclusive. Explicitly handle D9a’s TX DSP limitation rather than assuming continuous-stream TX retuning works.

**VERIFIED:** `plan/phase8/research/d9-production-investigation.md:30,76`; UHD [OBX phase synchronization](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/obx/obx_expert.cpp#L21). Five Sessions provide an observation, not proof of a guarantee.

### §2.9 — OK

The tmpfs, persistent Cargo cache, diagnostic-only host readings, preflight and log-directory changes are sensible. Recalculate duration after removing duplex sweeps.

**Brief changes:** clarify test-only source edits; preserve ambiguous queue results; specify aggregate duplex layouts and limited claims; normalize raw PLL errors; redesign the phase comparison around both LOs.

Not checked: no builds, tests, Docker or hardware runs. Actual queue depth and phase repeatability remain bench questions.

## コードレビュー

> plan · gpt-6.1-sol
Two bench steps cannot collect their promised evidence as written: the TX-buffer message is logged at TRACE level, but the command requests DEBUG; and the throughput observation omits burst `blocks`, with no saved manifest fallback. I also found that the capacity-label test uses condition IDs unlike those generated by the real sweep, so it misses a labeling defect.
VERDICT: FAIL

## Failure scenarios checked

- A delayed test thread must not change a fake test’s verdict; moved tests must retain their original assertions.
- Queue intervals before, across and after the target must remain distinct; only successes contribute to the value. Check 512 calls, alternating delays and agreement reporting.
- Duplex must require both measured values, reuse their actual layouts/rates, record overlap and decide no value.
- Capacity labels must name a successful condition; blockers must remain separate.
- Exhausted PLL retries must latch the record and prevent later device access.
- Stage errors/panics must preserve earlier results.
- Stream-open errors must use streamer text without changing `lost` semantics.
- MA-1 must control both tunes, preserve RF limits and reject unusable phase evidence.
- Bench commands must run safely and retain the evidence the runbook promises.

## Findings

### 1. Major — DEBUG cannot expose the requested TX-buffer capacity

**Location:** `plan/phase8/bench.md:188–196`

The command sets `UHD_LOG_CONSOLE_LEVEL=debug`, but UHD 4.10 emits the exact requested message through **`UHD_LOG_TRACE`**. Consequently, the subsequent grep cannot obtain the capacity using the documented logging level.

**Evidence:** [UHD v4.10 `chdr_tx_data_xport.cpp:227–232`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/chdr_tx_data_xport.cpp#L227-L232) receives the capacity and logs `"Received strs initializing buffer capacity..."` at TRACE.

**Fix:** Request `trace`, and confirm that the image’s libuhd build includes TRACE logging. Document an explicit fallback if it does not.

### 2. Major — The burst `blocks` evidence is discarded before the operator can read it

**Locations:**  
- `plan/phase8/bench.md:198`
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs:3377–3403`

The runbook asks for each burst’s `blocks`, falling back to another log if the observation lacks it. However, the throughput code reads the burst rows into memory, emits selected fields **without `blocks`**, and neither saves nor prints those rows. The documented record/log outputs therefore provide no such fallback.

`burst_count` counts bursts, not their blocks; it cannot answer the requested question.

**Fix:** Include the burst rows’ `blocks` in the throughput observation before discarding the Manifest, and assert the field in the fake rehearsal. Replace the “unverified” instruction with the actual record field.

### 3. Major — Requested flaky-test remediation remains unfinished

**Location:** `crates/ezsdr-radio-uhd/tests/fake.rs:2339–2365`

`ur_21_an_untimed_send_is_on_time_after_its_delivery` still sends on the test thread and asserts both no timing errors and exactly 20 bursts. A delay between `submit`’s clock read and dispatch can legitimately drop a burst, failing both assertions.

The implementer explicitly reproduced this with the injected 6 ms delay (`report-3e.md:71–73`) but left it unchanged. Keeping its property is necessary; mutation dependencies are not a reason to leave the known test bug.

The cold-change test similarly retains a test-thread deadline. Approximately 48 ms of slack, and passing a 6 ms injection, do not make its verdict independent of scheduling.

**Fix:** Preserve the on-time properties in controlled-clock unit tests, as the brief requires. Repoint affected mutation rows rather than weakening the assertions.

### 4. Major — Capacity-label selection cannot identify the blocking family in real sweeps

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:458–461`

The new selection compares the successful stage’s **complete condition ID** with the failed stage’s ID. Real stage IDs include their rates (`rx-grouped-100m`, `rx-grouped-200m`, etc.; lines 407–419), so a lower successful stage never has the same ID as its higher failed successor.

For example, when grouped passes at 100 MS/s and fails at 200 MS/s, while another condition also passes at the lower aggregate capacity, the code retains the first condition instead of preferring grouped’s successful predecessor.

The new test at lines 983–998 hides this by reusing `"grouped"` at several rates—unlike the real generator. Thus P1003 can be killed while the intended real-world behavior remains absent. The statement in `design/18-uhd-radio.md:389` is not implemented.

**Fix:** Match a stable condition family/layout identity, then return the successful stage’s actual ID at T. Test with IDs produced by `throughput_stages`.

### 5. Minor — P1010 tests an error message, not the claimed no-run decision

**Location:** `plan/maintenance/tools/mutations.json:13507–13515`

P1010 is named “the pair of limits is run when a value is unmeasured,” but its replacement still returns `Err`; it merely empties the error message. Its killing assertion therefore proves reason reporting, not that an unmeasured value cannot lead to device activity.

**Fix:** Rename this as a reason-preservation mutation, and add a mutation that actually permits the unmeasured path to construct/run a pair. Kill that mutation with an unmeasured-value test asserting zero device calls.

### 6. Minor — The named weak-loop test never exercises weak-loop rejection

**Locations:**  
- `crates/ezsdr-radio-uhd/tests/fake.rs:3357–3383`
- `crates/ezsdr-radio-uhd/tests/common/mod.rs:874`

The test’s name promises that a weak loop becomes inconclusive, but it only verifies a readable gain of `0.02`, moving phase and a truncated input. It never observes the `gain < 0.002` rejection. P1016/P1017 cover phase calculation and steadiness, not weak-signal rejection.

**Fix:** Add a below-threshold case through the phase-verdict path and assert the inconclusive reason.

### 7. Minor — The new queue rule misdescribes its timing evidence and lacks DA-1 structure

**Location:** `design/18-uhd-radio.md:374`

The text describes “the host return bracketed by the two device-time reads.” The implementation instead extrapolates from **one device-time read bracketed by two host timestamps** (`measurement.rs:861–873`).

The paragraph also introduces normative classification rules but embeds its test names in prose rather than ending the topic with an indented `*Test:*` line. It omits the retained requirement for successful trials in every condition.

**Fix:** Describe the actual clock bracket and success-count requirement, and separate the test references into the required `*Test:*` line.

## Other checks

- The streamer-text change preserves `lost`: both `check_rx` and `check_tx` pass `streaming=true` to the same `check_with` used previously.
- The short-send unit-test move retains the transmitted-prefix, zero EOB, no-following-data and single-EOB assertions.
- Queue ambiguity remains excluded from values; delay agreement is observation-only.
- Duplex retains measured layouts/rates and decides no value.
- PLL retry/latch and stage error/panic handling have direct assertions.
- Both Docker arrays contain tmpfs and persistent `CARGO_HOME`; added host diagnostics are read-only.
- No prohibited production changes, MockRadio changes or `JOIN_BOUND` changes were found.

Not checked: no edits, builds, tests, mutation runs, Docker or hardware execution. Actual phase repeatability and queue depth remain bench questions.

## 見直し 1

> plan · gpt-6.1-sol
VERDICT: FAIL

## Finding

### 1. Major — TRACE build check still gives the wrong answer

**Location:** `plan/phase8/bench.md:188–197`

The runbook twice claims that `UHD_LOG_MIN_LEVEL=1` includes TRACE, and presents the reported image value `1` as confirmation. In UHD 4.10, TRACE is level **0**. Its macro is compiled only under:

```cpp
#if UHD_LOG_MIN_LEVEL < 1
```

At `1`, `UHD_LOG_TRACE` expands to nothing. Setting `UHD_LOG_CONSOLE_LEVEL=trace` cannot restore the requested buffer-capacity message.

**Evidence:** [UHD v4.10 log.hpp:200–206](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/include/uhd/utils/log.hpp#L200-L206). The capacity message itself remains a TRACE call in [chdr_tx_data_xport.cpp:231–232](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/chdr_tx_data_xport.cpp#L231-L232).

**Fix:** Require `UHD_LOG_MIN_LEVEL=0` for TRACE support. If the image reports `1`, explicitly identify TRACE as compiled out and use the documented source-default fallback, marked **not verified on hardware**. Remove the false confirmation. Rebuilding the image is unnecessary unless actual capacity evidence is required.

## Other six findings: fixed

- **Burst blocks:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:3290,3404` retains `burst_blocks`; `measurement/multichannel.rs:709` exposes it per stream. Fake rehearsals assert nonempty positive counts. P1021 removes the extracted evidence and would fail those assertions.
- **Untimed-send tests:** `crates/ezsdr-radio-uhd/src/provider/control.rs:2362–2439` exercises production `control.book` with controlled time. The 1/3/4 ms cases correctly distinguish the 5 ms target minus the 2 ms device lead. The cold-change cases use the actual 50 ms origin: 1 ms is on time; 49 ms is late. These test Provider booking, not Kernel stamping; the retained fake tests cover the latter without asserting an absence of legitimate lateness. J34’s new deadline assertion at `tests/fake.rs:2373` observes a pre-origin request even when the Provider moves it.
- **Capacity family:** `measurement/multichannel.rs:463–464` matches rate-free families and returns the successful stage’s actual ID. Tests at `:1001–1018` use generated grouped/split IDs and check the fallback. P1003 and P1018 break the respective assertions.
- **Unmeasured duplex:** P1010 is renamed. P1019 actually constructs limits on the unmeasured path. Tests at `measurement/multichannel.rs:965–973` reject that path and assert `ran: false` and zero device calls.
- **Weak phase:** `tests/common/tone.rs:133–136` supplies the verdict used by MA-1. `tests/fake.rs:3385–3390` checks readable, weak and moving-phase results; P1020 bypasses weak rejection.
- **Queue specification:** `design/18-uhd-radio.md:374–375` now describes one device-time read bracketed by host timestamps, retains the per-condition success requirement, and separates the `*Test:*` line.

Production changes under `src/` remain limited to `uhd.rs`’s two streamer-error calls; the other additions are inside test modules. Both calls preserve the previous `lost` classification.

Not checked: no edits, builds, tests, mutation runs, Docker or hardware execution. Image contents and reported run results were not independently verified.

## 見直し 2

> plan · gpt-6.1-sol
VERDICT: PASS

No findings.

- `plan/phase8/bench.md:188` correctly states that UHD 4.10’s capacity message uses `UHD_LOG_TRACE`, compiled out when `UHD_LOG_MIN_LEVEL=1`. Runtime console settings cannot restore it.
- The source-default estimate, 262144 bytes, is consistent with the X300 HG configuration: 32768 words × 8 bytes. The paragraph explicitly marks it **not verified on hardware**.
- Checked all of `bench.md`: no removed TRACE benchmark command or instruction remains; the only TRACE reference explains why it will not run.

Not checked: image contents independently; the reported build setting is accepted from the orchestrator. No edits, builds, Docker, tests or hardware execution.
