# 段階 3d：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [41](../41-measurement-records.md) §8 の 3d．Phase 8 の bench の手順と道具を tree に入れた．
- `plan/phase8/bench.md`：手順．
- `plan/phase8/tools/bench-host.sh`：handoff §6.3 のホストの設定．既定は確かめるだけで，`--apply` で当てる．
- `plan/phase8/tools/run-record.sh`：commit の写しを新しい時刻で取り出し，専用の volume で build して，`hw_measurements_record` と指定の test を流す．
  - 流す前に att の確認を対話で求め，省く道は無い．
  - 記録は repo の外に書く．
- `plan/phase8/bench-results.md`：2026-10-27 の 4 回の shake-out と D9 の probe の記録．

同じ日の実機の shake-out の結果を入れて書き，3c-2c と並行して別の git worktree（branch `stage-3d`）で進めた（owner 2026-10-27 の許可）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | luna-fast（13:10〜13:57） | セルフレビュー 1 回（free の DeepSeek：P1 1〈burst の隙間の値の欠け〉，P2 5，P3 6．出力の置き場の変更の 1 件を退け，ほかは直した） |
| Sol | | FAIL：major 1（snapshot を元の時刻で取り出すので，cargo が別の commit の build を使いうる），minor 2（誤った test の名前が「通った」になる，ホストの設定より前に probe） |
| 修正 | luna-fast（14:00〜14:11） | 全部．4 回目と D9 の結果を足した |
| Sol 見直し | | FAIL：major 1（runner を SIGKILL すると lock が外れ，container だけ残りうる），minor 1（D9 の P1 の「1 件ずつ取り出された」は log で示されていない．orchestrator の指示書の書きすぎ） |
| 修正 2 | Opus（直接） | container に volume ごとの固定の名前を付けた（Docker が同じ名前の 2 つ目を拒む）．D9 の文を INFERRED に直した．小さな修正なので再レビューは省いた |

## 3. 数

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 2 | 3 | build の cache と実行の対応（2），手順の順，記録の文．`failure-mechanisms.md` の機構には当てない |
| セルフレビュー | 1（P1） | 11 | 直した（1 件は退けた） |

## 4. 残したこと

- 実機での `bench-host.sh --apply` と `run-record.sh` の通しは未実施．次の bench で．
- att の型番と最大入力電力は記録が無く，次の送信の前に確かめる（bench.md）．
- 旧 `~/ezsdr-bench/run-bench.sh` は lock を取らないので，新しい runner と同時に使わない．
