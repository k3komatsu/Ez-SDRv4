# 1b-4d-1c：実装と修正の回の報告

## report


# 1b-4d-1c 実装の報告 — 記録だけになった release を device に届ける（形 F'）と，版の削除

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `7968d98`．裁定は `adjudication-3.md` を最優先，次に `adjudication-2.md`，`adjudication.md`，計画 `test-plan-v3.md`（§10 に実装後の追記）．commit・push・`git add` はしていない．

## 1. 実装したもの

- **`targets(dir, chan, t)`**（`crates/ezsdr-radio-uhd/src/provider/control.rs:515`）：`t` に live な channel（current keys は `channels_at`，layout は `channel_live`），無ければ origin が `t` 以後で拒否されていない segment の channel（layout はその command の channel だけ，current keys は和）．
- **`release`**（`control.rs:621`）：`targets(dir, chan, effective)` で出し先を決め，live/pending を同じ経路で `apply(..., Some(effective))`，channel ごとの `released.push`，`issued()`，行（live は `at=effective`・note なし，pending は `at=e`・`note: "no channel"`・`issued`）．空なら `config.insert` と `at=e`・note・`issued` なしの行だけ．失敗は live の枝と同じく `device_failed` で return し，`config` も行も書かず，投影の entry は残す（裁定 3 §1.2）．depth の上限は無い（裁定 3 §1.1）．
- **`book_timed` の会計**（`control.rs:582`）：current keys は `targets` の長さ（booking のとき），layout は 1 本ずつ（`held.len() + released.len()`，cost 1）．
- **版の削除**：`ColdConfig.version`/`version()`/`changed()`，`Planned::version()`，`changed_projections`，`owner_step` の版引数，`configured` の `(seq, version)` → `Option<u64>`，`configure` の戻り値 → `Result<i64, DeviceError>`，`cancel_held` の版の加算，`AtomicU64` の import．
- **test 専用の道具**：`FakeDevice::set_time`/`read_back`（`device.rs`，`#[cfg(test)]`；`base` を `now` から切り出した），`test_support::Frozen`，`ApplyFails::timed`/`timed_lost`/`attempts`，`Core::owner_configure` の probe `"configured"`．
- **`design/18-uhd-radio.md`**：UR-17，UR-23，UR-24（第 1 段落の layout，Holding and release，depth，`applied` の行，第 3 段落），UR-36（全文），UR-24/UR-29/UR-36 の `*Test:*`，§6 の 2 行，Changes 1 行（§5）．

## 2. 計画 v3 から変えたところと理由

すべて裁定 3 による．計画 v3 にも §10 として追記した．

1. **release の depth の上限を取らない**（裁定 3 §1.1，裁定 2 §1.1 の取り下げ）．S4b は作らない．booking の会計だけを残し，booking 後の `cold` change で増えた分は device の back-pressure に任せる（UR-24 に 1 文，Changes に既知の制限として記録）．実装では上限の `held` への持ち戻しを書いてから裁定に合わせて削除した（最終 tree には無い）．
2. **失敗した release は投影から除かない**（裁定 3 §1.2，裁定 2 §1.2 の取り下げ）．`device_failed` で return し，`config` も行も書かず，entry は残す．S9(e) は「entry が残る・begin が configure しない・device は旧値・行も `config` も書かない」を固定し，名前を `ur_24_a_refused_release_records_no_row_and_keeps_the_projection` にした（v3 の `..._takes_its_update_out_of_the_projection` は裁定 3 で偽になるため使わない）．S9(b) は entry が残ること，S9(c) は owner の configure が値を両 channel に当てることに直した．
3. **S10 は既存テストの拡張**（v3 のまま）：2 本目の rig で pending の segment への timed の apply を通した．
4. **S3 の時計**：`Frozen` と epoch 0 の assert を足したつもりだったが，**この回の tree には `hold` が入っていなかった**（Sol のコードレビュー major 2）．修正の回 1 で，S3 の最初の booking の前に他の系列と同じ `hold(&core, &time, &device)` を入れた（`fix1-report.md` §1 の 2）．
5. **S16〜S18 は動かさない**（裁定 2 §2.10）．大きさは 1,500 行に達していない（§7）．

## 3. テストと行（足した・変えた・消した）

`crates/ezsdr-radio-uhd/src/provider/control.rs` の `mod tests`（S1-TX だけ `tx.rs`）：

| 系列 | テスト | 行 |
|---|---|---|
| S1 | `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`（旧 `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` を置換） | `control.rs:1304` |
| S1-TX | `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment` | `tx.rs:1959` |
| S2 | `ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure` | `control.rs:2628` |
| S3 | `ur_24_a_release_during_a_configure_s_return_reaches_the_device` | `control.rs:2656` |
| S4 | `ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots`（pending 1 channel と 2 channels） | `control.rs:2702` |
| S5 | `ur_24_a_release_after_its_instant_applies_on_arrival` | `control.rs:2746` |
| S6 | `ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing` | `control.rs:2766` |
| S7 | `ur_24_a_layout_release_reaches_the_later_segment_s_channel` | `control.rs:2783` |
| S8 | `ur_24_a_release_for_a_refused_transmit_segment_applies_nothing`，`ur_24_a_release_for_a_refused_receive_change_has_no_candidate` | `control.rs:2806`, `:2828` |
| S8b | `ur_24_a_release_stays_when_its_segment_is_refused_after_it` | `control.rs:2849` |
| S9(a) | `ur_29_a_lost_device_applies_no_released_update` | `control.rs:2875` |
| S9(b) | `ur_29_a_failed_released_update_is_a_device_failure` | `control.rs:2892` |
| S9(c) | `ur_29_a_partly_applied_release_keeps_the_projection`（n=1, n=0） | `control.rs:2917` |
| S9(d) | `ur_29_a_lost_device_between_a_release_s_channels_applies_no_more` | `control.rs:2951` |
| S9(e) | `ur_24_a_refused_release_records_no_row_and_keeps_the_projection` | `control.rs:2971` |
| S10 | `ur_25_the_line_lock_is_free_inside_every_control_device_call` に第 2 の rig を追加 | `control.rs:2030`（追加部 `:2059`） |
| S12 | `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`（(a)/(b)） | `control.rs:2999` |
| S13 | `ur_24_a_direction_halted_by_a_refusal_has_no_channel`（期待を書き換え） | `control.rs:1767` |
| S14 | `ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for` | `control.rs:3053` |
| S16 | `ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant` | `control.rs:3081` |
| S17 | `ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant` | `control.rs:3108` |
| S18 | `ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant` | `control.rs:3136` |

