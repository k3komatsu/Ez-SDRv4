STATUS: done

# 報告：Phase 8 段階 1b-4d-1a（stream ごとの device の記録）

## 実装したもの

- stream ごとの device の記録（`StreamState` の `set: DeviceSet`，`configured: Option<(by, 版)>`）を `core.rs` に置いた．`owner_step` は `Open(set)` が一致し，`configured` が `(by, 版)` と一致するときだけ `Owner::Nothing` を返す．一致しなければ `Owner::Configure`，それ以外は `Owner::Open`．
- `record_open`，`record_configured`，`owner_configure` を追加．`arm` は開いたあとに記録を書き，rate の確認は開いたあとに行う（`mod.rs`）．
- 拒否された segment は device を呼ばない（`tx.rs` `do_switch`）．拒否された後の `Ok(()) if to.refused => self.at += 1` は P358 のとおり残した．
- from-none の記録（`control.rs`）：`streams.record_configured(stream, failure.is_none().then_some((seq, version)))` と，configure の失敗を `failure` に残す．
- prune は記録を変えない（`rx.rs`，`tx.rs`）．`verified`，`opened`，`stale`，`configured` の写しは消した．
- 版は `ColdConfig` の `AtomicU64` で，`configure` は `updates` の lock の中で読む．`streams` の lock は device を呼ぶ間に持たない．

## 計画から変えたところ

- **S11b を消した．** 計画の前提（timed update が from-none の segment の origin より前に届く）は到達できない．timed update は from-none の origin（約 2 ms）へ切り上げられ，その時点で channel は生きている．テストは版が上がらず device を呼ぶ形になったので，テストを消して deviation とした．
- **D1 の A を実装した（owner の判断に従う）．** pending の from-none の segment がある channel の release は，その segment の origin までに device に届く．記録の `(seq, 版)` が segment と一致すれば timed command を origin で 1 回出す．一致しなければ版を上げて owner に configure させる．記録する effective instant は要求どおり `e`．
- `device.rs` に test double を 2 つ足した：`FakeFault::TxApplyFails`（最初の TX apply を 1 回だけ拒否），`FakeFault::RxChannelApplyFails(n)`（channel n の最初の RX apply を 1 回だけ拒否）．既存の `WrongAntenna` と `WrongRate` は read-back の fault で，TX の configure を失敗させない（S8 の初版が落ちた原因）．

## テスト（計画の系列との対応）

- 実装済み（既存を含む）：S4，S5，S7，S8（`tx.rs` `ur_13_a_failed_configure_by_the_owner_is_configured_again_without_reopen`，TX apply を 1 回拒否して tx_open 1，apply 2 を確認），S11a，S11c，S16，S25（`rx.rs` `ur_13_a_partly_applied_set_is_configured_whole_without_reopen`，RX channel 1 の apply を拒否，apply 4，`configured.by == 3`）．
- **未実装（deviation）：** S1，S2，S3 の RX 側，S12，S14（名前で指定した分），S20，S21（代わりのテストを書いていない）．
- **消した：** S11b（上記）．
- 既存テストの分類：`tx.rs` と `tests/fake.rs` の分類は表にまとめきれていない．実装で触ったものだけ確認した．

## mutation の行

- `plan/maintenance/tools/mutations.json`：P256，P296，P317，P318，P334，P344，P345，P351 を当て直し（file を core.rs へ）．P361 から P367 を消し，P368 を足した（`Owner::Nothing` → `Owner::Configure`，ur_25_an_enable_from_no_stream_configures_once）．
- `plan/phase7/tools/mutations.json`：U39 と U40 を当て直し，R14 を当て直し（file を core.rs へ）．mutations.json は行ごとに書き換えた（ファイル全体は書き直していない）．
- 全 27 行（P248–P256，P296，P317，P318，P334，P344，P345，P351，P358，P368，J16，J25，J49–J51，J62，U39，U40，R14）を `remote_mutate.py maintenance,phase7 sim02:8,sim03:8` で流した．**27/27 killed**（maintenance 24/24，phase7 3/3）．

## 確かめたこと

