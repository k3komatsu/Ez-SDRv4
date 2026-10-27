# 段階 3c-2c：指示書

## 詰まりの調査の依頼（`sol-hang-brief.md`）

Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD 8f4f060; the working tree has one TEMPORARY uncommitted edit in `crates/ezsdr-radio-uhd/tests/hardware.rs` that skips the raw queue/coercion group for a bench rerun — ignore it, read HEAD's version with `git show HEAD:<path>`). READ-ONLY: no edits, no git writes, no cargo, no tests, no hardware.

What happened on the bench (X300 rev 13, OBX in slot A, HG image FPGA 39.3, UHD 4.10.0.0, 10 GbE): `hw_measurements_record` ran every probe and Provider group, then `measure_command_queue_depth` (`crates/ezsdr-radio-uhd/tests/common/measurement.rs`, stage 3c-2a) began and the run never finished. UHD printed, for 1.5 hours, alternating:
`[ERROR] [0/DDC#0::CTRLEP] Control operation timed out waiting for space in command buffer (request: ctrl_payload{dst_port:3, src_port:3, seq_num:1, timestamp:0x0000000312c786da, ..., address:0x00420, op_code:1, num_data:1 data[0]:...}`
and the same with `timestamp:<not present>` (addresses 0x00420 and later 0x00430), the timed ones' timestamps advancing ~23 s apart. Log excerpt: `/home/komatsu/.claude/jobs/8aebdd6c/tmp/s2.txt` from line 190 (do not read the whole file; grep). No RX or TX streamer was open during the queue probe. Phase 7's raw probe `hw_b8_raw_queue_depth` (`tests/hardware.rs`, results in `plan/phase7/bench-results.md` 439-452: seven timed OBX tunes queued, the eighth `apply` blocked 3 s, then an untimed apply succeeded) did not hang.

Answer, with UHD 4.10 source (host and FPGA) as VERIFIED evidence (path:line or URL), INFERRED otherwise:
1. When does an RFNoC DDC (and DUC) block execute a timed control-port command — against which time (its own timekeeper, or the timestamps of samples passing through it)? With no RX stream running, does a timed DDC command ever execute, and does the DDC's command buffer then stay full?
2. Which UHD calls does `Device::apply` (`crates/ezsdr-radio-uhd/src/uhd.rs`) make for a timed frequency or gain change on an X300 + OBX, and which of them reach the DDC/DUC (addresses 0x420/0x430: what registers)? Why would the Phase 7 probe (same frequencies 999.6/1000.4 MHz, RX and TX) not hang while 3c-2a's did — differences in conditions (gain, alternating RX/TX, the setup `apply`, the untimed reset after each trial, the 3 s horizon, call counts)?
3. What does UHD do when the control op times out (error code thrown to `apply`? command left queued?), and why did the run not end — look at 3c-2a's trial loop, the drain wait and the untimed reset, and `measure_coercion` after it.
4. Recommend a queue-depth procedure that measures the device's back-pressure without deadlocking (for example: only commands to the radio block, or with an RX stream running so DDC commands execute, or a different command), and a guard so a stuck control port fails the probe quickly instead of hanging.

Output (final message): findings per question with evidence, then the recommendation. Under about 900 words.

## 実装（`impl-3c2c-brief.md`）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．HEAD は `8f4f060`．repository は /home/komatsu/local_work/Ez-SDRv4（main の tree）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．並行して別の agent が別の git worktree で `plan/phase8/` の bench.md と script を書いている（段階 3d）．あなたは `plan/phase8/bench.md` と script に触らない．main の tree を触るのはあなただけ．他の process を待たない．

### 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる）．
- ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- `src/` は変えない（要るなら止めて報告に書く）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．長い出力を文脈に読み込まない．負荷をかけない．
- テストの一時ファイルは /tmp のまま．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ（`  {` と 3 space の key）．tree に作ったファイルは mode 644 にする（Write で作ると 600 になり，remote mutation が全部落ちる）．
- Sol（`openai/gpt-6.1-sol`）は起こさない．実機は流さない．
- **起きたことを書く．していないことは書かない．**

### 1. 背景（今日の実機の流しで起きたこと）

