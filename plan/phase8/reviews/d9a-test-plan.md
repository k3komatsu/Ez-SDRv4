STATUS: plan

# D9a：試験計画（改訂 3）— R1（受信の周波数）と R5（FakeDevice の DSP FIFO）

対象は `d9a-brief.md` §2 の範囲。仕様は `note42-draft.md`。読んだ HEAD は 7c0de6a（`git show HEAD:<path>` のみ。tree には触れていない）。コードは書いていない。cargo も流していない。初版は `/tmp` に退避しただけで、この file を置き換えた。

記法：**VERIFIED** = HEAD のコードか UHD の source を読んで確かめた。**INFERRED** = 読んでいない、または推論。`control.rs:N` などは HEAD の行（一部は「付近」。実装前に確かめる）。

## 改訂 3：Sol の 3 回目（FAIL、major 1。orchestrator の決め）

最初の segment（`by == None`）は prepare が configure 済み（`provider/mod.rs:535-543`）で、`start` は未来の T0 を受け付けて T0 の前に control thread を起こす（`:611-619,665,696-707`）。`planned()` は `by == None` に settings を付けない（`core.rs:582-590`）ので、「deferral して configure が運ぶ」は `now < e < T0` の値を落とす。**決め：最初の segment は定義により configure 済みとして扱い、宛先が最初の segment の値は release の時に untimed で当てる**（T0 の前は何もその channel を流さない）。判定は §3 の「印」の 1 行で読む（`P'.settings` が無い＝prepare が configure した最初の segment は configure 済み）。道は増えない。固定試験 M6 を足し、§9 の `by == None` の INFERRED を削除した。

## 改訂 2：Sol の再 review（FAIL、major 2、minor 3。orchestrator は全部採用）

前回の Major 2・4、Minor 1・2 は解消、Major 1 の設計は維持（Sol は「他 stream が channel を流している最中に from-none が configure する」反例を HEAD で見つけていない）。

| 指摘 | 本計画の扱い |
|---|---|
| 設計の根拠の言い方が不正確（「configure は必ず `await_release` の後」は誤り：`owner_configure` が待つのは `Owner::Open` かつ競合がある場合だけ：`core.rs:1245-1275`） | §3 の理由を直した。guard が要らない根拠は `held_elsewhere` と `needed_before(.., i64::MAX)`（from-none：`control.rs:957-959`）、owner の `Configure`/`Nothing` の step は streamer が同じ set で開いたままなので `held_elsewhere` が検出する、の読み |
| Major 1：R5m の「最初の host sample は 3 つ目の値」が模型と矛盾（最初の host sample の timestamp が start なら、その tick の値は 1 つ目） | R5m を作り直した：3 つの連続 tick の履歴、3 つ目の値は start+2 以後の明示した観測点、host sample は timestamp と期待値の対応（start の sample は 1 つ目、start+200 の sample は 3 つ目）。「最初が 3 つ目」を削除 |
| Major 2：P4-B に AUTO/AUTO の DSP 変更を確認する非破壊の観測が無い（`tune_dsp` は MANUAL で書き換える。`apply` は `uhd_tune_result_t` を返さない） | probe 専用の `UhdDevice::tune_auto`（`tune_dsp` の隣、`uhd.rs:518-551`）を指定した（§7。**src の変更が要る**。`Device` trait には入れず production は呼ばない）。DSP の値が前後で違うときだけ P4-B を有効とし、違わなければ INCONCLUSIVE。条件 C は close の前に未消費の entry が有ることを記録する |
| Minor 1：M4 の「snapshot に入らない値」は作れない（book と configure は同じ `updates` lock） | M4 を実現可能な 2 順序（M4a、M4b）に分け、「snapshot に無い値を失わせる mutant」の説明を撤回。configure の終わりと印の公開の間の穴は M3 |
| Minor 2：宛先 segment と印の有効範囲を決める。`projections()` は全 configs を返す（`control.rs:471-473`）。M5 を探索に残さない | §3 に決めた：宛先 `P'` は `planned()` から毎回選び（origin が e より後、cut が無いか `instant(cut) > origin`、拒否なし）、印は **`P'.settings`（その segment の `by` の `ColdConfig`）だけ**を読む。M5 を固定試験 M5a（後の cut が configured segment を消す）と M5b（move の往復）にした |
| Minor 3：P6 の starvation を device 時刻で確定する | P6：最後に queue した sample の終端を記録し、device 時刻がそれを越えてから continuation を送る。Underflow が出なければ回復は「未検証」と記録 |

## 改訂 1：Sol の review（FAIL、major 6、minor 3。orchestrator は全部採用）

| 指摘 | 本計画の扱い |
|---|---|
| Major 1：`Holder::Owner` は「source の chain が止まった」を保証しない（`owner_configure` は claim の後で `await_release` を待つ：`core.rs:1239-1268`。source cut 100 ms・destination origin 160 ms・e=101 ms を 98 ms に release する例） | 「Owner ⇒ release の時に untimed」を**捨てた**。判定を「その segment の configure が**済んだ**（`ColdConfig` の印。projection の snapshot と同じ `updates` ロックの中で立てる）」だけにした（案 (i) の簡約。§3）。claim は見ない（印が立つ時点で source が解放済みの根拠は改訂 2 で直した：§3）。系列 M1（実際の `wait_release` の gate）、M2、M3 を足した |
| Major 2：fake の遅延評価が `rx_recv` の回数に縛られ、host が読まない間の消費と、後の書き込みが生成済みの sample を変える遡及を扱えない | R5 の消費を**device 時刻と実際の start/stop 区間**で決める（§1）。生成済みの sample の値は変更の履歴から引く（書き込みの後の tick にだけ効く）。試験 R5k（host が読まなくても消費）、R5l（後の更新・configure は生成済みの sample を変えない）を足した |
| Major 3：「1 sample」の rate が違う（DDC は decimation 前の入力 sample、DUC は interpolation 後） | 消費は radio rate の tick（mcr）で 1 tick に 1 つ。host sample とは別に観測する。試験 R5b を作り直し、R5m（decimation 200 で 3 つが連続 3 tick に取り出され、最初の host sample は 3 つ目の値）を足した |
| Major 4：32 と DUC の overflow を正確な契約にしている | 32 は**保守的な fake の上限**で、実機の容量ではない（INFERRED、`axi_tag_time.v` に別の出力 FIFO、value 側に別の保留がある）。DUC の overflow は**抽象の模型（INFERRED）**で、「最後が最新」を hardware の契約にしない。R5i は異なる instant と値を使い、模型が何をするかを MODEL と注記して観測する |
| Major 5：P4 は `tune_dsp` だけで production の AUTO/AUTO の radio queue を作らず、close/reopen も見ない | P4 を 3 条件に分けた：A) DSP-only の対照、B) AUTO/AUTO（DSP の値が実際に変わる target）、C) close/reopen（別条件）。呼び出しの前後の時刻、最後の RX sample、再開後の値を記録する。P4 前の RX Ceiling（系列 6）は**条件付き**と書く |
| Major 6：境界と競合の mutation に対応する観測が無い | 等値境界（`e == origin`、`e == cut`）、判定時に既知の cut と release の後の cut、configure の最中の交差（印を lock の外で公開する mutant を殺す）、TX の timed→untimed→timed→EOB→次 burst を試験にした。§8 に mutation 候補ごとの観測する試験の表 |
| Minor 1：12b は両側が同じ記録処理で、同じ間違いでも通る | 明示した期待行を足した（§2.1 の 12d：stopped、future、refused、stop/loss の `e == S`） |
| Minor 2：`dsp_log` の最初の点を「最初の sample」と同一視しない | 観測は sample の**有効値**（`freq_at`・loopback の sample）にした。履歴は補助 |
| Minor 3：P6 に途中の starvation を、文書に F3′ と README のリンクを | P6 に意図的な途中 starvation と継続を足した。文書に F3′ を現存の制約として、README からの link を足した（§6） |

