#!/bin/bash
# ============================================================================
# Satelite Proxy — one-click Linux server installer (root)
#
#  - copies the release binary + resources + frontend dist into /opt/satelite
#  - creates a dedicated system user (satelite)
#  - installs a systemd unit (satelite-web.service)
#  - TUN: creates /dev/net/tun and grants the service CAP_NET_ADMIN so
#    sing-box TUN mode works
#  - Transparent: optionally installs nftables rules (redirect+tproxy) so the
#    box can act as a LAN gateway / 旁路由. Requires SATELITE_TRANSPARENT=1
#    and the LAN-facing interface name in SATELITE_LAN_IF (auto-detected).
#  - binds 0.0.0.0:8268 by default; override with SATELITE_WEB_ADDR
#
# Usage:
#   sudo bash install.sh
#   # enable transparent-proxy (旁路由) after install:
#   sudo SATELITE_TRANSPARENT=1 SATELITE_LAN_IF=eth0 bash install.sh
#
# Optional env overrides:
#   SATELITE_WEB_ADDR=0.0.0.0:8268
#   SATELITE_DATA_DIR=/var/lib/satelite
#   SATELITE_TRANSPARENT=1        # also install nftables transparent rules
#   SATELITE_LAN_IF=eth0          # LAN-facing interface (default: auto-detect)
# ============================================================================
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "需要 root 权限: sudo bash install.sh" >&2
  exit 1
fi

# ---------- 配置 ----------
APP_NAME="satelite-web"
INSTALL_DIR="${INSTALL_DIR:-/opt/satelite}"
DATA_DIR="${SATELITE_DATA_DIR:-/var/lib/satelite}"
WEB_ADDR="${SATELITE_WEB_ADDR:-0.0.0.0:8268}"
SERVICE_USER="satelite"
TRANSPARENT="${SATELITE_TRANSPARENT:-0}"
LAN_IF="${SATELITE_LAN_IF:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_SRC="${SCRIPT_DIR}/../target/release/satelite-web"
RES_SRC="${SCRIPT_DIR}/../resources"
DIST_SRC="${SCRIPT_DIR}/../../dist"
NFT_SRC="${SCRIPT_DIR}/nft-transparent.sh"

echo "==> 安装到 $INSTALL_DIR (数据 $DATA_DIR, 监听 $WEB_ADDR)"

# ---------- 依赖检查 ----------
if ! command -v setcap >/dev/null 2>&1; then
  echo "缺少 libcap2-bin (setcap), 尝试安装..."
  apt-get update -qq && apt-get install -y -qq libcap2-bin >/dev/null
fi
if ! command -v nft >/dev/null 2>&1; then
  echo "缺少 nftables, 尝试安装..."
  apt-get update -qq && apt-get install -y -qq nftables >/dev/null
fi

# ---------- 目录 ----------
mkdir -p "$INSTALL_DIR/bin" "$INSTALL_DIR/resources" "$DATA_DIR/logs" "$INSTALL_DIR/deploy"
cp -f "$BIN_SRC" "$INSTALL_DIR/bin/$APP_NAME"
chmod 755 "$INSTALL_DIR/bin/$APP_NAME"

if [ -d "$RES_SRC" ]; then
  cp -rf "$RES_SRC/." "$INSTALL_DIR/resources/"
fi
if [ -d "$DIST_SRC" ] && [ -f "$DIST_SRC/index.html" ]; then
  mkdir -p "$INSTALL_DIR/dist"
  cp -rf "$DIST_SRC/." "$INSTALL_DIR/dist/"
fi

# 附带透明代理 nft 脚本（随包部署，供后续手动启停）
if [ -f "$NFT_SRC" ]; then
  cp -f "$NFT_SRC" "$INSTALL_DIR/deploy/nft-transparent.sh"
  chmod 755 "$INSTALL_DIR/deploy/nft-transparent.sh"
  echo "==> nft-transparent.sh 已部署到 $INSTALL_DIR/deploy/"
