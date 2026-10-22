# 1b-6a：実装の指示書（UHD Module：device を呼ぶ者と進行中の読みを遷移の関数だけが変える）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `8cd89e4`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-22-phase8-stream-state/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

設計メモ `plan/phase8/35-uhd-stream-state.md` を最初に全部読むこと．この段階はその **R1** だけ：`crates/ezsdr-radio-uhd/src/provider/core.rs` の `StreamState` の `busy`・`busy_seq`・`reading` を private な `Claims { holder: Holder, reading: bool }` にし，`Streams` の遷移の関数（メモ §2 R1 の表）だけが変える．`idle`・`active`・`delivered`（R2）と device の記録，loss は変えない．

この段階は **risky**（AGENTS.md §9：所有権と lifecycle）．**今回はテスト計画だけを書いて止まる**（§5）．コードは書かない．

## 2. やること（実装の段階で．今回は計画に書くだけ）

1. `core.rs`：`Claims`，`Holder { Free, Owner { seq }, Release, FromNone }`，`Claim`（取った stream と役割を持つ，`#[must_use]`），メモの表の関数（`claim_owner`，`claim_release`，`claim_from_none`，`may_call_from_none`，`begin_read`，`begin_shutdown_read`，`end_read`，`give_back`）．`move_source_busy` は `Owner{seq}` を読む．`Claim` の Drop は **test build でだけ** panic（返されずに落ちた），production では何もしない．lock を取り直さない．
2. `owner_configure` は失敗のとき `Err((error, claim))` を返し，呼び出し側（`Rx::begin`，`Tx::do_switch`）が同じ lock の節で拒否を book してから `give_back` する．今 `busy = false` を書いている 9 か所（core 5，rx 1，tx 1，control 2）と `reading` を書く所（tx 4 か所）を関数の呼び出しにする．field は private にして，`StreamState` の外から直接書けないようにする．
3. spec 18 の UR-25 の「Ownership」の段落を，holder と読みの遷移の言葉に書き直す（規則は変えない）．DA-1 の形式（AGENTS.md §2）．
4. テストのうち `.busy`・`.busy_seq`・`.reading` を直接読み書きする所を，関数か観測で置き換える．

## 3. 系列と期待（メモ §4 の R1．どれも Module の中の挙動で VERIFIED：HEAD のコードと spec 18 UR-25）

| # | 系列 | 期待 |
|---|---|---|
| S1 | owner が `Free` の stream を取り，configure に成功して返す | 返した後 `Free`．device の記録は configured |
| S2 | owner の configure が拒否される | 呼び出し側が拒否を book した後に `Free`．book の前に他の thread が `Free` を見ることはない |
| S2m | move の source の configure・reopen が拒否される | 両方の line に拒否が載るまで source は `Owner{seq}` のまま．destination は開かない（`move_source_busy`） |
| S3 | owner が `FromNone` の stream を見る | device を呼ばず，cursor・switch・idle の印・device に触らない |
| S4a | uhd-tx の読みの最中に from-none が `FromNone` を取る | 読みが終わるまで device を呼ばない．終わってから open する |
| S4b | from-none が先に `FromNone` を取り，その後 uhd-tx が読もうとする | uhd-tx は読まない（`begin_read` が false） |
| S5 | from-none が読みを待つ間に `Provider::stop` か loss | 何も book せずに `holder` を返す．`reading` は uhd-tx の `end_read` まで残る |
| S6 | `FromNone` のまま uhd-control が止まっている間に `shutdown` の読み | 読みは始まり，終わる．uhd-control が読みの最中に `give_back` する順と，`end_read` が先の順の両方で，最後に両方が解ける |
| S7 | `release_unneeded` | `Free` で idle の stream だけを `Release` で取って閉じ，返す．他の holder の stream は閉じない |
| S8 | 返し忘れ | 成功，拒否，loss，stop，待ちの中断の各道で `Claim` が `give_back` される．test build の Drop の panic が見張る |

## 4. テスト計画の書き方

`test-plan.md` に，系列ごとに次を書く．

- 今あるテストでその系列を見ているもの（名前と，何を assert しているか）．候補：`ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking`，`ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path`，`ur_25_a_from_none_enable_waits_for_a_transmit_report_read`，`ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop`，`ur_25_a_detached_control_does_not_hold_the_shutdown_s_read`，`ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`，`ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`，`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`，`ur_34_a_move_*_refusal_halts_both_lines_and_recovers`，`ur_34_a_move_destination_waits_for_source_busy_after_channel_release`，`ur_34_a_release_*`（`core.rs` の unit test）．自分で grep して足す．
- 足りない系列に足す新しいテスト：thread を使わない `Streams` の unit test（遷移の表の各行と「前が違うとき」）を基本にし，thread の順序が要るもの（S4a，S5，S6）は既存の probe（`core.probe`）と manual clock の形に合わせる．
- 各テストが何を assert し，どうやって観測するか（印を直接読まずに観測できるなら観測で．遷移の unit test は関数の戻り値と次の関数の戻り値で），どんな誤った実装で落ちるか．
- 印を直接読み書きしている今のテスト（`grep -rnE "\.(busy|busy_seq|reading)\b" crates/ezsdr-radio-uhd`）の一覧と，それぞれの置き換え方．
- mutation の行は計画に書かない（コードと一緒に足す）．ただし今の行のうち `busy`・`busy_seq`・`reading` を `old` か `new` に持つ行を grep して一覧にし，それぞれどう当て直すかを書く．

## 5. 守ること

