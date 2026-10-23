# 1b-7b テスト計画（D2）

> orchestrator の注（2026-10-23）：T21 の期待（後の fault の event が block の飛び全体 151 000 を時刻 1 000 で持つ）は誤りで，gate の前に直した．event は `(151 000, overrun, 1 000)`：back-pressure が報告した損失を 2 度数えない（メモ 36 の T20．[1b7b-opus](1b7b-opus.md) §3 の O1）．


## 範囲と共通の判定

対象は設計メモ36の決定4・5だけ。`RX_OVERFLOW` payload は `{ cause, lost }`、hot path は9 byte（cause 1 byte + little-endian `lost` u64）。報告が複数あっても、飛びのある次の block に対し event は最大1個、`lost` は block 全体、cause は overrun 優先。event時刻は最初に失った sample。飛びのない block は pending report を消費して event を出さず、report 後に block が無ければ event は出さない。Mock の back-pressure 報告は例外のまま。

共通関数名案：`ezsdr_radio::payloads::rx_overflow_payload(reports: &[RxOverflowCause], lost: Option<u64>) -> Option<RxOverflowPayload>`。入力なし、または `lost == None` なら `None`、それ以外は block 全体の `lost` を使い、Overrun が1つでもあれば `Overrun`、そうでなければ `Sequence`。trait・Kernel型・report型の共有は追加しない。

この関数の unit test（`crates/ezsdr-radio/tests/radio_model.rs`）は、空 report + `Some(lost)` と reportあり + `None` が `None`、Sequenceのみが Sequence、Sequence + Overrun と同じ種類の重複がそれぞれ1 payload（代表cause・全体lost）になることを直接 assert する。これで空入力や `None` が空の `all(...)` で通ることはない。

## 系列

