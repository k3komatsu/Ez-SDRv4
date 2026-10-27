STATUS: done

Stage 3e is done apart from the self-review, which produced no findings. Tests, clippy, rules, `git diff --check` and the 75 mutation rows pass. Nothing was committed, staged or stashed, and no hardware was touched.

**What was built**
- **§2.1 flaky tests:**
  - The late-burst `time_errors` assertion is removed from `ur_25_an_enable_from_no_stream_configures_once`, `ur_25_tx_channels_from_zero_transmits_the_next_burst` (renamed from `…_on_time`) and `ur_25_tx_channels_to_zero_ends_the_clock_and_a_re_enable_starts_a_new_one`.
  - `ur_22_a_send_cut_short_ends_the_device_burst_after_what_it_took` is now a controlled-clock unit test in `src/provider/tx.rs` (test-only change). The fake version is deleted; B18, B19 and B25 are `test_bin: lib`.
  - With a 6 ms sleep injected into `submit` in a copy outside the repo, the three fake tests passed 3 of 3 runs.
- **§2.2 queue depth:** a new `Ambiguous` class for intervals that straddle the target. The value comes from successes only, and the call limit is 512. Trials alternate the delay between the plan's and two thirds of it, and the observation records the ambiguous count and whether the blocked-call number agrees across delays. Test `the_twelve_historical_blocked_returns_are_classified_by_their_interval` gives 6 successes, 3 ambiguous and 3 incomplete.
- **§2.3 duplex:** duplex stages are removed from the throughput stage lists. After both values are decided, one pair runs for the throughput duration on the streams and rates that held each direction's value, and decides no value. Its observation is `provider_duplex_at_the_measured_limits`; it is not run if either value is `unmeasured`.
- **§2.4:** the capacity row names a condition that succeeded at T, preferring the one whose next step stopped the climb. The blocker is recorded separately.
- **§2.5:** when UR-7's retries are used up inside a record, the record latch is set. A probe's `set_sources` now takes the same retries.
- **§2.6:** a stage's `Err` or panic ends the climb and keeps the earlier stages.
- **§2.7:** the two `uhd.rs` stream-open calls read their error text from the streamer handle. A stub test in docker passes, and a mutated copy fails it for both rx and tx.
- **§2.8:** `hw_ma1_phase_after_a_pre_start_tune` is added, rehearsed on the fake. Design 18 UR-24 has the MA-1 Ceiling, and `KNOWN_LIMITATIONS.md` has KL-32.
- **§2.9 and the coordinator's addition:**
  - `run-record.sh` has `--tmpfs` and `CARGO_HOME` in both docker arrays, and writes `bench-host.txt`.
  - `bench-host.sh` also prints governor, C-states, NIC IRQ affinity and `ethtool -c` coalescing (read-only).
  - `bench.md` has the F11 fixes, F6 and `UR-7 retry 3` notes, new time estimates, and the day's runbook section.
  - `benchmark_rate` now has the three 200 MS/s runs (normal priority, `--priority high`, `--tx_spb 1000`) with spp recorded, a `UHD_LOG_CONSOLE_LEVEL=debug` read of the TX buffer capacity, and a step for reading `blocks` from the record. The env name exists in libuhd 4.10; the log wording and whether `blocks` appears in the observation are marked unverified.

**Changes from the brief**
- **Two `ur_21` tests left unfixed:** `ur_21_an_untimed_send_is_on_time_after_its_delivery` has the same cause but on-time is its property, and mutation rows U08 and U14 depend on it. It fails under the injected delay. Fixing it needs a controlled-clock unit test (rule 4), which I did not write. `ur_21_an_untimed_burst_after_a_cold_change…` has about 48 ms of slack and passes under 6 ms.
- **Server loopback not touched:** `protocol.rs`'s loopback runs on the default MockRadio with a simulated clock, so it cannot flake this way.
- **Queue success count:** I kept the existing five-successes-per-condition requirement. Only about 6 of the 12 historical readings would count as successes, so the queue depth will likely still come out `unmeasured` on the bench. This needs your decision.
- **KL number:** KL-28 was already taken, so the entry is KL-32.
- **MA-1 harness:** a Session action has no `at`, so the "set again at one instant" variant runs as a Spec Run; the "before T0" variant is a Session. Both read the same burst's phase.

