# 1b-4e-2：Sol 6.1 のコードレビュー

## 1. コードレビュー

判定：FAIL

**major 3件、minor 2件。** 連鎖 move の prune に実害があり、from-none の競合判定と mutation の等価判定にも穴があります。

## 指摘

### 1. major — 連鎖 move の prune が片側だけを消す【VERIFIED】

**箇所：** `crates/ezsdr-radio/src/timeline.rs:687–711`

境界を制限するとき、相手の**実際の prune 境界**ではなく、要求した watermark を参照しています。別の move によって相手の境界が止まっている場合、その制約が伝播しません。

**再現系列：**
1. c0 を A→B、`e=10, seq=1`。
2. 同じ c0 を B→C、`e=20, seq=2`。
3. B と C だけを `prune_stream(..., 30)`。

公開 API を使った probe の結果：

```text
B seq2=true, C seq2=false, move2=None
refuse B seq2=Ok(true); C halted=false
```

B は未完了の A→B により境界10で止まりますが、C は20を越えて prune し、B→C の記録まで消えます。その後の拒否が C に伝わらず、A1 と RM-34 に違反します。

**修正：** move の両側に対する実効境界を、制約が伝播しなくなるまで計算してください。floor の固定と move 記録の削除も、実際に両 item を畳める場合だけ行います。この系列と逆方向の prune 順序を、未 prune の `Lines` と比較する回帰テストが必要です。

### 2. major — from-none が、streamer 未取得の先行 segment を追い越す【INFERRED】

**箇所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:968–995`  
関連：`crates/ezsdr-radio-uhd/src/provider/core.rs:376–401, 1015–1017`

owner の open は先行する未処理 segment も待ちますが、from-none は他 stream の `DeviceSet::Open` しか確認しません。

**系列：**
- c0 の A→B と、後の B→none を予約する。B には有限の未処理 segment がある。
- A が解放を記録した直後、B がまだ開く前に、空の C へ c0 を join する。
- A/B の記録に c0 がないため、uhd-control が C を即座に開く。
- B は C の streamer を待つ。C は後の segment を開始して保持し続け、B の先行 segment が進まなくなる。

`device_gate` は呼出しを直列化するだけで、この先行関係を守りません。

**修正：** from-none の条件にも、他 stream の未処理の必要性を反映し、該当時は owner に委譲してください。既存の `needed_elsewhere` を利用でき、新しい待機機構は不要です。A の解放後・B の open 前で同期するテストを追加します。

この interleaving はコード上で追跡したもので、独立実行では未再現です。

### 3. major — P358 と P493 は等価 mutant ではない【VERIFIED】

**箇所：** `plan/maintenance/tools/mutations.json:7284, 8455`  
関連：`crates/ezsdr-radio-uhd/src/provider/tx.rs:731–751`、`crates/ezsdr-radio/src/timeline.rs:875–929`

**P358：** 「拒否された TX segment は必ず cut-at-origin」という根拠が成立しません。拒否より前に後続 Cold が予約されていると、拒否 segment に正の長さが残ります。公開 API の probe でも、拒否 segment が `origin=100, cut=100 samples`、つまり終了時刻200になることを確認しました。

この segment を読む turn では cut-at-origin の分岐を通らず、`to.refused` の guard が必要です。P358 を適用すると clock を採用します。

**P493：** TX の End は、予約済み segment を必ず消すわけではありません。後続 Cold も予約した系列では、origin より前の End 後にも対象 `Segment` が完全一致で残ることを probe で確認しました。したがって、lookup は `streams.stop` の代わりになりません。

**修正：** 両行の `equivalent` を外し、到達可能な系列を殺すテストに当ててください。「指定したテストでは差が出ない」は、等価性の証明ではありません。

### 4. minor — P494 の対象 guard は RX planner の不変条件と重複する【VERIFIED】

**箇所：** `crates/ezsdr-radio-uhd/src/provider/core.rs:1006–1008`  
`plan/maintenance/tools/mutations.json:8466`

RX の cut-at-origin segment は `timeline.rs:947–952` で `plan` から除外されます。直前に `streams.planned()` から取得した `current` に、その条件は到達しません。

P494 の等価性は認められますが、理由は「この Stop テストで先に戻る」より強い planner の不変条件です。

**修正：** 到達不能な RX 条件と対応する mutation 行を削除してください。

### 5. minor — S8 は projection と実効値を観測していない【VERIFIED】

**箇所：** `crates/ezsdr-radio-uhd/tests/fake.rs:6004–6034`

テストは timed `apply` の引数・予定実効時刻と、Manifest の claimed 値を確認しています。しかし、計画が約束した「移動後 configure の projection」と「device の実効値」は確認していません。

例えば、移動後の untimed configure が直前の retune を古い値へ戻しても、現在の assert は通ります。

**修正：** 移動後の configure が直前の timed 値を引き継ぐことと、後続 timed update 前後の実効値を追加確認してください。

## その他の確認

- **round-3 の修正：** RX/TX とも `streams` を解放してから `device_failed` を呼んでおり、指摘された再入 deadlock の修正は妥当です。対応する lost-owner テストも通過しました。
- **追加状態：** `active/completed` は有限 segment の処理完了、`busy_seq/move_source_busy` は source configure の結果確定前の destination open を防ぐ役割があります。`needed_before` は origin 条件を持ち、idle 解放用の `needed_elsewhere` と同一ではありません。
- **ロック：** 確認した経路では `device_gate → streams` の逆順取得は見つかりませんでした。native close は open と同じ `control` mutex を取得しています。
- **UHD 4.10：** streamer destructor、disconnect callback、graph の edge 削除を一次ソースで再確認しました。実機の別-stream move 後の連続 sample は未確認です。
- **仕様：** UR-25 の追加文は要求事項として書かれており、rationale の混入は認めません。ただし、指摘1・2により現状の実装は記載した保証を満たしません。
- **MockRadio：** 差分はありません。

## 独立検証結果

- timeline の `rm_34_*`：**39 passed**
- UHD lib：**238 passed**
- fake の `ur_34_*`：**21 passed**
- VH-8 move：**2 passed**
- 変更された mutation 行59件：`old` / `old2` は各々一度だけ一致。
- 公開 API の probe で、指摘1と指摘3の planner の反例を確認。

ソースファイルの変更、mutation 実行、commit・push はしていません。UHD feature build、実機 bench、全 gate は再実行していません。**指摘1〜3を解消してから再レビューが必要です。**

## 2. 修正の回 1 の再レビュー

判定：PASS

**新規の major / minor 指摘はありません。前回の5指摘は修正済みです。**

## 修正の確認

1. **前回1（major）：連鎖 move の prune — 解消【VERIFIED】**  
   `crates/ezsdr-radio/src/timeline.rs:687–726`  
   実効境界を不動点まで反復し、両側の境界が move を越えた場合だけ release floor を固定しています。  
   独立 probe で A→B→C→D、A→B→A、長い循環について、RX/TX・prune 対象と順序を変えた **2,880ケース**を確認しました。未 prune の `Lines` と plan が一致し、保持した move の release・拒否伝播・prune 完了後の再計画も一致しました。panic はありません。

2. **前回2（major）：from-none の追越し — 解消【VERIFIED】**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:968–970,1439–1537`  
   streamer 未取得でも、他 stream に未処理の必要 segment があれば owner に委譲します。回帰テストは B の open 前で同期し、C の予約中に device call がないことと、**A close → B open → B close → C open** を観測しています。独立実行でも通過しました。

   隣接条件も確認しました。`core.rs:375–384` は拒否済み・完了済み・長さゼロの segment を除外するため、それらだけでは追加の待ちを作りません。必要性のない enable の既存テストも通過しています。ただし、拒否済み／完了済みだけを残した from-none の専用 interleaving は独立実行しておらず、その結合動作は **INFERRED** です。

