# Phase 8 段階 1b-3a — orchestrator（Opus）のレビュー

対象：作業ツリー（HEAD `5228a35` からの diff：`crates/ezsdr-mock-radio/src/lib.rs` と mutation 3 list）．

## 判定：差し戻しなし（指摘 0，記録のみ 3）

確かめたこと:

- `git diff HEAD -- 'crates/*/tests/*'` は空．テストは1行も変わっていない．
- 組み直しの各関数（`emit_rx_until`，`publish_rx`，`apply_rx_fault`，`schedule_wakeup`，`cut_segments`，`end_open_segment`，`transmit_until_cut`，`stop_tx`，`stop_at`，`rx_settle`/`rx_finish`/`rx_begin`，`book_rx`/`book_tx`，`tx_reconcile`，`tx_switch`，`cancel_later`，`forget_later`，`handle_tx_burst`，`emit_tx_until`，`apply_update`，`advance`，`prepare`/`arm`/`start`/`cleanup`）を HEAD と1行ずつ突き合わせた．方向ごとの stream が1本のとき，乱数を引く順（`rx[0].rng`，`lo_retune`），event・block を出す順，`seq` の振り方，エラーの文面は HEAD と同じ．`stop_at` の「全 stream の `stop_tx` → 全 stream の `cancel_later`」は1本では HEAD の順．
- `Lines::book` は move も floor も無いとき `Line::book` と同じ結果（`settle_once` の release は `floor = NO_RELEASE` のまま変わらない．`timeline.rs:643`，`:680`）．
- channel mode の判定が `self.channel` から `tx[k].plan`/`rx[k].model` の有無に変わった箇所は，両者が `medium.join` の成功後に同時に立ち，`cleanup` が stream ごと作り直すので同値．
- 当て直した 28 行は同じ規則を同じ形で壊し，`remote-mutate-3.log` で 88/88 killed．`rules.py` 0 problem．

記録のみ（1b-3b で扱う．報告 §8 と一致）:

1. `start` 失敗時の巻き戻しが `Lines` を作り直して送信の line を入れ直す（`lib.rs:1915-1922`）．move を記帳するようになる 1b-3b では，この形は他の stream の move を落とす．そこで `Lines` に setup 用の削除を足すか，失敗時に line を入れない順序にする．
2. stream 0 の決め打ち（`handle_action` の宛先，cold key，`apply_update`，`apply_rx_fault(0, ..)`，`prepare` の declare）は D14 の宛先と layout の tree を入れる 1b-3b で外す．
3. `Lines::book` が毎回 `Lines` 全体を clone する．今の Mock の記帳頻度では問題ない．