X300＋OBX（HG image，10 GbE）で `hw_measurements_record` を流した．
- 1 回目：受信の throughput で止まった．試験の profile が link を宣言せず HG の既定 1 GbE（100 MB/s）になり，40 MS/s 以上の Run が validate で拒否され，T0 が無く `Err` で group ごと止まった．link は `8f4f060` で直した（`tests/common/mod.rs` の `bench_profile`）．拒否された Run を分類する修正は残っている．
- 2 回目：raw の queue depth（`measure_command_queue_depth`，3c-2a）で DDC の control port が詰まった．UHD は 10 s の `op_timeout` を繰り返したが，probe はエラーを観測として次の試行と `measure_coercion` に進み，1 時間半進まなかった．記録は最後にまとめて書くので何も残らなかった．
- 3 回目：4 分半後に装置を開き直したところで UR-7 の PLL lock の失敗で panic し，記録は残らなかった．

原因（Sol が UHD 4.10 の source で確かめた．`sol-hang.md` を読む．設計メモ `plan/phase8/39-dsp-timed-commands.md` §1 にも同じ事実が既にある）：
- RFNoC の DDC/DUC の時刻つきの設定は，block を通る sample の時刻でしか取り出されない（`axi_tag_time.v:206-211`）．DDC の FIFO は 32．満杯で制御の道が詰まり，装置は再起動が要る．
- 時刻つきの周波数の変更（`Device::apply`，AUTO/AUTO）は，radio（LO，timekeeper で流れる）に加えて DDC/DUC にも補正の周波数を書く（0 でも）．利得は radio だけ．
- radio の timed command は sample 無しで timekeeper で流れる．Phase 7 の「8 個目で 3 s」は radio の queue（INFERRED）．

### 2. 作るもの

#### 2.1 queue depth の測り方を直す（`measure_command_queue_depth`）

- **条件**：
  - 受信の利得と送信の利得：radio だけに届くので stream 無しでよい．
  - 受信の周波数：受信の streamer を開いて timed start し，試行の間ずっと別の thread で `rx_recv` して捨て（drain），最後の command の時刻の後まで流してから止める．DDC の設定が sample で取り出されることを前提にする（INFERRED と observation に書く）．
  - 送信の周波数と，受信と送信の交互：外す．DUC は送信の sample が要り，連続の送信を足すと RF の扱いが増えるため．値の行の condition と observation に外した理由を書く．
- **値**：測った条件の全試行の最小（今の読み方）．値の行の condition は最小が出た条件．
- **最初の失敗で群ごと止める**：raw の group（queue と coercion）の中で `apply` か `time_now` が一度でもエラーを返したら，その handle にはそれ以上何も送らない（試行も，untimed の reset も，coercion も）．
  - 診断を observation に残し，queue と coercion の key は `unmeasured`（理由にエラー）にする．
  - 片付け（`restore_measurement_defaults`）もその handle では呼ばない．代わりに observation に「装置の電源の入れ直しが要るかもしれない」と書く．
- **時間の上限**：raw の group 全体に壁の時計の上限を置く（既定 5 分．`MeasurementPlan` に）．越えたら同じく止める．`apply` の中で 10 s 待つ UHD の timeout はそのまま（1 回で止まるので上限を越えない）．
- 合成のテスト：最初の失敗で後の操作が呼ばれないこと（FakeDevice の fault か，試験の中の `Device` の実装で数える），上限を越えたら止まること，条件ごとの読み方．

#### 2.2 記録を必ず書く（`hw_measurements_record`）

- 各 group（raw の probe の列，Provider の各測定，raw の queue と coercion）を個別に実行する．panic は `catch_unwind` で受け，`Err` と同じく「その group は失敗」として扱う．
  - 失敗した group の key は `unmeasured`（理由に失敗の文）．observation に失敗を残す．
  - 次の group へ進む．
