# #125 and #127 (n125) — reviews

Sub-step review of a step that changes behaviour: GPT-6.1 Terra and an Opus re-check reviewer side by side, same prompt; after the fix round a Terra re-check.

## Terra

**Verdict: FAIL**

- **Major — competing defects produce different first refusals.** MockRadio (`crates/ezsdr-mock-radio/src/lib.rs:1659`) checks metadata and repeat limits before waveform bytes; UHD (`crates/ezsdr-radio-uhd/src/provider/control.rs:436`) checks bytes first. For example, submit a burst with valid target, clock and size, invalid metadata, and an absent input. Mock reports MR-16 for metadata; UHD reports the RM-13 missing-input reason. An oversized repeat with absent bytes has the same ordering discrepancy. Align the checks and add competing-defect tests for both providers.

- **Minor — the test input stores leak across tests and Runs.** mock_radio.rs (`crates/ezsdr-mock-radio/tests/mock_radio.rs:37`) and mock_layout.rs (`crates/ezsdr-mock-radio/tests/mock_layout.rs:76`) fall back to process-global maps populated by waveform helpers. An input registered by another test is therefore readable in a Run that did not register it, potentially hiding a missing-input sequence and making results depend on test order. Use per-harness stores.

- **Minor — stale test explanation and implementation detail in the specification.** mock_channel.rs (`crates/ezsdr-mock-radio/tests/mock_channel.rs:422`) still says only channel mode reads waveforms. RM-13 (`design/07-radio-model.md:149`) names the Rust helper and says both providers call it; that is implementation detail in normative rule text.

The shared check is called once on each validly addressed receipt path before booking; channel mode scans for finiteness and then decodes and clips, without a second refusal decision. The new plain-Mock test would fail against the old behavior. The listed mutation anchors target the intended decisions, although the UHD integration test exercises non-finite bytes rather than absent and wrong-length inputs.

`rules.py --base ea7bc5c1` passed. Searches found no stale removed reason texts in current code or Python tests; historical phase 3 records retain them. The 3 ms production allowance is shared, while the separate 2 ms UHD device lead can drift if changed independently, but the existing equality test detects that drift when run. I did not run builds or mutation rows. GitHub issue access was unavailable, so the review used the supplied issue descriptions.
## Opus

# Review of #125 / #127 (work/n125 vs ea7bc5c1): Opus re-check reviewer

**Verdict: PASS.** No major findings. There are five minors and some notes, all cheap to fix before landing.

## Failure scenarios (my list, then checked against the diff)

I wrote this list after the brief and the review prompt, and after a first look at the diff stat. I did not write it before reading any code.

