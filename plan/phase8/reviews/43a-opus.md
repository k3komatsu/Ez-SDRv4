# 段階 43a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [43](../43-refusal-keeps-origins.md)（owner が採用，2026-10-28）の実装．D9a の Deferred の値の消失（KL-33）と D9a2 の継ぎ足しで続いた指摘の根を断つ．

| 項目 | 中身 |
|---|---|
| RM-25 | 拒否された受信の segment は origin の計算では走ったものとして扱い（仮想の segment），plan・clock・記録・装置の仕事には出さない．後ろの origin と move の release は拒否で早まらない |
| 送信（owner：送受がずれない） | 拒否された送信の clock の後継は拒否が無かった場合の cut から．clock の記録の end は origin のまま．move の release が読む予定の end は仮想の cut を `omitted` に保つ |
| RM-6 規則 2 | 受信の start lead ≥ release window．`TimingEnvelope::check_start_lead` を `Description::build`，`with_measurement_timing`，`candidate_timing` の 3 か所で．受信の start lead の測定の候補は `[3,5,10,25,50,100] ms`（実機で 1 ms は 2026-10-28 に記録済み） |
| UHD Module | production は測定の道の validate だけ．`rx.rs` の拒否の guard は残し comment を直した |
| 文書 | design/07 RM-25・RM-34・RM-6，design/09 MR-34（Ceiling の半分が消えた），design/18 UR-24，KL-7・KL-15・KL-33（直った） |

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 設計メモ | Opus．Sol 2 回（ADOPT WITH CHANGES ×2） | 「origin を早めるのは拒否だけ」を狭い義務 O1・O2 に，時刻の前提を規則 2 として強制 |
| 試験計画 | Sonnet 5.5 | Sol 3 回：major 6 → 2 → 0 |
| 実装 | Sonnet 5.5．HEAD の基準の値を先に流して計画と照合（一致）．セルフレビュー 1 回（free DeepSeek：P1 0，P2 1，P3 2，直した） | mutation 163 行中 159．J04・P102・P608 は試験を足して kill，J25 は等価になり引退 |
| Sol コード | | FAIL：major 1（送信の move が source の拒否の後に 1 lead 早まる：`released()` が拒否された clock の記録の end を読む），minor 3 |
| 修正 1 | 仮想の cut を `omitted` に，timeline・Mock・UHD の回帰，burst の手渡し，Stop で release が実際に減る case，P1076，P1079〜P1081 | Sol 見直し PASS |

## 3. 数

| 出所 | major | minor | 機構 |
|---|---|---|---|
| Sol（設計メモ，2 回） | 3 | 3 | 証明の主張が広すぎる（O1・O2），時刻の前提が強制されていない，試験の観測 |
| Sol（計画，3 回） | 8 | 4 | 期待値・観測（試験だけ），測定の道の例外（規則 2 の抜け道） |
| Sol（コード） | 1 | 3 | `planned-state-read-as-actual`：拒否された送信の clock の記録の end を別の line が予定として読む |
| セルフレビュー | 0 | 3 | 文 |

設計メモ 43 で `planned-state-read-as-actual` の答えとした規則（拒否は予定の origin を動かさない）の，送信の move の読み手への抜けが 1 件．読み手を `released()` 1 か所にまとめて閉じた．

## 4. gate

- テスト 1995 全部通った．clippy，`rules.py` 0 件，docker の uhd（lib 441，`uhd_api` 5，`--features uhd` の clippy），Python 3.14・3.9 各 57．
- mutation の全リスト 1800/1802：G09（既知）と P606．P606 は `mr_20_a_change_the_plan_drops_is_no_candidate` が e+100 ms で止まり，仮想の segment の後で e+109 ms に始まる 2 つ目の clock を見ていなかった（試験だけの修正 2：e+200 ms まで進め clock が 1 つを assert）．P604・P606〜P608・P1034 を流し直して kill．
