## Why

我们需要把 miopunch 的“打洞 + 传输层 payload exchange”能力，从 P0/P2 的实验室虚拟 NAT 环境，推进到**真实网络**下的可重复验证。

当前我们已具备可用的真实测试资源：同局域网的 Host 与 Pixel 6a（ADB 可达、具备 root），以及家宽下的多子网（含全透与隔离子网）、移动网络与 Windows 主机。为了后续进入 P4（公网交换信道/加密等）以及最终开源发布，需要先把“在真实网络上到底跑哪些组合、如何跑、验收看什么”固化为一份最小的 case 清单与执行/验收流程。

## What Changes

- 新增一份“真实网络验证 runbook（case0-4）”临时文档，明确：
  - 每个 case 的两端环境（Host / Pixel6a / 家宽 H0/H1 / Windows）
  - 角色分配（client/visitor）
  - 统一的验收标准（必须出现 `transport.payload_exchanged` 等证据链）
  - 日志采集与失败定位（按 stage：signaling/gather/exchange/attempt/transport）
- 提供可复制的 YAML 模板（`--config`），把 MQTT/STUN/超时/传输选项固化，并约定“配置文件为主，CLI 覆盖为辅”。
- 给出 Android arm64 二进制的交叉编译与 ADB 部署方式（无需在手机安装 Go）。

## Capabilities

### New Capabilities
- `public-verification`: 真实网络验证用例（case0-4）与最小可执行 runbook（配置模板 + 验收标准）。

### Modified Capabilities
- (none)

## Impact

- 文档与流程：新增/更新 runbook 文档与配置模板（不改变核心 punching/dataplane 行为）。
- 工具链：补充 Android arm64 构建与 ADB 执行方式（仅文档/脚本层面）。
