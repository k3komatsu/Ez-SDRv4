# 1b-6a：Sol 6.1 のレビュー

設計メモ 35 の 2 回のレビューの扱いは [35-uhd-stream-state.md](../35-uhd-stream-state.md) §7．

## 1. テスト計画のレビュー

判定：FAIL

R1 の範囲と基本方針は妥当ですが、**S2・S4a・S8 を誤実装でも通せる観測の穴**があります。実装前に計画を修正してください。

対象は HEAD `8cd89e4`、作業ツリーは clean。以下の行番号の `test-plan.md` は依頼された作業ディレクトリ内のものです。既存コードの説明は確認済み、未実装テストや mutant の成否予測は **INFERRED** です。

## 指摘

### 1. major：S4a は「待ち関数に入った」だけで、読みを実際に待ったことを固定しない

**場所：** `test-plan.md:68,255–257`、`crates/ezsdr-radio-uhd/src/provider/tx.rs:2269–2285`、`provider/control.rs:692–706`

- **根拠：** `wait_reading` probe は、`reading` を調べるループの**前**にあります。既存テストはそこで control を止め、`tx_open == 0` を確認した後、control と読みの gate を続けて解放します。
- **系列：** `await_reading` に入るものの、ループ内で `reading` を無視して直ちに進む誤実装でも、確認時点では probe 自体が open を止めています。その後 TX が先に読み終えれば、追加予定の holder／reading assert も含めて通ります。N3 は遷移関数を調べますが、その結果を control が使わない誤りは固定しません。
- **修正：** 読み gate を保持したまま、control が実際の待ち判定を一度行ったことを同期して観測してください。その判断点で「待ちを選択し、open に進んでいない」を確認してから読みを解放します。sleep や scheduling の偶然を oracle にしないでください。

### 2. major：N10 の総 lock 取得回数では、拒否前の早すぎる返却を検出できない

**場所：** `test-plan.md:181–191,324–326`、`provider/core.rs:1049–1052`、`provider/rx.rs:308–318`、`provider/tx.rs:755–765`

- **根拠：** N10 は Streams lock の取得回数と最終状態だけを見ます。取得回数は、どの処理が同じ節にあったかを記録しません。TX の確認も「必要なら」に留まっています。
- **系列：** `owner_configure` が configure 失敗の記録と同じ節で先に返却し、呼び出し側が後の節で拒否だけ book する誤実装でも、取得回数は予定の **4 回**、最終状態は `refused + Free` にできます。その節間に move destination が開けます。既存 move unit test は source の拒否と解除をテスト自身が実施するため、production の返却順序の証明にはなりません。
- **修正：** RX／TX の両呼び出し側について、拒否処理に入る時点で source がまだ `Owner { seq }` を保持する観測を必須にしてください。move は両 line の拒否を確認し、その同じ lock 節で返却します。lock 回数は補助確認に留めます。

### 3. major：release の取得後 loss が、S8 の網羅から抜けている

**場所：** `test-plan.md:321–323`、`provider/core.rs:925–951`

- **根拠：** 計画自身が、claim 取得後・close 前の loss 分岐を決定的に踏めないと認め、review-only を候補にしています。
- **系列：** この分岐だけ `give_back` を忘れても、その経路を実行しないテストでは Drop の panic は働きません。N7 は Drop 機構の確認であって、各出口を通した確認ではありません。
- **修正：** claim 取得後、次の loss 検査前に同期点を置き、loss を publish して再開します。`Free`、close なし、返し忘れによる panic なしを確認してください。device gate の内側で close を止めるだけでは、この分岐の代わりになりません。

### 4. minor：遷移表の「前が違うとき」に抜けがある

**場所：** `test-plan.md:79–144`

- **根拠・系列：**
  - N3 は `may_call_from_none` を FromNone のときしか調べません。`!reading` だけを返し、holder を検査しない実装でも通ります。
  - N3 は FromNone 保持中の再度の `claim_from_none` を確認しません。
  - N1 の Owner 保持中の `claim_release` は、既定の `idle = false` によっても拒否されます。Owner の排他検査を落としても、この assert は通ります。
- **修正：** 不正な holder の検査では他の前提を成立させ、拒否理由を一つにしてください。`may_call_from_none` の Free／Owner／Release、FromNone の二重取得、idle・active なしの Owner に対する release 拒否を追加します。

### 5. minor：N8 は本物の Claim の unwind で二重 panic しうる

**場所：** `test-plan.md:160–165`

- **根拠：** N8 は本物の Owner Claim を保持してから、偽の Claim を返して panic させます。偽の Claim を `mem::forget` しても、本物の Claim は残っています。
- **系列：** 本物を束縛して保持すれば、期待した panic の unwind 中にその Drop も panic し、テストプロセスが abort します。保持しなければ、誤返却に到達する前の Drop が原因になる可能性があります。
- **修正：** 本物の Claim を外側に保持し、誤返却だけを `catch_unwind` で捕まえます。panic と holder 不変を確認し、最後に本物を正常返却してください。

### 6. minor：N9 の準備では receive stream 1 と move の line がない

**場所：** `test-plan.md:167–176`、`provider/core.rs:135–145,233–245`

