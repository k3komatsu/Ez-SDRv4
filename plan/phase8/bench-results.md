# Phase 8 — bench の記録

計画は [00-overview.md](00-overview.md) §6（BP0〜BP5）．Phase 7 の記録は [../phase7/bench-results.md](../phase7/bench-results.md)．実行の形は handoff §6．

## 2026-10-20：BP0 と BP5（X300 2 台）

### 機材とホスト

| | X300 #1 | X300 #2 |
|---|---|---|
| serial | 347D545 | 347D541 |
| `addr` | 192.168.40.36 | 192.168.40.37 |
| FPGA | UHD 4.10 の HG（39.3，git d375d68） | 同じ．元は UHD 4.9 の image（git c37b318）で，image の UHD 4.10 が `Major compat number mismatch for 0/Radio#0: Expecting 1, got 0` で開けなかった．owner の許可で `uhd_image_loader`（image の中）で書き換え，電源を入れ直した |
| slot A / B | OBX / 空 | UBX-160 v2（serial 32BC76F）/ CBX（serial 329AACB） |
| 配線 | OBX の TX/RX → 30 dB → RX2 | UBX の TX/RX → 30 dB → RX2（CBX は未配線） |
| ホストの port | `enp1s0f0np0` | `enp1s0f1np1` |

- 2 台は同時につながない（owner）．同じ 192.168.40.x なので，同時につなぐならホストに `/32` の経路を足す．
- ホストの設定は tree の外の `~/ezsdr-bench/usrp-net.sh`（ibfd の `set_addr.sh` を縮めたもの．owner が sudo で流す．再起動で戻る）：リンクのある `enp1s0f*` を NetworkManager から外し，MTU 9000，使ったことのある subnet（192.168.10/30/38/40/44）に `.10`，リングを上限，PCIe ASPM を切り，`wmem_max` 32 MiB・`netdev_max_backlog` 30000．リンクの無い port のアドレスは外す．段階 3 でこのリポジトリの script にする．`Maximum frame size: 8000 bytes` は 2 台とも．

### BP0：ループバック

UHD 4.10 の例 `txrx_loopback_to_file`，slot A，1 GHz，1 Msps，利得 0，正弦波 100 kHz・振幅 0.3，1 s．

| | 音 | 鏡像 | 送信なし |
|---|---|---|---|
| #2 UBX | −44.5 dBFS | −85.5 dBFS | −129 dBFS |
| #1 OBX | −45.6 dBFS | −89.8 dBFS | — |

UBX は送受信とも動く（「壊れているかも」は否定）．CBX は未確認（1.2 GHz 以上，配線なし）．

### BP5：Phase 7 の hardware テスト

`crates/ezsdr-radio-uhd/tests/hardware.rs` の 29 本（抜線の B9 と USRP2 を除く全部），commit の写しを tree の外に置いて image の中で build（`~/ezsdr-bench/run-bench.sh <commit> <args>`，target は volume `ezsdr-v4-bench-target`）．

| commit | 装置 | 結果 |
|---|---|---|
| `5396b39` | #2 | B1 が OBX を要求して落ち，B3・B4 は UR-5 が UBX と CBX の layout を拒否（「one device carries one kind」）．テストの `layout_of` が装置の全部の盤を layout に入れていた．`e671889` で slot A の盤と同じ種類だけにした |
| `e671889` | #2 | 24/29．4 本（`hw_b8_leads`・`stop_end`・`burst_at_a_sent_burst_s_end`・`burst_one_sample_after_a_burst`）は送信の clock の origin より前に burst を置いた（spec 22 VH-4 から 0 channel からの enable も `tx_start_lead_ns`＝50 ms を待つ．Phase 7 は約 4 ms）．1 本（`hw_b6_delay_by_rate`）は ref PLL の lock 失敗（UR-7）．`8fab132` で `past_tx_origin` を通し，`DeviceAuthority` を 1 回やり直す |
| `8fab132` | #2 | 落ちた 5 本と同じ helper の 2 本：7/7 |
| `8fab132` | #1 | **29/29** |

ref PLL の lock 失敗は最初の回で約 35 回の open のうち 2 回．やり直しはまだ一度も働いていない（最初の失敗は出力されずに捨てられる）．

### 数値（Phase 7 は #1＋OBX）

| 項目 | #1＋OBX | #2＋UBX | Phase 7 |
|---|---|---|---|
| `hw_b8_leads`：届く最小の lead | 3 ms（2 ms は 0.44 ms 遅れで drop） | 3 ms（2 ms は 0.51 ms） | 5 ms（3 ms は 0.035 ms 遅れ，2 ms は 1.20 ms） |
| `hw_b8_stop_end`：Stop の提出から音が止むまで | 10.27〜10.40 ms | 10.25〜11.29 ms | 10.29〜11.35 ms |
| `hw_b6_delay_by_rate`（400k・500k・1M・2M） | 35・44・44・36 sample | 同じ | 35・44・44 |
| `hw_perf_rx_rates`：10〜200 Msps | overflow 0 | overflow 0 | overflow 0 |
| `hw_b9_long_receive`：10 Msps 120 s | overflow 0，DEVICE_LOST なし | 同じ（12 億 sample） | — |
| `hw_b8_raw_timed_tune_phase`：1000.1234 MHz の 6 回の位相の幅 | timed 0.1°，untimed 161° | timed 0.9°，untimed 180° | timed 0.0°，untimed 142° |
| `hw_b8_preemption` | 20 ms は通り，10・5・3 ms は drop | — | 同じ形 |

- Stop は UR-23 の `s + 10 ms`（uhd-tx が受け取った時点から）に，提出からの delivery（Phase 7 で ≤ 1.4 ms）を足した範囲．
- 1b-4a〜4d の組み直しで悪くなった数値は無い．同じ #1 で lead は少し良い（3 ms が通る）．
- 2 台の motherboard の値（lead，Stop，遅れ，throughput）は揺れの中で一致する（P5 (1) の「悪い方」はこの範囲で変わらない）．盤の差は timed tune の位相の幅だけ（UBX 0.9°，OBX 0.1°）で，どちらも timed tune で位相が決まる側．

### 見ていないもの

複数 stream の layout（2 本の受信 streamer，H1），CBX，混在構成（BP4b），XG と 10 GbE 2 本（BP4c），抜線（B9）．
