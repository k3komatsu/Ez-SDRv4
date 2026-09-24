# Ez-SDR v4 — Handoff (2026-09-24)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `git@github.com:k3komatsu/Ez-SDR.git` |
| `main` | v4（この worktree）．Phase 0 の設計文書に続き，Phase 1 の spec と `ezsdr-kernel` を実装・受理済み．Phase 1 は完了．Phase 2 は計画（spec 06–10 と実装計画）を Gate P で受理済み（2026-09-24）．次は実装（§4）．|
| `master` | v3（D + C++ UHD bridge + Python client）．tip `4a474e9` = tag `v3.0.28`．**GitHub の default branch のまま** |
| tags | 35 個（`v2.11`, `v3.0.0`–`v3.0.28`）．すべて v3 系 |
| 履歴の関係 | **無関係（unrelated）**．graft も merge もしていない．v4 は clean-sheet なので今後も繋がない |

ローカル：

- `/Users/komatsu/GoogleDrive/github/Ez-SDRv4` = `main` の worktree（メイン）．
- `…/Ez-SDRv4/v3` = `master` の **git worktree**（入れ子 clone ではない）．`.gitignore` で除外．
- 設計文書が v3 のパス 21 件を証拠として引用しているので `v3/` は消さない（`design/` と Vision に加えて `plan/phase1/` からも引用がある）．確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- `v3/` を消してしまった場合：`git worktree prune && git worktree add v3 master`
- **Google Drive の同期で追跡ファイルが消えることがある**．2026-09-24 に `crates/ezsdr-kernel/tests/` と `schemas/`（61 ファイル）が作業ツリーから消え，`git restore crates/ezsdr-kernel/tests schemas` で戻した．作業を始める前に `git status --short` に ` D` 行がないことを確かめる．Drive の同期中に戻すと競合コピー（`… (1).…`）が増えるので，同期を止めてから戻す．

## 2. 設計文書の状態 — Phase 0 完了

- 単一の設計ソース = **Vision**：索引 [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) + [design/vision/](design/vision) の 11 part（§1–§68，番号は不変）．
- [design/v4-vision-audit.md](design/v4-vision-audit.md)（Findings 1–34，判定 READY WITH REQUIRED CHANGES）→ 全項目を Vision に反映済み．
- [design/v4-vision-rereview.md](design/v4-vision-rereview.md)（Findings R1–R22，判定 READY）→ 全項目反映済み．R13「規範部分の spec 化」は当初11ファイル分割で暫定対応したが，Phase 1 Step 5（D109，2026-09-23）で規範部分を accepted specs へ移し，完了した．
- 旧 CMA は退役．[design/archive/](design/archive) に保管（audit / rereview の `CMA §N` 引用のためだけに残す）．編集しない．
- Vision の改訂履歴は索引ファイル末尾の表（8 pass：7件は2026-09-21，最終の1件は2026-09-23）．

## 3. 実装の状態

Phase 1 の Kernel crate は **実装済み**（Step 4 完了）．

| | |
|---|---|
| workspace | ルートの `Cargo.toml`（`resolver = "3"`，edition 2024，`rust-version = "1.85"`）．`Cargo.lock` は commit 対象 |
| crate | `crates/ezsdr-kernel` version `4.0.0-alpha.1`．`#![forbid(unsafe_code)]`，`#![warn(missing_docs)]` |
| module | `id time contract stream hash module_api spec binding plan event policy run session manifest schema`．`plan` は `coercion` `compile` `graph` `islands` `links` `matching` `prepare` `validation` の private submodule に分割済み |
| 直接依存 | `serde` `serde_json` `schemars` `sha2` の4つだけ（exit criterion 6）．解決後のツリーは `Cargo.lock` で28 package（crate 自身を含む） |
| test | 353 件．`spec_binding`(102) `stream_contract`(69) `run_session`(76) `time_model`(49) `module_api`(27) `hashing`(9) `kernel_surface`(15) `schema_freeze`(4) `event_hotpath`(1) `lib`(1) |
| toolchain | Rust `1.85.0` / stable (`1.98.1`) の両方で 353 passed（2026-09-24）．`cargo +stable clippy --workspace --all-targets -- -D warnings` も通過 |
| schemas | `schemas/` に47個の JSON Schema 2020-12 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`NEW:` 件数が OV-23b の Kernel 成長指標．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |

Python client，wire protocol，MockRadio，Simulation Engine は未着手（Phase 2 以降）．

## 4. Phase の状態 — Phase 1 完了，Phase 2 は計画を受理済み（実装前）

### Phase 1 — specs 受理・Kernel 実装済み

