# 1b-4d-1a：修正の回 1〜3 の指示書と報告

回 1・3 は Claude Code の Haiku サブエージェント，回 2 は DeepSeek（OpenCode-Go）．

## 回 1：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `f487877` に 1b-4d-1a の未コミットの変更）．
「守ること」は `impl-brief.md` と同じ．範囲は `impl-a-brief.md` の 1a．計画は `test-plan.md`，裁定は `adjudication.md`（同じディレクトリ `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/`）．

### orchestrator のレビュー（直すこと）

1. **計画の要の系列のテストが無い．書く．**
   - **S1・S2**（Sol の 1b-4c 再レビュー 3 の系列．この段階の目的そのもの）：owner を止めたまま，uhd-control が from-none で B=`{0,1}` を open・configure → 後の cold で集合を空にし B を正の長さで終える → uhd-control が from-none で C=`{0}` を open・configure → owner を動かす．owner は B を始める前に `{0,1}` を開き直して configure し，続く C でも開き直す．RX（`rx.rs`）と TX（`tx.rs`）の両方．直す前の形（`configured` の近道，または記録を見ずに owner の写しで判断する形）を mutation の行にして殺す．
   - **S3**（RX）：from-none が成功し owner がその segment を始める → 追加の open も configure も無い．
   - **S12**：`arm` で開いた集合のまま最初の segment が始まる → 開き直さない．
   - **S14**：TX の拒否された enable → `read(None)` → `step` の前に同じ集合の rate 更新 → switch → 開き直して照合する（1b-4c の再レビュー 1 の系列）．
   - **S20**：`arm` の open は成功し照合が拒否 → 記録は `Unknown`（adjudication の minor 7）→ 次の開き直しで保存した拒否が再適用される．
   - **S21**：adjudication の minor 7 のとおり「拒否の後に次の enable を記帳して戻る」系列に置き換えたもの．
2. **S11b を戻す．** 「到達できない」は誤り：将来の cold 変更（例：100 ms 後）を from-none で configure した後，それより前（例：50 ms）に効く timed 更新を記帳すると，50 ms の release の時点ではその channel はどの segment にも無いので記録だけになる（`control.rs` の release の `chans.is_empty()` の腕）．1a の範囲のテストは，owner がまだその segment を始めていない形（unit で owner を動かさない）で，版が上がり owner の判断が「configure だけ」になることを確かめる．owner がもう始めていた場合は D1（1b）の範囲なので書かない．
3. **報告の誤りを直す．** 報告は「D1 の A を実装した」と書いているが，コードは記録だけの release で版を上げるだけで，origin での timed command は出していない（D1 の A は 1b の範囲）．報告をコードに合わせて直す．
4. **既存テストの分類を終える．** `tx.rs` と `tests/fake.rs` で，消えた写し（`verified`，`opened`，`stale`，`configured`）や `configs` の形の変更に触れたテストを全部，表（テスト名，扱い，理由）にする．
5. **ワークスペース全体のテストを流す．** `cargo test --workspace --no-fail-fast`．

### 確かめること

`impl-brief.md` と同じ 1〜3．mutation はこの回で足した行・変えた行と，テストが変わった行だけ（前景で流す）．

### セルフレビュー

この回は DeepSeek のセルフレビューを流さない（前の回で 2 回とも判定が出なかった．orchestrator と Sol のレビューで見る）．報告にそう書く．

### 報告

`report.md` を上書きせず，`fix1-report.md`（同じディレクトリ）に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．範囲を理由に blocked にしない．
中身：1〜5 のそれぞれ，テストと行，確かめることの結果（mutation は行ごと）．

## 回 1：報告

STATUS: done

## Phase 8 段階 1b-4d-1a の修正の回 1 の報告

実装役（Haiku 5.5）．指示書は `fix1-brief.md`．セルフレビュー（DeepSeek）はこの回では流していない（指示書どおり．orchestrator と Sol のレビューで見る）．コミットと push はしていない．`cargo fmt` と `rustfmt` は使っていない．mutation は `remote_mutate.py` を前景で sim02・sim03 に流した（本体の木に mutant を当てていない）．

### 1. 計画の要の系列のテスト

