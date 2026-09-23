# Ez-SDR v4 — Handoff (2026-09-23)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．設計の中身は書かない．どこに何があり，何が終わっていて，次に何をするかだけ．
開発時の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. リポジトリ

| | |
|---|---|
| remote | `git@github.com:k3komatsu/Ez-SDR.git` |
| `main` | v4（この worktree）．commit 16 個．Phase 0 の設計文書3個のあと，Phase 1 の spec と `ezsdr-kernel` crate（`79334fc`）とレビュー適用が続く．`origin/main` と同期 |
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
| 直接依存 | `serde` `serde_json` `schemars` `sha2` の4つだけ（exit criterion 6）．解決後のツリーは `Cargo.lock` で28 package（crate 自身を含む） |
| test | 345 件．`spec_binding`(99) `stream_contract`(69) `run_session`(72) `time_model`(49) `module_api`(26) `hashing`(9) `kernel_surface`(15) `schema_freeze`(4) `event_hotpath`(1) `lib`(1) |
| toolchain | `1.85.0` / `stable (1.98.1)` の両方で 345 passed（2026-09-23，D108 適用後）．`cargo +stable clippy --all-targets --all-features -- -D warnings` も通過 |
| schemas | `schemas/` に47個の JSON Schema 2020-12 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`NEW:` 件数が OV-23b の Kernel 成長指標．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |

Python client，wire protocol，MockRadio，Simulation Engine は未着手（Phase 2 以降）．

## 4. Phase 1 の状態 — spec 受理待ち（binding / role の規範を固め直し済み）

Vision §67 の Phase 1．詳細設計は [plan/phase1/](plan/phase1/)．計画本体と横断決定は [plan/phase1/00-overview.md](plan/phase1/00-overview.md)．

| spec | rule ID | 状態 |
|---|---|---|
| [00-overview.md](plan/phase1/00-overview.md) | OV-1..23b（副番含め27） | criterion 2 の per-rule 表を含め更新済み |
| [01-time-model.md](plan/phase1/01-time-model.md) | TM-1..21（副番含め31） | Gate A 通過，実装済み |
| [02-stream-contract.md](plan/phase1/02-stream-contract.md) | SC-1..32（副番含め47） | Gate A 通過，実装済み |
| [03-spec-and-binding.md](plan/phase1/03-spec-and-binding.md) | SB-1..49（副番含め60） | Gate B 通過，実装済み．SB-22 を SB-22a〜h と表 SB-T0〜T4 に分割（D95・D96） |
| [04-run-and-session.md](plan/phase1/04-run-and-session.md) | RS-1..52（副番含め60） | Gate B 通過，実装済み．RS-25a を追加（D103） |
| [05-module-api.md](plan/phase1/05-module-api.md) | MA-1..46（副番含め52） | Gate C 通過，実装済み．MA-16a を追加（D100） |

6文書で277ルール，欠番と未解決参照なし．撤回7件（SB-25a, SB-28, SB-32, RS-32a, RS-37, MA-4, MA-43 — OV-1 に従い番号は保持）．Gate A は敵対的レビュー3巡（31件・16件・13件），Gate B は2巡（19件・10件），Gate C は1巡（8件）．

**Step 4（crate 実装）は完了**．00-overview.md §3 の表の module 順どおりに実装し，各 rule ID を doc comment と test 名から引用している（OV-3）．

実装後に **Opus サブエージェントによる敵対的コードレビューを3巡**（AGENTS.md §8）：

