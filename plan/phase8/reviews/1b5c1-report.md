STATUS: done

# 1b-5c-1 実装報告（2026-10-22）

## 結果

UHD Module の current-key mode を削除し、常に binding の layout から作る `channels::Description` で動かすようにした。`keys` は absent または `"channel"` のみ受理して無視し、ほかの値は UR-5 で拒否する。spec 18 と UHD の layout テストも合わせた。指定外の `ezsdr-radio`、MockRadio、server、Python、spec 07、Vision は変更していない。すべて unstaged・uncommitted。

## 実装

- `crates/ezsdr-radio-uhd/src/provider/mode.rs` の `Mode::{Current, Layout}` と両 mode の共通 façade を削除。`channels::Description` を直接保持し、layout 用の小さな `DescriptionExt` に必要な操作だけ残した。
- `crates/ezsdr-radio-uhd/src/profile.rs` の current-key description builder / `Selection` 用 test builder と、`crates/ezsdr-radio-uhd/src/provider/mod.rs`, `control.rs`, `core.rs`, `tx.rs` の mode 分岐・current-key-only validation・key routing を削除。front-end 照合は O5 の共通の layout 経路に一本化した。
- UHD の unit / integration fixtures と hardware test inputs を layout keys (`radio.<d>.c<n>.*`, `radio.<d>.s<k>.*`)、stream nodes、`rx<n>` ports に移した。
- control differential は 1,000 generated sequences と全操作を維持し、正の channel count を 1-channel rig の `c0.stream = 0` membership に写し、instance-wide layout floor を使う。操作を除いたり置き換えたりしていない。

### 行数（`git diff --numstat`）

| Scope / path | Add | Delete | Note |
|---|---:|---:|---|
| `crates/ezsdr-radio-uhd/src/provider/mode.rs` | 43 | 235 | `Mode` façade removed; layout helpers remain |
| `crates/ezsdr-radio-uhd/src/profile.rs` | 1 | 21 | current-description builders removed |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | 456 | 657 | mode branches and current-only unit cases; embedded tests included |
| `crates/ezsdr-radio-uhd/src/provider/core.rs` | 41 | 69 | mode branches removed |
| `crates/ezsdr-radio-uhd/src/provider/mod.rs` | 46 | 103 | selection and duplicate front-end path removed |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | 30 | 97 | current-key cases/branches removed; embedded tests included |
| Other UHD `src/` files | 117 | 40 | `control/differential.rs`, `rx.rs`, `rx/differential.rs`, `test_support.rs` |
| UHD `tests/` | 246 | 389 | `common/mod.rs`, `fake.rs`, `hardware.rs` |
| `design/18-uhd-radio.md` | 63 | 63 | accepted spec update |
| mutation lists | 535 | 645 | maintenance + phase7 |
| **Total** | **1,578** | **2,319** | `src/` includes embedded unit tests |

移行前の直接参照 inventory（function 数）：

| File | 直接参照 | 扱い |
|---|---:|---|
| `tests/fake.rs` | 66 | layout keys / stream targets に移行 |
| `crates/ezsdr-radio-uhd/src/provider/control.rs` | 62 | layout keys / per-channel target に移行 |
| `crates/ezsdr-radio-uhd/src/provider/tx.rs` | 2 | layout rig に移行 |
| `tests/hardware.rs` | 4 | layout selector / stream paths に移行。実機実行はしていない |
| `src/provider/rx.rs`, `tests/uhd_api.rs` | 0 | current-key 直接参照なし |

### 削除した11テストと残った規則の確認

