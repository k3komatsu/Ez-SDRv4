# 設計メモ 37 の 37a：Sol 6.1 のレビュー

## 1. メモのレビュー

判定：FAIL

**手動時計を維持し、試験スレッドの実行時機に依存する判定をなくす方針は妥当です。** ただし、移設先の時計、掃除の対象、mutation の移行範囲が不足しています。

1. **major — `rig_with` の Authority だけでは device の時計を制御できない**  
   対象：`plan/phase8/37-test-thread-timing.md:50`

   **根拠：** `test_support::rig_with` は渡された設定で FakeDevice を作ります（`crates/ezsdr-radio-uhd/src/provider/test_support.rs:31-35`）。既定は実時間であり、`ManualTimeAuthority` とは別の時計です。Authority だけ進めても、FakeDevice の late・underflow・実際の再生位置は固定されません。既存の `ur_21_first_dispatch_*` の移設実績だけでは、sample-exact な continuation の検査まで成立する証拠になりません。

   **修正：** 移設には既存の manual FakeDevice＋`Frozen`＋`device_at` を使い、Core と device の両時計をそろえると明記してください（同ファイル`:140-175`、`src/provider/tx.rs:2253-2266`）。A の送信を実際に進めて `continues_at`・held sample を確認し、その状態で B／Stop／fault を投入します。既存部品で実現可能ですが、5本すべての系列の成立は **INFERRED** です。削除前に対応表と実装で確認する必要があります。

2. **major — 掃除の条件が、同じ原因の capture・Stop・raw device テストを漏らす**  
   対象：`plan/phase8/37-test-thread-timing.md:49-52`

   **根拠：** 「`run.now()`／`tx_at` から近い時刻を `drop` で予約」に限定すると、次が対象外になります。

   - `fake.rs:1557-1569`：Stop 前に読んだ時刻を終了期限の基準にするため、読取り後の deschedule が判定に混入します。
   - `fake.rs:2877-2884,2897-2903,2918-2925`：非参加者が `device.time_now()`→timed send を行います。初回送信が遅れれば、境界・underflow の検査が別のケースになります。
   - `fake.rs:3364-3367`：送信後、ACK 不在の確認までに停止すると、正しい ACK が既に到着し得ます。
   - `crates/ezsdr-acceptance/tests/uhd.rs:234-240`、`crates/ezsdr-server/tests/protocol.rs:1092-1100`：未来の capture 時刻を決めてから投入する隙間があります。TX の late policy は関係ありません。

   `fake.rs` の `ur_21_late_policies_on_the_device_lead`、`ur_23_bursts_go_out_in_target_order`、`ur_23_a_one_sample_burst_booked_ahead_starts_at_its_time` などにも、同じ timed booking の隙間があります。

   **修正：** 検索対象を「非参加者の時刻読取り／状態観測と、後続操作／assert の間に時間が進む全系列」に広げ、helper 経由も追って分類してください。「B を十分前に予約する」だけでも確率を下げるにとどまります。Spec の開始前予約、時刻が動いても成立する assertion、制御時計による検査を区別してください。上記 capture 系列の負荷下での実際の再発は **INFERRED** です。

3. **major — 移設するテストを参照する mutation が4行漏れている**  
   対象：`plan/phase8/37-test-thread-timing.md:53`

   **根拠：** 列挙された5行のほかに、以下も削除・移設対象を filter にしています。

   - phase7 **B08・B16・B20**：`ur_23_a_burst_booked_at_a_sent_burst_s_end_continues_it`
   - phase7 **B10**：`ur_23_a_stop_while_the_device_burst_waits_for_a_continuation_ends_it`

   参照：`plan/phase7/tools/mutations.json:1197-1204,1217-1224,1267-1274,1309-1316`。

   **修正：** この4行も対象に追加してください。移設後は filter だけでなく `test_bin: "lib"` も更新し、無変異で対象テストが実行・成功することと、各 mutant を意図した assertion が kill することを確認します。掃除で追加移設するテストについても、全リストの `filter`・`old`・`new` を検索してください。

