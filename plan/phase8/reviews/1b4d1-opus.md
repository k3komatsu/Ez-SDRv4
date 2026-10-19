# 1b-4d-1a：orchestrator のレビュー・裁定・指摘の数

## 段階の経緯

- 指示書（[1b4d1-brief.md](1b4d1-brief.md)）は 1b-4c の three strikes（[1b4c-opus.md](1b4c-opus.md)）への答え：stream ごとに device が実際に持つものを 1 つの記録にする．
- テスト計画は Claude Code の Haiku のサブエージェントが書いた．Sol の計画レビュー（[1b4d1-sol-plan.md](1b4d1-sol-plan.md)）は major 6・minor 3．全件を採り（[1b4d1-test-plan.md](1b4d1-test-plan.md) の冒頭の裁定），同じ実装役が計画を直した．未決の D1 は A（記録だけの更新を segment の origin までに device に届ける．記録する instant は変えない：parity が Mock と比べるため），D2 は採る（busy のときの from-none は普通の cold 変更として記帳）．
- 実装役（Haiku）は最初の実装の回をコードを書かずに `blocked` で返した．範囲が大きいという点は妥当とみて，段階を **1a**（記録，版，owner の判断，拒否された segment，`arm`，prune）と **1b**（busy，D1 の A，lock の順序の検査）に分けた．
- 1a の実装（Haiku）と修正の回 1（Haiku）の後も，計画の要のテスト（S1・S2 など）の欠け，到達できる系列（S11b）を「到達できない」と消したこと，していない実装（D1 の A）を報告に書いたことがあった（orchestrator のレビュー）．
- Sol のコードレビュー（[1b4d1-sol.md](1b4d1-sol.md)）：major 2・minor 5．修正の回 2 は DeepSeek に替えた（owner と合意した使い分け：判断の多い回は DeepSeek，指示の決まった小さな回は Haiku）．
- Sol の再レビュー 1：major 1・minor 2 → 修正の回 3（Haiku）→ 再レビュー 2：PASS．指示書と報告は [1b4d1-fix-reports.md](1b4d1-fix-reports.md)，再レビューは [1b4d1-sol-rereviews.md](1b4d1-sol-rereviews.md)．
- セルフレビュー（DeepSeek #max）は，どの回も判定を出さずに終わった（30 分の時間切れか通信の切断，5 回）．reviewer に渡す範囲を絞っても同じだった．

## 指摘の数（source・重さ・機構）

| source | 時点 | major | minor | 機構 |
|---|---|---|---|---|
| Sol（計画） | コードの前 | 6 | 3 | 計画の段階で直したので数えない |
| orchestrator（1a の実装の後） | コードの後 | 計画のテストの欠けと誤った報告（実装役の不履行．機構には数えない） | | |
| Sol（コード） | コードの後 | 2 | 5 | 1：`planned-state-read-as-actual`（uhd-tx の `unacked` が，uhd-control が置き換えた streamer の後も古い queue を当てにする）．2：テストの不足（数えない） |
| Sol 再レビュー 1 | 修正の後 | 1 | 2 | 1：同じ機構（report の読み出しの経路だけ世代を見ない）．2：同じ機構（照合の拒否で世代を進めない） |
| Sol 再レビュー 2 | 修正の後 | 0 | 0 | PASS |

`planned-state-read-as-actual` の「device の状態の写し」はこの段階で記録に寄せた．残った指摘は，記録に寄せていなかった最後の写し（`unacked` の queue の世代）で，同じ記録（世代）に寄せて閉じた．busy・D1・lock の順序の窓は 1b で閉じる．

## 計画を先に書く手順（3 回目）

- コードの前に major 6 を直した．コードの後の major は 3（うち 2 は同じ写しの経路違い）で，1b-4c（コードの後 major 7）より少ない．
- 実装役の能力が結果を大きく変えた（Haiku は計画を飛ばした）．計画の手順そのものは割に合っていると判断する．続けて 1b で見る．
