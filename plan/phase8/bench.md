# Phase 8 — 実機 bench 手順

Phase 8 の BP0〜BP5 を実行する手順．shake-out の観測は診断用で profile の値には使わない．完了した BP2/BP3 の測定記録は段階4の profile 判断の入力になり，`measurements/` に置いた記録を比較テストが読む（仕様 [41 §2, §4](41-measurement-records.md)）．実機で失敗しても bench 中にコードを直さず，log と `.partial.json` を保存して次の修正段階へ戻る．

## 機材と RF 安全

- 対象は X300 #1（serial `347D545`，slot A に OBX，`addr=192.168.40.36`）と #2（serial `347D541`，slot A に UBX 等，`addr=192.168.47.37`）．NIC はそれぞれ `enp1s0f1np1`・`enp1s0f0np0`．実際の配線と serial を probe で確認する．同時接続は同じ `/24` の経路が重なるため，必要な `/32` route を owner が用意していない限り一台ずつ使う．
- 送信する全ての段で，該当 board の **TX/RX → 30 dB 以上・18 GHz 定格の attenuator → 同じ board の RX2** を接続する．att は最大 TX 出力に対して十分な最大入力電力定格と余裕があることを銘板・仕様で確認し，型番と電力定格を記録する．アンテナはつながない．この日の装置では 40 dB attenuator を使用した．gain 0 dB，波形振幅 0.5 以下，通常の loopback は 1 GHz を使う．
- coercion の TX gain sweep は **streamer を開かず，LO を 1 GHz にした状態で最大 gain まで**試す．ただし CBX の下限は 1.2 GHz なので，CBX では「1 GHz」は 1.1〜1.2 GHz に clip される（UHD は error を出さない．#1 の未知の board の cleanup が 0.0 Hz を読んだ例が shake-out 4 にある）．coercion の TX frequency read-back は **全周波数範囲の格子**（OBX 10 MHz–8.4 GHz，UBX 10 MHz–6 GHz，CBX 1.2–6 GHz．gain 0，sample を送らない）を読む（`SAFE_TX_HZ` の範囲ではない）．古い試験（`pn`）が送る波形の振幅は 0.566 までで，0.5 以下なのは 3c-3a 以降の送信（`transmit_waveform`）．いずれの送信条件も上記 attenuator が必要で，RF safety の例外ではない（仕様 41 §8）．
- 送信の中心周波数を縛るのは bench の profile の `radio.rf_envelope`，送信 channel ごとの ±1 MHz の狭い band（1 GHz なら 999–1001 MHz）である．RM-19 が見るのは中心周波数だけで，この band は誤った retune に対する自動の防護になる（bench の retune は最大 ±400 kHz）．sample rate は band では制限されず，cabled loopback 上で **200 MS/s まで**使ってよい（owner 決定，2026-11-10；raw probe の上限は `SAFE_TX_MAX_RATE`）．安全は配線で担保する：30 dB 以上の attenuator（同軸と attenuator は 18 GHz 定格），gain 0 dB，振幅 0.5 以下，送信の前に owner が attenuator の接続を確認すること．`SAFE_TX_HZ`（中心 999.5–1000.5 MHz）は変えない．B7 の拒否確認は，Rust 版が 1.5 GHz，Python 版（`python/examples/bench_loopback.py:40`）が 2.4 GHz への retune で，どちらも全ての狭い band の外にある（Rust 版は実行前にそれを確認する）．
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

- #1 から #2 へ替えるとき，旧 NIC に `192.168.40.10/24` が残っていたら `sudo ip addr del 192.168.40.10/24 dev enp1s0f0np0` で外してから #2 の NIC を check/apply（BP3）．
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

- 機材の順は **#2（(a)・(b)・(c)・(e)）→ 盤を替えて #1（(a)・(b)・(c)・(d)・(f)）**．ブロックの番号は見出しの順で，実行は上の機材の順．
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

- 順は **#2 → #1**．ブロック (g) の T4（受信・送信）・T5 と (h) の T3 は **両方の盤**で流す（T6 は #2 のみ）．T4 の受信は送信を伴う（同じ盤の送信を 1 MS/s の tone 源にする）．
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
- 順は **#2 → #1**．
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

## <DATE> の手順（note 47 段階 5 の後）

`<COMMIT>` は段階 5 の試験（`hardware.rs` の「note 47 stage 5」の節，`common/n47s5.rs`，`common/iii.rs`）が入った commit．結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．出典：この文書の上の 2026-11-06 の節，[47 の段階 5](47-no-change-while-streaming.md) §4 の 5．[test plan](reviews/47s5-test-plan.md) §10 Q-4・§11 R5-m5，[design/07](../../design/07-radio-model.md) RM-15・RM-16・RM-37，[design/18](../../design/18-uhd-radio.md) UR-21・UR-22・UR-23・UR-38，`crates/ezsdr-radio/src/staging.rs`．

### 構成と順番

| | 機材 | 接続 | att |
|---|---|---|---|
| #1 | X300＋OBX（slot A） | `addr=192.168.40.36`，NIC `enp1s0f1np1`，`--subnet 40`，`BOARD=36` | 30 dB |
| #2 | X300＋UBX（slot A）・CBX（slot B） | `addr=192.168.47.37`，NIC `enp1s0f0np0`，`--subnet 47`，`BOARD=37` | 各 30 dB |

- att は三つの board とも 30 dB（2026-11-06 と同じ）．
- 二台は同時につないでよい（NIC ごとに subnet 一つ．owner 2026-11-08）．順は **#2 → #1**．(i) の ssh 切断は二台を同時に使う（#1 が送信，#2 が独立の受信機）．
- 試験はすべて slot A（OBX／UBX）．CBX（slot B）は流さない（`iii::pin_to_slot_a` が，試験ごとに slot A だけの layout を組む）．
- 基準は内部（`internal`）．PPS と 10 MHz は使わない．
- 時間は各ブロックの estimate．初回の build（volume が冷たい）は数分から十数分の追加（estimate）．

### 共通の準備（送信しない）

```sh
cd <COMMIT の tree>                      # main tree で commit を checkout した状態．git status が clean であること
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
hw() { # $1 = 試験名．EZSDR_UHD_ARGS と BOARD（36 か 37）と EZSDR_BENCH_NIC は呼ぶ前に機材ごとに export する．失敗なら非 0 を返す
  local dir=~/.cache/ezsdr-bench/$(date +%F) log status=0 n=1
  log="$dir/$1.$BOARD.log"
  while [[ -e $log ]]; do n=$((n + 1)); log="$dir/$1.$BOARD.$n.log"; done # 既存の log は上書きしない（.2，.3）
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_BENCH_NIC -e EZSDR_SLOT_B_ATTENUATOR_CONFIRMED -e EZSDR_OFS_TWO_CHANNEL_WAIVED \
    ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$1" -- --exact --ignored --nocapture --test-threads=1 \
    >"$log" 2>&1 || status=$?
  grep -h 'VERDICT\|test result\|panicked' "$log"
  ((status == 0)) && [[ $(grep -c '^test result:' "$log") == 1 ]] && grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"
}
run_all() { for t in "$@"; do hw "$t" || return 1; done; } # 最初の失敗で止まる
```

- log は `~/.cache/ezsdr-bench/<日付>/<試験名>.<BOARD>.log` に残る．同じ試験を同じ盤でもう一度流すと `.2.log`，`.3.log` になる．
- `hw` は docker／cargo の終了 status と，`test result:` の行がちょうど 1 つで `ok. 1 passed; 0 failed; 0 ignored;` であることの両方を見る．**`VERDICT` は読む．** 多くの試験は FAIL で panic して `hw` が非 0 を返すが，`hw_b8_stop_end`（(g)）は段階 5 から FAIL の試行で assert する．INCONCLUSIVE の行は閉じる根拠に数えない．
- この節の試験が読む環境変数は `EZSDR_UHD_ARGS`（盤）だけ．`hw` は 2026-11-06 と同じく `EZSDR_BENCH_NIC`（盤の NIC：#2 `enp1s0f0np0`，#1 `enp1s0f1np1`）も渡す．読むのは NIC の counter を取る試験（`iii::bench_nics`）で，この節にはない．計測の record（`EZSDR_MEASUREMENT_*`，`EZSDR_MEASURE_*`）は使わない．
- 2026-11-06 の節の (b) にあった試験のうち，段階 5c で削除されたもの（先行 burst の preemption の掃引）は今日の節にない（test plan Q-3）．
- `push_raw_observation` の結果は計測の record（`EZSDR_MEASUREMENT_*`）を指定したときだけファイルになる．この節は指定しない．raw の表は試験が出す行（`T2 raw …`）から読む．

stop の規則：`Control operation timed out waiting for space in command buffer`，`Reference Clock PLL failed to lock`，装置が応答しない → **電源を切って入れ直す**．panic を読んで owner に報告してから再開する．handle の latch（control timeout・device 喪失）が出た盤の以降の試行は流れない（`VERDICT` の行に「power-cycle the board」と出る）→ 電源を入れ直し，owner に伝える．

### (a) ホスト確認と装置確認（送信なし，#2 → #1，約 10 分，estimate）

PC の再起動の後は，まず owner が `sudo bash ~/ezsdr-bench/usrp-net.sh`（tree の外．NIC ごとに subnet を一つだけ付け，各 X300 が自分の NIC で答えるかを確かめる．ケーブルが逆なら `CABLES SWAPPED` で exit 1）を流す．

```sh
ping -c2 -W1 -I enp1s0f0np0 192.168.47.37 && ping -c2 -W1 -I enp1s0f1np1 192.168.40.36   # 各 X300 が自分の NIC で答える．答えなければ止める（ケーブルの取り違え）
plan/phase8/tools/bench-host.sh --nic enp1s0f0np0 --subnet 47            # #2．read-only．不足があれば exit 1
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.47.37
# 盤を替えて #1
plan/phase8/tools/bench-host.sh --nic enp1s0f1np1 --subnet 40
docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36
# NIC の counter が読めるか
cat /sys/class/net/enp1s0f0np0/statistics/rx_bytes /sys/class/net/enp1s0f1np1/statistics/rx_bytes
```

確認：serial（#2 は 347D541，#1 は 347D545），FPGA 39.3，slot A の board（UBX／OBX），master clock 200 MHz，`ref_locked` が true，10 GbE link．stop：NIC の不足が残る，find されない，`ref_locked` が false → 止めて owner へ．

### (b) 回帰：T1 の 9 試験（#2 UBX → #1 OBX，TX あり，約 40 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && run_all \
  hw_s1_short_first_sends hw_p70_one_sample_burst_then_timed_burst hw_p72_timed_tx_chunks_are_timed_and_flushed \
  hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_h5_held_last_sample hw_txp_b0_send_sizes                                           # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && run_all \
  hw_s1_short_first_sends hw_p70_one_sample_burst_then_timed_burst hw_p72_timed_tx_chunks_are_timed_and_flushed \
  hw_b6_txrx_and_repeat hw_b7_session_loopback hw_b8_burst_at_a_sent_burst_s_end hw_b8_burst_one_sample_after_a_burst \
  hw_h5_held_last_sample hw_txp_b0_send_sizes                                           # 盤を替えて #1 OBX（#2 が止まったら流さない）
```

各試験の VERDICT と，その行が出す TIME_ERROR と TX_UNDERFLOW の数（`iii::faults_verdict`，`hw_b8_burst_one_sample_after_a_burst` は `iii::late_control_verdict` と `iii::refusal_verdict`）．期待は次のとおり．それ以外の `radio.TIME_ERROR` は**想定外**．

| 試験 | 期待する `radio.TIME_ERROR` | TX_UNDERFLOW | 根拠 |
|---|---|---|---|
| `hw_s1_short_first_sends`（1，2，3，4 サンプル） | 0（case ごとに `BurstAck` が 1 つ） | 0 | UR-22（note 48 S1） |
| `hw_p70_…`（1 サンプルの burst と 4 サンプルの対照，booking の 2 通り） | 0（両方） | 0 | UR-22，UR-23 |
| `hw_p72_…` | 0 | 0 | UR-22，UR-28 |
| `hw_b6_txrx_and_repeat`（burst と repeat） | 0 | 0 | UR-22，UR-23 |
| `hw_b7_session_loopback` | 0 | 0 | UR-22 |
| `hw_b8_burst_at_a_sent_burst_s_end`（3 試行） | 0 | 0 | UR-23（B3） |
| `hw_b8_burst_one_sample_after_a_burst`（end + 1，end + 2，各 2 通り） | end + 1 は **0 で，booking は receipt で refuse される**（`radio.COMMAND_REJECTED` の理由 「UR-21: the burst overlaps an accepted booking」）．end + 2 は 0 で流れる | 0 | RM-15，staging.rs の `front`（長い burst の end の後の front は end + 2）．2026-11-06 では end + 1 が 1 の late だった |
| `hw_h5_held_last_sample`（隣接・3 000 sample の間隔・締切の run） | 0 | 0 | UR-22 |
| `hw_txp_b0_send_sizes`（1 と 5 MS/s） | 0 | 0 | UR-22，TXP |

- 「booking は refuse される」は 2026-11-06 と違う期待．refusal は `radio.COMMAND_REJECTED` のうち，その booking の Action を名指すものから判定する（`n47s5::refusals_of`）．それがなければ end + 1 の VERDICT は FAIL（`iii::refusal_verdict`）．
- **#78 と #70 は 2026-11-06 に閉じた**（handoff §1）．この節の (b) の FAIL は回帰である：止めて owner へ報告する．issue を開き直すのは orchestrator．この節は閉じる条件を持たない．
- stop：想定外の `radio.TIME_ERROR` や `radio.TX_UNDERFLOW`，装置の詰まり → 電源を切って入れ直す（同じ盤で次の試験を流さない）．

### (c) 送信の前線：T2（#2 UBX → #1 OBX，TX あり，約 25 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && run_all \
  hw_n47s5_front_after_a_short_burst hw_n47s5_raw_front_gaps                             # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && run_all \
  hw_n47s5_front_after_a_short_burst hw_n47s5_raw_front_gaps                             # 盤を替えて #1 OBX
```

- (c)1 `hw_n47s5_front_after_a_short_burst`：n ∈ {1, 2, 3, 4, 10, 32} × 1 MS/s と 2 MS/s，12 行の VERDICT（行の text は「T2 front after a N-sample head at R S/s, booked at +K samples」）．K は `iii::head_front`（n ≤ 3 は 5，4 ≤ n ≤ 32 は n + 1）．PASS：admitted，TIME_ERROR と underflow なし，BurstAck が 3 つ（calibration，head，tone），tone の onset が booking から，その試行の遅れ（A の最初の burst の onset，S5）± 50 sample（S7）．INCONCLUSIVE：capture に onset がない，遅れが測れない，または BurstAck が 3 つでない．FAIL：refuse，TIME_ERROR，underflow，onset が許容の外．
- (c)2 `hw_n47s5_raw_front_gaps`：観測だけ（VERDICT の行はない）．2 rate × 6 n × 6 gap（0〜5）× 4 試行 = 288 行．開始時の flush の ACK は `raw_tx_ready` が別に印字し，各行の codes には入らない．各行は `T2 raw, <rate> S/s, <n> samples (transmitted <padded>), gap <g> from the device end: complete|incomplete`，late の数，accepted の数，codes．gap は device の排他的な終わりからの距離（一つ後の sample が gap 1，record の front と同じ）．
- stop：(c)1 の FAIL，TIME_ERROR，装置の詰まり → 電源を切って入れ直す．(c)2 が途中で止まったら owner へ．

### (d) 保留の尾と Stop の領域：T3（#2 UBX → #1 OBX，TX あり，約 20 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47s5_held_tail_and_stop   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47s5_held_tail_and_stop   # 盤を替えて #1 OBX
```

6 行の VERDICT（各盤）．

1. (f) C を e に，J(e) の前に booking：PASS は device の burst が 1 つ，end-of-burst はその終わり（C の末尾）に 1 つだけ（e には送らない），ACK も 1 つ，onset が，その試行の遅れ（A の最初の burst の onset，S5）± 50 sample．
2. (f) C を e + 2 に，J(e) の後に booking（J(e) = e − 5 ms）：submit は J(e) の後の最初の wait の step．PASS は device の burst が 2 つ（A の保留の尾が J(e) で閉じ，C は別の burst），EOB と ACK が各 2 つ，C の onset が，その試行の遅れ ± 50 sample．C が遅れて届いた（下の 3 の判定）なら INCONCLUSIVE．その場合，同じ盤の 3 が PASS（時刻どおりに届いて refuse）なら，遅れは owner の dispatch の疑いがある：流し直さず owner へ報告（owner 不具合の疑い）．
3. (f) C を e + 1 に，J(e) の後に booking．submit は J(e) の後の最初の wait の step，表示は submit − J(e) と遅れの量（late_by）．判定は次の順（最初に当たったもので決まる，`iii::late_arrival_verdict`，`n47s5::held_refused_verdict`）：(i) `COMMAND_REJECTED` が C を名指す → 時刻どおりに届いて refuse．PASS（reason 「UR-21: the burst overlaps an accepted booking」），A だけが流れる（EOB と ACK が各 1 つ）．(ii) C が受理され TIME_ERROR がない → 時刻どおりに届いたのに refuse されなかった：FAIL．(iii) C を target とする late の drop（`TIME_ERROR`，cause late，outcome drop）があり，C を名指す refuse がない → 受信か dispatch で遅れた（同じ `TIME_ERROR`）：INCONCLUSIVE「late at receipt or at dispatch」（host latency か owner の dispatch の遅れ）．(iv) それ以外の `TIME_ERROR` → FAIL．e + 1 は，時刻どおりに届けば J(e) の前でも後でも refuse される．refuse の窓は (J(e), e + off·1 µs − 2 ms]（off は C の e からの offset，サンプル単位，1 MS/s で +1 sample；J(e) = e − 5 ms なので約 (e − 5 ms, e − 2 ms]；`join_by`，`decide`）．窓の外へ出ると late として drop される（(iii)）．INCONCLUSIVE は試験を 1 回だけ流し直す．もう一度 INCONCLUSIVE なら owner へ．FAIL は遅れを測ってから owner へ．
4. (b) Session（観察．判定される geometry は 5 の raw の close）：次の burst を Stop の後の最初の sample に（`staging::stop_sample`，RM-16；表示名は「Session: next burst at the record's stop sample」）．repeat の最初の burst は時刻つき（開始の 15 ms 後）で，capture はその target − 2 ms を覆う（方法は時刻つき，30 ms 待って Stop）．PASS は admitted，faults なし，onset が，repeat の遅れ（その試行の onset，S5）± 50 sample．overlap の refuse だけなら INCONCLUSIVE（「refused at the Kernel's stop sample」）：Provider の bound は Stop の受信から決まり，Manifest に記録がないので host latency を示せない（`iii::stop_booking_verdict`，S3）．overlap 以外の理由の refuse，二つ以上の refuse，admitted でない，faults，onset の外れは FAIL．
5. (b) raw：two zeros で閉じた burst（cursor c）の後，次の burst を c+3 と c+4 に（c+2 は印字だけ，各 4 試行）．PASS は c+3 と c+4 の全試行で ACK が 2 つ，late も underflow もない．FAIL は c+3 か c+4 で TIME_ERROR か underflow．
6. (h) R5-m5 の raw 対（1–3 sample の頭を c − 1 に，c + 3 へ詰め，c + 3 に booking）：PASS は全試行で ACK が 2 つ，late も underflow もない．**これが記録が違っているかもしれない唯一の場合**．FAIL なら止めて owner へ．すべて print する．

- 注意：(h) は Session では組めない（Stop を sample 単位で置けない）．raw と timed で device の形を再現する．記録が c + 3 の booking を受け入れること自体は単体試験（`iii.rs` `a_booking_at_the_stop_is_admitted_after_a_head_the_stop_cuts`）で確かめた．
- stop：FAIL（(f)，(b)，(h) とも），TIME_ERROR，underflow，装置の詰まり → 電源を切って入れ直す．

### (e) 反復の切り替え：T4（#2 UBX → #1 OBX，TX あり，約 15 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47s5_repeat_switch   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47s5_repeat_switch   # 盤を替えて #1 OBX
```

