## Why

在 `P3` 阶段我们已经能在实验台（P0 NAT lab）里稳定复现“打洞 + payload exchanged”，但在真实公网环境（移动网络/家庭宽带/跨区域）做实验时仍会被一些“可达性前置条件”卡住：

- `DNS`/环境异常会迫使测试退回到手工输入 `STUN/MQTT` 的 IP 形式，不利于复现与分享命令。
- 双栈环境下需要显式限制 `P2P/打洞` 的地址族，否则 `IPv4/IPv6` 路径互相干扰，增加定位成本。
- 中国大陆 / 非中国大陆分流会导致不同 `STUN` 观测面返回不同公网视角；如果不把“最终选定哪一类视角继续打洞”纳入主线语义，就难以稳定解释与复现实验。

本变更以 `P3.5` 的定位为约束：补齐“公网实验能跑、能解释、能复现”的最小能力，不提前进入 `P4` 的产品化与发布阶段。

## What Changes

- 为 peer CLI 增加短选项 `-4/-6`：
  - `-4`：仅约束 `P2P/打洞` 使用 `IPv4`（不限制 signaling 的联网行为）。
  - `-6`：仅约束 `P2P/打洞` 使用 `IPv6`（不限制 signaling 的联网行为）。
  - 默认 `auto`：沿用现有双栈策略。
- 增加“内置 DNS（仅用于 STUN/MQTT 解析）”：
  - 默认 `auto`：系统解析失败才回退到内置 DNS。
  - 内置 DNS 默认通过 `TCP/53` 查询；不引入 `DoT/DoH`。
  - 支持通过 CLI 与 YAML 配置固化/覆盖内置 DNS 的启用模式与 resolver 列表。
- 增加内置 `STUN` 名单并按 `cn` / `global(!cn)` 分组采样（来源以 `gonc` 为基线）：
  - 当用户显式提供 `--stun`/`stun:` 时：仅使用用户输入；不使用内置 STUN；不做 `cn/global` 分组与仲裁。
  - 当未显式提供 STUN 时：系统可采样 `cn/global` 两组并进行仲裁；`exchange` 最终只产出 **一个**“已选定的 candidate 视角/分组”，后续 attempt/punching 只沿该视角执行。
  - 仲裁顺序固定为：`可用性` → `NAT feature 难度` → `STUN RTT` → `成功次数` → `默认 global`（RTT 打平阈值 `30ms`）。
- 可观测性：
  - `debug` 日志必须输出完整的观测与仲裁证据链；
  - 非 debug 日志可递减细节，但必须至少记录最终选中的视角与关键原因。

## Capabilities

### New Capabilities
- `miopunch-public-reachability`: 为 `P3.5` 公网实验补齐最小可达性能力（`-4/-6` 仅约束 P2P、内置 DNS 仅用于 STUN/MQTT 解析、内置 STUN 的 cn/global 观测与单视角仲裁、以及对应的可观测性要求）。

### Modified Capabilities
- (none)

## Impact

- CLI/配置：
  - `cmd/miopunch`: 增加 `-4/-6`；扩展 YAML 配置以表达 P2P 地址族偏好与内置 DNS 策略（命名保持现有 `snake_case` 风格）。
- 解析/可达性：
  - `connectivity/*`: STUN 解析与 cn/global 观测仲裁；确保 `exchange` 语义只产出一个最终视角。
  - `internal/signaling/mqtt/*`: broker 主机名解析在需要时可回退到内置 DNS（不改变其余联网策略）。
- 依赖：
  - 可能引入最小 DNS client 依赖（或自实现 TCP/53 查询）；不引入 DoT/DoH。
- 验证：
  - 提供可复现的公网实验运行路径（非 P0 实验台），并在日志/事件中保留“最终选定视角”的证据。
