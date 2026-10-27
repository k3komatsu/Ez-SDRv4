# 41 — 測定記録，比較テスト，測定スイート（Phase 8 段階3）

Status: 計画（orchestrator，2026-10-26）．Sol のレビュー 1 回目（FAIL，major 7，minor 1）を受けて §2〜§5 を書き直し，再レビューは PASS（§7）．3a から実装する．

段階3は overview §3 の 2・3・7 と P2（測定記録と比較テスト，owner 受理 2026-10-16）を形にする．bench（段階 B）で測定スイートを流すだけにするための準備で，実機は要らない．P2・P3 で決まったことはここで繰り返さず，決まっていない形だけを決める．

## 1. 分け方（1 段階 ≈ 1,500 行，owner 2026-10-19）

| 段階 | 中身 | risky |
|---|---|---|
| 3a | 測定記録の型と schema，key ごとの扱い（§3），比較テスト．記録はまだ fixture だけ | いいえ（新しい型と純粋な比較．queue・deadline・lifecycle に触れない） |
| 3b | 測定スイートの枠，候補の値で測る仕組み（§5），今ある `hw_b8_*` などを記録を出す測定に組み直す．FakeDevice でのリハーサル | はい（fake と実機の境界） |
| 3c | 新しい測定：H1〜H5，#56，送信の start lead，overflow の restart gap 単独，送信と 2ch の throughput，coercion の read-back，BP4b（混在の同時運転）と BP4c（XG，2 link，再 open）の測定，NB-5（受信の `cold` lead），UR-17 の受信要求長の上限 | はい（同上） |
| 3d | ホスト設定の script（`set_addr.sh` を移す），`plan/phase8/bench.md`（BP0〜BP5 の手順，測定スイートの流し方） | いいえ |

## 2. 測定記録（3a）

**置き場と形**：`ezsdr_radio::measurement`（Rust の型，serde）と `schemas/radio/measurement.v1.json`（他の schema と同じく型から生成し，`schema_freeze` に入れる）．記録のファイルは `crates/ezsdr-radio/measurements/<日付>-<装置の serial>-<構成>.json`．version は上げない（AGENTS.md §6）．

**記録 1 つ ＝ bench 1 回 × 1 台 × 1 構成**（P2 (1)）．欄：

- `schema`（`ezsdr.radio.measurement.v1`．違えば拒否，invariant 39），`source`（`hardware` | `fake`），`profile`（`ProfileRef` の name と version），`selector`（Provider の binding の selector と同じ平らな形：motherboard，image，link，slot と front end の layout），`device`（serial，mboard，FPGA image，UHD の版），`host`（名前，設定の script とその版），`load`（`none` | `h4`），`commit`．
- **`conditions`**（Sol N1）：測定の組ごとの運転条件．`{ id, streams: [{ direction, stream, channels: [<論理 channel>], sample_rate_hz }], concurrent: [<同時に動かした stream>], reopened: bool }`．比較テストは各条件を selector の layout に照らす（channel が layout にある，rate が格子にある，stream の channel の集合が RM-30 の規則を満たす）．条件は値の適用範囲の記録で，profile の値を条件ごとに変えはしない（今の profile は条件で値を変えない）．
- **`values`**：比較する key ごとにちょうど 1 行（Sol N5）．`{ key, status: "measured" | "unmeasured", worst?, trials?, condition?, test?, reason? }`．`measured` は Provider を通して測ったもの（P3 (6b)）で，`worst`・`trials`・`condition`（`conditions` の id）・`test` を持つ．`unmeasured` は `reason` を持つ．
- **`observations`**（Sol N4，N7）：比べない結果．`{ name, test, condition?, trials, result, note? }`，`result` は測定ごとの構造（JSON の object）．H1〜H5，#56，raw の値（device を直接叩いた lead など），loopback の合計の遅れ，cold change の途切れの時間，候補の値ごとの結果（§5）など．比較テストは形だけを検査する．

## 3. key ごとの扱い（3a，文書監査 D10，Sol N3・N7）

