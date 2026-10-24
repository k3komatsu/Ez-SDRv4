# 設計メモ 37 の 37c：Sol 6.1 のレビュー

## 表のレビュー（FAIL）

判定：FAIL

**推奨：時刻の窓そのものを検査する系列は (4) に移す。記録された `e` への比較の付け替えだけでは、pending、同時刻、prebooked という前提が消える。** S の件数は現状のまま受理できない。

以下、`F` は `crates/ezsdr-radio-uhd/tests/fake.rs`。実行による再現は行っておらず、到達結果まで確定できない箇所は INFERRED とした。

## 1. 変更する行

1. **行48 — `ur_25_a_same_size_membership_swap_reopens_the_streamer`**  
   **`F:4893-4927`：S → (4)。**  
   最初の membership 更新後、次の更新までに `at` を越えると、「同時刻に `{0}` → `{1}`」ではなく「いったん空にして、後から `{1}` を有効化する」系列になる。同じ `at` を渡しても、後の更新は booking 時刻へ floor される。  
   **INFERRED：** 現在の assert はこの別系列でも通り得るため、「集合ではなく個数だけ比較する」回帰を見逃す。単なる最終 membership の確認に弱めず、同時刻の swap を固定する必要がある。

2. **行61 — `ur_34_two_moves_at_one_instant_swap_their_receive_channels_without_deadlock`**  
   **`F:5324-5331`：S → (4)。**  
   1本目の move 後に約300 ms 待たされると、中間の `[0, 1]` を持つ stream が開始できる。2本目は元の `e` ではなく後の booking 時刻になり、追加の SampleClock が登録される。`until` の「両方ちょうど2個」は、その状態を通り過ぎて待ち続け得る。  
   「両方が同じ `e` を使う」は requested instant の説明であって、effective instant の保証ではない。

3. **行62 — `ur_34_two_moves_at_one_instant_swap_their_transmit_channels_without_deadlock`**  
   **`F:5349-5354`：S → (4)。**  
   1本目と2本目の間に `e` を越える遅れがあれば、中間の `tx_open 1 [0, 1]` が可能になる。これは5354行の禁止条件そのもの。最終 channels が正しくても、この assert は落ちる。

4. **行65 — `ur_34_a_finite_middle_segment_releases_a_channel_along_a_move_chain`**  
   **`F:5504-5529`：S → (4)。**  
   `e1` を読んだ後、最初の投入前に350 msほど遅れると、`e1` と `e2=e1+150 ms` は両方過去になる。2つの effective instant の間隔は、もはや150 msではない。  
   **INFERRED：** B の origin より先に cut が来て、B の clock／block を必須とする5508–5509行、5522–5529行が成立しなくなる。「有限の中間 segment が実際に受信する」前提を制御時計で固定する。

5. **行66 — `ur_26_d14_a_layout_instance_names_its_streams`**  
   **`F:5574-5588`：S → (4)〔held burst と Stop の部分〕。**  
   burst を入れて `settle(5 ms)` した後、`sent` を読む前に100 ms遅れると、payload と EOB は既に送信済みになり得る。その後に RX Stop を出しても、新しい TX は増えないため、`sent + 2` と `> sent` が落ちる。  
   Node の宛先確認は残せるが、「Stop の時点でまだ held」という性質は別に固定する必要がある。

6. **行86 — `ur_21_a_burst_targets_its_own_transmit_stream`**  
   **`F:6663-6679`：S → (2)。**  
   最初の stream 0 の burst の時刻計算から投入まで40 ms以上遅れると、`SendAsapAndFlag` により stream 0 の `TIME_ERROR` が出る。6679行は蓄積した **全件** が stream 1 由来であることを要求するため落ちる。  
   意図した late burst の直前に event の境界を記録し、その Action／以後の event を検査する。asap は送信を保証しても、「余分な event が出ない」は保証しない。