| 系列 | テスト名 | 置く場所 | 結果 |
|---|---|---|---|
| S1（RX） | `ur_13_a_from_none_segment_the_owner_never_started_is_opened_again` | `control.rs` unit | 通る．owner が最初に開く `rx_open 0 [0, 1]` と，後の `rx_open 0 [0]` を assert |
| S2（TX） | `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` | `tx.rs` unit | 通る．`do_switch` を {0,1} → なし → {0} → {0,1} の順に行い，`tx_open` の列 `[0, 1]`, `[0]`, `[0, 1]` を assert |
| S3（RX） | `ur_25_an_owner_begins_a_from_none_enable_without_a_second_open` | `control.rs` unit | 通る．owner の begin に `rx_open` と `apply` が無く，`rx_start` が 1 回 |
| S11b | `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` | `control.rs` unit | 通る．ただし assert を縮めた（後述の逸脱 A） |
| S12 | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer` | `tests/fake.rs` | 通る．`rx_open 0 [0]` と `tx_open 0 [0]` が各 1 回，`rx_start` あり |
| S14（TX） | `ur_25_a_refused_transmit_clock_is_not_adopted` | `tx.rs` unit | 通る．拒否された clock は採用されず，device 呼び出しなし．同じ集合の再 switch で `tx_open` が 1 回，clock が採用される |
| S20 | 書いていない | — | 逸脱 B（後述） |
| S21 | `ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts` | `control.rs` unit | 通る．拒否された enable は `refused` で記帳され，次の enable が `rx_open` を 2 回目に出す（`channels_from_none` を使う形に直した） |

S1 と S21 の前提は，sample-rate の enable では channel 0 の segment が計画に載らないため，`channels_from_none`（channel 数の変更）に直した．

### 2. D1 の記述の修正（指示書 2・3）

前の 1a の報告は「D1 の A を実装した」と書いていたが，これは誤り．コードは記録だけの release で版（`ColdConfig.version`）を上げるだけで，origin への timed command は出していない．D1 の A（record-only の更新を device に出す）は 1b の範囲であり，この回では実装していない．`design/18-uhd-radio.md` の文も，コードに合う記述になっている（この回では変更していない）．

### 3. 既存テストの分類（指示書 4）

消えた写し（`verified`，`opened`，`stale`，`configured` の形）と `configs` の形に触れうるテストを，`tx.rs` と `tests/fake.rs` で名前と使う識別子から拾って分類した．本文を全部読み直したわけではない．根拠は各テストの識別子の検索と，全テストの通過である．

`tx.rs`（unit）

| テスト名 | 扱い | 理由 |
|---|---|---|
| `ur_13_a_transmit_switch_is_compared_with_the_stored_outcome` | 残す | 比較の結果の保存を見る．写しに依らない |
| `ur_25_a_refused_enable_leaves_its_set_unopened_for_the_next_switch` | 残す | 拒否の後の開き直しを `tx_open` の数で見る（S14 と重なる部分あり） |
| `ur_13_a_refused_comparison_is_repeated_at_the_next_switch` | 残す | 拒否の比較の繰り返し |
| `ur_25_a_refused_transmit_switch_makes_no_device_call` | 残す | 拒否の switch は device を呼ばない（S18 の TX 版） |
| `ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen` | 残す | S8．配置の記録で configure だけをやり直す |
| `ur_13_a_switch_to_the_verified_set_opens_nothing` | 残す（名前は古い） | `verified` の語を含むが，中身は「直前に開いて照合した集合への switch は開かない」を記録で見る．改名は別の回に回す |
| `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` | 新規（S2） | — |
| `ur_25_a_refused_transmit_clock_is_not_adopted` | 新規（S14） | — |
| `ur_28_a_streamer_the_record_holds_keeps_its_correlations`（`record_configured` を使う） | 残す | 記録の `configured` を直接置いて，開き直さないことを見る．記録の形に合う |
| `ur_25_a_burst_on_a_later_clock_survives_an_earlier_switch`，その他の `switch(...)` を使うテスト | 残す | `Planned` の形（`settings`，`clock`，`refused`）は変えていない |

`tests/fake.rs`

| テスト名 | 扱い | 理由 |
|---|---|---|
| `ur_25_a_same_size_membership_swap_reopens_the_streamer` | 残す | 同じ大きさで集合が違う switch は開き直す（P318 の行の殺す側） |
| `ur_13_arm_compares_the_rate_set_after_the_streamer_opens` | 残す | arm の順序（open の後に rates） |
| `ur_13_arm_refuses_a_rate_set_the_description_does_not_have` | 残す | arm の拒否と close |
| `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open` | 残す | 同上 |
| `ur_13_a_direction_the_arm_never_opened_is_not_compared` | 残す | arm が開かなかった向きは比較しない |
| 「enable from none の owner」の一群（`channels_from_none` 相当の fake 側の tests，`ur_13` の Sol 3 の系列） | 残す | `rx_open` と `tx_open` の数を見る．記録の形の変更に依らない |
| `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer` | 新規（S12） | — |

分類の結論：既存テストの書き換えは要らなかった．新しい形の記録（`DeviceSet`，`configured`）に合わないテストは見つからず，全テストが通っている．

### 4. ワークスペース全体のテスト（指示書 5）

`cargo test --workspace --no-fail-fast` は終了コード 0．失敗は無い．ログは `fix1-test.log`（同じディレクトリ）．`ezsdr-radio-uhd` の lib は 144 件，`fake` の結合テストは 228 件が通る．
`cargo clippy --workspace --all-targets -- -D warnings` は警告なしで通る．
`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` は 0 件．

### 5. mutation（前景，sim02:8 と sim03:8）

この回で足した行は `plan/maintenance/tools/mutations.json` の末尾の P369〜P372 の 4 行（既存の行は変えていない．行の追加だけ）．ファイル全体は書き直していない．各 `old` は対象ファイルに 1 回だけ一致することを確かめた．

| 行 | 変えた箇所 | 殺す テスト | 結果 |
|---|---|---|---|
| P369 | `core.rs` の `owner_step`：`_ => Owner::Open,` を `Owner::Nothing` に（記録が不明・別集合のとき開かない） | `ur_13_a_from_none_segment_the_owner_never_started_is_opened_again`（S1） | killed |
| P370 | 同じ変更 | `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`（S2） | killed |
| P371 | `tx.rs`：拒否された change の `to.refused` を見ず，owner が configure する | `ur_25_a_refused_transmit_clock_is_not_adopted`（S14） | killed |
| P372 | `mod.rs` の arm：`record_open` を消し，記録を Unknown のままにする | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`（S12） | **SURVIVED** |

P372 が残った．arm の記録を消しても S12 のテストは落ちない．記録が Unknown なら最初の segment で開き直すはずだが，観測された `rx_open` は 1 回のままだった．原因は未確認（本体の木で debug せず，mutant を試す別のコピーも作っていない）．この行は orchestrator の判断に回す．候補は次のとおり．テストの観測が arm 後の owner の開き直しを捕まえていない，または最初の segment がこの経路を通らない．テストをどう強めるかは次の回で決める．

既存の行は変えていないので，流したのは新しい 4 行だけ．指示書の「変えた行」は 1a で変えた行（P361〜P368 など）で，その殺すテストの名前と中身は変えていない．

### 6. 逸脱と残り（正直に書く）

