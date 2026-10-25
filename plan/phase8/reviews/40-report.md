# 設計メモ 40：実装役の報告

## 実装（`report.md`）

STATUS: done

### 設計メモ40 — 実装報告

### 結果

設計メモ40のD1・D2''・D3・D4を実装し、受理spec、schema、MockRadio、UHD Module、FakeDeviceテストを更新した。Stopはhandover済みの範囲を保ち、cold changeとdevice lossはそれぞれの瞬間で切る。バージョン、`SCHEMA_CHANGELOG.md`、orchestratorの設計メモ40は変更していない。commit、push、`git add`、`git stash`もしていない。

### 変更

- `crates/ezsdr-radio/`: `radio.timing.tx_in_flight_ns`を8つ目のTimingEnvelope memberとして追加し、x3xxを10 ms、idealを0にした。description、schema、capabilityの伝播を更新。
- `crates/ezsdr-radio-uhd/src/provider/tx.rs`: Descriptionのwindowを用い、payloadのexclusive cursorをSampleClock上で切り上げてWを越えるchunkを送らない。finite-tail、device EOB、empty-EOB padding、Stop後のsafe startとpreemptionの境界をFakeDeviceで確認するテストを追加・更新。
- `crates/ezsdr-radio-uhd/src/profile.rs`: `tx_in_flight_ns >= DEVICE_LEAD_NS`の前提を一行で記し、profileの単体テストを追加。runtime検査は追加していない。
- `crates/ezsdr-mock-radio/src/lib.rs`: 同一SampleClockで、受け取り順が先のopen/held burstから保守的frontを計算し、ordinary boundとfrontの遅い方を使う。W=0ではclosed-burst safe frontを記録しない。Stop/ProviderStopだけがhandover frontを残し、cold change/lossは残さない。
- `crates/ezsdr-mock-radio/tests/`: 同一瞬間のreceipt順、Stop H/H+1/H+2、idealのgap-free閉じ方、cold/lossのcut、loopback mediumの遅れたcutを固定。
- `crates/ezsdr-acceptance/tests/reactive.rs`: KE-03のStop後の期待sample数を80から1,000に変更（Run Stopでhanded burstがEOBまで流れる）。
- `design/07-radio-model.md`, `design/09-mock-radio.md`, `design/18-uhd-radio.md`と`schemas/radio/envelope.v1.json`: 規則・Test行・Changes行を更新。MR-17にはclosed-burst safe startが非zero windowの場合だけで、W=0は従来どおりと明記。U19のmutation説明もstrict windowに合わせた。
- `plan/maintenance/tools/mutations.json`, `plan/phase3/tools/mutations.json`, `plan/phase7/tools/mutations.json`: mutation anchor、filter、名称、字下げを更新。新規rowはP658–P660。P657は候補を試したがsurviveしたため削除し、IDは再利用していない。

差分は21 tracked filesで1,877 additions / 103 deletions。`crates/`のコード・テスト分は1,449 additions / 56 deletionsで、規模の大半は`provider/tx.rs`（902 changed lines）とMock/UHDのテスト。作業treeの`plan/phase8/40-preemption-bound.md`は読んだだけで変更していない。

### テストと既存テストの対応

- D2''の三つのreceipt順とlate policy: `mr_17_x3xx_tx_preemption_front`（same step、separate step/same instant、+1 root tick。SendAsapとDrop）、`ur_21_booking_receipt_order_changes_preemption`（bookedだけのAと、`Tx::step`で実handover済みのAを区別）。
- front式・境界・finite/repeat: `mr_17_handed_front_reduction_cases`, `mr_17_x3xx_tx_preemption_boundaries`, `mr_17_a_finite_burst_front_stops_at_its_end`, `ur_21_a_handed_repeat_sets_the_preemption_front`, `ur_21_a_preemption_front_boundary_is_on_time`, `ur_21_a_preemption_front_stops_at_a_finite_end`。
- S2: `mr_16_burst_refusals`の`moved_duplicate`はidealへ移し、held-start collisionの拒否を維持。x3xxの別caseではBがsample 2,003,002へ移り、`TIME_ERROR { send_asap }`、元の`requested_target=2,000,000`、拒否なしをassert。`mr_17_x3xx_moved_start_overlap_still_refuses`はrepeatのH=+10 msへ移したBがheld startと重なる場合の拒否を固定。
- D3 / D4 / EOB: `ur_21_send_does_not_cross_the_in_flight_window`（aligned 1 Mspsと390,625 sps）、`ur_21_tx_reads_in_flight_window_from_description`, `ur_21_a_finite_burst_with_a_held_tail_keeps_its_front`, `ur_21_a_post_eob_restart_uses_the_safe_sample`, `ur_23_stop_preserves_handed_cursor_after_stop`, `ur_23_stop_closes_a_running_repeat_at_its_handed_cursor`。
- Stop、medium、clock/stream分離: `mr_17_stop_preserves_front_until_expiry`はH/H+1をlate、H+2をon-timeで見る。`mr_25_stop_hands_a_future_repeat_through_the_in_flight_window`はBurstRecordに加え、loopbackが遅れたcutまで出てその後silenceになることを確認。`mr_20_x3xx_loss_cuts_transmit_at_its_instant`, `mr_32_x3xx_cold_change_cuts_at_its_instant`, `mr_17_tx_front_is_per_stream`, `mr_17_cold_change_does_not_cross_sample_clocks`, `ur_25_cold_change_does_not_inherit_old_tx_front`も維持。
- W=0 / late-state wording: `mr_17_ideal_closed_finite_burst_has_no_extra_gap`, `mr_17_ideal_stop_has_no_in_flight_gap`; front reductionのunit testは先行する同一instant receiptを含め、現在のreceiptと後続receiptを除外する。
- Review 3で追加したpreempt分岐のcase: `ur_21_a_late_move_past_the_cursor_closes_before_a_held_burst`。Bがcursorより後へmoveし、cursor上のCがdropされた後、Bがtimed SOBで始まることを確認。
- UHD front後のテスト名は、実際にはStopでなくfinite burst完了後を試していたため、`ur_21_a_target_after_a_completed_burst_is_on_time`に変更。Mockの対応testはStop後なので`mr_17_a_target_after_a_stopped_burst_is_on_time`。