Vision §67 の Phase 1．**受理済み**（2026-09-23，Step 5 完了）．5つの spec は `design/0N-*.md` に移り，横断決定・OV 規則・決定ログは [plan/phase1/00-overview.md](plan/phase1/00-overview.md) に残る．

| spec | rule ID | 状態 |
|---|---|---|
| [00-overview.md](plan/phase1/00-overview.md) | OV-1..23b（副番含め27） | criterion 2 の per-rule 表を含め更新済み |
| [01-time-model.md](design/01-time-model.md) | TM-1..21（副番含め31） | 受理済み，実装済み |
| [02-stream-contract.md](design/02-stream-contract.md) | SC-1..32（副番含め47） | 受理済み，実装済み |
| [03-spec-and-binding.md](design/03-spec-and-binding.md) | SB-1..49（副番含め60） | 受理済み，実装済み．SB-22 を SB-22a〜h と表 SB-T0〜T4 に分割（D95・D96） |
| [04-run-and-session.md](design/04-run-and-session.md) | RS-1..52（副番含め60） | 受理済み，実装済み．RS-25a を追加（D103） |
| [05-module-api.md](design/05-module-api.md) | MA-1..46（副番含め52） | 受理済み，実装済み．MA-16a を追加（D100） |

6文書で277ルール，欠番と未解決参照なし．撤回7件（SB-25a, SB-28, SB-32, RS-32a, RS-37, MA-4, MA-43 — OV-1 に従い番号は保持）．

受理までの経緯（gate ごとの敵対的レビュー，crate へのレビュー25巡，D51〜D109 の適用）と，Step 4 前後に決めたことの理由は [plan/phase1/history.md](plan/phase1/history.md) に移した．決定そのものの正本は [00-overview.md](plan/phase1/00-overview.md) §11．

exit criteria（§13）の達成状況（2026-09-23 時点）：

| # | 条件 | 状態 |
|---|---|---|
| 1 | 6文書の受理と §11 の全 verdict | **達成**．verdict は D1–D109．6文書を受理し，spec を `design/` へ移した（D109） |
| 2 | 全 rule に ID と OV-3 disposition | **277ルールに277行を用意し，GAP と未割当を解消**．内訳は default 207・process 22・producer 13・forward 11・withdrawn 7・consumer 1・分割 16（default+forward 12，default+producer 3，forward+producer 1）．`UNCERTAIN:` は D108 でゼロ **（達成）** |
| 3 | MSRV と stable で `cargo test` 通過，`#[ignore]` なし，pipeline が端から端まで動く | **達成**（Step 5 受理時点の記録：2026-09-23，1.85.0 / stable 1.98.1 ともに345 passed，`#[ignore]` なし）．end-to-end pipeline ケースも通過 |
| 4 | `schemas/`，`schema_freeze`，`SCHEMA_CHANGELOG.md` | **達成**．D51–D68・D95–D103・D104 の schema 変更を反映済み（`pattern` は無し） |
| 5 | `kernel_surface` 通過，`NEW:` 件数の記録 | **達成**．`109 NEW / 281 public items`（2026-09-23，`plan::check_sink_links` を D99 で削除） |
| 6 | 直接依存が §8 の4 crate ちょうど | **達成** |
| 7 | §12 の移動後に `design/` と `plan/phase1/` の全リンクが解決 | **達成**（238リンク，切れ0．`v3/` パス21件も全て存在） |


### Phase 1 から持ち越す教訓

レビューで繰り返し出た欠陥型．詳細は [history.md](plan/phase1/history.md) §1．

- **正しい関数を誰も呼んでいない**．pipeline の段は関数ではなく段として繋ぎ，呼び忘れを構造的に不可能にする．
- **ルールを初めて生かすと，意図しないものを拒否する**．判定を書く前に，そのデータが判定に必要な情報を持っているかを確かめる（`collect_prepare` で4回）．
- **判定を適用する作業そのものが欠陥を入れる**．修正を書いたら，その修正が参照する条文をもう一度読む．
- **置換せず追加してしまう**．判定を適用したら，置換対象が消えたことを grep で確かめる．
- **新しい拒否は1件ずつ無効化して，テストが落ちることを確かめる**（mutation）．落ちないものはテストを足すか，死にコードとして消す．
- **gate は列挙では閉じない**．`kernel_surface` は構文の新しい形で何度も抜けられた．仕組み（`use … as` や一覧外の `macro_rules!`）を禁止し，残りは process obligation として名指す（D100）．

### Phase 2 — 計画を受理済み（Gate P，2026-09-24），実装前

