STATUS: done

# 設計メモ 37（#69）37b 実装 — 報告

HEAD `adbd3a0` から，raw device の 13 本と `Direct` 系の 5 本を制御時計の unit test に移した．本番のコード（`crates/*/src/` の `#[cfg(test)]` の外）は変えていない．`cargo fmt`/`rustfmt`，ローカルの `mutate.py`，`git add`・commit・push・stash はしていない．作業 dir の orchestrator のファイルと見張りのプロセスは触っていない．

## やったこと（ファイルごと）

- `crates/ezsdr-radio-uhd/src/device.rs`（`#[cfg(test)]` の中だけ）: 表 B-2 の raw device 13 本を `tests/fake.rs` から移した．各テストの本体は既存の `manual(config)`・`participant(&device, f)` の上で動く（M1: 装置の時刻は参加者が待つ間だけ進む）．`reports_until` helper も一緒に移した．名前は変えていない．テストの本体は移設前と同じだが，(a) 空 channel list のテストは「channel 1 は空いている／channel 0 は塞がっている」を open の衝突で見る，(b) 空 eob と 1 sample 空けた burst は `transmitted` のサンプル値も見る，(c) 17 件目の queue は worker を `host.spawn` の参加者にし，「apply に入った」を合図してから 200 ms 完了しないことを見て，最後に join する，(d) 停止した stream 1 の tail は各ブロックの**最後のサンプルの instant** が停止時刻より前であることを見る（ブロックの先頭だけでは停止後のサンプルを許すため）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs`（`#[cfg(test)]` の中だけ）: `Direct` 系の 4 本を制御時計の unit test として置いた（`manual_rig`/`manual_rig_events` と `hold`，`device_at`）．`ur_24_hardware_timed_updates`（lead・late の applied instant・深さ 16・両方向），`ur_24_a_far_future_update_does_not_delay_a_nearer_one`（近い方が先に release され，遠い方は自分の instant），`ur_25_rx_channels_from_zero_starts_at_its_instant`（from-none enable の `rx_start` の origin と宣言された clock），`ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`（switch の configure に同じ instant の timed update が入り，release もされる，両到着順）．名前は変えていない．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs`（`#[cfg(test)]` の中だけ）: `ur_25_a_stream_silent_before_e1_switches_before_e2` を置いた．rx の `recv_timeout`・`turn`・`receive` は private なので，この 1 本だけは `control.rs` ではなくここに置いた（下記「残ったこと」）．系列は制御された Core 時計で，A の最後の block 5 ms・cut 10 ms・B の origin 60 ms を固定し，`rx.turn()` が fake に渡した timeout が bounded であること（`min_recv_timeout`），無音が cut+span を越えたときの stop 記録と clock の終わり，B の begin が origin より前であることを見る．
- `crates/ezsdr-radio-uhd/tests/fake.rs`: 13 本と 5 本，`reports_until` を削除し，Review N の節のコメントを raw の TX/report も `src/device.rs` の unit test だと直した．
- `crates/ezsdr-radio-uhd/tests/README.md`: raw の case の段落を，receive だけでなく transmit/report も含む形に直した（規則の文は変えていない）．
- `design/18-uhd-radio.md`: UR-33 の `*Test:*` で移した 13 本を「`device.rs` の manual clock の unit tests」の括弧へ移し，実時間の `ur_33_a_real_fake_s_time_counts_from_its_own_creation` は括弧の外へ戻した．§6 の `ur_24_hardware_timed_updates`・`ur_24_a_far_future…`・`ur_25_a_stream_silent…` の行に `(unit)` と今の assert を書き，`ur_33_a_burst_s_acknowledgement…` に `(unit)` を付けた．Changes に 2026-10-23 の 1 行を足した．規則本文は変えていない（動作の変更なし）．
- `plan/maintenance/tools/mutations.json`: P204/P205/P207/P213/P214/P215/P216/P217/P230/P231/P232/P264 の `test_bin` を `fake` → `lib`（12 行）．
- `plan/phase7/tools/mutations.json`: U20/U21/U37/B01/B02/B17/B30 の `test_bin` を `fake` → `lib`（7 行）．

## 表から変えた所とその理由