| 系列 | テストと操作 | 観測・assert | 見抜く誤実装 |
|---|---|---|---|
| T6 | `parity.rs` に Mock `rx_overflow` fault と UHD FakeDevice `Overflow` をT0の最初のreceive sampleで起こす比較を追加（Mock faultはT0、FakeFaultは `Duration::ZERO`）。 | 両方でeventが厳密に1個。cause=`Overrun`、payload `lost` = block headerの`lost`、flagsは `GAP_BEFORE | RESTARTED`。event時刻は `block.first_sample - block.lost`（各Provider自身のSampleClock上）で、9-byte hot payloadも確認。 | fault/report時点での即時emit、payloadが原因ごとの一部lost、時刻が再開blockの先頭、hot payloadの誤長さ。 |
| T7 | 同じ parity rig で Mock `rx_sequence_error` と UHD `SequenceError` をT0に置く。 | eventは1個、cause=`Sequence`、lostと時刻はblockの飛び全体／最初の損失sample。flagsは `GAP_BEFORE | SEQ_DISCONTINUITY` で `RESTARTED` なし。 | causeの取り違え、Sequence eventの欠落、lostをreport単位にする、flagの取り違え。 |
| T8 | parityで OverrunをT0、Sequence reportをT0+50msに置く。FakeFaultは両方 `Duration::ZERO`。最初のFake overrunでrestart blockが50ms先に動き、2つ目のreportはそのblockより前に届く。 | eventは厳密に1個、cause=`Overrun`、lostはblock全体。flagsに `RESTARTED` と `SEQ_DISCONTINUITY` の両方。時刻は最初の飛び。Mockのfault rowsは原因ごとの行と各行の `lost` を維持。 | reportごとの複数event、最後のcauseを採用、原因別lostを出す、片方のflagを落とす、Mock fault記録まで合算する。 |
| T9 | parityで同種のOverrunをT0とT0+50msに発生させる。FakeFaultは両方 `Duration::ZERO` で、2つ目は最初のrestart gap後の次のreceive requestで報告される。 | そのblockに対して両方ともeventは厳密に1個、cause=`Overrun`、lostはheaderの全体lost、timeは最初の損失sample。Mock fault rowsは2行のまま各faultの損失をassert。 | 同種reportを2 eventにする、最初/最後のreport分だけlostにする、Mockのfault行をまとめる。 |
| T10 | 既存 `ur_19_reports_before_one_block_keep_each_kind` の reportなしでtimestampが飛ぶケースを維持。 | blockの `lost` と `GAP_BEFORE | SEQ_DISCONTINUITY`、対象event集合が空であることをassert。 | reportのない飛びから `RX_OVERFLOW` を作る。Mock fault では同じ入力を作れないためUHDのみ。 |
| T11 | `rm_24_the_hot_form_round_trips` と `rm_24_a_delivered_payload_reads_in_either_form` を更新。 | 9 byteの実値（cause byte、`lost.to_le_bytes()`）を固定し、両cause/境界値をround-trip。8・10・旧17 byte、未知cause byteを拒否。objectとdrained arrayの両読込、`restart_gap_ns` を含む旧objectの拒否、範囲外byteの拒否をassert。`rm_20_schema_freeze` で両schemaを検査。 | cause/endian誤り、末尾切捨て等で17 byteを受理、旧fieldを黙って受理、array/objectのどちらかを誤読。 |
| T12 | T6〜T9の parity 比較で、back-pressureなし・event ring dropなしの条件を維持。 | report-bearing block/eventが少なくとも1つあること、event数を先にassertしたうえで、event payload `lost` の合計 = 対象 block header `lost` の合計を各Providerでassert。時刻とcauseも個別比較する。 | eventが空でも通る合計比較、各eventの値を見ない件数だけの比較、同じblockのlostを複数回足す。 |
| T17a | `ur_17_a_missed_start_and_overrun_share_the_block_summary`。ManualTimeAuthority rigで `LateCommand` → Overrun report → restart lead + 50 ms gap 後のknown-tick samplesを通常の `Rx::receive` 経由で与える。 | eventは厳密に1個、cause=`Overrun`、`lost` は missed start と overrun gap の合計、timeは最初の損失sample。block flagsは `GAP_BEFORE | RESTARTED`。FakeDeviceのwall-clock待ちには依存しない。 | missed-start分またはoverrun分だけのlost、reportごとのevent。Mockはstart failureで受信を始めないためUHDのみ。 |
| T17b | `rx.rs` unit test。ManualTimeAuthorityのrigで `LateCommand` → samples（overrun reportなし）を決定的に与える。 | 正の飛びを持つblockが実際に届いたことをblock headerの `GAP_BEFORE` と正しい `lost` でassertし、`LATE_COMMAND` eventが厳密に1個、`RX_OVERFLOW` eventが0個であることをassertする。 | 共通関数へのreport変換で `MissedStart` を `Sequence` にする誤り、またはmissed-start-only gapから `RX_OVERFLOW` を誤発火する実装。Mockはstart failureで受信を始めないためUHDのみ。 |
| T18 | `rx.rs` unit testで Overflow report → 飛びのないblock → reportなしの後続jumpを決定的に与える。共通関数unit testも `reports + None => None` を固定。 | 最初のblockは `lost=None`・eventなし、後続blockは `lost=Some(...)` と `SEQ_DISCONTINUITY`・eventなし。Overflow eventが0個であることを明示assert。 | lost=0 event、pending reportの後続jumpへの持ち越し。**同じreport/no-gap入力をMock faultでは生成できない**：`apply_rx_fault` は正のlossがある場合だけreportを積む（`crates/ezsdr-mock-radio/src/lib.rs:761-768`）。禁止されているtest-only injection pathは作らず、この箇所はUHDの決定的unit testと共通関数のtestで固定する。 |
| T19 | `vh_8_a_report_without_a_following_block_emits_no_overflow` で両ProviderにT0のOverrunを起こし、restart blockより前のT0+10msに同じstreamをStopする（Mock fault at_ns 0、UHD `FakeFault::Overflow(Duration::ZERO)`）。Mock faultは適用済み、UHDはFakeDevice report counter `rx_overflows` が1であることを終了後に確認する。 | 両方ともreportを受けた証拠があり、後続blockなし、`RX_OVERFLOW` eventなし。手動device clockとAction queue barrierでstopをrestartより前に置く。 | report受信時の即時emit、stop時のflush、report未処理なのにeventなしを誤って合格させる。 |
| T20 | Mockのみ。`BackPressure::Block`・容量1を満たしたままfault後の再開blockをFullにし、Fullを続け、次block前にstopする。 | fault由来eventと各Full時のback-pressure eventを発生回数・時刻・cause・lostで個別assert。例のx3xx条件ではreport時刻1k、Full時刻51k/101kの3 event（各 lost 50k）。fault rowは元の1行のまま。stop後の追加eventなし。維持する `mr_19_backpressure_is_an_overrun` も継続。 | back-pressureを共通report関数に通して二重計上、Fullごとのevent欠落、stop時の架空event。UHDはFullを数えるのみ（UR-17）。 |
| T21 (review 1 P1-2) | T20と同じ容量1のBlock linkで、1msと102msにoverrun faultを置く。1ms fault後の再開blockと次blockをFullにした後、2つ目のfaultを再開gap中に発火させ、次のblockまでstepする。 | 2つ目のreport後のblock headerの`lost`はwhole jump 151k。RM-17の規則どおりsummary eventはcause=`Overrun`, lost=151k, time=最初の欠落sample 1k。先行したback-pressureの各eventも個別に確認する。 | event timeをreport instantにする、block jumpを新しいloss分だけにする、previously reported lossをblock summaryから除く（後者はwhole-block payload規則を変える）。 |

