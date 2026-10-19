UHD の async report が使う channel 番号と、新しい channel 集合の扱いに不一致を見つけました。あわせて、拒否後の再開と、mutation の「equivalent」が実際に到達不能かを確認しています。
## 判定：FAIL（major の修正後に再レビュー）

### Findings

1. **major — TX report の channel 番号を取り違えている**  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:217`  
   `unacked` に device channel の集合を保存しているが、`TxReport.channel` は **streamer 内の位置**。stream 0 が `{1}` を送信すると、ACK／TimeError の channel は `0` なので照合できず、ACK が残り、`TIME_ERROR` が消える。  
   FakeDevice も `device.rs:1515-1516,1555-1556` で位置を返す。UHD 4.10 の `rfnoc_tx_streamer.cpp:152-159,312-319` でも確認した。  
   **修正:** pending report は `0..channel_count` で保持する。`{1}` の送信で ACK の退役と TimeError の target を確認するテスト・mutation を追加する。

2. **major — from-none の設定拒否が、RM-25 の「拒否した値を保持して halt」にならない**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:816-817`  
   open／rate check／configure が失敗すると、cold item と値を登録する前に戻る。例えば、全 RX channel が stream 外の状態から、antenna read-back が失敗する channel を join させると、拒否後も membership は元の `-1`。続く `start_rx` は、拒否した configuration を再試行しない。owner 側での拒否とは結果が異なる。  
   `tests/fake.rs:5964,6014` のテストは拒否と device call までしか見ず、halt・configuration of record・再開を固定していない。  
   **修正:** from-none の失敗も、値を保持した refused item として同じ timeline に記帳する。拒否→timed update→`start_rx`／cold による再開までテストする。

3. **major — equivalent のうち P344・P345・R26 は免除できない**  
   `plan/maintenance/tools/mutations.json:7235,7246`、`plan/phase7/tools/mutations.json:1118`
   - **P344/P345:** `StreamState.idle` の初期値は `false`。最初の owner poll より先に control が join を取れば、from-none を通らず owner が最初の streamer を開く。この場所の `check_rates` は必要。現在のテストは先に settle して、その順序を除いている。実際の scheduling による再現は **INFERRED／未実行**。
   - **R26:** 要素が空の方向では spec を設定しないため、`have > 0` が残る。mutant の `0..have` は `elements[0]` で panic する。これは現在も受理する片方向 layout で観測できる。  
   **修正:** P344/P345 は owner の初回 poll を制御するテストで kill する。R26 は空方向の既存テストへ retarget する。免除対応を道具へ足して済ませない。

4. **minor — 床上げ後に拒否される cold の event parity が未達**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:706-709,754-756`  
   未来の cold を予約した後、それより早い instant に RM-31 違反の更新を送ると、Mock は `LATE_COMMAND` と拒否を出すが、UHD は拒否だけ。report が認める差であり、共有の床関数だけでは一致しない。  
   **修正:** layout モードは Mock と同じ順序で報告し、この拒否列も parity test に入れる。current-keys の既存動作は別扱いで維持する。

5. **minor — VH-8 が裁定された比較を完了していない**  
   `crates/ezsdr-acceptance/tests/parity.rs:561-594`  
   floor test は T0 のみで、要求された「TX の future cold→それより早い RX cold」の instance-wide floor と clock の `ended_at` を比較していない。また、このファイルに論理更新の `(key, value, effective instant)` を抽出・比較する処理がない。M1 のテストも `tests/fake.rs:5269-5299` では calls のみで、裁定された RX channel 1 の内容を観測しない。  
   **修正:** これらの比較を追加する。現在のテストを、計画の全列を満たした証拠にはしない。

6. **minor — Manifest に存在しない rate key を記録する**  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:593-599`、`provider/mode.rs:244-248`  
   layout の configure は `radio.rx.c1.sample_rate_hz` のような未宣言 key を生成する。rate は channel key ではなく `radio.rx.s0.sample_rate_hz`。  
   **修正:** rate 行は stream key、frequency/gain/antenna 行は channel key にする。記録する全 key が、その instance の宣言に存在することをテストする。

7. **minor — spec 18 に偽の文と DA-1 違反が残る**  
   `design/18-uhd-radio.md:194-195,238,264,273,287`
   - UR-12 の `effective` が「ten keys」のまま。
   - UR-23 の reopen が「channel count changed」のまま。
   - UR-28 の source が無条件に `<id>/tx`。
   - UR-30 の `rates_checked` が arm 限定。
   - UR-33 に default serial `"FAKE0001"` が残る。
   - UR-12 に `this stage` が入り、段階の説明が規範文へ戻っている。  
   **修正:** layout／current を区別するか共通の規則へ引用する。段階の説明は Changes／decision record へ移す。

### report §9 point 8 への回答

- **(a)** event 差は今回修正することを推奨。4d への無条件持ち越しには反対。
- **(b)** equivalent を読む道具の統一自体は可能だが、今回の誤った免除の解決にはならない。
- **(c)** **P313:** 現段階では結果として equivalent。ただし「集合が等しい」という説明は誤りで、後続 stream に line がないため差が観測されない。**R23:** 正常な UHD が受理した非空 spec の長さを返す前提では冗長。削除か、その前提を明記した免除が妥当。**P344/P345/R26:** 上記の理由で不承認。

### 確認済み・健全だった点

- selector→serial 登録→master clock→subdev spec→front-end 照合の順序。
- serial の check＋insert は一つの lock 内。失敗時の guard 解放、cleanup では保持する構造も妥当。
- Mock の床・宛先・keys 判定の抽出は、元の判断を保っている。
- reopen の集合比較、per-channel configure、antenna read-back、D14 の主経路。
- 両 mutation list の全 `old` が現在のファイルにちょうど一回一致。retarget の意味も HEAD と照合した。P08/P09 は裁定された規則変更、それ以外に不当な意味変更は見つからなかった。
- 追加した Test 行の具体的なテスト名はすべて存在する。

**mutation が不足する主な規則:** 初回 owner-open の rate check、from-none 拒否後の状態と再開、非先頭 channel の TX report correlation、rate 行の宣言済み key、TX arm の rate check の独立した配線。

ファイル変更・build・テスト・mutation 実行はしていない。kill 成績は独立に再確認しておらず、報告された gate 通過を本レビューの根拠にはしていない。
