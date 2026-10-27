#!/usr/bin/env bash
set -uo pipefail

VERSION='bench-host 1'
NIC=${EZSDR_BENCH_NIC:-}
SUBNET=${EZSDR_BENCH_SUBNET:-40}
APPLY=0

usage() {
    cat <<'EOF'
Usage: bench-host.sh --nic X710-NIC [--subnet OCTET] [--apply]

Read-only verification is the default. --apply changes the named X710 NIC,
its PCIe-link ASPM controls, and three host-wide sysctls; it requires root.
EZSDR_BENCH_NIC and EZSDR_BENCH_SUBNET may supply the values.
EOF
}

fail() { printf 'bench-host: %s\n' "$*" >&2; exit 2; }

while (($#)); do
    case $1 in
        --nic) (($# >= 2)) || fail '--nic needs a value'; NIC=$2; shift 2 ;;
        --subnet) (($# >= 2)) || fail '--subnet needs a value'; SUBNET=$2; shift 2 ;;
        --apply) APPLY=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done

[[ -n $NIC ]] || fail 'name the single X710 port with --nic or EZSDR_BENCH_NIC'
case $NIC in
    enp1s0f0np0|enp1s0f1np1|enp1s0f2np2|enp1s0f3np3) ;;
    *) fail "refusing non-X710 or unrecognized NIC: $NIC" ;;
esac
[[ $SUBNET =~ ^[0-9]{1,3}$ ]] || fail 'subnet must be the 192.168 third octet (0–255)'
((10#$SUBNET <= 255)) || fail 'subnet must be the 192.168 third octet (0–255)'
SUBNET=$((10#$SUBNET))

ADDR="192.168.$SUBNET.10/24"
IP=${ADDR%/*}
MISSING=0
need() {
    if [[ $1 != "$2" ]]; then
        printf 'missing: %s (got %s; need %s)\n' "$3" "${1:-unavailable}" "$2" >&2
        MISSING=1
    fi
}

if [[ ! -d /sys/class/net/$NIC ]]; then
    printf 'missing: NIC %s is not present\n' "$NIC" >&2
    printf '%s nic=%s subnet=192.168.%s.0/24 carrier=unavailable mtu=unavailable addr=unavailable aspm=unavailable rx_ring=unavailable tx_ring=unavailable wmem_max=unavailable rmem_max=unavailable backlog=unavailable nm_managed=unavailable\n' \
        "$VERSION" "$NIC" "$SUBNET"
    exit 1
fi

if ((APPLY)); then
    [[ $(id -u) == 0 ]] || fail '--apply requires root (use sudo)'
fi

CARRIER=$(cat "/sys/class/net/$NIC/carrier" 2>/dev/null || true)
OPERSTATE=$(cat "/sys/class/net/$NIC/operstate" 2>/dev/null || printf unavailable)
MTU=$(cat "/sys/class/net/$NIC/mtu" 2>/dev/null || printf unavailable)
DEFAULT_ROUTE=$(ip route show default dev "$NIC" 2>/dev/null || true)
DEVICE=$(readlink -f "/sys/class/net/$NIC/device" 2>/dev/null || true)
ASPM_PATHS=()
if [[ -n $DEVICE ]]; then
    shopt -s nullglob
    ASPM_PATHS=("${DEVICE%.*}".*/link/*aspm)
    shopt -u nullglob
fi

RINGS=''
if command -v ethtool >/dev/null 2>&1; then RINGS=$(ethtool -g "$NIC" 2>/dev/null || true); fi
RX_MAX=$(awk '/Pre-set maximums:/{section=1;next}/Current hardware settings:/{section=0} section && $1=="RX:"{print $2;exit}' <<<"$RINGS")
TX_MAX=$(awk '/Pre-set maximums:/{section=1;next}/Current hardware settings:/{section=0} section && $1=="TX:"{print $2;exit}' <<<"$RINGS")
RX_RING=$(awk '/Current hardware settings:/{section=1;next} section && $1=="RX:"{print $2;exit}' <<<"$RINGS")
TX_RING=$(awk '/Current hardware settings:/{section=1;next} section && $1=="TX:"{print $2;exit}' <<<"$RINGS")
RX_MAX=${RX_MAX:-unavailable}; TX_MAX=${TX_MAX:-unavailable}
RX_RING=${RX_RING:-unavailable}; TX_RING=${TX_RING:-unavailable}

WMEM=$(sysctl -n net.core.wmem_max 2>/dev/null || printf unavailable)
RMEM=$(sysctl -n net.core.rmem_max 2>/dev/null || printf unavailable)
BACKLOG=$(sysctl -n net.core.netdev_max_backlog 2>/dev/null || printf unavailable)
if command -v nmcli >/dev/null 2>&1; then
    NM_MANAGED=$(LC_ALL=C nmcli -g GENERAL.NM-MANAGED device show "$NIC" 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)
    NM_MANAGED=${NM_MANAGED:-unavailable}
else
    NM_MANAGED=unavailable
fi

ALL_MATCHES=$(ip -4 -o addr show 2>/dev/null | awk -v want="$IP" '{split($4,a,"/"); if (a[1]==want) print $2 ":" $4}')
ADDR_OK=no
ADDR_CONFLICT=''
while IFS= read -r match; do
    [[ -n $match ]] || continue
    if [[ $match == "$NIC:$ADDR" ]]; then ADDR_OK=yes
    elif [[ $match == "$NIC:"* ]]; then :
    else ADDR_CONFLICT=$match
    fi
done <<<"$ALL_MATCHES"

ASPM_VALUES=()
for path in "${ASPM_PATHS[@]}"; do ASPM_VALUES+=("$(cat "$path" 2>/dev/null || printf unavailable)"); done
ASPM=$(IFS=,; printf '%s' "${ASPM_VALUES[*]:-unavailable}")

if ((APPLY)); then
    BLOCKED=0
    if [[ $CARRIER != 1 ]]; then printf 'bench-host: %s has no carrier; refusing to configure it\n' "$NIC" >&2; BLOCKED=1; fi
    if [[ -n $DEFAULT_ROUTE ]]; then printf 'bench-host: %s has a default route; refusing to configure it\n' "$NIC" >&2; BLOCKED=1; fi
    if [[ -n $ADDR_CONFLICT ]]; then printf 'bench-host: %s already has %s; refusing a duplicate address\n' "$ADDR_CONFLICT" "$IP" >&2; BLOCKED=1; fi
    if ((${#ASPM_PATHS[@]} == 0)); then printf 'bench-host: no PCIe ASPM controls found for %s\n' "$NIC" >&2; BLOCKED=1; fi
    if [[ $RX_MAX == unavailable || $TX_MAX == unavailable ]]; then printf 'bench-host: ethtool ring maxima unavailable for %s\n' "$NIC" >&2; BLOCKED=1; fi
    if [[ $NM_MANAGED == unavailable ]]; then printf 'bench-host: nmcli cannot verify NetworkManager state\n' >&2; BLOCKED=1; fi
    ((BLOCKED == 0)) || exit 1

    nmcli device set "$NIC" managed no || fail "could not unmanage $NIC in NetworkManager"
    ip link set dev "$NIC" mtu 9000 up || fail "could not set MTU/up on $NIC"
    ip address replace "$ADDR" dev "$NIC" || fail "could not set $ADDR on $NIC"
    ethtool -G "$NIC" rx "$RX_MAX" tx "$TX_MAX" || fail "could not set maximum rings on $NIC"
    for path in "${ASPM_PATHS[@]}"; do printf '0\n' >"$path" || fail "could not disable $(basename "$path") at $path"; done
    sysctl -qw net.core.wmem_max=33554432 net.core.rmem_max=50000000 net.core.netdev_max_backlog=30000 || fail 'could not set sysctl values'

    CARRIER=$(cat "/sys/class/net/$NIC/carrier" 2>/dev/null || true)
    OPERSTATE=$(cat "/sys/class/net/$NIC/operstate" 2>/dev/null || printf unavailable)
    MTU=$(cat "/sys/class/net/$NIC/mtu" 2>/dev/null || printf unavailable)
    RINGS=$(ethtool -g "$NIC" 2>/dev/null || true)
    RX_RING=$(awk '/Current hardware settings:/{section=1;next} section && $1=="RX:"{print $2;exit}' <<<"$RINGS")
    TX_RING=$(awk '/Current hardware settings:/{section=1;next} section && $1=="TX:"{print $2;exit}' <<<"$RINGS")
    RX_RING=${RX_RING:-unavailable}; TX_RING=${TX_RING:-unavailable}
    WMEM=$(sysctl -n net.core.wmem_max 2>/dev/null || printf unavailable)
    RMEM=$(sysctl -n net.core.rmem_max 2>/dev/null || printf unavailable)
    BACKLOG=$(sysctl -n net.core.netdev_max_backlog 2>/dev/null || printf unavailable)
    NM_MANAGED=$(LC_ALL=C nmcli -g GENERAL.NM-MANAGED device show "$NIC" 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)
    NM_MANAGED=${NM_MANAGED:-unavailable}
    ALL_MATCHES=$(ip -4 -o addr show 2>/dev/null | awk -v want="$IP" '{split($4,a,"/"); if (a[1]==want) print $2 ":" $4}')
    ADDR_OK=no
    while IFS= read -r match; do [[ $match == "$NIC:$ADDR" ]] && ADDR_OK=yes; done <<<"$ALL_MATCHES"
    ASPM_VALUES=()
    for path in "${ASPM_PATHS[@]}"; do ASPM_VALUES+=("$(cat "$path" 2>/dev/null || printf unavailable)"); done
    ASPM=$(IFS=,; printf '%s' "${ASPM_VALUES[*]:-unavailable}")
fi

need "$CARRIER" 1 'carrier'
need "$OPERSTATE" up 'operstate'
need "$MTU" 9000 'MTU'
need "$ADDR_OK" yes "$ADDR"
if [[ $RX_MAX == unavailable ]]; then printf 'missing: ethtool RX ring maximum is unavailable\n' >&2; MISSING=1
else need "$RX_RING" "$RX_MAX" 'RX ring maximum'
fi
if [[ $TX_MAX == unavailable ]]; then printf 'missing: ethtool TX ring maximum is unavailable\n' >&2; MISSING=1
else need "$TX_RING" "$TX_MAX" 'TX ring maximum'
fi
need "$WMEM" 33554432 'net.core.wmem_max'
need "$RMEM" 50000000 'net.core.rmem_max'
need "$BACKLOG" 30000 'net.core.netdev_max_backlog'
need "$NM_MANAGED" no 'NetworkManager managed state'
[[ -z $DEFAULT_ROUTE ]] || { printf 'missing: no default route on %s\n' "$NIC" >&2; MISSING=1; }
[[ -z $ADDR_CONFLICT ]] || { printf 'missing: %s is also assigned to %s\n' "$IP" "$ADDR_CONFLICT" >&2; MISSING=1; }
if ((${#ASPM_VALUES[@]} == 0)); then
    printf 'missing: PCIe ASPM controls for %s\n' "$NIC" >&2; MISSING=1
else
    for value in "${ASPM_VALUES[@]}"; do need "$value" 0 'PCIe ASPM'; done
fi

printf '%s nic=%s subnet=192.168.%s.0/24 addr=%s carrier=%s mtu=%s aspm=%s rx_ring=%s/%s tx_ring=%s/%s wmem_max=%s rmem_max=%s backlog=%s nm_managed=%s\n' \
    "$VERSION" "$NIC" "$SUBNET" "$ADDR" "${CARRIER:-unavailable}" "$MTU" "$ASPM" "$RX_RING" "$RX_MAX" "$TX_RING" "$TX_MAX" "$WMEM" "$RMEM" "$BACKLOG" "$NM_MANAGED"
exit "$MISSING"
