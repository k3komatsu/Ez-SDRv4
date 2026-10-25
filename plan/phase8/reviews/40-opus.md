# 設計メモ 40：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [40](../40-preemption-bound.md) の D1〜D5．これで，走っている burst への割り込みで Mock が UHD より緩い所（overview の Mock が緩い表）を直した．risky な段階として，指示書の系列の表 → 実装役のテスト計画 → Sol の計画レビュー（2 回）→ 実装の順で進めた（AGENTS.md §9）．

- **語彙**：`radio.timing.tx_in_flight_ns` を `TimingEnvelope`・key 表・capability に足した（x3xx 10 ms INFERRED，ideal 0．schema を再生成．version は上げない）．`ezsdr_radio::timeline::later_bound`（通常の bound と同じ clock の前線の遅い方）と `late_policy_front_outcome`（前線で late になる burst の `LatePolicy` の結果）を両 Module が使う．Kernel は変えていない．
- **UHD Module**：定数 `IN_FLIGHT_WINDOW_NS` をやめ，description の値を読む．window を厳密な上限にした（D3：`now + W` 以後の最初の sample から先の payload は渡さない．空の EOB の zero sample 1 つは例外）．割り込みの判定は実際の cursor と通常の bound の遅い方．device burst を閉じた後の安全な再開（payload の EOB は終わり＋1，空の EOB は H＋2）を前線として持つ．`send` が一部だけ受け取られたときと失敗したときは，受け取られた数（失敗なら送ろうとした数）で cursor を進めて前線にする．一度 late と知らせた burst に，再判定や `preempt` が 2 度目の `TIME_ERROR` を出さない．
- **MockRadio**：受け取りの順で先の burst（同じ瞬間も）から保守的な手渡しの終わり `handed` を出し（D2''），W が 0 でないとき予約の前線を `handed ＋ 2 sample` にする（D5）．Stop は手渡し済みの burst を手渡した分まで流し（D4），payload を残す cut（`handed` と前の Stop の cut の大きい方）と，その後の予約の前線（cut ＋ 2）を分けて持つ．cold change と loss はその瞬間で切る．ideal（W＝0）は HEAD と同じ．
- **spec**：07（RM-4，RM-6，RM-15，RM-16，RM-20，RM-29），09（MR-3，MR-17，MR-25，MR-32），18（§3，UR-21，UR-23）．各 Changes に 1 行．
- **既存のテストの書き換え**：`mr_16_burst_refusals` の `moved_duplicate` を ideal へ移して重なりの拒否を保ち，x3xx の場合は前線へ動くことを見る（S2）．KE-03 の Stop の後の放射は 80 から 1 000 sample（D4：1 ms の PING は Stop の時に全部手渡し済み）．

実装は GPT-6 Luna（OpenCode2：最初の 2 回は GitHub Copilot，修正の回から OpenAI．owner 2026-10-25），セルフレビューは Haiku 5.5（6 回）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| テスト計画 | Luna（OpenAI） | 系列 1〜11，D1・D2 の確かめ，系列 11 の UHD のずれを確認 |
| Sol 計画 1 | | FAIL：major 5，minor 1（D2 が UHD の hand-over を単純にしすぎ）→ D2'，D3・D4 を owner に |
| Sol 計画 2 | | FAIL：major 3，minor 1 → D2''（受け取りの順で保守的に）．owner の承認の範囲で実装へ |
| 実装 | Luna（Copilot） | セルフレビュー 2 回の途中で opencode が "Compaction produced no summary" で終わった（報告なし）．続きの指示書で再開，セルフレビュー 3 回目まで．テスト 1,734 |
| Opus | | O1，O2（minor） |
| Sol コード | | FAIL：major 3（F1〜F3），minor 1（F4＝O1） |
| 修正 1 | Luna（OpenAI） | 途中で文脈を失い自分の実行を別の run と取り違えて止まった．続きで再開．Kernel に足した公開メソッドを戻させた |
| Sol 見直し 1 | | FAIL：major 1（G1，修正 1 が持ち込んだ），minor 1（G2＝O2 の残り） |
| 修正 2 | Luna | |
| Sol 見直し 2 | | FAIL：major 1（H1），minor 1（H2）．同じ機構の 4 つ目 → 設計メモ §7，owner が D5 を決定 |
| 修正 3（D5） | Luna | 場合分けを消し一律の ＋2 |
| Sol 見直し 3 | | **PASS** |
| gate の mutation | | 1403/1407．G09（既知）のほかに 3 つ生き残った：P639（W＝0 の guard が冗長，同値），B18（D3 で窓が send を切り，`ur_22` の場面が保留の tail を通らなくなった），E21（Mock の前線が 2 つ目の PONG を drop して，`v58_12` が rearm の忘れを見分けなくなった） |
| 修正 4 | Luna | P639 は guard と行を消した．B18・E21 はテストの場面を直して killed |

