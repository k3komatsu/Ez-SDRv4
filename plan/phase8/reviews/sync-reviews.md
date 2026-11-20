# sync_check — GPT-6.1 Terra reviews

## review

**Verdict: FAIL.** The programs need fixes before their results can support D-1/D-4.

- **Major — sensor read can crash before any run line.** `plan/phase8/tools/sync_check/sync_check.c:50` passes an uninitialized sensor handle to UHD. UHD 4.10 dereferences that handle; it must first be created with `uhd_sensor_value_make`. For example, `probe --run 1 A1` can crash at its first sensor read, producing **no `S0` line**. Free the created handle on success and failure. [UHD 4.10 source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/usrp_c.cpp#L431-L440)

- **Major — time setting can falsely pass without taking effect.** `plan/phase8/tools/sync_check/sync_check.c:93` does not establish that the commands were armed within the observed second, and `plan/phase8/tools/sync_check/sync_check.c:81` accepts any integer-second `last_pps`. If both devices already agree at 100 seconds and both commands miss the next edge, the check can observe `last_pps=20200000000,20200000000` and emit `S3 ... verdict=PASS`, although neither has yet applied the requested zero.

- **Major — sequential time reads can falsely refuse synchronized devices.** `plan/phase8/tools/sync_check/sync_check.c:83` compares two sequential `time_now` readings without bracketing them with two handle-0 time readings as §3.7 requires. If the devices agree but the second read occurs 11 ms later on each attempt, the output is `S3 ... verdict=REFUSED reason="time_now(h1) differs from h0 by more than 10 ms (after 3 attempts)"`.

- **Major — PPS increments can misstate the evidence.** `plan/phase8/tools/sync_check/sync_check.c:111` reads the follower immediately after detecting the source edge, without ensuring the follower’s corresponding edge has arrived. For synchronized devices with a slightly later follower edge, it can print `S3 ... incr1=0,400000000,200000000 verdict=PASS`. Those apparent zero- and two-second increments could wrongly drive the item 6 decision.

- **Minor — S5a has a data race.** `plan/phase8/tools/sync_check/sync_check.c:291` reads `m.falses` and `m.relock` while the monitor thread writes them. This is undefined behavior and can corrupt the reported observation or wait.

- **Minor — cleanup and bounded output are incomplete.** Most `sync_check` paths leave USRP handles allocated; `align` also leaves RX streamers, metadata and buffers allocated (`plan/phase8/tools/sync_check/sync_check.c:271`). A blocked UHD call can leave a run with no line, and the `rx_grab` loop has no total deadline if receives repeatedly return zero samples (`plan/phase8/tools/sync_check/sync_check.c:185`). The required `order` field is also missing from the separate-mode line.

The source contains **no transmit operation**: `sync_check` uses RX streaming only; `tx_packet_probe` creates and queries a TX streamer but has no send, TX metadata, or TX-start operation. This is a static review; I did not build or run either program.
## re-check 1

**Verdict: FAIL.** The sensor-handle and bracketed-read findings are closed, but the time-setting and PPS-increment findings remain open. The fix also introduces result-classification and output races.

- **Major — time setting can still falsely pass.** If both clocks are already near zero and both successful commands fail to take effect, their next readings can still be `last_pps=200000000,200000000` and `time_now≈1.01 s`. The new range check accepts those values, producing `S3 ... verdict=PASS` without proving the reset occurred. `plan/phase8/tools/sync_check/sync_check.c:89`

- **Major — void attempts become a hardware refusal.** If an edge crosses command issuance on all three attempts, none is a valid verification, yet the output is `S3 ... attempts=3 void_attempts=3 ... verdict=REFUSED reason="an edge passed while the commands were issued (after 3 attempts)"`. The retry loop terminates, but its result misclassifies an inconclusive measurement. `plan/phase8/tools/sync_check/sync_check.c:101`

- **Major — follower waits can corrupt leader increments.** If the follower misses one PPS, its bounded 1.2-second wait can cross another leader edge. The next iteration then reads a two-second leader increment and can emit `S3 ... incr0=200000000,400000000,... missed=0,1 verdict=PASS`, misrepresenting the leader’s per-edge evidence. `plan/phase8/tools/sync_check/sync_check.c:128`

- **Major — watchdog can produce two verdict lines.** If `probe` prints `S0 ... verdict=PASS` and `uhd_usrp_free` blocks until the watchdog fires, stdout also receives `S0 ... verdict=ERROR error="timeout"`; the process exits 2. There is no coordination after the first line. `plan/phase8/tools/sync_check/sync_check.c:264`, `plan/phase8/tools/sync_check/sync_check.c:301`

The sensor handle is now created and freed, and the verification reads are bracketed. The follower’s edge is now awaited with a bound, but the increment issue above remains. Static inspection found no transmit operation in either program: `sync_check` uses RX streaming, and `tx_packet_probe` creates and queries a TX streamer without sending. No build, execution, or edit was performed.
## re-check 2

**Verdict: FAIL.**

- **Major — slow verification reads still become a refusal.** If both devices are synchronized but each handle-0 time bracket takes 11 ms on all three reads of all three attempts, every bracket is discarded. The program nevertheless emits `S3 ... attempts=3 ... verdict=REFUSED reason="the bracket crossed an edge or was wider than 10 ms in 3 reads (after 3 attempts)"`. No valid comparison established a hardware refusal. `plan/phase8/tools/sync_check/sync_check.c:120`

- **Major — a slightly later follower edge can cause a false refusal.** If handle 0’s edge is detected immediately and the correctly synchronized follower takes the same PPS a few ticks later, the immediate follower read can still contain the previous second. If this happens on all three attempts, the output is `S3 ... verdict=REFUSED reason="last_pps(h1)=... != last_pps(h0)=... (after 3 attempts)"`. The verification does not wait for the follower edge before treating that mismatch as a failed attempt. `plan/phase8/tools/sync_check/sync_check.c:118`

The prior nonce, all-void classification, independent increment polling, sensor-handle, and duplicate-verdict findings are closed in the inspected paths. Retries are bounded; a missing follower PPS does not block the increment loop.

Static inspection found no transmit operation in either program: `sync_check` streams RX only, and `tx_packet_probe` creates and queries a TX streamer without sending. No build, run, or edit was performed.