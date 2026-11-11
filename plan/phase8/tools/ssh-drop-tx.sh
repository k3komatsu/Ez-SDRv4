#!/usr/bin/env bash
set -euo pipefail

IMAGE=ezsdr-v4-dev:uhd4.10
TARGET_VOLUME=ezsdr-v4-bench-target
REOPEN_TEST=hw_noise_floor_receive

# Wall-clock time in ns, and the whole seconds left until the instant $1 (ns), rounded up.
now_ns() { date +%s%N; }
secs_left() { local rem=$(($1 - $(now_ns))); printf '%d\n' $(((rem + 999999999) / 1000000000)); }

# One reading of the independent receiver into RX_READING: tone or quiet. The command runs in its own session under timeout (at most
# 10 s, and not past the instant $1, ns). Its output goes to a file, so a background child cannot hold a pipe open, and its process
# group is killed afterwards whether or not the command has exited. A failing, timed-out or silent command is "unavailable".
rx_state() {
    if [[ -z ${EZSDR_DROP_RX_CHECK:-} ]]; then RX_READING=none; return 0; fi
    local budget status=0 pgid
    budget=$(secs_left "$1")
    ((budget > 10)) && budget=10
    ((budget < 1)) && budget=1
    rm -f "$OUT/rx-pgid" "$OUT/rx-out.txt"
    timeout -k 1 "$budget" setsid bash -c 'echo $$ >"$1"; eval "$2"' ezsdr-rx "$OUT/rx-pgid" "$EZSDR_DROP_RX_CHECK" >"$OUT/rx-out.txt" 2>>"$OUT/rx-check.err" || status=$?
    pgid=$(cat "$OUT/rx-pgid" 2>/dev/null || true)
    if [[ $pgid =~ ^[0-9]+$ ]]; then kill -KILL -- "-$pgid" 2>/dev/null || true; fi
    rm -f "$OUT/rx-pgid"  # so the exit cleanup never signals a group id that may have been reused
    RX_READING=unavailable
    if ((status == 0)); then RX_READING=$(tail -n 1 "$OUT/rx-out.txt" 2>/dev/null || true); fi
    case $RX_READING in
        tone|quiet) ;;
        *) RX_READING=unavailable ;;
    esac
}

# E9: how long the tone lasts after the server died with no cleanup. Reads the independent receiver every 0.5 s until it is quiet or
# 60 s have passed since the instant $1 (ns), and prints the measurement; $2 says what happened at $1. Sets E9_ENDED=1 when it read
# quiet. An unreadable reading is never quiet. "Still on" needs the last reading to be a tone (the loop ends at the deadline, so that
# reading is within one poll interval of it); a tone followed by unreadable readings to the deadline is INCONCLUSIVE.
e9_poll() {
    local ref=$1 deadline last_tone='' last_state='' first_quiet='' at
    deadline=$((ref + 60000000000))
    E9_ENDED=0
    while (($(now_ns) < deadline)); do
        rx_state "$deadline"
        at=$((($(now_ns) - ref) / 1000000))
        printf '%s rx_e9=%s at +%s ms\n' "$(date +%T)" "$RX_READING" "$at" | tee -a "$OUT/rx-check.log"
        last_state=$RX_READING
        case $RX_READING in
            tone) last_tone=$at ;;
            quiet) first_quiet=$at; E9_ENDED=1; break ;;
        esac
        sleep 0.5
    done
    local seen="no tone was read after it"
    [[ -z $last_tone ]] || seen="the last tone was read at +$last_tone ms"
    if ((E9_ENDED)) && [[ -n $last_tone ]]; then
        E9="the tone on the independent receiver ended after $2: $seen, the first quiet reading at +$first_quiet ms (the bracket is $((first_quiet - last_tone)) ms wide: the error)"
    elif ((E9_ENDED)); then
        E9="the tone on the independent receiver ended by +$first_quiet ms (upper bound: no tone was read after $2; the last tone was read before it)"
    elif [[ $last_state == tone ]]; then
        E9="the tone on the independent receiver was still on 60 s after $2: $seen"
    elif [[ -n $last_tone ]]; then
        E9="INCONCLUSIVE: the tone was last read at +$last_tone ms and the receiver was unreadable after it; $2"
    else
        E9="INCONCLUSIVE: the independent receiver could not be read within 60 s after $2"
    fi
    printf 'VERDICT E9 %s\n' "$E9"
}