- **根拠：** `Streams::with_streams(1, 2)` が用意する RX state は一つです。N9 の destination は RX stream 1 であり、さらに `with_streams` は timeline の line を挿入しません。
- **系列：** 記載どおりでは move の準備に失敗し、意図した `move_source_busy` の assertion まで到達できません。また「別 stream の seq を変える」だけでは、source の seq 比較を固定できません。
- **修正：** RX state を二つ用意し、source／destination の line を挿入します。同じ source で Claim を返して取り直し、`Some(7)`／`Some(8)`／`None` を比較してください。private field の直接変更は不要です。

## A–G の推奨

| 問い | 推奨 |
|---|---|
| **A** production の mismatch | **状態を変えない案を採用。** 他の holder を解除しない。test build の panic は維持し、N8 の二重 panic を避ける。 |
| **B** accessor | **採用。** production の guard 用に `holder`、観測用に `reading`。production が `may_call_from_none` だけで足りるなら、`reading` は test 専用でよい。 |
| **C** P385 | **残して当て直す。** `Option::take` による早期返却は表現でき、既存 `cold_booking` probe が保持を直接観測する。 |
| **D** release 後の loss | **同期点を足してテストする。** review-only は不可。指摘3。 |
| **E** S2 の観測 | **lock 回数だけでは不可。** 拒否時の holder を RX／TX とも直接観測する。指摘2。 |
| **F** `read_reports_before_close` | **`begin_read` 失敗時の skip を採用。** 成功した読みだけ `end_read` と対にする。到達しない分岐のために新たな状態や機構を増やさない。 |
| **G** `Tx::pass` の取得失敗 | **device を呼ばず false。** 可能なら `begin_read` を plan 変更前、同じ lock 節で判定し、失敗時には owner の状態も動かさない。 |

## その他の確認結果

- **T1／T2：** gate によって両順序を作る方針は妥当です。raw thread が manual clock の participant にならないこと、`apply` が `tx_async` と同じ family ではないことは現コードと一致します。ただし、実装・実行前なので決定性の最終確認は **INFERRED**。gate は起動前に arm し、失敗時も解放・join できる構造にしてください。
- **既存23か所の置換：** 対象一覧と置換方針に欠落・観測の弱体化は見当たりません。ただし release の通常テストでは Claim を忘れず保持・返却し、`mem::forget` は detach の模型に限定するのを推奨します。
- **mutation：** `.busy/.busy_seq/.reading` を old/new に持つ **17行＝R1対象14行＋host側3行** は確認できました。J279／J295／J312 は対象外、P505／P512 は現在の照合文字列を保つなら当て直し不要です。
  - P387／P388／P396／P402 も計画どおり別途追跡が必要です。
  - P384 を「from-none 自体を選ばない」に変える案は、元の「enable は行うが所有を取らない」から狙いが変わります。取得遷移の欠落を試す行として維持してください。
  - P398 は新しい `begin_read` が第二の guard になるため、旧 guard の削除だけで元の誤動作を再現できるか再確認が必要です。生き残りを隠すために観測を弱めないでください。
- **範囲：** R2、loss の仕組み、#67 に踏み込む scope creep はありません。UR-25 の改訂箇所の列挙も妥当です。

**推奨：指摘1〜3を必須として、4〜6と mutation の狙いを計画へ反映してから実装へ進む。**

未実施：コード変更、テスト実行、mutation、実機確認。ファイル・プロセスは変更していません。

## 2. コードレビュー

判定：FAIL

major はありません。minor 1件です。本体の所有権取得・返却に、新たな不具合は確認できませんでした。

1. **minor — `Release` を取得する thread の説明が実装と違う**
   - **場所：** `design/18-uhd-radio.md:269`、`crates/ezsdr-radio-uhd/src/provider/core.rs:183`、同 `:303`
   - **根拠（VERIFIED）：** 新しい説明では、uhd-control が idle streamer を閉じる間を `Release` としています。しかし production の `release_unneeded` の呼び出し元は `Rx::begin`（`rx.rs:301`）と `Tx::pass`（`tx.rs:177`）だけです。`control.rs` の呼び出しはすべてテスト内です。
   - **系列：** owner が次の仕事を持たず、別 stream がその channel を必要とすると、uhd-rx／uhd-tx 自身が `Release` を取得して close します。規範文が担当 thread を誤記しています。
   - **修正：** 規範文とコメントを「idle streamer の owner が閉じる間」に訂正してください。実装を uhd-control に移す必要はありません。

### 確認結果

- 成功・拒否・loss・stop・待ちの中断における claim の返却と、拒否の booking 前の保持を確認。
- S4a の待ち、S2 の holder、release の claim 後の loss は、probe と同期で観測されています。
- `Tx::pass` の読み開始順序の変更に、到達可能な挙動差は確認できませんでした。
- mutation 22行の再標的化を確認。`old` は各1回一致し、`new` に削除済み識別子はありません。UR-25 の列挙テスト名も実在します。
- 独立実行：`cargo test -p ezsdr-radio-uhd` **515 passed**、`rules.py --base 8cd89e4` **0 problems**、`git diff --check` 成功。

ファイルは変更していません。mutation の再実行、UHD feature／実機、workspace・Python の全 gate は未確認です。
