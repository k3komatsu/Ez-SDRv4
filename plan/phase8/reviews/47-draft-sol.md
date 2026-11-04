## Findings

**DRAFT NEEDS CHANGES：12件。** owner の機能選択は維持できます。主な不足は、その選択を既存の admission・booking・非同期送信へ落とす際の境界条件です。

以下、`D:行` は `~/.cache/ezsdr-fixes/2026-11-04-n46/47-draft.md` を指します。

### 1. §3 Q1：verb 名だけでは repeat 差替えを識別できない

**問題**：`switch_repeat` を普通の `CompileRule::TxBurst { repeat: true, … }` に登録するだけでは、Provider に届く時点で `start_repeat` と区別できません。

**証拠**：
- `crates/ezsdr-kernel/src/session.rs:399–409`：compile 後に残るのは waveform、repeat、at、late policy、metadata。verb 名は残りません。
- `design/07-radio-model.md:147–155`、`crates/ezsdr-radio-uhd/src/provider/control.rs:1288–1290`：現在は非空 metadata を拒否します。

**訂正**：案(i)を採用しつつ、Vocabulary 所有の metadata marker を定義してください。例えば `radio.repeat_operation = "switch"`。通常 burst は空、差替えはその marker だけ許可し、未知の組合せは拒否します。RM-12だけでなくRM-13・MR-16/MR-32・UR-21の変更が必要です。

### 2. §2.1・Q1：選択した境界を action log に残す道がない

**問題**：D:39 は「記録する」としか書いておらず、46 §8 の「action log と記録に残す」を満たしていません。RS-19 の coercion が記録するのは、Provider が後で選ぶ周期境界ではありません。

**証拠**：
- `plan/phase8/46-streaming-feature-ledger.md:226`。
- `crates/ezsdr-kernel/src/session.rs:365–377`：ASAP の coercion は compile 時に生成。
- `crates/ezsdr-kernel/src/coordinator/pipeline.rs:1034–1039`：Provider の booking を待っても、log に書く coercions は admission 側で既に作ったもの。
- `crates/ezsdr-kernel/src/session.rs:90–123`：`Outcome`/`LogEntry` に Provider の選択結果を戻す欄はありません。

**訂正**：少なくとも `{ action_id, predecessor, selected_at, old_period, new_period, outcome }` の Vocabulary 側結果記録を定義し、action log の `dispatched` と結び付けてください。これを「action log に残す」の実装とするなら、その対応関係を明記する必要があります。**選択時刻そのものを既存 LogEntry の coercion に入れられる、とは書けません。** literal な LogEntry 内記録を必須にするなら、結果を戻す仕組みを別途設計する必要があります。

### 3. §2.1：TX clock の変更条件と origin が46 §8と違う

**問題**：D:33 は「空の stream への rate change」で clock を替えるとしています。owner の決定は、設定を伴う burst の **booking 時** に新 clock を登録し、その burst の開始を origin とするものです。frequency/gain/antenna だけの場合も、rate update 単体の受付時ではありません。

**証拠**：
- `plan/phase8/46-streaming-feature-ledger.md:224`。
- D:16、33、60には、この booking と origin、前 burst 終了からの start lead が揃っていません。
- 現状は `crates/ezsdr-radio-uhd/src/provider/core.rs:1010–1033` と `design/07-radio-model.md:255` が、変更の booking と clock 登録を結び付けています。

**訂正**：
- 設定付き burst の booking を新 clock の登録点とする。
- origin は、その burst に決まった実際の開始。
- 前 burst の終わり＋TX start lead、設定完了からの readiness、現在の safe front を満たす。
- Kernel が旧 clock に解決済みの `at` を、Provider がどの手順で新 clock に移すかを規定する。
- RM-31/RM-32の再検査、X310 TX antenna の現行制約もここへ明記する。

### 4. §2.1：RX の「停止中」が予約済み start を含めて定義されていない

**最小系列**：

```text
RX Stop
start_rx(at=E) を予約する（設定Aを保存）
Eより前に cold update B を受け付ける
Eで start_rx が設定Aを使う
```

**誤った結果**：受け付けた B が次の開始に入らない。逆に snapshot を後から変更すれば、開始処理中の設定が書き換わり得ます。