# Whether the container $1 is confirmed stopped, and when. Returns 0 only when docker inspect reads running=false; then STOPPED_NS is
# the instant, in ns, and STOPPED_SRC where it came from: Docker's FinishedAt (the daemon's record of the exit, INFERRED to follow the
# process's death by a few ms), else now, which the caller reads just after the command that stopped it returned (so it is no earlier
# than the death, and the error is that command's latency). Otherwise (still running, or it cannot be inspected) it returns 1 with no
# instant, and STOPPED_STATE says what was read.
stopped_ns() {
    local running='' finished='' ns=''
    STOPPED_NS=''
    STOPPED_STATE=unreadable
    if read -r running finished < <(docker inspect -f '{{.State.Running}} {{.State.FinishedAt}}' "$1" 2>/dev/null); then STOPPED_STATE="running=$running"; fi
    [[ $running == false ]] || return 1
    ns=$(date -d "$finished" +%s%N 2>/dev/null || true)
    if [[ $ns =~ ^[0-9]+$ ]]; then
        STOPPED_NS=$ns
        STOPPED_SRC="the container's FinishedAt"
    else
        STOPPED_NS=$(now_ns)
        STOPPED_SRC="the return of the command that stopped it"
    fi
}

# E9 for the container $1, which a command has just tried to stop ($2 says which): polls only after the stop is confirmed. Otherwise it
# prints INCONCLUSIVE, no measurement, and warns that the device may still be transmitting. Sets E9_ENDED as e9_poll does.
e9_after_stop() {
    E9_ENDED=0
    if stopped_ns "$1"; then
        e9_poll "$STOPPED_NS" "$2, counted from $STOPPED_SRC"
    else
        printf 'VERDICT E9 INCONCLUSIVE: the container was not confirmed stopped: state %s; this is no measurement\n' "$STOPPED_STATE"
        printf '%s\n' "ssh-drop-tx: the server container $1 may still be running, so the device may still be transmitting: tell the owner" >&2
    fi
}

# --self-check: stopped_ns, e9_poll and e9_after_stop against a stubbed docker, receiver, clock and sleep. No hardware, docker or ssh,
# no real waiting, and no verdict that depends on the wall clock.
self_check() (
    OUT=$(mktemp -d)
    trap 'rm -rf "$OUT"' EXIT
    local fails=0 naps=0 fin=1794358923456789012 mode='' script=''
    CLOCK=1000000000000000000
    REF=0
    now_ns() { printf '%s\n' "$CLOCK"; }
    sleep() { CLOCK=$((CLOCK + 500000000)); ((++naps < 1000)) || { printf '%s\n' 'self-check FAIL: a poll loop did not end' >&2; exit 1; }; }
    rx_state() { RX_READING=$($script $(((CLOCK - REF) / 1000000))); }
    docker() {
        case $mode in
            running) echo 'true 0001-01-01T00:00:00Z' ;;
            fail) return 1 ;;
            stopped) echo 'false 2026-11-11T01:02:03.456789012Z' ;;
            nofinish) echo 'false garbage' ;;
        esac
    }
    ok() { if ! eval "$2"; then printf 'self-check FAIL: %s\n' "$1" >&2; fails=$((fails + 1)); fi; }
    has() { grep -Fq -- "$1" "$OUT/out"; }
    run() { E9_ENDED=7; e9_after_stop c "the stop" >"$OUT/out" 2>"$OUT/err"; : >"$OUT/rx-check.log"; }
    tone_then_quiet() { (($1 < 2000)) && echo tone || echo quiet; }
    tone_then_unreadable() { (($1 < 1000)) && echo tone || echo unavailable; }
    tone_throughout() { echo tone; }
    unreadable_throughout() { echo unavailable; }
    quiet_first() { echo quiet; }
    tone_gap_quiet() { (($1 < 1000)) && echo tone || { (($1 < 40000)) && echo unavailable || echo quiet; }; }

    mode=running; script=tone_throughout; run
    ok 'running=true: no instant' '[[ -z $STOPPED_NS ]]'
    ok 'running=true: INCONCLUSIVE, no polling, no measurement' 'has "VERDICT E9 INCONCLUSIVE: the container was not confirmed stopped: state running=true" && ! has rx_e9 && ((E9_ENDED == 0))'
    ok 'running=true: the live-server warning' 'grep -Fq "may still be transmitting" "$OUT/err"'
    mode=fail; run
    ok 'inspect failure: INCONCLUSIVE, no polling' 'has "not confirmed stopped: state unreadable" && ! has rx_e9 && ((E9_ENDED == 0))'

    mode=stopped; REF=$fin; CLOCK=$((fin + 300000000)); script=tone_then_quiet; run
    ok 'running=false with FinishedAt: that instant' '((STOPPED_NS == fin)) && [[ $STOPPED_SRC == "the container'"'"'s FinishedAt" ]]'
    ok 'tone then quiet: a measurement from that instant' 'has "the last tone was read at +1800 ms, the first quiet reading at +2300 ms" && has "counted from the container'"'"'s FinishedAt" && ((E9_ENDED == 1))'
    mode=nofinish; CLOCK=5000000000; REF=$CLOCK; run
    ok 'running=false, FinishedAt unreadable: the return of the stopping command' '((STOPPED_NS == 5000000000)) && has "the return of the command that stopped it"'

    mode=stopped; REF=$fin; CLOCK=$((fin + 300000000)); script=tone_then_unreadable; run
    ok 'tone then unreadable to the deadline: INCONCLUSIVE with the last tone time' 'has "VERDICT E9 INCONCLUSIVE: the tone was last read at +800 ms and the receiver was unreadable after it" && ! has "still on" && ((E9_ENDED == 0))'
    CLOCK=$((fin + 300000000)); script=tone_throughout; run
    ok 'tone through the last reading: still on' 'has "was still on 60 s after" && has "the last tone was read at +59800 ms" && ((E9_ENDED == 0))'
    CLOCK=$((fin + 300000000)); script=unreadable_throughout; run
    ok 'unreadable throughout: INCONCLUSIVE, never quiet' 'has "could not be read within 60 s" && ! has "ended after" && ((E9_ENDED == 0))'
    CLOCK=$((fin + 300000000)); script=quiet_first; run
    ok 'quiet on the first reading: an upper bound, not a bracket' 'has "ended by +300 ms (upper bound: no tone was read after the stop, counted from" && has "the last tone was read before it" && ! has "ended after" && ! has "wide" && ((E9_ENDED == 1))'
    CLOCK=$((fin + 500000000)); script=tone_gap_quiet; run
    ok 'unreadable readings inside the bracket: its actual width' 'has "the last tone was read at +500 ms, the first quiet reading at +40000 ms (the bracket is 39500 ms wide" && ! has "resolution" && ((E9_ENDED == 1))'

    ((fails == 0)) || exit 1
    printf '%s\n' 'ssh-drop-tx --self-check: all cases pass'
)

