# 段階 3b-1：指示書

## テスト計画（`plan-3b-brief.md`）

あなたは実装役です．今回は**テスト計画を書くだけ**で，コードは変えない（tree は読むだけ）．HEAD は `1a96de4`．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．** `cargo fmt`，mutation の道具，commit，`git stash` を使わない．長い出力はファイルに出して `tail` で見る．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．

### 1. 読むもの

- 設計メモ `plan/phase8/41-measurement-records.md`（全部．§5 と §7 の末尾の「3b の指示書へ」がこの段階）．`plan/phase8/00-overview.md` §4 の P2・P3（特に P3 (1) の読み方，(2) の回数，(6) Module の内部の定数，(6b) Provider 経由と raw）．
- 3a で入った `crates/ezsdr-radio/src/measurement.rs`（`Record`，`ValueRecord`，`Observation`，`compare`）と `crates/ezsdr-acceptance/tests/measurements.rs`．
- `crates/ezsdr-radio-uhd/tests/hardware.rs`：`hw_b8_leads`，`hw_b8_preemption`，`hw_b8_cold_change_capture`・`_low_rate`，`hw_perf_rx_rates`，`hw_b6_delay_by_rate`，`hw_b7_session_loopback`，`hw_b8_stop_end`，raw の `hw_b8_raw_device_lead`，`_restart_lead`，`_queue_depth`，`_in_flight_window`，`_burst_gap`，`_empty_eob_gap`，`_timed_tune_phase`．`tests/common/mod.rs`（`rehearse_*`，`assembly(..., adjust)`，`session`，`bench_profile`），`fake.rs` のリハーサル，`tests/README.md`（時刻の規則）．
- `crates/ezsdr-radio-uhd/src/provider/mod.rs`：`UhdRadio::from_binding`（Description を作る所，`ProviderInstance` の `min_command_lead` を作る所 `:320` 付近），`with_rx_stall`（bench だけが使う builder の前例．「Only Rust code reaches it (GZ-9)」）．`src/profile.rs`（`DEVICE_LEAD_NS`，`DELIVERY_ALLOWANCE_NS`，`terms()`）．`DEVICE_LEAD_NS` と `DELIVERY_ALLOWANCE_NS` を読む所（control・rx・tx）と，`core.description.timing.*` を読む所を grep する．

### 2. orchestrator の決めたこと（この形でテスト計画を書く）

#### 2.1 候補の値の道

- `UhdRadio` に bench だけが使う builder を 1 つ足す（`with_rx_stall` と同じ形と注記）：Description の `timing` の値（`TimingEnvelope`）を差し替え，それから作られるもの（`ProviderInstance.min_command_lead`，capability，Manifest の envelope）がすべて差し替えた値になる．`assembly` の `adjust` から呼ぶ．既定の構築（builder を呼ばない）は今と同じ値と振る舞いのまま．
- **Module の内部の定数 `DEVICE_LEAD_NS`・`DELIVERY_ALLOWANCE_NS` は差し替えない**（P3 (6)：Module の設計の値．Provider 経由の測定はそれを含んだ end-to-end の値を測る）．装置そのものの lead は raw の probe（observation）で見る．

#### 2.2 値ごとの測り方（3b の範囲）

