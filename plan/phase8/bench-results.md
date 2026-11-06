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

## 2026-10-21：front end を streamer から streamer へ移す（1b-4e-2 の INFERRED，X300 #1＋OBX）

`hw_m1_rx_front_end_moves_between_streamers`・`hw_m2_tx_front_end_moves_between_streamers`（`crates/ezsdr-radio-uhd/tests/hardware.rs`），UHD 4.10 の image，1 GHz・1 Msps・gain 0・振幅 0.3，30 dB の loopback．ログは `~/ezsdr-bench/dd4746a-36/`，失敗した版は `~/ezsdr-bench/7b71f8f-36/`．

| 系列 | 結果 |
|---|---|
| 受信：stream 0 で受けて止め，`rx_close(0)`，同じ front end を stream 1 で開いて timed start | stream 1 の最初の block は指定の tick ちょうど，500 ms・250 block に隙間も overflow もない．**VERIFIED** |
| 送信：stream 0 で burst，`tx_close(0)`，stream 1 で開いて burst（受信は通して読む） | 両方 `BurstAck`．loopback の rms は 0.00670（s0）と 0.00670（s1），静かなとき 0.000085．**VERIFIED** |
| 1 本の streamer が front end を持ったまま，別の stream で同じ front end を開く | 拒否される（`UHD error 47`，`lost: false`．UHD のログは "Attempting to reconnect output port 0/DDC#0:0"・"input port 0/DUC#0:0"．`uhd_usrp_last_error` の文は空）．**ただし拒否で済まない**：受信では，持っている側の streamer のその後の stop が "Cannot forward action stream_cmd … no neighbour found"（試みが Radio→DDC の edge を外した），受信・送信とも後で `std::out_of_range`（`map::at`）でプロセスが abort（SIGABRT）．**VERIFIED** |

- 1b-4e-2 の設計の前提（閉じた streamer の front end は別の stream の streamer で開ける）は実機でも成り立つ．
- FakeDevice は 2 本目の open を「拒否して，持っている streamer はそのまま」とまねるが，実機では持っている streamer を壊し，プロセスを落とす．Module はその open をしない設計（待ちと from-none の委譲）なので今の挙動は変わらない．同じ日に FakeDevice を直した：その open は拒否したうえで fake の drop でテストを失敗させる（UR-33，`ur_33_an_open_of_a_held_front_end_fails_the_test_at_drop`）．全テストで踏むものは無かった．

## 2026-10-27：shake-out（X300 #1＋OBX，結果は profile に使わない）

### 機材と配線

X300 #1，serial `347D545`，slot A OBX，slot B 空，UHD 4.10 の HG image（FPGA `39.3`，git `d375d68`），`addr=192.168.40.36`，10 GbE NIC `enp1s0f0np0`．master clock 200 MHz，最大 frame size 8000 bytes．40 dB / 18 GHz attenuator を TX/RX → RX2 に接続した．att の型番と最大入力電力は記録に残っていない（次の送信前に確認する）．probe は RX/TX とも 2 channel と報告したが，slot B は `Unknown (0xffff)`；OBX の実使用 channel は 1 本．記録は `~/.cache/ezsdr-bench/2026-10-27/` の `probe.txt` と各 `shakeout-*.log` にある．この shake-out 記録を `measurements/` に置かない．

### 記録済みの 1〜3 回目

| 回 | 結果 | 対応・範囲 |
|---|---|---|
| 1 | 約 1118 s 後，受信 throughput の `Run` に T0 がなく失敗．profile が HG の 10 GbE link を宣言せず，rate が admission で拒否されていた． | `8f4f060` で link を `link_gbps=[10]` と宣言．その後の shake-out で同じ箇所まで進めることを確認した．最終 record は未作成． |
| 2 | DDC の `Control operation timed out waiting for space in command buffer` が繰り返し，記録 suite が完了しなかった．shake-out 2 log で最初に失敗した request は timestamp 付きの address `0x00420`；その後 `0x00430` の request も失敗した． | log には `0x00420` が 60 request，後続の timestamp なし `0x00430` が 360 request ある．受信 sample が来るまで timed DDC frequency command は FIFO から抜けない（設計メモ [39 §1](39-dsp-timed-commands.md)）．詰まった handle へ追加 command・reset・coercion を送らず，必要なら電源を入れ直す．後続の queue-depth 測定では RX frequency 条件を sample drain 付きで行い，TX frequency / RX-TX 交互条件は連続 TX の RF 扱いを避けるため測定範囲から外した． |
| 3 | 264.33 s 後，open の UR-7 `Reference Clock PLL failed to lock to internal source` で panic．それ以前の測定行は log にあるが，suite と record は失敗． | 電源 cycle 後の B1 で `ref_locked: true` を確認．UR-7 は測定値に含めない． |

### 途中までに得た観測（いずれも参考値）

下表は shake-out log の出力であり，1 台・少数試行の観測．profile 値や複数台 parity の根拠にはしない．run 間で揺れた値はそのまま記す．

