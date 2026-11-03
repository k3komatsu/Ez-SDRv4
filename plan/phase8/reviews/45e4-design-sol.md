# Verdict: REVISE
分割の方向は採るべきですが，容量の共有範囲，再試行の契機，D と残存義務の定義を E4 前に直す必要があります。P1 はありません。
手順上の制約：読取りの除外ミスで草案の箇条書きも先に見えました。以下はコードに照らした検証ですが，「草案を読む前」の独立した回答ではありません。

## コードから導く数え方
- **K：装置ごとの保守的な予約容量。** cell への予約から数え，「列にある／取った／効いたかもしれない」は時刻を過ぎても数える。成功・部分成功の完了後は `device_now >= e` で外す。未適用が確定した拒否・取消しは外す。これは UR-24 の booking 上限とは別物。
- **D：chain ごとの resident entry 総数。** 予約から，call の終結と残存する消費義務の解消まで数える。K に数える値も含む。報告済み `left` も，後続 sample に必要なら残す。
- 容量の解放は pin，消費義務，restore の許可を解放しない。`Consumed` が `Done` より先に届く順も扱う（`crates/ezsdr-radio-uhd/src/brain/core.rs:627-674`）。
- 時刻は R の進捗ではなく，既存の root 時刻源から runtime が input に載せる。現状の `Input::Timer` は時刻を持たない（`crates/ezsdr-radio-uhd/src/brain/core.rs:201-203`）。