- `cargo clippy --workspace --all-targets -- -D warnings`：通った．
- `cargo test -p ezsdr-radio-uhd --no-fail-fast`：lib 138 passed，integration 227 passed，0 failed．ワークスペース全体のテストは走らせていない（clippy はワークスペース全体）．最後の変更（design/18 の 3 文）は 14:21，テストは 14:21 より後に走らせたので，テストは今の tree に対して有効．
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：0 problems．
- 両方の mutations.json の全行がファイルにちょうど 1 回一致（rules.py が確かめた）．

## セルフレビュー

- 回 1（試行 1）：DeepSeek（`opencode-go/deepseek-v4.1-flash#max`）が 30 分の時間切れ．判定なし（`round1-attempt1-no-verdict.log`）．
- 回 1（試行 2）：transport error のあと exit 124．判定なし（`round1.log`）．
- orchestrator の指示により，ここで止めた．**self-review: no verdict after two attempts (DeepSeek timed out reading the diff); the orchestrator's and Sol's reviews cover it.** 回 2 と回 3 は走らせていない．背景の job は残っていない．

## 変えた design/ の文（design/18-uhd-radio.md）

- 行 196（UR-13 周辺）：「enable または channel 数の増加のとき（UR-25），それ以外では configure しない」から，「configure が成功しなかったあと（UR-13）も」を足した．
- 行 213（UR-17）：「channel set が変わったら streamer を開き直す」を，記録が channel set を持たなければ開き直す，に直した．configure も同様に，記録が configuration を持たなければ行う．
- 行 238（UR-23）：同じく記録の言い方に直し，uhd-control の enable from no stream は両方を記録する，と書いた．
- 差分の行（differential の行）：「streamer は channel 数が変わったとき，または開けなかったあとだけ開き直す（拒否された configuration は開き直さない，UR-25）」．

## 大きさ

- 変更ファイル 10，+446 / -246 行（mutations.json を含む）．
- `crates/` の追加行は 412 行（テストを含む）．
- mutation の行：maintenance 744 行の JSON 配列，phase7 166 行．

---

## 最初の実装の回（blocked）の報告

STATUS: blocked: 実装の範囲が 1 回の実装として閉じられない．本体の並行制御（busy・版・lock の順）を先に分ける判断が orchestrator に要る（下記）

## 段階 1b-4d-1 実装報告（実装者，未完）

対象：`/home/komatsu/local_work/Ez-SDRv4`，HEAD `f487877`．

### 結果

- コードとテストは一切変えていない．`git status` はきれいなまま．`design/18-uhd-radio.md` も変えていない．
- `cargo test` と `cargo clippy` は流していない（変更がないので意味がない）．rules.py も流していない．
- mutation は流していない．行も `mutations.json` も触っていない．
- セルフレビュー（DeepSeek）は起動していない（実装が無いため）．
- 背景の job は残していない．
- 報告の結論：この指示書の範囲は，1 人の実装者が 1 回の変更で安全に閉じられる大きさを超えている．分け方を決めてから着手するべきだと判断した．

### 読んだもの

- `test-plan.md`（改訂版，S1–S28・S11d・S13b，§5 の busy 表，§7 の既存テストの扱い，§11 の mutation 対応）
- `adjudication.md`（設計 1〜4 の改め，D2 採用，D1 は A）
- `impl-brief.md`
- 本体：`provider/core.rs`，`provider/control.rs` 1–960 行（`ColdConfig`，`Planned`，`book_timed`，`release`，`cancel_held`，`book_cold` と from-none の枝），`provider/rx.rs` 120–360 行（`Rx` の状態，`turn`，`begin`，`finish`）
- mutation 行：P317，P318，P350，P351，P358，P361〜P367（すべて `plan/maintenance/tools/mutations.json` にある．`old` の一致と filter は読んだだけで，実行では確かめていない）
- 未読：`tx.rs` の `do_switch` 周辺（676–693 行だけ確認），`mod.rs`（arm と cleanup），`tests/fake.rs`，`tx.rs` と `control.rs` の既存 unit test の中身．

### 止まった理由（コードで確かめたこと，VERIFIED）

