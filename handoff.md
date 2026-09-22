# Ez-SDR v4 — Handoff (2026-09-22)

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

Phase 1 の Kernel crate は **実装済み**（Step 4 完了）．

| | |
|---|---|
| workspace | ルートの `Cargo.toml`（`resolver = "3"`，edition 2024，`rust-version = "1.85"`）．`Cargo.lock` は commit 対象 |
| crate | `crates/ezsdr-kernel` version `4.0.0-alpha.1`．`#![forbid(unsafe_code)]`，`#![warn(missing_docs)]` |
| module | `id time contract stream hash module_api spec binding plan event policy run session manifest schema`（00-overview.md §5 のレイアウトどおり） |
| 直接依存 | `serde` `serde_json` `schemars` `sha2` の4つだけ（exit criterion 6）．解決後のツリーは44 crate |
| test | 261 件．`run_session`(58) `spec_binding`(52) `stream_contract`(60) `time_model`(50) `module_api`(23) `hashing`(9) `kernel_surface`(8) `schema_freeze`(3) `event_hotpath`(1) `lib`(1) |
| toolchain | `1.85.0` / `stable (1.98.1)` の両方で 261 passed．`cargo clippy --all-targets` も警告ゼロ |
| schemas | `schemas/` に45個の JSON Schema 2020-12 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`NEW:` 件数が OV-23b の Kernel 成長指標．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |

Python client，wire protocol，MockRadio，Simulation Engine は未着手（Phase 2 以降）．

## 4. Phase 1 の状態 — spec 受理待ち

Vision §67 の Phase 1．詳細設計は [plan/phase1/](plan/phase1/)．計画本体と横断決定は [plan/phase1/00-overview.md](plan/phase1/00-overview.md)．

| spec | rule ID | 状態 |
|---|---|---|
| [01-time-model.md](plan/phase1/01-time-model.md) | TM-1..21（副番含め36） | Gate A 通過，実装済み |
| [02-stream-contract.md](plan/phase1/02-stream-contract.md) | SC-1..32（副番含め48） | Gate A 通過，実装済み |
| [03-spec-and-binding.md](plan/phase1/03-spec-and-binding.md) | SB-1..49（副番含め54） | Gate B 通過，実装済み |
| [04-run-and-session.md](plan/phase1/04-run-and-session.md) | RS-1..52（副番含め57） | Gate B 通過，実装済み |
| [05-module-api.md](plan/phase1/05-module-api.md) | MA-1..46 | Gate C 通過，実装済み |

6文書で259ルール，欠番と未解決参照なし，撤回5件（SB-28, SB-32, RS-37, MA-4, MA-43 — OV-1 に従い番号は保持）．Gate A は敵対的レビュー3巡（31件・16件・13件），Gate B は2巡（19件・10件），Gate C は1巡（8件）．

**Step 4（crate 実装）は完了**．00-overview.md §3 の表の module 順どおりに実装し，各 rule ID を doc comment と test 名から引用している（OV-3）．

実装後に **Opus サブエージェントによる敵対的コードレビューを3巡**（AGENTS.md §8）：

| 巡 | 範囲 | 指摘 | 結果 |
|---|---|---|---|
| 1 | spec 01 + 02 + hash | P0 1件，P1 5件，P2 6件 | 全件修正．P0 は非有限 float が `null` と同一ハッシュになる衝突（OV-15a が防ぐために書かれた失敗そのもの） |
| 2 | spec 03 + 04 + 05 + D3/D5 | P0 3件，P1 12件，P2 9件 | 全件修正．P0 は「Session が一切 compile できない」「matcher が sub-resource を二重予約」「未知 source の `DEVICE_LOST` が abort を黙らせる」 |
| 3 | 1・2 の修正の検証 | 20/28 clean，5 partial，新規 P0 1件 + P1 3件 + P2 7件 | 全件修正．新規 P0 は `Value::Num` が同じ非有限の穴（1巡目の修正が4つのうち3つしか塞いでいなかった） |

レビューの主要な発見は「pipeline の各段が関数としては正しいのに誰も呼んでいない」型の欠陥だった（`admit_islands`・`check_cycles`・`check_sink_links`・`CheckStage::Prepare` が全て未接続）．現在は `plan()` と `collect_prepare()` から呼ばれる．`kernel_surface` は3巡目に2通りの回避を実演されたので，brace 深さで module 階層を追う方式に書き換えた（inline `mod` と private `mod` + `pub use` の両方を検出．回避の再現は scratch copy で確認済み）．

exit criteria（§13）の達成状況：

| # | 条件 | 状態 |
|---|---|---|
| 1 | 6文書の受理と §11 の全 verdict | **未**．X1–X12・各 spec の Decisions 表・未決事項1–7 はユーザ判定待ち |
| 2 | 全 rule に ID と OV-3 disposition | 未（exit review の作業） |
| 3 | MSRV と stable で `cargo test` 通過，`#[ignore]` なし，pipeline が double で端から端まで動く | **達成**（1.85.0 / stable ともに 261 passed）．Session path は `validate` → `plan` まで通る（`rs_12_a_session_compiles_through_the_whole_pipeline`） |
| 4 | `schemas/` commit，`schema_freeze` 通過，`SCHEMA_CHANGELOG.md` の v1 entry | **達成** |
| 5 | `kernel_surface` 通過，`NEW:` 件数の記録 | **達成**．件数は `cargo test --test kernel_surface -- --nocapture` が `OV-23b: Kernel growth = N NEW: items of M public items` で出す |
| 6 | 直接依存が §8 の4 crate ちょうど | **達成** |
| 7 | §12 の移動後に `design/` と `plan/phase1/` の全リンクが解決 | 未（Step 5 の作業） |

