# 段階 3c-2a：指示書

## 実装（`impl-3c2a-brief.md`）

あなたは実装役です．HEAD は `7c58bc8`．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．** owner の決め（2026-10-26）で risky でない扱い：テスト計画もそのレビューも無く，すぐ実装する．

### 0. 守ること

`cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる），ローカルの `mutate.py` 禁止，mutant を作業ツリーに当てない，commit・push・`git add`・`git stash` 禁止，version を上げない，SCHEMA_CHANGELOG に書かない．`src/` は変えない（要るなら止めて報告に書く）．cargo とテストの出力はファイルに出して `tail` か `grep` で見る．自分の log や長い出力を文脈に読み込まない．負荷をかけない．テストの一時ファイルは /tmp のまま（TMPDIR を変えない）．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．`mutations.json` の新しい行は既存の最後の行と同じ字下げ（`  {` と 3 space の key）で．**Sol のレビューは orchestrator が起こす．あなたは Sol（`openai/gpt-6.1-sol`）を起こさない．** **起きたことを書く．していないことは書かない．**

### 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md` §3（key ごとの扱い），§8（3c-2a の行と，その下の「raw の `Device` を値に使う例外」の段落）．`plan/phase8/reviews/3c1-opus.md` と `3b2-opus.md`（risky でない段階で major になった形：実機でだけ起きる完了の証拠，止まりの時刻，前提の失敗）．
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`（`MeasurementPlan`，`build_record`，値の行と observation の作り方，`unmeasured_reason`），`tests/hardware.rs` の `hw_b8_raw_queue_depth`（522-549 行あたり）と `hw_measurements_record`，`SAFE_TX_HZ`（315 行）と `raw_tx`，`tests/fake.rs` の `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`．
- `crates/ezsdr-radio-uhd/src/device.rs`：`Device::apply`（186 行，`at` が `Some` なら timed），`Applied`（62 行：`rate`，`freq`，`gain`，`antenna` は untimed で antenna を名指した変更だけ `Some`）．FakeDevice には timed command の queue の back-pressure が無い（リハーサルでは止まる呼び出しが出ない）．
- `crates/ezsdr-radio/src/measurement.rs`：`profile_value`（348-370 行：channel の key の `Worst` の形），`grid_subset`（588 行〜：profile の `Step` と測った `Values` の比べ方）．
- `plan/phase7/bench-results.md` 439-452 行：OBX の timed tune は 7 個積めて 8 個目の `apply` が queue の空くまで（3 s）止まった（受信・送信とも）．`design/18-uhd-radio.md` 22 行（UHD の `set_command_time`：timed command は後の timed command を back-pressure する．VERIFIED）．

### 2. 作るもの

`measurement.rs` に `measure_command_queue_depth` と `measure_coercion` を足し，`hw_measurements_record` と fake のリハーサルに入れる．どちらも `Arc<dyn Device>` を直接使う（Provider の Run は開かない）．`hw_measurements_record` では Provider の測定の前か後かを決め，測定の後に装置を既定の状態（受信・送信の周波数は既定 1 GHz，利得 0，既定の antenna，bench の rate）に戻す．

#### 2.1 `radio.timing.command_queue_depth`（容量，p ≤ m）

| | |
|---|---|
| 1 試行 | 先の時刻 T（既定 3 s）の後に，1 ms ずつずらした timed な `apply` を順に呼ぶ（最大 K = 64 個）．各呼び出しの時間（host の時計）と，返った直後の `time_now` を残す．呼び出しの時間が T の半分を越えたら止める．その後，積んだ最後の時刻 ＋ 余裕まで待ち，untimed の `apply` が通ることを確かめる |
| 条件 | 受信の周波数（999.6 MHz ⇄ 1000.4 MHz を交互．LO が動く），送信の周波数（同じ値．`SAFE_TX_HZ` の中．送信の streamer は開かない），受信の利得（0 ⇄ 10 dB），送信の利得（同），受信と送信の周波数を交互に（同じ radio の queue を分け合うか） |
| 成功 | 止まった呼び出しがちょうど 1 つあり，その返りの `time_now` が最初に積んだ時刻以後（queue が空くまで止まった証拠），それより前の呼び出しはどれも T の半分よりずっと短い（既定の閾は 100 ms），どの呼び出しもエラーでない．深さ ＝ 止まった呼び出しより前に返った数．止まる呼び出しが K 個までに無ければ不完全（深さ ≥ K は値にしない），エラーは装置の失敗 |
| 読み方 | 条件ごとに成功の数が `EZSDR_MEASURE_QUEUE_TRIALS`（既定 5）に届かなければ `unmeasured`．届けば全条件・全試行の深さの**最小**．値の行の condition は最小が出た条件 |

#### 2.2 coercion の read-back（bench の layout の全 channel と全 stream）

untimed の `apply` で要求し，返った `Applied` を読む．比べる値は profile の Description（`profile_value` と同じ所）から取る．送信の側は streamer を開かず，周波数を `SAFE_TX_HZ` の中（既定 1 GHz）に置いてから変える．

| key | 測り方 | 値（`Worst`） |
|---|---|---|
| `radio.{rx,tx}.c<n>.gain_db` | profile の利得の格子の全点を要求 | 読み返しが要求と等しかった点の `Grid::Values`（等しいかは 1e-9 dB 以内） |
| `radio.{rx,tx}.c<n>.gain_step_db` | lo から hi まで profile の刻みの 1/5 ずつ要求 | 読み返しの異なる値を並べた隣どうしの差の最大（`Number`．装置の一番粗い刻み） |
| `radio.{rx,tx}.c<n>.antenna` | profile の antenna を全部要求 | 読み返し（`Applied.antenna`）が要求と等しかった名前の `Texts` |
| `radio.{rx,tx}.s<k>.sample_rate_hz` | profile の rate の格子の全点を，その stream の channel で要求（格子の形を読んで，点の列にできなければ報告に） | 読み返しが要求と等しかった点の `Grid::Values`（相対 1e-12 以内） |
| `frequency_hz`，`frequency_step_hz` | 受信は profile の周波数の格子の上で lo から hi まで 50 点，送信は `SAFE_TX_HZ` の中で 20 点を要求し，要求と読み返しの差を observation に | 値にしない（`unmeasured`．理由：連続の格子を抜き取りで測ったとみなすか，送信の RF 安全の外をどうするかは owner の判断待ち） |

- 要求と読み返しの全部（点ごと）は observation に残す．
- 1 点でもエラーが返った key は `unmeasured`（理由にエラー）．
- 値の行の condition は既存の規則（設計メモ 41 §2，`build_record` の検査）に合う形で．合わせ方は報告に．

#### 2.3 共通

- `unmeasured_reason` の `command_queue_depth` と channel・stream の key の文を「3c-2a で測る」に合うよう直す（FakeDevice の記録では今の「FakeDevice は実機の証拠ではない」）．`frequency_hz`・`frequency_step_hz` は上の理由．
- bench の時間の見積もり（呼び出しの数と装置の時間）を報告に．

**純粋なテスト**（合成した呼び出しの列と読み返しで）：queue の成功，止まりなし（不完全），止まりが短い（不完全），止まりの返りが最初の時刻より前（不完全），前の呼び出しが閾を越える（不完全），エラー（失敗），最小の読み方と成功の数の不足；利得の格子（一部の点が違う値で返ると Values から外れ，比較テストで profile の Step ⊄ 測定），利得の刻み（0.5 dB の装置で 0.5，1 dB の装置で 1），antenna（返りが違う名前），rate（違う rate で返る点）．`grid_subset` で比べて落ちる場合と通る場合を少なくとも 1 つずつ（`ezsdr_radio::measurement` の比較の関数を通して）．

**fake のリハーサル**：上の 2 つを足す（quick の plan で T と待ちを短く，試行 1 回）．assert は時刻に依らないものだけ（observation があり試行と点の数が設定どおり，分類が付く，記録が round trip，測った key は `unmeasured`）．リハーサルの時間の増えは数 s に．

### 3. 範囲

上の 2 つと記録への道だけ．H1〜H5，#56，UR-17，負荷，2 channel，BP4b・BP4c は後の段階．Provider と本番の道は変えない．新しい rig の種類，fake の fault，retry は足さない．FakeDevice に queue の模型を足さない．大きさの目安は約 1,200 行以内．

### 4. 確かめること

- 純粋なテストは 1 回．fake のリハーサルは 20 回（時刻とスレッドに依るため）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．`hardware.rs` は `docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`（実機は流さない）．
- mutation：各判定，読み方，値の行に行を足す（ID は既存の最大の次から）．新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
- spec：`design/18-uhd-radio.md` UR-34 の Test に新しいテストの名前（DA-1 の形）．Changes に 1 行（設計メモ 41，stage 3c-2a）．

### 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3c2a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3c2a-1.log 2>&1
```

前景で流す．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．成功の条件を弱める指摘は採る前に `3b2-opus.md`・`3c1-opus.md` と照らす．

### 6. 報告

`report-3c2a.md`．最初の行は `STATUS: done`．作ったもの（ファイルごと），この指示書から変えた所と理由，key ごとのテスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．

## セルフレビューの checklist（`checklist-3c2a.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `7c58bc8`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3c2a-brief.md`（§2 の表が仕様）．

見ること（4つ）：
1. **測定とテスト**：表の key ごとに，`measure_*` が表の「1 試行」「成功」「読み方」のとおりか．成功の判定が証拠の欠け（止まりが無い，止まりが短い，止まりの返りが最初の時刻より前，読み返しが無い）を成功にしないか．判定の関数が合成した呼び出しの列と読み返しで分類ごとに試されているか．fake のリハーサルが時刻に依る assert（分類の結果，値の大きさ）をしていないか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．`src/` の変更，H1〜H5・#56・UR-17・2 channel の測定，FakeDevice の queue の模型，新しい rig の種類・fault・retry，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：測った key だけが `Measured` で，他の key は理由つきの `unmeasured` か．`source: "fake"` の記録で `Measured` が無いか．値の行は §2 の 2 つの測定からだけで，`frequency_hz`・`frequency_step_hz` は `unmeasured` か．送信の側で streamer を開かず周波数が `SAFE_TX_HZ` の外に出ないか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3c2a-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3c-2a の実装です（`impl-3c2a-brief.md`，`report-3c2a.md`）．orchestrator のレビュー（`orch-3c2a.md`）と Sol のコードレビュー（`sol-3c2a.md`）が出ました．下の裁定ですべて直します．`impl-3c2a-brief.md` §0 の守ることはそのまま効く（`cargo fmt` 禁止，ローカルの mutation 禁止，commit 禁止，`src/` は変えない，長い log を文脈に読まない，出力はファイルへ）．**この tree を触っている他の agent はいません．他の process を待たない．Sol（`openai/gpt-6.1-sol`）は起こさない（再レビューは orchestrator が起こす）．セルフレビューも回さない．**

### 1. 裁定

- **Sol 1（major）：queue の分類が，前の呼び出しの返りの時刻を見ていない．** 呼び出しの間で host が止まって先の command が実行され queue が空いた後に，後の呼び出しの host の止まりを back-pressure と取り違えると，同時には積まれていない数を深さにしてしまう．規則を次にする：
  - 止まった呼び出しより前の呼び出しは，どれも返りの `time_now` が最初の command の時刻より前（その時までに queue のどの command も実行されていない）．どれか一つでも以後なら不完全（理由を残す）．返りの時刻が無いものも不完全．
  - 止まった呼び出しの返りの `time_now` は最初の command の時刻以後で，かつ最初の command の時刻 ＋ 上限以内（queue が空くのは最初の command の実行で，その直後に返るはず．上限は既存の `QUEUE_PREVIOUS_CALL_LIMIT_NS` と同じ 100 ms．定数の名前は意味に合わせてよい）．遅すぎる返りは不完全．
  - 合成のテスト：前の短い呼び出しの返りが最初の時刻以後（不完全），止まった呼び出しの返りが最初の時刻 ＋ 上限より後（不完全），境目（最初の時刻ちょうど・上限ちょうど）．それぞれに mutation の行．
  - observation と報告に：深さは「時刻つきの `apply` の呼び出し（高い水準の操作）」の数で，FIFO の entry の数ではない（Sol が UHD 4.10 の source で確認：OBX の timed tune は LO の register 書き込みと同期の書き込みを複数出す，`obx_expert.cpp`，`obx_cpld_ctrl.cpp`）．Provider（UR-24）も 1 command を 1 slot と数えるので，比べる量はこれでよい．back-pressure を host の呼び出しの時間と device の時刻から読むのは INFERRED（呼び出しが credit を待っていたことの直接の証拠ではない）と書く．
- **Sol 2（minor）**：`design/18-uhd-radio.md` UR-34 の本文から「from stage 3c-2a」を除く（経過は Changes に）．
- **O1（minor）**：`radio.{dir}.s<k>.sample_rate_hz` の k を channel の番号から取らず，Description（layout）の stream から取る：各方向の stream ごとに，その stream に入る最初の channel で要求する．Description にその情報が無ければ今のままにし，code の 1 行の注釈（`shortcut:` の形）と報告に．

### 2. 確かめること

- 直したテストと新しいテストの純粋なものは 1 回．fake のリハーサルを変えたら 20 回（変えなければ 1 回）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`，`hardware.rs` を変えたら docker の `--features uhd` の clippy．
- mutation：新しい行と，変えたコードや killing test に触れる行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID>...` で（前に変えたファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．新しい行は `  {` と 3 space の key．

### 3. 報告

`fix-3c2a-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**

## 修正 2（`fix2-3c2a-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3c-2a の実装と 1 回目の修正です（`fix-3c2a-brief.md`，`fix-3c2a-report.md`）．Sol の再レビュー（`sol-fix-3c2a.md`）が新しい major を 1 つ出しました．下の裁定で直します．`impl-3c2a-brief.md` §0 の守ることはそのまま効く．**他の agent はいません．他の process を待たない．Sol は起こさない．セルフレビューも回さない．**

### 1. 裁定

**Sol 再レビューの major：呼び出しごとの `time_now` が，測っている queue を乱す．** UHD 4.10 の `get_time_now()` は radio の register を control port で読む（`radio_control_impl.cpp:388-404`），control port は操作を一つずつ ack を待って処理する（`axis_ctrl_slave.v:201-242`）．だから timed な `apply` の直後の `time_now` は，積んだ timed command の後ろで最初の command の時刻まで待たされる（VERIFIED，Sol）．Phase 7 の raw の probe（`hw_b8_raw_queue_depth`）は呼び出しの間に `time_now` を読まず，7 個の後の 8 個目が 3 s 止まった．

規則を次にする：

- 試行の中で device の時刻を読むのは，**最初の timed な `apply` の前の 1 回だけ**．その読みの前と後の host の時刻（`Instant`）を `h_before`，`h_after`，読んだ値を `d0` とする．最初の command の時刻 `first_at` は `d0` ＋ T．
- 呼び出しの間では `time_now` を読まない．各呼び出しの返りの host の時刻 `h_i` だけを残す．
- device の時刻の上下の範囲は host の時計で出す（両者は同じ速さで進むとみなす．drift は INFERRED で無視できる大きさ．observation に書く）：
  - 上限：`device(h) ≤ d0 + (h − h_before)`（読みの値は `h_before` より後に取られた）．
  - 下限：`device(h) ≥ d0 + (h − h_after)`（読みの値は `h_after` より前に取られた）．
- 止まった呼び出しより前の呼び出しは，どれも返りの上限が `first_at` より前．
- 止まった呼び出しの返りは，下限が `first_at` 以後で，上限が `first_at` ＋ 100 ms 以内．
- 読みの往復（`h_after − h_before`）が大きすぎると範囲が広がって判定できないので，それが 100 ms を越えたら試行は不完全（理由を残す）．
- 待ち（積んだ最後の時刻 ＋ 余裕まで）と，その後の untimed の `apply` はそのまま（待ちの中の `time_now` は queue が空くまで止まるだけで，判定には使わない）．試行の終わりに読んだ device の時刻は observation に残してよい．
- observation に，各呼び出しの host の時刻からの上下の範囲，`d0`，`h_after − h_before` を残す．「back-pressure を host の時間から読むのは INFERRED」の文は残す．
- 合成のテスト：今の境目の場合を新しい形（`d0`，`h_before`，`h_after`，`h_i`）で書き直す．加えて，読みの往復が上限を越える（不完全），前の呼び出しの上限が `first_at` ちょうど（不完全），止まった呼び出しの下限が `first_at` の 1 tick 前（不完全）．それぞれに mutation の行．使わなくなった行は mutations.json から消すか付け替える．
- fake のリハーサル：FakeDevice の `time_now` は queue の後ろで待たないので，形だけの assert はそのまま通るはず．時刻に依る assert を足さない．

### 2. 確かめること

1 回目の修正の指示書 §2 と同じ（純粋なテスト 1 回，fake のリハーサルを変えたら 20 回，`cargo test --workspace`，clippy，`rules.py --base HEAD`，`git diff --check`，`hardware.rs` を変えたら docker の clippy，触れた mutation の行を `remote_mutate.py maintenance sim02:4,sim03:4`，全 list の `old` が 1 回一致）．

### 3. 報告

`fix2-3c2a-report.md`．最初の行は `STATUS: done`．直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**