- 書き換え・改名：`ur_24_a_cancelled_projected_update_makes_the_owner_configure_again` → `ur_24_a_cancelled_held_update_leaves_the_projection`（`control.rs:1336`）．`ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again` は版の assert を `configured == Some(by)` に，`ur_17_a_prune_keeps_the_device_record_and_the_configure_mark` は `Some(second)` に直した．
- 消したテスト：`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（S1 に置換），`ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version`（版の channel ごとの選択性だけを見ていた．`design/` に引用が無い）．
- helper：`manual_rig`/`manual_rig_events`/`layout_rig`/`layout_mode`/`device_at`/`hold`/`rows_of`/`events_of`（`control.rs`），`manual_rig`（`tx.rs`）．`device_at` は device の時刻を `at` に置き，**Core の時計も同じ時刻へ進めて両方を assert** する（裁定 3 §2）．`hold` は `Frozen` のあと epoch を 0 に決めて assert する．
- `--renames`（`rules.py` に渡した）：`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again -> ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`，`ur_24_a_cancelled_projected_update_makes_the_owner_configure_again -> ur_24_a_cancelled_held_update_leaves_the_projection`．

## 4. 確かめることの結果

1. `cargo test --workspace --no-fail-fast`：すべて成功（ezsdr-radio-uhd は lib 188，integration 228，ほか各 crate）．`cargo clippy --workspace --all-targets -- -D warnings`：警告なし（`--features uhd` はこの機械では流していない）．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`：`0 problem(s)`．`mutations.json` は全体を書き直さず，消した 10 行（J68，J71，P256，P310，P311，P318，P374，P375，P381，P393）をその場で削り，足した 20 行を末尾の書式（1 スペース）で加え，P312 の filter と `test_bin` を当て直した（`git diff --numstat` は 202/102）．
3. mutation（`remote_mutate.py`，sim02/sim03，すべて kill）：足した行 P405〜P424 の 20 行と，コードかテストが変わった行 J64，P312，P317，P368，P369，P370，P394，R14，U20，R21，J69，P262，P351．
   - 最終の 1 回（33 行，`sim02:8,sim03:8`）：**33/33 killed**．
   - 途中の結果と直し：1 回目は P408 が SURVIVED（S7 が `apply rx 0` を見ていなかった）→ S7 に assert を足して killed．P312 は base の filter が単位テストに変わり `test_bin` が `fake` のままだった → `lib` に直し，S6 を filter にして killed．修正後の再確認 15 行（P405，P409，P410，P412〜P415，P421〜P424，J64，U20，R21，J69）も **15/15 killed**．
   - 行ごと：P405 killed，P406 killed，P407 killed，P408 killed，P409 killed，P410 killed，P411 killed，P412 killed，P413 killed，P414 killed，P415 killed，P416 killed，P417 killed，P418 killed，P419 killed，P420 killed，P421 killed，P422 killed，P423 killed，P424 killed，J64 killed，P312 killed，P317 killed，P368 killed，P369 killed，P370 killed，P394 killed，R14 killed，U20 killed，R21 killed，J69 killed，P262 killed，P351 killed．
   - 全リスト，image の build，Python の suite は orchestrator の gate（この段階では流していない）．

## 5. セルフレビュー（reviewer：DeepSeek，各回で新しく起動）

**回 1**（P1 なし）：
- P2 S3：`effective == e` を device の初期時刻を決めずに assert していた → `device_at` で Core と device の両方を assert する形にした．ただし **`hold` はこの回の tree に入っていなかった（この行の「S3 にも入れた」は誤り．Sol のコードレビュー major 2）**：修正の回 1 で S3 の最初の booking の前に入れた（`fix1-report.md` §1 の 2）．
- P3 S13/S7：`at=` を見ていなかった → `at=<e>` を assert に足した．S9(e)：行と `config` を見ていなかった → 足した．S14：恒真の assert → 2 度目の enable の直後に読んだ `second != by` に変え，owner の configure の観測を残した．S1/S2/S1-TX：`configured`・owner の configure・switch の無呼び出しを足した．S4/S12(a)：順序を足した．device の epoch：`hold` に寄せた．
- P2 design：depth の文面と，3 段落目の「booking の集合」の矛盾 → 文を直した（§6）．P3：失敗の「can apply」，「later segment」の重複，「no row」「record's command」の未観測 → テストを足して解消．
- P3 コメント：`channel_live` と halted の queue のコメントを新しい規則に合わせた．

**回 2**（P1 なし）：
- P2：`released` が channel ごとに積まれることを観測するテストが無い（S4b 削除で穴）→ S12(b) に `control.released == [us(10_000); 3]` と両 channel の apply を足し，行 P424（push を loop の外へ）を足して killed を確認した．
- P3：S4 の `at=` と `depth` payload → 足した．S12(b) の channel → 足した．S3 の順序と旧値・`rx_start(O)` → 足した．`rows_of` の filter → `read_back` が null の行に変えた．S9(b) の kind → 足した．S10 の timed 性 → 足した．design の depth/失敗/UR-36 の全称 → 文を直した．

**回 3**（P1 なし，最終）：
- P2：裁定 3 §2 の「Core の時計も同じ時刻へ進めて両方を assert」が `device_at` に無い → `device_at(core, time, device, ticks)` にし，Core を進めて両方を assert する形に直した（S9(c)(e) は `hold` に寄せた）．
- P3 S12(a)：`LATE_COMMAND` の payload → `requested=9 700 µs`，`applied=10 000 µs` を足した．S4 の pending が 1 channel だけ → 2 channels の pending の block を足した（9 個目が `COMMAND_QUEUE_FULL`）．UR-24 の「at its instant」と depth の文言，UR-36 の「release が segment を持つ／持たない」の書き分け，Changes 行の UR-29 と消した 3 本目のテスト → 直した．UR-24 の Test 行の二重引用 → 消した．
- **退けた 1 件**：S8-RX が `!planned.refused` を観測しない，という指摘．`!refused` は S8-TX が観測しており，行 P407（filter は S8-TX）は killed である．RX の拒否は plan から除かれる（UR-37）ので，RX では `!refused` を観測できない．

3 回で P1 は無く，P2 はすべて直した．3 回の後に回 3 の指摘を直したので，最終 tree は回 3 のレビュー対象から少し動いている（上記の 6 件）．

## 6. 変えた `design/18` の文

- **UR-24**：layout の文（`instant it takes effect`，pending の segment の channel，refused を除く），Holding and release（`Some(the instant it takes effect)`，`on every channel it is given on`），depth（`the check made at each booking, from the channels targets gives each command then: one per channel under the current keys, one per command under the layout` と，booking 後の `cold` change は device の back-pressure），`applied` の行（live/pending/空の 3 つ），第 3 段落（refused を除く candidate，pending の行，失敗は行も `config` も書かず entry を残す）．
- **UR-17 / UR-23**：「unless the record holds that configuration」→「unless the record holds the segment's set and its command」．
- **UR-36**：record の 3 つ目を `(seq, version)` → `seq`，configure の記録，版の段落を「set と command が一致すれば configure しない／device に知らされなかった更新は UR-24 の release の形で届ける（`Provider::stop` の cancel を除く）」に置き換え，`*Test:*` を新しい名前に（旧 2 名は Changes の履歴へ）．
- **UR-24 / UR-29 の `*Test:*`**：新しい系列の名前を追加（UR-24 は 3 行に分けた）．
- **§6**：`ur_24_a_direction_halted_by_a_refusal_has_no_channel` と `ur_24_a_halted_direction_fills_no_queue_slot` の期待を書き換え．
- **Changes**：2026-10-20 の 1 行（UR-17，UR-23，UR-24，UR-29，UR-36，§6 の 2 行）．

## 7. 大きさ

`git diff --numstat`（src のみ）：**追加 889 行，削除 197 行**．内訳（追加）：本体 76 行（`targets`，`release`，`book_timed`，`core.rs`，`device.rs` の `base`），テストと test 専用の道具 813 行（`control.rs` と `tx.rs` の `mod tests` 719，`test_support.rs` 64，`device.rs` の `set_time`/`read_back` 29，`rx.rs` 1）．v3 の見積もり 1,250〜1,480 より小さく，1,500 行を超えていないので分割はしていない．`design/18` は 18/14，`mutations.json` は 202/102（行の追加と削除だけ）．

## fix1-report


# 修正の回 1 の報告 — Phase 8 段階 1b-4d-1c

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `7968d98`．tree の未 commit の 1b-4d-1c の実装に，Sol のコードレビュー（`sol-review.log` の `判定：FAIL`）の 4 件を裁定どおり直した．commit・push・`git add` はしていない．

## 1. 4 件の直し

