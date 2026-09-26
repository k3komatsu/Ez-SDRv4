# Ez-SDR v4 — Handoff (2026-09-27)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `origin` = `git@github.com:k3komatsu/Ez-SDRv4.git`（https://github.com/k3komatsu/Ez-SDRv4，default branch `main`）．2026-09-26 に v4 を新リポジトリへ分離した．v3 は https://github.com/k3komatsu/Ez-SDR（`master` のみ，v3 の tags もそちら）に残る |
| `main` | v4（この worktree）．**Phase 0–6 の受理完了**（Phase 6 Step X まで push 済み）．Phase 2 Gate X を owner が2026-09-25に受理し，spec 06–10 を `design/` へ移動，Vision issue 15件を適用（Step X 完了）．Step 16/Gate X/Step X は `bf3b1bf` で commit・push 済み．2026-09-25 の動的レビューで見つかったテスト欠落2件を追加テストで塞いだ（[implementation-notes.md](plan/phase2/implementation-notes.md) 末尾）．Opus `PASS_WITH_RISK` の残余リスクは §4 と `plan/phase2/00-overview.md` §11 に記録．**Phase 3 は 2026-09-26 に Gate P 受理・実装・Review C・Gate X 受理・Step X 完了**（spec 11 を `design/` へ移動，Vision issue 8件を適用）．**Phase 4 は同日に計画・実装・Review D/E・Gate X 受理（すべて推奨どおり）・Step X 完了**．**Phase 5 は同日に計画・実装・Review F/G・Gate X 受理（すべて推奨どおり）・Step X 完了**（spec 14 を `design/` へ移動，Vision issue 3 件を適用）．**Phase 6 は 2026-09-26〜27 に計画・実装・Review H/I・Gate X 受理（すべて推奨どおり）・Step X 完了**（spec 16 を `design/` へ移動，Vision issue 3 件を適用）．§4 参照．|
| `master` | v3（D + C++ UHD bridge + Python client）．tip `4a474e9` = tag `v3.0.28`．この clone ではローカルのみ（upstream なし）．GitHub 上の置き場は `k3komatsu/Ez-SDR` |
| `spike/uhd` | **使い捨ての UHD spike**（2026-09-26，push 済み）．`main` から分岐し，Kernel 1 行の変更（K1）と `spike/uhd/` crate を載せる．**`main` へ merge しない**．成果は発見の記録だけで，`main` の [plan/spikes/2026-09-26-uhd.md](plan/spikes/2026-09-26-uhd.md) に移した．実機検証は保留．§4「UHD spike」参照 |
| `gh-pages` | GitHub Pages 用の orphan branch（`main`・`master` と履歴を共有しない）．2026-09-26 に `24229a9` で作成し，プロジェクト概要サイト（`index.html`，`showreel/`）を載せた．最新は `af488c9`「update Pages status after Phase 4」で push 済み．**Pages は公開済み**：https://k3komatsu.github.io/Ez-SDRv4/（branch `gh-pages` の root から配信，2026-09-26 に `gh api repos/k3komatsu/Ez-SDRv4/pages` で `status: built` を確認）．**サイトの状態表示は Phase 4 時点のまま**（「Phase 5 の計画は未作成」などと書いてある）で，Phase 5 完了を反映するには更新が要る．AGENTS.md §1 のとおり，gh-pages は owner の依頼があるときだけ変更する |
| tags | ローカルに 35 個（`v2.11`, `v3.0.0`–`v3.0.28`）．すべて v3 系で，`Ez-SDR` 側にある．`Ez-SDRv4` には tag なし．**`git push --tags` / `--all` をしない**（v3 の tag と `master` が v4 リポジトリに上がる） |
| 履歴の関係 | **無関係（unrelated）**．graft も merge もしていない．v4 は clean-sheet なので今後も繋がない |

ローカル：

- `/Users/komatsu/GoogleDrive/github/Ez-SDRv4` = `main` の worktree（メイン）．
- `…/Ez-SDRv4/v3` = `master` の **git worktree**（入れ子 clone ではない）．`.gitignore` で除外．
- `/Users/komatsu/orca/workspaces/Ez-SDRv4/gh-pages` = `gh-pages` の git worktree（Orca の workspace．作業ツリーの外）．
- 設計文書が v3 のパス 22 件を証拠として引用しているので `v3/` は消さない（`design/` と Vision に加えて `plan/phase1/` からも引用がある）．確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- `v3/` を消してしまった場合：`git worktree prune && git worktree add v3 master`
- v3 の更新を取り込むときの取得元は `origin` ではなく `k3komatsu/Ez-SDR`（例：`git fetch git@github.com:k3komatsu/Ez-SDR.git master`）．
- **Google Drive の同期で追跡ファイルが消えることがある**．2026-09-24 に `crates/ezsdr-kernel/tests/` と `schemas/`（61 ファイル）が作業ツリーから消え，`git restore crates/ezsdr-kernel/tests schemas` で戻した．作業を始める前に `git status --short` に ` D` 行がないことを確かめる．Drive の同期中に戻すと競合コピー（`… (1).…`）が増えるので，同期を止めてから戻す．

## 2. 設計文書の状態 — Phase 0・1・2・3・4・5・6 完了

