テスト計画を読む前の独立シナリオを以下に固定します。

1. 同一 serial を再 bind（同じ／別 Device、異なる layout）→ 2件目を spec 変更前に拒否し、1件目の mapping・streamer は不変。同時 bind でも成功は1件だけ。
2. 登録後の open 検査失敗、cleanup、Drop、UR-7 reopen → 失敗・Drop 後は再 bind 可、cleanup だけでは生存 instance の登録を失わない。
3. 逆順／疎な／混在 layout を両 mode で開く → spec→channel 数→front-end 照合の順。空方向には spec を書かず、Basic/LF の番号違いを拒否する。
4. c1 のみ所属して arm → streamer は `[1]`、rate 照合も channel 1。open 前の照合はなく、集合の欠落・余分・内部差・read error は拒否し、cleanup で開いた streamer を閉じる。
5. 初期空 stream／RX link 無し→後から enable → 最初に実際に開いた時点で rate を照合する。未照合を合格として記録しない。
6. c0/c1 に異なる frequency・gain・antenna を設定→prepare、timed 更新、cold restart →各論理 channel に正しい値だけを適用し、segment の全所属 channel を untimed に再設定する。
7. 未所属 channel に timed 更新→所属追加、または所属予約と更新を逆順に受理→effective instant の membership で device 発行と再設定値を決め、他 channel に漏らさない。
8. RX/TX の per-channel timed 更新を交互に投入→instance 共通 queue の深さ・順序を守り、1 channel の更新を全 channel 分に数えない。
9. antenna read-back 不一致→prepare なら拒否、実行中なら当該 stream を halt。後続の成功した cold 更新／start_rx で正しい設定から復帰する。
10. membership を `-1→0→-1`、同数で `{0}→{1}`、最後の channel を除去→channel 集合で reopen を決め、空列を UHD に渡して channel 0 を誤開通しない。off／halt の再開規則と旧 TX clock の burst 拒否を保つ。
11. TX future cold→RX earlier cold（未所属 channel・line 無しも含む）→instance-wide floor、同時刻の到着順と LATE_COMMAND が一致。拒否した更新は床を上げず、current mode は従来の line-local floor。
12. stream-node 宛て UpdateParameter で別方向の key を更新→key が変更対象を決める。Stop は宛先だけ、root Stop は全体。空 s1・旧 `/rx`・未知 node の扱いは Mock と一致する。
13. future membership／rate／antenna の予約後に競合する更新→effective instant の設定全体で RM-31/32 を判定し、拒否時は plan・clock・device calls・後続 projection が不変。
14. stream 1 以上への所属を初期／runtime に指定→side effect 前に拒否。空 s1 の node 宣言と「s1 を実際に動かす」は区別する。
15. channel mode の通常受信・overflow・TX async error・cold restart→clock、event source、overflow handle、port が新 ID。current mode の既存 ID・記録は許可された追加以外不変。
16. 同じ短い操作列を Mock／Fake-UHD に入力→effective 設定、正規化した拒否、SampleClock の node・比・segment channel 集合を比較し、物理時刻や既知の #62 を同値性に混ぜない。
## 1. テスト計画を読む前に固定した独立シナリオ

1. 同一 serial を再 bind（同じ／別 Device、異なる layout）→ 2件目を spec 変更前に拒否し、1件目の mapping・streamer は不変。同時 bind でも成功は1件だけ。
2. 登録後の open 検査失敗、cleanup、Drop、UR-7 reopen → 失敗・Drop 後は再 bind 可、cleanup だけでは生存 instance の登録を失わない。
3. 逆順／疎な／混在 layout を両 mode で開く → spec→channel 数→front-end 照合の順。空方向には spec を書かず、Basic/LF の番号違いを拒否する。
4. c1 のみ所属して arm → streamer は `[1]`、rate 照合も channel 1。open 前の照合はなく、集合の欠落・余分・内部差・read error は拒否し、cleanup で開いた streamer を閉じる。
5. 初期空 stream／RX link 無し→後から enable → 最初に実際に開いた時点で rate を照合する。未照合を合格として記録しない。
6. c0/c1 に異なる frequency・gain・antenna を設定→prepare、timed 更新、cold restart →各論理 channel に正しい値だけを適用し、segment の全所属 channel を untimed に再設定する。
7. 未所属 channel に timed 更新→所属追加、または所属予約と更新を逆順に受理→effective instant の membership で device 発行と再設定値を決め、他 channel に漏らさない。
8. RX/TX の per-channel timed 更新を交互に投入→instance 共通 queue の深さ・順序を守り、1 channel の更新を全 channel 分に数えない。
9. antenna read-back 不一致→prepare なら拒否、実行中なら当該 stream を halt。後続の成功した cold 更新／start_rx で正しい設定から復帰する。
10. membership を `-1→0→-1`、同数で `{0}→{1}`、最後の channel を除去→channel 集合で reopen を決め、空列を UHD に渡して channel 0 を誤開通しない。off／halt の再開規則と旧 TX clock の burst 拒否を保つ。
11. TX future cold→RX earlier cold（未所属 channel・line 無しも含む）→instance-wide floor、同時刻の到着順と LATE_COMMAND が一致。拒否した更新は床を上げず、current mode は従来の line-local floor。
12. stream-node 宛て UpdateParameter で別方向の key を更新→key が変更対象を決める。Stop は宛先だけ、root Stop は全体。空 s1・旧 `/rx`・未知 node の扱いは Mock と一致する。
13. future membership／rate／antenna の予約後に競合する更新→effective instant の設定全体で RM-31/32 を判定し、拒否時は plan・clock・device calls・後続 projection が不変。
14. stream 1 以上への所属を初期／runtime に指定→side effect 前に拒否。空 s1 の node 宣言と「s1 を実際に動かす」は区別する。
15. channel mode の通常受信・overflow・TX async error・cold restart→clock、event source、overflow handle、port が新 ID。current mode の既存 ID・記録は許可された追加以外不変。
16. 同じ短い操作列を Mock／Fake-UHD に入力→effective 設定、正規化した拒否、SampleClock の node・比・segment channel 集合を比較し、物理時刻や既知の #62 を同値性に混ぜない。

