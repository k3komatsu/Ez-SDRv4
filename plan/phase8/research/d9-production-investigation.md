# D9 調査：production の DDC/DUC timed command（2026-10-27）

| 項目 | 内容 |
|---|---|
| 依頼 | 設計メモ 39 の D9：production の経路，§3 の推奨の検証，選択肢と推奨，owner の判断 |
| 読んだもの | AGENTS.md，`plan/phase8/39-dsp-timed-commands.md`，`research/x300-dsp-timed-commands.md`，`33-channels-and-streams.md` D9，`38-timed-release-order.md`，`bench-results.md` の D9 probe と log（`~/.cache/ezsdr-bench/2026-10-27/hw_d9_*.log`，`shakeout-2.log`），`crates/ezsdr-radio-uhd/src/`，`crates/ezsdr-radio/src/timeline.rs`，`crates/ezsdr-server/src/catalogue.rs` |
| UHD の source | docker image `ghcr.io/k3komatsu/uhd:v4.10` の `/root/tmp/uhd-4.10.0.0/`（tag `v4.10.0.0` の木と見なす．INFERRED）．以下 `uhd:<path>:<行>` は `https://github.com/EttusResearch/uhd/blob/v4.10.0.0/<path>` に当たる |
| 記法 | **VERIFIED**＝source か bench で確かめた．**INFERRED**＝推論 |
| 範囲 | repository は読むだけ．build も実機もしていない |

## 0. 要約

1. **P2 の「19 ms 早い」は in-flight window と release window の差ではない．** DUC の timed の値は，idle の間も「最後に処理した sample の header（時刻と has_time）」で判定され，その header は reset されない．UHD は open のたびに装置の時刻を 0 にするので，前の process の最後の burst の時刻が新しい instant より大きいと，値は渡した瞬間に取り込まれる（新しい F7）．P1〜P3 の全結果と合う（原因としては INFERRED，機構は source で VERIFIED）．
2. **送信の本当の穴は F3 の「数 ms 遅れ」ではない．** DUC は，burst の先頭からその sample までのすべての packet に時刻があるときだけ timed の値を当てる．uhd-tx は device burst の最初の send にしか時刻を付けない（`tx.rs:434`）．そのため burst の最初の約 2 ms より後の instant では，LO（radio）は e で切り替わるのに DSP は次の burst まで変わらず，残りの burst は誤った周波数で送信される（F3'，source で VERIFIED）．
3. **受信の wedge（F1）は production の 2 つの経路で起きうる**（VERIFIED）．Stop した受信 stream の channel への timed 周波数（`channel_live` は Stop を見ない），および遠い将来の segment 宛ての timed 周波数（`targets`）．FIFO が 32 で詰まると control path が止まることは，shake-out 2 で実際に起きた（DDC#0 の `0x420`＝`SR_FREQ_ADDR`）．
4. **F5：Run の終わりに残った設定は次の Run には持ち越されない．** UHD が open のたびに全 block を flush と reset で空にするからである（VERIFIED）．持ち越されるのは F7 の stale header だけ．
5. **推奨：LO/DSP の分割はしない（AUTO/AUTO のまま）．** 次の 3 つで直す．
   - **R1**：受信では，その instant に sample の流れていない chain に timed 周波数を渡さない．record は今のままで，次の configure が値を当てる．
   - **R4**：送信では，device burst のすべての send に時刻を付ける．
   - **R5**：FakeDevice に DDC/DUC の FIFO の model を足す．

   受信の stop を延ばす R2 と F7 の対処は，bench で必要かどうかを決める．D9 の規則 4 は改め，規則を UHD Module に置く．

## 1. UHD 4.10 の source から分かったこと（note 39 §1 と research の訂正を含む）

