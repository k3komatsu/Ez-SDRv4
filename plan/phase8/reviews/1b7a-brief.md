# 1b-7a：実装の指示書（held burst の取り消しの報告を語彙の 1 か所で決める：#51）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `1886528`，tree はきれい）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b7a/`．

**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig` など）は読むだけ．書き換えない．orchestrator が動かしている見張りのプロセス（`watchdog_proc.py`，`mutation_watchdog.py`）は止めない．自分で起こした背景の job だけを片付ける．`git stash` を使わない．**

## 1. 背景

設計メモ `plan/phase8/36-radio-report-rules.md` を最初に全部読むこと．この段階はその **決定 1・2**（1b-7a）だけ：held burst が外の出来事（`Stop`，`Provider::stop`，loss，cold change）で取り消されたときに `radio.COMMAND_REJECTED` を出すか記録だけにするかを，`ezsdr_radio` の取り消しの理由の enum だけが決める．D2（`RX_OVERFLOW`，決定 4・5）は次の段階 1b-7b で，この段階では触らない．

変わる挙動は 1 つだけ：**UHD Module の `Stop`（`<id>/tx/s<k>` か `<id>`）で取り消した held burst は，`COMMAND_REJECTED` を出さず記録だけにする**（今の MockRadio と同じ）．他の道（`Provider::stop`，loss，cold change，未開始の clock を cold change が飛ばした場合）の報告の有無は両 Provider とも今のまま．reason の文字列も今のまま（各 Provider の規則 ID を指す）．

この段階は **risky**（AGENTS.md §9：Mock と実機の境の報告を変え，`COMMAND_REJECTED` の数は Policy の `stop`／`abort` を通じて Run の終わり方を変えうる：RS-26）．**今回はテスト計画だけを書いて止まる**（§5）．コードは書かない．

## 2. やること（実装の段階で．今回は計画に書くだけ）

1. `crates/ezsdr-radio/src/lib.rs`（既存の `kinds`・`payloads` の近く）に取り消しの理由の enum（例：`Cancel { Stop, ProviderStop, Loss, ColdChange }`．名前は計画で決める）と，報告するかを返す関数 1 つ．`Stop`・`ProviderStop`・`Loss` は記録だけ，`ColdChange` は報告．それ以外の仕組み（reason の文字列の共有，trait，Kernel の型）は足さない．
2. MockRadio（`crates/ezsdr-mock-radio/src/lib.rs`）：`stop_tx(.., emit_command_rejected: bool)`（`:1112`）と `cancel_later(.., emit: bool)`（`:1357`）の bool をこの enum に置き換え，呼び出し側（`:1142`，`:1145`，`:1458-1459`，`:1938`，`:2068` の loss，`:2470`）は理由だけを渡す．cold change が未開始の clock を飛ばす所（`:1342-1347`）も同じ enum を通す．
3. UHD Module（`crates/ezsdr-radio-uhd/src/provider/tx.rs`）：`stop_now(.., emit: bool)`（`:686`）を同じ enum に置き換え（`TxCmd::Stop` は `Stop`，`shutdown` は `ProviderStop`），`do_switch` の held burst の取り消し（`:707-711`）と未開始の clock を飛ばす所（`:242-252`）も enum を通す．
4. spec：`design/07-radio-model.md` RM-16 に規則を 1 文（DA-1 の形式，AGENTS.md §2）．`design/18-uhd-radio.md` UR-26（と，`Stop` の取り消しを報告と書いている所があれば UR-23）を「記録だけ」に．`design/09-mock-radio.md` MR-25 が今の挙動を正しく書いているか確かめる．各 `*Test:*` 行と Changes の 1 行．
5. テスト：`parity.rs`（`crates/ezsdr-acceptance/tests/parity.rs`）に両 Provider を突き合わせるテスト，enum の unit test，UHD で `Stop` の `COMMAND_REJECTED` を前提にしている既存テスト（`crates/ezsdr-radio-uhd/src/provider/tx.rs:1018` 付近，`crates/ezsdr-radio-uhd/tests/fake.rs:6225`・`:6263` 付近など．自分で grep する）の観測を `rejected` の行に移す．

## 3. 系列と期待（メモ §4．どれも Module の中の挙動で VERIFIED：HEAD のコード，メモ §1 の表）

| # | 系列 | 期待 |
|---|---|---|
| T1 | 送信の held burst を残して `<id>/tx/s0` に `Stop` | 両 Provider とも `COMMAND_REJECTED` なし，`rejected` に取り消した burst ごとに 1 行 |
| T2 | 同じで device（`<id>`）の `Stop` | T1 と同じ |
| T3 | 同じで `Provider::stop` | T1 と同じ |
| T4 | held burst を残して cold change | 両方とも取り消した burst ごとに `COMMAND_REJECTED` 1 個（今のまま） |
| T5 | 理由の enum の表 | 4 つの理由それぞれの報告の有無（unit） |
| T13 | loss で held burst が消える | 両方とも `COMMAND_REJECTED` なし，`rejected` に記録（UHD の記録の reason は今のまま） |
| T14 | 未開始の将来の clock の上の burst の取り消し | cold change がその clock を飛ばしたなら両方とも `COMMAND_REJECTED`（今のまま），`Stop` なら両方とも記録だけ |
| T15 | RX の `Stop` の後と TX の `Stop` の後 | RX の `Stop` は TX の予約を消さない．TX の `Stop` の後の新しい burst は送られる |
| T16 | 取り消した burst | 送られず，予約も残らない（数の一致だけでなく，送信の記録と held の空で見る） |

## 4. テスト計画の書き方

`test-plan.md` に，系列ごとに次を書く．

- 今あるテストでその系列を見ているもの（名前と，何を assert しているか）．自分で grep する（`cancelled by`，`COMMAND_REJECTED`，`rejected`，`held`）．
- 足りない系列に足す新しいテスト．T1〜T4・T13〜T14 は `parity.rs` で両 Provider に同じ操作をして比べるのを基本にする（数，`kind`，送信の記録）．parity でできないもの（UHD の loss の道など）は各 crate のテストで，理由を書く．
- 各テストが何を assert し，どうやって観測するか，どんな誤った実装で落ちるか（例：UHD の `Stop` を `ColdChange` で渡す，enum の表で `Stop` を報告にする，取り消した burst が送られる）．
- UHD の `Stop` の `COMMAND_REJECTED` を前提にしている既存テストの一覧と，それぞれの観測の置き換え方（消さない．`rejected` の行に移す）．
- mutation の行は計画に書かない（コードと一緒に足す）．ただし今の行のうち `stop_now`，`stop_tx`，`cancel_later`，`emit`，`emit_command_rejected`，`command_rejected`，`reject_action_at`，`record_rejected_action` を `old` か `new` に持つ行を grep して一覧にし，それぞれどう当て直すかを書く（`plan/maintenance/tools/mutations.json` と `plan/phase*/tools/mutations.json`）．

## 5. 守ること

- **今回はコードを書かない．`test-plan.md` と `report.md` だけを書く．** `report.md` の最初の行は `STATUS: plan`．
- `cargo fmt`・`rustfmt` を流さない．mutation は流さない．`mutate.py` をローカルで流さない．commit・push・`git add`・`git stash` をしない．
- **指示書と設計メモに無い仕組みを足さない．** production に `#[cfg(test)]` だけの道を作らない．
- 前の段階で出た誤り：テストが誤った実装を見抜けない観測（数だけ見て送信の記録を見ない），照合が空でも通るテスト，到達できる系列のテストを代わりなしで消す，differential の操作を縮める，していないことを報告に書く．
- **起きたことを書く．していないことは書かない．**

