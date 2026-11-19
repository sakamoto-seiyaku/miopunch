# MIOPUNCH-146 原生重启后的成员首拨

正式原生业务沿用节点持有的成员连接。需要新建时，应用提供仍获许可的活跃路径、联系记录、当前地址和 relay，融合端点持有本轮绝对期限、候选调度及取消。先尝试活跃直连。它不可用时，取 300 秒窗口内按观察时间排序的一条已验证缓存 IP 和端口，与获准 relay 并行启动。尚未验证和过期的地址留给后续尝试。首选 relay 与第二条的起拨间隔取 250 毫秒和剩余期限四分之一中的较小值。某路腾出名额后按顺序继续，relay 最多两路在途。

每路直接拨指定物理地址。原有 relay actor 在指定 URL 尚未建立 actor 时，可以借用已知对端的另一条连接。四路夹具曾记录静默尾部候选通过其他 relay 进入真实 TLS 成员准入。本次将精确 relay 限定在融合候选自己的 NOQ sender 中，随候选结束释放。普通 sender 和成连后的传输继续使用原有替换规则。

原成员连接完成身份和成员准入，并确认实际数据路径后，才参与交付。交付在最后一次验证后将 permit 的真实远端地址保存为 `initial_data_path`。应用以该值学习持久联系，后续 PING 状态也分别报告该值和读取时选中的路径。错误缓存端口上的其他 TLS 公钥通过 `unknown_ca` 淘汰该条候选。已认证目标明确拒绝准入时结束本轮并取消余下尝试。

原 IPv6 实测中，两端通过成员认证后，QNT 在准入释放前新建一条 IP 路径并关闭原资格路径。最后一次 permit 复核失败。融合 IP 首拨现保留物理 bootstrap 直至每侧完成最终准入复核，随后恢复原选路。复测仍从同一 NativeClient PING 入口使用实际绑定的 IPv6 端口。

## 正式 NativeClient 入口

- 两个原生身份正式入网。通过原生 PING 在 IPv4 路径完成真实往返并保存验证记录。关闭并重开同一个拨号身份，在输入只有目标 ID 时，已验证缓存端口与本地获准 relay 同时起拨。缓存 IP 首先交付，relay 取消。活跃连接上的下一笔 PING 继续使用同一连接标识。
- 关闭并重开目标，保留它的身份。旧端口的独立 UDP socket 收到 QUIC Initial，relay 在原期限内交付，目标发送 PONG。再让另一个真实 iroh 设备绑定旧端口并接收融合 ALPN。发起端记录真实 TLS 公钥不匹配，正确 relay 继续交付。部分样本在第一份 PONG 前已在同一原连接验证并选中新直连，诊断同时保留交付时的 relay。
- 按相同输入验证 300 秒窗口、验证标记和观察时间。记录文件先放较旧验证地址，再放较新验证地址，首拨采用较新地址。仅公告及 301 秒的地址在有 relay 时延后，移除可用 relay 后过期地址仍可参与并取得 PONG。停用 UDP 后，即使保存近期验证记录也只起拨 relay。
- 在发起端关闭期间，目标以正式设备操作应用成员移除。发起端重开后仍保留旧本地成员视图。目标完成 TLS 身份交换后明确拒绝准入，发起端收到 `PermissionDenied`，同轮 relay 被取消。
- 从相同 NativeClient PING 入口实测 IPv4 回环、宿主实际 LAN 和绑定的 IPv6 套接字。测试核对对端身份、真实 PONG 和该连接的准入承载。

## 错峰与现有连接所有权

用透明代理延迟 relay 的实际 TCP 转发。900 毫秒及 4 秒的用例均核对每条候选接收同一个绝对期限、两路在途上限、指向目标的实际双向请求和回复以及所有尾部候选收尾。4 秒组让前两个 relay 静默，由后续可用 relay 交付，尾部静默候选已起拨并在收到业务回复前取消。

