STATUS: done

# Phase 8 段階 1b-4a（UHD Module の device 境界を stream ごと・subdev spec つきに広げる）実装報告

指示書: `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/brief.md`（HEAD `9816d78`，未コミットの作業ツリー）．

## 1. 変えたファイルと要点

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-radio-uhd/src/device.rs` | `Device` trait を stream 番号つきの署名にし，`serial`・`set_subdev_spec`・`rates` を足し，`Applied.antenna` を足した．`FakeDevice` は方向ごとの front end 表と現在の subdev spec，方向ごとに複数の streamer（それぞれ channel 列・start・stop・packet・async report を持つ），rate の格子と pre-chain の `{1.0}`，serial，antenna の読み返し，送信の buffer 数・長さの検査，`close_streams` の全解放を持つ．loopback は channel `c` を channel `c` に返す． |
| `crates/ezsdr-radio-uhd/src/uhd.rs` | `UhdDevice` を stream ごとの streamer の map（`BTreeMap<usize, Arc<…>>`）にし，`serial`（`uhd_usrp_get_rx_info` の `mboard_serial`），`set_subdev_spec`（`uhd_subdev_spec_make` + `uhd_usrp_set_{rx,tx}_subdev_spec`），`rates`（`uhd_usrp_get_{rx,tx}_rates` + `uhd_meta_range_{make,size,at,free}`），antenna の読み返し（`uhd_usrp_get_{rx,tx}_antenna`）を実装．FFI 宣言，`uhd_range_t` / `uhd_usrp_rx_info_t`，`struct_sizes` 6 件． |
| `crates/ezsdr-radio-uhd/src/provider/{mod,rx,tx,control}.rs` | 呼び出しを `rx_recv(0, …)`・`rx_open(0, &(0..n))`・`rx_start(0, …)`・`rx_stop(0, …)`・`rx_packet_samples(0)`・`tx_open(0, &(0..n))`・`tx_send(0, …)`・`tx_async(0, …)` に直した．**`UhdRadio` の挙動は変えていない**（`set_subdev_spec` は呼ばない．Manifest の section・event・テストの結果は HEAD と同じ）． |
| `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs` | `Scripted` を新署名に（stream 番号は今はモデルしない旨をコメント；`serial`・`set_subdev_spec`・`rates` は inner に転送）． |
| `crates/ezsdr-radio-uhd/src/provider/control/differential.rs` | 呼び出し 2 箇所． |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | 呼び出しの書き換え（stream 0），wrapper 3 種（`Watched`・`Descheduled`・`LostAtShutdown`），call 文字列の assert 3 箇所，新テスト 9 本（§5）． |
| `crates/ezsdr-radio-uhd/tests/hardware.rs` | 呼び出しの書き換え（stream 0）． |
| `crates/ezsdr-radio-uhd/tests/uhd_api.rs` | 構造体の大きさ 6 件（`uhd_range_t` 24，`uhd_usrp_rx_info_t` 64 を追加）． |
| `design/18-uhd-radio.md` | UR-2（trait の一覧・型），UR-3（関数の一覧，Threads，`serial` の INFERRED），UR-33（front end 表，subdev spec，streamers，rates/serial/空の列，送信の検査），§2・§6・Changes の 2026-10-18 行． |
| `plan/maintenance/tools/mutations.json` | 新規行 P199〜P229（31 行）． |
| `plan/phase7/tools/mutations.json` | 当て直し 6 行（B01・B15・B20・B22・B28・B30）と R12 の `new` の当て直し． |

## 2. trait の新しい署名（UR-2）

```rust
fn describe(&self) -> serde_json::Value;
fn fidelity(&self) -> Fidelity;
fn master_clock_rate(&self) -> u64;
fn channels(&self, dir: Dir) -> usize;
fn front_end(&self, dir: Dir, chan: usize) -> Result<String, DeviceError>;
fn serial(&self) -> Result<String, DeviceError>;
fn set_subdev_spec(&self, dir: Dir, markup: &str) -> Result<(), DeviceError>;
fn rates(&self, dir: Dir, chan: usize) -> Result<Vec<(f64, f64, f64)>, DeviceError>;
fn set_sources(&self, clock: &str, time: &str) -> Result<(), DeviceError>;
fn set_time_zero(&self, at_next_pps: bool) -> Result<(), DeviceError>;
fn time_now(&self) -> Result<i64, DeviceError>;
fn ref_locked(&self) -> Result<Option<bool>, DeviceError>;
fn apply(&self, dir: Dir, chan: usize, s: &Settings, at: Option<i64>) -> Result<Applied, DeviceError>;
fn rx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError>;
fn rx_start(&self, stream: usize, at: i64) -> Result<(), DeviceError>;
fn rx_packet_samples(&self, stream: usize) -> usize;
fn rx_stop(&self, stream: usize, at: Option<i64>) -> Result<(), DeviceError>;
fn rx_recv(&self, stream: usize, n: usize, timeout: Duration) -> RxRecv;
fn tx_open(&self, stream: usize, channels: &[usize]) -> Result<(), DeviceError>;
fn tx_send(&self, stream: usize, samples: &[&[Iq]], at: Option<i64>, sob: bool, eob: bool, timeout: Duration) -> Result<usize, DeviceError>;
fn tx_async(&self, stream: usize, timeout: Duration) -> Result<Option<TxReport>, DeviceError>;
fn close_streams(&self);
fn mark_lost(&self);
fn host(&self) -> HostClock;
```

`Applied { rate, freq, gain: f64, antenna: Option<String> }`：antenna は「antenna を名指した untimed の変更」の後に device が読み返した値で，それ以外（antenna を名指さない変更，timed の変更）は `None`．

## 3. 指示書から外れたところと理由

1. **空の spec と重複した組**：指示書は「UHD と同じく失敗にする」．UHD 4.10 の RFNoC `multi_usrp` は `_set_subdev_spec` でどちらも検査しない（重複は同じ channel を2度登録，空は channel map を空にする；`multi_usrp_rfnoc.cpp:1226-1290,2796-2850`，ソースで確認）．fake は **set の時点で両方を拒否**し，文面は `ValueError: duplicate subdev …` / `ValueError: empty subdev-spec markup string`（UHD の lib にはこの2文は無い）．表にない組は UHD の lookup 文そのまま．重複した組の UHD の失敗は stream を開くとき（"Attempting to reconnect … port!"）に来るので，fake も両方の時点で同じ規則を守る．設計には INFERRED と明記．
2. **開いていない stream への呼び出し**：`tx_async` は streamer が無くても timeout 待って `Ok(None)`（HEAD の fake は streamer が無くても device 全体の report を返していた．per-stream 化で queue は streamer のものになった）．`rx_recv` は streamer が無いとき最大 10 ms 待って `Failed`（lost の経路と同じ待ち；plan 30 §4.7 の「loop は毎回 clock で待つ」を守るため）．`rx_start`・`rx_stop`・`tx_send` は即時に `Err`．設計 UR-33 に呼び出しごとに書いた．
3. **失敗した reopen の後始末**：`UhdDevice`（と UR-25）と同じく，fake も **検査の前に古い streamer を外す**（拒否された reopen の後は stream が閉じている）．1b-4a では provider が reopen を失敗させないのでテストの結果は変わらない．
4. **subdev spec の設定で streamer を切る**：fake と `UhdDevice` は，`set_subdev_spec` の成功でその方向の streamer を切る（UHD の "Disconnect and clear existing chains"）．fake はさらにその方向の「開いた channel」の履歴を消す（`rates` の pre-chain の `{1.0}` と整合させる．INFERRED）．指示書には無いが，fake を UHD より緩くしないため．1b-4c は spec を streamer より先に設定するので影響しない．
5. **空の channel 列**：UHD は `sanitize_stream_args` で空を `{0}` と読む（ソースで確認）．`UhdDevice` も空を channel `0` として C に渡し，Rust 側の channel 数と device の実際を揃えた（HEAD は長さ 0 の配列を渡していた：受信で未定義動作になりうる潜在バグ）．fake は channel のない streamer を開く（この段階で空を名指ししうるのは「channel 数を 0 にする cold change の `tx_open(0, &[])`」だけで，その stream は何も送らない）．設計に INFERRED として書き，1b-4b へ渡す（§9）．
6. **送信の buffer の検査**：fake の `tx_send` に，`UhdDevice` と同じ「buffer 数 = streamer の channel 数」「最初の buffer より短い buffer は不可」を足した（指示書には無いが，streamer ごとの channel 列を入れた以上「UHD より緩くない」に必要）．
7. **テストの場面の変更 2 件**：`ur_33_scripted_faults_fire` は tx streamer を開くようにした（report は streamer ごとになったため．assert は変えていない）．`ur_33_one_channel_is_opened_by_one_streamer` などは call 文字列の assert を新しい記録形式（`rx_start 0 …`，`rx_stop 0 now`）へ直した（assert の意味は同じ）．
8. **設計の UR-15・UR-17・UR-25 の呼び出し形**：指示書の一覧（UR-2・UR-3・UR-33）に無いので，いったん `rx_start(0, T0)` 等に直した後 **stream に依存しない元の形へ戻した**（1b-4b が stream ごとにする）．Changes 行に記録．
9. **確かめること 3 への追加**：`uhd` feature の unit test（`uhd_streams_are_looked_up_by_number`）は指示書の `--test uhd_api` では走らないので，同じ image で `cargo test -p ezsdr-radio-uhd --features uhd --lib` も流した（§8 の check 3）．mutation の gate は default feature なので `uhd.rs` には行を作れない（§9）．

## 4. 確かめられず INFERRED にした UHD の挙動

- pre-chain（その channel で streamer を開く前）の `rates` が `[(1.0, 1.0, 0.0)]`（設計 33 D5．**BP0** で実機確認）．
- 空の spec・重複した組を UHD が拒否しないこと，および fake の拒否文面（set の時点では UHD に検査が無いことはソースで確認済み；実機での挙動は未確認）．
- 同じ channel を2つの streamer で開いたときの拒否（経路は `graph.cpp:179-197` のソースで確認，"reconnect … port!" は VERIFIED；実機で stream を作るときに同じ経路を通るかは未確認）．
- `serial` が `uhd_usrp_get_rx_info(0)` の `mboard_serial` で得られること（受信 channel を持たない装置では失敗しうる）．
- front end 表の Basic/LF の2本（`db_basic_and_lf.hpp` で確認）と，fake が作る名前 `BasicRX (0)`（`db_basic_and_lf.cpp` の name）．
- 残った front end が spec 変更をまたいで設定を保つこと．
- `UhdDevice` の新しい経路（`serial`・`set_subdev_spec`・`rates`・antenna の読み返し）の実機の挙動（**BP0**）．コンパイル・clippy・構造体の大きさは確認済み．**BP0 で確かめること**：pre-chain の rate，`set_subdev_spec` の失敗文面，`serial`，front end 名，antenna の読み返し，同じ channel の2 streamer の拒否．

## 5. 規則ごとのテストと mutation の行

新しい行は P199〜P229（31 行）．すべて `remote_mutate.py` で killed（§7）．

| 規則（spec 18） | テスト | mutation 行（誤りの形 → kill するテスト） |
|---|---|---|
| serial（UR-33） | `ur_33_the_serial_is_the_config_s` | P199 config を無視して固定値 → 同テスト |
| front end 表：Basic/LF の2本，Basic/LF の名前，LFTX の送信1本（UR-33） | `ur_33_the_front_end_table_follows_the_subdev_spec` | P200 BasicRX の受信を1本に，P201 Basic/LF の名前から番号を落とす，P225 BasicTX の送信を0本に → 同テスト |
| subdev spec の拒否（表にない組・3分割・空・重複）（UR-33） | `ur_33_a_subdev_spec_uhd_would_refuse_is_refused` | P208 表を引かない，P209 3分割を通す，P210 空を通す，P202 重複を通す → 同テスト |
| 1つの channel は1つの streamer：cross-stream，同じ列の重複，拒否された reopen，`close_streams`，spec 変更の切断（UR-33） | `ur_33_one_channel_is_opened_by_one_streamer`，`ur_33_setting_a_spec_disconnects_that_direction_s_streamers` | P203 rx cross-stream，P220 tx cross-stream，P218 rx 列の重複，P221 tx 列の重複，P219 rx reopen，P222 tx reopen，P224 close_streams が tx を残す，P226 spec 変更で切らない，P223 未知の channel を通す → 2 テスト |
| stream ごとの受信状態：channel 列，start，stop，loopback（UR-33） | `ur_33_two_receive_streamers_keep_their_own_channels` | P204 loopback が channel 0 固定，P214 `rx_recv` が stream 0 の列，P215 `rx_start` が stream 0 に記帳，P216 `rx_stop` が stream 0 に → 同テスト |
| stream ごとの送信：streamer の選択，async report の queue（UR-33） | `ur_33_two_transmit_streamers_keep_their_own_reports` | P217 `tx_send` が stream 0，P205 `tx_async` が stream 0 の queue → 同テスト |
| 送信の buffer 数・長さ（UR-33） | `ur_33_a_transmit_send_keeps_the_streamer_s_channel_count` | P227 個数の検査を外す，P228 長さの検査を外す → 同テスト |
| rates：格子 `(r, r, 0.0)`，未知の channel の拒否，pre-chain `{1.0}`，spec 変更で未接続に戻る（UR-33） | `ur_33_rates_report_the_grid_and_the_pre_chain_one` | P211 step を 1.0 に，P212 未知の channel を通す，P206 pre-chain を返さない，P229 spec 変更で履歴を消さない → 同テスト |
| `Applied.antenna`（UR-33，D16） | `ur_33_the_antenna_is_read_back_after_an_untimed_change` | P213 timed の変更でも antenna を返す → 同テスト |
| 構造体の大きさ 6 件（UR-3） | `uhd_api_struct_sizes_match_the_headers`（feature `uhd`） | （feature `uhd` のコードには行を作れない；§9） |
| `UhdDevice` の stream map（UR-2） | `uhd_streams_are_looked_up_by_number`（feature `uhd`，docker で実行） | 同上． |

## 6. 当て直した行（署名の変更で `old`/`new` が古くなったもの）

| 行 | 当て直した内容 | 壊す規則（同じ） |
|---|---|---|
| maintenance J323 | fake の「停止した stream の recv」の待ちの行が改名（`st.rx_next` → `streamer.next`） | 待たずに Timeout を返す |
| maintenance J326 | `rx_stop` の行が streamer 経由に | timed stop を honor する |
| maintenance J327 | `rx_start` の `else if` が `match` に | tail 中の start を tail の後ろに保たない |
| phase7 B01 | `rx_recv(0, …)` | cut で recv を bound しない |
| phase7 B15/B22 | `rx_packet_samples(0)` | ready instant の packet 項を落とす |
| phase7 B20 | 2 つの `tx_send(0, …)` | 継続の held sample を後に送る |
| phase7 B28 | `tx_send(0, &tail, …)` | held sample の送信を検査しない |
| phase7 B30 | `streamer.reports` | BurstAck を即返す |
| phase7 R12 | `new` が `rx_stop(0, Some(cut))`（当て直さないとコンパイルが通らず生き残った） | 予定された cut を timed stop として device に渡す |

全リストで `old` がちょうど 1 回一致し（maintenance 612，phase3 83，phase4 34，phase5 29，phase6 63，phase7 167），`old` ≠ `new` を機械で確認．

## 7. セルフレビュー（AGENTS.md §9 手順 2，Haiku 5.5 `anthropic/claude-haiku-5-5#high`，read-only）

