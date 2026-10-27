# D9a：orchestrator 側の独立レビュー（Opus，AGENTS.md §9 step 6）

対象：HEAD `660213b` の上の未 commit の差分（D9a：R1，R5，S2・F3′ の Ceiling，P4〜P7，`tune_auto`）．読むだけ（tree は変えていない）．
走らせたもの（`CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review`）：

- `cargo test -p ezsdr-radio-uhd --lib -- ur_24_ ur_25_ ur_29_a_deferred ur_33_`：190 passed
- `--test fake` の D9a の 4 本と変えた 2 本：6 passed
- `rules.py --base HEAD`：0．新しい・変えた mutation 行 30 本（P965–P989，J326，P214，P215，P219，P409）の `old` は tree で各 1 回
- 既知の ur_25／ur_22 の flaky は今回は出ていない．出ても D9a のせいにはしない（mi-3 は D9a が新しく入れた別の時刻依存）

**判定：major 1，minor 5，note 10．** R1 は brief どおり，道は cover 1 つ，宛先 1 つ，印 1 つ．次は変わらない：記録，行，`live`，`handed_at`，slot，floor，`LATE_COMMAND`，settle，loss，利得，送信（VERIFIED）．lock の順と from-none の印は安全（VERIFIED）．
major は位相．T0 より前の受信周波数を untimed で当てるので，`random_unless_timed_tune` の保証が消える．そのため Mock の約束が実機より強くなる．owner の判断が要る．

---

## Major

### MA-1 T0 より前の受信周波数を untimed で当てると LO の位相の保証が消え，Mock が実機より強い約束をする

- **場所**：`control.rs:745-746`．`settings` の無い宛先（＝ stream の最初の segment）を `Reach::Untimed` にする．試験は `ur_24_a_frequency_before_t0_for_the_first_segment_is_applied_untimed`（`control.rs:6171`）．
- **系列**：
  - x3xx の `startup_latency_ns` は 2 s（`x3xx.rs:396`）．`start_up` ＝ arm＋2 s（`mod.rs:572`）で，T0 ≥ `start_up`（`mod.rs:631-640`）．uhd-control は `start` で動き出すので（`mod.rs:696-707`），T0 の前に最大で約 2 s の窓がある．
  - Session は Running に入ると action が流れる（`pipeline.rs:450-471`；ここは INFERRED）．
  - connect の直後に 2 channel の周波数を設定すると，`at` 無しでも `e = now + 2 ms < T0` になる．すると宛先は最初の segment になり，release が untimed の `set_rx_freq` を出す．
- **HEAD との違い**：
  - HEAD は timed だった．LO は radio の queue で `e` に同期して変わり，DSP は T0 の最初の sample で当たる．
  - untimed の tune では fractional-N の LO の位相がばらばらになる．OBX で timed は 0.0–0.1°，untimed は 142° に散った（`plan/phase7/bench-results.md:513`，VERIFIED）．
- **Mock との違い**：
  - MockRadio の MR-34（`design/09-mock-radio.md:212-214`，`ezsdr-mock-radio/src/lib.rs:2166-2176`，`:1025-1036`）では，周波数 update は timed tune で，最初の segment は origin で untimed tune を取らない．
  - だから Mock では T0 で coherent になり，MR-27 は `timed_tune, valid: true` と記録する．実機は R1 で位相が乱れる．
  - これは「Mock enforces the hardware envelope」（AGENTS.md §3）と逆向きの食い違いになる．
  - 設計 33 D9 規則 4（`33-channels-and-streams.md:178`）と設計メモ 42 §2-7 は「見える振る舞いは変わらない」と書くが，位相についてはこれが成り立たない．D9 の文書（メモ 42，調査，Sol の検証，試験計画，Sol の計画レビュー）は位相を一度も扱っていない（`grep -i 'phase|位相|sync'` で 0 件）．
- **ほかの untimed の道は一致する**：configure 済みの宛先に向けた gap の値も，宛先の configure が運ぶ値も，Mock では segment の origin が untimed tune なので incoherent になり，実機と同じ．食い違うのは最初の segment だけ．
- **根拠**：機構，Mock の規則，bench は VERIFIED．Session がこの窓で周波数を設定する頻度は INFERRED．
- **直し（owner の判断）**：
  - **推奨 (A)**：コードは今のまま．
    - UR-24 に Ceiling を足す：「最初の sample より前が instant の受信周波数は untimed で当たる．`random_unless_timed_tune` の channel の LO 位相は，走行中の segment の中で次に timed retune するまで分からない．coherent に始めるには T0 以後に retune する」．
    - `KNOWN_LIMITATIONS.md` に 1 項目足す．
    - MR-34 を同じ規則にする改定を，Phase 8 の parity の項目として起こす．
  - **退けた案**：
    - (B) 最初の segment を `Reach::Timed` に戻す．1 行で HEAD・Mock と一致するが，T0 前の約 2 s に sweep する Session が DDC を 33 個目で詰まらせる F1-B が戻る．
    - (C) この場合だけ LO を timed，DSP を untimed に分ける．設計メモ 39 §3 で owner が退けた分割を蘇らせることになり，UHD の tune の道が 1 本増える．

