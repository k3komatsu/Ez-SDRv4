# 1b-5b：実装の指示書（MockRadio の今の key のモードを消す）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cbb275a`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`opus-notes.md` など）は読むだけ．書き換えない**（1b-5a で指示書が上書きされた）．

## 1. 背景と分け方

段階 1b-5（`plan/phase8/33-channels-and-streams.md` §5 の 1b-5 行，D12）は今の key（方向ごとで channel 番号の無い `radio.rx.frequency_hz` など，port `rx`，`DeviceDescription` の builder）を消す．3 つに分けた：

- 1b-5a（済み，`cbb275a`）：server の既定の profile と Python を layout の key へ．
- **1b-5b（これ）**：MockRadio の今の key のモードを消し，Mock を使うテストを layout の key へ移す．
- 1b-5c：UHD Module の今の key のモード，`ezsdr_radio` の `DeviceDescription` と今の key の宣言，selector の `keys`，spec 07・18 の今の key の文を消す．

この段階は **risky ではない**（AGENTS.md §9）．テスト計画と計画レビューは省く．

## 2. やること

1. **MockRadio**（`crates/ezsdr-mock-radio/src`）：
   - `profile.rs` の `Mode::Current` と，`Profile` の今の key の腕（`ProfileKind::Ideal` の `DeviceDescription` の literal を含む）を消し，Mock はいつも layout の description で動く．`is_layout()` が常に真になるなら，その分岐（`lib.rs` に 17，`profile.rs` に 2）を畳んで消す（`is_layout()` そのものも要らなければ消す）．
   - selector の `keys`：UHD は 1b-5c まで `keys: "channel"` で layout を選ぶので，**Mock は `keys` を受け付けて何もしない**（`"channel"` か無しなら layout，他の値は今と同じく拒否．`ezsdr_radio::channel_keys` の判定は今のまま呼んでよい）．selector の許す key の一覧で `rx`・`tx` を常に許す．
   - `x3xx::from_selector`（今の key の selection）を Mock が呼ばなくなるなら，その呼び出しを消す（`ezsdr_radio` 側の関数は UHD が使うので**消さない**．`ezsdr_radio`，`ezsdr-radio-uhd` の src はこの段階で変えない）．
2. **テスト**：今の key か今の key の selector を使う Mock のテストを layout の key に移す．同じ規則のテストが layout にすでにあれば，移さずに消してよい．どちらにしたかを，テストごとに報告の表に書く（`消した：<規則>，同じ規則の layout のテスト <名前>`）．対象（数は目安）：
   - `crates/ezsdr-mock-radio/tests/mock_radio.rs`（約 38），`mock_channel.rs`（約 18），`timeline.rs`（1），`mock_layout.rs` の今の key の部分．
   - `crates/ezsdr-acceptance/tests/v58.rs`（約 9），`x3xx.rs`（約 4），`uhd.rs`（約 2），`v61.rs`（1），`reactive.rs`，`crates/ezsdr-acceptance/src/experiments.rs`・`rig.rs`（port `rx` と今の key）．acceptance の rig は UHD（FakeDevice）にも使われる：UHD はまだ今の key のモードを持つので，rig の binding には `keys: "channel"` を付けて両方を layout にする．
   - `crates/ezsdr-kernel/tests`，`crates/ezsdr-sink/tests` などの port `rx`：MockRadio を使わないもの（Kernel の自前の fake の port 名など）は**変えない**．Mock を使うものだけ移す．
   - テストの繰り返しは helper にする：layout の selector を作る式，effective の key を引く式，`rx<k>` の feed を持つ profile を作る式．acceptance と Mock のテストで同じものを 2 回書かない（`crates/ezsdr-acceptance/src` か各 crate の tests の共通の場所に 1 つ）．
3. **spec 09**（`design/09-mock-radio.md`）：今の key のモード，selector の `keys` の意味（「Mock は受け付けて何もしない，1b-5c で消える」），MR-2・MR-3・MR-5・MR-6・MR-7・MR-32・MR-34・MR-38 と §5 のテストの表を，Mock が layout だけで動くように直す．spec 09 は DA-3 の書き直し前なので今の書き方のまま，marker と長い行を増やさない．Changes の行 1 つ．spec 07・18 はこの段階では変えない（1b-5c）．
4. **mutation の行**：消したコードを `old` に持つ行は消す（ID と理由を報告に）．消した・名前を変えたテストを `filter` に持つ行は，同じ規則を見る残ったテストへ当て直す．新しい規則は無い（消すだけ）ので新しい行は原則として要らない．当て直した行は `new` に古い識別子が残っていないかも grep する．ファイル全体を書き直さない．

## 3. 変えてはならない挙動

- MockRadio の layout のモードの挙動（`mock_layout.rs` と `parity.rs` が見ているもの）．
- VH-8（`crates/ezsdr-acceptance/tests/parity.rs`）の比較．
- 1b-5a の server と Python（`python/`，`crates/ezsdr-server`）．
- `crates/ezsdr-radio/src`，`crates/ezsdr-radio-uhd/src` は変えない．Mock を消すために要るように見えたら，変えずに報告の「穴」に書く．

