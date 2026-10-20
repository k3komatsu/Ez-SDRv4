# 1b-4d-1c：指示書

## plan-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree はきれい）．この段階は **risky**（AGENTS.md §9：device に出す更新の経路と，configuration の版という identity を変える）．だから今回は **テスト計画だけ** を書きます．コードは書きません（Sol が計画をレビューした後に，別の指示書で実装します）．

## 1. 読むもの

- `plan/phase8/reviews/1b4d1b-adjudication.md` §3：前の段階から送られてきたもの（Sol の計画レビューの major 1〜3）．
- `plan/phase8/reviews/1b4d1b-sol-plan.md` の major 1〜3：その本文．
- `plan/phase8/33-channels-and-streams.md` の 1b-4 行「分け方」の **1b-4d-1c**．
- `plan/phase8/reviews/1b4d1-test-plan.md` の S11d と S28．
- コード：`crates/ezsdr-radio-uhd/src/provider/control.rs` の `ColdConfig`（`version`，`changed`，`configure`），`changed_projections`，`book_timed`，`release`，`cancel_held`；`core.rs` の `owner_step`，`record_configured`，`owner_configure`．
- `design/18-uhd-radio.md` の UR-24，UR-36（と UR-17・UR-23 の「the record holds that configuration」）．

## 2. 問題（D1）

`release` は timed 更新を effective instant の 3 ms 前（release window）に出す．その instant にその方向（layout では論理 channel）が stream を持たないとき，今は device に出さずに記録だけにする．そして，origin が `e` 以後でその channel を持つ segment の `ColdConfig` の版を上げる（`changed_projections` → `changed()`）．owner は segment を始めるときに版を比べて，configure し直す（UR-36）．

穴：owner は pending の segment をすぐに始める（origin のずっと前に configure して，timed start を出す）．configure の**後に**記帳された，origin より前に効く更新は，release で版を上げても，もう始めた owner は configure し直さない．その segment は古い値で流れる（HEAD の 1b-4c からある hazard）．

## 3. orchestrator の決定：届け方（前の裁定の B4 を改める）

前の裁定（`1b4d1b-adjudication.md` §3 と `plan-brief` の B4）は，「記録が owner の configure を示すなら，origin を時刻にした timed command で出す」形だった．Sol の major 2（configure の完了と記録の公開の間の窓）と major 3（timed の queue の容量）は，どちらもこの形から生じる．さらに X3xx では，同じ slot の RX と TX が 1 つの radio block の command の列を共有する．だから origin の timed command は，その後に出る別の command（たとえば同じ slot の TX の更新）を遅らせる．Mock はこの遅れを再現しないので，VH-8 の parity が崩れる（INFERRED：RFNoC の radio block の ctrlport の command は順に処理される．「A timed command will back-pressure all subsequent timed commands」は VERIFIED：`design/18-uhd-radio.md:22`，UHD の `multi_usrp.hpp` の `set_command_time`）．

**この段階の形（F）**：記録だけになる release で，その方向の **pending の segment**（origin が `e` 以後で，拒否されていない segment）がその channel を持つなら，uhd-control が release のその場で，その channel に **untimed** で apply する（`device.apply(dir, chan, settings, None)`）．

- 記録を読んで判断しない．owner が configure したかどうかによらず，常に出す．owner がまだ configure していなければ，owner の configure は投影からその更新を当てる（同じ値を 2 度当てる．害は無い）．こうすれば major 2 の窓は生じない．
- timed command ではないので，`released` に入れない．queue の slot を使わない．major 3 の会計は生じない．
- release は `e` の 3 ms 前に起き，`e ≤ origin` なので，apply は origin の 3 ms 以上前に出る．device lead（2 ms，受け取ってから要る時間）より長い（INFERRED，bench の B8）．
- release が遅れて，`now + device lead > origin` になったとき（uhd-control が止まっていた）は late とし，今の late release と同じく `LATE_COMMAND` を出す．期待の細部（requested と applied に何を入れるか）は計画で提案する．
- `applied` の行は今のまま（`at` は `e`，`note: "no channel"`）．VH-8（`crates/ezsdr-acceptance/tests/parity.rs:685-703`）が比べるのは `key`・`claimed`・`at`．欄を足すなら，そこを確かめる．
- owner の configure と同時に走らないように，apply の間はその方向の投影の `updates` を持つ．issued の枝と同じで，`assert_line_free` を置き，core の lock は持たない（UR-25）．
- pending の segment が無い，またはどの segment もその channel を持たないなら，今のまま記録だけにする．device は呼ばない．

退けた案（計画に 1 行ずつ書く）：
- origin を時刻にした timed command（前の B4）：上の理由．
- 更新の `e` を pending の segment の origin まで切り上げる（前の計画の B）：記録する instant が変わる．Mock と spec 07 の両方を変える必要がある．
- 始めた owner が origin まで版を見直す：origin を過ぎた後の窓が残る．owner の loop も変わる．

**版**：この形では，release が版を上げる必要がなくなる．device はどの場合も値を受け取るからである．残る使い道は `cancel_held` だけになる．`cancel_held` の呼び出し元が `Provider::stop` だけで，stop の後にどの owner も configure しないなら，`ColdConfig.version`・`configured` の `(seq, version)` の版・`Planned::version()`・`owner_step` の版の比較を消せる．計画でこれを確かめ（呼び出し元を grep し，stop の後の owner の経路を読む），消せるなら消す計画にする．消せないなら，その理由と，残す読み手を書く．

## 4. 系列（各系列：テスト名，assert 1〜2 個，何で観測するか，落とす誤った実装）

最初に，brief を読む前のあなた自身の失敗の系列を 3 つ以上書き，下と比べる．