- 2 行の VERDICT（untimed，timed：未来の境界）．100 kHz の repeat（1 100 sample）から 200 kHz の repeat（1 100 sample）へ切り替える．capture は switch の前に始める（S5）．
- 試験は `switches` の行（境界，state）と `T4 switch: …` の行（行，境界，seams，device の sends）を出す．
- PASS：row が `applied`，境界 b の前 5 ms から b − 500 µs まで 100 kHz，b + 500 µs から後の 5 ms まで 200 kHz（±10 kHz），b をまたぐ power が定常の 1/4 を超える，device の sends に切り替えの EOB と SOB がない（S4），TIME_ERROR と underflow なし．
- INCONCLUSIVE：境界の両側に tone がない．FAIL：row が applied でない，tone が境界の外で変わる，power が落ちる，seams が 1 以上，faults．
- 注意：loop の遅れは試行ごとに最初の repeat の onset から求める（遅れが測れなければ INCONCLUSIVE，S5）．margin 500 µs は INFERRED（loop の遅れの散らばりと DUC の立ち上がり）．
- stop：FAIL，TIME_ERROR，underflow → 電源を切って入れ直す．

### (f) 未指定の送信：T5（#2 UBX → #1 OBX，TX あり，約 10 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47s5_untimed   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47s5_untimed   # 盤を替えて #1 OBX
```

2 行の VERDICT（各盤）．

1. (1) untimed の repeat，Stop，untimed の repeat．二つ目は admitted で，200 kHz の tone が capture に出る．PASS：admitted，tone あり，faults なし．INCONCLUSIVE：admitted だが tone がない．FAIL：refuse．
2. (2) timed の 30 000 sample の burst A が流れている間（A の開始から 5 ms 後）に，untimed の 1 000 sample の burst．PASS：admitted，A の front（A の開始 + 30 002 sample）以降に配置，onset が applied target から，A の遅れ（A の最初の burst の onset，S5）± 50 sample．FAIL：refuse，front より前に配置，TIME_ERROR，underflow．

- stop：FAIL，TIME_ERROR，underflow → 電源を切って入れ直す．

### (g) Stop の終わり：T6（#2 UBX → #1 OBX，TX あり，約 10 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_b8_stop_end   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_b8_stop_end   # 盤を替えて #1 OBX
```

- 3 試行，各 1 行の VERDICT．repeat は 100 kHz の 1 000 sample，時刻つき（提出の 15 ms 後．capture は提出の 5 ms 後に始まるので，最初の burst の target − 2 ms は capture に入る），Stop は capture の開始の 70 ms 後．loop が聞かなくなった点（`heard_until`）は，repeat の最後の 100-sample window の終わりから，その試行の遅れ（最初の burst の onset，S5）を引いた root tick．edge の探索は最初の burst の onset + 1 ms から（その前の window は使わない）．edge は，最初の 20 window の平均 power の 1/4 を超える最後の window で，その後に 5 window（0.5 ms）の静かな区間が capture 内にあり，定常の power が雑音の 4 倍を超えるときだけ（雑音だけ，capture が信号で終わる，edge の前に穴がある，のときは edge なし）．PASS：repeat が admitted で refuse されず，`heard_until` が，repeat の記録上の終わり（`bursts` の行の target と sample 数）から (100 + 50) sample 以内．FAIL：refuse，または (100 + 50) sample の外（`hw_b8_stop_end` は assert して test が失敗する）．INCONCLUSIVE：edge がない，遅れが測れない，または記録上の終わりがない．FakeDevice の rehearsal は INCONCLUSIVE になる（fake の受信は平均 power が tone より大きい ramp を返すため，edge が立たない；`device.rs`）．
- 印字だけで判定しない行：submit から沈黙までの時間，`stop_bound_text`（in-flight 10 ms と記録上の終わりの比較．`radio.timing.tx_in_flight_ns` は profile の値で，INFERRED）．
- `hw` の戻り値は 0 のままでも，VERDICT の FAIL を読む．
- stop：FAIL の行は owner へ．装置の詰まり → 電源を切って入れ直す．

### (h) 送信の rate：`hw_n47_raw_tx_rate_on_an_open_streamer`（#2 UBX → #1 OBX，TX あり，約 20 分，estimate）

**送信の前に owner に att を確認する（三つの board とも 30 dB）**

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47_raw_tx_rate_on_an_open_streamer   # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47_raw_tx_rate_on_an_open_streamer   # 盤を替えて #1 OBX
```

- 盤ごとに 1 行の VERDICT（1 → 2 MS/s）．期待する PASS：A の末尾 1 ms が A の tone（100 kHz），A の burst ACK が A の排他的な終わり e の窓（e − 2 sample から e + 1 tick），B の onset が「B の tick ＋ 2 MS/s の control で測った遅れ」と同じ窓に入る（下の分解能の注意），B の最初の窓が 100 kHz，`Applied.rate == 2 MS/s`，TIME_ERROR と underflow なし，A と B の窓にそれぞれ ACK が 1 つだけ，`tx_open` が 1 回．
- B の onset は control と同じ窓の位相に置く（`n47::same_window_phase`，2026-11-08）．11-06 の #2 の INCONCLUSIVE（B の onset が −1 200，−2 200，＋2 200 tick）は，control と B の窓の位相の違いから出たと読む（INFERRED．この修正の実機の結果はまだない）．
- 分解能は窓 1 つ（25 sample，5 000 tick）．onset は窓の始まりで読むので，delta は 0 か ±5 000 tick になる．**PASS は B の onset が control の予測と同じ窓に入ったことを示す．1 000 tick の精度は示さない**（INFERRED．細かく測るには sample ごとの onset が要り，それは別の仕組み）．許容 1 000 tick は変えない．
- delta が 0 でないときは INCONCLUSIVE（delta を印字する）．許容は広げない．FAIL は止めて owner へ．
- **両盤で PASS のときだけ，送信 rate の受け入れ条件（note 47 §1）が閉じる．** 閉じるのはこの条件だけ．送信の rate を受け入れる機能そのものは別の設計段階で決める（note 47 §4 の 4：設定を拒まれた clock の扱い，再試行が Provider の booking に届く道）．owner の時間と予約の棚卸しの後で，この機能が消えることもある．

### (i) 送信中の ssh 切断：`plan/phase8/tools/ssh-drop-tx.sh`（#1 OBX が送信，#2 UBX が独立の受信機，TX あり，約 12 分，estimate）

**送信の前に owner に配線と att を確認する**（att は三つの board とも 30 dB 以上）：#1 OBX の TX/RX → att（30 dB 以上）→ splitter．splitter の一方は #1 の RX2（C3 の loop はそのまま），もう一方は #2 UBX slot A の RX2．att は splitter の前に 1 個で，両方の受信経路を覆う（#1 の RX2 にも #2 の RX2 にも 30 dB 以上）．#2 は ssh を通さず，`rx-monitor.sh` が直接受信する．

前提（この順で確かめる）：

- `ssh -o BatchMode=yes localhost true` が成功する（鍵認証．sshd は port 10022 で，owner が `~/.ssh/config` の `Host localhost` に `Port 10022` を置く．script はポートを書かない）．失敗なら止める．
- 独立の受信機は #2 の monitor（手順 2）．monitor が起動しない（最初の 1 秒の window の半分以上が drop の tone の形なら起動を拒む），quiet を読めない，または最初の 1 秒に tone が見えるなら，(i) を流さない．流すなら C2 は NOT DONE と記録し，(i) は閉じない（C1 と C3 の結果は記録する）．

手順：

1. build を温める．`hw_rx_tone_monitor` は monitor として走る試験なので，ここでは `--no-run` で image の Cargo target volume に build だけ入れる．状態ファイルの規則は device を使わずに確かめる．

```sh
cd <COMMIT の tree>                      # git status が clean であること
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
  -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work -e CARGO_HOME=/cargo-target/.cargo-home \
  ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware --no-run
plan/phase8/tools/rx-monitor-state.sh --self-check
```

2. #2 の NIC を確認する（(a) と同じ．`bench-host.sh --nic enp1s0f0np0 --subnet 47` が通ること）．#1 は silent のまま monitor を起動する．`EZSDR_MONITOR_SECONDS=3600` は，server の build が長くかかっても drop の前に monitor が終わらないため（stop file で協調的に止まるので，長くても費用はない）．

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 EZSDR_MONITOR_SECONDS=3600
STATE=$(plan/phase8/tools/rx-monitor.sh start)   # 状態ファイルの path を出す．log は同じ場所の <state>.log
```

3. quiet を待つ（最大 60 s）．読めなければ 4 を流さず，5 で monitor を止めて log を読む．

```sh
for _ in $(seq 60); do [[ $(plan/phase8/tools/rx-monitor-state.sh "$STATE") == quiet ]] && break; sleep 1; done
```

4. ssh 切断を流す（#1 OBX．EZSDR_UHD_ARGS は #1 に戻して渡す．attenuator の確認は script が求める）．**直前に monitor が fresh な quiet を読んだときだけ走る**（`if`）．

```sh
if [[ $(plan/phase8/tools/rx-monitor-state.sh "$STATE") == quiet ]]; then
  EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
  EZSDR_DROP_RX_CHECK="$PWD/plan/phase8/tools/rx-monitor-state.sh $STATE" \
    plan/phase8/tools/ssh-drop-tx.sh <COMMIT> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/i-ssh-drop.console.log
else
  echo 'monitor not quiet: (i) not run'
fi
```

5. monitor を止める（ssh 切断の後．C3 は #1 を開き直すので，#2 の monitor とは衝突しない）．`plan/phase8/tools/rx-monitor.sh stop`．stop file を置き，monitor が receive を止めて閉じるのを最大 10 s 待つ．終わらなければ container を名指しで止め，その旨を出す（receive が clean に閉じていない）．

- 期待する 3 行の VERDICT：
  - `VERDICT C1 the server ends the Session when the link drops: PASS: …`（server が 15 s 以内に終了し，Manifest に `client_disconnect`，attached の Lease）．
  - `VERDICT C2 the transmission stops when the link drops: PASS: …`（drop の前に monitor が tone を読み，期限より前に quiet を読む．`rx-check.log` に `rx_before_drop=tone`）．
  - `VERDICT C3 a Session after the drop sees no tone: PASS: …`（`hw_noise_floor_receive`：drop の tone の形（`is_drop_tone_window`：coherence 0.9 超，100 kHz ± 20 kHz）で，平均電力が 4 LSB の振幅を超える window が 1 % 未満．行に最大の tone の形の window の振幅（LSB）を出す）．
  - C3 は Session の既定の受信 gain 0 dB に頼る（`receive_spec` は gain を固定しない．INFERRED）．
- exit 0 は C1・C2・C3 がすべて PASS のときだけ．
- INCONCLUSIVE：C1 は server の終了時刻が読めないとき，drop の前に server が終わっていたとき，Manifest に attached の `client_disconnect` がないとき．C2 は drop の前に tone が読めないとき（配線と att を見直す），または monitor の state が読めなかったとき（`unavailable`）．C3 は受信機が tone を返すと走らない（NOT DONE）．
- 注意：tone が止まったことは，clean な end of burst と FPGA の飢えを区別しない．RF の出力が 0 であることも示さない（LO の漏れが残りうる）．monitor の判定（`common/tone.rs` の `carries_drop_tone`）は，window の coherence が 0.9 を超え，周波数が 100 kHz ± 20 kHz にあり（`is_drop_tone_window`．125 kHz の格子点は外れる），power が最初の 1 秒の noise floor の 4 倍を超えること．1 % 以上の window がそうなら，その 200 ms は tone．起動は，最初の 1 秒の window の半分以上が drop の tone の形なら拒む（疎らな coherent window では拒まない）．200 ms の block の最後の sample が，その行を書く時刻の device 時刻より 300 ms を超えて遅れていれば，その行は unknown．ordinary な noise と 0.5 LSB の量子化 noise は，この判定で tone にならない（`common/tone.rs` の試験）．0.3〜0.4 LSB の量子化 noise は，lattice の window（窓の lag 積がすべて 45°）が 125 kHz で coherence 1 を読む．周波数の幅が ±25 kHz だったときは shape の window が 800 のうち平均約 6 個になり，1 % の bar（8 個）を超える run が約 3 割出た．±20 kHz で 125 kHz は外れ，平均約 0.3 個になる（シミュレーション，INFERRED）．DDC の後の相関した noise や ±2 の値は別の角度を作るので，C3 だけは二重の守りとして，shape に加えて平均電力が 4 LSB の振幅を超える window だけを tone と数える（`quiet_check`，`QUIET_CHECK_MIN_POWER`）．loopback の tone は約 −49 dBFS（INFERRED：`bench-results.md:27-28` から）で，この床より約 30 dB 上．C3 の verdict は最大の tone の形の window の振幅（LSB）を出すので，bench で level が残る．**未確認**：bench の noise 床が 0.3〜0.4 LSB のときの C3 の実測（P5〜P7 は雑音の coherence が高く INCONCLUSIVE だった）．）
- stop：C1 FAIL（15 s 後も server が動いていた）は owner へ．後始末の後も `docker ps` に `ezsdr-drop-` の container が残るなら owner へ．monitor の container が残るなら `rx-monitor.sh stop`．PID でない kill はしない．
- `EZSDR_DROP_WAIT=sleep`（server が `advance` の中にいるときの切断）と `EZSDR_DROP_CUT=kill`（server の死）は，この節の下の「#89 の後」の節．

### 時間の合計（estimate）

| ブロック | 内容 | 機材 | 時間 |
|---|---|---|---|
| (a) | host と装置の確認（TX なし） | #2 → #1 | 約 10 分 |
| (b) | 回帰（T1 の 9 試験） | #2 UBX，#1 OBX | 約 40 分 |
| (c) | 前線（T2：front 12 行と raw 288 行） | #2 UBX，#1 OBX | 約 25 分 |
| (d) | 保留の尾と Stop の領域（T3，6 行） | #2 UBX，#1 OBX | 約 20 分 |
| (e) | 反復の切り替え（T4，2 行） | #2 UBX，#1 OBX | 約 15 分 |
| (f) | 未指定の送信（T5，2 行） | #2 UBX，#1 OBX | 約 10 分 |
| (g) | Stop の終わり（T6，3 行） | #2 UBX，#1 OBX | 約 10 分 |
| (h) | 送信の rate（開いたままの streamer，2 盤） | #2 UBX，#1 OBX | 約 20 分 |
| (i) | 送信中の ssh 切断（3 行の VERDICT，#2 の monitor を含む） | #1 OBX，#2 UBX（受信） | 約 12 分 |
| **合計** | 盤の交換と電源 cycle，初回の build を含まない | | **約 162 分（約 2 時間 42 分）** |

### 閉じ方と記録

- **#78 と #70**：2026-11-06 に閉じた（handoff §1）．この節の (b) の FAIL は回帰として owner へ報告する．issue の開き直しは orchestrator．
- **段階 5 の検証**：(c)，(d)，(e)，(f) の VERDICT と (g) の VERDICT は，この節の結果として記録する．閉じる判断は orchestrator．
- **送信の rate の受け入れ条件**（note 47 §1）：(h) が両盤で PASS．閉じるのはこの条件だけで，機能の設計（note 47 §4 の 4）は別．
- **ssh 切断**：(i) の C1・C2・C3 が PASS．monitor が起動しなければ NOT DONE（閉じない）．

### 流した後

- orchestrator に返すもの：commit，各ブロックの VERDICT 行（log の path），(b) の表の TIME_ERROR の数（case ごと），(c)1 の 12 行，(c)2 の raw 表（288 行，rate・n・gap・complete／incomplete・late の数），(d) の 6 行，(e) の 2 行と `switches` の行，(f) の 2 行，(g) の 3 行と `heard_until` − 記録上の終わり，(h) の 2 行と A の ACK の e からの差・B の onset の delta（tick），(i) の 3 行と `docker ps` の残り，#2 の monitor の state file と log，電源 cycle の有無．
- repo へ写すのは，owner が確認したものだけ．log は `~/.cache/ezsdr-bench/<日付>/` に残す．ただし (i) の script の記録（`drop.log`，`rx-check.log`，`driver.log`）は `~/ezsdr-bench/<日付>/ssh-drop-*/` に，monitor の state file・log と console の log は `~/.cache/ezsdr-bench/<日付>/` に残る．両方を報告に書く．

## <DATE> の手順（段階 F の後：時刻で決める前線と送信の rate の下限）

`<COMMIT>` は段階 F（`reviews/stepF-*`）が `main` に入った commit．結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．出典：[bench-results.md](bench-results.md) の「2026-11-10」（UHD だけの掃引と owner の決定），[design/07](../../design/07-radio-model.md) RM-15・RM-16，[design/18](../../design/18-uhd-radio.md) UR-22・UR-23・UR-33，KL-36・KL-40．

規則（記録，uhd-tx，FakeDevice で同じ）：32 sample 以下の閉じた device burst（1〜3 は 4 に詰める）の後の次の安全な開始は，閉じた send の排他的な終わり＋g，g = max(3, ⌈15 µs × rate⌉)．ほかの閉じの後は＋3（長い頭・継ぎの前線は payload の終わり＋4，Stop の境は＋5）．送信の rate は 1 MS/s 以上．

| rate | 1 MS/s | 2 MS/s | 2.857 MS/s（N = 70） | 10 MS/s | 100 MS/s | 200 MS/s |
|---|---|---|---|---|---|---|
| g（sample） | 15 | 30 | 43 | 150 | 1500 | 3000 |

### 構成と順番

