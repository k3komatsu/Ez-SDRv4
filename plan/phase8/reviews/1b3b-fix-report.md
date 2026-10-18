STATUS: done

# Phase 8 段階 1b-3b 修正回（第1回）— F1〜F5 の反映

指示書: `/home/komatsu/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/fix-brief.md`（裁定 F1〜F5）．元の指示書 `brief.md` の「守ること」はすべて守っている（`cargo fmt`/`rustfmt` 不使用，mutation は `remote_mutate.py` だけ，`git add`・commit・push なし，今のモードのテストは1行も変えていない，2分超のコマンドは `nohup`＋`kill -0`）．ログと検証スクリプトは `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b3b/`（`fix3-*.log` と `fix-remote-mutate-final.log` が最終ツリーでの gate，`fix-self-review-1..3.log` がセルフレビュー）．HEAD `7993a16` からの変更ファイルは `git status` の10件だけ．

| file | この修正回での変更 |
|---|---|
| `crates/ezsdr-mock-radio/src/lib.rs` | F1（`handle_update` の `cold` の床と対象 stream の選び方，`key_direction` の追加）と F2（`handle_action` の `TxBurst` の振り分け） |
| `crates/ezsdr-mock-radio/tests/mock_layout.rs` | F1・F3・F4 のテスト8本を追加（13→21本） |
| `design/07-radio-model.md` | RM-25 に layout instance の床の規則を追加，*Test:* 行，Changes |
| `design/09-mock-radio.md` | MR-18 の `cold` と *Ceiling*，MR-40 の文，MR-2 の selector の既定（F5），MR-4 の規範本文（F5），MR-16・MR-20・MR-25・MR-29 の *Test:* 行，§6 の表と `rules` 列，Changes |
| `design/08-simulation.md` | SE-6 の引用を MR-12 → MR-7（セルフレビュー第1回の nit） |
| `plan/maintenance/tools/mutations.json` | P141〜P148 を追加，P143 を当て直し |
| （この修正回では変更なし） | `crates/ezsdr-radio/*`，server，Python，fixture，UHD Module，`Cargo.toml`，`schemas/`，`plan/phase3/tools/mutations.json`（後者は 1b-3b 本体の1行のまま） |

## 1. F1〜F5 それぞれの直し方・テスト・mutation の行

### F1（major）— `keys` instance の `cold` 変更の順序

直し方（`crates/ezsdr-mock-radio/src/lib.rs`，`handle_update`）:

1. **先に床を決める**（`keys` 側の分岐）．`asked = at|now`，`base = max(asked, now, 受信のキーなら self.start_tick)`，`floor = self.updates` に残っている `class == Cold` の更新の effective instant の最大，`e = max(base, floor)`．`asked < now || e > base` なら `LATE_COMMAND`（payload は `at` と `e`）．床は**方向・stream・line の有無を問わず instance 全体**で，`hardware_timed` の分岐には掛からない（`hardware_timed` は自分の instant，床は `cold` だけ）．適用済みの `cold` は `self.updates` から消えていて effective ≤ now なので `now` の床で足りる（`advance(u)` は effective ≤ u = now のものだけ適用する）．
2. **その instant の設定で対象 stream を選ぶ**．`projected(e, order)`（適用済み ＋ `e` までの pending 更新）にその変更を入れた候補を `cold_streams` に渡す（移動の拒否もここ）．**`asked` の設定で選ぶ旧形は消した**．床が「先に受けた `cold` 変更のどれの effective instant も下回らない」なので，先に受けた変更はすべてこの投影に入り，membership・throughput・全二重の判定が先の予約と矛盾しない．
3. その候補で `coerce`（`Description::coerce` が `violations` を通す）→ 対象 stream の line に記帳，は従来どおり1か所のまま．
4. 「受信の stream の開始」の床は，キーが受信の stream／channel を名指すとき `self.start_tick`（= T0 = 各受信 line の origin）を使う．**line を持たない（link のない）受信 stream でも床が掛かる**ようにするため（Sol 3 への回應）．
5. `key_direction(key)`（新規，`ezsdr_radio::concrete` から方向を取る読み出し1つ）を追加．今のモード（`keys` なし）の分岐は `timeline::command_instant(line, …)` のまま**1文字も変えていない**．

