# 段階 3a：指示書

## 実装（`impl-3a-brief.md`）

あなたは実装役です．HEAD は `548f675`．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．**

## 0. 守ること

- `cargo fmt`・`rustfmt` を流さない（tree は rustfmt の形でない．新しいコードは周りの形に手で合わせる）．ローカルで `mutate.py` を流さない．作業ツリーに mutant を当てない．commit・push・`git add`・`git stash` をしない．
- version を上げない．`schemas/SCHEMA_CHANGELOG.md` に書かない（AGENTS.md §6）．schema は型から生成し `schema_freeze` を緑に保つ．
- **文脈を守る**：cargo とテストの出力はファイル（作業 dir）に出して `tail`・`grep` で短く見る．自分の log を全部読まない．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- 負荷をかけない（`--test-threads` を上げない，並列の cargo をしない）．テストの一時ファイルは /tmp のまま，`TMPDIR` を変えない．
- **起きたことを書く．していないことは書かない．**

## 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md`（全部．§2〜§4 がこの段階，§7 の末尾の「3a の指示書へ」も）．
- `plan/phase8/00-overview.md` §4 の P2・P3（決まっていること）．
- コード：`crates/ezsdr-radio/src/channels.rs`（`Description`，`Description::new`，`capabilities()`，`from_selector`，RM-30 の stream の規則），`x3xx.rs`（`timing`，`performance`，`ProviderTerms`，`profile_ref`），`device.rs`（`Grid`），`lib.rs`（`keys`，`TimingEnvelope`，`PerformanceEnvelope`）．`crates/ezsdr-mock-radio/src/profile.rs`（`Profile::terms`，`BLOCK_LEN`），`crates/ezsdr-radio-uhd/src/profile.rs`（`terms`，`DEFAULT_BLOCK_LEN`）．
- schema の生成と `rm_20_schema_freeze`（`crates/ezsdr-radio/tests/radio_model.rs`）．`envelope.v1.json` がどう作られているかに倣う．
- spec：`design/07-radio-model.md` RM-4，RM-20，RM-27〜RM-30．

## 2. 作るもの

1. **`ezsdr_radio::measurement`**（新しい module）：
   - 記録の型（設計メモ §2 の欄：`schema`，`source`，`profile`，`selector`，`device`，`host`，`load`，`commit`，`conditions`，`values`，`observations`）．serde．`schema` の値は `ezsdr.radio.measurement.v1` で，違えば読むときに拒否．
   - key ごとの扱い `rule(key) -> Option<Rule>`（設計メモ §3 の表．下限，上限，包含，倍数，保証の強さ，真なら真，一致，比べない）．`c<n>`・`s<k>` の付く key は番号を外した形で引く．
   - **profile の値**：`Description` から key ごとの値を出す関数．周波数・利得は範囲の端ではなく `Grid` 全体（包含は格子の値の集合で比べる），antenna は集合，rate は `Description::rates` の格子．この関数の key の集合が `capabilities()` の key の集合と一致することを unit test で確かめる（`capabilities()` は private のままでよい．同じ crate の中から呼ぶ）．
   - **比較** `compare(record, description) -> Vec<Violation>`（純粋な関数）：設計メモ §4 の 1〜5．`Violation` は key と理由を持つ．§7 の minor：条件の id は一意，`(direction, stream)` は一意，stream の番号はその向きの channel 数の範囲，一つの向きで channel を 2 つの stream に入れない，`concurrent` は向きつきで宣言した stream を指す，`values`・`observations` の `condition` は宣言した id．
