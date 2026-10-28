# 段階 3c-3b：指示書

あなたは実装役（OpenCode の GPT-6 Luna）です．HEAD は `98b0d9a`（実装を始める時に `git log -1` で確かめる）．この指示書を書いた後に 3e，MA1，3f，3g，43a，TXP が入った：`tests/common/measurement.rs`（queue の試行，受信の start lead の候補 `[3,5,10,25,50,100] ms`，規則 2 の `candidate_timing`），`tests/common/measurement/multichannel.rs`（`GuardedDevice` の `raise_thread_priority` の転送，容量の行の family），`tests/common/probe_device.rs`，`plan/phase8/bench.md` を読み直してから始める．mutation の行の ID は P1124 から．**作業場所は git worktree `/home/komatsu/local_work/Ez-SDRv4-3c3b`（branch `wip/3c3b`，base 98b0d9a）だけ**．main の tree（/home/komatsu/local_work/Ez-SDRv4）は実機に使うので読むことも書くこともしない．worktree の `.cargo/config.toml` は target を `~/.cache/cargo-target/Ez-SDRv4-3c3b` に向けてある．docker の `--features uhd` の clippy は volume `ezsdr-v4-3c3b-target` を使い（`-e CARGO_TARGET_DIR=/cargo-target`），`ezsdr-v4-cargo-target` は使わない．remote_mutate は worktree の root から（ファイルは mode 644，必要なら `chmod -R go+rX`）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる）．
- ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- `src/` は変えない（要るなら止めて報告に書く）．本番の値と既定の道は変えない．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．長い出力を文脈に読み込まない．負荷をかけない（手元で `h4-load.sh` を長く回さない．自己確認は数秒で止める）．
- テストの一時ファイルは /tmp のまま．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．tree に作ったファイルは mode 644（script は 755）にする（Write で作ると 600 になり，remote mutation が全部落ちる）．
- Sol（`openai/gpt-6.1-sol`）は起こさない．実機は流さない．
- **実機の throughput の測定が同じ host で走っている間は cargo の build と test を流さない**（CPU の負荷が測定を乱す）．cargo を流す前に毎回 `ps -eo cmd | grep -c '[h]w_measurements_record'` が 0 であることを確かめ，0 でなければ 60 s ごとに確かめて待つ（読むことと書くことは待たずに進めてよい）．
- 並行や失敗の扱いを足すときは，仕組みを足す前に道の数を減らす形を考える（3c-2c と 3c-3a の教訓：失敗した装置の扱いで Sol の major が 12 件続き，最後は装置の手前の 1 つの `GuardedDevice` にまとめて閉じた．owner はこの形を残すと決めた．この段は新しい guard を足さず，それを使う）．
- **起きたことを書く．していないことは書かない．**

## 1. 背景

読むもの：設計メモ `plan/phase8/41-measurement-records.md`（特に §2・§3・§5・§8），`plan/phase8/bench.md`，`plan/phase8/bench-results.md` の 2026-10-27，`plan/phase8/reviews/3c2c-opus.md`，`plan/phase8/reviews/3c3a-opus.md` と `3c3a-sol.md`（同じ機構の指摘の経過），`plan/maintenance/audits-2026-10.md` の 6（H4 の定義），`plan/phase8/00-overview.md` の P3 と BP4b・BP4c の行，`design/07-radio-model.md` の RM-28・RM-31・RM-32，`design/18-uhd-radio.md` の UR-34．

3c-3 は設計メモ 41 §8 の最後の段で，前半の 3c-3a（selector，多 channel と同時運転，候補の builder，`GuardedDevice`）は済んだ．この 3c-3b は再 open（BP4c）と H4．どれも試験と bench の道具で，本番の Module と profile の値は変えない（値を決めるのは段階 4）．

