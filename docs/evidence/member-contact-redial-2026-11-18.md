# MIOPUNCH-136：成员地址学习与重启重拨

权威规格为 Plane MIOPUNCH-136（父规格 MIOPUNCH-135）。本票代码从 `9d3db16c` 起实施，公开 SDK 合并当前活跃路径、已认证成员公告和保存线索。节点目录的 `member.contacts.json` 保存逐地址时间及来源；300 秒只是排序偏好窗口，不是授权寿命。过期线索降级，当前成员资格和资源许可在消费时重新检查。显式同步 `remember_member_contact` 保持同步；异步连接及维护入口的完整缓存操作在短期 blocking I/O 边界执行，沿用原绝对期限与取消。

## 固定双端、真实进程重启

两端分别在本机生成独立测试身份，通过一次性 precode 准入，未复制在用身份或传输私钥。方向为办公网 Windows u22sz → 家宽 r_proxy；Linux 独立端口为 47136，目标身份为 `ba4aa29742cf233466608cb2c1380ad6212cf3fb24639f1923469bdea19f40b3`。

用户明确授权借用 Simplewall 已放行的原 Windows demo 可执行路径；由协调者备份并停止原任务后替换纯测试程序，节点及配置保留。未修改 Simplewall 规则或产品 DNS。

| 阶段 | Windows PID | 实际结果 |
| --- | --- | --- |
| learn，同一已有身份续行 | 19996 | peer 身份及序号 13601 匹配，aps1 relay 有效回复；连接加请求 24223ms，进程 exit 0 |
| 前进程已退出后的独立 reopen | 19936 | 调用目标仍为旧 euc1 relay 和 `127.0.0.1:9`；首个实际拨号为保存的 aps1，序号 13602 匹配；23597ms，exit 0 |
| 再次独立进程，禁用 DataRelay | 20320 | 实际依次拨旧 `127.0.0.1:9` 及学到的 `172.17.0.1:47136`、`172.18.0.1:47136`、`172.19.0.1:47136`、`192.168.4.14:47136`，没有 relay；12006ms 后不可达，观察程序 exit 0 |

第三阶段证明缓存 IP 进入了真实拨号机会，没有证明这些私网地址在公网可达，也不声称公网 UDP 打通。可达路径上的有效回复由前两阶段证明，真实 UDP 恢复由既有本地两秒断言证明。

初始独立程序路径被防火墙限制时，euc1 与 aps1 联系均未到服务端；换到用户指定允许路径后，SDK lookup 得到 aps1 `5.223.65.62`，实际受限连接 7946ms 成功。第一次材料阶段的服务端已收到 SAVE 并记录 Complete，测试 fixture 过早释放控制连接导致客户端最后确认报 PathUnavailable；按既有协议保留已持久材料和身份继续，而非重建身份。fixture 已按正式 demo 契约在原截止内保留 control 至对端关闭，未改产品协议。

## 验证及审查

- 原公开 SDK 红绿：旧目标独占两秒截止；修复后同入口真实 UDP、身份及业务字节通过。保留近期失效地址的有界回落、过期、取消、撤权和禁用资源断言。
- 最后集中修复后完整 `tests/member_contacts` 目标使用同一既有最终二进制、`--test-threads=1` 串行执行，3 passed / 2 ignored，21.46s；两个远端 fixture 由上述真实双端运行覆盖。
- 新 I/O 负例：实际缓存 writer 锁持有 400ms，公开旧 ALPN 连接的 60ms 期限及时返回，公共 coordinator 取消约 61ms 返回，不阻塞 runtime。
- 新饱和缓存回归：8 条过期已验证地址下，认证会话的新地址及同地址重新公告均能在关闭、重新打开后经实际 UDP 收到业务字节；过期验证证明没有被公告续用。
- Browser 共享入口编译通过，收据 `/tmp/miopunch-136-reviewed-browser-check/receipt.json`，exit 0，295 Fresh / 1 rebuilt。
- 两名全新专属审查者分别审 Standards／Spec；初审 1／2 项 P2，集中修复后原审查者定向复核均无残留。完整原报告与复核位于 `/tmp/member136-review/review.md`。

原始现场日志：`/tmp/miopunch-136-acceptance/{learn.completed,reopen,ip}.{stdout,stderr}.log`。本地最终完整目标、I/O 和保留回归分别记录于 `/tmp/miopunch-136-final-target.log`、`/tmp/miopunch-136-cache-io-regression.log`、`/tmp/miopunch-136-cache-retention-regression.log`。后续只改变 I/O 执行边界和缓存新鲜度保留，没有重跑路由及授权语义不变的公网场景。

## 宿主收尾

三个 Windows 测试进程均确认不存在；Linux 独立 server 633 通过本票 stop 正常结束，exit 0。原 Windows backup（195120594 字节）已还原，两原任务 Enabled 状态恢复；daemon 经原任务启动为 PID 20104，既有 `http://127.0.0.1:18460/api/status` 返回健康、hostState=running，节点仍为原 `e48707bfc03d355c9eb5e4cf5a0904b5cbce3c90075ccc52c744f557af167353`。未部署 140 版本、未恢复全局 Goal 或推进其他票。
