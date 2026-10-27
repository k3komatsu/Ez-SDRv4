# 段階 3c-2c：orchestrator のレビュー，裁定，数

## 1. 何をしたか

2026-10-27 の最初の実機の流し（shake-out，[bench-results](../bench-results.md)）で見つかった不具合を直し，owner の決め（設計メモ [41](../41-measurement-records.md) §8 の末尾）の周波数の格子を足した．どれも試験のコードで，`src/` は変えていない．

| 項目 | 中身 |
|---|---|
| queue depth の詰まり | 条件は受信の利得，送信の利得，受信の周波数の 3 つ．受信の周波数の条件では，先に受信が sample を返すことを確かめ，受け取った sample が最後の命令の時刻を過ぎるまで受信を止めない．送信の周波数と交互の条件は外した（DUC は送信の sample が要る．設計メモ 39 §1） |
| 最初の失敗で群を止める | 一つの latch を命令，drain，片付けで共有し，各呼び出しの前に見る．時間の上限（既定 5 分）も各呼び出しの前に見る．保証は「ある thread が失敗を latch した後，ほかの thread で新しく始まる装置の呼び出しは高々 1 つ（UHD の 10 s の timeout で終わる）．それより後は無い」 |
| 失敗した handle を drop しない | 装置は lease に入れる．lease は既定で handle を捨てる（drop しない）．群・片付け・それより前の group のどれも失敗しなかったときだけ「drop してよい」と記す．途中のどこで panic しても UHD の teardown は詰まった control port に触らない |
| 記録を必ず書く | group ごとに実行し，`Err` と panic を group の失敗として記録する．装置の識別は group の前に読む．group ごとに `.partial.json` を書く．probe の行は要約だけを記録に入れ，全部は `<OUTPUT>.rows/<probe>.jsonl` に書く（4 回目の記録は 157 MB だった） |
| UR-7 の再試行 | PLL の lock の失敗だけを，3 回まで 1 s おきに再試行する |
| 拒否された Run | 受信・送信の throughput，overflow，full duplex で，Run が始まらなければ `AdmissionRejection` に分類し，termination を記録する |
| 周波数の格子 | 各 channel で両端と整数でない MHz の位置を含む 200 点を要求する．全点が 0.05 Hz 以内のときだけ `frequency_hz`（lo〜hi，1 Hz 刻み）と `frequency_step_hz`（差の最大の 2 倍）を値にする．送信は利得 0，streamer なしで全範囲（owner：att は 18 GHz まで） |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| Sol の調査 | 実機の 2 回目の詰まりの原因を UHD 4.10 の source で調べた | DDC/DUC の時刻つきの設定は sample の時刻でしか取り出されない（`axi_tag_time.v:206-211`）．設計メモ 39 §1 に既にあった事実で，3c-2a の指示書を書くときに orchestrator が読み落とした |
| 実装 | Sonnet 5.5 のサブエージェント（owner 2026-10-27 の許可．13:10〜13:47） | セルフレビュー 1 回（free の DeepSeek：P1 1，P2 2，P3 4．6 件を直し 1 件を退けた） |
| Opus | | O1（minor：記録が大きすぎる） |
| Sol コード | | FAIL：major 5（drain が DDC の設定の取り出しを確かめない，失敗の後も呼び出しが残る，命令の thread の panic で記録が書けない，上限を試行の中で見ない，記録の仕上げが失敗した装置に頼る），minor 1（UR-7 の再試行が PLL 以外も再試行する） |
| 修正 | Sonnet（同じサブエージェント，〜14:20） | 全部．Sol 5 は「装置が開けず識別も取れなければ記録は書けない」を残した（orchestrator が受け入れた：測ったものが無い） |
| Sol 見直し | | FAIL：major 3（片付けの失敗で handle を捨てない，thread の間の競合，`rx_stop` の後の `rx_close` が上限を見ない），minor 1（上限のテストが時刻に依る） |
| 修正 2 | Sonnet（〜15:05） | 競合は一部だけ採った：二つの thread を直列にすると，queue depth が測る `apply` の止まる長さが変わるので，保証を「高々 1 つ」と正確に書き，テストを決定的にした．ほかは全部 |
| Sol 見直し 2 | | FAIL：major 1（部分の記録を書くときの panic で，lease を「失敗」と記す前に drop される），minor 1（競合のテストが host のスケジュールに依り，外れると hang する） |
| 修正 3 | Opus（直接） | lease の向きを逆にした（既定で捨て，最後に「drop してよい」と記す）．競合のテストは 2 s 先の horizon にし（止まりの閾 1 s），待ちを 60 s で失敗に変えた．20 回通った．小さな修正なので再レビューは省いた |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 9 | 3 | 失敗の後の呼び出しと片付け（5），記録の頑丈さ（2），取り出しの確認（1），上限（1）．`failure-mechanisms.md` の機構には当てない（新しい測定の道具の中） |
| Opus | — | 1 | O1 |
| セルフレビュー | 1（P1） | 6 | 直した（1 件は退けた） |

実機の詰まりから来た「失敗した装置にそれ以上触らない」の規則は，二つの thread と片付けと drop にまたがった．そのため 3 回の見直しでも穴が残り続けた．最後は「既定で捨てる」に向きを変えて，道の数を減らした．

## 4. gate

- テスト 1842 全部通った．gate の 1 回目は 1 本落ちたが，再現しなかった（workspace をもう一度流して 1842 全部，`ezsdr-radio-uhd` を 2 回流して 671 全部）．gate の log は合計しか残さないのでどのテストかは不明．実装役が報告した実時間の既知の `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time` と見る（INFERRED）．
- clippy，`rules.py`（名前を変えたテスト 1 つを `--renames` で）0 件，docker の uhd（lib 392，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1637/1639．G09 は既知．B18 は remote の shard（load average 30〜37）で元のテストが落ちた「BASELINE FAILED」で，流し直して kill された．元のテストもローカルで 3 回通った．

## 5. 残したこと

- 実機で：raw の queue depth と coercion（周波数の格子を含む）を `run-record.sh` で流す．流す前に att を確かめる．
- 実機で，lease で捨てた handle が UHD の teardown を本当に飛ばすかは未確認．
- `hardware.rs` の準備の段階の panic の枝は fake の試験が無い．
- **本番の Module への含み（段階 4 の前後，owner）**：今の UHD Module は流れていない chain にも時刻つきの周波数を渡しうる．DDC/DUC に残った設定は後の時刻なしの構成を上書きし，早く当たることもある（D9 の probe で確認，[bench-results](../bench-results.md) 2026-10-27）．設計メモ 39 §3 の推奨（周波数を LO と DSP に分け，DSP は sample が必ず通る chain にだけ渡す）は `src/` を変える仕事．