- **今回はコードを書かない．`test-plan.md` と `report.md` だけを書く．** `report.md` の最初の行は `STATUS: plan`．
- `cargo fmt`・`rustfmt` を流さない．mutation は `remote_mutate.py` だけで流す（今回は流さない）．`mutate.py` をローカルで流さない．commit・push・`git add`・`git stash` をしない．
- **指示書と設計メモに無い仕組みを足さない．** production に `#[cfg(test)]` だけの道を作らない（`Claim` の Drop の panic は設計メモが決めた例外）．
- 前の段階で出た誤り：到達できる系列のテストを代わりなしで消す，differential の操作を縮める，照合が空でも通るテスト，していないことを報告に書く．
- **起きたことを書く．していないことは書かない．**

---

## 1b-6a：計画の修正と実装の指示書

作業 dir `~/.cache/ezsdr-fixes/2026-10-22-phase8-stream-state/`．最初の指示書 `impl-brief.md` の §1・§2・§3・§5 の「守ること」はそのまま効く（ただし今回はコードを書く）．**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig`，`.*.snap`）は読むだけ．見張りのプロセスを止めない．`git stash`・commit・push・`git add`・`cargo fmt`・`rustfmt` をしない．指示書と設計メモに無い仕組みを足さない．**

## 1. 計画の修正（Sol の計画レビュー `sol-plan.log` の最後．orchestrator はすべて採用した）

`test-plan.md` を直してからコードを書く．直した所は `test-plan.md` の末尾に「## 修正（Sol の計画レビュー）」として一覧にする．

1. **S4a（major）**：読みの gate を持ったまま，uhd-control が `await_reading` の**ループの中の判定**（`may_call_from_none` が false を返した所）に一度来たことを同期して観測し，その時点で open していないことを確かめてから読みを解放する．`wait_reading` probe（ループの前）だけで観測しない．判定の後に probe が要るなら `core.probe` を 1 つ足してよい（test build でだけ働く既存の仕組み）．
2. **S2（major）**：RX と TX の両方の呼び出し側（`Rx::begin`，`Tx::do_switch`）で，拒否の book に入る時点で stream がまだ `Owner { seq }` を持つことを直接観測する（probe か，book の節の中で holder を読む）．move の source なら両方の line に拒否が載った同じ lock の節で返すこと．lock の回数は補助．
3. **release の後の loss（major）**：`release_unneeded` で claim を取った後，次の loss の検査の前に同期点（probe）を置き，そこで loss を publish して再開し，`holder` が `Free`，close が呼ばれない，Drop の panic が無いことを確かめるテストを足す．
4. **遷移表の抜け（minor）**：`may_call_from_none` を `Free`・`Owner`・`Release` で，`FromNone` の二重取得，`idle` で `active` が無い `Owner` の stream への `claim_release` の拒否（拒否の理由を 1 つにする）．
5. **N8（minor）**：本物の `Claim` を外で持ち，誤った返しだけを `catch_unwind` で捕まえ，panic と holder が変わらないことを確かめ，最後に本物を正しく返す．
6. **N9（minor）**：受信の state を 2 つ用意し，source と destination の line を入れる．同じ source で Claim を返して取り直し，`Some(7)`・`Some(8)`・`None` を比べる．private な field を直接変えない．
7. mutation：P387・P388・P396・P402 も追跡する．P384 は「enable はするが所有を取らない」の狙いのまま当て直す．P398 は新しい `begin_read` の guard があっても元の誤りを再現できるか確かめ，できなければ報告に書く（観測を弱めて生き残りを隠さない）．

未決の A〜G は次のとおり決めた：A 状態を変えない（test build は panic）．B accessor `holder(stream)` を足す（production の guard でも使ってよい），`reading(stream)` は production が要らなければ test 専用でよい．C P385 は残して当て直す．D 上の 3．E 上の 2．F `begin_read` が false なら読みを skip，成功した読みだけ `end_read` と対にする．G `Tx::pass` は `begin_read` を plan を変える前の同じ lock の節で判定し，失敗なら device を呼ばず，owner の状態も動かさずに false．

## 2. 実装

`impl-brief.md` §2 のとおり．計画のテストをすべて書く．`design/18-uhd-radio.md` の UR-25 の Ownership の段落と `*Test:*` 行，Changes の 1 行を直す（DA-1 の形式）．

## 3. 確かめること

1. `timeout 300` を付けて：`cargo test -p ezsdr-radio-uhd`，`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` の file を作業 dir に置く）．
4. 新しいテストを，同じ binary で `--test-threads=8` 以上で 20 回続けて流し，1 回も落ちないこと（決定性）．
5. mutation：当て直した行，足した行，`new` に消した名前を持つ行と `filter` に消したテストを持つ行（grep する）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>` で流す．全行の `old` が source にちょうど 1 回一致することも確かめる．全リストは orchestrator が流す．

## 4. セルフレビュー

reviewer は実装役と同じモデル．**同時に 1 本だけ**：自分のモデルの呼び出しが無い間（shell で reviewer を待つ間）に流す．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-22-phase8-stream-state/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-22-phase8-stream-state/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．利用の上限で止まったら（OpenCode の free の rate limit など），そこまでの事実を書いて `STATUS: blocked: limit` で終える．

## 5. 報告

`report.md` の末尾に「## 実装」を足す．最初の行を `STATUS: done`（か `STATUS: blocked: <理由>`）に変える．中身：計画の修正の一覧，変えた本体（`path` と行数），テストの一覧（新しい・変えた），mutation の行（当て直し・足した・grep の結果・流した結果），spec 18 の直した所，確かめることの結果（決定性の 20 回を含む），セルフレビューの指摘と扱い，穴．**起きたことを書く．していないことは書かない．**
