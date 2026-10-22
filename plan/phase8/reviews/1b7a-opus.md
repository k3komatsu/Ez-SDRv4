# 1b-7a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [36](../36-radio-report-rules.md) の決定 1・2（#51）．受理した held の `TxBurst` を開始前に取り消したとき，何を報告するかを `ezsdr_radio` の 1 か所で決める．

- `ezsdr_radio::CancelReason { Stop, ProviderStop, Loss, ColdChange }` と `cancellation_reports`（`crates/ezsdr-radio/src/lib.rs`）．`ColdChange` だけが `radio.COMMAND_REJECTED` を出し，ほかは `rejected` の section に記録するだけ．
- MockRadio（`stop_tx`，`cancel_later`，`stop_at`，`tx_switch` の飛ばした clock）と UHD Module（`stop_now`，`do_switch`，`adopt_later`）の bool 引数と直接の `command_rejected`／`reject_action_at` を，理由の enum と Provider ごとの helper 1 つ（`record_cancelled_burst`，`report_cancelled_burst`）に置き換えた．理由の文字列は Provider ごとのまま．
- 挙動の変化は UHD の `Stop` だけ：取り消した burst に `COMMAND_REJECTED` を出さなくなり，MockRadio と同じになった．UHD の loss は shutdown を通るので `ProviderStop`（記録だけ，`DEVICE_LOST` が loss を知らせる）．`Loss` を production で渡すのは MockRadio だけ．
- spec 07 RM-16（規則の 1 文），spec 18 UR-23・UR-26，Changes．
- テスト：parity 10 本（T1〜T4，T13〜T15c．s0・s1 の Stop の独立を含む），unit 7 本（表 1，Mock 2，UHD 4），`fake.rs` の 2 本を直した（`|| true` の同語反復を消した）．
- mutation：F27・R07 を当て直し，P561〜P572 を足した．

本体とテストで約 +1080／−70．実装は GPT-6 Luna（`openai/gpt-6-luna#max`；free の DeepSeek が上限に当たったため，owner 2026-10-22），セルフレビューは Go の DeepSeek（`opencode-go/deepseek-v4.1-flash#max`）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | free の DeepSeek は上限で中断（19:02，走っていたのは 1 本）．Luna が書き直した | 系列 T1〜T20 のうち 1b-7a の分 |
| Sol の計画レビュー | 自分の失敗の筋書きから | **FAIL**，major 2・minor 2 |
| 計画の修正と実装 | Luna，19:35〜23:05 | セルフレビュー 2 回（P1 なし），選んだ 14 行 14/14 killed，新しいテストを 20 回続けて 20/20 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | major なし，minor 2 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**（major なし），minor 1 |
| 修正 | Opus が直した（テストの barrier，引数 1 つ，`*Test:*` の重複，mutation 3 行） | |
| gate | §4 | 1281/1282（G09 は既知） |

## 3. 指摘と扱い

計画レビュー（Sol，すべて採用）：
- **major 1**：cold change の取り消し（T4，T14a）と loss の shutdown（T13）で，`held`／`later` と共有の `core.held` の予約が消えることを見ていなかった．unit test で予約まで見る．
- **major 2**：TX の s0・s1 に未来の burst を置き，s1 だけの Stop が 1 行・event なしで，s0 は送り続けることを見る parity test が無かった．
- minor 2：UHD は飛ばした未来の clock の burst を黙って捨てると計画に書いていたが，`tx.rs` は `UR-25: its transmit clock was never started` を報告していた（T14a は回帰のテスト）．取り消しの箇所をすべて挙げ，どれも enum を通ることを確かめる．

実装の後：
- **O1（minor）**：UR-23・UR-26 の `*Test:*` が RM-16 にある `vh_8_*` を重ねて挙げ，UR-26 の行は 1500 字を避けて 2 行に折っていた（`design/` でここだけ）．重複を外して 1 行に戻した．
- **O2（minor）**：MockRadio の `stop_at` が `lost: bool` と `cancel_reason` で同じ事実を 2 度受けていた．`lost` を外した．P565・P566 を当て直し，消した `lost` を `new` に持っていた F29 を `cancel_reason != Loss` に直した（指示書の grep の対象）．
- **Sol（minor）**：parity の `uhd_barrier` は `set_timer` の拒否 event が 1 個あれば戻るので，同じテストの 2 個目からの barrier は前の event で成り立っていた．出した barrier の数まで待つようにした．
- セルフレビュー（DeepSeek，2 回）：P1 なし．採った指摘（予約の assert，s0・s1，T13 の理由，RX Stop の受理，Mock の TX block 数など）は Luna が直した．P3 2 件（`shutdown(&mut self)`，UHD の loss が `ProviderStop`）は理由を付けて退けた（[1b7a-report](1b7a-report.md)）．Opus と Sol はどちらも退けた理由を妥当とした．

## 4. gate

テスト 1595（clippy，`rules.py` 0 件），docker の uhd（clippy，lib 271，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1281/1282（G09 は既知）．

- 最初のローカルの gate は 7 件と Python 1 件が落ちた．別の利用者の解析 job が 16 コアを埋めていた（load 15.5）．負荷が下がってから流し直して全部通った．
- P245 は全リストの中で filter のテスト `ur_26_every_stream_s_threads_are_joined_in_order` が変異の前に落ちた（BASELINE FAILED）．単独で流し直して killed．このテストは HEAD でも揺れる（下）．

**持ち越し（1b-7a の外）**：`ur_26_every_stream_s_threads_are_joined_in_order` は負荷が無くても落ちる．12 回中の失敗は `8cd89e4`（1b-6a の前）で 2，`2af9afd`（1b-6a）で 1，`437d2d9`（1b-6b，この段階の HEAD）で 6．落ちる回は TX の 2 本の repeat burst の片方が FakeDevice の上で underflow し（`UR-22: the device burst underflowed; burst not resumed`，`the transmit timeline expired`），UR-22 のとおり再開しないので，テストが待つ 2 本目の `tx_send` が来ない．1b-6b で TX のスレッドの 1 回り（`pass`）が遅くなった疑いがある（INFERRED）．1b-7b の前に調べる．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（テスト計画） | 2 | 2 | テストの観測の穴．機構なし |
| セルフレビュー（DeepSeek，2 回） | P1 0 | P2・P3（採った 9，退けた 2） | テストの観測と文．機構なし |
| Opus | 0 | 2 | 文の重複，同じ事実の引数 2 つ．機構なし |
| Sol（コード） | 0 | 1 | テストの順序の観測．機構なし |
| gate | 0 | — | 既存の揺れ 1（上の持ち越し．機構は未分類） |

計画を先に書く手順（AGENTS.md §9 の 2・3）は，1b-6a・1b-6b と同じく計画レビューで major を拾い（2），コードレビューでは major 0 だった．`mech:radio-rule-implemented-twice` の報告の規則のうち，held burst の取り消しはこれで 1 か所になった．残りは 1b-7b（D2：`RX_OVERFLOW`）．
