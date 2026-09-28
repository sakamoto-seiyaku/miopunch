## Context

当前主仓库已经包含多层能力（`connectivity/`、`internal/control/`、`internal/punching/`、`dataplane/` 等），并通过实验台与公网实验逐步验证“打洞 + payload exchanged”链路。随着迭代推进，代码中出现了一些典型的工程风险点：

- 资源回收语义不完整（例如 QUIC 连接只关 stream 不关 conn）。
- 库代码向 stdout 输出调试信息（破坏 event/log 的可机读性与定位效率）。
- 少量死代码/无效 helper 增加理解成本，且可能误导后续实现。
- 命名仍有少量 `xtcp` 残留与 `miopunch` 的收敛目标不一致。

本 change 以“保持可跑、可解释、可复现”的实验定位为约束，只做必要的工程性修复，不引入过度产品化设计。

## Goals / Non-Goals

**Goals:**
- 明确并修复控制面 QUIC 连接的 `Close()` 语义：`Close()` 必须释放底层连接资源，避免泄漏。
- 移除库代码中的 stdout 输出，将调试输出收敛到现有日志设施（`internal/logutil`）。
- 清理明显的未使用/无效 helper，降低误用概率。
- 收敛非必要的 `xtcp` 运行时标识（如 ALPN 字符串、包注释），向 `miopunch` 统一命名靠拢。
- 所有修复在本仓库内自洽并通过基础门禁：`gofmt`/`go test`/`go vet`/`check_no_xtcp_imports`。

**Non-Goals:**
- 不重写 `internal/punching/` 的 mode0..4 算法实现与行为（避免破坏 P0/P1 基线）。
- 不引入新的外部依赖或大规模重构（仅做小步收敛）。
- 不在本 change 内完成“真实公网全矩阵”的实验验证（保持在 P3.5 的既定节奏内推进）。

## Decisions

1) **控制面 QUIC 连接关闭语义**
- 选择：`internal/control` 中 `quicStreamRWC.Close()` 同时关闭 stream 与底层 QUIC conn（`CloseWithError(0, "")`）。
- 理由：`Dial()`/`Accept()` 对外返回的是 `io.ReadWriteCloser`，调用方按语义会认为 Close 能释放对应连接资源；只关 stream 会导致连接/协程泄漏，尤其在短会话与高频实验中放大。
- 备选：由调用方显式拿到并关闭 conn（需要暴露更多类型/接口，破坏简洁性）；不采用。

2) **BBR 调试输出收敛**
- 选择：将 `internal/dataplane/congestion/bbr` 的 `fmt.Printf` 输出改为 `internal/logutil` 的 debug 级别日志。
- 理由：stdout 输出会污染 CLI 的 event 输出与脚本解析；而 `logutil` 已具备可控级别与统一入口。
- 备选：完全移除 debugPrint（会降低定位能力）；不采用。

3) **`xtcp` 残留标识的处理**
- 选择：对当前“运行时协议标识”做最小收敛（例如 ALPN 字符串从 `miopunch-xtcp-*` 改为 `miopunch-*`）。
- 理由：避免继续扩大命名割裂；该变更对实验阶段可控。
- 备选：保留旧字符串以兼容旧二进制；在当前“无兼容性优虑”的阶段暂不引入额外分支逻辑。

4) **死代码/无效 helper 清理**
- 选择：删除未引用且易误导的 helper（例如未使用且不生效的 `withDeadline`、未被调用的 frame helper、未被引用的 NAT hole prepare glue）。
- 理由：减少未来演进时的误用空间；让关键路径更显式。

## Risks / Trade-offs

- [QUIC Close 语义变化] 可能导致连接比预期更早关闭 → **Mitigation**：本仓库 client/server 同步更新；通过 `go test ./...` 覆盖；必要时补充最小 QUIC 回归用例。
- [日志收敛] debugPrint 改为 logger 后可能“看起来没输出” → **Mitigation**：保持 debug 级别日志，并确保 CLI `--log-level debug` 时可见。
- [命名收敛] ALPN 变化可能导致与旧版本互联失败 → **Mitigation**：实验阶段接受；如后续需要兼容，再在 P4 引入双 ALPN 兼容策略。

