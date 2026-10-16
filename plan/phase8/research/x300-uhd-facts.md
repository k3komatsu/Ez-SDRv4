# X300 under UHD 4.10: facts for the P1 (c) spec

Sonnet researcher, 2026-10-16, read-only. Brief: the orchestrator's prompt (eight questions on dual 10 GbE, timed-command queues, DDC/DUC, subdev mapping, stream commands, device time, board-combination limits, sustained rates). Not yet checked by the orchestrator; VERIFIED here means the researcher read it.

Line numbers are not available for any finding. WebFetch returns converted text and strips the line gutter, and this session has no shell to clone or `grep -n` the tag. So no `#Lx-Ly` anchors are given. Each finding names the file and function, and each URL points at the whole file at v4.10.0.0. The few line numbers I give for `usrp_x3x0.dox` are approximate and may be off by one or two. Quotes are verbatim, and the fetch tool limited me to short ones.

Base URL for all files: `https://github.com/EttusResearch/uhd/blob/v4.10.0.0/`. Abbreviation: dox = `host/docs/devices/usrp_x3x0.dox`.

## 1. Dual 10 GbE link assignment

- **Static topology, VERIFIED.** The XG and HG yml files differ only in `image_core_name` and `default_target`.
  - Each has two radios (`radio0`, `radio1`), each with `NUM_PORTS: 2`.
  - Each has `duc0`/`duc1` with 1 port and `ddc0`/`ddc1` with 2 ports.
  - Each has four stream endpoints (SEPs) carrying radio data: ep0 and ep1 for slot A, ep2 and ep3 for slot B. Both files also have ep4 and ep5 for replay.
  - ep0 carries slot-A TX and RX0, ep1 carries slot-A RX1, ep2 carries slot-B TX and RX0, ep3 carries slot-B RX1.
  - Files: `fpga/usrp3/top/x300/x300_{HG,XG}_rfnoc_image_core.yml` and `fpga/usrp3/top/x300/yaml_include/x3xx_radio_base.yml`.
- **Link choice is per stream connection, not per radio block or slot, VERIFIED.**
  - `host/lib/rfnoc/graph_stream_manager.cpp`, `_check_dst_and_find_src()`, is called by both `create_device_to_host_data_stream()` and `create_host_to_device_data_stream()`.
  - If the caller passes no adapter id (`NULL_ADAPTER_ID`), it picks, among the local devices that can reach the SEP, the adapter with the fewest TX allocations (TX) or RX allocations (RX). It starts from `src_devs[0]`, and the comparison is strict `<`, so ties go to the lower device id.
  - The counters `_alloc_map[adapter].rx++` and `.tx++` only increment and never decrement.
  - The code carries a "TODO: Maybe we can come up with a better heuristic".
  - So it is a fewest-streams-so-far heuristic, not strict round-robin and not a stream arg.
- **A caller can force a link, VERIFIED.** The `rfnoc_graph::connect(..., adapter_id)` overloads in `host/lib/rfnoc/rfnoc_graph.cpp` take an `adapter_id`. If the id does not match any reachable link, `_check_dst_and_find_src()` throws `rfnoc_error` ("unreachable from the via device").
- **multi_usrp never passes an adapter id, VERIFIED.** `host/lib/usrp/multi_usrp_rfnoc.cpp`, `get_rx_stream` and `get_tx_stream`, call `_graph->connect(...)` without one. The only stream arg read is `args["streamer"]=="replay_buffered"`. So through multi_usrp the link is chosen automatically and cannot be configured.
- **A streamer can span links, VERIFIED.**
  - `connect()` is made once per streamer port (`connect_channel()`, `connections[strm_port]`), and each connect allocates independently.
  - Two channels of one streamer can therefore land on different links.
  - For example, ch0 can take link 0 and ch1 link 1 when the counters are 0/0 and the first allocation increments link 0.
- **Device discovery, VERIFIED.** `host/lib/usrp/x300/x300_eth_mgr.cpp`, `discover_eth()`, builds the connection list from `addr` and `second_addr`.
  - An address's index is mapped to `X300_IFACE_ETH0` or `ETH1` by the parity of its MB EEPROM IP index.
  - `get_links()` is "one device id to one connection".
  - A source comment says "We should probably do the load balancing elsewhere".
  - There is a note about avoiding two TX streams through one DMA FIFO, which it "punts on".
  - The link list is known only after discovery, so a connection with one address has one link and the heuristic has no choice.