| 削除したテスト | 残ったテスト／扱い |
|---|---|
| `ur_05_the_current_keys_set_uhd_s_default_order` | `ur_05_the_layout_builds_the_subdev_spec`, `ur_05_the_layout_s_order_is_the_device_s_channel_order`, `ur_05_a_layout_out_of_uhd_s_default_order_is_accepted`。layout の順が UHD の channel map に反映される規則に一本化。 |
| `ur_12_each_key_set_refuses_the_other_in_prepare` | `ur_12_prepare_refusals` と `ur_11_coerce_is_the_descriptions`。未知の `radio.rx.c2.frequency_hz` は RM-4 で拒否され、拒否前後で device apply は増えない。 |
| `ur_15_the_current_keys_name_their_one_stream_s_threads` | `ur_15_every_declared_stream_starts_its_own_thread` が 2×2 layout の全 thread names と join を exact で確認。 |
| `ur_24_a_halted_direction_fills_no_queue_slot` | 規則を「channel がなくても command は1 slot」に一本化。`ur_24_a_release_without_a_live_channel_takes_the_command_queue_slots` が 16 held + 17th refusal を確認。 |
| `ur_24_the_current_keys_held_updates_reach_every_channel` | current-key fan-out は撤去。`ur_24_a_channel_keyed_timed_update_acts_on_its_own_channel`, `ur_24_a_timed_update_for_a_channel_out_of_its_stream_waits_for_its_segment`, `ur_24_the_layout_destination_follows_the_effective_membership`, `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` が channel 指定・held update・membership を確認。 |
| `ur_24_the_queue_counts_a_command_per_channel` | `ur_24_a_channel_keyed_command_counts_one_queue_slot` と held/no-channel queue test。 |
| `ur_25_the_current_keys_floor_a_cold_change_only_at_its_own_stream` | current-key floor 撤去。`ur_25_a_layout_cold_change_is_floored_at_every_earlier_cold_change_of_the_instance` と 1,000-sequence control differential が instance-wide floor を確認。 |
| `ur_26_a_current_keys_antenna_update_is_refused` | `ur_26_an_unnumbered_key_is_refused_at_runtime` が direct runtime の unnumbered key を拒否し apply しないことを確認。Layout antenna の cold behavior は `ur_25_a_channel_s_antenna_change_cuts_its_stream`。 |
| `ur_29_a_partly_applied_release_keeps_the_projection` | per-channel `ur_29_a_failed_released_update_is_a_device_failure` と `ur_24_a_refused_release_records_no_row_and_keeps_the_projection`。 |
| `ur_29_a_lost_device_between_a_release_s_channels_applies_no_more` | multi-channel release はなくなった。`ur_29_a_lost_timed_release_publishes_device_loss` が apply 自身の lost error、`ur_29_a_loss_published_before_a_release_stops_the_call` が apply 前の published loss を確認。 |
| `ur_29_a_loss_published_between_a_release_s_channels_stops_the_rest` | single-channel の `ur_29_a_loss_published_before_a_release_stops_the_call` と次の command を止める `ur_29_a_loss_after_a_release_s_command_stops_the_one_behind_it`。 |

### 復元したテスト

- `crates/ezsdr-radio-uhd/src/provider/tx.rs`: `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` を layout keys に復元し、B `{0,1}` → empty → C `{0}` の control-open と owner-open、各 clock adoption を確認。
- 同 file: `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` を同じ layout membership 系列に復元。control の open 中に cursor・switch・idle mark・device call が変わらず、再開後に owner が B を開くことを確認。

Added regression tests: `ur_26_an_unnumbered_key_is_refused_at_runtime`, `ur_29_a_loss_published_before_a_release_stops_the_call`, and `ur_29_a_lost_timed_release_publishes_device_loss`. Added the `ApplyFails::timed_lost` test helper back for the last case. `receive_spec` now asserts its supported count is only 0 or 1, so future calls cannot silently turn a multi-channel test into a one-channel test.

## spec 18

UR-5/9/12/15/17/20/23/24/25/26/28/30/34 と §6 を layout-only に整合させた。`radio.rx.coherent` の unsupported capability claim を除き、RM-31/RM-32 runtime refusal を明示し、`keys` の継続受け入れと stage 1b-5c-2 での removal は Changes row に記録した。rule IDs は削除・変更していない。変更記録は Changes table の 2026-10-22 row。

## Mutation 行