spec: `design/07-radio-model.md:258`（RM-25 第2段落）に「With a layout description (RM-29) an `UpdateParameter` of a `cold` key is floored at every `cold` change of the same instance received earlier, whatever direction, stream or line it belongs to, and, when its key belongs to a receive stream, at that stream's start (RM-21) as well.」を追加し，*Test:* 行に4つのテストを名指し．`design/09-mock-radio.md` の MR-18 を追随（`cold` の文面を「under `keys` any `cold` change of the instance …」に，対象 stream を「in the configuration in force at `e`」に），**3つ目の *Ceiling*（`asked` の設定で stream を選ぶ既知の誤りを規範に書いたもの）を削除**．RM-40 から「RM-25's route to a move … is not taken.」を削除（段階の状態を規則本文に置かない，DA-1）．

テスト（`crates/ezsdr-mock-radio/tests/mock_layout.rs`）:

| test | 何を確かめるか（どの誤りを拒むか） |
|---|---|
| `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change`（:919） | Sol 1 の反例．`c1.stream=-1 @now+5 ms` の後に `at` なしで `c1.stream=1`．2つ目は `now+5 ms` に床付けされ（`LATE_COMMAND` が1件，`applied` は `[(-1, now+5 ms), (1, now+5 ms)]` の順），stream 1 の line は `now+5 ms` で切られて c1 を持つ新しい clock を始める（clock 2本，新しい clock の origin 以降のブロックは channel 数1で domain が新しい clock） |
| `mr_18_a_layout_cold_floor_crosses_directions`（:961） | 受信 c0 の antenna を `TX/RX` に @now+20 ms，続けて同じ board の送信 channel を stream に入れる @now+10 ms．2つ目は `now+20 ms` に床付けされ，その時点の設定で RM-31 により `COMMAND_REJECTED`（`rejected` 1件，`applied` は antenna の1件だけ），送信 channel は stream に入らず clock も持たない |
| `mr_18_a_layout_reservations_cannot_invalidate_an_earlier_one`（:981） | Sol 2 の membership 反例（`c1→s1 @now+20 ms`，`c1→s0 @now+10 ms` → `RM-34: moving channel c1 from stream s1 to stream s0` で拒否，適用は s1 だけ）と throughput 反例（1 GbE，s0=5・s1=10 Msps，`c1→s1 @now+20 ms` と `s0=20 Msps @now+15 ms` → 後者は `now+20 ms` に床付けされ RM-32 で拒否，理由に `120000000`） |
| `mr_18_a_layout_cold_floor_holds_without_a_line`（:1024） | Sol 3 の反例．link のない受信 stream の rate を @now+10 ms と @now+5 ms → 2つ目は `now+10 ms` に効き（`applied` の順が `[2e6, 4e6]`，各 `at` は `now+10 ms`，`LATE_COMMAND` 1件），最終の rate は2つ目の値 |
| `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant`（:1050） | `cold` の予約（@now+20 ms）の後に受けた `hardware_timed`（@now+10 ms）は自分の instant に効き，遅れ扱いされない（`applied` の順と `at`，`LATE_COMMAND` 0件）．床が `cold` だけに掛かることの pin |

mutation の行（`plan/maintenance/tools/mutations.json`，すべて `test_bin: mock_layout`）:

| id | 誤りの形（`old` → `new`） | kill するテスト | 結果 |
|---|---|---|---|
| P141 | `let floor = self.updates.iter().filter(…Cold…).map(…).max();` → `let floor: Option<i64> = None;`（床を掛けない） | `mr_18_a_layout_cold_change_is_floored_at_every_earlier_cold_change` | killed |
| P142 | 同じ `old` → `… && key_direction(&update.key) == direction …`（方向ごとにしか掛けない） | `mr_18_a_layout_cold_floor_crosses_directions` | killed |
| P143 | `let mut with = projected.clone(); … cold_streams(&key, &projected, &with)` → `let asked = self.projected(requested_tick.unwrap_or(now), order); … cold_streams(&key, &asked, &with)`（`asked` の設定で対象 stream を選ぶ） | `mr_18_a_layout_reservations_cannot_invalidate_an_earlier_one` | killed |
| P148 | `let requested = requested_tick.unwrap_or(earliest);` → 同じ床を `hardware_timed` にも掛ける形 | `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` | killed |

