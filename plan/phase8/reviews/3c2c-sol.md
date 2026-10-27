# 段階 3c-2c：Sol 6.1 のレビュー

## 実機の詰まりの調査（`sol-hang.log`）

#### 1. When do DDC/DUC timed commands execute?

**VERIFIED:** Against passing samples’ packet-derived time, not an independently advancing timekeeper.

- `fpga/usrp3/lib/rfnoc/axi_tag_time.v:114–131` loads time from the packet header and advances it on sample transfers. Lines **206–211** dequeue a timed setting only during a sample transfer whose time reaches the setting’s timestamp.
- DDC connects this mechanism at `fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_ddc/rfnoc_block_ddc.v:298–319`. When its timed FIFO fills, `rb_stb = ~timed_cmd_fifo_full` stops acknowledging further control operations.
- DUC also uses `axi_tag_time`, through `fpga/usrp3/lib/rfnoc/dds_timed.v:135–171`, with queued frequency values at **288–305**. However, DUC’s control acknowledgement is tied high (`rfnoc_block_duc.v:224`): **do not assume its overflow/back-pressure behaviour equals DDC’s.**

Thus, without RX samples, timed DDC frequency settings remain queued; wall-clock waiting cannot drain them. UHD explicitly documents this HG/XG lockup and the possible need for a full restart: `host/docs/devices/usrp_x3x0.dox:861–863`.

#### 2. What does `apply` send, and why did B8 pass?

**VERIFIED:** `crates/ezsdr-radio-uhd/src/uhd.rs:857–944` performs:

1. `uhd_usrp_set_command_time`;
2. `uhd_usrp_set_{rx,tx}_freq` with AUTO RF/AUTO DSP, or `set_{rx,tx}_gain`;
3. `uhd_usrp_clear_command_time`, including after setter failure;
4. rate/frequency/gain getters on success.

UHD distributes command time to radio and DDC/DUC (`host/lib/usrp/multi_usrp_rfnoc.cpp:964–987`). AUTO tuning sets RF, reads its achieved value, then sets the compensating DSP frequency—even when that compensation is zero (**626–728**, **1394–1404**, **2067–2077**).

OBX LO programming uses SPI (`host/lib/usrp/dboard/obx/obx_cpld_ctrl.cpp:94–101`), timestamped through the radio (`host/lib/usrp/x300/x300_radio_control.cpp:146–151`). Gain uses radio GPIO, not DSP (`obx/obx_expert.cpp:428–444`; `multi_usrp_rfnoc.cpp:1583–1587,2117–2121`).

Register identities are **VERIFIED**:
- **0x420:** DDC/DUC frequency, `132 × 8`.
- **0x430:** DDC decimation, `134 × 8`, not gain.
  
Sources: `host/lib/rfnoc/ddc_block_control.cpp:44–52,587,659–660`; `duc_block_control.cpp:45–51`.

**VERIFIED differences:** B8 submits eight tunes per direction, once, then sleeps and performs an untimed reset (`hardware.rs:523–549`). New code repeats five trials per condition, up to 64 calls each, including separate RX/TX frequency, gain, and alternating RX/TX conditions (`tests/common/measurement.rs:682–684,899–911,1076–1087`). Both use a 3-second horizon and 1-ms spacing. The new setup tune is **untimed TX**; it does not drain DDC.

**INFERRED:** B8 hit radio-side back-pressure before exhausting DSP storage; the radio executes against `radio_time` without samples (`rfnoc_block_radio.v:466–472`). Repeated trials accumulate DSP entries across waits/resets. Exact occupancy at the first timeout is unobservable here. Gain is not the initiating DSP write; alternating directions only distributes accumulation. Crucially, B8 also passed **in this same run** (`s2.txt:74–79`).

#### 3. Why did timeouts not end the run?

**VERIFIED:** `host/lib/rfnoc/ctrlport_endpoint.cpp:541–579` waits **10 seconds** when outstanding requests include timed commands, then throws `uhd::op_timeout`. For the logged **“waiting for space”** failure, the new request has **not been sent**; previously queued commands remain. Clearing command time does not cancel them.

The C wrapper returns **`UHD_ERROR_EXCEPT` (47)** (`host/include/uhd/exception.hpp:229–250`; `host/lib/error_c.cpp:18–34`), which Rust converts into `DeviceError`.

But the measurement treats failures as observations:

- A failed apply stops only that trial’s submission loop.
- The “drain” checks motherboard time, not DSP occupancy.
- An untimed frequency reset then hits the same blocked DDC.
- Outer loops continue after `DeviceFailure`.
- `measure_coercion` follows; numeric errors are collected rather than aborting, including the rate sweep.

