STATUS: done

段階 TX200 の probe（test code のみ，main tree の作業ツリー，未 commit，production code・`design/`・`KNOWN_LIMITATIONS.md`・mutation row は触っていない）。実機は流していない。

## 変えたもの

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`
  - `TX_WAVEFORM_SAMPLES_ENV`（`EZSDR_TX_WAVEFORM_SAMPLES`）と `tx_waveform_samples()` を足した。usize，既定 1 000，parse 不能は clear な message で panic，0 も panic。
  - `measure_tx_throughput`（1 channel の throughput sweep）の `super::waveform_of(&super::pn(1_000))` を `...pn(tx_waveform_samples())` に。既定では record のまま。
- `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs`
  - `run_stage`（多 channel の throughput の段）の `common::transmit_waveform(1_000)` を `common::transmit_waveform(super::tx_waveform_samples())` に。env var は artifact の complex sample 数なので，「1 本の stream が全 channel を持つと channel あたり半分」という今の意味はそのまま（grouped 40 MS/s は 500 sample/send のまま）。
- `crates/ezsdr-radio-uhd/tests/hardware.rs`（`hw_measurements_record` の直後）
  - `hw_tx200_hot_path_probe`（`#[test]`，`#[ignore = "needs a USRP: see plan/phase7/bench.md"]`，GZ-8 の gate の形）と helper 3 つ `tx200_lengths`（`EZSDR_TX200_LENGTHS`，既定 `[1000, 2000, 20000, 200000]`），`tx200_seconds`（`EZSDR_TX200_SECONDS`，既定 30），`tx200_line`。
  - shape は `measurement::StreamPlan` / `measurement::Stage` を直に作り，record の throughput 群と同じ `measurement::run_stage` で流す：single = 1 stream `[ch0]` 200 MS/s，grouped = 1 stream `[ch0,ch1]` 40 MS/s，split = 2 streams 各 `[ch0]`/`[ch1]` 100 MS/s。grouped と split は selector（`EZSDR_MEASUREMENT_SELECTOR`）が 2 つの TX channel を名指ししたときだけ。`MeasurementPlan` は `throughput_seconds` と，その 3 つだけの rate 列を持つ（全部の sweep は入れない）。
  - Run の作り直しは無い：`run_stage` が Spec（`stage_spec`）・profile（`stage_profile`，TX では `bench_envelope`）・Assembly・Run を作り，波形は `transmit_waveform`（全 sample の大きさ ≤0.5 を assert）で，TX 周波数は各 channel の board 既定（record の多 channel の段と同じ helper，`channel_hz`）。probe 自身は毎 trial の前に `EZSDR_TX_WAVEFORM_SAMPLES` を set するだけ。
  - 各 trial は 1 行：`TX200 len=<n> shape=<single|grouped|split> rate=<hz> class=<TrialClass> underflows=<n> sends=<bursts[].blocks の和> samples=<stream ごとの actual_samples> first_failure=<code|none>`。observation から埋まらない field（`-`）と，失敗 code のある trial は，そのあとに observation JSON を compact で出す（failure の tick と channel は observation に保持されていないので，failure では JSON が必ず付く）。success/failure は assert しない。最後に「全 trial を走らせた」数だけ assert。
- 1 点だけ brief からの読み替え：single 200 MS/s も `measure_tx_throughput` ではなく `run_stage` で流す。`measure_tx_throughput` は sweep のあと必ず `extra_throughput`（多 channel の段）を流すので，rate を 200 MS/s 1 本に絞っても grouped/split 200 MS/s が 30 s ずつ余分に走り，probe の 3 shape と揃わない。`run_stage` は record の grouped/split と同じ Run の作りで，送信の hot path（同じ block 2 000，同じ waveform wrap の cap，同じ repeat burst）は single の sweep と同一なので，len の効果は同じように測れる。`measure_tx_throughput` 側の env var は record suite を長い波形で流し直すために残した。

## 実機のコマンド（X300 #2，addr=192.168.40.37）

送信するので，先に owner が **slot A の UBX と slot B の CBX の両方**で TX/RX → ≥30 dB att → 同じ board の RX2 を確認する（probe は UBX 200 MS/s と CBX 40/100 MS/s を送る）。selector は `bench_profile` が `selector_override` として読む（record と同じ経路，`EZSDR_MEASUREMENT_SELECTOR` だけを読むので record の 8 つの metadata は要らない）。

