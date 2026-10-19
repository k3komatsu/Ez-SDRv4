判定：FAIL

device 状態の一記録への集約はできています。ただし、**TX の非同期 report の対応付けに回帰があり、版の主要系列もテストで固定されていません。**

独立に確認したシナリオは、control による streamer 置換後の旧 report、configure 後の投影変更、部分 configure 失敗、arm 後の同集合への切り替えです。busy・D1・競合検査は 1b の範囲として扱いました。

### 1. major — control が置換した TX streamer の旧対応付けが残る

**箇所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:671–675,1544–1559`

- **根拠：** `unacked` を消すのは、今回の `owner_configure` が reopen した場合だけです。control が from-none で既に開き直した場合、owner は `Nothing` と判断し、旧 streamer の対応付けを残します。native の `tx_open` は旧 handle を除去して新しいものに置換します（`crates/ezsdr-radio-uhd/src/uhd.rs:1010–1020`）。
- **系列：** 旧 burst の ACK 待ちを残して disable → owner が idle → control が from-none open → owner がその記録を採用 → 新 burst の ACK／TimeError。新 queue の report が旧 burst を消費し、TimeError の `target` と `late_by_ns` が旧 ClockDomain のものになります。実機再現は **INFERRED**、対応付けが残るコード経路は確認済みです。
- **修正：** 「owner が今回 open したか」ではなく、**report queue が置換されたか**で対応付けを破棄してください。単一記録に streamer の世代を置くなど、control 側の置換も owner が検出できる形が必要です。テストは実際に control の `tx_open` を通し、新 report が新 burst を指すことを検査してください。

変更された `ur_28_a_streamer_the_record_holds_keeps_its_correlations` は、実際の open なしで記録だけを書き換え、逆の期待を固定しています。**busy を足すだけでは、この非並行の系列は直りません。**

### 2. major — S11a・S11b が、設定省略の安全性を検査していない

**箇所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:1165–1180,1279–1300`

- **根拠：** S11a は開始済み RX に cold change を記帳しており、from-none configure を通りません。owner も動かしません。S11b も owner を動かさず、`all(...)` による gain 以外の apply の不在だけを見ています。apply が一件もなくても通ります。
- **系列：** from-none configure → 後着 timed gain → channel 不在で record-only release → owner begin。この中心系列について、版上昇も再 configure も device の最終値も未検査です。record-only の `changed()` を削除しても、現在の S11b は失敗しません。
- **修正：** 初期設定も `radio.rx.channels = 0` にしてください。現在の `control(&core, &[])` は初期 stream を空にするだけで、設定は `mode.defaults()` のままです（`:1098–1111`）。その上で 100 ms の enable、50 ms の gain、release、owner begin を順に駆動し、版上昇・timed apply なし・untimed 再 configure・最終 gain を検査してください。S11a は逆に、実際の from-none 後に timed release と owner begin を通し、再 configure なしを固定してください。

これは 1b の D1、つまり「owner が先に開始した場合」とは別の、**1a に割り当てられた系列の不足**です。

### 3. minor — 投影が変わらない更新でも版を上げている

**箇所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:625–626,663–665,1190–1200`

- **根拠：** record-only release と cancel は、全 `ColdConfig` の版を無条件に上げます。更新がその segment の origin より後、layout の別 channel、取消対象が存在しない場合も区別しません。
- **系列：** origin が約 1 s の segment に対して、1.5 s の gain を取消。origin の設定は変わりませんが、S11c 自体がこの場合の版上昇と `Owner::Configure` を期待しています。
- **修正：** segment の origin・channel に対する投影が実際に変わった場合だけ無効化してください。S11c は origin 前の更新取消を正例、origin 後／対象外 channel を「再 configure 不要」の負例にします。

裁定の「投影が変わり、device がその変化を受け取らないときだけ」と一致していません。

### 4. minor — arm の記録を削除しても S12 が通る理由は明確

**箇所：** `crates/ezsdr-radio-uhd/tests/fake.rs:4690–4704`

- **根拠：** 最初の RX は `UhdRadio::start` が直接 `rx_start` し、`Rx::new` に渡します。TX も arm の clock を直接受け取ります（`crates/ezsdr-radio-uhd/src/provider/mod.rs:653–671`）。初期 segment は `owner_configure` を通りません。
- **系列：** arm の `record_open` を削除して開始。現在のテストは open 各一回のまま通ります。P372 の生存は、この観測範囲から説明できます。
- **修正：** arm → start → **同じ集合の次の cold segment** → owner の begin／switch まで進め、追加 open がないことを RX・TX 両方で検査してください。S20 は `mod.rs` 内の unit test で、照合拒否後の `Unknown` と configured 不在を直接確認できます。

### 5. minor — S2 と S21 は、名前の系列を最後まで実行していない

**箇所：**
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:987–1004`
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1304–1320`

- **根拠：** S2 は control を使わず、owner 自身が全集合を順に開きます。「未開始 B の後に control が C を開く」という系列ではありません。S21 は control の open／apply までで、owner に次の enable を開始させません。
- **系列：** TX の古い B に設定済みの印を残したまま C が device を置換する誤実装や、拒否後の次 segment を owner が開始しない誤実装を、それぞれ検出できません。
- **修正：** S2 は RX の S1 と同様に control と owner を直接駆動し、B・C の採用前の再 open を検査してください。S21 は owner begin と `rx_start` まで確認してください。

### 6. minor — FakeFault の追加が明示的な裁定に反する

**箇所：** `crates/ezsdr-radio-uhd/src/device.rs:356–361,1065–1074`

- **根拠：** `TxApplyFails` と `RxChannelApplyFails` を追加しています。`adjudication.md:61` は、失敗注入を test 側の wrapper に置き、`FakeFault` を増やさないと明記しています。
- **系列：** S8・S25 のためだけの注入が、公開 fake の fault 語彙に残ります。
- **修正：** 指定どおり test wrapper に移してください。残すなら orchestrator の明示的な再裁定が必要です。

### 7. minor — spec 18 に新しい記録の規則が定義されていない

**箇所：** `design/18-uhd-radio.md:196,201,213,238,252–253`

- **根拠：** UR-17／UR-23 は「記録が configuration を保持する場合」と書き換わりましたが、その一致条件 `(seq, version)`、版の変更条件、失敗時の記録、prune の保持を定義していません。「拒否済み segment は device を呼ばない」という新規則も未記載です。
- **系列：** 成功済み configure の後に record-only 更新で版が変わった場合、UR-12 の限定列挙だけでは、再 configure する条件が読み取れません。
- **修正：** 記録と一致条件を一か所に定義し、UR-17／UR-23 から参照してください。新しい拒否規則と対応する Test 行も追加してください。

**確認できた点：** owner の照合は atomic な版読みで、`streams → updates` を追加していません。部分 configure 失敗は開集合を保持して configured を消し、prune は記録を保持します。保存済み rate 拒否と P358 の分岐も残っています。変更対象の二つの mutation JSON は、全 `old` が対象ソースにちょうど一回一致しました。

未実施：ビルド・テスト・mutation・実機。ファイル変更なし。busy と D1 の既知の窓は 1b の対象として未解決のままです。
