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
