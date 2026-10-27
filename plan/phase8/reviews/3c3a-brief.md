# 段階 3c-3a：指示書

## Sol の事前確認の依頼（`sol-pre-3c3-brief.md`）

Repository: /home/komatsu/local_work/Ez-SDRv4 (HEAD 7c0de6a). READ-ONLY: no edits, no git writes, no cargo, no tests, no hardware. Sol is started by the orchestrator only; do not start other agents.

You are checking an implementation brief BEFORE any code is written: `/home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/impl-3c3-brief.md` (Japanese; Phase 8 stage 3c-3, measurement tooling for multi-channel throughput, full duplex, reopen, and H4 load). Lesson from earlier stages: briefs that stated success conditions without checking UHD 4.10 / FPGA / Provider behaviour produced majors or wedged the X300 (e.g. DDC timed settings only leave their FIFO with passing samples, `plan/phase8/39-dsp-timed-commands.md` §1). Read the brief, design note `plan/phase8/41-measurement-records.md`, `plan/phase8/bench-results.md` (2026-10-20 and 2026-10-27 sections), and the code it names (`crates/ezsdr-radio-uhd/tests/common/measurement.rs`, `tests/common/mod.rs`, `tests/hardware.rs`, `src/provider/mod.rs`, `src/uhd.rs`, `crates/ezsdr-radio/src/channels.rs`).

For each procedure in the brief's §2 (2.1–2.6), check it against UHD 4.10 host source (and FPGA where relevant; UHD source may be in the docker image `ezsdr-v4-dev:uhd4.10` — you may not run docker, so use GitHub `EttusResearch/uhd` tag v4.10.0.0 via web if needed) and the repo code, and answer OK or CHANGE (with the concrete change), marking each fact VERIFIED (path:line or URL) or INFERRED. In particular:

1. §2.2/§2.3 multi-channel throughput on X300 (HG one 10 GbE; XG with `second_addr`): how UHD 4.10 assigns RX/TX streamers and channels to links; whether one streamer with two channels vs two streamers changes link use; whether two RX streams at different rates (different DDC decimations) are legal on one X300; whether the Module and Provider path in this repo can actually run conditions (a)–(d) today (two streams, `receive_spec` is 1-channel only, RM-29 ports `rx<k>`, link placements per port); whether the capacity-across-conditions value rule is consistent with note 41 §3 (upper bound p ≤ m) and P3 (1).
2. §2.3: is a `src/` measurement-only builder really needed to admit totals above the profile's `rx_bytes_per_s` (where is RM-32 judged: Kernel validate from the Description, `Description::violations`, the Module's prepare/configure?), or can the existing `with_measurement_timing`/`assembly` adjust path or a test-side profile change do it? Is a candidate of one link's capacity (1 250 MB/s at 10 GbE) meaningful given sc16 wire bytes and the rate grid 200e6/n?
3. §2.4 full duplex on one OBX (RX2 receive, TX/RX transmit) at rates up to 200 MS/s each over one 10 GbE: RM-31's rule, any UHD/X300 limit; leads measured with the other direction streaming — is anything in those procedures a timed DDC/DUC command to a non-streaming chain (note 39 F1–F6)?
4. §2.5 in-process reopen after dropping a clean handle: UHD 4.10 X300 known behaviour on make → destroy → make in one process (static state, claim, links, `rfnoc_graph` teardown); this repo's Phase 7 findings (`plan/phase7/bench-results.md`, UR-29, UR-33). Is reopening worth it, or should "reopened" be a second process run?
5. §2.6 H4: do this Module's `uhd-tx` / `uhd-control` threads, or UHD's own threads, run with realtime priority (the bench container has `--cap-add=SYS_NICE --ulimit rtprio=99`; UHD `set_thread_priority_safe`)? If so, does a nice-0 CPU busy loop load affect them at all, and what load would be meaningful for "wake-up latency under load"? Can the wake-up lateness be computed from `tests/common/probe_device.rs` rows as the brief says (does a timed `apply` row carry its instant `e`; is `release_window` known to the test; does a `tx_send` row carry the device time of its samples)? Propose the simplest observable that answers H4.
6. Anything in §2 that could wedge or damage the device, or that RF safety (`plan/phase8/bench.md` "機材と RF 安全") would forbid, e.g. TX on a second board's channel (CBX in slot B) that may not be cabled.

