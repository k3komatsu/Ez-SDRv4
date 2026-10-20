STATUS: done

# Phase 8 段階 1b-4b（UHD Module の内部を stream ごとに組み直す）実装報告

指示書: `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4b/impl-brief.md`（HEAD `fdfdf3b`，clean から開始）．
計画: 同 `brief.md`・`test-plan.md`（冒頭の orchestrator の裁定と Sol のテスト計画レビューの裁定が優先）・`sol-plan.md`．

## 1. 変えたファイルと要点

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-radio-uhd/src/provider/core.rs` | `Streams` を `lines: timeline::Lines`（key `(Direction, k)`）と `rx`/`tx: Vec<StreamState>` にし，instance 全体の `seq`・`ended`・`stop`・`version` を残した．`StreamState { configs, delivered, idle, clocks }` が stream ごとの状態．`stream`/`stream_mut`/`count`/`line`，`stream_key`・`dir_of`・`booked_stream` を足し，`book`・`refuse`・`end`・`prune`・`planned`・`changed`・`reconcile`・`record` を `(dir, k)` で動くようにした．`end` は存在する全 line に同じ `(e, seq)` の `End` を 1 回だけ記帳する． |
| `crates/ezsdr-radio-uhd/src/provider/mod.rs` | `Prepared.rx`/`tx` を `PreparedStream`（`k`・handle・channels・clock）の列にし，`Running.to_tx`/`to_rx` を sender の列にした（今は各 1 本）．`arm` は stream ごとに open し，tx の clock を `PreparedStream.clock` に登録する．`start` は列を長さ 1 の配列に `try_from` で取り（それ以外は `Rejected`，黙って捨てない），stream ごとに 1 thread（`uhd-rx`・`uhd-tx` の名前は不変）を起こす．`stop` は control → 全 tx sender → `join("uhd-tx")` → 全 rx sender → `join("uhd-rx")` の順を保つ．`mod tests` に `start` の guard の unit test を足した． |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | `Control::new` は `Lines::insert` で line を置き，`StreamState` の `clocks`・`idle` を初期化する．command は宛先 stream（今は方向ごとの stream 0，`booked_stream`）の line と `configs`・`idle` に記帳し，from-none の open は `k` を渡す．`rx_packet_samples(0)` は残した（final (a): `Control::new` が作る受信 line は stream 0 の 1 本だけなので，`k` にしても値は 0 で新しい情報がない．stream ごとの line を作る 1b-4c で `k` にする）． |
| `crates/ezsdr-radio-uhd/src/provider/rx.rs` | `Rx` が `k` を持ち，`rx_recv`・`rx_open`・`rx_start`・`rx_stop` と line の参照（`follow`・`finish`・`begin`・`samples`）が `k`/`(Rx, k)` を使う．`stop_at_cut` に `k` を渡す． |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | `Tx` が `k` を持ち，`tx_open`・`tx_send`・`tx_async`・`read`・`do_switch` が `k`/`(Tx, k)` を使う． |
| `crates/ezsdr-radio/src/timeline.rs` | `Lines::prune_stream(stream, before)` を足した（1 line だけを畳む．RM-34 の move は `Lines::prune` に残す旨を doc comment に書いた）．既存の `Lines` のコードは変えていない． |
| `crates/ezsdr-radio-uhd/src/provider/test_support.rs` | `LockSpy`（device call の入口で `core.streams.try_lock()` を確かめ，呼ばれた回数を数える `FakeDevice` の wrapper）を足した． |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | 新しい 7 テストと `Gated` wrapper（receive と transmit の call を gate で止め，`stop` の join 順を観測する）． |
| `design/18-uhd-radio.md` | §3 の thread の図，UR-3 の Threads，UR-26 の join の順，U2 の Threads 行を D15（stream ごとの thread）に合わせ，UR-16 と UR-26 の Test 行に新しいテストを足した．Changes に 2026-10-18 の 1 行を足した． |
| `plan/maintenance/tools/mutations.json` | 新規 39 行（P242〜P280）．当て直し 12 行（F40・G60・J15・J16・J21・J49〜J53・J62・J64）． |
| `plan/phase7/tools/mutations.json` | 当て直し 7 行（U22・R12・R13・B01・B02・B20・B28）． |

## 2. 構造の決定（裁定からの外れと理由）

裁定の「構造」はそのまま採った．外れたのは次の 4 点で，いずれも裁定の範囲内の判断:

1. **stream ごとの状態は `StreamState` 1 型**（`configs`・`delivered`・`idle`・`clocks`）にし，`Streams.rx`/`tx` を `Vec<StreamState>` にした（裁定の「stream ごとの型に移す」）．`tx_clocks` は `clocks` として同じ器に入れ，受信 stream では空のまま．`Default` が方向ごとに 1 本（k=0）を作る（今の key の layout）．
2. **`Prepared`/`Running` の列は「今は 1 本ずつ」**．command channel は方向ごとに 1 本のままで，`Running` の sender を `Vec` にした．`start` は `rx_streams`/`tx_streams` を長さ 1 の配列に `try_from` で取り，長さが違えば `Rejected`（`"UR-15: this stage starts one receive stream"` / `… transmit stream`）— 列の先頭だけを使って残りを黙って捨てない．1b-4c が stream ごとの channel と名前 `uhd-rx-s<k>` を入れる．
3. **item 4 の N1 と item 2 の T1 の行は同じ規則**（`Streams::end` の `ended` guard）なので 1 行（P243）にまとめ，両方のテストを報告に並べた．item 7 の D 行（shutdown の `expected + 1`）は採らなかった: `Rx::follow` の abort の床が同じ `cut_at(instant(expected))` を先に置き，`cut_at` は `min` なので，`+1` は観測できない（Sol の major 2「届かせられなければ行を採らない」）．その枝の観測できる効果（queued start の untimed stop 1 回）は既存の G60 が固定している．
4. **item 9 の T1 の行（P264）は計画の projected booking ではなく `ColdConfig::configure` の base にした**．`book_timed` の投影への記帳を消す行はほぼ equivalent だった: `release` の `issued` が同じ update を effective instant で投影に入れ直すので，configure がそれを見る（run 2 で SURVIVED，§5）．計画の「timed が switch に届かない」誤りを同じ形で壊す行として，`let settings = Settings { rate: Some(rate), ..self.settings(...) }` を `..self.base.clone()` にした（P264）．排他 bound の規則は Sol の major 2 の指示どおり純 `ColdConfig` のテストに結び，P279（`..(origin, 0)`，filter `ur_25_switch_projects_timed_updates_in_effective_order`）にした．

`Control::new` の `rx_packet_samples(0)` は裁定の条件付き項目のまま残した（§1 の control.rs の行に理由）．

## 3. 計画の項目ごとに，テストと行

いずれも `remote_mutate.py` で killed（§5）．「誤りの形」は行が壊す規則．

| 項目 | テスト（新しいもの） | 行 | 誤りの形 |
|---|---|---|---|
| 1 | `ur_25_a_receive_enable_without_a_link_is_only_recorded`（lib） | P242 | link がなくても受信 line を作る |
| 2 | `ur_26_a_stop_ends_every_line_once`（lib） | P243 | 2 回目の `end` も記帳する |
| 2 | `ur_26_a_second_stop_does_nothing`（fake） | P244 | 2 回目の `Provider::stop` が stop 行を書く |
| 2 | `ur_26_the_sending_threads_join_before_the_receiving_ones`（fake） | P245 | receive を transmit より先に join する |
| 3 | `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message`（fake） | P246 | `DEVICE_LOST` の message を変える |
| 4 | `ur_29_a_loss_ends_every_stream_once`（lib） | P247（と P243） | tx の line に End を記帳しない（P243 は 2 回目） |
| 5 | `ur_24_a_refusal_halts_only_its_own_stream`（lib） | P250・P251 | 拒否が全 line を halt する（P250），halt の読みが他方向（P251） |
| 5 | `ur_25_a_refusal_leaves_the_transmit_stream_alone`（lib） | P248・P250 | rx の拒否を tx line に載せる（P248） |
| 5 | `ur_25_a_refusal_leaves_the_receive_stream_alone`（lib） | P249・P250 | tx の拒否を rx line に載せる（P249） |
| 6 | `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one`（fake） | P255（と P254） | switch で segment の添字を進めない（P255），clock のない owner が次の segment を取る（P254） |
| 6 | `ur_25_an_enable_from_no_stream_configures_once`（fake） | P256 | from-none の configure を owner にもやらせる |
| 6 | 既存の enable テスト（R1・R2） | P252・P253 | from-none を常に false（P252）／常に `channels > 0`（P253） |
| 7 | （既存のテスト） | P257・P258 | 床の guard が standing plan を読む（P257），床が block の先頭（P258）．D は §2 の 3 |
| 8 | （既存のテスト） | P259〜P263 | clock を登録しない（P259），plan が clock を持たない（P260），cut で clock を終えない（P261），version を進めない（P262），arm の clock を seed しない（P263） |
| 9 | `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`（fake） | P264 | configure が base を使う（timed が switch に届かない） |
| 9 | `ur_25_an_enable_at_a_timed_update_s_instant_applies_it`（fake） | P265 | enable の merge が等号 instant を落とす（`..(e, 0)`） |
| 9 | `ur_25_a_cold_change_and_a_timed_update_at_one_instant_keep_their_own_instants`（lib） | P266・P267 | seq を進めない（P266），timed を cold の床まで上げる（P267） |
| 9 | `ur_24_the_device_queue_is_one_across_the_directions`（lib） | P268 | device queue の順を方向ごとにする |
| 9 | 既存の `ur_25_switch_projects_timed_updates_in_effective_order` | P279 | configure が origin と同じ instant の timed を落とす |
| 10 | `ur_25_the_line_lock_is_free_inside_every_receive_device_call`（lib） | P269・P270 | `begin` の device call の間（P269），cut の stop の間（P270）に line の lock を持つ |
| 10 | `ur_25_the_line_lock_is_free_inside_every_control_device_call`（lib） | P271 | from-none の open/configure の間（drop を外す） |
| 10 | `ur_25_the_line_lock_is_free_inside_every_transmit_device_call`（lib） | P272 | switch の device call の間 |
| 11 | （既存のテスト） | P273〜P277 | spawn/join の名前（P273〜P275），control 以外の panic が loss にしない（P276），join の timeout を記録しない（P277） |
| major 3 | `ur_17_a_stream_s_prune_leaves_the_other_stream_alone`（lib） | P278 | 1 stream の prune が全 line を畳む |
| 実装で増えた経路 | `ur_15_a_second_stream_is_refused_rather_than_half_started`（lib） | P280 | 列の長さ 1 を要求せず先頭だけを使い，残りを黙って捨てる |

観測の要点:
- item 2 の T1 は両 line の `items` の `Kind::End` を数え，同じ `(e, seq, abort)` であることと，2 回目の `end` が `seq` を進めないことを見る．T2 は Manifest の `timing` の `stop` 行 1 本，`rejected` に "did not join" が無いこと，`rx_stop` 1 回．T3 は `Gated` wrapper で `rx_recv` と `tx_send` を止め，両方が gate に入ったことを確かめてから `finish()`（30 s の `bounded` は watchdog）を流し，`rejected` の "did not join" の `thread` が `["uhd-tx", "uhd-rx"]` であることを見る（Sol の minor 1）．gate はテストの最後に開き，thread を解放する．
- item 3 は message・termination・`mark_lost` の位置を見る（Sol の major 1 により TX clock の終わりは求めない）．TX line の End は item 4 の unit が見る．
- item 5 の 3 テストは，拒否した側だけが halt し，他方の stream の items・plan・clocks が前後で同じで，timed update が他方では発行され，拒否した側では "no channel" で記録されることを見る．T2/T2b は `FakeFault::WrongRate` で owner の configure を失敗させる．
- item 6 の T1 は 0 への disable で 2 本目の clock が登録されず 1 本目が cut で終わること，burst が admission で SC-23 になること，再 enable の origin が cut + tx start lead 以後であること，`tx_open 0 []` が無いこと，2 本目の burst が新しい clock の domain に載ることを見る．T2 は `tx_open 0 [0]` と `apply tx 0 rate=1000000` が各 1 回であることを見る（R5 の killer）．
- item 9 の T1 は両順序で，switch の configure が等号 instant の timed 値（`freq=2100000000`，`at=-`）を運ぶこと，release が `at` で 1 回あること，`LATE_COMMAND` が無いこと，`applied` の `issued` 行が `at` であることを見る．T1(c) は timed を先に booking した場合だけ enable の configure に載り，逆順では release が届けることを見る．T2 は instance の `seq` が booking ごとに進むこと，held の instant が cold=300 ms／timed=100 ms の対照を含めて動かないこと，plan の projection を白箱で見る．T3 は方向をまたぐ device queue の順（rx の apply が 10 ms，`LATE_COMMAND` 1 件）を見る．
- item 10 の 3 テストは `LockSpy` で `rx_open`・`apply`・`rx_start`・`rx_recv`・`rx_stop`／`tx_open`・`tx_send`・`tx_async`・`apply`／`rx_open`・`tx_open`・`apply` が呼ばれたことを確かめ（spy が一度も呼ばれずに緑にならない，Sol の major 4），lock を持ったままの call を assert で落とす．rig は単一 thread なので `try_lock` の失敗は呼び手自身の保持を意味する．tx のテストはテスト自身の `tx_open` を含むので，`tx_open == 2`・`apply == 2` を数える（テストの呼び出しで自明に通らない）．
- major 3 のテストは rx の 2 つの change のうち先のものを prune し，rx の item と configuration が回収され，tx の item・configuration が残り，tx の command をまだ refuse できることを見る．
- 実装で増えた経路（`start` の列の長さ）は，`Prepared` を手で組んだ unit test で `rx`/`tx` の 2 本と 0 本を試し，`Rejected` と「thread が 1 本も起きない」ことを見る．

## 4. 当て直した行

`old` が一致しなくなった 19 行を，同じ規則を同じ形で壊すように当て直した（`new` の意味は変えていない）:

| 行 | 変わったところ |
|---|---|
| F40（maintenance） | `line(...)` の引数が `direction` → `key.0`（start lead を落とす規則は同じ） |
| G60 | `stop_at_cut` に `k` が入った（`if !stream.stopped` → `if false` の形は同じ） |
| J15 | `refuse` の引数が `Dir::Rx` → `key` |
| J16 | 〃 `Dir::Tx` → `stream_key(Dir::Tx, self.k)` |
| J21 | `self.tx_clocks` → `state.clocks` |
| J49・J50 | `streams.prune(Dir::*, by)` → `prune(key, by)` |
| J51 | `self.configs[dir]` → `self.stream_mut(stream).configs` |
| J52・J53・R12 | `stream.follow(&streams)` → `follow(&streams, key)` |
| J62 | `self.changed(core, dir)` → `changed(core, stream)` |
| J64 | `lines[dir]` → `line(booked_stream(dir))` |
| U22（phase7） | `streams.tx_clocks.last()` → `streams.stream(stream).clocks.last()` |
| R12 | 上の follow の形と `rx_stop` の stream 番号（`0` → `self.k`） |
| R13 | `stop_at_cut(&self.core, cut)` → `stop_at_cut(&self.core, self.k, cut)` |
| B01 | `rx_recv(0, …)` → `rx_recv(self.k, …)` |
| B02 | `stop_at_cut` の `k` |
| B20 | 2 つの `tx_send(0, …)` → `tx_send(self.k, …)`，block の順序を入れ替える形は同じ |
| B28 | `tx_send(0, &tail, …)` → `tx_send(self.k, &tail, …)`，`.map(|_| 1)` は同じ |

全 6 リスト 1039 行（maintenance 663，phase3 83，phase4 34，phase5 29，phase6 63，phase7 167）で `old`（あれば `old2` も）がちょうど 1 回一致することを機械で確認した．

## 5. 確かめること 1〜4 の結果（最終ツリーで実行）

| # | コマンド | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | 65 suite すべて ok，1299 passed / 0 failed（最後の編集の後の最終ツリー） | `/tmp/opencode/check1d.log`（途中の run は `check1.log`・`check1b.log`・`check1c.log`） |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | warning/error 0（最後の編集の後の最終ツリー） | `/tmp/opencode/check2d.log`（1 回目 `check2.log` で test の clippy 3 件を直した） |
| 3 | `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problem（最終ツリー） | 端末 |
| 4 | `REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8 <ids>` | 58 行すべて killed（新規 P242〜P280 の 39 行と当て直し 19 行） | `mutate-1b4b-2.log`（51/56），`mutate-1b4b-3.log`（5/6），`mutate-1b4b-4.log`（1/1），`mutate-1b4b-5.log`（3/3） |

