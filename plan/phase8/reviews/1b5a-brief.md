# 1b-5a：実装の指示書（server と Python を layout の key へ）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `698802e`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/`．

## 1. 背景と分け方

Phase 8 の段階 1b-5（設計 `plan/phase8/33-channels-and-streams.md` §5 の 1b-5 行，D11，D12）は，server・Python・fixture を layout の key（`radio.rx.c<n>.*`，`radio.rx.s<k>.*`）へ移し，今の key（`radio.rx.frequency_hz` など，方向ごとで channel 番号の無いもの），port `rx`，今の builder，selector の一時的な `keys` を消す．大きいので 3 つに分けた（orchestrator，2026-10-21）：

- **1b-5a（これ）**：server の既定の profile と Python の client を layout の key へ．Mock・UHD の今の key のモードは**まだ消さない**（1b-5b・1b-5c）．
- 1b-5b：MockRadio の今の key のモードを消し，Mock・acceptance・kernel のテストを移す．
- 1b-5c：UHD Module の今の key のモード，`DeviceDescription`，selector の `keys` を消し，UHD のテストを移す．

この段階は **risky ではない**（AGENTS.md §9：queue・deadline・寿命・FFI の所有・fake と実機の境界を変えない）．テスト計画と計画レビューは省いて実装に入る．

## 2. 決まっていること（owner と orchestrator）

D11（plan 33）と owner の決定（2026-10-21）：

1. **server の既定の profile**（`crates/ezsdr-server/src/catalogue.rs`）：radio の selector に `"keys": "channel"` を足し（1b-5c で消す），capture Sink の `feed` と placement の link の port を `rx0` にする．`sim.channel` の couplings の `tx_channel`・`rx_channel` は論理 channel 番号のまま．既定の layout で channel 0 が送受信とも stream 0 に入っていること（入っていなければ selector の既定値か `sim.channel` を合わせる．どちらにしたかを報告に書く）．
2. **Python の API**（`python/ezsdr/session.py`，spec 16 EA-16）：
   - `Radio.rx_channels[n]`・`Radio.tx_channels[n]`：`frequency`（`radio.<d>.c<n>.frequency_hz`），`gain`（`gain_db`），`antenna`（`antenna`），`stream`（`radio.<d>.c<n>.stream`，int．`-1` はどの stream にも入らない）．読むと effective の値，書くと `SetParameter` 1 つ（拒否なら `Rejected`）．
   - `Radio.rx_streams[k]`・`Radio.tx_streams[k]`：`sample_rate`（`radio.<d>.s<k>.sample_rate_hz`），`channels`（**読むだけ**：その stream に入っている論理 channel の番号の list を effective から作る．書き込みは無い．所属は `rx_channels[n].stream` で変える），`stop()`（`Target.resource(<radio>, "<d>/s<k>")`）．受信の stream は `start`・`next_at`・`capture`・`request`・`result`，送信の stream は `repeat`．
   - **`Radio.rx`・`Radio.tx` は `rx_streams[0]`・`tx_streams[0]` の別名**（owner 2026-10-21）．channel の属性（`frequency` など）は持たない．`Session.rx`・`Session.tx` も同じく `radio().rx_streams[0]`・`radio().tx_streams[0]`．`Session.rx_channels`・`tx_channels`・`rx_streams`・`tx_streams` は `radio()` のものへの近道．
   - **1 つの操作は 1 つの Action**：複数の key をまとめて変える近道は作らない．
   - `repeat(x)`：今は `radio.tx.channels` を波形の channel 数に合わせているが，layout では stream の channel の数は所属で決まる．波形の channel 数がその stream の `channels` の数と違えば，request を送る前に `Error`（理由に両方の数）．所属を勝手に変えない．
   - Target：受信の `start_rx` と `Stop` は `<radio>/rx/s<k>`，送信の `start_repeat` と `Stop` は `<radio>/tx/s<k>`（RM-29，MR-39）．
   - `next_at`：`status` の `sample_clocks` から，その stream の node（`<radio>/rx/s<k>`）の clock を選ぶ．
   - EA-17 の recorder：feed の port が `{ component: <radio>, port: "rx<k>" }` の binding を 1 つ選ぶ（0 個・複数は今と同じ `Error`）．
   - `Session.radios()` の「Radio Model の key（`radio.*`）を持つ fragment」はそのまま．
