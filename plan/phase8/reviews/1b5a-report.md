STATUS: blocked: default profile cannot assign TX c0 to stream 0 within the 1b-5a file boundaries

# 1b-5a 実装報告

## 1. §2 の実装

- **既定 profile**：selector に `keys: "channel"`、capture feed / link に `rx0` を設定。`sim.channel` coupling の論理 channel 番号は変更していない（`crates/ezsdr-server/src/catalogue.rs:189-215,223`）。UHD protocol fixture も layout selector / `rx0` を使う（`crates/ezsdr-server/tests/protocol.rs:716-735`）。
- **Python の channel/stream API**：channel key の生成、1 setter = 1 `SetParameter`、layout key 不在時の `Error` は `python/ezsdr/session.py:187-222,199-212`。stream `sample_rate` と effective membership から読むだけの `channels` は `:230-251`。selector から channel 数を作る箇所は `:206-212`。
- **alias / target / `next_at` / recorder / repeat**：`Radio` の stream-0 alias と stream が無い場合の `Error` は `session.py:396-418`、Session shortcut は `:591-611`。stream stop は `:249-251`、RX start / stream clock 選択は `:272-310`、`rx<k>` recorder の選択は `:328-337,614-623`、TX repeat の membership 数検査と target は `:377-389`。`aligned` の instant の扱いは維持（`:494-506`）。
- **examples**：`minimal.py:9-12` は TX c0 の membership を明示して layout stream API で repeat/capture。`sweep.py:14,22` は RX c0 key と `rx0`。`bench_loopback.py:25-39` は TX c0 key と stream API / sample-rate alias。package doc も `python/ezsdr/__init__.py:5-9` で更新。
- **Vision/spec 16**：Vision §3 は `design/vision/01-purpose-and-core-boundary.md:53-72,110`、§57 は `design/vision/10-implementation-path-and-testing.md:34-40`、§66 は `design/vision/11-boundaries-invariants-and-roadmap.md:167-171`。Revision history は `Ez-SDR_v4_ARCHITECTURE_VISION.md:164`。EA-9 JSON、EA-16/17 API・tests、Changes 行は `design/16-easy-api.md:45-68,78-111,123-167,186-205`。

## 2. テスト

- `python/tests/test_easy_api.py:155-174` — RX/TX channel key の read/write、stream rate、membership の方向、setter ごとの Action。
- `:176-196` — read-only `channels`、Radio/Session stream-0 alias、Session shortcut の方向が異なる sample rate でも一致。
- `:198-216` — stream 0 が無い alias の `Error`、stream 1 の RX start/stop と TX repeat target。
- `:218-234` — repeat の channel 数 mismatch が両数つき `Error` になり start_repeat を送らない、stream 1 capture が `rx1` recorder に書く。
- `:736-757` — `next_at` が s1 clock を選ぶ、layout key がない場合の `Error`。
- 既存の `test_ea_16_rx_stop_then_start` / timed/aligned/capture tests は stream-0 alias 経由で維持。
- `crates/ezsdr-server/tests/protocol.rs` の旧 keys/ports は channel/stream keys と `rx0` に移行（例 `:298-312,339-370,716-735,827-848`）。protocol binary は 43 tests passed。

## 3. 検証