| 巡 | 範囲 | 指摘 | 結果 |
|---|---|---|---|
| 1 | spec 01 + 02 + hash | P0 1件，P1 5件，P2 6件 | 全件修正．P0 は非有限 float が `null` と同一ハッシュになる衝突（OV-15a が防ぐために書かれた失敗そのもの） |
| 2 | spec 03 + 04 + 05 + D3/D5 | P0 3件，P1 12件，P2 9件 | 全件修正．P0 は「Session が一切 compile できない」「matcher が sub-resource を二重予約」「未知 source の `DEVICE_LOST` が abort を黙らせる」 |
| 3 | 1・2 の修正の検証 | 20/28 clean，5 partial，新規 P0 1件 + P1 3件 + P2 7件 | 全件修正．新規 P0 は `Value::Num` が同じ非有限の穴（1巡目の修正が4つのうち3つしか塞いでいなかった） |
| 4 | §11 の全判定項目 112件（Fable 5.1，AGENTS.md §8 の second opinion 例外） | confirm 91・amend 15・reverse 1・not-a-decision 5 | 全件をユーザ判定として §11 に採用．表に無い発見14件のうち5件（D31–D33 + D17/D18 の改訂）を §11 に追記 |
| 5 | crate 全体（`e8d5caa` の diff 重点） | P0 1・P1 11・P2 12 | 全件修正．P0 は `validate()` が RF envelope 違反を報告しても `plan()` が arm 可能な plan を返していた（`AdmissionResult::into_result` に src 内の呼び出し元ゼロ） |
| 6 | 5巡目の修正の検証＋修正への攻撃 | P0 1・P1 5・P2 11 | 全件修正．**P0 は5巡目の修正が入れた回帰** — MA-12 の再マッチが `merged.effective` を読み，2チャネル Spec が互いを拒否 |
| 7 | 独立レビュー（前2巡が見ていない領域） | P0 1・P1 7・P2 12 | 全件修正．**P0 はまた `collect_prepare`** — 再マッチを全 key に適用し，coercion を構造的に必ず拒否．SB-46 の `accept`/`warn` が到達不能だった |
| 8 | 2点集中（coercion 除外と surface gate） | P0 1・P1 2・P2 9 | 全件修正．P0 は除外を Provider 自己申告の `coercions` で判定していた件（key を並べれば MA-12 を自己免除でき，Manifest が適用値と違う値を記録した） |
| 9 | 8巡目の修正の検証 | **P0 ゼロ**・P1 2・P2 9 | `collect_prepare` から4巡ぶりに指摘なし．P1 2件はどちらも私の書いた箇所（ZST Provider でのポインタ判定，gate の残穴5通り）で修正済み |
| 10 | crate 全体の second opinion（Fable 5.1，AGENTS.md §8 の例外．全件を scratch copy で実行済み） | P0 1・P1 6・P2 10 | 適用済み．**P0 はまた `collect_prepare`，しかも7巡目の修正の中**（下記）．P1 のうち2件は直前コミット `13c4070` が古いブロックを置換せず追加していた件 |
| 11 | 10巡目の修正の検証（Opus，大規模修正のため必須の再レビュー） | P0 2・P1 1・P2 2 | 全件修正．**P0 2件はどちらも10巡目の修正自身**：RS-52 が component の `params` を先に見ていた（SB-2 が「the only place it can be … never a `ComponentDescriptor`'s `params`」と明記）と，SC-30b の繰り越しが `finish` で抜けていた．P1 は既存で，`collect_prepare` が他 Spec の `AdmissionResult` を拒否していなかった件 |
| 12 | 11巡目の修正の検証（新たな拒否を2つ足したため） | P1 1・P2 1 | 修正済み．P1 はまた11巡目の修正自身 — SC-30b の carry は flags・lost・blocks の3つを運ぶのに，繰り越しが blocks しか持っていなかった |
| 13 | `continuity.rs` に絞った検証（12巡目の作り直しが未レビューだったため） | **P0 ゼロ・P1 ゼロ**・P2 2 | 修正済み．差分 fuzz で lossless 経路は HEAD とビット同一，Gap の `link_dropped` と `lost` の総和が投入分と一致することを確認．**ここで収束** |
| 14 | D45–D50 の判断（Fable 5.1，AGENTS.md §8 の second opinion 例外） | 6件 | **全件採用・適用済み**．D50 は判断ではなく**バグの発見**だった：10巡目で入れた canonical-form 基準が `Int(1152921504606847000) == Num(2^60)` を true にしていた（float の canonical 形は「その f64 を名指す最短十進」であって正確な値ではない）．等価性は厳密な数値比較に戻した．新規ルール SB-15a（ポート方向），`CompileRule::TxBurst` に `late_policy`（schema 変更） |
| 15 | exit criterion 2 の per-rule 表（Sonnet 5 を文書ごとに1体，計6体．AGENTS.md §8 の「機械的調査 → Sonnet 5」） | レビューではなく初回261ルールの carrier 調査 | 初回表の副産物は未使用 predicate / field 15件，自己検査していない checker 6件，`GAP` 15件．D51–D68 の採用で表を266ルールへ更新し，`GAP` と未割当を解消（2026-09-23） |
| 16 | D51–D68 適用の再レビュー（Opus 5.5） | P0 1・P1 8・P2 十数件 | **正当な文書の誤拒否はゼロ**．P0 は `Manifest.policy` が必須だったこと（validate で落ちた Run に書ける値がない）．gate の無断の緩和，テストのない拒否2件，移行していない文書の出自記録を修正 |
| 17 | 16巡目の判断12件（Fable 5.1，AGENTS.md §8 の second opinion 例外） | 12件 | **全件採用・適用済み**（D69–D80）．うち6件が schema 変更．D76 は同日採用の D67 の key 文法を覆した |
| 18 | D69–D80 適用の再レビュー（Opus 5.5） | P0 0・P1 2・P2 8 | **正当な文書の誤拒否はゼロ**．P1 は2件とも D73 の不完全（多行 derive の取りこぼしと，serde が unit variant に `deny_unknown_fields` を適用しないこと） |
| 19 | 18巡目の判断4件（Fable 5.1） | 4件 | **全件採用・適用済み**（D81–D84）．D82 は引数を削る形に簡素化して適用 |
| 20 | D81–D84 適用の再レビュー（Opus 5.5） | P0 0・P1 1・P2 10 | **正当な文書の誤拒否はゼロ**．P1 は MA-16 の gate が inherent メソッドとジェネリクスを辿っていなかったこと |
| 21 | 20巡目の判断3件（Fable 5.1） | 3件 | **全件採用・適用済み**（D85–D87）．D87 は Session の既存バグ（複数役割 Module が bind 不能）の修正 |
| 22 | D85–D87 適用の再レビュー（Opus 5.5） | P0 0・P1 2・P2 7 | **正当な文書の誤拒否はゼロ**．P1 は Authority が resource でない binding を指せることと，MA-16 の gate が trait impl を辿らないこと |
| 23 | 22巡目の判断4件（Fable 5.1） | 4件 | **全件採用・適用済み**（D88–D91） |
| 24 | D88–D91 適用の再レビュー（Opus 5.5） | P0 0・P1 3・P2 7 | **正当な文書の誤拒否はゼロ**．P1 は D88×D89 の矛盾，gate の self type，SB-36 の need |
| 25 | 24巡目の判断3件（Fable 5.1） | 3件 | **全件採用・適用済み**（D92–D94）．ここでループを一時停止 |

