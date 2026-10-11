# 項目3 — 受け入れ判定を1本にする計画

| 項目 | 内容 |
|---|---|
| 状態 | **計画と追加判断はowner承認済み。実装未着手。accepted specへの移管前。** |
| 作成日 | 2026-10-11 |
| 承認 | 2026-10-11、owner「レビュー結果を反映して推奨でOKです．計画の修正をしてください」 |
| 独立レビュー | Claude Opus 5.5。条件付き賛成。指摘の反映は§2.3に記録 |
| 調査時の基準 | `main`、HEAD `a788875`、作業ツリーclean |
| 対象 | 凍結前改修の項目3と、項目3に追加された確定事項 |
| 作業 | ソース・記録の読み取りと計画作成。実装・テスト実行は未着手 |

## 1. 推奨する構成と範囲

宣言とtargetの参照を共通化し、すべてのKernel Action生成元をCoordinatorの同じadmissionへ通す。4段階に分け、各段階をテスト・独立レビュー・mutation gateまで完了させる。KC-28の文書検査・コンパイルとControlOpは、このKernel Action admissionの前段または外側にある（§5）。

生成元ごとの局所修正、templateの`class`を残して整合性検査を加える方法、scheduleのtargetを狭める方法は採らない。[項目3の決定記録](24-prefreeze-audit.md)に従う。

必須範囲は次のとおり。

- KC-24をKernel Action admissionの唯一の順序・拒否表にする。KC-28の前段・ControlOpの拒否は別表で明記する。
- 更新クラス、target分類、Specのkey位置、Valueの型検査を共通化する。
- scheduleのComponent target、固定ParamDecl、SB-30のrequested値を整合させる。
- Moduleイベントのsourceを自身のroot内に限定し、controlイベントと`Action::Emit`のASCII keyを検査する。
- `Resource.memory_domains`を追加し、Resource endpointを含むMA-39検証を完成させる。

Provider内部のcommand拒否処理の共通化は、handoffの任意事項として必須範囲から外す。項目8とPhase 8はこの計画に含めない。

## 2. 既決事項と、この計画で具体化する判断

### 2.1 既決事項

[24-prefreeze-audit.md](24-prefreeze-audit.md)のowner決定（18行目）と項目3（116行目以降）に従う。

- KC-24が唯一のadmission順序・拒否表となる。
- `ActionTemplate::UpdateParameter`と`CompileRule::UpdateParameter`から`class`を削除する。受信側が読む`Action::UpdateParameter.class`は必須のまま維持する。
- `ParamDecl.update_class`はoptionalとし、`None`はRun中固定を表す。固定宣言を未宣言扱いしてVocabularyへfallbackしない。
- 更新クラスを参照する関数を共有し、validateでも使う。
- `ExperimentSpec::keyed_entries()`でprefix検査とKeyDecl検査の走査を揃える。runtimeのKeyDecl値もkind検査する。
- `key_decl`は`Option<&KeyDecl>`を返す。
- scheduleのtargetはKC-23のResource・Output・Componentを認める。
- SB-30からneedsのEq値をrequested値へ加える処理を除く。
- Component paramの所有者はそのParamDeclを持つComponentであり、Vocabulary prefixを要求しない。`ParamDecl.schema`はopaqueのまま扱う。
- `emit_control`と`Action::Emit`にもASCII key検査を適用する。

[28-forward-audit.md](28-forward-audit.md)の判断3（13行目）に従い、Resourceの各nodeにmemory domainを宣言し、portsを持つnodeでは1件以上を要求する。Resource endpointを含むlinkをMA-39で検査し、MockRadioとUHDのstream nodeにhost domainを宣言する。

[27-role-typed-identities.md](27-role-typed-identities.md)の判断4（8行目、161行目以降）と[handoff.md](../../handoff.md)の項目3追記（938–941行目）に従い、Moduleイベントのsourceを自身のroot内に限定し、Executorのsource rootがrouting欠落時に`Unforeseen`となる処理を解消する。

### 2.2 今回承認された具体策

2026-10-11にownerがOpusレビューの反映と推奨案を承認した。§2.1の従来の決定と区別し、今回確定した具体策を記録する。

owner確認を経た4判断と代替案の比較を残す。