（P148 は fix-brief の3つに加えてセルフレビュー第1回の nit「`hardware_timed` は対象外であることを固定する行がない」に答えて足した．）

### F2（minor）— 今のモードの `TxBurst` の拒否の文面

`handle_action` の `TxBurst` を `match (self.profile.is_layout(), self.addressed(node))` にし，**今のモードは無条件で `handle_tx_burst(0, id, action, node, now)`** へ戻した（`keys` は今の振り分けのまま）．`handle_tx_burst` は HEAD と同じく **clock を先に**（送信 channel がなければ `MR-16: no transmit channel is configured`），**宛先を後に**調べる．テストは足していない（fix-brief も Opus O2 も「テストは捕まえない」としている）：今のモードの3テストファイルを1行も変えない縛りがあり，この組合せ（誤った宛先 ＋ 送信 channel なし）を固定する場所が既存の `mr_16_burst_refusals`（その組合せを含まない）しかないため．差は event payload と Manifest の文面だけで，振る舞い（拒否・何も変えない）は同じ．

### F3（minor）— 「全 stream」の道

| test | 何を確かめるか |
|---|---|
| `mr_25_a_device_stop_stops_every_layout_stream`（:1072） | 受信2本（UBX と BasicRX）と送信2本（UBX と BasicTX）を持ち，各送信 stream に held burst を置いた `keys` instance で，`<device>` 宛ての `Stop` が (a) 受信2本の clock をその instant に終わらせ（`ended_at` = stop instant），(b) 両送信 stream の held burst を取り消し（`rejected` に `MR-25: cancelled by stop` が2件），(c) 以後どの port にもブロックを出さない |
| `mr_20_a_lost_device_stops_every_layout_stream`（:1103） | 同じ4 stream で `device_lost`（T0+1 ms）: `step` が `DeviceLost` を返し，受信2本の clock が loss の instant に終わり，両送信 stream の held burst が `MR-20: device lost` で取り消される（2件） |

| id | 誤りの形 | kill するテスト | 結果 |
|---|---|---|---|
| P144 | device の `Stop` の stream 列挙を `vec![(Direction::Rx, 0)]` に（全 stream でなく1本だけ） | `mr_25_a_device_stop_stops_every_layout_stream` | killed |
| P145 | `stop_at` の受信の `End` の記帳を `.take(1)` に（loss が最初の受信 stream しか終わらせない） | `mr_20_a_lost_device_stops_every_layout_stream` | killed |

`start` 失敗時の巻き戻し（(c)）は fix-brief どおり**記録のまま**（`schedule_wakeup` を失敗させる手立てが要る．§6）．

### F4（minor）— stream ごとの送信 channel 数

`mr_16_each_transmit_stream_is_judged_by_its_own_channel_count`（:1131）: `ideal` の送信3 channel を s0 に2本（c0・c1），s1 に1本（c2）入れ，s0=2 Msps・s1=1 Msps．同じ 400 byte の waveform が s0 では 25 サンプル，s1 では 50 サンプルと解釈され（`section("bursts")` の `samples` が `[25, 50, 3]`），割り切れない 24 byte は s0 だけ拒否され（`rejected` 1件，`MR-16: target, clock, waveform size…`），s1 では 3 サンプルとして受理される．

| id | 誤りの形 | kill するテスト | 結果 |
|---|---|---|---|
| P146 | `open` の channel 数を `self.effective_stream_channels(Direction::Tx, 0)` に固定 | 同上 | killed |
| P147 | 同じ `old` → `…(Direction::Tx, 1)` に固定 | 同上 | killed |

### F5（minor）— spec 09 の文

