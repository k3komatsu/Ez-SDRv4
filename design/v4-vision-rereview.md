# Ez-SDR v4 Vision — Re-review of the Revised Documents

> **Status:** Second-pass architecture review（2026-09-21 の三段階改訂後の Vision / CMA を対象）
> **Reviewed:** `Ez-SDR_v4_ARCHITECTURE_VISION.md`（68 節、2983 行、Revision 注記 3 本）、`Ez-SDR_v4_core_module_architecture.md`（41 節、1527 行）
> **Baseline:** `design/v4-vision-audit.md`（Phase 0 監査、Findings 1–34）。本文書の Finding は **R1–R22** と番号を付け、監査の F 番号と区別する。
> **Question asked:** 監査の全項目を反映した改訂版は、(a) 内部矛盾なく、(b) 新たな穴を持ち込まず、(c) Phase 1「Kernel semantic model」の設計に入れる状態か。
> **Method:** 旧記述の残存を grep で機械検査 → 改訂で追加・変更した全節を通読 → 監査 Finding ごとの反映状況を照合 → 改訂が新たに導入した矛盾を洗い出す。外部一次資料の再検証は行っていない（改訂は監査で検証済みの事実に基づく）。  
> **Status update (2026-09-21):** R1, R2, R21 を Vision / CMA へ反映済み（Vision 冒頭の fourth pass 注記）。R3, R6, R17, R22 も反映済み（fifth pass 注記）。未反映は P2/P3（R4, R5, R7–R16, R18）のみ。  
> **Status update (2026-09-21, later):** R4, R5, R7–R12, R15, R16, R18 も反映済み（Vision sixth pass）。R13・R14 は Vision の 11 ファイル分割と索引への改訂履歴移動で対応（規範部分の spec 化は Phase 1）。本文書の全 Finding は反映済みで、未反映項目はない。§番号は不変、索引はルートの `Ez-SDR_v4_ARCHITECTURE_VISION.md`。  
> **CMA retirement note (2026-09-21):** CMA は退役し `design/archive/Ez-SDR_v4_core_module_architecture.md` に保管。本文書の CMA 参照は保管版を指す（R20 の整合確認は退役時点の版に対するもの）。

---

## 1. Executive Summary

**判定：READY — ただし Phase 1 の型を書き始める前に、改訂が持ち込んだ 3 件の契約形状の修正（R1, R2, R21）を先に入れること。**

- 監査の 34 Finding はすべて両文書に反映されており、旧記述の残存（`rx_channels`、`SimulationFidelity`、`VirtualClock`、「Placement is selected」、「RFNoC Executor / Island」）はゼロである。章番号は不変で、監査からの § 参照はすべて有効。
- しかし 3 回の改訂は**それ自身が矛盾を持ち込んだ**。最も重いのは次の 3 点で、いずれも数行の修正だが、放置すると Phase 1 で書く Spec / BindingProfile / SampleBlock の形に焼き込まれる。
  1. **R1**：§8 の資源要求を `channels: 2` に書き換えた結果、RX/TX 非対称（4 RX + 1 TX、RX-only sensing）を表現できなくなった。元の `rx_channels / tx_channels` の方が正しかった。
  2. **R21**：§10 / §20 で placement を「BindingProfile **と Spec の `graph.placement`**」に置いた。Spec に placement を書けば、その Spec は GPU のない host へ昇格できない。invariant 6・29（intent / binding 分離、single-host 前提の禁止）と直接矛盾する。placement は BindingProfile のみ、Spec は要件だけ。
  3. **R2**：Stream Contract は RX 側を規範化したが、TX 側は「block が target TimePoint を持つ」以外を定めていない。burst 境界（SOB/EOB）、連続 TX 中の時刻不連続の意味、`repeat` の表現が未定義。UHD は SOB ごとに time spec を要求し EOB で burst を閉じるので、この規定なしに Mock と実機の TX 意味論は一致しない。
- P1 級は 4 件（R3 Session Action の admission、R6 sample-rate 変更時の SampleClock、R17 Mock の PerformanceEnvelope 強制、R22 「Kernel」と「simulation kernel」の名称衝突）。残りは順序・表現・用語の整理（P2/P3）。
- Core 境界は改訂後も維持されている：§5 の Kernel 一覧は監査 §13 の最小 Kernel と一致し、Vocabulary への追加は Radio Model の `TimingEnvelope` と `phase_behavior_on_retune` のみ。改訂による **Kernel 概念の増加はない**。

