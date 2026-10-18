STATUS: blocked: orchestrator review O1–O5 の修正と最終 gate が別実装役／orchestrator の作業中。この報告は前セッションの実装状態を記録する。

# Phase 8 段階 1b-3c 実装報告

## 証跡の基準

- 対象は前セッションの HEAD `e4ca550` からの実装。現在進行中のレビュー修正は含めない。
- 「最後の編集」は、前セッションの `impl.log` で最後に記録された、`plan/phase3/tools/mutations.json` の M34 retarget とする。後続の `remote-mutate-m34-retarget.log` と `cargo-test-workspace-final.log` はこの編集後。ほかの各ログの前後関係は §8 に記す。
- 指示書の確認1〜7は再実行していない。記載は保存ログと `impl.log` の出力であり、最終 gate で orchestrator が流し直す。

## 1. 変更ファイルと要点

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-mock-radio/src/lib.rs` | `keys` instance の `cold` 移動を Provider 側で instance-wide effective instant に床付けし、その時点で投影・検証して両 line に原子的に記帳。link のない RX stream、TX の両 stream 更新、channel mode と Manifest を実装。 |
| `crates/ezsdr-mock-radio/src/channel.rs` | instance 共通 Transmitter が TX stream segment を channel の論理番号・block 内位置・時刻・path delay・gain・frequency・phase に従って扱う。 |
| `crates/ezsdr-mock-radio/src/profile.rs` | layout 用 Description とその設定検証を MockRadio から利用。 |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs` | move、channel mode、Manifest、T0 floor 等のテストを追加。最終 focused suite は34 tests。 |
| `plan/maintenance/tools/mutations.json` | P149〜P173を追加し、変更で旧 `old` が一致しなくなった P128/P143 等を retarget。重複していた P129 は削除。 |
| `plan/phase3/tools/mutations.json` | layout 分岐追加で旧箇所を外れた既存 mutation を retarget。 |
| `design/07-radio-model.md` | RM-25 の Provider floor と RM-34 / `book_move` の line floor の責務を整理。 |
| `design/08-simulation.md` | SE-6 の jitter の説明を stream ごとの node に追随。 |
| `design/09-mock-radio.md` | MR-18、Manifest の segment 情報、MR-31〜MR-41 等を更新。MR-40 を RM-34 に沿った move にし、MR-41 は撤回 stub。 |
| `design/11-simulation-channel.md` | CH-8 の論理 channel と segment の扱いを追随。 |
| `design/vision/05-coherence-calibration-and-runtime-semantics.md` | phase/coherence と path-delay の説明を更新。layout ごとの path-delay key の記載不足はセルフレビュー3で残った。 |
| `Ez-SDR_v4_ARCHITECTURE_VISION.md` | Vision revision history を追加。 |

`crates/ezsdr-radio` は変更していない。既存の `Lines::book_move` を呼び、effective instant は Provider が先に確定して渡す。UHD Module、server、Python、fixture、schema は変更なし。指示書の3つの既存 MockRadio test file（`mock_radio.rs`、`mock_channel.rs`、`timeline.rs`）にも変更なし。版上げ、commit、push、git add はしていない。

## 2. `ezsdr_radio` の追加・変更

なし。既存の `Lines` / `book_move` API を利用した。新しい処理は `ezsdr-mock-radio` 内に置いた。

## 3. 指示書から外れたところ

- `design/08-simulation.md`、`design/11-simulation-channel.md`、Vision とその index も更新した。今回の stream jitter、channel mapping、coherence/path-delay の仕様変更で旧記述が偽になるため。
- Phase 3 の既存 mutation 行も retarget した。layout 分岐が入った後も同じ規則の誤実装を狙うためで、行の `name`・`filter`・`test_bin`・crate は維持。
- Profile の visibility はこの段階では変更しない。UHD 1b-4 で `ezsdr_radio` の共有境界を見直す余地を残した。
- 実機・Mock↔X310 parity は実施していない。Phase 8 bench は owner の機材準備待ち。

## 4. 前段からの持ち越し

