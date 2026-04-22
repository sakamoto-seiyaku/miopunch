## Why

随着内置 STUN 列表扩充，当前串行采样路径会把 `stun-timeout` 总预算消耗在前面少数端点上，导致后续高优先级或同组候选根本试不到。`case1` 现在仍依赖显式 `--stun` 才能稳定通过，因此需要把内置 STUN 采样改成“优先级前置 + 受限并发 + 足够即停”，让默认路径真正可用于公网实验。

## What Changes

- 将内置 STUN 采样从串行请求改为受限并发请求，而不是一次性并发全部端点。
- 为内置 `cn` / `global` 两组 STUN 建立稳定的优先级顺序，把已验证更稳定的服务放在前面。
- 在每组采样中引入“足够信息即停止”的停止条件，拿到足够的 `mapped_addrs` / `ok_count` 后取消该组剩余请求。
- 让 `cn` / `global` 两组的内置采样并行推进，避免某一组独占整个 `stun-timeout` 预算。
- 保持显式 `--stun` 路径语义不变：仅用户提供列表、顺序解析、无内置优先级逻辑。
- 在 Android 真实网络下重新运行 `case1`，验证仅依赖内置 STUN 时是否可以跑通并完成 `transport.payload_exchanged`。

## Capabilities

### New Capabilities
- (none)

### Modified Capabilities
- `xtcp-connectivity`: 内置 STUN 采样从串行 best-effort 调整为按优先级排序的受限并发采样，并在获取足够 NAT 观测信息后提前停止。

## Impact

- `connectivity/*`：内置 STUN 的解析、发包、收包分发与停止条件逻辑。
- `internal/wire` / `coordinator`：不改语义，但会受新的 observation 数量与时序影响。
- `docs/reports/*`：补充新的内置 STUN case1 实测结果。
- 真实网络验证：需要 Android 移动网络 + host 重新跑 `case1`。
