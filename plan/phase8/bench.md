# Phase 8 — 実機 bench 手順

Phase 8 の BP0〜BP5 を実行する手順．shake-out の観測は診断用で profile の値には使わない．完了した BP2/BP3 の測定記録は段階4の profile 判断の入力になり，`measurements/` に置いた記録を比較テストが読む（仕様 [41 §2, §4](41-measurement-records.md)）．実機で失敗しても bench 中にコードを直さず，log と `.partial.json` を保存して次の修正段階へ戻る．

## 機材と RF 安全

- 対象は X300 #1（serial `347D545`，slot A に OBX，`addr=192.168.40.36`）と #2（serial `347D541`，slot A に UBX 等，`addr=192.168.47.37`）．NIC はそれぞれ `enp1s0f1np1`・`enp1s0f0np0`．実際の配線と serial を probe で確認する．同時接続は同じ `/24` の経路が重なるため，必要な `/32` route を owner が用意していない限り一台ずつ使う．
- 送信する全ての段で，該当 board の **TX/RX → 30 dB 以上・18 GHz 定格の attenuator → 同じ board の RX2** を接続する．att は最大 TX 出力に対して十分な最大入力電力定格と余裕があることを銘板・仕様で確認し，型番と電力定格を記録する．アンテナはつながない．この日の装置では 40 dB attenuator を使用した．gain 0 dB，波形振幅 0.5 以下，通常の loopback は 1 GHz を使う．
- coercion の TX gain sweep は **streamer を開かず，LO を 1 GHz にした状態で最大 gain まで**試す．ただし CBX の下限は 1.2 GHz なので，CBX では「1 GHz」は 1.1〜1.2 GHz に clip される（UHD は error を出さない．#1 の未知の board の cleanup が 0.0 Hz を読んだ例が shake-out 4 にある）．coercion の TX frequency read-back は **全周波数範囲の格子**（OBX 10 MHz–8.4 GHz，UBX 10 MHz–6 GHz，CBX 1.2–6 GHz．gain 0，sample を送らない）を読む（`SAFE_TX_HZ` の範囲ではない）．古い試験（`pn`）が送る波形の振幅は 0.566 までで，0.5 以下なのは 3c-3a 以降の送信（`transmit_waveform`）．いずれの送信条件も上記 attenuator が必要で，RF safety の例外ではない（仕様 41 §8）．
- `run-record.sh` は各実行で接続を質問し，`yes` 以外なら Docker や装置を使わず停止する．確認を省略する option はない．

## BP1 → BP0：ホスト設定，その後に装置確認

1. 装置に接続した **一台の NIC だけ**を指定し，装置の発見・probe より先に read-only check を行う．再起動後など不足値がある場合に限り owner が `--apply` を実行し，check をもう一度通す．例：

   ```sh
   plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 40
   sudo plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 40 --apply
   ```

   確認だけの形が不足値を出した場合に限り，owner が `--apply` を実行する．`--apply` は root を要求し，明示した NIC の MTU/address/ring/NetworkManager state，その PCIe link の ASPM control，および host 全体の sysctl 3 値を変更・再確認する．別の NIC の address は自動で外さない．既定の check は設定を変更せず，不足があれば exit 1．設定は再起動で戻る．

2. **BP0**：host 設定の check が通った後に装置を発見し，serial，board，10 GbE link，配線を確認する．HG/XG image の変更は owner の許可なしにしない．UHD 4.10 image で probe し，`Mboard`，FPGA version/git hash，daughterboard，master clock 200 MHz，`ref_locked` を記録する．host UHD 4.9 は FPGA 4.10 image と互換でないため，host 側の UHD command は使わない．

   ```sh
   ARGS=addr=192.168.40.36  # #2 は addr=192.168.40.37
   docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_find_devices --args "$ARGS"
   docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args "$ARGS"
   ```

   host 設定後の UHD log で `Maximum frame size: 8000 bytes` 前後を確認する．X300 の背面 port 0 は HG image では 1 GbE，port 1 は 10 GbE；この測定では 10 GbE link を使う．

## 実行順（BP5 は BP2 より先）

1. **BP1**：上記の host 設定を確認する．以降の各 run も，選んだ NIC の read-only check が通らなければ進めない．
2. **BP0**：2台の probe，UBX の認識と短い loopback，FPGA image の版・種類を確認する．UHD image と互換性が合わない場合は止める．
3. **BP5**：新しい profile / Module で Phase 7 の B3〜B7 相当が動くことを先に確認する．Rust の hardware test 名は `hw_b3_receive_at_t0`，`hw_b4_capture_at_a_sample_index`，`hw_b5_overflow`，`hw_b6_txrx_and_repeat`，`hw_b7_session_loopback`（`hardware.rs`）．下の `run-record.sh` に追加 test 名を渡すと，その test を先に実行し，成功後に記録 suite を流す．B7 の Python example は別途同じ snapshot / UHD 4.10 image で実行し，log を同じ run directory に残す．
4. **BP2**：#1＋OBX で記録 suite を負荷なしで流し，次に `EZSDR_MEASUREMENT_LOAD=h4` を付けて同じ suite を流す．後者は `run-record.sh` が build・preflight の後，記録 suite の前に `h4-load.sh` を起動し，成功・失敗・TERM/INT のいずれでも PID を指定して止め，reap する．既定は `nproc` 個の CPU busy worker と2個の memory-copy worker（各 worker は32 MiB の source と target の2本を持ち，予算は一時分を含め3本/worker；推定常駐量の合計上限は1 GiB）．必要なら `EZSDR_MEASUREMENT_H4_CPU`，`EZSDR_MEASUREMENT_H4_MEM`，`EZSDR_MEASUREMENT_H4_DURATION` で指定する；duration は記録 suite 全体より長くする（先に切れると run は失敗）．`load=h4` は `hw_h4_dispatch` だけでなく記録 suite 全体が負荷下だったことを示す．
5. **BP3**：#2 に OBX 以外の board を載せ，motherboard 値を同じ手順で測る．#1 から切り替えるとき，旧 NIC `enp1s0f0np0` に host address `192.168.40.10/24` が残っていれば，その address だけを明示的に外してから #2 の NIC を確認・設定する（script は別 NIC を変更しない）．

   ```sh
   ip -4 -o addr show dev enp1s0f0np0
   # 上の出力に 192.168.40.10/24 が残っている場合だけ実行
   sudo ip addr del 192.168.40.10/24 dev enp1s0f0np0
   plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40
   sudo plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40 --apply
   ```

   2台の差が大きい場合だけ同じ board で個体差と board 差を切り分ける．
6. **BP4**：UBX が使える場合は UBX 固有値を測る．
7. **BP4b**：混在 board，channel ごとに異なる設定，rate の違う stream，aligned stream，同時運転を試す．単独 stream の測定で同時運転の性能を主張しない．段階 3c-3a の道具：`EZSDR_MEASUREMENT_SELECTOR`（layout の JSON．例 `'{"rx":["0:A:obx:0","0:B:cbx:0"],"tx":["0:A:obx:0","0:B:cbx:0"]}'`．key は `rx`，`tx`，`image`，`links`，`link_gbps` だけで，`rx` と `tx` は両方か，どちらも無し（片方だけだと残りを装置の layout から引き継ぎ，att の確認が送信する channel を覆わないので，装置を開く前に拒否する），後の 3 つは 1 要素の list．layout が装置に合うかは Provider が UR-5・RM-28 で判定し，合わなければ最初の Run でその群の失敗になる）を渡し．throughput の値は，単独の条件と多 channel の条件だけで決める（同時運転の段は値にも段の列にも入れない）．両方の値が決まった後で，受信の値を持ちこたえた条件と送信の値を持ちこたえた条件を，それぞれの stream と channel と rate のまま同時に 1 組だけ `EZSDR_MEASURE_THROUGHPUT_SECS` の間流す（observation `provider_duplex_at_the_measured_limits`．値は作らない．これは 2 つの値の 1 回の確認で，admit される全部の組や長い運転の証拠ではない．落ちたら「2 つの値は同時には示せなかった」と書かれる．どちらかの値が `unmeasured` なら流さず，理由が書かれる）．lead と overflow の gap には，相手の向きを 10 MS/s で流した条件を，それぞれ別の群（値の key なし）として足す（observation だけで，値の行は決めない．失敗してもその群だけの失敗）．2 channel 以上の向きは 1 本の stream に全 channel，channel ごとに 1 本，rate を変えた channel ごとの 1 本（channel 0 は格子の高い方，channel 1 はその次に低い格子）の条件を流す．合計の bytes/s は，適用できる全部の条件で，その値以下のすべての段が成功した最大．record の raw probe は slot A（device channel 0，1 GHz）を動かすので，selector の rx・tx の channel 0 が slot A の UBX か OBX（または未指定）のときだけ走り，それ以外では各 probe が「この layout に当てはまらない」という observation になり，装置には触れない．`run-record.sh` の att の確認は，selector の `tx` の全 channel と，raw probe が走るとき，または preflight の `hw_*` を指定したときの slot A の board（preflight は selector を受けず slot A を動かす）を名指しし，1 回の `yes` で各 board の TX/RX → ≥30 dB att → 同じ board の RX2 を確かめる（Basic/LF の TX は繋いだ先の RX）．所要時間の目安：1 段 = `EZSDR_MEASURE_THROUGHPUT_SECS`（既定 60 s）と Run の準備（約 3.5 s）で約 63〜64 s．1 channel の向きは sweep の 7 段（約 7.5 分）．2 channel の向きは sweep 7 段＋(a) 7 段＋(b) 7 段＋(c) 6 段 = 27 段（2 channel × 200 MS/s は 1.6 GB/s で admission が拒むので，約 25 段が走る．約 26 分）で，RX と TX で 2 倍．同時運転の 1 組は 1 段（約 1 分）．
8. **BP4c**：XG image と 10 GbE 2本で throughput，再 open，同時 RX/TX を測る．FPGA image を書き換える前に owner の確認を得る．`EZSDR_UHD_ARGS` に `addr=…,second_addr=…`，`EZSDR_MEASUREMENT_SELECTOR` に `"image":["xg"],"links":[2],"link_gbps":[10]`，`EZSDR_MEASUREMENT_NICS` に使う NIC を空白区切りで渡す（各試行の前後で `/sys/class/net/<nic>/statistics/{rx,tx}_bytes` の差を observation に残す．stream の番号は物理の link ではない）．候補の天井は link 1 本分（10 GbE で 1 250 MB/s）で，2 本の合計より上は試さない（8c の範囲）．3c-3b の reopen は raw group の後，同一 process で latch が clear，lease が clean，lease 以外の `Arc` owner が無い場合だけ行う；残る owner を待ったり閉じたりしない．再測した RX throughput と RX/TX start lead は，それぞれ最小 throughput・最大 lead で base 値と合わせる．reopen を skip/失敗した値は `unmeasured` で，理由を記録する．別 process での再起動はこの reopen の代わりにならない．H4 は `hw_h4_dispatch` の observation-only probe で，timed RX gain update と TX repeat の同時測定から dispatch-delay proxy と nominal TX submission-headroom proxy を記録する（scheduler wake-up latency / RF execution time そのものではない）．

## 記録の流し方

`run-record.sh` は commit の snapshot を `$HOME/ezsdr-bench/<日付>/<commit>-<時刻>/src` に展開し，`tar -xm` で build より新しい mtime を付ける．同じ専用 Docker volume `ezsdr-v4-bench-target` を使う `run-record.sh` の実行は host 側の non-blocking `flock` で直列化する（lock file は `${XDG_CACHE_HOME:-$HOME/.cache}/ezsdr-bench/`）．tree 外の旧 `~/ezsdr-bench/run-bench.sh` もこの volume を使うため，それとは同時に実行しない（旧 script はこの lock を取らない）．main tree の `ezsdr-v4-cargo-target` とは共有しない．log と measurement は同じ run directory に置き，script は repo 内や `crates/ezsdr-radio/measurements/` には書かない．完了した BP2/BP3 の最終 `measurement.json` は確認後に `41-measurement-records.md` §2 の `crates/ezsdr-radio/measurements/<日付>-<serial>-<構成>.json` へ手動で写す（比較テストが読む記録）．shake-out の記録と `.partial.json` は写さない．

Cargo の cache freshness は，Docker・実機なしで小さな binary crate により確認できる：同じ manifest と作業 directory で出力が `A` / `B` の二つの snapshot を用意し，同じ `CARGO_TARGET_DIR` で順に build・実行する．`tar -xf` 相当で B を古い mtime のまま展開したとき A が再利用され，`tar -xm` 相当で展開したとき B が出ることを確かめる例：

