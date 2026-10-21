判定：FAIL

swap の指摘と S0 の調査は有用です。ただし、片側 prune の修正案が move の依存・両側拒否を失い、待機中の再検証と失敗後の回復にも穴があります。実装前に計画を修正してください。

## 独立に立てたシナリオ

テスト計画を読む前に、次を固定しました。

- RX/TX の移動先が先に動いても、移動元の cut 前の処理を失わず、物理 streamer の解放後にだけ開く。
- 空になる移動元、channel が残る移動元、連鎖・交換で循環待ちしない。
- Stop・halt・open/configure 拒否後も、残った streamer を後続の移動で回収できる。
- from-none の `busy`、TX report 読みと移動が競合しても二重操作しない。
- TX の旧 clock の burst と新 clock の予約を区別する。
- 片側 prune、拒否、loss、Provider stop でも両 line・clock・device 記録が一致する。
- 移動のない Stop→start に余分な reopen を加えない。

以下、計画への参照は `/home/komatsu/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/test-plan.md` です。

## 指摘

### 1. major — 片側 prune で move の記録を捨ててはいけない

**根拠：** 計画:295–301。`crates/ezsdr-radio/src/timeline.rs:488–494, 631–643, 699–727`。

§3.7 は片方の item が消えた時点で move を削除し、現在の release を固定 floor にします。しかし、もう片方ではまだ以下が必要です。

- move の拒否を両 line に伝えること。
- source の後続 booking/refusal による release の変化を destination に伝えること。

**系列：** destination だけ prune → まだ未処理の source の設定が拒否される。move の記録が消えていれば、`Lines::refuse` は source だけを拒否します。固定 floor だけでは、この関係を保存できません。

**修正：** per-stream prune を move 対応にする方向は妥当ですが、提案された「片側が消えたら関係も消す」は不採用。最小案として、未完了 move の item を pruning の境界に残し、両側が安全になるまで関係を保持する方式を検討してください。少なくとも次を、全 item を保持した場合との比較でテストします。

1. source 先／destination 先の prune。
2. その後の source release の変更。
3. その後の move 拒否と両側 halt。
4. 両側完了後の floor 保存。

**MockRadio への到達性：** 現在の Mock はどちらの prune も呼びません。検索 `prune` を `crates/ezsdr-mock-radio` 全体に実施して該当なしでした。したがって、報告された panic は共有 API で再現する問題ですが、現在の Mock の通常経路で到達するとは言えません。

### 2. major — swap 対策は必要。ただし安全な解放条件と連鎖のテストが不足

**根拠：** 計画:239–246, 357–389。`crates/ezsdr-radio-uhd/src/provider/rx.rs:193–203, 277–301`、`crates/ezsdr-radio-uhd/src/provider/tx.rs:293–315, 703–718`。

§5 の循環待ちの指摘には同意します。「衝突する open の前に、自分の旧 streamer を手放す」は、**旧 segment の処理を終えた owner が行う**条件で採用できます。

一方、`own_uncut`／`needing_elsewhere` を `cut.is_none()` だけで判定する案では、有限の未処理 segment を扱えません。

**系列：** A→B→C を先に予約し、B が channel を持つ中間 segment に既に cut がある。B が待つ channel の必要性を「uncut segment だけ」で探すと、A の解放が発動しない場合があります。

**修正：**

- 「uncut」ではなく「owner がまだ処理すべき segment」を基準にする。
- RX/TX 双方で、swap、有限の中間 segment を持つ連鎖を固定順序でテストする。
- RX の遅配された cut 直前の samples、TX の cut 直前の burst／held tail を処理してから close することを観測する。
- close 後に reopen が拒否されても、後続 cold/start で回復するテストを加える。

swap を未解決の穴として残して実装へ進む案は勧めません。

### 3. major — `Unknown` は「物理 streamer が無い」ではない

**根拠：** 計画:240, 243, 253。`crates/ezsdr-radio-uhd/src/provider/core.rs:359–378`。

