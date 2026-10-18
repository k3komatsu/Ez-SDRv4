テスト計画を読む前の独立シナリオを、以下で固定します。

1. RX/TX の同じ `k` と、同方向の異なる `k` に別々の変更を予約 → cut・configuration・clock・idle が対象 stream 以外へ漏れない。
2. 初期 channel 0 → cold enable → disable → 再 enable → owner が初めから存在し、正しい stream 番号で open/start/send/recv する。RX link なしでは受信を始めない。
3. RX の異なる stream で異なる量を配信 → orderly stop／abort／loss → 各 segment 自身の delivered floor で終了し、別 stream や旧 segment の値を使わない。
4. cold 変更を予約 → owner が device の設定を拒否 → 対象だけ halt、後続の成功変更で復帰。TX の拒否された clock は origin で終了する。
5. cold 変更の前後に timed update を追加・release・cancel → effective order の projection と instance 全体の queue 上限・release 順序が保たれる。
6. stream A の segment 終了・prune → stream B の未実行変更を拒否／開始 → B の item・configuration が早まって消えず、A の不要な configuration は回収される。
7. TX の将来 clock に burst を予約 → 途中の switch／拒否／Stop → 正しい clock の予約だけ維持・取消し、別 stream の burst・ACK・tail と混ざらない。
8. 各 RX owner が missed start／overflow／cut を処理 → restart・untimed stop・packet allowance・event handle・link がその stream 自身のものになる。
9. device call を意図的に止め、その間に別 thread が booking／stop／loss → line lock を取得でき、call 後も最新 plan に従う。
10. 複数 owner を起動 → stop／cleanup、うち1本が panic／wedge → 全 sender・join を処理し、TX-before-RX、bounded join、detach 時の非 close、既存 thread 名・Manifest 形を保つ。
11. 複数 line に対して loss／Provider stop を二度通知 → 全 line に一度だけ End が入り、loss は同時刻の変更より先に効く。
12. 異なる packet size の RX stream を用意 → cold restart → 各 line の ready allowance が自分の packet size を使う。未 open なら 0 と block_len の最大を使う。

以下、計画との照合に進みます。
採用された追加テストに、現行の正しい挙動では満たせない TX clock の期待値と、指定した誤実装を検出できない mutation 案が見つかりました。`prune` を全 line に適用しない裁定は妥当ですが、今回追加する stream 単位の prune を直接守るテストが不足しています。
## 1. 独立シナリオ（計画を読む前に固定したもの）

1. RX/TX の同じ `k` と、同方向の異なる `k` に別々の変更を予約 → cut・configuration・clock・idle が対象 stream 以外へ漏れない。
2. 初期 channel 0 → cold enable → disable → 再 enable → owner が初めから存在し、正しい stream 番号で open/start/send/recv する。RX link なしでは受信を始めない。
3. RX の異なる stream で異なる量を配信 → orderly stop／abort／loss → 各 segment 自身の delivered floor で終了し、別 stream や旧 segment の値を使わない。
4. cold 変更を予約 → owner が device の設定を拒否 → 対象だけ halt、後続の成功変更で復帰。TX の拒否された clock は origin で終了する。
5. cold 変更の前後に timed update を追加・release・cancel → effective order の projection と instance 全体の queue 上限・release 順序が保たれる。
6. stream A の segment 終了・prune → stream B の未実行変更を拒否／開始 → B の item・configuration が早まって消えず、A の不要な configuration は回収される。
7. TX の将来 clock に burst を予約 → 途中の switch／拒否／Stop → 正しい clock の予約だけ維持・取消し、別 stream の burst・ACK・tail と混ざらない。
8. 各 RX owner が missed start／overflow／cut を処理 → restart・untimed stop・packet allowance・event handle・link がその stream 自身のものになる。
9. device call を意図的に止め、その間に別 thread が booking／stop／loss → line lock を取得でき、call 後も最新 plan に従う。
10. 複数 owner を起動 → stop／cleanup、うち1本が panic／wedge → 全 sender・join を処理し、TX-before-RX、bounded join、detach 時の非 close、既存 thread 名・Manifest 形を保つ。
11. 複数 line に対して loss／Provider stop を二度通知 → 全 line に一度だけ End が入り、loss は同時刻の変更より先に効く。
12. 異なる packet size の RX stream を用意 → cold restart → 各 line の ready allowance が自分の packet size を使う。未 open なら 0 と block_len の最大を使う。

## 2. Findings

以下、`TP` は `/home/komatsu/.cache/ezsdr-fixes/2026-10-18-phase8-1b4b/test-plan.md`。

### major 1 — panic テストが、現行では終わらない TX clock の終了を要求している

**根拠:** TP:303。TX の `End` は現在の segment を cut せず `ended` を記録するだけ（`crates/ezsdr-radio/src/timeline.rs:820-830`）。origin が loss より前の running segment は、そのまま残る（同:867-873）。`reconcile` も cut のある clock だけを終了する（`crates/ezsdr-radio-uhd/src/provider/core.rs:181-185`）。

