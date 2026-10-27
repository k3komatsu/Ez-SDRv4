# 段階 3d：報告

## 実装の報告（`report-3d.md`）

STATUS: done

## 段階 3d 報告

### 作成・変更したもの

- `plan/phase8/tools/bench-host.sh`：明示した X710 NIC の read-only check と `--apply` を実装．usage に，PCIe-link ASPM と host 全体の sysctl も変更対象であることを明記した．mode 755．
- `plan/phase8/tools/run-record.sh`：commit snapshot，専用 Docker target volume，att の対話確認，metadata と外部 output/log を実装．mode 755．
- `plan/phase8/bench.md`：BP 手順，配線・RF 安全，X300 切替時の NIC address 処理，記録の保存・比較テストへの登録，失敗時の対応を記載．mode 644．
- `plan/phase8/bench-results.md`：2026-10-27 節を 2026-10-21 節の後に配置し，shake-out 1〜3 のログに合わせて記述を修正．末尾は指定どおり `4 回目：（未記入）` のまま．mode 644．

### 指示書からの変更・補足

- `run-record.sh` の既定出力先は指示書どおり `$HOME/ezsdr-bench/<日付>/` のままにした．DeepSeek の「すべて `$HOME/.cache/ezsdr-bench` に統一」案は採用しなかった：それは既存の手動 shake-out log の置き場であり，Phase 7 の `run-bench.sh` と今回の指示書が指定する scripted run の置き場とは用途が異なるため．`bench.md` に両者の区別を記した．
- 最終 BP2/BP3 record を比較テスト用 `measurements/` に owner が手動で登録する手順を追加した．script は repo 内へ書かず，shake-out と partial は登録しない．
- `bench.md` に attenuator の最大入力電力確認と型番・電力定格の記録を追加した．今回の 40 dB / 18 GHz attenuator の型番・最大入力電力は元の記録に無いため，結果にも「未記録」とした．

### 確認したこと

- `bash -n`：両 script とも成功．
- `bench-host.sh --nic enp1s0f0np0 --subnet 40`：root なしの read-only check が成功し，MTU 9000，ASPM 0，RX/TX ring 8160，指定 sysctl 値，NetworkManager unmanaged を表示．`--apply` は実行していない．
- `run-record.sh --help`：成功．att 確認へ `no` を入力：exit 1，Docker/device access より前に停止する文言を確認．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problem．`git diff --check`：成功．
- 差分は指定の 2 scripts と 2 Phase 8 Markdown files のみ．script は 755，Markdown は 644．commit/push/stage は行っていない．
- Docker，root/sudo，実機は使用していない．

### DeepSeek セルフレビュー（1 回）

`review-3d-1.log` の指摘は **P1 1 件，P2 5 件，P3 6 件**．扱いは以下のとおり．

1. **P1 — Burst gap の欠落値：修正．** empty-EOB 条件の gap 5 は shake-out 2 で 1/4 late（`Underflow+TimeError`），shake-out 1・3 は 0/4 と追記し，各測定 gap の run 別結果を明記した．
2. **P2 — #2 切替時の重複 host address：修正．** `bench.md` に，旧 NIC 上の `192.168.40.10/24` を確認し，残っていればその address だけを明示的に削除してから #2 NIC を check/apply する手順を追加した．
3. **P2 — output/log の置き場：既定変更案は退け，説明を修正．** 指示書に従い scripted run は `~/ezsdr-bench/<日付>/` を維持．既存の 2026-10-27 手動 shake-out log は `~/.cache/ezsdr-bench/2026-10-27/` にあることを区別して記載した．
4. **P2 — 最終 record と比較テスト：修正．** 完了した BP2/BP3 の最終 record を指定の `crates/ezsdr-radio/measurements/` に手動登録する流れを追加し，shake-out/partial は登録しないと明記した．
5. **P2 — 250 µs in-flight の run 1：修正．** shake-out 1 の underflow 0 / TIME_ERROR 0 を加え，run 2・3 の値と並べて run 間の不一致を記載した．
6. **P2 — DDC request address：修正．** 最初の失敗 request が timestamp 付き `0x00420`，その後 timestamp なし `0x00430` の request が続いたことと各件数を反映し，FIFO の説明を repo 内の設計メモ 39 §1 にリンクした．queue-depth 条件の変更説明を DDC の run 2 に移した．
7. **P3 — host 設定の影響範囲：修正．** script usage と bench 手順に，選択 NIC，関連 PCIe-link ASPM，host 全体の sysctl 3 値が対象と明記した．
8. **P3 — TX frequency sweep の条件：修正．** coercion の `SAFE_TX_HZ` read-back と，3c-2c の全範囲 frequency-grid（gain 0，sample なし）を分けて記した．
9. **P3 — UR-17 最大時間：修正．** 表記を 178.6 ms から 178.5 ms に訂正した．
10. **P3 — 4 回目と表の行数：修正．** 見出しを記録済みの 1〜3 回目にし，4 回目の内容は追加せず，所定の placeholder を節末に残した．
11. **P3 — 外部資料への参照と DDC 記述位置：修正．** `sol-hang.md` / 3c-2c brief への依存を除き，DDC FIFO の説明を repo 内の設計メモ 39 §1 にリンクして run 2 にまとめた．
12. **P3 — attenuator の電力定格：修正・未確認を明記．** 次回 TX 前に最大入力電力と余裕を銘板/仕様で確認し，型番・電力定格を記録する手順を追加．今回の記録にその値がないことを記載した．

