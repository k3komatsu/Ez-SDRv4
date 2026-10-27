# 段階 3d：Sol 6.1 のレビュー

## レビュー（`sol-3d.log`）

VERDICT: FAIL

#### 1. Major — Recorded commit can differ from the executed binary
**`plan/phase8/tools/run-record.sh:70–81`**

Snapshot extraction uses `tar -xf`, preserving the commit’s old timestamps. Every run then builds at `/work` against the same `ezsdr-v4-bench-target` volume. Cargo can regard an existing, newer artifact as fresh even when the archived source differs. The resulting measurement would identify `$SHA` while executing another commit’s code.

**Evidence:** The replaced `~/ezsdr-bench/run-bench.sh:24–25` deliberately uses `tar -xm` for fresh timestamps. AGENTS.md §7 explains this exact cache hazard. The new script drops that protection.

**Fix:** Restore extraction with fresh timestamps (`tar -xm`). Serialize builds sharing this volume, or isolate concurrent runs’ targets. Add a regression check that runs two differing snapshots through the same cache and verifies the second snapshot’s behavior.

#### 2. Minor — Misspelled preflight tests are reported as passed
**`plan/phase8/tools/run-record.sh:123–128`**

The name validation checks only syntax. A typo such as `hw_b3_recieve_at_t0` passes validation; Rust’s test harness runs zero matching tests and exits successfully. The script then prints “passed” and starts the measurement suite, silently skipping the requested BP5 check.

**Fix:** Verify each requested name against the snapshot’s test list, or require the captured result to show exactly one executed, passing test before reporting success.

#### 3. Minor — Initial setup probes the device before configuring its network
**`plan/phase8/bench.md:14–29`**

The numbered procedure runs discovery/probe and checks jumbo-frame size before configuring the host NIC. After reboot, the temporary address and MTU settings may be absent, so those commands cannot provide the stated checks.

**Evidence:** `handoff.md:118` says the host settings must be reapplied after reboot; `bench-host.sh:118–119` supplies the required MTU and address.

**Fix:** Put host check/apply before network discovery and probe. Check frame size after configuration.

#### Checks and scope

- Changes are confined to the specified four files; no `crates/`, `design/`, or `handoff.md` changes.
- Both scripts pass `bash -n`; `git diff --check` passes.
- Non-root host verification passes. `--help` works; answering `no` stops before Docker/device access.
- Static inspection found no host-changing operation without `--apply`. NIC address/MTU/ring/NetworkManager changes target the named interface; sibling-function ASPM changes are intentional and documented.
- Spot-checked shake-out durations, lead counts, in-flight errors, burst-gap exception, phase figures, loopback delays, UR-17, and #56 against the logs; these agree.

Not checked: root/apply behavior, Docker execution, or hardware. No files were changed.

## 修正の見直し（`sol-fix-3d.log`）

VERDICT: FAIL

#### 1. Major — OPEN: interrupted runs can escape the build lock
**`plan/phase8/tools/run-record.sh:61–67,86–95,131–160`**

Fresh extraction is fixed (`tar -xm`, line 80), but serialization does not survive every interruption. The lock is held by host processes; Docker containers are daemon-managed and do not hold it. Killing the runner’s host process group with SIGKILL can release the lock while its container continues building or running. A second invocation can then rebuild the shared target underneath that container, again compromising commit-to-binary attribution.

**Fix:** Hold the lock inside the shared volume for the actual container workload’s lifetime. Add an interruption regression demonstrating that a surviving container prevents another build/run from entering. The documented prohibition on concurrent legacy-script use remains necessary (`plan/phase8/bench.md:56`).

#### 2. Minor — CLOSED: nonexistent preflight names are rejected
**`plan/phase8/tools/run-record.sh:97–107,148–156`**

Names must match a complete line in the snapshot’s test list. Execution uses `--exact`, and success additionally requires exactly one passing test. No ordinary name/filter bypass found.

#### 3. Minor — CLOSED: host setup precedes discovery
**`plan/phase8/bench.md:12–31,35–37`**

The procedure now checks/applies host settings before discovery and probe, and checks frame size afterward.

#### 4. Minor — D9 text overstates the observed sample-level behavior
**`plan/phase8/bench-results.md:139`**

The assertion that all three commands were removed individually in the first samples follows the brief, but the cited experiment does not demonstrate it. `~/.cache/ezsdr-bench/2026-10-27/hw_d9_p1_ddc_entries_wait_for_samples.log:44–45` reports eight zero initial sample steps and a later median near 160 kHz. This supports the final timed value overwriting the untimed reset, not the individual dequeue timing.

**Fix:** Label the one-per-sample explanation as source-derived/inferred, or remove that clause. P2/P3 figures match their logs; fourth-run figures match the brief and spot-checked log output.

**Checks:** Both scripts pass `bash -n`; help and `git diff --check` pass.

Not checked: Docker, hardware, root/apply, or live interruption behavior. No files changed.
