# Ez-SDR v4 — Handoff (2026-09-22)

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
| test | 315 件．`spec_binding`(86) `stream_contract`(68) `run_session`(65) `time_model`(48) `module_api`(23) `hashing`(9) `kernel_surface`(9) `schema_freeze`(3) `event_hotpath`(1) `lib`(1) |
| toolchain | `1.85.0` / `stable (1.98.1)` の両方で 315 passed．`cargo clippy --all-targets` も警告ゼロ |
| schemas | `schemas/` に45個の JSON Schema 2020-12 + `SCHEMA_CHANGELOG.md`．`schema_freeze` が byte 単位で凍結．再生成は `EZSDR_UPDATE_SCHEMAS=1 cargo test --test schema_freeze` |
| kernel surface | `tests/kernel_surface_allow.txt` が公開 item の allow-list（= レビュー用チェックリスト，`module::name` で key 付け）．`NEW:` 件数が OV-23b の Kernel 成長指標．banned token は `tests/banned_tokens.txt`（識別子内も検出，`OV-23a` を書いた行だけ免除） |

Python client，wire protocol，MockRadio，Simulation Engine は未着手（Phase 2 以降）．

## 4. Phase 1 の状態 — spec 受理待ち

Vision §67 の Phase 1．詳細設計は [plan/phase1/](plan/phase1/)．計画本体と横断決定は [plan/phase1/00-overview.md](plan/phase1/00-overview.md)．

| spec | rule ID | 状態 |
|---|---|---|
| [01-time-model.md](plan/phase1/01-time-model.md) | TM-1..21（副番含め36） | Gate A 通過，実装済み |
| [02-stream-contract.md](plan/phase1/02-stream-contract.md) | SC-1..32（副番含め48） | Gate A 通過，実装済み |
| [03-spec-and-binding.md](plan/phase1/03-spec-and-binding.md) | SB-1..49（副番含め55） | Gate B 通過，実装済み |
| [04-run-and-session.md](plan/phase1/04-run-and-session.md) | RS-1..52（副番含め57） | Gate B 通過，実装済み |
| [05-module-api.md](plan/phase1/05-module-api.md) | MA-1..46 | Gate C 通過，実装済み |

6文書で261ルール（D47 で SB-15a を追加），欠番と未解決参照なし，撤回6件（SB-25a, SB-28, SB-32, RS-37, MA-4, MA-43 — OV-1 に従い番号は保持）．Gate A は敵対的レビュー3巡（31件・16件・13件），Gate B は2巡（19件・10件），Gate C は1巡（8件）．

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
| 13 | `continuity.rs` に絞った検証（12巡目の作り直しが未レビューだったため） | **P0 ゼロ・P1 ゼロ**・P2 2 | 修正済み．差分 fuzz で lossless 経路は HEAD とビット同一，Gap の `link_dropped` と `lost` の総和が投入分と一致することを確認．**ここで収束** |
| 12 | 11巡目の修正の検証（新たな拒否を2つ足したため） | P1 1・P2 1 | 修正済み．P1 はまた11巡目の修正自身 — SC-30b の carry は flags・lost・blocks の3つを運ぶのに，繰り越しが blocks しか持っていなかった |
| 14 | D45–D50 の判断（Fable 5.1，AGENTS.md §8 の second opinion 例外） | 6件 | **全件採用・適用済み**．D50 は判断ではなく**バグの発見**だった：10巡目で入れた canonical-form 基準が `Int(1152921504606847000) == Num(2^60)` を true にしていた（float の canonical 形は「その f64 を名指す最短十進」であって正確な値ではない）．等価性は厳密な数値比較に戻した．新規ルール SB-15a（ポート方向），`CompileRule::TxBurst` に `late_policy`（schema 変更） |
| 11 | 10巡目の修正の検証（Opus，大規模修正のため必須の再レビュー） | P0 2・P1 1・P2 2 | 全件修正．**P0 2件はどちらも10巡目の修正自身**：RS-52 が component の `params` を先に見ていた（SB-2 が「the only place it can be … never a `ComponentDescriptor`'s `params`」と明記）と，SC-30b の繰り越しが `finish` で抜けていた．P1 は既存で，`collect_prepare` が他 Spec の `AdmissionResult` を拒否していなかった件 |

