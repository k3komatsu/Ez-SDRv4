# bladeRF 2.0 micro (xA4 / xA9) facts for the generality check (item A2)

- **Tag read:** `2025.10`, the latest release on GitHub (`api.github.com/repos/Nuand/bladeRF/releases/latest`, published 2025-10-06T23:25:10Z).
- **Date of this report:** 2026-10-25.
- **How I read the sources:**
  - WebFetch returns a small-model summary, not raw text. I have no line numbers, and quotes are short fragments the fetcher gave back. Where a claim rests only on that summary, I say so.
  - "RAW" below means the prefix `https://raw.githubusercontent.com/Nuand/bladeRF/2025.10/`.
  - Every path starts at the repository root unless it begins with `host/libraries/libbladeRF/`. Write that prefix as `LIB/`.
- **Files read at the tag:**
  - **Public headers:** `LIB/include/libbladeRF.h`, `LIB/include/bladeRF2.h`.
  - **Board code:** `LIB/src/board/bladerf2/{bladerf2.c, common.c, common.h, capabilities.c, rfic_host.c, rfic_fpga.c}`.
  - **Streaming and backend:** `LIB/src/streaming/{sync.c, metadata.h}`, `LIB/src/backend/usb/{nios_access.c, libusb.c}`.
  - **Shared constants:** `fpga_common/include/{bladerf2_common.h, nios_pkt_retune2.h}`. The range tables are in `bladerf2_common.h`, not in `bladerf2.c`.
  - **HDL:** `hdl/fpga/ip/nuand/trigger/trigger.vhd`, `hdl/fpga/ip/nuand/synthesis/{fifo_reader.vhd, fifo_writer.vhd}`, `hdl/fpga/platforms/bladerf-micro/vhdl/{bladerf-hosted.vhd, rx.vhd, tx.vhd}`.
- **Not pinned:** the FPGA and firmware image versions a given unit runs. Capability thresholds are in G20 and G19.
- **AD9361 sources:** UG-570 and the datasheet timed out on every fetch. All AD9361 statements are INFERRED, from the libbladeRF code and secondary pages.

## G1 Device time counter