このクレートが繰り返し出した欠陥型は2つある．

**「正しい関数を誰も呼んでいない」** — 13巡で計8件が見つかり，exit criterion 2 の初回 sweep では未使用 predicate / field が15件に達した（2026-09-22 時点）．D51–D68 の採用で不要な8件を削除し，必要なものをテストまたは admission 経路へ接続した．`DataContractId::as_str` は現時点で呼び出し元がないが，低コストの識別子 accessor として意図的に残している．初回 sweep の経緯は §11「Findings from the exit-criterion-2 sweep」参照．

**「ルールを初めて生かすと，そのルールが意図しないものを拒否する」** — P0 5件のうち4件がこれで，4件とも `collect_prepare` の中，しかも毎回別方向（merge を resource 別に解釈／coercion を構造的に拒否／Provider の自己申告で免除／coercion preview を key だけで引く）．

4件目は**7巡目の修正そのものの中**にあった．除外を Kernel 自身の `coercions_preview` に向けたところまでは正しかったが，`Coercion` は `{key, requested, applied, reason}` でリソースを持たない — つまり **そのVecをkeyで引く限り per-resource にはなり得ない**．同じ key を別デバイスで制約する2チャネル Spec で，片方が coerce されると，直接満たされた（`coerce` を呼ばれてすらいない）もう片方が「`coerce` が返した値を適用していない」として拒否された．3巡連続で同じ関数の同じ構造を別角度から踏んでいる以上，次に `collect_prepare` を触るときは**まずデータが判定に必要な情報を持っているかを確認する**こと．Kernel 自身の記録は `PreviewedCoercion { resource, coercion }` になり，隣の `RejectedConstraint` と同じ形になった．

**「判定を適用する作業そのものが欠陥を入れる」** — 11巡目の P0 2件が10巡目の修正自身の中にあり，同種は7→8巡目，5→6巡目にもあった（計3回）．**修正を書いたら，その修正が参照する条文をもう一度読むこと**：RS-52 の件は SB-2 に「and the only place it can be」と書いてあり，読めば component を見に行く発想が出ない．

**「置換せず追加してしまう」** — 10巡目の P1 2件．`13c4070` は D44 の binding-description ブロックを**追加**したが，D44 が「削除した」と書いたポインタ比較ブロックはファイルに残ったままで，両方が走っていた（SB-22 の `sink` チェックも二重）．古いブロックは D44 が許す形（1つの description に対し2つのオブジェクトが同じ `instance().id` を報告する）を拒否する．**判定を適用したあとは，置換対象が消えたことを grep で確認すること．**

`kernel_surface` は5巡で5組の回避を実演された．いずれも「**接頭辞照合は綴りであり，綴りは改行で割れる**」という同じ形だったので，接頭辞照合を全廃した：宣言は行と次行に跨る**トークン列**として読み，lex できない構文は**トークンとして拒否**し，その上に「スキャン自身が frame 均衡で終わったこと」を assert する（brace クラスを綴りでなく原因で閉じる）．**実演された11通りすべてを scratch copy で捕獲確認．**この gate が macro 経由で隠していた public item が2件あった（`id.rs` の4つの id 型と `schema::document_schemas`）ので，X11 の「allow-list が review checklist である」は事実として偽だった．

その後 `syn::parse_file` による parser に置き換え（D34–D44 の採用，`13c4070`）たが，10巡目がさらに**5通り**を実演した．うち1つは**出荷されるクレートで現に穴が開いていた**：`is_testing_gated` が `cfg` のトークン列に `testing` が*含まれるか*で判定していたため，`#[cfg(not(feature = "testing"))]`（＝デフォルトビルド）の public item を gate が丸ごとスキップしていた．他は `pub use {…}`（brace group が root を名乗らない），`pub extern crate`（match の catch-all に落ちる），非 `.rs` ファイルの `include!`，同名 item を持つ inline module 2つが1つの allow key に潰れる（key が名前でなく深さを符号化していた）．**5通りすべて注入して捕獲を確認済み．**

exit criteria（§13）の達成状況（2026-09-23 時点）：

| # | 条件 | 状態 |
|---|---|---|
| 1 | 6文書の受理と §11 の全 verdict | **verdict は D1–D108 まで記録・適用済み**．6文書の受理そのものは未実施（§12） |
| 2 | 全 rule に ID と OV-3 disposition | **277ルールに277行を用意し，GAP と未割当を解消**．内訳は default 207・process 22・producer 13・forward 11・withdrawn 7・consumer 1・分割 16（default+forward 12，default+producer 3，forward+producer 1）．`UNCERTAIN:` は D108 でゼロ **（達成）** |
| 3 | MSRV と stable で `cargo test` 通過，`#[ignore]` なし，pipeline が端から端まで動く | **達成**（2026-09-23，1.85.0 / stable 1.98.1 ともに345 passed，`#[ignore]` なし）．end-to-end pipeline ケースも通過 |
| 4 | `schemas/`，`schema_freeze`，`SCHEMA_CHANGELOG.md` | **達成**．D51–D68・D95–D103・D104 の schema 変更を反映済み（`pattern` は無し） |
| 5 | `kernel_surface` 通過，`NEW:` 件数の記録 | **達成**．`109 NEW / 281 public items`（2026-09-23，`plan::check_sink_links` を D99 で削除） |
| 6 | 直接依存が §8 の4 crate ちょうど | **達成** |
| 7 | §12 の移動後に `design/` と `plan/phase1/` の全リンクが解決 | 未（spec 受理後の Step 5） |