このクレートが繰り返し出した欠陥型は2つある．

**「正しい関数を誰も呼んでいない」** — 13巡で計8件．さらに exit criterion 2 の per-rule 表を作る過程で**13件**が追加され，計**20件**になった（§11「Findings from the exit-criterion-2 sweep」）．テストから出発するレビューには原理的に見えない — 呼び出しもテストも無い関数だから．検出はスクラッチコピーから削除してビルドが通ることで確認している（`admit_islands`・`check_cycles`・`check_sink_links`・`CheckStage::Prepare`・SB-46 の coercion policy・`ComponentDescriptor::validate`・`check_effective_narrows`，加えて `Value::check_nesting` の ASCII 規則）．8件目は10巡目の `Admitter::admit` で，これだけ**未適用**：RS-17 が名指しする呼び出し口を作るとルール本文の判断が要るので D45 として記録してある．対策として，各ルールの本文に**呼び出し場所を明記**した（SB-30 は3点すべて，SB-41 は MA-12 の2義務，MA-37 は `validate()`，MA-41 は比較箇所）．レビュー側の提言：「`src/` 内に呼び出し元のない predicate を grep する」を exit review の手順に入れること — 4巡で7件を出している．

**「ルールを初めて生かすと，そのルールが意図しないものを拒否する」** — P0 5件のうち4件がこれで，4件とも `collect_prepare` の中，しかも毎回別方向（merge を resource 別に解釈／coercion を構造的に拒否／Provider の自己申告で免除／coercion preview を key だけで引く）．

4件目は**7巡目の修正そのものの中**にあった．除外を Kernel 自身の `coercions_preview` に向けたところまでは正しかったが，`Coercion` は `{key, requested, applied, reason}` でリソースを持たない — つまり **そのVecをkeyで引く限り per-resource にはなり得ない**．同じ key を別デバイスで制約する2チャネル Spec で，片方が coerce されると，直接満たされた（`coerce` を呼ばれてすらいない）もう片方が「`coerce` が返した値を適用していない」として拒否された．3巡連続で同じ関数の同じ構造を別角度から踏んでいる以上，次に `collect_prepare` を触るときは**まずデータが判定に必要な情報を持っているかを確認する**こと．Kernel 自身の記録は `PreviewedCoercion { resource, coercion }` になり，隣の `RejectedConstraint` と同じ形になった．

**「判定を適用する作業そのものが欠陥を入れる」** — 11巡目の P0 2件が10巡目の修正自身の中にあり，同種は7→8巡目，5→6巡目にもあった（計3回）．**修正を書いたら，その修正が参照する条文をもう一度読むこと**：RS-52 の件は SB-2 に「and the only place it can be」と書いてあり，読めば component を見に行く発想が出ない．

**「置換せず追加してしまう」** — 10巡目の P1 2件．`13c4070` は D44 の binding-description ブロックを**追加**したが，D44 が「削除した」と書いたポインタ比較ブロックはファイルに残ったままで，両方が走っていた（SB-22 の `sink` チェックも二重）．古いブロックは D44 が許す形（1つの description に対し2つのオブジェクトが同じ `instance().id` を報告する）を拒否する．**判定を適用したあとは，置換対象が消えたことを grep で確認すること．**

`kernel_surface` は5巡で5組の回避を実演された．いずれも「**接頭辞照合は綴りであり，綴りは改行で割れる**」という同じ形だったので，接頭辞照合を全廃した：宣言は行と次行に跨る**トークン列**として読み，lex できない構文は**トークンとして拒否**し，その上に「スキャン自身が frame 均衡で終わったこと」を assert する（brace クラスを綴りでなく原因で閉じる）．**実演された11通りすべてを scratch copy で捕獲確認．**この gate が macro 経由で隠していた public item が2件あった（`id.rs` の4つの id 型と `schema::document_schemas`）ので，X11 の「allow-list が review checklist である」は事実として偽だった．

