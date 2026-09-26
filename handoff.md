# Ez-SDR v4 — Handoff (2026-09-26)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `git@github.com:k3komatsu/Ez-SDRv4.git`（https://github.com/k3komatsu/Ez-SDRv4，2026-09-26 に `Ez-SDR` から変更） |
| `main` | v4（この worktree）．Phase 0–3 の受理完了．Phase 2 Gate X を owner が2026-09-25に受理し，spec 06–10 を `design/` へ移動，Vision issue 15件を適用（Step X 完了）．Step 16/Gate X/Step X は `bf3b1bf` で commit・push 済み．2026-09-25 の動的レビューで見つかったテスト欠落2件を追加テストで塞いだ（[implementation-notes.md](plan/phase2/implementation-notes.md) 末尾）．Opus `PASS_WITH_RISK` の残余リスクは §4 と `plan/phase2/00-overview.md` §11 に記録．**Phase 3 は 2026-09-26 に Gate P 受理・実装・Review C・Gate X 受理・Step X 完了**（spec 11 を `design/` へ移動，Vision issue 8件を適用）．§4 参照．|
| `master` | v3（D + C++ UHD bridge + Python client）．tip `4a474e9` = tag `v3.0.28`．**GitHub の default branch のまま** |
| `gh-pages` | GitHub Pages 用の orphan branch．2026-09-26 に空のコミット `24229a9` で作成．`main`・`master` と履歴を共有しない．push・Pages 設定は未実施 |
| tags | 35 個（`v2.11`, `v3.0.0`–`v3.0.28`）．すべて v3 系 |
| 履歴の関係 | **無関係（unrelated）**．graft も merge もしていない．v4 は clean-sheet なので今後も繋がない |

ローカル：

- `/Users/komatsu/GoogleDrive/github/Ez-SDRv4` = `main` の worktree（メイン）．
- `…/Ez-SDRv4/v3` = `master` の **git worktree**（入れ子 clone ではない）．`.gitignore` で除外．
- 設計文書が v3 のパス 22 件を証拠として引用しているので `v3/` は消さない（`design/` と Vision に加えて `plan/phase1/` からも引用がある）．確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- `v3/` を消してしまった場合：`git worktree prune && git worktree add v3 master`
- **Google Drive の同期で追跡ファイルが消えることがある**．2026-09-24 に `crates/ezsdr-kernel/tests/` と `schemas/`（61 ファイル）が作業ツリーから消え，`git restore crates/ezsdr-kernel/tests schemas` で戻した．作業を始める前に `git status --short` に ` D` 行がないことを確かめる．Drive の同期中に戻すと競合コピー（`… (1).…`）が増えるので，同期を止めてから戻す．

## 2. 設計文書の状態 — Phase 0・1・2・3 完了

- 単一の設計ソース = **Vision**：索引 [Ez-SDR_v4_ARCHITECTURE_VISION.md](Ez-SDR_v4_ARCHITECTURE_VISION.md) + [design/vision/](design/vision) の 11 part（§1–§68，番号は不変）．
- [design/v4-vision-audit.md](design/v4-vision-audit.md)（Findings 1–34，判定 READY WITH REQUIRED CHANGES）→ 全項目を Vision に反映済み．
- [design/v4-vision-rereview.md](design/v4-vision-rereview.md)（Findings R1–R22，判定 READY）→ 全項目反映済み．R13「規範部分の spec 化」は当初11ファイル分割で暫定対応したが，Phase 1 Step 5（D109，2026-09-23）で Phase 1 specs へ，Phase 2 Gate X（2026-09-25）で specs 06–10 へ反映し，完了した．
- accepted specs は Phase 1 の [design/01-time-model.md](design/01-time-model.md)〜[design/05-module-api.md](design/05-module-api.md) と Phase 2 の [design/06-kernel-coordinator.md](design/06-kernel-coordinator.md)〜[design/10-host-data-path.md](design/10-host-data-path.md)，Phase 3 の [design/11-simulation-channel.md](design/11-simulation-channel.md)（spec 12 の修正は design/04–09 に適用済み）．各 Phase の決定ログ・実装計画・rule exit evidence は [plan/phase2/](plan/phase2)，[plan/phase3/](plan/phase3) に残る．
- 旧 CMA は退役．[design/archive/](design/archive) に保管（audit / rereview の `CMA §N` 引用のためだけに残す）．編集しない．
- Vision の改訂履歴は索引ファイル末尾の表（Phase 0/1 の8 passに加え，Phase 2 Gate X の適用を2026-09-25に，Phase 3 Gate X の適用を2026-09-26に記録）．