`held_elsewhere` は `DeviceSet::Open` だけを見ます。しかし、open 成功後に rate 比較が失敗すると、実際の streamer は残ったまま記録は `Unknown` になります。`release_unneeded` も `Unknown` を無視するため、その streamer を回収できません。

**系列：** from-none の open 成功 → rate 比較拒否 → channel を別 stream に移す。記録上は競合なしでも、device は reconnect を拒否します。

**修正：** 新しい待機判定で `Unknown` を空集合と扱わないでください。物理的に保持する集合と、検証済み・利用可能な状態を区別するか、比較拒否時に owner が確実に閉じる方式を裁定し、この系列を追加します。UR-36 の変更も必要です。

### 4. major — 待機中の plan 変更を RX/TX とも再検証する必要がある

**根拠：** 計画:252–261。`crates/ezsdr-radio-uhd/src/provider/tx.rs:703–745`、`crates/ezsdr-radio/src/timeline.rs:631–643`。

TX は待機前の `to.refused` だけで判断する計画です。待機中に source の configure が失敗すると、同じ move の destination も refused になります。それでも古い `to` を採用し得ます。

RX も「segment が残るか」だけでは、origin/configuration が変わった場合に古い `next` を実行し得ます。

**系列：** destination が待機開始 → source が解放 → source configure を保留 → 拒否を記帳 → destination の待機を解除。

**修正：**

- 待機解除後、対象 segment の存在・拒否・origin/configuration と stop/loss を再確認する。
- 中断と拒否を区別し、中断だけで cursor／TX の `at` を進めない。
- TX の Stop は clock を切らず command queue に届くため、owner が待機で塞がれている間の Stop の扱いを明記し、held burst の取消しをテストする。
- `busy=true` の from-none を open/configure の途中で保持したまま source owner を進め、device 呼出しが重ならないテストを追加する。

同じ uhd-control は同期的な from-none 中に次の Action を処理しません。この直列性と、他 owner との競合は分けて検証してください。

### 5. major — S6 は両側 halt の観測と回復手順が誤っている

**根拠：** 計画:167–169, 278–280。`crates/ezsdr-radio/src/timeline.rs:488–494, 868–879`。

- `lines.refuse(stream, seq)` は既に move を両側拒否します。計画の「これでは一方しか halt しない」という誤実装説明は不正確です。新しく必要なのは両側の clock 再計算です。
- S2 の source は移動で空になります。新 segment が無いことだけでは、source が halted か判別できません。
- 拒否後も所属は move 後の値です。空の s0 に rate 変更だけを送っても、S6a の `rx_open 0 [1]` は起きません。
- S6c の「両 clock が e で ended」と、拒否された新 TX clock の「origin で ended」は区別が必要です。

**修正：** channel が残る S1 を用いて両 line の `halted(t)`／refused item を unit で確認し、RX の再開と TX の両 clock の終端を別々に観測します。回復テストは実際の所属に沿って書き直してください。source configure 拒否も追加します。

### 6. major — S3/S7 は待機に入ったことを保証せず、順序の主張も成立しない

**根拠：** 計画:143, 177–181, 227–228。`crates/ezsdr-radio-uhd/src/provider/rx.rs:228–229, 277–290`。

S1/S2 の destination は元の RX segment を e まで処理します。予約から 10–20 ms 後には、まだ open 待ちに入っていません。S7 は現状でも「未来の move が Stop/loss で消える」だけで通り、待機の出口を証明しません。

S3 の「別 command の `rx_stop 0` が destination open より前」も、control が止まらない証明ではありません。cold の instance-wide floor と独立 owner の実行順に左右されます。

**修正：**

- empty destination または test probe を使い、待機到達を通知させてから stop/loss/refusal を入れる。
- source 解放を gate で止めたまま、次 Action の booking 完了を直接確認する。
- 固定 sleep と 100 ms の実時間上限を correctness の根拠にしない。状態の到達を同期し、余裕のある watchdog だけを使う。
- `Provider::stop` の join テストと待機解除の unit テストを分け、既知の #67 を避ける系列を明示する。

