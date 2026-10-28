STATUS: done

# 段階 3g 報告（probe と音の判定）

## 作ったもの
- tone.rs：床を電力に（窓の平均電力 > 4×床，coherence > 0.5 は残す）．最初の窓から既に移っている場合は gapped にしない（`signal[0].0 != inside[0].0` のときだけ gapped）．doc と文言を noise power，閾値の書式を `{:.1e}` に．
- fake.rs `the_tone_classifier_finds_an_attenuated_shift_in_noise`：LSB 量子化雑音（tone 0.002）の 3 case（on time，F7 型 −19 ms 付近，雑音だけ）と「静かな窓が 5 つ」の case を追加．
- hardware.rs：helper `untimed_end_of_burst`．P4 は最初の tone を 30 000 sample，stop を device 時刻 e−5 ms，最初の stream の e−7〜e−5 ms の median を印字（分析 2c）．P5 は priming を 400 ms 先，印字する時刻を start＋tone.len()×200，3 つ目の case（reopen 後に時刻なしの空 EOB．`EZSDR_D9_P5_CASE=reopen`）．P6 は各 timed case の前，P7 は最初の tune の前に helper を呼び結果を印字．P6 の starvation は，queue した部分（start+1 ms〜queued_end）の |I|+|Q| の median の半分を閾値に，next の 100 sample 後から数える．
- plan/phase8/bench.md (c)：test 名の case 説明と「見る点」を新しい出力に合わせた．
- mutations.json：P994（old `4.0 * floor`），P998（old `signal[i].0 - (signal[i - 1].0 + SPAN) > 0`）を当て直し，P1045（`signal[0].0 != inside[0].0` → `true`）を足した．

## 分析から変えた所
- `*c > 0.5 && *power > threshold` の冗長な mutation 行は分析どおり足さない．
- P997 は電力の閾値になって（0 窓の floor が NaN で何も通らず）生き残ったので，「静かな窓 5 つで floor は None」の assert を test に足して殺した．
- P5 case 3 の verdict に「stale header を残してから消した」の文言を足した（case 1 と同じ「prerequisite established」だと誤読するため）．

## 確かめたこと
- `cargo test --workspace` rc=0，`cargo clippy --workspace --all-targets -- -D warnings` 通過，rules.py 0 problem，`git diff --check` 通過．tone の test 変更後は該当 test と radio-uhd の clippy を再実行．
- docker（volume `ezsdr-v4-probes-target`）で `--features uhd` の clippy 通過（hardware.rs の compile を含む）．
- mutation（maintenance,phase7，sim02:4 sim03:4）：P994 P995 P996 P997 P998 P1045 の 6 行すべて killed．
- 作業中に worktree が mode 700/600 で remote_mutate が `Permission denied` になったので，.git 以外を go+r(x) にした．

## していないこと
- 「LSB の回帰 case が旧規則で落ちる」ことは実行で確かめていない（旧 code を当てる build をしなかった．分析の Python 移植の主張のまま）．
- 音の判定の test は固定 seed で時刻に依らないので 20 回の繰り返しはしていない．
- 実機は流していない．hardware.rs は compile だけ．

## 明日の再実行の手順（bench.md (c)）
- P4〜P7 を今までどおり（att ≥ 30 dB の確認の上）流す．P5 は `EZSDR_D9_P5_CASE=timed|untimed|reopen` で 1 case ずつも可．P5 を先に流すと決め 5 の直しが効くか P6・P7 の前に分かる．
- 読む所：`noise power` と閾値，`moves_at`（約 −19 000 µs は「e より前（F7）」），P5 case A の `header ≥ e` が true か（false は INCONCLUSIVE），B・C の move が e か，P4 の `first stream's median e − 7..e − 5 ms`，P6 starvation の閾値（`over ...`）．MA-1 は再実行不要．

---

STATUS: done

3g 修正 1 回目（Sol FAIL の major 1・minor 1，両方採用）。
- Major（P6）：判定を純粋な関数 `tone.rs::continuation_in_loopback` に切り出した。queue した部分が 10 000 sample 未満か median 0 なら基準なし，続きの区間（next+100 sample から 1 900 sample）の捕れた sample が 1 500 未満なら欠けで，どちらも INCONCLUSIVE と理由。使える時だけ「queue 部の median の半分を越える sample の数」を出す。hardware.rs は送信の Underflow を「tx recovery」として別に印字し，受信の other（error）も印字する。
- Minor：`ToneReport.at_or_before`（最初の窓から移っていた場合）を足し，verdict は「leaves 100 kHz at or before ... µs」と印字。
- test：fake.rs `the_tone_classifier_finds_an_attenuated_shift_in_noise` に F7 型の「at or before」と on time の「at」の assert，新しい `the_p6_loopback_count_needs_a_baseline_and_a_captured_continuation`（Ok(1900)，Ok(0)，受信が queue 部を失った，queue 5 000 sample，median 0，空，続きの区間の欠け）。
- mutation：P994〜P998，P1045 に加え新しい P1047〜P1051 の 11 行すべて killed（sim02:4 sim03:4）。
- 確かめた：cargo test --workspace，clippy，rules.py（0 problem），git diff --check，docker（volume ezsdr-v4-probes-target）の --features uhd clippy，すべて通過。
- していない：実機，20 回の繰り返し（固定入力の純粋な test）。
