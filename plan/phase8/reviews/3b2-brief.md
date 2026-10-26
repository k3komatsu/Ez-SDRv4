# 段階 3b-2：指示書

## 実装（`impl-3b2-brief.md`）

あなたは実装役です．HEAD は `3678f2c`（段階 3b-1）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．** owner の決め（2026-10-26）で，この段階は risky でない扱い：テスト計画もそのレビューも無く，すぐ実装する．

### 0. 守ること

`cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる），ローカルの `mutate.py` 禁止，mutant を作業ツリーに当てない，commit・push・`git add`・`git stash` 禁止，version を上げない，SCHEMA_CHANGELOG に書かない．cargo とテストの出力はファイルに出して `tail` か `grep` で見る．自分の log や長い出力を文脈に読み込まない．負荷をかけない．テストの一時ファイルは /tmp のまま（TMPDIR を変えない）．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．**起きたことを書く．していないことは書かない．**

### 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md` §5，§7 の末尾（「3b の指示書へ」）．`plan/phase8/00-overview.md` §4 の P3（(1) 読み方，(2) 回数，(6) Module の内部の定数，(6b) Provider 経由と raw）．
- 3b-1 の記録：`plan/phase8/reviews/3b1-opus.md`，`reviews/3b1-sol.md` の「テスト計画のレビュー 1」の major 1〜7（ここで答える：特に 5 完了の判定，6 帰属，7 Run の使い方と時間）．
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`（3b-1：`TrialEvidence`，`attribute_evidence`，`classify_trial`，`summarize_candidates`，`build_record`，`unmeasured_reason`），`tests/common/mod.rs`（`session`，`spec_run`，`assembly(..., adjust)`，`bench_profile`，`rehearse_session_loopback`），`tests/hardware.rs` の `hw_b8_leads`，`hw_b8_preemption`，`cold_change_capture`，`hw_perf_rx_rates`，`hw_b7_session_loopback`，`hw_measurements_record`．`tests/README.md`（試験のスレッドの時刻を判定に使わない）．
- `crates/ezsdr-radio/src/measurement.rs` の `ValueRecord::Measured`，`Worst`，`rule`（lead 型は下限，`rx_bytes_per_s` は上限，`full_duplex`・`hardware_time` は「真なら真」）．
- `design/18-uhd-radio.md` UR-21（preemption の境界：手渡し済みの最初の sample と device lead の遅い方），UR-25（`cold` の rate change），UR-34．

### 2. 作るもの

`tests/common/` に `measure_*` を 5 つ．各々 `Arc<dyn Device>` と段・回数の設定を受け，値の行（または理由つきの `unmeasured`）と observation を返す．`hardware.rs` の `hw_measurements_record` は raw の probe に続けてこの 5 つを呼び，`build_record` に測った値を渡す（3b-1 の「すべて `unmeasured`」を，測った key だけ `Measured` に変える．他の key の理由はそのまま）．

**Run の使い方（Sol 7）**：段（候補の値）ごと，条件ごとに Run を 1 つ開き，その中で試行を順に一つずつ行う．試行の間に設定した間隔（既定 20 ms，前の試行の期待した終わりから）を置く．試行の区間は「その試行の予約の時刻から，次の試行の予約の時刻まで」（device の tick）で，3b-1 の `attribute_evidence` に渡す．Run の終わりに Manifest から証拠を取り出す．

**Manifest から証拠を取る関数（Sol 5，6）**：Manifest の `bursts`（target，sample 数，終わり），`radio.TIME_ERROR`（outcome と tick），`async`（code と tick），`rejected`（UR-22 の短い send の note），`radio.LATE_COMMAND`・`COMMAND_REJECTED`・`COMMAND_QUEUE_FULL`・`RX_OVERFLOW`，capture の continuity を，試行の予定（予約の時刻，target，期待する sample 数と終わり，完了の種類）と合わせて `Vec<TrialEvidence>` にする純粋な関数（入力は Manifest の section の JSON と予定）．合成した section で，6 分類のそれぞれと，tick の無い report，区間をまたぐ ack の場合を unit test する．

| key | 測定（元のテスト） | 候補（builder `with_measurement_timing`） | 1 試行 | 成功 | 読み方 |
|---|---|---|---|---|---|
| `radio.timing.min_timed_command_lead_ns` | Session の `send` を受け取りから lead L で（`hw_b8_leads`） | `min_timed_command_lead_ns` = L（Manifest に候補を残すため．Session の `send` の admission は lead を比べない） | 試行のたびに `run.now()` を読み，target = now + L の `send`（pn 100 sample）．実効の lead（Action の受け取りの時刻と target の差）を observation に残す | burst の record が target で始まり，sample 数と終わりが期待どおり，`BurstAck`，`TIME_ERROR` なし，async の失敗なし | 最小の lead（成功の suffix） |
| `radio.timing.tx_in_flight_ns` | 走っている repeat への割り込み（`hw_b8_preemption`） | `tx_in_flight_ns` = W | Run の初めに `start_repeat`（pn 1 000），各試行は lead = W ＋ `DEVICE_LEAD_NS` ＋ `DELIVERY_ALLOWANCE_NS` ＋ 1 ms の `send`（pn 100）で割り込み，次の試行の前に `start_repeat` し直す（lead の余り 1 ms は INFERRED．値を observation に残す） | 割り込む burst が target で始まり完全，`TIME_ERROR` なし，試行の区間に `Underflow`・`UnderflowInPacket`・`TimeError` なし | 最小の lead（成功の suffix） |
| `radio.timing.rx_start_lead_ns` | `cold` の rate change をまたぐ capture（`cold_change_capture`） | `rx_start_lead_ns` = S | 1 試行 = capture 1 本と rate change 1 回．条件 2 つ：1 MS/s ⇄ 2 MS/s，390 625 ⇄ 400 000 S/s（試行ごとに向きを交互に） | continuity が 2 区間で，古い clock は e₁ で終わり，新しい clock は 0 から，gap なし，`LATE_COMMAND` なし | 最小の lead，2 条件の交わり．値の行の `condition` は効いた方の条件 |
| `radio.perf.rx_bytes_per_s` | 受信の rate（`hw_perf_rx_rates`） | なし | rate ごとに Run 1 つを D 秒（試行 1） | `RX_OVERFLOW` なし，stats の受信 sample 数が D × rate と packet 1 つの差以内 | 容量（成功の prefix）× `wire_bytes_per_sample`（Description から） |
| `radio.full_duplex`，`radio.hardware_time` | loopback（`rehearse_session_loopback` の形） | なし | 試行ごとに Run 1 つ：repeat を送りながら時刻を決めた capture | capture が要求した sample から始まり（`hardware_time`），波形が見つかり（`full_duplex`），`TIME_ERROR`・`RX_OVERFLOW`・async の失敗なし | 全試行成功なら `true`，一つでも失敗なら `false`（「真なら真」の規則で比較が落ち，人が見る） |

- 今の `hw_b8_leads`，`hw_b8_preemption`，`cold_change_capture`，`hw_perf_rx_rates`，`hw_b7_session_loopback` の print と assert は残す（共有の関数に移すなら，その assert も移す）．`rehearse_session_loopback` の assert（panic）を測定が踏まないよう，判定を返す形を分けてよい．
- 段の既定値：lead `[0.5, 1, 1.5, 2, 3, 5, 10] ms`，in-flight `[0.25, 0.5, 1, 2, 3, 5, 10] ms`，RX start lead `[1, 2, 5, 10, 25, 50, 100] ms`，rate `[10, 20, 25, 40, 50, 100, 200] MS/s`．回数は環境変数で：`EZSDR_MEASURE_STEP_TRIALS`（既定 100），`EZSDR_MEASURE_THROUGHPUT_SECS`（既定 60），`EZSDR_MEASURE_LOOPBACK_TRIALS`（既定 20）．0 と不正は失敗．各段の候補・条件・分類ごとの数，実効の lead は observation に残す．bench の時間の見積もりを報告に書く（INFERRED）．
- 値の行の `trials` は選んだ段の試行の数，`test` は `tests/hardware.rs::hw_measurements_record`，`condition` は `build_record` に渡す条件の id（tx-1msps，rx の rate ごと，cold の 2 条件，duplex-1msps．`concurrent` は duplex で rx0・tx0）．
- 3b-1 の `summarize_candidates` に「どの条件が効いたか」を返す小さな追加は可．

**fake のリハーサル**：`fake.rs` で同じ 5 つの `measure_*` を FakeDevice（手動の時計）で，段を 1〜2 つ，試行を 1 回で流し，記録を書いて読み戻す．assert は時刻に依らないものだけ（tests/README.md の (2)）：observation が段ごとにあり試行の数が設定どおり，各試行に分類が一つ付いている，記録が round trip し `compare` の違反に `source` がある．fake の分類の結果や値の大きさは assert しない．`source: "fake"` の記録では，測った key も `unmeasured`（理由「FakeDevice は実機の証拠ではない」）にし，summary は observation に残す．`Measured` の行を作る道は合成した summary の unit test で確かめる．

### 3. 範囲

上の 5 つと，それを記録に入れる道だけ．queue depth は `unmeasured` のまま（3c）．`release_window_ns`，`startup_latency_ns`，`overflow_restart_gap_ns`，`tx_start_lead_ns`，`tx_bytes_per_s` も今の理由のまま．Provider と本番の道は変えない（`src/` は変えない．要るなら止めて報告に書く）．新しい rig の種類，fake の fault，retry は足さない．大きさの目安は約 1,500 行以内．

### 4. 確かめること

- 純粋なテストは 1 回．時刻やスレッドに依るテスト（fake のリハーサル）だけ 20 回．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．`hardware.rs` は `docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` で compile と clippy（実機は流さない）．
- mutation：分類の取り出し，読み方，値の行の作り方，各 `measure_*` の成功の判定に行を足す（1 space の字下げ，ID は既存の最大の次から）．新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
- spec：`design/18-uhd-radio.md` UR-34 の Test に新しいテストの名前を足す（DA-1 の形）．Changes に 1 行（設計メモ 41，stage 3b-2）．

### 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3b2.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3b2-1.log 2>&1
```

