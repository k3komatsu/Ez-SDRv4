# 実装の指示書：Phase 8 段階 1b-4c（UHD Module の新しい key のモード：開くときの照合，channel ごとの設定，stream は方向ごとに 1 本）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `4199e51`，clean）．

**この段階は 2 回に分けて起動する（AGENTS.md §9．リスクのある段階：装置との境界〈開くときの照合〉，同一性〈serial〉，寿命〈serial の登録と解放〉，queue と期限〈床と timed command〉が変わる）：**
- **1 回目（今回）**：テスト計画だけを書く．コードと tree には触れない．
- **2 回目**：Sol のレビューで直した計画をもとに実装する（別の指示で起動する）．

## 段階の切り分け（orchestrator の決定，2026-10-19）

設計 33 §5 の 1b-4c と 1b-4d を，レビューの量を抑えるために 3 つに分ける：
- **1b-4c（この段階）**：
  - selector の `keys: "channel"` のモード．
  - layout からの subdev spec と，開くときの照合．
  - 同じ serial の拒否．
  - channel ごとの設定・`apply`・timed の更新．
  - antenna の `cold`（D16）．
  - D14 の宛先．
  - instance 全体の床（RM-25）．
  - 実行時の `violations`．
  - **stream は方向ごとに 1 本（stream 0）だけを動かす**：channel を stream 1 以上に入れる設定は拒否する．
- **1b-4d**：方向ごとに複数の stream（D15 の stream ごとの thread と名前，stream ごとの node・event・overflow の handle，Manifest の plan の stream 番号）．1b-4b から送った一覧（設計 33 §5 の 1b-4 行「1b-4c へ送るもの」(1)〜(11)）は 4d へ送り直す．ただし (2) の D14 の宛先と，(5) の channel 列の読み（`Channels::first(n)` を stream の channel 集合に）は，この段階で stream 0 について行う．
- **1b-4e**：channel の移動（D3，RM-34）と，複数 stream の VH-8 harness．

## 最初に読むもの

- `AGENTS.md` を全部．特に §2 の DA-1，§3，§6，§7，§9．
- 設計 `plan/phase8/33-channels-and-streams.md`：§3，§4 の D3〜D5，D8，D9，D14〜D16，§5 の 1b-3 と 1b-4 の行．
- spec：
  - `design/18-uhd-radio.md` の UR-2，UR-5，UR-9〜UR-13，UR-20，UR-24〜UR-26，UR-30，UR-33．
  - `design/07-radio-model.md` の RM-25，RM-27〜RM-33．
  - `design/09-mock-radio.md` の MR-18，MR-38〜MR-40．
- 参考：MockRadio で同じモードを入れた commit `aeaf48f`（1b-3b．`git show aeaf48f`）と，その報告 `plan/phase8/reviews/1b3b-report.md` の §11 の 4・9．
- コード：
  - `crates/ezsdr-radio-uhd/src/provider/`（`mod.rs` の `from_binding`/`prepare`/`arm`，`core.rs` の `Core::settings`/`channels`/`configure`，`control.rs`，`rx.rs`，`tx.rs`）
  - `crates/ezsdr-radio-uhd/src/device.rs`（`Device` の trait と `FakeDevice` の `set_subdev_spec`・`rates`・`serial`・`front_end`）
  - `crates/ezsdr-radio/src/channels.rs`，`crates/ezsdr-radio/src/x3xx.rs`（`from_selector` の prefix の制限）
  - `crates/ezsdr-radio/src/timeline.rs`（`command_instant`）
  - `crates/ezsdr-mock-radio/src/lib.rs`（layout のモードの `addressed`，`cold_streams`，床の計算 `:1892-1905`）
  - `crates/ezsdr-server/src/catalogue.rs:110-150`（装置を開く場所と UR-7 の開き直し）

## この段階でやること（2 回目に実装する中身）