## 0. 先に結論

1. **R1 の「覆う」**：dir の拒否されていない planned segment `s` で、`s.config.channels` が chan を含み、`s.origin <= e`、`s.cut` が無いか `s.instant(cut) > e`（`cut` は sample 番号：`timeline.rs:182-187`）。`e == origin` は覆う、`e == instant(cut)` は覆わない（`Segment.cut` は「first sample not delivered」）。RX の Stop は `timeline.rs:941-944` で cut になる（VERIFIED）。
2. **`targets()` と `channel_live()` は変えない。** 行・settle・投影は今の `(chans, live)` に依る。R1 は `give()` の中、channel ごとの `is_lost()` と `probe("before_timed_apply")` の後、`device.apply` の直前に、受信の周波数に限って `flows(chan, e)` を見る。偽なら装置を呼ばない。`handed_at = Some(now)`、`released.push(e)` は今のまま（`settle` の Ceiling は `handed_at` で決まる：`control.rs:752-760`）。
3. **渡さなかった値を当てる道は 1 つだけ：** (a) 宛先 segment の configure が**まだ**なら、configure が projection を当てる（今の道。`owner_configure`／from-none）。(b) configure が**済み**なら、release の時に untimed で当てる（§3）。owner が projection をもう一度見直す案は採らない（§3 の理由）。
4. **FIFO は chain（front end）ごと、streamer の close では空にしない**（INFERRED）。
5. **VH-8 の differential は周波数を扱わない**（`generator/mod.rs:261`、`differential.rs:234-236`：`Op::Timed` は `gain_db` だけ）。R1 は検査されない。共有の生成器は広げない（MockRadio／spec 09 に触る。必須にしない）。UHD 内の試験（12b、12c、12d、15）で補う。

## 1. FakeDevice の R5（試験が立つ形で固定）

### 観測（`pub`。`read_back`/`set_time` は `#[cfg(test)]` で `tests/fake.rs` から読めない：`device.rs:779,789`）

| 観測 | 内容 |
|---|---|
| `dsp_pending(dir, chan) -> usize` | chain の FIFO の entry 数（呼ぶ前に device 時刻まで消費を進める） |
| `dsp_log(dir, chan) -> Vec<(tick, freq, timed)>` | 値が変わった点の履歴（取り出しはその入力 sample の tick、untimed は書き込みの時刻）。補助 |
| `freq_at(dir, chan, tick) -> f64` | その tick に chain が持つ値（履歴から引く） |
| `calls()` の `apply` の行 | 今のまま。timed の書き込みが装置に届いたかの観測 |
| sample の値 | RX の loopback の sample。`sample_at` は block の 1 つの値でなく **sample の tick の `freq_at`** を使う |

### 模型

- **FIFO**：`(dir, front end dest)` ごと。`dest` は `commands` が既に使うもの（`device.rs:1089-1092`）。streamer の open/close では空にしない（INFERRED）。
- **timed の freq の書き込み**：radio の in-order queue（`commands`、深さ `command_queue`）に今のとおり積み、同時にその chain の FIFO に `(effective, freq)` を積む。`settle()` は timed の **gain だけ**を当てる。`read_back().freq` は消費済みの値。
- **消費は device 時刻と流れている区間で決める**（`rx_recv` の呼び出しではない）：
  - 受信：各 streamer の「流れている区間」`[start, stop)` を持つ（start は `rx_start` の `at`、stop は `rx_stop` の時刻。tail は `end` までの区間として今のモデルに従う）。区間の中の **radio rate の入力 sample**（mcr の tick。X300 は 1 tick に 1 sample：H1、VERIFIED）ごとに、FIFO の先頭が `e <= tick` なら 1 つ取り出す。k 番目の取り出しの tick は `max(e_k, 前の取り出しの tick + 1, 区間の start)`。decimation 後の host sample とは別。
  - 送信：時刻つきの burst の区間の中の **interpolation 後の radio rate の tick** ごとに同じ（`has_time` が保たれる間だけ。H4、H5）。
  - 進め方は遅延でよい：書き込み・満杯の判定・`read_back`・`dsp_pending`・`rx_start`/`rx_stop`・`rx_recv` の前に必ず device 時刻まで進める（host が読まなくても消費され、満杯の判定が実機と同じ向きになる）。
  - 生成済みの sample は変更の**履歴**から値を引く。後の書き込み（tick が後）は過去の sample を変えない。
- **受信の満杯**：進めた後に FIFO が **32**（保守的な fake の上限。実機の正確な容量ではない：INFERRED、Major 4）なら、次の timed の freq の書き込みは `Err`（文言は `timed_out()` と同じ。100 ms は sleep しない）。FIFO は 32 のまま。untimed の freq の書き込みも同じ chain で `Err`（shakeout-2 で時刻なしの書き込みも詰まった：VERIFIED。gain だけの untimed は通す：INFERRED）。
- **送信の満杯**：**抽象の模型（INFERRED、Sol：時刻と値の対応が崩れる）**。32 のとき、新しい entry は時刻を捨て、その値は FIFO の最後の entry の値を置き換える。error にしない。「最後が最新」を hardware の契約にしない。試験は MODEL と注記する。
- untimed の書き込みはすぐ当たり、待っている timed の値が後の入力 sample で上書きする（H2）。
- DUC の stale header（F7）はモデルしない（P5 の後）。