前景で流す．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

### 6. 報告

`report-3b2.md`．最初の行は `STATUS: done`．作ったもの（ファイルごと），この指示書から変えた所と理由，key ごとのテスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．

## セルフレビューの checklist（`checklist-3b2.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `3678f2c`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3b2-brief.md`（§2 の表が仕様）．

見ること（4つ）：
1. **測定とテスト**：表の 5 つの key ごとに，`measure_*` が表の「1 試行」「成功」「読み方」のとおりか．成功の判定が証拠の欠け（record が無い，sample 数が足りない，ack が無い，区間の外の report）を成功にしないか．Manifest から証拠を取る関数が合成した section で分類ごとに試されているか．fake のリハーサルが時刻に依る assert（分類の結果，値の大きさ）をしていないか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．`src/` の変更，queue depth の測定，新しい rig の種類・fault・retry，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：測った key だけが `Measured` で，他の key は理由つきの `unmeasured` か．`source: "fake"` の記録で `Measured` が無いか．raw の probe の結果が値の行に入らないか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3b2-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3b-2 の実装です（`impl-3b2-brief.md`，`report-3b2.md`）．orchestrator のレビュー（`orch-3b2.md`）と Sol のコードレビュー（`sol-3b2.log` の最後の `VERDICT` 以降）が出ました．下の裁定ですべて直します．`impl-3b2-brief.md` §0 の守ることはそのまま効く（`cargo fmt` 禁止，ローカルの mutation 禁止，commit 禁止，長い log を文脈に読まない，出力はファイルへ）．**この tree を触っている他の agent はいません．他の process を待たない．**

