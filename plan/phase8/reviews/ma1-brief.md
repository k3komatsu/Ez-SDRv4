# 段階 MA1：MockRadio の LO の位相を D9a の規則にそろえる．いまの仕事は試験計画だけ

あなたは実装役（Claude Code の Sonnet サブエージェント）です．**この回の仕事は試験計画を書くことだけ**で，コードは書かない．repository /home/komatsu/local_work/Ez-SDRv4 は **読むだけ**（別の agent が今 tree の `crates/ezsdr-radio-uhd/tests/`，`plan/phase8/`，`design/18-uhd-radio.md` などを編集中．コードは `git show HEAD:<path>` で HEAD `a9b0a26` を読む）．出力は作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/ma1-test-plan.md` だけ．`cargo fmt`・mutation の道具・commit・`git stash` を使わない．cargo も流さない．Sol は起こさない．

## 1. 背景

D9a（`plan/phase8/42-d9-production-fix.md`，`plan/phase8/reviews/d9a-opus.md`）で，UHD Module は timed の受信の周波数を，その instant を走行中の segment が覆うときだけ装置に timed で渡し，それ以外は宛先の configure か release の時の untimed で当てるようになった（`crates/ezsdr-radio-uhd/src/provider/control.rs` の `give` まわり，覆う規則と宛先の規則）．untimed の tune の後の LO の位相は，OBX・UBX の timed tune の保証（`random_unless_timed_tune`）を持たない（MA-1，`plan/phase8/reviews/d9a-opus-review.md`）．

MockRadio の MR-34（`design/09-mock-radio.md`）は「適用された周波数の更新はすべて timed tune」として，`random_unless_timed_tune` の channel に timed-tune の定数を当てる．segment の origin は untimed tune（最初の segment を除く）．そのため，最初の segment の T0 より前の周波数の更新などで，Mock は位相が揃うと記録し，UHD では揃わない．Mock が実機より強い約束をする（AGENTS.md §3：Mock は実機より緩くてはいけない）．

owner の決め（2026-10-28，MA-1 の (a)）：位相の保証は「受信が流れている間の timed の tune」に限る．spec 18 の UR-24 と `KNOWN_LIMITATIONS.md` に Ceiling（別の段階 3e が書く）．MockRadio を同じ規則にそろえる（この段階）．

## 2. この段階の範囲

- **規則を 1 か所に**：「その instant に，その channel を含む拒否されていない segment が走っているか」の判定を `ezsdr_radio`（`timeline::Lines` か `channels` の近く．HEAD で UHD の `control.rs` が使う形を読み，どこに置くと両方が同じ関数を呼べるかを決める）に 1 つ置き，UHD Module の `give` と MockRadio の両方がそれを使う．UHD の振る舞いは変えない（D9a の試験がそのまま通る）．`plan/maintenance/failure-mechanisms.md` の `mech:radio-rule-implemented-twice` を避けるため．
- **Mock の位相**：周波数の更新の effective instant `e` で，その channel の stream が上の判定で走っていなければ，その更新は untimed tune として扱う（`random_unless_timed_tune` と `random` は `<device>/lo/retune` から 1 つ引き直し，coherence は invalid．`deterministic` は 0 のまま）．走っていれば今の timed tune のまま．送信の channel は D9a で変えていない（R4 は D9b）ので，送信の周波数は今のまま timed tune として扱うか，D9a の UHD の送信の振る舞い（burst の間の値は次の burst の先頭で当たる）と合わせて考えて，計画で決める（理由を書く）．
- **乱数の順**：引き直しの順は MR-30 の決定性を保つ（更新の適用の順）．既存の seed の再現の試験が変わるなら，どれがなぜ変わるかを挙げる．
- **記録**：MR-27 の `segments` の `coherence`（`basis`，`valid`）が新しい規則を表す．
- **spec**：MR-34 の文（DA-1 の形），MR-18 か MR-27 で変わる所，spec 07 に共有の判定を置くならその規則，VH-8（Mock と UHD の対の比較）が位相を比べるかを確かめ，比べるなら合わせる．

範囲の外：UHD の送信（D9b），profile の値，Kernel．

## 3. 系列（計画で各々にテストを当てる．足りなければ足す）

| # | 系列 | 期待 |
|---|---|---|
| 1 | `random_unless_timed_tune` の受信 channel に，T0 より前の instant の周波数の更新 | Mock は untimed tune：位相を引き直し，coherence invalid．T0 の最初の sample から新しい位相 |
| 2 | 1 の後，受信が流れている間に timed の更新 | timed-tune の定数，coherence valid |
| 3 | 受信を Stop した channel に更新，その後 restart | 今の規則（restart の origin は untimed tune）と合わせて，invalid のまま．引き直しの数が二重にならないか |
| 4 | 先の planned segment に宛てた更新（今の channel は stream の外） | untimed．その segment の origin の untimed tune と合わせた引き直しの数と順 |
| 5 | `e == origin`，`e == cut`（cut が判定の時に分かっている） | 境界の扱いが UHD の D9a の覆う規則と同じ（共有の関数なので同じになることを試験で固める） |
| 6 | `deterministic` と `random`（CBX）の channel で 1〜4 | `deterministic` は 0 のまま，`random` は今と同じく引き直し |
| 7 | 同じ seed の 2 回の Run | 同じ位相と coherence（MR-30） |
| 8 | 共有の判定を UHD の `give` が使う | D9a の試験（`control.rs` の 1a〜M6 ほか）がそのまま通る．UHD の振る舞いは変わらない |
| 9 | VH-8 の differential | 変わらない，または位相を比べるなら両方が同じ |

## 4. 計画の形

系列ごとに：テストの名前（`mr_34_…`，共有の判定は `rm_…` か `timeline` の unit test），置き場，何を assert するか，どう観測するか（`segments` の `coherence`，受信の sample の位相，`SimRng` の引き直しの数），どの間違った実装なら落ちるか．壊れる既存の試験の一覧と扱い．spec の変える規則の一覧．mutation の行の候補．最後に疑わしい所（事実の誤り，抜け，共有の判定を置く場所の案と退けた案）．

## 5. 報告

`ma1-test-plan.md` に計画．最初の行は `STATUS: plan`．最後のメッセージは計画の要約．Write が tree の外を拒んだら，計画の全文を最後のメッセージにする（orchestrator が保存する）．

---

# 段階 MA1：MockRadio の LO の位相を D9a の規則にそろえる（実装の指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．risky な段階で，試験計画は Sol の計画レビュー 6 回で PASS した．

**作業場所は git worktree `/home/komatsu/local_work/Ez-SDRv4-ma1`（branch `wip/ma1`，base `a9b0a26`）だけ**．main の tree（/home/komatsu/local_work/Ez-SDRv4）は別の agent（段階 3e）が編集中なので，読むことも書くこともしない（必要なら worktree の同じ path を読む）．worktree の `.cargo/config.toml` は target を `~/.cache/cargo-target/Ez-SDRv4-ma1` に向けてある（ほかの build と混ぜないため）．すべての cargo は worktree の root で．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（周りの書き方に手で合わせる．長い行はそのまま）．ローカルの `mutate.py` 禁止．mutant を worktree に当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- UHD Module の振る舞いを変えない（`reach` の覆う判定を共有の関数の呼び出しに置き換えるだけ．D9a の試験がそのまま通る）．UHD の新しい試験は足さない．送信・profile の値・Kernel は変えない．計画の冒頭の UHD の欠陥（Deferred の値の消失）は直さない（orchestrator が別の段で直す）．
- 仕組みを足しすぎない：Mock の hook は受け取りと適用の 2 か所だけ（計画 v6 の D2）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．負荷をかけない．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．作ったファイルは mode 644．
- Sol は起こさない．docker は使わない（docker の clippy は orchestrator が gate で流す）．
- **起きたことを書く．していないことは書かない．**

## 1. 読むもの

- 試験計画 `~/.cache/ezsdr-fixes/2026-10-26-stage3/ma1-test-plan.md`（v6．これが仕様．D1〜D3 の決め，系列と試験，§2 の壊れる既存の試験，§3 の spec の文，mutation の表 M1〜M14・M16，実装の順）．
- Sol の計画レビュー 6 回（`sol-ma1-plan{,2,3,4,5,6}.log` の最後の `> plan` 以降．読み方：`awk '/^> plan/{last=NR} {a[NR]=$0} END{for(i=last;i<=NR;i++) print a[i]}' <log> | grep -vE "^\S*(→|✱|⚙|✗)"`）．計画と食い違えば計画 v6 が優先．
- `AGENTS.md` §2（DA-1 の形，`KNOWN_LIMITATIONS.md` の規則），§3，§9．

## 2. 作るもの

計画 v6 のとおり：`ezsdr_radio::timeline::covering`（unit test つき），UHD の `reach` をそれに置き換え，Mock の受け取り（`PendingUpdate` に始まった segment の stream と origin を記録）と適用（同じ segment なら timed tune，それ以外は untimed tune），試験（計画の一覧），既存の試験の期待の直し（計画 §2，`mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` ほか．理由なしで期待を書き換えない．計画に無い試験が落ちたら，直す前に原因を報告に書く），spec（`design/09` MR-34・MR-27・*Changes*，`design/07` の新しい規則，計画 §3．`design/18` は 3e が編集中なので触らない），`KNOWN_LIMITATIONS.md` の MA-1 の項目は 3e が書くので触らない（Mock 側の Ceiling で書き足したい事があれば報告に書く），mutation の行．計画を広げた所（新しい状態や失敗の道）は計画に足す（`ma1-test-plan.md` の末尾に「実装で足したこと」）．

## 3. 確かめること

- 純粋なテストは 1 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base a9b0a26`（名前を変えたテストは `--renames` のファイルを作業 dir に書いて渡す），`git diff --check`．
- mutation：計画の行と，変えたコードや killing test に触れる既存の行（`reach` の既存の行，MR-34 の既存の行を grep で拾う）を worktree の root から `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:4,sim03:4 <ID> ...` で流す（`<lists>` は行が入っている list．変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．

## 4. セルフレビュー（1 回）

```
cd /home/komatsu/local_work/Ez-SDRv4-ma1 && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-ma1.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-ma1-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．free が失敗したら同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 5. 報告

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-ma1.md` に書き，最後のメッセージも同じ要約にする．最初の行は `STATUS: done`（止めたら `STATUS: stopped` と理由）．作ったもの，計画から変えた所と理由，テスト名，変えた既存の試験と理由，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．