2. **schema**：`schemas/radio/measurement.v1.json` を型から生成し，`schema_freeze` に入れる．
3. **MockRadio の `Profile::terms()` を公開する**（値は変えない）．比較テストが Mock の実際の値を読むため．公開の仕方は crate の今の形に合わせる（`pub fn terms()` か，`profile` module の公開）．
4. **比較テスト** `crates/ezsdr-acceptance/tests/measurements.rs`：`crates/ezsdr-radio/measurements/` の記録をすべて読み（ディレクトリが無いか空なら何も比べずに通る），記録ごとに selector から layout を作り，**MockRadio と UHD Module それぞれの** `Description::new(layout, <block_len>, <terms>)` で `compare` を呼び，違反が無いことを確かめる．`source: "fake"` と `profile` の不一致は失敗．`measurements/` は空のディレクトリを置かない（`.gitkeep` も足さない．無ければ通る形にする）．
5. **fixture とテスト**：`crates/ezsdr-radio/tests/fixtures/measurement/` に，規則ごとに通る記録と落ちる記録を置き，`crates/ezsdr-radio/tests/measurement.rs` で `compare` を試す．最低限：
   - 各扱い（下限・上限・包含・倍数・保証の強さ・真なら真・一致）で，profile が測定より緩いと落ち，同じか厳しいと通る．包含は「端は同じで格子が粗い」場合も落ちる（端だけで比べる誤りを捕まえる）．
   - `values` に key が欠ける，2 度ある，知らない key がある，で落ちる．`unmeasured` は通る．比べない key（`stream`，`block_len`）が `values` にあると落ちる．
   - `schema` の違い，`source: "fake"`，`profile` の違いで落ちる．
   - `conditions` の各規則（§2 の 1 の最後）の違反で落ちる．
   - 両 Provider：Mock の lead 2 ms と UHD の 5 ms の間の測定値（例 3 ms）の記録が，Mock の Description で落ち，UHD の Description で通る（比較テストが両方を見る理由）．
   fixture は手で書いた JSON．テストの中で記録を組み立てる helper を作るなら 1 つにする．
6. **spec**：測定記録と比較テストを `design/07-radio-model.md` に書く必要があるかを判断する．P2 は「記録は `ezsdr-radio` crate の中」で，語彙の公開の型になるので，RM の新しい規則（例：RM-35 として，記録の形・key ごとの扱い・比較の規則を数行，`*Test:*` 行つき，DA-1 の形）を足すのが筋．足すなら Changes に 1 行（設計メモ 41 を引く）．`plan/maintenance/tools/rules.py --base HEAD` が通ること．

## 3. 範囲

設計メモ §2〜§4 だけ．測定スイート（`measure_*`），候補の値で測る仕組み，`hardware.rs`・`fake.rs` の変更は 3b 以降で，ここでは作らない．profile の値は変えない．新しい capability の key を作らない．比較に使わない汎用の仕組み（記録の一覧表示，変換の道具など）を作らない．大きさの目安は本体とテストで約 1,500 行（fixture の JSON を除く）．

## 4. 確かめること

- 新しいテストを各 20 回続けて流して全部通る（`timeout 300`，1 binary ずつ）．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：`compare` の各扱いの判定（`>=` を `>` に，包含を端の比較に，key の重複の検査を外す，`source` の検査を外す，両 Provider の片方を外す，など）に行を足す（`plan/maintenance/tools/mutations.json` に，形は既存の行に倣い，行を足すだけで file を書き直さない．1 space の字下げを保つ．ID は既存の最大の次から）．新しい行を流して killed を確かめる：`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> <ID> ...`（ID は空白で区切る）．流す前に tree の変えたファイルと新しいファイルを mode 644 にする．各行の `old` が tree でちょうど 1 回一致すること．全リストは流さない（orchestrator の gate）．

## 5. セルフレビュー

報告の前に，読むだけの reviewer を新しく起こす．1 回ごとに新しい reviewer：

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3a-<n>.log 2>&1
```

前景で流す．free の model が上限や障害で失敗したら（log が空，エラーで終わる，上限の表示），同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` でやり直す．見つかったものを直して次の回へ．P1 以上が無くなるか 3 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に書く．

## 6. 報告

