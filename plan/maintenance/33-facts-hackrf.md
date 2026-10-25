# HackRF One and HackRF Pro facts for the generality check (item A1)

- **Tag read:** `v2026.01.3`, the "Latest" release on https://github.com/greatscottgadgets/hackrf/releases/latest. The release notes say "Fix mixer frequency lock failures" and "access to a larger SPI flash on the HackRF Pro". It was published about 30 January 2026 (the date is inferred from the tag name).
- **Date of this report:** 2026-10-25.
- **Files read at the tag:**
  - host: `host/libhackrf/src/hackrf.h`, `hackrf.c`, `host/hackrf-tools/src/hackrf_transfer.c`
  - firmware: `firmware/hackrf_usb/usb_api_transceiver.c`, `usb_api_praline.c`, `usb_api_m0_state.c`, `usb_bulk_buffer.h`, `sgpio_m0.s`, `hackrf_usb.c`
  - firmware common: `firmware/common/m0_state.h`, `radio.c`, `rf_path.c`, `hackrf_core.c`, `sgpio.c`, `fpga.c`
  - docs: `docs/source/hackrf_pro.rst` and the pages on hackrf.readthedocs.io
  - v3 evidence: `/home/komatsu/local_work/Ez-SDRv4/v3/source/device/hackrf.d`
- **How the sources were read:** WebFetch returns a summary written by a small model, not raw text, so I have no line numbers. Quotes are short fragments the fetcher returned. URLs have the form `https://raw.githubusercontent.com/greatscottgadgets/hackrf/v2026.01.3/<path>`.
- **"URL" below** means that prefix plus the path in parentheses.

## HackRF Pro: what is public