- **`keys` の切り分け**（MR-38 と同じ）：
  - `keys` が無い instance は今のまま．ただし下の「subdev spec」と「prefix の制限を外す」は両方のモードに効く．
  - `keys: "channel"` の instance は `channels::from_selector` の layout と `channels::Description` を使う（tree，capability，port，key，`coerce`，既定値）．
  - それ以外の値や型は `Rejected`（UR-5）．
  - 1 つの instance が新旧両方の key を受けることはない．
  - MockRadio の `Profile` の `Mode` と同じ形を目安にする．同じ判断を 2 か所に書かない．`cold_streams` と宛先の判定を `ezsdr_radio` に移して両 Provider が使えるなら，そうする（1b-3b の報告 §11 の 4）．
- **subdev spec**：
  - `from_binding` は，layout の要素が 1 つ以上ある方向ごとに `set_subdev_spec(dir, markup)` を呼ぶ．`channels`・`front_end` はその後に読む．
  - markup は要素を論理の順に `"<slot>:<frontend>"` にし，ASCII の空白 1 つで並べる．
  - 要素の無い方向は設定しない．
  - device の channel の番号は，論理 channel の番号と同じになる．
  - `x3xx::from_selector` の「UHD の既定の順の prefix」の制限（`x3xx.rs:567-580`）を外す．
- **開くときの照合**（D5）：
  - front end の名前（Basic/LF は front end の番号まで）．
  - **rate の集合は `arm` で，方向ごとに最初の streamer を開いた後に**比べる．開いた stream の 1 channel の `rates` が description の格子と集合として違えば，`Rejected`（UR-13）．
  - `arm` で streamer を開かなかった方向は比べない．比べたかどうかを Manifest に残す．
  - 周波数と利得の範囲の照合（D8．C API の追加が要る）は**この段階に入れない**（後の段階で，feature `uhd` の mutation の扱いと一緒に）．
- **同じ serial の拒否**（D5）：
  - `from_binding` で `serial()` を読み，プロセス全体の登録に入れる．登録は `UhdRadio` の `Drop` で外す．
  - 生きている `UhdRadio` が同じ serial を持っていれば `Rejected`（UR-5）．`serial()` の失敗も `Rejected`．
  - `FakeDevice` の既定の serial は instance ごとに違う値にする．並列のテストが衝突しないため．
- **channel ごとの設定**（keys のモード）：
  - stream 0 の channel 集合は `Description::stream_channels` で読む．streamer はその channel の列で開く．
  - rate は `s0.sample_rate_hz`．
  - 各 channel にその channel の周波数・利得・antenna を `apply` する．antenna も read-back（`Applied.antenna`）し，違えば拒否．
  - timed の更新（`radio.{rx,tx}.c<n>.{frequency_hz,gain_db}`）は channel `n` にだけ効く．
  - `radio.{rx,tx}.c<n>.antenna` は `cold`（D16）．
  - membership（`c<n>.stream`）は `-1` と `0` の間だけ受ける．1 以上は拒否する．
- **instance 全体の床**（RM-25）：
  - keys のモードの `cold` 変更は，その instance の先に受けたまだ効いていない `cold` 変更すべての effective instant を床にする．方向・stream・line は問わない．
  - MockRadio の layout のモードは同じ計算を inline に持つ（`lib.rs:1892-1905`）．`ezsdr_radio::timeline` に 1 つの関数として置き，両 Provider がそれを呼ぶ（`mech:radio-rule-implemented-twice`）．MockRadio の挙動は変えない．
  - 今の key のモードの床は，自分の stream の line のまま変えない．
- **宛先**（D14）：
  - `UpdateParameter` と `Stop` を `<id>/rx/s<k>`・`<id>/tx/s<k>` 宛てでも受ける．`TxBurst` は `<id>/tx/s0`，`start_rx` は `<id>/rx/s0`．
  - 何を受け，何を拒否するかは MockRadio の layout のモードと同じにする（VH-8）．
  - keys のモードの instance は，今の宛先 `<id>/rx`・`<id>/tx` を持たない．
- **node と event**：
  - keys のモードの stream 0 の SampleClock・event の source・`RX_OVERFLOW` の handle は `<id>/rx/s0`（送信は `<id>/tx/s0`）．
  - port は `rx0`．