```sh
mkdir -p "$HOME/ezsdr-bench"
D=$(mktemp -d "$HOME/ezsdr-bench/cache-freshness.XXXXXX")
mkdir -p "$D/A/src" "$D/B/src" "$D/work"
for version in A B; do
  printf '[package]\nname = "cache-freshness"\nversion = "0.1.0"\nedition = "2021"\n' >"$D/$version/Cargo.toml"
  printf 'fn main() { println!("%s"); }\n' "$version" >"$D/$version/src/main.rs"
  tar --mtime='@1600000000' -cf "$D/$version.tar" -C "$D/$version" .
done
tar -xf "$D/A.tar" -C "$D/work"
cargo build --quiet --manifest-path "$D/work/Cargo.toml" --target-dir "$D/target"
test "$("$D/target/debug/cache-freshness")" = A
tar -xf "$D/B.tar" -C "$D/work"
cargo build --quiet --manifest-path "$D/work/Cargo.toml" --target-dir "$D/target"
test "$("$D/target/debug/cache-freshness")" = A
tar -xm -f "$D/B.tar" -C "$D/work"
cargo build --quiet --manifest-path "$D/work/Cargo.toml" --target-dir "$D/target"
test "$("$D/target/debug/cache-freshness")" = B
```

これは Cargo の mtime 問題を切り分ける check であり，UHD build / hardware test の代用ではない．

```sh
EZSDR_BENCH_NIC=enp1s0f0np0 EZSDR_BENCH_SUBNET=40 \
  plan/phase8/tools/run-record.sh <commit> addr=192.168.40.36 hw_b1_probe
```

必ず `hw_measurements_record` を実行する．任意の `hw_*` test はその前の preflight として実行され，失敗したら record suite は開始しない．8つの必須 metadata は，output（自動で `/bench/measurement.json`），mboard と FPGA image（probe から。probe できない値は `EZSDR_MEASUREMENT_MBOARD` / `EZSDR_MEASUREMENT_FPGA_IMAGE` で指定），host name（既定は `hostname -s`），settings script / version（bench-host の path と出力），load（既定 `none`，`none` または `h4`），commit（snapshot の完全 hash）で組み立てる．`EZSDR_UHD_ARGS` には指定した UHD args を渡す．試行回数等は `EZSDR_MEASURE_STEP_TRIALS`，`EZSDR_MEASURE_OVERFLOW_TRIALS`，`EZSDR_MEASURE_THROUGHPUT_SECS`，`EZSDR_MEASURE_LOOPBACK_TRIALS` で調整できる（既定値は measurement code を参照）．queue depth の測定は段階 S2a-i-α で削除した．

出力には `build.log`，`probe.log`，preflight test ごとの log，`hw_measurements_record.log`，および `measurement.json` がある．3c-2c の途中経過 `<OUTPUT>.partial.json` は診断専用で，比較テストは読まない．各 group 後に上書きされ，final record が書けた後に削除される．中断時に残った partial は削除・改名せず log と一緒に保管する（再開可能な record ではない）．3c-3b の reopen は既定計画で追加約9〜10分（RX throughput 7段を各約30秒，RX/TX lead は step trials を半分にして再測；INFERRED）．この追加を含む目安は，#1＋OBX（1 channel）で約60〜75分，#2 の2 channel（UBX＋CBX）で約1時間35分〜1時間55分（INFERRED）．`h4` run は記録 suite 全体を負荷下で行うため，wall time は負荷なしの目安を越えることがある．H4 自体は200 updatesを10 ms間隔で流す約2秒の窓に加え，Run の準備・停止がある（INFERRED）．内訳（基準 suite，分）：raw probe 8，command lead 0.7，in-flight window 0.8，receive-start lead 8.5，transmit-start lead 1.6，受信 throughput 7.5（2 channel では 26），overflow 6，送信 throughput 7.5（同 26），full duplex 1.3，companion 11，raw group（coercion のみ；時間は未計測），同時運転の 1 組 1．SIGKILL や host failure では trap が動かず worker が残り得るため，その場合は `h4-load.log` と記録された PID を確認してから次の run に進む．UR-7 の再試行は 1 回で約 31 秒，PLL が戻らなければ record の latch が止める（下）．log はファイルに書いてから必要箇所を確認する．

## 既知の失敗と結果の扱い

- **DDC command buffer の詰まり**：受信 sample が流れていない chain に timed frequency command を積むと，DDC は sample packet の time が command の時刻に達するまで timed FIFO から取り出さない．UHD log に `Control operation timed out waiting for space in command buffer` が繰り返し出るのが症状．この状態で再試行，frequency reset，coercion を続けない．partial と log を保存して止め，装置が反応しなければ X300 の電源を切って入れ直し，probe と `ref_locked` をやり直す（設計メモ [39 §1](39-dsp-timed-commands.md)）．再起動が必要になる場合がある．
- **UR-7 PLL lock**：`Reference Clock PLL failed to lock to internal source` が出たら，その open は測定として数えない．決められた retry を消費した後は log を残して停止し，owner が電源を入れ直してから `uhd_usrp_probe` と `ref_locked` を再確認する．2026-10-27 に電源 cycle 後 `ref_locked=true` を観測したが，それは将来の成功を保証しない．
- **`did not join within 1 s`**：log にこれが出たら，その Provider の thread は置き去りで streamer は leak され（`JOIN_BOUND` 1 s と `SEND_TIMEOUT` 1 s が同じ長さ．直すのは別の段階），record の latch が立つ．以後の群と raw group は `not run` になり，record は早く書かれる．log と partial を保存し，装置が反応しなければ電源 cycle を考える．
- **`UR-7 retry 3`**：PLL が 3 回の再試行でも locked にならないとき，record の latch が立つ（以後の probe と群は `not run`，record は早く書かれる）．owner が電源 cycle し，`uhd_usrp_probe` と `ref_locked` を確かめてから record を流し直す．raw probe の `set_sources` にも同じ再試行と latch がある．
- **`receive Run has no T0`**：throughput の Run が admission/prepare で拒否され T0 が無い場合は，その rate を成功値として扱わない．run の termination と理由を残す．shake-out 1 では profile に 10 GbE link の宣言がなく，受信 throughput が拒否された；`8f4f060` で link を宣言した．
- **失敗は記録**：2026-10-27 の手動 shake-out log は実際に `~/.cache/ezsdr-bench/2026-10-27/` にある．shake-out log / partial は比較 test が読む `measurements/` に入れない．`run-record.sh` を使う記録は上記の `$HOME/ezsdr-bench/<日付>/` に出力する．段階4は複数台・負荷あり・必要回数の記録から値を決め，測れない値は未測定 / INFERRED のままにする．

## 実機の日の手順書（段階 3e 以後）

`<COMMIT>` は D9a と 3c-3a と 3e が入った commit（orchestrator が決める）．書いていないことは「未確認」．出典：この文書の上，`tools/run-record.sh`，`tools/bench-host.sh`，[bench-results.md](bench-results.md)（2026-10-20，10-27），[42 §4](42-d9-production-fix.md)，[reviews/d9a-test-plan.md](reviews/d9a-test-plan.md) §7，[reviews/bench-dry-run-1027.md](reviews/bench-dry-run-1027.md)．

### 前提と持ち物

| | X300 #1 | X300 #2 |
|---|---|---|
| serial / addr | 347D545 / `addr=192.168.40.36` | 347D541 / `addr=192.168.47.37` |
| NIC | `enp1s0f1np1` | `enp1s0f0np0` |
| 盤 | slot A OBX，slot B 空 | slot A UBX-160 v2（32BC76F），slot B CBX（329AACB） |
| FPGA | UHD 4.10 HG（39.3，git d375d68） | 同じ（2026-10-20 に書き換え済み） |

- 2 台は同時につながない（owner）．一台ずつ．#1 から #2 へ替えるとき，旧 NIC に `192.168.40.10/24` が残っていたら `sudo ip addr del 192.168.40.10/24 dev enp1s0f0np0` で外してから #2 の NIC を check/apply（BP3）．
- attenuator：30 dB 以上・18 GHz 定格．10-27・10-28 は #1 で 40 dB，#2 で 30 dB を 2 つ（同軸）．定格の電力は記録しない（OBX・UBX・CBX の最大出力は +20 dBm 前後（INFERRED：data sheet の記憶）で，一般的な固定 attenuator の定格（1〜2 W）より小さい．owner，2026-10-28）．アンテナは付けない．gain 0，振幅 0.5 以下，loopback は 1 GHz．TX する board ごとに「TX/RX → att → 同じ board の RX2」を 1 組（#2 で 2 channel なら UBX 用と CBX 用の 2 組．10-27 時点で CBX は未配線）．
- 電源 cycle の規則：`Control operation timed out waiting for space in command buffer`（DDC 詰まり）か UR-7 `Reference Clock PLL failed to lock` が出たら，再試行・reset・coercion を続けず，log と `.partial.json` を保存して止め，X300 の電源を切って入れ直す．戻ったら `uhd_usrp_probe` と `ref_locked=true` を確かめ直す．HG/XG image の書き換えは owner の許可なしにしない．
- host UHD は 4.9 で FPGA 4.10 と合わない．UHD のコマンドは image `ezsdr-v4-dev:uhd4.10` の中で使う．
- 出力：`run-record.sh` は `~/ezsdr-bench/<日付>/<commit12>-<時刻>/` に書く（host の確認の出力 `bench-host.txt` も：CPU governor，C-state，NIC の割り込みの向き先を含む）．手で流すものの log は `~/.cache/ezsdr-bench/<日付>/` に置く．**最初に `mkdir -p ~/.cache/ezsdr-bench/$(date +%F)`**（`tee` と redirect は dir を作らない）．同じ volume を使う旧 `~/ezsdr-bench/run-bench.sh` と同時に流さない．

### (a) ホスト確認 BP1 と装置確認 BP0（送信なし，約 10 分，estimate）

```sh
cd /home/komatsu/local_work/Ez-SDRv4
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 40            # read-only．不足があれば exit 1
sudo plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 40 --apply  # 不足時のみ owner が実行．再起動で戻る
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 40            # もう一度通す
ARGS=addr=192.168.40.36
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_find_devices --args "$ARGS"
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args "$ARGS"
```
確認：serial，Mboard，FPGA 39.3 / d375d68，slot A の board，master clock 200 MHz，`ref_locked`，`Maximum frame size: 8000 bytes` 前後，10 GbE link．stop：NIC 不足が `--apply` 後も残る，find されない，ref_locked が false → 止めて owner へ．

### (b) #1＋OBX の記録 suite（BP2．TX する board = #1 slot A OBX．約 50〜65 分，estimate）

wiring：#1 OBX の TX/RX → att → OBX の RX2．先に owner が接続を確認する（`run-record.sh` が聞くので `yes`）．`ref_locked` は `probe.log` に値が出ないので，preflight に `hw_b1_probe`（"B1 ref_locked" を出す．送信しない）を付ける：

```sh
cd /home/komatsu/local_work/Ez-SDRv4
EZSDR_BENCH_NIC=enp1s0f0np0 EZSDR_BENCH_SUBNET=40 \
  plan/phase8/tools/run-record.sh <COMMIT> addr=192.168.40.36 hw_b1_probe \
  2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/b-record.console.log
```
- 内容：3c-1（start lead，overflow gap，tx throughput），3c-2a（coercion），3c-2c（周波数の格子），3c-2b，既定の 1 channel の throughput，同時運転の 1 組，companion．
- 試行回数の既定（measurement.rs）：STEP 100，OVERFLOW 5，THROUGHPUT_SECS 60，LOOPBACK 20．変えるなら `EZSDR_MEASURE_STEP_TRIALS` 等を前置．
- BP5 を先に見たいとき（任意）：末尾に `hw_b3_receive_at_t0 hw_b4_capture_at_a_sample_index hw_b5_overflow hw_b6_txrx_and_repeat hw_b7_session_loopback` を足す（preflight．1 本でも失敗なら record は始まらない）．
- 確認（run directory）：`build.log`，`probe.log`，`bench-host.txt`，`hw_b1_probe.log`，`hw_measurements_record.log`，`measurement.json`（profile 比較の前に書かれるので，比較で落ちても残る）．`measurement.json.partial.json` が残っていたら final は書けていない．最後の profile 比較で落ちることがある（値が profile の 1000 MB/s に届かない）．これは段階 4 の材料で，bench 中に直さない．
- 見る点：coercion の格子，TX throughput の Underflow 位置，`provider_duplex_at_the_measured_limits`（`verdict`，`overlap`），overflow の「読み直しから再開まで」の最大と最小．
- stop：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock`（`UR-7 retry 3` が出たら latch が立つ），`did not join within 1 s`，`receive Run has no T0`（その rate は成功にしない）．電源 cycle → probe → record を保存．

### (c) D9 probe P4〜P7 と MA-1 の位相の probe（#1＋OBX，TX あり．各 5〜10 分，estimate）

test 名（`hardware.rs`）：`hw_d9_p4_ddc_entry_after_an_early_stop`（A 対照 / B AUTO/AUTO / C close-reopen．最初の tone は 30 ms で，stop は device の時刻 e − 5 ms に出す．B は `dsp1 != dsp0` を作れないと INCONCLUSIVE），`hw_d9_p5_a_stale_timed_header_after_a_reopen`（case A 時刻つき終了（priming の burst は 400 ms 先．stale = burst の start＋長さ）/ case B 時刻なし終了 / case C 時刻つき終了のあと reopen 後に時刻なしの空 EOB．同一 process で再現しなければ別 process：`EZSDR_D9_P5_CASE=timed`，`untimed`，`reopen` で 1 case ずつ），`hw_d9_p6_every_chunk_timed`（各 case の前に時刻なしの空 EOB で F7 を消す．2 ms chunk 全部に時刻，e=start+20 ms と e の 3 ms 前，途中 starvation．starvation の判定は burst 自身の信号の大きさの半分を閾値にする），`hw_d9_p7_the_current_transmit_shape`（任意），`hw_ma1_phase_after_a_pre_start_tune`（MA-1，UR-24 の Ceiling．Session（T0 より前に設定）と Spec Run（`at` つきで設定し直す．Session の `set` には時刻が無い）を N 回（`EZSDR_MA1_SESSIONS`，既定 5）ずつ流し，T0 より前に受信と送信の周波数を設定した場合と，受信が流れている間に同じ instant で設定し直した場合の，loopback の位相のばらつきを出す．値は作らない．loopback は 2 つの LO の相対の位相しか測れず，N 回は観測であって保証の証明ではない．欠け・弱い tone・不安定な位相は INCONCLUSIVE と理由が出る）．

wiring は (b) と同じ．**送信するので owner が att を確認してから**．`run-record.sh` は record suite を必ず流すので，probe 単体は次の形で流す（main tree の target volume．`ezsdr-v4-bench-target` と混ぜず，同時に流さない．作業ツリーが他の agent に編集されていないことを確認）：

```sh
cd /home/komatsu/local_work/Ez-SDRv4   # <COMMIT> の tree
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
for t in hw_d9_p4_ddc_entry_after_an_early_stop hw_d9_p5_a_stale_timed_header_after_a_reopen \
         hw_d9_p6_every_chunk_timed hw_d9_p7_the_current_transmit_shape hw_ma1_phase_after_a_pre_start_tune; do
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
    cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware $t -- --exact --ignored --nocapture \
    > ~/.cache/ezsdr-bench/$(date +%F)/$t.log 2>&1