| 承認された推奨 | メリット・デメリット | 採らなかった案と理由 |
|---|---|---|
| 各producer domainからconsumerへの到達を保証する | Providerが宣言したどのdomainで出しても読める。集合が広すぎる宣言は拒否されるため、実際に出しうる集合を正確に宣言する必要がある | 1組だけ接続可能なら許可する案は検証が小さいが、残るdomainの安全を保証できない。BindingProfileでdomainを選ぶ案は柔軟だが、選択欄・Providerへの伝達・遵守契約が増える |
| 内部のclass未確定要求をadmissionで完成させる | 必須classを持つ公開Actionを維持し、target・classの判定場所を揃える。内部型が1つ増える | compile/Armでclassを埋める案は内部型を減らせるが、targetとclassの拒否が生成元別に分かれる |
| compile・Compiled・resolveを内部APIにする | 需要のないコンパイラ契約を公開範囲から除ける。直接呼ぶ既存テストの移行が必要 | 公開APIとして整える案はRunなしで変換結果を調べられるが、内部要求型と宣言参照の文脈まで公開契約になる |
| target → ASCII → class → kindとする | 現行KC-24のtarget優先に揃えられる。実装と複数違反テストの更新が必要 | ASCIIをtargetより先にする案はpayloadを先に拒否できるが、現行規範の順序も変える。SessionのASCII検査は引き続き前段にあり、この差が観測される主な経路はModule提出Actionである |

- Resourceのdomainsは、送出portでは出しうる集合、受入portでは読める集合とする。各producer domainから少なくとも1つのconsumer domainへの到達を要求する（§7）。
- Sessionのコンパイル結果とtimed scheduleのArm時admission入力に、class未確定の内部更新要求を使う。target解決後に宣言から完成済みActionを作る。agendaの型は変更しない。
- `session::compile`、`Compiled`、`ActionTemplate::resolve`を内部APIにする。
- Kernel Action admissionの順序は、凍結、RunState、target、ASCII、class、kindを先頭とする。KC-28の前段とControlOpの拒否はこの順序の外側にある。
- Component宣言をSpecから直接参照し、`component_classes`キャッシュを削除する。固定と未宣言の区別は元のParamDeclで保持する。
- 同一Component内のParamDecl key重複を拒否する。
- constraintとruntime Valueで同じkind互換規則を使い、`Num`に`Int`を許可する。
- Moduleへroot付きEventSink viewを渡す。EventHandleにはプロセス内で一意なscope IDを1つ保持させ、別Run・別scopeの利用を拒否する。`resolve`をfallibleにする。
- Executorのsource rootは、prepare時に正常にparseしたIslandDeclから取得する。
- Resourceのdomain省略は空リストとする。portsのあるnodeでは空リストを拒否し、portsのない構造nodeでは許可する。
- radioのhost domainは呼び出し側が`HOST_MEMORY`をtree builderに渡し、portを持つrootに宣言する。

`component_classes`削除は、24-prefreeze-audit.mdの勧告にある「固定宣言をcomponent_classesに残す」という具体策からの変更である。固定宣言をVocabularyへfallbackさせない意味は維持し、Spec直接参照で実現することを今回承認した。

### 2.3 Opusレビューの反映記録

レビューはClaude Opus 5.5（`claude-opus-5-5`）が既決事項と現行ソースを読み取り、コード変更・テスト実行なしで行った。4判断への評価は、domainの全件到達保証・内部API化・target優先に賛成、内部更新要求に条件付き賛成。重大な設計問題はなく、次の条件・改善を反映した。

| 指摘 | 計画への反映 |
|---|---|
| M1: KC-24の「唯一」の範囲 | Kernel Action admissionに限定。KC-28の前段とControlOpの拒否を§5の別表に記載 |
| M2: handle識別の簡素化 | collector IDとscope IDの2欄を設けず、プロセス内で一意なscope IDを使う（§6） |
| M3: 改訂する規則の漏れ | KC-17・KC-28・RS-14・RS-17・RS-49a・RS-32・RS-33・MA-6・MA-25、host-data-pathの参照を追加（§8） |
| M4: 段階依存と途中の整合 | 3a→3bを必須とし、3aでは既存class等値検査を共通参照へ置換。3cと3dの独立性を明記 |
| timed scheduleの扱い | Arm時にadmissionされることを明記。agendaの型変更を削除（§5） |
| 拒否表の不足 | Providerのscalar制約とmatched Provider欠落を追加。ASCII拒否に不正payloadを転記しない（§5） |
| radio宣言の細部 | `HOST_MEMORY`を引数で渡し、portを持つrootに宣言。Resource受入テストはIn port付きdoubleを使う（§7） |
| その他の境界条件 | ParamDecl重複検査の場所、ModuleによるKernel/Unforeseen偽装の拒否、Emitのadmissionテスト範囲を明記 |

