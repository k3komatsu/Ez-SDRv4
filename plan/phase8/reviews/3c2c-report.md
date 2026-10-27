# 段階 3c-2c：報告

## 実装の報告（`report-3c2c.md`）

STATUS: done

## 段階 3c-2c 報告

### 作ったもの（ファイルごと）
- `crates/ezsdr-radio-uhd/tests/common/measurement.rs`
  - `MeasurementPlan` に `raw_group_limit_ms`（既定 300000）と `frequency_points`（既定 200，リハーサル 20）。queue の試行数の検査を 3 条件に合わせた。
  - `RawGroup`（最初の失敗の latch と壁の時計の上限）。`measure_command_queue_depth` と `measure_coercion` はこれを受ける。
  - queue の条件は rx-gain，tx-gain，rx-frequency の 3 つ。rx-frequency は受信を開いて timed start し，別 thread で drain し，最後の command の後に止める。tx-frequency と rx/tx 交互は外し，理由を observation の `excluded_conditions` に書いた。値は測った条件の最小。
  - 最初の `apply`/`time_now` のエラー（または上限）の後は，その handle に何も送らない（clock 読み，untimed reset，rx_stop，coercion，片付け）。止まった時は queue の key と，終わっていない coercion の key が `unmeasured`（理由つき）。
  - `measure_raw_group`（queue，coercion，片付け）。片付けは drop で走る guard（panic でも走る）。止まった時は走らせず observation `raw_restore` に「電源の入れ直しが要るかもしれない」を書く。
  - `GroupRun`：各 group と raw probe を `catch_unwind` で個別に実行し，失敗は key を `unmeasured`（理由に失敗の文）にして observation `group_failure_*` を残す。各 group の後に `<OUTPUT>.partial.json` を上書きする。`provider_groups()` と `raw_group_keys()` がその key の表。
  - 拒否された Run の分類：`rejected_run_reason`（T0 が無い Run の termination）。`measure_rx_throughput`，`measure_tx_throughput`，`measure_overflow_restart_gap`，`measure_full_duplex` で `AdmissionRejection` にして `run_termination` を残す。full duplex は拒否があれば値を `unmeasured` にする。
  - 周波数の格子：`frequency_sweep_points`（両端と整数/整数でない MHz の混合），`frequency_values`（全点 0.05 Hz 以内のときだけ `Grid::Step{lo,hi,step:1}` と 2×最大差）。TX は gain 0 にして sweep，終わりに 1 GHz へ戻す。streamer は開かない。`SAFE_TX_HZ` はこの sweep に適用しない（他の probe は不変）。`build_record_with_identity`（serial を group の前に読むため）。
- `tests/common/mod.rs`：UR-7 の再試行を 3 回まで，間に 1 s。
- `tests/common/probe_device.rs`：試験用の `ProbeDevice` に制御呼び出しの log と，n 番目の apply を失敗させる knob を追加。
- `tests/hardware.rs`：`hw_measurements_record` を `GroupRun` で組み直した（probe は ran ∪ failed = expected，serial を先に読む，記録を書いてから partial を消し，group の失敗，raw group の停止，片付けの失敗，profile の比較をその順に assert）。
- `tests/fake.rs`：新しいテスト 12 本（下），リハーサルを `measure_raw_group` と `provider_groups` に変更。
- `crates/ezsdr-radio/tests/measurement.rs`：格子の比較テスト 1 本。
- `design/18-uhd-radio.md`：UR-34 の文と `*Test:*`，Changes に 1 行。
- `plan/maintenance/tools/mutations.json`：新しい行 P862〜P892，P739，P820〜P822，P828 を新しいコードに付け替え。