- `design/09-mock-radio.md:40`（MR-2 の selector 表）: `master_clock_hz`, `image`, `links`, `link_gbps` の既定を **「200 000 000, `hg`, 1, and `link_gbps` 10 without `keys` and the image's own speed under it (1 for `hg`, 10 for `xg`; RM-28)」** とモード別に直した（実装: `ezsdr-radio/src/channels.rs:181` が image 依存，`src/x3xx.rs:538` が常に10）．Sol 5 の指摘どおり `keys` instance の `hg` の既定は 1．
- `design/09-mock-radio.md:39`（`rx`・`tx` の行）: セルフレビュー第3回の P1．「under `keys` for either profile, the counts' default 1 and 0」は `x3xx` では伪（`x3xx` は layout の要素列が必須，既定 1/0 は `ideal` の count だけ）．**「required for `x3xx`; under `keys`, `ideal`'s counts default 1 and 0」** に直した（`channels.rs:204-227`，design 33 D5 と一致）．
- `design/09-mock-radio.md:84`（MR-4）: 規範本文がテスト名に node・port の定義を委ねていた文を直し（テスト名は *Test:* 行へ），さらに RM-29 の node・port の形を写していた部分を落として **RM-29 を引くだけ**にした（DA-1）．

## 2. 当て直した mutation の行

P143 のみ（§1 の表．1b-3b 本体の当て直し7行〈maintenance/F37・J196・J276・P10・P12・P15，phase3/M60〉は全 `all` ランで killed のまま）．全6リストの全行について `old` が対象ファイルにちょうど1回一致することも確認した（`fix-remote-mutate-final.log`，セルフレビュー第2回・第3回の全数チェック）．

## 3. 確かめること（指示書 1〜7）の結果 — 最終ツリーで

| # | 何を | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | **1235 passed / 0 failed**（65 test bin．本体 1227 ＋ 新しい8本） | `fix3-cargo-test-workspace.log` |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0** | `fix3-clippy-workspace.log` |
| 3 | Docker の `--features uhd` clippy（`ezsdr-radio-uhd`） | **exit 0** | `fix3-docker-clippy-uhd.log` |
| 4 | Python（venv 3.14，`cargo build -p ezsdr-server` の後） | **Ran 47 tests … OK** | `fix3-python-suite.log` |
| 5 | `rules.py --base HEAD` / `--self-test` | **0 problem(s)** / **exit 0** | `fix3-rules.log`，`fix3-rules-selftest.log` |
| 6 | 全6リスト（`all sim02:8,sim03:8`，915 行） | **914/915 killed．maintenance 539/539（P122〜P148 を含む）**．唯一の非 kill は既知の `phase7/G09`（`crates/ezsdr-kernel/src/coordinator/stepping.rs` の `drain_and_react` と `device_paced` のテスト．spec 22 の step 2 から記録されている既存の生き残りで，この変更と無関係） | `fix-remote-mutate-final.log`（途中の 27 行再実行は `fix-remote-mutate-2.log`，本体の全ランは `fix-remote-mutate-all.log`） |
| 7 | `git diff HEAD -- tests/mock_radio.rs tests/mock_channel.rs tests/timeline.rs` | **空**（0 行） | `fix3-tests-diff.log`，`git status` は `fix3-git-status.log` |

（`cargo fmt`/`rustfmt` 不使用．mutant を作業ツリーに当てていない（切り分け不要）．版を上げていない（`Cargo.toml`・`schemas/` 無変更）．新規ファイルは 644．Python は `PYTHONDONTWRITEBYTECODE=1`．今のモードは `keys` selector 経由でしか新モードに到達できない．1〜5 と mutation は同じ最終ツリーで流し直した〈セルフレビュー第2回の P1 への対応〉．）

## 4. セルフレビュー（AGENTS.md §9 の手順2）— 新しい読み取り専用のレビュー役で3回

`opencode2 run --standalone -m opencode-go/deepseek-v4.1-flash --agent plan --auto "<fix-review-checklist.txt>"` を毎回**新しい**役として nohup で起動し，`kill -0` で待った（レビュー役には `cargo`・mutation を禁じ，読み取り・grep・`rules.py` だけを許した）．各回はそのときのツリーを凍結したままレビューしている．