| key | 測定（元のテスト） | 候補 | 読み方（P3 (1)） | 成功とみなす観測 |
|---|---|---|---|---|
| `radio.timing.min_timed_command_lead_ns` | Session の `send` を受け取りから lead L で（`hw_b8_leads`） | admission の値を L に下げる（`DEVICE_LEAD_NS` は 2 ms のまま） | 最小の lead：段 s 以上のすべての段で全試行が成功した最小の s | burst の record が target で始まり，`TIME_ERROR` なし，async に `TimeError`・`Underflow` なし |
| `radio.timing.tx_in_flight_ns` | 走っている repeat への割り込み（`hw_b8_preemption`） | `tx_in_flight_ns` を段の値に | 同上（最小の lead） | 割り込む burst が target で on time（`TIME_ERROR` なし，record の開始＝target，async に late なし） |
| `radio.timing.rx_start_lead_ns` | 受信の `cold` rate change を挟む capture（`hw_b8_cold_change_capture`） | `rx_start_lead_ns` を段の値に | 同上 | capture の連続性が e₁・e₂ で正しく，`LATE_COMMAND` なし（今の assert と同じ） |
| `radio.timing.command_queue_depth` | 新しい：Session で timed の周波数の update を k 個，release window の中に予約 | `command_queue_depth` を段の値に上げる | 容量：段 s 以下のすべての段で全試行が成功した最大の s | すべて `applied` に instant どおり，`COMMAND_QUEUE_FULL`・`LATE_COMMAND` なし |
| `radio.perf.rx_bytes_per_s` | 受信の rate（`hw_perf_rx_rates`） | なし（profile の上限の中で測る） | 容量：overflow なしで持った最大の rate × `wire_bytes_per_sample` | `RX_OVERFLOW` なし，stats の受信 sample 数が期間どおり |
| `radio.full_duplex`，`radio.hardware_time` | loopback（`hw_b7_session_loopback` か `hw_b6_delay_by_rate`） | なし | 真なら真 | 送受信を同時に動かして受信に送った波形が時刻どおりに現れる |

- 上の表にない比べる key は，3b の記録では `unmeasured` と理由（「3c で測る」，「Module の内部の定数（P3 (6)）」，「channel ごとの較正がない」など）．`release_window_ns` は Module の定数として `unmeasured`．
- raw の probe（`hw_b8_raw_*`）と，loopback の合計の遅れ（`hw_b6_delay_by_rate`），`hw_b8_stop_end` は observation．値の行にしない．
- 試行の結果は 4 つに分ける：admission の拒否（submit の結果），Provider の失敗（Provider の event：`TIME_ERROR`，`LATE_COMMAND`，`COMMAND_QUEUE_FULL`，`COMMAND_REJECTED`），装置の失敗（Manifest の `async` の report：`TimeError`，`Underflow` など），成功（上の表の観測がすべて揃う）．「すぐ拒否されなかった」は成功ではない．各段の候補の値と結果の数は observation に残す（値の行の `worst` の根拠）．
- 回数と段は既定値を持ち，環境変数で変えられる（bench は P3 (2) の目安：段の確定に各 100 回，時間は 20 回以上，throughput は 60 s 以上．リハーサルは 1〜2 回）．段の具体の刻みはテスト計画で提案する（今のテストの段を出発点に）．

#### 2.3 スイートの形

- `measure_<名前>(device: Arc<dyn Device>, plan: &Plan) -> Vec<Entry>`（`Entry` は値の行か observation）を `tests/common/` に置く（`rehearse_*` と同じく `fake.rs` と `hardware.rs` が呼ぶ）．
- 観測から値を出す関数（4 つの分類，最小の lead と容量の読み方）は純粋な関数にして，合成した観測で unit test する．今のテストの assert（`cold_change_capture` の連続性など）は共有の関数に移しても残す．
- `hardware.rs` に記録を書く ignored のテストを 1 本：すべての `measure_*` を USRP で流し，`source: "hardware"` の記録を環境変数の出力先に書く．device の serial は `Device::serial()`，FPGA image・UHD の版・host の設定・commit・load は環境変数か probe から（取れないものはテスト計画で決める）．出力先の環境変数が無ければ明示的に失敗する（黙って飛ばさない）．
- `fake.rs` のリハーサル：同じ `measure_*` を FakeDevice（手動の時計）で少ない回数流し，`source: "fake"` の記録を一時ファイルに書いて読み戻し，`compare` の違反が `source` の 1 件だけであること（形・key の網羅・条件が正しい）を確かめる．fake の値の大きさは assert しない．

### 3. 書くもの：`test-plan-3b.md`

risky な段階（AGENTS.md §9：fake と実機の境界）として，次の系列ごとに，テスト（名前，新か既存か），場面の作り方，assert，観測（どの記録・event・section から読むか），落とす誤り（どの誤った実装でこの assert が落ちるか）を表にする：