---

## 2. What was verified

| 検査 | 結果 |
|---|---|
| 旧記述の残存（`rx_channels`, `SimulationFidelity`, `VirtualClock`, `Placement is selected`, `RFNoC Executor`, `RFNoC Island`, `high-level planning`） | Vision 0 件、CMA 0 件（CMA §1 の動機列挙に「RFNoC / FPGA placement」1 件のみ残存 → R11） |
| 章番号 | Vision 68 節 / CMA 41 節、改訂前と同一。監査の § 参照は全て有効 |
| コードフェンスの均衡 | Vision 252 / CMA 142、いずれも偶数 |
| 監査 §12.1–§12.4 の反映 | 全項目反映（§6 のカバレッジ表） |
| 監査 §13「Recommended Minimal Core」と Vision §5 Kernel 一覧 | 一致。Kernel 概念の追加なし |
| 監査 §11 不変条件 1–10 と Vision §65 | 31–42 として全件収録 |
| 監査 §14.2 受け入れテスト追加 #11–#14 | §58 に収録 |
| 両文書間の整合 | CMA §8 の Mock selector が `model:` と `profile:` で混在（R11）、CMA §1 の表現 1 件（R11）以外は一致 |
| 用語 | Action 名の不統一（R9）、`DeviceTime` 残存 1 件（R9）、「Kernel」二義（R22） |

---

## 3. Findings

### R1 — 資源要求 `channels: N` は RX/TX 非対称を表現できない（改訂による退行）（2026-09-21 反映済み）

- **Severity:** `P0`（Spec スキーマの形。Phase 1 で焼き込まれる）
- **Area:** ExperimentSpec / Radio Model
- **Current text:** Vision §8 JSON `"channels": 2, "full_duplex": true, "coherent": true`、§8 Rules「`channels: 4, coherent: true`」、CMA §7 JSON 同文。改訂前は `rx_channels: 2, tx_channels: 2`。
- **Problem:** ISAC の受信アレイ（4 RX + 1 TX）、RX-only channel sounding、TX-only の repeat 送信機が要求できない。coherence も方向別に異なりうる（RX 4 ch coherent、TX 1 ch）。
- **Failure scenario:** Stress Test E（4 coherent RX + MIMO 検出器）を Spec に書けない。`full_duplex` と `channels` の組合せから RX/TX 数を推測する暗黙規則が生まれ、Provider ごとに解釈が割れる。
- **Recommended fix:** 方向別に要求する。
  ```text
  "radio": { "requires": {
      "rx": { "channels": 4, "coherent": true },
      "tx": { "channels": 1 },
      "full_duplex": true, "sample_rate_hz": 20e6, "hardware_time": true } }
  ```
  §8 の Rules 文と CMA §7、CMA §34 の例（「2 RX 2 TX」）を合わせる。
- **Confidence:** High

### R2 — Stream Contract の TX 側が未完成（burst 境界・時刻不連続・repeat）（2026-09-21 反映済み）

- **Severity:** `P0`（SampleBlock flags の形）
- **Area:** Stream Contract / Radio
- **Current text:** §23 rule 7「TX blocks carry the target TimePoint of their first sample, also in continuous mode」、§22 LatePolicy、§20/§35 の `tx.repeat` capability。
- **Problem:** 次が未定義。(1) 連続 TX で block 間に時刻の跳びがあったとき、それは新しい burst の開始か、エラーか。(2) burst の終端をどう示すか（UHD は EOB を要求し、次の SOB には time spec が必要 — CORDIC が SOB ごとにリセットされる）。(3) `repeat` は「repeat 属性を持つ 1 burst」か「block を送り続ける連続 TX」か。(4) TX 側の underflow / late は event のみで、Artifact 側（送信した波形の記録）に痕跡がない。
- **Failure scenario:** Reactor が PONG を 2 発連続で出す。Mock は隙間の空いた 2 block を「連続 TX の gap」として黙って送る（実際には何も送らない時間が空く）。X310 では 1 発目が EOB なしのまま 2 発目の time spec 付き packet が届き、UHD が `TIME_ERROR` または burst 混在を起こす。同じ Reactor が Mock と実機で異なる結果になる。
- **Recommended fix:** §23 に「TX side」節を追加する。
  - TX stream は **burst の列**。burst は時刻が連続する block の並び。`START_OF_BURST` / `END_OF_BURST` flag を SampleBlock に持たせ、時刻の跳びは明示的な EOB なしでは **error（`TX_DISCONTINUITY` event + burst を LatePolicy で処理）**とする。
  - `repeat` は burst の属性（`repeat: true`）。Provider は host loop か Replay で実装し、Replay の alignment / DRAM 制約は capability に出す（§20, §35）。
  - TX 側の Manifest 記録：burst ごとの target time、実送信時刻（Provider が分かる範囲で）、underflow / late event の対応。
  - Mock は同じ規則で SOB/EOB を検査する。
