STATUS: done

# Phase 8 段階 1b-3a — MockRadio の内部を stream ごとの状態に組み直す（挙動は変えない）

指示書: `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b3a/brief.md`．HEAD `5228a35` から．
自分の作業ログ（実行したコマンドと出力の写し）: `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b3a/impl.log`．

## 1. 変えたファイル

| file | 変更 |
|---|---|
| `crates/ezsdr-mock-radio/src/lib.rs` | 内部を stream ごとの状態（`RxStream`・`TxStream` の `Vec`）に組み直し，line を `timeline::Lines` に入れた．公開 API と観測できる挙動は不変（テストの diff は空）．+514/−358 行（コメントを含む） |
| `plan/maintenance/tools/mutations.json` | mock の 25 行のうち 6 行（F27・F29・F30・F32・J08・T26）を当て直し |
| `plan/phase3/tools/mutations.json` | mock の 57 行のうち **20 行**（M26・M27・M31・M32・M45・M47・M51・M63・M64・M65・M66・M68・M69・M70・M72・M75・M76・M80・M81・M82）を当て直し（M27 と M51 の 2 行で同じ 1 箇所） |
| `plan/phase4/tools/mutations.json` | mock の 5 行のうち 2 行（D09・D10）を当て直し |
| `plan/phase7/tools/mutations.json` | **変更なし**．唯一の mock 行 V05（`let origin = timeline::lattice(...)`）が新コードでそのまま1回一致するため |
| （`crates/ezsdr-radio/src/timeline.rs`） | **変更なし**（`Lines` に足したものは無い．§3） |
| （`crates/ezsdr-mock-radio/src/channel.rs`・`profile.rs`・`device.rs`・`time.rs`） | **変更なし** |

`git status --short`: 上の4ファイルのみ（未追跡のファイルなし）．テストは1行も変えていない（§5 の 7）．

要点:

1. 受信 stream 1本分の状態を `RxStream`，送信 stream 1本分を `TxStream` にまとめ，方向ごとに `Vec`（`k` が添字）で持つ（`MockRadio::streams`，`lib.rs:289-297`）．stream の node id（`<device>/rx`・`<device>/tx`），port の名前（`rx`），event の handle（`RX_OVERFLOW`），そして channel mode の受信モデル（`RxModel`）と送信 plan（`Arc<TxShared>`）も stream が持つ．
2. line は instance の `timeline::Lines` に入れ，`(Direction, k)`（`MockRadio::rx_key`/`tx_key`）で引く．記帳は `Lines::book`，読み出しは `Lines::line` を通す．`Line` を直接持つ field は instance にも stream にも無い．
3. 受信・送信の処理は stream 番号 `k` を引数に取る（`emit_rx_until(k, ..)`，`publish_rx(k, ..)`，`rx_settle(k, ..)`，`rx_begin(k, ..)`，`book_rx(k, ..)`，`tx_reconcile(k)`，`tx_switch(k, ..)`，`emit_tx_until(k, ..)`，`handle_tx_burst(k, ..)`，`stop_tx(k, ..)` ほか）．instance 全体の処理（profile，`config`，fault の予定，wakeup，`updates` の順序，`next_order`，Channel mode，`start`/`arm`/`stop`/`cleanup`）は instance に残した．`advance` は全 stream を番号順に回す（今は各方向1本なので順序は今と同じ）．
4. 今の key（`radio.{rx,tx}.channels` など）では方向ごとに stream は1本（`k = 0`）で，その config の channel 集合は `Channels::first(radio.d.channels)`（`MockRadio::line` は不変）．`radio.rx.channels = 0` でも stream の構造体は1本あり，channel が無いだけ（SampleClock を declare しない＝今の挙動どおり）．tree と port の名前は今のまま（`<device>/rx`・`<device>/tx`・port `rx`）．

## 2. field の表（どこへ置いたか）

### instance（`MockRadio` に残したもの）