1. **S1（D1，RX）**：from-none の enable を origin `O` に記帳する．owner が begin して configure する．その後，`e < O` の gain 更新を記帳する．release（channel が無い）→ uhd-control が untimed で apply を 1 回．owner は configure し直さない．fake の read-back は，fake 自身の時計が `O` を越えた後にその値．**実際の `segment.origin` を読む**（RX の start lead で 50 ms 以上．Sol major 1）．
2. **S1-TX**：同じ系列を送信で．
3. **S2（owner がまだ）**：更新が owner の configure の前に記帳される．最後の値は正しい．control の apply が出ること（2 度当てる）も固定する．
4. **S3（Sol major 2 の窓）**：owner の configure を device の apply の中で止める（LockSpy の gate）．その間に timed 更新を記帳し，release する．`updates` で直列になり，最後の値が正しいこと．gate と hook で同期する．sleep は使わない．
5. **S4（queue，Sol major 3）**：channel が無い間の記録だけの更新を `command_queue_depth` 個より多く受け，その後に live な channel への timed 更新を出す．`COMMAND_QUEUE_FULL` にならず，`released` も増えない．
6. **S5（遅れた release）**：`now + device lead > O` で release する．late として扱い，`LATE_COMMAND` が 1 回出る．
7. **S6（持つ segment が無い）**：pending の segment が無い，またはどれもその channel を持たない．device は呼ばれず，記録だけになる．
8. **S7（layout）**：最初の pending の segment はその channel を持たず，後の segment が持つ．apply が出る．論理 channel の番号で出る．
9. **S8（拒否された segment）**：拒否された segment だけがその channel を持つ．apply は出ない．
10. **S9（loss）**：release の前，または apply の失敗で loss になる．apply は出ず，`device_failed` になる．今の経路と同じ．
11. **S10（lock）**：apply の間，その thread は core の lock を持たない（1b の `assert_line_free`）．
12. **S11（版を消すなら）**：stop の後の経路で，消した版の代わりに何が owner の configure を決めるか．既存のテスト（1a の S11a〜S11c，`ur_24_a_record_only_release_after_a_from_none_configure_makes_the_owner_configure_again` など）を，残す・書き直す・消すのどれにするか，1 本ずつ理由をつける．

## 5. 計画に書くこと

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan.md` に日本語で書く．

1. あなた自身の系列と，§4 との差分．
2. 系列ごとに：テスト名，置く場所，assert（1〜2 個），観測（device の log，fake の read-back，記録，event），落とす誤った実装．
3. 版を消せるかどうかの判断（§3 の最後）と，その根拠の `path:line`．
4. 変える `design/` の文：UR-24（記録だけの release が pending の segment に届くこと），UR-36（版を消すなら，その全文），UR-17・UR-23 の「the record holds that configuration」の部分，それと Changes の行．DA-1 の形で．他に偽になる文があれば grep で探して挙げる．
5. mutation の行の方針：新しい規則ごとに 1 行．行の文字列はコードと一緒に書くので，ここでは対応だけ．
6. 大きさ：本体とテストの行数の見積もり（1,500 行以内に収める）．
7. 確かめられなかったこと（INFERRED）．

## 6. 守ること

- **コードを書かない**．tree を変えない．`cargo fmt`・`rustfmt`・`mutate.py`・`remote_mutate.py` を流さない．commit・push・`git add` をしない．
- 読むための `cargo test` は流してよい（build の出力は tree の外の，いつもの target）．背景の job を残さない．長い command は timeout を延ばし（600000 ms），前景で流す．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- 前の段階で出た誤り（同じ形を繰り返さない）：到達できる系列を「到達できない」と言って消す．実際の経路を通さず，記録を手で書き換えて確かめる．
- 書けたら，最初の行を `STATUS: done`（または `STATUS: blocked: <理由>`）にして終える．

## impl-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree はきれい）．この段階は risky（AGENTS.md §9）．

## 読むもの（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/`）

優先の順：
1. `adjudication-3.md`（あれば．Sol の計画 v3 の確認の裁定）
2. `adjudication-2.md`
3. `adjudication.md`
4. 計画 `test-plan-v3.md`（あなたが書いた．裁定で直すところ以外はこれに沿う）

## 計画 v3 が求めた裁定（orchestrator）

- §2.1 の時計の道具（`FakeDevice::set_time`・`Frozen`・`read_back`）：採る．test build だけにし，fake のほかの模型を変えない．
- §2.3 の失敗の除去の文面：採る．
- §3 S12(b)（`e` と effective の間の cold 変更）：残す．
- §7 の大きさ：1,500 行を少し超えても分けない．S16〜S18 は動かさない．超えたら報告に数を書く．

## やること

1. **実装する**：計画 v3（と裁定 3 があればそれ）のとおり．計画に無い状態や失敗の経路を実装で足したら，テストを足して計画（`test-plan-v3.md` に追記）と報告に書く．
2. **`design/18-uhd-radio.md`**：計画 v3 の §5 のとおり，DA-1 の形で書く．
3. **mutation の行**：計画 v3 §6 のとおり．新しい規則ごとに 1 行．版と `changed_projections` に当たる既存の行は，コードと一緒に消す．動いた既存の行は当て直す．名前を変えたテストを引く行は，`new` の識別子が残っていないか grep する．
4. **rules.py**：消した・名前を変えたテストの引用は `--renames` で渡す（計画 v3 §5）．

## 守ること

- `cargo fmt`・`rustfmt` を流さない（tree はわざと rustfmt の形にしていない．mutation の行はソースの文字列にちょうど一致する必要がある）．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．
- `plan/*/tools/mutations.json` はファイル全体を書き直さない．変える行・足す行だけを，周りの書式のまま書く．
- 背景の job を残さない．長い command（`cargo test --workspace`，`rules.py`，`remote_mutate.py`）は timeout を延ばし（600000 ms），前景で流す．
- Python は `PYTHONDONTWRITEBYTECODE=1`．
- テストの繰り返しは helper にする．テストは系列ごとに assert を 1〜2 個．sleep で競合を作らない（gate と probe で同期する）．
- 前の段階で出た誤り（同じ形を繰り返さない）：計画の要のテストを飛ばす．到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．記録を手で書き換えて，実際の経路を通さずに確かめる．