- 機材，接続，att は 2026-11-10 と同じ（#2 X300＋UBX slot A → #1 X300＋OBX slot A，ループバックの 30 dB，基準は内部）．試験は slot A だけ．
- **送信の前に owner に att を確認する**（毎回）．送信の rate は 200 MS/s まで（ケーブルのループバックに限る，owner 2026-11-10）．bench の envelope は送信の channel ごとに ±1 MHz のまま（RM-19 は中心周波数だけを見る）．
- 共通の準備（`hw` の関数，log の置き場，stop の規則）は「note 47 段階 5 の後」の節と同じ．
- 時間は estimate．tone が時間で決まる（10 ms）ので，(c)1 は前より長い．

### (a) host と装置の確認（送信なし）

「note 47 段階 5 の後」の (a) と同じ．`uhd_usrp_probe` の serial，FPGA 39.3，slot A の board，frame 8000．開くときの `Reference Clock PLL failed to lock` は一度だけ開き直し，続けて出たら電源を入れ直す（2026-11-10 は #2 で 3 回，#1 で 2 回）．

### (b) 回帰：T1 の 9 試験（TX あり）

「note 47 段階 5 の後」の (b) と同じ試験と同じ盤の順．期待の変わった所：
- `hw_b8_burst_one_sample_after_a_burst`：30 000 sample の burst の後，end＋3 は受付で拒否（`UR-21: the burst overlaps an accepted booking`），end＋4 は TIME_ERROR なしで鳴る（前は end＋1 と end＋2）．
- `hw_b7_session_loopback`：envelope の外への同調し直しは 1.5 GHz（すべての band の外であることを送信の前に assert）．
- ほかの 7 試験の期待は変わらない（段階 F で全 hardware 試験を点検した．B は頭や前の burst から十分離れているか，継ぎの規則で決まる）．

### (c) 前線（TX あり）

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && run_all \
  hw_n47s5_front_after_a_short_burst hw_n47s5_long_b_after_a_short_head_gap_below_and_at_the_front \
  hw_n47s5_a_long_close_at_100_and_200_msps hw_n47s5_raw_front_gaps                         # #2 UBX slot A
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && run_all \
  hw_n47s5_front_after_a_short_burst hw_n47s5_long_b_after_a_short_head_gap_below_and_at_the_front \
  hw_n47s5_a_long_close_at_100_and_200_msps hw_n47s5_raw_front_gaps                         # 盤を替えて #1 OBX
```

- (c)1 `hw_n47s5_front_after_a_short_burst`：n ∈ {1, 2, 3, 4, 10, 32} × 6 rate = 36 行．予約は頭の詰めた終わり＋g．PASS：admitted，TIME_ERROR と underflow なし，BurstAck が 3 つ，tone の onset が予約から遅れ ± 50 sample．
- N-H1 `hw_n47s5_long_b_after_a_short_head_gap_below_and_at_the_front`（raw，6 rate）：gap g で長い B が鳴る（BurstAck 2，late と underflow なし）．gap g − 1 は印字だけ．
- `hw_n47s5_a_long_close_at_100_and_200_msps`：20 000 sample の Session の burst の閉じ（保持した尾＋0）の 3 sample 後に次の burst．PASS は閉じの形どおりで両方の ACK，late と underflow なし．形が違えば INCONCLUSIVE．**200 MS/s の＋3 は測った最小（余裕 0）**．FAIL なら止めて owner へ（＋3 の値を諮り直す）．
- (c)2 `hw_n47s5_raw_front_gaps`：観測だけ（6 rate の表）．
- stop：(c)1 の FAIL，想定外の TIME_ERROR，装置の詰まり → 電源を入れ直し owner へ．

### (d) 保留の尾と Stop の領域：T3（TX あり）

```sh
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_n47s5_held_tail_and_stop   # #2
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_n47s5_held_tail_and_stop   # #1
```

- (f) の 3 行：J(e) の前の継ぎは一つの device burst．J(e) の後の C は e＋4（前は e＋2）で別の burst．e＋1 は受付で拒否．
- (b) raw：two zeros の閉じ（cursor c）の後は c＋5 で判定（c＋3・c＋4 は印字だけ）．
- (h) R5-m5：Stop が c − 1 で切った 1〜3 sample の頭（c＋3 まで詰める）の後，c＋3＋g に予約（rate ごと）．PASS は全試行で ACK 2，late と underflow なし．
- Session の行は INCONCLUSIVE のまま（Provider の境が記録にない，前と同じ）．

### 閉じ方と記録

- **#81**：(c)1 の 36 行と N-H1 が両盤で PASS なら閉じる．
- **#82**：(d) の R5-m5 が両盤の全 rate で PASS なら閉じる．
- **＋3 の余裕**：`hw_n47s5_a_long_close_at_100_and_200_msps` が両盤で PASS．FAIL なら owner に値を諮る．
- (b) の FAIL は回帰：止めて owner へ．

### 流した後

orchestrator に返すもの：commit，各試験の VERDICT 行と log の path，(c)1 の 36 行，N-H1 の行（gap g − 1 と g），長い閉じの行（形と ACK），(c)2 の表，(d) の行，電源の入れ直しの有無．log は `~/.cache/ezsdr-bench/<日付>/`．

## <DATE> の手順（#89 の後：client が去ったら，待っている request の中でも Session が終わる）

`<NEW>` は #89（S1〜S3）が `main` に入った commit．`<OLD>` は `8dd1016`（#89 の前）．結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．出典：[design/16](../../design/16-easy-api.md) EA-15・EA-16，[design/06](../../design/06-kernel-coordinator.md) KC-36，[KNOWN_LIMITATIONS.md](../../KNOWN_LIMITATIONS.md) KL-43，この文書の (i)（2026-11-09 の ssh 切断，server が idle のとき）．

目的：ssh が切れたとき server が `advance`（`sdr.sleep`）の中にいても Session が終わり，送信が止まること．(i) は idle の server しか調べていなかった．もう一つ，server が cleanup なしで死んだ後 X300 がどう止まるか（E9：INFERRED．repeat は host から供給される（KL-5）ので，samples が尽きて止まるはず．記録はまだない）を測る．**測るだけで，E9 の値を規則にはしない**．server の fail-stop（入力が終わって N 秒で exit する．A1）を足すかどうかは，E9 の結果を見て orchestrator と owner が決める．

### 構成と順番

- (i) の配線と同じ（二台を同時につなぐ）：#1 OBX の TX/RX → att（30 dB 以上）→ splitter．splitter の一方は #1 の RX2（C3 の loop），もう一方は #2 UBX slot A の RX2．att は splitter の前に 1 個で，両方の受信経路を覆う．#2 は ssh を通さず，`rx-monitor.sh` が直接受信する．
- **送信の前に owner に配線と att を確認する**（毎回．script は `yes` を求め，答えがなければ docker にも device にも触れずに止まる）．三つの run は一つの session で続けて流す．順は (i-a) → (i-b) → (ii)．
- 前提と monitor の起動は (i) の手順 1〜3 と同じ（`ssh -o BatchMode=yes localhost true`，`rx-monitor.sh start`，quiet を待つ）．**新しい client は server を自分の session で起動する**（`start_new_session`）ので，ssh の wrapper は prompt を出してはいけない．wrapper は `ssh -T -o BatchMode=yes` のまま．
- どの run も，直前に monitor が fresh な quiet を読んだときだけ走る．run の間に `rx-monitor-state.sh "$STATE"` が quiet を返すのを待つ（最大 60 s）．
- server と Python の client は `<COMMIT>` の tree から（script が snapshot を取る）．driver と relay は，いま走らせている tree のもの．だから (i-a) は **旧い server と旧い client に新しい driver** を使う．これが比べたい組である．
- 試験の前に（送信しない）：`python3 plan/phase8/tools/ssh-drop-driver.py --self-check`，`python3 plan/phase8/tools/ssh-drop-relay.py --self-check`，`bash -n plan/phase8/tools/ssh-drop-tx.sh`，`bash plan/phase8/tools/ssh-drop-tx.sh --self-check`（E9 の判定を stub の docker・受信機・時計で調べる．装置も docker も ssh も要らず，数秒で終わる．`all cases pass` が出ること）．

### 変数の意味

- `EZSDR_DROP_WAIT=sleep`：driver が `sdr.sleep(hold)` で待つ．server は drop のとき一つの `advance` の中にいる．`time`（既定）は `time.sleep`：server は idle．
- **evidence**（relay）：ssh の前に relay（`ssh-drop-relay.py`）が置かれ，client が書く request の header ごとに時刻と op を `$OUT/to-server.log` に書く．`sleep` では，drop の直前の request が `advance` で，drop の 1 s 以上前に書かれ，driver の `wait-start` より後であるとき，server が `advance` の中にいたと見なす（INFERRED：idle の server は frame を µs で取る）．無ければ C1 と C2 は `INCONCLUSIVE: no evidence that the server was inside advance at the drop: …`（本来の判定は括弧に残る）．
- **hold の余裕**（F7）：drop の時に `sleep` の残りが 30 s 未満なら（15 s の期限に 15 s を足す）C1 と C2 は INCONCLUSIVE．既定の hold 120 s，settle 5 s なら残りは約 115 s．`EZSDR_DROP_HOLD` を変えるときは，settle と monitor の確認を足しても 30 s 以上残ること．
- `EZSDR_DROP_CUT=kill`：local の ssh client を殺す代わりに server の container を `docker kill` する（server が cleanup なしで死ぬ）．C1 と C2 の代わりに `VERDICT E9 …` を出す．`EZSDR_DROP_RX_CHECK` が要る．
- **E9 の起点**：container が止まった時刻．Docker の `State.FinishedAt`（daemon が記録した exit の時刻．プロセスの死より数 ms 遅れる，INFERRED）を使い，読めなければ container を止めた command（`docker kill` / `docker stop`）が返った時刻を使う（死より後なので，E9 を最大でその command の遅れだけ短く数える）．行にどちらかが書かれる．**起点を出すのは，`docker inspect` が `running=false` を読んだ container だけ**：動いている，または inspect できないときは起点がなく，E9 は測らない．誤差は，括弧つきの行では最後の tone の読みと最初の quiet の読みの間の幅（行が ms で書く．読めなかった読みが間にあれば，monitor の分解能 1 s より広い）．emergency stop の E9 は `docker stop -t 2` が返った後の時刻から数える（stop の待ちの最大 2 s は含まない）．
- `docker kill` と `docker stop` を container 名（`ezsdr-drop-…`，一つに決まる）に対して使うのは，「PID でない kill はしない」の唯一の例外である（既存の emergency stop と同じ）．名前の pattern による kill はしない．
- **E9 の保証**：測定の行は，container が止まったと確認できた後にだけ出て，次の二つのどちらかだけ（ほかは INCONCLUSIVE）：(1) 括弧つきの行（`ended after …`）は，止まった後に読んだ最後の tone と最初の quiet の時刻を書き，その幅を誤差として書く．(2) 上限の行（`ended by +N ms (upper bound: no tone was read after …; the last tone was read before it)`）は，止まった後の最初の読みがすでに quiet だったもので，tone の読みは死の前のものしかない．tone が死より前に止まっていたかもしれないので，E9 は N ms 以下，という上限だけを言う（A1 の判断には上限で足りる）．読めなかった読み（`unavailable`）は quiet に数えない．`was still on 60 s after` は，60 s の窓の最後の読みが tone のときだけ出る．それ以外はすべて `VERDICT E9 INCONCLUSIVE: …` で，測定ではなく，exit は 0 でない：kill の前に tone が読めなかった，`docker kill` が失敗した，**container が止まったと確認できなかった**（`the container was not confirmed stopped: state …`．server がまだ動き，装置がまだ送信しているかもしれない．stderr の警告を owner に伝える），tone を読んだ後で受信機が窓の終わりまで読めなかった（`the tone was last read at +N ms and the receiver was unreadable after it`），60 s の間一度も読めなかった．emergency stop の `docker stop` が失敗したときも stderr に出て，exit は 0 でない．

### (i-a) `<OLD>`（`8dd1016`）で `sleep`：FAIL が期待（#1 OBX が送信，#2 が受信，TX あり，約 12 分，estimate）

```sh
if [[ $(plan/phase8/tools/rx-monitor-state.sh "$STATE") == quiet ]]; then
  EZSDR_DROP_WAIT=sleep EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
  EZSDR_DROP_RX_CHECK="$PWD/plan/phase8/tools/rx-monitor-state.sh $STATE" \
    plan/phase8/tools/ssh-drop-tx.sh 8dd1016 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/89-ia.console.log
fi
```

- 期待：`VERDICT C1 …: FAIL: server container … still running 15 s after the link dropped`（server は `advance` の中で ssh の切断を知らない），`VERDICT C2 …: FAIL: the tone was still on the independent receiver at +… s, the last reading before the 15 s deadline`．C3 は受信機の最後の読みが tone でなければ走る．exit 1 になるのが正しい．
- INCONCLUSIVE になったら（evidence または hold の余裕）流し直す．FAIL でないなら，旧い commit で #89 の穴が再現しなかった：owner へ（設計の前提が崩れる）．
- **E9 の最初の data point**（追加の run なし）：C1 と C2 を出した後，script は動いている server を `docker stop -t 2` で止める（PID 1 は handler のない SIGTERM を無視するので，実際は最大 2 s 後の SIGKILL．INFERRED）．これは host の unclean な死そのものなので，C2 が tone のままだったとき，script は container が止まったと確認できた後にだけ受信機を読み続け，`VERDICT E9 the tone on the independent receiver ended after the emergency docker stop …, counted from …`（最後の tone と最初の quiet の時刻と幅，ms，stop から．最初の読みが quiet なら上限の行 `ended by +N ms (upper bound: …)`）か，60 s 後も tone のままという行（最後の読みが tone のとき），または INCONCLUSIVE の行を出す．止まったと確認できなければ測らない．誤差は行が書く幅．
- 60 s たっても tone が残るなら，device はまだ送信しているかもしれない：**owner に伝える**．host からは止められない（server は死んでいる）．止めるのは USRP の電源を入れ直すことで（その前に `docker ps` と monitor の log を残す），新しい UHD の session を開くのは owner の言葉があるときだけ．

### (i-b) `<NEW>` で `sleep`：PASS が期待（同じ配線，約 12 分，estimate）

```sh
if [[ $(plan/phase8/tools/rx-monitor-state.sh "$STATE") == quiet ]]; then
  EZSDR_DROP_WAIT=sleep EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
  EZSDR_DROP_RX_CHECK="$PWD/plan/phase8/tools/rx-monitor-state.sh $STATE" \
    plan/phase8/tools/ssh-drop-tx.sh <NEW> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/89-ib.console.log
fi
```

- 期待：C1 PASS（server が 15 s 以内に終了し，Manifest に `client_disconnect`，attached の Lease．`termination.at` は sleep の終わりより前），C2 PASS（期限の前に quiet），C3 PASS．exit 0．
- 参考：(i)（2026-11-09，idle）では，切断から server の終了まで 1 s 未満，受信機は +0 s（1 s 単位）で quiet だった．`Provider::stop` は #1 で約 104 ms（同じ run の Manifest の `timing` の stop の行：at 1479191981，done 1500026409，200 MHz の root tick．VERIFIED，標本 1）．この run がそれと同程度なら，cut の待ちは stop の長さに足されていない．
- FAIL は owner へ（#89 の直しが効いていない）．INCONCLUSIVE なら流し直す．

### (ii) `<NEW>` で `kill`：E9 の測定（同じ配線，TX あり，約 12 分，estimate）

```sh
if [[ $(plan/phase8/tools/rx-monitor-state.sh "$STATE") == quiet ]]; then
  EZSDR_DROP_CUT=kill EZSDR_DROP_BOARD=obx EZSDR_BENCH_NIC=enp1s0f1np1 EZSDR_BENCH_SUBNET=40 EZSDR_UHD_ARGS=addr=192.168.40.36 \
  EZSDR_DROP_RX_CHECK="$PWD/plan/phase8/tools/rx-monitor-state.sh $STATE" \
    plan/phase8/tools/ssh-drop-tx.sh <NEW> 2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/89-ii.console.log