7. **行74 — `ur_25_a_transmit_refusal_from_no_stream_starts_no_clock`**  
   **`F:6105-6121`：S → (4)〔未来の refused origin より前に次の enable を入れる性質〕。**  
   `settle(30 ms)` 後に200 ms遅れると、2本目の enable は最初の origin より後になる。「origin を待たずに次を処理する」という検査ではなくなる。  
   **INFERRED：** origin まで不当に待つ実装でも、その待ちがテストの停止中に終わり、拒否2件などの assert は通り得る。通常実装の最終結果が同じ、という理由だけでは S にできない。

8. **行47 — `ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance`**  
   **`F:4838-4864`：(2) → (4)。**  
   最初の投入前に50 ms以上遅れると、最初の変更自身も late になり、`late.len()==1` が崩れる。最初の変更が実行された後まで2本目が遅れれば、今度は「先行する pending cold が floor」という前提が消える。  
   `e` 同士を比較するだけでは、pending floor を無視する回帰を固定できない。

9. **行60 — `ur_34_a_cold_change_before_the_move_s_release_is_floored_at_the_move`**  
   **`F:5298-5306`：(2) → (4)。**  
   move 投入後に100 ms以上遅れると、後続 cold は move の release 前ではなくなる。`applied` を記録済みの値に付け替えても、move による floor と「単に現在時刻へ floor」の区別がなくなる。

10. **行63 — `ur_34_a_transmit_move_ends_the_old_burst_and_sends_the_new_burst_on_its_stream`**  
    **`F:5383-5415`：(2) → (4)。**  
    最初の target の遅れだけではない。old sample を観測した後に数十 ms遅れると、`e=old_start+10 ms` は過去になり、`old_end==e` が崩れる。さらに、新 clock の観測後に遅れれば、`origin+10 ms` の新 burst も移動する。  
    最初の burst の記録時刻へ基準を替えるだけでは、cut、held tail、新 burst の全系列を守れない。

11. **行64 — `ur_34_a_transmit_move_carries_a_prebooked_burst_across_the_repeat_cut`**  
    **`F:5453-5469`：(2) → (4)。**  
    wrap の観測後に約300 ms遅れれば、5456行の `now < e` が直接落ちる。また、新 clock を読んでから投入するまでの遅れで、新 burst は「cut 前に予約済み」ではなくなる。  
    時刻の付け替えで後から送るテストにすると、prebooked という検査対象を失う。

12. **行75 — `ur_25_a_floor_raised_layout_join_is_judged_at_its_instant`**  
    **`F:6151-6176`：(2) → (4)。**  
    最初の antenna 更新と join の間に50 ms以上遅れると、antenna は既に effective であり、join が pending floor によって上げられる場面ではなくなる。後半の `rows[1]["e"]` だけ直しても、前半の性質は固定されない。

13. **行87 — `ur_26_two_receive_streams_end_at_their_own_stops`**  
    **`F:6715-6736`：(2) → (2)＋(4)。**  
    Stop 前の時刻を receipt の記録へ替える修正は妥当。ただし、別の窓が残る。stream 1 の終了を観測してから Abort までに50 ms以上遅れると、stream 0 の次の packet が届き得る。  
    **INFERRED：** delivered floor を誤って共有・消去する回帰が、その次の packet によって隠れる。通常の end-to-end 検査は (2)、この packet 間の性質は (4) に分ける。

14. **行59 — `ur_34_a_timed_update_crossing_the_move_reaches_each_streamer_channel_at_its_instant`**  
    **`F:5257-5264`：(4) → (4)、ただし理由と対象系列を追加。**  
    booking の遅れだけでなく、`wait_until(before+10 ms)`／`wait_until(after−10 ms)` の戻りから `device.apply(..., None)` の read-back までにも窓がある。後者の後に20 ms遅れれば、期待する10 MHzではなく20 MHzを読む。投入を開始前に予約するだけでは直らない。

**表の補足修正：**

