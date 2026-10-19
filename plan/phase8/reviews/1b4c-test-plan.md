# 1b-4c のテスト計画（DeepSeek V4.1 Flash が項目ごとに 16 本で書き，orchestrator がまとめた）

**この冒頭の裁定が，項目ごとの計画の本文より優先する．**

## orchestrator の裁定（実装の前に決めること）

### 構造

1. **`keys` の読み**
   - `keys` の値を読む関数は `ezsdr_radio` に 1 つだけ置く．MockRadio と UhdRadio の両方がそれを呼ぶ（今は Mock の `lib.rs:388-394` にある）．
   - 拒否の文は Mock と同じにし，UHD は `UR-5: ` を前に付ける．
   - 値 `"channel"` 以外は拒否する．文字列でないものも拒否する．
2. **prefix の制限を外す**（項目 2 の T7 を採る）
   - `x3xx::from_selector` の ORDER の loop は消すだけでなく，「同じ `(slot, frontend)` を 2 度名指さない」検査に置き換える．
   - その判断は `channels::check_layout` の重複の検査（`channels.rs:300-309`）と 1 か所にする．
   - P08・P09 は，その新しい検査に当て直す．
3. **両方のモードで開くときに行うこと**
   - 対象は次の 3 つ：subdev spec の設定，serial の拒否，`arm` の rate の集合の照合．
   - どれも装置の境界の規則で，key のモードとは関係ない．
   - 項目 14 の「今のモードにも入れるか」は**入れる**．今のモードの Manifest の `device` section にも同じ欄が出る．
4. **`from_binding` の中の順序**
   1. selector の解析（今のまま）
   2. `serial()` の読みと登録
   3. master clock
   4. `set_subdev_spec`（要素のある方向だけ）
   5. `channels`・`front_end` の照合
   - 拒否は装置に何かを書く前に決める（項目 5 の T2 の (ii)，H4）．
   - 登録は guard の値（`Drop` で外す）にする．`from_binding` が途中で失敗したら，guard が落ちて登録も外れる．
5. **`FakeConfig.serial`** は `String` にする．既定値は instance ごとに通し番号を振る（項目 5）．P199 は明示した値の検査で殺す．
6. **rate の集合の照合**
   - `arm` で，方向ごとに最初に開いた streamer の 1 channel について比べる．
   - テストは test wrapper `RatesMisreported` で行う．`FakeConfig` の knob は足さない（項目 4）．
7. **Manifest の `device` section**（項目 14 の形を採る）
   - `"serial": "<serial>"`
   - `"subdev_spec": {"rx": "<markup>"|null, "tx": …}`
   - `"rates_checked": {"rx": bool, "tx": bool}`（欄は欠かさない）
8. **keys のモードの `applied` の行**は具体の channel key で書く（`"key": "radio.rx.c1.frequency_hz"`．項目 15 の要求）．今のモードの行は HEAD のまま．
9. **antenna の read-back の fault**：`FakeFault::WrongAntenna { claimed, applied }` を足す（UR-33 に 1 行．#59 と同じ形）．
10. **stream 1 以上の拒否**
    - 規範の文は段階の話にしない（DA-1）．規則として書く：UhdRadio は各方向の stream 0 だけを動かす．channel を stream `k ≥ 1` に入れる設定は `prepare` で `Rejected`，更新は `COMMAND_REJECTED`．
    - 理由の文は `stream` と番号を名指す．
11. **VH-8 の比べ方**
    - `crates/ezsdr-acceptance/tests/parity.rs` に置く（PO-8．項目 15）．root は 200 MHz にそろえる．
    - 項目 15 の (b-1)〜(b-3) に，次の 2 つを足す：
      - (b-4) **床の列**．項目 11 の T2 と同じ操作・同じ instant を両 Provider に与え，`LATE_COMMAND` の requested/applied と clock の `ended_at` が一致すること．
      - (b-5) **channel の無い stream**．`<id>/rx/s1` 宛ての `start_rx` と `<id>/tx/s1` 宛ての `TxBurst` の扱いが両者で一致すること（項目 12 が送ったもの）．
    - これで項目 11 の T6（Mock 側の単独テスト）は採らない．比較が 1 か所で数値を固定する．
12. **項目 12 の harness の変更**を採る：`Pushed::recv` が target の path を node に解決する，宛先付きの push 補助，`ctx.inputs` に waveform を入れる入り口，event の pairs に stream の node．

### major（orchestrator が計画に足すもの）

- **M1（項目 10）：streamer の開き直しを channel 数で比べている**（`rx.rs:286` の `channels != self.opened`，`tx.rs:660` の `to.channels() != self.channels`）．
  - 項目 10 の計画は「同数の別集合はこの段階では作れない」として行を起こさないとしたが，**誤り**．同じ instant の 2 つの membership の変更で作れる．
  - 例：`c0.stream` を `0 → −1`，`c1.stream` を `−1 → 0`，両方 `at = T`．2 つ目は床で同じ `e` に載る．
  - そのとき集合は `{0}` → `{1}` で数は同じになり，今の比較では streamer を開き直さない．device channel 0 の samples が，論理 channel 1 として届く．
  - **修正**：
    - 開き直しの判定を，channel の**集合**（`Channels`）の比較にする．rx の `opened`，tx の switch，uhd-control の from-none．
    - テスト `ur_25_a_same_size_membership_swap_reopens_the_streamer`（受信と送信）を足す．
      - 上の 2 つの変更を同じ `at` で与える．
      - 期待：`rx_open 0 [1]`（送信は `tx_open 0 [1]`）がちょうど 1 回あること，新しい segment の `apply rx 1 … at=-` があること，`apply rx 0 … at=-` が増えないこと．
    - 受信では，FakeDevice の channel ごとの loopback で，block が channel 1 の内容であることも見る（1b-4a の per-channel loopback）．
    - mutation の行は「集合の比較を数の比較に戻す」形．今のモードでは数と集合が一致するので，既存のテストの結果は変わらない．

### 各項目の裁定（minor・確認）

- **項目 1**：採る．C5 の行（`coerce` と `tree` が別の description）は，モードの選択が 1 か所の `Profile` 相当にあるなら，その選択を壊す 1 行に統合してよい．
- **項目 2**：採る．T0 の純関数のテストは，markup の生成を関数にしたときだけ書く．T7 は上の構造 2．
- **項目 3**：採る．`fake.rs:169,171` の文が UHD の lookup の失敗の文に変わるのは，意図した変化の 3 つ目として記録する（final の A3 の 1）．
- **項目 4**：採る．範囲 `(start, stop, step)` の展開は，fake が singleton しか返さない．そのため，比較を純関数にして unit テストで範囲の展開も固定する（BP0 の前に UHD の meta range の形を受けられるように）．
- **項目 5**：採る．
  - T6（`ezsdr-server` の開き直しのテスト）は必須．
  - USRP2 の probe（`hardware.rs:290-305`）の期待する文は，serial が先に読まれることで変わりうる（INFERRED）．BP0 の確認事項に足す．
- **項目 6**：採る．T7 の fault は構造 9．
- **項目 7**：採る．
  - T3b の queue の数え方（channel key の 1 命令で 1 枠）は，今のモードの「方向の channel 数で数える」と並べて，UR-24 の文に書く．
  - 記録だけの行は `note: "no channel"` の形のまま，key を具体の channel key にする（構造 8）．
- **項目 8**：採る．T2（channel の無い stream の rate の変更は値だけになる）も採る．
- **項目 9**：採る．
- **項目 10**：M1 を足して採る．T6 の後半（拒否が値を残さない）も採る．
- **項目 11**：採る．T6 は構造 11 で parity に移す．C5（今のモードは自分の line のまま）の行は必須．
- **項目 12**：採る．
- **項目 13**：採る．
- **項目 14**：構造 7 のとおり．T3 の (iv) を採る．
- **項目 15**：構造 11 のとおり．比べないもの（device の calls，UHD だけの拒否）は doc comment に書く．
- **final**：A3 の 3 件と，B2 の design の一覧を採る．
  - 追加：MR-18 の `keys` の床の文は RM-25 の引用と Mock の機構だけにする（DA-1．項目 11 の final 送り）．
  - 追加：RM-26（`design/07-radio-model.md:295`）の「from a stream's initial state」に，床の関数とその入力を足す．

### Sol のテスト計画レビュー（`sol-plan.md`）の裁定：全件採用

項目ごとの計画の本文より，ここが優先する（上の orchestrator の裁定とは矛盾しない）．

- **major 1（項目 6 の clock の観測）**
  - `prepare`・`arm` の受信の clock は，`declared_sample_clocks()` で node と比を見る．`sample_clock_records()` の登録は 0 本．受信の登録は最初の block（`rx.rs:492-495`）．
  - 登録の record は，start の後の最初の block を待つ別のテストで見る．
  - channel の無い stream（T5）は，宣言も無いことを見る．
- **major 2（parity の fixture と比べるもの）**
  - fixture は受理できる形に直す．
    - FakeConfig は 1 slot に 1 board なので，混在の rx と tx は slot ごとに同じ board の layout にする．
    - case 3 の UBX c1 に 1 MHz や antenna `A` を与えない．
    - RM-31 は受信が stream に入っていなくても拒否する（`channels.rs:778-810`）．そのため受理側の列では，同じ slot の送信を stream に入れない．
  - 比べるのは**論理の更新**（key，値，effective instant）だけにする．
    - Mock は `lib.rs:1052-1057` の行．UHD は Action ごとの記帳（`cold_change`，timed の `issued`，記録だけの行）から取り出す．
    - 取り出し方を parity の doc comment に書く．`prepare` と restart の configure の read-back の行は比べない．
  - 送信の最初の origin は，各 Provider の arm instant に対する lattice の式で見る．受信と cold の共通 instant は値で比べる．
- **major 3（serial）**
  - T4 は `serial()` だけが固有のエラーを返す wrapper（`SerialFails`）で行う．
    - 理由の文がそのまま伝わること．
    - その後の device の書き込み（`set_subdev_spec` 以降）が 0 件であること．
  - 足すテスト：
    - **同時の bind**：`std::sync::Barrier` で 2 thread を揃え，同じ serial で `from_binding` する．成功した instance を保持したまま，成功が 1 件だけであることを見る．
    - **`cleanup()` だけでは外れない**：`cleanup()` の後も，instance が生きている間は重複を拒否する．drop の後は受ける．
  - 登録の「検査して入れる」は 1 回の lock の中で行う．
- **major 4（effective instant の投影）**
  - 項目 13 に次の列を足す．
    - **(a) 床上げ後の RM-31**：同じ slot で，送信の channel が stream に入っていない状態から始める．
      1. 受信の antenna `TX/RX` を，future の cold（`T+50 ms`）で予約する．
      2. 送信の join を，それより早い instant（`T+10 ms`）で要求する．床で `T+50 ms` に上がる．
      3. 上がった後の設定で RM-31 に当たり，拒否される．
    - **(b) 到着順の両方**：c1 の future join と，その後に効く timed の tune を，到着順を変えて 2 通り入れる．発行先が今の membership ではなく，effective instant の membership で決まることを見る．
  - どちらの列も，拒否の後に別の有効な更新を送り，拒否した値と instant が projection と床に残らないことを確かめる．
- **major 5（最初の実行時の open の rate の照合）**：照合は `arm` だけでなく，**方向ごとに最初に streamer を開いたとき**に行う．
  - 対象は `arm`，uhd-control の from-none の open，uhd-rx・uhd-tx の reopen．
  - 同じ関数を呼び，方向ごとの「比べた」の状態は instance の 1 か所に持つ．
  - 実行時の不一致は，その segment の設定の拒否として扱う（RM-25 の halt と `COMMAND_REJECTED`，理由は `UR-13: `）．`rates_checked` はそれを反映する．
  - テストを足す：
    - 送信の channel が全部 stream に入っていない状態で arm し，格子の違う device で送信の join をする → 拒否と halt．
    - `rates()` の read error（wrapper）→ 同じく拒否．
- **major 6（stream 0 の event の source）**
  - keys のモードで，受信の overflow（`FakeFault::Overflow`）と送信の async error（`FakeFault::TxReport`）を 1 件ずつ起こす．
  - source がそれぞれ `<id>/rx/s0`・`<id>/tx/s0` であること，event が消えていないことを見る．
  - collector には新しい source だけを許す．overflow の handle を古い node に戻す mutation の行を足す．
- **minor 1**：項目 7 の H8 を書き分ける．
  - 「物理の queue は RFNoC の block ごとで，各 queue の中は FIFO」：VERIFIED（`timed_cmd.dox:94-111`）．
  - 「instance 全体で 1 列」：Provider の規範のモデル（設計 33 D9）．
  - 項目 4 の送信の pre-chain は，条件付きの経路を VERIFIED にする（`duc_block_control.cpp:152-164`，`multi_usrp_rfnoc.cpp:2030-2037`）．「streamer の前なら必ず `{1.0}`」は INFERRED のまま．
- **minor 2**
  - 「何も変えない」の assert は，変更系の call（`apply`・`*_open`・`*_start`・`*_stop`・`set_subdev_spec`・`tx_send`）に限る．
  - 送信が無傷であることの対照は，送信を明示して有効にしてから作る．

### 1b-4d・1b-4e へ送るもの

final の「1b-4d へ送る一覧」1〜10 と「1b-4e へ送る」のとおり．

## 項目

## 1. keys の値

### (a) 既存のテスト

**同じ規則を Mock 側で固定しているもの**

- `crates/ezsdr-mock-radio/tests/mock_layout.rs:431` の `mr_38_a_layout_instance_uses_one_key_set` が，この順序の規則そのものを持つ．
  - `:447-449`: `"layout"`，`1`，`["channel"]` を `selector \`keys\` must be the string \`channel\`` で拒否する．
  - `:461-468`: `keys: "channel"` の instance は `radio.rx.frequency_hz` を `RM-4: not a radio key` で拒否し，`keys` の無い instance は `radio.rx.c0.frequency_hz` を同じ理由で拒否する（新旧の key set が交わらない）．
  - `:469-472`: layout instance は自分の `radio.rx.c0.frequency_hz` を受け付ける．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:511-531`: layout instance の `effective` が RM-27 の key 集合だけを持つこと（[item 6] の正の側の鏡）．
- `design/09-mock-radio.md:218`（MR-38）: 「one instance uses one key set, never both」。UHD の UR-5 に入れる規則の文面の基準．
- `plan/phase8/reviews/1b3b-report.md:24`，`:99`，`:220`: 1b-3b のモード切り分け，テストの要約，1b-4 への申し送り（`Profile` の `Mode` と同じ形を使う）．

**今のモードの振る舞いを固定しているもの（変えてはいけない）**

- `crates/ezsdr-radio-uhd/tests/fake.rs:95-131` の `ur_05_from_binding_refusals`．`:100` が `keys` なしの binding を通し，`:110-123` が未知の selector key（`"fake": true` を含む）を拒否する．`keys` を allowed に足しても `fake` は未知のまま（`:2022-2029` の `ur_33_no_document_selects_the_fake_device` が `:2027-2028` で確認）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:162-181` の `ur_05_the_profile_must_be_the_device_s_front_ends`．`:179-180` が `keys` なしの mixed layout を `one device carries one kind` で拒否する．`keys` なしは今の parser のままであることの固定．
- `crates/ezsdr-radio-uhd/tests/fake.rs:509-517` の `ur_11_coerce_is_the_descriptions`．今の key set（`radio.rx.sample_rate_hz` など）と coerce の結果．
- `crates/ezsdr-radio-uhd/tests/fake.rs:703-720` の `ur_12_prepare_refusals`．`:711` の制約なし `prepare` が成功し，`:715-719` の各拒否が `prepare` 経由で返る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:469-488` の `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section` と `:490-500` の `ur_10_instance`．`keys` なしの capabilities・envelope・instance の形．
- `crates/ezsdr-radio/tests/device.rs:93-118` の `rm_26_the_description_builds_the_tree`: 今の description の tree は `dev/rx` と `dev/tx`．
- `crates/ezsdr-radio/tests/channels.rs:320-354` の `rm_29_the_layout_description_builds_the_stream_tree`: layout description の tree は `dev/rx/s0…`，port は `rx0…`．
- `crates/ezsdr-mock-radio/tests/mock_radio.rs:309-419` の `mr_02_from_binding_refusals` と `:696-702` の `mr_08_effective_holds_exactly_the_ten_configuration_keys`: Mock の今のモードの selector 拒否と `effective` の 10 key．

**UHD 側に `keys` selector のテストは無い**

- `grep -n '"keys"' crates/ezsdr-radio-uhd/tests/fake.rs crates/ezsdr-radio-uhd/src/provider/mod.rs` は 0 件（exit 1）．UHD の `keys` は `src/provider/mod.rs:345` の `keys::CONFIGURATION` だけである．

### (b) 足すテスト

#### 1. `ur_05_the_keys_value_picks_one_key_set`

- **置き場所**: `crates/ezsdr-radio-uhd/tests/fake.rs`，`ur_05_from_binding_refusals` の直後（`:131` の後）．
- **操作の列**:
  1. `current` を `binding(json!({ "args": ARGS }), Some(x3xx()))` と `FakeConfig { serial: "ur05-current", ..FakeConfig::default() }` から `UhdRadio::from_binding` で作る（`keys` なし）．
  2. `layout` を同じ selector に `"keys": "channel"` を足したものと `FakeConfig { serial: "ur05-layout", ..FakeConfig::default() }` から作る．`binding()` helper（`:82-93`）が `rx`/`tx` の既定（`0:A:ubx:0`, `0:B:ubx:0`）を入れるので，layout は 2 channel ずつ・prefix の UBX になる．
  3. `current.coerce(&request(...))` に `ezsdr_radio::keys::CONFIGURATION` の 10 key をすべて有効な値（channels 1，rate 1 Msps，frequency 1 GHz，gain 0 dB，antenna `RX2`/`TX/RX`）で渡す．
  4. `layout.coerce` に同じ 10 key を渡す．
  5. `layout.coerce` に `radio.rx.c0.frequency_hz` 1 GHz，`radio.rx.c0.gain_db` 0 dB，`radio.rx.c0.antenna` `"RX2"`，`radio.rx.c0.stream` 0，`radio.rx.s0.sample_rate_hz` 1 Msps を渡す．
  6. `current.coerce` に同じ 5 key を渡す．
- **assert**:
  - 手順 3: `report.rejected.is_empty()`，`report.applied` が 10 key すべてを含む（値は要求どおり）．
  - 手順 4: `report.rejected.len() == 10`，各 `reason` が `"RM-4: not a radio key"`，`report.applied.is_empty()`．
  - 手順 5: `report.rejected.is_empty()`，`report.applied` が 5 key すべてを含む．
  - 手順 6: `report.rejected.len() == 5`，各 `reason` が `"RM-4: not a radio key"`．
  - `current.instance().tree.children` の id が `["usrp/rx", "usrp/tx"]`，ports が `["rx"]`．
  - `layout.instance().tree.children` の id が `["usrp/rx/s0", "usrp/rx/s1", "usrp/tx/s0", "usrp/tx/s1"]`，ports が `["rx0", "rx1"]`．
- **観測**: `coerce` の `CoerceReport`（`applied` / `rejected`）と，`from_binding` が作った `instance().tree`．device には `from_binding` の読み（master clock，channel，front end，item 2 の spec）以外をさせない．`keys` の選択が `coerce` と `tree` の両方に届いていることを見る．
- **注意**: `current` と `layout` を同時に生かすので，serial は明示的に変える（item 5 の「既定の serial は instance ごとに違う」に依存しない）．今の `FakeDevice` の既定 serial は `"FAKE0001"` で共通（`crates/ezsdr-radio-uhd/src/device.rs:377`）なので，両方 `FakeConfig::default()` にすると item 5 実装後に同じ serial で拒否されうる．

#### 2. `ur_05_every_other_keys_value_is_refused`

- **置き場所**: 同じく `crates/ezsdr-radio-uhd/tests/fake.rs`（テスト 1 の直後）．
- **操作の列**: `"keys"` に `"layout"`, `"Channel"`, `"channel "`, `""`, `1`, `true`, `["channel"]` を順に入れた binding を作り，それぞれ `UhdRadio::from_binding` を呼ぶ．selector には `args` と，`binding()` helper が入れる既定の `rx`/`tx` を持たせる．
- **assert**: 各値で `Err` が返り，`error.kind == ModuleErrorKind::Rejected`，`error.message == "UR-5: selector \`keys\` must be the string \`channel\`"`．`Ok` が返ったら `unwrap_err` で fail．
- **観測**: `from_binding` の `ModuleError` だけ．device の calls は見ない（`keys` の解析が device に触れる前かどうかはこの規則の対象外）．
- **注意**: この文面は Mock の `mock_layout.rs:449` と同じにし，UHD では `"UR-5: "` を付ける．spec 18 UR-5 の selector 表にこの文面を入れる（[item 16] の design 変更一覧）．

#### 3. `ur_12_each_key_set_refuses_the_other_in_prepare`

- **置き場所**: `crates/ezsdr-radio-uhd/tests/fake.rs`，`ur_12_prepare_refusals`（`:703`）の直後．
- **操作の列**:
  1. `layout` を `keys: "channel"`（既定の 2 UBX）で作り，`direct_prepare`（`:535-568`）に `("radio.rx.frequency_hz", 1 GHz)` を渡す．`direct_prepare` には `from_binding` に渡したのと同じ `Arc<FakeDevice>` を渡す．
  2. `current` を `keys` なしで作り，`direct_prepare` に `("radio.rx.c0.frequency_hz", 1 GHz)` を渡す．
- **assert**: 両方とも `Err`，`kind == ModuleErrorKind::Rejected`，`message == "UR-12: RM-4: not a radio key"`．同じ `FakeDevice` の `calls()` に `"apply "` で始まる call が無い（`prepare` が `coerce` の拒否で `Core::new` と `configure` に進んでいないこと）．
- **観測**: `prepare` の `ModuleError` と device の `calls`．正の側（layout の `prepare` が `radio.rx.c0.frequency_hz` を受けて `PrepareReport.effective` に載せること，channel ごとの `apply`）は item 6 のテストが持つ．ここでは「相手の key set を受けない」だけを固定する．

`keys` の値の判定を `ezsdr_radio` の 1 関数（Mock の `Profile::Mode` と同じ形）に移すなら，テスト 2 の値のループはそちらの unit test に置き，UHD 側のテスト 1・3 は `from_binding`/`prepare` がその判定を使う配線だけを見る．同じ判断を 2 か所に書かない．

### (c) 壊れうる形と mutation の行の案

行の ID は実装時に空き番号を取る（P122 が Mock の同じ行の先例）．`old` の正確な文字列はコードと一緒に決める．

- **C1: `keys` を selector key として受理しない**
  - 形: `crates/ezsdr-radio-uhd/src/provider/mod.rs:124-127` の allowed の chain に `"keys"` が無い．`keys: "channel"` が `"UR-5: unsupported selector key \`keys\`"` になる．
  - 落ちる assert: テスト 1 の `layout` の `from_binding(...).unwrap()` が panic．テスト 3 の layout 構築も panic．テスト 2 は文面が `unsupported selector key` になり `assert_eq!` が落ちる．
  - 行案: ファイル `crates/ezsdr-radio-uhd/src/provider/mod.rs`，`old` = `.chain(["args", "id", "clock_source", "time_source", "block_len", "reopen_on_unlock", "keys"])`（実装後の行），`new` = `"keys"` を落とした同じ行．`filter` = `ur_05_the_keys_value_picks_one_key_set`．
- **C2: `"channel"` 以外の値も layout にする**
  - 形: `old` = `Some(Value::Scalar(Scalar::Str(value))) if value == "channel" => true,`，`new` = `_ => true,`（P122 と同形）．`"layout"` や `1` でも layout instance ができる．
  - 落ちる assert: テスト 2 の `"layout"` の `unwrap_err` が panic（`Ok` が返る）．`"Channel"` も同様．
  - 行案: ファイル `src/provider/mod.rs`，`filter` = `ur_05_every_other_keys_value_is_refused`．
- **C2b: 文字列以外・未知の値が今のモードに落ちる**
  - 形: `old` = `_ => return Err(rejected("UR-5: selector \`keys\` must be the string \`channel\`")),`，`new` = `_ => false,`．`keys: 1`，`true`，`["channel"]` が今のモードとして通る．
  - 落ちる assert: テスト 2 の `1` / `true` / list の `unwrap_err` が panic．
  - 行案: 同じファイル・同じ `filter`．
- **C3: `keys` が無い instance を layout にする**
  - 形: `old` = `None => false,`，`new` = `None => true,`．
  - 落ちる assert: テスト 1 の `current` の 10 key がすべて拒否され `rejected.is_empty()` が落ちる; tree が `usrp/rx/s0…` になり `["usrp/rx", "usrp/tx"]` の assert が落ちる．テスト 3 の current の `prepare` も `radio.rx.c0.frequency_hz` を受け付けてしまい `Err` の assert が落ちる．
  - 行案: 同じファイル，`filter` = `ur_05_the_keys_value_picks_one_key_set`．
- **C4: `keys` を読むが，モードの選択を無視して常に今の description を使う**
  - 形: `old` = `keys: "channel"` のときに `channels::Description` を選ぶ腕（`Profile::Mode::Layout` 相当），`new` = 常に今の description（`DeviceDescription`）．`keys` の値の解析だけが残る．
  - 落ちる assert: テスト 1 の `layout` の 10 current key が受理され `rejected.len() == 10` が落ちる; layout の 5 key が拒否され `rejected.is_empty()` が落ちる; tree も `usrp/rx` になる．テスト 3 の layout の `prepare` も foreign key を受け付ける．
  - 行案: ファイル `src/provider/mod.rs`，`old` = モード選択の式，`new` = 今の description 固定．`filter` = `ur_05_the_keys_value_picks_one_key_set`．
- **C5: `coerce` と `tree` が別の description を使う（モード判定の二重実装）**
  - 形: `old` = `tree: description.tree(&resource, HOST_MEMORY),`（今の行），`new` = 常に今の description の tree，または `coerce` 側だけ今の description．`mech:radio-rule-implemented-twice` の芽．
  - 落ちる assert: テスト 1 の layout の tree（`usrp/rx/s0…`）または `coerce`（10 key の拒否）のどちらか．
  - 行案: ファイル `src/provider/mod.rs`，`old` は実装後の該当行，`new` はもう一方の description．`filter` = `ur_05_the_keys_value_picks_one_key_set`．既存行 `J277`（`plan/maintenance/tools/mutations.json`，`filter = ur_10_instance`）が `tree: description.tree(...)` に当たっているので，`description` の型を変えたら `J277` の当て直しも要る．
- **C6: 拒否の文面を一般化する**
  - 形: `old` = `"UR-5: selector \`keys\` must be the string \`channel\`"`，`new` = `"UR-5: selector \`keys\` must be a string"`．
  - 落ちる assert: テスト 2 の `assert_eq!(message, ...)`．
  - 行案: 同上，`filter` = `ur_05_every_other_keys_value_is_refused`．文面を規範にするなら入れる（入れないならテスト 2 は `contains("selector \`keys\`")` に弱めるが，Mock と揃える方針なら入れない）．

**この順序の範囲外の壊れ方**: `keys: "channel"` でも `x3xx::from_selector` を先に呼ぶ（mixed/reversed layout を拒否する）形は，テスト 1 が prefix の UBX だけを使うので落ちない．これは item 2 の `channels::from_selector` と prefix 制限のテストが殺す．item 1 のテストを mixed layout に広げない理由は，item 1 を item 2/3 の `set_subdev_spec`・front end 照合から独立させるため．

### (d) 頼る振る舞い

- **H1** (`multi_usrp_rfnoc.cpp:2822-2834`, VERIFIED): `set_subdev_spec` がその方向の channel map を作り直し，spec の順が channel の順になる．テスト 1/3 の layout instance は，item 2 が `"A:0 B:0"` を設定した後に `channels(Rx)=2`，`front_end(Rx,1)` が slot B の UBX であることに依存する．
- **H2** (`subdev_spec.cpp:19-20,58-59`, VERIFIED): markup は `<slot>:<fe>` を ASCII の空白 1 つで並べる．item 2 が `"A:0 B:0"` を作ることを前提にする．
- **H5** (`db_ubx.cpp:562`, `db_basic_and_lf.cpp:101-111`, VERIFIED): front end の名前は `UBX RX`，Basic/LF は `BasicRX (0)` のように番号を持つ．テスト 1/3 の `from_binding` の front end 照合が `UBX RX` を見る．
- **使わないもの**: H3（rate の集合は item 4 の `arm`），H4（同じ装置の 2 回 open は item 5 の serial），H6（antenna の read-back は item 9），H7（空・重複の spec はこの順序で作らない）．
- この順序の **key set の選択そのもの** は UHD の振る舞いではなく，Provider の selector 解析（`design/09-mock-radio.md:218` MR-38，`design/18-uhd-radio.md:124` UR-5，`:187` UR-10，`:191` UR-12）である．表に無い新しい UHD の仮定は無く，VERIFIED/INFERRED の追加も無い．
- `keys` は一時的な key で，1b-5 で消す（`plan/phase8/33-channels-and-streams.md:265`）．この順序のテスト名も 1b-5 で消える/改名される．

### 関わる他の順序

- **2（layout から subdev spec，prefix 制限の撤去）**: テスト 1/3 の layout instance は `from_binding` の中で `set_subdev_spec` を通る．item 2 の `"A:0 B:0"` / `"A:0"` が H1/H2 どおりでないと，key set の assert に届く前に落ちる．mixed/reversed layout が `channels::from_selector` を通ることは item 2 のテストが持つ（item 1 は prefix の UBX だけを使う）．
- **3（front end の照合が spec の後）**: テスト 1/3 の layout 構築は `front_end` の照合を通る．UBX の名前（H5）だけに依存し，Basic/LF の番号付き名前は item 3 のテストが持つ．
- **5（同じ serial の拒否と解放）**: テスト 1/3 は `current` と `layout` を同時に生かすので，`FakeConfig { serial: ... }` を明示的に変える．`ur_05_from_binding_refusals` は成功した radio をその場で drop してから次を作るので，item 5 の登録があっても壊れない．既定 serial の一意化そのものは item 5 のテスト．
- **6（`prepare` の stream 0，channel ごとの設定，`effective`）**: テスト 3 は拒否だけ．layout の `prepare` が `radio.rx.c0.frequency_hz` を受けて `PrepareReport.effective` に載せること，`radio.rx.s0.sample_rate_hz` を読み，channel ごとに `apply` することは item 6 のテストが持つ．`Core::settings`/`Core::channels` の layout 分岐も item 6．
- **14（Manifest）**: `keys` の値は Manifest に出ない（`from_binding` の `device` section は `describe()` と `args` だけで，selector を含まない）．方向ごとの spec の markup，serial，rate 照合の結果は item 14 が持つ．item 1 は section を増やさない．
- **15（VH-8）**: 同じ `keys: "channel"` の selector と Spec を Mock と UHD に与える比較は item 15．テスト 1 の `layout` instance（2 channel UBX）をそのまま UHD 側に使える．stream 1 本の比較だけを行う．
- **1b-5**: `keys` を消す段階で，テスト 1〜3 の「`keys` なし/`"channel"`」の分岐は不要になる．モードの判定を `ezsdr_radio` の 1 関数に移した場合，その unit test も同時に整理する．

## 2. layout から subdev spec を作る

対象は `crates/ezsdr-radio-uhd/src/provider/mod.rs:111-227` の `from_binding`（今の front end の照合は `:170-192`，`describe()` は `:193`）．今のモードの要素は `x3xx::from_selector` の `Selection`（`crates/ezsdr-radio/src/x3xx.rs:494-582`），`keys: "channel"` のモードの要素は `channels::from_selector` の `Layout`（`crates/ezsdr-radio/src/channels.rs:150-209`，要素の型は `:37-46`，`Layout` の `rx`/`tx` は `:62-65`）．markup は各要素を `<slot>:<frontend>` にして ASCII の空白1つで並べる（H2）．`set_subdev_spec` は FakeDevice ではその方向の channel map を作り直し（`crates/ezsdr-radio-uhd/src/device.rs:831-862`），UHD でも同じ（H1）．よって `device.channels`・`device.front_end`（`device.rs:154,156`）と，その後に呼ばれる `describe()`（`mod.rs:193`，`rx_channels`/`tx_channels` は `device.rs:794-795`）より前に置く必要がある．設計の根拠は `plan/phase8/33-channels-and-streams.md:131`（D5）と `:266`（1b-4 の行）．

### (a) 既存のテスト

**今のモードの振る舞いを固定しているもの（変えない）**

- `crates/ezsdr-radio-uhd/tests/fake.rs:96` `ur_05_from_binding_refusals` — 今のモードの拒否の一覧．`rx: []`,`tx: []` の拒否（`:120`）は parser（`x3xx.rs:557-559`）のままで不変．ここに spec の観測は無い．
- `crates/ezsdr-radio-uhd/tests/fake.rs:162` `ur_05_the_profile_must_be_the_device_s_front_ends` — front end の照合の文言を固定している．spec を先に設定すると，`:169`（`boards(vec!["UBX"])`）と `:171`（`boards(vec![])`）の期待が「channels が足りない」から `set_subdev_spec` の失敗（H7 の lookup error）に変わる．文言は順序 3 の担当．
- `crates/ezsdr-radio-uhd/tests/fake.rs:723` `ur_12_a_tx_only_layout_declares_no_receive_clock` — 今のモードで要素の無い方向を持つ唯一の既存テスト（`rx: []`）．clocks と applied だけを見て，spec の calls は見ない．
- `crates/ezsdr-radio-uhd/tests/fake.rs:139` `ur_09_the_obx_is_one_channel_from_10_mhz_to_8_4_ghz_defaulting_to_1_ghz` と `:184` `ur_09_the_cbx_is_one_channel_from_1_2_ghz_defaulting_to_2_45_ghz` — 1 board の layout で `from_binding` が通ること．結果は不変（calls が増えるだけ）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:110-123` — 悪い selector の拒否（`args` 無し，型違い，`rx`/`tx` の両方が空 など）．

**装置側で spec の意味を固定しているもの**

- `crates/ezsdr-radio-uhd/tests/fake.rs:2032` `ur_33_the_front_end_table_follows_the_subdev_spec` — spec が `channels`/`front_end` を作り直し，spec の順が channel の順になること（`:2053-2057`）．作る markup が満たすべき装置側の規則．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2073` `ur_33_a_subdev_spec_uhd_would_refuse_is_refused` — 空・重複・壊れた markup の拒否．`:2107-2111` のタブのケースが H2（区切りは ASCII の空白だけ）を固定している．作る markup が踏んではいけない形の一覧でもある．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2290` `ur_33_setting_a_spec_disconnects_that_direction_s_streamers` — H1（他方の方向の streamer は残る）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2310` `ur_33_a_timed_command_follows_its_front_end_across_a_spec` — `"B:0 A:0"` の後，channel 0 が B の front end になること（`:2332-2335`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2339` `ur_33_rates_report_the_grid_and_the_pre_chain_one` — spec の変更は chain を切り直す（`:2357-2358`）．
- `crates/ezsdr-radio-uhd/src/uhd.rs:1374-1400` と `crates/ezsdr-radio-uhd/tests/uhd_api.rs:47-56` — `UhdDevice::set_subdev_spec` の C の経路と handle の寿命（1b-4a のまま）．

**layout の側と，この段階で期待が動くテスト**

- `crates/ezsdr-radio/tests/channels.rs:208` `rm_28_the_layout_analysis_cases` — `owner_layout()`（`:74-80`）が指示書の例そのもの（rx 3，tx 2）．要素の `(slot, front_end, frontend)` を `:215-219` で固定している．markup の元になる要素の並びの根拠．
- `crates/ezsdr-radio/tests/channels.rs:223-224` — layout の parser は slot B だけ・既定でない順を既に受ける．今のモードの parser だけが受けていない．
- `crates/ezsdr-radio/tests/device.rs:182` `rm_26_the_layout_parser_checks_the_shared_selector` — **prefix の 3 ケース（`:201-203`）と文言の確認（`:210-212`）を書き換える**（ここで外す制限を固定している）．受理側に回し，`:190-193` の形で `rx[0].slot == Slot::B` を確かめる．
- `crates/ezsdr-mock-radio/tests/mock_radio.rs:309` `mr_02_from_binding_refusals` — **prefix のケース（`:341-342`，`:348-363`）を書き換える**．MockRadio の今のモードも同じ `x3xx::from_selector` を通る（`crates/ezsdr-mock-radio/src/lib.rs:442-445`）ので，制限を外すとこの結果が動く．

**「無い」ことの確認（実行した検索）**

- `grep -rn "set_subdev_spec" crates/ezsdr-radio-uhd/tests/` → `from_binding` が設定する spec を見るテストは無い（device を直に叩く `fake.rs:2042`… と wrapper の転送だけ）．
- `grep -rn '"keys"\|keys: "channel"' crates/ezsdr-radio-uhd/` → 一致なし（layout モードの UhdRadio テストはまだ無い）．
- `grep -rn "assert_eq!(device.calls()\|assert_eq!(calls," crates/ezsdr-radio-uhd/tests/ crates/ezsdr-radio-uhd/src/` → 完全一致は `crates/ezsdr-radio-uhd/src/provider/tx.rs:857` だけで，その rig は `from_binding` を通らない（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:14-66`）．`tests/fake.rs` の `calls()` 31 箇所はすべて prefix で絞る（`crates/ezsdr-radio-uhd/tests/common/mod.rs:452-454`）．
- この段階で偽になる規範文：`design/18-uhd-radio.md:124`（UR-5 の prefix の文），`:130`（selector の表），`design/09-mock-radio.md:39`（MR-2 の表），`design/07-radio-model.md:465`（`rm_26_...` の行）．`x3xx.rs:568-580` のコメントはコードと一緒に消える．一覧の集約は final の担当．

### (b) 足すテスト

**T0（lib，builder を純関数にした場合）`ur_02_a_subdev_spec_markup_names_each_element_s_pair`**

- 置き場：`crates/ezsdr-radio-uhd/src/device.rs` の `mod tests`（`:1615`）．`subdev_markup` を `parse_spec` の隣（`:597` の近く）に置くなら，ここで直接叩ける．`provider/mod.rs` に inline で書くなら T0 は省き，T1・T2 に吸収する．
- 操作と assert：`[(A,0),(B,0),(B,1)]` → `"A:0 B:0 B:1"`，`[(B,0),(A,0)]` → `"B:0 A:0"`，`[]` → `""`．観測は戻り値だけ（device は呼ばない）．

**T1 `ur_05_the_layout_builds_the_subdev_spec`（integration，`tests/fake.rs`）**

- device 1：`FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..FakeConfig::default() }`．binding は `json!({"args": ARGS, "keys": "channel", "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"]})`，profile は `x3xx()`（`binding()`，`fake.rs:82-93`）．
- `UhdRadio::from_binding` が `Ok`．`calls(&device, "set_subdev_spec")` に `"set_subdev_spec rx A:0 B:0 B:1"` と `"set_subdev_spec tx A:0"` が**両方**あり，ほかに `set_subdev_spec` が無い（方向間の順序は規範でないので，並びは固定せず集合で見る）．
- `device.channels(Dir::Rx) == 3`，`front_end(Rx,0) == "UBX RX"`，`(Rx,1) == "BasicRX (0)"`，`(Rx,2) == "BasicRX (1)"`，`channels(Dir::Tx) == 1`．ただしこの fake の既定の spec が既に `A:0 B:0 B:1` と同じ（`device.rs:548`）なので，状態の assert は整合性の確認で，呼び出しの assert が本体．
- `radio.instance().sections[&Namespace::parse("ezsdr.radio.uhd.device").unwrap()]["rx_channels"] == 3`（`describe()` が spec の後であることの観測；`fake.rs:487` と同じ読み方）．
- device 2：`FakeConfig { front_ends: vec!["UBX", "LFTX"], .. }`，`tx: ["0:A:ubx:0", "0:B:lf_tx:0"]`，`rx: ["0:A:ubx:0"]` → `"set_subdev_spec tx A:0 B:0"`，`front_end(Tx,1) == "LFTX (0)"`．
- 2 台に分ける理由：fake は slot ごとに 1 board・両方向（`device.rs:347-350`）で，例の slot B は rx では BasicRX，tx では LFTX なので 1 台には同居できない．

**T2 `ur_05_the_layout_s_order_is_the_device_s_channel_order`（integration，`tests/fake.rs`）**

- device `["OBX", "UBX"]`（slot A が OBX），layout の rx は `["0:B:ubx:0", "0:A:obx:0"]`，tx は `["0:A:obx:0"]`．
- `set_subdev_spec rx B:0 A:0` が calls にあり，`front_end(Rx,0) == "UBX RX"`，`(Rx,1) == "OBX RX"`（既定の `A:0 B:0` なら 0 が OBX なので，slot 順に並べ替える誤り・反転を落とす）．

**T3 `ur_05_a_direction_without_elements_sets_no_spec`（integration，`tests/fake.rs`）**

- 今のモードと `keys: "channel"` の両方で，rx `["0:A:ubx:0"]`，tx `[]`，既定の fake．
- `from_binding` が `Ok`，`calls` に `set_subdev_spec tx` が無い，`device.channels(Dir::Tx) == 2`（既定のまま）．空文字で呼ぶ実装は fake が拒否する（`device.rs:638-642`）ので `Ok` 自体が落ちる．

**T4 `ur_05_the_current_keys_set_uhd_s_default_order`（integration，`tests/fake.rs`）**

- 今のモード，rx `["0:A:ubx:0", "0:B:ubx:0"]`，tx `["0:A:ubx:0"]`，既定の fake．
- calls に `set_subdev_spec rx A:0 B:0` と `set_subdev_spec tx A:0`；`device.channels(Dir::Tx) == 1`（既定は 2 なので，設定が効いたことが状態でも見える）．

**T5 `ur_05_a_layout_out_of_uhd_s_default_order_is_accepted`（integration，`tests/fake.rs`）**

- 今のモード，rx `["0:B:ubx:0", "0:A:ubx:0"]`，tx `["0:A:ubx:0"]` → `Ok`，calls に `set_subdev_spec rx B:0 A:0`．
- 続けて rx `["0:B:ubx:0"]` → `Ok`，`set_subdev_spec rx B:0`，`channels(Rx) == 1`．
- parser 側は `crates/ezsdr-radio/tests/device.rs:201-203` の 3 ケースを受理に回し，`rx[0].slot == Slot::B` を assert する（`:190-193` の形）．MockRadio 側は `mock_radio.rs:341-342,348-363` を同じ判断に合わせる（重複の扱いは T7）．

**T6 `ur_05_the_spec_is_set_before_the_channel_map_is_read`（integration，`tests/fake.rs`）**

- `tests/fake.rs` に小さな wrapper `Recording` を足す（`Watched`・`Gated` と同じ形：`fake.rs:3196-3234`，`:3708-3788`）．`FakeDevice` を持ち，`channels`・`front_end`・`set_subdev_spec`・`serial`・`describe` を `Mutex<Vec<String>>` に記録して転送する．
- `FakeConfig { front_ends: vec!["BasicRX"], .. }`，layout モード rx `["0:A:basic_rx:1"]`，tx `[]`．
- assert：各方向について，最初の `set_subdev_spec <dir> …` の位置が，その方向の最初の `channels <dir>` と，すべての `front_end <dir> …` より前．`describe` は両方向の spec より後（`serial` の位置は問わない）．
- 観測：wrapper の log（`calls()` は `channels`/`front_end` を記録しない：`device.rs:814-822`）．

**T6b `ur_05_a_layout_naming_front_end_one_is_set_before_the_front_ends_are_checked`（integration，`tests/fake.rs`）**

- device `["BasicRX"]`，layout モード rx `["0:A:basic_rx:1"]`，tx `[]`．
- `from_binding` が `Ok`，`set_subdev_spec rx A:1`，`channels(Rx) == 1`，`front_end(Rx,0) == "BasicRX (1)"`．
- spec の前に `front_end` を読む実装は，順序 3 の番号までの照合があると「BasicRX (0)」を見て拒否する（結果で順序を捕まえる版）．今のモードの parser は front end 1 を拒否する（`x3xx.rs:608-612`）ので，このテストは layout モードでしか作れない．

**T7（要判断）`ur_05_a_repeated_front_end_is_still_refused`**

- 今のモード rx `["0:A:ubx:0", "0:A:ubx:0"]` と `["0:A:ubx:0", "0:B:ubx:0", "0:A:ubx:0"]`．外す ORDER の loop は 1a で「重複と順序の検査を置き換えた」もの（`plan/phase8/reviews/1a-opus.md:17`）なので，置き換えを入れないと MockRadio の今のモード（`crates/ezsdr-mock-radio/src/lib.rs:442-445`）は 1 つの front end に 2 channel を持つ layout を受ける．
- 推奨：`x3xx::from_selector` の ORDER の loop を「同じ (slot, frontend) を 2 度名指さない」検査に置き換え，`channels::check_layout` の同じ判断（`channels.rs:300-309`）と 1 か所に寄せる．そのとき `mr_02_from_binding_refusals` の needle は「position 1 names slot A」から重複の文に変わる．
- 採らない場合（装置に任せる）は，UhdRadio は fake の拒否（`device.rs:630-635`，H7 では実機は streamer まで拒否しない）で `Rejected`，MockRadio は受理という食い違いを記録する．

### (c) 壊れうる形と mutation の行の案

| # | 誤った実装 | (b) のどの assert が落ちるか |
|---|---|---|
| C1 | `set_subdev_spec` を呼ばない | T1・T4 の calls（前置きの spec が無い） |
| C2 | front end の照合の後に設定する | T6（log の位置），T6b（結果が `Rejected` になる） |
| C3 | markup の欄の取り違え（`frontend:slot`，timekeeper を slot に，`0:A`） | T1・T2（fake の `parse_spec` が拒否：`device.rs:601-644`） |
| C4 | 区切りが ASCII 空白1つでない（`""`，`", "`，タブ） | T1・T2（lookup error），T3（空の spec の拒否） |
| C5 | slot の綴り（小文字・`slot` の語） | T1・T2（lookup error） |
| C6 | 要素を slot 順・逆順に並べ替える | T2（`B:0 A:0` と channel の並び） |
| C7 | 両方向に同じ markup を渡す | T1・T4 の calls |
| C8 | 要素の無い方向でも呼ぶ（空文字） | T3（`Ok` が `Rejected` になる） |
| C9 | 要素のある方向を飛ばす | T1・T4 の calls（prefix の layout では照合は既定の spec でも通るので，calls だけが殺す） |
| C10 | 要素を layout でなく装置の channel 数から作る | T4（tx が既定 2 で `A:0 B:0` になる） |
| C11 | 今のモードだけ設定しない | T4 |
| C12 | prefix の制限を残す | T5（`Ok` が `Rejected` になる） |
| C13 | 重複を受ける（T7 を採る場合） | T7 |

行の案（`old` の正確な文字列はコードと一緒に決める）：

- R1：`device.set_subdev_spec(dir, &markup)` の行を no-op（`let _ = (dir, &markup);`）に．killer：T1．
- R2：join の `" "` を `","` に．killer：T1（fake が lookup error）．
- R3：`format!("{}:{frontend}", …)` を `format!("{}:0", …)` に．killer：T1（`B:1` が `B:0` になり重複で拒否）．
- R4：`is_empty` の `continue` を落とす．killer：T3（空の spec を fake が拒否）．
- R5：`elements.iter()` を `elements.iter().rev()` に．killer：T2．
- R6（T7 を採る場合）：重複の検査を `if false` に．killer：T7（両 Provider の parser テスト）．

既存の行：

- `plan/maintenance/tools/mutations.json:3962-3970` の P08 と `:3972-3980` の P09 は，`old` の ORDER の loop ごと消えるので削除（R6 を採るなら新しい検査に当て直す）．killer の `mr_02_from_binding_refusals` は残るケースのために残す．
- `plan/maintenance/tools/mutations.json:4014-4018`（front end の名前の照合）は順序 3 の検査に移るので，そこへ当て直す．
- 追加の行は `plan/maintenance/tools/mutations.json` に R1〜R5（と R6）．`old` は実装後に exact text を取る．prefix の受理そのもの（T5）はコードを消す規則なので行を持たない（削除は mutation で表せない）．

### (d) 頼る振る舞い

- **H1**（`multi_usrp_rfnoc.cpp:2822-2834`）— `set_subdev_spec` はその方向の channel map を作り直し，その方向の streamer を切る（他方の方向は残る）．channel `i` は spec の `i` 番目の組．**VERIFIED**．T1・T2・T4 の並びと，`from_binding` の時点で設定してよい理由（まだ streamer が無い）．
- **H2**（`subdev_spec.cpp:19-20,58-59`）— markup は `<db>:<fe>` を ASCII の空白で区切る；3 分割以上は `uhd_subdev_spec_make` の失敗．**VERIFIED**．T1・T2・T4，C3・C4，タブのケース（`fake.rs:2107-2111`）．
- **H7**（`multi_usrp_rfnoc.cpp:2796`）— RFNoC の `set_*_subdev_spec` は空・重複を検査しない．FakeDevice は拒否する（厳しい側，UR-33）．**VERIFIED**．T3（空で呼べば fake が拒否）と T7（重複は実機では streamer まで拒否されない）．
- **H5**（`db_basic_and_lf.cpp:101-111`）— Basic/LF の front end の名前は番号を持つ（`BasicRX (0)`）．**VERIFIED**（1b-4a）．T6b の「spec の前後で名前が変わる」観測（番号までの照合は順序 3）．
- fake 側の契約（UR-33，UHD の振る舞いではない）：`channels`/`front_end` は現在の spec（`device.rs:814-822`），spec の順が channel の順（`:853-860`），spec の変更は `opened` を空にする（`:846`）．

### 関わる他の順序

- **1（`keys` の値）**：T1・T2・T3・T6・T6b は `keys: "channel"` の受理と `channels::from_selector` の選択を前提にする．T4・T5 は今のモードの側．
- **3（front end の照合）**：spec の後に `channels`/`front_end` を読む検査は順序 3 の本体．T6b の結果による検出と，`ur_05_the_profile_must_be_the_device_s_front_ends` の 2 ケース（`fake.rs:169,171`）の文言の移動は順序 3 が持つ．
- **4（rate の集合）**：`from_binding` は `rates` を呼ばない．fake の spec の変更は `opened` を空にする（`device.rs:846`）ので，`rates_before_open` は streamer を開くまで `{1.0}` のまま（`fake.rs:2339-2358`）．`arm` の比較は `rx_open`/`tx_open` の後（`provider/mod.rs:427,433`）．
- **5（serial）**：同じ `from_binding` が `serial()` を読む（順序 5）．T6 の log は `serial` を許容し，spec との順序は固定しない．
- **6（stream 0 の設定）**：spec により装置の channel 番号が論理 channel の番号になることが，`rx_open(0, [1])` と `apply rx 1 …`（順序 6）の前提．
- **14（Manifest）**：`describe()` が spec の後に走る（`mod.rs:193`）ので，1 board の fake（`one_obx()`）では `rx_channels` が 2 から 1 に変わる．既存テストでこれを読むものは無い（`grep -rn "rx_channels\|tx_channels" crates/ezsdr-radio-uhd/tests crates/ezsdr-radio-uhd/src` → `uhd.rs:641-642` と `device.rs:794-795` の実装だけ）．markup 自体の記録は順序 14．
- **15（VH-8）**：MockRadio と UhdRadio に同じ layout を与えた比較は，この順序で prefix でない layout と今のモードの spec が揃って初めて成り立つ．MockRadio の今のモードの parser も同じ `x3xx::from_selector` なので，T5 と `mr_02_from_binding_refusals` の書き換えが両者を揃える．

## 3. front end の照合が spec の後に行われ，外れを拒否する

対象は「spec を設定した後に，方向ごとの各 logical channel の `Device::front_end` を layout の要素（Basic/LF は front end の番号まで）と比べ，違えば `Rejected` にする」規則．markup の組み立て自体は順序 2，rate の集合は順序 4 の担当で，ここでは**照合の位置・比較对象・失敗の伝え方**を固定する．今の照合は `crates/ezsdr-radio-uhd/src/provider/mod.rs:173-192` にあり，`x3xx::from_selector` の `Selection`（`crates/ezsdr-radio/src/x3xx.rs:401-433`）の `board.uhd_name()` との prefix 比較と，channel 数の比較をしている．新しいモードでは `channels::Element`（`crates/ezsdr-radio/src/channels.rs:36-46`，`front_end: FrontEnd` と `frontend: u32`）を同じ規則で見る．

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:162-181` **`ur_05_the_profile_must_be_the_device_s_front_ends`** — 今の照合にいちばん近いテスト．old mode の拒否の理由を丸ごと固定している（`fake.rs:167` の CBX-120 対 UBX，`:168` の slot B の CBX，`:170`・`:176`・`:177`・`:178` の board 種別，`:180` の混在 layout）．`:164`・`:174`・`:175` の `Ok` は，UBX/OBX/CBX の名前に番号を要求しないことも固定している．**`:169` と `:171` の「needs <k> channels」は順序が変わると結果が変わる**：spec を先に設定すると，1 slot の装置への `A:0 B:0`（`:169`）と slot の無い装置への `A:0`（`:171`）は `set_subdev_spec` の lookup 失敗が先に返る．この 2 assert は UHD の失敗文面を期待する形に書き換える（spec 18 §6 の表 `design/18-uhd-radio.md:339` も追随）．それ以外の assert は HEAD と同じ結果になる（各 board の front end 0 は spec が成功するため，今と同じ名前の不一致になる）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:96-131` **`ur_05_from_binding_refusals`** — UR-5 の拒否が `UR-5:` で始まることと，selector 側の拒否の形．新しい拒否もこの形に合わせる（このテストは `Rejected` kind を読まない，`design/18-uhd-radio.md:338`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2032-2071` **`ur_33_the_front_end_table_follows_the_subdev_spec`** — fake が spec 後に返す名前（`BasicRX (0)`／`(1)`，`UBX RX`，`LFTX (0)`）と，spec の順序が channel の順序になること（`:2042-2057`）．H5 の fake 側の証拠で，(b) の期待名の根拠．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2073-2111` **`ur_33_a_subdev_spec_uhd_would_refuse_is_refused`** — `set_subdev_spec` の失敗文面（`LookupError: Could not find radio on mboard 0 that matches subdev A:1`，`uhd_subdev_spec_make: …`）．(b4) が Provider にそのまま通すことを要求する文面．
- `crates/ezsdr-radio-uhd/tests/hardware.rs:32-45` **`hw_b1_probe`** — 実機で layout の channel と `front_end` の名前を比べるベンチ側の同じ規則（UR-34，ここでは走らない．front end 名は BP0 で確かめる，`plan/phase8/reviews/1b4a-report.md:137`）．
- 今の mode の prefix 拒否を固定しているテスト（順序 2 の変更で偽になるが，(b) の slot B だけの layout がこれに依存する）：`crates/ezsdr-mock-radio/tests/mock_radio.rs:341-361`．`x3xx.rs:570-580` を外すと，Mock の current mode の「position 0 names slot B」の 3 assert（`:353`・`:356`・`:361`）と，「position 2 names slot A」の前提が変わる．Mock の current mode の扱いは順序 2 が決めるが，この item のテストは slot B だけの layout を使うので，ここに依存することを記録する．
- この item が書き換えるコードを固定している mutation の行：`plan/maintenance/tools/mutations.json:4012` **P13**（`old` は `if !front_end.starts_with(element.board.uhd_name())`，`new` は `if false`）．比較の行が変わるので `old` を新しい比較に当て直す（killer の `ur_05_the_profile_must_be_the_device_s_front_ends` はそのまま生きる：`:167` が拒否を期待するため）．`plan/maintenance/tools/mutations.json:3962` **P08**・`:3972` **P09** は prefix の loop を壊す行で，loop 自体が消えるため `old` が無くなる（順序 2 が削除する）．

### (b) 足すテスト

すべて `crates/ezsdr-radio-uhd/tests/fake.rs`（test binary `fake`）に置く．`UhdRadio::from_binding` と `FakeDevice` は公開の境界で，Run を始めないため Manifest・event・clock は観測に使わない．観測は戻り値の `ModuleError` と `FakeDevice::calls()`・`channels()`・`front_end()` だけ．layout mode の binding は `binding()`（`fake.rs:82-93`）に明示的な `rx`/`tx` と `"keys": "channel"` を渡して作る（`binding` は欠けたときだけ既定を入れる）．

1. **`ur_05_the_front_end_check_reads_the_device_after_the_subdev_spec`**
   - 装置（別々の fake）：`FakeConfig { front_ends: vec!["CBX-120", "UBX"], ..FakeConfig::default() }`．既定の受信の名前は channel 0 が `CBX-120 RX`，channel 1 が `UBX RX`（`table_of`/`front_end_name`，`crates/ezsdr-radio-uhd/src/device.rs:421-447`）．
   - 2 例：(i) 今の mode（`keys` なし）で `rx: ["0:B:ubx:0"], tx: ["0:B:ubx:0"]`；(ii) layout mode で `rx: ["0:B:ubx:0"], tx: []`．
   - 操作：`UhdRadio::from_binding(&binding, fake.clone())`．
   - assert：`Ok`；`calls()` に `set_subdev_spec rx B:0` があり，(i) では `set_subdev_spec tx B:0` もある；`fake.channels(Dir::Rx) == 1`；`fake.front_end(Dir::Rx, 0) == "UBX RX"`（(i) では tx も `1`／`UBX TX`）．
   - 観測：**受理そのものが順序の証拠**．spec の前に読むと channel 0 は slot A の `CBX-120 RX` で，`ubx` の layout は拒否される．`calls()` は spec が実際に呼ばれたことを示す．壊れる形：read-before-spec，spec の呼び出しが無い，channel 数だけで spec を省く．

2. **`ur_05_a_board_that_differs_from_the_layout_is_refused`**
   - 装置（rx/tx の 2 例で別々の fake）：`FakeConfig { front_ends: vec!["UBX", "UBX"], ..FakeConfig::default() }`（両 slot，両方向に front end 0）．
   - rx：layout mode `rx: ["0:B:basic_rx:0"], tx: []`．`set_subdev_spec rx B:0` は UBX の front end 0 として成功するが，返る名前は `UBX RX` で layout は `basic_rx`．
   - tx：`rx: [], tx: ["0:B:basic_tx:0"]`．spec は成功し，名前は `UBX TX`，layout は `basic_tx`（`basic_rx` は送信を持たないので tx には使えない，`crates/ezsdr-radio/src/x3xx.rs:253-261`）．
   - assert：`Err`；`kind == ModuleErrorKind::Rejected`；`message.starts_with("UR-5:")`；rx は `contains("rx channel 0")`，tx は `contains("tx channel 0")`；実際の名前 `UBX RX`／`UBX TX` を `contains`；期待の board は `basic_rx` か `BasicRX` のどちらかの綴りで `contains`．さらに `calls()` にその方向の `set_subdev_spec … B:0` があり，`channels(dir) == 1`（拒否が spec の後であること）．
   - 壊れる形：spec を信じて名前を比べない；slot だけ比べる；tx を回さない；理由に方向・channel 番号を入れない．

3. **`ur_05_a_basic_front_end_s_number_is_checked`**
   - 対照（faithful）：装置 `FakeConfig { front_ends: vec!["BasicRX", "BasicRX"], .. }`，layout mode `rx: ["0:B:basic_rx:1"], tx: []`．assert：`Ok`；`calls()` に `set_subdev_spec rx B:1`；`front_end(Dir::Rx, 0) == "BasicRX (1)"`（layout の番号 1 が markup と名前の両方に効いている）．
   - 故障：同じ layout を，`front_end` だけを差し替えたテスト内の `Device` wrapper（例：`Mislabelled`．全メソッドを inner に転送し，`front_end` は返った名前の末尾の `(<n>)` を `(0)` にした `BasicRX (0)` を返す）に bind する．assert：`Err`；`Rejected`；`contains("rx channel 0")`；装置が返した名前 `BasicRX (0)` を `contains`；期待の board `basic_rx`／`BasicRX` を `contains`．
   - wrapper が要る理由をテストの doc に書く：`FakeDevice::front_end` は**現在の spec から名前を作る**（`crates/ezsdr-radio-uhd/src/device.rs:592-595`，`:818-822`）ので，faithful な fake では spec が成功した後に layout の番号と名前が食い違わない．この比較は Provider の規則（UR-5）で，装置の答えが食い違ったときに拒否することを固定する．UHD の実名は `(0)`／`(1)`（H5，`plan/phase8/research/basic-lf-uhd-facts.md:43-44`）なので，wrapper は故障注入であって装置の模型ではない．
   - 壊れる形：Basic/LF を board 名の prefix だけで比べる（番号を落とす）；Basic/LF に名前を定義せず常に拒否する（対照の `Ok` が落ちる）；3 board にも `(0)` を付ける（既存 `fake.rs:164`・`:174`・`:175` の `Ok` が落ちる）．

4. **`ur_05_a_subdev_spec_the_device_refuses_is_refused_with_uhd_s_text`**
   - 装置（rx/tx で別々）：`FakeConfig { front_ends: vec!["UBX"], .. }`（slot A だけ）．
   - rx：layout mode `rx: ["0:B:ubx:0"], tx: []`．tx：`rx: [], tx: ["0:B:ubx:0"]`．
   - assert：`Err`；`Rejected`；`message.starts_with("UR-5:")`；そして `message.contains("uhd_usrp_set_rx_subdev_spec: UHD error 41: LookupError: Could not find radio on mboard 0 that matches subdev B:0")`（tx は `set_tx_subdev_spec`）．これは `DeviceError.message`（`device.rs:626-628`）をそのまま含めるという意味（C API の文面の形は `design/18-uhd-radio.md:112`）．
   - 壊れる形：spec の失敗を握りつぶす——その場合，装置は既定のまま channel 0 が slot A の `UBX RX`／`UBX TX` で `ubx` に一致し，**受理されてしまう**；文面を要約に置き換える；方向を取り違える．

5. **`ur_05_each_channel_is_checked_at_its_own_device_channel`**
   - 装置：`FakeConfig { front_ends: vec!["BasicRX"], .. }`（1 枚で受信 front end 0 と 1）．layout mode `rx: ["0:A:basic_rx:1", "0:A:basic_rx:0"], tx: []`．
   - 正しい実装：spec は `A:1 A:0` で channel 0 が `BasicRX (1)`，channel 1 が `BasicRX (0)`（fake の順序は `fake.rs:2054-2057` が固定）．assert：`Ok`；`calls()` に `set_subdev_spec rx A:1 A:0`；`front_end(Dir::Rx, 0) == "BasicRX (1)"`，`(Dir::Rx, 1) == "BasicRX (0)"`．
   - 壊れる形：全要素を channel 0 で比べる（要素 1 は `(0)` を期待するが channel 0 は `(1)` で拒否）；既定の表の順序で期待名を作る；channel 番号を slot の番号で決める（両方 slot A）；channel 数が同じなので spec を省く（既定は `(0)`,`(1)` で逆になり拒否）——いずれも受理の assert が落ちる．

### (c) 壊れうる形と mutation の行の案

| # | 壊す規則と形 | 落ちる assert | mutation の行の案 |
|---|---|---|---|
| 3-1 | spec を front end の照合の後に置く（あるいは呼ばない） | (b1) の `Ok`：spec 前に読むと `CBX-120 RX` で拒否 | `crates/ezsdr-radio-uhd/src/provider/mod.rs`：`set_subdev_spec` の block を `channels`/`front_end` の loop の後ろへ移す（`old` は code と一緒に決める）．killer `ur_05_the_front_end_check_reads_the_device_after_the_subdev_spec` |
| 3-2 | `set_subdev_spec` の失敗を握りつぶす（`let _ =`／`if let Ok`） | (b4)：spec が効かず既定の `UBX RX` が `ubx` に一致して受理される | `provider/mod.rs`：`map_err(...)?` を `let _ = ...;` に．killer `ur_05_a_subdev_spec_the_device_refuses_is_refused_with_uhd_s_text` |
| 3-3 | 失敗の文面を UHD のテキストでなく要約にする | (b4) の `contains(<UHD text>)` | `provider/mod.rs`：`e.message` の代わりに固定の文．killer 同上 |
| 3-4 | layout の board と装置の front end を比べない（spec を信じる） | (b2) の拒否；(b4) の拒否 | `provider/mod.rs`：比較を `if false` に（**P13 の `old` を新しい比較の行に当て直す**，killer は既存 `ur_05_the_profile_must_be_the_device_s_front_ends` のまま） |
| 3-5 | Basic/LF の比較から番号を落とす（base 名の prefix だけ） | (b3) の故障側：`BasicRX (0)` 対 `basic_rx:1` を受理してしまう | `provider/mod.rs` か `ezsdr-radio/src/x3xx.rs` の期待名の組み立て：`format!("{} ({})", …)` を base だけに．killer `ur_05_a_basic_front_end_s_number_is_checked` |
| 3-6 | 全要素を channel 0 で比べる／既定表の順序で期待名を作る | (b5) の `Ok`（要素 1 の `(0)` 対 channel 0 の `(1)`） | `provider/mod.rs`：`front_end(dir, chan)` の `chan` を `0` に，または期待要素を `elements[0]` 固定に．killer `ur_05_each_channel_is_checked_at_its_own_device_channel` |
| 3-7 | tx 方向の照合を落とす | (b2)・(b4) の tx 例 | `provider/mod.rs`：`[(Dir::Rx, "rx", …), (Dir::Tx, "tx", …)]` から tx の要素を削る．killer (b2)/(b4) の tx |
| 3-8 | 理由から方向・channel 番号を落とす | (b2)・(b3) の `contains("… channel 0")` | `provider/mod.rs`：理由の `{name} channel {chan}` を削る．killer (b2) |
| 3-9 | 3 board にも `(0)` を要求する（Basic/LF 用の期待名を全 board に使う） | 既存 `fake.rs:164`・`:174`・`:175` の `Ok` | 期待名の分岐を `FrontEnd` 全体に広げる．killer 既存 `ur_05_the_profile_must_be_the_device_s_front_ends` |

注意：3-1 の「呼ばない」変異は (b1) と (b5) の `calls()` でも落ちる．3-4 は既存 P13 の当て直しで足りる（新しい (b2) も殺すが，行は 1 本に保つ）．行の `old` の正確な文字列はコードと一緒に決める．`test_bin: "fake"`，`crate: "ezsdr-radio-uhd"`．P08/P09（prefix loop）の削除は順序 2 の担当．

### (d) 頼る振る舞い

- **H1**：`set_subdev_spec` はその方向の channel の写像を作り直し，その方向の開いている streamer を切る（他方向は残る）．`multi_usrp_rfnoc.cpp:2822-2834`，VERIFIED．(b1)/(b2)/(b5) は「spec の後の写像を読む」ことの根拠．fake の実装は `device.rs:836-846`．
- **H2**：markup は `<db>:<fe>` を ASCII の空白で区切る．`subdev_spec.cpp:19-20,58-59`，VERIFIED．(b1)/(b5) の `calls()` が `B:0`／`A:1 A:0` を期待する根拠（組み立て自体は順序 2）．
- **H5**：front end の名前は `UBX RX` などで，Basic/LF は `BasicRX (0)` のように番号を持つ．`db_ubx.cpp:562`，`db_basic_and_lf.cpp:101-111`，VERIFIED（1b-4a）．(b3)/(b5) の期待名の根拠．`plan/phase8/research/basic-lf-uhd-facts.md:43-44`．
- **H7**：空の spec と重複を RFNoC の `set_*_subdev_spec` は検査しない（fake は厳しい側）．`multi_usrp_rfnoc.cpp:2796`，VERIFIED．(b4) の lookup 失敗は「表に無い組」であり，この項目の対象外だが，失敗文面は fake が UHD の言葉で返す（`multi_usrp_rfnoc.cpp:1226-1254`，`device.rs:623-628`）．
- **H8（追加）**：`get_{rx,tx}_subdev_name(chan)` は，現在の spec がその channel に写した front end の `name` を読む（`get_rx_subdev_name` = radio の `get_fe_name`）．`plan/phase8/research/basic-lf-uhd-facts.md:44`，VERIFIED．したがって spec の後で読む限り，名前は spec の写像と一致する（(b1)/(b5) の前提）．
- **H9（追加）**：C API は投げられた例外のテキストを `uhd_get_last_error` に残し，`<call>: UHD error <code>: <text>` の形で返る．`uhd/error.h:89-104`，`design/18-uhd-radio.md:112`，VERIFIED（C の wrapper）．(b4) が `DeviceError.message` をそのまま通すことの根拠．実機の具体的な文面は BP0 で確かめる（INFERRED）．
- 頼らないもの：rate の集合（順序 4），周波数・利得の範囲（この段階に入れない），slot の同一性（spec が作るので名前では比べない）．

### 関わる他の順序

- **1（`keys` の値）**：この item のテストはすべて `keys: "channel"` を必要とする．`UhdRadio::from_binding` の allowed keys（`crates/ezsdr-radio-uhd/src/provider/mod.rs:124-130`）に `keys` が入り，layout mode が `channels::from_selector`／`channels::Description` を使うことが前提．mode の判定を誤ると全テストが selector で落ちる．
- **2（subdev spec）**：(b1)・(b5) が期待する `calls()` の markup はこの順序の成果物．slot B だけの layout は `x3xx.rs:570-580` の prefix 制限の撤去に依存する．撤去は共有の `x3xx::from_selector` を変えるので，Mock の current mode の `mock_radio.rs:341-361` と P08/P09 も動く（Mock 側の扱いは順序 2 が決める）．また `from_binding` の `channels` 読みが spec の後になると，`fake.rs:169`・`:171` の channel 数の理由は UHD の lookup 文面に変わる（(a)）．これは brief の「変えないこと」の 2 例外に含まれない**3 つ目の差**なので，orchestrator は順序 2 の差として記録する必要がある．
- **4（rate の集合）**：同じ「spec の後に照合する」原則だが，対象は `arm` で最初の streamer を開いた後（UR-13，`design/18-uhd-radio.md:196`）．front end の照合を `arm` へ移してはいけない（`from_binding` の拒否であること）．
- **5（同じ serial）**：この item の拒否は `serial()` を登録した後に起きる（brief）．拒否した `from_binding` が登録を残すと，catalogue の開き直しの loop（`crates/ezsdr-server/src/catalogue.rs:122-147`）が 1 つ目の radio を drop した後の `from_binding` を自分で拒否する．順序 5 の「照合の失敗で登録は残らない」テストが一般則を押さえるが，(b2)・(b4) の後に同じ fake へ正しい layout で bind して `Ok` を見る 1 行を足してもよい（テストの意図は front end の照合のまま）．
- **6・7（stream 0 の設定，channel ごとの timed）**：logical channel と device の channel 番号が同じになるのは spec の写像による．(b5) はその順序を名前に写して固定する．
- **14（Manifest）**：`device` section が spec の markup と serial を残す（順序 14）．この item の拒否は Run の前なので Manifest は無い．
- **15（VH-8）**：front end の照合は装置の境界を持つ UHD Module だけの規則で，Mock には対応する装置が無い．VH-8 が比べられるのは selector 側の拒否（順序 1・2）までで，この item の拒否は VH-8 の対象外．

## 4. rate の集合の照合

規則（brief，順序 4）：`from_binding` は `rates` を呼ばない（H3）．`arm` は，方向ごとに最初の streamer を開いた後に，その stream の 1 channel の `rates` を description の格子（`DeviceDescription::rates`）と集合として比べ，違えば `Rejected`（UR-13）．開いた後も `{1.0}` なら拒否．streamer を開かなかった方向は比べず，比べたかどうかを Manifest に残す．実装の継ぎ目は `crates/ezsdr-radio-uhd/src/provider/mod.rs:407-443`（rx の open は `:423-428`，tx は `:429-440`）；`from_binding`（`:111-227`）ではない．description 側の格子は `crates/ezsdr-radio/src/device.rs:169` の `rates: Grid`，x3xx では `crates/ezsdr-radio/src/x3xx.rs:638` の `Grid::Values(rates(selection.master_clock_hz))`（639 値）．

### (a) 既存のテスト

- **arm の拒否の形**：`crates/ezsdr-radio-uhd/tests/fake.rs:810` `ur_13_arm_cases` — 外部参照が未ロックのとき `Failed { stage: Stage::Arm }` と `UR-13: ...` の理由．`Stage::Arm` での失敗を assert するのはこれと `crates/ezsdr-radio-uhd/tests/fake.rs:873` `ur_15_every_clock_starts_on_its_lattice` の前半（`:863`）だけ（`grep -n "Stage::Arm" crates/ezsdr-radio-uhd/tests/fake.rs` → `:814`，`:863`）．新しい拒否テストはこの形をそのまま使う．
- **fake の `rates` の値（唯一の直接テスト）**：`crates/ezsdr-radio-uhd/tests/fake.rs:2339` `ur_33_rates_report_the_grid_and_the_pre_chain_one` — 639 個の昇順 `(r, r, 0.0)`（`:2345-2350`），`rates_before_open` の pre-chain `[(1.0, 1.0, 0.0)]` と `rx_open` による接続（`:2352-2355`），spec 変更で未接続に戻る（`:2356-2358`）．`rates_before_open` を使うテストはこれを除いて無い（`grep -rn "rates_before_open" crates/ezsdr-radio-uhd/ | grep -v src/device.rs` → `tests/fake.rs:2341,2352` のみ）．
- **格子が両側で同一であること（既存が緑のままである根拠）**：`crates/ezsdr-radio-uhd/tests/fake.rs:761` `ur_12_every_advertised_rate_is_accepted` — description の `rates` が `ezsdr_radio::x3xx::rates(200_000_000)` そのものであること（`:769-775`），fake が同じ関数の格子を返すこと（`crates/ezsdr-radio-uhd/src/device.rs:876`）．既定の fake では比較が一致する．
- **両方向の arm が通る run**：`crates/ezsdr-radio-uhd/tests/fake.rs:873` `ur_15_every_clock_starts_on_its_lattice` — rx と tx の両方の streamer を開いて Running になる Spec Run（`:878-881`）．比較を足しても今のモードの結果が変わらないことを固定する最近傍．ほかに `:793-807`（capture と cold の rate 変更）．
- **link の無い Session の形**：`crates/ezsdr-radio-uhd/tests/fake.rs:1941` `ur_29_a_lost_device_is_found_while_idle`（`:1944-1948` で `rec` と `placements` を消す）— rx は既定 1 channel を持つが link が無いので arm は streamer を開かない（`mod.rs:424` の `core.links.is_empty()`）．T4 の土台．
- **「`rates` を呼ばない」ことの固定は無い**：HEAD で `Device::rates` を呼ぶコードは Provider のどこにも無い（定義は `crates/ezsdr-radio-uhd/src/device.rs:864` と `crates/ezsdr-radio-uhd/src/uhd.rs:738`，転送は `crates/ezsdr-radio-uhd/src/provider/test_support.rs:116`・`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:559` の `fn rates` だけ；検索：`grep -rn "\.rates(" crates/ezsdr-radio-uhd/src | grep -v "fn rates"` → 出力なし）．したがって「`from_binding` で呼ばない」「開いた後に呼ぶ」は (b) の T1 が固定する．

### (b) 足すテスト

置き場：5 本とも `crates/ezsdr-radio-uhd/tests/fake.rs`，`ur_13_arm_cases`（`:810`）の隣．新しい wrapper も同じファイルに置く（`Watched`（`:3187`）・`Descheduled`（`:3406`）・`Gated`（`:3708`）と同じ形）．

**足す道具 `RatesMisreported`**（テスト wrapper）：`FakeDevice` を内側に持ち，`rates` だけを方向ごと（`Option<Vec<(f64, f64, f64)>>` の rx / tx）に上書きする．`rates` はまず `self.inner.rates(dir, chan)?` を呼んで（call の記録 `crates/ezsdr-radio-uhd/src/device.rs:867` と channel の検査 `:868-870` を残す），上書きがあればそれを返す．`Device` の他は全部転送し，`host()` も転送する（`Gated` `:3758-3789` と同じ列挙）．内側は `ClockMode::Manual` で組む（`assembly` が `watch()` を armed にする `crates/ezsdr-radio-uhd/tests/common/mod.rs:243-249`）．格子が違う装置は UR-33 の「fake が忠実な項目」（`design/18-uhd-radio.md:276-285`）に無いので `FakeConfig`/`FakeFault` を増やさない（deschedule と同じ扱い，plan 30 §7）．退けた案：`FakeConfig` の knob（UR-33 の規範文と §6 の表が増える割に実機の振る舞いではない）．

**T1 `ur_13_arm_compares_the_rate_set_after_the_streamer_opens`**（integration）
- 操作の列：`with_tx(receive_spec(1, 1e6, 1e9, None), 1e6)` と `profile(&dir, json!({}), json!({}), false)`（`crates/ezsdr-radio-uhd/tests/common/mod.rs:181-207`，`:146-179`）で，`FakeConfig { rates_before_open: true, ..FakeConfig::default() }` の fake を `spec_run`．`running(&run)` の後に `run.finish()`．
- assert：(i) Running に達する（pre-chain を開く前ではなく後に読む）．(ii) `device.calls()` で `rx_open 0 [0]` の位置 < 最初の `rates rx 0` の位置，`tx_open 0 [0]` < `rates tx 0`；`rates rx 0`・`rates tx 0` は各 1 回（`calls(&device, "rates")`，`tests/common/mod.rs:452-454`）．(iii) 最初の `rx_open`/`tx_open` より前に `rates` の entry が 1 つも無い（`from_binding` が呼ばない）．(iv) `section(&manifest, "device")`（`tests/common/mod.rs:376-378`）で rx・tx が「比べた」と記録されている（形は順序 14；案：`"rates": { "rx": { "checked": true }, "tx": { "checked": true } }`）．
- 観測：`device.calls()`（`crates/ezsdr-radio-uhd/src/device.rs:647-649`）と Manifest `device` section．`opened` は `rx_open` が `:1071`，`tx_open` が `:1370` で埋め，`rates` は `:871-875` でそれを見る．

**T2 `ur_13_arm_refuses_a_rate_set_the_description_does_not_have`**（integration，2 ケース）
- 前準備：`let grid = ezsdr_radio::x3xx::rates(200_000_000); let mut off = grid.clone(); let middle = off.len() / 2; off[middle] += 0.5;`（`crates/ezsdr-radio/src/x3xx.rs:58-60`）．`+0.5` は他の格子値と衝突しない（どの 2 値の差も `200・10⁶/(2040・2039)` ≈ 48 Hz 以上）．`off` を `(r, r, 0.0)` の列にして `RatesMisreported` の rx 上書き（ケース 1）／tx 上書き（ケース 2）にする．ケース 2 は `with_tx` の run．
- assert：(i) Run が `Failed { stage: Stage::Arm }`（`ur_13_arm_cases` `:814` の形）．(ii) `failure(&manifest)`（`crates/ezsdr-radio-uhd/tests/common/mod.rs:407-412`）が `UR-13` と方向（`rx`／`tx`）と rate の語を含む．(iii) `device.calls()` で `rx_open 0 [0]` が `close_streams` より前（arm の失敗後，`cleanup` が閉じる；`mod.rs:544-561`）．(iv) `rates` を 1 回読んでいる（ケース 1 は `rates rx 0`）．
- 「中央 1 値の差し替え」にする理由：端（最小・最大）と個数を変えないので，端だけ・個数だけを見る実装が受けてしまう（(c) W4a/W4b）．集合は両方向に差が出るので，片側包除（`⊆`）の誤りも拒否側へ落ちる（正しい）．
- 観測：`run.state()`，`failure(&manifest)`，`device.calls()`．

**T3 `ur_13_arm_refuses_a_pre_chain_rate_set_after_the_open`**（integration）
- 操作：`RatesMisreported` の rx に常に `vec![(1.0, 1.0, 0.0)]` を返させ，`receive_spec(1, 1e6, 1e9, None)` の run．
- assert：`Failed { stage: Stage::Arm }`，`UR-13`；`rx_open 0 [0]` は済んでいる（開いた後に読んだ証拠）．
- 狙い：開いた後も `{1.0}` のとき「未接続だから」と除外して受けないこと（INFERRED：起きないはず．BP0）．
- 観測：`run.state()`，`failure(&manifest)`，`device.calls()`．

**T4 `ur_13_a_direction_the_arm_never_opened_is_not_compared`**（integration）
- 操作：`ur_29_a_lost_device_is_found_while_idle`（`:1941-1948`）と同じ linkless Session（`rec` と `placements` を消す）を，常に `{1.0}` を返す `RatesMisreported` の rx 上書きで `session` し，`past_t0`（`tests/common/mod.rs:331-334`）の後に `finish()`．
- assert：(i) `Stage::Arm` で失敗しない（Running まで行く）．(ii) `device.calls()` に `rates` が 1 つも無い．(iii) `section(&manifest, "device")` で rx（と 0 channel の tx）が「比べていない」と記録されている（案：`"checked": false`）．
- この形でなければならない理由：link が無い run では arm が `rx_open` を呼ばない（`mod.rs:423-428`）．比較を「channel 数 > 0 の方向すべて」に広げる誤りは，この run で pre-chain の `{1.0}` を読み，誤って拒否する．既定の fake（`rates_before_open: false`）では差が出ないので，常に `{1.0}` を返す wrapper が要る．
- 観測：`run.state()`，`device.calls()`，Manifest `device` section．

**T5 `ur_13_arm_takes_the_rate_set_in_any_order`**（integration）
- 操作：`RatesMisreported` の rx に，`ezsdr_radio::x3xx::rates(200_000_000)` の**逆順**の `(r, r, 0.0)` を返させる（集合は同じ）．`receive_spec(1, 1e6, 1e9, None)` の run．
- assert：Running に達する（比較が集合でなく列で行われると落ちる）．
- 根拠：設計 33 D5:131 は「rate の**集合**（範囲の端だけでなく）」と書く．UHD の `meta_range_at` の順は実機で決まる（BP0）ので順序を仮定しない．
- 観測：`run.state()`．

（実装が比較を純関数に切り出すなら，その関数の unit test — 範囲 `(start, stop, step)` の展開と集合化 — を lib に 1 本足す．置き場は実装時に決まる．）

### (c) 壊れうる形と mutation の行の案

新規コードは `arm`（`mod.rs:407-443`）に「開いた直後に 1 channel の `rates` を読み，`description.rates` と集合で比べ，違えば `rejected`」＋「比べたかどうかを `device` section に記録」を足す．誤った形と殺すテスト：

| 形 | 内容 | 殺すテスト | 行の案（新規，id は P281 以降；`old` は実装後に exact で取る） |
|---|---|---|---|
| W1 | 比較しない（読んでも判定しない） | T2（rx・tx の両ケース） | A：`old`=比較の `if`，`new`=`if false` |
| W2 | 開く前・`from_binding`/`prepare` で比べる | T1（`rates_before_open` の `{1.0}`）＋ T1 (iii) | B：`old`=open と比較の並び，`new`=入れ替え |
| W3 | streamer を開かない方向も比べる（link のガードの外へ） | T4 | C：`old`=ガードされた塊，`new`=ガードの外（`continue` の前に比較） |
| W4a | 端（先頭・末尾）と個数だけで比べる | T2（中央 1 値の差し替え） | D：`old`=集合の比較，`new`=端の比較 |
| W4b | 集合でなく列で比べる（順序を仮定） | T5（逆順） | E1：`old`=集合化，`new`=`rates == expected` |
| W5 | 一致したとき拒否する（条件の反転） | T1（一致で Running） | F：`old`=条件，`new`=否定を外す |
| W6 | 違う方向を読む（`Dir` の取り違え） | T2 ケース 1（rx だけ違い，tx の読みは一致 → 受ける） | G：`old`=rx 側の `read`，`new`=`Dir::Tx` |
| W7 | 開いた後も `{1.0}` を「未接続」として除外 | T3 | H：`old`=判定，`new`=pre-chain の値だけ等しいとみなす |
| W8 | 「比べたか」の記録が嘘（常に true） | T4 (iii) | I：`old`=記録，`new`=`true` |

- 行 A〜C・D・H・I は新規に足す（E1・F・G は A・D と同じ規則に重なるので実装を見て取捨；取るなら各 1 行）．`remote_mutate.py` の流し方は従来どおり（§7）．
- **当て直し**：全 663 行の `old`/`new` を走査したが，`arm` の中の文を `old` に持つ行は 0（`grep -n "rx_open\|tx_open\|startup_latency_ns\|start_up.store\|register_sample_clock" plan/maintenance/tools/mutations.json` → J09 の `new`（`provider/rx.rs`）だけ；`old` は無し）．この項目だけによる当て直しは見込み無し（段階全体の一覧は計画の最後の項目が扱う）．
- 追加の `rates` 呼び出しは `device.calls()` の列に現れる．完全一致で calls を見る既存テストは無い（`grep -n "calls()" crates/ezsdr-radio-uhd/tests/fake.rs` の一致は prefix の filter・count・`.position()` だけ；例：`:1758`，`:3866-3867`）ので，既存の結果は変わらない見込み（gate の実行で確認）．`set_subdev_spec` の増加（順序 2）と同じ種類の確認．
- この段階では観測しない形（記録）：(i) 範囲 `(start, stop, step)` の展開の誤り — fake は singleton しか返さない（`device.rs:876`）ので BP0 待ち（設計 33 D5:131 の「集合」はこれを含む）．(ii) 「最初の streamer」以外で比べる／比べない — 1b-4d の複数 stream．(iii) 読みの channel が stream の先頭か（今は channel 0 なので値では見えず，順序 6 の `c1.stream=0` の run で `rates rx 1` を call で見る」）— 順序 6 のテストに 1 行足す形が自然．

### (d) 頼る振る舞い

| # | 振る舞い | 根拠 | 確かさ |
|---|---|---|---|
| H3 | DDC の出力 rate の集合は chain がつながる前は `{1.0}`；chain は streamer を作るとき（`get_rx_stream`）につながる | brief の表 H3（`ddc_block_control.cpp:153-159`，`multi_usrp_rfnoc.cpp:337,1354-1361,2714-2735`）；fake 側は `device.rs:871-875` | VERIFIED（spec 変更の後に古い値が残るかは INFERRED，BP0） |
| H1 | `set_subdev_spec` はその方向の channel の写像を作り直し，開いている streamer を切る | brief の表 H1（`multi_usrp_rfnoc.cpp:2822-2834`）；fake は `device.rs:840-846`（streamer を切り，`opened` を消す） | VERIFIED |
| 追加 H8 | 送信（DUC）の rate の集合も，tx streamer を作る前は `{1.0}` | fake が両方向を同じ規則で持つ（`device.rs:871-875` と `opened[Dir::Tx]`：`device.rs:499`）．設計 33 D5:131 は `uhd_usrp_get_{rx,tx}_rates` を同じ規則で扱う | INFERRED（BP0 で測る） |
| 追加 H9 | `rates` の答えは `(start, stop, step)` の範囲の列として届く（1 rate = 1 範囲とは限らない） | `Device::rates` の署名（`device.rs:165-166`），`UhdDevice::rates` が `uhd_meta_range_at` を列にする（`crates/ezsdr-radio-uhd/src/uhd.rs:738-763`）．設計 33 D5:131「範囲の端だけでなく」 | VERIFIED（形） |

この照合は UR-5 の mcr 検査（`mod.rs:162-168`）の後を補う：同じ mcr でも DDC/DUC の decimation の集合は image で変わりうる（設計 33 D5:128，`image` はこの段階では形だけ `:132`）．したがって mcr の一致だけでは足りず，arm でしか判定できない（H3）．

### 関わる他の順序

- **2（subdev spec）／3（front end の照合）**：どちらも `from_binding` で arm より先．`rates` は spec の設定後でなければ意味がない（H1・H3；fake は spec 変更で `opened` を消す `device.rs:846`）．T1 の call 順 assert は「`set_subdev_spec` → `rx_open` → `rates`」の三段の二段目・三段目を担う（一段目と二段目は順序 2 のテスト）．front end で拒否される run は `rates` を 0 回呼ぶ（順序 3 のテストに 1 行足せる）．
- **5（serial）**：`serial()` も `from_binding` で読むので，同じ run で `set_subdev_spec`・`serial`・`rates` の順が1つの列に並ぶ．互いの前後（serial が spec の前か後か）は順序 5 が決める．T1 (iii) の「開く前に `rates` 無し」は serial の位置に影響しない．
- **6（prepare の stream 0）**：比較の読みは「開いた stream の 1 channel」．既定 fixture では channel 0 だが，順序 6 の `c0.stream=-1, c1.stream=0` の run では `rx_open 0 [1]` の直後に `rates rx 1` になる（call で観測；同上 (c) の記録）．
- **14（Manifest の `device` section）**：T1 (iv)・T4 (iii) が読む「比べた／比べていない」の record の形は順序 14 が決める（案を上に書いた）．この段階の `arm` は `instance.sections` の `ezsdr.radio.uhd.device` を書く（`mod.rs:285-297` と同じ入れ物；`cleanup` の後に `instance().sections` が Manifest へ読まれる：`crates/ezsdr-kernel/src/coordinator/ending.rs:98-121`）．markup・serial の record（順序 2・3・5）と同じ object に入るので，形は 14 と同時に確定する．
- **15（VH-8）**：MockRadio は `rates` の照合を持たない（自身の description が正）ので，VH-8 の「同じ拒否」の一覧に arm の rate 拒否は入らない側でよい（入れないことを順序 15 が確かめる）．UHD の run では `rates` の call が増えるため，device call を比べる層があれば許容が要る．
- **1（keys）**：rate の規則はモードを名指さない（順序 4 は両モードに効く）．T1〜T5 は今の key のモードの fixture（`profile()` の `rx` port，`crates/ezsdr-radio-uhd/tests/common/mod.rs:146-179`）で書き，layout のモードの run は順序 1・6・15 が同じ経路を通る．今のモードの Manifest・event・既存テストの結果を変えないことは，追加が `rates` の call と `device` section の record だけで，拒否は格子が違うときだけ起きる（既定の fake では一致，`:761-775`）ことに依る．

## 5. 同じ serial の拒否と解放

対象は brief.md:61-64（実装する中身）と brief.md:150-155（順序 5 の期待する結果）．`from_binding` で `serial()` を読み，プロセス全体の登録に入れ，生きている `UhdRadio` が同じ serial を持つなら `Rejected`，登録は `UhdRadio::Drop` で外す．`serial()` の失敗も `Rejected`．既定の `FakeDevice` の serial は instance ごとに一意にする．現状 `crates/ezsdr-radio-uhd/src/provider/mod.rs` に serial の語は無い（実行：`grep -n "serial" crates/ezsdr-radio-uhd/src/provider/mod.rs` → ヒット無し）．

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:96-131` `ur_05_from_binding_refusals`：1 つの device に `from_binding` を繰り返す（`:100` と `:125` は成功，その他は拒否）．成功した `UhdRadio` は文の終わりで drop されるので，**成功経路が drop で登録を返すこと**を間接的に固定している．名前・検証内容は変えない．
- `crates/ezsdr-radio-uhd/tests/fake.rs:162-181` `ur_05_the_profile_must_be_the_device_s_front_ends`：拒否の集合と文面．serial の登録はこの検査より前に読むので，拒否の文面・順序を変えないこと（`master_clock` → `set_subdev_spec` → front end の並びは item 2 と共通）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2362-2367` `ur_33_the_serial_is_the_config_s`：今は既定が `"FAKE0001"` であることと，config の serial が返ることを固定している．**この item で期待値を書き換える**（テスト名は変えない．(b) T0）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2002-2020` `ur_33_scripted_faults_fire`：`FakeFault::Lost` が以後の呼び出しを `lost: true` で失敗させること（T4 の前提）．`:2019` の `time_now().unwrap_err().lost`．
- `crates/ezsdr-server/tests/protocol.rs:930-943` `ea_07_a_reference_that_locks_on_the_reopen_connects`：catalogue の開き直し（UR-7）．`:904-917` の `unlocking_server` は open ごとに新しい `FakeDevice` を既定 serial で作るので，**同じ serial の 2 回目**は今は試していない（T6 が足す）．
- 変えてはいけない今のモードの振る舞い：`crates/ezsdr-radio-uhd/src/device.rs:824-829`（`serial()` は config の値を返し，`calls` に `"serial"` を残す）と `crates/ezsdr-radio-uhd/src/device.rs:747-755`（`Lost` の意味）．既存の mutation P199（`Ok(self.config.serial.to_owned())`）がこの 2 点を守らせている．
- 呼び出し列の完全一致を見る既存テストは無い（実行：`grep -rn "assert_eq!(\(device\|direct\.device\|.*\.device\)\.calls()\|assert_eq!(calls," crates/ezsdr-radio-uhd/tests/fake.rs` → `:1586` のフィルタ付き 1 件のみ）．`from_binding` が先頭に `serial` を 1 回足しても，件数を数える既存の assert（`:1451`，`:1524`，`:1757` などは prefix フィルタ）は影響しない．

### (b) 足すテスト

置き場所はすべて `crates/ezsdr-radio-uhd/tests/fake.rs`（`fake` binary）．serial は **プロセス全体の登録**なので，テストをまたいで衝突しないよう，テストごとに別の literal を使う（既定の fake は一意なので使ってよい）．`UhdRadio` は `Debug` を持たないので既存どおり `.err().unwrap()` で読む（`tests/fake.rs:98,163`）．操作列と観測は次のとおり．

1. **T2** `ur_05_a_second_binding_of_one_serial_is_refused`
   - `let serial = "DUP5-T2";`．`let a = fake(FakeConfig { serial: serial.to_owned(), ..FakeConfig::default() });`
   - `let first = UhdRadio::from_binding(&binding(json!({ "args": ARGS }), Some(x3xx())), a.clone()).unwrap();` の後 `let before = a.calls().len();`
   - (i) 同じ `Arc` へ 2 回目 → `Err`．assert：`kind == ModuleErrorKind::Rejected`（`tests/fake.rs:23` が import 済み），`message.starts_with("UR-5:")`，`message.contains(serial)`．
   - (ii) `a.calls()[before..] == ["serial"]`：同じ `Arc` への拒否は device の**書き込みを一切しない**（item 2 の `set_subdev_spec` より前に判定する．そうでないと H4 の「2 つ目の spec が 1 つ目を黙って上書きする」を自分で起こしてから拒否することになる）．
   - (iii) 別の `FakeDevice` で同じ serial（`fake(FakeConfig { serial: serial.to_owned(), ..Default })`）→ 同じ 3 つの assert（登録が serial で引かれていること）．
   - (iv) `first` が生きている間にもう一度 (iii) → やはり `Err`（拒否が 1 つ目の登録まで外さないこと）．
   - (v) `drop(first);` の後に (iii) → `Ok`（drop で解放）．
2. **T3** `ur_05_a_failed_binding_leaves_no_serial_registered`
   - `let serial = "DUP5-T3";`．`let a = fake(FakeConfig { serial: serial.to_owned(), master_clock_rate: 184_320_000, ..Default });`
   - `from_binding` → `Err` で `"UR-5: profile x3xx needs a 200 MHz"`（既存文面は `tests/fake.rs:127-130`）．
   - `assert!(!calls(&a, "serial").is_empty(), ...)`：**拒否された試行も serial を読んでいる**（読みと登録が，master clock の照合より前という前提を固定する．brief.md:154 の「serial を読んだ後の照合で失敗した → 登録は残らない」はこの順序を要求する）．
   - 同じ serial の健康な fake（`FakeConfig { serial: serial.to_owned(), ..Default }`，UBX 2ch 200 MHz）で `from_binding` → `Ok`．漏れていれば拒否される．
3. **T4** `ur_05_a_serial_read_that_fails_refuses`
   - `let a = fake(FakeConfig { serial: "DUP5-T4".to_owned(), faults: vec![FakeFault::Lost(Wall::ZERO)], ..Default });`（`Lost` は全呼び出しを `lost: true` で失敗させる，`device.rs:747-755`．文面は `device.rs:751` の `"fake: the device is gone"`）．
   - `Err` で `kind == Rejected`，`starts_with("UR-5:")`，`contains("the device is gone")`：
   - serial は `from_binding` の最初の device 呼び出し（`set_subdev_spec` より前）なので，`Lost(0)` では `serial()` が失敗する．これも順序の固定．
   - 同じ serial の健康な fake が `Ok`（失敗した試行が登録を残さない）．
4. **T7** `ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial`
   - `let a = fake(FakeConfig { serial: "DUP5-T7".to_owned(), faults: vec![FakeFault::Lost(Wall::from_millis(10))], ..Default });`
   - `first = from_binding(...).unwrap();` → `a.host().sleep(Wall::from_millis(20));` → `assert!(a.serial().is_err())`（前提）．
   - `drop(first);` → 同じ serial の健康な fake で `from_binding` → `Ok`．
   - `Drop` が device から serial を読み直す実装だと，ここで panic（`unwrap`）か解放漏れ（`unwrap_or_default`）になる．device が lost のときは device に触らない（`provider/mod.rs:556` と同じ原則）．
5. **T5** `ur_05_two_default_fakes_bind_together`
   - `let a = fake(FakeConfig::default()); let b = fake(FakeConfig::default());`
   - `assert_ne!(a.serial().unwrap(), b.serial().unwrap())`．
   - それぞれに `from_binding` して両方 `Ok`（同じ binary のテストが並列で走っても既定で衝突しない）．両方生かしたまま `drop((r1, r2))`．
   - これが無いと `fake` binary の既存テスト（`ur_09_…`，`ur_10_instance` など，`tests/fake.rs:141,466`）が並列で互いを拒否して大量に落ちる．つまり**一意な既定は登録と同時に入れる**．
6. **T0** `ur_33_the_serial_is_the_config_s`（`tests/fake.rs:2362-2367` を書き換え，名前は変えない）
   - `assert_ne!(fake(FakeConfig::default()).serial().unwrap(), fake(FakeConfig::default()).serial().unwrap())`（既定は instance ごと）．
   - `assert_eq!(fake(FakeConfig { serial: "F00BA2".to_owned(), ..FakeConfig::default() }).serial().unwrap(), "F00BA2")`（config の値はそのまま．既存 P199 がこれを守る）．
7. **T6** `ea_07_the_reopen_of_one_device_is_not_a_second_binding`（`crates/ezsdr-server/tests/protocol.rs`，`:930-943` の隣）
   - `open_device` の closure が開くたびに新しい `FakeDevice` を返し，**両方とも `serial: "REOPEN5".to_owned()`**，0 回目だけ `FakeFault::ReferenceDoesNotLock`（`unlocking_server`（`:904-917`）と同形）．
   - `Hello` → `Connect { profile: uhd_profile }` → `Response::Connected`，`opened.load() == 2`．
   - `finish` の Manifest の `timing` に `reopened_on_unlock`（`:938-941` と同じ観測）．
   - 1 つ目の `UhdRadio` が解放されないと，2 回目の `from_binding`（`catalogue.rs:128`）が `UR-5` で拒否され Connect が失敗する．物理の開き直しは新しい handle・同じ serial なので（H4），実機の UR-7 開き直しと同じ形．

### (c) 壊れうる形と mutation の行の案

行の ID は仮（P281〜）．`old` の正確な文字列はコードと一緒に決める．`file`/`test_bin`/`filter` は既存の並びに合わせる（例 P199：`file: crates/ezsdr-radio-uhd/src/device.rs`, `test_bin: fake`, `filter: ur_33_the_serial_is_the_config_s`）．

- **W1 二つ目を受け入れる**（挿入だけで照合しない）→ T2 (i)(iii) の `is_err` が落ちる．行：`provider/mod.rs`，照合の条件式を落とす（形：`if serials().contains(&serial) { return Err(...) }` を削除／`if false {`），`filter: ur_05_a_second_binding_of_one_serial_is_refused`．
- **W2 照合の鍵が serial でない**（`Arc` のポインタ，binding 名など）→ T2 (iii)（別 device・同じ serial）が落ちる．行：識別子を作る行を `format!("{:p}", Arc::as_ptr(&device))` に（1 行にまとまっている場合）．
- **W3 解放しない**（`Drop` なし／空／`cleanup` のみ）→ T2 (v) と T6 が落ちる．行：`Drop` の remove 行を無効化（形：`let _ = &self.serial;`），`filter: ur_05_a_second_binding_of_one_serial_is_refused`．
- **W4 拒否のときに登録まで外す**（1 つ目の分を消す）→ T2 (iv)（生きている間の 3 回目）が落ちる．行：重複分岐の `return Err(...)` の直前に `serials().remove(&serial);` を足す．
- **W5 serial を読んだ後の失敗経路で登録が残る**（早期 insert を戻り値に載せない）→ T3 が落ちる．行：`provider/mod.rs`，登録のガードを `UhdRadio` の構築時まで遅らせる／失敗経路で `mem::forget`（形はコードと共に）．`filter: ur_05_a_failed_binding_leaves_no_serial_registered`．
- **W6 `serial()` の失敗を握り潰す**（`unwrap_or_default()`，`.ok()`）→ T4 が落ちる．行：`let serial = device.serial().map_err(...)?;` を `let serial = device.serial().unwrap_or_default();` に．
- **W7 理由に serial を含めない** → T2 の `contains(serial)` が落ちる．行：理由の `format!` から `{serial}` を外す．
- **W8 理由が `UR-5:` で始まらない／種別が `Rejected` でない** → T2 の `starts_with`／`kind` assert が落ちる．行：種別を `Unsupported` にする等．
- **W9 既定の serial が定数のまま**（`"FAKE0001"`）→ T5 が落ち，さらに同 binary の並列テストが落ちる．行：`device.rs` の `Default` の serial 行を `serial: "FAKE0001".to_owned(),` に（形はコードと共に）．`filter: ur_05_two_default_fakes_bind_together`．
- **W10 `Drop` が device から serial を読み直す** → T7 が落ちる（lost で panic，または解放漏れ）．行：`Drop` の remove 行を `let serial = self.device.serial().unwrap_or_default(); …remove(&serial);` に．W3 と同じ行を触るので，`old` は `impl Drop` の見出しを含めて一意にする．
- **W11 serial を `set_subdev_spec` より後に読む** → T2 (ii)（`calls[before..] == ["serial"]`）と T4（`Lost(0)` で別の呼び出しが先に失敗し，serial の失敗の扱いが隠れる）が落ちる．行：読みと登録の 2 行を spec の呼び出しの下へ移す（隣接していれば 1 置換）．

当て直しが要りそうな既存の行：

- **P199**（`crates/ezsdr-radio-uhd/src/device.rs`，`filter: ur_33_the_serial_is_the_config_s`）：テスト名を変えず，`serial()` の本体（`device.rs:828`）を触らなければ `old` はそのまま．`new: Ok("FAKE0001".to_owned())` は書き換え後のテスト（既定が一意・明示値は config）で**なお落ちる**ことを確認する．`FakeConfig.serial` を `String` にするならこの行は無変化．
- **P13**（`provider/mod.rs`，`filter: ur_05_the_profile_must_be_the_device_s_front_ends`），**J174**（`filter: ur_05_from_binding_refusals`）：`from_binding` の同じ関数内だが別の行．`old` が引き続き厳密に 1 回一致するか確認して当て直す．
- `provider/mod.rs` の他の行（J36，J44，J66 など）と P244〜P280：AGENTS §7 の規則どおり，`old` の一致に加え，**名前を変えた識別子（`serials`・`hold`・`SerialGuard` など）が `new` に残っていないか grep** してから流す．

### (d) 頼る振る舞い

- **H4（VERIFIED，brief.md:108）**：同じ装置を 2 回開くと既存の `multi_usrp` が返り，2 つ目の binding の spec が 1 つ目を黙って上書きする（`multi_usrp_rfnoc.cpp:2902-2908`）．serial の拒否はこの破壊を防ぐための規則で，**拒否は device に何も書く前**（`set_subdev_spec` より前）でなければ意味がない（T2 (ii)）．
- 装置の同一性は motherboard の serial：`UhdDevice::serial` は `uhd_usrp_get_rx_info` の channel 0 の `mboard_serial`（`crates/ezsdr-radio-uhd/src/uhd.rs:693-708`；受信 channel の無い装置ではこの読みが失敗しうる，INFERRED，`design/18-uhd-radio.md:112`）．`Device` trait の doc も「refuses a second binding of one serial」と言っている（`crates/ezsdr-radio-uhd/src/device.rs:157-159`）．
- 実機の開き直しは**新しい handle・同じ serial**（`uhd.rs:480-485` の `open` は毎回 `uhd_usrp_make`）．だから登録は serial を鍵にし，`UhdRadio` の drop で外す（device の object の寿命ではない）．catalogue の開き直しループ（`crates/ezsdr-server/src/catalogue.rs:122-147`，`from_binding` は `:128`）が 1 つ目の radio を反復の終わりに落としてから次を呼ぶのは Rust の scope 規則による（T6 がそれを固定）．
- `FakeDevice` の serial は `FakeConfig::serial`（`crates/ezsdr-radio-uhd/src/device.rs:351-352,824-829`），既定は今 `"FAKE0001"`（`device.rs:377`）．既定を instance ごとに一意にするのは test double の規則（brief.md:64）で，根拠は H4 と「1 つの binary のテストは並列に走る」こと．`FakeConfig.serial` を `String` にし `Default` で通し番号を振る形を推奨．`&'static str` のまま既定値だけすり替える案は，明示した `"FAKE0001"` が別の値を名乗るので採らない（faithfulness）．
- `serial()` の失敗：fake では `FakeFault::Lost` が `lost: true` の `DeviceError` を返す（`device.rs:747-755`，文面 `device.rs:751`）．UR-5 の他の拒否と同じく `Rejected` にする（brief.md:63）．device が lost の後に触らない，という原則は `provider/mod.rs:556` に既にある（T7）．
- 実機の注意（INFERRED）：serial を master clock の照合より先に読むので，USRP2 probe（`crates/ezsdr-radio-uhd/tests/hardware.rs:290-305`，`#[ignore]`，実機でのみ）は，`get_rx_info` が serial を返さない装置なら拒否の文面が「200 MHz」から serial の失敗に変わる．BP0/BP9 で確かめる（現状の期待文面は `hardware.rs:304`）．
- 設計文の影響（「最後」の項目へ）：`design/18-uhd-radio.md:124`（UR-5 の拒否一覧に「生きている `UhdRadio` が持つ serial」と「`serial()` の失敗」を足す），`:279`（`serial` の既定 `"FAKE0001"` の文），`:470`（`ur_33_the_serial_is_the_config_s` の表の期待値）．UR-33 の *Test:* の列（`:286`）と `:97` の列挙は名前を変えないので触らない．

### 関わる他の順序

- **1（`keys` の値）**：selector の解析（`provider/mod.rs:124-161`）の後に serial の読みと登録を置く．selector が不正なら device に触れずに拒否する現状を変えない（item 1 の拒否の順序と device calls を狂わせない）．
- **2（layout から subdev spec）**：同じ `from_binding`．呼び出しの順は `serial` → master clock → `set_subdev_spec` → `channels`/`front_end`．item 2 の device calls の assert は，`set_subdev_spec` の前に `serial` が 1 つ入ることを許す形にする．T2 (ii) はこの順序を固定する．
- **4（rate の集合の照合）**：`from_binding` は `rates` を呼ばない（brief.md:145）．serial は rates ではないので衝突しない．T3/T4 の fake は `rates` を呼ばれる前に拒否することを確かめる副産物になる（`calls(&a, "rates")` が空，を足してもよい）．
- **14（Manifest）**：`Drop` のために `UhdRadio` が保持する serial の値がそのまま `device` section の serial になる．item 14 が `device.serial()` を読み直すなら 2 回目の読みになるが，T2 (ii) の `["serial"]` は**同じ `Arc` への拒否時のみ**を見るので影響しない．
- **15（VH-8）**：Mock 側に serial は無い．harness が 1 つのテストで UHD の radio を 2 つ生かす（作り直したうえで前を drop しない）と，この段階から拒否される．作り直しは前の `RunHandle` を消費してから行う．
- **「計画の最後に書くこと」**：(a) の一覧のうち書き換えるのは `ur_33_the_serial_is_the_config_s` の期待値のみ．`set_subdev_spec` の追加で device calls が変わる既存テストの一覧は item 2 の持ち物だが，item 5 も `from_binding` に `serial` を 1 つ足すので，同じ一覧に「先頭の `serial` を数えるテストが無いこと（実行した grep は (a) のとおり）」を添える．1b-4d へ送るものは brief.md:22 の一覧のまま（D14・channel 列・複数 stream）で，serial の登録と解放はここで閉じる．

## 6. prepare の stream 0

### (a) 既存のテスト

| テスト（`path:line`） | 固定していること | この項目との関係 |
|---|---|---|
| `crates/ezsdr-radio-uhd/tests/fake.rs:703-720` `ur_12_prepare_refusals` | 今のモードの prepare の拒否（Simulation，2 回目，Block link，3 channel） | 新しいモードの stream 1 の拒否を足しても，この4つは同じ kind と message のまま残る |
| `crates/ezsdr-radio-uhd/tests/fake.rs:722-734` `ur_12_a_tx_only_layout_declares_no_receive_clock` | channel が 0 の方向は SampleClock を宣言せず，channel も設定しない | 新しいモードの「stream 0 に channel がない」場合の直接の analogue（T5） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:736-748` `ur_12_the_applied_values_are_recorded` | 今のモードの `applied` の claimed/read-back と，方向ごとの `apply` | 新しいモードでも per-channel の `apply` と read-back の記録が同じ形で残る |
| `crates/ezsdr-radio-uhd/tests/fake.rs:750-758` `ur_12_a_rate_the_device_does_not_apply_is_refused` | rate の read-back が claim と違えば `UR-12` で拒否 | 新しいモードも同じ `configure` の判定を通る（T6） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:760-807` `ur_12_every_advertised_rate_is_accepted` | 639 の decimation rate，`decimation`，`200e6/3` の prepare と capture，`200e6/7` の cold switch | `s0.sample_rate_hz` はこの格子の値で，SampleClock の比は `N/1`。今のモードの rate の扱いを変えない |
| `crates/ezsdr-radio-uhd/tests/fake.rs:809-819` `ur_13_arm_cases` | `arm` の reference lock の拒否と，lock した場合の arm | 新しいモードの `arm` も同じ経路を通る（T1/T3） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:858-885` `ur_15_start_cases`，`ur_15_every_clock_starts_on_its_lattice` | 今のモードの start の拒否と，rx/tx の clock が lattice に載ること | 新しいモードの clock 宣言（`rx/s0`）が今のモードの clock を変えない |
| `crates/ezsdr-radio-uhd/tests/fake.rs:1592-1622` `ur_25_enabling_tx_applies_the_configuration` | 今のモードの `apply tx 0 rate=… freq=… gain=… antenna=… at=-` の完全な形 | `apply` の文面は新しいモードでも同じ。channel の番号は論理 channel の番号（T2/T3） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:1710-1761` `ur_25_a_receive_session_follows_its_recorded_plan` | 今のモードで streamer は 1 本，`rx_stop 0 now`，clock は `usrp/rx` | 今のモードの stream 0 は `usrp/rx` と `rx_open 0 [0..n]` のまま（新しいモードは `usrp/rx/s0`） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:2265-2288` `ur_33_an_empty_channel_list_is_channel_zero` | `rx_open(0, [])` は channel 0 の streamer になる（UHD の `sanitize_stream_args`） | stream 0 に channel がないときは `rx_open` を**呼ばない**ことの根拠（T5） |
| `crates/ezsdr-radio-uhd/tests/fake.rs:2369-2381` `ur_33_the_antenna_is_read_back_after_an_untimed_change` | untimed の antenna の read-back と，timed/antenna なしの `None` | prepare の antenna read-back の正の経路（T2/T7） |
| `crates/ezsdr-radio/tests/device.rs:93-121` `rm_26_the_description_builds_the_tree` | 今の builder の `<id>/rx`・`<id>/tx` node と port `rx` | 今のモードの tree を変えないこと |
| `crates/ezsdr-radio/tests/channels.rs:319-415` `rm_29_the_layout_description_builds_the_stream_tree` | layout builder の `<id>/rx/s<k>` node と port `rx<k>`，channel ごとの capability | 新しいモードはこの tree をそのまま使う（T1/T3/T5） |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs:544-575` `mr_39_each_receive_stream_declares_and_runs_its_own_clock`，`:577-598` `mr_13_a_stream_s_block_channels_follow_its_channel_set` | Mock の stream ごとの channel 集合と SampleClock，block の channel 列 | VH-8（item 15）の Mock 側の期待値。UHD 側の T1/T3 と対応する |

補助の harness は `tests/fake.rs:534-568` の `direct_prepare`，`:570-581` の `attached`（port `rx` を固定），`:596-701` の `Direct`（`prepare`/`arm`/`start` をまとめて行う）。新しいモードのテストはこの3つを拡張する（(b)）。

### (b) 足すテスト

すべて `crates/ezsdr-radio-uhd/tests/fake.rs` に置く。`from_binding` は `binding(json!({"args": ARGS, "keys": "channel", "rx": [...], "tx": [...]}), Some(x3xx()))` で作る（`fake.rs:82-93` の `binding` は，明示した `rx`/`tx` を上書きしない）。`FakeDevice` はテストごとに `fake(FakeConfig::default())`（`tests/common/mod.rs:298`）で作り，item 5 の「instance ごとに違う既定 serial」に依存する（並列のテストが同じ serial で衝突しないため）。

**T1 `ur_12_a_layout_s_stream_zero_declares_its_clock_and_opens_its_channels`**
- 準備: rx `["0:A:ubx:0", "0:B:ubx:0"]`，tx `[]`。constraints は `radio.rx.c0.stream=-1`，`radio.rx.c1.stream=0`，`radio.rx.s0.sample_rate_hz=2e6`。`attached_on("rx0", DropOldest)` を prepare の `ctx.links` に入れる。
- 通す列: `from_binding` → `prepare` → `arm`（`start` は呼ばない。`Direct::build` の prepare/arm までを `Direct::armed` として切り出し，`build` はそれを呼んでから `start` する。挙動は変えない）。
- assert:
  - `device.calls()` の `rx_open` はちょうど `["rx_open 0 [1]"]`（`starts_with("rx_open")` で filter）。
  - `direct.clocks.sample_clock_records()` は `stream.path() == "usrp/rx/s0"` が1本，`root_ticks_per_tick == Rational::new(100, 1)`。`usrp/rx` の record は無い。
  - `direct.radio.instance().tree.ports` は `["rx0", "rx1"]`（rx が2 channel のため）。`"rx"` は無い。`tree.children` に `usrp/rx/s0` と `usrp/rx/s1` がある。
  - `device.calls()` に `apply rx 1` があり，`apply rx 0` は無い（c0 は stream 0 に居ない）。`rx_open 0 []` は無い。
- 観測: device の calls（`device.rs:1047-1053` の `rx_open` の記録）と，`ClockRegistry::sample_clock_records()`，`instance().tree`。

**T2 `ur_12_a_layout_s_stream_zero_applies_each_channel_s_own_settings`**
- 準備: rx 2 channel，`c0.stream=0`，`c1.stream=0`，`s0.sample_rate_hz=2e6`。`c0` は freq `1e9`・gain `5.0`・antenna `RX2`，`c1` は freq `2.4e9`・gain `7.5`・antenna `TX/RX`。link は T1 と同じ `rx0`。
- 通す列: `from_binding` → `prepare` → `arm`。
- assert: `rx_open 0 [0, 1]`。`device.calls()` の `apply rx` の列は，順に
  `apply rx 0 rate=2000000 freq=1000000000 gain=5 antenna=RX2 at=-`，
  `apply rx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX at=-`
  （各1回。`device.rs:980-988,1021` の fake の文面）。
- 観測: 同じく device の calls。`coerce` の gain は 0.5 dB の格子なので `5` と `7.5` がそのまま出る。

**T3 `ur_13_a_layout_s_transmit_stream_zero_opens_its_channel_list`**
- 準備: rx `[]`，tx `["0:A:ubx:0", "0:B:ubx:0"]`。`c0.stream=-1`，`c1.stream=0`，`tx.s0.sample_rate_hz=2e6`，`tx.c1` は freq `2.4e9`・gain `7.5`・antenna `TX/RX`。link は不要（`arm` の tx は channel があれば開く）。
- 通す列: `from_binding` → `prepare` → `arm`。
- assert: `tx_open 0 [1]`。`apply tx 1 rate=2000000 freq=2400000000 gain=7.5 antenna=TX/RX at=-` があり，`apply tx 0` は無い。`sample_clock_records()` に `usrp/tx/s0` が1本，比 `100/1`（`arm` の lattice 登録まで見る）。
- 観測: device の calls と `ClockRegistry`。

**T4 `ur_12_a_layout_stream_above_zero_is_refused`**
- 準備: rx 2 channel と tx 2 channel の2ケース。それぞれ `c0.stream=0`，`c1.stream=1`。`direct_prepare(&mut radio, device, HardwareInLoop, Vec::new(), constraints)`（`fake.rs:534-568`）を使う。
- 通す列: まず `radio.coerce(&request(constraints))`（`fake.rs:502-507`）が `Ok` で `rejected` が空であることを確かめる（description の範囲は `-1..=1` のまま）。次に `prepare`。
- assert: `Err(ModuleErrorKind::Rejected)`。message は `"stream 0"` と `"c1"`（または `radio.rx.c1.stream`）を含む。`device.calls()` に `apply`，`rx_open`，`tx_open` が無い（拒否は `configure` より前）。
- 観測: `ModuleError` の kind と message，device の calls。rx と tx の両方を回すのは，ガードが片方向だけの実装を落とすため。

**T5 `ur_12_a_layout_s_stream_zero_with_no_channel_declares_no_clock`**
- 準備: rx 2 channel，`c0.stream=-1`，`c1.stream=-1`。link は `rx0`。
- 通す列: `from_binding` → `prepare` → `arm`。
- assert: prepare と arm が成功する。`device.calls()` に `rx_open` が無く，`apply rx` も無い。`sample_clock_records()` に `usrp/rx/s0` が無い。`instance().tree` には `usrp/rx/s0` と port `rx0` が残る（RM-29 の「channel のない stream は node だけで clock を持たない」）。
- 観測: device の calls，`ClockRegistry`，`instance().tree`。`rx_open(0, [])` を呼ぶ実装は fake では channel 0 の streamer になる（`ur_33_an_empty_channel_list_is_channel_zero`）ので，この assert がその誤りを落とす。

**T6 `ur_12_a_layout_a_rate_the_device_does_not_apply_is_refused`**
- 準備: rx 2 channel，`c1.stream=0`，`s0.sample_rate_hz=2e6`。`FakeConfig { faults: vec![FakeFault::WrongRate { claimed: 2e6, applied: 1.9e6, nth: 0 }], ..FakeConfig::default() }`。
- 通す列: `direct_prepare` で `prepare`。
- assert: `Err(Rejected)`，message は `"UR-12: the device applied"` と `"1900000"` / `"2000000"` を含む（`core.rs:553-555` の文面）。
- 観測: `ModuleError` の message と device の `apply`（`nth: 0` は stream 0 の最初の channel）。

**T7 `ur_12_a_layout_an_antenna_read_back_that_differs_is_refused`**
- 準備: rx 2 channel，`c1.stream=0`，`c1.antenna=TX/RX`。`FakeConfig` に「untimed の antenna の read-back を別の名前にする」fault（item 9 が #59 の形で足す `WrongAntenna { chan: 1, applied: "RX2" }` を想定）を入れる。
- 通す列: `direct_prepare` で `prepare`。
- assert: `Err(Rejected)`，message は `"UR-12"` と `"TX/RX"` / `"RX2"`（または channel 1）を含む。`device.calls()` に `rx_open` は無い（`configure` で落ち，`arm` に達しない）。
- 観測: `ModuleError` と device の calls。fault の定義と配置は item 9 の計画に属する。prepare と cold 変更は同じ `Core::configure` を通るので，fault は1つで両方の経路を試せる（item 9 の cold のテストと対にする）。

### (c) 壊れうる形と mutation の行の案

`old` の正確な文字列はコードと一緒に決める。以下は「壊す規則」と「形」で，サイトは `old`/`new` を置く場所。

| # | 誤った実装の形 | 落ちる assert | mutation の行の案（規則と形） |
|---|---|---|---|
| 1 | stream 0 の channel 集合を layout の全 channel `0..n` にする（`mod.rs:387-397` の `Description::stream_channels` を使わない） | T1 `rx_open 0 [1]`; T5（全 channel を設定/オープン） | `provider/mod.rs` の `stream_channels(config, dir, 0)` を `Channels::first(layout_count)` / `0..layout_count` に |
| 2 | SampleClock を `<id>/rx` に宣言する（`mod.rs:398`） | T1 の `usrp/rx/s0` | `id.child(&format!("{}/s0", dir.name()))` を `id.child(dir.name())` に |
| 3 | rate を `radio.rx.sample_rate_hz`（今の key）から読む（`core.rs:502-517`，`mod.rs:396`） | T1 の比 `100/1`，`apply … rate=2000000`; T2 | `keys::stream(dir, 0)` を `format!("radio.{}.sample_rate_hz", dir.name())` に |
| 4 | `prepare` の configuration filter が今の key set のまま（`mod.rs:345`） | T1/T2（s0 rate と per-channel 値が defaults に落ちる） | `channels::is_configuration(key)` を `keys::CONFIGURATION.contains(&key.as_str())` に |
| 5 | 全 channel に同じ `Settings` を渡す（`core.rs:533-556`） | T2 の `apply rx 1` の値 | `settings_for(config, dir, chan)` を `settings_for(config, dir, first)` に |
| 6 | antenna を `Settings` に入れない（`core.rs` の新 per-channel 設定） | T2 の `antenna=TX/RX`; T5 の no `apply` | `antenna: …` を `antenna: None` に |
| 7 | `device.apply` の channel に集合内の位置を渡す（論理番号でない） | T1 の `apply rx 1` が `apply rx 0` になる | `apply(dir, *chan, …)` を `apply(dir, position, …)` に |
| 8 | stream 0 でなく layout の全 channel を `configure` する（`core.rs:535`） | T1 の `apply rx 0` 不在; T5 | `for chan in set.iter()` を `for chan in 0..layout_count` に |
| 9 | `arm` の `rx_open`/`tx_open` が集合でなく `0..n` を渡す（`mod.rs:427,433`） | T1 `rx_open 0 [1]`; T3 `tx_open 0 [1]` | `stream.channels.iter()` を `0..count` / `(0..stream.channels.count())` に |
| 10 | channel のない stream でも `rx_open(0, [])` を呼ぶ（`mod.rs:424`） | T5 の no `rx_open`（fake は `[]` を channel 0 にする） | `if stream.channels.is_empty()` を `if false` に |
| 11 | channel のある stream で `rx_open` を呼ばない（古い `channels == 0` の誤用） | T1 の `rx_open 0 [1]`; T3 | `if stream.channels.is_empty()` を `if true` に |
| 12 | stream 1 以上のガードがない，または `> 1` だけ拒否する（`prepare` の新 guard） | T4 の `Err`（rx/tx） | `if any_stream > 0 { return Err(…) }` を削除，または `> 0` を `> 1` に |
| 13 | ガードを `configure` の後に置く | T4 の no `apply` | `configure` の呼び出しをガードの前に移す（1 行で表せないならガード削除の行 12 と同じテストで落とす） |
| 14 | ガードが rx だけ（tx を見ない） | T4 の tx ケース | `for dir in [Dir::Rx, Dir::Tx]` を `[Dir::Rx]` に |
| 15 | rate の read-back の検査を新モードでしない（`core.rs:553-555`） | T6 の `Err` | `if applied.rate != claim` を `if false` に |
| 16 | antenna の read-back を比べない（`configure` の新比較） | T7 の `Err` | 比較の `if applied.antenna.as_deref() != Some(antenna.as_str())` を `if false` に |
| 17 | tx の SampleClock を `<id>/tx` に宣言/登録する（`mod.rs:398`，`core.rs:563-568`） | T3 の `usrp/tx/s0` | `id.child(&format!("{}/s0", dir.name()))` を `id.child(dir.name())` に |
| 18 | tx の per-channel 設定を rx の key，または先頭 channel の値で作る | T3 の `apply tx 1` の値 | `settings_for(config, Dir::Tx, chan)` を `settings_for(config, Dir::Rx, chan)` / 先頭 channel に |

既存の mutation 行で `Core::channels` を `new` に含むのは `control.rs` の `J68`・`J69` だけ（`grep` で確認: `python3` で `plan/maintenance/tools/mutations.json` の全 663 行を `Core::channels` / `Core::settings` / `configure` / `declare_sample_clock` / `0..channels` / `rx_open(` で検索し，当たったのは `J68`・`J69` の2行）。`Core::settings`/`Core::configure` を対象にした既存行は無いので，この項目の行は追加が主。ただし `Core::channels` を新しいモード対応の関数に置き換えるなら，`J68`・`J69` の `new` に古い識別子が残っていないかを grep する（1b-4b の gate の U41）。`P280`（`start` の 1 stream ガード）は `prepare` ではなく `start` の行なので，この項目では触らない（item 10/12 で見る）。

### (d) 頼る振る舞い

| # | 振る舞い | 根拠 | 確かさ |
|---|---|---|---|
| D1 | `set_subdev_spec` は方向の channel map を作り直し，device の channel 番号は spec の順になる。streamer の `channel_list` はその device channel を開く | H1。`multi_usrp_rfnoc.cpp:2822-2834`。fake は `device.rs:831-862` と `1047-1073` で同じ | VERIFIED（H1） |
| D2 | layout の要素を論理 channel の順に spec にするので，論理 channel `n` は device channel `n` になる（T1/T3 の `rx_open 0 [1]`，`tx_open 0 [1]`，`apply … 1` の前提） | design 33 D5 の「device の channel の番号は論理 channel の番号と同じ」; H1/H2。spec の markup は `subdev_spec.cpp:19-20,58-59` | VERIFIED（H1，H2） |
| D3 | `apply(dir, chan, …)` はその channel だけを変える。frequency/gain/antenna は channel ごとの radio block の register で，同じ stream の他 channel を変えない | UR-2 の `Device` trait（`device.rs:176`）。UHD の `set_{rx,tx}_{freq,gain,antenna}` の channel 引数 | VERIFIED |
| D4 | 1 本の streamer の rate は1つで，stream の全 channel が同じ rate で動く。`s0.sample_rate_hz` を stream の各 channel に適用するのは design の stream の定義 | RM-3，RM-30，design 33 D3/D4。UR-12 の `configure` | VERIFIED（design） |
| D5 | antenna の getter は設定した property を読み，物理のスイッチを測らない。`Applied.antenna` は untimed の変更でその値を返す | H6。design 33 D16。fake は `device.rs:1039-1044`，`ur_33_the_antenna_is_read_back_after_an_untimed_change` | VERIFIED（H6） |
| D6 | 空の channel 列は channel 0 になる（UHD の `sanitize_stream_args`）。だから channel のない stream に `rx_open(0, [])` を渡してはいけない | `ur_33_an_empty_channel_list_is_channel_zero`; fake `device.rs:1051-1053` | VERIFIED（1b-4a） |
| D7 | H3（DDC の rate 集合は chain の前は `{1.0}`）はこの項目では使わない。rate の**集合**の照合は item 4 の `arm`。ここで見るのは apply された rate と SampleClock の比だけ | H3。`ddc_block_control.cpp:153-159` | この項目では非依存（item 4） |
| D8 | H7（fake の空 spec・重複の拒否は UHD より厳しい）もこの項目では使わない。空 spec は item 2/3 | H7。`multi_usrp_rfnoc.cpp:2796` | この項目では非依存（item 2/3） |

`apply` の正確な文面はテストダブルの観測形式（`device.rs:976-1045`）で，UHD の振る舞いではない。T2/T3 は claimed の値を calls から読み，read-back の正負は T7 と `Applied.antenna` で見る。

### 関わる他の順序

- **item 1（`keys` のモード）**: T1〜T7 の selector は `keys: "channel"` で，`coerce`・`tree`・`defaults`・`stream_channels` は `channels::Description` のもの。item 1 が無いと `from_binding` が `keys` を拒否し，T4 の `coerce` も `RM-4: not a radio key` になる。前提。
- **item 2（subdev spec）**: T1/T3 の device channel 番号は spec の順。テストの UBX A/B は既定の順と同じなので item 2 が無くても通るが，`from_binding` の `set_subdev_spec` が増えると `device.calls()` の先頭に `set_subdev_spec rx …` が入る。T1/T3 は `starts_with("apply")` / `starts_with("rx_open")` で filter するので影響しない。
- **item 3（front end の照合）**: テストの UBX A/B は既定の front end と一致する。T1/T3 は item 3 の照合の後に `channels`/`front_end` を読む実装でも通る。入れ替えた layout の channel 番号は item 2/3 のテストが持つ。
- **item 4（rate の集合）**: T1/T3 は 2e6（格子の値）で `arm` する。item 4 の「方向ごとに最初の streamer を開いた後に `rates` を比べる」が入っても，この rate は一致するので T1/T3 は緑のまま。T6 は apply の rate の read-back で，集合の照合とは別。
- **item 5（同じ serial）**: 各テストは新しい `FakeDevice` を作る。item 5 の「既定の serial を instance ごとに違う値にする」が無いと，並列のテストが同じ `FAKE0001` で互いを拒否しうる。T1〜T7 はこの変更に依存する。
- **item 7（channel ごとの timed の更新）**: prepare の per-channel `apply` と timed の `apply` は同じ `settings_for` / `configure` を使う。T2 の `apply rx 1 …` の値が，item 7 の `radio.rx.c1.frequency_hz` の timed `apply` の値と一致することを item 7 のテストで確かめる。T2 は prepare 時点の固定。
- **item 8（stream の rate の `cold`）**: T1/T3 が作る `PreparedStream` は channel の集合（`Channels`）と `s0` の rate を持つ。item 8 は新しい segment でこの集合の全 channel を untimed に設定する。T1/T5 の「集合が空なら開かない」は item 10 の「最後の channel が出る」場合の基礎。
- **item 9（antenna の `cold`）**: T7 の fault は item 9 が #59 の形で足すもの。prepare の read-back と cold 変更の read-back は同じ `Core::configure` の比較を通る。T7 は prepare 側，item 9 は cold 側を固定する。
- **item 10（membership）**: T4 の prepare の拒否と，item 10 の実行時の `→ 1 以上 → COMMAND_REJECTED` は同じ stage の制限。理由の文面（`stream 0` を含む）を共有する。T5 の「channel のない stream 0 は開かない」は，最後の channel が出た後の stream の扱いの基礎。
- **item 11（instance 全体の床）**: `PreparedStream` の集合と rate が `Lines` の初期状態に入る。T1/T3 の set がその値を決める。床の計算そのものは item 11。
- **item 12（宛先）**: T1/T3 の node `usrp/rx/s0`・`usrp/tx/s0` と port `rx0` が，item 12 の `UpdateParameter`/`Stop`/`start_rx`/`TxBurst` の宛先の前提。`Core::new` の `rx_id`/`tx_id` を `<id>/rx/s0`・`<id>/tx/s0` に変えるなら，`Direct` のイベントの `pairs`（`fake.rs:627` の `["usrp","usrp/rx","usrp/tx"]`）に `usrp/rx/s0`・`usrp/tx/s0` を足す必要がある（`prepare` の `RX_OVERFLOW` の handle の `resolve` が通るため）。T1〜T7 はイベントを assert しないが，prepare が成功するためにこの整合が要る。item 12 の計画と共有する。
- **item 13（実行時の判定）**: prepare の `coerce` は `channels::Description::coerce` の1か所。T4 は `coerce` が `c1.stream=1` を受け，`prepare` が stage の制限で拒否することを分けて固定する。実行時の `violations` は item 13。
- **item 14（Manifest）**: `device` section の spec markup・serial・rate の照合結果は item 14。T1/T3 が見るのは Kernel の `sample_clock_records` と device の calls で，`device` section ではない。`applied` の per-channel key（`radio.rx.c1.frequency_hz` など）の記録は UR-30/item 14 の担当。T2 は calls で見るので，`applied` の key の変更とは独立。
- **item 15（VH-8）**: Mock 側の `mr_39`/`mr_13` が stream 0 の channel 集合と SampleClock の期待値。UHD 側の T1/T3（clock の node と比）と T4（同じ拒否）が VH-8 の入力になる。per-channel の `apply` は Mock に device が無いので VH-8 の比較対象ではない。
- **「計画の最後に書くこと」**: UR-12 の「direction の全 channel を構成する」を layout モードでは「stream の channel 集合を構成する」に，UR-13 の「receive channels の streamer」を「stream の集合の streamer」に，UR-30 の `applied` の key を `c<n>` にする，といった `design/` の文の列挙と，既存 mutation 行の当て直しの一覧は，最後の項目が持つ。この項目の T1〜T7 と (c) の行はその一覧へ渡す。

## 7. channel ごとの timed の更新

対象は `crates/ezsdr-radio-uhd/src/provider/control.rs` の `book_timed`（`:398-435`），`release`（`:441-488`），`channels_at`（`:389-394`），`segment_config`（`:342-348`），`projections`（`:351-353`），`ColdConfig`（`:56-103`），`Core::settings`/`Core::configure`（`crates/ezsdr-radio-uhd/src/provider/core.rs:502-560`），装置側の `apply`（`crates/ezsdr-radio-uhd/src/device.rs:976-1045`）．規則は指示書 7 の3つ：`radio.rx.c1.frequency_hz` を `at` 付きで受けたら channel 1 にだけ timed の `apply` が 1 回；走っている stream に入っていない channel の更新は記録だけして装置に渡さず，次にその channel が入る stream の設定で untimed に適用する；device の queue は channel・方向をまたいで 1 列．

UHD Module には channel key を名指すテストも `keys` selector も今は無い（実行した検索: `grep -rn '"keys"' crates/ezsdr-radio-uhd/src/ crates/ezsdr-radio-uhd/tests/` → 0 件; `grep -rn "radio\.rx\.c[0-9]\|radio\.tx\.c[0-9]" crates/ezsdr-radio-uhd/` → 0 件）ので，(b) のテストは全部新規．意味の先行実装は MockRadio の layout モード (1b-3b) にあり，それが VH-8 の相手側になる．

### (a) 既存のテスト

**今のモードの timed の機構（変えてはいけない）**

- `crates/ezsdr-radio-uhd/tests/fake.rs:1327` `ur_24_hardware_timed_updates` — `at` 1 ms 先（late → `LATE_COMMAND`，適用は receipt の device lead 後）と 40 ms 先（`at=<at>` の timed `apply rx 0` が 1 回），16 深の queue の 17 個目が `COMMAND_QUEUE_FULL`（`:1331-1354`）．現行の release 機構と fake の call の書式を固定する．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1359` `ur_24_a_far_future_update_does_not_delay_a_nearer_one` — 遠い方に遅らせない（`:1368-1380`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1764` `ur_24_the_queue_counts_a_command_per_channel` — 現行 key は 1 命令＝方向の channel 数（2 channel で 8 個が queue を満たし，9 個目が拒否．`:1768-1774`）．channel モードの数え方（T3b）の対．
- `crates/ezsdr-radio-uhd/tests/fake.rs:3873` `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` — 切替の設定が origin の投影を持ち，timed は 1 回だけ出る（`:3892-3899`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:3904` `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` — enable の設定が同 instant の timed を含む／含まないが到着順で決まる（`:3912-3931`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1315` `ur_24_an_untimed_retune_is_on_time_after_its_delivery` — `at` 無しの timed 更新が late にならない（`:1319-1323`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1404` `ur_24_a_released_command_is_not_recalled_by_stop`，`:1432` `ur_24_stop_cancels_held_commands` — `Stop` は取り消さず，`Provider::stop` だけが held を消す（`:1423-1428`, `:1451-1455`）．channel ごとの更新でも不変．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:946` `ur_25_later_timed_update_survives_cold_switch` — cut 前の instant の timed が新 segment の設定に載る（`:961-966`）．T2 の untimed 適用の参照．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:970` `ur_25_switch_projects_timed_updates_in_effective_order` — 投影の有効順（`:971-982`）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1098` `ur_24_a_timed_update_acts_on_the_configuration_in_force_at_its_instant` — 自分の instant の channel 数で判定，記録だけの行は `note: "no channel"`（`:1113-1116`）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1120` `ur_24_a_direction_halted_by_a_refusal_has_no_channel` — halt 中は記録だけで enable が適用，`Stop` 済みは発行（`:1153-1163`）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1168` `ur_24_a_halted_direction_fills_no_queue_slot`（`:1180-1187`），`:1217` `ur_24_a_late_update_halted_before_it_can_act_has_no_channel`．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1191` `ur_24_the_device_queue_order_is_the_effective_order` — 1 本の queue の床（`:1203-1213`）．T3a の現行 key の対．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1357` `ur_24_the_device_queue_is_one_across_the_directions` — 方向をまたいで 1 列（`:1367-1377`）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1241` `ur_25_a_receive_enable_without_a_link_is_only_recorded`，`:1280` `ur_24_a_refusal_halts_only_its_own_stream`，`:1323` `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants`．

**channel ごとの規則の参照（MockRadio の layout モード）**

- `crates/ezsdr-mock-radio/tests/mock_layout.rs:601` `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` — channel key の timed が自分の channel に効き，applied の行は key ごと（`:610-616`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1489` `mr_33_layout_gain_and_frequency_timelines_are_per_logical_channel` — gain/freq は channel ごとの timeline．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1525` `mr_34_layout_mixed_boards_have_per_channel_timed_phase_behavior` — 同じ instant の 4 つの timed が channel ごとに効く（`:1569-1583`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1132` `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` — timed は cold の床に載らない（`:1140-1148`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1719` `mr_34_a_layout_receive_entry_is_an_untimed_tune` — 出て戻った channel の新 segment は untimed tune（`:1742-1760`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1960` `mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels` — `start_rx` の enable も同じ（`:1982-1999`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:2203` `mr_34_a_layout_unlinked_receive_stream_takes_no_untimed_tune` — link の無い受信 stream は untimed tune を取らない．
- 実装側: `crates/ezsdr-mock-radio/src/lib.rs:1877-1891`（`hardware_timed` の受付と queue は 1 命令 1 個，`:1881-1882`），`:2061-2113`（適用），`:1090-1115`（`untimed_tune`）．

**装置側**

- `crates/ezsdr-radio-uhd/tests/fake.rs:2310` `ur_33_a_timed_command_follows_its_front_end_across_a_spec` — timed の宛先は channel 番号でなく front end（`crates/ezsdr-radio-uhd/src/device.rs:1002-1007`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2383` `ur_33_the_fake_device_keeps_the_devices_queues` — in-order の queue（`:2397-2402`）．
- `crates/ezsdr-radio-uhd/tests/hardware.rs:506` `hw_b8_raw_queue_depth` — queue 深の実測（USRP が要る，`#[ignore]`）．

### (b) 足すテスト

前提（テストより先に要るもの．実装と一緒に決める）: channel モードの `Direct`／`session` は既存の `Direct::build`（`crates/ezsdr-radio-uhd/tests/fake.rs:619-663`，selector の追加は `:654-656`）で書ける（selector に `keys: "channel"` と `rx`/`tx` の要素列を渡す）．unit 側（T3a）には channel モードの rig が要る：`crates/ezsdr-radio-uhd/src/provider/test_support.rs` の `rig_with_links`（`:19-30`）の layout 版と，`crates/ezsdr-radio-uhd/src/provider/control.rs:785-797` の `control()` の channel 版．`Control::new` の line の channel 集合は `Description::stream_channels`（`crates/ezsdr-radio/src/channels.rs:557-566`）から入る（指示書「stream 0 の channel 集合は `Description::stream_channels` で読む」）．

**T1 `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）**

- 狙い：`radio.rx.c1.frequency_hz`（と `gain_db`）の timed 更新が channel 1 にだけ 1 回渡り，plan も clock も動かないこと．
- Case A（2 channel とも stream 0）: `Direct::with_links(FakeConfig::default(), &[("radio.rx.c1.stream", Scalar::from(0))], vec![attached(DropOldest)])` を channel モードの selector（`rx: ["0:A:ubx:0", "0:B:ubx:0"]`，`tx: ["0:A:ubx:0"]`）で建てる．`at = direct.now() + ms(40)` に `radio.rx.c1.frequency_hz = 2.1e9`（`HardwareTimed`），`at + ms(1)` に `radio.rx.c1.gain_db = 3.0` を入れ，`direct.settle(Wall::from_millis(80))`．
- Case B（channel 1 だけが stream 0）: constraints `[("radio.rx.c0.stream", Scalar::from(-1)), ("radio.rx.c1.stream", Scalar::from(0))]`（stream 0 の集合は `{1}`．streamer は `rx_open 0 [1]`）．同じく `c1.frequency_hz` を `at` 付きで入れる．Case B は「stream 内の位置」と「論理番号」を分ける．
- Case C（任意，tx）: tx 2 channel で `radio.tx.c1.stream = 0`，`radio.tx.c1.gain_db` を `at` 付きで入れる（指示書の例は rx なので任意）．
- assert（各 case）: device calls を読み，(i) `apply rx 1 ` かつ `freq=2100000000` の timed 呼び出し（`at=<at>` を含む）がちょうど 1，(ii) `apply rx 0 ` かつその freq が 0，(iii) 同 key の `at=-` が 0，(iv) gain も同様に `apply rx 1` ちょうど 1・`apply rx 0` 0，(v) `radio.COMMAND_REJECTED` と `radio.LATE_COMMAND` が空，(vi) `direct.clocks.sample_clock_records()` の `<id>/rx/s0`（channel モードの node，`plan/phase8/33-channels-and-streams.md:40`）が増えない（timed は cut しない），(vii) `direct.finish()` の `ezsdr.radio.uhd.applied` に `radio.rx.c1.frequency_hz` の `issued: true`（`at.ticks == at`）が 1（書式は `control.rs:484-486`）．
- 観測: `direct.device.calls()`（fake の timed call は `apply rx 1 rate=- freq=2100000000 gain=- antenna=- at=… effective=…`，`crates/ezsdr-radio-uhd/src/device.rs:1007`），`direct.clocks`，`instance.sections`．

**T2 `ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）**

- 狙い：走っている stream に入っていない channel の更新は記録だけして渡さず，その channel が入る stream の新しい segment の設定で untimed に適用されること（指示書 7 の2つ目）．
- 形: channel モード，`rx: ["0:A:ubx:0", "0:B:ubx:0"]`，`tx: ["0:A:ubx:0"]`，constraints なし（既定で c0 が stream 0，c1 は −1），link 1 本．(1) `at = direct.now() + ms(40)` に `radio.rx.c1.frequency_hz = 2.1e9`（`HardwareTimed`），`settle(ms(80))`．(2) `direct.update("radio.rx.c1.stream", Value::from(0), Cold, None)`，`settle(ms(200))`．
- assert (1): `apply rx 1 ` の呼び出しが1つも無い（`apply rx 0 ` もこの freq では無い）．`radio.COMMAND_REJECTED` が空．`direct.clocks` の `<id>/rx/s0` が 1 本のまま（記録だけの更新は cut しない．`control.rs:459-463` の形）．
- assert (2): `rx_open 0 [0, 1]` がある（segment が c1 を得て始まったことの確認．`crates/ezsdr-radio-uhd/src/provider/rx.rs:286` の login）．`apply rx 1 ` かつ `freq=2100000000` かつ `at=-` がちょうど 1．同じ key の `at=<…>` は 0（過去の instant で timed に渡さない）．`apply rx 0 ` に `freq=2100000000` が無い（漏れない）；`apply rx 0 ` の untimed 設定は `freq=1000000000`（c0 の既定）を含む．`COMMAND_REJECTED` は空．
- assert（`direct.finish()` 後，`applied` section）: (a) 記録だけの行（現行の形は `{ key, claimed, read_back: null, at: <その命令の instant>, note: "no channel" }`，`control.rs:461`）が `radio.rx.c1.frequency_hz` について 1 つ（`issued` は無い），(b) entry 後の untimed 設定の行（`channel: 1`，`claimed == read_back == 2.1e9`；行の形は `core.rs:544-551`）がある．
- 観測: `direct.device.calls()`，`direct.of("radio.COMMAND_REJECTED")`，`direct.clocks`，`instance.sections[...applied]`．

**T3a `ur_24_the_device_queue_is_one_across_the_channels`（unit，`crates/ezsdr-radio-uhd/src/provider/control.rs` の `mod tests`）**

- 狙い：release の床が 1 本で，channel（と方向）をまたいで効くこと（指示書 7 の3つ目）．release window の中の順序なので，制御時計の unit にする（`crates/ezsdr-radio-uhd/tests/README.md:54` が `ur_24_the_device_queue_order_is_the_effective_order` をこの理由で unit に置いている）．
- 形: channel モードの rig（rx stream 0 = `{0,1}`，tx stream 0 = `{0}`）で，(1) `book_timed(1, key("radio.rx.c1.gain_db"), 1.0, at(10_000))`，(2) `time.advance_to(core.at(us(7_500)))` して `release()`（c1 が release window 内で 10 ms に渡る），(3) `book_timed(2, key("radio.rx.c0.gain_db"), 2.0, at(9_700))` して `release()`，(4) `book_timed(3, key("radio.tx.c0.gain_db"), 3.0, at(9_600))` して `release()`（`us`/`at` は `control.rs:1200-1201` の形）．
- assert: `apply rx 0 … gain=2` と `apply tx 0 … gain=3` の `at=` がどちらも 10 000（c1 の release に載る），`LATE_COMMAND` が 2 件で requested/applied が (9 700, 10 000) と (9 600, 10 000)，`control.held` が空，`COMMAND_QUEUE_FULL` が無い．
- 観測: rig の `FakeDevice::calls()` と `EventCollector`（`control.rs:1208-1213` の形）．

**T3b `ur_24_a_channel_keyed_command_counts_one_queue_slot`（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）**

- 狙い：channel key の 1 命令は queue の 1 枠（方向の channel 数ではない）こと．
- 形: `Direct::build(FakeConfig { command_queue: 64, ..FakeConfig::default() }, &[("radio.rx.c0.stream", Scalar::from(0)), ("radio.rx.c1.stream", Scalar::from(0))], vec![attached(DropOldest)], channel selector)`．`far = direct.now() + ms(5_000)` に c0 を 8 個・c1 を 8 個（`gain_db`，instant を 1 tick ずつずらす）．`settle(ms(30))` で `COMMAND_QUEUE_FULL` が空．17 個目（c1）を入れて `settle(ms(30))` で `COMMAND_QUEUE_FULL` がちょうど 1 件，payload の `depth == 16`（`control.rs:424`，`fake.rs:1354` の形）．
- 現行 key の対照（`fake.rs:1764`：2 channel で 8 個が上限）と違い，channel key は 16 個で上限になる．方向の channel 数で数える実装なら 9 個目で拒否され，前半の「空」assert が落ちる．

### (c) 壊れうる形と mutation の行の案

`old`/`new` の正確な文字列は実装と一緒に決める（この段階にはまだコードが無い）．形式は「壊す規則 → 落ちる (b) の assert → 行の形」．

| # | 壊れうる形 | 落ちる assert | mutation の行の案 |
|---|---|---|---|
| C1 | release が名指しの channel でなく stream（方向）の全 channel に渡す | T1 A の (ii)(iv)，T2 (2) の `apply rx 0` 漏れ | `release()` の適用先を stream の channel 集合（現行の `for chan in 0..channels {`（`control.rs:472`）の形）にする．killer T1 |
| C2 | 論理番号でなく stream 内の位置（index）を使う | T1 B（`apply rx 1` が無く `apply rx 0` が出る） | 適用先を `Channels::iter()` の位置／`enumerate()` の `i` にする．killer T1 B |
| C3 | 走っていない channel（membership −1）にも timed で渡す | T2 (1) の「`apply rx 1` が 1 つも無い」 | 記録だけの門（`if channels == 0 {`，`control.rs:459`）を外す／channel の所属を見ない．killer T2 |
| C4 | 記録だけの枝が値を configuration に残さない | T2 (2) の `freq=2100000000` の untimed 適用が無い | `self.config.insert(...)`（`control.rs:460`）を消す．killer T2 |
| C5 | 記録した値を方向全体の settings に載せる（他 channel に漏れる） | T2 (2) の `apply rx 0 … freq=2100000000` | 投影・設定の channel の濾過を方向だけにする（現行の `\|timed\| timed.dir == dir`，`control.rs:344`・`:569` の形）．killer T2 |
| C6 | queue の数え方を方向（stream）の channel 数にする | T3b の前半（9 個目で `COMMAND_QUEUE_FULL`） | 新しい命令・held の数え方を direction の channel 数にする（J68/J69 の `new` の向き；killer T3b） |
| C7 | release の床を channel ごとに持つ（1 列でない） | T3a（rx の `at=` が 9 700 のまま・LATE_COMMAND 2 件でない） | `let latest = self.released.iter().copied().max();`（`control.rs:454`）を同じ channel の release だけに絞る（P268 の channel 版．killer T3a） |
| C8 | key の class を見ず `cold` として記帳する | T1 の (vi)（plan/clock が動く）と `at=` の timed が無い | `Control::book` の `HardwareTimed` の腕（`control.rs:285-288`）を `book_cold` へ回す．killer T1 |
| C9 | channel モードでも `radio.rx.channels`（現行 key）を読む | T1（記録だけになり `at=` の `apply` が出ない） | channel モードの読みを `Core::channels`（`core.rs:519-528`）のままにする．killer T1 |

当て直しが要りそうな既存の行（この項目の近く．全部は「計画の最後」が集める）: J68（`control.rs:422`），J69（`:423`），J71（`:458`），P268（`:454`）は `channels_at`／`released` の形が変われば．P264（`control.rs:100`）と P279（`:86`）は `ColdConfig` が channel ごとになれば．P265（`:569`）と P267（`:430` の `Timed { … }`）は `Timed` に channel が付き held の併合が channel ごとになれば．P232（`device.rs:1005`）は装置側なので不変のはず．

### (d) 頼る振る舞い

- **H2**（表にある，VERIFIED）: markup の書式（`<slot>:<front end>` を ASCII 空白で）→ layout から作る spec が device の channel の番号を決め，`apply rx 1` の `1` の意味になる．T1/T2 の setup がこれに載る（`subdev_spec.cpp:19-20,58-59`；指示書の表 H2）．
- **H8（追加）: 装置の timed command の queue は 1 本で順序どおり．後から渡した先の instant の命令は，先に渡したものを追い越さない**．根拠: UHD の `set_command_time` は後続の timed command を back-pressure する（`design/18-uhd-radio.md:22`，VERIFIED，`uhd/usrp/multi_usrp.hpp`）．fake はこれを `effective = at.max(st.last_command).max(now)` と in-order の settle で持つ（`crates/ezsdr-radio-uhd/src/device.rs:990-1013`，`:714-740`；`ur_33_the_fake_device_keeps_the_devices_queues`，`fake.rs:2383`）．深さ 16 は bench の実測（`crates/ezsdr-radio-uhd/tests/hardware.rs:506`）．Provider 側の規則（`released` の床，`control.rs:454-455`）がこれを守る．**実機で「先の instant の命令を後から渡すとどうなるか」の細部は INFERRED**（Provider はそう渡さない設計）．
- **H9（追加）: 流れていない chain への timed command は queue に溜まり，あふれると装置の full restart が要る**．根拠: `plan/phase8/33-channels-and-streams.md:174-177`（design 33 D9 規則 1・2，`usrp_x3x0.dox:861-863` を VERIFIED として引く）．ただしこれは DDC/DUC に行く command の話で，B8 は流していない radio block の tune が queue に溜まるだけであることを示す（`hardware.rs:506-528`，INFERRED）．この順序で効くのは「渡さず，次の設定で untimed に適用する」という規則の側で，規範は `design/18-uhd-radio.md:244`（UR-24 の「channel が無ければ記録だけ，enable が適用する」）を channel ごとに読んだもの＋指示書 7．
- **H10（追加）: timed の宛先は channel 番号の固定でなく，booking 時の front end（radio block）で，適用は今の spec のその front end の channel に行く**．根拠: `crates/ezsdr-radio-uhd/src/device.rs:1002-1007`（fake）と `ur_33_a_timed_command_follows_its_front_end_across_a_spec`（`fake.rs:2310`）．VERIFIED（fake）．T1 の「channel 1 にだけ」の `apply rx 1` がこの意味で読める．
- H1・H3〜H7 はこの順序の assert には効かない（T1/T2/T3 は spec の設定そのもの，rate の集合の照合，antenna の read-back，空 spec を観測しない）．
- 装置の `apply` の channel 番号が spec の範囲外なら失敗する（`crates/ezsdr-radio-uhd/src/device.rs:994-995`，`:1018-1019`）ので，channel の取り違えは沈黙せず call の失敗として観測に出る．

### 関わる他の順序

- **1（`keys` の値）**: T1/T2/T3 の前提（`keys: "channel"` の dispatch と，channel key の `HardwareTimed` class．`crates/ezsdr-radio/src/channels.rs:948-955`）．ここが誤ると item 7 のテストは prepare で落ち，item 7 の所見ではなくなる．
- **2・3（subdev spec と front end の照合）**: `apply rx n` の `n` は item 2 の spec から来る．spec の順・要素が違えば channel の番号が変わり，T1/T2 の call の assert が全部ずれる．device は layout の front end を持つ `FakeConfig` にする（既定 `["UBX","UBX"]`，`crates/ezsdr-radio-uhd/src/device.rs:376`）．
- **6（`prepare` の stream 0）**: `apply` の per-channel 経路（`Core::configure`／`ColdConfig::configure`）は item 6 の `prepare` と T2 の untimed 適用が共有する．T2 の `rx_open 0 [0, 1]` と `apply rx 1 … at=-` は item 6 の観測の一部でもある．
- **8（stream の rate の `cold` 変更）**: cut の新 segment が held／記録の値を untimed に載せる機構は同じ（`ur_25_later_timed_update_survives_cold_switch`，`control.rs:946`）．T2 の「次の設定で適用」はこの経路の channel 版．
- **9（antenna の `cold`）**: cold 側の兄弟．antenna が `book_timed` に入らないこと（class の切り分け）は item 1/9 の共有．
- **10（membership の変更）**: T2 の第 2 相そのもの．entry の streamer の channel 列（`rx_open 0 [0, 1]`）が誤れば，記録値が別の channel に載り T2 の `apply rx 1` assert が動く．
- **11（instance 全体の床）**: timed は cold の床に載らない（`mr_18_a_layout_hardware_timed_change_keeps_its_own_instant`，`mock_layout.rs:1132`；UR-24）．item 11 の床が `book_timed` に漏れれば T1 の `at` assert が動く．
- **12（宛先）**: T1/T2/T3 は `<id>` 宛て（`Direct::update`，`fake.rs:673-681`）．item 12 が `<id>/rx/s0` を足しても `<id>` は残るので，このテストは動かない．
- **13（実行時の判定）**: 更新の値（2.1 GHz，3 dB）は UBX の範囲内・antenna 変更なしなので RM-31/RM-32 に触れない．誤れば T1/T2 の「拒否が空」assert に出る．
- **14（Manifest）**: T1/T2 は `applied` の行を読む（`control.rs:484-486`，`:461`，`core.rs:544-551`）．`device` section の markup・serial・rate の照合は item 14 の範囲．
- **15（VH-8）**: T2 の列（channel key の timed → 走っていない channel に記録 → membership で入る → untimed 適用）は，同じ selector と Spec を MockRadio と UhdRadio に与える VH-8 の1ケースとしてそのまま使える．Mock 側の対は `mr_34_a_layout_receive_entry_is_an_untimed_tune`（`mock_layout.rs:1719`）と `mr_34_a_layout_stream_enable_is_an_untimed_tune_of_its_channels`（`:1960`）．

## 8. stream の rate の `cold` 変更

対象（実装の入る場所）：booking は `crates/ezsdr-radio-uhd/src/provider/control.rs:509-588`（対象 stream の選択は今 `:519` の `booked_stream(dir)` 固定），切替の設定は `control.rs:94-102`（`ColdConfig::configure`）→ `crates/ezsdr-radio-uhd/src/provider/core.rs:533-560`（`Core::configure` の `0..channels` ループ），投影は `control.rs:84-91`（`ColdConfig::settings`）と `control.rs:342-353`（`segment_config`／`projections`），受信の切替は `crates/ezsdr-radio-uhd/src/provider/rx.rs:273-320`（reopen・configure・timed start・clock の declare），送信は `crates/ezsdr-radio-uhd/src/provider/tx.rs:647-687`（`do_switch`）。キーの読みは `crates/ezsdr-radio/src/channels.rs:557-573`（`stream_channels`／`stream_of`）と `channels.rs:948`（`update_class`）をそのまま使う想定。

### (a) 既存のテスト

今のモードで，この振る舞いを固定しているもの（変えてはいけない）：

- `crates/ezsdr-radio-uhd/tests/fake.rs:1389` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` — 切替（e₂ の segment 開始）が untimed の `apply` で設定を全部かけ直し，保持された timed 更新（`at = now+320 ms`）の値を含む（assert `:1399`）。本順序の芯の今のモード版。
- `crates/ezsdr-radio-uhd/tests/fake.rs:3873` `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch` — 到着順 2 通りで，timed 更新が切替の設定に入る（`:3891-3895`）。
- `crates/ezsdr-radio-uhd/tests/fake.rs:761` `ur_12_every_advertised_rate_is_accepted` — `cold` 切替で 200e6/7 に到達し，2 本目の受信 clock の比が 7（`:799-806`）。
- `crates/ezsdr-radio-uhd/tests/fake.rs:1462` `ur_25_a_stream_silent_before_e1_switches_before_e2` — cut の untimed stop が timed start より先（`:1491-1493`），`rx_segment` 行の `at`／`origin`（`:1486-1488`），opened の streamer は 1 本。
- `crates/ezsdr-radio-uhd/tests/fake.rs:1711` `ur_25_a_receive_session_follows_its_recorded_plan` — 2 回の `cold` rate 変更をまたぐ plan の segment 列，clock と cut の対応，untimed stop の記録，`rx_open` が 1 回（`:1735-1759`）。
- `crates/ezsdr-radio-uhd/tests/fake.rs:1516` `ur_25_rx_channels_from_zero_starts_at_its_instant` — clock 記録の読み方（`direct.clocks.sample_clock_records()`，`:1524-1529`）。
- `crates/ezsdr-radio-uhd/tests/fake.rs:840` `ur_14_the_control_thread_never_waits_for_a_device_instant` — rate 変更後に受信 clock が 2 本（`:854-855`）。
- 送信側：`crates/ezsdr-radio-uhd/tests/fake.rs:1624` `ur_25_a_rate_change_admits_a_burst_on_the_new_clock`，`:1638` `ur_25_a_cold_transmit_change_cancels_the_held_bursts`（`:1646` "cancelled by a cold change"）。
- lib の unit：`crates/ezsdr-radio-uhd/src/provider/control.rs:970` `ur_25_switch_projects_timed_updates_in_effective_order`，`:946` `ur_25_later_timed_update_survives_cold_switch`，`:1323` `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants`，`:899` `ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it`，`crates/ezsdr-radio-uhd/src/provider/rx.rs:1078` `ur_25_a_second_change_follows_the_first`。
- まだ走っていない stream への `cold` が値だけになる今の型：`crates/ezsdr-radio-uhd/src/provider/control.rs:1241` `ur_25_a_receive_enable_without_a_link_is_only_recorded`。

MockRadio の同じモードの対（VH-8 の相手方，書き方の手本）：

- `crates/ezsdr-mock-radio/tests/mock_layout.rs:662` `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` — rate のキーが名指す stream だけが cut する（`:718-728`）。
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:733` `mr_09_each_transmit_stream_has_its_own_clock_and_line` — 送信の `cold` rate 変更が自分の clock だけを替え，lead 後に新しい clock（`:753-765`）。
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:2046` `mr_34_a_layout_rate_change_is_an_untimed_tune` — 受信（`:2070-2086`）。
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:2357` `mr_34_a_layout_transmit_rate_change_is_an_untimed_tune` — 送信の rate 変更で新 clock の origin で全 channel が untimed に設定される（`:2384-2411`）。
- channel の無い stream の状態：`crates/ezsdr-mock-radio/tests/mock_layout.rs:793` `mr_7_each_receive_port_takes_its_own_links`（「channel の無い stream に link があっても segment を始めず clock も無い」）。

UHD Module に channel mode のテストはまだ 1 本も無い（検索：`grep -n 'keys.*channel\|"channel"\|rx0\|layout\|is_layout\|channels::' crates/ezsdr-radio-uhd/tests/fake.rs` は 4 件だけで，`:723` の `ur_12_a_tx_only_layout_declares_no_receive_clock` とコメント・assert 2 行，`:2448` のコメントのみ）。

### (b) 足すテスト

3 本とも `crates/ezsdr-radio-uhd/tests/fake.rs`，`Direct` harness（`fake.rs:619-663`。selector の追加 key と constraint を取れる）。harness への追加は 1 つだけ：link を channel mode の port `rx0` に張れること（今の `attached()` は `:570-581` で port `rx` 固定；`prepare` の link 検査は `crates/ezsdr-radio-uhd/src/provider/mod.rs:358`）。この拡張はこの段階の複数項目（6・7・8・9・10・12・14・15）で共有する。観測は device の calls（`FakeDevice::calls()`，finish の前に読む），`direct.clocks.sample_clock_records()`，`finish()` 後の `ezsdr.radio.uhd.timing`／`applied` セクション。

**T1 `ur_25_a_layout_rate_change_cuts_its_stream_and_configures_every_channel`**（本順序の芯）

- 構成：`FakeConfig { front_ends: vec!["UBX", "BasicRX"], ..default }`，selector `{"keys": "channel", "rx": ["0:A:ubx:0", "0:B:basic_rx:0", "0:B:basic_rx:1"], "tx": ["0:A:ubx:0"]}`，link は `rx0`（DropOldest），constraints は `radio.rx.c1.stream = 0`，`radio.rx.c0.frequency_hz = 2.4e9`，`radio.rx.c1.frequency_hz = 25e6`（c2 は既定の −1，どの stream にも入らない。UBX の gain は 20.0 を constraint，BasicRX の gain は既定 0 のまま。owner の混在構成 `plan/phase8/33-channels-and-streams.md:122` と同じ形）。
- 通す操作：`now = direct.now()` を取ってから（1）`radio.rx.c1.frequency_hz = 30e6`（HardwareTimed，`at = now+320 ms`）→（2）`radio.rx.s0.sample_rate_hz = 2e6`（Cold，`at = now+300 ms`）→ settle 500 ms。（1）を先に積むのは，切替の `segment_config`（`control.rs:342-348`）が booking 時に保持中の timed 更新を projection に入れるため。
- assert：
  1. calls に `rx_stop 0 now` があり，その後に `rx_start 0 <origin>` があり，両者の間の `at=-` の `apply rx` がちょうど 2 本：`apply rx 0 rate=2000000 freq=2400000000 gain=20 antenna=RX2 at=-` と `apply rx 1 rate=2000000 freq=30000000 gain=0 antenna=AB at=-`（channel 0 → 1 の順）。channel 1 が 30e6 なのは（1）の値が入った証拠，channel 0 が 2.4e9 なのは channel ごとに自分の値である証拠。`rate=2000000` で絞るので prepare の apply（1e6）と区別する。
  2. 切替の前後に `apply rx 2 … rate=2000000 … at=-` が無いこと（segment の channel 集合に無い channel は設定しない。prepare が channel 2 にも apply するかは item 6 の判断で，ここでは問わない）。
  3. `rx_open 0 [0, 1]` がちょうど 1 回（channel 数が変わらない切替では開き直さない。今の R14 と同じ規則）。
  4. `usrp/rx/s0` の clock 記録が 2 本：1 本目 `root_ticks_per_tick = 200`，`ended_at = Some`（plan の 1 本目の `end` と同じ値）；2 本目 `root_ticks_per_tick = 100`，`origin` は 1 本目の終わりより後で 100 の格子の上（RM-25）。
  5. `finish()` 後の `timing`：`plan.rx` が 2 segment（1 本目の `end` は 1 本目の clock の `ended_at`，2 本目の `end` は Run の stop の cut），`rx_segment` 行が origin = 2 本目の clock の origin，`channels = 2`。
  6. tx 側が無傷：切替後に `tx_` で始まる call も `apply tx` も無い；`usrp/tx/s0` の clock は 1 本で `ended_at` はまだ None（「stream 0 だけが cut され」の，この段階で観測できる対照）。
- 観測：device の calls，clock 記録，`timing` セクション。

**T2 `ur_25_a_layout_rate_change_of_a_channel_less_stream_only_takes_its_value`**

- 構成：T1 と同じ（tx は無しでよい。rx 3 channel，c1 のみ stream 0）。
- 通す操作：`radio.rx.s1.sample_rate_hz = 2e6`（Cold，`at = now+100 ms`）を 1 本だけ → settle 200 ms。
- assert：切替操作の後に device の call が 1 本も増えない（`rx_stop`・`apply`・`rx_start` のどれも無い）；clock 記録が増えない（`usrp/rx/s0` は 1 本のまま）；`finish()` 後の `timing` に新しい `rx_segment` 行が無い；`radio.COMMAND_REJECTED` が空。値そのものが Manifest のどの行に残るか（`applied` 行や `cold_change` 行の有無）は今のモードの no-line の扱い（`control.rs:548-551`）と揃えるかを実装と一緒に決め，このテストは「効かない」ことだけを固定する。
- 根拠：RM-21（`design/07-radio-model.md:241`「a change to a stream that does not run only applies its value」），design 33 §3（`plan/phase8/33-channels-and-streams.md:46`「channel が1つも入っていない stream は SampleClock も block も持たない」），Mock の同型（`crates/ezsdr-mock-radio/tests/mock_layout.rs:793`）。

**T3 `ur_25_a_layout_transmit_rate_change_configures_the_new_segment`**（同じ規則の送信側の対）

- 構成：`front_ends: ["UBX", "BasicTX"]`，selector `{"keys": "channel", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0", "0:B:basic_tx:0"]}`，link は `rx0`，constraints は `radio.tx.c1.stream = 0`，`radio.tx.c0.frequency_hz = 2.4e9`，`radio.tx.c1.frequency_hz = 25e6`。
- 通す操作：`radio.tx.s0.sample_rate_hz = 2e6`（Cold，`at = now+100 ms`）→ settle 300 ms（tx thread は cut の instant に `do_switch` する，`tx.rs:237-244`）。
- assert：`apply tx 0 rate=2000000 freq=2400000000 … at=-` と `apply tx 1 rate=2000000 freq=25000000 … at=-`（各 channel が自分の値，untimed）；`usrp/tx/s0` の clock が 2 本（比 200 → 100，1 本目は cut で終わり，2 本目は booking 時に登録済み）；`timing` に `tx_switch` 行（`cut` と `origin`）；`tx_open 0 [0, 1]` は 1 回だけ（channel 数が変わらないので開き直さない）；受信側に新しい call が無い。burst は置かない（held burst の取り消しは `ur_25_a_cold_transmit_change_cancels_the_held_bursts` の規則のままで，本順序の芯ではない）。
- Mock 側の対：`mock_layout.rs:2357`（送信 rate 変更の untimed tune），`mock_layout.rs:753-765`（自分の clock だけ）。

（任意：channel mode の projection を純の関数（`ColdConfig` 相当）に切り出すなら，その関数の unit で channel ごとの fold 順を 1 本。無理に作らない。）

### (c) 壊れうる形と mutation の行の案

`old` の正確な文字列はコードと一緒に決める。行の id もそのとき。すべて (b) のどの assert が殺すかを付ける。

1. **キーが名指す stream を無視し，常に stream 0 に記帳する**（`control.rs:519` の channel mode 版）→ T2 が落ちる（`s1` の変更が `s0` を cut し，stop・segment・新 clock・call が出る）。行の案：channel mode の stream 選択を `let stream = booked_stream(dir);` に固定する（`old` は選択の式，`new` は `booked_stream(dir)`）。
2. **切替の設定が一部の channel にしか当たらない**（例：集合を回すループの `take(1)`）→ T1 assert 1（channel 1 の untimed apply が stop と start の間に無い），T3 も。行の案：`Core::configure` の channel mode の腕（`core.rs:535` のループの集合版）を 1 channel に。
3. **全 channel に同じ 1 つの設定を当てる**（最初／最後の channel の値）→ T1 assert 1（channel 0 が 2.4e9 で channel 1 が 30e6 の両方を求めているので，どちらの向きの取り違えも落ちる）。行の案：channel ごとの `Settings` の引きを `for_channel(0)` に固定する（`old` は channel ごとの引き，`new` は 0 固定）。
4. **切替の設定が保持された timed 更新を落とす**（projection を base だけで作る）→ T1 assert 1（channel 1 が 25e6 になり 30e6 でない）。行の案：`ColdConfig::settings` の `updates.range(..=(origin, u64::MAX))` を空 range `..(origin, 0)` に（今の行 P279 `plan/maintenance/tools/mutations.json`，R05 の channel mode 版。関数が移れば当て直す）。
5. **timed 更新の値をその channel だけでなく全 channel に配る**→ T1 assert 1（channel 0 が 30e6 になる）。行の案：projection の更新を channel で引かず全 channel に畳む（3 と同じ assert が殺す）。
6. **新しい clock の比を古いまま取る**（`control.rs:553`）→ T1 assert 4，T3（2 本目が 200 のまま）。ただしこれは既存 `ur_12_every_advertised_rate_is_accepted`（`:799-806`）も殺すので，行は既存の当て直しで足りる（文が変わらなければ新行は不要）。
7. **切替の適用を timed（`at` 付き）で出す**→ T1 assert 1（`at=-` を要求），T3 も。行の案：channel mode の `configure` → device の呼び出しを `device.apply(dir, chan, &settings, Some(self.core.now()))` に（`Core::configure` の channel mode の腕）。
8. **cut を他の stream にも記帳する／他方向も切る**（全 line を回す形）→ T1 assert 6（tx の call と clock），T3（受信側の無傷）。

既存の行で，この段階の書き換えで `old` が動きそうなもの（当て直しは送り (11) と同じ形で，`old` の一致だけでなく `new` に消えた識別子が残っていないかも grep する）：R05・P264・P279（projection と `settings`），U40（`rx.rs:287` の configure 腕），R14（`rx.rs:286` の reopen 条件），P253・P252・P256・P242（`book_cold` の from-none と no-line），U22（`clocks.last`），B15（`control.rs:195` の `rx_packet_samples(0)`），P267。

### (d) 頼る振る舞い

- **装置の channel 番号 = 論理 channel 番号**：`apply rx 1` が論理 1 の front end に届くこと（T1 の call の番号と値の assert の前提）。根拠は上の表の H1（`multi_usrp_rfnoc.cpp:2822-2834`，VERIFIED）と item 2 の「spec を論理の順に作る」，`FakeDevice` の同じ表（`crates/ezsdr-radio-uhd/src/device.rs:409-442`，UR-33）。
- **untimed stop → untimed configure → timed start** の列（UR-17，`design/18-uhd-radio.md:207-212`；configure は UR-12，`:193`）。今のモードでは実機で確認済み：`plan/phase7/bench-results.md:342-353`（`hw_b8_cold_change_capture`），`:811-821`（低 rate と長い block），`:910-918`（200 Msps と，受信 call の 4 位相），`:376-399`（restart lead は装置側 1 ms で足りる。`:399` "The device restarts on the requested tick at every lead down to 1 ms, warm, after a rate change"）— VERIFIED（1 channel，今のモード）。多 channel・混在での同じ列は **INFERRED**（BP4b：`plan/phase8/00-overview.md:120`）。
- **連続受信 stream の timed stop は効かない**（X3x0 は即時停止）ので cut は plan の値で，device には untimed stop を渡す：spec 18 §2（`design/18-uhd-radio.md:29`）と UR-25（`:246`），bench F1（`plan/phase7/bench-results.md:339`）— VERIFIED。
- **channel 数が変わらない切替では streamer を開き直さない**（UR-25 の switch の "reopens the streamer if the channel count changed"）：今のモードで warm restart（bench `:399`，開き直しは "new streamer" の行だけ `:384-396`）— VERIFIED（1 channel）。channel mode の同じ形は INFERRED（BP4b）。
- **cut の後の gap に timed 更新が release されてよいか**は D9 の DDC/DUC の規則（段階2，`plan/phase8/33-channels-and-streams.md:174-178`）の未決で，実機は INFERRED（BP0／段階2）。T1 はどちらの扱いでも成立する（device に渡す untimed の configure が値を含むことだけを見る）。
- **fake の観測の形**：untimed の apply は `apply <dir> <chan> … at=-`（`crates/ezsdr-radio-uhd/src/device.rs:1021`），timed は `at=<e> effective=<e'>`（`:1007`）。UR-33 の記録。

### 関わる他の順序

- **1・2・6**：同じ harness（`keys: "channel"`，`rx0` の link，channel の key の constraint）と channel mode の description を共有する。T1 の `rx_open 0 [0, 1]` と `apply rx <n>` は item 6 と重なる（item 6 が `prepare` を，item 8 が実行時の切替を押さえる）。`Channels::first(n)` → stream の集合の置き換え（1b-4c 送り (5)，stream 0 だけ）は T1 の segment の `channels = 2` と apply の channel 番号が押さえる。
- **7**：T1 の timed 更新（`radio.rx.c1.frequency_hz`）は channel 1 の key で，release が「channel 1 にだけ 1 回」であること，stream に入っていない channel の更新の扱いは item 7 のテストが押さえる。T1 は release の仕方に依存しない。
- **9**：同じ切替・`configure` の経路（antenna の read-back と拒否の halt は item 9）。T1 の `antenna=` の値は既定（RX2／AB）だけで，9 の分を先取りしない。
- **10**：membership の変化・`start_rx` も同じ「cut → 新 segment → 全 channel の untimed 設定」を使う。T2 の channel の無い stream は item 10 の「最後の channel が出る」と隣り合う。plan の行に stream の番号と segment ごとの channel 列を入れるのは 1b-4d の送り (9)。
- **11**：T1〜T3 は明示の `at` を 1 つだけ入れ，先に受けた `cold` 変更が無いので instance 全体の床に触れない（床は item 11 のテスト）。
- **13**：切替の設定も `violations` を通る（RM-31・RM-32）。T1 は受信 2 channel 2 Msps（16 MB/s），T3 は送信 2 channel 2 Msps（16 MB/s）で 1 GbE の 100 MB/s 以下（`design/07-radio-model.md:347`）に収まる値を選んである（切替が RM-32 で拒否されない）。
- **14**：切替の apply の `applied` 行の key の形（`radio.rx.c1.frequency_hz` か番号なしの形か）と `device` section は item 14 が決める。T1・T3 は device の calls と clock だけを見る。
- **15**：MockRadio の同じ操作は `crates/ezsdr-mock-radio/tests/mock_layout.rs:718-728`・`:2046`・`:2357`。VH-8 は stream 0 の clock の origin と比を突き合わせる。
- **1b-4d へ**：plan の行の stream 識別と segment ごとの channel 列（送り (9)），`rx_packet_samples(k)` の k 一般化（送り (2) の残り），複数 stream の VH-8（送り (4)）。

## 9. antenna の cold 変更

対象は `keys: "channel"` の instance の `radio.{rx,tx}.c<n>.antenna`（update class `cold`，RM-27: `design/07-radio-model.md:306`）で，その channel の入っている stream を区切り（RM-25: `design/07-radio-model.md:257`），新しい segment の設定として device に適用し `Applied.antenna` を読み返して記録し（D16: `plan/phase8/33-channels-and-streams.md:194-200`），read-back が違えば RM-25 のとおり拒否して halt する（UR-12 の「Configuring a direction」と UR-17 の halt: `design/18-uhd-radio.md:193,209`）．今の key のモードの `radio.{rx,tx}.antenna` の更新は今どおり `UR-26` で拒否する（`design/18-uhd-radio.md:249`）．

### (a) 既存のテスト

いちばん近いもの:

- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1110` `ur_25_a_refused_segment_is_booked_by_uhd_rx` — 装置が segment の設定を拒否したときの halt の形（item が `refused`，`rx_start` は前の 1 回だけ，拒否された値が設定の記録として残る）．(b) の read-back 拒否テストの雛形．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1389` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` — cold 変更が新しい segment の設定を device の calls（`apply rx 0 …`）で見る形．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1638` `ur_25_a_cold_transmit_change_cancels_the_held_bursts` — 送信側の cold 変更（送信 antenna のテストが同じ道を通る）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1559` `ur_25_a_cold_change_the_envelope_refuses_changes_nothing` — 拒否された cold 変更は何も変えない（`Direct` + `COMMAND_REJECTED` + `apply` なし）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2370` `ur_33_the_antenna_is_read_back_after_an_untimed_change` — device 境界の antenna read-back（untimed だけ値，無指定と timed は `None`；`device.rs:1042-1044`，`uhd.rs:886-905`）．この段階は 1b-4a で入ったこの規則の上に載る．
- 同じ規則の Mock 側: `crates/ezsdr-mock-radio/tests/mock_layout.rs:662` `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream`（`675-690` 行が antenna: stream 1 が cut され，新しい clock は cut + 受信 call 2 ms + 3 ms + lead 50 ms = `T0+65 ms`，stream 0 は無傷）．順序 15 の VH-8 はこれを Mock 側の基準にする．

今のモードの振る舞いを固定しているもの（変えない）:

- `crates/ezsdr-radio-uhd/tests/fake.rs:1593` `ur_25_enabling_tx_applies_the_configuration`（assert は `:1606`）— `apply tx 0 … antenna=TX/RX at=-` を完全一致で見る．prepare / enable の configure が antenna を渡し続けることの pin．
- `crates/ezsdr-radio-uhd/tests/fake.rs:737` `ur_12_the_applied_values_are_recorded` — 今のモードの `applied` の行（claimed / read_back）の形．

見つからないもの（実行した検索をそのまま）:

- 今のモードの antenna の更新の拒否（`UR-26: … is not updated at runtime by this Provider`，`src/provider/control.rs:294`）を確かめるテストは無い．
  `grep -n 'UR-26:' crates/ezsdr-radio-uhd/tests/fake.rs` → `1652`（コメント）と `1670`（`Stop` の宛先）の 2 件だけ．
- 今のモードの antenna の `applied` の行を読むテストは無い．UHD の tests は antenna の current-mode key をまったく名指していない．
  `grep -rn 'radio\.rx\.antenna\|radio\.tx\.antenna' crates/ezsdr-radio-uhd/tests` → 0 件．`grep -n 'antenna' crates/ezsdr-radio-uhd/tests/fake.rs` → `1606` と `2370-2379`（device 境界のテスト）だけ．

### (b) 足すテスト

共通の足場（テストと同じ回で入れる）:

- `attached`（`crates/ezsdr-radio-uhd/tests/fake.rs:570-581`）は port `rx` を決め打ちする．keys のモードの port は `rx0`（RM-29: `design/07-radio-model.md:324`，段階の item 6）なので，port を引数に取る `attached_on(policy, "rx0")` を足す（`attached` はそれを呼ぶだけにする）．
- `Direct::build`（`fake.rs:619-663`）は selector の追加 key をすでに受ける（`:654-656`）．keys のモードは `json!({"keys": "channel", "rx": [...], "tx": [...]})` で組む（layout が既定の 2 channel を上書きする）．waveform input は空（`fake.rs:642`）なので，burst を持つテストは `Direct` では作れない（既存の burst テストは session 側: `fake.rs:1024-1032`）．
- 新しい fault `FakeFault::WrongAntenna { claimed: &'static str, applied: &'static str }`（`FakeFault` の並びは `crates/ezsdr-radio-uhd/src/device.rs:259-331`，`WrongRate`（`:293-301`）と同じ形）: 「untimed の `apply` が `claimed` を名指したら，channel には `applied` を持たせ，read-back はそれを返す」．実装は `device.rs:1039-1041` の antenna の書き込みの隣（#59 の「read-back 拒否」と同じ，値が食い違って返る形）．

1. **`ur_25_a_channel_s_antenna_change_cuts_its_stream`** — `tests/fake.rs`．
   - 操作の列: `Direct::build(FakeConfig::default(), &[], vec![attached_on(DropOldest, "rx0")], json!({"keys": "channel", "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": []}))`（rx c0 は既定で stream 0，c1 は −1）→ T0 通過後に `direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None)` → `settle(150 ms)`．
   - assert: (i) device の calls に `apply rx 0 … antenna=TX/RX at=-` がちょうど 1 回あり，それが cut の `rx_stop 0` より後（call 列の index で見る）; `apply rx 1` は 1 回も無い（stream 0 の channel 列だけが configure される）．(ii) `finish()` の `timing` の plan 行（`core.rs:282-297`）で rx の segment が 2 つ，2 つ目の `origin - 1 つ目の end == ms(55)`（UR-25 の ready: cut + 受信 call 2 000 samples = 2 ms + 3 ms，RM-25 の lead 50 ms: `design/18-uhd-radio.md:246`，Mock の `mock_layout.rs:687-690` と同じ算術）．(iii) clock は `direct.clocks.sample_clock_records()` で rx が 2 本，1 本目が `ended_at` 付き，2 本目の `origin` が 1 本目の `ended_at` の `ms(55)` 後，`stream` は `usrp/rx/s0`（順序 3 の node 名; `design/07-radio-model.md:57`）．(iv) `finish()` の `applied` に `{"key": "radio.rx.c0.antenna", "channel": 0, "claimed": "TX/RX", "read_back": "TX/RX"}` の行がある（UR-30: `design/18-uhd-radio.md:265-266`）．(v) `direct.of("radio.COMMAND_REJECTED")` と `direct.of("radio.LATE_COMMAND")` は空（floor は無い: 床は順序 11）．
   - 観測: device calls，clock registry，`applied` / `timing` の section．
2. **`ur_25_a_transmit_antenna_change_replaces_its_clock`** — `tests/fake.rs`．
   - 操作の列: 送信に 2 つ目の antenna 名を持つ board が要る（duplex 3 種の送信 antenna は `TX/RX` だけ: `crates/ezsdr-radio/src/x3xx.rs:157-160`）ので BasicTX: `Direct::build(FakeConfig { front_ends: vec!["BasicTX"], ..Default::default() }, &[("radio.tx.c0.stream", Scalar::from(0))], vec![], json!({"keys": "channel", "rx": [], "tx": ["0:A:basic_tx:0"]}))` → `direct.update("radio.tx.c0.antenna", Value::from("A"), Cold, None)` → `settle(120 ms)`．
   - assert: `apply tx 0 … antenna=A at=-` が 1 回; tx の clock が 2 本（`usrp/tx/s0`）で 1 本目が `ended_at` 付き，2 本目の `origin >= 1 本目の ended_at`; `applied` の行（`radio.tx.c0.antenna`，claimed/read_back = A）; 拒否イベントなし．
   - 観測: device calls，clock registry，`applied`．held burst の取り消しは送信 cold 共通の道で，rate の `ur_25_a_cold_transmit_change_cancels_the_held_bursts`（`fake.rs:1638`）と Mock の `mr_40_a_transmit_move_cuts_both_clocks_and_cancels_both_held_bursts`（`design/09-mock-radio.md:391`）が既に押さえるので繰り返さない．（手の込んだ burst 付きが必要になったら session 側（`tx_session`，`fake.rs:1024`）に keys のモードの profile を作るのが順序 15 の仕事．）
3. **`ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records`** — `tests/fake.rs`．
   - 操作の列: 1 と同じ `Direct`（c0 は s0，c1 は −1）→ `direct.update("radio.rx.c1.antenna", Value::from("TX/RX"), Cold, None)` → `settle(120 ms)`．
   - assert: `apply rx 1` は 1 回も無い（device に渡さない: item 7 の「記録だけして渡さない」と同じ規則）; rx の clock は 1 本のまま（`ended_at` なし），plan の rx segment も 1 つ; `applied` に `radio.rx.c1.antenna` の行があり `read_back` は `null`（device に聞いていない）; `LATE_COMMAND` なし．
   - 観測: device calls，clock registry，`applied` / `timing`．「次にその channel が stream に入ったとき untimed に適用される」の続きは順序 10 の membership のテストで見る．
4. **`ur_25_a_refused_antenna_read_back_halts_the_stream`** — `tests/fake.rs`（`Direct`）．ハーネスの read-back 照合（`core.rs:533-560` の rate と同じ位置）と halt（`rx.rs:273-320` の refuse）を一度に踏む．
   - 操作の列: `Direct::build(FakeConfig { faults: vec![FakeFault::WrongAntenna { claimed: "TX/RX", applied: "RX2" }], ..Default::default() }, …, json!({"keys": "channel", …}))`（1 と同じ）→ `direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None)` → `settle(150 ms)`．
   - assert: (i) `direct.of("radio.COMMAND_REJECTED")` が 1 件で，reason が `"UR-12:"` で始まり `TX/RX` と `RX2` と `rx` と `c0`（方向と番号）を含む（文面は rate の `"UR-12: the device applied …"`（`core.rs:553-555`）に倣う: 例 `"UR-12: the device reads back RX2 for the claimed TX/RX antenna of rx channel 0"`）．(ii) `rx_start` の calls は最初の origin の 1 回だけ（拒否された segment は始まらない）; clock は 1 本で `ended_at` が cut（2 本目は無い）．(iii) `finish()` の `timing` の plan 行に拒否された origin の segment が無い（`ur_30_the_recorded_plan_has_no_refused_segment`（`src/provider/control.rs:877-896`）と同じ形）．(iv) `applied` の行に claimed `TX/RX` と read_back `RX2` が残る（rate の拒否と同じ順: 記録してから拒否．`core.rs:538-556`）．(v) `rejected` の section にも同じ理由がある（UR-30）．
   - 観測: event（reason），device calls，clock registry，`applied` / `timing` / `rejected`．
5. **`ur_26_a_current_keys_antenna_update_is_refused`** — `tests/fake.rs`（`Direct`，`keys` 無し＝今のモード）．
   - 操作の列: `Direct::with_links(FakeConfig::default(), &[], vec![attached(DropOldest)])` → `direct.update("radio.rx.antenna", Value::from("TX/RX"), Cold, None)`（Action を直に積むので，class の照合（`admission.rs:137-193` の step 5）は通らない）→ `settle(30 ms)`．
   - assert: `COMMAND_REJECTED` 1 件で reason が `"UR-26: radio.rx.antenna is not updated at runtime by this Provider"`（現在の文面 `control.rs:294`）; その後の `apply` が無い; clock は 1 本のまま; `timing` の plan が 1 segment のまま．加えて今のモードの `applied` の antenna の行が HEAD と同じであること: `{"key": "radio.rx.antenna", "channel": 0, "claimed": "RX2", "at": …}` で `read_back` の member を持たない（`core.rs:549-551`; 段階の「変えないこと」）．tx key（`radio.tx.antenna`）も同じ腕なので 1 件を足す．
   - 観測: event，device calls，`applied` / `timing`．

### (c) 壊れうる形と mutation の行の案

新規の行は次の空き番号（P281 以降）．`old` は実装と一緒に決める（`old` がちょうど 1 回一致すること，1b-4b の gate の教訓どおり `new` に改名後の識別子が残っていないことを確かめる）．filter は kill するテスト．既存の antenna の行（mock の P131，device.rs の P207/P213）と重ならない（`grep -n 'antenna' plan/maintenance/tools/mutations.json` で確認: UHD Provider の control/core には 1 行も無い）．

| # | 誤った実装の形 | 落ちる assert | 行の案（name / file / new の形 / filter） |
|---|---|---|---|
| 1 | antenna の cold 値が segment の設定に載らない（`ColdConfig` の投影（`control.rs:84-91`）か `Core::configure`（`core.rs:533-560`）から antenna が抜ける） | T1 の (i)(iv)，T3 の read_back | `a channel's antenna cold change is not in the segment's configuration (UR-25, D16)` / `provider/control.rs` / 該当行を落とす / `ur_25_a_channel_s_antenna_change_cuts_its_stream` |
| 2 | read-back を照合しない（`Applied.antenna` を捨てる） | T4 の (i)(ii)（拒否も halt も起きない） | `the device's antenna read-back is not compared (UR-12, D16)` / `provider/core.rs` / 比較を消す / `ur_25_a_refused_antenna_read_back_halts_the_stream` |
| 3 | 照合を claim どうしでやる（device を読まない） | 同上 | `the antenna comparison reads the claim, not the device (D16)` / `provider/core.rs` / `applied.antenna` を claim 側に置き換える / 同上 |
| 4 | read-back を記録しない（行が無い，または `read_back` が無い） | T1 の (iv)，T5 の行の pin | `the antenna read-back is not recorded (UR-30, D16)` / `provider/core.rs` / 行から読返し値を落とす / T1 |
| 5 | channel がどの stream にあるかを読まず常に stream 0 に記帳する（Mock の P131 と対） | T3（`apply rx 1` が増え，stream 0 が cut される） | `an antenna change of a channel in no stream cuts stream 0 (RM-27, RM-25)` / `provider/control.rs` / 対象 stream の選択を `stream_key(dir, 0)` に / `ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records` |
| 6 | layout の antenna key を今のモードと同じく拒否する（分岐が無い） | T1 の (i)（変更が効かない） | `a layout antenna key is refused as in the current keys (UR-26, D16)` / `provider/control.rs` / 分岐の腕を消す（else の UR-26 に落とす）/ T1 |
| 7 | 今のモードの antenna key を layout の cold として受ける（UR-26 を壊す） | T5（`COMMAND_REJECTED` が出ず `apply` が増える） | `a current-key antenna update is taken as a layout one (UR-26)` / `provider/control.rs` / モードの見分けを外す / `ur_26_a_current_keys_antenna_update_is_refused` |
| 8 | 変更を booking で即 device に適用する（segment の origin でなく；古い segment に新しい antenna が載る） | T1 の (i)（`apply` が `rx_stop` より前，または 2 回） | `the antenna change is configured at booking instead of at the segment's origin (UR-25)` / `provider/control.rs` / 即時 `configure` に置き換える / T1 |
| 9 | （任意）script した不一致が device の保持値を変えない（fault が効かない） | T4（不一致が起きず拒否が無い） | `the scripted antenna mismatch does not alter what the device holds (UR-33)` / `device.rs` / fault の腕を外す / T4 |

どの形も「弱いテストで通る」余地を残さない: T1〜T3 は device calls と clock（成立した結果），T4 は event と plan（拒否と halt），T5 は今のモードの Manifest と event（HEAD の不変）を見る．

### (d) 頼る振る舞い

- **H6**（brief の表）: antenna の getter は設定した property を読む．物理の switch を測るのではない — 設計 33 D16（`plan/phase8/33-channels-and-streams.md:200`），VERIFIED（1b-4a）．この段階の照合・記録はこの規則の上に載る: `Device::apply` は untimed のときだけ `Applied.antenna` を返す（`design/18-uhd-radio.md:96`，`crates/ezsdr-radio-uhd/src/uhd.rs:886-905`，fake は `device.rs:1042-1044`）．
- **antenna の property は streamer が開いていても書ける**（radio block の register 書き込み）— 新しい segment の configure は `rx_open`/`tx_open` の後なので，これが成り立たないと適用できない．根拠は D16 の引き（`x300_radio_control.cpp:142-148,203-206,296-312`，`db_ubx.cpp:842-871`；Sol 確認，VERIFIED）と `plan/phase8/33-channels-and-streams.md:199`．**switch がいつ完了・整定するかは頼らない**（それが antenna を `hardware_timed` でなく `cold` にした理由: 同 `:198-199`）．
- **antenna の setter/getter は channel ごと**: `uhd_usrp_set_{rx,tx}_antenna(h, ant, chan)` / `uhd_usrp_get_{rx,tx}_antenna`（`crates/ezsdr-radio-uhd/src/uhd.rs:182-183,194-195`；UHD 4.10 の `usrp.h`，VERIFIED）— per-channel の configure が正しい根拠．
- 表の H1〜H5・H7 には頼らない: H1（spec の変更が streamer を切る）は antenna の変更が subdev spec を触らないので無関係; H2・H5 は subdev spec と front end 名（順序 2・3）; H3 は rate（順序 4）; H7 は空 spec・重複（順序 2）．

### 関わる他の順序

- **1（`keys` の値）**: `keys: "channel"` の分岐と `Direct::build` の selector を共有する．「layout のモードは今の key を prepare で受けない」の逆（今のモードは layout の `radio.rx.c0.antenna` を受けない）を pin するのは 1 のテスト側（T1〜T4 は keys のモードだけを使う）．
- **2・3（subdev spec と front end）**: antenna の変更は spec を触らないので依存しない．ただし T1/T2 の足場が使う `channels::from_selector`（RM-28）と front end の照合は 2・3 の実装を前提にする．
- **6（`prepare` の stream 0）**: `Core::settings`/`configure` の channel ごと化，`rx_open(0, 列)`，port `rx0` が antenna を device に届ける土台．T1/T2/T3 の `apply …` の形（`at=-` の untimed）と「stream 0 の channel 列だけを apply する」は 6 の assert と同じ道具．
- **7（channel ごとの timed）**: `ColdConfig::settings`（`control.rs:84-91`）の投影に antenna を足す．「走っている stream に入っていない channel の更新は記録だけ」という item 7 の 2 番目の規則は T3 と対（T3 は antenna 側）．timed の周波数/利得と cold の antenna が同一 segment で混ざる場合の投影順は 7 の持ち分．
- **8（rate の cold）**: cut・新しい SampleClock・ready（cut + call + allowance + lead）の機構を共有する．T1 の算術（原点 = cut + ms(55)）は 8 のテストと同じ数字を使うが，8 は rate の格子と `call` の term を，9 は antenna の適用・照合・記録を見る．テストは重複させない．
- **10（membership）**: channel が stream に入る/出る瞬間と antenna の適用は続いている（T3 の「値だけ」の続きは「次にその channel が入る segment の configure で untimed に適用」）。`−1 → 0`・`0 → −1` の操作そのものと，「入った後の segment で antenna が載る」は 10 のテストで見る．`→ 1` 以上はこの段階では拒否（T1〜T4 の layout は stream 0 だけを使う）．
- **11（instance 全体の床）**: antenna の cold 変更も床を受ける（Mock の `mr_18_a_layout_cold_floor_crosses_directions: mock_layout.rs:1041-1058` は一方に antenna を使う）．T1/T3 は先に pending の cold が無い場面で instant を読む（床を混ぜない）．11 のテストの 2 つ目の変更に antenna を使うのは自然（Mock と同じ操作）．
- **12（宛先）**: T1〜T5 は `<id>` 宛ての `UpdateParameter`．`<id>/rx/s0` 宛てでも同じに動くことの pin は 12 のテスト側．T4 の `COMMAND_REJECTED` の source が device node（RM-11: `design/07-radio-model.md:139`）であることは 12 の event source と整合する．
- **13（実行時の判定）**: RM-31（同じ board の送信 channel が stream にある間，受信 antenna は `RX2` だけ: `design/07-radio-model.md:344`）を T1〜T4 の配置は踏まない（送信 channel を stream に入れない）．「`RX2` を離れる受信 antenna の更新が RM-31 で拒否される」テストは 13 の持ち分（Mock の `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime: mock_layout.rs:832-890` と対）．
- **14（Manifest）**: T1/T3/T4 が読む `applied` の行（UR-30）と `timing` の plan は，14 が定める section の形（`device` section の追加以外は 14 の持ち分）に従う．keys のモードの行の key の綴り（`radio.rx.c0.antenna`）は 6 と 14 で 1 つに揃える．
- **15（VH-8）**: 同じ antenna cold の列を Mock と UhdRadio（FakeDevice）に与え，`effective`，stream 0 の SampleClock の宣言（node），cut と新しい origin，拒否の理由の種別が一致することを見る．Mock 側の基準は `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream`（`mock_layout.rs:662`）．ジェネレータの `Change`（`crates/ezsdr-radio/tests/generator/mod.rs:45-50`）は今 `Channels`/`Rate` だけで antenna の項が無いので，VH-8 に antenna を入れるなら 15 の側で足す（この段階のジェネレータの変更は 15 の持ち分）．

## 10. membership の変更

keys のモード（1b-4c）で，`radio.{rx,tx}.c<n>.stream` の −1 → 0（入る）と 0 → −1（出る）を扱う順序．どちらも「その channel が入る／出ていた stream」＝ stream 0 の 1 本の line への `cold` 変更として記帳され（`design/09-mock-radio.md:154`「entry or exit is booked on its one line」，`plan/phase8/33-channels-and-streams.md:85` D3），segment が切れ，新しい channel の列で streamer が開き直り，新しい SampleClock が始まる．この段階は stream 0 だけを動かすので `→ 1` 以上は実行時に拒否する（`plan/phase8/33-channels-and-streams.md:21` の切り分け）．

### (a) 既存のテスト

いちばん近いもの．**今のモード（prefix の key）で同じ機構を通る**テストを含む（membership の実装はこの道を keys のモードへ写す）:

- `crates/ezsdr-radio-uhd/tests/fake.rs:3815` `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one` — 送信の「channel が 0 になる」＝今の key での「最後の channel が出る」．cut で古い clock が終わり（`:3840-3845`），再 enable が新しい clock を始め（`:3848`），`tx_open 0 []` を開かない（`:3850`）ことを固定している．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1516` `ur_25_rx_channels_from_zero_starts_at_its_instant` — 受信の「空の stream に channel が入る」．`rx_start 0 <origin>` が instant 以後で，SampleClock が最初の block で `usrp/rx` に登録される（`:1524-1529`）．keys のモードでは stream id が `usrp/rx/s0` になる（順序 3・14）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1535` `ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` — 受信 segment の origin が「configuration の終わり + call + allowance + lead」の次の格子になること（`:1549-1554`）．join/leave の新しい origin の assert は同じ規則の上に載る．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1462` `ur_25_a_stream_silent_before_e1_switches_before_e2` — cut で `rx_stop 0 now` が先，`rx_start 0 <e2>` が後（`:1491-1493`）．「stop < open < start」の形．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1559` `ur_25_a_cold_change_the_envelope_refuses_changes_nothing` — 拒否された `cold` 変更が device call を増やさない（`:1571`）．「何も変わらない」の形．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1500` `ur_25_tx_channels_from_zero_transmits_the_next_burst_on_time`・`:3856` `ur_25_an_enable_from_no_stream_configures_once` — enable が `tx_open 0 [0]` と `apply tx 0 …` を 1 回ずつで，configure を 2 度しない（from-none，UR-25）．開き直しが 1 回であることの形．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1711` `ur_25_a_receive_session_follows_its_recorded_plan` — `timing` の `plan` の segment と `clocks.sample_clocks` の対応，`rx_open` が 1 本であること（`:1737-1759`）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1638` `ur_25_a_cold_transmit_change_cancels_the_held_bursts` — `cold` の送信変更が held burst を取り消す（UR-23）．membership の変更も同じ道を通る（順序 8 の担当だが，送信のテストがこれを壊さないこと）．

lib（白箱，今のモード）:

- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1266` `ur_25_a_refusal_leaves_the_transmit_stream_alone`・`crates/ezsdr-radio-uhd/src/provider/tx.rs:1396` `ur_25_a_refusal_leaves_the_receive_stream_alone` — 拒否が他方の stream の items・plan・clock を動かさない．「何も変わらない」の白箱の形．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1294`・`crates/ezsdr-radio-uhd/src/provider/tx.rs:1443` `ur_25_the_line_lock_is_free_inside_every_{receive,transmit}_device_call` — 開き直し・configure・start の device call の間に line の lock を持たない（UR-25，D15）．新しい開き直しの道も同じ規則の下に置く．
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:1078` `ur_25_a_second_change_follows_the_first`・`:1110` `ur_25_a_refused_segment_is_booked_by_uhd_rx` — segment の切替と拒否の記帳の形．
- `crates/ezsdr-radio-uhd/src/provider/mod.rs:598` `ur_15_a_second_stream_is_refused_rather_than_half_started` — この段階が方向ごとに stream 0 だけを動かすことを固定する（`:481-482` の guard）．`→ 1` 以上の拒否と同じ「段階の制限」の規則．

共有の規則（`ezsdr_radio`，動かさない）:

- `crates/ezsdr-radio/tests/timeline.rs:430` `rm_21_a_cold_change_to_another_set_of_the_same_size_starts_a_segment` — 同じ大きさの別の集合は別の segment（RM-21，RM-25）．開き直しを「数が変わったとき」とだけ読むことの上位の規則．
- `crates/ezsdr-radio/tests/timeline.rs:449` `rm_21_a_stream_that_loses_its_last_channel_starts_again_when_one_enters` — 最後の channel が抜けたら segment なし，次に入ったら lead 後に始まる．空と再入の timeline 側の固定（`crates/ezsdr-radio/src/timeline.rs:854-869` の `Kind::Cold`，`:923-942` の `begin`）．
- `crates/ezsdr-radio/tests/channels.rs:481` `rm_30_coerce_cases`（`stream` −1・2・3 の判定）・`:693` `rm_32_throughput_cases`（membership だけの変更が RM-32 で拒否され，その key を名指す，`:838-860`）— `coerce`/`violations` は 1b-4c でも同じ関数（順序 13）．

Mock（VH-8 の参照実装，`plan/phase8/33-channels-and-streams.md:273`）:

- `crates/ezsdr-mock-radio/tests/mock_layout.rs:662` `mr_18_a_layout_instance_s_changes_belong_to_their_own_stream` — 同じ順序の参照：channel の出入りがその stream だけを切り，`clocks_of(...)` が 2 本目を登録し，空になった stream は segment を持たない（`:703-710`）．実装（`crates/ezsdr-mock-radio/src/lib.rs:2007-2050` の `cold_streams`）は 1b-4c で `ezsdr_radio` へ移し，両 Provider が呼ぶ（`plan/phase8/reviews/1b3b-report.md:215`）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:733` `mr_09_each_transmit_stream_has_its_own_clock_and_line` — 送信 stream の clock が cut + lead 後に替わる形．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:999` `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change` — membership の変更が instance の床に載る形（順序 11）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:601` `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` — D14 の宛先（順序 12）．

### (b) 足すテスト

置き場は `crates/ezsdr-radio-uhd/tests/fake.rs`（`Direct` ハーネス）．先にハーネスを 3 点だけ広げる（挙動は変えない）:

- `attached`（`fake.rs:570-581`）は port `rx` を直に書いている．keys のモードの port は `rx0`（D4）なので，port 名を受ける `attached_port("rx0", policy)` にする．さもないと `prepare` の link の検査に届かない．
- `Direct::build`（`fake.rs:619-663`）は selector を引数で受けるので，`json!({"keys": "channel", "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": []})` を渡す（`binding` の既定 rx/tx は明示で上書きされる．`fake.rs:82-93`）．FakeDevice の既定は UBX 2 本（`crates/ezsdr-radio-uhd/src/device.rs:376`）なのでこれで足りる．
- event の pair は `usrp`・`usrp/rx`・`usrp/tx` だけ（`fake.rs:627`）．順序 3 で stream の node が `usrp/rx/s0`・`usrp/tx/s0` になるので，その pair を足す（足さないとその node の event が collector に届かず，「0 件」の assert が偽陰性になる）．

各テスト（名前，列，assert，観測）．`settle` は manual clock なので，device の時間は thread が idle の間だけ進む（`crates/ezsdr-radio-uhd/tests/README.md:18-25`）．`at` は lead より先に置き，床（順序 11）を当てにしない．

1. **`ur_25_a_receive_membership_join_opens_the_streamer_with_the_new_set`**（`tests/fake.rs`）
   - 構成：constraints `[("radio.rx.c0.frequency_hz", 2.0e9), ("radio.rx.c1.frequency_hz", 1.1e9)]`，rx layout 2 channel，tx `[]`，link は `attached_port("rx0", DropOldest)`．既定の membership は rx c0 が stream 0，c1 は −1（`crates/ezsdr-radio/src/channels.rs:524-552`，`default_stream` `:965-967`）．arm で `rx_open 0 [0]`（`provider/mod.rs:427`），start で `rx_start 0 T0`．
   - 列：最初の block で clock が登録されるまで settle → `at = now + ms(300)` で `direct.update("radio.rx.c1.stream", Value::from(0), Cold, Some(at))` → `settle(ms(700))`．
   - assert：`rx_stop 0 now`（`rx.rs:606-609`，`device.rs:1121-1127`）が 1 回増え，`rx_open 0 [0, 1]` がちょうど 1 回，その後に `rx_start 0 <origin>`（順は stop < open < start，`fake.rs:1491-1493` の形）．`set_subdev_spec` と `rx_open 0 []` は無い．新しい segment の untimed の `apply` は channel ごとの値：`apply rx 0 … freq=2000000000 antenna=RX2 at=-` と `apply rx 1 … freq=1100000000 … at=-`（`control.rs:97-102` → `core.rs:533-556`）．`direct.clocks.sample_clock_records()` の `usrp/rx/s0` は 2 本：1 本目は `ended_at` があり，2 本目は `origin == rx_start` の引数，`origin >= at + lead`（lead は envelope から，`fake.rs:3846-3848` の形）．`finish()` の `timing` の `plan.rx` は 2 segment で 2 つ目の `channels: 2`（`core.rs:282-297`），`rx_segment` 2 行．`COMMAND_REJECTED`・`LATE_COMMAND` は 0 件．
   - 観測：device の calls，`timing` の `plan`・`rx_segment`・`cold_change`，clock の record，event．**集合そのものは `rx_open` の列で見る**（`plan` の行はまだ数しか持たない．stream の識別と列は 1b-4d，`plan/phase8/33-channels-and-streams.md:22` の (9)）．
2. **`ur_25_a_receive_membership_leave_opens_the_streamer_for_the_remaining_set`**
   - 構成：constraints に `("radio.rx.c1.stream", 0)` を足し，stream 0 = {0, 1}（arm は `rx_open 0 [0, 1]`）．
   - 列：`direct.update("radio.rx.c0.stream", Value::from(-1), Cold, Some(now + ms(300)))` → settle．
   - assert：`rx_open 0 [1]` がちょうど 1 回（`[0]`・`[0, 1]` ではない）．新しい segment の `apply` は channel 1 だけ（前後で `apply rx 0 … at=-` の数を比べる．prepare の configure が既に channel 0 を 1 回通っているため，`calls.len()` を控えてから数える）．`plan.rx` の 2 つ目の `channels: 1`．clock 2 本（1 本目は cut で終了）．`COMMAND_REJECTED` 0 件．
   - 観測：device の calls，clock の record，`timing` の `plan`．prefix 読み（c の 1）を殺す中心のテスト．
3. **`ur_25_the_last_receive_channel_leaves_no_clock_and_no_block`**
   - 構成：rx 2 channel，既定（stream 0 = {0}）．
   - 列：`direct.update("radio.rx.c0.stream", Value::from(-1), Cold, None)` → settle（leave が終わり uhd-rx が idle になるまで）．
   - assert：`rx_stop 0 now` が 1 回．その後 `rx_open`・`rx_start` は増えない（cnt を前後で比べる）．clock は 1 本のままで `ended_at == <cut の instant>`（`rx.rs:331-335`）．`finish()` の `plan.rx` は 1 segment で `end` があり，2 つ目が無い（`timeline.rs:854-869` の空の集合）．`rx_open 0 []` は無い．`COMMAND_REJECTED` 0 件．「block も無い」は clock が 2 本目を登録しないことと plan に segment が無いことで見る．
   - 観測：device の calls，clock の record，`timing` の `plan`．
4. **`ur_25_a_join_after_an_empty_stream_opens_its_own_set`**
   - 構成・前半：3 と同じ（既定 {0} → c0 が抜けて空）．後半：c0 が抜けたまま `direct.update("radio.rx.c1.stream", Value::from(0), Cold, None)`（入る channel は 0 ではなく 1）．
   - assert：`rx_open 0 [1]` がちょうど 1 回（prefix の `[0]` ではない．2 回でもない）．`rx_start 0 <origin2>` が 2 回目で，`origin2 >= <1 本目の cut> + lead`．clock は 2 本目（`origin == origin2`，`ended_at` 無し）．`apply rx 1 … at=-` がある．どちらの thread が開くか（from-none の uhd-control か uhd-rx の begin か）は問わない—assert は同じ列になる（`control.rs:554-579`，`rx.rs:283-301`）．
   - 観測：device の calls，clock の record．
5. **`ur_25_a_transmit_membership_change_opens_the_new_set_and_starts_its_clock`**
   - 構成：tx 2 channel，constraints `[("radio.tx.c0.stream", 0), ("radio.tx.c1.stream", 0)]`（stream 0 = {0, 1}）．link は無し（`Direct::new` の形）．arm は `tx_open 0 [0, 1]` と clock 1 本目（`mod.rs:429-440`）．
   - 列：`direct.update("radio.tx.c0.stream", Value::from(-1), Cold, Some(now + ms(300)))` → `settle`（2 本目の segment の origin を越えるまで）→ `direct.update("radio.tx.c1.stream", Value::from(-1), Cold, None)`（空）→ settle．
   - assert（前半）：`tx_open 0 [1]` が 1 回（`tx.rs:660-661`），`apply tx 1 … antenna=TX/RX at=-` が 1 回，clock 2 本（2 本目の `origin >= <1 本目の cut> + tx lead`，`core.rs:258-279`），`timing` に `tx_switch`（`tx.rs:682`）．前半の asser（後半）：clock は 2 本のまま（3 本目を登録しない），2 本目が cut で終了，`tx_open 0 []` は無い（`fake.rs:3815` の送信 0 の keys 版），`COMMAND_REJECTED` 0 件．
   - 観測：device の calls，clock の record，`timing` の `plan`・`tx_switch`．
6. **`ur_25_a_membership_in_a_later_stream_is_refused_and_changes_nothing`**
   - 構成：rx 2 channel・tx 2 channel．`for (key, target) in [("radio.rx.c1.stream", Value::from(1)), ("radio.tx.c1.stream", Value::from(1))]` で回す（coerce の範囲は −1…=1 なので，値 1 は `coerce` を通り `crates/ezsdr-radio/src/channels.rs:1006-1013`，Provider の段階の検査に来る）．
   - 前：`calls.len()` と `clocks.sample_clock_records()` を控える．
   - 列：`direct.update(key, target, Cold, None)` → settle．
   - assert：`radio.COMMAND_REJECTED` がちょうど 1 件，`kind == "update_parameter"`，`reason` がこの段階の「stream 0 だけを動かす」ことを述べ，値の stream 番号（`s1`）を名指す（文言はコードと一緒に決める．規則番号の接頭辞もそのときに）．device の calls は 1 つも増えない（`apply`・`rx_open`・`rx_stop`・`tx_open`・`set_subdev_spec`・`rates`）．clock の record は前と同じ（個数・origin・`ended_at`）．`LATE_COMMAND` は 0 件．`finish()` の `timing` に `cold_change`・`rx_segment`・`tx_switch` が増えず，`plan` の (origin, end) も同じ．`rejected` section に 1 行増える．
   - 後半（同じテスト）：「拒否が値を書いた」を殺すため，同じ channel の `…c1.stream` を 0 にして join が成功すること（`rx_open 0 [0, 1]` が 1 回）を見る（値 1 が残っていれば move と読まれ，また拒否される）．
   - 観測：event，device の calls，clock の record，`timing` の `plan`，`rejected` section．

### (c) 壊れうる形と mutation の行の案

`old` の正確な文字列はコードと一緒に決める．`crate: ezsdr-radio-uhd`，`test_bin: "fake"`，`filter` は下のテスト名（T1〜T6 は (b) の番号）．

| # | 壊す規則と形 | 殺す assert |
|---|---|---|
| 1 | **開き直しの列を `0..数` の prefix で作る**（`Rx::begin` の reopen と uhd-control の from-none の open，`Tx::do_switch` の reopen の 3 か所が同じ形の誤りをしうる）．`old` を「集合の列」を作る式，`new` を `(0..segment.config.channels.count())` の列に | T2 の `rx_open 0 [1]`（`[0]` になる），T4 の `rx_open 0 [1]`，T5 の `tx_open 0 [1]` |
| 2 | **空の stream にも開く**（`from_none` の `channels > 0`（`control.rs:557`）を外す，または空でも `rx_open`/`tx_open` する） | T3 の「`rx_open` が増えない・`rx_open 0 []` が無い」，T5 後半の `tx_open 0 []` が無い |
| 3 | **−1 → 0（入る）を移動として扱う**（`was < 0` の腕（`lib.rs:2040-2041` の写し）を落とし `ColdChange::Move { from: 0, to: 0 }` にする→ `SameStream` で拒否） | T1 の `rx_open 0 [0, 1]`（拒否されて開き直しが起きない） |
| 4 | **0 → −1（出る）がどの stream も名指さない**（`is < 0` の腕を落とす） | T2 の `rx_open 0 [1]` と，1 本目の clock の `ended_at` |
| 5 | **新しい segment の configure を前の集合のまま行う**（`ColdConfig::configure` の列を segment のものにしない） | T2 の `apply rx 1 … at=-`（数），T1 の `apply rx 0/1` |
| 6 | **channel ごとの値を使わず 1 つの `Settings` を全 channel に渡す**（`Core::configure` の今の形のまま） | T1 の `apply rx 1 … freq=1100000000`（順序 7 と共有の行．どちらかが持てばよい） |
| 7 | **`→1` 以上の拒否を落とす**（段階の検査を外す／`booked_stream(0)` に丸める） | T6 の `COMMAND_REJECTED` 1 件・device call 0・plan 不変のどれか（丸めれば `cold_change` と開き直しが現れ，外せば stream 1 の line で panic して `DEVICE_LOST` になる） |
| 8 | **拒否の後に値を書く**（`colds`/`config` に載せる） | T6 後半の join（値 1 が残れば move と読まれ拒否される） |

**行を起こさない（equivalent）と記録するもの**: 「開き直しを前に開いた channel 数と比べる」形（`self.opened` を数で持つ，`rx.rs:286`・`tx.rs:660` の比較）．1b-4c では単一 key の membership 変更が必ず数を変えるため，**同数の別集合が作れず**観測できない（T4 の再入は `idle` なら from-none が開くので，owner 側の比較そのものは通らないことがある）．行は移動（RM-34，同数の別集合）が入る **1b-4e** で作る．同様に「`-1 → 0` と `0 → -1` で対象を選ぶ」判定は，この段階ではどちらも stream 0 なので，値の読み違いだけが 7 で残る．

### (d) 頼る振る舞い

上の表 H1〜H7 のうち，**この順序が観測の根拠にするものは無い**（subdev spec は開いたときに設定済みで，membership は触らない．`rates` も呼ばない — UR-13 の照合は順序 4（arm）の担当）．代わりに次に頼る（表に無いので根拠と確かさを付ける）:

1. **streamer の channel 列の開き直しは，同じ stream の前の streamer を置き換える**（同じ channel を 2 つの streamer が持たない）．根拠：`crates/ezsdr-radio-uhd/src/uhd.rs:909-930`（`rx_open` が古い streamer を先に drop する，`:916-918`），`design/18-uhd-radio.md:278`（streamer ごとの channel 列と，graph の「2 つの streamer は同じ channel を持てない」`graph.cpp:179-197`）．**VERIFIED**（spec 18 の引用と `UhdDevice` の実装による）．T1・T2・T5 の「stop < open < start」と，開き直しが 1 回である assert はこの上に載る．
2. **空の channel 列は UHD では channel 0 を意味する**（`sanitize_stream_args`）．根拠：`crates/ezsdr-radio-uhd/src/uhd.rs:911-913`，`design/18-uhd-radio.md:278`，1b-4a の `FakeDevice`（`device.rs:1047-1050`）も `[]` を channel 0 とする．**VERIFIED**．だから「channel を持たない stream は streamer を開かない」は Provider の責任で，T3・T5 の「`rx_open 0 []`／`tx_open 0 []` が無い」はこれが破られたら落ちる．
3. **その装置の channel map の中の任意の部分集合で streamer を開ける**（開き直しの列は論理 channel の番号＝ device の channel の番号，順序 2）．根拠：`design/18-uhd-radio.md:278`（各 streamer が自分の channel 列を持つ），`plan/phase8/33-channels-and-streams.md:54`（device の channel の番号＝論理 channel の番号）．**VERIFIED**（spec の引用による）．
4. **集合の列は小さい順で，block の channel `i` は集合の `i` 番目**（`crates/ezsdr-radio/src/timeline.rs:61-64` の `Channels::iter`，`design/07-radio-model.md:57` RM-3/RM-29）．T2 の `[1]` のような「prefix でない列」の意味はこれで決まる．**VERIFIED**（リポジトリ内の規則）．
5. 受信 streamer の untimed stop のあとも，進行中の packet は届く（その stream の tail；新しい segment の block はその origin 以後）．根拠：`crates/ezsdr-radio-uhd/src/device.rs` の `RxStreamer`（`end`/`then`）と bench の「stop はその時刻を無視して即止まる」測定（`design/18-uhd-radio.md:29`）．**INFERRED**（bench B8）．assert はこれに依存しない（settle の長さの前提にだけ関わる）．

### 関わる他の順序

- **6（`prepare`／`arm` の stream 0）**: T1・T2・T5 は arm が正しい列（`[0]`・`[0, 1]`）で開いていることを前提にする．`→1` 以上の `prepare` の拒否は 6 の担当，T6 は実行時の拒否だけを見る．
- **8（rate の `cold`）**: 同じ cut・開き直し・新しい clock の道（`mod.rs:509-588`）。T1〜T5 は rate を変えないが，新しい segment の configure は同じ `ColdConfig` を通る．8 の既存テスト（`fake.rs:3815` ほか）を壊さないこと．
- **11（instance 全体の床，RM-25）**: membership の effective instant も床に載る（Mock の形は `mock_layout.rs:999`）．T1〜T6 は `at` を lead より先に置くか `at` 無しの単発で，床を当てにしない（2 つ以上の cold が飛ぶ列は 11 の担当）．
- **12（D14 の宛先）**: `UpdateParameter` を `<id>/rx/s0`・`<id>/tx/s0` 宛てでも受けるのは 12 の担当．T1〜T6 は root（`radio`）宛てで送る（`Direct::update` のまま）．
- **13（実行時の `violations`）**: membership の変更が RM-31（全二重 RX2）・RM-32（throughput）に触れる場合の `COMMAND_REJECTED` は 13 の担当．T1・T2 の構成（UBX 2 本の受信だけ，送信なし）は違反しない．
- **14（Manifest）**: `device` section と plan の stream の識別・論理 channel 列は 14（と 1b-4d）の担当．T1〜T6 が見るのは `timing` の `plan`，`applied`/`rejected`，clock の record まで．
- **15（VH-8）**: 同じ列と Spec を Mock と UHD（FakeDevice）に与えて `effective`・stream 0 の SampleClock・拒否を比べるのは 15．T1〜T5 は UHD の側だけを固定する．
- **1b-4d・1b-4e へ送るもの**: 複数 stream の VH-8 harness と stream ごとの thread・node（4d）．RM-34 の移動（同数の別集合）と，「開き直しを channel 数だけで比べる」形の mutation の行（4e）．また `plan/phase8/reviews/1b3b-report.md:212` の「要求された instant の設定で `cold` の対象 stream を選ぶ」不整合は 1b-3c の範囲のままで，T1〜T6 は単一の更新なので踏まない（踏むのは 2 つ以上の cold が飛ぶ列で，11 の担当）．

## 11. instance 全体の床（RM-25）

この項目で実装されること（計画の前提）：

- `keys: "channel"` のモードの `cold` 変更の effective instant を，**その instance が先に受けたまだ効いていない `cold` 変更すべて**の effective instant で床にする（方向・stream・line を問わない）．今 MockRadio の `handle_update` に inline にある計算（`crates/ezsdr-mock-radio/src/lib.rs:1892-1905`，床の行は `:1899`，late の判定は `:1902` の `if asked < now || e > asked {`）を `ezsdr_radio::timeline` の1関数に移し，両 Provider がそれを呼ぶ（`mech:radio-rule-implemented-twice`）．MockRadio の挙動は変えない．
- UHD Module の `keys` のモードの `book_cold`（`crates/ezsdr-radio-uhd/src/provider/control.rs:509-588`）がその関数を使う．今の key のモードの床は自分の stream の line のまま（`timeline::command_instant`，`crates/ezsdr-radio/src/timeline.rs:380-389`）変えない．
- 床の対象になる先着は「まだ効いていない `cold` 変更」で，Mock は `self.updates`（効いた時点で `:2062` で消える），UHD は `self.colds`（`control.rs:148`，`e` は床込みで入る `:550,:583`）が持つ．後着の `e` は「先着の effective instant の最大」で床上げされる（`now` より前のものは `max(now, …)` に吸収される）．

---

### (a) 既存のテスト

**MockRadio の layout モード — 抽出が挙動を1つも変えないことの見張り（1行も変えずに緑のまま）**

- `crates/ezsdr-mock-radio/tests/mock_layout.rs:999` `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change` — 同じ方向の先着への床，`LATE_COMMAND` の key，`applied` の instant，clock の cut と再開．行 P141 の killer．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1041` `mr_18_a_layout_cold_floor_crosses_directions` — 方向をまたぐ床（rx antenna `+20 ms` の後に受けた tx membership `+10 ms` が `+20 ms` に床上げされ，RM-31 で拒否）．**この項目の操作（tx rate → rx antenna）の逆向きの既存形**．行 P142 の killer．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1061` `mr_18_a_layout_reservations_cannot_invalidate_an_earlier_one` — 先着の予約を後着が追い越せない（行 P143 の killer）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1106` `mr_18_a_layout_cold_floor_holds_without_a_line` — line のない stream の先着でも床になる（床が line のものではない）．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1132` `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` — 床は `cold` だけのもの（timed は床に入らない）．行 P148 の killer．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:2524` `mr_18_a_layout_receive_floor_at_t0_emits_late_command` — 受信 stream の start の床と，late の向き（`e > asked` で立つ）．行 P163 の killer．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:1316` `mr_40_a_move_uses_the_instance_floor_across_streams_directions_and_missing_links` — move の経路も同じ inline の床を通る（`:1899` の1行を共有）．行 P149 の killer．move 本体は 1b-4e だが，抽出でこの行が動くため見張りに含める．

**UHD Module の今のモード — 変えないことの見張り**

- `crates/ezsdr-radio-uhd/src/provider/control.rs:899` `ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it` — 同じ stream の先着への床，`LATE_COMMAND` の requested/applied，plan．行 J17 の killer．**今のモードの床が自分の line だけであることを固定する既存の唯一のテスト**．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2955` `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time` — 今のモードの `cold` で `late.is_empty()`．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1389` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2` — 今のモードの切替の隣接．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1327` `ur_24_hardware_timed_updates` — timed が自分の instant を保つことの隣接（床が timed に広がらないこと）．

**timeline の line の床 — 別機構（抽出で触らない）**

- `crates/ezsdr-radio/tests/timeline.rs:385` `rm_26_pruning_folds_a_cold_change_floored_at_an_earlier_one`
- `crates/ezsdr-radio/tests/timeline.rs:488` `rm_34_a_move_is_floored_at_a_cold_change_of_either_line`

検索（見つからないことの根拠）：

- UHD Module に `keys` のモードのテストは今1本も無い：`grep -rn "\"keys\"" crates/ezsdr-radio-uhd/tests crates/ezsdr-radio-uhd/src` は一致 0（exit 1）．layout モードの UHD 側テストはすべて 1b-4c で新規．
- 今のモードで両方向の `cold` を1つのテストで受けるものは無い：`grep -n ", Cold," crates/ezsdr-radio-uhd/tests/fake.rs` の一致（:1396, :1477, :1521, :1543, :1564, :1583, :2854, :2967, :3885, :3917）は全部 `radio.rx.…`．tx の `book_cold` は unit（`control.rs` の stop の隣接）だけ．
- `command_instant` を壊す既存の mutation 行は無い：`grep -rn "command_instant" plan/*/tools/mutations.json plan/maintenance/tools/mutations.json` は一致 0．

---

### (b) 足すテスト

**T1（unit，`crates/ezsdr-radio/tests/timeline.rs`）: `rm_25_the_layout_instant_is_floored_at_every_earlier_cold_change`**

- 狙い：新関数そのものの振る舞いを，Provider 抜きで固定する．関数の引数の持ち方（先着の列を slice/iterator で受けて max を内側で取るか，max 済みの `Option<i64>` で受けるか）は実装で決める．下の assert は振る舞いで書く（例：`pub fn layout_instant(at: Option<i64>, now: i64, start: Option<i64>, earlier: impl IntoIterator<Item = i64>) -> (i64, bool)`）．
- 通す操作の列：関数を下の表の各ケースで呼ぶだけ．渡す「先着の instant」は**effective instant**（Mock の `updates` の key と UHD の `colds` の key は床込みの `e` を持つ，`lib.rs:2001`，`control.rs:550,583`）であること．
- assert（`(e, late)` の組）：

| at | now | start | earlier | e | late |
|---|---|---|---|---|---|
| Some(10) | 0 | None | [] | 10 | false |
| Some(10) | 0 | None | [50] | 50 | true |
| None | 5 | None | [50] | 50 | true |
| Some(10) | 20 | None | [] | 20 | true |
| Some(10) | 0 | Some(30) | [] | 30 | true |
| Some(30) | 0 | Some(30) | [] | 30 | false |
| Some(10) | 0 | None | [50, 20] | 50 | true |
| Some(50) | 0 | None | [10] | 50 | false |

  行2が brief の期待（床が動かすと late），行7が「方向・stream を問わない最大」，行5・6が受信 start の床と late の向き（Mock `:2524` と同じ規則），行8が床は要求を早めないこと．
- 観測：戻り値のみ．
- 備考：`design/07-radio-model.md:295`（RM-26 の「`ezsdr_radio::timeline` computes … RM-25 once … both Providers take their cuts, origins and clock registrations from it」）は，この関数がまさにその1か所になることを要求している．

**T2（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）: `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance`**

- 狙い：brief item 11 の期待そのもの — keys のモードで，送信の rate の `cold` の後に受けた受信の antenna の `cold` が `T+50 ms` に効き，`LATE_COMMAND`（requested `T+10 ms`，applied `T+50 ms`）が出ること．
- 形（`Direct`，`fake.rs:619` の `build` に selector を渡す）：

```rust
use ezsdr_kernel::module_api::UpdateClass::Cold;
let mut direct = Direct::build(
    FakeConfig::default(),
    &[("radio.tx.c0.stream", Scalar::from(0))],
    vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)],
    json!({ "keys": "channel", "rx": ["0:A:ubx:0"], "tx": ["0:B:ubx:0"] }),
);
let now = direct.now();
direct.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(50)));
direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, Some(now + ms(10)));
direct.settle(Wall::from_millis(200));
```

- layout の選び方の理由：rx を slot A，tx を slot B にする（RM-31 は「同じ timekeeper と slot」の board にだけ効くので，rx c0 の antenna を `TX/RX` に変えても違反しない）．rx c0 は既定で stream 0，tx c0 は constraint で stream 0．antenna の既定は board の最初の名前（rx `RX2`，`crates/ezsdr-radio/src/x3xx.rs:298-319`，`crates/ezsdr-radio/src/channels.rs:860-874`）なので `TX/RX` は実変更．
- assert：
  - `direct.of("radio.LATE_COMMAND")` が1件，`payload["key"] == "radio.rx.c0.antenna"`，`requested["ticks"] == now + ms(10)`，`applied["ticks"] == now + ms(50)`．
  - `direct.of("radio.COMMAND_REJECTED")` が空．
  - `finish()` の `timing` の `cold_change` 行がキーで引いて2本，どちらも `e == now + ms(50)`．
  - `direct.clocks.sample_clock_records()`：`usrp/rx/s0` は2本で `[0].ended_at == now + ms(50)`（1 Msps・200 tick/sample，`now − T0 = 1 ms` なので sample 境界），`[1].origin == now + ms(105)`（cut + receive call 2 000 サンプル = 2 ms + allowance 3 ms + rx start lead 50 ms；式は `timeline.rs:917-939` と `control.rs:195-204`）．`usrp/tx/s0` は2本で `[0].ended_at == now + ms(50)`，`[1].origin == now + ms(100)`（cut + tx start lead）．
  - device calls：`calls(&direct.device, "apply rx 0")` に `antenna=TX/RX at=-` が1件（新しい segment の configure），`calls(&direct.device, "apply tx 0")` に `rate=2000000` が1件．
- 観測：event（LATE_COMMAND）・Manifest `timing`・`ClockRegistry` の records・device calls．
- 備考：call 項が packet で変わる環境では `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`（`fake.rs:2955`）と同じ式（`block_len.max(packet) * 200 + ms(3) + ms(50)`）で書く．

**T3（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）: `ur_25_a_layout_cold_floor_holds_without_a_line`**

- 狙い：床は line のものではなく instance のもの（Mock `:1106` の UHD 版）．line を持たない受信側の先着 `cold` でも後着が床上げされる．
- 形：`Direct::build(FakeConfig::default(), &[], vec![], json!({ "keys": "channel", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"] }))`（link なし）．`direct.update("radio.rx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(50)))`（line なし → 値だけ，`control.rs:548-551`），続けて `direct.update("radio.tx.s0.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(10)))`．
- assert：`LATE_COMMAND` が1件（key `radio.tx.s0.sample_rate_hz`，requested `now+10ms`，applied `now+50ms`）；`timing` の `cold_change`（tx）の `e == now + ms(50)`．
- 観測：event と Manifest `timing`．rx の変更自体は Manifest に出ない（no-line 分岐は `timing` を書かない）ので，観測は後着の床で行う．

**T4（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）: `ur_25_a_layout_hardware_timed_change_keeps_its_own_instant`**

- 狙い：床は `cold` だけのもの（Mock `:1132`／P148 の UHD 版）．
- 形：T2 と同じ layout（link あり）．先に `radio.rx.s0.sample_rate_hz` を `Cold` `now+20ms`，続けて `radio.rx.c0.frequency_hz` を `HardwareTimed` `now+10ms`．
- assert：`LATE_COMMAND` が空；device calls に `apply rx 0 … freq=… at=<now+10ms> effective=…` があり `at` が `now + ms(10)`（床の `now+20ms` でない）；`timing` の `cold_change` の `e == now + ms(20)`．
- 観測：device calls の `at=` と event．
- 備考：item 7（channel ごとの timed）と重なるなら一本化してよい．

**T5（integration，`crates/ezsdr-radio-uhd/tests/fake.rs`）: `ur_25_the_current_keys_floor_a_cold_change_only_at_its_own_stream`**

- 狙い：今の key のモードの床は自分の line だけ（brief「今のモードでは今どおり」）．T2 と同じ形の操作が，今のモードでは床上げされないという対照．
- 形：`Direct::with_links(FakeConfig::default(), &[("radio.rx.channels", Scalar::from(1)), ("radio.tx.channels", Scalar::from(1))], vec![attached(ezsdr_kernel::stream::BackPressure::DropOldest)])`．`direct.update("radio.tx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(50)))`，`direct.update("radio.rx.sample_rate_hz", Value::num(2e6).unwrap(), Cold, Some(now + ms(10)))`．
- assert：`LATE_COMMAND` が空；`clocks` の `usrp/rx` は2本で `[0].ended_at == now + ms(10)`（instance の床に変わっていれば `now+50ms` になり LATE_COMMAND が1件出る）；`usrp/tx` は2本で `[0].ended_at == now + ms(50)`；`timing` の `cold_change` の `e` は rx `now+10ms`・tx `now+50ms`．
- 観測：event・clocks・timing．

**T6（integration，`crates/ezsdr-mock-radio/tests/mock_layout.rs`）: `mr_18_a_layout_cold_floor_crosses_directions_to_the_receive_side`**

- 狙い：T2 と**同じ操作・同じ instant・同じ event** を Mock 側で固定し，VH-8 の比較の数値を両側に置く（brief「MockRadio の layout のモードの同じ操作と同じ instant と event になる」）．
- 形：`built(Layout { profile: "x3xx", id: "mock", rx: elements(&["0:A:ubx:0"]), tx: elements(&["0:B:ubx:0"]), selector: &[], constraints: &[("radio.tx.c0.stream", eq(Scalar::from(0)))], env: &[], ports: &["rx0"] })`，`arm_start(T0)`，`now = T0 + 10_000_000`；`update("radio.tx.s0.sample_rate_hz", Value::num(2_000_000.0).unwrap(), Cold, Some(now + 50_000_000))`，`update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, Some(now + 10_000_000))`．
- assert：`step(now)` 後に `LATE_COMMAND` 1件（key `radio.rx.c0.antenna`，requested `now+10ms`，applied `now+50ms`）；`step(now + 50_000_000)` 後に `applied_of("radio.rx.c0.antenna") == [("TX/RX", now+50ms)]`，`applied_of("radio.tx.s0.sample_rate_hz") == [(2e6, now+50ms)]`；`clocks_of("mock/rx/s0")[0].ended_at == now+50ms`，`clocks_of("mock/tx/s0")[0].ended_at == now+50ms`．
- 観測：event・`applied` section・`ClockRegistry`．
- 備考：item 15 の VH-8 harness が同じ列を直接比べるなら，期待値の供給元として残すか一本化する（orchestrator の判断）．

---

### (c) 壊れうる形と mutation の行の案

（`old` の正確な文字列はコードと一緒に決める．`new` に，名前を変えた識別子が残っていないかを grep する — 1b-4b の gate の U41 の教訓．）

| # | 誤りの形 | 落ちる assert（killer） | 行の案 |
|---|---|---|---|
| C1 | layout モードの床が自分の stream の line だけ（`command_instant(line)` を使う／床の引数を `None`） | T2：rx の `cold_change.e` が `now+10ms`，`LATE_COMMAND` 0件，rx clock の cut が `now+10ms` | 新規：新分岐の床の引数を `None` にする（P141 と同じ形） |
| C2 | 床が key の方向だけ（`key_direction` で濾す） | T2（方向をまたぐ） | 新規：濾しを足す（P142 と同じ形） |
| C3 | 床が line の items からだけ（line のない先着を落とす） | T3 | 新規：床を `streams` の line の item から取る形へ（`old`/`new` はコードと一緒に） |
| C4 | layout モードで late の `LATE_COMMAND` を出さない | T2 | 新規：新分岐の `late` を `false` に（J17 と同型） |
| C5 | 今のモードも instance 全体の床にする | T5：`LATE_COMMAND` 1件・rx clock の cut が `now+50ms` | 新規：今のモードの `command_instant(line, …)`（`control.rs:522`）を新関数の呼び出しに置き換える |
| C6 | 床が timed にも掛かる（layout モード） | T4：`apply … at=now+20ms` になる | 新規：timed の `e` を床に max する（P148 の UHD 版） |
| C7 | 新関数が床を取らない（`base.max(floor)` → `base`） | T1 行2・行7；Mock `:999` | P141 を新関数の文に当て直す |
| C8 | 新関数の late が `e > base` の向き（受信 start の床で late を立てない） | T1 行5；Mock `:2524` | P163 を当て直す |
| C9 | 新関数が start の項を落とす | T1 行5・行6；Mock `:2524` | 新規（P163 の当て直しと分ける） |
| C10 | 先着の max でなく最初／min を取る | T1 行7 | 新規：`fold(base, i64::max)` を最初で止める形へ |
| C11 | `e` は床上げするが `late` を床前の要求 instant と比べない | T2 | C4 と同じ規則なら一本化 |

**当て直しが要る既存の行**（`old` が実装で動く）：

- `plan/maintenance/tools/mutations.json` の **P141・P142・P149**（ファイル行 5272・5282・5352）：`old` は `lib.rs:1899` の1行 `let floor = self.updates.iter().filter(...).max();`．この行が `timeline` へ移るので，同じ規則を壊す形に当て直す（床を `None` にする／方向で濾す／畳む）．
- 同 **P163**（ファイル行 5492）：`old` は `lib.rs:1902` の `if asked < now || e > asked {`．late の判定が関数へ移れば，新関数の文で同じ規則を壊す形に当て直す．
- `plan/phase7/tools/mutations.json` の **U41**（ファイル行 867）：`old` は `self.colds.insert((e, seq), (key.clone(), value));\n self.core.timing(...)`．layout の分岐が同じ挿入を共有する形ならそのまま；分けると一致回数が変わるので確認して当て直す．
- **J17**（ファイル行 598，`ezsdr-radio-uhd` の lib，killer `ur_25_a_cold_change_that_arrives_after_a_later_one_follows_it`）：`old` は `book_cold` の `if late { self.late_command(Some(key.clone()), requested.unwrap_or(now), e);`（`control.rs:545-547`）．今のモードの行がそのまま1回だけ一致するなら不変；layout の分岐を足してこの行が動くなら当て直す．
- 検索：`grep -rn "command_instant" plan/*/tools/mutations.json plan/maintenance/tools/mutations.json` は一致 0（今のモードの呼び出しを壊す行は今は無く，C5 の新規行が担う）．全リストの `old` を機械で走査した結果，`lib.rs` の床の行に一致するのは P141・P142・P149 の3行，`if asked < now || e > asked` は P163 の1行だけ．

---

### (d) 頼る振る舞い

- **この順序は H1〜H7 に依存しない**：床の判定は device call の前に決まる純粋な記帳の規則（RM-25，`design/07-radio-model.md:257`；MR-18 の `design/09-mock-radio.md:153`）で，UHD の測定値も FFI も使わない．表に無い装置の振る舞いも足さない．
- 変更の効果（cut・新しい segment・新しい SampleClock・configure）は RM-16/RM-25 と，受信の ready instant（1回の receive call = `block_len.max(rx_packet_samples)` と 3 ms の allowance，`crates/ezsdr-radio-uhd/src/provider/control.rs:195-204`，`design/18-uhd-radio.md:246`）に依存する．これは既存の `ur_25_a_cold_change_booked_anywhere_in_a_long_receive_call_is_on_time`（`fake.rs:2955`）が固定している．T2 の clock の期待値はこの式から出す．
- T2 が antenna の適用を assert する部分は **H6（VERIFIED，設計 33 D16）**：read-back は設定した property を読む．read-back 不一致で拒否する経路（item 9，#59 と同じ形）はこの項目では fault を入れない（一致する経路だけ）．
- **受信 stream の start の床は UHD では観測できない**（ceiling）：`Direct` は T0 を過ぎてから Action を積み，`book_cold` の `e` は `max(asked, now, …)` なので `start` は `now > T0` では支配しない．この項は Mock の `:2524` と T1 の行5・行6 が固定する．VH-8 でも比べられない．
- INFERRED は無し．床は規範（RM-25）で，装置の測定に依存しない．

---

### 関わる他の順序

- **1（keys の値）**：T2〜T4 は `keys: "channel"` の selector と layout の description（`prepare`・`book_cold` の分岐）が前提．`Direct::build`（`fake.rs:619`）に selector を渡す形は item 1 の実装に依存する．
- **2（layout から subdev spec）**：T2 は rx `0:A:ubx:0`・tx `0:B:ubx:0` を使うので，tx channel 0 が slot B の front end になること（item 2 の「device の channel の番号は論理 channel の番号と同じ」）と，front end の照合（item 3）に依存する．
- **6（`prepare` の stream 0）**：T2・T4 は numbered な rate key（`s0.sample_rate_hz`）と channel ごとの settings を `book_cold` が読むことが前提．C1 の行はこの近くの文に触れる．
- **7（channel ごとの timed の更新）**：T4 は item 7 と重なる（timed の instant そのもの）．一本化してよい．
- **9（antenna の cold）**：T2 の rx antenna の cut・再開・read-back は item 9 の機構．item 9 の fault 経路（read-back 不一致で拒否・halt）はここでは使わない．
- **13（実行時の判定）**：T2 の rx antenna の受理は RM-31 の判定に依存する（rx slot A・tx slot B なので違反しない）．同じ board にすると RM-31 が拒否し，床の観測にならない — layout の選び方がこの assert を守る．
- **15（VH-8）**：T6 が同じ操作列の Mock 側の期待値．item 15 の harness が runtime の更新まで比べるなら T6 と T2 の数値がその期待値になる．比べないなら2本のテストの並置で「同じ instant と event」を固定する．
- **final（計画の最後）へ送るもの**：当て直しの候補（P141・P142・P149・P163，確認 J17・U41）と，design の追随：RM-25 の *Test:* 行（`design/07-radio-model.md:258`）と UR-25 の *Test:* 行（`design/18-uhd-radio.md:248`，1500 字の上限に注意）に新しい UHD テストを足す．`design/07-radio-model.md:295`（RM-26 の「`ezsdr_radio::timeline` computes … RM-25 once … both Providers take … from it」）はこの抽出でむしろ真に近づく．「inline」の語は design に無い（`grep -n "inline" design/07-radio-model.md design/09-mock-radio.md` は一致 0）ので，抽出で偽になる規範文は見当たらない．1b-5 への申し送り（P141・P142・P149・P163 がどの分岐に依存するか）も final の一覧へ．

## 12. 宛先

この順序は，`keys: "channel"` の instance で Action の宛先がどう判定されるかを固定する：stream node（`<id>/rx/s<k>`・`<id>/tx/s<k>`）を受けること，`<id>` の `Stop` が全 stream を止めること，channel の無い stream node は MockRadio と同じ扱いになること，そして今の宛先（`<id>/rx`・`<id>/tx`）が keys のモードには無いこと．判定の参照は Mock の layout のモード（`crates/ezsdr-mock-radio/src/lib.rs:525-550` の `addressed`，`:1536-1543`（TxBurst），`:1549-1572`（Stop），`:1577-1588`（start_rx），`:1848-1852`（UpdateParameter））で，規則の文面は `design/07-radio-model.md:159`（RM-12「the changed key decides what changes, and the node only selects the SpecTime clock」）と `:324`（RM-29「one stream node per logical channel in each direction」）にある．今のモードの宛先は変えない（変えるのは「`set_subdev_spec` が増える」「prefix でない layout も受ける」の2つだけ；1b-4c の指示書 §「変えないこと」）．

### (a) 既存のテスト

- 真似る側（Mock の layout のモード，1b-3b で入った）：
  - `crates/ezsdr-mock-radio/tests/mock_layout.rs:601` `mr_12_d14_the_actions_of_a_layout_instance_name_a_stream` — `<device>`／stream node 宛ての `UpdateParameter` が通り（`:610-611`），`TxBurst` は tx の stream node だけで通り（`:613`，誤った宛先は `:619-622` で拒否），`Stop` は stream ごとで（`:640-646`），`start_rx` は rx stream node だけが受ける（`:627` は `<device>` 宛てを拒否），未知の番号 `rx/s9` は拒否（`:621`）．
  - `crates/ezsdr-mock-radio/tests/mock_layout.rs:1154` `mr_25_a_device_stop_stops_every_layout_stream` — `<device>` 宛ての `Stop` が全 stream（rx の clock がそこまで，tx の held burst が消える，`:1169-1176`）．
  - `crates/ezsdr-mock-radio/tests/mock_layout.rs:733` `mr_09_each_transmit_stream_has_its_own_clock_and_line` — 各 stream node 宛ての burst と，1本の `Stop` が他の stream に及ばないこと（`:779-789`）．
  - 判定の本体：`crates/ezsdr-mock-radio/src/lib.rs:533-535`（`k < self.rx.len()`／`k < self.tx.len()`＝**tree と同じ「方向ごとの channel 数」が stream node の範囲**），`:1624-1625`（channel の無い tx stream への burst は「MR-16: no transmit channel is configured」），`:1370-1375`（line の無い stream への `book_rx`/`book_tx` は no-op＝`start_rx`・`Stop` は拒否されない）．
- 今のモードの宛先を固定している UHD のテスト（この段階で結果が変わってはいけないもの）：
  - `crates/ezsdr-radio-uhd/tests/fake.rs:1650-1671` `ur_26_stop_actions` — `radio/tx`・`radio/rx`・`radio`・`radio/gpio` 宛ての `Stop` と，`radio/tx` 宛て `start_rx` の拒否（`:1664-1667`）．
  - `crates/ezsdr-radio-uhd/tests/fake.rs:1710-1761` `ur_25_a_receive_session_follows_its_recorded_plan` — `radio/rx` 宛ての `Stop` と `start_rx`（`:1728-1730`）．
  - `crates/ezsdr-radio-uhd/tests/fake.rs:1069-1091` `ur_21_txburst_refusals`・`:1074`（`radio/tx` 宛て burst が現行の宛先として通る）．
- 書き換わる実装（宛先の現在の判定）：`crates/ezsdr-radio-uhd/src/provider/control.rs:283-284`（update は `<id>` だけ），`:299-313`（`Stop` は `<id>`／`rx_id`／`tx_id`），`:318-332`（`start_rx` は `rx_id`），`:612-616`（burst は `tx_id`，channel 無しの理由）；`crates/ezsdr-radio-uhd/src/provider/core.rs:129-131`（`booked_stream` が stream 0 決め打ち），`:377-378`（`rx_id`＝`usrp/rx`，`tx_id`＝`usrp/tx`）．node の数は `crates/ezsdr-radio/src/channels.rs:418-448`（1 channel につき1 node，port `rx<k>`）が作る．
- 隣接して効くもの：`crates/ezsdr-radio-uhd/src/provider/mod.rs:598-610` `ur_15_a_second_stream_is_refused_rather_than_half_started`（`start` の列は今も方向ごとに1本：`:481-482`）．

### (b) 足すテスト

置き場は `crates/ezsdr-radio-uhd/tests/fake.rs`．`Direct`（`:597-701`）を layout の binding（selector に `keys: "channel"`，`rx`・`tx` は既定の FakeDevice（`crates/ezsdr-radio-uhd/src/device.rs:376` の UBX×2 スロット）に合う `["0:A:ubx:0","0:B:ubx:0"]`）で使い，受信は `rx0` port に link を1本つなぐ．このために足す道具（実装と同時に決める）：

- `Pushed::recv`（`:586-593`）が今は全 Action を node `usrp` にしてしまうので，target の path を解決した node を渡す（`Target::Resource { resource, path }` → `<resource>/<path>`；`crates/ezsdr-mock-radio/tests/mock_layout.rs:52-58` の `node_of` と同じ形）．既存テストは path が空なので結果は不変．
- 宛先を名指す push の補助（`update_at(path, …)`・`stop_at(path)`・`start_rx_at(path, …)`・`burst_at(path, …)`）と，`Direct::build` の `ctx.inputs`（`:642` が空）に waveform を入れる入り口（burst を**受ける**テストが要る．UR-21 が波形の bytes を要求する，`control.rs:625`）．link は `attached`（`:570-581`）の port を `rx0` にする（`prepare` の port 検査は順序6）．event の pairs（`:627`）に `usrp/rx/s0`・`usrp/tx/s0`（と `usrp/rx/s1`・`usrp/tx/s1`）を足す（`Core::new` が `RX_OVERFLOW` の handle を stream 0 の node で resolve する：順序5）．

**(b-1) `ur_26_d14_a_layout_instance_names_its_streams`**（受ける側）

- 前提：rx 2 channel・tx 2 channel の layout，リンク `rx0`，constraint `radio.tx.c0.stream = 0`（既定は rx c0 だけが stream 0：`crates/ezsdr-radio/src/channels.rs:965-967`）．
- 操作と assert：
  1. `radio` 宛て `UpdateParameter`（`radio.rx.c0.frequency_hz`，`hardware_timed`）→ 拒否されない．観測：`applied` 行と device の `apply rx 0 …freq=…` 呼び出し．
  2. `radio/rx/s0` 宛て `UpdateParameter`（`radio.rx.c0.gain_db` 等）→ 同上（device `apply rx 0 …gain=…`）．**stream node が受ける**ことの中心の assert．
  3. `radio/rx/s0` 宛てに **tx の key**（`radio.tx.c0.gain_db`）→ 受けて tx に効く（device `apply tx 0 …gain=…`）．「key が変えるものを決める」の assert（ROM-12:159）．
  4. `radio/tx/s0` 宛て `TxBurst` → 送られる（device `tx_send 0 …`，`bursts` に記録）．
  5. `radio/rx/s0` 宛て `Stop` → **受信だけ**切れる（`rx_stop` の行と `usrp/rx/s0` の clock の終わり）．ここで 4 の burst がまだ送信されることを assert する（rx の `Stop` が tx を止めない）．
  6. `radio/rx/s0` 宛て `start_rx` → 拒否されず，`usrp/rx/s0` の2本目の SampleClock が現れる（`crates/ezsdr-radio-uhd/tests/fake.rs:1516-1531` `ur_25_rx_channels_from_zero_starts_at_its_instant` と同じ観測）．
  7. 新しい held burst を `radio/tx/s0` 宛てで置き，`radio/tx/s0` 宛て `Stop` → その burst だけが消える（`bursts` に残らない，`COMMAND_REJECTED` の reason `UR-26: cancelled by Stop`：`crates/ezsdr-radio-uhd/src/provider/tx.rs:156,635`）．受信の `Stop` の行は増えない．
- 観測：device の calls，`applied`／`rejected` の section，`direct.clocks` の SampleClock（stream の id は `usrp/rx/s0`・`usrp/tx/s0`），イベント．

**(b-2) `ur_26_a_device_stop_stops_every_layout_stream`**（`<id>` 宛て `Stop`＝全 stream；`mock_layout.rs:1154` の対）

- 前提と同じ．held burst を tx に1本置き，rx を走らせたまま `radio` 宛て `Stop` → rx の clock が止まり，tx の burst も消える（拒否の行 `cancelled by Stop`）．方向ごとの1本しか動かさない段階なので「全 stream」は両方向で見る（同一方向の2本目は 1b-4d：指示書 §「1b-4d」）．

**(b-3) `ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`**（拒否側）

- 前提：同じ layout で，tx c0 は stream 0 に入れておく（burst の拒否が「channel 無し」で先に落ちないように）．
- 操作（各1 Action，target を明示）と assert（`rejected` の kind と reason，`COMMAND_REJECTED` の `action` がその Action の id であること：`crates/ezsdr-radio-uhd/tests/fake.rs:1087-1090` と同じ見方）：
  - `radio/rx`・`radio/gpio` 宛て `UpdateParameter` → kind `update_parameter`・reason `UR-26: the target is not this device`（`control.rs:283-284`）．**keys のモードでは `radio/rx` は宛先でない**ことの assert．
  - `radio/rx`・`radio/tx`・`radio/gpio`・`radio/rx/s9` 宛て `Stop` → kind `stop`・reason `UR-26: the Stop target is not this device or its streams`（`control.rs:301-302`）．
  - `radio`・`radio/tx/s0`・`radio/rx/s9` 宛て `start_rx` → kind `command`（tx node や instance は受け手でない；`control.rs:318,334`）．
  - `radio/tx`・`radio`・`radio/rx/s0`・`radio/tx/s9` 宛て `TxBurst` → kind `tx_burst`・reason `UR-21: the target is not this device's transmit stream`（`control.rs:615-616`）．
- 併せて「何も変わらない」を assert：clock の本数・`applied` が空のまま．Mock 側の同じ形は `mock_layout.rs:619-635`．

**(b-4) `ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment`**（channel の無い `s1`）

- 前提：rx 2 channel・tx 2 channel，`radio.tx.c0.stream = 0` だけ（`c1` は既定の −1）＝`tx/s1`・`rx/s1` は channel を持たない（この段階で `c<n>.stream ≥ 1` は拒否：順序10）．
- 操作と assert：
  1. 先に `radio/rx/s0` 宛て `Stop`（stream 0 を止めておく）→ 1本目の clock が終わる．
  2. `radio/rx/s1` 宛て `start_rx` → **拒否されない**（`COMMAND_REJECTED` なし，`rejected` に `command` の行なし）．`usrp/rx/s0` の clock は1本のまま（＝stream 0 の line に記帳されていない）．`usrp/rx/s1` の clock は0本．新たな block も来ない．根拠：Mock は stream node を stream と読んだ後，channel の無い stream でも拒否しない（`lib.rs:1577-1588`，line が無ければ `book_rx` は何もしない `:1370-1372`）．
  3. `radio/tx/s1` 宛て `TxBurst` → **拒否される**．assert：kind `tx_burst`，reason に `no transmit channel`（Mock `MR-16: no transmit channel is configured`（`lib.rs:1624-1625`）／UHD `UR-21: no transmit channel is configured`（`control.rs:612-613`））．`tx_send 0 …` が**増えない**（＝stream 0 に勝手に載らない），`bursts` は空．「未知の node」の拒否（target の理由）と区別するため **reason の中身まで** assert する．

### (c) 壊れうる形と mutation の行の案

`old` の文字列はコードと一緒に決める（id は実装時の空き番号，P281 以降を想定）．「行の案」は壊す規則と形：

1. **layout の update が stream node を受けない**（`<id>` だけを宛先に残す：`control.rs:283` の検査を layout でもそのまま使う）→ (b-1) の 2・3 が落ちる．行：`file: crates/ezsdr-radio-uhd/src/provider/control.rs`，`filter: ur_26_d14_a_layout_instance_names_its_streams`．
2. **stream node の範囲を stream 0 に限る**（境界を `k < 方向の channel 数` → `k == 0`）→ (b-1) の 5・6 と (b-4) の 2（rx/s1 が拒否される），(b-4) の 3（reason が target の理由になる）が落ちる．行：`file: この判定を書いた場所`（`control.rs` か，`ezsdr_radio` へ移したなら `crates/ezsdr-radio/src/channels.rs`／`lib.rs`），`filter: ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment`．**(b-4) の 3 は reason まで見るので，この mutant を殺せる**．
3. **keys のモードでも今の `rx_id`・`tx_id`（`usrp/rx`・`usrp/tx`）を stream と読む**→ (b-1) が落ち（`rx/s0` 宛てが受からない），(b-3) の `radio/rx` 拒否が落ちる．行：`filter: ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`．
4. **`TxBurst` の宛先の方向を見ない**（rx node も同じ番号の tx stream として扱う）→ (b-3) の `radio/rx/s0` 宛て burst の拒否が落ちる．等価な mutant になる形（内側の `node != tx_id` が同時に捕まえる場合）は行にしない．
5. **burst を宛先の `k` でなく stream 0 に記帳する**（`booked_stream(Dir::Tx)` のまま）→ (b-4) の 3（拒否されず `tx_send 0` が出る）が落ちる．行：`filter: ur_26_d14_a_channel_less_stream_keeps_the_mock_s_treatment`．(b-1) の 4 は stream 0 なので殺せない．
6. **`start_rx` を宛先の `k` でなく stream 0 に記帳する**→ (b-4) の 2（stream 0 が再開して2本目の clock ができる）が落ちる．
7. **stream 宛ての `Stop` が全 stream を止める**（Mock の P134（`plan/maintenance/tools/mutations.json:5202`）の UHD 版）→ (b-1) の 5（rx の `Stop` で tx の burst も消える）が落ちる．行：`filter: ur_26_d14_a_layout_instance_names_its_streams`．
8. **`<id>` 宛ての `Stop` が片方向だけ止める**（`device` を tx の条件から落とす：`control.rs:304`）→ (b-2) が落ちる．行：`filter: ur_26_a_device_stop_stops_every_layout_stream`．**注意：同じ行に既存の J11（`if device || node == Some(&self.core.tx_id) {` を `old` にする）が乗っており，この段階でこの分岐を書き換えるので J11 の `old` の当て直しが要る**（指示書 §守ることの「`new` に変えた識別子が残っていないか grep」）．
9. **`start_rx` がどんな node でも受ける**（`node == Some(&self.core.rx_id)` を外す：`control.rs:319`）→ (b-3) の `radio`・`radio/tx/s0` 宛てが落ちる（今のモードの `ur_26_stop_actions`（fake.rs:1664）も同じ規則を押さえる）．行：`filter: ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`．
10. **未知の node を instance と読む**（分類の落ち先を `Instance` にする）→ (b-3) の `radio/gpio` が落ちる．行：`filter: ur_26_d14_the_old_addresses_and_foreign_nodes_are_refused`．

補足：宛先の判定を Mock と共有するために `ezsdr_radio`（`stream_target` の隣，`crates/ezsdr-radio/src/lib.rs:637-651`）へ移す場合（1b-3b 報告 `plan/phase8/reviews/1b3b-report.md:215` の申し送り），行はその場所に置き，Mock 側の既存行（`addressed` を `old` に含む行は今は無い）と重ならないことを確かめる．移した関数は両 Provider のテストで殺せることを確かめる（Mock の layout テストは `crates/ezsdr-mock-radio/tests/mock_layout.rs:601` ほか）．

### (d) 頼る振る舞い

- 表の H1〜H7 に**直接には依存しない**．宛先の判定は host 側で完結し（受けるかどうかを device の前に決める），拒否の経路（(b-3)・(b-4) の tx/s1）は device を呼ばないこと自体を assert する（`tx_send`・`rx_open`・`rx_start` の呼び出しが増えない）．受けられた操作が device に届く側の振る舞い（burst の送信，rx の `Start`／`Stop`）は既存のテスト（`crates/ezsdr-radio-uhd/tests/fake.rs:1650` ほか）が押さえており，この順序はそれを再依存しない．
- 表に無いものとして1つ足す（VERIFIED）：**device の呼び出しは stream 番号でその stream に届く**（`Device` trait が 1b-4a で stream を名指す：`crates/ezsdr-radio-uhd/src/device.rs:159-172` の `rx_open(stream, …)`／`tx_open(stream, …)`，根拠は UR-2）．固定しているテスト：`ur_33_two_receive_streamers_keep_their_own_channels`（fake.rs:2116）と `ur_33_two_transmit_streamers_keep_their_own_reports`（fake.rs:2190）．(b-1) の 4（`tx_send 0`）と (b-4) の 3（`tx_send 0` が増えない）の観測がこれに乗る．

### 関わる他の順序

- **1（`keys` の値）**：宛先の判定は `keys` のモードでだけ変わる．今のモードの宛先（`radio/rx`・`radio/tx`）は不変で，それを固定するのは `ur_26_stop_actions`（fake.rs:1650-1671）と `ur_25_a_receive_session_follows_its_recorded_plan`（fake.rs:1730）．
- **5・6（node と event，`prepare` の stream 0）**：node の名前と数は `crates/ezsdr-radio/src/channels.rs:418-448` の tree，stream 0 の SampleClock・port `rx0` は順序6 が作る．(b) のテストはその node 名（`usrp/rx/s0` など）をそのまま使う．
- **10（membership）**：「`c<n>.stream ≥ 1` を拒否する」が (b-4) の前提（`s1` に channel が入らない）．将来この拒否が緩むと (b-4) の前提が変わる（1b-4d で複数 stream を動かすときに見直す）．
- **11（instance 全体の床）・13（実行時の判定）**：受けた更新は今と同じ key の経路（timed／cold・`violations`）に載る．この順序は「受けるかどうか」だけを決め，値の判定は item 11・13 の担当．
- **14（Manifest）**：拒否の観測は `rejected`（kind・reason）と `COMMAND_REJECTED`（`action`），受けた側は `applied`・`timing`・`bursts` で見る（UR-30，`crates/ezsdr-radio-uhd/src/provider/mod.rs:285-297`）．
- **15（VH-8）**：Mock と同じ受理・拒否の比較がこの順序の最終の裏取り（指示書 15）．ただし channel の無い stream への `start_rx`（Mock `lib.rs:1577-1588`＋`:1370-1372`）と burst（`:1624-1625`）は**Mock 側にテストが無い**（`crates/ezsdr-mock-radio/tests/mock_layout.rs` に該当なし；grep `"no transmit channel" crates/ezsdr-mock-radio/tests` → 一致なし）ので，item 15 の harness で両 Provider を並べるか，Mock 側の assertion を 1 本足すかは orchestrator の裁定．`Stop` を channel の無い `rx/s1` に宛てる場合（Mock は no-op で受理，`:1370-1372`）も同じ扱いで，この順序では名指しされていないので item 15 に送る．
- **1b-4d**：1b-4b から送った一覧の (2)「command の宛先」（`plan/phase8/reviews/1b4b-report.md:133`）のうち，stream 0 の分はここで実装・固定する．stream ごとの thread・名前・handle（同 §7 の 1・3）は 4d に残るので，判定は stream 0 を決め打ちせず `k` で書いておく（4d は (b-4) の拡張だけで済む）．

## 13. 実行時の判定

この項目は，keys のモードの instance で，更新の effective instant に投影した設定全体が `Description::coerce`（`violations`，すなわち RM-31・RM-32 を含む）で判定され，反れば `COMMAND_REJECTED` になって何も記帳されないこと，そして `prepare` に同じ設定を与えたときと同じ理由になること（同じ 1 か所の判定）を固定する．判定は host 側で閉じる（`coerce` は純粋，MA-11）。

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:1559` `ur_25_a_cold_change_the_envelope_refuses_changes_nothing` — 今のモードで，予約時（`book_cold`，`crates/ezsdr-radio-uhd/src/provider/control.rs:509`）の `coerce` が投影した設定全体（`control.rs:523-528` の `config_at(e)` + 更新 key）を判定し，拒否が `radio.COMMAND_REJECTED`（理由は `"RM-7: "` で始まり，`payload["action"]` が Action を名指す）になり，device を触らない（`apply`・`rx_stop` の呼び出しが無い）ことを固定している．新しいテストはこの形の keys のモード版．このテスト自体は今のモードの振る舞いとして変えない．
- `crates/ezsdr-radio-uhd/tests/fake.rs:510` `ur_11_coerce_is_the_descriptions` — `Provider::coerce` が description のものであること（今のモード）．keys のモードでは同じ 1 か所が `channels::Description` になる（順序 1 の担当）．
- `crates/ezsdr-radio-uhd/tests/fake.rs:704` `ur_12_prepare_refusals` と `:751` `ur_12_a_rate_the_device_does_not_apply_is_refused` — prepare が `coerce` の `rejected` をそのまま拒否にする（`crates/ezsdr-radio-uhd/src/provider/mod.rs:331-333`，`format!("UR-12: {}", refused.reason)`）ことの入口と，`UR-12:` 接頭辞の形．
- `crates/ezsdr-radio-uhd/src/provider/control.rs:810` `ur_14_booking_does_not_wait_for_a_future_cold_switch` — 予約が受理されれば `rejected` が空のままであること（`:833`）を unit で固定する（対照側）．
- `crates/ezsdr-radio/tests/channels.rs:598` `rm_31_full_duplex_cases`，`:693` `rm_32_throughput_cases` — 1 か所の判定そのもの（`crates/ezsdr-radio/src/channels.rs:575-626` の `coerce` が `:611-623` で `violations` を通し，違反を要求の key に課す）．とくに `:674-677`（`radio.rx.c0.antenna` を `TX/RX` にする 1 key の更新を，送信 channel が stream に居る設定の上で拒否する）と `:655-689`（設定全体の判定）がこの項目の核．新しいテストはこれを繰り返さず，Provider の経路（prepare と予約）だけを見る．
- `crates/ezsdr-mock-radio/tests/mock_layout.rs:831-890` `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` — MockRadio の同じ順序（実行時の拒否，`applied()` に何も残らない，`prepare` も同じ理由）．順序 15（VH-8）が比べる相手で，新しいテストはこの UHD 版．

### (b) 足すテスト

二つとも `crates/ezsdr-radio-uhd/tests/fake.rs` に置く（`Direct` rig，manual clock．`lib` の rig は今のモードの description で作られているので，keys のモード用の rig をこの項目のために増やさない——同じ振る舞いは `fake.rs` の 1 か所で見る）．どちらも keys の selector（`{"keys": "channel", "rx": [...], "tx": [...]}`）の binding を `Direct::build`（fake.rs:614-663）に渡す．下ごしらえ：

- link は port `rx0` に付ける（keys のモードの受信 port は `rx<k>`．今の `attached()`（fake.rs:570-581）は port `rx` 固定なので，port を渡せる形に広げるか，テスト内に `attached_to("rx0")` を置く）．link があることで受信 stream 0 の line ができ，「何も記帳しない」を plan・clock・device call で見られる．
- instance ごとに別の `FakeDevice`（`common/mod.rs:298`）で作る．既定 serial が instance ごとに違う値になる（この段階の項目 5）ので，同じテスト内の複数 instance は衝突しない．

**1. `ur_25_a_layout_instance_s_full_duplex_rule_is_judged_at_runtime`（RM-31）**

- 用意：`Direct::build(FakeConfig::default(), &[("radio.tx.c0.stream", Scalar::from(0))], vec![rx0 の link], json!({"keys": "channel", "rx": ["0:A:ubx:0"], "tx": ["0:A:ubx:0"]}))`．受信 c0 の既定 antenna は `RX2`（`crates/ezsdr-radio/src/channels.rs:886`）なので prepare は通り，送信 c0 は stream 0 に居る．
- 操作：`before = device.calls().len()` と受信 clock の record 数を控える；`direct.update("radio.rx.c0.antenna", Value::from("TX/RX"), Cold, None)`；`direct.settle(Wall::from_millis(200))`（新しい segment の origin——`e` + rx start lead 50 ms + 余裕——を越え，誤って予約された変更が device に届くなら届く長さ）．
- assert：
  - `direct.of("radio.COMMAND_REJECTED")` がちょうど 1 件，`payload["action"] == 1`（RM-22），`payload["reason"]` が `"RM-31: "` で始まり `"receive channel c0"` と `"transmit channel c0"` を含む（文面は `channels.rs:809-812`）．
  - 何も記帳しない：`direct.device.calls()[before..]` に `apply rx 0 … antenna=TX/RX`（fake.rs:1606 と同じ形）が無い；`finish()` の `ezsdr.radio.uhd.timing` に `what == "cold_change"` かつ `key == "radio.rx.c0.antenna"` の行が無い；同じ `timing` の `plan` 行の受信 segment が 1 本のまま；`direct.clocks.sample_clock_records()` の受信 clock が増えていない；`ezsdr.radio.uhd.applied` に `radio.rx.c0.antenna` を `TX/RX` とした行が無い；`ezsdr.radio.uhd.rejected` にはその理由の行がある（拒否そのものの記録，UR-30）．
  - `prepare` は同じ理由：同じ keys の binding（`fake.rs:82-93`）で `UhdRadio::from_binding` を作り，`direct_prepare`（fake.rs:535-568）に `&[("radio.tx.c0.stream", Scalar::from(0)), ("radio.rx.c0.antenna", Scalar::from("TX/RX"))]` を渡し，`error.message == format!("UR-12: {reason}")`（`reason` は上の event のもの）．
  - 対照（規則が「keys のモードは antenna を変えられない」ではないこと）：送信 c0 を stream に入れない 2 つ目の `Direct`（constraints 無し．既定 `-1`）で同じ更新 → `COMMAND_REJECTED` 0 件，`device.calls()` に `apply rx 0 … antenna=TX/RX`，`applied` に antenna の行がある．

**2. `ur_25_a_layout_instance_s_throughput_rule_is_judged_at_runtime`（RM-32）**

- 用意：`Direct::build(FakeConfig::default(), &[("radio.rx.s0.sample_rate_hz", Scalar::try_from(20e6).unwrap())], vec![rx0 の link], json!({"keys": "channel", "rx": ["0:A:ubx:0", "0:B:ubx:0"], "tx": ["0:A:ubx:0"], "link_gbps": [1], "image": ["hg"]}))`．受信は c0 が stream 0，c1 は none（既定），s0 は 20 Msps（1 channel で 80 MB/s ≤ 1 link の 100 MB/s．上限は `x3xx::performance(1)`，`crates/ezsdr-radio/src/x3xx.rs:469-475`；default は `channels::from_selector` で HG は 1 GbE，`channels.rs:180-181`）．
- 操作：`direct.update("radio.rx.c1.stream", Value::from(0), Cold, None)`；`settle(ms(200))`．
- assert：
  - `COMMAND_REJECTED` 1 件，`reason` が `"RM-32: "` で始まり `"160000000"` と `"rx"` を含む（文面は `channels.rs:851-854`）．
  - 何も記帳しない：`timing` に `cold_change`（key `radio.rx.c1.stream`）が無い；`device.calls()` に `rx_open 0 [0, 1]`（`crates/ezsdr-radio-uhd/src/device.rs:1050` の形）が無く，`apply rx 1` も無い；`applied` に channel 1 の行が無い．
  - `prepare` は同じ理由：`&[("radio.rx.s0.sample_rate_hz", Scalar::try_from(20e6).unwrap()), ("radio.rx.c1.stream", Scalar::from(0))]` → `error.message == format!("UR-12: {reason}")`．
  - 対照：s0 を 2 Msps にした 2 つ目の `Direct`（合計 16 MB/s）で同じ membership 更新 → `COMMAND_REJECTED` 0 件，`timing` に `cold_change` 行があり，切れの origin を越えた `settle` の後に `rx_open 0 [0, 1]`（UR-25 の streamer の開き直し）が見える．

### (c) 壊れうる形と mutation の行の案

1. **予約の判定が更新の key だけを見る（投影した設定全体でない）**．`control.rs:523-528` の現行の形（`candidate` = `config_at(e)` + 更新 key を要求の全体にする）を壊し，`Requested` の constraints を `{更新の key: 値}` だけにする．落ちる assert：テスト 1 の「`COMMAND_REJECTED` がちょうど 1 件で `RM-31:`」（送信 c0 の stream が要求に無いと既定 `-1` になり違反が消える），テスト 2 の「`RM-32:` と 160000000」（s0 の 20 Msps が要求に無いと既定 1 Msps で収まる）．行の案：`old` は `let mut candidate = self.config_at(e);` から要求を作る現物（正確な文字列はコードと決める），`new` は空の `BTreeMap` から始める形．killer はテスト 1（テスト 2 も落ちる）．既存の envelope テストも一部の形を殺すが，この行は keys のモードの投影を固定する新しい行として記録する．
2. **拒否の後も記帳して続ける（`return` を落とす）**．`control.rs:533-535` の `return self.core.command_rejected(...)` を `self.core.command_rejected(...)` にし，拒否した変更を予約・適用へ流す．落ちる assert：テスト 1 の「`cold_change`（`radio.rx.c0.antenna`）が無い／`apply rx 0 … antenna=TX/RX` が無い」，テスト 2 の同型（`cold_change` なし，`rx_open 0 [0, 1]` なし）．killer はテスト 2（テスト 1 も落ちる）．既存の envelope テストは予約時に device を触らず（link が無く cold_change 行も書かれない経路——`control.rs:548-551`），この形を殺さない——だから新しいテストで固定する．
3. **`prepare` が `coerce` の理由を運ばない**．`mod.rs:331-333` の `format!("UR-12: {}", refused.reason)` を固定の文面にする．落ちる assert：テスト 1・2 の `error.message == format!("UR-12: {reason}")`（`"RM-31:"`/`"RM-32:"` の包含含む）．killer はテスト 1（または 2）．
4. **`coerce` が `violations` を判定しない**（`channels.rs:611` の空反復）は P135（`plan/maintenance/tools/mutations.json:5212-5220`）が既にあり，killer は Mock のテスト．新しいテストもこの行で落ちるので UHD 側の行は要らない．
5. （行の形は実装に合わせる）**判定に今の description を使う**（モードの split が予約・prepare の `coerce` に届かない）．layout の key は `RM-4: not a radio key` などになる．落ちる assert：テスト 1・2 の理由の接頭辞．Mock の `profile.rs:185-190` と同型の split なら，そこの 1 行の行にできる（killer テスト 1）．よい 1 行にならなければ，この形は順序 1 のテストと行が見る．

### (d) 頼る振る舞い

- H1〜H7 のどれにも頼らない．この項目の判定は host 側で閉じる：`coerce` は純粋（MA-11，`design/07-radio-model.md:341`）で device を呼ばず，予約の拒否は `configure`・`apply` より前に起きる（`control.rs:529-535`）．テストが device を見るのは「触っていない」ことの観測としてだけ．
- 表に無い根拠：
  - RM-31 の「送信中は同じ board の受信 antenna は RX2」：UHD の資料（`dboards.dox:352-354,453`，CBX は `:309-311`）で **VERIFIED**（設計 33 D8，`plan/phase8/33-channels-and-streams.md:156`）．実機が本当に強制するかの確認は bench（BP0・BP4b）の仕事で，この項目のテストは頼らない（Module が装置より先に拒否する）．
  - RM-32 の「1 link 分の bytes/s が上限」：UHD は link を帯域でなく割り当て数で選ぶ（**VERIFIED**，設計 33 D9，`:160-162`）．上限の値は `x3xx::performance`（`x3xx.rs:469-475`）．
  - どちらも装置側の確認は bench の範囲で，ここでは確かめない（INFERRED を残すものはない）．

### 関わる他の順序

- **1（`keys` の値）**：この順序の全体が keys のモードの上で起きる．モードが選ばれないと，更新は `UR-26: … is not updated at runtime`（`control.rs:294`）で拒否され，テストは理由の不一致で落ちる．
- **6（`prepare` の stream 0）**：受信 stream の channel 列，rate の適用，channel ごとの `apply`，port `rx0` の link の受け入れ（UR-12 の port の検査）がテストの前提．
- **7（channel ごとの timed の更新）**：RM-31・RM-32 は `radio.{rx,tx}.c<n>.{frequency_hz,gain_db}`（`HardwareTimed`，`crates/ezsdr-radio/src/lib.rs:274-275`）では破れない（RM-30 の設定は周波数・利得を拘束に使わない）．よってこの順序のテストは `cold` の更新だけを使い，timed の側は順序 7 のテストと UR-24 に任せる．
- **9（antenna の `cold`，D16）**：RM-31 の更新が `book_cold` に届くのは antenna が `Cold`（`lib.rs:276`）だから．
- **10（membership）**：RM-32 の更新（`c1.stream`）が `book_cold` に届くのは `Some(Cold)`（`lib.rs:277`）だから．stream 1 以上への設定の拒否（この段階は stream 0 だけ）とも隣り合う．
- **11（instance 全体の床）**：判定は床でずれた `e` の設定で行われる（`plan/phase8/33-channels-and-streams.md:180`）．この順序のテストは先に受けた `cold` を持たない（`e` = 予約時）ので，床の側（および `config_at(e)` を `self.config` に取り違える形）は順序 11 のテストが持つ．
- **12（宛先）**：テストは更新を `<id>` 宛てに送る．MockRadio の layout のモードが instance と stream の node を受ける（`crates/ezsdr-mock-radio/src/lib.rs:1846-1852`，`addressed` は `:525-550`）のと同じ形で，keys のモードも `<id>` を受けること（`<id>/rx/s0` も受けるのは 12 側）．
- **15（VH-8）**：この 2 つのテストが UHD 側の拒否の列で，Mock の `rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` と対にされる（同じ selector と Spec で同じ拒否）．

## 14. Manifest

`ezsdr.radio.uhd.device` section に，(1) 方向ごとの subdev spec の markup，(2) serial，(3) rate の照合の結果（比べた／比べていない）が残ること（今の key のモードでも markup は残る）．足す欄の名と意味は，実装と UR-30 の改訂文とテストで同じにする：

```json
{ "…": "既存の describe() と args はそのまま",
  "serial": "F00BA2",
  "subdev_spec": { "rx": "A:0 B:0", "tx": "A:0" },
  "rates_checked": { "rx": true, "tx": false } }
```

- `subdev_spec.<dir>` は `Device::set_subdev_spec` に渡した文字列そのもの．要素の無い方向は `null`（または欠け）で，その方向には `set_subdev_spec` を呼ばない（空の spec を渡さない: H7）．
- `rates_checked.<dir>` は `arm` で `rates` を比べたかどうか．比べた方向は `true`，`arm` で streamer を開かなかった方向は `false`．`false` の方向も欄は欠けない（`json!(false)` と比較できる形）．
- 観測は二つ：unit は `radio.instance().sections[&Namespace::parse("ezsdr.radio.uhd.device").unwrap()]`（既存の `fake.rs:487` と同型），e2e は sealed Manifest の `section(&manifest, "device")`（helper `tests/common/mod.rs:376-378`；section は cleanup の後に Manifest へ移る `crates/ezsdr-kernel/src/coordinator/ending.rs:98-126`，`:258-266`）．`device` は `from_binding`（`crates/ezsdr-radio-uhd/src/provider/mod.rs:193-199`）と `arm`（`:407-443`）が `self.instance.sections` に書けば届く（`cleanup` の `write_sections` は applied/bursts/async/rejected/timing/stats だけを書き換える，`mod.rs:285-297`）．section の中身は自由形の `serde_json::Value`（`crates/ezsdr-kernel/src/manifest.rs:326-335`）なので schema は変わらないが，key は ASCII でなければならない（OV-15，`manifest.rs:446-450`）．

### (a) 既存のテスト

- `crates/ezsdr-radio-uhd/tests/fake.rs:470` `ur_09_profile_values_reach_the_capabilities_and_the_envelope_section`（assert は `:487`）— `device` section を読む唯一の既存テスト．`grep -rn "radio.uhd.device\|uhd.device" crates --include=*.rs | grep -v "src/provider"` の結果はこの 1 行だけ．`["fake"] == true` が残ること．
- `crates/ezsdr-radio-uhd/tests/fake.rs:1960` `ur_30_the_sections_are_written` — section 名の列挙（`device` を含む）．欄を増やしても列挙はそのまま通ること．
- `crates/ezsdr-radio-uhd/tests/fake.rs:96` `ur_05_from_binding_refusals` — 今のモードの `from_binding` の拒否の列（`:97-130`）．spec の設定と `serial()` の読みを足しても，各拒否が同じ順・同じ文面で先に決まることの基準．
- `crates/ezsdr-radio-uhd/tests/fake.rs:162` `ur_05_the_profile_must_be_the_device_s_front_ends` — front end の照合．spec を `channels`・`front_end` より先に設定するので，`:169`（`"UR-5: profile x3xx needs 2 rx channels; the device has 1"`）と `:171`（`"…needs 1 rx channels; the device has 0"`）は `set_subdev_spec` の失敗（UHD の LookupError 文面）が先に出る形に**結果が変わる候補**（item 3 が扱い，final の一覧に入れる）．`:166-168` と `:170` は spec が成功するので同じ文面のまま．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2362` `ur_33_the_serial_is_the_config_s` — 既定 serial `"FAKE0001"` を固定．item 5 が既定を instance ごとに変えるので当て直しになる（このテストには `mutations.json` の P199 が刺さっている）．私のテストは明示の `serial` を使い，既定に依存しない．
- `crates/ezsdr-radio-uhd/tests/fake.rs:2339` `ur_33_rates_report_the_grid_and_the_pre_chain_one` — 照合が読む `rates` の値（grid と前 chain の `{1.0}`）を unit で固定．照合の判定は item 4 だが，読む値はここで押さえられている．
- `crates/ezsdr-radio-uhd/tests/fake.rs:810` `ur_13_arm_cases` — `arm` の成功と ref_locked の拒否．照合を `arm` に足す位置の基準（このテストの `S` の記録と拒否は変えない）．
- 無いもの（検索付き）：`device` section の markup・serial・rate を見るテストは無い（上の grep）．`FakeDevice::calls()` を**完全一致**で比べるテストも無い（`grep -rn "calls()\.iter()\.eq\|assert_eq!(calls\|, calls)" crates/ezsdr-radio-uhd/tests/*.rs` は 0 件；既存の読みは prefix の filter，`tests/common/mod.rs:452-454`）．したがって `set_subdev_spec` が calls に 1〜2 行増えても既存の読みは当たらない．

### (b) 足すテスト

**T1 `ur_30_the_device_section_names_the_subdev_spec`**（`crates/ezsdr-radio-uhd/tests/fake.rs`，unit．`ur_09_…`/`ur_10_instance` の隣）— 今の key のモード：

- (i) 既定: `UhdRadio::from_binding(&binding(json!({"args": ARGS}), Some(x3xx())), fake(FakeConfig::default()))`（helper は rx/tx を `["0:A:ubx:0","0:B:ubx:0"]` にする，`fake.rs:82-94`）．`device["subdev_spec"]["rx"] == json!("A:0 B:0")`，`["tx"] == json!("A:0 B:0")`，`device["fake"] == json!(true)`．さらに「記録は渡した文字列そのもの」を，`calls` の `"set_subdev_spec rx A:0 B:0"` から引いた文字列と `device["subdev_spec"]["rx"]` の一致で見る（各区間 1 回）．
- (ii) 逆順: rx `["0:B:ubx:0","0:A:ubx:0"]`（tx は既定）→ `["rx"] == json!("B:0 A:0")`，calls に `"set_subdev_spec rx B:0 A:0"`．`x3xx::from_selector` の prefix の制限（`crates/ezsdr-radio/src/x3xx.rs:568-580`）を外した今のモードがこれで落ちる．
- (iii) 1 要素: `one_board("ubx")`（`fake.rs:134-136`，rx/tx とも `[0:A:ubx:0]`）→ 両方向 `json!("A:0")`．
- (iv) 要素の無い方向: rx `["0:A:ubx:0"]`，tx `[]` → `device["subdev_spec"]["tx"].is_null()`（`null` と欠けの両方に真なので omit も許す．空文字や `"A:0"` は落ちる）．calls に `"set_subdev_spec tx"` が無いこと．
- 観測: `instance().sections`（`crates/ezsdr-kernel/src/module_api.rs:607`，`:633`）と `FakeDevice::calls()`（`crates/ezsdr-radio-uhd/src/device.rs:646-649`）．sealed Manifest への経路は T3 と `ur_30_the_sections_are_written` が押さえる．

**T2 `ur_30_the_device_section_records_the_serial`**（同ファイル，unit，`keys: "channel"`）：

- `binding(json!({"args": ARGS, "keys": "channel", "rx": ["0:A:ubx:0","0:B:ubx:0"], "tx": ["0:A:ubx:0"]}), Some(x3xx()))`，device `fake(FakeConfig { serial: "F00BA2", ..FakeConfig::default() })` で `from_binding`．
- assert: `device["serial"] == json!("F00BA2")`，calls に `"serial"` が 1 回（`device.rs:824-829`）．既定の serial には依存しない．同じ値を登録に使うこと（2 つ目の拒否）は item 5 のテスト．

**T3 `ur_30_the_device_section_records_the_rate_check`**（同ファイル，e2e）— layout モード（fixture は item 6 が `tests/common/mod.rs` に足す `profile()`/`receive_spec()` の layout 版を使う）：

- (i) 比べる: rx 1 channel（規定で `c0.stream = 0`，`crates/ezsdr-radio/src/channels.rs:965-967`）・tx 1 channel（requires に `radio.tx.c0.stream = 0`）・`rec` の output と host link の placement → `captured(run)`（`common/mod.rs:401-405`）．`section(&manifest, "device")` で `rates_checked == json!({"rx": true, "tx": true})`；calls に `"rates rx 0"` と `"rates tx 0"` が各 1 回（`device.rs:864-877`）．
- (ii) 送信を比べない: 同じで `radio.tx.c0.stream` を入れない（tx は全部 `-1`）→ `json!({"rx": true, "tx": false})`，calls に `"rates tx"` が無い．`json!(false)` との比較にする（欄が欠けると `Null` で落ちる）．
- (iii) 受信の link が無い: `ur_29_a_lost_device_is_found_while_idle`（`fake.rs:1941-1957`）と同じく `placements` と `rec` の binding を外した Session（layout モード）→ `json!({"rx": false, "tx": false})`，calls に `"rates"` が 1 つも無い（`arm` は link が無ければ rx の streamer を開かない，`mod.rs:423-428`）．
- (iv) 条件付き（orchestrator の裁定待ち，§関わる他の順序）：今のモードにも照合を入れるなら，既存の `receive_run(FakeConfig::default(), 1, 1_000)`（`fake.rs:937-940`）の Manifest に `json!({"rx": true, "tx": false})` の 1 行を足す．layout モードだけなら入れない．
- 観測: sealed Manifest の `device` section と `FakeDevice::calls()`．
- （この3本は「誰が読むか」ではなく「何が残るか」を固定する．(i)(ii)(iii) で同じ欄が 3 通りに変わるので，定数・欠け・入れ替えの実装はどれかで落ちる．）

### (c) 壊れうる形と mutation の行の案

行は `plan/maintenance/tools/mutations.json` に `crate: "ezsdr-radio-uhd"`, `test_bin: "fake"` で足す．`old` の正確な文字列はコードと一緒に決める．`file` は特に断らなければ `crates/ezsdr-radio-uhd/src/provider/mod.rs`．

1. **markup の順を逆にする／区切りを変える**（`format!("…:{}", slot, frontend)` と `join(" ")` の式に `.rev()` を足す，区切りを `","` にする，`{:?}` で書く）→ T1(ii) の `json!("B:0 A:0")` と T1(i) の完全一致が落ちる．item 2 の call テストも同じ行を殺すので，行は 1 つだけ登録し killer をどちらかに決める（重複させない）．
2. **要素の無い方向にも欄を書く**（`elements.is_empty()` の枝を外し，`unwrap_or_default()` で `""` や既定 spec を書く）→ T1(iv) の `is_null()` と `set_subdev_spec tx` の不在が落ちる．
3. **今のモードで markup を書かない**（共通の代入を layout の枝だけに移す）→ T1(i)(ii)(iii) が落ちる．
4. **serial を書かない／別の値を書く**（`serial()` の結果を section に入れる式を定数 `"FAKE0001"` や `String::new()` に。`device.rs:377` の既定を使う形も同じ）→ T2 が落ちる．
5. **`rates_checked` を常に `true`**（比較していなくても）→ T3(iii) の `rx: false` が落ちる．
6. **常に `false`**（比べても）→ T3(i) の `rx: true, tx: true` が落ちる．
7. **rx と tx を取り違える**（方向の添字・キーの入れ替え）→ T3(ii) の `{"rx": true, "tx": false}` が落ちる．
8. **`false` の方向だけ欄を欠く**（`if compared { insert }`）→ T3(ii)(iii) の `json!(false)` 比較が落ちる（欠けは `Null`）．
9. **記録の時点が `arm` より前**（`from_binding` で「比べる予定」を書く）→ (i)(iii) では同じ値になるので**等価**（この2つでは捕まらない）．捕まえるには `arm` の途中で拒否される Run が要る（rates の集合違い）— item 4 の範囲なので行は作らず，記録だけ残す．

当て直しが要りそうな既存の行：P13（`mod.rs` の `if !front_end.starts_with(element.board.uhd_name())`）— front end の照合が spec の後に動いて文が変われば `old` を当て直す（item 3）．P199（`device.rs` の `Ok(self.config.serial.to_owned())`）— fake の serial の持ち方（instance ごとの既定）が変われば `old` と killer を当て直す（item 5）．J174（`mod.rs` の `id` の segments）は文がそのままなら当て直し不要．行を足すときは gate の教訓どおり，`old` の一致だけでなく名前を変えた識別子が `new` に残っていないかも grep する．

### (d) 頼る振る舞い

- **H1**（`set_subdev_spec` はその方向の channel の写像を作り直し，開いている streamer を切る．他方の方向は残る）— markup は「装置の channel がその順になる」文字列なので，記録は渡した文字列そのものでなければ意味が合わない．`from_binding` は streamer を開く前なので切る相手がいない．VERIFIED（brief の H1；fake 側の同型は `device.rs:836-846`）．
- **H2**（markup は `<db>:<fe>` を ASCII 空白 1 つで区切る．3 分割以上は失敗）— 記録がこの書式でなければ `set_subdev_spec` 自体が拒否する（`device.rs:601-644`；実 libuhd は `tests/uhd_api.rs:54-56`）．
- **H3**（DDC の rate の集合は chain がつながる前は `{1.0}`，streamer を作るときに grid になる）— 照合は「最初の streamer を開いた後」で，記録の `false` は「その時点で streamer を開かなかった」の意味になる．VERIFIED（brief の H3；fake の `rates_before_open` が前後を固定，`device.rs:864-877`）．spec の変更の後に古い値が残るかは INFERRED（BP0）．
- **H4**（同じ装置を 2 回開くと既存の `multi_usrp` が返り，2 つ目の spec が 1 つ目のものを黙って上書きする）— serial を Manifest に残す理由（拒否そのものは item 5）．VERIFIED（brief の H4）．
- **H5**（front end の名前は `UBX RX` のように，Basic/LF は番号を持つ）— 記録は名前ではなく layout の slot と front end の番号なので，装置の名前の揺れでは変わらない（fake の同型は `device.rs:435-442`）．VERIFIED（brief の H5）．
- **H7**（空の spec と重複を RFNoC は検査しないが fake は拒否する）— 要素の無い方向に空の spec を渡さず，記録も空文字にしないことの理由．VERIFIED（brief の H7）．
- INFERRED（UR-2 の `serial`: `crates/ezsdr-radio-uhd/design/18-uhd-radio.md:112`）: `serial()` は受信 channel 0 の `mboard_serial` であり，受信 channel の無い装置では失敗しうる．T2 は受信のある fake なのでこの経路は踏まない（item 5 の拒否が扱う）．

### 関わる他の順序

- **1（`keys` の値）**: どのモードが何を書くかを決める．**未決の一点**：brief は「今のモードでも markup は残る」とだけ明記し，「`keys` が無い instance は今のまま，ただし下の『subdev spec』と『prefix の制限を外す』は両方のモードに効く」とする一方，「同じ serial の拒否」と catalogue の開き直し（`crates/ezsdr-server/src/catalogue.rs:122-147`，今のモードの profile が通る）の文にはモードの限定が無い．T1 は今のモードの markup だけを固定し，`serial`・`rates_checked` を今のモードに書くかは固定しない（入っていても通る）．orchestrator が今のモードにも D5 を入れると決めたら，T3 に (iv) の 1 行を足す．
- **2（layout からの subdev spec）**: markup の文字列（順・区切り・空）は同じ．(c) の行 1・2 の killer は item 2 のテストのほうが強いならそちらに譲る（同じ行を二重に登録しない）．
- **3（front end の照合）**: spec を `channels`・`front_end` より先に設定する順序．T1 は記録だけを見る．(a) の候補（`fake.rs:169`, `:171` の文面）の当て直しは item 3 と final．
- **4（rate の集合の照合）**: 照合の判定（一致・`Rejected`）は item 4．T3 は「比べたかどうかの欄」だけを見る．(c) の行 5〜8 は T3 が killer．
- **5（同じ serial の拒否と解放）**: registry・拒否・fake の既定 serial・`ur_33_the_serial_is_the_config_s`（`fake.rs:2362`）と P199 の当て直しは item 5．T2 は明示の serial を使う．
- **6（prepare の stream 0）**: T3(i)(ii) が使う layout の profile/spec（`radio.{rx,tx}.c<n>.stream`，`radio.rx.s0.sample_rate_hz`）は item 6 の fixture．rx と tx のどちらが stream 0 を持つかで `rates_checked` が変わる．
- **12（宛先）・13（実行時の判定）**: `device` section には出ない（`UpdateParameter` の記録先は `applied`/`rejected`）．
- **15（VH-8）**: `device` section は UHD だけのもので Mock に相当が無い．VH-8 の比較対象（`effective`・SampleClock・拒否）に `device` section を入れないこと（入れると Mock 側に無い欄で落ちる）．
- **final**: UR-30 の改訂文（`device` の欄の一覧に `serial`・`subdev spec`・`rates_checked` を足す）と spec 18 UR-30 の *Test:* 行の更新．`set_subdev_spec` が増えて結果が変わる既存テストの候補：`ur_05_the_profile_must_be_the_device_s_front_ends`（`:169`, `:171` の文面）と `ur_33_the_serial_is_the_config_s`（`fake.rs:2362`，既定 serial）．calls を完全一致で見るテストと `device` section の完全一致の読者は無い（(a) の grep）．

## 15. VH-8 の比べ方

同じ `keys: "channel"` の selector と Spec（`Requested` の constraints，および同じ操作の列）を MockRadio と UhdRadio（FakeDevice）に与え，①`PrepareReport.effective`，②stream 0 の SampleClock の宣言（node と比），③同じ拒否，が一致することを直接比べる harness を作る（指示書「順序」15，設計 33 §6 の比較方針 `plan/phase8/33-channels-and-streams.md:273`）。期待値の規準は RM-30（`design/07-radio-model.md:341`「`effective` は RM-27 の configuration key ちょうど」）と RM-29（`design/07-radio-model.md:324`，stream の node），RM-25（`:257`，origin は根の格子）。**stream は方向ごとに 1 本だけ**（複数 stream への拡張と generator の layout 化は 1b-4e）。`c<n>.stream → 1` 以上は Mock が受理（移動，1b-3c）し UHD が拒否する段階なので，比較ケースに入れない（順序 10 が UHD 側だけを固定する）。

**置き場と形（推奨）**

- 新ファイル **`crates/ezsdr-acceptance/tests/parity.rs`**。理由: **PO-8** が「Module crate は他の Module crate を通常依存でも dev 依存でも取らない」「Module を2つ組めるのは `ezsdr-acceptance` だけ」と定め，`ma_03_no_module_crate_depends_on_another` が強制する（`plan/phase2/00-overview.md:142-144,157`）。両 Provider を1つのテストで並べる既存の場所もここ（`crates/ezsdr-acceptance/tests/x3xx.rs:1-4`，`:77-111`）。依存は既にある（`crates/ezsdr-acceptance/Cargo.toml:18,23`）。
- 退けた案: `crates/ezsdr-radio-uhd/tests/parity.rs`＋`ezsdr-mock-radio` の dev-dependency（PO-8 に反し `ma_03…` が落ちる）。`tests/fake.rs` に足す（同じく Mock を取れない）。
- **provider レベルで駆動する**（coordinator を通さない）。TM-13a の「宣言」は `declare_sample_clock` の中だけにあり，record は登録まで出ない（`crates/ezsdr-kernel/src/time/domain.rs:357-389`，`:400-424`）。`effective` も `PrepareReport` の戻り値が最も直接（Mock `crates/ezsdr-mock-radio/src/lib.rs:2568`，UHD `crates/ezsdr-radio-uhd/src/provider/mod.rs:404`）。rig の形は `crates/ezsdr-radio-uhd/tests/fake.rs:535-568`（`direct_prepare`）・`:597-701`（`Direct`）と `crates/ezsdr-mock-radio/tests/mock_layout.rs:199-268`（`harness`）の写し（PO-8 のため共有できない）。watchdog だけは path 取り込み（`crates/ezsdr-acceptance/tests/uhd.rs:9-11` と同じ。harness 構築の前に `watch()`: `uhd.rs:35-38`，R05）。
- **両者の根を 200 MHz にそろえる**（X310 の master clock: `crates/ezsdr-radio/src/x3xx.rs:19`。FakeDevice の既定 200 MHz: `crates/ezsdr-radio-uhd/src/device.rs:370`）。Mock の harness は根を 1 GHz で作っている（`mock_layout.rs:215`）ので，200 MHz の写しを作る。すると Mock の比 `root_rate × N / 200 MHz = N`（`lib.rs:1490-1498`）と UHD の `N`（`mod.rs:398-399`）が同じ数になり，root tick の instant も正規化なしで比べられる（UHD の尺度は `crates/ezsdr-radio-uhd/tests/common/mod.rs:44` の `ms`）。
- 共通の入力: selector は `keys:"channel"`, `rx`, `tx`, `id:"radio"` を共有し，UHD にだけ `args` を足す（Mock は `args` を unsupported で拒否: `lib.rs:399-410`。UHD は必須: `mod.rs:124-137`）。constraints は同じ map を両者に渡す（`coerce` は両者とも同じ `channels::Description`: Mock `crates/ezsdr-mock-radio/src/profile.rs:185-190`，`crates/ezsdr-radio/src/channels.rs:576-626`）。使う `Requested.resource` は両者 `radio`（不一致は拒否: `channels.rs:581-586`）。
- 記録 `Record`（両者の共通の形）: `effective: BTreeMap<Key, Value>`，`declared/Sorted: (node, 比)`（`ClockRegistry::declared_sample_clocks()`，`domain.rs:387-389`），`clocks: Vec<(node, 比, origin, ended_at)>`（`sample_clock_records()`，`domain.rs:425`），`applied: Vec<(具体の key, 値)>`（Mock の行は `value`: `lib.rs:1052-1058`。UHD の行は `claimed`: `crates/ezsdr-radio-uhd/src/provider/core.rs:533-550`），`rejected: Vec<(kind, reason)>`（Mock `lib.rs:961-968`，UHD `core.rs:439` の `reject_note`）。action queue の node 解決は Mock の `node_of`（`mock_layout.rs:52-58`）と同じ形（UHD 側は順序 12 の harness 変更で入る path 解決を使う: `fake.rs:586-593` が今は `usrp` 固定）。event の pairs に stream node（`radio/rx/s0`・`radio/tx/s0`）を入れる（`fake.rs:627` の写しに足す。`RX_OVERFLOW`・`LATE_COMMAND` の handle がその node で resolve される: Mock `lib.rs:2548-2560`）。

### (a) 既存のテスト

- 両 Provider を1つの selector から並べる既存の唯一の場所: `crates/ezsdr-acceptance/tests/x3xx.rs:78` `d3_the_two_providers_describe_one_layout_alike`（capability・既定値・`coerce` の拒否の一致），`:114`（channel 数），`:134`（受信の無い layout），`:153`（CBX の coherent），`:185`（layout は node でない）。**description までで，`prepare`/`effective`/SampleClock は比べていない**——この段階がそこを埋める。
- VH-8 の harness の形（同じ入力を2つに与えて記録を比べる）: `crates/ezsdr-mock-radio/tests/timeline.rs:117` `rm_26_the_timeline_against_mockradio`（1 000 列，層1），`crates/ezsdr-radio-uhd/src/provider/control/differential.rs:1`（層2），`crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:1`（層3）。generator は今の key の列しか作らない（`crates/ezsdr-radio/tests/generator/mod.rs:1-6`，`Change` は `Channels`/`Rate` だけ: `:43-50`）ので，layout の列の generator 化はしない（1b-4e）。
- Mock 側の期待値（VH-8 の基準）: `crates/ezsdr-mock-radio/tests/mock_layout.rs:432`（`keys` の拒否と両モードの非交差），`:482`（`effective` のちょうど RM-27 の key 集合と既定値），`:545`（`mock/rx/s0` の SampleClock の node と比），`:601`（D14 の宛先），`:662`（stream ごとの `cold`），`:832`（実行時の RM-31/RM-32 拒否），`:999`（instance 全体の床），定数の形は `:30-34`・`:215`。
- UHD 側の近いテスト（今のモード）: `crates/ezsdr-radio-uhd/tests/fake.rs:723` `ur_12_a_tx_only_layout_declares_no_receive_clock`（channel の無い方向は clock を宣言しない），`:873` `ur_15_every_clock_starts_on_its_lattice`（record の比），`:1516` `ur_25_rx_channels_from_zero_starts_at_its_instant`（0 から有へ。keys のモードでは node が `usrp/rx/s0` になる），`:1535`・`:1559`（準備項と「拒否は何も変えない」），`:704` `ur_12_prepare_refusals`，`:1960` `ur_30_the_sections_are_written`。
- 共有の builder（比較の土台で，動かさない）: `crates/ezsdr-radio/tests/channels.rs:152`（key 宣言），`:208`（layout 解析），`:320`・`:417`（tree），`:481`（`coerce`），`:598`（RM-31），`:693`（RM-32），`:897`（RM-19 channel ごと）。実装は `crates/ezsdr-radio/src/channels.rs:355-405`（`new`），`:524-552`（既定），`:557-566`（`stream_channels`），`:576-626`（`coerce`），`:727-731`（`violations`），`:776-819`（RM-31 の文面），`:934-943`（`is_configuration`），`:965-967`（`default_stream`）。
- 今のモードの振る舞いを固定するもの（結果を変えない）: `fake.rs:704`，`:723`（今の key でも `rx: []` は受信 clock なし），`:1960`，`:2032` `ur_33_the_front_end_table_follows_the_subdev_spec`，`:2073` `ur_33_a_subdev_spec_uhd_would_refuse_is_refused`，`x3xx.rs` の5本。`set_subdev_spec` の追加で device calls の完全一致が変わるテストの一覧は「計画の最後」の担当（final）。
- 現在の規則の文面（比較の期待値の根拠）: UR-12（`design/18-uhd-radio.md:191`。宣言の node は `<id>/rx`・`<id>/tx` のままで，`<id>/rx/s<k>` への書き換えは final の一覧），UR-13（`:196`），UR-25（`:246`），UR-26（`:249`），RM-25/29/30（`design/07-radio-model.md:257,324,341`）。

### (b) 足すテスト

すべて `crates/ezsdr-acceptance/tests/parity.rs`。実行: `cargo test -p ezsdr-acceptance --test parity vh_8_`。

**(b-0) 道具（parity.rs の中）**

- `MockRig`: `mock_layout.rs:199-268` の写し（200 MHz の根，`ManualTimeAuthority`＋`ClockRegistry`，`Queue`＋`node_of`，link `rx0`）。`prepare` の `PrepareReport` を保持する（今の harness は `effective` だけを保持: `mock_layout.rs:268`）。`arm_start(T0)`・`step(t)` は `mock_layout.rs:292-301` と同じ。
- `UhdRig`: `fake.rs:535-568`・`:597-701` の写し（`DeviceAuthority`＋`FakeDevice` `ClockMode::Manual`，`watch()`，link は port `rx0`）。`Direct::build` は `prepare` の戻り値を捨てている（`fake.rs:657`）ので保持する。`Fragment` は `fake.rs:647-653` の形（Mock 側は `mock_layout.rs:260-266`）。
- 変換: `applied` の行は `(具体の key, 値)` に正規化する。UHD の今の行は direction の key＋`channel` の形（`core.rs:533-550`）なので，**keys のモードの行は具体の channel key（`radio.rx.c0.frequency_hz`）にすることを順序 6/7 に求める**（Mock と同じ形でないと比較できない。順序 13 が timed の行を具体の key で書くのと同じ）。`at` は明示の `at` を持つ timed の更新だけで比べる（UHD の untimed の行の `at` は device を呼んだ実時刻: `core.rs:533-550`）。
- `rejected` の比較: 共有の規則（`RM-` で始まる文面）が理由の後半にあればその部分を完全一致で比べ，Provider 独自の規則（`MR-16`・`MR-18`・`UR-21`・`UR-26`）は **subject（どの Action が拒否されたか）だけ**を比べる（文面の接頭辞は設計上違う）。

**(b-1) `vh_8_the_two_providers_prepare_one_layout_alike`**

同じ selector＋同じ constraints を両方に与えて `prepare` し，記録を比べる。ケース表:

1. `rx ["0:A:ubx:0","0:B:ubx:0"]`，`tx ["0:A:ubx:0"]`，constraints 無し → `effective` は RM-30 の既定（`channels.rs:524-552`）で等しく，`declared` は `{"radio/rx/s0": 200}`（1 Msps，200 MHz 根）だけ（tx は既定でどの channel も stream に入らない: `channels.rs:965-967`）。
2. 混在の rx（`mixed()`，`mock_layout.rs:426`）＋ `tx ["0:A:ubx:0","0:B:lf_tx:0"]`，constraint `tx.c0.stream=0` → channel ごとの既定（UBX 1 GHz，BasicRX 0 Hz，antenna 集合: `channels.rs:860-881`）と `declared` の `rx/s0`・`tx/s0` が一致。FakeDevice の `front_ends` を layout に合わせる（`device.rs:335-383`，Basic/LF の名前は `device.rs:435-442`）。
3. 1 に `c0.frequency=1.1e9`, `c1.frequency=1e6`, `c1.antenna="A"`, `c1.stream=-1`, `s0.rate=10e6` → `effective` が map として完全一致（`declared` の比は 20）。
4. `radio.rx.c2.gain_db`（channel が無い）→ 両方 `coerce` が同じ `(key, reason)` で拒否（`channels.rs:638-645`，"RM-4: no such channel c2"）。`prepare` も両方拒否（Mock `lib.rs:2372-2375`，UHD `mod.rs:330-333`）。
5. `c0.frequency=7e9`（UBX の範囲外）→ 両方同じ理由で拒否（`crates/ezsdr-radio/src/device.rs:473-493`，"RM-8: … is outside …"）。
6. RM-31: `tx.c0.stream=0` と `rx.c0.antenna="TX/RX"` を同時に → 両方同じ理由で拒否（`channels.rs:776-819`，"RM-31: receive channel c0 of `UBX` cannot use antenna TX/RX while transmit channel c0 is in a stream"）。

assert: 受理のケースは `mock.effective == uhd.effective` と `mock.declared == uhd.declared`（node と比の map）。ケース 1・3 は期待値そのもの（既定値，比 200/20）も assert し，両者がそろって間違う場合を殺す。拒否のケースは `coerce` の `rejected` 列が完全一致（同じ `(key, reason)`）で，`prepare` も両方拒否。
観測: `PrepareReport.effective`，`ClockRegistry::declared_sample_clocks()`，`CoerceReport.rejected`。

**(b-2) `vh_8_the_same_receive_actions_leave_the_same_record`**

前提: rx 2 channel（`c0.stream=0` 既定・`c1.stream=-1` 既定），tx 2 channel で `tx.c1.stream=0`（slot B を流すので RM-31 が rx c0 に触れない: `channels.rs:792-797`），両方に port `rx0` の link（Mock `mock_layout.rs:229-237`，UHD は順序 6 の `rx0`），`T0 = 2 s + 1 ms`（`common/mod.rs:44` の `ms`）で arm/start。
列（`at` を全部明示し，床・期限を当てにしない。両者を同じ instant まで進めてから push する）:

1. `radio.rx.c0.frequency_hz=2.0e9` hardware_timed at T0+300 ms
2. `radio.rx.s0.sample_rate_hz=10e6` cold at T0+400 ms
3. `radio.rx.c1.antenna="TX/RX"` cold at T0+500 ms（stream 0 に居ない channel → 値だけ）
4. `radio.rx.c0.antenna="TX/RX"` cold at T0+600 ms（stream 0 の channel → cut）
5. `radio.rx.c0.frequency_step_hz=1.0` cold（class の無い key）
6. `radio.rx.c0.frequency_hz=7e9` hardware_timed（範囲外）
7. `radio/rx`（今の node）宛て `radio.rx.c0.gain_db` 更新
8. `start_rx` を `radio/rx/s1`（channel 無しの stream）宛て，`TxBurst` を `radio/tx/s1` 宛て（waveform は `ctx.inputs` に入れる: `fake.rs:642` の写しを広げる）

assert:
- `applied` の `(key, 値)` 列が等しい（1〜4。`at` は 1 でだけ比べる）。
- `sample_clock_records()` の `(node, 比, origin, ended_at)` 列が等しい。期待する列: `tx/s0`（arm で登録，1 channel），`rx/s0` の 1 本目（origin T0，2 の cut で終了），2 本目（2 の cut＋準備項の後の格子，4 の cut で終了），3 本目（4 の新しい segment。3 は stream を切らないので clock は増えない）。各 segment は最初の block まで進める（RM-25 の登録: Mock `lib.rs:1301`。UHD は uhd-rx の最初の block）。
- `rejected` は 5〜8 の4件（8 の `start_rx` は拒否されない）: 両方同じ Action を拒否し，6 は理由の `RM-` 以降が一致。5・7・8 の `TxBurst` は subject だけ。8 の `start_rx` の後で stream 0 が続いていること（clock が増えない・停止しない）。
- `LATE_COMMAND` は両方 0 件。
観測: 両者の `sample_clock_records()`，`applied`/`rejected` section，`COMMAND_REJECTED` event。
比べない: block の時刻と中身，device の calls，`apply` の read-back，Manifest の `timing`/`bursts`（Mock に device が無い，または順序 14）。

**(b-3) `vh_8_the_same_transmit_stream_clock`**

前提: `rx []`，`tx ["0:A:ubx:0","0:B:ubx:0"]` で `tx.c0.stream=0`，link 無し。arm の後，両方に `tx/s0` の clock が 1 本（比 `N`）出る。
assert: node と比の一致，および**各 provider で `origin == lattice(arm の instant, 比)`**（`design/07-radio-model.md:257`。arm の実時刻は両者で同じ tick にならないので，値そのものは比べない: Mock `lib.rs:2592-2598`，UHD `mod.rs:434-439`）。
列: `radio.tx.c0.stream=-1` cold at T0+300 ms → 両方 cut（`ended_at` が同じ instant）で新しい clock は無し。`radio.tx.c0.stream=0` cold at T0+400 ms → 両方 cut して新しい clock（origin は同じ式なので一致を assert）。
観測: clock records。

### (c) 壊れうる形と mutation の行の案

`old` の正確な文字列はコードと一緒に決める。`crate: "ezsdr-acceptance"`，`test_bin: "parity"`，`filter` は下のテスト名。

| # | 壊す規則と形 | 殺す assert |
|---|---|---|
| 1 | **UHD が `effective` に載せる key の集合を今のモードのままにする**（`keys::CONFIGURATION`（`crates/ezsdr-radio/src/lib.rs:110-120`）を使い `channels::is_configuration`（`channels.rs:934-943`）を使わない。今の行は `mod.rs:345`）→ layout の key が1つも載らない | (b-1) の 1・3（`effective` の map 比較。1 は既定値そのものも assert するので既定すら落ちる） |
| 2 | **宣言の node を stream の node にしない**（`mod.rs:398` の `id.child(dir.name())` を残す） | (b-1) の `declared` の map，(b-2) の clock record の node |
| 3 | **宣言の比を今の key の rate から取る**（`s0.sample_rate_hz` でなく `radio.{dir}.sample_rate_hz` 不在で 0/1 になる） | (b-1) の 1・3（比 200/20 の期待値），(b-2) の clock record の比 |
| 4 | **new segment の clock を登録しない／古い clock を終えない**（`end`/`register` の呼び出しを落とす） | (b-2) の clock record 列（本数・`ended_at`），(b-3) の `ended_at` |
| 5 | **class の無い key の更新を受ける**（`channels::update_class`（`channels.rs:944-956`）でなく今のモードの判定を使う） | (b-2) の 5（UHD だけ拒否しなくなる） |

**行を起こさない（equivalent / 別の item が殺す）と記録するもの**:

- channel ごとの device `apply` の値（1 channel の値の繰り返し，streamer の channel 列）は **item 15 の記録では見えない**（Mock に device が無い。UHD の行は要求値を写すだけ）→ 順序 6 の device calls のテストが殺す。
- UHD だけの拒否（subdev spec，`rates` の照合，serial，front end の照合，read-back の拒否）は比較に入れない（Items 2〜5・9 の担当。Mock に対応物が無い: `plan/phase8/reviews/1b4c` の各 plan の申し送り）。
- `→ 1` 以上の membership・複数の stream・移動（RM-34）はこの段階で両者が違うので入れない（順序 10，1b-4d/e）。
- instance 全体の床（RM-25）は列が `at` 明示の単発なので踏まない（順序 11 の担当）。
- **両者が同じ誤りを共有する場合（`channels::Description` 自体の誤り）はこの比較では落ちない**——item 15 は「ずれの網」であり，正しさの根拠は 1〜14 の spec 側のテストである。この限界を harness の doc comment に書く。

### (d) 頼る振る舞い

- 表 H1〜H7 のうち使うもの: **H5**（front end の名前。Basic/LF は番号を持つ: `db_ubx.cpp:562`，`db_basic_and_lf.cpp:101-111`。VERIFIED）は (b-1) の混在ケースの FakeDevice 構成に。**H6**（antenna の getter は設定値を読む。`plan/phase8/33-channels-and-streams.md:200`。VERIFIED）は (b-2) の 4 で read-back が拒否にならない前提に。**H1〜H4・H7 は使わない**（subdev spec は順序 2，serial は順序 5，`rates` は順序 4。item 15 は UHD 側のこれらの規則を前提にするだけ）。
- 表に無いもの（根拠と確かさを付けて足す）:
  1. **両 Provider の layout の description は同じ builder**（`channels::Description`，`channels.rs:355-405`）から作られる（Mock `crates/ezsdr-mock-radio/src/profile.rs:62-64`。UHD はこの段階で同じ builder へ）→ `effective`・`coerce`・`violations`・既定値が同じ関数から出る。**VERIFIED**（コード）。拒否の文面の完全一致と `effective` の一致の根拠。
  2. **X310 の master clock は 200 MHz**（`crates/ezsdr-radio/src/x3xx.rs:19`），FakeDevice の既定も 200 MHz（`crates/ezsdr-radio-uhd/src/device.rs:370`），Mock の根も 200 MHz にできる（`mock_layout.rs:215` の写し）。**VERIFIED**（コード）。比と instant を正規化なしで比べられる根拠。
  3. **受信の新しい segment の準備項は両者同じ**: `lead` は `timeline::start_lead_ns`（`crates/ezsdr-radio/src/timeline.rs:742-747`，rx/tx 50 ms，`x3xx.rs:458-468`），`call` は max(block_len, packet)=2000（Mock `crates/ezsdr-mock-radio/src/profile.rs:267-272`；UHD `control.rs:195`＋`device.rs:1114-1118` と既定 `rx_packet: None`＝0），`allowance` 3 ms（Mock `profile.rs:275-280`；`crates/ezsdr-radio-uhd/src/profile.rs:13`）。**VERIFIED**（コード）。違うのは `min_timed_command_lead_ns` だけ（2 ms と 5 ms: Mock `profile.rs:116-122`，UHD `profile.rs:24-30`）なので `at` は十分先（+300 ms）に置く。(b-2) の origin 一致の根拠。
  4. **明示の `at` の cold 変更は，instance の床と start より先なら `at` に効く**（Mock `lib.rs:1892-1905`。UHD はこの段階の実装）。Mock は **VERIFIED**（コード），UHD は **INFERRED**（この段階の実装が RM-25 に従うときだけ成り立つ。成り立たなければ (b-2) の clock record 列が両者でずれて落ちる）。
  5. **受信 clock の登録は RM-25 の最初の block**（Mock `lib.rs:1301`。UHD は uhd-rx の最初の block）→ 各 segment を block 1 つ以上進める。**VERIFIED**（コードと `design/07-radio-model.md:257`）。
  6. **(b-3) の tx clock の最初の origin は `arm` の instant の次の格子**（`design/07-radio-model.md:257`。Mock `lib.rs:2592-2598`，UHD `mod.rs:434-439`）。**VERIFIED**（コード）。両者の arm の実時刻は同じ tick にならないので，式に対する assert にする。

### 関わる他の順序

- **1（`keys`）**: この harness の入力の前提（Mock は既存 `lib.rs:388-394`，UHD はこの段階）。`keys` の値の拒否そのものは文面が違うので比較に入れない（Mock `mock_layout.rs:447-450`，UHD は順序 1）。
- **2〜5**: UHD だけの規則。harness は FakeDevice を layout に合わせるだけで，「同じ拒否」の一覧には入れない。
- **6（`prepare` の stream 0）**: (b-1)・(b-3) は順序 6 の実装の観測そのもの（`effective`・宣言・arm の登録）。channel の列（`rx_open`）は device call なので比べない（順序 6 の担当）。
- **7（channel ごとの timed）**: (b-2) の 1 が VH-8 の1ケース。走っている stream に入っていない channel の扱い（順序 7 の T2 の列: `~/.cache/ezsdr-fixes/...` ではなく対象のテストは各 plan。値の記録は比較対象）を足すなら 15 の表に1行足す。
- **8（rate）・9（antenna）・10（membership）**: (b-2)/(b-3) が両者で同じ列を回す。UHD 側の個別の assert（device calls，read-back の fault，`→1` の拒否）は各 item の担当。
- **11（instance 全体の床）**: 15 の列は床を踏まない。床の比較は順序 11 の UHD テストと Mock の `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change`（`mock_layout.rs:999`）の並置で足りる（plan-11 の裁定待ち）。
- **12（宛先）**: (b-2) の 7 が拒否側，8 が channel の無い stream の扱い（plan-12 の §関わる他の順序が「Mock 側にテストが無い」として 15 に送った2件をここで拾う）。受理側（`rx/s0`・`tx/s0` 宛て）の細部は順序 12。
- **13（実行時の判定）**: (b-2) の 6 が共有規則（`RM-` 以降の一致）を両者で見る1ケース。RM-31/RM-32 の実行時拒否を1行足すなら 15 の表へ（理由の完全一致が可能: `channels.rs:809-855`）。
- **14（Manifest）**: 15 は section（`applied`・`rejected`）と clock record まで。`device` section の markup・serial・`rates` の記録は順序 14 の担当で，UHD 側だけの比較になる。
- **final**: `parity.rs` を VH-8 の層として名指す文（`design/18-uhd-radio.md:322` の一覧，spec 22 の VH-8）と UR-12 の宣言の node（`design/18-uhd-radio.md:191`）の書き換えは final の一覧へ。mutation の当て直し（`P281` 以降の番号）も final。
- **1b-4e**: この harness の複数 stream 化（stream ごとの thread・node・宛先），generator の layout 化（`crates/ezsdr-radio/tests/generator/mod.rs:43-50` に antenna と stream を足す），移動（RM-34）の比較。

## 16. 計画の最後に書くこと

この項目は新しい規則を実装するのではなく、段階の出口で**機械的に確かめる一覧**を決める。以降は (a)〜(d) を、指示書の4つの丸（①今のモードの不変のテスト一覧と、`set_subdev_spec` で結果が変わる既存テスト／②書き換える `design/` の文／③当て直す mutation の行／④ 1b-4d・1b-4e へ送るもの）に対応させる。(a) が①、(b) が②＋変わる検査の置き換え、(c) が③、(d) が①の依拠する振る舞い、「関わる他の順序」が④。

### (a) 既存のテスト

**A1. `keys` なし（今のモード）の不変を固定するテスト。** まず母集団を検索で確定する：

- `grep -rn '"keys"' crates/ezsdr-radio-uhd/` → **0 件**（UHD 側には `keys` を渡すテストがまだ1つも無い。既存の UHD 側テストは全部が今のモードを固定している）。
- `grep -rn '"keys"' crates/ezsdr-radio/src/` → 0 件（語彙側にも `keys` は無い。追加は `ezsdr-radio-uhd` の selector の許容 key と `from_selector` 呼び分け＝この段階の実装）。

そのうえで、この段階が触る機構ごとに、最も近い（＝1行も変えてはならない）テストを挙げる：

| 機構 | テスト（path:line） |
|---|---|
| 開く・説明・refusal 文面 | `crates/ezsdr-radio-uhd/tests/fake.rs:96` `ur_05_from_binding_refusals`、`:139`／`:184` `ur_09_…`、`:491` `ur_10_instance`、`:510` `ur_11_coerce_is_the_descriptions` |
| prepare・arm・start | `fake.rs:704` `ur_12_prepare_refusals`、`:723` `ur_12_a_tx_only_layout_declares_no_receive_clock`、`:737` `ur_12_the_applied_values_are_recorded`、`:751`／`:761` rate の照合、`:810` `ur_13_arm_cases`、`:822`／`:840` `ur_14_…`、`:859` `ur_15_start_cases`、`:873` `ur_15_every_clock_starts_on_its_lattice`、`:598`（`crates/ezsdr-radio-uhd/src/provider/mod.rs` 内の unit `ur_15_a_second_stream_is_refused_rather_than_half_started`） |
| 実行時の receive/tx | `fake.rs:944`〜`:1000`（ur_17）、`:1070`〜`:1293`（ur_21〜23）、`:1315`〜`:1455`（ur_24）、`:1461`〜`:1770`（ur_25/26）、`:1788`〜`:1955`（ur_27〜29）、`:1960` `ur_30_the_sections_are_written`、`:1973` `ur_31_…` |
| 1b-4b で入った stream 状態の固定 | `fake.rs:3667` `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message`、`:3688` `ur_26_a_second_stop_does_nothing`、`:3792` `ur_26_the_sending_threads_join_before_the_receiving_ones`、`:3815`／`:3856`／`:3873`／`:3904` `ur_25_…` |
| 単体の rig（`from_binding` を通らない＝`set_subdev_spec` の増加の影響が無い） | `src/provider/control.rs:1030,1070,1116,1153,1208,1237,1252,1311,1372`、`src/provider/rx.rs:785,801,842,852,1029,1050,1128,1171,1189,1194,1290`、`src/provider/tx.rs:854-857,916,977,1225,1246,1266` |
| VH-8 の層2・層3（booking の不変） | `src/provider/control/differential.rs:85` `rm_26_the_timeline_against_uhd_control`、`src/provider/rx/differential.rs:111` `rm_26_the_timeline_against_uhd_rx`（どちらも rig で `from_binding` を呼ばないので、新しい `set_subdev_spec`/`serial` の影響を受けない。床の共有関数化で計画が変わらないことを、この2本がそのまま固定する） |
| FakeDevice 自身（`from_binding` を通らない） | `fake.rs:1982,2002,2023,2032,2073,2116,2190,2219,2232,2266,2290,2310,2339,2370,2383`、`src/device.rs` の unit 群（mutation の `filter` が名指すもの） |
| Mock の今のモード | `crates/ezsdr-mock-radio/tests/mock_radio.rs` の全テスト（`:309` を除く。下記 A3）と `tests/timeline.rs`、`tests/mock_channel.rs` |

出口の検査：実装後に `git diff HEAD -- crates/ezsdr-radio-uhd/tests/fake.rs crates/ezsdr-mock-radio/tests/mock_radio.rs crates/ezsdr-mock-radio/tests/timeline.rs crates/ezsdr-mock-radio/tests/mock_channel.rs` を取り、**hunk を1つずつ「A3 の3件のどれか」と対応づける**。どの hunk もこの3件に帰属できなければ不変違反である（1b-3b の報告 §9-7 と同じ形）。

**A2. `device.calls()` を完全一致で見るテストは無い。** 実行した検索：

- `grep -rn "assert_eq!(device.calls()" crates/` → 該当は `src/provider/tx.rs:857` の1件のみで、比較対象 `sent` は同じテストが組立**後**に取った snapshot（`:854`）なので、`from_binding` 時の新呼び出しでは壊れない。
- `grep -rn "calls()\[0\]\|calls().first()\|calls().last()\|calls\[0\]" crates/ezsdr-radio-uhd/` → **0 件**。
- 呼び出し列を前方一致・`position`・`before..`スライスで見るテスト（`fake.rs:1481,1491-1493,1508-1511,1562-1571,1605-1606,1703-1704,1757-1758,3695,3865-3867`、`rx.rs:1189,1194`）は、増える呼び出しが `from_binding` の先頭に付くだけなので順序・件数が変わらない。

したがって「calls の完全一致で結果が変わる既存テスト」は **0 件**であり、変わるのは A3 の3件だけ、と計画に記録する。

**A3. 結果が変わる既存テスト（意図した3件、すべて列挙する）。**

1. `fake.rs:162` `ur_05_the_profile_must_be_the_device_s_front_ends`（unit）。いまの期待は、要素数が装置に足りない場合 `"UR-5: profile x3xx needs 2 rx channels; the device has 1"`（`:169`）と `"UR-5: …needs 1 rx channels; the device has 0"`（`:171`）。新しい順序（layout の要素があれば `set_subdev_spec` を先に呼び、`channels`・`front_end` は後で読む）では、どちらも先に `set_subdev_spec` が失敗し、理由は `UR-5: uhd_usrp_set_rx_subdev_spec: … Could not find radio …` になる（フェイクの失敗文面は `crates/ezsdr-radio-uhd/src/device.rs:625-628,638-641`）。`:167,168,170,176-178` は要素ごとに確かめて**文面が変わらない**（同じ装置で `set_subdev_spec` が成功してから同じ front_end 名を読む）。すなわち hunk はこの2つの assert に限る。
2. `fake.rs:2362` `ur_33_the_serial_is_the_config_s`。`fake(FakeConfig::default()).serial()` が `"FAKE0001"` という期待（`:2365`）は、既定 serial を instance ごとの一意値にする変更で偽になる。同じ値の2つの `FakeDevice` から2つの生きた `UhdRadio` を作るテスト（例：`fake.rs:141` の radio を保持したまま `:154` でもう1つ作る）が衝突しないための変更で、理由をテストのコメントに書く。
3. `crates/ezsdr-mock-radio/tests/mock_radio.rs:309` `mr_02_from_binding_refusals`。prefix 制限（`crates/ezsdr-radio/src/x3xx.rs:567-580`）を外すので、`:341`・`:342`（`0:B:ubx:0` 単独）と `:350-363`（"position … names slot" の3 assert）は偽になる。コメント `:331-334` も追随する。なお空 layout の拒否（`:367-369`）は `"the layout names no front end"`（`x3xx.rs:558`）が担うので**残る**。

補足：`crates/ezsdr-radio-uhd/tests/hardware.rs`（`#[ignore]`）は gate の対象外だが、`set_subdev_spec` を実機に送るので BP0 の確認事項に一行残す（`plan/phase8/00-overview.md` の送り先はオーケストレータ）。

### (b) 足すテスト

**B1. 変わる検査の置き換え（A3 の3件それぞれに、置き換え後の検査を足す）。**

- `ur_05_the_profile_must_be_the_device_s_front_ends` 用：`set_subdev_spec` の失敗が `Rejected` に出て UHD の文面をそのまま含むこと、および `have < need` の理由が**どの経路で残るか**を固定する新しい検査（例：装置に slot B が無い／layout が装置に無い front end を名指す、の2分岐）。置き場所は `crates/ezsdr-radio-uhd/tests/fake.rs`（`ur_05_…` の隣）。
- `ur_33_the_serial_is_the_config_s` 用：既定 serial が2つの instance で異なること、明示 `serial` はその値、`from_binding` が**失敗した**ときは登録が残らないこと（同じ `FakeDevice` で拒否のあと成功できる）。同じ `FakeDevice` を共有する連続呼び出しは既存の `fake.rs:96` が踏むので、登録リークがあればそこで落ちる（(a) A1 の一覧で守る）。
- `mr_02_from_binding_refusals` 用：非 prefix の layout（`rx=["0:B:ubx:0"]`、`rx=["0:B:ubx:0","0:A:ubx:0"]`）が受理され、`Selection` の要素順が channel 順（slot B が channel 0）になることを固定する。Mock 側は `crates/ezsdr-mock-radio/tests/mock_radio.rs`、UHD 側は `fake.rs`（順序 2 の計画が持つ検査と同じ規則を、両 Provider で1本ずつ）。

**B2. 書き換える `design/` の文（各規則と一行の変更内容）。**

| 文 | 変更 |
|---|---|
| `design/18-uhd-radio.md:66-97` UR-2 | 明細は原則そのまま。`:96` の「a call's channels are the device's own numbers under its current subdev spec」が既に新しい意味を持つので、`*Test:*`（`:97`）の `ur_33_*` から serial テスト名の説明を新しい検査に合わせる。本文の変更が要るかは実装 diff と同時に確認し、無変更ならその旨を Changes に書かない（DA-1：根拠は decision record 側）。 |
| `:124-141` UR-5 | ①「its elements are a prefix of UHD's default channel order…」の句（`:124`）を削る（`x3xx::from_selector` の制限解除）。② `set_subdev_spec` を開くときに呼び、そのあと `channels`・`front_end` を読む、を追加。③ refusal 一覧に「同じ serial の生きた `UhdRadio`」と「`serial()` の失敗」、および `set_subdev_spec` の失敗（UHD の文面をそのまま含む）を追加。④ key 表（`:129-141`）に `keys`（`"channel"`）の行を追加（Mock の `design/09-mock-radio.md:35` と同じ形、値は `"channel"` のみ、他は `UR-5`）。 |
| `:171-186` UR-9 | `keys: "channel"` の instance の description は `channels::Description`（RM-29/RM-30）であることを1文追加。今の文は `x3xx` の description だけを語る。 |
| `:187-189` UR-10 | layout モードの tree・port（`<id>/rx/s<k>`、`rx<k>`）と configuration capability、`effective` が RM-30 の key になることを追加。 |
| `:191-195` UR-12 | fragment の受ける port は layout モードで `rx0`。`effective` はモードで切り替え。channel 数は `Description::stream_channels` で stream 0 の集合として読み、**stream 1 以上に入る設定を拒否**（この段階は stream 0 だけを動かす理由を文面に含める）。 |
| `:196-197` UR-13 | streamer は stream の channel **列**で開く。`rates` の集合照合を「方向ごとに最初の streamer を開いた後」に行い、不一致は `Rejected`（UR-13）、**比べたか／比べていないかを Manifest に残す**、`arm` で streamer を開かなかった方向は比べない、を追加。 |
| `:207-213` UR-17 | `rx_recv`・`rx_packet_samples` が stream の番号を取る（この段階は 0）、event の source が `<id>/rx/s0` であることを追加。 |
| `:218` UR-20 | `RX_OVERFLOW` の handle を **stream ごとに** resolve（`<id>/rx/s0`）と直す。 |
| `:241-244` UR-24 | layout モードの `radio.{rx,tx}.c<n>.{frequency_hz,gain_db}` が `hardware_timed` で、timed の更新が channel `n` にだけ効くこと（`apply` の宛先が channel）を追加。 |
| `:246-248` UR-25 | ① layout モードの instance 床（RM-25）は `ezsdr_radio::timeline` の**1つの関数**で、両 Provider が呼ぶ（`mech:radio-rule-implemented-twice` の解消）。今のモードの床は自分の stream の line のまま、を区別して書く。② 宛先 `<id>/rx/s<k>`・`<id>/tx/s<k>`。③ `rx_packet_samples(0)` 等の番号は stream の番号。 |
| `:249-253` UR-26 | ① layout モードの `radio.{rx,tx}.c<n>.antenna` は `cold`（D16）で、今のモードの `radio.{rx,tx}.antenna` は今どおり拒否。② `Stop` は `<id>`＝全 stream、`<id>/d/s<k>`＝その stream。③ layout モードの instance は `<id>/rx`・`<id>/tx` を宛先に**持たない**。 |
| `:265-268` UR-30 | `device` section に「方向ごとの spec の markup」「serial」「rate の照合の結果（比べた／比べていない）」を追加。`applied`・`timing`・`rejected` に新しい行（per-channel の `apply`、membership の拒否、`open` の照合）を追記。 |
| `:276-291` UR-33 | 既定 serial が instance ごとに異なること（並列のテストが衝突しないため）、`from_binding` が `serial`・`set_subdev_spec` を呼ぶこと、device の channel が開いた spec のものであることを追記。 |
| `design/07-radio-model.md:295-296` | 「`ezsdr_radio::timeline` computes RM-16, RM-21 and RM-25 once: **from a stream's initial state**, …」は、関数を1か所に移すと **floor について偽**になる（instance 床の入力は1つの stream の状態ではなく、instance のまだ効いていない `cold` 変更全部）。共有関数とその入力をこの段に足す。`*Test:*`（`:296`）に新しい関数の timeline テストを名指す。 |
| `design/09-mock-radio.md:153-154` | MR-18 の「Under `keys`, `e` is first floored at every earlier, still-pending `cold` change of the instance…」は、規則の本体を RM-25（と共有関数）に移すので、**規則の再掲をやめて引用＋Mock 自身の機構（受信した時点の receipt、`colds`、muted line）だけ**にする（DA-1：topic は1か所）。`:154` の「Its cut and next origin come from `ezsdr_radio::timeline`」に floor も加える。 |
| `plan/maintenance/22-timing-simplification.md:200-212` VH-8 | Decision 本体は履歴なので、床が共有関数になった1文と、harness を stream の次元へ広げるのが 1b-4d・1b-4e である1文を**この段階の記録**（変更表）に足す。`design/` の VH-8 引用（`design/18-uhd-radio.md:322,443,445`、`design/09-mock-radio.md:336`、`design/07-radio-model.md:470`）は文面据え置きで、共有関数のテスト名を足すときだけ追随する。 |
| Vision | **この段階では触らない**（1b-5）。触らない根拠の grep：`grep -n "radio.rx.channels" design/vision/02-spec-binding-and-compilation.md` → `:24`、`:28`、`:54`（「one key per direction」）および `grep -n "radio.rx.sample_rate_hz" design/vision/03-simulation-mock-and-time.md` → `:71` のまま。1b-5 で扱うことを記録に残す。 |
| `plan/phase8/33-channels-and-streams.md:266`（1b-4 行） | 1b-4c の実装内容と、4d へ送り直す一覧（下記）を追記する。 |

**B3. `design/18` §6 の行。** 足すテスト・変えるテストごとに §6 の表へ行を足す（表の体裁は `:339` の `ur_05_the_profile_must_be_the_device_s_front_ends` 行と `:460` の `ur_33_a_subdev_spec_uhd_would_refuse_is_refused` 行に倣う）。各規則の `*Test:*` に行を足すのは規則に新しいテストを名指すときだけ（変更の grep で確認：`grep -n "ur_33_the_serial_is_the_config_s" design/18-uhd-radio.md` → `:97`（UR-2 の Test）、`:286`（UR-33 の Test）、`:450` 付近の §6 行）。

**B4. grep で確かめること（出口の証拠にする検索）。** `grep -rn '"keys"' crates/ezsdr-radio-uhd/` → 新しい分岐だけが指す／`grep -rn "default channel order" crates/` → 0 件（`x3xx.rs:569,575` の削除、`mock_radio.rs:333` のコメント書き換え）／`grep -rn "position 0 names slot\|position 2 names slot" crates/` → 0 件／`grep -rn "updates.iter().filter" crates/ezsdr-mock-radio/src/lib.rs` → 0 件（床は共有関数へ）／`grep -rn "Channels::first" crates/ezsdr-radio-uhd/src/provider/` → 16 件から「stream の集合を読む」経路だけへ置き換わる（今のモードは従来どおり）。

### (c) 壊れうる形と mutation の行の案

`old` はコードと一緒に決める（既存通り）。以下は**当て直しが要りそうな既存行**で、実行した検索は `python3` で `plan/*/tools/mutations.json` を `file`/`old` について走査した（上記の表はその出力）。

| 行（list, id） | 何が動くか | 案 |
|---|---|---|
| maintenance P141, P142, P149（`crates/ezsdr-mock-radio/src/lib.rs:1899`） | 床の inline 計算が共有関数の呼び出しになる。`old` が消える | `old` を共有関数側（例：`ezsdr-radio/src/timeline.rs` の新関数）へ移し、kill するテスト（`mr_18_a_layout_*`, `mr_40_a_move_uses_the_instance_floor_*`）は不変。`rust-uhd` 側の新しい floor 行を足すので、**同じ規則に2つの行**ができないよう片方に寄せる |
| maintenance P08, P09（`crates/ezsdr-radio/src/x3xx.rs:575`） | prefix 制限のブロックごと削除。行が当たらない | 行を引退し、変わりに「非 prefix を拒否しつづける」交差検証の変異を作らない（規則自体が消えるため）。kill テスト `mr_02_from_binding_refusals` の期待も B1 で変わる |
| maintenance P13, phase7 R22, R26（`provider/mod.rs:184` 前後） | front-end ループの前後に `set_subdev_spec` が入り、`have < need` の位置も動く | `old` を行の残る検査に当て直す（P13 は「front end の一致を見ない」、R26 は「要素の位置を見ない」形を維持）。kill テストの期待文面の変更は B1 |
| maintenance J64, P251（`control.rs` の `booked_stream(dir)`） | 宛先が stream を持つようになり `booked_stream` の引数が変わる可能性 | `old` を新しい呼び出し形へ。`new` に旧識別子が残らないよう grep（1b-4b gate の教訓 U41） |
| maintenance J68, J69, J71, U20, R21（`channels_at`） | stream ごとの channel 数に一般化されると `channels_at(dir, t)` が stream 引数を取る | 同上。`new` 側の `Core::channels(&self.config_at(…))` が新 API 名になる場合は `new` も書き換える |
| maintenance P252, P253（`from_none`） | stream 引数が増える／判定が stream 0 固定でなくなる | 追随 |
| maintenance P263, P264（`booked_stream(Dir::Tx)`、`self.settings(&updates, …)`） | stream 番号・config 経路の一般化 | 追随 |
| phase7 B15, B22（`rx_packet_samples(0)`）、maintenance U41（`self.colds`） | `rx_packet_samples(k)`・床の実装位置が変わる | 追随。U41 は共有関数へ移るなら `old` ごと移す |
| maintenance P247（`provider/core.rs` の `for dir in [Dir::Rx, Dir::Tx]`） | stream 集合の反復が `count` の意味ごと変わりうる | 追随（今のモードは `k=0` だけ） |
| maintenance P199（`device.rs` の `serial()`） | 既定 serial が instance ごとになる | **注意**：mutant が `Ok("FAKE0001")` 固定だと新しい既定値では kill できない。新しい既定（atomic counter 等）に合わせた固定値へ |
| maintenance P245, P273, P274, P275（`mod.rs` の stop 順・thread 名） | 1b-4c では thread 名は `uhd-rx`/`uhd-tx` のまま（1b-4b の送り一覧 (1) は 4d） | 変更なしで残ることを確認。`start` の本文が書き換わるので `old` の前後が動いていないかだけ確認 |

新しく足す行（番号は実装時に採る）：`keys` の値の検査（`"channel"` 以外の拒否）、selector に `keys` を許す表、`set_subdev_spec` の markup 生成（順序・空白1つ・要素の無い方向は呼ばない）、開くときの照合の順序（spec → channels → front_end）、`arm` の rate 集合照合と Manifest の記録、serial の登録／`Drop` の解除、channel ごとの `apply`（周波数・利得・antenna と read-back 拒否での halt）、`c<n>.stream` の `-1`/`0` 限定の拒否、instance 床の共有関数、宛先（`rx/s<k>`・`tx/s<k>` と `<id>`）、実行時 `violations`。**行の一致は「ちょうど1回」を工具で確認**し、実行は `remote_mutate.py` だけ（`all sim02:8,sim03:8` を gate で）——書き換える `crates/ezsdr-radio/src/timeline.rs` の行は `rm_26_*`/`ur_*` の kill まで含めて。

### (d) 頼る振る舞い

指定の表 H1〜H7 をそのまま使う（この項目は新しい依拠を足さない）。関係づけだけ書く：H1（`set_subdev_spec` が方向の channel map を作り直し、その方向の streamer だけ切る）＝順序 2 の「開くときに1回だけ設定」の根拠、H2（markup は `<slot>:<front-end>` を ASCII 空白で）＝markup 生成、H3（DDC 出力の rate 集合は chain がつながる前 `{1.0}`）＝順序 4 を `arm` に置く根拠（`FakeDevice` の `rates_before_open` は `crates/ezsdr-radio-uhd/src/device.rs:353-356,864-877`）、H4（同じ装置を2回開くと既存の `multi_usrp` が返り、2つ目の spec が黙って上書きする）＝同じ serial の拒否（順序 5）の根拠、H5＝front end 名の照合（順序 3）、H6（antenna の getter は設定した property）＝D16 の read-back（順序 9）、H7（空・重複の spec を RFNoC は検査しない。fake が拒否＝厳しい側）＝markup 生成に空を渡さないこと。UHD に対して **INFERRED** のまま残るものも列挙する：serial が受信 channel 0 の `mboard_serial` であること（`design/18-uhd-radio.md:113`）、BP0 で測る事項（rate 集合の実測、同じ serial の別プロセス挙動）。

### 関わる他の順序

- 順序 1（`keys` の値）〜15（VH-8）はすべてこの項目の (a)〜(c) に1本ずつ対応する。とくに (a) A3 の3件は順序 1・3・5 の (b) の欄と一致するはずで、計画間で食い違ったら Sol のレビューで潰す。
- **1b-4d へ送る一覧**（設計 33 §5 の 1b-4 行「1b-4c へ送るもの」(1)〜(11) を送り直す。ただし (2) の D14 の宛先と (5) の channel 列の読みは、この段階が stream 0 について行うので、4d は**その一般化**として受ける）：
  1. stream ごとの thread 名 `uhd-rx-s<k>`・`uhd-tx-s<k>`、同名全 thread の join、`rejected` の `thread` の形（UR-16）。
  2. `booked_stream` の stream 一般化（今は stream 0）と `rx_packet_samples(k)`（この段階は `k = 0` のみ）。
  3. `<id>/d/s<k>` の node・SampleClock の stream id・event の source を k ≥ 1 に広げる（この段階は stream 0 だけ）。
  4. 複数 stream での `Streams::end` と VH-8 harness の stream 次元。
  5. `delivered` の床が「その line と同じ stream」であることを2本目の受信 stream で確かめるテスト。
  6. `Lines::prune_stream` と RM-34（moves）。
  7. UR-15・UR-17・UR-25・UR-26 の呼び出しの形と番号を全 stream へ（この段階の `[PreparedStream; 1]` ガードと stream-0 固定は 4d で外す）。
  8. Manifest の plan に stream の番号と segment ごとの論理 channel 列を持たせる（Sol の minor 2）。
  9. 拒否が他方の stream を変えないテストを「item のある line」で作り直す（Opus の minor）。
  10. mutation の `old` の一致に加え、名前の変わった識別子が `new` に残っていないかを grep（gate の U41）。
- **1b-4e へ送る**：channel の移動（D3、RM-34）と、複数 stream の VH-8 harness（この段階の stream 0 版を stream 次元へ）。
- **1b-5 へ送る**：Vision の key の記述（`design/vision/02-spec-binding-and-compilation.md:24-28,54`、`design/vision/03-simulation-mock-and-time.md:71`）と、古い key・今の builder・一時的な `keys` の削除（1b-3b 報告 §11 の 6・8 と同じ送り先）。
- 段階の gate：`rules.py --base HEAD`、`remote_mutate.py all sim02:8,sim03:8`、`cargo test`/Python はオーケストレータが段階の終わりに流す（この項目自体はテストもコードも足さない）。

## 実装で増えた経路（実装役が足したもの．指示書 3）

計画・裁定が名指していないが，実装が持ち込んだ状態と失敗の経路．いずれもテストと mutation の行がある（id は `plan/maintenance/tools/mutations.json`．P335・P336 は本節で追加）．

1. **`keys` の読みを 1 関数にした**（`ezsdr_radio::channel_keys`，`crates/ezsdr-radio/src/lib.rs`）．Mock と UHD の両方がこれを呼ぶ．テスト：`ur_05_every_other_keys_value_is_refused`（fake），`rm_38_the_keys_selector_is_read_once_for_both_providers`（device），`mr_38_a_layout_instance_uses_one_key_set`（mock_layout）．行：P122（当て直し），P282，P283．
2. **key のモードは enum `Mode` が description ごと持つ**（`provider/mode.rs`）．`coerce`・`tree`・`defaults`・`effective` が別々の description を選ぶ形（裁定 1 の C4・C5）は，この形では構造的に作れない．行：P281，P284〜P291，P303〜P306，P316，P329（mode の各判断），P333（filter）．
3. **receive の port 名は `Mode::rx_port`**（`rx` / `rx0`）．テスト：`ur_12_a_layout_s_stream_zero_declares_its_clock_and_opens_its_channels`（`rx0` の link が無いと prepare が拒否する）．行：P336．
4. **あきになった stream の `cold` 変更は applied の行を 1 本残す**（値だけ，`read_back: null`）．テスト：`ur_25_an_antenna_change_of_a_channel_in_no_stream_only_records`．行：P335．
5. **`check_rates` は方向ごとに 1 回だけ比べ，結果を instance に持つ**（`Core::rates`）．テスト：`ur_13_arm_compares_the_rate_set_after_the_streamer_opens`（`rates` の call が各方向 1 回），`ur_30_the_device_section_records_the_rate_check`．行：P292，P295，P296，P334．
6. **serial の登録は guard の値で持ち，`Drop` は device に触れない**．テスト：`ur_05_a_radio_dropped_after_its_device_is_lost_releases_its_serial`，`ur_05_a_parallel_bind_of_one_serial_succeeds_once`．行：P297〜P302（W10 の「Drop が device を読み直す」形は guard が device を持たないので作れない．理由を報告に書いた）．
7. **channel の集合は `Channels` で持ち，開き直しは集合の比較**（`rx.rs` の `opened`，`tx.rs` の `channels`，control の from-none）．テスト：M1 の `ur_25_a_same_size_membership_swap_reopens_the_streamer`（rx と tx），T1/T2/T3．行：P317，P318，P319．
8. **`channel_live` は timed の更新の宛先を effective instant の membership で決める**．テスト：`ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`．行：P310，P312．
9. **`leaves` は「空になった stream でも記帳する」ことを membership の leave に限る**．テスト：`ur_25_the_last_receive_channel_leaves_no_clock_and_no_block`．行：P319．
10. **`front_end_matches` と `layout_elements`・`subdev_markup` は mod.rs の関数**．テスト：`ur_05_*` の当該群．行：P284〜P291．
11. **rate の集合の比較は純関数 `ezsdr_radio::device::rate_set_matches`**（範囲の展開と集合化）．テスト：`ur_13_the_rate_set_comparison_expands_ranges_and_ignores_order`（device），`ur_13_arm_takes_the_rate_set_in_any_order`．行：P294．
12. **あきになった stream の `cold` 変更でも runtime の `violations` は投影で判定する**（13 の本体）．テスト：`ur_25_a_layout_instance_s_*_rule_is_judged_at_runtime`．行：P327〜P329．
13. **`prepare` の stream ≥ 1 の検査と runtime の検査は同じ `Mode::channel_in_later_stream`**．行：P308，P309（2 か所の配線），P322（宛先の stream 数の範囲）．