- 単一の設計ソース = **Vision**：索引 [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) + [design/vision/](design/vision) の 11 part（§1–§68，番号は不変）．
- [design/v4-vision-audit.md](design/v4-vision-audit.md)（Findings 1–34，判定 READY WITH REQUIRED CHANGES）→ 全項目を Vision に反映済み．
- [design/v4-vision-rereview.md](design/v4-vision-rereview.md)（Findings R1–R22，判定 READY）→ 全項目反映済み．R13「規範部分の spec 化」は当初11ファイル分割で暫定対応したが，Phase 1 Step 5（D109，2026-09-23）で Phase 1 specs へ，Phase 2 Gate X（2026-09-25）で specs 06–10 へ反映し，完了した．
- accepted specs は Phase 1 の [design/01-time-model.md](design/01-time-model.md)〜[design/05-module-api.md](design/05-module-api.md) と Phase 2 の [design/06-kernel-coordinator.md](design/06-kernel-coordinator.md)〜[design/10-host-data-path.md](design/10-host-data-path.md)，Phase 3 の [design/11-simulation-channel.md](design/11-simulation-channel.md)（spec 12 の修正は design/04–09 に適用済み），Phase 5 の [design/14-native-executor.md](design/14-native-executor.md)（spec 15 の修正は design/03–06 に適用済み）．各 Phase の決定ログ・実装計画・rule exit evidence は [plan/phase1/](plan/phase1)〜[plan/phase5/](plan/phase5) に残る．Phase 6 の [design/16-easy-api.md](design/16-easy-api.md)（server・protocol・Python）．spec 17 の修正は `design/03・04・06・10` に適用済みで，記録は [plan/phase6/17-amendments.md](plan/phase6/17-amendments.md)．
- 旧 CMA は退役．[design/archive/](design/archive) に保管（audit / rereview の `CMA §N` 引用のためだけに残す）．編集しない．
- Vision の改訂履歴は索引ファイル末尾の表（Phase 0/1 の8 passに加え，Phase 2 Gate X の適用を2026-09-25に，Phase 3・4・5 の Gate X の適用を2026-09-26に記録）．Phase 4 の spec 13 は amendment だけなので `design/` へ移すものはなく，本文は実装 commit で `design/02・05・07・08・09・10・11` に入っている．

## 3. 実装の状態

現在（2026-09-27，Phase 6 Review I の修正後）の状態：

| | |
|---|---|
| workspace | ルートの `Cargo.toml`（`resolver = "3"`，edition 2024，`rust-version = "1.85"`）．member は 12 crate：`ezsdr-kernel`，Vocabulary の `ezsdr-radio` `ezsdr-sim` `ezsdr-sink`，補助の `ezsdr-hostmem`，Module の `ezsdr-sim-engine` `ezsdr-mock-radio` `ezsdr-link-host` `ezsdr-sink-capture` `ezsdr-exec-native`，frontend（Module ではない Runtime）の `ezsdr-server`，テストの `ezsdr-acceptance`．`Cargo.lock` は commit 対象（external 27 package + workspace member 12）．Python パッケージ `python/ezsdr`（Python ≥ 3.9 + numpy） |
| Kernel crate | `crates/ezsdr-kernel` version `4.0.0-alpha.1`．`#![forbid(unsafe_code)]`，`#![warn(missing_docs)]` |
| module | `id time contract coordinator stream hash module_api spec binding plan event policy run session manifest schema`．`plan` は `coercion` `compile` `graph` `islands` `links` `matching` `prepare` `validation` の private submodule に分割済み |
| 直接依存 | `serde` `serde_json` `schemars` `sha2` の4つだけ（Phase 1 exit criterion 6）．Module crate は他の Module crate に依存しない（MA-3，`ma_03_no_module_crate_depends_on_another`） |
| test | workspace 684 件（1.85.0 / stable とも 0 failed，0 ignored）＋ Python 21 件（3.9 / 3.13）．Kernel package の内訳は Phase 5 時点で 456 件：`coordinator`(90) `spec_binding`(108) `stream_contract`(69) `run_session`(76) `time_model`(52) `module_api`(30) `run_doubles`(1) `hashing`(9) `kernel_surface`(15) `schema_freeze`(4) `event_hotpath`(1) `lib`(1) |
| toolchain | Rust `1.85.0` (`4d91de4e4`, 2025-02-17)（MSRV）/ stable `1.98.1` (`48a229cea`, 2026-09-01)．`cargo +stable clippy --workspace --all-targets -- -D warnings` clean |
| schemas | Kernel の `schemas/` 直下に 47 個の JSON Schema 2020-12，Vocabulary の `schemas/{radio,sim,sink}/` に 13 個，server の `schemas/server/` に 2 個，合計 62 個 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`cargo +stable test -p ezsdr-kernel --test kernel_surface ov_23b -- --nocapture` は **116 NEW / 292 public items**（Phase 3 の KB-1 で 1 つ増えてから変わっていない）．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |
| mutation | 各 Phase の `plan/phaseN/tools/mutate.py` と `mutations.json`．Phase 5 は 29/29，Phase 6 は 70/70 killed（Phase 6 の tool は Python の carrier も走らせる）．コピーの作り方は AGENTS.md §7 |

以下は Phase ごとの記録．

Phase 2 の workspace tests（2026-09-25，動的レビュー後の追加テスト込みでRust 1.85.0 / stable の両方で各550 passed）：

| crate | tests |
|---|---:|
| `ezsdr-kernel` | 443 |
| `ezsdr-radio` | 10 |
| `ezsdr-sim` | 6 |
| `ezsdr-sim-engine` | 7 |
| `ezsdr-hostmem` | 2 |
| `ezsdr-link-host` | 2 |
| `ezsdr-sink` | 2 |
| `ezsdr-sink-capture` | 15 |
| `ezsdr-mock-radio` | 34 |
| `ezsdr-acceptance` | 29 |
| **合計** | **550** |

Python client と wire protocol は未着手．Phase 2 はGate X受理・Step X完了まで済み（2026-09-25）．実装・review・exit artifacts・受理処理は `bf3b1bf` までに commit 済み．

Phase 3 の workspace tests（2026-09-26，Review C と Gate X の修正込みで Rust 1.85.0 / stable の両方で 604 passed）：

