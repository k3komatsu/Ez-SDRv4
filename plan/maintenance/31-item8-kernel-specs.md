# 項目8 — Kernel spec 01〜06 の古い規範の写しを消し、DA-1 の書式にそろえる計画

| 項目 | 内容 |
|---|---|
| 状態 | **承認済み（owner 2026-10-13「推奨で進めてください」：D2 は Opus のサブエージェント、D3 は入れる）。段階8a〜8e完了（06・01・02・03），判断1・4・6は反映済み。次は8f（04と小さな残り）→8g（05，§8 E3必須）→8h。全体は未完了。計画の独立レビュー反映は§7，8eの決定と検証は§8。** |
| 作成日 | 2026-10-13 |
| 調査時の基準 | `main`、HEAD `05b3c56`、作業ツリー clean |
| 対象 | 凍結前の改修の項目8（[24-prefreeze-audit.md](24-prefreeze-audit.md) §8、F44）。これは spec 22 の DA-3 の手順4（owner 承認 2026-10-07）にあたる。前向き監査の判断1・4・6（[28-forward-audit.md](28-forward-audit.md)、owner 承認 2026-10-10）も入れる |
| 目標 | 凍結で固まる Kernel の規範（spec 01〜06）に、矛盾する写しや古い文を残さない。規範文を短くし、経緯は Changes 表と `plan/` の決定記録に置く（DA-1） |

## 1. 推奨：DA-3 の手順4を、そのとおりに進める

まず機械的な検査 `rules.py` を作る（DA-3 の pass 1）。そのあと、spec 06、01、02、03、04、05 の順に1本ずつ DA-1 の書式で書き直し、spec ごとに commit する。前向き監査の判断1・4・6と、小さな残りは、関係する spec の回に入れる。

**spec ごとの gate**（§4.4 に詳細）:

- **`rules.py`（機械的な検査）:** 規則の ID が残るか、引用したテスト名が残るか、リンクが解決するか、mutation の行がちょうど1回一致するか。
- **参照の照合:** 変えた ID・節・表・見出しを、ほかの文書・コード・mutation の行から引いている箇所を確かめる。
- **意味の差分のレビュー（gate の本体）:** 消した文や変えた文、表のセル、列挙、疑似コードのそれぞれに、「移した（移した先の ID と、意味が保たれていること）」「経緯」「決定で削除（その決定の記録）」のどれかの印を付ける。新しく足した規範には、承認済みの決定を要求する。印のない変更が1つでもあれば不合格。

## 2. 根拠（HEAD `05b3c56` で確認、Sol R6 で更新）

- **E1 矛盾する写しが残っている（F44）:** spec 06 §2 の KA-1〜KA-22 は、spec 01〜05 に適用済みの改訂を「新しい規範文」として写したままである。そのあとの改訂（KB・KE・KG・KH・KI、spec 22、項目3）が元の規則を変えた。状態は次の3つに分かれる。

  | 箇所 | 状態 |
  |---|---|
  | KA-7（受信から測る）と MA-10（dispatch から測る）、KA-10 の Cold の行と UC-3、KA-12（「Phase 2 には子の Run がない」）と RS-6、KA-13 と KB-2、KA-1 と spec 05 §4 | 未修正（矛盾したまま） |
  | KA-4、KA-6 | 注記で置き換え済み（項目3）。本文は古いまま |
  | spec 06 冒頭の Scope と Not in scope（`design/06-kernel-coordinator.md:6-7`）。HardwareInLoop と Hardware を「範囲外」としているが、KC-2 はすでに実装済みとしている | 未修正 |
  | spec 02:299 の `closed_by_stream_gap` の古い説明 | 未修正（「UHD の alignment error のよくある結果」はすでに消えている） |
  | spec 03 B4、spec 04 §9 の7、spec 06 §13a | 未修正 |
  | F45（TM-13b、SC-13・16a・30）、F46（SC-31・31a）、F47（ProviderInstance の形と MA-10） | 主な修正は済み。この計画では今の修正を保ち、古い説明だけを除く |

