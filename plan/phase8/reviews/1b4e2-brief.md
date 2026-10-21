# 1b-4e-2：テスト計画の指示書（まだコードは書かない）

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `dde41e6`．tree はきれい）．この段階は **risky**（AGENTS.md §9：stream と stream の間で device の streamer の持ち主が変わる＝ownership と fake／hardware の境界）．**今回の仕事はテスト計画だけ**です．`crates/` も `design/` も `mutations.json` も変えない．計画を書いたら止まる．Sol が計画をレビューし，orchestrator が裁定してから実装の指示書を出します．

## 1. 目的

UHD Module（`crates/ezsdr-radio-uhd`）で，論理 channel を同じ方向の別の stream へ動かす（spec 07 RM-34）．今は `provider/control.rs:847-863` が `UR-25: ... does not move a channel between streams yet` で拒否している．MockRadio は 1b-3b から moves を持つ（`crates/ezsdr-mock-radio/src/lib.rs:1880-1965`，`timeline::Lines::book_move`・`refuse_move`）．UHD も同じ `Lines` を持つので，時刻（cut，release，origin，床）は共有の `Lines` が決める．この段階で新しく決まるのは **device の側**：どの thread がいつ，どの streamer を閉じ・開くか．

## 2. 読むもの

- `design/07-radio-model.md` の RM-25，RM-34（本文と `rm_34_*` のテスト表 420〜460 行あたり）．`design/18-uhd-radio.md` の UR-13，UR-24，UR-25，UR-28，UR-29，UR-33（FakeDevice の **streamers**：別の stream が持つ channel への open を `Attempting to reconnect ... port!` で拒否）．
- `plan/phase8/33-channels-and-streams.md` の D3，D4，D14，D15 と §5 の 1b-4 の行（とくに 1b-4d-1b の `busy`・`reading`・owner の担当）．
- `crates/ezsdr-radio-uhd/src/provider/control.rs` の `book_cold`（790 行から，from-none の経路），`core.rs` の `StreamState`（`set: DeviceSet`），`owner_step`，`owner_configure`，`open_streamer`，`rx.rs` の `begin`・`finish`，`tx.rs` の同じ所．`device.rs` の `Device` trait と FakeDevice の `rx_open`・`tx_open`（1121 行，1427 行）．`uhd.rs` の `rx_open`（909 行），`close_streams`（1085 行）．
- Mock の moves：`crates/ezsdr-mock-radio/src/lib.rs` の `ColdChange::Move` の経路，`crates/ezsdr-mock-radio/tests/mock_layout.rs` の move のテスト．
- VH-8 の harness：`crates/ezsdr-acceptance/tests/parity.rs`（1b-4e-1 で stream 0 と 1 を比べるようになった．`Started`，`planned`）．

## 3. orchestrator の調べ（VERIFIED／INFERRED）

- **H1**（VERIFIED，コード）：streamer は stream ごとに 1 本で，自分の stream の reopen は古いものを先に捨てる（`uhd.rs:917`，fake `device.rs:1130`）．別の stream の streamer が持つ channel を含む open は，fake が `Attempting to reconnect output/input port!` で拒否する（`device.rs:1135-1147`，`1435-1446`；UHD の `graph.cpp:179-197`）．
- **H2**（VERIFIED，コード）：受信の owner は，次の segment が無いと `idle` を立てるだけで streamer を閉じない（`rx.rs:287`，`finish` は clock と prune だけ）．送信も同じ（`tx.rs:220`）．`Device` には stream ごとの close が無い（`close_streams` は全部を捨てる，`Provider::stop` 用）．
- **H3**（INFERRED，**未テスト**）：だから **今の HEAD でも**，受信 channel c1 を stream 1 から外し（`radio.rx.c1.stream` を `-1`），後で stream 0 に入れると，stream 0 の reopen は stream 1 の残った streamer とぶつかり，fake が拒否して stream 0 が halt するはず．送信も同じ．moves はこの「外して入れる」を 1 回の操作にしたものなので，同じ衝突を必ず踏む．
- **H4**（VERIFIED，ハードウェア）：同じ stream の reopen（同じ front end を古い streamer を捨ててから新しい streamer へ）は X300 で動く（Phase 7 と 2026-10-20 の BP5）．**別の stream** の streamer へ同じ front end を移す場合も，古い streamer を捨てれば graph の接続が外れる，は INFERRED（UHD 4.x の streamer の破棄が graph の edge を外す．実機は bench で確かめる；今回は FakeDevice で固める）．
- **H5**（VERIFIED，コード）：uhd-control の from-none（channel の無い stream への最初の enable）は，**予約の時点で** uhd-control 自身が streamer を開いて設定する（`control.rs:925` から）．move の移動先が channel の無い stream なら，移動元がまだその channel を流している時点で開くことになり，H1 で拒否される．