usage() {
    cat <<'EOF'
Usage: ssh-drop-tx.sh COMMIT
       ssh-drop-tx.sh --self-check   (the E9 logic against stubs; no hardware, docker or ssh)

Needs EZSDR_UHD_ARGS (the device, e.g. addr=192.168.40.36), EZSDR_BENCH_NIC (the X710 port that
reaches the device, as run-record.sh) and EZSDR_DROP_BOARD (obx for #1, ubx for #2: the board in
slot A, which transmits). The owner confirms the attenuators before the transmission.

The Session is the production path: ezsdr.connect() with the profile ssh-drop-driver.py builds
(the bench binding shape of the Rust test helpers: UHD Module 0.4.0, profile x3xx 0.1.0, slot A's
board, the rx0 feed), an Attached Lease (the server's default), and EZSDR_SERVER set to a wrapper
that starts the lab server from the image over ssh to localhost. The Session transmits a 100 kHz
tone at 1 GHz, amplitude 0.4, until the link is dropped.

The verdicts C1 and C2 are taken before any intervention: the script drops the link (kills the
local ssh client by the PID the relay wrote), waits for the server to exit and for the
independent receiver to go quiet, prints both verdicts, and only then stops a server that is still
running (emergency cleanup).

EZSDR_DROP_WAIT=time|sleep (default time): how the Session waits for the drop. `time` is time.sleep,
the server idle between requests; `sleep` is sdr.sleep, the server inside one `advance` request
(#89). A relay between the client and ssh (ssh-drop-relay.py) logs each request header with its
time to $OUT/to-server.log. In `sleep` mode C1 and C2 read INCONCLUSIVE unless that log shows the
last request before the drop to be the `advance`, written at least 1 s before it and after the
driver's wait-start marker, and unless at least 30 s of the sleep (EZSDR_DROP_HOLD, default 120 s)
were left at the drop: the 15 s deadline plus 15 s.

EZSDR_DROP_CUT=link|kill (default link): `kill` runs `docker kill` on the server container instead
of killing the ssh client, and needs EZSDR_DROP_RX_CHECK. It measures how long the tone on the
independent receiver lasts after the server dies with no cleanup (E9): instead of C1 and C2 it
prints a `VERDICT E9` line (the last tone and the first quiet reading, in ms after the container
stopped, with the bracket's width as the error; `ended by +N ms (upper bound: …)` when the first reading
after the stop was already quiet; or the tone still on 60 s after it, last read as a tone). The instant counted from is the
container's FinishedAt (else the return of the command that stopped it; the line says which), and
only a container confirmed stopped (docker inspect: running=false) gives one. It prints
`VERDICT E9 INCONCLUSIVE` and no measurement when no tone was read before the kill, the container
was not confirmed stopped, or a tone was read but the receiver was unreadable from then to the end
of the 60 s (the line gives the last tone's time). When the tone is still on, or the container was
not confirmed stopped, the device may still be transmitting: tell the
owner. Exit status 0 only when the tone ended and C3 passes. In `link` mode, a server
still running at the verdicts (a FAIL) is stopped by the emergency `docker stop -t 2`, a SIGKILL
after at most 2 s, and the same measurement is taken from the moment it stopped, only once that stop is
confirmed, when the tone was on; a failed emergency stop is reported and makes the exit status non-zero.

EZSDR_DROP_RX_CHECK (optional): a command on an independent receiver (a spectrum analyser, or a
second board cabled through attenuation) whose last output line is "tone" or "quiet". Each call
runs under timeout (at most 10 s) in its own process group, which is killed after the call; a
failing or timed-out call is "unavailable". Without the check,
C2 is NOT DONE. An unreadable receiver before the drop stops the run before the drop. C3 (the
reopen) is skipped only when the receiver's last state is "tone", so the device is never reopened
under a tone.

Verdict lines (C1 and C2 share one deadline, 15 s after the drop, and are judged before any intervention;
`kill` mode prints E9 in their place):
  C1 the server exits when the link drops, and its Manifest records client_disconnect
     (FAIL when the server is still running at the deadline)
  C2 the receiver first reads quiet before the deadline
     (INCONCLUSIVE when there was no tone before the drop, or the last reading failed; FAIL when the
     tone is still there at the deadline, or quiet is first read only after it)
  C3 a new Session on the same device sees no drop tone: under 1 % of its windows have the drop's shape, coherent at 100 kHz (hw_noise_floor_receive)
  E9 a measurement, not a verdict: when the tone ended after the server died unclean (the kill, or the emergency stop)

A stopped tone does not separate a clean end of burst from FPGA starvation, and it does not show
zero RF output (LO leakage). Exit status 0 only when C1, C2 and C3 all PASS (`kill` mode: E9 ended and C3 PASS).
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi
if [[ ${1:-} == --self-check ]]; then self_check; exit; fi
if (($# != 1)); then usage >&2; exit 2; fi
COMMIT_REF=$1
: "${EZSDR_UHD_ARGS:?set EZSDR_UHD_ARGS to the device arguments}"
NIC=${EZSDR_BENCH_NIC:-}
[[ -n $NIC ]] || { printf '%s\n' 'ssh-drop-tx: set EZSDR_BENCH_NIC to the single connected X710 port' >&2; exit 2; }
case ${EZSDR_DROP_BOARD:-} in
    obx|ubx) ;;
    *) printf '%s\n' 'ssh-drop-tx: set EZSDR_DROP_BOARD to obx or ubx (the board in slot A)' >&2; exit 2 ;;
esac
WAIT_MODE=${EZSDR_DROP_WAIT:-time}
case $WAIT_MODE in
    time|sleep) ;;
    *) printf '%s\n' 'ssh-drop-tx: set EZSDR_DROP_WAIT to time or sleep' >&2; exit 2 ;;
esac
CUT=${EZSDR_DROP_CUT:-link}
case $CUT in
    link|kill) ;;
    *) printf '%s\n' 'ssh-drop-tx: set EZSDR_DROP_CUT to link or kill' >&2; exit 2 ;;