4. **minor — VERIFIED とした説明に系列・時期の誤りがある**  
   対象：`plan/phase8/37-test-thread-timing.md:27-33`

   **根拠：** 5本すべてが「A の終わりの8 ms前で B を予約」するわけではありません。Stop のテストは7 ms前で Stop を投入します（`fake.rs:3089-3092`）。c+1 のテストには先行予約の分岐もあります（`:3155-3173`）。また `ur_21_a_descheduled_*` の追加は `e19bba2`（2026-10-13、計画30段階A）で、Phase 7 Review N・Q 以来ではありません。

   **修正：** 初回 A の booking 遅れと、continuation／Stop の投入窓逸脱を分けて記述してください。負荷再現の「落ち方が1つだけ」は、今回読める資料ではログを検証できなかったため、独立レビューとしては **INFERRED** です。再現ログへの参照を付けてください。

### その他の判断

- **`ur_21_a_descheduled_*` の方針は成立します。** 現在の Session API では `start_repeat` が `SendAsapAndFlag` を選びます（`crates/ezsdr-radio/src/lib.rs:270-281`）。repeat に変わる点を明記し、wrapper 到達確認を残せば、参加者内の200 ms停止について「manual は device-late なし、real はあり」という意味は保てます。
- **`hold()` は今回は不要です。** 既存の制御時計で必要な状態を作れるなら、polling と模擬遅延の扱いを追加するより小さく安全です。lead 延長、再試行、filter 除外だけを退ける判断も妥当です。
- **負荷試験は再発検査であり、決定性の証明ではありません。** 16 busy loop と計画30の grid に加え、各移設テストで目的の状態・分岐への到達を観測し、mutation で判定の強さを確認する必要があります。
- **not risky の分類は条件付きで妥当です。** 既存の時計・fake・本番の期限や lifecycle を変えず、テストだけを移す範囲なら該当します。新しい時計機構や fake の挙動変更が必要になれば再分類してください。

未実施：テスト・負荷再現・mutation・実機確認。ファイル変更なし。

## 2. メモの再レビュー

判定：PASS

前回の major 3 件・minor 1 件は解消しています。設計変更を求める新たな major はありません。

### 前回指摘の確認

1. **major 1：解消 — Core と device の時計を両方制御する**
   `plan/phase8/37-test-thread-timing.md:53` は `manual_rig`・`Frozen`・`device_at` を指定しました。実装とも一致します（`crates/ezsdr-radio-uhd/src/provider/tx.rs:2253-2256`、`src/provider/test_support.rs:140-175`）。5 系列すべての成立は **INFERRED** とし、削除前の対応表・状態到達確認を要求している点も適切です。

2. **major 2：解消 — 掃除が TX booking 以外も含む**
   `plan/phase8/37-test-thread-timing.md:52-55` は、非参加者の観測と後続操作・assert の間の時間進行を対象にしています。追加された Stop、raw device、ACK、capture の参照先は、記述した系列と一致しました。
   
   §3 の四択も妥当です。ただし、(3) は送信時刻の移動が検査対象を変えない場合だけです。`ur_21_a_descheduled_…` では repeat への変更を明示し、wrapper 到達確認を残すため、この条件を満たします（`crates/ezsdr-radio/src/lib.rs:270-281`、`crates/ezsdr-radio-uhd/tests/fake.rs:3744-3762`）。

3. **major 3：解消 — mutation の 9 行が一致**
   全 mutation JSON を検索し、対象 6 テストの参照が **J295、B08、B10、B16、B20、B28、B29、B31、B32** の 9 行であることを確認しました。列挙漏れはありません。

4. **minor：解消 — 系列・時期・検証範囲の訂正**
   `plan/phase8/37-test-thread-timing.md:21-36` の Stop の 7 ms、c+1 の先行／後続予約、`ur_21` の追加時期はコード・履歴と一致します。再現はログのない orchestrator の観測と明示され、個々の失敗経路も **INFERRED** に分離されました。

### 実装指示書で明確にする点

5. **minor／非阻害 — J295 は `fake` に残す**
   `plan/phase8/37-test-thread-timing.md:56` は、9 行すべてを `lib` に変更する指示にも読めます。しかし `ur_21` は移設せず verb だけ変更する方針です（`:54`）。J295 の現行対象も `test_bin: "fake"` です（`plan/maintenance/tools/mutations.json:3180-3187`）。
   
   指示書では、**移設する B 系列は `lib`、J295 は `fake` のまま再検証**と区別してください。改名・削除時には mutation の `filter` に加えて `old*`／`new*`、および `design/18-uhd-radio.md:248,458-467` のテスト参照・説明も追跡対象にします。

`hold()` を追加せず既存の制御時計を使う判断、負荷試験を決定性の証明としない判断、既存機構・本番挙動を変えない条件での not risky 分類は支持します。

