# 段階 TXP：uhd-tx を実時間の優先度に，窓の端の最小の send．いまの仕事は試験計画だけ

あなたは実装役（Claude Code の Sonnet サブエージェント）です．**この回の仕事は試験計画を書くことだけ**で，コードは書かない．repository /home/komatsu/local_work/Ez-SDRv4 は **読むだけ**（別の agent が tree を編集中：`crates/ezsdr-radio/src/timeline.rs` ほか．code は `git show HEAD:<path>` で HEAD `d0a0e64` を読む）．出力は作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/txprio-test-plan.md` だけ．`cargo fmt`・mutation の道具・commit・`git stash` を使わない．cargo も流さない．Sol は起こさない．

## 1. 背景と owner の決め

- 送信の 100/200 MS/s の失敗の調査（`~/.cache/ezsdr-fixes/2026-10-26-stage3/tx-throughput-report.md`，Opus．§6 の推奨）：Module と UHD の 1 回の send の手間は律速でない．X300 の送信 buffer は 65 536 sample（200 MS/s で 0.33 ms）で，普通の優先度の uhd-tx は preempt されて buffer が尽きる（INFERRED）．低い rate では窓の端で約 3 sample の send が何万回も出る（shakeout-4，note 40 の D3 以後）．
- 実機 2026-10-28（`plan/phase8/bench-results.md`）：UHD の `benchmark_rate` で送信 200 MS/s・60 s，普通の優先度 underrun 8，`--priority high` 0，`--tx_spb 1000` 8．記録では #1 の送信は 50 MS/s（100 MS/s は約 57 s で underflow），#2 は 10 MS/s で止まった（原因は未確認）．
- owner の決め（2026-10-28）：実機で優先度が効くと確かめてから，次の 2 つを 1 つの段階として入れる．(1) uhd-tx を実時間の優先度に：新しい `Device` の method が UHD の `uhd_set_thread_priority(0.5, true)` を呼ぶ（FakeDevice は何もしない），拒まれたら記録して致命にしない．bench の container は既に `--cap-add=SYS_NICE --ulimit rtprio=99`．(2) 窓の端の最小の send の大きさ：実時間の優先度のもとで低い rate の小さな send が空回りにならないように．risky．

## 2. 範囲

- (1)（2）を調査の §6 の形で．spec 18 に 1 文と試験と mutation 行．送信の受け持ち（`crates/ezsdr-radio-uhd/src/provider/tx.rs`），`src/device.rs`（trait と FakeDevice），`src/uhd.rs`（FFI）．優先度の拒否の記録の形（既存の stat／timing／event のどれか）を計画で決める．
- 最小の send の規則：窓（UR-23 の in-flight の上限）の厳密さを保つこと，Stop と preempt の締め切り，held の最後の sample（D9 F3′ の扱い），repeat の wrap，burst の終わりの短い send（最小より小さい最後の部分は送る），時刻つきの命令との順．note 40（preemption bound）の D3 と UR-21〜UR-23 を読む．
- 範囲の外：chunk を packet の倍数に・repeat の wrap をまたぐ send（調査の §6 の「その後」），R4・R2・F7（D9b），受信の優先度（受信は保てている）．#2 の 10 MS/s の原因調査は範囲の外だが，計画の疑わしい所に書く．

## 3. 計画の形

risky な段階（AGENTS.md §9 step 2）：操作の系列ごとに，テスト，何を assert するか，どう観測するか（FakeDevice の send の大きさの列，時刻，in-flight の上限，Stop の締め切り），それを落とす間違った実装．UHD／OS の振る舞い（`uhd_set_thread_priority` の C API と失敗の返り値，SCHED_FIFO と rtprio の制限，container の外での失敗）を VERIFIED／INFERRED で（UHD 4.10 の source を読む）．壊れる既存の試験，spec の変える規則（DA-1 の文案），mutation の行の候補，疑わしい所．

## 4. 報告

`txprio-test-plan.md` に計画．最初の行は `STATUS: plan`．最後のメッセージは計画の要約．

---

# 段階 TXP：uhd-tx を実時間の優先度に，窓の端の最小の send（実装の指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．risky な段階で，試験計画は Sol の計画レビュー 3 回を経た（3 回目に残った 2 つの変更は下の §1 で先に計画に入れる）．

**作業場所は git worktree `/home/komatsu/local_work/Ez-SDRv4-txp`（branch `wip/txp`，base d0a0e64）だけ**．main の tree（別の agent が段階 43a を編集中）は読むことも書くこともしない．worktree の `.cargo/config.toml` は target を `~/.cache/cargo-target/Ez-SDRv4-txp` に向けてある．docker の `--features uhd` の build は volume `ezsdr-v4-txp-target` を使う（`ezsdr-v4-cargo-target` は使わない）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止．ローカルの `mutate.py` 禁止．mutant を worktree に当てない．commit・push・`git add`・`git stash` 禁止．version を上げない．
- 範囲は計画のとおり（`provider/tx.rs`，`src/device.rs`，`src/uhd.rs`，測定の包み役 `tests/common/`，spec 18 UR-2・UR-3・UR-23，*Changes*）．chunk を変えない，repeat の wrap をまたぐ send を作らない，受信の優先度は変えない，R4・R2・F7 は変えない．**計画に無い仕組みを足さない**．
- 試験の判定がテストの thread の時機に依らないこと（設計メモ 37）．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．負荷をかけない．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．worktree のファイルは mode 644（remote_mutate のため．必要なら `chmod -R go+rX`）．
- Sol は起こさない．実機は流さない．**起きたことを書く．していないことは書かない．**

## 1. 読むもの

- 試験計画 `~/.cache/ezsdr-fixes/2026-10-26-stage3/txprio-test-plan.md`（v3）．**最初に** Sol の 3 回目（`sol-txprio-plan3.log` の最後の `> plan` 以降）の 2 つの変更を計画に入れる：(1) M8b は `TxSpy` が 5 つ目の該当する idle の `tx_async` で owner の thread の上で snapshot を取り，channel の handshake で owner を止め，test が Shutdown を積んで放す（handshake の受け取りは有限時間で失敗させるためだけ）；(2) B0 の wrap で終わる block は `min(chunk, 31_000 − 直前までの offset)` で確かめる．
- Sol の計画レビュー 3 回（`sol-txprio-plan{,2,3}.log`），調査 `tx-throughput-report.md`，指示書の前の段 `txprio-brief.md`．読み方：`awk '/^> plan/{last=NR} {a[NR]=$0} END{for(i=last;i<=NR;i++) print a[i]}' <log> | grep -vE "^\S*(→|✱|⚙|✗)"`．
- `AGENTS.md` §2（DA-1），§9．

## 2. 作るもの

計画のとおり：`Device::raise_thread_priority`（default なし，FakeDevice は記録だけ，UhdDevice は `uhd_set_thread_priority(0.5, true)`），`Tx::run` の最初の文で 1 回，拒否は `rejected` に 1 行で致命にしない（`device_failed` と `note` の latch を通さない），成功は `stats.tx_realtime`，測定の包み役の転送，1 つの大きさ／準備の判定を 2 つの送る入口に（除外は `device_sob` だけ），試験（計画の一覧と `TxSpy`），spec，mutation の行．X8〜X10 は延期（feature つきの image が無い）と記録．実機の受け入れ B0〜B4 は `plan/phase8/bench.md` に手順として（送信の前の att の確認つき）．計画を広げた所は計画の末尾に「実装で足したこと」．

## 3. 確かめること

- 純粋なテストは 1 回．時刻に依るもの（計画の §5 の一覧）は 20 回．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．
- docker：`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-txp-target:/cargo-target -w /work -e CARGO_TARGET_DIR=/cargo-target ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` と同じ形で `cargo test -p ezsdr-radio-uhd --features uhd --lib`（F1〜F3）．
- mutation：計画の行と，変えたコードや killing test に触れる既存の行（tx.rs の窓と送る道の既存の行を grep で）を worktree の root から `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ID> ...`．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．mutation の ID は P1100 から始める（main で別の段が P1045 以後を使うため）．

## 4. セルフレビュー（1 回）

```
cd /home/komatsu/local_work/Ez-SDRv4-txp && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-txp.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-txp-1.log 2>&1
```

前景で（timeout 2,400 s 以上）．**終わるまで待ってから報告**．free が失敗したら `-m "opencode-go/deepseek-v4.1-flash#max"`．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 5. 報告

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-txp.md` と最後のメッセージ．最初の行は `STATUS: done`（止めたら `STATUS: stopped`）．
