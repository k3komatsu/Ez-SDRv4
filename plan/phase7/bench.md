# Phase 7 — the first bench session

The one part of Phase 7 that needs hardware: an X310 (or an X300) with **one OBX** daughterboard in slot A, cabled in loopback, on a Linux PC, driven by the implementation of specs 18 and 19 once §9's steps 1–6 of [`00-overview.md`](00-overview.md) are done. Everything before this page runs on a Mac without a USRP. The session answers two questions: do the acceptance crate's experiments and the Phase 6 snippets run on the X310 with only the profile changed (Vision §59; Gate X's criterion 9 if the owner accepts T18), and what are the values spec 18 marks INFERRED (inputs for Phase 8, not changes to any profile now).

**The bench is one OBX, not two UBX** (the owner, 2026-09-30: first one CBX, "UBX2枚での試験ではなくCBX1枚でループバックで可能なように", then "了解ですOBXに切り替えてください"). Its profile is `x310-obx` 0.1.0 (spec 18 UR-9): `x310-ubx`'s values with one channel each way and 10 MHz–8.4 GHz (UHD 4.10's `obx_freq_range`, `db_obx.hpp`, VERIFIED), the default frequency RM-5's 1 GHz. The OBX is the UBX's design grown: two MAX2871 LOs each way and the same timed LO phase sync (`obx_expert.cpp`'s `_sync_phase`, VERIFIED), so the numbers the session measures are an OBX's but come from the same kind of synthesizer as the UBX's (INFERRED: their tune code differs). UR-5 refuses a profile whose channels or front ends the device does not have. `x310-ubx` stays unmeasured; design-notes §9–§10 record what that leaves for Phase 8. The hardware tests take the profile from the front end UHD names on receive channel 0 (`OBX…` is `x310-obx`, `CBX…` `x310-cbx`, anything else `x310-ubx`) and run at that profile's default frequency; B1 fails unless it is `x310-obx`. **UHD 4.9 or later** drives an OBX (UHD added it in June 2025; an older UHD names it an unknown board, which UR-5 refuses).

**An X300 serves as well** (the owner, 2026-09-30: "USRP X310の代わりにUSRP X300でもいいですか？" — yes). UHD 4.10 drives both with its one `x300` driver, telling them apart only by the EEPROM's product code and the FPGA image (`x300_mboard_type.cpp`, VERIFIED); the X300's FPGA is the smaller XC7K325T, which matters only for RFNoC blocks such as Replay that this Module does not use (the repeat is streamed from host memory, spec 18 UR-22; INFERRED that the X300's default image carries what the steps need). UR-5 checks the master clock, the channels and the front ends, not the product, so the `x310-…` profiles bind an X300 unchanged. Load the X300's image (`uhd_image_loader` picks `usrp_x300_fpga_XG.bit` for it), and record the product (`X300` in the `pp_string`) at the top of `bench-results.md`, so that Phase 8 knows which device its numbers came from.

Results go to `plan/phase7/bench-results.md`, created at the session (GZ-10): the commit, `uhd_config_info --version`, the device's `pp_string`, and for each step the command, its outcome and the numbers it printed. The table at the end of [`plan/spikes/2026-09-26-uhd.md`](../spikes/2026-09-26-uhd.md) is filled from it.

## B0 — Set-up (no RF)

1. **Host.** Rust ≥ 1.85 (`rustup update`); UHD ≥ 4.9 (the OBX; `docker/uhd4.10/Dockerfile` has 4.10 and everything below — Ubuntu's `apt` UHD is older) with headers and its `.pc` file (Ubuntu: `sudo apt install libuhd-dev uhd-host pkg-config`, or the Ettus PPA; a source build under `/usr/local` needs `PKG_CONFIG_PATH` or `UHD_LIB_DIR=/usr/local/lib` to build, and `sudo ldconfig` or `LD_LIBRARY_PATH=/usr/local/lib` to run, since the build adds no rpath, spec 18 UR-35). Check `pkg-config --modversion uhd`.
2. **Network.** X310 10 GbE port 0 at `192.168.40.2`, host NIC `192.168.40.1/24`, MTU 9000; `sudo sysctl -w net.core.rmem_max=33554432 net.core.wmem_max=33554432`. `uhd_find_devices --args addr=192.168.40.2` lists it. An FPGA image that does not match UHD's compatibility number needs `uhd_image_loader --args addr=192.168.40.2` (UHD 4.x's image).
3. **Master clock.** The X310 must run at 200 MHz (spec 18 UR-5 refuses 184.32 MHz for both profiles); if `uhd_usrp_probe` shows another rate, add `master_clock_rate=200e6` to the device string.
4. **Software checks, still without RF.** In a clone at the implemented commit (the branch `worktree-phase7-impl` until it is merged into `main`; `handoff.md` §4 has the session's plan and what the tests implement of each step):
   ```sh
   cargo test --workspace                                   # the software exit criteria
   cargo test -p ezsdr-radio-uhd --features uhd             # the C API tests (uhd_api.rs); no device needed
   cargo test -p ezsdr-radio-uhd --test fake on_one_obx     # B3, B4, B6, B7 on a one-OBX fake (already in the first line)
   cargo build --release -p ezsdr-server --features uhd     # the lab server
   ```
   Record the four outcomes. `docker/uhd4.10/Dockerfile` (or `.devcontainer/`) is a host with all of this installed.

## RF safety (before any step that transmits: B6, B7, B8)

- Cable **TX/RX of the OBX (slot A) to its RX2 through ≥ 30 dB of attenuation** (or the IBFD front end that the owner already uses), or terminate TX/RX in a 50 Ω load for the steps that only need the transmitter to run.
- Every bench profile carries `radio.rf_envelope` with **one narrow allowed band around the test frequency**, `max_gain_db: 0` and `tx_enabled: [true]` (RM-19): a Session command outside it is refused before it reaches the device (§58 #16), which step B7 checks on purpose.
- Transmit gain 0 dB, waveform amplitude ≤ 0.5, a frequency in a band the lab is licensed or shielded for. The steps use the profile's default frequency, 1 GHz on `x310-obx` (RM-5's); if the lab needs another, give `x310-obx` its own default the way `x310-cbx` has one (`profile.rs`, spec 18 UR-9) and change the envelopes below with it, before the session.
- `ezsdr.rf_path` is `cabled` (the class is HardwareInLoop). A Hardware (over-the-air) Run is not part of this session.

## The bench profile

The profile that replaces the Mock one; only the radio binding, the Authority and `ezsdr.rf_path` differ from `crates/ezsdr-acceptance/src/rig.rs`'s `spec_profile` (Vision §59). `<dir>` is a capture directory on the bench PC.

```json
{
  "version": 1,
  "bindings": {
    "radio": {
      "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "profile": { "name": "x310-obx", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "selector": { "args": "addr=192.168.40.2", "id": "usrp", "clock_source": "internal", "time_source": "internal" }
    },
    "rec": {
      "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
      "selector": { "dir": "<dir>" }
    }
  },
  "authority": "radio",
  "placements": {
    "links": [{
      "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
      "from": { "component": "radio", "port": "rx" },
      "to": { "component": "rec", "port": "in" }
    }]
  },
  "environment": {
    "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2000000000 },
    "ezsdr.rf_path": { "path": "cabled" },
    "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999000000, "hi_hz": 1001000000 }], "max_gain_db": 0, "tx_enabled": [true] }
  }
}
```

Every hardware test takes the device string from `EZSDR_UHD_ARGS` and writes its profile itself. The Python step B7 reads the **Session profile** from the file `EZSDR_PROFILE` names (VE-5); save this one as `bench-session.json` (the Spec profile above with a `feed` on `rec`, as SB-22c requires of a Session):

```json
{
  "version": 1,
  "bindings": {
    "radio": {
      "module": { "id": "ezsdr.radio.uhd", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "profile": { "name": "x310-obx", "version": { "major": 0, "minor": 1, "patch": 0 } },
      "selector": { "args": "addr=192.168.40.2", "id": "usrp", "clock_source": "internal", "time_source": "internal" }
    },
    "rec": {
      "module": { "id": "ezsdr.sink.capture", "version": { "major": 1, "minor": 2, "patch": 0 } },
      "selector": { "dir": "<dir>" },
      "feed": { "port": { "component": "radio", "port": "rx" }, "policy": "drop_oldest", "capacity": 64 }
    }
  },
  "authority": "radio",
  "placements": {
    "links": [{
      "link": { "id": "ezsdr.link.host", "version": { "major": 1, "minor": 0, "patch": 0 } },
      "from": { "component": "radio", "port": "rx" },
      "to": { "component": "rec", "port": "in" }
    }]
  },
  "environment": {
    "ezsdr.time": { "class": "hardware_in_loop", "start_lead_ns": 2000000000 },
    "ezsdr.rf_path": { "path": "cabled" },
    "radio.rf_envelope": { "allowed_bands": [{ "lo_hz": 999000000, "hi_hz": 1001000000 }], "max_gain_db": 0, "tx_enabled": [true] }
  }
}
```

## The steps

Run each with `EZSDR_UHD_ARGS=addr=192.168.40.2 cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware <name> -- --ignored --nocapture`. The order matters: each step checks what the next relies on.

| # | Test (spec 18 UR-34) | What it does | Passes when | Records (for `bench-results.md` and Phase 8) |
|---|---|---|---|---|
| B1 | `hw_b1_probe` | `open`, `describe`, channel counts, `ref_locked`, three time reads 100 ms apart, and every channel's transmit frequency and gain as the device reports them (before any step transmits: what a Session would inherit if nothing configured the transmitter, spec 18 UR-25) | the device opens at 200 MHz; the profile is `x310-obx` (the test fails otherwise), with at least its 1 + 1 channels (UHD reports 2 + 2: slot B empty is its unknown board); time advances by 20 000 000 ± 1 % ticks per 100 ms | `pp_string`, UHD version, sensors, the front ends' names, the transmit settings found |
| B2 | `hw_b2_authority` | builds `DeviceAuthority`; schedules 100 wakeups 10 ms apart and measures how late each `next_wakeup` returns in host time; reads `relations()`; re-anchors for 10 s and records the anchor's drift | median lateness < 1 ms, maximum < 25 ms (the 20 ms nap plus scheduling); both relations present | the lateness distribution; the relations' uncertainties; the observed device/host drift (UR-8's INFERRED bound) |
| B3 | `hw_b3_receive_at_t0` | `experiments::receive(1, 1e6, f, Some(10_000))` at the bench frequency `f` under the bench profile (no RF needed) | `Completed`; the capture's first continuity map starts at receive sample 0 with no gap; `timing` shows the first block at index 0; `applied` shows the rate equal to the claim | the frequency read-back difference (K13: expected < 0.05 Hz); the first block's host delay after `start` |
| B4 | `hw_b4_capture_at_a_sample_index` | a Session capture of 10 000 samples `at` receive sample 50 000 (v3 behaviour 2) | the artifact's first sample is index 50 000 | — |
| B5 | `hw_b5_overflow` | a receive Run with `UhdRadio::with_rx_stall(500 ms, 300 ms)` at 10 Msps; if no overflow occurs, again with the stall at 1 s and then 2 s (the host's socket buffer, `net.core.rmem_max`, may absorb 300 ms of `sc16` at 10 Msps, about 12 MB) | `radio.RX_OVERFLOW { cause: overrun }` delivered; the capture spanning it has an `overflow_restart` gap whose `lost` equals the block's time jump (§58 #6) | the stall that first overflowed and the socket buffer sizes; the measured restart gap (MR-3 claims 50 ms, VERIFIED from UHD's source) |
| B6 | `hw_b6_txrx_and_repeat` | with the loopback cable: a timed burst and a timed capture on one device (v3 behaviour 3); then a 3 s `start_repeat` of a 1 000-sample waveform, captured | the burst's correlation peak is found; the repeat's capture shows the waveform back to back across every wrap; no `TX_UNDERFLOW` | the transmit-to-receive delay in samples (MR-3 claims 45, INFERRED); the repeat's `wraps` |
| B7 | Python, not a test | `EZSDR_SERVER=target/release/ezsdr-server EZSDR_PROFILE=<session profile> python3 python/examples/minimal.py` (Vision §3's snippet, unchanged); then `python3 python/examples/bench_loopback.py` (`tx.repeat(x)`; `y = sdr.rx.capture(N, at=sdr.after(0.05))`; `t = sdr.sleep(0.1)` and `z = sdr.rx.capture(N, at=t)`; then `sdr.rx.sample_rate = 19.5e6`; then a retune outside the RF envelope) | both run; `y` holds the waveform and starts at its instant; no `TIME_ERROR` in the Manifest (spike K6); the rate is coerced to 20 Msps and starts a new SampleClock (UR-25); the out-of-envelope retune raises `ezsdr.Rejected` naming `radio.rf_envelope` and is logged (§58 #16) | the Manifest paths; whether `z` started at `t` (EA-17's INFERRED case: does a local request outrun the device's receive latency?) |
| B8 | `hw_b8_leads` | bursts at leads of 10, 5, 3, 2, 1.5, 1, 0.5 ms from their receipt; timed retunes at the same leads; `cold` rate changes at restart leads of 100, 50, 25, 10 ms; a repeat stopped by `Stop`, measuring where its record ends; repeats at in-flight windows of 10, 5, 3, 2 ms, watching for `TX_UNDERFLOW`; a burst preempting a running repeat at leads of 20, 10, 5 ms; 1 to 32 timed retunes queued ahead, timing each `apply` call; a timed stop of the receive stream | — (a measurement, not a pass/fail step) | the device lead (the least lead with no `late_at_device`), the delivery allowance (receipt − dispatch, from `timing`), the restart lead, the least in-flight window without underflow, the preemption bound, how many timed OBX tunes fit before `apply` blocks (UR-24's queue depth), the release window's margin, the transmit end after `Stop`, whether a timed receive stop is honoured (UR-25's INFERRED point) |
| B9 | `hw_b9_unplug`, `hw_b9_unplug_transmit_only` and `hw_b9_unplug_and_reopen` (manual; also through a switch, the host's carrier kept: Review S, TG-S2) and `hw_b9_usrp2_probe` | during a receive Run, and again during a transmit-only Session with nothing sent, unplug the 10 GbE cable; separately, run the steps B3–B7 once more and check that no `DEVICE_LOST` appears; then `EZSDR_UHD_ARGS=addr=192.168.10.2 … hw_b9_usrp2_probe` on the USRP2, which opens it, prints `describe()` and builds a `UhdRadio` from a binding of the profile its front end names | `DEVICE_LOST` within about 1 s in both unplugged Runs, each ending `Stopped { policy { DEVICE_LOST } }` with its Manifest written; no false `DEVICE_LOST` in the rerun; the USRP2 opens and `from_binding` refuses it by UR-5 (100 MHz master clock) | the UHD error each unplug produced (UR-29's INFERRED list, `UHD_ERROR_RUNTIME`'s false positives); the USRP2's `pp_string` (its daughterboard, for a future profile) |

Criterion 9 of the overview's §10 is B0–B8 passing; B8 and B9 are recorded whatever they show.

## If a step fails

A failure on the bench is a finding, not a bug to patch at the bench: record it in `bench-results.md` with its output, and fix it in `main` with a test on `FakeDevice` that reproduces it before the step is run again. Common causes the spike anticipated: a UHD/FPGA compatibility mismatch (`uhd_image_loader`), a master clock other than 200 MHz (B0.3), socket buffers too small for the rate (UHD prints a warning at start), and a USRP2 that UHD 4.x no longer drives (not verified on this build).

## handoff からの記録（2026-10-14 に移動）

handoff.md に 2026-10-14 まであった記録を，そのまま移した（AGENTS.md §9：記録は作業の置き場に1か所）．当時の文なので，今の規則は accepted specs と照合すること．

**前提と規則**

- **実機構成**（owner，2026-09-30）：X310（**X300 でも可**：UHD は同じ `x300` driver で扱い，違いは FPGA の大きさだけで，この実装はそれを使わない．コードも profile `x310-obx` も変更不要．FPGA image は X300 用（`uhd_image_loader` が `usrp_x300_fpga_XG.bit` を選ぶ）を入れる．bench.md 冒頭）の slot A に **OBX 1枚**．TX/RX を 30 dB 以上の減衰器を通して同じ OBX の RX2 へ（ループバック）．送信だけの段のために 50 Ω 終端．10 GbE（SFP+）の Linux PC．外部の 10 MHz/PPS は不要．**UHD 4.9 以上が必須**（OBX は 2025 年 6 月に UHD に入った．古い UHD では unknown board になり UR-5 が拒否する）ので Docker 環境（UHD 4.10）を使う．X310 の FPGA image もその UHD に合わせる．profile は `x310-obx` 0.1.0（1 channel，10 MHz–8.4 GHz，既定周波数 1 GHz）で，テストは UHD が報告する front end 名（`OBX…`）から自動で選ぶ．OBX が 2 枚あっても同じ profile で動く（使うのは channel 0）．途中まで CBX 1枚で準備したので `x310-cbx` も残っている（OBX の driver が不調なときの予備）．事情は [design-notes.md](design-notes.md) §9・§10．
- 作業するのは branch `worktree-phase7-impl`（`origin` にある，この節を書いた最後の commit 以降）．`main` にはまだ Phase 7 がない．この branch に commit して push してよい（v4 の作業なので `master` と v3 の tag には触れない，AGENTS.md §1）．`main` への merge は owner の判断なのでしない．
- 実機で失敗したら，その場でコードを直さない（bench.md「If a step fails」）．`plan/phase7/bench-results.md` に出力ごと記録し，`FakeDevice` で再現するテストを先に書いてから直し，その段をやり直す．直したら `cargo test --workspace` を通して commit・push．
- 設計を変える必要があると判断したら（INFERRED だった値が違う，規則が実機に合わない），実装より先に `plan/phase7/design-notes.md` に「§10 bench」節を作って記録し，owner に判断を仰ぐ．spec 18 の数値（device lead 2 ms，delivery allowance 3 ms，restart lead 50 ms，in-flight window 10 ms，release window 3 ms，queue depth 16）は Phase 8 の入力として **記録するだけ** で，profile の値はこの session では変えない（bench.md 冒頭，spec 18 §3）．
- build 出力は作業ツリーの外に置く（`CARGO_TARGET_DIR` をツリー外に，AGENTS.md §7）．Python は `PYTHONDONTWRITEBYTECODE=1`．

**0. 取得と環境（B0）**

1. `git clone git@github.com:k3komatsu/Ez-SDRv4.git && cd Ez-SDRv4 && git switch worktree-phase7-impl`（`git log -1` がこの節を書いた commit 以降であること）．`v3/` は不要（`.gitignore` 済み）．
2. **推奨：Docker の UHD 4.10 環境**（`docker/uhd4.10/Dockerfile`，VS Code なら `.devcontainer/`）．v3 の `docker/v3_prebuild` と同じ base image `ghcr.io/k3komatsu/uhd:v4.10`（Ubuntu 26.04，UHD 4.10.0.0 を `/usr/local` に source build，FPGA image 取得済み，Python 3.14 + numpy）に Rust stable と 1.85.0 を足しただけ．source は mount，build 出力は volume `/cargo-target`（`CARGO_TARGET_DIR` と `EZSDR_SERVER` は設定済み）：
   ```sh
   docker build -t ezsdr-v4-dev:uhd4.10 docker/uhd4.10
   docker run -it --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
       --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target \
       ezsdr-v4-dev:uhd4.10
   ```
   `--network=host` は X310 の 10 GbE に届くため，SYS_NICE と rtprio は UHD の streaming thread の優先度のため．MTU と `net.core.{r,w}mem_max`（下の 3.）は host 側で設定する（container の設定ではない）．devcontainer（VS Code の「Reopen in Container」か `npx @devcontainers/cli up --workspace-folder .`）は同じ引数で起動し，user `ubuntu`（UID は host に合わせられる），build 出力は別の volume `ezsdr-v4-devcontainer-target` に置く．この環境では `LD_LIBRARY_PATH` も `UHD_LIB_DIR` も要らない（`ldconfig` 済み，`pkg-config` が `/usr/local/lib` を返す）．下の 4. のコマンドはこの中でそのまま動く（`export CARGO_TARGET_DIR` は不要）．
   - 2026-09-28 に Mac（arm64）上の amd64 emulation でこの image を build し，確認済み：`cargo test -p ezsdr-radio-uhd --features uhd --test uhd_api` 4 passed（x86_64 Linux での 69 関数の link と構造体サイズ 40/40/48/40），clippy `-D warnings`（feature `uhd`）clean，`ezsdr-server --features uhd` が `/usr/local/lib/libuhd.so.4.10.0` に link，Python 23 件 OK（3.14），devcontainer（`DOCKER_DEFAULT_PLATFORM=linux/amd64`）でも user `ubuntu` のまま `uhd_api` 4 passed．`cargo test --workspace --no-fail-fast` は 836 passed・3 failed で，落ちたのは wall-clock に依存するテストだけ：`fake.rs` の `rehearsal_b6`・`rehearsal_b7`，acceptance の `ea_07_a_session_on_the_fake_device`．`--test-threads=1` にすると別の 4 本（`ur_07_now_tracks…`，`ur_22`，`ur_23`，`ur_25_tx_channels_from_zero…`）が落ちる．B6 の中身は `TIME_ERROR late_at_device`（5.35 ms 遅れ）で，burst が `drop_and_flag` で捨てられていた．emulation では Mac native より 6 倍ほど遅く（`fake.rs` が 33 s → 194 s），落ちるテストが実行ごとに変わるので，emulation の遅さによるものと判断した（INFERRED）．実機の x86_64 では native に動くので，4. の `cargo test --workspace` が 839 passed になることを最初に確かめる．そこで落ちたら負荷の問題ではない可能性があるので，出力ごと記録する．
   Docker を使わない場合：Rust は `rustup` で stable と `1.85.0`．UHD 4.x の開発用パッケージと pkg-config：Ubuntu なら `sudo apt install libuhd-dev uhd-host pkg-config`（または Ettus PPA）．`pkg-config --modversion uhd` と `uhd_config_info --version` を記録．
   - `/usr/local` に source build した UHD なら `PKG_CONFIG_PATH=/usr/local/lib/pkgconfig` か `UHD_LIB_DIR=/usr/local/lib` で build，実行時は `sudo ldconfig` か `LD_LIBRARY_PATH=/usr/local/lib`（build.rs は rpath を付けない，UR-35）．
   - **未確認**：UHD 4.10 以外の版（Linux での build と構造体サイズは上の Docker 環境の UHD 4.10 で確認済み）．`src/uhd.rs` は UHD 4.10 の header に対して手書きで 69 関数を宣言しているので，古い UHD（Ubuntu 22.04 の apt は 4.1 系）で link error が出たら，足りない関数名を記録し，UHD を 4.6 以上にするか owner に相談する．`tests/uhd_api.rs` の構造体サイズ（40/40/48/40）は arm64 macOS で測り，x86_64 Linux の UHD 4.10（Docker）でも一致した．別の UHD 版で違ったら **実機に進まず** 記録して止まる（C API の構造体が合わないと未定義動作）．
3. ネットワークと装置（bench.md B0.2–B0.3）：X310 の 10 GbE port 0 を `192.168.40.2`，host NIC `192.168.40.1/24`，MTU 9000，`sudo sysctl -w net.core.rmem_max=33554432 net.core.wmem_max=33554432`．`uhd_find_devices --args addr=192.168.40.2`．FPGA image が合わなければ `uhd_image_loader`．master clock が 200 MHz でなければ device string に `master_clock_rate=200e6` を足す（UR-5 が 184.32 MHz を拒否する）．以下 `ARGS` はこの device string．
4. ソフトだけの確認（RF なし）：
   ```sh
   export CARGO_TARGET_DIR=$HOME/.cache/cargo-target/Ez-SDRv4
   cargo test --workspace                                  # 845 passed になるはず（libuhd を link しない）
   cargo +1.85.0 test --workspace                          # 同じ（MSRV）
   cargo test -p ezsdr-radio-uhd --features uhd            # fake 98（うち OBX 1枚の rehearsal 4，CBX 1枚の 1）+ uhd_api 4 が通り，hardware の 10 本は ignored
   cargo clippy -p ezsdr-radio-uhd -p ezsdr-server --features ezsdr-radio-uhd/uhd,ezsdr-server/uhd --all-targets -- -D warnings
   cargo build --release -p ezsdr-server --features uhd    # B7 用の server
   ```
   `tests/fake.rs` は wall-clock のテストが多い（1 回 33 s ほど）．負荷の高い機械で flaky なら記録（Review L・M で 2 本直した）．

**1. 記録ファイル**

`plan/phase7/bench-results.md` を作る（GZ-10）．冒頭に：装置の製品名（X300 か X310 か，`pp_string` に出る），commit hash，OS と kernel，CPU，NIC，`uhd_config_info --version`，FPGA image，`ARGS`，RF の配線（減衰量）．各段ごとに：実行したコマンド，合否，出力（`--nocapture` の `B<n> …` 行はそのまま貼る），bench.md の「Records」列の値．最後に `plan/spikes/2026-09-26-uhd.md` 末尾の表を埋める．

**2. 実行順（bench.md の表どおり，順序に意味がある）**

各段は `EZSDR_UHD_ARGS=$ARGS cargo test --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture`．テスト本体は `crates/ezsdr-radio-uhd/tests/hardware.rs`，B3–B7 の中身は `tests/common/mod.rs` の `rehearse_*`（`tests/fake.rs` が同じ関数を `FakeDevice` で毎回走らせているので，fake で通っていることは確認済み）．

| 段 | テスト | RF | 注意（実装側の事情） |
|---|---|---|---|
| B1 | `hw_b1_probe` | なし | `B1 profile: x310-obx` と，各 channel の front end 名（`OBX RX`・`OBX TX`）と channel 数（1 + 1 以上）が出ること．`x310-obx` でなければテストが失敗する（UHD の front end 名が `OBX` で始まっていない．`Unknown` なら UHD が 4.9 より古い）：名前を記録して止まる．UHD は空の slot B も unknown board の channel として数えるので，2 + 2 と出るのが正常．送信を始める前に，各 TX channel の周波数と利得をそのまま記録する（何も設定しない Session が引き継ぐ値，UR-25） |
| B2 | `hw_b2_authority` | なし | 10 s 待つ．lateness の中央値 < 1 ms，最大 < 25 ms．`DeviceAuthority::failed_reads()`・`discarded_reads()` は Manifest に出ないので，出力に無ければテストに `println!` を足して記録してよい（test コードの追加は可） |
| B3 | `hw_b3_receive_at_t0` | なし | `applied` と `timing` の section が出力される．周波数の read-back 差（K13：< 0.05 Hz 程度）を記録 |
| B4 | `hw_b4_capture_at_a_sample_index` | なし | bench.md は Session の capture と書くが，実装は Spec の schedule で `sink.capture_samples` を sample 50 000 に置く形（`rehearse_capture_at_a_sample_index`）．合格条件（最初の sample が 50 000）は同じ |
| B5 | `hw_b5_overflow` | なし | 300 ms → 1 s → 2 s の stall で overflow を起こす．どれでも起きなければ panic するので，そのときの `net.core.rmem_max` を記録 |
| — | RF 安全 | — | ここから送信する．bench.md「RF safety」：TX/RX → 30 dB 以上の減衰 → RX2（同じ OBX），利得 0 dB，振幅 ≤ 0.5．周波数は `x310-obx` の既定 1 GHz（テストも B7 の `bench-session.json` も envelope は 999–1001 MHz）．ラボの都合で変えるなら，`crates/ezsdr-radio-uhd/src/profile.rs` で `x310-obx` に `x310-cbx` と同じ形の既定周波数を持たせ（10 MHz–8.4 GHz 内），bench.md の 2 つの JSON の envelope を一緒に変え，spec 18 UR-9 も直して commit する（テストの周波数と envelope はこの既定値に従う） |
| B6 | `hw_b6_txrx_and_repeat` | あり | 相関で探すので，fake の「完全一致」は求めない（`exact = false`） |
| B7 | Python（bench.md B7） | あり | `bench-session.json` を bench.md の JSON から作る（`<dir>` を実在の directory に，`args` を `ARGS` に）．`EZSDR_SERVER=$CARGO_TARGET_DIR/release/ezsdr-server EZSDR_PROFILE=bench-session.json PYTHONDONTWRITEBYTECODE=1 python3 python/examples/minimal.py`，次に同じ環境変数で `python3 python/examples/bench_loopback.py`．Rust 版 `hw_b7_session_loopback` もあり，こちらは実機では「`TIME_ERROR` がひとつもない」ことを要求する（fake では負荷のため 5 ms までの send_asap を許している，Review M） |
| B8 | `hw_b8_leads` | あり | **実装は bench.md B8 の一部だけ**：burst を受信後 10, 5, 3, 2, 1.5, 1, 0.5 ms の lead で送り，各回の `TIME_ERROR` と `timing` を出す．表の残り（timed retune の lead，`cold` の restart lead 100/50/25/10 ms，`Stop` 後の送信終端，in-flight window 10/5/3/2 ms での underflow，preemption の bound，timed retune を何個積めるか（queue depth），receive の timed stop が効くか）は未実装．これらは Phase 8 の入力なので，`hardware.rs` に `hw_b8_*` を足して測るか（`#[ignore = "needs a USRP: see plan/phase7/bench.md"]` を付けること，governance の `gz_08` が検査する），測れなかったと記録する |
| B9 | `hw_b9_unplug`（手動），`hw_b9_usrp2_probe` | なし | unplug は 60 s の受信 Run 中に 10 GbE を抜く．`DEVICE_LOST` が約 1 s 以内，termination が `Stopped { policy { DEVICE_LOST } }`．送信だけの Session での unplug は bench.md の表にあるがテストは未実装（`hw_b9_unplug` を真似て足すか記録）．USRP2 は `EZSDR_UHD_ARGS=addr=192.168.10.2` で `hw_b9_usrp2_probe`：UR-5 の拒否（100 MHz）を確認．最後に B3–B7 をもう一度流し，偽の `DEVICE_LOST` が出ないことを確認 |

**3. 実機で特に見てほしい点（コードで INFERRED にしているもの）**

- UR-25：X3x0 が continuous stream の **timed stop** を守るか．守らないと uhd-rx が「cut を越えた sample」を見て untimed stop に切り替え，`timing` に `rx_stop_untimed` が出る．`cold` の rate 変更（B7 の `sample_rate = 19.5e6`）で確認．
- Review M P1-A：遅れた timed stream command が receive metadata の `LATE_COMMAND` で返るか（返れば `timing` に `rx_stop_late`）．
- UR-24：stream command と retune が device の同じ queue に並ぶか（B8 の追加測定）．
- UR-29：抜線でどの UHD error が出るか（`IO`/`USB`/`OS`/`RUNTIME`）．`RUNTIME` が streaming 中に偽陽性を出さないか（B9 の rerun）．
- UR-3：thread 安全性（control call は mutex，streamer は 1 thread 所有）．クラッシュや `double free` のメッセージが出たら最優先で記録．
- EA-17：B7 の `z = sdr.rx.capture(N, at=t)` が `t` から始まるか（`bench_loopback.py` が y と z の相関位置を出す）．
- UR-9（`x310-obx`）：B3 の `applied` で周波数の read-back が 1 GHz に一致するか．B8 の lead と queue に積める timed tune の数は OBX の値として記録する（UBX と同じ MAX2871 系だが tune の実装は別なので，UBX の値とみなさない，design-notes §10）．
- OBX の driver は新しい（UHD 4.9 から）：UHD の警告（特に `OBX` の「Unable to set dboard clock rate - phase will vary」）が出たら記録する．

**4. 終わったら**

1. `plan/phase7/bench-results.md` と `plan/spikes/2026-09-26-uhd.md` の表を commit・push（`docs(phase7): bench session`）．
2. `handoff.md` のこの節を結果で更新（合否，Phase 8 への入力値，未測定の項目）．
3. Gate X の判断材料をそろえて owner に渡す：00-overview §10 の exit criteria（criterion 9 は B0–B8 の合格），`plan/phase7/exit-review/`（OV-3 の rule disposition，まだ作っていない）は Gate X の前に必要．