Also list missing sequences the brief should name. Output (final message, English or Japanese): per item OK/CHANGE with evidence, then a short list of brief changes. Under about 1,200 words.

## 実装（`impl-3c3a-brief.md`）


あなたは実装役（Claude Code の Sonnet サブエージェント）です．HEAD は `7c0de6a`．repository は /home/komatsu/local_work/Ez-SDRv4（main の tree）．作業 dir は `~/.cache/ezsdr-fixes/2026-10-26-stage3/`．main の tree を触るのはあなただけ．別の Opus の agent が読むだけで並行に調べものをしている（tree は書かない）．

## 0. 守ること

- `cargo fmt`・`rustfmt` 禁止（tree は rustfmt の形でない．周りの書き方に手で合わせる）．
- ローカルの `mutate.py` 禁止．mutant を作業ツリーに当てない．
- commit・push・`git add`・`git stash` 禁止．version を上げない．SCHEMA_CHANGELOG に書かない．
- `src/` は §2.3 の測定用の builder 1 つ以外は変えない（ほかに要るなら止めて報告に書く）．本番の値と既定の道は変えない．
- cargo とテストの出力はファイルに出して `tail` か `grep` で見る．長い出力を文脈に読み込まない．負荷をかけない．
- テストの一時ファイルは /tmp のまま．helper の script は `/home/komatsu/.claude/jobs/8aebdd6c/tmp/` に．
- `mutations.json` の新しい行は既存の最後の行と同じ字下げ．tree に作ったファイルは mode 644（script は 755）にする（Write で作ると 600 になり，remote mutation が全部落ちる）．
- Sol（`openai/gpt-6.1-sol`）は起こさない．実機は流さない．
- 並行や失敗の扱いを足すときは，仕組みを足す前に道の数を減らす形を考える（3c-2c の教訓：latch・guard・lease を足し続けて収まらず，lease の既定を「捨てる」に逆にして閉じた）．
- **起きたことを書く．していないことは書かない．**

## 1. 背景

読むもの：設計メモ `plan/phase8/41-measurement-records.md`（特に §2・§3・§5・§8），`plan/phase8/bench.md`，`plan/phase8/bench-results.md` の 2026-10-27，`plan/phase8/reviews/3c2c-opus.md`，`plan/phase8/00-overview.md` の P3 と BP4b・BP4c の行，`design/07-radio-model.md` の RM-28・RM-31・RM-32，`design/18-uhd-radio.md` の UR-34．

3c-3 は設計メモ 41 §8 の最後の段：H4 の負荷，2 channel の throughput と同時運転，BP4b（混在構成），BP4c（XG，10 GbE 2 本，再 open）．大きいので 2 つに分ける：3c-3a（この指示書）は selector，多 channel，同時運転，候補の builder．3c-3b は再 open と H4．どれも試験と bench の道具で，本番の Module と profile の値は変えない（値を決めるのは段階 4）．

使える機材（bench は後日．このコードは実機なしで書いて fake でリハーサルする）：
- X300 #1：slot A に OBX，slot B は空．HG image．
- X300 #2：slot A に UBX-160，slot B に CBX．HG image．UBX＋CBX の混在は RM-28 で受け付けられる（`boards_compatible` は全部の組で true．拒否されるのは 1 つの slot に違う board を置く場合と，装置に無い board を layout が名指す場合．2026-10-20 の「one device carries one kind」は今は無い古い UR-5 の規則）．
- ほかに CBX，LFTX・LFRX・BasicTX・BasicRX が各 2 枚まで借りられる．2 channel の bench は「#2 の UBX＋CBX」か「#1 の slot B に CBX を挿す（OBX＋CBX）」か「Basic/LF を 2 枚」（owner が決める）．
- XG image への書き換えは owner の確認が要る（コードは HG・XG のどちらでも動くようにする）．

