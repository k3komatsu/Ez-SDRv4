# 1b-5c-2：orchestrator のレビュー，裁定，数

## 1. 何をしたか

1b-5（今の key を消す切り替え）の最後．誰も使わなくなった今の key の宣言と builder，selector の一時的な `keys` を消し，spec と Vision の文を layout の key だけにした．

- `ezsdr_radio`：`DeviceDescription`（`device.rs` −354），`x3xx::description`・`Selection`・`x3xx::from_selector`（`x3xx.rs` −236），D12 で置き換わった今の key の宣言と `keys::` の定数，`channel_keys`（`lib.rs` −293）．`radio` 語彙の key は 50 から 32 になった．「そのまま」の key（`radio.full_duplex`，`radio.hardware_time`，`radio.tx.repeat_*`，`radio.rx.block_len`，`radio.timing.*`，`radio.perf.*`）は残した．`ezsdr_radio::addressed` は今の key の node の引数を失った．
- selector の `keys` は MockRadio（MR-2）と UHD Module（UR-5）で知らない selector の key として拒否される．fixture（Mock，UHD，acceptance，server の protocol，Python のテスト）から外した．`python/ezsdr` は変えていない．
- RM-26（`DeviceDescription` の実装）を取り下げて stub にし，timeline の規則（pruning の条件を含む）は RM-25 へ，tree・defaults・拒否・coercion は RM-2・RM-4・RM-5・RM-8 と RM-29・RM-30 にある．実行時の理由の接頭辞 `RM-26:` は `RM-25:`・`RM-30:` に変わった．
- 持ち越し：`mr_08_effective_holds_exactly_the_ten_configuration_keys` を `mr_08_effective_holds_one_channel_and_its_stream_rate` に改名，spec 07 の `radio.rx.coherent`，1b-3c の minor 1（MR-30 の引き順のテスト `mr_30_untimed_receive_tune_precedes_transmit_at_one_origin`，mutation P192）．
- spec 03・07・09・18，Vision §8・§13・§26（index に revision の行），Kernel の `spec.rs` の doc の例と `schemas/experiment_spec.v1.json`（版は不変）．

diff は 36 ファイル，約 +570／−1,800．本体は差し引き約 −890．実装は Luna，セルフレビューは DeepSeek（1 回で P1 なし）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna，08:22〜11:11 | セルフレビュー 1 回（P1 なし，P2 5・P3 数件を直した）．自分で全リストの mutation も流した（1257/1258） |
| Opus | 指示書への忠実さ，消したテストの代わり，mutation の `new`・`filter` の grep | major なし，minor 2 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 1・minor 3 |
| 修正の回 1 | Luna，11:20〜11:48 | 変えた・足した mutation の 9 行 9/9 killed |
| Sol の再レビュー | 直したところと隣 | **PASS** |
| gate | §4 | 1260/1261（G09 は既知） |

指示書・checklist・Sol の brief は書き換えられず，見張りも止められなかった．Sol は 2 回とも 1 回目の起動で動いた．

## 3. 指摘と扱い

- **Sol 1（major）：消した `rm_26_coerce_cases` のケースの一部に代わりが無かった**．gain の切り上げ（31.3 → 31.5）と，antenna の `Set` で先頭が無効なときに次の有効な値を選ぶケース．残ったテストは切り下げと同じ距離の丸め，先頭が有効な集合しか見ていなかった．番号つきの key にして `rm_30_coerce_cases` へ移し，19.5 Msps → 20 Msps と送信の無い channel の拒否も足した．mutation P549（最も遠い rate）・P550（いつも切り下げ）・P551（集合の先頭だけ見る）を足して killed．同じ 30.25 dB の tie，7 GHz と throughput の拒否，整数 Hz でない rate の拒否は `mr_06_coerce_cases` が見ていることを Sol が確かめた．
- **Sol 2（minor）**：Vision §8 の例が「coherent な sensing array」の意図を失い，4 channel の所属だけになっていた．D13 のとおり，channel ごとの `random_unless_timed_tune` は timed tune の後の coherence を許すだけで，成り立った関係は Provider が Manifest に記録する，と書いた．
- **Sol 3（minor）**：RM-8 の `radio.d.frequency_step_hz`・`gain_step_db` と Vision §26 の `radio.{tx,rx}.path_delay_samples` が消えた key のままだった．channel 番号つきに直した．
- **Sol 4（minor）**：RM-26 にあった pruning の一般条件（後の booking が先行せず後の refusal が届かない境界より前だけを prune し，状態に畳み込み，plan を変えない）が RM-25 へ移っていなかった．移して `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item` を引いた．
- **O1（minor）**：P333 の mutant が消えた `keys::CONFIGURATION` から `if false {` に変わったが，`name` が古いままだった．直した．
- **O2（minor）**：`RM-26` → `RM-25` の置換で Mock のコメントが「RM-25, RM-25, RM-27」になっていた．直した．
- 消したテストの代わり（Opus が確かめた）：`mr_38_a_layout_instance_uses_one_key_set` の他の確認（消えた方向全体の key の拒否，番号の無い channel の拒否，layout の key の coercion）は `rm_30_coerce_cases`（`channels.rs` の「not a radio key」の loop）と `mr_03_*`・`mr_06_coerce_cases` にある．`rm_26_the_layout_parser_checks_the_shared_selector` は今の key の selector の parser のテストで，layout の parser の拒否（位置，重複）は `rm_28_the_layout_analysis_cases` が見ている．
- **process**：前の段階で起きた「`new` に消した名前を持つ行」「filter のテストが無くなった行」は，指示書に grep を書いたら起きなかった（Luna が 0 件を報告し，Opus も grep で確かめた）．到達できる系列を代わりなしで消す癖は，今回は coercion のケースの形でまた出た（Sol 1）．

## 4. gate

テスト 1556（HEAD の 1562 から消したテストを引き，足したテストを加えた），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 245，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1260/1261（maintenance・phase3〜6 は全部，phase7 は G09 だけが既知の survivor．P472 は今回 killed）．この段階で壊れた行は無かった．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，1 回） | P1 0 | P2 5・P3 数件 | 文の書き残しとテストの空照合．機構なし |
| Opus | 0 | 2 | mutation の行の名前，置換の誤り．機構なし |
| Sol | 1 | 3 | テストの範囲（消したケース），文の移し漏れ．機構なし |
| gate（mutation） | 0 | — | — |

`failure-mechanisms.md` に足すものはない．