- **Confidence:** High

### R21 — placement を Spec にも置いたことで intent / binding 分離と矛盾する（2026-09-21 反映済み）

- **Severity:** `P0`（Spec / BindingProfile スキーマの境界）
- **Area:** ExperimentSpec / BindingProfile / Planner
- **Current text:** §10「Placement ... stated explicitly in the BindingProfile (and, for processing components, in the Spec's `graph.placement`)」、§20「the BindingProfile (and the Spec's `graph.placement` for processing components) states which Executor and which MemoryDomain」。
- **Problem:** Spec に `placement: gpu0` を書けば、その Spec は GPU のない環境に昇格できず、invariant 6（intent と binding の分離）・8（同じ Spec が Simulation と Hardware で動く）・29（single-host 前提を public semantics に焼かない）に反する。監査 F13 の意図は「Core が最適化しない」であり、「Spec に placement を書く」ではなかった。
- **Failure scenario:** OTFS 実験を GPU host で Spec に `placement: gpu0` と書いて検証し、CPU-only の HIL 環境へ昇格しようとすると Spec の書き換えが必要になる。「BindingProfile だけ変える」約束が破れる。
- **Recommended fix:** placement は **BindingProfile のみ**：`placements: { <component_id>: { executor: ..., memory_domain: ... } }`。Spec は要件のみ（`requires: { executor_kind: gpu | any, budget: ... }`）。§10, §20, §5（「placement as bound」は Manifest の話なので維持可）、§50 を修正。
- **Confidence:** High

### R3 — Session の runtime Action が admission（envelope / RF envelope / update class）を通る規定がない（2026-09-21 反映済み）

- **Severity:** `P1`
- **Area:** Session / Safety / Lifecycle
- **Current text:** §3「Every Easy API call is a typed Action appended to the action log」「Runtime changes ... only through declared parameter update classes」。§52「`validate()` and `prepare()` enforce [the RF envelope]」。
- **Problem:** `validate()` / `prepare()` は Session 開始時に一度走る。その後の `sdr.rx.frequency = 1.575e9` は Action として log に残るが、**envelope・RF envelope・coercion policy を通る**とはどこにも書かれていない。invariant 42「Nothing transmits before validate() passes」が Session の途中変更に対して空洞化する。また invariant 10（RT path は JSON を解釈しない）に対し、Action がどこで parse されるかも未記述。
- **Failure scenario:** AI agent が Session 内で周波数を sweep し、許可帯域外に出る。Manifest には正しく記録されるが、送信は起きてしまう。
- **Recommended fix:** §3 Rules に追加：「Every Session Action is parsed and admitted on the control path — TimingEnvelope, RF envelope, coercion policy and update class — before it enters the real-time path as a typed Action. Rejected Actions are logged as rejected.」
- **Confidence:** High

### R4 — §10 の pipeline で Capability matching が Binding resolution の前にある（2026-09-21 反映済み）

- **Severity:** `P2`
- **Area:** Planner
- **Current text:** §10：Resource resolution → **Capability matching → Binding resolution (explicit)** → Plan construction → Admission。
- **Problem:** binding が明示なら、matcher は**束縛先 instance の宣言 capability**と Provider の coerce 回答に対して照合する（§8「asks the Provider whether a value can be coerced」）。instance が決まる前に照合はできない。順序が逆。
- **Recommended fix:** Resource resolution → Binding resolution → Capability matching (against the bound instances) → Plan construction → Admission checks。
- **Confidence:** High

