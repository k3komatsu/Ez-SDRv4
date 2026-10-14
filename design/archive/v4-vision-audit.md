# Ez-SDR v4 Vision — Adversarial Architecture Audit

> **Status:** Phase 0 Architecture Gate review（実装開始前の最終設計監査）
> **Reviewed documents:** `Ez-SDR_v4_ARCHITECTURE_VISION.md`（以下 *Vision*、§番号はこの文書の章番号）, `Ez-SDR_v4_core_module_architecture.md`（以下 *CMA*、§番号はこの文書の章番号）
> **Repository evidence:** `v3/`（D言語実装、UHD C++ bridge、Python client、config例、changelog）
> **External evidence:** UHD / GNU Radio 3.x & 4 / srsRAN Project / OpenAirInterface rfsimulator / SoapySDR / SigMF / Linux tuntap / Rust ABI の一次資料（Appendix B に URL 一覧。VERIFIED = 一次資料で確認、INFERRED = 推測・一般知識）
> **Scope:** 設計レビューのみ。実装・スキーマ・ABI の変更は行っていない。監査作成時点では Vision 文書も未変更。§12.1–§12.4 の全項目（P0 / P1 / P2 / P3 と明示的 reject）は 2026-09-21 に Vision / CMA へ反映した（各文書冒頭の Revision 注記を参照）。  
> **Structure note (2026-09-21):** Vision は同日中に `design/vision/` 配下の 11 ファイルへ分割された。§番号は不変で、本文書の「Vision §N」参照はそのまま有効。ルートの `Ez-SDR_v4_ARCHITECTURE_VISION.md` が索引（§ → ファイル対応表）と改訂履歴を持つ。  
> **CMA retirement note (2026-09-21):** `Ez-SDR_v4_core_module_architecture.md`（CMA）は同日退役し `design/archive/` に保管された。本文書の「CMA §N」参照は保管版を指す。保管版冒頭に CMA § → Vision § の対応表がある。
> **Reviewer stance:** adversarial / skeptical / evidence-driven / minimal-core oriented。Vision の主張を前提として受け入れず、「この設計で数年研究を続けたときどこで壊れるか」を探した。

---

## 目次

1. Executive Summary
2. Architecture Strengths
3. P0 Findings（実装前に必ず解決）
4. P1 Findings（初期 Core 設計へ反映）
5. P2 / P3 Findings（将来拡張点の確保で足りるもの）
6. Core Boundary Audit
7. Module Taxonomy Audit
8. Mock / Simulation Architecture Audit
9. Research Use-case Matrix
10. Comparison with Existing SDR Systems
11. Missing Design Invariants
12. Recommended Vision Changes
13. Recommended Minimal Core
14. Core + Mock Implementation Readiness（判定）
- Appendix A — Architecture Stress Tests A〜J の End-to-End 検証
- Appendix B — Evidence index（v3 該当箇所・外部一次資料）

---

## 1. Executive Summary

### 1.1 Vision 全体の評価

Vision の**方向性は正しい**。特に以下の判断は数年スパンで見ても壊れにくい。

- 「Core は transaction coordinator であり per-sample scheduler ではない」（Vision §11, §32 / CMA §28-29）
- 「Mock は UHD の偽物ではなく Radio Contract の対等な Provider」（Vision §12 / CMA §10）
- 「ExperimentSpec は intent、ExecutionPlan は implementation、real-time path は Spec を解釈しない」（Vision §10）
- 「ExperimentSpec をプログラミング言語にしない」（Vision §9）
- 「Run は自分が Simulation / Emulation / HIL / Hardware のどれかを記録する」（Vision §14 / CMA §17）
- 「typed RuntimeEvent、console text 禁止」（Vision §29 / CMA §36）

しかし、Vision は **「何を目指すか」は詳細に書いているが、「何を決めないと後で壊れるか」を決めていない**。列挙されている Core 概念は約 45 個（§5 の 22 個に §19/§25/§26/§27/§28/§30/§31/§32/§34/§38/§49 の追加分）に達し、それらを「Core が所有する」と宣言している。一方で、それらの間の**依存関係・安定性の階層・具体表現（特に時間・バッファ・ストリームの規範的意味）**は未定義である。これが本監査で見つけた最大の構造的リスクである：**「Core」という語が microkernel と domain vocabulary の両方を指しており、freeze の単位が定まっていない**（Finding 1）。

### 1.2 実装開始可能か

**READY WITH REQUIRED CHANGES**（§14 で根拠を詳述）。

Core + Radio Model + MockRadio + SimulationChannel の実装に進んでよいが、その前に **P0 の 6 項目**（§3）を Vision に反映し、特に以下の 3 つの「型」を先に決める必要がある。これらは最初のマイルストーンのあらゆる Provider / Processor シグネチャに現れ、後から変えると全 Module Contract が破壊されるからである。

1. **TimePoint の具体表現と Time Authority**（誰が時間を進めるか：device timekeeper か simulation kernel か）— Finding 2
2. **SampleBlock / stream の規範的セマンティクスと buffer ownership**（gap 表現、不変性、MemoryDomain handle）— Finding 3
3. **Resource の粒度**（composite device と provider-declared coherence）— Finding 6

### 1.3 最大の設計リスク（要約）

| # | リスク | 一言で |
|---|---|---|
| 1 | Core の二重定義 | 45 概念を一括 freeze すると麻痺、しないと約束が空になる |
| 2 | 時間の権威が未定義 | virtual time が「後付け」になり、Python の `sleep` が Mock と実機で違う意味になる |
| 3 | ストリーム意味論が未定義 | v3 は RX overflow を黙殺し RX にタイムスタンプがなかった。同じ穴が v4 の最初の契約に空く |
| 4 | Mock が「理想」を実装する | OAI rfsimulator と同じ false confidence。lead time / coercion / stop tail をモデルしない Mock は昇格を保証しない |
| 5 | Easy API と Run モデルの衝突 | 対話的セッションを Run として表現できないと Easy API が「第二のアーキテクチャ」になる |
| 6 | coherence の所有者が曖昧 | UHD `multi_usrp` は 1 ハンドルで複数機を束ねる。Core が Provider 横断の coherence を推論すると破綻 |

### 1.4 Core / Module 分離は妥当か

**分離の方針は妥当、境界線の引き方は 2 箇所で修正が必要**。

- Core を **Kernel（lifecycle・lease・event・manifest・time primitives・generic capability matching）** と **Vocabulary（Radio Model、Calibration、Coherence、PerformanceEnvelope 等の共有語彙 crate）** に分け、安定性ポリシーを別にする（Finding 1）。
- **RFNoC は Processing Module の placement 先ではなく Radio Provider 内部の capability**（Replay による repeat、DDC/DUC による rate 変換）として位置付ける（Finding 20）。Vision §20/§32 の「同じ Processor model を CPU/WASM/GPU/RFNoC に placement」は、RFNoC については成立しない。

### 1.5 Core + Mock 実装へ進める状態か

P0 6 件（すべて設計文書レベルの決定であり、実装量ではない）を反映すれば進めてよい。さらに、Mock は Vision §13 の「Simulation Environment」を **discrete-event simulation kernel** として設計しなければ、受け入れテスト #2（virtual time faster than wall clock）と #3（seed で再現）が同時に成立しない（Finding 2, 18）。

---

## 2. Architecture Strengths

無理に問題を作る必要はない。以下は**そのまま維持すべき**点であり、他システムの失敗を正しく回避している。

| 強み | Vision 該当 | なぜ維持すべきか |
|---|---|---|
| Core は sample scheduler ではない | §11, §32 / CMA §28-29 | GNU Radio 3.x は汎用 TPB scheduler を Core に持ち、RT 保証が best-effort のままになった（Appendix B: GR3 `realtime.h`）。Island 方式は正しい回避 |
| Mock = Radio Contract の対等 Provider | §12 / CMA §10 | v3 の `SimpleMockClient` は Python 側の別物で、`sync()` の意味すら実機と異なっていた（`v3/client/ezsdr.py`）。Vision はこの失敗を明示的に否定している |
| intent / binding 分離 | §8 / CMA §7-8 | 実験の可搬性の唯一の基盤。ExperimentSpec に `provider: uhd` を書かせない規律は正しい |
| Spec は言語ではない | §9 | v3 は設定 JSON に `CONSTANTS` と `!COMPUTE(...)` を追加した（`v3/changelog/v3.0.21.md`, `v3/source/settingfile.d`）。同じ圧力は v4 にも必ず来る。禁止条項があることは重要（Finding 10 で「代替手段」を追加提案） |
| compile before RUN、RT path は JSON を見ない | §10 | 正しい。v3 はメッセージごとに `BinaryReader` でパースしていた |
| typed RuntimeEvent | §29 / CMA §36 | v3 は UHD の overflow/underflow を `std::cout` に出すだけで、RX の error_code すら見ていなかった（`v3/cpp/uhd_usrp/multiusrp.cpp` `continuousReceiveImpl`）。srsRAN Project も typed event へ写像している（Appendix B） |
| ExecutionClass の記録 | §14 / CMA §17 | 「Simulation passed ≠ hardware verified」を Manifest に刻む設計は AI agent 運用の前提として正しい |
| 継続性・妥当性を data に持たせる | §28 | RuntimeEvent だけでは publication-grade にならないという認識は正しい（表現の具体化が Finding 3） |
| Probe は backpressure を与えない | §30 | 正しい。ただし新概念にする必要はない（Finding 26） |
| TxBurst が first-class | §22 | reactive radio の必須要件。v3 には存在せず、`onTime()` による開始時刻指定しかなかった |
| Peripheral timing class | §38 | USB 制御を sample-accurate と偽らない規律は ISAC/IBFD で必須 |
| GPIO を Radio と疎結合に | §39 / CMA §26 | 方針は正しい（ただし「同一デバイスのサブリソース」であることの明示が必要：Finding 6） |
| Host-I/O ≠ Peripheral | §40 / CMA §22 | OS 境界と実験装置は寿命・権限・時刻源が違う。分けて正しい |
| 「やらないこと」の明文化 | §63-64 | 稀に見る良い節。ただし §12（本監査）で「明示的に reject」すべき項目を追加する |
| 開発順序：Core → Mock → UHD | §57-59, §67 | 正しい。Mock→X310 parity test をマイルストーンにしている点は特に良い |

---

## 3. P0 Findings（実装開始前に必ず解決。後からでは Core / API 破壊になる）

> P0 はすべて「設計文書レベルの決定」であり実装量ではない。しかしどれも最初のマイルストーン（Core + Mock）の Module Contract の**型**に現れるため、先送りすると全 Provider / Processor シグネチャの破壊になる。

### Finding 1 — 「Core」が microkernel と domain vocabulary の二重定義になっており、freeze の単位が存在しない

- **Severity:** `P0`
- **Area:** Core / Module boundary
- **Current design:** Vision §5 は Core が定義する概念を 22 個列挙し、§19（Processor/Reactor/Timer/TxBurst）、§25（DeviceGroup/CoherentGroup/Array）、§26（CalibrationArtifact）、§27（update class / GraphEpoch）、§28（ContinuityMap/ValidityMap/Gap/Taint）、§30（Probe）、§31（MemoryDomain/DataLink/BufferContract/TransferRequirement）、§32（ExecutionIsland）、§34（PerformanceEnvelope）、§38（timing class）、§49（ComputeNode/NetworkLink/Placement）で追加している。合計約 45 概念。§65 invariant 4 は「Core remains a small Experiment Microkernel」、invariant 30 は「Core growth not proportional to feature count」と宣言する。§60 の crate 案は `ezsdr-types / kernel / module-api / experiment / time` に分かれているが、本文はすべてを「Core が所有」と書く。
- **Problem:** Vision 自身が「機能を 1 つ足すごとに Core 概念を 1 つ足す」構造になっている（MIMO→CoherentGroup、IBFD/ISAC→CalibrationArtifact、OTFS→Tensor、GPU→MemoryDomain、分散→ComputeNode）。これは invariant 30 の否定である。さらに、これら 45 概念の**安定性ポリシーが一律**（すべて「stable Module Contracts」の一部）なので、(a) 一括 freeze すれば cell-free で CoherentGroup にサイト間 ClockRelation を持たせたいとき Core semver major になり、全 Provider の再認証が必要になる、(b) freeze しなければ「stable contract」の約束が空になる、のどちらかに落ちる。
- **Failure scenario:** v4.2 で cell-free MIMO を始める。CoherentGroup（Core 型）に `inter_site_relation` を追加したい。Core 変更 → Mock / UHD / Soapy / Peripheral plugin / Python client がすべて追随。あるいは追随を避けるため `extensions.cellfree.inter_site_relation` に逃がす → CoherentGroup は形骸化し、実験ロジックは拡張フィールドに依存する（v3 の `setParamToDevice("set_time_unknown_pps_to_zero", "[]")` と同じ「文字列 escape hatch が本道になる」パターン）。
- **Why it matters:** 「Core を小さく保つ」は概念数ではなく**freeze する契約の範囲**の問題である。範囲を決めないまま実装を始めると、最初の crate 分割がそのまま freeze 単位になる。
- **Can it be deferred?:** **No.** crate 境界と「何を stable と呼ぶか」は最初のコミットで決まる。
- **Recommended direction:** Core を 3 層に分け、それぞれ別の安定性ポリシーを持たせる。
  1. **Kernel**（v4.0 で freeze、semver 厳守）：Run/Session lifecycle と transaction（prepare/arm/start/stop/cleanup）、Lease、Event envelope、`ClockDomain/TimePoint/Duration/ClockRelation/Deadline` と `TimeAuthority` interface、`SampleBlock/BufferRef/MemoryDomain` と Stream Contract、`DataContract` identity registry、generic Capability/Constraint matching、`ExperimentSpec/BindingProfile/ExecutionPlan/Manifest` の envelope（中身は namespaced）、Module API（descriptor と registry）。
  2. **Vocabulary crates**（個別に version、additive change を原則）：Radio Model（channels/streams/RF parameters/TimingEnvelope/PerformanceEnvelope/coherence basis）、Calibration vocabulary、Peripheral model、Endpoint model、Processing descriptors の標準 port 型。Module は Kernel + 必要な Vocabulary の特定 version に依存する。
  3. **Extensions**（namespaced、unstable）：backend 固有。
  「Core は小さい」invariant は **Kernel にだけ**適用する。Vision §5 の列挙は「Kernel が所有」「Vocabulary として提供」に二分して書き直す。
- **Core or Module?:** Kernel = Core。Vocabulary は Core 配下だが独立 version の共有 crate。CoherentGroup / CalibrationArtifact / PerformanceEnvelope / Tensor shape はすべて Vocabulary。
- **Evidence:** Vision §5, §19, §25–§32, §34, §38, §49, §60, §65 (4, 30); CMA §4, §39 (15)。GNU Radio 4 は 4.0.0-RC1 で「core API は今後 additive のみ」と宣言しつつ blocks は別に進化させている（Appendix B: GR4 RC1）— 同じ階層化の発想。v3: `IDevice.setParam(key, value)` 文字列 escape hatch が同期処理の本道になった（`v3/source/device/package.d`, `v3/cpp/uhd_usrp/multiusrp.cpp setParam`）。
- **Confidence:** High

### Finding 2 — 時間の具体表現と「時間の権威（Time Authority）」が未定義

- **Severity:** `P0`
- **Area:** Time / Simulation / Core
- **Current design:** Vision §15/§23/§24、CMA §18 は `ClockDomain, TimePoint, Duration, Deadline, ClockRelation` を Core 概念とし、「floating-point seconds を基本表現にしない」「UHD Module は device time、Simulation Module は deterministic virtual time を ClockDomain として expose」「functional simulation は wall clock より速く走る」と書く。§58 受け入れテスト #2「virtual time works faster than wall clock」、#3「deterministic runs reproduce with a seed」。
- **Problem:** 3 点が未定義。
  1. **表現**：UHD は `int64 秒 + double 端数`、srsRAN は `uint64 sample ticks`、SoapySDR は `int64 ns`（Appendix B）。Ez-SDR は 3 者と相互変換しつつ、20 Msps と 25 Msps（同じ 200 MHz master から派生）のストリーム間で**厳密に**サンプルを対応付ける必要がある。`f64 sample_rate` と `f64 seconds` では 10^12 サンプル級で丸めが出る。
  2. **権威**：実機では device timekeeper が時間を進め host は待つ。Simulation では**誰が virtual time を進めるのか**が書かれていない。Reactor の `SetTimer`、Processor の deadline、Peripheral の latency model、そして **Python 側の待ち**が同じ時間軸に従う必要がある。Vision は VirtualClock を Simulation Environment（Module）に置きつつ、Reactor timer と Python API は Core レベルにある。両者を結ぶ interface がない。
  3. **epoch**：USRP time の t=0 は PPS 同期時に任意に設定される。Run の epoch を host wall clock（UTC/TAI）にどう関係付けるか、Manifest に何を記録するかが未定義。camera / positioner との相関（§47）は epoch の関係付けなしには成立しない。