その後 `syn::parse_file` による parser に置き換え（D34–D44 の採用，`13c4070`）たが，10巡目がさらに**5通り**を実演した．うち1つは**出荷されるクレートで現に穴が開いていた**：`is_testing_gated` が `cfg` のトークン列に `testing` が*含まれるか*で判定していたため，`#[cfg(not(feature = "testing"))]`（＝デフォルトビルド）の public item を gate が丸ごとスキップしていた．他は `pub use {…}`（brace group が root を名乗らない），`pub extern crate`（match の catch-all に落ちる），非 `.rs` ファイルの `include!`，同名 item を持つ inline module 2つが1つの allow key に潰れる（key が名前でなく深さを符号化していた）．**5通りすべて注入して捕獲を確認済み．**

exit criteria（§13）の達成状況：

| # | 条件 | 状態 |
|---|---|---|
| 1 | 6文書の受理と §11 の全 verdict | **verdict は全件記録済み**（X1–X12，各 spec の Decisions 表，未決事項1–7，findings D1–D33）．残るのは6文書の受理そのもの（§12） |
| 2 | 全 rule に ID と OV-3 disposition | **表は完成，条件は未達**（2026-09-22）．[plan/phase1/exit-review/](plan/phase1/exit-review/README.md) に文書ごと1ファイル，**261ルールに261行**．内訳は default 206・process 18・producer 8・forward 6・withdrawn 6・consumer 1・**GAP 15**・OV-3 自身が UNCERTAIN 1．別に39セルが部分カバーの `UNCERTAIN:` 注記付き．**GAP 15件に carrier を付けるか，owner が marker を足すか，撤回するか**が残り．表を作ったこと自体の成果は §11「Findings from the exit-criterion-2 sweep」参照 — 呼び出し元ゼロの述語が7件から**20件**になり，9ルールが「自分を検査していない checker」を名指ししていた．**規模の実測（2026-09-22）**：`#[test]` 関数315個のうち **78個** はどの spec のテスト表にも載っていない（うち19個は hashing / kernel_surface / schema_freeze で，00-overview がグループとして名指ししている分）．逆向き（表が挙げるのに関数が無い）は**ゼロ**にした |
| 3 | MSRV と stable で `cargo test` 通過，`#[ignore]` なし，pipeline が double で端から端まで動く | **達成**（1.85.0 / stable ともに 315 passed，`#[ignore]` なし）．degenerate ではない：resource endpoint を含む graph が validate → plan を通り（`sb_15_a_bound_resource_port_is_a_link_endpoint`），Provider fragment は matched request を運び（`sb_39_a_provider_fragment_carries_the_matched_request`），Session は bound Sink を output として持つ（`rs_12_a_session_compiles_through_the_whole_pipeline`），coercion は accept/warn/reject の3分岐が到達可能（`sb_46_an_accepted_coercion_survives_prepare`） |
| 4 | `schemas/` commit，`schema_freeze` 通過，`SCHEMA_CHANGELOG.md` の v1 entry | **達成** |
| 5 | `kernel_surface` 通過，`NEW:` 件数の記録 | **達成**．件数は `cargo test --test kernel_surface -- --nocapture` が `OV-23b: Kernel growth = N NEW: items of M public items` で出す |
| 6 | 直接依存が §8 の4 crate ちょうど | **達成** |
| 7 | §12 の移動後に `design/` と `plan/phase1/` の全リンクが解決 | 未（Step 5 の作業） |

**§11 の判定は完了した**（2026-09-22，Fable 5.1 の second opinion をユーザ判定として採用）．
verdict は confirm 91・amend 15・reverse 1・not-a-decision 5．not-a-decision の5件（OQ4,
D1, D6, D11, D14）は encoding の帰結と fixture note で，裁定を要しない．

**次にやること**は amend 15件と reverse 1件の適用．second opinion は適用可否を3つに分けている：