| crate | tests |
|---|---:|
| `ezsdr-kernel` | 447 |
| `ezsdr-radio` | 10 |
| `ezsdr-sim` | 17 |
| `ezsdr-sim-engine` | 7 |
| `ezsdr-hostmem` | 2 |
| `ezsdr-link-host` | 2 |
| `ezsdr-sink` | 2 |
| `ezsdr-sink-capture` | 15 |
| `ezsdr-mock-radio` | 65 |
| `ezsdr-acceptance` | 37 |
| **合計** | **604** |

Phase 4（2026-09-26，Review D/E の修正込み）：1.85.0 / stable とも 626 passed（`ezsdr-kernel` 452，`ezsdr-radio` 12，`ezsdr-sink-capture` 24，`ezsdr-mock-radio` 66，`ezsdr-acceptance` 41，ほかは Phase 3 と同じ）．Kernel の public item は増えていない．

Phase 6（2026-09-27，Review H/I の修正込み）：1.85.0 / stable とも 684 passed，Python 21 passed（3.9.6 / 3.13.15）．Kernel の public item は増えていない（`RunHandle` に 3 メソッド，116 NEW / 292）．Kernel schema の変更なし．

Phase 5（2026-09-26，Review F/G の修正込み）：1.85.0 / stable とも 644 passed（`ezsdr-kernel` 456，新 crate `ezsdr-exec-native` 8，`ezsdr-acceptance` 47，ほかは Phase 4 と同じ）．Kernel の public item は増えていない（116 NEW / 292）．Kernel schema の変更は `experiment_spec` の `inputs` と，`ComponentKind` の `reactor` の説明文だけ．workspace member は 11 個になった．

Phase 3 の時点：`kernel_surface` は `116 NEW / 292 public items`（KB-1 の `module_api::InputStore` の 1 つだけ増えた）．mutation は Appendix C 83/83 + Review C の fix-check 15/15 + Gate X の fix-check 4/4 killed．

## 4. Phase の状態 — Phase 1・2・3・4・5・6 完了（Gate X受理）

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

### Phase 2 — Gate X受理・Step X完了

Vision §67 の Phase 2（Radio Model + Simulation Engine + MockRadio）．2026-09-24に Gate P，2026-09-25に Gate X を受理済み（[00-overview.md](plan/phase2/00-overview.md) §9，§11）．accepted specs 06–10 は [design/](design/) に移し，Phase 2 の決定・手順・exit evidence は [plan/phase2/](plan/phase2) に残した．Steps 1–15 は commit `6bf67fb` までに実装済み（[implementation-notes.md](plan/phase2/implementation-notes.md)）．Review K/M と Gate X の Opus review は完了．Gate X は `PASS_WITH_RISK`（blockerなし）を受けて owner が受理した．named risks は MA-8 timeout enforcement 未試験，Kernel 経由 Session `Stop(sink/rec)` test なし，reviewer は静的レビューのみ（2026-09-25 の動的レビューで exit criteria 8項目を実測確認し，Session 経由 `Stop(sink/rec)` も動作確認した）．Phase 2 の Vision issue 15件を適用し，stable/MSRV の workspace 547 tests と stable Clippy は適用前の最終コードで通過済み．残る test ceilings は [implementation-notes.md](plan/phase2/implementation-notes.md) と exit tables に記録した．

| 文書 | 中身 |
|---|---|
| [00-overview.md](plan/phase2/00-overview.md) | 範囲，横断決定 Y1–Y14，10 crate の構成と許される依存，運用規則 PO-1–PO-12，Vision §58/§61 と Phase 1 marker の対応表，gate，exit criteria，決定ログ（§11，Gate P で記入） |
| [06-kernel-coordinator.md](design/06-kernel-coordinator.md) | Phase 1 spec への修正 KA-1–KA-22，Run coordinator KC-1–KC-45，update class UC-1–UC-6 |
| [07-radio-model.md](design/07-radio-model.md) / [08-simulation.md](design/08-simulation.md) / [09-mock-radio.md](design/09-mock-radio.md) / [10-host-data-path.md](design/10-host-data-path.md) | `radio` Vocabulary（RM），`sim` Vocabulary と Simulation Engine（SE），MockRadio（MR），host メモリ・Link・`sink` Vocabulary・capture Sink（HD） |
| [20-implementation-plan.md](plan/phase2/20-implementation-plan.md) | 実装者（安価なモデルを想定）向けの 16 手順．各手順にファイル・シグネチャ・疑似コード・テスト表・mutation check・完了条件．付録 A = patch 適用後の Kernel API，付録 B = 全規則→手順→テストの対応（テストの無い規則 0） |
| [patches/01-kernel-amendments.patch](plan/phase2/patches/01-kernel-amendments.patch) | KA のうち coordinator 以外のコード（28 ファイル）．実装手順 1 で `96976c5` に適用済み．同パッチの新テスト 8 件は各規則を無効化すると落ちることを確認済み |
| [reviews/planning-reviews.md](plan/phase2/reviews/planning-reviews.md) | Gate P 前の Opus 敵対的レビュー 3 巡（指摘 56 → 34 → 18）と各指摘の判定．全件反映済み |

Phase 1 の Kernel には，最初の本物の Module が必ず踏む穴があった（例：`PrepareContext` のハンドルが借用で，Module が `prepare` 後にイベントも時刻も扱えない）．KA-1–KA-22 がそれを埋める．

**Gate P の判定**：00-overview.md §11 の決定ログに，Y1–Y14 と各 spec の決定表（K1–K12，R1–R9，S1–S7，M1–M10，H1–H8）の verdict を記入済み．2026-09-24 に owner が**全件を推奨どおり受理**した（§11 に記録）．特に確認が要った次の 4 点は，それに先立って個別に受理している：

