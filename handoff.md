# Ez-SDR v4 — Handoff (2026-09-21)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `git@github.com:k3komatsu/Ez-SDR.git` |
| `main` | v4（この worktree）．commit 3 個: `1dfd39f init` → `92ae363 fold v4 into repo` → `46de04b ignore .DS_Store`．`origin/main` と同期 |
| `master` | v3（D + C++ UHD bridge + Python client）．tip `4a474e9` = tag `v3.0.28`．**GitHub の default branch のまま** |
| tags | 35 個（`v2.11`, `v3.0.0`–`v3.0.28`）．すべて v3 系 |
| 履歴の関係 | **無関係（unrelated）**．graft も merge もしていない．v4 は clean-sheet なので今後も繋がない |

ローカル：

- `/Users/komatsu/GoogleDrive/github/Ez-SDRv4` = `main` の worktree（メイン）．
- `…/Ez-SDRv4/v3` = `master` の **git worktree**（入れ子 clone ではない）．`.gitignore` で除外．
- 設計文書が v3 のパス 20 件を証拠として引用しているので `v3/` は消さない．確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- `v3/` を消してしまった場合：`git worktree prune && git worktree add v3 master`

## 2. 設計文書の状態 — Phase 0 完了

- 単一の設計ソース = **Vision**：索引 [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) + [design/vision/](design/vision/) の 11 part（§1–§68，番号は不変）．
- [design/v4-vision-audit.md](design/v4-vision-audit.md)（Findings 1–34，判定 READY WITH REQUIRED CHANGES）→ 全項目を Vision に反映済み．
- [design/v4-vision-rereview.md](design/v4-vision-rereview.md)（Findings R1–R22，判定 READY）→ 全項目反映済み．R13「規範部分の spec 化」だけは 11 ファイル分割で暫定対応し，**本対応は Phase 1 の作業**として繰り延べ．
- 旧 CMA は退役．[design/archive/](design/archive/) に保管（audit / rereview の `CMA §N` 引用のためだけに残す）．編集しない．
- Vision の改訂履歴は索引ファイル末尾の表（7 pass，すべて 2026-09-21）．

## 3. 実装の状態

**なし**．Rust crate，Python client，スキーマ，wire protocol，いずれも未着手．`Cargo.toml` も存在しない．実装は Phase 1 の spec が 3 ゲートすべてを通ってから（§4）．

## 4. 進行中 — Phase 1: Kernel semantic model

Vision §67 の Phase 1．詳細設計を [plan/phase1/](plan/phase1/) に書いている．計画本体と横断決定は [plan/phase1/00-overview.md](plan/phase1/00-overview.md)．

| spec | rule ID | 状態 |
|---|---|---|
| [01-time-model.md](plan/phase1/01-time-model.md) | TM-1..21（副番含め36） | **Gate A：敵対的レビュー3巡完了，ユーザ確認待ち** |
| [02-stream-contract.md](plan/phase1/02-stream-contract.md) | SC-1..32（副番含め48） | **Gate A：敵対的レビュー3巡完了，ユーザ確認待ち** |
| [03-spec-and-binding.md](plan/phase1/03-spec-and-binding.md) | SB-1..49（副番含め54） | **Gate B：敵対的レビュー2巡完了，ユーザ確認待ち** |
| [04-run-and-session.md](plan/phase1/04-run-and-session.md) | RS-1..52（副番含め57） | **Gate B：敵対的レビュー2巡完了，ユーザ確認待ち** |
| [05-module-api.md](plan/phase1/05-module-api.md) | MA-1..46 | **Gate C：敵対的レビュー1巡完了，ユーザ確認待ち** |

決定済みの前提（2026-09-21）：草稿は `plan/phase1/`，受理後に spec 01–05 を `design/` へ移す（00 は plan/ に残す）；本文は英語；3 ゲート（A = 01+02，B = 03+04，C = 05+00）；Phase 1 は spec 受理後に Cargo workspace + 単一 crate `ezsdr-kernel` + tests + `schemas/` まで実装する．

