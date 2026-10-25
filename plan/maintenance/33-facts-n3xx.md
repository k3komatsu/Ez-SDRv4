# Facts: USRP N310, N320/N321 and X4xx delta, against checklist G1–G22 (G5 skipped)

- **Date:** 2026-10-25.
- **UHD tag read:** `v4.10.0.0`. Files are at `https://raw.githubusercontent.com/EttusResearch/uhd/v4.10.0.0/<path>`, so "path" below means that prefix plus the path.
- **No line numbers:** WebFetch returns a summary written by a small model, not raw text. Evidence is therefore given as file, function and short quoted fragments. Some quotes are fragments the fetcher returned.
- **Secondary sources:** Ettus KB pages returned 403. A few figures come from search-result snippets and are marked INFERRED.
- **Abbreviations:**
  - V = VERIFIED, I = INFERRED.
  - X300-DSP = `plan/phase8/research/x300-dsp-timed-commands.md`.
  - FAMILY = `plan/phase8/research/usrp-family-limits.md`.
  - MG = Magnesium, RH = Rhodium.
- **Shared code:** The RFNoC `radio_control_impl`, DDC/DUC blocks and `radio.yml` are common to X300, N3xx and X4xx. Where X300-DSP findings carry over, the item says so.

## Cross-cutting facts

- **Product mapping.**
  - **Answer:** The N310 is a motherboard with two Magnesium boards (AD9371). The N300 is the same with one board. The N320 is the same motherboard with two Rhodium boards. All run `type=n3xx` under MPM/mpmd.
  - **Status:** V.
  - **Evidence:** `mpm/python/usrp_mpm/periph_manager/n3xx.py` `product_map`: `('n310', (MG_PID, MG_PID)): 'n310'` and `('n310', (RHODIUM_PID, RHODIUM_PID)): 'n320'`.
  - **N321:** The MPM files read have no `n321` key. The N320/N321 difference visible in UHD is the LO distribution board (G15). I could not verify how UHD distinguishes N321 from N320.
- **FPGA image names.**
  - **Answer:** N310 images are HG (default), XG, HA, XA, AA and WX. N320 images are HG, XG, WX, XQ and AQ. `n3xx.py` `_update_fpga_type` remaps `XG` to `AQ` and `WX` to `XQ`. I did not see the condition for the remap.
  - **Status:** V.
  - **Evidence:** `host/docs/devices/usrp_n3xx.dox` "FPGA Image Flavors" tables; `n3xx.py` `_update_fpga_type`.
- **Image structure (N310).**
  - **Answer:** The N310 image has two `radio.yml` blocks with `NUM_PORTS: 2` each, two DDC and two DUC blocks with `NUM_PORTS: 2` each, and one `replay0` with `NUM_PORTS: 4` (`MEM_DATA_W: 64`, `MEM_ADDR_W: 31`). `chdr_width: 64`.
  - **Status:** V.
  - **Evidence:** `fpga/usrp3/top/n3xx/n310_rfnoc_image_core.yml`.
- **Image structure (N320).**
  - **Answer:** The N320 image has two radios with `NUM_PORTS: 1`, two DDC and two DUC with `NUM_PORTS: 1`, and `replay0` with `NUM_PORTS: 2`.
  - **Status:** V.
  - **Evidence:** `fpga/usrp3/top/n3xx/n320_rfnoc_image_core.yml`.

---

# Part 1: N310 (the N300 is a one-board subset)

**G1. Device time counter.**
- **Answer:** There is one counter per motherboard.
  - **Unit and width:** It counts ticks of the radio tick rate (the master clock rate, MCR) and is 64 bits wide.
  - **Setting it:** It is set through MPM RPC either immediately or at the next PPS.
  - **Shared by RX and TX:** Both radios receive the same `time` signal.
  - **Stream start:** I found no link between stream start and the counter (I).
  - **PPS source:** It is selected one-hot in the register `MB_CLOCK_CTRL`. The options are internal 10 MHz, internal 25 MHz, external, gpsdo, sfp0 and sfp1.
  - **MCR change:** `set_rate` calls `update_tick_rate`. It does not reset the counter.
