# Ez-SDR v4 — Handoff (2026-10-04)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `origin` = `git@github.com:k3komatsu/Ez-SDRv4.git`（https://github.com/k3komatsu/Ez-SDRv4，default branch `main`）．2026-09-26 に v4 を新リポジトリへ分離した．v3 は https://github.com/k3komatsu/Ez-SDR（`master` のみ，v3 の tags もそちら）に残る |
| `main` | v4．**Phase 0–7 の受理完了**（Phase 7 は 2026-10-01 に Gate X 受理，`worktree-phase7-impl` を merge．Phase 7 の Step X も同日に実施：spec 18 を `design/18-uhd-radio.md` へ移動，Vision issue 3 件を適用．exit-review（`plan/phase7/exit-review/`）も作成，Python 3.9 と `v3/` パスも確認済み．usrp-lnx02 の clone には 2026-10-01 に `k3komatsu/Ez-SDR` から `master` を取得して `v3/` worktree を追加した）．以前の記録：**Phase 0–6 の受理完了**（Phase 6 Step X まで push 済み）．Phase 2 Gate X を owner が2026-09-25に受理し，spec 06–10 を `design/` へ移動，Vision issue 15件を適用（Step X 完了）．Step 16/Gate X/Step X は `bf3b1bf` で commit・push 済み．2026-09-25 の動的レビューで見つかったテスト欠落2件を追加テストで塞いだ（[implementation-notes.md](plan/phase2/implementation-notes.md) 末尾）．Opus `PASS_WITH_RISK` の残余リスクは §4 と `plan/phase2/00-overview.md` §11 に記録．**Phase 3 は 2026-09-26 に Gate P 受理・実装・Review C・Gate X 受理・Step X 完了**（spec 11 を `design/` へ移動，Vision issue 8件を適用）．**Phase 4 は同日に計画・実装・Review D/E・Gate X 受理（すべて推奨どおり）・Step X 完了**．**Phase 5 は同日に計画・実装・Review F/G・Gate X 受理（すべて推奨どおり）・Step X 完了**（spec 14 を `design/` へ移動，Vision issue 3 件を適用）．**Phase 6 は 2026-09-26〜27 に計画・実装・Review H/I・Gate X 受理（すべて推奨どおり）・Step X 完了**（spec 16 を `design/` へ移動，Vision issue 3 件を適用）．§4 参照．|
| `master` | v3（D + C++ UHD bridge + Python client）．tip `4a474e9` = tag `v3.0.28`．この clone ではローカルのみ（upstream なし）．GitHub 上の置き場は `k3komatsu/Ez-SDR` |
| `spike/uhd` | **使い捨ての UHD spike**（2026-09-26，push 済み）．`main` から分岐し，Kernel 1 行の変更（K1）と `spike/uhd/` crate を載せる．**`main` へ merge しない**．成果は発見の記録だけで，`main` の [plan/spikes/2026-09-26-uhd.md](plan/spikes/2026-09-26-uhd.md) に移した．実機検証は保留．§4「UHD spike」参照 |
| `gh-pages` | GitHub Pages 用の orphan branch（`main`・`master` と履歴を共有しない）．2026-09-26 に `24229a9` で作成し，プロジェクト概要サイト（`index.html`，`showreel/`）を載せた．最新は `af488c9`「update Pages status after Phase 4」で push 済み．**Pages は公開済み**：https://k3komatsu.github.io/Ez-SDRv4/（branch `gh-pages` の root から配信，2026-09-26 に `gh api repos/k3komatsu/Ez-SDRv4/pages` で `status: built` を確認）．**サイトの状態表示は Phase 4 時点のまま**（「Phase 5 の計画は未作成」などと書いてある）で，Phase 5 完了を反映するには更新が要る．AGENTS.md §1 のとおり，gh-pages は owner の依頼があるときだけ変更する |
| tags | ローカルに 35 個（`v2.11`, `v3.0.0`–`v3.0.28`）．すべて v3 系で，`Ez-SDR` 側にある．`Ez-SDRv4` には tag なし．**`git push --tags` / `--all` をしない**（v3 の tag と `master` が v4 リポジトリに上がる） |
| 履歴の関係 | **無関係（unrelated）**．graft も merge もしていない．v4 は clean-sheet なので今後も繋がない |

ローカル：

- `/Users/komatsu/work/Ez-SDRv4` = `main` の worktree（メイン）．2026-09-27 に `/Users/komatsu/GoogleDrive/github/Ez-SDRv4` から移した．
- `…/Ez-SDRv4/v3` = `master` の **git worktree**（入れ子 clone ではない）．`.gitignore` で除外．
- `/Users/komatsu/orca/workspaces/Ez-SDRv4/gh-pages` = `gh-pages` の git worktree（Orca の workspace．作業ツリーの外）．
- 設計文書が v3 のパス 22 件を証拠として引用しているので `v3/` は消さない（`design/` と Vision に加えて `plan/phase1/` からも引用がある）．確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- `v3/` を消してしまった場合：`git worktree prune && git worktree add v3 master`
- clone を移動して `v3/` の git リンクが切れた場合（`git worktree list` で `prunable`，`git -C v3 status` が `not a git repository`）：`git worktree repair v3`（2026-09-27 に実施）
- v3 の更新を取り込むときの取得元は `origin` ではなく `k3komatsu/Ez-SDR`（例：`git fetch git@github.com:k3komatsu/Ez-SDR.git master`）．
- `~/work` は Google Drive で同期していない．Drive 上にあった頃（〜2026-09-27）は同期で追跡ファイルが消えることがあった（2026-09-24 に `crates/ezsdr-kernel/tests/` と `schemas/` の 61 ファイルが消え，`git restore` で戻した）．clone を Drive 配下へ戻すなら，作業前に `git status --short` に ` D` 行がないことを確かめる．

## 2. 設計文書の状態 — Phase 0〜7 完了（Phase 7 は 2026-10-01 に Gate X 受理・Step X 完了・クローズ）

- 単一の設計ソース = **Vision**：索引 [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) + [design/vision/](design/vision) の 11 part（§1–§68，番号は不変）．
- [design/v4-vision-audit.md](design/v4-vision-audit.md)（Findings 1–34，判定 READY WITH REQUIRED CHANGES）→ 全項目を Vision に反映済み．
- [design/v4-vision-rereview.md](design/v4-vision-rereview.md)（Findings R1–R22，判定 READY）→ 全項目反映済み．R13「規範部分の spec 化」は当初11ファイル分割で暫定対応したが，Phase 1 Step 5（D109，2026-09-23）で Phase 1 specs へ，Phase 2 Gate X（2026-09-25）で specs 06–10 へ反映し，完了した．
- accepted specs は Phase 1 の [design/01-time-model.md](design/01-time-model.md)〜[design/05-module-api.md](design/05-module-api.md) と Phase 2 の [design/06-kernel-coordinator.md](design/06-kernel-coordinator.md)〜[design/10-host-data-path.md](design/10-host-data-path.md)，Phase 3 の [design/11-simulation-channel.md](design/11-simulation-channel.md)（spec 12 の修正は design/04–09 に適用済み），Phase 5 の [design/14-native-executor.md](design/14-native-executor.md)（spec 15 の修正は design/03–06 に適用済み）．各 Phase の決定ログ・実装計画・rule exit evidence は [plan/phase1/](plan/phase1)〜[plan/phase5/](plan/phase5) に残る．Phase 6 の [design/16-easy-api.md](design/16-easy-api.md)（server・protocol・Python）．spec 17 の修正は `design/03・04・06・10` に適用済みで，記録は [plan/phase6/17-amendments.md](plan/phase6/17-amendments.md)．Phase 7 の [design/18-uhd-radio.md](design/18-uhd-radio.md)（UHD Provider）．spec 19 の修正は実装の commit で `design/` に入り，記録は [plan/phase7/19-amendments.md](plan/phase7/19-amendments.md)．Gate X の後に Kernel の race を直し，RS-36（`design/04`）と KC-31（`design/06`）の文を変えた（design-notes §21・§22）．
- 旧 CMA は退役．[design/archive/](design/archive) に保管（audit / rereview の `CMA §N` 引用のためだけに残す）．編集しない．
- Vision の改訂履歴は索引ファイル末尾の表（Phase 0/1 の8 passに加え，Phase 2 Gate X の適用を2026-09-25に，Phase 3・4・5 の Gate X の適用を2026-09-26に記録）．Phase 4 の spec 13 は amendment だけなので `design/` へ移すものはなく，本文は実装 commit で `design/02・05・07・08・09・10・11` に入っている．

## 3. 実装の状態

下の表は 2026-09-27（Phase 6 Step X 後の `618ae4a`）の状態．Phase 7 で変わったもの（`main`，2026-10-01，Phase 7 クローズ時の `b60f57a`）：crate `ezsdr-radio-uhd` が加わり 13 crate，`cargo test --workspace` 898 passed（stable と 1.85.0），`-p ezsdr-radio-uhd --features uhd` 174 passed（libuhd 4.10 の Docker image `ezsdr-v4-dev:uhd4.10` の中で），clippy clean，`kernel_surface` は 116 NEW / 292 public items（OV-23b が assert する），Phase 7 の mutation は `plan/phase7/tools/mutations.json` の 173 行で生きている行はすべて killed．詳細は §4「Phase 7」：

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

## 4. Phase の状態 — Phase 1〜7 完了（Gate X 受理）．次は Phase 8（未着手）

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

### Phase 7 — 実装済み（2026-09-27〜28，branch `worktree-phase7-impl`，`origin` へ push 済み，`main` へは未 merge）

owner の依頼（「Phase 7を実装してください」，続けて「終わったらサブエージェントOpus 5.5にレビューさせて，修正してください．修正が大規模なら再レビュー，小規模で低リスクならそれで終わりです」）で，00-overview §9 の手順 1–6 を実装し，Opus のレビューを 2 回回した．commit（`main` の `25e79f3` から）：