| # | 事実 | 根拠 | 確かさ |
|---|---|---|---|
| H1 | DDC の timed `SR_FREQ` は，packet 時刻が instant 以上の**入力 sample**（radio rate）が通るときだけ取り出される．取り出しは 1 sample に 1 つ．FIFO が満杯になると `rb_stb=0` で ctrl の ack が止まる（wedge） | `uhd:fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_ddc/rfnoc_block_ddc.v:298-319`，`.../rfnoc/axi_tag_time.v:207-211`，`.../rfnoc/ddc.v:118`．bench：`shakeout-2.log:274-278`（DDC#0 の `address:0x00420`．`ddc_block_control.cpp:47` の `SR_FREQ_ADDR=132*8`．時刻なしの書き込みも詰まる） | VERIFIED |
| H2 | DDC の untimed 書き込みはすぐ当たる．待っている timed は後の sample で上書きする．`clear`（EOB や rate の変更）は timed FIFO を消さない | `ddc.v:24,130-146`．P1 と合う | VERIFIED |
| H3 | **DUC にも `axi_tag_time` がある**（`dds_timed` の中）．research §4 の「DUC には `axi_tag_time` は無く」は誤り．値は tag が立つときだけ取り出される．tag は（時刻 ≥ instant）かつ（その sample の header の `has_time`） | `uhd:fpga/usrp3/lib/rfnoc/dds_timed.v:140-172,349-354`，`axi_tag_time.v:229-230`．DUC では `dds_timed` は interpolation の後にある（`rfnoc_block_duc.v:291-330`） | VERIFIED |
| H4 | DUC の出力の `has_time` は burst の最初の header から取られる．時刻のない入力 packet が来ると，その後は burst の終わりまで 0 になる | `uhd:fpga/usrp3/lib/rfnoc/axi_rate_change.v:396-433`（特に `410-419`） | VERIFIED |
| H5 | UHD は時刻つきの `send()` の fragment すべてに時刻を付ける．時刻なしの `send()` には付けない | `uhd:host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:297-317` | VERIFIED |
| H6 | DUC の untimed 書き込みは，DDS の入口に空きがあれば（idle なら即）取り込まれる．同じ cycle に timed が取り込まれると捨てられる | `dds_timed.v:349-353,410-426`，`fifo/axi_fifo.v:62-63`（SIZE 0 は flop 1 段），`rfnoc/axi_sync.v:101`．P3 と合う | VERIFIED |
| H7 | DUC は wedge しない（`rb_stb` は 1 固定）．満杯のとき，時刻の FIFO は新しいものを捨て，値は保留の register を上書きする．排出の後は最新の値が残る | `rfnoc_block_duc.v:224`，`axi_tag_time.v:157-189`，`control/axi_setting_reg.v:55-76` | 機構 VERIFIED，残る値は容量（約 34 と 33）からの INFERRED |
| H8 | **stale tag**：DUC の tag は idle の間も `vita_time_now` と，最後の sample の header（flop の `o_tdata`）から組み合わせで出る．これらは reset されない（reset で消えるのは `header_valid` だけ）．最後の sample に時刻があり，その時刻が instant 以上なら，値は渡された瞬間に取り込まれる．時刻の entry は残る．DDC は有効な sample が要るので当たらない | `axi_tag_time.v:114-134,229-230`，`rfnoc/split_stream.v:18`，`fifo/axi_fifo_flop.v:21,30-37`，`dds_timed.v:353`，`ddc.v:118` | VERIFIED |
| H9 | UHD は open（`uhd_usrp_make`）のたびに全 block を flush して reset する（ce domain を含む）．DDC/DUC の FIFO は空になる．装置の時刻も 0 にする | `uhd:host/lib/rfnoc/rfnoc_graph.cpp:108-109,714-722,985-998`，`client_zero.cpp:203-212`，`noc_shell_ddc.v:155-164`，`rfnoc/mb_controller.cpp:122-164` | VERIFIED |
| H10 | property の cache：DSP 周波数を cache と同じ値にすると何も書かない．rate の変更は cache の DSP 値をその時の command time で書き直す | `uhd:host/include/uhd/utils/dirty_tracked.hpp:87-94`，`.../rfnoc/property.hpp:215-219`，`host/lib/rfnoc/duc_block_control.cpp:298-313,348-378`，`ddc_block_control.cpp:331-362` | VERIFIED |
| H11 | `set_command_time` は mboard の全 RX/TX chain に掛かる．DDC/DUC で timed の書き込みを受けるのは `SR_FREQ` だけで，他の register への timed の書き込みは捨てられる．Module は timed の rate を送らない | `multi_usrp_rfnoc.cpp:964-988`，`axi_tag_time.v:91-97`．`control.rs:51-58`（`Timed` は freq か gain だけ） | VERIFIED |
| H12 | radio の送信は，時刻を burst の先頭と underrun の後でだけ見る．間に合っている時刻つきの続きは切れ目なく送られる．遅れた時刻つき packet は `ERR_TX_LATE_DATA` で捨てられる | `uhd:fpga/usrp3/lib/rfnoc/blocks/rfnoc_block_radio/radio_tx_core.v:336-380` | VERIFIED |
| H13 | AUTO は DSP＝（`actual_rf` − clip した f）×sign（RX は +1，TX は −1）．MANUAL は DSP の範囲で clip するだけ．範囲内の要求なら LO/DSP に分けても実効周波数は変わらない | `multi_usrp_rfnoc.cpp:66-67,692-714` | 式は VERIFIED．全 board で範囲内になることは INFERRED |

