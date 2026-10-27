# 設計メモ 42：D9 の本番の直し（DDC/DUC に届く timed の周波数）

Status: owner の決め（2026-10-27）．D9a（R1，R5，S2・F3' の Ceiling，probe P4〜P7）を実装する．D9b（R4，R2，F7）は次の実機の後．

| 項目 | 内容 |
|---|---|
| 段階 | Phase 8．3c-3a の後，3c-3b と 3c-3 の実機より前（owner 2026-10-27） |
| 材料 | Opus の調査 [research/d9-production-investigation.md](research/d9-production-investigation.md)（2026-10-27），Sol の裏取り [reviews/d9-sol-verify.md](reviews/d9-sol-verify.md)，設計メモ [39](39-dsp-timed-commands.md)，[bench-results](bench-results.md) 2026-10-27 の D9 probe |
| risky | はい（装置の queue，時刻つきの command の順，fake と実機の境） |

## 1. 分かったこと（UHD 4.10 の source．Sol が裏取り）

- F1（受信の詰まり）：経路 A（受信を Stop した channel．`channel_live` は Stop を見ない，`control.rs:516-523`，`timeline.rs:941-944`）と経路 B（遠い先の planned segment，`control.rs:532-538`）で，sample の流れない DDC に timed の `SR_FREQ` が溜まる．`booked` は instant を過ぎたものを数えないので深さで止まらない．DDC は満杯で ack を止める（`rfnoc_block_ddc.v:298-319`）．CONFIRMED．
- F3'（送信の burst の途中）：DUC の timed の値は，burst の先頭からその sample まで `has_time` が続くときだけ当たる（`axi_rate_change.v:396-433`，`axi_tag_time.v:207-230`）．uhd-tx は device burst の最初の send にだけ時刻を付ける（`tx.rs:434`；`:442`，`:579` は無し）．DSP の値が変わる tune なら，LO は e で変わり DSP は次の burst まで古い．CONFIRMED（実機は未）．
- F7：DUC の tag は idle でも最後の sample の header で立つ．open で時刻は 0．前の process の最後の時刻つきの header が残ると，新しい Run の最初の burst より前に渡した値がすぐ当たる．機構は CONFIRMED，P2 の「早い」の原因かは UNCLEAR．
- DUC は詰まらないが，溢れると時刻と値の対応が壊れる（`axi_setting_reg.v:55-87`）．害が無いとはしない．
- release window 3 ms は 1 MS/s の 1 例で間に合っただけ（保証ではない）．
- AUTO/AUTO の LO/DSP の配分は MANUAL で再現できるが，分割は要らない．

## 2. owner の決め（2026-10-27）

1. 3c-3a の後に，この直しを risky な段階として立て，3c-3b と 3c-3 の実機より先にする．次の実機の日に P4〜P7 も流す（どれも 8 個以下，送信を含むので att を確かめる）．
2. **R1（受信）**：timed の周波数は，その instant を拒否されていない走行中の segment が覆う受信 chain にだけ装置へ渡す．それ以外は装置に渡さず，記録と投影は今のまま，次にその chain が流れる前の configure が untimed で当てる．configure 済みの先の segment への例外は作らない（Sol）．宛先の segment の configure がもう済んでいる（UR-36 の印．投影の snapshot と同じ lock の中で立てる．最初の segment は prepare が configure したので済んでいるとみなす）なら configure は走り直さないので，その値は release の時に untimed で当てる．宛先は判断のたびに予定から読み直す（拒否されていない，その channel を含む，origin が e より後，長さが 0 でない，のうち最も早いもの）．試験計画は Sol の 4 回の計画レビューで PASS（[reviews/d9a-test-plan.md](reviews/d9a-test-plan.md)）．AUTO/AUTO のまま，設計メモ 39 §3 の LO/DSP の分割はやめる．
3. **R4（送信）**：burst の sample を運ぶ send にはすべてその位置の時刻を付ける．中身の無い緊急の EOB には付けない．採るのは P6 で確かめてから．underrun の後は遅れた packet が捨てられる（`radio_tx_core.v:337-409`）ので，UR-22 まわりを直す．
4. **R2**（受信の装置の stop を渡した周波数の instant の後まで延ばす）は P4 の結果で決める．
5. **F7**：P5 で確かめ，確かめられたら Run の始め（送信の streamer を開いた直後）に untimed の空の EOB を 1 つ送る．Run の終わりではない（異常終了や他の program の後を覆うため）．
6. **S2**（送信の channel が stream を離れて戻ると，残った timed の値が configure を上書き）：Ceiling．FakeDevice の FIFO の model（R5）で同じ振る舞いを再現して試験で固める．
7. D9 の規則 4 を改め，規則は UHD Module（spec 18）に置く．語彙と MockRadio は変えない．
8. risky として進める（指示書の系列 → 試験計画 → Sol の計画レビュー → 実装）．Sonnet は 2026-10-28 17:00 まで，越えたら luna-fast に引き継ぐ．

ほか（owner 2026-10-27）：利用者に見える小さな制約は root の `KNOWN_LIMITATIONS.md`（英語，README から link）にまとめる．1 項目：症状，条件，回避，根拠，状態．spec に利用者に見える Ceiling を足す commit は同じ commit で 1 項目足す（AGENTS.md に 1 行）．

## 3. R5（FakeDevice）

chain ごとに DSP の FIFO を model する．受信は sample が流れる間に 1 sample に 1 つ取り出す．送信は時刻つきの send でだけ取り出し，時刻なしの send が来たらその burst の終わりまで取り出さない．untimed の書き込みは待っている timed の値に上書きされる．DDC は満杯で詰まる（fake では error），DUC は溢れる．open で空．

## 4. bench（次の実機の日）

P4：timed の受信 tune の後，e の 2 ms 前に untimed で stop → 次の受信の最初で残りが当たるか（R2）．P5：時刻つきで終えた burst の後，新しい process で burst より前に timed の DUC → 時刻なしで終えた場合と比べる（F7）．P6：2 ms の chunk をすべて時刻つきで送り 20 ms で shift，切り替わりの位置と H2 の report（R4）．P7（任意）：今の形で F3'．

## 5. 変わる文書

spec 18（§2 の事実，UR-22，UR-24，UR-25，UR-33），設計 33 D9 の規則 1〜4，設計メモ 39（§1.4，F3 の機構，F5 の答え，F7）と research §4（DUC に `axi_tag_time` は有る）．