1. **故障注入 3 種を Phase 2 に入れる**（KA-22 で受理済み spec 04 の「fault injection は Phase 4」を改める）．覆すなら SE-3–SE-5，MR-20–MR-22，`v58_04`–`v58_06` を外し，SC-18 などの marker は forward のまま．
2. **x310-like の転送上限 1.0 GB/s/方向**（MR-3，INFERRED）．10 GbE の line rate 1.25 GB/s から Y13 で厳しい側に丸めた．Phase 8 の実測で置き換える前提．
3. **capture Sink に「全サンプル記録」モードを設けない**（HD-10，H8）．記録は `N` サンプル指定のみ．
4. **x310-like では `ezsdr.time.start_lead_ns ≥ 2 s` が必須**（MR-11）．足りない開始は拒否（遅れて開始して `LATE` を付ける案は Y13 で不採用）．

### Phase 3 — Gate X受理・Step X完了

Vision §67 の Phase 3（SimulationChannel + deterministic Runs）．計画は [plan/phase3/](plan/phase3) にあり，2026-09-26 に owner が Gate P を推奨どおり受理した（Z1–Z11，C1–C11，KB/VB，M11–M15，パッチ，計画レビューの判定；[00-overview.md](plan/phase3/00-overview.md) §11）．Phase 2 の教訓から，実装担当はコードを書き写さず，**検証済みのパッチ 5 枚を順に当てて検査するだけ**にした（[00-overview.md](plan/phase3/00-overview.md) Z4）．

**Steps 0–7，Review C，Gate X（2026-09-26，推奨どおり受理），Step X（spec 11 を [design/11-simulation-channel.md](design/11-simulation-channel.md) へ移動，Vision issue 8件を適用）まで完了．Review C の P2-1〜P2-9 は spec 12 VB-10 と KB-1 の KC-9 改正として反映済み．**

| 文書 | 中身 |
|---|---|
| [00-overview.md](plan/phase3/00-overview.md) | 範囲，Phase 2 に見つかった欠陥（KB-1，KB-2，VB-4–VB-6，VB-8），横断決定 Z1–Z11，運用規則 GV-1–GV-6，Vision §58 との対応，gate，exit criteria，決定ログ（§11，Gate P で記入） |
| [design/11-simulation-channel.md](design/11-simulation-channel.md) | spec 11（新規，Step X で `design/` へ移動）：`sim.channel`，medium，field，finality（CH-9），決定性．CH-1–CH-11，決定 C1–C11 |
| [12-amendments.md](plan/phase3/12-amendments.md) | spec 12：受理済み spec 04–09 への修正 KB-1，KB-2，**VB-1–VB-10**，新規則 RM-23，MR-31–MR-36，決定 M11–M15，Vision issue 4件 |
| [20-implementation-plan.md](plan/phase3/20-implementation-plan.md) | 手順 0–7 と X．付録 A（パッチごとの変更ファイル），B（全規則→テストの草稿），C（mutation 83 件） |
| [patches/](plan/phase3/patches) | 01-kernel，02-vocabularies，03-mock-radio，04-acceptance，05-design-text．**Gate P の記録として不変**（Review C の修正はここではなく repository に適用した） |
| [implementation-notes.md](plan/phase3/implementation-notes.md) | 実装担当の確認記録（Step 0–6 ごと）＋ `## Fixes after Review C`（11 条項の未固定を塞いだ詳細と P2-1 の実測） |
| [exit-review/](plan/phase3/exit-review) | 54 規則の OV-3 disposition（[README.md](plan/phase3/exit-review/README.md)，[11.md](plan/phase3/exit-review/11.md)，[12.md](plan/phase3/exit-review/12.md)）．Appendix B から 3 行をsupersede した．Step 7 後に 12.md のラベル誤り 4 行を直し，欠けていた 11 規則を追加 |
| [vision-issues.md](plan/phase3/vision-issues.md) | Vision issue 8件（spec 11 §7 の 4 件，spec 12 §3 の 4 件）．Step X で適用済み（Vision §§8, 13, 15, 16, 23, 25, 26, 57） |
| [reviews/planning-reviews.md](plan/phase3/reviews/planning-reviews.md) | Gate P 前の Opus 敵対的レビューと各指摘の判定（pass 1：P0 2・P1 6・P2 14，pass 2：P0 0・P1 3・P2 8，pass 3（範囲限定）：P0 1・P1 1・P2 6，pass 4（範囲限定）：P0 2・P1 2・P2 7，pass 5（範囲限定）：P0 0・P1 1・P2 4），P2 1件を理由付きで不採用とし他は全件反映済み |
| [reviews/review-c.md](plan/phase3/reviews/review-c.md) | Review C の記録（4 pass の判定，閉じた指摘，P2-1〜P2-9 の原文・検証・推奨と Gate X での受理）．原本は gitignore された `tmp/luna-primary-engineer/reviews/20260926-review-c-phase3-state/` |

**Review C（Opus，4 pass）**：`PASS_WITH_RISK`（初回）→ 11 件の test 修正後に `CHANGES_REQUIRED`（新規 **P0 = B1**）→ B1 修正後に `PASS_WITH_RISK` → 文本修正と P2-1 実測後に `PASS_WITH_RISK`（blockers なし）．B1 は「1 Run に 2 Mock があると `Manifest::write_section` の `insert` で片方の `ezsdr.radio.mock.*` section を黙って上書きする」．**MR-27 を per-instance 命名に改正**（owner 承認），Kernel は無変更．_closed_: B1，P1 2 件，未固定の規則条項 11 件，test gap 6 件，P2 nit 7 件．

**Gate X の判断（2026-09-26）**（`00-overview.md` §11）：