### 第1回（`fix-self-review-1.log`，約 20 分）— P0 0件，**P1 0件**，P2 2件，nit 4件

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | — | F1〜F5 はすべて直っている（実装・design・テスト・mutation の対応を `path:line` で確認） | — |
| 2 | P2 | RM-25 の新しい文と RM-34／`book_move` の床が食い違って見える（3本目の line の先の `cold` を移動が追い越せる．今は移動を拒否するので実害なし，1b-3c の前に要解消） | **採った**：RM-25 の文を Provider の `UpdateParameter` の規則であることが明確な形へ狭め（第3回でさらに「an `UpdateParameter` of a `cold` key」に）．RM-34（accepted な 1b-2b）と `book_move` はこの段階では触らず，**1b-3c の解消事項**として §6 に記録 |
| 3 | nit | `hardware_timed` は床の対象外であることを固定するテストと行がない | **採った**：`mr_18_a_layout_hardware_timed_change_keeps_its_own_instant` と P148 を追加 |
| 4 | P2 | F3/F4 の新しいテスト3本が規則の *Test:* 行にない（§6 の表にだけある） | **採った**：MR-16・MR-20・MR-25・MR-29 の *Test:* 行に名指しを追加 |
| 5 | nit | `design/08` の SE-6 が node の綴りを MR-12 に帰している（綴りは MR-7 step 8） | **採った**：引用を MR-7 に直した |
| 6 | nit | `stop_at` が `self.updates` を空にするので，`Stop`/loss の後に来た `cold` はそれらに床られない（RM-25 の文は無条件に読める） | **採らない（記録）**：`Provider::stop` と loss は残るコマンドを落とす（RM-16，MR-20）ので，効かない変更の instant を床に使う必要はない．文の読み方は 1b-3c の整理で一緒に見る（§6） |
| 7 | nit | Vision §8 の「方向ごとに1つの key」がまだ古い（1b-2a からの持ち越し） | **採らない（記録）**：1b-5 で直す．この段階が新たに伪にしたものではない |
| 8 | nit | `projected(tick, order)` を2回計算（冗長） | **採った**：1回にまとめ，P143 を当て直した |

### 第2回（`fix-self-review-2.log`，約 22 分）— **P0 0件，P1 1件**（証跡の鮮度），P2 2件，nit 4件

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P1 | 「確かめること」1・2 のログが，その後のテスト追加・`mutations.json`・`design/` の編集より古い（当時の mock_layout は20本，今は21本）．凍結した最終ツリーで流し直すこと | **採った**：最終ツリーで 1〜5 を流し直した（§3 の `fix2-*`→`fix3-*` ログ）．すべて成功，7 も空 |
| 2 | P2 | RM-25 と RM-34 の床の読み方（第1回 2 と同じ，具体例つき） | **第1回と同じ対応**（RM-25 の文を狭め，RM-34 は 1b-3c の解消事項） |
| 3 | P2 | `design/09` の MR-40 末尾「RM-25's route to a move … is not taken.」はこの段階の状態を規則本文に恒久の設計として書いている（1b-3c で古くなる．DA-1 にも関わる） | **採った**：その文を削除（拒否と「何も変えない」は第1文とテストで固定されている） |
| 4 | — | F1〜F5 はすべて直っている．`old` の1回一致を全リストで再確認，P141〜P143 が本当に kill されることも静的確認 | — |
| 5 | nit | `stop_at` の `updates` の扱い（第1回 6 と同じ） | **採らない（記録）**：同上 |
| 6 | nit | 同じ `t` で `harness.step(t)` を2回呼ぶ箇所の危うさ（静的には問題なし，実行確認は必須） | **採った（第3回と併せて）**：1回の step にまとめた（§5-6）．実行で全テスト成功（§3-1） |
| 7 | nit | `Profile::description()` が layout で `panic!`（1b-3b からの持ち越し） | **採らない（記録）**：§6（1b-4/1b-5 へ） |
| 8 | nit | `projected` の二重計算（第1回 8）と `start_tick` の床が実質常に無効である点 | **一部採った**：二重計算は第1回で解消済み（第2回が見たのはその前のツリー）．`start_tick` の床は line のない受信 stream でのみ効く（Sol 3 の反例）ので残す |

### 第3回（`fix-self-review-3.log`，約 22 分）— **P0 0件，P1 1件**，P2 4件，nit 3件

