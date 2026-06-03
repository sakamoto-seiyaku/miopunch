# Public Network Runbook (P3.5)

本 runbook 记录 `P3.5` 阶段在真实网络（移动网络/家宽/跨区域）做联通性实验的最小口径与可复现命令（不重复 P0 NAT lab 矩阵）。

## 验收口径（必须）

- 两端均出现 `transport.payload_exchanged`（端到端可交换 payload 的硬证据）
- `signaling=mqtt`：两端均出现 “connected to mqtt broker”（或等价 `stage=signaling kind=ok`）
- 交换链路证据：出现 `exchange.ok`（或等价字段）
- 若使用内置 STUN（未显式 `--stun`/`stun:`）：`exchange.ok` 的 kvs 中包含 `selected_view` / `selected_reason`

> 建议：公网实验统一加 `--log-level debug`，便于追溯 cn/global 观测与仲裁证据链。

## 关键开关（P3.5）

- `-4` / `-6`：只约束 **P2P/打洞** 地址族（不限制 signaling / MQTT / enroll / invite / approve）。
- `-u` / `-t`：只约束 **P2P 路径建立** 的网络族；当前 POC v1 是 UDP-only，显式 `-t` / `tcp_only` 会返回 unsupported，不会静默回落到 UDP。
- `--p2p-network auto|udp_only|tcp_only`、`--p2p-ip-family auto|v4|v6`：分别是 `-u/-t`、`-4/-6` 的长参数形式。
- `--builtin-dns-mode auto|on|off`：仅用于 **STUN/MQTT hostname 解析**；默认 `auto`（系统失败才 fallback）。
- `--builtin-dns <ip[:port],...>`：内置 resolver 列表（默认：`1.1.1.1,8.8.8.8,223.5.5.5,119.29.29.29`），查询协议为 `TCP/53`。
- `--stun <addr,...>`：显式指定 STUN 时：
  - 仅使用用户提供的 STUN
  - 不使用内置 STUN
  - 不做 cn/global 视角仲裁
- 显式禁用 STUN（从而禁用 punching）：`--stun ''` 或 YAML `stun: []`

## Build（Host）

```bash
export PATH=/usr/local/go/bin:$PATH
go build -o /tmp/miopunch-lab-host ./cmd/miopunch-lab
```

## Build（Android / arm64；无需 Go 环境）

```bash
export PATH=/usr/local/go/bin:$PATH
CGO_ENABLED=0 GOOS=android GOARCH=arm64 \
  go build -trimpath -ldflags="-s -w" \
  -o /tmp/miopunch-lab-android-arm64 ./cmd/miopunch-lab
```

部署：

```bash
adb push /tmp/miopunch-lab-android-arm64 /data/local/tmp/miopunch-lab
adb shell chmod +x /data/local/tmp/miopunch-lab
```

## Case0：LAN smoke（Host ↔ Android，同一局域网）

目标：先验证“能跑、能交换 payload”，降低公网变量。

Host（client）：

```bash
/tmp/miopunch-lab-host peer client \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker broker.hivemq.com:1883 \
  --mqtt-topic-prefix miopunch/case0 \
  --proxy case0 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --builtin-dns-mode auto \
  --once
```

Android（visitor）：

```bash
adb shell /data/local/tmp/miopunch-lab peer visitor \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker broker.hivemq.com:1883 \
  --mqtt-topic-prefix miopunch/case0 \
  --proxy case0 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --payload case0 \
  --builtin-dns-mode auto
```

说明：

- 若 Android（ADB shell）系统 DNS 不可用，可把两端都改为 `--builtin-dns-mode on`（允许直接使用 hostname）。
- 若想排除 IPv6 P2P 路径：加 `-4`。注意 `-4` 只能保证不选 `direct_ipv6`，同一局域网或同机测试仍可能先成功为 `direct_ipv4`，不等同于强制 `punching_ipv4`。
- 若想固定不走 punching：加 `--stun ''`。

## Case1：Android 移动网络 ↔ Host（公网 punching）

目标：验证真实公网下 punching + transport payload exchange。

Host（client）：

```bash
/tmp/miopunch-lab-host peer client \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker broker.hivemq.com:1883 \
  --mqtt-topic-prefix miopunch/case1 \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --builtin-dns-mode auto \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s \
  --once
```

Android（visitor）：

```bash
adb shell /data/local/tmp/miopunch-lab peer visitor \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker broker.hivemq.com:1883 \
  --mqtt-topic-prefix miopunch/case1 \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --payload case1mobile \
  --builtin-dns-mode auto \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s
```

验收要点：

- `exchange.ok` kvs 包含 `selected_view` / `selected_reason`
- 两端出现：
  - `stage=attempt kind=ok name="attempt.punching.ok"`（或 direct_ipv6）
  - `stage=transport kind=ok name="transport.payload_exchanged"`

## Evidence（cn/global 视角仲裁）

在 `--log-level debug` 下：

- `exchange.ok`：包含最终 `selected_view` / `selected_reason`
- `selected_view` 仅用于裁剪 **STUN 派生的公网 candidates**；不应被理解为对 `direct/local/assisted/portmap` 信息的全局过滤
- `mqtt` 场景：仲裁 debug evidence chain 会出现在 **visitor 侧**（因为 visitor 执行 analysis）
- `coord` 场景：仲裁 debug evidence chain 会出现在 **coordinator 日志**（analysis 在 coordinator 运行）
