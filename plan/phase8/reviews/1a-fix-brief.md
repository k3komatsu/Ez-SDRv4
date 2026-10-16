# 修正の指示書：Phase 8 段階 1a（レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前回の指示書 `~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/brief.md` の「守ること」はすべてそのまま有効です（`cargo fmt`/`rustfmt` 禁止，mutation は `remote_mutate.py` だけ，commit/push/`git add` 禁止，2分を超えるコマンドは nohup で背景，テストを弱めない，版を上げない）．レビューは2本：`~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/opus-review.md`（O1–O6）と `~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/sol-review.md`（末尾の「結論」以降の 1–8）．**両方を全文読んでから**始めること．orchestrator の採否：**全部採用**．以下は各項目の決定です（レビューの修正案と食い違うところはこちらが優先）．

## 直すこと

1. **CBX の位相（O1，Sol 1）**：`random` の channel は，適用された周波数 update のたびに（その方向の各 channel が）新しい位相を引き，effective instant から使う．乱数は device ごとの `SimRng`（例 `SimRng::new(seed, "<device>/lo/retune")`）を channel mode の状態に持ち，update の適用順に引く（MR-30 の決定性を保つ）．`random_unless_timed_tune` は今どおり timed 定数，`deterministic` は 0．MR-34 の文を書き直す（本文は規範だけ）．テストは名前を変えて（例 `mr_34_a_random_phase_is_redrawn_by_every_tune`）：CBX で2回の timed retune（実際に周波数を変える retune を含めること．loopback の周波数 gate が開いている区間で測る）の各前後で両 channel の回転が変わり，2 channel の位相差も変わること．P10 をこの新しいコードに当て直す．
2. **layout は UHD の既定の順の prefix（O2，Sol 2）**：各方向で要素 `i` の slot は `[A, B][i]`（つまり `[]`・`[A]`・`[A,B]` だけ）．重複と順序の検査はこれに置き換える．拒否理由は slot と位置を名乗る．`rm_26_the_layout_parser_checks_the_shared_selector`・`mr_02_from_binding_refusals` に B だけの layout の拒否を足す．P08/P09 を新しい検査の行に直す（kill を確かめる）．MR-2・UR-5 の表の文を合わせる．
3. **空の layout の既定 channel 数（O3，Sol 3）**：`x3xx::description` で `radio.rx.channels` の既定を `min(1, rx.len())` に．テスト：`rx: []`・`tx: ["0:A:ubx:0"]` で Spec が rx を要求しないとき，両 Provider で `radio.rx.channels` の適用値が 0，受信 SampleClock・受信 stream ができないこと（`d3_…` の非対称テストに足すか新しいテスト）．mutation 行を1つ足す．
4. **CBX の `coherent`（Sol 4）**：board ごとに `coherent` を決める（ubx・obx は true，cbx は false）．テスト：CBX の layout で `radio.rx.coherent` に `Eq(true)` を要求すると両 Provider が拒否する．mutation 行を1つ足す．`coherent` を true と仮定している既存のテスト・文書があれば合わせる（grep で全部）．
5. **`rules.py`（O4，Sol 5）**：`KNOWN_MISSING` の変更を元に戻す．`design/07` と `design/18` の 2026-10-17 の Changes 行から旧テスト名を消す（新しい名前だけで「renamed」と書く）．`~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/renames.txt` に `old -> new` を3行書き，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/renames.txt` が 0 problem になること．
6. **Mock の description を一度だけ作る（O5）**：`from_binding` で `DeviceDescription` を作って `Profile` に持ち（`ideal` も），`timing()`・`performance()`・位相の判定などはそれを読む．
7. **R11（Sol 6）**：`plan/phase7/tools/mutations.json` の R11 の `new` を受理条件に合う形（`&& (master_clock_hz as f64 / rate).fract() == 0.0`）に直し，商に丸め誤差を持つ合法な rate で kill されることを確かめる（kill されないなら報告に書く．テストを弱めない）．
8. **文書（O6，Sol 7，Sol 8）**：
   - `design/07-radio-model.md` §2 の位相の行：UBX と OBX は timed でなければランダム，CBX は毎回ランダム（timed の LO 位相同期が無い）．SBX は書かない．
   - Vision §26（`design/vision/05-coherence-calibration-and-runtime-semantics.md` の「Retune phase behaviour」）：値の例に `random` を足し，CBX を timed 同期の説明から分ける．既存の 2026-10-17 の revision history の行（`Ez-SDR_v4_ARCHITECTURE_VISION.md`）にこの変更を足す（行は増やさない）．
   - `design/09-mock-radio.md:48` 付近と `design/18-uhd-radio.md:163–177` 付近：規則の本文から理由・段階の経緯（「so that … cannot …」「not changed now」など）を外し，必要なら Changes 行か設計 33 へ．
   - `design/18-uhd-radio.md:330` 付近の §6 の表：two-UBX の受理など，テストが実際に assert していることを正しく書く．
   - この修正で偽になる文（MR-34，RM-9 の `coherent`，MR-2/UR-5 の layout，MR-3/UR-9 の表など）を grep で全部直す．

## mutation

流すのは，この修正で `old` かコードが変わった行と新しい行だけ（P08，P09，P10，R11，新しい行，ほかに触った行）：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase3,phase7 sim02:4,sim03:4 <ids>`（背景で）．全部 killed で，各行がちょうど1回一致すること．

## 確かめること

前回の 1〜5（`cargo test --workspace --no-fail-fast`，host clippy，Docker の `--features uhd` clippy，Python 3.14 suite，`rules.py --base HEAD --renames <上のファイル>`）と上の mutation．

## セルフレビュー

前回と同じ形で（新しい読み取り専用レビュー役，最大3回，前景）．チェックリストは前回のものに加え：「レビュー O1–O6・Sol 1–8 の各項目が直っているか，直し方が上の決定どおりか」．

## 報告

`~/.cache/ezsdr-fixes/2026-10-17-phase8-1a/fix-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．項目ごとに直したこと（ファイルと要点），テストとそれが捕まえる間違った実装，mutation 行と kill したテスト，確かめることの結果（数とログ），セルフレビューの各回，未解決．