1. builder を呼ばない既定の構築で，Description の timing・`ProviderInstance.min_command_lead`・Manifest の envelope が今と同じ．
2. builder で admission の lead を下げると，それより短い lead の `send` が admission を通る．`DEVICE_LEAD_NS` より短い実効の lead では Provider が late と決める（Provider の失敗に分類される）．
3. 装置の async の `TimeError` は，admission を通り `TIME_ERROR` が無くても装置の失敗に分類される．
4. 最小の lead の読み方：段ごとの成功・失敗の並び（下の段で偶然通る，途中の段で 1 回だけ失敗，全段失敗，全段成功）から `worst` が上の定義どおり出る．全段失敗なら値は `unmeasured`（理由つき）．
5. 容量の読み方：同じく．
6. raw の probe の結果は observation にだけ入り，値の行に現れない．
7. 記録は比べる key をちょうど 1 度ずつ持ち（測ったものは `measured`，ほかは `unmeasured` と理由），`compare` の網羅と条件の規則を満たす．`conditions` は測った stream・rate・同時運転を正しく書く．
8. リハーサル：FakeDevice での記録が `source: "fake"` で，`compare` の違反は `source` だけ．比較テストの道（acceptance）は fake の記録を拒否する．
9. 出力先の環境変数が無い hardware の記録のテストは明示的に失敗する．
10. 候補で組み立てた Provider の Manifest は差し替えた値を envelope に書く（記録を読む人が候補と分かる）．

加えて：
- 段と回数の既定値の案（bench の時間の見積もりつき）．
- device の版・FPGA image・host の設定・commit をどこから取るかの案．
- `fake.rs` のリハーサルが tests/README.md の規則（試験のスレッドの時刻を判定に使わない）を満たすか．満たさない形になるものは controlled-clock の unit test に移す案．
- mutation の行の案（どの式・分岐を変える変異か，どのテストで殺すか．`old` はコードと一緒に書くので案だけ）．
- 大きさの見積もり．約 1,500 行を大きく越えるなら，3b-1（枠・候補の道・分類と読み方・lead と rx rate と observation）と 3b-2（in-flight，start lead，queue depth，full duplex）に分ける案を書く．
- **INFERRED の印**：UHD や装置の振る舞いのうち推測のもの．
- 読んで分かった，設計メモや上の決めごとと合わない所，決まっていない所（止めずに列挙）．

最初の行は `STATUS: done`（止まったなら `STATUS: blocked` と理由）．**起きたことを書く．していないことは書かない．**

## テスト計画の改訂（`plan2-3b-brief.md`）

あなたは実装役です．今回も**テスト計画を書くだけ**で，コードは変えない．前の計画 `test-plan-3b.md` に Sol がレビューを返した（`sol-plan-3b.log` の最後の `VERDICT` 以降．FAIL，major 7）．orchestrator は 7 件をすべて採り，段階を分ける．**この tree を触っている他の agent はいません．他の process を待たない．** `plan-3b-brief.md` の守ることはそのまま効く．

### 1. 裁定