- **Status:** V, except the stream-start point, which is I.
- **Evidence:**
  - `fpga/usrp3/top/n3xx/n3xx_core.v`: `localparam NUM_TIMEKEEPERS = 1;`, `wire [63:0] radio_time;`, `timekeeper_i`, `.tb_timestamp (radio_time)`.
  - `n310_rfnoc_image_core.yml`: `{ srcblk: _device_, srcport: time, dstblk: radio0, dstport: time }`, and the same for `radio1`.
  - `host/lib/usrp/mpmd/mpmd_mb_controller.cpp`: `mpmd_timekeeper::set_ticks_now` calls `_rpc->set_timekeeper_time(_tk_idx, ticks, false)`, and `set_ticks_next_pps` passes `true`.
  - `n3xx_periphs.py`: `MB_CLOCK_CTRL = 0x0018`, `MB_CLOCK_CTRL_PPS_SEL_*`.
  - `magnesium_radio_control_init.cpp` `_init_mpm`: `_n3xx_timekeeper->update_tick_rate(_master_clock_rate)`.
  - `magnesium_radio_control.cpp` `set_rate`: "There is no explicit timekeeper reset" (fetcher's description).
- **Time read:** The motherboard-level read is an RPC (`get_ticks_now` returns `_rpc->get_timekeeper_time(_tk_idx, false)`). The radio-level read is `regs().peek64(regmap::REG_TIME_LO, get_command_time(0))` in `host/lib/rfnoc/radio_control_impl.cpp` `get_ticks_now`.

**G2. RX timestamp per buffer.**
- **Answer:** A timed RX start is supported. `issue_stream_cmd` with `TIMED` sets `RX_CMD_TIMED_POS` and `ticks = time_spec.to_ticks(get_tick_rate())`. `stream_now` sets no time bits. RX data packets carry a CHDR timestamp (I). X300-DSP §4 says the DDC derives time from packet headers, and the N3xx uses the same DDC.
- **Status:** V for the timed start. I for the per-packet timestamp on N3xx.
- **Evidence:** `host/lib/rfnoc/radio_control_impl.cpp` `issue_stream_cmd`.

**G3. TX bursts.**
- **Answer:** The host-side async report events for TX are `ERR_TX_LATE_DATA`, which maps to `EVENT_CODE_TIME_ERROR`, and `EVENT_TX_BURST_ACK`, which maps to `EVENT_CODE_BURST_ACK`. Burst start and end and the start time travel in the CHDR header (I, shared RFNoC path).
- **Status:** V for the events. I for the header contents.
- **Evidence:** `radio_control_impl.cpp` `async_message_handler`.

**G4. Timed frequency and gain.**
- **Answer:** On Magnesium, tune and gain are not hardware-timed in the sense the checklist means.
  - **AD9371 path (RPC, untimed):** The call is `request<double>(MAGNESIUM_TUNE_TIMEOUT, "set_freq", which, freq, false)`. `MAGNESIUM_TUNE_TIMEOUT` is 30000 ms, which is a timeout and not a schedule. Gain is `request<double>("set_gain", which, gain)`. `set_bandwidth` also uses RPC and "may re-initialize the dboard".
  - **Timed parts:** The SPI core is built with `regs().poke32(addr, data, get_command_time(0))`. The LO2 ADF4351 and the CPLD go through it. The DSA writes use `_gpio[chan]->set_gpio_out`, and I did not confirm whether those are timed.
  - **No `set_command_time`:** `magnesium_radio_control.cpp` does not call it.
  - **Queue:** `radio.yml` has `fifo_depth: 512` (shared). For the DDC, the X300-DSP §4 numbers apply (command FIFO 32, ack stall, 10 s timeout).
  - **Cost of an untimed tune (INFERRED):** Samples during the retune are undocumented. An arXiv experiment (2606.14465, via search snippet) reports about 88 ms for a frequency or bandwidth reconfiguration.
  - **Secondary source:** MathWorks Wireless Testbench lists the N300/N310 as not honouring `setCommandTime`.
- **Status:** V for the code path. I for the 88 ms figure and the MathWorks statement (secondary).
- **Evidence:**
  - `host/lib/usrp/dboard/magnesium/magnesium_ad9371_iface.cpp`.
  - `magnesium_radio_control_init.cpp` `_init_peripherals`.
  - `magnesium_radio_control_gain.cpp` `_set_all_gain`.
  - `host/include/uhd/rfnoc/blocks/radio.yml`.
  - https://www.mathworks.com/help/wireless-testbench/ref/wt.radio.e320.setcommandtime.html

**G6. RX overflow reporting.**
- **Answer:** The shared radio handler maps `ERR_RX_OVERRUN` to `ERROR_CODE_OVERFLOW` and `ERR_RX_LATE_CMD` to `ERROR_CODE_LATE_COMMAND`. The restart action uses `OVERRUN_RESTART_DELAY = 0.05` s (start continuous at `get_time_now() + delay`). I did not find a count of lost samples in the handler.
- **Status:** V.
- **Evidence:** `radio_control_impl.cpp`: the `async_message_handler` `SWREG_RX_ERR` case, and `static constexpr double OVERRUN_RESTART_DELAY = 0.05;`.
- **Caveat:** The fetcher notes that `cont_mode` is set but not read in this file, so the restart is driven from outside that file.

**G7. TX underflow and late data.**
- **Answer:** Both are reported. `ERR_TX_UNDERRUN` maps to `EVENT_CODE_UNDERFLOW` and `ERR_TX_LATE_DATA` maps to `EVENT_CODE_TIME_ERROR`. Both carry a timestamp and are posted to the input edge.
- **Status:** V.
- **Evidence:** `radio_control_impl.cpp` `async_message_handler`.

**G8. Reference and lock.**
- **Answer:** External reference inputs are 10 MHz, 20 MHz and 25 MHz. `ref_clk_freq` defaults to 10 MHz. 20 MHz needs the external time source.
  - **`ref_locked`:** It is a motherboard sensor that is true when every daughterboard's `get_ref_lock()` is true. A board without that method counts as locked. For Magnesium, `get_ref_lock()` returns `self.lmk.check_plls_locked()`.
  - **Polling only:** I found no async loss notification. The sensor is read on demand (`mpmd_mb_controller::get_sensor` calls `_rpc->get_mb_sensor(name)`). A thread updates LEDs every 1.0 s.
  - **Daughterboard lock sensor:** `lo_locked` queries `get_ad9371_lo_lock` over RPC. The RX variant `get_ad9371_rx_lo_locked_sensor` calls `get_ad9371_lo_lock("tx")`, which looks like a bug.
- **Status:** V.
- **Evidence:** `n3xx.py` `set_ref_clock_freq`, `get_ref_lock_sensor`, `_monitor_status`; `mpm/python/usrp_mpm/dboard_manager/magnesium.py`.

**G9. Multi-device aligned start.**
- **Answer:**
  - **Valid sync pairs (clock/time):** internal/internal, internal/sfp0 (White Rabbit), external/external, external/internal and gpsdo/gpsdo. External/gpsdo is commented out in `set_sync_source`. The manual says it can be enabled by patching and "will cause unreliable phase alignment".
  - **Multiple devices:** The manual says "an external reference, such as the CDA-2990 (OctoClock), is required". `external/internal` shares frequency only. `external/external` assumes reference and PPS are synchronized.
  - **White Rabbit:** It needs the WX/XQ image and uses SFP0, which is then unavailable for data. `set_sync_source` polls WR lock for 40 s.
  - **Aligned start:** `mpmd_mb_controller::synchronize` runs pre, timekeeper and post stages. The pre stage runs each device in parallel with `std::async`. The post stage is a no-op ("NOP for now").
  - **Precision:** I found no accuracy figure for inter-device alignment. Within one N310, `_sync_db_clock` aligns the two daughterboard clocks using a 69 ps trace offset for slot B and fails above 100 ps residual.
- **Status:** V.
- **Evidence:** `n3xx.py` `set_sync_source`; `mpmd_mb_controller.cpp`; `usrp_n3xx.dox` "Clock/Time Synchronization", "White Rabbit"; `mpm/python/usrp_mpm/dboard_manager/mg_init.py` `_sync_db_clock`.

**G10. Full duplex.**
- **Answer:** RX and TX are separate AD9371 paths and both can run (ATR ON and IDLE states for RX and TX in `_update_atr_switches`).
  - **TX antenna:** There is only `"TX/RX"`.
  - **RX antennas:** `{"TX/RX","RX2","CAL","LOCAL"}`. The defaults are RX `RX2` and TX `TX/RX`.
  - **Shared connector:** RX on `TX/RX` shares the connector with TX through a TRX switch. The source comments say: "The TRX switch in TX-idle mode defaults to TX-on mode…" and "CAL is only useful when we're both TXing and…".
  - **No duplex API:** `is_true_duplex` and `set_duplex` are absent.
  - **Rule for RX on TX/RX while TX is active:** The source does not state one (I).
- **Status:** V for the lists and the absence of a duplex API. I for the connector behaviour.
- **Evidence:** `magnesium_constants.hpp` `MAGNESIUM_RX_ANTENNAS`, `MAGNESIUM_DEFAULT_RX_ANTENNA`, `MAGNESIUM_DEFAULT_TX_ANTENNA`; `magnesium_radio_control_cpld.cpp` `_update_atr_switches`; `magnesium_radio_control.cpp` `get_tx_antennas` returns `{"TX/RX"}`.

**G11. Channels and what is shared.**
- **Answer:** The N310 has 4 RX and 4 TX channels. Subdev specs are `A:0 A:1 B:0 B:1` (RF0 to RF3). The N300 has 2.
  - **LO sharing:** The two channels of one daughterboard share the RX LO and share the TX LO. `_update_freq` loops over `MAGNESIUM_NUM_CHANS` (= 2) and applies the same RF frequency to every channel, with the comment "We need to update the frequencies on both channels, because they share one LO." So setting one channel's frequency changes its partner's.
  - **RX versus TX LO:** They are independent per direction.
  - **Across boards:** The two daughterboards are independent.
  - **Per channel:** Gain, antenna and bandwidth (`set_bandwidth(..., chan, dir)`) are per channel.
  - **Rate:** The MCR is common. `set_rate` sets A then B and throws "Different rates on radios 0 and 1!" on mismatch. Decimation and interpolation are per DDC/DUC port.
- **Status:** V.
- **Evidence:** `magnesium_radio_control.cpp` `_update_freq`, `set_rate`; `magnesium_constants.hpp` `MAGNESIUM_NUM_CHANS = 2`; `n310_rfnoc_image_core.yml`; `usrp_n3xx.dox` "Subdev Specifications".

**G12. Rate grid.**
- **Answer:** The valid MCRs are 122.88, 125 and 153.6 MHz. `Magnesium.init()` asserts that `master_clock_rate` is one of them. The MPM class default is `default_master_clock_rate = 125e6`. The manual says the boot default is set in `/etc/uhd/mpm.conf` (example 122.88e6).
  - **Link to the lane rate:** The JESD lane-rate relation `new_rate = master_clock_rate * M * N * (10.0 / 8) / L / S` with M=4, N=16, L=4, S=1 gives lane rate = 20 × MCR.
  - **Changing the MCR:** An MCR change at runtime is allowed. It runs `_reinit(rate)`, a full `init()`, with the log message "This may take some time."
  - **Rate grid:** The radio runs at the MCR. The DDC/DUC use the defaults in `ddc.yml` and `duc.yml` (`NUM_HB: 3`, `CIC_MAX_DECIM: 255`, `CIC_MAX_INTERP: 255`). The N310 image sets only `NUM_PORTS`, so the grid is the one found in X300-DSP/FAMILY (rate = MCR/N, N from `ddc_block_control.cpp` `coerce_decim`). I did not re-read `coerce_decim` for N3xx.
  - **Read-back:** The sample-rate resolver does `coerce_rate`. Coercion logs a warning ("Coercing requested sample rate from…").
  - **Rate-dependent behaviour:** The AD9371 RX and TX FIRs run at 2× the MCR. The manual gives max taps of 48 for RX and 32 for TX. `set_rx_bandwidth` and `set_tx_bandwidth` have a range of 20–100 MHz.
- **Status:** V for the MCR list, default and re-init. V for the DDC defaults. I that N3xx uses the same `coerce_decim` as X300.
- **Evidence:** `mpm/.../magnesium.py` `init`, `set_master_clock_rate`, `_reinit`; `mg_init.py` lane-rate formula; `magnesium_constants.hpp` `MAGNESIUM_RADIO_RATES = {122.88e6, 125e6, 153.6e6}`; `host/include/uhd/rfnoc/blocks/ddc.yml` and `duc.yml`; `usrp_n3xx.dox`.

**G13. Frequency and gain ranges.**
- **Answer:** The UHD range is 1 MHz to 6 GHz with a 1.0 Hz step, clipped (`MAGNESIUM_FREQ_RANGE.clip`). The manual says 10 MHz–6 GHz is the specified range.
  - **Direct and low band:** The AD9371 covers 300 MHz–6 GHz. Below that, the low-band LO (ADF4351, 35 MHz–4.4 GHz) mixes with fixed IFs (RX 2.4418 GHz, TX 1.95 GHz).
  - **LO resolution:** I did not find the AD9371 or ADF4351 frequency resolution.
  - **External LO:** It must be 2× the centre frequency (`// external LO required to tune at 2xdesired_frequency`). The Ettus product-page footnote (search snippet, I) limits the centre to 300 MHz–4 GHz.
  - **Aggregate gain:** RX 0–75 dB and TX 0–65 dB, both in 0.5 dB steps. In the `manual` profile the aggregate RX range is `(0,0,0)`.
  - **Stages:** AD9371 RX 0–30 (0.5). DSA 0–31.5 (0.5). AMP 0–10 (step 10). AD9371 TX 0–41.95 (0.05).
  - **Profiles:** `{"manual","default","default_rf_filter_bypass_always_on","default_rf_filter_bypass_always_off"}`.
  - **Band maps:** Band edges are customizable (TX has 4 bands and RX 7, via `tx_band_map` and `rx_band_map`).
- **Status:** V for the constants. I for the product-page footnote.
- **Evidence:** `magnesium_constants.hpp`; `magnesium_radio_control.cpp` `get_rx_gain_range`; `usrp_n3xx.dox` "Gain Ranges".

**G14. Wire format and transport.**
- **Answer:** `radio.yml` has `ITEM_W: 32`, which is sc16. I found nothing offering another over-the-wire format on RFNoC (I, same as FAMILY §1a).
  - **Ports by image:** SFP0 and SFP1 speeds depend on the image (HG: 1 GbE and 10 GbE; XG: both 10 GbE; Aurora variants). The default MTU is 9000.
  - **Throughput figures:** The manual gives none. The arithmetic is I: sc16 is 4 B per sample, so 125 Msps is 500 MB/s per channel against about 1.25 GB/s raw for 10 GbE.
  - **Link choice:** `mpmd_mb_iface.cpp` has no `link` argument. Links come from `addr` and `second_addr`. I did not read the stream-to-link assignment (`link_stream_manager`).
- **Status:** V for `ITEM_W`, images and MTU. I for the rest.
- **Evidence:** `radio.yml`; `usrp_n3xx.dox` "Network Connectivity", "FPGA Image Flavors"; `mpmd_mb_iface.cpp`.

**G15. Phase after retune and LO sharing.**
- **Answer:** Phase is not deterministic.
  - **Manual:** "without an external LO, the N310 'phase will be random between runs'". An external LO "reduces phase ambiguity to 180 degrees" because of the AD9371's by-2 divider. "The N310 itself has no way of aligning phase between channels."
  - **Code:** I found no phase or sync step in the tune path (`magnesium_radio_control.cpp` `set_rx_frequency` and `_update_freq`). `mg_init.py` has multi-chip sync (two SYSREF pulses) at init only.
  - **Ettus footnote:** "Phase coherency is not repeatable after retune or reinitialization of the RF front-end" (https://www.ettus.com/all-products/usrp-n310/, footnote 3).
  - **LO sharing:** Same-board channels share the RX LO and share the TX LO (G11). External LO inputs exist per daughterboard for TX and RX. `lo_export` does not appear and LO2 accepts only `"internal"`.
  - **Timed tune:** Not applicable, since tune is not timed.
- **Status:** V.
- **Evidence:** `usrp_n3xx.dox` "External LOs (N310)"; `mg_init.py` `init_jesd`.

**G16. Path delay.**
- **Answer:** No delay or latency value is in UHD for N310. `magnesium_constants.hpp` has no `delay` or `latency` constants. The AD9371 calibration list includes `PATH_DELAY`, `RX_LO_DELAY` and `TX_ATTENUATION_DELAY`. These are internal calibrations with no exposed number.
- **Status:** V for absence in these files.
- **Evidence:** `usrp_n3xx.dox` "RF Calibrations" table.

**G17. Repeat TX.**
- **Answer:** The FPGA image has a Replay block (4 ports, DRAM). There is no `multi_usrp` API for it. It is reachable only through RFNoC.
- **Status:** V.
- **Evidence:** `n310_rfnoc_image_core.yml` `replay0`.

**G18. Safety envelope.**
- **Answer:**
  - **TX power (secondary, I):** A search snippet of the Ettus KB table lists about +16 dBm for 10–500 MHz, +18 dBm for 500 MHz–4 GHz and +12 dBm for 4–6 GHz. The datasheet says maximum output needs all TX amplifiers enabled.
  - **Items beyond today's frame:**
    - External LO input ports for RX and TX, one per daughterboard.
    - Per-direction LNA and amplifier bypass booleans passed into the switch update (`_rx_bypass_lnas`, `_tx_bypass_amp`).
    - Gain profiles that change which stages are active.
  - **Bias tee:** None found in the N3xx dox.
- **Status:** I for the power figures. V for the code names. The bias-tee absence is limited to what the fetcher returned.
- **Evidence:** `magnesium_radio_control.cpp` `set_rx_frequency` (`_update_rx_freq_switches(freq, _rx_bypass_lnas, ...)`), `set_tx_frequency`.

**G19. Selector facts.**
- **Answer:** Device-args facts:
  - `addr`, `second_addr`, `mgmt_addr`, `discovery_port` (49700), `rpc_port` (49701), `force_mtu`.
  - `master_clock_rate`, `clock_source`, `time_source`, `ref_clk_freq`, `serialize_init`, `skip_boot_init`, `force_reinit`, `enable_gps`.
  - N310 only: `rx_lo_source`, `tx_lo_source`, `init_cals`, `tracking_cals`, `rx_gain_profile`, `tx_gain_profile`, `rx_band_map`, `tx_band_map`.
  - Discovery: `mpmd_find.cpp` sets `type=mpmd` and copies the other key/value pairs from the device reply.
  - Reported motherboard identity: `product` (from `product_map`) and `fpga` (image type).
  - FPGA compat: `N3XX_FPGA_COMPAT = (8, 1)`.
  - The radio block checks the device MCR against the `master_clock_rate` block arg and errors with "Master clock rate mismatch" if they differ.
- **Status:** V.
- **Evidence:** `usrp_n3xx.dox` "Device arguments"; `n3xx.py`; `mpmd_find.cpp`; `magnesium_radio_control_init.cpp` `_init_mpm`.

**G20. Timing numbers.**

| Item | Value | Where |
|---|---|---|
| Command lead | No minimum found. | – |
| Tune timeout (MG) | 30000 ms (`MAGNESIUM_TUNE_TIMEOUT`) | `magnesium_constants.hpp` |
| MCR change | Full re-init, duration not stated ("a fair amount of time"); RPC timeouts `MPMD_DEFAULT_INIT_TIMEOUT` 120000 ms, `MPMD_DEFAULT_REBOOT_TIMEOUT` 200000 ms | `mg_init.py`, `mpmd_impl.hpp` |
| Radio ctrl FIFO (timed) | 512 words | `radio.yml` |
| DDC command FIFO | 32 (X300-DSP §4, shared block) | – |
| Overrun restart gap | 0.05 s | `radio_control_impl.cpp` |
| Device-arg RPC timeouts | `MPMD_DEFAULT_RPC_TIMEOUT` 2000 ms; `MPMD_CLAIMER_RPC_TIMEOUT` 10000 ms | `mpmd_impl.hpp` |
| Per-direction start lead | Not found. | – |

- **Status:** V for the values listed. "Not found" for the rest.

**G21. Device loss.**
- **Answer:** The host claims the device and re-claims every `MPMD_RECLAIM_INTERVAL_MS = 1000`. MPM's claim expires after `TIMEOUT_INTERVAL = 5.0` s by default (overridable by `rpc_timeout_interval`). On expiry MPM logs "A timeout event occured!" and runs `_unclaim()`, which deinitializes the peripheral manager and clears the token.
  - **Host side:** If `claim()` fails, the loop throws `uhd::value_error("mpmd device reclaiming loop failed!")`. This is in a background task.
  - **No other signal:** There is no device-loss event and no heartbeat in the N3xx files read. How this reaches a streamer call is I.
- **Status:** V for claim, reclaim and expiry. I for how it reaches the caller.
- **Evidence:** `mpmd_mboard_impl.cpp` `make_claim_loop_task`, `claim`; `mpmd_impl.hpp` constants; `mpm/python/usrp_mpm/rpc_server.py` `_timeout_event`, `_unclaim`.

**G22. Parts on separate clocks.**
- **Answer:** There is one timekeeper (G1). The two daughterboards run on separate converter clocks, aligned by `_sync_db_clock` (G9). Time ticks come from the radio clock for both. The number of timekeepers is read from MPM (`_rpc->get_num_timekeepers()`), and for N3xx it is 1.
- **Status:** V.
- **Evidence:** `n3xx_core.v` `NUM_TIMEKEEPERS = 1`; `mpmd_mb_controller.cpp` constructor loop `register_timekeeper(tk_idx, ...)`.

---

# Part 2: N320 / N321 (Rhodium)

**G1, G2, G3, G6, G7, G8, G21, G22: same as N310.**
- **Shared code path:** The same `n3xx_core.v` (`ifdef N320`), `radio_control_impl`, `mpmd_*` files and `n3xx.py` apply.
- **Timekeeper:** One timekeeper, MCR ticks.
- **Image (N320):** Radios with `NUM_PORTS: 1`, so 2 channels in all.
- **Reference lock:** `Rhodium.get_ref_lock()` returns the LMK PLL status (False if the LMK is not initialized). The docstring says "This does not return a sensor dict."
- **`lo_locked`:** It is a per-direction sensor (`get_lo_lock_status`).
- **Two daughterboard clocks:** `_sync_db_clock` applies the same 69 ps and 100 ps rule (`rh_init.py`).
- **Status:** V.

**G4. Timed frequency and gain.**
- **Answer:** Rhodium has a hardware-timed path.
  - **SPI core:** It uses `regs().poke32(addr, data, get_command_time(0))`. The LMX2592 LO, the CPLD switches and the DSA gain index go through it. Gain is `_cpld->set_gain_index(index, new_band, RX_DIRECTION)` (TX is the same with `TX_DIRECTION`).
  - **Timed-command query:** `_get_timed_command_enabled()` returns `get_command_time(0) != time_spec_t::ASAP`.
  - **Known gap:** With `highband_spur_reduction` enabled, a tune across 450 MHz also sends an untimed RPC (`enable_*_lowband_lo`). The code warns: "Timed tuning commands that transition between lowband and highband, 450 MHz, do not function correctly when highband_spur_reduction is enabled!"
  - **No lock wait:** `lmx2592.cpp` `set_frequency` toggles FCAL and returns. It never calls `get_lock_status()` and has no sleep.
  - **DDC/DUC and queue:** As for X300, a timed tune also writes the DDC/DUC frequency (X300-DSP §2). Queue limits are the same shared blocks (`radio.yml` 512, DDC FIFO 32).
  - **Secondary source:** MathWorks lists N320 and N321 as honouring `setCommandTime`.
- **Status:** V for the code path. I for MathWorks (secondary).
- **Evidence:** `host/lib/usrp/dboard/rhodium/rhodium_radio_control_init.cpp` `_init_peripherals`; `rhodium_radio_control.cpp` `set_rx_frequency`, `set_tx_frequency`, `_get_timed_command_enabled`; `host/lib/usrp/common/lmx2592.cpp`; `rhodium_radio_control_lo.cpp`.

**G9. Multi-device aligned start.**
- **Answer:** Same sync pairs as N310. The N320/N321 images also include WX and XQ for White Rabbit. Rhodium `rh_init.py` overrides `ref_clk_freq = 62.5e6` when the time source is `sfp0`, and applies a 1-tick external PPS delay. It has PPS delay constants (`EXT_PPS_DELAY` 5, `N3XX_INT_PPS_DELAY` 4).
- **Status:** V.
- **Evidence:** `rh_init.py` `_sync_db_clock`.

**G10. Full duplex.**
- **Answer:** Antennas are TX `{"TX/RX","CAL","TERM"}` and RX `{"TX/RX","RX2","CAL","TERM"}`. The cpld file has no duplex check. The only cross-dependency is that the TX output switch depends on whether the TX frequency is low band (`_update_tx_output_switches`). A front-panel GPIO comment reads: "Pin 1 shall go high when channel 0 is receiving, or during full-duplex."
- **Status:** V.
- **Evidence:** `rhodium_constants.hpp`; `rhodium_radio_control_cpld.cpp`.

**G11. Channels and what is shared.**
- **Answer:** There are 2 channels in total, `A:0` and `B:0` (`UHD_ASSERT_THROW(chan == 0)`), one RX and one TX per daughterboard. Within a board, TX LO and RX LO are separate objects (`_tx_lo`, `_rx_lo`) with their own source, export, gain and power. So TX and RX frequency are independent. The low-band LO is one fixed frequency derived from the MCR, reported for both directions.
  - **MCR:** It is per motherboard. `Rhodium::set_rate` calls `"db_0_set_master_clock_rate"` only. How it reaches slot B is I.
- **Status:** V, except the MCR propagation to slot B, which is I.
- **Evidence:** `rhodium_radio_control_lo.cpp`; `rhodium_radio_control.cpp`; `n3xx.py` `get_clocks`.

**G12. Rate grid.**
- **Answer:** The valid MCRs are 200, 245.76 and 250 MHz. The ADC/DAC run at twice the MCR (`self.sampling_clock_rate = 2 * self.master_clock_rate`, JESD rates 400, 491.52 and 500 MHz). `Rhodium.default_master_clock_rate = 245.76e6`. The manual's example default is 200e6 in `mpm.conf`.
  - **Grid:** MCR/N with N from the shared DDC (N320 image sets only `NUM_PORTS: 1`, so defaults `NUM_HB 3`, `CIC_MAX 255`). The manual says rates are "up to 250 Msps" and gives 47-tap halfband filters that are not user-configurable.
  - **Bandwidth:** `get_*_bandwidth_range` is the single value `RHODIUM_DEFAULT_BANDWIDTH = 250e6`, with no setter.
  - **Re-init:** An MCR change re-inits via RPC (`SET_RATE_RPC_TIMEOUT_MS = 10000`) and then re-applies TX and RX frequency.
- **Status:** V for MCRs, default and 2×. I for the grid (as in Part 1).
- **Evidence:** `rhodium_constants.hpp` `RHODIUM_RADIO_RATES`; `mpm/.../rhodium.py` `init`, `default_master_clock_rate`, `set_master_clock_rate`; `rh_init.py`; `n320_rfnoc_image_core.yml`; `usrp_n3xx.dox`.

**G13. Frequency and gain ranges.**
- **Answer:** The UHD range is 1 MHz to 6 GHz, step 1.0 Hz (clipped). Ettus specifies 3 MHz to 6 GHz.
  - **LO1 (LMX2592):** 450 MHz–6 GHz, reference 122.88 MHz. Below the lowband threshold a fixed lowband LO is used (`target_lo = _get_lowband_lo_freq() - freq`).
  - **Aggregate gain:** RX 0–60 and TX 0–60, both in 1 dB steps. These are implemented as a CPLD gain index. LO gain 0–30 and LO power 0–63 are separate constants.
  - **LO resolution:** In `lmx2592.cpp` the minimum VCO step is 1 without spur dodging. With spur dodging it is at least a threshold (default 2 MHz in the code's units). I did not determine the Hz resolution.
- **Status:** V.
- **Evidence:** `rhodium_constants.hpp`; `rhodium_radio_control.cpp`; `lmx2592.cpp`.

**G14. Wire format and transport.**
- **Answer:** sc16 as in Part 1. The N320 has a QSFP+ port used for streaming over DPDK. N320 images: HG, XG, WX, XQ (QSFP+ lanes 0–1 at 10 GbE) and AQ (QSFP+ Aurora). 250 Msps × 4 B = 1 GB/s per channel (I, arithmetic) against about 1.25 GB/s raw for 10 GbE. RJ45 "does not support IQ streaming" (Ettus N321 page).
- **Status:** V for images and ports. I for the arithmetic.
- **Evidence:** `usrp_n3xx.dox` "Network Interfaces" and image table; https://www.ettus.com/all-products/usrp-n321/.

**G15. Phase after retune and LO sharing.**
- **Answer:**
  - **No phase step in UHD:** `lmx2592.cpp` has no `SYNC` or `VCO_PHASE_SYNC` identifiers and no phase language. The Rhodium files contain no phase or sync handling (`rhodium_radio_control_init.cpp`: no LO phase sync at init). UHD documentation is silent on phase after retune for N320/N321.
  - **Secondary:** The Ettus app note AN-725 (search summary; the KB page returned 403) says deterministic phase across retunes needs either a sync pulse or a shared LO. MathWorks says "By default, LOs are independent; they are not phase-coherent or phase-synchronized."
  - **External LO inputs:** One RX and one TX input on N320 (`LO IN RX`, `LO IN TX`; nominal +5 dBm ±2 dB). N321 has RX and TX LO IN1.
  - **N321 export:** A 1:4 splitter feeds OUT0–OUT3 from either the IN0 port or the slot-A LO. Each output has an internal terminator to disable before use.
  - **API:** `enable_lo_output(direction, port, enable)` and `enable_lo_export`, both asserting that an LO distribution board is attached. `_set_lo1_export_enabled` toggles RF output B and sends an RPC only if the distribution board is present. `LO_OUTPUT_PORT_NAMES = {"LO_OUT_0".."LO_OUT_3"}`. LO2 cannot be exported (`"The lowband LO cannot be exported"`).
  - **Scale:** The manual says up to 16 N321 modules share one LO (32×32 channels). Ettus says only the N321 exports.
  - **Timed tune plus shared LO:** Not documented.
- **Status:** V for code facts. I for the Ettus app-note and MathWorks claims (secondary).
- **Evidence:** `lmx2592.cpp`; `rhodium_radio_control_lo.cpp`; `rhodium.py` `enable_lo_output`, `enable_lo_export`, `is_lo_dist_present`; `usrp_n3xx.dox` "LO Sharing"; https://www.mathworks.com/help/wireless-testbench/ug/configure-lo-sharing.html.

**G16. Path delay.** None found in `rhodium_constants.hpp`, `rhodium_radio_control*.cpp` or `rh_init.py`. Status: V for absence in those files.

**G17. Repeat TX.** The N320 image has `replay0` with `NUM_PORTS: 2`, reachable only through RFNoC. V (`n320_rfnoc_image_core.yml`).

**G18. Safety envelope.**
- **Answer:** I did not find a published N320/N321 TX power figure.
  - **Search snippet:** One mailing-list anecdote (August 2025) reports about 1 dBm at gain 60 and attributes it to a hardware fault.
  - **Frame items:** LO export outputs (internal terminators must be disabled) are RF emission points not in a TX-band/gain/antenna envelope. External LO input level is nominal +5 dBm ±2 dB.
  - **Antenna `TERM`:** It is a selectable termination.
- **Status:** I (secondary) for the anecdote. V for the constants.

**G19. Selector facts.** The product is `n320` when the motherboard is an `n310` PID with two Rhodium PIDs. The N321 is not distinguished in the MPM files read. The LO distribution board is detected at init by `FPGAtoLoDist.lo_dist_present`. `master_clock_rate` is not listed in the manual's device-arg table for the N320 but `Rhodium.init()` asserts the same arg (I that it works as a device arg). Status: V for the product map and LO distribution detection.

**G20. Timing numbers.** `SET_RATE_RPC_TIMEOUT_MS = 10000`. The RFSoC-free N3xx numbers otherwise match Part 1 (FIFO 512, DDC FIFO 32, restart 0.05 s). I found no tune time, lock time or command-lead number for Rhodium. Status: V for the listed values.

---

# Part 3: X410 / X420 / X440 delta

## Items FAMILY already answers

| Item | What FAMILY answers | FAMILY section |
|---|---|---|
| G1 | Number of timekeepers (X410 one, X420/X440 two). Unit, width and set behaviour not covered. | §1c, §1d, §1e |
| G4 | Timed-tune class per device (X410/X440 none, X420 above 500 MHz). Queue depth not covered. | §1c–§1e |
| G8 | Clock and time source names. `ref_locked` on X4xx "not read". | §1c, §4 |
| G11 | Channel counts (X410 4+4, X420 2, X440 8 or 2). Rate sharing partly. | §1c–§1e |
| G12 | Radio rate = MCR; DDC/DUC only on `X4_200` images. The X4xx DDC grid is unread. | §1c–§1e, §4 |
| G13 | Frequency ranges and gain ranges per board (ZBX, HBX, FBX). | §1c–§1e |
| G14 | Transport and links. "sc16 only" is I. | §1c–§1e |
| G17 | Replay presence and sizes. | §1c–§1e |
| G19 | Image names, product names, MCR sets. | §1c–§1e |
| G22 | Two timekeepers on X420/X440, dual-rate X440. | §1d, §1e |

FAMILY does not answer G2, G3, G6, G7, G10, G15, G16, G18, G20, G21 or G9. G9 and the others follow.

**G1/G2/G3/G6/G7/G10/G21 (shared code, not re-read for X4xx).**
- The same `radio_control_impl` and `mpmd` code applies. `x400_radio_control.cpp` forwards frequency and gain to the daughterboard and takes `_lock` first.
- `x400_radio_control_impl::set_rate` logs that "X400 does not support runtime rate changes" and returns `get_rate()`. The MCR is read from MPM at construction (`_db_rpcc->get_master_clock_rate()`).
- **Status:** V for these two points. I for the others carrying over.
- **Evidence:** `host/lib/usrp/x400/x400_radio_control.cpp`.

**G8 (addition). `ref_locked`.**
- **Answer:** `ref_locked` is a motherboard sensor that checks "that all the daughterboards have locked to the external reference". Daughterboard sensors are enumerated from MPM at runtime. ZBX has per-channel `lo1_locked`, `lo2_locked` and `nco_locked`. HBX has `lo_locked` (same value for any channel or direction).
- **Status:** V.
- **Evidence:** `usrp_x4xx.dox` "Sensors" (`x4xx_usage_sensors`); `host/docs/devices/zbx.dox` "Sensors"; `host/docs/devices/hbx.dox`.

**G9. Multi-device sync.**
- **Answer:**
  - **Sources:** Time sources `internal, external, qsfp0, gpsdo`. Clock sources `mboard, internal, external, nsync, gpsdo`. `ext_clock_freq` defaults to 10 MHz.
  - **Clock reconfiguration:** On X440 it currently restarts MPM (`skip_mpm_reboot` is X440 only). It may reset non-persistent QSFP28 network configuration.
  - **Within a device:** MTS aligns all RF tiles to tile 0 (ADC only). Channels on one tile are always synchronized. In X440 dual-rate mode MTS is disabled, and the phase between channels of one daughterboard "is not guaranteed over retunes and reboots". Between daughterboards "an undefined phase relationship will be preserved".
  - **SYNC IN connectors:** "in the current version of UHD, they are not supported". TRIG IN/OUT is not supported in default images.
  - **Between devices:** UHD docs have no statement or accuracy figure on cross-device alignment for X4xx. `host/docs/sync.dox` returned 404. The manual page https://uhd.readthedocs.io/en/latest/page_sync.html names only X3x0, N2x0 and USRP2 hardware, so X4xx and N3xx are not in it.
  - **ZBX LO export:** `set_*_lo_export_enabled` throws "not supported on this radio".
  - **HBX:** LO sharing needs external cabling. One channel exports and the others import. It is set separately for TX and RX. The master also imports its own LO "so all channels see comparable external path delay." It is sufficient to tune all channels to the same frequency.
- **Status:** V.
- **Evidence:** `usrp_x4xx.dox` (`x4xx_usage_args`, `x440_usage_mcr_dualrate`, back-panel text); `zbx_dboard.cpp`; `hbx.dox`.

**G15. Phase after retune and LO sharing on ZBX/HBX/FBX.**
- **Answer:** The code has a sync step that runs only when a command time is set.
  - **ZBX sync expert:** `zbx_sync_expert::resolve()` (`host/lib/usrp/dboard/zbx/zbx_expert.cpp`) exits early with the trace "No command time: Skipping phase sync." Otherwise it finds "dirty LOs" and NCOs, issues LO sync pulses (`pulse_lo_sync`), and calls `_rfdcc->reset_ncos(..., _fe_time...)` and `reset_gearboxes`. Gearboxes are synced once per session. Comments say: "make sure the timekeepers are running in sync (by calling set_time_next_pps() accordingly)" and "If sync commands happen at different times, we make sure to send out the earlier one first."
  - **LO sync mode:** `zbx_lo_ctrl.cpp` calls `_lmx->set_sync_mode(true)` with the comment "In ZBX, we always run the LOs in sync mode."
  - **Conflict with the docs:** `zbx.dox` and `usrp_x4xx.dox` say timed tuning is not supported on X410/X440, specifically timed tuning of the RFSoC NCO. `zbx.dox` adds that the LOs could be tuned at a set time. Whether a timed tune on X410 yields aligned phase is not verified.
  - **NCO write:** The NCO frequency call `rfdc_set_nco_freq` is untimed (`zbx_rfdc_freq_expert::resolve`). Only `reset_ncos` and `reset_gearboxes` take a time. The reset waits a default 11 SYSREF cycles (`x400_rfdc_control.cpp` `reset_ncos`), and the check that the reset finished is a TODO.
  - **ZBX channels:** LOs of CH0 and CH1 are deliberately offset to avoid injection locking. LO states "can be queried and configured independently".
  - **`set_command_time` on ZBX:** It writes only to the RX front-end time node.
  - **X420 (HBX) and X440 (FBX):** `hbx_expert.cpp` and `fbx_expert.cpp` have the same "No command time: Skipping phase sync" early exit and call `reset_ncos`. FBX tunes only the NCO, which is untimed (`fbx_rfdc_freq_expert::resolve`). `hbx.dox` does not cover phase after retune.
- **Status:** V for the code. The docs conflict as noted.
- **Evidence:** the files named above.

**G16. Path delay.** I found no path-delay or latency API or value for ZBX, HBX, FBX or X400 radio. `zbx.dox` lists "Not Covered in This Page: Phase, Latency/delay, Tune time, IQ/DC calibration, Multi-device sync". `x400_rfdc_control.cpp` has no latency handling. Status: V for absence in those files.

**G18. Safety envelope (X4xx specifics not in FAMILY).** HBX has an `lo_pd` power-detector sensor. The exported LO power is "close to 0 dBm" by default (`hbx.dox`). Antenna `TERMINATION` and `CAL_LOOPBACK` exist on ZBX and HBX. `SYNC_INT` on HBX "Feeds a 5 MHz square wave into the ADC". I did not find a TX power range. Status: V.

**G20. Timing numbers.**
- **Command granularity:** For the X440 at 500 MHz MCR, "internally uses a 62.5 MHz clock" and "The command processor can only compare the command time with the current time every 16 ns" (https://uhd.readthedocs.io/en/latest/page_timedcmds.html).
- **Queue:** `radio.yml` `fifo_depth: 512` (the X410 image `x410_X4_200_rfnoc_image_core.yml` uses `block_desc: 'radio.yml'`, `NUM_PORTS: 2`).
- **ZBX gain tables:** When a command time is set, the CPLD gain-table reload is skipped. Gain readback "is not possible if there are timed commands queued up."
- **ADC self-calibration:** It runs after clocking changes (`x400_radio_control.cpp`, "Clocking reconfigured, running ADC Self Cal"). Duration not stated.
- **LO lock:** `ZBX_LO_LOCK_TIMEOUT_MS` is used but its value is not shown. The only literal is a 1 ms poll.
- **Not found:** Minimum command lead, startup delay, per-direction start lead.
- **Status:** V for the listed items.

**G22. Timekeepers (addition to FAMILY).** The X400 radio picks its timekeeper in the constructor as `_mb_control->get_num_timekeepers() >= 2 ? get_block_id().get_block_count() : 0`. Per `usrp_x4xx.dox`, timekeepers start in lockstep. `set_time_now` sets all of them, so they can desynchronize, and `set_time_next_pps` or `set_time_unknown_pps` is recommended. Status: V.

---

# Searches that found nothing

- Ettus KB pages returned HTTP 403: https://kb.ettus.com/N300/N310, https://kb.ettus.com/N320/N321, https://kb.ettus.com/USRP_N320/N321_LO_Distribution, and the `action=pdfbook&format=single` variants for N300/N310 and N320/N321.
- https://www.ettus.com/wp-content/uploads/2019/01/USRP_N310_Datasheet_v3.pdf returned a logo PDF with no tables.
- N320/N321 TX output power: WebSearch "USRP N320 datasheet maximum output power dBm…" found no figure. The N310 power figures come from a search snippet of the KB, not from a page I read.
- 404s at `v4.10.0.0`: `host/docs/usrp_n3xx.dox`, `mpm/python/usrp_mpm/mboard_regs.py`, `host/docs/timed_commands.dox`, `host/docs/sync.dox`, `host/lib/rfnoc/radio_control_impl.hpp`.
- `https://api.github.com/search/code?...` returned 401. Directory listings (`contents/...?ref=v4.10.0.0`) worked.
- Searches for `timed tune` and phase after retune on N310, N320 and N321 in UHD docs: no UHD statement. Searches for `timed tune` and phase alignment in the UHD manual returned only the generic timed-commands page, which does not mention N3xx.
- In `magnesium_radio_control.cpp`: no `get_command_time`, `set_command_time`, `lo_export`, `is_true_duplex` or `set_duplex`. In `rhodium_radio_control.cpp`: no `set_command_time`, `lo_export` or `get_ref_locked`. In `mpmd_mb_controller.cpp`: no `ref_locked`, `heartbeat` or device-loss terms.
- `mpmd_mb_iface.cpp` and `mpmd_link_if_mgr.cpp`: no `link` argument, no link-rate or stream-count logic.
- Path delay or latency constants: none in the Magnesium, Rhodium, ZBX or `x400_*` files read.
- `usrp_n3xx.dox`: no `bias`, `coheren`, `latency`, `throughput` or `overflow` terms (as returned by the fetcher).
- Not read: the MPM `mykonos` C++ `set_freq` internals and AD9371 LO resolution, `link_stream_manager` stream-to-link assignment, `rhodium.py` MCR propagation to slot B, the X4xx DDC decimation grid, `rpc_server.py` watchdog (the fetcher found none).

Skipped or unchecked: line numbers (the fetch tool drops them), and everything marked I rests on search snippets or on the shared X300 findings. Risk: for N310 and X4xx the docs and code disagree on timed tuning and phase (G4, G15), so treat the code reading as the primary fact.
