# 段階 3d：指示書

## 実装（`impl-3d-brief.md`）

あなたは実装役です．**作業する場所は git worktree `~/.cache/ezsdr-fixes/wt-3d`（branch `stage-3d`，HEAD `8f4f060`）だけ．** main の tree（/home/komatsu/local_work/Ez-SDRv4）は別の agent が段階 3c-2c で編集中なので，読むだけにして書かない．作業 dir（指示書と報告）は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．他の process を待たない．

### 0. 守ること

`cargo fmt` 禁止，commit・push・`git add`・`git stash` 禁止，version・SCHEMA_CHANGELOG に触らない．`crates/`・`design/`・`handoff.md` を変えない（handoff の §6 は orchestrator が gate で直す）．出力はファイルに出して `tail`・`grep` で見る．tree に作ったファイルは mode 644（script は 755）．Sol（`openai/gpt-6.1-sol`）は起こさない．実機と sudo は使わない（script は書くだけ，`bash -n` と dry-run で確かめる）．**起きたことを書く．していないことは書かない．**

### 1. 読むもの

- `plan/phase8/41-measurement-records.md` §5〜§8（測定の記録，段階の分け方，§8 の末尾の owner の決め 2026-10-27）．
- `handoff.md` §6（6.2 の装置と NIC，6.3 のホストの設定：ASPM，MTU 9000，sysctl〈wmem_max 32 MiB，rmem_max 50 MB，netdev_max_backlog 30000〉，NIC のリング，NetworkManager から外す，docker の実行の形）．
- tree の外の今の手作業の道具（読むだけ）：`~/ezsdr-bench/usrp-net.sh`（ホストの設定），`~/ezsdr-bench/run-bench.sh`（commit の snapshot と docker の volume `ezsdr-v4-bench-target` で hardware の test を流す），`~/ezsdr-bench/NOTES.md`．
- `plan/phase7/bench.md`（RF safety，B0〜B9 の形），`plan/phase8/bench-results.md`（Phase 8 の記録の置き場），`crates/ezsdr-radio-uhd/tests/common/measurement.rs` の `REQUIRED_ENV` と `process_metadata`（8 つの `EZSDR_MEASUREMENT_*`），`MeasurementPlan::process_env`（`EZSDR_MEASURE_*`），`tests/hardware.rs` の `hw_measurements_record`．
- 今日（2026-10-27）の実機の流しの手順のメモと log（tree の外）：`~/.cache/ezsdr-bench/2026-10-27/procedure.md`，`shakeout-1.log`〜`shakeout-4.log`（長い．`grep` で）．`~/.cache/ezsdr-fixes/2026-10-26-stage3/sol-hang.md`（DDC の詰まりの原因）．

### 2. 作るもの

1. **`plan/phase8/tools/bench-host.sh`**：`usrp-net.sh` を tree に移し，handoff §6.3 の全部（ASPM，MTU，sysctl，リング，NetworkManager）を当てる．
   - 既定は確かめるだけ（今の値を表示し，足りない所を挙げて exit 1）．`--apply` で root として当てる．
   - 冪等．X300 の subnet は引数か環境変数．
   - 最後に設定の version の文字列（script の中の定数．例 `bench-host 1`）と当たった値を 1 行で出す．これを `EZSDR_MEASUREMENT_SETTINGS_SCRIPT`・`_VERSION` に使う．
2. **`plan/phase8/tools/run-record.sh`**：`hw_measurements_record`（と指定した hardware の test）を流す．`run-bench.sh` の形：
   - 指定した commit の snapshot を作業 dir に取り出す．
   - 専用の docker の volume で build する（main の `ezsdr-v4-cargo-target` と分ける）．
   - `bench-host.sh` の確かめるだけの形が通らなければ止める．
   - 8 つの metadata を引数・環境変数から組む．commit は snapshot の hash．mboard と FPGA は `uhd_usrp_probe` の出力から取れるなら取る．
   - 出力は repo の外（既定 `~/ezsdr-bench/<日付>/`）．log も同じ所に．
   - **流す前に「att（30 dB 以上）が TX/RX → att → RX2 に入っているか」を対話で確かめ，`yes` 以外なら止める**（owner の決め：人が間違えるので毎回．`--att-confirmed` のような省略の道は作らない）．