fi
```

- repeat が鳴っている間に server の container を `docker kill`（SIGKILL）する．EOB も Manifest もない．出る行：`VERDICT E9 the tone on the independent receiver ended after the death of the server container by docker kill, counted from …: the last tone was read at +… ms, the first quiet reading at +… ms (the bracket is … ms wide: the error)`，または上限の行 `… ended by +N ms (upper bound: no tone was read after …; the last tone was read before it)`（止まった後の最初の読みが quiet．E9 は N ms 以下としか言えず，A1 の判断にはそれで足りる），または `… was still on 60 s after …`．C1 と C2 は出ない．C3 は quiet の後に走る．exit 0 は E9 が quiet を読み，C3 が PASS のとき．`VERDICT E9 INCONCLUSIVE: …`（上の「E9 の保証」の各場合）は測定ではない：配線か送信か受信機を直して流し直す（exit 1）．container が止まったと確認できないときは，装置が送信中かもしれないので先に owner に伝える．
- 読み方：E9 の終わりが (i-b) の C2 の時刻と同程度なら，ホストが死んでも装置は短い時間で止まる．数秒以上かかる，または 60 s 後も tone なら，KL-43 の (b) の「samples が尽きて止まる」は成り立たない：A1（server の fail-stop）の採否の材料になる．どちらでも，tone が止まったことは，clean な end of burst と FPGA の飢えを区別しない．RF の出力が 0 であることも示さない（LO の漏れが残りうる）．
- **E9 の条件を書く**（AGENTS §9 の bench の値）：rate は 1 MS/s，tone 100 kHz，1 GHz，振幅 0.4，#1 OBX，一回の run．ほかの rate や board に当てはめるには掃引の計画が先（境の近くで 1 セル 10 回以上）．
- 60 s たっても tone が残るなら，device はまだ送信しているかもしれない：**owner に伝える**．止めるのは USRP の電源を入れ直すことで，新しい UHD の session を開くのは owner の言葉があるときだけ．

### 閉じ方と記録

- **#89**：(i-a) が FAIL（旧い commit で穴が再現），(i-b) の C1・C2・C3 が PASS．monitor が起動しなければ NOT DONE（閉じない）．
- **E9**：(ii) の行と (i-a) の stop の後の行を bench-results.md に書く（条件つき）．A1 の採否と KL-43 の (b) の文は，その後に orchestrator と owner が決める．
- stop：(i-b) の C1 FAIL，後始末の後も `docker ps` に `ezsdr-drop-` の container が残る，E9 で 60 s 後も tone，または container が止まったと確認できない（`not confirmed stopped`，`emergency docker stop … failed`）→ owner へ．monitor の container が残るなら `rx-monitor.sh stop`．PID でない kill はしない（container 名への `docker kill` / `docker stop` は上の例外）．

### 流した後

orchestrator に返すもの：`<NEW>` の commit，(i-a)・(i-b)・(ii) の VERDICT 行（C1，C2，C3，E9）と log の path，`drop.log`（`entry evidence` の行を含む）と `to-server.log`，`rx-check.log`，`docker ps` の残り，#2 の monitor の state file と log，電源 cycle の有無．log は (i) と同じ場所（`~/ezsdr-bench/<日付>/ssh-drop-*/` と `~/.cache/ezsdr-bench/<日付>/`）．raw は bench/ の規則（AGENTS §9）で private repo の release へ．

## 2026-11-15 の項目：#67 の raw probe `hw_67_stop_before_origin`（origin より前の untimed stop と close，受信のみ，TX なし，約 5 分 / 台，estimate）

問い：[#67](https://github.com/k3komatsu/Ez-SDRv4/issues/67)。uhd-rx は，遠い origin に timed start を積んだ後 `Provider::stop` が来ても，spec 22 VH-6（#56）により origin を過ぎてから untimed stop するので，origin が 1 s より遠いと join に間に合わない。`Provider::stop` が始まったら origin を待たない修正は，**origin より前に untimed stop または close された streamer を X310 が安全に扱う**ときだけ成り立つ。既存の `hw_56_stop_between_starts` と `hw_56_stop_lag` はどちらも origin の後に止めるので，この問いに答えない（`~/.cache/ezsdr-fixes/2026-11-13-bench/procedure.md` §0-2）。

- **UHD 4.10 の読み（VERIFIED，v4.10.0.0 の source，Sol の code review）**：X310 の RX core は **STOP が acquisition-command FIFO を bypass して clear する**（origin を待っている timed start も消える）。よって `stop_before_origin` は **PASS するはず**（予想であって結果ではない）。「untimed の command は timed の command の後ろで待つ」という一般則は，この acquisition STOP には当てはまらない（[FIFO の扱い](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L163-L184)，[待ち状態での STOP](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v#L524-L550)）。**`close` は STOP を送らない**（streamer の destructor は graph の接続を外すだけ：[destructor](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp#L109-L114)，[切断 callback](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L340-L354)）ので，`close_before_origin` の結果が**本当の問い**である。新しい streamer が静かでも，古い acquisition が取り消されたことの証拠にはならない：origin + 500 ms まで観測して初めて意味を持つ。

- **TX なし**：受信だけ（RX2，gain 0，1 GHz）で，`tx_open` も送信もしない。att の確認は要らない（配線は他の項目のままでよい）。
- 流し方（`hw` は上の節の定義。`BOARD` と NIC は機材ごと）：

  ```sh
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_67_stop_before_origin   # #2 UBX
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_67_stop_before_origin   # #1 OBX
  ```

  結果は単独で log に出る（記録 suite には入れていない）。`grep -h 'VERDICT\|N67 cell\|TRACE n67\|EZSDR-INFRA' <log>`（`EZSDR-INFRA` は 15 分の上限を越えた hang の印）。
- 構成：1 GHz，rate 1・10・100 MS/s，rate ごとに 3 つの cell × 10 trial（trial ごとに cell を順に回す），**計 90 trial**。1 trial は 1 本の streamer で次を行う。
  - `stop_before_origin`：`rx_start(at = now + 2 s)`，約 100 ms 読む（何も来ないはず），untimed `rx_stop`（呼び出しの入りと戻りの device 時刻を別々に記録する），origin + 500 ms まで読む，同じ streamer で新しい origin（100 ms 先）に `rx_start`，最初の block を見る。
  - `close_before_origin`：同じだが stop の代わりに `close_streams()` の後 `rx_open` し直し（新しい streamer は start しない），その streamer で origin + 500 ms まで読む。
  - `stop_after_origin`（control，VH-6 の今の道）：`rx_start(at = now + 300 ms)`，origin + 100 ms まで読み（block が来るはず），untimed `rx_stop`，origin + 500 ms まで読む，新しい start。
  - どの trial も最後に untimed `rx_stop` と drain（2 回続けて timeout で quiet）をして次へ。
- **VERDICT**（`VERDICT n67 <cell> <rate> MS/s trial <i>: …`，定義は `iii::cancel_trial_verdict`）：
  - PASS：遠い origin の cancel の呼び出しが **origin より前に戻り**，cancel の後に block が 1 つも来ず（control だけは停止した stream の尾として戻りの後 2 ms まで許す。取り消した start には尾の許容はない），観測が origin + 500 ms まで届き，新しい start の最初の block の tick が新しい origin とちょうど一致し，samples が有限で全 0 でない。cancel の latency（入りから戻りまで）は**全 trial の TRACE と PASS の行に出る**。
  - FAIL：呼び出しの失敗（`time_now` の失敗を含む），または samples と timeout 以外の `rx_recv` の結果（`LateCommand`，`Overflow` など），**遠い origin の cancel が origin の後に戻った（`cancel returned after the origin`）**，遠い origin の cancel の前に block が来た，cancel の後の block（遠い origin の 2 variant は 1 つでも，control は尾の 2 ms を過ぎたもの），新しい start の最初の block が origin と違う，samples が全 0 か非有限，cleanup の drain が 3 s で静かにならない。
  - INCONCLUSIVE：control が stop の前に block を出さなかった，観測が origin + 500 ms に届かなかった，または新しい start の block が 2 s 待っても来ない（stream が戻らない）。
  - この probe は streamer の leak を見ない：device は全 trial を通して 1 つで，`uhd_kept_count` は device の drop しか数えないので，trial ごとの streamer の leak の証拠にならない（検査から外した）。
  - **hang**：device の**一生全体**（開く・設定する・90 trial・destructor・失敗の unwind）が **15 min** を過ぎると `EZSDR-INFRA:` の行を書いて abort する（UHD の呼び出しは中断できない。期待は約 5 分）。trial ごとの bound はない。hang した trial の VERDICT 行はない。X310 が戻らなければ電源の入れ直しが要るかもしれない：owner へ。
- 最後に cell ごとの `N67 cell <cell> <rate> MS/s: n PASS, n FAIL, n INCONCLUSIVE of 10`。FAIL が 1 つでもあれば test は失敗する（exit 1）。INCONCLUSIVE だけなら `test result: ok` で終わるので，cell の行を必ず読む。`TRACE n67 {json}` は trial ごとの tick 列（origin，`cancel_entered`，`cancel_returned`，`cancel_latency_ms`，`observed_to`，late の最初と最後，新しい origin との差，errors）。
- #67 への読み方（**結果は bench-results.md に書く（orchestrator）。条件を超えて一般化しない：1 GHz，この 3 rate，origin 2 s，1 cell 10 trial，この board**）。まず **trial の FAIL と「cancel が安全でない」という証拠を分ける**：
  - `Overflow`，`LateCommand`，host の scheduling（`time_now` の失敗，100 MS/s で host が追いつかない）は観測を無効にする：#67 については INCONCLUSIVE として読む（test の VERDICT は厳しく FAIL のまま）。**証拠になるのは，取り消した start の位置から来た block と，位置のずれた次の start だけ**。「新しい block が来ない」は壊れた証拠ではない（INCONCLUSIVE）。`cancel returned after the origin` は cancel の呼び出し自体の証拠（stop や close が origin まで戻らないなら，#67 の join には役に立たない）だが，host が止まっていたのが原因でないか latency を見て確かめる。
  - control が通っていても，他の trial の一時的な overload を除外したことにはならない。cell の 10 trial を見て，FAIL の理由を VERDICT の行で確かめる。
  - `stop_before_origin` と `close_before_origin` が全 rate で 10/10 PASS：origin の前の untimed stop と close を X310 は安全に扱う。ただし close の PASS は「開き直した streamer が静かで，次の start が origin どおり」を示すだけで，古い start の出力がその streamer に届く場合にだけ取り消しの証拠になる（INFERRED，未確認）。`Provider::stop` の後に origin を待たない shutdown が許される（設計は orchestrator と owner が決める）。
  - `stop_before_origin` が FAIL（cancel の後 origin で block が来る）：積んだ start は untimed stop で消えない。VH-6 のとおり origin を待つしかない（または別の取り消し方が要る）。UHD の読みと食い違うので，source と照らして orchestrator へ。
  - `close_before_origin` だけが FAIL：close は安全でない。stop は安全。
  - どちらかで次の start が誤った位置（block がある）：origin 前の cancel が streamer か device の状態を壊した証拠。block が来ないだけなら INCONCLUSIVE で，再現を見る。
  - `stop_after_origin`（control）が FAIL か INCONCLUSIVE：条件が成り立っていない（100 MS/s で host が追いつかず `Overflow`，新しい start が origin からずれる，など）。同じ rate の他の cell を読まない。`TRACE` の `next_offset_ticks` が control でも 0 でないなら，それは #67 と関係がない。

## 2026-11-15 の手順（#67 の probe に加えて：受信 restart の zeros の掃引，#75 の probe，open-flush の掃引）

> stage 4 の値のための計測（S2・T200・S5・S4）は，この下の「2026-11-15 の手順（stage 4 の値のための計測…）」の節．

出典：`~/.cache/ezsdr-fixes/2026-11-13-rxzeros/options.md` §4（掃引の計画）と末尾の owner の決定（direction D），`~/.cache/ezsdr-fixes/2026-11-13-n75/options.md`（#75）§2.4 と owner の決定（(3) は下の open-flush の掃引の後に enable），`crates/ezsdr-radio-uhd/tests/common/bench1115.rs`（cell の表・summary・verdict・決定の規則はここ，非 device の試験は default の suite と FakeDevice の rehearsal）．**どれも結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．** 4 つの試験は `hw` で単独に流せる（記録 suite には入れていない）．いずれも device の一生全体を 1 本の期限で囲み（`bounded`），過ぎると `EZSDR-INFRA:` の行を書いて abort する．hang の印なので電源を入れ直して owner へ．

- 構成・準備・`hw` の定義・stop の規則は「note 47 段階 5 の後」の節と同じ（#2 UBX slot A → #1 OBX slot A，30 dB，基準は内部，slot A だけ）．順は **#2 → #1**．
- **盤の移動（owner 2026-11-14：#2 の UBX を #1 の slot B へ，2 channel の cell のため）**：この節と下の stage 4 の節の **#2 の項目（任意の #2 の T200 を含む）をすべて先に**流す．次に両方の電源を落とし，UBX（32BC76F）を #2 の slot A から #1 の slot B へ移し，その UBX 用の loopback（TX/RX → ≥30 dB の att → 同じ UBX の RX2）を #1 に組む．移した後の #2 は slot A が空で，この日はもう使わない．#1 の (a) では probe で slot A の OBX と slot B の UBX を確かめる．#1 で 2 channel の cell が走る：(4) の 2 channel の cell（owner が slot B の att を確かめた後に `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1` を付けた 1 回．`bench1115::two_channel_gate`）と，S5 の (3)（受信のみ）．stage 4 の項目は slot A にだけ送信する（`pin_to_slot_a`・`slot_a_guarded`）ので，slot B に盤が入っても変わらない．
- **送信するのは (2)(2b)(3)(4)**．#67（(1)）は受信だけで att は要らない．**(2)(2b)(3)(4) の 1 本ごとの前に owner が各 board の TX/RX → ≥30 dB の att → 同じ board の RX2 を確かめる**（毎回．4 本を 1 回の確認で続けて流さない）．
- **(2) は送信 gain を 0 dB から +10・+20 dB まで上げる**．att を loopback に入れたまま送信 gain を最大まで許す owner の決定（2026-10-27，T5 の +6 dB と同じ）に基づく．上限は `bench1115::MAX_TONE_GAIN_DB` = 20 dB で，これを超える cell は送信の前に panic する．試験の終わり・失敗・unwind で gain は 0 dB に戻る（FakeDevice の試験が確かめる）．波形の振幅は 0.4 のまま，周波数は 1 GHz（`SAFE_TX_HZ`），rate は 1 MS/s の tone 源（(3) は 1・200 MS/s と 1・10 MS/s，(4) は 1・2・10 MS/s の 4 byte の零）．
- **(2)(3)(4) は slot A の channel 0 にだけ送信する**（(4) の 2 channel の cell は上の slot B の確認を別に要る）。`EZSDR_D9_CHANNEL` が 0 以外なら device を呼ぶ前に panic する（`hw` はこの変数を container に渡さないので 0）。
- 起動の形（盤ごとに export し直す．送信する probe は 1 本ずつ，確認のあと 1 コマンドだけ．FAIL が事実の報告である (3) は `run_all` ではなく一つずつ）：

  ```sh
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0   # #2 UBX．盤を替えて addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 が #1 OBX
  hw hw_67_stop_before_origin                                  # (1) 受信のみ．att 不要
  # --- owner が att（TX/RX → ≥30 dB → RX2）を確かめた．次の 1 本だけ ---
  hw hw_rxzeros_sweep                                          # (2) 送信
  # --- owner が att を確かめた．次の 1 本だけ ---
  hw hw_rxzeros_pn                                             # (2b) 送信（gain 0 dB の PN，受信 restart の zeros の相互相関）
  # --- owner が att を確かめた．次の 1 本だけ ---
  hw hw_75_t3_probes                                           # (3) 送信
  # --- owner が att を確かめた．次の 1 本だけ（2 channel の cell は走らない配線．slot B の att を owner が確かめて EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 を付けない限り slot B には送信しない．owner が免除しない限り decision は INCONCLUSIVE，(4) 参照）---
  hw hw_75_open_flush_sweep                                    # (4) 送信（slot B には送信しない形）
  # --- slot B の att も owner が確かめた場合だけ．1 本の呼び出しに付ける（export しない：shell に残ると後の試験にも渡る）---
  EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 hw hw_75_open_flush_sweep
  ```

### (1) `hw_67_stop_before_origin`（受信のみ，約 5 分 / 台，estimate）

詳細・VERDICT の読み方は上の「2026-11-15 の項目：#67」．決めること：origin より前の untimed stop と close を X310 が安全に扱うか（#67）．

### (2) `hw_rxzeros_sweep`（TX あり：tone 源，gain 0・+10・+20 dB，約 8 分 / 台，estimate．期限 30 分）

問い：受信 restart 直後の exact-zero の数は DDC の half-band 数 n_hb = min(v2(D), 3) の束の値か（owner の direction D：bound 0 / 4 / 11 / 15），そして (a) 毎回の start か restart だけか．

- **cell 35**（`bench1115::zeros_cells`）：rate cell 20（D = 1, 2, 3, 4, 6, 8, 10, 12, 20, 24, 25, 40, 50, 60, 100, 125, 200, 248, 250, 255．各 n_hb の束に 5 つ．rate は 0.784〜200 MS/s）× restart，10 trial（束の境の D = 3, 125, 255, 6, 10, 50, 250 は **15**）；start の種類 3 × level の rate 3（D = 200, 100, 2 = 1・2・100 MS/s）× 10 trial：fresh open（`rx_close`，rate，`rx_open`，`rx_start`），retune（受信周波数を搬送波 +50 kHz と搬送波で交互に），after_rate（companion の rate の segment の後に戻す）；level cell 6（同じ 3 つの D × gain +10・+20 dB，restart，10 trial）．各 cell は先頭に **warm-up の 1 trial**（dump はするが数えない）．trial は計 420．
- 1 trial は `hw_iii_raw_rx_restart_fill` と同じ形（`iii::restart_fill`）：tone 源が 20 ms 前から 60 ms の 100 kHz を送り，planned の 50 ms 前に start を積み，4 000 sample（最低 2 ms）を読む．数える条件は `iii::incomplete_reason`（rate の read-back，連続，100 kHz の tone，fill が測れた）．cell は計画した trial の **すべて**（10，15 の cell は 15）が complete のときだけ数える（80 % などの閾値は無い）．
- 出力：trial ごとに `RXZ trial {json}`（zeros，ramp10・ramp90，steady，precursor の peak，**先頭 40 sample の fc32（`head`）**，planned と first_tick，失敗した呼び出し）；cell ごとに `RXZ cell …` と `VERDICT rxz <cell>`；最後に `RXZ class n_hb <n>`（構造の範囲と候補の bound）・`RXZ observed`・`RXZ decision 1〜4`・`RXZ summary`。**決定の規則（`bench1115::RXZ_RULE`）は最初の行と summary の行に印字される**（結果が出る前に書いた）。
  ```sh
  grep -h 'RULE (\|RXZ cell\|RXZ class\|RXZ observed\|RXZ decision\|RXZ summary\|VERDICT\|EZSDR-INFRA' <log>     # 読む行
  grep -h 'RXZ trial' <log>                                                                             # 全 trial と先頭 40 sample
  ```
- VERDICT：FAIL は device の呼び出しの失敗か first block が planned tick と違う，INCONCLUSIVE は complete が計画した trial 数に足りない，PASS は数えた．**FAIL が 1 つでもあれば test は失敗する．INCONCLUSIVE だけなら `ok` で終わるので cell の行を必ず読む．**
- **決めること（規則，§4 の決定規則）**：**計画した 35 cell の全部が数えられる（各 cell の計画した trial がすべて complete で，呼び出しの失敗も first block のずれも無い）ときだけ** decision 1〜4 を出す．1 つでも欠けると 4 つとも `INCONCLUSIVE` で，欠けた cell の名前を書く（観測は `RXZ cell`・`RXZ class`・`RXZ observed` に残る）．D = 3, 6, 12, 24, 60 の cell も，解析 span を整数 sample かつ tone 解析の窓（25 sample）の整数倍に丸めてあるので complete になれる．
  | 行 | 結果 | 決まること |
  |---|---|---|
  | decision 1 | 4 つの束が全部 holds（gain 0・restart の rate cell の zeros の範囲が，束の前の範囲 13–15・9–11・3–4・0 の ±1 の中） | D：half-band 数の表が残る |
  | | 束が ±1 を超える | B：掃引した rate だけの表，他は `unmeasured` |
  | `RXZ class` の candidate bound | その束の**数えた全 cell（全 gain，全 start の種類）の zeros の max**．構造の比較（decision 1）とは別に出す．level の効果は bound を下げない．束の計画した cell に数えていないものがあると行に `(INCOMPLETE: …)` と `max over n of m planned` が出る：**その値は bound ではない** | 束の値 = **両 board の candidate bound の max**．束の値は `INCOMPLETE` の無い（計画した cell が全部数えられた）candidate bound からだけ作る |
  | decision 2 | fresh open が同じ D の restart の範囲の ±1 の外 | fill は measured の場合（restart）だけに掛ける．中なら**毎回の start**（(a) の答え） |
  | decision 3 | gain が範囲の端（min か max）を 2 より大きく動かす | KNOWN_LIMITATIONS：zero の数は述べた level に限る．Mock の値は bound のまま（候補の bound は全 level を含む） |
  | decision 4 | precursor の peak が steady の 5 % 以上（`head` の dump で確かめる） | fill は ramp が要る．コードの前に推薦を見直す |
- 条件を超えて一般化しない：1 GHz，1 MS/s の tone 源，gain 0・+10・+20 dB，rate 0.784〜200 MS/s の 20 点，各 10（15）trial，この 2 board，FPGA 39.3，UHD 4.10。**結果の「束の値」は 2 board の candidate bound の max を取ってから**（片方だけで決めない）。

### (2b) `hw_rxzeros_pn`（TX あり：PN，gain 0 dB のみ，約 3〜4 分 / 台，estimate．期限 20 分）

問い（owner，2026-11-13）：受信 restart の直後の zeros は **フィルタの群遅延**（restart で pipeline が空になり，信号は z sample 遅れて来るので zeros が先に出る）か，**別の fill で信号は時刻どおり**か．電力の閾値ではなく PN の相互相関で分ける．`hw_rxzeros_sweep`（(2)）の 1 phase にせず別の test にした：単独で流せる，送信 rate を受信 rate に合わせる（(2) の tone 源は 1 MS/s のまま），FAIL の意味が違う（PN が capture に無い）．

- **構成**（`bench1115::pn_sweep`，`pn_cells`）：rate 8（D = 200, 100, 40, 20, 8, 4, 2, 1 = 1, 2, 5, 10, 25, 50, 100, 200 MS/s．`hw_iii_raw_rx_restart_fill` の 8 rate）× 10 trial（先頭に warm-up 1，数えない）．slot A の channel 0 だけ，**送信 gain は 0 dB 固定**（振幅は 0.7 × 0.4 の QPSK PN = 大きさ 0.4，tone 源より低い電力．`tone_tx` が gain ≤ 20 dB と SAFE_TX_HZ を送信の前に assert，PN は大きさ ≤ 0.5 を assert）．各 cell で TX を閉じて**受信と同じ rate**で開き直す（PN の 1 sample = capture の 1 sample なので 1 sample のずれが相関に出る）．
- 1 trial：PN（周期 16 384 sample）を timed の `t0` から 1 本の burst として送り続ける（device の flow control に任せ，untimed の続きを送る．trial の終わりに EOB）．(a) 連続：受信を `ta`（t0 の 20 ms 前）から start し，**1 s 読んで捨ててから**（settled）PN 1 周期分（16 384 sample）を取る → `d_cont`（相関は先頭側）（PN を送った tick からの遅れ，sample 単位．1 MS/s で H1/H2 は 44）．(b) restart：stop，drain，PN の途中の `t1` に timed start，6 000 sample を取る → 先頭の exact zero を数え `z`，**zeros の後の sample** を PN に相互相関して `d_restart`．あわせて **sample 単位の残差**：zeros の後の先頭 256 sample を，(a) の連続 capture（同じ経路・同じ filter を通った実波形）の同じ PN 位相の sample（`res_gd`）と，`z` sample 前の位相の sample（`res_shift`）に，それぞれ最良の複素 gain で当てた相対 rms 誤差（理想 PN ではなく連続 capture と比べるので，filter による波形の変形は残差に入らない）．
- 出力：trial ごとに `RXZPN trial {json}`（z，d_cont，d_restart，delta，両方の相関の status（Found / Ambiguous / NoSignal = Absent）と peak/median，res_gd，res_shift，outcome，incomplete の理由，先頭 40 sample の `head`），rate ごとに `RXZPN cell …` と `VERDICT rxzpn <rate>`，最後に `RXZPN decision`・`RXZPN summary`．**決定の規則（`bench1115::PNZ_RULE`）は最初の行と summary に印字される**（結果の前に書いた）。
  ```sh
  grep -h 'RULE (\|RXZPN cell\|RXZPN decision\|RXZPN summary\|VERDICT\|EZSDR-INFRA' <log>     # 読む行
  grep -h 'RXZPN trial' <log>                                                                    # 全 trial
  ```
- **決めること**（trial ごと，`delta = d_restart - d_cont`，許容 ±1 sample）：
  | 結果 | 意味 |
  |---|---|
  | z < 3 | INSEPARABLE：2 つの読みの差が許容以内で分けられない（200 MS/s は z = 0 のはず）|
  | \|delta\| ≤ 1 かつ res_gd ≤ 0.35 | **zeros = 群遅延**：zeros は来るはずだった sample を置き換えるだけで，信号は時刻どおり（時間のずれ無し）|
  | \|delta − z\| ≤ 1 | **shifted**：信号が z sample 遅れて来る |
  | それ以外 | neither：trial の行を読む |
  rate の決定は 10 trial が全部 complete で一致したときだけ（割れたら MIXED，欠けたら INCONCLUSIVE）．全体は z ≥ 3 の rate が全部同じ読みのときにそれを言う．
- VERDICT：**FAIL は基盤だけ**（device 呼び出しの失敗，restart の first block が planned tick と違う，usable な capture で相関の peak が無い）．outcome 自体は事実で FAIL にしない．INCONCLUSIVE は complete が 10 に足りない（capture の gap，overflow，送信側の Underflow など，burst の ACK 無し）．**FAIL が 1 つでもあれば test は失敗する．INCONCLUSIVE だけなら `ok` で終わるので cell の行を必ず読む．**
- **INFERRED（bench で初めて確かめる）**：(i) 100・200 MS/s で host が送信（最大 1.6 GB/s の sc16）と受信を同時に続けられるか．できなければ該当 rate は Underflow / Overflow の INCONCLUSIVE になり，それは事実の報告（decision は INCONCLUSIVE）．(ii) 高い rate で PN が帯域制限され，隣の lag の相関が peak の 0.8 を超えると `Ambiguous` になり FAIL（相関の peak 無し）になる．そのとき FAIL は基盤の報告で，zeros の事実ではない．(iii) 探索範囲は ±2 048 sample：遅れがそれを超える rate は peak が出ない．
- 条件を超えて一般化しない：1 GHz，gain 0 dB，送信 rate = 受信 rate，8 rate，各 10 trial，この board，FPGA 39.3，UHD 4.10．

### (3) `hw_75_t3_probes`（TX あり：零と 1 ms の tone，gain 0，約 3 分 / 台，estimate．期限 15 分）

問い（#75，recovery の T3 flush）：(a) T3 を飛ばすと次の burst は変わるか（options.md §2.4），(b) 閉じた burst の ACK が新しい streamer に届くか（F1），(c) T3 の ACK を 150 ms 遅れて読んでも残っているか。**FAIL は device の事実の報告で，欠陥ではない**（test は FAIL で失敗するが，decision の行を読む）。

- (a)：rate 1・200 MS/s × 3 cell。skip の 2 cell は各 10 trial，control は **11 trial** で，control・open_then_skip・closed_then_skip・control・… の順に**交互に**走らせ，最後も control で終わる（各 skip の trial は前後の 2 つの control に挟まれる）。cell：`open_then_flush`（control：A を EOB なしで開いたまま，`tx_close`，`tx_open`，T3 を送り ACK を待つ），`open_then_skip`（同じで T3 を飛ばす），`closed_then_skip`（A を閉じて reopen，T3 を飛ばす）。そのあと B（1 ms の 100 kHz tone，200 MS/s は 0.1 ms）を 40 ms 先に timed で送り，同じ rate の RX で onset（容量の 90 % 分位の半分を最初に越えた sample）と長さと ACK を測る。ACK は **B の start 以後の tick** のものだけを B の ACK と数え（それより前の ACK は A の古い ACK として `old_acks` に出し，数えない．tick の無い ACK は INCONCLUSIVE）。receive の outcome が Timeout 以外（overflow など）か capture に gap（sample の tick が間隔どおりでない）がある trial は，onset と長さが B のものと言えないので **INCONCLUSIVE**（FAIL にも「T3 が要る」の根拠にもしない．`rx_other` と `contiguous` が trace に出る）。PASS：B が ACK され（ACK は `n47::burst_ack_holds`），onset が前後の 2 つの control の中点から **±400 tick（2 µs）** 以内で，長さが 400 tick 以内。**control の安定の規則**：1 つの rate の control の onset の offset が全部測れていて，max − min が **200 tick**（許容の半分）以下であること，かつ skip の trial を挟む 2 つの control の差も 200 tick 以下であること。満たさない control に対しては skip の trial を INCONCLUSIVE とし，T3 のせいにしない（`P75 a control stability` の行と claim）。`TRACE` は `P75 a trace …`（round，onset の offset，ACK の tick，report の列，位置ごとの ACK の数）。
- (b)：rate 1・10 MS/s × 2 cell × 10 trial。A は 1 ms の timed burst を EOB で閉じ，`a_ends_before_close`（A の ACK を読まずに `tx_close`，`tx_open`，すぐ T3）と `a_ends_after_reopen`（A が 120 ms 先で，閉じて開き直して T3）。新しい streamer の ACK を T3 の送信から A の終わり + 150 ms まで読み続け，**位置ごとの数と tick**（A の終わりとの差，T3 との差）を出す。PASS：ACK が 1 つで，tick が T3 の送信以後，かつ A の終わりではない（= **外来の ACK は観測されなかった**．ACK が streamer に属することの証明ではない：UHD は ACK を radio の graph の辺に載せる）。FAIL：2 つ以上，または唯一の ACK が A の終わり（= 古い burst の ACK が届いた）。device 呼び出しの失敗は `transport/setup failure`（FAIL の行，decision では INCONCLUSIVE）。INCONCLUSIVE：ACK なし，tick の無い ACK，T3 より前で A の終わりでもない ACK（誰のものか決められない）。
- (c)：rate 1・10 MS/s × 10 trial。T3 を送って **150 ms 読まず**，その後 100 ms 読む。PASS：ACK が 1 つ残っている（**保持を確認した**）。INCONCLUSIVE：ACK が無い（生成されなかった・見えなかった・期限切れ，のどれかは分からない：**欠けていることだけでは期限切れの証明にならない**），または tick の無い ACK・T3 の送信より前の tick の ACK（この T3 のものと決められない）。FAIL：ACK が 2 つ以上。device 呼び出しの失敗は `transport/setup failure`（FAIL の行，decision では INCONCLUSIVE）。
- `grep -h 'P75\|VERDICT\|EZSDR-INFRA' <log>`
- **決めること**：
  | 行 | 結果 | 決まること |
  |---|---|---|
  | `P75 a decision` | 計画した trial が揃っていない，control が清潔でない，control が不安定，device 呼び出しの失敗がある | INCONCLUSIVE（理由を書く）．T3 のせいにしない |
  | | open_then_skip の cell が FAIL（control は清潔で安定） | T3 は要る：#75 の option 2（確認された T3 だけが recovery を終える）の根拠が bench で確かめられた |
  | | closed_then_skip だけが FAIL | closed の burst の後でも skip で変わる：開いたままの A のせいではない．行を読む |
  | | 両 skip が全 PASS | 開いた burst の後でも次の burst は変わらない（この rate では hazard は見えない，INFERRED のまま）．#75 の規則を弱める根拠にはしない（条件：A は 1 000 sample の開いた burst，1・200 MS/s） |
  | `P75 b decision` | 計画した 40 trial が揃っていない，device 呼び出しの失敗，INCONCLUSIVE の trial | INCONCLUSIVE（推薦なし） |
  | | 全 PASS | **外来の ACK は観測されなかった**：F1 の ACK の routing の hazard はこの rate・A の終わり方では見えない（ACK が streamer に属することの証明ではない） |
  | | A の終わりの ACK が FAIL に出た（`ACK at A's end`） | F1 の routing は本物：flush は ACK を **tick** で照合する（数では不可） |
  | | FAIL はあるが A の終わりの ACK は無い（1 つの T3 に ACK が 2 つ以上） | 原因は観測していない：FAIL の行を読む（F1 の routing を示すものではない） |
  | `P75 c decision` | 計画した 20 trial が揃っていない，device 呼び出しの失敗，ACK の無い trial | INCONCLUSIVE：欠けた ACK は「失われた」と読まない．待ちを延ばす推薦もしない |
  | | 全 PASS | 保持を確認した：100 ms の窓を過ぎても ACK は残る（この rate では）．窓を過ぎたことは「report が失われた」ではない（再試行の待ちを延ばせる） |