- **B-3 の置き場所**: 表と指示書は 5 本とも `src/provider/control.rs` と書くが，`ur_25_a_stream_silent_before_e1_switches_before_e2` は rx の private な `recv_timeout`/`turn`/`receive` を見るので `src/provider/rx.rs` に置いた．control.rs からは同じ観測ができない（`receive`/`samples` を `pub(super)` にすると `#[cfg(test)]` のためだけの本番の可視性変更になる）．4 本は指示どおり control.rs．
- **B-3 の前提の訂正**: 指示書と表 B-3 は「`Direct::new`/`with_links` は実時間の時計」と書くが，実際は `Direct::armed_with` が `fake(config)` を呼び，`fake()` が `ClockMode::Manual` を強制するので manual clock である（`tests/common/mod.rs:305-309`）．実験のスレッドが参加者でないため窓の判定は同じように危うく，結論（制御時計の unit test へ）は変わらない．
- **silent テストの B-3 の「no LATE_COMMAND」**: 旧テストの `LATE_COMMAND` は control 経由の Kernel イベントで，rx の unit test には control がない．spec 18 の行は，テストが実際に見る「timeout が bounded」「stop 記録」「clock の終わり」「origin より前に begin」に直した．後継の start が装置に受け入れられたか（late でないか）はこの性質の外なので assert しない（始点を固定するためだけに実時間の reset を置くと，今度は試験のスレッドの遅れで spurious に落ちる）．
- **raw 13 本の assert を足した所**: 空 channel list は open の衝突，空 eob/1 sample gap は `transmitted` の値，queue は worker の参加と join，停止 tail は最後のサンプルの instant．表 B-2 の「assert / 観測」を満たすためで，系列は変えていない．
- **移設しなかったもの**: `ur_33_a_real_fake_s_time_starts_at_its_own_creation`（表のとおり，bracket の設計で遅れに依らない）．

## 移したテストと消したテストの対応

| 消した（`tests/fake.rs`） | 代わり |
|---|---|
| `ur_24_hardware_timed_updates` | `src/provider/control.rs` の同名 unit（制御時計） |
| `ur_24_a_far_future_update_does_not_delay_a_nearer_one` | 同上 |
| `ur_25_rx_channels_from_zero_starts_at_its_instant` | 同上（clock の first-block 登録は `ur_25_a_second_change_follows_the_first` が pin） |
| `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` | 同上 |
| `ur_25_a_stream_silent_before_e1_switches_before_e2` | `src/provider/rx.rs` の同名 unit（制御時計） |
| `ur_33_the_fake_device_loops_back_what_it_transmits` | `src/device.rs` の同名 unit |
| `ur_33_scripted_faults_fire` | 同上 |
| `ur_33_two_receive_streamers_keep_their_own_channels` | 同上（最後のサンプルの instant も見る） |
| `ur_33_two_transmit_streamers_keep_their_own_reports` | 同上 |
| `ur_33_an_empty_channel_list_is_channel_zero` | 同上（channel 1 が空いていることも見る） |
| `ur_33_a_timed_command_follows_its_front_end_across_a_spec` | 同上 |
| `ur_33_the_antenna_is_read_back_after_an_untimed_change` | 同上 |
| `ur_33_the_fake_device_keeps_the_devices_queues` | 同上（worker は参加者，entry の合図と join） |
| `ur_33_an_empty_end_of_burst_is_one_zero_sample` | 同上（`transmitted` の零サンプルも見る） |
| `ur_33_a_timed_start_at_the_previous_burst_s_end_is_late` | 同上（gap 1 の先頭サンプルも見る） |
| `ur_33_a_burst_that_runs_dry_underflows` | 同上 |
| `ur_33_a_burst_resumed_after_an_underflow_plays_late` | 同上 |
| `ur_33_a_burst_s_acknowledgement_comes_once_its_end_has_played` | 同上 |

`fake` binary は 255 → 237，`--lib` は 277 → 295．移した先の名前はすべて元と同じ．

## mutation の行

当て直しは上の 19 行（`test_bin` だけ．`filter`・`old`/`new` は本番コードなので不変）．足した行はない（表 A-3 の T3 は 37a の範囲外で，この段階の表にも行なし）．流した結果（`remote_mutate.py`，sim02:4）:

- 19 行目までは 18/19 killed．**B01 が survived**：B01 は `turn()` の `rx_recv` の引数を変えるが，最初のテストは `recv_timeout()` の値だけを見て呼び出しサイトを通っていなかった．テストを `rx.turn()` を通す形に直し（fake に渡した `min_recv_timeout` を見る），B01・B02 を再実行して両方 killed．
- 自己レビューの修正の後，変更したテストの行を再実行：P230/P231/B17/B01/B02 は killed，P204/P214/P215/P216/B01/B02 は killed．最終状態で 19/19 killed（U20/U21/U37/P264/P205/P207/P213/P217/P232/B30 はテストを変えていないので最初の 19 行の結果のまま）．
- 全リストの `old`/`old2` のうち，当て直した 19 行はどれも tree で 1 回だけ一致する（P232 は `old` と `old2`）．移したテスト名を `old`/`new` に持つ行は全リストに見つからない．全リストのうち，もともと 0 回・複数回一致する行（別 crate の `old` など）はこの段階で触っていない．