### 1. Sol major 1（`targets` が空でも，更新を要る segment が残る）

- **直し**（`crates/ezsdr-radio-uhd/src/provider/control.rs:622`–`639`）：記録だけの枝（`chans.is_empty()`）でも，`effective > e` なら，全投影の `updates` の entry を `(e, seq)` から `(effective, seq)` へ動かす（`:628`–`:635`）．live な枝と同じ `ColdConfig::issued()` を，同じ `assert_line_free("a recorded command's release")` と `self.projections(timed.dir)` の `updates` の lock で使う．`updates` の guard は `if` の終わりで落ち，`rec` の lock より前になる（UR-25 の順）．行（`at: e`，`note: "no channel"`，`issued` なし）と `config.insert` は今のまま．`LATE_COMMAND` は足していない．
- **テスト**：`ur_24_a_release_recorded_behind_a_released_command_moves_its_projection`（`control.rs:3054`–`3089`，Sol の系列そのもの）．RX は channels=0，from-none の enable を 100 ms に予約して gain 0 で configure 済み → RX channels=0 を 100.2 ms → 97.5 ms に TX の live な更新を 100.5 ms で release → 97.6 ms に RX gain=3 を 99.7 ms で予約・release → RX の owner が 100 ms の segment を begin．
  - assert (1)：投影の entry が `(100.5 ms, seq)` にあり（`:3079`–`:3083`），100 ms の segment の投影の gain は旧値 `0.0`；`control.released == [100.5 ms]`．
  - assert (2)：行は `(99.7 ms, issued なし, "no channel")`；owner の `begin()` は `apply` も `rx_open` も出さない；device の read-back は `0.0`（`:3084`–`:3089`）．
- **mutation**：P425 を 1 行足した（`plan/maintenance/tools/mutations.json:7874`–`:7883`）．記録だけの枝の `cold.issued(…, effective, …)` を `e` にして entry を動かさない（投影の gain が 3 になる）．killed．
- **`design/18`**：UR-24 の第 3 段落（`design/18-uhd-radio.md:254`）の「is recorded in the configuration at its own instant」を「at the instant it takes effect — its own, or a released command's it is late behind —」にした（DA-1，1 文）．UR-24 の `*Test:*`（`:257`）に新しいテストの名前と短い説明を足した．UR-36 の配送の文（`:271`）は §2 のとおり偽にならないので変えていない．

### 2. Sol major 2（S3 に時計の固定が無い）

- **直し**（`control.rs:2662`）：`ur_24_a_release_during_a_configure_s_return_reaches_the_device`（`:2657`）の最初の booking の前に，他の系列と同じ `hold(&core, &time, &device)` を置いた（`Frozen` と epoch 0 の assert）．
- **報告**：`report.md:23`（§2 の 4）と `:73`（§5 の回 1）を，「この回の tree には `hold` が入っていなかった／その記述は誤り．修正の回 1 で入れた」に直した．

### 3. Sol minor 3（TX の主系列の read-back で時計が一致していない）

- **helper の共有**：`device_at` を `test_support::device_at` に移し（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:141`–`150`），`control.rs` の tests は `use`（`control.rs:880`）で同じものを使う．S3 の `hold` もこれを通る．
- **直し**（`crates/ezsdr-radio-uhd/src/provider/tx.rs:1966`, `:1992`）：S1-TX で epoch 0 を同じ helper で決め，read-back の前に `device_at(&core, &time, &device, origin + 1 ms)` を置く．`(core.now(), device.time_now()) == (at, at)` は helper の中で assert される（RX の `device_at` と同じ形）．

### 4. Sol minor 4（UR-24 の容量の文）

- **`design/18:253`**：`in effect between a command's own instant and its release` を `booked after a timed command's booking` に直し，「timed の booking の後に予約された cold change が，release の出し先の channel を増やしうる」を書いた．理由（depth は booking のときの `targets` の channel で数える）は直前の文，既知の制限（device の back-pressure，current keys の制限で 1b-5 が消す）は Changes の 2026-10-20 の行（`design/18-uhd-radio.md:618`）に置いた（同語はもともとそこにあった）．Changes の行には，記録だけの release が effective の instant に投影へ移ることも足した．

## 2. UR-36 の配送の文の確認

`design/18-uhd-radio.md:271` の「an update the device was not told as its own timed command is given to it as UR-24's release gives it — to a device channel where the plan has one, otherwise recorded for the enable that brings one」は，この系列で偽にならない．gain=3 は effective（100.5 ms）に channel が 1 つも無いので device に timed で渡らず記録され（`config.insert`），「channel を持ってくる enable」は origin が effective 以後の segment の configure だけを指す．100 ms の segment は修正後，旧値のままの投影で configure されるので，この文の「記録した値を当てる enable」ではない．文は変えていない．

## 3. 確かめることの結果

1. `cargo test -p ezsdr-radio-uhd --no-fail-fast`：lib 189 passed，integration 228 passed（他は 0 test の bin）．`cargo clippy --workspace --all-targets -- -D warnings`：警告なし．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`：`0 problem(s)`．
3. mutation（`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 P312 P412 P416 P417 P425`，最終 tree）：**5/5 killed**．
   - P425 killed（新しい行，記録だけの枝で entry を動かさない）．
   - P412 killed（`old` が `if effective > e {` のままだと新しい枝と合わせて 2 箇所に当たるため，live の枝の次のコメント行まで含めて当て直した．`mutations.json:7745`–`7753`）．
   - P312 killed，P416 killed，P417 killed（同じ枝の既存の行．周りのコードが変わったので流した）．

## fix2-report


# 修正の回 2 の報告 — Phase 8 段階 1b-4d-1c

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `7968d98`．tree の未 commit の 1b-4d-1c と修正の回 1 に，Sol の再レビュー（`sol-rereview.log` 末尾，`判定：FAIL`）の major 1・2 を，指示書のとおり「timed command の効く時刻を booking で確定する」形で直した．commit・push・`git add` はしていない．

## 1. 直し

### 1. booking の床（`crates/ezsdr-radio-uhd/src/provider/control.rs:575`–`586`）

- `self.released.retain(|effective| *effective > now)` を floor の計算より前へ移し，`let floor = self.released.iter().copied().max().map_or(earliest, |latest| latest.max(earliest));`（`:582`）で `e = requested.map_or(floor, |requested| requested.max(floor))`（`:583`）．
- `requested` が floor より前なら `late_command(Some(key), requested, e)` を booking で **1 回**（`:584`–`586`）．lead と `L` のどちらで遅れても 1 回．
- `e` は `held` の key（`:612`）と全投影の entry（`:607`–`609`）にそのまま使う．

### 2. release は時刻を動かさない（`control.rs:619`–`676`）

- `effective = max(e, latest)` の式は残し，`debug_assert_eq!(effective, e);`（`:652`）と 1 行のコメント（`:650`–`651`，「The instant is the booking's: the floor there keeps it behind every command released, so the device queue's order cannot move it.」）．以降のコードは `e` を使う．
- release の `late_command`，`ColdConfig::issued` による投影の移動（live と記録だけの枝の両方）を削除．`ColdConfig::issued` は使用箇所が無くなったので関数ごと削除（`control.rs:99`–`102` にあった）．`ur_25_switch_projects_timed_updates_in_effective_order`（`:1719`）は `issued` を使わない形に書き直した（順序と等号 instant の assert は残した）．
- 行の `at` は live も pending も `e`（`:697`–`703`，`:671`）．

### 3. 記録を instant つきで保持する（指示書の 4 の major 1 を閉じるために必要だった）