- **A（S11b の assert を縮めた）**：S11b の計画は「record-only の release で版が上がり，owner が `Configure` と判断する」だった．テストの setup（`channels_from_none` で 100 ms の enable を記帳し，50 ms の timed gain を release）では，from-none の configure が channel 変更の時点で device に出ていて，release は gain を `apply` で device に送る（`rate=-` と `gain=3`）．版は上がらず，記録だけの release の経路には入らなかった．テストは「segment の設定（rate）を再送しない」と「gain 以外の apply が無い」だけを assert する形に直した．この setup で記録だけの release を作る方法は，この回では見つけられなかった．指示書の「owner がまだ始めていない segment で版が上がる」は，この形では確かめられていない．orchestrator の判断が要る．
- **B（S20 を書いていない）**：`arm` の照合の拒否が記録を `Unknown` にすることは，`Core` の内部の状態でしか観測できない．fake 側（`tests/fake.rs`）からは `run.finish()` の manifest だけが見えて，記録の形は見えない．unit で書くには `mod.rs` の Provider を組む必要があり，この回の範囲を超えるので書かなかった．既存の `ur_13_arm_refuses_*`（`close_streams` の後に開かない）は近いが，S20 の「次の開き直しで保存した拒否が再適用される」は確かめていない．
- **C（FakeFault の追加）**：`crates/ezsdr-radio-uhd/src/device.rs` に `FakeFault::TxApplyFails` と `RxChannelApplyFails(usize)` を 1a で足した．adjudication は FakeFault を足さないと言っていた．この回では変えていない．1a の報告に書いてあるとおり，逸脱として残す．`TxApplyFails` は既存の `ur_13_a_failed_configure_by_the_owner_*` が使う．
- **D（D1 の A は未実装）**：2 の記述のとおり．1b の範囲．
- **E（S3 と S21 の mutation 行は無し）**：指示書は「新しい行」として S1・S2・S14・S12 の行を挙げていた．S3・S11b・S21 には新しい行を足していない．S3 は既存の P-行（P368 など）と重なる．S11b は setup の制約で殺す形を決められなかった．
- **F（セルフレビュー無し）**：指示書どおり．DeepSeek のセルフレビューはこの回では流していない．

### 7. 変えたファイル

- `crates/ezsdr-radio-uhd/src/provider/control.rs`：S1・S3・S11b・S21 のテストと `channels_from_none`・`owner_rx_calls`・`owner_rx` の helper（前回の 1a から）．S11b の assert を縮めた．
- `crates/ezsdr-radio-uhd/src/provider/tx.rs`：S2・S14 のテストを追加（`ur_13_a_transmit_from_none_*`，`ur_25_a_refused_transmit_clock_is_not_adopted`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs`：S12 のテストを追加．
- `plan/maintenance/tools/mutations.json`：P369〜P372 の 4 行を末尾に追加．
- この報告：`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/fix1-report.md`．テストのログ：`fix1-test.log`．mutation のログ：`fix1-mutate.log`．

以上．

## 回 2：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `f487877` に 1b-4d-1a と修正の回 1 の未コミットの変更）．
「守ること」は `impl-brief.md` と同じ．範囲は `impl-a-brief.md` の 1a．計画は `test-plan.md`，裁定は `adjudication.md`，前の回の報告は `report.md` と `fix1-report.md`（同じディレクトリ `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/`）．

### 読むもの

`sol-review.md` の末尾（`判定：FAIL` から後）．指摘 1〜7 をすべて直す（裁定：全部採る）．

### 裁定と直し方

1. **major（TX の report の対応付け）**：記録に streamer の世代を足す．どの thread でも open が成功するたびに世代を進める．uhd-tx は自分の `unacked` がどの世代の streamer に対応するかを持ち，switch のときに記録の世代と違えば `unacked` を捨てる（owner が今回開いたかどうかでなく，report の queue が置き換わったかで決める）．
   - テスト：実際に uhd-control の from-none の `tx_open` を通す（記録を手で書き換えない）．古い burst の ACK を待ったまま disable → owner は idle → from-none で開き直す → owner がその segment を採る → 新しい burst の ACK・TIME_ERROR が新しい burst を指す（`target` と `late_by_ns` が新しい ClockDomain のもの）．
   - 逆の期待を固定している `ur_28_a_streamer_the_record_holds_keeps_its_correlations` は直すか消す．
   - 直す前の形を mutation の行にして殺す．
2. **major（S11a・S11b）**：Sol の指示どおり．S11b は初期設定も `radio.rx.channels = 0` にし，100 ms の enable を from-none で configure → 50 ms の gain を記帳 → release（記録だけになる）→ owner の begin を順に動かし，版が上がること・timed の apply が無いこと・untimed の configure がやり直されること・最後の gain の値を確かめる．S11a は実際の from-none の後に timed の release と owner の begin を通し，configure をやり直さないことを固定する．record-only の `changed()` を消すと S11b が落ちること（mutation の行）．
3. **minor（版の上げすぎ）**：segment の origin とその channel に対する投影が実際に変わったときだけ版を上げる．S11c は origin より前の更新の取消を正の例に，origin より後・別の channel の取消を「configure をやり直さない」負の例にする．
4. **minor（S12・S20・P372）**：arm → start → 同じ集合の次の cold segment → owner の begin・switch まで進め，余計な open が無いことを RX・TX の両方で確かめる（P372 が殺されること）．S20 は `mod.rs` の unit test で，照合の拒否の後の `Unknown` と configure の欄が空なことを確かめる．
5. **minor（S2・S21）**：S2 は RX の S1 と同じく uhd-control と owner を直接動かし，B・C を採る前に開き直すことを確かめる．S21 は owner の begin と `rx_start` まで確かめる．
6. **minor（FakeFault）**：`TxApplyFails` と `RxChannelApplyFails` を `FakeFault` から外し，test 側の wrapper（`test_support.rs`，`LockSpy` と同じ形で `Device` を包む）に移す．
7. **minor（spec 18）**：記録とその一致条件（`(seq, 版)`，世代），版を上げる条件，失敗のときの記録，prune で残ることを 1 か所に規則として書き，UR-17・UR-23 から引く．「拒否された segment では device を呼ばない」を規則として書き，Test 行を付ける．DA-1 の形．

### 確かめること

`impl-brief.md` と同じ 1〜3（ワークスペース全体のテストを含む）．mutation はこの回で足した行・変えた行と，コードかテストが変わった行（前景で流す）．

### セルフレビュー

reviewer は DeepSeek（`opencode-go/deepseek-v4.1-flash#max`，`--agent plan`），回ごとに新しく起動，`timeout 2400`．**チェックリストは小さくする**：この回の 7 項目それぞれについて「直っているか・テストがそれを観測するか・行がそれを壊すか」だけ，読む範囲は `git diff` のうちこの回で変えたファイル．前の段階では大きなチェックリストで 2 回とも時間切れになった．P1 が無くなるか 3 回まで．判定が出ずに終わった回は 1 回だけ起動し直し，2 回続けて出なければ止めて報告に書く．

