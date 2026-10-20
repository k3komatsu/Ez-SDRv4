現時点で、stream 0 の既存動作を変える差分は見つかっていません。一方、追加の順序テストは `cold_first` を切り替えても実際の予約順が変わらず、片側のケースを検査できていません。mutation の一致数と持ち越し項目も照合しています。
## Findings

### minor 1 — 予約順の逆順ケースが実行されていない
**`crates/ezsdr-radio-uhd/src/provider/control.rs:1335–1347`**

`cold_first = false` でも、実際の呼出しは常に cold → RX timed → TX timed。分岐が変えるのは `bookings.push` の位置だけで、両ケースの操作列・期待値は同じです。

**失敗シナリオ:** timed → cold の場合だけ projection／seq を取り違える実装を、この unit test は検出できません。別の integration test は逆順も検査しているため、非 blocking です。

**修正:** device 操作ではなく、実際の `book_cold`／`book_timed` の順を分岐させ、期待 seq も対応させる。

### minor 2 — 4c 持ち越し一覧から Manifest の stream 識別が抜けている
**`crates/ezsdr-radio-uhd/src/provider/core.rs:283–297`**  
**`/home/komatsu/.cache/ezsdr-fixes/2026-10-18-phase8-1b4b/report.md:130–139`**

`record()` は同方向の全 stream の segment を平坦化し、stream 番号を記録しません。テスト計画 final (c)-11 の宿題が、報告 §7 に引き継がれていません。

**失敗シナリオ:** 4c で RX stream 0・1 を動かすと、Manifest の各 segment がどちらのものか判別不能になります。現段階では各方向1本なので問題ありません。

**修正:** stream 識別付き plan と segment ごとの論理 channel 列の記録・検査を、4c の持ち越しに明記する。今回の Manifest は変更しない。

## 確認して妥当だった点

- HEAD `fdfdf3b` との差分を確認。stream 0 の動作を変える production regression は見つからなかった。
- instance-wide の seq／ended／stop／version、stream ごとの状態、End の記帳、prune の局所性、device call 前の lock 解放、既存 thread 名と join 順序は維持されている。
- P242–P280 と当て直し19行を照合。当て直しの破壊対象は HEAD と同じ。全6リスト **1039行の `old`／`old2` は各1回一致**した。
- spec 18 の変更した Test 行と assertion を確認。追加の DA-1 違反は見つからなかった。
- build・test・mutation は実行していない。実行時の通過・kill の報告は未検証（**INFERRED**）。

## Verdict

**PASS with minors — blocker 0、major 0、minor 2。**  
上記のテスト修正と持ち越し補完を推奨。最終 gate の実行確認は別途必要。
