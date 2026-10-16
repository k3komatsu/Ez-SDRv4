# Sol 6.1 への依頼：Phase 8 設計 33 第3版と追加の判断のレビュー

あなたは独立したレビュー役です．**読むだけで，ファイルを一切変更しないでください**（build も test も不要）．リポジトリは /home/komatsu/local_work/Ez-SDRv4．AGENTS.md（§3，§6，§9）に従ってください．

前回あなたは設計 33 第2版を再レビューしました（`plan/phase8/reviews/rereview-sol.md`，R1〜R4）．同時に Opus が独立にレビューしました（`plan/phase8/reviews/rereview-opus.md`，P0 1件・P1 11件・P2 12件）．orchestrator は両方を採否して第3版にしました（`plan/phase8/33-channels-and-streams.md` の §9 に表）．その後 owner が次の3つを受理しました（`plan/phase8/00-overview.md` §8 の判断ログの最後の3行）：

1. **P6（設計 33 D2）**：Kernel の `KeyDecl` に番号の付く segment を1つ許し，`registry.key_decl` が完全一致の次にそれで引く（Phase 8 で唯一の Kernel の変更）．
2. **D16**：channel ごとの antenna の update class を `cold` にする（Session で antenna を変えられるように）．
3. **用語**：tree の root（kind `radio.device`）は instance 全体を表し，timekeeper は `t<i>`（`radio.timekeeper`）．timekeeper を2つ持つ instance の1つだけを Spec が binds することはできない．

## 見てほしいこと

1. この3つの判断が健全か．特に：D2 の Kernel 変更が最小で穴がないか（SB-2 の5つの位置，SB-45/46，KC-24，schema，登録時の重なりの検査）．D16 の `cold` の antenna が RM-21・RM-25・`ezsdr_radio::timeline`・RF envelope（RM-19，D7）・UHD での antenna の切り替えと read-back（UR-12）・Mock と整合するか，UHD の antenna 切り替えが本当に timed にできないか（UHD v4.10.0.0 で確かめる）．用語の判断が Vision §8・§25，SB-22c/SB-T2，SB-34・SB-35，8b の方向と矛盾しないか．
2. 第3版の変更が前回の R1〜R4 と Opus の指摘を本当に解消したか，新しい問題を生んでいないか：D3 の7〜9（移動の ready と止まっている stream，timeline に1か所），D8（利得は RF 段だけ．UHD の gain group の配分で ADC が本当に 0 のままか，`set_rx_gain` の aggregate と read-back，Basic/LF の範囲 {0}，UBX・OBX の全二重で RX2），D9（run していない stream の channel への timed 更新を送らない規則，queue を1列にした単純化），D13（能力・構成・記録の分離，coherence の成立と失効，Basic/LF を弱い値で宣言），D14（stream の node を宛先にした `UpdateParameter` と `Stop`），D15（thread と lock），§5 の段階（instance ごとに新旧どちらか一方の key）．
3. Gate P に進めてよいか．受理済みの判断を覆す提案は，重大な理由があるときだけ P0 として挙げること．

## 出力

日本語で標準出力に：結論（Gate P に進めてよい／条件つき／戻す）と一文の理由，3つの判断それぞれへの賛否と理由，前回 R1〜R4 の解消状況，新しい指摘（P0/P1/P2，対象，中身，根拠 `path:line` か URL，VERIFIED/INFERRED）．見つからなかったことは実行した検索を書く．