実装順は3a、3b、3c、3dとする。3a→3bは必須の依存であり、3cと3dは3a・3bおよび互いに独立している。段階ごとの変更とgateを分け、必要なら独立した段階を先に進められる。

## 3. 現状の制約

- `admit_with`の前にSessionコンパイルが更新クラス不足を拒否するため、現状ではfrozen stateやtargetより先にclassエラーが出る。templateもArm時にclass付きActionへ変換される。
- Sessionの文書/ASCII検査、時刻解決、Vocabulary・artifactを扱うコンパイル、ControlOpの拒否もKC-24の前段または外側にある。
- timed scheduleはArm時にadmissionされ、順次更新したworking configurationで判定される。agendaが保持するのはuntimedのStopである。
- `Admitter`は登録チェック、coercion policy、runtime class存在検査の順に動き、Coordinatorにもclass検査がある。runtimeの`KeyDecl.kind`検査はない。
- validateのschedule targetはComponentを拒否する一方、実行時resolverは認める。
- prefix検査とKeyDecl検査の走査が異なり、output・scheduleの検査が不足している。
- ComponentDescriptorはSpec内にあり、routingや選択Executorがなくても対象ComponentのParamDeclを参照できる。
- Moduleは同じEventCollectorを受け取り、EventHandleは公開indexだけを持つ。source所属とhandleのcollector・scope所属は検査されない。
- MA-39はResource producerのfeedとResource endpointを含むgraph linkをskipする。
- radioの共有tree builderではrootにRX Out portがあり、rx/tx childのportsは空である。MockRadioとUHDはこのbuilderを共有する。

根拠となるソース・記録は§9に列挙する。現在の実装が計画どおりに動くことを示す記述ではない。

## 4. 段階3a — 宣言・target・Spec検証を共通化する

### 変更対象

`crates/ezsdr-kernel/src/spec.rs`、`binding.rs`、`module_api.rs`、`module_api/registry.rs`、`plan/validation.rs`。キャッシュの整理は`coordinator/state.rs`と`coordinator/pipeline.rs`も対象とする。

### 変更内容

1. `classify_target(spec, target)`を設け、Resource・Output・Componentの宣言上の分類を行う。実装ではvalidateだけが呼び、実行時resolverは検証済みplanから作ったroutingだけを読む（review P2-4：resolverに置いた呼び出しはmutationが生き残ったため削除）。Resource path検査は`classify_target`が持つ。
2. `declared_class(spec, target, key, registry)`を唯一の更新クラス参照にする。Resource・OutputはKeyDecl、ComponentはそのComponentのParamDeclを使う。ComponentのParamDeclが固定・欠落の場合、Vocabularyへfallbackしない。
3. `ParamDecl.update_class`を`Option<UpdateClass>`へ変更する。同一Component内のParamDecl key重複は`ComponentDescriptor::validate`で拒否し、異なるComponentの同じkeyは許可する。`component_classes`キャッシュを削除し、元のSpec宣言を参照する。
4. `keyed_entries()`で次の位置と、検査に必要なtarget・所有者・constraint/Valueの文脈を返す。

| 位置 | 検査 |
|---|---|
| Resourceの`requires`、`needs[].requires` | prefix、所有者、KeyDecl、constraintの型 |
| Outputの`params` | prefix、所有者、KeyDecl、Valueの型 |
| `policies.coercion` | prefix、所有者、KeyDecl |
| scheduleのUpdateParameter | targetに応じた所有者・宣言・値・更新クラス |

Component paramはParamDeclが所有し、Vocabulary prefixを要求しない。Component値にVocabularyのkindを適用せず、opaqueなschemaも新たに解釈しない。Command paramsとTxBurst metadataにはKeyDecl必須規則を加えない。