## 2. 系列と試験

置き場：**D** = `src/device.rs` の単体テスト（`ur_33_…`、manual clock の participant）、**C** = `src/provider/control.rs` の単体テスト（制御時計）、**F** = `tests/fake.rs`（`Direct` rig、`direct.device`）。時刻に依る試験は C か D の制御時計で決定的にする。F は pop の判定が device の tick だけで決まる形に限る。

### 2.1 R1

| # | 試験名 | 置き場 | 手順と assert | 観測 | 落とす誤実装 |
|---|---|---|---|---|---|
| 1a | `ur_24_a_stopped_receive_channel_is_given_no_timed_frequency` | C | Stop（cut）の後の e に timed の周波数 40 個（instant を越えて 1 つずつ release）。timed の `apply` 0、`dsp_pending==0`、行は 12d の期待、`COMMAND_REJECTED` 無し | calls、FIFO 数、行 | `flows` を見ない（経路 A）。`live` の意味を変える |
| 1b | `ur_24_a_stopped_receive_channel_still_takes_its_gain` | C | 同じ channel の timed の gain | calls に gain の timed が 1 | R1 を gain に広げる |
| 1c | `ur_24_a_restart_after_deferred_frequencies_starts_on_the_last_one` | F | Stop、40 個（3 ms 間隔）、`start_rx`、`rx_recv` が回る。assert：restart の**最初の sample の有効値**（`freq_at(origin)` と loopback の sample）が最後の値、timed の取り出しが無い、`dsp_pending==0`（Minor 2：履歴の最初の点でなく有効値） | sample の値 | 渡さない値を configure が運ばない。FIFO 経由で後の sample で変わる |
| 2a | `ur_24_a_frequency_for_a_far_planned_segment_is_deferred_to_its_configure` | C | 遠い origin の cold。configure はまだ（試験が `plan[k].configure` を呼ぶ形：`control.rs:4306` の形）。40 個。timed の `apply` 0、`dsp_pending==0`。configure の untimed の `apply` に instant 最大の値 | calls | 経路 B を残す。configure が最後の値を運ばない |
| 2b | `ur_24_deferred_frequencies_take_the_last_by_instant_not_by_arrival` | C | 3 個を instant の降順に予約 | configure の apply の値 | arrival 順で最後を採る |
| 2c | `ur_24_deferred_frequencies_survive_a_cold_rate_change` | C | stopped の channel に timed 3 つ、cold の rate 変更、configure | configure の apply に新 rate と最後の値 | cold の segment の configure から値を落とす |
| 4 | `ur_24_a_frequency_inside_a_running_segment_is_given_timed` | C・F | 走る segment の内側。C：timed の `apply` 1。F：`freq_at` が e 以上の最初の入力 sample の tick から変わる | calls、`freq_at` | `flows` の向きの間違い |
| 4b | `ur_24_a_running_stream_takes_many_retunes_without_filling_the_ddc` | F | 40 回、3 ms 間隔。**host の受信を途中で止めて（期限が来るまで読まない）も** `dsp_pending` は 16 を超えず、`device_failed` が無い。assert は消費が device 時刻で進むこと | FIFO 数、event | 取り出しが `rx_recv` に縛られる fake（偽の wedge） |
| 4c | `ur_24_the_frequency_boundaries_at_the_origin_and_the_cut` | C | 4 つ：`e == origin`（覆う：timed で渡る）、`e == origin - 1 tick`（覆わない）、`e == instant(cut)`（覆わない）、`e == instant(cut) - 1 tick`（覆う）。**cut は判定の時に既に予約済み**（Stop を release の前に book）。各々の timed の `apply` の有無 | calls | `<=` を `<`、`>` を `>=`、cut の除外を落とす |
| 5 | `ur_24_a_frequency_just_after_an_origin_is_given_timed_and_follows_the_configure` | C・F | `origin < e <= origin + window`。configure の untimed が先、timed が後。`freq_at` は origin で configure の値、e の入力 sample で timed の値。逆順の変種：timed の後に untimed v1 を書くと、v1 がすぐ当たり、e の入力 sample で timed の値に変わる（H2） | `freq_at`、calls の順 | untimed が待っている timed を消す fake／順を逆にする |
| 6 | `ur_24_a_cut_booked_after_the_release_leaves_the_entry_for_the_next_segment` | C・D | 4 の値を release した**後**に e より前の cut を book（判定の時は未知）。FIFO に 1 つ残り、restart の最初の入力 sample で取り出されて restart の configure を上書きする。**R2 は P4 の後の今の Ceiling。条件付き**：実機で stop が radio の queue の後ろで待つなら起きない（P4 の B 条件で分かる）。4c の「判定時に既知の cut」とは別 | `dsp_pending`、`freq_at` | 取り出しを close で空にする fake。取り出さない fake |
| 7 | `ur_24_each_channel_takes_its_own_deferred_value_in_a_two_channel_stream` | C・F | c0 は走り、c1 は外れている。c0 に 3、c1 に 40 | channel ごとの FIFO と `freq_at` | `(dir, chan)` でなく dir だけで FIFO |
| 12b | `ur_24_a_deferred_frequency_writes_the_rows_a_given_one_does` | C | 同じ booking の列を covered の chain と stopped の chain で流し、行を比べる（対）。**対だけでは不足（Minor 1）**なので 12d を併せる | `rec.applied` | `given` を偽にする、`handed_at=None` |
| 12c | `ur_29_a_deferred_frequency_stops_at_a_loss_like_a_given_one` | C | `probe("before_timed_apply")` で loss（`ur_29_a_loss_published_before_a_release_stops_the_call` の形）。`Handed::Lost`、行無し、held のまま | 行と held | `flows` を loss 確認の前に置く |
| 12d | `ur_24_deferred_frequency_rows_are_the_expected_ones` | C | **明示した期待行**（`{key, claimed, read_back:null, at}` に付くもの）：stopped：`issued:true`、note 無し／future segment（live でない）：`issued:true, note:"no channel"`／拒否で halted：`note:"no channel"`、`issued` 無し／`Provider::stop` が `S`：`e == S` は `handed_at <= S` なら `issued`（settle は `e <= S` を due）、`e > S` は `cancelled:"Provider::stop"`, `cancelled_at:S`／loss が `S`：`e == S` は**取り消し**（loss は `e < S` だけ due：`control.rs:741-777` の `due`）。floor・`late_command` event・queue slot（`booked`）も同じ | `rec.applied` と events | 対の両側が同じ間違いでも通る（12b）を殺す |
| 15 | `ur_26_a_deferred_frequency_settles_like_a_given_one` | C | `handed_at <= S` なら `issued`、`> S` なら `cancelled`（今の `ur_26_…` の周波数版） | 行 | `handed_at` に `None`／`now` の入れ忘れ |
| 14 | `ur_24_a_deferred_value_is_not_applied_after_provider_stop` | C | settle の後に release が来ても `apply` 0（`ended_at` の確認） | calls | 終了後の apply |

