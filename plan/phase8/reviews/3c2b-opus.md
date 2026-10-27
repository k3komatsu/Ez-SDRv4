# 段階 3c-2b：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §8 の 3c-2b．値の行を作らない観測の probe を 6 つ，`crates/ezsdr-radio-uhd/tests/hardware.rs` に足し，`hw_measurements_record` から呼ぶ．

| probe | 何を見るか |
|---|---|
| `hw_h1_late_multi_packet_burst` | 開始が過去の複数 packet の burst．二つの場合を見る：時刻つきの send が全部遅れる場合と，時刻つきの頭に時刻なしの続きの send が付く場合．報告，send ごとの記録，聞こえた全部の区間を残す（保守表 3） |
| `hw_h2_underflow_mid_burst` | burst の途中の Underflow．尾は，頭の予定の終わり ＋ 10 ms まで host の時計で止めてから送る．PN の相関でずれを出す（保守表 2） |
| `hw_h5_held_last_sample` | 保持した最後のサンプルの期限．三つの場合を見る：隣り合う burst（対照），前もって予約した正の隙間（対照），1 つずつ submit して EOB を受け付けた後に次を出す場合（期限の道）（保守表 4） |
| `hw_56_stop_between_starts` | 受信の `cold` の変更の e₁ と e₂ の間の Stop．順序は転送する `Device` の記録から後で分ける．装置の水準の drain をしてから，`start_rx` で回復させる（#56 の 1） |
| `hw_56_stop_lag` | Stop を submit してから装置の `rx_stop` までの遅れ（host の時計）．各試行は `start_rx` で再開した証拠の後（#56 の 2） |
| `hw_ur17_receive_call_duration` | `rx_recv` の 1 回の時間と中身．rate は 390 625 S/s と 1・10・50 MS/s，要求長は 2 000 と 65 536．cut の近くの overrun も 1 つ観測する（UR-17） |

試験だけの転送する `Device`（`tests/common/probe_device.rs`）が，内側の `Device` に全部を渡しつつ，`rx_start`・`rx_stop`・`rx_recv`・`tx_send` を host の時刻つきで記録する．Provider と本番の道は変えない．計算（聞こえた区間，PN の相関，報告の数）は `tests/common/observations.rs` の純粋な関数にして，テストを付けた．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| Sol の事前の確認 | 実装の前に，指示書の手順を UHD 4.10 の source と照らした（3b-2〜3c-2a で 3 回続けて，実機の振る舞いを確かめない成功の条件が major になったため） | 6 つとも手順を変えた（指示書 §7）．例：UHD は時刻つきの send を分けると後の packet にも時刻を付ける，`stop` の timing の行は Session の Stop ではない，100 kHz の tone は 10 sample で一巡する |
| 実装 | Luna fast（08:31〜09:37） | セルフレビュー 1 回（Go の DeepSeek．P1 1〈H5 の波形の振幅が 0.566 で RF の assert に落ちる〉，P2 2，P3 1．全部直した） |
| Opus | | O1（minor：reader の一時停止に drop の guard が無い），O2（note：実装役が mutation を tree の外のコピーから流した．orchestrator が Write で作った記録が mode 600 だったため），O3（note：bench の時間） |
| Sol コード | | FAIL：major 2（#56 の再開が `radio.rx.c0.stream = 0` で何もしない．H5 の正の隙間が期限の道を通らない） |
| 修正 | Luna fast（09:45〜10:14） | 全部．#56 は `start_rx`，H5 に 1 つずつ submit する場合，drop の guard |
| Sol 見直し | | FAIL：major 1（受け付けられなかった 1 sample の EOB で次に進む），minor 1（mutation の名前），note 1（記録の負荷） |
| 修正 2 | Luna fast（10:18〜10:29） | 全部 |
| Sol 見直し 2 | | PASS |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（事前の確認） | — | — | 手順の変更 6．実装の前なので数えない |
| Sol（コード） | 3 | 1 | 再開の操作の取り違え，期限の道を通らない構成，受け付けの確認の漏れ．`failure-mechanisms.md` の機構には当てない |
| Opus | — | 1 | O1 |
| セルフレビュー | 1（P1） | 3 | 直した |

事前の確認は，実装の後なら major になっていた手順の誤りを 6 つ先に直した（たとえば H1 は条件 B を足さなければ保守表 3 の問いに答えない）．それでもコードのレビューで major が 3 つ出た．どれも Provider の操作の意味（どの action が segment を予約するか，いつ burst を閉じるか）を読まないと見えない形だった．事前の確認を続けるかは，次の 2〜3 段階で見る．

## 4. 残したこと

- どの probe も実機では流していない．判定はせず，観測を記録するだけ．
- H5 の結果は計測を挟んだ idle の bench のもので，記録の負荷は測っていない．
- bench の時間：約 68〜90 s（INFERRED）．