使える機材（bench は後日．このコードは実機なしで書いて fake でリハーサルする）：
- X300 #1：slot A に OBX，slot B は空．HG image．
- X300 #2：slot A に UBX-160，slot B に CBX．HG image．UBX＋CBX の混在は RM-28 で受け付けられる（`boards_compatible` は全部の組で true．拒否されるのは 1 つの slot に違う board を置く場合と，装置に無い board を layout が名指す場合．2026-10-20 の「one device carries one kind」は今は無い古い UR-5 の規則）．
- ほかに CBX，LFTX・LFRX・BasicTX・BasicRX が各 2 枚まで借りられる．2 channel の bench は「#2 の UBX＋CBX」か「#1 の slot B に CBX を挿す（OBX＋CBX）」か「Basic/LF を 2 枚」（owner が決める）．
- XG image への書き換えは owner の確認が要る（コードは HG・XG のどちらでも動くようにする）．

今の測定の道（読むところ）：`crates/ezsdr-radio-uhd/tests/hardware.rs` の `hw_measurements_record`，`tests/common/measurement.rs`（`provider_groups`，`measure_rx_throughput`・`measure_tx_throughput`・`measure_rx_start_lead`・`measure_tx_start_lead`・`measure_overflow_restart_gap`，`candidate_session`，`summarize_candidates`，`DeviceLease`，`GroupRun`），`tests/common/measurement/multichannel.rs`（`GuardedDevice`，`Latch`，`guard_open`，`open_record_guard`，`CountingDevice`），`tests/common/mod.rs` の `bench_profile`・selector，`tests/common/probe_device.rs`，`tests/fake.rs` のリハーサル，`plan/phase8/tools/run-record.sh`．

## 2. 作るもの（3c-3b．Sol の事前確認 `sol-pre-3c3.md` §2.5・§2.6 と `sol-pre-3c3b.md` を反映）

### 2.1 再 open（BP4c）

3c-3a で記録の流れは「装置を開いて識別 → 閉じる → raw の probe → 開き直す → 群」になった（`tests/hardware.rs` の `hw_measurements_record`）．失敗した装置は `GuardedDevice` と 1 つの `Latch`（`tests/common/measurement/multichannel.rs`）が守り，latch の後は opener も呼ばれない（`guard_open`，`open_record_guard`）．この段は，それに「全部の群の後でもう一度開き直して測る」を足すだけにする．新しい latch・guard・lease の仕組みは足さない．

- 全部の群（raw の群を含む）の後で，次のすべてが成り立つときだけ開き直す：記録の latch が立っていない（`record_latch_reason()` が `None`），lease が clean と記された（3c-2c の `DeviceLease::mark_clean`），lease の装置の `Arc` の参照が自分だけ（`Arc::strong_count`）．候補の値の試行で Provider が報告した期待どおりの失敗（`TIME_ERROR` など）は latch を立てないので clean を崩さない（3c-3a の latch の規則のまま）．
- 開き直しは今の記録と同じ道（lease を drop → `guard_open` で開く，UR-7 の再試行の helper を含む）．同じ latch の下で流す．
- 開き直した後に流すもの：受信の throughput と，受信・送信の start lead（単独の条件だけ）を少ない回数（既定は各値の半分，`MeasurementPlan` に）．条件の `reopened` を true にし，値の計算にも条件として入れる（3c-3a の容量の規則と lead の最悪の読み方のまま）．
- 開き直さなかったら observation に理由（latch，clean でない，参照が残る）．
- 別の process での実行は別の基準で，同じ process の再 open の代わりにならない（bench.md に一行）．

### 2.2 H4：負荷の道具

- `plan/phase8/tools/h4-load.sh`：stress-ng は入っていないので bash と python3 だけで．
  - `--cpu N`（既定 `nproc`）の busy loop と `--mem M`（既定 2）の buffer の copy の loop（1 つ 256 MiB まで．上限を超える指定は拒否）．優先度は既定（nice 0）．realtime の busy loop は作らない．
  - 起こした process の PID を記録し，SIGTERM・SIGINT・EXIT の trap で PID で止める（pattern で kill しない）．`--duration S` で自分で止まれる．
  - `--self-test`：1 秒だけ起こして止まり，PID が残っていないことを確かめる．
- `run-record.sh`：`EZSDR_MEASUREMENT_LOAD=h4` のとき，記録の test の前に `h4-load.sh` を起こし，test の後に PID で止める（trap で必ず止める）．負荷の parameter と PID，前後の `/proc/stat` を run の dir に書く．`bench.md` の「`run-record.sh` 自体は H4 負荷を生成しない」を直す．