1. A Mock receipt path for `TxBurst` that skips the shared check: another dispatch arm, the layout mode, the `ideal` profile, or a repeat or switch burst. **Checked:** there is one dispatch arm (`lib.rs:1557` → `handle_tx_burst`), and switch, repeat, untimed and `ideal` bursts all go through `lib.rs:1672`. UHD likewise has one path, `control.rs:262` → `book_burst`, with the check at `control.rs:436`. Holds.
2. A side effect before the refusal on the Mock, such as a booking, a prune or a sequence number. **Checked:** every step before `lib.rs:1672` only reads state. `prune` and `settle_switches` come after the check. Holds.
3. Channel mode decodes differently. The non-finite check moved out of `decode_waveform`, and the clip and its count are unchanged. In channel mode, `decoded` is `Some` exactly when it was before. Holds (see N1 on the second pass).
4. A burst with several defects gets a different first reason on each Provider. **Found:** m1.
5. `ctx.inputs` is a snapshot, so a Session's later bytes would be absent and every Mock Session burst would be refused. **Checked:** it is not a snapshot. The Kernel hands both Providers the live `Arc<Mutex<…>>` store (`coordinator/pipeline.rs:176,591`), and `ezsdr-server` and `ezsdr-acceptance` pass. Holds.
6. Through the Kernel, the absent and wrong-length cases cannot reach a Provider, because RS-44a refuses them first (`coordinator/admission.rs:124-131`). So the user-visible part of #125 is the non-finite case only. The other two are defensive and reachable only from a harness. That is why `ur_21_txburst_refusals` and row N125-2 exercise only NaN. This is acceptable, but the brief's "one per Provider" is met on UHD by NaN alone.
7. The state lifecycle of the new `inputs` field. **Found:** m2.
8. The two leads disagree (#127). **Checked:** see N2.

## Minor

- **m1: the refusal order differs between the Providers for a burst with several defects.** The Mock checks metadata (`lib.rs:1661`) and the repeat length and alignment (`lib.rs:1667-1670`) before the bytes (`lib.rs:1672`), which is the order MR-32 states. UHD checks the bytes (`control.rs:436`) before the repeat maximum (`control.rs:450`) and the metadata (`control.rs:453`), and UR-21 states no order.
  - Sequence: on the x3xx Mock and on UHD, `tx_burst` with metadata `{"foo": 1}` and one NaN sample.
  - Outcome: the Mock answers `MR-16: target, clock, waveform size, channel count or metadata is invalid`; UHD answers `RM-13: a waveform sample is not finite`. Both refuse, so neither is looser.
  - This predates the change: the old UHD had the same order. Moving UHD's bytes check after its repeat and metadata checks, which is a two-block move, would align the order with MR-32's. It would also skip decoding a burst that is about to be refused.
- **m2: `cleanup()` does not clear the new `self.inputs`** (`lib.rs:2479-2503`). It clears `channel` and `medium`, which used to own the store, but the new field keeps the finished Run's store, and its waveform bytes, alive until the next `prepare`. The fix is one line: `self.inputs = None;`.
- **m3: a design test row still names the old texts.** `design/09-mock-radio.md:359` (`mr_32_waveform_refusals`) reads "refused in that order with MR-32's two reasons and RM-13's". All three are RM-13's now.
- **m4: no *Changes* rows.** The texts and the scope of RM-13, MR-32 ("in every mode") and UR-21 changed, which a user can see in the refusal reasons. Yet `design/07`, `09` and `18` get no row in their closing *Changes* tables, which AGENTS §2 asks for history. `rules.py` does not check this, and #117 added none either. It is still worth a row each, citing #125. #127 changes no design text, but `design/09:56` still gives the source as "the emulated UHD Module's (spec 18 UR-14)", and the value now lives in `ezsdr_radio::x3xx` (see N4).
- **m5: test scaffolding duplicated as a process-wide static.** The block made of `ZEROS` (a `static Mutex<BTreeMap>`), `struct Inputs` and its `InputStore` impl is copied in `tests/mock_radio.rs:36-48` and `tests/mock_layout.rs:75-87`.
  - Hidden sequence: every `waveform()` call silently registers its bytes, for every test in the binary, across threads. In `mock_layout.rs`, `Inputs(inputs)` falls back to `ZEROS`, so a layout test that wants "absent bytes" in channel mode finds them present unless it builds its own hash. The new Mock test avoids this by using `b"never stored"`.
  - In `mock_radio.rs`, `Inputs.0` is always empty (`Inputs(BTreeMap::new())` at lines 326, 706 and 733), so the field is dead there.
  - It works, but a unit `struct Zeros;` reading the static would be leaner in `mock_radio.rs`. A comment on the fallback in `mock_layout.rs` would keep the sequence visible.

## Notes

- **N1:** both Providers now read the bytes twice: once for finiteness in `waveform_bytes_refusal` and once to decode (`channel::decode_waveform` on the Mock, `control.rs:443-450` on UHD). Results are unchanged. On UHD, the extra pass runs before `self.core.now()` is read in the booking section, so a very large waveform makes the receipt a little later. This is negligible at normal sizes. Folding it away would mean the shared function returning the decoded samples, which is not worth doing now.
- **N2 (#127):** `DEVICE_LEAD_NS` is still a 2 ms literal (`radio-uhd/src/profile.rs:11`), while the Mock derives its lead as `timing().min_timed_command_lead_ns − allowance` (`mock-radio/src/profile.rs:160`).
  - This deviates from the brief's "derived on both sides". The orchestrator should record it as a disposition.
  - The only way the two can disagree is a change to `x3xx::timing().min_timed_command_lead_ns`. The UHD lead would then stay at 2 ms while the Mock's moved.
  - That change fails in the default suite: `ur_09_the_declared_lead_is_the_device_lead_plus_the_delivery_allowance` (not feature-gated; it passed in my run) pins both the sum and the literals, and `the_test_side_literals_equal_the_profile` pins the sum too.
  - So no real disagreement path remains while those tests stand. Deriving the lead would need a `const` command lead in `x3xx`, because `timing()` is not `const`, and that is optional.
- **N3:** other copies of 3 ms and 2 ms exist only as test oracles: `radio-uhd/src/provider/rx.rs:3073,3353,3466,3610` (`core.ticks(3_000_000)`), `mock-radio/tests/mock_channel/o1.rs:49`, and `radio-uhd/tests/common/{iii.rs:1104,c7.rs:30}`. They are independent expectations, and iii's are pinned by `the_test_side_literals_equal_the_profile`. There are no other production copies: I grepped `crates/*/src` for `3_000_000` and `2_000_000`.
- **N4:** `design/18:63` still calls the delivery allowance one of "the Module's own timing values". That is still true of its meaning; only the code location moved to the Vocabulary crate. It can be left as is, or get a few words when m4's rows are written.
- **N5:** the new RM-13 bullet (`design/07:149`) contains "in every mode" (mode is MockRadio's MR-31 concept, undefined in spec 07) and a function path. "On receipt, by every Provider" would read cleaner. The MR-32 *Test:* line does not name `rm_13_waveform_bytes_are_refused_at_receipt_outside_channel_mode`, which is listed under RM-13 only. The test name `rm_13_a_waveform_whose_bytes_are_absent_are_refused` has a grammar slip ("are … are").
- **N6: mutation rows.**
  - M42, M43 and M44 are retargeted to the shared function with small, unique anchors, each killed by its own pure test.
  - N125-1 reproduces exactly the old behaviour: check only in channel mode.
  - N125-2 is killed only through NaN: with the check skipped, NaN goes out. With absent bytes the mutant would instead panic at `expect`, but through the Kernel that case is unreachable.
  - N51-2C-17 now edits `ezsdr-radio/src/x3xx.rs` while its crate field stays `ezsdr-radio-uhd`, which works because the test binary compiles the dependency. With that move, the mutant also shifts the Mock's lead. No Mock test is named, but the one shared constant makes that equivalent.
  - `rules.py --base ea7bc5c1`: 0 problems.
- **N7:** I found no stale texts in `python/`, `KNOWN_LIMITATIONS.md` or other tests. I grepped for "bytes are not an input", "do not have its size_bytes", "sample is not finite", "UR-21: the waveform's" and "MR-32:". The only hits outside the diff are frozen historical patches under `plan/phase3/`.

## What I ran

- `cargo test -p ezsdr-mock-radio -p ezsdr-acceptance -p ezsdr-server -p ezsdr-radio` (review target dir): 32 test binaries, all ok.
- `cargo test -p ezsdr-radio-uhd --lib profile` and `--test fake -- ur_21_txburst_refusals the_test_side_literals_equal_the_profile`: ok.
- `rules.py --base ea7bc5c1`: 0 problems.

Not checked:
- I did not run the mutation rows.
- That the new Mock test fails on the old code is argued from reading: the old Mock accepts outside channel mode, so the `rejected` section lacks the three rows. I did not execute it.
- I did not run the `--features uhd` build or clippy, the Python suite, or the full `cargo test --workspace`.

## Terra re-check

**FAIL — item 1 remains open.**

- **Major — different repeat limits defeat the promised first refusal.** Submit a one-channel repeat of one complex sample containing NaN, with valid metadata. The X3xx Mock rejects its odd length for alignment at lib.rs:1668 (`crates/ezsdr-mock-radio/src/lib.rs:1668`); UHD permits alignment 1 and rejects the NaN at control.rs:441 (`crates/ezsdr-radio-uhd/src/provider/control.rs:441`). Their declared repeat maxima also differ. Thus RM-13 (`design/07-radio-model.md:149`) and the Changes row’s unconditional promise of the same first refusal are false. The new competing-defect tests cover metadata versus bytes, but do not expose this case. The differing provider limits are explicitly documented, so resolving the mismatch requires reconciling the promise with those limits.

Items 2–7 are closed as specified. Computing length from declared `size_bytes` does not access a mismatched buffer; the size check precedes allocation and decoding. I found no booking or transmission before the reviewed refusals. The changed rules and Changes rows otherwise follow the requested format; `rules.py --base ea7bc5c1` reports zero problems.

This was a read-only review. I did not run builds, tests, or mutation rows.