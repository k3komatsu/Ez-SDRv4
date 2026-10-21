# 1b-5c-1：実装の指示書（UHD Module の今の key のモードを消す）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `16fb216`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5c1/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景と分け方

段階 1b-5（`plan/phase8/33-channels-and-streams.md` §5 の 1b-5 行，D12）は今の key（方向ごとで channel 番号の無い `radio.rx.frequency_hz` など，port `rx`，`DeviceDescription` の builder）を消す．

- 1b-5a（済み）：server と Python を layout の key へ．
- 1b-5b（済み，`16fb216`）：MockRadio の今の key のモードを消した．Mock は selector の `keys` を，無いか `"channel"` なら受け付けて何もしない．
- **1b-5c-1（これ）**：UHD Module の今の key のモードを消し，UHD のテストを layout の key へ移す．spec 18．
- 1b-5c-2：`ezsdr_radio` の `DeviceDescription`・今の key の宣言・`channel_keys`，selector の `keys`（Mock・UHD・テスト・Python のテスト），spec 07 と Vision の今の key の文を消す．

この段階は **risky ではない**（AGENTS.md §9）．テスト計画と計画レビューは省く．

## 2. やること

1. **UHD Module**（`crates/ezsdr-radio-uhd/src`）：
   - `provider/mode.rs` の `Mode::Current` を消し，UHD はいつも layout の description で動く．`Mode` の enum が 1 つの腕だけになるなら，enum を消して description を直接使う（`Mode` の method で要るものは残す．呼び出し側の形を大きく変えない）．`is_layout()` の分岐（`control.rs`，`core.rs`，`mode.rs`）を畳んで消す．`current_key_direction`（`mode.rs`）と，その呼び出し（`control.rs`）を消す．
   - `provider/mod.rs` の `channel_keys` の読み：Mock と同じく，`keys` が無いか `"channel"` なら layout，他の値は拒否（1b-5c-2 で `keys` そのものを消す）．`profile.rs` の今の key の builder（`description`，テスト用の `ubx_description` など）で使われなくなったものを消す．
   - **1b-4c の O5**（`plan/phase8/reviews/1b4c-opus.md` §O5）：front end の照合が layout と今の key のモードの 2 つの腕に分かれている．今の key の腕を消して 1 つにする．
   - UR-26 の「今の key の direction antenna の拒否」など，今の key のモードだけの規則のコードは消す．
   - `crates/ezsdr-radio/src`（`DeviceDescription`，`x3xx::description`，今の key の宣言，`channel_keys`）は**この段階では消さない**（1b-5c-2）．UHD が呼ばなくなるだけ．`crates/ezsdr-mock-radio/src`，`crates/ezsdr-server/src`，`python/` は変えない．
2. **テスト**：今の key か今の key の selector を使う UHD のテストを layout の key へ移す．同じ規則のテストが layout にすでにあれば移さずに消してよい（テストごとに報告の表へ：移した／消した＋同じ規則を見る残ったテスト）．対象（数は目安）：`tests/fake.rs`（約 76），`src/provider/control.rs`（約 24），`tx.rs`（約 8），`rx.rs`，`control/differential.rs`（1），`test_support.rs`，`tests/hardware.rs`（約 10），`tests/uhd_api.rs`．
   - テストの繰り返しは helper にする：layout の selector，FakeDevice の rig を組む式，effective の key を引く式．`tests/common/mod.rs` と `src/provider/test_support.rs` に既にあるものを使い，同じものを 2 回書かない．
   - `tests/hardware.rs` は実機でしか流せない．docker の uhd の build と clippy が通ることを確かめ，どのテストを変えたかを報告に書く（orchestrator が X300 で流す）．
   - 照合が空（0 の sample，空の list）でも通るテストを作らない．
3. **spec 18**（`design/18-uhd-radio.md`）：今の key のモードを述べる文（UR-5 の `keys`，UR-9，UR-12，UR-15，UR-17，UR-26 の今の key の腕，`<id>/rx`・`<id>/tx` の node など）を，UHD が layout だけで動くように直す．`keys` は「無いか `"channel"` なら受け付けて何もしない」と書き，1b-5c-2 で消えることは Changes の行に書く．spec 18 は DA-3 で書き直した spec なので，AGENTS.md §2 の「Format of accepted specs」を守る（規則は normative な短い文と `*Test:*` 行，理由と経緯は Changes へ）．消えた規則の ID は 1 行の stub を残す（番号を振り直さない）．spec 07 と Vision はこの段階では変えない．
4. **mutation の行**：消したコードを `old` に持つ行は消す．**消す前に，その行の `old` が本当に今の source に無いことを確かめる**（1b-5b で，まだある行を消す誤りがあった）．消した・名前を変えたテストを `filter` に持つ行は，同じ規則を見る残ったテストへ当て直す．`equivalent` を持つ行の前提が変われば見直す．ファイル全体を書き直さない．

## 3. 変えてはならない挙動

- UHD Module の layout のモードの挙動（FakeDevice の layout のテスト，VH-8 の `parity.rs`，`rm_26_the_timeline_against_uhd_rx` などの differential）．
- 1b-4e-2 の moves と，FakeDevice の held front end の open の検査（`take_abort`）．
- `crates/ezsdr-radio/src`，`crates/ezsdr-mock-radio/src`，`crates/ezsdr-server/src`，`python/` は変えない．

## 4. 大きさ

見積もりは約 1 500 行（大半はテストの移し替えと消去，本体は減る）．始める前に触るテストを数えて報告に書く．2 000 行を大きく超えそうなら，どこで切るかを報告に書いてから続ける．

## 5. 守ること

- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add`・`git stash` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．
- **指示書に無い仕組みを足さない**．
- tree のファイルは mode 644．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．「このテストでは差が出ない」を equivalent の根拠にする．照合が空でも通るテスト．differential から操作を外して代わりを足さない．

## 6. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` のファイルを作業 dir に置く）．
4. mutation：当て直した行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>`．全行の `old` が source にちょうど 1 回一致することも確かめる．

## 7. セルフレビュー

reviewer は **DeepSeek**．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5c1/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5c1/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．

## 8. 報告

`~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5c1/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．OpenAI の利用上限で止まるときは最初の行を `STATUS: blocked: OpenAI limit` にして，そこまでの事実を書いて終える．中身：

1. 消した本体（`path` と行数）と畳んだ分岐．
2. テストごとの扱いの表（移した／消した＋同じ規則を見る残ったテスト）．
3. mutation の行：消した行（`old` が source に無いことの確認つき），当て直した行，結果．
4. 確かめることの結果．
5. セルフレビューの指摘と扱い．
6. 本体とテストの行数．
7. 穴．

**起きたことを書く．していないことは書かない．**
