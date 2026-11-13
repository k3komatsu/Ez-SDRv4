#!/usr/bin/env bash
# T200 (stage 4, D2/KL-5): bare UHD, no Ez-SDR code, transmitting 200 MS/s for 300 s, N runs, to compare with the Module
# (`hw_tx200_hot_path_probe`, `hw_t200_record_stage`; plan/phase8/bench.md, 2026-11-15). Each run is `uhd-repro/tx_repro.cpp`'s case `t200`,
# built and run in the bench container: it sets 1 GHz and gain 0 on slot A, reads them back and refuses to transmit outside 999.5-1000.5 MHz or above 20 dB.
# One line per run says clean / failed (counts) / INCONCLUSIVE, then a summary. Changes no host setting (H0/H1/H2 are the owner's, bench.md).
set -uo pipefail

IMAGE=ezsdr-v4-dev:uhd4.10
TOOL_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/uhd-repro
RUNS=${EZSDR_T200_BARE_RUNS:-3}
SECONDS_PER_RUN=${EZSDR_T200_BARE_SECONDS:-300}
CPUSET=${EZSDR_BENCH_CPUSET:-}
# One bound on a whole run (the build, opening the device, the transmit, the shutdown): the planned seconds and 3 minutes.
BOUND=${EZSDR_T200_BARE_BOUND:-$((SECONDS_PER_RUN + 180))}