### 7. major — 複数の assert が観測対象と一致していない

**根拠と修正：**

- **S1（計画:127）：** 2,000 samples／2 Msps の call は **1 ms** です。source の「2 ms」を修正。両 stream の stop が両 open より先、という cross-stream 順序も不要です。各 owner の stop→自分の open と、source release→destination open を確認してください。
- **S2（計画:131）：** destination は s1 なのに link が `rx2` です。`rx1` に修正しないと destination に line がありません（`provider/control.rs:263–270`）。
- **S5（計画:159）：** `tx_send ... n=` は channel 数ではなく samples 数です（`src/device.rs:1529–1543`）。wrapper で `samples.len()` を観測するか、channel 別の異なる波形を `transmitted` で確認してください。旧／新 clock の held burst、cut をまたぐ repeat も追加します。
- **S8（計画:189）：** timed apply は streamer 番号ではなく device channel を受けます（`provider/control.rs:707–731`）。「どの streamer に届く」ではなく、発行 instant、projection、実効値を観測してください。
- **S10（計画:208）：** s1 の最初の clock は move 前から存在します。`sequence` で新 segment を選び、cut-at-origin で省略される RX segment も考慮してください。

### 8. minor — H4 はソース確認と実機確認を分けるべき

UHD 4.10 の以下を確認しました。

- RX/TX streamer destructor は disconnect callback を呼ぶ。
- `multi_usrp_rfnoc` の callback は streamer と channel chain の接続を外す。
- graph の node 削除は edge を削除する。

一次資料：
[RX destructor](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_rx_streamer.cpp)、
[TX destructor](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/rfnoc_tx_streamer.cpp)、
[multi_usrp callbacks](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L338-L354)、
[graph edge 削除](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/graph.cpp#L790-L806)。

したがって、**最後の参照の破棄が論理 edge を外すことは VERIFIED（ソース）**です。X300 で別 streamer へ移した後の正常なサンプル継続は **INFERRED／bench 未確認**のままです。

また、計画:272 の「close は control mutex を取らない」は根拠不足です。新たに別 owner の close と open が並行するため、既存 open と同じ直列化を第一候補にしてください。native-boundary テストで、最後の Arc が残る場合と close/open の競合を確認します。

### 9. minor — S9 の結論は妥当。ただし仕様・理由文字列の修正漏れがある

**根拠：** 計画:195–202, 327–348。`provider/control.rs:507–515, 831–845, 876–886`、`timeline.rs:735–744, 868–872`。

通常の整合した key 経路では、四拒否が到達しないという結論に反証はありません。

- 同じ stream は no-op。
- 方向は key が固定。
- 所属は単一 key で、refusal 後も line は変更後の configuration を保持する。
- linkless stream は別の事前拒否になる。

ただし、これは membership projection と Lines の一致が前提です。二重 move、拒否後の次 move、prune 後の次 move を使い、この前提を固定してください。

併せて以下を訂正します。

- Mock の no-line 理由は `from stream s{from} to s{to}` であり、計画の `to stream s{to}` と一致しません（`crates/ezsdr-mock-radio/src/lib.rs:1912`）。
- UR-25 の現行 move 拒否文と旧テスト名を削除／置換する必要があります（`design/18-uhd-radio.md:268, 273`）。
- `effective != e` を拒否するなら、Mock 同様に booking を一時コピーで行ってから採用します。成功した `book_move` の後に単に return しても rollback にはなりません。

---

**推奨：** owner が旧 segment を処理し終えてから必要な物理資源を解放し、待機後は最新 plan を再取得する方式で計画を改訂してください。固定 floor による move 関係の早期削除と、swap を未解決のまま進める案は退けます。

ファイル変更・build・テスト実行・mutation は行っていません。S0 の実験結果は計画の記録を確認したもので、独立再実行ではありません。実機での streamer 移動は未確認です。