# 1b-7a：計画の修正と実装の指示書

作業 dir `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b7a/`．最初の指示書 `impl-brief.md` の §1・§2・§3・§5 の「守ること」はそのまま効く（ただし今回はコードを書く）．**作業 dir の orchestrator のファイル（`*-brief.md`，`checklist.md`，`.*.orig`）は読むだけ．見張りのプロセスを止めない．`git stash`・commit・push・`git add`・`cargo fmt`・`rustfmt` をしない．指示書と設計メモに無い仕組みを足さない．**

## 1. 計画の修正（Sol の計画レビュー `sol-plan.log` の最後．orchestrator はすべて採用した）

**`test-plan.md` を直してからコードを書く**．直した所は `test-plan.md` の末尾に「## 修正（Sol の計画レビュー）」として一覧にする．

1. **cold change の予約の解除（major）**：今の clock の cold change（T4）と，飛ばした将来の clock（T14a）の unit test に，取り消した burst が `held`／`later` から消え，共有の予約（`core.held`．`forget` が消すもの，`tx.rs:251, 258, 709`）からも消えたことの assert を足す．loss の後の `shutdown` の取り消し（T13）も同じ観測をする．Mock 側は，その burst の予約（`held`・`later`）が消えたことを同じく見る．
2. **TX の stream どうしの独立（major）**：両 Provider で s0 と s1 に将来の burst を持たせ，s1 だけに `Stop`．s1 は取り消しの行が 1 行で event なし，s0 は取り消されず実際に送られる（送信の記録で見る），を比べる parity test を足す（UHD の振り分けは `control.rs:354-372`）．
3. **T14a の説明の誤り（minor）**：`report.md` と `test-plan.md` の「UHD は飛ばした clock の burst を報告も記録もせず消す」は誤り．`tx.rs:251-252` は `forget` の直後に `command_rejected`（`"UR-25: its transmit clock was never started"`）を呼ぶ．T14a は今の挙動の回帰テストで，その reason はそのまま残す．「実装前には必ず失敗する」の説明も消す．
4. **enum を通らない道（minor）**：held burst の外の出来事による取り消しの箇所をすべて列挙し（Mock `lib.rs:1112` の `stop_tx`，`:1357` の `cancel_later`，`:1347` の cold change の直接の報告，`:1938` など；UHD `tx.rs:686` の `stop_now`，`:252` の `adopt_later`，`:710` の `do_switch` など．自分で grep する），どの箇所も理由の enum を渡すだけで，報告するかを決める分岐や `command_rejected`／`reject_action_at` の直接の呼び出しが残っていないことを確かめる．その一覧と結果を `report.md` に書く．受け取った時の拒否と専用の event を持つ drop（late policy，underflow）は範囲外（メモ 36 決定 1）．
5. 新しいテストは固定の sleep を判定に使わない．booking の完了と取り消しの完了を観測してから assert する．