### 2.3 H4：dispatch と headroom の observation（`hw_h4_dispatch`．値は作らない）

起床の遅れそのものは scheduler の trace が要る．ここでは代わりの量（proxy）を測り，observation にそう書く．
- `ProbeDevice` に，timed の `apply` の入口の行（`e`，host の時刻）と，`time_now` の値と前後の host の時刻を足す（今は記録しない．`probe_device.rs:233-276,339-351`）．
- Session で，送信の repeat（10 MS/s）を流しながら 200 個の timed の利得の変更（10 ms おき）を予約する（利得は radio だけに届き DDC/DUC に溜まらない．設計メモ 39 §1）．
- uhd-control の dispatch の遅れ：各 timed の `apply` の入口の host 時刻 − （`e − release_window` を host 時刻にしたもの）．release window は Description の値．device と host の対応は `time_now` の前後の host 時刻から作り，誤差の幅も記録する．
- uhd-tx の headroom：burst の時刻と，受け付けた channel ごとの数から，各 `tx_send` の時点で送る sample の時刻までの余裕を作る（続きの送信は `at` が無い．`provider/tx.rs:442`）．
- 分布（最小，中央，99%，最大）と数を observation に．Module の thread と UHD の thread の policy・affinity（`/proc/self/task/*/stat` と `status`），試行の間の CPU 使用率（`/proc/stat` の差）も記録する．負荷あり・なしは記録の `load` で分かれる（同じ test を 2 回流す）．
- 計算の関数は合成の行で単体テストする．

### 2.4 リハーサルとテスト

- `tests/fake.rs`：再 open（clean のとき開き直し，`reopened` の条件が値に入る．latch・clean でない・参照が残るときは開き直さず，opener も呼ばれない：3c-3a の counting opener で数える），dispatch の observation の形．壁の時計で判定しない．
- 純粋なテスト：clean の区別，clone の数の確かめ，dispatch と headroom の計算，`h4-load.sh --self-test` と `bash -n`．

### 2.5 Sol の事前確認の反映（`sol-pre-3c3b.md`．§2.1〜§2.4 より優先する）