新規・修正したtest groupsはそれぞれ20回連続で実行し全てpass。ログは`repeat-impl2-20x.log`と`repeat-review3-20x.log`。後者は最終`ur_21_`群（新規P660 test、rename、booking testを含む）とclock-isolation testを20/20で確認。

### Gate

- `cargo test --workspace`: **1,734 passed, 0 failed**。
- `cargo clippy --workspace --all-targets -- -D warnings`: pass。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems。
- `rules.py --self-test`: pass。
- `rm_20_schema_freeze`: pass。
- `git diff --check`: pass。mutation前の変更・新規filesはmode 644。
- 実行したmutationはすべて指定どおり`remote_mutate.py`からsim02/sim03へ。全mutation list、Docker/Python suites、hardware benchはこのbriefのgate外なので実施していない。

### Mutation結果

- 初回の実装段階ではP623–P651と関連するphase3/phase7 rowsをremoteで実行。最初のshard結果に不一致・compile errorがあったP625/P633/P640/P650/U15/C03を個別に直して再実行し、対象rowは最終的にkilled。P645/P652/P653/P654/P655/P656とM26/M27/M51/M65のfix roundも10/10 killed。
- 今回の追加・修正分: P627/P628/P629/P630/P631/P633/P639/P640/P641/P646/P647/P652/P654/P655/P658/P659/P660とphase3 M65は、最終remote runでkilled。P629/P630はhelper抽出後に最初のanchorがNOT FOUNDだったので4-spaceの実sourceへ修正して再実行し、2/2 killed。
- P657（`abandon`の`cursor+2`を`+1`にする候補）は`ur_23_a_continuation_whose_held_sample_is_not_taken_is_abandoned`をfilterにして試したが**SURVIVED**。既存testは後続burstのrestart boundaryを観測しないため、指示どおりrowを追加しなかった。abandon後のsafe-startは未mutation-pinned。
- full mutation listはorchestrator gateとして未実施。

### S1・S2の裁定

- Same-step A(+4 ms)→B(+3 ms): UHDはqueueをdrainしてからTX ownerがstepするためBを先にon-timeで送れる。Mockはreceipt orderでAを候補に数えBをlateにする。この保守的差を`mr_17_x3xx_tx_preemption_front`と`ur_21_booking_receipt_order_changes_preemption`の別期待値で固定。Kernel blockのEOBとdevice側EOBは別のframeなので、MockのKernel recordにEOBがありUHDがdeviceへEOBを送らないことは矛盾ではない。
- S2のx3xx collision refusalは別配置で到達可能（`mr_17_x3xx_moved_start_overlap_still_refuses`）。元のtwo-sample `moved_duplicate`だけではx3xxは拒否にならず、2,003,002へmoveする。

### Self-review 1–3 の指摘と扱い

### Review 1 (`review-1.log`)

