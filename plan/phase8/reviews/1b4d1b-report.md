STATUS: done

# Phase 8 段階 1b-4d-1b 実装の報告

リポジトリ `/home/komatsu/local_work/Ez-SDRv4`，HEAD `b190918`．計画は `test-plan-1b-v2.md`（裁定 §0 の分け方で書き直し，セルフレビューの 2 回・3 回の扱いを §10・§11 に追記）．commit・push・`git add` はしていない．tree は次の 8 ファイルが未 commit で変わっている：

```
crates/ezsdr-radio-uhd/src/provider/{core,control,rx,tx,test_support}.rs
design/18-uhd-radio.md
plan/maintenance/tools/mutations.json
plan/phase7/tools/mutations.json
```

## 1. 実装したもの（裁定の範囲）

- **B1 busy**：`StreamState` に `busy`／`reading`（`core.rs`）．uhd-control の from-none が判断の section（`idle && planned` を読む section）で `busy = true`，booking の section で false．owner（uhd-rx `begin`，uhd-tx `read`／`pass`）は `busy` を見て device を呼ばず，cursor・`idle` を書かない．RX の owner は busy を取らない．
- **B2 reading**：uhd-tx は plan の読みと `reading` の設定を**同じ section**で行い（`pass` → `plan_section`），`reports` の後に下ろす．from-none は `reading` が下りるまで clock の待ち（`await_reading`，`host.sleep(POLL)` の回し）で待ち，普通の cold 変更へ落ちない．shutdown の読みも `reading` を立てて下ろすが，**busy は見ない**（裁定 §1.1 の最後）．
- **B3**：D2 の取り下げどおり，from-none の条件は `idle && planned` のまま．owner が open 中の enable は普通の cold 変更として記帳する（`ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change`）．
- **B5 lock の順序**：`core::lock` は透明な `Locked` guard を返し，test build はこの thread が持つ `core::lock` の guard の数を thread-local に数える．`assert_line_free` を `updates` を取る 5 か所（`ColdConfig::book`・`changed`・`configure`，`release`・`cancel_held` の batch）の直前に置いた．`LockSpy` の line の検査も `try_lock` からこの数えに置き換え（別 thread の line を誤らない），対照のテストを 1 本足した．B5 が見るのは「`core::lock` の guard を持ったまま `updates` を取る」ことだけで，`rec → streams` の逆は見ない（doc と design に明記）．
- **loss と stop の出口**（裁定 §1.1）：
  - `reading` 待ちの loss：`await_reading` の `is_lost` で待ちを終え，device を呼ばず `busy` を下ろす．
  - open が返った後の loss（S26）：照合・configure・booking をしない．open の記録（`Open`）は残す．
  - 照合が返った後の loss：configure に入らない（`probe("apply")` 0 で固定）．
  - configure の channel の間の loss：`Core::configure` が各 channel の `apply` の前に `is_lost` を見る（owner の configure も同じ関数）．
  - `reading` 待ちの Provider の stop：stop の印で待ちを終え，device を呼ばず `busy` を下ろす．記帳は既存の stop の経路（この enable は何も記帳しない）．
  - 呼び出しの間に立った stop：次の section で見て**何も記帳せずに**返る（in-flight の呼び出しは取り消さない）．design の Changes と UR-25 にそう書いた．
  - join が 1 s で uhd-control を切り離したとき：uhd-tx の shutdown の読みは busy を見ない（S-detach）．
- **`Streams` の `busy`／`reading` の doc**，`core::lock` の doc，`LockSpy` の gate（`stop_before`／`stop_after`，gate 中の同じ方向の呼び出しで panic）・`log()`・`rates` の数え・thread-local の検査，`Core::probe`（test build のみの section の hook）を足した．

## 2. 計画 v2 から変えたところと理由

