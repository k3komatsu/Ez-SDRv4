# 段階 3g：orchestrator のレビュー，裁定，数

## 1. 何をしたか

2026-10-28 の実機で D9 の probe P5〜P7 の音の判定がすべて INCONCLUSIVE になり，P4 は stop が e の後になった（[bench-results](../bench-results.md) 2026-10-28）．Opus の分析（repo の外：`~/.cache/ezsdr-fixes/2026-10-26-stage3/probe-analysis-1028.md`）を仕様に，2026-10-29 の再実行のために試験の道具だけを直した（`src/` は変えない，risky でない）．

| 項目 | 中身 |
|---|---|
| 音の判定（`tests/common/tone.rs`） | 雑音の床を coherence でなく電力で（窓の平均電力 > 床の 4 倍，かつ coherence > 0.5）．受信 gain 0 dB で雑音が約 1 LSB のとき雑音の coherence が 0.55 前後になり，閾値 2×床が 1 を超えて何も通らなかった．burst の最初の窓から既に移っている場合は欠けでなく「at or before」 |
| P4 | 最初の音を 30 000 sample に（100 ms の音の send が送信 buffer で詰まり stop が遅れた），stop を device の時刻 e − 5 ms で |
| P5 | priming の burst を 400 ms 先に，印字する時刻を DUC が数える時刻に，owner の決め 5（Run の始めに時刻なしの空の EOB）を確かめる 3 つ目の case |
| P6・P7 | 最初の timed tune の前に時刻なしの空の EOB（F7 を消す）．P6 の starvation の判定は純粋な `continuation_in_loopback` に：基準と捕った区間が足りなければ INCONCLUSIVE，送信の Underflow と受信の error は別に印字 |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Sonnet 5.5（worktree）．セルフレビューなし | P997 が残り，静かな窓 5 つの assert を足して kill |
| Sol コード | | FAIL：major 1（P6 は受信が欠けると続きの数が必ず 0），minor 1（at or before） |
| 修正 1 | 2 件 | |
| Sol 見直し | | PASS |

## 3. 数

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（コード） | 1 | 1 | 観測の欠けを結果と取り違える（試験の道具） |

## 4. gate

- テスト 1957 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 433，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1774/1775（G09 は既知）．1 回目は worktree からの cherry-pick で 4 ファイルが mode 600 になり全行失敗，644 にして流し直した．
