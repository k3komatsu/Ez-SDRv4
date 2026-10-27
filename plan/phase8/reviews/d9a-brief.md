# 段階 D9a：指示書

試験計画を書く回の指示書（`d9a-brief.md`）：


あなたは実装役（Claude Code の Sonnet サブエージェント）です．**この回の仕事は試験計画を書くことだけ**で，コードは書かない．repository /home/komatsu/local_work/Ez-SDRv4 は **読むだけ**（別の agent が今 tree の `tests/` を編集中．production のコードは `git show HEAD:<path>` で HEAD `7c0de6a` のものを読む）．出力は作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/d9a-test-plan.md` だけ．`cargo fmt`・`rustfmt`・mutation の道具・commit・`git stash` を使わない．cargo も流さない（読むだけで足りる）．Sol は起こさない．

## 1. 読むもの

- `~/.cache/ezsdr-fixes/2026-10-26-stage3/note42-draft.md`（owner の決め．これが仕様）．
- `~/.cache/ezsdr-fixes/2026-10-26-stage3/d9-production-investigation.md`（Opus の調査．§1 の事実，§3 の経路，§5 の試験の系列）と `sol-d9-verify.md` の最後の節（Sol の裏取り：R1 の例外を外す，R4 は緊急の EOB に付けない，DUC の溢れは害が無いとしない）．
- `plan/phase8/39-dsp-timed-commands.md`，`plan/phase8/33-channels-and-streams.md` の D9，`plan/phase8/38-timed-release-order.md`．
- `design/18-uhd-radio.md` の UR-22，UR-24，UR-25，UR-33，UR-36．
- HEAD のコード：`crates/ezsdr-radio-uhd/src/provider/control.rs`（`channel_live` 516，`targets` 527，`release` 601，`give` 700，configure の印 955-1037），`provider/rx.rs`，`provider/tx.rs`（434，442，579），`provider/core.rs`（configure 1079-1087，1237-1289），`src/device.rs`（FakeDevice の apply と timed の値 1061-1095 あたり），`src/uhd.rs`（864-887）．
- 書き方の例：`plan/phase8/reviews/1b4e2-test-plan.md`．

## 2. この段階（D9a）の範囲

- **R1（受信の周波数だけ）**：timed の受信の周波数は，その instant `e` に，その channel を含む拒否されていない segment が走っている（`e` で sample がその DDC を通る：segment の origin ≤ `e`，`e` より前の cut が無い）ときだけ装置に timed で渡す．それ以外は装置に timed で渡さない．
  - 記録・行・event・投影は今と同じ（VH-8 の differential が変わらない）．
  - 渡さなかった値は，その channel が次に流れる前に untimed で当たらなければならない．宛先の segment の configure がまだなら configure が当てる．**configure がもう済んでいる**（uhd-control が先に configure した，UR-36 の印）なら，configure は走り直さないので，別に当てる道が要る．案：その chain がその時点から origin まで流れない（どの segment もその channel を走らせていない）なら，release の時に untimed で当てる．流れている（cut の前の segment がまだ走っている）間は当てない．どの案が正しく最も道が少ないかを計画で決め，理由を書く．
  - 受信の利得は変えない（radio にだけ届く）．送信は変えない（R4 は P6 の後の D9b）．
- **R5（FakeDevice）**：chain ごとに DSP の FIFO を model する．
  - 受信：timed の周波数は，sample が流れている間に，時刻が `e` 以上の sample で 1 つずつ取り出され，その sample から値が変わる．流れていなければ取り出されない．満杯（32）で次の timed の書き込みは error（実機の詰まりの代わり．fake では装置の失敗として返す）．
  - 送信：時刻つきの send でだけ取り出す．時刻なしの send が来たら，その burst の終わりまで取り出さない．
  - untimed の書き込みはすぐ当たるが，待っている timed の値が後の sample でそれを上書きする．
  - DUC は溢れても error にしない（満杯は時刻を捨て，値は最新で上書き．INFERRED）．
  - 装置を開くと空．
  - 今の FakeDevice は timed の値を instant ですべて当てる．この変更で壊れる既存のテストを grep で挙げ，それぞれ「テストが実機より緩い前提に頼っていた（直す）」か「production の振る舞いが変わった（R1 の結果）」かを分けて書く．
- **S2 の Ceiling** を R5 の上で試験で固める．
- **P4〜P7 の probe**（`tests/hardware.rs`，`hw_d9_p4_…` など．raw の `Device` を直接．値を出力するだけで assert しない．どれも timed の DSP の entry は 8 個以下．送信を含むので bench で att を確かめる）．P6 は 2 ms の chunk をすべて時刻つきで送る．中身は note42-draft §4．

範囲の外：R4（送信の時刻），R2，F7 の対処（どれも bench の後の D9b），MockRadio と語彙（spec 07，09）の変更，profile の値．

## 3. 系列（計画で各々にテストを当てる．足りなければ足す）

| # | 系列 | 期待 | 頼る事実 |
|---|---|---|---|
| 1 | 受信を Stop した channel に timed の周波数を 40 個 | 装置への timed の DSP の書き込みは 0．fake の DDC の FIFO は 0 のまま詰まらない．行は今と同じ．restart の最初の sample は最後の値 | DDC は sample でしか取り出さない（VERIFIED） |
| 2 | stream に無い channel に，遠い先の segment に宛てた timed の周波数を 40 個（configure はまだ） | 同上．segment の configure が最後の値を当て，最初の sample からその値 | 同上 |
| 3 | 2 と同じだが，宛先の segment は uhd-control がもう configure 済み | 値は origin より前に当たり，最初の sample から最後の値．FIFO は 0 | configure は走り直さない（UR-36） |
| 4 | 走っている segment の中の `e` に timed の周波数 | timed で渡り，`e` の sample から変わる | 今と同じ |
| 5 | segment の origin のすぐ後（release window の中）の `e` | timed で渡り（segment は `e` を覆う），`e` の sample で変わる．configure の untimed の書き込みとの順で値が崩れない | 待っている timed は後の untimed を上書きする（VERIFIED） |
| 6 | 4 の値を渡した後に，`e` より前の cut が予約される（R2 の前の Ceiling） | fake の FIFO に 1 つ残り，次の segment の最初の sample で当たる（今の Ceiling．R2 は P4 の後）．今の振る舞いとして固めるか，計画で扱いを書く | INFERRED（P4 で決める） |
| 7 | 走っている segment の中で，stream の別の channel の値と混ざる（2 channel） | channel ごとに正しい | — |
| 8 | 送信の burst の間に timed の周波数（今のまま渡す） | 次の burst の先頭の sample で当たる | P3（VERIFIED，bench） |
| 9 | S2：8 の後，channel が送信の stream を離れ，離れている間に別の値，戻る | 戻った後の最初の burst は残った値（Ceiling）．テストはその Ceiling を固め，spec と `KNOWN_LIMITATIONS.md` の項目の根拠になる | INFERRED（R5 の model） |
| 10 | 送信の burst の途中の chunk に `e`（今は時刻なし） | LO は `e`，DSP は次の burst まで古い（F3'．R4 は D9b）．fake でこの Ceiling を固める | VERIFIED（source） |
| 11 | 装置を開き直す | FIFO は空 | VERIFIED（source） |
| 12 | VH-8 の differential（`rm_26_the_timeline_against_uhd_control`） | 変わらない（R1 は見える振る舞いを変えない） | — |

ほかに考えること：受信の rate の変更（cold change）を挟む場合，`Provider::stop` で渡しかけの値，loss の後．計画に系列を足してよい（足した理由を書く）．

## 4. 計画の形

系列ごとに：テストの名前（`ur_…` の形），どこに置くか（unit か `tests/fake.rs` か），何を assert するか，その assert がどう振る舞いを観測するか（fake の FIFO の数，装置の呼び出しの列，sample の値など），どの間違った実装ならそのテストが落ちるか．時刻とスレッドに依るテストは ManualTimeAuthority / 手動の時計で決定的にする（`tests/README.md`）．壊れる既存のテストの一覧と扱い．spec 18 の変える規則の一覧（文は書かなくてよい，何を変えるか）．mutation の行の候補（場所と変え方）．最後に，この指示書の範囲や系列で疑わしい所（事実の誤り，抜け）を挙げる．

## 5. 報告

`d9a-test-plan.md` に計画．最初の行は `STATUS: plan`．最後のメッセージは計画の要約（系列ごと 1 行と疑わしい所）．

## 実装（`d9a-impl-brief.md`）


あなたは D9a の試験計画を書いた実装役です．今度は実装する．HEAD は `660213b`（3c-3a と設計メモ 42 が入った）．repository /home/komatsu/local_work/Ez-SDRv4 の main の tree を触るのはあなただけ（ほかの agent は書かない）．

## 0. 守ること（3c-3a と同じ）

- `cargo fmt`・`rustfmt` 禁止（周りの書き方に手で合わせる）．ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．負荷をかけない．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．tree に作ったファイルは mode 644．
- Sol は起こさない．実機は流さない（P4〜P7 の probe は書くだけ，`--features uhd` の docker clippy まで）．
- 並行や失敗の扱いは，仕組みを足す前に道を減らす．
- **起きたことを書く．していないことは書かない．**

## 1. 仕様

- 設計メモ `plan/phase8/42-d9-production-fix.md`（owner の決め）．
- あなたの試験計画 `plan/phase8/reviews/d9a-test-plan.md`（改訂 3 まで，Sol の 4 回目で PASS）と Sol の計画レビュー `plan/phase8/reviews/d9a-sol-plan.md`．計画どおりに実装し，実装で足した状態や失敗の道は計画に足す（計画のファイルは作業 dir の `d9a-test-plan.md` を直し，最後に tree の写しも同じにする）．
- 範囲：R1（受信の周波数だけ），R5（FakeDevice の DDC/DUC の FIFO），S2 と F3' の Ceiling の試験，P4〜P7 の probe（`tests/hardware.rs`，probe 専用の `UhdDevice::tune_auto`），spec 18（§2 の事実，UR-24，UR-25，UR-33 ほか計画が挙げる規則，Changes の行），設計 33 の D9 の規則 1〜4（設計メモ 42 のとおり），設計メモ 39 と `research/x300-dsp-timed-commands.md` の誤り（設計メモ 42 §5），`KNOWN_LIMITATIONS.md` は orchestrator が書くので触らない．
- 範囲の外：R4，R2，F7，MockRadio と語彙，profile の値．

## 2. 確かめること

- 純粋なテストは 1 回．時刻・スレッドに依るテスト（rig，fake の rehearsal）は 20 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，docker の clippy（`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`），`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames`），`git diff --check`．
- VH-8 の differential（`rm_26_the_timeline_against_uhd_control`）が変わらず通ること．
- mutation：計画 §8 の行を足し（ID は既存の最大の次から），新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ID> ...` で流す（repo の root から．前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．「BASELINE FAILED」はその行だけ流し直す．