## 確かめたことの結果

- 移した 18 本を各 20 回（1 本ずつ，`--test-threads=1`，負荷なし）：**18 本すべて 20/20**．修正の入ったテスト（empty channel list，empty eob，1 sample gap，queue，rx from zero，two receive，silent）は修正後に 20 回をやり直して 20/20．
- `cargo clippy --workspace --all-targets -- -D warnings`: 通過．`rules.py --base HEAD`: 0 problems．`git diff --check`: clean．
- `cargo test -p ezsdr-radio-uhd --lib`: 295/295（移設後の全レビュー修正を含む最終状態）．`--test fake`: 削除直後に 237/237．最終確認のときに他の利用者の `analysis_sefdm` が 6 本ほど CPU を占有し（load average 19.8），`rehearsal_b5_overflow`・`ur_17_blocks_carry_the_device_timestamps`・`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` の 3 本が落ちたが，3 本とも単独では通った．この 3 本は移設と無関係で，fake binary は削除以降変わっていない（その後の変更は `#[cfg(test)]` の lib 側だけ）．負荷の下の確かめは orchestrator の gate に残す．

## セルフレビューの指摘と扱い

Luna（`openai/gpt-6-luna#max`，読むだけ）を 3 回．P1 以上が 0 にならなかったので 3 回で止めた（指示書 §5）．

- **1 回目**（P1 3・P2 1）:
  1. 空 channel list が channel 0 を取ったことを見ていない → 直した（channel 1 は空く／channel 0 は塞がる open の衝突を追加）．
  2. 空 eob・1 sample gap がサンプル値を見ていない → 直した（`transmitted` の値と零サンプルを追加）．
  3. 17 件目の worker が動いたことを見ていない → 直した（worker を participant にし，entry の合図・200 ms・join；queue の instant を 20 ms にして join がすぐ終わるようにした）．
  4. rx from-zero が実時間の fake と，clock 登録を見ていない → 実時間の部分は直した（`manual_rig`+`hold` で装置を 0 に止め，origin が装置の時計より先であることと，宣言された clock を見る）．first-block の登録は `ur_25_a_second_change_follows_the_first` を名指しした（`rx.receive` は control.rs から呼べない）．
- **2 回目**（P1 1・P2 1・P3 2）:
  1. silent テストが実時間の fake で，start が late になり得る → 部分採用．start の受け入れはこの性質ではないので，reset と late の assert を外した（実時間の reset を置くと試験のスレッドの遅れで spurious に落ちる）．timeout の bounded 性・stop 記録・clock の終わり・origin より前の begin は制御された Core 時計で見ている．
  2. 17 件目の worker が apply の前に止まれば通る → **退けた**．worker は参加者で，entry の合図は apply の直前，200 ms の間は外側の参加者が時計を止めているので，queue が空いて通る道はない．apply の内側の probe は `#[cfg(test)]` のためだけの本番の道になり，指示書 §7 が禁じている．
  3. B-3 の配置が brief と違う（rx.rs） → 上に記録（rx の private な観測のため）．
  4. spec 18 の manual clock の括弧に実時間テスト → 直した（外へ戻した）．
- **3 回目**（P1 1・P2 1・P3 1）:
  1. silent テストが実時計で 6 ms の実時間待ちをする → 部分採用．6 ms の無音は実時間だが，値は `last_samples` をテストで設定し，待ちが長いほど quiet の条件は強くなるだけである（既存の rx の silent テストと同じ形）．判定は `recv_timeout`/`min_recv_timeout` の値・Core 時計で置いた分岐・stop 記録・clock の終わり・timing row で，試験のスレッドの遅れに依らない．装置の時計を凍らせた manual clock では `rx_recv` の wait が終わらないので，この系列は実時計のままにした．
  2. queue の worker（2 回目と同じ） → 上と同じ理由で退けた．
  3. 停止後の tail がブロックの先頭だけを見る → 直した（最後のサンプルの instant が停止より前であることを見る．P204/P214/P215/P216 は再実行して killed）．

## 残ったこと

- 指示書の「B-3 は 5 本とも control.rs」からの 1 本の逸脱（silent テストは rx.rs）．rx の private な `recv_timeout`/`turn`/`receive` を見るためで，control.rs に置くと本番の可視性を test のためだけに広げることになる．
- 指示書と表 B-3 の「`Direct` は実時間の時計」は誤り（`fake()` が Manual を強制する）．実装は制御時計で行った．
- 表 B-3 は `Direct` 系の 6 系列だけを挙げるが，`direct.now()` を読んでから `now + 40〜300 ms` に予約する同型のテストは `tests/fake.rs` にほかにも多数ある（`direct.now()` の grep で約 40 か所，例 `move_open_refusal_recovery`，`ur_34_…` の move 系，`ur_26_d14_…`）．表の範囲外なので触っていない．同じ規則の対象になり得るので，次の段階で表を作り直すか判断が要る．
- `fake` binary の全件は，他の利用者の負荷の下で 3 本（移設と無関係）が落ちた．単独では 3 本とも通る．orchestrator の gate で空いた状態の全件を流してほしい．