```sh
cd /home/komatsu/local_work/Ez-SDRv4
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
  --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
  -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e CARGO_HOME=/cargo-target/.cargo-home -e CARGO_TARGET_DIR=/cargo-target \
  -e EZSDR_UHD_ARGS=addr=192.168.40.37 \
  -e EZSDR_MEASUREMENT_SELECTOR='{"rx":["0:A:ubx:0","0:B:cbx:0"],"tx":["0:A:ubx:0","0:B:cbx:0"]}' \
  ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_tx200_hot_path_probe \
    -- --exact --ignored --nocapture \
  2>&1 | tee ~/.cache/ezsdr-bench/$(date +%F)/hw_tx200_hot_path_probe.log
```

- `-e CARGO_TARGET_DIR=/cargo-target` が要る：このツリーの untracked `.cargo/config.toml` が host の target dir を指すため（`run-record.sh` は `git archive` の新しい copy から build するので当たらない）。
- selector の `rx`/`tx` を省くと，#2 の layout は slot A の UBX 1 channel になり，single の 4 trial だけ走る。2 channel を測るときは必ず上の selector を渡す。
- 時間の目安：12 trial × （30 s + Run の準備 約 3.5 s）≈ **7 分**（+ device の open/describe 数秒）。build は共有 volume が温まっていれば 10 秒未満。短い試し流しは `-e EZSDR_TX200_LENGTHS=1000 -e EZSDR_TX200_SECONDS=10` で 3 trial ≈ 40 秒。`EZSDR_TX200_LENGTHS` の順は自由（与えた順に流す）。

## 読み方

- 期待（2026-10-28 の record）：len=1000 の single 200 MS/s は `class=DeviceFailure`，`underflows` > 0（4 回），grouped 40 MS/s は `DeviceFailure` で早期（約 93 ms）に 1 回，split 100 MS/s は成功。この 3 つが再現しなければ，30 s と 60 s の差か bench の状態が違い，A/B は成立しない。
- len を上げて single が `Success`/`underflows=0` になるなら，「波形が短い → 1 send が短い/毎 wrap で short send」が underflow の原因側で，sol の仮説 1 を支持する。200 000 でも `DeviceFailure` のままなら，波形長（per-send の大きさ）は原因ではない（仮説 2・3 の側）。
- `sends` は Provider が device へ出した send の数（`bursts[].blocks` の和）。len=1000 の single 200 MS/s・30 s なら約 6 000 000（1 000 sample/send），len=200 000 なら約 3 000 000（2 000 sample/send，wrap のときだけ短い）。
- `first_failure` は observation が持つ code だけ（tick と channel は observation に保持していない）。`json=` の `async_failures` の code 列で代える。最初の失敗の tick と channel はこの probe では出ない（brief どおり新しい配管は足していない）。

## build / test の出力

- host，`cargo check -p ezsdr-radio-uhd --tests`（`uhd` feature 無し。`hardware.rs` は `#![cfg(feature = "uhd")]` なのでこの check では中身を compile しない）:
  `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 2.57s`
- host，`cargo check -p ezsdr-radio-uhd --features uhd --tests` は build script で失敗（libuhd が無い）:
  `UR-35: the feature \`uhd\` needs libuhd: set UHD_LIB_DIR ...`
  ので以下は docker（`ezsdr-v4-dev:uhd4.10`）。
- docker，`cargo check -p ezsdr-radio-uhd --features uhd --tests`:
  `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 8.41s`（warning 0，dependencies 込みの最初の run），
  最後の edit のあとの再 run は `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in 0.26s`（package だけ re-check）
- docker，`cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware --no-run`:
  `Finished \`release\` profile [optimized] target(s) in 5.87s`
  `Executable tests/hardware.rs (/cargo-target/release/deps/hardware-5b8d97bcf7d2f4de)`
- docker，同じ binary の `-- --list`：`hw_tx200_hot_path_probe: test`（`hw_measurements_record: test`，`hw_txp_b0_send_sizes: test` と並ぶ）
- host，既定の経路が変わっていないこと，`cargo test -p ezsdr-radio-uhd --test fake`:
  `test result: ok. 346 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 74.77s`

`cargo fmt`/`rustfmt`，`mutate.py`/`remote_mutate.py` は流していない。commit もしていない。
