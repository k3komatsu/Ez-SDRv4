# 段階 43a：拒否で後ろの origin を早めない（RM-25）と，受信の start lead ≥ release window（RM-6）．いまの仕事は試験計画だけ

あなたは実装役（Claude Code の Sonnet サブエージェント）です．**この回の仕事は試験計画を書くことだけ**で，コードは書かない．repository /home/komatsu/local_work/Ez-SDRv4 は **読むだけ**（tree には別の段の未 commit の変更がある．code は `git show HEAD:<path>` で HEAD `afbb125` を読む．設計メモは tree の `plan/phase8/43-refusal-keeps-origins.md` を読む）．出力は作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/43a-test-plan.md` だけ．`cargo fmt`・mutation の道具・commit・`git stash` を使わない．cargo も流さない．Sol は起こさない．

risky な段階（予定の origin，profile の timing，Mock と UHD が共有する `ezsdr_radio::timeline`）．AGENTS.md §9 step 2：系列ごとに，テスト，何を assert するか，どう観測するか，それを落とす間違った実装．

## 1. 仕様

設計メモ 43（owner が採用，2026-10-28）がそのまま仕様：§5 の (d) と規則 2，§6 の試験の系列 T1〜T9，§7 の次の段，§8 の処置の表と「直さないもの」．owner の決めの補足（メモの Status）：
- 送信と受信を同じ周期で変える実験で，受信だけ拒否で origin が早まると送受がずれる．**送信側にも拒否で後ろの origin を早める動きがあるかを code で確かめ**（メモ §8 の送信の行，`timeline.rs:938-941,977-979`），あれば同じ規則にそろえる計画を書く（無ければ path:line で理由）．
- 規則 2 は ms 単位の時刻のための安全側の決まり（将来 FPGA で）．

D9a2（worktree `Ez-SDRv4-d9a2`）は捨てる．そこから何も持ってこない．

## 2. 範囲

- `crates/ezsdr-radio/src/timeline.rs`（RM-25 の拒否，`Omitted` の doc，RM-34 の仮想の release の会計），RM-6 の規則 2（description を作る所で validate，公開するすべての description の試験），`design/07` RM-25・RM-34・RM-6，`design/09` MR-34 の Ceiling の半分（refusal を処理する前提），`design/18` UR-24 への 1 句，`KNOWN_LIMITATIONS.md` KL-7（拒否の後の遅れ，遷移ごと）と KL-33（直ったら status を直す）．
- 期待が変わる既存の試験（メモの列挙：`crates/ezsdr-radio/tests/timeline.rs:269-271`，`:833-842`（prune の 2 case），`:1084-1095`，MR-34 の 2 本，それに `mr_20_*` と `ur_25_a_refused_*` を洗う）．期待は新しい正確な値に直し，下限に緩めない．VH-8 の比較は保つ．
- UHD Module の code は変えない（メモ §5）．変える必要が出たら止めて計画に書く．
- 範囲の外：送信の R4・R2・F7（D9b），profile の値そのもの（段階 4）．ただし規則 2 で今の profile の値が通ることを確かめる．

## 3. 計画の形

系列ごとに：テストの名前（`rm_25_…`，`rm_6_…`，`mr_…`，`ur_…`），置き場，何を assert するか（拒否された segment が plan・clock・記録・装置の仕事に出ないこと，生き残る origin と release の正確な値，prune の有無で同じ予定），どう観測するか，どの間違った実装なら落ちるか．Session と Spec Run の両方．壊れる既存の試験の一覧と新しい期待．spec の変える規則の一覧（DA-1 の形の文案）．mutation の行の候補．最後に疑わしい所（メモの INFERRED：O2「時刻 b の予約は b + start lead より前に覆いを足さない」など）．

## 4. 報告

`43a-test-plan.md` に計画．最初の行は `STATUS: plan`．最後のメッセージは計画の要約．

---

# 段階 43a：拒否で後ろの origin を早めない（RM-25）と受信の start lead ≥ release window（RM-6）（実装の指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．risky な段階で，試験計画は Sol の計画レビュー 3 回で PASS した．repository /home/komatsu/local_work/Ez-SDRv4（main の tree）．main の tree を触るのはあなただけ（実機の記録は commit の写しで流れるので tree の編集とぶつからない）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（周りの書き方に手で合わせる．長い行はそのまま）．ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- 範囲は計画 v3 と設計メモ 43 のとおり．UHD Module の production code は変えない（計画が要ると言う所を除く．出たら止めて報告）．送信の R4・R2・F7，profile の値（段階 4）は変えない．**計画に無い仕組みを足さない**．
- **production を変える前に**，計画が言う HEAD の基準の値（T6 の定数 o2 = 65 ms，o3 = 124 ms，o3′ = 70 ms，T-X の値ほか）を HEAD の code で実行して記録し，計画の値と合うかを確かめる．合わなければ止めて報告．
- 試験の判定がテストの thread の時機に依らないこと（設計メモ 37）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．負荷をかけない（**実機の記録が走っている間は heavy な build を避ける**：`~/ezsdr-bench/2026-10-28/` の最新の run の `measurement.json` か，`ps` に `hw_measurements_record` が無いことを確かめてから重い cargo を流す）．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．作ったファイル・変えたファイルは mode 644（cherry-pick や Write で 600 になる）．
- Sol は起こさない．docker の clippy と uhd の試験は orchestrator が gate で流す．実機は流さない．
- **起きたことを書く．していないことは書かない．**

## 1. 読むもの

- 試験計画 `~/.cache/ezsdr-fixes/2026-10-26-stage3/43a-test-plan.md`（v3．これが仕様）．
- 設計メモ `plan/phase8/43-refusal-keeps-origins.md`（owner が採用）．
- Sol の計画レビュー 3 回（`sol-43a-plan{,2,3}.log` の最後の `> plan` 以降．読み方：`awk '/^> plan/{last=NR} {a[NR]=$0} END{for(i=last;i<=NR;i++) print a[i]}' <log> | grep -vE "^\S*(→|✱|⚙|✗)"`）と設計メモへの 2 回（`sol-note43{,-2}.log`）．
- `AGENTS.md` §2（DA-1，KNOWN_LIMITATIONS），§9．

## 2. 作るもの

計画 v3 のとおり：`timeline.rs` の RM-25（拒否された受信の segment は origin の計算では走ったものとして扱い，plan・clock・記録・装置の仕事には出さない），送信も同じ規則（計画の T-X），`Omitted` の doc，RM-34 の仮想の release，規則 2（RM-6：構成の境目 3 か所の validate，private な timing を取る builder は最小に，`new` は薄い委譲），受信の start lead の測定の候補（`[3,5,10,25,50,100] ms`，rehearsal `[3,5] ms`），試験（計画の一覧．期待が変わる既存の試験は計画の新しい正確な値に），spec（`design/07` RM-25・RM-34・RM-6，`design/09` MR-34 の Ceiling の半分，`design/18` UR-24 の 1 句，*Changes*），`KNOWN_LIMITATIONS.md`（KL-7 の遅れは遷移ごと，KL-33 の status を直った事実に，KL-15 など計画の文案），mutation の行．計画を広げた所は計画の末尾に「実装で足したこと」．

## 3. 確かめること

- 純粋なテストは 1 回．時刻に依るものは 20 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（名前を変えた試験は `--renames` のファイルを作業 dir に書いて渡す），`git diff --check`．
- mutation：計画の行と，変えたコードや killing test に触れる既存の行（timeline の RM-25・RM-34 の既存の行，MR-34，UR-25 の拒否の行を grep で）を repo の root から `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:4,sim03:4 <ID> ...` で（触ったファイルは mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．

## 4. セルフレビュー（1 回）

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-43a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-43a-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．**終わるまで待ってから報告する**．free が失敗したら同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 5. 報告

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-43a.md` と最後のメッセージ．最初の行は `STATUS: done`（止めたら `STATUS: stopped` と理由）．作ったもの，HEAD の基準の値の照合，計画から変えた所と理由，テスト名，変えた既存の試験と新しい期待，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．