parityのevent時刻は異なるClockDomainId同士を直接比較せず、それぞれのreceive SampleClockでsample indexに正規化する。比較 rig のlinkを保持してblock headerを観測する。新しい系列はManualTimeAuthority/FakeDeviceのmanual clockまたは `rx.rs` の直接入力で決定的に作り、wall-clockの経過時間をassertしない（#68・#69）。

## 既存テストの置換・維持

- `radio_model.rs`: `rm_22_payloads_round_trip` の期待object、`rm_24_the_hot_form_round_trips` の17-byte固定・長さ・round-trip、`rm_24_a_delivered_payload_reads_in_either_form` の両形式を新payloadへ更新し、旧17-byteと旧object拒否を足す。`rm_20_schema_freeze` は生成結果を照合。
- `mock_radio.rs`: `mr_19_backpressure_is_an_overrun` はpayloadを `{cause,lost}` に更新し、profileの `overflow_restart_gap_ns` が実際のblock gapを作るassertは残す。`mr_20_faults_fire_at_their_instants` のsource/time/event-countとfault-row確認も維持。`mr_20a_the_overflow_comes_with_the_block_that_carries_it` は「先行eventなし→block時に1個」のevent値・時刻・flagを固定。`mr_21_overrun_shape` は1 msと10 msのfaultが同じ再開blockに合流する既存系列を残し、block/fault-rowのassertも維持するが、eventは `(time=1_000, cause=Overrun, lost=59_000)` の1個にする。T9の別の同block重複report比較はparityに置く。`mr_22_sequence_error_shape` にSequence eventのcause/lostを足す。`mr_37_the_overflow_travels_the_hot_path` は9-byte arrayと新payloadを固定。
- `rx.rs`: `ur_19_a_later_receive_stream_s_overflow_names_its_node` の17→9 byte assertionを更新し、cause/timeもassert。`ur_19_reports_before_one_block_keep_each_kind` は既存のalignment・reportなしのjumpを残し、混在reportと同種重複のevent数・payload cause/lostを直接見る。ここで`all(...)`だけをevent存在の根拠にしない。
- `v58.rs`: `v58_06_injected_overflow_is_a_uhd_overflow` のpayload equalityから `restart_gap_ns` を除く。capture continuityのgap・lostは不変。`v58_06_sequence_error_is_seq_discontinuity` のblock/capture assertionを維持し、必要なevent payload確認はT7 parityで担保。`v58_04_mock_events_reach_counters_policy_and_manifest` のcounter・policy・artifact markは維持。
- `mock_layout.rs` のlost/source比較は維持。`fake.rs::ur_19_a_layout_receive_report_names_its_stream` のsource確認、`ur_34_the_receive_stall_fires_after_its_device_clock_interval` のevent時刻assert、`common::rehearse_overflow` のcause確認（`rehearsal_b5_overflow` / `hw_b5_overflow`）も維持し、restart-gap field / 17-byte前提は足さない。`hardware.rs` のpayload表示は形のassertではない。
- `radio.timing.overflow_restart_gap_ns` はevent payloadと別のenvelope/profile設定。`radio_model.rs` RM-4 と `mock_radio.rs::mr_03_profile_values_reach_the_capabilities_and_the_envelope_section` のkey/value確認、およびcapture gapの確認は削らない。back-pressureのMR-19も別規則として残す。

実装段階で更新対象となる規範文書/schemaは、brief指定どおり `design/07-radio-model.md` のRM-11/17/18/22/24とtest表、`design/09-mock-radio.md` のMR-19/21/22/37とtest表、`design/18-uhd-radio.md` のUR-18/19/20、`schemas/radio/rx_overflow_payload.v1.json` と `rx_overflow_hot_payload.v1.json`。schemaは再生成してfreezeを通し、版・`SCHEMA_CHANGELOG.md` は変更しない。

## mutation rows（現状のold/new文字列検索）

`plan/**/mutations.json` の `old` / `new` を、brief記載の `restart_gap_ns|RX_OVERFLOW_HOT_BYTES|to_hot|from_hot|overflows|emit_overflow|emit_rx_overflow|Pending::Overrun|Pending::Sequence|RxOverflowCause` で検索した結果：