| commit | 中身 |
|---|---|
| `3172a1f` | 手順 1–2：Kernel KG-1…KG-14（device-paced class，data thread，KC-21a/KC-24a，MA-8 の強制，T0 の格子，TM-18 の relation，child Run 拒否），`radio` 1.3.0（`device` module：Grid，DeviceDescription，RM-26），MockRadio 1.3.0 |
| `27065e7` | 手順 3–4：`crates/ezsdr-radio-uhd`（Module `ezsdr.radio.uhd` 0.1.0：`Device` trait と `FakeDevice`，`DeviceAuthority`，`UhdRadio` の 3 thread，profile `x310-ubx`，feature `uhd` の `src/uhd.rs`（unsafe はここだけ，GZ-3），`tests/fake.rs`・`uhd_api.rs`・`hardware.rs`（全部 `#[ignore]`）） |
| `2c4a8ae` | 手順 5：server 0.2.0（catalogue に UHD，`Config.open_device`・`default_profile`，binary の `EZSDR_PROFILE`，`status.root_rate`，device-paced の child は KG-12 で拒否），Python 0.2.0（`sleep`/`wait_until` が時刻を返す，`Session.after`，`examples/bench_loopback.py`），acceptance `tests/uhd.rs`（§59 の rehearsal を FakeDevice で），design/16 |
| `89f02b7` | 手順 6：mutation（`plan/phase7/tools/`，Appendix A と spec 18 §8）．`mutate.py` は timeout で process group ごと kill するよう直した |
| `e2d4d26` | Review L（CHANGES_REQUIRED：P0 3・P1 8・P2 19）の修正：streamer の二重 free，UR-12 の rate 規則（512 中 487 を拒否していた），0 channel からの受信開始時刻，受信 timed stop の早すぎる発行，UHD のエラー文，preempt 時の end-of-burst，テスト多数．大規模なので再レビュー |
| `9b79b58` | Review M（同じ reviewer の再レビュー：P0 0・P1 2・P2 8）の修正：遅れた timed stop を missed start と取り違えて受信を再開していた件，1.85 で flaky だった `ur_21` など．小規模・局所的なので owner の規則どおり再レビューせず終了 |
| `def6dc9`，`3ffb3bd` | 実機検証の手順（この節の下），UHD 4.10 の Docker 環境と devcontainer |
| `09f11b4`，`15f0c61` | 実機を **X310 + CBX 1枚のループバック** に変更（owner，2026-09-30：「UBX2枚での試験ではなくCBX1枚でループバックで可能なように内容を修正してください」）：profile `x310-cbx` 0.1.0（1 channel，1.2–6 GHz，既定 2.45 GHz），UR-5 が front end 名と channel 数を実機と照合，テストは device の front end から profile を選ぶ．Opus レビュー（P2 5 件）の修正．[design-notes.md](plan/phase7/design-notes.md) §9 |
| （最後の commit） | 実機を **X310 + OBX 1枚のループバック** に再変更（owner，同日：「了解ですOBXに切り替えてください」）：profile `x310-obx` 0.1.0（1 channel，10 MHz–8.4 GHz，既定 1 GHz，UBX と同じ timed tune の位相同期）．`x310-cbx` は予備として残す．design-notes §10 |

状態（`9b79b58`）：`cargo test --workspace` 839 passed（stable と 1.85.0，libuhd なしでもビルド可），clippy `-D warnings` clean（feature `uhd` の有無とも），`cargo test -p ezsdr-radio-uhd --features uhd` は libuhd 4.10 のこの Mac で通る（実機なし），Python 23 件（3.9/3.13），mutation は `plan/phase7/tools/mutations.json` の 114 行（G09 だけ black-box では勝てない race として記録）．実装が spec と違うところは [design-notes.md](plan/phase7/design-notes.md) §6，Review L・M の全指摘と対応は §7・§8，レビュー本文は [reviews/](plan/phase7/reviews)．

注意：scratch copy を共有 target dir でビルドしたあと worktree をビルドすると，mutation 入りの成果物が再利用されることがある（Review L の修正中に 1 回起きた）．ビルド前に必ず `find crates schemas python Cargo.toml Cargo.lock -type f -exec touch {} +`．

（当時の）残り：実機セッション，`main` への merge，Gate X，Step X — **すべて 2026-10-01 に済んだ**（下の「Phase 7 のクローズ」）．

### Phase 7 の実機検証 — 次の工程（別エージェント向け，Linux サーバー + USRP X310 + OBX 1枚のループバック）

owner の指示（2026-09-28）：「実機では別のlinuxサーバーでUSRPを使うので，pushしておいてください．また実機検証は別エージェントで実施することになるので，handoffにも次の工程を細かく書いておいて」．この節だけで作業を始められるように書く．手順の本体は [plan/phase7/bench.md](plan/phase7/bench.md)（B0–B9，合格条件と記録する値）で，ここはその実行計画と，bench.md に書いていない実装側の事情．

#### 引き継ぎ（2026-09-30 セッション 1 → 次のセッション）

owner の依頼（2026-09-30）：「引き継ぎに必要な情報をまとめてどこかに保存しておいて」．次のセッションは `usrp-lnx02` の上で立つ（owner：「一旦家に帰るので，ssh先で別のセッションを立てようと思います」）．**この小節だけで再開できる**ように書く．結果の本体は [plan/phase7/bench-results.md](plan/phase7/bench-results.md)．

**1. 実機と環境（すべて確認済み）**

| 項目 | 値 |
|---|---|
| サーバー | `usrp-lnx02`（Ubuntu 24.04.4，x86_64，20 core）．Mac からは `ssh usrp-lnx02`（port 10022，user `komatsu`）．**sudo はパスワードが要る**：sudo の要る操作は owner に頼む |
| clone | `~/works/Ez-SDRv4`，branch `worktree-phase7-impl`（origin を追跡，`9344952` 以降）．作業前に `git pull --ff-only` |
| Rust / UHD | ホストには **Rust が無く，UHD は apt の 4.6（OBX を知らない）**．すべて Docker image `ezsdr-v4-dev:uhd4.10`（`docker/uhd4.10/Dockerfile`，サーバーで build 済み，UHD 4.10.0.0，Rust stable と 1.85.0）の中で行う．`komatsu` は `docker` group なので sudo 不要 |
| build 出力 | Docker volume `ezsdr-v4-cargo-target`（`/cargo-target`，ツリーの外，AGENTS.md §7）．release の `ezsdr-server --features uhd` は `/cargo-target/release/ezsdr-server` に build 済み |
| 装置 | **USRP X300**（serial 347D545），**FPGA は UHD 4.10 の HG image**（2026-09-30 に `uhd_image_loader` で書き換え，電源を入れ直した．owner 承認済み．もう書き換えないこと：ホストの UHD 4.6 からはこの X300 を使えなくなっている） |
| daughterboard | slot A **OBX**（`OBX RX`/`OBX TX`），slot B 空（UHD は unknown board の channel 1 として数える：2 + 2 channel が正常）．profile は自動で `x310-obx` |
| RF | OBX の TX/RX → SMA + 30 dB 減衰器 → OBX の RX2（owner 確認済み，bench.md の RF 安全条件を満たす）．利得 0 dB，振幅 ≤ 0.5，1 GHz（envelope 999–1001 MHz） |
| ネットワーク | X300 の 10 GbE（X300 側 port 1，HG image）↔ サーバー NIC（Intel X710）**port 0 `enp2s0f0np0`**，`192.168.40.10/24`，MTU 9000（NetworkManager「USRP 10Gb(40)」，自動接続）．**`ARGS=addr=192.168.40.36`**（192.168.40.2 ではない）．port 1 `enp2s0f1np1` は `192.168.44.10/24`（別サブネット，使わない）．`net.core.wmem_max` 33 554 432・`rmem_max` 50 000 000（owner が設定済み．再起動で消えるかは未確認） |
| 記録 | 各テストの全出力はサーバーの `~/ezsdr-bench/<名前>.log`，B7 用の Session profile は `~/ezsdr-bench/bench-session.json`（作成済み），Python の Manifest は `~/ezsdr-bench/ezsdr-runs/` |

**2. 実行の形**（`-w /work` を忘れない．セッション 1 は `/bench` から cargo を呼んで B6–B8 を空振りした）

```sh
cd ~/works/Ez-SDRv4
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture \
  > ~/ezsdr-bench/<test>.log 2>&1
```

Python の B7 は同じ container を `-w /bench -e PYTHONPATH=/work/python -e EZSDR_SERVER=/cargo-target/release/ezsdr-server -e EZSDR_PROFILE=/bench/bench-session.json` で，`python3 /work/python/examples/minimal.py` と `bench_loopback.py`（`PYTHONDONTWRITEBYTECODE=1`）．装置の確認は `docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36`．落とし穴：長い出力は `| cut` などで受けると最後まで届かないことがあったので，ファイルに書いてから読む．リンクが上がった直後の `uhd_find_devices` は空振りすることがある（数秒待って再実行）．

**3. 済んだこと**（詳細と数値は bench-results.md）

- B0（native x86_64 で 847 passed，stable と 1.85.0），B1（`x310-obx`，時刻 ±1 % 以内），B2（lateness 中央値 557 µs），B3（read-back 差 0），B4（先頭 sample 50 000），B5（1 s の stall で overrun）：**合格**．
- Python の B7：実行済み（`TIME_ERROR` 0 件，20 Msps への丸め）．
- 見つかったこと 3 件：(a) B3 で X300 が連続受信の **timed stop を守らず**，UR-25 の untimed 切り替えが働いた（B8 の受信 stop 測定で確かめる），(b) B5 で UHD の overrun 再開後の block 2 179 個が格子外（UR-17 の丸めで処理，推測），(c) `bench_loopback.py` が **受信**周波数を変えているので RF envelope に拒否されない（RM-19 は送信だけを制限する．直すのは例）．

**4. 次にやること（この順）**