- **P1-1 cold/lossがStopのin-flight horizonまで放射**: 修正。Stop/ProviderStopだけhand-overし、ColdChange/Lossは瞬間cut。x3xxのmedium/record testsとP654/P655/P659がkill。
- **P1-2 UR-23のINFERRED注記欠落**: 先行roundで復元。1 Msps以外のgap sufficiencyはINFERREDのまま。
- **P2-1 MR-25がclosed frontを含む記述と`closed=false`**: MR-25を「閉じたburstのsafe restartは後のbookingを制約するが、Stopでburstを再開しない」と明確化。Stop flushの`handed`計算にclosed frontを使わないのは意図的。
- **P2-2 Stop drain中のrejection timestampがhorizon**: 未変更・rejected。`emit_tx_until`の`until`はhandover drainの上限で、blockの`TIME_ERROR`はheader sample時刻を持つ。内部`COMMAND_REJECTED`の時刻をStop瞬間とする受理規則は見当たらず、Stop instantへ固定すると延長drain中に見つかったblock failureを誤った時刻に置くと判断。独立したtimestamp testは追加していない（残る不確実性）。
- **P2-3 W=0にもclosed gap**: 修正。共有`remember_tx_front`でwindow=0をno-opにし、idealのEOBとStop両方のgap-free testを追加。P639 killed。
- **P2-4 RM-15/MR-17の重複**: RM-15はProvider共通の契約、receipt-order機構はMR-17へ分離。
- **P2-5 weak per-stream/cold tests**: Mockはrecordのtarget/sampleを、UHDはstream 1の実device call/time specを確認。Mock cold-clock testにold-frontよりtarget tickが小さいguard assertを追加。UHDのfrontは各`Tx`がownするfieldで、別Txへ漏らす共有stateはない。
- **P2-6 test/mutation coverage**: Mock dropの全receipt順、UHD aligned +10ms境界、Stop front、held tail、EOB、per-stream、later-bound、cold/loss cutをtest/rowで固定。P652/P653/P654/P655/P636/P643/P656等はtargeted mutationでkilled。
- **P2-7/P2-8**: U19をexclusive edgeのrow名へ更新。旧`ur_21_a_preemption_front_includes_a_held_burst`は`ur_21_a_handed_repeat_sets_the_preemption_front`に変更済み。
- **P3**: phase3 mutation indentation、MR-3 table alignment、shared `later_bound`の使用を確認/修正。P636名は実mutationに合致し、U36名をlate-policy boundaryの説明に変更。Stop testの古いchunk-overshoot commentをstrict Wに合わせ、cold-clock testのguardも追加。

### Review 2 (`review-2.log`)

- **P1-1 adjacent burstのKernel EOB/SOB**: owner裁定どおりrejected。Mockの記録はKernel burst境界、UHDのdevice EOBはdevice frame。Mockのactive finite frontはend（+0）で、adjacent burstはon-timeにできる。
- **P2-1 expiryを観測しないtest**: `mr_17_a_target_after_a_stopped_burst_is_on_time`と`ur_21_a_target_after_a_completed_burst_is_on_time`へrenameし、Test行も更新。これらはburst後のB on-timeを見ており、state deletionは主張しない。
- **P2-2 end_open safe gaps / abandon / dispatch_start mutations**: end_openのpayload-EOB P640とempty-EOB P641はkilled、dispatch_start P658もkilled。abandon +2のP657候補はsurviveして削除（上記）。
- **P2-3 Wがdevice lead未満**: profile commentと`tx_in_flight_window_covers_device_lead`単体testで前提を固定。runtime validationは追加せず。
- **P2-4 same-instant reducer**: `handed_end_before`へreceipt-order predicateを共有し、earlier/current/later orderをunit testで固定。P629/P630 killed。
- **P2-5 current bound式の差**: rejected。ordinary RM-14 boundは本変更以前の契約であり、D2''のfront計算だけを共通化する範囲外。
- **P2-6 cold/lossにもfrontを記録**: Stop/ProviderStopだけのhandover branchにfront記録を限定。unit testとP659がkilled。
- **P3-1/P3-2/P3-4/P3-5**: mutation字下げ、U36名称、strict-W comment、cold-clock guardを修正。P636名称は既に適切。
- **P3-3 UR-23に根拠/INFERRED文**: owner裁定どおりrejected。accepted spec内のINFERRED注記を残す。

### Review 3 (`review-3.log`, 最終round)

- **P1-1 Mock x3xx closed-finite gap test不足**: rejected。`mr_17_a_finite_burst_front_stops_at_its_end` x3xxでactive finite frontの+0 boundaryをassertし、`mr_17_ideal_closed_finite_burst_has_no_extra_gap`でW=0を固定。ownerがadjacent Kernel EOB/device EOB差を明示的に裁定済み。x3xxのclosed EOB +1 sampleは2 ms ordinary leadに隠れて後続decisionへ独立した効果を持たず、illegalなextra gapを期待するtestは足さない。observable empty-EOB H+2は`mr_17_stop_preserves_front_until_expiry`で確認。
- **P1-2 UHD per-stream test不足**: rejected。`ur_21_tx_front_is_per_stream`はstream 0 Aとstream 1 Bを同一rigで実行し、stream 1が自身のtargetでtimed SOBを出しTIME_ERRORがないこと、cursorがstream1側であることをassert。`Tx`がstream単位にfrontをownする。
- **P2-1 preemptのmoved-past-cursor else branch**: 採用。pending C at cursorのもとでBがordinary boundへ遅れてmoveしたとき、Aをdevice EOBで閉じ、Cをdropし、Bがtimed SOBで始まる新test `ur_21_a_late_move_past_the_cursor_closes_before_a_held_burst`を追加。P660 killed。
- **P2-2 UHD future-handed A + Stop / P2-3 Mock medium samples**: rejected as already covered。`ur_23_stop_preserves_handed_cursor_after_stop`はs+4ms開始AをHまで実handover後にStopし、paddingとH/H+1/H+2をFakeDeviceで確認。`mr_25_stop_hands_a_future_repeat_through_the_in_flight_window`はBurstRecordとloopback sampleをdelayed cutまで、およびその後のsilenceを確認。
- **P2-4 held continuation後のMock front**: rejected。ownerの裁定ではKernel EOBとdevice EOBは異なるframe。held successorのown candidateはそのstart+1 sample以上をfrontに出し、前段のE+1は有効boundを変えない。追加の内部state-only testはしない。
- **P2-5 M65 filter空**: filterを`mr_25_a_stop_on_a_sample_instant_does_not_transmit_that_sample`に修正、M65 killed。
- **P2-6 P627/P628 mutation kill**: remote runで両方killed。
- **P2-7/P2-8**: UHD burst-after-stop名をcompleted burstに修正。cold-clock testは`new.instant(target)-device_lead`へdevice timeを置き、`now >= switch`をassertして時刻後退を防止。P647 killed。
- **P3-1/P3-2**: booking/hand-overのdoc commentを追加、U19文言をstrict exclusive edgeへ変更。
- **P3-3/P3-4**: rejected。UR-23のINFERRED/source noteはowner承認済み。schema descriptionの“Maximum time ahead”はwindowを上限と表し、sample-at-edgeの可否はnormative rule/testで定めるため変更しない。
- **P3-5/P3-6**: profile unit testはimpl2裁定が明示して要求。KE-03の80→1,000は既存test updateとして上記に記録。