---

## Minor

### mi-1 UR-24 の文が Untimed の道と矛盾する

- **場所**：`design/18-uhd-radio.md:262`．
- **矛盾**：「Otherwise the release makes no device call for the frequency」と書いた同じ段落で，「…by the release itself, as an untimed `apply`」とも書いている．Untimed の道では device を呼ぶ．
- **原因**：self-review で文を直したときに入った．
- **直し**：「…the release gives the device no timed command for the frequency…」．

### mi-2 書き換えた既存試験が，本番では起きない順の untimed 書き込みを正しいものとして固める

- **場所**：`control.rs:2804-2849` の `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`．
- **試験の順**：
  1. switch を時刻 0 で `plan[1].configure`（`:2827`）
  2. release は 298 ms
  3. 旧 segment の cut は 300 ms
- **何を固めるか**：新しい assert（`:2843`）は「298 ms に untimed で 1 回」．旧 segment の最後の 2 ms がまだ流れている間に DDC と LO を書き換えるのを，正しいとして固めている．
- **本番では起きない**：owner は前の segment を `finish` してから次を configure する（`rx.rs:205-212`，`:237-238`，`:286-306`）．だから 298 ms に印は立たず，Deferred になる．
- **なぜ問題か**：R1 の Untimed の安全は「configure 済みの宛先 ⇒ その origin まで誰もその chain を流さない」という不変条件に依る．コードはこれを検査しない．この試験は，その条件を破った状態を正しいと認めてしまう．
- **直し**：configure を 300 ms より後に動かす．assert は「298 ms の release は装置を呼ばない」「configure が 2.1e9 を運ぶ」（本番の順）．
- 姉妹の `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`（release 318 ms ＞ cut 300 ms）は到達できる順なので，そのままでよい．

### mi-3 `ur_24_a_released_command_is_not_recalled_by_stop` が時刻に依るようになった

- **場所**：`tests/fake.rs:2516`．key は周波数のまま．
- **今の順**：
  - update（20 ms，hardware_timed）は start でまとめて dispatch される．
  - Stop（19.5 ms）は agenda で時刻に dispatch される（`stepping.rs:434-460`）．受信の Stop は booking の時刻で cut する（`control.rs:431-438`）．
  - release（約 17 ms）が Stop の booking より先なので，今は Timed で通る．
- **崩れ方**：uhd-control が 2.5 ms 以上遅れると release が Stop の後になり，Deferred になる．`apply rx 0 rate=- freq=2300000000` が 0 になって失敗する．HEAD は Stop と関係なく timed だった．
- **直し**：計画 §4 のとおり key を gain にする．この試験の意図は記録と recall で，R1 ではない．
- mutation の filter ではないので，hidden survivor にはならない．

### mi-4 M4a・M4b に mutation の行が無い

- **何が抜けているか**：計画 §8 の「`give` が projection の lock を取らずに印を読む → M4b」の行が無い．報告の「行にしていない 4 件」にも挙がっていない．
- **行の案**：
  - old：`control.rs:759` の `let updates: Vec<_> = projections.iter().map(|cold| cold.updates.lock()).collect();`（tree で 1 回．settle の方は `let mut updates`）
  - new：`let updates: Vec<()> = Vec::new(); let _ = &projections;`
  - filter：`ur_24_a_release_waits_for_a_configure_that_carried_its_booking`．M4b の「300 ms 待つ」の assert が殺す．

### mi-5 3d（untimed の書き込みが拒否される場合）は安く書ける

- **今の穴**：Untimed の結果を捨てる mutant（`Reach::Untimed => { let _ = …; None }`）を殺す試験が無い．分岐は Timed と共有しているので必須ではない．
- **書き方**：`rig_with(FakeConfig { faults: vec![RxApplyFails], clock: Manual, command_queue: 64, .. })` で M6 と同じ並べ方にする（`test_support.rs:31`）．assert は次の 3 つ：
  - `device_failed` が 1
  - 行が無い
  - projection に残る
- **前提**：rig が受信の untimed `apply` をそれより前に出していないこと（確かめてから）．

