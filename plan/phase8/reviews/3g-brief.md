# 段階 3g：D9 の probe と音の判定を直す（実機の再実行のため．指示書）

あなたは実装役（Claude Code の Sonnet サブエージェント）です．**作業場所は git worktree `/home/komatsu/local_work/Ez-SDRv4-probes`（branch `wip/probes`，base afbb125）だけ**．main の tree（/home/komatsu/local_work/Ez-SDRv4）は実機に使うので読むことも書くこともしない．worktree の `.cargo/config.toml` は target を `~/.cache/cargo-target/Ez-SDRv4-probes` に向けてある．すべての cargo は worktree の root で．別の worktree `Ez-SDRv4-d9a2` には触らない．

小さな段階で risky ではない（試験の道具だけ．`src/` は変えない）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止．ローカルの `mutate.py` 禁止．mutant を worktree に当てない．commit・push・`git add`・`git stash` 禁止．version を上げない．
- `src/` は変えない．仕組みを足さない．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．テストの一時ファイルは /tmp．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．触ったファイルは mode 644．
- Sol は起こさない．実機は流さない．**起きたことを書く．していないことは書かない．**
- 送信する probe は今どおり 0 dB・振幅 ≤ 0.5．

## 1. 仕様

2026-10-28 の実機の結果の分析 `~/.cache/ezsdr-fixes/2026-10-26-stage3/probe-analysis-1028.md`（Opus）の §1.5（音の判定の直し）と §5（rerun の最小の直し）がそのまま仕様．file:line はそこにある．要点：
- `crates/ezsdr-radio-uhd/tests/common/tone.rs`：雑音の床を coherence でなく電力で（平均電力 > 4×床，coherence > 0.5 は残す）．burst の最初の窓から既に移っている場合は欠けでなく「それ以前に移った」と扱う（:75 あたり）．`tests/fake.rs` に LSB の雑音の回帰 case（今の規則で落ち，直しで通る）．mutation P994・P998 の当て直し．
- P4（`hardware.rs:1135`，`:1153` あたり）：最初の tone を短く（`tone[..30_000]`），stop を device の時刻 e − 5 ms で出す．
- P5（`:1217`，`:1222` あたり）：priming の burst を 400 ms 先に（`150_000` → `400_000`），印字する時刻を DUC が数える時刻に．owner の決め 5（F7 の直し：Run の始めに時刻なしの空の EOB）を直接確かめる 3 つ目の case．
- P6・P7（`:1290`，`:1354` あたり）：時刻なしの空の EOB を送る helper を呼んで F7 を消す．P6 の starvation の閾値（`:1342`）を burst 自身の信号の大きさに対する相対値に．
- MA-1：変えない（結果はそのまま使う）．

分析の file:line は afbb125 の行．分析と code が食い違えば code を読んで正しい直しを選び，理由を報告に書く．`plan/phase8/bench.md` の (c) の見る点を新しい出力に合わせる．

## 2. 確かめること

- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`．
- `--features uhd` の build：`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-probes-target:/cargo-target -w /work -e CARGO_TARGET_DIR=/cargo-target ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`（**main の tree の volume `ezsdr-v4-cargo-target` は使わない**．名前の別の volume を使う）．
- 時刻に依る fake の試験（音の判定の rehearsal があれば）は 20 回．
- mutation：触った行と新しい行を worktree の root から `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ID> ...`（触ったファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．

## 3. 報告

`~/.cache/ezsdr-fixes/2026-10-26-stage3/report-probes.md` と最後のメッセージ．最初の行 `STATUS: done`（止めたら `STATUS: stopped`）．作ったもの，分析から変えた所と理由，テスト名，mutation の行と結果，確かめたことの結果，明日の再実行の手順（bench.md (c) の更新）．