- **行70、行84** の (2) は可能だが、提案が不十分。`F:5919`、`F:6630` の `LATE_COMMAND` 空も処理する必要がある。記録時刻への比較変更だけでは落ち続ける。未来時刻が性質でなければ `at=None` などで窓をなくし、未来時刻の厳密性を残す部分は (1)/(4) に分ける。
- **行2、行5** は README が明示的に残す実時間診断であり、削除・移動を要求しない。ただし「遅れに依らない S」とは別枠にする。`F:283-284`、`F:438-439` の間の遅れは測定値に混入する。中央値は影響を減らすだけで、独立性の証明ではない。
- **行7、行8** は helper 自体の説明に限定する。`settle` は静止点ではなく、`until` の条件もすべて単調ではない。行61の「clock 数がちょうど2」が反例になる。

## 2. 欠けている系列と検索方法

grep リストに加え、対象 Rust ファイルで次を検索し、helper の呼び先まで追った。

- `advance_to|wait_until|\.now\(|time_now\(|Instant::now|elapsed\(|thread::sleep`
- `try_recv|is_empty\(|is_none\(`
- `send`、`send_at`、`tx_at`、`burst_in`、`wait`、`past_t0`、`past_tx_origin`
- Python：`monotonic|perf_counter|time\(|sleep`
- server：`advance|now|sleep|elapsed|Instant|recv_timeout|try_recv`

追加すべきものは以下。

1. **`test_v54_sleep_is_run_time`**  
   **`python/tests/test_easy_api.py:296-304`：未掲載 → (4) 相当の決定的検査＋性能測定の分離。**  
   `began` から `elapsed` までのテストプロセスの停止時間が、実時間10 s未満という判定に含まれる。仮想時刻の差が10 sであることは決定的に検査できるが、性能の上限は別物。上限を延ばすだけでは解決しない。

2. **`Direct::start`**  
   **`F:731-735`：未掲載 → (4) の setup として明示。**  
   helper 内で `now+2.001 s` を作ってから `radio.start` を呼ぶ。通常の数百 msの遅れには余裕があるが、「どれだけ遅れても S」ではない。制御時計へ移すテストでは、開始処理もその時計の下へ移す必要がある。

3. **`ur_07_callbacks_run_outside_the_schedule_lock`**  
   **`F:368-402`：未掲載 → S。**  
   時計を読み、別 thread から追加予約する系列がある。過去の予約を許す規則と handshake で判定するため、短い窓に依存しない。

4. **`ur_25_a_receive_session_follows_its_recorded_plan`**  
   **`F:1917-1966`：未掲載 → S。**  
   `wait` が `after` → `run.now()` を読む。clock が付かない segment を許し、記録済み plan と clock の対応を検査しており、この扱いを一覧に残すべき。

5. **`ur_23_a_stop_ends_the_burst_within_the_in_flight_window`**  
   **`F:1574-1599`：未掲載 → S〔37aで修正済み〕。**  
   `send` → `tx_at`、`wait` → `after` の間接読みがある。現在は Stop の ledger 時刻を使い、上限の窓を unit test へ分離している。helper の定義だけではなく、この呼び側も一覧に必要。

6. **server の device-paced 系列が丸ごと未掲載。少なくとも以下を追加する。**
   - `capture_at`／`status`：`crates/ezsdr-server/tests/protocol.rs:763-781` — 呼び側別に判定。
   - `ea_12_a_duration_is_whole_root_ticks`：同`:797-809` — **S**。返された horizon と now の関係。
   - `ea_07_a_session_on_the_fake_device`：同`:828-844` — **S**。繰り返し波形の offset を探索する。
   - `ea_17_a_capture_ahead_starts_at_its_instant`：同`:1089-1104` — **S**。37aで `first >= requested` に修正済み。
   - `ea_17_a_capture_at_a_passed_instant_says_where_it_started`：同`:1108-1119` — **S**。遅れは「過去」という条件を強める。

## 3. §3 の1 tick不足について

**再アンカーと整数丸めによって差が1 tick短くなる説明は成立する。しかし、単調 guard の説明は誤りであり、「本番の誤りではない」という断定と修正候補の並べ方は強すぎる。**

