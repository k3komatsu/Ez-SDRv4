# Phase 2 実装とレビューのプロンプト

Phase 2 を実装する AI エージェントと，途中のレビューを行う AI に渡すプロンプト．計画書・仕様書が英語なのでプロンプトも英語．報告は日本語で返すよう指示してある．各 `.txt` はそのまま貼り付けられる（例：`pbcopy < plan/phase2/prompts/01-implement-steps-1-8.txt`）．

## 使う順番

| # | ファイル | 誰に | 範囲 | 止まる所 |
|---|---|---|---|---|
| 1 | [01-implement-steps-1-8.txt](01-implement-steps-1-8.txt) | 実装担当 | 手順 1–8（Kernel 修正の適用と coordinator） | 手順 8 の後 |
| 2 | [02-review-k.txt](02-review-k.txt) | レビュー担当（Opus，AGENTS.md §8） | Review K：`96976c5` 以降の Kernel の差分 | レビュー結果の報告 |
| 3 | [03-implement-steps-9-15.txt](03-implement-steps-9-15.txt) | 実装担当 | 手順 9–15（新しい 9 crate と受け入れテスト） | 手順 15 の後 |
| 4 | [04-review-m.txt](04-review-m.txt) | レビュー担当（Opus） | Review M：Module と受け入れテスト | レビュー結果の報告 |
| 5 | [05-implement-step-16.txt](05-implement-step-16.txt) | 実装担当 | 手順 16（exit 表，marker，handoff） | Gate X の前 |

計画書 `20-implementation-plan.md` §0.4 と §9 のとおり，実装担当は手順 8 と手順 15 の後で必ず止まる．commit は owner がする．

## レビューの後

1. 指摘を 1 件ずつ判定し，判定を [00-overview.md](../00-overview.md) §11 に記録する（OV-5：判定なしに黙って反映しない）．
2. 受理した指摘を [fix-findings.txt](fix-findings.txt) の末尾に貼り，実装担当に渡す．実装担当は直すたびに mutation check と 3 コマンドをやり直し，`implementation-notes.md` に記録する．
3. P0 が出たら，修正後に範囲を絞ったレビューをもう 1 回だけ回す．計画段階のレビュー（[../reviews/planning-reviews.md](../reviews/planning-reviews.md)）では，直した一文が別の文書と食い違う形の欠陥が毎回出た．
4. 次の実装プロンプト（3 または 5）へ進む．

## 渡す前の確認

- 実装環境に Rust `1.85.0` と `stable` の両方が入っていること．
- 作業はこの作業ツリーで行うこと．`.cargo/config.toml`（ビルド出力の置き場所）は git 管理外なので，clone し直した環境には無い．
- 作業中は Google Drive の同期を止めておく（2026-09-24 に同期で追跡ファイルが消えたことがある，[handoff.md](../../../handoff.md) §1）．