### R5 — RealtimeEmulation の非決定性が明示されていない（2026-09-21 反映済み）

- **Severity:** `P2`
- **Area:** ExecutionClass / Determinism
- **Current text:** §15「Simulation run: DES kernel ... RealtimeEmulation run: the same kernel, paced to wall clock」、§32「RealtimeEmulation: Islands on real threads」、§58 #3「Deterministic runs reproduce with a seed」、§65 #25。
- **Problem:** 実スレッドの Island と wall clock pacing の組合せは本質的に非決定。「seed で再現」は **Simulation class のみ**の保証であることを明記しないと、RealtimeEmulation の flaky を bug と誤認する。また RealtimeEmulation の hybrid（環境モデルは DES が wall pace で進め、Island は thread で消費、deadline miss は実時間で測る）が 1 文で説明されていない。
- **Recommended fix:** §14 か §15 に「Determinism is a property of the Simulation class only. RealtimeEmulation trades it for real deadlines: the engine paces the environment models to wall clock while Islands run on threads.」§58 #3 を「(Simulation class)」で限定。
- **Confidence:** High

### R6 — sample-rate 変更時の SampleClock の同一性が未定義（2026-09-21 反映済み）

- **Severity:** `P1`
- **Area:** Time / Stream Contract
- **Current text:** §15「Each stream has a SampleClock: a ClockDomain derived from its device clock by an exact rational」、§27 `cold` 更新。
- **Problem:** `cold` で sample rate を変えると tick rate が変わる。同じ `ClockDomainId` のまま rate が変われば、変更前後の `TimePoint.ticks` は比較不能になり、Stream Contract の「時刻は単調」が壊れる。
- **Recommended fix:** 「A sample-rate change is a `cold` update that **ends the stream's SampleClock and starts a new ClockDomain** (new id, new epoch reference). Blocks carry the domain id; consumers detect the change from it; the Manifest records the sequence of SampleClocks.」
- **Confidence:** High

### R7 — Mock profile（`x310-like`）の値を実機で検証する手順が Vision にない（2026-09-21 反映済み）

- **Severity:** `P2`
- **Area:** Simulation / Testing
- **Current text:** §13「MockRadio enforces the envelope of the profile it emulates」、§59 Mock→X310 parity test。
- **Problem:** profile の envelope 値（lead time、coercion grid、restart gap）が実機より緩ければ false confidence が戻ってくる。誰が値を測り、どう版管理し、parity test が何を比較するかが未記述。また「制約なしの ideal profile」を算法開発用に許すか、許すなら fidelity vector で `timing: none` と記録されて昇格判定から除外されるか、が明示されていない。
- **Recommended fix:** §59 に「The parity test measures the hardware envelope (lead, coercion, stop tail, restart gap) and fails if the Mock profile is more permissive; profiles carry a version recorded in the `mock.*` Manifest section.」§13 に「An `ideal` profile may exist for algorithm work; it is recorded as `timing: none` and is not evidence for promotion.」
- **Confidence:** Medium

### R8 — DES engine（TimeAuthority 実装）が §7 の役割軸のどれにも属さない（2026-09-21 反映済み：Authority 役割を追加）

- **Severity:** `P3`
- **Area:** Terminology / Module API
- **Current text:** §7 roles = Provider / Executor / Sink / Link。§60 は `sim-kernel` を `modules/simulation/` に置く。
- **Problem:** TimeAuthority の実装は Resource Model の実装（Provider）でも処理エンジンでもない。実機側では Radio Provider が timekeeper として同じ役割を担う。
- **Recommended fix:** 役割に **Authority**（TimeAuthority の実装）を追加し、「UHD Module は Provider + Authority」「sim-engine は Authority（+ Simulation Provider の一部）」と書く。
- **Confidence:** High

### R9 — Action 名と時刻語の不統一（2026-09-21 反映済み）

- **Severity:** `P3`
- **Area:** Terminology
- **Current text:** §19 `EmitEvent / StopRun / AbortRun`、§5 `Emit, Stop, Abort`、§3 `Stop / Release`；§22 tree `target DeviceTime` と `deadline`、§19 `AbsoluteDeadline`、§23 `target TimePoint`。
- **Recommended fix:** Action 名を §5 に合わせて `Emit / Stop / Abort` に統一（Session の `Release` は Lease 操作として別置き）。§22 tree を `target TimePoint` / `late_policy` に、`deadline` を `AbsoluteDeadline` に。
- **Confidence:** High

