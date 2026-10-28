STATUS: done

段階 43a 報告（詳細は 43a-test-plan.md §13「実装で足したこと」）

- HEAD の基準：T6 の o2=65 ms・o3=124 ms・o3'=70 ms，T-X1=1200 ms は計画どおり．T3-a と T1-d の仮想 2 case は HEAD で落ちることを確認（release 1000 ms，S3 の origin 早まり）．
- 作ったもの：timeline.rs（`refused_running`・`refused_clock`，cut/finish/prune/released），規則 2（`TimingEnvelope::check_start_lead`，`Description::build`，`with_measurement_timing`，`candidate_timing`），受信の候補 `[3,5,10,25,50,100]`/rehearsal `[3,5]`，試験，spec 07/09/18，KNOWN_LIMITATIONS（KL-7・KL-15・KL-33），mutation 行 P1052〜P1075．
- 計画から変えた所：計画の rehearsal tx `[1,2]` は誤りで実際は `[1]`（不変）．検査関数は B3 のため Result．既存の試験の期待を計画 §5 のほか 6 本直した（§13）．Mock の fault は move と近い instant だと move を拒否するので T5 Mock は change 10 ms・move 100 ms．
- 確認：cargo test --workspace 1992 通過（0 失敗），clippy -D warnings 通過，rules.py（--renames 付き）0 problems，git diff --check 問題なし．
- mutation：timeline.rs の全行＋Mock/UHD の拒否・MR-34・測定の行＋新 24 行，計 163 行．159 killed．J04（start_rx on 動作中の stream：試験に case を足して killed），P102（Lines::refuse の settle：release が変わる固定 case を足し filter を変更して killed），P608（拒否の book：filter を `mr_20_a_start_rx_resumes_a_refused_stream`（mock_layout）に変更して killed）．**J25（rx.rs `if !refused { from = origin + 1 }`）は生き残り**：RM-25 の変更後，拒否された segment の origin に再 plan された segment が来る経路が無くなり等価．行の削除か rx.rs の guard の簡素化は orchestrator の判断（UHD Module の本体は変えていない）．
- セルフレビュー（free DeepSeek，1 回）：P1 0，P2 1（design 09 の 170→175 ms と design 18 の 65→119 ms）は直した，P3 2（KL-33 の When を過去形に，design 18 Changes 行の文）は直した．
- 残り：m9 の「記録の cut を仮想 cut にそろえる」変種の行は作れなかった．docker の uhd・Python・全 mutation list は gate．

---

STATUS: done

43a 修正 1 回目の報告

1. Major（送信の move が拒否の後に早まる）：直した。timeline.rs の `Cold` で `refused_clock` を取る所に，拒否された clock の仮想の cut を `omitted`（move の release だけが読む）に入れる 1 行を足した．clock の記録の end は origin のまま．試験：timeline `rm_34_a_transmit_move_after_a_refused_source_change_waits_the_cut_the_clock_would_have_had`（release 300，宛先 350，記録 150。拒否を予約時／後から），Mock `mr_20_a_transmit_move_after_a_refused_source_change_begins_as_if_not_refused`（初め空の宛先，登録された clock の origin 350 ms，sample 4 000 の burst が 354 ms に遅れず再生，C の clock の end 150 ms／拒否なしは 300 ms），UHD `ur_25_a_transmit_move_after_a_refused_source_change_begins_the_destination_as_if_not_refused`（core，宛先の segment と登録された clock の origin 350 ms，C の clock の end 150 ms）．
2. Minor（UHD の送信の試験に burst）：control.rs の 2 本に burst の送出を足した（`submit_burst`：sample 16 000 を 354 ms，繰り返しの拒否は sample 1 000 を 551 ms として uhd-tx へ渡る root の instant を観測．拒否の前・後・間の 3 通り）．装置の send は uhd-tx の thread を要するので観測せず，uhd-control が uhd-tx へ渡す target までで確かめた．
3. Minor（Stop と prune）：`rm_34_a_refusal_before_a_stop_or_in_a_chain_does_not_change_a_release` を作り直し，拒否された segment の origin（1 050）と予約済みの move（1 200）の間（1 100）に Stop を置き，release が 1 200 から 1 100 に実際に減ること，宛先が 1 250 から 1 200 になることを assert してから拒否あり／なしを比べる．chain は prune の前の plan と release を取り，prune の後と比べる．
4. Minor（m9 の記録の cut）：P1076（timeline）を足し，拒否の処理の `self.end_at_origin(stream)?;` を `self.cut(stream, item, false)?;` にして `rm_25_a_transmit_change_after_a_refusal_begins_as_if_not_refused` の記録の end で killed．同じ行の Mock・UHD 版（P1077・P1078）は生き残った：Mock と UHD は clock の end を拒否の記録の時点で決め（後継を予約する前），後継の予約では end が変わらないので，この変異は両者の観測では等価．そのため 2 行は外した（記録の cut は timeline の試験だけが見る）．move 側の行 P1079〜P1081（omitted の push を外す）は 3 つの試験で別々に killed．
5. J25：引退（mutations.json から外した）．理由：RM-25 の変更後は拒否された segment の origin に再 plan される segment が来ず，カーソルを進める guard の有無が観測に出ない（等価）．別の観測で当て直す形は作れない（guard は rx.rs:315-320 に残す．到達はする）．rx.rs:283-284 の古い comment を「次の segment を元の所に保つ」に直した（comment のみ）．
確かめたこと：cargo test --workspace 1995 通過・失敗 0，clippy -D warnings 通過，rules.py（--renames）0 problems，git diff --check 問題なし，mutation：timeline.rs の全行＋拒否・MR-34・測定の行＋新しい行（計 168 行）のうち 166 killed，残り 2（P1077・P1078）は上のとおり外した．
spec：design/07 の RM-34 に「拒否で origin 終わりにした送信の clock は後継の cut まで数える」の句と新しい試験 3 本を足した．


---

STATUS: done

P606：(a) で直して killed。原因：試験の場面（同じ instant の 2 つの change）は plan が落とした change と空でない plan が今も両立している．変わったのは観測：変異は最初の change を拒否するが，RM-25 の変更後はその拒否された segment が仮想として走り続け，2 つ目の change の segment は origin e+109 ms（仮想の cut + 呼び出し 1 ms + 余裕 3 ms + lead 50 ms）に始まる．試験は e+100 ms で止まっていたので，その clock がまだ登録されず，正しい拒否（2 つ目）と変異（1 つ目）が区別できなかった．直し：mock_radio.rs の試験を e+200 ms まで進め，2 つ目の clock が始まらない（clock 1 つ）ことを assert．P606 killed．
確認：cargo test -p ezsdr-mock-radio 通過，clippy -D warnings 通過，git diff --check 問題なし，P606 と同系の P604・P607・P608・P1034 も killed（5/5）。