今の測定の道（読むところ）：`crates/ezsdr-radio-uhd/tests/hardware.rs` の `hw_measurements_record`，`tests/common/measurement.rs`（`provider_groups`，`measure_rx_throughput`・`measure_tx_throughput`・`measure_rx_start_lead`・`measure_tx_start_lead`・`measure_overflow_restart_gap`，`candidate_session`，`summarize_candidates`，`DeviceLease`，`GroupRun`），`tests/common/mod.rs` の `bench_profile`・`layout_of`（今は slot A と同じ種類の board だけを layout にする）・`receive_spec`（1 channel だけ），`tests/common/probe_device.rs`，`tests/fake.rs` のリハーサル，`plan/phase8/tools/run-record.sh`．

## 2. 作るもの（3c-3a．Sol の事前確認 `sol-pre-3c3.md` を反映済み．再 open と H4 は次の 3c-3b）

### 2.1 layout と selector を環境から渡す

- 新しい環境変数 `EZSDR_MEASUREMENT_SELECTOR`：JSON の object．受け付ける key は `rx`，`tx`（layout の要素の列），`image`，`links`，`link_gbps` だけ．ほかの key，JSON でないもの，object でないものは装置を開く前に拒否する（`process_metadata` と同じ所で）．
- 与えられたら selector に重ねる．今の `bench_profile` は `rx`・`tx` を `layout_of` で上書きする（`tests/common/mod.rs:140-145`）ので，明示の `rx`・`tx` を残すように直す．無ければ今のまま（既定の道は変えない）．layout の検査は Provider の `from_binding`（UR-5，RM-28）に任せ，試験の側で同じ規則を書き直さない．selector と装置の front end が合わなければ，装置を開いた後の最初の Run で UR-5 の拒否になり，その群の失敗として記録される（テストで固める）．
- `EZSDR_UHD_ARGS` は今のまま（XG の 2 本は `addr=…,second_addr=…` で owner が渡す）．
- 全部の測定（Provider の群，raw の群，raw の probe）が同じ selector を使う．各 `measure_*` と probe が自分で `bench_profile` を呼ぶ所をすべて grep して揃える．
- **channel ごとの設定**：今の `board_of`・`bench_hz`・`bench_envelope`（`tests/common/mod.rs:83-135`）は slot A の board の既定の周波数 1 つを全体に使い，UBX・OBX・CBX しか受けない．channel ごとに，その channel の board（`Board::of_front_end`，Basic/LF を含む）の既定の周波数と，その周波数を囲む帯（今と同じ ±1 MHz，利得 0）にする．RF の帯は RM-33 の channel ごとの envelope で渡す．CBX の既定は 1.2 GHz 以上で，今の約 1 GHz の帯の外になる．att は 18 GHz まで効く（owner）ので，帯を channel ごとにすれば足りる．
- `run-record.sh`：
  - `EZSDR_MEASUREMENT_SELECTOR` をそのまま container に渡す．
  - att の確認の文に，送信する channel をすべて名前で出す（selector の `tx` の各要素．無ければ「slot A の board の TX/RX」）．「送信する**全部の board**の TX/RX → att → 同じ board の RX2」を 1 回の `yes` で確かめる．確認を省く option は作らない．
  - 測定に使う NIC の名前を `EZSDR_MEASUREMENT_NICS`（空白区切り，既定は `EZSDR_BENCH_NIC`）で container に渡す（§2.2 の NIC ごとの byte 数）．
  - `--help` に書く．

### 2.2 多 channel の受信・送信の throughput（BP4b・BP4c の値の行）

`measure_rx_throughput`・`measure_tx_throughput` を，layout のその向きの channel 数に合わせて条件を作るようにする．channel が 1 つなら今のまま（今の条件・observation・値の行の形を変えない）．