`report-3a.md`．最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．中身：作ったもの（ファイルごと），設計メモから変えた所とその理由，テストの一覧（何を落とすか），spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（どの model で回したかも），残ったこと．


## セルフレビューの checklist（`checklist-3a.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `548f675`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない（`--test-threads` を上げない，並列の cargo をしない）．mutation は流さない．

読むもの：設計メモ `plan/phase8/41-measurement-records.md`（§2〜§4 と §7），作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/impl-3a-brief.md`．

見ること（4つ）：
1. **規則とテスト**：設計メモ §3 の扱いごと，§4 の 1〜5 ごと，§7 の条件の規則ごとに，profile が緩いと落ち，同じか厳しいと通るテストがあるか．包含は格子全体で比べているか（端だけで比べる誤りを fixture が捕まえるか）．比較テストが MockRadio と UHD Module の両方の実際の terms と block_len で Description を作っているか．照合が空でも通る形（記録が 0 件のとき以外），数だけ見て値を見ない形は指摘．各 mutation の行は，どのテストのどの assert で kill されるか（読んで判断する）．
2. **範囲**：指示書 §2・§3 に対して，足りない所と余計な所．測定スイート，候補の値の仕組み，新しい capability の key，profile の値の変更，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：`Description` の `capabilities()` の key がすべて `rule` に入っているか（grep と unit test）．profile の値を出す関数の key の集合が `capabilities()` と一致するか．`c<n>`・`s<k>` の key がすべての番号で引けるか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/07` に足した規則が DA-1 の形か（規則は数行で `*Test:*` 行で終わる，理由や issue 番号を本文に書かない，Changes に 1 行），コードと食い違う文が無いか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．


## 修正 1 回目（`fix-3a-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3a の実装です（`impl-3a-brief.md`，`report-3a.md`）．orchestrator のレビュー（`orch-3a.md`）と Sol のコードレビュー（`sol-3a.log` の最後の `VERDICT` 以降）が出ました．下の裁定ですべて直します．`impl-3a-brief.md` §0 の守ることはそのまま効く（`cargo fmt` 禁止，ローカルの mutation 禁止，commit 禁止，長い log を読まない，出力はファイルへ）．**この tree を触っている他の agent はいません．他の process を待たない．**

## 1. 裁定

- **Sol 1（major）整数を `f64` にして比べるので，2^53 を超える値で緩い profile が通る．** 一番小さい形で：比べる数（profile と測定，格子の `Integer` の端も）が `f64` で正確に表せない大きさ（絶対値が 2^53 以上）なら，丸めて比べずに違反として拒否する．Sol の 2 つの例（`repeat_max_samples` 9 007 199 254 740 993 対 …992，`Integer` 2^53..=2^53+1 対 2^53 の点）を回帰テストにする（拒否されること）．型を作り直さない．
- **Sol 2（major）ファイルを読む道（`compare_files`）で，片方の Provider の判定を消しても acceptance のテストが落ちない．** fixture のディレクトリに，Mock だけで落ちる記録（lead 3 ms など）と UHD だけで落ちる記録を置き，`compare_files` の道で両方が失敗することを確かめるテストにする（通る記録だけのディレクトリのテストは残す）．各 Provider の判定を消す mutation の行を足す．
- **Sol 3（minor）1 点の格子の包含で刻みの比を要求している．** `Step { lo: 6, hi: 6, step: 2 }` は `{6}` で，`Step { 0, 6, 3 }` に含まれる．端を確かめた後，`first == last` ならその 1 点が含まれるかだけを見る．1 点の場合と，`Integer` の profile の各組み合わせ（対 `Values`，`Integer`，`Step`）のテストを足す．
- **Sol 4（minor）「measured の値が 1 つも無い記録は拒否」は計画に無い．** 測れなかった値をすべて `unmeasured` と理由で書き，observation だけを持つ記録は P3 (5) どおり正しい記録．この制限と RM-35 の該当文，そのテストと mutation の行を消す（ID は再利用しない）．
- **O1（minor）acceptance のテストが UHD の selector の `block_len` を読み直している．** `radio.rx.block_len` は比べない key なので比較の結果を変えない．helper，そのテスト `measurement_selector_block_len_is_used_for_uhd_only`，P691 を消し，UHD は `DEFAULT_BLOCK_LEN` で作る．

