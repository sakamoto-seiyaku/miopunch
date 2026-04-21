# 2026-04-13 Public Verification Cases

本报告记录 `public-verification-cases` 这轮“真实网络/真实设备”验证的运行口径与执行结果。

- 本轮不重复 `P0` 的 NAT 打洞矩阵与既有 lab case，仅记录本轮新增验证（case0-4）相关。
- 当前已完成 `case0`（LAN smoke）与 `case1`（Android 移动网络 ↔ Host）。

## 验收口径（必须）

- 两端均出现 `transport.payload_exchanged`（作为“端到端可交换 payload”的硬证据）
- `signaling=mqtt` 场景：两端均出现 “connected to mqtt broker”（或等价的 `stage=signaling kind=ok`）
- 交换链路证据：出现 `exchange.ok`（或等价字段），证明 visitor 收到 client 信息并回传响应

## 环境（case0）

- Host: WSL2 / Debian 11 (bullseye)
- Android: Pixel 6a（ADB 可用，`su` 可用）
- Host 与 Pixel 同一局域网

Host 构建（便于在本机直接执行 `miopunch-lab`）：

```bash
/usr/local/go/bin/go build -o /tmp/miopunch-lab-host ./cmd/miopunch-lab
```

## Android 编译与部署（不需要 Go 环境）

交叉编译：

```bash
CGO_ENABLED=0 GOOS=android GOARCH=arm64 \
  /usr/local/go/bin/go build -trimpath -ldflags="-s -w" \
  -o /tmp/miopunch-lab-android-arm64 ./cmd/miopunch-lab
```

推送与执行：

```bash
adb -s 28201JEGR0XPAJ push /tmp/miopunch-lab-android-arm64 /data/local/tmp/miopunch-lab
adb -s 28201JEGR0XPAJ shell chmod +x /data/local/tmp/miopunch-lab
adb -s 28201JEGR0XPAJ shell /data/local/tmp/miopunch-lab --help
```

## Case0：LAN smoke（Host ↔ Pixel6a）

### Case0A：`signaling=coord`（PASS，但不覆盖 punching）

说明：

- 本次 `coord` 路径未配置 STUN，日志会出现 `gather.stun.skip`，因此不构成“公网打洞/穿透”的验证。
- case0A 主要是降低变量做 LAN 端到端 smoke。

结果要点（实际观测）：

- 走 `direct_ipv6`（局域网内存在 ULA IPv6，且默认优先尝试 IPv6）
- 两端均出现 `transport.payload_exchanged`

### Case0B：`signaling=mqtt`（PASS，确认“远程交换信令”链路可用）

关键结论：在 Android（ADB shell + `CGO_ENABLED=0`）环境下，**hostname DNS 解析不可用**，因此 MQTT/STUN 必须配置为 **IP**（或改为 Termux 等带 resolv.conf 的环境，或后续做 DNS/Resolver 注入）。

本次成功使用的（IP）示例：

- MQTT broker: `54.36.178.49:1883`（示例：`test.mosquitto.org` 的某个 A 记录）
- STUN: `74.125.250.129:19302`（`stun.l.google.com`） + `111.206.174.3:3478`（`stun.miwifi.com`）

执行命令（两端必须一致：`--proxy/--secret/--user/--mqtt-topic-prefix`）：

Host（client）：

```bash
/tmp/miopunch-lab-host peer client \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case0 \
  --proxy case0 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --stun 74.125.250.129:19302,111.206.174.3:3478 \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 60s \
  --once
```

Pixel（visitor）：

```bash
adb -s 28201JEGR0XPAJ shell /data/local/tmp/miopunch-lab peer visitor \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case0 \
  --proxy case0 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --payload case0mqtt \
  --stun 74.125.250.129:19302,111.206.174.3:3478 \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 60s
```

结果要点（实际观测）：

- 两端出现：
  - `stage=signaling kind=ok msg="connected to mqtt broker"`
  - `stage=exchange kind=ok msg="exchange.ok"`
  - `stage=transport kind=ok name="transport.payload_exchanged"`
