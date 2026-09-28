## Why

当前仓库里与控制面相关的语义散落在 `internal/controlplane`、`internal/task` 与历史 POC v0 contract 之间：JSON wire、签名输入、MQTT topic、invite/enroll 以及 dial 协商都纠缠在一起。结果是：

- 旧实现能跑，但已经不适合作为“可解释、可独立实现”的 v1 主干。
- v0 JSON/AES-GCM 与这轮 v1 TLV/peer_e2e_v1 的 source of truth 混在一起，后续实现很容易再漂移。
- 如果不先把 01 改成真正的抽离蓝图，后面的 `02/03/04/05` 只会继续往旧栈上补。

本 change 的职责不再只是“冻结口径”，而是把 **POC v1 的 peer-targeted control-plane wire/security contract** 变成第一块可实施的抽离地基。

## What Changes

- 将 01 重写为并行抽离蓝图：新语义进入 `internal/pocv1/wire` 与 `internal/pocv1/peere2e`，legacy `internal/controlplane` 只保留参考地位。
- 定义并实现 v1 唯一 wire/security contract：TLV bytes、outer/inner envelope、固定 transcript、`peer_e2e_v1`、drop-only errors、golden vectors。
- 明确 01 只拥有顶层 `kind` 名字集合，不拥有 `join_request/enroll_response/dial_offer/dial_answer` 的 body schema；这些分别交给 `02` 与 `04`。
- 通过 delta spec 明确旧 `miopunch-poc-control-plane-wire-format` 仅是 `legacy/v0` 历史合同，不再约束当前 v1 peer-targeted 消息。
- 给 01 增加真实实施任务、测试与验收项，而不是继续保留“Done/Freeze”式占位文档。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-controlplane-wire`: 当前 POC v1 peer-targeted 控制面 wire/security 的唯一事实源。

### Modified Capabilities

- `miopunch-poc-control-plane-wire-format`: 明确其仅保留给已归档的 POC v0 JSON/AES-GCM 历史语境，不再作为当前 v1 的 source of truth。

## Impact

- 计划新增代码：`internal/pocv1/wire/*`、`internal/pocv1/peere2e/*`、相关 `testdata` / golden vectors。
- 计划保留的 legacy 参考：`internal/controlplane/message.go`、`sign.go`、`encoding.go`、`msg_id.go` 等历史实现。
- 不包含 invite/enroll body、presence、dial/punch 策略、session recipe 或 GUI/runtime 编排；这些由后续 `02/03/04/05/07` 拥有。
- 本次改动是 OpenSpec/docs-only；代码实现与测试在后续 apply 阶段按新的 tasks 执行。