- `Control` に `released_at: BTreeMap<(i64, u64), Timed>`（`:206`，release した command の値と instant）と `recorded: BTreeSet<(i64, u64)>`（`:210`，channel が無くて記録だけになり，まだ決着していないもの）を足した．`Timed` は `#[derive(Clone)]`（`:39`）．
- `held_updates`（`:453`–`462`）は held と `released_at` を合わせて返す：release の後に作る segment の投影も，値の instant を保つ．`config_at`（`:492`–`501`）は `released_at` を instant から重ねる．
- release の `e ≤ now` の `released_at` は `config` に畳む（`:666`–`675`，以後に始まる segment の基底）．畳むときに `recorded` に残っていれば行を `no channel` で書く（`:670`–`672`）．
- 記録だけの command は，instant が release window 内にある間 `release` のたびに `targets` で決め直し，その間に予約された segment が instant に channel を持つなら device に timed で渡す（`:626`–`645`）．device queue の順を守るため，既に渡したより後ろの instant の command があるときは渡さない（`:632` の `self.released.iter().all(|released| *released <= e)`）．
- 拒否された release は `released_at` からも消し（`:635`–`637`），`config` に畳まない（UR-24 の「writes no configuration of record」）．
- `Provider::stop` は `held` と未決着の `recorded` の両方を cancel し，投影から entry を外して `cancelled` の行を書く（`:709`–`730`）．
- `give`（`:681`–`708`）に「device に渡して行を書く」を切り出した（live の枝と決め直しの枝で共有）．

### 4. `design/18-uhd-radio.md`（DA-1）

- UR-24（`:250`–`:254`）：`LATE_COMMAND` の文を「floor が動かした `at` は booking で late」に，release の文を「instant は booking で確定し，release 済みの最後より前にならない」，記録の文を「instant つきで記録し，window 内で決め直し，渡せない・instant が過ぎたものは `no channel`」に置き換えた．修正の回 1 が足した「記録だけの release は effective の instant に投影へ移る」は削除．`Provider::stop` が recorded も cancel することを足した．
- `*Test:*`（`:257`–`:259`）：名前を変えたテストと新しいテストを反映（`ur_24_a_recorded_release_is_booked_at_the_instant_it_can_take_effect`，`..._segment_booked_after_a_recorded_release_holds_it_at_its_instant`，`..._segment_begun_before_a_recorded_release_configures_the_old_value`，`..._recorded_command_is_given_before_a_later_one`，`..._stop_cancels_a_recorded_command`）．
- UR-30（`:294`）：`applied` の文を「`issued`，channel が無かったものは `note: "no channel"`」「held も recorded も cancel」に直した（HEAD から「every released command recorded as issued」が実装と食い違っていた）．
- UR-36（`:271`）：「otherwise recorded at its instant, decided again there while it is ahead, and applied by the segment that begins at or after it」に直した．UR-26（`:277`）の `Provider::stop` の文に recorded を足した．
- Changes の 2026-10-20 の行（`:618`）に，booking の floor，記録の instant と決め直し，改名・新しいテストを反映した．

## 2. テスト

### 足した

- `ur_24_a_recorded_release_is_booked_at_the_instant_it_can_take_effect`（`control.rs:3189`，指示書 4 の S12c の書き直し．旧 `ur_24_a_release_recorded_behind_a_released_command_moves_its_projection`）：booking の `LATE_COMMAND`（99.7 → 100.5），投影の entry が `(100.5, seq)` で release の前後で動かないこと，行が `(100.5, no channel)`，`released == [100.5]`，owner が configure しないこと，read-back 0.0．
- `ur_24_a_segment_booked_after_a_recorded_release_holds_it_at_its_instant`（`:3236`，major 1）：記録の後に 100.3 ms の enable を予約し，その投影が 100.3 で旧値・100.5 から 3 であること，release の決め直しで device が 100.5 に 3 を取ること（100.4 で 0.0，100.6 で 3.0，行は `issued`），`config_at(101)` も 3 であること．
- `ur_24_a_segment_begun_before_a_recorded_release_configures_the_old_value`（`:3277`，major 2）：owner の configure を **booking と release の間**に置き（probe ではなく，同じ thread で順序を固定），owner の apply に `gain=3` が無いことと 100 ms の read-back が 0.0 であること．
- `ur_24_a_recorded_command_is_given_before_a_later_one`（`:1398`）：記録した command と後に予約した command が，channel が来た same pass で instant 順に device に渡ること（device の `at` と行）．
- `ur_24_a_stop_cancels_a_recorded_command`（`:1432`）：`Provider::stop` が記録だけの command を cancel し，投影から entry が消え，instant が過ぎても release の行も device call も無いこと．

### 直した

- `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`（`:3121`，S12 (a)(b)）：booking の floor と `LATE_COMMAND`（1 回，9 700 → 10 000），held の key と投影の entry が `(10 000, seq)` で release の前後で動かないこと，device が `at=10000`，(b) は `cold` を timed の **後**に booking して 2 channel に渡る形に書き直した．期待（device の順序，`at`）は変えていない．
- `ur_25_switch_projects_timed_updates_in_effective_order`（`:1719`）：`issued` を消したので，投影の順序と等号 instant の assert だけにした（名前は変えていない）．
- 行の時点が変わったため，`ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing`，`ur_24_a_release_for_a_refused_transmit_segment_applies_nothing`，`ur_24_a_release_for_a_refused_receive_change_has_no_candidate`，`ur_24_a_refusal_halts_only_its_own_stream`，`ur_24_a_late_update_halted_before_it_can_act_has_no_channel` で，instant を過ぎてからもう一度 `release()` を呼んで行を見るようにした（`no channel` の行は決着のときに出る）．`ur_29_a_failed_released_update_is_a_device_failure` は，instant 後に `config_at` と行が変わらないことを足した（拒否は畳まない）．
- `ur_25_later_timed_update_survives_cold_switch`（`:1696`）と `ur_24_a_direction_halted_by_a_refusal_has_no_channel`（`:1870`）の `control.config[&gain]` を `config_at(...)` に変えた（値は instant から `config_at` が返す）．
- `ur_25_later_timed_update_survives_cold_switch` ほかの `config_at` の doc（`:490`–`491`）を「released」に直した．

### 消した

- `ur_24_a_release_recorded_behind_a_released_command_moves_its_projection`（改名，上のとおり）．
- 一度書いた `ur_24_a_recorded_command_is_settled_before_a_later_one_is_given` は，その系列では両方の command が同じ pass で順に渡り，意図した経路（後に渡す command が先に commit する）を通らないと分かったので，実装側の「held を渡す前に recorded を決着する」block ごと消して，代わりに決め直しの guard（`:632`）を残した．guard の否側は current keys では再現しない（後述）．

### mutation の行（`plan/maintenance/tools/mutations.json`，`plan/phase7/tools/mutations.json`）

- 消した：P411，P412，P413，P423，P425（`issued` の移動と release の `late_command` に当たっていた），P268 は floor の行に当て直した（名前も「the device queue's order is the instance's one, across the directions」）．phase7 の U21 は削除した `late_command` に当たっていたので，`let e = requested.map_or(floor, ...)` の行に当て直した．
- 当て直した：P409，P410，P414，P415，P416，P417，P418，P419，P424（`give` の行の深さと，記録の行が変わったため）．
- 足した：P426（floor に `L`），P427（booking の `LATE_COMMAND`），P428（投影の entry の key が floor），P429（記録の決め直し），P430（instant が過ぎたものの畳み込み）．`debug_assert_eq!` の行は作っていない：assert は booking の floor から常に成り立ち，外しても観測できる差が出ない（equivalent）ため．

## 3. 確かめることの結果