- **分ける（Sol 7）**：3b-1 は「builder，記録の書き出し，証拠の純粋な規則，raw の probe の observation 化，記録を書く hardware の入口，fake のリハーサル」．Provider を通した測定の群（最小の command lead，in-flight，受信の start lead，受信の throughput，full duplex）は 3b-2 で，Run の使い回しと試行の分け方を測定ごとに設計してから（3b-2 は別の計画とレビュー）．**3b-1 の記録では比べる key はすべて `unmeasured`（理由つき）**．
- **queue の深さ（Sol 1）**：`applied` の時刻は予約した instant で，装置が実際にいつ実行したかの観測ではない．Provider 経由で観測できる道が決まるまで `unmeasured`（「装置の実行時刻を観測する道がない．3c で決める」）．raw の `hw_b8_raw_queue_depth` は observation に残す．
- **admission の floor（Sol 2）**：Session の `send` は `DropAndFlag` で，admission は lead を比べない．builder が admission を変えることは，Spec の schedule の `RejectAtPlan` の burst で確かめる：既定では lead 4 ms が拒否され，候補 3 ms では通る（KC-19，SC-27．`crates/ezsdr-kernel/src/coordinator/pipeline.rs:1098-1126`）．
- **短い lead の実行時の判定（Sol 3）**：Provider が `DEVICE_LEAD_NS` で落とす burst は装置に届かない．`fake.rs` の通しのテストでは確かめず，`provider` の制御された時計の unit test（`tx.rs` の rig）で「Provider の失敗で，装置への送信なし」を確かめる．装置の late は注入した report で別に確かめる．
- **分類は失敗の出所で（Sol 4，5，6）**：結果は 6 つ：
  1. admission の拒否（submit の結果）．
  2. Provider の失敗（Provider が決めたもの：`TIME_ERROR` の outcome が `drop`・`send_asap`・`refused`・`plan_violation`，`LATE_COMMAND`，`COMMAND_REJECTED`，`COMMAND_QUEUE_FULL`）．
  3. 装置の失敗（`TIME_ERROR { late_at_device }`，Manifest の `async` の `TimeError`・`Underflow`・`UnderflowInPacket`・`SeqError*`．`TIME_ERROR` が対にならない async の report も含む）．
  4. 不完全（エラーは無いが期待した完了が無い：record が無い，sample 数や終わりが期待と違う，`rejected` に UR-22 の短い send の note がある）．
  5. 帰属できない（試行の区間の外の report，tick の無い report，二つの試行にまたがる ack など，どの試行のものか決められない証拠がある）．
  6. 成功（上のどれも無く，期待した観測がすべて揃う）．
  成功以外はどれも合格の試行に数えない．**試行の区間**：試行は一つずつ行い，次の試行との間に設定した間隔を置く．tick を持つ report は区間に入る試行のもの，tick の無い report と区間の外の report は「帰属できない」として，その時に開いていた試行（無ければ直前の試行）を不合格にする．
- **読み方**：前の計画のとおり（最小の lead は「それ以上のすべての段が全試行成功の最小の段」，容量は「それ以下のすべての段が全試行成功の最大の段」，全段失敗は `unmeasured`）．試行の数が段の規定に届かない段は成功にしない．複数の条件（cold change の前後など）にまたがる値は，すべての条件で合格した段だけを採る（3b-2 で使う．3b-1 では関数とテストだけ）．
- **metadata**：環境変数が無い，空，不正（load が `none`・`h4` 以外など）なら記録を書かずに失敗する．テストを置く．
- **in-flight の P3 の文言**：overview の P3 (6) は古い（in-flight window は設計メモ 40 で profile の値）．これは orchestrator が直す．計画には書かない．

### 2. 書くもの：`test-plan-3b1.md`

前の計画の形（系列ごとの表：テスト，場面，assert，観測，落とす誤り）で，3b-1 の範囲だけ：