3. **前回3（major）：P358 / P493 の等価判定 — 解消【VERIFIED】**  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:1127–1163`  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:1594–1651`  
   両行の `equivalent` は削除済みです。テストはそれぞれ、正の長さを持つ拒否 TX segment、Provider stop 後も完全一致で plan に残る TX segment を明示的に確認しています。以前の「先に別条件で戻る」系列ではありません。

4. **前回4（minor）：到達不能な RX guard — 解消【VERIFIED】**  
   `crates/ezsdr-radio-uhd/src/provider/core.rs:985–1004`  
   RX cut-at-origin guard と P494 は削除済みです。planner の除外規則は `crates/ezsdr-radio/src/timeline.rs:962–967` に残っています。

5. **前回5（minor）：S8 の観測不足 — 解消【VERIFIED】**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:6021–6047`  
   destination open 後の untimed configure が 10 MHz を引き継ぐことと、後続 timed update 前後の device 実効値が **10 MHz → 20 MHz** となることを確認しています。独立実行でも通過しました。

## predicate 統合・mutation

- `needed_elsewhere` は削除済みです。`needed_before(..., i64::MAX)` は origin 条件を全 `i64` に対して満たすため、`release_unneeded` の必要性判定を変えません。close／保持の両テストも通過しました。
- P523・P524、再設定された P252・P253・P489・P496・P508、および P358・P493について、変更箇所と失敗を観測する assertion の対応を確認しました。
- `impl.log:1882–1901` に対象 **14/14 killed** の remote 実行記録があります。今回は mutation を再実行していません。
- maintenance **878行**、phase7 **165行**の全 `old` / `old2` が、それぞれ source にちょうど1回一致しました。

## 独立検証

| 対象 | 結果 |
|---|---:|
| timeline | 53 passed |
| UHD lib | 240 passed |
| fake | 269 passed |
| parity | 20 passed |
| 連鎖・循環 prune probe | 2,880ケース通過 |
| `git diff --check` | 問題なし |

リポジトリのファイル変更、mutation 実行、commit・push はしていません。全 workspace gate、UHD feature build、Python、実機 bench は未実施であり、実機の inter-stream 動作は今回の判定では保証しません。