### 2.1b configure 済みの宛先と `await_release`（Major 1）

印 = `ColdConfig` の「configure 済み」。configure が projection の snapshot を読む**同じ `updates` ロックの中**で立てる（from-none は uhd-control が configure するので、configure の成功の直後に立てる）。判定は `give` が全 projection の `updates` ロックを取った後に印を読む（§3）。

| # | 試験名 | 置き場 | 系列と assert | 観測 | 落とす誤実装 |
|---|---|---|---|---|---|
| M1 | `ur_24_a_gap_value_for_a_move_destination_waits_for_the_source_to_release` | C（`wait_release` の probe と gate） | **Sol の例**：source stream の cut=100 ms、destination の origin=160 ms（move）。destination の owner は `probe("wait_release")`（`core.rs`、`await_release` の最初の待ち）で止める。gap の値 e=101 ms を 98 ms に release。assert：**`apply` が 0（timed も untimed も）**、印は偽、source の 100 ms 前の sample の `freq_at` は旧値。gate を解く → owner が configure して destination の origin の sample は新値 | calls、`freq_at`（source の sample と destination の sample） | 「Owner の claim ⇒ untimed」。印を見ずに常に untimed |
| M2 | `ur_24_a_gap_value_for_a_configured_segment_is_applied_untimed_at_its_release` | C | from-none で configure 済みの segment（印が立つ）、origin の前の e に 40 個。untimed の `apply` が release の時に入り、最後が最後の値、timed の `apply` 0、`dsp_pending==0`、owner が begin しても `apply` は増えない、origin の sample は最後の値 | calls の列、sample | 例外を作って timed で渡す。何も当てない。owner が configure を走り直す |
| M3 | `ur_24_a_value_booked_just_after_the_owner_configures_is_not_lost` | C | owner の configure の中、`probe("configured")`（configure が返り、`record_configured` の前。**configure の終わりと印の公開の間の穴**）で止める。そこで book と release：印は configure の `updates` ロックの中で既に立っているので untimed で当たる。assert：値が当たる（`freq_at` の最後）。**印を configure の後（ロックの外）で立てる mutant を殺す**（その mutant では印が偽で、deferral になって値が落ちる） | `freq_at` | 印を `updates` ロックの外で公開 |
| M4a | `ur_24_a_booking_during_a_configure_waits_and_the_release_after_it_applies_untimed` | C | owner の configure を `probe("apply")`（`Core::configure` の最初の channel の apply、`updates` ロックを持ったまま）で止める。別 thread の `book_timed` が `ColdConfig::book` の同じロックで**待つ**（gate を解くまで booking が完了しない：assert）。gate を解く：configure は snapshot に無い値を運ばず、booking が完了し、続く release が untimed で当てる。`device.calls()`：configure の untimed の apply が先、release の untimed が後。最終の `freq_at` は booking の値 | calls の順、`freq_at` | release が configure より前に当たる／booking が configure を待たない |
| M4b | `ur_24_a_release_waits_for_a_configure_that_carried_its_booking` | C | booking が configure の前に完了（snapshot に入る）。configure を `probe("apply")` で止め、別 thread の `give` を走らせる。`give` は `updates` ロックで待ち、configure の後に印（真）を読み、冗長でも untimed を 1 回当てる。assert：その untimed の apply が configure の apply の**後**にある。**これは冗長な当て（§3）を pin する試験**で、`give` が印をロックの外で読む mutant（configure の最中の偽の印を読んで deferral し、apply が無い）を殺す。冗長な当てを pin したくなければ、この mutant の行は付けず「等価寄り」と記録する | calls の順 | ロックを取らない判定 |
| M6 | `ur_24_a_frequency_before_t0_for_the_first_segment_is_applied_untimed` | C（制御時計） | `Start{ t0: 100 ms, rx: Some(..) }` の rig（prepare が configure 済み、manual clock は 0 ms）。`now < e < T0`（e=50 ms）の timed の周波数 v1 を book、47 ms で release。assert：timed の `apply` が **0**、untimed の `apply`（`at=-`、`freq=v1`）が **1**、`dsp_pending==0`、record の行は 12d の期待。T0 まで時計を進め `rx_start(T0)` と `rx_recv`：**T0 の最初の sample の `freq_at(T0)` が v1**（v0 ではない） | calls、`freq_at`、sample | `by == None` を deferral にする（最初の sample が v0 のまま）、最初の segment を宛先から外す |
| M5a | `ur_24_a_later_cut_that_removes_a_configured_segment_leaves_no_stale_mark` | C | from-none で `P'`（origin O）が configure 済み（印が立つ）。その後、**O 以前の instant の cut**（Stop か cold）が `P'` を零長にする（`instant(cut) <= origin`：`needed_before` が除く形）。gap の値 e < O を release。assert：`P'` は選ばれず（零長の segment は宛先にならない）、untimed の `apply` が 0、timed も 0、`P'` の古い印が使われない。次の segment `P''`（別の `by`）の configure が値を運ぶ | calls（release の時の apply が 0）、configure の apply | 宛先の選択が零長の segment を含む／古い `ColdConfig` の印を使う |
| M5b | `ur_24_a_move_round_trip_does_not_use_the_first_segment_s_mark` | C | channel c が s0（segment A、configure 済み）→ s1（B）→ s0（A2）と move。B が走り、B の cut（c2）の後・A2 の origin の前の e を、B がまだ流れている時刻（c2 の前）に release。assert：宛先は A2（A は origin ≤ e で選ばれない）、A2 の印は偽（owner が `await_release` の gate で待つ：`probe("wait_release")`）、**release の時の untimed が 0**、B の最後の sample の `freq_at` は旧値。gate を解くと A2 の configure が値を運ぶ。「どれかの projection が configured」を印とする mutant は A の古い印を読んで untimed を当て、B の末尾の sample を壊す | `freq_at`（B の末尾）、calls | 「全 projection のどれか」を見る（`projections()` は全 configs を返す：`control.rs:471-473`）、宛先を `by` でなく stream の最新から選ぶ |

