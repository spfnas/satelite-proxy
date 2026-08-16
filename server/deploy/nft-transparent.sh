#!/bin/bash
# ============================================================================
# Satelite transparent proxy — nftables rules for redirect (TCP) + tproxy (UDP)
#
# Steers LAN traffic (except local/private destinations and the proxy host
# itself) into satelite-web's transparent inbounds:
#   - TCP:        REDIRECT → 127.0.0.1:12345 (redirect inbound)
#   - UDP:        TPROXY   → 127.0.0.1:12346 (tproxy inbound)
#
# Usage (root, on the gateway/server):
#   sudo bash nft-transparent.sh enable     # apply rules
#   sudo bash nft-transparent.sh disable    # flush rules
#   sudo bash nft-transparent.sh status     # list rules
#
# Tune variables below to match your LAN / proxy ports.
# ============================================================================
set -euo pipefail

# ---- Config (edit as needed) ------------------------------------------------
# Interface facing LAN clients. Auto-detect when not given: prefer the
# interface holding the default route, fall back to the first real NIC.
if [ -z "${LAN_IF:-}" ]; then
  LAN_IF="$(route -n 2>/dev/null | awk '$1=="0.0.0.0" {print $8; exit}')"
  if [ -z "$LAN_IF" ]; then
    LAN_IF="$(ip -o link show 2>/dev/null | awk -F': ' '{print $2}' | grep -vE '^(lo|docker|br-|veth|tun|tap|virbr|vbr|lxc)' | head -1)"
  fi
  LAN_IF="${LAN_IF:-eth0}"
  echo "==> auto-detected LAN_IF=$LAN_IF"
fi
REDIRECT_PORT="${REDIRECT_PORT:-12345}"   # redirect inbound (TCP)
TPROXY_PORT="${TPROXY_PORT:-12346}"       # tproxy inbound (TCP+UDP)
# IP ranges excluded from proxying (kept direct).
LOCAL_NETS=(192.168.0.0/16 10.0.0.0/8 172.16.0.0/12 127.0.0.0/8 169.254.0.0/16)
# The proxy host itself (server's own LAN IP) — skip to avoid loops.
SERVER_IP="${SERVER_IP:-$(ip -4 addr show dev "$LAN_IF" 2>/dev/null | awk '/inet /{print $2; exit}')}"

TABLE="satelite_tproxy"
CHAIN_PREROUTING="satelite_prerouting"
CHAIN_MARK="satelite_mark"

enable() {
  echo "==> enabling transparent proxy (if=$LAN_IF, tcp→$REDIRECT_PORT, udp→$TPROXY_PORT)"
  # IP rule: packets with our fwmark go to the special routing table.
  ip rule add fwmark 0x1 table 100 2>/dev/null || true
  # Route table 100: default route via the LAN gateway (for tproxy sockets).
  ip route add local 0.0.0.0/0 dev lo table 100 2>/dev/null || true

  nft -f - <<EOF
table inet $TABLE {
  chain $CHAIN_PREROUTING {
    type filter hook prerouting priority mangle; policy accept;

    # Only handle traffic arriving from the LAN.
    iifname "$LAN_IF" accept
    # Skip traffic to the server itself / local nets.
    ip daddr $SERVER_IP accept
$(for n in "${LOCAL_NETS[@]}"; do printf '    ip daddr %s accept\n' "$n"; done)
$(for n in "${LOCAL_NETS[@]}"; do printf '    ip6 daddr %s accept\n' "$n"; done 2>/dev/null || true)
    # DNS to the server itself (dnsmasq / sing-box DNS) stays direct.
    tcp dport 53 accept
    udp dport 53 accept

    # TCP → REDIRECT (original destination preserved by redirect inbound).
    meta l4proto tcp tproxy to 127.0.0.1:$REDIRECT_PORT accept

    # UDP → TPROXY (mark so replies route back).
    meta l4proto udp tproxy to 127.0.0.1:$TPROXY_PORT meta mark set 0x1 accept
  }
}
EOF
  # Also mark TCP packets so their sockets are bound on lo (kernel 5.x+ uses
  # the tproxy mark for replies; REDIRECT handles TCP transparently).
  ip rule add fwmark 0x2 table 100 2>/dev/null || true
  nft -f - <<EOF
table inet $TABLE {
  chain $CHAIN_MARK {
    type filter hook output priority mangle; policy accept
    meta mark set 0x0 accept
  }
}
EOF
  echo "==> done. Verify: sudo nft list table inet $TABLE"
  echo "==> Ensure the app's Transparent toggle is ON and core is running."
}

disable() {
  echo "==> disabling transparent proxy"
  ip rule del fwmark 0x1 table 100 2>/dev/null || true
  ip rule del fwmark 0x2 table 100 2>/dev/null || true
  ip route del local 0.0.0.0/0 dev lo table 100 2>/dev/null || true
  nft delete table inet $TABLE 2>/dev/null || true
  echo "==> done"
}

status() {
  echo "==> ip rules"
  ip rule show | grep -E "fwmark 0x[12]" || echo "  (none)"
  echo "==> nft table"
  nft list table inet $TABLE 2>/dev/null || echo "  (no table)"
}

case "${1:-status}" in
  enable)  enable ;;
  disable) disable ;;
  status)  status ;;
  *) echo "usage: $0 {enable|disable|status}" >&2; exit 1 ;;
esac