| field | 中身・根拠 |
|---|---|
| `instance`，`source_root`，`profile`，`config`，`selector` | 今のまま（MR-4，MR-18） |
| `prepare_called`/`prepared`/`armed`/`started` | 今のまま（MA-7，MR-9，MR-11） |
| `actions`，`clocks`，`time`，`events`，`root` | 今のまま（MA-5a） |
| **`lines: timeline::Lines`（新）** | instance の全 stream の line，(Direction, k) で引く（RM-26，RM-34） |
| **`rx: Vec<RxStream>`（新）** | 受信 stream．`k` が番号（RM-29，MR-10） |
| **`tx: Vec<TxStream>`（新）** | 送信 stream．`k` が番号（RM-29，MR-10） |
| `faults`，`start_tick`，`sync_end`，`wakeup`，`device_lost_reported` | fault の予定と start/arm・停止の状態は instance のもの（MR-20，MR-9，MR-11，MR-25） |
| `updates`，`next_order` | UC-2 の順序と到着順は instance で一意（MR-18，RM-25） |
| `medium`，`channel` | SimulationChannel は instance に1つ．`ChannelMode.lo_retune` の引き順も2方向で共通（MR-30，MR-34） |

### `RxStream`（`k` 番目の受信 stream）

| field | 元（HEAD の field / 式） | 役割 |
|---|---|---|
| `node: ResourceId` | `self.instance.id.child("rx")`（3か所の都度生成） | SampleClock の stream id（RM-7 step 6）と `Stop`/`start_rx` の宛先（RM-21）．今は `<device>/rx` |
| `port: String` | literal `"rx"`（prepare の link 照合） | `ctx.links` をどの port で選ぶか（MR-7 step 7） |
| `overflow: Option<EventHandle>` | `overflow_handle` | `radio.RX_OVERFLOW` の handle（MR-37） |
| `clock: Option<SampleClockHandle>` | `rx_handle` | prepare で declare し最初の block で register（RM-7，RM-25） |
| `from: i64` | `rx_from` | まだ始まっていない segment の origin の下限（RM-25） |
| `running: Option<Rx>` | `rx` | 受信中の segment（RM-21） |
| `links: Vec<Arc<dyn DataLink>>` | `links` | この stream の port に付いた link（MR-7） |
| `pool: Option<HostPool>` | `pool` | block のバイト列（MR-13） |
| `rng: Option<SimRng>` | `rng` | `block_len_jitter` の generator，`<device>/rx`（MR-12） |
| `model: Option<channel::RxModel>` | `ChannelMode.rx` | channel mode の利得・周波数・位相・timed tune 定数（MR-33〜MR-35） |

`Rx` 構造体（受信中の segment：`handle`/`domain`/`origin`/`ratio`/`channels`/`next`/`planned`/`flags`/`lost`/`overflows`/`ended`）は今のまま（RM-21，RM-25，MR-19，MR-37）．

### `TxStream`（`k` 番目の送信 stream）

| field | 元（HEAD の field） | 役割 |
|---|---|---|
| `node: ResourceId` | `self.instance.id.child("tx")`（2か所の都度生成） | SampleClock の stream id（RM-7 step 6）と `TxBurst` の宛先（RM-15）．今は `<device>/tx` |
| `clock: Option<SampleClockHandle>` | `tx_handle` | prepare で declare し `arm` で register（MR-9） |
| `clocks: Vec<TxClock>` | `tx_clocks` | 登録した送信 clock，plan の segment の順（RM-25，VH-2） |
| `cur: Option<usize>` | `tx_cur` | 今の送信 clock（MR-18） |
| `later: Vec<(usize, HeldBurst)>` | `tx_later` | 計画中の変更が始める clock に記帳した burst（KC-21a） |
| `tracker: Option<BurstTracker>` | `tx_tracker` | 今の clock の tracker（MR-15） |
| `device: DeviceModel` | `tx_device` | 今の clock の device の framing モデル（MR-15，MR-24） |
| `held: BTreeMap<i64, HeldBurst>` | `held` | 保持した burst（RM-15） |
| `open: Option<OpenBurst>` | `open_tx` | 送信中の burst（MR-15） |
| `plan: Option<Arc<channel::TxShared>>` | `ChannelMode.tx` | channel mode の送信 plan（MR-32）．medium に渡したのと同じ Arc |