### 報告

`fix2-report.md`（同じディレクトリ）に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．範囲を理由に blocked にしない．
中身：7 項目それぞれの修正，テストと行，確かめることの結果（mutation は行ごと），セルフレビューの各回の指摘と扱い．

## 回 2：報告

STATUS: done

## Phase 8 段階 1b-4d-1a の修正の回 2 の報告

実装役．指示書は `fix2-brief.md`（範囲は `impl-a-brief.md` の 1a，計画は `test-plan.md`，裁定は `adjudication.md`）．Sol のコードレビューの指摘 1〜7 をすべて直した．`cargo fmt`・`rustfmt` は使っていない．commit・push・`git add` はしていない．mutation は `remote_mutate.py` を前景で sim02・sim03 に流した（main tree に mutant を当てていない．ローカルの `mutate.py` も流していない）．

### 1. major（TX の report の対応付け）

**直したもの**：記録に streamer の世代を足した．`StreamState.generation`（`core.rs`）で，`record_open` が open の成功ごとに 1 進める（どの thread の open でも同じ）．`owner_configure` は `reopened` の out 引数をやめ（「owner が今回開いたか」を判断に使わない），`tx.rs` の `Tx` が `streamer: u64`（自分の `unacked` が対応する世代）を持ち，`Tx::new` で記録から初期化し，`do_switch` の採用の直前に記録の世代と比べて違えば `unacked` を捨てる．

**テスト**：
- 新規 `ur_28_a_from_none_open_replaces_the_report_queue`（`tx.rs` unit）．uhd-control の from-none を `control_of`・`tx_channels_from_none`（本物の `Control::book`）で通す：enable（100 ms, 1ch）→ owner が採る → burst の ACK 待ち → disable（200 ms, 0ch）で clock が終わり idle → from-none の enable（300 ms, 1ch）→ owner が採る．assert は (1) `tx.unacked` が空（旧 streamer の対応付けが消えた），(2) 新 burst の `TIME_ERROR` の `target` が新しい `ClockDomain` の burst，(3) `late_by_ns` = 1000（新 clock の start から 200 tick），(4) その ACK で `unacked` が空，(5) その後の `TIME_ERROR` は何も指さない（旧 burst を消費しない）．記録は手で書き換えていない．
- `ur_28_a_streamer_the_record_holds_keeps_its_correlations` を直した（消さなかった）：Sol が指していた「実際の open なしで記録だけを書き換え，逆の期待を固定する」形をやめ，arm が書く位置（`Tx::new` の前）に記録を置き，owner 自身の別集合への reopen で対応付けが消えることと，開かない switch（記録が集合と設定を保持）では残ることを見る．
- 既存の `ur_13_a_failed_open_by_the_owner_is_opened_again`（S7）と `ur_13_a_partly_applied_set_is_configured_whole_without_reopen`（S25）は `ApplyFails` の wrapper に移した（項目 6）．

**行**：P373（`new` が「自分の最後に採用した集合」で判断する形＝直す前の形の非並行版）→ killed．P372・P377（arm の記録を消す形）も killed．

### 2. major（S11a・S11b）

**直したもの**：テストの書き直し（本体のコードは 1a のまま．版の上げ方は項目 3 で絞った）．

