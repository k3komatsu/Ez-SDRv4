# 1b-5c-2：実装の指示書（今の key の宣言，`DeviceDescription`，selector の `keys` を消す）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `b3e6ae2`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b5c2/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

段階 1b-5（`plan/phase8/33-channels-and-streams.md` §5 の 1b-5 行，D12 の key の対応表）は今の key を消す切り替えの最後．

- 1b-5a（済み）：server と Python を layout の key へ．
- 1b-5b（済み）：MockRadio の今の key のモードを消した．
- 1b-5c-1（済み，`b3e6ae2`）：UHD Module の今の key のモードを消した．記録は `plan/phase8/reviews/1b5c1-opus.md`．
- **1b-5c-2（これ）**：誰も使わなくなった今の key の宣言と builder，selector の一時的な `keys` を消し，spec と Vision の文を直す．

この段階は **risky ではない**（AGENTS.md §9）．テスト計画と計画レビューは省く．

## 2. やること

1. **`crates/ezsdr-radio/src`**：
   - `device.rs` の `DeviceDescription`（と，それだけが使う `x310_defaults`・`envelope`・`tree`・`coerce` など）を消す．`Grid`・`rate_set_matches` など layout が使うものは残す．
   - `x3xx.rs` の `description`（今の builder）と，それだけが使う `Selection`・`x3xx::from_selector` などを消す．`channels.rs` が使うもの（board，slot，front end の表，`ProviderTerms`，`profile_ref`，`PROFILE`，`MASTER_CLOCK_HZ` など）は残す．何を残すかは呼び出し元を grep して決める．
   - `lib.rs` の今の key：D12 の表で「第3版」に置き換わった key（`radio.{rx,tx}.channels`・`sample_rate_hz`・`frequency_hz`・`gain_db`・`antenna`・`frequency_step_hz`・`gain_step_db`，`radio.phase_behavior_on_retune`，`radio.rx.coherent`，`path_delay_samples` があればそれも）の `keys::` の定数と `radio_keys()` の宣言を消す．「そのまま」の key（`radio.full_duplex`，`radio.hardware_time`，`radio.tx.repeat_*`，`radio.rx.block_len`，`radio.timing.*`，`radio.perf.*`）は残す．layout の description や Mock・UHD が今も読む key を消さないよう，消す前に各 key を grep する．
   - `channel_keys` を消す．
2. **selector の `keys`**：一時的な key だったので消す．`keys` を書いた selector は，他の知らない selector の key と同じに拒否される（Mock の `KEYS` 定数と受け付けのコード，UHD の UR-5 の受け付ける key の一覧から `keys` を外す）．`keys` を書いている fixture とテストからすべて外す：Mock と UHD のテスト，`crates/ezsdr-acceptance/src/layout.rs`・`rig.rs`・`tests/parity.rs`，`crates/ezsdr-server/tests/protocol.rs`，`python/tests/test_easy_api.py`（211，330 行付近），UHD の `src/provider/control.rs`・`mod.rs` のテスト．`keys` の値を確かめていたテスト（「`"channel"` と無いのは受け付け，他は拒否」）は「`keys` は知らない key として拒否」の 1 本ずつ（Mock と UHD）にする．
3. **テスト**：消したコードを使うテスト（`crates/ezsdr-radio/tests/device.rs` の `x3xx::description` と `channel_keys` のテスト，`tests/radio_model.rs` の今の key のテストなど）は，同じ規則を layout の `channels::Description` で見るテストがあれば消し，無ければ layout へ移す．テストごとに報告の表へ（移した／消した＋同じ規則を見る残ったテスト）．**到達できる系列のテストを代わりなしで消さない．** 照合が空（0 の sample，空の list）でも通るテストを作らない．
4. **1b-5b からの持ち越し**：
   - `crates/ezsdr-mock-radio/tests/mock_radio.rs` の `mr_08_effective_holds_exactly_the_ten_configuration_keys` は，名前のまま layout の 5 つの key を確かめている．中身に合う名前に変え，spec 07（`design/07-radio-model.md:103`）と spec 09（`design/09-mock-radio.md:295`）の引用も直す（`rules.py --renames` の file に書く）．
   - spec 07 は今の key の `radio.rx.coherent` を記述している（`design/07-radio-model.md:119` 付近）．D12 の表のとおり消えた key なので直す．