### 指示書から変えた所
- `measure_coercion` と `measure_command_queue_depth` の関数の形を変えた（`RawGroup` と plan を受ける）。hardware と fake の呼び出しは `measure_raw_group` 経由。
- 値の行（`ValueRecord`）には自由文の欄が無いので，外した条件の理由は observation にだけ書いた（値の condition は最小が出た測定済みの条件の id）。
- raw group が途中で止まった時，止まる前に読み返せた値は `Measured` のまま残る（本物の測定のため。design/18 の文もそう書いた）。
- 記録を書いた後，group の失敗，raw group の停止，片付けの失敗のどれかがあればテストを fail させる（記録は先に書く）。
- 拒否された Run の分類は Spec の Run 3 箇所と full duplex の Session。残り 4 つの timing sweep（`candidate_session`）は Session を開く所で Spec の Run ではないので変えていない（T0 が無ければ panic し，`GroupRun` が group の失敗にする）。
- 失敗した streaming の trial は `rx_stop` も送らない（rx_close はしない）。

### テスト
- 新規：`raw_group_sends_nothing_after_its_first_failed_apply`，`..._after_a_failed_clock_read`，`..._after_a_failed_timed_apply`，`raw_group_stops_a_streaming_trial_without_stopping_or_resetting_the_receiver`，`raw_group_limit_stops_every_later_call`，`raw_group_restores_after_a_panic_in_it`，`queue_depth_streams_and_drains_the_receiver_only_for_receive_frequency`，`coercion_stops_after_its_first_failed_apply`，`coercion_stopped_midway_keeps_what_it_measured_and_marks_the_rest_unmeasured`，`coercion_opens_no_streamer`，`group_run_keeps_the_other_groups_after_an_error_or_a_panic`，`rejected_runs_are_admission_rejections_not_group_failures`，`queue_conditions_stream_only_for_receive_frequency_and_the_value_is_the_least_depth`，`frequency_sweep_covers_both_ends_and_mixes_whole_and_off_mhz_points`，`frequency_values_need_every_point_within_a_twentieth_of_a_hertz`，`ezsdr-radio` の `rm_35_a_measured_frequency_sweep_passes_or_fails_the_profile_grid_by_its_ends_and_step`。
- 変えた：`provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`（3 条件，周波数 20 点と TX の全範囲，key の表との一致，TX sweep の順序），`measurement_plan_from_env_...`。
- fake のリハーサルとスレッドを使うテストを 20 回：失敗 0 / 20（`rep20-summary.log`）。ただしその後のレビュー対応で変えたテストは純粋なもの（panic の文，ProbeDevice の knob）。

### 確かめたこと
- `cargo test --workspace`：全部 ok（`workspace-test-3c2c.log`）。レビュー対応の後に `-p ezsdr-radio-uhd -p ezsdr-radio` を再実行して ok。
- `cargo clippy --workspace --all-targets -- -D warnings`，docker の `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`：どちらも clean（レビュー対応の後にも）。
- `rules.py --base HEAD`：0 problem。`git diff --check`：clean。全 list の `old` が tree でちょうど 1 回一致（1,241 行）。
- mutation（`remote_mutate.py maintenance sim02:4,sim03:4`）：1 回目 110 行（既存の関連行 80 と新しい行 30）で 109 killed，P875 が生き残り（原因：テストの `panic!("boom {}", 7)` が定数に展開されて `&str` になり，String の枝を通らない）。テストを直し P892 を足して P874〜P877，P892 を再実行：5/5 killed。レビュー対応の後に probe_device の行，loopback の行，P739，P862〜P892 の 45 行：45/45 killed。
- 新しい行 P862〜P892 のうち，検証できない所：UR-7 の再試行（hardware の helper），full duplex の拒否の枝には行もテストも無い。

### bench の時間の見積もり（推測）
- queue：3 条件 × 5 試行 × 約 3.1 s ≒ 50 s 台。coercion の gain と rate は前と同じ。周波数 sweep は 2 channel 方向 × 200 点で，1 点あたりの LO の時間が分からず（INFERRED：数十 ms から 100 ms 台）数分以内。上限 5 分（`raw_group_limit_ms`）が効く。