| 観測 | log の結果 |
|---|---|
| Device lead | 500 µs・300 µs は各回 0/8 late．200 µs は shake-out 1/2/3 で 0/8，8/8，2/8 late（run 間で不一致）． |
| TX in-flight | 500–10000 µs は記録された run で underflow / TIME_ERROR なし．250 µs は shake-out 1 で underflow 0・TIME_ERROR 0，2 で 1・35，3 で 0・38（run 間で不一致）． |
| Burst gap | gap 0 sample は 4/4 `BurstAck+TimeError`；測定した gap 1,2,3,5,10,20,50,100,1000 は各 4/4 `BurstAck`．empty EOB 条件では B の gap 0・1 が各 run 4/4 late，gap 2・3 は各 run 0/4，gap 5 は shake-out 1・3 が 0/4，2 が 1/4 late（`Underflow+TimeError`）．A の最後の data buffer に EOB を置く条件では gap 0 が各 run 4/4 late，gap 1・2・3・5 は各 run 0/4． |
| Timed tune phase | `1000.1234 MHz` の timed tune spread は shake-out 1/2/3 で 99.0° / 137.5° / 156.0°，untimed は 148.6° / 117.8° / 118.7°．少数 run で安定せず，この値から profile を決めない． |
| Loopback delay | B6：400 kS/s 35 samples（87.5 µs），500 kS/s 44（88 µs），1 MS/s 44（44 µs），2 MS/s 36（18 µs）．shake-out 2 の 500 kS/s だけ TIME_ERROR 1，ほかは 0． |
| UR-17 | 390625，1M，10M，50M S/s × request 2000 / 65536 の 8 条件で，各 200/200 回が要求 sample 数を返した．呼び出し時間は最小 2.2 µs，最大 178.5 ms の範囲で揺れた． |
| H2 / H5 | H2 underflow mid-burst：underflow 3，no-underflow 0．H5 held-last-sample：600 submits，600 burst records． |
| #56 | Stop between starts は 3 trials．Stop lag は 20 counted Stops，0 not counted． |

記録 suite の見込みは 30〜40 分（INFERRED）．shake-out 1 は途中で終了し，shake-out 2〜3 も正常完了していないため，完全な measurement record はまだ揃っていない．段階4では X300 2台，負荷ありの記録，必要試行数を揃えてから profile を判断する．

### 4回目（電源 cycle 後，commit `8f4f060`＋一時的なローカル変更）

raw queue と coercion を一時的に外して実行．測定 suite は最後まで進み，`shakeout-4.json`（約157 MB）を書いたが，最後の profile 比較で `hw_measurements_record` は失敗した．所要 2,155.32 s（約35分55秒）．profile との比較で落ちたのは RX throughput（800 MB/s，profile 1,000 MB/s）と TX throughput（200 MB/s，profile 1,000 MB/s）．TX throughput は 100 MS/s で約29 s 後に Underflow，200 MS/s では約3.3 s 後に Underflow した．B1 の `ref_locked` は電源 cycle 後に true．

| key | 結果 | 条件・注記 |
|---|---|---|
| `radio.full_duplex`，`radio.hardware_time` | true（20/20） | duplex-1msps |
| `radio.perf.rx_bytes_per_s` | 800,000,000 | rx-200msps |
| `radio.perf.tx_bytes_per_s` | 200,000,000 | tx-50msps |
| `radio.timing.min_timed_command_lead_ns` | 5,000,000 | tx-1msps，100試行 |
| `radio.timing.tx_in_flight_ns` | 3,000,000 | tx-1msps，100試行 |
| `radio.timing.rx_start_lead_ns` | 1,000,000 | cold-1msps-2msps，100試行，一番小さい候補 |
| `radio.timing.tx_start_lead_ns` | 1,000,000 | tx-cold-1msps-2msps，100試行，一番小さい候補；構成が間に合った証拠で，burst は未観測 |

追加観測（shake-out のため profile 値には使わない）：

| 観測 | 結果 |
|---|---|
| Overflow gap | 最大13.97 s（stall と socket backlog drain を含む）．読み直しから再開までの推定最大75.0 ms（1 MS/s，INFERRED，高めの推定）．`overflow_restart_gap_ns` は `unmeasured` のまま．|
| Burst gap | 通常 burst は gap 0 sample で4/4 `BurstAck+TimeError`，gap 1〜1000 sample は各4/4 `BurstAck`．empty EOB 条件は gap 0・1 で4/4 late，gap 2・3・5 は0/4 late；最後の data buffer の EOB 条件は gap 0 で4/4 late，gap 1・2・3・5 は0/4 late．|
| Timed tune phase | `1000.1234 MHz`：timed spread 0.8°，untimed spread 170.1°．|
| Loopback delay | 400 kS/s：35 samples（87.5 µs），500 kS/s：44（88 µs），1 MS/s：44（44 µs），2 MS/s：36（18 µs）．TIME_ERROR は0．|
| UR-17 | 390625，1M，10M，50M S/s × request 2000 / 65536 の8条件で，各200/200回が要求 sample 数を返した．呼出し時間は2.2 µs〜178.6 ms．|
| H1 / H2 / H5 | H1 multi-packet burst 18試行．H2 underflow mid-burst：underflow 3，no-underflow 0．H5 held-last-sample：600 submits，600 burst records．|
| #56 | Stop between starts：3 trials．Stop lag：20 counted Stops，0 not counted．|

## D9 probe（設計メモ [39 §3](39-dsp-timed-commands.md)，`8f4f060` の写しを別 volume で build）

| probe | 結果 |
|---|---|
| `hw_d9_p1_ddc_entries_wait_for_samples` | 受信していない channel に時刻つき DDC shift を20・40・60 kHz渡し，untimed で0に戻してから受信開始すると，tone は約160 kHz（median 約159.98 kHz）．untimed reset を各 command の時刻より前に行っても後に行っても，最後の時刻つき設定（60 kHz）が残った．F2 を確認．3 件が 1 sample に 1 件ずつ取り出されることは log では確かめていない（設計メモ 39 §1 の source からの推定，INFERRED）．|
| `hw_d9_p2_duc_entry_after_its_samples` | burst 前に時刻つき DUC shift を渡す（e = burst start + 20 ms）と，tone は e の約19 ms前，burst の先頭から移動した．head samples の送信後，e の3 ms前に渡すと e の+25 µsで移動した．|
| `hw_d9_p3_duc_entries_between_bursts` | burst 間の時刻つき +50 kHz は次の burst に当たった．時刻つき +80 kHz の直後に untimed 0 を渡しても，次の burst は約+80 kHz（median 約179.9 kHz）となり，時刻つき値が残った（F4）．|