- **Failure scenario:** Python が `sdr.tx.burst(x, at=sdr.time.now()+0.01); time.sleep(0.5); y = sdr.rx.capture(N)` と書く。functional simulation は 10 秒分を 0.3 秒で走るので `time.sleep(0.5)` は virtual time で「無限に長い」か「順序不定」になる。受け入れテスト #2 と #3 が同時に成立しない。第二のシナリオ：2 ストリームの `first_sample_time`（f64 秒）を比較して 4 チャネル整列を判定するテストが、長時間 Run で間欠的に 1 サンプルずれる。
- **Why it matters:** TimePoint は SampleBlock、Event、Action、TxBurst、Deadline、Manifest のすべてに入る。表現を後から変えると全契約が変わる。Time Authority は Mock（DES kernel）の構造そのものを決める。
- **Can it be deferred?:** **No.** Mock の最初の設計判断（DES kernel か否か）と Python API の待ち方（`sleep` か `wait_until` か）が直接依存する。
- **Recommended direction:**
  - `TimePoint { domain: ClockDomainId, ticks: i64 }`、`ClockDomain { tick_rate: Rational{num,den}, epoch: EpochRef }`。各 stream の `SampleClock` は device clock から**厳密有理数**で派生した ClockDomain とし、有理関係のある domain 間は exact 変換、それ以外は `ClockRelation{offset, drift, uncertainty, measured_at, validity}` 経由でのみ変換可（変換結果は uncertainty を伴う型にする）。
  - Kernel に `TimeAuthority` interface（`now(domain)`, `wait_until(TimePoint)`, `schedule(TimePoint, Action)`）を置く。Hardware Run では Radio Provider の timekeeper + host monotonic の ClockRelation が実装、Simulation Run では **discrete-event simulation kernel** が実装。MockRadio / SimulationChannel / MockPeripheral はこの DES kernel 上のモデルであり、RealtimeEmulation は同じ DES kernel を wall clock に pace したもの。
  - Python API は device time に対する `sleep` を持たない。`run.wait_until(t)`, `run.wait_for(event)` のみ。Easy API の `capture(N)` は「`now + lead` に開始」を内部で決める（lead は Finding 4 の envelope から）。
  - Manifest に `epoch ↔ UTC` の ClockRelation（不確かさ付き）を必須記録。
- **Core or Module?:** 型と `TimeAuthority` interface は Kernel。DES kernel・timekeeper 実装は Module。
- **Evidence:** Vision §15, §23, §24, §47, §58; CMA §18。UHD `time_spec_t`（`metadata.hpp`）、srsRAN `time_spec.to_ticks(srate)`（`baseband_gateway_transmitter_metadata.h`）、SoapySDR `getHardwareTime` ns（`Device.hpp`）— Appendix B。v3: `onTime(t)` が float 秒→ns 変換（`v3/client/ezsdr.py`）、RX 側に timestamp が一切ない（`multiusrp.cpp continuousReceiveImpl` は `md.time_spec` を捨てる）。ISAC 実例（X410×3 + MOCAP）は開始時刻オフセットの後処理で相関している — ClockRelation そのもの（Appendix B: Yan et al.）。
- **Confidence:** High

### Finding 3 — Stream / SampleBlock の規範的セマンティクスと buffer ownership が未定義

- **Severity:** `P0`
- **Area:** Core data plane / Radio / Processing
- **Current design:** Vision §23 は SampleBlock の概念形 `{samples, first_sample_time, sample_rate, sequence, validity/flags}` を示す。§28 は `ContinuityMap / ValidityMap / Gap / Taint` を「possible concepts」として挙げる。§31 と CMA §27 は `BufferContract` を Core が model すると書き、「Core は一つの ring 実装を hard-code しない」と述べる。
- **Problem:** 「概念がある」だけで**規範（Provider が何を保証し何を禁止されるか）**がない。実機で決めなければならない事項：
  1. **overflow 時の表現**：UHD はエラー時に 0 サンプルを返し、`OVERFLOW` は「内部バッファ溢れ」と「host 側 sequence error」の両方に overload されていて `out_of_sequence` で区別する。連続モードでは約 50 ms 後に自動再開するので、**overflow 1 回 = 最低 50 ms のギャップ**である（Appendix B: UHD metadata.hpp / rfnoc_rx_streamer.cpp）。Provider はギャップを**ゼロ埋めしてはならない**（黙って埋めると publication data が汚染される）。
  2. **多チャネルの部分欠損**：`ALIGNMENT` error、片チャネル欠損。validity は per-channel でなければならない。
  3. **block size の非保証**：実機は packet 単位（10GbE 8000 byte ≒ sc16 で 2000 サンプル）で届く。Mock が任意長を返すなら、Processor が「4096 固定」を仮定するバグを Mock で検出できない。
  4. **スケール規約**：fc32 の full-scale ±1.0 が ADC full scale を意味するか。Mock の clipping 閾値と実機を一致させないと IBFD の saturation 観測（§46）が Mock と実機で意味を変える。
  5. **ownership**：誰が確保し、いつ解放し、発行後に可変か、fan-out（Probe）はコピーか参照か、MemoryDomain（host / pinned / GPU / WASM linear）をどう表すか。`&[T]` を契約に焼き込むと WASM/GPU で契約が壊れる。
  6. **TX 側**：block は先頭サンプルの target TimePoint を持つ（TxBurst）。連続 TX は contiguity 前提で、破れたら `UNDERFLOW/TIME_ERROR`。UHD は start-of-burst ごとに CORDIC をリセットするので**すべての SOB に time spec が必要**（Appendix B: page_sync）。
- **Failure scenario:** v3 の再演：`continuousReceiveImpl` は `md.error_code` を見ない。overflow が起きても Python には連続配列が返り、OFDM チャネル推定の平均に 50 ms のギャップが混入する。誰も気づかない。第二：Mock で 4096 サンプル固定を仮定した検出器が X310 で 2000 サンプル packet を受けて境界跨ぎを取りこぼす。第三：SIC Processor が RX block を参照保持したまま executor が ring を再利用 → use-after-free、回避のため全コピー → 200 Msps で破綻。
- **Why it matters:** SampleBlock と BufferRef は Radio / Processing / Host-I/O / Simulation の**全**契約に現れる。最初の Mock 実装がこの型を決めてしまう。
- **Can it be deferred?:** **No.**
- **Recommended direction:** Kernel に **Stream Contract**（規範文書 + 型）を置く。
  - `SampleBlock`：**publish 後 immutable**、参照カウント（Arc 相当）、発行側 island の pool/arena から確保、`MemoryDomain` タグ付き。フィールド：`first_sample_time: TimePoint(SampleClock)`, `len`, `channels`, `valid: per-channel mask or range list`, `flags: {GAP_BEFORE{lost: Option<u64>}, SEQ_DISCONTINUITY, LATE, PARTIAL_CHANNELS, RESTARTED}`, `format/scale: DataContract ref`。
  - 規範：stream 内で時刻は単調、ギャップは flag と時刻の跳びで表し**決して埋めない**、block 長は非保証（Mock は fidelity option として block 長を jitter させる）、full-scale 規約は DataContract に含める。
  - fan-out は参照共有（コピーなし）。backpressure は link ごとの宣言 policy `{block, drop_oldest, drop_newest}`。Probe link は常に drop 系。
  - TX block は target TimePoint を持ち、Provider は lead 不足を `LATE` として typed event 化する。
  - Artifact レベルの `ContinuityMap` は block flags から**導出**する（別途手で作らない）。SigMF 出力は captures の `core:sample_start` + `core:global_index` で gap を表現し、per-channel validity と calibration 参照は `ezsdr` 拡張 namespace で持つ（SigMF 公式・community 拡張に validity/gap/calibration の語彙は存在しない — Appendix B）。
- **Core or Module?:** 型と規範は Kernel。pool / ring / DataLink 実装は Module。
- **Evidence:** v3 `multiusrp.cpp continuousReceiveImpl`（error_code 無視）、`uhd_rfnoc.cpp continuousReceive`（error を cout、LATE_COMMAND のみ再開）、`hackrf.d continuousReceiveCallback`（queue 満杯で 1000 回 yield 後に printf して破棄）；UHD `rx_metadata_t` OVERFLOW/out_of_sequence、`OVERRUN_RESTART_DELAY 0.05 s`、`ALIGNMENT`；GR4 `CircularBuffer` の claim/publish（publish 後は reader 専有）；srsRAN `baseband_gateway_buffer_pool`（RT path 無 allocation）；SigMF `core:global_index` と拡張一覧 — Appendix B。
- **Confidence:** High

### Finding 4 — Mock は「理想」ではなく「制約エンベロープ」を実装しなければ昇格の約束が空になる（Reactor の lead-time semantics を含む）

- **Severity:** `P0`
- **Area:** Simulation / Radio / Reactor / Validation
- **Current design:** Vision §12–§17 は Mock を first-class とし、CMA §13 は「serious MockRadio may *eventually* model transport delay / overflow / underflow / clock drift」と書く。§14 の `SimulationFidelity` は単一 enum（functional / timing-model / RF-model / hardware-quirk-model）。§22 TxBurst に `deadline` はあるが、Provider が**自分の時間制約を宣言する**仕組みも、Mock がそれを**強制する**義務もない。§38 は Peripheral にのみ timing class を要求している。
- **Problem:** 「BindingProfile を変えるだけで実機で動く」が成立する条件は、実機の制約が (a) validate() 時に見え、(b) Mock 上で同じ制約が**強制される**ことである。実機の制約は具体的である：timed command の最小 lead（transport / rate / packet size 依存）、起動遅延（PPS 同期は最大 2 秒）、stop 後の tail drain、rate/gain/frequency の coercion、overflow 後 50 ms の再開ギャップ、timed command queue の深さ（X3x0 では streaming なしに timed command を連発すると DDC/DUC の queue が溢れ full restart が必要）、Replay の word/item alignment。すべて accept する Mock は **OpenAirInterface rfsimulator と同じ**である：同じ device interface を実装し lockstep で動くが、late / underflow / overflow を一切模倣せず、timestamp gap は warning のみ（Appendix B: rfsimulator/simulator.cpp）。これは「実機で初めて壊れる」false confidence の既知の先例である。また単一 fidelity enum では「timing は模倣、RF は未模倣」を表せない。
- **Failure scenario:** PING→PONG Reactor が `target = rx_time + 50 µs` で PONG を出す。Mock（functional）では成功。X310 では全 PONG が `TIME_ERROR`（`L`）になり実機は一度も応答しない。開発者は「実機が不安定」と結論する。第二：Spec が `sample_rate_hz: 19_500_000` を要求。Mock は受理、X310 は 20 Msps に coerce（UHD は warning を log し `get_rx_rate()` で読み戻せと書く — Appendix B）。OFDM シンボル境界がずれるが、実験ロジックには何も報告されない。
- **Why it matters:** これは Vision の中心命題（§2, §59, §68）の成立条件である。Reactor Action の time semantics（lead validation, late policy）は Kernel の Action 型に入るため後から変えると Reactor 契約が壊れる。
- **Can it be deferred?:** **No**（envelope schema と Action semantics）。RF-model の充実は deferrable。
- **Recommended direction:**
  1. Radio Model に **TimingEnvelope** を必須化：`min_timed_command_lead`, `startup_latency_max`, `stop_tail_max`, `timed_command_queue_depth`, `overflow_restart_gap`, coercion 関数（rate grid = master_clock / N、gain step、freq resolution）。既存の `PerformanceEnvelope`（§34）と並置する。
  2. **Mock は emulate する profile（`x310-like`）の envelope を強制する**：lead 不足の command は `LATE_COMMAND / TIME_ERROR` を実機と同じ typed event で出す、rate は同じ grid に coerce、stop は tail を届ける、overflow injection は 50 ms ギャップ + `RESTARTED` を再現。RF 模倣は SimulationChannel に残す。
  3. `validate()` は Reactor / schedule の応答制約（例：SIFS ≥ `min_timed_command_lead`）を**束縛先 Provider の envelope**に対して検査する（Mock でも実機でも同じコード）。
  4. Kernel の `TxBurst` Action semantics に **late policy** `{reject_at_plan, send_asap_and_flag, drop_and_flag}` を定義する。
  5. `SimulationFidelity` を **per-aspect fidelity vector** `{timing, continuity/faults, coercion, rf, transport}` に置き換え Manifest に記録する。昇格判定は aspect ごとに行う。
- **Core or Module?:** envelope schema → Radio Model（Vocabulary）；強制 → Mock module；validate の検査・late policy・fidelity vector → Kernel。
- **Evidence:** v3 `syncUSRPLoopTXRX`（sleep 1 s → `set_time_unknown_pps_to_zero` → sleep 1 s → wait → `onTime(0.2)`）、v3 `stopContinuousReceiveImpl` drain loop（1 回 0.1 s timeout）、v3 `test_v3_TXRX.py` の `onTime(0.1)`；UHD `set_time_unknown_pps` ≤ 2 s、`LATE_COMMAND` 後は radio idle で再 command 必要、`OVERRUN_RESTART_DELAY`、rate coercion、X3x0 command queue overflow — Appendix B；OAI rfsimulator（lockstep、late/underflow/overflow 非模倣）と OAI `tx_sample_advance` 設定項目（TX lead を device ごとに持つ）；srsRAN `max_processing_delay_slots` / `time_alignment_calibration`；NVIDIA Aerial の RU emulator は O-RAN timing window を検証する（Appendix B）。
- **Confidence:** High

### Finding 5 — Easy API（対話的セッション）と Run / ExperimentSpec モデルの関係が未定義

- **Severity:** `P0`
- **Area:** Core lifecycle / Python frontend
- **Current design:** Vision §3 の Easy API（`sdr.rx.frequency = ...; sdr.tx.repeat(x); y = sdr.rx.capture(N)`）、§54「Python と ExperimentSpec は complementary」、§57「最初のマイルストーンで Easy API が software のみで動く」、§50「every execution creates a Run」、§10「Spec は RUN 前に compile され RT path は Spec を解釈しない」、§66「Simple human experiment が同じ Core を使うこと」が litmus。
- **Problem:** 対話的利用は「設定を変える → repeat 開始 → capture → gain 変更 → capture」という**長寿命コンテキストでの逐次変更**である。これを Run にどう写像するかが未定義：
  - `capture` ごとに Run なら、その Spec は何で、先に始めた TX repeat の waveform hash は provenance にどう現れるのか。
  - `with` ブロック全体が 1 Run なら、`sdr.rx.gain = 20` は runtime mutation であり「Spec compiled before RUN」と衝突する。
  未決のまま実装すると、Easy API は v3 と同じ**独自コマンドチャネル**（Spec / Plan / Manifest を通らない）になり、§66 の litmus に反する「第二のアーキテクチャ」が生まれる。
- **Failure scenario:** 学生の論文が「gain 20 で取得」と書くが、capture Run の Manifest には前の「Run」で設定された TX waveform hash がない。再現不能。より悪いケース：Easy API 経路が Manifest を一切生成しない。
- **Why it matters:** 最初のマイルストーンそのもの。Session を Kernel の Run 種別として持つか否かは lifecycle 状態機械の設計を変える。
- **Can it be deferred?:** **No.**
- **Recommended direction:** Kernel に **Session** を「action log を持つ Run 種別」として定義する。`connect()` は Session Run を開き（BindingProfile 既定の capability から暗黙 Spec を生成）、Easy API の各呼び出しは typed Action（`SetParameter{class: cold}`, `StartRepeat{waveform_ref}`, `Capture{n, at}`, `Stop`）として **TimePoint 付きで action log に追記**される。capture 結果は Session Run の Artifact。Manifest = 暗黙 Spec + action log + effective configs。再現 = log の replay。Session 内の runtime 変更は Finding 14 の宣言済 update class 経由のみ。Session 内の `sdr.run(spec)` は child Run（publication path）。lifecycle 機構は 1 つ、コマンドチャネルは増やさない。`__exit__` の挙動は Lease（Finding 12）で決める。
- **Core or Module?:** Kernel。
- **Evidence:** Vision §3, §10, §50, §54, §57, §66; v3 の `MessageDispatcher` と `CyclicRX/TX Controller` のコマンド体系（provenance なし）— `v3/source/dispatcher.d`, `v3/source/controller/*.d`。
- **Confidence:** High

### Finding 6 — Resource の粒度：composite device と provider-declared coherence が必要。Core は Provider 横断の coherence を推論してはならない