5. `key_decl`を`Option<&KeyDecl>`に変更し、呼び出し側が文脈に応じて拒否を作る。未知prefix・未宣言key・ext owner不在の条件をspecで明記する。
6. kind互換規則を共通化する。`Num`は`Num`と`Int`、その他は同じkindを認める。List・Mapは外側のkindを検査する。constraintのscalar検査もこの互換規則を使う。
7. scheduleのComponent targetを許可し、target検査後に共通の宣言参照を行う。
8. SB-30のrequested値からneedsのEq値を除く。needsのconstraint検証は維持する。

3aではtemplateとCompileRuleの`class`がまだ残る。validateなどの既存class等値検査を共通の`declared_class`へ差し替え、全callerをoptional ParamDeclに対応させて段階末をgreenにする。class欄の削除と、Sessionのclass不足拒否をadmission内へ移す変更は3bで行う。**ただし`Admitter`のruntime class検査と`declared_classes`の削除は3aで実施した**（review P2-10：class拒否はadmissionがKC-24 step 5として持ち、runtimeの`Admitter`はadmissionの1か所だけなので、3bへ残すと同じ拒否が2か所に分かれる）。

### 完了条件

output・scheduleの検査漏れ、各位置のext owner、固定/未宣言Component、同じkeyのtarget別宣言、重複ParamDecl、numeric互換、SB-30の変更をテストで固定する。レビュー（2026-10-11）の修正で次も固定する：`ext.`のstripは1回（P1-1）、component keyはVocabulary prefix不要（P1-2）、missing ParamDeclはno-fallbackでcomponent値にkind検査を適用しない（P1-3）、runtimeのclass拒否はModuleとSessionの両経路（P1-4）、List/Mapの外側kind（P2-2）、keyed_entriesのneeds・coercion・schedule位置（P2-1）、MA-37がRS-52に先行（P2-5）。段階3a末に§8のgateを通す。

## 5. 段階3b — クラス生成と実行時admissionを集約する

### 変更対象

`crates/ezsdr-kernel/src/session.rs`、`event.rs`、`coordinator/admission.rs`、`coordinator/pipeline.rs`。Arm時のadmission入力と関連テスト、`tests/kernel_surface_allow.txt`および公開surfaceの集計・説明を整合させる。

### 変更内容

`ActionTemplate::UpdateParameter`と`CompileRule::UpdateParameter`から`class`を削除する。受信側が使う`Action::UpdateParameter.class`は必須のまま維持する。

内部のadmission入力は、完成済み`Action`と、target・key・value・時刻を持つclass未確定の更新要求の2形態とする。この型は内部処理用であり、wire schemaには追加しない。

- Sessionコンパイルは、更新について未確定要求を返す。timed scheduleはArm時の時刻解決後、更新を未確定要求として直ちにadmissionへ渡す。
- admissionでtargetを確認した後、共通の宣言参照からclassを導いてActionを作る。
- Module提出Actionのclassは、同じ参照結果と比較する。
- `session::compile`、`Compiled`、`ActionTemplate::resolve`を内部APIへ整理する。直接呼ぶ既存テストは内部単体テストまたはCoordinator経由の振る舞いテストへ移す。`ezsdr-sink/tests/sink_vocabulary.rs`はRunを通し、`sink.capture`が時刻付きUpdateParameterとして届くことを検査する。
- コンパイラには構文・verb変換のエラーを残し、更新クラス不足をコンパイル時に拒否する処理は削除する。

timed scheduleのclass不足は共通の宣言参照を使うvalidateで`Failed { validate }`となる。正常にvalidateされたSpecでは、Arm時までに宣言が変わらないため同じclass不足は生じない。KC-17の`(instant, schedule index)`順とworking configurationによるadmissionを維持し、start時には完成済みのadmitted Actionをdispatchする。agendaには引き続きuntimedのStopだけを格納する。

### KC-24の順序

KC-24を次の唯一のKernel Action admission順序とし、各rowのreason、`key`、`requested`の設定も改訂specに記載する。この順序にはArm時のschedule admissionも含む。Session入口のKC-28前段とControlOpは、後掲の別表に記載する。

表は項目3全体の最終契約である。3bでは既存のASCII検査対象（UpdateParameter・Command・TxBurst）をこの順序へ揃え、3cでAction::Emitを同じASCII判定へ追加する。3cを先に進めた場合も、検査対象の追加と拒否順序の変更を分けて扱う。