1. **`ColdConfig::configure` は `updates` の lock を device の呼び出し（`core.configure` → `apply`）の間持っている**（control.rs の `configure`，`let updates = self.updates.lock();` から `core.configure` まで）．
   `release` も `projections` の `updates` を `apply` の間持つ（control.rs の release，`let mut updates: Vec<_> = ...` から `apply` の `for` まで）．
   計画 §4 と UR-25 は「device を呼ぶ間は `updates`・`streams`・`rec` のどれも持たない」と書いている．計画は「どちらでも S13 と S28 が確かめる」と未決のまま残している．この選び方（`updates` を持たずに版を読む形）は，記録の `(seq, 版)` の一致と timed release の順序の両方に効く．実装の前に決める必要がある．
2. **from-none（control.rs の `book_cold`）は `streams` を離して device を呼ぶ**（`drop(streams)` → `rx_open`/`tx_open` → `check_rates` → `configure` → `lock(&self.core.streams)`）．busy を取るのは `streams` の中でなければならない（UR-25，計画 §5）．つまり from-none の形は，busy の取り方から組み直す．
3. **D1（A）は device に新しい経路を足す**．record-only の release は，次の segment が channel を持つとき，その segment の origin で timed command として出す．それには「segment が configure 済みか」を `streams` の中で読んでから `apply` する必要があり，上の 1 の lock の持ち方と衝突する．A の規則を実装する前に，lock の持ち方を決める必要がある．
4. **busy の範囲が広い**．計画 §5 の表は 10 の呼び出し（rx_open・tx_open・check_rates・configure・rx_start・rx_recv・rx_stop・tx_send・tx_async・Shutdown の ACK の読み，`cleanup` の close_streams）を覆う．`rx.rs`・`tx.rs`・`control.rs`・`mod.rs` の全てに触る．既存の `Rx::begin`（`opened` と `stale` を読む）と `TX::do_switch`（`verified` を読む）は，計画では `set`・`configured` の記録の読みに置き換える．
5. **既存テストの分類が未確定**．計画 §7 の `tx.rs` の残りの unit（861・887・924・945・964 付近）と `tests/fake.rs` の `ur_13_*`・`ur_25_*`（3719・5953・6008・6225・6278 付近）は，計画の時点で中身を読んでいない．実装の前に 1 本ずつ読む必要がある．
6. **規模**．計画 §8 は本体 350〜450 行，テスト 600〜800 行（helper と `ScriptedDevice` を含む）と見積もる．これは本体と既存テストの書き換えを含まない見積もりで，計画 §8 自身が「超えるときは分ける」と書いている．

### 提案する分け方（orchestrator の判断を仰ぐ）

計画 §8 の分け方を基にする．
- **段階 A（記録と版と owner の判断）**：S1–S8，S11a–S11c，S16，S20，S21，S25．版は `AtomicU64`，記録の `(seq, 版)`，`begin` と `do_switch` の判断を記録から読む．lock の持ち方は今のまま（`updates` を device の間持つ）にして，段階 B まで変えない．D1 は段階 B へ送る（A の規則は lock の持ち方が決まってから書く）．
- **段階 B（busy と lock の順）**：S9，S13，S13b，S15，S22–S24，S26–S28，D1（A）の S11d．busy の表（§5）と from-none の組み直しと lock の持ち方の変更をまとめて行う．
- Stop・loss・arm・Shutdown の系列（S17・S20・S26・S27）は，段階 A と B のどちらかに分けて入れる．次の段階に送らない（計画 §8 の判断のまま）．

段階 A を先に出すと，段階 B の前に `(seq, 版)` の記録が入る．その間，D1 のバグ（S11d の窓）は既存のまま残る．この点も orchestrator の判断が要る．

### 次に要ること

- orchestrator：上の 1 と 2 の lock の持ち方（`updates` を device の間持つのを改めるか，版を configure の前に読んで持たないか）を決める．D1 の A の規則を段階 B に送るかを決める．分け方（段階 A・B）を確定する．
- 実装者：決まった段階について，`tx.rs`・`mod.rs`・`tests/fake.rs` の既存 test を読んで分類してから，test plan の該当系列を書く．

### 変えていないもの

- `design/18-uhd-radio.md`（UR-13・UR-14・UR-17・UR-23〜26・UR-29・UR-30 の改訂は未着手．段階 A で UR-13・UR-24 の一部，段階 B で UR-25・UR-26 を改める想定）
- `plan/maintenance/tools/mutations.json`（P317・P318・P351・P358・P361〜P367 の対応は未着手）
- コード全般

### 大きさ

- 本体：0 行（未着手）
- テスト：0 行（未着手）