`TxClock`，`HeldBurst`，`OpenBurst`，`PendingUpdate` は今のまま．`ChannelMode` は `medium`・`fragment`・`inputs`・`lo_retune` だけになり，`rx`/`tx` は stream へ移した．`WorkKind` は `HeldBurst` と `RxBegin` が stream 番号を持つ（`Fault`・`Update` は instance のものなので持たない）．

## 3. `Lines` に足したもの

**何も足していない．** `Lines::new`・`insert`・`line`・`book`（1b-2b で入った既存の API）だけを使った．`Line` の読み出し（`plan`，`stream.config`，`made`，`segment`）は `Lines::line(key) -> Option<&Line>` 越しに行う．

唯一 `Lines` に無い操作は「line を落とす」で，これが要るのは (a) `start` が失敗して巻き戻すとき（`self.rx_line = None` 相当），(b) `cleanup` の2か所だけ．指示書は `Lines` への**読み出しだけ**の追加を許しているので，削除 API は足さず，どちらも `Lines` を作り直して代えた（(a) では `arm` が先に記帳した送信の line を clone して入れ直す．`lib.rs:1915-1922`）．この段階は move を1つも記帳しないので，作り直しで失うものは無い（`moves`・`floors` は空）．1b-3b で move を記帳するようになったらこの2か所は見直しが要る（§8）．

## 4. 指示書から外れたところ

なし．判断が要ったのは次の3点で，いずれも指示書の範囲で最も素直な形にした:

1. `Lines` に削除が無いため，line を落とす2か所は `Lines` を作り直した（§3）．
2. fault（`RxOverflow`・`RxSequenceError`）の適用は，今は受信 stream が1本なので `apply_rx_fault(0, index)` を直に呼ぶ（`lib.rs:1605`，コメントつき）．**fault がどの stream を切るか**は 1b-3b で複数 stream になったときに決める（fault の予定そのものは指示どおり instance に残した）．`advance` の候補の並び（`WorkKind`）は instance の fault をそのまま持つ．
3. event の source: stream ごとの handle（`RX_OVERFLOW`）は指示どおり stream が持たせ，`emit_overflow`/`emit_rx_overflow` は stream 番号を受ける（`lib.rs:464`，`608`）．instance の `emit_event(suffix, ..)`（MR-28 の `<root>/rx`・`<root>/tx`・`<root>`）は変えていない（今は suffix が stream の名前と一致する．1b-3b で `<device>/rx/s<k>` を名乗る形への追随が要る．§8）．

## 5. 確かめること 1〜7 の結果

| # | 何を | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | **64 test bin，1214 passed / 0 failed**（`error` 行 0）．既知の負荷弱テスト `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` は3回の実行とも落ちていない（1回で通ったので単独再実行は不要）．**最終のツリー**（lib.rs 23:12:14）で流した | `cargo-test-workspace-3.log`（初回 `cargo-test-workspace.log` と `-2.log` も同じ結果） |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **0 warning**（exit 0）．**最終のツリー**で流した | `impl.log` |
| 3 | Docker の `--features uhd` clippy（`ezsdr-radio-uhd`） | **exit 0**（cached．`ezsdr-mock-radio` は `ezsdr-radio-uhd` の依存ではないので再 build は起きない）．**最終のツリー**で流し直した | `docker-clippy-3.log` |
| 4 | Python（venv 3.14） | **47 tests OK**（6.3s，`Ran 47 tests ... OK`）．**最終のツリー**で `cargo build -p ezsdr-server` の後流した（server は同じツリーで build 済み） | `impl.log` |
| 5 | `rules.py --base HEAD` | **0 problem(s)**．`--self-test` も `ok` | `impl.log` |
| 6 | 当て直した mutation の行 | **88/88 killed**（§7） | `remote-mutate-3.log`（最終のツリー） |
| 7 | `git diff HEAD -- 'crates/*/tests/*'` | **空**（型名の追随も不要だった．`git status --short` も実装の4ファイルのみ） | `impl.log` |

（3 の Docker clippy が 0.2 秒で終わるのは，`--features uhd` の build が volume `ezsdr-v4-cargo-target` にキャッシュされており，変更した `ezsdr-mock-radio` が `ezsdr-radio-uhd` の依存でないため．`Finished` 行より前に警告・エラーは1行も出ていない．）

## 6. セルフレビュー（AGENTS.md §9 の手順2）