### R10 — §9 の Spec 概念形に `version` がなく、`graph` / `policies` / `schedule` の中身が改訂内容と結びついていない（2026-09-21 反映済み）

- **Severity:** `P3`
- **Area:** ExperimentSpec
- **Recommended fix:** tree を `version / requirements / resources / graph (components + links; requirements only, no placement) / schedule (Actions with AbsoluteDeadline) / outputs / policies (closed Policy table + coercion policy) / extensions` に更新。
- **Confidence:** High

### R11 — 残存表現の整理（2026-09-21 反映済み）

- **Severity:** `P3`
- **Area:** Editorial
- **Items:**
  - §31「Execution planning must consider ... transfer planning」→「Admission must validate ... transfer reachability and cost」。
  - §32「The Runtime **may** perform admission checks before RUN」→ §10 が admission を必須にしたので **must**。
  - §36 の generic outputs 一覧に `TimingEnvelope` を追加。
  - CMA §1「RFNoC / FPGA placement」→「RFNoC-backed radio capabilities」。
  - CMA §8「Software simulation binding」の `model: x310-like` を Vision と同じ `profile: x310-like` に。
  - §65 #20「RuntimeEvent/Metric data」は counter を指すなら「RuntimeEvent / counter data」の方が §29「no metrics framework」と読み違えにくい。
- **Confidence:** High

### R12 — schema-first を掲げつつ Rust 構文（`Option<u64>`, `i64`）で形を書いている（2026-09-21 反映済み：言語中立表記 + 索引に「shapes are illustrative」注記）

- **Severity:** `P3`
- **Area:** Editorial / Consistency with §10
- **Current text:** §23 `GAP_BEFORE { lost: Option<u64> }`、§28 `Gap { start_time, lost: Option<u64>, cause }`、§15 `ticks: i64`。
- **Recommended fix:** 言語中立に（`lost: optional count`、`ticks: signed 64-bit`）。あるいは status block に「概念形は例示であり、規範 schema は `design/` の仕様書に置く」と明記（R13 と併せて）。
- **Confidence:** High

### R13 — Vision が規範文書を抱え込み始めている（+21%）（2026-09-21 対応：Vision を 11 ファイルに分割。規範部分の spec 化は Phase 1）

- **Severity:** `P3`（Phase 1 開始時に対処）
- **Area:** Document structure
- **Problem:** Vision は「Non-purpose: not the final Rust API」と言いつつ、Stream Contract、Time model、Session、PrepareReport、Manifest tree という**規範**を含むようになった。実装が進むと Vision と実装の乖離が起きやすい。
- **Recommended fix:** Phase 1 の開始時に、規範部分を `design/stream-contract.md`、`design/time-model.md`、`design/session-model.md`、`design/binding-profile.md` に切り出し、Vision は「なぜ + 不変条件 + リンク」に戻す。監査の § 参照は Vision 側に残るので、切り出し後も節は残して要約 + リンクにする。
- **Confidence:** High

### R14 — Revision 注記が header に 3 本並んでいる（2026-09-21 反映済み：索引ファイルの Revision history へ移動）

- **Severity:** `P3`
- **Recommended fix:** 実装開始時に末尾の「Revision history」節か `CHANGELOG.md` に移す。
- **Confidence:** High

### R15 — Python の wall-clock `sleep` に対する実務的な代替が示されていない（2026-09-21 反映済み）

- **Severity:** `P3`
- **Area:** Python frontend
- **Current text:** §15「Python has `run.wait_until(t)` and `run.wait_for(event)`; it does not have a device-time `sleep`」。
- **Problem:** 実機 Run では `time.sleep` は無害に動くので、利用者はそれを書く。同じスクリプトが Simulation で壊れる。規則だけでは防げない。
- **Recommended fix:** Python client に `sdr.sleep(d)`（= `wait_until(now + d)`、実機では wall clock と一致）を用意し、`time.sleep` は「wall-clock only、Simulation では意味を持たない」と文書化。§54 に一文。
- **Confidence:** Medium