**Bench time estimate**
- One-channel layout (#1+OBX): about 50–65 minutes, down from 65–80. Each direction has 7 sweep stages, and the pair is one more stage.
- Two-channel layout (#2): about 1 h 25 min – 1 h 45 min, down from 1 h 40 – 2 h. Each direction has 27 stages, of which about 25 run, plus the pair.
- The MA-1 probe adds about 3 minutes.

**Test and gate results**
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- Docker clippy (`--features uhd`) and the docker `ffi_tests` pass.
- 20 repetitions of the timing-dependent set (the three fixed tests, the MA-1 tests, the pair tests, the queue-alternation test, and the tx.rs unit test) had 0 failures. I did not run the 73-second record rehearsal 20 times; it passed once in the workspace run.
- Mutation: `remote_mutate.py maintenance,phase7 sim02:4,sim03:4` over 75 rows, 75 of 75 killed. The rows are P254, P255, P368, B18, B19, B25, the touched existing P8xx and P9xx rows, and the new P999–P1017. P807, P808, P829, P842, P843 and P926 were re-aimed because their old text changed. All `old` strings match exactly once across every list.
- `rules.py --base HEAD` with `renames-3e.txt` reports 0 problems.

**Self-review**
The free model hit its limit, and the Go model (`opencode-go/deepseek-v4.1-flash#max`) died on a cancelled web search with no final message. Neither run produced findings, so there are none to fix or reject. The failed free-model log is saved as `review-3e-1-free-failed.log`.

**Left open**
- `tests/README.md` was not updated with the one-line rule the flaky-test report suggested.
- MockRadio (MR-34), D9b and `JOIN_BOUND` are untouched.

Key paths:
- `/home/komatsu/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/common/measurement.rs`
- `/home/komatsu/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs`
- `/home/komatsu/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/tests/common/mod.rs`
- `/home/komatsu/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/src/provider/tx.rs`
- `/home/komatsu/local_work/Ez-SDRv4/crates/ezsdr-radio-uhd/src/uhd.rs`
- `/home/komatsu/local_work/Ez-SDRv4/design/18-uhd-radio.md`
- `/home/komatsu/local_work/Ez-SDRv4/plan/phase8/bench.md`
- `/home/komatsu/local_work/Ez-SDRv4/plan/maintenance/tools/mutations.json`

Logs are in `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/` (`mut-3e.log`, `workspace-test-3e.log`, `rep3e-summary.log`).

Risk: the MA-1 and `benchmark_rate` steps transmit and need the att confirmation; the `benchmark_rate` gain and frequency are unknown.

---

STATUS: done

The earlier summary said the self-review produced no findings. That was wrong: the Go-model review had still been running and finished afterwards (`review-3e-1.log`). Its findings are handled below. It listed 1 P1, 3 P2 and 6 P3.

**Review findings**
- **P1, Ceiling wording (fixed):** a Session's `set` has no `at`, so "set again with `at`" was not runnable by a Session user. `design/18-uhd-radio.md` UR-24, `KNOWN_LIMITATIONS.md` KL-32 and `bench.md` now say a Spec Run schedules the frequency again with an `at`.
- **P2-2, three same-form flaky tests (rejected):**
  - `ur_21_an_untimed_send_is_on_time_after_its_delivery` has on-time as its property. The mutation rows U08 and U14 and KG-8 depend on it. It does fail under the injected 6 ms delay. The right fix is a controlled-clock unit test, which I did not write.
  - `ur_21_an_untimed_burst_after_a_cold_change…` has about 48 ms of slack and passed under 6 ms.
  - The `protocol.rs` loopback runs on the default MockRadio with a simulated clock, so it cannot flake this way.
- **P2-3, `stall_limit_ns` (fixed):** the observation now says half of each trial's own `ahead_ns`, not a single 1.5 s value.
- **P2-4, ffi test not in the gate (already done):** I ran it in the docker image, and it passes. A mutated copy that reverts either stream-open call to the device handle fails it.
- **P3-5, benchmark_rate one direction (rejected):** `--help` says `--rx_rate` alone is RX-only and `--tx_rate` alone is TX-only.
- **P3-6, P3-7, P3-8, P3-10 (fixed):** `bench.md` says the record is written before the profile comparison and names `EZSDR_D9_P5_CASE=timed|untimed`. The queue unmeasured reason now says stage 3e. `bench-host.sh`'s NIC-absent summary line carries the new fields.
- **P3-9, MA-1 probe observation (fixed):** the probe is not one of the record's raw probes, so nothing would have collected its observation. It now only prints its result.

After these fixes, clippy, the queue and `the_twelve…` tests, `rules.py` (0 problems) and `git diff --check` pass, and every mutation row's `old` text still matches exactly once. I did not rerun the 75-row remote mutation pass or the 20-run repetition after the fixes. The edits are one comparison-neutral text change in `measurement.rs`, a print in `hardware.rs`, and docs and script text.

The coordinator's addition (three 200 MS/s `benchmark_rate` runs with spp recorded, the UHD debug-log read of the TX buffer capacity, the `blocks` read, and `ethtool -c` coalescing in `bench-host.sh`) is done in `bench.md` section (d) and `bench-host.sh`. The `UHD_LOG_CONSOLE_LEVEL` name exists in libuhd 4.10. The log wording and whether `blocks` appears in the observation are marked unverified.

Everything else in my earlier report stands. The main open point is that the queue depth will probably still come out `unmeasured` on the bench, because I kept the five-successes-per-condition requirement. That needs the orchestrator's decision.
---

STATUS: done

3e 修正 1 回目．7 件すべて「直した」．

1. Major 1（TRACE）：直した．bench.md (d) は `UHD_LOG_CONSOLE_LEVEL=trace`．image の libuhd が `UHD_LOG_MIN_LEVEL=1`（TRACE を含む）か `strings` で確かめる手順を先に置いた（2026-10-28 に image で 1 を確認）．行が出ないときは UHD の source の既定値 262144 bytes を記録し「実機では未確認」と書く．
2. Major 2（blocks）：直した．`tx_throughput_evidence` が各 burst 行の `blocks` を `burst_blocks` として返し，1 channel の `provider_tx_throughput_<rate>` と 2 channel の stage の `streams[].burst_blocks` に入る．fake の rehearsal 2 本が非空と 1 以上を assert．bench.md の「未確認」を実際の field 名（`burst_blocks`，`jq` つき）に置換．行 P1021 を追加．
3. Major 3（ur_21 2 本）：直した．`ur_21_an_untimed_send_is_on_time_after_its_delivery` と `ur_21_an_untimed_burst_after_a_cold_change_is_on_time_at_its_clock_s_origin` を `src/provider/control.rs` の制御時計の unit test に移した（前者：receipt + 5 ms で stamp，1・3 ms 後に読むと on time，4 ms 後は late で drop．後者：origin に stamp，1 ms 後は on time で k=0，49 ms 後は late）．fake 側は thread の時機に依らない 2 本：`ur_21_untimed_sends_are_each_played_or_flagged`（burst + drop = 20），`ur_21_an_untimed_burst_after_a_cold_change_starts_no_earlier_than_its_clock_s_origin`（target と TIME_ERROR の deadline が origin 以後）．U14 は `test_bin: lib`，J34 は新しい fake 名に当て直し（最初は生き残ったので，TIME_ERROR の deadline の assert を足して kill）．spec 18 の該当の Test 行・§6 の行を直した．server の loopback はそのまま．
4. Major 4（容量の label）：直した．`FamilyStage` に rate を除いた `family`（`rx-sweep`，`rx-grouped` など）を持たせ，止めた段と同じ family の T で成功した段の実際の ID を返す．試験は `throughput_stages` が作る実 ID（`rx-grouped-100m` など）．design/18 の文を実装に合わせた．P1003 を当て直し，P1018（family の照合）を追加．
5. Minor 5（P1010）：直した．P1010 は「理由を残す」行に改名．unmeasured の値で pair を作って走らせる mutation P1019 を追加し，試験で unmeasured のとき観測が `ran: false`・装置の呼び出し 0 を assert して kill．
6. Minor 6（weak loop）：直した．`tone::phase_verdict`（gain < 0.002 は "too weak"，動く位相は "not steady"）に判定を集め，ma1 の道もそれを使う．試験で閾値より下（gain 0.001）と動く位相の理由を assert．P1020 を追加．
7. Minor 7（design/18 の queue の規則）：直した．「1 回の装置の時刻の読みを 2 つの host の時刻で挟む」に直し，各条件の成功の数の要件を書き，試験名は字下げした `*Test:*` 行に分けた（coercion の文も別行）．

確かめたこと
- `cargo test --workspace`：全 68 の test result が ok．`cargo clippy --workspace --all-targets -- -D warnings`：通る．docker の clippy・ffi の試験は uhd.rs が変わっていないので再実行していない（前回通過）．
- `rules.py --base HEAD --renames renames-3e.txt`：0 problem．`git diff --check`：clean．全 list の mutation `old` が tree でちょうど 1 回一致．
- 時刻に依る試験の 20 回（ur_25 の 3 本，ur_21 の fake 2 本，MA-1 の 2 本，pair 3 本，queue の交互，lib の ur_21 unit）：失敗 0．J34 を kill した追加の assert 入りの ur_21 の fake 2 本は別に 20 回，失敗 0．
- mutation（remote_mutate，maintenance,phase7，sim02:4,sim03:4）：83 行（新しい P999–P1021，当て直した行，touched code・killing test に当たる行，B18・B19・B25，U14，J34）で 82 が kill，生き残った J34 は assert を足して単独で再実行し kill．

残ったこと
- queue の深さが `unmeasured` になりうる点（5 回成功の要件を残した）は前回報告のまま，orchestrator の判断待ち．
- 自己レビューは 1 回で止めた（前回の報告のとおり）．