- `cargo test --workspace --no-fail-fast`: passed, 1,569 tests; `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Python 3.14 suite: 55 passed。Python 3.9 container suite: 55 passed。
- `rules.py --base HEAD` 単独は、改名した `test_ea_16_repeat_sets_the_channel_count_only_when_it_differs` の引用1件で失敗。rename record (`renames.txt`) を渡した `rules.py --base HEAD --renames .../renames.txt` は 0 problems、`rules.py --self-test` も passed。
- `git diff --check`: passed。指定範囲の旧 direction key / bare port `rx` grep: 0 hits。`crates/ezsdr-radio/src`、`crates/ezsdr-mock-radio/src`、`crates/ezsdr-radio-uhd/src` は無変更。
- **Remote mutation** (`maintenance,phase6 sim02:3,sim03:3`, COMPILE ERROR を生存扱い)：

| row | result | row | result |
|---|---|---|---|
| maintenance/F03 | killed | phase6/F03 (同 ID collision) | killed |
| maintenance/P01 (同 ID collision) | killed | phase6/P01 | killed |
| maintenance/P11 (同 ID collision) | killed | phase6/P11 | killed |
| P528 | killed | P529 | killed |
| P530 | killed | P531 | killed |
| P532 | killed | P533 | killed |
| P534 | killed | P535 | killed |
| P536 | killed | P537 | killed |
| P538 | killed | P539 | killed |
| P540 | killed | P541 | killed |
| P542 | killed | P543 | killed |
| P544 | killed | P545 | killed |
| P546 | killed | P547 | killed |

Combined selection ran 26/26 rows; same ID `F03`, `P01`, `P11` exists in both named lists, so it also exercised those three unchanged colliding rows. After the final layout-profile test fixture edit, P547 was rerun separately and killed (1/1). Logs: `mutation.log`, `mutation-final-fix.log`.

## 4. セルフレビュー

- Round 1 (`review-1.log`): P1 was TX/RX membership test symmetry; fixed by asserting TX s0 `[0]`, s1 `[]` while RX membership differs, with mutation P544. P2 stream-1 targets and Vision §66 snippet; fixed with a stream-1 action test and updated snippet. P3 next_at fixture order, asymmetric ideal counts, stale comment, zero-stream alias error, checked-by rows, and EA-19 snippet correspondence; fixed.
- Round 2 (`review-2.log`): **P1 なし**。P2 Session shortcut direction observation and rules rename; fixed with different RX/TX s0 rates and the documented rename file. P3 legacy profile missing layout key, alias-without-stream spec, A6 snippet citation, and `Rx.capture` docstring; fixed by `Error` validation, spec text, current snippet reference and stream-specific wording. No third review was run because P1 was absent.

## 5. 行数

Diff counts for implementation files: server profile +3/−2, Python client +115/−45 (118 added, 47 removed). Tests: protocol +17/−16, Python tests +156/−38 (173 added, 54 removed). Examples, Vision, spec and mutation records are separate from those counts.

## 6. 穴

**Required default TX membership remains unmet.** `Description::defaults()` assigns RX c0 to s0 only and all TX channels to `-1` (`crates/ezsdr-radio/src/channels.rs:515-535,958-960`). `channels::from_selector()` accepts physical layout values but no membership override (`:150-208`). In 1b-5a, the Radio crates are forbidden to change and the default-profile content is limited to `keys` and port updates; `sim.channel` couplings refer to logical channels and cannot assign stream membership. Therefore the server default does not have TX c0 in s0, and `sdr.tx.repeat(x)` alone raises the intended channel-count mismatch. Examples/tests explicitly set `tx_channels[0].stream = 0`; this does not satisfy the default-profile requirement. A later owner/orchestrator decision must change the permitted boundary (for example, the Radio default or a profile mechanism) before this stage can be marked done. No commit, push or `git add` was performed.

---

STATUS: done

# 1b-5a 修正の回1 報告

## 指摘と修正

- **Sol 1 / Opus 1 — selector 依存を除去**：`_channel_count` を削り、effective の `radio.<d>.c<n>.stream` / `radio.<d>.s<k>.sample_rate_hz` key から一覧を構築する `_indices` に置換。`Radio` は `session.effective[radio]` を一度読み、channel・stream の各配列を作る。`_Stream.channels` も1つの effective snapshot から所属を読む（`python/ezsdr/session.py:199-214,233-249,405-410`）。
  - 再現テストは selector が空で、effective に RX c0/c1+s0/s1、TX c0+s0 を与え、一覧長 `2,2,1,1`、TX s0 alias、構築時と `channels` 読取時の各1 status read を確認（`python/tests/test_easy_api.py:752-769`）。spec 16 は番号一覧の出所を effective key と記載（`design/16-easy-api.md:86-87,154`）。P545 をこのテストへ、P533/P544 は effective membership の判定へ当て直した。
- **Opus 4 — missing-key error**：メッセージを `EA-16: <radio> has no key <key>` に変更（`python/ezsdr/session.py:199-203`）。selector の語を含まず、欠落 key を指定するテストを追加（`python/tests/test_easy_api.py:771-775`、spec 16 `:155`）。P547 をこのテストへ当て直した。
- **Opus 2 — aliases に戻す**：§3・§57・§66 と examples の repeat/capture を `sdr.tx.repeat(x)` / `sdr.rx.capture(N)` に戻し、TX membership の明示設定だけを残した。channel 設定は番号付き channel API、sample rate は stream-0 alias（Vision Part 01 `:53-72,110`、Part 10 `:34-40`、Part 11 `:167-171`、`python/examples/minimal.py:9-12`、`bench_loopback.py:25-38`）。
- **Opus 3 — docstring**：`stop()` の RX停止/TX再送の説明（RM-16/RM-21）を戻し（`python/ezsdr/session.py:251-255`）、`Tx.repeat` の PPS edge（UR-7 / `next_pps`）の説明も戻した（`:381-388`）。
- 裁定に従い、既定 TX membership を要求する以前の block は撤回済みとして扱った。Radio の既定値は変更していない。

## 確認結果

- Python 3.14: `python -m unittest discover -s python/tests` — **56 passed**。
- `cargo test -p ezsdr-server` — **43 passed**。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames .../renames.txt` — **0 problems**。`rules.py --self-test` passed。全 mutation row の `old` はちょうど1回一致。
- Mutation remote gate（`maintenance,phase6 sim02:3,sim03:3`）：
  - P533 — killed
  - P544 — killed
  - P545 — killed
  - P547 — killed
  - P548 — killed
  - 合計 **5/5 passed**（COMPILE ERROR / survivor なし）。ログ: `fix1-mutation.log`。