| 持ち越し | 扱い |
|---|---|
| 1b-3b §11 #1／fix-report §6 #1／Sol の必須整理：RM-25 instance-wide floor と RM-34 / `book_move` の line floor | **対応。** Provider が方向・stream・line のない stream を含む先行 `cold` の床から `e` を決め、設定を `e` に投影・検証してから `book_move(..., Some(e), ...)` を呼ぶ。RM-25/RM-34 も責務を分離。P149/P151 と floor test で固定。 |
| 要求 instant と床付け後 instant の membership 不整合（1b-3b §11 #1） | **対応。** move / membership 判定を床付け後の設定で行う。範囲外 stream を記帳前に拒否する test と P165 も追加。 |
| line のない RX stream への／からの move | **対応。** 停止状態の line を持たせ、未接続 RX は segment、SampleClock、block を作らない。panic しない。P153 と linkless test で固定。 |
| Sol minor：T0 まで床付けされた RX `cold` の `LATE_COMMAND` | **layout mode で対応。** MR-18 と T0 test が通知を固定。現 mode の挙動は変えない判断だったが、現 mode の文言・例外の記載は orchestrator review O1 に残り、別実装役が修正中。 |
| RX move の release＋destination lead、TX move の両 clock cut と両 held burst cancel | **対応。** `mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order`、`mr_40_a_transmit_move_cuts_both_clocks_and_cancels_both_held_bursts`。 |
| MoveError 時に何も変更しないこと | 状態・line・Manifest の不変は test 済み。`COMMAND_REJECTED` **event 種別** assertion は未固定（P173 の kill ログだけでは代替できない）。セルフレビュー3と orchestrator O5 で指摘され、別実装役が修正中。 |
| 1b-3b §11 #2／fix-report §6 #2：device Stop/loss の全 stream と start rollback | `<device>` Stop と `device_lost` は前段で test / mutation（P144/P145）済み。複数 stream `start` 失敗時の巻き戻しは、失敗注入手段がなく test/mutation なしのまま。rollback と Manifest 行の検証は持ち越し。 |
| 1b-3b §11 #3：MR-10、MR-16、MR-29 等の専用 mutation | MR-10 の専用行、MR-29 の不正 `start_rx` target 行、MR-16 の内部 target check（layout routing では到達不能）は未解決。旧 MR-34 の否定文は今回の channel-phase 規則に置き換わり、P158〜P160 で phase の誤りを試す。MR-41 の受理側は P162 を追加したが、その test の assertion が弱いとの O4 が残る。 |
| 1b-3b §11 #4／fix-report §6 #4：Stop/loss 後の floor の読み方 | Stop/loss が取消した pending update を今後の floor に含めない前提で実装・記述。O1 の MR-18 文言修正は別実装役の対応待ち。 |
| 1b-3b §11 #5〜#7：`Concrete`/`concrete` visibility、Vision の旧 radio-key 文、`Profile.selection` | visibility の見直しは1b-4へ。Vision の「direction ごとに一つ」の旧記述と対応 mutation 整理は1b-5へ。`Profile.selection` の未使用は今回対象外。 |
| 1b-3b §11 #8〜#9：1b-5 mutation 整理、1b-4 UHD への申し送り | 1b-5 で P10/P12/P15 と P122〜P140 の適用先を見直す。UHD は未変更で、1b-4 が layout selector / Description を利用する段階。 |
| 1b-3b fix-report §6 #2：`Transmitter` 写像、`lo_retune` draw order、`Profile::description()` panic | **対応。** channel mode は instance 共通 Transmitter で論理 channel を segment に写像。RX→TX→timed-tune constants の draw order と channel 単位 retune を test。`Profile::description()` 呼び出しは MockRadio 内で除去（セルフレビュー時の grep は0件）。 |
| fix-report §6 #3：`cold_streams` の戻り値、invalid stream の拒否文 | 今回の move 実装に沿って整理し、範囲外 stream の拒否を MR-18/P165 で固定。 |
| 既知の Phase 7 `G09` | 変更対象外。全リスト実行の既知 survivor として残る。 |

## 5. 規則ごとの test と mutation 行

`P149`〜`P173` はすべて `plan/maintenance/tools/mutations.json`。通常の行は `remote-mutate-all-final.log` にあり、初回で問題が出た行と最終 targeted run は §6 に分けて記載。