`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash --agent plan --auto "<チェックリストと指示書のパス>"` を，毎回**新しい**読み取り専用のレビュー役として nohup で起動し，`kill -0 <pid>` で終了を待った．

### 第1回（`self-review-1.log`，約13分）

指摘は P2 2件と nit 4件．「挙動が変わった到達可能な行は無い」「HEAD の field はすべて移設先が一意」「`Lines` に足したものは無い」「`design/09` で偽になる文は無い」は確認済みとされた．扱い:

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P2 | 到達不能な拒否経路を3つ削った（`self.instance.id.child("rx"/"tx")` の `Result` を構築時に `expect` したため，`MR-7:`/`MR-18:` の「ノードが作れない」拒否と文面が消えた） | **採った**（コード変更なし．`ResourceId::child` は再パースの失敗で，`instance.id` は検証済み1セグメント，`"rx"`/`"tx"` は正当なので HEAD でも到達不能．ここに記録する） |
| 2 | P2 | 「channel mode ⇔ `plan`/`model` が Some」の不変条件が2箇所（設定と cleanup）に分かれた | **採った**：`prepare` の設定箇所に，両者が同時に立ち cleanup が stream ごと作り直すので互いに生き残らない，と1コメント追加（→ 第2回の P2 で代入の順序も直した．現在は `lib.rs:1808-1817`）．`debug_assert` は足していない |
| 3 | nit | mutation バッチを流したリビジョンが最終と少し違う（F30/J08 の `new` 修正と F32/F29 の末尾改行の後） | **採った**：最終のツリーで 88 行を**もう一度全部**流した（`remote-mutate-2.log`．§7） |
| 4 | nit | workspace テストが最終 `lib.rs`（コメントのみ変更後）より前の版 | **採った**：最終のツリーで `cargo test --workspace` を流し直した（1214 passed，§5 の1） |
| 5 | nit | stream 0 のハードコード（`handle_action` の宛先照合，cold key → stream 0，`apply_rx_fault(0, ..)`，`apply_update` の `self.tx[0]`/`self.rx[0]`） | **採らない**（今は1本なので挙動は同一．1b-3b の作業として §8 に記録．`apply_rx_fault` の呼び出しにはコメントを付けた） |
| 6 | nit | `lo_retune` が instance に残る（MR-34 の引き順） | **採らない**：MR-34 の「その方向のチャンネルを更新適用順に引く」を満たし，指示書（Channel mode は instance）とも一致．複数 stream のときの引き順の定義は 1b-3b の宿題として §8 に記録 |

### 第2回（`self-review-2.log`，約20分）

第1回の修正後のツリーに対する，新しい読み取り専用のレビュー役．「挙動が変わった到達可能な行は無い」「誤配置なし」「28行とも同じ規則を同じ形で壊している」「`design/09` で偽になる文は無い」は確認済みとされた（reviewer 自身が `cargo test -p ezsdr-mock-radio` 94 passed と clippy 0 warning を実行し，`remote-mutate-2.log` の 88 killed と COMPILE ERROR 0 も確認）．指摘は P2 1件と nit 2件:

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P2 | 第1回で足した「channel mode ⇔ `plan`/`model` が Some」のコメントが `medium.join(...)?` の失敗経路では偽（`plan`/`model` を `join` の**前**に立てていたため，join が拒否すると `self.channel` は None のまま両者だけが残る）．今は観測できない（`prepare_called` が真，`root` 未設定で `stop`/`step`/`arm` は入れない）が，コメントが実在しうる経路で嘘になる | **採った**：2つの代入を `medium.join(...)?` の**後**に移した（`lib.rs:1814-1816`）．HEAD と同じく「join の拒否では何も残らない」に戻り，コメントもそのまま真になる |
| 2 | nit | 報告の phase3 の行数（20 と書いた）が実際（19）と違う | **採った**：本報告を 20 行（M75 を含む 28 行）に直した（§1，§7） |
| 3 | nit | 報告の「第2回」が未記入 | **採った**：この節 |

（付随の観察: transmit plan は stream ごとになったが `medium.join` に渡す `Transmitter` は instance に1つ．1b-3b で複数の送信 stream を持つときの medium 側の写像は §8 に記録．）

