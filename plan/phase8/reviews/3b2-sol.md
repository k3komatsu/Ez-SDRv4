# 段階 3b-2：Sol 6.1 のレビュー

## コードのレビュー（`sol-3b2.log`）

VERDICT: FAIL

#### Failure scenarios

- A complete hardware burst fails because its ACK timestamp differs from the assumed exclusive end.
- A successful interrupting burst is credited even though the repeat it should interrupt already failed.
- Submission-time reports fall outside a trial because its interval starts after submission returns.
- Host scheduling changes the throughput sample count or masks missing samples.
- An adverse measurement prevents its own record from being saved.
- Candidate construction, conditions or evidence extraction are wrong, while helper-only tests still pass.

#### Findings

**1. Major — ACK completion requires an unsupported UHD timestamp contract**  
`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1303–1320`

Only `BurstAck.tick == expected_end_tick` completes either TX measurement. Every other timestamp is silently discarded.

**VERIFIED from UHD 4.10 source:** [`radio_tx_core.v:369–375,451–460`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v#L369-L375) generates the ACK at the radio EOB event and puts `radio_time_d1` into its FIFO. [`radio_control_impl.cpp:1314–1317`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/radio_control_impl.cpp#L1314-L1317) forwards that timestamp; [`rfnoc_tx_streamer.cpp:320–323`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_tx_streamer.cpp#L320-L323) converts it directly to `time_spec`. None converts it to the Provider’s exclusive sample end.

The exact offset for this X300/DUC configuration remains **INFERRED**, not verified by this review. Requiring equality is therefore not a validated completion rule; legitimate hardware acknowledgements can leave every TX candidate incomplete. The synthetic test merely supplies the assumed timestamp.

**Fix:** Define ACK association using UHD’s actual event semantics and isolated burst ordering, retaining the independent full-length/EOB checks. Add a source-grounded non-exclusive-end ACK case; do not guess a one-sample tolerance.

**2. Major — In-flight measurements can succeed without interrupting a running repeat**  
`crates/ezsdr-radio-uhd/tests/common/measurement.rs:934–950,1344–1350`

The initial repeat runs for 100 ms before the first trial. No assertion establishes that it remains active when interrupted. Known-tick failures before the trial are discarded.

Concrete case: the repeat underflows during settling and is abandoned; its underflow timestamp precedes the first trial. The later 100-sample burst transmits successfully. Extraction checks only that burst and its ACK, so the candidate can pass without testing preemption. Admission of `start_repeat` is not evidence of playback.

**Fix:** Require repeat evidence extending to each interruption boundary. Keep setup failures as explicit stage evidence rather than silently excluding them. Add a synthetic case with a failed/absent repeat and an otherwise complete interrupting burst; it must not pass.

**3. Major — Throughput compares a fixed duration with an uncontrolled stopping interval**  
`crates/ezsdr-radio-uhd/tests/common/measurement.rs:1075–1083`

`advance_to(T0+D)` does not stop acquisition at that device instant. `run.finish()` subsequently initiates shutdown; the Provider books its stop using a fresh `core.now()` (`src/provider/mod.rs:723–727`). Nevertheless, the final cumulative sample count is compared with exactly `D×rate`, allowing only one packet.

At 200 MS/s, a 1 ms delay before shutdown adds 200,000 samples without any overflow. Healthy reception can fail; extra acquisition can also compensate for missing samples. The pure tolerance test cannot observe this interval error.

**Fix:** Measure samples within a device-bounded interval, or derive the expected count from the actual recorded acquisition boundaries. Test delayed shutdown independently of sample loss.

**4. Major — Adverse measurements are discarded instead of recorded**  
`crates/ezsdr-radio-uhd/tests/hardware.rs:1521–1524`

Comparison panics before writing the record. This defeats the measurement workflow precisely when hardware disproves the profile.

There is also a deterministic default example: the largest throughput candidate is 200 MS/s × 4 bytes = **800 MB/s**, while the current 10 GbE profile advertises **1,000 MB/s** (`crates/ezsdr-radio/src/x3xx.rs:407–412`). If that candidate succeeds, comparison rejects the profile and no record is written—even after the full suite completes. A measured `full_duplex=false` has the same problem.

**Fix:** Save the validated measurement record before asserting profile agreement. Preserve comparison failure as the test result. Add a hardware-free case proving a disagreeing record survives.

**5. Minor — “Effective lead” and trial start are sampled after receipt**  
`crates/ezsdr-radio-uhd/tests/common/measurement.rs:848–864`

The observation uses `run.now()` after `submit()` returns, rather than `entry.time`, the logged arrival instant (`crates/ezsdr-kernel/src/session.rs:117–122`). Submission work or descheduling makes the reported lead artificially shorter. It also excludes the beginning of the operation from the trial interval.

**Fix:** Use the returned log entry’s receipt timestamp for both calculations. Test with a later post-submit clock reading.

#### Tests, scope and records

- P715–P761 each match their source exactly once. The mutations target real helper decisions, but do not establish the missing interval, repeat-playback or UHD ACK semantics above.
- The FakeDevice rehearsal appropriately avoids asserting classification outcomes or measured magnitudes; its supplied values become `unmeasured`.
- UR-34’s cited functions exist, and the added citations remain inside its indented Test text.
- No `src/` changes, new rig, fault, retry or queue-depth measurement were found.
- Default execution opens **55 Provider Runs**: 7 command-lead, 7 in-flight, 14 cold-change, 7 throughput and 20 loopback. Their configured two-second starts add roughly **110 seconds**, before other overhead; the report’s 13.4-minute estimate explicitly excludes setup.

No files changed. No builds, tests, mutations or hardware operations were run; runtime results in the implementation report were not independently reproduced.

## 修正の見直し（`sol-3b2-fix.log`）

VERDICT: FAIL

1. **Minor — Structural validation admits an invalid alignment measurement.**  
   `crates/ezsdr-radio-uhd/tests/common/measurement.rs:292–299`  
   The blanket `reason.starts_with("profile value ")` exemption also admits structural failures. For example, measured `radio.tx.repeat_align_samples = 0` produces “profile value … is not a positive multiple of measured value 0” (`crates/ezsdr-radio/src/measurement.rs:550–556`). The writer saves this record, although zero cannot define an alignment multiple. The new structural-rejection test covers only fake source.  
   **Fix:** Separate the positive-value requirement from the multiple comparison, reject invalid operands before writing, and test zero alignment alongside a valid profile disagreement.

2. **Minor — The receipt-time regression still lacks a test that catches the original mistake.**  
   `crates/ezsdr-radio-uhd/tests/common/measurement.rs:880–882,1629–1636`  
   The implementation correctly passes `entry.time`, but the pure test supplies the timestamp directly. Changing the caller back to post-submit `run.now()` leaves that test passing. The fix report confirms that this original mutation survived; replacement P772 merely changes the helper’s arithmetic by one tick. It therefore does not protect the decision being fixed.  
   **Fix:** Add deterministic coverage of the timestamp selection with distinct receipt and post-submit times, and retain a mutation that restores the original wrong selection.

3. **Minor — Required early-ended-repeat coverage is missing.**  
   `crates/ezsdr-radio-uhd/tests/common/measurement.rs:1814–1838`  
   The test covers an absent repeat, a repeat ending exactly at the interrupt target, and a setup underflow. It never supplies a repeat record ending **before** the target. P766 reverses the equality and is killed by the positive case; that does not establish rejection of the requested negative case.  
   **Fix:** Add an otherwise complete interrupting burst with a repeat ending one sample early; require `Incomplete`.

**Verified:** The other adjudicated changes are present: non-exclusive-end ACK acceptance, zero/two-ACK rejection, offset observations, boundary failure retention, recorded receive intervals, write-before-profile-assert ordering, and limiting-condition selection. The TX owner keeps repeat-to-interrupt continuation inside one device burst (`src/provider/tx.rs:418–432,735–744`), supporting one device EOB per successful isolated trial. Multiple receive clocks conservatively yield `Incomplete`. All 18 requested mutation rows match exactly once and change real decisions.

No files changed. No builds, tests, mutations, or hardware operations were run; reported runtime results were not independently reproduced.