---

## Note

1. **from-none の印を `updates` の外で立てる件**（`control.rs:1099-1105`）：安全（VERIFIED）．
   - `segment_config` は `:1106` の insert まで他の thread から見えない．store は insert より先で，どちらも `streams` の lock の下にある．booking も release も同じ uhd-control の thread で走る．
   - from-none の configure は `segment_config.updates` を読まない（`candidate`＋`held ≤ e`，`:1054-1067`）．だから UR-25 の「snapshot と同じ lock」は from-none には言葉が緩い．
   - 潜在する差：from-none が選ぶのは `≤ e`（cold の instant）で，`≤ origin`（≥ ready＋lead）ではない．(e, origin] の値は release の Untimed が運ぶので今は問題ない．
   - (e, origin] の値が enable より前に release 済みのことは起きない（INFERRED）．channel が live なのに segment が無いのは stream が Stop された時だけで，Stop された stream への cold は segment を始めない（`timeline.rs:945-967`）．
2. **lock の順**：`give` は全 RX projection の `updates` を取ってから `streams`（`reach`）を取る．`settle`→`targets` と同じ順．`book` と `configure` は line が空いていることを assert する（`control.rs:128,149,758,810`）．逆順は無い（VERIFIED）．
3. **`reach` の中で RX では死んでいる条件が 2 つある**：
   - `!planned.refused`：RX の plan には拒否された item の segment が無い（`timeline.rs:945-967`）．
   - 長さ 0 の除外（`:743`）：RX の長さ 0 の segment は plan から外される（`timeline.rs:998-1004`．M5a の注記どおり）．
   - どちらも `targets`・`needed_before` に合わせたもので，mutant は殺せない．消すか対称のために残すかは orchestrator の判断．
4. **M5b は本番の状態ではない**：
   - owner が動くと，stream 1 は戻りの segment に `Owner::Configure` で入る．streamer がまだ {1} で開いているからで，`owner_configure` は `needed_before` を `Open` のときしか見ない（`core.rs:1245-1251`）．
   - だから印は最初の segment の直後（約 100 ms）に立つ．試験の「198.5 ms に未 configure」とは違い，release は untimed で当てる．
   - これが安全なのは，その間 stream 0 の中間の segment が channel 1 を開けないから．ただしそれ自体が，1 channel の stream が往復する move で行き先を飢えさせる既存の穴に見える（INFERRED，D9a の範囲外）．
   - 「configure 済み ⇒ 他は流さない」に頼る前に，Direct で owner まで動く往復の試験で両方を確かめるとよい．
5. **plan が覆っても sample が流れない場合（F1 の残り）**：
   - segment の `rx_start` が拒否された場合：`rx.rs:332-335` は `device_failed` だけで，segment は plan に残る．
   - owner が飢えている場合（note 4）．
   - どちらも timed の entry が DDC に溜まる．missed start は 50 ms 後の restart で有界．KL-9 か D9b に 1 行．
6. **Untimed の release は uhd-control の thread で，全 RX projection の lock を持ったまま走る**：untimed の書き込みが radio の待っている timed command の後ろで待つなら（F6，INFERRED），その pass の後ろの command は遅れて届く．queue が空なら untimed tune は呼び出しから約 185 µs で始まる（`phase7/bench-results.md:437`）．P4 で計る．
7. **Fake の細部**：
   - `set_subdev_spec` は受信の streamer を `end_rx_flow` 無しで消す（`device.rs:1081`）．spec を変えるのは prepare だけ（`mod.rs:287`）なので到達しない．
   - fake の DDC が詰まるのは周波数の書き込みだけ．X300 ではその DDC への ctrlport はすべて止まる（rate の書き込みも．`rfnoc_block_ddc.v:298-319` からの INFERRED）．configure は必ず周波数を含むので，本番の道は隠れない．
   - fake の flow は受信の overflow を無視して取り出し続ける．X300 は overflow で radio が止まる（INFERRED）．
8. **Fake の速さ**：
   - `freq_constant`（`device.rs:554-563`）は block ごと・channel ごとに，増える一方の log を全部読む．
   - log は tick の順なので，`log.iter().rev().take_while(|(t, ..)| *t > from).any(|(t, ..)| *t <= to)` で済む．
   - 200 MS/s の rehearsal は 31–59 s で，120 s の watchdog まで余裕は約 2 倍しかない．
