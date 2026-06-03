# Roadmap

## 文档状态

- 当前版本收束到 `POC v1` 当前主线。
- 旧 `P0/P1/P2/P3/MNT/XTCP/TCP Door-2` 路线保留为历史/延期参考，不再作为当前 active OpenSpec specs 或当前验证 gate。
- 当前事实源：
  - `docs/decisions/poc-v1-charter.md`
  - `openspec/specs/miopunch-poc-v1-current-mainline/spec.md`
  - `openspec/specs/miopunch-poc-v1-*`

## 当前主线：POC v1

`miopunch` 当前主线是面试/demo-ready 的 P2P remote-control POC。

当前能力边界：

- shared daemon + LocalAPI
- CLI product verbs: `up`, `init-network`, `invite`, `approve`, `join`, `ls`, `ping`, `sh ls`, `sh`, `revoke`
- desktop GUI control console
- Android control-lite APK
- POC v1 wire/enroll/persist/presence/punch/session/runtime
- UDP direct-first path establishment
- UDP punching fallback
- KCP + TLS 1.3 + yamux secure session
- remote shell demo flow

当前 POC v1 pathing 是 UDP-only。`tcp_only` 是 explicit unsupported scope，不能静默回落到 UDP。

## 当前验证口径

当前主线验证以 host checks + 真实 demo evidence 为准。

Host checks:

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
openspec validate --all --strict
```

真实 demo evidence:

- Android/Linux 或 GUI/Linux 网络创建、加入、批准
- `ls`
- `ping`
- `sh ls`
- interactive `sh`
- `selected_path=direct_ipv4|direct_ipv6|punching_ipv4`
- daemon/app logs 和 CLI/report evidence

VM lab gates 暂缓，不作为当前 POC v1 必过项。

## 历史/延期路线

以下路线保留为历史和未来设计参考，但不再是当前 active specs：

- `P0` NAT lab testbed
- `P1` XTCP kernel extraction
- `P2` XTCP connectivity enhancements
- `P3` generic dataplane experiments
- `P3.5` public-network reachability experiments with cn/global STUN arbitration
- `MNT-01/02/03` mainline network test gates
- TCP Door-2 / TCP punching
- POC v0 product/control-plane specs

对应旧 OpenSpec specs 已移入：

```text
archive/openspec-specs/2026-06-04-pre-pocv1/
```

恢复任何旧路线为当前 gate 前，必须新建 OpenSpec change，重新定义其与 current POC v1 的关系、验证范围和验收 evidence。

## 下一阶段优先级

1. 保持 POC v1 CLI/GUI/Android demo 可重复演示。
2. 收敛 active OpenSpec changes，避免 main specs 与 completed changes 分裂。
3. 补齐真实 demo runbook 和证据路径，减少每次手工复测成本。
4. 如确实需要 VM lab，先设计 current POC v1 lab gate，而不是直接恢复旧 XTCP/MNT gate。