### spec
- `design/18-uhd-radio.md`：UR-34 に 3c-2c の段落と `*Test:*`（12 名＋追加 5 名），片付けの文を「raw group が止まった時を除く」に直した，Changes に 2026-10-27 の 1 行（設計メモ 41 stage 3c-2c）。

### セルフレビュー（free DeepSeek，1 回）
`review-3c2c-1.log`，P1 1 件，P2 2 件，P3 4 件。
- P1-1（止まった group の key が全部 `unmeasured` とは限らない）：直した。design の文を「終わっていない key」に狭め，途中で止まるテスト（`coercion_stopped_midway_...`）を足した。
- P2-1（4 つの timing sweep の拒否）：退けた。Session を開く所で Spec の Run ではない。design の文を「Spec Run」に狭めた。
- P2-2（`build_record` が止まった後の `serial()` で落ちうる）：直した。`build_record_with_identity` を足し，hardware の記録は group の前に serial と describe を読む。
- P3-1（full duplex の枝にテストと行が無い，`Err` を拒否にしている）：一部直した。`Err` は拒否にせず元の `?` に戻した。枝のテストは作れる seam が無いので残した（下）。
- P3-2（design:351 と :362 の矛盾）：直した。P3-3（`*Test:*` の抜け）：直した。P3-4（TX sweep が streamer を開かない）：`coercion_opens_no_streamer` を足した。
- 読むだけのレビューのため，テストと mutation は reviewer が流していない。

### 残ったこと
- full duplex の拒否の枝（T0 の無い Session）には，fake で通すテストも mutation 行も無い。
- UR-7 の再試行 3 回は hardware の helper の中で，fake のテストが無い。
- 4 つの timing sweep（`candidate_session`）は拒否された Session を group の失敗として扱う（分類しない）。
- `hw_measurements_record` と周波数 sweep の実機での動きは実機を流していないので未確認。bench で流す前に TX の att とケーブルが 18 GHz まで耐えることを必ず確かめる。
- 周波数の抜き取りの間の点は INFERRED（observation に記載）。

## 修正の報告（`fix-3c2c-report.md`）

STATUS: done

## 段階 3c-2c 修正の報告

指摘はすべて直した。変えたのはテスト側（`crates/ezsdr-radio-uhd/tests/common/{measurement,mod,probe_device}.rs`，`tests/{fake,hardware}.rs`）と `design/18-uhd-radio.md`，`plan/maintenance/tools/mutations.json` だけ。`src/` は触っていない。

### 指摘ごと
- **Sol 1**（`measurement.rs` の `RxProgress`，`drain_receiver`，`wait_for_rx_samples`，`wait_for_rx_end`，`queue_trial`）：drain thread が返った sample の終わりの tick（first_tick ＋ 数 × master/1 Msps）を共有の値に進め，`LateCommand`・overflow・alignment・bad packet・timeout も数える。設定を出す前に最初の `Samples` を待ち（上限 1 s），無ければ試行を失敗にして群を止める。最後の命令の時刻＋余裕まで sample の終わりが届くのを待ち（上限 `ahead`＋余裕＋2 s），届かなければ失敗にして群を止める。行に `rx_progress` を残す。
  - テスト：`a_started_receiver_that_returns_no_samples_stops_the_group_before_any_setting_goes_out`（LateCommand の後 timeout），`a_receiver_whose_samples_stop_short_of_the_last_command_fails_the_trial`，`a_receiver_whose_samples_pass_the_last_command_completes_the_trial`。`ProbeDevice::script_rx` で返す列を決める。`rx_recv > 0` の assert は `queue_depth_streams_...` から外した（rx_start/rx_stop の数は残した）。