esac
if [[ $CUT == kill && -z ${EZSDR_DROP_RX_CHECK:-} ]]; then
    printf '%s\n' 'ssh-drop-tx: EZSDR_DROP_CUT=kill measures the tone after the kill and needs EZSDR_DROP_RX_CHECK' >&2
    exit 2
fi
HOLD=${EZSDR_DROP_HOLD:-120}
command -v python3 >/dev/null 2>&1 || { printf '%s\n' 'ssh-drop-tx: python3 is required (the relay)' >&2; exit 2; }
command -v docker >/dev/null 2>&1 || { printf '%s\n' 'ssh-drop-tx: docker is not installed' >&2; exit 2; }
command -v flock >/dev/null 2>&1 || { printf '%s\n' 'ssh-drop-tx: flock is required to protect the shared Cargo target volume' >&2; exit 2; }
ssh -n -o BatchMode=yes -o ControlMaster=no -o ControlPath=none localhost true >/dev/null 2>&1 || {
    printf '%s\n' 'ssh-drop-tx: ssh to localhost must work without a password (BatchMode); the server runs through it' >&2
    exit 2
}

printf '%s\n' "Before proceeding, confirm that the slot A TX/RX → ≥30 dB, 18 GHz-rated attenuator → the same board's RX2 path is connected, and that a 1 GHz tone at amplitude 0.4 may be transmitted. Type exactly \"yes\":"
IFS= read -r ATT_CONFIRM || ATT_CONFIRM=''
[[ $ATT_CONFIRM == yes ]] || {
    printf '%s\n' 'ssh-drop-tx: attenuator not confirmed; stopped before Docker or device access' >&2
    exit 1
}

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO=$(realpath -e "$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)")
SHA=$(git -C "$REPO" rev-parse --verify "${COMMIT_REF}^{commit}")
SETTINGS_SUMMARY=$("$SCRIPT_DIR/bench-host.sh" --nic "$NIC" --subnet "${EZSDR_BENCH_SUBNET:-40}") || {
    printf '%s\n' 'ssh-drop-tx: host settings are not ready; see bench-host.sh output above' >&2
    exit 1
}
printf '%s\n' "$SETTINGS_SUMMARY"

