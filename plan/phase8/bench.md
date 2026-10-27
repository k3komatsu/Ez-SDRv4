# Phase 8 — 実機 bench 手順

Phase 8 の BP0〜BP5 を実行する手順．shake-out の観測は診断用で profile の値には使わない．完了した BP2/BP3 の測定記録は段階4の profile 判断の入力になり，`measurements/` に置いた記録を比較テストが読む（仕様 [41 §2, §4](41-measurement-records.md)）．実機で失敗しても bench 中にコードを直さず，log と `.partial.json` を保存して次の修正段階へ戻る．

## 機材と RF 安全

- 対象は X300 #1（serial `347D545`，slot A に OBX，`addr=192.168.40.36`）と #2（serial `347D541`，slot A に UBX 等，`addr=192.168.40.37`）．NIC はそれぞれ `enp1s0f0np0`・`enp1s0f1np1`．実際の配線と serial を probe で確認する．同時接続は同じ `/24` の経路が重なるため，必要な `/32` route を owner が用意していない限り一台ずつ使う．
- 送信する全ての段で，該当 board の **TX/RX → 30 dB 以上・18 GHz 定格の attenuator → 同じ board の RX2** を接続する．att は最大 TX 出力に対して十分な最大入力電力定格と余裕があることを銘板・仕様で確認し，型番と電力定格を記録する．アンテナはつながない．この日の装置では 40 dB attenuator を使用した．gain 0 dB，波形振幅 0.5 以下，通常の loopback は 1 GHz を使う．
- coercion の TX gain sweep は **streamer を開かず，LO を 1 GHz にした状態で最大 gain まで**試す．coercion の TX frequency read-back は `SAFE_TX_HZ` の範囲で行う．これは別の 3c-2c frequency-grid 測定（全周波数範囲，gain 0，sample を送らない）と混同しない．いずれの送信条件も上記 attenuator が必要で，RF safety の例外ではない（仕様 41 §8）．
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
4. **BP2**：#1＋OBX で記録 suite を負荷なしで流す．次に H4 の負荷を実際に動かした run を行い，metadata の `load=h4` を使う．`run-record.sh` 自体は H4 負荷を生成しない．
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
7. **BP4b**：混在 board，channel ごとに異なる設定，rate の違う stream，aligned stream，同時運転を試す．単独 stream の測定で同時運転の性能を主張しない．段階 3c-3a の道具：`EZSDR_MEASUREMENT_SELECTOR`（layout の JSON．例 `'{"rx":["0:A:obx:0","0:B:cbx:0"],"tx":["0:A:obx:0","0:B:cbx:0"]}'`．key は `rx`，`tx`，`image`，`links`，`link_gbps` だけで，`rx` と `tx` は両方か，どちらも無し（片方だけだと残りを装置の layout から引き継ぎ，att の確認が送信する channel を覆わないので，装置を開く前に拒否する），後の 3 つは 1 要素の list．layout が装置に合うかは Provider が UR-5・RM-28 で判定し，合わなければ最初の Run でその群の失敗になる）を渡し．両方向に channel があれば，受信と送信を同時に流す条件（throughput の容量に入る）は既定で走る．lead と overflow の gap には，相手の向きを 10 MS/s で流した条件を，それぞれ別の群（値の key なし）として足す（observation だけで，値の行は決めない．失敗してもその群だけの失敗）．2 channel 以上の向きは 1 本の stream に全 channel，channel ごとに 1 本，rate を変えた channel ごとの 1 本（channel 0 は格子の高い方，channel 1 はその次に低い格子）の条件を流す．合計の bytes/s は，適用できる全部の条件で，その値以下のすべての段が成功した最大．record の raw probe は slot A（device channel 0，1 GHz）を動かすので，selector の rx・tx の channel 0 が slot A の UBX か OBX（または未指定）のときだけ走り，それ以外では各 probe が「この layout に当てはまらない」という observation になり，装置には触れない．`run-record.sh` の att の確認は，selector の `tx` の全 channel と，raw probe が走るとき，または preflight の `hw_*` を指定したときの slot A の board（preflight は selector を受けず slot A を動かす）を名指しし，1 回の `yes` で各 board の TX/RX → ≥30 dB att → 同じ board の RX2 を確かめる（Basic/LF の TX は繋いだ先の RX）．所要時間の目安：1 段 = `EZSDR_MEASURE_THROUGHPUT_SECS`（既定 60 s）．2 channel の向きは (a) 7 段，(b) 7 段，(c) 6 段，duplex は 7 段で，合計約 27 段（約 27 分，RX と TX で 2 倍）．1 channel でも両方向があれば duplex の段（7 段）が足される．
8. **BP4c**：XG image と 10 GbE 2本で throughput，再 open，同時 RX/TX を測る．FPGA image を書き換える前に owner の確認を得る．`EZSDR_UHD_ARGS` に `addr=…,second_addr=…`，`EZSDR_MEASUREMENT_SELECTOR` に `"image":["xg"],"links":[2],"link_gbps":[10]`，`EZSDR_MEASUREMENT_NICS` に使う NIC を空白区切りで渡す（各試行の前後で `/sys/class/net/<nic>/statistics/{rx,tx}_bytes` の差を observation に残す．stream の番号は物理の link ではない）．候補の天井は link 1 本分（10 GbE で 1 250 MB/s）で，2 本の合計より上は試さない（8c の範囲）．再 open と H4 は 3c-3b．

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