## 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．（`--features uhd` の clippy はこの機械では流せない．orchestrator が image の中で流す．）
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（`--renames` を付けて）．
3. mutation：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>` で，足した行・変えた行と，コードかテストが変わった行を流す．全リストは orchestrator の gate で流す．COMPILE ERROR も生き残りとして扱う．

## セルフレビュー

reviewer は DeepSeek．回ごとに新しく起動する．

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat <チェックリストのファイル>)" > <ログ> 2>&1
```

チェックリスト（ファイルに書く）．読む範囲は `git diff HEAD` のうち `crates/ezsdr-radio-uhd/src/` と `design/18-uhd-radio.md`．reviewer には「read-only．cargo・mutation・mutate.py・remote_mutate.py を流さない．指摘を P1〜P3 で，`path:line` つきで．無ければ『P1 なし』」と書く．項目：
1. 計画 v3 の各系列にテストがあり，そのテストが計画の言うものを観測するか．
2. release が device に出すすべての経路（live，pending，put back の後）が，`Some(effective)`，`released`，`issued()`，depth の上限を通るか．失敗が投影から除かれるか．
3. 版を読む・書く場所が残っていないか（grep：`version()`，`changed(`，`configured` の組）．
4. この変更で偽になる `design/18` の文（UR-17，UR-23，UR-24，UR-36，§6 の表）が残っていないか．

P1 が無くなるか 3 回まで．判定を出さずに終わった回は 1 回だけ起動し直す．指摘はすべて直すか，理由をつけて退ける．

## 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/report.md` に日本語で書く．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．範囲の大きさを理由に blocked にしない．
中身：実装したもの，計画 v3 から変えたところと理由，テストと行（足した・変えた・消した），確かめることの結果（mutation は行ごと），セルフレビューの各回の指摘と扱い，変えた `design/` の文，大きさ（本体とテストの行数）．

## fix1-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree に 1b-4d-1c の実装が未 commit で入っている．あなたが書いたもの）．Sol のコードレビュー（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-review.log` の末尾，`判定：FAIL` 以降）の 4 件を，次の裁定のとおり直してください．

## 裁定

1. **Sol major 1（`targets` が空でも，その更新を要る segment が残る）**：採る．
   - 原因：live な枝は `issued()` で投影の entry を `effective` へ動かす．記録だけの枝（`chans.is_empty()`）は動かさず，`e` のまま残す．`effective > e` のとき（release 済みの command の後ろ），device の in-order の queue ではこの更新は `effective` より前に効かない．それなのに投影は `e` から効く形になり，`e` と `effective` の間に origin を持つ segment で device と食い違う．
   - 直し：記録だけの枝でも，`effective > e` なら，全投影の `updates` の entry を `(e, seq)` から `(effective, seq)` へ動かす．live な枝の `issued()` と同じ関数を使い，同じ lock の取り方（`assert_line_free`，`projections` の `updates`）にする．行（`at: e`，`note: "no channel"`，`issued` なし）と `config.insert` は今のまま．`LATE_COMMAND` は足さない（今の記録だけの枝のまま）．
   - テスト：Sol の系列（RX は channels=0，from-none の enable を 100 ms に予約して configure 済み → RX channels=0 を 100.2 ms → 97.5 ms に TX の live な更新を 100.5 ms で release → 97.6 ms に RX gain=3 を 99.7 ms で予約・release → RX の owner が 100 ms の segment を begin）を 1 本．assert：(1) 投影の entry が `(100.5 ms, seq)` にあり，100 ms の segment の投影の gain は旧値．(2) owner は configure せず，device は旧値で，行は `at: e`・`note`．
   - 名指しする誤った実装：記録だけの枝で entry を動かさない（投影の gain が 3 になる）．mutation の行を 1 行足す．
   - `design/18`：UR-24 の第 3 段落の「With no such segment … is recorded in the configuration at its own instant」に，「late behind a released command, at the instant it takes effect」の旨を足す（DA-1，1 文）．UR-36 の配送の文（`design/18-uhd-radio.md:271`）がこの系列で偽にならないか確かめる．
2. **Sol major 2（S3 に時計の固定が無い）**：採る．S3（`ur_24_a_release_during_a_configure_s_return_reaches_the_device`）の最初の booking の前に，他の系列と同じ `hold(&core, &time, &device)` を置く．報告（`report.md` §2 の 4 と §5 の回 1）は「足した」と書いていたが，tree に無かった．報告を直す．
3. **Sol minor 3（TX の主系列の read-back で時計が一致していない）**：採る．`tx.rs` の S1-TX で，read-back の前に Core も同じ時刻へ進め，`(core.now(), device.time_now()) == (at, at)` を assert する（RX の `device_at` と同じ形．helper を共有できるならそうする）．
4. **Sol minor 4（UR-24 の容量の文）**：採る．`design/18-uhd-radio.md:253` の文を，「timed の booking の後に予約された cold change が，release の出し先の channel を増やしうる」と書く．理由と既知の制限は Changes の行に置く．

## 守ること

