STATUS: done

# 1b-4e-1 実装の報告 — 複数 stream の VH-8 harness

## 1. 足したテスト（指示書 §3 の番号順）

すべて `crates/ezsdr-acceptance/tests/parity.rs`。同じ selector と同じ Spec を `MockRig`（MockRadio）と `UhdRig`（UhdRadio + FakeDevice）に通し、各々の T0 からの相対で比べる。layout は rx 2 channel ＋ tx 2 channel（`0:A:ubx:0`、`0:B:ubx:0`）で、stream ごとに channel 1 本。

| §3 | テスト | path:line | 比べたもの |
|---|---|---|---|
| 1 | `vh_8_two_streams_each_prepare_alike` | `parity.rs:1048` | `effective` 全体、宣言した SampleClock の `(node, ratio)` 4 本（`radio/rx/s1`・`radio/tx/s1` を含む）。期待値も絶対値で固定 |
| 2 | `vh_8_a_stream_one_rate_change_cuts_that_stream_alone` | `:1072` | stream 1 の `cold` rate 変更（1→4 Msps、e=+100 ms）。全 clock 行 `(node, ratio, 相対 origin, 相対 end)`。stream 0 の clock は切れない（`ended: None`）。stream 1 の次は cut の 55 ms 後 |
| 3 | `vh_8_a_stream_one_join_leaves_the_same_ready_gap` | `:1093` | `-1` の channel が動いている stream 1 に入る（e=+100 ms）。cut から次の origin まで 55 ms（受信 call 2 ms＋allowance 3 ms＋lead 50 ms） |
| 4 | `vh_8_a_channel_less_stream_one_starts_the_same_first_segment` | `:1118` | channel の無い stream 1（clock 無し）に最初の channel が入る（e=+200 ms）。最初の clock が e に始まる（両方） |
| 5 | `vh_8_a_transmit_stream_one_has_the_same_clock` | `:1147` | 送信 channel を stream 1 で有効にしたときの clock（e=+200 ms）。(node, ratio, 相対 origin) が一致し、stream 0 には clock が無い |
| 6 | `vh_8_the_instance_floor_crosses_streams` | `:1172` | stream 0 の未効 `cold` 変更（e=+150 ms）が stream 1 の変更（要求 +100 ms）を同じ床へ押す。`LATE_COMMAND` の (requested, applied)＝(+100 ms, +150 ms) と、両 stream の cut/origin |
| 7 | `vh_8_a_stream_one_refusal_leaves_stream_zero_alone` | `:1198` | stream 1 の antenna 変更を RM-31 が拒否。`COMMAND_REJECTED` の理由（Mock の `MR-18: ` を除いて一致、`RM-31:` prefix）、`coerce` の拒否 key、両 stream の clock が不変 |
| 8 | `vh_8_the_same_manifest_plan_rows` | `:1257` | stop 後の UHD `ezsdr.radio.uhd.timing` の `plan` 行と Mock `ezsdr.radio.mock.segments` 行を、`(direction, stream, 相対 origin, channels)` で比較（方向は配列名と `direction` 列で吸収）。5 segment を絶対値で固定 |
| 8（sequence） | `vh_8_the_same_manifest_sequence`（`#[ignore = "divergence: …"]`） | `:1282-1283` | `sequence` 列のみ。最初の segment の `null` は一致、command の番号が 1 違い（下記 §2） |

helper（指示書 §4）:
- `MockRig::with_ports` / `UhdRig::with_ports`（`parity.rs:208`・`:325`）: 指定した受信 port に link を付ける。既存 `new` は `&["rx0"]` への委譲で、既存 `vh_8_*` の意味は変えない。
- `Started`（`:957`）: 両 rig の起動（Mock は `arm_start`、UHD は `arm`→T0→`start`）、T0 の決定、`cold` 更新の同期、stop、相対 clock 行（`relative`）。
- `mock_segments` / `uhd_plan`（`:913`・`:933`）: Manifest の行の読み出し。
- `late_rows` / `rejected_reasons`（`:731`・`:740`）: 既存 2 テストにあった同型の閉包をここへ寄せ、新しい 2 テストもこれを使う。
- `planned`（`:1243`）: plan の 2 テストが共有する 3 つの更新と stop。
- `Queue::queued`（`:144`）: UHD の control thread が Action を取ったことを待つ（device clock が止まっている間に）。