**§11 の判定は完了した**（2026-09-22，Fable 5.1 の second opinion をユーザ判定として採用）．
verdict は confirm 91・amend 15・reverse 1・not-a-decision 5．not-a-decision の5件（OQ4,
D1, D6, D11, D14）は encoding の帰結と fixture note で，裁定を要しない．

**bin (i)–(iii) はすべて適用済み**（記録として残す）．second opinion は当時，適用可否を3つに分けていた：

| bin | 内容 | 状態 |
|---|---|---|
| (i) prose / コードのみ，rule text を変えない | D15・D24・S1・B4・OQ4 | **適用済み**（2026-09-22） |
| (ii) rule text を変えるが freeze 前で自己完結 | OQ2・R10・D8・D16・D22・D23・D25・D26・D27・D32・D33，および D17 + D18 + D31 + D29 のクラスタ | **適用済み**（2026-09-22） |
| (iii) 待ち | confirm が含意する Vision 編集（OV-6 のため §12 手続き），および Phase 2 の値を要する2件（resource port の producer 側 memory domain，port contract と format coercion の関係） | 未（§12 / Phase 2） |

**§11 の verdict は D1–D50 まで全件記録済み**（D34–D44 と X11，および D45–D50 は 2026-09-22 の Fable 5.1 second opinion を採用）．採用に伴い次を適用した：`Value` の cross-kind 等価・比較を正確化（D34），`to_root` が非整数 tick の schedule を拒否（D35），`BurstStep::Discontinuity.then_ended`（D36），schedule entry の `target` 検査と「Spec target は Spec 相対」の明文化（D37），OV-3 に4つ目の marker と exit criterion 2 を per-rule 表へ（D38/D39），`constraints_hit` 削除（D40，schema 変更），`coerce` は node あたり1回・map 全体で呼ぶ（D41），要求していない key の coercion は malformed report（D42），`SessionLog::append` が不正な Action を拒否（D43），**instance identity は binding description**（D44，ポインタ比較を廃止）．

**X11 は半分だけ反転した．** `cargo public-api` の却下は維持し，`syn` ベースのスキャナの却下を撤回して `kernel_surface` を `syn::parse_file` に載せ替えた．決め手は**このプロジェクトが実際に回すループの中での失敗の形**：手書き lexer は**黙って**失敗し（5巡で11通り，うち2つは出荷中の public item を隠していた），`cargo public-api` は大声で失敗するが `cargo test` の**外**（X12 が管理しない nightly．CI が無い段階では「動かない gate」は「省かれた gate」），`syn` は MSRV toolchain 上の `cargo test` の**中**で大声で失敗する．OV-18 を当てると `syn` は全条項を満たす — 手書きの代替は709行，MSRV 1.71，**新規 transitive crate ゼロ**（`serde_derive`/`schemars_derive` 経由で既に解決済み，`Cargo.lock` の差分は1行）．X11 はこの代替を記録していながら，自分で書いた OV-18 を当てずに却下していた．gate は 709 → 499 行になった．ただし **parser にすれば終わりではなかった**：10巡目が parser 自身に5通りの穴を実演し，うち1つ（`is_testing_gated` が `cfg` のトークン列に `testing` が*含まれるか*で判定していた件）は**出荷ビルドで現に開いていた**．手書き lexer の穴が「綴りの見落とし」だったのに対し，parser の穴は**述語のロジック**で，parser 化では消えない種類だった．現在の gate は617行で，実演された20通りすべてを捕獲する（1つは `#![forbid(unsafe_code)]` がより手前で拒否）．3つの述語には直接の負テストが付いている．

**コードレビューは13巡で収束した**（5–13巡目は crate に対する敵対的レビュー）．13巡目で P0 ゼロ，`collect_prepare` からも4巡ぶりに指摘なし．9巡目の結論は「**crate は収束．gate は収束していないが，それは足し算では収束しない**」で，`kernel_surface` の残る問いはコードではなく**道具の選択**：

> **`cargo public-api` を採るか — 決着済み（2026-09-22）．** 却下を維持し，代わりに `syn` ベースの parser を採った（X11 の半分反転，上記）．ただし**parser にしても終わりではなかった**：その後のパスが parser 自身に5つの穴を実演し，それは lexing の見落としではなく**述語のロジック**の誤りだった．現在の gate は3つの述語に直接の負テストを持つ．

§11 の verdict は D1–D50 まで**全件記録済み**で，未適用の判定は残っていない．

bin (ii) で構造が変わった点（spec 03/04/05 と crate，schema 7件を再生成）：

