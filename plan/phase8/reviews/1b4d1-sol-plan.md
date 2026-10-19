# 判定：要修正

device の状態を stream ごとの一記録に集約する方針は妥当です。ただし、**`seq` だけの設定同一性、操作権の範囲、競合テストの観測方法**が未確定で、この計画のまま実装には進めません。

以下の `brief.md`・`test-plan.md` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-19-phase8-1b4d1/` 配下です。

## 先に立てた独立シナリオ

brief の系列・test-plan を読む前に、既存コードから次を抽出しました。

- control の open 中に owner が旧 segment を開始する。逆に、owner の device 呼び出し中に control が streamer を置換する。
- open 失敗で旧 streamer も失われる。複数 channel の configure が途中まで成功して失敗する。
- from-none configure 後、後着の timed 更新が投影を変える。更新は「device に投入される場合」と「channel 不在で記録だけになる場合」を分ける。
- Stop・Shutdown・別 thread の loss が device 呼び出し中に起きる。
- prune は履歴を消しても device の実状態を変えない。
- arm の成功・照合拒否を実際の記録に反映する。
- device 呼び出し中の line lock 解放だけでなく、`updates → streams → rec` の順序を保つ。

比較すると、次の不足があります。

## major

### 1. `seq` だけでは、記録だけになった後着 timed 更新を取りこぼす

**対象：** `brief.md:30,48`、`test-plan.md:54,137–143`

次の系列が S11 にありません。

1. channel が空で owner は idle。
2. 将来の enable を control が from-none configure する。
3. その後、**enable の effective instant より前**に効く timed 更新を記帳する。
4. control が更新を release する。その時点の channel は空なので、device には出さず設定に記録する。
5. owner が enable の segment を始める。

`book_timed` は既存の `ColdConfig` に更新を追加しますが、channel 不在の release は device に適用しません（`crates/ezsdr-radio-uhd/src/provider/control.rs:568–571,596–605`）。ここで同じ cold `seq` を理由に configure を省くと、古い周波数・gain のまま始まります。

**期待：** 更新を後続 segment の configure が適用する。timed device call は **0 回**です。「後着更新は必ず timed command として別に効く」という裁定は成立しません。

**推奨：** `seq` に加え、適用した投影を識別する版、または投影変更時の一致記録の無効化を採用してください。少なくとも「channel 不在で記録だけになった更新」を扱う必要があります。投入済み command の移動・取消も同じ規則で整理し、単に全更新で再 configure する方式は避けます。

この系列の実行再現は未実施で **INFERRED**。分岐と投影更新はコードで確認済みです。

### 2. 操作権が `begin`／`do_switch` だけでは、TX の async read と streamer 置換が競合する

**対象：** `brief.md:21–25`、`test-plan.md:16,39–46,125–129`

TX は clock がなく idle でも、run loop で `reports()` を呼びます（`crates/ezsdr-radio-uhd/src/provider/tx.rs:144–150,714–716`）。`do_switch` だけで busy を確認しても、control の from-none open 中に `tx_async` が入れます。逆向きに、`tx_async` 中の streamer を control が置換する経路も残ります。

native 側では `tx_async` が旧 streamer の `Arc` を保持し、`tx_open` は map から旧 streamer を外して新しいものを作ります（`crates/ezsdr-radio-uhd/src/uhd.rs:1010–1020,1050–1052`）。旧 streamer の寿命は、その map 更新だけでは終わりません。

**追加する系列と期待：**

- control の `tx_open` を止めて TX の通常 poll を実行 → `tx_async` を含め、新しい streamer 操作を始めない。
- TX の `tx_async` を止めて control の enable を受け取る → read が終わるまで streamer を置換しない。
- configure 成功から RX の `rx_start`／TX の採用まで → 操作権を途中で渡さない。

busy が覆う device call と、解放する境界を先に明記してください。Shutdown の ACK read も対象です。UHD で具体的にどのエラーになるかは **INFERRED** ですが、現在の call 順序と `Arc` の保持は確認済みです。

### 3. S9 は、busy 確認を削除しても通る構成になり得る

**対象：** `test-plan.md:125–129`

from-none の新 segment は、open・configure が返ってから記帳されます（`crates/ezsdr-radio-uhd/src/provider/control.rs:801–841`）。S9 が「空の plan からの最初の enable」なら、open 中に owner が `begin` しても次の segment がありません。**busy を無視する誤実装でも何もしません。**

**修正：** S1 と同様に、owner が開始できる旧 B segment を残した状態で、後続 C の open を止めてください。RX・TX 両方で、busy 確認の削除が B の device call／採用を実際に起こす構成にします。owner の cursor や pending switch が待機中に進まないことも確認してください。

### 4. S11 の観測は「実際に適用された周波数」を証明しない

**対象：** `test-plan.md:60,65,138–143`

`bursts` は周波数を記録しません。timed の `applied` 行も、device の read-back ではなく claimed 値と投入記録です（`crates/ezsdr-radio-uhd/src/provider/control.rs:626–629`、`design/18-uhd-radio.md:274`）。したがって「`bursts` か `applied` の周波数」で正しい device 状態は確認できません。

また、Run を通すだけでは「configure 完了後、owner 開始前」という順序を固定できません。

**修正：**

- gate または直接駆動で順序を固定する。
- origin 後に fake の実設定を read-back するか、周波数一致に依存する loopback 出力を確認する。
- 「timed command を投入する系列」と、指摘1の「記録だけの系列」を分ける。

S5・S10 の「clock が空」も観測を修正してください。TX clock は記帳時に登録され、拒否された clock も origin で終わる記録を持ちます（`crates/ezsdr-radio/src/timeline.rs:875–881,892–895`）。期待は「clock record がない」ではなく、**owner が採用せず、burst を送らず、拒否 clock が origin で終わる**ことです。

### 5. 部分成功、別 thread の loss、Shutdown が未網羅

**対象：** `test-plan.md:107–123,177–202`

S6・S8 は最初の `apply` の失敗だけです。これでは「全 channel 成功前に configured を確定する」実装を十分に固定できません。

**追加する系列と期待：**

- `{0,1}` の channel 0 は成功、channel 1 は apply 失敗／read-back 拒否 → `Open({0,1})` を維持し、configured は不一致。次の成功 segment は reopen せず、**両 channel** を configure する。
- control の open／configure を止めて、別 thread が loss を記録 → 実行中の call が戻っても、次の rates／apply／start を始めない。S18・S19 の「その call 自身が lost を返す」だけではこの窓を検査できません。
- busy 中に Shutdown → 操作権を解放して終了する。TX の ACK 待ちと RX の終了処理を妨げず、以後 segment を開始しない。

S17 の「stop 後、owner は device を呼ばない」は一般の期待として誤りです。Orderly stop では既存 RX の drain・untimed stop、TX の EOB・ACK read が必要です（`design/18-uhd-radio.md:257`）。**新 segment は始めないが、既存 stream の終了 call は許す**と分けてください。

### 6. lock 解放のテストだけでは、新しい設定照合の lock 順序を固定できない

**対象：** `test-plan.md:152–155`

既存 `LockSpy` は device call 時に `streams` が空いていることを見ます（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:164–169`）。`host_held` は host getter の観測で、`ColdConfig.updates` の取得順は検査しません。

