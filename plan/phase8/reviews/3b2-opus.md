# 段階 3b-2：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §5 の後半：Provider を通した測定の群を 5 つ，`crates/ezsdr-radio-uhd/tests/common/measurement.rs` に．

| key | 測り方 | 読み方 |
|---|---|---|
| `radio.timing.min_timed_command_lead_ns` | 候補の lead ごとに Session を 1 つ．Session の `send` を受け取り（log の entry の時刻）から L で．試行を順に | 成功の suffix |
| `radio.timing.tx_in_flight_ns` | 候補の window W ごとに Session を 1 つ．走っている repeat に W ＋ device lead ＋ delivery allowance ＋ 1 ms（INFERRED）で割り込む | 成功の suffix |
| `radio.timing.rx_start_lead_ns` | 候補ごと，条件（1 ⇄ 2 MS/s，390 625 ⇄ 400 000 S/s）ごとに Session を 1 つ．`cold` の rate change をまたぐ capture | 成功の suffix，2 条件の交わり．値の行は効いた条件 |
| `radio.perf.rx_bytes_per_s` | rate ごとに Spec の Run を D 秒 | 成功の prefix × `wire_bytes_per_sample` |
| `radio.full_duplex`，`radio.hardware_time` | loopback の Session を試行ごとに | 全試行成功で真，一つでも失敗で偽 |

成功の判定は Manifest の section から試行ごとの証拠を取り出す純粋な関数（`extract_trial_evidence`）で，3b-1 の 6 分類と帰属を使う．送信の完了は試行の区間で burst の target 以後の `BurstAck` がちょうど 1 つ（UHD 4.10 は ack に radio が EOB を処理した時刻を入れる．排他的な終わりとの差は INFERRED で，差を observation に残す）．in-flight は割り込まれた repeat の record が割り込みの target でちょうど終わることも要る．throughput の期待する数は記録された受信の SampleClock の区間 × rate．`hw_measurements_record` は形の検査が通れば記録を先に書き，それから profile との比較を assert する．FakeDevice の記録では測った key も `unmeasured`．queue depth は 3c．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna（18:44〜22:07） | risky でない扱い（テスト計画なし）．セルフレビュー 1 回（free の DeepSeek，P1 0，P2 3，P3 5） |
| Opus | | O1（major：throughput を `D × rate` と比べるが止まりは `finish()` 任せ），O2（minor：rx start lead の条件が固定），O3（minor：ack の差を残さない） |
| Sol コード | | FAIL：major 4（`BurstAck` の tick を排他的な終わりと等しいと要求，repeat が走っていなくても in-flight が成功，O1 と同じ，比較が通らないと記録を書かない），minor 1（実効の lead が submit の後の時刻） |
| 修正 | Luna fast（22:15〜23:10） | 全部．区間の外の失敗の証拠を捨てるセルフレビューの変更を戻した |
| Sol 見直し | | FAIL：minor 3 |
| 修正 2 | Opus（直接） | minor 1：倍数の測った値が正でない記録を書く前に拒否（P775）．minor 3：repeat が 1 sample 早く終わる場合を足した．minor 2 は退けた（下）．Luna が足した 14 行の mutation の字下げを揃えた |

minor 2 を退けた理由：`tx_trial` が `entry.time` を使う選択は呼ぶ所の 1 行で，二つの読みの間で時刻が進む決まった rig が統合の Run に無い（手動の時計は進まず，元の mutant は生き残った）．`entry.time` は Kernel が log に書く受け取りの時刻（`crates/ezsdr-kernel/src/session.rs:117-122`）．minor だけの修正なので owner の決め（2026-10-26）で Sol の再レビューを省いた．

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 4 | 4 | どれも新しい測定の道具の中（完了の証拠，区間，止まりの時刻，記録の保存）．`failure-mechanisms.md` の機構には当てない |
| Opus | 1 | 2 | O1 は Sol 3 と同じ．指示書の表が「D × rate」と書いていたのが元で，orchestrator の指示の誤り |
| セルフレビュー | — | — | P2-2（区間の外の証拠を捨てる）は 3b-1 の規則を弱めた変更で，修正で戻した |

risky でない扱いにした段階で major 4 がコードのレビューに出た．どれも bench でしか起きない形（実機の ack の時刻，止まりの遅れ，repeat の失敗，食い違う結果の保存）で，FakeDevice の試験では見えない．3c の指示書では，実機での成功の条件を UHD の source で確かめた形で書く．

## 4. 残したこと

- `BurstAck` の時刻と排他的な終わりの差：最初の bench の記録で読む．
- `radio.perf.rx_bytes_per_s`：1 channel の最大は 200 MS/s × 4 byte = 800 MB/s で，profile の 1 000 MB/s に届かない．今の測定では比較が必ず落ちる．2 channel で測るか profile の値を見直すかは 3c で決める．
- bench の時間：既定で Run 55 本，装置の時間で約 13 分と Run の始まり約 2 分（INFERRED）．実機では何も流していない．