1. **P2-1〜P2-9：推奨どおり受理・反映済み**．P2-1・P2-3・P2-5 は ceiling（CH-9 / MR-30 / Z10，MR-18，MR-32 の exit 行），P2-2 は Kernel 修正（KC-9 が検証済み entry だけを store に残す，`kb_01_b_the_store_keeps_no_unverified_entry`），P2-4・P2-7 は文面修正（RM-16 / MR-25 の pending command，M13；`mr_25_a_stream_stop_keeps_the_pending_commands`），P2-8 は crate description，P2-6・P2-9 は対応なし．詳細は [review-c.md](plan/phase3/reviews/review-c.md) §3 と spec 12 VB-10
2. **spec 11・12 と Z1–Z11 / C1–C11 / M11–M15：推奨どおり受理**（exit criteria 1）．Review C の評価は Z・C・M11/12/14/15 とも「支持」，M13 は P2-7 の修正で解消

**Phase 4 へ持ち越すもの**：Module error で短絡した round の意味（P2-1 の ceiling，`step_until_quiescent`），TX block header の最初の消費者が直すべき cold change 順序（P2-3 の ceiling）．

### Phase 4 — Gate X受理・Step X完了

2026-09-26，owner の委任（「計画を立てて，実装したほうが速いなら実装まで」）で，計画と実装を 1 セッションで行った．Gate P は置かず，同日の Gate X で owner がすべて推奨どおり受理し（「すべて推奨で受理します」），Step X（Vision issue 2 件）も済んだ．正本は [plan/phase4/00-overview.md](plan/phase4/00-overview.md)（範囲・決定 Q1–Q8・§11 の owner 判断待ち），spec 13 は [plan/phase4/13-amendments.md](plan/phase4/13-amendments.md)，記録は [implementation-notes.md](plan/phase4/implementation-notes.md)．

| 項目 | 内容 |
|---|---|
| 範囲 | Phase 1–3 でほぼ済んでいたので，証拠のある穴 4 つだけ：KD-1（Module error で round が途中で切れる，Phase 3 P2-1），VC-1/VC-2（hot path を使う Module が一つもなかった．`radio` 1.2.0 RM-24 が `RX_OVERFLOW` の 17 byte 形式と decoder を持ち，MockRadio 1.2.0 MR-37 が hot path で出す．D51 どおり Kernel は decode しない），VC-3（SigMF：capture Sink 1.1.0 HD-15 が全 capture を `.sigmf-data` + `.sigmf-meta` にする，SC-32），Session `Stop(sink/rec)` の Kernel 経由 test |
| 範囲外 | 新しい fault kind（それぞれ機構を持つ Phase へ），CalibrationArtifact，artifact store（Phase 6），drift，MA-8 の Kernel 側強制（Phase 7），spike の K2/K5/K6/K8/K11（Phase 7），P2-3（Phase 10） |
| Kernel | `step_until_quiescent` と coordinator の `round` だけ．public item の増減なし（116 NEW / 292），Kernel schema 変更なし |
| レビュー | Review D（Claude Opus）と Review E（OpenCode の SpaceBunny，owner の依頼で Orca orchestration から並列に起動）の 2 本．どちらも CHANGES_REQUIRED．P0 1・P1 5（重複あり）を含む全指摘をテスト付きで修正し，owner 判断が要るものは §11 に残した．SpaceBunny の報告は `tmp/review-spacebunny/REPORT.md`（git 管理外） |
| 検証 | 1.85.0 / stable とも workspace 626 passed，Clippy clean，mutation は Appendix A の全件 killed，link 全解決 |
| Gate X の判断（§11） | **K3**（TX clock が arm 起点のため，`start_lead_ns` が sample 周期の倍数でないと burst が 1 sample 遅れる．Simulation でも再現し，ceiling test `k3_an_off_grid_start_lead_moves_a_burst_to_the_next_transmit_sample` で固定）は **Phase 7** で TX model と一緒に直す．drain 順（ring が control より先）と drop した hot body の mark は **ceiling**．P2-3 は **Phase 10**．Q1–Q8 と spec 13 は受理 |
| Step X | Vision issue 2 件を適用（§28/§51 の Normative 行，§29 の hot path の bytes の持ち主），索引に改訂履歴の行．[vision-issues.md](plan/phase4/vision-issues.md) |

### Phase 5 — Gate X受理・Step X完了

2026-09-26，owner の委任（「Phase4と同様に計画を立てて実装まで．実装後に Opus 5.5 でレビューし，修正して再レビュー．修正が小規模で低リスクなら再レビュー不要．これをループ」）で，Phase 4 と同じく計画と実装を 1 セッションで行った．Gate P は置いていない．正本は [plan/phase5/00-overview.md](plan/phase5/00-overview.md)（範囲・決定 R1–R11・§11 の決定ログ）．新しい Module の spec 14 は Step X で [design/14-native-executor.md](design/14-native-executor.md) へ移した．Kernel 修正の spec 15 は [plan/phase5/15-amendments.md](plan/phase5/15-amendments.md)，記録は [implementation-notes.md](plan/phase5/implementation-notes.md)．