3回目までの失敗理由・修正は上表のとおり．4回目も profile 比較で失敗しており，完全に成功した measurement suite はまだ無い．queue/coercion の再測定，profile 値の見直し，overflow restart gap の扱いは段階4で判断する．shake-out と D9 の log / JSON は `~/.cache/ezsdr-bench/2026-10-27/` に保存し，`measurements/` へは置かない．

## 2026-10-28：最初の記録 suite（X300 #1＋OBX，commit `afbb125`）

### 機材と配線

X300 #1（serial 347D545，FPGA 39.3 / d375d68，HG），slot A OBX．OBX の TX/RX → 40 dB attenuator → RX2（同軸．owner が送信の前に確認）．host の設定は `bench-host.sh` がすべて揃っていると判定（MTU 9000，ring 8160，governor performance）．`ref_locked` は true（`hw_b1_probe`）．出力は `~/ezsdr-bench/2026-10-28/afbb125c704e-094947966920515/` と `~/.cache/ezsdr-bench/2026-10-28/`（repo の外）．

### UHD 付属の `benchmark_rate`（送信 200 MS/s，60 s）

| 実行 | underrun | 送った sample | spp |
|---|---|---|---|
| 普通の優先度 | 8 | 11 999 989 924 | 1996 |
| `--priority high` | 0 | 12 000 047 808 | 1996 |
| `--tx_spb 1000` | 8 | 11 999 987 000 | 1996 |

UHD 自身でも普通の優先度では 200 MS/s の送信に underrun が出て，thread の優先度を上げると消える．send を小さくしても変わらない．送信の throughput の調査（uhd-tx の preemption と X300 の小さな送信 buffer）と合う．owner の決め（2026-10-28）：この結果を受けて uhd-tx を実時間の優先度にする段階を立てる．

### 記録（`hw_measurements_record`，約 48 分）

`measurement.json` は書けた．最後の profile との比較で 3 件の違反（段階 4 の材料．bench 中は直さない）：

| key | 実機 | profile |
|---|---|---|
| `radio.perf.rx_bytes_per_s` | 800 000 000（`rx-200msps`） | 1 000 000 000 |
| `radio.perf.tx_bytes_per_s` | 200 000 000（`tx-50msps`） | 1 000 000 000 |
| `radio.timing.command_queue_depth` | **7**（`queue-rx-frequency`） | **16** |

- queue の深さ（段階 3e・3f の規則）：3 条件とも 5 回すべて成功，あいまい 0，2 s と 3 s の遅れで止まった呼び出しの番号が一致（受信の周波数 7，受信の利得 28，送信の利得 86 の timed `apply`）．**profile の 16 は実機より緩い**（Mock が実機より緩い：段階 4 で直す）．
- 送信の throughput：100 MS/s は 60 s の約 57 s 分で underflow，200 MS/s は約 10 s 分．send は 1 回約 1000 sample（`burst_blocks`）．
- 同時運転の 1 組（受信 200 MS/s と送信 50 MS/s，60 s）：成功，overflow なし．
- overflow の読み直しから再開まで：最大 78.3 ms（1 MS/s）．
- そのほかの値（lead，in-flight，coercion の格子，利得の段 0.5 dB，周波数 10 MHz〜8.4 GHz）は `measurement.json`．

### D9 の probe と MA-1

- **MA-1**（`hw_ma1_phase_after_a_pre_start_tune`）：T0 より前に設定 → 5 Session の位相のばらつき 164.5°．流れている間に `at` で設定し直す → 2.3°．UR-24 の Ceiling と KL-32 を支える観測（2 つの LO の相対位相で，受信だけの寄与は分けられない）．
- **P5〜P7**：音の判定がすべて INCONCLUSIVE（判定の閾値が 1 を超えて通らない．受信 gain 0 dB で雑音が約 1 LSB のため雑音の床の coherence が高い）．周波数の中央値から：P6「burst の前に渡す」と P7 で e より前に新しい周波数が出たのは実機の早い適用で F7（直前の process が DUC に残した時刻つきの header が新しい e より後，機構は source で VERIFIED，原因は INFERRED）．P5 は F7 の前提を作れず INCONCLUSIVE．
- **P4**：100 ms の音の send が送信 buffer で詰まり，stop が e の約 20 ms 後になった（probe の時刻の bug）．R2 は未決．
- 分析：`~/.cache/ezsdr-fixes/2026-10-26-stage3/probe-analysis-1028.md`（repo の外）．probe と判定の直し（段階 3g）の後に 2026-10-29 に再実行する．

### 2 channel の記録（X300 #2，UBX-160 v2 slot A＋CBX slot B，commit `afbb125`，約 85 分）

X300 #2（serial 347D541，FPGA 39.3 / d375d68），NIC `enp1s0f0np0`（#1 と同じ port に挿し替え），selector `{"rx":["0:A:ubx:0","0:B:cbx:0"],"tx":["0:A:ubx:0","0:B:cbx:0"]}`．UBX と CBX それぞれ TX/RX → 30 dB attenuator → 同じ board の RX2（同軸，owner が確認）．出力は `~/ezsdr-bench/2026-10-28/afbb125c704e-112734601394077/`．`measurement.json` は書けた．profile との比較の違反は 2 件（段階 4 の材料）：

| key | 実機（#2） | profile | #1（OBX，1 channel） |
|---|---|---|---|
| `radio.perf.rx_bytes_per_s` | 1 200 000 000（`rx-mixed-200m-100m`．止めた条件 `rx-grouped-200m`，1 600 000 000） | 1 000 000 000 | 800 000 000 |
| `radio.perf.tx_bytes_per_s` | **40 000 000**（`tx-10msps`．止めた条件 `tx-20msps`） | 1 000 000 000 | 200 000 000 |
| `radio.timing.command_queue_depth` | **8**（`queue-rx-frequency`．受信の利得 28，送信の利得 86） | 16 | 7 |