## 4. 大きさ

見積もりは約 1 200 行（大半はテストの移し替えと消去）．目安は本体とテストで約 1 500 行（owner）．始める前に，触るテストを数えて報告に書く．2 000 行を大きく超えそうなら，どこで切るか（たとえば Mock の crate と acceptance）を報告に書いてから続ける．

## 5. 守ること

- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add`・`git stash` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．背景の job を残さない．
- **指示書に無い仕組みを足さない**．
- tree のファイルは mode 644（remote mutation の copy が 600 を読めない）．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．「このテストでは差が出ない」を equivalent の根拠にする．照合が空（0 の sample，空の list）でも通るテスト．

## 6. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. Python 3.14 の suite（`cargo build -p ezsdr-server` の後，`PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`）．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` のファイルを作業 dir に置く）．
4. mutation：当て直した行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>`．全行の `old` が source にちょうど 1 回一致することも確かめる．

## 7. セルフレビュー

reviewer は **DeepSeek**．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．

## 8. 報告

`~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．OpenAI の利用上限に当たったら `STATUS: blocked: OpenAI usage limit` にして，そこまでの事実を書いて終える．中身：

1. 消した本体（`path` と行数）と畳んだ分岐．
2. テストごとの扱いの表（移した／消した＋同じ規則の layout のテスト）．
3. mutation の行：消した行，当て直した行，結果．
4. 確かめることの結果．
5. セルフレビューの指摘と扱い．
6. 本体とテストの行数．
7. 穴．

**起きたことを書く．していないことは書かない．**

---

# 1b-5b 修正の回 1（実装役：Luna）

あなたの 1b-5b の実装（未 commit の tree）を直します．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5b/`．Sol のレビュー `sol-1.log` の末尾（「判定：FAIL」から下）を読む．

**orchestrator のファイル（`*-brief.md`，`checklist.md` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない**（前の回で止められた．自分で起こした背景の job だけを片付ける）．

## 裁定：Sol の 3 件をすべて採る

1. **major：VH-8 の differential（`crates/ezsdr-mock-radio/tests/timeline.rs` の `rm_26_the_timeline_against_mockradio`）から channel の集合の変化が消えた**．方向全体の channel 数の操作を捨てたままにせず，layout の所属の操作に置き換える：
   - 生成する操作に，`radio.<d>.c<n>.stream` の変更を足す：stream から抜ける（`-1`），戻る，2 channel・2 stream の layout で別の stream へ動く（RM-34 の move），空の stream へ入る．
   - 期待側（`ezsdr_radio::timeline` の plan）は channel の集合と stream ごとの plan を追う．Mock の側は同じ操作を Action で出す．比べるものは今の differential と同じ（clock，cut，fault の適用，Stop，start）．
   - 1,000 seed の中で，抜ける・戻る・move・空の stream への join の各遷移が実際に生成されたことを数えて assert する（どれかが 0 なら失敗）．
   - spec 09 の「手書きのケースで代替」の文を直す．
   - 本体（`crates/ezsdr-mock-radio/src`，`crates/ezsdr-radio/src`）で Mock と timeline が食い違ったら，本体を直さずにテストを `#[ignore = "divergence: ..."]` にせず，**失敗のまま止めて報告**（`STATUS: blocked: divergence ...`，系列と seed を書く）．
2. **minor：`mr_18_a_disable_that_arrives_after_a_later_change_follows_it` の意味が逆になった**（`mock_radio.rs:2220-2241`）．今の reentry のテストは残し，それとは別に元の系列のテストを戻す：最初に `radio.tx.s0.sample_rate_hz` の cold 更新，次に早い時刻の `radio.tx.c0.stream = -1`（disable）が届き，disable は先の変更の後へ繰り下がり，新しい送信の clock は自分の origin で終わる（旧テストの終わりの条件）．元の名前を使い，`renames.txt` から元の名前の改名を外す（新しい reentry のテストは新しい名前のまま）．
3. **minor：P162 は消してはいけなかった**（`old` は今の source にも 1 回一致する）．P162 を戻し，常に layout になった今の意味に合わせて，`Some(spec)` を MR-41 の拒否へ置き換える mutant に当て直す．

## 確かめること

変えたテスト binary（`timeline`，`mock_radio`），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD --renames <renames.txt>`．differential は 3 回続けて流して揺れないこと．mutation：P162 と，`rm_26_the_timeline_against_mockradio` と直したテストを `filter` に持つ行すべてを `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>`．全行の `old` がちょうど 1 回一致．tree のファイルは mode 644．セルフレビューは流さない．

守ること：指示書 §5 と同じ．

## 報告

`fix1-report.md`（日本語）．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．指摘ごとの直し（`path:line`），differential の生成の数（遷移ごと），mutation の行ごとの結果．**起きたことを書く．していないことは書かない．**