- 全部の後で必ず記録を書き（形の検査が通れば），それから profile との比較を assert する（今の `write_record_before_profile_assert` の順）．
- raw の probe の `expected` の集合の assert は，失敗した probe を別の集合（failed）に入れて，「ran ∪ failed = expected」にする．
- 記録の途中経過：各 group の後に，それまでの observation と値を `<OUTPUT>.partial.json`（同じ dir）に上書きで書く．最後の記録が書けたら消す．形は記録と同じでなくてよい（診断用，比較テストは読まない）．
- テスト：group の失敗と panic が記録に残り，ほかの group の値が残ることを FakeDevice か純粋な関数で（hardware の要らない形で）．

#### 2.3 装置を開くときの UR-7 の再試行

`tests/common/mod.rs` の Run の組み立ては UR-7 の PLL lock の失敗に 1 回だけ再試行している（285 行あたり）．3 回目の流しはそれでも落ちた．再試行を 3 回までにし，間に 1 s 置く．試験の helper の中だけで，`src/` は変えない．

#### 2.4 拒否された Run を分類する

Provider の測定（`measure_rx_throughput`，`measure_tx_throughput`，`measure_overflow_restart_gap`，ほか Spec の Run を開く所すべて）で，Run が validate や prepare で失敗して T0 が無いとき，`Err` で group を止めない．その段（rate）の試行を `AdmissionRejection` に分類し，Run の termination の理由を observation に残す．grep で「T0」を取る全部の所を挙げて直す．

#### 2.5 周波数の格子を値にする（owner の決め 2026-10-27，設計メモ 41 §8 の末尾）

- 受信と送信の各 channel で，profile の周波数の格子（OBX は 10 MHz〜8.4 GHz，1 Hz 刻み）の両端と，整数でない MHz の位置を混ぜた数百点（既定 200）を untimed の `apply` で要求して読み返す．
  - 送信は sample を送らず，利得 0 にしてから．送信の streamer は開かない．
  - 送信の `SAFE_TX_HZ` の制限は coercion の周波数の掃引には適用しない（owner：att とケーブルは 18 GHz まで．bench で流す前に必ず att を確かめる）．
  - それ以外の送信の probe の `SAFE_TX_HZ` は変えない．
- 全点が 0.05 Hz 以内で読み返せたときだけ値にする：
  - `frequency_hz`：測った `Grid::Step { lo, hi, step: 1 }`（lo，hi は要求した両端）．
  - `frequency_step_hz`：差の最大の 2 倍．
  - 1 点でも外れたか，エラーが返れば `unmeasured`（理由）．
- 抜き取りの間の点は INFERRED と observation に書く．
- 比較テスト（`ezsdr_radio::measurement`）で，profile の格子が通る場合と落ちる場合を合成で 1 つずつ．
- 受信の `frequency_hz` の要求は全範囲（今の 50 点の observation を置き換える）．

#### 2.6 片付けを必ず走らせる

coercion の後の `restore_measurement_defaults` を drop で走る guard にする（panic でも走る）．ただし 2.1 の「最初の失敗」の後は走らせない（その handle に送らない）．

### 3. 範囲

上の 6 つだけ．H3，H4，2 channel，BP4b・BP4c，Provider と本番の道は変えない．新しい rig の種類，fake の fault の種類は足さない．試験の中の `Device` の実装（数える用）は可．大きさの目安は約 1,500 行以内．

### 4. 確かめること

- 純粋なテストは 1 回．fake のリハーサルを変えたら 20 回（時刻とスレッドに依るため）．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．
- `hardware.rs` は docker の clippy：`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`．
- mutation：判定と読み方に行を足し（ID は既存の最大の次から），新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（repo の root から．前に変えたファイルと新しいファイルを mode 644）．
  - 全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．
  - 消したコードの行は付け替えるか消す．
- spec：`design/18-uhd-radio.md` UR-34 の文とテストの名前（DA-1 の形）．Changes に 1 行（設計メモ 41，stage 3c-2c）．