1. builder を呼ばない既定の構築が今と同じ（前の #1）．
2. builder が timing のすべての値を Description・`ProviderInstance.min_command_lead`・capability・envelope の section・Manifest の envelope に届け，Module の実行時の読み手（release window，queue depth，in-flight，start lead，overflow restart gap，startup latency）が候補を読む（前の #10．読み手ごとに一つの観測）．
3. admission の floor：`RejectAtPlan` の burst で既定は拒否，候補は通る（上の裁定）．
4. `DEVICE_LEAD_NS` は候補で変わらない：制御された時計の rig で，候補の admission の lead を 1 ms にしても，2 ms より近い burst は Provider が落とし，装置に送られない．
5. 分類の 6 つ：合成した証拠で，各分類と，対になる `late_at_device`，対にならない async の `TimeError`，Provider の drop，underflow，admission の拒否，record の無い完了，途中で切れた burst（開始は target だが sample 数が足りない，UR-22 の note），区間の外の report，tick の無い report，二つの試行にまたがる ack，本当の成功．
6. 読み方（最小の lead，容量，試行の数の不足，複数条件の合格の交わり）．
7. raw の probe（`hw_b8_raw_*`，`hw_b6_delay_by_rate` の合計の遅れ，`hw_b8_stop_end`）が observation を返し，値の行を作らない．今の print と assert は残す．
8. 記録を組み立てる関数：比べる key をちょうど 1 度ずつ（3b-1 ではすべて `unmeasured` と理由），observation を条件に結ぶ，`compare` の網羅と条件の規則を満たす．
9. 出力先と metadata：環境変数が無い・空・不正で失敗，既存のファイルを上書きしない．
10. fake のリハーサル：FakeDevice（手動の時計）で記録の道を通し，`source: "fake"` の記録を書いて読み戻し，`compare` の違反が `source` の 1 件だけ．試験のスレッドの時刻を判定に使わない（tests/README.md）．acceptance の道は fake を拒否する（3a で済み．参照だけ）．
11. 記録を書く hardware の入口（ignored）：raw の observation だけを持つ記録を書く．

加えて：mutation の行の案，大きさの見積もり（3b-1 は約 1,500 行以内），INFERRED の印，決まっていない所．最初の行は `STATUS: done`．**起きたことを書く．していないことは書かない．**

## 実装（`impl-3b1-brief.md`）

あなたは実装役です．HEAD は `1a96de4` の次（P3 (6) の文言の commit）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．**この tree を触っている他の agent はいません．他の process を待たない．**

### 0. 守ること

`impl-3a-brief.md` §0 と同じ：`cargo fmt`・`rustfmt` 禁止，ローカルの `mutate.py` 禁止，mutant を作業ツリーに当てない，commit・push・`git add`・`git stash` 禁止，version を上げない，SCHEMA_CHANGELOG に書かない，cargo とテストの出力はファイルに出して `tail` で見る，自分の log を全部読まない，負荷をかけない，テストの一時ファイルは /tmp のまま．**起きたことを書く．していないことは書かない．**

### 1. 計画と裁定

計画は `test-plan-3b1.md`（系列 1〜12）．Sol の再レビュー（`sol-plan2-3b.log` の最後の `VERDICT` 以降，FAIL：major 2，minor 1）を orchestrator が次のとおり裁定した．計画のこの部分を置き換えて実装する：

- **系列 3（major 1）を具体にする**．制御された時計の rig で，読み手ごとに：
  - startup latency：`arm` を A で呼び，覚えた境界が A＋候補であること，その直前の `start(T0)` が拒否され，境界以後の `start` が通ること（`provider/mod.rs:538-544,602-617`）．
  - TX と RX の start lead：名前のある操作（`cold` の rate change か，stream の enable）の結果の origin が候補の lead で決まること（`provider/control.rs:247-255`）．RX の missed start の回復も RX の lead を読む（`provider/rx.rs:450`）ので，一つ置く．
  - in-flight：手渡しの境界（`window_cut`）と chunk の大きさ（`send` の `chunk`）は別の読み手（`provider/tx.rs:301-302,404`）．境界は候補どおりであることを見る．chunk の大きさは観測できる assert があれば置き，無ければ置かない理由を報告に書く．
  - release window と queue depth：計画のとおり（予約は未来の時刻，channel は有効なもの）．