## Findings
### P2-1 — chain ごとの K は共有容量を守らない
`plan/phase8/45-uhd-decision-core.md:141`；`crates/ezsdr-radio-uhd/src/device.rs:1420-1455`
- 反例：`共有queue容量=K; chain AにK件予約; chain Bに1件予約; 全件のeは未来` → 両 chain の判定は通るが，共有 queue が満ち，設定の呼出しが待つ。native lock を介して stop も遅れうる。
- **VERIFIED：** FakeDevice は RX/TX・channel をまたぐ一つの `commands` 列を使う。`:413` の 16 はその列の容量で，chain ごとの容量ではない。
- **VERIFIED：** UR-24 の booking も instance 全体で，held・given・refused を将来の instant まで数える（`crates/ezsdr-radio-uhd/src/provider/control.rs:656-666`）。
- **VERIFIED：** X310 を「装置全体に物理 queue が一つ」とするのも誤り。UHD は RFNoC block ごとの queue を記述し，X300 の radio control は block ごとに一つの timed interface を使う。同じ radio block の制御は共有され，DDC の sample 消費 FIFO は別である。[UHD timed_cmd.dox:110-115](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/docs/driver_usage/timed_cmd.dox#L110-L115)，[x300_radio_control.cpp:124-143](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp#L124-L143)。
- 推奨：E4 の K は装置全体の保守的な software budget にする。物理 slot 数や「一 tune＝一 register command」と同一視しない。安全な具体値は **INFERRED／要測定**。

### P2-2 — 最早 instant の timer だけでは held が進まず，既存 drain timer とも衝突する
`plan/phase8/45-uhd-decision-core.md:141`；`crates/ezsdr-radio-uhd/src/brain/core.rs:436-439,616-674`
- 反例：`K=1; AをTaken; eを越えてcall継続; Bをheld; Aが成功してDone` → held 時点には「終わった」値がなく timer を置けない。Done で再試行しなければ，R が止まっている限り B は永久に held。
- 別の反例：`drain中; 容量timerがdrain期限より先に発火; 既存Input::Timerを処理` → 現実装では即 stop し，drain を早く終える。
- Done・拒否・取消し・消費・時刻更新で再評価する。まだ満杯なら次の将来 instant に再予約する。timer は用途と対象 epoch／予約を識別し，重複・旧 timer で stop／再開しないこと。
- 「最新の timer 時刻」は，**正しい root の時刻を input が運び，更新が R に依存しない場合だけ**妥当。現在の timer は時刻を送らない（`crates/ezsdr-radio-uhd/src/brain/threads.rs:955-982`）。期限そのものを現在時刻と取り違えない。

### P2-3 — D の対象が曖昧で，resident work の上限にならない
`plan/phase8/45-uhd-decision-core.md:142-144`
- 反例：`Dは容量離脱済みだけを数える; その数=D-1; K件が未到来Done; 全件のeが来る` → 新規予約なしでも証拠待ちは D を超える。K と別の排他的な列として扱うなら，記載した上限は守れない。
- D は**予約時から全 resident entry を数える**のが最小で明確。満杯で held にするのは，無断の捨て・合併・偽の消費よりよい。ただし設定遅延という観測可能な代償がある。
- 「R が D 個分遅れたときだけ」も不正確。call 継続中，次の sample が来ない `left`，停止後の残存義務でも占有する。

### P2-4 — cut による「始末」と entry の削除を結び付けてはいけない
`plan/phase8/45-uhd-decision-core.md:142`；`crates/ezsdr-radio-uhd/src/brain/core.rs:705-715`
- 反例：`Aを適用; Stopでcut<e; drain期限; AをLeftと報告して削除; 次segment開始` → 装置に残った A を後続 sample が消費しても追跡できず，restore の抑止も失う。
- 草案の「cut の証拠で始末するまで」は，既存の残す手順（同 note `:130`）と衝突する。記録を一度だけ確定することと，物理的な残存義務を消すことを分ける。

### P3-1 — 現行との差と旧説明を訂正する
`plan/phase8/45-uhd-decision-core.md:138,142`
- 現行の等価物は `RxChain.debt`／`left`。push・移送・sample による除去に resident 件数の上限はなく，「上限なし」は **VERIFIED** にできる（`crates/ezsdr-radio-uhd/src/provider/core.rs:141-142,239-259,307-325,367-370`）。
- cell work は実際に件数依存。clone と再照合が列を処理する（`crates/ezsdr-radio-uhd/src/brain/threads.rs:853-867`，`cell.rs:379-385`）。D は理由があるが，大きくすれば無料ではない。
- `:138` の単一上限と「新しい見える Ceiling にはしない」は更新が必要。D で従来より設定が遅れる可能性とは両立しない。

## 操作列と草案の対応
| 操作列 | 判定 |
|---|---|
| 一 chain に K 超，R 停滞，device 時刻だけ進む | 分割の方向は正しい。再試行の完全性が不足（P2-2）。 |
| call が e を越える／部分失敗 | 未完了を数える点は正しい。部分失敗も残存義務を保持する。 |
| 未適用の拒否／Queued の取消し | 容量解放は正しい。held の再評価契機を追加。 |
| Stopping／Stop が cut を短縮 | 既存の受入閉鎖・取消し・pin は維持。cut だけで削除しない（P2-4）。 |
| plan cancel／epoch／古い recv | 既存手順は維持。旧 timer が新 epoch を動かさない条件が不足。 |
| pins／drain／LateCommand | K 解放と独立に維持。容量 timer を drain・再開命令として扱わない。 |
| loss／panic | 既存の failure gate を維持。容量解放後も held を再 dispatch しない。 |
| 複数 chain／timer と Done の競合 | 共有 K と冪等な再評価が不足（P2-1・2）。 |
| D 到達／報告済み left が残る | held は妥当。対象と限界を訂正（P2-3・4）。 |

## E4 に必要な case・oracle・測定
- **K 超：** R を停止し，同 chain に K+2 件。device 時刻と Done だけで held が順に進む。全 ID を保持し，消費は捏造しない。
- **遅い call：** K=1，Taken のまま e を越す。未完了中は解放せず，遅い Done 後は消費 input なしで再試行する。成功・部分失敗・拒否を分ける。
- **D 到達：** resident D 件＋held。timer では D を解放せず，正当な除去後に再試行。cut 報告だけでは left を失わない。
- **交差：** 二 chain の共有 K，同時刻の複数値，取消しと take，Stop 短縮，epoch 交代，Done／消費／timer の全順序，重複・旧 timer，loss／panic 後の timer。
- oracle は独立に容量 predicate と resident 数を検査し，pin・left・exactly-once disposition・許可なしの Stopping 再開禁止も維持する。進行性は公平な配送と call 完了を前提に検査する。
- **INFERRED：** K による E2b の結合は除けるが，有限 D では同じ結合が閾値 D に移る。固定した K/D で，別々の hit 数・最大占有・held 滞在・R backlog・cell 時間・全更新の適用遅延を測る。D-hit が遅延に先行すれば「移しただけ」である。

## 置換案（§4.5）
> **上限は二つ**．値の記録は一つに置き，命令の状態・値の時刻・消費と残存義務から数える．別々に更新する容量の旗や列は持たない．
> **装置の容量 K** は装置ごとの保守的な予約枠で，UR-24 の booking の上限とは別である．予約した値は「列にある」「取った」「効いたかもしれない」の間数える．成功・途中まで効いた失敗で終わった値は，装置の時刻が値の時刻以上になるまで数える．未適用が確定した拒否・取り消しは数えない．容量が空いても，ピン留めと消費の義務は消えない．
> 装置の時刻は runtime が既存の root の時刻源から input に載せ，受信の進捗は使わない．完了・取り消し・消費・時刻の更新で held を順に再評価する．時刻待ちなら最も早い将来の instant に timer を置く．容量の timer は drain と区別し，古い timer は現行の状態を確かめるだけとする．
> **保持する値の上限 D** は chain ごとの全 resident entry を予約から数え，K に数える値と報告済みの `left` も含む．cut での報告は消費の証拠ではない．未完了の呼び出しと残存義務がなくなった値だけを除く．
> どちらかが満ちれば held に残す．D は cell の写しと照合を抑えるが，受信の遅れや残存義務が続けば設定を遅らせうる．現行の debt／left にこの件数上限はない．K/D は測定前に固定し，占有・上限への到達・待ち時間を別々に測る．

保守性：状態から導出する方針はよい。ただし「容量」「記録の確定」「残存義務」は別の事実であり，一つの完了フラグに潰さないこと。
Not checked: ビルド・実行・実機容量測定。編集・git 状態変更なし。禁止された holdout は未読。
