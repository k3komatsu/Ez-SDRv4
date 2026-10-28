# Phase 8 — 実機 bench 手順

Phase 8 の BP0〜BP5 を実行する手順．shake-out の観測は診断用で profile の値には使わない．完了した BP2/BP3 の測定記録は段階4の profile 判断の入力になり，`measurements/` に置いた記録を比較テストが読む（仕様 [41 §2, §4](41-measurement-records.md)）．実機で失敗しても bench 中にコードを直さず，log と `.partial.json` を保存して次の修正段階へ戻る．

## 機材と RF 安全

- 対象は X300 #1（serial `347D545`，slot A に OBX，`addr=192.168.40.36`）と #2（serial `347D541`，slot A に UBX 等，`addr=192.168.40.37`）．NIC はそれぞれ `enp1s0f0np0`・`enp1s0f1np1`．実際の配線と serial を probe で確認する．同時接続は同じ `/24` の経路が重なるため，必要な `/32` route を owner が用意していない限り一台ずつ使う．
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

必ず `hw_measurements_record` を実行する．任意の `hw_*` test はその前の preflight として実行され，失敗したら record suite は開始しない．8つの必須 metadata は，output（自動で `/bench/measurement.json`），mboard と FPGA image（probe から。probe できない値は `EZSDR_MEASUREMENT_MBOARD` / `EZSDR_MEASUREMENT_FPGA_IMAGE` で指定），host name（既定は `hostname -s`），settings script / version（bench-host の path と出力），load（既定 `none`，`none` または `h4`），commit（snapshot の完全 hash）で組み立てる．`EZSDR_UHD_ARGS` には指定した UHD args を渡す．試行回数等は `EZSDR_MEASURE_STEP_TRIALS`，`EZSDR_MEASURE_OVERFLOW_TRIALS`，`EZSDR_MEASURE_THROUGHPUT_SECS`，`EZSDR_MEASURE_LOOPBACK_TRIALS`，`EZSDR_MEASURE_QUEUE_TRIALS` で調整できる（既定値は measurement code を参照）．queue depth では `EZSDR_MEASURE_QUEUE_TRIALS`（既定 5）は条件ごとに要る**成功**の数で，成功がそろうまで条件ごとに最大 `EZSDR_MEASURE_QUEUE_MAX_TRIALS`（既定 15，`EZSDR_MEASURE_QUEUE_TRIALS` 以上）回まで試行を続ける（段階 3f）．あいまい・不完全の試行は成功に数えない．15 回でそろわなければ queue depth は `unmeasured` で，理由に条件ごとの成功の数が書かれる．

出力には `build.log`，`probe.log`，preflight test ごとの log，`hw_measurements_record.log`，および `measurement.json` がある．3c-2c の途中経過 `<OUTPUT>.partial.json` は診断専用で，比較テストは読まない．各 group 後に上書きされ，final record が書けた後に削除される．中断時に残った partial は削除・改名せず log と一緒に保管する（再開可能な record ではない）．3c-3b の reopen は既定計画で追加約9〜10分（RX throughput 7段を各約30秒，RX/TX lead は step trials を半分にして再測；INFERRED）．この追加を含む目安は，#1＋OBX（1 channel）で約60〜75分，#2 の2 channel（UBX＋CBX）で約1時間35分〜1時間55分（INFERRED）．`h4` run は記録 suite 全体を負荷下で行うため，wall time は負荷なしの目安を越えることがある．H4 自体は200 updatesを10 ms間隔で流す約2秒の窓に加え，Run の準備・停止がある（INFERRED）．内訳（基準 suite，分）：raw probe 8，command lead 0.7，in-flight window 0.8，receive-start lead 8.5，transmit-start lead 1.6，受信 throughput 7.5（2 channel では 26），overflow 6，送信 throughput 7.5（同 26），full duplex 1.3，companion 11，raw group 2〜7（queue depth の試行は 1 回約 3〜4 s で，条件 3 つ．成功が早くそろえば 15 試行ほど，どの条件も上限まで行けば 45 試行．INFERRED），同時運転の 1 組 1．SIGKILL や host failure では trap が動かず worker が残り得るため，その場合は `h4-load.log` と記録された PID を確認してから次の run に進む．UR-7 の再試行は 1 回で約 31 秒，PLL が戻らなければ record の latch が止める（下）．log はファイルに書いてから必要箇所を確認する．

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
| serial / addr | 347D545 / `addr=192.168.40.36` | 347D541 / `addr=192.168.40.37` |
| NIC | `enp1s0f0np0` | `enp1s0f1np1` |
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
- 内容：3c-1（start lead，overflow gap，tx throughput），3c-2a（queue depth［queue の上限 512 回，試行ごとに目標までの遅れを 3 s と 2 s で交互に］，coercion），3c-2c（周波数の格子），3c-2b，既定の 1 channel の throughput，同時運転の 1 組，companion．
- 試行回数の既定（measurement.rs）：STEP 100，OVERFLOW 5，THROUGHPUT_SECS 60，LOOPBACK 20，QUEUE 5．変えるなら `EZSDR_MEASURE_STEP_TRIALS` 等を前置．
- BP5 を先に見たいとき（任意）：末尾に `hw_b3_receive_at_t0 hw_b4_capture_at_a_sample_index hw_b5_overflow hw_b6_txrx_and_repeat hw_b7_session_loopback` を足す（preflight．1 本でも失敗なら record は始まらない）．
- 確認（run directory）：`build.log`，`probe.log`，`bench-host.txt`，`hw_b1_probe.log`，`hw_measurements_record.log`，`measurement.json`（profile 比較の前に書かれるので，比較で落ちても残る）．`measurement.json.partial.json` が残っていたら final は書けていない．最後の profile 比較で落ちることがある（値が profile の 1000 MB/s に届かない）．これは段階 4 の材料で，bench 中に直さない．
- 見る点：queue depth（`raw_command_queue_depth` の `ambiguous_trials`，`blocked_index_across_ahead.agree`），coercion の格子，TX throughput の Underflow 位置，`provider_duplex_at_the_measured_limits`（`verdict`，`overlap`），overflow の「読み直しから再開まで」の最大と最小．
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