## 2. 実装

`impl-brief.md` §2 のとおり．計画のテストをすべて書く．spec：RM-16 の 1 文，UR-26（と必要なら UR-23），MR-25 の確認，各 `*Test:*` 行と Changes の 1 行（DA-1 の形式）．

## 3. 確かめること

1. `timeout 300` を付けて：`cargo test -p ezsdr-radio -p ezsdr-mock-radio -p ezsdr-radio-uhd -p ezsdr-acceptance`，`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．
2. docker の uhd：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`．
3. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`（テストの名前を変えたら `--renames` の file を作業 dir に置く）．
4. 新しい parity test と変えた UHD のテストを，同じ binary で `--test-threads=8` 以上で 20 回続けて流し，1 回も落ちないこと（決定性）．
5. mutation：当て直した行，足した行，`new` に消した名前を持つ行と `filter` に消したテストを持つ行（grep する）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py <lists> sim02:3,sim03:3 <IDs>` で流す．enum の表の各行（`Stop` を報告にする，`ColdChange` を記録だけにする）と，各呼び出し箇所の理由（UHD の `Stop` を `ColdChange` に，など）を変える行を足す．全行の `old` が source にちょうど 1 回一致することも確かめる．全リストは orchestrator が流す．

## 4. セルフレビュー

reviewer は Go の DeepSeek（owner 2026-10-22）．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "opencode-go/deepseek-v4.1-flash#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b7a/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-22-phase8-1b7a/review-<n>.log 2>&1
```

P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．利用の上限で止まったら，そこまでの事実を書いて `STATUS: blocked: limit` で終える．

## 5. 報告

`report.md` の末尾に「## 実装」を足す．最初の行を `STATUS: done`（か `STATUS: blocked: <理由>`）に変える．中身：計画の修正の一覧，変えた本体（`path` と行数），§1 の 4 の取り消しの箇所の一覧と確かめた結果，テストの一覧（新しい・変えた），mutation の行（当て直し・足した・grep の結果・流した結果），spec の直した所，確かめることの結果（決定性の 20 回を含む），セルフレビューの指摘と扱い，穴．**起きたことを書く．していないことは書かない．**

# 1b-7a：セルフレビューの checklist（Go DeepSeek）

You are a read-only reviewer of an uncommitted change in /home/komatsu/local_work/Ez-SDRv4 (Phase 8 stage 1b-7a: a held burst cancelled by an outside event — Stop, Provider::stop, loss, cold change — is reported with radio.COMMAND_REJECTED or only recorded as one `ezsdr_radio` enum alone decides; the UHD Module's Stop no longer reports; everything else keeps its current reporting). Read-only: do not edit files; do not run cargo, python, mutate.py or remote_mutate.py. Read `git diff HEAD`, the design note `plan/phase8/36-radio-report-rules.md` (decisions 1 and 2) and the test plan `~/.cache/ezsdr-fixes/2026-10-22-phase8-1b7a/test-plan.md`.

Check only these four, and report each finding as P1 (wrong behaviour, a planned sequence with no test, or a test that cannot fail on the bug it names), P2 or P3, with `path:line`:

1. Every planned test exists and observes what the plan says (events, `rejected` rows, transmissions, the held/later/shared reservations); it would fail if the guard it pins were removed (no empty captures, no tautologies, no sleeps as the oracle).
2. Every cancellation site in MockRadio and the UHD Module passes the enum and nothing else decides reporting there (grep `command_rejected`, `reject_action_at`, `record_rejected_action`, `reject_note` near cancellations); each site passes the right cause; a Stop of one transmit stream cancels only that stream's bursts.
3. Nothing outside decisions 1–2 changed (no D2 / RX_OVERFLOW change, no reason-string change, no new mechanism, no `#[cfg(test)]`-only production path).
4. RM-16, UR-26 (UR-23), MR-25 say what the code now does, in the accepted-spec format; every `*Test:*` name exists; mutation rows whose `new` names a removed item or whose `filter` names a removed test were found and fixed.

If you find nothing of P1, say 「P1 なし」. Write in Japanese. The orchestrator's gate (full mutation lists) is not a finding.