| 順序 | 判定 | 拒否の`check` |
|---|---|---|
| 1 | dispatchが凍結されていない | `ezsdr.dispatch` |
| 2 | Session・Module originでRunがRunning | `ezsdr.run_state` |
| 3 | targetが自身のroleで解決する。ModuleのStop(None)もここで拒否 | `ezsdr.target` |
| 4 | payloadのMap keyがASCII | `ezsdr.value` |
| 5 | 更新クラスが宣言され、Module指定classが一致する | `ezsdr.update_class` |
| 6 | UpdateParameterの要求値がKeyDeclのkindに適合する | `ezsdr.value` |
| 7 | TxBurstのlate policy、stream clock、時刻、入力storeを検査する | 現行の各`check` |
| 8 | Session・scheduleのProvider updateがscalarである（SB-5）。List/Mapを拒否 | `ezsdr.value` |
| 9 | 上記coerceに使うmatched Providerが存在する | `ezsdr.target` |
| 10 | Session・scheduleのProvider updateをcoerceする。error・reject・適用値欠落を拒否 | `ezsdr.coercion` |
| 11 | coerce後の値もkindに適合する | `ezsdr.value` |
| 12 | 登録されたRuntime AdmissionCheckを実行する | 各check |
| 13 | coercion policyを適用する | `ezsdr.coercion` |

Component値にはVocabulary由来の型検査を適用しない。Coldも宣言に一致すれば有効である。

`requested`へ値を転記するclass・kind・coercionなどの拒否は、必ずASCII検査の後に置く。ASCII違反の拒否は`requested: None`とし、不正なMap keyを持つpayloadを拒否記録へ転記しない。これによりログやManifestのsealへ不正な値が入ることを防ぐ。

### KC-28前段とControlOpの拒否

改訂KC-24・KC-28に、KC-24はKernel Action admissionの唯一の順序であり、KC-28の入口・文書検査・時刻解決・コンパイルおよびControlOpはその前段または外側にある、と明記する。内部要求型で移すのは更新クラスの確定・拒否であり、Session全体の拒否をKC-24の順序へ移すものではない。

| 位置 | 条件・拒否 | 記録 |
|---|---|---|
| Session入口（KC-28手順1） | cleanup後の`Ended`、Spec Runへの`NotSession` | 戻り値のerror |
| `SessionLog::check_entry` | 文書・ASCII違反などの`Malformed` | 戻り値のerror。sequence番号を消費しない |
| `session_earliest` | 時刻解決の拒否、`ezsdr.time` | Session entryの`Rejected` |
| `session::compile` | Vocabulary・artifactの拒否、`ezsdr.vocabulary`・`ezsdr.artifact` | Session entryの`Rejected` |
| ControlOpのRunChild | `ezsdr.run_child` | Session entryの`Rejected` |
| ControlOpのAdopt/Renew | `ezsdr.lease` | Session entryの`Rejected` |

ControlOpをKernel Actionへ変換してKC-24に通す変更は、この項目に含めない。前段で拒否された場合はKernel Action admissionに到達しない。複数違反の比較テストは、前段を通過したActionを使う。前段・ControlOpの拒否自体もKC-28との整合を検査する。

`Admitter`の重複した更新クラス検査は削除する（3aで実施済み：runtimeのclass拒否はadmissionがKC-24 step 5として持つ。review P2-10）。Runtimeの拒否順序を重複記載した規則はKC-24への参照に置き換える。validate/prepareなどRuntime以外の既存Admitter用途は維持する。

Module Abortの例外、Session batchの一括拒否、拒否されたActionがIDを消費しない規則、admission lockの範囲を維持する。coerce後の値が不正なら拒否し、effective configurationへ記録しない。

### 完了条件

Session SetParameter・Vocabulary verb・Arm時のschedule・Moduleを比較し、適用されるチェックの順序、coerce後の不正値、拒否時のconfigurationとActionIdを固定する。Moduleでは不正targetと非ASCII payloadの同時違反がtarget拒否になることを確認する。Sessionの非ASCII入力は前段で`Malformed`となること、scheduleの宣言不備はvalidateで拒否されることも固定する。Providerのscalar制約、matched Provider欠落、前段・ControlOpの拒否を表と照合する。既存のclass一致テストは新しい生成経路に合わせて更新する。段階3b末に§8のgateを通す。

