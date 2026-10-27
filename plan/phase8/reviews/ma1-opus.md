# 段階 MA1：orchestrator のレビュー，裁定，数

## 1. 何をしたか

D9a の Opus レビューの MA-1（owner の決め 2026-10-28，(a)：位相の保証は「受信が流れている間の timed の tune」に限る）．MockRadio の MR-34 を D9a の UHD Module の規則にそろえる risky な段階．

| 項目 | 中身 |
|---|---|
| 共有の判定 | `ezsdr_radio::timeline::covering`（RM-36）：その instant にその channel の sample が通る segment（origin ≤ t，cut が無いか instant(cut) > t）．UHD の `reach` はこれを呼ぶだけ（振る舞いは不変）．`mech:radio-rule-implemented-twice` を避ける |
| Mock の規則（MR-34） | 受信の周波数の更新は，適用の時に e を覆う受信 segment（link の有る stream だけ，`covering`）の origin が e − W（W は release の窓，X3xx で 3 ms）以下のときだけ timed tune．それ以外は untimed tune（`random_unless_timed_tune`・`random` は `<device>/lo/retune` から 1 回引く，`deterministic` は不変）．送信は timed tune のまま |
| Ceiling | e − W より後に始まった segment が覆う更新は UHD が timed で渡しても untimed と数える（保守的）．UHD の owner が refusal を処理し timed start を origin の 2 ms 前までに出すことを前提とする（Mock は host の遅れを模擬しない）．引きの 2 回は模擬の決まり |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 試験計画 | Sonnet 5.5 | Sol の計画レビュー 6 回：major 3 → 3 → 3 → 2 → 2 → 0 |
| 計画の裁定 | 適用の時の判定（r1）→ 消えない印（r2）→ 受け取りの時の覆いが前の plan を見られない（r3，拒否で早まる segment）→ 規則を「受け取りの時に始まっていた同じ segment」に狭める（orchestrator）→ owner の遅れを前提として明記（r4）→ UHD の欠陥を範囲外へ（r5） | |
| 実装 | 同じく Sonnet 5.5（worktree，3e と並行）．セルフレビュー 1 回（free は上限，Go の DeepSeek：P1 2，P3 4） | |
| Sol コード | | FAIL：major 1（MR-34 の Ceiling 2 つが受信に限っていない・例外が抜けている．spec の文だけ），note 1（UHD の `holds` の channel の絞りを殺す試験が無い） |
| 修正 | orchestrator が MR-34 の 2 文を直した | Sol 見直し PASS |
| 合流で見つけた穴 | orchestrator：Spec Run の時刻つきの Action は全部 Run の初めに Provider に渡る（KC-17/KC-18）ので，受け取りの時の規則では Spec Run の周波数の更新が Mock で常に untimed になる（UHD は release の時に決めて timed で渡す）．計画レビュー 6 回とコードレビューが見落とした | |
| 規則の差し替え | orchestrator の案「適用の時に覆う segment の origin ≤ e − W」．UHD は e − W より前に release しない（`control.rs:643-669`，Sol VERIFIED） | Sol：ADOPT WITH CHANGES |
| 修正 2 | Sonnet：受け取りの記録を消し窓の規則に．試験 4 本の期待を timed に，境界の試験，Spec Run の予約の regression（`v58.rs`），mutation P1026〜P1028・P1036 の置き換えと P1037．`mr_32_b_…` の更新の instant を窓の外へ | |

## 3. 数（AGENTS.md §9 step 8）

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（計画，6 回） | 13 | 9 | 判定の時点と plan の変化の順が UHD と Mock で違う（6：release と適用，flip-flop，拒否で早まる segment，owner の遅れ）．観測できない assert・殺せない mutation 行（5）．試験の時刻（2） |
| Sol（コード） | 1 | — | spec の文（受信に限る，例外） |
| orchestrator（合流） | 1 | — | 判定の時点：Action が Provider に渡る時刻（Spec Run は Run の初め）を規則が前提にしていなかった |
| セルフレビュー | 2 | 4 | mutation ID の衝突（merge の作業），MR-34 の文 |

計画の段のレビューで，Mock が実機より緩くなる系列が 4 つ見つかり，規則を狭めて塞いだが，狭めた規則（受け取りの時）は Spec Run を常に untimed にする逆の誤りを生み，合流の時に見つかった．最後の規則は UHD の判定の時点（release，e − W 以後）に合わせる．`radio-rule-implemented-twice` の機構：判定の算術は 1 か所（RM-36）になったが，判定の**時点**は Provider ごとに違うので，Mock は UHD が判定しうる最も早い時点で覆われている場合だけ timed とし，前提を spec に書いた．教訓：計画の系列は Session の push だけで書かれ，Spec Run の Action の渡り方を誰も確かめなかった．

## 4. 範囲外で見つかったこと

- **UHD の欠陥（D9a，Sol が code の道を VERIFIED）**：Deferred にした受信の周波数の値が，拒否の replan で宛先の origin が e より前に早まると装置に一度も届かない（境界 e > o3'）．到達は INFERRED（refusal の処理が release より遅れる順）．別の UHD の段で直す．同じ段で `reach` の `holds` の channel の絞りの回帰を足す．

## 5. gate

- テスト 1955 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 433，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1761/1762（G09 は既知）．mutation の ID は 3e と重なったので合流の時に P1022〜P1037 に振り直した（計画の記録の M1〜M16 と対応）．
