**報告: Phase 8 lane 1「tool fixes」(work/tools、base 464f699d)**

欠陥 (1) S4 と (2) S5 は修正し、全チェックが通りました。(3) C2 はツール側では解けないので、変更せずに止めました。未コミットです。production(Module / Kernel)の変更はありません。

```
git diff --stat (6 files, 276 insertions(+), 27 deletions(-))
 tests/common/measurement.rs   (S4: overflow_capture_samples)
 tests/common/s4bench.rs       (S5: 3 関数)
 tests/fake.rs, tests/hardware.rs
 plan/maintenance/tools/mutations.json   (TOOLS-01..05, 11..14)
 plan/phase8/bench.md
```
(パスは `crates/ezsdr-radio-uhd/` 以下)

**オフロード内訳**
- DeepSeek が担当: s4bench.rs、measurement.rs、fake.rs、hardware.rs の編集、mutation rows 案、S4 節の bench.md。2 回(ds-1 = S5、ds-2 = S4)で、差し戻しはありません。
- 私が担当: 要点行を task に書くこと、diff の逐次レビュー、probe、TOOLS-05 の追加、S5 節の bench.md、実行と検証。

## (1) S4 の capture(200 MS/s で RS-8a 超過)

**原因**
- `overflow_manifest`(measurement.rs:2828)は capture に `after + stall + 250 ms` 分の sample を求めていた。200 MS/s で 250 M sample = 2.0 GB(cf32)。
- Sink は書きながらその全 byte を hash する(`ezsdr-sink-capture/src/lib.rs`)。
- S4 が capture から読むのは `first`、`overflow_restart` の gap 開始・len・lost と `partial:false` だけで、sample は一つも読まない(VERIFIED、`extract_overflow_trial`)。
- stall 中に capture へ書かれるのは受信 buffer が持つ分(INFERRED 50 MB sc16)までで、残りは gap(byte なし)。だから stall 全体を求める必要がない。

**修正**
- 新関数 `overflow_capture_samples`(measurement.rs:2814)は `after + min(stall, 2×fill) + 100 ms` 分を求め、125 M sample(1 GB)で上限を切る。
- 100 MS/s × 500 ms が 125 M sample で、これは bench で RS-8a 内に収まった最大の大きさ(2 台 40/40 trial)。
- 値(旧式との比較):

| rate / stall | 新 | 旧 |
|---|---|---|
| 1 MS/s / 20 s | 20.6 M | 20.75 M |
| 100 MS/s / 0.5 s | 85 M | 125 M |
| 100 MS/s / 2 s | 85 M | 275 M |
| 200 MS/s / 0.5 s | 125 M(cap。上限なしだと 144.8 M) | 250 M |

- cap が効くのは 200 MS/s だけ。record の 4 cell(1/10/25/50 MS/s)は旧式以下で、cap は効かない。
- 穴が閉じる前に capture が終わった trial は Incomplete → cell が INCONCLUSIVE になる。failed の Run ではない。

**テスト**
- pure test `overflow_capture_samples_asks_for_the_gap_not_the_stall`(上の全行、rate 0、cap の境界、旧式との大小、overflow 文言)。20 回連続で 20/20 通過。
- FakeDevice の rehearsal は作って外した。fake は stall を実時間で消費するため、30 s の stall に 40 s かかり DEVICE_LOST で終わった(12 s 超なので削除)。fake はこの境界を安く見せられません。

**rows**
- TOOLS-11(held = stall 全体)、12(cap 削除)、13(2×→1×)、14(tail 100→0)。全部 killed(remote_mutate、sim02/sim03)。
- `overflow_manifest` が新関数を呼ぶ配線の row は書いていない。killer が作れないため。1 行の呼び出しで、目視で確認した。

**bench.md**
- `--tmpfs` の bullet と S4 節を書き換えた。1 Run の capture は最大 1 GB で、`EZSDR_BENCH_TMPFS=12g` は不要、既定の 4 GB で足りる。
- Incomplete の意味と、`EZSDR-INFRA ... RS-8a` が出た時の手順(止める・電源入れ直し・owner へ)も入れた。hardware.rs の comment も同様に直した。