- **所有の確かめ**：今の正常な道では `hardware.rs:2201-2204` が `guard` を持ったまま `guard.clone()` を lease に渡すので，参照は 2 以上．最後の streamer の確認の後で手元の `guard` を drop してから数える．`lease.device()` は clone を作るので使わず，lease が持つ `Arc` を直接見る読み取りの関数（clean か，参照の数）を `DeviceLease` に足す（新しい lifecycle の仕組みは足さない）．Session の Authority・Provider，`CountingDevice`，probe が持つ参照や，bounded join で detach した Authority の thread（`src/authority.rs:530-548`）が残っていれば開き直さない（待たない，強制的に閉じない）．
- **記録のまとめ方**：同じ helper を流し直して `reopened=true` を付けるだけでは，条件の id がぶつかり（`measurement.rs:2153-2159`），比べる key が 2 度になって拒否される（`measurement.rs:210-226`）．再 open の条件と observation には別の id を付け，key ごとに値は 1 つ：容量は必要な条件の集合の上で（3c-3a の規則），lead は必要な条件の最大．適用できる再 open の測定が失敗したら，それ以前の成功だけで値にしない．「半分の回数」は最低 1．
- **負荷の script**：`--mem M` は worker の数とし，確保する常駐メモリの**合計**に上限を置く（copy の buffer と一時の分を含む）．worker の数と有限で正の duration を起こす前に検査する．worker は python の PID を直接持ち，追えない子孫を作らない．起こす前に後始末を入れ，正常終了・起動の失敗・TERM・INT で全部止めて回収（reap）する．SIGKILL や host の故障では後始末できないことを bench.md に書く．`run-record.sh` は負荷の準備ができるのを待ち，worker が途中で死んだか duration が切れたかを記録する（そうでないと `load=h4` が負荷なしの試行を名乗る）．使う script は展開した snapshot のもの（`run-record.sh:121-124`）．self-test の確保は小さく．
- **dispatch の量**：200 個を一度に予約しない（profile の深さは 16，それより先の予約は拒否される：`x3xx.rs:397`，`control.rs:575-583`）．予約を回しながら（前のものが済んでから次）入れる．受理，実際の `e`，apply の結果，呼ばれなかったものも記録する（成功した呼び出しの分位数だけにしない）．入口の時刻は probe の hook や lock の前に取る．向き・channel と，利得だけの設定であることを行に入れる．**受信の利得**を変え，送信の利得は 0 のまま．送信は 3c-3a の振幅を確かめた波形（`tests/common/mod.rs:324-329`）を使い，att の確認は今のまま．
- **時刻の対応**：前後で挟んだ装置の時刻の読みで host 時刻に写す．Authority 自身は anchor の間を外挿する（`src/authority.rs:100-147`）．遅れは符号つきの区間で記録し，anchor の古さと master clock の換算も残す．host と装置の時計の速さが同じこと，drift の扱いは INFERRED と書く．挟みの幅だけを長い区間の誤差の上限にしない．anchor が無い・不連続なら捨てる．
- **送信の headroom**：途切れない burst について，時刻つきの SOB と channel **ごと**の受け付けた sample の累積から作る（合計ではない）．呼び出しの入口での次の sample の名目の時刻までの余裕とし，戻りの時点の余裕も任意で．SOB・EOB・streamer の取り替えでやり直し，error・underflow の後のあいまいな所は捨てる．RF の実行時刻の確認ではなく「名目の提出の余裕」と書く．
- **記録の suite に入れる**：`hw_h4_dispatch` を `hw_measurements_record` の probe の列に入れる（今は 15 個．preflight は負荷を起こす前に流れる：`run-record.sh:200-212`）．probe の行の大きさに上限を置き，切り詰めたら書く．
- **足す系列**：正常な raw の群の後で古い handle の destructor が済んでから opener が呼ばれる．不安全な条件・残る wrapper や thread・restore の失敗・unwind のそれぞれで opener が呼ばれない．再 open の結果が悪い・欠けるときに値が 1 つで記録が有効．queue の満杯，apply が無い，負の遅れ，時計の不連続，部分の送信・0 の送信，underflow．負荷の後始末（完了，TERM，INT，起動の途中，測定の失敗）．latch の後は後始末も再 open も拒まれる（今のまま）．

## 3. 範囲

上だけ．H3，D9 の本番の直し（D9a・D9b），profile の値，Provider の本番の道は変えない．新しい rig の種類，fake の fault の種類は足さない．試験の中の `Device` の実装は可．大きさの目安は約 1,500 行以内．越えそうなら止めて，どこで分けるかを報告する．

## 4. 確かめること

- 純粋なテストは 1 回．fake のリハーサルを変えたら 20 回（時刻とスレッドに依るため）．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`bash -n` を script に．
- `hardware.rs` は docker の clippy：`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-3c3b-target:/cargo-target -w /work -e CARGO_TARGET_DIR=/cargo-target ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`．
- mutation：判定と読み方（再 open の条件と飛ばす理由，dispatch と headroom の計算，負荷の script の止め方）に行を足し（ID は既存の最大の次から），新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（repo の root から．前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．「BASELINE FAILED」はその行だけ流し直す（remote の負荷）．
- spec：`design/18-uhd-radio.md` UR-34 の文とテストの名前（DA-1 の形），Changes に 1 行（設計メモ 41，stage 3c-3b）．`plan/phase8/bench.md` の BP2（負荷の起こし方）と BP4c（再 open）．

## 5. セルフレビュー（1 回）

```
cd /home/komatsu/local_work/Ez-SDRv4-3c3b && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3c3b.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3c3b-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 6. 報告

`report-3c3b.md`．最初の行は `STATUS: done`（途中で止めたら `STATUS: stopped` と理由）．作ったもの（ファイルごと），この指示書から変えた所と理由，テスト名，bench の時間の見積もり（負荷なし・あり，再 open の分），spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．最後のメッセージは報告の要約．
