# 1b-7b：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [36](../36-radio-report-rules.md) の決定 4・5（D2）．`radio.RX_OVERFLOW` を何個，どの値で出すかを `ezsdr_radio` の 1 か所で決める．

- `RxOverflowPayload` から `restart_gap_ns` を外した：`{ cause, lost }`，hot path は 9 byte（byte 0 が cause，1〜8 が `lost` の LE u64）．旧 object と旧 17 byte は拒否する．schema 2 つを再生成した．
- `ezsdr_radio::payloads::rx_overflow_payload(reports, lost)`：block の前の report から 0 個か 1 個の payload を作る．cause は overrun があれば overrun，なければ sequence．`lost` は block の飛びのうち，まだどの event も報告していない分．MockRadio（`publish_rx`）と UHD Module（`Rx::samples`）が呼ぶ．
- MockRadio：fault ごとの event を，その report を運ぶ block ごとの 1 個にした．report は fault ごとの損失を持ち，event の `lost` はその合計，時刻は最初の `k_f`．back-pressure（MR-19）は今のまま捨てた時点で報告し，共通の関数を通らない．
- UHD：report ごとの event を block ごとの 1 個にした．missed start（`LateCommand`）だけの飛びと，飛びの無い block に付いた report は event を出さない（今のまま）．`ALIGNMENT_ERROR` と block の flags は今のまま．
- spec 07 RM-11・RM-17・RM-18・RM-22・RM-24 とテスト表，spec 09 MR-19・MR-21・MR-22・MR-37 とテスト表，spec 18 UR-17〜UR-20 とテスト表，Changes．
- テスト：parity 5 本（`vh_8_*`：overrun，sequence，混ざった report，同じ種類の繰り返し，block の来ない report），unit（`rm_17_*`，UHD の `ur_17_*` 2 本・`ur_18_*`，Mock の `mr_19_a_fault_overflow_and_repeated_backpressure_are_counted_once`），既存の `mr_19`・`mr_20a`・`mr_21`・`mr_22`・`mr_37`・`ur_19`・`rm_24`・`v58_06` の期待を新しい形にした．
- mutation：F43〜F46，D35〜D38，U47〜U49 を足し，F32・D33 ほかを当て直した．

本体とテストで約 +560／−140．実装は GPT-6 Luna（`openai/gpt-6-luna#max`），セルフレビューは Go の DeepSeek（`opencode-go/deepseek-v4.1-flash#max`）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | Luna | 系列 T6〜T12，T17〜T20 |
| Sol の計画レビュー | 自分の失敗の筋書きから | **FAIL**，major 1・minor 2 |
| 計画の修正と実装 | Luna，07:08〜09:55 | セルフレビュー 2 回（1 回目 P1 2 件，2 回目 P1 なし），選んだ 24 行 24/24 killed，テスト 20 回続けて 240/240 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | **major 1**，minor 1 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 1（Opus と同じ）・minor 2 |
| 修正 | Opus が直した | Sol の再レビュー PASS，触った 5 行 5/5 killed |
| gate | §4 | |

## 3. 指摘と扱い

計画レビュー（Sol，すべて採用）：
- **major**：missed start だけの飛び（`LateCommand → Samples`）で `RX_OVERFLOW` を出さないことを見るテストが無かった．`MissedStart` を `Sequence` に変換する誤りが通る．UHD の unit test を足した（`ur_17_a_missed_start_without_another_report_is_not_an_overflow`）．
- minor 2：mutation の D33 の漏れ，`mr_21_overrun_shape` の系列の誤記（1 ms と 10 ms の fault が同じ再開 block に合流する．event は `(1 000, overrun, 59 000)` の 1 個）．

実装の後：
- **O1＝Sol 1（major）**：MockRadio で back-pressure の後，次の block の前に fault が来ると，fault の event の `lost` が block の飛び全体になり，back-pressure の event と，`Full` で捨てた block の fault の event が報告済みの損失をもう一度数えていた．`mr_21_overrun_shape` の最後の系列で event の `lost` の合計は 351 000，実際に失ったのは 201 000．時刻も報告済みの sample 1 000 に戻っていた．テストと spec 09 のテスト表はこの値を固めていた．セルフレビュー 1 回目の P1-2 が同じことを指摘したが，Luna は「決定 5 の block 全体の `lost` が優先する」として退け，報告の「既知のリスク」に回した．メモ 36 の T20 は「2 重に数えない」と決めており，決定 5 の「block の飛び全体」はまだどの event も報告していない飛びについての文なので，owner の裁定は要らないと判断した．直し方：Mock の report が fault ごとの損失を持ち，共通の関数にはその合計を渡す．時刻は最初の `k_f`．RM-17 に「先の event が報告した損失を除く」，MR-19 に「後の event はこの損失を数えない」を足した．UHD は back-pressure の event を持たないので変わらない．
- **O2＝Sol 2（minor）**：`apply_rx_fault` の字下げが崩れていた．手で直した．
- **Sol 3（minor）**：spec 09 のテスト表に足した 3 行が 4 列（表は 3 列）で，`|` が escape されていなかった．直した．
- 修正の mutation：F46 を「`reported` を block の `lost` にする」に当て直した．F43 の mutant は `reported` が使われなくなって型が推論できず compile error になったので，本体の `reported` に型を書いた．
- セルフレビュー（DeepSeek，2 回）：1 回目の P1-1（T17a のテスト無し）は Luna が直した．P1-2 は上の O1．2 回目は P1 なし，P2・P3 は Luna が直した（[1b7b-report](1b7b-report.md)）．

## 4. gate

テスト 1601（clippy，`rules.py` 0 件），docker の uhd（clippy，lib 274，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1292/1293（G09 は既知）．

- ローカルの workspace のテストは 4 件が落ちた（流し直すと 2 件）．どれも `crates/ezsdr-radio-uhd/tests/fake.rs` の実時間の FakeDevice の TX のテストで，1b-7b は触っていない．別の利用者の解析 job で load は 15〜16．落ちた 6 本を単独で 3 回ずつ流すと 5 本は 3/3，`ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned` は 2/3．#69 の揺れで，#69 に挙げた 3 本の外の `ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`，`ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`，`ur_23_a_burst_one_sample_after_a_burst_is_played` も負荷で落ちた（Luna の報告にもある）．#69 の設計メモはこれらも扱う．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（テスト計画） | 1 | 2 | テストの観測の穴．機構なし |
| セルフレビュー（DeepSeek，2 回） | P1 2（採った 1，退けた 1 は誤り：O1） | P2・P3（採った） | テストの観測と文．機構なし |
| Opus | 1 | 1 | O1：規則の例外（back-pressure を共通の関数の外に置いた）との境で，メモの 2 つの文のうち片方だけを実装した．機構なし（下） |
| Sol（コード） | 1（O1 と同じ） | 2 | 同上，書式 |
| gate | 0 | — | 既存の揺れ（#69，`mech:wall-clock-test-oracle`．§4） |

O1 は `radio-rule-implemented-twice` に数えない：Provider の間で規則が割れたのではなく，Mock の中の back-pressure と fault の 2 つの報告の境で，メモの決定 5 と T20 のうち T20 を落とした．退けたセルフレビューの指摘を orchestrator と Sol が拾った．計画を先に書く手順（AGENTS.md §9 の 2・3）は，計画レビューで major を拾い（1），コードレビューの major は計画に入っていた系列（T20）の期待の誤りだった．`mech:radio-rule-implemented-twice` の報告の規則（held burst の取り消しと `RX_OVERFLOW`）はこれで両方とも 1 か所になった．