- **E2 規範文に経緯が多い:** 次の数字は、どう数えたかとあわせて示す。
  - 箇条書きの規則の ID は 316 個。`^- \*\*[A-Z]{2,3}-\d+[a-z]?\d*\*\*` で数えた（`TM-16a1` と `TM-16b1` を含む。DA-3 の正規表現では 314 個になる）。
  - これとは別に、spec 06 §2 には KA の見出しが 22 個ある。
  - 長さは、01〜06 の合計で 79 191 語、1 500 文字を超える行が 32 行ある。
  - 経緯の印の数は、数え方を決めないと再現できないので、目標にはしない。印が減ったかどうかは、`rules.py` が base と head を同じ正規表現で比べて確かめる。
- **E3 凍結すると直せなくなる:** spec 01〜06 は v4.0 の Kernel の規範になる。凍結のときに2つの規範文が食い違っていれば、どちらも凍結した意味を持ってしまう。`mech:fact-stated-twice-in-spec` は、すでに4件（#39、#41、#43、#44）起きている。
- **E4 書式の効果は試行済み:** spec 22 の試行版（DA-2）では、時刻の規則 17 本が 5 150 語から 2 427 語に減った。
- **E5 機械的な検査の道具がまだない:** `rules.py` は作られていない。

## 3. 採らない案

- **古い写しだけを消す（F44 の最小の直し）:** 経緯の印の大半と長すぎる行が、凍結の規範に残る。owner が承認した DA-3 の手順4からも外れる。
- **全 spec を1回で書き直す:** 規範文の取りこぼしを、レビューで見つけきれない。
- **`rules.py` なしで進める:** 規則の ID の消失や、引用したテスト名の切れを、人の目にしか頼れなくなる。
- **参照を管理する本格的な道具を作る:** 範囲が広がりすぎる。spec ごとの grep による照合で足りる（Sol R3）。

## 4. 進め方

### 4.1 段階

1. **段階8a `rules.py` と AGENTS.md の規則**
   - base と head を比べる、約60〜80行の、標準ライブラリだけの検査を作る。
   - **規則の ID:** 箇条書きの ID（`TM-16a1` のような数字の後ろの添字も含む）、見出しの ID（spec 06 §2 の `### KA-n`）、廃止の跡を数える。どれも残っているか、廃止として載っていることを確かめる。ID の重複は拒否する（Sol R1）。
   - **引用したテスト名:** 残っているか、改名として載っていて新しい名前が `fn` / `def` として実在することを確かめる。今すでに実在しない経緯上のテスト名（`design/06-kernel-coordinator.md:109,255` など）は、初期状態の既知の例外として一覧に入れる（Sol R5）。
   - **相対リンク:** 解決することを確かめる。
   - **mutation の行:** 全6リストについて、文字列の検査だけで、ちょうど1回一致することを確かめる（`mutate.py` は起動しない）。
   - **経緯の印と長すぎる行:** base と head を同じ正規表現で数え、増えていないことを確かめる。
   - **`--sentences`:** 文の対応づけの補助（`difflib`）も作る。あくまで補助で、gate ではない。
   - **自己確認:** 小さなテスト用の文書を用意し、ID の消失、引用名の消失、切れたリンク、0回／複数回の一致を、それぞれ拒否することを確かめる（Sol R5）。
   - **AGENTS.md:** §2 に、spec 22 DA-1 の「Binding text」を同じ commit で入れる。