| 項目 | 内容 |
|---|---|
| 進め方 | まず試作（Executor + responder + 2 台の MockRadio を実 coordinator で走らせる）で Kernel の穴を 3 つ実測し，その証拠から計画した |
| 範囲 | §58 の「minimal reactive test」そのもの：A が PING，B の Reactor が受信 sample から PING の先頭 sample を検出し，その時刻 + turnaround に timed PONG を返す．radio が lead を実機と同じ envelope で判定する |
| Kernel | KE-1（Spec の `inputs`：schedule に載らない入力を宣言でき，KC-9 が検証して store に入れる．これがないと Reactor は自分の波形を送れなかった），KE-2（Module の TxBurst の波形が Run の入力か admission で確かめる，RS-44a），KE-3（drain 中の `ezsdr.dispatch` / `ezsdr.run_state` 拒否は Module の失敗ではない．MA-14a），KE-4（RS-17 の reactive coerce は「しない」で確定，MA-30 の Action latency は Phase 10 へ），KE-5（MA-24・UC-1・UC-2：Action を適用しない Executor は拒否する）．public item の増減なし |
| 新 Module | `ezsdr.exec.native` 1.0.0（`crates/ezsdr-exec-native`）：compiled-in component を `impl` の kind・id・hash で読み込み（MA-19b を carry），component id 順に step，component が積んだ Action を submit．Action は適用しない（NX-7），Simulation のみ |
| Reactor | `crates/ezsdr-acceptance/src/responder.rs`（application logic．Kernel と Executor の ABI だけを使う，§58 #10） |
| carriers | `tests/reactive.rs`：§58 #9（PONG が 15 092 sample に届く），#11（turnaround 不足は drop / send_asap で TIME_ERROR．境界は 14 000 sample ちょうど），#3（seed で再現，radio の名前に依存しない），#12（block 長に依存しない，block 境界をまたぐ PING も 1 回だけ応答），KE-3（drain 中の判断は abort にならない） |
| 発見 | block-length jitter 下では block が最大 4 000 sample になり，5 ms の turnaround は late になりうる（デバイスの配送遅延を lead に含めるから．実機と同じ挙動で欠陥ではない）．jitter を使う carrier は 7 ms |
| レビュー | Review F（Opus）CHANGES_REQUIRED：P0 2（仕様文の矛盾：KC-9 の旧文，NX-7 と MA-24/UC-2）・P1 4・P2 11 → 全件修正．Review G（Opus，修正の再レビュー）CHANGES_REQUIRED：P0 0・P1 4・P2 9 → 全件修正．残りは文面と 2 テストだけなので，owner の規則どおり再レビューせず終了．記録は [reviews/](plan/phase5/reviews) |
| 検証 | 1.85.0 / stable とも 644 passed，Clippy clean，mutation 29/29 killed，link 全解決 |
| ビルド衛生 | 共有 target dir（`Ez-SDRv4-review`）では，mtime を保ったコピーや他コピーの後のビルドが古い（改変済みの）成果物を再利用する．AGENTS.md §7 に対策を書いた（`rsync -a --no-times --exclude .cargo`，ビルド前に touch，ビルドを交互に走らせない） |

**Gate X の判断**（2026-09-26，owner「すべて推奨で受理します」，[00-overview.md](plan/phase5/00-overview.md) §11）：

1. **Event edge**（Detector などから Reactor へイベントを渡す経路）は **Phase 10**，Reactor にイベントを渡す最初の Processor と一緒に作る．Kernel の `Endpoint::EventIn` / `EventOut` は handle を持たない未完成の variant なので，形は v4.0 凍結までに決める．
2. **Vision §22** は文言を修正した（Vision issue 2，Step X で適用）：Simulation はデバイスに渡った時点からの lead を判定し，component 自身の処理時間は数えない．budget を仮想時間で課す案は Phase 10 の budget / `PROCESSOR_DEADLINE_MISS` と一緒に．
3. **Action で変更できない component parameter**（Vision §27）：`ParamDecl.update_class` を **v4.0 凍結前に optional にする**（Phase 10 の最初の parameter 適用 Executor と一緒に）．それまでは NX-7 の拒否で表に出る．
4. R1–R11，N1–N6，spec 14・15 を受理．

**Step X**：spec 14 を [design/14-native-executor.md](design/14-native-executor.md) へ移し，`design/05` と crate の参照を書き換えた．Vision issue 3 件（§9 の Spec の形に `inputs`，§19 に spec 14 の Normative 行，§22 の文言）を適用し，索引に改訂履歴の行を加えた（[vision-issues.md](plan/phase5/vision-issues.md)）．

### Phase 6 — Gate X受理・Step X完了

2026-09-26〜27，owner の委任（「それではPhase4/5と同じ用にPhase6も設計と実装をしてください」）で，Phase 5 と同じく試作から計画し，実装し，Opus のレビューを回した．正本は [plan/phase6/00-overview.md](plan/phase6/00-overview.md)（範囲・決定 S1–S11・§11 の決定ログ），spec 16 は Step X で [design/16-easy-api.md](design/16-easy-api.md) へ移した（server・protocol・Python，EA-1…EA-19，決定 A1–A8），spec 17 は [plan/phase6/17-amendments.md](plan/phase6/17-amendments.md)（KF-1…KF-4，VD-1），記録は [implementation-notes.md](plan/phase6/implementation-notes.md)．