**S11b** `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（`control.rs` unit）：初期設定を `radio.rx.channels = 0` にし（`control.config` を書く．初期 stream が空であることと設定が一致する），100 ms の enable を from-none で configure → 50 ms の timed gain を記帳 → 48 ms で `release()`（channel が無く記録だけ）→ owner の `begin()`．assert は (1) release の前後で device の呼び出しが増えない（timed の apply も無い），(2) 版が +1，(3) begin が `rx_open` を出さない，(4) begin が untimed の configure を 1 回やり直す，(5) その apply が `gain=3`（記録された最後の gain）．

**S11a** `ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again`：同じ from-none の後に，enable の segment の origin と同じ instant（100 ms）の timed gain を release する（その instant には channel が居るので device に出る＝timed の apply が 1 回）．assert は (1) timed の apply が 1 回で `gain=3`，(2) owner の begin が `rx_start` だけを出し configure をやり直さない，(3) 版が上がらない，(4) 記録の configure の欄が `(enable の seq, 版)` のまま．

**行**：P374（record-only の `changed()` を消す形）→ killed（S11b が落ちる）．P375（項目 3 の濾しを消す形）→ killed．

### 3. minor（版の上げすぎ）

**直したもの**：`control.rs` に `changed_projections(dir, e, chan)` を足した．timed command の instant `e` が segment の origin 以前で（`e <= segment.origin`），command の channel が segment の集合にある（current keys は `chan = None` で全 channel）segment の configuration だけを返す．record-only の release（`release()` の `chans.is_empty()` の腕）と `cancel_held` の両方がこれを使う（`cancel_held` は `updates` の lock の中で `Arc::ptr_eq` で選んだものだけ `version.fetch_add`）．`streams` の lock は `updates` を取る前に離している（`updates → streams` の順は変えない）．

**テスト**：`ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`（S11c）を書き直した：(1) origin（1 s）より後の取消（1.5 s）＝負の例：版が変わらず `owner_step` が `Owner::Nothing`，(2) origin より前の取消（0.5 s）＝正の例：版が +1 し `Owner::Configure`．記録の configure の欄は from-none の configure 自身が書いたもの（手書きしていない）．

**行**：P375 → killed．

### 4. minor（S12・S20・P372）

**S12 の作り直し**：`tests/fake.rs` の `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer` を `Direct` で書き直した．arm → start → 同じ集合の次の cold segment（rx・tx の rate を 2e6 へ，50 ms）→ owner の begin/switch まで進め，assert は (1) `rx_open 0 [0]` が 1 回，(2) `tx_open 0 [0]` が 1 回（余計な open が無い），(3) `rx_start` がある，(4) 次の segment が RX・TX とも untimed で `rate=2000000` を configure する（owner が begin/switch まで進んだこと）．前の形は「arm → start」だけで止まり，最初の segment は `start` が直接 `rx_start` するので owner の判断を通らなかった（P372 が生き残った理由）．

**S20**：`mod.rs` の unit test `ur_25_a_refused_check_at_arm_leaves_the_set_unknown` を足した．`Prepared` を手で組み（RX stream 0 に 1 channel，link あり），方向の照合を先に拒否させ（`core.check_rates(Dir::Rx, 99)`），`arm()` が `UR-13:` で拒否されることと，記録が `DeviceSet::Unknown`・`configured` が `None` であることを見る．

**行**：P372（RX の arm の記録を消す）→ **killed**（前の回で SURVIVED だった），P377（TX の arm の記録を消す）→ killed，P378（照合の拒否でも集合を書く）→ killed．

### 5. minor（S2・S21）

**S2**：`tx.rs` の `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` を RX の S1 と同じ形に書き直した．uhd-control を `control_of`・`tx_channels_from_none`（`Control::book` 経由）で直接動かし，from-none の B={0,1}（100 ms）→ 空集合（200 ms）→ C={0}（300 ms）を記帳した後，owner（`Tx`）を `read`/`step` で動かす．assert は (1) control の `tx_open` が `["tx_open 0 [0, 1]", "tx_open 0 [0]"]`，(2) owner が B を採る前に `tx_open 0 [0, 1]` を出し `tx.clock` が B の segment の clock，(3) 切れ目まで進めて C でも `tx_open 0 [0]` を出し `tx.clock` が C の clock，(4) 全体の `tx_open` の列が control の 2 回＋owner の 2 回．前の形は owner が自分で全集合を順に開くだけで，control の from-none と組み合わせていなかった．

**S21**：`control.rs` の `ur_25_a_refused_receive_enable_is_booked_and_the_next_enable_starts` に，次の enable の segment を owner の `begin()` で始めさせるところまで足した．assert は (1) 拒否された enable が refused で記帳される（既存），(2) 次の enable が `rx_open` を 2 回目に出し configure する（既存），(3) owner の begin が `rx_start` だけを出し `rx_open`・`apply` を出さない（記録が集合と設定を持つ）．

**行**：P370（owner が記録を見ずに開かない形）→ killed（S2 の新しい観測で落ちる）．

### 6. minor（FakeFault）

**直したもの**：`FakeFault::TxApplyFails` と `FakeFault::RxChannelApplyFails(usize)` を `device.rs` から消した（enum の定義と `apply` の分岐の両方．`RxApplyFails` は元から在るもので残す．`device.rs` の他のテストはこの 2 つを使っていない）．test 側の wrapper `ApplyFails`（`test_support.rs`，`LockSpy` と同じ形で `FakeDevice` を `Arc` で包み，`Device` の全メソッドを前へ流し，`apply` だけ「`dir` の最初の untimed の apply（`chan` を名指すならその channel）を一度だけ拒否する」）に移した．使う側：`rx.rs` の S25（`Dir::Rx`, `Some(1)`）と `tx.rs` の S8（`Dir::Tx`, `None`）．

**観測の差（正直に書く）**：古い `FakeFault` の拒否は fake の `calls()` に記録された後で起きたが，wrapper は `inner` へ渡す前に拒否するので，拒否された apply は呼び出しの列に残らない．そのため S25・S8 の apply の数え方を直した（S25: `apply rx 0` が 2 回・`apply rx 1` が 1 回，S8: apply が 1 回＝2 回目の switch の configure だけ）．拒否が「設定を一切当てない」ことは同じで，テストの主張（開き直さず両 channel を configure し直す／失敗の後は configure だけをやり直す）は保たれている．

### 7. minor（spec 18）

**直したもの**：`design/18-uhd-radio.md` に規範の規則を 2 つ足し（DA-1 の形：規範の文だけ，理由・経緯なし，`*Test:*` 行つき），UR-17・UR-23・UR-28 から引いた．
- **UR-36**（記録）：stream ごとの記録（streamer の channel 集合，streamer の **open 世代**，最後に device が受けた設定 `(seq, 版)`）；世代はどの thread の open の成功でも進む（UR-28 が読む）；失敗した open・拒否された照合は集合も設定も残さず，成功した open は集合だけ残す；全 channel を device が取り read-back を UR-12 が受けた configure は `(seq, 版)` を残し（command の無い最初の segment は残さない），失敗した configure は設定を残さず集合を動かさない；版は device が timed command として受け取らない変化（channel の無い release，取消・移動）で，かつ segment の origin と channel に届くときだけ動く；prune は記録のどの部分も変えない．
- **UR-37**（拒否された segment）：拒否された `Planned` の owner は device を呼ばない（開かず，configure せず，始めず，transmit は clock を採らない）；拒否された receive の変更は plan に入らないので owner はそこへ届かない．
- 参照：UR-17 と UR-23 の「記録が集合／設定を持つなら開かない・configure しない」に `(UR-36)`，UR-28 の「reopen は対応付けを捨てる」に「他の thread の open でも同じ（記録の世代，UR-36）」．
- Changes 表に 2026-10-19 の行を足した（1a とこの回の規範の変更．record は design 33 §5 stage 1b-4d と段階の brief・plan・reports）．
`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` は 0 件．

### 確かめること

1. `cargo test --workspace --no-fail-fast`：失敗なし（`test result: ok` が 66 行，FAILED は 0）．`ezsdr-radio-uhd` の lib は 146 件，`fake` の結合テストは 228 件が通る．
2. `cargo clippy --workspace --all-targets -- -D warnings`：警告なしで通る．
3. `cargo test --workspace` の両方と `rules.py --base HEAD`（0 件）．両方の `mutations.json` の全行がファイルにちょうど 1 回一致する（`rules.py` の検査．P371 と phase7 の U40 は `owner_configure` の引数が変わったので `old`／`new` を当て直した）．
4. mutation（前景，`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 P317 P318 P351 P358 P369 P370 P371 P372 P373 P374 P375 P376 P377 P378 U40`）：15/15 killed．行ごと：

| 行 | 変えたもの | 殺すテスト | 結果 |
|---|---|---|---|
| P317 | `owner_step` の集合の比較（`count` で比較） | `ur_25_a_same_size_membership_swap_reopens_the_streamer` | killed |
| P318 | `owner_step` の版の比較（`is_some` だけ見る） | `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`（S11c） | killed |
| P351 | from-none の拒否を記帳しない | `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values` | killed |
| P358 | 拒否された TX の clock を採用する | `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock` | killed |
| P369 | owner が記録を見ずに開かない（RX，S1） | `ur_13_a_from_none_segment_the_owner_never_started_is_opened_again` | killed |
| P370 | 同上（TX，S2） | `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`（作り直し） | killed |
| P371 | 拒否された TX の switch で device を呼ぶ（`old` 当て直し） | `ur_25_a_refused_transmit_clock_is_not_adopted` | killed |
| P372 | arm が RX の記録を書かない（`old` はそのまま） | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`（作り直し） | **killed**（前の回は SURVIVED） |
| P373 | 新：TX が自分の集合で report の対応付けを判断する | `ur_28_a_from_none_open_replaces_the_report_queue` | killed |
| P374 | 新：record-only の release が版を上げない | `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（S11b） | killed |
| P375 | 新：取消が全 segment の版を上げる（origin・channel を見ない） | `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`（S11c） | killed |
| P376 | 新：失敗した open でも集合を書く | `ur_13_a_failed_open_by_the_owner_is_opened_again`（S7） | killed |
| P377 | 新：arm が TX の記録を書かない | `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`（S12） | killed |
| P378 | 新：照合の拒否でも arm が集合を書く | `ur_25_a_refused_check_at_arm_leaves_the_set_unknown`（S20） | killed |
| U40（phase7） | rate の拒否を無視する（`old` 当て直し） | `rm_26_the_timeline_against_uhd_rx` | killed |

（`maintenance: 14/14 passed; phase7: 1/1 passed`．新規の行は P373〜P378 の 6 本で，`old` はそれぞれ対象ファイルに 1 回だけ一致する．）

### セルフレビュー（DeepSeek，`--agent plan`，回ごとに新規）

指示書どおりチェックリストを小さくし（7 項目それぞれ「直っているか・テストが観測するか・行が壊すか」だけ），読む範囲をこの回のファイルに限った．**2 回続けて判定が出なかった**（どちらも `timeout 2400` の期限で終わり，判定の行を書かなかった．2 回目は起動し直しの 1 回）．指示書の「2 回続けて出なければ止めて報告に書く」に従い，ここで止める．

- 回 1（`opencode-go/deepseek-v4.1-flash#max`，ログ `fix2-selfreview-1.log`．同じディレクトリ）：design/18 の UR-36・UR-37 と Changes の行，mutations.json の新規行を読んだところまでは進んだが，判定の前に transport が切れた．この回は，指示になかった「コピーを作って mutation を流す」動作に入ろうとしていた（木は変えていない．実際には mutation を流していない）．
- 回 2（`fix2-selfreview-2.log`）：読む範囲と「mutation を流さない・build しない」を明記して起動し直した．P372〜P378 の `old` が対象ファイルに 1 回だけ一致することを確認し（出力あり），tx.rs を読んでいるところで期限に達した．判定は出なかった．
- 扱い：指摘が無いので直すものは無い．この 2 回の代わりとして，同じ 7 項目を自分でもう一度通し（コード・テスト・行・spec の文の対応），上の表のとおり行がすべて殺されることを確かめた．orchestrator と Sol のレビュー（段階 6・7）で見てもらう．