| bin | 内容 | 状態 |
|---|---|---|
| (i) prose / コードのみ，rule text を変えない | D15・D24・S1・B4・OQ4 | **適用済み**（2026-09-22） |
| (ii) rule text を変えるが freeze 前で自己完結 | OQ2・R10・D8・D16・D22・D23・D25・D26・D27・D32・D33，および D17 + D18 + D31 + D29 のクラスタ | **適用済み**（2026-09-22） |
| (iii) 待ち | confirm が含意する Vision 編集（OV-6 のため §12 手続き），および Phase 2 の値を要する2件（resource port の producer 側 memory domain，port contract と format coercion の関係） | 未（§12 / Phase 2） |

**§11 の verdict は D1–D44 まで全件記録済み**（D34–D44 と X11 の扱いは 2026-09-22 の Fable 5.1 second opinion を採用）．採用に伴い次を適用した：`Value` の cross-kind 等価・比較を正確化（D34），`to_root` が非整数 tick の schedule を拒否（D35），`BurstStep::Discontinuity.then_ended`（D36），schedule entry の `target` 検査と「Spec target は Spec 相対」の明文化（D37），OV-3 に4つ目の marker と exit criterion 2 を per-rule 表へ（D38/D39），`constraints_hit` 削除（D40，schema 変更），`coerce` は node あたり1回・map 全体で呼ぶ（D41），要求していない key の coercion は malformed report（D42），`SessionLog::append` が不正な Action を拒否（D43），**instance identity は binding description**（D44，ポインタ比較を廃止）．

**X11 は半分だけ反転した．** `cargo public-api` の却下は維持し，`syn` ベースのスキャナの却下を撤回して `kernel_surface` を `syn::parse_file` に載せ替えた．決め手は**このプロジェクトが実際に回すループの中での失敗の形**：手書き lexer は**黙って**失敗し（5巡で11通り，うち2つは出荷中の public item を隠していた），`cargo public-api` は大声で失敗するが `cargo test` の**外**（X12 が管理しない nightly．CI が無い段階では「動かない gate」は「省かれた gate」），`syn` は MSRV toolchain 上の `cargo test` の**中**で大声で失敗する．OV-18 を当てると `syn` は全条項を満たす — 手書きの代替は709行，MSRV 1.71，**新規 transitive crate ゼロ**（`serde_derive`/`schemars_derive` 経由で既に解決済み，`Cargo.lock` の差分は1行）．X11 はこの代替を記録していながら，自分で書いた OV-18 を当てずに却下していた．gate は 709 → 499 行になった．ただし **parser にすれば終わりではなかった**：10巡目が parser 自身に5通りの穴を実演し，うち1つ（`is_testing_gated` が `cfg` のトークン列に `testing` が*含まれるか*で判定していた件）は**出荷ビルドで現に開いていた**．手書き lexer の穴が「綴りの見落とし」だったのに対し，parser の穴は**述語のロジック**で，parser 化では消えない種類だった．現在の gate は617行で，実演された20通りすべてを捕獲する（1つは `#![forbid(unsafe_code)]` がより手前で拒否）．3つの述語には直接の負テストが付いている．

**コードレビューは9巡で収束した**（5–9巡目は crate に対する敵対的レビュー）．9巡目で初めて P0 ゼロ，`collect_prepare` からも4巡ぶりに指摘なし．9巡目の結論は「**crate は収束．gate は収束していないが，それは足し算では収束しない**」で，`kernel_surface` の残る問いはコードではなく**道具の選択**：

> **`cargo public-api` を採るか．** X11 は nightly rustdoc JSON を理由に却下したが，5巡分の回避はその選択の代価．gate は現在，隠せる構文に対して総当たり的に閉じており，自分が mis-lex したら frame 均衡の assert で大声で落ちる．それでも依然としてテストファイル内の行ベーススキャナである．この答えが OV-23 と X11 の一文の去就を決める．

§11 の verdict は D1–D44 まで**全件記録済み**で，未適用の判定は残っていない．

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