LOCK_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/ezsdr-bench"
mkdir -p "$LOCK_DIR"
exec 9>"$LOCK_DIR/$TARGET_VOLUME.lock"
flock -n 9 || {
    printf 'ssh-drop-tx: another run is using Docker target volume %s; stopped\n' "$TARGET_VOLUME" >&2
    exit 1
}

DAY_DIR="$HOME/ezsdr-bench/$(date +%F)"
OUT=$(realpath -m "$DAY_DIR/ssh-drop-${SHA:0:12}-$(date +%H%M%S%N)")
case "$OUT" in
    "$REPO"|"$REPO"/*) printf 'ssh-drop-tx: refusing output inside repository: %s\n' "$OUT" >&2; exit 2 ;;
esac
[[ $OUT != *[[:space:]]* ]] || { printf 'ssh-drop-tx: output path has spaces: %s\n' "$OUT" >&2; exit 2; }
mkdir -p "$DAY_DIR"
mkdir "$OUT" "$OUT/src" "$OUT/capture"
printf '%s\n' "$SETTINGS_SUMMARY" >"$OUT/bench-host.txt"
git -C "$REPO" archive --format=tar "$SHA" | tar -xm -C "$OUT/src"
printf 'snapshot=%s\noutput=%s\n' "$SHA" "$OUT"

DRIVER_PID=""
SSH_PID=""
DROP_NAME=ezsdr-drop-$(basename "$OUT")
cleanup() {
    local status=$?
    trap - EXIT INT TERM
    [[ -z $DRIVER_PID ]] || kill "$DRIVER_PID" 2>/dev/null || true
    [[ -z $SSH_PID ]] || kill -KILL "$SSH_PID" 2>/dev/null || true
    # An interrupted rx_state skips its own group kill; its recorded session leader's group may still hold descendants.
    local rx_pgid
    rx_pgid=$(cat "$OUT/rx-pgid" 2>/dev/null || true)
    if [[ $rx_pgid =~ ^[0-9]+$ ]]; then kill -KILL -- "-$rx_pgid" 2>/dev/null || true; fi
    if docker container inspect "$DROP_NAME" >/dev/null 2>&1; then
        docker stop -t 2 "$DROP_NAME" >/dev/null 2>&1 || true
        docker rm -f "$DROP_NAME" >/dev/null 2>&1 || true
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

DOCKER=(docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)"
    --tmpfs /tmp:rw,size=4g -v "$OUT/src:/work" -v "$TARGET_VOLUME:/cargo-target" -w /work
    -e CARGO_HOME=/cargo-target/.cargo-home -e "EZSDR_UHD_ARGS=$EZSDR_UHD_ARGS" "$IMAGE")
if ! "${DOCKER[@]}" cargo build --release -p ezsdr-server --features uhd >"$OUT/build-server.log" 2>&1; then
    printf 'ssh-drop-tx: server build failed; log: %s\n' "$OUT/build-server.log" >&2
    exit 1
fi

export DROP_OUT=$OUT DROP_NAME DROP_PIDFILE=$OUT/ssh.pid DROP_VOLUME=$TARGET_VOLUME DROP_IMAGE=$IMAGE DROP_RELAY=$SCRIPT_DIR/ssh-drop-relay.py
cat >"$OUT/ssh-server.sh" <<'EOF'
#!/usr/bin/env bash
# EZSDR_SERVER for ssh-drop-tx.sh: the lab server in the image, reached over ssh to localhost. The relay writes ssh's pid to
# $DROP_PIDFILE and logs the request headers the client sends to $DROP_OUT/to-server.log.
exec python3 "$DROP_RELAY" "$DROP_OUT/to-server.log" "$DROP_PIDFILE" -- \
    ssh -T -o BatchMode=yes -o ControlMaster=no -o ControlPath=none localhost \
    docker run -i --name "$DROP_NAME" --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$DROP_OUT:/bench" -v "$DROP_VOLUME:/cargo-target:ro" "$DROP_IMAGE" /cargo-target/release/ezsdr-server "$@"
EOF
chmod +x "$OUT/ssh-server.sh"

PY=${EZSDR_DROP_PYTHON:-$HOME/.cache/ezsdr-venv/bin/python}
[[ -x $PY ]] || PY=python3
EZSDR_SERVER=$OUT/ssh-server.sh EZSDR_DROP_OUT=$OUT EZSDR_DROP_HOLD=$HOLD EZSDR_DROP_WAIT=$WAIT_MODE \
    EZSDR_DROP_BOARD=$EZSDR_DROP_BOARD EZSDR_UHD_ARGS=$EZSDR_UHD_ARGS \
    PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=$OUT/src/python \
    "$PY" "$SCRIPT_DIR/ssh-drop-driver.py" >"$OUT/driver.log" 2>&1 &
DRIVER_PID=$!

wait_for_marker() {
    local deadline=$((SECONDS + $2))
    until [[ -e $OUT/$1 ]]; do
        if [[ -e $OUT/driver-done ]]; then return 1; fi
        if ((SECONDS >= deadline)); then return 1; fi
        sleep 0.5
    done
}
if ! wait_for_marker tx-started 60; then
    printf 'ssh-drop-tx: the transmission did not start; driver: %s, log: %s\n' "$(cat "$OUT/driver-done" 2>/dev/null || echo 'no answer')" "$OUT/driver.log" >&2
    exit 1
fi
SSH_PID=$(cat "$DROP_PIDFILE" 2>/dev/null || true)
sleep "${EZSDR_DROP_SETTLE:-5}"

# Sleep mode (#89): C1 and C2 mean something only if the server was inside one `advance` when the link dropped, with enough of the
# sleep left. The relay's log is the evidence (the server cannot be asked): ENTRY is empty when the evidence is there, else the reason.
# Entry itself is INFERRED from the gap: an idle server takes a frame within microseconds, and it skips only a frame read after the end.
entry_check() {
    ENTRY=''
    [[ $WAIT_MODE == sleep ]] || return 0
    local stamp op body last_stamp='' last_op='' wait_start left
    : >>"$OUT/to-server.log"
    while IFS=$'\t' read -r stamp op body; do
        if [[ $stamp =~ ^[0-9]+$ ]] && ((stamp <= $1)); then last_stamp=$stamp; last_op=$op; fi
    done <"$OUT/to-server.log"
    wait_start=$(cat "$OUT/wait-start" 2>/dev/null || true)
    if [[ ! $wait_start =~ ^[0-9]+$ ]]; then
        ENTRY='the driver wrote no wait-start marker'
    elif [[ -z $last_stamp ]]; then
        ENTRY='no request header reached the relay before the drop'
    elif [[ $last_op != advance ]]; then
        ENTRY="the last request before the drop was $last_op, not advance"
    elif ((last_stamp < wait_start)); then
        ENTRY='the last advance was written before the wait began'
    elif (($1 - last_stamp < 1000000000)); then
        ENTRY="the advance had been written only $((($1 - last_stamp) / 1000000)) ms before the drop"
    else
        left=$(awk -v hold="$HOLD" -v ms="$((($1 - wait_start) / 1000000))" 'BEGIN { printf "%d", hold - ms / 1000 }')
        if ((left < 30)); then ENTRY="the sleep had only $left s left at the drop (30 s are needed: the 15 s deadline and 15 s more)"; fi
    fi
}

BEFORE=none
if [[ -n ${EZSDR_DROP_RX_CHECK:-} ]]; then
    BASE_DEADLINE_NS=$(($(now_ns) + 10000000000))
    BEFORE=unavailable
    while (($(now_ns) < BASE_DEADLINE_NS)); do
        rx_state "$BASE_DEADLINE_NS"
        BEFORE=$RX_READING
        if [[ $BEFORE == tone ]]; then break; fi
        sleep 0.5
    done
    if [[ $BEFORE == unavailable ]]; then
        printf '%s baseline: the independent receiver could not be read; stopped before the drop\n' "$(date +%T)" | tee -a "$OUT/rx-check.log" >&2
        exit 1
    fi
fi
printf '%s rx_before_drop=%s\n' "$(date +%T)" "$BEFORE" | tee -a "$OUT/rx-check.log"

SESSION=$(cat "$OUT/session-dir")
MANIFEST=$OUT/${SESSION#/bench/}/manifest.json
SERVER_PID=$(docker inspect -f '{{.State.Pid}}' "$DROP_NAME" 2>/dev/null || echo 0)
DROP_NS=$(now_ns)
DEADLINE_NS=$((DROP_NS + 15000000000))
since_drop() { printf '%s' $((($(now_ns) - DROP_NS) / 1000000000)); }
if [[ $CUT == kill ]]; then
    printf '%s kill: docker kill of server container pid=%s session=%s wait=%s\n' "$(date +%T)" "$SERVER_PID" "$SESSION" "$WAIT_MODE" | tee -a "$OUT/drop.log"
    KILL_OK=1
    docker kill "$DROP_NAME" >/dev/null 2>&1 || KILL_OK=0
else
    printf '%s drop: ssh client pid=%s server container pid=%s session=%s wait=%s\n' "$(date +%T)" "$SSH_PID" "$SERVER_PID" "$SESSION" "$WAIT_MODE" | tee -a "$OUT/drop.log"
    kill -KILL "$SSH_PID" 2>/dev/null || true
    SSH_PID=""
fi
entry_check "$DROP_NS"
if [[ $WAIT_MODE == sleep ]]; then
    printf '%s entry evidence: %s\n' "$(date +%T)" "${ENTRY:-complete}" | tee -a "$OUT/drop.log"
fi

if [[ $CUT == link ]]; then
    # C1 and C2 are judged against this one deadline, before any intervention. The loop only decides when to stop looking: C1 reads the
    # server's exit time from Docker (the wrapper keeps the stopped container), so it does not depend on when the loop looked.
    SERVER_GONE=0
    QUIET_IN_TIME=0
    RX_LAST=$BEFORE
    RX_LAST_AT=0
    while (($(now_ns) < DEADLINE_NS)); do
        if ((!SERVER_GONE)) && [[ $(docker inspect -f '{{.State.Running}}' "$DROP_NAME" 2>/dev/null) != true ]]; then
            SERVER_GONE=1
            printf '%s server container not running at +%s s\n' "$(date +%T)" "$(since_drop)" | tee -a "$OUT/drop.log"
        fi
        if [[ -n ${EZSDR_DROP_RX_CHECK:-} ]] && ((!QUIET_IN_TIME)); then
            rx_state "$DEADLINE_NS"
            RX_LAST=$RX_READING
            RX_LAST_AT=$(since_drop)
            printf '%s rx_after_drop=%s at +%s s\n' "$(date +%T)" "$RX_LAST" "$RX_LAST_AT" | tee -a "$OUT/rx-check.log"
            if [[ $RX_LAST == quiet ]] && (($(now_ns) < DEADLINE_NS)); then QUIET_IN_TIME=1; fi
        fi
        if ((SERVER_GONE)) && { ((QUIET_IN_TIME)) || [[ -z ${EZSDR_DROP_RX_CHECK:-} ]]; }; then break; fi
        sleep 0.5
    done

    docker inspect "$DROP_NAME" >"$OUT/server-container.json" 2>/dev/null || true
    SERVER_STATE=$(docker inspect -f '{{.State.Running}} {{.State.FinishedAt}}' "$DROP_NAME" 2>/dev/null || true)
    read -r SERVER_RUNNING SERVER_FINISHED <<<"$SERVER_STATE" || true
    EXIT_NS=''
    if [[ $SERVER_RUNNING == false ]]; then EXIT_NS=$(date -d "$SERVER_FINISHED" +%s%N 2>/dev/null || true); fi
    if [[ -z $SERVER_RUNNING ]]; then
        C1="INCONCLUSIVE: no state for the server container $DROP_NAME"
    elif [[ $SERVER_RUNNING == true ]]; then
        if (($(now_ns) >= DEADLINE_NS)); then
            C1="FAIL: server container $DROP_NAME still running 15 s after the link dropped"
        else
            C1="INCONCLUSIVE: server container $DROP_NAME still running when the check ended, before the 15 s deadline"
        fi
    elif [[ -z $EXIT_NS ]]; then
        C1="INCONCLUSIVE: the server container's exit time is unreadable: $SERVER_FINISHED"
    elif ((EXIT_NS < DROP_NS)); then
        C1="INCONCLUSIVE: the server container exited before the link dropped ($SERVER_FINISHED)"
    elif ((EXIT_NS > DEADLINE_NS)); then
        C1="FAIL: server container $DROP_NAME still running 15 s after the link dropped; it exited at +$(((EXIT_NS - DROP_NS) / 1000000000)) s"
    elif [[ $SERVER_PID != 0 ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        C1="FAIL: server pid $SERVER_PID is still alive after the drop"
    elif [[ -e $MANIFEST ]] && grep -Fq '"client_disconnect"' "$MANIFEST" && grep -Eq '"kind": ?"attached"' "$MANIFEST"; then
        C1="PASS: server pid $SERVER_PID exited at +$(((EXIT_NS - DROP_NS) / 1000000000)) s and its Manifest records client_disconnect under an attached Lease"
    else
        C1="INCONCLUSIVE: the server exited but no attached Manifest with client_disconnect at $MANIFEST"
    fi
    if [[ -n $ENTRY ]]; then C1="INCONCLUSIVE: no evidence that the server was inside advance at the drop: $ENTRY (it would have read ${C1%%:*})"; fi
    printf 'VERDICT C1 the server ends the Session when the link drops: %s\n' "$C1"

    if [[ -z ${EZSDR_DROP_RX_CHECK:-} ]]; then
        C2="NOT DONE: EZSDR_DROP_RX_CHECK is unset; no independent receiver"
    elif [[ $BEFORE != tone ]]; then
        C2="INCONCLUSIVE: no tone on the independent receiver before the drop ($BEFORE)"
    elif ((QUIET_IN_TIME)); then
        C2="PASS: the tone went quiet on the independent receiver at +$RX_LAST_AT s, within 15 s of the drop and before any intervention"
    elif [[ $RX_LAST == unavailable ]]; then
        C2="INCONCLUSIVE: the independent receiver could not be read at +$RX_LAST_AT s, the end of the 15 s wait"
    elif [[ $RX_LAST == quiet ]]; then
        C2="FAIL: the independent receiver first read quiet at +$RX_LAST_AT s, after the 15 s deadline"
    else
        C2="FAIL: the tone was still on the independent receiver at +$RX_LAST_AT s, the last reading before the 15 s deadline"
    fi
    if [[ -n $ENTRY ]]; then C2="INCONCLUSIVE: no evidence that the server was inside advance at the drop: $ENTRY (it would have read ${C2%%:*})"; fi
    printf 'VERDICT C2 the transmission stops when the link drops: %s\n' "$C2"
else
    C1=''
    C2=''
    docker inspect "$DROP_NAME" >"$OUT/server-container.json" 2>/dev/null || true
    KILLED_STATE=$(docker inspect -f 'running={{.State.Running}} exit={{.State.ExitCode}}' "$DROP_NAME" 2>/dev/null || echo unreadable)
    printf '%s server container after the kill: %s\n' "$(date +%T)" "$KILLED_STATE" | tee -a "$OUT/drop.log"
    E9_ENDED=0
    if ((!KILL_OK)); then
        printf 'VERDICT E9 INCONCLUSIVE: the docker kill failed (state %s); this is no measurement\n' "$KILLED_STATE"
    elif [[ $BEFORE != tone ]]; then
        printf 'VERDICT E9 INCONCLUSIVE: no tone before the kill (%s); this is no measurement\n' "$BEFORE"
    else
        e9_after_stop "$DROP_NAME" "the death of the server container by docker kill"
    fi
    # The relay reaps ssh once the remote container is gone; the cleanup must not signal that pid after it is free.
    for _ in $(seq 50); do kill -0 "$SSH_PID" 2>/dev/null || { SSH_PID=""; break; }; sleep 0.1; done
fi

kill "$DRIVER_PID" 2>/dev/null || true
DRIVER_PID=""
STOP_FAILED=0
if docker container inspect "$DROP_NAME" >/dev/null 2>&1; then
    printf '%s emergency docker stop of %s, after the verdicts were recorded\n' "$(date +%T)" "$DROP_NAME" | tee -a "$OUT/drop.log"
    if ! docker stop -t 2 "$DROP_NAME" >/dev/null 2>&1; then
        STOP_FAILED=1
        printf '%s\n' "ssh-drop-tx: the emergency docker stop of $DROP_NAME failed: the server may still be running and the device may still be transmitting: tell the owner" >&2
    fi
    # A server still running at the verdicts dies here, of a SIGKILL after at most 2 s: an unclean death, so the receiver tells E9 too,
    # but only once the container is confirmed stopped.
    if [[ $CUT == link && ${SERVER_RUNNING:-} == true && -n ${EZSDR_DROP_RX_CHECK:-} && $RX_LAST == tone ]]; then
        e9_after_stop "$DROP_NAME" "the emergency docker stop of the server container (a SIGKILL after at most 2 s)"
    fi
fi

rx_state "$(($(now_ns) + 10000000000))"
LAST=$RX_READING
printf '%s rx_before_reopen=%s\n' "$(date +%T)" "$LAST" | tee -a "$OUT/rx-check.log"
if [[ $LAST == tone ]]; then
    C3="NOT DONE: not run, because the independent receiver still reports a tone"
elif "${DOCKER[@]}" cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware "$REOPEN_TEST" -- --exact --ignored --nocapture >"$OUT/$REOPEN_TEST.log" 2>&1 \
    && grep -Eq '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$OUT/$REOPEN_TEST.log"; then
    C3="PASS: $(grep -m1 '^VERDICT' "$OUT/$REOPEN_TEST.log" | sed 's/^VERDICT //')"
else
    C3="FAIL: $REOPEN_TEST did not pass; log: $OUT/$REOPEN_TEST.log"
fi
printf 'VERDICT C3 a Session after the drop sees no tone: %s\n' "$C3"

if [[ $CUT == link && $C1 == PASS* && $C2 == PASS* && $C3 == PASS* ]] && ((!STOP_FAILED)); then
    printf 'ssh-drop-tx: all three checks passed; records in %s\n' "$OUT"
    exit 0
fi
if [[ $CUT == kill ]]; then
    if ((E9_ENDED)) && [[ $C3 == PASS* ]] && ((!STOP_FAILED)); then
        printf 'ssh-drop-tx: E9 measured and C3 passed; records in %s\n' "$OUT"
        exit 0
    fi
    ((E9_ENDED)) || printf '%s\n' 'ssh-drop-tx: no E9 measurement (see the VERDICT E9 line); if the tone was still on, the device may still be transmitting: tell the owner' >&2
fi
printf 'ssh-drop-tx: not all checks passed; records in %s\n' "$OUT" >&2
exit 1