### 残り・未実施

- P657候補が示した通り、abandonしたdevice burstのempty-EOB H+2後に別burstを再開するsequenceは既存testで直接観測していない。mutation rowはsurviveしたため残していない。
- Stop drain中のinternal `COMMAND_REJECTED` timestamp（Review 1 P2-2）は契約未確定として未変更。
- x3xxのW=10 msは現行profile valueでINFERRED。hardware parity / Phase 8 measurementは未実施。
- 全mutation list、Docker、Python 3.9、`--features uhd` clippy、hardware benchは実施していない。これらはbrief/orchestratorのgateに残した。

### 実行ログ

主なログはこのdirectoryの`repeat-impl2-20x.log`, `repeat-review3-20x.log`, `workspace-test-final.log`, `clippy-final.log`, `rules-final.log`, `rules-self-test-final.log`, `schema-freeze-final.log`, `review-1.log`, `review-2.log`, `review-3.log`。


## 修正 1 回目（`fix1-report.md`）

STATUS: done

### 設計メモ40 — fix 1 closeout

### 結果

F1–F4、O1、O2を実装し、review 4 と review 5 のP1を処理した。Kernel crate の差分は空。`test-plan.md` はorchestrator所有のため変更していない。すべて未stage・未commit。

前セッションは文脈を失った後、自分の実行を別runと取り違えて停止した。この続きでは他processの調査や待機はせず、既存ログと現在のtreeから再開した。

### F1–F4 / O1 / O2

- **F1 — one-sample burstのsafe restart:** Mockの`handed_end`が非zero window時だけpayload EOBのexclusive end + 1 sampleを返す（`crates/ezsdr-mock-radio/src/lib.rs:315`）。Mock/FakeDeviceのexact-end late、次sample on-time対は`mock_channel.rs:671`と`tx.rs:2616`。UHDのone-sample bufferはpayload EOBで送る（`tx.rs:431`）。通常のdevice-lead boundがその後のburstをlateにするため、UHD側に追加gapは足していない。
- **F2 — repeated Stop:** MockのStop cutにopen/held/既存handover frontを含め、繰返しStopで既handover分を取り消さない（`crates/ezsdr-mock-radio/src/lib.rs:1171-1192`）。recordとloopbackを確認する`mr_25_repeated_stop_preserves_handed_samples`（`mock_channel.rs:724`）を追加。UHDのcursor/EOBは`tx.rs:2479,2539`で確認。
- **F3 — partial/failed send recovery:** `abandon(device_open, handed)`が既に進んだ`open.next`またはcontinuation cursorへ、受領済み数（`Err`は保守的なattempted数）を足してfrontを記録する（`tx.rs:584-599`）。short-send/failed-sendとcontinuation-tail系列は`tx.rs:4631,4693`; FakeDeviceのaccepted samples、EOB paddingと保守的cursorを別々にassertする（`tx.rs:4733-4742`）。
- **F4 / O1 — spec test citations:** `design/18-uhd-radio.md:239-250`で既存UR-21/UR-23テスト名と説明を復元し、新規名を追加。UR-22本文が変わっていないためChanges行からUR-22を除外し、UR-23のINFERRED注記は保持（Changes: `design/18-uhd-radio.md:682`）。design/07・09の既存test名も維持。
- **O2 — duplicate `TIME_ERROR`:** 再評価時は既にlate判定済みのburstに再度eventを出さない（`tx.rs:653-656`）。controlled-clock/FakeDevice test `ur_21_a_send_asap_move_is_reported_once_across_handovers`（`tx.rs:1954`）で一回だけをassert。

### Review 4

