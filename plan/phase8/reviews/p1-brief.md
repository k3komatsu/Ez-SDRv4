# Sol 6.1 への依頼：Phase 8 P1（混在 daughterboard の表し方）の独立レビュー

あなたは独立したレビュー役です．**読むだけで，ファイルを一切変更しないでください**（build も test も不要）．リポジトリは /home/komatsu/local_work/Ez-SDRv4（Ez-SDR v4）．まず AGENTS.md を読み，§3（変えない設計制約），§6（adversarial・evidence-driven・VERIFIED/INFERRED を区別），§5（v3 は証拠であって雛形ではない）に従ってください．

## 背景

- Phase 8（Vision §59 の Mock ↔ X310 parity）の計画の下書き：`plan/phase8/00-overview.md`．その §4 の P1 は，X310 の device description を `radio` 語彙に1か所だけ置き，MockRadio と UHD Module が同じ値で使う，という判断（owner 受理済み）．部品は motherboard `x3xx`（X300/X310 共通）と board の種類．
- その後 owner が指摘：「実験では片方の slot に UBX，もう片方に LFTX/LFRX という構成がよくある．LFTX/LFRX/BasicTX/BasicRX はさらに混在でき，UBX＋LFTX＋BasicRX もよくやる」．
- 今の v4 の `radio` 語彙は，周波数・利得・antenna を方向ごとに1つしか持たない（`design/07-radio-model.md` の RM-2，RM-3，RM-4 の表）．MockRadio は1 instance で resource を1つしか受けない（`design/09-mock-radio.md` MR-7，§5 の M1）．UHD Module は profile ごとに front end 名を照合する（`design/18-uhd-radio.md` UR-5，UR-9）．description のコードは `crates/ezsdr-radio/src/device.rs`，profile は `crates/ezsdr-mock-radio/src/profile.rs` と `crates/ezsdr-radio-uhd/src/profile.rs`．
- v3 は channel のグループごとに rate・freq・gain・ant と subdev を設定できた（`v3/docs/v3.md` の `tx-subdev`・`tx-channels`・`for-channels`，`v3/config_examples/old/x310_BasicTXRX.json`）．
- orchestrator（Claude Opus）が UHD v4.9.0.0 の `host/lib/usrp/dboard/db_basic_and_lf.cpp` で確かめたこと：Basic/LF board は LO を持たず `freq/value` は常に 0，tune は DSP のみで範囲は ±max_freq，利得はない，RFNoC の装置では `ant_mode` が "AB" 固定で antenna の選択肢が AB/BA/A/B の接続（IQ/QI/I/Q）になる．帯域の定数値は未確認．

## orchestrator の提案

**(b) 推奨：設定を共有する channel のまとまり（グループ）を，device tree の1つの node にする．** 1台の X300 の Provider の下に，グループごとに `radio.device` の node を並べる（例：`ubx`〈slot A，rx/tx 各 1ch〉，`lftx`〈slot B の TX 側〉，`basicrx`〈slot B の RX 側，mode A/B なら 2ch〉）．各 node の中は今の「方向ごとに1つの設定」のままで，RM-4 のキー・`coerce`・`DeviceDescription` を流用する．P1 の board の種類ごとの description が node の description になる．profile 名は `x3xx` 1つ，slot の構成は selector に書き，Module は開くときに実物と照合する．board の種類（ubx・obx・cbx・basic_rx/tx・lf_rx/tx）は語彙の表に1行ずつ．
- 主張した利点：v3 の `for-channels` と1対1．片方向だけの board も自然（片方向の channel 数 0）．組み合わせで名前が増えない．
- 認めた欠点：RM-2（device 1台に node 1つ）と MR-7（1 instance 1 resource）を変える．Spec は使うグループごとに resource を1つ要求する．グループをまたぐ受信は1つの block にならず node ごとの stream（同じ時計の timestamp でそろえる）．

**(a) 退けた案：channel ごとのキー**（`radio.rx.ch.<c>.frequency_hz` など）．キー・Session の操作・coerce が channel 数だけ膨らむ．1つの stream なので channel ごとに rate を変えられない．

## 問い

1. 指摘された穴は本当か．RM-4 で混在構成が本当に表せないか，v3 は本当に表せたか．根拠つきで．
2. (b) と (a) を比べ，**ほかの案も探してください**（例：1台を複数の Provider instance で分け合う，slot ごとの node，グループを Spec ではなく binding だけで決める，など）．観点：AGENTS.md §3 の制約（Kernel を小さく，intent と binding の分離，coherence は所有する Provider が宣言，Stream Contract と SampleClock は stream ごと），Spec の matching（`design/03-spec-and-binding.md` の SB 規則，特に SB-23），Session の Action の宛先，グループ間の時刻合わせと coherence，グループごとに rate を変えられるか，§59 の parity と測定記録への影響，MA-3（Module 間の依存禁止），Phase 8b（1 instance で複数 USRP，SB-23）と 8c（RFNoC）との整合．
3. UHD の事実を UHD 4.10 の source（https://github.com/EttusResearch/uhd ，tag v4.10.0.0）で確かめてください：X300 の各 slot に RX 用と TX 用の別 board を挿せるか；X300 で Basic/LF の mode を antenna で選ぶか；X300 で受信 streamer を複数作り，それぞれ違う rate にできるか；UHD 4 の multi_usrp で X300 の channel と subdev（slot）の対応がどう決まるか；Basic/LF の帯域の定数値．VERIFIED/INFERRED を付け，URL と行を示すこと．
4. (b) を採るなら，設計メモで決めなければならないことの一覧と，壊れるもの・リスク・scope creep の指摘．

## 出力

日本語で，次の形で標準出力に書いてください．
- 結論：(b) を採る／変更つきで採る／退ける（別案を推奨），と一文の理由．
- 指摘：重大度（P0：決定を変えるべき，P1：設計メモで必ず扱う，P2：記録でよい），中身，根拠（`path:line` か URL）．見つからなかったことは，実行した検索を書く．
- 推奨する案と，退ける案（各1行の理由）．
