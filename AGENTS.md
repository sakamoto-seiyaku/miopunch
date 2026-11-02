@/home/js/.codex/RTK.md

# miopunch agent rules

The repository root is a new design baseline. The former Go/Android code,
OpenSpec material, and project-local development skills are under
`archive/legacy-go-openspec-2026-09-28/` without a current implementation
role. Do not apply the old Go/OpenSpec workflow or its test gates to new work
by default. Whether old tests and scripts should be reused, adapted, or
replaced is undecided.

Do not turn research suggestions into product decisions. Read the current
design entry points before planning implementation, and record new decisions
explicitly. Use Plane for tasks; never use Plane MCP.

## 实施与验证

主线程、子代理、临时驱动和 worktree 同规；授权内连续完成，不逐步确认。

- 尽早跑通最小真实入口，覆盖适用的 UI、宿主、提交与独立读取；静态或单元通过不替代入口验收。
- 代码与断言同审；先核实前置及真实失败，区分产品、测试、环境问题；沿用原操作和核心断言验证修复，不盲重试或弱化验收。
- 按目标、不变量及关联路径收齐同类问题后批量修复。第 3 轮未解或累计等待 1800 秒，先到即扩查共享模块、全部入口、正负场景与真实运行；按稳定类别累计，换名或入口不清零，并行双审算一轮。
- 主线程调度、汇总、验收；代理核对差异后自行完成授权检查、构建和脚本小修。复用有效证据；无行为改动不重建、不重启全审；仅实质设计缺口交用户决策。

## Agent skills

### Rust 构建与产物

细则见 `docs/agents/build-cache.md`。

- Cargo 管依赖与重编判断，不另造持久构建图缓存。共用 `target/` 和 `scripts/cargo_cache.py` 构建锁；不复制缓存、不按票或运行新建 target，仅真实工具链／ABI 差异隔离。
- 用 `-p`、`--lib/--bin/--test` 和必要 features 限定目标；辅助程序留工具包，全 workspace 仅用于跨包影响或明确验收。
- 使用 `--locked`，依赖齐全时加 `--offline`；沿用提交的 profile、工具链和稳定链接器路径。不随运行换 flags／wrapper；源、锁或产物前置校验失败即停。
- 同批同图编译一次、多场景复用；跨批由 Cargo 判定。记录真实 Fresh/rebuilt，意外重编先查因；输入未变复用通过结果。
- 工具按版本存 `.artifacts/tools/`；先查现有工具和收据再报缺失，不依赖临时或票号目录。
- `target` 不是档案。仅为当前交付、验收或必要复现用 `scripts/pin_artifact.py` 固定产物，复用链接；不全量 pin Fresh，不逐场景复制源码和二进制。禁直接硬链 target、改写共享 pin；strip 等转换仅用独立部署副本，运行结束回收。
- 保留须有当前消费者或明确发布／复现用途；“每族最新”、历史日志／收据提及、fingerprint 不同均非保留理由。清理先查宿主可见进程与实际引用，按明确名单回收闲置产物；保护在用程序、工具和有效依赖缓存，禁整删 deps 或盲目 cargo clean，记录实际释放量。
- 沿用现有容量阈值，超限先查占用，不擅自扩容或另建 store。日志、来源足以复现即可；哈希仅做必要身份、去重或协议校验，有效结果直接复用，不增收据框架或重复存证。

### Issue tracker

Issues, PRDs, and decision tickets live in Plane's `MIOPUNCH` project. Use the
global `plane-ops` skill with this repo's defaults, not Plane MCP. See
`docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage roles in Plane. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo. Read `CONTEXT.md`, the current design notes,
and relevant ADRs before architectural work. Archived Go/OpenSpec material is
historical only. See `docs/agents/domain.md`.