- **1 回目**（`selfreview-1.log`）：P0 なし，P1 4 件・P2 6 件．直した：拒否された reopen が古い streamer を残す（rx/tx，テストと P219/P222），同じ列の重複（P218/P221），規則に行が無い分（P208〜P219 の 12 行＋テスト），`UhdDevice` の stream map の unit test，spec 変更の切断（fake と `UhdDevice`），`serial` の null と free，設計の段階名・空の列の扱い．指示書の一覧外だった UR-15/17/25 の呼び出し形は戻した．
- **2 回目**（`selfreview-2.log`）：P0 なし，P1 5 件・P2 10 件弱．直した：tx 側の 3 検査とテスト（P220〜P222），spec 変更の切断のテストと P226，`uhd_streams_are_looked_up_by_number` を UR-3 の Test 行へ，設計の「短く待つ」「空の列を名指ししない」の誤り，§6 の文面（重複・空は UHD より厳しい；carry-over に INFERRED とテスト），`table` の重複解消，serial の free，trait doc の precondition．
- **3 回目**（`selfreview-3.log`）：P0 なし，P1 2 件・P2 6 件．直した：`UhdDevice` が空の列を channel 0 として渡す（Rust 側の channel 数と一致；潜在の未定義動作を除去），fake の `tx_send` に buffer 数・長さの検査（P227/P228 と新テスト），`serial` の error 経路の free，spec 変更で `opened` を消す（P229），spec の順序が channel 順であることの assert．
  **残した P2（未解決として報告）**：(a) `rx_recv` の 10 ms 待ちは設計が書いているがテストは `Failed` だけを見る（mutation 行なし）；(b) `stream_cmd` が control を持ったまま stream の call を待つ（HEAD と同じ順序．1b-4b へ）；(c) UR-33 の段落に理由と参照が残る（DA-1 の形は後段の書き換えで切る）．
