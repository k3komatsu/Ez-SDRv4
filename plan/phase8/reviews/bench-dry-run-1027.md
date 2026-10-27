# Bench dry run: `run-record.sh` at HEAD 660213b (read-only, 2026-10-27)

I read the code at HEAD with `git show`. Nothing was built or run, and no device was touched.
- D9a is still uncommitted (it touches `device.rs`, `control.rs`, `uhd.rs` and `fake.rs`), so it is not part of this trace. Re-check F5, and anything else about `UhdDevice`, on the commit the bench actually uses.
- Evidence comes from the 10-20 and 10-27 logs in `~/ezsdr-bench/` and `~/.cache/ezsdr-bench/2026-10-27/`.
- Every `path:line` below is at HEAD unless it says otherwise.
- `hardware.rs` stands for `crates/ezsdr-radio-uhd/tests/hardware.rs`, and the other `tests/common/…` paths are under the same crate.

## Verdict

- **No BLOCKER found.** Neither (a) nor (b) is expected to fail outright, hang or transmit anything unsafe.
  - The record should be written in every failure mode except four:
    - the setup open or identify step fails (by design);
    - a malformed selector element (F13);
    - a process abort (F5);
    - a disk-write error.
  - The run will still end in "measurement failed", because the throughput values disagree with the profile, but the record is written before that comparison.
- **The day's main new value is probably lost:** `radio.timing.command_queue_depth` will almost surely come out `unmeasured` (F1).
  - The per-call rows survive in the record, so stage 4 can reclassify them.
- **`radio.perf.rx_bytes_per_s` changes meaning (F2).**
  - It will read about 200 MB/s instead of shake-out 4's 800 MB/s.
  - Its label will name a 1-channel condition, while the real limit comes from a duplex stage's transmit underflow.
- **Neither script has ever run on this host.** There is no `~/ezsdr-bench/<date>/` and no lock file, so the bench day is the first real run of `run-record.sh` and `bench-host.sh`.
  - Reading both line by line found no shell bugs. bash is 5.3, `flock`, `ethtool`, `nmcli`, `python3` and `docker` are all present, and the ASPM sysfs paths of both X710 ports exist.

## Findings, ranked

**F1. WRONG RECORD (high likelihood): the queue depth will be `unmeasured`.**
- **Where:**
  - `measurement.rs:902-905`: a trial counts only if the blocked call's lower device-time bound `d0 + (h_i − h_after)` is at or after `first_at`, with no tolerance.
  - `:1333-1341`: all 5 trials of every condition must succeed.
  - `:716`: `queue_max_calls` is 64.
- **Evidence (VERIFIED):** I took each raw-probe log's "apply ms per call" and summed the calls up to the blocked one, then subtracted 3000 ms. That is how long after the 3 s target the blocked call returned.
  - Results: +0.17, +0.12, +0.11, +0.11, +0.07, +0.05, +0.04, +0.03, +0.01, −0.01, −1.40 and −2.42 ms.
  - The negative ones:
    - −0.01 ms: #1 Rx, `~/ezsdr-bench/8fab132-36/hw_b8_raw_queue_depth.log`;
    - −1.40 ms (Rx) and −2.42 ms (Tx): `shakeout-2.log:84-88`.
  - So 3 of 12 readings fail, and all the others pass by ≤0.17 ms. The record's classifier has the same structure.
  - P(15 of 15 trials succeed) ≈ 1 %.
- **Second cause (INFERRED):** a tune blocked at call 8–9, but a gain write is a few ctrlport words. The gain conditions may therefore never block within 64 calls, which gives "no timed apply blocked before the call limit".
- **Consequence:** the value row is `unmeasured`. The `raw_command_queue_depth` observation keeps every trial's `d0`, `h_before`/`h_after`, `h_i` and duration. The raw group's observations are not summarised, so all of that stays in the record.
- **Smallest fix:**
  - At `:903`, make a trial Incomplete only when the **upper** bound `< first_at`, or give the lower bound a few ms of tolerance.
  - Raise `queue_max_calls` (to 512, say).
  - Or accept the loss and reclassify from the rows in stage 4.