## 4. orchestrator の決定（計画はこれを前提にする．反対なら計画の「異論」に理由を書く）

- **D1：持ち主が手放す**．streamer の device 呼び出しはその stream の owner thread がする（今の UR-25 の担当を変えない）．owner は自分の streamer が持つ channel を，**同じ方向の別の stream の line が，まだ始まっていない segment で必要としていて**，かつ自分の line がその channel を持つ segment をもう走らせない（今の segment の cut を過ぎた／idle）とき，手放す：次の segment があれば今の reopen（その channel 抜き）で，無ければ新しい `Device::rx_close(stream)`・`tx_close(stream)` で閉じる．必要が無いとき（Stop の後の idle など）は今どおり閉じない（Stop → start の遅れを変えない）．
- **D2：開く側が待つ**．streamer を開く thread（owner，または uhd-control）は，開く集合の channel を同じ方向の別の stream の device の記録（`StreamState.set`）が持っている間は開かず，host clock で待つ．loss，`Provider::stop`，自分の stream の Stop・拒否で待ちを抜ける（今の `await_reading` と同じ形）．待ちは owner の turn を止めるだけで，uhd-control を止めない．
- **D3：uhd-control の from-none は，入る channel を別の stream の記録が持っているときは使わない**．その segment は移動先の owner が自分の cut（origin の前）で開いて設定する（普通の変更と同じ経路．ready は予約）．uhd-control は待たない．
- **D4：`Device::rx_close`・`tx_close`**．`UhdDevice` はその stream の streamer を map から外して落とす（thread が Arc を持っていれば，それが手放したときに解放，`close_streams` と同じ）．fake は外して `calls` に `rx_close {stream}` を残す．streamer の無い stream への close は何もしない．
- **D5：拒否**．move の移動先または移動元の device が設定を拒否したら，その command の `seq` が move なら `Lines::refuse_move(seq)`（両方の line を halt），そうでなければ今の `refuse`．
- **D6：moves の受付**．`control.rs:847-863` の拒否を外し，`Lines::book_move` で予約する．床と `e` は今の `layout_instant`，`coerce` は今の経路．RM-34 の拒否（同じ stream，方向違い，移動元が持たない，移動先が持つ）は `book_move` の `Err` を `COMMAND_REJECTED` の `RM-34: ...` にする（Mock の理由の文と prefix を合わせる）．
- 範囲外：moves の Python の API，Session の verb，bench（実機）．

これより良い形（例えば，閉じるのを開く側がする）があると思ったら，計画の「異論」に，D1〜D3 と比べた利点と欠点を書く．計画の本体は D1〜D6 で書く．

## 5. 大事な系列（それぞれにテストを計画する）

受信を R，送信を T とする．c は論理 channel．FakeDevice の slot は 2 つまで（UR-33）：UBX なら各方向 2 channel．Basic/LF の盤は片方向だけで，受信の盤（selector の `lf_rx`・`basic_rx`，fake の `"LFRX"`・`"BasicRX"`）は 1 枚で受信 2 channel（2 枚で 4）．3 channel 以上の受信の系列は Basic/LF の盤で組み，送信の系列は 2 channel で組む．期待はすべて FakeDevice の上で，`calls` の順，event，plan の行で観測できる形にする．