- **HG image, INFERRED.** With one address there is one adapter, so all streams, including every SEP, use that link. I did not read the explicit "HG ignores second_addr" code.
  - The doc says dual 10 GbE requires the XG image: dox lines ~171-172, "ensure either the XG image is installed". Dual-link use is described only for XG.
- **Hardware, VERIFIED.** `fpga/usrp3/top/x300/bus_int.v` instantiates `eth_interface0` and `eth_interface1`. Both feed the same image core (`s_eth0_*`/`s_eth1_*`).
  - The crossbar is inside `rfnoc_image_core`, which I did not read.
  - So any SEP is reachable from either port. That is consistent with the host `_src_map`, which lists every device that can reach each SEP.

URLs: `host/lib/rfnoc/graph_stream_manager.cpp`, `host/lib/rfnoc/rfnoc_graph.cpp`, `host/lib/usrp/x300/x300_eth_mgr.cpp`, `host/lib/usrp/multi_usrp_rfnoc.cpp`, `fpga/usrp3/top/x300/bus_int.v`, `fpga/usrp3/top/x300/x300_XG_rfnoc_image_core.yml`.

## 2. Timed command queue

- **The queue is per RFNoC block, VERIFIED (doc).** The manual page `page_timedcmds` says "In RFNoC devices, each RFNoC block has its own command queue." and "Older devices have fewer (or only one command queue)." It also says "When the command queue is full, UHD will block until the USRP has signaled that there is space for more commands."
  - URL (4.9 build, since the 4.10 build shows the same text): https://uhd.readthedocs.io/en/uhd-4.9/page_timedcmds.html.
  - The `.dox` source at the tag was a 404 under the paths I tried, so this is the built doc, not the tag file.