1. **`read` と `begin_reading` の 2 section をやめ，`pass` の中で plan の読みと `reading` の設定を同じ `streams` の section にした**（`plan_section` に切り出し）．セルフレビュー 3 回目の P2（2 つの section の間に enable が入る窓）を，テストで競合を作るのではなく**窓そのものを消して**閉じた．`read` は test と `do_switch` の経路として busy を見る形で残す．窓を駆動していた `ur_25_a_transmit_turn_that_meets_the_enable_s_mark_takes_no_report_read` は消した（mutation の P398 は `pass` の busy の検査を消す行に置き換え，J50・J59 は `plan_section` の行に当て直した）．
2. **S24b の assert**：owner の最初の segment は設定を持たない（`by` が None で `owner_configure` が configure できない）ので `rx_start` は観測できない．plan の segment 数（+1）と booking 後の `idle`，owner の `rx_open 0 [0]` とその記録を assert する（計画 §2 を直した）．
3. **S9a/S9b の「最初の open」**：gate の後を mark し，**owner の最初の** open を数える形に直した（`last()` ではなく），計画も直した．
4. **「何も記帳せずに返る」の観測**：RM-25 は End の後の item を plan から落とすので `plan_of` では見えない（レビュー 2 回目の P2）．line の `Kind::Cold` の数を数える `rx_colds`／`tx_colds` を足し，早期 return の 6 つのテスト（S26・S27e・S-stop・S-loss-wait・stop-before-calls・stop-after-open・loss-after-rates）で 0 を assert する．
5. **stop の扱いの範囲**：呼び出しの**間**に立った stop は再検査しない（`busy` は booking しない出口で下りる）．裁定 §1.1 は「`reading` を待っている間の stop」を要求しているので，design の Changes 行と UR-25 の文をその実装に合わせて狭めた（レビュー 2 回目の P2-1）．
6. **追加した系列**（裁定 §1.1 の出口と，レビュー 2・3 回目の指摘）：
   - `ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`（`reading` 待ちの loss．tx の gate は control が終わるまで離さないので，待ちを終えられるのは loss だけ）
   - `ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`（open の後の stop）
   - `ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device`（判断の section の stop）
   - `ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing`（照合の後の loss．`probe("apply")` 0 で「configure に入らない」を見る）

## 3. テストと行

### 足したテスト（18 本，すべて unit）

`control.rs`：`ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`，`ur_25_a_device_call_passes_while_another_thread_holds_the_line`，`ur_25_an_open_is_recorded_only_after_the_device_returns`，`ur_25_a_from_none_holds_the_streamer_from_its_decision_to_its_booking`，`ur_25_an_enable_behind_an_owner_s_open_is_a_cold_change`，`ur_25_a_lost_stream_starts_no_rate_check_apply_or_booking_after_the_call`，`ur_25_a_from_none_leaves_the_stream_unbusy_on_every_path`，`ur_29_a_lost_device_stops_a_from_none_enable_between_channels`，`ur_25_an_enable_from_no_stream_after_the_provider_s_stop_calls_no_device`，`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing`，`ur_29_a_lost_stream_between_the_rate_check_and_the_configure_applies_nothing`，`ur_25_a_configuration_taken_under_the_line_lock_is_refused`（should_panic）．
`tx.rs`：`ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing`，`ur_25_a_transmit_poll_during_a_control_open_starts_no_streamer_call`，`ur_25_a_from_none_enable_waits_for_a_transmit_report_read`，`ur_25_a_from_none_enable_ends_its_wait_at_the_provider_stop`，`ur_29_a_lost_device_ends_a_from_none_enable_s_wait_for_a_report_read`，`ur_25_a_detached_control_does_not_hold_the_shutdown_s_read`．

helper：`spy_rig`（control・tx），`rx_enable_from_none`／`tx_enable_from_none`，`rx_record`／`tx_record`，`rx_colds`／`tx_colds`，`Rx::cursor()`（cfg(test)），`test_support::probe`，`LockSpy::{stop_before,stop_after,log,rates}`．
消したテストは無い（既存の名前も assert も変えていない）．`tx.read` が `bool` を返すようになったが，既存の呼び出しは戻り値を無視する．

### mutation の行（`plan/maintenance/tools/mutations.json`，`plan/phase7/tools/mutations.json`）