- **Announcement and product pages** (VERIFIED as published, not checked against GSG's own announcement page).
  - Retailer and press pages (core-electronics, elektor, cnx-software, hak5) describe a successor to the HackRF One.
  - Operating range is 100 kHz to 6 GHz, tunable 0 Hz to 7.1 GHz.
  - It is half-duplex, up to 20 Msps, with 8-bit I/Q.
  - It has a 16-bit "extended-precision" mode at low sample rates and a 4-bit "half-precision" mode up to 40 Msps.
  - It uses an FPGA instead of the CPLD, has a TCXO, a USB-C High-Speed USB 2.0 port, and a "hardware-disable transmit mode" option.
  - The year of launch is not confirmed. Hak5 mentions an initial September shipment.
- **Docs:** `docs/source/hackrf_pro.rst` at the tag and https://hackrf.readthedocs.io/en/latest/hackrf_pro.html. They repeat the specs above.
  - They give no TX power figure.
  - They state "Designed to be backward compatible with HackRF One software and hardware".
  - They give no firmware or host-software version requirement.
  - They say "Trigger input and output are accessible through the clock connectors".
- **libhackrf board id:**
  - `hackrf.h` has `BOARD_ID_PRALINE = 5` and `HACKRF_PLATFORM_PRALINE (1 << 4)`.
  - `hackrf_board_id_name` returns "HackRF Pro" for `BOARD_ID_PRALINE`, and `hackrf_board_rev_name` lists revisions r0.1 to r1.2 (URL `host/libhackrf/src/hackrf.c`).
  - So the Pro's internal name is Praline.
  - `enum hackrf_usb_board_id` has no Praline entry.
  - `hackrf.c` reports only three USB product ids for the open functions: `hackrf_one_usb_pid = 0x6089`, jawbreaker `0x604b`, rad1o `0xcc15`. This is as reported by the fetcher.
  - INFERRED: the Pro enumerates with 0x6089. I did not read `usb_descriptor.c`.
- **Pro-only API.** These calls check `USB_API_REQUIRED(device, 0x0109)`, and their doc comments say "HackRF Pro".
  - `hackrf_set_p1_ctrl`, `hackrf_set_p2_ctrl`, `hackrf_set_clkin_ctrl`
  - `hackrf_set_narrowband_filter`
  - `hackrf_set_fpga_bitstream`
  - `hackrf_fpga_read_register` and `hackrf_fpga_write_register`
- **16-bit and 4-bit modes:**
  - `hackrf.h` documents no 4-bit or extended-precision mode, and `radio.c` and `fpga.c` have no comments on them.
  - The docs advertise them, but I found no libhackrf call for them.
  - Status: the existence of the modes is VERIFIED from the docs; how the host selects them is UNKNOWN.
- **Firmware and FPGA:** `firmware/hackrf_usb/usb_api_praline.c` exists, and `fpga.c` provides `fpga_set_trigger_enable`, `fpga_set_rx_decimation_ratio` and `fpga_set_rx_quarter_shift_mode`. The FPGA RTL was not read.

## G1 — device time counter

- **Answer:** No readable time counter exists on the One or the Pro. The nearest things are two byte counters.
  - `m0_count` counts bytes the M0 core has moved through SGPIO.
  - `m4_count` counts bytes the M4 core has moved over USB.
  - The counters are 32-bit. `hackrf_transfer` widens them to 64-bit in software and comments that they wrap "about every 107 seconds at 20 Msps". That matches bytes, not samples (2^32 / 40 MB/s ≈ 107 s).
  - They are read with `hackrf_get_m0_state` (USB API 0x0106).
  - They cannot be set or aligned to PPS. A trigger can start capture (see G3/G9). Nothing is "reset to 0 at stream start" in a documented way.
  - I did not see where firmware zeroes the counters at mode start.
- **Status:** VERIFIED that no timestamp API exists.
  - `hackrf.h` has no timestamp function.
  - `fpga.c` has no timestamp or counter.
  - `usb_api_transceiver.c` has no timestamp, only the byte counters.
  - Zeroing of the counters at mode start is INFERRED to be unknown.
- **Evidence:**
  - URL (`firmware/common/m0_state.h`): `struct m0_state` fields `requested_mode`, `active_mode`, `next_mode`, `m0_count`, `m4_count`, `num_shortfalls`, `longest_shortfall`, `shortfall_limit`, `threshold`, `error`. The file gives no units.
  - URL (`firmware/hackrf_usb/usb_api_transceiver.c`): `if ((m0_state.m0_count - usb_count) >= USB_TRANSFER_SIZE)` and `m0_state.m4_count += bytes_transferred;`.
  - URL (`host/hackrf-tools/src/hackrf_transfer.c`): `update_stats()` extends the counters to 64-bit.
  - The Pro has the same API and the same `m0_state` code path (`usb_api_transceiver.c` is shared). Nothing Pro-specific was found.

## G2 — per-sample time in an RX buffer

- **Answer:** No timestamp reaches the host with the RX data.
  - The `hackrf_transfer` struct carries `device`, `buffer`, `buffer_length`, `valid_length`, `rx_ctx` and `tx_ctx`.
  - The buffer is "interleaved 8 bit I/Q samples".
  - There is no timed start. The only start control is the hardware trigger (G3/G9), or starting at whatever moment the device switches to RX.
  - Without a trigger, the first sample's time can only be estimated from the host clock. The `hackrf_set_transceiver_mode` vendor request is sent over USB and the M0 starts at an unspecified later moment. No latency figure is stated in any source I read.
  - With the trigger, the docs claim a start synchronization error of "less than one sample period" between devices.
- **Status:** VERIFIED for the struct and the lack of timestamp. The latency of an untriggered start is UNKNOWN.
- **Evidence:**
  - URL (`host/libhackrf/src/hackrf.h`): `hackrf_transfer` struct and its buffer comment.
  - URL (`host/libhackrf/src/hackrf.c`): `start_rx` stores `rx_ctx`, calls `hackrf_set_transceiver_mode(device, HACKRF_TRANSCEIVER_MODE_RECEIVE)`, then `prepare_setup_transfers`. The RX buffer is `TRANSFER_BUFFER_SIZE 262144`.
  - https://hackrf.readthedocs.io/en/latest/hardware_triggering.html: "less than one sample period".
  - v3: `/home/komatsu/local_work/Ez-SDRv4/v3/source/device/hackrf.d:376-403` does not read any timestamp. The RX callback only pushes `transfer.buffer[0 .. valid_length]` into a queue.
- **Pro:** same as One unless stated. No source gives Pro-specific timestamp behaviour.

## G3 — timed TX, burst marks

- **Answer:**
  - TX has no start-time field and no burst start/end mark in the transfer.
  - TX starts after the host sends data. In firmware `tx_mode` the M0 starts streaming only when the 32 KiB buffer is full: `if (!started && (m0_state.m4_count == USB_BULK_BUFFER_SIZE)) { baseband_streaming_enable(...) }`.
  - `hackrf_transfer` can be used with hardware sync (`-H`). `main()` calls `hackrf_set_hw_sync_mode()` before the RX/TX branch, so sync applies to TX too (see G9).
  - End of burst: `hackrf_enable_tx_flush(device, callback, ctx)` gives a callback "when all the data was transmitted and all data transfers were completed". `hackrf_set_tx_block_complete_callback` is called per completed USB transfer.
  - Then `hackrf_stop_tx` ends the burst.
- **Status:** VERIFIED.
- **Evidence:**
  - URL (`host/libhackrf/src/hackrf.h`): the doc comments of `hackrf_enable_tx_flush` and `hackrf_set_tx_block_complete_callback`.
  - URL (`firmware/hackrf_usb/usb_api_transceiver.c`): the `tx_mode` code above.
  - v3: `hackrf.d:177-311` implements "burst" as a host-side queue plus `hackrf_start_tx` on the first buffer and `hackrf_stop_tx` at the end (`beginBurstTransmit`, `burstTransmit`, `endBurstTransmit`). It adds no device-side mark.

## G4 — timed retune or gain change

- **Answer:** None for tune or gain. All are immediate USB control transfers.
  - Setting frequency: `hackrf_set_freq` sends the MHz and Hz parts (`SET_FREQ` vendor request, `hackrf.c`). I found no time field.
  - Changing the baseband filter, LNA, VGA and amp are likewise single vendor requests.
  - No queue depth and no stated duration.
  - No source says what happens to samples while the device retunes.
- **Opera Cake only (not the radio):** `OPERACAKE_MODE_TIME` plus `hackrf_set_operacake_dwell_times` (`hackrf_operacake_dwell_time{uint32_t dwell; uint8_t port}`, up to 16 entries) switch antenna ports by time. The hackrf.h comment does not state the dwell unit.
- **Sweep mode:** `hackrf_init_sweep` / `hackrf_start_rx_sweep` retune across a list of ranges inside the firmware with a fixed capture per tune (`num_bytes` must be a multiple of `BYTES_PER_BLOCK` 16384). The sweep mode is a built-in sequence, not a timed command.
- **Status:** VERIFIED for "no timed API" (nothing in `hackrf.h`). "Duration of retune" is UNKNOWN (no source states it).
- **Evidence:** URL (`host/libhackrf/src/hackrf.h`) declarations, listed above.

## G6 — RX loss report

- **Answer:** Loss is counted in the device, in bytes, and is not placed in time.
  - M0 shortfall handling in `sgpio_m0.s`: "In an RX shortfall, data is discarded. In TX mode, zeroes are written to SGPIO."
  - Counters: `NUM_SHORTFALLS` goes up by one when a shortfall starts. `LONGEST_SHORTFALL` is updated when the current one exceeds it. "Each missed exchange extends the current shortfall length by 32 bytes."
  - The host reads `num_shortfalls` and `longest_shortfall` with `hackrf_get_m0_state` (USB API 0x0106). They are cumulative since the last mode start (INFERRED, not confirmed; the rollback comment suggests they are reset at the next start).
  - Total bytes lost cannot be read exactly. Only the number of loss events and the longest single loss are available. The sum of all lost bytes is not a field.
  - "When in a shortfall, the byte counter does not advance." So `m0_count` keeps counting delivered bytes only. A discarded stretch leaves no gap in `m0_count`, which means the host cannot place the loss in the stream from the counters.
  - The M0 can stop on a limit: `hackrf_set_rx_overrun_limit(value)` (bytes, "not whole IQ pairs") makes the device return to IDLE after that many missing bytes. Default 0 means no limit. The M0 then sets `error` (1 = RX_TIMEOUT, 2 = TX_TIMEOUT, 3 = MISSED_DEADLINE).
  - Host-side: `-S` in `hackrf_transfer` uses a ring buffer and counts `stream_drop` (host queue overflow). The libhackrf RX path itself has no report: the callback returns 0 or non-zero and libusb transfers complete or not.
  - Nothing in the USB transfer reports a loss. The host can only poll the M0 state.
- **Status:** VERIFIED. The exact reset of the counters is INFERRED.
- **Evidence:**
  - URL (`firmware/hackrf_usb/sgpio_m0.s`): the shortfall comments and counters.
  - URL (`host/libhackrf/src/hackrf.h`): `hackrf_m0_state` struct and `hackrf_set_rx_overrun_limit` comment.
  - URL (`host/hackrf-tools/src/hackrf_transfer.c`): `-B` prints "underruns" for TX and "overruns" for RX from `num_shortfalls` / `longest_shortfall`.
  - https://hackrf.readthedocs.io/en/latest/synchronization_checklist.html: "After each run, check each device with `hackrf_debug -S`. A non-zero shortfall count means samples were dropped or late."
  - v3: `hackrf.d:376-403` has no loss accounting. The RX callback drops data into a host queue and prints a message when the queue is full.

## G7 — TX underflow report

- **Answer:** Same mechanism as G6, mirrored.
  - On a TX shortfall the M0 writes zeros to SGPIO (so zeros are transmitted).
  - Counters: the same `num_shortfalls` and `longest_shortfall`.
  - There is no asynchronous event. The host only learns by polling `hackrf_get_m0_state`, or by the device auto-stopping when the limit set by `hackrf_set_tx_underrun_limit` is reached.
  - While in TX_START mode zeros are not counted ("If in TX start mode, don't count this as a shortfall").
- **Host-side behaviour that v3 hit:** the TX callback must return filled data. v3 comment at `hackrf.d:294-296`: "これをしないと，HackRFが今後一切callbackを呼ばなくなるみたい" (if the buffer is not filled, HackRF seems never to call the callback again). v3 therefore zero-fills the buffer when the queue is empty.
- **Status:** VERIFIED for the counters and the zero-fill. The v3 comment is a developer observation, not documentation (INFERRED).
- **Evidence:** URL (`firmware/hackrf_usb/sgpio_m0.s`), URL (`host/libhackrf/src/hackrf.h` for the underrun limit comment), `v3/source/device/hackrf.d:291-298`.

## G8 — external frequency reference

- **Answer:** A 10 MHz CLKIN exists, and presence can be read.
  - `hackrf_get_clkin_status(device, uint8_t* status)` returns "external clock detected (0/1)" (USB API 0x0106).
  - Firmware picks the clock automatically: `activate_best_clock_source()` calls `si5351c_clkin_signal_valid(&clock_gen)`. If valid it uses `CLOCK_SOURCE_EXTERNAL`. On One and Pro it may then try the PortaPack oscillator, and otherwise falls back to the on-board crystal (the Pro has a TCXO per the docs).
  - It is activated at transceiver startup (`transceiver_startup` ends with `activate_best_clock_source()`), so an input that appears or vanishes during a stream is not re-evaluated by that code (INFERRED, from where the call is made).
  - No lock-lost event exists. `hackrf_get_clkin_status` is a poll of the Si5351C input-valid flag.
  - Expected input: 10 MHz square wave, 0 V to 3.0-3.3 V. Docs say a sine wave works but may add phase noise.
  - CLKOUT: `hackrf_set_clkout_enable(device, uint8_t value)` (USB API 0x0103).
  - Pro: `hackrf_set_clkin_ctrl` selects the CLKIN signal (`CLKIN_SIGNAL_P1 = 0`, `CLKIN_SIGNAL_P22 = 1`), and P1/P2 can carry `P1_SIGNAL_CLKIN`, `P1_SIGNAL_AUX_CLK1/2`, `P2_SIGNAL_CLK3`.
- **Status:** VERIFIED.
- **Evidence:**
  - URL (`firmware/common/hackrf_core.c`): `activate_best_clock_source`.
  - URL (`host/libhackrf/src/hackrf.h`): the declarations above.
  - https://hackrf.readthedocs.io/en/latest/synchronization_checklist.html.
  - v3: `hackrf.d:128-132` uses `hackrf_set_clkout_enable`. It does not use CLKIN status.

## G9 — multi-device alignment

- **Answer:** Two mechanisms, clock and trigger. They are independent.
  - Clock sharing: one device's CLKOUT to the others' CLKIN (10 MHz). `hackrf_clock -o` enables CLKOUT, and `hackrf_clock -i` checks CLKIN (host software 2022.09.1 or later).
  - Trigger (`hackrf_set_hw_sync_mode(device, value)`, USB API 0x0102):
    - The firmware handler only stores the flag (`endpoint->setup.value != 0`).
    - At `transceiver_startup` the firmware calls `trigger_enable(radio_get_trigger_enable(&radio, RADIO_CHANNEL0))`.
    - On One that writes the GPIO `gpio_trigger_enable` (`hackrf_core.c`: `gpio_write(sgpio_config.gpio_trigger_enable, enable);`).
    - On Pro it writes the FPGA register (`fpga_set_trigger_enable` → `set_FPGA_STANDARD_CTRL_TRIGGER_EN`).
    - The trigger gate is then done in the CPLD (One) or FPGA (Pro). I did not read the CPLD/FPGA source, so the exact gating logic is INFERRED.
  - Observable behaviour (docs, VERIFIED): the device in sync mode sits in "Waiting for trigger..." until it sees a pulse on its trigger input; the trigger fires on the rising edge of a 3.3 V pulse. The triggering device needs no special setting, since "No special argument is needed to enable the trigger output". Several devices can be triggered from one output.
  - One uses header P28 inside the case: pin 15 = trigger output, pin 16 = trigger input, pin 2 = ground.
  - Pro: trigger in/out are on the SMA ports P1/P2 (`P1_SIGNAL_TRIGGER_IN = 0`, `P1_SIGNAL_TRIGGER_OUT = 3`, `P2_SIGNAL_TRIGGER_IN = 2`, `P2_SIGNAL_TRIGGER_OUT = 3`), selected with `hackrf_set_p1_ctrl` and `hackrf_set_p2_ctrl`.
  - Level or edge: the upstream docs say rising edge. The HTime fork (https://pyhackrf2-htime.readthedocs.io/en/latest/pyhackrf2_htime.html, a third-party wrapper) says "A/D sampling starts on the trigger hardware signal, and the trigger is a level signal that must be held for the whole sampling period". This is INFERRED for upstream and unresolved between the two.
  - Applies to RX and TX: `hackrf_transfer` calls `hackrf_set_hw_sync_mode()` before the RX/TX branch (VERIFIED), but the docs show only RX examples.
  - Accuracy: "less than one sample period" (docs, no measurement quoted).
  - Dropped RX samples or late TX samples put signals out of sync afterwards (sync checklist), so alignment after the start relies on no shortfall.
- **Status:** VERIFIED for the API and the doc claims; the gating internals and level/edge are INFERRED.
- **Evidence:**
  - https://hackrf.readthedocs.io/en/latest/hardware_triggering.html
  - https://hackrf.readthedocs.io/en/latest/synchronization_checklist.html
  - URL (`firmware/hackrf_usb/usb_api_transceiver.c`): `usb_vendor_request_set_hw_sync_mode`, `transceiver_startup`.
  - URL (`host/hackrf-tools/src/hackrf_transfer.c`): the `-H` handling.
  - v3: `hackrf.d:90-95` uses `hackrf_set_hw_sync_mode` (RX or TX per the `mode` field).

## G10 — half-duplex switching

- **Answer:** HackRF is half-duplex (docs for both boards). One transceiver mode is active at a time.
  - libhackrf: `hackrf_start_rx` / `hackrf_start_tx` call `hackrf_set_transceiver_mode` (`RECEIVE = 1`, `TRANSMIT = 2`, `OFF = 0`), then set up USB transfers.
  - `hackrf_stop_rx` / `hackrf_stop_tx` call `cancel_transfers` first, and return early on error without sending OFF. Otherwise they send `hackrf_stop_cmd` (mode OFF).
  - Firmware: `request_transceiver_mode` flushes both bulk endpoints, stores the mode and increments `seq`. The running `rx_mode` or `tx_mode` loop sees the changed seq, calls `transceiver_shutdown()`, and the main loop dispatches the next mode.
  - The code does not require OFF between RX and TX (VERIFIED for `request_transceiver_mode` and the vendor request, which accepts any of OFF, RX, TX, RX_SWEEP, CPLD_UPDATE at any time). Whether direct RX to TX works in practice is not stated, and `hackrf_start_tx` is not documented as requiring a prior stop.
  - Time for a switch: not stated in any source I read. `radio_switch_mode` re-applies sample rate, frequency, filter (non-Praline), amp, gains and bias tee on every mode start, so a switch includes reprogramming the tuning chips (VERIFIED from `radio.c`; duration UNKNOWN).
  - `transceiver_shutdown` clears the TX path, puts the MAX283x in standby and sets `M0_MODE_IDLE`. TX is not left on after OFF.
  - Both a stop and a start are needed. v3 does this: `stopContinuousReceive` calls `hackrf_stop_rx`, and `endBurstTransmit` calls `hackrf_stop_tx`. v3 sets one mode per opened device (`HackRFMode.TX` or `RX` from config), so it never switches a device (`hackrf.d:68`).
  - libhackrf keeps one callback and one stream per device. `hackrf_transfer` has both `rx_ctx` and `tx_ctx` in the same struct but only one is used per start.
- **Status:** VERIFIED for the call sequence; switching time UNKNOWN.
- **Evidence:** URL (`host/libhackrf/src/hackrf.c`): `hackrf_start_rx`, `hackrf_start_tx`, `hackrf_stop_rx`, `hackrf_stop_tx`, `hackrf_set_transceiver_mode`. URL (`firmware/hackrf_usb/usb_api_transceiver.c`): `request_transceiver_mode`, `transceiver_shutdown`. URL (`firmware/common/radio.c`): `radio_switch_mode`.

## G11 — channels and what is shared

- **Answer:** One RX channel and one TX channel in one transceiver. The firmware calls the single channel `RADIO_CHANNEL0`. There is no multi-channel API.
  - Because the device is half-duplex, RX and TX share the one sample rate. `radio_set_sample_rate` stores one clock-generator rate (`config->sample_rate[RADIO_SAMPLE_RATE_CLOCKGEN]`).
  - Shared: frequency (one LO/IF path), sample rate, baseband filter, bias tee (`antenna[RADIO_ANTENNA_BIAS_TEE]`).
  - Separate by direction: LNA and VGA gain for RX, TX VGA gain for TX, RF amp on/off for either direction.
  - On Pro, `radio_switch_mode` re-applies RX or TX gains for the mode being started.
- **Status:** VERIFIED (`radio.c`, `hackrf.h`).
- **Evidence:** URL (`firmware/common/radio.c`): `radio_switch_mode`, the `RADIO_CHANNEL0` constant used in `usb_api_transceiver.c`. v3: `numChannelImpl() ... return 1` at `hackrf.d:188` and `hackrf.d:325`.

## G12 — sample rate

- **Answer:**
  - Documented range: `hackrf_set_sample_rate` comment "Sample rate should be in the range 2-20MHz, with the default being 10MHz. Lower & higher values are technically possible, but the performance is not guaranteed".
  - The docs say rates below 8 MHz are "not recommended", because the MAX5864 is not specified below 8 MHz (https://hackrf.readthedocs.io/en/latest/sampling_rate.html).
  - Fractional method, host side (`hackrf.c`): `hackrf_set_sample_rate(double freq)` searches a divider i in 1..31 (`MAX_N = 32`), finds the first i for which the fractional part of `freq*i` fits the mask test `if (!(a & m) || !(~a & m))`, falls back to `i = 1`, then computes `freq_hz = (uint32_t)(freq * i + 0.5)` and `divider = i`. It calls `hackrf_set_sample_rate_manual(freq_hz, divider)`, so the effective rate is `freq_hz / divider`.
  - `hackrf_set_sample_rate_manual` sends `freq_hz` and `divider` in the `SAMPLE_RATE_SET` vendor request. Header: "@param divider frequency divider. Must be in the range 1-31".
  - Firmware (`usb_api_transceiver.c`): `.num = set_sample_r_params.freq_hz * 2`, `.div = set_sample_r_params.divider`.
  - Rounding and read-back (`radio.c`): `sample_rate.hz = (sample_rate.num + (sample_rate.div >> 1)) / sample_rate.div` rounds to integer Hz. libhackrf has no read-back of the applied sample rate that I found. The firmware does read back the applied baseband filter (`radio_get_filter`) and updates the UI with it, but that is not sent to the host.
  - Rate-dependent filter: `hackrf_set_sample_rate_manual` on success sets the baseband filter to `(uint32_t)(0.75 * freq_hz / divider)`. The header says this function also sets the baseband filter, so any `hackrf_set_baseband_filter_bandwidth` call "should only be made after this".
  - Baseband filter allowed values (header): "1.75, 2.5, 3.5, 5, 5.5, 6, 7, 8, 9, 10, 12, 14, 15, 20, 24, 28MHz". `hackrf_compute_baseband_filter_bw` returns the largest table entry at or below the request, and 1.75 MHz for anything lower. The set call itself sends the value as given (`bandwidth_hz & 0xffff` and `>> 16`).
  - Pro: `radio.c` for PRALINE has `#define MAX_AFE_RATE 40000000` and `#define MAX_N 5`. It raises the resampling exponent n so the AFE rate stays within the limit, then calls `fpga_set_rx_decimation_ratio` ("resampling ratio is 2**n"). The filter is derived from the rate: `(sample_rate * 3) / 8` plus an offset term when shifting. The Pro's allowed rate range (beyond "up to 20 Msps" and 4-bit "up to 40 Msps" in the docs) is not given in the sources I read. `sample_rate_frac_set()` range checks were not seen.
  - v3: `hackrf.d:79` calls `hackrf_set_sample_rate(_device, rate)` with a double, then `hackrf_set_baseband_filter_bandwidth` after it (`hackrf.d:84-88`), matching the header's ordering requirement.
- **Status:** VERIFIED for One; Pro range UNKNOWN beyond the docs.
- **Evidence:** URL (`host/libhackrf/src/hackrf.c`), URL (`host/libhackrf/src/hackrf.h`), URL (`firmware/common/radio.c`), https://hackrf.readthedocs.io/en/latest/sampling_rate.html.

## G13 — frequency and gain

- **Frequency.**
  - Header: `hackrf_set_freq` "center frequency in Hz. Defaults to 900MHz. Should be in range 1-6000MHz, but 0-7250MHz is possible." Resolution is in whole Hz at the API (`uint64_t freq_hz`, sent as MHz plus Hz).
  - `hackrf_set_freq_explicit` takes an IF and an LO. Header: "IF ... Must be in the range of 2150-2750MHz", "LO ... Must be in the range 84.375-5400MHz, defaults to 1000MHz".
  - The code check in `hackrf.c` differs from the header: `if (if_freq_hz < 2000000000 || if_freq_hz > 3000000000)` and `if (path > 2)`. The header and the code disagree, as reported.
  - The actual tuning resolution of the LO and rounding are not stated in libhackrf. They depend on the RFFC5071/5072 mixer and MAX283x (One). I did not read `tuning` code.
  - Pro: docs give 100 kHz to 6 GHz operating, 0 Hz to 7.1 GHz tunable. `radio.c` for PRALINE uses a tune-config table: a frequency is looked up by `rf_range_end_mhz`, returns `RADIO_ERR_INVALID_PARAM` if no row matches, and a frequency of 0 matches the first row. The table itself is in another file that was not read.
  - Pro uses offset tuning, so the sample rate affects the tuning configuration (`radio_set_sample_rate()` calls `radio_set_frequency()`). The tuning also affects the baseband filter bandwidth. `radio.c` comments: "Because of offset tuning on Praline, the sample rate can affect the tuning configuration" and "the tuning configuration can affect the baseband filter bandwidth".
  - Also Pro: `fpga_set_rx_quarter_shift_mode` applies a quarter-sample-rate shift. Explicit tuning sets quarter-shift NONE.
  - Frequency range depends on direction only through the Pro tune tables (`praline_tune_config_rx`, `_rx_sweep`, `_tx`). Their contents were not read.
- **Gain** (from `hackrf.h` and `hackrf_transfer.c`):

| Stage | API | Range and step |
|---|---|---|
| RX LNA (IF) | `hackrf_set_lna_gain` | 0-40 dB, 8 dB steps |
| RX VGA (baseband) | `hackrf_set_vga_gain` | 0-62 dB, 2 dB steps |
| TX VGA | `hackrf_set_txvga_gain` | 0-47 dB, 1 dB steps |
| RF amp | `hackrf_set_amp_enable` | on/off; header title says "14dB", body says "~11dB" (inconsistent) |

  - `hackrf_transfer` warns on non-multiples but "does not enforce the gain ranges". The firmware treats a returned `real_db` of 0 from the MAX283x/2831 call as an error (`radio.c`).
  - The amp is one switch, shared by RX and TX paths (`RADIO_GAIN_RF_AMP`).
  - Whether the sum is a useful single number is not stated by any source.
  - Gain does not depend on frequency in any source I read. The Pro's MAX2831 gain ranges are not given in what I read.
- **Status:** VERIFIED for the header values; Pro frequency table and tuning resolution UNKNOWN.
- v3: `hackrf.d:98-125` uses `hackrf_set_freq`, `hackrf_set_amp_enable`, and `hackrf_set_txvga_gain` (TX) or `hackrf_set_lna_gain` (RX) as the single "gain". It never sets the VGA.

## G14 — wire format and throughput

- **Answer:**
  - Format: 8-bit signed I and Q, interleaved ("interleaved 8 bit I/Q samples", `hackrf.h`). That is 2 bytes per complex sample. v3 treats it as `ComplexInt!byte` (`hackrf.d:146-148`, element type `ComplexInt8`).
  - Pro: the same 8-bit format by default. Extended 16-bit and half-precision 4-bit modes are advertised, with no libhackrf call found.
  - Host link: USB 2.0 High Speed (480 Mbit/s). At 20 Msps the stream is 40 MB/s. The sync checklist says "At 20 MHz, one device uses nearly all of a USB 2.0 bus unless you run very low sample rates", so each device should get its own USB bus.
  - Buffering: host `TRANSFER_COUNT 4` × `TRANSFER_BUFFER_SIZE 262144` bytes. Device `USB_BULK_BUFFER_SIZE 0x8000` (32768 bytes ≈ 0.82 ms at 20 Msps, by arithmetic from 40 MB/s). `hackrf_get_transfer_buffer_size` returns the transfer size in bytes, and `hackrf_get_transfer_queue_depth` the queue depth.
  - No effective measured throughput is given in the sources I read.
- **Status:** VERIFIED for format and sizes; the 0.82 ms is my arithmetic.
- **Evidence:** URL (`host/libhackrf/src/hackrf.h`), URL (`host/libhackrf/src/hackrf.c`), URL (`firmware/hackrf_usb/usb_bulk_buffer.h`), https://hackrf.readthedocs.io/en/latest/synchronization_checklist.html, https://hackrf.readthedocs.io/en/latest/hackrf_one.html ("8-bit", "High Speed").

## G15 — phase

- **Answer:** No source states anything about phase after retune, LO sharing, or phase alignment for HackRF.
  - Structure: one channel, one LO path (G11), so "do channels share an LO" does not arise for one device.
  - Phase relation after a retune is not documented. `radio_switch_mode` reapplies the frequency on every mode start (VERIFIED), which implies the phase need not persist across a start (INFERRED).
  - Across two devices sharing a CLKIN, phase relation is not stated.
- **Status:** UNKNOWN for the behaviour. Searches below.

## G16 — path delay

- **Answer:** No delay figure for RX or TX appears in any source I read. Only the data path structure is known: an M0 copy through SGPIO and a 32 KiB device buffer (≈0.82 ms at 20 Msps by arithmetic). On the Pro, decimation by 2^n in the FPGA adds a filter delay of unknown size.
- **Status:** UNKNOWN. Searches below.

## G17 — hardware repeat TX

- **Answer:** None. libhackrf has no repeat function in `hackrf.h`. `hackrf_transfer -R` is a host-side repeat of the TX file ("repeats the TX file to fill buffers when it runs short"). v3's `LoopByBurst` mixin does the repeat on the host (`hackrf.d:258`).
- **Status:** VERIFIED (no such function in `hackrf.h`, searched).

## G18 — safety: TX power, antenna power, amp

- **TX power.**
  - One (https://hackrf.readthedocs.io/en/latest/hackrf_one.html), maximum TX power by band: 1-10 MHz 5 to 15 dBm (rises with frequency); 10-2170 MHz 5 to 15 dBm (falls with frequency); 2170-2740 MHz 13 to 15 dBm; 2740-4000 MHz 0 to 5 dBm; 4000-6000 MHz -10 to 0 dBm.
  - Maximum RX input: -5 dBm; the page says up to 10 dBm may be tolerated with the RX amp off.
  - Pro: no TX power figure in the docs. Pro has a "Hardware option to disable transmit mode" and "enhanced RF port protection".
- **Antenna port power (bias tee).**
  - `hackrf_set_antenna_enable`: "Enable or disable the 3.3V (max 50mA) bias-tee (antenna port power). Defaults to disabled." The header also says "the firmware auto-disables this after returning to IDLE mode, so a perma-set is not possible". Other sources disagree with the header:
    - `rf_path.c`: the OFF path "never touches `SWITCHCTRL_ANT_PWR`, so antenna/bias state carries through OFF unchanged".
    - `radio.c`: "At mode OFF, the value is only stored".
    - `radio_switch_mode` re-applies the stored bias-tee setting at each RX/TX start.
  - So the header comment and the code reading differ. Status: INFERRED that the setting is kept in the radio config and applied at each start. Not tested on a device.
  - Safe initial state: bias tee off (`SWITCHCTRL_SAFE`, `.antenna[RADIO_ANTENNA_BIAS_TEE] = {.enable = false}`).
  - v3 uses it: `hackrf.d:109-113` (`hackrf_set_antenna_enable`, config key `useANTPwr`).
- **RF amp:** on/off (G13). Reset by `rf_path_set_direction(OFF)` (`rf_path_set_lna(rf_path, 0)`: amp-bypass bit set and both amp-power-off bits set). `radio_switch_mode` re-applies the stored amp setting on each start.
- **Pro-specific settings that are not on One:** P1 and P2 signal selection, CLKIN selection, narrowband filter on/off, FPGA bitstream selection. `hackrf_set_p1_ctrl` can select `TRIGGER_OUT`, `AUX_CLK1/2`, etc. as outputs from an SMA port (VERIFIED enums; their electrical effect on safety is not stated).
- **Status:** VERIFIED for ranges, bias tee limits and the One's power table; the idle behaviour of bias tee is a header-versus-code conflict (INFERRED).

## G19 — device configuration description

- **Answer:** What distinguishes a unit, from the API:
  - board id: `BOARD_ID_JELLYBEAN/JAWBREAKER/HACKRF1_OG/RAD1O/HACKRF1_R9/PRALINE` with `hackrf_board_id_read`, `hackrf_board_id_name`, `hackrf_board_rev_read`/`_name`, `hackrf_supported_platform_read` (USB API 0x0106);
  - firmware version and USB API version (`hackrf_usb_api_version_read`, "MM.mm" 16-bit);
  - serial (`hackrf_open_by_serial`, used by v3 `hackrf.d:74`);
  - part id and board revision (r9 versus earlier changes the bias-tee and clock-detect paths in firmware, G8/G18).
- Expansion: Opera Cake boards (`hackrf_get_operacake_boards`, up to `HACKRF_OPERACAKE_MAX_BOARDS 8`, ports PA1-PA4 and PB1-PB4), PortaPack as an alternate clock source (`CLOCK_SOURCE_PORTAPACK`, G8).
- Pro-only: FPGA bitstream index (`hackrf_set_fpga_bitstream`), P1/P2 and CLKIN selections, narrowband filter.
- One and Pro differ in code paths on: sample-rate/decimation (Pro FPGA), filter source (Pro derives it from the rate), tune (table on Pro), trigger (CPLD GPIO vs FPGA register).
- **Status:** VERIFIED (API list); what an SDR runtime needs is not judged here.

## G20 — timing envelope numbers

- **Command lead:** no timed commands, so none (G4).
- **Startup delay:** not stated in any source. RX: after `hackrf_set_transceiver_mode` returns, `rx_mode` schedules USB reads once `m0_count - usb_count >= USB_TRANSFER_SIZE` (the first transfer needs that many bytes: `USB_TRANSFER_SIZE` value was not retrieved). TX: streaming starts only when the 32768-byte buffer is full (`m4_count == USB_BULK_BUFFER_SIZE`); at 20 Msps that is 16384 samples (≈0.82 ms, my arithmetic).
- **Queue depth:** host `TRANSFER_COUNT 4` × 262144 bytes; device ring 32768 bytes. At 2 bytes per sample, one host transfer is 131072 samples (6.55 ms at 20 Msps, my arithmetic).
- **Release window:** none.
- **Overflow resume gap:** none defined. An RX shortfall discards data and the M0 resumes when the SGPIO exchange interrupt is serviced ("Each missed exchange extends the current shortfall length by 32 bytes"). Per-direction start lead: not stated.
- **Status:** VERIFIED for the sizes; delays UNKNOWN.

## G21 — disconnect

- **Answer:** libhackrf has no disconnect event.
  - `hackrf_libusb_transfer_callback` handles RX and TX in one function. It sets `success = (status == LIBUSB_TRANSFER_COMPLETED)`. On any other status it sets `streaming = false`. `LIBUSB_TRANSFER_NO_DEVICE` is not named in `hackrf.c`, so a disconnect takes the same path as any failed transfer.
  - The event loop: `if ((error != 0) && (error != LIBUSB_ERROR_INTERRUPTED))` sets `streaming = false`.
  - The user callback is invoked only if `device->streaming`. After failure it is no longer called and the transfer is not resubmitted.
  - The host detects it by polling `hackrf_is_streaming(device)`, which returns `HACKRF_TRUE` only while streaming, otherwise `HACKRF_ERROR_STREAMING_THREAD_ERR` (-1002), `HACKRF_ERROR_STREAMING_STOPPED` (-1003) or `HACKRF_ERROR_STREAMING_EXIT_CALLED` (-1004).
  - Control calls fail with `HACKRF_ERROR_LIBUSB` (-1000), `HACKRF_ERROR_NOT_FOUND` (-5) or `HACKRF_ERROR_BUSY` (-6); the full enum is above. Short reads of state structs give `HACKRF_ERROR_LIBUSB`.
  - `hackrf_transfer` has no explicit disconnect message. It exits when no bytes transfer for one second ("Couldn't transfer any bytes for one second."), or when `hackrf_is_streaming()` stops being true (it prints "Exiting... hackrf_is_streaming() result" and the code).
  - `hackrf_stop_rx` / `hackrf_stop_tx` after a failed stream: `cancel_transfers` returns `HACKRF_ERROR_OTHER` when transfers are not set up, and the stop returns early.
  - A non-disconnect device stop exists: the underrun/overrun limit makes the device go IDLE. The host sees that as the stream no longer producing data and no event (the M0 `error` field would show it, polled).
  - v3 does not detect a disconnect: it prints on a failed `hackrf_start_*` / `hackrf_stop_*` and keeps going (`hackrf.d:344-346`, `hackrf.d:356-358`).
- **Status:** VERIFIED for the libhackrf behaviour from the fetcher's reading of `hackrf.c`; I did not read the file line by line.
- **Evidence:** URL (`host/libhackrf/src/hackrf.c`): `hackrf_libusb_transfer_callback`, `hackrf_is_streaming`; URL (`host/libhackrf/src/hackrf.h`): `enum hackrf_error`; URL (`host/hackrf-tools/src/hackrf_transfer.c`).

## G22 — one root per device

- **Answer:** One sample clock per device. The Si5351C clock generator makes the sample clock and converter clock (`sgpio` takes an external clock on SGPIO8: "SGPIO8 Clock Input (External Clock)", and `radio_set_sample_rate` has only the `RADIO_SAMPLE_RATE_CLOCKGEN` element). The M0 and M4 cores are separate processors, but both counters (G1) count bytes of the same stream.
  - Pro: the FPGA resamples RX by 2^n after the converter (`MAX_AFE_RATE 40000000`), so the host rate and the converter rate differ by a power of two, both derived from the same clock.
  - Nothing in the sources suggests parts running on separate clock domains visible to the host.
- **Status:** INFERRED from the clock structure in `sgpio.c` and `radio.c`; no source states it as a design fact.

## Searches that found nothing

- WebFetch `https://raw.githubusercontent.com/greatscottgadgets/hackrf/v2026.01.3/firmware/hackrf_usb/usb_api_m0_state.c`: it only copies `m0_state` to the host; field definitions are in `firmware/common/m0_state.h`, where the file gives no units. The byte unit is INFERRED from `hackrf_transfer`'s 107 s comment.
- WebFetch `https://hackrf.readthedocs.io/en/latest/external_clock_interfacing.html`: HTTP 404.
- WebSearch "hackrf firmware trigger_enable hw_sync_mode trigger rx start waits for trigger input pin CPLD sgpio source code": no CPLD or FPGA source. The trigger gating internals were not found. The only description of level-sensitive behaviour is from the HTime fork's wrapper (third party).
- `firmware/common/sgpio.c`: contains no `hw sync`, `capture_start` or `baseband_streaming_enable`. The stream gate is SGPIO10 (clear = enable) and the capture qualifier is SGPIO9.
- `firmware/hackrf_usb/usb_api_praline.c`: the FPGA register requests and any clock-detect logic are not in it, and helper functions such as `p1_ctrl_set` are only in `hackrf_core.c`.
- `firmware/common/radio.c`: no Praline tune table, no MAX2831 gain ranges and no sample-rate range check (`sample_rate_frac_set()` was not read). Neither 2 MHz nor 20 MHz limits appear there.
- `docs/source/hackrf_pro.rst` and the Pro page: no P1/P2 mention, no TX power, no firmware version, no 16-bit/4-bit programming interface. `hackrf.h` has no 4-bit or extended-precision function.
- WebFetch of `hackrf_pro.html` for a TX power table: not given.
- WebSearch "HackRF Pro Great Scott Gadgets announcement specifications": returned retailer and press pages only; GSG's own announcement page was not fetched.
- G15 (phase after retune, LO sharing, timed-tune phase), G16 (path delay): no source mentions either. I did not search for forum measurements.
- Not read at all: the CPLD and FPGA sources, `firmware/common/tuning*.c` (LO resolution and rounding), the MAX283x / MAX2831 drivers (gain tables), `usb_descriptor.c` (Pro USB PID), `USB_TRANSFER_SIZE`'s value, and whether `m0_count` and `m4_count` are zeroed at mode start.

Not checked beyond the above: I did not run any code on a device, so every behaviour reported as "not stated" may exist in untested hardware.