**次にユーザがやること**が2つある：

1. **§11 の decision log を埋める**（exit criterion 1）．X1–X12，各 spec の Decisions 表，未決事項1–7 の verdict が空のまま．
2. **findings 30件に verdict を出す**．00-overview.md §11 に3つの表がある：「Findings from Step 4」（D1–D14，実装中に出たもの），「Findings from the Step 4 code review」（D15–D26，レビュー1・2巡で出たもの），「Findings from the verification pass」（D27–D30，3巡目）．spec を黙って直さず記録してある．

   このうち **D3 と D5 だけは spec 本文を修正済み**（ユーザの事前承認による）．3巡目が両方とも spec の記述ミスと確認した：
   - `01-time-model.md` TM-21 の「2^124」→ Derived domain の公称レート項は capped 項の**積**なので 2^62 に達し，交差積は 2^186．隣の「193 bits」段落と同じ議論で，checked 128-bit が上限である
   - `04-run-and-session.md` §4 の「four of RS-27」→ five（+ EventKind の文法も同じ行で訂正）．同じ古い数字が 04 §9 と 00-overview.md §3 にもあり，そこでは Vision §29 の解釈も誤っていた（§29 の12個のうち Kernel が残すのは**2個**）

   **判断が要る主なもの**：D17（Spec Run では Sink を特定する手段が spec 上存在しない — SC-21 / SB-15 / SB-17 が Spec path で強制できない），D27（SB-34 は node の排他を定めていない），D15（SC-30c の本文と §6 の擬似コードが矛盾）．

その後が Step 5（§12 の受理手続き：spec 01–05 を `design/` へ移動，R13 の Vision 編集はユーザの別途承認が要る）．

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

## 6. 決めたことの記録（理由）

### 2026-09-22（Step 4: crate 実装 + コードレビュー3巡）

- MSRV は X12 どおり **1.85 のまま**．edition 2024 の let-chain は 1.88 以降なので，8箇所を nested `if let` に書き直した（§11 D13）．spec を動かすより code を直すほうが小さい．
- `Constraint` / `CapabilityValue` / `OutputSource` は struct variant．OV-13 の internal tag は payload が map でない newtype variant を serde が扱えない（§11 D1）．
- `kernel_surface` の allow-list（279行）は「Core remains small」のレビュー用チェックリストそのもの．`NEW:` 110件が OV-23b の成長指標．
- banned token の検査は OV-23 の文面どおり行単位で，`OV-23a` を明示した行だけ免除．UHD の証拠引用7箇所には marker を付け，`replay` / `taint` の散文4箇所は語を替えた（§11 D9）．
- `EventCollector::new` で ring の mutex を1度 lock して捨てる．macOS では最初の lock が OS primitive を box するので，RS-32 の allocation 計測が1件だけ外れていた（§11 D11）．
- **非有限 float は serialisation 時に拒否する**（`hash::serialize_finite_f64`）．`serde_json` は非有限を `null` に落としてから `Value` を作るので，canonicaliser には情報が届かない．Kernel が持つ float document field 4箇所すべてに付けた：`ClockRelation.drift` / `.drift_uncertainty` / `Scalar::Float` / `Value::Num`（§11 D1 の P0，3巡目の P0）．
- **pipeline の段は関数ではなく段として繋ぐ**．`admit_islands` / `check_cycles` / `check_sink_links` / `CheckStage::Prepare` / SB-46 の coercion policy は，どれも正しい関数として存在しながら誰も呼んでいなかった．`plan()` と `collect_prepare()` の中に入れたのは，呼び忘れを構造的に不可能にするため．
- **`kernel_surface` は brace 深さで module 階層を追う**．列0の `pub` だけを見る実装は，inline `pub mod` に何個でも隠せて（allow-list 1行で足りる），private `mod` + `pub use` なら0行で済んだ．3巡目に両方実演されたので書き換え，scratch copy で再現を確認した．
- **exclusivity は spec が定めていない**．SB-34 は「異なる sub-resource に bind **してよい**」とだけ言う．排他を要求すると満たせる binding を拒否し，自由にすると1本のチャネルを2つに渡す．実装は「異なる node を優先し，他に無いときだけ共有」（§11 D27）．

### 2026-09-21（v4 を同じ repo へ）

- 別 repo だった v4 を同じ repo の `main` に収めた．v4 は commit 1 個・remote なしだったので history rewrite も force push も不要だった．
- 履歴を繋ぐ選択肢（graft，rebase，`--allow-unrelated-histories`，`merge -s ours`）はすべて不採用．存在しない系譜を記録するだけだから．
- 入れ子 clone だった `v3/` は，未 push commit・ローカル限定 branch / tag・stash がゼロであることを確認してから削除し，worktree に置き換えた．