未実施：テスト、負荷再現、mutation、実機確認。ファイル変更なし。負荷再現の数値と全5系列の移設成立は独立には未検証です。

## 3. コードレビュー

判定：FAIL

### 指摘

1. **major — Stop の期限判定に試験スレッドの遅れが残る**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:1590–1598`

   **根拠：** `stopped.time` は Provider が Stop を受け取った時刻ではない。`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1352` で読み、dispatch（`:1498`）より後に、その値をログへ保存する（`:1506`）。

   **系列：** `submit` 内で時刻を読んだ直後、dispatch 前に試験スレッドが60 ms停止する。その間も repeat は続き、正常な Stop でも `end <= stop_at + 30 ms` が成立しなくなる。遅延挿入による再現は未実施（INFERRED）だが、時刻取得と dispatch の隙間はコードで確認した。

   **修正：** in-flight window の上限は制御時計の unit test で固定する。fake 側では停止・EOBなど時刻の隙間に依らない性質を確認する。`LogEntry.time` への置換だけでは解決しない。

2. **major — in-flight 判定を通らずに、同じ期待結果で合格できる**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:1454–1466`

   **根拠：** 5 ms先の2操作も `tx_at` の読みから提出までに遅れ得る。遅れると、`SendAsap` と `Drop` は uhd-tx の in-flight 判定ではなく、`control.rs:1052–1064` の予約時判定だけで生成される。現在の assert は両者を区別しない。

   **系列：** 2本とも時刻計算後に遅れて予約されると、予約時に `[SendAsap, Drop]` が揃う。U44 のように `preempt` の再判定を無効にしても、残りの assert はその欠落を検出できない経路がある。mutation の実行検証はしていない（INFERRED）。

   **修正：** 5 msの2操作による再判定を制御時計の unit test に移し、予約時には on-time、uhd-tx で初めて移動・破棄されたことを確認する。対応する mutation filter も移す。spec 18 §6 の説明も合わせる。

### 確認できたこと

- 本番コードの変更なし。
- 表から移した4系列（#2・#5・#8・#16）の理由は、既存コードと整合する。
- 削除した fake テスト8本には同名の unit test がある。
- `uhd_59_receive_capture_starts_at_the_requested_index` は厳密な開始位置を固定している。
- 指定の mutation 16行は `old` が各1回一致し、指定 binary に filter のテストが存在する。削除名を参照する取り残しも確認されなかった。

**実行結果：** UHD unit 275、fake 256、acceptance UHD 9、server protocol 43、すべて成功。`git diff --check` も成功。

負荷試験・mutation・実機試験は未実施。通常実行の成功だけでは、上記の遅延経路は否定できない。

## 4. 修正の再レビュー

判定：PASS

前回の major 2件は解消。新たな major はありません。minor 1件が残ります。

### 指摘

1. **minor — spec の試験説明だけが repeat のまま**
   `design/18-uhd-radio.md:446`

   **根拠：** 制御時計の Stop テストを「a repeat」と説明していますが、実際は `tx.rs:3152` の `held(...)` が作る有限 burst です（`tx.rs:935`: `repeat: false`）。

   **影響：** in-flight 上限の検証は成立していますが、説明と実際の系列が一致しません。

   **修正：** 「a repeat」を「an open burst」に直す。本番コードやテストの変更は不要です。

### 再確認結果

- **Stop の上限：解消。** `tx.rs:3155–3175` は時計を固定し、複数回の step 後も `open.next == 22_000`、Stop 後の record が窓以内、device の ACK がその1 sample後であることを確認しています。
- **最初の hand-over での再判定：解消。** `tx.rs:1782` の新テストは、予約時には target 不変・イベントなし、その後 uhd-tx で初めて SendAsap／Drop になることを区別します。移動先の予約、B の実サンプル、継続時の SOB なしも確認し、削除した fake テストの性質を置き換えています。
- 残した fake 2本には、前回指摘した上限判定・5 ms の再判定窓への依存はありません。3本目の受付と記録も非空で確認されています。
- T2 は最後の送信 `(1, false, true)` と直後の無信号を確認し、空の EOB への置換を見逃しません。
- U19・U36・U44・R06・B04・B10 はすべて `lib` を指し、`old` は各1回一致。対応する assertion が変異を検出する筋も確認しました。削除名の取り残し、本番コードの変更はありません。

**実行結果：** unit 277件、fake 255件すべて成功。`git diff --check` 成功。

負荷試験・mutation 実行・実機試験は未実施。ファイルは変更していません。
