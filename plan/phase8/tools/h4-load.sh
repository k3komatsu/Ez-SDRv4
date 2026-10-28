#!/usr/bin/env bash
set -euo pipefail

BUFFER_MIB=32
MAX_MEM_WORKERS=16
MAX_MEM_BUDGET_MIB=1024
MAX_CPU_WORKERS=256
MAX_DURATION_SECONDS=604800
CPU_WORKERS=""
MEM_WORKERS=2
DURATION_SECONDS=""
SELF_TEST=0
OPTIONS_SEEN=0

usage() {
    cat <<'EOF'
Usage: h4-load.sh [--cpu N] [--mem M] [--duration S]
       h4-load.sh --self-test

Starts CPU busy loops and memory-copy loops (default: nproc CPU workers,
2 memory workers). --mem counts workers; each copies 32 MiB buffers. The
estimated resident-memory budget includes three buffers per memory worker and
16 MiB per Python worker, capped at 1 GiB. Workers are Python processes started
directly by this script and are stopped by their recorded PIDs.
EOF
}

fail() {
    printf 'h4-load: %s\n' "$1" >&2
    exit 2
}

valid_uint() {
    [[ $1 =~ ^[0-9]{1,9}$ ]]
}

while (($#)); do
    case $1 in
        --cpu)
            ((SELF_TEST == 0)) || fail '--self-test does not accept other options'
            (($# >= 2)) || fail '--cpu needs N'
            valid_uint "$2" || fail '--cpu must be a positive integer'
            OPTIONS_SEEN=1
            CPU_WORKERS=$((10#$2))
            shift 2
            ;;
        --mem)
            ((SELF_TEST == 0)) || fail '--self-test does not accept other options'
            (($# >= 2)) || fail '--mem needs M'
            valid_uint "$2" || fail '--mem must be a nonnegative integer'
            OPTIONS_SEEN=1
            MEM_WORKERS=$((10#$2))
            shift 2
            ;;
        --duration)
            ((SELF_TEST == 0)) || fail '--self-test does not accept other options'
            (($# >= 2)) || fail '--duration needs S'
            valid_uint "$2" || fail '--duration must be a positive integer number of seconds'
            OPTIONS_SEEN=1
            DURATION_SECONDS=$((10#$2))
            shift 2
            ;;
        --self-test)
            ((OPTIONS_SEEN == 0 && SELF_TEST == 0)) || fail '--self-test does not accept other options'
            SELF_TEST=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *) fail "unknown argument: $1" ;;
    esac
done

if ((SELF_TEST)); then
    (($# == 0)) || fail '--self-test does not accept other options'
    test_dir="$(mktemp -d "${TMPDIR:-/tmp}/ezsdr-h4-self-test.XXXXXX")"
    test_pid=""
    cleanup_self_test() {
        if [[ -n $test_pid ]]; then
            kill -TERM "$test_pid" 2>/dev/null || true
            wait "$test_pid" 2>/dev/null || true
        fi
        rm -r -- "$test_dir"
    }
    trap cleanup_self_test EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    "$0" --cpu 1 --mem 1 >"$test_dir/load.log" 2>&1 &
    test_pid=$!
    ready=""
    ready_deadline=$((SECONDS + 15))
    while [[ -z $ready ]]; do
        ready=$(grep -m1 '^H4_READY ' "$test_dir/load.log" || true)
        [[ -n $ready ]] && break
        if ! kill -0 "$test_pid" 2>/dev/null; then
            wait "$test_pid" || true
            test_pid=""
            cat "$test_dir/load.log" >&2
            fail 'load exited before workers were ready'
        fi
        if ((SECONDS >= ready_deadline)); then
            kill -TERM "$test_pid" 2>/dev/null || true
            wait "$test_pid" 2>/dev/null || true
            test_pid=""
            cat "$test_dir/load.log" >&2
            fail 'worker readiness timed out'
        fi
        sleep 0.05
    done
    sleep 1
    kill -TERM "$test_pid" 2>/dev/null || true
    if wait "$test_pid"; then
        status=0
    else
        status=$?
    fi
    test_pid=""
    if ((status != 143)); then
        test_pid=""
        cat "$test_dir/load.log" >&2
        fail "signal self-test exited with status $status, expected SIGTERM (143)"
    fi
    stopped=0
    while IFS= read -r line; do
        [[ $line == H4_READY\ * ]] && ready=$line
        [[ $line == 'H4_STOPPED reason=signal '* ]] && stopped=1
    done <"$test_dir/load.log"
    [[ -n $ready && $stopped == 1 ]] || { cat "$test_dir/load.log" >&2; fail 'worker readiness or signal shutdown was not recorded'; }
    worker_list=""
    for field in $ready; do
        case $field in worker_pids=*) worker_list=${field#worker_pids=} ;; esac
    done
    [[ -n $worker_list ]] || fail 'the ready line has no worker PIDs'
    IFS=, read -r -a worker_pids <<<"$worker_list"
    for pid in "${worker_pids[@]}"; do
        [[ $pid =~ ^[0-9]+$ ]] || fail "invalid worker PID in self-test: $pid"
        if kill -0 "$pid" 2>/dev/null; then fail "worker PID $pid survived the self-test"; fi
    done
    printf 'h4-load self-test passed: one-second load stopped and reaped PIDs %s\n' "$worker_list"
    exit 0
fi

if [[ -z $CPU_WORKERS ]]; then
    command -v nproc >/dev/null 2>&1 || fail 'nproc is required to choose the default CPU worker count'
    CPU_WORKERS="$(nproc)"
    valid_uint "$CPU_WORKERS" || fail 'nproc returned an invalid worker count'
fi
((CPU_WORKERS > 0 && CPU_WORKERS <= MAX_CPU_WORKERS)) || fail "--cpu must be in 1..=$MAX_CPU_WORKERS"
((MEM_WORKERS <= MAX_MEM_WORKERS)) || fail "--mem must be in 0..=$MAX_MEM_WORKERS"
if [[ -n $DURATION_SECONDS ]]; then
    ((DURATION_SECONDS > 0 && DURATION_SECONDS <= MAX_DURATION_SECONDS)) || fail "--duration must be in 1..=$MAX_DURATION_SECONDS seconds"
fi
memory_budget_mib=$((MEM_WORKERS * 3 * BUFFER_MIB + (CPU_WORKERS + MEM_WORKERS) * 16))
((memory_budget_mib <= MAX_MEM_BUDGET_MIB)) || fail "estimated memory use exceeds ${MAX_MEM_BUDGET_MIB} MiB"
command -v python3 >/dev/null 2>&1 || fail 'python3 is required'

state_dir="$(mktemp -d "${TMPDIR:-/tmp}/ezsdr-h4-load.XXXXXX")"
worker_pids=()
ended_by=error

stop_workers() {
    local pid
    for pid in "${worker_pids[@]}"; do kill -TERM "$pid" 2>/dev/null || true; done
    for pid in "${worker_pids[@]}"; do wait "$pid" 2>/dev/null || true; done
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    stop_workers
    printf 'H4_STOPPED reason=%s status=%s\n' "$ended_by" "$status"
    rm -r -- "$state_dir"
    exit "$status"
}

trap cleanup EXIT
trap 'ended_by=signal; exit 130' INT
trap 'ended_by=signal; exit 143' TERM

worker_code='import sys, time
mode, ready, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
if mode == "mem":
    source = bytearray(b"x") * size
    target = bytearray(size)
    view = memoryview(target)
with open(ready, "x", encoding="ascii"):
    pass
if mode == "cpu":
    value = 1
    while True:
        value = (value * 1664525 + 1013904223) & 0xffffffff
else:
    while True:
        view[:] = source
'

for ((index = 0; index < CPU_WORKERS; index++)); do
    ready_file="$state_dir/ready-cpu-$index"
    python3 -u -c "$worker_code" cpu "$ready_file" 0 &
    worker_pids+=("$!")
done
for ((index = 0; index < MEM_WORKERS; index++)); do
    ready_file="$state_dir/ready-mem-$index"
    python3 -u -c "$worker_code" mem "$ready_file" "$((BUFFER_MIB * 1024 * 1024))" &
    worker_pids+=("$!")
done

startup_at=$SECONDS
for ((index = 0; index < CPU_WORKERS; index++)); do
    ready_file="$state_dir/ready-cpu-$index"
    while [[ ! -e $ready_file ]]; do
        kill -0 "${worker_pids[index]}" 2>/dev/null || { ended_by=worker_start_failed; fail "CPU worker ${worker_pids[index]} exited before readiness"; }
        ((SECONDS - startup_at < 15)) || { ended_by=worker_start_failed; fail 'CPU worker readiness timed out'; }
        sleep 0.05
    done
done
for ((index = 0; index < MEM_WORKERS; index++)); do
    pid_index=$((CPU_WORKERS + index))
    ready_file="$state_dir/ready-mem-$index"
    while [[ ! -e $ready_file ]]; do
        kill -0 "${worker_pids[pid_index]}" 2>/dev/null || { ended_by=worker_start_failed; fail "memory worker ${worker_pids[pid_index]} exited before readiness"; }
        ((SECONDS - startup_at < 15)) || { ended_by=worker_start_failed; fail 'memory worker readiness timed out'; }
        sleep 0.05
    done
done

worker_list="$(IFS=,; printf '%s' "${worker_pids[*]}")"
printf 'H4_READY controller_pid=%s cpu=%s mem=%s duration_s=%s worker_pids=%s memory_budget_mib=%s\n' \
    "$$" "$CPU_WORKERS" "$MEM_WORKERS" "${DURATION_SECONDS:-none}" "$worker_list" "$memory_budget_mib"
started_at=$SECONDS
while :; do
    for pid in "${worker_pids[@]}"; do
        if ! kill -0 "$pid" 2>/dev/null; then
            status=0
            wait "$pid" || status=$?
            ended_by=worker_exit
            printf 'H4_WORKER_EXIT pid=%s status=%s\n' "$pid" "$status"
            exit 1
        fi
    done
    if [[ -n $DURATION_SECONDS ]] && ((SECONDS - started_at >= DURATION_SECONDS)); then
        ended_by=duration
        exit 0
    fi
    sleep 0.1
done