2 channel 以上の向きの条件：
- (a) 1 本の stream に全 channel（同じ rate．UHD の 1 つの streamer は channel の rate を揃える）．
- (b) channel ごとに 1 本の stream（同じ rate）．
- (c) channel ごとに 1 本の stream で rate が違う（BP4b．DDC/DUC の rate は port ごとで合法）：channel 0 は rate の格子の高い順，channel 1 はその次に低い格子の rate．
- 各条件で rate は合計の bytes/s の大きい順に試す．1 試行は今の手順（D 秒，記録された区間 × rate，overflow・underflow なし）を **stream ごと**に満たすこと：stream ごとの Spec，受信は宣言した全部の `rx<k>` に sink と placement，送信は stream ごとの action と波形，完了の証拠も stream ごと．合計の数で 1 つの channel の飢えを隠さない．Run が admission で拒否されたら `AdmissionRejection`（今と同じ）．
- 条件を変えるとき（例：(a) から (b)）は，前の streamer を閉じてから次の streamer を開く（持ったまま同じ front end を 2 本目で開くと UHD は edge を壊して abort する．`bench-results.md` 1b-4e-2）．今の Run の終わりで閉じていることを確かめ，閉じていない道を作らない．
- link：UHD 4.10 は link を channel の接続ごとに，向きごとの割り当ての数で選ぶ（byte 数ではない，数は減らない．`graph_stream_manager.cpp:188-219,252-284`）．stream の番号を物理の link と同一視しない．各試行の前後で `EZSDR_MEASUREMENT_NICS` の各 NIC の `/sys/class/net/<nic>/statistics/{rx,tx}_bytes` を読み，差を observation に．streamer を開いた順も observation に．
- `conditions` の行は §2 の形（stream ごとの channel と rate，`concurrent`）．

値（P3 (1) の「容量 ＝ 全試行で持ちこたえた最大」を条件の集合に広げる）：
- 合計の bytes/s を T とする．m は「適用できる全部の条件で，要求した T 以下のすべての段の全試行が成功した」ような，成功した段の T の最大．
- 適用できる条件のどれかが欠けている（流れなかった，T 以下で `AdmissionRejection` が出た，試行が足りない）なら `unmeasured`（理由）．`AdmissionRejection` は装置の失敗としては数えず observation に残す．最初に失敗した T を容量にしない．
- 条件ごとに格子で表せる合計が違う（1 channel は 800 MB/s まで，(c) は 1 200 MB/s を表せる）．T ごとに「その T を表せる条件」を明示し，表せない条件は T の判定に入れない．この扱いを observation に書く．
- 値の行の `condition` は m を決めた条件．observation に条件ごとの「全試行が成功した段」「失敗した段」を残す．
- 1 条件なら今の `capacity_value` と同じ m になることを純粋なテストで固める．

### 2.3 候補の throughput（profile の上限を越えて試す）

RM-32 は Description の coercion と `violations`（prepare を含む）で判定され（`channels.rs:599-605,721-724,817-848`，`provider/mod.rs:470-472`），selector や profile の編集では performance を変えられない．3b の `with_measurement_timing`（`src/provider/mod.rs:367-373`）と同じ形で，測定のときだけ Description の performance を差し替えて面を作り直す builder を足す（`UhdRadio::with_measurement_performance` など）．既定の構築と本番の道は変えない．
- 候補の値は「admission を通すための天井」で，link の実効容量の測定ではない（Ethernet と protocol の overhead がある）．`link_gbps` の 1 本分（10 GbE なら 1 250 MB/s）を天井にする．格子で表せる合計で意味があるのは 2 channel の 200＋100 MS/s ＝ 1 200 MB/s（200＋200 は 1 600 で天井の外）．
- RM-32 の「方向ごとの合計は link 1 本分以下」（設計 33 D9）は変えない．XG・2 本でもそれより上は試さない（8c の範囲）．
- 単体テスト：差し替えた値で admission が通り，差し替えない既定の構築は今の値のまま．

### 2.4 受信と送信の同時運転（full duplex）