## 2. bench の P1〜P3 の読み方

- **P1**：H1 と H2 のとおり．untimed の 0 はすぐ当たり，その後，受信の最初の 3 sample（200 MS/s，各 5 ns）で 20・40・60 kHz が順に取り出された．最後の 60 kHz が残った．
- **P3**：
  - burst 1 は動かない．sample の時刻が instant より前なので待つ．
  - burst 2 は先頭の sample で +50 kHz．
  - burst 3 の前の untimed 0 は，idle の間にすぐ取り込まれた（stale の時刻は at(40) で，at(50) より前なので tag は立たない）．burst 3 の先頭で timed の +80 kHz が当たった．H3 と H6 のとおり．
- **P2 の 2 つ目**：e の 3 ms 前に渡した．head（時刻つき send 1 回．tail は時刻なし，`tests/hardware.rs:1006,1014`）を UHD に渡してから約 33 ms 後である（log：`head sent by 105462711`，`dsp at 138828631`）．結果は e ちょうど．+25 µs は解析窓の 25 sample 1 つ分（`hardware.rs:906-915`）．DUC は radio に押し戻されていて先へ進まない．**release window 3 ms で足りる**（VERIFIED，bench）．
- **P2 の 1 つ目**（e の 19 ms 前，burst の先頭から動いた）：in-flight window では説明できない．P3 の burst 1 も同じ「burst より前に渡す」条件だが，早まらなかった．H8 と H9 で説明がつく．
  - P2 は新しい process で，UHD は時刻を 0 にした．
  - P1 の最後の送信は時刻つきの send 1 回で，約 1.3×10⁸ tick で終わった（P1 log の `other` は 133,412,492）．そのため DUC には `vita_time_now≈1.3e8` と `has_time=1` が残っていた．
  - P2 の e は約 5.46×10⁷ tick（`dsp at 20624749` ＋ 30 M ＋ 4 M）．idle の間に tag が立ち，値は渡された瞬間に取り込まれた．
  - P2 の 2 つ目が正しく当たったのは，1 つ目の burst が時刻なしの tail で終わり，stale の `has_time` が 0 になったからである．

  原因としては INFERRED．3 つの probe の結果と，P1→P2→P3 の順に流したこととは合う．
- **production への意味**：
  - (i) TX の timed DUC 書き込みに release window 3 ms は足りる．
  - (ii) e の sample は時刻つきの packet になければならない．production はそれを満たしていない（F3'）．
  - (iii) 各 Run の最初の burst の前に渡した TX の DSP は早まりうる（F7）．Run ごとに open して時刻を 0 にする（`catalogue.rs:121-136` → `lib.rs:73` → `uhd.rs:487`．さらに `authority.rs:372`）．