1. `git pull --ff-only`．
2. B6 `hw_b6_txrx_and_repeat`，Rust の B7 `hw_b7_session_loopback`，B8 `hw_b8_leads` を上の形で実行し，bench-results.md に記録（B8 は bench.md の表の一部しか実装されていない：下の B8 の行を参照）．
3. `python/examples/bench_loopback.py` の `sdr.rx.frequency = 2.4e9` を `sdr.tx.frequency = 2.4e9` に直す（bench.md「If a step fails」に従い，直す前に記録済み．例なので fake のテストは不要）．commit・push し，Python の B7 をやり直す．EA-17（`z` が `t` から始まるか）を Manifest の時刻から出す．
4. B5 の restart gap の長さを記録していない：`rehearse_overflow` か `hw_b5_overflow` に `println!` を足して再実行（テストコードの追加は可）．
5. B9：owner に 10 GbE ケーブルを抜いてもらう手作業（受信 Run 中．送信だけの Session での抜線はテスト未実装）．その後 B3–B7 を再実行し偽の `DEVICE_LOST` がないことを確認．USRP2 があれば `hw_b9_usrp2_probe`．
6. `plan/spikes/2026-09-26-uhd.md` 末尾の表を埋め，この節と bench-results.md を更新して commit・push．Gate X の材料（00-overview §10，criterion 9 は B0–B8 の合格，`plan/phase7/exit-review/` は未作成）を owner に渡す．

**4a. セッション 2（2026-09-30，`usrp-lnx02` 上の Claude Code）でしたこと**

- 上の 4. の 2–4 は済み：B6 合格（テストの相関を直してから．遅延 44 samples），Rust の B7 合格，`bench_loopback.py` を送信周波数に直して Python の B7 合格（`rf_envelope` が拒否），EA-17 は「`z` は `t` から始まる」，B8 は実行（3 ms 以下は UR-21 が受け取り時に捨てるので device lead そのものは測れていない），B5 の restart gap は 456 ms（大半は stall，UHD の再開は ≤ 約 67 ms）．（「受信の timed stop は 9 回中 9 回守られなかった」と書いたのは誤り：それらの Run では timed stop は device に出ていない．part 2 で訂正）．詳細と出力は bench-results.md「Session 2」．
- 直したのはテストコードだけ（Module のコードと profile の値は変えていない）．`cargo test --workspace` 849 passed，`--features uhd` 103 passed・10 ignored，clippy clean．
- **push**：このサーバーの `origin` は HTTPS で認証情報がない．SSH は通るので `git push git@github.com:k3komatsu/Ez-SDRv4.git worktree-phase7-impl` で push し，`git fetch origin` で追跡を合わせた（remote の設定は変えていない）．
- 次：B9 の抜線（owner がケーブルを抜く手作業．直前に止まって確認する），spike 表の記入，design-notes §11 の F1–F3 への owner の判断．

**4b. セッション 2 part 2（owner が遠隔の間，手作業なしでできる確認をすべて）**

- owner：「ちょっと今手元にUSRPがなくて遠隔でやってます．なので，とりあえず今のうちに今の状態でUSRPを使って確認しておいた方がいいことを考えて全部やってください」．
- B8 の未実装の行を `Device` を直接叩くテスト（`hw_b8_raw_*`）と Module 経由のテストで測り，受信レート，レートごとの遅延，120 s の受信と 60 s の送信だけの Session（偽の `DEVICE_LOST` なし），60 s の drift も測った．最後に B1–B8 を `85ac77b` で通しで流し直して全部合格．結果は bench-results.md「Session 2, part 2」．
- **設計を変える必要のある発見が 3 つ**（design-notes §11，owner の判断待ち，コードは未変更）：F1 X300 は受信の timed stop の時刻を無視して即時に止まる（FPGA source で VERIFIED）→ `cold` な受信変更で e1 前の約 50 ms が flag なしで欠ける；F2 その後の再開が 100 ms の receive timeout 待ちで e2 に遅れる；F3 前の burst の終わりの tick ちょうどに timed SOB を置くと device が late として捨てる（UR-23 の preemption がこの形）．FakeDevice はどれも実機より緩い．
- 前の記録「受信の timed stop は 9/9 守られなかった」は誤りだったので訂正した（その Run では timed stop は device に出ていない）．

**4c. F1–F3 の修正（owner：「F1-F3は推奨で」）**

- `bc0db98`：FakeDevice を X300 と同じ厳しさにして 6 本の再現テストが落ちるのを確かめてから，uhd-rx は timed stop を出さず cut で untimed stop（F1），cut 待ちの間の `rx_recv` の timeout を cut + 1 block に（F2），uhd-tx は隙間 0 の次の burst を同じ device burst で続ける（F3）．spec 18 UR-17・UR-23〜26・UR-33・§6，mutations（124 行，変更したファイルの 31 行を再実行，R18 は取り下げ）．workspace 851 passed（stable と 1.85.0），clippy clean．
- 実機（bench-results.md「Session 2, part 3」）：cold 変更で e₁ まで sample が届き e₂ に遅れず開始，20 ms の preemption の burst が再生される．B1–B8・B9 の長時間 Run・Python B7 すべて合格．
- 新しい発見：X300 の ref PLL が `set_clock_source` で lock しないことが 2 回（約 24 回の open 中）．UR-7 で再試行するかは owner の判断待ち．
- 次：B9 の抜線（owner が現地で），spike 表の記入，main への merge と Gate X（owner）．

**4d. Review N〜R（Opus による修正のレビュー，2026-09-30〜10-01）**

- Review N（F1–F3 の修正）→ `aaf1e00` で直し，Review O → `10ddb73`・`b3c94ba`（O-B1：burst の最後の 1 sample を手元に残して EOB を付ける．空の EOB は UHD が 0 の 1 sample にするので device burst が 1 sample 延びる，`hw_b8_raw_empty_eob_gap` で VERIFIED．O-B2：受信 cold 変更の e₁ を進行中の受信呼び出しの後ろへ）．Review P は blocker なし（`reviews/review-p.md`），P2 の指摘を owner の「推奨通りにしてください」で直した（design-notes §14）．
- 記録：design-notes §12–§14，bench-results.md「Session 2」part 4–6，`reviews/review-n.md`・`review-o.md`・`review-p.md`．
- Review Q → `bd8151c`（D-1：B2 が終了時に double free で 1 回異常終了．`DeviceAuthority` の drop が `uhd-clock` を join していなかった．fake で再現するテストを書いてから修正）．Review R は blocker なし，NB-R1・NB-R2 は記録のみ（owner），NB-R3・TG-R1 は owner の判断待ち（design-notes §16）．
- **X300 の状態**：B1 の `ref_locked` が part 5 から false になり，ref PLL の lock 失敗が増えた（bench-results.md part 7）．owner が 2026-10-01 に **X300 の電源を落とした**．owner が戻したと言うまで実機は動かさない．
- X300 は owner が電源を入れ直した（2026-10-01）．その後の全段は合格（part 8）．ref PLL の lock 失敗は open の約 4 %，`ref_locked` とは連動しない．
- **B9 済み**（part 9）：抜線で X300 がプロセスごと落ちる F4（UHD の radio `deinit` が destructor から例外を投げる，gdb で VERIFIED）を `0c2ae4d` で直した（失われた device は free しない）．その後，受信・送信だけの両方で `DEVICE_LOST` → Policy で停止 → Manifest，ただし `DEVICE_LOST` まで 3–4 s／6.0 s，終了時に UHD の static teardown で abort（UR-29 に記録）．B3–B7 再実行で偽の `DEVICE_LOST` なし．USRP2 は不要（owner）．
- Review S（F4 の修正と B9 のレビュー）：P1 の S-B1（気づいていない死んだ link は free される）を直した（読み出しが 1 s 失敗し続けたら lost，drop の前に 1 回読む，失った device は args ごとに覚えて次の open で片付ける）．UR-7 は lock 失敗の理由を文で返し，既定で 1 回開き直す（`reopen_on_unlock` で止められる）．design-notes §18・§19．
- **B9 をスイッチ経由でも実施**（part 10，`8ebf593`）：送信だけの Session も S-B1 の規則（`UHD error 47`）で 3.0 s 後に `DEVICE_LOST`，受信は 1.10 s，同じプロセスでの開き直しも成功，どのプロセスも正常終了．現在の配線は PC → L2 スイッチ → X300．
- Review T（`4fd2ec2`）は blocker なし．P2 を `1d3f382` で直し，直結で実機確認（part 11）：送信だけの抜線は新しい規則で 3.0 s，開き直しで保持していた device が 1 → 0（`reclaim` が free），正常終了．配線は直結に戻した．spike 表は記入済み（`80f947a`）．
- **Gate X 受理**（2026-10-01，owner「終わったらmainへmergeしてください．そしてGate Xを受理します」）．`worktree-phase7-impl` を `main` へ merge．**Step X**：spec 18 を `design/18-uhd-radio.md` へ，Vision issue 3 件（§35・§15・§32）を適用．受理時に未確認だった 3 点（criterion 2 の exit-review，Python 3.9，`v3/` パス）も実施，exit-review の 9 か所の未検査の文にテストを足した（mutation C01–C10）．
- その途中で `kg_02` の不安定なテストから **Kernel の race** を見つけて直した（`b7a7271`，design-notes §21）：RS-36 の escalation flag は hot path が「止める」event の中身を捨てたときだけ立てる．Review U（blocker なし）の P2 も直した（`b60f57a`，§22）．

**Phase 7 のクローズ**（2026-10-01，owner「記録を直してPhase 7を閉じてください」，00-overview §11 の最後の行）

- Phase 8 へ送ったもの：NB-5（TimingEnvelope の parity），UR-32（spec 18 の「0.1.0 では満たさない」），B8 の未測定の項目と spec 18 の lead の値（00-overview §3「Phase 8 inputs」），OBX 用の Mock profile を作るか・`radio` 語彙に phase `random` を足すか（Phase 8 で owner が決める），複数台と v61_04（Phase 8 の後）．
- 記録だけ（作業予定なし）：TG-4・TG-5，KC-36 の `Release` での更新（観測できず assert できない），X300 の参照 PLL の lock 失敗（open の約 4 %，UR-7 の 1 回の開き直しで救える）．
- **X300 の状態（2026-10-01 クローズ時）**：つながっていない（`enp2s0f0np0` の carrier 0）．Phase 8 で実機を使う前に owner に戻してもらい，`uhd_usrp_probe` で確認する．配線は直結．
- 次：Phase 8（Mock ↔ X310 parity）の計画．owner の依頼を待つ．