2. **段階8b〜8g 1 spec ずつ:** 順番は 06 → 01 → 02 → 03 → 04 → 05。各 spec の回でやることは次のとおり。
   - DA-1 の書式にする。規則は一つの話題を数行で書き、`*Test:*`（または `*Producer obligation*`）の行で終える。理由・経緯・issue 番号は書かない。経緯は末尾の Changes 表と `plan/` の決定記録へ移す。
   - 規則の ID は付け直さない。廃止する規則は1行の跡を残す。一つの話題は一か所にだけ書く。
   - 表（SC-24 の状態遷移表、KC-24 の拒否表など）、列挙、疑似コードも規範なので、文と同じく意味の差分のレビューの対象にする（Sol R2）。
   - #60 で足した注記（TM-16c の「root の時間軸ごとの順序」「同時に due なら root が先」など）には、経緯の印が付いていても規範の中身が入っている。経緯として消さずに、規範として残す（Sol R2）。
   - **spec 06 の回:** §2 の KA-1〜KA-22 を、**元の見出しと ID を保ったまま**、1行の跡（改訂した先の今の規則への参照と、`plan/phase2/` への参照）にする（Sol R3）。spec 07〜18 から、いま有効な参照として引かれている KA（RM-7 の KA-6、SE-11 の KA-11、NX-6 の KA-13、HD-16 の KA-12 など）は、跡がその参照先を示すので切れない。KC 規則、Assembly と RunHandle の型、§10 の参照は残す。冒頭の Scope と Not in scope を今の範囲に直す。§13a の古い文を消す。
3. **段階8h 締め:** 24-prefreeze-audit の Owner decisions の項目8の行を完了にし、handoff の凍結前チェックリストを更新する。

### 4.2 各 spec の回に入れる個別の変更

| spec | 入れる変更 | 根拠 |
|---|---|---|
| 01 | TM-18 の最後の文を消す。最初の文は「v4.0 の `clocks.relations` は Authority の relation だけを持ち、自分の timekeeper を持つ Provider は自分の section に relation を書く（RS-39）」とする。`ClocksSection.relations` の doc も同じにする | 前向き監査の判断1 |
| 02 | sample stream の規則（SC-12、SC-13、SC-16 の Rx/Tx の意味、SC-18、SC-23〜SC-32）を `ezsdr.stream.*` の契約に限る規則を1つ足す。ほかの契約では、その Vocabulary の spec が `first_sample_time` の domain と `len` の単位・向きを決め、gap と burst のフラグは使わない。SC-10・10a・11・14・15・19〜22 はすべてのブロックに適用する。design/02:7 と §4 の注記をそれに合わせる。SC-4 と Vision §21 から `ezsdr.event.<schema-id>` を消し、Vision の改訂履歴に1行足す。spec 02:299 の `closed_by_stream_gap` の古い説明を消す | 判断4、F44 |
| 03 | SB-39 に「Sink の fragment の content はその `OutputReq`（Session の暗黙の `OutputReq` も含む、SB-22c）、Island の fragment の content はその `IslandDecl`」と足す。`Fragment.content` の doc から役割ごとの形を参照し、schema を作り直す。`plan/compile.rs:142,158` の2つの `unwrap_or(serde_json::Value::Null)` は、Provider の content がすでにやっているのと同じエラーの変換（`map_err(...)?`、`compile.rs:34-38`）にする（Sol R7）。テストは Sink と Island の両方の content を確かめ、それぞれを別々に壊す mutation の行を用意する。実際には起きない serialization の失敗を、テスト用の仕組みを足してまで起こすことはしない。B4 の古い文を消す | 判断6、F44 |
| 04 | §9 の7の古い文を消す。RS-16 に「Module に由来する未配送の Action の id は action log に現れない」と1文足す | F44、handoff の任意項目 |
| 05 | — （書式だけ） | — |
| 06 | §2 を KA-n の跡にする。冒頭の範囲の表を直す。§13a の古い文を消す | F44、Sol R6 |
| 小さな残り（D3） | `session.rs` の `Admitter` の doc comment を、実際の呼び出し（admission だけ）に合わせる | handoff の「記録だけ」 |
| 小さな残り（D3） | Provider の Manifest section の `rejected[].action`（kind の文字列）を `kind` に改名する。対象は、MockRadio の `Rejected.action`（`crates/ezsdr-mock-radio/src/lib.rs:656-661`）、UHD の拒否の記録（`provider/core.rs:373-375`、`provider/tx.rs:243,266`、`provider/control.rs` の `reject_note`）、それを読むテストと mutation の行、規範の MR-27（`design/09-mock-radio.md:146`）と UR-30（`design/18-uhd-radio.md:192`）の限定的な修正。**イベント `radio.COMMAND_REJECTED` の payload の `action`（ActionId）は変えない**。この section に schema はない（Sol R4） | handoff の任意項目 |