## 6. 段階3c — イベント発信元とASCII検査を保証する

### 変更対象

`crates/ezsdr-kernel/src/event.rs`、`run.rs`、`coordinator/pipeline.rs`、`coordinator/state.rs`、MockRadioのprepare、UHDの`Core::new`、sink-capture、exec-nativeおよびtest doubleのEventSink利用箇所。traitの署名・handleの公開fieldを使うcallerを全workspaceで移行する。文書更新はRS-32・RS-33・MA-6のほか、PrepareContextのsource/root契約を対象とする。

### 変更内容

Moduleにはprepare時にrootを保持するEventSink viewを渡し、集計・キュー・counterは共有collectorへ委譲する。Executorのviewはprepareされるfragment（Island）単位とする。

| Moduleのroot | 許可するsource |
|---|---|
| ProviderのNode | 同一ResourceIdと、segment境界を満たす子孫 |
| SinkのOutput | 同じOutputのみ |
| ExecutorのIsland | 同じIslandのみ |

`resolve`を`Result<EventHandle, RunError>`へ変更し、範囲外sourceをfallback前に拒否する。自分の範囲内にある未登録source/kindの既存fallbackは維持する。

Moduleが`Kernel`または`Unforeseen`をsourceとして指定することも拒否する。範囲内の未知source/kindをcollectorがfallbackへ集約する処理と、Moduleによる`Unforeseen`の直接指定は区別する。

EventHandleの内部indexを非公開にし、プロセス内で一意なscope IDを1つ保持させる。scope IDはcontrol経路で`AtomicU64`から払い出し、再利用・wraparoundによる衝突を許さない。viewとKernel内部用sinkに別のscope IDを与える。hot `emit`はscope IDの一致と内部indexを固定時間で検査し、別Run・別scopeのhandleを拒否する。collector IDを追加する必要はない。ring recordにはscope IDを追加せず、表現・サイズを維持する。hot経路に割り当てやsource pathの解析を追加しない。

`emit_control`と`Action::Emit`へ再帰的なASCII key検査を追加する。`Action::Emit`は現在どおり対象Moduleへdispatchし、collectorへの直接発行には変更しない。

Executorのsource rootはprepare時に正常にparseしたIslandDeclから取得し、routing欠落時に`Unforeseen`へ置き換える分岐を削除する。

### 完了条件

他Provider・他Output・他Island・Kernel/Unforeseenの偽装、Resource path境界、範囲内fallback、別Run/別scopeのhandle、controlイベントの非ASCII payload、RT経路のzero-allocationとrecord sizeをテストする。`Action::Emit`は現在のModuleが未対応として拒否するため、今回のASCII規則はadmissionでdispatch前に拒否されることを検査する。Emitを新たに扱うModuleやdelivered経路は追加しない。段階3c末に§8のgateを通す。

## 7. 段階3d — Resourceのmemory domain検証を完成させる

### 変更対象

`crates/ezsdr-kernel/src/module_api.rs`、`plan/validation.rs`、`plan/compile.rs`、`plan/islands.rs`、`plan/links.rs`、`crates/ezsdr-radio/src/device.rs`、MockRadio/UHDのtree呼び出し、Resource literalを持つKernel test fixtureとradio treeのテスト。

### 変更内容

`Resource.memory_domains: Vec<MemoryDomainId>`を追加する。省略時は空リストとし、portsを持つnodeはvalidateで空リストを拒否する。portsのない構造nodeには空リストを許可する。検査はProvider treeの全nodeに適用し、bindingされていないnodeの記述子不備も拒否する。

共有radio builderの`tree(device, memory_domain)`にdomain引数を追加し、MockRadio/UHDが既存の`ezsdr_hostmem::HOST_MEMORY`を渡す。値を`MemoryDomainId::local(0)`として別のcrateへ写さず、`ezsdr-radio`へhostmem依存を追加しない。domainはportを持つrootに宣言し、portのないrx/tx childは空リストとする。28-forward-audit.md判断3の「stream nodesにhostを宣言」は、この現行treeでは実際にportを持つrootへの宣言として具体化する。現在のport配置は維持する。

Island検証にはResource名の集合だけでなく、実際にbindingされたnodeのdomainを渡す。Resource nodeの検索は既存のendpoint解決と整合させ、graph linkとSink feedのResource endpoint skipを削除する。

