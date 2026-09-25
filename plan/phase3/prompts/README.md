# Phase 3 実装とレビューのプロンプト

Phase 3 を実装する AI エージェントと，途中のレビューを行う AI に渡すプロンプト．計画書・仕様書が英語なのでプロンプトも英語で，報告は日本語で返すよう指示してある．各 `.txt` はそのまま貼り付けられる（例：`pbcopy < plan/phase3/prompts/01-implement-steps-0-6.txt`）．

Phase 3 の実装担当はコードを書かない．`plan/phase3/patches/` の 5 枚のパッチを順に当て，各手順の検査を実行し，結果を記録する（`00-overview.md` Z4，GV-3）．パッチは Gate P の前に scratch copy で検証済みである．`612b9eb` の clone に順に当てると，1.85.0 と stable の両方で 592 tests が通り，Clippy も通り，83 個の mutation がすべて落ちる．計画段階の敵対的レビューの指摘と判定は [../reviews/planning-reviews.md](../reviews/planning-reviews.md) にある．

## 使う順番

| # | ファイル | 誰に | 範囲 | 止まる所 |
|---|---|---|---|---|
| 1 | [01-implement-steps-0-6.txt](01-implement-steps-0-6.txt) | 実装担当 | 手順 0–6（base の確認，パッチ 5 枚，mutation 83 個） | 手順 6 の後 |
| 2 | [02-review-c.txt](02-review-c.txt) | レビュー担当（Opus，AGENTS.md §8） | Review C：`612b9eb` からの差分と spec 11・12．**実行して確かめる**動的レビュー | レビュー結果の報告 |
| 3 | [03-implement-step-7.txt](03-implement-step-7.txt) | 実装担当 | 手順 7（exit 表，Vision issues，handoff） | Gate X の前 |

## レビューの後

1. 指摘を 1 件ずつ判定し，判定を [00-overview.md](../00-overview.md) §11 に記録する（OV-5：判定なしに黙って反映しない）．
2. 受理した指摘を [fix-findings.txt](fix-findings.txt) の末尾に貼り，実装担当に渡す．パッチ適用後の修正はリポジトリで直接行い，修正ごとに mutation と 3 コマンドをやり直して `implementation-notes.md` に記録する．パッチ自体は Gate P で受理したものの記録として残し，書き換えない．
3. P0 が出たら，修正後に範囲を絞ったレビューをもう 1 回だけ回す．
4. 手順 7 のプロンプトへ進む．

## 渡す前の確認

- 実装環境に Rust `1.85.0` と `stable` の両方が入っていること．`python3`（標準ライブラリのみ）が使えること（`plan/phase3/tools/mutate.py`）．
- 作業はこの作業ツリーで行うこと．`.cargo/config.toml`（ビルド出力の置き場所）は git 管理外なので，clone し直した環境には無い．
- 作業中は Google Drive の同期を止めておく（2026-09-24 に同期で追跡ファイルが消えたことがある，[handoff.md](../../../handoff.md) §1）．