- **実行時の判定**：更新の effective instant に投影した設定全体を `Description::coerce`（`violations` を含む）で判定する．`prepare` と同じ 1 か所．
- **変えないこと**：今の key のモードの挙動．例外は二つだけ：
  - `set_subdev_spec` の呼び出しが増える．prefix の layout なら UHD の既定と同じ spec．
  - prefix でない layout も受けるようになる．
  - Manifest・event・既存のテストの結果は HEAD と同じ．

## 守ること

`~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/brief.md` の「守ること」と同じ．要点：
- `cargo fmt` を使わない．
- mutation は `remote_mutate.py` だけで流す．
- commit・push・`git add` をしない．
- 2 分を超えるコマンドは nohup で流し，`kill -0` で待つ．
- テストを弱めない．
- 版を上げない．

加えて（1b-4b の gate の教訓）：既存の mutation の行を当て直すとき，`old` の一致だけでなく，名前を変えた識別子が各行の `new` に残っていないかも grep する．

## 頼る装置と UHD の振る舞い

| # | 振る舞い | 根拠 | 確かさ |
|---|---|---|---|
| H1 | `set_subdev_spec` はその方向の channel の写像を作り直し，開いている streamer を切る．他方の方向の streamer は残る | `multi_usrp_rfnoc.cpp:2822-2834`（UHD 4.10．docker image `ezsdr-v4-dev:uhd4.10` の `/root/tmp/uhd-4.10.0.0/host/lib/usrp/`） | VERIFIED |
| H2 | markup の書式は `<db>:<fe>` を空白で区切る．区切りは ASCII の空白だけで，3 分割以上は `uhd_subdev_spec_make` の失敗 | `subdev_spec.cpp:19-20,58-59` | VERIFIED |
| H3 | DDC の出力 rate の集合は，chain がつながる前は `{1.0}`．chain は streamer を作るとき（`get_rx_stream`）につながる | `ddc_block_control.cpp:153-159`，`multi_usrp_rfnoc.cpp:337,1354-1361,2714-2735` | VERIFIED（spec の変更の後に古い値が残るかは INFERRED．BP0） |
| H4 | 同じ装置を 2 回開くと，既存の `multi_usrp` が返る．2 つ目の binding の spec が 1 つ目のものを黙って上書きする | `multi_usrp_rfnoc.cpp:2902-2908` | VERIFIED |
| H5 | front end の名前は `UBX RX` などで，Basic/LF は `BasicRX (0)` のように front end の番号を持つ | `db_ubx.cpp:562`，`db_basic_and_lf.cpp:101-111` | VERIFIED（1b-4a） |
| H6 | antenna の getter は設定した property を読む．物理のスイッチを測るのではない | 設計 33 D16 | VERIFIED（1b-4a） |
| H7 | 空の spec と重複を，RFNoC の `set_rx_subdev_spec` は検査しない．FakeDevice は拒否する（厳しい側．UR-33 に明記済み） | `multi_usrp_rfnoc.cpp:2796` | VERIFIED |

## 1 回目の成果物：テスト計画（項目ごとに分けて起動する）

1 回目は，項目ごとに別の実行として起動する．順序 1〜15 に 1 本ずつ，「計画の最後に書くこと」に 1 本の，計 16 本．あなたが担当するのは，起動の指示が名指しした**1 項目だけ**．
- 読むだけにする（read-only で起動される）．ファイルは書かず，最後の回答を計画そのものにする．
- 書式（見出しをこのとおりに）：
  `## <番号>. <順序の名前>` / `### (a) 既存のテスト` / `### (b) 足すテスト` / `### (c) 壊れうる形と mutation の行の案` / `### (d) 頼る振る舞い` / `### 関わる他の順序`
- 根拠は必ず `path:line` で書く．「見つからない」と書くときは，実行した検索（grep のコマンドそのまま）を添える．