Evidence: `measurement.rs:999–1018,1079–1083,1129–1155,1370–1385`; `hardware.rs:2208–2215`.

**INFERRED:** Two approximately 10-second failures plus trial overhead explain the advancing ~23-second timestamps. Later **0x430** failures (`s2.txt:264–266`) indicate progression into rate coercion—not one infinitely blocked call.

#### 4. Recommendation

**Measure full AUTO/AUTO applies with the relevant chains actively streaming and continuously drained:** RX for DDC; timestamped TX samples for DUC, within RF safety limits. Establish streaming before submitting commands and maintain it beyond the last scheduled timestamp.

A radio-only probe using `dsp_freq_policy=NONE` is simpler, but measures **radio-only capacity**, not current `Device::apply` capacity.

Add a **separate-process deadline**, with forced termination, covering probe and teardown. On the first control timeout, persist diagnostics and abort the entire raw group: no resets, further trials, or coercion against that handle. Duration checks after `apply` returns are not watchdogs.

No files, tests, or hardware were changed/run. Streaming recovery after an already wedged port was not verified; restart may be required.

## コードのレビュー（`sol-3c2c.log`）

VERDICT: FAIL

### Scenarios reviewed

- RX starts late, stops producing samples, falls behind, or never reaches the last timed setting.
- An `apply`, clock read, receive call or cleanup operation fails; later calls still reach the handle.
- The command thread panics while the drain thread remains alive.
- The raw-group deadline expires during a trial.
- A group fails, but record assembly subsequently touches the failed device.
- A non-PLL error succeeds on retry and disappears from the result.
- Refused Runs, frequency-grid bounds/tolerance/step, and FakeDevice measurement claims.

### Findings

#### 1. Major — Calling `rx_recv` does not establish that the DDC queue drained

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1019–1036,1131–1141`

The drain thread ignores everything except `RxRecv::Failed`: samples and their timestamps, `LateCommand`, repeated timeouts, overflow and alignment errors. Completion waits for **motherboard time**, not samples passing the final setting’s timestamp.

For example, a late RX start produces `LateCommand` followed by timeouts. The command loop can still classify radio back-pressure as success, wait for motherboard time, reset and close the receiver while DDC settings remain queued. This preserves the mechanism behind the original hang.

The test at `crates/ezsdr-radio-uhd/tests/fake.rs:963–977` checks call counts, not successful samples covering the commands. P868/P869 therefore prove only that the receive branch and loop execute. Its `rx_recv > 0` assertion also depends on scheduling: a drain thread first scheduled after `stop=true` makes no receive call.

**Fix:** Establish successful reception before submitting settings. Track sample-time progress through the final command, reject missing/invalid progress, and test delayed starts, stalled reception and incomplete spans with controlled synchronization.

#### 2. Major — “Nothing more after the first failure” is not true

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1019–1036,1638–1648`

There are several remaining paths:

- The drain thread continues calling `rx_recv` after the command thread’s failed `apply`; it does not share the failure latch.
- A failed `rx_stop` is immediately followed by `rx_close`.
- A failed restoration `apply` is followed by restoration of later channels.

The new control log omits receive calls and close operations, so the first-failure tests cannot detect all these paths. Final device destruction also calls `answers()` and potentially teardown after a raw failure (`crates/ezsdr-radio-uhd/src/uhd.rs:633–658`).

Consequently, the unconditional first-failure statement in `design/18-uhd-radio.md:362` is false.

**Fix:** Apply one failure policy across command, drain and cleanup paths. Stop initiating calls after failure, distinguish already-in-flight calls, and explicitly handle failed-device destruction. Extend the operation log and tests to cover those paths.

#### 3. Major — A command-thread panic can prevent the record from ever being written

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1018–1030`

If `queue_trial_calls` panics, execution skips `stop.store(true, ...)`. The scoped-thread destructor then waits for the drain thread, whose loop can continue forever. The outer `catch_unwind` never receives the panic, and neither the restore guard nor the final record writer is reached.

`raw_group_restores_after_a_panic_in_it` tests the opposite direction—a panic **inside the drain thread**—so it misses this case.

**Fix:** Make drain cancellation run during unwinding, before the scope joins. Add a test Device that panics in a timed `apply` while reception remains active, and verify group failure plus record continuation.

#### 4. Major — The raw-group deadline is not checked before each call

**Location:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1094–1141,1206–1208`

Queue measurement checks `raw.halted()` only before a whole trial. If the deadline expires during that trial, further timed applies, clock polling and the untimed reset still run. The comment claiming a check “before each call” and UR-34’s deadline guarantee are therefore inaccurate.