- 连接路径：先 `attempt.v6.fail`，随后回退 `attempt.punching.ok`，最终完成 payload 交换

### Case0C：`signaling=mqtt` + 内置 STUN（PASS，验证 cn/global 视角仲裁不影响 LAN assisted）

说明：

- 不显式配置 `--stun`，走 **内置 STUN cn/global 采样 + 视角仲裁**（`selected_view/selected_reason` 可见）。
- Pixel（ADB shell）系统 DNS 仍不可用，但 `P3.5` 已提供 `--builtin-dns-mode on`，因此 **STUN hostname 不再需要预解析为 IP**。

本次命令要点：

- MQTT broker 仍使用 IP：`54.36.178.49:1883`
- STUN：使用内置列表（不传 `--stun`）
- 内置 DNS：`--builtin-dns-mode on`

执行命令（示例）：

Host（client）：

```bash
/tmp/miopunch-lab-host peer client \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case0-20260414-r6 \
  --proxy case0r6 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --builtin-dns-mode on \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 120s \
  --once
```

Pixel（visitor）：

```bash
adb -s 28201JEGR0XPAJ shell /data/local/tmp/miopunch-lab peer visitor \
  --log-level debug \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case0-20260414-r6 \
  --proxy case0r6 --secret case0secret --user case0 \
  --data-proto quic --quic-cc brutal \
  --payload case0r6 \
  --builtin-dns-mode on \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 120s
```

结果要点（实际观测）：

- `exchange.ok` kvs 含 `selected_view=global selected_reason=availability`
- 最终走 `attempt.punching.ok`，且 remote addr 为 `192.168.4.x:*`（LAN assisted 生效）
- 两端均出现 `transport.payload_exchanged`

证据文件：

- Host: `docs/reports/2026-04-14-case0-lan-internalstun.host.jsonl`
- Pixel: `docs/reports/2026-04-14-case0-lan-internalstun.android.jsonl`

## Case0 暴露问题（已确认/已复现）

### 1) Android（ADB shell）DNS 解析不可用：MQTT/STUN 需用 IP

复现（Pixel 侧直接用 hostname 连接 MQTT）：

```text
dial tcp: lookup broker.hivemq.com on [::1]:53: read udp [::1]:45399->[::1]:53: read: connection refused
```

影响：

- `--mqtt-broker` 使用 hostname 将直接失败
- `--stun` 若使用 hostname 也会失败

当前建议（不做代码修改的前提下）：

- 公网实验阶段统一使用 IP 配置（把域名预解析到 A 记录）

后续可选（需要额外 change）：

- 在 Android 环境支持显式 DNS server / resolver 注入，或建议在 Termux 环境运行以获得正常 resolv.conf

### 2) 默认优先 IPv6，当前无 `--disable-v6` 选项

现状：

- `connectivity.Attempt` 在存在 IPv6 candidate 时会先尝试 IPv6 direct
- 当前 CLI 只有 `--attempt-v6-timeout`，没有显式 `--disable-v6`

影响：

- LAN smoke 容易“走 IPv6 直接通”，掩盖对 IPv4-only / punching 的关注点

后续动作建议（需要额外 change）：

- 增加显式 `--disable-v6`（或 config 等价项），让 case 可稳定固定为 IPv4-only 口径

## Case1：Android 移动网络 ↔ Host（PASS，公网 punching 成功）

场景说明：

- Android 侧切到移动数据网络
- Host 保持当前家宽 / 本机环境
- 两端继续使用 `signaling=mqtt`

本次最终成功使用的参数：

- MQTT broker: `54.36.178.49:1883`
- STUN:
  - `106.13.249.54:3478`
  - `106.13.248.6:3478`
  - `106.12.251.193:3478`
  - `124.221.129.2:3478`
  - `124.222.69.57:3478`
  - `111.206.174.3:3478`

Host（client）：

```bash
/tmp/miopunch-lab-host peer client \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case1-20260414c \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --stun 106.13.249.54:3478,106.13.248.6:3478,106.12.251.193:3478,124.221.129.2:3478,124.222.69.57:3478,111.206.174.3:3478 \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s \
  --once
```