1. `cargo test --workspace --no-fail-fast`：66 test binary すべて ok（`ezsdr-radio-uhd` の lib は 193 passed，integration の fake ほかも含む）．`cargo clippy --workspace --all-targets -- -D warnings`：警告なし．`--features uhd` は流していない（指示書どおり orchestrator が image で流す）．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`：`0 problem(s)`．renames に 1 行足した（`ur_24_a_release_recorded_behind_a_released_command_moves_its_projection -> ur_24_a_recorded_release_is_booked_at_the_instant_it_can_take_effect`）．
3. mutation（`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 ...`，最終 tree）：**18/18 killed**．
   - 新しい行：P426 killed，P427 killed，P428 killed，P429 killed，P430 killed．
   - 当て直した行：P268 killed，P409 killed，P410 killed，P414 killed，P415 killed，P416 killed，P417 killed，P418 killed，P419 killed，P424 killed，U21 killed．
   - テストを変えた行：P279 killed，P312 killed．
   - 全リストは orchestrator の gate で流す（fix の回は触った行だけ，指示書どおり）．

## 4. セルフレビュー（DeepSeek，`opencode-go/deepseek-v4.1-flash#max`，read-only，3 回）

- **回 1**（`fix2-selfreview.log`）：P1 1 件「記録だけの release が，instant より前に始まる segment には永久に届かない」— **採った**．`released_at` に入れるだけで device には渡らないため，投影は 100.5 から 3，device は 0 のままという食い違いになる（指示書の major 1 のテスト自身の系列）．決め直し（`recorded`）を実装し，S12d に device の read-back を足した．P2 1 件「major 2 のテストが booking → release → configure の順で，壊れた実装でも通る」— **採った**：booking → configure → release の順に直し，`gain=3` が owner の apply に無いことを主 assertion にした．P3（S12d のコメントの誤り，queue の文言，UR-30 の文，`config_at` の doc，assert の数，メッセージの文言）— コメント・文言・doc は直した．assert の数は，S12 の書き直しが 1 つの系列（(a)(b)）を 1 本で見るためで，fix2-brief が S12 の書き直しを求めている以上，分割しない（理由をここに書く）．
- **回 2**（`fix2-selfreview2.log`）：P1 1 件「recorded の give が，後に release した command の後ろに並び，device の effective instant が booking 後に変わる」— **採った**：`released` に自分より後の instant があるときは決め直しで渡さない guard を足した．P2 2 件「拒否された release が次の turn で `config` に畳まれる」「`Provider::stop` が recorded を cancel しない」— **両方採った**（拒否は `released_at` からも消す／`cancel_held` で recorded も cancel）．P3（layout の queue 会計，instant 通過後の give，UR-30 の `channel`，テストの弱さ，`set_time` のコメント）— layout の会計は design/18 の known-limit の文に「後で始まる segment が channel を増やす」として記載済みで，recorded の再入も同じ制限に入る（文言は回 3 で確認）．instant 通過後の give は回 3 で「window 内だけ決め直す」に直した．UR-30 の `channel` と `set_time` のコメントは HEAD からの既存で，この段階の変更ではない（触っていない）．テストの弱さは回 3 の P2-1 で補った．
- **回 3**（`fix2-selfreview3.log`）：P1 1 件「決め直しで give した後，`released_at` から消しているので，その後に作る segment の投影と configuration of record から値が抜ける」— **採った**（成功時は `released_at` に残し，拒否時だけ消す）．P2-1「give の後に予約した segment を通るテストが無い」— **採った**（S12d に `config_at(101)` の assert を足した）．P2-2「layout の queue 会計が recorded を数えない」— **採った**：layout の `pending` に `recorded.len()` を足した（`control.rs:594`–`598`，「One slot per command: held, released, and recorded and not yet given」）．design/18 の「one per command under the layout」がそのまま成り立つ．P2-3「guard の否側が未テスト」— **退けた**：current keys では，後に渡す command が live であるためには「instant 以降に始まる segment が channel を持つ」必要があり，それは先の command の `targets` にも見えるので，先の command は guard に達する前に渡される（再現するのは layout の，自分の channel だけを持つ segment の場合）．guard は 1 行で，誤れば device の順序が壊れるので残した．P3（UR-26・UR-30 の文，UR-24 の guard の説明，Test 行の 2 本，テストのコメント）— すべて直した．

## 5. 起きたこと

- `ur_25_switch_projects_timed_updates_in_effective_order` を書き直したとき，1 つの `assert_eq!` の中で `cold.updates.lock()` を 2 回呼んでしまい，テストが 60 秒以上固まった（`timeout 300` で打ち切り）．lock を変数に取って直した．以後，1 本・1 binary の実行には `timeout 300` を付けている．
- S12d の最初の版（cut を入れた系列）では transmit segment の origin が 147.6 ms になり，`origin < 100.5` の assert が落ちた．cut を 20 ms に移して origin が 100.3 ms になる形にした（回 3 の P3 で，そのとき書いた「origin は常に instant より後」というコメントが誤りだと指摘され，直した）．
- `mutations.json` の編集で，行の移動を index でやったため 2 回 JSON を壊した（comma 抜け，重複 block）．その場で差分を読み直して直し，`json.load` と全行の「`old` が 1 回だけ一致」の check で確認した．行は失っていない（maintenance 791 行，phase7 166 行，すべて一致 1 回）．
- `design/18` の UR-24・UR-30 が 1 500 文字を超えて `rules.py` が落ちたので，文を詰め，段落と `*Test:*` の列を改行した（`0 problem(s)` に戻した）．

## 6. していないこと

- `--features uhd` の clippy，実機（X310）での確認，bench．
- mutation の全リスト（`all`）．fix の回なので，触った行と新しい行だけを流した（指示書どおり）．`debug_assert_eq!` の行は作っていない（equivalent のため，理由は §2）．
- `plan/phase8/33-channels-and-streams.md` と stage の test-plan の更新（指示書の「やること」に無い）．`ur_24_a_recorded_command_is_settled_before_a_later_one_is_given` は書いてから消した（§2）．

## fix3-report


# 修正の回 3 の報告 — Phase 8 段階 1b-4d-1c

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `7968d98`．tree の未 commit の 1b-4d-1c と修正の回 1・2 に，指示書のとおり「channel を待つ command は held に残し，release はそこで止まる」形を入れた．`recorded` の集合は消した．commit・push・`git add` はしていない．

## 1. 直し

### 1. channel を待つ command は held に残る（`crates/ezsdr-radio-uhd/src/provider/control.rs:616`–`661`）

- `Control::release` の loop は `held` を `(e, seq)` の順に見て，window 内の先頭の command について：
  - `targets` が空で `e > now` → **`break`**（`control.rs:629`–`633`）．command は held に残り，後ろの command は出ない．
  - `targets` が空で `e <= now` → held から除き，`no channel` の行を書き，`of_record` で構成の記録に畳む（`control.rs:634`–`639`）．
  - 空でなければ今のとおり `give`（`Some(e)`，`released`，行）して `released_at` に入れる（`control.rs:641`–`651`）．
- `effective = max(e, released.max())` と `debug_assert_eq!(effective, e)` は残した（`control.rs:641`–`645`）．コメントの意味（instant は booking で確定済み）も残した．
- 畳み込みを `fn of_record(&mut self, timed: Timed)` に切り出した（`control.rs:661`–`666`）．`e <= now` の channel 無しと，release した command の instant 経過の両方がこの 1 行を通る（`self.config.insert` は 1 か所）．

### 2. `recorded` を消し，`released_at` は give したものだけを持つ

