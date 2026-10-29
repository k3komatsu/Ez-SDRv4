STATUS: done

二つの小さな実機 probe（test code のみ，risk なし）を `crates/ezsdr-radio-uhd/tests/hardware.rs` に入れた．commit はしていない．

## 変更の要約

1. **slot の選択（`EZSDR_D9_CHANNEL`）**
   - `d9_channel()` を足した（`OnceLock` で 1 回だけ読む．既定 0 = slot A，1 = 2 board の X300 の slot B）．型は `usize`，数でなければ panic する．
   - `raw_rx` / `raw_tx` が `apply` と `rx_open` / `tx_open` に `d9_channel()` を渡す．これで P1〜P7 と D70 の stream 0 がその channel に開く（`tx_send` / `rx_start` / `tx_async` は stream 番号だけなので変更なし）．
   - `raw_uhd()` は channel が 0 より大きいときだけ，両 direction に `set_subdev_spec(dir, "A:0 B:0")` をして `channels(dir)` にその channel があることを assert する（production の Module も `from_binding` で同じ spec を設定する．1 board だけの装置では channel 1 がないので，そこで理由が出る）．channel 0 のときは従来とまったく同じ呼び出し列．
   - P1〜P7 は各 test の先頭で `let chan = d9_channel();` とし，`tune_dsp` / `tune_auto`（P4）/ P4 の close-reopen の `rx_open` を `chan` に揃えた．P1〜P3 も同じ helper を使うので，`EZSDR_D9_CHANNEL=1` のときは probe 全体が slot B で動く（open だけ 1 で tune が 0 という食い違いを作らない）．
   - **RF safety の assert は変えていない**：`raw_tx` の `SAFE_TX_HZ.contains(&applied.freq)`，各 probe の `SAFE_TX_HZ` assert はそのまま．

2. **`hw_d70_one_sample_burst_then_timed_burst`（#70 の直接 probe）**
   - P6/P7 と同じ 1 Msps・100 kHz・振幅 0.4 の tone と loopback を使う．
   - case A: `[0]` の 1 sample を untimed・SOB+EOB で送り，10 ms 待って TX の report を取る（#70 が名指す形）．case B（対照）: 同じ位置で `untimed_end_of_burst`（0 の 2 sample，untimed，SOB+EOB）．
   - そのあと `start = now + 150 ms`（sample 境界）に 30 ms の tone burst を timed・SOB+EOB で送り，`start + 40 ms` まで受信する．受信は `rx_at = now + 20 ms` から動かし，priming の前に 1 ms 以上待つので，時刻を失って「送った途端に鳴る」場合（lead の約 150 ms 前）も capture に入る．
   - 判定: `window_power`（`[rx_at + 1 ms, rx_at + 8 ms)` の 25-sample window の平均電力，10 window 未満なら `None`）を floor にし，`tone_start`（coherence > 0.5 かつ power > 4 × floor の 25-sample window が 3 連続する最初）を探す．`classify_tone` と同じ読み方で，最初に鳴った window の tick を `start` からの µs にして出す．
   - 出力（1 case 1 行）: `D70 <case>: tone starts at <Some(µs)|None> µs from its timed start; priming tx [...]; tx [...]; receive errors [...]`．成功の assert はなく，両 case が必ず走るだけ（診断）．

## 確かめたこと

- `docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target -w /work -e CARGO_HOME=/cargo-target/.cargo-home ezsdr-v4-dev:uhd4.10 cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware --no-run`：`Finished release`，`Executable tests/hardware.rs (…)`．
- 同じ image で `docker run … ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`：`Finished dev profile`，warning なし．
- `cargo test -q --workspace po_02`：`po_02_every_crate_forbids_unsafe_code` 1 本 ok，ほか 0 本（filtered），失敗なし．
- `cargo test -q -p ezsdr-acceptance --test governance gz_08`：1 本 ok（新しい test も `#[ignore = "needs a USRP: see plan/phase7/bench.md"]` の行を持つ）．
- `git diff --check`：clean．編集した file は mode 644．`cargo fmt` / `rustfmt` / `mutate.py` は使っていない．

## bench の実行コマンド（X300 #2 = 192.168.47.37，slot B = `EZSDR_D9_CHANNEL=1`）

`plan/phase8/bench.md` (c) の形（`--cap-add=SYS_NICE --ulimit rtprio=99`，main tree の target volume）．**送信するので，owner が att（slot B の TX/RX → ≥30 dB → 同じ board の RX2）を確認してから**．作業ツリーが他の agent に編集されていないことを確認する．

```sh
cd /home/komatsu/local_work/Ez-SDRv4
mkdir -p ~/.cache/ezsdr-bench/$(date +%F)
for t in hw_d70_one_sample_burst_then_timed_burst hw_d9_p4_ddc_entry_after_an_early_stop \
         hw_d9_p5_a_stale_timed_header_after_a_reopen hw_d9_p6_every_chunk_timed \
         hw_d9_p7_the_current_transmit_shape; do
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.47.37 -e EZSDR_D9_CHANNEL=1 \
    ezsdr-v4-dev:uhd4.10 \
    cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware $t -- --exact --ignored --nocapture \
    > ~/.cache/ezsdr-bench/$(date +%F)/$t.log 2>&1
done
```

P5 を case ごとに別 process で流すとき（bench.md (c) のとおり）:

```sh
for c in timed untimed reopen; do
  docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
    -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.47.37 -e EZSDR_D9_CHANNEL=1 -e EZSDR_D9_P5_CASE=$c \
    ezsdr-v4-dev:uhd4.10 \
    cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_d9_p5_a_stale_timed_header_after_a_reopen -- --exact --ignored --nocapture \
    > ~/.cache/ezsdr-bench/$(date +%F)/hw_d9_p5_$c.log 2>&1
done
```

slot A（UBX）の対照は `-e EZSDR_D9_CHANNEL=1` を外して同じ loop を流す（`EZSDR_D9_CHANNEL` の既定は 0）．D70 だけを 1 本流すなら:

```sh
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS=addr=192.168.47.37 -e EZSDR_D9_CHANNEL=1 \
  ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_d70_one_sample_burst_then_timed_burst -- --exact --ignored --nocapture \
  > ~/.cache/ezsdr-bench/$(date +%F)/hw_d70_one_sample_burst_then_timed_burst.log 2>&1
```

## 注意（orchestrator の判断が要る点）

- この probe は 1 GHz 固定である．X300 #2 の slot B は **CBX**（serial 329AACB）で，CBX の範囲は **1.2〜6 GHz**（`design/18-uhd-radio.md` UR-9，`cbx_freq_range` VERIFIED）．`raw_tx` は `apply` の read-back に `SAFE_TX_HZ` を要求するので，slot B の CBX では 1 GHz の tune が 1.2 GHz に clip され（INFERRED），**probe は送信の前に assert で止まる**可能性が高い．slot B で流すには slot B が UBX/OBX であるか，probe の周波数を CBX の範囲に上げる変更（この brief の外）が要る．1 GHz のまま CBX で流すと，`D70` も P4〜P7 も最初の `raw_tx` で落ちて log に理由だけが残る見込み．
- 変更は `crates/ezsdr-radio-uhd/tests/hardware.rs` のみ．`plan/phase8/bench-results.md` の既存の未 commit の差分（2026-10-29 の記録）には触れていない．
- 自己レビューはしていない（brief のとおり）．
