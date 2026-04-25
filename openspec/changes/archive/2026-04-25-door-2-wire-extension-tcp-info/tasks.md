## 1. Wire 扩展（TCP 字段）

- [x] 1.1 在 `NatHoleVisitor/NatHoleClient` 增加 `tcp_direct_addrs/tcp_mapped_addrs/tcp_stun_cn/tcp_stun_global`
- [x] 1.2 在 `NatHoleResp` 增加 `peer_tcp_direct_addrs/tcp_candidate_addrs/tcp_selected_view/tcp_selected_reason`
- [x] 1.3 预留 `tcp_punching_enabled/tcp_detect_behavior`（只加字段，不启用行为）

## 2. coordinator 派生（analysis）

- [x] 2.1 回传对端 `peer_tcp_direct_addrs`（dedup + host:port 过滤）
- [x] 2.2 派生 `tcp_candidate_addrs`（优先选中 view 的 mapped_addrs，否则回退 tcp_mapped_addrs；过滤 + dedup）
- [x] 2.3 在双方提供 TCP cn/global 观测时产出 `tcp_selected_view/tcp_selected_reason`

## 3. 单元测试

- [x] 3.1 `internal/wire` roundtrip：tcp 字段 encode/decode 不丢失
- [x] 3.2 `internal/coordinator`：派生/回传逻辑与 TCP view selection 的行为测试

## 4. 验证

- [x] 4.1 `go test ./...`
- [x] 4.2 `go vet ./...`
- [x] 4.3 `bash scripts/check_no_xtcp_imports.sh`