- 条件 (d)：受信の stream と送信の stream を同時に，同じ rate で（送信は repeat の連続）．受信と送信の throughput の値の計算に，それぞれの条件として入れる（`concurrent` に相手の stream）．RM-32 は向きごとの予算で，200 MS/s は各向き 800 MB/s．1 channel の OBX でも測れる（RM-31：同じ board の送信が stream にあれば受信は RX2）．
- 相手の向きが試行の間ずっと流れていたことを証拠で示す（相手の stream の区間と数，underflow・overflow なし）．示せなければその試行は失敗．200/200 は持ちこたえる保証のない試行（今の bench で送信は 100・200 MS/s で underflow した）．
- 送信は今の送信の throughput と同じ RF の条件（利得 0，channel ごとの帯，振幅 0.5 以下）．timed の周波数の変更は使わない（設計メモ 39 の F1〜F5 を起こさない．利得は radio だけに届く）．
- leads：`measure_rx_start_lead` と `measure_tx_start_lead` に，相手の向きを 10 MS/s で流し続けたままの条件を足す（値は今の「最小の lead」の読み方で，条件の集合の最悪＝大きい方）．送信の start lead が「構成の完了」を見ていて on time の burst を見ていない今の限界（`measurement.rs:2489-2522`）はそのまま書き，強い保証に言い換えない．
- overflow の restart gap にも送信を流したままの条件を 1 つ（時間の型で最大）．

### 2.5 失敗の扱い

- 制御の道の timeout（`Control operation timed out`，`apply` の error）が出たら，3c-2c と同じくその handle に何も送らず群を止める（reset・coercion を続けない）．
- 次の系列をテストで固める：相手の向きの失敗，部分の送信（受け付けた数が足りない），burst の ACK が来ない，1 channel だけの飢え，selector と layout の不一致（UR-5 の拒否），admission の拒否が T 以下にある条件．

### 2.6 リハーサルとテスト

- `tests/fake.rs`：2 channel の fake（OBX＋CBX の front end．`FakeConfig` の `front_ends`）で，selector を関数に直接渡し，§2.2・§2.4 の群を少ない回数で流す．記録が §2 の形で書け，比較テストの規則 3・5 を満たすこと．fake の値の大きさや分類を壁の時計で判定しない（`tests/README.md`）．
- 純粋なテスト：selector の解釈（受け付ける key，拒否），条件の集合からの容量の値（1 条件で今と同じ，表せない条件を除く，欠けた条件で `unmeasured`，T 以下の失敗），lead・時間の値の条件の集合での最悪，channel ごとの周波数と帯．
- `bash -n` を script に．

## 3. 範囲

上だけ．再 open と H4（3c-3b），H3，D9 の本番の直し，profile の値，Provider の本番の道は変えない．新しい rig の種類，fake の fault の種類は足さない．試験の中の `Device` の実装は可．大きさの目安は約 1,500 行以内．越えそうなら止めて，どこで分けるかを報告する．

## 4. 確かめること

- 純粋なテストは 1 回．fake のリハーサルを変えたら 20 回（時刻とスレッドに依るため）．
- `cargo test --workspace`，`cargo clippy --workspace --all-targets -- -D warnings`，`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD`，`git diff --check`，`bash -n` を script に．
- `hardware.rs` は docker の clippy：`docker run --rm --user $(id -u):$(id -g) -v $PWD:/work -v ezsdr-v4-cargo-target:/cargo-target -w /work ezsdr-v4-dev:uhd4.10 cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`．
- mutation：判定と読み方（容量の値，最悪の値，selector の拒否，channel ごとの帯，§2.3 の builder）に行を足し（ID は既存の最大の次から），新しい行と，変えたコードや killing test に触れる既存の行を `REMOTE_SSH="ssh -o BatchMode=yes" PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ID> ...` で流す（repo の root から．前に変えたファイルと新しいファイルを mode 644）．全 list の `old` が tree でちょうど 1 回一致．全 list は流さない．「BASELINE FAILED」はその行だけ流し直す（remote の負荷）．
- spec：`design/18-uhd-radio.md` UR-34 の文とテストの名前（DA-1 の形），§2.3 の builder を書く所（UR の該当の規則），Changes に 1 行（設計メモ 41，stage 3c-3a）．`plan/phase8/bench.md` の BP4b・BP4c の手順（selector の渡し方，NIC の名前，att の確認が全 TX board）．