## 2. 確かめること

- 直したテストと新しいテストを各 20 回．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：新しい行と，変えたコードや killing test に触れる行を `remote_mutate.py` で流す（前に mode 644）．全 mutation list の `old` が tree でちょうど 1 回一致すること．
- セルフレビューを 1 回（`review-3a-4.log`，`impl-3a-brief.md` §5 のコマンドと `checklist-3a.md`．free が失敗したら Go）．P1 が出たら直す．もう一度は回さない．

## 3. 報告

`fix-3a-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．**起きたことを書く．していないことは書かない．**


## 修正 2 回目（`fix2-3a-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3a と修正 1 回目です（`fix-3a-brief.md`，`fix-3a-report.md`）．Sol の見直し（`sol-3a-fix.log` の最後の `VERDICT` 以降）と orchestrator の指摘を直します．`impl-3a-brief.md` §0 の守ることはそのまま効く．**この tree を触っている他の agent はいません．他の process を待たない．**

## 1. 直すこと

- **Sol 1（major）acceptance の gate の assert（`crates/ezsdr-acceptance/tests/measurements.rs:53-57` 付近）を消しても，どのテストも落ちない．** 実際のディレクトリを確かめるテストと同じ assert の道（関数にまとめるなど）を，Mock だけで落ちる fixture と UHD だけで落ちる fixture にも通し，それぞれ失敗する（panic するなら `#[should_panic]` ではなく結果を返す形で確かめる方がよい）ことを確かめる．通る fixture の場合も残す．各 Provider の gate の assert を消す mutation の行を足す．
- **Sol 2（minor）RX と TX の数を取り違えても，非対称の範囲のテストが落ちない．** RX が 2 channel，TX が 1 channel の description で，RX の stream・channel `1` を使う正しい記録が違反なしで通ることを確かめる（今の TX の拒否の場合と対にする）．`measurement.rs:476` 付近の RX の枝で `description.tx.len()` を使う mutation の行を足す．
- **orchestrator（minor）`ur_21_a_moved_start_on_a_held_burst_s_is_refused`（`crates/ezsdr-radio-uhd/src/provider/tx.rs:2911`）が壁時計で落ちる．** 修正 1 回目の workspace の実行で 1 度落ちた（`fix-3a-workspace.log`：「B is not held: [25644, 25680]」）．テストは壁時計で動く FakeDevice の上で A を手渡させ，`next` を読んでから B を予約するので，その間に cursor が進むと B が C の開始へ動かず，拒否されない．設計メモ 40 で手渡しが窓の端まで細かく進むようになり起きやすくなった．`crates/ezsdr-radio-uhd/tests/README.md` の規則と，同じファイルの制御された時計のテスト（`provider::test_support` の手動の時計，設計メモ 37 で直したテストの形）に倣い，時計を止めた状態で A を手渡し，B と C の予約と判定を同じ止めた時刻で行う形に直す．テストの主張（B は C の開始へ動いて拒否，`TIME_ERROR { refused }`，`COMMAND_REJECTED`，C は開始を保つ）は変えない．このテストを filter に持つ mutation の行を全リストで grep し，流して killed を確かめる．

## 2. 確かめること

- 直したテストと新しいテストを各 20 回（`ur_21_a_moved_start_on_a_held_burst_s_is_refused` は 50 回）．`cargo test --workspace`，clippy -D warnings，`rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．
- mutation：新しい行と，変えたテストを filter に持つ行を `remote_mutate.py` で流す（前に mode 644）．
- セルフレビューは回さない．

## 3. 報告

`fix2-3a-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**