done
```
見る点：音の判定は雑音の**電力**（`noise power`，閾値は床の 4 倍，coherence > 0.5 も要る）．`moves_at` が burst の最初の窓（約 −19 000 µs）なら「e より前に動いた」（F7）で欠けではない．P4 は各呼び出しの前後の device 時刻，`dsp0`/`dsp1`，最後の RX sample の tick，最初の stream の e−7〜e−5 ms の median，再開後の最初の sample の周波数（R2）．P5 は tone が動き始める µs（e 基準）を case A・B・C で（A で前に動けば F7 VERIFIED，B・C で e なら決め 5 の直しが効く．A の `header ≥ e` が false なら INCONCLUSIVE）．P6 は tone の移る位置，Underflow と継続 packet の drop（Underflow が出なければ回復は「未検証」，R4）．P7 は F3′．MA-1 は 2 通りの `phases_deg` と `spread_deg`，INCONCLUSIVE の数．stop：control timeout（P4 は sample の流れない chain に timed を積む）→ 電源 cycle．

### (d) UHD 付属の benchmark_rate（Module と切り分ける．送信の 200 MS/s を 3 通り，約 10 分，estimate）

PC と UHD の限界が Module のせいかを切り分ける（送信の throughput の調査：`~/.cache/ezsdr-fixes/2026-10-26-stage3/tx-throughput-report.md`）．`benchmark_rate` は gain と周波数を指定する option を持たず，装置の状態のまま送る（**未確認**：その gain と周波数）．**送信するので，他の送信の手順と同じに owner が att（TX/RX → ≥30 dB → RX2）を確認してから**．同じ container の形（`--network=host --cap-add=SYS_NICE --ulimit rtprio=99`）で，送信 200 MS/s を 60 s，3 通り：(1) 普通の優先度，(2) `--priority high`，(3) `--tx_spb 1000`．どれも出力の送信の packet の大きさ（"Setting TX samples per packet (spp)" の行）を記録する．100 MS/s と受信（`--rx_rate`）は任意の対照．

```sh
run() { # $1 = log name, rest = benchmark_rate options
  name=$1; shift
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" ezsdr-v4-dev:uhd4.10 \
    /usr/local/lib/uhd/examples/benchmark_rate --args addr=192.168.40.36 --channels 0 --duration 60 "$@" \
    > ~/.cache/ezsdr-bench/$(date +%F)/benchmark_rate-$name.log 2>&1
}
run tx200-normal --tx_rate 200e6
run tx200-high   --tx_rate 200e6 --priority high
run tx200-spb1000 --tx_rate 200e6 --tx_spb 1000
grep -H "samples per packet\|Num underruns\|Num transmitted\|Num sequence" ~/.cache/ezsdr-bench/$(date +%F)/benchmark_rate-tx200-*.log
```
見る点：各 log の `Benchmark rate summary`（送信した sample 数，underrun / sequence error / timeout の数）と spp．UHD 自身が underrun を出すなら PC か UHD の限界で，Module の直しの対象ではない（KL-5）；3 通りのうち高い優先度か小さい spb だけで出なくなるなら，Module の優先度か packet の大きさの問題．

X300 の送信 buffer の容量（262144 bytes の見込み）は，UHD 4.10 では TRACE の log にしか出ない（`chdr_tx_data_xport.cpp:231-232` の "Received strs initializing buffer capacity to N bytes"）．TRACE は `UHD_LOG_MIN_LEVEL` が 0 の build にしか入らない（`log.hpp:200-206` の `#if UHD_LOG_MIN_LEVEL < 1`）が，image の libuhd は `UHD_LOG_MIN_LEVEL=1`（2026-10-28 に確認）なので，`UHD_LOG_CONSOLE_LEVEL=trace` でも出ない．この日は log から読まず，UHD の source の既定値 262144 bytes を「実機では未確認」として記録する（読むには `UHD_LOG_MIN_LEVEL=0` で UHD を build した image が要る）．

200 MS/s の送信の run の burst の `blocks`：`measurement.json` の observation `provider_tx_throughput_200000000`（1 channel）の `burst_blocks`（その run の各 burst が device へ何回 send したか．1000 sample の send なら 200 MS/s・60 s で 12 000 000 回前後）を読んで記録する（`jq '.observations[] | select(.name=="provider_tx_throughput_200000000") | .result.burst_blocks'`）．2 channel の stage の observation `provider_tx_throughput_<stage id>` では `streams[].burst_blocks`．

host：C-state・governor・NIC の割り込みの向き先・coalescing（`ethtool -c`）は `bench-host.sh` が読むだけで出力し，`run-record.sh` が `bench-host.txt` に残す（設定は変えない）．

### (e) 3c-3a 2 channel（TX する board = UBX と CBX．約 1 時間 25 分〜1 時間 45 分，estimate）

構成：**#2（UBX slot A＋CBX slot B）**：addr=192.168.40.37，NIC `enp1s0f1np1`（bench-host の check/apply を #2 の NIC で，旧 NIC の address を外す）．配線は UBX 用と CBX 用の 2 組．CBX は 1.2 GHz 以上のみで既定 2.45 GHz．**未確認**：selector 内の CBX の中心周波数の決まり方（selector は rx, tx, image, links, link_gbps だけ）．#1 の slot B に CBX を移す場合は電源を落として盤を入れ替え，selector の `ubx` を `obx` に．

**先に owner が「slot A の盤（raw probe が動く）と slot B の CBX，それぞれの TX/RX → ≥30 dB att → 同じ board の RX2」を確認する**．`run-record.sh` は selector の tx の全 channel と slot A を名指しして 1 回 `yes` を求める．

```sh
cd /home/komatsu/local_work/Ez-SDRv4
EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 \
EZSDR_MEASUREMENT_SELECTOR='{"rx":["0:A:ubx:0","0:B:cbx:0"],"tx":["0:A:ubx:0","0:B:cbx:0"]}' \
  plan/phase8/tools/run-record.sh <COMMIT> addr=192.168.40.37 hw_b1_probe \
  2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/e-record.console.log
```
- `rx` と `tx` は両方か無しのどちらか（片方だけだと exit 2）．raw probe（slot A，1 GHz）は rx と tx の channel 0 が slot A の UBX/OBX のときだけ走る．
- 時間：2 channel の向きで 27 段（約 25 段が走る．約 26 分）× 2 向き，同時運転の 1 組，companion．
- 確認：`measurement.json` の 2 channel の throughput 条件（stream 全 channel 1 本 / channel ごと 1 本 / rate 違い），容量の行の condition（値を持ちこたえた条件）と `provider_*_throughput_capacity` の `blocker_condition`，同時運転の 1 組の `rx_limit_condition` と `tx_limit_condition`，`not run` の群（latch が立った印）．stop は (b) と同じ．XG と 10 GbE 2 本（BP4c）はこの日の範囲外．

### (f) 送信の実時間の優先度と窓の端の最小の send（段階 TXP の受け入れ B0〜B4．#1＋OBX，TX あり．約 30 分，estimate）

コードの段階は FakeDevice の試験までで，優先度が実機で効くかは**未確認（INFERRED）**．`tx_realtime` は「`uhd_set_thread_priority` が成功した」であって「実時間になった」ではない（UHD が `HAVE_THREAD_PRIO_DUMMY` で build されていると何もせず成功する）ので，policy は `/proc` で読む．**送信するので，他の送信の手順と同じに，owner が att（TX/RX → ≥30 dB → RX2）を確認してから**．毎回，記録するもの：Manifest の `stats`（`tx_realtime`，`tx_samples`，`tx_errors`），`rejected`，`events` の `TX_UNDERFLOW`／`TIME_ERROR`，`bursts[].blocks`；uhd-tx／uhd-rx／uhd-control の各 thread の `/proc/<pid>/task/<tid>/stat`（第 41 欄 policy，RR = 2；第 18 欄 priority）；`/proc/sys/kernel/sched_rt_runtime_us`・`sched_rt_period_us`（container の中と host）；uhd-rx の `rx_blocks`・overflow；timed 命令の `applied` の遅れ；Stop の完了までの時間．

- **B0 低い rate（窓が律速）**：1 MS/s と 5 MS/s，repeat の waveform **31 000 sample**，60 s．TX chunk は `min(20 000, window/4)` なのでこの rate では 2 500 / 12 500 sample（`DEFAULT_BLOCK_LEN = 2 000` は RX のまま）．**1 回ごとの send の大きさは Manifest に無い**（`bursts[].blocks` は block の数 u32 で，列ではない：`kernel stream/burst.rs`，`provider/tx.rs` の `record`）．大きさの分類は別の**有限の診断 run**（1 MS/s と 5 MS/s で各約 5 s）で，専用の `#[ignore]` 試験 `hw_txp_b0_send_sizes`（`tests/hardware.rs`）が device を `ProbeDevice` で包んで走らせ，31 000 sample の repeat（振幅 ≤ 0.5，gain 0 dB）の `tx_send` の行の `sample_counts[0]` を空の end-of-burst（0）を除いて `B0 sizes rate <hz>: …` と印字・検査する．**送信するので先に owner が att を確認してから**，(c) の probe と同じ形で：

```sh
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_txp_b0_send_sizes -- --exact --ignored --nocapture \
  > ~/.cache/ezsdr-bench/$(date +%F)/hw_txp_b0_send_sizes.log 2>&1
```
その列を累積 offset で分類する：(i) 最初の timed SOB は利用できた window により短くてもよく，`1…chunk`；(ii) 後続 send は `min(chunk, 31 000 − offset)`（offset は直前までに渡した samples を waveform 長で割った余り）で，wrap で終わる send は残り，そのほかは chunk；(iii) chunk は 1 MS/s で 2 500，5 MS/s で 12 500．(ii) に違反があれば失敗．最初の SOB が chunk 全体なら各周期の列は `[2 500 × 12, 1 000]` / `[12 500, 12 500, 6 000]`．blocks/s は 60 s の run の Manifest の `bursts[].blocks` の総数（block 数）を 60 で割る（production に send ごとの保存は足さない）．基準の run（`d0a0e64` の build，同じ設定）と比べ，uhd-tx の `blocks`／s が `rate / chunk × 1.1` 以下に減り，uhd-tx の CPU 時間（`/proc/<tid>/stat` の utime + stime を 60 s で割る）と自発的でない context switch（`/proc/<tid>/status`）が基準より減ることを**記録**する（割合の閾値は INFERRED で gate にしない）．underrun 0．
- **B1 100 MS/s**・**B2 200 MS/s**：60 s，`--cap-add=SYS_NICE --ulimit rtprio=99` つき．`tx_realtime` = owner 数，uhd-tx の policy = 2．underrun を `benchmark_rate --priority high`（(d) の記録）と比べる．
- **B3 同時運転**：送信 100 MS/s + 受信 + timed の control 更新（frequency／gain）+ burst 途中の Stop．uhd-rx の overflow 0，timed 命令の遅れが release 窓内，Stop が 1 s 以内に完了．uhd-rx／uhd-control の runnable 待ちを `/proc/<tid>/schedstat` で記録（実時間の uhd-tx が同じ core の通常 thread を飢えさせる兆候）．
- **B4 cap 無し**：`--ulimit rtprio=0` で cap なし（と host の通常 user）：`rejected` に `set_thread_priority` の行が 1 本出て run が完走し，DEVICE_LOST にならない．

結果は bench-results.md へ（orchestrator）．KL-5（50 MS/s を超える送信の失敗）の書き換えは B1〜B3 の結果を見てから．

### 流した後