従来の記録で未定義だった複数domainの意味と量化は、今回のowner承認で次のように確定した。Resourceの送出portはそのnodeが出しうる集合、受入portはそのnodeが読める集合を使う。producerの集合を`P`、consumerの集合を`C`として、すべてのgraph link・Sink feedで次の1式を適用する。

```text
∀p∈P, ∃c∈C:
  p == c または selected Link.connects に (p,c) または (c,p) がある
```

Componentの集合はIslandで選んだdomainのsingleton、Sinkの集合はそのdescriptorの読み取りdomain、Resourceの集合は実際にbindingされたnodeの宣言とする。現在のComponent→Sinkの判定は`P`がsingletonの場合であり、同じ規則をResource endpointへ広げる。既存のpair対称性を維持し、MA-25・MA-39の文面もこの意味へ揃える。Coreがdomainを自動選択する仕組みやBindingProfileへのdomain選択欄は追加しない。

### 完了条件

Resource→Component、Component→Resource、Resource→Resource、Resource→Sinkについて、同一domain、変換可能、不可能、複数producer domainの一部だけ接続可能なケースを固定する。現在のradio treeにはIn portがなく、TXはTxBurst経路であるため、Component→ResourceとResource→Resourceの受入側はIn portを持つtest doubleで構成する。portsあり/なしnodeの空宣言、未binding nodeの記述子不備、実際にbindingされたnodeの参照、radio treeのhost宣言も検査する。段階3d末に§8のgateを通す。

## 8. schema・移行・検証・完了手順

### schemaと移行

各段階で影響する生成schemaとfixtureを同時に更新する。主な対象は`action_template`、`vocabulary_descriptor`、`experiment_spec`、`component_descriptor`、`resource`、`provider_instance`。埋め込みschemaへの影響も生成結果で確認する。`action`の必須classは維持する。

削除したclassを持つ旧template/CompileRuleは明示的に拒否し、fixtureを新形式へ移す。版番号を変更せず、SCHEMA_CHANGELOGへ追記しない。AGENTS.mdの現行方針を、古いauditのchangelog追記案より優先する。

PythonのSetParameterは既にclassを送らないためAPI変更は不要だが、拒否結果と既存操作を確認する。

schema再生成:

```sh
EZSDR_UPDATE_SCHEMAS=1 cargo test -p ezsdr-kernel --test schema_freeze
```

### 各段階のgate

関連テスト、独立レビューの指摘解消、schema整合、workspaceテスト、Clippy、同じtreeからbuildしたserverでのPythonテスト、6リストのmutation gateを完了条件とする。

修正中は影響したmutation行と新規行だけを再実行し、段階末に全リストを実行する。各mutation rowは編集後も正確に1か所へ一致させる。`missing`行はIDを指定して再実行し、未解決のまま完了扱いしない。

```sh
for l in maintenance phase3 phase4 phase5 phase6 phase7; do
  REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py "$l" sim02:6,sim03:6
done
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings
cargo build -p ezsdr-server
PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests
```

mutation listsは`plan/{maintenance,phase3,phase4,phase5,phase6,phase7}/tools/mutations.json`。build出力、review copy、remote shardの扱いはAGENTS.md §7に従う。

### 文書と最終完了

計画と追加判断はowner承認済みである。実装前に、承認済みの内容をもとにKC-24各rowのreason・field設定を含む改訂specを確定する。計画の承認と、実装完了後のaccepted specへの移管は区別する。

文書更新・最終照合の対象は次のとおり。

| 文書・規則 | 整合させる内容 |
|---|---|
| design/03: SB-2・SB-16・SB-30 | key走査、scheduleのComponent target、needsのEq値 |
| design/04: RS-14・RS-17・RS-49a・RS-52 | 内部要求、Runtime順序のKC-24参照、classの宣言元、classなしtemplate。RS-17のRuntime以外の段は維持 |
| design/04: RS-32・RS-33 | fallible resolve、opaque handle、source scopeとfallback |
| design/05: MA-6・MA-10・MA-25・MA-34・MA-39 | EventSink/root契約、Resource domains、Component所有、接続判定の1式 |
| design/06: KC-17・KC-23・KC-24・KC-28 | Arm時のtimed admission、target分類、拒否表と範囲、compileの署名と前段・ControlOp |
| design/10-host-data-path.md | sink_vocabulary.rsのテストをRun経由へ移すことに伴う参照更新 |
| kernel_surfaceのallowlist・集計・説明 | 内部API化後の公開範囲と件数。現行の114/291を固定値として残さない |

