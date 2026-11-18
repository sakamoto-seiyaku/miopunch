# MIOPUNCH-138：双端 relay 选区与起拨时机对照

规格：[MIOPUNCH-138](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ede9f6d6-84b9-4163-9c07-1dfc6d22105f)，父规格 MIOPUNCH-135。讨论基线 `9d3db16c`。

本票交付实验入口、实测结果及现场建议；没有改变 SDK 默认选路、home 或生产拨号并行策略。2026-11-18 公网最终八组有效完成：七组交付连接并收到十四份身份、序号匹配的回复；一组连接期限耗尽，未进入业务请求。复用受控双端十八个场景的通过结果。此前 DNS/路径权限前置失败不进入最终对照结论。

## 实验边界与复现

- 公网使用不切网的家宽 Linux `r_proxy` 与 Windows `u22sz`，独立临时网络、独立节点身份，事先经 Native SDK 手工入网；两方向串行各四组。每组启动后有界运行，客户端及服务端均结束才开下一组。
- 测试 actor 用 `NativeNetwork::open`、节点资源配置、受限端点及现有 `ConnectionCoordinator` 的成员业务入口；正常身份、成员权限、资源与双向路径验证仍执行。`relay_comparison_fixture` 仅在 `nat-lab-test-hooks` 下提供实验 owner，不安装后台调度器。
- 四组依次为 normal-auto、relay-auto、relay-fixed、relay-fixed-early。relay-only 关闭 UDP/TCP 直连、成员数据 relay 和观察；normal 允许 UDP，TCP 仍关闭。四个获准公共 relay 始终相同：aps1/euc1/use1/usw1；目标候选也保持相同，normal 额外包含端点实际 IP。
- auto 使用四个 relay 的自动 home；fixed 两端只把 APS 配为 home，但拨号候选、资源许可及期限仍为原集合。fixed-early 在同样 home、候选与期限下，把候选按 250ms 间隔启动，首个通过公共协调器验证者交付，取消并收齐其他尝试。
- 公网每次等待 home 注册最多 10s，连接期限 30s；成功交付后在同一连接上分别发 seq1（3s）与 seq2（10s），校验回复身份和序号。两笔请求分别计时，第二笔不是对第一笔延长期限。服务端寿命上限 90s，外层每组 150s。
- Windows actor 始终位于用户已允许的原 demo.exe 路径；节点状态仍用本票临时目录。Windows 服务端由同步 SSH `WaitForExit` 持有。旧 demo 已停止，原任务保持禁用，原 backup、node/config 保留；结束不还原旧 demo。没有调整 Simplewall、DNS 或网络连接。

最终 Linux/Windows 测试目标均以 `--locked --offline`、相同 feature 编译，各自 3 targets rebuilt（其余 466/527 Fresh）。最终部署程序两端 `--list` 正常，并由实际八组执行消费；没有额外 checksum/manifest。见 [Native 构建](final-native-build.json)、[Windows 构建](final-windows-build.json)。

在已入网并已部署 actor 的同一组临时节点上逐组执行：

```sh
rtk proxy timeout 150s python3 scripts/check_relay_route_comparison.py \
  --output /tmp/mio138-public-final --case windows-to-linux-relay-fixed-early
```

输出目录必须是新目录；`--case` 可选两个方向与上述四条件。runner 使用既有 SSH 别名、容器和 Windows 允许路径，上传的是单组 JSON 配置及目标地址，原始节点数据库不跨设备复制。

## 公网最终结果

连接列是 actor 的 `handshakeMs`：从拨号开始到公共协调器完成协商、fresh ACK、Ready/Commit 后向应用交付，包含 QUIC 建连时间，并非仅 TLS 握手。原始 `connection-timeline` 保留每条 route-dial、control-establishment 的本地单调时间，可区分传输建连与应用放行；双方应用就绪时间分别来自各端，未用跨主机 wall clock 相减。

| 方向 | 条件 | 连接交付/超时 ms | 3s 请求 ms | 10s 请求 ms | 拨号至首份有效回复 ms | 实际 route 尝试 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Windows→Linux | normal-auto | 11368 | 1016 成功 | 981 成功 | 12386 | 1 |
| Windows→Linux | relay-auto | 9481 | 734 成功 | 1043 成功 | 10216 | 1 |
| Windows→Linux | relay-fixed | 29915 | 2063 成功 | 3597 成功 | 31980 | 1 |
| Windows→Linux | relay-fixed-early | 14815 | 2090 成功 | 1134 成功 | 16906 | 4 |
| Linux→Windows | normal-auto | 30002 超时 | 未发起 | 未发起 | 无 | 1 |
| Linux→Windows | relay-auto | 12933 | 1256 成功 | 1001 成功 | 14190 | 1 |
| Linux→Windows | relay-fixed | 12547 | 1073 成功 | 1087 成功 | 13622 | 1 |
| Linux→Windows | relay-fixed-early | 10548 | 779 成功 | 1251 成功 | 11328 | 4 |

所有成功连接最终选中 APS relay；每组两份回复保持同一 `stableId`，实际由对端原身份回复。normal 允许 UDP 不等于本次建立了 UDP 路径。

除反向 normal-auto 外，双方注册均报告 APS connected。该反例中 Windows 服务端先尝试 APS，约 10s 后报告连接超时并切到 EU，在启动后 10.726s 显示 EU connected；Linux 客户端为 APS connected。客户端第一条 APS route-dial 占满 30s，没有其他实际候选获得起拨机会。日志支持“home 与首个候选不同且未获备用机会”的归因，不证明同一时段其他 relay 或 UDP 一定可用，也不证明自动选区永远失败。