3. **例**：`python/examples/minimal.py`・`sweep.py`・`bench_loopback.py` を新しい API に．`sdr.rx.frequency = …` は `sdr.rx_channels[0].frequency = …` に，`sdr.rx.sample_rate` はそのまま（stream の属性）．
4. **Vision**：`design/vision/01-purpose-and-core-boundary.md`（§3 の 2 つの snippet，57〜70 行あたり）と `design/vision/10-implementation-path-and-testing.md:36` の snippet を新しい API に直し，`Ez-SDR_v4_ARCHITECTURE_VISION.md` の revision history に 1 行（AGENTS.md §2）．Vision の他の記述（§8 の `radio.rx.channels` の説明など）はこの段階では触らない（1b-5c）．
5. **spec 16**（`design/16-easy-api.md`）：EA-9 の例の JSON，EA-16 の表，EA-17，EA-19，§5 のテストの表と `*Checked by*` を新しい API と port `rx0` に．accepted spec の書き方（AGENTS.md §2「Format of accepted specs」）：spec 16 はまだ DA-3 の書き直し前なので，今の書き方のまま，marker と長い行を増やさない．Changes の表があれば 1 行．

## 3. 変えてはならない挙動

- Python の他の API（`aligned`，`after`，`sleep`，`wait_for`，`run`，`read`，`waveform`，`samples`，`Target`，Seconds の読み方，例外の種類）．
- `aligned` の中で `start`・`repeat`・`request` が `at` を共有すること（stream ごとの対象になっても同じ）．
- `capture` の HD-10 の連続（`request` を続けて出すと隙間なく続く）．
- server の protocol（EA-2〜EA-15）．既定の profile の中身は port と `keys` 以外変えない．
- MockRadio と UHD Module の本体（`crates/ezsdr-mock-radio/src`，`crates/ezsdr-radio-uhd/src`，`crates/ezsdr-radio/src`）は変えない．Mock が layout の key で足りないことが分かったら，直さずに報告の「穴」に書いて止める（`STATUS: blocked`）．

## 4. やること

1. 上の §2 を実装する．
2. テスト：`python/tests/test_easy_api.py`（今の key と port `rx` の箇所を移す．新しい API ごとに 1 本以上：channel の属性の読み書き，stream の `sample_rate`，`channels` が読むだけで所属に従うこと，`rx_channels[n].stream` で所属が変わること，`Radio.rx` が `rx_streams[0]` と同じ object（か同じ振る舞い）であること，`repeat` の channel 数の不一致の `Error` が request の前に出ること，stream 1 の `capture` が port `rx1` の recorder を使うこと〈profile に 2 つ目の recorder を足して〉，`next_at` が stream の clock を選ぶこと）．`crates/ezsdr-server/tests/protocol.rs` の今の key と port `rx` を使うテスト（約 12 本）を移す．
3. テストの繰り返しは helper にする（profile に recorder を足す手順，effective の読み）．
4. mutation の行：`plan/maintenance/tools/mutations.json` の `maintenance` に P528 から．server と Python の新しい規則ごとに 1 行以上（Python の行が今の道具で流せるかを `plan/maintenance/tools/mutate.py` の読みで確かめる．流せないなら行を作らず報告に書く）．`old` を変えた既存の行（`plan/*/tools/mutations.json` で `crates/ezsdr-server`・`python/` を file に持つ行）は当て直す．当て直した行は `new` に古い識別子が残っていないかも grep する．ファイル全体を書き直さない．

## 5. 守ること

- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる（Python も周りの書き方）．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．背景の job を残さない．
- **指示書に無い仕組みを足さない**．足りないと思ったら，足さずに報告の「穴」に書く．
- 大きさの目安は本体とテストで約 1 500 行（owner）．この段階の見積もりは約 500 行．大きく超えそうなら，どこで切るかを報告に書いてから続ける．
- Python は `PYTHONDONTWRITEBYTECODE=1`．build は tree の外．tree の新しいファイルは mode 644．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．「このテストでは差が出ない」を mutation の equivalent の根拠にする（equivalent は，どの系列からも届かないことを示せるときだけ）．

## 6. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. Python 3.14：`PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`（先に `cargo build -p ezsdr-server`）．Python 3.9 は handoff §5 の container のコマンド．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`．
4. mutation：新しい行と当て直した行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase6 sim02:3,sim03:3 <IDs>`．COMPILE ERROR も生き残りとして扱う．全行の `old` が source にちょうど 1 回一致することも確かめる．

## 7. セルフレビュー

reviewer は **DeepSeek**（owner 2026-10-21：OpenAI の 5 時間枠を節約する）．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/review-<n>.log 2>&1
```

`checklist.md` は orchestrator が置いた（読むだけ）．P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．gate（全 mutation）は orchestrator がするので指摘にしない．

## 8. 報告

`~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．OpenAI の利用上限に当たったら `STATUS: blocked: OpenAI usage limit` にして，そこまでの事実を書いて終える．中身：

1. §2 の各項をどう実装したか（`path:line`）．
2. テストの一覧（何を確かめるか，`path:line`）．
3. 確かめることの結果（mutation は行ごと）．
4. セルフレビューの指摘と扱い．
5. 本体とテストの行数．
6. 穴．

**起きたことを書く．していないことは書かない．**

---

# 1b-5a 修正の回 1（実装役：Luna）

あなたの 1b-5a の実装（未 commit の tree）を直します．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/`．orchestrator の `opus-notes.md` と Sol のレビュー `sol.log` の末尾（「判定：FAIL」から下）を読む．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`opus-notes.md`）は読むだけ．書き換えない**（前の回で `impl-brief.md` が上書きされた）．

## 裁定

- **blocked の扱い**：送信の channel は既定でどの stream にも入らない（RM-30）．指示書 §2.1 の「既定の profile で送信 c0 が stream 0」は orchestrator の誤りで，取り下げる．送信は `tx_channels[n].stream = k` で明示して有効にする（今の例とテストのとおり）．報告の「穴」からこの項を外す．

## 直すもの

1. **Sol 1（major）＝ Opus 1**：`_channel_count` を消す．channel と stream の一覧は，effective（`session.effective[radio]`）を 1 回読んで，方向ごとの `radio.<d>.c<n>.stream` の key から channel の番号を，`radio.<d>.s<k>.sample_rate_hz` の key から stream の番号を集めて作る．stream の数を channel の数から決めない．`_Stream.channels` も同じ effective から作る．selector を名指ししない（EA-18）．テスト：selector が空でも effective に RX c0・c1／s0・s1，TX c0／s0 がある Session で長さが 2・2・1・1 になり，`Radio.tx` が TX s0 になる（Sol の再現）．spec 16 の該当の文（`design/16-easy-api.md:86` あたり）も直す．P545・P547 を selector に依存しないテストへ当て直す．
2. **Opus 4**：`_effective` のエラー文から `keys='channel'` を消す（`EA-16: <radio> has no key <key>`）．
3. **Opus 2**：Vision（§3 の 2 つの snippet，§57，§66）と `python/examples/minimal.py`・`bench_loopback.py` は，別名のまま書く（`sdr.tx.repeat(x)`，`sdr.rx.capture(N)`，`sdr.rx.sample_rate = …`）．HEAD から足すのは `sdr.tx_channels[0].stream = 0` の 1 行と，channel の属性（`sdr.rx_channels[0].frequency` など）の書き換えだけにする．Vision の本文（`01-purpose-and-core-boundary.md` の「Assigning the transmit channel … `sdr.tx_streams[0].repeat(x)` …」）も別名で書く．
4. **Opus 3**：削りすぎた docstring を戻す：`Tx.repeat` の PPS の説明（`at` が whole second of a root whose epoch is `pps` なら PPS edge，UR-7，`next_pps`），`_Stream.stop` の RM-16・RM-21 の振る舞い（受信は `start()` まで止まったまま，送信は次の `repeat` で送る）．