語彙に key ごとの扱いを 1 か所で宣言する（`ezsdr_radio::measurement::rule(key)`）．Description の capability の key は全部いずれかに入る（入らない key があれば比較テストが落ちる）．

| 扱い | 比較（profile の値 p，記録の最悪値 m） | key |
|---|---|---|
| 下限 | p ≥ m | `radio.timing.min_timed_command_lead_ns`，`startup_latency_ns`，`overflow_restart_gap_ns`，`rx_start_lead_ns`，`tx_start_lead_ns`，`tx_in_flight_ns`，`release_window_ns`；`radio.{rx,tx}.c<n>.frequency_step_hz`，`gain_step_db`（p の刻みが実機より細かければ緩い） |
| 上限 | p ≤ m | `radio.timing.command_queue_depth`，`radio.perf.rx_bytes_per_s`，`tx_bytes_per_s`，`radio.tx.repeat_max_samples` |
| 包含 | p の格子 ⊆ m の格子（`Grid` の値の集合として） | `radio.{rx,tx}.c<n>.frequency_hz`，`gain_db`，`antenna`；`radio.{rx,tx}.s<k>.sample_rate_hz` |
| 倍数 | p は m の倍数 | `radio.tx.repeat_align_samples` |
| 保証の強さ | p の保証は m 以下（`deterministic` ＞ `random_unless_timed_tune` ＞ `random`） | `radio.{rx,tx}.c<n>.phase_behavior_on_retune` |
| 真なら真 | p が true なら m も true | `radio.full_duplex`，`radio.hardware_time` |
| 一致 | p = m | `radio.perf.wire_bytes_per_sample`；`radio.{rx,tx}.c<n>.path_delay_samples`（channel ごとに別に較正したときだけ `measured`．loopback の合計は observation で，TX と RX に振り分けない） |
| 比べない | — | `radio.{rx,tx}.c<n>.stream`（構成の選択），`radio.rx.block_len`（Provider の選択）．記録の `values` に書かない |

channel 数は layout から決まり，key ではない．

## 4. 比較テスト（3a，Sol N2）

置き場は `crates/ezsdr-acceptance/tests/measurements.rs`（MockRadio と UHD Module の両方に依存する crate）．`measurements/` の記録をすべて読み，記録ごとに：

1. `schema` が違えば拒否．`source: "fake"` は拒否．`profile` が `x3xx::profile_ref()` と一致しなければ失敗．
2. selector から layout を作り（`channels::from_selector`），各 Provider の `terms()` と block length で `Description` を作る：MockRadio は `BLOCK_LEN`，UHD は `DEFAULT_BLOCK_LEN` を使う（`radio.rx.block_len` は比べないので selector 値は使わない）．MockRadio の `Profile::terms()` は今 `pub(crate)` なので公開する（UHD の `profile::terms()` は公開済み）．両方の Description をそれぞれ比べる（どちらかが緩ければ失敗．stage 4 で値をそろえるまで，2 つの terms は違う）．
3. `values` に，§3 で比べる key がそれぞれちょうど 1 度現れ，知らない key は無い．
4. `measured` の値は §3 の扱いで，両 Provider の Description が記録より緩くない．
5. `conditions` が §2 の規則で layout に合い，`values` と `observations` の `condition` が `conditions` にある．

記録がまだ無いので，3a は fixture（`tests/fixtures/`）で各規則の通る場合と落ちる場合を試す．`measurements/` が空でもテストは通る（実機の記録は bench の後）．

## 5. 測定スイート（3b・3c）