### (4) `hw_75_open_flush_sweep`（TX あり：4 sample の零，gain 0，約 2 分 / 台，estimate．期限 15 分）

問い（#75 の (3)）：`arm` の open 時の T3（`core.rs` `open_tx_device_locked`）の ACK は，1・2・10 MS/s の 1 channel・2 channel の送信 streamer で 100 ms 以内に来るか。owner の決定：(3) は**この掃引の 1・2 channel の cell を見たあと**に enable し，ACK が 1 度でも欠けたら owner に諮り直す。

- 1 trial：`tx_close`，`tx_open`（所要の wall 時間 = `open_ms`），T3（4 つの零，untimed の SOB+EOB，100 ms の期限で送り，期限内に全 channel 位置の ACK を読む = `flush_ms`）。期限を過ぎても欠けた位置は 400 ms 余分に待って late かどうかを出す。cell は rate 3 × channel 数 2，各 10 trial（1 channel は 3 cell，2 channel は 3 cell）。
- **2 channel の cell は slot B にも送信する．slot B の board を入れ替えたあとは，owner が slot B の board の TX/RX → ≥30 dB の att → 同じ board の RX2 を確かめ，そのときだけ `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 hw hw_75_open_flush_sweep` の形で 1 本の呼び出しに付けて流す**（`export` はしない：shell に残ると次の試験にも渡る．この節の `hw` は `-e` で container に渡す．`EZSDR_OFS_TWO_CHANNEL_WAIVED` も同じ）。付けないと，board が OBX／UBX でも cell は `NOT RUNNABLE: slot B's attenuator is not confirmed …` と出て slot B には送信しない（INCONCLUSIVE，`hw` は 0）。
- **2 channel の cell は現在の配線では走らない**。2 channel の streamer は slot A と B の両方が **OBX か UBX** のときだけ（1 GHz に同調できる）。#1 は OBX＋空の slot B（UHD は第 2 channel を未知の board として報告する），#2 は UBX＋CBX（CBX は 1.2 GHz 以上で `SAFE_TX_HZ` の外）。この 2 台では cell が `OFS cell … NOT RUNNABLE: <理由>` と出て INCONCLUSIVE（`not runnable`）になり，test は失敗しない（`hw` は 0）。走らせるには盤の入れ替え（電源を落として #2 の slot B の CBX を OBX にする，各 board に att を loopback で付ける）が要る．owner の決定事項（CBX を 1.2 GHz 以上で零だけ送ることを許すなら，`two_channel_runnable` を 1 行変える）。**2 channel の cell が NOT RUNNABLE のあいだ，全体の `OFS decision` は INCONCLUSIVE のまま**．owner が 2 channel の cell を免除したときだけ `EZSDR_OFS_TWO_CHANNEL_WAIVED=1` を付けて流す（その decision の行に免除と書かれる）。
- 出力：`OFS trace …`（trial ごと：ACK の tick と送信時の device 時刻との差，位置ごとの ACK，open と flush の時間），`OFS cell …`（全位置 ACK の trial 数，tick の差・open・flush の min / median / max），`VERDICT n75 ofs …`，`OFS decision`。`grep -h 'OFS\|VERDICT\|EZSDR-INFRA' <log>`。
- VERDICT：PASS は T3 が 4 sample 通り全位置が期限内に ACK，FAIL は欠けた・短い send・呼び出しの失敗，INCONCLUSIVE は tick の無い ACK か送信より前の tick の ACK（この flush のものと決められない．decision は INCONCLUSIVE になり enable しない）。
- **決めること**：`OFS decision` が「enable (3)」→ open 時の flush も確認された ACK だけで終える（#75 の (3) を enable。計画した cell と trial が揃い，2 channel の cell が走ったか owner が免除したときだけ出る．条件：rate・channel 数・board を書く）。「ask the owner again」→ 欠けた cell と trial を添えて owner へ。`INCONCLUSIVE`（2 channel の cell が NOT RUNNABLE で免除なし，または計画した trial の欠け）→ enable しない．走らなかった 2 channel の cell は `unmeasured` のまま（結果に書く）。

### 時間の合計（estimate）

| ブロック | 1 台 | 備考 |
|---|---|---|
| (1) `hw_67_stop_before_origin` | 約 5 分 | 受信のみ |
| (2) `hw_rxzeros_sweep` | 約 8 分 | trial 420，各 約 0.6〜1 s |
| (3) `hw_75_t3_probes` | 約 3 分 | trial 62（(a) の control は 11）+ 40 + 20 |
| (4) `hw_75_open_flush_sweep` | 約 2 分 | 1 channel の 3 cell だけ走る |
| 合計 | 約 18 分 | |

2 台で約 36 分．そのほか：ホスト確認と装置確認（「note 47 段階 5 の後」の (a)）約 10 分，初回の build（volume が冷たい）数分から十数分，att の確認と盤の入れ替え。**全体で約 1 時間（estimate）**。stop：`EZSDR-INFRA`（期限超過 = hang），`Control operation timed out`，`Reference Clock PLL failed to lock`，装置が応答しない → 電源を入れ直して owner へ。

### 閉じ方と記録

log は `~/.cache/ezsdr-bench/<日付>/<試験名>.<BOARD>.log`。bench の記録は `bench/<日付>-<topic>/`（`conditions.json`，`results/`（log の `RXZ trial`・`P75 … trace`・`OFS trace` の行から機械的に），`README.md`，`raw.txt`）と private repo の release（AGENTS §9）。orchestrator に返すもの：commit，各 log の path，`RXZ class`・`RXZ decision` の行（両 board），`RXZ trial` の dump（`head`），`P75 a/b/c cell` と `decision` の行，`OFS cell` と `OFS decision` の行，電源の入れ直しの有無，att の型番（owner の確認）。

## 2026-11-15 の手順（stage 4 の値のための計測：S2 throughput の繰り返し，T200，S5 の lead の rung，S4 の overflow gap）

出典：stage 4 の owner の決定（2026-11-13，D0〜D13：`~/.cache/ezsdr-fixes/2026-11-13-stage4/decisions.md`），その提案（同じ dir の `proposal.md` §3・§4），T200 の追加（`~/.cache/ezsdr-fixes/2026-11-15-bench/additions.md`），`crates/ezsdr-radio-uhd/tests/common/s4bench.rs`（判定・決定の行はここ．非 device の部分の unit test と，FakeDevice の rehearsal は default の suite）．**結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．** 決定の行は**観測した事実だけ**を言い，計画した証拠が揃わなければ `INCONCLUSIVE`．profile の値はこの日に変えない（4a は bench の後）．

### 流す項目と順（全体で約 4.5 時間の見込み，estimate．必須のあとに任意）

| # | 項目 | 1 台あたり | 要否 | TX |
|---|---|---|---|---|
| 0 | (a) host と装置の確認（「note 47 段階 5 の後」の (a)） | 約 10 分 | 必須 | なし |
| 1 | #67 の `hw_67_stop_before_origin`（下の CloseThenStop の cell を含む） | 約 5 分（cell が増えた分は未見積り） | 必須 | なし |
| 2 | S5 `hw_s5_lead_rungs` | 約 10〜15 分 | 必須 | あり（command lead の pass だけ） |
| 3 | S2 `hw_s2_throughput_repeats`（duplex 以外の 4 cell） | 約 45 分 | 必須 | あり |
| 3b | 同じ，duplex の 2 cell（5 回ずつ） | 約 10 分 | 任意 | あり |
| 4 | S4 `hw_s4_overflow_gap` | 約 30 分 | 必須 | なし |
| 5 | T200 の H0：bare UHD ×3，Module の probe ×3，record suite の形 ×3 | 約 50 分（9 run × 約 5.5 分） | #1 は必須，#2 は任意 | あり |
| 5b | T200 の H1（C-state を POLL/C1 に），H2（NIC の IRQ の向き先）：同じ 9 run | 各 約 50 分 | 任意（H1 は #1 で H0 に underrun が出たとき．H2 は H1 で差が無いとき） | あり |
| 5c | record suite の形を waveform 20 000 sample で ×3 | 約 17 分 | 任意 | あり |