两个 early 组均启动四条 route（Windows 组约 1/256/507/755ms，Linux 组的实值见 JSON），相对对应串行 fixed 各多三条协调器尝试；APS 获胜，其余被取消并收齐。这是应用可观察到的 route 尝试成本，不冒充底层 TCP 重连数、发包数或字节成本。公网每条件每方向仅一次，时间差含公网波动；自动与固定恰好同为 APS 的成功组也不能证明固定 home 本身更快。

复现入口在独立双审中发现旧 `.target.stop`/`.target.json` 会被同组重跑复用，已改为启动前同步清理本组服务端文件。针对同一组名进行一次 focused 复测：Windows 新 target 于本次启动后发布，服务端运行 45.85s，stop 与 target 时间差 39.208s；两端结束后均无本票 actor。该复测的双方 APS home 均成功注册，但客户端中途短暂断开重连，连接最终在 30.004s 超时，未进入请求。这是新增有效负样本，单列于 [focused-rerun.json](focused-rerun.json)，不替换原八组，也不把它算作复现入口错误；提前起拨不保证公网成功。

机器结果 [public-results.json](public-results.json) 保留每端身份、home 状态/错误、实际候选、route 起止、传输到控制的阶段、双方应用就绪、首回复和请求结果。全部原始 stdout/stderr 在 [public-logs.tar.gz](public-logs.tar.gz)，没有私钥或节点数据库。

## 受控双端：断路、恢复和慢路径

[controlled-results.json](controlled-results.json) 与 [原始日志](controlled.log.gz) 来自真实 Iroh relay、真实入网 SDK 双端，在同一受控拓扑执行九条件、重复两轮，共十八场景；测试整体通过（149.33s）。控制器透明转发 TCP，不实现 relay 协议、不制造业务回复。连接总期限为 4s；业务仍分别为 3s/10s。每次新建端点，从注册开始记录，并在本组结束后收齐服务端与转发任务。

| 条件 | 两次连接结果 | 两次业务结果 |
| --- | --- | --- |
| healthy-normal / healthy-relay-only | 四次全部成功，0.666–0.768s | 3s、10s 均成功 |
| cut-auto / cut-fixed | 四次均约 4.001s 超时 | 未发起 |
| cut-fixed-early | 0.907s / 0.958s 成功 | 均成功 |
| restored-fixed | 0.740s / 0.719s 成功 | 均成功 |
| slow-fixed | 4.002s / 4.001s 超时 | 未发起 |
| slow-fixed-early | 2.302s / 2.444s 成功 | 均成功 |
| business-slow-fixed | 0.677s / 0.732s 成功后再加延迟 | 两次 3s 均超时；10s 一次超时、一次 9.637s 成功 |

断路条件丢弃第一 relay 的转发字节；fixed home 是健康的第二 relay，自动 home 使用相同两候选。slow 条件给第一 relay 每个转发 chunk 加 150ms；business-slow 在连接已放行后给两条转发各加 1800ms/chunk，隔离业务等待与握手问题。转发 chunk 延迟不等于固定 RTT。

切断的串行候选耗尽连接机会，而相同 home/许可/候选/预算下的提前起拨在两轮都取得另一条有效连接；恢复后原串行配置再次可用。自动 home 与固定 home 均不能单独克服被阻塞的第一拨号候选。较长请求期限有时能收到慢回复，但一次 10s 仍超时，不能宣称延长期限保证业务可用。

每场景 `before/after` 包含两个透明转发端的 TCP 连接数及转发字节，可直接计算增量；这些数包含客户端 home 注册连接，不能与公共协调器 route 尝试数等同。两轮 cut-auto 各 4 条新 TCP，cut-fixed 与 cut-fixed-early 各 2 条；转发字节及完整计数见结果 JSON。

## 本轮现场建议与限制

建议下一次双端集成验收采用一套明确的实验配置：两端固定获准 APS home、保留原四个 relay 候选及权限、候选每 250ms 提前启动，连接仍 30s；原 3s 请求保持独立验收，10s 只作额外的慢路径观测。理由是这套配置在初轮两个真实方向均收到有效回复，并在受控阻塞/慢首候选时保留其他候选机会；随后 focused 公网复测仍出现连接超时，因此它只是下一轮需检验的配置，不能保证恢复；代价是当前四候选最多增加三条并发协调器尝试。它是下次集成的配置建议，**不是已经安装到普通 SDK/demo 的默认行为**。

公网没有切热点或回办公室，也未定位办公网 UDP 回程或 WAN 源端口；自动 home 的单次失败与集成 demo 的既有反例仍是保留的证据。这里不能替代 MIOPUNCH-140 的同版集成与后续现场验收。未测重复样本分布、长期资源成本和其他公共 relay 的稳定可达性；不从一次成功推导全局默认。

运行结束独立读取：Linux 容器 `docker top` 无本票 relay-probe actor，仅保留既有 demo 宿主；Windows 原允许路径下 actor 数为 0，两项旧 demo 任务均禁用，原 backup 存在。

必要源码检查：测试文件 rustfmt、runner Python 语法、限定差异空白检查通过。最终审查仅针对本票 fixture、测试、runner 和上述证据，不包含其他票提交或 demo 工作区改动。

独立审查：Standards 与 Spec 首轮各报告同一项 P2 复现控制文件残留问题，已集中修复并以同组名复测核实本次发布与生命周期。Standards 另有非阻断的重复平台分支建议；现有脚本保留明确的 Linux/Windows 生命周期操作，未为此增加抽象。两名原审查者的 focused 复核均确认 P2 已解决，未发现新增阻断或规格问题；没有重新全审、重编或重跑矩阵。