item 4 の run の内訳: run 2 は新規 P242〜P278 と当て直し 19 行の 56 行で 51 killed．残った 5 行は，P264 が SURVIVED（§2 の 4），P265・P269・P270・P272 が COMPILE ERROR（P265 は `..e` が tuple key の `range` にならない → `..(e, 0)`；P269/P270/P272 は `let _held = lock(&self.core.streams)` が `self` の不変 borrow を guard の寿命だけ延ばし，`&mut self` と衝突 → `let _core = self.core.clone(); let _held = lock(&_core.streams);`）．直したあと run 3 で P265・P269・P270・P272・P279（新規）が killed，run 4 で P264 が killed，run 5 で P246（テスト名の変更）・P272（テストの assert の変更）・P280（新規）が killed．tx の lock のテストは，gate の前で `reports` を読むと device burst が枯れて `abandon` が EOB を送ってしまうため，`reports` を `do_switch` の後に移した（テストの観測は変えていない）．

## 6. セルフレビュー（AGENTS.md §9 の 5，Haiku 5.5，read-only）

- **1 回目**（`selfreview-1.log`）: P1 なし．P2 1 件: `start` が列の先頭だけを使い残りを黙って捨てる（裁定の「thread は列の各 stream に 1 本」に反する潜在の失敗）．P3 1 件: Changes 行が変えていない UR-14・UR-16 を挙げている．→ どちらも直した: `start` は列を長さ 1 の配列に `try_from` で取り，他は `Rejected`（§2.2）．Changes 行の規則は本文を変えたものに直した．この修正で新しい失敗経路が 1 つ増えたので，unit test（`ur_15_a_second_stream_is_refused_rather_than_half_started`）と行 P280 を足した（§3 の最後，AGENTS.md §9 の 4）．
- **2 回目**（`selfreview-2.log`）: P1 なし，P3 5 件と確認 1 件．処理:
  1. panic のテスト名が「両方の stream を終える」と主張するのに tx 側を assert しない（Sol の major 1 で assert は足せない）→ 名前を `ur_16_a_panicking_thread_ends_a_two_stream_run_with_its_message` にし，行 P246 の filter も直した（Run が終わることを名前にした）．
  2. tx の lock テストの `tx_open` が「テスト自身の呼び出し」でも満たされる → `tx_open == 2`・`apply == 2` を数える形にし，`tx_send >= 3`・`tx_async > 0` と並べた（自明に通らない）．
  3. 報告 §2 の `start` の記述が古い → この報告を `try_from` の形に直し，新しい経路のテストと行を足した（上の 1 回目の処理と同じ）．
  4. Changes 行が UR-25・UR-29 を挙げるが本文が変わっていない → 規則の列挙を UR-3・UR-26・U2・§3 に直した（per-stream の機構は記述に残した）．
  5. UR-26・UR-16 の Test 行が新しいテストを指していない → 両方の Test 行に名前を足した．
  - 確認事項（`rx_packet_samples(0)` の条件付きの裁定）: §1 の control.rs の行に理由を書いた（final (a) のとおり）．