Pixel（visitor）：

```bash
adb -s 28201JEGR0XPAJ shell /data/local/tmp/miopunch-lab peer visitor \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case1-20260414c \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --payload case1mobile \
  --stun 106.13.249.54:3478,106.13.248.6:3478,106.12.251.193:3478,124.221.129.2:3478,124.222.69.57:3478,111.206.174.3:3478 \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s
```

结果要点（实际观测）：

- 两端均出现：
  - `stage=signaling kind=ok msg="connected to mqtt broker"`
  - `stage=exchange kind=ok msg="exchange.ok"`
  - `stage=attempt kind=ok name="attempt.punching.ok"`
  - `stage=transport kind=ok name="transport.payload_exchanged"`
- 事件路径：
  - 先 `attempt.v6.start`
  - 再 `attempt.v6.fail`
  - 随后 `attempt.punching.start`
  - 最终 `attempt.punching.ok`
- Host 侧观测到的远端 `raddr`：`114.254.0.143:5362`
- Android 侧观测到的远端 `raddr`：`111.194.89.226:21338`
- 传输层：`quic + brutal`
- payload 交换成功：`bytes=11`

本次实验暴露的额外结论：

- STUN server 集合过少时，容易出现 `mapped_addrs` 不足，导致 punching 被控制面直接禁用。
- 本次前两轮失败分别表现为：
  - Android 侧 `mapped_addrs=0`，错误为 `visitor has insufficient STUN mapped_addrs`
  - 两端 `mapped_addrs=1`，错误为 `client has insufficient STUN mapped_addrs`
- 把 STUN 集合扩展到 6 个公网 IP 后，两端都拿到 `mapped_addrs=3`，随后 punching 成功。

### 2026-04-14 复跑留档（完整命令 + 完整日志）

说明：

- 本节保存一次完整成功运行的命令与完整 JSON 日志，避免只保留摘要。
- 本次运行使用 topic `miopunch/case1-20260414d`。

完整命令（Host / client）：

```bash
/tmp/miopunch-lab-host peer client \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case1-20260414d \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --stun 106.13.249.54:3478,106.13.248.6:3478,106.12.251.193:3478,124.221.129.2:3478,124.222.69.57:3478,111.206.174.3:3478 \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s \
  --once
```

完整命令（Android / visitor）：

```bash
adb -s 28201JEGR0XPAJ shell /data/local/tmp/miopunch-lab peer visitor \
  --signaling mqtt \
  --mqtt-broker 54.36.178.49:1883 \
  --mqtt-topic-prefix miopunch/case1-20260414d \
  --proxy case1 --secret case1secret --user case1 \
  --data-proto quic --quic-cc brutal \
  --payload case1mobile \
  --stun 106.13.249.54:3478,106.13.248.6:3478,106.12.251.193:3478,124.221.129.2:3478,124.222.69.57:3478,111.206.174.3:3478 \
  --stun-timeout 15s \
  --disable-portmap \
  --hello-timeout 20s --exchange-timeout 20s --overall-timeout 180s
```

完整日志文件：

- Host: `docs/reports/2026-04-14-case1-mobile-host.host.jsonl`
- Android: `docs/reports/2026-04-14-case1-mobile-host.android.jsonl`

关键运行数据（本次复跑）：

- Android `mapped_addrs=3`，Host `mapped_addrs=4`
- Host `attempt.punching.ok`：`raddr=114.254.0.143:37385`
- Android `attempt.punching.ok`：`raddr=111.194.89.226:18057`
- 两端均出现 `transport.payload_exchanged`
- 本次 payload 长度为 `11` 字节（`case1mobile`）

关于“是否真的互传数据”：

- 是，当前 `payload_exchanged` 不是单纯表示握手成功。
- 数据面语义是：visitor 发送 payload，client 回 `ok:<payload>`，只有完成这一轮 exchange 后才会发出该事件。