- 必須だけの合計：(10 + 5 + 12 + 45 + 30) × 2 台 + T200 H0 の #1 50 分 + att の確認と盤の入れ替え約 15 分 ≒ **約 4.3 時間**．3b（+20 分）を足して約 4.6 時間．5b・5c と #2 の T200 は 5 時間に収まる範囲で．**owner 2026-11-14：8 時間まで流してよい → 任意も全部流す**：#2 で 0〜4・3b・T200 H0・5c，盤の移動，#1 で 0〜4・3b・T200 H0・H1・5c，H2 は H1 で差が無いときだけ（約 7.5〜8 時間）．時間は estimate で，初回の build（volume が冷たい，数分から十数分）は別．
- 順は **#2 → 盤の移動 → #1**（owner 2026-11-14 の盤の移動：上の節の「盤の移動」．#2 の T200（任意）は移す前に．#1 の T200 は移した後の slot A OBX）．board ごとに 0 → 1 → 2 → 3 → 4（S4 は TX なしなので，TX の待ち時間の合間に流してよい）．T200 は 3 のあと，host の設定（H0 → H1 → H2）ごとに board を固定して流す．
- **送信するのは 2・3・3b・5・5b・5c**．**送信する command ごとの前に owner が「その board の slot A の TX/RX → ≥30 dB の att → 同じ board の RX2」を確かめる**（毎回．command をまたいで続けて流さない．1 つの command の中の複数の run は，その command が最初に求める 1 回の `yes` で流す：`t200-bare-uhd.sh` の 3 run）．1・4 は受信だけで att は要らない．
- **slot B には送信しない**：どの項目も channel 0（slot A）にだけ送信し，周波数は 1 GHz（`SAFE_TX_HZ`），rate は 200 MS/s まで，gain は Provider の既定（0 dB）のまま触らない（`bench1115::MAX_TONE_GAIN_DB` = 20 dB の上限は，gain を変える probe の規則で，これらは gain を変えない）．`EZSDR_SLOT_B_ATTENUATOR_CONFIRMED` は**付けない**（S5 の 2 channel の pass は slot B で受信するだけ）．bare UHD は UHD 付属の `benchmark_rate` ではなく，`plan/phase8/tools/uhd-repro/tx_repro.cpp` の case `t200`（Ez-SDR のコードなし）で送る：自分で 1 GHz・gain 0 dB・slot A（`A:0`，`TX/RX`）を設定し，**送る直前に周波数と gain を読み戻して表示し，999.5〜1000.5 MHz の外か 20 dB 超なら送らず abort する**（`t200 RF read back: …` の行．`benchmark_rate` は周波数と gain の option を持たず，その値を確かめられなかったので替えた）．
- 各試験は device の一生全体を 1 本の期限で囲む（`bounded`．過ぎると `EZSDR-INFRA:` の行を書いて abort：hang の印．電源を入れ直して owner へ）．期限は計画した時間の 1.25 倍＋15 分（S5 は計画の見積り＋15 分，T200 の record の形は run 数 ×（T＋60 s）＋15 分）．
- **FAIL ではなく観測**：これらは device の失敗（overflow・underflow）を試験の失敗にしない．試験が失敗する（exit 1）のは，harness が続けられないとき（group の `Err`，handle の latch，streamer の取り残し：その盤は電源を入れ直し owner へ）だけ．`hw` の exit 0 は PASS ではない：**決定の行（`S2 decision`，`T200 decision`，`S5 decision`，`S4 decision`）を必ず読む**．

### 共通の準備（送信しない）

```sh
cd <COMMIT の tree>                      # git status が clean であること
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
hw() { # $1 = 試験名．EZSDR_UHD_ARGS と BOARD（36 か 37）と EZSDR_BENCH_NIC は呼ぶ前に機材ごとに export する．失敗なら非 0 を返す
  local dir=~/.cache/ezsdr-bench/$(date +%F) log status=0 n=1
  log="$dir/$1.$BOARD.log"
  while [[ -e $log ]]; do n=$((n + 1)); log="$dir/$1.$BOARD.$n.log"; done # 既存の log は上書きしない（.2，.3）
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" \
    --tmpfs /tmp:rw,size=${EZSDR_BENCH_TMPFS:-4g} ${EZSDR_BENCH_CPUSET:+--cpuset-cpus=$EZSDR_BENCH_CPUSET} \
    -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_BENCH_NIC \
    -e EZSDR_S2_TRIALS -e EZSDR_S2_DUPLEX_TRIALS -e EZSDR_S2_SECONDS -e EZSDR_S2_CELLS -e EZSDR_T200_RUNS \
    -e EZSDR_S4_TRIALS -e EZSDR_S4_STALL_RATE_HZ -e EZSDR_S4_STALLS_MS -e EZSDR_TX_WAVEFORM_SAMPLES -e EZSDR_TX200_LENGTHS -e EZSDR_TX200_SECONDS \
    -e EZSDR_MEASURE_THROUGHPUT_SECS -e EZSDR_MEASURE_STEP_TRIALS -e EZSDR_MEASURE_TIMED_LEADS_NS -e EZSDR_MEASURE_TIMED_LEAD_TX_RATES_HZ \
    -e EZSDR_MEASURE_RX_START_LEADS_NS -e EZSDR_MEASURE_RX_START_RATES_HZ -e EZSDR_MEASURE_RX_START_CHANNELS -e EZSDR_MEASURE_OVERFLOW_AFTER_MS \
    ezsdr-v4-dev:uhd4.10 cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$1" -- --exact --ignored --nocapture --test-threads=1 \
    >"$log" 2>&1 || status=$?
  grep -h 'VERDICT\|test result\|panicked\|EZSDR-INFRA' "$log"
  ((status == 0)) && [[ $(grep -c '^test result:' "$log") == 1 ]] && grep -q '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$log"
}
```

- 前の節の `hw` に足したもの：試験が読む環境変数（上の `-e`．**設定していない変数は渡らない**），`--tmpfs` の大きさ（既定 4 GB．**S4 の Run は受信 sample の file を /tmp に作る**：100 MS/s の stall 0.5 s の Run で約 1 GB，200 MS/s では約 2 GB，100 MS/s の stall 2 s の Run で約 2.2 GB の見込み（INFERRED：sink の capture の長さ `after + stall + 250 ms` × rate × 8 byte．Run ごとに消える）．S4 は `EZSDR_BENCH_TMPFS=12g` を付けて流す），`--cpuset-cpus`（H2 だけ）．
- host の確認と NIC の export は「note 47 段階 5 の後」の (a) と同じ（#2 `addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0`，#1 `addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1`）．
- 追加の環境変数（`MeasurementPlan::ENV_NAMES`，既定は record suite の値のまま．record suite を流すときは使わない）：

| 変数 | 意味 | 既定 |
|---|---|---|
| `EZSDR_MEASURE_TIMED_LEADS_NS` | command lead の rung（昇順，ns） | record：0.5/1/1.5/2/3/5/10 ms．S5：3.5/4/4.5/5 ms |
| `EZSDR_MEASURE_TIMED_LEAD_TX_RATES_HZ` | command lead の送信の rate | record：1 MS/s．S5：1/10/100 MS/s |
| `EZSDR_MEASURE_RX_START_LEADS_NS` | 受信 start lead の rung | record：3/5/10/25/50/100 ms．S5：3/4/5/7.5/10 ms |
| `EZSDR_MEASURE_RX_START_RATES_HZ` | 受信 start lead の rate | record：1 MS/s．S5：10/100 MS/s |
| `EZSDR_MEASURE_RX_START_CHANNELS` | 受信 stream の channel 数（1 か 2） | 1（S5 は両方を流す） |
| `EZSDR_MEASURE_STEP_TRIALS` | rung ごとの trial | 100 |
| `EZSDR_MEASURE_OVERFLOW_AFTER_MS` | 最初の block から stall までの ms | 500 |
| `EZSDR_MEASURE_THROUGHPUT_SECS`，`EZSDR_TX_WAVEFORM_SAMPLES` | throughput の Run の秒，送信の波形の sample 数（record suite の形） | 60，1 000 |
| `EZSDR_S2_TRIALS`，`EZSDR_S2_DUPLEX_TRIALS`，`EZSDR_S2_SECONDS`，`EZSDR_S2_CELLS` | S2 の回数（10），duplex の回数（5），1 Run の秒（60），流す cell の id（コンマ区切り） | 左の括弧 |
| `EZSDR_T200_RUNS` | T200 の record の形の回数 | 3 |
| `EZSDR_S4_TRIALS`，`EZSDR_S4_STALL_RATE_HZ`，`EZSDR_S4_STALLS_MS` | S4 の trial（20），stall の掃引の rate（100 MS/s）と stall（100,500,2000 ms） | 左の括弧 |

### 1. #67 の CloseThenStop の cell（受信のみ，#67 の step で追加．この step では足さない）

`hw_67_stop_before_origin` が cell **CloseThenStop**（origin の先の start を積み，close し，開き直し，untimed の stop，古い origin + 500 ms まで静かか）を持つ：受信のみ，期待 PASS，**両 board**，1・10・100 MS/s × 10 trial．流し方と読み方は「2026-11-15 の項目：#67」．

```sh
export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_67_stop_before_origin   # #1 OBX
export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_67_stop_before_origin   # #2 UBX
```

### 2. S5 `hw_s5_lead_rungs`（D3・D4．TX あり：command lead の pass だけ．約 10〜15 分 / 台，estimate．期限は計画 + 15 分）

問い：D3（`min_timed_command_lead_ns` = 5 ms）は TX 1・10・100 MS/s で 100/100 か，D4（`rx_start_lead_ns` = 10 ms）は 10・100 MS/s と 2 channel で 100/100 か．record suite と同じ group（`measure_min_timed_command_lead`，`measure_rx_start_lead`）を，rung と rate を変えて 3 つの pass で流す：(1) command lead：rung 3.5/4/4.5/5 ms × TX 1・10・100 MS/s × 100 trial（1 channel，slot A）；(2) 受信 start lead：`restart-same`・`restart-retune`（1 GHz をまたぐ）× rate 10・100 MS/s × rung 3/4/5/7.5/10 ms × 100 trial（1 channel）；(3) (2) を 2 channel で（receive のみ，layout の最初の 2 つの受信 channel が UBX か OBX のときだけ．CBX だと `not runnable` の行を出して何も流さない：任意）．

```sh
# --- owner が att を確かめた．次の 1 本だけ ---
hw hw_s5_lead_rungs
grep -h 'S5 decision\|S5 .* stage \|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_s5_lead_rungs.$BOARD.log   # 読む行（trial の行は `S5 <group> trial {json}`）
```

- 出力：trial ごとに `S5 command-lead trial {json}`・`S5 rx-start-lead-1ch trial {json}`・`S5 rx-start-lead-2ch trial {json}`（condition，rung_ns，class，late_command，start_lead_short，expected_origin，origin，first_tick，lead_left_ns…），stage ごとに `S5 <group> stage <condition> <rung> ms: k/n Success, …`，pass ごとに `S5 decision <group>`．
- 決定の行：各 condition で「その rung と，より大きい rung のすべてが全 trial Success の最小 rung」と，owner の決めた値（command lead 5 ms，受信 start lead 10 ms）が**全 condition で 100 %** か（そうでない condition の名前）．計画した stage か trial が欠ければ `INCONCLUSIVE`．
- 条件を超えて一般化しない：1 GHz，slot A，この 2 台（board ごとに別に読む），rate は上の 3 つと 2 つ，各 100 trial．`restart-retune` の stage は record の R1 で `START_LEAD_SHORT` の警告で fail した（proposal §0.1 #5）：fail の中身は trial の行の `late_command`・`start_lead_short` で見分ける．

### 3. S2 `hw_s2_throughput_repeats`（D1・D2．TX あり．約 45 分 / 台（duplex を除く 4 cell），estimate．期限は計画の 1.25 倍 + 15 分）

問い：D1（RX 200 MS/s が 1 本ごとに 60 s 通るか：1 台で 1 回でも落ちれば `rx_bytes_per_s` = 4e8），D2（TX 200 MS/s：「device が受け付けて鳴らす」を 10 s の窓で，「どれだけ保つか」を 60 s で，**両方を出す**）．record suite の `run_stage` で，1 channel の RX 100・200 MS/s，TX 100・200 MS/s を各 10 回 × 60 s，duplex（RX 200 + TX 100，RX 100 + TX 100）を各 5 回．cell の id は `rx-100msps`，`rx-200msps`，`tx-100msps`，`tx-200msps`，`duplex-rx200msps-tx100msps`，`duplex-rx100msps-tx100msps`．

```sh
# --- owner が att を確かめた．次の 1 本だけ（duplex を除く 4 cell）---
EZSDR_S2_CELLS=rx-100msps,rx-200msps,tx-100msps,tx-200msps hw hw_s2_throughput_repeats
# --- 任意（owner が att を確かめた．次の 1 本だけ）---
EZSDR_S2_CELLS=duplex-rx200msps-tx100msps,duplex-rx100msps-tx100msps hw hw_s2_throughput_repeats
grep -h 'S2 decision\|S2 stop\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_s2_throughput_repeats.$BOARD.log
grep -h 'S2 trial' ~/.cache/ezsdr-bench/$(date +%F)/hw_s2_throughput_repeats.$BOARD.log                      # trial ごと
```

（変数を前置きした `hw` は，その 1 回だけに渡る．`export` しない．）

- trial の行（`S2 trial {json}`）：cell，trial，outcome（`Success`／`DeviceFailure`…），受信と送信の sample 数と期待，`rx_overflow_events` と最初の時刻（T0 から ns），`tx_underflow_events` と最初の時刻，async の失敗．
- 決定の行：cell ごとに Success・failed・inconclusive の数，RX の overflow の数と最初の時刻，TX の「最初の 10 s に TX_UNDERFLOW が無い trial の数」と「60 s の全体が Success の trial の数」と最初の underflow の時刻（**両方を出す**），200 MS/s の cell には owner の規則（D1：RX 200 MS/s が 1 台で 1 回でも落ちれば 4e8．D2：上の読み）が今回の盤で発火したか．計画した trial が揃っていない，Incomplete の trial がある，TX の 10 s の窓が言えない trial がある（Run が 10 s より短い，**device が時刻なしで報告した underflow が窓の後に読まれた**：Provider はその event を読んだ時刻で記録するので，窓の中で起きたかどうかが分からない）と `INCONCLUSIVE`．時刻なしの報告の数は行（`tx_underflow_untimed_reports`）と決定の行に出る．窓の中で読まれた時刻なしの報告は，真の時刻がそれより前なので，窓の中の underflow として数える．
- D1 の「両 board で 10/10」は，2 台の決定の行を並べて読む（片方だけで決めない）．2 channel の 200+100 MS/s（1.2e9）は CBX が戻るまで**測れない**（今日は流さない）．
- 条件を超えて一般化しない：1 GHz，slot A，waveform 1 000 sample（`EZSDR_TX_WAVEFORM_SAMPLES`．T200 は waveform 20 000 でも流す），60 s，各 10 回，host は H0（`bench-host.sh` の出力を残す）．

### 4. S4 `hw_s4_overflow_gap`（D5．受信のみ，att 不要．約 30 分 / 台，estimate．期限は計画の 1.25 倍 + 15 分）

問い：overflow からの再開後の時間（restart-after-resume：record の INFERRED の高め側の推定）は rate 1/10/25/50/100/200 MS/s で安定か（owner の基準 (max − min)/max ≤ 0.5），最大は D5 の 140 ms を超えるか，stall の長さ（0.1/0.5/2 s，100 MS/s）に依存するか．各 cell は 20 trial．stall は record と同じ（1 MS/s 20 s，10 MS/s 2 s，25 MS/s 1 s，50・100・200 MS/s 0.5 s）．

```sh
EZSDR_BENCH_TMPFS=12g hw hw_s4_overflow_gap
grep -h 'S4 decision\|S4 stop\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_s4_overflow_gap.$BOARD.log
grep -h 'S4 trial' ~/.cache/ezsdr-bench/$(date +%F)/hw_s4_overflow_gap.$BOARD.log                            # trial ごと
```

- trial の行：rate，stall，outcome，overflow の数，gap（`gap_ns`：stall 中の損失も含む全体），`restart_after_resume_ns`（無ければ理由）．
- 決定の行：cell ごとに min・median・max と (max − min)/max と基準を満たしたか；`D5 bound`（rate の cell 全体の最大，整数 ms への切り上げ，140 ms 以下か超えるか）；`stall length at <sweep の rate> MS/s`（既定は 100 MS/s，`EZSDR_S4_STALL_RATE_HZ` で変えるとその rate．その rate の cell ごとの stall の median・max）．計画した 20 trial が complete（overflow がちょうど 1 回，gap が全部読めた，restart の時間がある）でない cell は `INCONCLUSIVE`．**100 MS/s の 0.1 s の stall は socket buffer（INFERRED で 50 MB）の fill 時間（約 125 ms）より短く，overflow しない（Incomplete → INCONCLUSIVE）かもしれない**：それ自体が結果で，stall の長さの行にそう出る．
- 条件を超えて一般化しない：1 channel，1 GHz，受信のみ，host は H0．**restart-after-resume は stall の始まりを推定で置く高め側の値で，RM-17 の定義（overrun の instant `f` から）ではない**：RM-17 の書き直し（4b）の入力で，値を決める前に design の step が先．

### 5. T200：200 MS/s の送信，bare UHD と Module（D2・KL-5．TX あり．約 50 分 / 設定 / 台，estimate）

問い：bare UHD が 200 MS/s を 300 s underrun 0 で送り，Module（同じ設定）が underflow するなら，Ez-SDR v4 の欠陥（Module を直す．issue）．両方 underflow なら host（その設定を `bench-host.sh --apply` と KL-5 へ）．record suite の 60 s の stage が落ち，probe が 300 s 通った理由（10-29 と R1・R2）も探す．**#1（OBX，slot A）が先**，#2 は任意．att は送信する command ごとの前に owner が確かめる（script の 1 回の呼び出し = 1 回の `yes`．その中の 3 run は続けて流れる）．各設定で 9 run（それぞれ 300 s，約 5.5 分）：

