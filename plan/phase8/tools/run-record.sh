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

printf '%s\n' 'Before proceeding, confirm the TX/RX → ≥30 dB, 18 GHz-rated attenuator → RX2 path is connected. Type exactly "yes":'
IFS= read -r ATT_CONFIRM || ATT_CONFIRM=''
[[ $ATT_CONFIRM == yes ]] || {
    printf '%s\n' 'run-record: attenuator not confirmed; stopped before Docker or device access' >&2
    exit 1
}

NIC=${EZSDR_BENCH_NIC:-}
[[ -n $NIC ]] || { printf '%s\n' 'run-record: set EZSDR_BENCH_NIC to the single connected X710 port' >&2; exit 2; }
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
    -v "$SRC:/work" -v "$OUT:/bench" -v "$TARGET_VOLUME:/cargo-target" -w /work
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
    -v "$SRC:/work" -v "$OUT:/bench" -v "$TARGET_VOLUME:/cargo-target" -w /work
    -e "EZSDR_UHD_ARGS=$UHD_ARGS"
    -e EZSDR_MEASUREMENT_OUTPUT=/bench/measurement.json
    -e "EZSDR_MEASUREMENT_MBOARD=$MBOARD"
    -e "EZSDR_MEASUREMENT_FPGA_IMAGE=$FPGA_IMAGE"
    -e "EZSDR_MEASUREMENT_HOST_NAME=$HOST_NAME"
    -e "EZSDR_MEASUREMENT_SETTINGS_SCRIPT=$SETTINGS_SCRIPT"
    -e "EZSDR_MEASUREMENT_SETTINGS_VERSION=$SETTINGS_VERSION"
    -e "EZSDR_MEASUREMENT_LOAD=$LOAD"
    -e "EZSDR_MEASUREMENT_COMMIT=$SHA")
for name in EZSDR_MEASURE_STEP_TRIALS EZSDR_MEASURE_OVERFLOW_TRIALS EZSDR_MEASURE_THROUGHPUT_SECS EZSDR_MEASURE_LOOPBACK_TRIALS EZSDR_MEASURE_QUEUE_TRIALS; do
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

if ! "${RECORD_CONTAINER[@]}" "${TEST_COMMAND[@]}" hw_measurements_record -- --exact --ignored --nocapture >"$OUT/hw_measurements_record.log" 2>&1; then
    printf 'run-record: measurement failed; log: %s; record/partial: %s/measurement.json[.partial.json]\n' "$OUT/hw_measurements_record.log" "$OUT" >&2
    exit 1
fi
printf 'hw_measurements_record: passed (record: %s/measurement.json; log: %s/hw_measurements_record.log)\n' "$OUT" "$OUT"