5. **1b-3c からの持ち越し（minor 1）**：MockRadio で，同じ origin の受信と送信の untimed tune が「受信が先，stream 番号の順，channel の並び順」で引かれること（MR-30，`design/09-mock-radio.md:205`，`tune_order`，`crates/ezsdr-mock-radio/src/lib.rs:2154`）を固定するテストを足し，mutation の行（順を入れ替える mutant）を 1 つ足す．テストは引いた位相の値（`SimRng` の stream から）で順を観測する．
6. **spec と Vision**：
   - spec 07，09，18，03 で今の key・`DeviceDescription`・`keys` を述べる文を，layout の key だけの文に直す（D12 の対応表）．spec 07・09・18 は DA-3 で書き直した spec なので AGENTS.md §2 の「Format of accepted specs」を守る（規則は normative な短い文と `*Test:*` 行，理由と経緯は Changes へ）．消えた規則の ID は 1 行の stub を残す（番号を振り直さない）．spec 03 は inline の経緯を持ってよいが，markers と長い行を増やさない．
   - Vision（`design/vision/02-spec-binding-and-compilation.md`，`03-simulation-mock-and-time.md`）の今の key の例を layout の key に直す．§ の番号は変えない．index（`Ez-SDR_v4_ARCHITECTURE_VISION.md`）の revision history に 1 行足す．
   - Kernel の `crates/ezsdr-kernel/src/spec.rs` の doc comment の例（`radio.rx.channels`，schema の `experiment_spec.v1.json` に出る）を layout の key に直し，schema を作り直して `schema_freeze` を green に保つ．version は上げない．Kernel のテストが例の文字列として使う `radio.rx.frequency_hz` など（radio の語彙を登録しないもの）はそのままでよい．
7. **mutation の行**：
   - 消したコードを `old` に持つ行は消す．**消す前に，その行の `old` が本当に今の source に無いことを確かめる．**
   - 消した・名前を変えた・`#[cfg(test)]` にした名前を **`new` に持つ行**（mutant が compile できなくなる）を grep して直す．
   - 消した・移したテストを **`filter` に持つ行**は，同じ規則を見る残ったテストへ当て直す．当て直した行は，その mutant がそのテストの道を本当に通るかを確かめる．
   - production から呼ばれなくなったコードを `old` に持つ行（1b-5c-1 の P282・R24 のように）は，同じ規則を持つ layout の道に当て直すか，理由を書いて消す．
   - ファイル全体を書き直さない．

## 3. 変えてはならない挙動

- MockRadio と UHD Module の layout の挙動（Mock・UHD・acceptance の layout のテスト，VH-8 の `parity.rs`，differential）．
- server と Python の client の挙動（Python の client のコード `python/ezsdr` は変えない．変わるのはテストの selector だけ）．
- `radio.full_duplex` など「そのまま」の key．

## 4. 大きさ

見積もりは約 1 500 行（大半は削除と文の直し）．始める前に，消す宣言と builder の行数，触るテストの数，spec と Vision の箇所の数を数えて報告に書く．2 000 行を大きく超えそうなら，どこで切るかを報告に書く（コードと文は同じ段階で直す：`rules.py` は spec の `*Test:*` の名前を確かめる）．

## 5. 守ること

- `cargo fmt`・`rustfmt` を流さない．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add`・`git stash` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．
- **指示書に無い仕組みを足さない．** production のコードに `#[cfg(test)]` だけの道を作らない．
- tree のファイルは mode 644．
- 前の段階で出た誤り：到達できる系列のテストを「同じ判断を別のテストが見ている」として代わりなしで消す．differential や generator の操作を丸めて何もしない操作にする．していない実装を報告に書く．「このテストでは差が出ない」を equivalent の根拠にする．照合が空でも通るテスト．自分で当て直した行しか mutation を流さず，`new` や `filter` で壊れた行を見落とす．

## 6. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. Python 3.14：`cargo build -p ezsdr-server` の後，`PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`．
4. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` の file を作業 dir に置く）．
5. mutation：§2 の 7 で直した・当て直した・足した行と，`new`・`filter` の grep で見つかった行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>` で流す．全行の `old` が source にちょうど 1 回一致することも確かめる．

## 7. セルフレビュー

reviewer は **DeepSeek**．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b5c2/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b5c2/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．

## 8. 報告

