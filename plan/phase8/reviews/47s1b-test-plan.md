# Test plan: note 47 stage 1b — channel names in the radio selector (draft 2, as built)

`test-plan-s1b.v1.md` is draft 1 (phase 1). This draft adopts Sol's review (`sol-s1b-plan-final.md`, dispositions in `dispositions-s1b-plan.md`) and the rulings (`rulings-s1b-plan.md`), and records what was built. VERIFIED = run in this session. Phase 2 deviations from draft 1 are listed in §7.

## 0. The shape, as built

- **One parser, name-free layout.** `crates/ezsdr-radio/src/channels.rs`: `split_entry(raw)` is the only place `#` is read; `element()` parses the part before it and echoes the whole `raw` in its refusals. `names_of(selector)` returns `ChannelNames { rx, tx }` (`Vec<Option<String>>`, index = logical channel number) and refuses a bad name or a name used twice in one radio (rx and tx one namespace, so a duplex front end listed in both carries two names). `from_selector` calls it, so Mock and UHD refuse at `from_binding` with no code of their own. `Layout`, `Element` and `Description` carry no name. One rule: `name_error(&str)`, shared by selector names, stream names (`initial.rs`) and, through `names.json`, Python. `canonical_number(&str)` is the one definition of "a number": ASCII digits only, no leading zero, a `u32`; any other legal spelling is a name (disposition 4).
- **Server.** `catalogue::channel_tables(&BindingProfile)` builds each radio's `DirectionNames` and refuses a name two radios give (EA-7); `assemble` calls it first (so `connect`, `run_child` and `ezsdr_server::assemble` in the acceptance rigs all get it) and `connect` calls it again for the reply table. `initial.rs`: `route` regroups streams under their owner radio before the old loop; `channels_of` and `member` resolve names through the owner's table. The reply has `channels` beside `streams`; `StreamNames` is renamed `DirectionNames` and serves both (schema regenerated).
- **Wire rule for name-only streams (ruled).** A stream's owner is the radio owning the names it mentions (in `channels` and in value keys); it is moved there whatever key it came under, a key naming no radio being fine; names of two radios, of the other direction, unknown names, and a number (channel or value key) beside names of a radio other than the key are refused; the stream's own number or name moves with it (stream numbers per owner, names unique per `connect`).
- **Identity (disposition 1).** The Kernel keeps the selector opaque and `binding_description` includes it, so a name is part of the binding; a child Run repeats the parent's selector names included. No stripping enters the Kernel. RM-28 and RS-25a say so.
- **Python.** `connect` takes `str` channels and value keys (name rule, duplicates by number and by name, the type); membership needing name resolution (a number beside names only, a name beside numbers only) is left to the server; `_Channels(list)` gives `rx_channels["loop-rx"]` with a `KeyError` listing the known names.

## 1. Every place that reads, compares or hashes a selector string