- **Sink は bind するもので place するものになった**（D17）．`outputs[]` が `feed: SinkFeed { port, policy, capacity }` を持ち，`bindings` が resource 名・output id・Island の executor 名を1つの namespace で束ねる（SB-22）．`Binding.provider` → `module`，`ComponentPlacement.module` は削除，**SB-25a は撤回**（OV-1 に従い番号は保持）．`plan::sink_components` と `session::check_placement_modules` も削除で，Kernel surface は純減．bound Sink は `ResourceId { node: LOCAL, path: <output id> }` で addressable．
- **Executor Module は文書が指す**（D18）．`bindings[island.executor]` から解決し，runtime は descriptor だけ渡す．
- **Resource が `ports` を宣言する**（D31）．resource endpoint は Island の外側の node として扱い，MA-39 は placement を要求しない．
- **`plan()` が `AdmissionResult` を取る**（D32）．Provider fragment の `content` は `{ selector, requested }`．
- **`Resource.shareable`**（D27，既定 false）．排他 node の二重 bind は `NodeAlreadyBound` で両者を名指しして拒否．
- **`UpdateParameter.at`**（OQ2，全 class で optional）．`KeyDecl.update_class`（D33）．Manifest の `version`（R10）．`Scalar` は untagged（D23）．`BurstTracker::set_late`（D8）．`ChannelMask`/`BlockFlags` の内部フィールドは `pub(crate)`（S1）．

判定の根拠は OV-5（gate 後の不一致は §11 に verdict を記録してから適用する）と OV-12（freeze は
v4.0 以降．crate は `4.0.0-alpha.1` なので公開面はまだ凍っていない）．よって「normative text を
変えるから §12 まで待つ」という項目は1つもない．

**(ii) のクラスタ**（D17・D18・D31・D29，D32・D33 も同じ欠陥）は，spec 03/04/05 と
`binding.rs`・`spec.rs`・`module_api.rs`・`plan.rs`・`session.rs`，新規型 `SinkFeed` 1つ，schema
再生成3件，既存テスト約12件の修正と新規6–8件 — 概算 400–700 行．second opinion の結論は
「**Phase 1 の Step 5 より前にやるべき**」：Phase 1 の定義（00 §1「Phase 2 は admission と
module 境界について何も決めずに MockRadio を書ける」）が admission と module 境界の決定を
Phase 1 に要求しており，先送りすると audit §14.3 の失敗（Mock の実装が契約になる）を
v1 schema に消費者がついた状態で再現する．

---

### Phase 1 に残っているもの（2026-09-23 時点）

Step 4（crate 実装），D51–D68・D69–D80・D81–D84・D85–D87・D88–D91・D92–D94 の採用は完了．レビューループは owner の判断で一時停止し，binding と role の規則を先に書き下ろした（D95–D103，下記）．その適用への適合性レビューも済み，残っていた raised 4件も D104–D107 で適用した．次は exit criterion 2 の部分カバー注記を受理レビューで確認し，6つの Phase 1 文書を受理すること．

#### D51–D68 — 採用・適用済み（2026-09-23）

推奨と判断理由は [00-overview.md §11](plan/phase1/00-overview.md) に記録．主な変更：

- `HotLayout` と `MemoryDomain` を公開面から削除し，未使用 API 8件を除去．
- `SpecSection::migrated` に元の version / body を渡す経路を追加し，provenance を記録．
- `PrepareContext.components` に当該 Island の `ComponentDescriptor` を渡す．`Manifest` に compiled `Policy` を記録．
- `LinkPlacement` を選択した Link descriptor と policy で admission 検証し，v4.0 では `cross_process: true` を拒否．（`link-{i}` の key 文法は D76 で端点方式に置き換え）
- OV-3 の process / producer / forward marker と split rule を更新し，`SC-7` / `MA-16` の API gate，schema freeze，migration，RunId，Policy，cleanup assertion 等の carrier を追加．JSON Schema と changelog も更新．

exit criteria は [exit-review/README.md](plan/phase1/exit-review/README.md) が最新．267ルールの disposition はすべて割当済み（default 215・process 20・producer 13・forward 10・withdrawn 7・consumer 1・分割 marker 1）で，`GAP` と未割当はゼロ．部分カバーの `UNCERTAIN:` 注記が37セルあり，criterion 2 の受理判断で確認する．

#### D69–D80 — D51–D68 適用の再レビューと，その判断（2026-09-23，採用・適用済み）

D51–D68 の未コミット適用を Opus 5.5 が敵対的に再レビューした（P0 1・P1 8・P2 十数件）．**正当な文書を誤って拒否する変更はゼロ**．判断不要のものはそのまま直し，判断が要る12件は Fable 5.1（AGENTS.md §8 の second opinion 例外，owner の依頼）に推奨を出させて全件採用した．理由と却下案は [00-overview.md §11](plan/phase1/00-overview.md)「Re-review of the D51–D68 application, and D69–D80」．

判断不要で直したもの：
- **banned-token gate が誰の判断もなく緩められていた**（`///` に `OV-23a` と書けば次の `pub` 行が免除）．同一行の形に戻し，免除を削除．`/// see OV-23a` 付きの `pub fn uhd_open_rfnoc` が捕まることを確認．
- **MA-28 の拒否2件にテストがなかった**（policy 不一致，admission 側の `cross_process`）．テストを足し，どちらを無効化しても落ちることを確認．
- **`SpecSection::migrated` が移行していない文書にも出自を記録していた**．2つの文書から version を読む形にした．
- spec の形（`Policy`，`PrepareContext.components`，`BurstTracker`）と changelog の記述をコードに合わせた．