- `plan/maintenance/tools/mutations.json`: 51 個の生きている mutation を現行の `Description` / per-channel code に再 anchor し、削除テストの filter を残った layout test に retarget した。IDs: `F40`, `J277`, `P265`, `P279`, `P290`, `P291`, `P292`, `P294`, `P303`–`P306`, `P313`, `P315`, `P316`, `P319`–`P326`, `P333`, `P335`–`P337`, `P339`, `P342`, `P347`, `P349`, `P352`, `P353`, `P355`, `P405`–`P408`, `P422`, `P428`, `P436`, `P437`, `P443`, `P449`, `P462`, `P463`, `P465`, `P497`, `P501`, `P502`, `P519`.
- Removed `J17`, `J64`, `P13`, `P251`, `P338`, `P340`, `P421`: each row's old source text was present once in HEAD and occurs zero times in the current source. `P370` was restored with its TX-specific filter; although its code mutant matches `P369`, the RX and TX tests are distinct. Retargeted surviving filters in `J69`, `J70`, `P383`, `P404`, `P419`, `P434`, `P471`.
- `plan/phase7/tools/mutations.json`: removed `R22`, `R26`, `R27` after verifying their inline front-end loop text is absent; updated `R21` to a non-equivalent missing-slot mutant and the current queue-slot test.
- Remote targeted gate: `maintenance,phase7`, sim02:3 + sim03:3, `REMOTE_CPUS=8`. 60 selected rows all killed. First run: J69's obsolete mutant did not compile and was rewritten; the 60-row run then killed J69. P279 initially survived with an unsuitable filter; it was retargeted to the origin-order unit test and rerun, where it was killed. Exact-source check for all mutation lists is green (`rules.py`). The full six-list mutation gate was not run; this was the brief's targeted-row gate.

## Checks

| Check | Result |
|---|---|
| `cargo test --workspace --no-fail-fast` | passed; UHD lib 235 tests and fake integration 265 tests passed; all workspace targets passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| UHD Docker clippy (`--features uhd --all-targets`) | passed |
| UHD Docker `--lib --test uhd_api` | passed; lib 243, `uhd_api` 5 |
| `rules.py --base HEAD --renames ...` | 0 problems |
| `rules.py --self-test` | passed |
| Remote mutation rows | 60/60 killed, with the reruns described above |
| Hardware tests | not run; they require the USRP and remain for the owner |
| Python suites | not run; Python was outside this stage's scope and unchanged |

## DeepSeek self-review

- Round 1: P1-1 (runtime rejection of an unsupported/unnumbered key lacked a remaining test) was fixed by `ur_26_an_unnumbered_key_is_refused_at_runtime`.
- Round 2: P1 なし. Adopted and fixed P2-1 (restored/reanchored live mutations), P2-2 (removed `radio.rx.coherent` claim), P2-3 (UR-25 cites RM-31/RM-32), and P2-4 (added apply-returned-lost test). P3 fixes: exact owner open sequence assertion; no-apply assertion for unknown prepare key; corrected §6 selector/rate/port/throughput descriptions; corrected UR-5 source, UR-10 wording, Changes rule IDs, hardware indentation, receive fixture count assertion, mixed-board exact reason; removed duplicate P370; renamed the new unnumbered-key test. No third round was run because the brief says to stop once P1 is absent.

## 残る穴・境界

- TX-specific multi-channel B→C reopen sequence from the removed current-key test is not duplicated under layout keys; the shared owner decision is pinned on RX, and TX stream ownership/open behavior is covered by the remaining TX tests. This is the accepted P3 residual.
- `tests/hardware.rs` was migrated and compiled by Docker clippy but not run on an X300. Python suites were not run. Full `all` mutation lists remain the orchestrator's gate; only the affected/retargeted rows above were run.
- No commit, stage, push, stash, schema/version update, or `cargo fmt` was run. All modified repository files are mode 644 and remain unstaged.

---

# 修正の回 1（指示書は 1b5c1-opus.md §3）


# 1b-5c-1 修正回1 報告（2026-10-22）

## O1（major）：TX from-none 系列を layout keys で復元

- `crates/ezsdr-radio-uhd/src/provider/tx.rs:1104` の `tx_channels_from_none` を channel 集合 `&[u32]` を受け取る helper に変更し、旧 `tx_enable_from_none` を統合。c0/c1 の `radio.tx.c<n>.stream` を必要なときだけ更新する。
- 同 `tx.rs:1153` の `ur_13_a_transmit_from_none_segment_the_owner_never_started_is_opened_again` を復元。layout B `{0,1}` → empty → C `{0}` について、control の open 順、owner の B/C reopen、clock adoption を確認する。`tx.rs:2180` の `ur_25_a_transmit_owner_waiting_behind_a_from_none_open_starts_nothing` も同系列に復元し、busy 中の cursor・switch・idle mark・device call が動かないことを維持する。
- 2 本の unit test は c1 を layout の初期 config で stream 0 に割り当て、Start は no-stream の状態にする。B を有効にすると control が一度に `{0,1}` を開き、空への移行では両 channel が抜け、C は c0 だけで開く。二つの helper が別々に持っていた設定処理は残していない。
- `report.md:42` の削除テスト表を13本から11本にし、復元した2本を別節へ移した。