### 1. 裁定

- **Sol 1（major）＋ O3：`BurstAck` の tick が排他的な終わりと等しいことを要求している．** UHD 4.10 は ack に radio が EOB を処理した時刻を入れる（Sol が source で確認：`radio_tx_core.v:369-375,451-460`，`radio_control_impl.cpp:1314-1317`，`rfnoc_tx_streamer.cpp:320-323`）．排他的な終わりとの差は分からない（INFERRED）．許容の幅を推測で入れない．規則を次にする：試行の区間の中で，その試行の burst の target 以後に来た `BurstAck` がちょうど 1 つなら，それがその試行の完了（試行は一つずつで，一つの試行が作る device の EOB は一つ）．0 個は不完全，2 個以上は帰属できない．sample 数と終わり（burst の record）の確認はそのまま．各試行の ack の tick と期待した終わりの差を stage の observation に残す（bench で意味を読むため）．終わりと等しくない ack で成功する場合，ack の無い場合，二つある場合を合成のテストに．
- **Sol 2（major）：in-flight の測定が，repeat が走っていなくても割り込みの burst だけで成功する．** 各試行で，割り込まれた repeat の burst の record が，割り込みの burst の target でちょうど終わっていること（repeat が割り込みの時まで流れていた証拠）を成功の条件に足す．無ければ不完全．加えて，区間の外の tick を持つ失敗の証拠を捨てない：最初の試行より前のものは最初の試行を，最後の試行より後のものは最後の試行を「帰属できない」にする（3b-1 の規則に戻す．セルフレビューの P2-2 の変更を取り消す）．repeat が無い・途中で失敗した場合と，最初の試行の前の underflow の場合を合成のテストに．
- **Sol 3 = O1（major）：throughput の期待する sample 数が `D × rate` で，止まる時刻は `finish()` が決める．** 期待する数を Manifest の受信の SampleClock の記録の区間（origin から終わり．終わりの記録が無ければ不完全）× rate で出し，packet 1 つの差以内を成功とする．止まりが遅れた場合（区間が D より長いが数は区間どおり → 成功）と，sample が欠けた場合（区間どおりより少ない → 不完全）を合成のテストに．
- **Sol 4（major）：比較が通らないと記録を書かずに panic する．** `hw_measurements_record` は，記録の形の検査（`compare` の違反のうち profile との食い違い以外：key の網羅，condition，source など）が通れば先に書き，その後で profile との比較を assert する．食い違う記録が書かれて残ることを hardware の要らないテストで確かめる（関数に分けて）．
- **Sol 5（minor）：実効の lead と試行の始まりが submit の後の `run.now()`．** 返った log の entry の受け取りの時刻（`entry.time`）を使う．
- **O2（minor）：`rx_start_lead_ns` の値の行の condition が `cold-1msps-2msps` 固定．** summary で効いた条件を返し，それを使う．cold の条件は両方の rate を書く（向きの書き方は既存の `Condition` の形で表せる範囲で．表せなければ報告に書く）．

### 2. 確かめること

- 直したテストと新しいテストは純粋なものは 1 回．fake のリハーサルを変えたらそれだけ 20 回（変えなければ 1 回）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．`hardware.rs` を変えたら docker の `--features uhd` の clippy（`impl-3b2-brief.md` §4 のコマンド）．
- mutation：新しい行と，変えたコードや killing test に触れる行を `remote_mutate.py ... sim02:4,sim03:4 <ID>...` で流す（前に変えたファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
- セルフレビューは回さない（Sol が修正を再レビューする）．

### 3. 報告

`fix-3b2-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**