- **Depth is an FPGA parameter set in the block yml, not a host constant, VERIFIED.**
  - `host/include/uhd/rfnoc/blocks/radio.yml`, `control:` section: `fpga_iface: ctrlport`, `timed: True`, `has_status: True`, `fifo_depth: 512`.
  - The RFNoC FPGA spec (https://uhd.readthedocs.io/en/latest/page_rfnoc_fpga.html) defines `fifo_depth` as "Depth of the input AXI-Stream Control FIFO in words".
  - So the queue is the block's input control FIFO. Per block, it holds 512 words.
  - The number of commands that fit is INFERRED from the words per control packet. I did not read that conversion.
  - `ctrlport_endpoint.v` has `SLAVE_FIFO_SIZE` (default 5), which is an unrelated slave request FIFO. The fetch of `ctrlport_timer.v` and `noc_shell.v` returned 404.
- **Sharing, INFERRED from the VERIFIED per-block ctrl FIFO.** Per-block means one queue per Radio block, so per slot.
  - The two channels of a slot share it. DDC0 and DUC0 each have their own queue.
  - Stream commands go to the DDC (`issue_stream_cmd` in `multi_usrp_rfnoc.cpp` sends to `rx_chain.ddc` if present). DDC `set_freq` uses `set_command_time(time, chan)` on the DDC.
  - Radio tune and gain are not timed in `host/lib/rfnoc/radio_control_impl.cpp`. `set_rx_frequency` and `set_rx_gain` only cache values. The summary I got of this file shows no command-time use there.
  - The X300 radio's dboard SPI and register pokes use `get_command_time(0)` (`host/lib/usrp/x300/x300_radio_control.cpp`).
  - So stream, tune and gain commands do NOT all share one queue. A stream start goes to the DDC queue and radio-register or dboard SPI traffic goes to the radio queue.
- **Backpressure, VERIFIED (dox lines ~848).** "Repeatedly issuing timed commands without streaming will result in the command queue of the DDC / DUC backing up." This is the X300 dox statement that the DDC and DUC queues back up, and it confirms the DDC and DUC have their own queues.

## 3. DDC and DUC per radio, and rates

- **DDC, VERIFIED.** The yml has `ddc0` and `ddc1`, each with `NUM_PORTS: 2`, in both HG and XG. That gives 2 DDC chains per slot and 4 in total.
  - The wiring is `radio0 out_0 -> ddc0 in_0 -> ep0` and `radio0 out_1 -> ddc0 in_1 -> ep1`.
  - The two ports of a DDC are independent in `host/lib/rfnoc/ddc_block_control.cpp`.
    - `decim`, `samp_rate`, frequency and scaling are per-port vectors set by `_register_props(chan)`.
    - Each port has its own register bank (`REG_CHAN_OFFSET` 2048).
    - Decimation and rate resolvers capture `chan`.
  - Only two things are shared: the list of valid decimations (`_valid_decims`) and the halfband and CIC limits.
  - So frontends "0" and "1" of a BasicRX can run at different rates, and a different decimation per port is allowed.
  - They are in different streamers or channels.
  - The radio input rate is the master clock rate (`set_rate` in `x300_radio_control.cpp`: "tick rate can't actually be changed at runtime").
- **DUC, VERIFIED.** `duc0` and `duc1` each have `NUM_PORTS: 1`. The inputs `radio0 in_1` and `radio1 in_1` are explicitly marked `_device_ _none_` ("deliberately not connecting"). So the FPGA image has one TX chain per slot.
  - This contradicts the old doc line (dox ~28-29, ~825: "2 TX DUC chain", "two complete DDC and DUC DSP chains").
  - The yml is the authority, the dox line is stale.
- **TX channel count per slot, VERIFIED.** In `x300_radio_control.cpp`, `_init_db()` accepts 1 or 2 TX or RX frontends and sets ports to match.
  - Source comment: "Note: The Basic and LF boards have two possible frontends on the rx side, and one on the tx side."
  - `host/lib/usrp/dboard/db_basic_and_lf.hpp` has `rx_frontends{"0","1"}` and `tx_frontends{"0"}`.
  - So BasicTX and LFTX give 1 TX channel per slot, not two.
- **multi_usrp harmonises rates within a streamer, VERIFIED.** `multi_usrp_rfnoc.cpp`, `get_rx_stream`: if a channel's rate differs, it logs "Inconsistent RX rates when creating streamer! Harmonizing to" and re-applies one rate to all channels. TX is the same.
  - The raw `rfnoc_rx_streamer` does not check rate equality (it only checks that channels are connected).
  - I saw no check that a radio or DDC is already used by another streamer.

## 4. Subdev specs and channel mapping

- **Frontend names, VERIFIED.** The default subdev-name list in `host/include/uhd/usrp/dboard_manager.hpp` is `std::vector<std::string>(1, "0")` for all four `register_dboard` overloads.
  - UBX is registered with no list (`host/lib/usrp/dboard/db_ubx.cpp`), so one frontend named "0".
  - Basic/LF RX use "0" and "1" (`db_basic_and_lf.hpp`), and TX uses "0".
  - Property-tree names are `UBX RX`, `UBX TX`, `BasicRX (<subdev>)`, `BasicTX (<subdev>)`, `LFRX (...)`, `LFTX (...)`.
  - CBX and OBX: I did not read the registration lines. CBX is INFERRED to be "0". The tool read the OBX registration as a single call `register_dboard(OBX_RX_ID, OBX_TX_ID, &make_obx, "OBX")` with no subdev list, so "0" by the default.
- **RFNoC antennas for Basic/LF, VERIFIED.** `db_basic_and_lf.cpp`: the RFNoC IDs `BASIC_RX_RFNOC_PID` etc. use `rx_frontends`/`tx_frontends`, and `ant_mode` is always "AB". `antenna_mode_to_conn` is `{"AB","IQ"},{"BA","QI"},{"A","I"},{"B","Q"}` in `db_basic_and_lf.hpp`.
  - So frontend "0" and "1" are both real-signal inputs of one RFNoC board with antenna mode "AB".
- **Channel to frontend, VERIFIED (partly).** `x300_radio_control.cpp`: `get_chan_from_dboard_fe()` and `get_dboard_fe_from_chan()` map the frontend name to the radio port (the `_rx_fe_map` entry is filled by `_set_rx_fe`).
  - `multi_usrp_rfnoc.cpp`: the constructor builds one default subdev spec per radio, using each radio's slot name and frontend, for each mboard.
  - `_generate_mboard_rx_chans()` matches each spec pair (slot letter plus frontend name) to a radio via `get_chan_from_dboard_fe(sd_name, RX)`.
  - The global channel index counts up through mboards, then through the subdev spec entries, in the order of the spec.
  - `set_rx_subdev_spec` rebuilds the map: it disconnects the old chains and re-registers the new channels from 0.
  - I did not read the part of the file with `_set_subdev_spec` in full.
- **Example for UBX in A and BasicRX (two real) in B, INFERRED from the above.**
  - `rx_subdev_spec = "A:0 B:0 B:1"` gives ch0 = UBX, ch1 = BasicRX "0", ch2 = BasicRX "1".
  - `tx_subdev_spec = "A:0 B:0"` for UBX plus an LFTX/BasicTX on B.
  - I could not run it, so this is not a tested example. It follows from the code above.
  - A single-RX B channel can use "A:0 B:1".

## 5. Stream commands and command time

- **Stream commands, VERIFIED.**
  - `rfnoc_rx_streamer.cpp`, `issue_stream_cmd`: if `num_channels > 1` and `stream_now`, it throws `uhd::runtime_error`.
  - Otherwise it posts one action carrying the unchanged `stream_cmd` (including `time_spec`) to every input edge in a loop over channels, so every channel receives the same stream command.
  - multi_usrp (`MUX_RX_API_CALL(issue_stream_cmd)`) is per channel: it goes to the DDC if one exists, otherwise the radio.
  - The ddc handler (`issue_stream_cmd_action_handler`) is per port and scales `num_samps` by `_decim.at(chan)`.
  - Radio: `radio_control_impl.cpp` stores `_last_stream_cmd[chan]` and writes `REG_RX_CMD_TIME_HI/LO` per channel.
- **Does a timed start align all channels on one tick, even across slots? INFERRED yes, if the time is valid.**
  - The same `time_spec` goes to every channel.
  - Both radios use the single timekeeper (see 6). The yml gives both radios the same `time` port.
  - The code rejects `stream_now` for multi-channel streamers because the channels would not be time-aligned, which supports this.
  - I did not read or run the FPGA command-timing path.
  - multi_usrp `set_command_time`: it is per mboard. `MUX_MB_API_CALL` fans out to all mboards, and for a given mboard it calls `set_command_time(time, chain.block_chan)` on every radio, and on the DDC or DUC, of every RX and TX channel chain of that mboard. So one call sets the command time on both slots.

## 6. Device time

- **One timekeeper, VERIFIED (FPGA).** `bus_int.v`: `localparam NUM_TIMEKEEPERS = 1;`, exposed as `RB_NUM_TIMEKEEPERS`.
  - `x300_core.v` feeds a single `time_sync` wire into every `fe_control` instance.
  - The yml connects `_device_ time` to both `radio0` and `radio1`.
- **Host, VERIFIED.**
  - `x300_mb_controller.cpp` reads the count from `TK_NUM_TIMEKEEPERS` and registers `x300_timekeeper` objects (one for this FPGA).
  - `multi_usrp_rfnoc.cpp`: `set_time_now` and `set_time_next_pps` are per mboard (`MUX_MB_API_CALL`) and loop over every timekeeper of that mboard.
  - `x300_mb_controller::synchronize()` uses `get_timekeeper(0)`.
  - `radio_control_impl.cpp`, `get_ticks_now`, uses `get_timekeeper(0)` for older images and a radio register read on newer ones (the code notes "not guaranteed that timekeeper 0 is the one assigned to this radio").
  - So both radios of one X300 are on the same timekeeper.

## 7. Daughterboard combination restriction

- **The doc quote, VERIFIED (dox ~lines 530-536).** The built manual (https://uhd.readthedocs.io/en/uhd-4.9/page_usrp_x3x0.html) says: "There are daughterboards that require non-default clock values.", "Not all combinations of daughterboards work within the same device, if daughterboard clock requirements conflict.", and (dox ~536) "will try and set the daughterboard clock rate themselves." It points to the Ettus KB article for compatibility. That article returned HTTP 403, so I could not read it.
  - The doc does not name the boards. Its only board-specific rule is "(when using TwinRX, only 200 MHz is available)".
- **Mechanism in code, VERIFIED.** `host/lib/usrp/x300/x300_clock_ctrl.cpp`:
  - `DB0_RX` and `DB1_RX` share `CLKout2_3_DIV`, and `DB0_TX` and `DB1_TX` share `CLKout4_5_DIV` ("changing one daughterboard's rate changes the other's").
  - So the dboard clock is per direction and shared across both slots.
  - `lock_dboard_rate()` refuses later changes in a locked direction.
  - `set_dboard_rate` refuses when the user set `dboard_clock_rate`, and throws "invalid dboard rate requested" if the rate does not divide the VCO.
- **Which boards act, VERIFIED for UBX and OBX.**
  - `db_ubx.cpp`, `rev >= 1` only: if the dboard clock exceeds `pfd_freq_max` (25 MHz default, 50 MHz after a branch I did not trace), it picks the highest listed rate at or below it that divides the codec rate, then calls `set_clock_rate` and `lock_clock_rate`.
  - `db_obx.cpp` does the same with a 50 MHz PFD limit, in `_initialize_ref_clocks()`.
  - On failure, both log "Unable to set dboard clock rate - phase will vary".
  - The CBX, SBX and Basic/LF files I read do not set the clock: `db_cbx.cpp` was only seen reading `get_clock_rate()` in `set_lo_freq`. I did not trace the Basic/LF constructor in full.
- **Combinations, INFERRED.**
  - UBX or OBX in one slot plus a Basic or LF board in the other: the Basic/LF boards do not request a clock, so I expect no conflict.
  - UBX with OBX, or two of the same: the same target rate, so I expect no conflict.
  - UBX or OBX with a board that needs a different clock (the doc names TwinRX as needing 200 MHz): affected after the first lock.
  - Whether a given pair fails depends on the board's clock request and the order of initialisation. I verified no complete table.

## 8. Sustained rates

- **10 GbE, VERIFIED (dox ~lines 169-184).** The XG dual-10 GbE section gives only an example: `./benchmark_rate --args="type=x300,addr=<Primary IP>,second_addr=<secondary IP>"` "with the maximum rate of 200 Msps per channel". It gives no single-link figure, and does not name sc16.
  - The doc says "Up to 160 MHz of RF bandwidth with 16-bit samples" (dox ~line 32).
- **1 GbE, VERIFIED not documented in the UHD manual.** I found no sustained 1 GbE rate in `usrp_x3x0.dox`.
  - Ettus's product page lists "Dual 1 Gigabit Ethernet – 25 MS/s Full Duplex" and "200 MS/s per channel" for dual 10 GbE (https://www.ettus.com/?p=1902, seen only as a search result snippet).
  - The Ettus KB page "Using Dual 10 Gigabit Ethernet on the USRP X300/X310" returned 403, so I did not read its caveat directly. A search result summary says full two-channel receive needs a fast multicore host.
  - `x300_eth_mgr.cpp` sizes the send buffer as `link_rate / 50` (about 20 ms of data) and uses `GE_DATA_FRAME_*` for 1 GbE and `XGE_DATA_FRAME_*` otherwise. That is a buffer rule, not a rate limit.

## Not found

- `fpga/usrp3/lib/rfnoc/core/ctrlport_timer.v` and `noc_shell.v` returned 404 at this tag. The tree listing of that directory (28 files shown) has no `ctrlport_timer.v` or `noc_shell.v`, so the timed-queue implementation is not in those files. I found it only as the block's `fifo_depth` in `radio.yml`.
- `host/docs/timed_commands.dox` and `host/docs/rfnoc_timed_cmds.dox` returned 404 at this tag. I used the built docs instead.
- I did not find where the `x300_clock_ctrl` `dboard_clock_rate` arg is parsed, nor `host/lib/rfnoc/ddc_block_control.cpp` radio-side clock effects.
- I did not open `x300_radio_control.cpp` in full. `TwinRX` and other boards were not examined.

Sources:
- https://github.com/EttusResearch/uhd/tree/v4.10.0.0 (files named above)
- https://uhd.readthedocs.io/en/uhd-4.9/page_timedcmds.html
- https://uhd.readthedocs.io/en/uhd-4.9/page_usrp_x3x0.html
- https://uhd.readthedocs.io/en/latest/page_rfnoc_fpga.html
- https://www.ettus.com/?p=1902