| file:id | 現行filter | 実装時の当て直し |
|---|---|---|
| `plan/maintenance/tools/mutations.json:F30` | `mr_20a_the_overflow_comes_with_the_block_that_carries_it` | reportの即時emitを検出する既存の「block前はeventなし→block時に1個」のassertを維持。 |
| `plan/maintenance/tools/mutations.json:F32` | `mr_20_a_fault_at_a_loss_s_instant_removes_nothing` | stream終了時にpendingをemitする変異を、同時刻loss後のeventなしで検出。元の到達可能なfault系列は維持。 |
| `plan/phase4/tools/mutations.json:D05`, `D06` | `rm_24_the_hot_form_round_trips` | cause-byte swapと長さ検査抜けを維持。新9-byte固定と17-byte拒否でkillする。 |
| `plan/phase4/tools/mutations.json:D09` | `mr_37_the_overflow_travels_the_hot_path` | fault eventのcontrol-path化を検出するよう、group後もhot arrayをassertするテストをfilterに保つ。 |
| `plan/phase4/tools/mutations.json:D33` | `to_hot` を含む | 共通のemit箇所が変わらなければ既存行を維持し、変わる場合は新しい実装箇所に当て直す。 |
| `plan/phase4/tools/mutations.json:D10` | `mr_19_backpressure_is_an_overrun` | back-pressure eventのcontrol-path化を検出。payload形だけ更新し、MR-19のhot path確認は残す。 |
| `plan/phase7/tools/mutations.json:U24`, `U26` | `ur_19_reports_before_one_block_keep_each_kind` | Overrun flagの誤りをT6/T8でも検出。`emit`→`emit_control`変異は集約後の9-byte hot event assertionへ当て直す。 |
| `plan/phase4/tools/mutations.json:D35`–`D38` | `rm_17_reported_losses_make_one_payload_for_the_block_jump` | overrun priority, whole-block `lost`, empty-report refusal and gapless-block refusalを各分岐で固定。 |
| `plan/maintenance/tools/mutations.json:F43` | `mr_21_overrun_shape` | 同じblock前の重複reportからeventを複数emitする誤りを検出。 |
| `plan/maintenance/tools/mutations.json:F44` | `mr_21_overrun_shape` | 前blockのOverrun reportを次blockのSequence reportへ持ち越す誤りを検出。 |
| `plan/maintenance/tools/mutations.json:F45` | `mr_19_a_fault_overflow_and_repeated_backpressure_are_counted_once` | back-pressure eventをreport aggregationへ遅延し、時刻/lostを変える誤りを検出。 |
| `plan/maintenance/tools/mutations.json:F46` | `mr_21_overrun_shape` | 先行back-pressureを含むblock summaryの時刻が最新reportの時刻ではなく、jumpの最初のlost sampleであることを固定。 |
| `plan/phase7/tools/mutations.json:U47`–`U49` | `ur_19_reports_before_one_block_keep_each_kind`, `ur_18_a_report_on_a_gapless_block_is_consumed_without_an_event`, `ur_17_a_missed_start_without_another_report_is_not_an_overflow` | UHDのreport-per-event, pending持ち越し, missed-start→Sequence誤変換を検出。 |

検索には `plan/phase3/tools/mutations.json:M19`（“a delay overflows the root”）も部分一致したが、radio overflowとは無関係なので変更しない。追加したD35–D38、F43–F45、U47–U49は、共通関数と両Providerの判定境界を固定する。

## 修正（Sol の計画レビュー）

- T17にmissed-start-onlyの決定的な `LateCommand → Samples` 系列を追加し、blockが正の `lost` と `GAP_BEFORE` を持つこと、`LATE_COMMAND` が1個、`RX_OVERFLOW` が0個を明示するT17bに分けた。
- mutation row一覧に `plan/phase4/tools/mutations.json:D33` を追加した。共通emit箇所の変更有無に応じて維持または当て直す。
- `mr_21_overrun_shape` の系列説明を訂正した。同じ再開blockに合流する既存の1 ms・10 ms faultを維持し、eventだけをtime 1,000・cause Overrun・lost 59,000の1個に直す計画とした。

## 実装時の詳細化

- parityのT6–T9/T19はMock faultのT0またはT0+50msとUHDの`FakeFault::... (Duration::ZERO)`で揃えた。FakeDeviceのfault時刻はdevice time zeroからの絶対値で、最初のreceive requestで発火するzero instantがT0に対する決定的な共通境界になる。T8/T9の2つ目のzero-duration faultは最初のoverrunのrestart後、50ms時点の次のreceive requestで報告される。
- T19を`vh_8_a_report_without_a_following_block_emits_no_overflow`として実装し、Stop前のMock fault rowとUHD `rx_overflows` counterでreport受信を観測する。
- review 1 の P1-2 混在系列を `mr_21_overrun_shape` の T21 ケースで固定した。採用済みRM-17のblock-wide `lost`を維持し、back-pressure後の別fault summaryもそのblockのwhole jumpを持つ。イベント合計が実損失の和を超えることはこの混在条件での既知リスクとして `report.md` に記録する（T12はback-pressureなしの範囲）。