- **Sol 2**：`RawGroup` を内部可変（`&self`）の共有 latch にし，命令・drain・片付けが呼び出しの前に見る。rx_stop が失敗したら rx_close をしない。`restore_measurement_defaults` は各 channel の前に latch を見て，最初の失敗で止まる（以前の「全 channel を試す」を変えたので，テスト名を `measurement_defaults_restoration_stops_at_the_first_apply_error` に改名した。`rules.py` は `--renames` に `old -> new` を渡さないと通らない）。止まった handle は `leak_after_stop`（`mem::forget`，`shortcut:` 注釈つき）で捨て，`hardware.rs` が記録を書いた後に呼ぶ。`ProbeDevice` の記録に `rx_recv`・`rx_stop`・`rx_close`・`drop` を足した。
  - テスト：`raw_group_sends_nothing_after_a_failed_timed_apply`，`raw_group_stops_a_streaming_trial_...`，`a_failed_receiver_stop_is_not_followed_by_a_close`，`a_failed_raw_group_s_handle_is_leaked_not_dropped`，restoration のテスト。失敗の後は，すでに飛んでいた `rx_recv` 1 回までを許し，ほかの呼び出しが無いことを見る。
  - `design/18` UR-34 の文を書き直した。
  - 未確認：実機での `UhdDevice` の drop（teardown）。leak は `Arc` の最後の持ち主が漏らす前提で，`hardware.rs` の中だけで保証した。
- **Sol 3**：`StopOnDrop`（unwind でも scope の join の前に drain を止める）。テスト `a_panic_on_the_command_thread_stops_the_drain_and_the_record_continues`（受信が流れている時の timed apply の panic が群の失敗になり，次の群が走る）。`raw_group_restores_after_a_panic_in_it` は，drain の panic（今は latch 済みで片付けない）から timed apply の panic に作り直した。
- **Sol 4**：命令の各 `apply` の前，clock 読み，wait の各周回，untimed reset の前に `raw.halted()`。テスト `the_raw_group_deadline_is_checked_between_two_device_calls`（2 回目の呼び出しが上限を越えると後ろの呼び出しが無い）。
- **Sol 5**：`hardware.rs` で `usrp()` の open，binding，description，serial，describe を group の前に `catch_unwind` の中で行い，失敗は `GroupRun::record_failure("device_setup", ...)` に残して partial を書く。ただし装置の識別が無いと Record が作れないので，その場合は記録を書けず，診断だけが残る（panic する）。これは Sol の求めた「記録が書ける」を満たしていない所。`build_record_with_identity` で，識別を先に持てば装置無しで組み立てられる。テスト `the_record_is_assembled_from_an_identity_read_before_the_groups_with_no_device`。ただし FakeDevice は `mark_lost` しても serial が失敗しないので，「group の後で装置が使えなくなる」状況そのものは作れず，装置を drop した後で組み立てられることだけを確かめた。
- **Sol 6**：`open_with_pll_retries`（`common/mod.rs`）。`REFERENCE_DID_NOT_LOCK` の文を含むエラーだけ最大 3 回，ほかはすぐ返す。テスト `ur_7_retries_only_the_reference_clock_not_locking`（PLL で 3 回目に成功，回数切れ（4 回呼ぶ），再試行しない 1 回，fake の PLL fault の文が一致）。
- **O1**：`summarise_value`・`summarise_observation`（`measurement.rs`）。probe の observation の 20 行を越える配列を，数，数値の最小・中央・最大，`class` などの数，最初と最後の 3 行，`error` のある行（5 つまで）に要約し，全部を `<OUTPUT>.rows/<probe>.jsonl`（`{path,index,row}`）に書く。入れ子の配列にも効く。`.partial.json` も同じ要約を使う。テスト `a_probe_s_long_arrays_are_summarised_and_their_rows_go_to_a_side_file`（要約の大きさ <10 kB，行の数，side file）。実機の記録が 1 MB 以下になるかは未確認（実機を流していない）。