- **系列 4（major 2）は消す**．Kernel の admission が `ProviderInstance.min_command_lead` で `RejectAtPlan` を拒否することは Kernel のテスト `kc_19_a_reject_at_plan_burst_with_a_short_lead_is_refused`（`crates/ezsdr-kernel/tests/coordinator.rs:813`）が見ている．3b-1 が示すのは「builder が `min_command_lead` を候補にする」（系列 2）だけでよい．統合テストで 3〜5 ms の境界を作る仕組みは足さない．報告にこの理由を書く．
- **系列 11（minor）を具体にする**．リハーサルは observation を一つ実際に作る：Spec に前もって予約した操作（例：送信の burst と受信の capture の loopback．`rehearse_txrx_and_repeat` の場面か，`hw_b6_delay_by_rate` の場面を FakeDevice で）を流し，その結果の observation の中身が Run の証拠（Manifest の burst の record の target，capture の連続性，相関で求めた遅れなど，その操作が実際に出したもの）と一致することを assert する．hardware の壁時計の測定（`hw_b8_raw_device_lead` の送信の間隔，`hw_b8_raw_in_flight_window` の pacing など）は fake の判定にしない．
- 計画の系列 1，2，5〜10，12 はそのまま．
- `overflow_restart_gap_ns` は UHD Module に実行時の読み手がない（Sol も確認）．値の伝わり方だけを見る（系列 2）．意味は 3c で決める．

### 2. 範囲

計画（裁定で直したもの）の全部．Provider を通した測定の群（最小の command lead，in-flight，受信の start lead，throughput，full duplex）は 3b-2 で，ここでは作らない．3b-1 の記録の比べる key はすべて `unmeasured`（計画の「3b-1 の unmeasured reason」）．builder は `with_rx_stall` と同じ形で，本番の道（builder を呼ばない構築）は変えない．新しい rig の種類，fake の新しい fault，retry，`#[cfg(test)]` だけの本番の道は足さない（builder は bench の Rust のコードだけが使う公開の関数で，`with_rx_stall` の前例どおり）．大きさの目安は約 1,500 行以内．

### 3. 確かめること

- 新しいテストと直したテストを各 20 回．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`rm_20_schema_freeze`．`--features uhd` が要る `hardware.rs` は，`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings` で compile と clippy を確かめる（実機は流さない）．
- mutation：計画の「Mutation row の案」の行を `plan/maintenance/tools/mutations.json` に足し（行を足すだけ，1 space の字下げ，ID は既存の最大の次から），新しい行と，変えたコードや killing test に触れる既存の行を `remote_mutate.py` で流す（`REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...`．前に変えたファイルと新しいファイルを mode 644．全リストの `old` が tree でちょうど 1 回一致）．全リストは流さない．
- spec：`design/18-uhd-radio.md` に builder（bench だけが使う候補の timing）を書く必要があるか判断する．UR-34（bench の手順）か `with_rx_stall` を書いている所に倣い，あれば 1 文と Test 行．Changes に 1 行（設計メモ 41 を引く）．