## 2. Findings

以下、`P` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/test-plan.md`。**blocker なし、major 6件、minor 2件。**

### Major 1 — prepare/arm の RX clock を、まだ存在しない record で観測している

`P:664–667` は start 前に `sample_clock_records()` が1本あることを要求する。しかし prepare は宣言だけで、RX の登録は最初の block 時である。

**根拠:** `crates/ezsdr-kernel/src/time/domain.rs:381–389,398–426`、`crates/ezsdr-radio-uhd/src/provider/rx.rs:492–495`。

**修正:** 項目6 T1 は `declared_sample_clocks()` で node・比を検査し、登録済み record は0本とする。start 後の最初の block で record を検査するテストとは分ける。空 stream の T5 も「record が無い」だけでなく「宣言が無い」を検査する。

### Major 2 — parity の fixture と比較対象が、そのままでは成立しない

- `P:1661` の slot B＝BasicRX／LFTX は、現行 FakeConfig の1 slot＝1 board では作れない。項目2自身がこの制約を認識している（`P:269–270`）。
- `P:1662` はケース1の UBX c1 に 1 MHz・antenna `A` を要求し、受理ケースにならない。
- `P:1672,1677` は slot B の TX を所属させたまま同 slot の RX antenna を `TX/RX` にする。RX が未所属でも現在の RM-31 判定は拒否する（`crates/ezsdr-radio/src/channels.rs:778–810`）。
- `P:1653,1685` の全 `applied` 列比較は、Mock の論理更新記録と UHD の prepare／restart の全 channel read-back 記録を混同する。前者は `crates/ezsdr-mock-radio/src/lib.rs:1052–1057`、後者は `crates/ezsdr-radio-uhd/src/provider/core.rs:535–550`。
- `P:1686` は TX 初期 origin まで比較する一方、`P:1695` は arm instant が異なるため直接比較しないとする。

**修正:** 受理可能な fixture に直し、比較対象を「論理更新の key・値・effective instant」に限定する抽出規則を明示する。hardware configure 行は除外する。TX 初期 origin は各 arm instant に対する lattice 式で検査し、RX/cold の共通 instant と区別する。

### Major 3 — serial 読み失敗のテストが、握り潰しを検出しない

`P:571–574` の `Lost(0)` は serial 以外の後続呼び出しも失敗する。そのため `serial().unwrap_or_default()` に変えても、後続 `set_subdev_spec` の同じエラーで期待どおり拒否され、W6（`P:603`）を殺せない。

**根拠:** `crates/ezsdr-radio-uhd/src/device.rs:824–835`。

**修正:** **serial だけが固有エラーを返す wrapper** を使い、その理由の伝播と、後続書き込みが0件であることを検査する。また独立シナリオ1・2の未網羅分として、次を追加する。

- 同一 serial の並行 bind：Barrier で同期し、成功 instance を保持した状態で成功が1件だけ。
- `cleanup()` 後も instance 生存中は重複拒否、Drop 後は受理。

registry の check＋insert は1回の lock 内で行うことを裁定に明記する。sleep による競合再現は不要。

### Major 4 — 「effective instant の投影」を壊す実装が残る

項目13は現在値だけの単発テストで、`config_at(e)` を `self.config` に戻す誤りを殺せない。`P:1530` は項目11へ委ねているが、項目11の列は pending 設定による違反を作らない。

**根拠:** `P:1487–1505,1265–1313`、`crates/ezsdr-radio-uhd/src/provider/control.rs:380–385,522–535`。

**修正:** 少なくとも次の短い列を追加する。

- 同 slot、TX 未所属→RX antenna `TX/RX` を future cold で予約→TX join をそれより早い時刻で要求→床上げ後の設定で **RM-31 拒否**。
- c1 の future join と、その後に効く timed tune を両到着順で投入→発行先を現在の membership ではなく effective membership で判定。

拒否後に別の有効更新も送り、拒否値・拒否時刻が projection／床に残らないことを確認する。

### Major 5 — 初期空方向の runtime enable が rate 照合を迂回する

裁定は照合を arm に限定する（`P:30–32`）。一方、初期空 stream は arm で開かず、後の enable は control 内で初めて開く。

**根拠:** `crates/ezsdr-radio-uhd/src/provider/mod.rs:423–433`、`crates/ezsdr-radio-uhd/src/provider/control.rs:554–575`。

**修正:** 初期 TX 全未所属→不一致 rate-set の device で TX join、という列を追加し、**最初の runtime open の照合・拒否時の状態**を決める。推奨は arm と first-enable が同じ照合を呼ぶこと。arm-only の未照合 enable を意図的に許すなら、実装前に owner の判断と保証範囲の明記が必要。`rates()` の read error も wrapper で追加する。

### Major 6 — stream 0 の event identity 変更を正のイベントで固定していない

event pairs を追加して prepare を通すだけでは、旧 source の handle を取得・使用する誤りを検出できない。旧／新の両 source を collector に許しているため、なおさら見逃す。

**根拠:** `P:765,1398,1804–1805`、`crates/ezsdr-radio-uhd/src/provider/core.rs:377–384`。

**修正:** channel mode で RX overflow と TX async error を各1件発生させ、source がそれぞれ `/rx/s0`・`/tx/s0` であること、期待イベントが消えていないことを検査する。overflow handle の source を旧 ID に戻す mutation を対応させる。k≥1 の一般化だけを4dへ送る。

### Minor 1 — UHD の物理 queue と Provider の保守的な1列モデルを混同している

`P:870` の「装置の timed command queue は1本、VERIFIED」は誤り。UHD は **RFNoC block ごとの queue** と明記する。

**一次資料:** UHD 4.10 `host/docs/driver_usage/timed_cmd.dox:94–111`。1列化は `plan/phase8/33-channels-and-streams.md:172` の採用モデル。

**修正:** 「各物理 queue 内は FIFO：VERIFIED」「instance-wide 1列：Provider の規範モデル」に分ける。H8〈TX pre-chain `{1.0}`〉の条件付き経路は `host/lib/rfnoc/duc_block_control.cpp:152–164` と `multi_usrp_rfnoc.cpp:2030–2037` で **VERIFIED** にできる。ただし「streamer 前なら必ず `{1.0}`」は **INFERRED** のまま。

### Minor 2 — 無副作用 assertion に観測ノイズが混ざる

`P:940,1158` の「device calls が1本も増えない」は、動作中の recv／async／定期確認まで禁止しうる。また項目8 T1は TX membership を設定せずに TX clock が1本あることを要求する（`P:925,933`）。既定の TX membership は全て−1である（`crates/ezsdr-radio/src/channels.rs:522–552`）。

**修正:** 無副作用は `apply/open/start/stop` 等の変更系 call に限定する。TX 無傷の対照は TX を明示的に有効化して作る。既存の manual clock 方針は維持してよい。

## 3. 裁定・一次資料の評価

構造の方向は妥当。**selector→serial guard→master clock→spec→front-end**、両 mode の open checks、RAII guard、Manifest の明示欄、M1 の集合比較、acceptance crate の parity、stream-0-only と空 s1 node の区別を支持する。

H1・H2・H4・H5・H6・H7 は一次資料で確認できた。ただし H1 は「当該方向の chain を disconnect」であり、全 streamer handle の解放まで意味しない。H3 の `{1.0}` も input-rate 未確定という条件付き。根拠は UHD 4.10 の `multi_usrp_rfnoc.cpp:2832–2840,2902–2906`、`subdev_spec.cpp:19–20,45–59`、`db_basic_and_lf.cpp:100–107`、`x300_radio_control.cpp:281–319`、`ddc_block_control.cpp:153–159`。

**Verdict: まだ実装開始不可。** 上記 major を計画へ反映し、とくに parity の成立条件と first-enable の rate 照合を先に確定する。
