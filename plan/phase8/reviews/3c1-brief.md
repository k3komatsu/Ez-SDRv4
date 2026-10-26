# 段階 3c-1：指示書

## 実装（`impl-3c1-brief.md`）

あなたは実装役です．HEAD は `6cbbbe4`（設計メモ 41 §8）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．** owner の決め（2026-10-26）で risky でない扱い：テスト計画もそのレビューも無く，すぐ実装する．

### 0. 守ること

`cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる），ローカルの `mutate.py` 禁止，mutant を作業ツリーに当てない，commit・push・`git add`・`git stash` 禁止，version を上げない，SCHEMA_CHANGELOG に書かない．`src/` は変えない（要るなら止めて報告に書く）．cargo とテストの出力はファイルに出して `tail` か `grep` で見る．自分の log や長い出力を文脈に読み込まない．負荷をかけない．テストの一時ファイルは /tmp のまま（TMPDIR を変えない）．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．`mutations.json` の新しい行は既存の最後の行と同じ字下げ（`  {` と 3 space の key）で．**起きたことを書く．していないことは書かない．**

### 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md` §5，§8．`plan/phase8/reviews/3b2-opus.md`（3b-2 で major になった形：実機でだけ起きる完了の証拠，止まりの時刻，前提の失敗，食い違う記録の保存）．
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`（3b-1・3b-2 の全部：`MeasurementPlan`，`candidate_session`，`extract_trial_evidence`，`classify_trial`，`summarize_candidates`，`measure_rx_start_lead`，`measure_rx_throughput`，`build_record`，`write_record_before_profile_assert`），`tests/common/mod.rs` の `rehearse_overflow`，`tests/hardware.rs` の `hw_measurements_record`，`hw_b5_overflow`，`hw_b8_cold_change_timing_at_the_extremes`（Manifest の `timing` の segment の行 `origin` と `at`），`tests/README.md`．
- `design/07-radio-model.md` RM-17（overrun は `[f, f + overflow_restart_gap_ns)` の sample を失う），RM-25；`design/18-uhd-radio.md` UR-21（device lead の判定），UR-23（`cold` の送信の変更で burst が終わる），UR-25，UR-34．`src/provider/mod.rs` の `with_rx_stall`．

### 2. 作るもの

`measurement.rs` に `measure_*` を 3 つ足し，`hw_measurements_record` と fake のリハーサルに入れる．3b-2 の形（候補の段ごと・条件ごとに Run，試行を順に，Manifest から証拠，FakeDevice の記録では測った key も `unmeasured`）に倣う．

| key | 測り方 | 成功 | 読み方 |
|---|---|---|---|
| `radio.timing.tx_start_lead_ns` | 候補 S（builder `with_measurement_timing` の `tx_start_lead_ns`）ごとに Session 1 つ．送信を有効にし，試行ごとに送信の rate を `cold` で変える（1 ⇄ 2 MS/s を交互．もう一つの条件は送信の rate の格子から 500 kS/s ⇄ 1 MS/s のように選び，選んだ理由を報告に） | 新しい送信の segment の `timing` の行で `at`（始めた時刻）が `origin` より前，試行の区間に `TIME_ERROR`・`LATE_COMMAND`・`COMMAND_REJECTED`・async の失敗なし．加えて，新しい clock の origin に burst を置き，それが on time で完全（3b-2 の送信の完了の規則）．**この burst の予約が試験のスレッドの時刻に依るなら**（新しい clock が見えるのが origin の直前など），burst の条件は入れず，`at < origin` とエラーなしだけで判定し，その理由を報告に書く | 最小の lead（成功の suffix），2 条件の交わり，値の行は効いた条件 |
| `radio.timing.overflow_restart_gap_ns` | rate ごと（1，10，25，50 MS/s）に，試行ごとに Spec の受信の Run を 1 つ，`with_rx_stall(after, duration)` で overrun を 1 回起こす（`rehearse_overflow` の形．panic せずに証拠を返す形に分ける） | `RX_OVERFLOW`（cause overrun）がちょうど 1 つ，capture の continuity に `OverflowRestart` の gap がちょうど 1 つで `lost == len`，payload の `lost` と一致．gap の長さ（ns）をその試行の値に | 成功した試行の gap の**最大**（LowerBound：profile ≥ 測定）．rate ごとに成功の数が `EZSDR_MEASURE_OVERFLOW_TRIALS`（既定 20）に届かなければ `unmeasured`．値の行の condition は最大が出た rate．overrun が起きなかった試行は不完全（host の buffer が吸った）で，値に入れない |
| `radio.perf.tx_bytes_per_s` | rate（10，20，25，40，50，100，200 MS/s）ごとに送信の Run を 1 つ，repeat を D 秒（`EZSDR_MEASURE_THROUGHPUT_SECS`） | `TX_UNDERFLOW` と async の `Underflow`・`UnderflowInPacket`・`SeqError*` なし，stats の送信 sample 数が記録された送信の SampleClock の区間（origin から終わり．終わりの記録が無ければ不完全）× rate と packet 1 つの差以内（3b-2 の受信と同じ規則） | 容量（成功の prefix）× `wire_bytes_per_sample` |

- 3b-1 の読み方（`summarize_candidates`）に「最大」の型が無ければ，overflow の gap 用に小さな純粋な関数を足す（成功の数の不足は `unmeasured`）．
- 段と回数の既定値：tx start lead は rx と同じ `[1, 2, 5, 10, 25, 50, 100] ms`，試行は `EZSDR_MEASURE_STEP_TRIALS`．`with_rx_stall` の `after` と `duration` は `hw_b5_overflow` の値を出発点に，rate ごとに overrun が起きる大きさを選び，選び方を報告に（INFERRED）．
- 各測定の段・条件・分類ごとの数，gap の長さの全部，送信の区間と数は observation に残す．
- `unmeasured_reason` の 3 つの key の理由を「3c-1 で測る」に合うよう直す（FakeDevice の記録では今の「FakeDevice は実機の証拠ではない」）．
- bench の時間の見積もり（Run の本数と装置の時間）を報告に．

**純粋なテスト**（合成した Manifest の section で）：送信の start lead の成功と，`at >= origin`・エラーあり・burst の欠けで不成功；overflow の gap が 0 個・2 個・`lost != len`・overrun でない cause で不成功，最大の読み方と成功の数の不足；送信の throughput の止まりの遅れ（成功）と sample の欠け・underflow（不成功）．

**fake のリハーサル**：3b-2 の `provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements` に 3 つを足す（段を 1〜2 つ，試行 1 回）．assert は時刻に依らないものだけ（observation があり試行の数が設定どおり，分類が付く，記録が round trip，測った key は `unmeasured`）．リハーサルの時間を延ばしすぎない（今 31 s．足す分は数 s に）．

### 3. 範囲

上の 3 つと記録への道だけ．queue depth，H1〜H5，#56，coercion，UR-17，負荷，2 channel，BP4b・BP4c は 3c-2・3c-3．Provider と本番の道は変えない．新しい rig の種類，fake の fault，retry は足さない．大きさの目安は約 1,500 行以内．

### 4. 確かめること

- 純粋なテストは 1 回．fake のリハーサルは 20 回（時刻とスレッドに依るため）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．`hardware.rs` は `docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`（実機は流さない）．
- mutation：各測定の成功の判定，読み方，値の行に行を足す（ID は既存の最大の次から）．新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
- spec：`design/18-uhd-radio.md` UR-34 の Test に新しいテストの名前（DA-1 の形）．Changes に 1 行（設計メモ 41，stage 3c-1）．

### 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3c1.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3c1-1.log 2>&1
```

前景で流す．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．区間の外の失敗の証拠を捨てる，成功の条件を弱める，といった指摘は採る前に 3b-1・3b-2 の規則（`3b2-opus.md`）と照らす．

### 6. 報告

`report-3c1.md`．最初の行は `STATUS: done`．作ったもの（ファイルごと），この指示書から変えた所と理由，key ごとのテスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．

## セルフレビューの checklist（`checklist-3c1.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `6cbbbe4`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3c1-brief.md`（§2 の表が仕様）．

見ること（4つ）：
1. **測定とテスト**：表の 3 つの key ごとに，`measure_*` が表の「1 試行」「成功」「読み方」のとおりか．成功の判定が証拠の欠け（record が無い，sample 数が足りない，ack が無い，区間の外の report）を成功にしないか．Manifest から証拠を取る関数が合成した section で分類ごとに試されているか．fake のリハーサルが時刻に依る assert（分類の結果，値の大きさ）をしていないか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．`src/` の変更，queue depth・H1〜H5・2 channel の測定，新しい rig の種類・fault・retry，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：測った key だけが `Measured` で，他の key は理由つきの `unmeasured` か．`source: "fake"` の記録で `Measured` が無いか．raw の probe の結果が値の行に入らないか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3c1-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3c-1 の実装です（`impl-3c1-brief.md`，`report-3c1.md`）．orchestrator のレビュー（`orch-3c1.md`）と Sol のコードレビュー（`sol-3c1.md`）が出ました．下の裁定ですべて直します．`impl-3c1-brief.md` §0 の守ることはそのまま効く（`cargo fmt` 禁止，ローカルの mutation 禁止，commit 禁止，`src/` は変えない，長い log を文脈に読まない，出力はファイルへ）．**この tree を触っている他の agent はいません．他の process を待たない．**

### 1. 裁定

- **Sol 1＋Sol 2（major）：送信の throughput の数の比べ方．** 送信の SampleClock は Stop で終わらない（`provider/core.rs:703-717`，`provider/tx.rs:939-955`）ので今は全段が不完全になる．また clock の origin は arm の時で repeat は T0 から（約 2 s の差），`tx_samples` は host が渡した数で stop の cut より先の分（in-flight の窓）を含み，RX の packet の大きさは送信の許容の根拠にならない．規則を次にする：
  - 走らせる長さ：repeat の burst の始まり（その Run が記録した burst の target．Manifest のどこから取るかはコードを読んで決め，報告に `path:line`）から D 秒先まで `advance_root` する（今の「clock の origin ＋ D」をやめる）．
  - 成功：`TX_UNDERFLOW` と async の `Underflow`・`UnderflowInPacket`・`SeqError*`・`TimeError` が無い，`rejected` に short send / send did not complete が無い，**`BurstAck` がちょうど 1 つ**で，その tick が burst の始まり ＋ D 以後（D 秒以上流れた），`tx_samples` が D × rate 以上．underflow なしの連続の burst では装置が流した数は渡した数と等しいので，上側の許容は要らない．
  - 失敗の証拠の分類は今のまま（DeviceFailure）．ack が 0 個は不完全，2 個以上は帰属できない．
  - observation に burst の始まり，ack の tick，`(ack − 始まり)` から出した sample 数，`tx_samples`，その差を残す（bench で読む）．
  - RX の packet の大きさを読むための Session（`packet_run`）と `packet_samples` の許容をやめる（fake のリハーサルの時間も戻るはず）．
  - 合成のテスト：成功，ack 0 個，ack 2 個，ack が始まり ＋ D より前，`tx_samples` が D × rate より少ない，underflow あり．
- **Sol 3 ＝ O1（major）：overflow の値が試験の stall の長さに依る．** Sol が UHD 4.10 の source で確かめた：受信の再開は reader の `recv` が溜まりを読み切って overrun を見た後に始まる（`rx_streamer_zero_copy.hpp` の `get_recv_buffs()`，`rfnoc_rx_streamer.cpp` の `_handle_overrun()`）．gap 全体は stall と host の buffer の関数で，装置の性質でない．次のようにする：
  - `radio.timing.overflow_restart_gap_ns` の値の行は常に `unmeasured`．理由：「the continuity gap includes the configured reader stall and host-buffer drain; the restart part is an inferred estimate, and whether it is the profile's quantity waits for the first bench reading (owner)」に当たる文．
  - 試行ごとの observation に，gap 全体（今のまま）に加えて `restart_after_resume_ns` を足す：gap の終わり（gap の start ＋ len，capture の SampleClock の sample）から，reader が読み直した時刻の推定（capture の最初の sample ＋ `after` ＋ `stall`，同じ SampleClock の sample に直す）を引いた時間（ns）．INFERRED：stall の起点は host が最初の block を受け取った時刻で最初の sample より遅いので，推定は早めに出て，この値は大きめ（溜まりを読み切る時間も含む）．負なら null と理由．rate ごとの最大と全試行の値も observation に．
  - 3 つのうちこの key だけは記録の値にしないので，`summarize_overflow_gaps` は observation の要約に使う（値の行を作らない）．成功の判定（`extract_overflow_trial`）はそのまま．
  - 既定の試行数 `EZSDR_MEASURE_OVERFLOW_TRIALS` を 5 に下げる（値にしない observation に 20 分は要らない）．
  - 合成のテスト：完全な試行が揃っても値は `unmeasured`，`restart_after_resume_ns` の計算（正，負→null）．
- **Sol の他の確認（tx_switch は構成の終わりで burst の証拠でない）**：指示書が許した範囲．stage の `burst_criterion` の文に「tx_switch.at は構成の終わりと clock の採用で，送信した burst の証拠ではない」と足すだけ．

### 2. 文書

- `design/18-uhd-radio.md` UR-34 の Test の名前を今のテストに合わせる．Changes の 3c-1 の行を「TX start-lead and TX throughput measurements feed their measured keys; the overflow group records the full gap and an inferred restart-after-resume estimate as observations and leaves its key unmeasured; …」に当たる文に．
- `tests/README.md` の段落を今の形に合わせる．
- `report-3c1.md` の bench の見積もり（Run の本数と装置の時間）を直す（fix の報告に新しい表を書く．`report-3c1.md` は書き換えない）．

### 3. 確かめること

- 直したテストと新しいテストの純粋なものは 1 回．fake のリハーサルは変えたので 20 回．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`，`hardware.rs` を変えたら docker の `--features uhd` の clippy（`impl-3c1-brief.md` §4）．
- mutation：新しい行と，変えたコードや killing test に触れる行（P776–P790 のうち触れるもの）を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID>...` で（前に変えたファイルを mode 644）．消したコードの行は mutations.json から消すか新しいコードに付け替える．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．新しい行は `  {` と 3 space の key．
- セルフレビューは回さない（Sol が修正を再レビューする）．

### 4. 報告

`fix-3c1-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，新しい bench の見積もり，残ったこと．**起きたことを書く．していないことは書かない．**
