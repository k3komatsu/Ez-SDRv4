# 段階 3c-2a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §8 の 3c-2a．Provider が呼ぶのと同じ `Device::apply` を直接使う測定を 2 つ，`crates/ezsdr-radio-uhd/tests/common/measurement.rs` に足した（3b-1 の「raw の probe は値にしない」の例外．理由は設計メモ 41 §8 の末尾）．

| key | 測り方 | 読み方 |
|---|---|---|
| `radio.timing.command_queue_depth` | 3 s 先から 1 ms ずつずらした timed な `apply` を順に呼ぶ．条件は受信・送信の周波数，受信・送信の利得，受信と送信の周波数を交互 | 止まった呼び出しより前に返った数．条件ごとに 5 試行，全部の**最小** |
| `radio.{rx,tx}.c<n>.gain_db`，`antenna`，`radio.{rx,tx}.s<k>.sample_rate_hz` | profile の格子の全点を untimed の `apply` で要求して読み返す | 読み返しが要求と等しかった点の集合 |
| `radio.{rx,tx}.c<n>.gain_step_db` | profile の刻みの 1/5 ずつ掃引 | 読み返しの異なる値の隣どうしの差の最大 |
| `frequency_hz`，`frequency_step_hz` | 受信は全範囲から 50 点，送信は `SAFE_TX_HZ` の中で 20 点を読み返して observation に | `unmeasured`（owner の判断待ち） |

queue の深さは，時刻つきの `apply` の呼び出しという高い水準の操作の数で，FIFO の entry の数ではない．Sol が UHD 4.10 の source で確かめたとおり，OBX の timed tune は register の書き込みを複数出す．Provider（UR-24）も 1 command を 1 slot と数えるので，比べる量はこの数でよい．

試行の判定は次の形にした：
- device の時刻は最初の timed な `apply` の前に 1 回だけ読む．
- その後の時刻は，host の時計から device の時刻の上下の範囲を出して判定する．
  - 止まった呼び出しより前の呼び出しは，どれも上限が最初の command の時刻より前．
  - 止まった呼び出しは，下限が最初の command の時刻以後で，上限が最初の command の時刻 ＋ 100 ms 以内．
- host の時計と device の時刻が同じ速さで進むことは INFERRED．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna fast（04:07〜06:17） | risky でない扱い．セルフレビュー 1 回（free の DeepSeek が上限で Go の DeepSeek．P1 1〈fake のリハーサルが呼び出しの数を時刻に依って assert〉，P2・P3 は全部直した） |
| Opus | | O1（minor：rate の key の stream 番号を channel から取る），O2（note：送信の利得の掃引の RF 安全），O3（note：FakeDevice には深さ 16 の queue があった．指示書の誤り） |
| Sol コード | | FAIL：major 1（前の呼び出しの返りの時刻を見ないので，呼び出しの間に queue が空いた試行を深さとして数えうる），minor 1（UR-34 の本文に段階の名前） |
| 修正 | Luna fast（06:25〜07:29） | 全部．O1 は Description に stream の所属が無いので `shortcut:` の注釈で今のまま |
| Sol 見直し | | FAIL：新しい major 1（呼び出しごとの `time_now` が測っている queue を乱す） |
| 修正 2 | Luna fast（07:35〜08:05） | device の時刻は試行の前に 1 回だけ読み，その後は host の時計の上下の範囲で判定する |
| Sol 見直し 2 | | PASS |

`time_now` が queue を乱す仕組み（Sol が source で確認，VERIFIED）：
- UHD 4.10 の `get_time_now()` は radio の register を control port で読む（`radio_control_impl.cpp:388-404`）．
- control port は操作を一つずつ，ack を待って処理する（`axis_ctrl_slave.v:201-242`）．
- だから timed な command を積んだ後の時刻の読みは，最初の command の時刻まで待たされる．

最初の修正の指示書はこの読みを判定に使う形で書いた．orchestrator の指示の誤りで，3b-2・3c-1 と同じく，実機の振る舞いを source で確かめずに成功の条件を書いた形．

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 2 | 1 | どれも新しい測定の道具の中（返りの時刻の未確認，時刻の読みが測る対象を乱す）．`failure-mechanisms.md` の機構には当てない |
| Opus | — | 1 | O1 |
| セルフレビュー | 1（P1） | — | 直した |

risky でない扱いの段階で 3 回続けて（3b-2，3c-1，3c-2a），実機の振る舞いを読まないと見えない major がコードのレビューに出た．3c-2b の指示書では，成功の条件が頼る UHD の振る舞いを先に Sol に確かめさせる（テスト計画のレビューの代わりに，指示書の条件の表を Sol が source と照らす短い回）．

## 4. 残したこと

- **owner に確かめる（bench の前）**：
  - 送信の利得を 0 から最大まで掃引する（送信はしないので出るのは LO の漏れだけ）．bench.md の RF safety は送信する段階で利得 0 dB を求めているので，3d で bench.md に書く前に確かめる．
  - `frequency_hz`・`frequency_step_hz` を値にするか．連続の格子を抜き取りで測ったとみなしてよいかと，送信の測定を `SAFE_TX_HZ` の外でどう扱うか．
- **Provider への含み（INFERRED）**：Provider が run の中で `time_now` を読むと，release window（3 ms）の中で積んだ timed command の後ろで待たされうる．device lead の見積もりに入っているかを段階 4 で確かめる．
- bench の時間：queue は約 77 s と呼び出しの時間．coercion は約 2,100 回の `apply`（rate の変更の時間は未測定）．
- rate の key の stream 番号：Description に stream の所属が入ったら直す（`shortcut:` の注釈の所）．