- **P1 tail二重計上 — 却下。** `open.next`はheld tailの後のexclusive cursor。`ur_23_a_continuation_tail_is_counted_at_recovery`（`tx.rs:4693-4760`）は受け取ったpayload/paddingと保守的recovery cursorを分けて検証し、tailが二重に受領済みsampleへ加算される挙動を示さなかった。P665/P666もkilled。
- **P2 Mockのheld-cut continuation後にclosed frontを残す — 修正。** `lib.rs:1854`の境界を`mr_17_held_cut_continuation_does_not_remember_a_closed_front`（`lib.rs:2820`）とP668で固定。
- **P2 late-policy写像の重複 — 修正。** 共通判定を`ezsdr_radio::timeline::late_policy_front_outcome`へ集約（`timeline.rs:823-837`）、MockとUHDから利用（`mock-radio/src/lib.rs:1691`, `tx.rs:634-636`）。policy/boundary testは`timeline.rs:153`、P669 killed。
- **P2 異なるSampleClockのtick比較 — 修正。** Mock/UHDのcold-change testsはSampleClock値を直接比較せず、host instantまたは同clock内でassert（`mock_radio.rs:1148`, `tx.rs:2040`）。
- **P3 W=0 UHD — 却下。** UHDで使うx3xx profileはdevice leadを覆うwindowを要求し、W=0はそのprofile経路にない（`profile.rs:38`）。
- **P3 unused `closed`引数 — 修正。** 使用されない引数を削除。現在のMock/UHDに`closed: bool`はない。
- **P3 test-planの名前ずれ — 未変更。** 名前の対応はreportへ記録。orchestrator所有の`test-plan.md`は変更しない。
- **P3 UR-23 ChangesのUR-22 — 修正。** UR-22本文に変更がないためChanges行から除外。INFERRED注記は維持。
- **P3 preempt/TIME_ERROR規則 — 反映済み。** UR-21はlate済み`send_asap`の再評価で二重eventを出さないと規定（`design/18-uhd-radio.md:238-240`）。

### Review 5

Reviewは1回実行済み（`review-5.log`）。ログ自身がtreeの変更中に読んだと述べ、別の`ur_23_a_continuation_tail_is_counted_at_recovery` FAILED出力も含む。再レビューはせず、現在のtreeのテスト・workspace logとmutation結果で該当点を再確認した。

- **P1 stale mutation anchors P649/P650/P658 — 修正。** P649を共有timeline helper（`plan/maintenance/tools/mutations.json:9993`）へ、P650を`front_late_by`変換（同:10003）、P658をUHDのhelper呼び出し（同:10083）へretarget。3行とも現在の`old`は一度一致し、targeted remote mutationでkilled。
- **P2 `preempt`の`if h < bound`に専用mutation rowがない — open。** 該当sequenceを含む既存testはあるが、この条件を単独で殺すrowは今回追加していない。fix1bのtargeted row範囲外としてmutation coverageの残件に記録。
- **P2 test-planに4系列がない — 未変更。** `ur_23_abandoned_send_preserves_the_accepted_cursor`、tail recovery、`mr_25_repeated_stop_preserves_handed_samples`、`rm_15_preemption_front_applies_each_late_policy`は実装/testに存在する。計画ファイルはorchestrator所有のため編集しない。
- **P2 per-stream testでclosed frontを試していない — 未変更。** `front`はstreamごとの`Tx`状態で、既存UHD/Mock isolation tests（`tx.rs:1996`, `mock_radio.rs:1126`）が同じstream-local stateを検査する。closed-front専用caseは追加していない。
- **P2 Mock Stopのfinite-tail safe boundary — open/risk。** MockはStop後frontを`open.next + 2`で保存する（`mock-radio/src/lib.rs:1190`）。reviewのpayload EOB後`+1`との差はINFERREDで、finite burstをtail保持中にStopする一致系列をまだ試していない。parityへの影響は今後そのsequenceで確認する。
- **P3 `LatePolicy::outcome`との重複 — 却下。** 現在のKernel APIには`LatePolicy::outcome`はなく、`LatePolicy::decide`のみ（`crates/ezsdr-kernel/src/stream/burst.rs:93-137`）。Kernel APIを増やさず、front mappingをVocabulary timelineに置く裁定を維持。
- **P3 `preempt`のOnTime armは到達不能かもしれない — 維持。** 到達不能はreview側の推論で未証明。`LatePolicy::decide`結果の扱いを残す。
- **P3 profile testがtest planにない — 維持。** `tx_in_flight_window_covers_device_lead`（`profile.rs:38`）はUHD profileのW≥device-lead前提を固定する。
- **P3 DA-1形式への懸念 — 却下。** MR-17は1規則とTest行で、rationale/historyを規則本文に加えていない。`rules.py --base HEAD`は0 problems。
- **P3 burst完了後のtestがfront保持を証明しない — 却下。** `ur_21_a_target_after_a_completed_burst_is_on_time`とMock対応testは、safe restart後にon-timeであることを確認し、front保持のmutation killを主張しない（test-plan §7）。
- **P3 review logのFAILED断片 — 確認。** `review-5.log`内の断片は現行の判定として採用せず、tail/abandon test各20/20とworkspace test 1,742 passedの専用ログを採用した。review自体は再実行していない。

### Checks

