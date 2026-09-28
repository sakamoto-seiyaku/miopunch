## Context

MNT-01 对应 `docs/decisions/mainline-network-test-charter.md` 的场景 1：主线连接性矩阵。当前仓库已有 NAT lab、MQTT signaling、TCP Door-2、事件期望和 POC e2e 能力，但它们分别覆盖夹具可信度、代表路径或产品控制面，没有形成“真实主线节点 + MQTT-only + 完整二节点连接矩阵”的主线 gate。

本 change 只创建场景 1 的实现计划和验收契约。它必须避免继续扩大旧 `coord` 路径，并避免把场景 2 的 join/governance 或场景 3 的多节点 overlay 提前混入。MNT-01 可以为现有主线 hello/auth 握手注入最小认证 bootstrap，但这不是 join/governance 行为覆盖。

## Goals / Non-Goals

**Goals:**

- 建立主线二节点连接性测试入口，以真实 `miopunch` 主线节点作为被测对象。
- 使用自部署 MQTT broker 作为唯一主线信令入口，并收集 broker log/pcap 证据。
- 用分轴 profile 覆盖 UDP 无向 15 类组合和 TCP 有向 49 类组合。
- 为 success、preferred success、diagnostic failure 和 required failure 建立稳定验收分类。
- 为 TCP hard/irregular 组合建立 bounded repeat/retry、预算和诊断口径。
- 提供 smoke/selftest/fulltest 三层 gate，便于日常回归和完整矩阵验收。

**Non-Goals:**

- 不测试 `invite`、`approve`、`join`、governance、decl 同步或多节点 membership。
- 不验证 logN 邻居维护、bootstrap、overlay 收敛或扰动恢复。
- 不引入公网 MQTT required gate。
- 不新增中心化数据面 relay。
- 不在本 change 中修复测试暴露出的产品代码问题。

## Decisions

### 1. MQTT-only mainline signaling

MNT-01 required gate 使用测试环境自部署 MQTT broker。`coord` 只可能继续存在于历史 lab regression 中，不计入 MNT-01 通过标准，也不作为 fallback。

Rationale: 当前主线信令/control-plane 口径已经收敛到 MQTT；继续依赖 `coord` 会让连接性结果不能代表主线。

### 2. Fixture provides only minimum bootstrap material

场景 1 不测 join/governance，因此二节点 fixture 可以提供 identity、peer、hello/auth bootstrap、MQTT endpoint、STUN endpoint 和网络画像。hello/auth bootstrap 仅包含现有主线 hello handshake 所需的 governance head snapshot 与 member approval decl，并必须在 `fixture.json` 中以 `auth_bootstrap` 披露。fixture 不得提供 NAT 判定、candidate 结论、attempt path、邻居状态、成功缓存或 payload 结果。

Rationale: 这样既能隔离连接能力，又不会通过预置结果“骗过”连接性测试。

### 3. Matrix dimensions are separated from specialty axes

UDP matrix 使用 5 个 UDP profile 的无向 15 类 pair。TCP matrix 使用 7 个 TCP profile 的有向 49 类 pair，保留 `dialer -> target` 角色。`auto`、IPv6、portmap、loss/netem、blocked、STUN unavailable、QUIC/KCP/BBR/Brutal 不乘入主矩阵，而是在 smoke/selftest 中用代表 case 做专项覆盖。

Rationale: 主矩阵必须完整，但不能把所有修饰轴无限笛卡尔积化。

### 4. Outcome classification replaces boolean pass/fail

每个 case 必须声明期望分类：`success-required`、`success-preferred`、`diag-fail-allowed` 或 `fail-required`。TCP hard/irregular case 可以用可解释失败通过，但必须证明尝试路径、预算、停止条件和原因。

Rationale: TCP 打洞比 UDP 更复杂，用单一 pass/fail 会造成不稳定组合误判或被静默跳过。

### 5. Evidence is part of the contract

所有 case 都必须收集足够证据：MQTT signaling、candidate discovery、attempt path、selected/failed path、payload evidence 或 failure reason。成功 case 必须有 payload exchange；失败 case 必须有 failure stage、reason 和 `stop_condition`，并写入 per-attempt artifact 与 summary。broker log/pcap 不得出现数据面 payload。

Rationale: MNT-01 的目标是可复现、可诊断、可复盘，而不是只看 exit code。

### 6. Gate layering controls cost

- smoke：少量代表 case，验证 MQTT-only 主链路、direct、punching、TCP hard 诊断、auto 顺序、STUN unavailable 和 transport variants。
- selftest：覆盖 UDP 15 类、TCP 高风险代表组合、IPv6 fallback 和 loss/netem specialty。
- fulltest：覆盖 UDP 15 类与 TCP 49 类完整矩阵，并输出包含 required/preferred/diagnostic/required-fail/unexpected 计数的聚合报告。

Rationale: 日常开发需要快速反馈，主线合入前需要完整矩阵。

## Risks / Trade-offs

- [Risk] TCP hard/irregular 组合结果不稳定 → [Mitigation] 使用 bounded repeat/retry、分类验收和稳定诊断，而不是要求全部成功。
- [Risk] 矩阵规模导致 fulltest 过慢 → [Mitigation] 明确 smoke/selftest/fulltest 分层，避免日常 gate 跑完整矩阵。
- [Risk] fixture 过度注入导致测试失真 → [Mitigation] 将 fixture 可注入字段和禁止字段写入 spec，并在 artifacts 中保留快照。
- [Risk] 与历史 `xtcp`/`coord` 命名混淆 → [Mitigation] 新增入口和文档使用 `miopunch`/`mainline` 命名；历史路径只作为 legacy 保留。
- [Risk] 测试暴露产品缺陷拖慢 MNT-01 → [Mitigation] 缺陷记录到 `docs/notes/mainline-network-test-findings.md`，阻塞项单独标注并后续拆 fix change。
