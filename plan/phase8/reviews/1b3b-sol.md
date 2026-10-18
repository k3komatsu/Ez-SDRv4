指定 brief に従い、差分・仕様・テスト／mutation 行を read-only で独立レビューします。ファイル変更や mutation 実行は行いません。
`cold` の順序保証に問題を確認しました。membership の単純な出入りでも設定と line が食い違い得るため、移動を追加する次段階だけの問題ではありません。別 stream の予約済み更新が後続の予約で制約違反になるケースも追跡しています。
**受理は保留を推奨します。major 3件、minor 3件です。** 動的な再現は実行していないため、以下の実行結果は **INFERRED**（コード経路は確認済み）です。

### 1. major — membership が同値だと `cold` の床を失う
`crates/ezsdr-mock-radio/src/lib.rs:1667–1704,1783–1785`

初期状態を `c1.stream=1` とし、同じ receipt で順に：
1. `c1.stream=-1` を未来の `T+5 ms` に予約。
2. `c1.stream=1` を `at` なしで受信。

2 は要求時点で `was == is` となり、対象 line も床もなく今すぐ適用されます。その後1が効き、最終状態は `-1`。RM-25 なら2を `T+5 ms` に床付けし、最終状態は `1` です。

**修正:** 同値判定より先に対象 stream の床を決め、確定した effective instant で membership と記帳対象を再計算する。この反例は移動を使わず、1b-3c への延期理由は成立しません。

### 2. major — 後から挿入した更新が、予約済み更新を無効にしても再判定しない
`crates/ezsdr-mock-radio/src/lib.rs:1710–1743,1806–1809`

初期 `c1.stream=-1` で、先に `c1→s1 @ T+20 ms`、次に `c1→s0 @ T+10 ms` を予約すると、両方とも booking 時には「未所属からの追加」として受理されます。最終的に設定は `s1` なのに、両 line が c1 を保持し得ます。MR-40 の移動拒否も迂回します。

同じ穴は throughput にもあります。1 GbE、s0=20 Msps・c1未所属・s1=20 Msps で、順に：
- s0=5 Msps @+10 ms
- c1→s1 @+20 ms
- s0=20 Msps @+15 ms

各 booking は通りますが、+20 ms 後は **160 MB/s > 100 MB/s**。`apply_update` に再判定がありません。

**修正:** 更新の挿入で影響する後続予約も再検証し、configuration と各 line を整合させる。membership と throughput の両反例をテスト／mutation に追加する。

### 3. major — link のない RX stream は更新順を保持できない
`crates/ezsdr-mock-radio/src/lib.rs:1700–1704,1730,2234`

RX line は link のある stream にしか作られません。未接続 s1 に rate 変更を `@T+10 ms`、続いて `@T+5 ms` と予約すると、どちらも line がなく、後者が先に適用されます。RM-25 の arrival-order floor が働きません。

**修正:** layout モードでは、出力の有無と独立して各 stream の予約／床を保持する。未接続 stream の逆順予約を固定するテストと mutation が必要です。

### 4. minor — 旧モードの拒否文面が変わる
`crates/ezsdr-mock-radio/src/lib.rs:1327–1331`

`keys` なし・TX未設定で、誤った宛先に `TxBurst` を渡すと、HEAD は clock 検査から `"no transmit channel is configured"`、変更後は先行する宛先検査から一般的な `"target, clock, … is invalid"` になります。event payload と Manifest の観測差です。

**修正:** 旧モードでは従来の検査順を維持する。

### 5. minor — stream ごとの TX channel count をテストが固定していない
`crates/ezsdr-mock-radio/tests/mock_layout.rs:339–342,656–712`

TXテストは全 stream が1 channelで、waveform helper も1 channel固定です。layout の channel count を常に1とする誤実装を捕まえません。報告の「MR-16 の内側の target 検査は到達不能」は、channel-count 検査の mutation を省く理由にはなりません。

**修正:** 異なる channel 数の2 stream で、同じ byte size の解釈と拒否を確認し、対応する mutation を追加する。

### 6. minor — selector の既定値が新モードと不一致
`design/09-mock-radio.md:40`

`link_gbps` の既定を10としていますが、layout モードでは `hg` の既定は1です（RM-28）。既定 binding の envelope が文書の読みと異なります。また同ファイル`:84` は規範本文で node／port の定義をテスト名に委ねています。

**修正:** モード別の既定値を明記し、MR-4 は RM-29 を引用する。テスト名は `*Test:*` 行へ移す。

### 確認して問題なかった点
- 新旧 key set の分離、stream ごとの clock／port、fault の最小 running-stream 選択、event source、独立 jitter generator はコードと assertion を照合。
- 指定26 mutation 行の `old` は**全てちょうど1回一致**。当て直した7行の変更は `old/new` のみで、規則の壊し方も維持。
- 既存3テストファイルの diff は空。`git diff --check` も問題なし。
- 保存ログの **26/26 killed** は確認。ただしテスト・build・mutation の独立再実行はしていません。ファイル変更なし。
