# #60 — FakeDevice を手動の時刻で動かす計画

| 項目 | 内容 |
|---|---|
| 状態 | **段階Aまで完了（2026-10-13、§15）。** #60 を閉じる条件と mutation の並列度を上げる条件を、どちらも満たした。段階Bは不要（D2 の残りは段階Aの中で片づいた）。計画の本文は第5版のまま |
| 作成日 | 2026-10-12 |
| owner の判断 | 2026-10-12「私が決めることはあなたの推奨でOKです」：D1 は受け入れる。手動の時計にするのは smoke set だけで、既定は実時間。D2 は段階Aの実測のあとに決める。同日「所要時間が変わらなくても並列化できるのと決定的になるだけでよいです」：短縮は目標にしない |
| 独立レビュー | Sol 6.1、第1回 REJECT（P0 1・P1 8・P2 1）、第2回 REJECT（P0 は解消、P1 2・P2 1）、第3回 REJECT（P1 1）、第4回 APPROVE_WITH_CHANGES（P2 2）。全件採用。反映先は §11〜§13 |
| 調査時の基準 | `main`、HEAD `a50348a`、作業ツリー clean |
| 対象 | [#60](https://github.com/k3komatsu/Ez-SDRv4/issues/60)（spec 22 の Follow-ups 3、VH-8）、[failure-mechanisms.md](failure-mechanisms.md) の `mech:wall-clock-test-oracle` |
| 目標 | FakeDevice の smoke set を、負荷をかけても結果が変わらない（決定的な）テストにし、mutation を高い並列度で流せるようにする。所要時間の短縮は目標にしない |

## 1. 推奨：「遅れる時計」を Device に持たせる

FakeDevice、`DeviceAuthority`、Provider のスレッドが読む**ホストの時刻**を、1つの時刻源 `HostClock` から読むようにする。`HostClock` は `Device` が渡す。実機の `UhdDevice` と既定の `FakeDevice` は実時間の時計を渡し、動きは今と同じ。smoke set の FakeDevice だけが**手動の時計**を選ぶ。

手動の時計の規則は次の7つ。

- **M1 参加者:** 参加者は、uhd crate が自分で起こすスレッドだけ（`uhd-rx`、`uhd-tx`、`uhd-control`、`uhd-clock`）。`HostClock::spawn` は、スレッドを起こす**前に**登録する。起動に失敗したら登録を取り消す。スレッドが終わるとき（panic を含む）は、guard が登録を外す。
- **M2 idle の定義:** 参加者が idle なのは、次の2つのどちらかの間だけ。
  - 時計の待ち（`sleep`、時計の condvar 待ち）の中にいる間
  - 時計の lock（§4.7）の取得を待っている間

  それ以外の時間は、std の mutex 待ちも含めて busy とみなす。
- **M3 busy の間は止まる:** 参加者が1つでも busy なら、時刻は止まる。
- **M4 期限で止まる:** 待っている参加者の期限に時刻が着いたら、その参加者が動き出すまで、時刻はそこで止まる。次の参加者も、その時点で busy とみなす。
  - 期限に着いた参加者
  - notify で起こされた参加者
  - 時計の lock を手渡された参加者
- **M5 実時間より速くならない:** 全員が idle の間だけ、時刻は実時間の速さで進む。busy の間に止まった分を、あとで取り戻すことはない。
- **M6 参加者でないスレッドの待ち:** 参加者でないスレッド（試験のスレッド、Kernel の `ezsdr-data` と `ezsdr-bounded`）も手動の時刻で待つ。ただし、その待ちは時刻を止めない。
- **M7 sleep は短くならない:** `sleep(d)` は、手動の時刻が d だけ進むまで終わらない。ほかの notify や spurious wake で早く終わることはない。

これが #60 の言う「idle handshake」の具体形である。試験のスレッドが時刻を進めるのではなく、時計が自分で進む。

**効果（INFERRED）:** uhd のスレッドが OS に待たされても、デバイスの時刻は進まない。だから booking の遅れ、burst の遅れ、受信の overflow が、ホストの負荷では起きなくなる。段階0と段階Aで実測して確かめる。デバイスの tick で書いた envelope（start lead、in-flight window、3 ms の delivery allowance など）は1つも緩めない。

**Kernel は変えない。** 参加者でないスレッドから見た時刻の進みは、今以下の速さになるだけである。

## 2. 根拠

- **E1 FakeDevice:** `Instant::now()` と `std::thread::sleep` で動いている（`crates/ezsdr-radio-uhd/src/device.rs:419,506,636,1140`、sleep は約20か所）。
- **E2 Authority:** `DeviceTime` は、ホストの `Instant` を基準にデバイスの時刻を外挿している（`authority.rs:74-106,135-143,175-184`）。そのため、Fake の時刻だけを仮想にすると、Authority の外挿と食い違う。
- **E3 Provider のスレッド:** 実時間で眠り、実時間を読んでいる（`rx.rs:156-157,201,228,299,354,558`、`tx.rs:127,133,755-758`、`control.rs:224-241,712`）。polling の待ちが実時間なので、負荷で起床が遅れると、そのまま期限に遅れる。
- **E4 負荷試験の結果:** `--test-threads=128` で 134 件中 34〜37 件が落ちた。症状は「booking の遅れ、サンプルの欠けや余り、link の drop」（handoff.md の 425・589・712 行付近）。個別に流しても、ときどき落ちるテストがある：`rehearsal_b6_…`、`uhd_61_01`、`uhd_61_03`、`ur_14_a`、`ur_21_…`、`ur_2x_…`。
- **E5 実時間の上限:** Kernel 側の待ちは実時間で上限を持つ。`DEFAULT_HOST_BUDGET_NS` = 5 s（KC-12a の `bounded`、KC-21a の `wait_finished`、`coordinator/paced.rs:198-257`）。Provider 側には `JOIN_BOUND` = 1 s（`provider/mod.rs:56`）がある。
- **E6 待ちの調査:** 通常の経路では、Session の action を受け取るときに待たない（control の `actions.recv()`、tx の `try_recv`）。ただし、参加者同士で共有する lock を device の呼び出しの間ずっと持つ箇所はある（`ColdConfig::configure`、`control.rs:88-92`。`release`、`control.rs:450-461`）。これが §4.7 の理由である（Sol R01）。

## 3. 採らない案

- **試験側が時刻を進める手動時刻（#60 の文面どおりの案）:** 外部の駆動役と uhd の参加者だけで handshake を組むことはできる。しかし、uhd 以外のスレッドとの同期が、別の問題として残る。
  - 時刻を進める判断が、Kernel のスレッドと試験のスレッドの状態に依存する。Kernel の `ezsdr-data`、`ezsdr-bounded`、Kernel の呼び出しの中にいる試験のスレッドが、その例である。
  - これらに確実な同期を足すには、Kernel に試験用の仕組みを入れることになる（AGENTS.md §3「Kernel stays small」）。
  - 実時間で上限を持つ待ち（E5）と組み合わせると、止まった時刻の中で上限が切れる。

  そのため、時計が自分で進む方式を選ぶ（Sol R10 を受けて表現を限定した）。
- **Fake の時刻だけを仮想にする案:** Authority の外挿（E2）と、Provider の polling の待ち（E3）が実時間のまま残るので、遅れはなくならない。
- **余裕を広げる案・再試行する案:** `tests/README.md` が禁じている。退行も隠してしまう。
- **smoke set をさらに削る案:** Coordinator と bench の予行を通しで流すテストは、ほかにない。spec 22 の Follow-ups 6 は補完にはなるが、置き換えにはならない。
- **lock の問題（§4.7）への別解:**
  - `ColdConfig` が lock を持たずに device を呼ぶよう作り直す案は採らない。投影と device への発行が原子的であることを保つ方法を、新しく設計し直すことになり、本番の挙動も変わる。
  - Fake の `apply_delay` を busy な std の sleep にする案も採らない。configure の間にデバイスの時刻が進まなくなり、`ur_25_a_receive_enable_counts_its_start_lead_from_the_end_of_its_configuration` が試している意味が消える。

## 4. 設計

### 4.1 `HostClock`

uhd crate に1ファイルを足す（`src/host.rs`）。

- **API:**
  - `now() -> Duration`（時計の原点から）
  - `sleep(d)`
  - 時計の condvar 待ち（期限付き、notify で起きる）と notify
  - 時計の lock（§4.7）
  - `spawn(name, f) -> JoinHandle`
  - 手動の時計を作る `HostClock::manual()`
- **実時間の場合:** std をそのまま呼ぶ。
- **手動の場合:** M1〜M7 を、時計の内部の mutex 1つで実装する。
- **線形化点と lock の順序（Sol R04）:**
  - condvar 待ちでは、外の guard を持ったまま時計の内部の mutex を取る。その mutex の中で、待つ者の登録と外の guard の解放を、ひとかたまりにして行う。
  - notify・期限への到達・lock の手渡しは、どれも時計の内部の mutex の中で起きる。待っていた参加者を busy にするのも、同じ操作の中で行う。
  - 起きた側は、時計の内部の mutex を放してから外の guard を取り直す。
  - 順序は「外の guard → 時計の内部の mutex」の一方向だけにする。

### 4.2 置き場所

`Device::host(&self) -> HostClock` を足す。安く clone できる型にし、既定の実装は実時間の時計を返す。

- `Core` と `DeviceAuthority` はすでに device を持っているので、コンストラクタも server の catalogue も変えなくてよい。
- 試験用に Device を包む型（`tests/fake.rs` の `Watched` など）は、`host()` も中身へ転送する。転送しないと、既定の実時間の時計を継承してしまい、Authority と Fake の時刻が再び食い違う（Sol R07）。

### 4.3 置き換える箇所（本番コード）

- **`device.rs`:** `zero`、`now`、すべての `sleep`、`sleep_ticks`、`apply` の満杯待ち、`tx_send` の flow control の期限。
- **`authority.rs`:** `Anchor.host`（手動の時計の時刻を足し、実時間の時刻と両方を持つ。§4.6）、`read` の bracket、`extrapolate`、`nap` の condvar（`schedule` と `cancel` の notify を含む）、`wait_until`、`uhd-clock` の起動と停止待ち。
- **`provider/rx.rs`:** `last_samples`（UR-29 の無音の監視）、Idle の待ち、stall の注入、stall を発火させる起点（今は `first_block`、`rx.rs:222-228`）。
- **`provider/tx.rs`:** 1 ms の待ち、`IDLE` の待ち、`shutdown` の最後の BURST_ACK を待つ 100 ms の窓（`tx.rs:751-758`）。この窓はデバイスの ACK を待つ時間なので、手動の時刻で測る（Sol R02）。
- **`provider/control.rs`:** `last_check`、`POLL`、`failing_since` の `begun`。
- **`provider/mod.rs`:** スレッドの起動を `host.spawn` にする。
- **lock（§4.7）:** `ColdConfig.updates`。

**実時間のまま残すもの（意図的）:**

| 対象 | 理由 |
|---|---|
| Kernel の上限（E5）、`JOIN_BOUND`、Authority の `CLOCK_JOIN` | 「デバイスの呼び出しの中で止まったスレッド」を見切るための上限である。そのスレッドは busy なので、手動の時刻はその間止まる。上限が実時間でなければ、いつまでも発火しない |
| UR-30 の `host_delay_ms`（`rx.rs:156,558-560`） | ホストの遅れの測定値なので、実時間で測る（Sol R06）。今の `first_block` は、この記録と stall の起点（上の一覧）の2つに使われているので、実時間の診断用の時刻と、`HostClock` の stall の起点とに分ける（Sol S02） |
| `DeviceTime` が公開する `host.monotonic`（`now(HOST_MONOTONIC)`、HOST の時刻での待ちと予約。§4.6 のとおり root には直さない）と、UR-8 の relation | TM-16a1 は、HardwareInLoop と Hardware の `host.monotonic` を実際のホストの時計と定めている。手動の時計は Authority の中の外挿だけに使い、予約された domain の意味は変えない（Sol S01。§4.6） |
| RS-22 の Lease と RS-5 の遷移の UTC | Kernel の `HostClock`（実時間）が測るもので、uhd の時計とは関係しない（Sol R08） |
| `UhdDevice` が UHD C API に渡す timeout（`uhd.rs:774-786,853-886`）と、UHD の control・stream の mutex（`uhd.rs:451-452`） | 実機の側での実時間の待ちである。`UhdDevice` は実時間の時計しか使わないので、時計の lock にする必要もない（Sol R02、第2回の lock の監査） |

### 4.4 どのテストが手動の時計を使うか（Sol R03・R07）

`FakeConfig` に、実時間か手動かを選ぶ欄を足す。既定は実時間。段階Aでは、実装役が次の分類の一覧表を出し、orchestrator が決める。

- **手動に切り替える:**
  - smoke set の通しのテスト：`tests/fake.rs` の Run を使うテスト、`ezsdr-acceptance/tests/uhd.rs`、`ezsdr-server/tests/protocol.rs` のうち FakeDevice で Run するもの
  - Provider のスレッドを実際に動かす `Direct` 系（`fake.rs:546-639` など）
- **実時間に残す:**
  - 意図的に永久に止まるテスト（`FakeFault::TxBlocks`、`fake.rs:819-843` の wedged・join・leak）。止まった呼び出しは busy なので、手動の時刻は進まず、試験が `finish` まで届かない。
  - crate 内の `ManualTimeAuthority` rig のテスト。
- **個別に分類する:** Authority 自身のテスト。少なくとも次のものを名指しする。
  - 進む速さ（`fake.rs:208-216`）
  - `schedule` と `cancel` による起床（`:220-241,369-386`）
  - bracket の破棄（`:258-265`）
  - callback が lock を放すこと（`:317-344`）
  - idle 中の再アンカーと drift（`:390-396`）
  - drop と最後の handle（`:2856-2890`）

  実時間の遅れを測る診断テストは、実時間に残してよい。ただし、通知と停止の機能は、手動の時計の上でも別にテストする（§7）。

### 4.5 試験側の待ち

試験のスレッドは参加者ではない。smoke set の中で、実時間で待ってデバイスの進みを期待している箇所は、手動の時刻で待つように直す。

- `std::thread::sleep` は、`device.host().sleep` か `run.advance_to` にする（`fake.rs` に約19か所）。
- 実時間で測っている assertion（`begun.elapsed()` など）も、§4.4 の一覧表で分類する。

### 4.6 意味の変化（D1、owner 承認済み）

- **CPU 時間:** 手動の時計では、ホストの CPU 時間がデバイスの時間軸に乗らない。smoke set は「Provider が実時間に追いつくこと」を証明しなくなる。これは Phase 8 の bench の仕事になる（`tests/README.md` はすでにそう書いている）。
- **意図的なホストの停止:** `with_rx_stall`、`SlowSend`、`Unreachable`、`timed_out`、`apply_delay` は時計の待ちになるので、その間もデバイスは進む（M7 により短くもならない）。overflow、underflow、デバイス喪失の試験は、今と同じ意味で残る。
- **時刻を表す数値:** UR-14、UR-27、UR-29 の ms や s は、手動の時計ではその時計の時間になる。例外は §4.3 の表のとおり。
- **ホストとの relation と予約された domain（Sol R06・S01）:** `host.monotonic` と `utc` は、手動の時計でも本物のままにする。TM-16a1（`design/01-time-model.md:177`）と TM-18（`:184`）は変えない。
  - Authority の `Anchor` は、デバイスの時刻を読んだ瞬間を、手動の時計と実時間の両方で記録する（実時間の時計なら両者は同じ）。
  - 外挿（`extrapolate`）は手動の時計で行う。bracket の幅の規則（UR-7 の 1 ms）も、外挿に使う手動の時計の幅で判定する。
  - `now(HOST_MONOTONIC)` は実時間を返す。
  - **HOST_MONOTONIC の時刻での待ちと予約は、root に直さない（Sol T01）。** TM-16c と TM-16d は「指定された domain の `now` が `t` に着いたら終わる・発火する」と定めている。だから、HOST の時刻の `wait_until` は実時間の `now(HOST_MONOTONIC)` で判定する。HOST の時刻の `schedule` は、HOST の時刻のまま持ち、`next_wakeup` も実時間で判定して、その HOST の時刻を返す。
    - 今のコードは、HOST の時刻を anchor 経由で root の tick に直して待っている（`authority.rs:112-126,175-198,384-409`）。これでは手動の時計が止まっている間、`now(HOST)` は進んでいるのに待ちが終わらない。
    - この規則は、実時間の時計でも手動の時計でも同じにする。モードで分岐させない。そのため、**本番の経路でも**次のことが変わる（Sol U01）。
      - HOST の時刻の判定は、anchor の外挿から実時間に変わる。
      - HOST の時刻を device の root に直すときの `Inexact`（`authority.rs:117-126`）が出なくなる。
      - `next_wakeup` と callback は、HOST の予約については HOST の時刻を返す。
      - root と HOST の間の順序と `InPast` の境界が変わる。
    - HOST の期限を待っている間の再判定は、手動の root が止まっていても実時間で起きる。途中の sleep や condvar の timeout も、HOST の期限については実時間で測る（Sol U02）。
    - root の時刻の予約は今のまま、手動の時計の外挿で判定する。
    - 順序：同じ domain の中は時刻順、同じ時刻なら登録順（TM-16c）。別の domain の予約の間は、`next_wakeup` が due だと判定した順にする。同じ判定で両方が due なら root を先にする。KG-9 の `InPast`（最後に発火した時刻より前を拒否する規則）は、**正規化したあとの root の時間軸ごと**に判定する。device の root とそこから派生した domain は、1つの queue と `last_fired` を共有し、HOST だけを別にする（Sol U01）。TM-16c と KG-9 の「ascending time」「last instant」は、異なる root の間で数値を比べることを意味しない、と amendment で明記する。
    - smoke の経路にも Kernel にも、HOST の時刻で待つ・予約する箇所はない（第3回のレビューで確認済み：Coordinator、schedule、Session、server の `within`・`until` は primary root）。したがって、この変更は Authority の契約を正すもので、smoke の結果は変えない。
  - UR-8 の relation（root → `host.monotonic`、root → `utc`）は、実際のホストの時計と UTC で bracket を取る。手動の時計では、デバイスの時刻が止まっても host と UTC は進むので、有効期間（`Validity`）を**測った瞬間だけ**（`from = to =` 測った root の時刻）にする。実時間の時計では今のまま（`to` なし）。
  - relation を省くと、device-paced の Run の cleanup failure になる（`crates/ezsdr-kernel/src/coordinator/ending.rs:282-303`）。そのため省かない。
  - 手動の時計の UTC を合成する案は採らない。予約された `utc` は実際の UTC で、method 名を変えてもその意味は変わらない（Sol S01）。
- **書き直す文書:**
  - `tests/README.md` の「The fake's clock … must not be slowed」
  - spec 18 の「wall-clock double」の記述（§1、UR-33、§6）

### 4.7 時計の lock（Sol R01）

参加者が、device の呼び出しや時計の待ちの間ずっと持つ lock のうち、ほかの参加者も取る lock は、**時計の lock** にする。

- **実時間の場合:** std の `Mutex` と同じ。
- **手動の場合:**
  - 取得を待つ参加者は idle とみなす（M2）。
  - 解放するときは、待っている参加者の1人へ、時計の内部の mutex の中で lock を手渡し、その参加者を busy にする（M4）。
  - こうすると、持ち主が時計の待ちにいる間は時刻が進み、手渡しの直後は止まる。
- **既知の対象:** `ColdConfig.updates`。
- **洗い出し:** 段階Aでは、実装役が uhd crate のすべての共有 lock について「device の呼び出しや時計の待ちの間ずっと持つか」の一覧を出す。
  - Fake の内部の `state` は、待つ前に必ず放しているので std のままにする（`device.rs:731,833,862,927` など）。この点も一覧に含める。
  - 第2回のレビューでは、`updates` 以外に対象となる lock は見つからなかった（`release`、`cancel_held`、RX と TX の owner、`Provider::stop`、Authority の各 mutex を確認済み）。
- **lock の順序:** 今の `updates → streams → rec` の順序（`provider/core.rs:52-56`）を保つ。時計の内部の mutex を持ったまま、外の mutex を取ったり device を呼んだりしない。論理的な時計の lock の guard と、時計の内部の mutex の guard は別物である。

## 5. この設計が扱わない残り

- **(b) Kernel の非参加者が遅れる:**
  - `ezsdr-data` の消費が遅れて link が drop する。
  - drop がなくても、Sink のファイル処理、completion event の生成と drain、KC-46a の後の control loop の起床が遅れ、event を待つ試験の期限に間に合わない。

  Sink は data thread の上で動く（`ezsdr-sink-capture/src/lib.rs:420-489,576-589`）。feed の capacity を上げても、このうち drop しか解決しない（Sol R08）。
- **(c) 試験のスレッドの遅れ:** 試験のスレッドが `now` を読んでから action を dispatch するまでに OS に待たされると、時刻が先に進む。
- **(d) 実時間の上限:** 負荷が非常に高いと、手動の時刻は実時間よりずっと遅く進み、KC-12a・KC-21a の 5 s や `JOIN_BOUND` の 1 s に近づくおそれがある。

段階0と段階Aで実測し、残ったら **D2** として owner に判断してもらう。

| 残り | 案 |
|---|---|
| (b) の drop | smoke の profile の feed capacity（今は 64）を上げる。デバイスの envelope ではない |
| (b) のそれ以外 | 試験の待ちを event の到着で判定する形に直す。実測で、どの待ちが残るかを見てから決める |
| (c) | 段階B（下記） |

**段階B（Sol R09）:** 試験用の `hold()` で時刻を止める案である。D2 で採るときは、計画を書き足して独立レビューを受け直す。

- `hold()` の間も polling の待ち（control の `POLL`、tx の `IDLE`、rx の Idle）は実時間でも終わり、polling を続ける。
- 擬似的なデバイスの遅延（`SlowSend`、`apply_delay`、RX の stall）は、実時間で早く終わってはいけない（M7）。
- 実時間で起きた参加者を busy にする操作も、M4 と同じ原子性で行う。
- デバイスの queue を満たす action をまたいで `hold` してはいけない。

## 6. 段階

1. **段階0（基準の実測。orchestrator が sim03 で実行）:**
   - 負荷試験（`--test-threads=128`）を3回流す。対象は `-p ezsdr-radio-uhd --test fake`、`-p ezsdr-acceptance --test uhd`、`-p ezsdr-server --test protocol`（Sol R08）。
   - 落ちたテストを、`TIMING_PRECONDITION_UNMET` と Manifest のイベントを根拠に、(a) uhd のスレッドの期限切れ、(b)、(c)、(d)、その他に分類する。結果はこのファイルに記録する。
2. **段階A（本体）:**
   - 範囲：§4 のすべてと、§7 のテスト、§8 の文書。
   - 終わりに gate を通す（全6リストの mutation を sim02/sim03 で流す）。そのあと、段階0と同じ負荷試験を、§14 の4条件（CPU と `--test-threads` の組み合わせ）でそれぞれ3周流す。
   - **#60 を閉じる条件:** (a) の失敗が 0 件で、通常の実行が全部通る。(b)〜(d) の件数を報告する。
   - **mutation の並列度を上げる条件:** 負荷試験の失敗が全部で 0 件。(b)〜(d) が残ると、変異のせいではない失敗で mutant を誤って kill した扱いになるので、並列度は D2 の対策が済むまで上げない（Sol R08）。
3. **段階B（D2 次第。§5）。**
4. **締め:**
   - #60 と failure-mechanisms の行を閉じ、spec 22 の Follow-ups 3 を完了にし、handoff を更新する。
   - 条件を満たしたら、handoff の別項目として mutation の並列度を上げる。

**目標にしないこと:** uhd の suite の短縮。手動の時刻は実時間の速さが上限なので、短くはならない。負荷がかかると、busy の間に時刻が止まる分だけ、実時間で見た所要時間は延びることもある（Sol S03）。

## 7. テストと mutation の行

- **時計の規則（`src/host.rs` の中、規則ごとにテスト1つと mutation の行1つ）:** 順序は barrier で固定する。各テストが捕まえる誤った実装は次のとおり。

  | 規則 | 捕まえる誤り |
  |---|---|
  | M1 | 起動前に登録しない、起動の失敗で登録が残る、panic した参加者が登録に残って時刻が永久に止まる、最後の参加者が終わったあとの時刻 |
  | M2・M3 | busy でも進む |
  | M4 | 期限を越える、同じ期限の待ちが複数あるとき、notify と期限が競合したとき、notify と起床の間に時刻が進む |
  | M5 | 実時間より速い（手動の時刻の増分を実時間の増分と比べる。busy の間に止まった分をあとで取り戻す実装も落とす） |
  | M6 | 非参加者の待ちが時刻を止める |
  | M7 | ほかの notify で sleep が早く終わる |
  | §4.7 | 待っている者を busy として扱う、手渡しの直後に時刻が進む |
- **lock の回帰テスト（Sol R01）:** 正の `apply_delay` を持つ configure と、同じ投影への timed update を barrier で重ね、手動の時計で deadlock せずに終わることを確かめる。mutation の行も付ける。
- **Authority の手動の時計での機能テスト（Sol R04・R07）:** `schedule`、`cancel`、Authority の drop、KC-46a の wake の実経路を、手動の時計で確かめる。
- **ホストの domain と relation（Sol S01）:** busy の区間を挟み、`now(HOST_MONOTONIC)` が実時間の進みと一致すること、手動の時計の relation の有効期間が測った瞬間だけであることを確かめる。mutation の行も付ける。
- **HOST の時刻の待ちと予約（Sol T01）:** 実時間と手動の両方の時計で確かめる。それぞれ mutation の行を付ける。
  - 参加者を busy にしたままでも、着いた HOST の時刻の `wait_until` が戻る。
  - 着いた HOST の予約が次の `next_wakeup` で発火し、その HOST の時刻を返す。
  - 再アンカーを挟んでも、HOST の予約の順序が保たれる。
  - root と HOST が同じ判定で due になったときは root が先になる。
  - `InPast` は root の時間軸ごとに判定する（device の root と派生 domain は1つの軸）。
  - 参加者を busy にしたまま、**これから来る** HOST の期限での `wait_until` と `next_wakeup` が、期限に着いたら終わる。root の予約だけを待っているところへ HOST の予約を足す場合も含める。HOST の待ちの timeout を手動の時計で測ってしまう mutant を、これで落とす（Sol U02）。
- **stall の起点（Sol S02）:** 最初の block のあと、busy の実時間の停止を挟む。手動の時刻で `after` が満ちる前には stall が発火しないことを確かめる。mutation の行も付ける。
- **TX の shutdown（Sol R02）:** 遅く進む手動の時計の上でも、最後の BURST_ACK を回収することを確かめる。
- **決定的な再現テスト（Sol R05）:** 新しい FakeFault は足さない。既存の `Watched` のような、試験の中だけの Device wrapper で作る。
  - wrapper は、uhd-tx の `tx_send` の中で std の sleep（busy）を 200 ms 入れる。これで「OS に待たされた参加者」を決定的に作れる。
  - 順序は handshake で固定する：参加者が busy になる → 近い期限がすでに決まっている → 200 ms の実時間の停止 → 同じ呼び出しの中で送信。
  - 確かめること：手動の時計では on time、実時間の時計では late（Fake の late の判定、`device.rs:1075-1101`）。
- **suite 全体を止めない番人（Sol R05）:** 統合テストと、時計と時計の lock の単体テストには、実時間の上限を付ける。時計が止まる mutant や deadlock する mutant は、そのテストの上限で落とす。suite 全体の 900 秒待ちまで止めない。
- **既存の mutation の行:** 置き換えた行に一致していたものは直す。各行が編集後もちょうど1回一致するようにする（AGENTS.md §7）。

## 8. 直す文書

- **`design/18-uhd-radio.md`:**
  - §1 と §6（wall-clock double、実時間の timing）
  - UR-2（`Device::host`）
  - UR-7（Authority がホストの時刻を device の時計から読むこと。上限は実時間のまま。HOST の時刻の `wait_until`・`schedule`・`next_wakeup` は実時間で判定し、root に直さないこと。queue と `InPast` は root の時間軸ごと。本番での変化は §4.6 のとおり。spec 18 の該当行は `design/18-uhd-radio.md:109-112`）
  - TM-16c と KG-9 の「ascending time」「last instant」が root の時間軸ごとであることの明記（design/01 への短い注記。規則の中身は変えない）
  - UR-8（手動のときの relation の有効期間）
  - UR-14・UR-27・UR-29（ms や s がどの時計か）
  - UR-30（`host_delay_ms` は実時間）、UR-34（B5 の stall の起点は `HostClock`）
  - UR-33（実時間か手動か、M1〜M7、§4.7）
  - 末尾の Changes に1行
- **`crates/ezsdr-radio-uhd/tests/README.md`:** §4.6 の書き直し。
- **`plan/maintenance/22-timing-simplification.md`:** Follow-ups 3 を完了にする。
- **`plan/maintenance/failure-mechanisms.md`:** `mech:wall-clock-test-oracle` の行。

## 9. 分担（AGENTS.md §9）

- **段階0:** orchestrator が流して分類する。
- **段階A:** DeepSeek が実装し、自己レビューを最大3回行う。そのあと Opus がレビューし、Sol が独立にレビューする。採否を決めて修正し、gate を通して commit・push する。

## 10. owner の判断

- **D1:** §4.6 の意味の変化を受け入れる（承認済み）。
- **範囲:** 手動の時計にするのは smoke set（§4.4 の分類）だけで、既定は実時間（承認済み）。
- **D2:** §5 の残りの扱い。段階Aの実測のあとに決める。

## 11. 第1回の独立レビューの反映（Sol 6.1、REJECT）

| ID | 内容 | 反映先 |
|---|---|---|
| R01 P0 | 共有 lock を持ったままの時計の待ちが deadlock を作る（`ColdConfig.updates`） | §2 E6、§3、§4.7、M2・M4、§7 |
| R02 | TX の shutdown の 100 ms の窓が一覧から漏れていた。UHD C API の timeout の扱い | §4.3、§7 |
| R03 | `TxBlocks`（永久に止まる）の扱いが決まっていなかった | §4.4（実時間に残す） |
| R04 | condvar の線形化点と、通知経路のテストが不足していた | §4.1、M7、§7 |
| R05 | M5 の検査に実時間との比較がなかった。再現テストの順序。`FakeFault` を増やす必要はない。suite 全体を止めない番人 | M1、M5、§7 |
| R06 | UTC relation と `host_delay_ms` の意味が変わる。直す文書の漏れ | §4.3、§4.6、§8 |
| R07 | `Direct` 系、wrapper の `host()` の転送、Authority のテストの名指し | §4.2、§4.4 |
| R08 | 残りの範囲（event の遅れ）、Lease、server の負荷試験、#60 を閉じる条件と並列度を上げる条件の分離 | §4.3、§5、§6 |
| R09 | 段階Bで、実時間でも終わる待ちと擬似的な遅延を区別する。段階Bは別にレビューする | §5 |
| R10 P2 | 純粋な手動時刻を退ける理由が強すぎた | §3 |

## 12. 第2回の独立レビューの反映（Sol 6.1、REJECT。P0 は解消）

Sol は、R01〜R05 と R07〜R10 を解消と判定した。R06 は、S01 と S02 が残るので未解消とされた。§4.7 は、実際のコードの lock 関係に照らして成り立つと確認された。

| ID | 内容 | 反映先 |
|---|---|---|
| S01 P1 | 手動の時計を `host.monotonic` と `utc` として公開することは TM-16a1・TM-18 に反し、method 名では意味を変えられない | §4.3 の表、§4.6（予約された domain は本物のまま。relation の有効期間を測った瞬間だけにする）、§7。design/01 は変えない |
| S02 P1 | `first_block` を実時間に残すと、RX stall の発火位置が負荷で変わる | §4.3（stall の起点は `HostClock`、診断用の時刻は実時間に分ける）、§7、§8 |
| S03 P2 | 所要時間は「今と同じ」ではなく、延びることもある | §6 |
| 補足 | UHD の内部の mutex は実時間専用と明記する。lock の順序を保つ。時計の lock の単体テストにも実時間の上限を付ける | §4.3、§4.7、§7 |

## 13. 第3回の独立レビューの反映（Sol 6.1、REJECT）

Sol は、S02 と S03 を解消と判定した。S01 も、予約された domain を本物のまま残すことと、relation の有効期間を測った瞬間だけにすることは妥当とされた。点の有効期間は `ClockRelation::new` が受け付け、Manifest の収集も有効期間の長さを要求しない（`relation.rs:135-148,220-229`、`ending.rs:282-305`）。

| ID | 内容 | 反映先 |
|---|---|---|
| T01 P1 | HOST の時刻での待ちと予約を root に直すと、手動の時計が止まっている間、`now(HOST)` は進むのに待ちが終わらず予約も発火しない（TM-16c・TM-16d に反する） | §4.6（HOST の時刻は HOST のまま、実時間で判定する。実時間と手動の両方の時計で同じ規則）、§7、§8 の UR-7 |

**第4回（APPROVE_WITH_CHANGES）。** T01 は解消と判定された。P0 と P1 の新しい指摘はない。

| ID | 内容 | 反映先 |
|---|---|---|
| U01 P2 | 本番の経路で変わることの記述が足りない。「domain ごと」は「root の時間軸ごと」と書くべき。§4.3 の表に `root_tick` が残っていた | §4.3 の表、§4.6、§8 |
| U02 P2 | これから来る HOST の期限を待っている間のテストがない | §4.6、§7 |

## 14. 段階0の記録（2026-10-12、HEAD `27e3c27`）

**実行環境:** sim03（32 コア、負荷なし）。mutation と同じ Docker image（`plan/maintenance/tools/remote/Dockerfile`）を使い、CPU を `--cpus` で制限した。suite は3つ（`-p ezsdr-radio-uhd --test fake`、`-p ezsdr-acceptance --test uhd`、`-p ezsdr-server --test protocol`）で、各条件を3周した。ログは sim03 の `~/ezsdr-stress/out/`。

| CPU | `--test-threads` | fake（128件）の失敗数（1周目・2周目・3周目） | uhd（8件） | protocol（41件） |
|---|---|---|---|---|
| 16 | 128 | 0・0・0 | 全件通過 | 全件通過 |
| 4 | 32（mutation の shard と同じ条件。`REMOTE_CPUS` = 4、スレッド数はコンテナの中で見える CPU 数） | 3・3・3 | 全件通過 | 全件通過 |
| 4 | 128 | 19・19・19 | 全件通過 | 全件通過 |
| 2 | 128 | 28・30・27 | 全件通過 | 全件通過 |

**分かったこと:**

- **mutation の gate は今も影響を受けている。** mutation の shard と同じ条件（4 CPU、スレッド 32）で、毎周 3件が落ちた。落ちるテストは周ごとに違った（`ur_21_late_policies_on_the_device_lead`、`ur_23_…` の6件ほど）。そのため、その時たまたま落ちたテストに mutant の kill を誤って数えることがありうる。
- **16 CPU なら落ちない。** 過去の記録（34〜37件の失敗）は、CPU の少ない環境での結果と考えられる（**INFERRED**）。
- **失敗するのは fake suite だけ。** acceptance の uhd と server の protocol は、どの条件でも落ちなかった。

**分類**（9回の fake のログを `---- … stdout ----` ごとに読んだ）:

| 区分 | 根拠 | 件数（ログで症状が出たファイル数 / 9） |
|---|---|---|
| (a) uhd のスレッドの期限切れ：送信の途切れ | `radio.TX_UNDERFLOW`（`starved`）、`TIMING_PRECONDITION_UNMET` に記録 | 6 |
| (a) uhd のスレッドの期限切れ：受信の溢れ | `radio.RX_OVERFLOW` | 6 |
| (a) か (c)：booking の遅れ | `radio.TIME_ERROR`（`late`、`late_by` が 5〜195 ms）。late policy の結果が期待と違う（`ur_21_…`、`ur_23_…`） | 7 |
| (b) Kernel の非参加者の遅れ | `LINK_BACKPRESSURE`、`EVENTS_DROPPED` はどちらも 0 件 | 0 |
| (d) 実時間の上限 | `KC-21a`、`KC-12a` の失敗は 0 件 | 0 |

- **booking の遅れを (a) と (c) に分けられない理由:** smoke のテストは `tx_at(run, lead)` で `run.now()` から送信時刻を計算し、すぐに `submit` する（`tests/fake.rs:928-942`）。だから、試験のスレッドの遅れ (c) と、uhd-control・uhd-tx の遅れ (a) のどちらでも、同じ症状になる。
- **段階Aの判定:** 段階Aのあとに同じ4条件を流し直す。残った失敗が全部「Session の burst の booking の遅れ」なら (c) と判定し、D2 に回す。
- **(b) と (d) について:** 今の測定では出ていない。段階Aのあとにも出なければ、D2 の対象は (c) だけになる。

## 15. 段階Aの記録（2026-10-13）

**実装の経過。**

1. DeepSeek V4.1 Flash が §4 を実装した。1回目は §7 のテストの半分を残して終わり、2回目で揃えた。機械的な判定（`check.py`）による送り返しは1回だった。
2. Opus と Sol 6.1 が独立にレビューした（O1〜O5、R01〜R10）。
   - 時計が止まっている間の CPU の空回りが見つかった。原因は、共有の condvar による一斉の起床と、止まった時計で測った残り時間を実時間で待つ nap。
   - M4 と M5 の違反（notify で起こされた参加者が busy にならない、mutex を取る前に実時刻を読む）も見つかった。
3. DeepSeek は修正の回で時計の中心に苦戦し、そこで止めた（owner の判断）。
4. 時計の中心（待ち・起床・手渡し）は、Opus のサブエージェントが書き直した。対象を絞った起床、時計が止まっている間は眠る待ち、notify と手渡しでの busy 化、mutex を取ってからの実時刻の読み取り、spawn の巻き戻しである。
5. 残りは DeepSeek が仕上げた。共通のテストの見張り、`Tx::shutdown` の無限ループ、U06 などである。
6. Sol の再レビュー3回（R11〜R22）と、gate で生き残った phase7 の R03 は、Opus が直した。
7. owner の判断で、ほかに3つの変更を入れた。
   - D2 の2件（`ur_33` の受信の打ち切りのテストを、参加者のスレッドで動く単体テストにした。`ur_07` の drop から 50 ms の上限を外した）
   - Authority の最初の anchor の例外をなくした（本番の変更）
   - 参加者でないスレッドで FakeDevice を直接動かしていた残りの3本も、単体テストにした
8. 実装の詳細は、`~/.cache/ezsdr-fixes/2026-10-12-60/` の `stage-a-report.md`（DeepSeek）と `opus-core-report.md`（Opus）にある。

**gate**（全6リスト、sim02/sim03）:

| リスト | 結果 |
|---|---|
| maintenance | 356/356 |
| phase3 | 83/83 |
| phase4 | 34/34 |
| phase5 | 29/29 |
| phase6 | 63/63 |
| phase7 | 165/167 |

- phase7 の G09 は既知の生き残り。
- R03 は、テストが実時間の揺らぎに頼っていたため手動の時計で生き残った。決定的な単体テストに移して kill した。
- 最終の修正の回で変えた行（maintenance 8、phase7 8、C01）も kill した。

**§14 と同じ負荷試験**（sim03、mutation と同じ image、各条件3周）:

| CPU | `--test-threads` | fake（130件）の失敗数 | uhd（9件） | protocol（42件） | fake の所要時間 |
|---|---|---|---|---|---|
| 16 | 128 | 0・0・0 | 全件通過 | 全件通過 | 45〜46 s |
| 4 | 32 | 0・0・0（段階0は 3・3・3） | 全件通過 | 全件通過 | 49 s |
| 4 | 128 | 0・0・0（段階0は 19・19・19） | 全件通過 | 全件通過 | 45〜46 s |
| 2 | 128 | 0・0・0（段階0は 28・30・27） | 全件通過 | 全件通過 | 48〜49 s |

**判定。**

- **#60 を閉じる条件**（(a) が 0 件で、通常の実行が全部通る）: 満たした。
- **mutation の並列度を上げる条件**（負荷試験の失敗が全部で 0 件）: 満たした。(b)〜(d) はどれも出なかったので、段階Bは要らない。
- **所要時間:** 段階0の 35 s から 45〜49 s に延びた。M4 で、Provider の 1 ms の polling の期限ごとに時計が止まるためである（§6 のとおり。短縮は目標ではない）。

**途中で分かった規則の違反と対策**（次の実装の依頼に効く記録）:

- **作業ツリーへの直接の実験:** DeepSeek は、指示書で禁じたあとも、作業ツリーのファイルに mutant や実験を当てては戻すことをくり返した。実験はリポジトリの外のコピーで行うよう、毎回の指示書に書く必要がある。
- **負荷用ループの放置:** DeepSeek は負荷用のループを Mac に放置した。見張り（`watchdog.sh`）で止めている。
- **セルフレビューの扱い:** DeepSeek は、セルフレビューをバックグラウンドで起動したままセッションを終えることが3回あった。
- **見張り自身の誤動作:** 指示書の文面に照合の文字列そのものが含まれていて、見張りが実装役のプロセスを止めた。見張りは、シェルのプロセスだけを対象にするよう直した。

## handoff からの記録（2026-10-14 に移動）

handoff.md に 2026-10-14 まであった記録を，そのまま移した（AGENTS.md §9：記録は作業の置き場に1か所）．当時の文なので，今の規則は accepted specs と照合すること．

2. ~~**#60**~~ → **完了（2026-10-13，[plan 30](30-fakedevice-manual-time.md) §15）**：smoke set の FakeDevice は手動の時計（`crates/ezsdr-radio-uhd/src/host.rs`，規則 M1〜M7 と時計の lock）で動き，uhd のスレッドが busy の間は時刻が止まる．§14 の負荷試験（sim03，16/4/4/2 CPU × 128/32/128/128 スレッド，各3周）で fake・uhd・protocol とも**失敗 0**（段階0は 0/3/19/28）．gate は全6リストで既知の G09 以外 kill．所要時間は fake suite 35 s → 45〜49 s（M4 で 1 ms の polling ごとに時計が止まるため，短縮は目標外）．分担：DeepSeek が実装→Opus と Sol のレビュー→時計の中心は Opus サブエージェントが書き直し→残りを DeepSeek→Sol の再レビュー3回と R03 を Opus．owner の判断：D2 の2件，Authority の最初の anchor の例外を削除（本番の変更），参加者でないスレッドの FakeDevice 直接テスト4本を単体テストへ．実装の報告は `~/.cache/ezsdr-fixes/2026-10-12-60/`（`stage-a-report.md`，`opus-core-report.md`；判定スクリプト `check.py`，送り返し `drive.sh`，見張り `watchdog.sh`）．**mutation の並列度も上げた（2026-10-13）**：`remote_mutate.py` が複数リスト（`all`）を1回で流し，実行ごとの作業ディレクトリ〈`~/ezsdr-mut/<tag>`〉を使う．`all sim02:8,sim03:8` で全リスト約8分（以前は約17分），結果は従来の gate と同じ（G09 以外 kill）．**実装役への指示書に毎回書くこと**（今回の違反から）：作業ツリーに mutant や実験を当てない（リポジトリの外のコピーで），Mac で負荷をかけない，セルフレビューはフォアグラウンドで終わるまで待つ．