fi

# ---------- 专用用户 ----------
if ! id "$SERVICE_USER" >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
  echo "==> 已创建系统用户 $SERVICE_USER"
fi
chown -R "$SERVICE_USER:$SERVICE_USER" "$DATA_DIR" "$INSTALL_DIR/dist" 2>/dev/null || true
chmod -R 755 "$INSTALL_DIR"

# ---------- TUN 设备 ----------
# root 才有权限创建; 失败不致命(非 TUN 模式仍可用)
if mkdir -p /dev/net && [ ! -e /dev/net/tun ]; then
  mknod /dev/net/tun c 10 200 2>/dev/null || true
  chmod 600 /dev/net/tun 2>/dev/null || true
  echo "==> 已创建 /dev/net/tun"
else
  echo "==> /dev/net/tun 已存在或容器禁止创建(非 TUN 模式可用)"
fi

# ---------- systemd ----------
UNIT="/etc/systemd/system/${APP_NAME}.service"
cat > "$UNIT" <<EOF
[Unit]
Description=Satelite Proxy (pure Web)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$SERVICE_USER
Group=$SERVICE_USER
WorkingDirectory=$INSTALL_DIR
Environment=SATELITE_DATA_DIR=$DATA_DIR
Environment=SATELITE_RESOURCE_DIR=$INSTALL_DIR/resources
Environment=SATELITE_WEB_DIST=$INSTALL_DIR/dist
Environment=SATELITE_WEB_ADDR=$WEB_ADDR
ExecStart=$INSTALL_DIR/bin/$APP_NAME
Restart=on-failure
RestartSec=2
# TUN / transparent 需要 CAP_NET_ADMIN + CAP_NET_RAW (redirect/tproxy 原始套接字);
# 非 root 服务靠 capability 提权, 无需把整个服务跑成 root
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "$APP_NAME" >/dev/null
systemctl restart "$APP_NAME"

sleep 1
if systemctl is-active --quiet "$APP_NAME"; then
  echo "==> 已启动: systemctl status $APP_NAME"
  echo "==> 访问: http://<服务器IP>:${WEB_ADDR##*:}/  (默认 8268)"
  echo "==> 健康检查: curl http://127.0.0.1:${WEB_ADDR##*:}/health"
  echo "==> 日志: journalctl -u $APP_NAME -f"
else
  echo "!! 启动失败, 查看日志: journalctl -u $APP_NAME -xe" >&2
  exit 1
fi

# ---------- 透明代理 (旁路由) ----------
if [ "$TRANSPARENT" = "1" ]; then
  if [ -z "$LAN_IF" ]; then
    # 自动检测: 排除 lo/虚拟网卡, 取第一个有公网/局域网私网 IP 的实体网卡
    LAN_IF="$(ip -o link show 2>/dev/null | awk -F': ' '{print $2}' | grep -vE '^(lo|docker|br-|veth|tun|tap|virbr|vbr|lxc)' | head -1)"
    if [ -z "$LAN_IF" ]; then
      LAN_IF="$(route -n 2>/dev/null | awk '$1=="0.0.0.0" {print $8; exit}')"
    fi
  fi
  if [ -z "$LAN_IF" ]; then
    echo "!! 无法检测局域网网卡, 请手动指定 SATELITE_LAN_IF=eth0" >&2
    exit 1
  fi
  echo "==> 启用透明代理 (旁路由): LAN_IF=$LAN_IF"
  LAN_IF="$LAN_IF" bash "$INSTALL_DIR/deploy/nft-transparent.sh" enable
  echo "==> 透明代理 nft 规则已启用 (TCP redirect / UDP tproxy)"
  echo "==> 提示: 局域网设备把网关/DNS 指向本机 IP 即可全局走代理;"
  echo "==>       关闭: sudo bash $INSTALL_DIR/deploy/nft-transparent.sh disable"
fi

echo "==> 安装完成 ✅"