### 2.2 R5（D）

| # | 試験名 | 手順と assert | 落とす誤実装 |
|---|---|---|---|
| R5a | `ur_33_a_timed_ddc_value_waits_for_an_input_sample_at_or_after_its_time` | `apply(Rx,0,{freq},Some(e))`。`dsp_pending==1`、`freq_at` は旧値。流れている区間が e に達すると、**その入力 sample の tick** で変わる | instant で当てる（今の fake）。block の先頭で当てる |
| R5b | `ur_33_a_ddc_takes_one_entry_per_radio_rate_tick` | e より前の entry を 3 つ。取り出しの tick が連続 3 tick（区間の start、+1、+2） | 全部を 1 tick で取り出す |
| R5m | `ur_33_decimation_does_not_change_the_consumption_rate` | rate 1 MS/s（ratio 200）、start = `s`。期限切れの 3 つ（v1、v2、v3）。**履歴**：`dsp_log` が tick `s`、`s+1`、`s+2` の 3 点（v1、v2、v3）。**観測点**：`freq_at(s)==v1`、`freq_at(s+1)==v2`、`freq_at(s+2)==v3`（3 つ目の値は `s+2` 以後の明示した点）。**host sample は timestamp と期待値を対応させる**：timestamp `s` の sample は v1、`s+200` の sample は v3（3 つが 3 tick で消費済み）。「最初の host sample は 3 つ目」とは書かない（filter の過渡も単純に 3 つ目とは同一視できない）。ratio を 1 と 200 で比べ、**取り出しの tick の列は同じ** | 取り出しを host sample で数える（`s`、`s+200`、`s+400` になる） |
| R5k | `ur_33_a_flowing_ddc_consumes_while_the_host_does_not_read` | 区間が流れている間、`rx_recv` を呼ばず device 時刻だけを進める。entry を 40 個（instant は過ぎる）。`dsp_pending==0`、33 個目で `Err` が出ない | 消費が `rx_recv` の呼び出しに縛られる |
| R5l | `ur_33_a_later_write_does_not_change_samples_already_produced` | 区間が流れ、host が読まない間に sample が生成される。その後 untimed v2 を書き、timed v3 を積む。後で `rx_recv`：生成済みの tick の sample の有効値は旧値のまま、書き込みの tick 以後だけ新値 | 現在の `settings` で全 sample を作る（遡及） |
| R5c | `ur_33_a_stopped_ddc_takes_nothing` | entry を積み `rx_stop`、時間を進める：取り出されない。`rx_start` で再開すると区間の start から取り出す | 止まった後も時刻で取り出す |
| R5d | `ur_33_a_full_ddc_refuses_the_next_timed_write` | 32 個（流れていない区間）→ 33 個目が `Err`、`dsp_pending==32`、33 個目の `calls()` の行無し。untimed の freq も同じ chain で `Err`、gain だけの untimed は通る。別 chain は通る。**32 は fake の上限と注記** | 33 個目を捨てる、全 chain を詰まらせる |
| R5e | `ur_33_an_untimed_write_lands_at_once_and_a_waiting_entry_overwrites_it_later` | 2 つの順（untimed→timed、timed→untimed）で `freq_at` を比べる（H2） | 順を逆にする |
| R5f | `ur_33_a_ddc_fifo_outlives_its_streamer` | 積み、`rx_close`、`rx_open`、`rx_start`：entry は残る。INFERRED を固めるだけ（P4 の C 条件で確かめる） | close で空にする |
| R5g | `ur_33_a_timed_duc_value_is_taken_only_while_has_time_holds` | burst を `at=Some` の send、`at=None` の send、**`at=Some` の send**、EOB に分け、e を 3 つ目の send の内側に置く。**途中の timed は `has_time` を戻さない**（H4：subsequent timed inputs do not restore it）：取り出されない。次の burst の最初の timed の send の最初の tick で取り出す。別に、e が最初の send の中なら最初の send の tick で変わる | 途中の timed で `has_time` を復活させる／続きでも取り出す |
| R5h | `ur_33_an_untimed_burst_takes_no_duc_entry` | 時刻なしで始まる burst：取り出さない | `has_time` を見ない |
| R5i | `ur_33_a_full_duc_is_a_model_that_breaks_the_time_value_pairing` | **MODEL（INFERRED）**：33〜36 個を**異なる instant・異なる値**で積む。error にならない。`dsp_pending <= 32`。どの値がどの tick で当たったか（対応が崩れること）を観測する。「最後が最新」を assert しない。spec は Ceiling とし、hardware の契約にしない | error にする、全 entry を落とす |
| R5j | `ur_33_a_fresh_device_has_empty_fifos` | 新しい装置は空（弱い。開き直しの API が無い：INFERRED） | FIFO を static にする |

### 2.3 送信と S2

| # | 試験名 | 置き場 | 内容 | 落とす誤実装 |
|---|---|---|---|---|
| 8 | `ur_24_a_transmit_frequency_booked_between_bursts_takes_the_next_bursts_first_sample` | F | burst 1（60 ms、2 ms の chunk）、gap に e、burst 2。burst 1 の間 `dsp_pending==1`、burst 2 の最初の send（`at=Some`）の最初の tick で変わる（`freq_at`）。P3（bench、VERIFIED）と同じ | gap で取り出す |
| 9 | `ur_25_a_transmit_channel_that_leaves_and_returns_keeps_its_old_timed_value` | C（TX の layout、`device.tx_open/tx_send` 直接） | V1 を gap に timed、burst を出さず channel が離れる、V2（`no channel`）、戻る（configure が V2 を untimed）、burst 2 の最初の timed の send で V1 が取り出される。`freq_at(burst 2 の最初の tick) == V1`、record は V2。S2 の Ceiling | FIFO を tx_close／configure で空にする（fake が実機より緩い） |
| 10 | `ur_24_a_transmit_frequency_inside_a_burst_waits_for_the_next_burst` | F | burst 1 の 20 ms に e。uhd-tx は先頭だけ `at`（`tx.rs:434`、`:442`、`:579` は無し）。burst 1 の間取り出しなし、`calls()` に `at=-` の続き。burst 2 の最初で取り出し。F3′ を今の Ceiling として固める（R4 は D9b で反転） | 続きの send でも取り出す |

## 3. configure 済みの扱い：選んだ設計と理由

**宛先 segment と印（改訂 2 で固定）**