新規 21 行：P382（RX の owner は busy を見る），P383（TX の owner は plan を busy で読まない），P384（from-none は busy を立てる），P385（booking の前に下ろさない），P386（成功の後に下ろす），P387（`reading` を待つ），P388（read の後に `reading` を下ろす），P389（早い戻りで busy を下ろす），P390（open の後の loss を見る），P391（configure の各 channel の前の loss），P392（shutdown の読みは busy を見ない），P393（owner は line を持ったまま configure しない），P394（configure の前に `assert_line_free`），P395（`await_reading` の loss），P396（判断の section の stop），P397（照合の後の loss で configure に入らない），P398（`pass` の busy の検査），P399（早期 return で記帳しない），P400（post-call の stop の検査），P401（`await_reading` の stop），P402（shutdown の `end_reading`）．
当て直した行 6 本：P271（`drop(streams)` の context を拡大），P343（`let check = if open.is_ok() && !is_lost()…` に），P351（`failure` の match の configure の arm に），U39（`Some(self.core.configure(…))` に），J50（`plan_section` の `prune` の arm に），J59（`plan_section` の version の条件に）．変えていない行：P374・P375〜P381（`release` を触らないので）ほか．
ファイル全体の書き直しはせず，変える行・足す行だけを周りの書式のまま編集した．`rules.py` が全行の一致（各 1 回）を見ている．

## 4. 確かめることの結果

1. `cargo test --workspace --no-fail-fast`：66 のテスト結果すべて ok（0 failed）．`ezsdr-radio-uhd` の lib は 168 本（この段階で +18）．
   `cargo clippy --workspace --all-targets -- -D warnings`：clean．
   `cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`：**このマシンでは流せない**．`build.rs` が `UHD_LIB_DIR` か `pkg-config --variable=libdir uhd` を要求し，どちらも無い（`UR-35: the feature uhd needs libuhd`）．usrp-lnx03 に libuhd が無いためで，コード側の失敗ではない（前の段階と同じ条件）．
2. `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`：`0 problem(s)`．
3. 両方の `mutations.json` の全行がファイルにちょうど 1 回一致する（rules.py が見ている）．
4. mutation（`remote_mutate.py`，sim02:8,sim03:8，`REMOTE_SSH="ssh -o BatchMode=yes"`）：この段階で触れた 27 行を流し，**27/27 killed**（必要な行だけの中間の run も含めて，生き残りは 0）．
   - P271, P343, P351, U39, J50, J59（当て直した既存の行）
   - P382〜P402（新規 21 行）
   - 途中で生き残った P385・P388・P398 は，テスト／行を直して再実行し killed にした（P385 は probe の位置と行の向き，P388 は control の終了前に Shutdown を送っていたこと，P398 は窓のテストが負荷で揺れたこと——窓そのものを消した）．
   全リストは orchestrator の gate で流す（この段階では触れた行だけ）．
5. 追加の確認：P398 の単独 run（sim02:2,sim03:2）と 16 shard の run の両方で killed．テストは `cargo test -p ezsdr-radio-uhd --lib` で 168 本すべて green．

## 5. セルフレビュー（reviewer は DeepSeek，回ごとに新規起動）

- **1 回目**（`self-review-1.log`）：P1 なし．P2 4 件・P3 6 件．扱い（計画 §9）：
  - P2 `reading` 待ちの loss のテスト欠落 → S-loss-wait と P395 を追加．
  - P2 design の「booking で返す」の全称 → UR-3・UR-25・Changes を「booking，または booking せずに終える section」に．
  - P2 design の待ちの出口 → loss と stop も待ちを終えることを UR-25 に．
  - P2 shutdown の読みの例外 → UR-25 の段落に UR-16 の例外として．
  - P3 S24b の assert／S9 の「最初」／S26 の Test 行／`assert_line_free` の範囲／「one owner」の文／configure の各 channel の loss／`rx.rs` の整形 → すべて採った．
- **2 回目**（`self-review-2.log`）：P1 なし．P2 2 件・P3 4 件．扱い（計画 §10）：
  - P2 stop が between-calls で再検査されない → 裁定の要求（wait 中の stop，判断 section の stop）に合わせて実装と design を揃え，`ur_26_an_enable_from_no_stream_a_stop_recorded_after_its_open_books_nothing` と P400 を追加．
  - P2 「記帳しない」が未観測 → `rx_colds`／`tx_colds` と P399 を追加．
  - P3 stop の `!stopped` 分岐 → stop-before-calls のテストと P396．
  - P3 照合後の loss → loss-after-rates のテストと P397．
  - P3 UR-3 の shutdown の例外，UR-25 の `reports` の範囲 → 採った．