- `git diff --check` passed。`crates/ezsdr-radio/src`、`crates/ezsdr-mock-radio/src`、`crates/ezsdr-radio-uhd/src` は無変更。
- セルフレビューは指示どおり実行していない。commit / push / `git add` なし。

## 置き場の確認

指定された `opus-notes.md` と `checklist.md` は読み取りのみ。`impl-brief.md` は作業 dir に存在しなかったため、作成・変更していない。

---

STATUS: done

# 1b-5a 修正の回2 報告

この回の開始時点で silence assertion と `wait_for_tx_signal` は未 commit diff に含まれていたため、ソース変更は追加せず、旧系列をprobeで再現してから対象テスト・gateを検証した。

## silence の原因と修正

- **`test_ea_17_two_channels_round_trip`**（`python/tests/test_easy_api.py:340-355`）：RX c1 と TX c0/c1 の stream membership を個別の cold Action で更新するため、TX s0 に clock segment が2つできる。probe の `status.sample_clocks` は T0=2,000,000,000 ticks に対し TX s0 origin が 2,050,000,000 と 2,100,000,000。旧 `sleep(0.005)` 後の capture は最新 origin より前で min/max magnitude とも0だった。現在は最新 TX s0 origin の後、x3xx TX path delay 45 samples を越えた次 sample まで Run を `wait_until` する（`:70-78,351`）。
- **`test_ea_17_requests_made_ahead_capture_contiguous_samples`**（`:430-446`）：TX c0 の所属変更後、repeat の stream clock は T0+50 ms。旧5 ms待ちは送信開始前なので、連続性はあっても2 captureとも silence だった。旧系列を再実行した probe でも `(1000,)` / `(1500,)` の両 capture は min/max magnitude が0。今は capture request を2本連続で出す直前に TX s0 の実効 origin＋46 samples まで Run を進める（`:435-444`）。requests の順序と HD-10 の連続性 assertion は維持。
- **`test_v58_01_the_same_script_under_another_profile`**（`:318-336`）：default x3xx は TX s0 を T0+50 ms から開始するのに、共有 script が5 ms後に capture していた。probe では default の capture は min/max=0、ideal profile は TX origin がT0で信号あり。共有 script を `wait_for_tx_signal` 経由にし、両 profile で effective stream clock の後から capture する（`:318-335`）。
- **silence の防止**：`rotation` は offset 算出より先に `np.min(np.abs(y)) > 1e-6` を assertion し、docstring も non-silent capture を前提とする（`python/tests/test_easy_api.py:41-48`）。wait helper は `status.sample_clocks` の最新 TX s0 origin とその clock の root ticks/sample から目標 instant を作り、Run time の `wait_until` を呼ぶ（`:70-78`）。probe では更新後の capture の min magnitude が約 `5e-4`。

## その他の信号比較の grep

`python/tests` を `rotation(`、`np.allclose` / `assert_allclose`、`correlate`、magnitude check で検索。waveform offset/phasor を比較するのは `test_v57_repeat_then_capture_in_software`、`test_v58_01_the_same_script_under_another_profile`、`test_ea_17_two_channels_round_trip`、`test_ea_17_requests_made_ahead_capture_contiguous_samples`、timed repeat/aligned/next_at の各 test（`test_easy_api.py:104,334-335,355,444,476,500,519`）。すべて `rotation` を通り silence assertion が適用される。他の直接の `assert_allclose` は timed repeat / aligned の**意図した先頭 silence**を確認するもの（`:465-466,489-490`）で、silence を waveform match として扱う比較は見つからなかった。

## 確認結果

- `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=... python -m unittest discover -s python/tests`: **56 passed**。
- `cargo test -p ezsdr-server`: **43 passed**。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD --renames .../renames.txt`: **0 problem(s)**。
- mutation（remote のみ）:
  - phase6/P02 — killed
  - phase6/P03 — killed
  - maintenance/J186 — killed
  - maintenance/P02 — killed（`P02` ID collision により decimation mutant も選択）
  - maintenance/P03 — killed（`P03` ID collision により grid mutant も選択）
  - 合計 **5/5 passed**、survivor / COMPILE ERROR なし（`fix2-mutation.log`）。全 row の `old` は対象 source にちょうど1回一致。
- remote mutation は最初の2回、mutation JSON と変更 source の mode 600 により container が copy/read できず、mutant 実行前に失敗した。追跡ファイルを mode 644 に戻して同じ remote gate を再実行し、上記5行すべて killed を確認。
- セルフレビューは指示どおり実行していない。`cargo fmt`・local mutation・commit・push・`git add` は行っていない。
