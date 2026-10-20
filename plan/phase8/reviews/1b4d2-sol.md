# 1b-4d-2：Sol のコードレビュー

## コードレビュー

判定：FAIL

## 指摘

1. **major — 起動途中の device loss を失敗処理に引き継がない**  
   `crates/ezsdr-radio-uhd/src/provider/mod.rs:683-718`
   - **根拠:** `rx_start` のエラーを文字列へ変換し、`DeviceError.lost` を捨てて `stop(Abort)` へ進む。stream 間にも `core.is_lost()` の確認がない。
   - **系列:** s0 の owner が動き始めた後、s1 の `rx_start` が `lost: true` を返す。loss が公開されないため、s0 の停止処理が失われた device に `rx_stop` を呼び、cleanup も `close_streams` を呼び得る。また、s0 が先に loss を公開しても、起動側は s1 の start を続ける。実機での停止・解放の影響は **INFERRED**。
   - **修正:** lost エラーを `Core::device_failed`／`device_lost` に渡してから開始済み owner を止める。次の stream を始める前にも公開済み loss を確認する。P1 に lost エラー、S15 に先行 owner の loss と後続 start の交差を足す。
   - **機構:** `stream-lifecycle-scattered`。

2. **major — S16 が裁定の停止保証を十分に観測しない**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:6975-6987`  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:2337-2352`
   - **根拠:** RX は gate を保持したまま同期的に `stop` を完了させるため、裁定指定の「join の 1 s 内に終わる経路」ではなく detach 経路を固定している。TX は `do_switch()` の後に owner を進めず、burst も渡していない。
   - **系列:** TX が採用した終了済み clock 上の burst を誤送信する実装でも、送る機会のない現在の「`tx_send` なし」assert は通る。P474 は clock の採用だけを検査し、この誤送信を崩さない。
   - **修正:** RX は stop の記帳後に gate を解放し、owner の終了と非 detach を確認する。必要なら unit の同期点で固定する。TX は対象 clock の burst を渡し、gate 解放後の通常の owner 処理まで進め、送信されないことを確認する。対応する mutation も足す。
   - テストの不足であり、失敗機構の件数には加算しない。

3. **minor — gate の失敗時解放と worker 終了待ちが不完全**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:6803-6808,6830-6834`  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:2269-2282`
   - **根拠:** S13 の解放 guard は `entered_n()` の後に作られる。S14 は明示的な `release()` のみ。T4 も途中の assert が失敗すると `reading=true` が残る。detach 後は sleep だけで、worker の終了を確認していない。
   - **系列:** 到着待ちや assert が失敗すると、既に停止中の worker を残したままテストが終了する。
   - **修正:** 待ち始める前に解放 guard を設置し、成功・失敗の両方で終了通知を待つ。sleep を終了確認の代わりにしない。

4. **minor — UR-23 から current keys の Stop 宛先が落ちた**  
   `design/18-uhd-radio.md:245`
   - **根拠:** burst を終える宛先を `<id>/tx/s<k>` または `<id>` としたが、current keys は引き続き `<id>/tx` を受理する。
   - **系列:** current keys の `Stop(<id>/tx)` はコードでは burst を終了するが、この規範文には含まれない。
   - **修正:** current keys の `<id>/tx` と layout の `<id>/tx/s<k>` を併記する。

## 確認範囲

per-stream の links・clock・event handle・cold/timed/burst routing、所有権の lock 順、既存 parity／differential 経路を静的に確認した。maintenance 831 行と phase7 165 行の mutation anchor は、すべて現ツリーで各1回一致。`git diff --check` も問題なし。

テスト・mutation・実機は再実行していないため、実装報告の pass／kill 実績は未検証。ファイル変更なし。

## 修正の再レビュー

判定：PASS

再レビュー対象の4件は解消。追加の major / minor 指摘はありません。

1. **起動途中の loss**
   `crates/ezsdr-radio-uhd/src/provider/mod.rs:683-733`  
   lost な `rx_start` の失敗を公開してから開始済み owner を停止し、後続 stream の起動前にも loss を確認する。停止時の `rx_stop` と cleanup の `close_streams` の抑止も確認。`start_stream` probe の処理は test build 限定。

2. **停止と configure の交差**
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:841-889`  
   RX は実際の `begin` を gate し、origin での untimed stop、配布なし、`Turn::Exit` を観測する。detach を期待しない。  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:2320-2367`  
   TX は空でない plan と origin で終了した clock に burst を渡し、通常の `pass` まで進めて送信なしを検査する。旧テストの空 plan 問題も解消。

3. **gate と終了待ち**
   `crates/ezsdr-radio-uhd/tests/fake.rs:3838-3945,6851-6888`  
   S13・S14 は待機前に解放 guard を設置。sleep の代わりに worker の TLS 終了通知を待つ。  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:2259-2294`  
   T4 の `reading` も assert 失敗時に解除される。

4. **規範と周辺経路**
   `design/18-uhd-radio.md:245`  
   current keys の `<id>/tx` が復元済み。隣接する start／stop／cleanup、所有権・loss・timed release の経路に、新たな破綻は認めなかった。

`git diff --check` は問題なし。maintenance 835行・phase7 165行の mutation anchor はすべて各1回一致。P476〜P479 が対象テストを壊す対応は静的に確認したが、実際の kill は **INFERRED**。

未実施：build・テスト・mutation・実機の再実行。thread 作成失敗の動的検証は未固定のまま。ファイル変更なし。