- **形**：今の `rehearse_*`（`tests/common/mod.rs`．`fake.rs` と `hardware.rs` が同じ関数を FakeDevice と USRP で呼ぶ）に倣い，`measure_<名前>(device: Arc<dyn Device>, plan) -> Vec<Entry>`（`Entry` は値の行か observation）を `tests/common/` に置く．`hardware.rs` は USRP で呼んで記録を書く（出力先は環境変数）．
- **候補の値で測る**（Sol N6）：Provider を通した「最小の lead」型の値は，profile の値より小さい候補でも試さないと，緩められるかが分からない（admission と Module が profile の値で拒否する）．測定の時だけ Provider を候補の値で組み立てる道を 1 つ置く：UHD Module の `terms()` と `DEVICE_LEAD_NS` などの値を，測定用の構築（例：`UhdRadio` の test 用の builder か `assembly` の `adjust`）で差し替える．本番の値と道は変えない．各試行は候補の値と結果を記録し，結果を「admission の拒否」「Provider の失敗（`TIME_ERROR` など）」「装置の失敗（async の report）」「成功」に分ける．値の行の `worst` は P3 (1) の読み方で，候補ごとの結果は observation に残す．3b の指示書で，値ごとの差し替え方と段の刻みを決める．
- **リハーサル**（Sol N8）：`fake.rs` は同じ `measure_*` を FakeDevice で少ない回数呼び，記録が §2 の形で書けて比較テストの規則 3・5 を満たすことを見る．加えて，観測から値を出す関数（最悪値の読み方，結果の分け方）は合成した観測と制御された時計の unit test で確かめる．fake の値の大きさを壁時計で判定しない（tests/README.md）．`hardware.rs` の今の assert は共有の関数に移しても残す．
- 3b で組み直す：`hw_b8_leads`，`hw_b8_raw_device_lead`，`hw_b8_raw_restart_lead`，`hw_b8_raw_queue_depth`，`hw_b8_raw_in_flight_window`，`hw_b8_preemption`，`hw_b8_raw_burst_gap`，`hw_b8_raw_empty_eob_gap`，`hw_perf_rx_rates`，`hw_b6_delay_by_rate`，`hw_b8_cold_change_*`．raw のものは observation に書く．

## 6. 決めていないこと（各段階の指示書で）

- 3b・3c の測定ごとの読み方（P3 (1) のどの型か），候補の差し替え方，段の刻み．
- H4 の負荷の作り方（3c）．
- ホスト設定の script の置き場と sudo の扱い（3d）．

## 7. Sol のレビュー 1 回目（2026-10-26）

FAIL（major 7，minor 1）．すべて採った．N1：selector は物理の構成だけで，stream の channel・rate・同時運転は別に要る → `conditions`．N2：Description は block_len と Provider の terms でも決まり，Mock（2 ms）と UHD（5 ms）の lead が違う → 両 Provider の値で比べる．N3：key の表が足りない → §3 で全 key を扱う．N4：capability にならない結果（H1〜H5，#56 など）の置き場が無い → `observations`．N5：raw と Provider の値が同じ key で 1 度の規則と食い違う → `values` は Provider の値だけ，raw は observation．N6：admission が profile の値で拒否するので緩める向きを測れない → 候補の値で組み立てる道．N7：loopback の遅れは TX と RX の path delay に分けられない → observation．N8：リハーサルが形しか見ない → 観測から値を出す関数の unit test．

再レビュー：PASS．3a の指示書へ：条件の id は一意，`(direction, stream)` も一意，stream の番号はその向きの範囲，一つの向きで channel を 2 つの stream に入れない，`concurrent` は向きつきで宣言した stream を指す（minor）．3b の指示書へ：測定ごとに，候補を受ける admission の値と実行時の確認（`DEVICE_LEAD_NS` は control・RX・TX で別に使われ，start lead などは `x3xx::timing` から来る），据え置く定数とその理由，既定の構築が今の値のままであること，統合テストからの入り方，段と回数，成功とみなす観測（async の失敗を含む．「すぐ拒否されなかった」は成功ではない）．

## 8. 3c の分け方（orchestrator，2026-10-26．owner「推奨で作って」）

3c は §1 の表のままでは 1 段階に収まらないので 3 つに分ける．どれも試験と bench のコード（`tests/` と ignored の `hardware.rs`）で，本番の Module と profile の値は変えない．profile の値を変える判断は段階 4（P3）．3b-2 で major になった形（実機でだけ起きる完了の証拠，止まりの時刻，前提の失敗）を避けるため，各測定の成功の条件は UHD の source か今の bench の記録に当てて書き，当てられないものは INFERRED として observation に残す．

