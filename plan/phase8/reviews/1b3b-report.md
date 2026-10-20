STATUS: done

# Phase 8 段階 1b-3b — MockRadio の新しい key のモード（layout の builder，複数 stream，channel ごとの設定，D14 の宛先）

指示書: `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/brief.md`．HEAD `9b36a91` から．実行したコマンドとログは `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/` にある（ゲートのログは `*.log`，コードのハッシュは `code-hashes-before-doc-edits.txt`）．commit・push・`git add` はしていない（`git status` は下のファイルのみ）．

## 1. 変えたファイルと要点

| file | 変更 | 要点 |
|---|---|---|
| `crates/ezsdr-mock-radio/src/lib.rs` | 大きい（+…/−…，`git diff --stat` で 681 行の差分） | selector の `keys` で新旧の key set を切り分け，stream を layout の長さだけ持ち，宛先（D14）・channel ごとの `cold` の記帳・移動の拒否・`violations` の実行時判定・fault の stream 選択・event の source を新しいモードで正しくする |
| `crates/ezsdr-mock-radio/src/profile.rs` | 書き直し | `Profile` が2つの description を持つ（`Mode::{Current,Layout}`）：tree・coerce・defaults・envelope・stream の数・path・port を `Profile` が1か所で答える |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs` | 新規（13 tests，644） | 新しいモードの規則ごとのテスト（§7） |
| `crates/ezsdr-radio/src/channels.rs` | 小（+39/−9） | Mock が使う読み出しを公開（§2） |
| `crates/ezsdr-radio/src/lib.rs` | 小（+9/−6） | `Concrete`/`concrete` の公開と `radio_keys` の `pub(crate)` 化（§2） |
| `design/09-mock-radio.md` | 127 行の差分 | MR-38〜MR-41 を新設し，MR-2〜MR-22・MR-28〜MR-37・Scope・§4・§6・Changes を追随（§6，DA-1） |
| `design/07-radio-model.md` | 5 行 | RM-11 の event の source を「その stream の node」（RM-3，RM-29）に直し，Changes 行を追加 |
| `design/08-simulation.md` | 3 行 | SE-6 の「MockRadio の受信 jitter は `"<device>/rx"`」を stream ごとの node に直し，Changes 行を追加（指示書の文書一覧に無いが，この変更で偽になるため） |
| `plan/maintenance/tools/mutations.json` | +14 行，6 行当て直し | §8 |
| `plan/phase3/tools/mutations.json` | 1 行当て直し | §8 |

要点（実装）:

1. **モードの切り分け**：selector の一時的な key `keys` が `"channel"` のときだけ layout の key set になり（それ以外の値・型は MR-2 の拒否），`x3xx` は `ezsdr_radio::channels::from_selector`，`ideal` は `channels::ideal_from_selector` を使う．`Profile::new` は `Mode::Layout(channels::Description)` を持ち，instance は `coerce`・`tree`・`defaults`・`envelope`・`stream_path`・`rx_port`・stream の数をすべて `Profile` 経由で取る（1つの instance が新旧両方の key を受けることはない）．
2. **stream**：方向ごとに stream は layout の長さだけあり，node は `<device>/rx/s<k>`・`<device>/tx/s<k>`，受信の port は `rx<k>`．`stream_set` が「その stream の集合（`radio.d.c<n>.stream == k`）と rate（`radio.d.s<k>.sample_rate_hz`）」を1か所で返し，`prepare` は channel のある stream ごとに declare，`arm` は送信 stream ごとに登録・line の記帳，`start` は link のある受信 stream ごとに line を入れて最初の segment を T0 で始める．block の channel は集合の小さい順で，ramp の `im` は block の中の位置（MR-13 に明記）．jitter の generator は stream ごとに `<device>/rx/s<k>`．
3. **宛先（D14）**：`addressed` が宛先を `<device>`／自 instance の stream node／その他に分け，`TxBurst` は tx stream node のみ，`Stop` は `<device>`＝全 stream・stream node＝その stream，`start_rx` は rx stream node のみ，`UpdateParameter` は `<device>` か stream node で key が何を変えるかを決める．stream 0 の決め打ちは `handle_action`・cold key・`prepare` の declare から無くなった（§5）．
4. **channel ごとの設定と更新**：`cold` は key が名指す stream だけに記帳する（rate→その stream，antenna→その channel の入っている stream，membership→入る／出る stream）．`a → b`（どちらも 0 以上）は `COMMAND_REJECTED`（`RM-34: moving channel c<n>…`）で何も変えない．判定は「effective instant に投影した設定全体の `coerce`」で，`channels::Description::coerce` が `violations`（RM-31 全二重 RX2，RM-32 throughput）を通すため `prepare` も実行時も同じ1か所で判定される（`1b-2a` の coerce にあるので追加実装は不要．テストで両方を確かめた）．
5. **instance のものと stream のもの**：fault（`rx_overflow`・`rx_sequence_error`）は「そのときに走っている番号の一番小さい受信 stream」を切り，どれも走っていなければ `applied: false`．`device_lost` は全 stream．event の source は stream の node（`<root>/rx/s<k>`・`<root>/tx/s<k>`）で，`RX_OVERFLOW` の handle は受信 stream ごとに resolve する．新しいモードで `sim.channel` を宣言されたら `prepare` が `MR-41: ` で拒否する（channel mode を持たない）．
6. **今のモードは不変**：既存テストの diff は空で，`cargo test --workspace` は 1227 passed / 0 failed．`addressed`・`update_class_of`・`cold_streams`・`arm`／`start` の現キー側は HEAD と同じ経路（1方向1 stream）を通る．`Stop` の device 分岐の内部順も HEAD どおり tx→rx に戻した（`arrival()` を消費しないので観測差は無いが，差を作らないため）．

## 2. `ezsdr_radio` に足したもの

Mock が必要とする読み出しだけで，挙動の変更は無い（既存テストは全部通る）:

- `crates/ezsdr-radio/src/channels.rs`
  - `pub fn is_configuration(&self, key)`：`pub(crate)` → `pub`（`prepare` の `effective` に入る configuration key の判定）．
  - `pub fn update_class(&self, key) -> Option<UpdateClass>`（新規）：RM-27 の update class を，`radio_keys()` の唯一の `KeyDecl` 表から引く（規則の二重実装を避けるため，表を作り直さず宣言の綴りに直して引く）．
  - `pub fn stream_channels(&self, configuration, rx, k) -> timeline::Channels`（新規）：RM-3/RM-30 の「stream `k` が持つ channel の集合」を1か所で作る（Mock と，1b-4 の UHD Module が使う）．
  - `pub fn stream_of(&self, configuration, rx, number) -> i64`（新規）：その逆（channel がどの stream か）．上の `stream_channels` と同じ内部関数を使うので，membership の読みが2つに分かれない（セルフレビュー第1回の nit への対応）．
- `crates/ezsdr-radio/src/lib.rs`
  - `Concrete` と `concrete` を `pub(crate)` → `pub`：Mock が key の segment（`c<n>`/`s<k>`，leaf）を読んで，「どの stream か」「どの class か」を決める．`concrete` は RM-27 の綴りを読む唯一の場所として既に1か所なので，2つ目の解析を書かないために公開した（最小性の観点は §11 の未解決に挙げる）．
  - `radio_keys()` を `pub(crate)` に（`channels::update_class` が表を引くため）．
- `crates/ezsdr-mock-radio/src/profile.rs`：`Mode`（`Current`/`Layout`）と，`tree`・`coerce`・`defaults`・`envelope`・`timing`・`performance`・`profile_ref`・`rx_streams`・`tx_streams`・`stream_path`・`rx_port`・`is_layout`・`channels`・`description`．`description()` は layout instance では panic するが，呼び出し元は channel mode の分岐だけ（そこは layout では MR-41 で先に拒否される）．

## 3. 指示書から外れたところと理由

1. **`arm` は channel の無い送信 stream にも line を入れる**（指示書「channel のある送信 stream ごとに…line を入れる」の上位集合）．理由：あとから `cold` の `c<n>.stream` で最初の channel が入る送信 stream は，line が無いと `book_tx` が何もせず clock も登録されず，その stream が永久に動かない（今のモードが唯一の送信 stream に line を無条件で入れているのと同じ形）．spec 09 の MR-9 は "it books every transmit stream's line" とした．
2. **`design/08-simulation.md`（SE-6）を直した**：指示書の「書き換える文書」に spec 08 は無いが，「MockRadio の受信 jitter は `"<device>/rx"`」の文は新しいモードで偽になる（今は stream ごとの node）．1文と *Changes* 行だけ．design 33 の「書き換える文書」表に spec 08 が無いのは段取りの見落としと思われるので，orchestrator が design 33 の表を直すかの判断材料に挙げる．
3. **`Stop` の device 分岐は tx→rx の順**（HEAD と同じ）．実装を書き直すとき rx→tx になったのを戻した（差を作らないため．`arrival()` は `stop_tx`/`cancel_later` では消費されず，観測差は無い）．
4. **`Profile::description()` が layout で panic する**：channel mode の分岐だけが読むので到達不能（MR-41 が `prepare` で先に拒否する）．型で守る（`Option` を返す）のは channel mode 側の書き換えを伴うのでこの段階ではコメントで明記した（未解決 §11 に挙げる）．
5. **`Concrete`/`concrete` の `pub`**：§2 のとおり（これを避けるには `cold_streams` 全体を `channels` へ移すことになる．1b-4 で自然にやる）。
6. **`tests/mock_layout.rs` のハーネスに「port ごとに複数の link」を張れる入り口を足した**（`harness_links`）：MR-7 step 7 の「port ごとの Block link 規則」を確かめるため．

## 4. 1b-3a の報告 §8（持ち越し8項目）それぞれの扱い

| # | 内容 | 扱い |
|---|---|---|
| 1 | fault がどの stream を切るか | **ここで処理**：`faulted_rx_stream` が「その instant に走っている番号の一番小さい受信 stream」を選ぶ（`rx_runs` を `apply_rx_fault` と共有）．どれも走っていなければ `applied: false`．テスト `mr_21_a_fault_cuts_the_lowest_running_receive_stream`（overflow と sequence error，片方だけ走っている場合も），行 P133 |
| 2 | イベントの source（suffix） | **ここで処理**：`emit_event(path)` が source root から segment をたどる．stream の event は `<root>/rx/s<k>`・`<root>/tx/s<k>`（`TIME_ERROR` は `self.tx[k].path`）．テスト `mr_28_a_layout_instance_s_events_name_their_stream_node`，行 P132 |
| 3 | `Lines` の作り直し（`start` 失敗時の巻き戻し） | **ここで処理（`Lines` に API は足していない）**：`start` 失敗時は「全送信 stream の line を clone して `Lines::new()` に入れ直し，全受信 stream の `running`/`from`/`pool` を初期化」．move を記帳しない段階なので作り直しで失うものは無い．**複数 stream で巻き戻ることを確かめるテストと mutation の行は無い**（§11） |
| 4 | stream 0 の決め打ち | **ここで処理**：`handle_action`（宛先）・cold key（`cold_streams` の stream 選択と RM-25 の床の線）・`apply_update`（`update.tx` が `(k, to, passed)`）・`prepare` の declare をすべて `k` で一般化した．残る `self.rx[0]`/`self.tx[0]` は channel mode（＝現キー専用，layout では MR-41 で到達不能）と `addressed` の現キー分岐だけ（§5） |
| 5 | `ChannelMode.lo_retune` の引き順 | **1b-3c**（指示書どおり）．新しいモードは channel mode を持たない（MR-41）ので，複数 stream での引き順は 1b-3c の channel mode の話 |
| 6 | `Lines::book` が毎回 `Lines` 全体を clone する | **変更なし**（この段階では記帳が増えていない：`Stop`・`start_rx`・`cold`・`End` だけ）．move を記帳する 1b-3c で見る |
| 7 | `RxStream.port` と `node`，`RX_OVERFLOW` の handle の node | **ここで処理**：`port` は `rx<k>`，`node` は `<device>/rx/s<k>`，handle は受信 stream ごとにその node で resolve．テスト `mr_38_the_tree_and_capabilities_are_the_layout_s`（ports），`mr_21_a_fault_cuts_the_lowest_running_receive_stream`（overflow の source が fault の切った stream），行 P124・P140 |
| 8 | medium に渡す `Transmitter` の写像 | **該当なし（1b-3c）**：medium に渡すのは channel mode の送信 plan で，新しいモードは channel mode を持たない（MR-41）．複数送信 stream の写像は channel mode を新しいモードへ広げる 1b-3c の話 |

## 5. 「全部・だけ・すべて」と stream 0 の残り（grep で確認）

`rx_key(0)`/`tx_key(0)` は 0 件．`self.rx[0]`/`self.tx[0]` は次の4か所だけで，いずれも**新しいモードでは到達不能**:

- `lib.rs` の `addressed` の非 layout 分岐（`if self.profile.is_layout() { …; return }` の後）．
- `lib.rs` の `apply_update` の channel mode 分岐内（layout は `self.channel` が常に `None`）．
- `lib.rs` の `prepare` の channel mode 分岐内（その前に `Some(_) if self.profile.is_layout()` で MR-41 の拒否がある）．

「全 stream」の実装：device `Stop`（`(0..tx.len())` → `(0..rx.len())`），`stop_at`（全 tx/rx），`prepare` の declare（全 stream），`start`（link のある全受信 stream），`arm`（全送信 stream），`cleanup`．「どの宛先でも」は `addressed` が `<device>` と自 instance の stream node だけを Stream にし，layout の番号外は `Other` に落ちる．

## 6. spec 09 と spec 07（DA-1）

- `design/09-mock-radio.md`：**新設** MR-38（`keys` と2つの key set），MR-39（stream ごとの node・port・clock・集合と block の channel），MR-40（移動の拒否），MR-41（`sim.channel` の拒否）．**追随** MR-2（selector 表に `keys`，`rx`/`tx` の2つの読み），MR-3・MR-4・MR-5・MR-6・MR-7（step 6/7/8/10），MR-9・MR-10・MR-11，MR-12（jitter の generator），MR-13（`c` は block の中の位置），MR-16（target と channel 数），MR-18（宛先・class・`cold` の stream・`violations`・*Ceiling* を3つに），MR-20・MR-21・MR-22（fault の stream），MR-28（event の source），MR-29（宛先），MR-31（layout は channel mode にならない），MR-34（layout の phase は channel ごと），MR-37（stream ごとの handle），Scope・§4・§6（`mock_layout.rs` の 13 テストの表）・Changes．
- `design/07-radio-model.md`：RM-11 のみ（event の source node を「その stream の node」に）＋ Changes 行．
- `design/08-simulation.md`：SE-6 の jitter の stream 名＋ Changes 行（§3-2）．
- `design/vision/*`・`Ez-SDR_v4_ARCHITECTURE_VISION.md`：**変更していない**．§2 の「方向ごとに1つの key」と §8 の例，§13 の grid の文は 1b-2（RM-27 の導入）で既に一般化として偽だが，この diff が新たに偽にしたものではない（plan 33 の「書き換える文書」の Vision 行は 1b-5 で処理する想定．§11 に挙げる）．§8 の `instances: 2` は 1a が「illustrative として残す」と決定済み（Vision の Changes 表）．
- 規則の文面は経緯・段階・issue 番号を含まず，各規則の `*Test:*` は実在するテストだけを名指す（`rules.py --base HEAD` が 0 problem）．

## 7. 規則ごとのテストと mutation の行

`crates/ezsdr-mock-radio/tests/mock_layout.rs`（新規，13 tests）．各行は「同じ規則を誤る形」で，`old` はソースにちょうど1回一致し，`name`・`filter`・`test_bin` は全て kill するテストを名指す（`remote-mutate-5.log` で 26/26 killed）．

| test | 確かめる規則（規則 → 誤った実装を拒む検査） |
|---|---|
| `mr_7_each_receive_port_takes_its_own_links` | MR-7 step 7：2 port に Block 1本ずつは受理，1 port に2本は `MR-7: multiple rx links cannot use Block back-pressure`；channel の無い stream に link があっても segment を始めず clock も無い（現キーの `channels=0`＋link と同じ） |
| `mr_09_each_transmit_stream_has_its_own_clock_and_line` | MR-9/RM-25：2つの送信 stream が各自の ratio（500/1000）で `arm` に登録，`cold` の rate 変更がその stream の clock だけを cut して lead 後に新 clock，各 stream の node への burst が保持・送信されて2記録，`Stop` がその stream の held burst だけを消す |
| `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` | MR-12/16/18/25/29：`<device>`／stream node への更新が適用，burst は tx stream node のみ（誤った宛先は MR-16 の拒否，capability だけの key と instance 全体の key は not updateable），`Stop` は stream 単位（他 stream は動き続け，`start_rx` がその node で再開） |
| `mr_13_a_stream_s_block_channels_follow_its_channel_set` | MR-13/RM-3：集合 `{0,2}` の block は 2 channel，2つ目の `im` は位置の `1/64`（論理番号なら `2/64`） |
| `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` | MR-18/27/40：antenna の `cold` が cut・2 ms の受信 call・3 ms の allowance・50 ms の lead で新しい clock を始める（`T0+65 ms`），channel ごとの RM-8 の範囲（BasicRX の 200 MHz と 10 dB は拒否，UBX の同じ値は可），membership の出入りがその stream だけ，移動は `RM-34` で拒否され値も clock も変わらない，rate はその stream の clock だけを替える |
| `mr_21_a_fault_cuts_the_lowest_running_receive_stream` | MR-20/21/22/28/37：2 stream が走っていれば番号の小さい方（行 `applied: true, lost: 50000`，その stream の overflow と `RESTARTED` block，他 stream は無傷），片方だけなら走っている方（overflow の source がその stream の node），sequence error は `SEQ_DISCONTINUITY` で 2000 サンプル |
| `mr_28_a_layout_instance_s_events_name_their_stream_node` | MR-28/RM-11：遅れた burst の `TIME_ERROR` の source が `mock/tx/s0` |
| `mr_30_two_layout_mocks_one_seed_identical_output` | MR-30/12：同じ seed で同じ header・section・event，jitter の長さが 2000 と限らず，2 stream が1つの generator 列を共有しない |
| `mr_38_a_layout_instance_uses_one_key_set` | MR-38/2/6：`keys` の値（`"layout"`・1・list は拒否），新旧の key set が交わらない（`RM-4: not a radio key`），無い番号は `RM-4: no such channel c2` |
| `mr_38_the_tree_and_capabilities_are_the_layout_s` | MR-38/3/4/5：node `<device>/rx/s0..2`・`<device>/tx/s0`，port `rx0..2`，channel ごとの range・gain・antenna・phase・path delay，stream ごとの rate 格子，`radio.rx.channels` と `radio.rx.coherent` が無い，RM-30 の既定と `effective` のキー集合，`ideal` の layout |
| `mr_39_each_receive_stream_declares_and_runs_its_own_clock` | MR-39/7/10/11：rate の違う2 stream が別 SampleClock（ratio 100 と 1000，origin T0）と別 instant の block（`T0+199 901` と `T0+1 999 001`） |
| `mr_41_a_layout_instance_takes_no_simulation_channel` | MR-41：`sim.channel` を宣言された layout instance は `MR-41: ` で拒否 |
| `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` | RM-31/32・MR-7/18/30：全二重 RX2 と throughput の拒否が実行時にも効き（他 stream に入っていなければ antenna は変えられる），`prepare` 自身も `violations` で拒否する |

| id | 誤りの形（`old` → `new`） | kill するテスト |
|---|---|---|
| P122 | `keys` の値の検査を `_ => true` に（どの値でも layout になる） | `mr_38_a_layout_instance_uses_one_key_set` |
| P123 | `ideal` の `keys` instance を `None`（現キーの description）に | `mr_38_the_tree_and_capabilities_are_the_layout_s` |
| P124 | port 名を `rx<k>` から `rx` に（全 stream が同じ port） | 同上 |
| P125 | `start` が全受信 stream を stream 0 の line で始める | `mr_39_each_receive_stream_declares_and_runs_its_own_clock` |
| P126 | block の `channels` を 1 に固定 | `mr_13_a_stream_s_block_channels_follow_its_channel_set` |
| P127 | 全 layout stream が1つの jitter generator 名を共有 | `mr_30_two_layout_mocks_one_seed_identical_output` |
| P128 | 移動を拒否せず `Ok` で stream `was` に記帳 | `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` |
| P129 | `MR-41:` の接頭辞を落とす | `mr_41_a_layout_instance_takes_no_simulation_channel` |
| P130 | `channels::update_class` が常に `Cold` を返す | `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` |
| P131 | antenna を常に stream 0 に記帳 | `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` |
| P132 | `TIME_ERROR` の source を instance（`""`）に | `mr_28_a_layout_instance_s_events_name_their_stream_node` |
| P133 | fault が番号の大きい走行 stream を切る（`.rev()`） | `mr_21_a_fault_cuts_the_lowest_running_receive_stream` |
| P134 | `Stop` が常に全 stream を止める | `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` |
| P135 | `coerce` が `violations` を判定しない（空の反復） | `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` |
| P136 | 送信 stream の clock を stream 0 の集合と rate で登録 | `mr_09_each_transmit_stream_has_its_own_clock_and_line` |
| P137 | layout の channel の既定 membership を常に −1 に | `mr_38_the_tree_and_capabilities_are_the_layout_s` |
| P138 | 存在しない channel 番号を最初の channel で代用（`RM-4: no such channel` を出さない） | `mr_38_a_layout_instance_uses_one_key_set` |
| P139 | channel の key を configuration でなくす（`effective` に載らない） | `mr_39_each_receive_stream_declares_and_runs_its_own_clock` |
| P140 | 全受信 stream の `RX_OVERFLOW` の handle を stream 0 の node で resolve | `mr_21_a_fault_cuts_the_lowest_running_receive_stream` |

追加した規則のうち，**専用の行が無い**ものは MR-10（node id を作る `stream_path` で，tree/port の行とテストが間接に押さえる），MR-16（layout の誤った宛先は `handle_action` の振り分けで拒否され，内側の target 検査は新しいモードでは到達しないため，その行は equivalent になる），MR-29（`start_rx` の宛先は P134 と同じテストで，単独では壊せない），MR-34（layout が phase を引かないのは到達不能な否定文）．§11 に挙げる．

## 8. 当て直した mutation の行（7 行，全部 killed）

`old` が実装の書き換えで動いたので，同じ規則を同じ形で壊す行に当て直した（`name`・`filter`・`test_bin`・`crate` は不変．`old` は全部ちょうど1回一致）:

| list | id | 何が動いたか（当て直しの要約） |
|---|---|---|
| maintenance | F37 | 「受信 enable の start lead を receipt から数える」— `Item` の行が書き換わった（`timeline::Channels::first(u32::from(channels))` → `channels`）ので `old` を追随（`ready: now` → `tick` の mutant は同じ） |
| maintenance | J196 | 「burst の拒否が Action の id を名乗る」— 拒否を1か所（`refuse_tx_burst`）にまとめ，`?;` が付かない式になったので `old` を追随 |
| maintenance | J276 | 「Provider root が host でない domain を宣言する」— `tree:` が `profile.tree(...)` になったので追随 |
| maintenance | P10 | 「CBX の `random` の phase を timed tune が固定する」— `Mode::Current(description) => description.phase_behavior_on_retune != "random"` の腕に追随 |
| maintenance | P12 | 「Mock が独自の gain range を名乗る」— `Profile::new` の `x3xx` の腕が `Mode::Current(Box::new(match …))` になったので追随 |
| maintenance | P15 | 「Mock の設定が description の既定でなく RM-5 の 1 GHz で始まる」— `self.description().defaults` → `self.profile.defaults()` に追随 |
| phase3 | M60 | 「MR-17 の `TIME_ERROR` の時刻を切り上げる」— `emit_late_burst` が `k` を受けるようになったので追随 |

## 9. 確かめること（指示書 1〜7）の結果

| # | 何を | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | **1227 passed / 0 failed**（65 test bin）．最終ツリーで | `cargo-test-workspace-final.log`（ほかに `-6.log` など） |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0**（警告 0）．最終ツリーで | `clippy-workspace-final.log` |
| 3 | Docker の `--features uhd` clippy（`ezsdr-radio-uhd`） | **exit 0**．最終ツリーで | `docker-clippy-uhd-final.log` |
| 4 | Python（venv 3.14，`cargo build -p ezsdr-server` の後） | **Ran 47 tests … OK**．最終ツリーで | `python-suite-final.log` |
| 5 | `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | **0 problem(s)** | `rules-final.log` |
| 6 | 足した 19 行（P122〜P140）と当て直した 7 行 | **26/26 killed**（COMPILE ERROR・SURVIVED・NOT FOUND なし）．コードとテストの md5 は `remote-mutate-5.log` のリビジョンと同一（`code-hashes-before-doc-edits.txt` と `md5sum -c` で確認）なので，その後の文書だけの修正はこの結果を無効にしない | `remote-mutate-5.log`（ほかに `-4.log` 22/22，`-3.log` 21/21） |
| 7 | `git diff HEAD -- crates/ezsdr-mock-radio/tests/mock_radio.rs crates/ezsdr-mock-radio/tests/mock_channel.rs crates/ezsdr-mock-radio/tests/timeline.rs` | **空**（0 行）．`git status` は §1 の 10 ファイルだけ | `tests-diff-final.log`，`git-status-final.log` |

（`cargo fmt`/`rustfmt` は使っていない．mutant を作業ツリーに当てていない（切り分けは無し：`old` が全部1回一致したため不要だった）．版は上げていない（`Cargo.toml` は無変更）．新規ファイルは 644．Python は `PYTHONDONTWRITEBYTECODE=1`．新しいモードは `keys` selector 経由でしか到達できず，Spec・server・Python・fixture・UHD Module は触っていない．）

## 10. セルフレビュー（AGENTS.md §9 の手順2）

`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash --agent plan --auto "<チェックリスト（`review-checklist.txt`）>"` を毎回**新しい**読み取り専用のレビュー役として nohup で起動し，`kill -0` で終了を待った（`review1.sh`）．レビュー役には `cargo`・mutation を走らせず，読み取りと grep と `rules.py` だけで確認するよう指示した．1回，レビュー中に私がツリーを直してしまった回があるので，その回は破棄して（ログも残さず）凍結したツリーで第1回をやり直した．第1回・第2回・第3回はそれぞれ，そのときのツリーを凍結したままレビューしている．

### 第1回（`self-review-1.log`，約 20 分）— P0 なし，**P1 1件**，P2 5件，nit 多数

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P1 | 送信 stream が2本以上あるケースのテストが1本も無く，MR-9 の `*Test:*` が実際には確かめないこと（各送信 stream の clock と line）まで書いている | **採った**：`mr_09_each_transmit_stream_has_its_own_clock_and_line` を追加（2 送信 stream，rate 変更・burst・`Stop`），mutation 行 P136 を追加，MR-9 の `*Test:*` を実テストに合わせ，§6 にテストの行を追加 |
| 2 | P2 | MR-22 の「MR-21 が名指す stream」に対応する layout テストが無い | **採った**：`mr_21_a_fault_cuts_the_lowest_running_receive_stream` に sequence error の場合（`SEQ_DISCONTINUITY`，2000 サンプル，他 stream の無傷）を追加し，MR-22 の `*Test:*` を追随，§6 の行を更新 |
| 3 | P2 | MR-7 step 7（port ごとの Block link）はテスト自体が無い | **採った**：`mr_7_each_receive_port_takes_its_own_links` を追加（ハーネスに `harness_links` を追加），§6 にテストの行を追加 |
| 4 | P2 | `cold_streams` を「要求された instant」の設定で選び，記帳・`coerce` は床で後ろにずれうる `tick` の設定で行う（食い違いの芽） | **採らない（未解決へ）**：1b-3c の D3（2本の line の床と移動）の範囲．第3回が具体的な反例を出したので §11 に詳しく残す．MR-18 の *Ceiling* に明記した |
| 5 | P2 | `ezsdr_radio` の `Concrete`/`concrete` の `pub` 化は最小より広い | **採らない（記録）**：§2・§3-5．1b-4 で `cold_streams` を `channels` へ移すときに `pub(crate)` に戻せる．§11 に残す |
| 6 | P2 | `Profile::description()` が layout で panic する（型で守られていない） | **一部採った**：到達不能の理由をコメントで明記（§3-4）．`Option` 化は channel mode 側の書き換えを伴うので §11 に残す |
| 7 | nit | device `Stop` の内部順が HEAD と入れ替わった | **採った**：tx→rx に戻した（HEAD と同じ） |
| 8 | nit | 生の `value` を membership の読みに渡している（型が違うと余計な `LATE_COMMAND` を出しうる） | **採った**：`cold_streams` は candidate の値が Int でなければどの stream も名指さない（`int_config_opt`） |
| 9 | nit | channel mode だけの `rx[0]`/`tx[0]` に理由コメントが無い | **採った**：`apply_update` の channel mode 分岐にコメントを追加 |
| 10 | nit | MR-31・MR-34 の `*Test:*` が `mr_41` を名指していない，MR-18 の *Ceiling* が現キー名のまま | **採った**：両方の `*Test:*` に `mr_41_a_layout_instance_takes_no_simulation_channel` を足し，*Ceiling* を「その stream の channel 数」に直し，`cold` の *Ceiling* を1つ足した |
| 11 | nit | `stream_of` が channels.rs と Mock に二重にある | **採った**：`channels::Description::stream_of` を足し，Mock の写しを消した（§2） |
| 12 | nit | P129 は接頭辞落としだけで「受理する」mutant を殺す行が無い | **採らない（記録）**：拒否の腕を外す mutant は layout で `description()` に達して panic する（きれいな kill にならない）．§11 に残す |
| 13 | P2 | Vision §2:54・§8:89（`instances: 2`）が偽 | **採らない（記録）**：1a・1b-2 からの持ち越しで，この段階が新たに偽にしたものではない（第2回・第3回も同じ判断．`instances: 2` は 1a が illustrative として残すと決定済み）．§11 に残す |

### 第2回（`self-review-2.log`，約 22 分）— **P0・P1 なし**，P2 6件，nit 多数

指摘と扱い（要点）:

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P2 | `design/08-simulation.md`（SE-6）の jitter の stream 名が新しいモードで偽 | **採った**：1 文を直し，design 08 の *Changes* に行を追加（§3-2） |
| 2 | P2 | MR-7 step 7 が引用する拒否文面が実装（`multiple rx links cannot use Block back-pressure`）と違う | **採った**：spec の引用を実装の文面に合わせた（テストは変更していない） |
| 3 | P2 | 規則ごとの行の欠け（MR-5・MR-6・MR-10・MR-16・MR-29・MR-34・MR-37；MR-7 step 7 はテストも無い） | **一部採った**：MR-5・MR-6 の行 P137・P138，MR-37（stream ごとの handle）の行 P140 と `mr_21…` の assertion を追加．MR-7 step 7 は第1回でテストを追加済み．MR-10・MR-16・MR-29・MR-34 は §7 の理由で残す（§11） |
| 4 | P2 | layout の `Stop`（全 stream）・`device_lost`・`start` 失敗時の複数 stream 巻き戻しにテストが無い | **採らない（未解決へ）**：§11．`start` 失敗は `schedule_wakeup` の失敗を外部から作れないため，テストの作り方から設計が要る（1b-3c の weakness として残す） |
| 5 | P2 | Vision の key 記述（1b-2 からの持ち越し） | **採らない（記録）**：§11 |
| 6 | nit | `Concrete`/`concrete` の `pub`，`Profile::description()` の panic | **採らない（記録）**：第1回と同じ（§11） |
| 7 | nit | `is_configuration` の `*Test:*` など，「実際には確かめないこと」を書いた `*Test:*` が残る | **採った**（第3回と併せて）：MR-4・MR-38・MR-40 の `*Test:*` を実際の assertion に合わせて削った |

### 第3回（`self-review-3.log`，約 22 分）— **P0・P1 なし**，P2 6件，nit 多数

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P2 | `cold_streams` を `asked` の設定で選び，記帳は `tick` の設定で行うため，2つの scheduled な membership 更新で設定と line が食い違いうる（具体的な反例つき） | **採らない（未解決へ）**：1b-3c の D3 の範囲．§11 に反例と直し方を残す（MR-18 の *Ceiling* に明記済み） |
| 2 | P2 | MR-34 の `*Test:*` が「layout は何も引かない」を `mr_41` で確かめたように書いている（観測不能） | **採った**：その句を落とした |
| 3 | P2 | `design/08` の *Changes* 行が無い | **採った**（第2回の 1 と同じ対応） |
| 4 | P2 | `report.md` 未提出 | **本報告** |
| 5 | P2 | 規則ごとの行の欠け（第2回 3 と同じ）と，テスト・行の無い「全 stream の `Stop`／`device_lost`／巻き戻し」 | **採らない（未解決へ）**：§11 |
| 6 | P2 | Vision の持ち越し | **採らない（記録）**：§11 |
| 7 | nit | `Profile.selection` がもう読まれない（`pub` のまま） | **採らない（記録）**：HEAD でも読まれておらず（1a 以前から），この段階の変更ではない．§11 |
| 8 | nit | `handle_update` の生の `value` を membership の読みに渡す点（第1回 8）は修正済みであることを確認，`P138` の `new` が型として通ることも確認 | — |
| 9 | nit | `Concrete`/`concrete` の `pub`，`Profile::description()` の panic | **採らない（記録）**：§11 |
| 10 | nit | `design/07:241`（RM-21）が move を `cold` として規範化しており，Mock の拒否（MR-40）と読める差がある | **採らない（記録）**：段階どおり 1b-3c で解消する．§11 に挙げる |

3回で P0・P1 は第1回の1件（送信2本，採って修正済み）だけで，第2回・第3回は P0・P1 なし．指摘の残りは上記の「採らない（記録）」で，理由を §11 に残す（指示書の「findings left open after round 3 are listed as open, not dropped」に従う）．

## 11. 未解決のこと（1b-3c・1b-4・orchestrator へ）

1. **`cold` の対象 stream を「要求された instant」の設定で選ぶ不整合（1b-3c，D3 の範囲）**：`handle_update` は `cold_streams` を `projected(requested_tick.unwrap_or(now))` で呼ぶが，記帳と `coerce` は床で後ろにずれた `tick` の設定で行う．反例（第3回）：`radio.rx.c1.stream` を `T+5 ms` で `1 → -1`，続けて `at` 無しで `-1 → 1` を受けると，2つ目は `asked = now` で `was == is` となり line に何も記帳せず，`tick` も `now`（`T+5 ms` より前）になる．RM-25 の「受信した順の `cold` 変更の床」では 2つ目は `T+5 ms` 以降に効くべきなので，設定（`c1.stream = 1`）と line（`T+5 ms` で集合が空になる）が食い違う．直し方：`cold` の対象 stream を「key が名指す stream」（membership は `was` と `is` の両方）として床の線を決め，`tick` を確定した後にもう一度 `cold_streams` を呼んで記帳対象（集合が変わる stream）を取る．そうすると MR-18 の *Ceiling* も消える．**この段階の範囲外**（1b-3c の「移動の2本の line の床」と同じ機構）なので，テストと一緒に 1b-3c で直す．
2. **layout の「全 stream」の道でテストと行が無いもの**：(a) `<device>` 宛ての `Stop` が全 stream を止めること，(b) `device_lost` が全 stream を止めること，(c) `start` 失敗時の複数 stream の巻き戻し（`schedule_wakeup` を失敗させる手立てが要る）．(a) と (b) は M-row の形で足せる（`mr_12_d14…` の `<device>` Stop を layout で見る）．(c) は設計（失敗を注入できる形にする）から要る．
3. **専用の mutation 行が無い規則**：MR-10（`stream_path`），MR-16（layout の誤った宛先＝ `handle_action` の振り分けで拒否され，内側の検査は到達しないので equivalent になる），MR-29（`start_rx` の宛先），MR-34（到達不能な否定文）．**MR-41 の「拒否そのもの」を外す行**（受理側）は layout で `description()` の panic になるため作っていない．
4. **`ezsdr_radio` の `Concrete`/`concrete` の `pub`**：1b-4 で `cold_streams`（と `addressed` の key 判定）を `channels` 側の1か所に移せば `pub(crate)` に戻せる．UHD Module が同じ判定を使うときにやるのが自然．
5. **`Profile::description()` の panic**：`Option<&DeviceDescription>` にするか，channel mode の設定を `Profile::channels()` 側に寄せるか．今は channel mode の中だけなので到達不能だが，新しいモードへ channel mode を広げる段階（1b-3c）で必ず踏む．
6. **Vision の radio の key 記述（1b-2 からの持ち越し）**：`design/vision/02` §2:54 の「方向ごとに1つの key」と §8 の例（`radio.rx.channels` など，`:24-29`），`design/vision/03:71` の `radio.rx.sample_rate_hz` の grid 文．layout の key を Spec に使わせる段階（1b-5）で直す．plan 33 の「書き換える文書」は Vision を挙げているので，どの段階でやるかを orchestrator が決める．`design/07:241`（RM-21）の move の文も 1b-3c で追随する．
7. **`Profile.selection` が読まれない**（`pub` のまま）：1a 以前からの未使用．消すか doc に理由を書くのは別の整理．
8. **1b-5 への申し送り**：古い key と今の builder を消すとき，`P10`・`P12`・`P15` の3行（現キーの分岐に依存）と，`P122`〜`P140` のうち `keys` selector・`is_layout` 分岐・現キー分岐に依存する行は当て直しか削除が要る（とくに `P12` は `Mode::Current` 側だけを壊すので，受け入れテストが layout 側へ移ると生き残る）．
9. **1b-4（UHD Module）への申し送り**：同じ `keys` の切り分け（`keys: "channel"`），`channels::from_selector`／`ideal_from_selector`，`Description::stream_channels`／`stream_of`／`update_class`／`is_configuration`，`stream_target`，`Profile` の `Mode` の形をそのまま使える．UHD 側で必要なのは subdev spec の作成（`Selection` の prefix 制限の撤去），stream ごとの streamer と thread（D15），`Applied` への antenna（D5/D16），開くときの照合（D5）．