Searches (from the repository root, excluding `v3/`, `spike/`, `target`):
`git grep -n -E 'selector|binding_description|provider_content|selector_id|layout_elements|front_end_matches|subdev_spec' -- crates python plan/phase8/tools ':!*.json' ':!*.md'` (Sol's), `grep -rn "selector" crates python --include=*.rs --include=*.py`, `grep -rn "split(':')\|splitn(\|rsplit" crates --include=*.rs`, `grep -rn "split(':')\|split(\":\")\|\.split(\"#\|strip_prefix(\"0:\|starts_with(\"0:\|\"0:[AB]:" crates python plan/phase8/tools --include=*.rs --include=*.py --include=*.sh`, `grep -rn "contains(&element)\|\.contains(&" crates/ezsdr-radio-uhd/tests/common`, `grep -rln "ubx:\|basic_rx:\|:A:\|:B:" .`, `grep -rn "SELECTOR_KEYS\|from_selector\|ideal_from_selector\|selector_id" crates --include=*.rs`.

**Readers of a layout entry string (5; Sol's finding 2 added R5):**

| # | `path:line` | What it does with the string | Change |
|---|---|---|---|
| R1 | `crates/ezsdr-radio/src/channels.rs` `element()` | splits on `:`, four fields, echoes `raw` | reads `split_entry(raw).0`; echoes the whole `raw` |
| R2 | `crates/ezsdr-radio-uhd/tests/common/mod.rs:176` `channel_hz` | `element.split(':').nth(2)` is the board | unchanged: field 2 precedes `#name`, so a strip would be an equivalent mutant; pinned by `a_named_layout_element_has_its_boards_frequency_and_description` |
| R3 | `plan/phase8/tools/run-record.sh:121,124` | `v[0] in ("0:A:ubx:0", "0:A:obx:0")` and `startswith("0:A:")` decide what the **RF attenuator confirmation** names | strips `#…` before comparing; the prompt lists the boards without names; hand check in §6 |
| R4 | `crates/ezsdr-acceptance/src/layout.rs` helpers | build `rx`/`tx` from caller strings | none (strings pass through) |
| R5 | `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2115-2120` `raw_probes_applicable` | `["0:A:ubx:0","0:A:obx:0"].contains(&element)` decides whether the raw probes touch the device (a named UBX would be recorded `inapplicable_to_this_layout`) | compares `channels::split_entry(element).0`; row P948 retargeted |

**Production code that holds a selector but never reads an entry string** (verified, no change): `ezsdr-mock-radio/src/lib.rs:389-421`, `ezsdr-radio-uhd/src/provider/mod.rs:149,156,213-240` (`layout_elements`, `front_end_matches` read `Element`), `ezsdr-server/src/catalogue.rs` UHD `args`/sources, `ezsdr-sink-capture`, `ezsdr-sim-engine`, `ezsdr-radio/src/measurement.rs:246` (a record stores the map and hands it to `from_selector`), `mock-radio/src/lib.rs:487` `selector_id()` (the `id` key).

**Whole-selector hashes and comparisons** (names change them, as ruled): `ezsdr-kernel/src/plan/compile.rs:22` (the plan fragment), `plan/validation.rs:425-430` and `coordinator/pipeline.rs:1114-1126` (the binding description, which RS-25a compares: see sequence 8). The Spec hash input is unchanged (sequence 1).

**Existing tests, fixtures, rows that moved.** Stage 1's refusals "needs stage 1b" flipped: `protocol.rs` `ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run` (a name this profile does not give; `"00"` is no name; `"+0"` is a name no channel has) and the Python `test_ea_16_connect_refuses_a_bad_map_before_any_request` (a value keyed by a name no channel of the stream can be, a bad channel name, a name twice). The stream-name cases use the shared `names.json` through `name_error`. Rows retargeted: P948 (R5), P1334 and P1353-P1355 (the name rule and `canonical_number` moved to `channels.rs`; killers `rm_28_*`), P1335 (an unresolved name read as channel 0), P1338 (`if !channels.contains(&number)`), P1345 (the Python local-membership test), P1351 (made unique: `"sample_rate_hz")], names.get("tx", {}))`). `rules.py --base HEAD` passes.

## 2. Sequences and tests (all VERIFIED passing)

Fixtures: `named_profile` (protocol.rs): rx `loop-rx` c0, `side` c1; tx `loop-tx` c0, c1 unnamed; `two_radios`: a `second` Mock with `r2-rx`, `r2-tx`; Python `named_profile(rx, tx, couplings)`.

1. **Same behaviour with and without names.** `ezsdr-radio/tests/channels.rs` `rm_28_a_selector_name_does_not_change_the_layout` (`from_selector` and `Description::new` equal, `names_of` positions, four layouts incl. partly named and reordered); `ezsdr-mock-radio/tests/mock_radio.rs` `mr_02_a_named_selector_builds_the_unnamed_radio` (`instance()` and tree equal); `ezsdr-acceptance/tests/x3xx.rs` `d3_a_named_selector_gives_each_provider_its_unnamed_description` (named vs plain trees **within** each Provider, board keys and defaults across Providers: Sol's finding 6); `ezsdr-acceptance/tests/parity.rs` `vh_8_a_named_selector_prepares_as_the_unnamed_one_on_both_providers` (the concrete parity test: effective and declared SampleClocks, plain vs named on both rigs and across); `ezsdr-acceptance/tests/uhd.rs` `uhd_59_a_named_selector_calls_the_device_as_the_unnamed_one` (a reordered layout through a FakeDevice Session: identical call log, `rx_start`'s wall-clock argument normalised, no `#` in any call); `ezsdr-radio-uhd/tests/fake.rs` `ur_30_a_named_selector_records_and_sets_the_unnamed_subdev_spec` (the recorded `device.subdev_spec` equals the string actually passed to `set_subdev_spec`, the whole call log equal); `ezsdr-server/tests/protocol.rs` `ea_10_a_named_selector_connects_as_the_unnamed_one_and_the_record_holds_numbers` (Mock Session: effective, `spec`, `admission`, `action_log`, `events`, `clocks` equal).
2. **Name refusals, no Run.** `channels.rs` `rm_28_the_name_rule_cases` (the shared `names.json`: every bad class, good ones incl. full-width digits, `+0`, `-0`), `rm_28_a_name_is_used_once_in_a_radio` (same name twice in rx, in tx, once in rx and tx, a duplex front end with two names); `mock_radio.rs` `mr_02_from_binding_refusals` (named cases through `MockRadio::from_binding`); `fake.rs` `ur_05_a_bad_selector_name_is_refused_before_the_device_is_touched` (no `serial` or `set_subdev_spec` call); `protocol.rs` `ea_09_a_bad_selector_name_is_refused_before_any_run` (every bad name, `Refused`, `EA-7: radio:`, `assert_no_run`). The number rule: `rm_28_a_number_is_a_canonical_ascii_decimal`.
3. **Same name in two radios.** `protocol.rs` `ea_09_a_name_is_unique_in_the_whole_profile` (rx/rx, tx/tx, rx/tx clashes; with no `initial`, with numbers only; the distinct profile connects).
4. **Resolution.** `protocol.rs`: `ea_10_a_channel_name_is_resolved_to_its_number` (a number beside a name, value keys by name and by number, `loop-rx` held as the number 0, `side` by name), `ea_10_a_key_is_a_number_only_when_it_is_a_canonical_ascii_decimal` (`+0`, `-0`, full-width one, `00a` as names of c1; `00`, `01`, overflow `4294967296` and a 20-digit string refused as no name: Sol's finding 4), `ea_10_a_name_that_cannot_be_resolved_is_refused` (unknown; tx name in rx and rx in tx; name twice; name and its own number; name in two streams; two value keys `"1"`/`"side"` for one channel, for gain and for antenna; a value for a name not in the stream; a value keyed by a name of the other direction), `ea_10_a_stream_of_names_goes_to_the_radio_that_owns_them` (under a radio that exists and under `nowhere`; stream numbers per owner; two stream-0s of one owner refused; numbers under an unknown key still refused), `ea_10_a_stream_of_two_radios_is_refused` (names of two radios; a number or number-valued key of `radio` beside a name of `second`, either spelling; a number written under the other radio; one owner succeeds). Python (`ConnectRequests`, mocked): `test_ea_16_connect_takes_channel_names`, `test_ea_16_connect_leaves_a_mixed_reference_to_the_server` (`Stream([0], gain={"loop-rx": 5})`, `Stream(["loop-rx"], gain={0: 5})`, both: Sol's finding 3), `test_ea_16_the_name_rule_matches_the_server` (the shared `names.json`, channel and stream names), the extended `test_ea_16_connect_refuses_a_bad_map_before_any_request` (bad channel name, digit-only, name twice, name in two streams, value key no channel can be, bad value-key name, bool key); `EasyApi.test_ea_16_a_channel_is_reached_by_its_name_through_a_server` (an unknown name and a tx name in `rx=` come back as `Error` with the server's text).
5. **The name table.** `protocol.rs` `ea_10_connected_carries_the_channel_name_table` (named profile with no `initial`, only named entries listed by position; a late name keeps its position; an entry per radio, empty for an unnamed one; `ideal` empty; the sink and Authority are no radios), `ea_06_schema_freeze` and `ea_06_connect_initial_is_in_the_schema` (regenerated `reply_frame.v1.json`); Python `ConnectRequests.test_ea_16_a_channel_is_reached_by_its_name` (names to numbers by direction, numbers/length/iteration unchanged, `KeyError` listing the known names, a name of the other direction a `KeyError`, an unnamed radio) and the real-server `EasyApi` test above.
6. **Portability.** Python `EasyApi.test_ea_16_one_program_of_names_runs_on_two_layouts`: one program of names on a layout with `loop-rx`/`loop-tx` at c0 and one with them at c1 (couplings `(0,0)` and `(1,1)`); each asserts the numbers and the stream's channels (so a run on c0 twice cannot pass) and the loopback ramp. Virtual time only (`sdr.sleep(0.003)`).
7. **Records.** `protocol.rs` sequence 1's test: the named Manifest's `spec`, `admission`, `action_log`, `events`, `clocks` hold no name or `#` and equal the plain ones; the `binding` section holds `0:A:ubx:0#loop-rx` and differs from the plain one; the log's key is `radio.rx.c1.gain_db`.
8. **Child Run (disposition 1).** `protocol.rs` `ea_14_a_child_run_repeats_its_parent_s_selector_names_included`: a named Session; the derived child profile and an explicit copy of the parent's named profile are admitted; the same selector with the names removed, and with one name changed, are `Rejected` with `RS-25a: radio binds an instance its parent does not`; a child profile with a name used by two radios is refused at assembly. RM-28 and RS-25a say it.
9. **Measurement tool readers.** `fake.rs` `raw_probes_are_inapplicable_when_channel_zero_is_not_slot_a_on_a_ubx_or_obx` extended: named UBX/OBX slot A (and a named CBX at position 1) apply, named CBX/slot-B/mixed channel 0 are refused as the plain ones are, row P948; `a_named_layout_element_has_its_boards_frequency_and_description` (R2).

Time: no test adds a wall-clock read, thread or timeout. The Server tests call `Server::handle` synchronously; Python uses the Mock's virtual `sdr.sleep`; the two FakeDevice Sessions (`uhd_59_a_named_selector_calls_the_device_as_the_unnamed_one`, `ur_30_*`) compare call logs after normalising the one wall-clock-derived call argument (`rx_start`) and compare clocks by stream and rate only, so no verdict depends on timing (`plan/phase8/37-test-thread-timing.md`).

## 3. Mutation rows (appended P1361-P1386; retargeted P948, P1334, P1335, P1338, P1345, P1351, P1353-P1355)

Probes: `plan/…/probe.py`-style driver outside the repository (a copy under `/tmp/s1b-probe-tree`, shared review target dir, `CARGO_INCREMENTAL=0`, compile error = NOT killed); logs `/tmp/s1b-probe-<id>.log`, summary `/tmp/s1b-probe-summary.log`. Results in §5.

| Id | Mutation | Named killer |
|---|---|---|
| P948 | the raw-probes slot-A test compares the whole entry stripped (`split_entry(element).0`) → `if false` | `raw_probes_are_inapplicable_when_channel_zero_is_not_slot_a_on_a_ubx_or_obx` |
| P1334 | `name_error`: digits-only name accepted (was the server's inline copy) | `rm_28_the_name_rule_cases` |
| P1335 | a channel name resolves to channel 0 | `ea_10_a_channel_name_is_resolved_to_its_number` |
| P1338 | a value for a channel the stream lacks accepted | `ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run` |
| P1345 | Python: local membership refusal off | `ConnectRequests.test_ea_16_connect_refuses_a_bad_map_before_any_request` |
| P1351 | Python: tx streams read the rx stream names | `ConnectRequests.test_ea_16_a_name_is_looked_up_in_its_own_direction` |
| P1353 | `canonical_number`: leading zeros allowed | `rm_28_a_number_is_a_canonical_ascii_decimal` |
| P1354 | `name_error`: control characters allowed | `rm_28_the_name_rule_cases` |
| P1355 | `name_error`: non-ASCII digits count as digits | `rm_28_the_name_rule_cases` |
| P1361 | `split_entry` hands `element` the whole entry | `rm_28_a_selector_name_does_not_change_the_layout` |
| P1362 | names of rx and tx kept in separate sets | `rm_28_a_name_is_used_once_in_a_radio` |
| P1363 | a name repeated within a direction accepted | same |
| P1364 | an unnamed entry is left out of the name list | `rm_28_a_selector_name_does_not_change_the_layout` |
| P1365 | rx names read as tx | same |
| P1366 | `from_selector` ignores `names_of` | `rm_28_the_name_rule_cases` |
| P1367 | `canonical_number` accepts a sign | `rm_28_a_number_is_a_canonical_ascii_decimal` |
| P1368 | `#` allowed in a name | `rm_28_the_name_rule_cases` |
| P1369 | a name used by two radios accepted | `ea_09_a_name_is_unique_in_the_whole_profile` |
| P1370 | only rx names enter the table and the uniqueness set | `ea_10_connected_carries_the_channel_name_table` |
| P1371 | `assemble` does not check names (a child profile) | `ea_14_a_child_run_repeats_its_parent_s_selector_names_included` |
| P1372 | `connect` expands against no names | `ea_10_a_channel_name_is_resolved_to_its_number` |
| P1373 | `connected.channels` empty | `ea_10_connected_carries_the_channel_name_table` |
| P1374 | a name of the other direction accepted | `ea_10_a_name_that_cannot_be_resolved_is_refused` |
| P1375 | a stream may mix radios' names | `ea_10_a_stream_of_two_radios_is_refused` |
| P1376 | a number beside another radio's names accepted | same |
| P1377 | a stream of names not moved to its owner | `ea_10_a_stream_of_names_goes_to_the_radio_that_owns_them` |
| P1378 | an empty `initial` for a resource is dropped | `ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run` |
| P1379 | names in value keys do not decide the radio | `ea_10_a_stream_of_two_radios_is_refused` |
| P1380 | two value keys for one channel accepted | `ea_10_a_name_that_cannot_be_resolved_is_refused` |
| P1381 | a value key is a number when it parses as one | `ea_10_a_key_is_a_number_only_when_it_is_a_canonical_ascii_decimal` |
| P1382 | Python: tx channels read the rx name table | `ConnectRequests.test_ea_16_a_channel_is_reached_by_its_name` |
| P1383 | Python: an unknown name's error loses the known names | same |
| P1384 | Python: a channel name skips the rule | `ConnectRequests.test_ea_16_the_name_rule_matches_the_server` |
| P1385 | Python: a number beside names only refused locally | `ConnectRequests.test_ea_16_connect_leaves_a_mixed_reference_to_the_server` |
| P1386 | Python: a stream name skips the rule | `ConnectRequests.test_ea_16_the_name_rule_matches_the_server` |

Dropped as Sol said (finding 5): the `split_once` -> `rsplit_once` row (equivalent: the entry is refused either way); reused P1334/P1335 instead of duplicates. No Module-side row exists: neither Provider receives a name, and their equivalence is pinned by sequence 1. No row for `run-record.sh` (no test reaches it): the hand check below.

## 4. Spec amendments

RM-28 (grammar, rule, one namespace per radio, no effect on the description, a name is part of the binding; `*Test:*` lines; `ideal` names nothing), RS-25a (a name is part of the binding; test `ea_14_a_child_run_repeats_…`), MR-2's `rx`/`tx` row, UR-5 and the measurement selector paragraph, EA-3's table row, EA-10 (the Assembly refuses a name two radios give), EA-10a (numbers vs names by canonical spelling, routing, refusals, `connected.channels`, test list), EA-16 (channel names, name table, `KeyError`), each with a Changes row. No version bump, no SCHEMA_CHANGELOG; `schemas/server/reply_frame.v1.json` regenerated. `rules.py --base HEAD` passes (EA-10a's long line split so no line over 1 500 characters is added).

## 5. Probe results

(the table follows §7)

## 6. Hand check of `plan/phase8/tools/run-record.sh` (the attenuator prompt)

Run with `echo no | bash plan/phase8/tools/run-record.sh HEAD addr=192.0.2.1` and `EZSDR_MEASUREMENT_SELECTOR` unset or set (log `/tmp/s1b-runrecord-handcheck.log`; the script stops at the prompt, before Docker or the device, because `no` is not `yes`): no selector, plain UBX slot A and named UBX slot A all print `confirm that EVERY transmitting channel (slot A board (raw probes and preflight tests))`; plain CBX B prints `(0:B:cbx:0)` and the named one prints the same `(0:B:cbx:0)`; a named `tx` of CBX B and OBX A prints `(0:B:cbx:0, 0:A:obx:0)`; a named layout with tx channel 0 on B names `0:B:cbx:0`. Each run ended `run-record: attenuator not confirmed; stopped before Docker or device access`, exit 1. The prompt still appears, with the same boards named, with and without names.

## 7. Deviations from draft 1

- The name-rule fixture is `crates/ezsdr-radio/tests/fixtures/names.json`, read by the Rust radio and server tests (`include_str!`) and the Python test (relative path); no `NAME_CASES` constant.
- Sequence 1's Manifest and record checks are in the server test, not `v58.rs` (the server connect already builds both runs).
- `channel_hz` (R2) is unchanged; a strip would be an equivalent mutant.
- A small `Case` type alias in two tests satisfies clippy's `type_complexity`.
- Name-only streams: "a number whose radio differs" also covers a number-valued value key.
- `member`'s and `channels_of`'s name lookups index the owner's table because `route` has already resolved every name; a missing name there would be a bug (a panic), not a refusal, so they have no refusal text.

## Probe results (out of tree; compile error would count as NOT killed)

| Row | Verdict | Killer |
|---|---|---|
| P948 | killed | `raw_probes_are_inapplicable` |
| P1334 | killed | `rm_28_the_name_rule_cases` |
| P1335 | killed | `ea_10_a_channel_name_is_resolved_to_its_number` |
| P1338 | killed | `ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run` |
| P1345 | killed | `test_easy_api.ConnectRequests.test_ea_16_connect_refuses_a_bad_map_before_any_request` |
| P1351 | killed | `test_easy_api.ConnectRequests.test_ea_16_a_name_is_looked_up_in_its_own_direction` |
| P1353 | killed | `rm_28_a_number_is_a_canonical_ascii_decimal` |
| P1354 | killed | `rm_28_the_name_rule_cases` |
| P1355 | killed | `rm_28_the_name_rule_cases` |
| P1361 | killed | `rm_28_a_selector_name_does_not_change_the_layout` |
| P1362 | killed | `rm_28_a_name_is_used_once_in_a_radio` |
| P1363 | killed | `rm_28_a_name_is_used_once_in_a_radio` |
| P1364 | killed | `rm_28_a_selector_name_does_not_change_the_layout` |
| P1365 | killed | `rm_28_a_selector_name_does_not_change_the_layout` |
| P1366 | killed | `rm_28_the_name_rule_cases` |
| P1367 | killed | `rm_28_a_number_is_a_canonical_ascii_decimal` |
| P1368 | killed | `rm_28_the_name_rule_cases` |
| P1369 | killed | `ea_09_a_name_is_unique_in_the_whole_profile` |
| P1370 | killed | `ea_10_connected_carries_the_channel_name_table` |
| P1371 | killed | `ea_14_a_child_run_repeats_its_parent_s_selector_names_included` |
| P1372 | killed | `ea_10_a_channel_name_is_resolved_to_its_number` |
| P1373 | killed | `ea_10_connected_carries_the_channel_name_table` |
| P1374 | killed | `ea_10_a_name_that_cannot_be_resolved_is_refused` |
| P1375 | killed | `ea_10_a_stream_of_two_radios_is_refused` |
| P1376 | killed | `ea_10_a_stream_of_two_radios_is_refused` |
| P1377 | killed | `ea_10_a_stream_of_names_goes_to_the_radio_that_owns_them` |
| P1378 | killed | `ea_10_a_request_the_layout_cannot_meet_is_refused_before_any_run` |
| P1379 | killed | `ea_10_a_stream_of_two_radios_is_refused` |
| P1380 | killed | `ea_10_a_name_that_cannot_be_resolved_is_refused` |
| P1381 | killed | `ea_10_a_key_is_a_number_only_when_it_is_a_canonical_ascii_decimal` |
| P1382 | killed | `test_easy_api.ConnectRequests.test_ea_16_a_channel_is_reached_by_its_name` |
| P1383 | killed | `test_easy_api.ConnectRequests.test_ea_16_a_channel_is_reached_by_its_name` |
| P1384 | killed | `test_easy_api.ConnectRequests.test_ea_16_the_name_rule_matches_the_server` |
| P1385 | killed | `test_easy_api.ConnectRequests.test_ea_16_connect_leaves_a_mixed_reference_to_the_server` |
| P1386 | killed | `test_easy_api.ConnectRequests.test_ea_16_the_name_rule_matches_the_server` |

P1335 first SURVIVED: the test then held names only for channels that were c0 (a name read as channel 0 gave the same result). The test now holds `side` (c1) by name beside the number 0, and the re-probe kills it. P1338 and P1371 were re-probed against the final test files and are killed. All 35 rows: killed, none compile-failed, none not found.