| 段階 | 中身 | 値の行 |
|---|---|---|
| 3c-1 | 送信の start lead（`cold` の送信の rate change の後，新しい clock の origin に置いた burst が on time），overflow の restart gap 単独（`with_rx_stall` で起こした overrun の gap を rate ごとに），送信の throughput（repeat を D 秒，記録された送信の区間 × rate と underflow なし） | `tx_start_lead_ns`（最小の lead），`overflow_restart_gap_ns`（全試行の最大．LowerBound で profile ≥ 測定），`tx_bytes_per_s`（容量） |
| 3c-2a | queue depth（`Device::apply` の timed な変更を先の時刻に k 個積み，返りが前の command の時刻まで止まる呼び出しで深さを読む．条件は受信・送信の周波数と利得，受信と送信を交互），coercion の read-back（利得と sample rate の格子を全部，antenna を全部，untimed の `apply` で要求して読み返す．利得の刻みは細かい掃引で．周波数は受信の全範囲と送信の `SAFE_TX_HZ` の中を抜き取り，observation だけ） | `command_queue_depth`（容量），`radio.{rx,tx}.c<n>.gain_db`・`gain_step_db`・`antenna`，`radio.{rx,tx}.s<k>.sample_rate_hz`．`frequency_hz`・`frequency_step_hz` は `unmeasured`（連続の格子を抜き取りで測ったとみなすか，送信の RF 安全の外をどうするかは owner） |
| 3c-2b | H1，H2，H5，#56，UR-17 の受信要求長の上限．H3（送信の `max_num_samps`）は `Device` に読む道が無く `src/` を変えるので段階 4 に | observation だけ |
| 3c-3 | H4 の負荷（記録の `load` を `h4` にして同じ suite を流す手順と，負荷を作る script の置き場は 3d と合わせる），2 channel の throughput と同時運転，BP4b（混在構成），BP4c（XG，10 GbE 2 本．image の書き換えは owner に確認） | 2 channel の throughput．構成ごとの記録 |

受信の throughput の値（profile 1 000 MB/s，1 channel の最大は 200 MS/s × 4 byte = 800 MB/s）は，3c-3 の 2 channel の測定を待ってから段階 4 で決める．

3c-2a の値は raw の `Device` を通すが，3b-1 の「raw の probe は値にしない」の例外にする：queue の深さと coercion は装置そのものの性質で，Provider が呼ぶのと同じ `Device::apply` で測り，判定は装置が返すもの（back-pressure で止まった呼び出し，読み返した値）だけに頼る（3b-1 の Sol major 1 は Provider の `applied` の時刻が予約でしかないことで，この道はそれを使わない）．（orchestrator，2026-10-27．owner「推奨で作って」）

**owner の決め（2026-10-27）**：
- 送信の利得の掃引（3c-2a の coercion）は最大まで．条件は，送信の streamer を開かず，LO を 1 GHz に置くこと．bench の手順では，流す前に毎回 owner に att（30 dB 以上，18 GHz 定格，TX/RX → att → RX2）が入っていることを確かめる．受信の利得の掃引は制限しない．
- 周波数の格子は，厳しい抜き取りで値にする（3c-2c）．
  - 両端と整数でない位置を含む数百点を要求し，全点が 0.05 Hz 以内で読み返せたときだけ，実機の格子を「lo〜hi，1 Hz 刻み」とする．
  - 刻みは差の最大の 2 倍とする．抜き取りの間の点は INFERRED と observation に書く．
  - 送信も OBX の全範囲で測る（sample は送らない，利得 0．att とケーブルは 18 GHz まで効く）．
- overflow の `overflow_restart_gap_ns` は最初の bench の数を見て決める．
  - 「読み直しから再開まで」の推定（3c-1）を見る．全 rate・全試行で「最大と最小の差が最大の半分以下」なら，その最大を値にし，RM-17 の文を「consumer が読み直した後に失う長さ」に直す（spec 07 の改訂）．
  - そうでなければ `unmeasured` のままにし，parity から外すかを改めて決める．
- 順番：最初の実機の流し（shake-out，profile の値には使わない）を 3c-2b の後に行い，3c-2c，3d（その結果を入れる），3c-3 の順にする．