9. **probe**：
   - `tune_auto`（`uhd.rs:555-590`）は `tune_dsp` と request と返す値の他は同じ．request を取る private helper 1 つにまとまる．
   - P4 の C 条件は B 条件の AUTO tune が残した LO の上で走る（rf NONE の `tune_dsp` は LO を戻さない）．条件ごとに `tune_auto(Rx, 0, 1e9, None)` で戻すと，C の基準が A と同じになる．
   - P4-B は DSP が変わる δ を見つけるまでに，e の timed entry を最大 4 個積む（8 個以内，出力に残る）．
10. **Gate**：
    - `design/18` は `KNOWN_LIMITATIONS.md` を引くが，tree に無い（案は作業 dir にある）．
    - KL-9 の「To be fixed in stage D9a」を「経路 A・B は直した，系列 6 は Ceiling（P4 待ち）」に，KL-11 (a) を「`ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value` で固めた」に直す．
    - README の link と AGENTS.md の 1 行を同じ commit に入れる（owner 2026-10-27）．
    - メモ 42 §5 にまだ UR-22 が挙がっている．

---

## 書かれなかった試験の判定

| 系列 | 判定 |
|---|---|
| 2c | 不要．`ur_24_a_frequency_for_a_far_planned_segment_is_deferred_to_its_configure` が cold の rate 変更を通り，configure の `rate=2000000` と最後の値を assert している |
| 3d | 不要だが安い（mi-5） |
| 12b | 不要．12d の明示した行の方が強い |
| 15，`handed_at > S` | 不要．`handed_at = Some(now)` はどの道でも reach の判断より前にある（`control.rs:771-775`）．R1 だけの道は無い |
| 14 | 不要．`settle` が `held` と `released_at` を空にする（`control.rs:805-839`） |
| 印を lock の外に出す行 | 受け入れる．drop と store の間に hook が無く，`probe("configured")` はどちらより後．lock の中の store はコードの注記と M3・M4 で固まる |
| 7（2 channel） | control で書かれた（P989） |

## 変えてはいけない振る舞い（確かめた）

- **呼び出しの道**：
  - `give` を呼ぶのは `release` だけ（`control.rs:656`）．
  - 本番の device `apply` は 2 か所：`core.rs:1087`（configure，untimed）と `control.rs:777-778`．
- **印**：立てるのは `control.rs:166`（`updates` の中）と `:1103`（from-none）．読むのは `:746` だけ．
- **Deferred の記録**：Deferred も同じコード（`:771-794`）で `Handed::Given(handed_at)` を返す．だから行，`live`，settle は同じ．`booked` も変わらない（`:596-611`）．
- **利得と送信**：`reach` は `Dir::Rx` かつ周波数のときだけ（`:775`）．
- **拒否と loss**：
  - 拒否と lost は同じ枝を通る（`:781-790`）．
  - loss の確認は reach より前（`:762-767`，試験 12c）．
- **VH-8**：gain だけを見るので，R1 も MA-1 も検査しない．

## 設計の意図（確かめた）

- **道の数**：cover 1 つ（`:732-735`），宛先 1 つ（`:742-744`），印 1 つ（`:103`）．
- **印を立てる所が 2 つあるのは要る**：from-none は segment ができる前に configure するので，`ColdConfig::configure` を通せない．
- **規則の二重化**：core.rs・rx.rs との二重化は，inline の長さ 0 の判定（`core.rs:631`，`tx.rs:793`）だけ．
- **古い印**：消されないが，読むのは今の plan の宛先を通してだけ．長さ 0 になった segment は plan から外れるので，その印には届かない．
- **残る前提**：「configure 済み ⇒ origin まで chain は流れない」は，owner が `finish` の後に configure することと，graph が 2 つ目の streamer を拒むことで成り立っている．コードでは検査しない（mi-2，note 4）．

## Fake と実機

- **事実と合う**：H1（radio rate の tick に 1 つ），H2（untimed はすぐ当たり，待つ timed が後で上書き），H4（has_time は戻らない），H7（抽象の模型と明記），H9（新しい装置は空）．
- **実機より厳しい向き**：32 は保守的（`axi_fifo` の 32 に加えて別の FIFO がある）．
- **実機より緩い向き**：note 7 の 3 点．どれも本番の道は隠さない．
- **gain に替えた試験**：regression は隠していない．radio queue の front end の意味を固め続けている．DSP の FIFO も front end ごと（`dsp_of`，`device.rs:538-544`）なので周波数も front end に付いて行くが，それを直接見る試験は無い．

## spec の文

- §2，UR-25，UR-33，設計 33 D9 の規則 1〜3 はコードと合う．
- 合わないのは 2 か所：mi-1（UR-24 の 1 文）と，MA-1（規則 4・メモ 42 §2-7 の「見える振る舞いは変わらない」）．
- spec の差分が挙げる試験名は全部，tree に 1 回だけある．
