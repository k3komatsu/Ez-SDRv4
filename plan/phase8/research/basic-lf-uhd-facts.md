# UHD v4.10.0.0: Basic/LF daughterboards on X300/X310

Fact-finding for Phase 8 stage 1b-2 (sonnet-researcher, 2026-10-17, read-only, from UHD source on GitHub). Not run on hardware. VERIFIED = read in source; INFERRED = deduced from the quoted code.

**Line numbers.** `db_basic_and_lf.cpp` numbers are exact. `db_basic_and_lf.hpp` numbers may be off by one. Numbers marked "~" in other files are estimates; the function name is the anchor.

Base URL: `https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/`

- DB-CPP = [dboard/db_basic_and_lf.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp)
- DB-HPP = [dboard/db_basic_and_lf.hpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.hpp)
- XRC = [x300/x300_radio_control.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_radio_control.cpp)
- MUR = [multi_usrp_rfnoc.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp_rfnoc.cpp)
- MU = [multi_usrp.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/multi_usrp.cpp)

## 1. Frequency range, tuning and bandwidth

- VERIFIED: `BASIC_MAX_BANDWIDTH = 250e6`, `LF_MAX_BANDWIDTH = 32e6` (DB-HPP ~L23-24). The LF constant is 32 MHz, not the documented 30 MHz.
- VERIFIED: `antenna_mode_bandwidth_scalar` is A=1, B=1, AB=2, BA=2 (DB-HPP ~L29-30).
- VERIFIED: `freq/range` is `(-max_freq, +max_freq)` ([DB-CPP L128-130](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L128-L130) RX, [L225-227](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L225-L227) TX): BasicRX/BasicTX ±250 MHz, LFRX/LFTX ±32 MHz, every frontend.
- VERIFIED: `freq/value` has only a publisher returning 0.0 ([L125-127](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L125-L127), TX [L222-224](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L222-L224)): no LO, no coercer, no `los`, `lo_locked` or `tune_args`.
- VERIFIED: X300 `set_rx_frequency`/`set_tx_frequency` write `freq/value` and return the read-back (XRC ~L329-361), so the RF frequency is 0.
- VERIFIED: `tune_xx_subdev_and_dsp` (MU ~L115-240), AUTO policy: `target_rf = clipped_requested + lo_offset` (lo_offset 0, `use_lo_offset` false), `actual_rf` read back (0), `target_dsp = (actual_rf - clipped_requested) * sign`, set on the DDC/DUC; `get_rx_freq = actual_rf - actual_dsp * sign`. `multi_usrp_rfnoc.cpp` has the same structure, with `make_overall_tune_range` when a DDC exists (body read from a summary only).
- VERIFIED: clip range `make_overall_tune_range(fe_range, dsp_range, bw)` ([multi_usrp_utils.hpp L23-34](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/include/uhdlib/usrp/multi_usrp_utils.hpp#L23-L34)): start = `max(0, fe.start + max(dsp.start, -bw/2))`, stop = `fe.stop + min(dsp.stop, bw/2)`.
- VERIFIED: DDC range is `±input_rate/2` ([ddc_block_control.cpp L130-134](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/rfnoc/ddc_block_control.cpp#L130-L134)), ±100 MHz at 200 MHz.
- INFERRED (arithmetic from the formula, DDC ±100 MHz): overall `get_rx_freq_range` is 0…350 MHz for BasicRX (AB/BA or A/B), 0…64 MHz for LFRX AB/BA, 0…48 MHz for LFRX A/B. Whether the DDC accepts values beyond ±100 MHz was not traced; treat the 350 MHz end as unconfirmed.
- INFERRED: the DDC does all the tuning (`set_rx_freq(f)` leaves RF at 0, DDC at `-f`); the same for TX with `TX_SIGN = -1`.
- VERIFIED: `bandwidth/value` and `bandwidth/range` (min = max) are `scalar[ant] * max_freq` ([L143-149](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L143-L149), TX [L240-246](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L240-L246)); default mode AB: Basic 500 MHz (A/B 250), LF 64 MHz (A/B 32). An antenna change rewrites bandwidth and `connection` ([L150-169](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L150-L169), TX [L247-266](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L247-L266)). The reported bandwidth is not the analog bandwidth.

## 2. Gain

- VERIFIED: both boards create `gains` as a phony node with no children ([RX L124](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L124), [TX L221](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L221)): no gain stages.
- VERIFIED: XRC `_init_codecs` (~L1280-1310): `rx_codec/gains/digital/range` = (0, 6.0, 0.5), one node per radio (shared by the slot); `tx_codec/gains` empty (no DAC gain).
- VERIFIED: XRC `_init_dboards` (~L1660-1700): the RX gain group holds `ADC-digital` (low priority) plus the dboard's stages (none here); TX builds a group with no stages.
- INFERRED: `get_rx_gain_range()` = 0…6 dB step 0.5 (`ADC-digital` only); `get_tx_gain_range()` = 0…0 (`gain_group` not read); RX names `["ADC-digital"]`, TX none.

## 3. Antenna

- VERIFIED: `antenna/options` = keys of `antenna_mode_to_conn` (A, AB, B, BA; order unverified), `antenna/value` default `"AB"` ([RX L131-136](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L131-L136), [TX L228-233](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L228-L233)); map AB→IQ, BA→QI, A→I, B→Q. Same for RX "0", RX "1", TX "0".
- VERIFIED (approximate lines): XRC `set_rx_antenna` refuses a name not in the map and calls `set_fe_connection(conn, 0.0)`; `set_tx_antenna` calls `set_mux(conn)`.

## 4. Front-end names

- VERIFIED: `name` = `"<base> (<frontend>)"` ([RX L100-111](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L100-L111), [TX L197-208](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L197-L208)) with base `BasicRX`, `LFRX`, `BasicTX`, `LFTX`: e.g. `BasicRX (0)`, `BasicRX (1)`, `LFTX (0)`.
- VERIFIED: `get_rx_subdev_name(chan)` = the radio's `get_fe_name`, which reads that `name` node (MUR, XRC). INFERRED: prefix matching on `BasicRX`, `LFRX`, `BasicTX`, `LFTX` is safe.
- VERIFIED, quirk: the TX `id` node copies the RX strings (`basicrx`/`lfrx`, [L219-220](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L219-L220)); `id` is not `name`.

## 5. Dboard IDs and detection

- VERIFIED (DB-HPP ~L11-21): BASIC_TX 0x0000, BASIC_RX 0x0001, LF_TX 0x000E, LF_RX 0x000F; RFNoC PIDs OR `0x6300`. All eight registered ([L71-84](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L71-L84)).
- VERIFIED: XRC `_init_db` (~L1490-1530) reads the EEPROM, ORs the RFNoC flag for the four base PIDs and presents the RFNoC frontends. X300 auto-detects all four.

## 6. Other properties; phase

- VERIFIED: `connection` = `antenna_mode_to_conn[mode]` ([L137-140](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L137-L140)); `sensors` empty; `enabled` true; `use_lo_offset` false ([L141-142](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L141-L142)); no `lo_locked`; the constructor disables the dboard clock ([L171-177](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/dboard/db_basic_and_lf.cpp#L171-L177)).
- VERIFIED: XRC has no `sync_phase`, `lo_locked`, `use_lo_offset` or `phase` (searched); its `set_command_time` override is "for TwinRX only"; `set_rx_frequency`/`set_tx_frequency` do not call it.
- INFERRED: only the DDC/DUC tunes; phase determinism then depends on the DDC/DUC frequency-word handling, which was not read. Design 33's `deterministic` for Basic/LF stays to be confirmed on the bench.

## 7. Constraints

- VERIFIED: Basic/LF never set or lock the dboard clock (searched `db_basic_and_lf.cpp` for `set_clock_rate`, `lock_clock_rate`, `get_clock_rate`: only `set_clock_enabled`). `x300_dboard_iface.cpp` (~L45-116) forwards clock calls to the clock controller.
- VERIFIED: [x300_clock_ctrl.cpp](https://github.com/EttusResearch/uhd/blob/v4.10.0.0/host/lib/usrp/x300/x300_clock_ctrl.cpp): `set_dboard_rate` needs an integer divisor of the VCO (~L141-147) and throws if `dboard_clock_rate` was passed or a dboard locked the rate (~L167-182); master clock 120, 184.32 or 200 MHz with a 10 MHz reference (~L601-669).
- VERIFIED: `db_cbx.cpp` (~L56) only reads the dboard clock rate. UBX and OBX were not read.
- INFERRED: a Basic/LF board beside a UBX/OBX/CBX adds no clock constraint of its own.
- VERIFIED: XRC calls no `dboard_clock_rate`, `lock_clock_rate` or `set_clock_rate` (searched).
- VERIFIED: each RX channel has its own `rx_frontend_core_3000`, both at the master clock rate; one `x300_adc_ctrl` and one `rx_codec/gains/digital` per radio; the radio's `set_rate` cannot change the rate (XRC ~L256-258); DDC decimation and frequency are per DDC channel (`ddc_block_control.cpp`).
- INFERRED: RX "0" (antenna A) and "1" (antenna B) can run at different DDC rates and frequencies; nothing found forbids it. Antenna is per channel. How the connection modes map to the ADCs when both channels use `AB` was not traced.

## Not read

`gain_group` internals; UBX and OBX sources; `multi_usrp_rfnoc.cpp` bodies (summaries only); `x300_mb_controller.cpp` has no `dboard_clock_rate`, `lock_clock_rate` or `set_clock_rate` (searched).