### 第3回（`self-review-3.log`，約17分）

第2回の修正後のツリーに対する，新しい読み取り専用のレビュー役．**P0/P1/P2 は無し**（「挙動が変わっていない」「状態の置き方に誤りなし」「28行とも同じ規則を同じ形で壊している」「`design/09` で偽になる文は無い」）．このレビュー役は自分でも `remote_mutate.py ... <当て直した28行の id>` を sim02/sim03 で流し，**28/28 killed** を独立に確認した（`review-mutate.log`）．指摘は nit 3件:

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | nit | 報告の「第3回」が未記入 | **採った**：この節 |
| 2 | nit | `RX_OVERFLOW` の handle を resolve する node が `source_root.child("rx")` の決め打ち（`lib.rs:1832`）．1b-3b で `<device>/rx/s<k>` になるときの追随先として §8 に挙げておくべき | **採った**：§8 item 7 に加えた（コードは今 `"rx"` で正しい） |
| 3 | nit（観察） | `cleanup` が旧の消していなかった `rng` も消す（stream ごと作り直すため）．`prepare` が必ず上書きし `prepared=false` で `step`/`arm` は入れないので観測差は無い | **採らない**（記録のみ．§7 の「当て直しで挙動が変わっていない」根拠の1つ） |

3回のレビューとも「挙動が変わった到達可能な行は無い」で一致したので，ここでセルフレビューを終える（指示書の「指摘が無くなるか3回」に該当）．

## 7. 当て直した mutation の行（28行，全部 killed）

`remote_mutate.py` で 88 行（`crates/ezsdr-mock-radio/src/*` の全行）を流した: `remote-mutate-3.log`（**最終のツリー**，lib.rs 23:12:14 / phase3 の JSON 23:12:53 のスナップショット，**88/88 killed**）．行の選別は `maintenance,phase3,phase4,phase7` × `sim02:8,sim03:8`．経緯（すべて `kill -0` で完走を確認）:

1. `remote-mutate.log`（当て直し直後のツリー）: **86/88**．F30 と J08 の **mutant が古い API を参照して COMPILE ERROR**（`old` は一致したが `new` が `self.rx` を `Option` として使い，`self.rx_register()`/`self.rx_cut()` を呼んでいた）→ 2行の `new` を当て直し，`remote-mutate-f30j08.log` で 2/2 killed．
2. `remote-mutate-2.log`（第1回セルフレビューの nit 3 への対応）: **88/88**．
3. `remote-mutate-3.log`（第2回セルフレビューの P2 への対応後）: **88/88**．これが最終のツリーの1リビジョンでの結果．`COMPILE ERROR`・`SURVIVED`・`NOT FOUND`・`BASELINE FAILED` の行は無い（`grep -icE "compile|surviv|not found|baseline"` → 0）．第3回セルフレビューのレビュー役も，当て直した28行だけを別途流して **28/28 killed** を確認している（`review-mutate.log`，レビュー役が独立に実行）．