3. **`plan/phase8/bench.md`**：Phase 8 の bench の手順．
   - 機材と配線，RF safety（Phase 7 の規則に加え，coercion の送信の利得の掃引〈最大まで，streamer なし〉と周波数の掃引〈全範囲，利得 0，sample なし〉が入ったこと，att の定格），att の確認．
   - ホストの設定（script），装置の確認（`uhd_find_devices`，`uhd_usrp_probe`，FPGA の互換），`hw_b1_probe`，記録の流し（script），`.partial.json`（3c-2c で入る途中経過）の扱い．
   - **DDC の詰まり**：時刻つきの周波数を受信が流れていない chain に積むと詰まり，電源の入れ直しが要る（設計メモ 39 §1，今日の 2 回目）．症状（`Control operation timed out waiting for space in command buffer` の繰り返し）と対処．
   - PLL の lock の失敗，見込みの時間，記録をどこに置くか（`measurements/` は比較テストが読むので，shake-out の記録は置かない），profile の値に使うまでの流れ（段階 4）．
   - 今日の shake-out で分かったことを「既知の癖」として：link の宣言（`link_gbps`），HG の 10 GbE，`ref_locked` が電源の入れ直しで true になったこと．
4. **`plan/phase8/bench-results.md`** に 2026-10-27 の節：機材（X300 #1 347D545＋OBX，HG 39.3，40 dB の att，`enp1s0f0np0`，192.168.40.36），流した 4 回（何が止めたか，直したこと：`8f4f060`，一時的に外したもの），取れた主な観測（log から．device lead，in-flight，burst の隙間，timed tune の位相のずれ，loopback の遅れ，UR-17，H2・H5・#56 の行），残ったこと．
   - 4 回目の結果は orchestrator が後で足す．節の最後に「4 回目：（未記入）」と置く．

### 3. 範囲

上の 4 つだけ．Rust のコードは変えない．

### 4. 確かめること

`bash -n` で両方の script，`bench-host.sh` を確かめるだけの形で 1 回流す（root でなく）．`run-record.sh --help` が通ること，att の確認で `no` と答えると止まること（docker と実機に進まない形で）．`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（worktree の中で），`git diff --check`．

### 5. セルフレビュー（1 回）

```
cd ~/.cache/ezsdr-fixes/wt-3d && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3d.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3d-1.log 2>&1
```

free が失敗したら Go（`opencode-go/deepseek-v4.1-flash#max`）で同じ回．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

### 6. 報告

`report-3d.md`．最初の行は `STATUS: done`．作ったもの，指示書から変えた所と理由，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．

## セルフレビューの checklist（`checklist-3d.md`）

あなたは読むだけの reviewer です．git worktree `~/.cache/ezsdr-fixes/wt-3d` の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `8f4f060`）を見る．ファイルを書き換えない．script を root で流さない．docker と実機に触らない．

読むもの：`~/.cache/ezsdr-fixes/2026-10-26-stage3/impl-3d-brief.md`（§2 が仕様）．

見ること：
1. **script**：`bench-host.sh` が既定で何も変えないか，`--apply` が handoff §6.3 の全部を冪等に当てるか，誤った NIC を触る道が無いか．`run-record.sh` が att の確認を省けないか，main の cargo の volume を使わないか，出力を repo の中（特に `measurements/`）に書かないか，metadata の 8 つを全部組むか．
2. **bench.md**：RF safety，att の確認，DDC の詰まりと電源の入れ直し，PLL の失敗，記録の置き場が書いてあるか．書いてあることが repo のコードと合っているか（環境変数の名前，test の名前）．
3. **bench-results.md**：2026-10-27 の節が log と合っているか（数字を log で確かめる）．
4. **範囲**：`crates/`・`design/`・`handoff.md` を変えていないか．