- **宛先 `P'`**：`give` の判定ごとに、`streams` ロックの下で `planned(stream)` を全 stream から読み直し、次を満たす最初のもの：拒否されていない、`config.channels` が chan を含む、`origin > e`、cut が無いか `instant(cut) > origin`（零長の segment は除く）。origin が最小のもの。`P'` が無ければ（Stop した channel で後の segment が無い場合など）untimed も当てない（後の segment の configure か `of_record` が運ぶ）。
- **印**：`P'.settings`（`Planned.settings` = `configs[segment.by]` の `ColdConfig`：`core.rs:582-596`）の「configure 済み」だけを読む。**`P'.settings` が無い（`segment.by == None`、stream の最初の segment）は定義により configure 済み**：prepare が configure した（`mod.rs:535-543`）ので、判定は「`P'.settings.is_none_or(|config| config.configured())`」の 1 行で読む。別の印も別の経路も足さない。T0（= その origin）の前は何もその channel を流さない（`rx_start` は origin）ので、release の untimed は安全。（Sol の 4 回目：`settings` が無いのはもう 1 つ，`by == Some(_)` で configs から prune された過去の segment（`core.rs:574-578`，`rx.rs:355-371`，`tx.rs:199-218`）．これは origin が e より後でないので宛先の候補にならない．未 configure の将来の segment が `settings: None` になる道は無い．）`projections()`（全 configs）は使わない。選び直しのたびに `planned()` を読むので、plan から消えた segment の古い印は見えない。
- **印の意味**：configure が成功した（snapshot をその segment の projection から作った）。**有効範囲**は「`P'` が plan にあり、まだ origin に達していない間」。device の record が後で変わる（再 open で `state.configured` が消える）と印は古くなるが、その間に当てる untimed は idle な chain への冗長な当てで害は無い（次の configure が同じ projection を運ぶ）。

**3 つの場合**

| 場合 | 例 | 当てる道 |
|---|---|---|
| (1) `P'` の configure がまだ | M1（source がまだ流れている間、owner が `await_release` を待つ）、Owner::Open の通常の segment | configure が projection を当てる。release は何もしない（今の道） |
| (2) 印が立ち、source は流れない | M2（from-none、owner の configure の後） | release が untimed で当てる |
| (3) 印が立ち、他 stream の segment が同じ channel をまだ流す | — | **HEAD では見つからない**（下の根拠）。M5b が「古い印を使う」誤実装を殺す |

**選んだ案：(i) の簡約。** 印を、configure が projection の snapshot を読む同じ `updates` ロックの中で立てる。`give` は全 projection のロックを取った後に `P'` の印を読む：偽なら deferral、真なら untimed。plan の区間の guard は足さない。

**理由（少ない道で安全）**

1. (3) が起きない根拠（読み。move の chain の全順序は INFERRED）：
   - from-none：条件に `held_elsewhere(stream, channels) == None` と `!needed_before(stream, channels, i64::MAX)`（`control.rs:957-959`）が入る。他 stream が channel を開いている／origin に関わらず必要としている間は from-none にならない。
   - owner：`owner_configure` が `await_release` で待つのは `Owner::Open` かつ競合があるときだけ（`core.rs:1245-1275`）。`Configure` と `Nothing` の step は streamer が**同じ set で開いたまま**の segment で、他 stream がその channel を保持していれば `held_elsewhere` が検出し（record の set が重なる）、`Open` の step に落ちて待つ。
   - したがって「configure 済みの `P'` の channel を他 stream が走らせている」状態は HEAD の経路では作れない。`await_release` を根拠にはしない。
2. 印をロックの中で立てるので「snapshot に入らず、印も未公開」の隙間が無い。configure の終わりと印の公開の間の穴は M3 が、ロックの待ちは M4a/M4b が見る。
3. owner が origin で projection を見直す案 (ii) は採らない：見直しの後〜`rx_start(origin)` の間（start lead 50 ms）に book された値が落ちる窓が残る。release の untimed なら直ちに当たる。道は release の 1 つ。
4. timed のまま渡す案は、1 つの batch が `command_queue_depth` を越えて FPGA の容量を超えうる（Sol）ので退けた。

冗長な当て：from-none の configure が既に運んだ値を release がもう一度 untimed で当てることがある。同じ値で chain は流れていないので害は無い。`ur_25_an_enable_at_a_timed_update_s_instant_applies_it` の `configured` の数を直す（§4）。

untimed の失敗は timed の失敗と同じ扱い（`device_failed`、`Handed::Refused`、行無し、projection に残る）。試験 3d（下）。

| # | 試験名 | 内容 |
|---|---|---|
| 3d | `ur_24_a_refused_untimed_release_is_a_refusal_like_a_timed_one` | M2 の `apply` を失敗させる（`DeviceFault`/`RxApplyFails`）。`device_failed` の event 1、行無し、projection に残る |

## 4. 壊れる既存のテスト（HEAD の grep から。実装後に cargo で確かめる）

分類：**R1** = production の振る舞いが変わった、**緩い** = テストが fake の緩さに頼っていた、**不変**。読んだものは「読んだ」。