- `impl-brief.md` の「守ること」のとおり（`cargo fmt` を流さない，mutation は `remote_mutate.py` だけ，`mutations.json` は全体を書き直さない，commit・push・`git add` をしない，背景の job を残さない）．
- 修正の回では，触った行と新しい行だけを流す：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:8,sim03:8 <IDs>`．

## 確かめること

1. `cargo test -p ezsdr-radio-uhd --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`．
3. mutation：新しい行と，触ったコード・テストに当たる行．

セルフレビューはしない（この回は小さい．Sol が修正を見直す）．

## 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/fix1-report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：4 件それぞれの直しと `path:line`，足したテストと行，確かめることの結果（mutation は行ごと）．**していないことを書かない**．

## fix2-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree に 1b-4d-1c と修正の回 1 が未 commit で入っている．あなたが書いたもの）．

## なぜ（設計の判断，three strikes）

Sol の修正の再レビュー（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-rereview.log` の末尾，`判定：FAIL`）の major 1・2 は，どちらも修正の回 1 の「投影の entry を release のときに `(e, seq)` から `(effective, seq)` へ動かす」から生じた．
- 動かす前に owner が configure していれば，device は `e` の値を受け取ったまま残る（major 2）．
- `config.insert` は時刻によらない基底なので，動かした後に作った segment は `effective` より前から値を持つ（major 1）．

この段階では，同じ仕組み（記録と device の値の食い違い，`planned-state-read-as-actual`）の指摘がこれで 3 回を超えた．だから局所の直しをやめ，**原因を消す**．

原因は，timed command の効く時刻を booking で `e` と決めながら，release で「release 済みの command の後ろ」として `effective` に変えていることである．後から変えた時刻は，その投影をすでに読んだ configure に届かない．

**確定できる理由**：
- release は held を `(e, seq)` の順に出す（`release` の `first_entry`）．
- booking のとき `released` にある command の最後の時刻を `L` とする（`retain(> now)` の後）．booking の後，この command より先に release される held command は，`e` がこの command より小さいものだけである．
- その effective は `max(e_h, L) ≤ max(e, L)` なので，`L` を越えて大きくなることはない．
- だから，この command の release のときの `effective = max(e, latest)` は，booking のときの `max(e, L)` と等しい（INFERRED：コードから読んだ推論．テストで固定する）．

## やること

1. **booking の床**（`book_timed`）：効く時刻を `e = max(requested, now + device lead, L)` にする．`L` は `released` の最後の時刻（`retain` の後．空なら無し）．
   - requested がこの床より前なら，今の lead の遅れと同じく `LATE_COMMAND`（requested と，床で決まった `e`）を booking のときに 1 回出す．lead と `L` のどちらで遅れても，出すのは 1 回．
   - `e` を `held` の key と投影の entry に使う．
2. **release**：`effective` を求める式は残してよい．ただし `debug_assert_eq!(effective, e)` を置き，その意味（booking で確定済み）をコメントに 1 行書く．
   - release での `late_command` と，`issued()` による投影の移動（live な枝と，修正の回 1 で足した記録だけの枝の両方）を消す．`ColdConfig::issued` を使う所が無くなれば，関数ごと消す．
   - 行の `at` は `e`（live も pending も．`effective == e`）．
3. **queue の会計**：変えない（booking で `targets` の長さ）．
4. **テスト**：
   - S12（`ur_24_a_release_behind_a_released_command_keeps_the_device_queue_order` の (a)(b)）を，booking で決まる形に書き直す．
     - 1 つ目の command（10 000 µs）を release した後に，9 700 µs を requested として 2 つ目を booking する．
     - booking のときに `LATE_COMMAND`（requested 9 700，applied 10 000）が 1 回出る．held の key と投影の entry が 10 000 µs で，release で device に `at=10000` で出る．投影は release の前後で動かない．
     - (b) の 2 channels も同じ形で．
   - 修正の回 1 のテスト `ur_24_a_release_recorded_behind_a_released_command_moves_its_projection` を，Sol の系列のまま書き直す．RX gain=3 を 99.7 ms で booking するときに，効く時刻が 100.5 ms に決まり（`LATE_COMMAND`），100 ms の segment の投影は旧値．release は記録だけで，投影は動かない．名前は `..._is_booked_at_the_instant_it_can_take_effect` のように意味に合わせて改める．
   - **Sol の再レビューの 2 つの系列を，それぞれ 1 本のテストにする**：
     - (major 1) その後に RX の enable を 100.3 ms に予約すると，新しい segment の投影は 100.3 ms で旧値，100.5 ms からは 3．
     - (major 2) RX の owner が configure する segment（from-none ではない）で，owner の configure が 99.7 ms の booking の後に来る．owner の configure は 100 ms の値として旧値を device に渡す（3 を渡さない）．probe か gate で順序を固定する．
   - 既存の live な枝の late のテスト（`ur_24_the_device_queue_order_is_the_effective_order` など）は，`LATE_COMMAND` が出る時点（booking）と，`held` の key に合わせて直す．期待そのもの（device の順序，`at`）は変えない．
5. **mutation の行**：消したコード（release の `late_command`，`issued()`，修正の回 1 の移動の行 P425 など）に当たる行は消す．新しい規則ごとに 1 行足す：booking の床に `L` を使う，`LATE_COMMAND` を booking で出す，`debug_assert`（落とせないなら行を作らず理由を報告に書く）．
6. **`design/18-uhd-radio.md`**：UR-24 で，「A command whose `e` precedes a command already released …」と effective の文を，「効く時刻は booking で決まる：requested を，device lead と，release 済みの最後の command で床にする．その床で遅れれば booking で `LATE_COMMAND`」に置き換える（DA-1）．修正の回 1 で足した「記録だけの release は effective の instant に投影へ移る」を消す．Changes の 2026-10-20 の行を合わせる．UR-36 の配送の文（`:271`）が全経路で真か確かめる．

## 守ること

- `impl-brief.md` の「守ること」のとおり（`cargo fmt` を流さない，mutation は `remote_mutate.py` だけ，`mutations.json` は全体を書き直さない，commit・push・`git add` をしない，背景の job を残さない）．
- **1 本のテストや 1 つの test binary を流すときは `timeout 300` を付ける**（修正の回 1 では，固まったテストを 20 分待っていた）．
- 修正の回では，触った行と新しい行だけを流す．

## 確かめること

