## 1. OpenSpec 与 charter 修正

- [x] 1.1 重写 proposal，明确本 change 是 code-affecting 的 POC e2e harness change，不是 docs-only
- [x] 1.2 重写 design，绑定现有 `lab/host/labctl` QEMU VM 流程和 VM 内 Docker 拓扑
- [x] 1.3 重写 delta spec，覆盖 host 命令、guest harness、Docker 拓扑、systemd node、broker、selftest/fulltest、LocalAPI WS helper、artifacts 和范围边界
- [x] 1.4 用中文重写 `docs/decisions/poc-e2e-container-lab-charter.md`，包含实现拓扑、用例清单、artifacts 和验证合同

## 2. Host labctl 集成

- [x] 2.1 在 `lab/host/labctl` usage 和命令分发中新增 `poc-e2e-selftest`、`poc-e2e-fulltest`
- [x] 2.2 两个 host 命令都复用现有顺序：`cmd_up`、`cmd_wait`、`cmd_wait_guest`、`cmd_push_guest`、`cmd_push_bin`、guest runner、`cmd_pull_artifacts`
- [x] 2.3 确保 guest runner 失败时仍拉取 artifacts，并保留 guest runner 的最终 exit code
- [x] 2.4 更新 `lab/README.md`，说明新 POC e2e 命令和 artifact 预期

## 3. Guest Docker harness

- [x] 3.1 新增 guest runner：`mlab-poc-e2e-selftest` 和 `mlab-poc-e2e-fulltest`
- [x] 3.2 新增共享 guest helper，处理 run dir、Docker network/container 命名、命令捕获、JSON/report 提取、readiness wait、artifact 收集、redaction 检查和 cleanup trap
- [x] 3.3 新增 systemd node Docker assets，包含 Debian、systemd、tmux、jq/curl、journal 工具、诊断工具和 miopunch binaries
- [x] 3.4 新增真实 `mosquitto` broker 启动逻辑，Docker DNS endpoint 固定为 `broker:1883`
- [x] 3.5 新增 node 启动逻辑，固定 systemd PID 1、`/var/lib/miopunch` 独立 state、`/run/miopunch/localapi.sock`、cgroup/tmpfs 参数和 per-node artifact 路径

## 4. Product-path selftest 用例

- [x] 4.1 实现 `node-a`、`node-b` daemon install/start readiness：`miopunch install-system-daemon`、`systemctl is-active miopunch`、system socket LocalAPI probe
- [x] 4.2 实现 broker state pinning：写入 `node-a` `/var/lib/miopunch/state.json`，使 `local.mqtt_broker=broker:1883`
- [x] 4.3 实现 `node-a invite --mode approve --uses 1 --expires 15m`、并行 `node-a approve <code>`、`node-b join <code>`，并导出 reports
- [x] 4.4 从 reports 和 state snapshots 提取并断言 `peer_id`、`net_id`、membership bundle 证据、identity、net、governance head、decls 和 seed peer state
- [x] 4.5 实现 `node-b ping <node-a-peer-id>` 和 `node-b sh ls <node-a-peer-id> local` 断言
- [x] 4.6 在 `node-a` 创建稳定 tmux session，并通过 lab helper 验证 `node-b` LocalAPI WebSocket `sh_attach` marker bytes
- [x] 4.7 实现 `node-a revoke <node-b-peer-id> --dangerous`，并断言后续 member 访问因 authorization/hello/revoke 被拒绝

## 5. Fulltest 诊断用例

- [x] 5.1 使用 `node-c` 实现 approve 缺失 timeout 和错误 approver rejection
- [x] 5.2 实现 invite max-uses 和 invite expiry，并断言失败原因确定
- [x] 5.3 实现 daemon restart 后 identity、net、governance、decls、ping 和 shell listing 持久化检查
- [x] 5.4 实现 broker outage 诊断，并断言 reports 指向 broker reachability
- [x] 5.5 实现第二 member 用例，证明 revoke 一个 member 不影响另一个 approved member
- [x] 5.6 实现 single-writer shell attach conflict，并断言 `SH_IN_USE` 或当前 shell lock reason
- [x] 5.7 使用现有 product state/config knob 实现非默认 data protocol smoke
- [x] 5.8 实现 `--redact` report 检查，覆盖 invite code、secret key、net secret、invite secret
- [x] 5.9 实现 fulltest packet capture，并产出 broker 非 data-plane relay 的 shell marker 证据

## 6. LocalAPI WebSocket lab helper

- [x] 6.1 在 `tools/miopunch-poc-e2e` 新增 `sh-attach` helper 命令（构建后可用 `miopunch-poc-e2e` 运行）
- [x] 6.2 helper 连接 `unix:/run/miopunch/localapi.sock`，创建 `sh_attach`，用 subprotocol `miopunch.sh.v0` dial `/api/v0/tasks/{task_id}/ws`，发送 marker bytes，并等待预期输出
- [x] 6.3 helper 输出机器可读 JSON，失败时输出有用 stderr 诊断，并对 setup/send/read/task 失败返回非零 exit code
- [x] 6.4 参考现有 localapi test pattern，为 helper 参数校验和 WebSocket 协议行为添加 focused Go tests

## 7. Artifacts 与 cleanup

- [x] 7.1 每个 case step 收集 stdout、stderr、exit code、report markdown 和 parsed summary
- [x] 7.2 收集 daemon journals、`/var/lib/miopunch` snapshots、broker logs、`docker inspect`、network inspect、image metadata 和 cleanup logs
- [x] 7.3 确保成功和失败路径都执行 cleanup，并记录匹配 run-id 的容器/网络残留
- [x] 7.4 确保 selftest 不要求 packet captures，fulltest 必须要求 pcap artifacts
- [x] 7.5 确保用于评审的 artifacts 已脱敏，或有明确 secret handling 说明

## 8. Verification

- [x] 8.1 运行 `openspec status --change poc-e2e-container-lab-charter`
- [x] 8.2 运行 `openspec validate poc-e2e-container-lab-charter`
- [x] 8.3 运行 `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 8.4 运行 `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 8.5 运行 `bash scripts/check_no_xtcp_imports.sh`
- [x] 8.6 运行 `./lab/host/labctl poc-e2e-selftest`
- [x] 8.7 运行 `./lab/host/labctl poc-e2e-fulltest`
- [x] 8.8 mainline 合入前，除新增 POC e2e 命令外，还要运行 `$dev` 要求的现有完整 lab gate set
