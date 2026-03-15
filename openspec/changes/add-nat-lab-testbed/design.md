# Design: nat-lab-testbed

## Summary

`P0` 测试实验台采用“单个 `QEMU VM` + VM 内部 Linux 网络拓扑”的结构。
`QEMU VM` 负责隔离宿主环境，`netns / veth / nftables or iptables / tc` 负责表达 NAT 拓扑、链路扰动和角色关系。

## Key Decisions

### Single-VM lab host

- 只使用一个 `QEMU VM` 作为实验母机。
- 不在 `P0` 引入多 VM 拓扑。
- 不在 `P0` 直接改动 `Windows` 或 `WSL2` 默认网络。

### VM management and baseline

- 基础镜像使用 Debian 官方 `qcow2 / cloud image`。
- VM 通过 `SSH` 管理，并允许正常联网。
- VM 内安装 `docker`，但 `P0` 不把 Docker 定义为 NAT 拓扑主控。

### Snapshot model

- `base-ready`：基础镜像、`SSH` 与所需工具已就绪。
- `lab-ready`：case 定义、切换脚本与校验逻辑已就绪，但没有 active case 与待测二进制。
- 快照用于恢复环境基线，不用于保存单个测试结果。

### Case model

- case 以 `RFC 4787` 为主分类。
- `NAT1-4` 作为兼容标签。
- `frp Easy/Hard + Behavior` 作为工程标签。
- 多个 case 可以共存定义，但任意时刻只运行一个 active case。
- 仅当实际测试确认方向性差异时，才将代表场景细分为更小 case。

### NAT implementation boundary

- `P0` 先使用 Linux 内核已有能力模拟 NAT 行为。
- `P0` 不引入用户态 NAT emulator。
- 如果后续发现某些关键 NAT 行为无法稳定或可解释地复现，再单独提出后续 change。

## Validation Notes

- `mapping behavior` 当前只做 `EIM / APDM` 两类区分，`ADM` 暂不做。
- `nat4-regular` 为稳定复现“目的地址相关”的映射行为，会对不同 remote IP 使用不同 SNAT 端口子区间；该行为属于实验台实现细节。