### 確かめたこと
- `cargo test --workspace`：全部 ok。変更の後に `-p ezsdr-radio-uhd -p ezsdr-radio` を再実行して ok。
- fake の timing とスレッドのテストとリハーサル（`raw_group`，`a_panic`，`a_started`，`a_receiver`，`a_failed`，`the_raw`，`queue_depth`，`coercion_`，`provider_measurement_groups`，`group_run`，`a_probe`）を 20 回：失敗 0 / 20。deadline のテストは直した後に単独で 20 回：失敗 0 / 20。
- `cargo clippy --workspace --all-targets -- -D warnings`，docker の `--features uhd` の clippy：clean。`git diff --check`：clean。`rules.py --base HEAD --renames <改名 1 行>`：0 problem（`--renames` なしだと改名したテストの 1 件で落ちる）。全 list の `old`/`old2` が tree で 1 回一致（1,254 行）。
- mutation（`remote_mutate.py maintenance sim02:4,sim03:4`）：関連する既存の行と新しい行の 124 行で 120 killed，4 survived（P831，P862，P863，P897）。
  - P831：`return Err` を外しても次の channel の latch 確認が `Err` を返すので同値。latch を外す mutant に付け替え。P862：群の loop の `raw.fail` は latch が試行の中に移って余分になったので，その行を消し，`queue_failed` の latch を外す mutant に付け替えて filter を clock 読みのテストにした。P863：latch が wait を止めるので filter だけでは同値。wait の確認も外す二つ目の編集（`old2`/`new2`）つきに付け替え。P897：2 回目の呼び出しが stall limit を越えて loop が止まっていたので，テストの `queue_ahead_ns` を 2 s にして上限だけが loop を止めるようにした。
  - 付け替え後，P831，P862，P863，P897，P893，P895，P870，P869 を再実行して全部 killed（P862 は filter を直して再実行）。
  - 新しい行 P893〜P905（readiness，end の待ち，sample の数，StopOnDrop，deadline，rx_stop の後の close，restore の latch，leak，UR-7，要約の閾値・side file・median・outliers），付け替えた P831・P866・P869・P870・P872・P873。P893〜P905 は一度目の流しで P897 以外 killed，P897 は付け替えて killed。
  - 全 list は流していない。

### 残ったこと
- 失敗した装置の drop の効果（UHD の teardown を飛ばせるか）と，`Arc` の他の持ち主が無いことは実機でしか確かめられない。
- 装置を開けない時は記録を書けない（識別が要る）。診断は `.partial.json` に残る。
- `hardware.rs` の setup の `catch_unwind` の枝と，`leak_after_stop` を呼ぶ位置には，fake テストが無い。
- full duplex の拒否の枝，UR-7 の hardware helper の実機での動きは前回と同じく未確認。

## 修正 2 の報告（`fix2-3c2c-report.md`）

STATUS: done

## 段階 3c-2c 修正の報告（2 回目）

変えたのはテスト側（`crates/ezsdr-radio-uhd/tests/common/{measurement,probe_device}.rs`，`tests/{fake,hardware}.rs`）と `design/18-uhd-radio.md`，`mutations.json` だけ。`src/` と coordinator が commit した docs（`plan/phase8/bench.md`，`tools/` ほか）の中身は触っていない（remote mutation が copy できるよう，tree の読めない mode だけ 644/755 に直した。`tools/*.sh` の実行 bit は戻した）。

### 裁定ごと
- **major 1（片付けの失敗で handle を捨てない）**
  - `measure_raw_group` は latch の状態を片付けの後で読む（`measurement.rs`，restore が失敗すると `stopped` も Some）。
  - 装置を `DeviceLease` に入れた。失敗の印がつくと drop で `mem::forget` するので，記録を書く所の panic や assert の unwind でも効く。`run_raw_group_leased` が，raw group・片付け・先の group のどれかが失敗したら印をつける。`hardware.rs` はこの lease の形に変え，`leak_after_stop` は消した。
  - テスト `a_failed_restore_leaks_the_handle_even_when_the_record_s_writing_panics`：呼ぶ側の流れ全体（片付けが失敗 → 記録を組む → 書く所で panic → 巻き戻し）で drop が走らず，何も失敗しない同じ流れでは drop が走る。`ProbeDevice::fail_restoring_apply` を足した。