## 3. 実装の状態

Phase 1 の Kernel crate は **実装済み**（Step 4 完了）．

| | |
|---|---|
| workspace | ルートの `Cargo.toml`（`resolver = "3"`，edition 2024，`rust-version = "1.85"`）．`Cargo.lock` は commit 対象 |
| crate | `crates/ezsdr-kernel` version `4.0.0-alpha.1`．`#![forbid(unsafe_code)]`，`#![warn(missing_docs)]` |
| module | `id time contract coordinator stream hash module_api spec binding plan event policy run session manifest schema`．`plan` は `coercion` `compile` `graph` `islands` `links` `matching` `prepare` `validation` の private submodule に分割済み |
| 直接依存 | `serde` `serde_json` `schemars` `sha2` の4つだけ（exit criterion 6）．`cargo tree -p ezsdr-kernel` の解決ツリーはKernel自身を含む28 package．workspace全体のlockはexternal 27 package + workspace member 10 package |
| test | Kernel package 443 件：`coordinator`(81) `spec_binding`(107) `stream_contract`(69) `run_session`(76) `time_model`(52) `module_api`(27) `run_doubles`(1) `hashing`(9) `kernel_surface`(15) `schema_freeze`(4) `event_hotpath`(1) `lib`(1) |
| toolchain | Rust `1.85.0` (`4d91de4e4`, 2025-02-17) / stable `1.98.1` (`48a229cea`, 2026-09-01) の両方で workspace 550 passed，0 failed，0 ignored（2026-09-25）．`cargo +stable clippy --workspace --all-targets -- -D warnings` も再通過 |
| schemas | Kernelの `schemas/` 直下に47個の JSON Schema 2020-12．Phase 2 Vocabularyの `schemas/{radio,sim,sink}/` に10個あり，合計57個 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`cargo +stable test -p ezsdr-kernel --test kernel_surface ov_23b -- --nocapture` は **115 NEW / 291 public items**（2026-09-25）．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |

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

`kernel_surface` は `116 NEW / 292 public items`（Phase 3 で Kernel の public item は 1 つだけ増えた：KB-1 の `module_api::InputStore`）．mutation は Appendix C 83/83 + Review C の fix-check 15/15 + Gate X の fix-check 4/4 killed．`cargo +stable clippy --workspace --all-targets -- -D warnings` clean，`check_links.py` 全リンク解決．Toolchain：Rust `1.85.0`（MSRV）と `stable 1.98.1` の両方で 2026-09-26 に 604 passed．

## 4. Phase の状態 — Phase 1・2・3 完了（Gate X受理）

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

### 次にすること

0. **Phase 4 の計画**：Vision §67 の Phase 4（Events, failure, continuity, artifacts）．Phase 3 は Gate X 受理・Step X 完了（2026-09-26）．上の持ち越し 2 件を範囲に含める．
1. **Phase 2 は完了**：Gate X owner acceptance と Step X を2026-09-25に完了．spec 06–10 は `design/` にあり，15件の Vision issue を適用済み．
2. **Gate X の named risks**：`MA-8` timeout enforcement は未試験，Kernel 経由 Session `Stop(sink/rec)` test は未実装，Opus review は静的のみ．
3. その他の test ceilings：sc16 capture と contract change 時の `partial`，KC-29 の直接 assert，到達不能な N8/N10 分岐，KA-12 marker check/acquire race の決定的 test seam，Provider→Sink datapath の nonzero drop→Manifest 経路．詳細は [implementation-notes.md](plan/phase2/implementation-notes.md) と exit tables を参照．

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