| ID | mutation（誤った実装） | kill を狙う test |
|---|---|---|
| P149 | move の床を2 line の pending `cold` に限定 | `mr_40_a_move_uses_the_instance_floor_across_streams_directions_and_missing_links` |
| P150 | MoveError なのに configuration を変更 | `mr_40_a_move_error_changes_neither_line_configuration_nor_manifest` |
| P151 | Provider が `e` を決めず、Lines に別 instant を選ばせる | `mr_40_a_move_uses_the_instance_floor_across_streams_directions_and_missing_links` |
| P152 | TX move で source stream だけを切り替える | `mr_40_a_transmit_move_cuts_both_clocks_and_cancels_both_held_bursts` |
| P153 | linkless RX が `start_rx` 後に block を発行 | `mr_40_a_move_to_and_from_a_linkless_receive_stream_never_publishes_there` |
| P154 | Manifest の segment channel 順を block order の逆にする | `mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order` |
| P155 | TX が論理番号を block 内 position として読む | `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` |
| P156 | RX field を論理 channel でなく block position で読む | 同上 |
| P157 | TX gain update を指定 channel でなく channel 0 に適用 | `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` |
| P158 | LO 初期 phase を RX より TX 先に draw | `mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior` |
| P159 | `random` channel が retune 時に再 draw せず timed constant を使う | `mr_34_a_cbx_layout_redraws_only_the_retuned_channel` |
| P160 | `deterministic` channel が初期 random phase を draw | `mr_34_ideal_layout_channels_are_deterministic_and_reproduce_with_the_seed` |
| P161 | instance 共通 TX plan を stream 0 にしか割り当てない | `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` |
| P162 | `keys` layout を channel mode から MR-41 で拒否 | `mr_31_a_layout_instance_joins_simulation_channel`（assertion の不足は O4 で修正中） |
| P163 | T0 に床付けした RX 更新で `LATE_COMMAND` を出さない | `mr_18_a_layout_receive_floor_at_t0_emits_late_command` |
| P164 | SimulationChannel を TX stream ごとに fragment へ join | `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` |
| P165 | 範囲外 stream move を coerce 拒否より先に記帳 | `mr_18_a_layout_rejects_an_out_of_range_stream_before_booking_a_move` |
| P166 | TX frequency update を指定 channel でなく channel 0 に適用 | `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` |
| P167 | RX gain update を指定 channel でなく channel 0 に適用 | 同上 |
| P168 | RX frequency update を指定 channel でなく channel 0 に適用 | 同上 |
| P169 | move 後、番号の小さい旧 stream の segment を優先 | `mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream` |
| P170 | timed tune で layout coherence Manifest を更新しない | `mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior` |
| P171 | TX path delay 前に sample を radiate | `mr_32_a_layout_transmit_path_delay_silences_samples_before_the_antenna` |
| P172 | segment end (`ended_at`) を Manifest に書かない | `mr_40_a_receive_move_waits_for_release_and_lead_and_records_channel_order` |
| P173 | MoveError が `COMMAND_REJECTED` event を出さない | `mr_40_a_move_error_changes_neither_line_configuration_nor_manifest`。ログは kill だが test は event kind を assert していないため、証拠として不十分（O5）。 |

## 6. mutation の当て直し・生き残り

- maintenance: P128（move を拒否せず旧 stream に記帳）、P143（要求時点の configuration で stream を選択）を retarget。P129 は削除。P149〜P173 は追加。最終の `old` 一致チェックで報告対象行は各1回。
- phase3: M34/M45〜M50/M57/M59/M62/M68/M69/M75 を layout 分岐に retarget。M36（RX clipping）と M55（RX LO rotation sign）も当初の survivor を受けて layout 分岐へ retarget。最終 M34 retarget 後の `rules.py` は0問題、old 一致は1回。
- 初回全リスト `remote-mutate-all-final.log`: **934/939**。maintenance 561/563（P154 compile error、P171 survived）、phase3 81/83（M36/M55 survived）、phase4 34/34、phase5 29/29、phase6 63/63、phase7 166/167（既知 G09）。
- `remote-mutate-fix-round.log`: P154、M36、M55 は killed。P171 はまだ survived。
- P171 は初回と途中の二回で survived。layout の path delay を無効化する mutation に retarget 後、`remote-mutate-p171-final-3.log` で **killed**。旧候補の `saturating_sub` 変更自体は同じ誤りを表していなかった。
- `remote-mutate-m34-retarget.log`（最後の編集後）: phase3/M34 `MR-32 transmit path delay ignored` **killed**。
- したがって、修正後に個別実行された task row は P154/P171/M36/M55/M34 すべて killed と記録される。ただし **最後の編集後の全リスト `all` は流していない**。単一の最終全リスト結果として扱わない。G09 は既知の無関係 survivor。
- P173 は実行ログ上 killed だが、test assertion の不足をセルフレビュー3と orchestrator review が確認した。正しい assertion／mutation としては未解決。

## 7. 確かめること1〜7