- **3 回目**（`selfreview-3.log`）: P1 なし，P3 4 件．処理:
  1. 規範文の `(design 33 D15)` を Changes へ移すべき（DA-1）→ 退けた: 同じ `design 33 D…` の参照は同じファイルの既存の規範文に複数ある（UR-2 の trait 一覧の `design 33 D16`，UR-5 の本文と selector 表の `design 33 D5`，§2・Depends on の `design 33 D5`）．1b-4a と文書監査を通った形と同じで，新しい理由・履歴を規範文に足してはいない．参照を一掃するなら，UR-25 の本文を stream ごとの番号に直す 1b-4c で行う（§7 の 8 に足した）．
  2. `control.rs:195` の `rx_packet_samples(0)` を `booked_stream(Dir::Rx).1` にするかコメントを足すべき → 退けた: 裁定の final (a) のとおり 0 のままとし，理由はその場のコメントに書いてある（「the call term is the receive line's, stream 0's, the one line the current keys have; 1b-4c reads each stream's own packet size」）．§1 にも書いた．
  3. UR-25 の Test 行に新しいテストが無い → 直した: 規則「device の呼び出しの間に line の lock を持たない」を固定する 3 つの lock テストを UR-25 の Test 行に足した（1464 文字，`rules.py` の 1500 文字の上限内）．拒否の 2 テスト（`ur_25_a_refusal_leaves_the_transmit_stream_alone`・`…receive_stream_alone`）は同じ行に収まらないため 1b-4c で行う（§7 の 8）．
  4. 増えた経路（`start` の列）を計画ファイルに足すべき（§9 の 4）→ 直した: `test-plan.md` の末尾に「実装で増えた経路」の節を足した（テスト名と行 P280）．