- repo へ写すのは**完了した最終の BP2/BP3 の `measurement.json` だけ**：`crates/ezsdr-radio/measurements/<日付>-<serial>-<構成>.json`（41 §2）．shake-out，`.partial.json`，P4〜P7・MA-1・benchmark_rate の log は写さない．写す前に owner と確認．
- orchestrator に返すもの：commit hash，run directory の path，各ブロックの結果（完走 / 止まった場所と最後の log 行），`probe.log`（FPGA，serial）と `ref_locked`，att の型番・電力定格，電源 cycle の有無，P4〜P7 と MA-1 の出力，benchmark_rate の summary，`measurement.json` の観測の要約（overflow gap の最大・最小，coercion の格子，throughput の最大成功段，同時運転の 1 組の verdict）．bench-results.md への追記は orchestrator．
- 結果から出る owner の決め：overflow_restart_gap_ns と RM-17（最大と最小の差が最大の半分以下ならその最大を値に）；R4 の採否（P6）；R2 の要否（P4）；F7（P5）；MA-1（MockRadio の MR-34 をそろえる段階の前に，位相のばらつきの 2 通りの差）；受信・送信の throughput の値（profile 1000 MB/s）は 2 channel の結果を待って段階 4 で決める．

## 2026-11-01 の手順（D9b-1a・D9b-2・L1・L2 の実機確認，#70・#72）

**この節は 2026-11-05 の節に置き換えられた（履歴として残す）．**

`<COMMIT>` は D9b-1a・D9b-2・L1・L2 と本日の試験（`hardware.rs` の `hw_p72_…`・`hw_p70_…`・`hw_p4_…`・`hw_l1_…`・`hw_noise_floor_receive`，`hw_txp_b0_send_sizes` の修正，`hw_b9_transmit_only_session` の `EZSDR_B9_SECONDS`，`plan/phase8/tools/ssh-drop-tx.sh`）が入った commit（orchestrator が決める）．書いていないことは「未確認」．出典：この文書の上，[bench-results.md](bench-results.md)（2026-10-28，10-29），[39 §6](39-dsp-timed-commands.md)，[44 §3–§4](44-device-chain-state.md)，handoff §3（現在の構成）．

### 構成と順番

上の古い節のアドレスと NIC は 2026-10-29 より前の配線のもの．この節はこの表に従う．

| | 機材 | 接続 | att |
|---|---|---|---|
| #1 | X300＋OBX（slot A） | `addr=192.168.40.36`，NIC `enp1s0f1np1`，`--subnet 40` | 40 dB |
| #2 | X300＋UBX（slot A）・CBX（slot B） | `addr=192.168.47.37`，NIC `enp1s0f0np0`，`--subnet 47` | 各 30 dB |

- 2 台は同時につながない（owner）．機材の順は **#2（(a)・(b)・(c)・(e)）→ 盤を替えて #1（(a)・(b)・(c)・(d)・(f)）**．ブロックの番号は見出しの順で，実行は上の機材の順．
- 試験はすべて slot A（OBX／UBX）で行う．CBX（slot B）は 1.2 GHz 以上で `SAFE_TX_HZ`（999.5〜1000.5 MHz）の外なので，この日は流さない．
- 注意：上の表の address と NIC は handoff §3（2026-10-29 の構成）に合わせた．段階 3e の節の表も同じ値に直した．(a) で `bench-host.sh` と `uhd_usrp_probe` が通らなければ止めて owner へ．
- 基準は内部（`internal`）．#1 から #2 への PPS と 10 MHz は配線済みだが，この日の試験は使わない．
- 時間は表の下の合計に入れた estimate．初回の build（volume が冷たい）は数分から十数分の追加（estimate）．

### 共通の準備（送信しない）

```sh
cd <COMMIT の tree>                      # main tree で commit を checkout した状態．git status が clean であること
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
hw() { # $1 = 試験名．EZSDR_UHD_ARGS は呼ぶ前に機材ごとに export する．失敗なら非 0 を返す
  local log=~/.cache/ezsdr-bench/$(date +%F)/$1.log status=0
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_D9_P5_CASE -e EZSDR_B9_SECONDS -e EZSDR_TX200_LENGTHS -e EZSDR_TX200_SECONDS \
    ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$1" -- --exact --ignored --nocapture --test-threads=1 \
    >"$log" 2>&1 || status=$?
  grep -h 'VERDICT\|test result\|panicked' "$log"
  ((status == 0)) && [[ $(grep -c '^test result:' "$log") == 1 ]] && grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"
}
run_all() { for t in "$@"; do hw "$t" || return 1; done; } # 最初の失敗で止まる
```
`VERDICT <主張>: <PASS|FAIL|INCONCLUSIVE>: <内容>` の行を読む．`hw` は docker／cargo の終了 status と，`test result:` の行がちょうど 1 つで `ok. 1 passed; 0 failed; 0 ignored;` であることの両方を見る．失敗すると `run_all` は止まり，同じ機材の残りも次の機材も流さない．log の panic を読み，owner に報告してから再開する．

### (a) ホスト確認と装置確認（送信なし，#2 → #1，約 10 分，estimate）

```sh
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 47            # #2．read-only．不足があれば exit 1
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.47.37
# 盤を替えて #1
plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36
```
確認：serial（#2 は 347D541，#1 は 347D545），FPGA 39.3，slot A の board（UBX／OBX），master clock 200 MHz，`ref_locked` が true，10 GbE link．stop：NIC の不足が残る，find されない，ref_locked が false → 止めて owner へ．

### (b) raw probe で機構を再確認（#2 UBX → #1 OBX，TX あり，約 35 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 && run_all hw_d9_p5_a_stale_timed_header_after_a_reopen hw_d9_p6_every_chunk_timed hw_d9_p7_the_current_transmit_shape \
  hw_d70_one_sample_burst_then_timed_burst hw_d9_p4_ddc_entry_after_an_early_stop                # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_d9_p5_a_stale_timed_header_after_a_reopen hw_d9_p6_every_chunk_timed hw_d9_p7_the_current_transmit_shape \
  hw_d70_one_sample_burst_then_timed_burst hw_d9_p4_ddc_entry_after_an_early_stop                # 盤を替えて #1 OBX（#2 が止まったら流さない）
```
- これは raw probe（`raw_uhd()`，Module を通さない）．目的は今日のイメージで前の結果が再現するかの確認．
- P5 は 3 case（A：時刻つきの前の burst が終わった後の stale header，B：時刻なし終了，C：reopen 後の 2 サンプル）．同じ process で再現しなければ `EZSDR_D9_P5_CASE=timed`・`untimed`・`reopen` で 1 case ずつ（`hw` が渡す）．
- 期待（bench-results 2026-10-28，10-29 の記録）：P5 C と P6 は DUC の移動が e＋25 µs；P7 は e＋40 ms まで動かない（F3′．本番の送信と同じ形を対照に残す）；D70 は 1 サンプルの burst の後の時刻つき burst が約 150 ms 早く出る（#70 の再現；2 サンプルの対照は時刻どおり）；P4 は止めた後の DDC の設定が reopen の最初の sample に残る（A，C），AUTO（B）は残らない．
- stop：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock` → 電源を切って入れ直す（上の規則）．期待と違う（P6 が e より後，P7 が早く動く，D70 の対照が早い）→ (c) に進まず owner へ．

### (c) 本番の試験 4 本（#2 UBX → #1 OBX，TX あり，約 25 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 && run_all hw_p72_timed_tx_chunks_are_timed_and_flushed hw_p70_one_sample_burst_then_timed_burst \
  hw_p4_rx_timed_tune_then_early_stop hw_l1_held_rx_tune_applies_when_its_segment_streams                # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_p72_timed_tx_chunks_are_timed_and_flushed hw_p70_one_sample_burst_then_timed_burst \
  hw_p4_rx_timed_tune_then_early_stop hw_l1_held_rx_tune_applies_when_its_segment_streams                # 盤を替えて #1 OBX（#2 が止まったら流さない）
```
期待される VERDICT（本番の Module を Session または Spec Run で通す）．各試験の log には T3 の証拠（`flush evidence:` または `#72 1a:` の行：送った flush と ACK の数）が出る．
- (c)1 `hw_p72_…`：`#72 1a`（T1：全 send に時刻，T2：closing send（0 sample の EOB を含む）が 2 sample 以上，T3：flush の ACK が 1 つ以上）は **PASS**．FAIL は止めて owner へ．INCONCLUSIVE（payload が 2 send 未満，closing send がない，flush がない，ACK が flush より少ない）は閉じる根拠に数えない．1b（DUC の tune）は取り除いた（P3）．
- (c)2 `hw_p70_…`：booking の 2 通り（最初と一緒に／最初を送った後）とも **PASS**（one-sample と 2 sample の対照の onset の差が 50 sample 以下，TIME_ERROR と TX_UNDERFLOW なし）．INCONCLUSIVE は連続性の gap か，onset がない理由が行に出る．
- (c)3 `hw_p4_…`：case ごとに次の VERDICT が出る（early＝cut 18 000，e より前；control＝cut 26 000，e の後）．
  - handover：timed receive change が stop の booking より前に device に返った → **PASS**（後なら INCONCLUSIVE）．
  - stop の effective cut：early は e より前，control は e 以降 → **PASS**．逆なら INCONCLUSIVE（「case に入っていない」）．
  - published sample が cut と一致 → **PASS**．rejected の rx_drain・rx_restore がない → **PASS**（あれば FAIL）．
  - device の sample（e の後を含む．publication の cut とは別に分類）で 100 kHz → 50 kHz → **PASS**．early で e の後の sample が保持されず分類できないときは **INCONCLUSIVE**（理由が出る）．この場合 P4 の early は閉じない．
  - 同じ device の次の Run（successor segment ではなく別の Run）が 100 kHz を受ける → **PASS**．
  - VERDICT ではなく印字だけ：「successor segment in the same Run: not exercised」，「restoration after Provider::stop: not observed」．
- (c)4 `hw_l1_…`：1 つの Spec Run に 2 segment を置く（rx を 40 ms で stop，50 ms で start_rx，150 ms で rx 周波数の変更．変更は Run の開始時に予約される）．2 行とも **PASS**：「L1 smoke: the first segment holds 100 kHz」，「L1 smoke: the second segment holds 100 kHz up to e and moves to 50 kHz at e」．held の経路（変更の e が次の segment の準備中に来る場合）は**未実行**と記録する（印字は出ない）．

閉じる規則は下の「#70・#72 の閉じ方」．

### (d) L2 regression：既存の Session・Spec の送信の試験（#1 OBX，TX あり，約 20 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_b8_preemption hw_h5_held_last_sample hw_txp_b0_send_sizes && EZSDR_B9_SECONDS=60 hw hw_b9_transmit_only_session   # 最後は送信しない smoke（60 s）
```
- 期待：各 test が `test result: ok. 1 passed`．`hw_b6` は `B6 bursts:`，`hw_b7` は `B7 time errors: []`．`hw_txp_b0_send_sizes` は各 rate の `B0 sizes rate …` で，payload の大きさが窓の規則に合い，closing send が 2 sample 以上（T2）．
- 対象は Session か Spec Run のもの．raw（`raw()`，`raw_uhd()`）は含まない．
- optional（raw，機構の観測．L2 の判定には入れない）：`hw_h1_late_multi_packet_burst`，`hw_h2_underflow_mid_burst`．`hw <名前>` で流し，結果は L2 の表とは別に記録する（約 5 分）．

### (e) TX200：200 MS/s の送信（#2 UBX，1 channel，約 15 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**．TX200 は PN（200 MS/s，帯域が広い）を送り，`SAFE_TX_HZ` の範囲に収まらない．**owner が att と広帯域の送信を確認してから流す**（2026-10-29 に owner の確認のもとで流した）．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37                        # #2 UBX slot A（1 channel）
EZSDR_TX200_LENGTHS=1000,20000 EZSDR_TX200_SECONDS=300 hw hw_tx200_hot_path_probe
```
- 期待：`TX200 verdict: 2 of 2 trials succeeded`．各試行の行は `class=Success`，underflow 0（2026-10-29 TXS の記録と同じ：1 000 と 20 000 の両方が 300 s を通る）．1 試行でも `failed` が出たら，その試行の行（長さ・shape・class）を残して owner へ．
- 結果は 2026-10-29 TXS の記録と比べる．どちらも通れば TXS の結論（200 MS/s の 1 channel は 300 s を通る）を今日の commit で再確認したことになる．

### (f) 送信中の ssh 切断（#1 OBX，TX あり，約 8 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

前提：`ssh localhost` が鍵認証（`BatchMode`）で通ること．独立の受信機を owner に聞く（スペクトラムアナライザ，または #1 の TX を別の受信に配線）．**無いなら `EZSDR_DROP_RX_CHECK` を置かずに流す**：その場合 C2 は NOT DONE と記録され，(f) は閉じない（C1 と C3 の結果は記録する）．