## 3. production の経路（成果物 1）

UHD を呼ぶ所は 3 つ．timed は `uhd.rs:864-867`（command time）＋`uhd.rs:886-887`（AUTO/AUTO）で，`control.rs:718`（`give`）から呼ばれる．untimed は同じ行で，`core.rs:1087`（configure）から呼ばれる．`tune_dsp`（`uhd.rs:518-553`）は probe 専用である．
- grep：`grep -rn "tune_dsp\|uhd_usrp_set_rx_freq\|uhd_usrp_set_tx_freq" crates/ --include=*.rs`．tests 以外では `uhd.rs:180,192,521,541-542,886-887` だけ．
- timed の rate は通らない（`control.rs:51-58`）．gain と antenna は radio だけ（research §2，H11）．
- stream command は DDC を通って radio へ行く．DDC の entry を作るかは確かめていない（INFERRED：作らない）．

| # | 系列（短く） | UHD 呼び出し | 起きること | F | 確かさ |
|---|---|---|---|---|---|
| A | 受信 stream を `Stop` した後，その channel に timed 周波数を繰り返す | `control.rs:384-414`（Stop は取り消さない）→ `channel_live` は真（`control.rs:516-523`．`halted` は拒否だけを見る：`timeline.rs:349-354`．Stop は `on=false` にするだけ：`timeline.rs:941-944`）→ `give` `control.rs:631` → `control.rs:718` → `uhd.rs:866,886` | DDC に entry が溜まり，restart までは取り出されない．`booked` は instant を過ぎた entry を数えない（`control.rs:575-585,607-608`）ので，queue の深さ 16（`x3xx.rs:397`）では止まらない | **F1**（wedge，電源の入れ直し） | VERIFIED（code＋H1）|
| B | channel が e で stream に無い（layout の外，halt，cut と次の segment の間）．遠い将来に planned segment があり，そこへ timed 周波数を繰り返す | `targets` が後の segment を返す（`control.rs:532-538`）→ `control.rs:631,718` → `uhd.rs:886` | segment の最初の sample まで entry が溜まる | **F1** | VERIFIED |
| C | live な A を渡した後，e の前で cut する（Stop，cold，拒否）．device の stop が e より前に効くと A が残る．後に「渡さない」値（no channel）があると，restart の configure を A が上書きする | stop は untimed（`rx.rs:504-508,635-654`）．configure は `core.rs:1087` → `uhd.rs:886` | 残った A が最初の sample で当たる．値が投影と食い違うのは，後に渡さない値があるときだけ | **F2** | 経路は VERIFIED．stop が e より前に効くかは INFERRED：radio の ctrl FIFO は順番どおりなので，timed の SPI の後ろで stop が e まで待つかもしれない（F6．research §3） |
| D | uhd-control の enable from none が configure した segment に timed で渡す．その segment が始まる前に cut される | `control.rs:816-`（configure）→ `targets` → `control.rs:718` | C と同じく残る．稀 | F2 | 経路は VERIFIED，起きる頻度は INFERRED |
| E | `Provider::stop` で，e が `S` より後の entry を渡したまま | `settle` は記録だけ（`control.rs:741-777`） | 次の open で FIFO は空になる（H9）．同じ Run の中には影響しない | F5（実害なし） | VERIFIED |
| F | **送信 burst の 2 回目以降の send の範囲に e が入る timed 周波数**（chunk は 2 ms 以下：`tx.rs:404`） | 時刻は `device_sob` の send だけ（`tx.rs:434`）．tail（`tx.rs:442`）と続きは時刻なし．release は `control.rs:718` → `uhd.rs:887` | LO は e で切り替わり，DSP は次の device burst の先頭まで変わらない．残りの burst は「新しい LO＋古い DSP」で送信される | **F3'**（新．note 39 の F3 は機構が違う） | VERIFIED（H3，H4，H5）．bench はまだ |
| G | burst の間に timed 周波数を渡す（TX は常に `channel_live`：`timeline.rs:941`） | `control.rs:718` → `uhd.rs:887` | 次の burst の最初の sample で当たる．効果は正しい．間の configure（untimed）とも，投影の順と一致する | F4（害なし） | VERIFIED（P3） |
| H | G の後，burst を出さないまま channel が送信 stream を離れ，no channel の値を挟んで戻る | G ＋ `core.rs:1087` | 残った entry が，戻った後の最初の burst で configure を上書きする | F2（S2．稀） | 経路は VERIFIED |
| I | burst のない間に送信の timed 周波数を 34 個以上 | 同上 | DUC は wedge しない．次の burst で最新の値が残る | F1 の TX 版（害は小さい） | INFERRED（H7） |
| J | Run の最初の burst より前に渡した送信の timed 周波数．前の session の最後の DUC packet が時刻つき | `control.rs:718`．時刻を 0 にするのは H9 と `authority.rs:372` | 値は渡した瞬間に取り込まれる．e が最初の burst の最初の約 3 ms に入るときだけ見えて早まる．残った時刻の entry が次の値を早めることがある | **F7**（新） | 機構は VERIFIED．P2 の 1 つ目の原因としては INFERRED |
| K | configure の rate 変更が cache の DSP 値を untimed で書き直す | `uhd.rs:869-874` → H10 | 投影と一致するので害なし | — | VERIFIED |
| L | timed の SPI が release window の中にあると，後ろの stream command が待つ | `control.rs:718` と `rx.rs:332`／`rx.rs:641` | start や stop が最大 3 ms 遅れる | F6 | INFERRED（今のまま） |