## 3. 裁定（退けたもの）

- review-2 P1（Mock の隣の burst の Kernel の EOB）：Kernel の burst の枠と device の枠は別．UHD は最後の sample を device lead の締め切りまで残すので，締め切り前に来た隣の burst は続く．
- review-4 P1（UHD の tail を 2 度数える）：continuation の cursor は保留した tail の後にあり，tail を送っても進まない（Sol も同意）．
- Sol P2-5（通常の bound の式が Mock と UHD で違う）：今回より前からの RM-14 の bound で範囲の外．
- UR-23 の INFERRED の注記（DA-1 の形）：accepted spec は INFERRED を規則の中に持つ．残した．

## 4. 数（AGENTS.md §9 step 8，`plan/maintenance/failure-mechanisms.md` に照らして）

コードの後の指摘（計画レビューの 8 major は計画の段で直したので数に入れない）：

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol | 5（F1，F2，F3，G1，H1） | 3（F4，G2，H2） | F1・F2・G1・H1：`radio-rule-implemented-twice`（Mock が UHD の device burst の枠を推し量る）．F3：`planned-state-read-as-actual`（送る前の cursor を，一部だけ受け取られた後も使った）．G2：`radio-rule-implemented-twice`（一つの Provider の中の 2 つの道） |
| Opus | 0 | 2（O1，O2） | O2：`radio-rule-implemented-twice`（同上）．O1 は Test 行の写し |
| セルフレビュー | P1 8（直した 4，退けた 4） | — | 直した P1：cold・loss の cut，INFERRED の注記，mutation の行の当て直し 2 |
| gate の mutation | — | — | 生き残り 3（P639 同値，B18・E21 は変更でテストの場面が外れた力の喪失．数には入れない） |

- **三度目**：`radio-rule-implemented-twice` の major が一つの段階の中で 4 つ（F1，F2，G1，H1）．局所の直しを止め，設計メモ 40 §7 で問い，owner が D5（Mock は device の枠を推し量らず一律の ＋2）を決めた．Mock の場合分けが消え，Sol の見直しは PASS．Mock は前線ちょうど（H，H＋1）を狙う burst に最大 2 sample 厳しい．
- **計画を先に書く手順**：計画レビューは 8 major を code の前に見つけた（D2 の単純化）．それでも code の後に major 5 が出た．うち 4 は D2'' が残した「Mock が device の枠を推し量る」設計の角で，計画の段では 1 sample の burst，2 回目の Stop，held の続きの Stop を系列に挙げていなかった．
- **opencode の長い run**：実装役が 2 度，文脈の圧縮で止まった（報告なしで exit 0）．指示書に「長い log を読まない，出力はファイルへ」と書いた後の回は止まっていない．

## 5. 残り

- uhd-tx は B を受け取りより後の周回で決めるので，その遅れの分 UHD の cursor は Mock の `handed` より先にありうる．x3xx の W（INFERRED）を Phase 8 の bench で測るときに含める（設計メモ §7）．
- 再開の隙間（＋1，＋2）が 1 Msps より上で足りるかは INFERRED（UR-23）．bench で．
- UHD の `preempt` の `if h < bound` だけを殺す mutation の行は無い．

## 6. gate

テスト 1745（失敗 0），clippy，`rules.py` 0 件，docker の uhd（clippy，lib 388，`uhd_api` 5），Python 3.14・3.9 各 57，mutation の全リスト 1405/1406（G09 は既知）．修正 4 の後に全部を流し直した．