- tail recovery、abandoned-send cursor、O2 re-evaluation、Mock held-cut front、shared late-policy mapping：各20/20、全てpass（`fix1b-*-20x.log`）。
- F1 Mock/UHD one-sample restart testsとF2 repeated-Stop Mock testも各20/20（`fix1-final-20x.log`、240 focused passes、0 failures）。
- `cargo test --workspace`: 1,742 passed, 0 failed (`fix1b-workspace.log`).
- `cargo clippy --workspace --all-targets -- -D warnings`: pass (`fix1b-clippy-final.log`).
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: 0 problems (`fix1b-rules-final.log`).
- `git diff --check`: pass (`fix1b-diff-check.log`).
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: pass (`fix1b-schema-freeze-final.log`).
- Remote targeted mutations P649, P650, P658, P665, P666, P668, P669: **7/7 killed** (`fix1b-target-mutations-final.log`). Earlier targeted rows P631, P657, P661–P667 are recorded killed in `fix1-mutations.log` / `fix1-final-mutations.log`. `old` anchors across maintenance, phase3 and phase7 mutation lists each match exactly once.
- Full mutation lists, Python suites, `--features uhd` clippy and hardware bench were not run in this fix1 gate.

### 残り

Review 5 P2 items remain: the dedicated `if h < bound` mutation, extra test-plan rows (owner decision), closed-front stream-isolation case, and the inferred Mock finite-tail Stop boundary. No schema version, changelog, Kernel API, commit, or stage action was added.


## 修正 2 回目（`fix2-report.md`）

STATUS: done

### G1 — Mock Stop payload cut

- `crates/ezsdr-mock-radio/src/lib.rs:236-237,1174-1199,1383-1407` now keeps the Stop payload cut (`stop_cut`) separate from the safe restart front. The cut uses the handed payload end and prior Stop cut; the safe front only constrains later bookings.
- `crates/ezsdr-mock-radio/tests/mock_channel.rs:671-696` asserts one record for A, cancellation of B, radiation through A's final handed sample, and silence at the safe-gap sample. `mr_25_stop_does_not_radiate_unhanded_window_edge_payload`: 20/20.
- FakeDevice counterpart `crates/ezsdr-radio-uhd/src/provider/tx.rs:2587-2614` asserts A's buffer stays open for the held edge burst, Stop sends the empty-EOB padding sample, and B's payload is absent. `ur_23_stop_does_not_send_unhanded_window_edge_payload`: 20/20.
- F2 remains covered by `mr_25_repeated_stop_preserves_handed_samples` (`crates/ezsdr-mock-radio/tests/mock_channel.rs:752-784`): 20/20.
- `design/09-mock-radio.md:182-184` states the distinct payload and restart bounds and cites the regression tests. The unrelated `ke_03_a_decision_after_the_stop_is_not_an_abort` citation was removed from this MR-25 list to keep the line within the rules check's 1,500-character limit.
- The FakeDevice test is cited in `design/18-uhd-radio.md:248-251`.

### G2 — duplicate SendAsap report

- `crates/ezsdr-radio-uhd/src/provider/tx.rs:617-621,659,714-717` shares `report_send_asap` between dispatch and preemption, suppressing a second event after the burst has already been marked late.
- `ur_21_a_preempted_send_asap_move_keeps_one_error_and_its_requested_target` (`crates/ezsdr-radio-uhd/src/provider/tx.rs:2000-2040`): 20/20. It asserts one `send_asap` `TIME_ERROR` with target 1,000, preserves `requested_target` 1,000 after moving B to 13,000, observes B's first continuation sample at 13,000, and observes no second error.
- `design/18-uhd-radio.md:238-240` describes the no-duplicate rule and cites the regression test.

### Checks and mutations

- `cargo test --workspace --no-fail-fast`: passed. The first run exposed stale FakeDevice EOB/padding expectations in the counterpart test; after correcting them, the workspace run passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: passed (0 problems).
- `git diff --check`: passed.
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: passed.
- Targeted remote mutations: P645, P659, P667, P670 and P671 all killed. P667 and P670 initially had invalid mutation replacements and returned COMPILE ERROR; their replacements were corrected, then both rows were rerun and killed. Rows are in `plan/maintenance/tools/mutations.json:9953,10093,10173,10203,10213`.

### Remaining

- The independent Sol review is pending; no self-review was run, as instructed. No hardware or Python suite was run for fix2.
- Changes remain uncommitted and unstaged.


## 修正 3 回目（判断 D5）（`fix3-report.md`）

STATUS: done

### Design note 40 — fix 3 (D5)

### Implementation