| | 何を | 流し方 | 何回 |
|---|---|---|---|
| ① | bare UHD | `plan/phase8/tools/t200-bare-uhd.sh "$EZSDR_UHD_ARGS" ~/.cache/ezsdr-bench/$(date +%F) <ラベル>`（container の中で `tx_repro.cpp` を build して `tx_repro --case t200 --rate 200e6 --secs 300 --spp 1996 --wave tone`．「yes」を求める．run ごとに 1 本の期限（300 s + 180 s．過ぎると container を kill し，残りの run は流さない）．Ctrl-C か TERM は container を kill して終わる（exit 130．以降の run は始まらない）．run ごとに clean / failed / INCONCLUSIVE の行） | 3 |
| ② | Module の probe | `EZSDR_TX200_LENGTHS=20000,20000,20000 EZSDR_TX200_SECONDS=300 hw hw_tx200_hot_path_probe`（波形 20 000，`run_stage`．1 回の呼び出しで 3 run） | 3 |
| ③ | record suite の形 | `EZSDR_MEASURE_THROUGHPUT_SECS=300 EZSDR_T200_RUNS=3 hw hw_t200_record_stage`（`measure_tx_throughput`：record の Spec，波形 1 000 sample，record の判定） | 3 |
| ③' | 任意（5c）：③ の波形を 20 000 に | `EZSDR_TX_WAVEFORM_SAMPLES=20000 EZSDR_MEASURE_THROUGHPUT_SECS=300 EZSDR_T200_RUNS=3 hw hw_t200_record_stage` | 3 |

- ③ と ② の違いは（i）波形の長さ（1 000 と 20 000），（ii）Run の作り方（`measure_tx_throughput` と `run_stage`：Spec，`CountingDevice`，判定の細部）．③' が ③ と ② の間：③' ≈ ② なら長さが原因，③' ≈ ③ なら Run の作り方．record suite の literal な 60 s は `EZSDR_MEASURE_THROUGHPUT_SECS` を省く（既定 60）．
- 出力：② は `TX200 len=… shape=single rate=200000000` の行と VERDICT（既存），③ は `T200 run {json}`（outcome，tx_samples，minimum_samples，`tx_underflow_events`，`first_underflow_ns_after_burst_start`）と `T200 decision (record-suite shape, 200 MS/s, <T> s, waveform <W> samples)`：Success・failed の run 数，run ごとの underflow の数と最初の時刻．①は `t200 RF read back: freq … gain …`（送る前），`T200 bare run n: clean`／`failed: <数>`／`INCONCLUSIVE: <理由>`（`t200 summary` の数も付く）と `T200 bare summary`．① の判定：underflow・underflow_in_packet・seq_error・seq_error_in_burst・time_error・送信 timeout がすべて 0 で，send に渡した sample が rate × 秒の 99 % 以上，tx_repro の exit が 0 なら clean．どれかの数が非 0 なら failed（数を並べる）．RF の読み戻しか summary の欄が欠ける，渡した sample が足りない，exit が非 0，期限切れ（`EZSDR-INFRA`）は INCONCLUSIVE．計画した run が揃わないと ③ と ① は `INCONCLUSIVE`．
- 先行の証拠：bench-results.md 2026-10-28（benchmark_rate 200 MS/s：普通の優先度 8 underrun，`--priority high` 0）・2026-10-29（#2：benchmark_rate 300 s underrun 0（spp 2000・10 000），spp 1000 は 5〜6；Module 波形 20 000 は TXS の前に約 150・約 122 s で underflow，TXS の後 300 s ×2 clean）・2026-11-06（600 s：6 回のうち 3 回が 1 underflow）．今日の結果をこれらと並べて読む．
- 条件を超えて一般化しない：1 GHz，slot A，gain は既定，この 2 台のうち流した盤，各設定 3 run．

#### host の設定（owner が sudo で変える．script は変えない．`bench-host.sh` が状態を記録する）

H0 = 今の状態（C1〜C3 有効，NIC の IRQ は 0〜15，adaptive coalescing）．設定を変える前後に `plan/phase8/tools/bench-host.sh --nic <NIC> --subnet <40|47>` を流して出力を残す（cstates と nic_irq_affinity の欄）．**戻し方を先に保存する．再起動でも戻る**．

```sh
# --- H1：idle state を POLL と C1 だけにする（owner，sudo）---
D=~/.cache/ezsdr-bench/$(date +%F); mkdir -p "$D"
for s in /sys/devices/system/cpu/cpu*/cpuidle/state*; do echo "$s $(cat $s/name) $(cat $s/disable)"; done > "$D/cstate-before.txt"   # 戻すための記録
for s in /sys/devices/system/cpu/cpu*/cpuidle/state*; do
  case $(cat "$s/name") in POLL|C1) ;; *) echo 1 | sudo tee "$s/disable" >/dev/null ;; esac
done
grep . /sys/devices/system/cpu/cpu0/cpuidle/state*/name /sys/devices/system/cpu/cpu0/cpuidle/state*/disable        # 確かめる
# --- H1 を戻す：保存した値に書き戻す ---
while read -r s name disable; do echo "$disable" | sudo tee "$s/disable" >/dev/null; done < "$D/cstate-before.txt"

# --- H2：NIC の IRQ を uhd-tx と別の core に固定する（H1 で差が無いときだけ．H1 の設定は戻さずに重ねてよい）---
NIC=enp1s0f1np1                                      # #1．#2 は enp1s0f0np0
IRQS=$(awk -v nic="$NIC" '$0 ~ nic { sub(":", "", $1); print $1 }' /proc/interrupts)
for i in $IRQS; do echo "$i $(cat /proc/irq/$i/smp_affinity_list)"; done > "$D/irq-before.txt"                      # 戻すための記録
systemctl is-active irqbalance                       # active なら，止めるか（sudo systemctl stop irqbalance），戻すとき start する
for i in $IRQS; do echo 0-3 | sudo tee /proc/irq/$i/smp_affinity_list >/dev/null; done                                 # NIC の IRQ を core 0〜3 に
export EZSDR_BENCH_CPUSET=4-15                       # container（Module と tx_repro の thread）を core 4〜15 に（docker --cpuset-cpus）
# --- H2 を戻す ---
unset EZSDR_BENCH_CPUSET
while read -r i affinity; do echo "$affinity" | sudo tee /proc/irq/$i/smp_affinity_list >/dev/null; done < "$D/irq-before.txt"
```

- H2 は「uhd-tx の core から NIC の IRQ を離す」を，core 0〜3（IRQ）と 4〜15（container）に分ける形で行う（owner の「IRQ affinity を 1 core から離す」より強い形．INFERRED で十分．uhd-tx の core を正確に知りたければ，run 中に `ps -eLo tid,psr,comm | grep uhd-tx` で読んで，その core だけを IRQ の集合から外してもよい）．16 core の host を前提：core の数が違えば範囲を変える．
- 設定ごとに run のあいだに host の他の負荷が無いこと（`uptime`）を確かめ，`bench-host.sh` の出力を ① ② ③ の log と並べて残す．
- **`irqbalance` が走っていると affinity は戻される**：H2 の前に `systemctl is-active irqbalance`．

### 流した後・stop・記録

- stop：`EZSDR-INFRA`，`Control operation timed out`，`Reference Clock PLL failed to lock`，`S2 stop`／`S4 stop`（handle の latch，streamer の取り残し）→ **電源を入れ直し owner へ**．T200 の ① で INCONCLUSIVE の run（RF 読み戻しの abort，summary の欠け，期限切れ）→ 装置を確かめ直す（期限切れは container を kill してあり，その script は以降の run を流さない．ほかに `docker ps` で `ezsdr-t200-bare-*` が残っていないかを見る）．
- orchestrator に返すもの：commit，各 log の path（`~/.cache/ezsdr-bench/<日付>/`），**決定の行**（`S2 decision`×cell，`S5 decision`×pass，`S4 decision`（cell ごと，`D5 bound`，`stall length`），`T200 decision`，`T200 bare summary`），2 台分，host の設定（H0/H1/H2）ごとの `bench-host.sh` の出力，電源の入れ直しの有無，att の型番（owner の確認）．raw の log は bench/ の規則（AGENTS §9）で private repo の release へ．
- 結果から決まること（orchestrator と owner が決める．**今日は値を変えない**）：D1（`rx_bytes_per_s` 8e8 の確認：#1・#2 の `rx-200msps` が 10/10 か．落ちれば 4e8），D2（TX 200 MS/s の admission：`tx-200msps` の 10 s の窓の figure と T200），D3（command lead 5 ms），D4（受信 start lead 10 ms），D5（gap の 140 ms と RM-17：4b の design の step への入力）．

## 11-15 の後の手順（保守表 3：遅れた device burst の cell C1〜C7．**11-15 には流さない**）

出典：保守表 3 の design（`~/.cache/ezsdr-fixes/2026-11-14-mt3/`：C1〜C6 は `fable-review.md` §2.4，C7 は `addendum.md` §2.4），owner の決定 O7（2026-11-14：bench は 11-15 の後．code は source で確かめた「遅れた device burst は device ごと捨てられる」規則で先に進める）．**道具は C1〜C6 が揃っている**（bench-tools の step `bt1`）：各 cell に `#[ignore]` の `hw_mt3_c*`（body は `tests/common/mt3.rs`・`mt3_cascade.rs`・`mt3_two.rs`・`mt3_tail.rs`），判定の純関数（手で組んだ evidence の単体 test つき），default の suite の FakeDevice の rehearsal（`mt3_c*_rehearsal_…`）．**C7 は bt2（Module 水準．production の hook が要る）で，まだ無い**：下の表の C7 は何を測るかの定義で，流す command も test の名前も無い．**結果は bench-results.md に書く（orchestrator）．書いていないことは「未確認」．**

### 問い（INFERRED のもの）

- 規則：X310 の radio は遅れた packet を 1 sample / tick で捨て，DUC は最初の時刻を burst の終わりまで運ぶので，最初の時刻つき packet が遅れた device burst は EOB まで全部捨てられ，BurstAck が来ない（FPGA の source で VERIFIED．H1 が #1 の 1 MS/s，1 回の 20 000-sample 送信で VERIFIED）．Module が送る形（chunk ごとに時刻つき，SOB と EOB），他の rate，#2，後ろに積んだ burst（cascade）は INFERRED．fake はこの規則に合わせて drain し，pad で device より厳しくしてある（UR-33：pad の各項は INFERRED で，C3・C4 が測る）．
- 結果の使い道：C1 が全 PASS なら Module の形でも device burst ごと捨てられる（この board・rate・条件で）．FAIL（一部が鳴った，または BurstAck が来た）なら規則の前提が崩れる：すぐ orchestrator へ（fake・UR-22 の abandon・UR-28 の retire が前提にしている）．INCONCLUSIVE は何も示さない：理由（受信の穴，control が聞こえない，TimeError が来ない）を直して流し直す．

### 流す項目

| cell | 形 | 測ること | 道具 | TX |
|---|---|---|---|---|
| C1 | raw UHD．Module の chunk の形（`segment::send_cap`：1 MS/s で 2 500，10 MS/s で 20 000 sample）の時刻つき send × 8（最初に SOB，最後に EOB，各 send に最初の sample の時刻）．**前に on-time の control の burst**（同じ形，聞く）．late burst は最初の sample の時刻を host の基準の時刻の 1 ms 前にし，host はその基準の時刻以後に最初の chunk を送る（最初の send は少なくとも 1 ms 遅れる．最後の chunk が自分の時刻の 1 ms 以上前に渡されたことを判定が確かめる）．1・10 MS/s × 10 trial | chunk の形でも device burst ごと捨てられるか | `hw_mt3_c1_chunked_late_burst`（書いてある） | あり |
| C2 | C1 の trial を 100・200 MS/s の送信で（**受信は 10 MS/s**，tone の窓と閾値は C1 のまま），late burst は 20 ms．#2 でも（200 MS/s では packet + 2 clock の間隔が残るか） | rate と board | `hw_mt3_c2_chunked_late_burst_high_rates` | あり |
| C3 | C1 の late burst の最初の send を 32 sample に（UR-22 の最小の最初の send），あとは Module の chunk で 20 ms．1・10 MS/s | DUC の出力 packet が短く始まるときの overhead（pad の `2 × packets`） | `hw_mt3_c3_short_first_send` | あり |
| C4 | cascade：A（C1 の late burst の形，20 ms，1 ms 遅れ）の最初の TimeError の tick を t0 とし，B（時刻つき，1 chunk，SOB+EOB）を `t0 + S × per + δ` に置く．δ は固定の格子（粗：−20・−10・50・100 µs，細：0〜30 µs を 1 µs 刻み，fake の limit とその +20 µs），各 δ 10 trial．1・10 MS/s | drain の終わりの境界，fake の pad が覆うか（UR-33，M3） | `hw_mt3_c4_cascade_boundary` | あり |
| C5 | 2 channel（slot A と B の 1 つの 2 channel streamer，受信も両方）で，開始が host の基準の時刻の ±数百 µs の burst | 片方の channel だけ鳴ることがあるか（RM-11 の「何も送られなかった」） | `hw_mt3_c5_two_channel_late_start`．**slot B にも送信する**：slot B の board と att を owner が確かめ，`EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1` を付けた 1 本だけ（11-15 の (4) と同じ規則．配線が無ければ `NOT RUNNABLE` で送信しない） | あり（slot A と B） |
| C6 | 保持した尾の close を 1 ms 遅らせる（Module の close：尾と零の 2 sample，SOB なし EOB，尾の時刻）．1・10 MS/s | Underflow，その後 TimeError，ACK なしか（M2） | `hw_mt3_c6_held_tail_starves` | あり |
| C7 | Module の水準（Session）で，`start_repeat` を開始で遅らせた場合と，途中で飢えさせた場合（`tests/fake.rs` の `Descheduled` を `UhdDevice` に）．m = 10・1・0.2 ms の掃引，1 cell ≥ 10 trial，**両 board** | [from, R) の無音，R で波形が位相連続で戻る，cap に達しない．τ（Module の道での host から radio への遅れ）を測るのは C7 だけ．そのあと owner が m を決める | **bt2，まだ無い**（production の hook が要る．repeat の再開の段階（段階 D）の後） | あり |

### C1 の流し方

- 条件：slot A（channel 0）にだけ送信する．1 GHz（`SAFE_TX_HZ`），gain 0 dB，振幅 0.4（0.5 以下．`tone_wave` が確かめる），rate は 1・10 MS/s，tone は搬送波から 100 kHz（他の tool と同じ．rate / 10 にすると 10 MS/s で 1.001 GHz になり `SAFE_TX_HZ` を出る）．**1 本の command ごとの前に，owner が「その board の slot A の TX/RX → ≥30 dB の att → 同じ board の RX2」を確かめる**（毎回．続けて流さない）．`pin_slot_a` が CBX と `SAFE_TX_HZ` の外を拒む．`EZSDR_D9_CHANNEL` は付けない（slot B には送信しない）．
- 板は H1 と同じ #1（OBX）を先に，次に #2（UBX）．`hw` は「`<DATE>` の手順（note 47 段階 5 の後）」の「共通の準備」の定義（この test は環境変数を足さない）．device の一生全体（開く・全 trial・teardown・失敗の unwind）を **15 分**の期限（`bounded`）で囲む：過ぎると `EZSDR-INFRA:` の行を書いて abort する（hang の印．電源を入れ直して owner へ．期待は約 3 分）．約 3 分 / 台（estimate：build の後．2 rate × 10 trial，1 trial は約 0.2 s 〜 1 s）．

  ```sh
  # --- owner が att を確かめた（#1 OBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_mt3_c1_chunked_late_burst
  # --- owner が att を確かめた（#2 UBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_mt3_c1_chunked_late_burst
  grep -h 'VERDICT\|MT3 C1\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c1_chunked_late_burst.$BOARD.log
  ```

- 1 trial：device の時刻 d0 を読み，受信を d0 + 20 ms に start，control の burst を d0 + 60 ms に（各 chunk は時刻の 10 ms 前に送る：Module の window），late burst を d0 + 130 ms の host の時刻（基準）に送る（最初の sample の時刻は 1 ms 前．間で report は読まない：静かな 20 ms の読み 3 回（control の report の後）だけで late の送信が 60 ms 以上遅れ，20 ms の burst の全 chunk が自分の時刻の後に渡されて，packet ごとに捨てる device でも全部捨てて PASS しうる）．report は全部最後に読む（tick で control と late に分ける）．受信は専用の thread が late burst の終わり + 2 chunk + 1 ms まで読む（送信を待たない）．最後に rx_stop と drain．
- **VERDICT**（`VERDICT mt3 c1 <rate> MS/s trial <i>: …`，定義は `mt3::late_chunked_verdict`．cell の行 `MT3 C1 cell <rate> MS/s: …` は全 trial の合成で，1 つの FAIL で FAIL，PASS でない trial があれば INCONCLUSIVE）：
  - FAIL（正の証拠．受信の欠けや control の有無に関わらない）：late burst の最初の sample 以後の tick の BurstAck（control の ACK は前の tick），または，loopback の delay（control の onset から trial ごとに測る．25 sample の粒度）を引いた聞こえた区間が late burst の区間と**重なる**（中にあるだけではない）．
  - INCONCLUSIVE：device 呼び出しの失敗，受信 block が late の区間 ± 1 chunk を穴なしで覆わない，その範囲に samples 以外の受信の結果（overflow など），control が聞こえない（90 % 以上の sample，onset が最初の sample から 1 chunk 以内）か control 自身が TimeError・Underflow，tick の無い report，late burst の最後の send が自分の時刻の 1 ms 以上前に渡されていない（渡した時刻を device の時刻で読む：packet に届いた時刻ではない．packet ごとに捨てる device なら，時刻に間に合った chunk は鳴るはずで，後ろの chunk が時刻の後に渡されると無音の説明にならない），late の区間に TimeError が無い（遅れていない：何も示さない）．
  - PASS：上のすべてが揃い，late の区間に何も聞こえず BurstAck も無く，TimeError が来た．
  - **この test は INCONCLUSIVE でも失敗する**（rehearsal と同じ）．`hw` の exit 0 は全 cell が PASS のとき．
- 聞こえた区間の判定は tone の形（25 sample の窓の coherence 0.9 超，周波数が 100 kHz の ±20 %，電力が 4 LSB の床より上．`tone_intervals`）．H1 の「sample の大きさ 0.001 超」ではない：FakeDevice の受信は送信の無い所で ramp を返し，大きさでは tone と区別できないため．**閾値は bench の前に実機で確かめていない**（INFERRED：C3 の quiet check と同じ床と窓．**10 MS/s では 1 sample の位相の進みが 0.063 rad と小さく，周波数の推定と 25 sample の窓の coherence が 1 MS/s より雑音に弱い**：INFERRED，C1 の 10 MS/s の trace で確かめる）．trial の `MT3 C1 trace {json}` に区間・受信の覆い・report の数が出る．
- **条件を超えて一般化しない**：1 GHz，この 2 rate，8 chunk，最初の send は少なくとも 1 ms 遅れ，1 cell 10 trial，この board．全 PASS でも「他の rate・board・cascade」は C2〜C7 が測る．

