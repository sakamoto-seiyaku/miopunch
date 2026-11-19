# MIOPUNCH-143 首个成员回复

原生应用显式绑定融合端点，提供持久设备身份、由 p2panda 成员记录更新的准入策略、获准数据 relay 和本地建连预算。首票装配原生 iroh 数据 relay，成员业务走专用 ALPN `miopunch/session/member-fusion/1`。普通调用方继续用 `Endpoint::builder(…).bind()`。新 ALPN 沿用 SCRAM 和双边准入阶段。浏览器和旧原生成员 ALPN 继续使用各自的既有入口。[`NativeNetwork::bind_member_fusion_endpoint`](../../src/native_network/member_fusion.rs#L27)

iroh 使用正常 connect、accept 和 QUIC 流。端点从实际路径中取获准 relay，在同一连接取新鲜 ACK，通过交付前复查才交出原连接。建连、准入和交付共用一个绝对截止，端点接受调用方更早的截止及取消信号。建连守卫在失败、超时或调用丢弃时收回未交付的连接。资源策略由 iroh 的发送和资格检查执行，当前成员许可由应用复查。应用取得普通 `Connection` 后直接开流。单个候选的建连由端点处理。[`Builder::member_fusion`](../../vendor/iroh/src/endpoint.rs#L976) · [建连期限](../../vendor/iroh/src/endpoint/restricted.rs#L621) · [建连守卫](../../vendor/iroh/src/endpoint/connection.rs#L358)

## 行为基线对应入口

| 基线 | 旧触发及核心断言 | 融合验收入口 |
| --- | --- | --- |
| 125 | 实际首拨、单流失败及显式 repair 以阶段、关闭来源和本端 stable_id 对照。[旧拨号/关闭时间线](../notes/2026-11-17-connection-flow-improvement-design.md#5-miopunch-125时间线与关闭来源) | 首票沿正常 connect/accept 核对两端准入、relay 确认、首个回复及撤权拒绝。149 对照候选实际起拨和结算，154 对照原流恢复或连接终止。 |
| 126 | 后台联系占用时启动前台拨号。同一连接的旧 repair 和 active 改善同时触发，仅一个拥有者能开路。[前后台测试](../../src/connection_coordinator_active_tests.rs#L876) · [旧受保护 repair](../../tests/path_maintenance.rs#L362) | 144 用融合 connect 验证并发首拨，151 用持续业务流触发双端改善并验证同一轮占用，154 用原流恢复核对失败后生存。 |
| 127 | demo 令单流读写失败或请求超时，另一条流及后续请求继续。保持同一 IP 变更真实路由时只唤醒一次。[旧流处理](../notes/2026-11-17-connection-flow-improvement-design.md#7-miopunch-127demo-错误隔离与网络通知) · [同 IP 通知](../../src/bin/network_demo/network_notification_test.rs#L61) | 145 从实际服务流验证单流终止隔离，154 从携带数据的原流验证真实切网通知及结果。 |
| 128 | 注入高 RTT 或停在 Hello/Result，沿原截止退出。错 nonce/类型保持协议错误，候选受阻让其他项继续。[公开 UDP 截止](../../tests/legacy_nat_runtime.rs#L130) · [错 Result](../../src/connection_coordinator_active_tests.rs#L1425) | 首票沿原截止完成准入及 relay 验证，明确拒绝结束本次业务拨号。147/148 在融合端点控制交换复用时间、候选及错误断言，149 在首拨入口复用取消和期限断言。 |
| 129 | 持有真实业务租约和原流，已直连项跳过，合格 relay 会话取得改善机会。发现与同步按租约暂停及恢复。[原租约测试](../../tests/network_maintenance.rs#L784) | 145 使用独立业务流的持有期限，151 在双向实际业务与空闲维护条件下从融合端点验证公平改善。 |
| 130 | 同代和换代分别检查增强名额、完整观察和发包上限，驱动 Hello 碰撞及已有分步公开入口。[轮次预算](../../tests/legacy_nat_runtime.rs#L511) · [实际 Hello 交接](../../src/connection_coordinator_active_tests.rs#L2094) | 156 复现原版争用及结算，151 走融合双端名额入口，157 验证实际观察，150 以胜出 socket、原连接的新鲜 ACK 和业务回复结算。 |
| 131 | repair 在 relay 停顿时先到期，原连接仍有获准路径时保留原流，解除停顿后复验。唯一载体被撤时读到终止。[原流留存](../../src/connection_coordinator_active_tests.rs#L303) · [移除 relay](../../tests/network_maintenance.rs#L156) | 154 从融合端点保持业务并切断承载，分别观察恢复或终止。153 以允许实际转发数据的成员路径作承载对照。 |
| 132 | 保持直连注册并制造双向黑洞，健康备份新鲜 ACK 后，原流在既有 6 秒期限内补齐正确字节。解除黑洞并取得新鲜 ACK 后重获选路资格。[低/高 RTT 黑洞组](../../tests/path_maintenance.rs#L1326) | 147 从健康 relay 验证原流换到 UDP，154 在同一条业务流复用旧黑洞触发、截止及回落断言。 |

## 双进程实测

先通过正式手动准入将身份持久化。两个独立原生进程用相同的获准 `http://127.0.0.1:45897/` iroh relay 建连。两端读取经过 TLS 验证的对端身份，确认该 relay 是实际选中路径，核对 `fusion ping 143` 和 `fusion pong 143`。应答端随后撤销拨号端成员资格。拨号端仍以旧凭据重拨，应答端在真实准入阶段返回 `PermissionDenied`，两端均未得到普通连接。下表来自最终复验的 `/tmp/miopunch-143-final-native/cargo.stderr.log`。[`resources` 集成场景](../../tests/support/member_fusion.rs#L24)

| 结果 | 跨端 attempt / round | 拨号端本地 stable_id / 单调毫秒 | 应答端本地 stable_id / 单调毫秒 |
| --- | --- | --- | --- |
| 业务回复 | `0736ca63bb8a08dbe50c0a6ebb35a12a` / 1 | `125917503180672` / 准入 30–370，relay 确认 400，读取回复 402 | `136896445872064` / 准入 371–699，relay 确认 706，回复确认 734 |
| 撤权后拒绝 | `7a7a0857eef8add329341006c8c7539b` / 1 | `131772483374400` / 准入 30，收到关闭后停止 42 | `136896445846688` / 准入及明确拒绝 907 |

共同尝试标识使用专用 TLS exporter 标签生成。准入证明使用独立的 TLS binding。端内阶段共用 125 的 `localAttemptId` 和单调时钟。跨端配对使用 `attemptId` 与 `round`。表中的 stable_id 与时间数在各自进程中读取。

复现命令需要允许本机进程绑定回环 socket。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-143-two-process -- \
  test --locked --offline -p miopunch-native-probe --test resources --jobs 2 \
  member_fusion::approved_member_gets_first_relay_reply_and_revoked_member_cannot_open_a_stream \
  -- --exact --nocapture
```

## 普通入口与受限入口

原版 `ordinary_endpoint` 两个消费者均通过，分别覆盖 early data/0-RTT 和普通地址、路由、hooks 与 socket 方法。最终复验回执为 `/tmp/miopunch-143-final-native/receipt.json`。既有 `restricted_endpoint` 的 19 个用例全部通过。四个原生集成目标各自的结果见 `/tmp/miopunch-143-native-serial/cargo.jsonl`。

额外检查中的两条既有失败在冻结的 `00b6de27` 工作树用同一触发和断言重现。`member_attempt` 的固定 350 毫秒用例到期时未观察到双方进入 hook。`resources` 的 p2panda 高层 LAN 用例未观察到 active mDNS 发布。对照回执为 `/tmp/miopunch-143-native-baseline/receipt.json`。

## 浏览器共享源码编译

改动前在 `00b6de27` 实际构建浏览器 WASM。`694f004d..00b6de27` 的差异仅包含设计文档，编译输入相同。退出码 101，有两项既有错误，共享 `connection_coordinator.rs:1043` 引用未链接的 `noq_proto`，以及 `browser-probe/src/member_connection.rs:145` 调用 `improve_existing` 时少了 `seal_hello` 参数。基线回执为 `/tmp/miopunch-143-wasm-baseline/receipt.json`。

融合改动后的同参数构建也退出 101。两条诊断的错误码、内容、源码和行号均与基线一致。融合入口限定在原生目标。融合版回执为 `/tmp/miopunch-143-wasm-fusion/receipt.json`。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-143-wasm-baseline -- \
  build --locked --offline -p miopunch-browser-probe --target wasm32-unknown-unknown \
  --features rtc-probe --lib --jobs 2
```