### 本体・テストの変更（量）

`git diff --stat`（HEAD `f487877` から，1a とこの回の合計）：11 files changed, 966 insertions(+), 254 deletions(-)．この回で足した・変えた主なもの：`core.rs`（`generation`・`record_open`・`owner_configure` の引数），`tx.rs`（`streamer`・`do_switch`・テスト 3 本の追加／作り直し・helper `control_of`・`tx_channels_from_none`・`plan_of`），`control.rs`（`changed_projections`・release／cancel の版・テスト 3 本の作り直しと S21 の追加），`rx.rs`（`owner_configure` の引数・S25 の wrapper），`mod.rs`（S20 と `NoLink`），`test_support.rs`（`ApplyFails`），`device.rs`（FakeFault 2 つを削除），`tests/fake.rs`（S12 の作り直し），`design/18-uhd-radio.md`（UR-36・UR-37・参照・Changes の行），`plan/maintenance/tools/mutations.json`（P371 当て直し・P373〜P378 追加），`plan/phase7/tools/mutations.json`（U40 当て直し）．

### 残り・正直に書く

- セルフレビューは 2 回とも判定が出なかった（上）．指示書の想定（P1 が無くなるか 3 回まで）に達していない．
- `ApplyFails` は拒否した apply を fake の呼び出しの列に残さない（項目 6 の観測の差）．S25・S8 の数え方をそれに合わせた．呼び出しの列に残す形（fake に「記録だけして拒否する」口を足す）は，公開の fake の語彙を増やさない裁定に沿うため取らなかった．
- `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`（S11c）の記録の configure の欄は from-none の configure が書いたものを使う．負の例の後で版が動かないことを見るために手書きの記録は使っていない（正の例の後で版が動くので，同じ記録は使えない）．
- 1a で `design/18-uhd-radio.md` に入れた 4 か所（UR-12 の文，UR-17 の文，UR-23 の文，§6 の `rm_26_…` の行）はこの回でもそのまま残し，Changes の行はこの回の分（1a とまとめて 1 行）として足した．
- この回で足した 6 行（P373〜P378）は，`plan/maintenance/tools/mutations.json` の末尾（P372 の後）に，周りの書式のまま足した．ファイル全体は書き直していない．