### 4. セルフレビュー

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3b1.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3b1-<n>.log 2>&1
```

前景で流す．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．P1 以上が無くなるか 3 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

### 5. 報告

`report-3b1.md`．最初の行は `STATUS: done`．作ったもの（ファイルごと），計画から変えた所と理由，系列ごとのテスト名，spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い（model も），残ったこと．

## セルフレビューの checklist（`checklist-3b1.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `1a96de4`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3b1-brief.md`（§1 の裁定が優先）と `test-plan-3b1.md`，設計メモ `plan/phase8/41-measurement-records.md` §5．

見ること（4つ）：
1. **系列とテスト**：計画の系列（1〜3，5〜12．系列 3 と 11 は指示書 §1 の形）に，その場面と期待値のテストがあるか．テストは観測を assert で見ているか（制御された時計の rig で，試験のスレッドの時刻を判定に使っていないか．tests/README.md）．分類の 6 つと帰属の規則が合成した証拠で一つずつ試されているか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．照合が空でも通る形，数だけ見て値を見ない形は指摘．
2. **範囲**：指示書 §2 に対して，足りない所と余計な所．3b-2 の Provider の測定の群，新しい rig の種類・fault・retry，builder を呼ばない本番の道の変化，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：builder が `TimingEnvelope` の 8 つの値すべてを Description・`min_command_lead`・capability・envelope の section・Manifest に届けるか（grep）．記録の比べる key がすべて `unmeasured` で理由つきか．raw の probe の結果が値の行に一つも入らないか．`DEVICE_LEAD_NS`・`DELIVERY_ALLOWANCE_NS` が builder で変わらないか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，コードと食い違う文が無いか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．

## 修正（`fix-3b1-brief.md`）

あなたは実装役です．tree の未 commit の変更は段階 3b-1 の実装です（`impl-3b1-brief.md`，`report-3b1.md`）．orchestrator のレビュー（`orch-3b1.md`）と Sol のコードレビュー（`sol-3b1.log` の最後の `VERDICT` 以降）が出ました．下の裁定ですべて直します．`impl-3b1-brief.md` §0 の守ることはそのまま効く（`cargo fmt` 禁止，ローカルの mutation 禁止，commit 禁止，長い log を読まない，出力はファイルへ）．**この tree を触っている他の agent はいません．他の process を待たない．**

### 1. 裁定

- **Sol 1 = O2（major）条件をまたぐ読み方が，一つの条件だけが試した段の失敗を消す．** 条件ごとに，その条件のすべての段で合格の集合（最小の lead：それ以上の自分の段がすべて合格する段，容量：それ以下の自分の段がすべて合格する段．試行の数の不足は不合格）を先に求め，それから条件の間で交わりを取る．最小の lead は交わりの最小，容量は交わりの最大．交わりが空なら `unmeasured`（理由つき）．Sol の例（A：1 合格，2 不合格，3 合格．B：1，3 合格 → lead 3，容量 1）と，A の 2 が試行不足の場合をテストに足す．
- **Sol 2（major）span が試行の区間からはみ出しても，一つの試行とだけ重なればその試行に入る．** span は一つの試行の区間に完全に含まれる（`start >= trial.start && end <= trial.end`）ときだけその試行に入れる．それ以外（区間の端をまたぐ，二つにまたがる）は重なるすべての試行を「帰属できない」に，どれとも重ならなければ今の `mark_unattributable` の規則で．一つの試行とだけ重なり端をまたぐ span（両側の端）のテストを足す．
- **Sol 3 = O3（major として扱う）hardware の入口が，出力先の失敗を全 probe の後で知る．** `metadata_from_env` か入口の最初で，出力先が既にあれば，あるいは親の dir が無ければ，USRP を開く前に失敗する．最後の `create_new` の保護は残す．hardware の要らないテストで，既存のファイルと親の無い path が拒否されることを確かめる（関数を分けるなら，入口がそれを probe より前に呼ぶこと）．
- **Sol 4（minor）P734 の名前と変異が合わない．** 名前どおり，足りない環境変数に空でない既定値を入れる変異にし，missing の assert で kill されることを確かめる．
- **O1（minor）`with_measurement_timing` が `from_binding` の導出（tree，`min_command_lead`，envelope の section）を写している．** 二つが呼ぶ private の helper を一つにする（`from_binding` の振る舞いは変えない）．この行に触れる既存の mutation の行は，`old` が tree でちょうど 1 回一致するよう直す．
- **O5（minor）`design/18` UR-34 の文言．** 「replace the Description's candidate timing」を「replace the Description's timing with candidate values」に．
- O4 は直さない（note）．

### 2. 確かめること

- 直したテストと新しいテストは純粋な関数なので 1 回でよい（時刻やスレッドに依るテストを変えたときだけ 20 回）．`cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`rules.py --base HEAD`，`git diff --check`．`hardware.rs` を変えたら docker の `--features uhd` の clippy（`impl-3b1-brief.md` §3 のコマンド）．
- mutation：新しい行と，変えたコードや killing test に触れる行（P731 ほか読み方，帰属，metadata，P734，builder の行）を `remote_mutate.py ... sim02:4,sim03:4 <ID>...` で流す（前に変えたファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致すること．全 list は流さない．
- セルフレビューは回さない（Sol が修正を再レビューする）．

### 3. 報告

`fix-3b1-report.md`．最初の行は `STATUS: done`．指摘ごとに直したこと（`path:line`），テスト，mutation の行と結果，確かめたことの結果，残ったこと．**起きたことを書く．していないことは書かない．**
