# 設計メモ 37 の 37b：Sol 6.1 のレビュー

## 1 回目（FAIL）

判定：FAIL

本番コードの変更はありません。ただし、移設後のテストに **major 2件**あります。

## 指摘

1. **major — queue の待機判定が worker の実行時機に依存する**  
   `crates/ezsdr-radio-uhd/src/device.rs:2303–2314`

   **根拠：** `entered.send(())` は `apply` の**前**です。合図を受け取っても、17件目が queue の待機処理に入った証拠にはなりません。

   **系列：** worker が合図直後に200 ms以上停止すると、queue 制限を無効にした実装でも `done.recv_timeout` は期待どおり timeout します。その後 worker が復帰して即座に返れば、`join` も成功します。参加者であることは時計の進行を止めますが、worker が実行されたことまでは保証しません。この誤合格の再現は未実施（**INFERRED**）です。

   **修正：** 200 ms の「未完了」を機能判定に使わず、参加者内で17件目の呼出し前後の device 時刻を観測してください。開始時刻が0で、最初の command が20 msまで残る状態を確認し、正常終了時刻が20 ms以降であることを固定します。実時間の上限はハング防止だけに残せます。

2. **major — silent RX の代替テストが、切替に必要な timeout 上限を固定していない**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:1472–1494`

   **根拠：** timeout の assertion はどちらも「100 ms未満」だけです。一方、後継 segment の開始記録は、device の実時間待ちとは独立した Core 時計を15 msに止めたまま検査しています。

   **系列：** `recv_timeout()` が誤って一律99 msを返しても、両 assertion は通ります。実際の待ちが後継の origin＝60 msを越えても、Core の記録は15 msのままなので「origin より前に切り替えた」と判定できます。また、旧テストの「1 ms未満を渡さない」という assertion も失われています。これらの変異による実行確認は未実施（**INFERRED**）です。

   **修正：** この固定条件では、Core＝10 msで5 ms、Core＝15 msで下限の1 msになることを検査し、`turn()` が device に渡した値も固定してください。「100 msではない」だけでなく、cut＋quiet spanに収まることを観測する必要があります。

## 確認結果

- raw device 13本・Direct系5本には同名の移設先があります。
- 再対象化した mutation **19行**は、`old`／`old2` が各1回一致し、filter は移設先の `lib` テストを指しています。全リストで旧 binary への取り残し、移設名の `old`／`new` 内参照はありません。
- README・spec 18の移設先参照は概ね整合しています。
- UHD unit：**295/295成功**。
- fake：**233成功・4失敗**：
  - `ur_12_every_advertised_rate_is_accepted`
  - `rehearsal_b6_txrx_and_repeat_on_one_obx`
  - `ur_25_a_late_switch_drops_the_old_stream_s_tail`
  - `ur_34_the_receive_stall_fires_after_its_device_clock_interval`

上記4本の失敗箇所は今回の編集対象外ですが、変更前との比較はしておらず、原因・今回との差分との因果関係は未確認です。

未実施：mutation実行、負荷試験、実機試験。ファイル変更なし。通常テストの成功だけでは、指摘した誤合格経路は否定できません。

## 修正の再レビュー（PASS）

判定：PASS

前回の **major 2件は解消**。今回の再レビュー範囲では、新たな major / minor の指摘はありません。

1. **queue待機の判定**
   `crates/ezsdr-radio-uhd/src/device.rs:2295–2324`  
   17件目の呼出し前後に、worker自身がdevice時刻を読み、開始＝0、正常復帰後≧20 msを検査しています。workerは参加者なので、実行が遅れても時計は進みません。queue制限を外して即座に返る実装では、復帰後も0となり失敗します（変異による実行確認は未実施、**INFERRED**）。実時間の上限はハング監視だけです。200 msの待ちはdevice時計を進めるために使われています。

2. **silent RXのtimeout**
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:1471–1486`  
   `quiet_span`＝2,000 samples÷1 MS/s＋3 ms＝5 ms。`recv_timeout()` の式に従い、Core＝10 msで5 ms、15 msで下限1 msを検査しています。`turn()` がdeviceへ渡す値も1 msに固定されています。一律99 ms、下限の削除、呼出し側で後継originを越えるtimeoutを渡す変更は失敗します（変異未実行、**INFERRED**）。

3. **隣接系列・specとの整合**
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:1487–1499`  
   stop記録・旧clockの終端＝10 ms、後継の開始処理＝15 ms、origin＝60 msを確認しています。隣接する停止・loss系列と、`design/18-uhd-radio.md:455,517` の説明にも矛盾はありません。

**実行結果：** UHD unit test **295/295成功**。`git diff --check` 成功。

未実施：mutation、負荷試験、実機試験、`fake` binary全件。前回失敗した4本の再確認はorchestratorのgateに残ります。ファイル変更なし。