### 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3c2c.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3c2c-1.log 2>&1
```

前景で流す（長いので Bash の timeout を 2,400 s 以上に）．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

### 6. 報告

`report-3c2c.md`．最初の行は `STATUS: done`．作ったもの（ファイルごと），この指示書から変えた所と理由，テスト名，bench の時間の見積もり，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．最後のメッセージは報告の要約．

## セルフレビューの checklist（`checklist-3c2c.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `8f4f060`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3c2c-brief.md`（§2 の表が仕様）．

見ること（4つ）：
1. **測定とテスト**：表の key ごとに，`measure_*` が表の「1 試行」「成功」「読み方」のとおりか．最初の失敗の後にその handle へ何も送らないか，記録が group の失敗と panic の後も必ず書かれるか，拒否された Run が分類されるか，周波数の格子が 1 点の外れで `unmeasured` になるか．fake のリハーサルが時刻に依る assert（分類の結果，値の大きさ）をしていないか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．`src/` の変更，H3・H4・2 channel の測定，`plan/phase8/bench.md` と script への変更，新しい rig の種類・fault の種類，指示書 §2.3 以外の retry，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：測った key だけが `Measured` で，他の key は理由つきの `unmeasured` か．`source: "fake"` の記録で `Measured` が無いか．`frequency_hz`・`frequency_step_hz` は全点が 0.05 Hz 以内のときだけ値になるか．送信の側で streamer を開かず利得 0 で周波数を掃引するか．受信の周波数の queue の条件で受信の stream が最後の command の後まで流れて drain されるか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3c2c-brief.md`）

tree の未 commit の変更は段階 3c-2c の実装です．orchestrator のレビュー（`orch-3c2c.md`）と Sol のコードレビュー（`sol-3c2c.md`）の指摘をすべて直します．`impl-3c2c-brief.md` §0 の守ることはそのまま効く．Sol は起こさない．セルフレビューも回さない（Sol が再レビューする）．

### 1. 裁定（全部採る）

- **Sol 1（major）：受信の周波数の条件で，DDC の設定が sample で取り出されたことを確かめていない．**
  - 設定を出す前に，受信が sample を返していること（最初の `Samples`）を確かめる．
  - drain の thread は返った sample の終わりの tick（`first_tick` ＋ 数）を共有の値に進める．`LateCommand`・overflow・続く timeout も記録する．
  - 試行は「受け取った sample の終わり ≥ 最後の命令の時刻 ＋ 余裕」になるまで受信を止めない．上限の時間までに届かなければ，その試行は失敗にし，群を止める（受信が進まない ＝ DDC の設定が残りうる）．
  - 合成のテスト：開始の遅れ（LateCommand の後 timeout），受信の停止，区間が最後の命令に届かない場合を，時刻に依らない同期（試験の `Device` で返す列を決める）で．`rx_recv > 0` のような thread の順に依る assert をやめる．
- **Sol 2（major）：最初の失敗の後にも呼び出しが残る．**
  - 一つの失敗の latch（`Arc<AtomicBool>` など）を，命令，drain，片付け（`rx_stop`・`rx_close`・restore の各 channel）で共有し，各呼び出しの前に見る．失敗の後は何も始めない．すでに飛んでいる呼び出しは待つだけ．
  - 失敗した handle の破棄：`UhdDevice` の drop は UHD の teardown を呼ぶ（`src/uhd.rs:633-658`）．失敗の後はその handle を `std::mem::forget` で捨て，drop を走らせない（注釈：`shortcut: the wedged handle is leaked so its teardown does not touch the control port; a power cycle is needed anyway`）．
  - 試験の `Device` の呼び出しの記録に `rx_recv`・`rx_stop`・`rx_close`・drop を足し，失敗の後に何も無いことを確かめる．
  - `design/18` UR-34 の文を実際の振る舞いに合わせる．
- **Sol 3（major）：命令の thread の panic で drain の thread が止まらず，記録が書けない．**
  - drain の取り消しを，unwind でも scope の join より前に走る guard にする．
  - テスト：時刻つきの `apply` で panic する試験の `Device`（受信は流れている）で，群が失敗になり記録が続くこと．
- **Sol 4（major）：時間の上限を各呼び出しの前に見ていない．**
  - 上限の状態を queue の送信と drain に渡し，次の装置の呼び出しの前に毎回見る．
  - テスト：二つの呼び出しの間で上限が来たとき，二つ目が呼ばれないこと．