| 項目 | 内容 |
|---|---|
| 形 | **Python は別プロセス**（S1）：Rust の `ezsdr-server` が Module を組み込み，1 プロセス 1 Session を stdio 上の `ezsdr.protocol` 1（JSON のヘッダ行＋生のボディ）で動かす．プロトコルは Kernel の文書と `RunHandle` の呼び出しをそのまま運ぶ．Python パッケージ `ezsdr` は名前付け（`rx.frequency` → `radio.rx.frequency_hz`）と `capture` の組み立てだけで，Run の意味は持たない |
| Kernel | KF-1 `RunHandle::events`（配信済みイベントを Run 中に読む），KF-2 `wait_for`（Run の時間でイベントを待つ，Vision §15），KF-3 `run_child`（child Run を同期で実行，親の Lease・host clock・check を引き継ぐ，`ezsdr.children`，RS-25a に「runtime check が読む環境セクションは親と同じ」を追加），KF-4（SB-14 が存在しない Manifest フィールドを参照していた → forward に）．public item の増減なし |
| Vocabulary / Module | `sink` 1.1.0 の `sink.CAPTURE_WRITTEN`，capture Sink 1.2.0 が capture を書き終えるたびに出す．要求に番号を振り，`REQUEST_REJECTED` にも付ける（Review H/I） |
| Python | `connect()`（既定はサーバ側の x310-like ループバック），`tx.repeat`，`rx.capture`，`rx.request` / `rx.result`（先に要求を出すと連続して取れる，owner の依頼），`sleep`・`wait_until`・`wait_for`，`run(spec)`（child Run）．Vision §3・§54・§57 の例がそのまま動く |
| carriers | Python：§57，§58 #1/#3/#13/#16，§3 の 2 つ目の例，§54 ほか 21 件．Rust：Session の child Run として Phase 5 の Reactor が動く（`v58_09_a_reactor_runs_in_a_child_run_of_a_session`） |
| レビュー | Review H（Opus）CHANGES_REQUIRED：P0 5・P1 6・P2 12 → 全件修正．Review I（再レビュー）CHANGES_REQUIRED：P0 1（H の P0-2 が別経路で残っていた）・P1 3・P2 7 → 全件修正．修正は小規模で，すべてテストと mutation 付きなので owner の規則どおり再レビューせず終了．記録は [reviews/](plan/phase6/reviews) |
| 検証 | 1.85.0 / stable とも 684 passed，Python 21（3.9 / 3.13），Clippy clean，mutation 70/70 killed，link 全解決 |
| 注意 | Phase 6 の commit のうち `79c7655` は，会話から派生した別エージェントが作業途中のツリーをそのまま commit したもの（中身は Review H の修正の途中）．失われたものはない |

**Gate X の判断**（2026-09-27，owner「すべて推奨で受理します」，[00-overview.md](plan/phase6/00-overview.md) §11）：

1. **Session replay と artifact store** は Phase 7 のあと，frontend で一緒に作る（Kernel の型は変わらない）．
2. **KF-4**：Spec builder のソースハッシュは，v4.0 凍結前に Manifest の `spec` へ optional の `source` として足す（最初の Spec builder と一緒に）．Vision §9 はそのまま（Vision issue 4 は不要）．
3. S1–S11，A1–A8，spec 16・17 を受理．

**Step X**：spec 16 を [design/16-easy-api.md](design/16-easy-api.md) へ移し，参照を書き換えた．Vision issue 3 件（§3 の Normative 行に spec 16 と KC-37a，§15 に KC-29・KC-29b，§62 のプロセス境界の表にクライアント）を適用し，索引に改訂履歴の行を加えた（[vision-issues.md](plan/phase6/vision-issues.md)）．

**Phase 7 へ持ち越すもの**（Phase 6）：実機（wall-paced）での `sleep` と `capture` の間の往復遅延と「今から d 秒後」を返す補助（`00-overview.md` §3），`run_child` 中に親のデバイスを止めるか（S5 の ceiling），remote listener と server 所有のプロファイル・認証（S9），`body_bytes` の上限．

### UHD spike（2026-09-26，branch `spike/uhd`）

Phase 4 の計画の前に，owner の提案で「Phase 1–3 の Kernel が実機 USRP にどこまで通用するか」を確かめる使い捨て実装を作った．実機は X310+UBX と USRP2（owner の IBFD+SEFDM 実験系）．**Linux PC で実行する**（開発した Mac には USRP を繋がない）．

合意した条件：

- コードは捨てるか参考として残すだけ．成果は発見の記録．
- 作業は `spike/uhd` branch だけで行い，`main` へは merge しない．
- Kernel を変えるのは spike branch 上だけ．1 か所につき 1 commit とし，変えたこと自体を発見として記録する．
- 今は design 文書も spec も変えない．実測した値（lead，丸めの刻み，遅延）は記録するだけで，Mock profile の更新は Phase 8 の仕事．

中身（発見は `main` の [plan/spikes/2026-09-26-uhd.md](plan/spikes/2026-09-26-uhd.md)．実機の手順は `spike/uhd` branch の `spike/uhd/README.md`）：

| 項目 | 状態 |
|---|---|
| commit | `9ce8b6b` Kernel の KC-2 を 1 行変更（K1：device-paced class を通す），`d10f1c7` spike crate，その後 README に Linux 手順．`origin/spike/uhd` に push 済み |
| crate | `spike/uhd/` は**独立した cargo workspace**（`main` は libuhd なしでビルドでき，PO-2 の unsafe 禁止に触れない）．Module `ezsdr.radio.uhd` 0.1.0 が Provider（step されない，MA-15）と `Pacing::Device` の Authority を **1 つの binding** で兼ねる（`"authority": "radio"`）．UHD C API は約 40 関数を手書きで `extern "C"` 宣言 |
| Spec | acceptance crate の `experiments::*` をそのまま使う（変えたのは周波数と利得だけ）．Mock のテストとの違いは BindingProfile だけ（Vision §59） |
| オフライン検証 | wall-clock の fake device（`args = "fake"`）で `cd spike/uhd && cargo test` の 6 本が通過．`main` の workspace テストも K1 を当てた状態で全通過 |
| 実機 | **保留**（2026-09-26，owner の判断でソフトだけの開発を続ける）．Mac では「UHD のエラー文が返ること」までしか確かめていない |
| 発見 | FINDINGS.md の K1–K13（実機前・fake で VERIFIED）．大きいのは K2（agenda が空になると Spec Run が完了する），K3（TX clock が arm 起点のため TX/RX の grid が 1 sample 未満ずれる），K5（Session の admission が step されない Provider の処理を待たない），K6（RS-19 の lead を admission から数えている），K8（Sink が step されるのは client が呼んでいる間だけ） |

実機検証を再開するときの手順（Linux PC．保留中．測定値は Phase 7/8 の入力なので，ソフトだけの Phase はこれを待たない．`spike/uhd` は Phase 3 の Kernel から分岐しており，Phase 4・5 の Kernel 変更（KD-1，KE-1…KE-5）は入っていない）：