- `Control` から `recorded: BTreeSet<(i64, u64)>` とその doc，`new` の初期化を削除（`control.rs:188`–`211`，`:283`–`292`）．
- release の頭にあった「recorded を決め直す」loop と，`released_at` の畳み込みにあった `recorded.remove` を削除．
- `cancel_held` は `held` だけを cancel する（`control.rs:698`–`718`）．`recorded` と `released_at` を継ぐ chain を削除．channel 待ちの command は held にいるので `Provider::stop` が cancel する（UR-26）．
- layout の queue の会計は `held.len() + released.len()`（`control.rs:588`–`591`）．`recorded.len()` の足し算を削除．held が channel 待ちを含むので「1 command 1 slot」はそのまま成り立つ．
- `held_updates` は held と `released_at` を合わせたまま（`control.rs:441`–`453`）．後に作る segment の投影は，release 済みの値も booking 時の instant で持つ．

### 3. `design/18-uhd-radio.md`（DA-1）

- UR-24：`release` の段落（`:254`）を「channel を待つ command は held に残り，release はそこで止まる（device に出す順は `e` の順）．`e` を過ぎても channel が無ければ `no channel` で記録し，構成の記録に畳む」に直した．止めることの影響（後ろの command は `e` が過ぎるまで出ず，近ければ device lead より短い時間で出て，device は着いたときに適用する）も同じ段落に 1 文で書いた．
- UR-24 の layout の文（`:250`）：「with none of them it is recorded」→「it stays held, as the release below says」．
- UR-24 の booking の floor の文（`:251`）：「one the floor moves is late」→「**an `at`** the floor moves is late」．コードは `at` があるときだけ `LATE_COMMAND` を出す（セルフレビュー P2 の指摘，§4）．
- UR-24 の `Provider::stop` の文（`:255`）：「those still held and those recorded and not yet settled」→「those still held, one waiting for a channel included」．
- UR-24 の `*Test:*`（`:258`）：改名 5 本と新しい 2 本を反映．`ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it` を「release が待っている command で止まる」の Test にした．
- UR-36（`:273`）：「otherwise recorded at its instant, decided again there while it is ahead」→「otherwise held for a channel until its instant, recorded `no channel` if none came」．
- UR-26（`:279`）：`Provider::stop` が cancel するものを「the held timed commands, one waiting for a channel included」に．
- UR-30（`:296`）：「every held or recorded one cancelled」→「every held one cancelled, a command whose instant passed with no channel recorded `no channel`」．ついでに，セルフレビュー P3 の指摘どおり「beside `channel`」を「a configure's row names it in `channel` too」に直した（release と cancel の行は `channel` を持たない）．
- Changes の 2026-10-20 の行（`:621`）：recorded を決め直す形の記述を，held に残して止まる形に書き換えた．改名したテスト名も反映．

## 2. テスト

### 足した（2 本）

- `ur_24_a_command_waiting_for_a_channel_keeps_a_later_booking_behind_it`（`control.rs:1419`，反例）：rx gain 3.0 を 100.5 ms に booking → release で channel が無いので held に残る → tx gain 4.0 を requested 99.8 ms で booking（floor は `released` が空なので `now + 2 ms = 99.6`，e = 99.8）→ rx channels 1 を 100.3 ms に booking → 同じ pass で device に出る順が `tx at=99800` → `rx at=100500`，両方の行が `issued`．旧 fix2 の形では，recorded の loop が先に rx を 100.5 で device に渡し（`released = [100500]`），held の loop の tx が `effective = 100500 != e` になって `debug_assert` が落ちる系列である（この tree には旧形が残っていないので，旧形での実行はしていない．機構からの推論）．
- `ur_24_a_command_waiting_for_a_channel_holds_back_the_one_behind_it`（`control.rs:1448`，止めること）：rx gain 3.0 を 100.5 ms，tx gain 4.0 を 101.0 ms に booking．98.5 ms の release では rx が channel 無しで `e > now` → break し，tx（channel は live，window 内）も出ない（`held.len() == 2`，device の timed apply 0）．100.6 ms の release で rx は `e <= now` の `no channel`（行は `(100500, issued=false, no channel)`，`config_at` は 3.0），その後に tx が `at=101000` で出る．

### 直した（改名 5 本，assert は変えていない）

- `ur_24_a_recorded_command_is_given_before_a_later_one` → `ur_24_a_command_waiting_for_a_channel_is_given_before_a_later_one`（`control.rs:1385`）
- `ur_24_a_stop_cancels_a_recorded_command` → `ur_24_a_stop_cancels_a_command_waiting_for_a_channel`（`:1478`）
- `ur_24_a_recorded_release_is_booked_at_the_instant_it_can_take_effect` → `ur_24_a_command_waiting_for_a_channel_is_booked_at_the_instant_it_can_take_effect`（`:3235`）
- `ur_24_a_segment_booked_after_a_recorded_release_holds_it_at_its_instant` → `ur_24_a_segment_booked_while_a_command_waits_for_a_channel_holds_it_at_its_instant`（`:3283`）
- `ur_24_a_segment_begun_before_a_recorded_release_configures_the_old_value` → `ur_24_a_segment_begun_before_a_late_command_s_instant_configures_the_old_value`（`:3324`）

5 本は，held に残す形でも期待（device の順序・`at`・行・投影・read-back）がそのまま成り立った．直したのは名前とコメントだけである（S12c/d/e/f，S11d，`ur_24_a_late_update_halted_before_it_can_act_has_no_channel` の「recorded を決め直す」という記述）．`--renames` には足していない：この 5 本は HEAD でまだ存在せず（修正の回 2 で作った），HEAD の `design/` が cite していないため，`rules.py` の rename 検査の対象にならない（`renames.txt` は修正の回 2 のまま，`rules.py` は `0 problem(s)`）．

### 消したテスト

なし．

### mutation の行（`plan/maintenance/tools/mutations.json`，`plan/phase7/tools/mutations.json`）

- 消した：P417（`self.recorded.insert`），P429（recorded を window 内で決め直す guard），phase7 の R03（release が `effective` を動かす前提の行．いま `effective` は `debug_assert` だけが読み，`|_| e` にしても観測できる差が出ない equivalent になるため）．
- 当て直した：P416（`no channel` の行が held の loop に移り，`self.core.at(order.0)` → `self.core.at(e)`），P430（畳み込みの insert が `of_record` に移った），P428（filter を改名した `ur_24_a_segment_begun_before_a_late_command_s_instant_configures_the_old_value` に）．
- 足した：P431（止める `break`：`if e > now {` → `if false {`），P432（`e <= now` の畳み込みの呼び出し：`self.of_record(timed);` → `let _ = timed;`）．
- 全行の `old` が 1 回だけ一致することを，`json.load` と文字列の count で確認した（maintenance 791 行，phase7 165 行，すべて 1 回）．

## 3. 確かめることの結果

1. `cargo test --workspace --no-fail-fast`：66 test binary すべて ok（exit 0）．`ezsdr-radio-uhd` の lib は 195 passed．
2. `cargo clippy --workspace --all-targets -- -D warnings`：警告なし．`--features uhd` は流していない（指示書どおり orchestrator が image で流す）．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`：`0 problem(s)`．design/18 の 1 500 文字超の行は 0（最大 1 493）．
4. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <19 IDs>`，最終 tree，**19/19 killed**．
   - 新しい行：P431 killed，P432 killed．
   - 当て直した行：P416 killed，P428 killed，P430 killed．
   - 触った code/test に当たる行：P268 killed，P312 killed，P409 killed，P410 killed，P414 killed，P415 killed，P418 killed，P419 killed，P421 killed，P422 killed，P424 killed，P426 killed，P427 killed，U21 killed．