**F2. WRONG RECORD: the RX capacity is decided by duplex TX underflow, and its label names a 1-channel condition.**
- **Where:**
  - `multichannel.rs:421-428`: duplex stages run for a 1-channel layout too.
  - `:712-713`: any `TX_UNDERFLOW` fails the stage (UR-34 "judge both").
  - `:457-461`: `best = (total, at[0].condition)`.
- **Evidence:** VERIFIED that TX alone underflows at 100 MS/s after about 29 s (`bench-results.md:111`). INFERRED that it does the same in duplex.
  - In (a), `rx-duplex-100m` fails, so the capacity becomes 200 MB/s.
  - The first stage at 200 MB/s is the 1-channel `rx-50msps`, so the record names that condition.
  - The blocker is visible only in `provider_rx_throughput_capacity.first_blocker`.
- **Smallest fix:** label the value with the stage at T that belongs to the blocking family (here `rx-duplex-50m`), not `at[0]`. Also tell stage 4 that the RX value is now TX-bounded by design.

**F3. RISK (state VERIFIED, frequency INFERRED): a reference PLL that stays unlocked does not stop the record.**
- **Where:**
  - `tests/common/mod.rs:367-371`: 3 retries, then `unwrap`. That fails only the current group.
  - `hardware.rs:321-327`: `raw()` unwraps `set_sources` with no retry.
  - The raw group never sets sources.
- **Evidence:**
  - On 10-27 the unlock persisted until a power cycle (`bench-results.md:90`; `ref_locked` was false before the cycle).
  - Each failed `set_clock_source` costs UHD's 30 s.
- **Consequence:**
  - About 2 min is lost per group (×11) and 30 s per raw probe, so 25–30 min in all.
  - The record is then mostly failures, and the raw group's timing runs on an unlocked reference.
  - `bench.md:94` says to stop once the retries are used up.
- **Smallest fix:** when the retries are exhausted inside a record, set the record latch (a 3-line helper next to `install_record_latch`). Later probes and groups are then "not run", the record is written quickly, and the owner power-cycles. Until then, watch the log for `UR-7 retry 3` and press Ctrl-C.

**F4. RISK: one stage error throws away a whole throughput group.**
- **Where:**
  - `multichannel.rs:791`: `run_stage(...)?`.
  - `tests/common/mod.rs:371`: `unwrap` after the PLL retries.
  - `measurement.rs:3093` and `:3309`: `spec_run` unwraps.
  - `GroupRun::run` (`:2091-2101`) appends nothing on `Err` or panic.