## 4. note 39 §3 の推奨の検証（成果物 2）

1. **P2 の早まりは規則と合うか**：合わない．1 つ目は「sample より先に，sample が必ず通る chain へ」という §3 の条件を満たしていたのに早まった．原因は F7 で，§3 の規則では防げない．2 つ目は規則のとおりに当たった．release window で足りる．
2. **「sample が必ず通る」を Module は知れるか**：
   - 受信は知れる．plan の segment の origin，cut，refused，channel の集合から「e を覆う segment があるか」が決まる（`targets` の材料，`control.rs:532-538`）．
   - 例外は release の後に予約される cut（経路 C）だけ．release の 3 ms 以内に限られる．
   - 送信は知れない．burst は別々に予約され，後から来ることも，取り消されることもある．ただし DUC は wedge せず，渡す順＝投影の順なので，「通るときだけ渡す」は要らない．
3. **次の受信開始が timed のとき**：問題ない．deferral した値は投影にあり，owner の `begin` が `rx_start(origin)` の前に untimed で configure する（`rx.rs:306,332`，`core.rs:1237-1289`．`owner_step` は segment ごとに configure する：`core.rs:599-611`）．
   - DDC の untimed はすぐ当たる（H2）．LO は start lead（50 ms，`x3xx.rs:400`）の間に落ち着く．
   - 例外：uhd-control が既に configure した segment（enable from none，`Owner::Nothing`）では，その後の値が configure に入らない．こういう segment には今どおり timed で渡す必要がある．entry は最初の sample で取り出される．数は start lead の中で queue の深さまで．
4. **送信の規則（P2 を踏まえて）**：
   - release は今のまま（instant の順，3 ms 前）．
   - **device burst のすべての send に時刻を付ける**（R4）．付ければ DUC は burst のどこでもその sample で当たる（H4，H5，H12）．
   - gap の値は次の burst の先頭で当たる（P3）．
   - 残るのは F7 と，稀な S2．
