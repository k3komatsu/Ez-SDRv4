# 段階 3f：queue の深さの試行を成功 5 回まで続ける（指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．repository /home/komatsu/local_work/Ez-SDRv4（main の tree）．main の tree を触るのはあなただけ．小さな段階で risky ではない（測定の道具．本番の `src/` は変えない）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（周りの書き方に手で合わせる）．ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- `src/` は変えない．仕組みを足さない（今の loop と分類をそのまま使う）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．触ったファイルは mode 644．
- Sol は起こさない．実機は流さない．**起きたことを書く．していないことは書かない．**

## 1. owner の決め（2026-10-28）

queue の深さは今，条件ごとに `queue_trials`（5）回の試行を流し，5 回とも成功のときだけ値になる（`crates/ezsdr-radio-uhd/tests/common/measurement.rs` の `measure_command_queue_depth`，`queue_depth_value`，`summarize_queue_depth`）．3e で目標をまたぐ試行は `Ambiguous`（成功に数えない）にしたので，2026-10-27 の 12 回の読みは成功 6・あいまい 3・不完全 3 で，5 回続けて成功する見込みは低い．

決め：**成功が 5 回そろうまで，条件ごとに最大 15 回まで試行を続ける**．15 回でそろわなければ今どおり `unmeasured`．要る成功の数（5）と証拠の強さは変えない．`Ambiguous` は数えないまま．

## 2. 作るもの

- `MeasurementPlan` に試行の上限（既定 15，env `EZSDR_MEASURE_QUEUE_MAX_TRIALS`，`queue_trials` 以上でなければ validate で拒む．record の u32 の上限の確かめ（今の `queue_trials * 3` の所）は上限の側で）．`queue_trials` は「要る成功の数」の意味になる：名前を変えるか doc で明確に（名前を変えるなら env 名と既存の試験も）．
- loop：条件ごとに，成功が `queue_trials` に達したら止める．上限に達したら止める．device の失敗・`raw.halted()` で止めるのは今どおり．交互の遅れ（試行の番号の偶奇）は今どおり続ける．
- `summarize_queue_depth` は「成功の数 ≥ 要る数」で判定し，値は成功だけから（今どおり）．`unmeasured` の理由の文に「最大 N 回で M 回の成功」を書く．observation に条件ごとの試行の数と成功の数を残す（`ambiguous_trials` と並べて）．record の `trials` の数は実際に流した数．
- 試験（純粋なものを優先）：(a) 成功・あいまい・不完全が混ざる列で，5 回目の成功で止まり以後の試行を流さない，(b) 15 回で 5 回にならなければ `unmeasured` で 15 回流した，(c) 上限 < 要る数を validate が拒む，(d) fake の rehearsal（`fake.rs:719`，`:722`，`:920`，`:1298-1299`）の期待を新しい数え方に合わせる（fake で全部成功なら各条件 5 回で止まる）．
- mutation：止める判定（成功の数 ≥ 要る数で止める，上限で止める），validate，`summarize_queue_depth` の判定に行を足す．変えたコードや killing test に当たる既存の行も当て直す．
- spec：`design/18-uhd-radio.md` UR-34 の queue の規則（375 行あたり）に「成功が要る数そろうまで，最大の回数まで」を DA-1 の形で（Test 行に新しい試験），*Changes* に 1 行（段階 3f）．`plan/phase8/bench.md` の queue の所要時間の見積もりと env の説明を直す．

## 3. 確かめること

- 純粋なテストは 1 回．時刻に依るもの（fake の rehearsal）は 20 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（名前を変えたテストは `--renames` のファイルを作業 dir に書いて渡す），`git diff --check`．
- mutation：新しい行と当て直した行を repo の root から `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ID> ...` で（触ったファイルは mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．

## 4. セルフレビュー

しない（小さな段階．Sol が見る）．

## 5. 報告

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-3f.md` と最後のメッセージ．最初の行は `STATUS: done`（止めたら `STATUS: stopped` と理由）．作ったもの，指示書から変えた所と理由，テスト名，spec の変更，mutation の行と結果，確かめたことの結果，残ったこと．