- **S0（H3 の回帰．最初に書くテスト）**：R：s0 {c0}，s1 {c1}．c1 → `-1`（s1 は空になる），その後 c1 → s0．期待：s0 の reopen [0,1] が通る．`COMMAND_REJECTED`・device の失敗なし．s1 の streamer は閉じられている（`rx_close 1` が s0 の `rx_open 0 [..1..]` より前）．T でも同じ．**今の HEAD で落ちることを先に確かめる**（落ちなければ H3 は誤りで，計画にそう書く）．
- **S1**：R：s0 {c0,c1}，s1 {c2}，どちらも走っている．c1 を s1 へ move（e = 今＋100 ms）．期待：両方 e で cut．s0 は {c0}，s1 は {c1,c2}．移動先の origin は RM-34 の release ＋ 自分の受信 call・allowance・lead 以後．`calls` で s0 の reopen [0] が s1 の reopen [1,2]（または [2,1]：block の順は RM-27）より前．`reconnect` の拒否なし．
- **S2**：移動元の唯一の channel の move：s0 {c1}，s1 {c2}．期待：s0 は次の segment が無く，`rx_close 0` → s1 の reopen．s0 の line は以後 channel を持たない．
- **S3**：移動先が channel の無い stream（clock も streamer も無い）．期待：uhd-control は予約の時点で開かない（D3）．移動先の最初の segment の origin は RM-34 どおり．予約の直後に別の stream へ送った command が待たされずに受け付けられる（uhd-control が止まっていない）．
- **S4**：移動元が move の前に Stop されている（`rm_34_the_destination_waits_a_stopped_source_s_cut` の device 版）．期待：移動元は idle だが，必要が出たので閉じる（D1）．移動先はその後に開く．
- **S5**：T の move：s0 {c0}，s1 {c1}，c0 を s1 へ（s0 は空になる）．e の前に s0 へ burst，e の後に s1 へ burst．期待：前の burst は c0 で s0 から出る，後の burst は s1 で c0 と c1 を含む．送信 clock は両方の stream で e に予約される（`rm_34_a_move_books_both_transmit_clocks`）．`reconnect` なし．
- **S6**：移動先の設定を device が拒否（fake の fault：open か apply の失敗）．期待：`refuse_move` で両方の line が halt．`COMMAND_REJECTED` 1 件．移動元が今の segment を手放したなら，その状態が記録と一致する（記録が「開いている」と言うのに device に無い，を作らない）．
- **S7**：move の予約と e の間に loss，または `Provider::stop`．期待：誰も open せず，D2 の待ちが抜け，thread がすべて join される．
- **S8**：動かす channel への timed 更新（UR-24）：e の前の instant は移動元の streamer の channel に，e の後は移動先の channel に出る．
- **S9**：RM-34 の拒否 4 種（同じ stream，方向違いは key の形で起きないなら理由を書く，移動元が持たない，移動先が持つ）．期待：`RM-34:` で拒否，line も device も変わらない．Mock と同じ理由の文．
- **S10**：move の後，移動先へ release より前の `cold` 変更：`Lines` の床どおり（共有の規則なので UHD の側は 1 本で足りる）．
- **S11（VH-8）**：`parity.rs` に S1 と S3 の比較を足す（clock の行，plan の行，`sequence` は順位で）．Mock と違ったら，本体を直さず `#[ignore = "divergence: ..."]` にして報告．

各系列で，「間違った実装ならどう落ちるか」を 1 つ以上書く（例：D1 で閉じない → `reconnect` の拒否で移動先が halt；D2 で待たない → fake の `calls` で open が close より前；D3 を守らない → from-none の open が予約の時点で拒否される）．

