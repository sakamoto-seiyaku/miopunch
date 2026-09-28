## Context

`internal/coordinator` 是 punching 的中心协调端，负责在 visitor/client 交换信息前后给出可机读的 `NatHoleResp`（包含 `Error` 字段）。目前该模块中存在历史遗留的 `xtcp` 字符串出现在错误文案中，导致：

- 用户在运行 `miopunch` 时看到 `xtcp`，与项目命名收敛目标冲突；
- 日志/事件/实验 runbook 输出出现割裂，降低可复现性与可解释性。

该问题不涉及协议、行为或状态机，只涉及对外字符串。

## Goals / Non-Goals

**Goals:**
- 将对外错误文案中的 `xtcp` 替换为 `miopunch` 或更中性的表述（如 `proxy` / `peer`），保持语义不变。
- 仅修改字符串，不改变任何打洞、交换、分析、鉴权逻辑。
- 通过基础门禁：`gofmt`、`go test`、`go vet`、`check_no_xtcp_imports`。

**Non-Goals:**
- 不修改 `wire` 协议字段或错误码体系（仍使用现有 `Error` string）。
- 不引入 i18n/错误类型化等产品化设计。

## Decisions

- 选择将 “xtcp server ...” 等文案替换为 “proxy ...”/“peer ...” 的中性描述：
  - 理由：实验阶段更关注“哪个 proxyName 出问题”而非角色命名；中性描述更稳定、也更不易再次引入 `xtcp`。
- 同一文件内所有对外错误文案一次性清理，避免只修复单行后仍有残留。

## Risks / Trade-offs

- [下游脚本依赖具体错误字符串] 可能导致匹配失败 → **Mitigation**：实验阶段优先以结构化事件字段（stage/kind/name/kvs）为判断依据；如确有脚本依赖，再在脚本侧做兼容。

