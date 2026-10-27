# 段階 3c-3a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §8 の 3c-3 を 3c-3a（selector，多 channel，同時運転，候補の builder）と 3c-3b（再 open と H4）に分けた前半．試験と bench の道具で，`src/` は測定専用の builder `UhdRadio::with_measurement_performance` 1 つだけ．

| 項目 | 中身 |
|---|---|
| selector | `EZSDR_MEASUREMENT_SELECTOR`（`rx`，`tx`，`image`，`links`，`link_gbps` だけ．`rx`・`tx` は両方か無しか）．装置を開く前に検査し，記録の全部の profile に重ねる．channel ごとの周波数と RF の帯（その board の既定 ±1 MHz） |
| 多 channel | 受信・送信それぞれ，1 本の stream に全 channel・channel ごとの stream・rate の違う stream（BP4b）・同時運転（既定）．成功は stream ごとの証拠（sample の数，受け付けた send，ACK）．NIC ごとの byte 数を observation に |
| 容量 | 合計 T ごとに「T 以下の段が，その T を表せる全部の条件で全試行成功」の最大．走らなかった段・admission の拒否は止める側．候補の天井は link 1 本分（10 GbE で 1 250 MB/s，admission を通すためで link の容量の測定ではない） |
| 相手を流したままの lead と overflow | 値を持たない別の群（observation だけ）．失敗しても単独の値は残る |
| 失敗した装置 | 装置の手前の `GuardedDevice` と 1 つの latch：`apply` の失敗，制御の timeout，loss，開いたままの streamer，別の stream が持つ channel を開く（UHD の abort，1b-4e-2）で latch し，以後は何も装置に届かない（opener も開かない，`describe` は控え，drop は捨てる）．記録の識別は raw の probe より前に読む |
| raw の probe | selector の受信・送信の channel 0 が slot A の UBX・OBX のときだけ流す．それ以外は「この layout には当てはまらない」と記録 |
| RF の安全 | att の確認は送信する全 channel と，raw の probe か preflight が流れるときの slot A を名前で出す．新しい送信の波形は振幅 ≤ 0.5 を送る前に検査 |
| spec | UR-34（DA-1 の形），UR-5 の `link_gbps` の既定の誤り（HG は 1 GbE）を直した |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| Sol の事前確認 | 指示書の手順を UHD 4.10 と照らした（`sol-pre-3c3.md`） | CHANGE：6 項目全部を変えた（channel ごとの設定，link の割り当ては channel の接続ごと，容量は欠けた条件で `unmeasured`，候補の天井は admission の天井，相手が流れた証拠，H4 は proxy）．大きいので 3c-3a・3c-3b に分けた |
| 実装 | Sonnet 5.5 サブエージェント（約 2 時間 8 分） | セルフレビュー 1 回（free の DeepSeek は上限，Go の DeepSeek：P1 3，P2 8，P3 6） |
| Sol コード | | FAIL：major 4（相手の証拠が窓に属さない，latch の抜け，raw の probe が selector を無視，同時運転が既定で切れる），minor 1（DA-1） |
| 修正 1 | 相手の条件を observation に，`GuardedDevice` に集約，raw の probe の当てはまり，duplex を既定に | |
| Sol 見直し | | FAIL：major 5（opener・teardown・`describe` が包みの外，開いたままの streamer の検出が遅い，相手の失敗が単独の値を消す，preflight の送信が att の確認に出ない，新しい送信の振幅 0.566），minor 1（`rx_recv` の latch の試験） |
| 修正 2 | opener の前に latch，`describe` を控える，latch 後の drop は捨てる，識別を probe の前に，包みが開いた stream を数える，相手を別の群に，preflight で slot A，振幅 0.495 | |
| Sol 見直し 2 | | FAIL：major 2（同じ stream の開き直しで正常な raw の probe が latch する，片方だけの selector で送信が確認に出ない） |
| 修正 3 | 別の stream が持つ channel を開くときだけ止める（`UhdDevice::rx_open` は同じ stream の開き直しで古いものを先に捨てる），片方だけの selector を拒否 | |
| Sol 見直し 3 | | PASS |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（事前確認） | — | — | 6 項目を変更 |
| Sol（コード，見直し 3 回） | 11 | 3 | 失敗した装置に触らない（6：latch の抜け 1，opener・teardown 1，streamer の検出 1，開き直しの誤検出 1，相手の失敗 1，`rx_recv` の latch 1 は minor），RF の安全（3：preflight，片方だけの selector，振幅），証拠（1：相手の窓），範囲（2：raw の probe，duplex の既定） |
| Opus | — | — | 指摘なし |
| セルフレビュー | 3（P1） | 14 | 直した（P1-3 は退けた，テストを足した） |

**三振の規則**：「失敗した装置にそれ以上触らない」は 3c-2c（major 9 のうち 6）と 3c-3a（6）で同じ機構の指摘が続いた．`failure-mechanisms.md` の既存の機構には当てない（測定の道具の中）が，3 を越えたので設計を疑う：測定の群ごとに別の process で流せば，guard・lease・latch のほとんどが要らなくなる．owner に諮った（2026-10-27，返事待ち．推奨は今の形を残し，理由と固める試験を `failure-mechanisms.md` に書くこと）．

## 4. gate

- テスト 1877 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 392，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1687/1689．G09 は既知．P879（3c-2c の行：拒否された受信の段を成功に数える）が生き残った：1 channel の layout でも duplex の条件が既定で走り，同じ合計で拒否されて容量の上昇を止めるので，値の assert では見分けられなくなった．orchestrator が `rejected_runs_are_admission_rejections_not_group_failures` に「拒否された段が容量の observation で成功に数えられない」の assert を足し，P879 は kill．

## 5. 残したこと

- fake で試していない：`hardware.rs` の流れ（識別 → probe → 開き直し → 群，latch で群を飛ばす），probe 自身の handle での latch，本物の `open()`．att の確認は手で確かめただけ．
- 段の順は昇順（Sol 受け入れ），同時運転の段は受信と送信の群で別々に流れる（bench の時間が増える），相手の群は常に流れる（lead の時間が約 2 倍）．
- 2 channel の bench：#2 の UBX＋CBX（slot B の CBX の送信は配線が要る）か #1 に CBX を挿す．