Vision §67 の Phase 2（Radio Model + Simulation Engine + MockRadio）．文書はすべて [plan/phase2/](plan/phase2) にあり，2026-09-24 に Gate P で受理された（[00-overview.md](plan/phase2/00-overview.md) §9，§11）．spec 06–10 は実装者を拘束し，20-implementation-plan.md が作業順序．

| 文書 | 中身 |
|---|---|
| [00-overview.md](plan/phase2/00-overview.md) | 範囲，横断決定 Y1–Y14，10 crate の構成と許される依存，運用規則 PO-1–PO-12，Vision §58/§61 と Phase 1 marker の対応表，gate，exit criteria，決定ログ（§11，Gate P で記入） |
| [06-kernel-coordinator.md](plan/phase2/06-kernel-coordinator.md) | Phase 1 spec への修正 KA-1–KA-22，Run coordinator KC-1–KC-45，update class UC-1–UC-6 |
| [07-radio-model.md](plan/phase2/07-radio-model.md) / [08-simulation.md](plan/phase2/08-simulation.md) / [09-mock-radio.md](plan/phase2/09-mock-radio.md) / [10-host-data-path.md](plan/phase2/10-host-data-path.md) | `radio` Vocabulary（RM），`sim` Vocabulary と Simulation Engine（SE），MockRadio（MR），host メモリ・Link・`sink` Vocabulary・capture Sink（HD） |
| [20-implementation-plan.md](plan/phase2/20-implementation-plan.md) | 実装者（安価なモデルを想定）向けの 16 手順．各手順にファイル・シグネチャ・疑似コード・テスト表・mutation check・完了条件．付録 A = patch 適用後の Kernel API，付録 B = 全規則→手順→テストの対応（テストの無い規則 0） |
| [patches/01-kernel-amendments.patch](plan/phase2/patches/01-kernel-amendments.patch) | KA のうち coordinator 以外のコード（28 ファイル）．`96976c5` に `git apply` でき，1.85.0 / stable ともに 361 passed，clippy 通過．新テスト 8 件は各規則を無効化すると落ちることを確認済み．**まだ当てていない**（実装手順 1） |
| [reviews/planning-reviews.md](plan/phase2/reviews/planning-reviews.md) | Gate P 前の Opus 敵対的レビュー 3 巡（指摘 56 → 34 → 18）と各指摘の判定．全件反映済み |

Phase 1 の Kernel には，最初の本物の Module が必ず踏む穴があった（例：`PrepareContext` のハンドルが借用で，Module が `prepare` 後にイベントも時刻も扱えない）．KA-1–KA-22 がそれを埋める．

**Gate P の判定**：00-overview.md §11 の決定ログに，Y1–Y14 と各 spec の決定表（K1–K12，R1–R9，S1–S7，M1–M10，H1–H8）の verdict を記入済み．2026-09-24 に owner が**全件を推奨どおり受理**した（§11 に記録）．特に確認が要った次の 4 点は，それに先立って個別に受理している：

1. **故障注入 3 種を Phase 2 に入れる**（KA-22 で受理済み spec 04 の「fault injection は Phase 4」を改める）．覆すなら SE-3–SE-5，MR-20–MR-22，`v58_04`–`v58_06` を外し，SC-18 などの marker は forward のまま．
2. **x310-like の転送上限 1.0 GB/s/方向**（MR-3，INFERRED）．10 GbE の line rate 1.25 GB/s から Y13 で厳しい側に丸めた．Phase 8 の実測で置き換える前提．
3. **capture Sink に「全サンプル記録」モードを設けない**（HD-10，H8）．記録は `N` サンプル指定のみ．
4. **x310-like では `ezsdr.time.start_lead_ns ≥ 2 s` が必須**（MR-11）．足りない開始は拒否（遅れて開始して `LATE` を付ける案は Y13 で不採用）．

### 次にすること

1. 実装担当とレビュー担当に渡すプロンプトは [plan/phase2/prompts/](plan/phase2/prompts)（使う順番は README）．実装者は [20-implementation-plan.md](plan/phase2/20-implementation-plan.md) の §0 を読んでから，手順 1（patch 適用）から順に進める．実装者は commit しない（手順ごとの commit 件名案は `plan/phase2/implementation-notes.md` に残す）．
2. 手順 8 の後（Review K：Kernel の差分）と手順 15 の後（Review M：Module と受け入れテスト）で止まり，owner が敵対的レビューを 1 本ずつ回す．指摘は 00-overview.md §11 に verdict つきで記録する（OV-5）．
3. Phase 2 の exit（Gate X）で spec 06–10 を `design/` へ移し，各 spec の「Vision issues found」を owner の承認のもとで Vision に反映する．

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