### 2026-04-14 debug 复跑（NAT 分类与 punching 策略）

说明：

- 本次在 `miopunch` 中新增 `--log-level` 后，以 `--log-level debug` 对 case1 再复跑一次。
- 关键 NAT 分类与 detect behavior 日志来自 Android 侧，因为 `signaling=mqtt` 下的 `AnalyzeOnce` 在 visitor 本地执行。

本次 debug 结论：

- visitor（Android / 移动网络）被分类为：
  - `NatType=HardNAT`
  - `Behavior=BehaviorPortChanged`
  - `PortsDifference=6358`
  - `RegularPortsChange=false`
- client（Host / 家宽侧）被分类为：
  - `NatType=EasyNAT`
  - `Behavior=BehaviorNoChange`
- analyzer 选择：
  - `Mode=2`
  - visitor detect behavior: `Role=receiver`, `TTL=7`, `ListenRandomPorts=256`
  - client detect behavior: `Role=sender`, `SendDelayMs=3000`, `SendRandomPorts=1000`

这与代码策略一致：

- `Mode 2` 对应 “HardNAT 作为 receiver，EasyNAT 作为 sender”。
- 本次实际运行也符合该路径：先 `attempt.v6.fail`，再 `attempt.punching.start`，最后两端都 `attempt.punching.ok` 并完成 `transport.payload_exchanged`。

### 2026-04-14 当前代码复跑（case1）

说明：

- 本轮在 `p3.5-next` 当前代码上重新执行 case1。
- 先尝试“`-4` + 内置 STUN + `broker.hivemq.com:1883` + `--builtin-dns-mode on`”，随后回退到已知可用的公网 IP broker/STUN 组合。

#### 1) 内置 STUN + hostname broker（FAIL）

命令要点：

- Host / Android 两端都使用 `-4`
- `--mqtt-broker broker.hivemq.com:1883`
- 不显式传 `--stun`
- `--builtin-dns-mode on`

结果：

- Host 侧内置 STUN 采样成功，但 MQTT 连接超时：
  - `stage=signaling kind=fail msg="mqtt connect failed"`
  - 实际解析到的 broker 为 `tcp://35.156.35.233:1883`
- Android 侧内置 STUN `cn/global` 两组都未拿到可用映射地址：
  - `direct_addrs=0`
  - `mapped_addrs=0`
  - 最终 `gather failed`
- 因而这一轮没有进入 `exchange.ok` / `attempt.punching.ok` / `transport.payload_exchanged`

日志文件：

- Host: `docs/reports/2026-04-14-case1-mobile-builtin-hostname-fail.host.jsonl`
- Android: `docs/reports/2026-04-14-case1-mobile-builtin-hostname-fail.android.jsonl`

#### 2) 显式 broker IP + 显式 STUN IP 集合（PASS）

命令要点：

- Host / Android 两端都使用 `-4`
- `--mqtt-broker 54.36.178.49:1883`
- `--stun 106.13.249.54:3478,106.13.248.6:3478,106.12.251.193:3478,124.221.129.2:3478,124.222.69.57:3478,111.206.174.3:3478`
- `--data-proto quic --quic-cc brutal`
- `--disable-portmap`

结果：

- Host `mapped_addrs=3`，Android `mapped_addrs=3`
- 两端都出现：
  - `stage=signaling kind=ok msg="connected to mqtt broker"`
  - `stage=exchange kind=ok msg="exchange.ok"`
  - `stage=attempt kind=ok name="attempt.punching.ok"`
  - `stage=transport kind=ok name="transport.payload_exchanged"`
- Host `attempt.punching.ok`：`raddr=114.254.0.97:38728`
- Android `attempt.punching.ok`：`raddr=221.216.228.198:61065`
- payload 交换成功：`bytes=11`

补充观测：

- Android / visitor NAT 分类：
  - `NatType=HardNAT`
  - `Behavior=BehaviorPortChanged`
  - `PortsDifference=45874`
- Host / client NAT 分类：
  - `NatType=EasyNAT`
  - `Behavior=BehaviorNoChange`