## 6. 計画の形

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/test-plan.md`，日本語．最初の行は `STATUS: plan`．

1. S0 の実験の結果（今の HEAD で書いて流した**使い捨て**のテストの結果．tree の外の写しで流すか，流した後に消す．tree には残さない）．
2. 系列ごとの表：テストの名前（`ur_34_*` か `rm_34_*`，置き場所 `tests/fake.rs` か unit），手順，assert，観測のしかた（`calls` の順，event，plan の行，`StreamState.set`），それを落とす誤った実装．
3. D1〜D6 を実装に移すときに触る所（関数と `path:line`）と，増える状態・失敗の経路（それぞれのテストも）．
4. design の文で直すもの：UR-25（moves を実装した，持ち主の手放しと開く側の待ち），UR-33（`rx_close`・`tx_close`），RM-34 の `Test:` に足すテストの名前，plan 33．
5. 異論（あれば）．
6. 穴：計画で固められないもの．

## 7. 守ること

- `crates/`・`design/`・`plan/`・`mutations.json` を変えない（S0 の使い捨ては tree の外）．
- `cargo fmt`・`rustfmt` を流さない．`mutate.py`・`remote_mutate.py` を流さない．commit・push・`git add` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．背景の job を残さない．tree の写しで build するなら `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review` と `CARGO_INCREMENTAL=0`，写しは `rsync -a --no-times --exclude .cargo --exclude target`．
- 指示書に無い仕組みを足さない．足りないと思ったら「穴」か「異論」に書く．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していないことを報告に書く．
- セルフレビューは要らない（計画は Sol が見る）．

---

# 1b-4e-2：実装の指示書

あなたは実装役です．リポジトリは /home/komatsu/local_work/Ez-SDRv4（HEAD `dde41e6`，tree はきれい）．この段階は **risky**：UHD Module で論理 channel を同じ方向の別の stream へ動かす（spec 07 RM-34）．streamer の持ち主が stream から stream へ移る．

## 1. 読むもの（この順に）

作業 dir は `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/`．

1. `plan-brief.md`：orchestrator の調べ（H1〜H5）と決定（D1〜D6）と系列（S0〜S11）．
2. `test-plan.md`：DeepSeek のテスト計画．S0 の実験（今の HEAD で「channel を `-1` にしてから別の stream に入れる」が `reconnect` で落ちることを受信・送信・idle の移動先で確かめた）．
3. `sol-plan.md`：Sol の計画レビュー（FAIL，major 7）．
4. `adjudication.md`：**orchestrator の裁定（A1〜A10）．これが D1〜D6 と計画より優先する．**

AGENTS.md（とくに §6，§7，§9）と，`design/07-radio-model.md` の RM-25・RM-26・RM-34，`design/18-uhd-radio.md` の UR-13・UR-24・UR-25・UR-30・UR-33・UR-36 を読む．

## 2. やること

1. **計画を直す**：`test-plan.md` を裁定に合わせて改訂し，`test-plan-v2.md` に書く（元のファイルは残す）．裁定の A1〜A10 のそれぞれが計画のどこに入ったかを冒頭に表で書く．Sol の指摘の訂正（A7 など）はすべて反映する．
2. **実装する**：改訂した計画の順に．最初に S0 の 3 本（今の HEAD で落ちる回帰）を書いて落ちることを確かめ，それから D1〜D6 と A1〜A10 を入れて通す．
   - 共有の `ezsdr_radio::timeline`（A1）は UHD の変更より先に，自分のテストで固めてから．
   - `Device::rx_close`・`tx_close` は既定実装を置かない（実装漏れをコンパイラに見つけさせる）．
   - 実装で増えた状態や失敗の経路は，計画（`test-plan-v2.md`）に足してテストを書く（AGENTS.md §9 の 4）．
3. **design の文**：計画の §4 と裁定のとおり，`design/18-uhd-radio.md`（UR-25，UR-30，UR-33，UR-36，今の move の拒否の文と古いテスト名の置き換え，Changes の行），`design/07-radio-model.md`（RM-34 の per-stream prune，`*Test:*`），`plan/phase8/33-channels-and-streams.md`（1b-4e-2 の行）．accepted spec の書き方は AGENTS.md §2 の「Format of accepted specs」（規則は短い normative な文と `*Test:*` 行，理由と経緯は Changes へ）．`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` を通す．
4. **VH-8**：`crates/ezsdr-acceptance/tests/parity.rs` に S11 の 2 本（clock，plan の行，`sequence` は既存の順位の比べ方，`updates`）．Mock と違ったら本体を直さず `#[ignore = "divergence: ..."]` にして報告．
5. **mutation の行**：`plan/maintenance/tools/mutations.json` の `maintenance` に P489 から．新しい規則ごとに 1 行以上（close の判定，待ち，待ちの出口，読み直し，from-none の条件，move の予約，両側の clock の再計算，per-stream prune の境界，照合の失敗での close，`applied` の行）．`old` を変えた既存の行（計画 §3.7 が挙げる P252・P253・P385・P386・P278 ほか，grep で探す）は当て直す．当て直した行は `new` に古い識別子が残っていないかも grep する．ファイル全体を書き直さない．