5. **分割で実効周波数は変わるか**：変わらない（H13）．
   - ただし分割は要らない．流れていない受信 chain には LO も DSP も渡さず，次の configure（untimed の AUTO/AUTO）に任せればよい．送信は今どおり AUTO/AUTO の timed で，LO は e に，DSP は次の時刻つき sample に当たる．
   - 分割すると，次の 3 つを持ち込む．2 回の呼び出し，sign と clip の再現，UHD の cache の「同じ値は書かない」への依存（H10）．
6. **F5（Run の終わりの残り）とゼロの保証**：
   - FIFO の中身は，次の open（Run ごと）で必ず空になる（H9）．Module が Run の終わりに消す必要はない．
   - 残るのは DUC の stale header（H8）だけで，これは F7 の元である．
   - Run の中では，R1（と必要なら R2）で「stream より長く生きる DDC の entry」を無くす．

## 5. 選択肢と推奨（成果物 3）

### 推奨：R1＋R4＋R5．R2 と F7 は bench で決める．AUTO/AUTO のまま

- **R1（受信）**：受信の timed 周波数を device に渡すのは，次のどちらかのときだけ．
  - (a) channel の stream の，拒否されていない planned segment が e を覆う（origin ≤ e で，cut の instant より前）．
  - (b) 宛先の segment の configure がもう済んでいる（record の印．UR-36）．

  それ以外は device を呼ばない．record（`applied` の行，`live`，`released_at`，settle）と投影は今のままで，次の configure が値を当てる．gain は今のまま（radio だけ）．
  - 経路 A と B（F1）が消える．DDC の entry は「覆われている」か「始まる直前」に限られ，queue の深さで抑えられる．
- **R4（送信）**：device burst のすべての send に，最初の sample の時刻を付ける．対象は本体（`tx.rs:434`），held tail（`tx.rs:442`），閉じる EOB（`tx.rs:579`）．`UhdDevice::tx_send` は `at.is_some()` をそのまま `has_time_spec` にする（`uhd.rs:1063-1088`）ので，tx.rs だけの変更である．
  - 経路 F（F3'）が消える．underrun の後の扱いは変わる．遅れた packet は時間を詰めずに `ERR_TX_LATE_DATA` で捨てられる（H12．Stream Contract の「gap は詰めない」に近い）．
- **R5（FakeDevice）**：chain ごとに DSP の FIFO を持つ（`device.rs:1061-1095` は今，timekeeper の意味ですべてを instant で当てていて，実機より緩い）．
  - 受信は，流れている間の sample で 1 つずつ取り出す．
  - 送信は，時刻つきの send の sample でだけ取り出す．時刻なしの send が来たら，その burst の終わりまで has_time を 0 にする．
  - untimed はすぐ当て，待っている timed で上書きする．
  - DDC は満杯で wedge させ，以後の呼び出しを `op_timeout` にする．DUC は満杯で最新の値を残す．
  - open で空にする．
- **R2（条件付き）**：uhd-rx の untimed stop を，その stream の channel に渡した受信周波数の最新の instant を device の時刻が過ぎてから出す（`stop_at_cut`，`rx.rs:635-654`．待ちは最大で release window）．bench の P4 で，radio の ctrl FIFO が既にこの順を守っている（stop が待っている timed の SPI の後ろで待つ）と分かれば要らない．
- **F7（条件付き）**：P5 で再現したら選ぶ．
  - (i) Ceiling として記録する．
  - (ii) Run の終わりに，各送信 stream へ時刻なしの空の EOB を送る．0 の sample が 1 つ出て，stale の `has_time` が 0 になる．
  - (iii) Run の開始で，装置の時刻を前の session より先へ進める（UR-7 の epoch を変える）．

  推奨は (i) か (ii)．

### 退けた案（利点／欠点）
- **note 39 §3 のとおり（分割，DSP は流れる所だけ，受信 stop の延長，送信は P2 次第）**
  - 利点：流れていない chain でも LO が早めに落ち着く．
  - 欠点：分割で UHD の呼び方が増える（H10，H13 への依存）．送信の「流れる」は知れない．F3' と F7 は直らない．