usage() {
    cat <<'EOF'
Usage: t200-bare-uhd.sh UHD-ARGS LOG-DIR [LABEL]
       t200-bare-uhd.sh --self-check

Builds uhd-repro/tx_repro.cpp in the bench container and runs `tx_repro --case t200 --rate 200e6 --secs 300` 3 times
(EZSDR_T200_BARE_RUNS and EZSDR_T200_BARE_SECONDS change the count and the length; EZSDR_BENCH_CPUSET, e.g. 4-15, is docker's --cpuset-cpus,
for host setting H2; EZSDR_T200_BARE_BOUND is the bound on one run in seconds, 180 s more than the run's length by default). UHD-ARGS is e.g. addr=192.168.40.36.
Each run's output goes to LOG-DIR/tx_repro-t200-LABEL-N.log. One line per run: its verdict and counts.
  clean         every count is 0, the samples handed to send reach 99 % of rate x seconds, tx_repro exited 0.
  failed        an underflow, an underflow in a packet, a sequence error, a time error or a send timeout was counted.
  INCONCLUSIVE  the RF read-back or a field of the summary is missing, the samples handed are short, tx_repro exited non-zero,
                or the run outlived its bound (the container is then killed and no further run starts).
Ctrl-C, SIGTERM, SIGHUP or SIGQUIT kills the container (SIGKILL: no end of burst, the stream just stops) and starts no later run; exit status 130.
Only a run's bound expiring sends tx_repro a TERM first, so that it ends its burst.
It transmits: the owner confirms the slot A TX/RX -> >=30 dB attenuator -> the same board's RX2 once per invocation, before the first run (type exactly "yes").
EOF
}

# The value of `key` on the last "t200 summary" line of a log, or `?` when it is not there as an integer.
summary_value() { # $1 = key, $2 = log
    local value
    value=$(awk -v key="$1" '/^t200 summary / { for (i = 3; i < NF; i += 2) if ($i == key) v = $(i + 1) } END { print v }' "$2")
    [[ $value =~ ^[0-9]+$ ]] && printf '%s' "$value" || printf '?'
}

# Prints the verdict of one run, `clean`, `failed: ...` or `INCONCLUSIVE: ...`, from its log and tx_repro's exit status.
t200_verdict() { # $1 = log, $2 = exit status
    local log=$1 status=$2 key missing=() fail=()
    local -A v=()
    grep -q '^t200 RF read back: freq ' "$log" || missing+=('RF read-back')
    for key in handed expected timeouts underflow underflow_in_packet seq_error seq_error_in_burst time_error; do
        v[$key]=$(summary_value "$key" "$log")
        [[ ${v[$key]} == '?' ]] && missing+=("$key")
    done
    if ((${#missing[@]})); then printf 'INCONCLUSIVE: no usable %s in the log (exit %s)' "${missing[*]}" "$status"; return; fi
    for key in timeouts underflow underflow_in_packet seq_error seq_error_in_burst time_error; do
        ((${v[$key]} == 0)) || fail+=("$key ${v[$key]}")
    done
    if ((${#fail[@]})); then local IFS=,; printf 'failed: %s' "${fail[*]}"; return; fi
    if ((${v[handed]} * 100 < ${v[expected]} * 99)); then printf 'INCONCLUSIVE: %s samples handed to send of %s expected (< 99 %%)' "${v[handed]}" "${v[expected]}"; return; fi
    if ((status != 0)); then printf 'INCONCLUSIVE: tx_repro exited %s with no failure counted' "$status"; return; fi
    printf 'clean'
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi

# One run: the bounded container, run in the background so that a trapped SIGINT/SIGTERM is taken at once (`wait` returns); on expiry (timeout's 124, or
# 137 when it had to kill) the container is killed by name. `--init` makes tini PID 1, so tx_repro gets the TERM that `timeout` sends and ends its burst.
# Sets RUN_STATUS.
RUN_NAME='' RUN_PID=''
run_one() { # $1 = name, $2 = uhd args, $3 = seconds, $4 = log
    RUN_NAME=$1
    timeout -k 10 "$BOUND" docker run --rm --init --name "$1" --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" \
        ${CPUSET:+--cpuset-cpus="$CPUSET"} --tmpfs /tmp:rw,exec,size=256m -v "$TOOL_DIR:/src:ro" "$IMAGE" \
        bash -c 'g++ -O2 -std=c++17 /src/tx_repro.cpp -o /tmp/tx_repro -luhd -lpthread && exec /tmp/tx_repro "$@"' tx_repro \
        --args "$2" --case t200 --rate 200e6 --secs "$3" --spp 1996 --wave tone >"$4" 2>&1 &
    RUN_PID=$!
    wait "$RUN_PID"
    RUN_STATUS=$?
    RUN_PID=''
    if ((RUN_STATUS == 124 || RUN_STATUS == 137)); then
        timeout 30 docker kill "$1" >/dev/null 2>&1
        printf 't200-bare: EZSDR-INFRA: %s outlived its %s s bound and its container was killed\n' "$1" "$BOUND" >>"$4"
    fi
    RUN_NAME=''
}

# Ctrl-C, TERM, HUP or QUIT: the container is killed (it would otherwise keep transmitting with nothing to stop it), and no later run starts. A signal that
# comes before the container exists makes the kill by name fail, so the docker client is killed and awaited (timeout's -k 10 bounds that) and the
# container removed by name afterwards: that also removes one that was created but not started.
stop_on_signal() {
    printf 't200-bare-uhd: interrupted; killing the container\n' >&2
    [[ -z $RUN_NAME ]] || timeout 30 docker kill "$RUN_NAME" >/dev/null 2>&1
    kill $(jobs -p) 2>/dev/null
    wait
    [[ -z $RUN_NAME ]] || timeout 30 docker rm -f "$RUN_NAME" >/dev/null 2>&1
    exit 130
}

if [[ ${1:-} == --self-check ]]; then
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    fails=0
    ok() { if [[ $2 != "$3" ]]; then printf 'self-check FAIL: %s: got [%s], wanted [%s]\n' "$1" "$2" "$3" >&2; fails=$((fails + 1)); fi; }
    rf='t200 RF read back: freq 1000000000 Hz gain 0.0 dB rate 200000000 antenna TX/RX'
    summary() { # underflow underflow_in_packet seq_error seq_error_in_burst time_error timeouts handed
        printf 't200 summary rate 200000000 secs 300.000 spp 1996 wave tone freq 1000000000 gain 0.0 handed %s expected 60000000000 timeouts %s underflow %s underflow_in_packet %s seq_error %s seq_error_in_burst %s time_error %s burst_ack 1 other 0\n' "$7" "$6" "$1" "$2" "$3" "$4" "$5"
    }
    log=$tmp/log
    verdict() { t200_verdict "$log" "$1"; }
    { echo "$rf"; summary 0 0 0 0 0 0 60000000000; } >"$log"
    ok 'a clean run' "$(verdict 0)" clean
    { echo "$rf"; summary 0 0 0 0 0 0 59400000000; } >"$log"
    ok 'exactly 99 % of the samples is enough' "$(verdict 0)" clean
    { echo "$rf"; summary 0 0 0 0 0 0 59399999999; } >"$log"
    ok 'one sample below 99 % is short' "$(verdict 0)" 'INCONCLUSIVE: 59399999999 samples handed to send of 60000000000 expected (< 99 %)'
    { echo "$rf"; summary 8 0 0 0 0 0 60000000000; } >"$log"
    ok 'underflows fail the run' "$(verdict 0)" 'failed: underflow 8'
    { echo "$rf"; summary 0 2 0 0 0 0 60000000000; } >"$log"
    ok 'an underflow in a packet fails the run' "$(verdict 0)" 'failed: underflow_in_packet 2'
    { echo "$rf"; summary 0 0 1 0 0 0 60000000000; } >"$log"
    ok 'a sequence error fails the run' "$(verdict 0)" 'failed: seq_error 1'
    { echo "$rf"; summary 0 0 0 3 0 0 60000000000; } >"$log"
    ok 'a sequence error in a burst fails the run' "$(verdict 0)" 'failed: seq_error_in_burst 3'
    { echo "$rf"; summary 0 0 0 0 4 0 60000000000; } >"$log"
    ok 'a time error fails the run' "$(verdict 0)" 'failed: time_error 4'
    { echo "$rf"; summary 0 0 0 0 0 5 60000000000; } >"$log"
    ok 'a send timeout fails the run' "$(verdict 0)" 'failed: timeouts 5'
    { echo "$rf"; summary 1 0 0 0 0 2 10; } >"$log"
    ok 'a failure with a short volume is still a failure, both counts shown' "$(verdict 0)" 'failed: timeouts 2,underflow 1'
    { echo "$rf"; summary 0 0 0 0 0 0 60000000000; } >"$log"
    ok 'an exit status of 4 with no failure counted is not clean' "$(verdict 4)" 'INCONCLUSIVE: tx_repro exited 4 with no failure counted'
    { echo "$rf"; summary 0 0 0 0 0 0 60000000000 | sed 's/ underflow_in_packet 0//'; } >"$log"
    ok 'a missing field is unknown, not 0' "$(verdict 0)" 'INCONCLUSIVE: no usable underflow_in_packet in the log (exit 0)'
    { echo "$rf"; summary 0 0 0 0 0 0 60000000000 | sed 's/ time_error 0/ time_error ?/'; } >"$log"
    ok 'a field that is not a number is unknown' "$(verdict 0)" 'INCONCLUSIVE: no usable time_error in the log (exit 0)'
    summary 0 0 0 0 0 0 60000000000 >"$log"
    ok 'no RF read-back is unknown' "$(verdict 0)" 'INCONCLUSIVE: no usable RF read-back in the log (exit 0)'
    printf 't200 RF read back: freq 1000000000 Hz gain 0.0 dB rate 200000000 antenna TX/RX\nt200 ABORT: not transmitting outside 999.5-1000.5 MHz or above 20 dB\n' >"$log"
    ok 'a refusal to transmit leaves no summary' "$(verdict 3)" 'INCONCLUSIVE: no usable handed expected timeouts underflow underflow_in_packet seq_error seq_error_in_burst time_error in the log (exit 3)'
    printf 'no output\n' >"$log"
    ok 'an empty log is unknown' "$(verdict 1)" 'INCONCLUSIVE: no usable RF read-back handed expected timeouts underflow underflow_in_packet seq_error seq_error_in_burst time_error in the log (exit 1)'
    # A stub docker that never returns: `run` logs its name and whether --init was given, then sleeps; `kill` logs and ends the sleeper (the container's death).
    mkdir "$tmp/bin"
    cat >"$tmp/bin/docker" <<STUB
#!/bin/sh
if [ "\$1" = run ]; then
    [ -n "\$STUB_LATE" ] && { echo started >"$tmp/docker-started"; trap '' TERM; sleep 1; }
    n= i=
    while [ \$# -gt 0 ]; do [ "\$1" = --name ] && n=\$2; [ "\$1" = --init ] && i=' init'; shift; done
    echo "run \$n\$i" >>"$tmp/docker-calls"; echo \$\$ >"$tmp/docker-pid"; exec sleep 60
fi
echo "\$@" >>"$tmp/docker-calls"; kill \$(cat "$tmp/docker-pid")
STUB
    chmod +x "$tmp/bin/docker"
    : >"$tmp/docker-calls"
    PATH="$tmp/bin:$PATH" BOUND=1 run_one self-check-t200 addr=none 1 "$tmp/expiry.log"
    ok 'a run past its bound is stopped by timeout' "$RUN_STATUS" 124
    ok 'the container is started with --init and killed by name' "$(cat "$tmp/docker-calls")" $'run self-check-t200 init\nkill self-check-t200'
    ok 'the expiry is in the log as an infrastructure line' "$(grep -c '^t200-bare: EZSDR-INFRA: self-check-t200 outlived its 1 s bound' "$tmp/expiry.log")" 1
    ok 'an expired run is inconclusive' "$(t200_verdict "$tmp/expiry.log" "$RUN_STATUS" | cut -c1-14)" 'INCONCLUSIVE: '
    # The whole script: three runs planned, "yes" given.
    full() { PATH="$tmp/bin:$PATH" EZSDR_T200_BARE_RUNS=3 EZSDR_T200_BARE_SECONDS=1 EZSDR_T200_BARE_BOUND=${1:-60} bash "${BASH_SOURCE[0]}" addr=none "$tmp/logs-$2" lbl; }
    # N3: an expired run stops the series (bench.md's stop rule); the other two runs do not start.
    : >"$tmp/docker-calls"
    echo yes | full 1 expiry >"$tmp/expiry.out" 2>&1
    ok 'an expired run ends the series with one run started' "$(grep -c '^run ' "$tmp/docker-calls")" 1
    ok 'the series says it stopped' "$(grep -c '^T200 bare summary .*INCONCLUSIVE' "$tmp/expiry.out")" 1
    # N1/N5: a signal to the script during a run kills and removes the container and starts no later run; the script exits 130. INT, TERM, HUP and QUIT.
    # `set -m` gives the background job default signal handling (a non-interactive one ignores INT, which could then not be trapped), as a terminal's Ctrl-C does
    # the foreground script; the subshell execs the script, so `$!` is the script's PID and only it is signalled (the container is the script's to stop).
    printf 'yes\n' >"$tmp/yes"
    start() { # $1 = out file, $2 = bound; STUB_LATE is passed on
        exec env PATH="$tmp/bin:$PATH" EZSDR_T200_BARE_RUNS=3 EZSDR_T200_BARE_SECONDS=1 EZSDR_T200_BARE_BOUND="$2" ${STUB_LATE:+STUB_LATE=1} bash "${BASH_SOURCE[0]}" addr=none "$tmp/logs-$1" lbl
    }
    norm() { sed 's/-[0-9]*-1/-PID-1/'; }
    for sig in INT TERM HUP QUIT; do
        : >"$tmp/docker-calls"
        set -m
        (start "sig$sig" 60 <"$tmp/yes" >"$tmp/sig$sig.out" 2>&1) &
        pid=$!
        set +m
        for _ in $(seq 100); do grep -q '^run ' "$tmp/docker-calls" && break; sleep 0.1; done
        kill "-$sig" "$pid"
        for _ in $(seq 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
        if kill -0 "$pid" 2>/dev/null; then kill -KILL "$pid"; ok "SIG$sig stops the script" hung stopped; fi
        wait "$pid"
        ok "SIG$sig: the script exits 130" "$?" 130
        ok "SIG$sig: the container was started once, killed and removed by name" "$(norm <"$tmp/docker-calls")" $'run ezsdr-t200-bare-PID-1 init\nkill ezsdr-t200-bare-PID-1\nrm -f ezsdr-t200-bare-PID-1'
    done
    # N4: a signal before the container exists (the stub's `run` starts late and ignores TERM, so the kill by name finds nothing): the docker client is killed
    # once timeout's -k 10 has passed, and the container that appears meanwhile is removed by name before the script exits.
    : >"$tmp/docker-calls"
    rm -f "$tmp/docker-started"
    set -m
    (STUB_LATE=1 start late 60 <"$tmp/yes" >"$tmp/late.out" 2>&1) &
    pid=$!
    set +m
    for _ in $(seq 100); do [[ -e $tmp/docker-started ]] && break; sleep 0.1; done
    kill -INT "$pid"
    for _ in $(seq 300); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
    if kill -0 "$pid" 2>/dev/null; then kill -KILL "$pid"; ok 'a signal before the container exists stops the script' hung stopped; fi
    wait "$pid"
    ok 'early signal: the script exits 130' "$?" 130
    ok 'early signal: the late container is removed by name' "$(norm <"$tmp/docker-calls" | tail -n 1)" 'rm -f ezsdr-t200-bare-PID-1'
    ((fails == 0)) || exit 1
    printf '%s\n' 't200-bare-uhd --self-check: all cases pass'
    exit 0
fi

if (($# < 2)); then usage >&2; exit 2; fi
UHD_ARGS=$1
LOG_DIR=$2
LABEL=${3:-run}
[[ $RUNS =~ ^[1-9][0-9]*$ && $SECONDS_PER_RUN =~ ^[1-9][0-9]*$ && $BOUND =~ ^[1-9][0-9]*$ ]] || { printf 't200-bare-uhd: EZSDR_T200_BARE_RUNS, _SECONDS and _BOUND must be positive integers\n' >&2; exit 2; }
[[ $LABEL =~ ^[A-Za-z0-9_.-]+$ ]] || { printf 't200-bare-uhd: the label may hold letters, digits, _ . - only\n' >&2; exit 2; }
mkdir -p "$LOG_DIR" || exit 2

printf '%s\n' "Before proceeding, confirm that the slot A TX/RX → ≥30 dB, 18 GHz-rated attenuator → the same board's RX2 path is connected. Type exactly \"yes\":"
read -r CONFIRM
[[ $CONFIRM == yes ]] || { printf 't200-bare-uhd: not confirmed; nothing was started\n' >&2; exit 1; }

trap stop_on_signal INT TERM HUP QUIT
failed=0
verdicts=()
for ((n = 1; n <= RUNS; n++)); do
    log="$LOG_DIR/tx_repro-t200-$LABEL-$n.log"
    [[ ! -e $log ]] || { printf 't200-bare-uhd: %s exists; not overwriting it\n' "$log" >&2; exit 2; }
    run_one "ezsdr-t200-bare-$$-$n" "$UHD_ARGS" "$SECONDS_PER_RUN" "$log"
    verdict=$(t200_verdict "$log" "$RUN_STATUS")
    counts=$(awk '/^t200 summary / { sub(/^t200 summary /, ""); print }' "$log" | tail -n 1)
    printf 'T200 bare run %d: %s (%s) (%s)\n' "$n" "$verdict" "${counts:-no summary line}" "$log"
    verdicts+=("${verdict%%:*}")
    [[ $verdict == clean ]] || failed=1
    # bench.md's stop rule: a run that outlived its bound means the device or the host is not trusted; no further run starts.
    [[ $RUN_STATUS == 124 || $RUN_STATUS == 137 ]] && { stopped=" (stopped after run $n of $RUNS: it outlived its bound)"; break; }
done
note=''
[[ ${verdicts[*]} == *INCONCLUSIVE* ]] && note=' INCONCLUSIVE: a run is not usable evidence'
note+=${stopped:-}
printf 'T200 bare summary (200 MS/s, %s s, %d runs, label %s): [%s]%s\n' "$SECONDS_PER_RUN" "$RUNS" "$LABEL" "${verdicts[*]}" "$note"
exit "$failed"
