#!/usr/bin/env bash
# The ssh drop's independent receiver, read from the monitor's state file (rx-monitor.sh). ssh-drop-tx.sh reads it through
# EZSDR_DROP_RX_CHECK, so it takes the last line's state (tone or quiet) only when that line is at most 1 s old.
set -uo pipefail

usage() {
    cat <<'EOF'
Usage: rx-monitor-state.sh STATE_FILE
       rx-monitor-state.sh --self-check

Prints the state of the last line (tone or quiet) and exits 0 when that line is at most 1 s old. Otherwise prints unavailable
and exits 1: a missing or empty file, an unknown state, a stale or malformed line.
EOF
}

state() {
    local line ms word _ now
    line=$(tail -n 1 -- "$1" 2>/dev/null) || line=''
    read -r ms word _ <<<"$line"
    now=$(date +%s%3N)
    if [[ $ms =~ ^[0-9]+$ && ($word == tone || $word == quiet) ]] && ((ms <= now && now - ms <= 1000)); then
        printf '%s\n' "$word"
        return 0
    fi
    printf '%s\n' unavailable
    return 1
}

self_check() {
    local fails=0 status
    dir=$(mktemp -d /tmp/rx-monitor-state.XXXXXX) || exit 2
    trap 'rm -rf -- "$dir"' EXIT
    # expect NAME AGE_MS LINES... : the file holds one line per LINE (its state word, stamped AGE_MS ago, or a
    # future stamp for a negative AGE_MS); the last one is read. The stamp is taken when the line is written.
    expect() { # $1 name, $2 the age of the last line in ms (negative: a future stamp), $3 the state word, $4 the want, $5 the exit
        local got status=0
        printf '%s %s\n' "$(($(date +%s%3N) - $2))" "$3" >"$dir/state"
        got=$(state "$dir/state") || status=$?
        if [[ $got == "$4" && $status == "$5" ]]; then
            printf 'ok    %s: %s (exit %s)\n' "$1" "$got" "$status"
        else
            printf 'FAIL  %s: got %s (exit %s), want %s (exit %s)\n' "$1" "$got" "$status" "$4" "$5"
            fails=$((fails + 1))
        fi
    }
    expect "fresh tone" 0 tone tone 0
    expect "fresh quiet" 0 quiet quiet 0
    expect "0.9 s old" 900 quiet quiet 0
    expect "1.1 s old" 1100 tone unavailable 1
    expect "a line from the future" -5000 tone unavailable 1
    expect "stale tone" 5000 tone unavailable 1
    expect "unknown" 0 unknown unavailable 1
    printf '' >"$dir/state"
    status=0
    got=$(state "$dir/state") || status=$?
    if [[ $got == unavailable && $status == 1 ]]; then printf 'ok    empty file: unavailable (exit 1)\n'; else printf 'FAIL  empty file: got %s (exit %s)\n' "$got" "$status"; fails=$((fails + 1)); fi
    status=0
    got=$(state "$dir/missing") || status=$?
    if [[ $got == unavailable && $status == 1 ]]; then printf 'ok    missing file: unavailable (exit 1)\n'; else printf 'FAIL  missing file: got %s (exit %s)\n' "$got" "$status"; fails=$((fails + 1)); fi
    ((fails == 0))
}

case ${1:-} in
    --self-check) self_check ;;
    -h | --help) usage ;;
    '' | -*) usage >&2; exit 2 ;;
    *)
        (($# == 1)) || { usage >&2; exit 2; }
        state "$1"
        ;;
esac