**Phase 7 後の保守**（2026-10-01〜02，`main` の `b8548da`・`287fff9`）

- `b8548da`：private な実行時の責務の整理（refactor）．その途中で `tests/fake.rs` の 2 本が不安定に落ちた：`ur_25_enabling_tx_applies_the_configuration`（loopback が返らない）と `ur_23_a_burst_one_sample_after_a_burst_is_played`（device の Underflow / TimeError，または予約時の late policy で Drop）．
- 原因（調査と Opus レビュー 2 回）：どちらも refactor 前の `0882992` でも負荷をかけると再現する．wall-clock のテストが，uhd-tx が host の期限を守ることを前提にしている：in-flight window 10 ms，保持した最後のサンプルは端の 2 ms 前（device lead）まで，予約は開始の 2 ms 前まで．FakeDevice の時計は実機と同じく host が止まっても進む．Underflow 後に残りを今の時刻から流す fake の動きは UHD 4.10 の `radio_tx_core.v`（`:341`，`:347`，`:359–370`）と整合する（INFERRED）．調査の証拠（トレース，負荷実験のログ）はリポジトリの外（`/tmp/ezsdr-origin-investigation/`，セッションの scratchpad）にあり，消えうる．
- `287fff9`（テストだけ）：`ur_23_a_burst_one_sample_after_a_burst_is_played` は uhd-tx が device に渡した burst の構造（サンプル数，B の時刻）を先に確かめ，そのあと従来どおり strict な end-to-end を見る．予約時に Drop された試行だけ前提不成立として最大 3 回まで再試行する．新規テスト `ur_33_a_burst_resumed_after_an_underflow_plays_late` で Underflow 後の fake の動きを固定した．`ur_25` と B6 rehearsal は失敗時に async の報告を出す．mutation（空の end-of-burst，Underflow 後の再開位置，Underflow の報告）は killed．TX テストの直列化と `SlowSend` で Underflow を起こすテストは試したが，効果がない／テスト自体が負荷に依存するので入れていない．
- **残課題**（owner の判断待ち，Phase 8 の前に）：

| # | 分類 | 内容 | 次の一手 |
|---|---|---|---|
| 1 | production | uhd-tx は待機中の held burst を，自分の時計で開始時刻を過ぎていても late policy を判定し直さずに渡す（`tx.rs` の `step()`）．結果は `TIME_ERROR { late_at_device }` になるが，RM-11（`design/07-radio-model.md:135`）の「自分の時計では間に合って渡した」に反する．`send_asap` も守られない | UR-21 を改めて，渡す時点でも `LatePolicy::decide` するか決める |
| 2 | spec の空白 | Underflow 後，uhd-tx の `next` は予定の時間軸のまま，device は後ろへずれる．device が持つのは window＋ずれ（fake では 50 ms の flow control で止まる）．UR-23 の「`s` で止めた burst は `s + 10 ms` までに終わる」は無条件に書いてあるが成り立たない．burst record の時刻もずれる．Underflow 後の再開を決めた契約はない | 実機の測定（下の H2）のあと，burst を終わらせるか，時刻を取り直すか，上限の書き方を変えるか決める |
| 3 | fake の忠実度 | fake は TimeError で burst 全体を捨てる．FPGA の既定 policy は `TX_ERR_POLICY_PACKET`（host は `REG_TX_ERROR_POLICY` を書かない）で，遅れた packet だけ捨て，あとの時刻なし packet はすぐ流す．bench は 1 packet の burst しか測っていない．複数 packet だと UR-28 の `unacked` との対応付けもずれうる | H1 のあと UR-33 と fake を直すか決める |
| 4 | 運用上の制約 | 保持した最後のサンプル（design-notes N-5）は，burst の端ごとに uhd-tx が約 2 ms 以内に動く必要がある．負荷のある開発機では `rehearsal_b6_*` と `ur_23` の end-to-end がこれで落ちる（B6 の burst は 1 buffer なので，Underflow は最後のサンプルでしか起きない） | Review N B3／O-B1 の設計の代償として受け入れるか，Phase 8 で見直すか |
| 5 | テスト | 負荷で落ちうるテストが残る．開発用 Mac（load average 7–60）での全 suite は 17 回中 3 回失敗（すべて `rehearsal_b6` 系）．ほかに `ur_21_an_untimed_send_is_on_time_after_its_delivery` と `ur_23_a_continuation_s_first_send_that_takes_nothing_ends_the_device_burst` が予約時の遅れ（uhd-control が 10 ms 以上遅れた）で 1 回ずつ落ちた．`ur_23_a_burst_one_sample…` の late leg は，B が A の最後の buffer の後に予約されたことを確かめていない（以前から） | 残すか，個別に前提の確認を足すか |
| 6 | 実機（Phase 8） | H1：開始が遅れた複数 packet の burst（TimeError の数，残りが流れるか，BurstAck）．H2：burst の途中の Underflow（再開時刻，ずれ，途中の packet を捨てるか，loopback の位相）．H3：TX の `max_num_samps`．H4：負荷下の uhd-tx・uhd-control の起床遅延．H5：保持したサンプルの 2 ms 期限 | Phase 8 の bench 計画に入れる |

- 検討して採らなかったもの：FakeDevice の時間を遅くする（例：1/10 倍速）．`DeviceAuthority` は device の時計が公称の 200 MHz で進むとして外挿するので，fake だけ遅くすると壊れる．Authority と Provider の wall-clock の待ちまでそろえると，host の期限を 10 倍緩めることになり，Mock が実機より緩くなる（§13，§34）．決定的な検査は Simulation Engine か，`Tx::step()` を直接回す unit test で行う．

**Bug-fix maintenance, 2026-10-03 (owner-authorized fixes, review, commit and push)**

The preceding Phase 7 maintenance table records the findings before this audit.
Its items 1 and 2 are now fixed: first dispatch reapplies the burst's late policy,
and underflow/loss or an expired transmit timeline abandons the damaged device
burst rather than resuming untimed payload. Hardware H1–H5 remain Phase 8 inputs;
these fixes do not claim a new hardware measurement.