- 送信は #2 では 10 MS/s で止まり，#1（50 MS/s）よりかなり低い．原因（board，2 channel の構成，uhd-tx の優先度）は未確認．uhd-tx の実時間の優先度の段階の後に見直す．
- queue の深さは #1 の 7 と #2 の 8 の両方が profile の 16 より小さい（Mock が実機より緩い：段階 4）．受信の利得の条件で 1 試行が成功でなかった（6 試行で成功 5）．
- 同時運転の 1 組（受信 200＋100 MS/s と送信 10 MS/s，60 s）：成功，overflow なし．装置の詰まり・PLL のはずれは無し．

### 2 channel の記録，uhd-tx の優先度の後（X300 #2，同じ配線，commit `98b0d9a`，約 83 分）

出力は `~/ezsdr-bench/2026-10-28/98b0d9a919c9-154746201756884/`．`measurement.json` は書けた．test の失敗は profile との比較の違反 3 件だけ（受信 600 MB/s，送信 260 MB/s，queue の深さ 8，いずれも profile 1 GB/s・16 より小さい）．

| 条件 | 結果 |
|---|---|
| 送信 1 channel | 100 MS/s まで成功，200 MS/s は DeviceFailure |
| 送信 split（streamer 2 本） | 100 MS/s × 2（800 MB/s）まで成功，200 MS/s は admission で拒否 |
| 送信 mixed | 100＋50 MS/s（600 MB/s）まで成功，200＋100 MS/s は DeviceFailure |
| 送信 grouped（1 streamer に 2 channel） | 25 MS/s × 2 まで成功，**40 MS/s × 2（320 MB/s）から DeviceFailure** |
| 受信 1 channel | 100 MS/s まで成功，**200 MS/s は DeviceFailure** |
| 受信 grouped | 50 MS/s × 2 まで成功，100 MS/s × 2 は DeviceFailure |
| 受信 split / mixed | split 100 MS/s × 2，mixed 200＋100 MS/s まで成功 |

- 送信は前回（`afbb125`）の 10 MS/s から，1 channel で 100 MS/s，streamer 2 本で計 800 MB/s まで上がった．uhd-tx の実時間の優先度（段階 TXP）の効果と読む（INFERRED：変えたのは commit だけで，配線と NIC は同じ）．記録の値 260 MB/s は grouped の失敗で決まる（`tx-grouped-40m` が止めた条件）．1 streamer に 2 channel を束ねた送信が弱い理由は未調査．
- 受信の値は前回の 1.2 GB/s から 600 MB/s に下がった．`rx-200msps` と `rx-grouped-100m` が今回だけ DeviceFailure．受信の経路は TXP で変えていないので，試行ごとのばらつきと読む（INFERRED）．失敗の詳細は log に残っていない．

### D9 P5〜P7 の再実行，2 サンプルの終わりの burst（X300 #2，UBX slot A，30 dB loopback，作業 tree＝`98b0d9a`＋`untimed_end_of_burst` を 0 の 2 サンプル・SOB＋EOB・時刻なしに変更）

log は `~/.cache/ezsdr-bench/2026-10-28/rerun2/`．3 本とも test は成功．

| probe | 結果 |
|---|---|
| P5 C（時刻つきで終えた burst の後，reopen してから 2 サンプルの burst） | DUC の移動は e＋25 µs（中央値 99 950 → 149 984 Hz）．古い時刻の header は 2 サンプルの burst で消える：**F7 の回避策 VERIFIED** |
| P6（全 chunk に時刻） | 移動は e＋25 µs（tune を burst の前に出しても，e の 3 ms 前に出しても同じ） |
| P6 starvation | 過ぎた時刻の続き（2000 サンプル）は送れたが，Underflow の後は TimeError が並び，続きのサンプルは loopback に 0（捨てられた）．tx の回復は観測した：**R4 の挙動 VERIFIED（遅れた続きは送られない）** |
| P7（今の uhd-tx の形：最初の send だけ時刻つき，後は時刻なし） | **DUC の移動が e＋40 ms まで起きない**（tone は前後とも約 99 950 Hz，信号の窓は全部あり）：**F3′ VERIFIED**（#2，UBX） |

- 前回 P5 C・P6・P7 で tone が見えなかったのは，1 サンプルの burst（EOB だけの send）で次の burst の時刻が失われたためと読める（P5 C と P6 は今回見えた）．P7 は tone が見えたうえで動かないので，前回とは別の結果．
- P7 の原因は，時刻なしの packet しか来ない間は DUC の時刻つき command が実行されない，と読む（INFERRED：P6 との違いは続きの send の時刻の有無だけ）．本番の uhd-tx は P7 の形なので，送信中の時刻つきの送信周波数の変更は，次の時刻つき burst まで効かない（issue #72，D9b で直す）．

## 2026-10-29：2 台を同時に接続（#1＝X300 347D545＋OBX，192.168.40.36，`enp1s0f1np1`；#2＝X300 347D541＋UBX/CBX，`ip-addr3` を 192.168.47.37 に変更，`enp1s0f0np0`）

両方とも FPGA 39.3，frame 8000 byte．att は #1 OBX 40 dB，#2 UBX・CBX 各 30 dB（owner が確認）．#1 から #2 へ PPS と 10 MHz を配線済み（今日の測定はすべて `internal`）．log は `~/.cache/ezsdr-bench/2026-10-29/`．commit は `e10afa5`．

### 200 MS/s の送信の切り分け（#2，UBX slot A，`--priority high`／uhd-tx は実時間の優先度）

