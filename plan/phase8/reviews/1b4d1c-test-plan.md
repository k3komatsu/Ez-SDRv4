STATUS: done

# Phase 8 段階 1b-4d-1c テスト計画 v3 — channel が無い release を，pending の segment の origin までに device へ届ける（形 F'）

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `cddad68`（tree はきれい）．指示書：`plan-brief.md`．**裁定は `adjudication-2.md` を最優先**とし（`adjudication.md` と v2 より優先），Sol の再レビュー `sol-plan2.log` の末尾（1372 行「**推奨：F' の方針は維持し…**」以降）に従う．v2 は `test-plan-v2.md`，v1 は `test-plan.md`．

コードは書いていない．tree は変えていない．`cargo test`・`cargo fmt`・`rustfmt`・`mutate.py`・`remote_mutate.py`・commit・push・`git add` はしていない．テストも流していない．以下の `path:line` は HEAD のもの．

## 0. v2 からの差分（裁定 2 と Sol の再レビュー）

1. **時計（Sol 1，裁定 §2.1）**．v2 の read-back と `effective` の assert は，`rig()` の `Core`（`ManualTimeAuthority`）と `FakeDevice` の時計が別なので成立しない（`test_support.rs:62-75`，`device.rs:573-588`，`core.rs:512-554`）．test 専用の道具 `FakeDevice::set_time(at)`・`read_back(dir, chan)` と，device の時計を止める `Frozen` を足し，`set_time` の直後に `device.time_now() == at` を assert する（§2.1）．read-back を使う全テストがこれを使う．
2. **release の queue の上限（Sol 2，裁定 §1.1）**．release の pass で `released.len()`（channel ごと，effective 未満）と，これから出す channel 数の和が `command_queue_depth` を超えるなら，その command を `held` に戻して pass を終える．次の pass で slot が空いていれば出し，`e` を過ぎていれば `Some(effective)` で出す（新しい late の規則は作らない）．booking の会計（`targets` の長さと `COMMAND_QUEUE_FULL`）はそのまま．系列 S4b を足す．
3. **失敗した release は投影から除く（Sol 4，裁定 §1.2）**．`apply` が lost でない `Err` を返したら，その `(e, seq)` を全投影の `updates` から除き，`device_failed` で return する（行も `config` も書かない）．部分適用は channel 0 に値が残り，投影は古い値；後の configure はその古い値を当てる．系列 S9(e) を足す．これで版を消す根拠 (a) が失敗経路を含めて成り立つ（§4）．
4. **release の後に segment が cut・拒否される（Sol 7，裁定 §1.3）**．release 済みは取り戻さない．系列 S8b を 1 本足す（§3）．
5. **失敗の観測（Sol 3，裁定 §2.3）**．`ApplyFails` の入口で，失敗の判定と inner の呼び出しの前に `dir/chan/at` を `attempts` に記録する．S9(a) は `LockSpy::seen("apply")`，S9(d) は `attempts` で見る．S9(c) は「拒否する channel が最後でない」並び（channel 0 を拒否し，channel 1 を呼ばない）も足す．`ApplyFails` に `timed`・`timed_lost` の 2 口を足す（`ApplyFails::new` は今のまま）．
6. **S12（Sol 5，裁定 §2.4）**．device の command の `at=10 000 µs` を直接 assert し，pending の行の `at=9 700 µs`・`note="no channel"`・`issued=true` の 3 欄を assert する．release 済みの後ろで，`e` と effective の間に cold 変更が来る第 2 の例も足す（`targets` を `e` で引く誤りを落とすため，§3 S12(b)）．
7. **fake の FIFO と frequency（Sol 7，裁定 §2.5・§2.6）**．fake は直さない．S2・S12 が証明する範囲を「呼び出しの順」と「fake の in-order queue が保つ `at` の順」に限定し，untimed の apply と timed の queue の関係は INFERRED（#62，bench B8）とする．frequency の DSP 側（idle の DDC/DUC）はこの段階では測らない．テストは gain で書き，Sol の系列（idle 中の timed frequency → owner の untimed configure → 長い無通信 → 最初のサンプル）を bench 行きとして §8 に記録する．
8. **S16〜S18 の観測（Sol 1，裁定 §2.2）**．read-back は設定の状態だけを見る．主張を「cut の直前・`e` の直前・`e` の直後の 3 点の device の設定」に限定し，`design/18` の Test の説明も「最後のサンプル」と書かない．名前も `..._keeps_the_setting_until_its_instant` に改める．
9. **S3（Sol 8，裁定 §2.7）**．主張を「記録を公開する前の窓」に限る．lock の保証はこの系列では落とせないと書き，`updates` の既存の timed の枝の lock は残す（S10 の既存テストが覆う）．
10. **S14（裁定 §2.8）**．`mark` は「2 度目の from-none の configure の後，`begin()` の前」に置く，と本文に明記する．
11. **S15（裁定 §2.9）**．延期は支持されたが，join の順（control → TX → RX，`mod.rs:695-704`）を書き，「plan の end だけが守る」と断定しない（INFERRED）．版削除の安全の証明には使わない．
12. **分割（裁定 §2.10）**．S16〜S18 だけを後に送る fallback は採らない．1,500 行を超える見込みなら §7 の順で orchestrator に提案する（S16〜S18 は動かさない）．
13. **`design/18`（Sol 6，裁定 §3）**．layout は今のとおり常に 1 本ずつ数える（「候補が無ければ 0」は current keys だけに書く）．第 1 段落の「at that instant」は effective instant にする．`:252` の「Each released command is recorded … with `e`」を live の行と pending の行に分ける．UR-24 に depth の上限と失敗の除去を足す．UR-36 の Test から「旧テストが外れる」を Changes に移す．
14. 大きさを数え直した：約 1,250〜1,480 行（§7）．

**この v3 で裁定してほしいこと**：§2.1 の時計の道具（`set_time`・`Frozen`・`read_back`），§2.3 の失敗の除去の文面，§3 S12(b) を残すか（`e` と effective の間の cold 変更），§7 の大きさが 1,500 に近いときの分け方（S16〜S18 は動かさない前提で，どこを先に落とすか）．

## 1. 自分で立てた系列と，指示書・裁定との差分

brief を読む前に立てたもの（v2 の M1〜M18 を F' で読み直し，裁定 2 の項目を M19〜M23 として足した）：

| 自分で立てた系列 | 指示書・裁定 | 差分 |
|---|---|---|
| M1 RX：owner が begin → `e < O` の更新を記帳 → release → timed 1 回 → 実 origin の後の read-back | S1，裁定 §2.1 | read-back の前に device の時計を `O` に合わせる（§2.1） |
| M2 TX：同じ | S1-TX | 同じ |
| M3 記帳が configure より前（記録は `configured == None`） | S2 | 2 度当てることを固定する（値はどちらも 3.0） |
| M4 configure の戻りと記録の公開の間の release | S3，裁定 §2.7 | probe を離す前に `configured == None`．lock の主張はしない |
| M5 queue：channel が無い間の記録だけの更新の会計 | S4，裁定 §2.8 | 17 個目は booking で `COMMAND_QUEUE_FULL`（同じ pass の順序も見る） |
| M6 遅れた release | S5，裁定 §2.4 | 新しい late の規則を作らず，live な枝の到着時の適用を固定する |
| M7 pending の segment が無い | S6 | 同じ |
| M8 layout：前の segment は持たず後の segment が持つ | S7 | 同じ（command の channel だけ） |
| M9 拒否された segment だけが持つ | S8，裁定 §2.6 | TX は plan に残る拒否 segment，RX は plan から消えた segment |
| M10 loss と apply の失敗 | S9，裁定 §2.5 | (a)〜(d) に (e)（from-none の configure 済み＋owner が begin）を足す |
| M11 lock | S10 | 既存の `ur_25_the_line_lock_is_free_inside_every_control_device_call` を拡張 |
| M12 版を消す判断と既存テストの分類 | S11 | §4 と §9 |
| M13 halted の方向に拒否されていない pending の enable がある | — | S13．`e` の timed で出る．§6 の行も変える |
| M14 release 済みの command の後ろ（`effective > e`） | — | S12．timed で `effective` に出し，`issued` で投影を動かす |
| M15 seq だけの同一性 | — | S14（`mark` の位置を明記） |
| M16 古い segment がまだ流れている間の release | — | S16〜S18（3 点の read-back） |
| M17 stop と owner の境界 | — | S15（次の段階へ送る） |
| M18 TX の `reading` と release の関係 | — | テストは足さない（§8） |
| M19 **queue が booking の後に増える**：channel も pending も無い間に 17 個を記帳し，その後に enable を予約して同じ window で release する | S4b（裁定 §1.1），Sol 2 | 16 個で止まり，17 個目は held，slot が空いてから出る．live が 1→2 に増える形も |
| M20 **release の失敗の後に owner が configure しない**：from-none で configure 済み，`e < O` の更新の timed apply が拒否，owner が begin | S9(e)（裁定 §1.2），Sol 4 | 投影から除くので，owner も device も更新前の値 |
| M21 **release の後に宛先が拒否される** | S8b（裁定 §1.3），Sol 7 | 取り戻さない．device は effective に適用し，`released` は retire し，行は残る |
| M22 **偽の失敗を観測する**：device を呼ばなかったことを apply の log で見る | S9(a)(d)（裁定 §2.3），Sol 3 | wrapper の入口の `attempts` で見る（fake の `mark_lost` は lost にしない，§8） |
| M23 **時計が合っていない** | §2.1（裁定 §2.1），Sol 1 | `set_time`＋`Frozen`＋`time_now` の assert |

## 2. 形 F'（テストが依存する点）

**D1（採る）**：pending の segment の channel に出したときの `applied` の行は，今の記録だけの行のまま `at: e`，`note: "no channel"` に **`issued: true`** を足す（裁定 §2.4・Sol 10）．UR-30 の「every released command recorded as issued」が真のままになり，VH-8 の `updates()`（`parity.rs:685-706`）は `note` の行を除くので比較は変わらない．`control/differential.rs:270-272` の `issued || note` も 1 行のまま．

### 2.1 時計（test 専用）

`rig()` の `Core` の `ManualTimeAuthority` と `FakeDevice` の `HostClock` は別で，`Core::new` に同期は無い（`test_support.rs:62-75`，`device.rs:573-588`）。既存のテストは timed command の**呼び出しの log**だけを見て，device の設定を読んでいない（`grep`: `control.rs` の read-back は無く，`tests/fake.rs` も `calls()` と `transmitted()` だけ）．だから道具を足す（`device.rs`，test build だけ）：

- `FakeDevice::set_time(&self, at: i64)`：`st.offset = at - base`（`base` は `now(st)` の offset 抜き，`device.rs:731-741`）にして `settle`（`:746-759`）を呼ぶ．device の時計を root ticks の `at` に置く．**前に戻さない**（テストは昇順だけ）．
- `FakeDevice::read_back(&self, dir: Dir, chan: usize) -> Settings`：`settle` の後に `st.settings[dir][chan]` から `Settings { rate, freq, gain, antenna }` を返す（`device.rs:679-691` の隣）．
- `test_support::Frozen::of(device: &FakeDevice)`：device の host clock の participant を 1 本立て，テストが終わるまで busy にする（`host.rs` の M3：busy な participant がいる間は時計が止まる）．`Drop` で release して join する．これで `set_time` と read-back の間に real time が時計を進めない（`e - 1 tick` の比較が意味を持つ）．
- テストは `FakeConfig { clock: ClockMode::Manual, command_queue: 64, .. }` で rig する（`drift_ppm` は既定の 0 の fake だけ：`now` の drift 項が 0 でないと `set_time` の等式が崩れる）．**command_queue を 64 にする理由**：clock を止めたまま fake の queue（既定 16）を超える apply を出すと，fake の `apply` が `host.sleep(1 ms)` で止まった時計を待ち，テストが hang する（`device.rs:1023-1045`）．queue を広げると誤実装は log の差で落ちる．
- 各テストは `device.set_time(t)` の直後に `assert_eq!(device.time_now().unwrap(), t)` を置く（2 つの時計が一致していることの assert）．

