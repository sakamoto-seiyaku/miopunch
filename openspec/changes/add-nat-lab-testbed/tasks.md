# Tasks

- [ ] 准备单个 `QEMU VM` 实验母机，基于 Debian 官方 `qcow2 / cloud image` 完成基础启动与 `SSH` 管理。
- [ ] 在 VM 内安装并验证基础工具：`iproute2`、`nftables/iptables`、`tc`、`tcpdump`、`conntrack`、`docker` 等。
- [ ] 建立 `base-ready` 与 `lab-ready` 两级快照与回滚流程。
- [ ] 搭建 VM 内部的基础 lab 拓扑模型，支持 `peer`、`nat`、`stun`、`coord` 等角色。
- [ ] 定义 case 切换机制，允许多个 case 共存定义，但任意时刻只运行一个 active case。
- [ ] 定义最小 case 元数据 schema 与产物目录结构（包含 `RFC 4787`、`NAT1-4`、`frp` 标签与运行结果字段）。
- [ ] 实现并固化 NAT profile 的 `mapping behavior` 验证（`EIM / ADM / APDM`）。
- [ ] 实现并固化 NAT profile 的 `filtering behavior` 验证（`EIF / ADF / APDF`）。
- [ ] 为第一批主覆盖集实现 NAT profile 配置与验证断言，确认其 `RFC 4787`、`NAT1-4`、`frp` 标签成立。
- [ ] 定义并落地最小观测产物：`stdout/stderr`、`pcap`、`nft/iptables ruleset`、`conntrack` 状态、`tc qdisc` 状态、阶段化失败诊断时间线。
- [ ] 补充使用文档，说明镜像来源、快照恢复、case 切换、代表场景覆盖与限制。
- [ ] 运行 `openspec validate add-nat-lab-testbed --strict --no-interactive` 并修复问题。
