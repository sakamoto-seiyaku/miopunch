## Context

在 P3.5（STUN + MQTT 信令）完成后，我们希望把验证从“P0 虚拟 NAT 实验台”推进到**真实网络**，并用一个小而固定的 case 清单（case0-4）覆盖我们当前能拿到的设备与网络组合。

当前可用环境：
- **Host（当前开发机）**：Linux（可运行/编译 miopunch）
- **Pixel 6a（Android）**：ADB 可达、具备 `su`，与 Host 同一局域网（case0）
- **家宽 + 路由器多子网**：
  - `H0`：全透/Hub 子网（可访问其他子网）
  - `H1`：严格隔离子网（不可访问其他子网；但可正常访问公网）
- **移动网络**：Android 手机切到数据网络
- **Windows**：可选加入覆盖
- **公网依赖**：
  - STUN：你提供的 CN + overseas 列表
  - MQTT：使用“官方提供的 MQTT 服务”（非自建 broker）

本变更的定位是“探索阶段的最小可执行 runbook”，不追求面向终端用户的兼容性与 UX。

## Goals / Non-Goals

**Goals:**
- 固化 **case0-4** 的拓扑、角色分配（client/visitor）与统一验收标准。
- 给出可复制的执行方式：
  - `miopunch peer ... --config <yaml>` 为主（避免手敲长命令）
  - CLI flags 作为 override（必要时快速调整）
- 把执行方式与结果记录到一份临时 runbook/报告文档（建议放在 `docs/reports/2026-04-13-public-verification-cases.md`，后续可按日期滚动）。
- Android 侧使用 **arm64 二进制交叉编译**（运行不需要安装 Go），并给出 ADB 部署/执行方式。
- 每个成功 case 都能明确产出证据链：至少包含 `transport.payload_exchanged`（两端）。

**Non-Goals:**
- 不引入端到端加密、强鉴权、版本兼容协商（P4 再做）。
- 不把这些 case 自动化进 CI（真实网络不可控）。
- 不扩展为“全矩阵测试”（先最小集跑通，再扩展）。

## Decisions

- **信令介质**：统一使用 `--signaling mqtt`（官方 MQTT 服务）作为公网验证的默认路径；`--signaling coord` 仅作为局域网 case0 的可选简化路径（降低变量）。
- **传输选择面（先最小）**：
  - 默认：`--data-proto quic --quic-cc brutal`（P3 重点）
  - 通过 runbook 提供可选扩展：`bbr` / `kcp`（后续再扩大覆盖）
- **角色分配**：
  - `client` 放在“更稳定的常驻端”（例如家宽主机），作为服务端（`ServeAndExchange`）
  - `visitor` 放在“操作驱动端”（例如手机），携带 `--payload` 并在完成后退出（`DialAndExchange`）
- **验收标准**：
  - **MUST**：两端日志均出现 `transport.payload_exchanged`
  - **SHOULD**：记录最终 path（`direct_ipv6` / `direct_ipv4` / `portmap` / `punching_ipv4`）及 STUN 映射信息，用于后续对齐真实网络表现
- **配置模板**：
  - 仓库内仅提交**不含敏感信息**的模板（MQTT user/pass、secret 使用占位符）
  - 统一字段：`signaling/mqtt_broker/mqtt_topic_prefix/mqtt_user/mqtt_pass/stun/proxy/secret/data_proto/quic_cc/...`

### Case Matrix (case0-4)

- **case0（LAN smoke）**：Host ↔ Pixel6a（同局域网）
  - 目的：验证 Android 二进制可运行、与 Host 端到端 payload 可交换
  - 默认建议：`signaling=coord`（Host 起 `miopunch coord`，Pixel6a 直连）以减少外部依赖；可选再跑 `signaling=mqtt` 覆盖 MQTT 路径
- **case1（Mobile ↔ H0）**：移动网络 Android ↔ 家宽全透子网主机（H0）
  - 目的：真实公网主路径（移动数据到家宽）
- **case2（Mobile ↔ H1）**：移动网络 Android ↔ 家宽隔离子网主机（H1）
  - 目的：验证隔离子网环境对打洞/传输是否有额外影响
- **case3（H0 ↔ H1）**：家宽全透子网主机 ↔ 家宽隔离子网主机
  - 目的：同一公网出口 + 内网互访受限时的行为（hairpin/ACL 差异）
- **case4（Windows coverage）**：Windows@H0 或 Windows@H1 参与（与 Mobile 或另一端互通）
  - 目的：覆盖 Windows 平台的实际可运行性

## Risks / Trade-offs

- **[官方 MQTT 接入限制]** 可能要求 `tls://`/`wss://`、端口固定、topic 有前缀限制 → runbook 以占位符形式记录，执行时按服务商要求填入；必要时切换到 `wss://` 以穿透常见网络限制。
- **[真实 NAT/ACL 不确定]** case1-4 的失败不可避免 → 通过 stage-level 事件（signaling/gather/exchange/attempt/transport）定位失败阶段，并记录到报告中。
- **[Android 运行环境差异]** 某些路径/目录存在 `noexec` → runbook 约定二进制放 `/data/local/tmp` 或 Termux 私有目录执行。