## 4. セルフレビュー（DeepSeek，read-only，1 回）

`opencode-go/deepseek-v4.1-flash#max`，`--agent plan`，ログ `fix3-selfreview.log`（チェックリスト `fix3-selfreview-brief.md`）．判定は **P1 なし**，P2 1 件，P3 7 件．CLI の exit は 1 だったが，ログには判定まで書けている（エラー行は無い）．

- **P2-1「`at` の無い command が `released` の floor で遅れても `LATE_COMMAND` が出ない」** — **採った（design の文を狭めた）**．コードは `at` があるときだけ late を出す（`book_timed` の `requested.filter(...)`）．UR-24 の `:250` も「An `at` the floor below moves is late there」なので，`:251` と Changes の行を「**an `at`** the floor moves」に直して実装と一致させた．動作は変えていない（この回の指示に無い挙動変更を避けた）．
- **P3-2「`effective` は vacuous．消すか guard として書け」** — **退けた**：fix2 の指示書 §2 と fix3 の規則 1 が `effective == e` を残すことを求めており，debug build で崩れないことを固定する guard である．今は常に成り立つのが正しい（booking の floor で確定している）．
- **P3-3「新しいテストのコメントが通らない枝を主張する」** — **採った（コメントを直した）**：channel は pass の前に booking した `cold` change が持ってくるので，「the channel a `cold` change brought before the next pass gives the waiting one its own instant there」に直した（assert は変えていない）．
- **P3-4「`ApplyFails::timed: Option<bool>` は常に `Some`」/ P3-5「`set_time` の "Forward only" が強制されていない」** — **退けた**：どちらもこの回より前（段階の実装と修正の回 1）の `test_support.rs` と `device.rs` で，この変更の機構（recorded の除去と release の順序）と関係しない．この回の指示書の範囲外なので触っていない．
- **P3-6「テストの doc が assert しない挙動を言う」** — **採った**：`ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing` のコメントから「; the enabling change applies it」を消した（`control.rs:2924`）．
- **P3-7「UR-30 の 'beside `channel`' は release と cancel の行に成り立たない」** — **採った**：`applied` の文を「a configure's row names it in `channel` too」に直した（`design/18:296`）．
- **P3-8「layout の文が holding を落としている」** — **採った**：UR-24 の layout の文を「with none of them it stays held, as the release below says」に直した（`design/18:250`）．

チェックリストの 1（device に出す順が `e` の順から外れる経路）は「見つからない」，2（recorded の名残）は「無い」，3（新しいテストが反例の実際の経路を通るか）は「両方通る」という答えだった．

## 5. 起きたこと

- この回の最初の `cargo check`・`cargo test`・clippy は一度で通った．固まったテストは無い（テストはすべて `timeout 300` で流した）．
- セルフレビューの `opencode2 run` は exit 1 で終わったが，ログの末尾に P1 なしの判定と指摘が書けており，エラー行は無かった．
- mutation は 1 回の remote run（19 行，sim02 8 shard・sim03 8 shard）で完走し，再実行はしていない．
- `mutations.json` は，変える block と足す block を `edit` で書き，全体の書き直しはしていない．追記後に JSON の妥当性と全行の一致を確認した．

## 6. していないこと

- `--features uhd` の clippy，実機（X310）での確認，bench．
- mutation の全リスト（`all`）．fix の回なので，新しい行と触った行だけを流した（指示書どおり）．全リストは orchestrator の gate で流す．
- `plan/phase8/33-channels-and-streams.md` と test-plan の更新（指示書の「やること」に無い）．新しい 2 本に S 番号は振っていない（test-plan に無い系列のため）．
- 旧 fix2 の形（recorded を決め直す実装）での新しいテストの実行．tree に旧形が残っていないため，反例で落ちることは機構からの推論として §2 に書いた（セルフレビューも同じ理由で実行できていない）．

## fix4-report


# 修正の回 4 の報告 — Phase 8 段階 1b-4d-1c

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `7968d98`．tree の未 commit の 1b-4d-1c と修正の回 1〜3 に，Sol の再レビュー（`sol-rereview2.log` の末尾，判定 FAIL）の 3 件を指示書のとおり入れた．規則 1〜3（booking で時刻を確定，channel 待ちは held に残って release を止める，`released_at` の畳み込み）は変えていない．commit・push・`git add` はしていない．

## 1. 直し

### 1. Sol major 1：畳み込みの順序（`crates/ezsdr-radio-uhd/src/provider/control.rs:616`–`668`）

- channel の無いまま `e <= now` になった command は，その場で `of_record` に渡さず，`no channel` の行を書いた同じ時点で `released_at.insert((e, seq), timed)` に入れる（`control.rs:640`–`646`）．
- 構成の記録への畳み込みは release の最後の 1 つの loop だけにした（`control.rs:659`–`667`）．`released_at.range(..=(now, u64::MAX))` を `(e, seq)` の順に見て `of_record` を呼ぶ．
- `self.config.insert` を呼ぶのは `of_record`（`control.rs:672`–`674`）の 1 か所のまま．
- 「held は `e` の順に処理されるので，release 済みの A（`e_A`）がまだ `released_at` にあって，それより前の `e` の command が後から入ることは無い」を 1 文（3 行に折り返し）のコメントにした（`control.rs:661`–`663`）．
- `give` が refusal で false を返したとき，または loss で return したときは，その pass の fold は走らない．その間は `config_at` と `held_updates` が `released_at` の値を補い，次に成功した pass で `(e, seq)` 順に畳まれる（挙動は変えていない）．

### 2. Sol major 2：release の channel の間の loss（`control.rs:616`–`709`）

- `release` の loop の各 command の前に `self.core.is_lost()` を見る（`control.rs:624`–`628`）．lost なら device を呼ばずに `return` する（入口の検査 `control.rs:617`–`619` はそのまま）．
- `give` の各 channel の `apply` の前に `self.core.is_lost()` を見る（`control.rs:685`–`690`）．lost なら `false` を返す．`device_failed` は呼ばない（loss はすでに公開されている）．`Core::configure` の channel ごとの検査（`core.rs:730`–`737`）と同じ位置に置いた．
- loss で `give` が false を返した command は held から除かれたままで `released_at` にも入らない．device は lost なので以後の `release` は入口で止まり，行も構成の記録も書かない（新しい 2 本の loss テストがこの行の不在を見る）．

### 3. Sol minor 3：UR-24 の理由の文（`design/18-uhd-radio.md`，DA-1）

- `:251` の「so that … never has to move its instant」と「so that the device queue's order is the effective order (UC-2)」は理由なので，規範から外した．理由は Changes の 2026-10-20 の行（`:621`）に「so that the device queue's order is the instant order and no projection's instant changes after a configuration was made from it」として残した（Changes の行には元から投影の理由があり，queue の理由を足した）．
- booking の floor と `LATE_COMMAND` の説明の重複を 1 回にした．`:250` の「An `at` the floor below moves is late there: …」を消し，`:251` の「A command's instant is settled at its booking …」の 1 文に「an `at` the floor moves is late there, `radio.LATE_COMMAND { key, requested: at, applied: e }` with `e` the settled instant」として置いた．
- UR-29（`:289`）に 1 文足した：「a release with a command or a channel still to give sees it before each and makes no further device call」．
- UR-29 の `*Test:*`（`:292`）に新しい 2 本，UR-24 の `*Test:*`（`:257`）に新しい 1 本を足した．Changes の 2026-10-20 の行（`:621`）に，畳み込みの順序（「settles into the configuration of record with the released commands in instant order」），release の loss の文，新しいテスト名を足した．

## 2. テスト

### 足した（3 本）