1. `git fetch origin && git switch spike/uhd`．`pkg-config --modversion uhd` で UHD 4.x が見えることを確かめ，`cd spike/uhd && cargo test` で 6 本が通ることを確認する（Linux でのビルドはまだ一度も試していない）．
2. `spike/uhd/README.md` の表の順に実行する：`find` → `probe ARGS` → `rx` → `session-rx` → `overflow` → `txrx` → `repeat` → `loopback` → `leads`．`ARGS` は X310 の 10 GbE なら `addr=192.168.40.2`，1 GbE と USRP2 なら `addr=192.168.10.2`．USRP2 には `--grid ideal` を付ける．**TX（`txrx` 以降）は ≥ 30 dB の減衰器を挟み `--tx-gain 0` で**．各 Run の出力は `spike/uhd/out/<cmd>-<time>/` に残る（`.gitignore` 済み）．
3. 結果で FINDINGS.md 末尾の「実機で埋める表」を埋め，実機でしか分からない発見（INFERRED だったもの，UHD 固有の挙動）を追記する．spike branch に commit する．
4. 実機で分かったことを `main` の `plan/spikes/2026-09-26-uhd.md` にも反映する（末尾の表と，INFERRED だった項目）．

注意：

- `crates/ezsdr-kernel` の `kernel_surface` gate（OV-23a）は，Kernel のソースに `uhd` という語があるとコメントでも落ちる．spike で Kernel を触るときはコメントにもこの語を書かない．
- Kernel の自前テストは KC-2 を WallPaced についてしか固定していない（K1 で既存テストが 1 本も落ちなかった理由）．
- 実機で詰まりそうな点：FPGA image の互換番号（UHD 4.10 用の image が要る，`uhd_image_loader`），X310 の master clock が 184.32 MHz だと `x310-like` の grid（200 MHz/N）と合わない，UHD 4.x が USRP2 をまだサポートしているかは未確認．

### 次にすること

1. **Phase 7 の計画**：Native UHD Provider（Vision §67）．Phase 6 から持ち越すもの（上の Phase 6 の節）と，Phase 4 からの項目（下の 5）．
2. **v4.0 凍結前にすること**：`Endpoint::EventIn` / `EventOut` の形（event edge は Phase 10），`ParamDecl.update_class` を optional にすること（以上 Phase 5 Gate X），Manifest の `spec.source`（Spec builder のソースハッシュ，Phase 6 Gate X，KF-4）．
3. **Session replay と artifact store**：Phase 7 のあと frontend で（Phase 6 Gate X）．
4. **Phase 10 へ持ち越すもの**（Phase 5）：event edge，component parameter を適用する Executor（UC-2…UC-6，MA-24），component の処理時間（budget）を仮想時間で課すこと，component parameter key の MA-34 検査，MA-30 の Action latency．
5. **Phase 7 へ持ち越すもの**（Phase 4 Gate X）：K3 の修正（TX model と一緒に），spike の K2・K5・K6・K8・K11，MA-8 の Kernel 側強制，UHD の `ERROR_CODE_ALIGNMENT` は部分 channel を返さない（VERIFIED）ので SC-31a の per-channel `ALIGNMENT` に producer がないかもしれないこと．詳細は [plan/phase4/00-overview.md](plan/phase4/00-overview.md) §3．
6. **Phase 2 の Gate X named risks**：Kernel 経由 Session `Stop(sink/rec)` test は Phase 4 で追加（`v58_13_a_session_stop_…`）．`MA-8` の強制は Phase 7 へ（Phase 4 §3）．
7. **GitHub Pages の状態表示**（https://k3komatsu.github.io/Ez-SDRv4/）は Phase 5 完了まで反映済み（別エージェントの `origin/gh-pages` commit「update Pages status after Phase 5」）．Phase 6 を反映するかは owner の判断（gh-pages は依頼があるときだけ変更する，AGENTS.md §1）．
8. Phase 2 のその他の test ceilings：sc16 capture と contract change 時の `partial`，KC-29 の直接 assert，到達不能な N8/N10 分岐，KA-12 marker check/acquire race の決定的 test seam，Provider→Sink datapath の nonzero drop→Manifest 経路．詳細は [implementation-notes.md](plan/phase2/implementation-notes.md) と exit tables を参照．

## 5. v3 → v4 切替で残っている作業（人間の判断が要るもの）

2026-09-26 に v4 を `k3komatsu/Ez-SDRv4` へ分離したので，default branch の切替（旧 #1），`release: published` がどの branch の workflow を使うか（旧 #4），dub registry の `~master`（旧 #5）は問題でなくなった．v3 の `Ez-SDR` は `master` と `dub.json` をそのまま持つ．残るもの：

| # | 項目 | 状態 |
|---|---|---|
| 1 | container image 名の衝突：v3 の `.github/workflows/ezsdr-build.yml`（`Ez-SDR` 側）は `release: published` で `ghcr.io/<owner>/ezsdr:latest` を push する（`matrix.uhd == DEFAULT_UHD` のとき）．ghcr の package は owner 単位なので，v4 が同じ `ezsdr` 名で image を出すと `:latest` を取り合う | 未．v4 の image 名を変えるか，v3 利用者を固定 tag（例 `:v3.0.28`）へ誘導してから v4 をリリースする |
| 2 | v3 readme の `ghcr.io/k3kaimu/ezsdr:latest` と現 owner `k3komatsu` の不一致 | 未確認．workflow は `github.repository_owner` を使うので現在の image は `k3komatsu/ezsdr` のはず |
| 3 | `Ez-SDR`（v3）の readme に v4 リポジトリ `Ez-SDRv4` への一行を足す | 未 |