- **Consequence:** in (b), up to 32 admitted stages (about 34 min of one direction's data) can be dropped by a single late PLL miss or a stage-level `Err`. An example of such an `Err` is "no active transmit stream-k SampleClock".
- **Smallest fix:**
  - In `extra_throughput`, catch a stage's `Err` or panic.
  - Record it as that stage's failure (`class: None` plus an observation), then stop the climb.
  - Keep the stages measured before it, and the 1-channel sweep.

**F5. RISK (low likelihood, high cost): a streamer open that fails never carries UHD's text, so a control timeout there never latches.**
- **Where:**
  - `src/uhd.rs:962` and `:1055` read `uhd_usrp_last_error` from the device handle.
  - UHD's C API saves `uhd_usrp_get_rx_stream` / `uhd_usrp_get_tx_stream` errors on the **streamer** handle.
  - The guard (`multichannel.rs:141-145`) latches errors from non-`apply` calls only on loss or when the text contains "Control operation timed out".
- **Evidence:** VERIFIED in `bench-results.md:73`: UHD error 47 at a streamer open came with an empty `uhd_usrp_last_error` text.
- **Consequence:**
  - A wedge that first shows up at an open is not latched, so the next probe or group uses the wedged handle.
  - Its teardown can throw out of a destructor (SIGABRT), leaving no record, only the partial.
- **Smallest fix:** use `check_rx(h, "uhd_usrp_get_rx_stream", …)` and `check_tx(h, …)`. D9a is editing `uhd.rs` now.

**F6. RISK (INFERRED): a Provider thread left detached latches the record, and every later group is lost, the raw group included.**
- **Where:**
  - `src/provider/mod.rs:75`: `JOIN_BOUND` is 1 s.
  - `src/provider/tx.rs:108`: `SEND_TIMEOUT` is 1 s.
  - `mod.rs:767-772`: a detached thread leaks its streamers.
  - `hardware.rs:2212`: `latch_if_streamers_open`.
  - `measurement.rs:1907-1921`: the raw group runs last.
- **Why it can happen:** a send that waits out its full 1 s at Shutdown also exceeds the 1 s join.
  - Not seen on 10-27: shake-out 4 ran every group.
  - But 3c-3a adds 14 duplex stages, up to 200 MS/s each way, ahead of overflow, transmit throughput, full duplex and the raw group.
- **Fix:**
  - Make `JOIN_BOUND` longer than `SEND_TIMEOUT` (2 s, say).
  - Until then, "did not join within 1 s" in the log means the rest of the record is "not run".

**F7. NOTE (owner's ramdisk rule): the container's `/tmp` is on the NVMe SSD.**
- **Where:** `run-record.sh:136-139` and `:181-193` mount no `--tmpfs`, so `TempDir` writes go to docker's overlay on `/dev/nvme0n1p2`.
- **Size:** about 10–20 GB of temporary capture writes per record:
  - overflow captures of 166–500 MB per trial (`measurement.rs:2950-2952`);
  - about 2,800 receive-start-lead captures of ~2.5 MB each.
- **Fix:** add `--tmpfs /tmp:rw,size=4g` to both docker arrays. 46 GB of RAM is free.

**F8. NOTE: `ref_locked` is recorded nowhere.**
- **Where:**
  - `probe.log` lists only sensor names (`probe.txt:35`: "Sensors: temp_fpga, ref_locked").
  - `run-record.sh:160-177` parses only Mboard and FPGA.
- The runbook (§2b, §3) expects to read `ref_locked` from `probe.log`.
- **Fix:** pass `hw_b1_probe` as a preflight. It prints "B1 ref_locked" and transmits nothing; it adds slot A to the prompt, which transmits anyway.

**F9. RISK (low): every container start downloads crates again.**
- `CARGO_HOME` lives in the image layer. VERIFIED: "Updating crates.io index / Downloading crates" appears in every 10-27 log.
- If crates.io cannot be reached when the record container starts, the run ends "measurement failed" with nothing sent to the device.
- **Fix:**
  - Check the internet connection first.
  - Or set `-e CARGO_HOME=/cargo-target/.cargo-home` (the volume root is 1777; INFERRED to work).

**F10. NOTE (runbook): the console log is lost.**
- §2(b) and §2(d) do `| tee ~/.cache/ezsdr-bench/$(date +%F)/…`, but nothing creates that directory before §2(c).
- `run-record.sh` creates `~/ezsdr-bench/<date>/` and `~/.cache/ezsdr-bench/`, not this one. The run's own logs are still saved.
- **Fix:** `mkdir -p` first.

**F11. NOTE (docs vs code, RF):** every point below still stays within the ≥30 dB, 18 GHz-rated attenuator that the prompt names, and nothing transmits outside the boards it names.
- **Full-range sweep:**
  - `bench.md:9` says the coercion TX frequency read-back stays inside `SAFE_TX_HZ`.
  - In the code, that read-back *is* the full-grid sweep (`measurement.rs:1748-1757`): OBX 10 MHz–8.4 GHz, UBX 10 MHz–6 GHz, CBX 1.2–6 GHz, with no samples.
- **CBX gain sweep:** the "LO at 1 GHz" for the CBX gain sweep is really about 1.1–1.2 GHz.
  - UHD clips without an error. That is VERIFIED on #1's unknown board: `shakeout-4.json` cleanup reads back 0.0 Hz with no error.
- **Waveform level:** older steps send `pn` at |z| = 0.566 (`tests/common/mod.rs:582-590`), above `bench.md`'s ≤0.5.

**F12. NOTE (the prompt):**
- `run-record.sh:81-89` asks for "yes" before it checks `EZSDR_BENCH_NIC`.
- For (a), the wording "slot A board (raw probes and preflight tests)" does not say that the record's groups, and the full-range sweep, also transmit on slot A.
- With no selector, two boards of the same kind in A and B would put slot B into the layout (`tests/common/mod.rs:91-111`), but the prompt names only slot A. That case does not arise on this bench.

**F13. NOTE: a bad selector element is caught only after the device is opened.**
- For example `"0:B:cbx:1"` passes the script's Python check and `parse_selector`, which checks keys only (`tests/common/mod.rs:118-130`).
- It fails only at the setup `.expect("the bound X3xx layout")` (`hardware.rs:2181-2182`). By then the build, probe and preflight have all run, and no record is written.
- **Fix:** call `channels::from_selector` inside `process_metadata`.

**F14. NOTE:** the notes on the summarised probe rows give container paths, `/bench/measurement.json.rows/…` (`measurement.rs:2016-2018`).

**F15. NOTE: the timed-tune-phase observation from inside the record is not reliable.**
- In-record readings spread 95–177° (7 of 8 readings; `bench-results.md:101,129`). Separate processes give 0.0–0.9° (`:54`).
- Mechanism INFERRED: the DUC releases timed values per F3′/F7 in note 42. Do not use this observation before D9b.

**F16. NOTE: the raw probe `hw_b8_raw_queue_depth` is still in the record.**
- `hardware.rs:2160` and `:524-551`: it queues ≥8 timed DDC and DUC tunes with no samples flowing.
- VERIFIED harmless: UHD flushes every block on open (`research/d9-production-investigation.md:36`), and the shake-outs carried on normally afterwards.
- It duplicates the raw group's measurement and could be dropped.

**F17. NOTE: the record is written after the lease's teardown.**
- `hardware.rs:2200-2233` drops the handle, then `:2236-2247` writes the record.
- If a clean run's teardown hung or aborted, the record would be lost.
- VERIFIED low risk: teardown on a healthy handle ran 20 or more times in every shake-out.

## Trace

| Step | (a) #1, OBX in A, slot B empty, no selector | (b) #2, UBX in A + CBX in B, selector `{"rx","tx":["0:A:ubx:0","0:B:cbx:0"]}` | Record? | Transmits |
|---|---|---|---|---|
| Checks and prompt | Prompt names "slot A board…"; then the NIC check (F12) | Selector checked for both or neither; prompt names slot A and `0:B:cbx:0` | – | none |
| `bench-host` (read-only) | needs `--apply` after a reboot; exit 1 stops | the same on `enp1s0f1np1`; first remove `.10/24` from the old NIC | – | none |
| Build, list, `uhd_usrp_probe` | new containers, crates download (F9); a PLL miss at init is only a warning | the same | – | none |
| Preflight (optional) | slot A at 1 GHz, gain 0 | the same (slot A UBX) | not started if it fails | slot A |
| Setup: open, identify, close | `describe` cached; serial; layout `0:A:obx:0`; closed before the probes | layout from the selector; `CBX RX`/`CBX TX` match UR-5 (VERIFIED in `probe-347D541-uhd410.log`) | none if it fails (by design) | none |
| 15 raw probes, each reopening | UR-17 opens a second handle while the first is alive: one shared UHD device, no re-init (VERIFIED in the shake-outs). The restore step touches the unknown slot-B board without error (VERIFIED in `shakeout-4.json`). No UR-7 retry (F3) | applicable (UBX in A). The restore step sets the CBX TX to "1 GHz" (clipped, no error, INFERRED) | yes; a latch makes the rest refused | slot A, 999.5–1000.5 MHz, gain 0 |
| Reopen, 11 groups on one handle | each Run sets sources (3 retries), zeroes the time, sets subdev `A:0`. Duplex stages run at 10–200 MS/s (F2, F6); companions at 10 MS/s | subdev `A:0 B:0`; the 1-channel groups use channel 0 (other channels default to −1); 34 stages per direction, 2 refused at admission | yes; a group error is recorded | (a) slot A at 1 GHz; (b) also the CBX at 2.45 GHz ±1 MHz |
| Raw group (300 s limit) | queue on rx-gain, tx-gain, rx-frequency (streams 1 MS/s, drained on a second thread; no deadlock, because `rx_recv` does not take the control mutex); coercion; restore | 2 channels per direction; the CBX grid is 1.2–6 GHz | yes; on a wedge it latches and the handle is leaked | LO sweep with no samples; gain up to 31.5 dB at about 1 GHz with no samples |
| Write, then compare | record of a few MB plus `.rows`; throughput disagrees with the profile, so the test FAILS *after* writing | the same | written | – |

## Duration, from the code's defaults

- **Assumptions:** 100 trials per step, 60 s per throughput stage, about 3.5 s to set up each Run. My formula gives 42 min for shake-out 4's set of groups, which actually took 36 min, so I scale the result by about 0.85.
- **Components (minutes):**

  | Component | (a) | (b) |
  |---|---|---|
  | Raw probes | 8 | 8 |
  | Command lead | 0.7 | 0.7 |
  | In-flight window | 0.8 | 0.8 |
  | Receive-start lead | 8.5 | 8.5 |
  | Transmit-start lead | 1.6 | 1.6 |
  | Receive throughput | 15 (14 stages) | 34 (34 stages, 2 refused) |
  | Overflow | 6 | 6 |
  | Transmit throughput | 15 | 34 |
  | Full duplex | 1.3 | 1.3 |
  | Companions | 11 | 11 |
  | Raw group | 2–3 | 3–4 |

- **(a):** about 60–70 min of record; with the build, the probe and network delays, about **65–80 min**.
- **(b):** about **1 h 40 to 2 h**.
- Each UR-7 retry adds about 31 s; an unlock that persists adds 25–30 min (F3).
- The existing estimates are too low: `bench.md:89` says 30–40 min and the runbook says 40–60 min for (a).

## Checked and found OK

- **Latch text:** `is_control_timeout` matches UHD's real text, "Control operation timed out waiting for space in command buffer" (`shakeout-2.log:274`). This holds for calls whose text comes from the device handle or the streamer handle.
- **Records stay small:** the 157 MB of shake-out 4 came from four raw probes (`hw_56_*`, `ur17`, `h5`), and those are now summarised into `.rows`.
- **The partial file** is rewritten after every group and removed once the record is written.
- **Selector and NICs:** the selector reaches the container (`run-record.sh:194`).
  - NIC counters are read from `/sys/class/net` inside a `--network=host` container (INFERRED); a read that fails gives null only.
- **Structure of the record:**
  - Every condition ID is unique or identical in content.
  - Every rate is on the grid.
  - The `s1` keys exist for 2 channels.
  - So `merge_values` and `compare` raise no structural violation.
- **The queue thread:** the drain thread holds only the streamer's lock, so a timed `apply` blocked for 3 s cannot deadlock it.
- **Waits:** all waits are bounded (1 s, 5.01 s, 3 s, and UHD's ~10 s).
- **Preflight check:** the `-q` output matches the script's `test result: ok. 1 passed; 0 failed; 0 ignored;` (10-20 logs).

## Not checked

- D9a's uncommitted changes.
- The real size of the radio and DDC command FIFOs (F1's second cause).
- Whether a UBX or CBX tune waits for lock (only matters for the raw group's time limit; INFERRED fine).
- The D9 probes and the commands in runbook §2(c).
