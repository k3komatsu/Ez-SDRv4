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

**なし**．Rust crate，Python client，スキーマ，wire protocol，いずれも未着手．`Cargo.toml` も存在しない．

## 4. 次にやること — Phase 1: Kernel semantic model

Vision §67 の順序．Phase 1 の入口は次の 3 つ：

1. **R13 の切り出し**．Vision に入り込んだ規範部分を `design/` の spec へ移す．対象は索引ヘッダに列挙済み：Stream Contract（§23），time model（§15），Session（§3），BindingProfile（§8），PrepareReport（§11），Manifest（§50）．ファイル名は rereview R13 案のとおり `design/stream-contract.md`, `design/time-model.md`, `design/session-model.md`, `design/binding-profile.md`（PrepareReport / Manifest も同様）．Vision 側の節は消さず「要約 + リンク」に戻す（audit の § 参照を生かすため）．
2. **Kernel の最小定義**．audit §13「Recommended Minimal Core」の tree が freeze 対象の一覧．audit §14.1 の項目 1–7 が P0．
3. **完了条件**．Vision §58 の acceptance tests #1–#16（Phase 1–6 で順に満たす）．UHD は Core + Mock が動くまで触らない（§59）．

型を書くときの拘束文は Vision §5（tiers），§15（time），§23（Stream Contract），§13（TimingEnvelope / Mock 強制），§3（Session），§8（composite resource，BindingProfile）．

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