各順序について：
- (a) その振る舞いに今いちばん近い既存のテスト（`path:line`）．変えてはいけない今のモードの振る舞いを固定しているものを含む．
- (b) 足すテスト．名前，どこに置くか（lib か `tests/fake.rs` か），通す操作の列，assert，どう観測するか（device の calls，Manifest，event，plan など）．
- (c) 誤った実装の形．それぞれについて，(b) のどの assert がそれで落ちるか．mutation の行の案（壊す規則と形．`old` の正確な文字列はコードと一緒に決める）．
- (d) その順序が頼る UHD・装置の振る舞い（上の表の H1〜H7．表に無いものは，根拠と VERIFIED/INFERRED を付けて足す）．

### 順序（それぞれ，期待する結果）

開くとき：
1. **`keys` の値**：
   - 無い → 今のモードのまま．
   - `"channel"` → layout のモード．
   - 他の文字列や，文字列でない値 → `Rejected`（UR-5）．
   - layout のモードの instance は今の key（`radio.rx.frequency_hz` など）を `prepare` で受けない．今のモードは layout の key を受けない．
2. **layout から subdev spec を作る**：
   - rx `[0:A:ubx:0, 0:B:basic_rx:0, 0:B:basic_rx:1]`，tx `[0:A:ubx:0, 0:B:lf_tx:0]` → device の calls に `set_subdev_spec rx A:0 B:0 B:1` と `set_subdev_spec tx A:0 B:0` が，`channels`・`front_end` を読む前に入る．
   - 逆順の rx `[0:B:ubx:0, 0:A:ubx:0]` → spec は `B:0 A:0` で，channel 0 の front end は B の board．
   - 要素の無い方向 → `set_subdev_spec` を呼ばない．
   - 今のモードでも同じ（prefix でない layout も通る）．
3. **front end の照合が spec の後に行われ，外れを拒否する**：
   - 装置の slot B が UBX で，layout が `basic_rx` → `Rejected`．理由に方向と channel 番号を含める．
   - Basic/LF の front end の番号が違う → `Rejected`．
   - 装置に無い front end を spec が名指す（`set_subdev_spec` の失敗）→ `Rejected`．UHD の失敗の文面をそのまま含める．
4. **rate の集合の照合**：
   - `from_binding` では `rates` を呼ばない（H3）．
   - `arm` は，方向ごとに最初の streamer を開いた後に `rates` を比べる．一致すれば続ける．
   - FakeDevice に別の格子を返させる → `arm` が `Rejected`（UR-13）．streamer は `cleanup` で閉じられる．
   - 開いた後も `{1.0}` → `Rejected`（INFERRED：起きないはず．BP0）．
   - 受信の link が無く streamer を開かない方向は比べない．Manifest に「比べていない」と残る．
5. **同じ serial の拒否と解放**：
   - 同じ serial の 2 つ目の `from_binding`（同じ `Arc` の device でも，別の device でも）→ `Rejected`．理由に serial を含める．
   - 1 つ目を drop した後の同じ serial → 受ける．catalogue の UR-7 の開き直しの loop（`catalogue.rs:122-147`）は，1 つ目の radio を drop してから次の `from_binding` を呼ぶので，開き直しが自分に拒否されない．
   - `serial()` の失敗 → `Rejected`．
   - `from_binding` が serial を読んだ後の照合で失敗した → 登録は残らない．
   - 既定の FakeDevice どうし → 衝突しない．

stream 0 の設定（keys のモード）：

6. **`prepare` の stream 0**：
   - rx の layout が 2 channel で，`c0.stream=-1`，`c1.stream=0` → `rx_open(0, [1])`．SampleClock は `<id>/rx/s0`，port は `rx0`，rate は `s0.sample_rate_hz`．
   - 各 channel に自分の周波数・利得・antenna を `apply` する．device の calls の `apply rx 1 …` の値で見る．
   - channel を stream 1 以上に入れる設定 → `Rejected`．理由が，この段階は stream 0 だけを動かすことを言う．
7. **channel ごとの timed の更新**：
   - `radio.rx.c1.frequency_hz` を `at` 付きで → channel 1 にだけ timed の `apply` が 1 回．
   - 走っている stream に入っていない channel の更新 → 記録だけして device に渡さない．次にその channel が入る stream の設定で，untimed に適用する．
   - device の queue は channel・方向をまたいで 1 列（UR-24）．