最终样本的错峰分别为 226.41 毫秒和 251.16 毫秒，计算值分别为 224.95 毫秒和 250 毫秒。两组各只有可用 relay 的一个候选完成 TLS 并进入成员准入。

首次回归在互拨的获胜入站连接发布时，立即取消了仍在处理原双边裁决的单条出站连接。对端的接受入口观察到这次提早结束的底层候选。取消现已作用于显式路由竞速的余下尝试，单条映射拨号仍通过原双边裁决结束。MIOPUNCH-144 的相同真实跨进程测试继续核对两端收敛、落选及单次业务交付。

## 复现

原生测试使用本机 TCP、UDP 和真实 relay。回执及测量写入各次运行指定的目录。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-146-native -- \
  test --locked --offline -p miopunch-native-probe --test native_service_streams \
  --jobs 2 -- --test-threads=1 --nocapture

rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-146-owner -- \
  test --locked --offline -p miopunch-native-probe --test resources \
  --jobs 2 member_fusion:: -- --test-threads=1 --nocapture

rtk proxy python3 scripts/cargo_cache.py run --output /tmp/miopunch-146-ipv6 -- \
  test --locked --offline -p miopunch-native-probe --lib --jobs 2 \
  native_client::tests::native_client_pings_a_member_over_the_bound_ipv6_socket \
  -- --exact --nocapture

rtk proxy env MIOPUNCH_TEST_LAN=192.168.4.5 \
  python3 scripts/cargo_cache.py run --output /tmp/miopunch-146-lan -- \
  test --locked --offline -p miopunch-native-probe --test native_service_streams \
  --jobs 2 native_ping_keeps_lan_candidates -- --exact --ignored --nocapture
```

LAN 用例的 `MIOPUNCH_TEST_LAN` 取执行宿主的实际 IPv4 地址。

## 本机证据

- 最终原生联系重启、300 秒窗口、错身份和真实拒绝的完整时间线为 `/tmp/miopunch-146-final-native-redial.log`。
- 宿主 LAN `192.168.4.5:48443` 与 IPv6 `[::1]:46284` 均在所拨的 IP 承载上交付并返回 PONG。日志为 `/tmp/miopunch-146-final-lan.log` 和 `/tmp/miopunch-146-final-ipv6-pass.log`。
- 最终物理 relay 时间线为 `/tmp/miopunch-146-final-physical-relay.log`。原 144 跨进程回归为 `/tmp/miopunch-146-final-owner.log`。
- 最终普通 iroh 接口 2 项、受限业务及撤权 1 项、原有成员联系 3 项通过。日志为 `/tmp/miopunch-146-final-ordinary.log`、`/tmp/miopunch-146-final-restricted.log` 和 `/tmp/miopunch-146-final-contacts.log`。共享构建复用了 463 个产物并重建 11 个，回执为 `/tmp/miopunch-146-final-artifacts/receipt.json`。
- 同一正式 PING 夹具在缓存 IP 完成目标 TLS 身份验证，收到 SCRAM 首帧后重置控制流。修复前误判为明确拒绝并取消同轮 relay。修复后该 IP 记录为传输失败，健康 relay 交付原连接，3.515 秒收到目标 PONG。已认证目标的明确成员准入拒绝在 107 毫秒终止整轮，取消 relay，业务未交付。红绿时间线分别为 `/tmp/miopunch-146-scram-red.run.log` 和 `/tmp/miopunch-146-scram-green.run.log`。
- 最终浏览器 WASM 定向检查退出 101。两项诊断及主定位均与 145 相同。E0433 位于 `src/connection_coordinator.rs:1043`，无法解析 `noq_proto`。E0061 位于 `browser-probe/src/member_connection.rs:145`，调用传入六个参数，签名要求七个。对照回执为 `/tmp/miopunch-145-wasm/receipt.json` 和 `/tmp/miopunch-146-shared-wasm/receipt.json`。