```sh
cd <COMMIT の tree>
EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
EZSDR_DROP_RX_CHECK='<独立の受信機に tone か quiet を最後の行に出すコマンド（owner が決める）>' \
  plan/phase8/tools/ssh-drop-tx.sh <COMMIT> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/f-ssh-drop.console.log
```
- Session は本番の経路：`ssh-drop-driver.py` が組む profile（UHD Module 0.4.0，profile x3xx 0.1.0，slot A の OBX，rx0 の feed，Attached の Lease（server の既定））で `ezsdr.connect()` し，`EZSDR_SERVER` の ssh wrapper（`ssh -T -o BatchMode=yes -o ControlMaster=no -o ControlPath=none localhost`）越しに `ezsdr-v4-dev:uhd4.10` の中の `ezsdr-server` を起動する．100 kHz の tone を 1 GHz・振幅 0.4 で repeat する．
- 順序：`tx-started` の後 5 s で **local の ssh client を PID で殺す**（wrapper が書いた pidfile）．drop の前に受信機を読めなければ（失敗・timeout），drop せずに止まる．drop の後は server の終了と受信機の quiet を，drop から 15 s の**一つの期限**で待つ（受信機の 1 回の読みは 10 s まで）．C1 と C2 の VERDICT を先に出し，**その後で**まだ動いている server を止める（後始末であって判定には使わない）．C3 は受信機の最後の読みが tone でなければ走る（読めないときも走る．その時点で送信は止まっている）．
- 確認（3 行の VERDICT）：C1 server が終了し，Manifest に `client_disconnect`（attached の Lease）．C2 期限より前に受信機が quiet を読めば PASS．期限まで tone が残れば FAIL．期限の後に初めて quiet を読んでも FAIL．読めなければ INCONCLUSIVE．受信機が無ければ NOT DONE．C3 `hw_noise_floor_receive`（同じ device を新しい Session で開き直し，coherent tone が 1 % 未満）．**exit 0 は C1・C2・C3 がすべて PASS のときだけ**．
- 注意：tone が止まったことは，clean な end of burst と FPGA の飢えを区別しない．RF の出力が 0 であることも示さない（LO の漏れが残りうる）．
- C1 FAIL（15 s 後も server が動いていた）は owner へ．後始末の後も `docker ps` に `ezsdr-drop-` の container が残るなら owner へ．

### #70・#72 の閉じ方

- **#70**：(c)2 `hw_p70_…` が **#2 UBX と #1 OBX の両方で PASS** で閉じる．
- **#72**：(c)1 の `#72 1a` が **両 board で PASS**，かつ (b) の raw P6 が **両 board で PASS** で閉じる．INCONCLUSIVE は閉じる根拠に数えない．
- **INCONCLUSIVE は閉じない**．
- 記録：[bench-results.md](bench-results.md) の `## 2026-11-01：…` 節（まだない．bench の後に orchestrator が書く）に判定を書く．

### 時間の合計（estimate）

| ブロック | 内容 | 機材 | 時間 |
|---|---|---|---|
| (a) | host と装置の確認（TX なし） | #2 → #1 | 約 10 分 |
| (b) | raw probe 5 本 | #2 UBX，#1 OBX | 約 35 分 |
| (c) | 本番の試験 4 本 | #2 UBX，#1 OBX | 約 25 分 |
| (d) | L2 regression 7 本＋ smoke 1 本（optional の raw 2 本は +5 分） | #1 OBX | 約 20 分 |
| (e) | TX200（2 長さ × 300 s） | #2 UBX | 約 15 分 |
| (f) | ssh 切断 | #1 OBX | 約 8 分 |
| **合計** | 盤の交換と電源 cycle，初回の build を含まない | | **約 113 分（約 2 時間）** |

### 流した後

- orchestrator に返すもの：commit，各ブロックの VERDICT 行（log の path），(b) の P5〜P4 の値，(c)1 の `#72 1a:` 行（flush の数と ACK の数），(c)3 の successor と restoration の印字，(c)4 の held 未実行の印，(e) の行，(f) の 3 行と使った受信機（または NOT DONE），電源 cycle の有無．
- repo へ写すのは，owner が確認したものだけ．log は `~/.cache/ezsdr-bench/<日付>/` に残す．

## 2026-11-05 の手順（note 47 段階 3 の後）

`<COMMIT>` は note 47 段階 3（`d27718a` 以降）と本日の試験（`hw_n47_*`，`plan/phase8/reviews/bench-n47-renames.txt`）が入った commit（orchestrator が決める）．書いていないことは「未確認」．出典：この文書の上，[bench-results.md](bench-results.md)（2026-10-28，10-29），[39 §1・§6](39-dsp-timed-commands.md)，[47 §1・§2.1・§4](47-no-change-while-streaming.md)，`KNOWN_LIMITATIONS.md` の KL-35，2026-11-01 の節（置き換えた履歴）．

### 構成と順番

| | 機材 | 接続 | att |
|---|---|---|---|
| #1 | X300＋OBX（slot A） | `addr=192.168.40.36`，NIC `enp1s0f1np1`，`--subnet 40` | 40 dB |
| #2 | X300＋UBX（slot A）・CBX（slot B） | `addr=192.168.47.37`，NIC `enp1s0f0np0`，`--subnet 47` | 各 30 dB |

- 2 台は同時につながない（owner）．順は **#2 → #1**．ブロック (g) の T4（受信・送信）・T5 と (h) の T3 は **両方の盤**で流す（T6 は #2 のみ）．T4 の受信は送信を伴う（同じ盤の送信を 1 MS/s の tone 源にする）．
- 試験はすべて slot A（OBX／UBX）．CBX（slot B）は 1.2 GHz 以上で `SAFE_TX_HZ`（999.5〜1000.5 MHz）の外なので流さない．
- 基準は内部（`internal`）．#1 → #2 の PPS と 10 MHz はこの日も使わない．
- 時間は各ブロックの estimate．初回の build（volume が冷たい）は数分から十数分の追加（estimate）．

### 共通の準備（送信しない）

```sh
cd <COMMIT の tree>                      # main tree で commit を checkout した状態．git status が clean であること
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
hw() { # $1 = 試験名．EZSDR_UHD_ARGS は呼ぶ前に機材ごとに export する．失敗なら非 0 を返す
  local log=~/.cache/ezsdr-bench/$(date +%F)/$1.log status=0
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_D9_P5_CASE -e EZSDR_B9_SECONDS -e EZSDR_TX200_LENGTHS -e EZSDR_TX200_SECONDS \
    -e EZSDR_N47_T5_CONTROL \
    ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$1" -- --exact --ignored --nocapture --test-threads=1 \
    >"$log" 2>&1 || status=$?
  grep -h 'VERDICT\|test result\|panicked' "$log"
  ((status == 0)) && [[ $(grep -c '^test result:' "$log") == 1 ]] && grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"
}
run_all() { for t in "$@"; do hw "$t" || return 1; done; } # 最初の失敗で止まる
```

`VERDICT <主張>: <PASS|FAIL|INCONCLUSIVE>: <内容>` の行を読む．`hw` は docker／cargo の終了 status と，`test result:` の行がちょうど 1 つで `ok. 1 passed; 0 failed; 0 ignored;` であることの両方を見る．`EZSDR_N47_T5_CONTROL=1` は (g) の T5 の **破壊的な対照**（m = −5 ms）を流すときだけ，owner の明示の承認のもとで置く．置かなければ m = −5 ms は流れない．

stop の規則：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock`，装置が応答しない → **電源を切って入れ直す**．panic を読んで owner に報告してから再開する．`run_all` は最初の失敗で止まり，同じ盤の残りも次の盤も流さない．

### (a) ホスト確認と装置確認（送信なし，#2 → #1，約 10 分，estimate）

PC の再起動の後は，まず owner が `sudo bash ~/ezsdr-bench/usrp-net.sh`（tree の外．NIC ごとに subnet を一つだけ付け，各 X300 が自分の NIC で答えるかを確かめる．ケーブルが逆なら `CABLES SWAPPED` で exit 1）を流す．2026-11-05 は PC 側の 2 本のケーブルが逆で，両方の NIC に同じ subnet が付いていたため，ブロードキャストの `uhd_find_devices` だけが見え，`uhd_usrp_probe` は「No devices found」になった．

```sh
ping -c2 -W1 -I enp1s0f0np0 192.168.47.37 && ping -c2 -W1 -I enp1s0f1np1 192.168.40.36   # 各 X300 が自分の NIC で答える．答えなければ止める（ケーブルの取り違え）
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 47            # #2．read-only．不足があれば exit 1
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.47.37
# 盤を替えて #1
plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36
```

確認：serial（#2 は 347D541，#1 は 347D545），FPGA 39.3，slot A の board（UBX／OBX），master clock 200 MHz，`ref_locked` が true，10 GbE link．stop：NIC の不足が残る，find されない，`ref_locked` が false → 止めて owner へ．

### (b) raw probe で機構を再確認（#2 UBX → #1 OBX，TX あり，約 35 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 && run_all hw_d9_p5_a_stale_timed_header_after_a_reopen hw_d9_p6_every_chunk_timed hw_d9_p7_the_current_transmit_shape \
  hw_d70_one_sample_burst_then_timed_burst hw_d9_p4_ddc_entry_after_an_early_stop                # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_d9_p5_a_stale_timed_header_after_a_reopen hw_d9_p6_every_chunk_timed hw_d9_p7_the_current_transmit_shape \
  hw_d70_one_sample_burst_then_timed_burst hw_d9_p4_ddc_entry_after_an_early_stop                # 盤を替えて #1 OBX（#2 が止まったら流さない）
```

- これは raw probe（`raw_uhd()`，Module を通さない）．目的は今日のイメージで前の結果が再現するかの確認．
- P5 は 3 case（A：時刻つきの前の burst が終わった後の stale header，B：時刻なし終了，C：reopen 後の 2 サンプル）．同じ process で再現しなければ `EZSDR_D9_P5_CASE=timed`・`untimed`・`reopen` で 1 case ずつ（`hw` が渡す）．
- 期待（bench-results 2026-10-28，10-29 の記録）：P5 C と P6 は DUC の移動が e＋25 µs；**P7 は historical device-shape control**：旧 production の時刻なしの続きの形で，DUC の移動が e＋40 ms まで起きないことを示す（#72 の再現．今の Module は時刻なしの続きを持たない）；D70 は 1 サンプルの burst の後の時刻つき burst が約 150 ms 早く出る（#70 の再現；2 サンプルの対照は時刻どおり）；P4 は止めた後の DDC の設定が reopen の最初の sample に残る（A，C），AUTO（B）は残らない．
- stop：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock` → 電源を切って入れ直す．期待と違う（P6 が e より後，P7 が早く動く，D70 の対照が早い）→ (c) に進まず owner へ．

### (c) 本番の試験（#2 UBX → #1 OBX，TX あり，約 25 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 && run_all hw_p72_timed_tx_chunks_are_timed_and_flushed hw_p70_one_sample_burst_then_timed_burst \
  hw_n47_rx_restart_with_new_values                                                              # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_p72_timed_tx_chunks_are_timed_and_flushed hw_p70_one_sample_burst_then_timed_burst \
  hw_n47_rx_restart_with_new_values                                                              # 盤を替えて #1 OBX（#2 が止まったら流さない）
```

- (c)1 `hw_p72_…`：`#72 1a`（T1：全 send に時刻，T2：closing send（0 sample の EOB を含む）が 2 sample 以上，T3：flush の ACK が 1 つ以上）は **PASS**．FAIL は止めて owner へ．INCONCLUSIVE（payload が 2 send 未満，closing send がない，flush がない，ACK が flush より少ない）は閉じる根拠に数えない．
- (c)2 `hw_p70_…`：booking の 2 通りとも **PASS**（one-sample と 2 sample の対照の onset の差が 50 sample 以下，TIME_ERROR と TX_UNDERFLOW なし）．INCONCLUSIVE は連続性の gap か，onset がない理由が行に出る．
- (c)3 `hw_n47_rx_restart_with_new_values`（T2，note 47 段階 3）：次の 4 群の VERDICT が出る．
  - (a) 最初の segment が 100 kHz → **PASS**．INCONCLUSIVE は最初の 1 ms に coherent な窓がないとき．
  - (b) 流れている間に受信の周波数を f1＋50 kHz にすると，Kernel は admit し，Provider がその Action の id を持つ `radio.COMMAND_REJECTED` を 1 つ返す → **PASS**．Kernel が拒んだ，id が一致しない，2 つ以上 → FAIL．続く「gap なし・100 kHz のまま」は最初の segment の block の連続と tone で判定する．
  - (c) `Stop`（`radio/rx/s0` だけ）→ 周波数と gain の設定 → 時刻つき `start_rx`：期待する origin は**予約した segment の時刻**（`start_rx` の `at`，root tick）で，装置の call からは取らない．装置の start と最初の published block がその origin にあり，`GAP_BEFORE` が無い（capture の continuity に leading gap が無い）→ **PASS**．最初の 1 ms は**開始から終わりまで**連続した窓で覆われ，各窓が 50 kHz（途中の窓だけ正しくても PASS にしない）→ **PASS**．noise の床は**送信の repeat を止めた後**の静かな窓から測る（受信を止めていた区間からは測らない）．`radio.LATE_COMMAND` があれば FAIL．`radio.START_LEAD_SHORT` は **数えて印字するだけ**（FAIL にしない．KL-35：profile の lead はまだ測り直していない）．INCONCLUSIVE は origin の行がない，block がない，静かな窓がないとき．
  - (d) 新しい周波数と gain の read-back：**configure 時**の read-back（Manifest の `applied` の `claimed`/`read_back`/`difference`）で，Run の後の装置の再測定ではない → **PASS**．行がなければ INCONCLUSIVE．
  - アンテナは「not exercised」と印字する（loopback が 1 つの port に配線されている）．
