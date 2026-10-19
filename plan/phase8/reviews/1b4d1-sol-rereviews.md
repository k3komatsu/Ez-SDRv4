# 1b-4d-1a：Sol 6.1 の再レビュー

## 再レビュー 1

判定：FAIL

前回 major 2 は解消。major 1 は通常の switch では直りましたが、report 読み出し側に穴が残っています。

### 1. major — switch 前・Shutdown の report 読み出しでは世代を照合しない

**箇所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:149–150,684–688,715–718,790–800`

- **根拠：** 世代変更による `unacked` の破棄は `do_switch()` 内だけです。通常の loop は switch より先に `reports()` を呼び、Shutdown は switch を通らず `reports()` に入ります。
- **系列：** 旧 burst の ACK 待ち → disable → control が from-none open を完了 → owner が採用する前に通常 poll または Shutdown。置換後の queue から `TimeError` を読むと、旧 streamer の `target` を消費します。世代未照合の経路は確認済み。実機でこの窓に report が届くことは **INFERRED** です。
- **修正：** report を対応付ける前にも記録の世代を同期してください。通常 poll と Shutdown の両系列を、実際の from-none open と test wrapper の report 注入で固定してください。1b の busy だけでは、open **完了後**のこの系列は防げません。

### 2. minor — open 成功後の rate 照合拒否で世代が進まない

**箇所：** `crates/ezsdr-radio-uhd/src/provider/core.rs:298–302,700–706`、`crates/ezsdr-radio-uhd/src/provider/control.rs:855–860,882`

- **根拠：** 世代を進める条件が、open 自体の成功ではなく「open と rate 照合の両方の成功」です。native は照合より前に streamer を置換しています（`crates/ezsdr-radio-uhd/src/uhd.rs:1011–1020`）。
- **系列：** 保存済み rate 拒否 → `tx_open` 成功 → 保存済み拒否を再適用。queue は置換済みなのに世代が同じです。UR-36 の「every successful open」と一致しません。
- **修正：** open 成功と照合結果を分け、成功した open は世代を進め、照合拒否は集合・設定だけを Unknown にしてください。「open 自体の失敗では進まない」と合わせて検査してください。

### 3. minor — channel の濾しを単独で壊す誤りを検出できない

**箇所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:486–493,1246–1272`、`plan/maintenance/tools/mutations.json:7475–7482`

- **根拠：** S11c は current keys の origin 前後だけです。指示された layout の「別 channel」の負例がありません。P375 は origin と channel の条件を同時に消すため、origin の assert だけで殺せます。
- **系列：** `{0}` の segment に対し、origin より前の c1 更新を取消。channel 条件だけを削除しても現在の S11c は通ります。
- **修正：** 対象外 channel の取消で版不変・`Owner::Nothing` を確認し、channel 条件だけを外す mutation を追加してください。

### 確認できた点

- **通常 switch の世代同期：** control の実際の from-none open、owner 自身の reopen、open しない switch の保持をテストしています。
- **S11a／S11b：** from-none → timed booking → release → owner begin を通し、timed 発行と record-only 後の再 configure を区別しています。前回 major 2 は閉じています。
- **投影の範囲：** `changed_projections` 本体は origin・channel を濾し、`streams` を離してから `updates` を取っています。
- **S12／S20：** 同集合の次の cold segment まで RX・TX を進め、arm の照合拒否では Unknown・configured 不在を直接確認しています。
- **S2／S21：** control と owner を組み合わせ、TX の B・C 再 open、RX の拒否後の次 enable の begin まで進めています。
- **失敗処理：** open 自体の失敗では世代を進めず、部分 configure 失敗は開集合を保持して設定記録を消します。
- **失敗注入：** 新しい二つの FakeFault は除去され、test 専用 `ApplyFails` に移っています。
- **spec：** UR-36／UR-37 と UR-17／UR-23／UR-28 の参照・Test 行は追加済みです。ただし世代の実装不一致は指摘2のとおりです。
- **P372–P378：** 全 `old` が対象ソースにちょうど一回一致。各変異と指定テストの観測の対応を静的確認しました。kill 結果自体は未再検証です。
- **1b への接続：** busy・D1 option A・lock-order 検査を妨げる新しい構造上の問題は見当たりません。ただし report 読み出しにも世代同期が必要です。

未実施：ビルド・テスト・mutation・実機。ファイル変更なし。

## 再レビュー 2

判定：PASS

前回の指摘1〜3は解消。1b-4d-1a の範囲で、新たな指摘はありません。

### 確認できた点

- **指摘1：解消。** `crates/ezsdr-radio-uhd/src/provider/tx.rs:710–724`：`sync_streamer()` を switch と report の対応付け前で共有。通常 poll・switch 前の読み・Shutdown の ACK 読みはいずれも世代を照合します。`Stop` 後の読みも同じ経路です。
- **回帰テスト：適切。** `crates/ezsdr-radio-uhd/src/provider/tx.rs:1645–1693`：実際の from-none open 後、owner が新 segment を採る前に report を注入し、poll／Shutdown が旧 burst を指さないことを観測しています。
- **指摘2：解消。** `crates/ezsdr-radio-uhd/src/provider/core.rs:297–329`：open 成功なら照合拒否でも世代を進め、集合・設定だけを Unknown／なしにします。open 自体の失敗では進めません。
- **全 open 経路：整合。** `crates/ezsdr-radio-uhd/src/provider/control.rs:854–883`、`core.rs:729–749`、`mod.rs:587–598`：from-none・owner・arm は同じ記録規則を使います。
- **指摘3：解消。** `crates/ezsdr-radio-uhd/src/provider/control.rs:1300–1335`：`{0}` の segment の origin より前にある c1 更新を取り消し、版不変と `Owner::Nothing` を確認しています。
- **P376・P378–P381：適切。** `plan/maintenance/tools/mutations.json:7485–7543`：各 `old` は対象ソースに正確に1回一致。変異はそれぞれ失敗 open の集合、照合拒否の集合、report の世代照合、成功 open の世代、channel 条件を壊し、指定テストの assert に対応しています。
- **規範：整合。** `design/18-uhd-radio.md:257–263`：UR-36 の「every successful open」と実装は一致しています。
- **既存実行記録：確認。** `fix3-mutation.log:3–21` は対象行すべて killed、`fix3-test-workspace.log` は全66結果行で失敗0です。

未実施：ビルド・テスト・mutation の再実行と実機確認。実機での report 到着条件は **INFERRED**。ファイル変更なし。  
残る from-none／owner の競合窓は、裁定どおり1bの busy 対応範囲であり、このPASSはその解消を意味しません。
