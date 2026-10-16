# Sol 6.1 への依頼：Phase 8 設計 33 第2版の再レビュー

あなたは独立したレビュー役です．**読むだけで，ファイルを一切変更しないでください**（build も test も不要）．リポジトリは /home/komatsu/local_work/Ez-SDRv4．AGENTS.md（§3，§6，§9）に従ってください．

前回あなたは `plan/phase8/00-overview.md` と `plan/phase8/33-channels-and-streams.md` の第1版をレビューしました（`plan/phase8/reviews/plan-sol-review.md`，P0 1件・P1 12件）．orchestrator は全件を採って第2版にしました（33 の §8 に扱いの表）．その後 owner が D1・D9・D13 を受理し，D13 に「shared LO（LO の共有）」の段を加えました．

## 見てほしいこと

1. 前回の13件それぞれが，第2版で本当に解消したか（解消／一部／未解消，根拠つき）．
2. 修正が新しい問題を生んでいないか：特に D3 の「channel の移動」（`ezsdr_radio::timeline` との整合，RM-16，RM-25，halt），D5 の平らな selector（SB-4 で本当に書けるか，`id` との関係，Mock と UHD の共通部分の境界），D8 の ADC 利得の分け方（UHD 4.10 の `x300_radio_control.cpp` の gain group），D9 の「合計 ≤ 1本」と §59，D13 の3段（aligned・coherent・shared LO）と Vision §25 の CoherentGroup の `basis`，`random` の追加，Basic/LF の `deterministic`（INFERRED）．
3. owner が受理した判断（00-overview §8 の判断ログ）を前提にして，Gate P に進めてよいか．受理済みの判断を覆す提案は，重大な理由があるときだけ P0 として挙げること．
4. 段階（33 §5，00-overview §5）が各段階で tree を緑に保てるか．

## 出力

日本語で標準出力に：結論（Gate P に進めてよい／条件つき／戻す）と一文の理由，前回13件の解消状況の表，新しい指摘（P0/P1/P2，対象，中身，根拠 `path:line` か URL，VERIFIED/INFERRED）．見つからなかったことは実行した検索を書く．