- analyzer 仍选择 `Mode=2`，即 `HardNAT(receiver) <- EasyNAT(sender)`，与此前结论一致。

日志文件：

- Host: `docs/reports/2026-04-14-case1-mobile-explicit-ip-pass.host.jsonl`
- Android: `docs/reports/2026-04-14-case1-mobile-explicit-ip-pass.android.jsonl`

#### 3) 显式 broker IP + 内置 STUN（默认 `--stun-timeout=3s`，FAIL）

说明：

- 本轮代码已把 case1 实测可用的 CN STUN IP 前置，并把内置路径的预解析窗口限制到前 `6` 个 concrete endpoints，避免先把整份内置清单全部解析完。
- 在这个前提下重新用“仅内置 STUN”执行 case1，broker 仍固定为显式 IP，避免把 MQTT hostname 解析问题混入本轮结论。

命令要点：

- Host / Android 两端都使用 `-4`
- `--mqtt-broker 54.36.178.49:1883`
- 不显式传 `--stun`
- `--builtin-dns-mode on`
- 保持默认 `--stun-timeout=3s`

结果：

- Host 侧内置 STUN 正常：
  - `cn available=true count=2 ok_count=2`
  - `global available=true count=2 ok_count=2`
- Android 侧在默认 `3s` 预算下仍未拿到可用映射地址：
  - `cn available=false count=0 ok_count=0`
  - `global available=false count=0 ok_count=0`
  - 最终 `gather failed`
- 因而这一轮没有进入 `exchange.ok` / `attempt.punching.ok` / `transport.payload_exchanged`

日志文件：

- Host: `docs/reports/2026-04-14-case1-mobile-builtin-3s-fail.host.jsonl`
- Android: `docs/reports/2026-04-14-case1-mobile-builtin-3s-fail.android.jsonl`

#### 4) 显式 broker IP + 内置 STUN（`--stun-timeout=15s`，PASS）

命令要点：

- Host / Android 两端都使用 `-4`
- `--mqtt-broker 54.36.178.49:1883`
- 不显式传 `--stun`
- `--builtin-dns-mode on`
- `--stun-timeout 15s`
- `--hello-timeout 40s --exchange-timeout 40s`
- `--data-proto quic --quic-cc brutal`
- `--disable-portmap`

结果：

- Host：
  - `cn available=true count=2 ok_count=2`
  - `global available=true count=2 ok_count=2`
- Android：
  - `cn available=true count=2 ok_count=2`
  - `global available=false count=0 ok_count=0`
- visitor 侧 debug 日志显示：
  - `selected_view=cn reason=availability`
  - `visitor nat={HardNAT BehaviorPortChanged}`
  - `client nat={EasyNAT BehaviorNoChange}`
- 两端都出现：
  - `stage=signaling kind=ok msg="connected to mqtt broker"`
  - `stage=exchange kind=ok msg="exchange.ok"`
  - `stage=attempt kind=ok name="attempt.punching.ok"`
  - `stage=transport kind=ok name="transport.payload_exchanged"`
- Host `attempt.punching.ok`：`raddr=114.254.0.97:3016`
- Android `attempt.punching.ok`：`raddr=221.216.228.198:6303`
- payload 交换成功：`bytes=12`（`case1builtin`）

结论：

- 仅依赖内置 STUN 的 case1 现在可以在真实 Android 移动网络上跑通。
- 但当前 Android 侧对内置 STUN 的默认 `3s` 预算仍偏紧；提高到 `15s` 后可以稳定完成本轮 `case1`。

日志文件：

- Host: `docs/reports/2026-04-14-case1-mobile-builtin-15s-pass.host.jsonl`
- Android: `docs/reports/2026-04-14-case1-mobile-builtin-15s-pass.android.jsonl`

## Case2-4（占位）

- Case2：Android data network ↔ Home isolated subnet（`signaling=mqtt`）
- Case3：Home hub subnet ↔ Home isolated subnet（同宽带不同子网）
- Case4：Windows coverage（选 H0 或 H1 端点）