- `crates/ezsdr-mock-radio/src/lib.rs`: for nonzero `tx_in_flight_ns`, bookings now use the maximum earlier-received handed payload end plus two samples. A normally closed burst and a Stop use the same two-sample reservation margin. `W = 0` still creates no front; cold change and loss still cut at their instant.
- Stop keeps the payload cut separate: `max(at_stop, handed payload end, previous Stop cut)`. It never adds the restart margin to radiated payload. The post-Stop reservation front is `cut + 2` for nonzero windows.
- Removed the Mock-only one-sample `end + 1` exception, the closed-burst `end + 1` front, the held-cut-continuation no-front exception, and Stop's branch-specific safe-front calculation. The local `safe_front` tuple value was removed. No persistent field was removed: `front` and `stop_cut` remain separate because they represent the booking bound and payload cut; `handed_end` remains as the common payload-end reducer.
- `crates/ezsdr-mock-radio/tests/mock_radio.rs` and `mock_channel.rs`: updated the boundary and Stop regressions below. `crates/ezsdr-radio-uhd/src/provider/tx.rs` changed only in its FakeDevice test; no UHD runtime code changed.

### Changed expectations and regressions

| Test / case | Before | After |
|---|---|---|
| `mr_17_x3xx_tx_preemption_boundaries` | First safe repeat restart at `H` | `H` and `H+1` late; first safe restart at `H+2` (`2,010,002`) |
| `mr_17_a_finite_burst_front_stops_at_its_end` | Safe start at exclusive end `+1` (`2,008,001`) | Exclusive end `+2` (`2,008,002`); `late_by_ns` 3,001,000 → 3,002,000 |
| `mr_17_a_one_sample_burst_uses_the_safe_sample` | Safe start at `2,004,002` | `2,004,001` and `2,004,002` late; `2,004,003` on time |
| held-cut continuation unit case | No remembered closed front | `mr_17_held_cut_continuation_keeps_the_uniform_front`: cut 30 retains front 32 |
| x3xx move in `mr_16_burst_refusals` | Moved target `2,003,002`; late duration 3,002,000 ns | Moved target `2,003,004`; late duration 3,004,000 ns |
| `mr_25_stop_does_not_radiate_unhanded_window_edge_payload` (H1) | Verified the payload cut, but no C at `H+1` / `H+2` | Payload cut remains `H=10,000`; C at 10,001 is late, C at 10,002 is on time. Loopback still has no payload at the cut or the late C's sample; on-time C radiates at 10,047 after channel delay. |
| `ur_23_stop_does_not_send_unhanded_window_edge_payload` | FakeDevice padding and absence of unhanded B | Also asserts C at `H+1` late and `H+2` on time, including the empty-EOB zero sample, timed SOB, and payload sample |

The Mock H1 case is B at 10,000, one-sample A at 9,999, Stop, then C at 10,001 and 10,002. The FakeDevice pair asserts the same boundary: Stop's empty EOB pads at H, H+1 is rejected, and H+2 starts a new device burst.

### Specifications

- `design/09-mock-radio.md`: MR-17 states the uniform two-sample Mock frontier, the W=0 exception, and the Provider-specific equality boundary; MR-25 keeps the payload cut separate from the retained reservation front.
- `design/18-uhd-radio.md`: UR-23 clarifies that a continuation reserved at the next sample keeps the device burst open without EOB, and cancellation before hand-over closes it with an empty EOB. UHD's strict payload window and runtime behavior are unchanged.
- `design/07-radio-model.md` RM-15 was left unchanged, as requested.

### Mutations

- Removed obsolete rows P661 and P668; IDs were not reused. Retargeted stale source anchors P645, P646, P652, P662 and P670 to the current Stop/front implementation. Added P672 (+1 frontier), P673 (+0 frontier) and P674 (Stop front +1).
- Self-review found P627's existing window mutation used a test that could not distinguish the profile value. Retargeted it to `mr_25_a_stopped_burst_transmits_every_sample_before_the_stop` (`mock_channel`); remote mutation killed it.
- Final selected remote results: **9/9 killed** — P627, P645, P646, P652, P662, P670, P672, P673 and P674. P674 first survived with the H1 test because normal burst closure had already set the same front; it was retargeted to the repeat-Stop regression, and the final run killed it. An intermediate P674 run had `BASELINE FAILED` due to a wrong `test_bin`; the metadata was corrected before the successful run.
- All 1,407 `old` anchors across the six mutation lists match exactly once; IDs are unique within each list. P661/P668 are absent from the maintenance list.

### Verification

- Seven focused tests, each run 20 times (140/140 passed): `mr_17_held_cut_continuation_keeps_the_uniform_front`, `mr_17_x3xx_tx_preemption_boundaries`, `mr_17_a_finite_burst_front_stops_at_its_end`, `mr_17_a_one_sample_burst_uses_the_safe_sample`, `mr_25_stop_does_not_radiate_unhanded_window_edge_payload`, `ur_23_stop_does_not_send_unhanded_window_edge_payload`, and `mr_16_burst_refusals`.
- The first focused compile caught an ambiguous integer type and a shadowed `records` helper in the new test code; both were corrected before the final 20-run set.
- `cargo test --workspace`: passed, 1,745 passed and 0 failed. An earlier run caught the stale `mr_16_burst_refusals` x3xx target; its expectation was updated to `2,003,004` and the test then passed 20/20 and in the final workspace run.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: passed, 0 problems.
- `git diff --check`: passed.
- `cargo test -p ezsdr-radio --test radio_model rm_20_schema_freeze`: passed.

### Self-review

