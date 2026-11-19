# MIOPUNCH-144 普通成员共享连接

原生融合端点以本网络的持久身份为边界，为每个已认证对端持有一条普通成员业务连接。同端等待者共用进行中的首拨，各自保留截止和取消。已有连接直接开流，每条业务流持有独立的活动租约。最后一条业务流释放后，所有者按本节点配置的探索间隔收尾。周期 ping 使用无需活动租约的维护流，两端均不因此重置空闲时钟。

融合 ALPN 为 `miopunch/session/member-fusion/2`。沿用 SCRAM，原有准入阶段的一次双向交换在实际数据路径通过检查后完成。已认证 ID 较大的一端在候选上发 `READY`，接受较小一端的裁决。较小一端在获胜候选上发 `COMMIT`。两条合格互拨候选同时待交付时，较小 ID 发起的一条胜出。两端通过原连接交付业务流，落选候选关闭。Replica、访客服务、入网和恢复使用各自的 ALPN。

## 双进程回环 relay

夹具先通过正式手动准入保存两个节点的身份和成员状态。运行时使用实际 TLS、准入、获准 iroh relay 和 QUIC 业务流，分别核对两端 `stable_id`、单调时间、已认证身份和双向有效载荷。跨端用同一握手的 TLS exporter 关联。可在输出中按 `fusion-first-reply`、`fusion-wire`、`fusion-idle` 和 `member-fusion-admission-stage` 查找记录。

- 同端两个并发等待者各取得一条流，随后再次拨号也使用同一个 `stable_id`。同端只观察到一次实际 TLS 成功后进入成员准入。一起参与的取消和短截止调用分别得到明确错误，其他两个等待者继续交付。撤权后的实际重拨在准入阶段被拒绝。
- 两种单向首拨方向都只有 `READY` 和 `COMMIT` 两条准入阶段消息。首条业务流的 QUIC stream index 也核对了 SCRAM 和现有准入阶段所用的控制流。专门用于仲裁的额外往返为零。
- 互拨时两端分别观察到两个 TLS 握手进入准入且准入区间重叠。较小 ID 一端先看到两条合格候选的 `READY`，再提交自己发起的候选。两端的首份业务回复沿共同胜者，落选候选双方均记录退出，连接接受入口未收到第二条。
- 一条测试流遭主动 reset 后，另一条流保持活跃超过空闲预算，随后仍收到正确回复。最后一个活动租约释放后，两端继续定期交换实际 ping 和回复，空闲较短的节点按本机预算发出关闭，另一端收到其空闲关闭原因。
- 已准入的独立端点分别尝试在融合端点协商 Replica、终端服务、入网和恢复协议，均失败。

以下记录取自 `/tmp/miopunch-144-final-native/cargo.stderr.log`。单向与互拨样本使用同一对已认证节点，ID 前八位分别为 `b1a3f544` 和 `d049f927`。毫秒值使用各端本地单调时钟。

| 场景 | 跨端 attempt ID | 较小 ID 本端 stable_id / 首份业务回复毫秒 | 较大 ID 本端 stable_id / 首份业务回复毫秒 |
| --- | --- | --- | --- |
| 较小 ID 单向首拨 | `8d2701ee878d958b87d443c6ec22f9cf` | `138812875171728` / 449 | `129600505905440` / 412 |
| 较大 ID 单向首拨 | `29e04ec34e1944044c70452918ea7b8d` | `132143997374928` / 471 | `132459073490096` / 401 |
| 互拨胜者 | `253c1363f0b3c1b399e0b1469e7449da` | `131912137805808` / 787 | `136917921767536` / 771 |

互拨落选候选为 `b8c1971f582bdcaaba9836dfc07e275c`，两端分别在本地 756 ms 和 769 ms 记录 `superseded`。空闲样本 `bdc5247d51274c86c416d156b1651ee6` 在 600 ms 配额下一端于 601 ms 收尾。两端各确认发送和接收了十次周期 ping。

复现命令需要允许本机进程绑定回环 socket。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-144-two-process -- \
  test --locked --offline -p miopunch-native-probe --test resources --jobs 2 \
  member_fusion:: -- --test-threads=1 --nocapture
```

## 兼容回归

原生宽回归共 116 条通过，4 条既有失败。普通 iroh 消费者 2 条、受限端点 19 条全部通过。`member_attempt` 的固定 350 ms 用例和 `resources` 的 p2panda mDNS 发布用例与 [143 冻结记录](native-iroh-member-fusion-143.md#普通入口与受限入口) 相同。库内另外两条在 `8fd94f90` 冻结源码上也重现，分别在断言原连接仅有 relay 路径和关闭活动 IP 后等待 relay 回复时失败。原生回执见 `/tmp/miopunch-144-final-native/receipt.json` 与 `/tmp/miopunch-144-frozen-lib/receipt.json`。

浏览器 WASM 按 143 的同参数构建，仍因既有 `E0433`（`noq_proto`）与 `E0061`（`improve_existing` 缺 `seal_hello` 实参）退出 101。两条诊断的错误码、消息、源码路径和行号与 143 相同。回执见 `/tmp/miopunch-144-final-wasm/receipt.json`。