**証拠**：
- D:28 は「次の start_rx が効くまで」を停止中とします。
- `crates/ezsdr-radio-uhd/src/provider/control.rs:493–509`：start の booking 時に `config_at(e)` の snapshot を作ります。
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:502–540`：実際の configure/start は後で owner が行います。

**訂正**：`Stopped` と `StartBooked/Configuring` を区別し、設定の受付と start の予約を同じ状態遷移で確定させてください。最小案は、**予約済み start があれば別の cold update を拒否**することです。将来時刻の update→start の順序も、同時刻の受信順だけでなく予約順として固定する必要があります。

### 5. §3 Q2：「host 上で空」と「装置への設定変更が安全」が混同されている

**最小系列**：

```text
有限burstの最後のpayload/EOBを、装置時刻より先にsend
host側ではopen burstもheld burstもなくなる
rate/frequency changeを受付・configure
装置では前burstがまだ出ている
```

**誤った結果**：前 burst の末尾を新設定で処理・送信する可能性があります。「次の開始が50 ms後」は、**設定を早く適用しない保証にはなりません**。

**証拠**：
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:661–760`：send 完了時に host の burst を終えますが、payload は in-flight window 内を先行送信します。
- `design/18-uhd-radio.md:246–254`：EOB padding と payload の終わりは別です。
- **VERIFIED**：[UHD `set_tx_rate`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1999-L2018) は DUC の input rate を変更し、前 burst の放射完了を待つ処理ではありません。
- **VERIFIED**：[DUC `set_interp`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/duc_block_control.cpp#L569-L591) は interpolation・M/N・time increment を書きます。
- **VERIFIED**：[FPGA `duc.v`](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/fpga/usrp3/lib/rfnoc/duc.v#L71-L108) には active 中の interpolation 切替えを clear まで保留する仕組みがあります。これは、host の空判定だけで全設定が安全になる証明ではありません。

**訂正**：設定を staging する受付条件と、装置へ apply できる条件を分離してください。旧 burst の EOB・padding を含む drain、recovery、report generation を扱い、設定完了から次の開始までの lead も保証します。具体的な drain の安全条件・余裕は **INFERRED、実機確認対象**です。

### 6. §2.4・Q6：Provider の拒否では、現在の Python は `Rejected` を投げない

**最小系列**：

```text
RXが流れている
許容範囲内のfrequencyを設定
Kernel admissionは通る
ProviderがCOMMAND_REJECTEDを出す
```

**誤った結果**：D:61 は `Rejected` を約束しますが、Python の呼出しは成功を返し、`effective` には拒否された値が見えます。

**証拠**：
- `crates/ezsdr-kernel/src/coordinator/admission.rs:283–286`：dispatch 時点で admitted configuration を更新。
- `design/06-kernel-coordinator.md:335` KC-27：Provider が拒否しても、その値は configuration に残る。
- `python/ezsdr/session.py:110–113`：`_admitted` は Kernel log entry の outcome しか見ません。

**訂正**：Kernel の `Outcome::Rejected` と Provider の `radio.COMMAND_REJECTED` を区別してください。Easy API が後者でも例外を出すなら、既存の action id で booking refusal を照合する手順と完了条件が必要です。KC-27を変更せず、読み戻しが「admitted value」であることも残してください。

### 7. §1・§2.4：案(A)の複数 Action に、設定の寿命・部分失敗の規則がない

**問題**：同じ `at`・受信順は、複数の update と開始を一つの操作にはしません。

**最小系列**：

```text
停止中に frequency=A を送る → 成功
gain=B を送る → admissionで拒否
startは送られない
後の別のstartが、残ったAを意図せず使う
```

または：

```text
TXの設定updateを受付
別originのburstが先にbookingされる
元の呼出しのburstが拒否される／別設定になる
```

**証拠**：
- D:20、59–60。
- `crates/ezsdr-kernel/src/coordinator/pipeline.rs:995–1035`：一つの submit 内は保護しますが、Python が続けて出す別 submit 間の transaction ではありません。
- `crates/ezsdr-radio-uhd/src/provider/control.rs:566–574,1233–1343`：設定 projection と burst booking は別処理です。

**訂正**：案(A)は維持して、設定を「次の開始に使う staging」として定義し、いつ snapshot に固定し、何が予約を取得し、失敗・Stop・loss・終了で何を残すかを書いてください。Python は全引数・waveform を送信前に検査し、途中の拒否後に開始を送らないことも必要です。原子的な all-or-nothing は、現在の案では約束できません。

### 8. §2.1：重複拒否は、開始時刻の集合だけでは実装できない

**最小系列**：

```text
有限burst A=[100,200) を予約
burst B=[150,250) を予約
開始時刻は違うので既存held集合を通過
```

別系列：

```text
隙間に予約したburstが、dispatch時のSendAsapで後ろへ移動
その移動先が別の予約済みburstと重なる
```

**誤った結果**：一般の preemption が残るか、予約済みの後続 burst を切ります。

**証拠**：
- `crates/ezsdr-radio-uhd/src/provider/control.rs:1336–1343`：共有予約は現在 `(clock.domain,start)` の重複だけ。
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:987–1015`：first dispatch でも開始が動きます。
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:498–524`：既存 held burst も実行時に preempt します。

**訂正**：active・owner queueへ移送中・held の全予約区間を検査してください。有限 burst の exclusive end での隣接だけ許し、repeat は専用差替え以外では無限区間とします。LatePolicy による移動後も同じ検査が必要です。

### 9. §2.1・§2.3：repeat 差替えの予約と送信の間に必要な同期が未定義

**系列と誤った結果**：

| 最小系列 | 誤った結果 |
|---|---|
| control が境界Bを計算 → 通知が遅れる → TX owner がB以後を旧波形でhand → 差替え到着 | 記録したBで切り替わらない、またはsampleを二重送信 |
| 差替えを予約 → Stopが取消し → 遅れて差替えがownerへ到着 | 停止した送信が復活 |
| 差替えを予約 → loss / Provider::stop → 差替えの処理 | 終了後のsend・device call |
| 同じrepeatへ差替えを二つ予約 | 同じ境界に衝突、または後者が誤ったperiod/originで計算 |
| 差替えを予約 → F8で前repeatをabandon → recovery完了 | 存在しない前repeatへ「隙間なく続く」として送信 |

**証拠**：
- `crates/ezsdr-radio-uhd/src/provider/tx.rs:276–326`：command queue と各 send pass は非同期。
- 同 `:480–509,579–603`：window と既知の held cut を使って送信。
- 同 `:1106–1112`：Stop は held/later を取り消す。
- 同 `:307–311`、`design/18-uhd-radio.md:247`：recovery は別の lifecycle。

**訂正**：境界選択と送信上限の公開を、handed cursor を進める owner と直列化してください。選んだBを後で動かさず、Stop/loss/終了を跨ぐ古い予約は取り消す。同一 repeat identity に対する pending switch は一つに限定するのが最小です。F8で前repeatが失われた場合の差替えの結果も規定してください。

### 10. §4：段階1は「UHDに同じ拒否だけ」では緑にならない

**問題**：段階1で共有 class と Mock の正の動作を変え、UHD の対応を段階2・4へ残すためです。

**証拠**：
- `crates/ezsdr-radio-uhd/src/provider/control.rs:437–447`：共有 `update_class` に従うと frequency/gain が既存 `book_cold` へ入り、現行の対象選択 `:1059–1079` にはその leaf がありません。
- `crates/ezsdr-radio/tests/generator/mod.rs:16–78`：VH の生成系列そのものが Timed / live cold / membership change を含みます。
- `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:87–120`：旧 floor と timed rule を期待。
- `crates/ezsdr-acceptance/tests/parity.rs:587–936,1405–1644`：join、floor、更新の parity。
- `python/tests/test_easy_api.py:623–630`：普通の repeat を二度呼ぶ既存試験も新しい重複拒否に抵触。

**訂正：縦に切る段階を推奨します。**

1. 初期 membership の設定経路と fixtures を用意。
2. membership 固定を両 Provider・VH・Python・spec に同時適用。
3. RX の新しい設定/start規則を両 Provider・VH・Pythonへ適用。旧RX機構の除去もこの段階内。
4. TX の空判定・設定付きburst・clock規則を両 Providerへ同時適用。
5. 重複拒否と repeat 差替えを両 Provider・parity・Pythonへ同時適用。
6. 到達不能になった共有機構を整理。

各段階で生成器・oracle・対応する mutation family も更新し、単に旧系列を skip して緑にしないでください。

### 11. §2.4・§2.5：初期 membership の入口がなく、Visionの本文も影響する

**問題**：D:58 の「BindingProfileで決める」だけでは、現在の Session にTXを使わせる具体的な入口がありません。

**証拠**：
- `design/07-radio-model.md:310`：TX channel は既定で全て stream none。
- `crates/ezsdr-kernel/src/session.rs:487–490`：Session の暗黙 Spec の resource requirements は空。
- `design/16-easy-api.md:45–65`：現在の default profile に membership 設定はありません。
- `python/examples/minimal.py:9–12` は connect 後の `.stream = 0` に依存。
- Visionにも同じ操作があります：
  - `design/vision/01-purpose-and-core-boundary.md:53–72,110`
  - `design/vision/11-boundaries-invariants-and-roadmap.md:167–171`

**訂正**：prepare 前の初期設定をどの既存／追加フィールドで渡すか決め、default Session とカスタム profile の双方を更新してください。Spec側の membership requirements は残ります。**Vision本文を一切変えないというD:9、66は撤回し、少なくとも§3・§66の例を新APIに合わせる必要があります。**

§27のclass集合、§58 #9/#16、§61の4行動、§65の不変条件そのものは、この機能制限だけでは変更不要です。ただし§3の「一つのEasy API call＝一つのlog entry」という説明も、案(A)の複数 submit と整合させる必要があります（同ファイル`:106`）。

### 12. §4：rate変更の実機確認を最後に置くと、ownerの条件を満たす前に機能を受理する

**問題**：46 §8 は「開いたままの streamer で rate を変える」ことを、RX/TXとも実機確認条件にしています。段階5まで通常機能として有効にしてはいけません。

**証拠**：
- `plan/phase8/46-streaming-feature-ledger.md:221,224`。
- D:82–86。
- **VERIFIED**：[UHD RX rate](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1314-L1333)、[TX rate](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp#L1999-L2018) の setter は存在します。しかし、その存在は当該 stop/drain/restart 系列の保証ではありません。

**訂正**：RX/TX各実装段階の受理条件に、対応する実機確認を置いてください。未確認の間はその部分を未受理として残す。benchでは、新rateだけでなく旧burst末尾、時刻、初回sample、read-back、streamer非再生成も確認対象です。

---

## Removal inventory

**区別**：**削除**＝その意味の処理を除去可能。**改訂**＝残存機能が使うため削除不可。記載した広い関数領域を丸ごと消す意味ではありません。

### 規則と named tests

長い Test 行は、その行に列挙された**全 named tests を対象集合**としています。prefix だけで無条件に削除してよい、という分類ではありません。

| Spec | 削除／改訂する規則 | Named tests の対象 | 残存機能による使用 |
|---|---|---|---|
| **07：宣言・能力** | **改訂** RM-1、RM-4、RM-6、RM-20、RM-27、RM-29、RM-35。Q3でqueue/release能力を除けばRM-10/11/22も整理 | `rm_04_the_key_table_is_exactly_the_declared_one`、`rm_27_the_layout_builder_keys_are_declared`、`rm_20_schema_freeze`、RM-6の全 named tests（`:91`）、RM-35の全 named tests（`:347`） | min lead、startup、start leads、in-flight、rate/grid、RF検査は残る。queue/release能力だけを分離 |
| **07：更新・計画** | **改訂** RM-7、RM-12〜16、RM-21、RM-25、RM-30〜33。**withdraw** RM-34。timed tune専用 caller消滅後にRM-36もwithdraw候補 | RM-21全名（`:236`）、RM-25全名（`:261–263`）、**RM-34全名（`:328–330`）**、`rm_36_covering_boundaries`、`rm_12_the_verbs_compile_as_declared`、`rm_31_rm_32_a_layout_instance_s_violations_are_judged_at_runtime` | Stop/start_rx、cut、refusal、prune、clockは残る。RM-34のmove/releaseは残存機能に不要 |
| **07：TX** | RM-13 metadata、RM-14 late judgement、RM-15 overlap/preemption、RM-16 cold cancellationとtimed-command settlementを改訂 | `rm_14_the_recorded_requested_target_is_the_requested_at`、`rm_14_the_handed_burst_keeps_its_requested_at`、RM-16全名（`:173–174`）。cold cancellationのみのケースは除去／拒否試験へ | exact continuation、Stop、loss、Provider::stopの取消しは残る |
| **09：初期化・時刻** | **改訂** MR-3/4、MR-9/10、MR-12、MR-15〜18、MR-20、MR-25、MR-27、MR-29 | `mr_09_each_transmit_stream_has_its_own_clock_and_line`、`mr_18_hardware_timed_updates`、`mr_18_a_scheduled_pair_is_checked_when_it_applies`、MR-18全名（`:155–159`）、MR-20 refusal全名（`:169`）、MR-25全名（`:184`） | 初期化、Stop/start_rx、loss、拒否後のhaltは残る。live cold再構成・timed queue固有の期待を除去 |
| **09：membership** | **withdraw** MR-40。MR-38/39のmove/entry/exit文と関連Testを改訂 | **MR-40の全名（`:239`）**、`mr_32_a_logical_transmit_channel_follows_its_move_to_another_stream`、`mr_40_a_move_to_and_from_a_linkless_receive_stream_never_publishes_there` | 固定複数stream、linkless stream、channel順、streamごとのclockは残る |
| **09：RF模型** | **改訂** MR-27、MR-32〜35。timed phase分類・move tune・live updateを除去 | MR-32全名（`:210`）、MR-33全名（`:212`）、**MR-34全名（`:222–226`）**、`mr_35_a_cold_receive_change_samples_the_field_on_the_new_clock` | gain/frequency/phaseのsample時刻履歴、noise scaling、path delay、clipping、seed再現性は残る |
| **18：timed設定** | UR-24のrelease/queue/floor、eligibility、debt/drain/flush/left、timed TX tuneを除去し、新しいstaged設定規則へ。UR-26のrestore・timed settlementも除去／改訂 | **UR-24全 named tests（`:278–293`）**、UR-26のrestore/hand-over/settlement群（`:338–344`）。`ur_24_a_loss_during_rx_receive_finishes_without_recovery_calls`等のloss系列は書き直して残す | 普通のRX stop、late-start再発行、loss、設定/read-backは残る。timed update固有の義務は不要 |
| **18：構成・ownership** | **改訂** UR-14/15、UR-17、UR-23、UR-25、UR-30、UR-36/37。UR-25のRelease/FromNone/moveだけ除去 | **UR-25全名（`:311–318`）**、UR-36/37のnamed tests、`ur_34_*`のmove/entry/exit/release全ケース | owner claim、Free、phase、report-read排他、generation、configure refusal、Stop/start_rx、F8は残る |
| **18：TX予約・送信** | **改訂** UR-21〜23、UR-28。一般preemptionを除去、差替えへ置換 | UR-21全名（`:244–245`）、UR-23全名（`:257–262`）。特に `ur_21_booking_receipt_order_changes_preemption`、`ur_21_a_send_asap_move_is_reported_once_across_handovers`、`ur_23_txs_late_preemption_uses_the_handed_frontier` を新規則へ | continuation、held tail、safe front、first-dispatch lateness、F8、async correlationは残る |
| **18：Device・bench** | UR-2/3/33のtimed apply・RX flush模型はcaller次第で縮小。UR-9/10/30/34は能力・測定・benchを改訂 | `ur_22_from_none_open_waits_for_t3_flush_ack_before_timed_tx_dsp`は旧経路。`ur_22_t3_flush_ack_is_a_timed_dsp_barrier`等は新設定/recoveryの同期へretarget。`hw_ma1_phase_after_a_pre_start_tune`、H4 timed-gain測定群も再設計 | T3、DUC timestamp/framing、EOB、ACK、underflow模型は消さない。raw Device characterizationを残すなら、そのtimed模型も必要 |
| **16** | **改訂** EA-9、EA-16〜19。必要ならEA-10の初期設定説明。EA-11のadmission結果とProvider結果を区別 | `test_v3_parameters_through_attributes`、`test_ea_16_channel_keys_and_stream_membership`、`test_ea_16_stream_one_actions_target_their_nodes`、`test_ea_16_repeat_uses_stream_membership_without_setting_it`、`test_ea_17_a_capture_across_a_rate_change_is_refused`、`test_ea_16_rx_stop_then_start`、`test_ea_19_the_examples_run`。初期membershipを使う他のsetupも変更 | capture across clocks/gapsの拒否自体は残る。Stop/start、aligned、capture、波形検査は残る |

参照先はすべて `design/07-radio-model.md`、`design/09-mock-radio.md`、`design/18-uhd-radio.md`、`design/16-easy-api.md` の該当行です。

### Production regions

略号：`R=crates/ezsdr-radio/src/`、`M=crates/ezsdr-mock-radio/src/`、`U=crates/ezsdr-radio-uhd/src/`。

| 領域 | 削除／改訂対象 | 残存機能による使用 |
|---|---|---|
| `R/lib.rs:116–133,160–230`、`R/channels.rs:723–744` | cold cancellation理由、key classes、verbs、metadataの規則を改訂 | Action routing・初期configuration判定は残る |
| `R/lib.rs:530–561,781–788,804–826`、`R/x3xx.rs:380–413`、`R/channels.rs:289–343,376–422`、`R/measurement.rs:294–379` | queue/release能力・start-lead check・schema/比較規則をQ3に合わせ変更 | その他のenvelope値、初期能力、測定比較は残る |
| `R/timeline.rs:193–199,423–428` | `covering`、`timed_instant`を削除候補 | `covering`のproduction callersはMock timed phaseとUHD timed releaseだけ。Stop/start_rxは直接使わない |
| `R/timeline.rs:288–335,430–793` | channel release、move、依存伝播、move pruneを削除／単純化 | `Lines`全体を削除とは限らない。line集合・固定streamのbook/refuse/pruneはまだ使用 |
| `R/timeline.rs:229–287,378–419,853–1039` | live coldとinstance floorを改訂。Startが新設定を持つ形へ | cut、delivered floor、Start/Stop/End、refusal、lattice、pruneは残る |
| `R/timeline.rs:795–813` | preemption用boundのcallerを改訂 | Stop後・EOB後のsafe frontとfirst dispatchにも使用。丸ごと削除不可 |
| `M/lib.rs:154–343` | timed pending、cold/move分類、TX予約状態を縮小／改訂 | RX/TX state、held/open burst、front、pending start設定は残る |
| `M/lib.rs:1024–1188,2261–2321` | covering、timed tune、coherence更新を削除／改訂 | `untimed_tune`、segment coherence、channelの値の履歴は残る |
| `M/lib.rs:1330–1453,1514–1636` | segment開始・clock switch・projectionを改訂 | Stop/start_rxと設定付きTX clockが使う |
| `M/lib.rs:1715–1972` | burst受付、front、次burstによる切断を改訂 | exact continuation、repeat、Stop、sample accountingが使う |
| `M/lib.rs:1974–2260` | timed queue/floor、move、live cold/refusal経路を除去・再構成 | staging、coerce、RM-31/32、開始時refusalは残る |
| `M/channel.rs:13–111`、`M/lib.rs:2323–2435` | RF値・radiation segment・work選択を新規則へ | 過去sampleを正しく計算する履歴は必要。gain/frequency timelineの全削除は不可 |
| `M/profile.rs`、`M/lib.rs:2500–2803` | envelope、初期membership、arm/startの設定を改訂 | prepare/arm/T0は残る |
| `U/provider/control.rs:41–183,221–350` | Timed/Given/Reach/Handed、held/released履歴、ColdConfigのtimed更新融合を削除／縮小 | immutableな開始設定snapshotとconfigureは残る |
| 同 `:377–413,522–542,564–952` | restore計画、held projection、timed booking/release/settlementを削除 | `config_at`はstaged設定用に改訂。終了時のpending設定結果記録は必要 |
| 同 `:964–1228` | from-none、move、instance floor、live cold再構成を除去 | 新しい停止中受付・予約・全configuration検査へ置換 |
| 同 `:420–510,1233–1343` | dispatch、start予約、burst予約・marker・interval検査を改訂 | Stop/start_rx、通常burst、差替えが使う |
| `U/provider/core.rs:92–99,127–144,163–178,239–261,277–327,353–371` | RxEntry、Restore、debt/left/latest、eligibility、flush/consumeを削除 | `RxChain`全体は残す。Off/Armed/Streaming/Uncertain/Lost、intent、segment identityはStop/start/lossで使用 |
| 同 `:640–670,899–958,1663–1731` | claim_release/from_none、channel競合・release待ちを削除 | `claim_owner:628–638`、`begin_read:672–696`、`give_back:701–712`等はF8/report/shutdownで残る |
| 同 `:802–870,960–1041,1373–1421,1583–1661,1733–1812` | planner/device record/configure/open/registerを単純化・改訂 | loss、Provider::stop、start_rx、read-back refusal、clock登録は残る |
| 同 `:1428–1581` | timed設定とのbarrier callerを改訂 | **T3/open/F8/report同期は残る** |
| `U/provider/rx.rs:268–488,856–885` | timed DDC drain/flush/restoreを除去。普通のcut/finish部分を残す | untimed stop、quiet deadline、loss、Provider::stopは残る |
| 同 `:502–591,593–854` | 開始設定・refusal・deadline receive・sample処理を改訂 | start_rx、cut、missed start、overflow/sequence、lossが使用 |
| `U/provider/tx.rs:468–603,979–1094` | 一般preemptionを除去、差替えとoverlap拒否へ | `dispatch_start`、safe front、送信上限、隣接continuationは残る |
| 同 `:1115–1222` | live cold/membership switchを改訂 | 設定付きburstのclock/configure/refusalに必要な部分は残る |
| 同 `:101–207,609–890,910–964,1097–1112` | callerの適応は必要だが削除対象ではない | **held tail、EOB、F8、generation、Stop/shutdown**が使用 |
| `U/provider/mod.rs:345–356,443–680,683–745` | timing検査、初期化、空stream owner、終了時処理を改訂 | prepare、T0、bounded join、cleanup、loss安全性は残る |
| `U/device.rs:507–580,602–735,1407–1519`、`U/uhd.rs:526–588,887–972` | timed設定模型・raw tune・applyのcallerを棚卸し | untimed apply、timed RX start、**T1/T2/T3、DUC framing、F8**は残る。低レベルDevice全体の削除は不可 |
| `python/ezsdr/session.py:110–113,187–255,276–293,375–400,568–580` | setter、start、repeat/send/switch、Provider refusalの照合を改訂 | waveform変換、channel数検査、Target、capture、Stopは残る |
| `python/examples/minimal.py:9–12`、`python/examples/sweep.py`、`python/tests/test_easy_api.py:60–72` | post-connect membership設定をprepare前設定へ移動 | examples・fixtures自体は残す |

### Mutation-row families

**family単位の処置です。広いID範囲を丸ごと消してはいけません。** 特に `cold`、`front`、`timed` を名前に含むだけでは、残存機能か判断できません。

| List / family | 削除・retargetするもの | 残すもの |
|---|---|---|
| `plan/phase3/tools/mutations.json` | phaseの旧timed期待：M31–M33、M46、M55、M63。live cold：M51、M68–M70。旧open-burst拒否：M72、M78、M80–M82 | gain/frequency履歴・LO rotation・Stop・path delayの意味は新start/burst系列へretarget。単純削除しない |
| `plan/phase6/tools/mutations.json` | P12のkilling testをStop→設定付きstartでのclock跨ぎへ。P01/P11等のsetupを初期membershipへ | P06の例外検査、captureの時刻・validity・gap・deadline、波形channel数 |
| `plan/phase7/tools/mutations.json`：timed | U20/U21/U37/U38、R05/R21、C02 | 純粋な低レベルDevice characterizationを残す場合は対応行も維持 |
| 同：構成 | V06、U22/U39–U41、R04/R07、B15/B22、U23を新start/burst系列へ | U40のrate read-back refusal、R12–R15のRX stop/reuse/終了の意味 |
| 同：preemption | U17/U36/U44、R06、B03/B04、C03を専用switchまたはoverlap拒否へ | B08–B10、B14/B16、B18–B21、B25–B32、B41：continuation/EOB/ACK/失敗処理は残る |
| `plan/maintenance/tools/mutations.json`：RM-34/MR-40/UR-34 | move、join/leave、from-none、release ownership、move pruneの各family（P81–P121ほか、P1060–P1062、P1079–P1081、P1275/P1276） | 固定構成の複数stream・初期channel/grid/board検査は残す |
| 同：RM-21/RM-25/MR-18/UR-25 | live cold、instance floor、old future-clock cancellation、refused move群。P1052–P1069もcold専用部分を分離 | refused **start**、cut、prune、loss、clock identityはretargetして残す |
| 同：RM-36/MR-34/UR-24 | P1022–P1037、P1075、timed release/queue/floor/target projection群 | deterministic/random phase、seed、per-channel設定の検査は残す |
| 同：D9b-2/L1 | P1211–P1295内のdebt/drain/restore/eligibility/handoff/left family | P1253、P1260–P1262、P1264、P1269、P1289–P1292、P1295等は、普通のstart/stop/loss/cleanupの意味を残すよう再評価 |
| 同：TX window/preemption | P629–P674、P1108–P1118、P1165/P1166等の一般preemption/cold cutをretarget | full-chunk wait、continuation tail、Stop front、sample accountingは残す |
| 同：D9b-1a/L2 | P1170のkilling testはlive retuneでなくT1単独へ。P1175–P1178は新設定/open barrierへ。P1300/P1301等の「次segmentでhalt解除」を新設定付きburstへ | P1171–P1210、P1296–P1315のEOB、T3、recovery、generation、cursor/tailは基本的に残る |
| 同：測定/H4・能力 | P1070–P1074、P1139、P1142–P1158、P1316–P1318などを新しい制御呼出し測定・能力集合へ | throughput、TX headroom、underflow、priority、観測の時刻対応は残す |

**追加で同じ段階へ含める試験領域**：

- `crates/ezsdr-radio/tests/generator/mod.rs:16–78`：旧操作集合の再設計。
- `crates/ezsdr-mock-radio/tests/timeline.rs:26–27,304`：`rm_26_the_timeline_against_mockradio`。
- `crates/ezsdr-radio-uhd/src/provider/control/differential.rs:44–120`：booking oracle。
- `crates/ezsdr-radio-uhd/src/provider/rx/differential.rs:53–98`：実owner・refusal・readiness oracle。
- `crates/ezsdr-acceptance/tests/parity.rs:475–936,1389–1644,1751–1908`：membership/floor/live-update群を除去・新系列へ置換。
- 同 `:1646–1750,1790–1826,1910–2040`：Stop/loss/continuation/repeatの残存系列は維持。
- `crates/ezsdr-acceptance/tests/v58.rs:496–580`：§58 #16の受付試験を新設定経路へ。
- Python suite、serverのdefault-profile試験、rehearsal/measurement fixturesも初期membership変更の対象です。

---

## Q1–Q6：独立した回答

### Q1：repeat差替えのAction

**推奨：案(i)、TxBurst＋Vocabulary metadata marker。Kernel Action形状・admissionは変更不要です。**

RS-19は `at=None` を残しません。`max(now + instance lead, latest live SampleClock origin)` を埋めます（`design/04-run-and-session.md:180`、`pipeline.rs:1143–1168`）。その後、admissionがstreamのSampleClockへ変換します（`admission.rs:145–176`）。

Providerはmarkerを読み、その `at` を実行時刻そのものではなく要求の下限として扱い、受付時の

```text
now + in-flight window + device lead
```

以後の最初の `origin + k·N` を選べます。丸めはVocabularyの意味として規定できます。

ただし必要なのは、**marker、既存repeat identity、境界の予約、handed cursorとの直列化、結果記録**です。RS-19のcoercionが選択境界を記録してくれるわけではありません。

`Command` に波形bytesを独自に載せる案(ii)は、既存のinput store・waveform validationを再利用できず、不要な別経路になります。

### Q2：TXの「流れていない」

**推奨：予約受付上の空と、装置をconfigureできる空を分ける。**

- 受付時：active・held・owner queue移送中・pending switch/configurationの予約がない。
- apply時：旧device burstが閉じ、そのEOB/paddingを含む旧sampleの処理が終わり、recoveryとの排他が成立している。
- 新開始：前burst終端＋TX start lead、および設定完了後の必要leadを満たす。

UHD setterがrateを書き換えること、FPGAがinterpolation変更を一部保留することは **VERIFIED**。しかし「hostが空なら旧sampleに影響しない」「50 ms後に始めれば設定を今入れてよい」は **未検証のINFERRED** です。ソースはFinding 5の3 URL。

開いたstreamerの再利用自体はownerの実機確認条件を維持します。

### Q3：hardware_timedの残り

**推奨：radioのtimed-update専用能力と機構は除去し、開始・送信に必要なtiming能力は残す。**

除去対象：
- `command_queue_depth`
- timed update用 `release_window_ns`
- `timed_instant`
- radio更新由来の `COMMAND_QUEUE_FULL` と専用payload/schema。ただし将来用途だけを理由に残さない。

残す：
- `min_timed_command_lead_ns` / `ProviderInstance.min_command_lead`
- startup latency
- RX/TX start leads
- TX in-flight window
- overflow restart gap

**注意**：release windowは現在、RX successor readinessにも入ります（`timeline.rs:1020–1038`、RM-25）。削除時は、その項だけを除き、旧rateのreceive call・delivery allowance・start leadまで一緒に消してはいけません。RM-6、RM-20、RM-35、profile、schema、測定fixturesを同時変更します。

### Q4：位相

**推奨：開始前configureはuntimed tuneとし、UBX/OBXの位相保持を約束しない。**

- **VERIFIED**：[UBX source](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_ubx.cpp#L949-L958)、[RX側](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_ubx.cpp#L1093-L1102) はcommand timeの有無でphase sync設定を切り替えます。
- per-board宣言はRM-9/RM-29、UR-9のまま。CBXは元からrandomです。
- Mockは新しい開始の実際のconfigureに対応してphaseを更新し、失敗・取消し・同値設定で何回drawするかを規定してください。

「同長repeatのsample周期ロック」と「RF LOの位相coherence」は別保証です。波形差替えだけでretuneしない限り、前者のために後者を失わせる必要はありません。

### Q5：channelごとの値

**推奨：logical channel番号をkeyとするdict。sample_rateだけstream scalar。**

例：

```python
rx.start(
    at=t,
    frequency={0: 2.45e9, 2: 2.46e9},
    gain={0: 10, 2: 12},
    sample_rate=20e6,
    antenna={0: "RX2", 2: "RX2"},
)
```

理由：
- stream内で値が違う構成を表せる。
- 配列の位置とlogical channel番号を混同しない。
- 存在しないchannel・他streamのchannelを送信前に検査できる。
- scalar broadcastとdictの二系統を同時に設ける必要がない。

省略channelは変更しない。各keyを既存 `UpdateParameter` に展開し、全引数を検査してから送ります。部分成功の寿命はFinding 7の規則が必要です。

### Q6：admissionとstream状態判定の順

**推奨：現在のKernel順序を維持。ただしQ6の説明を「envelopeが常に先」とだけ書かない。**

実際の順は：

1. dispatch/run state、target、ASCII
2. declared class
3. value kind
4. Provider `coerce`（Session/scheduleの更新）
5. proposed configuration
6. registered admission checks＋coercion policy
7. admitted configuration更新・dispatch
8. Providerのbookingでstream状態・予約・projectionを検査

根拠：`crates/ezsdr-kernel/src/coordinator/admission.rs:35–199,210–286`。

したがって、
- out-of-profile rateはRF check以前に`coerce`で拒否され得る。
- profile内だがPerformanceEnvelope超過のrateも`coerce`で拒否され得る。
- RF envelope外のTX frequencyはregistered checkで拒否。
- 許容値だが流れているstreamへの変更は、その後Providerが拒否。
- 後者はKernel logのRejectedではなく、**admitted entry＋Provider event**。KC-27上、admitted configurationには値が残る。

§58 #16の試験は、停止／空streamに対して、**admissionで拒否・log記録・未dispatch**を確認してください。別試験で、合法値へのProvider状態拒否とaction idの対応を確認します。Provider bookingは現在も再度configurationをproject/coerceしています（`control.rs:1004–1028`）。Kernelの検査を通ったことは、新しい予約競合検査の代わりにはなりません。

編集・format・git操作・build・test・mutation・実機操作はしていません。hardwareの安全なrate切替え条件は、上記ソース確認を越えて検証していません。

DRAFT NEEDS CHANGES — 12 findings