**時計を止めたまま release するか，進めてから release するか**：`set_time` の直後に `time_now` の等式を assert するテストは，device の host clock に `Frozen` を保持する（S1・S1-TX・S2・S4・S4b・S5・S8b・S9(c)(e)・S12・S16〜S18）．S1・S1-TX・S2・S4・S4b の 1 回目・S12(a)(b)・S16〜S18 は device を 0 のまま release する（`effective == e`）；read-back の前に `set_time` で進める（S8b・S9(c)(e) も同じ）．S5 は 51 ms に進めてから release する（到着時の適用）．S4b の 2 回目は 11 ms に進めてから release する（残った command が到着時に適用される）．S3・S6・S8・S9(a)(b)(d) は `set_time` を呼ばない（ログと記録だけで見る）．

### 2.2 `targets`

```rust
/// UR-24: the channels a timed command of `chan` (`None`: every channel of `dir`) is given
/// on at `t` — those in force there, or, with none in force, those of the segments planned
/// to begin at or after `t` that are not refused. The flag: the direction's own channels
/// were in force (a live stream), which decides the `applied` row's shape.
fn targets(&self, dir: Dir, chan: Option<usize>, t: i64) -> (Vec<usize>, bool)
```

- live：layout は `channel_live(dir, c, t)` なら `(vec![c], true)`，そうでなければ pending を見る．current keys は `channels_at(dir, t)` が `n > 0` なら `((0..n), true)`，そうでなければ pending を見る（`channels_at` `:533-538`，`channel_live` `:544-549`）．
- pending：`lock(&core.streams).planned(booked_stream(dir))`（`core.rs:311-323`）から，`!p.refused`（`core.rs:319`）かつ `p.segment.origin >= t` の segment を選ぶ．layout はその channel を含む segment があれば `([c], false)`，無ければ `(vec![], false)`．current keys はそれらの `p.segment.config.channels` の和（`Channels::iter` で昇順，`BTreeSet` で重複を除く）で，空なら `(vec![], false)`．
- `targets` の中で `streams` を取るのは 1 回だけ（`channels_at`/`channel_live` を呼んだ**後**に，`planned` のために取る．入れ子にしない）．device の呼び出しの間も持たない（UR-25）．

### 2.3 `release`（`control.rs:619-678`）

1. `effective` を今のとおり `max(e, released.max)` で決める（`:632-633`）．
2. `let (chans, live) = self.targets(timed.dir, timed.chan, effective)`．
3. `chans` が空：今の記録だけの枝（`:642-651`）から `changed_projections`・`changed()` を消し，`config.insert` と `at: e`・`note` の行（`issued` なし）だけにする．device は呼ばない．`released` にも入れない．
4. `self.released.len() + chans.len() > depth` なら，その command を `held` に戻し（`self.held.insert((e, seq), timed)`），**この pass を終える**（`break`）．`depth` は `self.core.mode.timing().command_queue_depth`．`released.len()` は頭の `retain`（`:625`）の後の，channel ごとの，effective を過ぎていない数．
5. 空でなく slot がある：今の live な枝（`:653-677`）と同じ経路で，`Some(effective)` の apply，`released.push(effective)`（channel ごと），`issued()`，`effective > e` なら `late_command`．行は `live` なら今のまま（`at: effective`，`issued`），`!live` なら `at: e`，`note`，`issued`（D1）．apply の間は今の timed の枝と同じく `projections` とその `updates` を持ち，`assert_line_free("a timed command's release")` を置く（UR-25）．`streams` は apply の間持たない．
6. `Err`：その `(e, seq)` を **全投影の `updates` から除き**（`updates.remove(&(e, seq))`，`cancel_held` `:691` と同じ），`device_failed` で return（部分適用の後に続きを当てない）．`config` にも行にも書かない（裁定 §1.2）．lost の `Err` は今のとおり `device_failed`（`device_lost`）で return し，投影は触らない（device はもう無い）．

`BREAK` のとき `held` に戻した command は，次の pass で `effective` を計算し直す（`released` が retire していれば `effective = e` になり，`e` を過ぎていれば `Some(e)` で出て device が到着時に適用する）．新しい late の分岐は作らない．**borrow**：今の `first_entry()` のまま `targets(&self)` を呼ぶと `held` の可変 borrow と衝突するので，先に `entry.remove()` で `timed` を所有し（`e, seq` は先に写す），上限なら `self.held.insert((e, seq), timed)` で戻す．

### 2.4 `book_timed`（`:553-612`）

- layout：今のとおり `pending = held.len() + released.len()`，`cost = 1`（`:594-599`）．
- current keys：`pending = held.iter().map(|((e, _), t)| self.targets(t.dir, t.chan, *e).0.len()).sum() + released.len()`，`cost = self.targets(dir, chan, e).0.len()`（command 自身の `e`）．
- `COMMAND_QUEUE_FULL` は booking のときだけ（`:600-605` のまま）．booking の会計は booking の時点の `targets` の snapshot で，その後の cold 変更で増えうる（§8）；**device の queue を実際に超えないことを保証するのは 2.3 の release の上限**である，と §8 に書く．

### 2.5 版

`ColdConfig.version`（`:68`），`version()`（`:104-106`），`changed()`（`:111-115`），`configure` の戻りの版（`:151-170`），`Planned::version()`（`:198-200`），`changed_projections`（`:489-497`），`owner_step` の `version` 引数（`core.rs:329-341`），`configured` の `(seq, version)`（`core.rs:172`，`record_configured` `:378-380`），`owner_configure` の `next.version()`（`core.rs:820`）と `record_configured`（`:829`），`book_cold` の `version`（`:874`，`:948`），`cancel_held` の `version.fetch_add`（`:693`）を消す．`configured` は `Option<u64>`（seq）．`configure` は `Result<i64, DeviceError>`（decimation だけ）を返す．`Streams.version`（plan の版，`tx.rs:214`）は別物で残す．`use std::sync::atomic::{AtomicU64, Ordering}`（`:8`）は `AtomicU64` を消して `Ordering` だけ残す（`:1096` が使う）．版を説明しているコメント（`control.rs:65-67`，`:644-645`，`:683-685`，`core.rs:169-171`，`:325-328`）も合わせる．

### 2.6 test 専用の道具

- `FakeDevice::set_time`，`read_back`，`test_support::Frozen`（§2.1）．
- `ApplyFails`（`test_support.rs:341-372`）：`attempts: Mutex<Vec<(Dir, usize, Option<i64>)>>` を持ち，`apply` の入口で，失敗の判定と inner の呼び出しの**前**に `(dir, chan, at)` を積む．口は `new`（今のまま，untimed・not lost），`timed`（timed・not lost），`timed_lost`（timed・lost：`DeviceError { lost: true, .. }`）．`chan: Some(n)` はそのまま効く．`attempts()` をテストが読む（S9）．
- `owner_configure` の `next.configure` が `Ok` を返した直後・`record_configured` の前に `self.probe("configured", None)` を 1 行（test build だけ）足す（`core.rs:817-837`）．S3 が使う（`Core::probe` `core.rs:559-569`）．
- 既存の `channels_from_none`（`:1392`），`owner_rx`（`:1403`），`plan_of`，`time.advance_to`，`device.calls()`，`LockSpy`（`test_support.rs:199`），`Gate`，`probe`（`:155`）を使う．`sleep` は使わない．

## 3. 系列

形式：テスト名／置く場所／assert（1〜2 個）／観測／落とす誤った実装．すべて `provider/control.rs` の `mod tests` か `provider/tx.rs` の `mod tests` に置く（integration の `tests/fake.rs` には置かない：時計を止める道具は crate の unit test からしか使えない）．