### C2〜C6 の共通（C1 の流し方に足すもの）

- 条件は C1 と同じ：slot A（channel 0）にだけ送信する（**C5 だけは slot B にも**），1 GHz（`SAFE_TX_HZ`），gain 0 dB，振幅 0.4，tone は搬送波から 100 kHz，`EZSDR_D9_CHANNEL` は付けない．**1 本の command ごとの前に，owner が「その board の slot A の TX/RX → ≥30 dB の att → 同じ board の RX2」を確かめる（毎回．続けて流さない．C5 は slot B の同じ配線も）**．**順と配線（owner 2026-11-14：UBX を #2 に戻してよい）**：11-15 の「盤の移動」の後の配線から始め，**#1 → 盤を戻す → #2** の順．11-15 が早く終わればその日に続けてよい．
  1. **#1**（slot A は OBX，slot B は移した UBX）：C1〜C4・C6（slot A だけに送信），C5（2 channel．この配線でだけ流せる）．
  2. **盤を戻す**：#1 と #2 の電源を落とし，UBX（32BC76F）を #1 の slot B から #2 の slot A へ戻す．#1 の slot B の loopback（att ごと）を外し，#2 に「slot A の TX/RX → ≥30 dB の att → 同じ UBX の RX2」を組み直す．電源を入れ，11-15 の (a) と同じ probe（送信なし）で #2 の slot A の UBX を確かめる．
  3. **#2**（slot A は UBX）：C1〜C4・C6．C5 は流さない（slot B が空）．
- 期限は device の一生全体で **15 分**（`bounded`．過ぎると `EZSDR-INFRA:` の行を書いて abort する：hang の印で，電源を入れ直して owner へ）．**INCONCLUSIVE でも test は失敗する**（`hw` の exit 0 は全 cell が PASS のとき）．
- C1 より後の cell の late burst は **どの rate でも 20 ms**（C1 の 8 chunk は 10 MS/s で 16 ms）：100・200 MS/s の host が実時間の 1.5 倍程度でしか送れなくても，最後の chunk が自分の時刻より前に渡されるようにするため（INFERRED：10 GbE で 200 MS/s は実時間の約 1.5 倍）．判定は C1 と同じく「最後の chunk が自分の時刻の 1 ms 以上前に渡された」ことを確かめ，足りなければ INCONCLUSIVE．
- `MT3 rates: transmit [...] receive [...] S/s (requested …)`：`apply` が返した実際の rate．要求と違えば trial は INCONCLUSIVE．
- `hw` は **「`<DATE>` の手順（note 47 段階 5 の後）」の「共通の準備」の定義**（`-e EZSDR_SLOT_B_ATTENUATOR_CONFIRMED` と `-e EZSDR_OFS_TWO_CHANNEL_WAIVED` を container に渡す．C5 に要る）．「2026-11-15 の手順（stage 4 の値のための計測…）」の「共通の準備」の `hw` はこの変数を渡さないので，**C5 を流す shell では後者を定義し直さない**（同じ shell に両方を貼ると後から貼った方が残る）．C2〜C4・C6 はどちらの定義でも流れる．

### C2 の流し方：C1 を 100・200 MS/s で（約 3 分 / 台，estimate．期限 15 分）

- 送信 100・200 MS/s，**受信 10 MS/s**（`Device::apply` で別に設定する．tone の窓と閾値は C1 のまま：INFERRED．evidence は受信の間隔を送信と別に持つ）．late burst は 20 ms（100 MS/s で 2 000 000 sample = Module の chunk 20 000 × 100 send，200 MS/s で 4 000 000 sample × 200 send）．1 cell 10 trial．
- **観測（判定の項ではない）**：late の TimeError の tick を全部，順に，隣との差，最頻の差，最頻の整数倍の差（欠けた report：FPGA の error FIFO は 32 entry で report を落としうる，R7）を出す：`MT3 C2 gaps <rate> MS/s trial <i>: …`．packet 長は UHD の streamer の値を Device が出さないので **INFERRED（最頻の差）**と明記される．late の report は burst の後にまとめて読むので，数千の report のうち古い tick が欠ける可能性がある（UHD 4.10 の送信の async queue は `boost::lockfree::queue` を 1 000 で作る：`rfnoc_tx_streamer.cpp:18,28`．容量は初期の node の数で，満杯で捨てるとは読めない：boost の仕様からの INFERRED）．判定には影響しない（新しい report だけを使う）．読むこと：200 MS/s でも差が packet + 2 clock のままか（Fable §2.1）．
- VERDICT：C1 と同じ（`VERDICT mt3 c2 <rate> MS/s trial <i>`，`MT3 C2 cell <rate> MS/s`）：FAIL＝late burst の最初の sample 以後の BurstAck，または loopback の delay を引いた聞こえた区間が late の区間と重なる；INCONCLUSIVE＝呼び出しの失敗（rate が要求と違う場合を含む），受信の穴，control が聞こえない，tick の無い report，最後の send が 1 ms 前に渡されていない，late の TimeError が無い；PASS＝すべて揃い無音・ACK なし・TimeError あり．

  ```sh
  # --- owner が att を確かめた（#1 OBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_mt3_c2_chunked_late_burst_high_rates
  # --- owner が att を確かめた（#2 UBX．盤の移動の前）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_mt3_c2_chunked_late_burst_high_rates
  grep -h 'VERDICT\|MT3 C2\|MT3 rates\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c2_chunked_late_burst_high_rates.$BOARD.log
  ```

### C3 の流し方：late burst の最初の send が 32 sample（約 3 分 / 台，estimate．期限 15 分）

- late burst だけ最初の send が 32 sample（UR-22 の最小の最初の send），あとは Module の chunk で 20 ms（control は chunk のまま）．1・10 MS/s，送受信の rate は同じ，1 cell 10 trial．
- **観測（判定の項ではない）**：C2 と同じ gaps に加えて，late の report の最初から最後までの tick の幅を **partial span**（最後の packet 自身の drain を含まない）と明記して `S × per` と並べる：`MT3 C3 gaps <rate> MS/s trial <i>: …`．fake の pad の `2 × packets` が表す overhead の見当．
- VERDICT：C1 と同じ（`VERDICT mt3 c3 …`，`MT3 C3 cell …`）．

  ```sh
  # --- owner が att を確かめた（#1 OBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_mt3_c3_short_first_send
  # --- owner が att を確かめた（#2 UBX．盤の移動の前）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_mt3_c3_short_first_send
  grep -h 'VERDICT\|MT3 C3\|MT3 rates\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c3_short_first_send.$BOARD.log
  ```

### C4 の流し方：cascade の境界（送信のみ．約 4〜6 分 / 台，estimate．期限 15 分）

- 1 trial：A（C1 の late burst の形：20 ms，最初の sample の時刻は host の基準の 1 ms 前）を送り，report を A の最初の TimeError まで読む．その tick が **t0**：radio が A の最初の packet を時刻検査で遅れと見つけたときの radio の時刻（`radio_tx_core.v` の `ST_TIME_CHECK`：VERIFIED）で，**host への到着の時刻でも A の timestamp でもない**．B（時刻つき，1 chunk，SOB+EOB が 1 つの send）を `t0 + S × per + δ`（sample の格子に切り上げ）に置き，すぐ渡す．受信はしない．
- δ は固定の格子（適応しない）：粗 −20・−10・50・100 µs，細 0〜30 µs を 1 µs 刻み，**fake の limit**（A に対する `drain_limit` − drain の終わり：1 MS/s で 256 µs，10 MS/s で 242 µs．test 側で fake と同じ式から計算し，rehearsal が fake の境界と一致することを確かめる），**limit + 不確かさ**（下の FAIL が始まる最初の δ．limit と +20 µs の間で device の drain が終わっても見落とさない），limit + 20 µs．各 δ 10 trial，δ を回す順に 1 周ずつ（host の drift が 1 つの δ に偏らない）．1・10 MS/s．
- 有効な trial だけを数える（INCONCLUSIVE の条件）：A の各 send（最初を除く）が drain に届く 1 ms 以上前に渡されている（drain は data が無いと待つので，host の stall が drain を延ばし，pad を host のせいにしてしまう），B が `t0 + S × per` の 2 ms 以上前に渡されている（**INFERRED**：B が drain の後ろに積まれたことの目安．転送の遅れは測っていない），A の send 数が計画どおり（fake の pad は send ごとに 1 packet），Underflow・UnderflowInPacket・sequence error の report が無い，tick の無い report が無い，B の最初の sample より前の BurstAck が無い．B は **played** ＝ B の最初の sample 以後の tick の BurstAck，**dropped** ＝ それが無く TimeError がある；どちらでもなければ INCONCLUSIVE．
- 出力：`MT3 C4 <rate> MS/s: the fake's limit …`，trial ごとに `MT3 C4 trial <rate> MS/s delta <µs> us #<i>: <Played|Dropped|Inconclusive> {trace}`（t0，b_first，格子に切り上げた後の δ，A の各 send の（自分の時刻，渡された時刻）と最小の margin），δ ごとに `MT3 C4 cell <rate> MS/s delta <µs> us: <n> played, <n> dropped, <n> inconclusive`，**`MT3 C4 boundary <rate> MS/s: last all-dropped delta …, first all-played delta …, mixed cells […]`**（device 自身の境界の bracket と，その間の混ざった cell．1 つの数ではない），`VERDICT mt3 c4 <rate> MS/s`．
- **VERDICT**（sweep）：**FAIL**＝**実効の δ**（B の時刻を sample の格子に切り上げた後の，`t0 + S × per` からの差．cell の名目の δ ではない）が fake の limit + 不確かさ（sample の間隔 + 4 master clock：t0 と fake の drain の始まりの食い違い）以上の trial で B が dropped（device が fake より後まで drain する＝UR-33 が崩れる．fake が緩い）；**INCONCLUSIVE**＝limit 以上の δ に decided（played か dropped）が 10 trial に満たない cell がある（または limit 以上の δ が無い）；それ以外 **PASS**（limit から不確かさの内の dropped は数えて text に出す．FAIL にしない）．**limit より下の δ は device 自身の境界の測定で，判定の項ではない**．
  - FAIL なら：その cell の trace を添えてすぐ orchestrator へ（fake の pad が device の drain を覆っていない）．

  ```sh
  # --- owner が att を確かめた（#1 OBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_mt3_c4_cascade_boundary
  # --- owner が att を確かめた（#2 UBX．盤の移動の前）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_mt3_c4_cascade_boundary
  grep -h 'VERDICT\|MT3 C4 \(cell\|boundary\|[0-9]\)\|MT3 rates\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c4_cascade_boundary.$BOARD.log
  ```

### C5 の流し方：2 channel の開始（**slot B にも送信する**．約 3 分，estimate．期限 15 分）

- **盤の移動の後の #1 だけ**（slot A に OBX，slot B に UBX）．owner が **slot B の board の TX/RX → ≥30 dB の att → 同じ board の RX2 も**確かめ，そのときだけ `EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 hw …` の形で 1 本の呼び出しに付ける（`export` はしない）．付けない，または slot A・B の片方が OBX／UBX でない，受信が 2 channel でないときは `MT3 C5 NOT RUNNABLE: <理由>` と出て，**何も送信せず**，test は INCONCLUSIVE として失敗する．
- 1 つの 2 channel の送信 streamer（channel 0・1）と 2 channel の受信．trial は C1 と同じ（受信を先に start，control を on time，late の burst）で，late の burst の最初の sample の時刻を host の基準の時刻 + δ にする．1 MS/s，burst 20 ms．channel ごとに evidence（report は channel で分け，tone も channel ごとに聞く）と C1 の判定を持つ．
- 2 つの cell：**clear**（基準の 1 ms 前，10 trial：両 channel とも C1 の判定が PASS でなければならない）と **marginal**（δ ∈ −200・−100・−50・−20・0・20・50・100・200 µs，各 10 trial．host の制御呼び出しの遅れが遷移の位置を決めるので格子は広い：INFERRED）．marginal の trial は channel ごとに **その channel の report から先に**読む：played＝late burst の最初の sample 以後の tick の BurstAck があり TimeError が無い；dropped＝TimeError があり ACK が無く，C1 の判定が PASS（無音・受信の覆い・control が聞こえた）；それ以外は inconclusive（tick の無い report，ACK と TimeError の両方，どちらも無い，C1 の判定が INCONCLUSIVE，**ACK が無いのに tone が聞こえた：slot A の tone の漏れか部分的な再生の疑い**）．tone だけで played とは読まない（漏れが本物の split を隠さないため）．
- 読み方（`MT3 C5 reading: …`，`MT3 C5 marginal <δ> us: …`，`MT3 C5 trial <δ> us #<i>: channel 0 …, channel 1 …`）：
  - **FAIL（SPLIT observed）**：ある trial で片方の channel が played，もう片方が dropped．**これは探していたもの**で，device の事実（RM-11 の「何も送られなかった」は「少なくとも 1 channel で」に直す必要がある）であって tool の欠陥ではない：その δ と trace を添えて orchestrator へ．clear cell が FAIL（明らかに遅い burst が鳴った）も FAIL．
  - **INCONCLUSIVE**：どの cell でも，ある trial が played の channel と「ACK が無いのに tone が聞こえた」channel の組を含む（漏れが隠した split の可能性：`possible split hidden by a leak`），clear cell が PASS でない，遷移をまたいでいない（両 channel とも dropped の δ より上に，両 channel とも played の δ が無い），遷移の隣の cell（最後の all-dropped の δ から最初の all-played の δ まで）の decided な trial が 10 に満たない．
  - **PASS**：split が無く，遷移をまたいでいる．「no split in N decided marginal trials (M run) at these conditions」（この rate・board・条件でだけ）．
  - **tone の漏れ**（INFERRED）：slot A の tone が slot B の受信に 4 LSB の床を超えて漏れると，その channel は「ACK が無いのに聞こえた」で inconclusive になる（played にも dropped にもしない）．漏れが続くと transition の隣の cell の decided が足りず INCONCLUSIVE になる．fake の channel は完全に分離しているので rehearsal では出ない．`MT3 C5 trace` の channel ごとの `BurstAck`・`TimeError` の数と `heard` で確かめる．
  - trial の行に **最初の late send が渡された時刻**（device の時刻を send の後に読む）と最初の sample の時刻の差が出る（`MT3 C5 trial …` と trace の `late_first_handed_and_own_time`）：host の制御呼び出しの遅れが transition のどこに来るかを読む（遷移が格子の外なら，この差で次の格子を決める）．PASS の行は，split が無かった decided な trial の数（run した数も括弧で）と，transition の隣の cell（最後の all-dropped から最初の all-played まで）の decided な trial の数を分けて書く．

  ```sh
  # --- owner が att を確かめた（#1：slot A の OBX と slot B の UBX の両方）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && EZSDR_SLOT_B_ATTENUATOR_CONFIRMED=1 hw hw_mt3_c5_two_channel_late_start
  grep -h 'VERDICT\|MT3 C5\|MT3 rates\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c5_two_channel_late_start.$BOARD.log
  ```

### C6 の流し方：保持した尾が飢える（送信のみ．約 1 分 / 台，estimate．期限 15 分）

- 1 trial：20 ms の burst の head（最初の `S − 1` sample を Module の chunk で，on time，最初の send に SOB，EOB なし）を送る．close は **Module の close**（`src/provider/tx.rs`：尾と零の 2 sample，SOB なし，EOB，**尾の時刻** = 最初の sample + `(S − 1)` sample）で，device の時刻が尾の時刻 + 1 ms を過ぎてから渡す（device が先に空になる．host の負荷はこの決まった保持で替える：close が遅れる理由に device の挙動は依らない）．**待った後の時刻を渡す前に読む**（渡した後の読みは「後に渡した」ことの証拠にならない）．1・10 MS/s，各 10 trial．
- VERDICT（`VERDICT mt3 c6 <rate> MS/s trial <i>`，`MT3 C6 cell`，`MT3 C6 trace`：全 report を順に）：**FAIL**＝burst の最初の sample 以後の tick の BurstAck，または report の順で最初の Underflow（か UnderflowInPacket）より前に TimeError，または TimeError の tick が最初の Underflow の tick より前．**INCONCLUSIVE**＝Underflow か TimeError のどちらかが無い（**両方が要る**．同じ channel で tick つき．Underflow と TimeError は burst の最初の sample 以後の tick のものだけ数える：前の trial の残りは数えない），**SeqError か SeqErrorInBurst の report が 1 つでもある（tick が無くても．UHD は転送 packet を失うと tick なしで出す．head がそれで飢えたのかもしれない）**，close が尾の時刻の 1 ms 以上後に渡されていない，head の最後の send が自分の時刻の 1 ms 以上前に渡されていない（head の host の stall は保持した尾ではない），head が早く飢えた（最初の Underflow の tick が head の最後の sample の時刻 − 1 chunk より前：host の stall であって保持した尾ではない），呼び出しの失敗，tick の無い report．**PASS**＝head の終わりに Underflow，その後に（順にも tick にも）TimeError，ACK なし．R4 が割り込まれた packet の残りを先に捨てて TimeError が複数出てもよい．

  ```sh
  # --- owner が att を確かめた（#1 OBX）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.40.36 BOARD=36 EZSDR_BENCH_NIC=enp1s0f1np1 && hw hw_mt3_c6_held_tail_starves
  # --- owner が att を確かめた（#2 UBX．盤の移動の前）．次の 1 本だけ ---
  export EZSDR_UHD_ARGS=addr=192.168.47.37 BOARD=37 EZSDR_BENCH_NIC=enp1s0f0np0 && hw hw_mt3_c6_held_tail_starves
  grep -h 'VERDICT\|MT3 C6\|MT3 rates\|EZSDR-INFRA' ~/.cache/ezsdr-bench/$(date +%F)/hw_mt3_c6_held_tail_starves.$BOARD.log
  ```

### C7：bt2，まだ無い

Module の水準で repeat の再開に m の掃引を入れる cell．production の hook が要るので bt2 で書く．道具が揃うまで流さない．

### 流した後・記録

- stop：`EZSDR-INFRA`，`Control operation timed out`，`Reference Clock PLL failed to lock`，装置が応答しない → 電源を入れ直して owner へ．
- orchestrator に返すもの：commit，各 log の path，`MT3 C1 cell` と `VERDICT mt3 c1` の行（2 台分），`MT3 C1 trace` の dump（`head`），C2〜C6 は各項の grep の行（C2・C3 は `MT3 C* gaps` を全部，C4 は `MT3 C4 cell`・`MT3 C4 boundary`・`VERDICT mt3 c4`，C5 は `MT3 C5 reading` と marginal の行，C6 は `MT3 C6 trace`），電源の入れ直しの有無，att の型番（owner の確認）．raw の log は bench/ の規則（AGENTS §9）で private repo の release へ．C2〜C6 は下の各項（C7 は bt2 の後）．