One read-only review was run (`review-6.log`); it reported one P1, four P2s and four P3s. It was not rerun, per the brief.

- **P1 — P627 filter could not kill the profile-window mutant:** fixed by retargeting to the ideal-profile Stop test; the remote mutant was killed.
- **P2 — UHD `send`'s `.max(1)` / release-build guard:** not changed. This predates D5, the caller guards currently make the zero-count path unreachable, and the brief explicitly says not to change UHD runtime. The reviewer noted the guard interaction was an inference, not a demonstrated failure; leave it for separate review.
- **P2 — internal-state assertions in the Mock unit tests:** retained as unit-level checks of the exact front state. The H1 integration regression additionally observes late/on-time actions and loopback samples; existing cold-change/loss integration tests cover their external effects.
- **P2 — abandonment/recovery cases absent from `test-plan.md`:** not changed; those paths predate fix 3, were not modified here, and the plan file is orchestrator-owned.
- **P2 — RM-15 / MR-17 boundary wording:** fixed. MR-17 now states that a Mock's two-sample bound may be later than the Provider's actual cursor and equality applies at each Module's own bound.
- **P3 — design note 40 §8 has older expectations:** not edited because the design note is orchestrator-owned and the fix3 brief says not to modify it.
- **P3 — `mr_17_a_one_sample_burst_uses_the_safe_sample` name:** retained; it still tests the first safe sample, now at the uniform two-sample boundary.
- **P3 — UR-23 continuation wording:** clarified as described above.
- **P3 — unrelated P509 / Phase 7 U36 mutation naming:** not changed; outside D5.

### Remaining

- Full mutation lists, Python suites and hardware bench were not run; only the specified selected mutations and Rust gates were run.
- No staging, commit or push was performed. Changes remain unstaged and uncommitted.


## 修正 4 回目：gate の mutation の生き残り（`fix4-report.md`）

STATUS: done


### P639 — ideal Mock TX front

- **原因:** `remember_tx_front`'s zero-window guard was redundant. At `W = 0`, `tx_handed_front` returns `front = None`; the remembered `tx.front` is only read in the `W > 0` branch (`crates/ezsdr-mock-radio/src/lib.rs:1402-1408`). The guard was removed (`:1411-1414`).
- **対応:** Removed the guard and retired P639 from `plan/maintenance/tools/mutations.json`; its ID is not reused.
- **Mutation:** P639 was not run because its mutation row was removed as equivalent. The unchanged `mr_17_ideal_closed_finite_burst_has_no_extra_gap` test passed 20/20.

### B18 — held tail after a short UHD send

- **原因:** D3's moving in-flight `window_cut` limits each handoff (`crates/ezsdr-radio-uhd/src/provider/tx.rs:301-302, 328-333`). The old `ShortSend(1)` case hit a non-EOB send, not the held-tail path; the FakeDevice trace showed its second send as `eob=false`. B18 only changes behavior when `send` has saved a held tail (`tx.rs:431-459`).
- **対応:** Added a 6,000-sample case with `ShortSend(30)`, targeting the final payload buffer (the 31st device send), where the last sample is held inside the window (`crates/ezsdr-radio-uhd/tests/fake.rs:2837-2865`). The existing assertions still require an empty EOB after what was accepted and no later payload.
- **Mutation:** The first added case (`ShortSend(2)`) still survived; after targeting send 30, final results were B18 killed, and the related B19 and B25 rows killed. Test passed 20/20.

### E21 — responder rearm across blocks

- **原因:** HEAD's test used one 1,000-sample PING at 11,500, straddling the 12,000 block boundary (`HEAD:crates/ezsdr-acceptance/tests/reactive.rs:286`). Resetting rearm at that boundary would request another PONG at 19,000, overlapping the first PONG at 18,546 by 46 samples. With the new Mock front, the prior burst's end plus two samples (19,048) makes that second request late and drop-and-flag discards it (`crates/ezsdr-mock-radio/src/lib.rs:1402-1408, 1692-1717`), so the test still observed one response. The responder's state is processed across blocks at `crates/ezsdr-acceptance/src/responder.rs:141-163`.
- **対応:** The test now sends PINGs at 11,000 and 12,050: the first crosses the boundary, and the second follows only 50 quiet samples in the next block. It asserts both PINGs were sent and exactly one PONG, at 18,046 (`crates/ezsdr-acceptance/tests/reactive.rs:283-288`). If rearm resets, the second PONG at 19,096 is separate and observable.
- **Mutation:** E21 killed; E18 (the other row filtered to this test) killed. Test passed 20/20.

### Checks

- Remote mutation run (`maintenance,phase5,phase7`, sim02/sim03): B18, B19, B25, E18, E21 all killed (5/5). P639 was retired, not run.
- `cargo test --workspace`: passed, 1,745 tests across 66 harnesses; `rm_20_schema_freeze` passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `rules.py --base HEAD`: 0 problems. `git diff --check`: passed.
- Two initial focused Cargo checks were launched together; one waited on Cargo's build-directory lock. All later Cargo commands ran sequentially.
- Self-review was skipped as instructed. No hardware run was performed.