### 4.3 分担（AGENTS.md §8・§9）

- **8a（`rules.py`）:** 小さなツールなので orchestrator が書く。
- **8b〜8g（spec の書き直し）:** 設計文書の編集は判断を伴うので、Opus のサブエージェントが書く（AGENTS.md §8）。
  - 1回に1 spec。今回の #60 の経験から、DeepSeek は規範文の取りこぼしと、途中で終える危険が大きいと判断した。
  - 小さなコードの変更（判断6、`kind` への改名、doc comment）は、その spec の回のサブエージェントが一緒に行う。`kind` への改名は 03 か 04 の回にまとめる。
- **レビュー:** spec ごとに Sol 6.1 が独立に、§4.4 の意味の差分のレビューを行う。orchestrator が判定し、gate を通して commit・push する。

### 4.4 gate（spec ごと）

1. **`rules.py`:** base と head で通る。
2. **参照の照合（Sol R3）:** その回で変えた ID・節・表・見出しを、次の場所から grep で引き、引いている側の意味が保たれていることを確かめる。経緯上の引用と、いま有効な規範への引用は分けて扱う。
   - `design/07-*.md` 〜 `design/18-*.md`
   - Vision の `Normative:` の行（節や表を指すものも含む）
   - `schemas/`
   - コードのコメント
   - mutation の行の `name`
3. **意味の差分のレビュー**（Sol、§1）。
4. **コードを変えた回だけ（Sol R5）:**
   - `cargo test --workspace` と `cargo clippy --workspace --all-targets -- -D warnings`
   - schema を作り直したら `schema_freeze`
   - 修正の途中では変えた行だけを流し、その回の最後に `remote_mutate.py all sim02:8,sim03:8` の全 gate を流す（AGENTS.md §7）

## 5. 決めていただくこと

D1（全 spec を DA-1 の書式にすること）と D4（Vision §21 の修正）は、すでに承認済みの DA-3 の手順4と判断4に含まれるので、改めて決める必要はない（Sol の総括）。新しく決めていただくのは次の2つである。

- **D2 書き手:** spec の書き直しを Opus のサブエージェントに任せる（推奨）か、§9 の既定どおり DeepSeek にする（Opus のレビューを厚くする）か。
- **D3 小さな残り2件**（`rejected[].action` を `kind` に改名すること、`Admitter` の doc comment）と RS-16 の1文を、項目8に入れるか。推奨は「入れる」。どれも数行で済み、版の互換を気にしない今のうちが一番安い（凍結の対象は Kernel だけなので、「凍結前にしか直せない」わけではない。Sol の総括による訂正）。

## 6. この計画に入れないもの

- DA-3 の手順2（spec 07・09・18 の残り）、手順3（11 を 08 に統合）、手順5（spec 10・14・16。02 はこの計画で扱う）、手順6（監査の移動）。例外は、`kind` への改名に伴う 09 の MR-27 と 18 の UR-30 の限定的な修正である。
- 凍結前の「規則ごとの確認」（規則を安い外部エージェントで1本ずつ確かめるもの）。項目8のあとの別の作業である。
- Phase 8 以降。

## 7. 独立レビューの反映（Sol 6.1、APPROVE_WITH_CHANGES）

