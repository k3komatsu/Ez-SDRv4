# 段階 3e：実機の準備（指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．HEAD は `a9b0a26`（D9a まで入った）．repository は /home/komatsu/local_work/Ez-SDRv4（main の tree）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．main の tree を触るのはあなただけ．別の agent が tree の外の写しで送信の throughput を調べ，別の agent が MockRadio の試験計画を書いている（どちらも tree は書かない）．

次の実機の日に，流した結果がそのまま使えるようにする段階．どれも小さく，risky ではない（試験と bench の道具．本番の `src/` は §2.7 の 1 か所だけ）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（周りの書き方に手で合わせる）．ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- `src/` の本番のコードは §2.7 だけ（試験だけの変更は §2.10 のとおり許す．ほかに要るなら止めて報告に書く）．本番の値と既定の道は変えない．MockRadio は変えない（MR-34 をそろえるのは別の段階）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．負荷をかけない．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．tree に作ったファイルは mode 644（script は 755）．
- Sol は起こさない．実機は流さない．
- 失敗の扱いを足すときは，仕組みを足す前に道を減らす．`GuardedDevice` と 1 つの latch の形は owner が残すと決めた（`plan/maintenance/failure-mechanisms.md`）．新しい guard を足さず，それを使う．
- **起きたことを書く．していないことは書かない．**

## 1. 読むもの