| 送るもの | 1 回の send | 時間 | 結果 |
|---|---|---|---|
| `benchmark_rate` | 約 2000（既定） | 300 s | underrun 0 |
| `benchmark_rate` | 10 000 | 300 s | underrun 0 |
| `benchmark_rate` | 1000 | 60 s／300 s | underrun 6／5 |
| `benchmark_rate` 送受信同時（各 200 MS/s） | 1000 | 60 s | underrun 2，overrun 0 |
| Module 1 channel（`hw_tx200_hot_path_probe`），波形 1000 | 1000 | 30 s／300 s | 成功／**約 26 s で underflow** |
| Module 1 channel，波形 20 000 | 2000 | 300 s（2 回） | **約 150 s と約 122 s で underflow** |
| Module grouped 40 MS/s × 2，波形 20 000 | 1000／channel | 300 s | 成功 |
| Module split 100 MS/s × 2（800 MB/s），波形 20 000 | 2000 | 300 s | 成功 |

- send が大きいほど途切れにくい（UHD と Module の両方）．同じ send の大きさでは Module の方が途切れやすい（INFERRED：send ごとの確保・metadata・parse・lock，Sol の読み〈[reviews/tx200-investigation-sol](reviews/tx200-investigation-sol.md)〉）．
- 30 s の試行（前日の `hw_tx200_hot_path_probe` 初回，12 試行すべて成功）は短すぎて，数分に 1 回の途切れを捉えない．記録の suite の失敗（1 channel 200 MS/s，grouped 40 MS/s）は，この稀な途切れに当たったと読む（INFERRED）．
- 直し（段階 TXS）：送信の send の上限を受信の block（2000）から切り離して大きくし，短い繰り返し波形は burst ごとに一度だけ並べて送り，send ごとの確保などを減らす．

### D9 probe，#1 OBX（P4〜P7）

#2 UBX（前日）と同じ：P4 A／C で使われなかった DDC の設定が停止・close／reopen を越えて残り，再開の最初の sample が拾う（R2 VERIFIED on OBX）；P4 B（AUTO）は残らない；P5 A は 19 ms 早く移る（F7），P5 C（2 サンプルの burst）と P6 は e＋25 µs；P6 の遅れた続きは捨てられる（R4）；**P7 は移らない（#72 VERIFIED on OBX）**．

### #1 → #2 の 10 MHz と PPS（UHD だけ，Module は未使用）

`uhd_usrp_probe` と `test_pps_input`（`--args addr=192.168.47.37,clock_source=external --source external`）は #2 を外部の基準で開けた．`test_pps_input` は「Attempt to detect the PPS and set the time... Success!」：#1 の背面の PPS が #2 に届く（VERIFIED）．10 MHz は，X300 が基準の lock に失敗すると開く時に error を返す（UR-7 と同じ経路）ので lock したと読む（INFERRED：`ref_locked` の値は読んでいない）．UHD Module の外部基準の経路（UR-7）は未実行（8b）．

### TXP B4（実時間の優先度の権限なし：`--ulimit rtprio=0`，`SYS_NICE` なし）

`hw_tx200_hot_path_probe`（1 channel 200 MS/s，波形 20 000，10 s）：Run は終わりまで走り（test 成功，DEVICE_LOST なし），underflow 4 回，約 0.2 s 分（39 846 000 sample）で burst が止まった．実時間の優先度ありの約 122〜150 s と比べ，TXP の効果は大きい（INFERRED：別の build が並行して走っていた）．`set_thread_priority` の拒否の行が Manifest に出たかは，この probe の出力からは見えない（未確認）．

### #70 の直接の probe（`hw_d70_one_sample_burst_then_timed_burst`，#2 UBX slot A）

1 サンプルの burst（SOB＋EOB，時刻なし）の後の時刻つき burst：tone は時刻より **約 150 ms 早く**（送った時点で）出た．1 サンプルの burst には BurstAck が返らない．対照（0 の 2 サンプルの burst の後）：時刻の ＋34 µs．**#70 VERIFIED**（1 サンプルの burst は装置の中で閉じず，次の burst の時刻を失わせる）．CBX（slot B）では流していない：raw probe の送信の安全範囲（1 GHz ± 0.5 MHz）が CBX の範囲（1.2–6 GHz）の外．DDC/DUC は motherboard の FPGA にあるので結果は同じと読む（INFERRED）．

### TXS の後の 200 MS/s（#2 UBX，2026-10-29 14:28〜14:38）

TXS（send は最大 20 000 sample，短い repeat は tile）の worktree で `hw_tx200_hot_path_probe`（`EZSDR_TX200_LENGTHS=1000,20000`，`EZSDR_TX200_SECONDS=300`，1 channel，att 30 dB は owner が確認）．log は `~/.cache/ezsdr-bench/2026-10-29/tx200-probe-txs.log`．

| 波形 | send の数 | 1 send あたり | 結果 |
|---|---:|---:|---|
| 1 000 | 3 000 021 | 約 20 000 | Success，underflow 0（TXS の前は約 26 s で underflow） |
| 20 000 | 3 000 019 | 約 20 000 | Success，underflow 0（TXS の前は約 150 s で underflow） |

1 channel 200 MS/s の送信は 300 s を通った（VERIFIED，2 試行）．cap 20 000 はこの rate で足りる．より長い時間と RX 同時運転は段階 4 の throughput で測る．

## 2026-11-05：note 47 段階 3 の後（#2＝X300 347D541＋UBX/CBX，#1＝X300 347D545＋OBX，commit `9dab08a`＋実機で直した試験）

手順は [bench.md](bench.md) §「2026-11-05 の手順」．att は 3 つの board とも 30 dB で，それぞれ自分の loopback（owner）．log は `~/.cache/ezsdr-bench/2026-11-05/`（repo の外）．PC は当日再起動した（kernel 7.0.0-42）．