## 3. 守ること

- `cargo fmt`・`rustfmt` を流さない（tree はわざと rustfmt に合わせていない．mutation の行が source の文字列と一致しなくなる）．新しいコードは周りの書き方に手で合わせる．
- mutation は `plan/maintenance/tools/remote/remote_mutate.py` だけで流す．`mutate.py` をローカルで流さない．main tree に mutant を当てない．
- commit・push・`git add` をしない．
- 1 本のテストや 1 つの test binary には `timeout 300` を付ける．背景の job を残さない．
- **指示書と裁定に無い仕組みを足さない**．足りないと思ったら，足さずに報告の「穴」に書く．
- テストは状態の到達で同期する（A6）．固定の sleep や狭い実時間の上限を正しさの根拠にしない．
- 1 段階の大きさの目安は本体とテストで約 1 500 行（owner）．大きく超えそうなら，どこで切るかを報告に書いてから続ける（止まらなくてよい）．テストの繰り返しは helper にする．
- Python は `PYTHONDONTWRITEBYTECODE=1`．build は tree の外（main tree は `.cargo/config.toml` で `~/.cache/cargo-target/Ez-SDRv4` に出る）．
- 前の段階で出た誤り：到達できる系列を「到達できない」と言って消す．していない実装を報告に書く．記録を手で書き換えて，実際の経路を通さずに確かめる．

## 4. 確かめること

