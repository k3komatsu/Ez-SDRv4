# D13 — Terra reviews


## terra

**Verdict: FAIL.** The placement tool can admit records that D13 intends to exclude, and it can change measurement values while writing them.

**Major findings**

1. **Deleted groups can pass admission.** `place_measurement.py:60` collects individual words from every writer string literal; `line 116` accepts a condition if its words occur anywhere in that collection. For example, `raw-device-coercion` and `rx-10msps` are current condition names, so a deleted condition named `raw-rx-coercion` would pass even if no writer produced it. The self-test’s `queue-rx-gain` case only catches a missing word, not this failure. Compare complete group identities or their explicitly defined generated forms, and test a deleted group made entirely of surviving words.

2. **A transient-failure record is admitted and its evidence discarded.** A `load: none` run can have a transient throughput failure: the writer stops its capacity ladder at the first failed trial and records the blocker in an observation (`multichannel.rs:588`). The placement check examines neither the failure nor the observations, then clears the observations at `place_measurement.py:151`. The lower measured value consequently binds both profiles through `measurements.rs:34`, while the placed file no longer explains the failure. D13 calls for the latest **clean** record (`stage4-proposal.md:68`). Require an explicit review of the raw evidence before placement and make that requirement clear in the tool’s workflow.

3. **JSON conversion can silently change a measured number.** `place_measurement.py:133` parses all numbers as Python values and `line 160` serializes them again. A JSON value such as `9007199254740993.0` becomes `9007199254740992.0`; only `args` and `observations` were authorized to change. Preserve measurement-number values through conversion and add a self-test that compares the placed values with the input. The current self-test checks the selector and observations but never checks `values` (`line 209`).

The November 13 R1 failure is **not explained solely by stage 4a making the record stale**. Its TX result is 400 MB/s because 200 MS/s underflowed (`bench record`); the later bench also observed TX failures at 200 MS/s on both boards (`November 16 results`). The current profile nevertheless declares 800 MB/s (`x3xx.rs:420`). A fresh record could therefore fail TX comparison too; its actual result must decide that.

For the sample-rate grids, the writer sweeps the description’s rate grid and retains requested points only when hardware read-back matches (`measurement.rs:1073`, `line 1274`); comparison requires exact membership (`measurement.rs:563`). The old record’s grids contain the expected `200 MHz / N` values when read as Python floats, so I cannot attribute the reported failure to stage 4a or establish its precise cause from this read-only review. The same exact-membership path will be used on a fresh record, so it needs investigation before assuming November 27 will pass.

The key check derives its set from `lib.rs` constants and leaves plus `rule()` exclusions, and checks missing, extra, and duplicate keys. The ordinary address lookup matches the record’s serial; `--args` deliberately overrides it. Duplicate detection compares serial and the selector’s listed layout fields. I did not run the self-test, build, or format, as requested.

## terra-recheck

What this change does: It tightens measurement-record placement, moves the sustained TX throughput result out of compared values, and enables accurate float parsing for workspace JSON. The comparison rule and float fix are consistent with the owner’s decision, but placement still admits records without enough evidence and can accept condition IDs the current writer cannot produce.

**Independent failure scenarios**

- A retired or impossible condition retains a familiar generated shape and passes admission.
- A failed run is copied after its observations are removed; placement treats the missing evidence as an empty list.
- A clean run has ordinary observations, including the newly moved TX result, and is unnecessarily treated as a failed run.
- A number or another field changes while the placement tool rewrites JSON.
- The TX result disappears from the bench record when observations are stripped, or an old `values` row remains accepted.
- A 17-digit sample rate changes by one ULP while parsing JSON, changing grid membership or canonical hashes; JSON parsing on the real-time path would also make the feature relevant to timing.

**Major findings**

1. **A stripped bench copy bypasses the observations refusal.** `place_measurement.py:185` refuses a nonempty list but accepts a missing `observations` field. `bench_extract.py:93` removes that field, and the placement tool explicitly accepts such copies. Minimal sequence: a throughput ladder stops on a transient failure; extract the bench copy; place that copy without an override. The tool writes the record and only afterward prints a warning, so the lower value can bind the profiles without the required review or reason. Refuse missing observations and require the raw record.

2. **The TX measurement is lost from the retained bench copy.** The writer now puts the TX row solely in an observation (`measurement.rs:163`). Both bench extraction and placement remove observations (`bench_extract.py:93`, `place_measurement.py:199`). Minimal sequence: run a successful measurement and retain the usual extracted bench record and placed record. Neither contains the TX result, contrary to `KL-5` and `design/18`, which say it remains in the bench record. Preserve the observation in a retained bench artifact, while keeping it out of the placed `values`.