- **Answer:**
  - **Reading it:** `bladerf_get_timestamp(dev, dir, &ts)` returns a 64-bit FPGA counter. There is one register per direction (`NIOS_PKT_8x64_TIMESTAMP_RX` and `_TX`).
  - **Unit:** samples. The header says "Free-running FPGA counter that monotonically increases at the sample rate of the associated channel".
  - **Intended use:** the header says the call gives only "a coarse estimate of the current timestamp when starting up a stream". It must not be used for individual samples in a running stream. Use `bladerf_metadata.timestamp` for that. "The timestamp counter will have advanced during the time that the captured value is propagated back from the FPGA to the host."
  - **Counters:**
    - RX and TX are two separate 64-bit signals, `rx_timestamp` and `tx_timestamp`.
    - They are clocked from the same signal, `ad9361.clock`: `alias tx_clock is ad9361.clock` and `alias rx_clock is ad9361.clock`.
    - Each has its own reset (`rx_ts_reset`, `tx_ts_reset`).
    - The wrapper `rx.vhd` drives `timestamp_reset` low only while `meta_en = '1'`. So the counter is held at reset while metadata is disabled and runs once metadata is enabled. The reset is not tied to the start of an individual stream.
    - `meta_en` comes from the Nios GPIO bit `nios_gpio.o.meta_sync`, shared by both directions.
  - **Setting or zeroing:** no API sets it. The GitHub issue "Add an API call that resets the RX and TX timestamps to 0" (#397) was closed "Won't Fix". The workaround given there is to disable both modules, then call `bladerf_sync_config()` and `bladerf_enable_module()` again. The same issue says "the first call to bladeRF_sync_config() will enable timestamps". That is a maintainer's comment, not documentation, so I treat the reset-on-reconfigure behaviour as INFERRED.
  - **PPS or trigger:** the `time_tamer` inputs `sync_in` and `pps` are tied to `'0'` in `bladerf-hosted.vhd`. The counter has no PPS alignment. A trigger gates samples (G9) but does not zero the counter.
  - **Sync format:** timestamps are only on with `BLADERF_FORMAT_SC16_Q11_META` or `SC8_Q7_META`. `BLADERF_CAP_TIMESTAMPS` appears at FPGA 0.1.0.
- **Status:**
  - VERIFIED: the unit (samples), the width (64 bits), the two registers, the clock, the `meta_en` hold-in-reset, and the absence of a set-timestamp API and of PPS alignment.
  - INFERRED: whether the RX and TX counters read the same value. A user on GitHub issue #962 says they match, which is third-party and unconfirmed. From the HDL they share a clock and are released by the same `meta_en`, but each can lag by a cycle or so.
  - INFERRED: "reset at module re-enable".
- **Evidence:**
  - `LIB/include/libbladeRF.h`: `bladerf_get_timestamp`, `struct bladerf_metadata`.
  - `LIB/src/backend/usb/nios_access.c`: `nios_get_timestamp`.
  - `hdl/fpga/platforms/bladerf-micro/vhdl/bladerf-hosted.vhd`: `rx_timestamp`, `tx_timestamp`, `rx_ts_reset`, `tx_ts_reset`, the `sync_in` and `pps` ties.
  - `.../rx.vhd` and `.../tx.vhd`: the `timestamp_reset` process.
  - https://github.com/Nuand/bladeRF/issues/397 and https://github.com/Nuand/bladeRF/issues/962.
  - `LIB/src/board/bladerf2/capabilities.c`: `bladerf2_get_fpga_capabilities`.

## G2 Per-sample time in an RX buffer

- **Answer:**
  - **Header:** with `BLADERF_FORMAT_SC16_Q11_META`, each USB message starts with a 16-byte header, which replaces the first 4 samples of the block. It holds an 8-byte little-endian timestamp at offset 0x04 and a 4-byte little-endian flags word at offset 0x0c. The timestamp is the one for the first sample of the message.
  - **Block size:** 1024 samples on USB 2.0, 2048 on USB 3.0 (header doc, per the fetcher).
  - **`bladerf_sync_rx` with a metadata struct:**
    - On input, `metadata->timestamp` is the first-sample time you want.
    - `sync_rx` seeks by dropping samples ("Discarding rest of buffer", "Seeking within message", "Seeking to message").
    - If that time is already past and nothing has been copied yet, it returns `BLADERF_ERR_TIME_PAST`.
    - With `BLADERF_META_FLAG_RX_NOW`, the call returns whatever is available, and the first sample's time is written back to `metadata->timestamp`.
  - **Timed start on the device:** there is none for RX streams. The stream runs from `bladerf_enable_module()`. The user gets a timed start by choosing the requested timestamp, or by using the trigger (G9).
  - **Header contents:** the FPGA header also carries two `mini_exp` bits at [113:112], and a constant `0x12344321` in the low 32 bits for the non-packet format.
- **Status:** VERIFIED (header, `sync.c`, `metadata.h`, `fifo_writer.vhd`).
- **Evidence:**
  - `LIB/src/streaming/sync.c`: `sync_rx`, states `SYNC_META_STATE_HEADER` and `SYNC_META_STATE_SAMPLES`.
  - `LIB/src/streaming/metadata.h`: `METADATA_HEADER_SIZE`, `METADATA_TIMESTAMP_SIZE`, `METADATA_FLAGS_OFFSET`.
  - `hdl/fpga/ip/nuand/synthesis/fifo_writer.vhd`: the meta word layout.
  - `LIB/include/libbladeRF.h`: the `BLADERF_FORMAT_SC16_Q11_META` doc and `BLADERF_META_FLAG_RX_NOW`.

## G3 Timed TX, burst marks

- **Answer:**
  - **Start time:** yes, per burst. `BLADERF_META_FLAG_TX_BURST_START` marks the buffer as the start of a burst, and its `metadata->timestamp` is the send time. Only the `_META` formats support this, and `bladerf_sync_tx` requires a metadata struct with them.
  - **End mark:** `BLADERF_META_FLAG_TX_BURST_END` marks the buffer as the end of a burst. It flushes the remainder of the sync working buffer. The header doc says "Since v1.3.0, the library enforces the trailing zero samples the hardware needs" (as the fetcher reports it).
  - **No timestamp:** `BLADERF_META_FLAG_TX_NOW`, used together with `TX_BURST_START`, means "transmit as soon as possible". The timestamp is then written to the FPGA header as 0 (`sync_tx`: with `now` set, "the header timestamp is written as 0").
  - **Inside a burst:** `BLADERF_META_FLAG_TX_UPDATE_TIMESTAMP` zero-pads up to `metadata->timestamp` within a burst. The code fills the gap between `user_meta->timestamp` and the running `curr_timestamp` with zeros. If fewer than three zeros would end a message, the padding carries into the next message.
  - **Handling in `handle_tx_parameters`:**
    - A second `BURST_START` while in a burst returns `BLADERF_ERR_INVAL`.
    - `BURST_START` without `TX_NOW` and with a past timestamp returns `BLADERF_ERR_TIME_PAST`.
    - `TX_NOW` without `BURST_START` returns `BLADERF_ERR_INVAL`.
    - `UPDATE_TIMESTAMP` alone with a past timestamp returns `BLADERF_ERR_TIME_PAST`.
    - `UPDATE_TIMESTAMP` together with `BURST_START` is ignored and logged.
    - `BURST_END` outside a burst returns `BLADERF_ERR_INVAL`.
  - **Late timestamp in the FPGA** (`fifo_reader.vhd`):
    - A buffer whose time is already past is released immediately. The "future" test fails, so `meta_time_go` goes high. There is no drop path and no lateness flag.
    - The comparison is a plain 64-bit unsigned compare with no wrap handling.
    - So the host check in `sync_tx` is the only place a late burst start is refused. Past that, late data goes out at once.
  - **Between bursts:** nothing is zero-padded for you. When the sample FIFO has no data due, `data_v` is cleared. The code shows no explicit zero fill. The bit values on the output while waiting are INFERRED, because `fifo_reader.vhd` does not force them to zero.
  - **Samples reach the FPGA only when a buffer fills:** the header says "Samples will only be sent to the FPGA when a buffer have been filled."
- **Status:** VERIFIED for flags, host checks, and the FPGA late-release rule (from the fetcher's reading of `fifo_reader.vhd`). INFERRED for the sample values on the DAC while idle.
- **Evidence:**
  - `LIB/include/libbladeRF.h`: `BLADERF_META_FLAG_TX_*`, the `bladerf_sync_tx` doc.
  - `LIB/src/streaming/sync.c`: `handle_tx_parameters`, `sync_tx`, `timestamp_in_past`.
  - `hdl/fpga/ip/nuand/synthesis/fifo_reader.vhd`: `META_LOAD`, `META_WAIT`, `meta_time_go`, `META_NOW`.

## G4 Scheduled retune and gain

- **Answer:**
  - **Frequency:** yes.
    - Call `bladerf_schedule_retune(dev, ch, timestamp, frequency, quick_tune)`. It needs a stream configured with the `_META` format for that channel.
    - The timestamp is the channel's sample timestamp.
    - `BLADERF_RETUNE_NOW` is `(bladerf_timestamp)0`, meaning "retune occurs immediately". A past timestamp also retunes immediately.
    - Quick tune: `bladerf_get_quick_tune(dev, ch, &qt)` captures the settings for the current frequency. The header says it gives "a faster retune, with a potential trade off of increased phase noise". These settings are "sensitive to changes in the operating environment" and must be refreshed.
    - **2.0 micro:** "NULL quick_tune parameters are not supported by the bladeRF 2.0 micro", so a quick-tune profile is mandatory.
  - **Queue:** the FPGA/Nios holds a queue of scheduled retunes.
    - `BLADERF_ERR_QUEUE_FULL` (-15) is returned if the queue is full. The code logs "The FPGA's retune queue is full. Try again after a previous request has completed."
    - I found no numeric queue depth anywhere (see "Searches that found nothing").
    - `bladerf_cancel_scheduled_retunes(dev, ch)` clears it (Nios packet `NIOS_PKT_RETUNE2_CLEAR_QUEUE`, `0xffffffffffffffff`). It is also done automatically at `bladerf_close()`.
    - Limits on profiles: `NUM_BBP_FASTLOCK_PROFILES 256` (Nios side) and `NUM_RFFE_FASTLOCK_PROFILES 8` (AD9361 slots). The code logs "Reached maximum number of TX quick tune profiles." when the cap is hit. Quick tune also sets `rfic_reset_on_close = true` as a workaround for RFIC state drift.
  - **Accuracy:** the doc says the timestamp is "the sample timestamp to perform the retune operation". The response packet reports a 64-bit "duration" of how long the operation took, valid only for a retune-now request. I found no stated accuracy or latency numbers.
  - **Gain or anything else:** the only scheduled operation in the public API is retune. `bladerf_set_gain`, `_set_gain_stage`, `_set_bandwidth`, `_set_sample_rate` are immediate calls with no timestamp argument (INFERRED from `libbladeRF.h`: no timed variant found).
  - **Immediate retune cost:** with host tuning (default) each call goes through AD9361 SPI from the host. In FPGA tuning mode, `_rfic_fpga_spinwait` polls the write-queue length up to `TRIES 30` × `DELAY 100` µs, about 3 ms, then returns `BLADERF_ERR_TIMEOUT`. Default tuning mode is host, unless `BLADERF_DEFAULT_TUNING_MODE=fpga` is set and the FPGA has `BLADERF_CAP_FPGA_TUNING`.
  - **What samples do while retuning:** not stated in any source.
- **Status:** VERIFIED for the API and the behaviours above. Queue depth, timing accuracy and the effect on samples: UNKNOWN.
- **Evidence:**
  - `LIB/include/libbladeRF.h`: `bladerf_schedule_retune`, `BLADERF_RETUNE_NOW`, `bladerf_cancel_scheduled_retunes`, `bladerf_get_quick_tune`.
  - `LIB/src/board/bladerf2/bladerf2.c`: `bladerf2_schedule_retune` (checks `BLADERF_CAP_SCHEDULED_RETUNE`, calls `backend->retune2`), `bladerf2_get_quick_tune`.
  - `fpga_common/include/nios_pkt_retune2.h`.
  - `LIB/src/backend/usb/nios_access.c`: `nios_retune2`.
  - `LIB/src/board/bladerf2/rfic_fpga.c`: `_rfic_fpga_spinwait`.
  - `LIB/src/board/bladerf2/common.c`: `default_tuning_mode`.

## G6 RX loss report

- **Answer:**
  - **In the FPGA** (`fifo_writer.vhd`): when the sample FIFO is full, samples presented are not written. The writer drops to `HOLDOFF` and resumes only when `fifo_enough` is true, which is stricter than "not full". No new header is started until then. The first header after recovery carries the (later) counter value, so the timestamp counter keeps running through the gap.
  - **Counters:** `overflow_count` (64-bit) counts discontinuous overflow events, not samples. It is wired `overflow_count => open` in `rx.vhd`, so it is not exposed. The header carries no overflow field.
  - **In libbladeRF** (`sync_rx`, as the fetcher summarizes it):
    - If samples were already copied in this call and the next header's timestamp differs from the expected `curr_timestamp`, it sets `BLADERF_META_STATUS_OVERRUN` and returns early with the partial count.
    - `actual_count` then holds the number of contiguous valid samples in the buffer. The header says it "will not be equal to the requested count in the event of a discontinuity", and the contents past that count are undefined.
    - The debug log shows expected versus received timestamps, but `user_meta->timestamp` is not updated on this return path (fetcher's reading).
  - **Lost count:** no field gives the number of lost samples. The host can compute it from the timestamp jump. The next `bladerf_sync_rx` call, with `RX_NOW` set, writes back the first returned sample's timestamp. Lost = that value minus the expected next timestamp.
  - **Meaning of the flag:** the header says "A sample overrun has occurred. This indicates that either the host (more likely) or the FPGA is not keeping up."
  - **Other status:** `BLADERF_META_FLAG_RX_HW_UNDERFLOW (1<<0)` is documented as asserted "when an underflow is detected in the sample buffering system on the device". `BLADERF_META_FLAG_RX_HW_MINIEXP1` and `_MINIEXP2` (1<<16, 1<<17) report the expansion pins.
  - **Host stream errors:** worker `stream_error` values pass through to the caller. Calling `sync_rx` again restarts the stream.
  - **Timeouts:** `bladerf_sync_rx` returns `BLADERF_ERR_TIMEOUT` when `timeout_ms` expires, and `BLADERF_ERR_TIME_PAST` for a past requested time.
- **Status:** VERIFIED for the FPGA and header behaviour. The `sync_rx` details rest on the fetcher's summary of `sync.c` (VERIFIED at that level). The lost-count derivation is INFERRED from those facts.
- **Evidence:**
  - `LIB/include/libbladeRF.h`: `BLADERF_META_STATUS_OVERRUN`, `actual_count`, `BLADERF_META_FLAG_RX_HW_UNDERFLOW`.
  - `LIB/src/streaming/sync.c`: `sync_rx`.
  - `hdl/fpga/ip/nuand/synthesis/fifo_writer.vhd`: `WRITE_SAMPLES`, `HOLDOFF`, `fifo_enough`, `overflow_count`.
  - `hdl/fpga/platforms/bladerf-micro/vhdl/rx.vhd`: `overflow_count => open`.

## G7 TX underflow report

- **Answer:**
  - **FPGA** (`fifo_reader.vhd`):
    - `underflow_detected` is set when the stream is enabled, the sample FIFO is empty, and data is due. Due means the meta entry's time has been reached or metadata is off.
    - Waiting for a future timestamp is not an underflow.
    - It drives `underflow_count`, which counts discontinuous underflows, and an LED stretched by `underflow_duration`.
    - In `tx.vhd` the count is wired `underflow_count => open`, and the LED is the only output. So the host cannot read it from the counter.
  - **Host:** `BLADERF_META_STATUS_UNDERRUN (1<<1)` exists in the header: "A sample underrun has occurred. This generally only occurs on the TX channel when the FPGA is starved." The fetcher's reading of the header adds that libbladeRF does not report it yet. In `sync.c` the only underflow flag handled is the RX-side `BLADERF_META_FLAG_RX_HW_UNDERFLOW`, and `sync_tx` has no underrun handling. So on TX the host gets no underrun event from libbladeRF. Worker stream errors (`BLADERF_ERR_IO` and similar) surface through the sync call.
  - **Late burst start:** refused by `sync_tx` with `BLADERF_ERR_TIME_PAST` (G3).
- **Status:** VERIFIED for `fifo_reader.vhd`, `tx.vhd`, and the `sync_tx` handling. The "does not report it yet" sentence is the fetcher's reading of a header comment (INFERRED).
- **Evidence:** `hdl/fpga/ip/nuand/synthesis/fifo_reader.vhd` (`underflow_detected`, `underflow_count`), `hdl/fpga/platforms/bladerf-micro/vhdl/tx.vhd`, `LIB/include/libbladeRF.h` (`BLADERF_META_STATUS_UNDERRUN`), `LIB/src/streaming/sync.c`.

## G8 External frequency reference

- **Answer:**
  - **Input:** a REFIN connector, `J95` U.FL, "nearly any frequency up to 300 MHz", default 10 MHz. It is AC-coupled and internally biased for 3.3 V CMOS or sine, and the reference oscillator section says not to exceed 3.3 Vpp (Nuand wiki "Taming Your VCTCXO"). Code range: `bladerf2_pll_refclk_range` is min 5000000, max 300000000, step 1 (Hz); `BLADERF_REFIN_DEFAULT = 10000000`.
  - **API:**
    - `bladerf_set_pll_enable(dev, true)` enables the ADF4002 phase detector and synthesizer. "Enabling this disables the VCTCXO trimmer DAC, and vice versa" (code: `CFG_GPIO_PLL_EN` bit 11).
    - `bladerf_set_pll_refclk(dev, hz)` and `bladerf_get_pll_refclk_range` set and bound the reference.
    - `bladerf_get_pll_lock_state(dev, &locked)` reads the ADF MUXOUT bit (`RFFE_CONTROL_ADF_MUXOUT`). The wiki says to expect "(locked)" after a delay.
  - **Clock select:** `bladerf_set_clock_select(dev, CLOCK_SELECT_ONBOARD|EXTERNAL)` switches the system clock between the on-board VCTCXO and an external clock input (`CFG_GPIO_CLOCK_SELECT`, bit 18). `bladerf_set_clock_output(dev, bool)` enables the clock output (`CFG_GPIO_CLOCK_OUTPUT`, bit 17). Set-clock-select logs a warning at high speed: "USB 3.0 recommended for reliable clock select assignment." Nuand's product text says the micro can "receive and provide its 38.4MHz fundamental clock from and to other devices".
  - **Loss reporting:** no event. Lock state is a poll of one bit. No source covers detection of a lost or absent reference on the external-clock path, only the PLL lock bit.
  - **PPS:** no 1PPS input is described for the micro. `bladerf2_set_vctcxo_tamer_mode` and `_get_` just return `BLADERF_ERR_UNSUPPORTED`. The `BLADERF_VCTCXO_TAMER_1_PPS` and `_10_MHZ` modes in the header are for other boards.
  - **CLK_OUT level and connector:** not found (see "Searches that found nothing").
- **Status:** VERIFIED for the API and register bits. REFIN connector name and level come from the Nuand wiki (VERIFIED as documented, not checked against schematics). CLK_OUT and CLK_IN connectors and levels UNKNOWN.
- **Evidence:**
  - `LIB/src/board/bladerf2/bladerf2.c`: `bladerf_set_pll_enable`, `bladerf_get_pll_lock_state`, `bladerf_set_clock_select`, `bladerf_set_clock_output`, `bladerf2_set_vctcxo_tamer_mode`, `bladerf2_pll_refclk_range`.
  - `fpga_common/include/bladerf2_common.h`: `BLADERF_REFIN_DEFAULT`, `CFG_GPIO_PLL_EN`, `CFG_GPIO_CLOCK_OUTPUT`, `CFG_GPIO_CLOCK_SELECT`.
  - `LIB/include/bladeRF2.h` docs.
  - https://github.com/Nuand/bladeRF/wiki/Taming-Your-VCTCXO.
  - https://www.nuand.com/bladerf-2-0-micro/.

## G9 Multi-device alignment

- **Answer:**
  - **Trigger API:**
    - `bladerf_trigger_init(dev, ch, signal, &trig)`, `bladerf_trigger_arm(dev, &trig, arm, 0, 0)`, `bladerf_trigger_fire(dev, &trig)`, `bladerf_trigger_state(...)`.
    - Roles: `BLADERF_TRIGGER_ROLE_DISABLED`, `_MASTER`, `_SLAVE`.
    - Signals valid as of FPGA v0.6.0: `BLADERF_TRIGGER_J71_4`, `BLADERF_TRIGGER_J51_1` ("J51 pin 1, mini_exp[1] on xA4/xA5/xA9") and `BLADERF_TRIGGER_MINI_EXP_1`. All three behave the same. The Nios code says "Only 1 external trigger is currently supported".
    - Capability: `BLADERF_CAP_TRX_SYNC_TRIG` at FPGA 0.6.0.
  - **Trigger hardware:** in `bladerf-hosted.vhd`, `rx_trigger_line` and `tx_trigger_line` are both aliased to `mini_exp1`, so RX and TX share one trigger pin. A master drives the pin; slaves take it as an input.
    - **Wiring caution:** the header warns "Configuring two devices in the trigger chain (or both RX and TX on a single device) as masters can damage the associated FPGA pins."
    - **Fire:** `trigger_fire` works only for a master. For a slave it returns `BLADERF_ERR_INVAL`.
    - **Shutdown:** disarm triggers before stopping streams, or shutdown blocks for the full timeout.
  - **What it gates:**
    - In `trigger.vhd` the logic is level-sensitive, with no clock, no synchronizer and no edge detector. `triggered <= not armed or not trigger_in;` and `signal_out <= signal_in when triggered else DEFAULT_OUTPUT;`.
    - The master drives `trigger_out <= not fired when master else 'Z'`.
    - The RX side gates `data_v` to `'0'` while untriggered (`rx_mux`), so samples are marked invalid. The TX side gates the sample FIFO empty flag.
    - Header doc: with the role disabled, samples are not gated.
  - **Alignment accuracy:** no figure in any source. Master to slave delay is only combinational logic plus board wiring. UNKNOWN.
  - **Timestamp counters across boards:** each board has its own counters starting at its own enable (G1), and there is no PPS alignment. To align, use the trigger for a common start, and the sample timestamps after that are per board.
  - **Clocks across boards:** on-board 38.4 MHz VCTCXO, optionally disciplined by REFIN (G8) or replaced by an external clock input, with a clock output for daisy-chaining. Two boards sharing one reference have a defined frequency relationship, not a documented phase relationship.
- **Status:** VERIFIED for the API, pin aliasing and HDL logic. Alignment accuracy and external-clock connector details UNKNOWN.
- **Evidence:**
  - `LIB/include/libbladeRF.h`: `bladerf_trigger_*`, `enum bladerf_trigger_signal`, `enum bladerf_trigger_role`.
  - `LIB/src/backend/usb/nios_access.c`: `nios_read_trigger`, `nios_write_trigger`.
  - `hdl/fpga/ip/nuand/trigger/trigger.vhd`.
  - `hdl/fpga/platforms/bladerf-micro/vhdl/{bladerf-hosted.vhd, rx.vhd, tx.vhd}`.
  - `LIB/src/board/bladerf2/capabilities.c`.

## G10 Full duplex

- **Answer:**
  - Yes. The FPGA has independent RX and TX paths with separate streams, each configured with its own `bladerf_sync_config()` and enabled with `bladerf_enable_module()` for RX and TX. The 2.0 micro is described as 2x2 MIMO.
  - Order and limits: RF front ends must be enabled before streaming (`bladerf_enable_module` doc). The aggregate throughput check is `check_total_sample_rate`, with `MAX_SAMPLE_THROUGHPUT` of 80 Msps, or 160 Msps with `BLADERF_FEATURE_OVERSAMPLE`. It sums the sample rate of each enabled RX and TX channel and logs a warning, "total sample throughput", if the sum is higher. It is a warning, not an error return, for the call.
  - TX with timing: wait for samples to reach the front end before disabling the module (`bladerf_enable_module` doc).
  - Switch time between directions: not applicable. RX and TX are separate chains.
- **Status:** VERIFIED (header, `common.c`, HDL).
- **Evidence:** `LIB/src/board/bladerf2/common.c` (`check_total_sample_rate`, `MAX_SAMPLE_THROUGHPUT`), `LIB/include/libbladeRF.h` (`bladerf_enable_module`, `bladerf_sync_config`), `LIB/src/board/bladerf2/rfic_host.c` (`_rfic_host_enable_module` calls `check_total_sample_rate`).

## G11 Channels and what is shared

- **Answer:**
  - **Count:** 2 RX and 2 TX. Channel macros: `BLADERF_CHANNEL_RX(n)` is `((n)<<1)|0` and `BLADERF_CHANNEL_TX(n)` is `((n)<<1)|1`. Layouts: `BLADERF_RX_X1`, `BLADERF_TX_X1`, `BLADERF_RX_X2` ("x2 RX (MIMO)"), `BLADERF_TX_X2`. `bladerf2_sync_config` accepts those four and returns -EINVAL for anything else. Enabling a channel forces a frequency re-set to recalibrate (`_rfic_host_enable_module`). In MIMO, `rffe` control bits `MIMO_RX_EN_0/1` and `MIMO_TX_EN_0/1` enable channels.
  - **Frequency:** set per channel through `bladerf_set_frequency(dev, ch, hz)`. The API takes a channel. The AD9361 has one RX LO and one TX LO. The code calls `ad9361_set_rx_lo_freq` or `ad9361_set_tx_lo_freq` once per direction, and its band-select updates the SPDT bits "for both channels in the direction". The one-LO-per-direction structure is INFERRED from that and from the AD9361 datasheet (not fetched). `bladerf_set_frequency` on RX1 and RX2 therefore reaches the same direction LO. What the API reports back for the second channel is not stated.
  - **Sample rate:**
    - The API sets it per channel (`bladerf_set_sample_rate(dev, ch, rate, &actual)`), and `rfic_host.c` calls `ad9361_set_tx_sampling_freq` or `ad9361_set_rx_sampling_freq` by direction, with no coupling code in libbladeRF.
    - In the upstream no-OS AD9361 driver both setters call `ad9361_calculate_rf_clock_chain(...)` and `ad9361_set_trx_clock_chain(phy, rx, tx)`, so each reprograms both directions. This is from `analogdevicesinc/no-OS` `main`, not the copy vendored in the Nuand tag (its path 404'd), so INFERRED for bladeRF.
    - An AD9361 forum answer says RX and TX must have equal rates. Another thread cites a 1x or 2x ratio. A GitHub issue comment says "Setting the RX sample rate also sets the TX rate, and vice versa."
    - Net: INFERRED that RX and TX sample rates are tied. Both directions share one data clock in the FPGA (`ad9361.clock`).
  - **Bandwidth:** set per channel through `bladerf_set_bandwidth`; calls `ad9361_set_rx_rf_bandwidth` or the TX equivalent by direction.
  - **Gain:** per channel (`bladerf_set_gain(dev, ch, gain)`). The RX gain mode is per channel: channel 0 maps to `gc_rx1_mode`, channel 1 to `gc_rx2_mode`. TX gain mode does not exist.
  - **RF port (antenna):** per channel via `bladerf_set_rf_port` / `bladerf_get_rf_ports`; the port/SPDT selection is made by band in `select_band` (low band 70 MHz to 3 GHz, high band above, per the `bladerf2_common.h` band maps). Names of the ports: not found.
  - **Bias tee:** per channel direction, bits `RFFE_CONTROL_RX_BIAS_EN` (bit 5) and `RFFE_CONTROL_TX_BIAS_EN` (bit 10), so one per direction rather than per channel.
- **Status:** VERIFIED for counts, layouts, API granularity and gain mode mapping. LO sharing and RX/TX rate tying: INFERRED (AD9361 documents not read).
- **Evidence:** `LIB/include/libbladeRF.h`, `LIB/src/board/bladerf2/{bladerf2.c, rfic_host.c, common.c}`, `fpga_common/include/bladerf2_common.h`, https://github.com/analogdevicesinc/no-OS (`drivers/rf-transceiver/ad9361/ad9361_api.c`), https://github.com/Nuand/bladeRF/issues/962, https://ez.analog.com/rf/wide-band-rf-transceivers/design-support/f/q-a/564971/tx-and-rx-sampling-rate.

## G12 Sample rate and bandwidth

- **Answer:**
  - **Rate range** (`fpga_common/include/bladerf2_common.h`):
    - Base: min 520834, max 61440000, step 2, scale 1. The step of 2 is inside the table, so odd rates are not in the range.
    - Oversample: min 6250000, max 122880000, step 2.
    - `bladerf2_sample_rate_range_4x`: min 520834, max 2083334, step 1. Below 2083334 the code switches RX to `BLADERF_RFIC_RXFIR_DEC4` and TX to `BLADERF_RFIC_TXFIR_INT4`. It also steps through an intermediate 30 MHz rate when crossing the 40e6 and 2083334 thresholds.
  - **Set and read-back:**
    - `bladerf_set_sample_rate(dev, ch, rate, &actual)` takes an integer Hz. `bladerf_set_rational_sample_rate` exists, but on this board it drops the fraction: `integer_rate = rate->integer + rate->num / rate->den` with integer division. `bladerf2_get_rational_sample_rate` reports `num = 0`, `den = 1`.
    - So rates are whole Hz. Read-back is through `actual` or `bladerf_get_sample_rate`, which come from `ad9361_get_*_sampling_freq`. Whether the AD9361 rounds the request is not stated in libbladeRF, so the `actual` value is the way to know.
  - **Oversample mode:** `bladerf_enable_feature(dev, BLADERF_FEATURE_OVERSAMPLE, true)`; enum comment "Enforces AD9361 OC and 8bit mode". In `bladerf2_set_sample_rate`, in that mode `rate /= 2` before `rfic->set_sample_rate`. `bladerf2_sync_config` rejects `SC16_Q11` and `SC16_Q11_META` in this mode with `BLADERF_ERR_UNSUPPORTED` ("16bit format unsupported with OVERSAMPLE feature enabled"). `bladerf2_set_bandwidth` logs "bandwidth assignements with oversample feature enabled yields unkown results".
  - **Bandwidth:** `bladerf2_bandwidth_range` is min 200000, max 56000000, step 1 (Hz). The header says the device "is capable of a discrete set of bandwidth values. The caller should check the `actual` parameter". The code clamps to the range, then calls the AD9361 RF-bandwidth setter and reads it back.
  - **Filters tied to rate:** RFIC FIR enums `BLADERF_RFIC_RXFIR_{BYPASS,CUSTOM,DEC1,DEC2,DEC4}` and `BLADERF_RFIC_TXFIR_{BYPASS,INT1,INT2,INT4}`. FIR choice follows the rate thresholds above, and init sets the defaults. `bladerf_get_sample_rate_range` and `bladerf_get_bandwidth_range` return these tables, with the oversample table when that feature is on.
  - **Device-reported range:** a user printout (GitHub issue #947) shows `Range: [520834, 122880000]` for the sample rate and `[200000, 56000000]` for bandwidth. Product page: 0.521 to 61.44 Msps and "<0.2 to 56 MHz".
- **Status:** VERIFIED (`bladerf2_common.h` tables; `bladerf2.c` code).
- **Evidence:** `fpga_common/include/bladerf2_common.h` (`bladerf2_sample_rate_range_*`, `bladerf2_bandwidth_range`), `LIB/src/board/bladerf2/bladerf2.c` (`bladerf2_set_sample_rate`, `bladerf2_set_rational_sample_rate`, `bladerf2_get_rational_sample_rate`, `bladerf2_set_bandwidth`, `bladerf2_sync_config`), `LIB/src/board/bladerf2/rfic_host.c`, `LIB/include/libbladeRF.h` (`bladerf_set_bandwidth`, `BLADERF_FEATURE_OVERSAMPLE`), https://github.com/Nuand/bladeRF/issues/947.

## G13 Frequency and gain

- **Frequency range** (`bladerf2_common.h`): RX min 70000000, max 6000000000, step 2. TX min 47000000, max 6000000000, step 2. The band maps show TX low band starting at 46875000. Out-of-range requests return `BLADERF_ERR_RANGE` (`rfic_host.c: set_frequency`). `bladerf_frequency` is a 64-bit Hz value. LO resolution and rounding by the AD9361: not stated in libbladeRF. `bladerf_get_frequency` reads back.
- **RX gain** (`bladerf2_rx_gain_ranges`, gain in dB, step 1, scale 1):

| Stage | 0 to 1.3 GHz | 1.3 to 4 GHz | 4 to 6 GHz | Offset to overall |
|---|---|---|---|---|
| overall (unnamed) | 1-17 to 77-17 = -16 to 60 | -4-11 to 71-11 = -15 to 60 | -10-2 to 62-2 = -12 to 60 | -17 / -11 / -2 |
| "full" | 1 to 77 | -4 to 71 | -10 to 62 | 0 |

  The `-17.0f`, `-11.0f`, `-2.0f` offsets and the arithmetic for the unnamed rows are in the source table. A user printout (issue #947) shows RX overall `[-15, 60]` and "full" `[-4, 71]` at 2.2 GHz. I read the unnamed row's computed min and max from the formula strings, so the derived numbers are my arithmetic. Header: "On receive channels, 60 dB is the maximum gain level."
- **TX gain** (`bladerf2_tx_gain_ranges`, scale 0.001): overall `-23.75` to `66` (code `__round_int64(1000*(-89.750 + 66.0))` to `1000*(0+66.0)`), step 0.25, offset 66.0. Stage "dsa": `-89.75` to `0`, step 0.25. Header: "On transmit channels, 60 dB is defined as approximately 0 dBm." In `rfic_host.c`, a TX `set_gain_stage` value below -89 dB is forced to `-89750` (mute value), and TX channels are muted at init.
- **Stage names:** RX `"full"` (get and set), `"digital"` (get only, per `rfic_host.c`); TX `"dsa"`. `bladerf_get_gain_stages` collects the named entries. Unnamed entries are the system gain.
- **Gain ranges depend on frequency:** yes, for RX, across the three bands above (the header says the range "may vary depending on the configured frequency, so it should be checked after setting the desired frequency"). TX does not vary by frequency in the table.
- **AGC:** `bladerf_set_gain_mode(dev, ch, mode)` with `BLADERF_GAIN_DEFAULT` (automatic), `BLADERF_GAIN_MGC`, `BLADERF_GAIN_FASTATTACK_AGC`, `_SLOWATTACK_AGC`, `_HYBRID_AGC`. The names exposed as `bladerf2_rx_gain_modes` are "automatic", "manual", "fast", "slow", "hybrid". RX only; for TX, `bladerf2_get_gain_modes` returns no modes, and `bladerf_set_gain_mode` on TX logs a warning and does nothing in `rfic_host.c`.
- **Single number:** `bladerf_set_gain` is documented as "an overall system gain, optimally proportioning the gain between multiple gain stages if applicable". The value is clamped to the range for the current frequency.
- **Status:** VERIFIED (`bladerf2_common.h`, `rfic_host.c`, `bladerf2.c`).
- **Evidence:** `fpga_common/include/bladerf2_common.h`, `LIB/src/board/bladerf2/{rfic_host.c, bladerf2.c}`, `LIB/include/libbladeRF.h` (`bladerf_set_gain`, `bladerf_set_gain_mode`, `bladerf_get_gain_stage_range`).

## G14 Sample formats and throughput

- **Formats** (enum doc in `libbladeRF.h`):
  - `BLADERF_FORMAT_SC16_Q11`: 16-bit interleaved I/Q, values in [-2048, 2048) for [-1.0, 1.0). The AD9361 data converters are 12-bit.
  - `BLADERF_FORMAT_SC16_Q11_META`: as above, with a 16-byte metadata header replacing the first 4 samples of each block.
  - `BLADERF_FORMAT_SC16_Q11_PACKED`: 12-bit packing, "allows for a 33.3% higher sample rate when bandwidth limited by the host interface". Buffer length must be a multiple of 4096 samples. It costs CPU.
  - `BLADERF_FORMAT_SC8_Q7`: values in [-128, 128). `BLADERF_FORMAT_SC8_Q7_META` is the 8-bit version with a header. `BLADERF_CAP_FPGA_8BIT_SAMPLES` appears at FPGA 0.15.0.
  - `BLADERF_FORMAT_PACKET_META`: packets for FPGA cores.
- **Host link:** USB 3.0 SuperSpeed (bus powered); also works at high speed. `bladerf2_open` picks the message size by speed.
- **Sync buffering:** `bladerf_sync_config(dev, layout, format, num_buffers, buffer_size, num_transfers, stream_timeout)`. The doc says `num_buffers` must exceed `num_transfers`, and `buffer_size` (in samples) must be a multiple of 8192 bytes (2048 on firmware older than 2.5.0), times 3 for the packed format.
- **Throughput:** no measured figure found. The library's own cap is `MAX_SAMPLE_THROUGHPUT`, 80 Msps (160 Msps in oversample mode), summed over enabled channels and directions. At 4 bytes per complex SC16 sample, 80 Msps is 320 MB/s (my arithmetic). Exceeding it logs a warning, not an error.
- **Status:** VERIFIED for formats and caps. Effective USB throughput UNKNOWN.
- **Evidence:** `LIB/include/libbladeRF.h` (`enum bladerf_format`, `bladerf_sync_config`), `LIB/src/board/bladerf2/common.c` (`check_total_sample_rate`), `LIB/src/board/bladerf2/capabilities.c`.

## G15 Phase

- **Answer:**
  - **Channels sharing an LO:** libbladeRF, as read, has one `set_frequency` path per direction (`ad9361_set_rx_lo_freq` / `ad9361_set_tx_lo_freq`), and one band select for both channels of a direction. This is consistent with one RX LO and one TX LO in the AD9361. I did not read the AD9361 text, so INFERRED. Also, a secondary source says "both transmitters and receivers share the same oscillator on the micro 2.0" (the GNSS paper abstract, as search returned it; the PDF itself timed out).
  - **Phase after retune:** no Nuand or ADI source I could reach states what phase an RX or TX channel has after a retune, or that two channels keep a fixed relative phase. The quick-tune doc only says the faster retune has "a potential trade off of increased phase noise".
  - **Timed retune and phase:** nothing in the scheduled-retune doc or code says phase is aligned across the retune.
  - **The internal AD9361 phase:** an ADI forum reply (AD9364) says there is no internal RF synchronization and mentions a "constant phase offset of 180 degrees from the internal LO divider" with an external LO. That is a user-forum statement about a sibling part, INFERRED at best for the bladeRF.
  - **Across boards:** no phase relationship is documented, only the shared reference (G8).
- **Status:** UNKNOWN for the behaviours; INFERRED for the one-LO-per-direction structure.
- **Evidence:** `LIB/src/board/bladerf2/rfic_host.c` (`set_frequency`, `select_band`), `LIB/include/libbladeRF.h` (`bladerf_get_quick_tune`), https://ez.analog.com/rf/wide-band-rf-transceivers/design-support/f/q-a/105611/adding-my-own-dpd-module-to-fmcomms2-hdl/315578 (forum).

## G16 Path delay

- **Answer:** no RX or TX latency or group delay is stated in the libbladeRF header, docs, or code I read. The path includes the AD9361 digital filters (the FIR choice depends on the rate: DEC1, DEC2, DEC4 for RX; INT1, INT2, INT4 for TX; G12), the FPGA sample FIFOs, and USB message buffers of 1024 or 2048 samples (G2). The FIR delay in samples is not given in what I read; the FIR coefficient tables (`bladerf2_rfic_rx_fir_config`, `_dec2`, `_dec4`, `bladerf2_rfic_tx_fir_config`, `_int2`, `_int4`) are declared in `common.h` but I did not read their definitions.
- **Status:** UNKNOWN.
- **Evidence:** `LIB/src/board/bladerf2/common.h` (the extern declarations); searches in "Searches that found nothing".

## G17 Hardware repeat TX

- **Answer:** none. No repeat or loop function appears in `libbladeRF.h` for the sync or async TX path (searched for "repeat" and "loop" among the functions I read; the only loopback entries are `bladerf_set_loopback` with modes "none", "firmware" and "rf_bist", which route samples inside the device and are not a waveform repeater).
- **Status:** VERIFIED for the part of the header I read; the last 60 000 characters of `libbladeRF.h` were only partly covered (G21 section note), so a repeat function elsewhere in the file cannot be fully excluded.
- **Evidence:** `LIB/include/libbladeRF.h`; `LIB/src/board/bladerf2/bladerf2.c` (`bladerf2_loopback_modes`).

## G18 Safety

- **TX power:**
  - Nuand product page: "+8 dBm CW output power" (the page does not say min, typ or max).
  - `bladerf_set_gain` doc: 60 dB TX is "approximately 0 dBm". TX gain range is -23.75 to 66 dB, and the "dsa" stage is -89.75 to 0 dB (G13).
  - The max RX input power: not found.
- **Bias tee:** `bladerf_set_bias_tee(dev, ch, enable)` / `bladerf_get_bias_tee`. Setting a channel's bias tee sets one control bit per direction (RX bit 5, TX bit 10 of the RFFE control register). It needs `STATE_FPGA_LOADED`. The setter's doc title is a copy of the getter's ("Get current bias tee state"), and `enable` is "True to activate bias tee, false to deactivate". Voltage and current limits: not found in these sources. The product page mentions bias-tee accessories (BT-100, BT-200) but not an on-board bias tee.
- **TX muted at init:** `_rfic_host_initialize` calls `rfic->set_txmute(dev, ch, true)` for each TX channel ("Mute TX channels"). TX gain below -89 dB maps to the mute value.
- **Other settings with safety relevance:**
  - `bladerf_set_clock_output` enables an output.
  - The trigger master drives the `mini_exp1` pin, and two masters can damage FPGA pins (header warning).
  - `bladerf_set_loopback` should only change while RX and TX are disabled ("unexpected behavior may occur").
  - A header warning on the 1PPS/10 MHz tamer path says exceeding 1.8 V on J71-1 can damage the FPGA, but this applies to other boards because the 2.0 micro's tamer functions are unsupported.
  - `bladerf_get_power_source` reads the source (USB VBUS or DC).
- **Status:** VERIFIED for API and bits. TX power figure: Nuand page, VERIFIED as published. Bias-tee electrical limits UNKNOWN.
- **Evidence:** `LIB/src/board/bladerf2/bladerf2.c` (`bladerf_set_bias_tee`), `LIB/src/board/bladerf2/rfic_host.c`, `LIB/include/libbladeRF.h`, https://www.nuand.com/bladerf-2-0-micro/.

## G19 Device configuration description

- **Answer:** what identifies a unit from the code:
  - **Board and USB ids:** `bladerf2_matches()` checks the USB vendor and product ids. `bladerf_get_board_name` and `bladerf_devinfo` (serial, USB bus and address) exist; I did not read the struct.
  - **FPGA size:** the size is derived from the bitstream length: `bladerf2_get_fpga_bytes` returns A4 2632660, A5 4244820, A9 12858972 bytes. The product page gives 49 kLE (xA4) and 301 kLE (xA9).
  - **Versions and capabilities:** FPGA and firmware version are read at open, and capability flags derive from them:
    - FPGA: 0.1.0 `BLADERF_CAP_TIMESTAMPS`; 0.3.0 `BLADERF_CAP_PKT_HANDLER_FMT`; 0.3.2 `BLADERF_CAP_VCTCXO_TRIMDAC_READ`; 0.4.1 `BLADERF_CAP_MASKED_XBIO_WRITE`; 0.6.0 `BLADERF_CAP_TRX_SYNC_TRIG`; 0.10.0 `BLADERF_CAP_SCHEDULED_RETUNE`; 0.10.1 `BLADERF_CAP_FPGA_TUNING`; 0.12.0 `BLADERF_CAP_FPGA_PACKET_META`; 0.15.0 `BLADERF_CAP_FPGA_8BIT_SAMPLES`.
    - Firmware: 1.7.1 loopback; 1.8.0 query device ready; 1.9.0 log read; 2.1.0 supports bladeRF2; 2.3.0 flash id; 2.3.1 FPGA source; 2.4.0 short packet.
  - **Feature state:** `BLADERF_FEATURE_DEFAULT` or `_OVERSAMPLE` (G12), tuning mode host or FPGA (G4). A compatibility check warns on mismatched versions (`bladerf2.c`: "A failure only warns in most cases").
  - **Front ends and switches:** the RF port and SPDT selection is by band (G11). `bladerf_get_rf_switch_config` reads it back.
  - **Extra hardware:** expansion pins `mini_exp1` and `mini_exp2` (header J51 on xA4/xA5/xA9). Other modules (a PLL card on a different board) are not the micro's.
- **Status:** VERIFIED (`capabilities.c`, `bladerf2.c`, product page). The USB product id values: not read.
- **Evidence:** `LIB/src/board/bladerf2/capabilities.c`, `LIB/src/board/bladerf2/bladerf2.c` (`bladerf2_get_fpga_bytes`, `bladerf2_matches`, `_bladerf2_initialize`), https://www.nuand.com/bladerf-2-0-micro/.

## G20 Timing envelope numbers

- **Command lead (scheduled retune):** no minimum lead is stated. A past timestamp retunes immediately, so a lead of zero is accepted.
- **Retune queue depth:** UNKNOWN (no source gives a number). Overflow gives `BLADERF_ERR_QUEUE_FULL`.
- **Retune duration:** the response carries a 64-bit "duration" field, no value stated. FPGA-tuning mode polls for up to about 3 ms (G4).
- **TX start lead:** a burst start at time T requires T not past at the call (`BLADERF_ERR_TIME_PAST`). The host also needs time for the buffer to fill before it is sent to the FPGA ("Samples will only be sent to the FPGA when a buffer have been filled"), so the lead is at least one buffer. `bladerf_get_timestamp` doc suggests (as the fetcher reports it) tens to hundreds of ms.
- **RX start:** `bladerf_enable_module` starts the stream. Timed capture start works by the requested timestamp (G2), with a seek of a few messages; startup delay not stated. Worker start waits up to `SYNC_WORKER_START_TIMEOUT_MS` (250 ms by default) in `sync.c`.
- **Overflow resume gap:** the writer pauses until `fifo_enough` (more than one DMA buffer plus meta FIFO headroom) before resuming. No number of samples given.
- **Release window:** none in the retune interface; nothing similar found.
- **Message block size:** 1024 samples (USB 2.0), 2048 (USB 3.0) for metadata blocks.
- **Status:** the structure is VERIFIED; the numbers are UNKNOWN except where given.
- **Evidence:** `LIB/src/streaming/sync.c`, `fpga_common/include/nios_pkt_retune2.h`, `LIB/src/board/bladerf2/rfic_fpga.c`, `hdl/fpga/ip/nuand/synthesis/fifo_writer.vhd`, `LIB/include/libbladeRF.h`.

## G21 Disconnect

- **Answer:**
  - **Mapping** (`LIB/src/backend/usb/libusb.c`, `error_conv`): `LIBUSB_ERROR_NO_DEVICE` and `LIBUSB_ERROR_BUSY` map to `BLADERF_ERR_NODEV` (-7). `LIBUSB_ERROR_IO` maps to `BLADERF_ERR_IO` (-5). `LIBUSB_ERROR_TIMEOUT` maps to `BLADERF_ERR_TIMEOUT` (-6). Overflow, pipe, interrupted and not-found map to `BLADERF_ERR_UNEXPECTED` (-1).
  - **Stream transfers** (`lusb_stream_cb`): on a status other than completed, the state becomes `STREAM_SHUTTING_DOWN`. `LIBUSB_TRANSFER_NO_DEVICE` sets `stream->error_code = BLADERF_ERR_NODEV`. `LIBUSB_TRANSFER_ERROR`, `STALL` and `OVERFLOW` set `BLADERF_ERR_IO`. `LIBUSB_TRANSFER_TIMED_OUT` sets `BLADERF_ERR_TIMEOUT`. Cancelled sets nothing (expected during teardown). The user callback runs only while the state is `STREAM_RUNNING`. The fetcher could not show how `error_code` reaches the caller after the stream ends.
  - **Sync API:** worker errors come back through `sync_rx` and `sync_tx` (`SYNC_STATE_CHECK_WORKER` passes `stream_error` to the caller). There is no asynchronous disconnect event or callback in the API I read. Detection is by return value of a call.
  - **Other codes:** `BLADERF_ERR_UPDATE_FPGA` (-12), `BLADERF_ERR_UPDATE_FW` (-13), `BLADERF_ERR_FPGA_OP` (-16), `BLADERF_ERR_PERMISSION` (-17), `BLADERF_ERR_WOULD_BLOCK` (-18), `BLADERF_ERR_NOT_INIT` (-19). The table also includes `BLADERF_ERR_TIME_PAST` (-14) and `BLADERF_ERR_QUEUE_FULL` (-15).
  - **Device not responding:** `_rfic_fpga_spinwait` and the NIOS calls return `BLADERF_ERR_TIMEOUT` or `BLADERF_ERR_FPGA_OP`.
- **Status:** VERIFIED for the codes and mapping (`libbladeRF.h` error table, `libusb.c`).
- **Evidence:** `LIB/include/libbladeRF.h` (the `BLADERF_ERR_*` list), `LIB/src/backend/usb/libusb.c` (`error_conv`, `lusb_stream_cb`), `LIB/src/streaming/sync.c`.

## G22 One root per device

- **Answer:**
  - **Counters and clocks:** the HDL has one sample clock for RX and TX (`ad9361.clock`, the Nios-system port `ad9361_data_clock_clk`). Two timestamp counters (RX, TX) run from it (G1). The only other domains in `bladerf-hosted.vhd` are the FX3 PCLK PLL (USB side), an 80 MHz system clock created "from 38.4 MHz", and the Nios side.
  - **Channels:** both RX channels and both TX channels use the same `rx_clock` / `tx_clock`. A GitHub comment says in dual channel "both channel use the same timestamp" (issue #962, third-party).
  - **Parts with a separate clock:** the AD9361 RX and TX synthesizers are separate PLLs from the same reference (INFERRED from the AD9361 structure; datasheet not read), and the optional ADF4002 disciplines the 38.4 MHz VCTCXO. Neither appears as a host-visible clock domain.
  - **Multi-board:** each board has its own root counter pair. They are unaligned unless a trigger starts them together (G9).
- **Status:** VERIFIED for the single `ad9361.clock` domain in `bladerf-hosted.vhd`. INFERRED for the synthesizer structure.
- **Evidence:** `hdl/fpga/platforms/bladerf-micro/vhdl/bladerf-hosted.vhd` (`alias tx_clock`, `alias rx_clock`, `sys_clock` comment, `fx3_pclk_pll`).

## Searches that found nothing

- **UG-570 and AD9361 datasheet:** every WebFetch timed out: `https://www.analog.com/media/en/technical-documentation/data-sheets/AD9361.pdf` and `https://www.analog.com/media/en/technical-documentation/user-guides/ad9361-reference-manual-um-570.pdf`. So no UG-570 text was read, and the claims on RX/TX LO sharing, sample-rate tying and group delay stay INFERRED or UNKNOWN. `https://wiki.analog.com/resources/eval/user-guides/ad-fmcomms2-ebz/multichip-sync` returned 404.
- **Doxygen docs:** `https://www.nuand.com/libbladeRF-doc/v2.5.0/group__FN__BLADERF2.html` returned 404. `https://www.nuand.com/libbladeRF-doc/` returned an empty page. I used the headers at the tag instead.
- **Hardware docs:** `https://www.nuand.com/bladeRF-2.0-micro-hardware-guide` returned 404. The Mouser brief PDF (`https://www.mouser.com/datasheet/2/813/bladeRF_2_micro-brief-1509124.pdf`) timed out. The wiki home page loaded without a connector list (`Architecture` and `Bindings` pages failed to load). The CLK_OUT and CLK_IN connector names and levels, and the RX and TX RF port names, were not found. The GNSS paper `https://spcomnav.uab.es/docs/conferences/Low-cost_SDR_Platforms_for_Multi-Frequency_GNSS_Synchronous_Acquisition_1769266014.pdf` returned ECONNREFUSED.
- **Range tables:** the first tries at `LIB/src/board/bladerf2/{bladerf2.c, common.c, common.h}` and at `host/common/include/bladerf2_common.h` and `.../board/bladerf2/bladerf2_common.h` found nothing. The tables are in `fpga_common/include/bladerf2_common.h`.
- **Retune queue depth:** WebSearch "bladeRF nios retune queue size RETUNE_QUEUE scheduled retune queue length 16 bladerf2 nios_retune2 queue" found nothing. `nios_pkt_retune2.h` has no size. The Nios firmware source was not read.
- **GitHub code search and API:** `https://github.com/search?q=repo%3ANuand%2FbladeRF+bladerf2_tx_frequency_range&type=code` is behind sign-in. `https://api.github.com/search/code?...` returned 401.
- **HDL listing:** `https://api.github.com/repos/Nuand/bladeRF/contents/hdl/fpga/platforms/bladerf-2/vhdl?ref=2025.10` returned 404 (the platform directory is `bladerf-micro`).
- **No-OS copy at the tag:** `RAW thirdparty/analogdevicesinc/no-OS/ad9361/sw/ad9361_api.c` and the directory listing returned 404. I used `https://raw.githubusercontent.com/analogdevicesinc/no-OS/main/drivers/rf-transceiver/ad9361/ad9361_api.c` instead, which is a different tree.
- **FIR coefficient definitions and delay:** not read (only extern declarations in `LIB/src/board/bladerf2/common.h`). No source on RX or TX latency.
- **Phase after retune:** WebSearch on bladeRF 2.0 micro phase coherence, RX1/RX2 phase after retune, returned only Ettus and LimeSDR threads and issue #947 (not about phase).
- **1PPS on the micro:** no source describes a 1PPS input. `bladerf2_set_vctcxo_tamer_mode` returns `BLADERF_ERR_UNSUPPORTED`.
- **Repeat TX:** no function found in the parts of `libbladeRF.h` I read. The last portion of `libbladeRF.h` (characters 100000 onward) was returned in part, so the absence is not exhaustive.
- **RAW `hdl/fpga/platforms/bladerf-micro/vhdl/rx.vhd` and `tx.vhd` internals:** the timestamp counter module (`time_tamer`) VHDL and the Nios firmware's retune queue code were not read.

Skipped: line numbers (the fetcher gives none), the Nios firmware source, and the time-tamer and FPGA-tuning internals. Risk: several `sync.c` and HDL statements above rest on the fetcher's paraphrase, so recheck the overrun return path and the late-TX rule with `git grep` before relying on them.