| # | severity | 指摘 | 扱い |
|---|---|---|---|
| 1 | P1 | `design/09-mock-radio.md:39`（`rx`・`tx` の既定）が `x3xx` under `keys` について伪（layout の要素列は必須で，既定 1/0 は `ideal` のみ．F5 の直し残し） | **採った**：既定の欄を「required for `x3xx`; under `keys`, `ideal`'s counts default 1 and 0」に直した |
| 2 | P2 | RM-25 の新文と RM-34 の移動の床（第1回・第2回と同じ．「configuration key の `cold` 変更」でも移動を含むと読める） | **採った**：RM-25 の文を **「an `UpdateParameter` of a `cold` key」** に直し，移動（`Lines::book_move` が記帳する `Kind::Cold` の item）を読まない形にした．RM-34／`book_move` を instance 全体へ広げるのは 1b-3c（§6-1） |
| 3 | P2 | §6 の `rules` 列と規則の *Test:* 行の不一致4件（`mr_38_a_layout_instance_uses_one_key_set` に MR-6，`mr_39_…` に MR-38，`mr_41_…` に MR-31 が無く，`rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` を名指す *Test:* 行が無い） | **採った**：§6 の3行に MR-6・MR-38・MR-31 を足し，MR-18 の *Test:* 行に `rm_31_rm_32_…` を名指しした |
| 4 | P2 | 範囲外の `stream` 値（例 `5`）で，チャネルが stream に入っているとき拒否の理由が RM-34 の移動文面になる（`was < 0` のときは RM-8/RM-4 で拒否，という非対称．振る舞いは正しい） | **採らない（記録）**：fix-brief F1 が順序を「(2) `cold_streams`（移動の拒否もここ）→ (3) `coerce`」と指定しており，振る舞い（拒否・何も変えない）は同じ．理由の文面の整理は 1b-3c の移動実装と同じ場所（§6-3） |
| 5 | P2 | Vision の「one key per direction」ほか（1b-2 の持ち越し，この fix の範囲外） | **採らない（記録）**：1b-5（§6-5） |
| 6 | nit | MR-4 の規範本文が RM-29 の node・port の形を写している（DA-1 は Provider spec が Vocabulary の規則を引いて mechanism を足す形を求める） | **採った**：形の再掲を落とし，RM-29 を引くだけにした |
| 7 | nit | `cold`/`with` の二重 clone，`cold.into_iter().flatten()`，`cold_streams` が stream の列と移動拒否を1つの `Result` に混ぜている読みにくさ（挙動は正しい） | **採らない（記録）**：1b-3b 本体の形．1b-3c で移動を扱うときに `cold_streams` の戻り値の形（拒否を分けるか）と併せて見るのが自然（§6-3） |
| 8 | nit | 同じ `t` の2回 step（第2回 6）と，`Profile::description()` の panic | **採った／記録**：step は1回にまとめた．`description()` は §6-2 |

第3回の修正のあと，4回目は fix-brief の「最大3回」に従って起動していない．第3回の修正は機械的な確認（テスト・clippy・Docker clippy・Python・`rules.py`・mutation の全ラン）で通してあり，**新しい読み取り専用レビュー役による再確認は受けていない**（orchestrator と Sol のレビューが次の段）．

## 5. 指示書・fix-brief から外れたところ，足したもの

1. **P148 と `mr_18_a_layout_hardware_timed_change_keeps_its_own_instant`**（fix-brief の F1 の3行に追加）: 床が `cold` だけに掛かることを固定していなかったため（セルフレビュー第1回 3）．
2. **`handle_update` で `projected(tick, order)` を1回にまとめた**（fix-brief は形を指定していない）: セルフレビューの nit．P143 を同じ意味の形に当て直した．
3. **`design/09` MR-40 の1文を削除**，**`design/07` RM-25 の文を「an `UpdateParameter` of a `cold` key」に限定**，**`design/08` SE-6 の引用を MR-12 → MR-7**，**MR-4 から RM-29 の形の再掲を削除**，**§6 の `rules` 列3件と MR-18 の *Test:* 行**，**`design/09:39` の `rx`・`tx` の既定**（第3回の P1）: セルフレビューの指摘への対応．
4. **テストの `step` を1回にまとめた**（第2回・第3回の nit）: 「同じ instant で2回 step して受領を刻む」形をやめ，更新を push してから1回の step で受領と同時に公開する形にした（`advance_to` は同値の instant を受けるが，実装依存の前提を残さない）．assertion は変えていない．
5. F1 の throughput 反例の数値を fix-brief の例（s0=5 Msps @+10，c1→s1 @+20，s0=20 Msps @+15）から**変えた**（s1=10 Msps，s0 を 5 → 20 Msps @+15）: fix-brief の数値では，床がなくても3つ目が 1 GbE の 100 MB/s を超えて拒否され（160e6），「床を掛けない」誤りを殺せない．`x3xx::performance(1)` = 100e6 B/s，`wire_bytes_per_sample` 4（`x3xx.rs:470-475`）なので，(5+10)·4 = 60e6 ≤ 100e6 が床の前，(20+10)·4 = 120e6 > 100e6 が床の後と分かれる組にした．membership 反例（Sol 2）と Sol 1・Sol 3・方向をまたぐ形は fix-brief のとおり．
6. F3 のテストの layout は受信 UBX＋BasicRX，送信 UBX＋BasicTX にした（1つの slot に duplex board と別 board は置けず，`two()` と2送信の組合せは `MR-2` の layout の拒否になるため）．
7. F2 にテストを足していない（§1 F2）．