- **configure が DSP を origin で timed に書く**（残った entry を後ろから上書きする）
  - 利点：受信 stop の延長が要らず，S2 も消える．
  - 欠点：FIFO は届いた順である．送信の configure は cut の時点（UR-23）で行われ，owner が遅れることもあるので，e > origin の entry が先に届くと head-of-line で順が崩れる．分割と profile の制約（start lead ≥ window）も要る．
- **segment の開始ごとに合成の DSP command を release の列に入れる**
  - 利点：instant の順が構成上保たれる．
  - 欠点：design 38 で固めた release に，記録しない種類を足す．UR-12 の read-back も変わる．
- **chain ごとに数え，満杯を `COMMAND_QUEUE_FULL` にする**
  - 利点：経路を変えない．
  - 欠点：装置の内部を見せ，Mock との parity を崩す．F2 は直らない（note 39 §4 と同じ）．
- **LO だけを timed にする（`dsp NONE`）**
  - 利点：FIFO を使わない．
  - 欠点：LO の分解能の残りだけ周波数がずれる．
- **流れていない chain への timed 周波数を拒否する**
  - 利点：単純．
  - 欠点：MockRadio は受けて `issued` を記録する（UR-24 の試験「stopped: the gain is issued」）．parity が崩れる．
- **送信の DSP を uhd-tx が持つ**
  - 利点：S2 と gap での溜まりが消える．
  - 欠点：thread をまたぐ経路と，uhd-tx からの装置呼び出しが増える．よくある系列の正しさには要らない．
- **送信を時刻なしのまま，周波数を burst の境目だけにする**
  - 利点：tx.rs を変えない．
  - 欠点：burst 内のホッピングを失う．hardware_timed の約束を狭める．

### 最小の `src/` の変更（見積もり）
| ファイル | 関数 | 内容 | 規模 |
|---|---|---|---|
| `provider/control.rs` | `targets`，`release`（`527-539`，`601-647`） | 受信周波数で (a)(b) でなければ device を呼ばない．行と投影は今のまま．`covers(dir, chan, e)` を plan から作る | 約 40 行 |
| `provider/tx.rs` | `send`，`close_device_burst`（`403-527`，`572-582`） | すべての send に時刻を付ける | 約 10 行 |
| `device.rs` | `FakeDevice::apply`，`tx_send`，`rx` の sample 生成 | R5 の FIFO model | 約 150 行 |
| （P4 次第）`provider/rx.rs`，`core.rs` | `stop_at_cut` | chain ごとの「渡した受信周波数の最新 instant」で stop を待たせる | 約 25 行 |
| （P5 次第）`provider/mod.rs`／`tx.rs` | `Provider::stop` | 時刻なしの空の EOB | 約 10 行 |

### 固める試験（FakeDevice の系列．risky：指示書で表にする）
1. 受信 stream を 10 ms で Stop し，20〜59 ms に timed 周波数を 40 個渡す → DDC の entry は 0，`op_timeout` は無い．100 ms の `start_rx` で最初の block が最後の値，行は今どおり `issued`．
2. 1 s の cold で channel を足し，その前に 40 個渡す → configure までは entry 0．segment は最後の値で始まる．
3. enable from none の configure の後，origin より前の timed 周波数 → timed で 1 つ渡り，最初の sample で取り出される．
4. 走っている segment の中の timed 周波数 → e の sample で切り替わる（DDC model）．
5. live の A を渡した後，cut で stop が e より前，no channel の B，restart → R2（か P4 の機構）がなければ最初の block は A（壊れることを示す）．あれば B．
6. 60 ms の burst を chunk で送り，20 ms の timed 送信周波数 → R4 があれば 20 ms ちょうど．無ければ次の burst まで変わらない（has_time の model）．
7. burst の間の timed（＋その間の cold で configure）→ 次の burst の先頭で投影の値．
8. burst のない間に 40 個 → 次の burst は最後の値，wedge しない．
9. Run の終わりに entry を残す → 次の open で空．