## 7. 1b-4c へ送るもの

1. **thread の名前と join**: `Running.threads` は `(&'static str, JoinHandle)` で，`join` は名前で 1 本しか引かない．stream ごとの名 `uhd-rx-s<k>`・`uhd-tx-s<k>` と，同名の全 thread を join する形は 4c．`rejected` の `thread` の形（UR-16）も同時に決める．
2. **command の宛先**: `booked_stream(dir)` は stream 0 を返す．D14 の `<r>/rx/s<k>`・`<r>/tx/s<k>` の宛先，`Control` の stream ごとの channel，`rx_packet_samples(k)`（`Control::new` の `call`），`stop`/`start_rx` の宛先，`planned(k)` の選択は 4c．
3. **node と event の持ち主**: `rx_id`・`tx_id` は方向ごとに 1 つ．`rx/s<k>`・`tx/s<k>` の node，SampleClock の stream id，`RX_OVERFLOW` の handle，`LATE_COMMAND`・`ALIGNMENT_ERROR` の source は 4c（final (c) の 2・3）．
4. **`Streams::end` の複数 line**: 全 line に 1 回の End は実装した（4b）．複数 stream での検証（同じ `seq` を全 line に付ける形，2 本目の line の cut と clock の終わり）は 4c の VH-8 harness（final (c) の 4）．
5. **channel の並び**: `Channels::first(n)`（`0..n`）と `Planned::channels()` の数だけを返す形は 4c の layout の channel 列（RM-27）に置き換える（final (c) の 9）．
6. **`delivered` の床**: 床は `StreamState.delivered` に移したが，「line と床の組が同じ stream」である正しさは 1 本では見えない（item 4 の unit が両 line の End を見るだけ）．2 本目の受信 stream で確かめる（final (c) の 5）．
7. **`Lines::prune_stream` と RM-34**: move を booking する Provider は `Lines::prune`（両 line）で畳む．UHD が move を入れる 1b-4d に見直す（doc comment に書いた）．
8. **設計の呼び出し形**: `rx_start(0, T0)`・`rx_recv(0, …)`・`rx_packet_samples(0)` と UR-26 の join の列挙は，今の 1 stream の番号 0 としては真のまま．4c が stream ごとの番号と名前を入れるときに UR-15・UR-17・UR-25・UR-26 を直す（Changes の 2026-10-18 行に書いた）．あわせて UR-25 の Test 行の残り（拒否の 2 テスト）と，規範文の `(design 33 D…)` の参照を Changes へ移すかをそのときに見る（セルフレビュー 3 回目の P3 への回答）．