## 3. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-d9a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-d9a-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．free が失敗したら同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 4. 報告

最後のメッセージを報告にする（Write が拒まれたら orchestrator が保存する）．最初の行は `STATUS: done`（止めたら `STATUS: stopped` と理由）．作ったもの，計画から変えた所と理由，テスト名，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．

## セルフレビューの checklist

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked，HEAD `660213b`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：`plan/phase8/42-d9-production-fix.md`（仕様），`plan/phase8/reviews/d9a-test-plan.md`（試験計画），作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/d9a-impl-brief.md`．

見ること（4つ）：
1. **計画の系列とテスト**：計画の各系列（1〜12，M1〜M6，R5a〜m，12b〜d）にテストがあり，計画どおりに観測し（FIFO の数，装置の呼び出しの列，`freq_at` の sample の値），計画の mutation の行がそのテストで kill されるか（読んで判断）．時刻とスレッドに依るテストが手動の時計で決定的か．
2. **範囲**：R4・R2・F7，MockRadio・語彙・profile の変更，`KNOWN_LIMITATIONS.md` への変更は指摘．足りない所．
3. **「すべて」の主張**：受信の timed の周波数が装置に timed で渡るのは「e を走行中の segment が覆う」ときだけか（`give` の全部の道を grep）．渡さなかった値が必ず当たるか（configure 前，configure 済み，最初の segment）．configure 済みの印を投影の snapshot と同じ lock で立てるか．利得と送信は今のままか．FakeDevice の DDC は装置の時刻で実際に流れた区間だけ取り出すか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：spec 18・設計 33 の D9・設計メモ 39・research の直しが設計メモ 42 のとおりか，DA-1 の形か，挙げたテストが存在するか．

orchestrator の gate は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．