### 変わる spec と規則
- spec 18：
  - §2 に H1〜H9 と H12 の事実を足す．
  - **UR-22**：repetition も含め，すべての send に時刻を付ける．
  - **UR-24**：受信周波数の release の条件．行は変えない．
  - UR-25：configure が deferral した値を当てることを明記する．
  - **UR-33**：FakeDevice の DSP FIFO．
  - （R2 を採れば）UR-17／UR-25 の stop．
  - 試験の表の `ur_24_a_release_without_a_live_channel_reaches_the_pending_segment`：segment が未 configure なら timed の apply は無くなる．
- spec 07（RM-*）と spec 09（MR-*）：変えない．見える振る舞いが変わらないため．
- 設計 33 の D9：
  - 規則 1 → R1．送信は「burst が開いていて in-flight の中」を外す．
  - 規則 2 → 受信だけ，未 configure の宛先だけ．
  - 規則 3 → R2 か P4．
  - 規則 4 → UHD Module に置く．
- メモ 39：§1.4 と F3 の機構，F5 の答え，F7 を足す．research §4 の「DUC に `axi_tag_time` は無い」も直す．

### bench で決めること（どれも entry 8 個以下．送信は att の確認が先）
- **P4**：timed の受信 tune（e）を渡した後，e の 2 ms 前に untimed で stop する．最後の受信 sample が e を越えるか，restart で古い DSP が出るか → R2 の要否．
- **P5**：時刻つきの send 1 回で送った後，新しい process で burst より前に timed DUC を渡す．前の burst を時刻なしで終えた場合と比べる → F7 の確認．
- **P6**：60 ms を 2 ms の chunk ですべて時刻つきで送り，20 ms の DUC shift を見る．H2（underflow mid-burst）の report の変わり方も見る → R4．
- **P7（任意）**：今の uhd-tx の形（先頭だけ時刻つき）で F3' を確かめる．

## 6. owner に決めてもらうこと（1 項目に 1 つ）
1. R1 を採るか（流れていない受信 chain への timed 周波数は device に渡さず，次の configure が当てる．record は変えない）．
2. R4 を採るか（送信の全 send に時刻を付ける．underrun の後は時間を詰めずに捨てる．P6 の後で決める）．
3. 実装の前に P4〜P7 を X300 #1＋OBX で流すか（推奨．送信を含む）．
4. R2（受信 stop を待たせる）は P4 の結果で決めてよいか．
5. F7 の扱い：Ceiling，時刻なしの空の EOB，epoch の変更のどれにするか（P5 の後）．
6. S2（送信 chain が stream を離れて戻る間の残り）を Ceiling にするか．代わりに，送信の timed 周波数を stream が無くても物理 chain に渡すか．
7. D9 の規則 4 を改め，規則を UHD Module に置くか．note 39 の LO/DSP の分割はやめるか．
8. この段階を risky として進めるか（指示書の系列 → 実装役の試験計画 → Sol の計画レビュー）．

## 7. 確かめていないこと
- X300 の FPGA image（`39.3`，git `d375d68`）がこの木の `rfnoc_block_ddc/duc` と同じかは INFERRED．FIFO の正確な容量（`axi_fifo_short` の 32）も INFERRED．
- DUC と radio の間の buffer の大きさは測っていない．P2 からは 3 ms より小さいとしか言えない．
- DDC の stream command の handler が DDC に entry を作らないことは読んでいない．
- R1 の「configure 済み」の判定と，owner の begin との lock の順（`give` は `updates` の lock を持ったまま呼ぶ：`control.rs:705`）は，指示書の系列で固める必要がある．

確かめていないこと：上の 4 点と，P4〜P7 の実機の結果．risk：F3'（送信 burst 内の周波数の誤り）と F1（受信の wedge）は，今の code で起きる．
