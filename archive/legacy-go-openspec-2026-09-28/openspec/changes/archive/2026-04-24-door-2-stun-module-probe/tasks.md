## 1. STUN 复用模块（internal/stunclient）

- [x] 1.1 创建 `internal/stunclient` 包与最小 API（endpoint 解析/规范化、scheme 过滤、DNS 解析）
- [x] 1.2 迁移/复刻现有 UDP STUN 受限并发实现（单 socket read loop + txid 分发），并保证无 goroutine 泄漏
- [x] 1.3 增加 TCP STUN roundtrip（Dial + BindingRequest + 按 header length ReadFull + 解析 XOR-MAPPED-ADDRESS）

## 2. connectivity 复用与 endpoint scheme 支持

- [x] 2.1 重构 `connectivity` 的 STUN 解析/解析（resolve/normalize/discovery）以复用 `internal/stunclient`
- [x] 2.2 更新 endpoint 语义：接受 `udp://` / `tcp://` / dual（`host:port`）；UDP-only gather 必须忽略 `tcp://` 端点
- [x] 2.3 更新/补齐单元测试：endpoint scheme 过滤、解析错误路径、以及内置 buckets 的稳定性测试

## 3. miopunch-lab：stun probe 入口

- [x] 3.1 在 `miopunch-lab stun` 下增加 `probe` 子命令（不破坏现有 UDP STUN server 行为）
- [x] 3.2 实现 `stun probe` flags：`--builtin` / `--stun`、`--attempts`、`--ok-threshold`、超时/并发、内置 DNS flags
- [x] 3.3 输出 JSONL：每 endpoint 一条记录（包含 spec 要求的最小字段 + decision）

## 4. 证据采集与内置 STUN 列表更新

- [x] 4.1 运行 `miopunch-lab stun probe --builtin` 产出可复现证据（落到 `docs/reports/`，文件名含日期）
- [x] 4.2 基于证据更新 `connectivity/stun_internal.go`：标注 `udp://` / `tcp://` / dual，并移除明显不可用端点
- [x] 4.3 更新 `connectivity/stun_internal_test.go` 以匹配新的 scheme 标注与排序口径

## 5. 验证

- [x] 5.1 运行 `go test ./...`
- [x] 5.2 运行 `go vet ./...`（可选但推荐）