新実装が記録の照合から `ColdConfig` の取得へ進むとき、`streams → updates` を作ると、既存の timed release／configure との順序が逆転します（`crates/ezsdr-radio-uhd/src/provider/control.rs:134,612–624`）。

**期待：** 同じ投影への timed book／release と owner configure を重ねても完了し、照合した投影と成功記録が食い違わない。新しい照合・成功記録の critical section を計画に示し、逆順取得を落とす検査を追加してください。

## minor

### 7. arm・prune・refused の期待が計画内で一致していない

- **arm：** `test-plan.md:30` は照合拒否を `Unknown`、`:196` は `Open(set)` としています。arm だけ例外にする根拠がありません。裁定どおり、照合拒否は `Unknown` に統一してください。
- **prune：** `test-plan.md:35` は configured を保持、`:174` は `None` にするとしています。device が変わらず seq も再使用しないので、**保持**を推奨します。
- **refused RX：** S21 は未決にする必要がありません。通常の timeline は拒否された RX cold segment を plan に含めません（`crates/ezsdr-radio/src/timeline.rs:873–879`）。到達可能な「拒否後に次の enable を記帳して復帰する」系列を主検査にしてください。
- **「今と同じく device を呼ばない」：** 現 TX は refused 判定より先に open／configure を実行し得ます（`crates/ezsdr-radio-uhd/src/provider/tx.rs:676–690`）。`brief.md:26` は現状説明ではなく、今回の新しい規則として記してください。

### 8. 既存テスト・mutation は、削除ではなく規則の移植まで対応付ける必要がある

**対象：** `test-plan.md:205–234`

stale／verified に依存する内部テストを置き換える方針は妥当です。ただし、次を修正してください。

- P361〜P367 の各規則を、新しいどの行・どの killing test に移すか対応表にする。「コードが消えるので行も消す」だけでは回帰検知が減ります。
- P358 は refused 判定を前へ移すなら当て直しが必要です。P351 も from-none の書き換え対象です。「残す」と断定せず、実装時に exact match と filter を点検してください。
- S10 は P350 の検査にはなりません。P350 は **device channel 番号と streamer 内位置の区別**です（`plan/maintenance/tools/mutations.json:7295–7302`）。既存の対応テストを残してください。
- S1 は open が成功する系列なので、「成功前に Open を書く」という誤りは落としません（`test-plan.md:83`）。その mutation は失敗または call 中の観測に割り当てます。

### 9. 約1,250行は暫定値。安全系列を次段階へ送る分割案は不可

**対象：** `test-plan.md:238–240`

共有 helper と test-only wrapper を使う方向なら、約1,500行は目標として不自然ではありません。ただし、現見積もりには TX async の操作権、部分成功、後着の記録専用 timed 更新、lock 順序の検査が入っていません。**現時点で「収まる」と断言する根拠は不足しています。**

超過時に S17〜S21 を送る案は退けます。Stop・loss・arm は今回変える lifecycle の安全条件です。まず重複テストをまとめ、必要なら今回の変更を安全に閉じる単位で再分割してください。

---

**推奨：指摘1〜6を計画へ反映してから実装開始。** 一記録への集約は維持し、古い cache に追加フラグを重ねる案には戻さないでください。

未確認：ビルド・テスト・mutation・実機は実行していません。UHD 4.10 の streamer 作成・graph 接続処理は container 内の一次ソースで確認しました。ファイル変更はありません。