| list | id | 当て直しの要約（old → new） | kill したテスト（`test_bin` / filter） |
|---|---|---|---|
| maintenance | F30 | `apply_rx_fault` の overflow 即時 emit: `self.rx_register()`→`self.rx_register(k)`，`self.rx.as_mut()`→`self.rx[k].running.as_mut()`，`emit_overflow(payload,..)`→`emit_overflow(k, payload,..)` | `mock_radio` / `mr_20a_the_overflow_comes_with_the_block_that_carries_it` |
| maintenance | F32 | 同じ規則を `rx_finish` で: `self.rx_settle(t)`→`self.rx_settle(k, t)`，`self.rx`→`self.rx[k].running`，`emit_overflow` に `k` | `mock_radio` / `mr_20_a_fault_at_a_loss_s_instant_removes_nothing` |
| maintenance | F27 | 損失で送信を止める: `self.stop_tx(now, reason, false)?;`→`self.stop_tx(k, now, reason, false)?;`（`if reason != DEVICE_LOST` の mutant） | `mock_radio` / `mr_20_a_lost_device_transmits_nothing_after_the_loss` |
| maintenance | F29 | 損失の stop が受信 stream を開いたまま: `self.book_rx(Item{..})`→`self.book_rx(k, Item{..})`（`if !lost` の mutant） | `mock_radio` / `mr_20_a_loss_ends_its_receive_stream_at_its_instant` |
| maintenance | J08 | 故障行の `lost` を cut で切る: `self.rx_cut()`→`self.rx_cut(k)` | `mock_radio` / `mr_21_overrun_shape` |
| maintenance | T26 | 損失の cut を1サンプル手前: `self.cut_segments(cut)`→`self.cut_segments(k, cut)`（`cut - 1` の mutant） | `mock_channel` / `ch_09_a_faulted_round_does_not_depend_on_the_stepping_order` |
| phase3 | M26 | stop の部分 block 抑止: `self.open_tx`→`self.tx[k].open`，`emit_tx_until(now,..)`→`emit_tx_until(k, now,..)` | `mock_channel` / `mr_25_a_stopped` |
| phase3 | M27 | stop が放射を切る: `self.cut_segments(cut)`→`self.cut_segments(k, cut)` | `mock_channel` / `mr_25_a_stopped` |
| phase3 | M31 | rx の timed tune が位相を保つ: `mode.rx.*`→`model.*` | `mock_channel` / `mr_34_a_timed_tune` |
| phase3 | M32 | tx の timed tune が位相を保つ（同じ1行，インデントを実体（32 空白）に合わせた） | `mock_channel` / `mr_34_a_timed_tune` |
| phase3 | M45 | segment を誤った origin で登録: `mode.tx.plan()`→`shared.plan()` | `mock_channel` / `mr_32_a_loopback` |
| phase3 | M47 | rx 利得無視: `mode.rx.gain_db`→`model.gain_db` | `mock_channel` / `mr_33_gain` |
| phase3 | M51 | cold change が放射を切る（M27 と同じ1箇所，`reason !=` の形） | `mock_channel` / `mr_32_a_cold` |
| phase3 | M63 | rx の timed tune が tx の定数を使う: `model.*` と `self.tx[0].plan` に追随 | `mock_channel` / `mr_34_a_timed_tune`（filter 空＝`mock_channel` 全体） |
| phase3 | M64 | tx の周波数 timeline 未更新: `plan.frequency_hz` のインデントを実体（28 空白）に合わせた | `mock_channel` / filter 空 |
| phase3 | M65 | stop の cut が1 tick 後: `self.transmit_until_cut(now,..)`→`(k, now,..)` | `mock_channel` / filter 空 |
| phase3 | M66 | rx 周波数 timeline を gate が無視: `mode.rx.frequency_hz`→`model.frequency_hz` | `mock_channel` / filter 空 |
| phase3 | M68 | stop が全 clock の segment を終わらせる: `map_or(k,..)`→`map_or(k_cut,..)` | `mock_channel` / `mr_32_a_stop_after_a_cold_change` |
| phase3 | M69 | 同（撤回）: `retain(... *start < k)`→`*start < k_cut` | `mock_channel` / `mr_32_a_stop_after_a_cold_change` |
| phase3 | M70 | cold change が segment 世代を保つ: `self.tx_cur`→`self.tx[k].cur` | `mock_channel` / `mr_32_a_stop_after_a_cold_change` |
| phase3 | M72 | open burst の次サンプルで burst を許す: `self.open_tx`→`self.tx[k].open` | `mock_channel` / `mr_16_a_burst_at_or_before` |
| phase3 | M75 | 結合の拒否で `MR-31:` の prefix を落とす: `format!("MR-31: {error}")`→`format!("{error}")`（`old` は `medium.join` の行から `Some(ChannelMode` まで） | `mock_channel` / `mr_31_channel_mode` |
| phase3 | M76 | stop が cut 手前の held burst を最大1つしか開かない: `self.open_tx`/`self.held`→`self.tx[k].open`/`self.tx[k].held`，`emit_tx_until(k,..)` | `mock_channel` / `mr_25_a_stop_transmits_every_held` |
| phase3 | M80 | open burst の判定を target で行う: `self.tx[k].open` 版 | `mock_channel` / `mr_16_a_burst_at_or_before` |
| phase3 | M81 | 次サンプルの1つ後を拒否: 同上 | `mock_channel` / `mr_16_a_burst_at_or_before` |
| phase3 | M82 | 次サンプルでの拒否を channel mode だけに: 同上 | `mock_channel` / `mr_16_a_burst_at_or_before` |
| phase4 | D09 | 注入した overflow を control path で出す: `self.emit_overflow(k, payload,..)`→`self.emit_event("rx", ..)` | `mock_radio` / `mr_37_the_overflow_travels_the_hot_path` |
| phase4 | D10 | back-pressure の overrun を control path で出す: 同（`emit_rx_overflow` 内の1行） | `mock_radio` / `mr_19_backpressure_is_an_overrun` |