必ず `hw_measurements_record` を実行する．任意の `hw_*` test はその前の preflight として実行され，失敗したら record suite は開始しない．8つの必須 metadata は，output（自動で `/bench/measurement.json`），mboard と FPGA image（probe から。probe できない値は `EZSDR_MEASUREMENT_MBOARD` / `EZSDR_MEASUREMENT_FPGA_IMAGE` で指定），host name（既定は `hostname -s`），settings script / version（bench-host の path と出力），load（既定 `none`，`none` または `h4`），commit（snapshot の完全 hash）で組み立てる．`EZSDR_UHD_ARGS` には指定した UHD args を渡す．試行回数等は `EZSDR_MEASURE_STEP_TRIALS`，`EZSDR_MEASURE_OVERFLOW_TRIALS`，`EZSDR_MEASURE_THROUGHPUT_SECS`，`EZSDR_MEASURE_LOOPBACK_TRIALS`，`EZSDR_MEASURE_QUEUE_TRIALS` で調整できる（既定値は measurement code を参照）．

出力には `build.log`，`probe.log`，preflight test ごとの log，`hw_measurements_record.log`，および `measurement.json` がある．3c-2c の途中経過 `<OUTPUT>.partial.json` は診断専用で，比較テストは読まない．各 group 後に上書きされ，final record が書けた後に削除される．中断時に残った partial は削除・改名せず log と一緒に保管する（再開可能な record ではない）．標準の suite は 30〜40 分見込み（INFERRED）なので，log はファイルに書いてから必要箇所を確認する．

## 既知の失敗と結果の扱い

- **DDC command buffer の詰まり**：受信 sample が流れていない chain に timed frequency command を積むと，DDC は sample packet の time が command の時刻に達するまで timed FIFO から取り出さない．UHD log に `Control operation timed out waiting for space in command buffer` が繰り返し出るのが症状．この状態で再試行，frequency reset，coercion を続けない．partial と log を保存して止め，装置が反応しなければ X300 の電源を切って入れ直し，probe と `ref_locked` をやり直す（設計メモ [39 §1](39-dsp-timed-commands.md)）．再起動が必要になる場合がある．
- **UR-7 PLL lock**：`Reference Clock PLL failed to lock to internal source` が出たら，その open は測定として数えない．決められた retry を消費した後は log を残して停止し，owner が電源を入れ直してから `uhd_usrp_probe` と `ref_locked` を再確認する．2026-10-27 に電源 cycle 後 `ref_locked=true` を観測したが，それは将来の成功を保証しない．
- **`receive Run has no T0`**：throughput の Run が admission/prepare で拒否され T0 が無い場合は，その rate を成功値として扱わない．run の termination と理由を残す．shake-out 1 では profile に 10 GbE link の宣言がなく，受信 throughput が拒否された；`8f4f060` で link を宣言した．
- **失敗は記録**：2026-10-27 の手動 shake-out log は実際に `~/.cache/ezsdr-bench/2026-10-27/` にある．shake-out log / partial は比較 test が読む `measurements/` に入れない．`run-record.sh` を使う記録は上記の `$HOME/ezsdr-bench/<日付>/` に出力する．段階4は複数台・負荷あり・必要回数の記録から値を決め，測れない値は未測定 / INFERRED のままにする．
