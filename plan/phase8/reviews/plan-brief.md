# Sol 6.1 への依頼：Phase 8 の計画と設計 33 の独立レビュー

あなたは独立したレビュー役です．**読むだけで，ファイルを一切変更しないでください**（build も test も不要）．リポジトリは /home/komatsu/local_work/Ez-SDRv4（Ez-SDR v4）．まず AGENTS.md を読み，§3（変えない設計制約），§4（作業の順番），§6（adversarial・evidence-driven・VERIFIED/INFERRED を区別，scope creep は欠落と同じ重さの欠陥，Pre-release なので長期的に良い破壊的変更は可，version は上げない），§9（分担）に従ってください．

## レビューする文書

1. `plan/phase8/00-overview.md`：Phase 8（Vision §59 の Mock ↔ X310 parity）の計画．§4 の判断 P1〜P5 と §8 の判断ログは owner が受理済み（判断ログの各行に owner の言葉がある）．受理済みの判断そのものを覆す提案は，重大な理由があるときだけ P0 として挙げ，理由を示すこと．
2. `plan/phase8/33-channels-and-streams.md`：P1 (c)（channel と stream を分ける）の設計の下書き．**これが主な対象**．D1〜D12 の推奨と，§7 の「残る問い」に必ず答えること．
3. 参考：`plan/phase8/reviews/p1-sol-review.md`（あなたの前回のレビュー），`plan/phase8/research/x300-uhd-facts.md`，`plan/phase8/research/usrp-family-limits.md`（sonnet の調査．VERIFIED は調査役が読んだという意味で，orchestrator の照合は一部だけ）．

## 照合すべき accepted specs と code

`design/07-radio-model.md`（RM-*），`design/09-mock-radio.md`（MR-*），`design/18-uhd-radio.md`（UR-*），`design/03-spec-and-binding.md`（SB-1〜SB-5，SB-T0，SB-22c と SB-T2，SB-23，SB-34〜SB-36，SB-45），`design/04-run-and-session.md`（RS-12，RS-52），`design/05-module-api.md`（MA-10，MA-11，MA-34），`design/06-kernel-coordinator.md`（KC-23〜KC-27），`design/16-easy-api.md`（EA-9，EA-16），`design/11-simulation-channel.md`，Vision `design/vision/05-coherence-calibration-and-runtime-semantics.md`（§25）と §59（`design/vision/10-implementation-path-and-testing.md`）．code：`crates/ezsdr-kernel/src/module_api/registry.rs`（`key_decl`），`crates/ezsdr-kernel/src/spec.rs`（`KeyDecl`），`crates/ezsdr-radio/src/device.rs`，`crates/ezsdr-radio/src/timeline.rs`，`crates/ezsdr-mock-radio/src/`，`crates/ezsdr-radio-uhd/src/`．UHD は tag v4.10.0.0（https://github.com/EttusResearch/uhd ）．C API の関数（`uhd_usrp_get_rx_rates`，subdev spec の設定，複数 streamer）が `host/include/uhd/usrp/usrp.h` にあるかも確かめてください．

## 特に見てほしいこと

- 設計 33 の D1（設定を device の fragment に番号付きの key で置く）と D2（Kernel の `KeyDecl` に番号の付く segment を足す）は，owner が受理した (c)「channel の node が channel ごとの設定を持つ」の意図を満たすか．退けた K2（node ごとの設定）と比べて長期的にどちらが良いか．Kernel の変更として最小か，穴（SB-2 の key の位置，`policies.coercion`，SB-45，schema，matching）はないか．
- Spec と binding の分離（AGENTS.md §3）：channel の通し番号を binding の layout が決めることで，Spec が binding に依存しすぎないか．
- 1本の aligned stream に設定の違う channel を入れる（owner の要件）ことが，RM-16・RM-25・`ezsdr_radio::timeline`・Stream Contract（spec 02）と矛盾しないか．membership の変更（D3）の扱い．
- D8（channel 間の拘束）と D9（throughput と queue の上限）が，Mock と UHD Module の両方で同じ判定になるか（runtime の更新 KC-26 を含む）．
- §59 の parity：計画（00-overview の P2・P3 と bench）と設計 33 が整合し，Mock が実機より緩い箇所が残らないか．
- 段階（00-overview §5，設計 33 §5）の順番と，各段階で tree が緑のままになるか．scope creep と欠落．

## 出力

日本語で，次の形で標準出力に書いてください．
- 結論：計画と設計 33 を，そのまま進めてよい／変更つきで進めてよい／設計をやり直す，と一文の理由．
- 指摘：重大度（P0：決定や設計を変えるべき，P1：Gate P の前に直す，P2：記録でよい），対象（文書と節），中身，根拠（`path:line` か URL）．VERIFIED/INFERRED を付ける．見つからなかったことは，実行した検索を書く．
- 設計 33 §7 の5つの問いへの答え．
- D1〜D12 それぞれへの賛否（一行ずつ）．
