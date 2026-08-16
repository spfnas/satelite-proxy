# Satelite Proxy — Linux 服务器部署

纯 Web 版 satelite（无桌面端），用于在 Linux 服务器/NAS 上做代理网关或旁路由。

## 快速开始（root，远程一键，无需 Rust）

```bash
curl -fsSL https://raw.githubusercontent.com/spfnas/satelite-proxy/main/server/deploy/install.sh | sudo bash
```

脚本会自动：
- 检测架构，下载对应 GitHub Release 包（`satelite-linux-{arch}.tar.gz`：预编译二进制 + 前端 + 透明代理脚本）
- 安装到 `/opt/satelite`，创建专用用户 `satelite`，注册 systemd 服务
- 失败时自动尝试 GitHub 加速镜像（ghfast.top）

安装后访问：`http://<服务器IP>:8268/`，健康检查 `curl http://127.0.0.1:8268/health`。

## 透明代理 / 旁路由

```bash
# 安装时直接启用透明代理 nft 规则（旁路由模式）
sudo SATELITE_TRANSPARENT=1 SATELITE_LAN_IF=eth0 bash -c "$(curl -fsSL https://raw.githubusercontent.com/spfnas/satelite-proxy/main/server/deploy/install.sh)"

# 或后期手动启停
sudo bash /opt/satelite/deploy/nft-transparent.sh enable    # 启用
sudo bash /opt/satelite/deploy/nft-transparent.sh disable   # 关闭
```

透明代理需要 root（nftables 规则由 root 执行）；服务本身以专用用户
`satelite` 运行，通过 `AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW`
让 sing-box 获得建 redirect/tproxy 原始套接字的权限，**无需把整个服务跑成 root**。

## 源码树模式（本地开发 / 自己编译）

```bash
# 1. 构建 release 二进制（需要 Rust 工具链）
cd server && cargo build --release && cd ..
# 2. 安装（root）— 检测到本地编译产物会自动复用，不下载
sudo bash server/deploy/install.sh
```

## 可选环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `INSTALL_DIR` | `/opt/satelite` | 安装目录 |
| `SATELITE_DATA_DIR` | `/var/lib/satelite` | 数据目录 |
| `SATELITE_WEB_ADDR` | `0.0.0.0:8268` | Web 控制面板监听地址 |
| `SATELITE_TRANSPARENT` | `0` | `1` 时启用透明代理 nft 规则 |
| `SATELITE_LAN_IF` | 自动检测 | 面向局域网客户端的网卡 |
| `RELEASE_TAG` | `latest` | 指定版本（如 `v0.1.0`） |
| `RELEASE_URL` | — | 完全覆盖下载地址（离线/内网场景） |

## 升级

```bash
# 直接重跑安装脚本即可（systemd 会重启服务，数据保留在 SATELITE_DATA_DIR）
curl -fsSL https://raw.githubusercontent.com/spfnas/satelite-proxy/main/server/deploy/install.sh | sudo bash
```

## sing-box 内核

首次启动时面板会提示下载内核，或在「设置 → 内核」里手动下载
（从 GitHub SagerNet/sing-box releases 拉取，1.13.x 已实测支持）。