## 修正の回（Sol）

Sol の major 2 件をどちらも採った．変えたのは `#[cfg(test)]` の中の 2 本と，spec 18 のその 2 行の説明だけ．本番のコードは変えていない．

- **major 1（`src/device.rs`，`ur_33_the_fake_device_keeps_the_devices_queues`）**：200 ms の「完了しない」を判定から外した．worker は 17 件目の `apply` の前に装置の時刻を読み（`started`），呼び出しが戻った後にまた読む（`finished`）．参加者は 17 件目の呼び出しの間ずっと時計を止めている（素の channel で待つ間も participant は busy のまま）ので，呼び出し前の時刻は 0 で，16 件は 20 ms 以降に残っている．参加者はその後 `host.sleep(200 ms)` で時計を進め，17 件目は最初の command が due になる 20 ms まで戻れない．assert は (a) 呼び出し前の時刻が 0，(b) 戻った時刻が `ms(20)` 以後，(c) `apply rx` が 17 件で，17 件目が `at=<100 ms> effective=<100 ms>`，(d) 最後に join．200 ms は機能の判定ではなく，時計を先へ進める待ちにだけ残し，実時間の上限は `recv_timeout(1 s)` のハングの見張りにだけ残した．
- **major 2（`src/provider/rx.rs`，`ur_25_a_stream_silent_before_e1_switches_before_e2`）**：`recv_timeout()` の式（`cut + quiet_span - now`，1 ms の下限）から期待値を出した．この rig は 200 MHz，block 2 000 samples，`quiet_span` 5 ms なので，(a) cut の 10 ms で `recv_timeout() == 5 ms`，(b) cut+span を過ぎた 15 ms で `== 1 ms`（下限），(c) `rx.turn()` が装置に渡した値 `device.min_recv_timeout() == 1 ms`．「100 ms 未満」の 2 つの assert を置き換えた．旧テストの「1 ms 未満を渡さない」は (b)(c) の等値で戻った．
- **spec 18**：`design/18-uhd-radio.md` の 2 行の「assert」欄を今の assert に合わせた（queue は「17 件目が止まる」→「queue が満杯の零で呼ばれ，最初の command の 20 ms まで戻らない」，silent は「100 ms より短い」→「cut+span の 5 ms，その後は 1 ms の下限」）．規則本文と Changes の 2026-10-23 の行はそのまま（同じ変更の記述）．

### mutation

`remote_mutate.py phase7 sim02:2 B01 B02`（`chmod 644` の後）：**B01 killed，B02 killed**．B01 は `rx_recv` に `RECV_TIMEOUT`（100 ms）を渡す変異で，`min_recv_timeout == 1 ms` が落ちる．B02 は stop の記録を消す変異で，`applied` の `rx_stop` 1 件が落ちる．

queue のテストを `filter` に持つ行は**全リストに無い**（`plan/maintenance/tools/mutations.json` と `plan/phase{3,4,5,6,7}/tools/mutations.json` を grep：filter にこの名前を持つ行 0，`old`/`new` にこの名前を持つ行 0，`command_queue` を `old` に持つ行 0）．指示書は「queue のテストの行」を前提にしていたが，存在しないので流していない．このテストの新しい assert が殺す誤り（queue の制限を外して 17 件目が零で戻る実装）は，行が無いため mutation では確かめていない．

### 確かめたこと

- 直した 2 本を各 20 回（`--test-threads` 既定，負荷なし，`timeout 300`，1 binary ずつ）：**20/20，20/20**．
- `cargo test -p ezsdr-radio-uhd --lib`：295/295．
- `cargo clippy --workspace --all-targets -- -D warnings`：通過．`rules.py --base HEAD`：0 problems．`git diff --check`：clean．

### セルフレビュー（修正の回，1 回）

`review-fix-1.log`（`openai/gpt-6-luna#max`，`--agent plan`，checklist は `checklist-37b.md`）：**P1 0，指摘なし**．直す所が無かったので 1 回で止めた（指示書の「1 回，P1 が出たらもう 1 回まで」）．reviewer はユニット 295 件と `git diff --check` を自分でも流して成功を確認し，queue の待機判定は参加者の登録で時計が止まるので worker の遅れで崩れない，と書いている．mutation・負荷・docker gate は checklist の範囲外なので，上のとおりこちらで B01/B02 を流した．