**修正:** 採用した「TX SampleClock の `ended_at`／plan の `end` が Some」を撤回する。TX line に loss の `End` が入ることは項目4の unit で観測し、panic の integration は message・termination・mark_lost を検査する。今回、期待値に合わせて TX の意味を変えてはいけない。

### major 2 — 採用した mutation と killer の組に、検出できないものがある

- **TP:824:** `..=(origin, u64::MAX)` → `..(origin, u64::MAX)` は、通常の seq の等号 instant を除外しない。さらに T1 の cold instant は segment origin より前なので、origin 境界の検査にならない（`control.rs:86`、`timeline.rs:912-930`）。
- **TP:835-840:** 全予約を 300 ms にすると、「timed を cold の床まで上げる」誤実装でも instant は変わらない。
- **TP:630:** shutdown の `expected + 1` を `ur_26_a_stopped_stream_keeps_its_end` で殺す説明は成立しない。Abort のケースは先の `rx.poll()` で終了済み（`rx.rs:828-834`）。cut も `min` で維持される（同:98-100）。

**修正:** origin と同じ instant を純 `ColdConfig` テストで検査し、mutation は `(origin, 0)` 未満など、本当に等号 instant を落とす形にする。床の検査には cold=300 ms／timed=100 ms の対照を追加。shutdown の行は、その枝まで到達する専用 sequence に結び直す。**現案の生存予測は INFERRED（実行禁止のため未実行）。**

### major 3 — 今回新設する stream 単位 prune の regression test がない

**根拠:** TP:11,1091 の問題認識は正しい。現行 `Streams::prune` は対象 line だけ（`core.rs:139-145`）、`Lines::prune` は全 line（`timeline.rs:495-519`）。既存 J49〜J51 の killer は片方向の rig なので、他方向の予約を消す誤実装を守らない（`rx.rs:1124-1145`、`tx.rs:1034-1081`）。

**修正:** RX/TX の2 line を使い、A の prune 後も B の item・configuration が残り、B の予約を拒否できる unit を追加する。A の不要な configuration の回収も同時に assert。新 API が move 非対応なら、その制約と、move 導入前の再検討を明記する。「既存行を変えない」こと自体は安全性の根拠にならない。

### major 4 — lock spy の「every」が owner の configure を観測していない

**根拠:** TP:911-913 の RX/TX spy は `apply` を監視対象に含めない。しかし RX/TX owner の configure は `apply` を呼ぶ（`rx.rs:281-282`、`tx.rs:655-662` → `control.rs:94-101` → `core.rs:445-446`）。その call の間だけ line lock を持つ誤実装は、control 用 spy では検出できない。

**修正:** owner 側も `apply` を監視する。各テストに、実際に通す sequence と call の到達確認を列挙する。少なくとも same-channel rate switch、channel 数変更、cut の stop、TX tail/EOB、report read、control の RX/TX enable と timed release を区別する。spy が一度も呼ばれず緑になる形を防ぐ。

### minor 1 — join-order テストは wedge 到達を同期する

**根拠:** TP:215 は repeat を送ってから finish するが、両 owner が wrapper 内に入った確認がない。既存 helper も submission と origin 待ちまで（`tests/fake.rs:1035-1041`）。

**修正:** wrapper から RX/TX の入口到達を通知し、両方を確認してから finish。30 s は watchdog として使い、約2 s を厳密な合格時間にしない。永久 thread より、検査後に解放できる gate が望ましい。未同期時の負荷依存失敗は **INFERRED**。

### minor 2 — real/manual clock の説明に誤りがある

**根拠:** TP:900,908 は `FakeConfig::default()` を理由に integration を real clock と扱うが、`Direct::build` は `fake(config)` を使い（`tests/fake.rs:620`）、その helper が Manual に上書きする（`tests/common/mod.rs:298-303`）。

**修正:** clock mode は helper まで追って訂正する。既存テストの lock 検出力についても、その前提で再評価する。

## 3. 裁定・一覧の確認

- **妥当:** instance-wide の `seq`・`ended`・`stop`・`version`、全 line への一度限りの End、Prepared／sender／thread の列化。
- **妥当:** `rx_packet_samples(k)` を line ごとに読む裁定。`0..channels` は channel 番号なので置換しない。
- grep で production の stream 0 呼出し **16箇所**を確認。final (a) の一覧は一致した。
- mutation の provider 行数 **maintenance 53／phase7 81**も一致。現在の全134行で `old` は各1回一致した。実装後の当て直し対象は形次第なので、再照合が必要。
- 1b-4c 送りは概ね独立シナリオ1・3・7・8・10・12の複数 stream 部分を含む。ただし final (c):1100 の「全 line End の形は4cで決める」は冒頭裁定と矛盾する。**実装は4b、複数 stream での検証は4c**に統一する。
- 独立シナリオ6の cross-line prune は、複数 stream の公開を待たず今回検査できる。延期しない。

## Verdict

**修正後に実装へ進める。blocker はなし、major 4件を先に直すこと。**  
編集・build・mutation 実行は行っていない。
