<!-- re-review 1 (after fix rounds 2-3): sol-rereview.md -->

判定：要修正

### 1. major — 省略されたsegmentのuntimed tuneが後続segmentへ引き継がれない
**箇所：** `crates/ezsdr-mock-radio/src/lib.rs:1072,1099–1102,2004`、`crates/ezsdr-radio/src/timeline.rs:888–893`

**失敗シナリオ：** UBX RXをtimed tune → streamから外す → 同じinstantに「再加入」「rate変更」の順で予約する。再加入segmentは開始前にcutされ、RXのplanから省略される。一方、rate変更の`before`には再加入済みのchannelが記録される。その結果、残ったsegmentではuntimed tuneされず、以前のtimed定数と`timed_tune, true`が残る。`Stop`→`start_rx`→開始前のcold変更にも同じ機構がある。

**修正案：** booking上のmembershipと実際にtune済みの状態を区別し、省略segmentの未実行retuneを後続へ引き継ぐ。両系列の位相・coherenceを固定するテストとmutationを追加する。MR-34の文も合わせる。**S2は未閉鎖。** 実行再現は未実施のため、観測結果は **INFERRED**。

### 2. minor — TX moveの宛先保持を固定するテストがない
**箇所：** `crates/ezsdr-mock-radio/tests/mock_layout.rs:2270–2328`、`crates/ezsdr-mock-radio/src/lib.rs:1977`

**失敗シナリオ：** TXだけ宛先の`record_held`を省く誤りは、宛先が空の現テストでは検出できない。P188は移動元の保持でkillされる。

**修正案：** 2 channelで十分。`c0→s0`、`c1→s1`をtimed tune後、`c0`を`s1`へ移し、既存`c1`の位相保持を検査する。TX宛先だけを壊すmutationを足す。

### 3. minor — 同時untimed tune間のdraw orderにkilling testがない
**箇所：** `design/09-mock-radio.md:206`、`crates/ezsdr-mock-radio/src/lib.rs:2333–2338`

**失敗シナリオ：** 同一originでRX/TXやstream番号の順を逆転しても、新テストは各々ほぼ単一のretuneだけを観測するため、規定した順序違反を固定しない。P187が検査するのはcommandとの前後関係。

**修正案：** 同一originの複数retuneについて、各channelへ割り当たる乱数を検査し、順序反転mutationを追加する。

**確認済み：** P175–P189の`old`は各1回一致。各rowが指定する誤実装とassertは対応し、保存ログも全件killed。旧3テストファイルは差分なし。MR-39・MR-40と§6の記述に追加の問題は認めなかった。なお、`lib.rs:2203–2204`のKernel前提自体はMA-30からは保証されないが、現在の非deterministic profileでは2 ms leadが同instantの後着timed更新を後へ送る。

ファイル変更・build・mutation再実行は行っていません。

---

<!-- re-review 2 (after fix round 4): sol-rereview2.md -->

旧majorの原因だった予約時membership記録は削除され、実際のsegment originでretuneする形になっています。残る重点は、同時RX/TXの乱数順序と、enable・move-out・事後cutのテスト／mutationによる固定です。
判定：要修正

実装上の新たなmajorは見つかりません。残るのは検証の不足です。

### 1. minor — RX/TX同時originのdraw順が未固定
- **箇所:** `crates/ezsdr-mock-radio/src/lib.rs:2317`、`design/09-mock-radio.md:206`
- **失敗シナリオ:** receive-before-transmitを逆転しても、P190はchannel順、P191は同方向のstream順しか検査しません。同時originは、RXのcall＋allowance分だけTXのcold変更を後に予約すれば構成可能です（実行未確認：**INFERRED**）。
- **修正:** RX/TX双方のorigin一致と、各channelに割り当たる乱数をassertし、方向順反転mutationを追加してください。報告§4.1は未閉鎖です。

### 2. minor — enable・move-outを個別に落とすmutationがない
- **箇所:** `crates/ezsdr-mock-radio/tests/mock_layout.rs:1960,2091`、`plan/maintenance/tools/mutations.json:5732`
- **失敗シナリオ:** `start_rx`のsegmentだけ、またはmove-out側の残存channelだけretuneを省く誤り。両テストには検出するassertがありますが、現在のmutation filterは両テストを実行しません。P194が固定するのはre-entry＋rate変更です。
- **修正:** 各経路のretuneを省くrowを追加し、それぞれのテストでkillを確認してください。報告§4.2のこの2件は未閉鎖です。

### 3. minor — 事後cutのCeilingが未検証
- **箇所:** `design/09-mock-radio.md:209`、`crates/ezsdr-mock-radio/src/lib.rs:2683`
- **失敗シナリオ:** originを処理済みのTX clockを、そのoriginでcold変更またはProvider stopがcutする場合。P193は処理前にcutを予約するケースで、このCeilingを固定しません。
- **修正:** originを一度処理してから同instantでcutし、既消費drawと後続drawの位置を検査してください。報告§4.3は、Ceilingとしての明記だけでは検証済みになりません。

**確認済み:** 旧major 1はmembership記録の削除と全channel retuneで解消。旧minor 2は対象の仕組みが消滅。旧minor 3はchannel順・stream順まで閉鎖しています。旧3テストファイルは差分なし、新retune処理は`keys`下です。maintenance・phase3の全`old`は各1回一致し、保存ログは対象23行すべてkilled。MR-34・§4・§6に追加の形式上の問題は認めませんでした。

ファイル変更・build・mutation再実行はしていません。実行時の結論は静的確認による **INFERRED** です。