| # | 結果 | ログ／最後の編集との前後 |
|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast`: **1248 passed / 0 failed**（65 test-result summaries）。最終 Mock layout は34 tests pass。 | `cargo-test-workspace-final.log`。**後**。M34 retarget の後に完走。 |
| 2 | workspace clippy: exit 0。 | `cargo-clippy-workspace-final.log`。**前**。最終 retarget 後の再実行ログではない。 |
| 3 | Docker `--features uhd` clippy: exit 0。 | `docker-clippy-uhd-final.log`。**前**。 |
| 4 | `cargo build -p ezsdr-server` success、Python **47 tests OK**。 | `python-suite-final.log`（build server を含む）。**前**。 |
| 5 | `rules.py --base HEAD --renames …`: 0 problem(s)、`--self-test`: ok。 | `rules-final.log` 自体は**前**。ただし M34 retarget 後に同じ rules check と self-test が成功した出力が `impl.log` にある（**後**、専用 log への保存なし）。 |
| 6 | 全リストは上述の **934/939**。対象 survivor の targeted rerun は §6 の通り。 | `remote-mutate-all-final.log` は**前**。`remote-mutate-fix-round.log` と P171 の最終 rerun も最後の M34 編集より前。`remote-mutate-m34-retarget.log` は**後**。全リストの最終再実行は未実施。 |
| 7 | 指定3 test file の diff は空。 | `tests-diff-final.log` は0 byteで**前**。`impl.log` にも `git diff --quiet …` 成功の出力があるが、最後の M34 JSON 編集より前。以後の前セッション編集は mutation JSON のみ。 |

補足: `focused-mock-layout-final.log` / `mock-layout-final-focused.log` は33/33 pass だが、いずれも**前**。後の focused 34/34 と最終 workspace test は `impl.log` / `cargo-test-workspace-final.log` に記録。

## 8. セルフレビュー各回

| 回 | 判定・主な指摘 | 対応／最後の編集との前後 |
|---|---|---|
| 1 (`self-review-1.log`) | BLOCKED。stream 1→0 の逆向き TX move で旧 segment が新 segment を隠す可能性。D13 の coherence basis / valid range が Manifest にない。 | segment precedence を直し P169 を追加。coherence Manifest と test/P170 を追加。両方とも後続の review で確認。**前**。 |
| 2 (`self-review-2.log`) | CHANGES_REQUIRED。sample 0 開始の TX path delay が `saturating_sub` で早期 radiate し得る。Manifest `ended_at` の test gap。rules log が古い。 | layout-only early-sample guard と sample-0 test/P171、`ended_at` test/P172、rules 再確認を実施。**前**。 |
| 3 (`self-review-3.log`) | PASS_WITH_RISK、実装 blocker なし。残り: Vision の layout path-delay key の説明不足、MR-31 join test の assertion と説明の不一致、MoveError event 種別 assertion 不足、start rollback test gap、非zero RX path delay の将来リスク。 | P169〜P172 と `ended_at` は確認済み。MR-31 / MoveError は未完で、後の orchestrator O4/O5 と一致。**前**（最終 M34 mutation retarget より前）。 |

## 9. orchestrator review と残件

`opus-review.md` は最後の編集後のレビュー。別実装役が修正中のため、以下はこの報告に含まれず未解決として記録する。

1. **O1**: MR-18 から現 mode の receive line origin floor と T0 floor 時に `LATE_COMMAND` を出さない扱いが抜けている。現 mode のコード挙動は維持し、仕様を補う。
2. **O2**: `start` が現 mode でも linkless RX の `line()` を呼び、従来挙動を変える可能性。layout と link のある経路に制限。
3. **O3**: `start` rollback コメントが「この段階は move なし」のまま。KC-* による不可達を説明するか、必要なら Lines 全体を保存して戻す。
4. **O4**: `mr_31_a_layout_instance_joins_simulation_channel` は空 coupling 等だけを確認し、design の「join/arm」の主張を固定していない。
5. **O5**: MoveError test は rejection/state/Manifest を確認するが `radio.COMMAND_REJECTED` event kind を確認しない。P173 の kill はこの欠落を補わない。

さらに、start 失敗時の複数 stream rollback の test/mutation、MR-10・MR-29 専用 mutation、`Concrete`/`concrete` の visibility 見直し（1b-4）、Vision の旧 key wording と mutation 整理（1b-5）は持ち越す。非zero RX path delay は現 profile では0で、将来有効化するなら現行扱いを再確認する。Phase 7/G09 は無関係な既知 survivor。

## 10. 未実施・次へ渡すもの

- 確認1〜7の最終再実行は本報告では行わない。特に workspace/Docker clippy、Python、mutation 全リストは最後の編集後のログが揃っていないため、orchestrator gate で流す。
- 実機 parity、bench、UHD 1b-4 は未実施。Phase 8 bench は owner の準備待ち。
- 1b-5 の切り替えでは Vision の旧 radio key 記述と mutation 行を整理する。
- この報告は、別実装役の O1〜O5 修正前の証跡を記録したもの。修正後の結果を追記・置換する場合は、後続側がその時点のログで報告する。