- **ホスト**：PC 側の 2 本の 10 GbE のケーブルが逆だった（#1 が `enp1s0f0np0`，#2 が `enp1s0f1np1` で答えた）．`usrp-net.sh` が両方の NIC に同じ subnet を付けていたので，ブロードキャストの `uhd_find_devices` は二台を見つけ，`uhd_usrp_probe` は「No devices found」になった．owner がケーブルを入れ替えた．`usrp-net.sh` は NIC ごとに subnet を一つにし，取り違えを `CABLES SWAPPED` で止める形に直した．手順の (a) に `ping -I` の確かめを足した．
- **(b) raw の D9 probe**：二台とも前の記録と同じ．P5 A は −19 ms，P5 B と C は e＋25 µs，P6 は e＋25 µs，P7 は移らない，D70 は 1 サンプルの burst の後が −149.8 ms（対照は＋21〜38 µs），P4 は A と C で DDC の設定が残り（150 kHz），B は残らない．
- **(c) 本番の試験**：
  - `hw_p72_…` は二台とも PASS（26 send がすべて時刻つき，閉じる send は 2 サンプル以上，flush の ACK あり）．#72 の閉じる条件（(c)1 と raw P6 が両方の board で PASS）を満たした．ただし owner は #72 を「note 47 で前提がなくなった」として閉じると決めた（下）．
  - `hw_p70_…` は二台とも FAIL．#70 の症状は Module の道では出ない（後の burst の onset は 1 サンプルの試行で 25 サンプル，対照で 25〜26 サンプル）．FAIL の理由は，対照を含む両方の試行の TIME_ERROR 1 と TX_UNDERFLOW 10〜11（#78）．#70 は開いたまま．
  - `hw_n47_rx_restart_with_new_values`（段階 3 の道）は二台とも PASS．流れている間の値は Action の id つきで拒まれ，区間は途切れない．`Stop`→値→時刻つき `start_rx` は予約した origin のとおりに始まり，`GAP_BEFORE`・`LATE_COMMAND` なし，`START_LEAD_SHORT` 0．新しい区間は信号の立ち上がりから 50 kHz．設定時の read-back は 1000.05 MHz と 6 dB．試験の期待は実機で 3 か所直した（owner の承認．origin は登録された SampleClock から，窓は新しい区間の sample から，判定は立ち上がりから）．
  - **装置の性質**（INFERRED：DDC のフィルタが満たされる時間）：受信を再開するたびに，最初の約 13 サンプルが厳密に 0，続く約 10 サンプルで振幅が立ち上がる（1 MS/s，UBX と OBX）．古い周波数のサンプルはない．gap の印は付かない．owner：v4 の制限ではなく装置の性質なので，FakeDevice と MockRadio（profile の値）で再現する．rate ごとの数はまだ測っていない．
- **(d) L2 の送信の試験（#1）**：`hw_b6_…`・`hw_b7_…`（`TX_UNDERFLOW` starved，burst の約 1.4〜1.5 ms 目），`hw_b8_burst_at_a_sent_burst_s_end`（装置の TimeError/Underflow），`hw_b8_burst_one_sample_after_a_burst`（`radio.TIME_ERROR`）が FAIL．`hw_b8_preemption`・`hw_h5_…`・`hw_txp_b0_send_sizes`・`hw_b9_transmit_only_session`（60 s）は PASS．Module を通る送信はどれも UHD が `U` を約 9 個出し，raw の送信は出さない → #78．
- **(e) TX200 と (f) ssh 切断**：#78 を直した後の実機に延ばした（owner）．
- **(g) raw の note 47 の probe**：
  - T4 受信（開いたままの streamer で 1 → 2 → 10 → 1 MS/s）：二台とも PASS（read-back，block の時刻の間隔，tone）．**note 47 §1 の受信の rate の受け入れの条件を満たした**．
  - T4 送信（1 → 2 MS/s）：#2 PASS，#1 INCONCLUSIVE（B の最初の窓に coherent な tone がない）．送信の rate の条件はまだ閉じない．
  - T5（前の burst の後に時刻なしで送信の設定）：二台とも，m＝0（実際は e の約 0.2 ms 後），＋1，＋5，＋20 ms と ＋5 ms の gain（＋6 dB）のすべてで A の末尾が無傷（tone 100 kHz，電力の比 0.998〜0.999）．burst ACK は毎回 e＋1 tick．B はどれも新しい値（例外は #1 の m＝＋1 の 1 回で，B の最初の窓が INCONCLUSIVE）．**破壊的な対照**（owner の承認，#2，e の 4.6 ms 前）は A の末尾が新しい周波数（約 200 kHz）に変わり PASS：probe は乱れを見つけられ，時刻なしの送信の設定は burst の途中ではすぐ効く．B の始まりは対照の遅れから数 µs ずれる（＋7〜14 µs など）．試験はこれを INCONCLUSIVE として印字する（実機で直した，owner の承認）．
  - T6 制御の呼び出し（#2，50 回，中央値）：受信の周波数 17 µs（小さな変更）／39 µs（1 GHz をまたぐ），送信の周波数 15／42 µs，gain 8〜20 µs，rate 24〜58 µs，`time_now` 113 µs，`rx_open` 0.32 ms，`tx_open` 0.65 ms，サンプルの流れる中での時刻つきの受信の周波数 114 µs．関数が戻るまでの時間で，RF の整定ではない．
  - T6 受信の buffer（#2）：1 MS/s は 5 s 止めても overflow なし，10 MS/s は 559 ms まで大丈夫で 635 ms で overflow，50 MS/s は 100 ms まで大丈夫で 176 ms で overflow（`rmem_max` 50 MB，`recv_buff_size` は既定．約 25〜30 MB の socket buffer に相当，INFERRED）．
- **(h) 受信の start lead の群**：二台とも PASS．候補の最小 3 ms でも，4 条件（値を変えない再開，1 GHz をまたぐ再開，それぞれ送信の companion〈10 MS/s の PN〉つき）のすべての再開が予約した origin にあり，`LATE_COMMAND` も `START_LEAD_SHORT` もない．残りのリード（origin − 設定の完了）の最小は約 6.7〜9.4 ms（origin は止めた時刻＋release window などでも後ろへ押される，INFERRED）．profile の `rx_start_lead_ns` 50 ms は実機より大きく余裕がある（段階 4 の材料，profile はまだ変えない）．companion の送信は Run 全体で fault 0〜2（#78 と同じものと読む）．