- **Severity:** `P0`
- **Area:** Core resource model / Radio / MIMO / Peripheral
- **Current design:** Vision §8 の資源要求は `radio: {requires: {rx_channels: 2, tx_channels: 2, ...}}` というフラットな capability。§25 は `DeviceGroup / CoherentGroup / Array` を Core 概念とし CoherentGroup が「members, common clock source, LO topology, ...」を持つ。§39 / CMA §26 は GPIO を独立 Resource とし UHD GPIO Provider を挙げる。§45「CoherentGroup → aligned samples」。CMA §20「Module は他の concrete Module を呼ばない」。
- **Problem:** UHD では coherence と alignment は**単一ハンドル**（`multi_usrp` / `rfnoc_graph`。`addr0,addr1` で複数筐体を束ねる）の性質である。整列した多チャネル受信は「1 streamer に複数 channel」でのみ得られ（UHD は最初の stream command を近未来の `time_spec` で出して packet を整列させろと書く）、同期は同一オブジェクトからの timed command で行い、GPIO は radio の timekeeper を共有し（X3x0 の timed GPIO は radio の timed interface 経由 — Appendix B）、Replay DRAM は device 内部にある。Core が radio / gpio / timekeeper を**独立資源として独立 Provider に束縛**すると、(a) 2 筐体 4 チャネル coherent を 1 provider instance として表現できず、(b) GPIO provider と Radio provider が 1 つの device handle を取り合い、(c) v3 changelog v3.0.17 が示す「PPS を出す機体から順に初期化しないと起動に失敗する」arm ordering の置き場がない。逆に Core が独立 radio の集合から CoherentGroup を**合成**すれば、共有 clock/LO のない集合を coherent と誤認する。
- **Failure scenario:** ケース 2（X310×2、10 MHz/PPS 共有、4 ch coherent）。BindingProfile が `radio_a`, `radio_b` を別々に束縛し Core が CoherentGroup{a,b} を作る。各 provider が自分の `multi_usrp` を開く。単一 streamer が存在しないので sample は整列しない。整列を行う主体がないため `ALIGNMENT` も報告されない。MIMO 検出器は筐体間のランダムなオフセットを見る。Mock では成功していた。
- **Why it matters:** ExperimentSpec の `resources` の形と ExecutionPlan の依存順序は最初のマイルストーンで決まる。後から composite に変えると Spec スキーマと BindingProfile が両方壊れる。
- **Can it be deferred?:** **No.**
- **Recommended direction:**
  - Resource model を **composite tree** にする：Provider instance は `Device`（または `DeviceSet`）を expose し、その下に sub-resource `{channels, streams, timekeeper(ClockDomain), gpio_banks, replay/dram, sensors}` を持つ。
  - **Coherence は Provider が自分の channel 集合について宣言する**（`CoherentGroup` は Provider 所有データ。根拠 `basis: {shared_ref_clock, shared_pps, lo_sharing, timed_tune}` を明示）。Core は Provider **横断**では ClockRelation のみ合成し、CalibrationArtifact が主張しない限り coherent とは呼ばない。
  - Spec は `radio: {channels: 4, coherent: true}` と要求し、binding はそれを宣言できる**単一 provider instance**（例：UHD provider with `addr0,addr1`）に写像する。写像不能なら validate() が失敗する。
  - ExecutionPlan の fragment に依存辺（arm ordering：PPS master 先）を持たせる。Core の transaction coordinator は DAG 順に prepare/arm する。
  - GPIO は同一 provider instance の sub-resource。smart-antenna plugin は「GPIO capability」に束縛され、Core がその sub-resource へ解決する（CMA §20 のルールは維持される：同一 Module 内の依存は cross-module ではない）。
- **Core or Module?:** composite Resource tree と依存順序は Kernel；sub-resource の種類と coherence basis は Radio Model（Vocabulary）。
- **Evidence:** UHD page_sync（単一 `multi_usrp` + `set_command_time`）、`stream_cmd.hpp` の多チャネル整列注記、X3x0 `gpio_atr_3000` が radio の `timed_wb_iface` 上（source から VERIFIED、文書化は INFERRED）— Appendix B；v3 changelog v3.0.17（PPS ordering）、v3 config `args: "addr0=...,addr1=..."` で 1 MultiUSRP が 2 筐体を束ねる（`v3/docs/v3/usrp.md`）；RENEW hub / LuMaMi OctoClock tree / Techtile PTP（coherence は物理的分配の事実であり、ソフトが合成するものではない — Appendix B）。
- **Confidence:** High

---

## 4. P1 Findings（初期 Core アーキテクチャへ反映。後付けコストが大きい）

### Finding 7 — Capability matching が radio 固有キーを Core に焼き込む形になっている

- **Severity:** `P1`
- **Area:** Core / Radio Model
- **Current design:** Vision §8 / CMA §7 の例は `rx_channels, tx_channels, full_duplex, sample_rate_hz, hardware_time` を `requires` に直書きし、CMA §34 は「Experiment requires 2 RX ... MockRadio provides 2 RX ... → capability validation succeeds」と Core が radio キーを比較する図を示す。
- **Problem:** Core の matcher が radio キーを知ると、Peripheral / Endpoint / Processor の capability を足すたびに Core が育つ（invariant 30 違反）。また `sample_rate_hz` の一致は「等しい」ではなく「coercion 後に許容範囲内」という Provider 固有の判定である。
- **Failure scenario:** smart antenna の `beam_count`、TAP の `mtu`、GPU executor の `memory_bytes` を requires に書きたい。Core の matcher enum を毎回拡張し、Python client の validation も追随する。
- **Why it matters:** matcher は validate() の中核で、Spec スキーマの `requires` の形を決める。
- **Can it be deferred?:** No（matcher の形が Spec 形を決める）。実装は簡単。
- **Recommended direction:** Kernel の matcher は **schema-driven / generic**：`requires` は `{key: Constraint}`（`eq / range / set / min / max / present`）、Provider は `capabilities: {key: value | range | set}` を宣言、Vocabulary crate がキーの意味と型を定義する。coercion を伴うキー（rate 等）は Provider 側の `coerce(request) -> Result<applied, Vec<Warning>>` に委ね、matcher は「coerce 可能か」だけを問う。radio 固有キーは Radio Model に置く。
- **Core or Module?:** matcher = Kernel；キー定義 = Vocabulary。
- **Evidence:** Vision §8, §52, §65 (30); CMA §7, §34; UHD rate coercion（Appendix B）。
- **Confidence:** High

### Finding 8 — Processor / Reactor：Core が所有すべきは Descriptor と Action/Event 語彙であり、実行 ABI ではない。Island 間の cycle 規則と deadline の 2 種類を明文化する

- **Severity:** `P1`
- **Area:** Processing / Core / Real-time
- **Current design:** Vision §19「Core は Processor と Reactor の semantic contract を定義、実行 engine は Module」、CMA §19 は `ProcessorDescriptor / ReactorDescriptor / PortContract` を Core に置き executor を Provider とする。§32 ExecutionIsland、§20「abstraction を weak な LCD にしない」。「cycle / feedback」「multi-rate」「deadline の意味」は未記述。
- **Problem:** (1) 「semantic contract」が work() 相当の呼び出し規約を含むなら、それは GNU Radio の block API の再発明で、native / WASM / GPU で同じ呼び出し規約は成立しない（GPU は batch、WASM は linear memory への copy、native は slice）。(2) Reactor → TxBurst → Radio → RX → Reactor は graph 上の cycle。GR 系はサンプル edge の cycle を許さない。どの edge で cycle を許すかが未定義。(3) 「deadline」に 2 種類ある：Processor の**相対**処理予算（block 到着から）と TxBurst の**絶対**device-time 期限。混同すると admission check が意味を失う。
- **Failure scenario:** Core が `fn work(&mut self, inputs: &[&[T]], outputs: &mut [&mut [T]])` を契約にする。GPU executor は毎 block で host↔device copy を強いられ CPU より遅くなる（Vision §31 自身が警告する事態）。WASM executor は同じ slice を linear memory に写す 2 回目の copy を持つ。契約を変えると全 Processor が壊れる。
- **Why it matters:** Descriptor と ABI を分けるかどうかは module-api crate の最初の設計。
- **Can it be deferred?:** No（境界の決定）。executor 自体は後で良い。
- **Recommended direction:**
  - Kernel は `ComponentDescriptor { kind: Processor|Reactor, ports: [(name, dir, DataContract)], params: schema + update class, timing: {budget, batch_pref, stateful, parallelism}, impl_ref: hash/identity }` と Action/Event 語彙のみ。
  - **実行 ABI は Executor が所有**（native trait、WASM block ABI、GPU kernel launch 規約）。Executor 間の相互接続は DataLink（handle 渡し）であり、共通呼び出し規約ではない。
  - 規則：**SampleStream edge の cycle は禁止。cycle は Island 境界を跨ぐ Event/Action edge（非同期・キュー付き）でのみ許可**。適応フィルタの係数フィードバックは Processor 内部状態であり graph 構造ではない。
  - deadline を `RelativeBudget(Duration)`（Processor）と `AbsoluteDeadline(TimePoint)`（TxBurst / PeripheralCommand）に分けて型で区別する。miss は typed event + policy。
  - multi-rate は Island 内 executor の責務。Kernel は port の rate 宣言を validate に使うだけ。
- **Core or Module?:** Descriptor / Action / Event / cycle 規則 / deadline 型 = Kernel；ABI・scheduling = Executor Module。
- **Evidence:** Vision §19, §20, §31, §32; CMA §19, §29。GR4 の `processOne / processBulk / work` は block API を Core に持つ設計（Appendix B: Block.hpp）— Ez-SDR はこれを**しない**選択が可能。GR4 の scheduler は deadline / budget primitive を持たず「bounded execution time」は potential policy のまま（Appendix B: core README）。Agora は frame 単位の静的 pipeline を master → doer で回す（Appendix B）。
- **Confidence:** High

### Finding 9 — DataContract を open registry として設計しないと、Port が IQ 中心のまま固定される（逆に、汎用型システムにしてもいけない）

- **Severity:** `P1`
- **Area:** Core data model / Processing
- **Current design:** Vision §21 / CMA §21 は `SampleStream<T>, Packet/PDU<T>, Tensor<T, Shape>, Event<T>, Control, TimedAnnotation` を「eventually include」と列挙する。Port と DataContract は §5 で別概念。
- **Problem:** 2 つの失敗方向がある。(a) v4.0 が SampleStream しか実装せず Port 型を enum で閉じると、Packet/Tensor 追加時に Port が変わり全 Module が再コンパイル。(b) 逆に Tensor の shape 代数や generic `<T>` を Core の型検査に持ち込むと GR4 的な型システムを Core が抱える。
- **Failure scenario:** TUN/TAP 追加（Phase 9）で `Port::Packet` を enum に追加 → Mock / UHD の Radio Model crate まで再ビルド・再認証。OTFS で `Tensor<cf32, [N, M]>` を追加 → shape 検査を Core に実装する誘惑。
- **Why it matters:** Port の形は Module API の一部で凍結対象。
- **Can it be deferred?:** No（registry 形式の決定）。Packet / Tensor の実装は deferrable。
- **Recommended direction:** `Port = {name, direction, contract: DataContractId}`。`DataContract` は **identity + 属性 + 互換規則**を持つ open registry（例：`ezsdr.stream.cf32{full_scale=1.0}`, `ezsdr.pdu.bytes{max_len}`, `ezsdr.tensor.cf32{shape=[..]}`, `ezsdr.event.<schema-id>`）。Kernel の型検査は「identity が一致するか、宣言された互換変換があるか」だけ。Tensor の shape 整合は Executor / validate プラグインの責務。v4.0 は `stream.*` と `event.*`/`control` のみ登録；`pdu.*`, `tensor.*` は追加登録のみで Port を変えない。
- **Core or Module?:** registry 機構 = Kernel；具体 contract 定義 = Vocabulary。
- **Evidence:** Vision §21, §48; CMA §21。GR4 は `PortType::STREAM / MESSAGE` と `PortDomain`（CPU/GPU/NET/FPGA）を port 属性として持ち、domain 変換は明示 block（Appendix B: Port.hpp）。
- **Confidence:** High

### Finding 10 — Core 型は schema-first（言語中立シリアライズ + version）でなければならない。ExperimentSpec / Manifest の versioning と migration policy が欠けている。v3 の `!COMPUTE` 圧力への「代替手段」も欠けている

- **Severity:** `P1`
- **Area:** Core / Frontends / WASM / Plugins / Reproducibility
- **Current design:** Vision §9 は Spec に式を持ち込むことを禁止し、§10 で「RT path は JSON を parse しない」と書く。§62/§64 で WASM・out-of-process plugin・Python・MCP が同じ domain model を使う。Spec / Manifest の `version` フィールドと migration 方針への言及はない。
- **Problem:** (1) Event / Action / TxBurst / Manifest は Python、WASM Reactor、out-of-process Peripheral plugin、Run record の 4 箇所で同じ意味を持つ必要がある。Rust 型だけで定義すると、WASM ABI（Finding 8）と plugin protocol（Finding 23）で別々のシリアライズが生え、v3 の「controller ごとに手書き binary protocol」（`0b00010000` 系の msgtype）を再演する。(2) v3 の設定形式は v1→v2→v3 と変わり、`app.d` に converter が積まれた。v4 の Spec も変わる。方針がないと「古い Spec が黙って別の意味で解釈される」事故が起きる。(3) v3 は禁止していなくても `CONSTANTS` / `!COMPUTE(...)` を JSON に足した。同じ圧力（パラメータ sweep を Spec 内で書きたい）は v4 にも来る。禁止条項だけでは防げない。
- **Failure scenario:** WASM Reactor が返す `TxBurst` の serialize 形と Python client の `TxBurst` が別実装になり、フィールド追加で片方が壊れる。第二：v4.3 の Spec に `coherent: true` が追加され、v4.1 の Spec を読んだ v4.3 runtime が既定値 false で黙って走る。
- **Why it matters:** シリアライズ形と version 方針は最初の Spec / Event 型を書く時点で決まる。
- **Can it be deferred?:** No（方針）。具体技術（CBOR / FlatBuffers / serde+JSON Schema）の選定は Vision の範囲外で良い。
- **Recommended direction:** invariant として「Kernel の公開型はすべて言語中立 schema を持ち、schema に version を持つ」。Spec / BindingProfile / Manifest は `version` 必須、Kernel は **N-1 までの migration か明示拒否**のどちらかを保証（黙って解釈しない）。**式の代替**：パラメータ化は Python 側の Spec builder（`ezsdr.spec.build(...)`）で行い、生成物の Spec は完全展開済み（Manifest には生成元 Python の hash も記録）。Spec 内テンプレート機能は「should explicitly reject」。
- **Core or Module?:** Kernel。
- **Evidence:** Vision §9, §10, §42, §44, §62; v3 `app.d convertSettingJSONFromV1ToV2 / V2ToV3`、`settingfile.d parseSettingFile`（`!COMPUTE` 評価器）、`tcp_iface.d ifaceVersion "3.0.11"` 完全一致チェック、各 controller の手書き msgtype。
- **Confidence:** High

### Finding 11 — `prepare` の戻り値としての EffectiveConfig / Coercion report が契約に含まれていない

- **Severity:** `P1`
- **Area:** Core lifecycle / Radio / Reproducibility
- **Current design:** Vision §50 は Manifest に「requested RF parameters / actually applied RF parameters」を記録すると書く。§36 は Provider が `Warning / Constraint` を出すと書く。しかし lifecycle（§11）の `prepare` が何を**返す**かは未定義。
- **Problem:** 「applied」は Provider が `prepare` 時に決める（UHD は set_rx_rate 後に `get_rx_rate()` を読み戻せと明記）。これが構造化された戻り値でないと、Manifest 記録は「Provider がログに書いたものを後で拾う」形になり、実験ロジック（Python / Reactor）が applied 値を参照できない。Mock の `x310-like` が同じ coercion 規則を持たなければ Finding 4 の false confidence になる。
- **Failure scenario:** Spec `sample_rate_hz: 19.5e6`。X310 は 20e6 に coerce。Python の OFDM demod は Spec の 19.5e6 で symbol 長を計算する。Manifest には両方あるが、実験は既に壊れている。
- **Why it matters:** `prepare` の戻り型は Module API の凍結対象。
- **Can it be deferred?:** No（戻り型）。
- **Recommended direction:** `prepare(fragment) -> PrepareReport { effective: EffectiveConfig, coercions: Vec<Coercion{key, requested, applied, reason}>, warnings, constraints_hit }`。Kernel は coercion があれば policy `{accept, warn, reject}`（Spec で指定、既定 warn）を適用し、Python / Reactor に `run.effective()` を提供する。Mock は profile ごとの coercion 規則を実装する。
- **Core or Module?:** 型と policy = Kernel；規則 = Provider。
- **Evidence:** Vision §36, §50, §52; UHD `set_rx_rate` doc（Appendix B）；v3 `multiusrp.cpp setupRxChannels` は「Actual RX Rate」を cout に出すだけ。
- **Confidence:** High

### Finding 12 — Lease の意味論：attached / detached を決めないと「client 切断で TX を止める」と「repeat を残して測定に行く」が衝突する

- **Severity:** `P1`
- **Area:** Core ownership / Python API
- **Current design:** Vision §53「TX must not continue indefinitely merely because a client disappears」。§5 に Lease。Easy API（§3）は `with ezsdr.connect() as sdr:` の中で `tx.repeat(x)`。
- **Problem:** v3 の CyclicTX は**意図的に** client 切断後も loop 送信を続ける（server が loop を所有；`tcp_iface.d` は切断で `continue Lconnect` するだけで controller には触れない）。「repeat を設定 → 切断 → スペアナで測る」は実際の教育用途である。Vision §53 は逆を言う。両方正しい。Lease に**モード**がないと、どちらかのユーザが Provider 固有 hack に逃げる。
- **Failure scenario:** `with` を抜けた瞬間に TX が止まる仕様にすると、学生は `while True: sleep()` で block する。止まらない仕様にすると AI agent の crash で TX が永久に残る（§53 の懸念）。
- **Why it matters:** `__exit__` の意味は Easy API の最初の実装で決まり、変えると利用者コードが壊れる。
- **Can it be deferred?:** No（API semantics）。
- **Recommended direction:** `Lease { mode: Attached | Detached{ttl: Duration, renewable} }`。既定 Attached（session 終了で全 TX 停止・資源解放）。Detached は明示指定 + TTL 必須 + Manifest に記録 + 再接続で adopt 可能。TTL 切れ・runtime abort では §53 の cleanup を実行。
- **Core or Module?:** Kernel。
- **Evidence:** Vision §3, §53; v3 `tcp_iface.d eventIOLoop`（切断時に controller を止めない）、`cyclictx.d`（loop は controller thread が所有）。
- **Confidence:** High