出力は日本語．指摘ごとに P1（誤り・安全）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3d-brief.md`）

作業する場所は git worktree `~/.cache/ezsdr-fixes/wt-3d` だけ（main の tree には書かない）．Sol のレビュー（`sol-3d.md`）の指摘をすべて直し，2026-10-27 の 4 回目と D9 の結果を bench の記録に足します．`impl-3d-brief.md` §0 の守ることはそのまま効く．Sol は起こさない．セルフレビューも回さない．

### 1. 裁定（全部採る）

- **Sol 1（major）**：`run-record.sh` の snapshot を `tar -xm`（新しい時刻）で取り出す（元の `run-bench.sh:24-25` と AGENTS.md §7）．同じ volume を使う build を直列にする（lock file か，実行中なら止める）．二つの違う snapshot を同じ cache で流して二つ目の振る舞いになることを確かめる試験は，docker を使わない形で書けなければ bench.md の手順に書き，報告に残す．
- **Sol 2（minor）**：指定した test の名前を snapshot の test の一覧（`cargo test ... -- --list` の出力）と照らし，無ければ止める．または結果の出力で「1 passed」をちょうど確かめる．
- **Sol 3（minor）**：bench.md の手順で，ホストの設定の確認と適用を装置の発見と probe より前に置く．frame の大きさの確認は設定の後．

### 2. bench-results.md に足す（節の最後の「4 回目：（未記入）」を置き換える）

**4 回目**（電源を入れ直した後．raw の queue と coercion は一時的に外した．commit `8f4f060`＋ローカルの一時的な変更）
- 最後まで走り，記録を書いた（`~/.cache/ezsdr-bench/2026-10-27/shakeout-4.json`，157 MB．大半は #56・H5・UR-17 の装置の呼び出しの行で，3c-2c で要約に減らす）．
- 2,155 s．profile との比較で落ちたのは 2 つ．受信の throughput（測定 800 MB/s，profile 1,000 MB/s）と，送信の throughput（測定 200 MB/s，profile 1,000 MB/s）．
- 送信は 100 MS/s で約 29 s 流した後に Underflow，200 MS/s ではすぐ Underflow．
- 電源の入れ直しの後，B1 の `ref_locked` は true になった（それまでは false）．
- 測った値：

| key | 値 | 条件 |
|---|---|---|
| `radio.full_duplex`，`radio.hardware_time` | true（20/20） | duplex-1msps |
| `radio.perf.rx_bytes_per_s` | 800 000 000 | rx-200msps |
| `radio.perf.tx_bytes_per_s` | 200 000 000 | tx-50msps |
| `radio.timing.min_timed_command_lead_ns` | 5 000 000 | tx-1msps，100 試行 |
| `radio.timing.tx_in_flight_ns` | 3 000 000 | tx-1msps，100 試行 |
| `radio.timing.rx_start_lead_ns` | 1 000 000（一番小さい候補） | cold-1msps-2msps，100 試行 |
| `radio.timing.tx_start_lead_ns` | 1 000 000（一番小さい候補．構成が間に合った証拠で，burst は見ていない） | tx-cold-1msps-2msps，100 試行 |

- overflow：gap 全体の最大 13.97 s（stall を含む），読み直しから再開までの推定の最大 75.0 ms（1 MS/s，INFERRED，大きめ）．

**D9 の probe**（設計メモ 39 §3．`8f4f060` の写しから別の volume で build）

| probe | 結果 |
|---|---|
| `hw_d9_p1_ddc_entries_wait_for_samples` | 受信していない channel に時刻つきの DDC の shift を 20・40・60 kHz 渡し，時刻なしで 0 に戻してから受信を始めると，tone は約 160 kHz．残った時刻つきの最後の設定（60 kHz）が時刻なしの 0 を上書きした（0 に戻したのが設定の時刻の前でも後でも同じ）．3 つは最初の数 sample で 1 つずつ取り出された（設計メモ 39 の F2 を確認） |
| `hw_d9_p2_duc_entry_after_its_samples` | burst を送る前に時刻つきの DUC の shift（e は開始の 20 ms 後）を渡すと，tone は e の約 19 ms 前（burst の最初）から動いた．頭の sample を送った後，e の 3 ms 前に渡すと e の +25 µs で動いた |
| `hw_d9_p3_duc_entries_between_bursts` | burst の間の時刻つきの +50 kHz は次の burst に正しく当たった．時刻つきの +80 kHz の直後に時刻なしの 0 を渡すと，次の burst は +80 kHz（時刻つきが時刻なしを上書きした．F4 を確認） |

### 3. 確かめること

`bash -n`，`run-record.sh --help`，att の確認で `no` で止まること，test の名前の照合が誤った名前で止まること（docker と実機に進まない形で確かめられる範囲で），`rules.py --base HEAD`，`git diff --check`．

### 4. 報告

`fix-3d-report.md`．最初の行は `STATUS: done`．直したこと（`path:line`），確かめたことの結果，残ったこと．