## 5. セルフレビュー（1 回）

```
timeout 2400 opencode2 run --standalone -m "opencode/deepseek-v4.1-flash-free#max" --agent plan --auto "Read /home/komatsu/.cache/ezsdr-fixes/2026-10-26-stage3/checklist-3c3a.md and do what it says. Your final message is the review." > ~/.cache/ezsdr-fixes/2026-10-26-stage3/review-3c3a-1.log 2>&1
```

前景で流す（Bash の timeout を 2,400 s 以上に）．free が失敗したら（空の log，エラー，上限の表示）同じ回を `-m "opencode-go/deepseek-v4.1-flash#max"` で．1 回で止める．指摘はすべて「直した」か「退けた（理由）」を報告に．

## 6. 報告

`report-3c3a.md`．最初の行は `STATUS: done`（途中で止めたら `STATUS: stopped` と理由）．作ったもの（ファイルごと），この指示書から変えた所と理由，テスト名，bench の時間の見積もり（1 channel・2 channel，HG・XG），spec の変更，mutation の行と結果，確かめたことの結果，セルフレビューの指摘と扱い，残ったこと．最後のメッセージは報告の要約．

## セルフレビューの checklist（`checklist-3c3a.md`）

あなたは読むだけの reviewer です．リポジトリ /home/komatsu/local_work/Ez-SDRv4 の未 commit の差分（`git diff HEAD` と untracked の新しいファイル，HEAD `7c0de6a`）を見る．ファイルを書き換えない．`cargo fmt`，mutation の道具，`git stash`，commit を使わない．テストは `CARGO_TARGET_DIR=~/.cache/cargo-target/Ez-SDRv4-review CARGO_INCREMENTAL=0 timeout 300 cargo test ...` だけ．負荷をかけない．mutation は流さない．

読むもの：作業 dir `~/.cache/ezsdr-fixes/2026-10-26-stage3/` の `impl-3c3a-brief.md`（§2 が仕様）と `sol-pre-3c3.md` の最後の節（UHD の事実）．

見ること（4つ）：
1. **測定とテスト**：§2.2 の条件 (a)〜(d) が stream ごとの Spec・sink・placement・送信の action を持ち，完了の証拠を stream ごとに見るか（合計で 1 channel の飢えを隠さないか）．条件を変えるとき前の streamer を閉じてから開くか．容量の値が「適用できる全条件で T 以下の全段が成功」の規則どおりか，欠けた条件で `unmeasured` になるか，最初に失敗した T を容量にしていないか．同時運転で相手の向きが試行の間ずっと流れた証拠を見るか．§2.3 の builder が既定の構築を変えないか．channel ごとの周波数と帯．fake のリハーサルが時刻に依る assert（分類の結果，値の大きさ）をしていないか．各 mutation の行はどのテストのどの assert で kill されるか（読んで判断）．
2. **範囲**：指示書 §3 に対して，足りない所と余計な所．§2.3 の builder 以外の `src/` の変更，再 open・H4（3c-3b）・H3 の実装，profile の値の変更，新しい rig の種類・fault の種類，version の上げ，SCHEMA_CHANGELOG への追記は指摘．
3. **「すべて」の主張**：全部の測定（Provider の群，raw の群，raw の probe）が同じ selector を使うか（`bench_profile` と `layout_of` の呼び出しを全部 grep）．`run-record.sh` の att の確認が送信する全 board を名前で出すか，省く option が無いか．`source: "fake"` の記録で `Measured` が無いか．各 mutation の行の `old` が tree で 1 回だけ一致するか．
4. **文書**：`design/18` に足した文が DA-1 の形か，挙げたテストが存在するか．`bench.md` の手順が script と合うか．

orchestrator の gate（全 mutation，docker，Python）は指摘にしない．出力は日本語．指摘ごとに P1（誤り・テストが性質を見ていない）／P2（弱い・漏れ）／P3（文・形），`path:line`，根拠，直し方．最初の行に `P1 の数：n`．