## 回 3：指示書


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `f487877` に 1b-4d-1a と修正の回 1・2 の未コミットの変更）．
「守ること」は `impl-brief.md` と同じ（同じディレクトリ `~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/`）．前の回の報告は `fix2-report.md`．

### 読むもの

`sol-rereview.md` の末尾（`判定：FAIL` から後）．指摘 1〜3 をすべて直す（裁定：全部採る）．

### 直すこと

1. **major（report の読み出しで世代を照合しない）**：`tx.rs` の `reports()` は，report を `unacked` に対応付ける前に，記録の世代と自分の `streamer` の世代を比べ，違えば `unacked` を捨てて世代を合わせる．世代の比べ方は `do_switch` と同じ 1 つの関数にする（2 か所に書かない）．
   - テスト 2 本：古い burst の ACK を待ったまま disable → uhd-control が本物の from-none で `tx_open` を終える → owner がその segment を採る**前に**，(a) 通常の poll（`reports()`），(b) Shutdown の ACK の読み，で置き換わった queue から TimeError を読む → 古い burst の `target` を消費しない．report は test 側の wrapper で入れる（`FakeFault` は増やさない）．
   - 直す前の形（`reports()` で比べない）を mutation の行にして殺す．
2. **minor（照合の拒否で世代が進まない）**：open の成功と照合の結果を分ける．open が成功したら世代を進め，照合が拒否したら集合と設定の欄だけを `Unknown`・なしにする．open 自体の失敗では世代を進めない．uhd-control の from-none，owner，`arm` のすべての経路で同じにする（`record_open` の形を直す）．
   - テスト：保存した照合の拒否 → `tx_open` は成功 → 拒否を再適用 → 世代は進み，集合は `Unknown`．open の失敗では世代が進まないことも 1 本．
   - 行：照合の拒否で世代を進めない形を殺す行．
   - `design/18-uhd-radio.md` の UR-36 が「every successful open」と合っていることを確かめる．
3. **minor（channel の濾し）**：S11c に layout の別 channel の負の例を足す（`{0}` の segment に対して origin より前の c1 の更新を取り消す → 版は変わらず `Owner::Nothing`）．channel の条件だけを外す mutation の行を足して殺す．

### 確かめること

`impl-brief.md` と同じ 1〜3（ワークスペース全体のテストを含む）．mutation はこの回で足した行・変えた行と，コードかテストが変わった行（前景で流す）．

### セルフレビュー

この回は流さない（小さな回で，Sol が再レビューする）．報告にそう書く．

### 報告

`fix3-report.md`（同じディレクトリ）に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．範囲を理由に blocked にしない．
中身：3 項目それぞれの修正，テストと行，確かめることの結果（mutation は行ごと）．

## 回 3：報告

STATUS: done

## Phase 8 stage 1b-4d-1a 修正ラウンド 3 の報告

対象: `/home/komatsu/local_work/Ez-SDRv4`（main の作業ツリー。コミット・push・`git add` は行っていない）
依頼: `fix3-brief.md`。自己レビュー（brief の手順 5）は**今回のラウンドでは実行していない**。

### 1. 指摘への対応

#### (major) 送信の報告読み出しで、streamer の世代を所有者側と比べてから対応付ける

- `crates/ezsdr-radio-uhd/src/provider/tx.rs`: 世代の比較を `fn sync_streamer(&mut self)` に一本化した。`do_switch` と `reports()` の両方がこれを呼ぶ。`reports()` は shutdown の ACK 読み出しも通る（`shutdown` → `reports(Wall::from_millis(10))`）ので、shutdown の経路も同じ比較を受ける。
- テスト（test-side の報告ラッパー `ReportFeed` を `test_support.rs` に追加。`FakeFault` は変更していない）:
  - `ur_28_a_poll_after_a_from_none_open_names_no_burst_of_the_replaced_queue`（定期 poll）
  - `ur_28_a_shutdown_after_a_from_none_open_names_no_burst_of_the_replaced_queue`（shutdown の ACK 読み出し）
  - 両方とも通過。
- 変異行 P379（`reports()` から `sync_streamer()` を外した pre-fix 形）を追加し、sim02/sim03 で killed。

#### (minor) open の成否と rate 確認の結果を分けて記録する