## 2. 違い（divergence）

**Manifest の `sequence` の番号（1 件）** — `vh_8_the_same_manifest_sequence` を ignore にした。

- MockRadio: 最初の booked command の segment の `sequence` が `0`。`crates/ezsdr-mock-radio/src/lib.rs:1838`（`let order = self.next_order;`）、`:1902`（加算）、初期値 `:2527`（`self.next_order = self.faults.len() as u64`、fault 無しで 0）。
- UHD: 同じ command が `1`。`crates/ezsdr-radio-uhd/src/provider/core.rs:249-252`（`next_seq` が加算してから返す）、行は `:432` の `"sequence": segment.by`。
- 規則: `design/09-mock-radio.md:181`（MR-27）と `design/18-uhd-radio.md:302`（UR-30）はどちらも「その segment を始めた command の arrival order」とだけ書き、番号の起点を定めていない。UHD の単体テスト `ur_30_the_recorded_plan_has_no_refused_segment`（`crates/ezsdr-radio-uhd/src/provider/control.rs:1806-1812`）は 2 番目の command を `Some(2)` と固定している（Mock 側は値を固定するテストが無い）。
- どちらが spec に合うかは、起点を規則が書いていないためこの段階では決められない（両方とも自己整合）。orchestrator の判断を仰ぐ。テストは両者の現値を別々に固定し（Mock `[None, None, Some(0), Some(1), Some(2)]`、UHD `[None, None, Some(1), Some(2), Some(3)]`）、最後の比較で差を示す。最初の segment の `null` と相対順序は一致している。

**違いではなかったもの（起きたこと）**: 送信 stream 1 への channel の entry を当初 e=+200 ms（受付 +150 ms）で比べたところ、UHD の origin だけ 21 ms 遅れた。UHD の ready は「from-none の configuration の終わり」（UR-25）で、Mock の ready は受付（MR-18）なので、e が ready＋lead に近すぎたテスト設計の問題。e=+300 ms に離すと両方 300 ms で一致した。仕様の差ではない。

## 3. 確かめることの結果

- `cargo test --workspace --no-fail-fast`: 失敗 0（`parity` は 17 passed / 1 ignored）。
- `cargo clippy --workspace --all-targets -- -D warnings`: 警告 0。
- `parity` の 3 回連続: 3 回とも `17 passed; 0 failed; 1 ignored`（3.90〜3.91 s）。
- `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`: `0 problem(s) against HEAD`。
- mutation（`REMOTE_CPUS=8 REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:2,sim03:2 …`）:

| 行 | 何を壊すか | filter | 結果 |
|---|---|---|---|
| P485 | Mock が stream 1 の受信 clock を stream 0 の node に宣言 | `vh_8_two_streams_each_prepare_alike` | killed |
| P486 | Mock が stream 1 だけの変更で stream 0 も切る | `vh_8_a_stream_one_rate_change_cuts_that_stream_alone` | killed |
| P487 | UHD の plan 行が常に stream 0 | `vh_8_the_same_manifest_plan_rows` | killed |
| P488 | instance の床が先行する `cold` 変更を無視 | `vh_8_the_instance_floor_crosses_streams` | killed |

  既存の parity を kill 役に持つ行も流した: P333（`vh_8_the_two_providers_prepare_one_layout_alike`）killed、P352（`vh_8_a_floored_refusal_is_reported_in_the_same_order`）killed。P352 は既存テストの閉包を helper へ寄せた後にもう一度流して killed。`BASELINE FAILED` は無し。修正の回の後、P485〜P488 を再実行して 4/4 killed。
- 本体（`crates/*/src`）は変更していない。`git status` は `parity.rs` と `plan/maintenance/tools/mutations.json` の 2 ファイルだけ。