GitHub issues [#1–#10](https://github.com/k3komatsu/Ez-SDRv4/issues?q=is%3Aissue)
were handled in order, with a separate GPT-6.1 Sol review before each fix commit:

| Issue | Fix | Commit |
|---|---|---|
| #1 | Refuse an existing capture artifact path; preserve its data and metadata | `5809f0e` |
| #2 | Validate and reduce deserialized Rational values | `c1e905a` |
| #3 | Refuse overlapping same-direction cold changes until the owner finishes | `930457a` |
| #4 | Python `samples()` requires every channel's validity to cover the entire span | `56ae95f` |
| #5 | Reject runtime updates whose class differs from the target's declaration | `8bb7571` |
| #6 | Recheck the first timed TX dispatch; preserve cold cuts and replace reservations atomically | `40dbb0d` |
| #7 | Refuse negative ClockRelation measurement bounds without changing frozen schemas | `90e934e` |
| #8 | End damaged/expired TX timelines, preserving independent future held bursts | `99511ab` |
| #9 | Record RX stop issuance once across LateCommand, repeated Abort and Shutdown | `535d4f3` |
| #10 | Correct timing measurements; prove control ordering with a controlled clock; report measured stream prerequisites before waveform/index assertions | this maintenance commit |

Final software validation: **916 passed, 0 failed, 0 ignored**, both stable and
Rust 1.85.0; Python **28 passed** on 3.9 and 3.13 against the server built from
this tree; UHD 4.10 API/link tests **4 passed**; Clippy with `-D warnings` clean
for the workspace and for the UHD/server feature build. All reviews returned
PASS; #6's cold-cut/reservation findings and #7's schema-description finding
were fixed and re-reviewed before commit.

Maintenance item 5's host-load limitation remains explicit. The 128-thread
stress probe of 134 wall-clock trials had **100 passed, 34 failed**, including
8 `TIMING_PRECONDITION_UNMET` diagnostics. This is not a passing stress run or
proof that the host caused every failure. No fake clock/envelope was relaxed,
no trial was ignored, and a missed prerequisite remains a test failure. The
subsequent diagnostic regression brings the ordinary fake suite to 135 trials,
all passing in the normal stable/MSRV runs. See
[the test guide](crates/ezsdr-radio-uhd/tests/README.md) for commands and failure
interpretation. Audit evidence is in `~/.cache/ezsdr-audits/2026-10-03-3b0444e/`;
fix-validation logs are in `~/.cache/ezsdr-fixes/2026-10-03/`.

**Independent audit follow-up, 2026-10-03 (issues #11–#14)**

The second audit checked current `main` against the specifications and all
existing issues, including closed #1–#10 and the known stress failures above.
Only four new, verified defects were filed. The owner then authorized fixes,
GPT-6.1 Sol subagent review, commit and push, in issue order:

| Issue | Fix | Commit |
|---|---|---|
| [#11](https://github.com/k3komatsu/Ez-SDRv4/issues/11) | Enforce the host-domain invariant when deserializing RelativeBudget | `c46bbe5` |
| [#12](https://github.com/k3komatsu/Ez-SDRv4/issues/12) | Refuse cancellation handles from another authority's root before queue access | `2dbcc62` |
| [#13](https://github.com/k3komatsu/Ez-SDRv4/issues/13) | Refuse capability ranges whose interiors are not proved covered by the declared set | `04387d7` |
| [#14](https://github.com/k3komatsu/Ez-SDRv4/issues/14) | Report clock-ID exhaustion without reuse or partial SampleClock declarations | this maintenance commit |

Rust source compatibility: `ClockRegistry::allocate_id` now returns
`Result<ClockDomainId, TimeError>`; callers must handle `LimitExceeded`.
TM-11 documents the exhaustion sentinel. Serialized types and frozen schemas
are unchanged. #13 conservatively refuses general non-singleton ranges against
discrete declarations; singleton and Boolean intervals remain supported.
Current runtime prepare reports use `CapabilityValue::One`.

Final software validation after #11–#14: **923 passed, 0 failed, 0 ignored**
on both stable and Rust 1.85.0; Python **28 passed** on each of 3.9 and 3.13
against the freshly built server; UHD 4.10 API/link tests **4 passed**.
Workspace and UHD/server feature Clippy builds pass with `-D warnings`.
Each fix passed a separate GPT-6.1 Sol review before commit; #13's suggested
test strengthening was applied and re-reviewed.

The 128-thread stress failures and hardware H1–H5 remain unresolved validation
items; these fixes provide no new stress or hardware measurement. Independent
audit evidence is in
`~/.cache/ezsdr-audits/2026-10-03-b4c2934-independent/`; fix-validation and Sol
review logs are in `~/.cache/ezsdr-fixes/2026-10-03-second/`.

**Third audit follow-up, 2026-10-03 (issues #15–#18 and #5 residual)**

The third independent audit checked `b0c8ce8` against the accepted contracts,
all existing issues #1–#14, and the known stress/hardware limitations. Four new
functional defects were reproduced and registered. The owner authorized the
same sequential fix, GPT-6.1 Sol subagent review, commit and push procedure:

| Issue | Fix | Commit |
|---|---|---|
| [#15](https://github.com/k3komatsu/Ez-SDRv4/issues/15) | Refuse collisions across resource and qualified need names before matching | `ddb30c0` |
| [#16](https://github.com/k3komatsu/Ez-SDRv4/issues/16) | Validate derived Sink addresses before setup; preserve the parent Session on child validation failure | `f53efdf` |
| [#17](https://github.com/k3komatsu/Ez-SDRv4/issues/17) | Require complete, unique lifecycle reports and verify ownership before merging | `691bc56` |
| [#18](https://github.com/k3komatsu/Ez-SDRv4/issues/18) | Preserve pending link DropCarry in partial captures and SigMF metadata on lifecycle/targeted Stop | `9e47349` |
| [#5](https://github.com/k3komatsu/Ez-SDRv4/issues/5) residual | Cache update classes per component and judge a Module update against its target's declaration | this maintenance commit |

The #5 residual was not registered again: two components declaring the same
key with different classes shared one map, allowing the later declaration to
replace the target's. The fix keeps private per-component maps and passes only
the target's declarations to admission/compilation. Provider and Sink keys
continue to use their registered Vocabulary declarations. Public APIs and
serialized schemas are unchanged. No client-component bug is claimed: current
Sessions derive no graph components, and Spec Runs reject SessionAction submit
with `NotSession`.

All five fixes passed separate GPT-6.1 Sol reviews before commit. #16's Python
regression initially expected an exception; it was corrected to assert the
existing structured child `Failed { Validate }` result and successful parent
capture, then passed re-review. The other four reviews required no changes.
Each fix has a regression case observed failing on the previous implementation.

Final software validation: **932 passed, 0 failed, 0 ignored** on both stable
and Rust 1.85.0; Python **29 passed** on each of 3.9 and 3.13 against the freshly
built server; UHD 4.10 API/link tests **4 passed**. Workspace and UHD/server
feature Clippy builds pass with `-D warnings` (all targets).

The recorded 128-thread stress failures (100/134 passed, 34 failed) and hardware
H1–H5 remain unresolved; these fixes provide no new stress or hardware
measurement and relax no timing envelope. Audit evidence is in
`~/.cache/ezsdr-audits/2026-10-03-b0c8ce8-third/`; fix-validation and Sol review
records are in `~/.cache/ezsdr-fixes/2026-10-03-third/`.

**Fourth audit follow-up, 2026-10-03 (issues #19–#24)**

The owner authorized sequential issue validation, repair, GPT-6.1 Sol review,
commit, push and closure. Each issue was reproduced with a regression failing
on its preceding implementation, then independently reviewed before its fix
commit:

| Issue | Fix | Commit |
|---|---|---|
| [#19](https://github.com/k3komatsu/Ez-SDRv4/issues/19) | Select dropped Abort over Stop without changing delivered-event cause order | `e2ee2b1` |
| [#20](https://github.com/k3komatsu/Ez-SDRv4/issues/20) | Share cold configuration with RX/TX owners; project later timed updates through e2 in actual effective order | `a71ed96` |
| [#21](https://github.com/k3komatsu/Ez-SDRv4/issues/21) | Remove cancelled held updates from cold projections; preserve issued updates and stream Stop semantics | `a13229b` |
| [#22](https://github.com/k3komatsu/Ez-SDRv4/issues/22) | Refuse any TX channel slice shorter than the count sent to UHD before entering C | `7d38c1f` |
| [#23](https://github.com/k3komatsu/Ez-SDRv4/issues/23) | Serialize public native calls and metadata/error-text access per streamer | `d4339f6` |
| [#24](https://github.com/k3komatsu/Ez-SDRv4/issues/24) | Validate finite, nonnegative drift uncertainty before multiplying by elapsed ticks | this maintenance commit |

All six reviews returned PASS. #22's initial push preceded checking a failed
Clippy result: a test-only one-element Vec triggered `useless_vec`. This was
corrected to an array in `7c124ad`, re-reviewed by Sol, and verified with feature
Clippy before proceeding to #23. No production behavior changed in that follow-up.

UR-25/UR-26 clarify shared cold projections and cancellation; UR-3 documents
the enforced public native-call boundary. Public Rust interfaces, serialized
types and frozen schemas remain unchanged. Native-boundary tests substitute
streamer C entry points only in the unit-test executable, use no USRP and never
read dummy streamer or sample pointers; separate UHD API integration tests
still link real libuhd.

Final software validation: **938 passed, 0 failed, 0 ignored** on both stable
and Rust 1.85.0; Python **29 passed** on each of 3.9.6 and 3.13.15 against the
freshly built server. Feature-enabled library tests, including the native
boundary regressions: **19 passed** on both Rust toolchains; real UHD 4.10 API
integration tests: **4 passed**. Workspace and UHD/server feature Clippy builds
pass with `-D warnings` (all targets).

Existing stress failures and hardware H1–H5 remain unresolved validation items.
These fixes provide no new stress or hardware measurement and relax no timing
envelope. Reproduction, validation and Sol review records are in
`~/.cache/ezsdr-fixes/2026-10-03-fourth/`.

**Fifth audit follow-up, 2026-10-04 (issues #25–#34)**

The owner again authorized sequential validation, repair, GPT-6.1 Sol review,
commit, push and closure. All ten issues were checked against accepted contracts
and reproduced by a failing regression before repair.

| Issue | Fix | Commit |
|---|---|---|
| [#25](https://github.com/k3komatsu/Ez-SDRv4/issues/25) | Validate sub-resource constraint declarations, types and extension ownership | `ea4acad` |
| [#26](https://github.com/k3komatsu/Ez-SDRv4/issues/26) | Resolve sink targets only through declared outputs and Sink fragments | `6cc58a9` |
| [#27](https://github.com/k3komatsu/Ez-SDRv4/issues/27) | Validate Action Values before coercion or dispatch | `4bd68ab` |
| [#28](https://github.com/k3komatsu/Ez-SDRv4/issues/28) | Escalate EOB device errors and stop further TX calls after device loss | `7cb3547` |
| [#29](https://github.com/k3komatsu/Ez-SDRv4/issues/29) | Check cold boundaries before mutation; widen near-limit owner arithmetic | `07924f5` |
| [#30](https://github.com/k3komatsu/Ez-SDRv4/issues/30) | Correlate async TX reports independently per channel | `d9e02a7` |
| [#31](https://github.com/k3komatsu/Ez-SDRv4/issues/31) | Convert delayed TX error targets through their original immutable clocks | `1fdc002` |
| [#32](https://github.com/k3komatsu/Ez-SDRv4/issues/32) | Refuse unconvertible explicit update deadlines without side effects | `fa6942b` |
| [#33](https://github.com/k3komatsu/Ez-SDRv4/issues/33) | Emit fractional-rate Mock tails through e1 and defer clock replacement | `e16915a` |
| [#34](https://github.com/k3komatsu/Ez-SDRv4/issues/34) | Finish contract-changing captures with their pending drop provenance | this maintenance commit |

Sol's review loops found additional paths in #28 (report handling followed by
TX step after loss), #29 (an accepted near-limit boundary overflowing in its
owners), and #33 (collapsed cold-update ordering, backward boundaries after a
rate change, and disable/enable losing the boundary floor). Each was repaired
and covered by regressions before its final PASS. #33's two existing channel
tests now query the new clock at e1; their waveform, root-target and burst-record
assertions are preserved. #34 covers both supported contract-change directions,
with and without a subsequent capture request.

UR-24/UR-28 and MR-18 clarify refusal, original-clock report correlation and
old-tail timing. Public Rust interfaces, serialized types and frozen schemas
remain unchanged. FakeDevice models normal ACK/time-error reports per TX
channel; packet-level behavior still requires the recorded H1 hardware measurement.

Final normal software validation: **959 passed, 0 failed, 0 ignored** on both
stable and Rust 1.85.0. Python **29 passed** on each of 3.9.6 and 3.13.15 against
the freshly built server. Feature-enabled UHD library tests: **31 passed** on
both Rust toolchains; real UHD 4.10 API/link tests: **4 passed**. Workspace and
UHD/server feature Clippy builds pass with `-D warnings` (all targets).

A separate final-tree FakeDevice run with `--test-threads=128` had **102 passed,
34 failed**. It exposed late booking, missing/extra samples and link-drop gaps;
it is not a passing stress result. Host pacing pressure is consistent with the
previous recorded failures (INFERRED), but this single probe does not establish
absence of stress regressions. No timing envelope or assertion was relaxed.
Hardware H1–H5 remain unmeasured in this session.

Reproduction and validation logs, with the review-loop summary, are in
`~/.cache/ezsdr-fixes/2026-10-04-fifth/`.

**Sixth audit follow-up, 2026-10-06 (issues #35–#38)**

The owner authorized the same sequential validation, repair, review, commit,
push and closure for the four implementation issues of the sixth audit. Under
AGENTS.md §8 the independent reviewer was a Claude Opus 5.5 subagent instead of
GPT-6.1 Sol. Each issue was reproduced by a regression failing on its preceding
implementation before repair. The six `[spec]` issues #39–#44 of the same audit
are design questions and remain open.

| Issue | Fix | Commit |
|---|---|---|
| [#35](https://github.com/k3komatsu/Ez-SDRv4/issues/35) | Leave a Mock RX fault before the replacement clock's origin unapplied (MR-20) | `1c6d361` |
| [#36](https://github.com/k3komatsu/Ez-SDRv4/issues/36) | Keep every distinct RX report until the next block: union of flags, one event each | `f6eb904` |
| [#37](https://github.com/k3komatsu/Ez-SDRv4/issues/37) | `Device::tx_async` returns `Result`; a failed read is loss or a counted error, never "no report" | `fd33160` |
| [#38](https://github.com/k3komatsu/Ez-SDRv4/issues/38) | Return a completed child's Manifest with no `path` when it cannot be written | `9141c16` |

The review loops changed two fixes. For #35 a cold-cut clamp was withdrawn:
a pending cold change can still be refused when it applies, and the old stream
then keeps running. Same-instant ties keep spec 09 §4's `(instant, insertion)`
order, so a fault at e1 still meets the old stream. Whether that order should
change is #44's question. For #37, FakeDevice fails `tx_async` only while a
transmit streamer is open, as `UhdDevice` makes no call without one. Otherwise
`ur_29_a_lost_device_is_found_while_idle` would no longer exercise uhd-control's
time read. #36 gained a published-block flag check after a mutation that kept
only the last report survived the first test.

UR-19, UR-28, UR-29, UR-30 and EA-14 state the new behaviour, and spec 18's
trait listing shows the new `tx_async` signature. The public `Device` trait
signature changed (only this crate implements it). `stats.tx_errors` is a new
key. No serialized type or frozen schema changed.

Open follow-ups the reviews found, outside these issues:

- MockRadio: a fault after the old stream's last sample but before e1, or at e1
  ahead of the cold change, records a loss of samples that an accepted cold
  change later removes. The emitted `RX_OVERFLOW` then has no matching gap in
  any delivered block. Correct accounting would wait until the update applies.
  Filed as [#45](https://github.com/k3komatsu/Ez-SDRv4/issues/45).
- Server: the `Ran.path` doc comment and schema description could say
  "absent when it could not be written". In the EA-10 connect-failure path, a
  `manifest.json` that cannot be written loses that Manifest, as the spec
  currently allows.

Final normal software validation: **963 passed, 0 failed, 0 ignored** on both
stable and Rust 1.85.0. Python **29 passed** on each of 3.9.6 and 3.13.15
against the freshly built server. Feature-enabled UHD library tests: **33
passed** on both Rust toolchains; real UHD 4.10 API/link tests: **4 passed**.
Workspace and UHD/server feature Clippy builds pass with `-D warnings` (all
targets).

A separate final-tree FakeDevice run with `--test-threads=128` had **100
passed, 37 failed**, with 5 `TIMING_PRECONDITION_UNMET` lines. Its failures are
overrun and link-drop gaps and late bookings under host load. The failing set
differs from the fifth probe's (8 newly failing, 5 no longer). Host pacing
pressure is consistent with this (INFERRED), but one probe does not establish
the absence of stress regressions; it is not a passing stress result. No timing
envelope or assertion was relaxed. Hardware H1–H5 remain unmeasured.

Reproduction, mutation and validation logs, with the review-loop summary, are
in `~/.cache/ezsdr-fixes/2026-10-06-sixth/`.

**5. 決まったこと・owner の判断待ち**

- 決定済み（owner，2026-10-06）：**v4 はリリース前なので，このソフトウェアにとって本当に長期的に有益なら，破壊的な仕様変更をしてよい．** 凍結前の schema（v4.0 はまだ凍結していない．OV-12），Python API，Vocabulary・Module の版，既存の fixture の変更も含む．互換性のコストを理由に，長期的に劣る案を選ばない．ただし規律は保つ：schema の差分は再生成して `schemas/SCHEMA_CHANGELOG.md` に記録する（OV-12），版を上げる，古い文書は移行するか拒否し，黙って読み替えない（不変条件 39）．spec 20（`plan/maintenance/20-amendments.md`）の判断はこの前提で行っている．
- 決定済み（2026-09-30）：実機は **X300 + OBX 1枚のループバック**（UBX 2 枚 → CBX 1枚 → OBX と変更．profile `x310-obx`，`x310-cbx` は予備），X300 は X310 の代わりで可，FPGA 書き換え承認．
- owner の判断待ち（Phase 8 で）：Phase 8 の Mock 比較で OBX 用の Mock profile を作るか（MockRadio の `x310-like` は UBX の値），`radio` 語彙に phase `random` を足すか（`x310-cbx` の制約，§10 の design-notes）．

**前提と規則**

- **実機構成**（owner，2026-09-30）：X310（**X300 でも可**：UHD は同じ `x300` driver で扱い，違いは FPGA の大きさだけで，この実装はそれを使わない．コードも profile `x310-obx` も変更不要．FPGA image は X300 用（`uhd_image_loader` が `usrp_x300_fpga_XG.bit` を選ぶ）を入れる．bench.md 冒頭）の slot A に **OBX 1枚**．TX/RX を 30 dB 以上の減衰器を通して同じ OBX の RX2 へ（ループバック）．送信だけの段のために 50 Ω 終端．10 GbE（SFP+）の Linux PC．外部の 10 MHz/PPS は不要．**UHD 4.9 以上が必須**（OBX は 2025 年 6 月に UHD に入った．古い UHD では unknown board になり UR-5 が拒否する）ので Docker 環境（UHD 4.10）を使う．X310 の FPGA image もその UHD に合わせる．profile は `x310-obx` 0.1.0（1 channel，10 MHz–8.4 GHz，既定周波数 1 GHz）で，テストは UHD が報告する front end 名（`OBX…`）から自動で選ぶ．OBX が 2 枚あっても同じ profile で動く（使うのは channel 0）．途中まで CBX 1枚で準備したので `x310-cbx` も残っている（OBX の driver が不調なときの予備）．事情は [design-notes.md](plan/phase7/design-notes.md) §9・§10．
- 作業するのは branch `worktree-phase7-impl`（`origin` にある，この節を書いた最後の commit 以降）．`main` にはまだ Phase 7 がない．この branch に commit して push してよい（v4 の作業なので `master` と v3 の tag には触れない，AGENTS.md §1）．`main` への merge は owner の判断なのでしない．
- 実機で失敗したら，その場でコードを直さない（bench.md「If a step fails」）．`plan/phase7/bench-results.md` に出力ごと記録し，`FakeDevice` で再現するテストを先に書いてから直し，その段をやり直す．直したら `cargo test --workspace` を通して commit・push．
- 設計を変える必要があると判断したら（INFERRED だった値が違う，規則が実機に合わない），実装より先に `plan/phase7/design-notes.md` に「§10 bench」節を作って記録し，owner に判断を仰ぐ．spec 18 の数値（device lead 2 ms，delivery allowance 3 ms，restart lead 50 ms，in-flight window 10 ms，release window 3 ms，queue depth 16）は Phase 8 の入力として **記録するだけ** で，profile の値はこの session では変えない（bench.md 冒頭，spec 18 §3）．
- build 出力は作業ツリーの外に置く（`CARGO_TARGET_DIR` をツリー外に，AGENTS.md §7）．Python は `PYTHONDONTWRITEBYTECODE=1`．

**0. 取得と環境（B0）**

1. `git clone git@github.com:k3komatsu/Ez-SDRv4.git && cd Ez-SDRv4 && git switch worktree-phase7-impl`（`git log -1` がこの節を書いた commit 以降であること）．`v3/` は不要（`.gitignore` 済み）．
2. **推奨：Docker の UHD 4.10 環境**（`docker/uhd4.10/Dockerfile`，VS Code なら `.devcontainer/`）．v3 の `docker/v3_prebuild` と同じ base image `ghcr.io/k3komatsu/uhd:v4.10`（Ubuntu 26.04，UHD 4.10.0.0 を `/usr/local` に source build，FPGA image 取得済み，Python 3.14 + numpy）に Rust stable と 1.85.0 を足しただけ．source は mount，build 出力は volume `/cargo-target`（`CARGO_TARGET_DIR` と `EZSDR_SERVER` は設定済み）：
   ```sh
   docker build -t ezsdr-v4-dev:uhd4.10 docker/uhd4.10
   docker run -it --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
       --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target \
       ezsdr-v4-dev:uhd4.10
   ```
   `--network=host` は X310 の 10 GbE に届くため，SYS_NICE と rtprio は UHD の streaming thread の優先度のため．MTU と `net.core.{r,w}mem_max`（下の 3.）は host 側で設定する（container の設定ではない）．devcontainer（VS Code の「Reopen in Container」か `npx @devcontainers/cli up --workspace-folder .`）は同じ引数で起動し，user `ubuntu`（UID は host に合わせられる），build 出力は別の volume `ezsdr-v4-devcontainer-target` に置く．この環境では `LD_LIBRARY_PATH` も `UHD_LIB_DIR` も要らない（`ldconfig` 済み，`pkg-config` が `/usr/local/lib` を返す）．下の 4. のコマンドはこの中でそのまま動く（`export CARGO_TARGET_DIR` は不要）．
   - 2026-09-28 に Mac（arm64）上の amd64 emulation でこの image を build し，確認済み：`cargo test -p ezsdr-radio-uhd --features uhd --test uhd_api` 4 passed（x86_64 Linux での 69 関数の link と構造体サイズ 40/40/48/40），clippy `-D warnings`（feature `uhd`）clean，`ezsdr-server --features uhd` が `/usr/local/lib/libuhd.so.4.10.0` に link，Python 23 件 OK（3.14），devcontainer（`DOCKER_DEFAULT_PLATFORM=linux/amd64`）でも user `ubuntu` のまま `uhd_api` 4 passed．`cargo test --workspace --no-fail-fast` は 836 passed・3 failed で，落ちたのは wall-clock に依存するテストだけ：`fake.rs` の `rehearsal_b6`・`rehearsal_b7`，acceptance の `ea_07_a_session_on_the_fake_device`．`--test-threads=1` にすると別の 4 本（`ur_07_now_tracks…`，`ur_22`，`ur_23`，`ur_25_tx_channels_from_zero…`）が落ちる．B6 の中身は `TIME_ERROR late_at_device`（5.35 ms 遅れ）で，burst が `drop_and_flag` で捨てられていた．emulation では Mac native より 6 倍ほど遅く（`fake.rs` が 33 s → 194 s），落ちるテストが実行ごとに変わるので，emulation の遅さによるものと判断した（INFERRED）．実機の x86_64 では native に動くので，4. の `cargo test --workspace` が 839 passed になることを最初に確かめる．そこで落ちたら負荷の問題ではない可能性があるので，出力ごと記録する．
   Docker を使わない場合：Rust は `rustup` で stable と `1.85.0`．UHD 4.x の開発用パッケージと pkg-config：Ubuntu なら `sudo apt install libuhd-dev uhd-host pkg-config`（または Ettus PPA）．`pkg-config --modversion uhd` と `uhd_config_info --version` を記録．
   - `/usr/local` に source build した UHD なら `PKG_CONFIG_PATH=/usr/local/lib/pkgconfig` か `UHD_LIB_DIR=/usr/local/lib` で build，実行時は `sudo ldconfig` か `LD_LIBRARY_PATH=/usr/local/lib`（build.rs は rpath を付けない，UR-35）．
   - **未確認**：UHD 4.10 以外の版（Linux での build と構造体サイズは上の Docker 環境の UHD 4.10 で確認済み）．`src/uhd.rs` は UHD 4.10 の header に対して手書きで 69 関数を宣言しているので，古い UHD（Ubuntu 22.04 の apt は 4.1 系）で link error が出たら，足りない関数名を記録し，UHD を 4.6 以上にするか owner に相談する．`tests/uhd_api.rs` の構造体サイズ（40/40/48/40）は arm64 macOS で測り，x86_64 Linux の UHD 4.10（Docker）でも一致した．別の UHD 版で違ったら **実機に進まず** 記録して止まる（C API の構造体が合わないと未定義動作）．
3. ネットワークと装置（bench.md B0.2–B0.3）：X310 の 10 GbE port 0 を `192.168.40.2`，host NIC `192.168.40.1/24`，MTU 9000，`sudo sysctl -w net.core.rmem_max=33554432 net.core.wmem_max=33554432`．`uhd_find_devices --args addr=192.168.40.2`．FPGA image が合わなければ `uhd_image_loader`．master clock が 200 MHz でなければ device string に `master_clock_rate=200e6` を足す（UR-5 が 184.32 MHz を拒否する）．以下 `ARGS` はこの device string．
4. ソフトだけの確認（RF なし）：
   ```sh
   export CARGO_TARGET_DIR=$HOME/.cache/cargo-target/Ez-SDRv4
   cargo test --workspace                                  # 845 passed になるはず（libuhd を link しない）
   cargo +1.85.0 test --workspace                          # 同じ（MSRV）
   cargo test -p ezsdr-radio-uhd --features uhd            # fake 98（うち OBX 1枚の rehearsal 4，CBX 1枚の 1）+ uhd_api 4 が通り，hardware の 10 本は ignored
   cargo clippy -p ezsdr-radio-uhd -p ezsdr-server --features ezsdr-radio-uhd/uhd,ezsdr-server/uhd --all-targets -- -D warnings
   cargo build --release -p ezsdr-server --features uhd    # B7 用の server
   ```
   `tests/fake.rs` は wall-clock のテストが多い（1 回 33 s ほど）．負荷の高い機械で flaky なら記録（Review L・M で 2 本直した）．

**1. 記録ファイル**

`plan/phase7/bench-results.md` を作る（GZ-10）．冒頭に：装置の製品名（X300 か X310 か，`pp_string` に出る），commit hash，OS と kernel，CPU，NIC，`uhd_config_info --version`，FPGA image，`ARGS`，RF の配線（減衰量）．各段ごとに：実行したコマンド，合否，出力（`--nocapture` の `B<n> …` 行はそのまま貼る），bench.md の「Records」列の値．最後に `plan/spikes/2026-09-26-uhd.md` 末尾の表を埋める．

**2. 実行順（bench.md の表どおり，順序に意味がある）**

各段は `EZSDR_UHD_ARGS=$ARGS cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture`．テスト本体は `crates/ezsdr-radio-uhd/tests/hardware.rs`，B3–B7 の中身は `tests/common/mod.rs` の `rehearse_*`（`tests/fake.rs` が同じ関数を `FakeDevice` で毎回走らせているので，fake で通っていることは確認済み）．

| 段 | テスト | RF | 注意（実装側の事情） |
|---|---|---|---|
| B1 | `hw_b1_probe` | なし | `B1 profile: x310-obx` と，各 channel の front end 名（`OBX RX`・`OBX TX`）と channel 数（1 + 1 以上）が出ること．`x310-obx` でなければテストが失敗する（UHD の front end 名が `OBX` で始まっていない．`Unknown` なら UHD が 4.9 より古い）：名前を記録して止まる．UHD は空の slot B も unknown board の channel として数えるので，2 + 2 と出るのが正常．送信を始める前に，各 TX channel の周波数と利得をそのまま記録する（何も設定しない Session が引き継ぐ値，UR-25） |
| B2 | `hw_b2_authority` | なし | 10 s 待つ．lateness の中央値 < 1 ms，最大 < 25 ms．`DeviceAuthority::failed_reads()`・`discarded_reads()` は Manifest に出ないので，出力に無ければテストに `println!` を足して記録してよい（test コードの追加は可） |
| B3 | `hw_b3_receive_at_t0` | なし | `applied` と `timing` の section が出力される．周波数の read-back 差（K13：< 0.05 Hz 程度）を記録 |
| B4 | `hw_b4_capture_at_a_sample_index` | なし | bench.md は Session の capture と書くが，実装は Spec の schedule で `sink.capture_samples` を sample 50 000 に置く形（`rehearse_capture_at_a_sample_index`）．合格条件（最初の sample が 50 000）は同じ |
| B5 | `hw_b5_overflow` | なし | 300 ms → 1 s → 2 s の stall で overflow を起こす．どれでも起きなければ panic するので，そのときの `net.core.rmem_max` を記録 |
| — | RF 安全 | — | ここから送信する．bench.md「RF safety」：TX/RX → 30 dB 以上の減衰 → RX2（同じ OBX），利得 0 dB，振幅 ≤ 0.5．周波数は `x310-obx` の既定 1 GHz（テストも B7 の `bench-session.json` も envelope は 999–1001 MHz）．ラボの都合で変えるなら，`crates/ezsdr-radio-uhd/src/profile.rs` で `x310-obx` に `x310-cbx` と同じ形の既定周波数を持たせ（10 MHz–8.4 GHz 内），bench.md の 2 つの JSON の envelope を一緒に変え，spec 18 UR-9 も直して commit する（テストの周波数と envelope はこの既定値に従う） |
| B6 | `hw_b6_txrx_and_repeat` | あり | 相関で探すので，fake の「完全一致」は求めない（`exact = false`） |
| B7 | Python（bench.md B7） | あり | `bench-session.json` を bench.md の JSON から作る（`<dir>` を実在の directory に，`args` を `ARGS` に）．`EZSDR_SERVER=$CARGO_TARGET_DIR/release/ezsdr-server EZSDR_PROFILE=bench-session.json PYTHONDONTWRITEBYTECODE=1 python3 python/examples/minimal.py`，次に同じ環境変数で `python3 python/examples/bench_loopback.py`．Rust 版 `hw_b7_session_loopback` もあり，こちらは実機では「`TIME_ERROR` がひとつもない」ことを要求する（fake では負荷のため 5 ms までの send_asap を許している，Review M） |
| B8 | `hw_b8_leads` | あり | **実装は bench.md B8 の一部だけ**：burst を受信後 10, 5, 3, 2, 1.5, 1, 0.5 ms の lead で送り，各回の `TIME_ERROR` と `timing` を出す．表の残り（timed retune の lead，`cold` の restart lead 100/50/25/10 ms，`Stop` 後の送信終端，in-flight window 10/5/3/2 ms での underflow，preemption の bound，timed retune を何個積めるか（queue depth），receive の timed stop が効くか）は未実装．これらは Phase 8 の入力なので，`hardware.rs` に `hw_b8_*` を足して測るか（`#[ignore = "needs a USRP: see plan/phase7/bench.md"]` を付けること，governance の `gz_08` が検査する），測れなかったと記録する |
| B9 | `hw_b9_unplug`（手動），`hw_b9_usrp2_probe` | なし | unplug は 60 s の受信 Run 中に 10 GbE を抜く．`DEVICE_LOST` が約 1 s 以内，termination が `Stopped { policy { DEVICE_LOST } }`．送信だけの Session での unplug は bench.md の表にあるがテストは未実装（`hw_b9_unplug` を真似て足すか記録）．USRP2 は `EZSDR_UHD_ARGS=addr=192.168.10.2` で `hw_b9_usrp2_probe`：UR-5 の拒否（100 MHz）を確認．最後に B3–B7 をもう一度流し，偽の `DEVICE_LOST` が出ないことを確認 |

**3. 実機で特に見てほしい点（コードで INFERRED にしているもの）**

- UR-25：X3x0 が continuous stream の **timed stop** を守るか．守らないと uhd-rx が「cut を越えた sample」を見て untimed stop に切り替え，`timing` に `rx_stop_untimed` が出る．`cold` の rate 変更（B7 の `sample_rate = 19.5e6`）で確認．
- Review M P1-A：遅れた timed stream command が receive metadata の `LATE_COMMAND` で返るか（返れば `timing` に `rx_stop_late`）．
- UR-24：stream command と retune が device の同じ queue に並ぶか（B8 の追加測定）．
- UR-29：抜線でどの UHD error が出るか（`IO`/`USB`/`OS`/`RUNTIME`）．`RUNTIME` が streaming 中に偽陽性を出さないか（B9 の rerun）．
- UR-3：thread 安全性（control call は mutex，streamer は 1 thread 所有）．クラッシュや `double free` のメッセージが出たら最優先で記録．
- EA-17：B7 の `z = sdr.rx.capture(N, at=t)` が `t` から始まるか（`bench_loopback.py` が y と z の相関位置を出す）．
- UR-9（`x310-obx`）：B3 の `applied` で周波数の read-back が 1 GHz に一致するか．B8 の lead と queue に積める timed tune の数は OBX の値として記録する（UBX と同じ MAX2871 系だが tune の実装は別なので，UBX の値とみなさない，design-notes §10）．
- OBX の driver は新しい（UHD 4.9 から）：UHD の警告（特に `OBX` の「Unable to set dboard clock rate - phase will vary」）が出たら記録する．

**4. 終わったら**

1. `plan/phase7/bench-results.md` と `plan/spikes/2026-09-26-uhd.md` の表を commit・push（`docs(phase7): bench session`）．
2. `handoff.md` のこの節を結果で更新（合否，Phase 8 への入力値，未測定の項目）．
3. Gate X の判断材料をそろえて owner に渡す：00-overview §10 の exit criteria（criterion 9 は B0–B8 の合格），`plan/phase7/exit-review/`（OV-3 の rule disposition，まだ作っていない）は Gate X の前に必要．

### Phase 7 — 設計（Gate P 受理）

2026-09-27，owner の依頼（「Phase7の設計と実装を進めてください．実機がいる直前まで進めてください」）で着手し，途中で「設計だけで止めてください．ただし，設計は詳しく設計してください」と変更された．**コードは書いていない**（KG-4 の実装に着手した分は commit せず `git checkout` で戻した）．成果物はすべて [plan/phase7/](plan/phase7)（未 commit）：

| 文書 | 中身 |
|---|---|
| [00-overview.md](plan/phase7/00-overview.md) | 穴 19 件（spike K1–K13，Phase 2/4/6 からの持ち越し，設計中に見つけたもの）と証拠，範囲・範囲外，決定 T1–T18，crate 構成，GZ-1…GZ-10，テスト方針，Phase 8 への入力表，手順，exit criteria |
| [19-amendments.md](plan/phase7/19-amendments.md) | spec 19：Kernel KG-1…KG-14（device-paced class を駆動，data thread，client 呼び出しなしで無線機を止める，KC-21a/KC-24a，MA-8 強制，T0 の格子，lead は dispatch から，TM-16c/TM-18/TM-13b/TM-13e/UC-3，child Run 拒否），Vocabulary/Module/frontend VE-1…VE-6（`radio` 1.3.0，MockRadio 1.3.0，server，Python の `after`），mutation 一覧 |
| [18-uhd-radio.md](design/18-uhd-radio.md) | spec 18：`ezsdr.radio.uhd` 0.1.0（UR-1…UR-35，U1–U14）。UHD の C API（feature `uhd`，unsafe は `src/uhd.rs` だけ），`Device` trait と `FakeDevice`，profile `x310-ubx` 0.1.0，device-paced Authority，送受信・更新・停止・事象・Manifest，テストと mutation |
| [bench.md](plan/phase7/bench.md) | 実機セッションの手順 B0–B9（Linux PC + X310 + OBX 1枚のループバック（2026-09-30 に UBX から CBX，さらに OBX へ変更），RF 安全，プロファイル，各段の合格条件と記録） |
| [design-notes.md](plan/phase7/design-notes.md) | 確認した根拠，設計中の発見，Review J・K の全指摘と対応 |

レビュー（luna-primary-engineer の手順を参考に Opus を subagent で）：**Review J**（CHANGES_REQUIRED：P0 4・P1 11・P2 23）→ 全件を文書で修正（大規模）→ **Review K**（同じ reviewer の再レビュー：新規 P0 1・P1 5・P2 14，J の未解決 1）→ 全件修正．K の修正は小規模（推奨修正そのもの，局所的）なので owner の規則どおり再レビューせず終了（理由は design-notes §5）．記録は [prompts/](plan/phase7/prompts)，[reviews/](plan/phase7/reviews)．

**Gate P**（2026-09-27，owner「すべて推奨で受理します」，00-overview §11）：T1–T18・U1–U14・spec 18/19 を受理．multi-device と v61_04 は Phase 8 の後（device→host の relation と一緒に），USRP2 profile は B9 の `pp_string` の後，remote listener・server 所有の profile・認証・`body_bytes` は独立した後の phase，Gate X は実機 B0–B8 の合格を要する．**実装はしない**（owner「実装はしないで」）．

注意（実装に入るとき）：リポジトリを `~/work/Ez-SDRv4` へ移したとき mtime が保たれたため，cargo が旧パスでビルドした古いテストバイナリを再利用し，パスを読むテスト 21 本が落ちた．`find crates schemas python Cargo.toml Cargo.lock -type f -exec touch {} +` の後は 684 passed（design-notes §1）．

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

0. **Phase 7 後の保守の残課題**（§4「Phase 7 後の保守」の表）：production の 1–2 と fake の 3 は owner の判断，6 は Phase 8 の bench 計画へ．
1. **Phase 8（Mock ↔ X310 parity）の計画**（owner の依頼待ち）：Phase 7 は 2026-10-01 にクローズ済み（merge，実機セッション，Gate X，Step X）．Phase 8 へ送ったものは §4「Phase 7 のクローズ」，Phase 8 inputs は [plan/phase7/00-overview.md](plan/phase7/00-overview.md) §3．実機を使う前に X300 を戻してもらい `uhd_usrp_probe` で確認する．
2. **v4.0 凍結前にすること**：`Endpoint::EventIn` / `EventOut` の形（event edge は Phase 10），`ParamDecl.update_class` を optional にすること（以上 Phase 5 Gate X），Manifest の `spec.source`（Spec builder のソースハッシュ，Phase 6 Gate X，KF-4）．
3. **Session replay と artifact store**：Phase 7 のあと frontend で（Phase 6 Gate X）．
4. **Phase 10 へ持ち越すもの**（Phase 5）：event edge，component parameter を適用する Executor（UC-2…UC-6，MA-24），component の処理時間（budget）を仮想時間で課すこと，component parameter key の MA-34 検査，MA-30 の Action latency．
5. **Phase 7 へ持ち越したもの**（Phase 4 Gate X；Phase 7 の設計で全件に行き先あり，00-overview §8）：K3 の修正（TX model と一緒に），spike の K2・K5・K6・K8・K11，MA-8 の Kernel 側強制，UHD の `ERROR_CODE_ALIGNMENT` は部分 channel を返さない（VERIFIED）ので SC-31a の per-channel `ALIGNMENT` に producer がないかもしれないこと．詳細は [plan/phase4/00-overview.md](plan/phase4/00-overview.md) §3．
6. **Phase 2 の Gate X named risks**：Kernel 経由 Session `Stop(sink/rec)` test は Phase 4 で追加（`v58_13_a_session_stop_…`）．`MA-8` の強制は Phase 7 へ（Phase 4 §3）．
7. **GitHub Pages の状態表示**（https://k3komatsu.github.io/Ez-SDRv4/）は Phase 5 完了まで反映済み．その後，別エージェントが概要ページの読みやすさを改善した（`origin/gh-pages` の最新は `030e19a`「improve landing page readability」）．Phase 6 を反映するかは owner の判断（gh-pages は依頼があるときだけ変更する，AGENTS.md §1）．
8. Phase 2 のその他の test ceilings：sc16 capture と contract change 時の `partial`，KC-29 の直接 assert，到達不能な N8/N10 分岐，KA-12 marker check/acquire race の決定的 test seam，Provider→Sink datapath の nonzero drop→Manifest 経路．詳細は [implementation-notes.md](plan/phase2/implementation-notes.md) と exit tables を参照．

## 5. v3 → v4 切替で残っている作業（人間の判断が要るもの）

2026-09-26 に v4 を `k3komatsu/Ez-SDRv4` へ分離したので，default branch の切替（旧 #1），`release: published` がどの branch の workflow を使うか（旧 #4），dub registry の `~master`（旧 #5）は問題でなくなった．v3 の `Ez-SDR` は `master` と `dub.json` をそのまま持つ．残るもの：

| # | 項目 | 状態 |
|---|---|---|
| 1 | container image 名の衝突：v3 の `.github/workflows/ezsdr-build.yml`（`Ez-SDR` 側）は `release: published` で `ghcr.io/<owner>/ezsdr:latest` を push する（`matrix.uhd == DEFAULT_UHD` のとき）．ghcr の package は owner 単位なので，v4 が同じ `ezsdr` 名で image を出すと `:latest` を取り合う | 未．v4 の image 名を変えるか，v3 利用者を固定 tag（例 `:v3.0.28`）へ誘導してから v4 をリリースする |
| 2 | v3 readme の `ghcr.io/k3kaimu/ezsdr:latest` と現 owner `k3komatsu` の不一致 | 未確認．workflow は `github.repository_owner` を使うので現在の image は `k3komatsu/ezsdr` のはず |
| 3 | `Ez-SDR`（v3）の readme に v4 リポジトリ `Ez-SDRv4` への一行を足す | 未 |