`~/.cache/ezsdr-fixes/2026-10-22-phase8-1b5c2/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．OpenAI の利用上限で止まるときは最初の行を `STATUS: blocked: OpenAI limit` にして，そこまでの事実を書いて終える．途中の区切り（大きさの報告など）を書くときは，最初の行を `STATUS: checkpoint` にする．中身：

1. 消した本体（`path` と行数）．
2. テストごとの扱いの表（移した／消した＋同じ規則を見る残ったテスト）．
3. mutation の行：消した行（`old` が source に無いことの確認つき），当て直した行，`new`・`filter` の grep の結果，流した結果．
4. spec と Vision の直した箇所．
5. 確かめることの結果．
6. セルフレビューの指摘と扱い．
7. 本体とテストの行数．
8. 穴．

**起きたことを書く．していないことは書かない．**

---

## 1b-5c-2 修正の回 1

作業 dir `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b5c2/`．指示書 `impl-brief.md` の §5（守ること）はそのまま効く．**作業 dir の orchestrator のファイルは読むだけ．見張りのプロセスを止めない．`git stash`・commit・push・`git add`・`cargo fmt` をしない．指示書に無い仕組みを足さない．**

Sol のレビュー（`sol.log` の最後）と orchestrator のレビューから，次をすべて直す．

1. **Sol 1（major）**：HEAD の `rm_26_coerce_cases`（`git show HEAD:crates/ezsdr-radio/tests/device.rs`）にあって残ったテストに無いケースを，番号つきの layout の key にして `crates/ezsdr-radio/tests/channels.rs` の `rm_30_coerce_cases` へ移す．少なくとも gain の切り上げ（31.3 → 31.5）と antenna の `Set` で先頭が無効なときに次の有効な値を選ぶケース（`Set(["LNA", "TX/RX"]) → "TX/RX"`）．ほかにも HEAD のケースで残ったテストに無いものがあれば同じく移し，報告に一覧を書く．
2. **Sol 2（minor）**：Vision §8（`design/vision/02-spec-binding-and-compilation.md` の例と本文）：元の例の意図（4 channel の coherent な sensing array）を残す．plan 33 D13 のとおり，channel ごとの `random_unless_timed_tune` は timed tune の後に位相差が分かる（coherent）ことを許すだけで，coherence は Run が全 channel を timed tune したときに成り立ち，成り立った関係は Provider が Manifest に記録する，と 1〜2 文で書く．§25 の言い方と矛盾させない．
3. **Sol 3（minor）**：`design/07-radio-model.md` の RM-8（`radio.d.frequency_step_hz`・`gain_step_db`）と Vision §26（`design/vision/05-coherence-calibration-and-runtime-semantics.md` の `radio.{tx,rx}.path_delay_samples`）を channel 番号つきの key だけの記述にする．他にも消えた key（D12 の表）を今の規則として書いている所が `design/` と Vision に無いか grep し，あれば直す（Changes 表の経緯の行はそのまま）．
4. **Sol 4（minor）**：HEAD の RM-26 にあった pruning の一般条件（将来の booking が先行せず将来の refusal が届かない境界より前だけを prune し，状態に畳み込み，plan を変えない）を RM-25 に移し，`rm_26_a_pruned_line_plans_as_one_that_keeps_every_item` を `*Test:*` に引く．HEAD の RM-26 の他の文も，消えたものが無いか見直す（tree・defaults・refusal・coercion は RM-29 以降にあるか）．
5. **orchestrator O1（minor）**：`plan/maintenance/tools/mutations.json` の P333 の `name` を新しい mutant（`if false {`：effective の configuration を空にする）に合わせる．
6. **orchestrator O2（minor）**：`crates/ezsdr-mock-radio/src/lib.rs` のコメント「RM-25, RM-25, RM-27」の重なりを直す（置換で同じ番号が 2 つになった所が他にも無いか `RM-25, RM-25` を grep する）．

確かめること：触ったテスト binary（`timeout 300`），`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD --renames <作業 dir の renames.txt>`．mutation：1 で足したケースが新しい行を要するなら行を足し，足した行と，`rm_30_coerce_cases` を filter に持つ行を `remote_mutate.py` で流す．全リストは orchestrator が流す．

セルフレビューは不要．`report.md` の最後に「## 修正の回 1」を足して，項目ごとに何をしたか（ファイルと行），流したものの結果を書く．最初の行は `STATUS: done`．**していないことは書かない．**