### Finding 13 — Placement / memory / transfer planning は v4.0 では「明示 + validate」に限定し、最適化器を作らない（planner pipeline の縮小）

- **Severity:** `P1`
- **Area:** Core planner / Scope
- **Current design:** Vision §10 の compile pipeline は「Placement / strategy selection → Time / schedule planning → Memory / transfer planning」を含み、§20 は placement が capability / timing / format / envelope / memory / limits から**選択される**と書く。§31「Core should model enough information to plan」。§63 は「Kubernetes-like cluster manager にならない」と宣言する。
- **Problem:** 「選択する」planner は最適化器である。placement 空間（CPU/WASM/GPU/RFNoC × memory domain × transfer path）で最適解を出す Core は、Vision が否定する generic scheduler そのものになる。GR4 も NVIDIA Aerial も placement は**明示**（GR4 は `PortDomain` と明示変換 block、Aerial は手で組んだ slot pipeline）で、自動最適化はしていない（Appendix B）。
- **Failure scenario:** v4.1 で GPU OTFS を入れる際、planner に「copy cost model」「executor benchmark」「memory domain graph」を実装し始め、Core が 1 年育つ。実験者は結局 `placement: gpu0` と書きたいだけ。
- **Why it matters:** planner の責務範囲は Core サイズを直接決める。
- **Can it be deferred?:** 最適化器を**作らない**判断は今。後で足すのは additive。
- **Recommended direction:** v4.0 pipeline：`schema validate → semantic validate → resolve bindings (explicit) → build ExecutionPlan (fragments + DataLinks + dependency DAG) → admission checks (type/rate/domain compatibility, envelope constraints, deadline feasibility) → prepare → arm`。placement は BindingProfile / Spec の `graph.placement` に**明示**、Core は不整合を**拒否**するだけ。自動 placement / graph fusion / transfer 最適化は「should explicitly reject（v4.x で必要性が実証されるまで）」。
- **Core or Module?:** validator = Kernel。最適化は存在させない。
- **Evidence:** Vision §10, §20, §31, §63; CMA §27–28。GR4 `PortDomain`（Appendix B）、Aerial cuPHY slot pipeline（Appendix B）。
- **Confidence:** High

### Finding 14 — RUN 中の構造的 graph mutation を明示的に禁止し、parameter update は宣言済 class 経由に限定する

- **Severity:** `P1`
- **Area:** Real-time / Core lifecycle
- **Current design:** Vision §27 は update class `{cold, block_boundary, atomic_realtime, hardware_timed}` と GraphEpoch を挙げる。Processor / Reactor の追加・削除・再接続を RUN 中に許すかは書かれていない。
- **Problem:** GNU Radio 3.x の `lock()/unlock()` は scheduler 停止 → 全 graph 再 flatten → 再起動で、work thread から呼ぶと deadlock、buffer 長は変えられない（Appendix B: top_block_impl.cc）。これは flowgraph mutation の代表的な失敗である。Vision が沈黙していると、Reactor 用途（「復調器を差し替えたい」）から構造 mutation の要求が来て、Core に GR 式 lock/unlock が生える。
- **Failure scenario:** adaptive modulation で MCS ごとに別 Processor を差し替える設計にし、RUN 中の再配線を Core に実装 → RT path に世代管理と barrier が入り、deadline 保証が崩れる。
- **Why it matters:** 「禁止」は今書けば無料。後で禁止すると利用者コードが壊れる。
- **Can it be deferred?:** No（禁止の宣言）。GraphEpoch（prepared 次世代への atomic switch）は deferrable。
- **Recommended direction:** invariant：「RUN 中の graph 構造変更は禁止。変更は stop → re-plan → start、または将来の GraphEpoch switch」。parameter は `ComponentDescriptor.params` に宣言された class のみ変更可。MCS 切替は Processor の**パラメータ**（内部で複数モードを持つ）として設計する。v3 の `applyToDeviceSync`（関連 thread を全部 pause して変更）は `cold` class の実例。
- **Core or Module?:** 規則 = Kernel；class の実装 = Executor / Provider。
- **Evidence:** Vision §27; GR3 `top_block::lock/unlock` 制約（Appendix B）；GR4 は settings を work 境界で適用（`applyStagedParameters`）— block_boundary class と同じ（Appendix B: Settings.hpp）；v3 `controller/package.d applyToDeviceSync`。
- **Confidence:** High

### Finding 15 — Calibration：Core は型・validity・provenance だけを持ち、適用は Processor / Provider に置く。delay calibration は SISO でも必須。LO ランダム位相は Mock の coherence model に入れる

- **Severity:** `P1`
- **Area:** Calibration / Coherence / Simulation
- **Current design:** Vision §26 の CalibrationArtifact（kind / target / coefficients / method / validity / uncertainty / provenance）、§25 CoherentGroup の「phase calibration state」、§46 IBFD、§47 ISAC。「誰が適用するか」「calibration をどう生成するか」は未記述。
- **Problem:** (1) 適用点が未定義だと実験ごとに別の場所（Provider 内 / Processor / 後処理）で適用され、Manifest の「used calibration」が意味を持たない。(2) calibration 手順自体は実験（既知信号 TX → RX → 推定）であり、Core に専用機構は不要。(3) TX/RX の固定遅延は SISO の timing 主張にも必要（srsRAN は `time_alignment_calibration` をサンプル単位で持ち、OAI は `tx_sample_advance` を持つ — Appendix B）。(4) UBX/CBX/SBX は retune ごとにフロントエンド間位相がランダムで、timed tune（`set_command_time`）でのみ一定に保てる（Appendix B: page_sync）。Mock が「位相 0」のまま coherent MIMO を成功させると実機で崩れる。
- **Failure scenario:** 4 ch coherent 検出器を Mock で検証。実機で retune 後に位相がランダム化、per-Run calibration をしていないので検出器が壊れる。Manifest は「calibration: none」でも Run は成功扱い。
- **Why it matters:** CalibrationArtifact の型と「適用点は Processor/Provider」の規則は Vocabulary の初期形。Mock の coherence model は Radio Model の fidelity vector（Finding 4）に入る。
- **Can it be deferred?:** 型と規則は No。calibration 手順ライブラリは Yes。
- **Recommended direction:** `CalibrationArtifact` は Artifact の一種（Vocabulary）。適用は `CalibrationRef` を受け取る Processor（複素ゲイン / 遅延補正）または Provider（timed tune、FPGA 遅延）で、Kernel は validity 条件（freq / rate / gain / temperature / age / hw identity）の**検査と使用記録**のみ行う。calibration 手順は ExperimentSpec の一種（`outputs: CalibrationArtifact`）。Radio Model に `phase_behavior_on_retune: {deterministic, random_unless_timed_tune}` を持たせ、Mock はこれを模倣する。delay calibration（TX↔RX サンプルオフセット）は Provider profile の既定値 + Run ごとの上書き。
- **Core or Module?:** validity 検査・使用記録 = Kernel；型 = Vocabulary；適用・推定 = Module。
- **Evidence:** Vision §25, §26, §46, §47; UHD page_sync / page_dboards（timed tune、TwinRX LO sharing）；srsRAN `time_alignment_calibration`；OAI `tx_sample_advance`；Agora の reciprocity calibration は array 外の同期参照ノードを必要とする（Appendix B）。
- **Confidence:** High

### Finding 16 — Event pipeline の storm semantics と Policy の閉集合性が未定義

- **Severity:** `P1`
- **Area:** Observability / Core policy
- **Current design:** Vision §29「bounded event path、storm と queue loss 自体が observable」。§5 に Policy。§53 に失敗時 policy の項目列。
- **Problem:** (1) 「observable」の機構が未定。overflow が 1 秒に 1 万回起きると event queue（UHD 自身の async queue も深さ 1000 — Appendix B）は溢れる。溢れた後に何が残るか（件数か、最初の N か、最後の N か）が決まっていないと Manifest の「overflow 回数」が信用できない。(2) Policy が「rules engine」に育つ危険（Spec 内に条件式を書きたくなる → Finding 10 と同じ圧力）。
- **Failure scenario:** 10 分の Run で overflow 3 万回。event queue は 4096。Manifest は「4096 events」と記録し、論文は「overflow は稀」と書く。
- **Why it matters:** Event envelope と counter の型は Kernel 凍結対象。
- **Can it be deferred?:** No（semantics）。
- **Recommended direction:** Event envelope = `{source: ResourceId, domain+time: TimePoint, severity, kind, payload(schema)}`。各 source × kind に **never-dropping counter**（atomic）を持ち、event 本体は bounded queue で sampled。drop 発生時は `EVENTS_DROPPED{kind, count}` meta-event を必ず 1 件出す。Manifest には counter 最終値と sampled events の両方。Policy は**閉集合**：`on(kind) -> {continue, mark_artifact, stop, abort}` の宣言表であり、式言語を持たない。
- **Core or Module?:** Kernel。
- **Evidence:** Vision §29, §53; CMA §36; UHD `ASYNC_MSG_QUEUE_SIZE 1000`、`O/D/U` の console print を UHD 文書自身が「generally harmless」と書く（publication-grade では無害でない — Appendix B）；v3 `uhd_rfnoc.cpp` の async metadata は cout のみ。
- **Confidence:** High

### Finding 17 — AI agent が実機へ昇格する前の「RF safety envelope」が BindingProfile / validate に存在しない

- **Severity:** `P1`
- **Area:** Policy / Safety / Binding
- **Current design:** Vision §43–§44 は AI agent が Spec を生成し hardware を launch できると書く。§52「failure should happen before RF transmission whenever possible」。§62 はセキュリティ境界を挙げるが RF 出力の制限（帯域・出力・アンテナ端子）には触れない。
- **Problem:** ExperimentSpec を AI が生成する世界では、`frequency: 1.575e9, gain: max` が validate を通る。研究室の免許・実験局条件は BindingProfile（環境）側の制約であり、Spec（意図）とは独立に強制されるべきである。
- **Failure scenario:** agent が探索的にキャリア周波数を sweep し、許可帯域外で送信する。Mock では何も起きない。
- **Why it matters:** BindingProfile のスキーマに envelope を持つかは初期設計。additive だが、無いまま実機に agent を触らせる期間を作るべきでない。
- **Can it be deferred?:** 実機 Provider 実装（Phase 7）までは Yes。BindingProfile スキーマに枠を作るのは今。
- **Recommended direction:** BindingProfile に `rf_envelope: {allowed_bands, max_gain_db or max_power_dbm per channel, tx_enabled, antenna_ports}` を持たせ、validate() と prepare 時に Kernel の policy として強制。Provider は applied 値をこの envelope に対して再検査する（coercion で帯域外に出る場合も拒否）。
- **Core or Module?:** 強制 = Kernel policy；スキーマ = Radio Model。
- **Evidence:** Vision §43, §44, §52, §62; 一般知識（INFERRED）。
- **Confidence:** Medium（重要性は High、Vision の意図との整合は確認できないため）

### Finding 18 — 決定論的 simulation には「stepped execution mode」が必要。RealtimeEmulation だけが実スレッドを使う

- **Severity:** `P1`
- **Area:** Simulation / Processing executor
- **Current design:** Vision §58 #3「seed で再現」、§15「functional simulation は wall clock より速く」、§32 Island は CPU affinity / RT policy を持つ。
- **Problem:** Island ごとに実スレッドが走り、Probe link が drop policy を持ち、Reactor が event 到着順で状態遷移するなら、実行は本質的に非決定的である。seed 固定だけでは再現しない。決定論は「同じ virtual time 順序で同じ event を配る」ことで得られ、それには executor が **DES kernel に step 駆動される mode** を持つ必要がある。
- **Failure scenario:** CI で PING→PONG テストが 3% flaky。原因は Probe drop と Reactor timer の順序。開発者は sleep を足す。
- **Why it matters:** native executor の設計（自走スレッド前提か、step 駆動可能か）は Phase 2–5 で決まる。
- **Can it be deferred?:** No（executor の駆動モデル）。
- **Recommended direction:** ExecutionClass ごとに駆動モデルを規定：`Simulation` = DES kernel が全 Island を step 駆動（単一論理スレッド、または決定的 barrier）、drop policy は決定的（virtual time ベース）；`RealtimeEmulation / HIL / Hardware` = 実スレッド。Executor 契約に `step(until: TimePoint)` を必須化する。
- **Core or Module?:** 要件 = Kernel（Executor 契約）；実装 = Module。
- **Evidence:** Vision §15, §32, §58; GR4 `externalStep` execution policy が同種の概念（Appendix B: Scheduler.hpp）；OAI rfsimulator は lockstep で決定性を得ている（Appendix B）。
- **Confidence:** High

### Finding 19 — Manifest は namespaced sections と content hash を前提に設計する（過剰化の防止と拡張の両立）

- **Severity:** `P1`
- **Area:** Reproducibility / Core
- **Current design:** Vision §50 は約 25 項目を列挙し「大きな artifact は参照で」と書く。CMA §38 も同様。構造（誰がどの項目を書くか）は未定義。
- **Problem:** UHD version / FPGA image / daughterboard は UHD Provider しか知らない。Core が項目を enum で持つと Provider ごとに Core が育つ。逆に自由形式だと比較不能。
- **Failure scenario:** Soapy Provider 追加時に Manifest に `soapy_driver_version` を足すため Core 変更。
- **Why it matters:** Manifest の envelope は Kernel 凍結対象。
- **Can it be deferred?:** No（envelope 構造）。項目の充実は Yes。
- **Recommended direction:** Manifest = **Kernel envelope**（run id, execution class, fidelity vector, spec hash + 本体, binding hash + 本体, plan summary, module identities + versions, epoch↔UTC relation, event counters + sampled events, artifact refs with hashes, lease/termination reason）+ **namespaced sections**（`uhd.*`, `mock.*`, `host.*`, `calibration.*`）を各 Module が書く + **optional environment capture**（CPU/NIC topology 等、profile で on/off）。すべて content-addressed（spec / waveform / component / calibration の hash）。
- **Core or Module?:** envelope = Kernel；sections = Module。
- **Evidence:** Vision §50–§51; CMA §38; SigMF の `core:sha512` / extensions namespace 方式が同型（Appendix B）。
- **Confidence:** High

### Finding 20 — RFNoC は「Processor の placement 先」ではなく「Radio Provider 内部の capability」として位置付ける

- **Severity:** `P1`
- **Area:** Processing / Radio / Scope
- **Current design:** Vision §20 は processing Provider に `RFNoC / FPGA` を含め、§32 に「RFNoC Island: DDC → FIR」、§67 Phase 11 に「RFNoC Processor placement」。§63 は「universal FPGA language にならない」。
- **Problem:** RFNoC block は device 内の**既存**ブロック（Radio / DDC / DUC / FFT / FIR / Replay / Custom）の接続と設定であり、任意の user Processor を置くには FPGA image を作る必要がある（Vision 自身が対象外とする）。よって「同じ Processor model を RFNoC に placement」は成立せず、成立するのは「repeat を Replay で」「rate 変換を DDC/DUC で」「FFT を FFT block で」という **Radio capability の実装選択**である。v3 の `UHD_RFNoC` device と `USRP_TX_LoopDRAM` はまさにこれで、`connections` / `tx-streamers.type: replay` という**device 設定**として表現されていた（`v3/changelog/v3.0.17.md`）。
- **Failure scenario:** Processor model を CPU/WASM/GPU/RFNoC の最小公倍数で設計し（Vision §20 が禁じる LCD）、結局 RFNoC executor は「DDC と FIR だけ」の特殊ケースになる。
- **Why it matters:** Processing Module の設計対象から RFNoC を外すと、Processor 契約が単純になり（CPU/WASM/GPU の software component だけ）、Radio Model に `tx.repeat` の実装選択（host loop / device DRAM）が入る。これは Mock↔Hardware の同値性にも関わる：Replay は word/item alignment と DRAM サイズの制約を持ち、host loop は wrap 時の underflow リスクを持つ。
- **Can it be deferred?:** 位置付けの決定は No。RFNoC 実装は Yes。
- **Recommended direction:** Radio Model に `tx.repeat{max_len, alignment}`, `rx.decimation`, `fft` 等の**capability**を持たせ、UHD Provider が RFNoC graph 構築で実装する。Custom RFNoC block は `extensions.uhd.rfnoc.*` の expert 拡張。Vision §20/§32/§67 の「RFNoC Processor placement」を「RFNoC-backed radio capability」に書き換える。
- **Core or Module?:** Radio Model（Vocabulary）+ UHD Provider。Core 変更なし。
- **Evidence:** Vision §20, §32, §63, §67; UHD Replay `record/play` の alignment 制約と `get_mem_size`（Appendix B）；v3 `uhd_rfnoc.cpp`（`connections`, replay streamer, `mem_size / numUsedChannels` 分割）、`looptx_rfnoc_replay_block.cpp`。
- **Confidence:** High