1. `cargo test --workspace --no-fail-fast` と `cargo clippy --workspace --all-targets -- -D warnings`．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/renames.txt`（名前を変えたテストは renames に足す）．
3. mutation：新しい行と，触ったコード・テストに当たる行．

セルフレビューを 1 回（DeepSeek，`impl-brief.md` の形．チェックリスト：(1) 効く時刻が booking の後に変わる経路が残っていないか（`effective`，`issued`，投影の entry の key を grep），(2) 新しいテストが Sol の 2 つの系列を実際の経路で通るか，(3) `design/18` の UR-24・UR-36 の文が実装と一致するか）．

## 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/fix2-report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：直しと `path:line`，足した・直した・消したテストと行，確かめることの結果（mutation は行ごと），セルフレビューの指摘と扱い．**起きたこと（固まったテスト，途中の失敗）も書く．していないことは書かない**．

## fix3-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree に 1b-4d-1c と修正の回 1・2 が未 commit で入っている．あなたが書いたもの）．

## なぜ（orchestrator のレビュー）

修正の回 2 は，channel の無い command を `released_at` と `recorded` に移し，release のたびに決め直す形にした．この形には順序の穴がある．

**反例**：
1. 記録だけの command A（`e` = 100.5 ms）は `released` に入らない．
2. そのため，後から booking した command B（requested 99 ms）の床に A の 100.5 ms が入らず，B の `e` は 99 ms になる．
3. 次の pass で A に channel が来ると，`recorded` の loop が先に A を device に渡す（`released = [100.5]`）．
4. その後の held の loop で B を release すると，`effective = max(99, 100.5) = 100.5 ≠ e` になり，`debug_assert_eq!(effective, e)` が落ちる．release build では device が B を 100.5 ms に適用し，投影（99 ms）と食い違う．

`recorded` を別に持つかぎり，device に出す順と `e` の順がずれうる．だから別の集合をやめる．

## 規則（これに直す）

1. **channel を待つ command は held に残す**．`recorded` を消す．release の loop は held を `(e, seq)` の順に見て，window 内の先頭の command について：
   - `targets(dir, chan, e)` が空でない → 今のとおり `give`（`Some(e)`，`released`，行）．
   - 空で，`e > now` → **その command を held に残し，この pass を止める**．後ろの command はその後で出る．device に出す順は常に `e` の順になり，`effective == e` は崩れない．
   - 空で，`e <= now` → held から除き，`no channel` の行を書き，構成の記録に畳む（今の「instant が過ぎた recorded」と同じ扱い）．
2. **`released_at` は残す**．release した（`give` した）command を，効く時刻で構成の記録に畳むためのもの（HEAD にあった「live な release の値が，`e` より前に始まる segment の基底に入る」穴も閉じている）．channel の無いまま `e` を過ぎた command は，そのまま `config` に畳む．
3. **held に残すので，次はそのまま効く**：`Provider::stop` の cancel（`cancel_held`），layout の queue の数え方（`held.len()`），`held_updates`（後に作る segment の投影）．`recorded` のための足し算（`recorded.len()`，`cancel_held` の chain）は消す．
4. **止めることの影響**：channel 待ちの command の後ろの command は，その `e` が過ぎるまで出ない．後ろの command の `e` が近ければ，device lead より短い時間で出て，device は着いたときに適用する（今の遅れた release と同じ．新しい late の規則は作らない）．これを `design/18` の UR-24 に 1 文で書く．

## テスト

- **反例を 1 本**：A を記録だけにした後に B を booking し，同じ pass で A に channel が来る．device に出る順が B（99 ms）→ A（100.5 ms）で，両方の `at` が自分の `e` であること．`debug_assert` も落ちない．
- 修正の回 2 のテスト（`ur_24_a_segment_booked_after_a_recorded_release_holds_it_at_its_instant`，`ur_24_a_segment_begun_before_a_recorded_release_configures_the_old_value`，`ur_24_a_recorded_command_is_given_before_a_later_one`，`ur_24_a_stop_cancels_a_recorded_command`）は，held に残す形で同じ期待が成り立つか確かめ，必要なら直す．
- **止めることを 1 本**：channel 待ちの A の後ろに，live な B（`e` が A より後）がある．A の `e` が過ぎるまで B は出ず，A が `no channel` で畳まれた後に B が出る．
- 消した仕組みに当たる mutation の行は消し，新しい規則ごとに 1 行足す（止める `break`，`e <= now` の畳み込み）．

## design/18

UR-24 の「recorded … decided again」の文を，「channel を待つ command は held に残り，release はそこで止まる（device に出す順は `e` の順）．`e` を過ぎても channel が無ければ `no channel` で記録し，構成の記録に畳む」に直す（DA-1）．UR-26・UR-30・UR-36・Changes の「recorded」の語も合わせる．

## 守ること・確かめること・報告

- `fix2-brief.md` と同じ（`cargo fmt` を流さない，mutation は `remote_mutate.py` だけ，`mutations.json` は全体を書き直さない，commit・push・`git add` をしない，1 本のテストには `timeout 300`）．
- 確かめること：`cargo test --workspace --no-fail-fast`，clippy，`rules.py`（`--renames`），mutation は新しい行と触った行．
- セルフレビュー 1 回（DeepSeek，read-only）．チェックリスト：(1) device に出す順が `e` の順から外れる経路が残っていないか，(2) `recorded` の名残（変数，doc，design の語）が残っていないか，(3) 新しいテストが反例を実際の経路で通るか．
- 報告：`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/fix3-report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．**起きたことも書く．していないことは書かない**．

## fix4-brief


あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `cddad68`，tree に 1b-4d-1c と修正の回 1〜3 が未 commit で入っている）．Sol の再レビュー（`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-rereview2.log` の末尾，`判定：FAIL` 以降）の 3 件を，次のとおり直してください．規則 1〜3（booking で時刻を確定，channel 待ちは held に残って release を止める，`released_at` の畳み込み）は変えない．

## 1. Sol major 1：畳み込みの順序（採る）