- **B0 低い rate（窓が律速）**：1 MS/s と 5 MS/s，repeat の waveform **31 000 sample**，60 s．**1 回ごとの send の大きさは Manifest に無い**（`bursts[].blocks` は block の数 u32 で，列ではない：`kernel stream/burst.rs`，`provider/tx.rs` の `record`）。大きさの分類は別の**有限の診断 run**（1 MS/s と 5 MS/s で各約 5 s）で，専用の `#[ignore]` 試験 `hw_txp_b0_send_sizes`（`tests/hardware.rs`）が device を `ProbeDevice` で包んで走らせ，block 2 000 で 31 000 sample の repeat（振幅 ≤ 0.5，gain 0 dB）の `tx_send` の行の `sample_counts[0]` を空の end-of-burst（0）を除いて `B0 sizes rate <hz>: …` と印字する。**送信するので先に owner が att を確認してから**，(c) の probe と同じ形で：

```sh
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_txp_b0_send_sizes -- --exact --ignored --nocapture \
  > ~/.cache/ezsdr-bench/$(date +%F)/hw_txp_b0_send_sizes.log 2>&1
```
その列を累積 offset で分類する：(i) 最初の block（SOB）は ≤ chunk（2 000）；(ii) 累積が 31 000 の倍数になる block（wrap で終わる）は ≤ `min(chunk, 31 000 − 直前までの offset)`；(iii) それ以外の block は chunk に等しい．(iii) に違反があれば失敗．blocks/s は 60 s の run の Manifest の `bursts[].blocks` の総数（block 数）を 60 で割る（production に send ごとの保存は足さない）．基準の run（`d0a0e64` の build，同じ設定）と比べ，uhd-tx の `blocks`／s が `rate / chunk × 1.1` 以下に減り，uhd-tx の CPU 時間（`/proc/<tid>/stat` の utime + stime を 60 s で割る）と自発的でない context switch（`/proc/<tid>/status`）が基準より減ることを**記録**する（割合の閾値は INFERRED で gate にしない）．underrun 0．
- **B1 100 MS/s**・**B2 200 MS/s**：60 s，`--cap-add=SYS_NICE --ulimit rtprio=99` つき．`tx_realtime` = owner 数，uhd-tx の policy = 2．underrun を `benchmark_rate --priority high`（(d) の記録）と比べる．
- **B3 同時運転**：送信 100 MS/s + 受信 + timed の control 更新（frequency／gain）+ burst 途中の Stop．uhd-rx の overflow 0，timed 命令の遅れが release 窓内，Stop が 1 s 以内に完了．uhd-rx／uhd-control の runnable 待ちを `/proc/<tid>/schedstat` で記録（実時間の uhd-tx が同じ core の通常 thread を飢えさせる兆候）．
- **B4 cap 無し**：`--ulimit rtprio=0` で cap なし（と host の通常 user）：`rejected` に `set_thread_priority` の行が 1 本出て run が完走し，DEVICE_LOST にならない．

結果は bench-results.md へ（orchestrator）．KL-5（50 MS/s を超える送信の失敗）の書き換えは B1〜B3 の結果を見てから．

### 流した後

- repo へ写すのは**完了した最終の BP2/BP3 の `measurement.json` だけ**：`crates/ezsdr-radio/measurements/<日付>-<serial>-<構成>.json`（41 §2）．shake-out，`.partial.json`，P4〜P7・MA-1・benchmark_rate の log は写さない．写す前に owner と確認．
- orchestrator に返すもの：commit hash，run directory の path，各ブロックの結果（完走 / 止まった場所と最後の log 行），`probe.log`（FPGA，serial）と `ref_locked`，att の型番・電力定格，電源 cycle の有無，P4〜P7 と MA-1 の出力，benchmark_rate の summary，`measurement.json` の観測の要約（overflow gap の最大・最小，queue depth と `ambiguous_trials`，coercion の格子，throughput の最大成功段，同時運転の 1 組の verdict）．bench-results.md への追記は orchestrator．
- 結果から出る owner の決め：overflow_restart_gap_ns と RM-17（最大と最小の差が最大の半分以下ならその最大を値に）；R4 の採否（P6）；R2 の要否（P4）；F7（P5）；MA-1（MockRadio の MR-34 をそろえる段階の前に，位相のばらつきの 2 通りの差）；受信・送信の throughput の値（profile 1000 MB/s）は 2 channel の結果を待って段階 4 で決める．