## O2（minor）：ColdConfig の test-only 経路を除去

- `crates/ezsdr-radio-uhd/src/provider/control.rs:63-99` から `test_base`、`test_dir`、test-only settings projection と `settings_on` の cfg 分岐を削除。production と test の両方が layout map を `channels::Description::settings` に渡す。`ColdConfig::new` は map を受け取り、未使用だった `with_clock` の dir 引数も削除した。呼び出し側は同 file の406、782–783、851行などで更新。
- `rx.rs`、`rx/differential.rs`、`core.rs` と TX の default 設定 fixture は空 map に変更。Settings 値を使う TX switch fixture は `tx.rs:1035` で layout configuration map にする。effective-order test は `control.rs:2070-2095` で at-origin update を本番の `configure` に通し、device read-back が gain 7 / frequency 1.1 GHz であることを確認する。
- `control.rs:549` は `let cost = 1` を `pending + 1` に畳んだ。

## S3（major）：control differential の2-channel遷移を維持

- `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:111-153` の期待側は count change を c0/c1 の個別 membership 更新に展開。`:310-337` の実行側も layout channel keys を Control に渡し、no-op count change も c0 の no-op key update として保持して instance-wide cold floor を変えない。既存操作は削除していない。
- 1,000 seed 全体で `1→2` 等の2-channel化は **495**、2-channelからの縮小は **172**。両方を非0で assert し、differential 全体が通った。

## S4（minor）：spec 18 の用語

- `design/18-uhd-radio.md:180` の UR-9 冒頭を `ezsdr_radio::channels::Description` に修正し、`x3xx` terms との関係を記述。
- 復元したテストを UR-13 (`:208`) と UR-25 (`:280`) の `*Test:*` 行に戻した。これにより `rules.py` の引用検査も解消。

## Mutation

当回に変更した mutation 行だけを実行：`J69`, `P279`, `P355`, `P370`, `P465`, `U20`, `R21`。sim02:3 + sim03:3、`REMOTE_CPUS=8`。**7/7 killed**。

- P279 を旧 test projection から production の `settings_on` の origin/channel 投影へ再 anchor。at-origin update の device read-back test が kill。
- P370 を復元。P369 とコード mutant は同じでも、filter は RX と TX の別 test なので重複扱いせず、TX test を再び指す。
- J69/P355/P465/U20/R21 は O2 の source 形に合わせて `old`（および mutant）を更新。
- maintenance 891行、phase3–6/phase7 も含め、全 mutation `old` は source に各1回一致。`rules.py` と同じ source-count check が通った。

## 確認結果

| Check | Result |
|---|---|
| `timeout 300 cargo test -p ezsdr-radio-uhd --lib --no-fail-fast` | 237 passed |
| `timeout 300 cargo test -p ezsdr-radio-uhd --test fake --no-fail-fast` | 265 passed |
| default `--test hardware`, `--test uhd_api` | 各0 tests（UHD featureなし） |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| Docker UHD clippy (`--features uhd --all-targets`) | passed |
| Docker UHD `--lib --test uhd_api` | lib 245 passed、`uhd_api` 5 passed |
| `rules.py --base HEAD --renames ...` | 0 problems |
| `rules.py --self-test` | passed |
| targeted remote mutation | 7/7 killed |

修正中の最初の differential run は72 seedで clock mismatch となった。no-op Channels count の cold floor が実行側から抜けていたため、実行側にも no-op key update を残して修正し、full 1,000-seed test で再確認した。最初の compile attempt の private `config_at` access も `pub(super)` にして解消した。

## 未実施・作業状態

実USRPでの hardware suite は実行していない（機材が必要。Docker clippy は `--all-targets` で通過）。full mutation lists は今回の指示どおり流さず、変更した7行のみ実行。セルフレビューは指示どおり未実施。brief/checklist/watchdog は変更・停止していない。commit、stage、push、stash、cargo fmt はしていない。変更ファイルはすべて mode 644。

---

# 修正の回 2（gate の mutation．指示書は 1b5c1-opus.md §6）


## Compile error の4行