| 場所 | 理由 | 分類 | 直し方 |
|---|---|---|---|
| `control.rs` `ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment`（1751、読んだ） | c1 の pending segment（origin 300 ms）へ e=200 ms の周波数。`apply rx 1 … at=200ms` が 1 を期待 | R1 | rig が from-none なら（INFERRED）untimed の `apply` が release の時に 1。行は変えない |
| `control.rs` `ur_25_the_switch_applies_the_configuration_in_effect_at_e2`（4306、読んだ） | e=320 ms は cut の後・origin の前。release で timed が 1 を期待 | R1 | timed 0。configure は試験が直接呼ぶ（印は立つ）ので、**configure の後に release が来る順なら untimed が 1 つ増えうる**：順を確かめて期待を直す |
| `control.rs` `ur_25_a_timed_update_at_the_cold_change_s_instant_reaches_the_switch`（2732、読んだ） | e=300 ms = cut の時刻（覆わない） | R1 | 同上。`timed_first` の両方で |
| `control.rs` `ur_25_an_enable_at_a_timed_update_s_instant_applies_it`（4340、読んだ） | from-none。`timed == 1`、`configured == usize::from(timed_first)` | R1（冗長な当てを含む） | `timed == 0`。`configured`：`timed_first` 真は from-none が運ぶ分に release の冗長が加わり 2、偽は 1 |
| `tests/fake.rs` `ur_24_a_released_command_is_not_recalled_by_stop`（2502、読んだ） | 周波数 e=20 000 tick、Stop は 19 500 tick（cut が e の前）。timed 1 を期待 | R1（意図が消える） | 意図（released な command は Stop で取り戻されない）を保つため key を **gain** に替える |
| `tests/fake.rs` `ur_24_stop_cancels_held_commands`（2530、読んだ） | e=1 ms、Stop は 10 µs | R1 | gain に替える。50 s の方は不変 |
| `control.rs` `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`（4684、4710/4717/4722 の `read_back(Rx,1).freq`、読んだ） | `read_back` は即時に当たる前提 | **緩い**（R5）＋ R1 の可能性（INFERRED） | `read_back().freq` は消費まで古い。`calls()` と `freq_at` で見る |
| `control.rs` `ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`（4392、読んだ） | c1 が t0 の segment に覆われる（INFERRED） | 不変の見込み | 通ること |
| `control.rs` `ur_24_hardware_timed_updates`、`ur_24_a_far_future_update_does_not_delay_a_nearer_one`（2623、2667、読んだ） | rig の最初の segment が cut 無し（INFERRED） | 不変の見込み | segment が無ければ R1 |
| `control.rs` `ur_24_the_layout_destination_follows_the_effective_membership`（4528） | 読んでいない | INFERRED | 実装後に分ける |
| `tests/fake.rs` `ur_24_an_untimed_retune_is_on_time_after_its_delivery`（2485）、`ur_25_enabling_tx_applies_the_configuration`（2636、loopback）、`ur_26_d14_a_layout_instance_names_its_streams`（5973） | rx が `Direct` で流れていれば不変。loopback は消費の時刻に依る | INFERRED | 実装後に確かめる |
| `tests/common/measurement.rs` の queue probe（`device.apply(.., Some(at))`：1241） | DDC の満杯で rehearsal が `Err`（shakeout-2 の wedge と同じ原因） | **緩い** | rehearsal の期待（`provider_measurement_groups_fake_rehearsal_round_trips_without_claiming_measurements`）を見直す。別の agent が編集中 |
| `src/device.rs` の `ur_33_a_timed_command_follows_its_front_end_across_a_spec`（2220 付近。timed の freq と `read_back`） | freq が `settle` で当たらなくなる | **緩い**（R5） | gain の timed に替える |
| VH-8 の 2 本 | gain だけ | 不変（R1 は検査されない：§0.5） | — |

直す順序：先に fake（R5）→ 落ちる試験を分類 → R1 → 落ちる試験を分類。

## 5. spec 18 の変える規則（文は書かない）

- §2 の事実：H1、H2、H4、H5、H9 を足す。H7（DUC の満杯）は INFERRED と明記し「害が無い」「最後が最新」と書かない。32 は fake の保守的な上限と書く。
- **UR-24**：受信の周波数は、その instant に拒否されていない走行中の segment が覆うときだけ装置に timed で渡す。それ以外は渡さず、記録・行・投影は変えない。configure が**済んだ**宛先 segment の値は release の時に untimed で当てる。
- **UR-25**（296 行の段落）：configure が deferral した値を当てること、configure が済んだ segment への値は release が当てること。印は configure の snapshot と同じ lock の中で立つ。
- **UR-33**：FakeDevice の DSP FIFO（§1。消費は device 時刻と流れている区間、radio rate、DDC は 32 で満杯の timed と freq の untimed が error、DUC の満杯は抽象の模型、streamer の close で空にしない INFERRED、`read_back().freq` は消費済みの値）。
- UR-22：変えない（R4 は D9b）。UR-36：`configured` の mark の意味は同じ。
- 試験の表：570 行、565 行の説明、新しい試験の行。
- 利用者に見える Ceiling は §6。

## 6. 文書の義務（owner 2026-10-27：同じ commit で）

`KNOWN_LIMITATIONS.md`（root、英語）に 3 項目：(1) S2：送信の channel が stream を離れて戻ると、離れる前の timed の周波数が戻った後の最初の burst に当たる（系列 9）。(2) 受信の周波数の後に cut が来ると、残った値が次の segment の最初の sample に当たる（系列 6。**条件付き**：P4 の B 条件の結果で反転しうる。R2 は D9b）。(3) **F3′：送信の burst の途中に e が来る timed の周波数は、LO は e で変わり DSP は次の burst まで古い**（系列 10。R4 は D9b。現存の制約）。各項目：症状、条件、回避、根拠、状態。**README から `KNOWN_LIMITATIONS.md` への link を足す**。`plan/maintenance/tools/rules.py`（`--base` に変更の前の commit）を通す。AGENTS.md の 1 行は owner の指示で足す。

## 7. P4〜P7 の probe（`tests/hardware.rs`、`#[ignore]`、assert 無し、出力だけ）

道具は P1〜P3 と同じ（`raw_uhd`、`tone_1msps`、`tone_freqs`、`median_freq`、`first_steps`、`flush_tx`：VERIFIED）。timed の DSP entry は 8 個以下。**TX の loopback を使うので、実行の前に owner が att ≥ 30 dB が loopback に入っていることを確かめる。** `SAFE_TX_HZ` の `assert!` を P2/P3 と同じに。

