> plan · gpt-6.1-sol
## Conclusion

**Investigate small-send overhead and grouped-stream flow-control stalls first—not a larger TX window.** The code establishes both mechanisms, but the record alone cannot prove which causes the starvation.

Two premises in the brief need correcting:

- **VERIFIED:** the record already identifies the failures as `Underflow`.
- **VERIFIED:** throughput sends are **1,000 samples/channel for single and split streams, but 500/channel for grouped streams**, not 2,000.

### Evidence notation

Below, `UHD/` means the [UHD v4.10.0.0 source tree](https://github.com/EttusResearch/uhd/tree/v4.10.0.0).

`record` means:

`~/ezsdr-bench/2026-10-28/98b0d9a919c9-154746201756884/measurement.json`

## 1. What actually failed

**VERIFIED from the saved record:**

| Trial | Evidence | Playback duration before closure¹ |
|---|---|---:|
| Single 200 MS/s | Four `Underflow`; no short-send rejection | ≈18.46 s |
| Grouped 40 MS/s/channel | One `Underflow`; zero short sends; both channel ACKs present | ≈93 ms |

Sources: `record:221845–221904`, `record:222228–222258`.

¹Calculated from burst-start and ACK ticks at the 200 MHz master clock. These are **not first-underflow timestamps**.

Thus these failures are not merely missing acknowledgements or ambiguous `DeviceFailure` classifications.

## 2. TX hot path and window

### Waveform source

**VERIFIED:** this test does **not** continuously feed samples through a Kernel stream link.

1. Single-channel throughput creates a 1,000-complex-sample artifact and schedules one repeating burst:  
   `crates/ezsdr-radio-uhd/tests/common/measurement.rs:3374–3382`.
2. Multi-channel throughput likewise creates one artifact containing 1,000 complex samples total:  
   `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:627–629`.
3. `uhd-control` decodes the artifact once into channel-major host vectors, then hands an `Arc` to `uhd-tx`:  
   `crates/ezsdr-radio-uhd/src/provider/control.rs:1170–1188,1257–1265`.
4. `uhd-tx` repeats those vectors using a modulo offset and borrowed slices:  
   `crates/ezsdr-radio-uhd/src/provider/tx.rs:405–425`.

There is no recurring producer-thread handoff or artifact decoding per send.

### Window and waiting

**VERIFIED:**

- Normal X3xx descriptions use **`tx_in_flight_ns = 10_000_000`**, including throughput Runs:  
  `crates/ezsdr-radio/src/x3xx.rs:393–402`;  
  `crates/ezsdr-radio/src/channels.rs:352–366`.
- The record reports **5 ms as a measured candidate result**, but this does **not** replace the window for subsequent throughput tests:  
  `record:3302–3307`;  
  `crates/ezsdr-radio-uhd/tests/common/measurement.rs:2254–2278,3382`.
- Each turn computes `window_cut = now + W`, expressed on the sample clock:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:305–307`.
- Except for the first timed buffer, it waits until the entire desired chunk fits:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:415–420`.
- A blocked turn ends in **`std::thread::sleep(200 µs)`** on real hardware—no condition-variable wake or spin:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:107,157–159`;  
  `crates/ezsdr-radio-uhd/src/host.rs:305–310`.

At 200 MS/s:

| Quantity | Samples | Playback time |
|---|---:|---:|
| Declared window | 2,000,000 | 10,000 µs |
| Block-length ceiling | 2,000 | 10 µs |
| Actual throughput waveform/send | 1,000 | **5 µs** |
| Requested idle sleep | 40,000 | **200 µs** |

**Important distinction:** 10 ms is an upper bound on handover, **not a guarantee of 10 ms buffered playback**. UHD can block before this software window fills.

**VERIFIED:** `benchmark_rate` repeatedly calls `send` without its own window or sleep. UHD waits for destination flow-control credits internally:

- `UHD/host/examples/benchmark_rate.cpp:265–278`
- `UHD/host/lib/include/uhdlib/rfnoc/chdr_tx_data_xport.hpp:235–241`
- `UHD/host/lib/transport/inline_io_service.cpp:274–290`

**INFERRED:** if UHD’s usable buffering is smaller than the software window, the window-full sleep is not the steady-state bottleneck. We need a window-wait counter and cursor-ahead measurements before blaming that sleep. The current record does not establish actual queued margin.

## 3. Per-send overhead

### Send sizes

**VERIFIED:** the desired chunk is capped by both `block_len` and the remaining waveform before wrap:

`crates/ezsdr-radio-uhd/src/provider/tx.rs:405–410`.

Consequently:

| Configuration | Samples/channel/send | Required sends/s |
|---|---:|---:|
| Single 200 MS/s | 1,000 | 200,000 |
| Split 100+100 MS/s | 1,000 | 100,000 per owner |
| Grouped 40+40 MS/s | **500** | 80,000, each handling two channels |

The grouped reduction follows directly from decoding `bytes / (8 × channels)`:

`crates/ezsdr-radio-uhd/src/provider/control.rs:1170–1178`.

The record corroborates it: grouped 40 MS/s accepted 3,722,500 samples/channel in 7,445 blocks—exactly 500/block (`record:222228–222236`).

### Work repeated for every send

**VERIFIED:**

- Two allocated slice-pointer vectors in `Tx::send`:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:425,460`.
- Another allocated pointer vector plus metadata creation/free in the Rust FFI adapter:  
  `crates/ezsdr-radio-uhd/src/uhd.rs:1114–1121`.
- UHD metadata creation/free really performs C++ `new`/`delete`:  
  `UHD/host/lib/types/metadata_c.cpp:101–120`.
- A contract identifier is parsed and allocated per block:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:505`;  
  `crates/ezsdr-kernel/src/contract.rs:28–45`.
- Stream-state locking, async polling and another stream-state lock occur before each step:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:170–184`.
- Device streamer-map and per-streamer-call mutexes are taken for both send and async polling:  
  `crates/ezsdr-radio-uhd/src/uhd.rs:1102–1112,1128–1129`.
- Statistics and burst bookkeeping also lock the shared record:  
  `crates/ezsdr-radio-uhd/src/provider/tx.rs:515–521,535–546`;  
  `crates/ezsdr-radio-uhd/src/provider/core.rs:949–950`.
- Multi-channel measurements add a shared counting mutex after every send:  
  `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:319–326`.

There is **no routine per-send text logging or full waveform copy** in `Tx::send`; normal contiguous tracker updates are counter updates, not accumulating JSON block rows:

`crates/ezsdr-kernel/src/stream/burst.rs:354–371`.

### Format, conversion and packets

**VERIFIED:** both implementations default to **CPU `fc32`, wire `sc16`**. Conversion is therefore not a format difference explaining the comparison.

- Module: `crates/ezsdr-radio-uhd/src/uhd.rs:1083–1086,509–512`.
- Benchmark: `UHD/host/examples/benchmark_rate.cpp:505–508`.

UHD converts samples into packet buffers and sends each channel separately:

`UHD/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:425–447`.

For UDP, packet release reaches a socket `send`:

`UHD/host/lib/include/uhdlib/transport/udp_boost_asio_link.hpp:138–140`;  
`UHD/host/lib/include/uhdlib/transport/udp_common.hpp:128–146`.

UHD sends a short request as a short packet rather than combining successive requests:

`UHD/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:279–285`.

**INFERRED:** the record’s 1,000/500-sample requests increase packet/syscall frequency and amplify the Module’s additional allocations, locking and bookkeeping. Benchmark’s default send size is its maximum packet size, approximately 1,996 here—not 1,000:

`UHD/host/examples/benchmark_rate.cpp:849–863`;  
`plan/phase8/bench-results.md:153–157`.

## 4. Other threads and grouped versus split

### Other threads

**VERIFIED:** only the TX owner explicitly raises its priority in this path:

`crates/ezsdr-radio-uhd/src/provider/tx.rs:139–144`;  
`crates/ezsdr-radio-uhd/src/provider/mod.rs:394–409`.

Control and the test producer are not required to supply each repeat block. However, TX shares ordinary mutexes with other threads. Reading `now` also acquires the clock-anchor mutex, although it does not perform a hardware time read per block:

`crates/ezsdr-radio-uhd/src/provider/core.rs:927–928`;  
`crates/ezsdr-radio-uhd/src/authority.rs:142–147,245–252`.

**INFERRED:** non-RT lock holders can delay TX. Their actual scheduling policies and lock-hold durations were not retained in this record.

There is also a priority-comparison caveat: **`benchmark_rate --priority high` elevates its main thread before device creation**, as well as its TX worker:

`UHD/host/examples/benchmark_rate.cpp:549–551,564–575,223–225`.

Both request UHD’s default 0.5 real-time priority—Linux `SCHED_RR`, normally priority 50:

`UHD/host/include/uhd/utils/thread.hpp:17,35–44`;  
`UHD/host/lib/utils/thread.cpp:47–69`.

**INFERRED:** inherited scheduling may make the benchmark’s supporting threads different from the Module’s. “Both TX threads request RT” is not proof of equivalent process scheduling.

### Why grouped can fail much earlier

**VERIFIED:** UHD obtains buffers for **all channels before converting and releasing any channel’s packet**:

`UHD/host/lib/include/uhdlib/transport/tx_streamer_zero_copy.hpp:95–106`;  
`UHD/host/lib/include/uhdlib/transport/tx_streamer_impl.hpp:435–447`.

Thus grouped has:

- Half-length per-channel waveforms: 500 samples.
- One owner doing both channels’ conversion and packet submission.
- A synchronization point where waiting for one channel delays submission for the other.

Split has independent owners and independent progress.

**INFERRED:** this combination—short packets plus serial/channel-coupled flow control—is the strongest explanation for grouped failing at 40 MS/s/channel while split passes at 100. Aggregate Ethernet bandwidth alone does not explain that ordering. It is not yet proof of a UHD grouped-stream defect.

## 5. Ranked hypotheses and cheapest experiments

**Recommendation:** use one small, isolated hardware diagnostic, not another 83-minute record run. Retain the existing short-waveform case; a longer waveform alone would hide a real user-facing limitation.

### 1. Small-send overhead, amplified by grouped channel coupling

**Highest priority; INFERRED causality.**

First, no-code UHD controls on #2, using the existing Docker/RF-safe setup:

```sh
benchmark_rate --args addr=192.168.40.37 \
  --tx_subdev 'A:0 B:0' --tx_channels 0,1 \
  --tx_rate 40e6 --tx_spb 500 --priority high --duration 60 \
  --underrun-threshold 0 --seq-threshold 0
```

Repeat with `--tx_spb 1000`, then `1996`; compare `--multi_streamer`.

- Grouped UHD failure at 500 but not 1,996 implicates packet size/grouped transport.
- UHD success at 500 while the Provider fails implicates additional Provider work or scheduling.

**Small probe needed:** add an ignored `hw_tx200_hot_path_probe` running only single-200, grouped-40 and split-100, with the same safe waveform amplitude. Compare 1,000 versus 1,996 samples **per channel**, then larger chunks.

Proposed invocation, **after adding that test**:

```sh
cargo test --release -p ezsdr-radio-uhd --features uhd \
  --test hardware hw_tx200_hot_path_probe -- --ignored --exact --nocapture
```

Reuse the existing Run construction and evidence extraction, without `ProbeDevice`’s per-call JSON recording.

**Likely fix:** reuse pointer/metadata storage; remove invariant parsing from each send; batch short waveform repetitions into a reusable contiguous buffer. Separate TX chunk sizing from RX block sizing if measurements justify it.

**Risk:** batching must preserve waveform wrap, partial sends, cuts/preemption and the strict in-flight bound. Merely enlarging the test waveform is not a production fix.

### 2. Scheduling jitter or priority inversion

**INFERRED.**

During that isolated probe, sample `/proc/<pid>/task/*/{stat,status}` for TX, control and UHD workers; check RT throttling. Compare the unchanged probe launched under `chrt -r 50` against TX-only elevation.

This is diagnostic, not a recommendation to run everything RT.

**Likely fix:** shorten shared critical sections or elevate only a demonstrated dependency.

**Risk:** blanket RT promotion can starve networking or other device work.

### 3. Software-window sleep causes starvation

**Possible, but currently less supported.**

Add only aggregate counters: window-full waits, send-duration histogram and minimum cursor-ahead time. Compare the same probe with measurement timing windows of 5, 10 and 20 ms, using the existing `with_measurement_timing` hook:

`crates/ezsdr-radio-uhd/tests/common/measurement.rs:2272–2278`.

- No window waits near failure rejects this hypothesis.
- Repeated window sleeps followed by depleted margin supports it.

**Likely fix:** deadline-aware waiting/refill, not an unconditional spin or arbitrary window increase.

**Risk:** changing the window changes Stop/preemption behavior and Mock parity. Also, raising `block_len` alone cannot help a 1,000/500-sample waveform: wrap remains the smaller limit. The record selector does not currently accept `block_len` as an environment override (`tests/common/mod.rs:114–115`).

## 6. Keep the failure evidence

**No change is needed to retain which code failed these throughput trials:** `async_failures`, underflow counts and short-send evidence already exist.

Sources:

- `crates/ezsdr-radio-uhd/tests/common/measurement.rs:3339–3353,3428–3429`
- `crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:714–729`

The useful minimal improvement is to retain the **full failure report rows**—`code`, `tick`, `channel`, `stream`—rather than only code strings. Production already records those fields:

`crates/ezsdr-radio-uhd/src/provider/tx.rs:897–900`.

Also retain priority-success/rejection evidence. For multi-stream observations, distinguish stream-local underflow events from Run-wide faults: currently all events are passed to every stream classifier, so one stream’s underflow can mark its healthy companion `DeviceFailure`:

`crates/ezsdr-radio-uhd/tests/common/measurement/multichannel.rs:708–719`.

No files changed and no RF tests run. Actual buffer occupancy, scheduler stalls and the first-underflow timing remain unmeasured; the causal ranking is therefore provisional.