- `ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order`（`control.rs:2956`，Sol の系列そのまま）：RX に channel があり（`control(&core, &[Dir::Rx])`），101 ms に全 channel が抜ける `cold` change を予約（後の enable は無い）．gain=1 の A（`e` = 100 ms）を 97.5 ms に release すると `released_at` に残る．同じ key の gain=2 の B（`e` = 102 ms）を予約し，次の release を 102.5 ms にする．assert：(1) `config_at(i64::MAX)` が 2（修正前は A が後から畳まれて 1），(2) その後に予約した enable（200 ms，channels 1）の segment の `configure` が device に gain 2 を渡す（`read_back`）．行は A が `(100 ms, issued)`，B が `(102 ms, no channel)`．
- `ur_29_a_loss_published_between_a_release_s_channels_stops_the_rest`（`control.rs:3187`）：2 channel の timed release（A 49 ms，B 50 ms，release 47 ms）．channel 0 の `apply` が返った直後に `LockSpy::stop_after("apply")` で止め，別 thread が `core.device_lost` を公開してから gate を release する．assert：apply は 1 回だけ（`apply rx 0`, gain 3, at 49 ms），channel 1 は呼ばれず，後に続く command も呼ばれず，行は書かれず，`DEVICE_LOST` は 1．
- `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`（`control.rs:3219`，セルフレビュー P3-1）：A（gain=1，`e` = 49 ms，その時点では channel が live）と B（gain=2，`e` = 51 ms）を予約し，50 ms に channel が抜ける `cold` change を置く．45 ms の release は window 外なので 2 本とも held．52 ms の release で A の `apply` が返った後に別 thread が loss を公開する．assert：apply は A の 1 回だけ，B（`e <= now`，channel 無し）の `no channel` の行は書かれない，`DEVICE_LOST` は 1．これは `release` の loop の per-command 検査（`control.rs:626`）を固定する：S9f だけでは `give` の per-channel 検査が先に止めるので，`:626` を消しても green になる．

### 直したテスト

- 既存の assert は変えていない．`ur_29_a_loss_published_between_a_release_s_channels_stops_the_rest` には `DEVICE_LOST` が 1 であることの assert を足した（下のセルフレビュー P3-2）．

### mutation の行（`plan/maintenance/tools/mutations.json`）

- P432（channel 無しの畳み込み）の `old` を，`of_record(timed);` から新しい `self.released_at.insert((e, seq), timed);`（16 space の行）に当て直した．
- 足した：P433（同じ行を `self.of_record(timed);` に戻す＝修正前の順序．S19 を殺す），P434（`give` の各 channel の前の検査を `if false` に．S9f を殺す），P435（release の loop の各 command の前の検査を `if false` に．S9g を殺す）．
- `plan/phase7/tools/mutations.json` は変えていない．全体の書き直しはしていない．maintenance 794 行・phase7 165 行のすべての `old` が 1 回だけ一致することを `json.load` と文字列の count で確認した．

## 3. 確かめることの結果

1. `cargo test --workspace --no-fail-fast`：66 test binary すべて ok（exit 0）．合計 1470 passed，0 failed．`ezsdr-radio-uhd` の lib は 198 passed（修正前 195＋3 本）．
2. `cargo clippy --workspace --all-targets -- -D warnings`：警告なし．`--features uhd` は流していない（orchestrator が image で流す）．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`：`0 problem(s)`．
4. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <26 IDs>`，最終 tree，**26/26 killed**（maintenance 21/21，phase7 5/5）．
   - 新しい行：P433 killed，P434 killed，P435 killed．
   - 当て直した行：P432 killed．
   - 触った code/test に当たる行：P268, P312, P409, P410, P414, P415, P416, P418, P419, P421, P422, P424, P426, P427, P428, P430, P431, U20, U21, U37, R21, C02 — すべて killed．
   - その後，S9f・S9g の assert を `spy.seen("apply")` を足して強めた（code は変えていない）ので，P434・P435 を 1 回だけ再実行した：`remote_mutate.py maintenance sim02:4,sim03:4 P434 P435`，**2/2 killed**．

## 4. セルフレビュー（DeepSeek，read-only，1 回）

`opencode-go/deepseek-v4.1-flash#max`，`--agent plan`，ログ `fix4-selfreview.log`（チェックリスト `fix4-selfreview-brief.md`）．CLI の exit は 0．判定は **P1 なし，P2 なし，P3 3 件**．

- **P3-1「release の per-command loss 検査（`:626`）が unpinned」** — **採った**．S9f では loss が最初の `give` の中で公開されるため `give` の per-channel 検査（`:688`）で止まり，`:626` を消しても green になる，という指摘は正しい．S9g（`:3219`）と P435 を足した．S9g は A の `give` が成功した後に loss を公開し，B の `no channel` の行が書かれないことを見る．P435 は killed．
- **P3-2「S9f が loss の一度だけの報告を固定しない」** — **一部採った**．S9f に `DEVICE_LOST` 1 の assert を足した（test の loss が実際に publish されたことを固定する）．ただし `Core::device_lost` は `streams` の下で `lost` を見て冪等に返る（`core.rs:635`–`642`）ので，二重に報告する経路が無く，「報告しない」ことはこの assert でも観測できない．指摘の前提（`device_failed` が二度目の報告を起こしうる）は成り立たないと判断した．
- **P3-3「`set_time` の forward only が強制されていない（`device.rs:756`）」** — **退けた**：`device.rs` の test helper で，この回の機構（畳み込みの順序と release の loss の検査）に無い．fix3 の範囲外 P3 と同じ扱いにした．必要なら別の step で扱う．

チェックリストの答え：(1) `config.insert` へ値を入れる経路はすべて `(e, seq)` の順（唯一の insert は `of_record`，その呼び出しは最後の fold のみ），(2) `release` が device を呼ぶすべての点（`give` の各 `apply`）の前に loss の検査があり，`release` の各 command の前にもある，(3) 新しい 3 本はそれぞれ no-channel の枝と fold，`give` の channel 間，command 間という実際の経路を通る（tree の production code を呼ぶ）．

## 5. 起きたこと

- `cargo test`・clippy・`rules.py` は一度で通った．固まったテストは無い（1 本のテストは `timeout 300`，workspace は `timeout 1800` で流した）．
- mutation は 1 回の remote run（26 行，sim02 8 shard・sim03 8 shard）で完走した．assert を強めた後に P434・P435 だけを 1 回再実行した（2/2 killed）．
- mutation が copy した tree には S9g と P435 が入っている．その後 `design/18` の `*Test:*` 行に新しいテスト名を足した（code は変えていない）．S9f・S9g の assert を `seen("apply")` で強めた後，P434・P435 を再実行した（§3）．
- `mutations.json` は block の `edit` のみで，全体の書き直しはしていない．追記後に JSON の妥当性と全行の一致を確認した（§2）．
- `cargo test --workspace` は 3 回流した（S19・S9f の追加後，S9g の追加後，S9f・S9g の assert を強めた後）．どれも exit 0．報告の数は最後のものである．

## 6. していないこと

- `--features uhd` の clippy，実機（X310）での確認，bench．
- mutation の全リスト（`all`）．fix の回なので新しい行と触った行だけを流した（指示書どおり）．全リストは orchestrator の gate で流す．
- `plan/phase8/33-channels-and-streams.md` と test-plan の更新（指示書の「やること」に無い）．新しい 3 本に S 番号は振っていない（報告内の S19・S9 f/g はこの報告の呼び名）．
- 修正前の実装（channel 無しをその場で `of_record` する形）での新しいテストの実行．tree に旧形が無いので，落ちることは P433 の killed で確かめた．