**SIGSEGV(ツールの teardown か Module かの判定)**
- `journalctl -k` で見ると、3 回の crash(10:43 #2 S4、11:36 tx200 probe、18:47 S4)は同じ ip offset 0x7601f1。場所は libuhd 4.10 `multi_usrp_rfnoc.cpp` の `unordered_map<..radio_control>::operator[]`(addr2line で確認)。
- RS-8a の abandoned 後、Provider の thread が生きたまま process が終わる時に起きる。
- INFERRED: UHD の exit 時 crash で、Sink / Module の欠陥ではない。gdb は ptrace_scope で使えず、直接の確認はできていません。

**INFERRED(未確認)**
- cleanup 時間が capture の byte 数でどう伸びるかは未確認です。
- 1 GB が 200 MS/s の Sink(hash 1.6 GB/s)でも 5 s に収まるかは bench 待ちです。100 MS/s での 1 GB の実績しかありません。
- 200 MS/s で受信 buffer が 124 ms 分(2×fill)を超えて持つと、穴の直後に capture が終わり INCONCLUSIVE になります。結論が誤るのではなく、Incomplete 側に倒れます。

## (2) S5 の 2 channel pass

**原因**
- `hw_s5_lead_rungs` は 3 つ目の pass の前に `set_selector_override(None)` を呼ぶ。
- すると layout が `layout_of`(mod.rs:105)から決まる。`take_while(board == first)`(同 110)は先頭と同じ board だけを残す。
- bench の #1 は slot A が OBX、slot B が UBX なので受信 channel は 1 本になり、`rx_two_channel_gate`(measurement.rs:2380)が "not runnable" を返す。
- 既存の rehearsal は OBX×2 の fake だったので、mixed layout を一度も通らず green のままだった。

**修正(s4bench.rs:505 / 528 / 542、hardware.rs は呼ぶだけ)**
- `s5_two_channel_selector`: device 自身の前 2 つの受信 front end から `{rx: [0:A:b0:0, 0:B:b1:0], tx: [0:A:b0:0]}` を作る。受信は A と B、送信は A のみ。slot B が空・未知・channel 2 本未満なら Err。
- `s5_two_channel_receive_lead`: 保存・設定・復元を行う。Err なら `INCONCLUSIVE: not runnable` の 1 行。
- `s5_passes`: 3 つの pass を順に流し、前の pass が error なら後の pass を止める。
- 偏差: 1 pass 目と 2 pass 目の後は device の spec が slot A だけに残る。channels が 2 未満なら `set_subdev_spec(Rx, "A:0 B:0")` で戻す。UHD の channel 数は subdev spec の大きさだから必要。
- 実機の spec 挙動は INFERRED です。fake では probe で必要と確認しました(下記)。

**テスト**
- `s5_passes_run_all_three_passes_on_a_mixed_obx_ubx_fake_and_restore_the_slot_a_pin`(新規)。mixed fake で次を見る。
  - selector の値。
  - 3 つの decision と、2ch が `restart-retune-rx-2msps-2ch` を名指して "not runnable" でないこと。
  - `rx_open 0 [0, 1]`、slot B の transmit なし、pin の復元。
  - 不正な plan で error 後に後続が止まること(device 呼び出しなし)。
- 既存 `s4bench_s5_lead_passes_run_the_records_groups_at_other_rates_and_channels` を mixed の fake に変更し、2 つの case を足した。
  - 空 slot B(`one_obx`)は 1 行の not runnable で、`apply rx 1` なし、override 復元。
  - OBX+CBX は `0:B:cbx:0` の not runnable。
- 20 回連続で 20/20 通過。

**rows**
- TOOLS-01(2 本目を slot A)、02(tx に slot B 追加)、03(復元なし)、04(error 後も続行)、05(fallback の subdev 切り替えを除去)。全部 killed。
- TOOLS-05 は私が追加した。ds-1 の偏差がどのテストでも守られていないと row が無かったため。
- probe(/tmp の copy)で、fallback を外すと上の 2 テストが落ちることを確認した。

**bench.md**
- S5 節の (3) の説明を書き換えた。

## (3) C2(100 / 200 MS/s)はツール側では解けない(停止)

**原因(bench/2026-11-18-mt3 のログ)**
- 20 trial すべて、最後の send の handed が own time より 1.1〜1.3 ms 後だった(例: 100 MS/s の `Some((57547220, 57323594))` = 約 1.12 ms 遅れ)。
- 最初の send の遅れは約 1 ms で、20 ms の burst の最後までほぼ一定。host の送りは終始 stream rate で、radio 側の flow control に pace されている。
- 遅れた最初の packet の後に on-time の packet が来ないので、「最後が 1 ms 前に handed」という前提は成り立たない。

**1 ms の前提が成り立たない理由**
- radio の ingress buffer は 256 KB で、200 MS/s で 0.33 ms 分、100 MS/s で 0.66 ms 分しかない(INFERRED)。
- 前提の margin は、burst の長さや最初の遅れではなく、この buffer の余裕で決まる。
- さらに `handed` は `time_now` の読み出し後の値で、その latency が 100 µs 台あると見込まれる(INFERRED)。この余裕では判別できない。

**100 MS/s の見込みと、変更しなかった理由**
- 遅れを数十 µs に絞り、radio 側の margin に前提を置き換える設計なら、100 MS/s では成立する可能性がある。
- 200 MS/s は余裕が 0.33 ms で、host 側の read では前提を確かめられない。
- どちらも bench で host→radio の遅れを測らないと前提が決められません。production surface を増やさずに設計はできますが、未検証の値を verdict に入れることになります。
- 前提が成り立たない trial を PASS にすると、per-packet drop の device でも沈黙して false PASS になります。そのため verdict と C2_RATES は変更していません。
- 現在の挙動(INCONCLUSIVE + 理由に margin の値)は前提の欠落を正しく報告しており、false PASS は出ません。
- 提案: C2 の 100/200 MS/s は bt2 まで bench 計画から外す(INCONCLUSIVE が確定で bench 時間の無駄)。

## チェックと確認

- `cargo fmt --all --check`: OK。
- `cargo clippy --workspace --all-targets -- -D warnings`: OK。
- UHD feature の clippy は `rdev.py` 経由で OK。`hardware` test の build も `--no-run` で OK。
- `cargo test --workspace`: 3160 passed、0 failed。
- 新テストの 20 回連続: S5 の 2 本と S4 の 1 本、すべて 20/20。
- `rules.py --base 464f699d`: 0 problems。
- mutation rows: 9/9 killed(TOOLS-01〜05、11〜14)。
- Python suite は走らせていません(Python の変更なし)。

**adversarial review(テストごとに 1 行)**
- `overflow_capture_samples_asks_for_the_gap_not_the_stall`: TOOLS-11〜14 の誤変更がそれぞれこの test の値の assertion で落ちる(killed)。
- `s5_passes_run_all_three_passes_on_a_mixed_obx_ubx_fake_and_restore_the_slot_a_pin`: TOOLS-01〜05 が killed。旧挙動(`layout_of` 由来)でも "not runnable" の assertion で落ちる(コード読みで確認)。
- `s4bench_s5_lead_passes_run_the_records_groups_at_other_rates_and_channels`: 空 slot B の部分は、`board(1)` を `board(0)` に変える probe で落ちた。ただしこの probe は先に mixed の部分で落ちるため、空 slot B の assertion そのものでは確認できていません。単独の probe は未実施です。
- 誤 verdict の可能性: S5 は前提が欠けると INCONCLUSIVE 側に倒れる(false PASS の経路なし)。S4 は capture 不足で Incomplete → INCONCLUSIVE になる。
- 逆に false FAIL になりうる点: S4 の 2×fill が小さすぎる場合の誤った Incomplete は INCONCLUSIVE にしかならない。

**偏差**
- S4 の `overflow_manifest` 配線の row と FakeDevice rehearsal は作っていない(上記の理由)。
- C2 は変更なしで停止。
- S5 の `set_subdev_spec` fallback は task 外の偏差で、上記のとおり確認済み。

**INFERRED の一覧**
- cleanup 時間の byte 数依存(1 GB が 200 MS/s の Sink で 5 s に収まるか)。
- 受信 buffer 50 MB。
- 実機での S5 の subdev spec 挙動。
- SIGSEGV が UHD の teardown 由来であること。
- X310 の radio ingress buffer 256 KB と host の flow control が C2 の前提を決めていること。

bench は未実施です。attenuator の確認も不要(S4 は受信のみ、S5 の 2ch は送信なし)。次回 bench で見る点は、S4 200 MS/s が RS-8a 内に収まること、S5 の 2ch の decision 行、S5 は #1 のみ(#2 は slot B が空)です。

主なファイル:
- `~/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/common/s4bench.rs`
- `~/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/common/measurement.rs`
- `~/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/fake.rs`
- `~/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/hardware.rs`
- `~/local_work/Ez-SDRv4/plan/maintenance/tools/mutations.json`
- `~/local_work/Ez-SDRv4/plan/phase8/bench.md`
- ログ類: `~/.cache/ezsdr-fixes/2026-11-19-tools/`(ds-1.log、ds-2.log、rows.log)
## Fix round (after Sol's code review)

Sonnet (supervising Go DeepSeek, ds-fix-1 and ds-fix-2, no send-backs) on Sol's F1–F5 and F8, commit e04a2d7d:

- F1: S4 no longer reads its evidence from an IQ capture. A test-only recording link `HeaderMapLink` (the S9 `FirstBlockLink` shape) folds every published block header through the Kernel's `ContinuityBuilder` into the map `extract_overflow_trial` reads; the Spec's Sink captures 1 sample (`OVERFLOW_CAPTURE_SAMPLES`). `overflow_capture_samples` and rows TOOLS-11–14 removed. No production change.
- F2: `fake::s4_overflow_manifest_reads_the_gap_from_the_headers_and_captures_one_sample` (FakeDevice, 1 MS/s, stall 300 ms → one `overflow_restart` gap, Success, an 8-byte capture; stall 0 ms → Incomplete); `overflow_receive_spec_asks_for_one_sample`; `overflow_headers_form_the_capture_map_and_a_complete_trial` (pure).
- F3: `with_selector` restores the previous override on return and on unwind; S5 uses it (`selector_tests::with_selector_restores_the_previous_override_on_a_return_and_after_a_panic`).
- F4: `widen_rx_map` changes the device; a failed widening is on the decision line. Rehearsals: #2's shape (UBX in A, empty B, a one-channel map; the fake refuses the widening — UHD 4.10's answer INFERRED, either gives one INCONCLUSIVE line) and a failed widening (`FakeFault::Lost`).
- F5, F8: bench.md's S4 text (one-sample capture, when a trial is Incomplete, SIGSEGV observed after the RS-8a timeouts with the teardown attribution INFERRED); C2's 100/200 MS/s marked "do not run" until bt2.
- Rows TOOLS-01–05, 21–26, 31–32: 13/13 killed (remote). fmt, clippy (workspace and UHD), hardware build, `cargo test --workspace` 3175 passed, `rules.py --base c31a2138` clean; new tests 20/20.