### R16 — coercion policy の既定 `warn` は publication path に対して緩い可能性（2026-09-21 反映済み：Session は warn、Spec Run は rate/frequency を reject・gain は warn）

- **Severity:** `P3`（設計判断の確認）
- **Area:** Lifecycle
- **Current text:** §11「`accept`, `warn` (default) or `reject`」。
- **Consideration:** Session（対話）では `warn` が自然だが、明示 Spec の Run（publication path）では sample rate / frequency の coercion を `reject` 既定にした方が「19.5 Msps のつもりで 20 Msps」を防げる。Spec 種別で既定を分けるかを決める。
- **Confidence:** Medium

### R17 — Mock が PerformanceEnvelope を強制する義務が書かれていない（2026-09-21 反映済み）

- **Severity:** `P1`
- **Area:** Simulation / Admission
- **Current text:** §13 は TimingEnvelope の強制を義務化。PerformanceEnvelope は「next to its PerformanceEnvelope」と並置されるだけ。
- **Problem:** 1GbE 相当の profile に 4×200 Msps を要求しても Mock は受理し、実機で admission に落ちる。F4 の false confidence がスループット次元で残る。
- **Recommended fix:** §13 rule 1 に「... and rejects requests that exceed the profile's PerformanceEnvelope at `validate()` / `prepare()`」を追加。
- **Confidence:** High

### R18 — child Run と Session Lease の関係が未記述（2026-09-21 反映済み）

- **Severity:** `P3`
- **Area:** Session / Lease
- **Current text:** §3「`sdr.run(spec)` inside a Session creates a child Run」、§53 Lease modes。
- **Recommended fix:** 「A child Run inherits the Session's Lease; ending the Session ends its children. A Detached Session keeps its children until the TTL expires.」
- **Confidence:** High

### R19 — （R3 に統合）

### R20 — CMA との整合（確認のみ）

- CMA §4 / §19 / §21 / §24 / §27 / §36 / §38 は Vision と一致。R1・R21 を修正する際は CMA §7 と §27 の対応箇所も同時に直す。

### R22 — 「Kernel」が Core の凍結層と DES engine の両方を指している（2026-09-21 反映済み：DES 側を Simulation Engine に改名）

- **Severity:** `P1`（用語だが、Kernel 概念の境界を語る文書で二義は致命的）
- **Area:** Terminology
- **Current text:** §5「Kernel (frozen)」= Core の層；§13 tree `SimulationKernel`、§15「discrete-event simulation kernel」、§32「the discrete-event kernel step-drives」、§57「Simulation kernel」、§60 `sim-kernel`、§67。CMA も同様。
- **Problem:** 「Kernel does not own per-sample execution」（§5）と「the kernel step-drives every Island」（§32）が同じ文書に並ぶ。読者（特に AI agent）は誤読する。
- **Recommended fix:** DES 側を **Simulation Engine**（`SimulationEngine`, `sim-engine`, "discrete-event engine"）に改名。「Kernel」は Core の層にのみ使う。該当箇所は Vision 6 件・CMA 4 件。
- **Confidence:** High

---

## 4. Residual risks that text cannot resolve（Phase 1–2 で実装が答える項目）

| 項目 | なぜ文書では決められないか | 見極めるマイルストーン |
|---|---|---|
| 参照カウント SampleBlock の 200 Msps でのコスト | Arc 相当の atomic と pool 割当の実測が必要 | Phase 2–3（Mock で block 長・rate を振った benchmark） |
| RealtimeEmulation の hybrid（DES engine + 実スレッド Island）の実装可能性 | pacing と thread wake-up の相互作用は設計図では見えない | Phase 5（PING→PONG を RealtimeEmulation で） |
| UHD Provider が複数 streamer を内部で整列させて「1 stream N channels」を出せるか（dual 10GbE） | UHD の streamer と transport の制約は実機依存（INFERRED） | Phase 7–8 |
| Replay による `tx.repeat` の alignment / DRAM 制約が Radio capability として十分に表現できるか | 実機の word size と item size の組合せ | Phase 8 parity test |
| WASM Executor の copy 回数（linear memory への転送） | ABI 設計と benchmark | Phase 11 |
| `step(until)` を native executor に強制することの性能ペナルティ（実機モードでは使わないが契約に残る） | 実装で無視できるかは設計次第 | Phase 10 |