Gate A は敵対的レビュー3巡（31件・16件・13件）を経て通過．判定基準として指定された `ContinuityBuilder` の7シナリオ＋自作1件を Python で実行し 8/8 通過（`trace.py`）．Gate B は2巡（19件・10件）．2巡目の P1 3件はいずれも「時刻値が Spec・Session・Kernel Action の間を渡る継ぎ目」で，`ActionTemplate`，`Vocabulary{at}`，`TxBurst.at: AbsoluteDeadline` で一括解消した．

レビュー中に v3 引用の事実確認も走らせ，1件が未裏付けと判明（`v3.0.21.md` に `CONSTANTS`/`!COMPUTE` の動機は書かれていない．Vision §9 の読み）．該当箇所は「証拠ではなく Vision の読み」と明記．副産物として，`v3.0.20.md` の1行目が `# EzSDR v3.0.17` であることが判明し，audit Finding 6 が出典を誤った理由が説明できた．

Gate C は1巡（8件）．P0 は1件で，`PrepareContext` に Action の送信口が無く，Reactor が `TxBurst` を発行する手段が API に存在しなかった．Vision §19 の反応型モデルと §58 #9 が実装不能だった．`ActionSubmitter` を足し，MA-6 の handle 集合と MA-46 の凍結集合にも入れた（v4.0 後に足すと Kernel major になるため）．

6文書で259ルール，欠番と未解決参照なし，撤回5件（SB-28, SB-32, RS-37, MA-4, MA-43 — OV-1 に従い番号は保持）．型名はすべて定義済み．

Phase 1 の完了条件は 00-overview.md §13．R13（Vision の規範部分を要約 + リンクに戻す）は **spec 受理後に別途ユーザ承認を得てから**実施する（手順は 00-overview.md §12）．

型を書くときの拘束文は Vision §5（tiers），§15（time），§23（Stream Contract），§13（TimingEnvelope / Mock 強制），§3（Session），§8（composite resource，BindingProfile）．audit §13「Recommended Minimal Core」が freeze 対象の一覧で，§14.1 の項目 1–15 は 00-overview.md §10 の traceability 表に対応付けてある．

## 5. v3 → v4 切替で残っている作業（人間の判断が要るもの）

v4 が使える状態になるまで着手不要．ただし順序に注意：**default branch を `main` に変える前に Docker の件を片付ける**．

| # | 項目 | 状態 |
|---|---|---|
| 1 | GitHub の default branch を `master` → `main` | 未．v4 が使えるまで据え置き |
| 2 | `ghcr.io/<owner>/ezsdr:latest` が v4 初回リリースで v3 → v4 に化ける | 未．v3 の `.github/workflows/ezsdr-build.yml` が `release: published` で `:latest` を push する（`matrix.uhd == DEFAULT_UHD` のとき）．v3 利用者を固定 tag（例 `:v3.0.28`）へ誘導してから v4 をリリースする |
| 3 | v3 readme の `ghcr.io/k3kaimu/ezsdr:latest` と現 owner `k3komatsu` の不一致 | 未確認．workflow は `github.repository_owner` を使うので現在の image は `k3komatsu/ezsdr` のはず |
| 4 | `release: published` がどの branch の workflow file を使うか | **未検証**．初回の v4 release で実測する |
| 5 | dub registry の `ezsdr` package | 既存 tag は解決し続ける．`~master` 指定と新 version 登録は default branch に `dub.json` がある前提 |
| 6 | `master` の readme に branch 構成の一行を足す | 未 |

## 6. 今日決めたこと（理由の記録）

- 別 repo だった v4 を同じ repo の `main` に収めた．v4 は commit 1 個・remote なしだったので history rewrite も force push も不要だった．
- 履歴を繋ぐ選択肢（graft，rebase，`--allow-unrelated-histories`，`merge -s ours`）はすべて不採用．存在しない系譜を記録するだけだから．
- 入れ子 clone だった `v3/` は，未 push commit・ローカル限定 branch / tag・stash がゼロであることを確認してから削除し，worktree に置き換えた．