- **3 回目**（`self-review-3.log`）：P1 なし．P2 1 件・P3 3 件．扱い（計画 §11）：
  - P2 `read`／`begin_reading` の間の窓が未テスト → **窓を構造的に消した**（上記 §2 の 1）．
  - P3 呼び出しの間の stop が design に無い → UR-25 と Changes に 1 文，テストと P400 は 2 回目で追加済み．
  - P3 早期 return の gate の行 → P399．
  - P3 `await_reading` の stop と shutdown の `end_reading` の行 → P401・P402．
- 3 回目の P2 修正（`pass` の統合）はその回のレビューを受けていないが，`cargo test --workspace`（66 ok）と，そのコードに当たる mutation 4 行（J50・J59・P383・P398）の killed で確かめた．4 回目は brief の上限（3 回まで）を超えるので起動していない．
- 判定を出さずに終わった回は無い（各回とも P1 の有無と指摘を返した）．指摘はすべて採るか，上記の理由で範囲を狭めた（黙って持ち越したものは無い）．

## 6. 変えた `design/` の文（`design/18-uhd-radio.md`，DA-1）

- **UR-3**：streamer の担当の文に，enable from no stream の間の uhd-control の一時所有（booking まで，または booking せずに終えるまで）と，join が切り離した control の shutdown の読みの例外（UR-25，UR-16）を足した．全 1 段落を 2 段落に割った（1 行が 1 500 字を超えないため）．
- **UR-17**：「An owner whose stream's `busy` mark is set begins nothing that turn, and reads the plan again at the next (UR-25).」を 1 文．Test 行に `ur_25_an_owner_waiting_behind_a_from_none_open_starts_nothing`．
- **UR-23**：「An owner whose stream's `busy` mark is set makes no switch that turn (UR-25).」を 1 文．Test 行に TX の 3 本．
- **UR-25**：短い別の段落 **Ownership.** を足した——busy の持ち主と返す section，owner の device 呼び出しの禁止（shutdown の読みの例外），`reading`（`reports` の呼び出し全体）と clock の待ち，loss と stop の出口，`Core::configure` の各 channel の loss（UR-29），lock の順序（`updates` → `core::lock` の state）と検査の範囲（`updates` を state の下で取ることだけを見る；`rec → streams` の逆は見ない）．Test 行に 17 本のテスト名を足した．
- **Changes**：2026-10-19 の行（UR-3・UR-17・UR-23・UR-25）を足した．理由はこの段階の裁定とこの報告へ．
- UR-24・UR-36・UR-17 の他の文は触っていない（B4 は 1c）．`design/18` の差分は +14/−6 行．

## 7. 大きさ

| 区分 | 足す | 消す |
|---|---|---|
| 本体（`core.rs` 79，`control.rs` 92，`rx.rs` 12，`tx.rs` 54） | 237 | 61 |
| テストと helper（`test_support.rs` 149，`control.rs` の test 383，`tx.rs` の test 253） | 785 | 12 |
| `design/18-uhd-radio.md` | 14 | 6 |
| 合計（mutation の JSON を除く） | **約 1,040 行** | 約 80 |

計画 v2 の見積もり（約 1,270 行，本体 300・テスト 905）より小さく，1,500 行以内．mutation の行は新規 21 + 当て直し 6 = 27 行（JSON の行数は上の合計に含めない）．

## 8. 1c へ送るもの（この段階では実装していない）

- B4（D1 の A）：`release` の記録だけの枝，S11d の正・逆，S28 の重ね合わせ，Sol major 1〜3（owner が始めた後の系列，configure の成功記録の公開の窓，queue の会計と過ぎた origin）．
- UR-24・UR-36 の文（Sol minor 10 の点検と，計画 §9.9 の plan 33 の宿題——後者は orchestrator が段階を閉じるときに直す）．
- `plan/phase8/33-channels-and-streams.md:266` の 1b-4d-1b の行は前の裁定（owner が busy を持ち，from-none は busy のとき cold に落ちる）のまま．この段階の B1〜B3 と食い違うので，段階を閉じるときに orchestrator が直す必要がある．