---

## 5. Coverage of the Phase 0 audit（監査 Finding → 反映先）

| 監査 F | 反映先（Vision §） | 状態 |
|---|---|---|
| F1 tiers | §5, §4, §60, §65 #31 | 反映。R22 の名称衝突あり |
| F2 time / authority | §15, §23, §54, §65 #32 | 反映。R5, R6 の補足要 |
| F3 stream contract | §23, §28, §31, §65 #33–34 | 反映。R2（TX 側）要 |
| F4 mock envelope | §12, §13, §14, §17, §22, §56, §65 #35 | 反映。R7, R17 の補足要 |
| F5 session | §3, §50, §54, §57, §66, §65 #37 | 反映。R3, R18 の補足要 |
| F6 composite / coherence | §8, §11, §25, §39, §45, §65 #36 | 反映。R1 の退行あり |
| F7 generic matcher | §8, §10 | 反映。R4 の順序修正要 |
| F8 descriptor / ABI / cycles / deadlines | §19, §32 | 反映。R9 の名称統一要 |
| F9 DataContract registry | §21 | 反映 |
| F10 schema-first / builder | §9, §10, §65 #39 | 反映。R12 |
| F11 PrepareReport | §10, §11, §13, §52 | 反映。R16 は確認事項 |
| F12 lease modes | §3, §53 | 反映。R18 |
| F13 validator not optimiser | §10, §20, §31, §63, §65 #41 | 反映。**R21 の矛盾あり** |
| F14 no structural mutation | §27, §64, §65 #40 | 反映 |
| F15 calibration | §26, §25 | 反映 |
| F16 counters / policy table | §29, §53 | 反映 |
| F17 RF envelope | §8, §52, §43, §62, §65 #42 | 反映。R3 で Session にも適用が必要 |
| F18 step-driven | §15, §32 | 反映。R5 |
| F19 manifest structure | §50 | 反映 |
| F20 RFNoC | §4, §20, §32, §35, §64, §67 | 反映。CMA §1 の残存（R11） |
| F21 TX tap / self-coupling | §16, §23, §46 | 反映。R2 が TX 側を補完 |
| F22 three axes | §7 | 反映。R8（Authority 役割） |
| F23 environment | §8, §16, §17, §58 #8/#14, §65 #38 | 反映 |
| F24 process boundary | §35, §62 | 反映 |
| F25 node-qualified ids | §49 | 反映 |
| F26 one-shot helper | §41（CMA §24） | 反映 |
| F27 probe | §30, §63 | 反映 |
| F28 taint | §28, §63 | 反映 |
| F29 explicit conversion | §21, §34, §64 | 反映 |
| F30 sensors by reference | §47, §63 | 反映 |
| F31 timing class per instance | §38 | 反映 |
| F32 taxonomy | §37, §60, §63 | 反映 |
| F33 no metrics framework | §29, §6, §63 | 反映 |
| F34 v3 behavioural compat | §61 | 反映 |

---

## 6. Verdict and next steps

```text
READY
（Phase 1 の型を書く前に R1, R2, R21 を修正すること。R3, R6, R17, R22 は Core freeze 前。）
```

**根拠：**

- 監査の 34 Finding はすべて反映され、Kernel 概念は増えていない。Core 境界の判断（3 層、validator-not-optimiser、provider-declared coherence、RFNoC = radio capability）は改訂後も一貫している。
- 見つかった問題は**改訂が持ち込んだ局所的な矛盾**であり、方向転換を要するものはない。R1 / R2 / R21 は数行の修正だが、いずれも Phase 1 で最初に書く型（Spec の resources、SampleBlock flags、BindingProfile の placements）に直結するため、書き始める前に直す。
- R22（Kernel の二義）は用語だが、Core 境界を語る文書の中心語なので Core freeze 前に解消する。

**推奨する順序：**

1. R1, R2, R21 を Vision / CMA に反映（Spec 形・TX 意味論・placement の置き場所）。
2. R3, R6, R17, R22 を反映。
3. R4, R5, R7–R18 は Phase 1 の作業中に随時。
4. Phase 1 開始時に R13（規範部分の `design/` への切り出し）を行い、Vision を「why + invariants + links」に戻す。

*本文書は評価のみを行い、Vision / CMA には変更を加えていない。*