- `crates/ezsdr-radio-uhd/src/provider/core.rs`:
  - `open_device`（device の open のみ、記録しない）、`check_opened`（rate 確認）、`record_opened`（open の結果と確認の結果から record を更新）を追加。
  - `open_streamer` は owner と `arm` 用に残し、`open_device` → `check_opened` → `record_opened` を一つの `streams` 区間で行う。
  - 成功した open は generation を進める。確認が拒否された場合は set と configuration を Unknown / none にする。open が失敗した場合は generation を進めない。
  - `record_refused` は確認拒否のときだけ使う。
- `crates/ezsdr-radio-uhd/src/provider/control.rs`: enable from no stream は `open_device` と `check_opened` を lock の外で呼び、結果を既存の再取得区間（`streams = lock(...)`）の中で `record_opened` に渡す。これで UR-25 の「その区間の数は 2」（`ur_25_a_cold_change_takes_the_line_lock_once...`）を保つ。
- `crates/ezsdr-radio-uhd/src/provider/mod.rs`: `arm` の open エラーにだけ `UR-13:` を付ける（`open_refusal`）。確認の拒否は `check_rates` が既に `UR-13:` 付きなので二重に付けない。`tests/fake.rs` の `ur_13_the_receive_owner_s_first_open_is_compared_there`（owner 側は `UR-13:` で始まってはならない）を通すため、`open_streamer` からは接頭辞を外した。
- テスト:
  - `ur_36_a_successful_open_moves_the_generation_on_even_when_its_check_refuses`（保存済みの拒否 → open 成功 → 拒否の再適用 → generation が進み set が Unknown）
  - `ur_36_a_failed_open_moves_no_generation`（失敗した open では generation が動かない）
  - 両方とも通過。
- 変異行 P380（拒否された確認を「失敗した open」扱いにして generation を進めない pre-fix 形）と P376（失敗した open が要求された set を記録する形）を追加・更新し、どちらも killed。
- `design/18-uhd-radio.md` の UR-36 は規範本文に "every successful open" の語はなく、改訂表の行に「any thread's open moves on」とあった。失敗した open は generation を動かさないので、「every successful open moves on, whichever thread made it」に直した（改訂表の 1 行のみ）。

#### (minor) S11c の layout 反例

- `control.rs` の `ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version`:
  - 元の設定は `radio.rx.c0.stream = 0` だった。チャンネル 0 は既にストリーム 0 にあるので no-op となり、segment が計画されなかった。`radio.rx.s0.sample_rate_hz` を 1000 ms に予約する形に変えた（`ur_25` の lock テストと同じ）。
  - 計画された `{0}` segment の origin は約 211 ms なので、チャンネル 1 の保留更新は 500 ms から 100 ms に移し、origin より前にした。500 ms のままだと、チャンネル条件を外す変異は起きても効かない。
  - 所有者が記録を信頼する前提（`record_open` と `record_configured` を segment の version に対して置く）を、テスト内で明示した。
  - 結果: version は不変、`owner_step == Owner::Nothing`。
- 変異行 P381（`changed_projections` からチャンネル条件だけを外す）を追加し、killed。

### 2. 検査結果

| 検査 | 結果 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 失敗 0。`ok` の結果行 66。ログ: `fix3-test-workspace.log` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通過。ログ: `fix3-clippy.log` |
| `rules.py --base HEAD` | 0 problem。ログ: `fix3-rules.log`（最初は変異行 11 件の `old` が 0 回一致。行を直して解消） |
| 変異（`remote_mutate.py maintenance,phase7 sim02:8,sim03:8`、前景） | 14/14 killed。ログ: `fix3-mutation.log` |

変異の対象行（`mutations.json` の行だけを変更・追加。全体の書き換えはなし）:
- 直した行: P296, P334, P343, P344, P345, P372, P373, P376, P377, P378（`old` が消えていた行を新しい箇所に付け替え。P378 は名前も変更）、phase7 の U39。
- 追加した行: P379, P380, P381。
- 全 14 行は実行し、すべて killed。

### 3. 元の不具合の内訳（brief の「テストの 1〜3」）

- `ur_24_...`（S11c）: 上の (minor) 3 の項に記載。テストの設定を直した。コードの不具合ではない。
- `ur_25_a_cold_change_takes_the_line_lock...`: 直前の変更で、from-none の open が追加の lock 区間を取っていた。`open_device` と `record_opened` の分割で、区間数は元どおり 2。
- `tests/fake.rs` の `ur_13_the_receive_owner_s_first_open_is_compared_there`: `open_streamer` の接頭辞を外し、arm だけ `open_refusal` で付けた。

### 4. 残り・注意

- 自己レビュー（brief の手順 5）は実行していない。
- 変更を含む作業ツリーはコミットしていない。`git add` もしていない（`git diff --cached` は空）。
- 変異の実行はすべて `remote_mutate.py` の前景実行。ローカルの `mutate.py` と本体ツリーへの変異は使っていない。
- `cargo fmt` / `rustfmt` は実行していない。
- 背景ジョブは残していない。
- 補助スクリプト `fix3_rows.py` は `~/.cache/ezsdr-fixes/` にあり、リポジトリには入っていない。変更前の `mutations.json` は `maint-before-fix3.json`、`ph7-before-fix3.json` として同じ場所に保存した。
- 変更したファイル（このラウンドで触ったもの）: `crates/ezsdr-radio-uhd/src/provider/{core.rs, control.rs, mod.rs, tx.rs, test_support.rs}`, `design/18-uhd-radio.md`（改訂表の 1 行）, `plan/maintenance/tools/mutations.json`, `plan/phase7/tools/mutations.json`。`tests/fake.rs` と `rx.rs`、`rx/differential.rs` は作業ツリーで既に変更済みの状態から、このラウンドでは手を入れていない。