- **P446** (`control.rs:403-405`)：mutant を `booked_stream(Dir::Rx)` から `stream_key(Dir::Rx, 0)` に変更。test-only helper を参照せず、stream 0 に誤 book する同じ mutant。`fake.rs:7205-7233` で start_rx の later-stream booking が別 stream に反映される差を見て **killed**。
- **P450** (`control.rs:966` 以下)：mutant を `booked_stream(Dir::Tx)` から `stream_key(Dir::Tx, 0)` に変更。`fake.rs:7314` の二つの transmit clock/target 差を見て **killed**。
- **P464** (`core.rs:287`)：mutant を `booked_stream(dir_of(stream))` から `stream_key(dir_of(stream), 0)` に変更。`fake.rs:7237-7269` の stream 1 refusal の差を見て **killed**。
- **P469** (`control.rs:393-408`)：removed `self.core.mode.stream_channels` を `self.core.description.stream_channels(&self.config_at(self.core.now()), true, k as u32)` に変更。`fake.rs:7651-7671` が channel-less start_rx を拒否せず、後の join で開始することを検査し **killed**。

## Survivors と使われなくなった行

- **P265** (`control.rs:889-902`)：既存テスト `ur_25_an_enable_at_a_timed_update_s_instant_applies_it` (`fake.rs:4262-4290`) に filter を変更。c0 が no-stream (`stream = -1`) の状態から、同時刻の timed update と from-none enable を試し、enable の untimed configure に適用値が入ることを device calls で確認する。**killed**。
- **P424** (`control.rs:495-506, 585-608, 632-655`) は **survived**。`book_timed` (`:511-527`) は numbered channel key だけを受理し、`Timed.chan` は1 channel。`targets` は live 時 `[chan]`、planned 時 `then_some(chan).collect()` を返すため、release は空を除き `give` に必ず1要素を渡す。`give` 内外に1回 push する mutant はこの source のどの layout call sequence でも同じとなる根拠を確認した。行・filterは変更せず、`equivalent` も付けていない。判定は orchestrator に委ねる。
- **P282** を mutation list から削除。`channel_keys` の production 呼び出しは `ezsdr-radio-uhd/src/provider/mod.rs:251` と `ezsdr-mock-radio/src/lib.rs:363`。両方とも `.map_err(...)?;` で selector の不正だけを検証し、`bool` の戻り値は使わない（検索 `channel_keys\(`、`crates/**/*.rs`）。関数定義は `ezsdr-radio/src/lib.rs:711`。この行は1b-5c-2で `channel_keys` と共に消える予定。
- **R24** を `x3xx::description` から layout の道へ re-anchor：`ezsdr-radio/src/channels.rs:853-865` の `Channel::of` にある `default_frequency_hz: front_end.default_frequency_hz()` を1 GHzに変える mutant。CBX 既定2.45 GHzを確認する既存 `fake.rs:194` により **killed**。`x3xx::description(` の repository 検索結果は `ezsdr-radio/tests/device.rs` の5テストだけで、production caller は無い。
- **R04** (`ezsdr-radio/src/timeline.rs:380-388`) の filter を `fake` の layout cold test から、`command_instant` を使う `start_rx` unit test に変更。`control.rs:405` がその呼び出し。`ur_25_a_start_rx_takes_the_line_lock_once_for_its_instant_and_its_booking` (`control.rs:2606-2631`) に requested instant と `Kind::Start` item の `e` が一致する assertion を追加し、mutant を **killed**。

## 検証

| Check | Result |
|---|---|
| `timeout 300 cargo test -p ezsdr-radio-uhd --lib --no-fail-fast` | 237 passed |
| `timeout 300 cargo test -p ezsdr-radio-uhd --test fake --no-fail-fast` | 265 passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `rules.py --base HEAD --renames ...` | 0 problems |
| Mutation source exact-count check (maintenance, phase3–7) | every active `old`/`old2` occurs exactly once |
| Targeted remote mutation (P265, P424, P446, P450, P464, P469, R04, R24) | 7 killed; P424 survived as described above |

P282 は削除対象として許可された行なので remote run には含めていない。full mutation list は orchestrator の既存 `mutation-all.log` にある1252/1262から再実行していない。self-review は指示どおり行っていない。

既存の修正・orchestrator 管理ファイルは保持した。`fix2-brief.md`、`checklist.md`、`sol*.log`、`mutation-all.log` は変更していない。作業開始時から変更済みだった `plan/phase8` と `handoff.md` はこの修正では触っていない。stage/commit/push/stash はしていない。