## 確かめること

変えたテスト（`python/tests`，`crates/ezsdr-server/tests/protocol.rs`）と Python 3.14 の suite，`cargo test -p ezsdr-server`，`rules.py --base HEAD --renames <今の renames.txt>`．mutation は新しい行・当て直した行（P545，P547 ほか触った行）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase6 sim02:3,sim03:3 <IDs>`．全行の `old` がちょうど 1 回一致．セルフレビューは流さない．

守ること：指示書 §5 と同じ（`cargo fmt` 禁止，mutation は remote だけ，commit・push・`git add` 禁止，`timeout 300`，指示に無い仕組みを足さない）．

## 報告

`fix1-report.md`（日本語）．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．指摘ごとの直し（`path:line`），テスト，mutation の行ごとの結果，確かめることの結果．**起きたことを書く．していないことは書かない．**

---

# 1b-5a 修正の回 2（実装役：Luna）

あなたの 1b-5a の実装（未 commit の tree）を直します．作業 dir は `~/.cache/ezsdr-fixes/2026-10-21-phase8-1b5a/`．orchestrator のファイル（`*-brief.md`，`checklist.md`，`opus-notes.md`，`1b5a-opus.md`）は読むだけ．書き換えない．

## 何が起きたか（orchestrator の gate）

全 mutation で phase6 の **P02**（EA-17 の capture の転置なし）と **P03**（2-D の波形を interleave せずに送る）が生き残った．殺し役の `test_ea_17_two_channels_round_trip` は，1b-5a の書き換えの後，**受けた sample がすべて 0** のまま通っている．`python/tests/test_easy_api.py` の `rotation` は，`y` が 0 だと `phasor = 0` になり `assert_allclose(0, 0)` で一致してしまう．

orchestrator が tree の外の写しで `rotation` の先頭に `assert np.min(np.abs(y)) > 1e-6` を足して流したところ，3 本が silence で落ちた：

- `test_ea_17_two_channels_round_trip`（1b-5a で silence になった．HEAD では信号があり P02・P03 を殺していた）
- `test_ea_17_requests_made_ahead_capture_contiguous_samples`（**HEAD でも silence**）
- `test_v58_01_the_same_script_under_another_profile`（**HEAD でも silence**）

## 直すもの

1. `rotation` が silence を一致としない：`y` のどの sample も 0 でない（magnitude が十分ある）ことを先に確かめる．helper の docstring も合わせる．
2. 上の 3 本が実際に信号を受けて照合するように直す．なぜ 0 になっているか（cold の所属の変更が効く前の capture，送信の始まる前の区間，recorder の stream の取り違え など）を確かめてから直す．テストの意味（EA-17 の 2 channel の往復，HD-10 の連続，v58 #1 の同じ script が別の profile で動く）は変えない．`sleep` で待つなら，Run の時間（`sdr.sleep`）で，効く instant が分かるならその instant まで．
3. 他に `rotation` を使わず，信号の有無を確かめずに照合しているテストがないか `python/tests` を grep し，あれば同じく直す（無ければ報告にそう書く）．

## 確かめること

Python 3.14 の suite（全部），`rules.py --base HEAD --renames <今の renames.txt>`．mutation は phase6 の P02，P03 と，上で直したテストを殺し役（`python` の欄）に持つ行すべて（`plan/*/tools/mutations.json` を grep）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase6 sim02:3,sim03:3 <IDs>`．全行の `old` がちょうど 1 回一致．セルフレビューは流さない．

守ること：指示書 §5 と同じ（`cargo fmt` 禁止，mutation は remote だけ，main tree に mutant を当てない，commit・push・`git add` 禁止，`timeout 300`，指示に無い仕組みを足さない）．

## 報告

`fix2-report.md`（日本語）．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．3 本それぞれの silence の原因と直し（`path:line`），grep の結果，mutation の行ごとの結果．**起きたことを書く．していないことは書かない．**