判断して変えたもの（**schema に効くものは太字**）：
- **D69** `Manifest.policy` を `Option` に（validate で落ちた Run は Policy を持たない）．
- **D73** 全文書型に `deny_unknown_fields`（入れ子の未知フィールドも拒否．schema は `additionalProperties: false`）．
- **D75 / D76** `LinkPlacement { link, from, to }` を端点で識別し，**Sink の feed も data link として Link 選択の対象に**．`ExecutionPlan.links` に feed を graph links の後ろの番号で載せる．**D67 の `link-{i}` 文法を覆した**（並べ替えで Link が黙って付け替わるため）．`graph.links` の同じ `(from, to)` の重複を SB-15 で拒否．
- **D78** `Binding.module` と `Fragment.instance` を `ModuleRef`（バージョン固定）に．Provider インスタンスの `instance().module` と食い違えば拒否．
- D70 RS-11 を stage に依存しない性質として言い直し，空振りしていたテストを置き換え．D71 MA-19 を ABI の形として言い直し．D72 MA-16 の gate をシグネチャが名指す型のフィールドまで推移的に検査（`PrepareContext` に `&dyn Sink` を足す回避を捕獲）．D74 MA-28 を MA-28 / **MA-28a**（producer）に分割．D77 Island をまたぐ link にも `connects` を検査．D79 MA-27a に descriptor の一致検査を明記．D80 MA-17 に producer marker（exit-review の「D38 が変換した」は誤りだった）．

#### D81–D84 — 2回目の再レビューと，その判断（2026-09-23，採用・適用済み）

D69–D80 の適用を Opus 5.5 がもう一度レビューした（P0 ゼロ・P1 2・P2 8，**正当な文書の誤拒否ゼロ**）．新しい拒否13件を1件ずつ無効化し，テストが落ちなかったのは SB-25 の "declared twice" だけだった．

判断不要で直したもの：
- **D73 が2通り不完全だった**．(1) `deny_unknown_fields` を足したスクリプトが1行の derive しか拾わず，ID 4型と `Version` が漏れていた（`pre: "rc1"` 付きの version が release として読めた）．(2) serde は内部タグ付き enum の **unit variant** にこの属性を適用しないため，`{"kind":"completed","stage":"validate"}` が `Completed` として読めた．該当する25個（11 enum）を空の struct variant `Completed {}` に変えた．schema とシリアライズ結果は不変．
- MA-16 の gate に残っていた3経路（supertrait，型エイリアス，role 以外の trait のメソッド）を塞いだ．3つとも注入して gate が落ちることを確認．
- Session の `implicit_spec` が未登録バージョンの binding を黙って捨てていたので拒否に．"declared twice" に直接のテスト．古い記述を整理．

判断して変えたもの（Fable 5.1 の推奨を全件採用）：
- **D81** `SinkDescriptor.memory_domains` を追加し，feed も Sink の domain に対して `connects` を検査（schema）．
- **D82** Executor / Sink / Link の descriptor に `module: ModuleRef`．admission が binding と突き合わせる（Executor は `plan()`，Sink は `validate` と `plan()`）（schema）．Fable 案は `register_link_descriptor(module, descriptor)` に不一致検査を足すものだったが，**引数を削って不一致を表現不能にする形に簡素化**した．
- **D83** graph component 名を SB-22 の名前空間に加え，output id の重複を validate で拒否（component が resource を隠す第3の衝突も解消）．
- **D84** SB-9 のエラー型が深さで分かれることは許容し，本文に明記．

#### D85–D87 — 3回目の再レビューと，その判断（2026-09-23，採用・適用済み）

D81–D84 の適用を Opus 5.5 が再レビューした（P0 ゼロ・P1 1・P2 10，正当な文書の誤拒否ゼロ）．

判断不要で直したもの：
- **P1**：MA-16 の gate が型の **inherent メソッド**（`impl PrepareContext { fn peer() -> &dyn Sink }`）と，構造体・エイリアスの**ジェネリクス**（`dyn Sink` を境界やデフォルトに持つもの）を辿っていなかった．両方を辿るようにし，3経路とも注入して gate が落ちることを確認．supertrait の赤テストが gate の処理を写しただけだったので，同じ関数を呼ぶ形に．
- `plan()` でも Sink のバージョンを検査（`plan()` は `validate` 済みを前提しない）．D81 に `plan()` 経由のテストと，Link の宣言方向で繋がるケース（フィクスチャの Link が両方向を宣言していたため，片方向の照合を消しても気づけなかった）．Executor 共有のテスト．古い記述の整理．

判断して変えたもの（Fable 5.1 の推奨を全件採用）：
- **D85** component と Island executor が同名なら引き続き拒否（ルールは名前空間であって，現在の解決経路ではない）．Island の fragment id `island_<n>` も名前空間に予約．
- **D86** `memory_domains` が空の Sink（validate）と Executor（`plan()`）を拒否．
- **D87** Session では binding 名1つが役割1つで，**profile が役割を選ぶ**（Island の executor 名 → Executor，`feed` あり → Sink，それ以外で Provider を持つ → resource）．複数役割の Module は役割ごとに bind する．これまでは Provider+Sink の Module がどう bind しても通らなかった（MA-1 と矛盾）．

#### D88–D91 — 4回目の再レビューと，その判断（2026-09-23，採用・適用済み）

D85–D87 の適用を Opus 5.5 が再レビューした（P0 ゼロ・P1 2・P2 7，正当な文書の誤拒否ゼロ）．

判断不要で直したもの：
- **P1**：MA-16 の gate が **trait impl**（`impl Iterator for PrepareContext` で `&dyn Sink` を返す）を辿らず，関数本体や `const _` 内の impl も見えていなかった．収集をファイル全体の `syn::visit::Visit` に書き直し，7経路を赤テストで1つずつ固定．
- Session で Island executor の binding に付いた `feed` が黙って捨てられていた → 拒否．Sink 役割のない Module への `feed` は `WrongBindingRole` に．`plan()` でも Sink の空 `memory_domains` を検査．古い件数・RS-14 の Sink アドレス（HEAD から誤り）を訂正．