### Finding 21 — IBFD に必要な 2 点：TX 側の timed tap と SimulationChannel の self-coupling が Vision に明示されていない

- **Severity:** `P1`
- **Area:** IBFD / Simulation / Data model
- **Current design:** Vision §46 は IBFD を「special-case Core API なしで」支えると書き、§16 は SimulationChannel に self-interference を列挙、CMA §15 に IBFD 図がある。
- **Problem:** digital SIC は **実際に送信された TX サンプルと device time で整列した参照**を必要とする。実機では TX block の target TimePoint + 較正済 TX→RX 遅延（Finding 15）で整列できるが、これは「TX 側 SampleBlock が時刻を持ち、Probe/Tap で分岐できる」ことが前提。連続 TX（burst でない）でも block ごとの時刻が必要。また SimulationChannel は「N TX → M RX の全結合（同一 device 含む）」を扱う topology でなければ SI path を模倣できない。
- **Failure scenario:** TX 経路の block に時刻がなく、SIC Processor が「TX キューに入れた順」で整列を試み、underflow 一回でずれる。
- **Why it matters:** TX block の時刻フィールドは Stream Contract（Finding 3）の一部。
- **Can it be deferred?:** No（Stream Contract に含める）。SI model は Yes。
- **Recommended direction:** Stream Contract に「TX block は先頭 target TimePoint を持つ（連続でも）」を含め、Tap は TX edge にも置ける。SimulationChannel は `H: [tx_ports] × [rx_ports]` の結合行列（同 device 含む）+ per-path model（PA 非線形、遅延）を持つ。analog canceller は Peripheral（bounded_latency）。
- **Core or Module?:** Stream Contract = Kernel；channel topology = Simulation Module。
- **Evidence:** Vision §16, §30, §46; CMA §15; FlexICoN/COSMOS：canceller は SPI over USB adapter で設定され、digital SIC タップは GNU Radio 側で計算（Appendix B）。
- **Confidence:** High

### Finding 22 — Module / Provider / Plugin は 3 つの直交軸として定義し直す（Executor を追加）

- **Severity:** `P1`（コストは低いが module-api の命名・構造に直結）
- **Area:** Terminology / Module API
- **Current design:** Vision §7「Module = deployable unit、Provider = capability を提供する Module、Plugin = 動的発見/別配備の Module」。CMA §19 は処理 engine も「Provider」と呼ぶ。
- **Problem:** 「Provider」が Resource 実装（Radio）と Execution 実装（native executor）の両方に使われ、「Plugin」は配備形態なのに Module の種類のように読める。3 語が同じ軸上で重なっている。
- **Failure scenario:** module-api に `trait Provider` が 1 つ生え、Radio と Executor の lifecycle が無理に統一される。
- **Why it matters:** module-api crate の型名。
- **Can it be deferred?:** No（命名）。
- **Recommended direction:** **Module** = 配備・version 単位（crate / process）。**役割**（Module が実装する契約）：**Provider**（Resource Model の実装：Radio / Peripheral / Endpoint / Simulation）、**Executor**（Processing engine：native / WASM / GPU）、**Sink**（Artifact writer）。**配備形態**：in-process（Rust trait）/ **Plugin**（out-of-process、typed protocol）。1 Module は複数役割を持てる（UHD Module = Radio Provider + GPIO Provider + Timekeeper）。
- **Core or Module?:** Kernel（module-api）。
- **Evidence:** Vision §7; CMA §19.
- **Confidence:** High

### Finding 23 — 環境（SimulationChannel / fault schedule / 実験室 RF 環境）の記述場所が Spec と BindingProfile の間で未定義

