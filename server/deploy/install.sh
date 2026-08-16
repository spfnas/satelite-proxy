#!/bin/bash
# ============================================================================
# Satelite Proxy — one-click Linux server installer (root)
#
# 用法（远程一键，无需 clone 源码 / Rust 工具链）：
#   curl -fsSL https://raw.githubusercontent.com/spfnas/satelite-proxy/main/server/deploy/install.sh | sudo bash
#
# 或源码树模式（本地已有编译产物时自动复用，无需下载）：
#   sudo bash server/deploy/install.sh
#
# 功能：
#   - 自动下载对应架构的 GitHub Release 包（bin + dist + 透明代理脚本）
#   - 把 release 包解压到 /opt/satelite
#   - 创建专用系统用户 satelite
#   - 安装 systemd 单元 satelite-web.service
#   - TUN：创建 /dev/net/tun 并授予 CAP_NET_ADMIN，sing-box TUN 模式可用
#   - 透明代理（旁路由）：可选安装 nftables 规则（redirect+tproxy），
#     需 SATELITE_TRANSPARENT=1 + LAN 网卡名（默认自动检测）
#   - 默认监听 0.0.0.0:8268，可用 SATELITE_WEB_ADDR 覆盖
#
# 可选环境变量：
#   INSTALL_DIR=/opt/satelite
#   SATELITE_DATA_DIR=/var/lib/satelite
#   SATELITE_WEB_ADDR=0.0.0.0:8268
#   SATELITE_TRANSPARENT=1
#   SATELITE_LAN_IF=eth0
#   RELEASE_TAG=latest            # 指定版本（默认 latest）
#   RELEASE_URL=...               # 完全覆盖下载地址（离线/内网场景）
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
RELEASE_TAG="${RELEASE_TAG:-latest}"

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "!! 不支持架构: $ARCH（仅 x86_64 / aarch64）" >&2; exit 1 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_SRC="${SCRIPT_DIR}/../target/release/satelite-web"
PKG_URL="${RELEASE_URL:-https://github.com/spfnas/satelite-proxy/releases/download/${RELEASE_TAG}/satelite-linux-${ARCH}.tar.gz}"
PKG_NAME="satelite-linux-${ARCH}.tar.gz"

echo "==> 安装到 $INSTALL_DIR (数据 $DATA_DIR, 监听 $WEB_ADDR, arch=$ARCH)"

# ---------- 依赖检查 ----------
if ! command -v curl >/dev/null 2>&1; then
  echo "缺少 curl, 尝试安装..."
  apt-get update -qq && apt-get install -y -qq curl >/dev/null
fi
if ! command -v tar >/dev/null 2>&1; then
  echo "缺少 tar, 尝试安装..."
  apt-get update -qq && apt-get install -y -qq tar >/dev/null
fi
if ! command -v setcap >/dev/null 2>&1; then
  echo "缺少 libcap2-bin (setcap), 尝试安装..."
  apt-get update -qq && apt-get install -y -qq libcap2-bin >/dev/null
fi
if ! command -v nft >/dev/null 2>&1; then
  echo "缺少 nftables, 尝试安装..."
  apt-get update -qq && apt-get install -y -qq nftables >/dev/null
fi

# ---------- 目录 ----------
mkdir -p "$INSTALL_DIR/bin" "$INSTALL_DIR/dist" "$INSTALL_DIR/deploy" "$DATA_DIR/logs"

# ---------- 获取文件（源码树 / 远程 release 二选一）----------
if [ -x "$BIN_SRC" ]; then
  echo "==> 检测到本地编译产物, 复用: $BIN_SRC"
  cp -f "$BIN_SRC" "$INSTALL_DIR/bin/$APP_NAME"
  chmod 755 "$INSTALL_DIR/bin/$APP_NAME"
  DIST_SRC="${SCRIPT_DIR}/../../dist"
  if [ -d "$DIST_SRC" ] && [ -f "$DIST_SRC/index.html" ]; then
    cp -rf "$DIST_SRC/." "$INSTALL_DIR/dist/"
  fi
  NFT_SRC="${SCRIPT_DIR}/nft-transparent.sh"
  if [ -f "$NFT_SRC" ]; then
    cp -f "$NFT_SRC" "$INSTALL_DIR/deploy/"
    chmod 755 "$INSTALL_DIR/deploy/nft-transparent.sh"
  fi
else
  echo "==> 下载 release 包: $PKG_URL"
  TMP_PKG="$(mktemp)"
  if curl -fL --connect-timeout 15 --max-time 300 "$PKG_URL" -o "$TMP_PKG"; then
    :
  else
    echo "!! 直连下载失败，尝试 GitHub 加速镜像..." >&2
    MIRROR_URL="https://ghfast.top/${PKG_URL}"
    curl -fL --connect-timeout 20 --max-time 300 "$MIRROR_URL" -o "$TMP_PKG"
  fi
  tar xzf "$TMP_PKG" -C "$INSTALL_DIR"
  rm -f "$TMP_PKG"
  echo "==> release 包已解压到 $INSTALL_DIR"
fi

# ---------- 安装后完整性检查 ----------
if [ ! -x "$INSTALL_DIR/bin/$APP_NAME" ]; then
  echo "!! $INSTALL_DIR/bin/$APP_NAME 不存在, 安装中止" >&2
  exit 1
fi
chmod 755 "$INSTALL_DIR/bin/$APP_NAME"
chmod 755 "$INSTALL_DIR/deploy/nft-transparent.sh" 2>/dev/null || true

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