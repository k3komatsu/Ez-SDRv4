#!/usr/bin/env bash
set -euo pipefail

IMAGE=ezsdr-v4-dev:uhd4.10
TARGET_VOLUME=ezsdr-v4-bench-target
REOPEN_TEST=hw_noise_floor_receive

usage() {
    cat <<'EOF'
Usage: ssh-drop-tx.sh COMMIT

Needs EZSDR_UHD_ARGS (the device, e.g. addr=192.168.40.36), EZSDR_BENCH_NIC (the X710 port that
reaches the device, as run-record.sh) and EZSDR_DROP_BOARD (obx for #1, ubx for #2: the board in
slot A, which transmits). The owner confirms the attenuators before the transmission.

The Session is the production path: ezsdr.connect() with the profile ssh-drop-driver.py builds
(the bench binding shape of the Rust test helpers: UHD Module 0.4.0, profile x3xx 0.1.0, slot A's
board, the rx0 feed), an Attached Lease (the server's default), and EZSDR_SERVER set to a wrapper
that starts the lab server from the image over ssh to localhost. The Session transmits a 100 kHz
tone at 1 GHz, amplitude 0.4, until the link is dropped.

The verdicts C1 and C2 are taken before any intervention: the script drops the link (kills the
local ssh client by the PID the wrapper wrote), waits for the server to exit and for the
independent receiver to go quiet, prints both verdicts, and only then stops a server that is still
running (emergency cleanup).

EZSDR_DROP_RX_CHECK (optional): a command on an independent receiver (a spectrum analyser, or a
second board cabled through attenuation) whose last output line is "tone" or "quiet". Each call
runs under timeout (at most 10 s) in its own process group, which is killed after the call; a
failing or timed-out call is "unavailable". Without the check,
C2 is NOT DONE. An unreadable receiver before the drop stops the run before the drop. C3 (the
reopen) is skipped only when the receiver's last state is "tone", so the device is never reopened
under a tone.

Verdict lines (C1 and C2 share one deadline, 15 s after the drop, and are judged before any intervention):
  C1 the server exits when the link drops, and its Manifest records client_disconnect
     (FAIL when the server is still running at the deadline)
  C2 the receiver first reads quiet before the deadline
     (INCONCLUSIVE when there was no tone before the drop, or the last reading failed; FAIL when the
     tone is still there at the deadline, or quiet is first read only after it)
  C3 a new Session on the same device sees no coherent tone (hw_noise_floor_receive)

A stopped tone does not separate a clean end of burst from FPGA starvation, and it does not show
zero RF output (LO leakage). Exit status 0 only when C1, C2 and C3 all PASS.
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi
if (($# != 1)); then usage >&2; exit 2; fi
COMMIT_REF=$1
: "${EZSDR_UHD_ARGS:?set EZSDR_UHD_ARGS to the device arguments}"
NIC=${EZSDR_BENCH_NIC:-}
[[ -n $NIC ]] || { printf '%s\n' 'ssh-drop-tx: set EZSDR_BENCH_NIC to the single connected X710 port' >&2; exit 2; }
case ${EZSDR_DROP_BOARD:-} in
    obx|ubx) ;;
    *) printf '%s\n' 'ssh-drop-tx: set EZSDR_DROP_BOARD to obx or ubx (the board in slot A)' >&2; exit 2 ;;
esac
command -v docker >/dev/null 2>&1 || { printf '%s\n' 'ssh-drop-tx: docker is not installed' >&2; exit 2; }
command -v flock >/dev/null 2>&1 || { printf '%s\n' 'ssh-drop-tx: flock is required to protect the shared Cargo target volume' >&2; exit 2; }
ssh -o BatchMode=yes -o ControlMaster=no -o ControlPath=none localhost true >/dev/null 2>&1 || {
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

export DROP_OUT=$OUT DROP_NAME DROP_PIDFILE=$OUT/ssh.pid DROP_VOLUME=$TARGET_VOLUME DROP_IMAGE=$IMAGE
cat >"$OUT/ssh-server.sh" <<'EOF'
#!/usr/bin/env bash
# EZSDR_SERVER for ssh-drop-tx.sh: the lab server in the image, reached over ssh to localhost.
echo $$ >"$DROP_PIDFILE"
exec ssh -T -o BatchMode=yes -o ControlMaster=no -o ControlPath=none localhost \
    docker run -i --name "$DROP_NAME" --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,size=4g \
    -v "$DROP_OUT:/bench" -v "$DROP_VOLUME:/cargo-target:ro" "$DROP_IMAGE" /cargo-target/release/ezsdr-server "$@"
EOF
chmod +x "$OUT/ssh-server.sh"

PY=${EZSDR_DROP_PYTHON:-$HOME/.cache/ezsdr-venv/bin/python}
[[ -x $PY ]] || PY=python3
EZSDR_SERVER=$OUT/ssh-server.sh EZSDR_DROP_OUT=$OUT EZSDR_DROP_HOLD=${EZSDR_DROP_HOLD:-120} \
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
printf '%s drop: ssh client pid=%s server container pid=%s session=%s\n' "$(date +%T)" "$SSH_PID" "$SERVER_PID" "$SESSION" | tee -a "$OUT/drop.log"
kill -KILL "$SSH_PID" 2>/dev/null || true
SSH_PID=""

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
printf 'VERDICT C2 the transmission stops when the link drops: %s\n' "$C2"

kill "$DRIVER_PID" 2>/dev/null || true
DRIVER_PID=""
if docker container inspect "$DROP_NAME" >/dev/null 2>&1; then
    printf '%s emergency docker stop of %s, after the verdicts were recorded\n' "$(date +%T)" "$DROP_NAME" | tee -a "$OUT/drop.log"
    docker stop -t 2 "$DROP_NAME" >/dev/null 2>&1 || true
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

if [[ $C1 == PASS* && $C2 == PASS* && $C3 == PASS* ]]; then
    printf 'ssh-drop-tx: all three checks passed; records in %s\n' "$OUT"
    exit 0
fi
printf 'ssh-drop-tx: not all checks passed; records in %s\n' "$OUT" >&2
exit 1