### S1（RX，主系列）
- **名前**：`ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`（`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（`:1283`）を置き換える）．
- **進め方**：`control(&core, &[])`，`rx.channels = 0`．manual clock の fake，`Frozen`．`channels_from_none(&mut control, &core, 2, 100, 1)`（from-none の enable；uhd-control が open・configure する）．`plan_of` から **実際の** `segment.origin` を `O` として読む（from-none の enable の segment なので `O = 100 ms`，`e` の 50 ms より後）．`owner_rx(&core).begin()` を先に呼ぶ（owner は `rx_start(0, O)` を出し，記録が一致するので configure しない）．その後に gain 3.0 を `at = 50 ms` で記帳し，`time.advance_to(47 ms)` で `release()`（device の時計は 0 のまま）．
- **assert**：(1) `apply rx 0 … gain=3` は timed でちょうど 1 回，`at=<e> effective=<e>`（`at=-` ではない；from-none の configure の apply は別）．(2) `device.set_time(O + core.ticks(1_000_000))` の後に `device.time_now()` がその値と等しい と `read_back(Dir::Rx, 0).gain == Some(3.0)`；行は `at == e`，`note == "no channel"`，`issued == true`，`control.released == [e]`．
- **観測**：`device.calls()`，`read_back`，`core.rec.applied`，`control.released`，`plan_of` の origin．
- **落とす誤り**：untimed でその場に当てる（`at=-` になる），記録だけの枝を残す（apply が出ない），`Some(e)` でなく `None` で出す，live を `e` ではなく origin で判断する（live の枝に落ちて行が `note` なしになる），`targets` を `channels_at` のままにする（pending の channel を見ない）．

### S1-TX（送信）
- **名前**：`ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment`．`tx.rs` の `mod tests`（`spy_rig` `tx.rs:1926`，`control_of` `:920`，`tx_channels_from_none` `:928`，`plan_of` `:940`）．
- **進め方**：`control_of(&core)`（layout でない，`Start { t0: 0, rx: None, tx: None }`），`tx_channels_from_none(&mut control, &core, 2, 100, 1)`（from-none）．`plan_of` から `O`（TX の start lead）．`Tx::new(core.clone(), 0, cmds, None, Channels::first(0))` で `read(None)` と `step()` を 1 回ずつ（`retarget` が empty の segment を飛ばすので switch は enable の segment；`owner_configure` は `Nothing` で `tx_open`/`apply` を出さない）．その後に gain 3.0 を 50 ms で記帳し，47 ms で release．
- **assert**：(1) `apply tx 0 … gain=3 … at=<e>` が timed で 1 回，owner の switch の後に `at=-` の apply が増えない．(2) `set_time(O + core.ticks(1_000_000))` の後，`read_back(Dir::Tx, 0).gain == Some(3.0)`．
- **観測**：`spy.log()`／`device.calls()`，`read_back`，`plan_of`．
- **落とす誤り**：RX だけ直す（TX の枝が記録だけのまま），untimed にする，channel を streamer の中の位置にする（`0` は device の channel）．

### S2（owner がまだ configure していない）
- **名前**：`ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure`．
- **進め方**：`control(&core, &[])`，`rx.channels = 0`，manual clock，`Frozen`．`streams.stream(rx).idle = false` にして，`rx.channels = 1` を 100 ms に記帳する（from-none にならず，owner が後で configure する pending の segment）．gain 3.0 を 50 ms で記帳し，`time.advance_to(47 ms)` で release（記録の `configured` はまだ `None`）．その後に `owner_rx(&core).begin()`．
- **assert**：(1) release の timed apply が 1 回で `gain=3`（記録を見て判断しない）．(2) owner の configure が同じ channel に `at=-` で `gain=3` をもう 1 回当てる（2 度当てる）；`set_time(O + core.ticks(1_000_000))` の後 `read_back(Dir::Rx, 0).gain == Some(3.0)`．
- **観測**：`device.calls()`，`streams.stream(rx).configured`，`read_back`，`plan_of`．
- **落とす誤り**：`configured` を見て「owner がまだだから記録だけ」にする（timed apply が 1 回も出ない），owner の configure を省く，release が `configured` を書いてしまう．

### S3（configure の戻りと記録の公開の間）
- **名前**：`ur_24_a_release_during_a_configure_s_return_reaches_the_device`．
- **進め方**：`control(&core, &[])`，`rx.channels = 0`，manual clock（read-back はしない）．`idle = false` にして `rx.channels = 1` を 100 ms に記帳する（owner が configure する）．`owner_configure` の `configure` の `Ok` の直後・`record_configured` の前に probe `"configured"` を置き，test はそれを「main が合図するまで待つ」closure に差し替える（`probe(&core, "configured", …)`，`control.rs:2344` と同じ形）．owner を device の host clock の participant で走らせ（`tx.rs:2040` と同じ `host.spawn`），`begin()` を進める．probe に入ったら main が `configured == None` を assert し，gain 3.0 を 50 ms で記帳して 47 ms で release し，合図を返す．
- **assert**：(1) device の log は「configure の `at=-`（古い値）」→「timed の `at=<e>`（gain=3）」の順で，timed はちょうど 1 回（device の時計は 0 のままなので `effective == e`）．(2) probe の中の `configured == None`，join の後は `Some(by)`，segment は `rx_start(O)` で始まる．
- **観測**：`device.calls()` の順，`streams.stream(rx).configured`，`plan_of`．
- **落とす誤り**：release が記録を読んで「まだ一致しない」から apply を省く，release が `configured` を先に書く（probe の assert が落ちる）．
- **注**：probe は configure の完了後なので，「`updates` を持たない誤実装」でも configure→release の順にはなる（Sol 8）．この系列が固定するのは「記録を公開する前の窓でも device に届くこと」だけである．timed の枝の `updates` の lock は既存のままで，S10 と `ur_24_the_device_queue_order_is_the_effective_order`（`:1851`）が覆う．S3b（`LockSpy` の gate だけの系列）は作らない（裁定 §2.7）．

### S4（queue の会計：booking）
- **名前**：`ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots`．
- **進め方**：`rx.channels = 0`，`channels_from_none(..., 100, 1)`（pending の segment は {0}）．`at = 2.0 + 0.05·i ms`（`i = 0..8`）と `at = 10.0 + 0.05·i ms`（`i = 0..8`）で gain 1〜16 を記帳し，その後に 17 個目を 10.5 ms で記帳する（booking で `cost = 1`，`pending = 16`）．`release()` を now = 0 で 1 回（最初の 8 個），`time.advance_to(7.5 ms)` で 2 回目（次の 8 個）．device は 0 に止め，最後に `set_time(11 ms)`．
- **assert**：(1) 17 個目は booking で `COMMAND_QUEUE_FULL`（`depth = 16`）が 1 回，`rejected` の行が 1 つ，`held.len() == 16`；`released` は 1 回目の後に 8 個，2 回目の後に 8 個．(2) timed の apply は 16 回で，log の順は gain 1…16（instant の昇順），`at=<e>`；`set_time(11 ms)` の後の `read_back` は 16.0．
- **観測**：`events` の `radio.COMMAND_QUEUE_FULL`，`core.rec.rejected`，`device.calls()`，`control.released`，`held`，`read_back`．
- **落とす誤り**：cost を `channels_at` のまま（0）にする（17 個目が通る），`released.push` を pending の枝でしない（会計が 0 のまま），`released` を channel ごとに積まない，release を `held` の逆順・`released` の後に出す（最後の値が 16 でない），記録だけの枝で release を打ち切る（2 回目の 8 個が出ない）．

### S4b（release は device の queue を depth より多く埋めない；裁定 §1.1）
- **名前**：`ur_24_a_release_never_fills_the_device_queue_past_its_depth`．
- **進め方（a：Sol 2 の反例）**：`control(&core, &[])`，`rx.channels = 0`．**pending の segment が無いまま** gain 1〜17 を `at = 10.0 + 0.05·i ms`（`i = 0..17`）で記帳する（各 booking の `cost = 0`，`pending = 0`：channel も pending も無い）．その後に `rx.channels = 1` を 100 ms に記帳する（`idle = false` の enable；pending の segment は {0}）．`time.advance_to(7.9 ms)` で `release()`（device は 0）．
  - **assert**：(1) 1 回目の pass の後，timed の apply は 16 で，`released.len() == 16`，`held.len() == 1`（17 個目），`COMMAND_QUEUE_FULL` は 0．(2) `set_time(11 ms)` と `time.advance_to(11 ms)` の後に `release()`：17 個目が `at=<10.8 ms> effective=<11 ms>`（到着時）で出て，計 17，`held` は空，`read_back` は 17.0．
- **進め方（b：live が 1→2 に増える）**：`control(&core, &[Dir::Rx])`（channel 0 は 0 から live）．gain 1〜16 を `at = 10 ms`（同じ instant，seq 順）で記帳する（booking の `cost = 1`，16 個で満杯）．その後に `rx.channels = 2` を 9.8 ms に記帳する（booking の後なので cost は増えない）．`time.advance_to(7.9 ms)` で `release()`．
  - **assert**：(1) 1 回目の pass は 8 個 × 2 channel = 16 apply で，9 個目は `held` に残る．(2) `time.advance_to(11 ms)` と `set_time(11 ms)` の後に `release()`（残り 8 個）で計 32 apply，`read_back(Dir::Rx, 0)` と `(Dir::Rx, 1)` は最後の gain 16.0，`COMMAND_QUEUE_FULL` は 0．
- **観測**：`device.calls()`，`control.released`／`held`，`events`（`COMMAND_QUEUE_FULL` が 0），`read_back`．
- **落とす誤り**：release の上限を外す（(a) で 17 個，または (b) で 32 個が 1 回目に出る），`released.len()` を channel ごとに数えない（(b) で 16 個出る），上限で `break` せずに続ける（後ろの command が追い越す），上限で `held` に戻さず捨てる（17 個目が出ない）．
- **注**：fake の `command_queue` は 64 にする（§2.1）．上限の正しさを fake の back-pressure（hang）ではなく log の数で見るためである．実機の queue の容量は UR-9 の INFERRED のまま（§8）．

### S5（遅れた release は live な枝のとおり）
- **名前**：`ur_24_a_release_after_its_instant_applies_on_arrival`．
- **進め方**：`control(&core, &[Dir::Rx])`（channel 0 は 0 から live），manual clock，`Frozen`．gain 3.0 を `at = 50 ms` で記帳し，**release せずに** `time.advance_to(51 ms)` まで進め，`device.set_time(51 ms)`（`time_now == 51 ms` を assert）で `release()`．
- **assert**：(1) `LATE_COMMAND` は 0 回；device の log は `apply rx 0 … at=<e> effective=<51 ms>`（device の時計が `e` を越えているので到着時），`read_back` は 3.0．(2) 行は `at == e`，`issued == true`，note なし．
- **観測**：`events`，`device.calls()`，`core.rec.applied`，`read_back`．
- **落とす誤り**：release の遅れに新しい `LATE_COMMAND` を足す，`at` を `now` にする，apply をやめる（値が届かない）．
- **注**：`effective` の正確な値は**時計を止めているから**言える（51 ms の participant が busy）．実機では `effective` は到着の tick で，`e` より後であることだけが言える（§8）．

### S6（pending の segment が無い）
- **名前**：`ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing`．
- **進め方**：`control(&core, &[])`，`rx.channels = 0`，enable を記帳しない．gain 3.0 を 50 ms で記帳し，`time.advance_to(47 ms)` で release．
- **assert**：(1) device の log に apply が 1 つも無い．(2) 行は `at == e`，`note == "no channel"`，`issued` が無い；`control.config[gain] == 3.0`；`released` は空．
- **観測**：`device.calls()`，`core.rec.applied`，`control.config`，`released`．
- **落とす誤り**：pending を確かめずに device を呼ぶ，行の `at` を `now`/`effective` にする（VH-8 の比較対象），`issued` を付ける，`config` への記帳をやめる（後続の enable が値を落とす）．

### S7（layout：前の pending は持たず，後の pending が持つ）
- **名前**：`ur_24_a_layout_release_reaches_the_later_segment_s_channel`．`rig_on_mode` と layout の `Description`（2 つの RX front end，`keys: "channel"`）．`control(&core, &[Dir::Rx])`（初期の segment は `Start` の {0}；c1 の既定は `stream = -1`，`channels.rs:958-960`）．
- **進め方**：`radio.rx.c1.stream = 0` を 100 ms（join），`= -1` を 200 ms（leave），`= 0` を 300 ms（rejoin）に記帳する（segment はおよそ 155 ms に {0,1}，255 ms に {0}，355 ms に {0,1}）．`radio.rx.c1.gain_db = 3.0` を **240 ms** で記帳し，**237.5 ms** で release する（window 内）．240 ms で c1 は live でなく（200 ms の leave），pending は 255 ms の {0}（c1 を持たない）と 355 ms の {0,1}（持つ）．
- **assert**：(1) `apply rx 1 … gain=3 … at=<e>` が 1 回．(2) `apply rx 0 … gain=3` が 0 回．
- **観測**：`device.calls()`（`chan` は論理 channel の番号），`plan_of` の各 segment の `channels`．
- **落とす誤り**：最初の pending segment だけを見て「持たない」と判断する（apply が出ない），command の channel を見ずに segment の全 channel に当てる（c0 に出る），streamer の位置で当てる，`refused` を見ない（S8 と重なる）．

### S8（拒否された segment；TX と RX）
- **名前**：`ur_24_a_release_for_a_refused_transmit_segment_applies_nothing` と `ur_24_a_release_for_a_refused_receive_change_has_no_candidate`．
- **進め方（TX）**：`control(&core, &[])` の config で `radio.tx.channels = 0` にし（`control.config.insert`），`idle = false` にして `radio.tx.channels = 1` を 100 ms に記帳する（pending の segment，origin 100 ms）．`by` を `plan_of` から読み，`refuse(&core, booked_stream(Dir::Tx), by)` で拒否にする（TX は拒否された segment も plan に残る，`timeline.rs:868-884`）．gain 3.0 を 50 ms（`e < origin`）で記帳し，`time.advance_to(47 ms)` で release．後に拒否されていない enable を置かない．
- **進め方（RX）**：同じ並びで，`by` を `plan_of` から読んでから `refuse` する（RX の拒否は plan から除かれる，`timeline.rs:875-879`）．gain 3.0 を 50 ms で記帳し，`time.advance_to(47 ms)` で release．
- **assert**：両方とも (1) apply が 1 つも無い．(2) 行は `at == e`，`note`，`issued` なし．
- **観測**：`device.calls()`，`core.rec.applied`，`plan_of`（TX は `refused == true` の segment が残る；RX は残らない）．
- **落とす誤り**：TX で `refused` を見ない（`!refused` を `true` にすると S8-TX が apply を出す），RX で拒否を「channel を持つ」と数える．

### S8b（release の後に宛先が拒否される；裁定 §1.3）
- **名前**：`ur_24_a_release_stays_when_its_segment_is_refused_after_it`．
- **進め方**：`control(&core, &[])`，`rx.channels = 0`，manual clock，`Frozen`，`idle = false`．`rx.channels = 1` を 100 ms に記帳し（pending の segment，origin 100 ms），gain 3.0 を 50 ms で記帳し，`time.advance_to(47 ms)` で release（timed 1 回）．その後にその segment の `by` を `refuse(&core, booked_stream(Dir::Rx), by)` で拒否にし，`set_time(51 ms)`，`time.advance_to(51 ms)` でもう一度 `release()`．
- **assert**：(1) `read_back(Dir::Rx, 0).gain == Some(3.0)`（device は effective で適用した），apply は 1 回のまま，`COMMAND_REJECTED` は 0．(2) 行は `at == e`・`note`・`issued` のまま；`time.advance_to(52 ms)` の後の `release()` で `released` が空になる（effective を過ぎて retire）．
- **観測**：`device.calls()`，`read_back`，`core.rec.applied`，`control.released`，`events`．
- **落とす誤り**：拒否で release 済みの apply を取り消す・行を消す，`released` を retire しない（queue の会計が残る），拒否を「持たない」と読んで apply をやめる（(1) の apply が 0 になる）．

### S9（loss と apply の失敗：a〜e）
- **名前**：`ur_29_a_lost_device_applies_no_released_update`，`ur_29_a_failed_released_update_is_a_device_failure`，`ur_29_a_partly_applied_release_keeps_the_projection`，`ur_29_a_lost_device_between_a_release_s_channels_applies_no_more`，`ur_24_a_refused_release_takes_its_update_out_of_the_projection`．
- **進め方**：
  - (a) S2 の並び（0 channels → `idle=false` の enable を 100 ms，owner はまだ）で，device を `LockSpy` にし，`core.device_lost("the test's loss")` を release の前に呼ぶ．release の後に `spy.seen("apply") == 0`（attempt が無い）と `core.is_lost()`．`LockSpy` は `apply` を数える前に `assert_line_free` を呼ぶ（`test_support.rs:258-260`）．
  - (b) 同じ並びで `ApplyFails::timed(inner, Dir::Rx, None)`．release の timed apply が唯一の apply．`time.advance_to(47 ms)` で release．
  - (c) 同じ並びで enable を 2 channels（`channels_from_none` ではなく `idle=false`）にし，`ApplyFails::timed(inner, Dir::Rx, Some(n))` を **`n = 1`（最後）と `n = 0`（最後でない）の 2 例**で回す．channel 0 と 1 は `rx.channels = 2` の enable の segment の channel．release の後に `owner_rx.begin()` で configure させる．
  - (d) 同じ 2 channels の並びで `ApplyFails::timed_lost(inner, Dir::Rx, Some(0))`（channel 0 を `lost: true` で落とす）．
  - (e) **from-none で configure 済み**の並び：`control(&core, &[])`，`rx.channels = 0`，`channels_from_none(..., 100, 1)`（uhd-control が open・configure，記録はその segment の seq）．gain 3.0 を `at = 50 ms` で記帳し，`ApplyFails::timed(inner, Dir::Rx, None)` で `time.advance_to(47 ms)` の後に release（拒否）．その後に `owner_rx.begin()`．
- **assert**：
  - (a) apply の attempt が 0，`core.is_lost()`，行なし．
  - (b) `COMMAND_REJECTED`（`kind: "update_parameter"`）が 1，`applied` の行なし，`attempts` は timed の 1 つだけ，投影の `updates` が空（`(e, seq)` が除かれた），`control.config[gain]` は更新前．
  - (c) `n = 1`：`attempts` は channel 0・1 の 2 つ；channel 0 の timed apply 1 回，channel 1 で拒否；`COMMAND_REJECTED` 1；`applied` の行なし；投影の `updates` が空；`set_time(51 ms)` の後に `read_back(0) == 3.0`，`read_back(1) == 0.0`；その後の configure が両方に `at=-` で **0.0** を当てる（投影の値）. `n = 0`：`attempts` は channel 0 の 1 つだけ（channel 1 を呼ばない）；`set_time(51 ms)` の後に device は両方 0.0．
  - (d) `DEVICE_LOST` が 1，`core.is_lost()`，`attempts` は channel 0 の 1 つだけ（channel 1 を呼ばない），行なし．
  - (e) `attempts` は「enable の configure の `at=None`」と「timed の `at=Some(e)`」の 2 つ；拒否の後 `begin()` は configure せず（`rx_open`/`apply` が増えない），`configured` は `(by)` のまま，`set_time(O + core.ticks(1_000_000))` の後に `read_back(0) == 0.0`（device も投影も更新前）．
- **観測**：`device.calls()`，`core.is_lost()`，`core.rec.rejected`／`applied`，`control.held`／`config`，`events`，`ApplyFails::attempts()`，`LockSpy::seen("apply")`，`plan_of` の投影の `updates`，`read_back`．
- **落とす誤り**：先頭の loss の検査を消す（(a) で attempt が出る），lost の検査を apply の後に置く，apply の `Err` を捨てる（`device_failed` を呼ばない），失敗の後に `config.insert` と行の記録をする，投影から除かない（(e) で後の configure が 3.0 を当てる），部分適用の後に次の channel を当てる（(c) の `n=0` で channel 1 の attempt が出る），loss の後に次の channel を呼ぶ（(d) で channel 1 の attempt）．

### S10（lock）
- **名前**：`ur_25_the_line_lock_is_free_inside_every_control_device_call`（既存，`:2041`）を拡張．
- **進め方**：既存の `LockSpy` の経路（manual clock）に，S1 の release（pending の segment への timed の apply）を足した第 2 の rig を置く．`LockSpy::enter` の `assert_line_free`（`test_support.rs:151-153`）が，`updates` を持ったまま device を呼んでいないことを見る（`updates` は `core::lock` ではない）．
- **assert**：(1) spy が timed の apply を 1 回見た．(2) panic しない（line の guard を持ったまま device を呼んでいない）．
- **観測**：`LockSpy::seen("apply")`，`log()`．
- **落とす誤り**：`streams` を持ったまま apply する．対照（別 thread が line を持つ）は既存の `ur_25_a_device_call_passes_while_another_thread_holds_the_line`（`:2315`）．

**S11**（版を消す判断と既存テストの分類）は §4 と §9．

### S12（release 済みの command の後ろ；`at` と行の 3 欄）
- **名前**：`ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`．`control.rs` の `mod tests`（`ur_24_the_device_queue_order_is_the_effective_order`（`:1851`）の隣，µs で組む）．
- **進め方 (a)**：`control(&core, &[Dir::Rx])`（channel 0 は 0 から live），manual clock，`Frozen`．
  1. gain 1.0 を `at = 10 000 µs` で記帳し，7 500 µs で release（timed，`released = [10 000]`）．
  2. `rx.channels = 0` を 9 000 µs に記帳（cut），`rx.channels = 1` を 100 000 µs に記帳（`idle=false` の pending の enable）．
  3. gain 2.0 を `at = 9 700 µs` で記帳し，7 600 µs で release（`effective = max(9 700, 10 000) = 10 000`）．
- **進め方 (b)（`e` と effective の間の cold 変更）**：(a) の 1 の後に，`rx.channels = 2` を 9 800 µs に記帳する（gain 1.0 の release の**後**なので，その apply は 1 channel のまま）．gain 2.0 を 9 700 µs で記帳し，7 600 µs で release（`effective = 10 000`）．10 000 µs の config は 2 channels なので，2 channel に当たる．
- **assert (a)**：(1) gain 2.0 の apply は timed で `at=10000`（device の `effective=10000`），1 つ目の timed の apply の**後ろ**の log 位置；`released == [10 000, 10 000]`；`LATE_COMMAND` が 1 回（requested 9 700，applied 10 000）．(2) pending の行は `at == 9 700`，`note == "no channel"`，`issued == true`；投影の entry は `(10 000, seq2)` に動き，`set_time(10 001 µs)` の後の `read_back` は 2.0；`plan_of` の pending の segment の設定も 2.0．
- **assert (b)**：gain 2.0 は `at=10000` で **channel 0 と 1 の 2 回**当たり，行は `at == 10 000`・note なし・`issued`．`targets` を `e`（9 700）で引くと channel 0 だけになるので，回数で落ちる．
- **観測**：`device.calls()` の `at=`/`effective=` と順，`control.released`，`events` の `LATE_COMMAND`，`read_back`，`plan_of` の `updates` の key．
- **落とす誤り**：untimed でその場に当てる，`issued()` を呼ばない（投影の entry が 9 700 のまま），`effective` でなく `e` で apply する（`at=9700`），`released.push` をしない，行の `at` を `effective` にする，`targets` を `e` で引く（(b) で 1 channel）．
- **根拠**：UHD の一般 timed queue は in order（`timed_cmd.dox:94-115`，Sol の引用，VERIFIED）．F' は timed なので，同じ queue の順序が投影の順序（instant，seq）と一致する．untimed の apply を含めた順序の模型は fake と違う（§8）．

### S13（halted：既存テストの期待を変える）
- **名前**：`ur_24_a_direction_halted_by_a_refusal_has_no_channel`（既存，`:1780`）を書き換える．
- **何が変わるか**：halt した方向に 300 ms の enable（拒否されていない，origin は 300 ms 以上）があり，gain 3.0 が 200 ms に due する．F' では `targets` がその pending の segment の channel を返すので，199 ms の release で **`e` の timed の apply が 1 回**出る（今の assert は `usize::from(!halt)`，つまり halted では 0）．
- **新しい assert**：(1) halted でも stopped でも `apply … gain=3 … at=<200 ms>` が 1 回．(2) 行は halted が `at = 200 ms`・`note == "no channel"`・`issued == true`，stopped が live なので note なし・`at = 200 ms`；`control.config[gain] == 3.0`；`plan_of` の 300 ms の segment の設定は gain 3.0．
- **観測**：`device.calls()`，`core.rec.applied`，`plan_of`，`control.config`．
- **なぜ `!halted` を targets に足さないか**：halt は「timed の command をすぐ出さない」理由であって「値を後で持つ segment に届けない」理由ではない．enable の configure は結局その値を当てる（既存のコメントの「either way the 300 ms segment's configuration holds the gain」）ので，timed で先に届けても device の最終状態は変わらない．条件を足すと release がもう 1 つ（halted）の判断を持ち，規則が 2 つになる．
- **落とす誤り**：halted を「持たない」と扱う（apply が出ない），`halted` を `channels_at` と別に読んで食い違わせる，untimed で出す（`at=-`）．

### S14（seq だけの同一性）
- **名前**：`ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for`．
- **進め方**（裁定 §2.8 の順）：`control(&core, &[])`．booking の順を明記する：from-none の enable を 100 ms（{0}）→ gain 1.0 を 100 ms → `rx.channels = 0` を 200 ms → gain 2.0 を 300 ms → from-none の enable を 300 ms（{0}）．100 ms の configure は gain 0.0（gain 1.0 はその後に記帳される），300 ms の configure は gain 2.0（gain 1.0 と 2.0 を held で読む）．**`mark = device.calls().len()` を 2 度目の from-none の configure の後・`owner_rx(&core).begin()` の前に置く**．
- **assert**：(1) `mark` の後に owner の configure の `at=-` が 1 回出て，それは `gain=1`（1 つ目の segment の値）で，`gain=2` ではない；`rx_open` は出ない（集合 {0} は一致する）．(2) `begin()` の**前**の記録は `Some(by2)`（2 度目の enable の seq）で，`begin()` の**後**は `Some(by1)`（1 つ目の enable の seq；集合は同じで command が違うので configure した）．
- **観測**：`device.calls()` の `mark` 以降，`streams.stream(rx).configured`，`plan_of`．
- **落とす誤り**：`configured.is_some()` だけで省く（別の command の configure を信じる；apply が出ず device は gain=2 のまま），集合の一致だけで省く（gain が 2 のまま），結果の値でなく apply の回数だけで読む．

### S15（stop と owner の境界；**次の段階へ送る**）
- **系列**：owner が `Planned` を取った後（`rx.rs:279-304` の `next`，TX は `tx.rs:718` の `to`）に `Provider::stop` が来て，`streams.stop` が立ち，`Provider::stop` が `end` を記帳して `cancel_held` する（`mod.rs:688-695`）．owner は保持した `next` で `owner_configure` を実行しうる（`rx.rs` はその間に stop を再確認しない）．
- **期待（今の設計）**：`end` の cut/plan が優先し，origin が stop 以降の segment は clock を持たない（VH-2，`core.rs` の `reconcile`）．`cancel_held` が消した更新は `updates.remove` で投影から消えるので，owner の configure はその値を当てない（INFERRED：gate のテストは無い）．`Provider::stop` は **uhd-control → 各 TX → 各 RX の順に join する**（`mod.rs:695-704`）ので，uhd-control の join が返っても RX の configure はまだ走りうる；「plan の end だけで守る」とは断定しない．
- **提案**：この段階ではテストを足さず，1b-4d-2 へ送る．理由：(a) 版を消す根拠は「読み手がいない」ではなく §4 の 3 点で，この窓は版が防ぐものではない（Sol 7），(b) この段階の変更（F' と版の削除）はこの窓の挙動を変えない，(c) 1b-4d-2 は stream ごとの thread と stop の join を広げるので，そこで gate を 1 つ置けば同じ系列を固定できる．**INFERRED**：`rx.rs`/`tx.rs` の stop の再確認を足すかは次の段階の判断．

### S16（RX：古い segment がまだ流れている間の release）
- **名前**：`ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant`．
- **進め方**：`control(&core, &[Dir::Rx])`（channel 0 は 0 から live，gain の初期値は fake の 0.0），manual clock，`Frozen`．`rx.channels = 0` を 100 ms，`rx.channels = 1` を 200 ms に記帳する（古い segment は 100 ms で cut，再開の segment は 200 ms）．gain 3.0 を `at = 100.1 ms` で記帳し，**97.5 ms** で release（`channels_at(100.1) == 0`，pending は 200 ms の segment）．
- **assert**：(1) 3 点の read-back：`set_time(core.ticks(100_000_000) - 1)`（cut の直前）と `set_time(core.ticks(100_100_000) - 1)`（`e` の直前）は 0.0，`set_time(core.ticks(100_100_000) + 1)`（`e` の直後）は 3.0；各点で `time_now` の assert．(2) apply は timed の `at=100.1 ms` が 1 回．
- **観測**：`device.calls()`，`read_back`（設定の状態だけ），`plan_of` の cut/origin．
- **落とす誤り**：F の untimed で 97.5 ms に当てる（cut の直前が 3.0 になる），`e` より前（99 ms など）に当てる（`e` の直前が 3.0），apply をやめる（`e` の直後が 0.0），`channels_at(e)` を cut で判断する．
- **注**：read-back は**設定の状態**であって，古い stream のサンプルではない．「最後のサンプルまで古い」とは主張しない（裁定 §2.2）．

### S17（TX：同じ）
- **名前**：`ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant`．
- **進め方**：`control(&core, &[Dir::Tx])`（TX は 0 から live，clock あり），manual clock，`Frozen`．`radio.tx.channels = 0` を 100 ms，`= 1` を 200 ms．gain 3.0 を 100.1 ms，release は 97.5 ms．TX の plan は拒否でない segment を保つ．
- **assert**：S16 と同じ（3 点で 0.0／0.0／3.0；timed の apply は `apply tx 0 … at=100.1 ms` が 1 回）．
- **観測**：`device.calls()`，`read_back(Dir::Tx, 0)`，`plan_of` の clock/cut．
- **落とす誤り**：RX だけ直す，TX の release を streamer の `reading` と混ぜる（device の apply は streamer に触らない），untimed にする．

### S18（layout：leave／rejoin）
- **名前**：`ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant`．
- **進め方**：layout（2 RX front end，`keys: "channel"`）．`control()` の `Start` は {0} なので，**`Start { t0: 0, rx: Some((Channels::first(2), 200)), tx: None }` と `control.config` の `radio.rx.c1.stream = 0`** で作る（c1 の既定は -1，`channels.rs:958-960`）．`plan_of` の初期の segment が {0,1} であることを確かめる．`radio.rx.c1.stream = -1` を 100 ms，`0` を 200 ms に記帳する．`radio.rx.c1.gain_db = 3.0` を 100.1 ms で記帳し，97.5 ms で release（`channel_live(1, 100.1) == false`，pending の 255 ms の segment は c1 を持つ）．
- **assert**：(1) `apply rx 1 … gain=3 … at=100.1 ms` が 1 回，`apply rx 0 … gain=3` は 0 回．(2) `read_back(Dir::Rx, 1)` は cut の直前（99.999 ms）0.0，`e` の直前（100.099 ms）0.0，`e` の直後（100.101 ms）3.0；各点で `time_now` の assert．
- **観測**：`device.calls()`，`read_back`，`plan_of` の各 segment の `channels`（c1 を離れる前と後）．
- **落とす誤り**：leave した channel の古い segment に untimed で効かせる，layout の全 channel に当てる（c0 に出る），c1 の番号ではなく streamer の位置で当てる．

### 退けた案（この段階で採らない，1 行ずつ）
- **F（release のその場で untimed に当てる）**：古い segment が流れている間に効き（Sol 1），UHD では untimed の command も先行する timed command の後で実行される（Sol 2）ので，投影と device が逆になる．
- **origin を時刻にした timed command**：origin まで後ろの command を止める（同じ slot の RX/TX が 1 つの radio block の command の列を共有する）．
- **更新の `e` を pending の segment の origin まで切り上げる**：記録する instant が変わる（`applied` の `at` と投影の entry が origin になる）ので，Mock（MR-33）と spec 07 の `e` の意味を両方変える．
- **始めた owner が origin まで版を見直す**：origin を過ぎた後の窓が残り，owner の loop（`rx.rs` の `begin` と `tx.rs` の `read`）が変わる．版を消すこの段階では成立しない．
- **queue の容量を booking のときだけ守る**：booking の後の cold 変更で当たる channel が増えると，device の queue を超える（Sol 2，S4b）．release の上限を足す．
- **失敗した release を投影に残す**：後の configure が拒否された値を当ててしまう（Sol 4，S9(e)）．投影から除く．

## 4. 版を消せるか

**判断：消せる**（`ColdConfig.version`，`version()`，`changed()`，`changed_projections`，`Planned::version()`，`configured` の `(seq, version)`，`owner_step` の版，`configure` の戻りの版）．根拠は「読み手がいない」ではなく，次の 3 点と，裁定 §1.2 で失敗の経路が閉じたことである．

- **版を上げる場所は 2 つだけ**：`release` の記録だけの枝（`control.rs:646-648`）と `cancel_held`（`:686-695`）．
- **(a) 投影を device に知らせずに変える経路が無い**：F' で記録だけになるのは **live も pending の segment も無い**ときだけで，そのとき e 以後に channel を持つ segment は plan に無い（`targets` が空）ので，投影を device に知らせずに変えても，その値を必要とする owner は存在しない．後に enable が来れば，その configure が `config_at`（`control.rs:775`）と held の更新（`:916-925`）から値を当てる（S6 が固定する）．pending の segment があれば device に timed で届く（S1・S2・S3）．**失敗した release は投影から除く**（§2.3 の 6，S9(e)）ので，device に知らせずに残る値はない．部分適用では device が取った channel に値が残るが，投影は古い値で，後の configure はそれを当てる（裁定 §1.2）．S14 が「別の command の configure を信じない」ことを固定する．
- **(b) `cancel_held` の呼び出し元は `finish` だけ**：`finish`（`control.rs:340-344`）は `run` が `streams.stop.is_some()` を見たとき（`:319-321`）にしか呼ばれない．`streams.stop` を立てるのは `Provider::stop`（`mod.rs:691`）だけである．stop の後はどの owner も新しい segment を始めない（`ur_26_no_segment_begins_once_provider_stop_has_begun`，`rx.rs:1237`）．`start` の後も `streams.stop` は残るので同じである．
- **(c) `configured == Some(by)` は集合も一致するときにだけ configure を省く理由になる**：`owner_step` は `DeviceSet::Open(open) if open == set` を先に見て，`configured == by.zip(version)` を後に見る（`core.rs:329-341`）．版を消すと `configured == Some(by)` になる（S14）．
- **残すもの**：`cancel_held` の `updates.remove`（投影から cancelled の値を消す）と `applied` の `cancelled` 行（`ur_24_stop_cancels_held_commands`，`tests/fake.rs:1547`），`Streams.version`（plan の版，`tx.rs:214` が読む別物）．`configured` は `Some(seq)` だけ．
- **Sol 7 の反例**（owner が `Planned` を保持した後の stop）は版が防ぐものではない（保持した `next` の `settings` は `updates` を共有するので，`cancel_held` の `remove` は configure の読む前に効く，INFERRED）．この窓は S15 に書き，次の段階へ送る．

## 5. `design/` の変更（DA-1 の形）

`design/18-uhd-radio.md` の diff として，実装の gate で入れる．規範だけを書き，理由はこの計画と Changes の行に置く．**行の長さ**：UR-24 の `*Test:*` は今 1 452 文字（`:254`）で，足す名前は約 1 980 文字なので，足すと 3 400 文字を超える．`,\n  ` で **3 行**に分ける（UR-17 の `*Test:*` の 228–229 行と同じ形）．分けた各行は 1 500 未満にする（`rules.py` の LONG）．UR-36（`:270`）と UR-29（`:287`）は 1 行のままで 1 500 未満に収まる（§5 の追加分で約 1 200 文字）．

### UR-24（`:250` の第 1 段落の最後の文）
置き換える文（今）：

> Under the layout the key is a numbered channel's (`radio.<d>.c<n>.{frequency_hz,gain_db}`, RM-27): the command acts on channel `n` alone and takes one queue slot, and one for a channel the stream does not run at its instant is recorded and applied by the segment that later holds it (UR-25).

新：

> Under the layout the key is a numbered channel's (`radio.<d>.c<n>.{frequency_hz,gain_db}`, RM-27): the command acts on channel `n` alone and takes one queue slot, and one for a channel the stream does not run at the instant it takes effect is given to the device there, on that channel of the segments planned to begin at or after it that run the channel and are not refused; with none of them it is recorded, and the segment that later holds the channel applies it (UR-25).

### UR-24（`:251` の Holding and release の第 1 文）
今：

> It holds the command in its own list, ordered by `e`, and releases it — `apply(dir, chan, settings, Some(e))` on every channel of the direction (`set_command_time(e)`, the setting, `clear_command_time`), under the control-call mutex (UR-3) — when `e` is no more than §3's release window ahead.

新：

> It holds the command in its own list, ordered by `e`, and releases it — `apply(dir, chan, settings, Some(the instant it takes effect))` on every channel it is given on (`set_command_time`, the setting, `clear_command_time`), under the control-call mutex (UR-3) — when `e` is no more than §3's release window ahead.

### UR-24（`:252` の depth の文と最後の文）
今（depth）：

> With UR-9's `command_queue_depth` device commands — one per channel of its direction — held or released and not yet in effect, a new one that would exceed them is refused with `radio.COMMAND_QUEUE_FULL { key, depth: the profile's command_queue_depth }` and not applied (INFERRED: whether that many UBX tunes fit the device queue without blocking).

新：

> With UR-9's `command_queue_depth` device commands — under the current keys one per channel of the direction, under the layout one per command — held or released and not yet in effect, a new one that would exceed them is refused with `radio.COMMAND_QUEUE_FULL { key, depth: the profile's command_queue_depth }` and not applied (INFERRED: whether that many UBX tunes fit the device queue without blocking); a release that would leave more than that in effect waits in the list, and a later pass gives it to the device, which applies it on arrival once its instant has passed.

今（最後の文）：

> Each released command is recorded in `applied` with `e`; the value is the one admission already coerced.

新：

> Each released command is recorded in `applied`: one given on the channels in force at its instant, at that instant, `issued`; one given on a later segment's channels, at its own, with `note: "no channel"` and `issued`; the value is the one admission already coerced.

### UR-24（`:253` の第 3 段落）
今（全文）：

> A command is released on the channels of its direction in force at the instant it takes effect — its own, or the released command's it is late behind; a `cold` change booked for a later instant does not count yet — and one whose direction has no channel then, or is halted by a refusal at or before that instant (RM-25), counts no channel against the depth (counted at its own instant while held), is recorded in the configuration, at its own instant, and applies nothing; enabling the direction applies it (UR-12, UR-25). A refusal booked after a command was released does not take it back. A `Stop` cancels no command; `Provider::stop` cancels those still held (UR-26).

新：

> A command is released on the channels of its direction in force at the instant it takes effect — its own, or the released command's it is late behind; a `cold` change booked for a later instant does not count yet. When none is in force — no channel at all, or a refusal halting the direction at or before that instant (RM-25) — it is released at that instant on the channels of the segments planned to begin at or after it that are not refused — under the layout, its own channel when one of them runs it —, and under the current keys the depth counts those channels, at its own instant while held. With no such segment it is recorded in the configuration at its own instant and applies nothing; under the current keys it counts no channel against the depth at its release; enabling the direction applies it (UR-12, UR-25). A command given on a later segment's channels is recorded at its own instant with `note: "no channel"` and `issued`. A release the device refuses, other than by a lost device, is reported as a refusal, records no `applied` row, and takes its entry out of every configuration it was projected into, so no later configure applies it; the channels it reached keep the value the device took. A refusal booked after a command was released does not take it back. A `Stop` cancels no command; `Provider::stop` cancels those still held (UR-26).

`A command whose e precedes a command already released ...` の文はそのまま（`effective > e` の late は今の規則）．

### UR-24 の `*Test:*` の追記（design には英語で）
今の行の `ur_26_a_stop_cancels_no_update`（both unit）の後ろ，`;` の前に次を挿入し，長さに応じて **3 行**に分ける（§5 の冒頭）：

> `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` (one timed apply at its instant; the read-back after it), `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment`, `ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure` (the owner's configure applies the value a second time), `ur_24_a_release_during_a_configure_s_return_reaches_the_device` (the record is unpublished in the probe; the command still reaches the device), `ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots` (the first sixteen take slots; the seventeenth is `COMMAND_QUEUE_FULL`), `ur_24_a_release_never_fills_the_device_queue_past_its_depth` (the queue's depth bounds a pass; the command held by it goes on arrival), `ur_24_a_release_after_its_instant_applies_on_arrival` (no `LATE_COMMAND`; the device applies on arrival), `ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing`, `ur_24_a_layout_release_reaches_the_later_segment_s_channel` (channel 1 alone, the later segment's), `ur_24_a_release_for_a_refused_transmit_segment_applies_nothing` and `ur_24_a_release_for_a_refused_receive_change_has_no_candidate`, `ur_24_a_release_stays_when_its_segment_is_refused_after_it` (a refusal after the release takes nothing back), `ur_24_a_refused_release_takes_its_update_out_of_the_projection` (no later configure applies the refused value), `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order` (timed at the effective instant; the projection moved; the row at its own instant), `ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant`, `ur_24_a_release_after_a_transmit_channel_leaves_keeps_the_setting_until_its_instant` and `ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant` (the device's settings just before the cut and the instant, and just after), and `ur_24_a_direction_halted_by_a_refusal_has_no_channel` (its expectation changes).

### UR-36（`:264-270`，全文）
第 1 段落の `(seq, version)` を `seq` にし，第 4 段落と第 5 段落を次のようにする（全文）：

> - **UR-36** Each stream keeps one record of what its device holds (UR-13), as fields of its own state, written under the stream's line lock by whichever thread made the device call and read there by any other: the channel set its streamer is open on, the streamer's **open generation**, and the command that began the last configuration the device was given, `seq`.
>   The generation advances on every successful open of the stream, whoever made it — uhd-control's enable from no stream included —, so that a correlation against the replaced streamer's asynchronous reports is told from the new one's (UR-28).
>   An open that fails, or a rate comparison that refuses, leaves no set and no configuration known; a successful open leaves its set and no configuration known.
>   A configure whose every channel the device took, and whose read-backs UR-12 accepts, records its segment's command as the configuration the device was given, or no configuration where that segment's command is none, the stream's first; a configure that fails leaves no configuration known, its set unmoved.
>   A segment whose channel set the record holds open and whose command the record holds is not configured again (UR-17, UR-23); an update the device was not told as its own timed command is given to it as UR-24's release gives it.
>   Pruning changes no part of the record.
>   *Test:* `ur_13_a_failed_open_by_the_owner_is_opened_again`, `ur_13_a_failed_configure_is_configured_again_without_reopen`, `ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen`, `ur_13_a_partly_applied_set_is_configured_whole_without_reopen`, `ur_17_a_prune_keeps_the_device_record_and_the_configure_mark`, `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` (the record's command is the segment's and the value still reaches it as its own timed command), `ur_24_a_release_during_a_configure_s_return_reaches_the_device` (the record is unpublished; the command is still given), `ur_24_a_cancelled_held_update_leaves_the_projection` (a cancellation removes the projection's entry), `ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for` (the set matches and the command does not), `ur_25_a_refused_check_at_arm_leaves_the_set_unknown`, `ur_28_a_from_none_open_replaces_the_report_queue` and `ur_28_a_streamer_the_record_holds_keeps_its_correlations`; the owner's decision from the record by `ur_13_a_from_none_segment_the_owner_never_started_is_opened_again`, `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again`, `ur_25_an_owner_begins_a_from_none_enable_without_a_second_open` and `ur_25_the_first_segment_after_arm_does_not_reopen_the_streamer`.

**UR-17**（`:215`）：「unless the record holds that configuration (UR-36)」→「unless the record holds the segment's set and its command (UR-36)」．
**UR-23**（`:241`）：「unless the record holds that configuration (uhd-control's enable from no stream records both) (UR-36)」→「unless the record holds the segment's set and its command (uhd-control's enable from no stream records both) (UR-36)」．

**UR-29** の `*Test:*`（`design/18:287` の行）に，S9 の 4 本を**名前だけ**足す（この行の既存の形に合わせる）：`ur_29_a_lost_device_applies_no_released_update`，`ur_29_a_failed_released_update_is_a_device_failure`，`ur_29_a_partly_applied_release_keeps_the_projection`，`ur_29_a_lost_device_between_a_release_s_channels_applies_no_more`．S9(e) は UR-24 の Test に足した（規則は release の側）．

**UR-36 の Test から移す履歴**：v2 が UR-36 の Test に書いていた「(The old `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` and `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again` leave.)」は DA-1 の履歴なので，規範の Test 行には置かず，Changes の行に書く（裁定 §3・Sol 6）．

### §6 の表
- `:465`（`ur_24_a_direction_halted_by_a_refusal_has_no_channel`）：「halted: no `apply`, the gain recorded `no channel`」→「halted: the gain given to the device at 200 ms on the 300 ms enable's channel and recorded `no channel` at its instant」．stopped の欄はそのまま（issued）．
- `:466`（`ur_24_a_halted_direction_fills_no_queue_slot`）：「a halted direction counts no channel」→「a halted direction with no segment planned after the refusal counts no channel」．
- `:467`（`ur_24_a_late_update_halted_before_it_can_act_has_no_channel`）はそのまま（後に segment が無いので候補が無い）．
- `:468`（`ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant`）はそのまま（150 ms の更新の後に segment が無い）．

### Changes の行（`:614` の表の末尾に 1 行）

> | 2026-10-20 | UR-17, UR-23, UR-24, UR-36, and §6's `ur_24_a_direction_halted_by_a_refusal_has_no_channel` and `ur_24_a_halted_direction_fills_no_queue_slot` rows | a release uhd-control records with no channel in force is given to the device as a timed command at its own instant, on the channels of the segments planned to begin at or after it that are not refused; a pass leaves no more commands in effect ahead than the profile's depth and holds the rest for a later pass; it is recorded `no channel` with `issued`, and with no such segment it applies nothing and the enable applies it; a release the device refuses takes its entry out of every configuration it was projected into. The record's identity of a configuration is its segment's command and the set it is open on, so `ColdConfig`'s version and what read it go (a segment's configure is not repeated, and every other change reaches the device as UR-24 gives it); the two former tests `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` and `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again` leave, replaced by `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment` and `ur_24_a_cancelled_held_update_leaves_the_projection` | [Phase 8 design 33](../33-channels-and-streams.md) §5 stage 1b-4d-1c, this stage's brief and reviews |

### `--renames`（`rules.py` が引用の消えたテスト名で落ちるので，実装の gate で渡す）
- `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again -> ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`
- `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again -> ur_24_a_cancelled_held_update_leaves_the_projection`

`ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version` は `design/` に引用が無いので，消しても `--renames` は要らない（§9）．

### 他に偽になる文（grep：`version`，`record-only`，`record holds`，`no channel`，`applied`，`at that instant` を `design/` で）
- `design/18:613` ほか Changes の行の「a version moves on only where…」は**履歴**なので直さない（DA-1）．新しい行が版を消したことを書く．
- `design/18:291`（UR-30）の「every released command recorded as issued」は，D1 の `issued: true` で真のまま．ただし失敗した release は行を書かない（今も同じ）．この段階で新しく偽になる文ではないので変えない（UR-24 の新しい文が「reported as a refusal, records no `applied` row」を持つ）．
- `design/18:271-272`（UR-37 の「A refused receive change is left out of its stream's plan」）はそのまま（S8-RX が固定する）．
- `design/09-mock-radio.md:203`（MR-33）の「a change at `e` reaches samples at or after `e`」は**偽にならない**：F' の timed は `e` に効き，古い segment は `e` 以前に cut されている（S16〜S18 が設定の側で固定する）．ただし VH-8 の `updates()`（`parity.rs:685-706`）は `note` の行を除くので，**Manifest の比較はこの経路を覆わない**（§8）．
- `design/18:22`（timed command の VERIFIED）はそのまま．`design/18:58` の release window 3 ms / device lead 2 ms もそのまま（F' は timed なので，3 ms の window が 2 ms の lead より長いことが根拠）．
- `design/18:251` の「a retune released between a stream command's issue and its instant can precede it in the device's queue (INFERRED: whether stream commands share the device's command queue)」は，Sol の UHD 評価で RX の start が RX 専用の FIFO に行くと分かった（§8）が，TX と一般 queue の関係は INFERRED のままなので**変えない**（この段階の裁定に無い）．
- `design/18:185` の「`command_queue_depth` is counted by UR-24 as held plus released commands not yet in effect」は，UR-24 の新しい depth の文（current keys は channel ごと，layout は command ごと，release は上限で待つ）と整合する．変えない．

## 6. mutation の行の方針

新しい規則ごとに 1 行（`old` の文字列はコードと一緒に書くので，ここでは対応だけ）．

| 規則 | 行の当て先 | 落とすテスト |
|---|---|---|
| live が無ければ pending の segment の channel に出す | `targets` の pending の枝を空にする | S1，S1-TX，S7，S16〜S18 |
| pending は最初の 1 つではなく，channel を持つものを探す | pending の最初の segment だけを見る | S7 |
| 拒否された segment を除く | `!p.refused` を `true` にする | S8-TX |
| layout は command の channel だけ | `chan` の filter を外す | S7，S18 |
| timed で出す（untimed にしない） | `Some(effective)` を `None` にする | S1，S1-TX，S12，S16〜S18 |
| `released` に channel ごとに入れる | `released.push` を消す | S4，S4b，S12 |
| `issued()` で投影を動かす | `issued` の相当を消す | S12（投影の entry が 9 700 のまま） |
| `effective > e` なら `LATE_COMMAND` | 判定の `if` を消す | S12 |
| pending の行は `at: e`・note・`issued` | `at` を `effective` にする | S12 |
| pending の行に `issued` を付ける | `issued` を消す | S1，S12 |
| 空の枝の行に `issued` を付けない | 空の枝に `issued` を足す | S6 |
| 空なら `config` に記帳する | `config.insert` を消す | S6 |
| release の上限（`released.len() + chans.len() <= depth`） | 上限の `if` を消す | S4b(a)(b) |
| 上限で `held` に戻す | `insert` を消す（捨てる） | S4b(a)（17 個目が出ない） |
| 上限で pass を終える | `break` を `continue` にする | S4b（後ろの command が追い越す） |
| 失敗したら投影から除く | `updates.remove(&(e, seq))` を消す | S9(e)（後の configure が 3.0 を当てる） |
| 失敗したら `device_failed` で返る | `Err` の腕を `continue` にする | S9(b)(c)(d) |
| 部分適用の後に次を当てない | `apply` の `Err` の `return` を消す | S9(c) の `n=0`（channel 1 の attempt が出る） |
| `configured` は command だけで比べる | `configured == by` を `configured.is_some()` にする | S14 |
| `book_timed` の cost は `targets` の長さ（current keys） | `cost` を 0 にする | S4（17 個目が通る） |
| `book_timed` の held の和も `targets` | held の和を 0 にする | S4（17 個目が通る） |
| `targets` を `effective` で引く（release） | `effective` を `e` にする | S12(b)（channel 1 に当たらない） |

- **作らない行**：`origin >= e` の境界（`>` に変える行）．記録だけの枝に来るのは「live も pending も無い」ときだけではないが（S16〜S18 は pending がある），`origin == e` は到達しない（from-none は ready + start lead，普通の change は booking/e + lead）．`changed_projections` の `e <= origin` と同じ形にしておく．
- **消す行**：版（`version`，`changed`，`record_configured` の `(seq, version)`，`owner_step` の版）と `changed_projections` に当たる既存の行は，コードの削除と一緒に消す（`old` が一致しなくなるので残せない）．`version` を `Some(by)` の比較に変える行は，`new` の識別子が残っていないか grep する（gate の U41）．
- 修正の round は触った行と新しい行だけを流す（AGENTS.md §7）．全リストは段階の最後に `remote_mutate.py all sim02:8,sim03:8` で 1 回．

## 7. 大きさ（見積もり）

| 部分 | 行 |
|---|---|
| 本体（`targets`，`release` の枝・上限・失敗の除去，`book_timed` の会計） | 95〜125 |
| 版の削除（`ColdConfig`，`owner_step`，`record_configured`，`configure`，`changed_projections`，`book_cold`） | −45〜−60（足す行は 20 以内） |
| test 専用（`set_time`，`read_back`，`Frozen`，`ApplyFails` の 2 口と `attempts`，probe） | 60〜75 |
| S1，S1-TX | 100 |
| S2，S3（probe と thread） | 115 |
| S4，S4b | 130 |
| S5，S6 | 70 |
| S7，S8，S8b | 140 |
| S9（a〜e） | 150 |
| S10，S13 | 45 |
| S12（a・b） | 80 |
| S14 | 65 |
| S16〜S18（3 点の read-back） | 190 |
| 既存テストの書き換え（S11a，prune，cancel の書き直し，`rx_record`/`tx_record`，`mod.rs`/`rx.rs`/`tx.rs` の assert） | 70 |
| 合計 | **約 1,250〜1,480**（削除を除く） |

指示書の 1,500 行に収まる見込みだが，上限に近い．**S16〜S18 は動かさない**（早期適用の防止を挙動変更と同じ gate に置く；S16〜S18 だけを後に送る fallback は採らない，裁定 §2.10）．実装で 1,500 を超えるなら，orchestrator に次の順で提案する：(a) S4b の (b)（live 1→2）を落とす（上限・`break`・`held` に戻すは (a) が落とすが，「`released` を channel ごとに数える」は (b) だけが落とすので，落とすなら §6 のその行も作らないと明記する），(b) S12(b) を落とす（`targets` を effective で引く規則は S12(b) だけが落とすので，落とすなら §6 のその行も作らないと明記する），(c) S14 を次の段階（版の削除の確認）へ送る．(a)〜(c) でも超えるなら，段階そのものの分け方（例：`targets` と release の枝だけで 1 つの gate，その直後に残り）を orchestrator が決める．

## 8. 確かめられなかったこと（INFERRED）

- **timed の tune と stream の start の queue**：RX の timed start は RX 専用の command の列（register → RX FIFO）に行き，一般の timed tune の queue とは別である（Sol の UHD 評価：`U/host/lib/rfnoc/radio_control_impl.cpp:1116-1133`，`U/fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_rx_core.v:157-176,311-343`，VERIFIED）．TX の timed start-of-burst と一般 queue の関係は INFERRED のまま（`design/18:251` の親文）．S1・S1-TX はこの区別を仮定する．
- **front end の timed gain は stream を待たない**：X3xx の timekeeper は `sample_rx_stb` で動き，dboard への制御は `USE_TIME=1` で同じ `radio_time_tb` を使い，時刻の到達後に settings bus へ出す（Sol の UHD 評価：`U/fpga/usrp3/top/x300/bus_int.v:541-556,918-940,943-965`，`U/fpga/usrp3/lib/rfnoc/utils/ctrlport_to_settings_bus.v:184-203`，VERIFIED）．**実機での gain 変更の完了時刻は未測定**（INFERRED）．
- **frequency の DSP 側**：`set_command_time` は radio だけでなく DDC/DUC にも設定し（`U/host/lib/usrp/multi_usrp_rfnoc.cpp:964-986`），frequency は RF と DSP の両方に届き（同 `:1394-1404,2067-2077`），DUC の timed frequency は sample 側の時刻追跡を使い（`U/fpga/usrp3/lib/rfnoc/dds_timed.v:135-170`），timed FIFO の取り出しには timestamp 付き sample の転送が要る（`U/fpga/usrp3/lib/rfnoc/axi_tag_time.v:206-211`）．idle の DDC/DUC の timed frequency は，sample が来るまで消費されない（VERIFIED：Sol の引用）．よって「front end には `e` に効くが，DSP 側は最初のサンプルまで queue に残りうる」（INFERRED）．この段階のテストは **gain** で書く．**bench に送る系列**：idle 中の timed frequency → owner の untimed configure → 長い無通信 → 最初の timestamp 付き sample で，再開時の設定と残存 queue を見る（実機での最終的な誤設定・blocking は INFERRED）．
- **streamer を開いていない channel にも timed の apply が効く**（from-none の前，halted の後，leave の後）：UHD の mother board の setter は radio block の register で，streamer に依らない（INFERRED）．fake は `st.spec` だけで判定する（`device.rs:1008-1045`）．S7・S13・S16〜S18 はこれを仮定する．
- **release の遅れ**：`e` を過ぎた release は `Some(e)` で出し，device は到着時に適用する（`multi_usrp.hpp` の「If the time spec is late, the command will be activated upon arrival」，VERIFIED）．fake の `effective = at.max(last_command).max(now)`（`device.rs:1021-1041`）がその模型．S5 は，時計を止めているから `effective == 51 ms` を assert するが，**実機の到着 tick は測っていない**．
- **fake の untimed の apply は timed の queue を迂回する**（`device.rs:1047-1081`）．UHD の一般 queue は受信順で，untimed の command も先行する timed command の後で実行される（`timed_cmd.dox:94-115`，VerIFIED）．この段階では fake を直さない．S2 は「timed の apply → owner の untimed configure の順に **呼ばれた**」ことと，2 度目の値が同じであることだけを固定する（順序が逆でも最終値は 3.0）．S12 は fake の in-order queue の `at` の順だけを固定する．**差は INFERRED**（#62，bench B8）．VH-8 の Manifest の比較は `note` の行を除くので，この経路を覆わない．
- **queue の会計と booking の後**：`book_timed` は booking の時点の `targets` で数えるので，その後に予約された `cold` change で当たる channel が増えても，booking の cost は増えない（今と同じ）．だから **booking だけでは device の queue を守れない**（Sol 2）．守るのは release の上限（S4b）である．S4b は fake の `command_queue` を 64 にして，上限を log の数で見る（時計を止めたまま 17 個目を出すと fake が block して hang するので，back-pressure では観測しない）．実機の queue の容量は UR-9 の INFERRED のまま．
- **`read_back` の `settle`**：fake の timed queue を読む前に `settle` を呼ぶのは test の模型で，実機の read-back（`get_rx_gain` 等）は測っていない．S16〜S18 が見るのは**設定の状態**だけで，古い stream のサンプルではない．
- **`mark_lost` は fake を lost にしない**：`FakeDevice::mark_lost` は `calls` に 1 行足すだけで（`device.rs:1659-1661`），`lost()` は `FakeConfig::faults` の `Lost` だけを読む（`:779-787`）．Sol 3 の「`core.device_lost()` は inner fake も lost にする」はこの fake については成り立たない．S9(a) が `attempts`／`seen("apply")` で見るのは，この差に依存しないためである．
- **`origin == e` は到達しない**（§6）．`targets` の `>=` と `>` はテストで区別できない．
- **S15 の stop と owner の境界**：`rx.rs:279-304`/`tx.rs:718` が `Planned` を保持した後に stop が来る窓はあるが，`end` の plan と `cancel_held` の `updates.remove` が何を守るかは gate のテストで固定していない（次の段階）．join の順（`mod.rs:695-704`）は読んだが，RX の configure が uhd-control の join の後に走りうることは，テストで固定していない（INFERRED）．
- **`reading` を待たない**：timed の apply は control の呼び出しで，`UhdDevice` の control-call mutex（UR-3）の下にあり，streamer の mutex とは別（`design/18:116`）．よって uhd-tx の `tx_async` と並んでよい．テストは足さない（fake は streamer を持たないので観測できない）．
- **devices の lead と cut の実値**：S16〜S18 の 3 点は fake の模型（cut で止まり，timed は `e` に効く）による．実機の X310 の cut と lead は bench（B8）の値で，この段階では測らない．
- **`set_time`/`Frozen` は test の模型**：`time_now == at` の assert は時計を止めているから成り立つ．実機では device の時計は止まらない．

## 9. 既存テストの扱い（S11 の答え）

| テスト（場所） | 扱い | 理由 |
|---|---|---|
| `ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again`（`control.rs:1254`） | 残す（版の assert を外し，`configured == Some(by)` にする） | 規則は変わらない（timed で届く更新で configure し直さない） |
| `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again`（`:1283`） | **置き換え＋改名**（S1） | 規則が逆になった（owner は configure せず，device が timed で受ける） |
| `ur_24_a_cancelled_projected_update_makes_the_owner_configure_again`（`:1310`） | **書き換え＋改名**：`ur_24_a_cancelled_held_update_leaves_the_projection` | 版が消える．cancel が投影の entry を消すことと `applied` の `cancelled` 行を固定する（`ur_24_stop_cancels_held_commands` と重ならない範囲で，投影の `settings` が base に戻ることを見る） |
| `ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version`（`:1340`） | **消す** | 版の channel ごとの選択性だけを見ていた．`design/` に引用が無い．channel の filter は S7/S18 と既存の `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel` が持つ |
| `ur_17_a_prune_keeps_the_device_record_and_the_configure_mark`（`:1371`） | 残す（`Some((second, 7))` → `Some(second)`） | prune が記録を変えない規則はそのまま |
| `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant`（`:1758`） | 残す | 3 番目の 150 ms の更新の後に segment が無いので，記録だけのまま（S6 と同じ） |
| `ur_24_a_direction_halted_by_a_refusal_has_no_channel`（`:1780`） | **期待を書き換え**（S13） | halted でも pending の enable に timed で届く |
| `ur_24_a_halted_direction_fills_no_queue_slot`（`:1828`） | 残す（コメントを「後に segment が無い」に狭める） | 50 ms の 16 個は live で 1 ずつ，100/200 ms は後に segment が無いので 0．名前はそのまま（§6 の行を狭める） |
| `ur_24_the_device_queue_order_is_the_effective_order`（`:1851`） | 残す | live な枝の規則は変わらない（S12 はその隣） |
| `ur_24_a_late_update_halted_before_it_can_act_has_no_channel`（`:1877`） | 残す | 後に segment が無いので候補が無い |
| `ur_24_a_refusal_halts_only_its_own_stream`（`:1940`） | 残す | 拒否された方向に後の enable が無い（200 ms の gain は記録だけ） |
| `ur_25_the_line_lock_is_free_inside_every_control_device_call`（`:2041`） | 拡張（S10） | 新しい timed の経路を覆う |
| `ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`（`tests/fake.rs:4965`） | 残す | join は release の後に記帳されるので pending が無く，後で untimed に当たる（今のまま） |
| `ur_25_a_refused_enable_from_no_stream_halts_and_keeps_its_values`（`tests/fake.rs:6303`） | 残す | gain の release の時点で pending の segment が無い（拒否された enable だけ）．§8 の INFERRED．実行で確かめる |
| `ur_24_stop_cancels_held_commands`（`tests/fake.rs:1547`） | 残す | cancel の `applied` の行は変わらない（`updates.remove` だけ） |
| `ur_24_hardware_timed_updates`（`tests/fake.rs:1442`），`ur_24_a_far_future_update_does_not_delay_a_nearer_one`（`:1474`），`ur_24_the_queue_counts_a_command_per_channel`（`:1879`） | 残す | 会計は live の channel 数と `targets` で同じ（2 channels の 9 個目，rx 1 channel の 17 個目） |
| `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`，`ur_25_an_enable_at_a_timed_update_s_instant_applies_it`（`tests/fake.rs`） | 残す（実行で確かめる） | 300 ms の instant で channel は live（`channels_at` が 1）なので，今のとおり timed で出る |
| `rx_record`/`tx_record` の `configured`（`control.rs:2272`，`tx.rs:1952`），`mod.rs:810,826`，`rx.rs:1159,1165`，`control.rs:2390`，`tx.rs:2074` | 型を `Option<u64>` に合わせる | `(seq, 版)` が `seq` になる（`zip` と `|(by, _)|` を直す） |
| `control/differential.rs`（`rm_26_the_timeline_against_uhd_control`） | 残す（変更なしの見込み） | 系列は 2〜5 個の timed op で depth 16 に届かず，`applied` の note の行は今と同じ `at: e`（`:270-272`）．F' は行を変えない |
| `rx/differential.rs`（layer 3） | 残す（実行で確かめる） | release の device の apply が増えても，plan・clock・Manifest の比較対象は変わらない見込み |
| `parity.rs` の VH-8 | 残す（変更なしの見込み） | 系列に「live の channel が無い timed 更新」が無く，`note` の行は比較から除かれる（§8） |

## 10. 実装のあとの追記（裁定 3 と，実装で足したもの）

`adjudication-3.md` が最優先なので，`adjudication-2.md` §1.1 の release の上限と §1.2 の失敗の除去は**取らない**．v3 の §2.3 の 4・6 と S4b，§4 の「失敗は投影から除く」はこの追記で置き換える．

### 10.1 裁定 3 による変更

1. **release の depth の上限は無い**（裁定 3 §1.1）．`release` は booking の会計で決まった分をそのまま出す．booking の後に `cold` change で当たる channel が増えると，device の queue の深さを超えうる．その結果は device の back-pressure で uhd-control の apply が待つことで，待つ長さは queue の先頭の instant まで（おおよそ release window）．`design/18` の UR-24 に 1 文で書き，Changes に既知の制限として記録する．S4b は作らない．したがって §6 の行「release の上限（`released.len() + chans.len() <= depth`）」「上限で `held` に戻す」「上限で pass を終える」も作らない．
2. **失敗した release は live な枝と同じ**（裁定 3 §1.2）．`apply` が lost でない `Err` を返したら `device_failed` で return し，`config` にも行にも書かず，投影の entry は**残す**．後の configure がその値を当てうる（`COMMAND_REJECTED` が報告する）．S9(b) は entry が残ること，S9(c) は owner の configure が両 channel に値を当てることを固定する．S9(e) は「entry が残る・begin が configure しない・device は旧値・行も `config` も書かない」を固定し，名前を `ur_24_a_refused_release_records_no_row_and_keeps_the_projection` にした（以前の `..._takes_its_update_out_of_the_projection` は裁定 3 で偽になるので使わない）．§6 の行「失敗したら投影から除く」も作らない．
3. S9(e) の名指しする誤った実装は「失敗の後に行か `config` を書く実装」である（裁定 3 §1.2）．
4. 裁定 3 §1.3（release の後に宛先が拒否される）は残す → S8b は v3 のまま．

### 10.2 実装で確定したテスト（v3 §3 との対応）

| v3 の系列 | テスト（`provider/control.rs` の `mod tests`，S1-TX だけ `provider/tx.rs`） |
|---|---|
| S1 | `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`（v3 の `ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` を置き換え） |
| S1-TX | `ur_24_a_transmit_release_without_a_live_channel_reaches_the_pending_segment` |
| S2 | `ur_24_a_release_without_a_live_channel_precedes_the_owner_s_configure` |
| S3 | `ur_24_a_release_during_a_configure_s_return_reaches_the_device` |
| S4 | `ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots` |
| S5 | `ur_24_a_release_after_its_instant_applies_on_arrival` |
| S6 | `ur_24_a_release_without_a_live_channel_or_a_later_segment_applies_nothing` |
| S7 | `ur_24_a_layout_release_reaches_the_later_segment_s_channel` |
| S8 | `ur_24_a_release_for_a_refused_transmit_segment_applies_nothing`，`ur_24_a_release_for_a_refused_receive_change_has_no_candidate` |
| S8b | `ur_24_a_release_stays_when_its_segment_is_refused_after_it` |
| S9(a) | `ur_29_a_lost_device_applies_no_released_update` |
| S9(b) | `ur_29_a_failed_released_update_is_a_device_failure` |
| S9(c) | `ur_29_a_partly_applied_release_keeps_the_projection`（`n = 1` と `n = 0`） |
| S9(d) | `ur_29_a_lost_device_between_a_release_s_channels_applies_no_more` |
| S9(e) | `ur_24_a_refused_release_records_no_row_and_keeps_the_projection`（裁定 3 で改名） |
| S10 | `ur_25_the_line_lock_is_free_inside_every_control_device_call` に 2 本目の rig（pending の segment への timed の apply）を足した |
| S11 | 下表 |
| S12 | `ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order`（(a) と (b)） |
| S13 | `ur_24_a_direction_halted_by_a_refusal_has_no_channel`（期待を書き換え） |
| S14 | `ur_25_an_owner_configures_a_segment_the_record_s_command_is_not_for` |
| S15 | 次の段階（1b-4d-2）へ送る（v3 のまま） |
| S16〜S18 | `ur_24_a_release_after_a_receive_channel_leaves_keeps_the_setting_until_its_instant`，`..._after_a_transmit_channel_...`，`ur_24_a_layout_release_after_a_channel_leaves_keeps_the_setting_until_its_instant` |

- テスト専用の道具：`FakeDevice::set_time`・`read_back`（`device.rs`，`#[cfg(test)]`），`test_support::Frozen`，`ApplyFails` の `timed`・`timed_lost` と `attempts`，`Core::owner_configure` の probe `"configured"`．`set_time` の直後の `time_now` の assert は `manual_rig`/`manual_rig_events`/`layout_rig` と `device_at` の helper に寄せた．
- 既存のテスト：`ur_24_a_timed_update_after_a_from_none_configure_is_sent_without_configuring_again` は版の assert を `configured == Some(by)` に，`ur_17_a_prune_keeps_the_device_record_and_the_configure_mark` は `Some(second)` に，`ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking`・`tx.rs` の `tx_record` 系・`rx.rs` の `ur_13_a_partly_applied_set_is_configured_whole_without_reopen` は `Option<u64>` に合わせた．`ur_24_a_cancelled_projected_update_makes_the_owner_configure_again` は `ur_24_a_cancelled_held_update_leaves_the_projection` に書き換え，`ur_24_a_cancelled_update_of_another_layout_channel_leaves_the_version` は消した（版の channel ごとの選択性だけを見ていた）．
- **大きさ**：v3 の見積もり 1,250〜1,480 行に対し，実装の追加は 889 行（削除 197 行）：本体 76，テストと test 専用の道具 813．内訳は報告 §大きさ．