**owner の決定（2026-11-05）**：送信の chunk ごとの時刻（D9b-1a）は最初だけ時刻つき・続きは時刻なしに戻し（#70 の 2 サンプル以上の閉じる send は残す），#72 は note 47 で前提がなくなったとして閉じる．先に FakeDevice で #78 を再現する．その前に，Run の中の値の変更や burst の途中の時刻つきの設定のために入れた仕組みを棚卸しする設計メモ 48 を書き，理由がなくなった物を簡素化の段階で消す．その後の実機で 200 MS/s の Module の負荷の試験（受信・送信の 1 channel の長時間，同時運転）と延ばした (c) の #70・(d)・(e)・(f) を流す．

### 2026-11-05（続き）：UHD だけの最小再現（C++，`plan/phase8/tools/uhd-repro/tx_repro.cpp`）

#78 の原因を Module から切り離すため，`multi_usrp`・`tx_streamer`・`recv_async_msg` だけで送る C++ を書いた（Ez-SDR のコードを通さない．Sol の事前確認 FIX FIRST の 7 件を入れてから流した）．二台（#2 UBX，#1 OBX，slot A，30 dB loopback，1 GHz，gain 0 dB）で同じ結果．log は `~/.cache/ezsdr-fixes/2026-11-05-n48s1/uhd-repro/`（repo の外），まとめは同じ所の `hardware-facts-cpp.md`．

- **単独の短い burst**（一回の send に SOB＋EOB，各 3 回）：

  | rate（補間） | 時刻つき 2 | 時刻つき 3 | 時刻なし 2 | 時刻なし 3 | 4〜512 |
  |---|---|---|---|---|---|
  | 1 MS/s（200） | 捨てられる（TimeError 32，ACK なし） | 捨てられる | Underflow 8〜9 | Underflow 13 | 問題なし |
  | 2 MS/s（100） | 捨てられる（TimeError 28） | 捨てられる（TimeError 6） | Underflow 4 | Underflow 6 | 問題なし |
  | 10 MS/s（20） | 問題なし | 問題なし | Underflow 0〜1 | Underflow 1 | 問題なし |
  | 50，200 MS/s | 問題なし | 問題なし | 問題なし | 問題なし | 問題なし |

  時刻なしの 2 サンプルの SOB＋EOB は T3 flush の形で，その Underflow が #78 の一つ目の出どころ（VERIFIED）．時刻つきの burst の ACK は 126 回すべて終わりの 1 tick 後．
- **一つの burst を二つの send に分ける**（時刻つきの最初の send＋時刻なしの閉じる send，1 と 2 MS/s，各 3 回）：1＋2，1＋3，2＋2，2＋3，3＋2 は試行によって捨てられる・Underflow・ACK のずれ（−3〜＋53 tick）．**4＋2 は 12 回すべて問題なし**．規則は合計の長さではなく，**device burst の最初の send が channel ごとに 4 サンプル以上**（VERIFIED，1 と 2 MS/s）．仕組みは DUC の補間のフィルタに小さな packet が留まること（INFERRED）．
- **閉じた後の次の burst**：長い burst（9 999＋2 サンプルの close）の後，次の時刻つきの burst を close の終わりちょうどに置くと丸ごと捨てられ，＋1，＋2，＋3，＋1000 サンプルなら鳴る（各 5 回）．UR-22 の「閉じた後の安全な開始は終わり＋1 サンプル」は装置の規則（VERIFIED）．
- **#70 の試験の形**（2 サンプルの burst の 25 µs 後に 1 万サンプル）：2 サンプルの burst が捨てられ（TimeError 32，ACK なし），後の burst は時刻どおり．続きの send を全部時刻つきにしても最初だけにしても同じ．「すべての send に時刻」は #78 のどの症状とも関係しない．
- 試していないこと：一つの streamer で二つの channel，10〜50 MS/s の間の rate．

## 2026-11-06：note 48 の後（#2＝X300 347D541＋UBX/CBX，#1＝X300 347D545＋OBX，commit `4f7e9e4`）

手順は [bench.md](bench.md) §「2026-11-06 の手順」．二台を同時につないだ（NIC ごとに subnet 一つ）．att は三つの board とも 30 dB，有線の loopback（owner，この日は毎回の確認を省いた）．PC は 11-05 から再起動していない．log は `~/.cache/ezsdr-bench/2026-11-06/`（repo の外）．

