# 段階8e — R1/R2 修正記録（2026-10-13）

plan 31 のDA-1化と判断6に対する独立レビューのR1/R2（P2）を親が採用した。規範の正本は `design/03-spec-and-binding.md`。この記録は理由を保持し、規範条件を追加しない。E1/E2/E3はowner確認待ちであり、この修正では変えない。

**後続の決定と検証:** 以下はR1/R2修正時点の記録。ownerはその後E1/E2の明確化とE3の8g持越しを承認した（[計画31 §8](31-item8-kernel-specs.md#8-段階8eの決定と8gへの必須持越し)）。現行のSB-37と関連する写しはSB-T4への参照に改訂済み。J328〜J330は最終remote gateで実測killed。下記の「owner待ち」「実測は留保」は当時の状態であり、現在の未完了事項はE3だけである。

## R1 — 宣言内容の欠落を比較で検出する

既存の `source_component`、`member`、`island` と、device-pacedテストの `RelativeBudget` パターンを再利用する。Islandのfixtureは `id=local(3)`、`executor=exec`、`components=[{source, memory_domain=local(0)}]`、`affinity=Some([1,3])`、`rt_policy=Some({sched: fifo, priority: 50})`。componentにはhost.monotonicの1,000,000 ticksの正のbudgetを宣言し、MA-39の要求を満たす。

`IslandDecl`、`ComponentPlacement`、`RtPolicy`はderiveした `PartialEq/Eq` で全fieldを比較する。componentsを空にすれば非空listとの比較が失敗し、affinity/rt_policyを省略すれば `None` と `Some` の比較が失敗する。J329のnullは維持し、J330は型の正しいid/executor/componentsだけを返し、両optional fieldを落とす。拒否経路は型復元ではなく宣言全体の `assert_eq!`。これは静的根拠であり、mutationの実測は親gateに留保する。

## R2 — 規範から移した理由

| 箇所 | 保持する理由・説明 | 規範側に残す条件 |
|---|---|---|
| SB-37 | Matchingがbindingを先行できない理由は、比較するcapabilitiesがbindingの選んだinstanceのものだから。 | 元の段階列挙を変更せず、`Matching cannot precede binding.`を保持。E1のadmission順序は未変更。 |
| SB-24 | Authority fragmentは呼出し用ではなく、AuthorityのModuleとversionをplan/Manifestへ運ぶ記録用の機構。Vision §7のUHD timekeeperがresource同居、sim-engineが独立slotの例。 | lifecycle不在、coordinatorのlifecycle呼出し禁止、`instance`によるModule/versionのplan/Manifest記録、SB-22fの検査、`content=null`、全slot/衝突/Authority数の条件。 |
| SB-T4後の注記 | SB-30/SB-46が複数段階にある理由は、requested/applied/proposedの別configurationを判断するからであり、同一検査の重複ではない。 | SB-T4全セルとSB-30の段階別configuration、SB-46の各policyの条件は不変。 |
| SB-15 | resource endpointはIsland外なのでplacementがIslandを要求しない。Sinkはboundなのでgraph.links consumerではない。linkを両端で識別するため重複pairは曖昧になる。 | Island外、Island不要、reachability、Block拒否のSB-17帰属、Sink非consumer、両端識別、pairは高々1回の各条件を独立文で保持。 |
| SB-17 | output自体がlink宣言なのでpolicy/capacityを持つ。SinkはModule roleでExecutor-loaded componentではないのでgraph.components/Islandを参照しない。 | outputがlink宣言であることも含め、全field、拒否、owner、binding、非placement、参照しない条件を保持。 |
| SB-22g | Specのoutputがfeedを宣言することが、Spec bindingにfeedを許さない理由。 | Spec bindingのfeed禁止、outputのfeed宣言、Sessionのexactly条件を保持。 |
| SB-25 | 両端でlinkを識別するためgraph.linksの並べ替えがModule再bindingを起こさない。SinkはboundなのでIslandに配置しない。 | 両端識別、再bindingしないこと、exact placement、Sinkの非配置を保持。 |
| SB-26 | readerが4名だけを受けるためVocabulary sectionを読む操作を表現できない。 | 4名のみ、全他sectionはopaque、Vocabulary sectionのreadは表現不可の各条件を保持。 |
| SB-39 | malformed arm_orderの拒否はMA-41のmalformed ezsdr.time拒否と同じ方針。 | arm_orderの型・field・fragment存在・拒否条件をすべて保持。理由の比較句だけ移す。 |

規範中の `so` / `therefore` を無差別には消さない。SB-6の数値比較とcanonical hashの限界、SB-16のtarget/class/clock解決、SB-22のtyped identity、SB-41の報告帰属、SB-43のgrid admissionなどは意味を持つ条件であるため保持した。長行の改行はrow matcherへの影響を増やす必要がないため今回は行わない。SB-15aのonly、SB-37の既存段階列挙、MA-12/SB-46はowner待ちのまま。