### 残ったこと

- 4 回目の shake-out 結果は依頼どおり未記入で，placeholder のまま．
- attenuator の型番と最大入力電力は今回の記録から確認できず，次回送信前の確認が必要．
- hardware/Docker を使った run は行っていない．

## 修正の報告（`fix-3d-report.md`）

STATUS: done

## 段階3d 修正報告

### 変更

- `plan/phase8/tools/run-record.sh:61-67,79-80`：共有 Cargo target volume の実行を host 側 non-blocking `flock` で直列化し，commit snapshot を `tar -xm` で展開する．Docker-free の cache freshness 手順を `plan/phase8/bench.md:56-80` に記録した．toy crate の同一 target test では，古い mtime の B snapshot は A の binary を再利用し，`tar -xm` 後は B を実行した．
- `plan/phase8/tools/run-record.sh:97-107,110`：snapshot の `cargo test -- --list` と要求 test 名を照合し，不一致なら `uhd_usrp_probe` より前に停止する．`plan/phase8/tools/run-record.sh:148-156` では preflight がちょうど1件成功したことも確認する．
- `plan/phase8/bench.md:12-31,35-37`：host 設定の check/apply を device discovery・probe より前に移動し，最大 frame size の確認も設定後にした．旧 tree 外 `~/ezsdr-bench/run-bench.sh` は同じ volume の lock を取らないため，同時実行しない旨も記載した（同 script は指示により編集していない）．
- `plan/phase8/bench-results.md:109-143`：4回目の測定・失敗理由・主要観測と D9 の3 probe の結果を追加した．

Sol の指摘3件はすべて反映した．commit/push はしていない．`crates/`，`design/`，`handoff.md` は変更していない．

### 確認

- `bash -n`（両 script）：pass．`run-record.sh --help`：pass．両 script は mode 755．
- `bench-host.sh --nic enp1s0f0np0 --subnet 40` の non-root read-only check：exit 1．対象 NIC が carrier なし / operstate down のためで，設定変更はしていない．
- att 確認に `no`：exit 1，Docker/device access 前に停止（fake Docker の trace は空）．
- test 名の照合：fake Docker の test list にない `hw_typo_is_not_real` を exit 2 で拒否し，probe と measurement を呼ばないことを確認．lock 占有時も exit 1 で Docker 前に停止した．
- Cargo freshness regression：Docker・実機なし，同じ作業 directory と target で `-xf` は古い A を再利用し，`-xm` は B に更新することを確認．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problems．`git diff --check` と変更ファイルの trailing-whitespace 検査：pass．

Sol の再レビュー・セルフレビュー，Docker image 内の build/test，sudo `--apply`，実機試験は指示どおり実施していない．

### 残り

- 実機の設定適用と Docker/UHD 上の実行は未検証．旧 `run-bench.sh` を新しい runner と同時に使わないこと．
- 4回目の記録 suite は JSON を書いたが profile throughput 比較で失敗した（RX 800 MB/s，TX 200 MB/s に対し profile は各1,000 MB/s）．`overflow_restart_gap_ns` は `unmeasured` のまま；profile の判断と再測定は段階4．
- attenuator の型番・最大入力電力は記録されておらず，次の送信前に確認が必要（bench-results.md §2026-10-27 の機材記録）．