判断して変えたもの（Fable 5.1 の推奨を全件採用）：
- **D88** Authority は Spec の **resource** でなければならない．使われていない binding が ExecutionClass（Simulation＝決定性を主張できる唯一のクラス）を決められた穴を塞いだ．
- **D89** **どの役割も担わない binding は拒否**（Spec Run・Session とも）．未登録バージョン 9.9.9 の binding が検査を素通りして Manifest に載り得た．テスト側は `profile_binding` が `exec` を無条件に bind していたのを，Island を持つテストだけに限定．
- **D90** MA-16 の gate は非 `pub` メソッドも辿る（保守的なまま）．
- **D91** X7（node ≠ LOCAL の拒否）を `NodeId` の deserialize で全文書に強制し，Rust 値で渡る ID は `validate` で検査．これまで `ClockRegistry::register` だけが検査していた．schema は不変．

#### D92–D94 — 5回目の再レビューと，ループの一時停止（2026-09-23）

D88–D91 の適用を Opus 5.5 が再レビューした（P0 ゼロ・P1 3・P2 7，正当な文書の誤拒否ゼロ）．P1 は (1) **D88 と D89 の組み合わせで Authority だけを持つ Module（Vision §7 の `sim-engine`）がどうやっても bind できない**，(2) MA-16 の gate が self type が単純パスでない impl（`impl IntoIterator for &Ctx`，`impl dyn Events`）を見逃す，(3) SB-36 の need が binding のない Provider に解決される．(2)(3) と P2 は判断不要として修正．判断3件は Fable 5.1 の推奨を採用：
- **D92** `authority` は，どの役割にも名指されない binding を**専用の Authority**として名指せる（`Role::Authority` の fragment を持つ）．推論は resource だけ．**D88 の「名指された非 resource は拒否」をこの場合に限り覆した**．schema 変更なし．
- **D93** 同一 instance の2名は候補2つのまま拒否し，メッセージに候補名を出す．
- **D94** `NodeId` の schema に `maximum: 0` は付けない（30 schema を v2 にしないと外せないため）．

**ループはここで一時停止**（owner「一度もっと仕様を固めたほうがいい」）．5巡とも P0・誤拒否はゼロだったが，毎回新しい P1 が出た．その型は2つ：MA-16 の gate が新しい構文形で抜けられること，過去の判断同士の組み合わせで矛盾が出ること（D88×D89）．どちらもルールがレビューで決まっていて，事前に書かれていないことを示す．停止時点の未決事項は [00-overview.md §11](plan/phase1/00-overview.md)「Fifth re-review, D92–D94, and the pause」の末尾に4つ：
1. **SB-1 の文法が文書では強制されていない**（`"Radio Bad!"` が `Ident` として通る．確認済み）
2. `plan()` が D89・D91 を再実行しない
3. MA-16 の gate は構文ベースで列挙では閉じられない（ルールの書き方を決める必要）
4. D89 による「1つのラボ用 profile を複数 Spec で使う」運用の不可（承知の上で採用）

未追跡の `Cargo (1).lock` と `crates/ezsdr-kernel/Cargo (1).toml` は Google Drive の競合コピー（9/22 の古い版）．削除はユーザ判断．

#### D95–D103 — 仕様の固め直し（2026-09-23，採用・適用済み）

レビューループを止め，binding と role の規則を先に書き下ろした．置き場所は spec 03 の SB-22…SB-22h・SB-24 と表 SB-T0〜T4 の1箇所で，理由と退けた案は [00-overview.md §11](plan/phase1/00-overview.md)「Firming up the specification, and D95–D103」にある．草案は owner に出す前に Opus 5.5 の敵対的チェックを1回通し（P0 1・P1 6），推奨を3つ変えた（typestate → 構造検査の共用，MA-16 の allow-list 化 → 仕組みの禁止，schema `pattern` → 説明文のみ）．owner は推奨を全件採用した．

- **D95** SB-1 を文法表 SB-T0 にした．8つの名前型（`Ident` `Namespace` `Key` `ModuleId` `EventKind` `DataContractId` `ContentHash` `ResourceId.path`）を deserialize 時に `parse` で検査し，Rust 値で渡る `ResourceId` は `validate` で検査する．schema には `pattern` を載せない（D94 と同じ理由）．port 名と `PortRef` の両半分は `Ident`．
- **D96** 名前空間に authority 名と予約語 `sink` を加えた．runtime の入力は slot 名でしか読まない．役割判定は1関数（`require_role`）にした．
- **D97** `authority` を必須にした（schema は required）．推論は廃止（SB-24 の省略規定，D88 の推論半分，D93 を覆す）．
- **D98** `AuthorityDescriptor.module` を追加（schema）．
- **D99** `plan()` は `validate()` の構造検査と endpoint 検査を同じ関数で再実行する．guard は「matched node を bound instance が宣言している」まで確かめ，`collect_prepare` も同じ guard を使う．場当たりの再検査と `check_sink_links` は削除した．
- **D100** MA-16 の走査は残した．そのうえで `src/` の `use … as` と一覧外の `macro_rules!` を禁止し，MA-16a（process）を新設した．MA-6 にシグネチャ単位の検査を置いた．
- **D101** D89 を確認した．**D102** 文言を修正した．**D103** `RunChild` に `binding_hash`（schema）を足し，RS-25a（forward）を置いた．