- **major 3（rx_stop の後の rx_close）**：`rx_stop` が成功した後，`rx_close` の前に `raw.halted()` を見る。テスト `the_deadline_passing_during_the_receiver_s_stop_leaves_the_close_unsent`（成功した `rx_stop` の後に注入した経過時間で上限を越えさせる）。
- **minor（上限のテスト）**：`RawGroup::advance_elapsed`（注入した経過時間）を足し，`the_raw_group_deadline_is_checked_between_two_device_calls` は実時間の sleep をやめて，2 回目の timed apply の中で経過時間を進める。timed apply の数がちょうど 2 であること，その後に何も無いことを見る。
- **major 2（thread の競合）**
  - 直列にはしていない。保証を正確に書いた：ある thread が失敗（または上限）を latch した後，ほかの thread が新しく始める呼び出しは高々 1 つで，UHD の 10 s の timeout で終わり，それより後は無い。`RawGroup` の doc と，queue の observation の `after_a_failure`，`design/18` UR-34 に同じ文を入れた。
  - テストを順に依らない形にした。`ProbeDevice::script_rx_with`（呼び出しを保持する script）と `before_timed_apply` で呼び出しを飛行中に保ち，`the_drain_begins_no_call_after_the_command_thread_latches`（命令の失敗が latch された後に drain が解放されて，新しい `rx_recv` を始めない）と `the_command_thread_begins_no_call_after_the_drain_latches`（drain の失敗が latch された後に，飛んでいた timed apply が戻って，次の apply・clock 読み・reset が無い）の両方向。待ちと解放は channel と latch を見る別 thread で決め，sleep は使わない。
  - 失敗の印が latch より前に出る順は，`assert_nothing_new_after` を「印の後に drain の read 以外の呼び出しが無い」だけにして順に依らなくした（read の数は上の二つのテストで見る）。
  - 試せていないこと：latch を見てから呼ぶまでの間に入る「高々 1 つ」そのものは，コードに待ちを足さずには決まった順で作れないので，テストは「飛行中の 1 つが戻った後に新しいものが無い」を見ている。

### 確かめたこと
- `cargo test --workspace`：全部 ok。
- clippy（workspace，docker の `--features uhd`）：clean。`git diff --check`：clean。`rules.py --base HEAD --renames <前回の改名 1 行>`：0 problem。全 list の `old`/`old2` が 1 回一致（1,260 行）。
- 20 回：変えたテストと隣（`raw_group`，`the_raw_group`，`the_deadline`，`the_drain`，`the_command_thread`，`a_failed`，`a_receiver`，`a_started`，`a_panic`，`queue_depth`，`coercion_`，`group_run`，`a_probe`，`the_record`）で 20/20 通った。その前の同種の 20 回（リハーサル `provider_measurement_groups` を含む）では 1 回，部分一致で入った別のテスト `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`（実時計の Late 判定）が落ちた。私は触っていないテストで，リハーサルを含め私のテストは 20/20 通った。
- mutation（`remote_mutate.py maintenance sim02:4,sim03:4`）：触れた行と新しい行の 130 行が 130/130 killed。付け替え：P900（lease の drop），P873（stopped の読みが変わった）。新しい行：P906（restore より前の stop），P907（上限後の close），P908（注入した経過時間），P909（lease の印），P910（命令 loop の latch 確認），P911（drain の失敗の latch）。全 list は流していない。

### 残ったこと
- 実機での `UhdDevice` の drop を飛ばす効果と，`Arc` の他の持ち主が無いことは未確認（実機を流していない）。
- `hardware.rs` の setup の panic の枝，lease を使う実際の `hw_measurements_record` の流れは fake では通していない（lease の流れは `run_record_flow` の試験で通した）。
- latch を見てから呼ぶまでの「高々 1 つ」の窓そのものは，決まった順では試していない。
