# MIOPUNCH-145 原生普通业务按流分发

正式 `NativeClient` 的文件、终端、剪贴板、TCP/UDP 转发、Web HTTP/WebSocket 和业务 PING 在同一对已准入原生成员之间共用由节点持有的 QUIC 连接。融合准入与专用协议共用节点原有的 iroh 端点和 relay 注册。接收方从各条业务流识别服务，并由现有服务权限和运行时检查授权。文件、终端和 Web 等服务各自持有可取消的流及其对端上下文。节点生命周期和成员撤销结束共享连接。

浏览器与访客继续使用其服务 ALPN。浏览器侧可接收的文件、剪贴板保留 ALPN 协商不支持融合时的原协议，访客使用其当前受邀服务。策略同步、设备管理、入网和恢复保持独立准入。只有显式启用的受限联系候选、且无获准数据候选或现有共享尝试时，正式入口直接使用已有的联系到数据路径。MIOPUNCH-148 将在融合入口负责这一步。

原 relay 竞速在应用只广告专用协议时，会把请求的 `miopunch/service-policy/1` 改成通用成员 ALPN。真实 relay 单路径上的策略同步因此收到 TLS `no_application_protocol`。竞速现在传递原调用方的 ALPN，供策略同步和保留的服务专用入口使用。

## 正式 SDK 入口

- 文件和 PING 的并发首拨在同一条连接上取得结果。独立读回的落盘文件与输入二进制字节完全相同。文件结束后的又一次 PING 在原连接返回 `PONG`。
- 两端分别读取转发状态，确认 TCP 和 UDP 对应同一条本端连接上的不同流。真实转发套接字验证了两种业务各自的字节。主动取消 UDP 流后，原 TCP 流继续收发。新 UDP 流也实际收发后，提供方保存撤权规则并终止该活流。后续 UDP 请求被拒，真实 UDP 目标没有收到请求。原 TCP 流照常收发且两端连接标识保持一致。
- 管理员通过正式设备操作撤销成员并读回 `effective_locally`。两端的 TCP 套接字读到 EOF。被撤销节点重新拨号无法取得业务 PONG。
- 冷启动时只给出已启用的 MQTT 联系地址。真实受保护协商在原候选连接上建立并验证 UDP 数据路径，TCP 转发从实际目标取回响应字节。
- 另一组成员端点只广告专用文件 ALPN。正式原生文件入口通过原协议完成发送，接收方独立核对文件字节，发送方取得对应回执。

## 高 RTT、中继上的并发大小流

用所有测试端共用的回环 relay，每条代理 TCP 单腿增加 350 ms。小请求使用正式 PING 的 5 秒原期限。预热后在同一条连接、同一拓扑下先单跑 PING，再在 64 MiB 文件流持续写入时发送同一 PING。两次均核对选中中继与收到的 `PONG`。排队从 SDK 调用开始，止于提交 PING 和 FIN，响应从提交到完整回复。

| 场景 | 排队 | 响应 | 提供方的大流进展 |
| --- | ---: | ---: | --- |
| PING 单跑 | 0.143 ms | 1409.652 ms | 无大流 |
| PING 与文件并发 | 0.231 ms | 1493.124 ms | 524288 → 851968 字节，窗口内观察到 5 次增加 |

首轮在提供方完成首个 64 KiB 文件块时启动小请求，窗口内文件增加到 128 KiB，未达到多次进展断言。复测在已收 512 KiB 后测量稳定写入阶段，沿用原拓扑、期限、字节断言与多次进展断言。日志分别在 `/tmp/miopunch-145-acceptance.log` 和 `/tmp/miopunch-145-high-2.log`。

上述五项专项同批通过，结果见 `/tmp/miopunch-145-final-acceptance.log`，编译回执见 `/tmp/miopunch-145-checked/receipt.json`。

验收需要本机回环 socket。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-145-services -- \
  test --locked --offline -p miopunch-native-probe --test native_service_streams \
  --jobs 2 -- --test-threads=1 --nocapture
```

## 兼容回归

受限构建器的自定义载体白名单在融合准入选择数据路径前，与应用允许的载体求交集。双端只用真实 TCP 自定义载体。修复前，空白名单仍交付业务连接。修复后，空白名单拒绝融合交付，重合名单完成双向业务字节，专用 ALPN 在同一端点完成双向受限交换。共享及独立 relay、专用文件 ALPN 和原受限入口回归也通过。红绿日志分别在 `/tmp/miopunch-145-custom-red.run.log` 和 `/tmp/miopunch-145-custom-green/`。

原有 MIOPUNCH-144 双进程共享专项 3 项通过，另有 1 项仅供专项启动的工作进程用例。MIOPUNCH-132 原高 RTT relay 对照在 25 ms 和 350 ms 两种单腿时延下均于原有 6 秒期限内返回正确字节。运行日志分别为 `/tmp/miopunch-145-144-regression.log` 和 `/tmp/miopunch-145-relay-control.log`。

已有终端生命周期、16 个并发附连的容量约束和输入输出背压通过。转发取消、满容量释放和已配置访客重开通过。剪贴板在隔离 Xvfb 的真实系统选择区通过待启动取消测试。相关日志在 `/tmp/miopunch-145-regress-final/` 与 `/tmp/miopunch-145-diagnose-clipboard.log`。

Web HTTP 正常回复、公开请求被丢弃和两端关闭触发的目标 EOF、身份重开与 WebSocket 压力关闭通过。等待 HTTP 目标时，停读通知终止工作。回复已发送时，应用回执决定正常完成。首次取消所缺的目标 EOF 与修复结果分别记录在 `/tmp/miopunch-145-diagnose-web.log` 和 `/tmp/miopunch-145-web-eof-web.log`。

宿主命令验收使用自有 relay，并使用现有桌面宿主入口线程和 Tokio worker 的 8 MiB 栈配置。测试通过独立读回 28 字节文件、停止实际待回复 HTTP 请求、重新打开同一身份和明确停止。主机的 76 条直连地址超过邀请的 32 条上限，验收资源显式关闭了直连。日志在 `/tmp/miopunch-145-host-stack-host.log`。原生 `rtc-probe` 类型检查通过，回执见 `/tmp/miopunch-145-native-rtc-typecheck/receipt.json`。

当前浏览器 WASM 验证遇到 [144 已记录](native-iroh-member-fusion-144.md#兼容回归) 的 `noq_proto` 引用和 `improve_existing` 缺 `seal_hello` 两项编译错误，回执见 `/tmp/miopunch-145-wasm/receipt.json`。
