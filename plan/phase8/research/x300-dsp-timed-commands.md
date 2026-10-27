# X300 の DDC/DUC と timed command（UHD 4.10）

Sonnet researcher，2026-10-24，読むだけ．設計メモ [39](../39-dsp-timed-commands.md) の材料．ファイルはどれも tag `v4.10.0.0` を `https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/<path>` で読んだ（取得の道具が行番号を落としたので，引用と関数名で示す）．Opus は結論を Phase 7 の bench（`plan/phase7/bench-results.md:439-452`）と照らした．

## 1. 資料

`host/docs/devices/usrp_x3x0.dox`，"Multiple Timed Command Advisory"（`x3x0_misc_timed_cmds_lockup`），VERIFIED：
- "In the HG and XG images, the DDC & DUC derive their sense of time from the header of passing packets."
- "Repeatedly issuing timed commands without streaming will result in the command queue of the DDC / DUC backing up and overflowing, putting the device in a state where a full restart is required."

## 2. 1 つの timed tune が届く所（`host/lib/usrp/multi_usrp_rfnoc.cpp`），VERIFIED

- `tune_xx_subdev_and_dsp`：RF を設定 → RF の read-back → DSP の目標 `actual_rf_freq - clipped_requested_freq`（AUTO）→ DSP を設定．`dsp_freq_policy` が `POLICY_NONE` なら DSP の `set_freq` は呼ばれない（`case tune_request_t::POLICY_NONE: break; // does not set`）．
- `set_rx_freq`：`rx_chain.rf_core->set_rx_frequency(...)` の後に `rx_chain.ddc->set_freq(...)`．`set_tx_freq` は DUC で同じ．
- `set_rx_gain`／`set_tx_gain`：`rf_core` だけ．DDC/DUC に触らない．
- `set_command_time`：各 chain の radio と，あれば DDC（RX）・DUC（TX）に．
- よって AUTO/AUTO の timed tune は，radio への timed の書き込み（daughterboard の SPI）と，DDC か DUC への timed の周波数の書き込み 1 つになる．

## 3. radio（`host/lib/usrp/x300/x300_radio_control.cpp`，`rfnoc_block_radio.v`，`ctrlport_timer.v`），VERIFIED

- daughterboard の SPI は `regs().poke32(addr, data, get_command_time(0))`：timed で radio の ctrlport へ．
- radio の ctrl FIFO は 512 word（`radio.yml` の `fifo_depth: 512`）．`ctrlport_timer` は `radio_time`（timekeeper）で動き，`exec = pending && (!has_time || on_time || ...)`：時刻待ちの command はその後ろを全部止めるが，sample は要らない．
- RX の stream command（`issue_stream_cmd`）は DDC を素通りして radio に行き，radio が `REG_RX_CMD_TIME_*` を書く（untimed の書き込みで，時刻は radio の中で扱う）．radio の ctrl FIFO で timed の SPI の後ろに並ぶ（INFERRED）．

## 4. DDC/DUC（`rfnoc_block_ddc.v`，`rfnoc_block_duc.v`，`axi_tag_time.v`，`dds_timed.v`）

- host：`ddc_block_control::_set_freq` と `duc_block_control` は `poke32(SR_FREQ_ADDR, ..., get_command_time(chan))`，VERIFIED．
- DDC の時刻は `axi_tag_time` の `vita_time_now`：packet の CHDR header の時刻で読み込み，sample ごとに進む．timekeeper は無い．待っている設定は `vita_time_now >= fifo_set_time` の sample が通るときだけ取り出される，VERIFIED．sample が来なければ永久に残る．
- 深さ：`CMD_FIFO_SIZE = 5`（log2）で 32（parameter は VERIFIED，`axi_fifo` の SIZE の意味は INFERRED）．
- DDC が満杯：`rb_stb = ~timed_cmd_fifo_full` で書き込みの ack が止まり，DDC の ctrl の道（64 word）が詰まる．untimed の書き込みも同じ道なので止まる．host は `MASSIVE_TIMEOUT = 10.0` s の後 `op_timeout`，VERIFIED．
- untimed の書き込みは `axi_tag_time` の FIFO を通らずにすぐ当たる，VERIFIED．待っている timed の設定は，後で sample が通ると同じ register を上書きする（INFERRED：同じ `SR_FREQ_ADDR`）．
- DUC：（訂正，設計メモ 42：DUC にも `axi_tag_time` は有る．`dds_timed.v` の中で，tag は時刻が instant 以上で，かつその sample の header の `has_time` が立つとき）`axi_setting_reg set_freq_timed`（FIFO 32 は INFERRED）．`phase_inc_timed_tready ? phase_inc_timed_tdata : phase_inc_tdata` で，timed の値が来ると untimed より優先，VERIFIED．満杯でも ack を止めない（`rb_stb` は常に 1）．満杯の書き込みがどうなるかは読んでいない（INFERRED：捨てられる）．
- DUC の時刻は TX の packet の header から（INFERRED）．

## 5. Phase 7 の bench との照合（INFERRED）

`hw_b8_raw_queue_depth`：何も流さずに 3 s 先の timed tune を 8 個．7 個はすぐ返り，8 個目が約 3 s 待った．10 s の `op_timeout` にならず時刻で抜けたので，詰まったのは radio の ctrl FIFO（1 tune で 11〜12 の timed SPI の書き込みと見積もる）．DDC には 7 個の timed の周波数の設定が残っていたはずで，その後の untimed の tune は FIFO を通らないので返った．

## 6. 読んでいないもの

`ddc.v`，OBX の daughterboard の driver（1 tune の SPI の数），`axi_fifo`，block ごとの `ctrl_max_async_msgs`，host の `multi_usrp_rfnoc.cpp` の最後の 20 KB．