- stop：T2 の FAIL，`radio.START_LEAD_SHORT` が 0 でないこと自体は止めないが，(h) の結果と一緒に記録する．(b) が INCONCLUSIVE なら (c) は続けてよいが，段階 3 の証拠には数えない．

### (d) L2 regression：既存の Session・Spec の送信の試験（#1 OBX，TX あり，約 20 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.40.36 && run_all hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_b8_preemption hw_h5_held_last_sample hw_txp_b0_send_sizes && EZSDR_B9_SECONDS=60 hw hw_b9_transmit_only_session   # 最後は送信しない smoke（60 s）
```

- 期待：各 test が `test result: ok. 1 passed`．`hw_b6` は `B6 bursts:`，`hw_b7` は `B7 time errors: []`．`hw_txp_b0_send_sizes` は各 rate の `B0 sizes rate …` で，payload の大きさが窓の規則に合い，closing send が 2 sample 以上（T2）．
- 対象は Session か Spec Run のもの．raw（`raw()`，`raw_uhd()`）は含まない．
- optional（raw，機構の観測．L2 の判定には入れない）：`hw_h1_late_multi_packet_burst`，`hw_h2_underflow_mid_burst`．`hw <名前>` で流し，結果は L2 の表とは別に記録する（約 5 分）．

### (e) TX200：200 MS/s の送信（#2 UBX，1 channel，約 15 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**．TX200 は PN（200 MS/s，帯域が広い）を送り，`SAFE_TX_HZ` の範囲に収まらない．**owner が att と広帯域の送信を確認してから流す**（2026-10-29 に owner の確認のもとで流した）．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37                        # #2 UBX slot A（1 channel）
EZSDR_TX200_LENGTHS=1000,20000 EZSDR_TX200_SECONDS=300 hw hw_tx200_hot_path_probe
```

- 期待：`TX200 verdict: 2 of 2 trials succeeded`．各試行の行は `class=Success`，underflow 0（2026-10-29 TXS の記録と同じ：1 000 と 20 000 の両方が 300 s を通る）．1 試行でも `failed` が出たら，その試行の行（長さ・shape・class）を残して owner へ．
- 結果は 2026-10-29 TXS の記録と比べる．どちらも通れば TXS の結論（200 MS/s の 1 channel は 300 s を通る）を今日の commit で再確認したことになる．

### (f) 送信中の ssh 切断（#1 OBX，TX あり，約 8 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

前提：`ssh localhost` が鍵認証（`BatchMode`）で通ること．独立の受信機を owner に聞く（スペクトラムアナライザ，または #1 の TX を別の受信に配線）．**無いなら `EZSDR_DROP_RX_CHECK` を置かずに流す**：その場合 C2 は NOT DONE と記録され，(f) は閉じない（C1 と C3 の結果は記録する）．

```sh
cd <COMMIT の tree>
EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
EZSDR_DROP_RX_CHECK='<独立の受信機に tone か quiet を最後の行に出すコマンド（owner が決める）>' \
  plan/phase8/tools/ssh-drop-tx.sh <COMMIT> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/f-ssh-drop.console.log
```

- Session は本番の経路：`ssh-drop-driver.py` が組む profile（UHD Module 0.4.0，profile x3xx 0.1.0，slot A の OBX，rx0 の feed，Attached の Lease（server の既定））で `ezsdr.connect()` し，`EZSDR_SERVER` の ssh wrapper 越しに `ezsdr-v4-dev:uhd4.10` の中の `ezsdr-server` を起動する．送信の channel は **connect の時**に組に入る（`.stream` を Run の中で変えない）．100 kHz の tone を 1 GHz・振幅 0.4 で repeat する．
- 順序：`tx-started` の後 5 s で local の ssh client を PID で殺す（wrapper が書いた pidfile）．drop の前に受信機を読めなければ（失敗・timeout），drop せずに止まる．drop の後は server の終了と受信機の quiet を，drop から 15 s の一つの期限で待つ（受信機の 1 回の読みは 10 s まで）．C1 と C2 の VERDICT を先に出し，**その後で**まだ動いている server を止める（後始末であって判定には使わない）．C3 は受信機の最後の読みが tone でなければ走る（読めないときも走る．その時点で送信は止まっている）．
- 確認（3 行の VERDICT）：C1 server が終了し，Manifest に `client_disconnect`（attached の Lease）．C2 期限より前に受信機が quiet を読めば PASS．期限まで tone が残れば FAIL．期限の後に初めて quiet を読んでも FAIL．読めなければ INCONCLUSIVE．受信機が無ければ NOT DONE．C3 `hw_noise_floor_receive`（同じ device を新しい Session で開き直し，coherent tone が 1 % 未満）．**exit 0 は C1・C2・C3 がすべて PASS のときだけ**．
- 注意：tone が止まったことは，clean な end of burst と FPGA の飢えを区別しない．RF の出力が 0 であることも示さない（LO の漏れが残りうる）．
- C1 FAIL（15 s 後も server が動いていた）は owner へ．後始末の後も `docker ps` に `ezsdr-drop-` の container が残るなら owner へ．

### (g) note 47 の生の probe（#2 UBX → #1 OBX，TX あり，約 50 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37
run_all hw_n47_raw_rx_rate_on_an_open_streamer hw_n47_raw_tx_rate_on_an_open_streamer hw_n47_raw_tx_setting_after_a_burst_end \
  hw_n47_raw_control_call_durations hw_n47_raw_rx_buffer_capacity                                # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36
run_all hw_n47_raw_rx_rate_on_an_open_streamer hw_n47_raw_tx_rate_on_an_open_streamer hw_n47_raw_tx_setting_after_a_burst_end
# T6 は #2 のみ（この行は #1 では流さない）
```

```sh
# m = −5 ms の破壊的な対照は **owner の明示の承認のもとでのみ**（盤を替えたら EZSDR_UHD_ARGS も替える）
export EZSDR_UHD_ARGS=addr=192.168.47.37
EZSDR_N47_T5_CONTROL=1 hw hw_n47_raw_tx_setting_after_a_burst_end
```

- **T4 受信** `hw_n47_raw_rx_rate_on_an_open_streamer`：レート 1 → 2 → 10 → 1 MS/s の各変化に 1 行の VERDICT．tone 源は固定 1 MS/s で試行ごとに送り直し，各試行の後にその burst の終わりを ACK で確かめてから次へ進む．古い stream の packet は静かな区間まで drain し，静かにならなければ**その rate は適用せず**その試行は INCONCLUSIVE（次の試行の前にまた drain する）．PASS の条件：`Applied.rate` が頼んだ値，最初の新しい block が計画の tick，連続する block の tick が sample 数×（200 MHz / rate）だけ進む，loopback の tone が新しい rate でもその Hz にある，`rx_open` が probe 全体で 1 回．FAIL は止めて owner へ．INCONCLUSIVE は新しい block がない，tone がない，または drain が静かにならないとき（loopback を疑う）．
- **T4 送信** `hw_n47_raw_tx_rate_on_an_open_streamer`：A の末尾の 1 ms が A の tone，A の burst ACK が A の排他的な終わり e の窓 `[e − 2 sample, e + 1 tick]` の中（駆動側は EOB sample を消費した時点で ACK するので最大 1 sample 早い），B がその tick（control burst で測った loopback の遅れを含む）に始まり B の tone が**100 kHz**（B が送る tone．搬送波は変えない），`Applied.rate == R2`，TIME_ERROR／underflow なし，A と B の窓にそれぞれ burst ACK が 1 つだけ（**すべて数える**ので重複は FAIL），`tx_open` が 1 回 → **PASS**．ACK の e からの差は必ず印字する．INCONCLUSIVE は A か B の ACK がない，ACK が窓の外（その burst の終わりは確認できない），tone がないとき．FAIL は止めて owner へ．
- **T5** `hw_n47_raw_tx_setting_after_a_burst_end`：m = 0，＋1，＋5，＋20 ms の各試行で，A の burst ACK が e の窓 `[e − 2 sample, e + 1 tick]` の中（外なら INCONCLUSIVE．差は印字する），A の末尾が f1 のまま（tone・電力・移動の有無．**capture の窓は control burst で測った loopback の遅れだけずらす**），B がその遅れを含む最初の窓から f2 → **PASS**．＋5 ms の gain の試行は gain を 0 dB から **＋6 dB** に上げ（att の確認はこの節の冒頭のとおり．owner は 2026-10-27 に，att を loopback に入れたまま送信 gain を最大まで許している），read-back を assert し，A の末尾の振幅が **1 dB 以内**（電力で 10^(±1/10)）で不変，B の最初の窓の振幅が A の定常より ＋6 dB ± 1.5 dB → **PASS**．m = −5 ms は `EZSDR_N47_T5_CONTROL=1` を **owner の承認のもとで**置いたときだけ流す **破壊的な対照**で，A の末尾が動く／切れる（hazard が見える）→ PASS，見えなければ INCONCLUSIVE（FAIL ではない）．この `apply` は A の ACK を待つ前に出す．各試行は `apply` の装置時刻の bracket（issue と return）で分類し，bracket が e より前に無い（issue か return が e 以降の）−5 ms の試行は **PASS にせず INCONCLUSIVE**；m ≥ 0 の試行も issue が e より前なら INCONCLUSIVE．m = −5 ms は production の操作ではない．各試行の bracket と，e からの margin を行に残す．試行の前に送信の周波数と gain 0 dB を戻し，終わりにも gain 0 dB に戻す．結果は**保証された最小 margin ではなく観測の集合**として段階 4 に渡す．
- **T6 呼び出しの時間** `hw_n47_raw_control_call_durations`：50 回ずつの min／median／p99／max を印字する（VERDICT ではない）．行の多くは setter＋read-back／enqueue の時間だが，**`rx_tune_auto_with_command_time` の行だけは** `UhdDevice::tune_auto` を命令の時刻つきで呼ぶ時間（受信の stream で命令の時刻を一つ先に置く）で，どの行も **RF の整定でも時刻つきの実行の遅れでもない**．その行の周波数変更は**受信だけ**を，その instant を覆う受信 sample がある間に行い，覆えなくなったら**そこで `rx_stop` して panic する**（timed の命令が装置の FIFO に残りうる．**次の試験の前に装置を電源で入れ直す**；それ以上の `apply` も受信の reopen もしない）．**送信の timed は測らない**：設計メモ 39 §1 と raw P7 のとおり，時刻なしの続きは DUC を drain しないので，timed の送信命令は FIFO に残って X300 を詰まらせうる（queue すること自体がこの probe のしてよい測定ではない）．送信の gain と rate の時間は**送信 sample を流さずに**測る（0 ↔ ＋1 dB，1 ↔ 2 MS/s）．gain の測定の後に送信 gain を **0 dB** に戻す．
- **T6 受信 buffer** `hw_n47_raw_rx_buffer_capacity`：1，10，50 MS/s で，まず連続性の基準を確かめ，100 ms と 5 s の間を（観測された bracket の中だけ）最大 8 試行で二分する．各試行は stall の前に「restart がその tick から 20 ms まで途切れずに流れた」ことを確かめ，overflow が無いときは drain した sample が stall の区間を覆っていることを確かめる．どちらも示せない試行は **incomplete** として記録し，二分には使わない（受信が止まっていたのを「余裕がある」と読まない）．最初に `Overflow` した stall と，しなかった最長の stall を記録する．overflow が 100 ms 未満なら「overflow below 100 ms」，5 s でも出なければ「none through 5 s」と記録する．50 MS/s は 2026-10-28 の記録が 200 MS/s で 800 MB/s を出しているので候補にするが，今日の盤の基準が通らなければ「測らない」と記録する．`net.core.rmem_max`／`rmem_default` と UHD の `recv_buff_size`（device args になければ既定，実際の socket の大きさは不明）を印字する．sysctl は変えない．timed の命令はこの probe に入れない．
- stop：T4・T5 の FAIL，装置の詰まり（`Control operation timed out…`），T6 の timed 受信で coverage を失った panic（`n47 control calls: the timed receive apply lost coverage`）→ **電源を切って入れ直す**（次の試験を同じ装置で流さない），owner へ．

### (h) T3：受信の start lead の群だけを流す（#2 UBX → #1 OBX，TX あり（companion），約 20 分，estimate）

**送信の前に owner に att を確認する（#1 OBX 40 dB，#2 各 30 dB）**．companion の群は PN を 10 MS/s で repeat する（広帯域の送信．owner の att の確認のもとで流す）．companion の送信 rate は connect の時に 10 MS/s を要求して clock の read-back を確かめ，条件の行には**実際に read-back した rate** を書く（profile がその rate を表せなければ既定の 1 MS/s で走らせ，log に理由を出す）．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 && hw hw_n47_rx_start_lead_group        # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 && hw hw_n47_rx_start_lead_group        # 盤を替えて #1 OBX
```