- **Sol 5（major）：記録の仕上げが失敗した装置に頼る．**
  - 装置の識別（serial など）と binding・description は，測定の group の前に読んで持っておく．
  - 共有の `usrp()` の open と準備の失敗も group の失敗として記録する．
  - テスト：group の後で装置が使えなくなっても記録が書けること．
- **Sol 6（minor）**：UR-7 の再試行は PLL の lock の失敗（`Reference Clock PLL failed to lock`）だけにする．ほかのエラーはすぐ返す．テスト：PLL で再試行して成功，回数切れ，再試行しないエラー．
- **O1（minor，orchestrator）：記録が大きすぎる．**
  - 2026-10-27 の記録は 157 MB だった．#56 の 2 つで 68 MB，UR-17 で 16 MB，H5 で 13 MB．
  - probe の observation は要約だけにする：数，最小・中央・最大，分類ごとの数，最初と最後の数行と外れた行．
  - 行の全部は記録の外の別ファイル（OUTPUT と同じ dir の `<OUTPUT>.rows/<probe>.jsonl`）に書く．
  - 目安は記録全体で 1 MB 以下．`.partial.json` も同じ要約で．
  - 純粋なテストで，要約が行を持たないことと，別ファイルの道．

### 2. 確かめること

impl の §4 と同じ：
- `cargo test --workspace`，clippy，`rules.py --base HEAD`，`git diff --check`，docker の `--features uhd` の clippy．
- fake のリハーサルを変えたら 20 回．
- 新しい行と触れた行の remote mutation．全 list の `old` が 1 回一致．

### 3. 報告

`fix-3c2c-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．最後のメッセージは報告の要約．**起きたことを書く．していないことは書かない．**

## 修正 2（`fix2-3c2c-brief.md`）

Sol の再レビュー（`sol-fix-3c2c.md`）の残りを直します．`impl-3c2c-brief.md` §0 の守ることはそのまま効く．Sol は起こさない．セルフレビューは回さない．

### 1. 裁定

- **major 1（採る）：片付けの失敗で handle を捨てない．**
  - 片付け（restore）の後の latch の状態で，handle を捨てるかを決める．
  - 記録を書く所の失敗や panic でも捨てる判断が効くよう，unwind に強い形にする．
  - テスト：呼ぶ側の流れ全体（片付けが失敗 → 記録 → 捨てる，drop が走らない）を確かめる．`leak_after_stop` だけの試験にしない．
- **major 3（採る）**：`rx_stop` が成功した後，`rx_close` の前にも latch と上限を見る．テスト：上限を進める scripted な `rx_stop` の後に `rx_close` が呼ばれないこと．
- **minor（採る）**：上限のテストを，注入した経過時間（試験の中で進める）にし，実時間の sleep と thread の順に依らないようにする．
- **major 2（一部だけ採る）：thread の間の呼び出しの競合．**
  - 二つの thread の呼び出しを一つの lock で直列にすることはしない．`apply` が止まる長さ（queue depth の測り方そのもの）を変えるため．
  - 代わりに保証を正確に書く：「ある thread が失敗を latch した後，ほかの thread で新しく始まる装置の呼び出しは高々 1 つ（latch を見た後に呼ぶまでの間に入るもの）．それも UHD の 10 s の timeout で終わる．それより後の呼び出しは無い」．
  - `design/18` UR-34 の文，code の注釈，observation をこの保証に合わせる．
  - テスト：thread の順に依らない形にする．試験の `Device` で失敗の時点と呼び出しの順を決め（barrier などの同期），「失敗の後に各 thread で高々 1 つ」を両方の向きで確かめる．
  - 失敗の印が latch より前に出る順も，テストの判定が順に依らない形に直す．

### 2. 確かめること

前の回と同じ：
- `cargo test --workspace`，clippy，docker の `--features uhd` の clippy，`rules.py --base HEAD`（名前を変えたテストがあれば `--renames`），`git diff --check`．
- 時刻とスレッドに依るテストを変えたら 20 回．
- 新しい行と触れた行の remote mutation．全 list の `old` が 1 回一致．

### 3. 報告

`fix2-3c2c-report.md`．最初の行は `STATUS: done`．最後のメッセージは報告の要約．**起きたことを書く．していないことは書かない．**