- **Severity:** `P1`
- **Area:** ExperimentSpec / BindingProfile / Simulation
- **Current design:** Vision §8 は Spec = intent、BindingProfile = 実装選択。§16 SimulationChannel は Module。§17 fault injection は「software execution が注入する」。どの文書に channel model / fault schedule を書くかは未記述。
- **Problem:** Stress Test B（MockRadio A → channel → MockRadio B）で channel model を **ExperimentSpec に書くと、その Spec は実機に昇格できない**（実機に channel 設定は存在しない）。逆に fault schedule を BindingProfile に書くと、「同じ Spec を Mock と実機で走らせ、Mock 側だけ fault を注入する」ができて正しい。ただし channel emulator（実機の Peripheral）を使う実験では channel が**資源**になる。この境界を決めないと Spec の可搬性が最初のテストで崩れる。
- **Failure scenario:** 受け入れテスト #8（2 台の Mock が SimulationChannel で通信）を Spec に `channel: {awgn_snr: 10}` と書いて通す。Phase 8 の X310 parity test で Spec が validate を通らない。
- **Why it matters:** Spec / BindingProfile のスキーマ境界は最初のマイルストーンで固定される。
- **Can it be deferred?:** No.
- **Recommended direction:** BindingProfile を `bindings`（資源 → Provider）+ `environment`（Simulation: channel model / fault schedule / virtual clock 設定；Hardware: rf_envelope / 配線メモ / clock 配布）の 2 部構成にする。Spec は環境を仮定しない。channel emulator や attenuator を**実験が制御する**場合のみ Spec の resources（Peripheral）に現れる。Manifest は environment 部を必ず記録。
- **Core or Module?:** Kernel（BindingProfile スキーマ）。
- **Evidence:** Vision §8, §16, §17, §58 (#8); CMA §8, §14。
- **Confidence:** High

---

## 5. P2 / P3 Findings（将来拡張点の確保で足りる、または対応不要）

### Finding 24 — Module カテゴリごとの process 境界（in-process trait / out-of-process protocol）と UHD crash isolation

- **Severity:** `P2`
- **Area:** Module API / Security / Robustness
- **Current design:** Vision §62「third-party vendor SDK は Core process 外」「TUN/TAP 権限は分離」「WASM が trust boundary」。§35 UHD は「narrow native C++ bridge」。§64 に「runtime-downloadable third-party backend ABI」は future。
- **Problem:** Rust は安定 ABI を持たず、動的ロードは `#[repr(C)]` + `abi_stable` 等か process 分離になる（Appendix B）。UHD は例外・abort を起こし得る（v3 は `--retry` で server 全体を再起動する運用だった）。in-process なら UHD crash = runtime crash。out-of-process なら data plane が IPC を跨ぐ（200 Msps では shm 必須）。
- **Failure scenario:** 後から UHD provider を別 process に出そうとしたとき、Radio contract が `&[T]` を前提にしていて shm 化できない。
- **Why it matters / Can it be deferred?:** Finding 3 の handle-based buffer が守られていれば `RemoteRadioProvider`（同じ trait を IPC で実装）は additive。よって **P2**。
- **Recommended direction:** v4.0：Radio / Executor は in-process trait、Peripheral plugin と privileged host-I/O helper は out-of-process typed protocol。DataLink は process 境界を shm/DMA 可能な実装でのみ跨ぐ。UHD bridge は例外を status に変換し、abort は `DEVICE_LOST` event + Run abort policy に写す（v3 の `--retry` を Kernel policy に昇格）。
- **Core or Module?:** 方針 = Kernel 文書；実装 = Module。
- **Evidence:** Vision §35, §62, §64; v3 `app.d`（`flagRetry`, `RestartWithConfigData`）；Rust ABI（Appendix B）。
- **Confidence:** High

### Finding 25 — 分散実行のために今やるべきは「node-qualified identifier」だけ

- **Severity:** `P2`
- **Area:** Distributed
- **Current design:** Vision §49 / §64：v4.0 は local 解決、ComputeNode / NetworkLink / Placement は future だが「public model を置き換えずに追加可能であること」。
- **Problem:** `ResourceId / ClockDomainId / MemoryDomainId / IslandId` が host-local 整数だと multi-host で衝突する。
- **Recommended direction:** ID をすべて `{node: NodeId, local: ...}` に、v4.0 は `node = local` 固定。ExecutionIsland は既に node を含む。NetworkLink は DataLink の一種として後から登録。**ComputeNode / Placement の最適化は作らない**（Finding 13）。
- **Core or Module?:** Kernel（ID 型）。
- **Evidence:** Vision §49, §64, §65 (29)。Techtile（PTP）/ White Rabbit（Bigler 2018: coherent DL beamforming には数十 ps 級）— 分散 coherence はソフト側で保証できず、ClockRelation の uncertainty として扱う（Appendix B）。
- **Confidence:** High

### Finding 26 — TUN/TAP の privileged helper は「常駐 proxy」ではなく「one-shot setup」で足りる

- **Severity:** `P2`
- **Area:** Host-I/O / Security
- **Current design:** Vision §41 / CMA §24「Core → typed IPC → helper(CAP_NET_ADMIN) → /dev/net/tun」。
- **Problem:** 常駐 helper が全 packet を IPC で中継すると copy と遅延が増える。Linux では **device の作成には CAP_NET_ADMIN が必要だが、自分が所有する既存 device への接続は不要**（kernel doc）。multiqueue（`IFF_MULTI_QUEUE`）は 3.8 以降でサポート。
- **Recommended direction:** helper は「persistent TAP を作り owner/group を設定し up/IP 付与して終了」する one-shot。runtime は非特権で `/dev/net/tun` を開いて `TUNSETIFF` する。packet drop は kernel 統計から `TUN_QUEUE_DROP` を導出。timestamp は host clock（ClockRelation で device time に写す）。
- **Core or Module?:** Host-I/O Module。Core 変更なし。
- **Evidence:** Vision §41; CMA §24; Linux tuntap doc（Appendix B）。
- **Confidence:** High（権限）／ Medium（persistent device の非特権 open は kernel doc に TUNSETPERSIST/TUNSETOWNER の記述がなく、一般知識として INFERRED）

### Finding 27 — Probe / Tap は新しい Core 概念にしない（DataLink の lossy policy + Recorder Module）

- **Severity:** `P2`
- **Area:** Instrumentation
- **Current design:** Vision §30 は Probe を「概念」として提案し、制御項目（sample every N、decimate、max rate、drop if busy）を挙げる。
- **Judgment:** **This should NOT be added to Core as a distinct concept.** Finding 3 の fan-out（参照共有）と DataLink の `drop_*` policy があれば、Probe は「lossy DataLink + 標準 Recorder Sink」で表現でき、CFO 推定値等の中間値は Processor の追加出力 port（`event.*` / `tensor.*` contract）で出せる。「sample every N」「decimate」は Recorder のパラメータ。
- **Recommended direction:** Vision §30 を「Probe は DataLink policy と Recorder の組合せである」と書き換える。Core 概念数を 1 減らす。
- **Evidence:** Vision §30; GR4 も probe は通常 block（Appendix B）。
- **Confidence:** High

### Finding 28 — Taint の伝播は convention であり、Core が強制できるものではない

- **Severity:** `P2`
- **Area:** Data validity
- **Current design:** Vision §28 に `Taint`。
- **Judgment:** Processor が入力 gap を出力にどう反映するかは算法依存（FFT 1 ブロックは全体汚染、FIR は tail 長だけ）。Kernel は block flags（Finding 3）を定義し、**Executor の既定動作を「1:1 写像なら flags を伝播」**とする。それ以上は Processor 作者の責務として文書化。`Taint` を Core 型にしない。
- **Evidence:** Vision §28。
- **Confidence:** High

### Finding 29 — サンプル形式のネゴシエーションと Convert Processor の自動挿入は後回し（明示一致を要求）

- **Severity:** `P2`
- **Area:** Data plane / Planner
- **Current design:** Vision §20 placement は sample format を考慮、§21 `SampleStream<T>`。
- **Problem:** v3 は `srvfmt/devfmt`（fc32/sc16/sc8）を streamer ごとに持ち、controller 型が sample 型でテンプレート化されていた。形式不一致は setup で `enforce`。v4 で自動変換挿入を planner に入れると Finding 13 の最適化器化の入口になる。
- **Recommended direction:** v4.0 は DataContract identity の**明示一致**を要求し、不一致は validate で拒否。標準 `Convert` Processor をユーザが明示配置。自動挿入は後日。wire format（sc16/sc8）は Provider 内部の PerformanceEnvelope 次元。
- **Evidence:** v3 `ezsdr_enums.hpp`, `multiusrp.cpp getTxStreamer`（srvfmt/devfmt）、`controller/package.d newController`。
- **Confidence:** High

### Finding 30 — ISAC の外部センサ（camera / MOCAP）データは Ez-SDR に取り込まず、時刻相関付きの参照だけを持つ

- **Severity:** `P2`
- **Area:** ISAC / Scope
- **Current design:** Vision §47「同じ Run で IQ time、peripheral state、external sensor time を相関」。
- **Judgment:** Ez-SDR は sensor-data platform にならない。camera / MOCAP は (a) trigger を受ける Peripheral（`trigger_input` capability）か、(b) 自分の clock で timestamp した `frame_captured` Event + 外部ファイル参照を出す Peripheral として現れる。動画本体は Artifact **参照**（hash + path）。相関は ClockRelation（Finding 2）と後処理。実際の X410 ISAC 研究も開始時刻オフセットの後処理で MOCAP を相関している（Appendix B）。
- **Recommended direction:** Vision §47 に「external sensor data は by reference」を追記。Core 変更なし。
- **Confidence:** High

### Finding 31 — Peripheral の timing class は Provider の**種類**ではなく**インスタンス / device** ごとに宣言する

- **Severity:** `P2`
- **Area:** Peripheral / Radio GPIO
- **Current design:** Vision §38 は timing class を Peripheral の性質として挙げ、§39 は「UHD GPIO Provider」を 1 つの Provider として描く。
- **Problem:** 同じ UHD GPIO でも X3x0 は radio の timed interface 経由で hardware_timed（source から確認、文書化は INFERRED）、X4x0 は ATR 以外は untimed（INFERRED）。class を Provider 種で固定すると誤る。
- **Recommended direction:** timing class は `prepare` 時に Provider instance が capability として返す。validate は instance の宣言を使う。Mock Peripheral も class を宣言し、best_effort なら遅延分布を模倣する。
- **Evidence:** UHD GPIO API pages / x300 / x400 source（Appendix B）。
- **Confidence:** Medium

### Finding 32 — Module taxonomy：「Distributed」は module カテゴリではない。ClockSource は Peripheral の一種。Artifact は Sink 役割

- **Severity:** `P3`
- **Area:** Taxonomy
- **Judgment:** §60 の `modules/distributed/future` は配備トポロジであり Module 種別ではない（NetworkLink は DataLink Provider）。OctoClock / GPSDO / PTP grandmaster は「ClockSource Peripheral」（lock 状態 event、ClockRelation 証拠を出す）として既存カテゴリに収まる。`artifact/` は Sink 役割の Module 群。§7 で詳述。
- **Confidence:** High

### Finding 33 — Metrics framework は Core に入れない

- **Severity:** `P3`
- **Area:** Observability
- **Judgment:** queue occupancy / deadline miss 率 / overflow 数は Finding 16 の counter と sampled event で表現できる。Prometheus 的な metrics registry を Kernel に置く必要はない（export は Sink Module）。**This should NOT be added to Core.**
- **Confidence:** High

### Finding 34 — v3 wire 互換は不要だが、v3 の「挙動」互換テスト項目は今のうちに抽出する

- **Severity:** `P3`
- **Area:** Migration
- **Judgment:** Vision §1 は v3 を behavioral requirement の源とする。抽出すべき挙動：(1) repeat TX の連続性（wrap で underflow しない）、(2) `alignSize` が実現していた「capture 先頭のサンプル位置を決定的にする」要求 → v4 では `capture(n, at: TimePoint | sample_index)` で置換（alignment 倍数という quirk を Python API に再出させない）、(3) timed start（`onTime`）、(4) 複数筐体の PPS 同期起動。wire protocol / msgtype 互換は不要。
- **Evidence:** v3 `cyclicrx.d`（alignSize）、`test_continuous_recv.py`（`nSamples-1` alignment で 1 サンプル drift を手動検出 — runtime が continuity を提供しなかった証拠）。
- **Confidence:** High

---

## 6. Core Boundary Audit

### 6.1 判定表（Vision が列挙する概念 → 置き場所）

| 概念 | Vision の位置 | 監査判定 | 理由 |
|---|---|---|---|
| Run / Session / lifecycle transaction | Core | **Kernel** | 唯一の状態機械。Session は Run 種別（F5） |
| Lease | Core | **Kernel** | attached/detached（F12） |
| ExperimentSpec / BindingProfile / ExecutionPlan envelope | Core | **Kernel**（中身は namespaced） | version + migration（F10）、environment 部（F23） |
| Manifest / Artifact reference | Core | **Kernel envelope** + Module sections | F19 |
| ClockDomain / TimePoint / Duration / Deadline / ClockRelation | Core | **Kernel** | 具体表現を確定（F2） |
| TimeAuthority interface | （なし） | **Kernel**（追加） | F2 |
| SampleBlock / BufferRef / MemoryDomain / Stream Contract | Core（概念のみ） | **Kernel** | F3 |
| DataContract registry / Port | Core | **Kernel**（機構）/ Vocabulary（定義） | F9 |
| Event envelope / counters / Policy（閉集合） | Core | **Kernel** | F16 |
| Action 語彙（TxBurst, SetTimer, UpdateParameter, PeripheralCommand, Emit, Stop, Abort）+ late policy | Core | **Kernel** | F4, F8 |
| Capability / Constraint matching | Core | **Kernel**（generic） | F7 |
| Resource composite tree / dependency DAG | （フラット） | **Kernel**（追加） | F6 |
| Module API（descriptor, registry, roles） | Core | **Kernel** | F22 |
| ExecutionIsland（宣言・admission） | Core | **Kernel**（宣言のみ）/ Executor（scheduling） | F8, F13 |
| Radio Model（channels, RF params, TimingEnvelope, PerformanceEnvelope, coherence basis, repeat/decimation capability） | Module（radio-model） | **Vocabulary crate** | F1, F4, F20 |
| CoherentGroup / DeviceGroup / Array | Core | **Vocabulary**（Provider 所有データ） | F6 |
| CalibrationArtifact | Core | **Vocabulary**（Artifact 種別）；適用は Module | F15 |
| Peripheral model / timing class | Core | **Vocabulary**；class は instance 宣言 | F31 |
| NetworkEndpoint / HostEndpoint model | Core | **Vocabulary** | — |
| Processor / Reactor Descriptor | Core | **Kernel**（descriptor）；ABI は Executor | F8 |
| update class（cold / block_boundary / atomic_realtime / hardware_timed） | Core | **Kernel**（列挙）；実装は Module | F14 |
| GraphEpoch | Core（where useful） | **Defer**（構造 mutation 禁止を先に） | F14 |
| ContinuityMap / ValidityMap / Gap | Core | **Kernel**：block flags；Artifact map は導出 | F3 |
| Taint | Core | **Convention**（Core 型にしない） | F28 |
| Probe / Tap | Core | **Reject as concept**（DataLink policy + Recorder） | F27 |
| DataLink（実装）/ BufferContract / TransferRequirement | Core | DataLink identity = Kernel；実装 = Module；Transfer 最適化 = **Reject** | F13 |
| Placement（自動） | Core | **Reject**（明示 + validate のみ） | F13 |
| PerformanceEnvelope | Provider | **Vocabulary** | §34 維持 |
| Profiles（auto/balanced/throughput/latency） | Runtime | **Module / BindingProfile hint** | Core 不要 |
| ComputeNode / NetworkLink | future | node-qualified ID のみ **Kernel**；他は future | F25 |
| Metric | Core | **Reject as framework**（counter + event） | F33 |
| SimulationFidelity（enum） | Core | **Kernel**（fidelity vector に置換） | F4 |
| RF safety envelope | （なし） | **Kernel policy**（追加）/ Radio Model schema | F17 |

### 6.2 Core（Kernel）に入れてはいけないもの

Vision §6 / CMA §5 の列挙（UHD API、CUDA、/dev/net/tun、802.11 アルゴリズム等）は正しい。本監査で**追加**する「入れてはいけないもの」：

1. **Radio 固有 capability キー**（`rx_channels`, `sample_rate_hz` 等）— Vocabulary へ（F7）
2. **Processor の実行 ABI（work 関数の呼び出し規約）**— Executor へ（F8）
3. **自動 placement / transfer 最適化 / graph fusion**（F13）
4. **RUN 中の構造的 graph mutation 機構**（F14）
5. **Policy の式言語 / rules engine**（F16）
6. **Spec 内テンプレート・式**（F10；Python builder へ）
7. **Probe という独立概念**（F27）
8. **Taint の伝播規則**（F28）
9. **Metrics framework**（F33）
10. **Calibration の適用・推定アルゴリズム**（F15）
11. **SimulationChannel / fault model の中身**（Module）
12. **センサデータ（映像等）の取り込み**（F30）
13. **RFNoC graph 構築**（F20；UHD Provider 内）
14. **Provider 横断の coherence 推論**（F6）

---

## 7. Module Taxonomy Audit

### 7.1 現行分類の評価

| Vision の分類 | 判定 | 補足 |
|---|---|---|
| Radio | 維持 | Provider 役割。GPIO / timekeeper / DRAM を sub-resource として同一 Module が提供（F6）。RFNoC は内部実装（F20） |
| Processing | 維持（範囲縮小） | **Executor** 役割：native / WASM / GPU。RFNoC を除外（F20） |
| Host-I/O | 維持 | Endpoint Provider。helper は one-shot（F26） |
| Peripheral | 維持 | Provider 役割。out-of-process Plugin が既定形態。ClockSource もここ（F32） |
| Simulation | 維持（役割を明確化） | DES kernel（TimeAuthority 実装）+ Mock Providers + SimulationChannel + FaultInjector。「Mock は Radio カテゴリの Provider でもある」ため、分類は**役割**で読む |
| Artifact | 名称変更 | **Sink** 役割（raw-iq / SigMF / HDF5 writer） |
| Distributed | **削除** | 配備トポロジ。NetworkLink は DataLink Provider（F32） |
| （欠落）DataLink | **追加** | SPSC ring / shm / pinned copy / RDMA など。Kernel は identity のみ |

### 7.2 用語の整理（F22）

```text
軸 1：単位      Module（crate / process。version の単位）
軸 2：役割      Provider（Resource Model 実装）| Executor（Processing engine）| Sink（Artifact writer）| Link（DataLink 実装）
軸 3：配備形態  in-process（Rust trait）| Plugin（out-of-process typed protocol）
```

1 つの Module は複数の役割を持てる（UHD Module = Radio Provider + GPIO Provider + Timekeeper）。「Plugin」を役割のように使わない。

---

## 8. Mock / Simulation Architecture Audit（最終判断）

| 論点 | 判断 | 根拠・条件 |
|---|---|---|
| **Mock と Hardware の同値性** | **論理的意味論（型・lifecycle・event・continuity・timed command の合否）については達成可能。RF 挙動については達成不能であり、主張してはならない** | 条件：Radio Model が TimingEnvelope / coercion / PerformanceEnvelope を必須化し、Mock がそれを**強制**する（F4）。Stream Contract が規範化される（F3）。OAI rfsimulator は同じ interface を持ちながら late/underflow/overflow を模倣せず false confidence を生んだ（Appendix B） |
| **Virtual Time** | **Simulation Environment は discrete-event simulation kernel として設計し、Kernel の TimeAuthority を実装する** | F2。Reactor timer、Processor deadline、Peripheral latency、Python の待ちがすべて virtual clock に従う。Python に device-time `sleep` を与えない |
| **Deterministic execution** | **functional Simulation は step 駆動（単一論理スレッド or 決定的 barrier）。RealtimeEmulation 以上のみ実スレッド** | F18。drop policy も virtual time で決定的に |
| **Fault injection** | **維持。ただし注入結果は「実機と同じ typed event + 同じ block flags + 同じ再開ギャップ」でなければならない** | F3, F4。UHD overflow = 0 サンプル返却 + 50 ms 後再開 + `out_of_sequence` 区別（Appendix B）。Mock の overflow injection はこれを再現する |
| **RF / channel modeling** | **SimulationChannel を MockRadio から分離する方針は正しい。N TX × M RX 全結合（同一 device 含む）を topology とする。初期は loopback / gain / delay / AWGN で十分** | F21。RF fidelity は fidelity vector で `none` と明記 |
| **Fidelity classification** | **単一 enum を per-aspect vector `{timing, continuity/faults, coercion, rf, transport}` に置換** | F4。昇格判定は aspect ごと |
| **HIL への昇格** | **成立する。ただし HIL = 「実機 timekeeper + 実 transport + 模擬 RF（ケーブル / attenuator）」と定義し、envelope は実機値を使う** | ExecutionClass の定義に「TimeAuthority が device か DES か」「RF が OTA か cable か simulated か」の 2 軸を含める |
| **AI-agent validation** | **成立する。条件：validate() が envelope を検査し（F4）、Session/Run の Manifest が常に生成され（F5）、RF safety envelope が実機昇格前に強制される（F17）、Spec が完全展開済みで hash 可能（F10）** | Aerial の RU emulator が O-RAN timing window を検証するのと同じ「制約を模倣する emulator」が必要（Appendix B） |
| **Mock の追加要件（Vision 未記載）** | block 長 jitter option、coercion grid、stop tail、retune 時ランダム位相 option、command queue depth、startup latency | F3, F4, F15 |
| **環境記述の場所** | channel / fault schedule は BindingProfile の `environment`。Spec には書かない | F23 |

---

## 9. Research Use-case Matrix

判定語：**Supported naturally**（Kernel + Radio Model のまま）／**Supported with planned extension**（Module / Vocabulary 追加で可、Core 変更なし）／**Architecture change required**（Vision の記述を変える必要あり）／**Not appropriate for Ez-SDR**。

| ユースケース | 判定 | 理由・必要な拡張 | 関連 Finding |
|---|---|---|---|
| SISO | Supported naturally | Session/Run + Radio Model + Stream Contract。delay calibration（TX↔RX オフセット）が Provider profile にあれば timing 主張も可 | F2, F3, F5, F15 |
| 802.11-like packet radio | Supported with planned extension | `pdu.*` DataContract、Reactor + TxBurst late policy、TAP Endpoint。**ただし SIFS 16 µs は host 生成 timed TX では不可**。validate() が envelope 違反を事前に報告し、「relaxed SIFS の 802.11-like」として成立する | F4, F8, F9 |
| MIMO（単一筐体 2×2） | Supported naturally | 多チャネル aligned stream は Provider 内。per-channel validity | F3, F6 |
| coherent multi-USRP | Supported with planned extension | 単一 provider instance（`addr0,addr1`）+ provider-declared coherence + arm ordering DAG + timed tune。Mock はランダム位相 option | F6, F15 |
| massive MIMO（数十 ch、単一 host、GPU） | Supported with planned extension | Tensor contract + GPU Executor + 明示 placement + pinned DataLink。性能は PerformanceEnvelope で admission | F9, F13 |
| massive MIMO scale-out（多 host） | Supported with planned extension（future） | node-qualified ID + NetworkLink DataLink + ClockRelation。最適化器は作らない | F25, F13 |
| cell-free MIMO | Supported with planned extension | サイト間は ClockRelation + CalibrationArtifact（OTA sync は研究アルゴリズム = Processor）。Core は coherence を推論しない | F6, F15, F25 |
| IBFD | Supported with planned extension | TX timed tap、SimulationChannel self-coupling、Peripheral bounded-latency loop、block_boundary 係数更新 | F21, F14, F31 |
| ISAC | Supported with planned extension | 多チャネル + Peripheral（hardware_timed trigger / best_effort positioner）+ Tensor + 外部センサは参照のみ | F2, F30, F31 |
| OTFS | Supported with planned extension | `tensor.*` contract + GPU/CPU Executor。Core 変更なし | F9 |
| smart antenna | Supported with planned extension | Peripheral Plugin、GPIO sub-resource または USB、timing class は instance 宣言 | F6, F31 |
| TUN/TAP networking | Supported with planned extension | Endpoint Provider（one-shot 特権 setup、multiqueue）、`pdu.ethernet` contract、drop 統計 → event | F26, F9 |
| GPU PHY | Supported with planned extension | GPU Executor（batch）、MemoryDomain、明示 placement。Aerial 同様に手で組む | F8, F13 |
| RFNoC PHY | **Architecture change required（位置付けの変更）** | 「Processor placement」ではなく Radio capability（Replay / DDC / DUC / FFT）。custom block は expert 拡張。任意 user Processor の FPGA 配置は **Not appropriate** | F20 |
| AI-generated WASM | Supported with planned extension | 条件：schema-first Event/Action（F10）、handle-based buffer（F3）、Executor 所有 ABI（F8）、step 駆動（F18）。WASM 固有事項は Core に漏れない | F3, F8, F10, F18 |

---

## 10. Comparison with Existing SDR Systems

一次資料で確認できた事実（Appendix B）に基づく。目的は Ez-SDR を同じものにすることではなく、**他が苦労した問題を同じ形で再発させないこと**。

| システム | 実際に苦労した / 選択した点（VERIFIED） | Ez-SDR への教訓 | 再発リスク箇所 |
|---|---|---|---|
| **GNU Radio 3.x** | thread-per-block scheduler；`lock()/unlock()` は全 graph を再 flatten し work thread から呼ぶと deadlock、buffer 長は変更不可；RT は best-effort（`RT_NOT_IMPLEMENTED / RT_NO_PRIVS`）；時刻は `rx_time` tag（uint64 s + double frac） | scheduler を Core に持たない（Vision §32 は正しい）；構造 mutation を禁止（F14）；時刻は tag ではなく block の型付きフィールド（F2, F3） | Processor 契約が block API 化する（F8）；「差し替え」要求から lock/unlock が生える（F14） |
| **GNU Radio 4**（RC1、GA 未達） | compile-time typed ports + MESSAGE port；scheduler は job-list 静的分割で deadline/budget primitive なし（「bounded execution time」は potential）；settings は work 境界で適用（`applyStagedParameters`）+ tag 駆動；double-mapped circular buffer の claim/publish；`PortDomain`（CPU/GPU/NET/FPGA）と**明示**変換 block；`externalStep` policy | block_boundary update class と一致（F14）；placement は明示（F13）；step 駆動 executor の先例（F18）；publish-once buffer（F3） | Core が GR4 の型システムを再発明する（F9）；deadline を持たない scheduler をそのまま「Island」と呼ぶ（F8） |
| **UHD / RFNoC** | OVERFLOW は「buffer 溢れ」と「sequence error」に overload（`out_of_sequence` で区別）；error 時は 0 サンプル；連続モードは 50 ms 後に自動再開；`LATE_COMMAND` 後は radio idle；async queue 深さ 1000；`set_time_unknown_pps` ≤ 2 s；SBX/UBX/OBX は retune で位相ランダム、timed tune で一定；CORDIC は SOB ごとにリセット；Replay は word/item alignment；rate は coerce；`O/D/U` を「generally harmless」と文書化；X4x0 GPIO は timed 不可（INFERRED） | Stream Contract に「gap を埋めない・50 ms 再開・per-channel validity」（F3）；TimingEnvelope（F4）；provider-declared coherence + timed tune を Mock が模倣（F6, F15）；Replay 制約を repeat capability に（F20）；typed event と counter（F16）；timing class は instance 宣言（F31） | v3 の再演：error_code を見ない、async を読まない、cout で済ませる |
| **SoapySDR** | 戻り値 `TIMEOUT/OVERFLOW/UNDERFLOW/TIME_ERROR/...`、flags `HAS_TIME/END_BURST/...`、`readStreamStatus` で TX 非同期、hardware time は int64 ns；`setCommandTime` は deprecated | 最小公倍数 API の限界：timed command / 多チャネル整列が弱い。Ez-SDR は LCD API でなく **capability envelope** で差を表す（F4, F7） | Radio Model を Soapy 水準に薄くして UHD の timed 機能を expert 拡張に押し出す |
| **srsRAN Project** | typed `radio_event {LATE, UNDERFLOW, OVERFLOW, ...}` + source + optional timestamp、UHD code からの写像表；時刻は `uint64` sample ticks；`radio_session::start(init_time)` で全 stream 同時開始；`time_alignment_calibration`（サンプル単位の TX/RX 遅延）；slot 先読み `max_processing_delay_slots`；pre-allocated buffer pool | 整数 tick 時刻（F2）；typed event 写像（F16）；aligned start = Session 級の primitive（F6）；delay calibration は SISO でも必須（F15）；RT path 無 allocation（F3） | — |
| **OpenAirInterface rfsimulator** | 同じ `openair0_device` interface を TCP で実装；lockstep（全 peer の timestamp が揃うまで block）；late/underflow/overflow を**模倣しない**；timestamp gap は warning のみ；`tx_sample_advance` を device 設定に持つ | **Mock-as-peer の最も近い先例であり、false confidence の実証例**。Mock は envelope を強制せよ（F4）；lockstep = DES 決定性（F18） | 「serious Mock は eventually 模倣する」（CMA §13）を放置する |
| **NVIDIA Aerial** | GPU slot batch（CUDA graph）、DOCA GPUNetIO で NIC→GPU 直接、PTP 必須（ptp4l/phc2sys）、late な L2 message は drop、RU emulator が O-RAN timing window を検証 | 異種 placement は手で組む（F13）；GPU = batch Island（F8）；emulator は timing 制約を検証する（F4） | 「同じ Processor を GPU に自動 placement」（Vision §20）を信じる |
| **SigMF** | captures の `core:global_index` が drop を表す唯一の機構；`core:sha512`；extensions namespace；Collections で多録音；validity/gap/calibration の公式・community 拡張は**存在しない**（NTIA `ntia-sensor` に calibration object と `overload` bool） | gap は captures 分割 + global_index、validity/calibration は `ezsdr` 拡張（F3, F19）；content hash 方式（F19） | 独自形式を発明する |
| **Agora / RENEW** | frame 単位の静的 pipeline（master → doer）、DPDK、hardware trigger 同期、reciprocity calibration に array 外の同期参照ノード、emulated RRU packet generator | Island の静的スケジュール（F8）；calibration は Run（F15）；coherence は物理分配（F6） | 動的 scheduler を欲しがる |
| **LuMaMi** | 50 USRP-RIO の FPGA 分散処理、OctoClock tree、<500 µs 予算 | host では届かない領域があることの明示 → PerformanceEnvelope と admission（F4, F13）；FPGA は Ez-SDR の対象外（§63） | — |
| **Techtile / White Rabbit** | PTP 既定、OctoClock 任意、WR は sub-ns；coherent DL beamforming には数十 ps 級（Bigler 2018） | 分散 coherence はソフトで合成不可。ClockRelation の uncertainty として扱う（F6, F25） | Core が cross-site CoherentGroup を作る |
| **FlexICoN / COSMOS（IBFD）** | analog canceller は SPI（USB adapter 経由）で設定、digital SIC は GNU Radio 側、X310 は OctoClock-G 同期 | Peripheral bounded-latency loop + TX timed tap（F21, F31） | — |
| **ISAC on X410 + MOCAP** | host 協調の開始時刻（<0.1 ms）、MOCAP は開始時刻オフセットで後処理相関 | ClockRelation + 外部データは参照（F2, F30） | 映像を取り込む |
| **OTFS testbeds（N310+PAAM、B210+VDI）** | 波形は MATLAB/Python 生成、USRP は IQ 搬送 | Tensor contract + 明示 placement で足りる（F9） | — |

### Ez-SDR が同じ失敗を繰り返しそうな箇所（要約）

1. **v3 と同じ穴**：RX error_code 無視、RX timestamp 欠落、async event を console に流す、文字列 escape hatch が本道になる。→ F3, F16, F1。
2. **OAI rfsimulator と同じ穴**：Mock が制約を模倣しない。→ F4。
3. **GR3 と同じ穴**：block API と lock/unlock を Core に持つ。→ F8, F14。
4. **Soapy と同じ穴**：LCD 抽象化。→ F4, F7。
5. **独自 planner**：GR4 も Aerial もやっていない自動 placement を Core に持つ。→ F13。

---

## 11. Missing Design Invariants

Vision §65 の 30 項目に**追加**すべき不変条件。数を増やしすぎないため、長期的に守る必要があり、かつ既存 30 項目から導けないものに限定した。

1. **Every timestamp names its ClockDomain, and sample time is integer ticks at a rational rate, never floating-point seconds.**（F2）
2. **Stream semantics are normative: gaps are represented by flags and time jumps and are never silently filled; validity is per channel.**（F3）
3. **A Mock that accepts what its emulated hardware would reject is a bug. Mock implements the declared constraint envelope, not the ideal.**（F4）
4. **Buffers are immutable-after-publish, reference-counted handles tagged with a MemoryDomain; no Module contract bakes in host-memory slices.**（F3）
5. **Kernel public types are schema-first with language-neutral serialization and explicit versions; old Specs are migrated or refused, never silently reinterpreted.**（F10）
6. **No structural graph mutation during RUN; parameters change only through declared update classes.**（F14）
7. **Coherence is declared by the Provider that owns the channels; Core never infers coherence across Providers.**（F6）
8. **Placement is explicit and validated, never optimized by Core.**（F13）
9. **Nothing transmits before validate() passes, including the RF safety envelope of the BindingProfile.**（F17）
10. **Stability has three tiers — Kernel (frozen), Vocabulary (versioned, additive), Extensions (unstable) — and "Core is small" applies to the Kernel.**（F1）

---

## 12. Recommended Vision Changes

### 12.1 must change before implementation（2026-09-21：Vision / CMA へ反映済み）

| 変更 | Finding |
|---|---|
| §5 / CMA §4 の Core 概念列挙を「Kernel が所有」「Vocabulary として提供」に二分し、3 層の安定性ポリシーを明記 | F1 |
| §15 / §23 に TimePoint の具体表現（integer ticks + rational rate + ClockDomain）、epoch、`TimeAuthority` interface、「Python は device-time sleep を持たない」を追加 | F2 |
| §23 / §28 を「Stream Contract」（immutable block、per-channel validity、gap 不埋、block 長非保証、full-scale 規約、TX block の target time、fan-out 参照共有、link policy）として規範化 | F3, F21 |
| §12–§17 に「Mock は TimingEnvelope / coercion / stop tail / restart gap を強制する」義務、Radio Model に TimingEnvelope 必須、TxBurst late policy、fidelity vector を追加 | F4 |
| §3 / §54 / §57 に Session（action log を持つ Run 種別）を定義し、Easy API はすべて Session Action として記録されることを明記 | F5 |
| §8 / §25 / §39 を composite resource tree、provider-declared coherence、arm ordering DAG に書き換え | F6 |
| §8 / §16 / §17 に「environment（channel / fault schedule / rf_envelope）は BindingProfile、Spec には書かない」を追加 | F23 |

### 12.2 should change before Core freeze（2026-09-21：Vision / CMA へ反映済み。F16 の Policy 閉集合は失敗方針の節である Vision §53 に、counter は §29 に配置）

| 変更 | Finding |
|---|---|
| capability matching を generic / schema-driven と明記 | F7 |
| §19 を「Core は Descriptor と Action/Event 語彙のみ、ABI は Executor」に修正；cycle 規則と deadline 2 種を追記 | F8 |
| §21 を DataContract open registry として書き直す | F9 |
| schema-first・version・migration・Python builder による式の代替を追加 | F10 |
| `prepare -> PrepareReport{effective, coercions}` を lifecycle に追加 | F11 |
| Lease attached / detached(TTL) | F12 |
| §10 / §20 の planner を「validate + explicit」に縮小 | F13 |
| §27 に「構造 mutation 禁止」を追加、GraphEpoch は defer | F14 |
| §26 に適用点の規則、calibration-as-Run、delay calibration、Mock の retune 位相 model | F15 |
| §29 に counter + `EVENTS_DROPPED`、Policy 閉集合 | F16 |
| BindingProfile に rf_envelope | F17 |
| §15 / §32 に step 駆動 executor 要件 | F18 |
| §50 を envelope + namespaced sections + content hash に | F19 |
| §20 / §32 / §67 の RFNoC を Radio capability に書き換え | F20 |
| §7 を Module / 役割 / 配備形態の 3 軸に | F22 |

### 12.3 can defer（2026-09-21：Vision §64 future work と該当節に反映済み）

| 項目 | Finding |
|---|---|
| out-of-process Radio Provider（RemoteProvider）、UHD crash isolation の実装 | F24 |
| NetworkLink / multi-host（ID の node-qualification 以外） | F25 |
| TUN/TAP helper 実装詳細 | F26 |
| Convert Processor の自動挿入 | F29 |
| GraphEpoch の atomic switch | F14 |
| RF-model fidelity の充実、Taint 伝播ガイド | F28 |
| timing class の instance 宣言（Peripheral 実装時） | F31 |

### 12.4 should explicitly reject（2026-09-21：Vision §63 / §6 に追記済み）

| 項目 | Finding |
|---|---|
| 自動 placement 最適化、graph fusion、transfer planner | F13 |
| RUN 中の構造的 graph mutation（GR3 式 lock/unlock） | F14 |
| Policy の式言語 / rules engine | F16 |
| Spec 内テンプレート・式・定数展開（v3 `!COMPUTE` の再演） | F10 |
| Probe を独立 Core 概念にすること | F27 |
| Taint を Core 型にすること | F28 |
| Metrics framework を Core に置くこと | F33 |
| 任意 user Processor の RFNoC/FPGA 配置 | F20 |
| Core が Provider 横断で CoherentGroup を合成すること | F6 |
| センサデータ（映像等）の取り込み | F30 |
| 「Distributed」を Module カテゴリとすること | F32 |

---

## 13. Recommended Minimal Core

レビュー結果を踏まえた **Kernel**（freeze 対象）の最小定義。これ以外は Vocabulary か Module。

```text
Kernel
├── identity        RunId, SessionId, NodeId-qualified {ResourceId, ClockDomainId, MemoryDomainId, IslandId}, ModuleId+version
├── lifecycle       Run / Session state machine: validate → plan → prepare → arm → start → running → stop → cleanup
│                   ExecutionPlan = {fragments, DataLinks, dependency DAG}; PrepareReport{effective, coercions}
├── ownership       Lease{Attached | Detached{ttl}}; cleanup policy on abort / disconnect / timeout
├── time            ClockDomain{tick_rate: Rational, epoch}, TimePoint{domain, ticks}, Duration, Deadline{Relative | Absolute},
│                   ClockRelation{offset, drift, uncertainty, measured_at, validity}, TimeAuthority interface
├── data            MemoryDomain, BufferRef, SampleBlock{time, len, channels, valid mask, flags}, Stream Contract (normative),
│                   DataContract registry (identity + compat), Port, DataLink identity + policy{block|drop_oldest|drop_newest}
├── events/actions  Event envelope{source, time, severity, kind, payload(schema)}, never-dropping counters, EVENTS_DROPPED,
│                   Action set {TxBurst(+late policy), SetTimer, UpdateParameter(class), PeripheralCommand, Emit, Stop, Abort},
│                   Policy = closed table on(kind) -> {continue, mark, stop, abort}
├── matching        generic Capability / Constraint matcher (schema-driven), coercion policy {accept|warn|reject}
├── resources       composite Resource tree (Device → sub-resources), Binding resolution (explicit)
├── spec/binding    ExperimentSpec envelope (versioned), BindingProfile = {bindings, environment (incl. rf_envelope)},
│                   Manifest envelope (namespaced sections, content hashes), Artifact reference
├── module-api      roles {Provider, Executor, Sink, Link}, ComponentDescriptor{ports, params+class, timing, impl hash},
│                   ExecutionIsland declaration + admission, step(until) requirement for Simulation class
└── execution class ExecutionClass {Simulation, RealtimeEmulation, HIL, Hardware} + fidelity vector

Vocabulary (versioned separately)
├── radio-model     channels/streams/RF params, TimingEnvelope, PerformanceEnvelope, coherence basis, repeat/decimation capability
├── calibration     CalibrationArtifact kinds, validity conditions
├── peripheral      Peripheral capabilities, timing classes, trigger in/out
├── endpoint        NetworkEndpoint / HostEndpoint
└── contracts       standard DataContracts: stream.*, pdu.*, tensor.*, event.*, control

Modules
    radio-mock, radio-uhd, radio-soapy, sim-kernel(DES), sim-channel, sim-faults, exec-native, exec-wasm, exec-gpu,
    hostio-tuntap/udp/pcap/file, peripheral plugin-host + plugins, link-spsc/shm/pinned, sink-sigmf/raw, frontends
```

Kernel に**含めない**：Radio 固有キー、Processor ABI、placement 最適化、graph mutation、rules engine、Spec 式、Probe 概念、Taint 伝播、metrics framework、calibration 適用、channel model、RFNoC graph 構築、sensor data 取り込み、cross-provider coherence 推論。

---

## 14. Core + Mock Implementation Readiness

### 14.1 `Core + Radio Model + MockRadio + SimulationChannel` に進む前に解決すべき項目

| # | 項目 | 種別 | Finding |
|---|---|---|---|
| 1 | Kernel / Vocabulary / Extension の 3 層と freeze 単位を Vision に明記 | 文書決定 | F1 |
| 2 | TimePoint 表現（ticks + rational）、epoch、TimeAuthority interface、DES kernel としての Simulation | 型 + 構造 | F2, F18 |
| 3 | Stream Contract（SampleBlock 型、flags、per-channel validity、ownership、fan-out、link policy、TX target time、full-scale） | 型 + 規範 | F3, F21 |
| 4 | Radio Model の TimingEnvelope / coercion 必須化と Mock の強制義務、TxBurst late policy、fidelity vector | 型 + 規範 | F4 |
| 5 | Session（action log 付き Run）と Easy API の写像、Lease attached/detached | lifecycle | F5, F12 |
| 6 | composite Resource tree、provider-declared coherence、arm ordering DAG | 型 | F6 |
| 7 | BindingProfile = bindings + environment（channel / fault / rf_envelope） | スキーマ | F23, F17 |
| 8 | generic matcher と PrepareReport | 型 | F7, F11 |
| 9 | DataContract registry と Port 形 | 型 | F9 |
| 10 | schema-first + version 方針（技術選定は不要） | 方針 | F10 |
| 11 | Descriptor / ABI 分離、cycle 規則、deadline 2 種 | module-api | F8 |
| 12 | Event counter + EVENTS_DROPPED、Policy 閉集合 | 型 | F16 |
| 13 | Manifest envelope + namespaced sections | 型 | F19 |
| 14 | RFNoC の位置付け変更（Processing 設計対象から外す） | 文書決定 | F20 |
| 15 | 用語 3 軸 | 文書決定 | F22 |

項目 1–7 が **P0**（Core + Mock の型に直結）、8–15 は P1 だが Core freeze 前に反映する。

### 14.2 受け入れテスト（Vision §58）への追加提案

- #2 と #3 は「DES kernel + step 駆動」でのみ同時成立する。テストに「同じ seed で event 順序まで一致」を含める。
- **#11（新）Mock envelope test**：`x310-like` Mock に lead 不足の timed TX を出すと実機と同じ `TIME_ERROR` event が出る。19.5 Msps 要求が 20 Msps に coerce され PrepareReport に現れる。
- **#12（新）Block-size independence**：Mock の block 長 jitter option で Processor が壊れない。
- **#13（新）Session manifest**：Easy API のみの利用で Manifest に action log と waveform hash が残る。
- **#14（新）Environment portability**：Spec を変えず BindingProfile の environment（channel model）だけ変えて再実行できる。

### 14.3 判定

```text
READY WITH REQUIRED CHANGES
```

**根拠：**

- **READY である理由**：Vision の方針（Core = transaction coordinator、Mock = peer、intent/binding 分離、typed event、compile-before-RUN、非目標の明文化）は一次資料で確認した他システムの失敗を正しく回避しており、Core + Mock から始める順序も正しい。P0 に挙げた項目はいずれも**設計文書上の決定**であり、新しい研究や大規模な試作を要しない。
- **REQUIRED CHANGES である理由**：P0 の 6 項目（+F23）は、いずれも最初のマイルストーンの **Module Contract の型**（TimePoint、SampleBlock/BufferRef、Resource tree、Session/Lease、TimingEnvelope、BindingProfile.environment）に現れる。未決のまま Mock を書くと、Mock の実装がそのまま契約になり、UHD Provider（Phase 7）で「Mock→X310 parity test」が失敗したときに Core / API を破壊して直すことになる。これは Vision §59 が「abstraction boundary を再考せよ」と書いている事態そのものであり、事前に避けられる。
- **NOT READY ではない理由**：Core/Module 分離の方針自体は妥当で、境界線の修正（3 層化、RFNoC の位置付け）は Vision の意図に沿った精緻化であり、方向転換ではない。scope creep の候補（自動 placement、graph mutation、rules engine、Spec 式）は Vision §63 の精神で「明示的に reject」すれば足りる。

---

## Appendix A — Architecture Stress Tests（End-to-End 検証）

各テストを Vision のモデルで頭の中で通し、**どこで semantic gap が生まれるか**を記す。

### A. Simple student experiment（Python → TX repeat → RX capture）

- 流れ：`connect()` → Session Run（暗黙 Spec：radio 1 台、既定 capability）→ `tx.repeat(x)` = Action StartRepeat{waveform hash} → `rx.capture(N)` = Action Capture{n, at: now + lead}。
- Gap：(1) Session が Run でなければ Manifest がない（F5）。(2) `now` は device time で lead は Provider envelope（F2, F4）。(3) `with` 終了で TX が止まるか（F12）。(4) 戻り値は配列だけでなく `first_sample_time` と flags を持つ（F3）。(5) repeat の waveform 長が Replay alignment を満たすかは実機 binding で validate（F20）。
- 結論：Kernel の Session / Lease / Stream Contract が決まれば Supported naturally。

### B. Pure software（MockRadio A → channel → MockRadio B）

- 流れ：Spec に radio A, B（2 資源）。BindingProfile.environment に SimulationChannel（A.tx→B.rx の結合、delay/AWGN）。DES kernel が A の TX block（time T）を T+delay で B の RX に届ける。
- Gap：(1) channel を Spec に書くと実機へ昇格不能（F23）。(2) 単一 virtual clock を A/B/channel が共有する必要（F2）。(3) 決定性は step 駆動（F18）。
- 結論：Supported naturally（F2, F18, F23 の解決後）。

### C. Reactive packet radio（RX → decode PING → Reactor → timed PONG）

- 流れ：RX SampleBlock(T0) → detector Processor → Event PacketDetected{t = T0 + k ticks} → Reactor → Action TxBurst{target = t + turnaround, waveform generated} → Radio TX。
- Gap：(1) turnaround ≥ `min_timed_command_lead` を validate が検査、不足なら plan 時に拒否（F4）。実機の host 生成 TX では 802.11 SIFS 級は不可能で、Vision §56 の litmus は「relaxed SIFS」でのみ成立する。この事実を Mock が隠してはならない。(2) 動的生成 waveform の block ownership（F3）。(3) Reactor の event 順序決定性（F18）。(4) late 時の policy（F4）。(5) cycle は Event/Action edge 経由（F8）。
- 結論：Supported with planned extension。**Vision §56 の表現を「envelope 内の turnaround で」と修正すべき**。

### D. Linux networking（ping → TAP → MAC → PHY → radio）

- 流れ：TAP Endpoint（`pdu.ethernet`）→ MAC Reactor（Packet event → TxBurst）→ PHY Processor（pdu → stream）→ Radio。逆方向は Radio → PHY RX → MAC → TAP write。
- Gap：(1) TAP 読み側の backpressure は drop policy + kernel 統計 → `TUN_QUEUE_DROP`（F16, F26）。(2) packet timestamp は host domain、latency 測定は ClockRelation（F2）。(3) 特権は one-shot setup（F26）。(4) MTU / 順序は MAC の責務。
- 結論：Supported with planned extension。

### E. MIMO（4 coherent RX → channel estimate → MIMO detector）

- 流れ：Spec `radio: {channels: 4, coherent: true}` → 単一 provider instance（X310×2 with addr0,addr1、または Mock x310-like ×2 相当）→ aligned SampleBlock（4 ch）→ channel estimator（tensor 出力）→ detector。
- Gap：(1) provider-declared coherence と arm ordering（F6）。(2) 片チャネル欠損は per-channel valid mask、Policy continue/abort（F3, F16）。(3) retune 後の位相ランダム → timed tune + calibration Run（F15）。Mock はランダム位相 option を持つ。(4) tensor contract（F9）。
- 結論：Supported with planned extension。

### F. IBFD（simultaneous TX/RX → analog/SI model → adaptive SIC）

- 流れ：TX Processor → Radio TX（block に target time）→ Tap → SIC Processor 参照入力；Radio RX → SIC Processor；SIC 係数は block_boundary 更新；analog canceller は Peripheral（bounded_latency）へ PeripheralCommand。Simulation では SimulationChannel の self-coupling が SI path。
- Gap：(1) TX timed tap と self-coupling topology（F21）。(2) TX↔RX 遅延 calibration（F15）。(3) saturation は Processor が計算する metric → event（Core 変更なし）。
- 結論：Supported with planned extension。

### G. ISAC（multi-channel RF + smart antenna + positioner + sensing）

- 流れ：coherent RX/TX（E と同じ）+ smart antenna Peripheral（GPIO sub-resource で hardware_timed、または USB best_effort）+ positioner Peripheral（best_effort、自 clock の event）+ sensing Processor（range-Doppler tensor、GPU 可）→ Artifacts。camera は trigger 付き Peripheral + 参照。
- Gap：(1) timing class は instance 宣言（F31）。(2) 外部センサは参照のみ（F30）。(3) 大容量 tensor は Artifact 参照 + Sink（F19）。(4) すべての event が ClockDomain 付き time を持つ（F2）。
- 結論：Supported with planned extension。

### H. OTFS / GPU（IQ → GPU → TF/DD tensor → detector）

- 流れ：Radio RX → DataLink（host → pinned → GPU MemoryDomain）→ GPU Executor（batch Island）→ `tensor.*` → detector（GPU or CPU）→ Sink。
- Gap：(1) 明示 placement と domain 互換 validate（F13）。(2) BufferRef が GPU domain を表せる（F3）。(3) batch latency は Island の宣言と admission（F8）。
- 結論：Supported with planned extension。

### I. Fault（RX overflow + peripheral timeout + client disconnect）

- 流れ：overflow → Provider が 0 サンプル + 次 block に `GAP_BEFORE{lost}` + `RESTARTED`（50 ms）+ Event RX_OVERFLOW → counter++ → Policy continue + mark artifact；peripheral timeout → Event PERIPHERAL_TIMEOUT → Policy（Spec 指定：abort or continue）→ baseline restore；client disconnect → Lease Attached なら TX 停止 → cleanup 順序（TX off → RX stop → peripheral restore → release）→ Manifest termination reason。
- Gap：(1) block flags の規範（F3）。(2) counter と storm（F16）。(3) Lease mode（F12）。(4) Mock は同じ event / flags / gap を注入で再現（F4）。
- 結論：Supported naturally（F3, F12, F16 の解決後）。

### J. Mock → Hardware promotion（同一 Spec、binding を Mock から X310/UHD へ）

semantic gap の一覧（すべて Mock が envelope を強制すれば **plan / validate 時**に検出できるか、fidelity vector で「未模倣」と明記できる）：

| # | Gap | 検出 / 扱い |
|---|---|---|
| 1 | 起動 epoch と PPS 同期（≤2 s）、start lead | TimingEnvelope（F4）；Mock は startup latency を模倣 |
| 2 | rate / gain / freq coercion | PrepareReport（F11）；Mock は同 grid |
| 3 | stop 後の tail drain | Stream Contract（F3）；Mock は tail を届ける |
| 4 | overflow = 0 サンプル + 50 ms 再開 + out_of_sequence 区別 | block flags（F3）；fault injection が同型 |
| 5 | retune 後 LO 位相ランダム（UBX/CBX/SBX） | Radio Model `phase_behavior_on_retune`（F15）；Mock option |
| 6 | repeat 長の Replay alignment / DRAM 上限、host loop wrap の underflow | repeat capability 制約（F20）；validate |
| 7 | gain step / 実効値 | coercion（F11） |
| 8 | full-scale と clipping | DataContract の scale 規約（F3） |
| 9 | block 長（packet 単位） | 非保証 + Mock jitter（F3） |
| 10 | 起動直後の過渡（DC settle、gain settle） | RF fidelity = none と明記；hardware-quirk-model で将来 |
| 11 | timed command queue depth（X3x0） | TimingEnvelope（F4） |
| 12 | 多チャネル ALIGNMENT error | per-channel validity + event（F3, F16） |
| 13 | timed GPIO の可否（X3x0 可 / X4x0 不可） | timing class instance 宣言（F31） |
| 14 | DC offset / IQ imbalance / LO leakage | RF fidelity = none；SimulationChannel option |
| 15 | transport / NIC 限界（1GbE で 4×200 Msps） | PerformanceEnvelope + admission（§34） |

結論：**論理的 gap（1–9, 11–13）は今回の P0/P1 で「validate 時に見える」ものにできる。RF gap（10, 14）は模倣せず、fidelity vector で明示する。** これが「Design once, validate in software, promote to hardware」の誠実な形である。

---

## Appendix B — Evidence Index

### B.1 Repository evidence（v3）

| ファイル | 該当箇所 | 示す事実 |
|---|---|---|
| `v3/cpp/uhd_usrp/multiusrp.cpp` | `continuousReceiveImpl`, `stopContinuousReceiveImpl`, `setParam` | RX `md.error_code` / `time_spec` を無視；stop は 128 サンプル recv を 0.1 s timeout で drain；PPS 同期は文字列 key `set_time_unknown_pps_to_zero` + `shared_mutex` + async task |
| `v3/cpp/uhd_usrp/uhd_rfnoc.cpp` | 285–330, 436–480, 997–1052 | async metadata と RX error を `std::cout` のみ；LATE_COMMAND で再開；`set_time_next_pps`；Replay `mem_size / numUsedChannels` |
| `v3/cpp/uhd_usrp/looptx_rfnoc_replay_block.cpp` | 290–400 | `get_word_size` による word 数計算、record/play、`record_restart` で flush |
| `v3/source/controller/cyclicrx.d` | `onRunTick`, `alignSize` | 「alignSize の倍数位置から capture」という v3 独自 semantics；SPSC request queue |
| `v3/source/controller/cyclictx.d`, `controller/package.d` | `run()`, `applyToDeviceSync` | PRIORITY_MAX busy loop（sleep なし）、GC.disable；関連 thread 全 pause で cold 更新 |
| `v3/source/device/package.d` | `IDevice.setParam/getParam` | 文字列 key/value escape hatch |
| `v3/source/device/hackrf.d` | RX/TX callback | callback 駆動 device（pull 型 UHD と異なる実行モデル）；queue 満杯で printf して破棄；ComplexInt8 |
| `v3/source/tcp_iface.d` | `eventIOLoop`, `ifaceVersion` | 切断で controller を止めない；protocol version 完全一致 |
| `v3/source/app.d` | `convertSettingJSONFromV1ToV2/V2ToV3`, `flagRetry`, `RestartWithConfigData` | 設定形式が 3 世代；UHD 例外で server 再起動 |
| `v3/source/settingfile.d`, `v3/changelog/v3.0.21.md` | `parseSettingFile` | `CONSTANTS` / `!COMPUTE` 評価器（設定の言語化） |
| `v3/changelog/v3.0.17.md` | 起動順序 | PPS 源から順に初期化しないと起動失敗；`UHD_RFNoC` device と `replay` streamer type |
| `v3/client/ezsdr.py` | `onTime`, `syncUSRPLoopTXRX`, `SimpleClient.sync`, `SimpleMockClient`, `changeAlignSize`, `typeConvert` | float 秒→ns；同期ダンス（sleep 1 s ×2 + 0.2 s lead）；Python 側 Mock が別 API（`sync()` の意味が異なる、`skipRx` は未定義変数を参照）；wire/CPU dtype 変換 |
| `v3/client/examples/test_continuous_recv.py` | 全体 | `nSamples-1` alignment で 1 サンプル drift を手動検出（runtime が continuity を提供しない） |
| `v3/docs/v3/usrp.md`, `v3/config_examples/*.json` | `args: addr0,addr1`, `timeref/clockref` 配列, `tx-streamers.channels` | 1 MultiUSRP が複数筐体を束ねる；streamer = channel 集合 |
| `v3/docker/v3_prebuild/Dockerfile` | `UHD_VERSION=v4.7` | 参照 UHD 版 |

### B.2 External primary sources

**UHD（Ettus Research）** — VERIFIED unless noted
- `rx_metadata_t` / `async_metadata_t`：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/include/uhd/types/metadata.hpp ／ https://files.ettus.com/manual/structuhd_1_1rx__metadata__t.html
- overflow 再開（`OVERRUN_RESTART_DELAY` 0.05 s、error 時 0 サンプル）：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/lib/rfnoc/rfnoc_rx_streamer.cpp ／ `.../host/lib/rfnoc/radio_control_impl.cpp`（source から確認）
- `recv_async_msg`、queue 深さ 1000：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/include/uhd/stream.hpp ／ `.../host/lib/rfnoc/rfnoc_tx_streamer.cpp`
- 同期・timed tune・LO 位相・CORDIC reset：https://files.ettus.com/manual/page_sync.html ／ https://files.ettus.com/manual/page_dboards.html ／ https://files.ettus.com/manual/page_usrp_x3x0.html ／ https://files.ettus.com/manual/page_octoclock.html
- `set_time_unknown_pps` ≤ 2 s、rate coercion：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/include/uhd/usrp/multi_usrp.hpp
- Replay block 制約：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/include/uhd/rfnoc/replay_block_control.hpp ／ https://files.ettus.com/manual/classuhd_1_1rfnoc_1_1replay__block__control.html ／ https://files.ettus.com/manual/page_usrp_x4xx.html（64-byte word は INFERRED）
- stream command / 多チャネル整列 / LATE 後 idle：https://raw.githubusercontent.com/EttusResearch/uhd/master/host/include/uhd/types/stream_cmd.hpp ／ https://files.ettus.com/manual/page_stream.html ／ `.../host/examples/benchmark_rate.cpp` ／ `.../host/python/uhd/usrp/multi_usrp.py`（stop 後の drain）
- transport / `O D U` は「generally harmless」：https://files.ettus.com/manual/page_transport.html ／ https://files.ettus.com/manual/page_usrp_x3x0_config.html ／ https://files.ettus.com/manual/page_general.html
- GPIO timed 可否：https://files.ettus.com/manual/page_gpio_api.html ／ https://files.ettus.com/manual/page_x400_gpio_api.html ／ `.../host/lib/usrp/x300/x300_radio_control.cpp`（timed_wb_iface；文書化は INFERRED）／ `.../host/lib/usrp/x400/x400_gpio_control.cpp`（untimed poke32；INFERRED）

**GNU Radio 4 / 3.x** — VERIFIED
- Port / Block / Scheduler / Settings / CircularBuffer / README：https://raw.githubusercontent.com/fair-acc/gnuradio4/main/core/include/gnuradio-4.0/{Port.hpp,Block.hpp,Scheduler.hpp,Settings.hpp,CircularBuffer.hpp,Buffer.hpp} ／ https://raw.githubusercontent.com/fair-acc/gnuradio4/main/core/README.md ／ https://raw.githubusercontent.com/fair-acc/gnuradio4/main/README.md
- status：https://www.gnuradio.org/news/2026-03-22-gr4-release-candidate-1/ ／ https://www.gnuradio.org/news/2026-08-16-gr4-easy-to-build/ ／ https://www.gnuradio.org/news/2025-12-17-gr4-transform-sdr-workflows/ ／ https://www.gnuradio.org/news/2022-12-03-low_level_api/
- GR3 TPB / lock-unlock / realtime / rx_time：https://raw.githubusercontent.com/gnuradio/gnuradio/main/gnuradio-runtime/lib/tpb_thread_body.cc ／ `.../gnuradio-runtime/lib/top_block_impl.cc` ／ `.../gnuradio-runtime/include/gnuradio/top_block.h` ／ `.../gnuradio-runtime/include/gnuradio/realtime.h` ／ `.../gr-uhd/include/gnuradio/uhd/usrp_source.h` ／ https://www.gnuradio.org/doc/doxygen-3.7.7.2/page_operating_fg.html

**srsRAN Project** — VERIFIED
- https://raw.githubusercontent.com/srsran/srsRAN_Project/main/include/srsran/radio/radio_event_notifier.h ／ `.../include/srsran/radio/radio_session.h` ／ `.../lib/radio/uhd/radio_uhd_tx_stream.cpp` ／ `.../lib/radio/uhd/radio_uhd_rx_stream.cpp` ／ `.../include/srsran/gateways/baseband/baseband_gateway_transmitter_metadata.h` ／ `.../include/srsran/gateways/baseband/buffer/baseband_gateway_buffer_pool.h` ／ `.../include/srsran/phy/lower/lower_phy_configuration.h` ／ `.../lib/radio/uhd/radio_uhd_impl.cpp`
- https://docs.srsran.com/projects/project/en/latest/user_manuals/source/config_ref.html ／ https://docs.srsran.com/projects/project/en/latest/user_manuals/source/troubleshooting.html

**OpenAirInterface** — VERIFIED（GitHub mirror）
- https://raw.githubusercontent.com/OPENAIRINTERFACE/openairinterface5g/develop/radio/COMMON/common_lib.h ／ `.../radio/rfsimulator/README.md` ／ `.../radio/rfsimulator/simulator.cpp`

**SoapySDR** — VERIFIED
- https://raw.githubusercontent.com/pothosware/SoapySDR/master/include/SoapySDR/{Errors.h,Constants.h,Device.hpp}

**SigMF** — VERIFIED
- https://sigmf.org/ ／ https://raw.githubusercontent.com/sigmf/SigMF/sigmf-v1.x/sigmf-spec.md ／ https://github.com/sigmf/SigMF/tree/main/extensions ／ https://github.com/sigmf/community-extensions ／ https://github.com/NTIA/sigmf-ns-ntia/blob/master/ntia-sensor.sigmf-ext.md

**NVIDIA Aerial** — VERIFIED（TAI / slot 先読み数値は INFERRED）
- https://docs.nvidia.com/aerial/cuda-accelerated-ran/26-1/cubb/cuphy_developer_guide/index.html ／ https://docs.nvidia.com/aerial/cuda-accelerated-ran/latest/cubb/cuphy_developer_guide/cuphy_components.html ／ https://docs.nvidia.com/aerial/cuda-accelerated-ran/latest/quickstart_guide/running_cubb-end-to-end.html ／ https://docs.nvidia.com/aerial/cuda-accelerated-ran/latest/overview.html

**Testbeds** — VERIFIED unless noted
- Agora：https://github.com/Agora-wireless/Agora ／ RENEW：https://renew-wireless.org/technology.html ／ https://github.com/renew-wireless/RENEWLab（hub trigger 数は INFERRED）
- LuMaMi：https://lup.lub.lu.se/search/files/6253008/4857740.pdf ／ https://arxiv.org/abs/1701.01161
- Techtile：https://arxiv.org/pdf/2202.04524 ／ White Rabbit for distributed mMIMO（Bigler et al., ISPCS 2018）：https://thomaszemen.org/papers/Bigler18-ISPCS-paper.pdf ／ https://ieeexplore.ieee.org/document/8543079/
- FlexICoN / COSMOS IBFD：https://wimnet.ee.columbia.edu/wp-content/uploads/2021/10/orbit_cosmos_comnets_2021.pdf ／ https://github.com/Wimnet/flexicon_cosmos
- ISAC X410 + MOCAP：https://arxiv.org/html/2602.00054 ／ https://ieee-dataport.org/documents/multi-static-ofdm-radar-dataset-human-activity-analysis-three-usrp-x410-based
- OTFS OTA：https://arxiv.org/html/2511.07610 ／ https://arxiv.org/html/2504.15947

**Linux / Rust** — VERIFIED
- tuntap（CAP_NET_ADMIN は device 作成 / 非所有 device 接続に必要、multiqueue 3.8+、IFF_NO_PI）：https://docs.kernel.org/networking/tuntap.html
- PREEMPT_RT mainline（6.12）：https://www.phoronix.com/news/Linux-6.12-Does-Real-Time
- Rust に安定 ABI がなく動的ロードは `#[repr(C)]` + `abi_stable` 等に依る：https://docs.rs/abi_stable/ ／ https://internals.rust-lang.org/t/a-stable-modular-abi-for-rust/12347

---

*End of audit. 既存 Vision 文書は変更していない。本文書のみを追加した。*