- channel の無いまま `e <= now` になった command も，その場で `of_record` に渡さず，`released_at` に `(e, seq)` で入れる．`no channel` の行はそのとき書く（今と同じ時点）．
- 構成の記録への畳み込みは，release の最後の 1 つの loop だけにする（`released_at.range(..=now)` を `(e, seq)` の順に）．`self.config.insert` を呼ぶのは `of_record` の 1 か所のまま．
- held は `e` の順に処理されるので，release 済みの A（`e_A`）がまだ `released_at` にあって，それより前の `e` の command が後から入ることは無い．これを 1 行のコメントで書く．
- テスト：Sol の系列（RX に channel があり，101 ms で全 channel が抜ける．後の enable は無い．gain=1 の A（`e` = 100 ms）を 97.5 ms に release．同じ key の gain=2 の B（`e` = 102 ms）を予約．次の release は 102.5 ms）．assert：(1) 最後の `config_at` が 2．(2) その後に予約した enable の segment の configure が gain 2 を device に渡す．
- mutation：畳み込みを 1 つの loop にした規則に 1 行足す（例：channel 無しの枝で `released_at` に入れずに直接 `of_record` する形に戻す）．

## 2. Sol major 2：release の channel の間の loss（採る）

- `give` の各 channel の `apply` の前と，release の loop の各 command の前に `self.core.is_lost()` を見る．lost なら，device を呼ばずに返す（`device_failed` は呼ばない．loss はすでに公開されている）．`Core::configure` の channel ごとの検査（`core.rs:730-737`）と同じ形にする．
- テスト：2 channel の timed release で，channel 0 の `apply` が成功した直後に，別 thread の loss を公開する（LockSpy の gate，または `Core::probe` で `device_lost` を呼ぶ）．assert：(1) channel 1 の `apply` は呼ばれない（入口で記録する attempt の列で見る）．(2) 後に続く command も呼ばれない．
- mutation：各 channel の前の検査を消す行を 1 行．

## 3. Sol minor 3：UR-24 の理由の文（採る）

- `design/18-uhd-radio.md:251` の「so that … never has to move its instant」は理由なので，規範から外して Changes の行へ移す．booking の floor と `LATE_COMMAND` の説明が `:250–251` で重なっているので，1 回だけにする（DA-1）．
- UR-24 か UR-29 に，release が channel の間で loss を見ることを 1 文で足す．`*Test:*` に新しいテストを足す．

## 守ること・確かめること・報告