- **(a)**：二台とも ping・`bench-host.sh`・`uhd_usrp_probe` が通った（FPGA 39.3）．NIC の counter はコンテナから読める．
- **(b) #78 と #70（Module の送信）**：`hw_b6`・`hw_b7`・`hw_b8_burst_at_a_sent_burst_s_end`・`hw_b8_preemption`（5 と 3 ms は late 1 つ，10 と 20 ms は 0）・`hw_h5`・`hw_txp_b0`・`hw_p72` は二台とも PASS（11-05 は `hw_b6`・`hw_b7`・`hw_b8` が #78 で FAIL）．`hw_b8_burst_one_sample_after_a_burst` は #2 で PASS（終わり＋1 は late 1 つ，＋2 は 0），#1 で＋2 の試行が Underflow．`hw_s1_short_first_sends` は二台とも，試行のおよそ 2 割で長い burst が数サンプルで Underflow し，続く packet が late（約 160 個）になる．短い burst の長さ（1〜4）には関係しない．`hw_p70` は「一緒に予約」が二台とも PASS（onset の差 0〜1 サンプル），「送った後に予約」は二台とも，対照を含めて TIME_ERROR 1 つ：A の 41 ms 後に A＋5 ms の B を予約するので B は予約の時に装置の lead の中にある（試験の形の欠陥，INFERRED：cause は印字していない）．#70 と #78 はまだ閉じない．
- **#78 の残りの原因（VERIFIED）**：Module の send を `ProbeDevice` で記録した（一時的な診断，commit しない）．失敗した試行では長い burst の最初の send が 6〜28 サンプル（in-flight の窓に入る分だけ），通った試行では 145〜156 サンプル．続きの send は開始の約 7 ms 前に時刻つきで出ていて，ホストは遅れていない．UHD だけの C++（`plan/phase8/tools/uhd-repro/tx_repro.cpp` の case `head`，続きは最初の send の 0〜2 ms 後，時刻つきと時刻なし）で再現した：

  | rate（補間） | 最初の send ≤ 28 サンプル | ≥ 30 サンプル |
  |---|---|---|
  | 0.5，1，1.25，2，2.5 MS/s（400〜80） | 毎回 Underflow（時刻つきの続きは late 約 340 個，時刻なしは TimeError 1 つ） | 問題なし |
  | 4，5，8，10 MS/s（50〜20） | 2 サンプルでも問題なし | 問題なし |

  #1 の 0.5〜10 MS/s と #2 の 1・2 MS/s（各 6〜10 回）．続きが 0 ms 後（すぐ）なら 28 サンプルでも通ることがある（#2，1 MS/s，5 回中 3 回）．S1 の「device burst の最初の send は 4 サンプル以上」は閉じる send がすぐ続く形（11-05）だけの規則で，続きが後から来る形では低い rate で 30 サンプル以上が要る．C++ の case `s1`（`hw_s1` と同じ形，最初の send が 2 000）は 120 回すべて問題なし．
- **(c) 受信の再開**：`hw_iii_raw_rx_restart_fill` は二台・8 rate とも PASS（各 5 回，同じ rate ではほぼ同じ値，UBX と OBX で同じ）：

  | rate（MS/s） | 1 | 2 | 5 | 10 | 25 | 50 | 100 | 200 |
  |---|---|---|---|---|---|---|---|---|
  | 先頭の 0（サンプル） | 14〜15 | 10 | 14 | 10〜11 | 13 | 9〜10 | 3〜4 | 0 |
  | 90 % までの立ち上がり（サンプル） | 9〜10 | 8 | 9〜10 | 7〜8 | 11 | 7〜8 | 8〜9 | 0 |

  `hw_n47_rx_restart_with_new_values` は二台とも全 PASS（S2 の後も予約した origin どおり）．`hw_n47_rx_start_lead_group` は二台とも PASS で最小の候補 5 ms（11-05 は 3 ms）：3 ms の `restart-retune` は 100 回とも装置の start と最初の block が origin どおりだが，残りの lead が約 4.1 ms で `START_LEAD_SHORT` が出て数えない．`restart-same` は 3 ms で 100 回成功．`hw_b8_raw_restart_lead` は 1 ms の lead まで開始が予約どおり（＋0 tick）．
- **(d) 送信の rate**：#1 は PASS（11-05 の INCONCLUSIVE は試験の窓の直しで消えた）．#2 は 3 回とも INCONCLUSIVE：A の ACK・tone・read-back は合い，B の onset だけが予想から −1 200，−2 200，＋2 200 tick（許容 1 000）．向きがそろわないので onset の読みの精度と読む（INFERRED）．送信の rate の受け入れはまだ閉じない．
- **(e) TX200**（#2，300 s）：長さ 1 000 と 20 000 とも PASS（link の drop 0）．
- **(f) 200 MS/s の負荷**（各 600 s）：受信は #2 で PASS（1 200 億サンプル，overflow・drop 0），#1 は装置の側は完全だが Module の feed が受け手に渡す前に 46 block を落とした．送信は #2 が約 2.2 s，#1 が約 159 s で一度 Underflow（続く packet が late）．同時運転は #1 で両方向とも 600 s PASS，#2 は受信が完全で送信が約 1.5 s で Underflow．NIC の error・drop はどれも 0．200 MS/s の送信は 6 回中 3 回（TX200 の 2 回，#1 の同時運転）通り，3 回は時刻がばらばらの一度の Underflow：安定しない．
- **(g) ssh 切断**：流せなかった（この PC の `ssh localhost` が接続を拒む：sshd が動いていない）．送信なし．
- **その他**：#2 で C++ の 2 MS/s を開く時に一度 `Reference Clock PLL failed to lock to internal source`．直後の `uhd_usrp_probe` は正常．owner が電源を入れ直し，その後は正常．

## 2026-11-06（続き）：note 48 S3 の後（#2＝X300 347D541＋UBX/CBX，#1＝X300 347D545＋OBX，commit `de7720c`）

owner が電源を入れ直した後，att は三つの board とも 30 dB（owner 確認）．`uhd_usrp_probe` は二台とも正常．log は `~/.cache/ezsdr-bench/2026-11-06-s3/`．

- **(b) Module の送信**：`hw_s1_short_first_sends` は二台とも 9 回ずつ流して 4 case すべて PASS（S3 の前は試行のおよそ 2 割で長い burst が Underflow していた）．`hw_p72`・`hw_b6`・`hw_b7`・`hw_b8_burst_at_a_sent_burst_s_end`・`hw_b8_burst_one_sample_after_a_burst`（end＋1 は late 1 つ，＋2 は 0）・`hw_b8_preemption`（20 と 10 ms は 0，5 と 3 ms は late 1 つ）・`hw_h5`・`hw_txp_b0` は二台とも PASS．`hw_p70` は「一緒に予約」が二台とも PASS（onset の差 0），「送った後に予約」は二台とも TIME_ERROR 1 つで FAIL：11-06 と同じ試験の形の欠陥（INFERRED，note 48 §4 の 3）．
- #78 の Module の側の原因（最初の send の大きさ）は，この (b) で再び現れなかった．閉じるのは `hw_p70` の形を直して両盤で PASS になってから（bench.md の「#78 が閉じるのは」）．
