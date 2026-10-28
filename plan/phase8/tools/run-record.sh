#!/usr/bin/env bash
set -euo pipefail

IMAGE=ezsdr-v4-dev:uhd4.10
TARGET_VOLUME=ezsdr-v4-bench-target
SETTINGS_SCRIPT=plan/phase8/tools/bench-host.sh

usage() {
    cat <<'EOF'
Usage: run-record.sh COMMIT UHD-ARGS [hw_test ...]

Always runs hw_measurements_record; optional hardware tests run first as
preflight. Set EZSDR_BENCH_NIC (and optionally EZSDR_BENCH_SUBNET) to select
the single host NIC. Metadata may be supplied with EZSDR_MEASUREMENT_*; the
commit and output are set from the archived snapshot and this run directory.

H4 load (optional): set EZSDR_MEASUREMENT_LOAD=h4 to run the record under
PID-managed CPU and memory-copy workers. EZSDR_MEASUREMENT_H4_CPU (default:
nproc), EZSDR_MEASUREMENT_H4_MEM (default: 2 workers), and
EZSDR_MEASUREMENT_H4_DURATION (optional seconds) configure that load.

Layout and link (stage 3c-3a), all optional:
  EZSDR_MEASUREMENT_SELECTOR  a JSON object of rx, tx (layout elements, both or
      neither), image, links and link_gbps only, laid over every profile of the record, e.g.
      '{"rx":["0:A:obx:0","0:B:cbx:0"],"tx":["0:A:obx:0","0:B:cbx:0"]}'.
      Whether it fits the device is judged by the Provider (UR-5, RM-28).
      image, links and link_gbps are one-element lists: "image":["xg"], "links":[2],
      "link_gbps":[10]. Pass the XG device's two addresses in UHD-ARGS
      (addr=...,second_addr=...). The raw probes drive device channel 0 (slot A)
      at 1 GHz: with a selector they run only when its rx and tx channel 0 (each
      when named) are slot A on a UBX or OBX, and otherwise each is recorded as
      inapplicable to the layout and none touches the device.
  EZSDR_MEASUREMENT_NICS      space-separated NICs whose byte counters each
      throughput trial reads (default: EZSDR_BENCH_NIC).
The attenuator confirmation names every transmitting channel of the selector,
and slot A when the raw probes will run (no selector, or a selector whose
channel 0 is slot A or absent) or any preflight test is requested: a single "yes" covers every
board's TX/RX → attenuator → the same board's RX2 (a Basic/LF TX board: the RX
you wired it to). There is no option to skip it.
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi
if (($# < 2)); then usage >&2; exit 2; fi
COMMIT_REF=$1
UHD_ARGS=$2
shift 2
TESTS=("$@")

for test_name in "${TESTS[@]}"; do
    [[ $test_name =~ ^hw_[A-Za-z0-9_]+$ && $test_name != hw_measurements_record ]] || {
        printf 'run-record: invalid preflight hardware test: %s\n' "$test_name" >&2
        exit 2
    }
done
command -v flock >/dev/null 2>&1 || { printf '%s\n' 'run-record: flock is required to protect the shared Cargo target volume' >&2; exit 2; }
[[ ${EZSDR_MEASUREMENT_LOAD:-none} == none || ${EZSDR_MEASUREMENT_LOAD:-none} == h4 ]] || {
    printf 'run-record: EZSDR_MEASUREMENT_LOAD must be none or h4\n' >&2
    exit 2
}

H4_PID=""
H4_EARLY_EXIT=0
H4_LOG=""
H4_RUN_DIR=""
RECORD_PID=""

finish_h4_load() {
    [[ -n $H4_PID ]] || return 0
    local was_alive=0 status=0
    kill -0 "$H4_PID" 2>/dev/null && was_alive=1 || H4_EARLY_EXIT=1
    if ((was_alive)); then kill -TERM "$H4_PID" 2>/dev/null || true; fi
    if wait "$H4_PID"; then status=0; else status=$?; fi
    H4_PID=""
    if ((was_alive)) && ((status != 143)); then H4_EARLY_EXIT=1; fi
    if ! grep -Fq 'H4_STOPPED reason=signal ' "$H4_LOG"; then H4_EARLY_EXIT=1; fi
    if grep -Eq '^H4_WORKER_EXIT |^H4_STOPPED reason=duration |^H4_STOPPED reason=worker_start_failed ' "$H4_LOG"; then H4_EARLY_EXIT=1; fi
    cat /proc/stat >"$H4_RUN_DIR/h4-proc-stat-after.txt" || return 1
    printf 'exit_status=%s\nearly_exit=%s\n' "$status" "$H4_EARLY_EXIT" >>"$H4_RUN_DIR/h4-load.txt"
    if ((H4_EARLY_EXIT)); then
        printf 'run-record: H4 helper or worker ended before the measurement completed; log: %s\n' "$H4_LOG" >&2
        return 1
    fi
    return 0
}

stop_record_container() {
    [[ -n $RECORD_PID ]] || return 0
    docker stop -t 2 "$CONTAINER_NAME" >/dev/null 2>&1 || true
    if kill -0 "$RECORD_PID" 2>/dev/null; then kill -TERM "$RECORD_PID" 2>/dev/null || true; fi
    wait "$RECORD_PID" >/dev/null 2>&1 || true
    docker stop -t 2 "$CONTAINER_NAME" >/dev/null 2>&1 || true
    RECORD_PID=""
}

cleanup_run() {
    local status=$?
    trap - EXIT INT TERM
    if [[ -n $H4_PID ]]; then finish_h4_load || status=1; fi
    stop_record_container
    exit "$status"
}
trap cleanup_run EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

TX_BOARDS='slot A board (raw probes and preflight tests)'
if [[ -n ${EZSDR_MEASUREMENT_SELECTOR+x} ]]; then
    TX_BOARDS=$(PREFLIGHT=$((${#TESTS[@]} > 0)) python3 -I -c '
import json, os, sys
try:
    selector = json.loads(os.environ["EZSDR_MEASUREMENT_SELECTOR"])
    assert isinstance(selector, dict)
    tx = selector.get("tx", [])
    assert isinstance(tx, list) and all(isinstance(e, str) for e in tx)
except Exception:
    sys.exit(1)
if ("rx" in selector) != ("tx" in selector):
    sys.exit(3)
# The raw probes drive device channel 0 (slot A) at 1 GHz; they run unless the rx or tx channel 0 of the selector is something else.
probes = all(not v or v[0] in ("0:A:ubx:0", "0:A:obx:0") for v in (selector.get("rx"), selector.get("tx")))
# Preflight tests (hw_*) run without the selector and drive slot A, so slot A is named whenever one is requested.
slot_a = probes or os.environ.get("PREFLIGHT") == "1"
print(", ".join((["slot A board (raw probes and preflight tests)"] if slot_a else []) + [e for e in tx if not (slot_a and e.startswith("0:A:"))]) or "none")
') || {
        if (($? == 3)); then printf '%s\n' 'run-record: EZSDR_MEASUREMENT_SELECTOR must give both rx and tx, or neither (the missing one would be inherited, and its transmitters not confirmed)' >&2
        else printf '%s\n' 'run-record: EZSDR_MEASUREMENT_SELECTOR must be a JSON object whose tx is a list of strings' >&2; fi
        exit 2
    }
fi
printf '%s\n' "Before proceeding, confirm that EVERY transmitting channel (${TX_BOARDS}) has its board's TX/RX → ≥30 dB, 18 GHz-rated attenuator → the same board's RX2 path connected. Type exactly \"yes\":"
IFS= read -r ATT_CONFIRM || ATT_CONFIRM=''
[[ $ATT_CONFIRM == yes ]] || {
    printf '%s\n' 'run-record: attenuator not confirmed; stopped before Docker or device access' >&2
    exit 1
}

NIC=${EZSDR_BENCH_NIC:-}
[[ -n $NIC ]] || { printf '%s\n' 'run-record: set EZSDR_BENCH_NIC to the single connected X710 port' >&2; exit 2; }
MEASUREMENT_NICS=${EZSDR_MEASUREMENT_NICS:-$NIC}
SUBNET=${EZSDR_BENCH_SUBNET:-40}
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)
REPO=$(realpath -e "$REPO")
SHA=$(git -C "$REPO" rev-parse --verify "${COMMIT_REF}^{commit}")
SHORT_SHA=${SHA:0:12}

SETTINGS_SUMMARY=$("$SCRIPT_DIR/bench-host.sh" --nic "$NIC" --subnet "$SUBNET") || {
    printf '%s\n' 'run-record: host settings are not ready; see bench-host.sh output above' >&2
    exit 1
}
printf '%s\n' "$SETTINGS_SUMMARY"
SETTINGS_VERSION=${SETTINGS_SUMMARY%% nic=*}

LOCK_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/ezsdr-bench"
mkdir -p "$LOCK_DIR"
exec 9>"$LOCK_DIR/$TARGET_VOLUME.lock"
flock -n 9 || {
    printf 'run-record: another run is using Docker target volume %s; stopped\n' "$TARGET_VOLUME" >&2
    exit 1
}

DAY_DIR="$HOME/ezsdr-bench/$(date +%F)"
RUN_NAME="$SHORT_SHA-$(date +%H%M%S%N)"
OUT=$(realpath -m "$DAY_DIR/$RUN_NAME")
case "$OUT" in
    "$REPO"|"$REPO"/*) printf 'run-record: refusing output inside repository: %s\n' "$OUT" >&2; exit 2 ;;
esac
mkdir -p "$DAY_DIR"
mkdir "$OUT"
printf '%s\n' "$SETTINGS_SUMMARY" >"$OUT/bench-host.txt"
SRC="$OUT/src"
mkdir "$SRC"
# Cargo freshness is mtime-based; archive timestamps can leave a newer target artifact looking current.
git -C "$REPO" archive --format=tar "$SHA" | tar -xm -C "$SRC"
printf 'snapshot=%s\noutput=%s\n' "$SHA" "$OUT"

command -v docker >/dev/null 2>&1 || { printf '%s\n' 'run-record: docker is not installed' >&2; exit 2; }
docker image inspect "$IMAGE" >/dev/null 2>&1 || { printf 'run-record: missing Docker image %s\n' "$IMAGE" >&2; exit 2; }

# One container name per target volume: Docker refuses a second container with the same name even if
# this runner was killed and its container survived, so no two builds ever share the volume.
CONTAINER_NAME="ezsdr-bench-$TARGET_VOLUME"
if docker container inspect "$CONTAINER_NAME" >/dev/null 2>&1; then
    printf 'run-record: container %s still exists (an earlier run?); wait for it or remove it\n' "$CONTAINER_NAME" >&2; exit 1
fi
CONTAINER=(docker run --rm --name "$CONTAINER_NAME" --network=host --cap-add=SYS_NICE --ulimit rtprio=99
    --user "$(id -u):$(id -g)"
    --tmpfs /tmp:rw,size=4g
    -v "$SRC:/work" -v "$OUT:/bench" -v "$TARGET_VOLUME:/cargo-target" -w /work
    -e CARGO_HOME=/cargo-target/.cargo-home
    -e "EZSDR_UHD_ARGS=$UHD_ARGS" "$IMAGE")
TEST_COMMAND=(cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware)

if ! "${CONTAINER[@]}" "${TEST_COMMAND[@]}" --no-run >"$OUT/build.log" 2>&1; then
    printf 'run-record: build failed; log: %s\n' "$OUT/build.log" >&2
    exit 1
fi

if ((${#TESTS[@]})); then
    if ! "${CONTAINER[@]}" "${TEST_COMMAND[@]}" -- --list >"$OUT/hardware-tests.list.log" 2>&1; then
        printf 'run-record: could not list hardware tests; log: %s\n' "$OUT/hardware-tests.list.log" >&2
        exit 1
    fi
    for test_name in "${TESTS[@]}"; do
        if ! grep -Fxq "$test_name: test" "$OUT/hardware-tests.list.log"; then
            printf 'run-record: preflight test %s is not in the snapshot test list; log: %s\n' "$test_name" "$OUT/hardware-tests.list.log" >&2
            exit 2
        fi
    done
fi

if ! "${CONTAINER[@]}" uhd_usrp_probe --args "$UHD_ARGS" >"$OUT/probe.log" 2>&1; then
    printf 'run-record: uhd_usrp_probe failed; log: %s\n' "$OUT/probe.log" >&2
    exit 1
fi

probe_value() {
    awk -v key="$1" 'index($0,key) { sub("^.*" key "[[:space:]]*", ""); sub(/[[:space:]]*$/, ""); print; exit }' "$OUT/probe.log"
}
MBOARD=${EZSDR_MEASUREMENT_MBOARD:-$(probe_value 'Mboard:')}
FPGA_VERSION=$(probe_value 'FPGA Version:')
FPGA_HASH=$(probe_value 'FPGA git hash:')
FPGA_IMAGE=${EZSDR_MEASUREMENT_FPGA_IMAGE:-}
if [[ -z $FPGA_IMAGE && -n $FPGA_VERSION ]]; then
    FPGA_IMAGE=$FPGA_VERSION
    [[ -z $FPGA_HASH ]] || FPGA_IMAGE+=" git $FPGA_HASH"
fi
[[ -n $MBOARD ]] || { printf 'run-record: set EZSDR_MEASUREMENT_MBOARD; not found in %s\n' "$OUT/probe.log" >&2; exit 2; }
[[ -n $FPGA_IMAGE ]] || { printf 'run-record: set EZSDR_MEASUREMENT_FPGA_IMAGE; not found in %s\n' "$OUT/probe.log" >&2; exit 2; }
HOST_NAME=${EZSDR_MEASUREMENT_HOST_NAME:-$(hostname -s)}
LOAD=${EZSDR_MEASUREMENT_LOAD:-none}

RECORD_CONTAINER=(docker run --rm --name "$CONTAINER_NAME" --network=host --cap-add=SYS_NICE --ulimit rtprio=99
    --user "$(id -u):$(id -g)"
    --tmpfs /tmp:rw,size=4g
    -v "$SRC:/work" -v "$OUT:/bench" -v "$TARGET_VOLUME:/cargo-target" -w /work
    -e CARGO_HOME=/cargo-target/.cargo-home
    -e "EZSDR_UHD_ARGS=$UHD_ARGS"
    -e EZSDR_MEASUREMENT_OUTPUT=/bench/measurement.json
    -e "EZSDR_MEASUREMENT_MBOARD=$MBOARD"
    -e "EZSDR_MEASUREMENT_FPGA_IMAGE=$FPGA_IMAGE"
    -e "EZSDR_MEASUREMENT_HOST_NAME=$HOST_NAME"
    -e "EZSDR_MEASUREMENT_SETTINGS_SCRIPT=$SETTINGS_SCRIPT"
    -e "EZSDR_MEASUREMENT_SETTINGS_VERSION=$SETTINGS_VERSION"
    -e "EZSDR_MEASUREMENT_LOAD=$LOAD"
    -e "EZSDR_MEASUREMENT_COMMIT=$SHA"
    -e "EZSDR_MEASUREMENT_NICS=$MEASUREMENT_NICS")
if [[ -n ${EZSDR_MEASUREMENT_SELECTOR+x} ]]; then RECORD_CONTAINER+=(-e "EZSDR_MEASUREMENT_SELECTOR=$EZSDR_MEASUREMENT_SELECTOR"); fi
for name in EZSDR_MEASURE_STEP_TRIALS EZSDR_MEASURE_OVERFLOW_TRIALS EZSDR_MEASURE_THROUGHPUT_SECS EZSDR_MEASURE_LOOPBACK_TRIALS EZSDR_MEASURE_QUEUE_TRIALS EZSDR_MEASURE_QUEUE_MAX_TRIALS; do
    if [[ ${!name+x} ]]; then RECORD_CONTAINER+=(-e "$name=${!name}"); fi
done
RECORD_CONTAINER+=("$IMAGE")

for test_name in "${TESTS[@]}"; do
    if ! "${CONTAINER[@]}" "${TEST_COMMAND[@]}" "$test_name" -- --exact --ignored --nocapture >"$OUT/$test_name.log" 2>&1; then
        printf 'run-record: preflight test %s failed; log: %s\n' "$test_name" "$OUT/$test_name.log" >&2
        exit 1
    fi
    if ! grep -Eq '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$OUT/$test_name.log"; then
        printf 'run-record: preflight test %s did not execute exactly one passing test; log: %s\n' "$test_name" "$OUT/$test_name.log" >&2
        exit 1
    fi
    printf 'preflight %s: passed (log: %s)\n' "$test_name" "$OUT/$test_name.log"
done

if [[ $LOAD == h4 ]]; then
    H4_LOAD="$SRC/plan/phase8/tools/h4-load.sh"
    H4_ARGS=()
    [[ ${EZSDR_MEASUREMENT_H4_CPU+x} ]] && H4_ARGS+=(--cpu "$EZSDR_MEASUREMENT_H4_CPU")
    [[ ${EZSDR_MEASUREMENT_H4_MEM+x} ]] && H4_ARGS+=(--mem "$EZSDR_MEASUREMENT_H4_MEM")
    [[ ${EZSDR_MEASUREMENT_H4_DURATION+x} ]] && H4_ARGS+=(--duration "$EZSDR_MEASUREMENT_H4_DURATION")
    H4_LOG="$OUT/h4-load.log"
    H4_RUN_DIR=$OUT
    bash "$H4_LOAD" "${H4_ARGS[@]}" >"$H4_LOG" 2>&1 &
    H4_PID=$!
    printf 'load=h4\ncpu_request=%s\nmem_request=%s\nduration_request=%s\ncontroller_pid=%s\nworker_pids=not_ready\n' \
        "${EZSDR_MEASUREMENT_H4_CPU:-nproc}" "${EZSDR_MEASUREMENT_H4_MEM:-2}" \
        "${EZSDR_MEASUREMENT_H4_DURATION:-unlimited}" "$H4_PID" >"$OUT/h4-load.txt"
    ready=""
    ready_deadline=$((SECONDS + 20))
    while [[ -z $ready ]]; do
        ready=$(grep -m1 '^H4_READY ' "$H4_LOG" || true)
        if [[ -n $ready ]]; then break; fi
        if ! kill -0 "$H4_PID" 2>/dev/null; then
            H4_EARLY_EXIT=1
            finish_h4_load || true
            printf 'run-record: H4 load failed before readiness; log: %s\n' "$H4_LOG" >&2
            exit 1
        fi
        if ((SECONDS >= ready_deadline)); then
            H4_EARLY_EXIT=1
            printf 'run-record: H4 workers did not become ready; log: %s\n' "$H4_LOG" >&2
            exit 1
        fi
        sleep 0.1
    done
    H4_CPU="" H4_MEM="" H4_DURATION="" H4_WORKER_PIDS=""
    for field in $ready; do
        case $field in
            cpu=*) H4_CPU=${field#cpu=} ;;
            mem=*) H4_MEM=${field#mem=} ;;
            duration_s=*) H4_DURATION=${field#duration_s=} ;;
            worker_pids=*) H4_WORKER_PIDS=${field#worker_pids=} ;;
        esac
    done
    [[ -n $H4_CPU && -n $H4_MEM && -n $H4_DURATION && -n $H4_WORKER_PIDS ]] || {
        H4_EARLY_EXIT=1
        printf "run-record: H4 helper's ready line is incomplete; log: %s\n" "$H4_LOG" >&2
        exit 1
    }
    printf 'load=h4\ncpu=%s\nmem=%s\nduration_s=%s\ncontroller_pid=%s\nworker_pids=%s\nready_line=%s\n' \
        "$H4_CPU" "$H4_MEM" "$H4_DURATION" "$H4_PID" "$H4_WORKER_PIDS" "$ready" >"$OUT/h4-load.txt"
    cat /proc/stat >"$OUT/h4-proc-stat-before.txt"
fi

"${RECORD_CONTAINER[@]}" "${TEST_COMMAND[@]}" hw_measurements_record -- --exact --ignored --nocapture >"$OUT/hw_measurements_record.log" 2>&1 &
RECORD_PID=$!
if wait "$RECORD_PID"; then
    RECORD_STATUS=0
else
    RECORD_STATUS=$?
fi
RECORD_PID=""
if ((RECORD_STATUS != 0)); then
    printf 'run-record: measurement failed; log: %s; record/partial: %s/measurement.json[.partial.json]\n' "$OUT/hw_measurements_record.log" "$OUT" >&2
    exit 1
fi
if [[ $LOAD == h4 ]] && ! finish_h4_load; then exit 1; fi
printf 'hw_measurements_record: passed (record: %s/measurement.json; log: %s/hw_measurements_record.log)\n' "$OUT" "$OUT"