- `authority.rs:142-147` と `device.rs:762-771` から、default の drift・offset では、外挿は装置の真の整数値以下になり得る。**古いアンカーの floor の和が真値より1 tick高くなることはない。** `last` の最大値を取っても、この前提では真値を越えない。表の166行の説明は削除する。
- 再アンカーで丸めの位相が変わる説明は成立する。例えば200 MHzで、host 時刻を ns で表すと：
  - 古い anchor が0、`e` の読取りが5：`e=1 tick`
  - 新しい anchor が20,000,001、`ready` の読取りが80,000,005：外挿値は16,000,000 ticks
  - 差は15,999,999 ticksだが、host 上の経過は正確に80 ms。
- `host.rs:307-314` は sleep の期限を現在時刻＋duration にするため、この算術例に早起きは不要。実際の失敗がこの再アンカーだったかは、ログがないため **INFERRED**。

**性質を守る修正は、構成終了時刻 `c` を既知にした制御時計の unit test で、次を独立に検査すること。**

1. `ready` が booking 時刻ではなく、構成終了時刻 `c` を記録する。
2. `origin = ceil_lattice(max(e, c + lead))` が厳密に成立する。

通しのテストでは、現在の1733行の厳密な origin 検査を残せる。ただし、それだけにすると `ready` と origin が一緒に誤る回帰を見逃すため、上の独立検査が必要。

`>=80 ms−1 tick` への許容変更は採らない。`ready` だけを装置時刻に替える案も、比較する時計の違いを解決した証明にはならず、今回のテスト掃除のために本番コードを変更すべきではない。

未実施：cargo、mutation、負荷試験、実機試験。ファイル変更なし。上記の INFERRED 系列は、制御時計で到達状態を確認してから確定する必要がある。

## コードレビュー（FAIL）

判定：FAIL

**推奨：以下の major を直してから受理する。** テストはすべて通るが、移設によって元の検査対象を通らず合格する箇所がある。

## 指摘

1. **major — 有限 segment のテストが cut に到達していない**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:1258-1280`
   - `Segment.cut` は sample index（`crates/ezsdr-radio/src/timeline.rs:165-167`）だが、`clock.instant(5_000)`＝1,000,000 を入れている。実際の cut は5 msではなく1秒になる。
   - 5,000 sample の受信後も旧 segment のまま。`rx.poll()` は次 segment を開始しないため、最後の1,000 sample も旧 clock に載る。期待値 `first == 10_000` がそれを見逃す。新 segment の先頭なら、その新 clock 上の index は0になる。
   - **修正：** `cut: Some(5_000)` とし、旧 clock の終了、次 segment の `begin()`、異なる clock の先頭0を検査する。move chain 側も、Bの実受信・完了によってCが進む系列を復元する。現在の control test（`control.rs:4628-4638`）は計画だけである。

2. **major — swap の replacement が旧 owner の解放と交換完了を代替していない**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:4538-4566`  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:3111-3134`
   - RX側は旧 streamer を開かず、`Rx` owner も走らせず、試験自身が両旧 segment を `complete()` する。旧 owner がcutまで受信して解放する経路は通らない。
   - TX側は片方の zero-length segment をskipする所まで。両方の新 streamer が開くことを見ず、最後のassertは `next == None` でも合格する。
   - **シナリオ（INFERRED）：** ownerがmove後の旧segmentを完了できない、またはskip後に新segmentを読まない回帰でも、これらのreplacementは通り得る。削除前のfake testは両streamの再開／openを必須としていた。
   - **修正：** 実際の旧streamerと両ownerを既存部品で進め、owner自身の解放から両新streamerへの到達を必須にする。`None` を成功扱いしない。同サイズswap（`control.rs:4359-4428`）で失われた、新segmentの実サンプルのchannel確認も残す。

3. **major — configuration終了時刻 `c` の独立した観測になっていない**  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:4026-4054`
   - probeは `device.apply` **直前**（`core.rs:1062-1068`）。そこでCore時計を200 msにし、その後は進めていない。
   - したがって200 msはapply前にもapply後にも同じ値であり、「構成の終了」を区別できない。80 msの`apply_delay`は別のdevice host時計で経過する。
   - **シナリオ（INFERRED）：** 最後のapplyが終わる前にreadyを採る誤りを、この観測では排除できない。F39がkillされることは、booking時刻への置換を検出する証拠であって、この穴の証明にはならない。
   - **修正：** 既存の`HoldApply`または`LockSpy`の戻り側gateで、apply中と完了側のCore時刻を分け、既知の終了時刻からreadyとoriginを独立に検査する。