- `fix3-brief.md` と同じ（`cargo fmt` を流さない，mutation は `remote_mutate.py` だけ，`mutations.json` は全体を書き直さない，commit・push・`git add` をしない，1 本のテストには `timeout 300`）．
- 確かめること：`cargo test --workspace --no-fail-fast`，clippy，`rules.py`（`--renames`），mutation は新しい行と触った行．
- セルフレビューは 1 回（DeepSeek，read-only）．チェックリスト：(1) `config.insert` へ値が入る経路がすべて `(e, seq)` の順か，(2) release が device を呼ぶすべての点の前に loss の検査があるか，(3) 新しいテストが Sol の系列を実際の経路で通るか．
- 報告：`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/fix4-report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．**起きたことも書く．していないことは書かない**．

## sol-plan-brief


You are the independent reviewer (AGENTS.md §9 step 3). Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD `cddad68`, clean). **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

## The step

D1 option A: a timed update `release` records with no channel (`chans.is_empty()` in `Control::release`, `crates/ezsdr-radio-uhd/src/provider/control.rs`) must still reach the device before the origin of a pending segment that holds its channel. Today it bumps the `ColdConfig` version of the segments it reaches, which helps only an owner that has not yet configured; an owner that already began keeps the old value (the hazard). You reviewed the earlier form of this (`plan/phase8/reviews/1b4d1b-sol-plan.md`, majors 1–3: a segment begun first, the window between a configure and its record, the device queue's accounting and a past origin).

The orchestrator changed the form (brief `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/plan-brief.md` §3, "F"): at the release, uhd-control applies the value **untimed** on the pending segment's channels, whatever the record says; nothing goes into `released`; the applied row keeps `at = e`; a release too late for the origin is `LATE_COMMAND`. With that, nothing moves the version but `cancel_held`, which only Provider::stop reaches, so the plan proposes removing the version entirely (the record's identity of a configuration becomes the segment's command `seq` alone).

**First**, before reading the brief §3 and the plan, write your own failure scenarios for this purpose from the code (`provider/{control,core,rx,tx,mod}.rs`, `device.rs`) and UHD 4.10 where it matters (`docker run --rm ghcr.io/k3komatsu/uhd:v4.10 …`, sources under `/root/tmp/uhd-4.10.0.0/`): is an untimed apply at the release (≥ 3 ms before `e`, `e ≤ origin`) right on X3xx for a segment whose timed start is already queued; what else could change a segment's projection without the device being told once the version is gone (cold changes, layout moves, refusals, prune, arm, a configure that fails half way); concurrency with an owner's configure on the same channel; queue order; Mock parity (VH-8, `crates/ezsdr-acceptance/tests/parity.rs:685-703`).

**Then** read the brief and the plan `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan.md`, and compare.

The orchestrator's notes, for you to check rather than accept:
- F in place of a timed command at the origin: the reasons in brief §3 (your majors 2 and 3 do not arise; an origin-timed command would hold back later commands on the same radio block, which Mock does not model — INFERRED). Is F sound, and is anything lost against the timed form?
- Removing the version (plan §3): the plan says only two sites bump it and the second is unreachable after Provider::stop. Is any reader of the version left that matters, and does `configured == Some(by)` alone suffice?
- The plan holds the projections' `updates` across the untimed apply and admits the mutation row for it cannot be killed (plan §5, S3b). The orchestrator thinks the lock is unneeded: releases go in `e` order and the device ends with the last value either way, so the lock and S3b should go. Confirm or refute with a sequence.
- H (plan §0, S13): a direction halted by a refusal with a pending, unrefused enable now gets the value untimed. Right?
- S5's late payload (`requested: e`, `applied: now + device lead`).

Report:
1. Your scenarios the brief and plan miss, each with its expected result.
2. Wrong or weak points of F and of removing the version; your recommendation.
3. Plan tests that would not fail on the wrong implementation they name, or observe the wrong thing (S3's probe, S12, S14 in particular).
4. The `design/18` text in plan §4: correct, DA-1 shaped, anything it makes false that is not listed.
5. Whether the size estimate is credible.

Japanese. Severity major / minor for each item, `path:line` where it applies. Mark unverified points INFERRED. End with `判定：PASS` or `判定：要修正`.

## sol-plan2-brief


You are the independent reviewer (AGENTS.md §9 step 3). Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD `cddad68`, clean). **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

You reviewed plan v1 (`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/test-plan.md`; your review is the end of `sol-plan.log` in the same directory, verdict 要修正). The orchestrator adopted all your findings and changed the form to F' (`adjudication.md` in that directory): a release with no live channel goes to the device as a **timed command at its own effective instant**, on the channels of the unrefused segments planned to begin at or after it, through the same path as a live release (`released`, `issued()`, late behind a released command); the queue counts those channels; the applied row stays `at: e`, `note: "no channel"`; the configuration version is removed. Plan v2 is `test-plan-v2.md` there.

Check:
1. Each of your findings 1–10: closed by v2, or what is still open (with the sequence).
2. New failure scenarios F' itself opens: a timed command at `e` on a front end whose stream is not running (X3xx, UHD 4.10 — sources under `/root/tmp/uhd-4.10.0.0/` in `docker run --rm ghcr.io/k3komatsu/uhd:v4.10 …`), on a halted direction (S13), a segment cut or refused after the release, the queue counted at booking only (plan §8), the owner's untimed configure after a released timed command (FIFO order), version removal (plan §4).
3. Tests that would not fail on the wrong implementation they name (S4's 17th command, S12, S14, S16–S18's 99/101 ms read-backs on the fake's model, S9's new `ApplyFails` modes, S3's probe).
4. The decisions the plan asks for (its §0 last paragraph): D1 (`issued: true` on the `note` row), S5 (late release as today), S9 (`ApplyFails` modes), S15 (stop/owner window to the next stage), §7 (no split).
5. The `design/18` text of plan §5: correct, DA-1 shaped, anything still made false.

Japanese. Severity major / minor, `path:line`. Mark unverified points INFERRED. End with `判定：PASS` or `判定：要修正`.

## sol-plan3-brief


You are the independent reviewer (AGENTS.md §9 step 3). Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD `cddad68`, clean). **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

In `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/`: your re-review of plan v2 is the end of `sol-plan2.log` (要修正, majors 1–6); the orchestrator's ruling on it is `adjudication-2.md`; plan v3 is `test-plan-v3.md` (its §0 lists the changes from v2).

This is a narrow check, not a new full review:
1. Your majors 1–6 of the v2 re-review: closed by v3, or what is still open (with the sequence).
2. The two new rules of `adjudication-2.md` §1: (1) a release never fills the device queue past its depth — a command that would is put back in `held` and released on a later pass, late as today if `e` has passed; (2) a release `apply` failing (not lost) removes that `(e, seq)` from every projection before `device_failed`. Are they sound against the code (`crates/ezsdr-radio-uhd/src/provider/control.rs` `release`, `book_timed`, `cancel_held`) and UHD 4.10 (`docker run --rm ghcr.io/k3komatsu/uhd:v4.10 …`, `/root/tmp/uhd-4.10.0.0/`)? In particular: a put-back command and the in-order rule `effective = max(e, latest released)`; a put-back command when `Provider::stop` cancels; a held command that a later pass releases behind one it was ahead of; removing a failed update that an owner's configure, already holding `updates`, is about to read.
3. The test tools of v3 §2.1 (`FakeDevice::set_time`, `Frozen`, `read_back`): do they couple the two clocks without changing what the fake models elsewhere?
4. Anything in v3 that would make a named wrong implementation pass.

Japanese. Severity major / minor, `path:line`. Mark unverified points INFERRED. End with `判定：PASS` or `判定：要修正`.

## sol-brief


You are the independent reviewer (AGENTS.md §9 step 7). Repository: /home/komatsu/local_work/Ez-SDRv4. **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push. Read-only commands are fine.

The uncommitted tree (`git diff` against HEAD `cddad68`) implements stage 1b-4d-1c, whose plan you reviewed three times (`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/`: `sol-plan.log`, `sol-plan2.log`, `sol-plan3.log`; rulings `adjudication.md`, `adjudication-2.md`, `adjudication-3.md` — the last one wins: it withdrew the put-back at the depth and the removal of a failed update, and accepted your clock findings; plan `test-plan-v3.md` with its §10; the implementer's `report.md` — do not trust it, check the tree).

In short: `Control::targets` gives a timed command its channels (live ones, else those of unrefused segments planned at or after the instant); `release` sends every non-empty target set through the one timed path (`Some(effective)`, `released`, `issued()`), recording a pending one `at: e`, `note: "no channel"`, `issued: true`; `book_timed` counts with `targets` under the current keys; the configuration version is gone (`configured: Option<u64>`, `owner_step` by set and command).

Look for:
1. Your own failure scenarios first, then: every path that issues a command to the device, every path that read the version, and whether anything still lets a segment run without an update the projection holds (other than the documented refused-release case of `adjudication-3.md` §1.2 and the booking-only accounting of §1.1).
2. Behaviour that must not change: the live release path (rows, lateness, counts), the 1b-4d-1b ownership and lock rules, Mock parity (`crates/ezsdr-acceptance/tests/parity.rs`), `differential.rs`.
3. Tests' power: each sequence's test observes what the plan says and its mutation row (P405–P424 and the re-anchored rows in the report) breaks it; the two clocks are coupled where a read-back is asserted (`device_at`, `hold`, `FakeDevice::set_time`, `Frozen`), and the test-only tools change nothing else the fake models.
4. `design/18-uhd-radio.md` as changed: correct against the code, DA-1 shaped, anything still made false.

Output in Japanese. First line `判定：PASS` or `判定：FAIL`. Numbered findings with severity (major / minor), `path:line`, evidence, scenario, fix. Mark unverified points INFERRED.

## sol-rereview-brief


You are the independent reviewer (AGENTS.md §9 step 8). Repository: /home/komatsu/local_work/Ez-SDRv4. **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

Your code review of this stage is the end of `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-review.log` (FAIL: majors 1–2, minors 3–4). The orchestrator adopted all four (`fix1-brief.md` in that directory); the implementer's account is `fix1-report.md` (do not trust it; check the tree, `git diff` against HEAD `cddad68`).

The fix for major 1: in `Control::release`'s recorded-only branch, when `effective > e`, every projection's entry moves from `(e, seq)` to `(effective, seq)` with the same `issued()` and lock as the live branch; the row and `config.insert` are unchanged; test `ur_24_a_release_recorded_behind_a_released_command_moves_its_projection`, row P425.

Check:
1. Each of your four findings: closed, or what is still open.
2. The fix's neighbours: does moving the entry to `effective` open a new mismatch (a segment with origin in `[e, effective)` on another channel or stream; a layout command; a later held command of the same key; `cancel_held` finding the entry at its new key; the owner's configure reading `updates` concurrently)? Is there any other path left where the projection holds a value from an instant the device cannot have it?
3. The new test and P425: does the test observe what it claims, and does the row break it?
4. The `design/18` text changed in this round.

Japanese. First line `判定：PASS` or `判定：FAIL`. Numbered findings with severity (major / minor), `path:line`, evidence, scenario, fix. Mark unverified points INFERRED.

## sol-rereview2-brief


You are the independent reviewer (AGENTS.md §9 step 8). Repository: /home/komatsu/local_work/Ez-SDRv4. **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

Your last re-review is the end of `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-rereview.log` (FAIL: moving a projection entry to `effective` at release cannot reach a configure that already read it; a later segment took the value too early from `config`). The orchestrator treated this as a third strike on one mechanism and removed the cause instead of patching (`fix2-brief.md`, then `fix3-brief.md` after its own review found an ordering hole in the implementer's round-2 additions). Accounts: `fix2-report.md`, `fix3-report.md` (do not trust them; check `git diff` against HEAD `cddad68`).

The rules now:
1. A timed command's instant is fixed at booking: `e = max(requested, now + device lead, latest released)`; an `at` the floor moves is `LATE_COMMAND` at booking. Release never moves it (`debug_assert_eq!(effective, e)`); no projection entry moves after booking (`ColdConfig::issued` is gone).
2. Release walks `held` in `(e, seq)` order; a command whose `targets` are empty stays held and stops the pass while `e > now`; once `e <= now` with none, it is recorded `no channel` and folded into the configuration of record. So the device receives commands in `e` order.
3. Released commands are kept in `released_at` and folded into the configuration of record only when their instant passes (`config_at`, `held_updates`), so a segment made later holds each at its instant.

Check:
1. Your two re-review majors: closed, with the sequences you gave.
2. Rule 1's claim that the booking floor is final (no later release can put a command with a later instant ahead of this one): any path that breaks it — a command waiting at the head (rule 2), `Provider::stop`, a refusal `return` mid-pass, a lost device, two directions.
3. Rule 2's cost: a waiting command holds back later ones until its instant; anything worse than a late arrival (a command never released, a held command counted twice or not at all in the queue accounting, a cancel missed)?
4. Rule 3: `config` versus `released_at` — any reader of `self.config` that now misses a released value not yet folded (UR-12 validation, `config_at`, layout `candidate`, `channels_at`, Manifest), and any value folded twice.
5. Tests' power for the new sequences (`ur_24_a_command_waiting_for_a_channel_*`, `ur_24_a_segment_*`, the S12 rewrite) and rows P426–P432.
6. `design/18` as changed (UR-24, UR-26, UR-30, UR-36, Changes): correct, DA-1 shaped.

Japanese. First line `判定：PASS` or `判定：FAIL`. Numbered findings with severity (major / minor), `path:line`, evidence, scenario, fix. Mark unverified points INFERRED.

## sol-rereview3-brief


You are the independent reviewer (AGENTS.md §9 step 8). Repository: /home/komatsu/local_work/Ez-SDRv4. **Read-only**: no edits, no builds into the tree, no `cargo fmt`, no mutation tools, no commit or push.

Your last re-review is the end of `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4d1c/sol-rereview2.log` (FAIL: major 1 the settlement order of a released command and one without a channel; major 2 no loss check between a release's channels; minor 3 a reason in UR-24). The fix brief is `fix4-brief.md`, the implementer's account `fix4-report.md` (do not trust it; check `git diff` against HEAD `cddad68`).

Check:
1. Your three findings: closed, with the sequences you gave (tests `ur_24_a_released_command_and_one_without_a_channel_settle_in_instant_order`, `ur_29_a_loss_published_between_a_release_s_channels_stops_the_rest`, `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`; rows P432–P435).
2. The fix's neighbours: a pass that returns early (refusal, loss) before the final settlement loop; a command given while the device is lost; any other writer of `self.config` or reader that misses `released_at`.
3. Anything else in the stage's diff you would still block on. If nothing is left that you would block on, say PASS.

Japanese. First line `判定：PASS` or `判定：FAIL`. Numbered findings with severity (major / minor), `path:line`, evidence, scenario, fix. Mark unverified points INFERRED.