- `plan/phase8/reviews/bench-dry-run-1027.md`（実機の前の机上の通し．F1〜F11）．
- `plan/phase8/reviews/flaky-tests-1027.md`（揺れるテスト 3 本と同じ形の 4 本，直し方）．
- `plan/phase8/42-d9-production-fix.md`，`plan/phase8/reviews/d9a-opus.md` §5（MA-1）．
- `~/.cache/ezsdr-fixes/2026-10-26-stage3/bench-day-runbook.md`（実機の日の手順書の下書き）．
- `plan/phase8/bench.md`，`plan/phase8/tools/run-record.sh`，`plan/phase8/tools/bench-host.sh`．
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`，`tests/common/measurement/multichannel.rs`，`tests/common/mod.rs`，`tests/hardware.rs`，`tests/fake.rs`．

## 2. 作るもの

### 2.1 揺れるテスト（`flaky-tests-1027.md` のとおり）

- `ur_25_an_enable_from_no_stream_configures_once` と `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`：遅れていないことを見る assert（fake.rs の該当の 1 行ずつ）を消す．2 本目の名前から「on time」を外し，それを引く mutation の行（P254 ほか）の filter，`design/18-uhd-radio.md` の該当の Test 行を直す．
- `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took`：時計を制御できる `src/provider/tx.rs` の unit test に移し（`ur_22_a_first_send_cut_short_keeps_the_reports_paired` の隣，設計メモ 37a と同じ形），fake の版を消す．B18，B19，B25 の行を `test_bin: lib` に．spec 18 の Test 行を unit test と書く．
- 同じ形の 4 本（`ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one`，`ur_21_an_untimed_send_is_on_time_after_its_delivery`，`ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin`，`crates/ezsdr-server/tests/protocol.rs` の loopback）：報告の読み方で同じ原因か確かめ，同じなら同じ形で直す．違えば報告に書く．
- 直した試験は注入した遅れ（テストの thread を 6 ms 止める，報告の方法）でも通ることを，tree の外の写しで確かめる（写しは `rsync -a --no-times --exclude .cargo --exclude target` で作り，終わったら消す）．

### 2.2 queue の深さの判定（F1）

- 試行を `Incomplete` にするのは，止まった呼び出しの戻りの**上限**の装置の時刻が目標より前のときだけにする（今は下限が目標以後でないと数えない．`measurement.rs` の `classify_queue_trial` あたり）．上限と下限の幅は記録に残す．
- `queue_max_calls` を 512 に．
- 実機の 12 回の読み（報告 F1：+0.17〜−2.42 ms）を合成の値として，新しい判定でどれが成功になるかを純粋なテストで固める．

### 2.3 同時運転は値に入れず，1 組を確かめる（owner 2026-10-28）

- 受信・送信の throughput の値は，単独の条件と多 channel の条件だけで決める．同時運転の段の列（`rx-duplex-*`，`tx-duplex-*`）は値の計算と段の列から外す．
- 代わりに，受信と送信の値が両方決まった後で，同時運転を 1 組だけ流す：受信をその値（の rate）で，送信をその値（の rate）で同時に，throughput の秒数だけ．結果（stream ごとの証拠，送信が流れていた割合）は observation `provider_duplex_at_the_measured_limits` に．値は作らない．どちらかの値が `unmeasured` なら流さず，理由を observation に．
- 段の数の見込み（bench の時間）を報告に．

### 2.4 容量の行の条件の名前（F2）

- `capacity_over_conditions` が値の行に付ける条件を，その T で止めた（あるいは最後に持ちこたえた）条件にする（今は `at[0]`）．

### 2.5 PLL が外れたら記録を止める（F3）

- 記録の中で UR-7 の再試行を使い切ったら，記録の latch を立てる（`install_record_latch` の隣の小さな helper）．以後の probe と群は「not run」になり，記録は早く書かれる．raw の probe の `set_sources`（`hardware.rs` の `raw()`）にも同じ再試行と latch を．
- テスト：再試行を使い切ると latch が立ち，以後の opener が呼ばれない（3c-3a の counting opener）．

### 2.6 段の失敗で群を捨てない（F4）

- `extra_throughput` で段の `Err` と panic を受け，その段の失敗（`class: None` と observation）として記録して上昇を止める．それまでの段と 1 channel の sweep は残す．
- テスト：途中の段が `Err` を返しても，それより前の段の値と observation が残る．

### 2.7 streamer を開く失敗の UHD の文（F5．`src/uhd.rs`）

- `uhd_usrp_get_rx_stream` と `uhd_usrp_get_tx_stream` の失敗の文を，装置の handle でなく streamer の handle から読む（`check_rx`・`check_tx`．`uhd.rs` の該当の 2 か所）．`--features uhd` の docker clippy で確かめる（実機は無い）．

### 2.8 MA-1 の Ceiling と位相の probe（owner 2026-10-28：(a)）

- spec 18 の UR-24 に Ceiling：流れていない chain に当てる受信の周波数は untimed なので，その tune の後の LO の位相は timed tune の保証（`random_unless_timed_tune`）を持たない．保証は受信が流れている間の timed の tune だけ．`*Test:*` は §2.8 の probe と，D9a の M6 の試験．
- `KNOWN_LIMITATIONS.md` に 1 項目（症状，いつ，回避：位相が要るなら受信が流れ始めた後に `at` つきで周波数を設定し直す，根拠，状態：MockRadio の MR-34 をそろえるのは別の段階，実機の probe 待ち）．AGENTS.md の規則どおり同じ commit で．
- probe `hw_ma1_phase_after_a_pre_start_tune`（`tests/hardware.rs`，値は作らない）：Session を N 回（既定 5）開き，それぞれ T0 の前に受信と送信の周波数を設定して（D9a の道で untimed になる），loopback の位相を測る．同じ N 回で，流れ始めた後に `at` つきで設定し直した位相も測る．2 つのばらつきを出す．送信は 0 dB，振幅 ≤ 0.5（3c-3a の `transmit_waveform`），att の確認は今の run-record の規則．
- KL-9，KL-11 など D9a の項目で変わる所は無いことを確かめる．

### 2.9 bench の道具（F7〜F11）

- `run-record.sh`：両方の docker の配列に `--tmpfs /tmp:rw,size=4g`．`-e CARGO_HOME=/cargo-target/.cargo-home`（volume の root に置く．試しに `bash -n` と，docker を使わない部分の自己確認）．
- `bench-host.sh` の確認（読むだけ）に足す：CPU の governor（`/sys/devices/system/cpu/cpu*/cpufreq/scaling_governor`），C-state（`/sys/devices/system/cpu/cpu0/cpuidle/state*/disable` と名前），NIC の IRQ がどの CPU に向くか（`/proc/irq/<n>/smp_affinity_list`，NIC の irq は `/proc/interrupts` から）．不足として落とさず，値を出力して記録に残す（`--apply` は変えない）．
- `bench.md`：F11 の文の食い違い（coercion の送信の周波数は全範囲，CBX の「1 GHz」は 1.1〜1.2 GHz，古い試験の振幅 0.566），所要時間（机上の通しの見込み：#1＋OBX 65〜80 分，2 channel 1 時間 40 分〜2 時間），F6（「did not join within 1 s」が出たら以後の群は not run）．
- 手順書：`bench-day-runbook.md` を直して `bench.md` の節にする（P4〜P7 と §2.8 の probe の実際の名前，`mkdir -p` の log の置き場，`hw_b1_probe` を preflight に，UHD の `benchmark_rate`〈`/usr/local/lib/uhd/examples/benchmark_rate`〉で送信・受信を 100・200 MS/s で 60 s ずつ流して PC と UHD の限界を Module と切り分ける手順，§2.3 の段の数での時間）．

### 2.10 Sol の事前確認の反映（`sol-pre-3e.md`．§2.1〜§2.9 より優先する）

- **§0 と §2.1 の食い違い**：`src/provider/tx.rs`（と要るなら `control.rs`）の**試験だけ**の変更は許す（本番のコードは §2.7 だけ）．時刻どおりの性質を「何かが送られた」に弱めない：消す assert の性質は，時計を制御できる試験に残す．
- **queue の判定（§2.2 を置き換え）**：上限と下限の区間が目標をまたぐ試行は「あいまい」（成功にも失敗にもしない別の分類）にし，成功に数えない．値は成功の試行だけから．あいまいの数は observation に．目標までの遅れを変えた試行（例：3 s と 2 s）で止まる呼び出しの番号が一致するかも記録する（back-pressure の推論を強める）．合成の試験に，遅いが queue で止まっていない呼び出しと，目標をまたぐ区間を入れる．昔の 12 回の読みを成功にすることを求めない．512 回は妥当（UHD の radio の control word は 512，timed の 1 語の要求は 6 語）だが保証ではない．利得の条件で止まらなければ今どおり `unmeasured`．
- **同時運転の 1 組（§2.3 を補う）**：値は bytes/s なので，1 つの rate や layout を決めない．それぞれの向きで値を持ちこたえた条件（stream と channel と rate）をそのまま使い，その 2 つを同時に流す．多 channel の容量を channel 0 の rate に縮めない．使った条件を observation に記録する．1 組の 60 s は「確認」であって，admit される全部の組や長い運転の証拠ではないと書く．落ちたら「2 つの値は同時には示せなかった」と明記する．stream ごとの失敗と，実際に重なって流れた区間を記録する．
- **容量の条件の名前（§2.4 を補う）**：値に付ける条件は，その値で**成功した**条件．止めた条件は blocker として別に記録する（高い T の失敗した条件を低い値に付けない）．
- **PLL（§2.5 を補う）**：raw の `set_sources` は UHD の文をそのまま返し，`DeviceAuthority::new` は `REFERENCE_DID_NOT_LOCK` に直す（`src/authority.rs:363-369`）．試験の helper で同じ形に直してから今の再試行の helper に渡す．試験：再試行で成功すれば latch しない，使い切ると latch，関係ない失敗は再試行しない，latch 後に opener が呼ばれない，失敗した群と以後の not run が記録に残る．
- **streamer の文（§2.7 を補う）**：UHD は 2 つの関数とも `UHD_SAFE_C_SAVE_ERROR(h_s, …)` で streamer の handle に保存する（確認済み）．`--features uhd` の clippy だけでは確かめられないので，文の取り出し元を確かめる試験を stub で 1 つ（できる範囲で．できなければ理由を報告に）．
- **位相の probe（§2.8 を置き換え）**：送受の loopback は 2 つの LO の**相対**の位相しか測れない．比べる 2 つ：(i) T0 より前に受信と送信の周波数を設定（D9a で受信は untimed．送信は今の道），(ii) 受信が流れている間に，受信と送信の両方を**同じ未来の instant** で設定し直す（受信の更新は D9a の覆う規則で timed になる．送信は burst の間に置く：D9a の送信の DSP の Ceiling F3′ を避ける）．LO が実際に書き換わる周波数を選び，落ち着くのを待ち，時刻の揃った sample の位置で既知の波形に対して位相を推定する．欠け・弱い音・不安定な位相は INCONCLUSIVE（D9a の `tests/common/tone.rs` の考え方を使ってよい）．N 回（既定 5）は観測であって保証の証明ではないと書く．
- **時間**：同時運転の段の列を外した後で bench の時間を計算し直す．

## 3. 範囲

上だけ．MockRadio（MR-34），R4・R2・F7（D9b），`JOIN_BOUND`（F6）は変えない．送信の throughput の Module 側の直しは別の調査の結果を待つ．

## 4. 確かめること

- 純粋なテストは 1 回．時刻・スレッドに依るテスト（直した揺れるテスト，fake の rehearsal）は 20 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，docker の clippy（`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`），`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（名前を変えたテストは `--renames` のファイルを作業 dir に書いて渡す），`git diff --check`，`bash -n` を script に．
- mutation：新しい判定（queue の上限，容量の条件の名前，PLL の latch，段の失敗の扱い，同時運転の 1 組の前提）に行を足し，新しい行と，変えたコードや killing test に触れる既存の行（P254，P368，B18，B19，B25 を含む）を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ID> ...` で流す（repo の root から．変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
- spec：spec 18 の UR-24（MA-1），UR-34（同時運転の 1 組，queue の判定），揺れるテストの Test 行，Changes に 1 行（段階 3e）．

## 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3e.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3e-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．free が失敗したら同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 6. 報告

最後のメッセージを報告にする（Write が拒まれたら orchestrator が保存する）．最初の行は `STATUS: done`（止めたら `STATUS: stopped` と理由）．作ったもの，指示書から変えた所と理由，テスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．
