# MIOPUNCH-153 成员数据中继参与备用档选路

融合拨号可以使用已附着、当前获准的 `MemberDataTransport`。成员数据中继和原生 relay 共用备用档及 iroh 的分档选路。融合所有者从现有联系地址取得候选，在已准入的原 QUIC 上验证。候选必须在该连接的实际绑定收到新鲜 ACK，才可参与选路并承载业务。原有成员信令只承载受保护联系和建路控制。

[三节点回归](../../tests/support/fused_member_data.rs) 复用真实节点入网和外层成员连接。两端先建立各自与第三成员的已准入连接，由第三成员明确启用数据转发并接受两份独立租约。业务两侧独立核对 TLS 对端、共同 attempt、本地 stable ID、路径与 STREAM 帧，第三成员独立记录实际转发。在线用例在同一双向流核对序号回显和双方流标识，拨号计数为 `[1, 0]`。

| 对照 | 实际验收 |
| --- | --- |
| 仅获准成员数据中继 | 两端通过各自对应的成员转发地址完成准入与首次业务，相互核对完整请求和回复。第三成员转发计数在业务往返期间增加 |
| 原生 relay 每段增加 65ms | 双方收到成员候选的 `PATH_RESPONSE` 后仍保留原生 relay，候选 ACK 暂缓期间 STREAM 帧为零。恢复 ACK 后，实际 RTT 低至少 5ms 的成员中继承载原流 |
| 快原生 relay | 以成员中继完成首次业务后，原连接验证新提供的原生 relay，双方选用实际 RTT 低至少 5ms 的原生 relay，并在该承载上继续原流 |
| 仅接受侧取得成员数据地址 | 接受侧在原连接的候选控制流提供经同一转发成员注册的本方地址。拨号侧验证后，两端在接受侧发起的原流上经成员中继继续收发 |
| 前两个已配置原生 relay 不可达 | 两个端口实际拒绝连接。第三个原生 relay 在原连接 `PathId(3)` 收到新鲜 ACK，两端从原流读到完整往返 |
| 同一转发成员的前两份租约已退役 | 两份旧注册各自经过 `PATH_RESPONSE` 且当时没有候选 ACK。接受侧更换自己的注册，继续使用刚刚提供过的对端地址。第三份当前地址在原连接取得新鲜 ACK，并经原流双向收发 |
| 两条备用路径均已验证，恢复主档 UDP | 恢复后的 UDP 有独立新鲜 ACK，通过双向原流字节证明其获选。每个方向的回环 UDP 增加 20ms 延迟，实际 RTT 因而未达到备用档内所需的 5ms 改善 |
| 仅准成员信令 | 复验 148 的正反例。有获准 UDP 时，两端从实际 UDP 承载收到业务字节。纯信令是唯一可用路径时报告业务路径不可用 |

最终样本在两个方向量得成员中继 89.9ms 和 99.5ms，慢原生 relay 271.3ms 和 279.5ms。反向对照的原生 relay 是 3.4ms 和 4.5ms，成员备用是 76.8ms 和 74.6ms。主档用例中两端选中的 UDP 为 43.2ms 和 29.3ms，原生 relay 为 3.8ms 和 13.5ms。首次成员中继业务期间，第三成员独立录得 5 次转发。原始记录位于 `/tmp/miopunch-153-final/` 的 `member-relay.log`、`primary.log` 和 `signaling-host.log`。

候选交接修复前，接受侧单独提供地址的用例等不到成员候选。第三个 relay 的用例只有两个不可达候选被创建。修复后，两项及同时受影响的成员数据、UDP 主档和原流 TCP 用例通过。接受侧发起的原流经成员中继时，两侧 RTT 为 81.8ms 和 85.9ms。第三个 relay 的 RTT 为 2.0ms 和 1.5ms。记录位于 `/tmp/miopunch-153-review-red/` 和 `/tmp/miopunch-153-review-green3/`。

租约更新用例复用了两个外层成员连接，把两份旧注册的路径证明与第三份注册区分开。初版在旧 `PathId(1)` 和 `PathId(2)` 退役后等不到第三份。修复后第三份由双方原 QUIC `PathId(3)` 承载，两侧各收 4 个后续业务 STREAM 帧，继续通过接受侧打开的原流核对有效往返。记录位于 `/tmp/miopunch-153-lease-red2/lease-red.log` 和 `/tmp/miopunch-153-lease-final/lease.log`。

152 的原流 TCP 六项、普通端点两项、受限端点 19 项及 149 的非忽略慢 relay 回归通过。WASM 检查的 E0433 和 E0061 与 152 的消息、位置完全一致。回执为 `/tmp/miopunch-153-wasm/receipt.json`。

## 复现

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-153-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test member_relay --test contact_member_priority --no-run --jobs 2
```

从 Cargo 回执选择 `member_relay` 测试产物。普通回环组运行 `fused_member_data:: --test-threads=1 --nocapture`。在另一临时网络命名空间启用 `lo`，用下列规则只给回环 UDP 加延迟，再运行 `fused_member_data::fused_direct_path_wins_with_both_data_relays_available --exact --ignored --nocapture`。用例在该临时命名空间安装并撤除两端 UDP 黑洞规则。

```sh
ip link set lo up
tc qdisc add dev lo root handle 1: prio
tc qdisc add dev lo parent 1:3 handle 30: netem delay 20ms
tc filter add dev lo parent 1: protocol ip pref 1 flower ip_proto udp flowid 1:3
```

`contact_member_priority` 按 [148 的验收拓扑](native-iroh-member-contact-148.md)，在真实宿主回环运行 `fused_member_forwarding --test-threads=1 --nocapture`。仅回环和带私有 veth 的临时命名空间均在旧手工入网前置遇到 `PathUnavailable`。