- 3 回で指摘は尽きていない（上記 (a)〜(c)）が，いずれもこの段階の誤りではなく「次の段階で決める」「行を足すならテストの設計から」のもので，指示書の「3 回」の上限に達したのでここで止めた．

## 8. 確かめること 1〜6 の結果（最終ツリー，すべて最後の編集の後に同じツリーで実行）

| # | コマンド | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | 65 suite すべて ok，1273 passed / 0 failed | `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/check1-final3.log` |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | warning/error 0 | `check2-final3.log` |
| 3 | docker（`ezsdr-v4-dev:uhd4.10`）: `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` → `cargo test -p ezsdr-radio-uhd --features uhd --test uhd_api` → `cargo test -p ezsdr-radio-uhd --features uhd --lib` | clippy 0，uhd_api 4 passed / 0 failed，lib 106 passed / 0 failed（`uhd_streams_are_looked_up_by_number` を含む） | `check3-final3.log` |
| 4 | `cargo build -p ezsdr-server` → `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=…/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests` | 47 tests，OK | `check4-final3.log` |
| 5 | `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problem | `check5-final3.log` |
| 6 | `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8` | 987/988 killed．生き残りは既知の `phase7/G09` のみ | `check6-final3.log`（途中の新規行・当て直し行の run は `check6-rows-new.log`，`check6-rows-new2.log`，それぞれ 30/30，8/8 killed） |

新しい行の run（`check6-rows-new.log`：P199〜P219・J323・J326・J327・B01・B15・B20・B22・B28・B30 = 30/30 killed；`check6-rows-new2.log`：P220〜P226・R12 = 8/8 killed）も同じツリーで流した．`plan/*/tools/mutations.json` の `old` は 6 リスト 988 行すべてちょうど 1 回一致することを機械で確認した．

## 9. 未解決のこと（1b-4b・1b-4c・owner へ）

- **1b-4b**：`UhdRadio` の呼び出しは今もすべて stream 0（`rx_recv(0, …)` 等）．`Scripted`（`provider/rx/differential.rs`）は stream 番号を無視している（コメント参照）．設計 UR-15・UR-17・UR-25 の呼び出し形はこの段階で変えていない．`stream_cmd` が control を持ったまま stream の call を待つ順序も HEAD のまま（UR-3 の Threads はこの順序を許す書き方）．
- **1b-4b（owner に諮る）**：channel 数を 0 にする cold change は `tx_open(0, &[])` を呼ぶ．UHD（と今の `UhdDevice`）はこれを channel 0 と読み，fake は channel のない streamer を開く．その stream は何も送らないので差は見えないが，どちらに揃えるかは stream ごとの段階で決める（設計 UR-33 に INFERRED として記載）．
- **1b-4c**：`set_subdev_spec` は spec を streamer より先に設定する前提（この段階の fake と `UhdDevice` は spec の設定でその方向の streamer を切る）．`opened`（`rates` の pre-chain の履歴）は spec 変更で消える，という今の規則もそこで確認する．
- **自己レビューの残り P2**：`rx_recv` の 10 ms 待ちのテストと mutation 行；UR-33 の段落の理由・参照を Changes へ移す DA-1 の仕上げ（spec 18 の他の規則と同時に）．
- **BP0（bench）**：§4 の INFERRED 全部．とくに pre-chain の rate，reconnect の文面，`serial`，Basic/LF の front end 名，antenna の読み返し．