## 6. 未解決（1b-3c・1b-4・orchestrator へ）

1. **1b-3c: RM-34 の移動の床を instance 全体へ広げる**．RM-25 の新しい規則（spec 07:258）は `cold` key の `UpdateParameter` を「同じ instance の先に受けた `cold` 変更のどれよりも早く」効かせるが，RM-34（spec 07:354）と `ezsdr-radio::timeline::Lines::book_move` の床は**その2本の line** の先着 `cold` だけを見る．移動はいま拒否される（MR-40）ので実害はないが，1b-3c で移動を記帳する前に，RM-34／`book_move` を instance 全体の床へ（または RM-25 の文を移動にも掛かる形へ）揃える必要がある．design 33 D3's1（両 stream の遅い方で床る）と fix-brief F1（「1b-3c の移動の床もこれに含まれる」）の突き合わせもここで行う．セルフレビュー第1〜3回がともに同じ指摘（P2）を出した．
2. **1b-3c: 1b-3a からの持ち越しで残るもの**（元の報告 §11 の 2・4・5・8）: `start` 失敗時の複数 stream の巻き戻し（失敗を注入できる形にする），`ChannelMode.lo_retune` の引き順，medium に渡す `Transmitter` の写像，`Profile::description()` の panic（`Option` にするか channel mode の設定を `Profile::channels()` 側へ寄せる），`Concrete`/`concrete` の `pub` の見直し．
3. **専用の mutation 行が無い規則と，読みにくさ**（元の報告 §11 の 3．セルフレビュー第3回 4・7）: MR-10，MR-16 の内側の宛先検査（layout では到達不能），MR-29 の `start_rx` の宛先，MR-34 の否定文，MR-41 の受理側（layout で `description()` の panic になる）；範囲外の `stream` 値の拒否の理由（`cold_streams` → `coerce` の順によるもの）と `cold_streams` の戻り値の形は 1b-3c の移動の実装と一緒に見る．
4. **`stop_at` が残る `updates` を空にする**（`Provider::stop` と loss）: 落とされた `cold` 変更は効かないので床に使わない，という現在の読みを RM-25 の文に反映するかは 1b-3c の整理に含める（セルフレビュー第1回 6・第2回 5）．
5. **Vision の radio の key 記述**（`design/vision/02:24-29,54` の「方向ごとに1つの key」と §8 の例，`design/vision/03:71` の grid 文）は 1b-5（切り替え）で直す（1b-2a からの持ち越し．この段階が新たに伪にしたものではない）．
6. **`phase7/G09`**（`crates/ezsdr-kernel/src/coordinator/stepping.rs` の `drain_and_react` と `device_paced` のテスト）は全リストで唯一の非 kill で，spec 22 の step 2 から記録されている既知の生き残り（`plan/maintenance/29-unified-admission.md:363` ほか）．この変更とは無関係（この段階の作業対象ファイルではない）．
7. **orchestrator へ**: design 33 §5 の 1b-3b の行と報告の参照（`plan/phase8/reviews/`），この修正回の記録の置き場（`plan/phase8/00-overview.md` か reviews の下），handoff，`plan/maintenance/failure-mechanisms.md` の `mech:` の数え方（F1 は `keys` instance の `cold` の順序という1つの機構で，Opus 1件・Sol 3件が出た）の判断は orchestrator の担当．私は commit・push・`git add` をしていない．