4. **major — drift追従テストが再アンカー停止を見逃す**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:449-461`
   - bracket化は妥当だが、許容幅を100 µsから1 msへ広げている。
   - 100 ppmで2秒なら、再アンカーが一度もなくても差は約200 µs。新assertはその実装を許す。元の「何も呼ばなくてもdeviceへ追従する」という検査が弱まった。
   - **修正：** bracket化と検出力を両立させる。必要なら追従の判定を制御時計のunit testへ分離する。許容幅の拡大だけで済ませない。

5. **major — captureが0件でも合格できる**  
   `crates/ezsdr-radio-uhd/tests/fake.rs:2546-2586`
   - artifactの検査は存在するものへのループだけ。予定した`changes`件が生成されたというassertがない。
   - **シナリオ：** capture開始が欠落してartifactが空なら、clock跨ぎ・old clockのcut・ramp検査を全部skipし、`problems.is_empty()`で合格できる。
   - **修正：** `rec`のartifact数が予定件数と一致し、各captureが非空で必要な両clockの有効sampleを持つことを必須にする。

6. **major — 行74を止めた理由は既存部品と矛盾する**  
   `/home/komatsu/.cache/ezsdr-fixes/2026-10-23-note37/report.md:64`
   - 「rate拒否wrapperを新設しないと作れない」とするが、既に`core.check_rates(Dir::Tx, 99)`でUR-13拒否を保存する手法がある（`crates/ezsdr-radio-uhd/src/provider/tx.rs:1242-1243`）。from-noneの反復拒否にも同じ手法を使っている（`control.rs:1228-1240`）。
   - これは新しいfake faultではない。制御時計で未来の拒否をbookingし、既存Tx ownerを進め、origin前に次のenableが処理されるか検査できる。
   - **修正：** この既存手法で行74を移設する。なお94・95の`OnceFails`がintegration側にしかないという説明は確認できた。63・64の移設不能までは確認できず、**INFERRED**である。

7. **minor — spec／報告に実装と合わない記述が残る**  
   `design/18-uhd-radio.md:667`  
   `/home/komatsu/.cache/ezsdr-fixes/2026-10-23-note37/report.md:84,109`
   - Changesはno-op testをunitへ移したとするが、実際はfakeに残る。
   - 報告はno-opの`LATE_COMMAND`不在assertを戻したとするが、`tests/fake.rs:5539-5567`にはない。
   - Python調査の「5,000 block、20 M samples」は計算違い。1 MS/s・10秒・RX一channelなら10 M complex samplesである。
   - **修正：** 実装に合わせて記録を訂正する。

## 確認結果

- `cargo test -p ezsdr-radio-uhd --lib`：**313 passed**。
- `cargo test -p ezsdr-radio-uhd --test fake`：**223 passed**。
- 本番コードは変更なし。新設部分はtest module内。
- mutation全リストの`old`／`old2`、計1,314箇所は各1回一致。指定11行は`lib`内のfilterに到達し、削除済みfake testへの参照残りなし。各変異とassertの対応も読んだ。**mutation自体は実行していない。**
- 行59のread-back窓、pending floor、held burst時のRX Stopは、制御時計で固定されている。
- Python調査のfree-running／計算量／実時間性能判定という説明は概ね妥当。ただし、過去の遅さの原因と§58 #2の一般的成立は、提示測定だけでは確定できない。

未実施：負荷試験、実機、Python再測定、mutation実行。ファイル変更なし。**通常テストが全通過しても、上記の検査欠落は残る。**

## 修正の再レビュー（FAIL）

判定：FAIL

項目1–4には **major 3件**が残る。owner による解放は復元されたが、同サイズ swap の実サンプル検査と、遅延に左右されない検査力が不足している。

## 指摘

1. **major — 同サイズ swap の実サンプル検査が未復元**
   - **箇所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:4431-4444,4459-4470`
   - **証拠：** 旧集合を `record_open()` に書き、`owner_configure()` 後の `rx_open`／`tx_open` と `apply` を確認するだけ。旧 streamer の実際の open、新 segment の start・受信・送信は通らない。
   - **シナリオ：** 新 streamer の open／設定は正しくても、owner が旧 channel のサンプル対応を使い続ける回帰を検出できない（**INFERRED**）。前回 major 2 が求めた「新 segment の実サンプルの channel 確認」が残っている。
   - **修正：** 制御時計上で旧 streamer から実 owner を切り替え、channel を識別できる実サンプルを新 segment で検査する。