8. **stream の rate の `cold` 変更**：
   - `radio.rx.s0.sample_rate_hz` → stream 0 だけが cut され，新しい segment の全 channel が untimed に設定される．その instant までに効く timed の更新を含む（MockRadio の MR-34 の「始まった segment は全 channel の untimed tune」と同じ）．
   - 新しい SampleClock が始まる．
9. **antenna の `cold` 変更**（D16）：
   - stream 0 に入っている `c<n>` の antenna → stream 0 が区切られ，新しい segment の設定で antenna が適用され，read-back が記録される．
   - read-back が違う（FakeDevice の fault が要る．#59 の read-back 拒否と同じ形）→ その変更は拒否され，stream が halt する（RM-25）．
   - stream に入っていない channel の antenna → 値だけが変わり，segment は区切られない．
   - 今のモードの `radio.{rx,tx}.antenna` の更新は，今どおり拒否（UR-26）．
10. **membership の変更**：
    - `c<n>.stream` を `-1 → 0`（入る）・`0 → -1`（出る）→ stream 0 が区切られ，新しい channel の列で streamer を開き直し，新しい clock になる．
    - 最後の channel が出る → stream に channel が無くなり，clock も block も無い．
    - `→ 1` 以上 → `COMMAND_REJECTED`．何も変わらない（plan，clock，Manifest）．
11. **instance 全体の床**（RM-25）：
    - 送信の rate の `cold` を `T+50 ms` で受けた後に，受信の antenna の `cold` を `T+10 ms` で受ける → 受信の変更は `T+50 ms` に効き，`LATE_COMMAND`（requested `T+10 ms`，applied `T+50 ms`）．
    - 今のモードでは今どおり自分の line だけが床．
    - MockRadio の layout のモードの同じ操作と同じ instant と event になる．
12. **宛先**（D14）：
    - `<id>/rx/s0` 宛ての `UpdateParameter`（key が変えるものを決める）・`Stop`・`start_rx`，`<id>/tx/s0` 宛ての `TxBurst` → 受ける．
    - `<id>` 宛ての `Stop` → 全 stream．
    - channel の無い `<id>/tx/s1` 宛ての `TxBurst`，`<id>/rx/s1` 宛ての `start_rx` → MockRadio と同じ扱い．
    - keys のモードで `<id>/rx` 宛て → 拒否．
13. **実行時の判定**：
    - 投影した設定が RM-31（全二重の受信 antenna は RX2）や RM-32（throughput）に反する更新 → `COMMAND_REJECTED`，何も記帳しない．
    - `prepare` で同じ設定を与えたときと同じ理由になる．
14. **Manifest**：`device` section に，方向ごとの spec の markup，serial，rate の照合の結果（比べた／比べていない）が残る．今のモードでも markup は残る．
15. **VH-8 の比べ方**：同じ keys のモードの selector と Spec を MockRadio と UhdRadio（FakeDevice）に与える → 同じ `effective`，同じ stream 0 の SampleClock の宣言（node と比），同じ拒否．stream 1 本の場合だけ（複数 stream は 1b-4e）．

### 計画の最後に書くこと

- 今のモードの振る舞いを変えないことを固定するテストの一覧（`path:line`）と，`set_subdev_spec` の呼び出しが増えて結果が変わる既存のテスト（device の calls を完全一致で見るもの）．
- 書き換える `design/` の文の一覧：
  - spec 18 の UR-2，UR-5，UR-9，UR-10，UR-12，UR-13，UR-17，UR-20，UR-24〜UR-26，UR-30，UR-33．
  - spec 07 と 09 で，床の関数を 1 か所に移すことで偽になる文．
  - spec 22 の VH-8．
  - Vision は 1b-5．
  - grep で確かめる．
- 当て直しが要りそうな既存の mutation の行（`old` が変わるもの，`new` に変わる識別子を持つもの）．
- 1b-4d・1b-4e へ送る一覧．

## 1 回目のセルフレビュー

**なし．** 計画は Sol がレビューする（AGENTS.md §9 の 3）．