1. `cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`．新しい `ur_34_*` と `parity` は 3 回続けて流し，揺れないことを見る．
2. `rules.py --base HEAD`．
3. UHD の feature の build：`R="docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10"; $R cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings; $R cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`（`UhdDevice` の close を足すので必要）．
4. mutation：新しい行と当て直した行を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:3,sim03:3 <IDs>`（`phase7` の行なら list 名を合わせる．複数の list は `maintenance,phase7`）．COMPILE ERROR も生き残りとして扱う．`BASELINE FAILED` が出たらその行だけ流し直して報告に書く．tree の新しいファイルは mode 644 にしてから流す．

## 5. セルフレビュー

reviewer は Luna（あなたと同じ model）．回ごとに新しく起動する：

```
cd /home/komatsu/local_work/Ez-SDRv4 && timeout 2400 opencode2 run --standalone -m "openai/gpt-6-luna#max" --agent plan --auto "$(cat ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/checklist.md)" > ~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/review-<n>.log 2>&1
```

`checklist.md` は orchestrator が置いた（読むだけ）．P1 が無くなるか 3 回まで．**P1 が無くなったら次の回は流さない**．指摘はすべて直すか，理由をつけて退ける．gate（全 mutation，Python）は orchestrator がするので指摘にしない．

## 6. 報告

`~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/report.md`，日本語．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．中身：

1. 裁定 A1〜A10 と D1〜D6 のそれぞれをどう実装したか（`path:line`）．
2. テストの一覧（系列との対応，`path:line`）．S0 の 3 本が実装の前に落ちたこと．
3. 「違い」（VH-8 の divergence）．
4. 確かめることの結果（mutation は行ごと）．
5. セルフレビューの指摘と扱い．
6. 本体とテストの行数．
7. 穴．

**起きたことを書く．していないことは書かない．**

---

# 1b-4e-2 修正の回 1（実装役：Luna）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の tree（1b-4e-2 の実装，あなたが書いたもの）を直します．作業 dir は `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e2/`．Sol のコードレビュー `sol-1.log` の末尾（「判定：FAIL」から下）と orchestrator の下書き `opus-notes.md` を読む．以下が orchestrator の裁定で，採ったものだけを直す．

## 直すもの

1. **Sol 1（major）：連鎖 move の per-stream prune が片側だけを畳む**（`crates/ezsdr-radio/src/timeline.rs` の `prune_requested`）．相手の「要求した」境界ではなく，相手の**実効の**境界で止める．実効の境界は，どの stream の境界も下がらなくなるまで繰り返して求める（不動点）．floor の固定と move の記録を落とすのは，両方の line が実際にその move の item を畳むときだけ．テスト（`crates/ezsdr-radio/tests/timeline.rs`）：Sol の系列（c0 を A→B `e=10`，B→C `e=20`，B と C だけを 30 で prune → B→C の item と記録が両方に残り，その後の拒否で C が halt）と，prune の順を変えたもの．どれも prune しない `Lines` との plan の比較で．`Line::released` の `expect` に届かないことも．mutation の行を 1 つ（不動点の繰り返しを 1 回にする，など）．
2. **Sol 2（major）：from-none が，まだ streamer を取っていない先行の未処理 segment を追い越す**（`control.rs` の `from_none` の条件）．他の stream の未処理の segment がその channel を要るとき（既存の `needed_before`／`needed_elsewhere` で判定，新しい待ちの仕組みは足さない）は from-none にせず owner に任せる．テスト：A の解放の後，B の open の前で probe で止め，その間に空の C へ join → uhd-control は device を呼ばず，B が先に開き，C は B の後に開く．mutation の行を 1 つ．
3. **Sol 3（major）：P358 と P493 は equivalent ではない**．Sol の probe：拒否より前に後続の Cold が予約されていると，拒否された TX segment は正の長さを持つ（`origin=100, cut=100 samples`）．TX の End の後も，後続の Cold があれば同じ segment が plan に残る．両行の `equivalent` を外し，その到達できる系列のテストを書いて，行の `filter` をそのテストに当てる．
4. **Sol 4（minor）：P494 の guard は到達しない**．RX の cut-at-origin の segment は planner が plan から除く（`timeline.rs:947-952`）．`await_release` の RX の cut-at-origin の条件（`core.rs` の `dir == Dir::Rx && current.segment.cut...`）を消し，P494 の行を消す．
5. **Sol 5（minor）：S8 のテストが projection と実効値を見ていない**（`tests/fake.rs:6004-6034`）．移動の後の configure が直前の timed の値を引き継ぐこと，後の timed update の前後の実効値を足す．
6. **Opus（minor）：`needed_elsewhere` は `needed_before(stream, set, i64::MAX)` と同じ**（`core.rs:376-409`）．1 本にする（`needed_elsewhere` を消して呼び出し側で `i64::MAX` を渡すか，`needed_elsewhere` を 1 行の委譲にする）．挙動は変えない．当て直す行があれば当て直す．

採らないもの（直さない）：TX の Stop を line に予約すること，UR-25 の文の長さ（orchestrator が gate で見る）．

## design の文

1 と 2 で文が偽になるところだけ直す（`design/07-radio-model.md` RM-34 の per-stream prune，`design/18-uhd-radio.md` UR-25 の from-none）．新しいテスト名を `*Test:*` に足す．`rules.py --base HEAD` を通す（renames は今の `rules-renames.txt`）．

## 守ること

指示書 `impl-brief.md` §3 と同じ（`cargo fmt`・`rustfmt` 禁止，mutation は `remote_mutate.py` だけ，commit・push・`git add` 禁止，`timeout 300`，指示に無い仕組みを足さない，状態の到達で同期する）．tree の新しいファイルは mode 644．

## 確かめること

1. 変えた crate のテスト（`cargo test -p ezsdr-radio`，`-p ezsdr-radio-uhd`，`-p ezsdr-acceptance --test parity`）と `cargo clippy --workspace --all-targets -- -D warnings`．新しいテストは 3 回続けて流す．全体の `cargo test --workspace` は orchestrator の gate でする．
2. `rules.py --base HEAD`．
3. mutation：新しい行，当て直した行，P358，P493，それと timeline の move の行（P102，P103，P108 など `prune` を old に持つ行を grep）を `REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:3,sim03:3 <IDs>`．全行の `old` が source にちょうど 1 回一致することも確かめる．

セルフレビューは流さない（orchestrator と Sol が見る）．

## 報告

`fix1-report.md`（日本語）．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．指摘ごとにどう直したか（`path:line`），足したテスト，mutation の結果（行ごと），確かめることの結果，穴．OpenAI の利用上限に当たったら `STATUS: blocked: OpenAI usage limit` にして，そこまでの事実を書いて終える．**起きたことを書く．していないことは書かない．**