- 条件は `restart-same`（値を変えずに `Stop` → `start_rx`）と `restart-retune`（1 GHz の調整経路の境界を毎回またぐ周波数．999.5 MHz と 1000.5 MHz）．候補は `radio.timing.rx_start_lead_ns` の階段（3，5，10，25，50，100 ms）．
- 各段階は，`radio.LATE_COMMAND` も `radio.START_LEAD_SHORT` もなく，装置の start と最初の published block が**Module が予約した origin**（その restart の SampleClock の origin）にあるときだけ成功．`GAP_BEFORE` はこの最初の block の位置で判定する（Sink は「最初の block が要求より後」にだけ印を付ける）．各 trial は warning の有無にかかわらず `lead_left_ns`（予約した origin − configure の完了時刻．configure の `applied` の行，warning の payload があればそれ）を記録する．**PASS** は最小の候補が全条件を満たすこと；**INCONCLUSIVE** はどの候補も満たさない（理由が値の行に出る）．FAIL はない（測れなかった理由を記録する）．
- 結果は「測った条件」として記録し，最悪の場合の保証とは書かない．profile の `rx_start_lead_ns` はこの場では変えない（段階 4 の材料）．`radio.timing.tx_start_lead_ns` は「未測定」のまま（段階 4）．fake の rehearsal が late・short・成功の 3 候補を覆う（`rx_start_lead_group_runs_on_the_fake`）．

### 閉じ方と記録

- **#70**：(c)2 の `hw_p70_…` が **#2 UBX と #1 OBX の両方で PASS** で閉じる．
- **#72**：(c)1 の `#72 1a` が **両 board で PASS**，かつ (b) の raw P6 が **両 board で PASS** で閉じる．INCONCLUSIVE は閉じる根拠に数えない．
- **受信の rate の受け入れ条件**（note 47 §1・§4 段階 3）：(c)3 の T2 と (g) の T4 受信が **両 board で PASS** のときだけ閉じる（新しい rate，旧 burst の末尾，時刻，最初の sample，read-back，streamer を作り直さないこと）．**送信の rate の受け入れ条件**（段階 4）：(g) の T4 送信が **両 board で PASS** のときだけ閉じる．
- **T5 の結果**は段階 4 が使う margin として [bench-results.md](bench-results.md) の 2026-11-05 の節（bench の後に orchestrator が書く）に，bracket と hazard の有無をそのまま残す．**T3 の結果**は測った lead と最小の `lead_left_ns` を残す．
- 2026-11-01 の節の P4・L1 の期待と閉じ方の欄はこの節が置き換える（P4・L1 は note 47 段階 2 で消えた振る舞いを測っていた）．raw P7 は historical device-shape control として残す．bench-results.md には 11-01 の見出しがなかったので，この日も見出しを足さない（bench の後に orchestrator が書く）．

### 時間の合計（estimate）

| ブロック | 内容 | 機材 | 時間 |
|---|---|---|---|
| (a) | host と装置の確認（TX なし） | #2 → #1 | 約 10 分 |
| (b) | raw probe 5 本 | #2 UBX，#1 OBX | 約 35 分 |
| (c) | 本番の試験 3 本（T2 を含む） | #2 UBX，#1 OBX | 約 25 分 |
| (d) | L2 regression 7 本＋ smoke 1 本（optional の raw 2 本は +5 分） | #1 OBX | 約 20 分 |
| (e) | TX200（2 長さ × 300 s） | #2 UBX | 約 15 分 |
| (f) | ssh 切断 | #1 OBX | 約 8 分 |
| (g) | T4（受信・送信）・T5・T6 | #2 UBX，#1 OBX（T6 は #2） | 約 50 分 |
| (h) | T3 の群（companion を含む） | #2 UBX，#1 OBX | 約 20 分 |
| **合計** | 盤の交換と電源 cycle，初回の build を含まない | | **約 183 分（約 3 時間）** |

### 流した後

- orchestrator に返すもの：commit，各ブロックの VERDICT 行（log の path），(b) の P5〜P4 の値と P7 の historical の行，(c)1 の `#72 1a:` 行，(c)3 の T2 の 4 群と `radio.START_LEAD_SHORT` の数，(e) の行，(f) の 3 行と使った受信機（または NOT DONE），(g) の T4 の各 rate の行，T5 の各 m の bracket と hazard，T6 の統計と buffer の bracket，(h) の最小 lead と最小 `lead_left_ns`，電源 cycle の有無．
- repo へ写すのは，owner が確認したものだけ．log は `~/.cache/ezsdr-bench/<日付>/` に残す．

## 2026-11-06 の手順（note 48 の後）

`<COMMIT>` は T1〜T5（テスト，`common/iii.rs`，runbook）が入った commit（2026-11-06 は `4f7e9e4`．結果は [bench-results.md](bench-results.md) §「2026-11-06」）．書いていないことは「未確認」．出典：この文書の上の 2026-11-05 の節，[bench-results.md](bench-results.md) の 2026-11-05 の節とその C++ の続き，[48 の棚卸し](48-simplification-inventory.md) S1，[design/18](../../design/18-uhd-radio.md) UR-21・UR-22・UR-23・UR-28．

### 構成と順番

| | 機材 | 接続 | att |
|---|---|---|---|
| #1 | X300＋OBX（slot A） | `addr=192.168.40.36`，NIC `enp1s0f1np1`，`--subnet 40`，`BOARD=36` | 30 dB |
| #2 | X300＋UBX（slot A）・CBX（slot B） | `addr=192.168.47.37`，NIC `enp1s0f0np0`，`--subnet 47`，`BOARD=37` | 各 30 dB |

- att は三つの board とも 30 dB（owner 2026-11-06．2026-11-05 の当日の値に合わせた）．
- 2 台は同時につながない（owner）．順は **#2 → #1**．
- 試験はすべて slot A（OBX／UBX）．CBX（slot B）は 1.2 GHz 以上で `SAFE_TX_HZ`（999.5〜1000.5 MHz）の外なので流さない．`iii::pin_to_slot_a` が，試験ごとに slot A だけの layout を組む（TX200・T4 は slot B を送信の channel にしない）．
- 基準は内部（`internal`）．PPS と 10 MHz はこの日も使わない．
- 時間は各ブロックの estimate．初回の build（volume が冷たい）は数分から十数分の追加（estimate）．

### 共通の準備（送信しない）

```sh
cd <COMMIT の tree>                      # main tree で commit を checkout した状態．git status が clean であること
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
hw() { # $1 = 試験名．EZSDR_UHD_ARGS，BOARD（36 か 37）と EZSDR_BENCH_NIC は呼ぶ前に機材ごとに export する．失敗なら非 0 を返す
  local dir=~/.cache/ezsdr-bench/$(date +%F) log status=0 n=1
  log="$dir/$1.$BOARD.log"
  while [[ -e $log ]]; do n=$((n + 1)); log="$dir/$1.$BOARD.$n.log"; done # 既存の log は上書きしない（.2，.3）
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_D9_P5_CASE -e EZSDR_B9_SECONDS -e EZSDR_TX200_LENGTHS -e EZSDR_TX200_SECONDS \
    -e EZSDR_N47_T5_CONTROL -e EZSDR_LOAD200_SECONDS -e EZSDR_LOAD200_STAGES -e EZSDR_BENCH_NIC \
    ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$1" -- --exact --ignored --nocapture --test-threads=1 \
    >"$log" 2>&1 || status=$?
  grep -h 'VERDICT\|test result\|panicked' "$log"
  ((status == 0)) && [[ $(grep -c '^test result:' "$log") == 1 ]] && grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"
}
run_all() { for t in "$@"; do hw "$t" || return 1; done; } # 最初の失敗で止まる
```

- log は `~/.cache/ezsdr-bench/<日付>/<試験名>.<BOARD>.log` に残る．同じ試験を同じ盤でもう一度流すと `.2.log`，`.3.log` になる．
- `hw` は docker／cargo の終了 status と，`test result:` の行がちょうど 1 つで `ok. 1 passed; 0 failed; 0 ignored;` であることの両方を見る．`VERDICT` は「合格」と同じではない．INCONCLUSIVE の行は閉じる根拠に数えない．
- `EZSDR_LOAD200_SECONDS`（既定 600）と `EZSDR_LOAD200_STAGES`（既定 `rx,tx,duplex`；`rx`，`tx`，`duplex` の部分集合．ほかの名前は試験が開く前に panic する）は (f) だけで使う．`EZSDR_BENCH_NIC` は盤の NIC（#2 `enp1s0f0np0`，#1 `enp1s0f1np1`）で，(f) の NIC の byte と error／drop のカウンタを `/sys/class/net/<nic>/statistics/` から読む．コンテナは `--network=host` なので，ホストの sysfs を読む（(a) で一度確かめる）．
- `run_all` は最初の失敗で止まる．(f) は `hw` を盤ごとに一つずつ呼ぶ（下の (f) を見る）．

stop の規則：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock`，装置が応答しない → **電源を切って入れ直す**．panic を読んで owner に報告してから再開する．`run_200_stage` が `run_stage` の Err，「streamer を開いたまま」，または handle の latch（control timeout・device 喪失．`GuardedDevice::latched`）を返すと，その盤の以降の試行は流れない（`VERDICT` の行に「power-cycle the board」と出る）→ 電源を入れ直し，owner に伝える．

### (a) ホスト確認と装置確認（送信なし，#2 → #1，約 10 分，estimate）

PC の再起動の後は，まず owner が `sudo bash ~/ezsdr-bench/usrp-net.sh`（tree の外．NIC ごとに subnet を一つだけ付け，各 X300 が自分の NIC で答えるかを確かめる．ケーブルが逆なら `CABLES SWAPPED` で exit 1）を流す．

```sh
ping -c2 -W1 -I enp1s0f0np0 192.168.47.37 && ping -c2 -W1 -I enp1s0f1np1 192.168.40.36   # 各 X300 が自分の NIC で答える．答えなければ止める（ケーブルの取り違え）
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 47            # #2．read-only．不足があれば exit 1
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.47.37
# 盤を替えて #1
plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36
# NIC の counter が読めるか（(f) の EZSDR_BENCH_NIC が頼る）
cat /sys/class/net/enp1s0f0np0/statistics/rx_bytes /sys/class/net/enp1s0f1np1/statistics/rx_bytes
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 sh -c 'cat /sys/class/net/enp1s0f0np0/statistics/rx_bytes'   # コンテナからも読めること
```

確認：serial（#2 は 347D541，#1 は 347D545），FPGA 39.3，slot A の board（UBX／OBX），master clock 200 MHz，`ref_locked` が true，10 GbE link，NIC の counter がコンテナから読める．stop：NIC の不足が残る，find されない，`ref_locked` が false，counter が読めない → 止めて owner へ．

### (b) #78 と #70：Module の送信の試験（#2 UBX → #1 OBX，TX あり，約 30 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && run_all \
  hw_s1_short_first_sends hw_p70_one_sample_burst_then_timed_burst hw_p72_timed_tx_chunks_are_timed_and_flushed \
  hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_b8_preemption hw_h5_held_last_sample hw_txp_b0_send_sizes                          # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && run_all \
  hw_s1_short_first_sends hw_p70_one_sample_burst_then_timed_burst hw_p72_timed_tx_chunks_are_timed_and_flushed \
  hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_b8_preemption hw_h5_held_last_sample hw_txp_b0_send_sizes                          # 盤を替えて #1 OBX（#2 が止まったら流さない）
```

各試験の VERDICT と，その行が出す TIME_ERROR と TX_UNDERFLOW の数（`iii::faults_verdict`．`hw_b8_burst_one_sample_after_a_burst` は `iii::late_control_verdict`，`hw_b8_preemption` は `iii::preemption_verdict` で，どちらも TIME_ERROR の cause を見る）．期待する TIME_ERROR は case ごとに次のとおり．それ以外の `radio.TIME_ERROR` は**想定外**．

| 試験 | 期待する `radio.TIME_ERROR` | TX_UNDERFLOW | 根拠 |
|---|---|---|---|
| `hw_s1_short_first_sends`（1，2，3，4 サンプル） | 0（case ごとに 1 つずつ `BurstAck`） | 0 | UR-22（note 48 S1） |
| `hw_p70_…`（1 サンプルの burst と 4 サンプルの対照，booking の 2 通り） | 0（両方） | 0 | UR-22，UR-23 |
| `hw_p72_…` | 0 | 0 | UR-22，UR-28 |
| `hw_b6_txrx_and_repeat`（burst と repeat） | 0 | 0 | UR-22，UR-23 |
| `hw_b7_session_loopback` | 0 | 0 | UR-22 |
| `hw_b8_burst_at_a_sent_burst_s_end`（3 試行） | 0 | 0 | UR-23（B3） |
| `hw_b8_burst_one_sample_after_a_burst`（end + 1，end + 2，各 2 通り） | end + 1 は **1**（cause `late`），end + 2 は 0 | 0 | UR-22（最後の sample は閉じる送信に 1 sample の 0 を足して 2 sample に詰める．次の安全な開始は閉じる送信の排他的な終わり（詰め物を含む）＋1 sample＝payload の終わり＋2） |
| `hw_b8_preemption`（先行 burst の 20，10，5，3 ms） | 20 ms は 0，5 と 3 ms は 1，10 ms は 0 か 1（cause は `late`）．違えばその case は INCONCLUSIVE | 0 | UR-21（先頭の未引き渡し sample と device lead で判定．10 ms の境界は INFERRED） |
| `hw_h5_held_last_sample`（隣接・3 000 sample の間隔・締切の run） | 0 | 0 | UR-22 |
| `hw_txp_b0_send_sizes`（1 と 5 MS/s） | 0 | 0 | UR-22，TXP |