| ID | 内容 | 反映先 |
|---|---|---|
| R1 P1 | ID の検査が、`TM-16a1` のような ID と KA の見出しを取りこぼす。重複も拒否すべき | §2 E2、§4.1（段階8a） |
| R2 P1 | 消した文の印だけでは、規範の変更、表のセル、疑似コード、#60 の注記の中身を守れない | §1、§4.1（段階8b〜8g） |
| R3 P1 | KA や節・表への参照（spec 07〜18、Vision の `Normative:`、schema、コード、mutation の名前）の照合を明示すべき。KA の跡は元の見出しと ID を保つべき | §1、§4.1、§4.4 |
| R4 P1 | `kind` への改名の対象（MR-27、UR-30、すべての書き手と読み手）と、対象外（イベント payload の `action`）を明記すべき。この section に schema はない | §4.2、§6 |
| R5 P1 | コードを変える回の gate（通常のテスト、`schema_freeze`、全 mutation gate）、`rules.py` の自己確認、引用名の改名の扱い、初期状態の既知の例外 | §4.1、§4.4 |
| R6 P2 | 対象の一覧を HEAD に合わせ、未修正・注記済み・修正済みを分ける。spec 06 の冒頭の範囲の表を加える。経緯の印の数え方を明記する | §2 E1・E2、§4.2 |
| R7 P2 | 判断6のコードは、単純な `?` ではなく既存のエラーの変換に合わせる。Sink と Island の両方を検査する | §4.2 |
| 総括 | D1・D4 は承認済みの内容の確認で、決め直しは不要。D3 の「凍結前にしか直せない」は言い過ぎ | §5 |

## 8. 段階8eの決定と8gへの必須持越し

ownerは2026-10-13、独立レビューのE1/E2/E3に対するquestionへ「推奨どおり進める」を承認した。これは書式だけでなく、次の意味の明確化の承認である。

- **E1（8e）:** SB-37はbinding→matching（bound instanceのcapabilityを比較する）の関係だけを定め、各段階の検査・拒否は既存SB-T4を正本として参照する。`plan.rs`のpipeline docとVision §10の写しも同期する。本番制御フローは並べ替えない。
- **E2（8e）:** SB-15aの「only」の範囲はportのdirection/contract検査に限る。placementとendpoint memory-domain reachabilityはMA-39を参照する。既存reachability検査は削除・弱化しない。
- **E3（8g必須）:** `design/05-module-api.md` MA-12のreport自己申告を根拠にするcoercion例外を、SB-46の例外条件への参照に縮約する。安全なSB-46と実装を維持し、既存bypass拒否テスト`sb_44_a_provider_may_not_exempt_a_key_by_declaring_a_coercion`（および`sb_46_an_accepted_coercion_survives_prepare`）の拒否・許可条件を確認する。8gのbriefに必ず入れ、凍結前に閉じる。8eではspec05を編集しない。

**段階8eの検証記録:** 独立レビューのR1（Islandの非既定contentを固定するテスト）とR2（規則内の理由文）を修正し、再レビューは`PASS_WITH_RISK`（残るE3は上記8gへの承認済み持越し）。`cargo test --workspace`、workspace clippy（all-targets、`-D warnings`）、kernel/serverのschema freeze、`rules.py --base 55cbb2d`とself-test、diff checkは成功。全mutation gate（`all sim02:8,sim03:8`）は734/735、追加J328・J329・J330はすべてkilled。唯一のSURVIVEDは従来のphase7/G09（handoff §4、plan 30 §15に記録済み）で、remote runnerの終了コードは1。全行成功とは扱わず、既知例外以外に生き残り・実行漏れなしと記録する。ログとレビューは`~/.cache/ezsdr-fixes/2026-10-13-item8/03-gate-*.log`および`sol-03-review.md`・`sol-03-recheck.md`。ownerの指示でhandoffとともにcommit・pushする。
