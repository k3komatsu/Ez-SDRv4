#!/usr/bin/env bash
# Starts the ssh drop's independent receiver (hw_rx_tone_monitor) on the board that receives the attenuated transmission, and stops it.
set -euo pipefail

IMAGE=ezsdr-v4-dev:uhd4.10
TARGET_VOLUME=ezsdr-v4-cargo-target
NAME=ezsdr-rx-monitor
PIDFILE=${XDG_CACHE_HOME:-$HOME/.cache}/ezsdr-bench/rx-monitor.pid

usage() {
    cat <<'EOF'
Usage: rx-monitor.sh start [STATE_FILE]
       rx-monitor.sh stop

start: run from the checkout's root with EZSDR_UHD_ARGS set to the receiving board (#2: addr=192.168.47.37). It runs
hw_rx_tone_monitor in the image in the background, in the container NAME, with its pid in PIDFILE, and prints the state file's path.
The state file gets one line every 200 ms: `<unix ms> <tone|quiet|unknown>` (rx-monitor-state.sh reads it). A line says unknown when
its block's last sample is more than 300 ms behind the device's clock. Refuses to start when the state file or its stop file
(`<state>.stop`) exists or the container is running. The first second must be silent; the monitor refuses to start if it is not, and
its log says why. Optional: EZSDR_MONITOR_SECONDS (default 180), EZSDR_MONITOR_HZ (default 1 GHz, the drop's carrier).
The log is next to the state file.

stop: creates the stop file, so the monitor stops and closes its receive, and waits up to 10 s for the container to exit. Only then
(or if it has not exited by then) does it stop the container by its name, and it says which. The state file stays.
EOF
}

die() {
    printf 'rx-monitor: %s\n' "$*" >&2
    exit 2
}

start() {
    : "${EZSDR_UHD_ARGS:?set EZSDR_UHD_ARGS to the receiving board, e.g. addr=192.168.47.37}"
    command -v docker >/dev/null 2>&1 || die 'docker is not installed'
    [[ -f Cargo.toml && -d crates/ezsdr-radio-uhd ]] || die 'run from the checkout root'
    local state=${1:-$HOME/.cache/ezsdr-bench/$(date +%F)/rx-monitor-$(date +%H%M%S).state}
    [[ ! -e $state ]] || die "the state file exists already: $state"
    docker container inspect "$NAME" >/dev/null 2>&1 && die "container $NAME exists: run stop first"
    mkdir -p -- "$(dirname -- "$state")" "$(dirname -- "$PIDFILE")"
    local dir logfile stop
    dir=$(realpath -- "$(dirname -- "$state")")
    state=$dir/$(basename -- "$state")
    logfile=$state.log
    stop=$state.stop
    [[ $state != *[[:space:]]* ]] || die "the state path has spaces: $state"
    [[ ! -e $stop ]] || die "the stop file exists already: $stop"
    docker run --rm --name "$NAME" --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
        -v "$PWD:/work" -v "$TARGET_VOLUME:/cargo-target" -v "$dir:$dir" -w /work \
        -e CARGO_HOME=/cargo-target/.cargo-home -e EZSDR_UHD_ARGS -e EZSDR_MONITOR_OUT="$state" -e EZSDR_MONITOR_STOP="$stop" \
        -e EZSDR_MONITOR_SECONDS -e EZSDR_MONITOR_HZ \
        "$IMAGE" cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware hw_rx_tone_monitor -- --exact --ignored --nocapture --test-threads=1 \
        >"$logfile" 2>&1 &
    printf '%s\n%s\n' "$!" "$stop" >"$PIDFILE"
    sleep 3
    docker container inspect "$NAME" >/dev/null 2>&1 || { tail -n 5 -- "$logfile" >&2; die "the monitor exited at start; log: $logfile"; }
    printf '%s\n' "$state"
    printf 'log: %s\n' "$logfile" >&2
}

stop() {
    local stop_file='' waited=0
    if [[ -f $PIDFILE ]]; then
        stop_file=$(sed -n 2p -- "$PIDFILE")
    fi
    if docker container inspect "$NAME" >/dev/null 2>&1; then
        if [[ -n $stop_file ]]; then
            : >"$stop_file"
            while docker container inspect "$NAME" >/dev/null 2>&1 && ((waited < 10)); do
                sleep 1
                waited=$((waited + 1))
            done
        fi
        if docker container inspect "$NAME" >/dev/null 2>&1; then
            printf 'rx-monitor: %s still running after %s s (no stop file recorded, or it did not exit): stopping it by name; its receive was not shut down cleanly\n' "$NAME" "$waited" >&2
            docker stop -t 10 "$NAME" >/dev/null
        else
            printf 'stopped %s on its stop file after %s s\n' "$NAME" "$waited"
        fi
    else
        printf '%s is not running\n' "$NAME"
    fi
    if [[ -f $PIDFILE ]]; then
        local pid
        pid=$(sed -n 1p -- "$PIDFILE")
        [[ $pid =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null && kill "$pid" 2>/dev/null || true
        rm -f -- "$PIDFILE"
    fi
}

case ${1:-} in
    start) shift; start "$@" ;;
    stop) stop ;;
    -h | --help) usage ;;
    *) usage >&2; exit 2 ;;
esac