- **#78 が閉じるのは**：(b) の全試験が**両盤**で PASS のとき．PASS は表の「期待する `radio.TIME_ERROR`」と一致したときだけ．想定外の `radio.TIME_ERROR`（期待を超える，または cause が `late` でない．ただし `hw_b8_preemption` では 2 つ以上か cause が `late` でないときだけ FAIL で，lead ごとの期待との数の違いは INCONCLUSIVE），`radio.TX_UNDERFLOW`，device の async の fault は FAIL．期待する TIME_ERROR（`hw_b8_burst_one_sample_after_a_burst` の end + 1，`hw_b8_preemption` の 5 と 3 ms）は negative control であり，削らない．
- **#70 が閉じるのは**：`hw_p70_…` が**両盤**で PASS のとき．対照（4 サンプル）の障害も判定に入る．
- INCONCLUSIVE は閉じる根拠に数えない．FAIL は止めて owner へ．
- stop：想定外の `radio.TIME_ERROR` や `radio.TX_UNDERFLOW`，装置の詰まり → 電源を切って入れ直す（同じ盤で次の試験を流さない）．

### (c) 受信の restart：fill，start lead，raw の restart（#2 UBX → #1 OBX，TX あり（1 MS/s の tone 源と，companion の 10 MS/s），約 40 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．companion の群は PN を 10 MS/s で repeat する（広帯域の送信．owner の att の確認のもとで流す）．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && run_all \
  hw_iii_raw_rx_restart_fill hw_n47_rx_restart_with_new_values hw_n47_rx_start_lead_group hw_b8_raw_restart_lead   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && run_all \
  hw_iii_raw_rx_restart_fill hw_n47_rx_restart_with_new_values hw_n47_rx_start_lead_group hw_b8_raw_restart_lead   # 盤を替えて #1 OBX
```

- (c)1 `hw_iii_raw_rx_restart_fill`（T2）：8 つの受信 rate（1，2，5，10，25，50，100，200 MS/s）それぞれに 1 行の VERDICT と，「exact zeros in fc32」「90 % まで」の最小・中央値・最大（sample と ns）．fc32 は sc16 から変換した値で，全部 0 の sc16 は正確に 0.0 になる（その理由で「ワイヤの 0」と数える．直接の観測ではない）．PASS は三つ以上の完全な試行が計画の tick に最初の block を持つとき．**FAIL は最初の block が計画の tick にないときだけ**．INCONCLUSIVE は，試行が完全でない（rate の read-back が違う，span に穴，steady の tone がない，drain が静かにならない）ときで，理由が行に出る．最小・中央値・最大は完全な試行だけで取る．rate ごとの `III fill observation {…}` の行には全試行（計画の tick と最初の block，zeros，ramp10，ramp90，steady，tone，coverage，applied，complete と reason）が残る．log からその行を返す．
- (c)2 `hw_n47_rx_restart_with_new_values`：T2 の 4 群（note 47 段階 3）．
- (c)3 `hw_n47_rx_start_lead_group`：各候補の `lead_left_ns` と，最小の値．PASS は最小の候補が全条件を満たすとき．INCONCLUSIVE は満たす候補がない（理由が行に出る）．
- (c)4 `hw_b8_raw_restart_lead`：restart の lead（S2b-i 後）の観測．最小の `lead_left_ns` を残す．
- 受信の restart の結果の記録：(c)1 の rate ごとの fill と，(c)3・(c)4 の最小の `lead_left_ns`．
- stop：T2 の FAIL，装置の詰まり → 電源を切って入れ直す．(c)1 が INCONCLUSIVE でも (c)2 から先は続けてよい（記録する）．

### (d) 送信 rate の受け入れ：`hw_n47_raw_tx_rate_on_an_open_streamer`（#2 UBX → #1 OBX，TX あり，約 20 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47_raw_tx_rate_on_an_open_streamer   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47_raw_tx_rate_on_an_open_streamer   # 盤を替えて #1 OBX
```

- 1 行の VERDICT（1 → 2 MS/s）．PASS の条件：A の末尾の 1 ms が A の tone，A の burst ACK が A の排他的な終わり e の窓（e − 2 sample から e + 1 tick），B の onset が「B の tick＋control で測った遅れ（2 MS/s の control の遅れ）」の 1 000 tick 以内（誤差の許容は変えない），B の最初の窓が 100 kHz，`Applied.rate == 2 MS/s`，TIME_ERROR と underflow なし，A と B の窓にそれぞれ ACK が 1 つだけ，`tx_open` が 1 回．
- **B の onset が 1 000 tick を外れると INCONCLUSIVE**（delta を印字する．2026-11-06 の T3 で，#1 の 2026-11-05 の +3 200 tick は INFERRED の DUC の立ち上がりであり，許容は広げない）．
- 両盤で **PASS** のときだけ，送信 rate の受け入れ条件（note 47 §1・段階 4）が閉じる．FAIL は止めて owner へ．

### (e) TX200：200 MS/s の送信（#2 UBX，slot A の 1 channel，約 15 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．TX200 は PN（200 MS/s，帯域が広い）を送り，`SAFE_TX_HZ` の範囲に収まらない．**owner が att と広帯域の送信を確認してから流す**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0          # #2 UBX slot A（1 channel）
EZSDR_TX200_LENGTHS=1000,20000 EZSDR_TX200_SECONDS=300 hw hw_tx200_hot_path_probe
```

- 期待：長さ（1 000，20 000）ごとに 1 行の `TX200 len=… shape=single rate=200000000 …` と VERDICT．PASS は class Success，送信 sample が受理された，async の失敗なし，underflow なし，**供給側（Module の feed）の drop が 0**．`link_drops` が読めない場合は INCONCLUSIVE ではなく FAIL（claim は「device への送信」まで）．
- 2026-10-29 TXS の記録（1 000 と 20 000 の両方が 300 s を通る）と比べる．
- 1 試行でも FAIL が出たら，その行（長さ・class・underflow）を残して owner へ．

### (f) 200 MS/s の受信・送信・両方：T4 の負荷試験（#2 → #1，TX あり（200 MS/s の PN），約 70 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）．owner は広帯域（200 MS/s の PN）の送信も確認する**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0          # #2 UBX slot A
hw hw_iii_load_200msps
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1          # #1 OBX slot A（#2 が止まったら流さない．流すかは owner が決める）
hw hw_iii_load_200msps
```

- 既定は 600 s の stage が三つ（`rx`，`tx`，`duplex`）．短く試すなら `EZSDR_LOAD200_SECONDS=60` を `hw` の前に export する．`EZSDR_LOAD200_STAGES=rx` のように部分集合を選べる．
- stage ごとに 1 行の `T4 <stage> at 200 MS/s for <秒> s: class …; streams …; rx_overflow …; tx_underflow_events …; link_drops …; nic …; left_open …` と，その VERDICT．PASS は class Success，受信の overflow なし，送信の underflow なし，async の失敗なし，**link の drop が全部 0**．link の drop が読めない（null）ときは FAIL．
- 結果の範囲：link の drop が 0 なら「device から host まで」の claim．Module の feed までの drop の数は `link_drops` が示す．NIC の byte と counter の差は `nic` の欄（200 MS/s は片方向 800 MB/s で，10 GbE の全二重の一方向の量）．
- stage が成功しなかっても，次の stage は流れる．ただし `run_stage` が Err を返す，streamer が開いたまま終わる，または handle が latch した（`GuardedDevice::latched`）ときは，その盤の以降は流れない（電源を切って入れ直す）．
- stop：`Control operation timed out…`，streamer が開いたまま，handle の latch，装置の詰まり → 電源を切って入れ直し，owner へ．#2 の stop で #1 を流さない（owner が決める）．

### (g) 送信中の ssh 切断（#1 OBX，TX あり，約 8 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**

前提：`ssh localhost` が鍵認証（`BatchMode`）で通ること．独立の受信機を owner に聞く（スペクトラムアナライザ，または #1 の TX を別の受信に配線）．**無いなら `EZSDR_DROP_RX_CHECK` を置かずに流す**：その場合 C2 は NOT DONE と記録され，(g) は閉じない（C1 と C3 の結果は記録する）．

```sh
cd <COMMIT の tree>
EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
EZSDR_DROP_RX_CHECK='<独立の受信機に tone か quiet を最後の行に出すコマンド（owner が決める）>' \
  plan/phase8/tools/ssh-drop-tx.sh <COMMIT> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/g-ssh-drop.console.log
```

- Session は本番の経路：`ssh-drop-driver.py` が組む profile（UHD Module 0.4.0，profile x3xx 0.1.0，slot A の OBX，rx0 の feed，Attached の Lease（server の既定））で `ezsdr.connect()` し，`EZSDR_SERVER` の ssh wrapper 越しに `ezsdr-v4-dev:uhd4.10` の中の `ezsdr-server` を起動する．送信の channel は **connect の時**に組に入る（`.stream` を Run の中で変えない）．100 kHz の tone を 1 GHz・振幅 0.4 で repeat する．
- 順序：`tx-started` の後 5 s で local の ssh client を PID で殺す（wrapper が書いた pidfile）．drop の前に受信機を読めなければ（失敗・timeout），drop せずに止まる．drop の後は server の終了と受信機の quiet を，drop から 15 s の一つの期限で待つ（受信機の 1 回の読みは 10 s まで）．C1 と C2 の VERDICT を先に出し，**その後で**まだ動いている server を止める（後始末であって判定には使わない）．C3 は受信機の最後の読みが tone でなければ走る（読めないときも走る．その時点で送信は止まっている）．
- 確認（3 行の VERDICT）：C1 server が終了し，Manifest に `client_disconnect`（attached の Lease）．C2 期限より前に受信機が quiet を読めば PASS．期限まで tone が残れば FAIL．期限の後に初めて quiet を読んでも FAIL．読めなければ INCONCLUSIVE．受信機が無ければ NOT DONE．C3 `hw_noise_floor_receive`（同じ device を新しい Session で開き直し，coherent tone が 1 % 未満）．**exit 0 は C1・C2・C3 がすべて PASS のときだけ**．
- 注意：tone が止まったことは，clean な end of burst と FPGA の飢えを区別しない．RF の出力が 0 であることも示さない（LO の漏れが残りうる）．
- C1 FAIL（15 s 後も server が動いていた）は owner へ．後始末の後も `docker ps` に `ezsdr-drop-` の container が残るなら owner へ．

### 閉じ方と記録

- **#78**：(b) の全試験が両盤で PASS，想定外の `radio.TIME_ERROR` 0，`radio.TX_UNDERFLOW` 0．
- **#70**：(b) の `hw_p70_…` が両盤で PASS．
- **受信の rate の受け入れ条件**（note 47 §1・§4 段階 3）：(c)1 の T2 の rate ごとの VERDICT が両盤で PASS（rate の行を全部）と，(c)2 の 4 群が両盤で PASS．
- **送信の rate の受け入れ条件**（段階 4）：(d) が両盤で PASS．
- **200 MS/s の受信・送信・両方**：(f) の stage の VERDICT が両盤で PASS（`link_drops` と NIC の欄を残す）．(e) の TX200 は 2026-10-29 TXS の結論の再確認．
- **ssh 切断**：(g) の C1・C2・C3 が PASS．受信機が無ければ NOT DONE（閉じない）．

### 時間の合計（estimate）

| ブロック | 内容 | 機材 | 時間 |
|---|---|---|---|
| (a) | host と装置の確認（TX なし） | #2 → #1 | 約 10 分 |
| (b) | #78 と #70（T1 の 10 試験） | #2 UBX，#1 OBX | 約 30 分 |
| (c) | 受信の restart（T2，start lead，raw の restart） | #2 UBX，#1 OBX | 約 40 分 |
| (d) | 送信 rate（T3） | #2 UBX，#1 OBX | 約 20 分 |
| (e) | TX200（2 長さ × 300 s） | #2 UBX | 約 15 分 |
| (f) | T4 の負荷試験（3 stage × 600 s，2 盤） | #2 UBX，#1 OBX | 約 70 分 |
| (g) | ssh 切断 | #1 OBX | 約 8 分 |
| **合計** | 盤の交換と電源 cycle，初回の build を含まない | | **約 193 分（約 3 時間 15 分）** |

### 流した後

- orchestrator に返すもの：commit，各ブロックの VERDICT 行（log の path），(b) の表の TIME_ERROR の数（case ごと），(c)1 の rate ごとの fill（最小・中央値・最大）と T2 の VERDICT と `III fill observation` の行，(c)3・(c)4 の最小の `lead_left_ns`，(d) の 1 行と ACK の e からの差，(e) の行，(f) の stage ごとの行（`link_drops`，NIC の byte と counter），(g) の 3 行と使った受信機（または NOT DONE），電源 cycle の有無．
- repo へ写すのは，owner が確認したものだけ．log は `~/.cache/ezsdr-bench/<日付>/` に残す．
