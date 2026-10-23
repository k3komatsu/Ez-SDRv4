# 1b-7b：実装役（GPT-6 Luna）の報告

> orchestrator の注（2026-10-23）：「既知のリスク」の 2 重計上は gate の前に直した（[1b7b-opus](1b7b-opus.md) §3 の O1）．

Phase 8 1b-7b（D2）を実装した。`RxOverflowPayload` は `{ cause, lost }`、hot path は9 byte。`ezsdr_radio::payloads::rx_overflow_payload` が一つのblock jumpにつき最大1 eventを決め、MockRadioとUHD RXが呼ぶ。Mockのback-pressure eventは従来どおり即時emitし、共通関数を通さない。

## 計画の修正・実装

- Solの3指摘を反映：T17bのmissed-start-only系列、D33の追加、`mr_21_overrun_shape` が同じrestart blockに集める1 ms/10 ms faultsであること（eventはtime 1,000・cause Overrun・lost 59,000）。
- T17aのmissed-start+overrun系列をUHD unit testに追加。block-wide `lost`（missed-startとoverrunの両方）、最初の損失時刻、flags、`LATE_COMMAND` 1件と `RX_OVERFLOW` 1件をassert。
- `RxOverflowPayload::rx_overflow_payload`：`crates/ezsdr-radio/src/lib.rs`、呼出し元：`crates/ezsdr-mock-radio/src/lib.rs::publish_rx` と `crates/ezsdr-radio-uhd/src/provider/rx.rs::Rx::samples`。UHDは`MissedStart`をreport causeに変換しない。gapless blockでpendingを消費し、`ALIGNMENT_ERROR`とflagsは維持。
- Mock fault reportsは `(first lost sample, cause)` を保持し、block jump全体の`lost`を使う。prior back-pressure lossを含む場合もevent timeはそのjumpの最初の損失sample。
- 新/変更テスト：`rm_17_reported_losses_make_one_payload_for_the_block_jump`、`vh_8_a_reported_overrun_matches_the_block_jump`、`vh_8_a_reported_sequence_error_matches_the_block_jump`、`vh_8_reports_before_one_block_make_one_overrun_summary`、`vh_8_repeated_overruns_before_one_block_make_one_summary`、`vh_8_a_report_without_a_following_block_emits_no_overflow`、`ur_17_a_missed_start_and_overrun_share_the_block_summary`、`ur_17_a_missed_start_without_another_report_is_not_an_overflow`、`ur_18_a_report_on_a_gapless_block_is_consumed_without_an_event`、`mr_19_a_fault_overflow_and_repeated_backpressure_are_counted_once`。`mr_21_overrun_shape` と既存payload testsも更新。
- `test-plan.md` にT21を追加して、back-pressure後のfaultを含むblock-wide summaryの時刻とlostを固定。T20はfault reportと繰返しFullの別eventを固定。

変更は15 files（`git diff --stat`: 562 insertions / 142 deletions）。主要な差分：`parity.rs` +164/−7、Mock production +22/−19、Mock tests +73/−6、UHD RX +97/−42、Radio Model +17/−9、3 accepted specs +38/−28。2つのradio schemaを再生成。version・`schemas/SCHEMA_CHANGELOG.md` は変更していない。

## 検証

| 確認 | 結果 |
|---|---|
| `rm_20_schema_freeze` / schema regeneration | pass |
| `rules.py --base HEAD` | pass（0 problems） |
| `git diff --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| UHD Docker clippy (`--features uhd`) | pass |
| UHD Docker `--features uhd --lib --test uhd_api` | pass: 274 + 5 tests |
| determinism repeat | 20 consecutive rounds; 240/240 test commands passed (`--test-threads=8`) across parity, Radio Model, Mock, UHD RX and acceptance cases |
| targeted remote mutations | 24/24 killed; final affected-row round 7/7 killed. Full mutation listsはorchestratorのgateに残した |
| Python suite | 55/56 pass。`test_v54_sleep_is_run_time` は11.10 sで10 s上限を超過し、単独再実行も11.18 sで失敗 |
| `cargo test -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-radio-uhd -p ezsdr-acceptance` | RX関連・Mock・parityなどはpass。`ezsdr-radio-uhd/tests/fake.rs` のTX timing casesはsuite loadでflaky |
| `cargo test --workspace --no-fail-fast` | 1 target failed: 同じ `fake.rs` のTX timing tests（下記） |

`fake.rs` の失敗は変更対象外のTX timing casesで、suiteと単独の結果が揺れた。workspace gateでは `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst`、`ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`、`ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it` が失敗し、個別では前2つがfail、最後がpass。別suite runでは `ur_23_a_burst_one_sample_after_a_burst_is_played` がfailし単独でpass。`ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned` もpackage gate後の個別実行で再現failした（`StalledSend`/`FailSend` のcase未到達）。短い壁時計窓を持つ既存fake TX testsの不安定さであり、今回のRX差分との因果は確認していない。

## セルフレビューと扱い

- Review 1（DeepSeek）：T17a欠落はテストを追加して修正。P1-2はdecision 5のblock-wide `lost`とback-pressure例外が優先するため、このbrief内では残差payloadへ変更しないと判断してreject。時刻はfirst lost sampleになるよう修正し、T21で系列を固定した（リスク節参照）。
- Review 2（DeepSeek）：P1なし。gapless testのTest行、jump blockに限定したflags文、MR-19のdirect `lost`、MR-22のoverrun-priority test、UR-17 Changes行、MR-37の9-byte assertion、`mr_20a` flags、§6のmissed-start+overrun行、mutation JSONのインデントノイズを修正。
- 残るP3相当の穴：UHD RXは`len > 0` blockの中でpendingを消費する。in-tree `UhdDevice` は`received == 0`をTimeoutにし（`crates/ezsdr-radio-uhd/src/uhd.rs:964-983`）、FakeDeviceも`got == 0`ではSamplesを返さない（`crates/ezsdr-radio-uhd/src/device.rs:1433-1453`）。独自 `Device` がempty `Samples`を返した場合のpending消費は試していない。

## 既知のリスク

Back-pressure eventの直後、別faultが次のaccepted block前に届く場合、RM-17のwhole-block `lost` summaryには先にback-pressure eventで報告済みのlossも含まれる。T21は受理済みcontractを固定する一方、event payloadの合計は実損失を重複計上する。T12はback-pressureなしの場合だけ保証する。これを避けるにはdecision 5のamendmentが必要であり、この実装では行っていない。

commit・push・`git add`・`cargo fmt`・`rustfmt`・local mutationは実行していない。