- **`old` は全部ちょうど1回一致**（自作の検査と `rules.py` の `exactly once` の両方）．`new` は同じ規則を同じ形で壊している（差分は条件か値1つ）．`name`・`filter`・`test_bin` は1つも変えていない．
- **当て直さなかった行（61行）の `new`** が古い API（`self.rx.as_*`，`self.rx_handle`，`self.rx_line`，`self.tx_line`，`self.tx_clocks`，`self.tx_cur`，`self.tx_later`，`self.held`，`self.open_tx`，`self.overflow_handle`，`self.tx_tracker`，`self.tx_device`，`self.rx_from`，`self.pool`，`self.rng`，`self.links`，`mode.rx`，`mode.tx`）を参照していないことを全部検索して確認（0件．`self.rx_register`・`self.rx_cut` への誤マッチ3件は当て直し済みの F30/F32/J08）．コンパイルできない mutant は無い（88行すべてが killed，COMPILE ERROR は 0）．
- 新しい規則の追加は無いので新しい行も無い．既知の生き残り phase7/G09 は mock の行ではない（今回は mock の行だけを流した）．

## 8. 未解決のこと（1b-3b へ渡す）

1. **fault がどの stream を切るか**（`advance` の `apply_rx_fault(0, index)`，`lib.rs:1605`）．今は受信 stream が1本なので `0`．複数 stream のとき「そのときに走っている segment」をどの stream から選ぶかを決める（MR-21 の文は今のままでも読める）．
2. **イベントの source**（`emit_event` の suffix）．今は `""`（`<root>`）と `"tx"`（`<root>/tx`）．stream ごとの node 名（`rx/s<k>`・`tx/s<k>`）になる段で，suffix を stream から取る形にする．
3. **`Lines` の作り直し**（`start` の失敗時の巻き戻し `lib.rs:1915-1922` と `cleanup`）．move を記帳するようになったら，他の stream の move を落とす．`Lines` に「1本落とす」読み出し以外の操作を足すか，作り直さない形にするかを 1b-3b で決める．
4. **stream 0 のハードコード**（`handle_action` の宛先照合，cold key → stream 0，`apply_update` の `self.tx[0]`/`self.rx[0]`）．宛先が `<device>/rx/s<k>` を名指すようになる段で，宛先から `k` を決める形にする．
5. **`ChannelMode.lo_retune` の引き順**．instance に1つ（MR-34 の引き順は今のとおり rx/tx 共通）．複数 stream で「その方向の全 channel を適用順に」を守る形にするとき，stream をまたぐ引き順の定義が要る．
6. **`Lines::book` は毎回 `Lines` 全体を clone する**（1b-2b の実装）．今の Mock の記帳は `Stop`・`start_rx`・`cold` 変更・`End` だけなので問題にならないが，記帳が増えるときに見る．
7. 尺度の変更ではないが，`RxStream.port` は今 `"rx"`（1b-3b では `rx<k>`），`node` は `<device>/rx`（同 `rx/s<k>`）になる（D4）．`prepare` の link 照合と tree は 1b-3b で `channels` builder の tree に替わる．`RX_OVERFLOW` の handle を resolve する node（今は `source_root.child("rx")` の決め打ち，`lib.rs:1832`）も同じ段で stream ごとの node 名に追随する．
8. medium に渡す `Transmitter` は instance に1つ（今の `medium.join` は `self.tx[0].plan` の Arc）．1b-3b で複数の送信 stream を持つとき，medium 側がどの送信 stream を読むかの写像を決める（今回は該当なし）．