新しい拒否は1件ずつ scratch copy で無効化し，すべてテストが落ちることを確認した．無効化しても通ったもの4件は，テストを足すか死にコードとして消した．どちらの gate も，`src/` に禁止構文を注入すると落ちる．

適用後に**適合性の敵対的再レビュー**（Opus 5.5，規則を探すのではなく書いた規則とコードの一致を確認）を1回回した．結果は P0 1・P1 4・P2 7．いずれも書いた規則が答えを決めているので，判断を仰がずに直した．
- P0：SB-2 の owner / `KeyDecl` 検査と SB-6 の kind 検査が matching の中にあった．そのため `plan()` が他の Spec の結果で未登録の `ext.` キーを通していた．これを構造検査へ移した．同じ流れで，SB-30 の第1点も `plan()` で再実行するようにした．
- P1：テストの無かった4経路（tree node，`arm_after` と need key の読み取り，artifact kind の無い Sink，`RunChild` の2つの hash）にケースを足した．
- P2：`executor_kind` を `Namespace` に型付け（schema），文言を修正した．

記録だけしたもの：Session action の target が X7 / SB-1 の検査を経ずに log に入る件（D91 と同じ類型．SB-1 の範囲外）．範囲外で見つけて記録だけしたもの：MA-46 の freeze 用 schema 一覧が D25 以前のまま（`StopCause` / `StopMode` / `StepOutcome`），`collect_prepare` が report の無い fragment を黙って飛ばす件．

#### D104–D107 — raised 4件の適用（2026-09-23，採用・適用済み）

owner が4件とも推奨を採用した．規則は増やしていない（277のまま）．理由と退けた案は §11「The four raised items, D104–D107」にある．
- **D104**：MA-46 の一覧を `StopMode` / `StepOutcome` に直し，両方に schema を足した．role signature が持つ文書型に schema があるかを `ma_06_role_signatures_…` が確かめる．`ov_22_schema_freeze` は登録外の schema ファイルも拒否する（OV-22 の「`schemas/` と照合」を両方向にした）．
- **D105**：report の無い resource は `collect_prepare` が拒否する（SB-41）．
- **D106**：`SessionLog::append` は，target の X7 / SB-1 と，entry の時刻および `at` の X7 を検査する（RS-15）．
- **D107**：SB-39 の guard が `matched` 全体を見る（need の entry と余分な key）．need の key は `need_key` 1か所で作る．

新しい拒否9件は，scratch copy で1件ずつ無効化し，すべてテストが落ちることを確かめた．

#### D108 — `UNCERTAIN:` 37セルの解消（2026-09-23，採用・適用済み）

owner が推奨を全件採用した．規則数は277のまま．理由と退けた案は §11「The 37 `UNCERTAIN:` cells, D108」，行の分類は [exit-review/README.md](plan/phase1/exit-review/README.md) にある．
- **テストを追加**：OV-1，TM-13a，TM-16c，SC-20，SC-20b，SB-17，SB-20，RS-3，RS-27，MA-2，MA-38（関数6本と既存テストへのケース追加）．TM-7 と TM-15 の注記は古く，テストはすでにあった．
- **分割 marker**：Phase 1 の半分をテストし，呼び出し箇所を Phase 2 の coordinator か Provider に回す（SC-31a の書き方）．12規則が default+forward，SB-23 と MA-13 が default+producer，MA-24 が forward+producer．
- **型か allow-list を担い手とする**：SB-1，SB-12，SC-23，MA-36，OV-20，SC-22．
- **規則文の修正**：SB-8（matcher に範囲を絞った．「Kernel は channel や sample rate を名指さない」は SC-14 と TM-13a に反していた），OV-21/SB-8 の key 一覧に `test.gain` を追加，OV-17 の「section ごとの hash」を削除，MA-26 の「unknown」を `CleanupFailure` に言い換え，OV-1 を process に（OV-3 の範囲を OV-1…OV-19 に）．

新しいテストが担う7経路は，scratch copy で1件ずつ無効化し，それぞれ新しいテストだけが落ちることを確かめた．

#### 次にすること

1. 6つの spec を受理する（OV-5：Gate A・B・C ごとに decisions 表，規則，テスト表の順）．
2. 受理後に §12 の Step 5 を進め，spec を `design/` へ移動して全リンクを確認する．Vision の規範文編集は OV-6 / §12 の別途承認を経て行う．

作業ツリーは未コミット．このセッションでは commit / push していない．

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

### 2026-09-22（Step 4: crate 実装，敵対的レビュー13巡，exit criterion 2 の per-rule 表）

- MSRV は X12 どおり **1.85 のまま**．edition 2024 の let-chain は 1.88 以降なので，8箇所を nested `if let` に書き直した（§11 D13）．spec を動かすより code を直すほうが小さい．
- `Constraint` / `CapabilityValue` / `OutputSource` は struct variant．OV-13 の internal tag は payload が map でない newtype variant を serde が扱えない（§11 D1）．
- `kernel_surface` の allow-list は「Core remains small」のレビュー用チェックリストそのもの．2026-09-22 時点の gate は **`NEW:` 111件 / public item 286件**を報告した．D66 適用後の現在値は **110 / 282**（2026-09-23）．OV-23b の成長指標には gate の出力を使う．
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
