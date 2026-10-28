> plan · gpt-6.1-sol
P1 の数：2

## P1

1. **時刻付き underflow が集計から消える**  
   `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:395–398`
   - Provider は時刻付き TX report のイベントを TX SampleClock で発行します（`src/provider/tx.rs:901–907`）。`ticks_in(root)` は変換ではなく domain 検査なので、これらを `filter_map` が捨てます。
   - 結果：underflow があっても件数が 0 になり、曖昧になった送信の headroom を分布に含めます。整数の underflow tick を直接渡す単体テストでは、この接続ミスを検出できません。
   - **修正：** Manifest の SampleClock 記録で root に変換し、変換不能も明示する。時刻付き underflow を通す統合テストを追加する。

2. **TX repeat が途中終了しても H4 を `complete` と記録する**  
   `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:383–437`
   - `stop_after` が見るのは gain 更新の失敗だけです。TX が部分送信や underflow で終了しても、RX gain の適用が続けば `status=complete` になります。Provider は underflow 時に burst を中断します（`src/provider/tx.rs:918–922`）。
   - 結果：「10 MS/s TX repeat と同時」という条件を満たさない dispatch 分布を、完了した測定として残します。
   - **修正：** TX の継続・更新区間との重なりを確認し、成立しなかった区間と結果を partial／invalid と記録する。gain は成功するが TX が途中終了する試験を追加する。

## P2

3. **観測行が無制限で、待機のたびに全行をコピーする**  
   `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:275–285,393–394,429–430`
   - `matching_timed_apply()` は各ポーリングで `probe.rows()` 全体を複製します。その間、送信記録側も同じ Mutex を待ちます。1,000 sample の repeat は 10 MS/s で毎秒約1万回の send になり、測定自身が増大するコピー負荷を発生させます。
   - 指示書の「行数上限と切り詰めの記録」もありません。
   - **修正：** timed apply だけを直接検索・取得し、大量の TX 行をコピーしない。保持・出力に上限を設け、欠落件数を記録する。

4. **正方向の時計不連続を正常な対応として扱う**  
   `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:75–106,119–128`
   - 検査しているのは主に anchor の増加順です。区間内で時計が飛んでも、両端の device tick が増加していれば補間が成功し、狭い bracket の dispatch／headroom を出します。
   - 現在の「不連続」テストは停止・逆転だけで、この場合を検出しません。
   - **修正：** master clock と中間の bracketed readings を用いて対応の整合性を検査し、不整合な区間を除外する。増加順のまま時計が飛ぶ合成試験を追加する。

5. **測定中の SIGTERM では負荷 cleanup がすぐ実行されない**  
   `plan/phase8/tools/run-record.sh:86–94,306–309`
   - foreground の Docker コマンドを待つ Bash は、コマンド終了まで trap を遅延します。runner の PID に TERM を送っても、長時間の測定中は H4 worker が動き続けます。同じ構造の無負荷な確認でも、trap は foreground コマンド終了後でした。
   - **修正：** 測定を追跡可能な非同期プロセスとして待ち、signal 時に測定 container と負荷を停止・回収する。runner 待機中の TERM／INT を試験する。

未実施：ビルド、実機、mutation 実行。maintenance の全1,472行と P1124–P1147 の source 一意一致は確認済み。

## Re-review after fix round 1

> plan · gpt-6.1-sol
既存5件は修正済みですが、隣接する正常系に **新規P2が1件**あります。P1はありません。

| 前回の指摘 | 判定 |
|---|---|
| 1. SampleClock付きunderflowの脱落 | **修正済み**。Manifest経由で変換し、変換不能も明示。統合テストが旧挙動を検出します。 |
| 2. TX早期終了でもcompleteになる | **修正済み**。終了後のgain更新を分布から除外し、partialにします。ただし下記の正常系回帰があります。 |
| 3. 全観測行のコピーと無制限保持 | **修正済み**。一致行のみ取得し、保持上限・欠落件数を記録します。 |
| 4. 正方向の時計不連続を許す | **修正済み**。master clockとの速度整合性を検査。新しい合成テストは旧挙動なら失敗します。 |
| 5. signal時のcleanup遅延 | **修正済み**。非同期起動＋waitになっています。抽出したcleanupとダミー測定プロセスで、TERM・INT双方の即時停止・回収を確認しました。 |

### 新規P2：正常終了の空EOBを送信失敗と判定する

**箇所：** `crates/ezsdr-radio-uhd/tests/common/measurement/h4.rs:435–440,525–527`

正常なrepeat停止では、Providerが空バッファのEOBを送ります（`crates/ezsdr-radio-uhd/src/provider/tx.rs:576–585`）。今回の判定は、その正常な `accepted_samples=0` まで不完全送信に数え、H4を `partial` にします。

既存の正常系テストをdebuggerで確認した結果：
- gain更新：2件ともTXと同時
- 欠落行：0件
- `incomplete_tx_sends=1`
- `partial=true`、理由は `incomplete_tx_send`

**修正：** 正常な空EOBをpayload送信失敗と区別し、正常系テストに `status == "complete"` を追加してください。

**検証：** H4の11テストと行数上限テストは成功。P1148–P1157は各1か所一致。`git diff --check`も成功。

**判定：新規P2を直してから受理。**  
未確認：実機・mutation再実行。fake全体は120秒でタイムアウトし、完走確認できていません。ファイル変更なし。

## Orchestrator: new P2 (empty end-of-burst counted as incomplete) fixed in h4.rs; the fake returns accepted_samples 1 for the no-sample EOB, so the check keys on sample_counts all 0 and no error; P1158.

## Gate: P1134 retired
P1134 (`*span > 0` → `>= 0` in `map_device_tick`) survived the full gate: fix round 1 added `clock_rate_matches`, which refuses a stopped device clock (span 0 with a positive host span) before the division, so the mutant is equivalent. The filter stays as a guard; the row is removed (its ID is not reused).