2. **major — 行63・64の TX テストは、時計解放後の試験スレッド遅延で落ち得る**
   - **箇所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:3236-3252,3325-3338`
   - **証拠：** `Frozen` を落とした後、owner は `std::thread::spawn` で起動される。その後の `device.set_time()` → `tx1.step()` も参加者でない試験スレッド上。device 時計はこの隙間に進める。
   - **シナリオ：** destination の SOB を送る回で、`set_time()` 後に試験スレッドが数 ms 待たされる。Core 時計では予約が未来でも、FakeDevice では過去になり、`device.rs:1589-1597` が burst を落とす。新波形の assert が失敗する（実行での再現は未確認、**INFERRED**）。
   - **修正：** 待ちのある切替は既存の参加者として実行し、その後の時刻設定・送信操作は再び `Frozen` 下で行う。時計解放から参加者登録までにも隙間を残さない。
   - **補足：** 通常経路では、63の held tail／EOB／close、64の repeat wrap／cut／予約済み destination burst は到達し、サンプルも検査している。問題は場面の欠落ではなく、その成立がホスト遅延に依る点。

3. **major — drift の bracket は U35 を遅延次第で見逃す**
   - **箇所：** `crates/ezsdr-radio-uhd/tests/fake.rs:459-462`
   - **証拠：** 許容幅は100 µsへ戻ったが、`before` から `ours` までの遅延も bracket に入る。U35で再アンカーが止まり約200 µs遅れた Authority でも、この間に約100 µs以上進めば下限を満たし得る。上限側も満たす。
   - **シナリオ：** 同じ変異が、試験スレッドをすぐ動かすと失敗し、少し遅らせると合格する。実際の変異実行はしていないが、比較式と `authority.rs:142-147` の外挿からの **INFERRED**。
   - **修正：** 再アンカー追従と U35 の検出は、既存の制御時計・参加者を使う unit test に分離する。bracket を広げる方式は正常系の読み取り誤差には対応するが、一定の変異検出力は保証しない。

## その他の確認結果

- **前回 major 1：解消。** `rx.rs:1391-1418` は sample index の cut、旧 clock の終了、新 clock の先頭0を検査する。chain もB自身の受信・完了からCの openへ進む。
- **前回 major 2：部分解消。** RX／TX swap は実 owner が旧 segment を完了し、両新 streamer に到達する。試験側の `complete()` や `None` 成功扱いはない。同サイズ swap の不足は指摘1。
- **前回 major 3：解消。** `control.rs:4070-4096` の戻り側 gate は、device apply 完了後・configuration の終了時刻読み取り前に時計を変える。早すぎる `ready` と終了側を区別できる。
- **前回 major 5：解消。** `fake.rs:2557-2586` は予定 artifact 数、非partial、予定 sample 数、両 clock の有効サンプルを必須にする。
- **前回 major 6：解消。** `control.rs:1253-1282` は保存した rate 拒否を通り、Tx owner が拒否 segment を飛ばした後、次の enable が拒否 origin 前に自身の Action として処理されることを検査する。
- **前回 minor：解消。** no-op の `LATE_COMMAND` 不在assert、Changes、Pythonの sample 数は訂正済み。

**RXの2テストについて：** `rx.rs:894,1476` の解放後に遅延すると、FakeDevice の start 自体は late になり得る。ただし、このテストは `rx_recv()` を呼ばず、late は `device.rs:1206,1297-1300` の未読状態に残る。したがって、その遅延だけでは現在の判定は変わらない。解放・open の検査としては成立するが、「新 streamer が正常に受信できた」証明ではない。

**mutation の読解確認：**
- P509・P518・P337・U35の `old` は各1回一致し、filter は指定binaryに存在する。
- P509は最後の非zero sample、P518は destination 波形、P337は no-op の device call／clock／late 検査で検出する。
- U35には指摘3の穴が残る。mutation は実行していない。

## 項目5：B5の artifact 欠落（37cの判定外）

**overflow の成立は、capture artifact の成立を保証しない。**

- `rx.rs:562-600` は overflow event を、block の publish より先に出す。
- `tests/common/mod.rs:597-606` は固定 horizon まで進めて直ちに `finish()` し、capture の開始・書き出し完了は待たない。
- CaptureSinkは未開始ならartifactを返さない（`ezsdr-sink-capture/src/lib.rs:427-428,593-600`）。開始済みでも、flush・全ファイル読み取り・hash計算の後で初めてartifactを登録する（同`:433-450`）。
- cleanupには実時間5秒の期限がある（`ezsdr-kernel/src/run.rs:575,623-637`）。consumerのdrain／stopが間に合わない、または失敗すれば、overflow eventがあってもManifestのartifactは空になり得る。

**INFERRED：** 負荷下でのconsumer処理・書き出し・cleanupの遅延が有力な経路。ただし、提示されたpanicだけでは「未開始」「Sinkエラー」「cleanup timeout」のどれだったかは確定できない。なお、単なるsample不足なら開始済みcaptureはpartialとして返るため、それだけで `{}` は説明できない。

**推奨は形式(4)。** overflow → 再開block → `OverflowRestart` gap → payloadとgapのlost一致を、制御時計のunit testで固定する。大きなcaptureの書き出しを機能判定から分離する。

形式(1)はcaptureが既にSpec予約なので単独では不足、(3)はTX用で無関係。形式(2)で固定horizonをcapture完了状態の観測へ替える改善はできるが、実時間cleanup期限の影響まで消すには分離が必要。

## 実施した確認

- UHD lib：**318 passed**
- drift、long-block capture、slow-link capture：各 **1 passed**
- `git diff --check`：問題なし
- 対象3ファイルの本番部分はHEADから変更なし

未実施：負荷試験、実機、mutation実行、B5再実行。ファイル変更なし。通常テストの通過だけでは、上記3件の検査不足は解消しない。

## 2 回目の再レビュー（FAIL）

判定：FAIL

**major 1件が残っています。** 前回 major 1・3と、B5の置換、RX側の追加修正は確認できました。

## 指摘

1. **major — TX行63・64は、owner終了後から再holdまで時計が進む**
   - **箇所：** `crates/ezsdr-radio-uhd/src/provider/tx.rs:3310-3325,3405-3419`
   - **証拠：** 両ownerを参加者として登録してから最初の`Frozen`を解放する順序は修正済みです。しかし、workerは結果を送って終了し、試験スレッドは受信・join後に次の`Frozen`を取得します。この間、参加者は残りません。時計は進みます（`src/host.rs:118-125,404-409`）。
   - **シナリオ：** 両worker終了直後に試験スレッドが遅延すると、再hold時点のdevice時刻が送信処理の上限を越えます。行63では`k > ms(92)`、行64では`k > until`となり、送信ループ自体が実行されず、新波形のassertが失敗します。**INFERRED：意図的な遅延による再現は未実施。** 通常実行の通過では、この隙間は否定できません。
   - **修正：** destination workerが参加者のまま結果送信後も待ち、試験スレッドが次の`Frozen`を取得してから終了を許可してください。時計の保護を切れ目なく引き渡す必要があります。

## その他の確認結果

- **同サイズswap：解消。** RXは旧segmentをowner自身が完了し、新channelのloopbackを受信・publishまで検査します（`src/provider/rx.rs:1634-1679`）。TXも実ownerが切り替え、新channelの波形と旧channelへの送信不在を検査します（`src/provider/tx.rs:3196-3231`）。
- **U35：解消。** 制御時計をholdしてdeviceとAuthorityを比較します。U35では最初のanchorから500 ms以上のdriftが残り、許容2,500 ticksを越えます。試験スレッドの遅延でbracketが広がる穴はありません（`src/authority.rs:850-870`）。fake側はdriftなしの読み取りbracketだけです。
- **B5：確認済み。** overflow reportから再開block、`OverflowRestart` gap、payloadとの`lost`一致を検査します（`src/provider/rx.rs:1061-1093`）。`rehearse_overflow`と`hw_b5_overflow`はHEADから変更なしです。
- **RX追加修正：確認済み。** finite segmentは制御時計上で次segmentの実受信まで確認。move chain・同時刻swapも、owner登録が時計解放より先になっています。
- **mutation：読解確認済み。** P317・P509・P518・U35の`old`は各1回一致し、filterはlib内の対象テストに到達します。対応assertが変異を検出する経路も確認しました。ただしP509・P518の無変異テストには指摘1が残ります。

## 実施した確認

- UHD lib：**322 passed**
- fakeの読み取りbracket：**1 passed**
- `git diff --check`：問題なし
- 対象4ファイルの本番部分：HEADから変更なし

未実施：負荷試験、意図的遅延、mutation実行、実機。ファイル変更なし。

## 3 回目の再レビュー（PASS）

判定：PASS

指摘なし（major 0、minor 0）。今回の再レビュー範囲では、前回のmajorは解消しています。

1. **TX 行63・64：時計の引き渡しを確認。**  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:3308-3325,3409-3424`  
   両workerを最初の`Frozen`解放前に登録し、destinationは結果送信後も参加者として待機します。試験側は次の`Frozen`を取得してから終了を許可するため、owner終了後に時計が無制限に進む隙間はありません。

   sourceを待たせない理由も妥当です。標準channelで待つ参加者はbusyのまま時計を止めます。一方、destinationの`await_release`は同じ時計の1 ms sleepを待つため、sourceを保持すると、既に入ったsleepが終了できません（`src/host.rs:118-125,228-250`、`src/provider/core.rs:1209`）。

2. **RX move chain：同じ隙間は解消。**  
   `crates/ezsdr-radio-uhd/src/provider/rx.rs:933-968`  
   最後のCが結果送信後も参加者として残り、次の`Frozen`取得後に解放されます。A・Bを保持しない理由も、後続ownerの時計sleepとの関係で整合しています。

3. **ほかのownerテスト：同種の隙間なし。**  
   - RX同時刻swap：時計の観測は参加worker内で完結し、join後は保存済み結果・記録だけを検査します（`rx.rs:1548-1601`）。
   - TX同時刻swap：判定に使う`Core::now()`は手動Authorityの値で、join後のdevice時計に依存しません（`tx.rs:3131-3160`、`core.rs:911-912`）。
   - RX・TX同サイズswap：`Frozen`を保持したままowner操作とサンプル検査を行います（`rx.rs:1617-1684`、`tx.rs:3173-3231`）。

**実行確認：** 指定のreview target・`timeout 300`でUHD lib **322 passed、0 failed**。

未実施：意図的遅延の再現、負荷試験、mutation、実機。報告書の遅延注入実測は独立には再確認していません。ファイル変更なし。
