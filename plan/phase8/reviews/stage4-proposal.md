# Phase 8 stage 4 — profile values from the bench: proposal (design only, no code)

Author: the implementer (OpenCode Sonnet 5.5 #high), 2026-11-13, on `work/stage4` (tree unchanged; this file is the only output). Brief: `~/.cache/ezsdr-fixes/2026-11-13-stage4/brief-design.md`.

**Offloaded to DeepSeek: nothing.** All raw reading was done with my own Python scripts over the raw `measurement.json` files on this host (usrp-lnx03); the scripts are in `/tmp/opencode/` (`dump.py`, `leads.txt`), outside the tree.

## 0. How to read this

Evidence sources (every number below is read from these; the repo copies under `bench/` are the trimmed ones — `values` and `conditions` only, no `observations`):

| Id | Session / commit | Board | Layout | Load | Raw file (this host) | Repo copy of its `values` |
|---|---|---|---|---|---|---|
| **A** | 2026-10-28, `afbb125` | #1 OBX (347D545) | 1 rx + 1 tx | none | `~/ezsdr-bench/2026-10-28/afbb125c704e-094947966920515/measurement.json` | `bench/2026-10-28-record-suite/results/measurements/home-ezsdr-bench__afbb125c704e-094947966920515__measurement.json` |
| **B** | 2026-10-28, `afbb125` | #2 UBX+CBX (347D541) | 2 rx + 2 tx | none | `…/afbb125c704e-112734601394077/measurement.json` | `…__afbb125c704e-112734601394077__measurement.json` |
| **C** | 2026-10-28, `98b0d9a` (after TXP) | #2 UBX+CBX | 2 rx + 2 tx | none | `…/98b0d9a919c9-154746201756884/measurement.json` | `…__98b0d9a919c9-154746201756884__measurement.json` |
| **R1** | 2026-11-13, `3f1f87f` | #1 OBX | 1 rx + 1 tx | none (CPU 10.7 %) | `~/ezsdr-bench/2026-11-13/3f1f87f921ce-114706189933052/measurement.json` | `bench/2026-11-13-n67-records/results/measurements/runs__3f1f87f921ce-114706189933052__measurement.json` |
| **R2** | 2026-11-13, `3f1f87f` | #1 OBX | 1 rx + 1 tx | `h4` (16 busy + 2 memcpy workers, CPU 99.99 %) | `…/3f1f87f921ce-122349474797197/measurement.json` | `…__3f1f87f921ce-122349474797197__measurement.json` |

Field paths: `.values[key=K]` for a value row, `.observations[name=N].result…` for the raw evidence (named in each row). All five: HG image, FPGA 39.3 (git d375d68), UHD 4.10.0.0, host `usrp-lnx03` (16 cores, MTU 9000, `bench-host`), selector `link_gbps: [10]`, 30 dB TX/RX → RX2 loopback, 1 GHz. `shakeout-4.json` (2026-10-27) is not evidence (it has no `observations`, a temporary-code commit, and the plan says it never enters `measurements/`).

VERIFIED = read from a raw record or a source line I opened; INFERRED = my reading, or the record's own note says so.

### 0.1 Seven findings that shape the proposal

1. **Nothing is in `crates/ezsdr-radio/measurements/` yet** (the directory does not exist), so the acceptance comparison (`crates/ezsdr-acceptance/tests/measurements.rs`) passes vacuously today. The failure the brief mentions is the hardware test's own in-process comparison. I re-did the comparison offline (profile values from `x3xx.rs`, UHD `profile.rs`, Mock `profile.rs`, rule directions from `measurement.rs:293-317`) against the five records' measured rows:

   | Record | Mock fails | UHD fails |
   |---|---|---|
   | A | rx 1e9 > 8e8; tx 1e9 > 2e8; **lead 2 ms < 5 ms** | rx; tx |
   | B | tx 1e9 > 4e7; lead | tx |
   | C | rx 1e9 > 6e8; tx 1e9 > 2.6e8; lead | rx; tx |
   | R1 | rx 1e9 > 8e8; tx 1e9 > 4e8; lead | rx; tx |
   | R2 | rx 1e9 > **2e8**; tx 1e9 > 4e8; lead | rx; tx |

   So besides throughput, **the Mock's 2 ms command lead is looser than every record's 5 ms** (the brief names only throughput). `rx_start_lead_ns` (50 ms) and `tx_in_flight_ns` (10 ms) are stricter than measured, so they pass.
2. **The three 2026-10-28 records (A, B, C) cannot be placed as they are.** (a) They carry `radio.timing.command_queue_depth` (7, 8, 8), `radio.timing.release_window_ns` and `radio.timing.tx_start_lead_ns`, which no longer exist (note 48 S2b-i, note 47 stage 4; RM-20 refuses them) → `compare` reports "unknown Radio Model capability key". (b) Their `rx_start_lead_ns` (1, 1, 3 ms) and `tx_in_flight_ns` (5 ms) come from groups that were deleted (`design/18-uhd-radio.md:417`). (c) Their code predates TXS, #78, note 47/48, the L1/L2 chain-state work. They are history, not evidence for the current Module. Only R1 and R2 are current-code records, and both are #1 OBX one channel.
3. **The trimmed bench copies cannot be placed either**: `Record.observations` has no `#[serde(default)]` (`measurement.rs:260`) and `bench_extract.py:67` pops it, so a copy fails to deserialize. Placement needs a conversion step that writes `"observations": []` (no schema change). All five records also carry `selector.args = "type=fake-rig"` although the runs were on hardware (the device identity is in `device.serial`; `compare` ignores `args`) — a recording quirk of the installed measurement selector, to fix at the source.
4. **`rx_bytes_per_s` binds only multi-channel sums.** The rate grid tops out at 200 MS/s per channel = 8e8 B/s (`x3xx::rates`, 4 B/sample), so any `p_rx ≥ 8e8` changes nothing for a single channel; it decides what *sums* of channels are admitted (RM-32). `tx_bytes_per_s` is the opposite: the real one-channel limit (100 MS/s = 4e8 B/s) is below the grid maximum, so it is the one value that changes what a single-channel user can ask for.
5. **The RX-start-lead "measured value" is partly an artefact of the pass rule.** A stage fails on `radio.START_LEAD_SHORT` (lead left < the declared command lead 5 ms), which is only a warning (the start keeps its origin). From R1/R2 `.observations[name=provider_rx_start_lead].result.candidates[].trials[]` (late_command / start_lead_short / first_tick vs expected_origin): R1 restart-retune at 3 ms: 100/100 short, **0 late, 0 off-origin**; R2 at 3 ms: 94 short, 7 late, 7 off-origin; R2 at 5 ms: 23 short, 0 late. So "5 ms" mostly equals the 5 ms command lead plus a small configure time; the device itself restarts on time (raw `hw_b8_raw_restart_lead` on #2, `bench/2026-11-13-n67-records/README.md` item 3: +0 ticks down to 1 ms, warm and cold). The value is therefore coupled to `min_timed_command_lead_ns`; if that ever changes, the start-lead measurement must be re-run.
6. **The measured "restart after resume" is not what RM-17 defines.** RM-17/MR-21 lose `[f, f + gap)` from the overrun instant `f`; the measurement is `gap_end_sample − estimated_reader_resume_sample` (`.observations[name=provider_overflow_restart_gap].result.restart_after_resume_rule`, and its own note: "INFERRED … value is high-side"). A constant measured from the consumer's resume cannot bound a loss measured from `f` (a consumer stalled longer than `gap` after `f` makes the Mock lose less than the device). This is why 41 §8 already says RM-17 must be re-worded; it is a spec 07/09 change both Providers call, so §9 step 0 (design) applies to it.
7. **A per-direction Σ cap cannot express a per-streamer limit.** C (#2, after TXP, before TXS): one TX channel at 200 MS/s failed (8e8 B/s) while two streamers at 100 MS/s each (also 8e8 B/s) succeeded; one grouped streamer failed from 40 MS/s ×2. RM-32 has one number per direction. I do not propose to change the model now; I propose a conservative cap (§3 K7) and a sweep that tells whether the model needs a per-stream key (S3).

## 1. The profile today

`ezsdr_radio::x3xx::timing(terms)` (`x3xx.rs:405`) and `performance(link_gbps)` (`x3xx.rs:416`, `link_gbps × 100 MB/s`, so **1e9 B/s on 10 GbE, 1e8 on 1 GbE**); the two Providers' own terms are `mock_radio::Profile::terms()` (`profile.rs:46`: lead 2 ms, repeat max 268 435 456, align 2) and `ezsdr_radio_uhd::profile::terms()` (`profile.rs:20`: lead 5 ms = `DEVICE_LEAD_NS` 2 ms + `DELIVERY_ALLOWANCE_NS` 3 ms, repeat max 4 294 967 295, align 1). Everything else is one shared value. Comparable keys of a one-channel layout (rules from `measurement.rs`):

| Key | Rule | Mock | UHD |
|---|---|---|---|
| `radio.timing.min_timed_command_lead_ns` | ≥ | 2 000 000 | 5 000 000 |
| `…startup_latency_ns` | ≥ | 2 000 000 000 | same |
| `…overflow_restart_gap_ns` | ≥ | 50 000 000 | same |
| `…rx_start_lead_ns` | ≥ | 50 000 000 | same |
| `…tx_in_flight_ns` | ≥ | 10 000 000 | same |
| `radio.perf.rx_bytes_per_s`, `tx_bytes_per_s` | ≤ | 1 000 000 000 (10 GbE) / 100 000 000 (1 GbE) | same |
| `radio.perf.wire_bytes_per_sample` | = | 4 | same |
| `radio.tx.repeat_max_samples` | ≤ | 268 435 456 | 4 294 967 295 |
| `radio.tx.repeat_align_samples` | multiple | 2 | 1 |
| `radio.{rx,tx}.c<n>.{frequency_hz,gain_db,antenna}`, `radio.{rx,tx}.s<k>.sample_rate_hz` | ⊆ | per board | same |
| `radio.{rx,tx}.c<n>.{frequency_step_hz,gain_step_db}` | ≥ | 1.0 Hz, 0.5 dB | same |
| `radio.{rx,tx}.c<n>.phase_behavior_on_retune` | ≤ strength | UBX/OBX `random_unless_timed_tune`, CBX `random` | same |
| `radio.{rx,tx}.c<n>.path_delay_samples` | = | tx 45, rx 0 | same |
| `radio.full_duplex`, `radio.hardware_time` | true⇒true | true | true |

## 2. Cross-cutting policies the per-key rows depend on

**P-load (needs the owner first, D0).** P3 (accepted 2026-10-16) says "worst of 2 boards and the H4 load, rounded to the strict side". H4 is a deliberately hostile load (16 busy workers + 2 memcpy workers on a 16-core host, `h4-load.txt`, CPU 99.99 %). Taken literally it gives `rx_bytes_per_s` = 2e8 (R2: 100 and 200 MS/s overflow) — refusing every receive above 50 MS/s for every user of every host. R2 itself suggests the limit is the host, not the device: the same 100 MS/s passes on the *reopened* 30 s stage in R2 (`reopened_provider_rx_throughput_100000000`, Success) and fails on the 60 s stage; I read that as host scheduling noise (INFERRED). My recommendation is an **asymmetric reading**: H4 binds the *time-type lower bounds* (a too-large value costs a few ms of pause or Mock pessimism, a too-small one lets a Spec pass on the Mock and run late on a busy host — and a busy host is the normal case for an experimenter who processes samples), but does **not** bind the *capacity* values, which come from unloaded records; the loaded result goes to the bench README and a KNOWN_LIMITATIONS entry.

**P-admission (D13).** Records enter `crates/ezsdr-radio/measurements/` only if (a) produced by the current measurement groups (no removed key, no deleted group), (b) `load: none` under the recommended P-load, (c) one per (motherboard, layout), the latest clean one at the stage's closing commit; every placed record binds the profile forever (the test compares all of them), so a record with a transient failure would lower the profile permanently. The bench dir keeps everything else.

**P-rounding.** P3 (3): no margin by default. A value is one of the stage ladder rungs measured (throughput rungs follow the rate grid: 200 / 100 / 66.7 / 50 MS/s …) or, for a time value, the maximum observed rounded up to the next whole ms (the loop-timing resolution is µs; the profile fields are ns).

## 3. Per-key rows

Format: **now (Mock / UHD)** · **measured** · **proposal and rule** · **what a user can no longer do** · **tests / Mock behaviour that change** · **coverage and V/I**.

### K1 `radio.timing.min_timed_command_lead_ns`
- **Now:** Mock 2 ms / UHD 5 ms.
- **Measured** (`.observations[name=provider_min_timed_command_lead].result.candidates[]`, condition `tx-1msps`, 100 trials per rung, rungs 0.5 / 1 / 1.5 / 2 / 3 / 5 / 10 ms): rungs 0.5–2 ms: ProviderFailure 100/100 in all five records; **3 ms: failures A 20, B 19, C 17, R1 15, R2 19 of 100**; **5 ms and 10 ms: Success 100/100 in all five** (R2 under H4 included). Value rows (`.values[key=…]`): 5 000 000 in all five, `trials 100`, condition `tx-1msps`.
- **Proposal: 5 ms for both Providers.** Rule: the maximum over records of the smallest rung that held 100/100 (= 5 ms; load makes no difference). UHD unchanged; Mock 2 → 5 ms; the x3xx `timing` takes the value itself (`ProviderTerms.min_timed_command_lead_ns` disappears, the 2 + 3 ms split stays a UHD-internal fact asserted by a test).
- **Users can no longer:** on the Mock, issue a command/burst 2–5 ms ahead and see it played (KL-16's first "When" item disappears; the Mock now judges it late as the USRP does).
- **Changes:** `Profile::terms()` (Mock), `x3xx::ProviderTerms`, `ezsdr-mock-radio/src/lib.rs:409,1629,1656` read the value (no code change beyond the number); tests pinning 2 ms: `ezsdr-mock-radio/tests/mock_radio.rs:535-568`, the fixture pair `mock-only-rejects.json`/`uhd-only-rejects.json` and `ezsdr-acceptance/tests/measurements.rs:67-111` (`…enforce_provider_specific_rejections`, `measurement_between_the_provider_leads_passes_only_uhd` — both exist only because the leads differ; rebuild them on another key, e.g. repeat align), `ezsdr-radio/tests/measurement.rs:22-27` (`mock_terms`/`uhd_terms`), `ezsdr-radio-uhd/src/profile.rs:33-37`, VH-8 harness expectations where Mock and UHD leads differ (`ezsdr-acceptance/tests/parity.rs`, `oracle/mod.rs:32`), Python tests that wait on the Mock's 2 ms (to find: run the suite).
- **Coverage:** VERIFIED for TX at 1 MS/s, one channel, 100 trials per rung; **nothing between 3 and 5 ms** (a 4 ms rung would tell whether 5 is tight); other TX rates and 2-channel layouts not measured — the cost is host-side (device lead 2 ms is raw-VERIFIED ≤ 0.2 ms unloaded: R1 `hw_b8_raw_device_lead`, late 0 down to 200 µs; R2 loaded: 1 of 8 late at 2 ms, 6 of 8 at 1.5 ms) so I expect it rate-independent (INFERRED). Sweep S5.

### K2 `radio.timing.startup_latency_ns`
- **Now:** 2 s, both. **Measured:** nothing; no observation in any record (searched the observation names of A, B, C, R1, R2 for `startup|arm|repeat|wire`: none; the one `startup` hit in `hw_measurements_record.log:68` is `T2 raw startup flush`, the TX open flush, 4 of 4 acknowledged). All records: `.values[key=…startup_latency_ns]` = `unmeasured`.
- **Proposal: keep 2 s, keep `unmeasured`**, reason changed to "owner decision, KL-15 (a policy value: every Run's T0 is ≥ 2 s after `arm`)". The record already allows this (an unmeasured row passes).
- **Users can no longer:** unchanged.
- **Coverage:** INFERRED (policy). `UhdRadio::with_measurement_timing` already accepts a candidate startup latency (`provider/mod.rs:885-936`), so a group could measure `arm`→first accepted `start` (sweep S9, optional) — the only user-visible gain would be a shorter wait before T0.

### K3 `radio.timing.overflow_restart_gap_ns` (together with RM-17)
- **Now:** 50 ms, both (UHD's `OVERRUN_RESTART_DELAY`, spec 07 line 20 "VERIFIED").
- **Measured** (`.observations[name=provider_overflow_restart_gap].result.rate_measurements[].trials[].restart_after_resume_ns`; 1 channel, rates 1 / 10 / 25 / 50 MS/s, stalls 20 / 2 / 1 / 0.5 s, `after_ms` 500, 5 trials per rate per record; every trial Success):

  | rate (MS/s) | unloaded A, B, C, R1 (ms) | loaded R2 (ms) |
  |---|---|---|
  | 1 | 73.886 … **85.443** (C trial 0; the other 19 trials 73.9–78.3) | 98.408 … 139.111 |
  | 10 | 69.053 … 73.758 | 89.538 … 121.076 |
  | 25 | 68.630 … 77.115 | 91.558 … 137.020 |
  | 50 | 68.609 … 77.417 | 89.678 … 105.053 |

  Owner's stability criterion (41 §8, 2026-10-27): (max − min)/max ≤ 0.5 at every rate. Unloaded 6–13.5 %, loaded 15–33 % → **met in both**. `maximum_restart_after_resume_ns`: A 78.255, B 74.542, C 85.443, R1 75.454, R2 139.111 ms. The solo `maximum_gap_ns` (13.97 s … 14.03 s) is the full gap incl. the 20 s stall and the socket drain — not a device value (the record's `gap_scope` says so). The value row stays `unmeasured` ("pending the first hardware bench reading and owner decision").
- **Proposal:** value = the maximum, rounded up to a whole ms: **86 ms unloaded, 140 ms under H4**; with the recommended P-load (H4 binds time-type values) **140 ms**. **And** RM-17/MR-19/MR-21 re-worded as 41 §8 already decides: the restart is `gap` after the instant the consumer takes blocks again, not after `f` (design step 0; both Providers call it; the MR-19 back-pressure path needs "the consumer resumed" = the first publish that is accepted after `Full`).
- **Users can no longer:** assume a receive overflow costs 50 ms; on the Mock an overflow now loses ≥ 140 ms after the consumer returns (pessimistic against the device's 69–85 ms unloaded; KL-8's "within about 67–75 ms" becomes "69–85 ms unloaded, 89–139 ms under full CPU load").
- **Changes:** `x3xx::timing`; `ezsdr-mock-radio/src/lib.rs:580,735` (the loss window), MR-19/MR-21 and RM-17 text, tests `mr_19_*`, `mr_21_*`, `vh_8_*overrun*`, `rm_17_*`; `ezsdr-radio-uhd/tests/fake.rs:159,505,517`; `ezsdr-mock-radio/tests/mock_radio.rs:545,564`; the measurement code must emit a measured row (`tests/common/measurement.rs`, the overflow group) and a **new record** carries it (placed records are not edited).
- **Coverage / V-I:** the estimate is INFERRED and high-side by its own note; rates 100 and 200 MS/s were not run; one channel only; the companion `…_with_tx` group (observation only) gives full gaps of 0.45 s at 50 MS/s with a 10 MS/s companion (R1) but no restart-after-resume; 5 trials per rate per record (20 per rate over four unloaded records, 5 loaded). Sweeps S4. Because the semantic change is a Vocabulary rule, I would stage the gap separately (see §6).

### K4 `radio.timing.rx_start_lead_ns`
- **Now:** 50 ms, both.
- **Measured** (`.observations[name=provider_rx_start_lead].result.candidates[]`, 1 channel at 1 MS/s, conditions `restart-same` and `restart-retune` (across the board's 1 GHz LO boundary, gain 3/6 dB alternating), rungs 3 / 5 / 10 / 25 / 50 / 100 ms, 100 trials each): R1 (#1, none): restart-same 3 ms Success 100; restart-retune 3 ms 100 failures (all START_LEAD_SHORT, see 0.1 #5), **5 ms Success 100**. R2 (#1, H4): restart-same 3 ms 23 failures, 5 ms 1; restart-retune 3 ms 100, **5 ms 23**, **10 ms Success 100 in both**. #2 UBX (`~/.cache/ezsdr-bench/2026-11-13/hw_n47_rx_start_lead_group.37.log:7-8`, `Measured … worst: 5000000, trials: 100, condition: "restart-retune"`; not a record file): 5 ms. Value rows: R1 5 000 000, R2 10 000 000 (`.values[key=…rx_start_lead_ns]`, condition `restart-retune`). A/B/C rows (1, 1, 3 ms) come from the deleted run-time rate-transition group: stale.
- **Proposal: 10 ms** (rule: maximum over (boards × loads) of the smallest rung with 100/100; P-load = H4 binds). Alternative 5 ms (quiet host only).
- **Users can no longer:** unchanged in capability; a receive restart's pause shrinks (KL-7: one receive call + 3 ms allowance + start lead: ~55 ms → ~15 ms at 1 MS/s with the default block).
- **Changes:** `x3xx::timing`; Mock restart origins (RM-25) — `ezsdr-radio/tests/timeline.rs` (20 hits of 50 ms literals, mostly synthetic timings), `ezsdr-mock-radio/tests/mock_radio.rs:540-568`, `mock_layout.rs`, `mock-radio/tests/timeline.rs`, `ezsdr-radio-uhd/tests/fake.rs:159,505-517`, `src/provider/rx.rs`, `control.rs`; **Python**: `python/tests/test_easy_api.py:738` (`sleep(0.062)`, the 53 ms pause), `:768` (`lead = 50_000_000`), `:95`, `:644`; `design/16-easy-api.md` test table line 206 (sleep 0.062); `python/ezsdr/session.py:360`; KL-7 and KL-15 text.
- **Coverage:** 1 MS/s, one channel, #1 OBX (record) and #2 UBX (log); **not covered: other rates, 2 channels, CBX/Basic/LF boards, a restart that retunes both LOs**. The configure cost (LO settle ~60 µs, UHD read-backs) grows per channel, so a 2-channel layout could need more (INFERRED). Sweep S5. VERIFIED for the covered condition.

### K5 `radio.timing.tx_in_flight_ns`
- **Now:** 10 ms, both. **Measured:** R1/R2 `unmeasured` (the Provider group was deleted in step 5; reason text in `.values`). Older A/B/C: `provider_tx_in_flight` 5 ms smallest 100/100 rung (3 ms: 99, 99, 94 of 100; ≤ 2 ms: ProviderFailure 99 + 1 Unattributable) — group relied on preemption, gone, pre-note-48 Module. Raw (`hw_b8_raw_in_flight_window`, 1 trial per window): R1 all windows 0.25–10 ms clean; **R2 (H4): 10 and 5 ms clean; 3, 2, 1, 0.5 ms 37–39 TimeError + 1–2 Underflow; 0.25 ms 0 TimeError, 1 Underflow**. Bench verdicts (`hw_b8_preemption.36/37.log` lines in `bench/2026-11-06-n48/results/verdicts.tsv` and `…-n48s3/results/verdicts.tsv`, both boards): preempt leads 20 and 10 ms 0 `TIME_ERROR`, 5 and 3 ms one late — that checks the Module obeys its own W, not the device (not re-run in the 11-13 session).
- **Proposal: keep 10 ms, row stays `unmeasured`** (INFERRED, ≥ 2× margin over the loaded raw minimum of 5 ms). A relaxation to 5 ms would need a new Provider-level group built on Stop (handoff §2 item 2); I do not propose it.
- **Users can no longer:** unchanged (a burst must start ≥ 10 ms after the receipt that books it while another plays: KL-13/36/37/39 text stays).
- **Tests:** none change. Mutation row `P625` anchors `tx_in_flight_ns: 10_000_000,` in `x3xx.rs` and keeps matching.

### K6 `radio.perf.rx_bytes_per_s`
- **Now:** 1 000 000 000 on 10 GbE (100 000 000 on 1 GbE), both.
- **Measured** (`.observations[name=provider_rx_throughput_<rate>].result.stage` and the `provider_rx_throughput_<condition>` ones; 60 s, **1 trial per stage per record**; value row = largest T such that every condition with a stage ≤ T succeeded, design 18 line 405):

  | Record | one channel | two channels (B, C: UBX+CBX) | value row |
  |---|---|---|---|
  | A | ✓ to 200 MS/s | — | 8.0e8 |
  | B | ✓ to 200 MS/s | split 100+100 ✓, grouped 100×2 ✓, mixed 200+100 ✓ (grouped/split 200×2 above the 1.25e9 ceiling: not run) | **1.2e9** |
  | C | ✓ to 100; **200 ✗ (rx_overflow true)** | grouped 100×2 ✗; split 100×2 ✓; mixed 200+100 ✓ | 6.0e8 |
  | R1 | ✓ to 200 (also reopened, 30 s) | — | 8.0e8 |
  | R2 (H4) | ✓ to 50; **100 ✗, 200 ✗ (rx_overflow)**; reopened ✓ to 100, 200 ✗ | — | **2.0e8** |

  Also (bench-results.md:361, 2026-11-06, 600 s each, `4f7e9e4`): 200 MS/s receive PASS on #2; on #1 the device side was complete but the Module's feed dropped 46 blocks (a Module issue, not capacity). NIC counters (`provider_duplex_at_the_measured_limits.result.nic_bytes`): B's 1.8e10 samples over 60 s crossed the link at 1.209 GB/s (96.7 % of the 1.25 GB/s line rate): the link, not the cap, is the next wall.
- **Proposal: 8.0e8 B/s on 10 GbE** (one channel at 200 MS/s; admits 100+100 MS/s, refuses 200+100 and 200+50). Rule: the unloaded value of the current code; the stale C failure and the H4 failure do not bind (P-load, P-admission). It is conditional on S1–S3 (below): if 10 of 10 two-channel runs of mixed 200+100 hold on #2 at the current code, 1.2e9 becomes the proposal (it is the largest stage ever held, and 96.7 % of the line); if one-channel 200 MS/s fails ≥ 1 of 10 per board, the proposal drops to the next rung that held (4.0e8, 100 MS/s). The 1 GbE value (100 MB/s) is INFERRED (the HG port 0; never measured).
- **Users can no longer:** with 8e8: ask for RX streams summing above 200 MS/s on one link (200+100, 200+50 MS/s, 3 channels at 100 MS/s…); single-channel unchanged. With 2e8 (P3 literal): any single stream above 50 MS/s.
- **Changes:** `x3xx::performance` becomes a table by link (it is no longer `link_gbps × 100 MB/s` once rx ≠ tx); tests: `ezsdr-radio/tests/channels.rs:328,332,444,667,752`, `ezsdr-mock-radio/tests/mock_radio.rs:502,550,568`, `ezsdr-radio-uhd/tests/fake.rs:520`, `rm_32_throughput_cases` (spec 07 line 418), `ezsdr-radio-uhd/tests/hardware.rs:3713` comment, KL-14 (the "1 GB/s, 250 MS/s" workaround text), `plan/phase3/patches` untouched. The measurement ceiling (`multichannel.rs:769`, 1.25e9 candidate) is separate and stays.
- **Coverage:** one board for two channels; two-channel evidence is from code before TXS, #78 and the RX owner rewrites (stale for 0.1 #2); one 60 s trial per stage per record. **Not covered:** XG/two links (`links: 2` shares one link's cap by design R13), 1 GbE, Basic/LF boards, disk-bound capture. INFERRED for the host: "under load the limit is the host". Sweeps S1–S3.

### K7 `radio.perf.tx_bytes_per_s`
- **Now:** 1 000 000 000 (10 GbE).
- **Measured** (`provider_tx_throughput_<rate>`, 60 s, 1 trial): one channel — A ≤ 50 ✓, 100 ✗, 200 ✗ (pre-TXP); B (pre-TXP) 10 ✓, 20 ✗, 25 ✗, 40 ✓, 50 ✗, 100 ✓, 200 ✗ (non-monotone); C ≤ 100 ✓, 200 ✗; **R1 ≤ 100 ✓, 200 ✗ (Underflow + TimeError); R2 the same**. Two channels (C, before TXS): split 100×2 ✓ (8e8 B/s), mixed 100+50 ✓, mixed 200+100 ✗, grouped ≤ 25×2 ✓ and **40×2 ✗**. Value rows: A 2.0e8, B 4.0e7, C 2.6e8, R1 **4.0e8**, R2 **4.0e8**. Further (bench-results.md:284-293, 361): after TXS one channel at 200 MS/s passed 300 s twice (#2, 2026-10-29) and 300 s again on 11-06, but 3 of the 6 runs of 200 MS/s transmit on 11-06 (600 s solo and duplex, both boards) ended in one Underflow after 1.5–159 s ("stable it is not"). R1's duplex confirmation (RX 200 + TX 100 MS/s together, 60 s): **DeviceFailure, TX flow 0.309 of the duration**, RX complete (`.observations[name=provider_duplex_at_the_measured_limits].result`); R2's RX 50 + TX 100 held.
- **Proposal: 4.0e8 B/s on 10 GbE** (100 MS/s one channel). Rule: the one-channel value of the current code on both boards, unloaded and loaded alike (the same in R1 and R2); conditional on S1/S2 (10 trials at 100 and at 200 MS/s per board). 1 GbE stays 1e8 (INFERRED).
- **Users can no longer:** ask for TX above 100 MS/s total on a link (200 MS/s one channel, 2 × 100 MS/s split — which C once held — , 2 × 66.7 MS/s). KL-5 changes from "awaiting bench" to a Ceiling with a number; "RX 200 + TX 100 at once" stays a documented Ceiling (the profile has no joint key).
- **Changes:** as K6, plus `KL-5`, `KL-14`.
- **Coverage:** single channel on #1 at the current code (R1, R2: one 60 s trial each at 100 MS/s); #2 only before TXS/#78; two channels only before TXS. The Σ-cap model is conservative (finding 7). Sweeps S1, S2, S3.

### K8 `radio.perf.wire_bytes_per_sample`
- **Now:** 4 (sc16), both. **Measured:** `unmeasured` ("independent measurement or channel-specific calibration is required"). Independent evidence (not a value row): NIC counters per sample in the duplex stage — A 4.032 rx / 4.053 tx, B 4.029 / 4.065, C 4.034 / 4.052, R1 4.031 / 4.036, R2 4.051 / 4.033 B/sample (bytes ÷ the streams' `actual_samples`), i.e. 4 B plus 0.7–1.6 % Ethernet/CHDR framing; and UR-3 (`otw_format` sc16, UHD source).
- **Proposal: keep 4, keep `unmeasured`** with the NIC figures in the reason. The rule is `Equal`; no honest measurement yields exactly 4.000 from a counter, and the streamer's wire format is a property read from UHD, not a bench result. Both Providers read one shared constant (no drift possible).
- **Users can no longer:** unchanged. **Coverage:** VERIFIED within 1.6 % for sc16 on 10 GbE at 1–200 MS/s.

### K9 `radio.{rx,tx}.c<n>.path_delay_samples` (tx 45, rx 0)
- **Measured** (`.observations[name=hw_b6_delay_by_rate].result.rates[]`, correlation delay of the *loopback total*): 0.4 MS/s 35, 0.5 MS/s 44, 1 MS/s **44**, 2 MS/s **36** samples (A, C; R1 and R2 only 1 and 2 MS/s, same values); no TIME_ERROR. The record rows are `unmeasured`: the total cannot be split between TX and RX (41 §2 N7), and the rule is `Equal`.
- **Proposal: keep 45/0, keep `unmeasured`.** The truth is rate-dependent (44 at 1 MS/s, 36 at 2 MS/s — 9 samples off at 2 MS/s), which a scalar cannot express; a per-rate table (long-term correct, schema/spec change, a loopback sweep over every TX decimation structure: ~30 distinct (halfband, CIC) shapes, ≥ 5 trials each) is its own step and I do not propose it for stage 4. KL-16's path-delay sentence stays.
- **Users can no longer:** unchanged. **Coverage:** VERIFIED only at 4 rates; Mock = 45 at every rate (INFERRED for others).

### K10 `radio.{rx,tx}.c<n>.phase_behavior_on_retune`
- **Now:** UBX/OBX `random_unless_timed_tune`, CBX `random`. **Measured:** `unmeasured` in all records. Raw (`.observations[name=hw_b8_raw_timed_tune_phase].result.cases[]`, 6 phases per case): at 1.0 GHz timed spread 0.07 / 0.07 / 0.24 / 0.07 / 0.09° (A, B, C, R1, R2); at 1000.1234 MHz timed **0.04 / 0.04 / 126.55 / 0.06 / 0.03°** — in C (#2 UBX) one of six timed retunes jumped (47.98° → −78.57°); untimed 121–177° at the fractional frequency. Counting both timed cases of the five records that is 60 timed retunes, 30 of them at the fractional frequency, with one outlier. Plus the 2026-10-27 shake-outs on #1 OBX: timed spread 99°, 137.5°, 156° in three runs (0.8° in the fourth) (bench-results.md:101).
- **Proposal: keep, `unmeasured`.** Two flags for the owner, neither a stage-4 value: (a) these are raw (Device-level) observations; since note 47 the Provider has no timed-tune form (KL-3), so the `random_unless_timed_tune` claim is about a capability no Provider path can use today, while MockRadio already redraws per begun segment (MR-34); (b) the timed case is not unconditionally deterministic (1 outlier in 30 fractional-frequency timed retunes of the five records, plus the three bad 2026-10-27 shake-out runs). I recommend recording this for the freeze review (handoff §4) rather than weakening the profile now.
- **Coverage:** 60 timed retunes at two frequencies on two boards (OBX, UBX); CBX never; no sweep proposed unless the owner wants the claim measured (≥ 30 timed retunes per board at each of 3 fractional frequencies, `hw_b8_raw_timed_tune_phase` with a larger count, no new code).

### K11 `radio.tx.repeat_max_samples`, `radio.tx.repeat_align_samples`
- **Now:** Mock 268 435 456 and 2 (RFNoC Replay's limits, spec 09 line 54); UHD 4 294 967 295 and 1 (host-streamed repeat, UR-22). **Measured:** `unmeasured` in all records; no observation (searched). The hardware has played repeats of 1 000 and 20 000 samples (TX200 probes) and the C2 switch test; no long or odd-length repeat.
- **Proposal: leave both Providers' values and the rows `unmeasured`.** The Mock is *stricter* than the UHD Module (a Spec valid on the Mock is valid on the USRP; the rules allow stricter), and KL-16 already lists it as a Ceiling ("keep repeats of even length ≤ 2^28"). Levelling the Mock *up* to the UHD's 2^32−1 would claim a limit (34 GB of cf32) nobody has run, i.e. a Mock possibly looser than hardware; levelling the UHD *down* to the largest measured length needs a new group (S8).
- **Users can no longer:** unchanged. **Coverage:** INFERRED (a `u32` limit, host RAM).

### K12 Grids and steps (unchanged; for completeness)
`radio.{rx,tx}.c<n>.frequency_hz` ⊆: measured `Step {lo, hi, step 1.0}` over 200 sampled points, 10 MHz–8.4 GHz (OBX), 10 MHz–6 GHz (UBX), 1.2–6 GHz (CBX); `…frequency_step_hz` measured 0.0 (profile 1.0 ≥); `gain_db` 0–31.5 in 0.5 dB (316 trials), `gain_step_db` 0.5; antennas RX `RX2`,`TX/RX`, TX `TX/RX`; `radio.{rx,tx}.s<k>.sample_rate_hz` measured 639 values (profile's are a subset; TX profile is the ≥ 1 MS/s subset, KL-40). All pass today; R1/R2 are OBX only. **No CBX record at the current code** (the owner removed the CBX on 2026-11-12) and no Basic/LF board has ever been measured: those claims have no record to be compared with.

### K13 `radio.full_duplex`, `radio.hardware_time`
true/true; measured true in 20/20 trials (condition `duplex-1msps`) in all five records. No change.

### K14 Link speed variants
`performance(1)` = 1e8 B/s each way (INFERRED: 80 % of 125 MB/s; never measured — every record has `link_gbps: [10]`); `links: 2`/XG shares one link's cap by design (design 07 R13). Not changed; D12.

## 4. Where the evidence does not cover the value's conditions, and the sweeps

AGENTS §9: a value applied beyond its measured conditions needs a sweep plan, ≥ 10 trials per cell near a boundary, reviewed before the value is decided. The sweeps below are bench-only (no `src/` change except where marked), all with the attenuators confirmed by the owner before any transmit.

| Sweep | Question | Cells | Notes |
|---|---|---|---|
| **S1** | Current-code records on **both** boards, unloaded, repeated | #2 UBX (+CBX back in slot B for S3) and #1 OBX, `hw_measurements_record` ×3 each (~40 min per one-channel run) | P5 (1): both boards. There is no #2 record at the current code at all (R1/R2 are #1 only). Gives 3 value rows per board. |
| **S2** | One-channel throughput boundary | RX at 100 and 200 MS/s ×10 × 60 s per board; TX at 100 and 200 MS/s ×10 × 60 s per board; duplex RX 200 + TX 100 ×5, RX 100 + TX 100 ×5 | Decides K6 (800 vs 400 MS/s-rung) and K7 (100 vs 200 MS/s), and whether a joint Ceiling entry is needed. Needs a loop group in the test tool (small, test-side only). |
| **S3** | Two-channel throughput at the current code | #2 UBX+CBX: RX mixed 200+100, split 100+100, grouped 50+50; TX split 100+100, grouped 25+25, mixed 100+50; ×5 each, 60 s | Needs the owner to reinstall the CBX. Decides whether the cap is 8e8 or 1.2e9 (RX) and whether TX needs a per-stream limit. |
| **S4** | Overflow restart gap | rates 1/10/25/50/**100/200** MS/s × 20 trials per board, unloaded and H4; stalls 0.1/0.5/2 s at one rate to see whether the post-resume time depends on the stall length; if the UHD metadata allows, take the restart sample from the device time instead of the estimate | Replaces the INFERRED high-side estimate. Needed before RM-17's rewrite is final. |
| **S5** | Receive start lead and command lead beyond 1 MS/s / 1 channel | `restart-retune` and `restart-same` at 10 and 100 MS/s and, with the CBX, two channels; rungs 3 / 4 / 5 / 7.5 / 10 ms ×100; command lead rungs 3.5 / 4 / 4.5 / 5 ms ×100 at TX 1, 10, 100 MS/s | Existing groups with new plan values (`MeasurementPlan::rx_start_leads_ns`, `tx` leads); no `src/` change. |
| S6 | (optional) 4 ms between the two rungs | included in S5 | |
| S7 | 1 GbE (HG port 0) throughput | RX/TX 10, 20, 25, 30 MS/s ×5 × 60 s | Needs a 1 GbE NIC on the host; owner decision D12. |
| S8 | (optional) Repeat limits | repeat lengths 1, 3, 1001, 2^20, 2^24, 2^28 at 10 MS/s, one wrap + seam check | Only if the owner wants K11 measured; needs a new group. |
| S9 | (optional) Startup latency | `arm`→`start` accepted with candidates 0.1/0.25/0.5/1 s ×50 (uses `with_measurement_timing`) | Only if the owner wants K2 lowered. |
| S10 | (optional) `tx_in_flight_ns` on Stop | a Provider group built on `Stop` (not preemption), windows 3/5/7.5/10 ms ×100 | Only if the owner wants K5 relaxed. |

Time budget I would ask for: S1+S2+S5 ≈ 4–5 h of bench with both boards (the 11-15 session); S3+S4 a second session once the CBX is back; the rest optional.

## 5. Owner decisions

Each: **recommendation** · alternatives · what a user sees.

**D0 — Load policy (decide first; changes K4, K3, K6).** *Recommend:* H4 binds time-type lower bounds only; capacity values from unloaded records; loaded results in the bench README + a KNOWN_LIMITATIONS entry (extend KL-4 "Host and UHD setup": under full CPU load, receive above 50 MS/s overflowed on #1). *Alternative A:* P3 literally (H4 binds everything): rx 2e8 (a stream above 50 MS/s is refused at validate() for everyone), start lead 10 ms, gap 140 ms. *Alternative B:* H4 never binds: start lead 5 ms, gap 86 ms, rx 8e8 — a busy host sees START_LEAD_SHORT warnings (23 of 100 at 5 ms in R2) and longer real gaps than the Mock models. This is a change to the accepted P3 → it is the owner's.

**D1 — `rx_bytes_per_s` (10 GbE).** *Recommend:* 8.0e8 now, to be confirmed by S1–S3 under the stated rule (1.2e9 if two-channel 200+100 holds 10/10 on the current code; 4.0e8 if one-channel 200 MS/s fails ≥ 1/10 on a board). *Alternatives:* 6.0e8 (strict "all records", fed by C's stale single failure; refuses 200 MS/s), 1.0e9 (status quo: looser than A and R1 — the comparison fails), 1.2e9 now (rests on B and C before TXS), 2.0e8 (D0 A). *User:* with 8e8 two-channel sums above 200 MS/s are refused; one channel unchanged.

**D2 — `tx_bytes_per_s` (10 GbE).** *Recommend:* 4.0e8 (100 MS/s). *Alternatives:* 2.0e8 (A's value, pre-TXP — stale), 8.0e8 (hardware failed 200 MS/s in R1, R2, C, in 3 of 6 runs on 11-06; admission would be looser than the device — against §3), per-stream limit key (a Vocabulary change; decide after S3). *User:* TX above 100 MS/s is refused at validate() instead of underflowing at run time (KL-5: Ceiling with a number); runs that sometimes held 200 MS/s on #2 no longer start.

**D3 — `min_timed_command_lead_ns`.** *Recommend:* 5 ms for both. *Alternatives:* Mock stays 2 ms (fails every record), both 3 ms (15–20 % of Provider trials fail at 3 ms), both 10 ms (no evidence need). *User:* on the Mock a command 2–5 ms ahead is judged late (as on the USRP); Specs that relied on 2–5 ms leads on the Mock must change; the 2/5 ms differences in KL-16 disappear.

**D4 — `rx_start_lead_ns`.** *Recommend:* 10 ms. *Alternatives:* 5 ms (D0 B; 0.57 ms margin over the warning threshold on a quiet host; warnings under load), 50 ms (status quo: passes the comparison but keeps a ~55 ms pause for no measured reason; P3 (4) says relax to measured). *User:* every receive restart pauses ~15 ms instead of ~55 ms; Python tests that assume the 53 ms pause change.

**D5 — `overflow_restart_gap_ns` and RM-17.** *Recommend:* adopt the owner's 2026-10-27 plan (criterion is met): value 140 ms (86 ms if D0 B), RM-17 re-worded "after the consumer reads again", **staged as its own design step** (spec 07/09, Mock MR-19/MR-21, VH-8), with S4 first. *Alternatives:* keep `unmeasured` and exclude the key from parity (41 §8's other branch) — the Mock then still loses 50 ms where the device loses 69–139 ms; status quo 50 ms (fails the comparison once the row is measured). *User:* after a receive overflow the Mock's loss window and `RX_OVERFLOW.lost` grow ~3×; KL-8 text changes.

**D6 — `tx_in_flight_ns`.** *Recommend:* keep 10 ms, `unmeasured`. *Alternative:* build a Stop-based group (S10) and relax to 5 ms. *User:* unchanged.

**D7 — `startup_latency_ns`.** *Recommend:* keep 2 s (KL-15 says settled), `unmeasured`. *Alternative:* S9 and lower it. *User:* unchanged.

**D8 — `wire_bytes_per_sample`.** *Recommend:* keep 4, `unmeasured`, NIC figures in the reason. *Alternative:* a measured row derived from the streamer's wire format. *User:* unchanged.

**D9 — `path_delay_samples`.** *Recommend:* keep 45/0, `unmeasured`; KL-16 keeps the 35/44/44/36 note. *Alternatives:* 44 (the 1 MS/s value), a per-rate table (a separate long-term step). *User:* unchanged.

**D10 — `phase_behavior_on_retune`.** *Recommend:* keep, `unmeasured`; list the claim in the freeze review (no Provider timed-tune form since note 47). *Alternative:* weaken UBX/OBX to `random` now (Mock already behaves so; Specs that constrain on the stronger value stop matching). *User:* unchanged.

**D11 — `repeat_max_samples` / `repeat_align_samples`.** *Recommend:* leave as they are (Mock stricter), `unmeasured`. *Alternatives:* level the Mock up to the UHD's (a Mock possibly looser than hardware), or measure and level both (S8). *User:* unchanged.

**D12 — 1 GbE and XG.** *Recommend:* keep 1e8 B/s INFERRED for `link_gbps: [1]`, one link's cap for `links: [2]`; list both in KL-6/KL-14; measure only if the owner can attach a 1 GbE NIC (S7) or an XG image (BP4c). *User:* unchanged.

**D13 — Which records are placed.** *Recommend:* P-admission (§2): only R1-style records from the stage-4 closing commit, one per (board, layout), `load: none`; A, B, C stay in `bench/`; a conversion script writes `observations: []` and stamps the real `args`. *Alternative:* place R2 as well (forces D0 A), or place everything (the unknown-key and stale-group rows fail the test). *User:* none directly; it decides what binds the profile.

## 6. Implementation outline (the step after the decisions)

The step is **risky** in §9's sense (it changes deadlines the Provider and the Mock both read, and the boundary between the Mock and the hardware it stands for). Recommended staging, each stage leaving the tree green:

0. **Design (§9 step 0)** for K3 only (RM-17/MR-19/MR-21 and the measured quantity): Opus draft, owner direction, Fable review. K1, K4, K6, K7 are values, not rule changes, and need no step 0.
1. **Bench (S1, S2, S5, then S3, S4)**; bench session dir `bench/<date>-stage4/` with `conditions.json`, `results/`, `README.md`, `raw.txt` (§9 bench records). The measurement groups change only in plan values and the loops of S2/S4 (test-side; FakeDevice rehearsal for each, §9 hardware-test rule).
2. **4a — values** (files): `crates/ezsdr-radio/src/x3xx.rs` (`timing`, `performance` as a per-link table, `ProviderTerms` loses the lead), `crates/ezsdr-mock-radio/src/profile.rs`, `crates/ezsdr-radio-uhd/src/profile.rs` (the 2 + 3 ms identity test), `crates/ezsdr-radio/measurements/<date>-<serial>-<config>.json` (converted records), `plan/phase8/tools/` (conversion script), design 07/09/18 tables (MR-3, UR-9, §3 values; no RM-17 yet), KNOWN_LIMITATIONS KL-4/5/7/14/15/16 (in the same commit, AGENTS §2), `plan/phase8/bench-results.md`, handoff.
3. **4a tests** (rewrite, do not delete): the value pins listed in K1/K4/K6/K7; the two acceptance fixture tests rebuilt on another key; a test that Mock and UHD Descriptions agree on every comparable key except the repeat pair (a differential for the profile, the `d3_…` test extended); the comparison test with the placed records (it exists); Python tests of K4. **Audit table** for every test whose expectation comes from a changed rule (§9 step 1): hardware tests that read the profile (`hardware.rs:713`, `2464`, `2653`, `2708`, the n47/n47s5 groups) listed with still-right / fixed / cannot-tell. **FakeDevice rehearsal** for each hardware test whose expected value follows a changed rule.
4. **Mutation rows:** `P625` keeps matching (`tx_in_flight_ns: 10_000_000,`); new rows on `x3xx::timing` values and on the `performance` table (smallest unique anchors, Rust token anchors) and on the converted records' reading; run the touched rows, then the full lists at the step's local commit (§7), via `remote_mutate.py` only.
5. **4b — K3** after step 0 and S4: RM-17/MR-19/MR-21, Mock `apply_rx_fault` and the back-pressure path, VH-8, the measurement group emitting a measured gap row, a new record, KL-8, spec text. Until 4b the gap row stays `unmeasured` and 4a is green.
6. **Gate:** `gate.py --mutation all --base <pre-change commit>`; Python 3.14 and 3.9 suites (the K4 tests); `rules.py`; schema freeze unchanged (no schema change: profile values are not in `measurement.v1`/`envelope.v1`).
7. **Records:** step record (brief, test plan, dispositions, `gate.json`) in `plan/phase8/reviews/`; failure-mechanism counts matched against `plan/maintenance/failure-mechanisms.md` — I see no new mechanism in this stage, but "a rule implemented twice (the lead in two `terms()`)" is the existing `radio-rule-implemented-twice` family and K1 removes one copy.

## 7. Searches behind the negative claims

- "No record has a startup/arm/repeat/wire observation": Python over `observations[].name` of the five raw records with regex `startup|repeat|wire|arm|align|queue|phase|delay` — hits only `hw_b8_raw_queue_depth`, `raw_command_queue_depth` (A, B, C), `hw_b8_raw_timed_tune_phase`, `hw_b6_delay_by_rate`.
- "`crates/ezsdr-radio/measurements/` does not exist": `ls crates/ezsdr-radio/measurements` → no such directory.
- "No mutation row anchors a profile value other than `P625`": regexp over `plan/maintenance/tools/mutations.json` rows touching `x3xx.rs`, `profile.rs`, `rx_bytes_per_s`, `overflow_restart_gap_ns:`, `ProviderTerms` → P01–P03, P50, P51, P70–P72, P625, P785, P943, N49F-08/09/20; only P625 anchors a value literal.
- Test-impact lists are grep-based (literal `50_000_000`, `2_000_000`, `bytes_per_s`, `ProviderTerms`) and an upper bound; the real list is what `cargo test` and the Python suites report after the change.