The deadline test at `crates/ezsdr-radio-uhd/tests/fake.rs:925–947` exercises an already-expired group, not expiry during an operation sequence. P865/P867 do not cover the queue’s missing checks.

**Fix:** Pass the deadline state through queue submission and draining, checking it before each subsequent device call. Test expiry between two calls and assert that the second call never occurs. A blocked call itself remains subject to its documented timeout.

#### 5. Major — Record finalization still depends on a possibly failed device

**Location:** `crates/ezsdr-radio-uhd/tests/hardware.rs:2173–2184`

The shared `usrp()` open remains outside group handling. More importantly, device identity is read **after all Provider groups**, despite the comment and report saying it is read before the groups.

If a Provider group fails because the device becomes unavailable, `device.serial().expect(...)` can panic outside `GroupRun`, leaving only the partial diagnostic and no final record.

**Fix:** Capture identity and binding information before running measurement groups. Convert subsequent open/setup failures into recorded group failures rather than allowing them to bypass finalization. Test this ordering with a Device that becomes unavailable after a group.

#### 6. Minor — UR-7 retries every Authority error, not just PLL-lock failure

**Location:** `crates/ezsdr-radio-uhd/tests/common/mod.rs:282–288`

Every `DeviceAuthority::new` error receives three retries. That includes time-reset and clock-read failures, not only the identified PLL failure. A transient non-PLL failure can therefore disappear into an otherwise successful measurement.

**Fix:** Retry only the named PLL-lock failure; propagate other errors immediately. Add counting tests for PLL retry success, exhaustion, and a non-retryable error.

### Checks and limits

The frequency sweep’s endpoints, 0.05 Hz threshold, twice-maximum-error calculation, TX gain-zero ordering, profile-grid comparison and FakeDevice value suppression are consistent with the brief. Spec throughput/overflow refusals are classified with termination evidence.

All newly cited test functions exist. P862–P892 and the five changed older rows each match their target once; several cover bookkeeping rather than the failure scenarios above. No out-of-scope production or bench-script changes were present.

No files were changed; no builds, tests, mutation runs or hardware experiments were run. Hardware recovery and teardown after a wedged control port remain unverified.

## 修正の見直し（`sol-fix-3c2c.log`）

VERDICT: FAIL

### Previous findings

- **1 — CLOSED:** Reception is established before submission, and sample progress must pass the last command plus margin (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1142,1258`).
- **2 — OPEN:** The shared latch does not fully prevent later calls or failed-device teardown; see findings below.
- **3 — CLOSED:** `StopOnDrop` cancels draining before the scope joins during unwinding (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1053,1140`).
- **4 — OPEN:** Submission and polling now check the deadline, but successful `rx_stop` proceeds directly to unchecked `rx_close` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1150–1152`).
- **5 — CLOSED, with the accepted limit:** Identity is captured before Provider groups, and record assembly uses saved identity (`crates/ezsdr-radio-uhd/tests/hardware.rs:2175–2195,2205–2212`). Setup failure leaves partial diagnostics.
- **6 — CLOSED:** Only the named PLL-lock error is retried (`crates/ezsdr-radio-uhd/tests/common/mod.rs:314–323`); counting tests cover success, exhaustion and other errors (`crates/ezsdr-radio-uhd/tests/fake.rs:966–980`).
- **O1 — CLOSED:** Probe arrays are summarized and retained whole in JSONL sidecars (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1863–1915`). Comparable values are unchanged; stage-4 evidence remains in those sidecars. Actual full-record size remains unverified.

### Remaining findings

#### Major — Restoration failure does not trigger leaking

**Evidence:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1809–1818` snapshots `stopped` **before** restoration. If restoration fails, it latches the failure but returns the earlier `None`. The caller’s leak decision ignores `restored` (`crates/ezsdr-radio-uhd/tests/hardware.rs:2219–2222`).

With otherwise successful groups, restoration failure therefore reaches the assertion and drops the device. `UhdDevice::drop` calls `answers()` and potentially teardown (`crates/ezsdr-radio-uhd/src/uhd.rs:642–658`), touching the failed control port.

**Fix:** Capture final latch state after restoration and make leak ownership unwind-safe, including record-writing failures. Test the complete caller sequence, not only `leak_after_stop` in isolation.

#### Major — Failure checks and call admission still race

**Evidence:** The drain checks the latch separately from invoking `rx_recv` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1031–1032`); submission similarly checks before computing and invoking `apply` (`:1212–1224`).