## 4. セルフレビュー（reviewer: DeepSeek，1 回）

ログ: `~/.cache/ezsdr-fixes/2026-10-20-phase8-1b4e1/review-1.log`。**P1 なし**。指示書の「P1 が無くなったら次の回は流さない」に従い、2 回目は流していない。指摘と扱い:

| # | 重大度 | 指摘 | 扱い |
|---|---|---|---|
| 1 | P2 | ignore した sequence テストの期待値が Mock の行に UHD の番号を要求しており、一致している `null` の部分も捨てている | 直した: 両者の現値を別々に固定し、先頭 2 segment の `null` の一致と、最後の比較（差）を残した |
| 2 | P3 | `constraints.retain(...)` が no-op（`rx_two()` に tx key が無い） | 直した: `retain` と `mut` を削除（2 か所） |
| 3 | P3 | `Started::cold` の「booking は now」というコメントが同期の実態より強い（queue empty は poll 前） | 直した: queue empty は control thread が Action を取ったこと、booking の instant はその poll 内で読む、とコメントを正確にした |
| 4 | P3 | `late_of`・`rejected` の閉包が既存と同型、plan テストの 3 更新が重複 | 直した: `late_rows`・`rejected_reasons` を file-level helper にし既存 2 テストも使う。plan の 2 テストは `planned()` を共有 |
| 5 | P3 | 拒否テストの `coerce` request を Mock の `effective` から両方に使っている | 直した: 各 rig の `effective` から作り、`effective` の一致も assert |
| 6 | P3 | T0 の 200-tick lattice が固定で、constraints から導いていない | 直した（コメントで前提を明示）: この比較の rate は 1/2/4 Msps で比が 200 を割る、より粗い stream を足す比較は広げる、と helper の doc に書いた |
| 7 | P3 | 冒頭 doc comment が ignore した sequence 比較に触れていない | 直した: 1 段落を追加 |

退けた指摘は無い。

## 5. テストの行数

- `crates/ezsdr-acceptance/tests/parity.rs`: 881 行 → 1 296 行（diff +458 / −25。既存 2 テストの閉包を helper へ寄せた分の削除を含む）。
- 新しいテスト: 9 本（§3 の 8 比較 ＋ sequence の ignored 1 本）。
- `plan/maintenance/tools/mutations.json`: +40 行（P485〜P488）。

## 6. 穴

- **item 8 の `sequence` は両者で一致しない**（§2）。`stream`・`origin`・`channels` は一致し、テストで固定した。`sequence` は ignore したテストが現値を記録するだけなので、番号を決めるまで cross-provider の比較としては未固定。
- Manifest の行は指示書の 4 列（`stream`・`sequence`・相対 `origin`・`channels`）だけを比べ、`end`/`ended_at`・`ticks_per_sample`・Mock の `phase_behavior`/`coherence` は比べていない（指示書の列に無いため）。
- item 5 は「送信 channel を stream 1 で有効にしたとき」の実行時 entry で比べた。`arm` 時の送信 clock は、Mock の arm が root 0、UHD の arm がその実時間ぶん後（この環境で約 25 400 ticks）になり、T0 からの相対 origin が rig の準備時間でずれるため、比較の軸にしていない（stream 0 の arm 時 clock は既存 `vh_8_the_same_transmit_stream_clock` が見ている）。arm 時の stream 1 の clock を T0 相対で比べるには、arm の瞬間を rig から取れるようにする必要がある（今回は指示書に無い仕組みを足さないため見送り）。
- 送信 stream 1 への entry は、UHD の from-none の configuration が終わる時刻（ready）に依存する。e を受付の 150 ms 後に置いて余裕を持たせたが、負荷で configuration が 150 ms を超えると origin が動き得る（既存の UHD テストも同じ形の実時間依存を持つ）。今回の 3 回の連続実行では揺れなかった。
- 冒頭の doc comment の「per stream (0 and 1)」は、比較が stream 0 と 1 の 2 本ずつであることを指す。3 本目以降の stream は試していない（layout の channel 数は 2 本ずつ）。
