# Tasks

## 1. Scaffolding

- [ ] 初始化 Go 模块（`go.mod`）与最小可运行的目录结构。
- [ ] 定义最小 CLI：能够启动/连接协调端、运行两个 peer 端并输出可机读诊断信息。
- [ ] 从 `frp/`（submodule）复制/抽离 `xtcp/nathole` 相关代码，尽量保持上游结构与行为；记录来源 commit/tag。
- [ ] 保留复制文件的上游许可证与版权头部，并在仓库中补齐必要的第三方许可证与归因文件。

## 2. Control plane (signaling)

- [ ] 定义最小信息交换协议（会话创建、候选地址交换、重试/超时语义），优先复用并裁剪 `frp` 现有消息与流程，避免重新设计。
- [ ] 实现协调端与 peer 端的最小握手流程，优先复用并裁剪 `frp` 现有实现；仅做必要的鉴权/防误配边界，不追求完整安全体系。
- [ ] `control plane` 传输支持 `KCP / QUIC` 选项（`TCP` 仅作为默认基线，不计入该选择；不含 `fallback relay`），并提供最小的参数与诊断输出。

## 3. Discovery & classification

- [ ] 实现 STUN discovery（最小可用）并输出关键发现结果。
- [ ] 定义 NAT 行为信息记录结构（为后续 `RFC 4787` 扩展保留空间）。

## 4. Hole punching core

- [ ] 实现 UDP 打洞核心状态机（发包策略、候选端口轮转、确认机制、超时与重试）。
- [ ] 在已打通的 UDP 五元组之上，支持 P2P 数据面使用 `KCP / QUIC` 两种传输，并可被协商/选择。
- [ ] 暂不实现额外“加密/压缩”包装逻辑；`QUIC` 仅使用其内建 TLS，`KCP` 不做额外封装（必要时只保留最小防串线机制）。
- [ ] `P1` 不实现 `fallback/relay`：直连失败就失败；失败路径必须可观测、可定位、可复盘。

## 5. Observability

- [ ] 定义阶段枚举与事件模型：`discovery/signaling/punching/confirm/transport`。
- [ ] 输出可机读时间线（例如 `json` event stream）并在失败时携带阶段与关键条件。

## 6. Testing

- [ ] 单元测试：协议编解码、状态机、超时/重试、候选选择。
- [ ] 集成回归：接入 `P0` 实验台，在代表性 case 上验证成功/失败路径与诊断信息完整性。

## 7. Docs

- [ ] 补充最小使用文档：如何在 `P0` 实验台中运行、如何解读诊断时间线、如何查看 artifacts。

## 8. Validation

- [ ] 运行 `openspec validate add-xtcp-kernel --strict --no-interactive` 并修复问题。

## Verification (post-implementation)

- `go test ./...`
- `./lab/host/labctl selftest`
