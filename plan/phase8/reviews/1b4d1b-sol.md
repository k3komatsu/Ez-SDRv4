# 1b-4d-1b：Sol 6.1 の独立コードレビュー

指示書は [1b4d1b-brief.md](1b4d1b-brief.md) §4．扱いは [1b4d1b-opus.md](1b4d1b-opus.md)．

判定：PASS

**major は見つかりませんでした。minor 2件はテストの固定不足です。** 本体の所有権・終了経路は今回の範囲で整合しています。

## 指摘

1. **minor：S13b に対応する mutation 行がない**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:2336`、`crates/ezsdr-radio-uhd/src/provider/core.rs:788`
   - **証拠：** テストは `"open"` probe で記録を読みますが、probe は device 呼出しの前です。両 mutation リストに S13b を殺す行はありません。
   - **系列：** probe 通過後、実際の `rx_open`／`tx_open` より前に記録を公開する誤実装は、この観測だけでは検出できません（**INFERRED：未実行**）。
   - **修正：** device の open を gate で停止し、停止中の記録が旧状態であることを確認する。呼出し前に記録を公開する mutation 行を付けてください。

2. **minor：S9 TX が idle mark の不変を確認していない**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:1980–1983`
   - **証拠：** 戻り値、cursor、switch、device call は確認していますが、RX 側と異なり `idle` の assert がありません。
   - **系列：** busy の早期 return で `idle=false` だけを書いても、このテストは通ります（**INFERRED：未実行**）。計画が要求する「idle mark を動かさない」を固定できません。
   - **修正：** busy 中の `read`／`pass` 前後で idle が不変であることを assert してください。

## 確認結果

- **TX の `end_reading → step`：** control が担当を取得できる idle 状態では clock・switch がなく、後続 `step` は device を呼びません。`do_switch` 中は idle=false のため、`read(Some(...))` が busy で prune を落とす通常経路も見つかりませんでした。
- **RX：** busy による `begin=false` は run loop の Idle に戻ります。receive・stop は実行中の stream に対する処理で、その間 idle は立ちません。
- **終了・記録：** from-none の通常出口と早期出口で busy を解放します。open を実行した経路では `record_opened` が configured を消します。呼出し前に終了して旧 configured が残る場合も、device は未変更で、loss／Provider stop は全 line を終了させます。
- **manual clock：** control は participant で、`await_reading` の host sleep は参加者を idle にします。TX が reading を残して panic した場合も、既存の `Provider::spawn`（`mod.rs:391–395`）が loss に変換します。shutdown の report read は busy を見ません。
- **lock：** production の `Locked` に guard の解放時点を変える箇所は見つかりませんでした。本体の `updates` 取得5か所すべてに事前検査があります。
- **mutation：** 指定27行はすべて現在のソースに各1回一致しました。殺傷結果は再実行していません。
- **UHD 根拠：** 4.10 の `graph.cpp:187–197` の reconnect 拒否を確認しました。
- **design/18：** 今回の本体と矛盾する新規規則は見つかりません。B4 の変更も難しくしていません。

**plan 33:266 の閉鎖時修正：** owner が busy を取得する説明と「busy なら通常 cold へ fallback」を削除し、control の from-none による一時所有、TX reading 待機、loss・stop の出口、限定された lock-order 検査へ置換してください。D1 option A・S11d・S28 の重ね合わせは **1c** に分離します。

未実施：ビルド・テスト・mutation・実機。ファイル変更なし。競合の実行時帰結は静的レビューによる判断です。