| probe | 条件と手順 | 記録 | 決まること |
|---|---|---|---|
| P4 `hw_d9_p4_ddc_entry_after_an_early_stop` | **A 条件（対照）**：`tune_dsp(Rx,0,f1,Some(e))`（DSP だけ）。**B 条件（production 相当）**：probe 専用の `UhdDevice::tune_auto(dir, chan, target_hz, at) -> Result<(f64, f64), DeviceError>`（`actual_rf_freq`、`actual_dsp_freq`）を `tune_dsp` の隣（`uhd.rs:518-551`）に足す。**src の変更が要る**（probe 専用、`Device` trait に入れない、production は呼ばない。`apply` の freq の枝と同じ AUTO/AUTO の `uhd_tune_request_t`、同じ command time の設定と clear、`result` の 2 つの値を返す。`tune_dsp` と同じ「Design note 39's bench probes only」の doc）。手順：まず untimed の `tune_auto` で基準 f0（`dsp0` を記録）、次に timed の `tune_auto(f1, Some(e))`（`dsp1` を記録。結果は呼び出しの時に host 側で決まる：INFERRED）。f1 は f0 から LO の分解能より小さい δ を複数試し（出力に δ と `dsp0`/`dsp1` を残す）、**`dsp1 != dsp0` のときだけ P4-B を有効**とし、作れなければ **INCONCLUSIVE** と出力する。どちらも e の 2 ms 前に untimed の `rx_stop`、e より後の origin で再開。**C 条件（別）**：stop の後 `rx_close`→`rx_open`→再開。close の前に**未消費の entry が有ること**を記録する（stop の device 時刻 < e、最後の RX sample の tick < e、timed の書き込みが e の前に返ったこと：消費されていない証拠） | 各呼び出しの前後の device 時刻、`dsp0`/`dsp1`/`actual_rf_freq`、最後の RX sample の tick、再開後の最初の sample の周波数（`first_steps`）と median | R2 の要否。系列 6 の Ceiling（条件付き）。R5f |
| P5 `hw_d9_p5_a_stale_timed_header_after_a_reopen` | case A：時刻つきの send で burst を終える→装置を drop→開き直す→burst の前に timed の DUC→burst。case B：時刻なしで終える。**同一 process で開き直して stale が再現するかは INFERRED。再現しなければ別 process（環境変数で case）** | tone が動き始める µs（e 基準）、median | F7 |
| P6 `hw_d9_p6_every_chunk_timed` | 60 ms を 2 ms の chunk（2 000 sample）に分け**すべての send に時刻**（`sob` は最初だけ）。DUC を e=start+20 ms に（e の 3 ms 前に渡す変種を 1 つ）。**意図的な途中 starvation**：20 ms の chunk の後、**最後に queue した sample の終端 tick（`start + queued·ratio`）を記録**し、**device 時刻がその終端を越えてから**（`time_now` を polling）continuation を時刻つき（終端の次の sample の時刻）で送る（host の sleep では先行して queue した sample が残り underflow しない）。`flush_tx` で `Underflow` と継続の late packet の drop（`radio_tx_core.v:337-409`）を記録する。**Underflow が観測できなければ、回復の系列は「未検証」と出力する** | tone の移る位置、queue した sample の終端と継続を送った device 時刻、`flush_tx` の report、継続の最初の sample の有無 | R4 の採否。H2（観測できた場合だけ） |
| P7 `hw_d9_p7_the_current_transmit_shape` | 今の uhd-tx の形（最初の send だけ時刻つき、続きは `at=None` の 2 ms の chunk）。同じ DUC の e | 同上 | F3′ の bench での確認（任意） |

fake では走らない。`cargo test -p ezsdr-radio-uhd --test hardware --no-run` が通ること（コンパイルのみ）が確認。mutation の行は付けない。

## 8. mutation の行の候補と、それを観測する試験

行は実装の後に `remote_mutate.py` で一致 1 回を確かめて書く。

| mutation 候補（場所と変え方） | 観測する試験 |
|---|---|
| `give` の `flows` の条件を反転 | 1a、1c、4、4c |
| 受信の周波数だけという限定を外して gain も止める | 1b |
| 覆う判定の `s.origin <= e` を `<` | **4c**（`e == origin`） |
| `instant(cut) > e` を `>=` | **4c**（`e == instant(cut)`） |
| `cut.is_none_or` を落とす／`refused` の除外を落とす／channel の包含を落とす | 4、7、4c |
| `handed_at` を `None` に | 12b、12d、15 |
| `self.released.push(e)` を渡さない枝で落とす | 既存の `control.released.len()` の試験 |
| 渡さない枝の untimed の apply を落とす | M2、2a |
| untimed の apply の `at` を `Some(e)` にする | M2（`dsp_pending`） |
| 印の条件を落として常に untimed | **M1**（source の旧 sample）、**M5b** |
| 印を `updates` ロックの外（configure の後）で公開 | **M3**（M4a の「snapshot に無い値」とは別） |
| `give` が projection のロックを取らずに印を読む | **M4b**（冗長な当てを pin する場合のみ。そうでなければ等価寄りとして行を付けない） |
| 印を「`Holder::Owner`」で代用 | **M1** |
| 印を「どれかの projection が configured」にする／宛先を最新の segment から選ぶ | **M5b** |
| 宛先の選択が零長の segment（cut <= origin）を含む | **M5a** |
| 最初の segment（`settings` 無し）を configure 済みと読まない（`is_none_or` を `is_some_and` に） | **M6** |
| R5m：消費を host sample で数える／tick の列がずれる | **R5m**（履歴と timestamp の対応） |
| `device.rs`：容量 32→33 | R5d（`dsp_pending==32` の assert） |
| 満杯の `Err` を落とす／untimed の freq の満杯の `Err` を落とす | R5d |
| 取り出しの `e <= tick` を `<` | R5a |
| 1 tick に 1 つ → まとめて | R5b、R5m |
| 取り出しを `rx_recv` の呼び出しに縛る | **R5k**、4b |
| 生成済みの sample を現在の値で作る（遡及） | **R5l** |
| `settle()` に freq を当てさせる | R5a |
| `rx_close` で FIFO を空にする | R5f、6、9 |
| TX：`has_time` を見ない／途中の timed で戻す | **R5g**、R5h、10 |
| 満杯の DUC の模型を落とす | R5i（MODEL の観測） |

## 9. 指示書の範囲や系列で疑わしい所

1. **VH-8 は周波数を扱わない**（§0.5）。共有の生成器は広げない。12b・12c・12d・15 で補う。
2. **(3) が起きないこと（§3）は `held_elsewhere`/`needed_before` の読みで、move の chain 全体の全順序は INFERRED。** guard は足さない設計で、M5a/M5b が古い印・零長の segment の誤りを固定の試験で殺す。反例が見つかれば別途 guard を足す。
3. note42-draft §2 の 2 は configure が走り直さない場合の道を書いていない（本計画 M2）。draft の文を直すか尋ねる。
4. 系列 6 の「FIFO に残る」は fake が streamer の close をまたいで FIFO を持つ前提（INFERRED）。P4 の C 条件で反転しうる。
5. P4 の B 条件で「DSP の値が実際に変わる target」を作れるかは INFERRED（LO の分解能に依る）。
6. 受信の FIFO の容量、DUC の満杯、radio rate が mcr（1 tick に 1 sample）であること（X300 の 200 MHz：VERIFIED は mcr = radio rate まで、INFERRED は他の board）。
7. tests の編集の衝突：`tests/fake.rs`、`tests/common/measurement.rs`、`tests/hardware.rs` は別の agent が編集中。直しと probe の追加はその後に 1 人で。
8. `ur_24_a_timed_update_reaches_a_later_stream_s_pending_segment` の rig が from-none かは INFERRED。
9. 系列 8 は burst 2 の最初の send が `at=Some`（`device_sob`）の前提。前の burst の続き（`continuing`、時刻なし）なら取り出されない。gap を取る。
10. 本数：必須の 11 本（M1、M3、M4a、M5b、M6、4c、R5g、R5k、R5l、9、10）を先に、残りは実装役の判断で束ねてよい（2b と 2c は 2a の変種、M2 と 3d は束ねてよい）。