3. **Condition admission still accepts IDs the writer cannot produce.** `place_measurement.py:87` checks broad patterns, not the writer’s generated identities. For example, it accepts `tx-1000000hz`, but `tx_rate_id` emits `tx-1msps` for that rate. Likewise, it accepts arbitrary rate and mixed-family combinations. A retired generated condition matching a surviving pattern can therefore enter a placed record. Derive the actual IDs from the applicable rate lists and generation rules, including layout and reopen restrictions.

4. **The clean-run workflow always needs an override.** A normal TX measurement produces observations, and the new writer adds the `not_compared_…` observation even when the measured result is successful (`measurement.rs:166`). The placement tool treats *every* observation as evidence of a stopped ladder or failure (`place_measurement.py:185`). Thus a clean raw record cannot follow the documented “take a clean run” path; its operator must override the refusal. Distinguish ordinary evidence from failure evidence, or explicitly revise the admission workflow so the required review is meaningful.

**Additional problem**

5. **The lossless writer can replace a real string with a number.** `dump_raw` substitutes unescaped placeholder strings globally. For `{"note":"@@RAW0@@","value":1.5}`, it emits `{"note":1.5,"value":1.5}`. I reproduced this with a read-only Python invocation. Use a serializer that preserves numeric tokens without replacing occurrences in unrelated strings.

**Original findings**

1. **PARTLY closed.** `raw-rx-coercion` is now refused and tested, but generated-form admission remains too broad (finding 3).
2. **PARTLY closed.** A raw record with observations is refused and an override prints its reason, but a stripped bench copy bypasses the check (finding 1). The refusal prints observation names rather than their contents; the operator must inspect the raw record to review the evidence.
3. **PARTLY closed.** Float tokens such as `9007199254740993.0` are preserved and tested (`place_measurement.py:277`), but placeholder collisions can alter another field (finding 5). A bench copy also has already passed through Python float parsing in `bench_extract.py:90`.

The `NotCompared` implementation matches RM-35: the key remains in the profile, is excluded from comparable `values`, and the placement key set excludes it (`measurement.rs:304`, `place_measurement.py:75`). Historical bench records still contain the old TX `values` row; the placement check deliberately refuses them. The remaining document mismatch is retention of the new observation, described in finding 2.

Every workspace crate that uses `serde_json` depends on the kernel, so its feature declaration reaches those builds. The new Rust grid round-trip and Python Session tests cover the reported rate (`measurement test`, `Python test`). The supplied workspace log reports fixture and schema-freeze tests passing; this re-check did not rerun them. Source inspection found text parsing at server transport input (`transport.rs:100`), and no JSON text parser in the UHD RX or TX paths. I did not benchmark timing or independently rerun the Python suite, build, or format.

**Verdict: FAIL.**

## terra-recheck2

**Verdict: FAIL.** The observation handling and TX-result retention are improved, but the placement tool still accepts derived bench copies and can admit records produced by changed record-suite code.

**Major findings**

1. **The raw-record check also accepts new bench copies.** `bench_extract.py:90` parses and reserializes the record while retaining `observations`. The check at `place_measurement.py:153` therefore accepts that copy as raw. Python float parsing can already have changed a measured number before the lossless placement writer sees it. Require an identifiable raw input, or make extraction preserve the original JSON bytes and revise the raw-only contract accordingly.

2. **The commit comparison omits code that decides what the record contains.** `DEFINES` at `place_measurement.py:30` omits `hardware.rs:2449`, which selects the raw probes and invokes the measurement groups. A commit can change that selection while all listed paths remain unchanged, so a record from before the change passes the comparison. Include the record-suite entry point and examine its record-shaping helpers.

3. **A refused replacement can delete the existing placed record.** With `--replace`, `place_measurement.py:178` removes the existing file before `serializing the replacement`. If the incoming record contains `@@RAW`, serialization raises `Refused` after deletion, leaving the old record gone and potentially an empty target. Serialize and validate before removing or replacing anything.

**Prior findings**

- **Major 1 — PARTLY closed.** Old copies without `observations` are refused, and observations in accepted records are printed and reviewed. New extracted copies pass, as described in finding 1.
- **Major 2 — CLOSED.** The extracted bench copy retains all observations, including the TX result, at `bench_extract.py:90`.
- **Major 3 — PARTLY closed.** The broad condition patterns are removed and the record commit must be an ancestor of HEAD, but the comparison misses record-shaping code, as described in finding 2.
- **Major 4 — CLOSED.** `place_measurement.py:156` prints each observation and requires a note only for names outside `not_compared_*`.
- **Problem 5 — PARTLY closed.** The reported placeholder collision is refused at `place_measurement.py:59`, but that refusal can destroy an existing replacement target, as described in finding 3.

For uncommitted changes at bench time, `run-record.sh:177` builds an archive of the selected commit and records that SHA, so dirty source files in the invoking tree do not enter that run. Placement does, however, read current working-tree key definitions while comparing committed snapshots; its result can therefore reflect uncommitted definition edits.

This was a read-only source review. I did not build, format, or run the self-test.