A thread can pass its check, be descheduled, and begin its device call after the other thread latches failure. This is not an already-running call. Additionally, the scripted failure marker precedes actual latching; the drain can perform multiple receives in that interval. The test’s “at most one” allowance therefore has a thread-order-dependent verdict (`crates/ezsdr-radio-uhd/tests/fake.rs:853–857,1030–1034`).

**Fix:** Synchronize call admission with failure publication, explicitly distinguishing admitted/in-flight calls. Use controlled barriers to test both race directions.

#### Major — Deadline expiry during stop still permits close

**Evidence:** `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1150–1152` checks once, calls `rx_stop`, then unconditionally closes on success. If stop returns successfully after the deadline, close is a new post-deadline call.

**Fix:** Recheck before close. Add a scripted stop that advances an injected deadline and assert that close never starts.

#### Minor — The deadline test remains timing-dependent

**Evidence:** `crates/ezsdr-radio-uhd/tests/fake.rs:921–934` uses a real 50 ms deadline and 100 ms sleep. Descheduling before the first timed apply can leave no timed entry, making `rposition(...).unwrap()` fail without exercising the intended sequence.

**Fix:** Inject elapsed-time state and advance it inside the scripted second call.

### Mutation assessment and limits

P893–P896 target readiness, sample completion/count and unwind cancellation. P897 targets loop deadline checks, but inherits the timing-dependent test. Rewritten P831/P862/P863/P866/P869/P870/P872/P873 target their stated decisions; they do not cover the remaining races or restoration-to-leak integration.

The raw group is last and leaking occurs after record writing; no subsequent group needs its leaked resources (`crates/ezsdr-radio-uhd/tests/hardware.rs:2193–2219`).

Read-only review only: no files changed, builds, tests, mutations or hardware runs.

## 修正 2 の見直し（`sol-fix2-3c2c.log`）

VERDICT: FAIL

#### Findings

1. **Major — OPEN: failure reporting can unwind before the lease is marked failed.**  
   `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1876–1882` marks the lease only after `groups.run` returns. That function writes the partial record first (`:2053`). If that write fails, its `println!` can panic—for example, with a closed stdout (`:2082–2083`). A restoration failure followed by this reporting panic therefore drops an unmarked lease and reaches UHD teardown (`crates/ezsdr-radio-uhd/src/uhd.rs:642–658`).

   **Fix:** Publish the lease’s failure state before failure reporting or partial-record I/O. Test restoration failure followed by a panic during that reporting, not only after `run_raw_group_leased` returns.

2. **Minor — OPEN: the race tests still depend on host scheduling.**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:998–1067` coordinates the two threads, but both tests require the seventeenth timed apply to reach their synchronization hooks. The rehearsal uses a 20 ms queue horizon (`common/measurement.rs:729`); any earlier apply taking more than 10 ms ends its condition early (`:1212,1265`). Ordinary descheduling can therefore prevent the seventeenth call from occurring while the scripted second receive waits indefinitely.

   **Fix:** Inject call-duration measurement for these tests, or otherwise arrange their synchronization without requiring earlier wall-clock-measured calls to stay below a threshold.

#### Closure assessment

- **Restoration-to-leak major: OPEN**, as finding 1. The normal restoration-error path now reads the latch after restoration (`common/measurement.rs:1834–1837`), and a subsequent final-record panic correctly leaks the marked lease (`:1863–1867`).
- **Thread-race major: production bound CLOSED; deterministic-test requirement OPEN**, as finding 2. The command and drain loops recheck the latch before subsequent calls (`:1047–1048,1229–1241`), consistent with the accepted allowance of one crossing call, not zero.
- **Close-recheck major: CLOSED.** Successful stop rechecks the deadline before close (`:1167–1169`); the injected-stop test observes no close (`tests/fake.rs:942–954`).
- **Deadline-test minor: CLOSED.** Elapsed time is advanced inside the second scripted apply, replacing the former sleep-based trigger (`tests/fake.rs:924–938`).

#### Ownership and mutation assessment

`usrp()` and `raw()` retain no hidden device clone (`tests/hardware.rs:25–27,320–325`). Provider assemblies share the supplied `Arc` (`tests/common/mod.rs:283–300`); the probe wrapper owns its inner handle (`tests/common/probe_device.rs:73`). Dropping these clones cannot destroy a device retained by a leaked lease.

P900, P873 and P906–P911 target the named decisions (`plan/maintenance/tools/mutations.json:12157–12164,12427–12544`). They do not cover the pre-mark reporting-panic path above.

Not checked: builds, tests, mutations or hardware, as instructed. No files changed.