最終段階ではこれらを改訂specに照合し、受け入れ手順に従ってaccepted textとhandoffへ反映する。旧規則にある重複したRuntime順序やResource endpoint skipの記述を残さない。項目8やPhase 8へは進まない。

## 9. 調査根拠

行番号は調査時のHEAD `a788875`に対する参照であり、実装後の位置を保証しない。

| 内容 | 根拠 |
|---|---|
| 項目3のowner決定・範囲・F7/F8/F9 | `plan/maintenance/24-prefreeze-audit.md:18,116,324` |
| Resource domainsの追加決定 | `plan/maintenance/28-forward-audit.md:13` |
| Module source scopeの追加決定 | `plan/maintenance/27-role-typed-identities.md:8,161` |
| 現在地・追加範囲・gate | `handoff.md:933,938,941,946` |
| 現行KC-17・KC-24・KC-28 | `design/06-kernel-coordinator.md:438,469,492` |
| Session前段とControlOp | `crates/ezsdr-kernel/src/coordinator/pipeline.rs:1337` |
| 現行admissionとdispatch | `crates/ezsdr-kernel/src/coordinator/admission.rs:39,67,159,239,272` |
| Admitter、Compiled、Session class生成 | `crates/ezsdr-kernel/src/session.rs:267,335,381,404,470` |
| Component宣言がSpec内に存在 | `crates/ezsdr-kernel/src/spec.rs:742` |
| ParamDeclとComponent検証 | `crates/ezsdr-kernel/src/module_api.rs:419,468,500` |
| key位置とkind、prefix走査 | `crates/ezsdr-kernel/src/spec.rs:229,574,662,711,1066` |
| constraintのkind互換 | `crates/ezsdr-kernel/src/binding.rs:299` |
| schedule/KeyDecl検査 | `crates/ezsdr-kernel/src/plan/validation.rs:388,429,512` |
| registryのkey lookup | `crates/ezsdr-kernel/src/module_api/registry.rs:186` |
| templateと時間解決 | `crates/ezsdr-kernel/src/event.rs:780,835` |
| EventHandle、EventSink、handle検査、emit | `crates/ezsdr-kernel/src/event.rs:258,552,568,593,637` |
| scopeに使えるResourceId境界 | `crates/ezsdr-kernel/src/id.rs:306` |
| Moduleへのcollectorとsource供給、Island parse、routing | `crates/ezsdr-kernel/src/coordinator/pipeline.rs:634,771,801,1851` |
| source_rootのfallback | `crates/ezsdr-kernel/src/coordinator/state.rs:420` |
| Event hot-path既存テスト | `crates/ezsdr-kernel/tests/event_hotpath.rs:48` |
| Resource、Link.connects | `crates/ezsdr-kernel/src/module_api.rs:540,736` |
| Resource endpointをskipするMA-39 | `crates/ezsdr-kernel/src/plan/islands.rs:168` |
| IslandContextとbound Resource検索 | `crates/ezsdr-kernel/src/plan.rs:141`、`crates/ezsdr-kernel/src/plan/links.rs:218` |
| MockRadio/UHD共有radio tree | `crates/ezsdr-radio/src/device.rs:305`、`crates/ezsdr-mock-radio/src/lib.rs:285`、`crates/ezsdr-radio-uhd/src/provider/mod.rs:180` |
| host domain IDと呼び出し側の依存 | `crates/ezsdr-hostmem/src/lib.rs:10`、`crates/ezsdr-mock-radio/Cargo.toml:14`、`crates/ezsdr-radio-uhd/Cargo.toml:13` |
| schema登録と生成gate | `crates/ezsdr-kernel/src/schema.rs:88`、`crates/ezsdr-kernel/tests/schema_freeze.rs:20` |
| Python SetParameterのclassなし送信 | `python/ezsdr/session.py:508` |

外部資料は使っていない。今回の調査はrepository内のaccepted specs、owner決定記録、現行ソース、既存テストとmutation表に基づく。
