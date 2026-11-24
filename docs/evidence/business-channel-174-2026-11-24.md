# MIOPUNCH-174 原终端附件跨承载继续交互

后续以包含原流换路修复的版本复测了全部 195 段，本文两段也均在原观察窗口内收齐。`20210805-006` 再次在新 QUIC 上保留原附件与原 shell。[全量复测报告](hsrnet-terminal-replay-2026-11-23.md#2026-11-24-全量复测)

原生与浏览器终端附件共用可靠业务通道。附件 ID、原 tmux control client、工作区及 shell 状态在底层 QUIC 结束后保留，终端输入与输出通过原通道按序补齐。原生节点的 TCP 与终端共用节点发送／接收额度和既有联系退避。浏览器沿其已有 ConnectionCoordinator 和独立服务连接角色接入相同字节进度协议。[MIOPUNCH-174](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/509ae474-a428-429b-ae1b-efd30bc634e4) · [ADR 0021](../adr/0021-business-channel-continuity.md)

## 接线与结束状态

BusinessChannel 的载体生命周期契约供 Native/WASM 共用。有界本地记录流承载现有 TerminalInput/TerminalOutput，可靠进度、去重、重放、额度及恢复截止继续由同一个通道处理。终端首开身份绑定原 TLS exporter 与流号，替代开流每次递增代际，Ready 与进度握手共用原恢复截止。满 16 业务时，为识别恢复请求保留有界头部容量，新业务仍受 16 项上限约束。

通道内部记录 IO 持续等待载体事件、调用方取消或恢复期限，健康静默超过 60 秒也可继续读写。普通独立记录操作保持原时限。到期时，双方上层终端 IO 读尽已确认缓冲后才报告失败，公开读取接口仍交付已经排入应用队列的输出。显式 detach 在原期限内排空本附件输出，关闭／Drop、撤权及服务停止保留各自收尾行为。

高铁回放曾在新成员载体准入后立即报告 connection lost。记录确认其真实原因是 ApplicationClosed code 0、reason 为已有的 member connection superseded。MemberConnection 公开严格匹配该已有仲裁原因的判定，业务通道随后继续通过节点所有者取得已选载体。其它显式应用关闭继续结束业务。

## 真实入口

| 验收 | 实际结果 | 记录 |
| --- | --- | --- |
| 原附件与原 shell | 46.53 秒通过。双方旧 QUIC 均 TimedOut，原附件恢复后提交新命令，独立 shell PID、变量与两行实际执行标记保持。 | `/tmp/miopunch-174-boundaries/terminal_keeps_original_attachment_and_shell_after_member_replacement.log` |
| 首开及替代 Ready 丢失 | 81.42 秒通过。原 Ready 暂停、第一轮替代 Ready 再次因真实断网丢失，在同一恢复窗口接回唯一原 tmux control client 并正常分离。 | `/tmp/miopunch-174-final/terminal_lost_initial_ready_recovers_the_original_control_process.log` |
| 16 附件恢复 | 56.62 秒通过。16 个原控制客户端均继续新命令，原附件共用一个新成员 QUIC，读尽各自输出后正常 detach。 | `/tmp/miopunch-174-final/terminal_full_attachment_capacity_recovers_on_one_member_connection.log` |
| 到期输出排空 | 48.39 秒通过。恢复到期后仍先读到已确认、尚未被应用读取的原终端输出，再得到失败结果。 | `/tmp/miopunch-174-boundaries/terminal_expiry_delivers_queued_confirmed_output_before_failure.log` |
| 到期输入排空 | 51.53 秒通过。暂停已接管命令到 tmux 的写入，恢复到期后解除暂停，原命令仍写出独立文件，随后原控制客户端结束。 | `/tmp/miopunch-174-final/terminal_provider_expiry_delivers_confirmed_input_before_ending_control_client.log` |
| 健康静默 | 静默 62 秒后原通道新 IO 正确，QUIC 与流 ID 保持，整个场景 70.80 秒。 | `/tmp/miopunch-174-boundaries/healthy_retained_channel_survives_more_than_sixty_seconds_of_application_silence.log` |
| TCP 在途／半关闭／撤权 | 双向各 256 KiB 精确续接、丢 ACK 后精确尾部与 FIN、撤权后无目标重开均通过，分别 47.99、48.22、47.83 秒。 | `/tmp/miopunch-174-final/results.json` |
| Browser/WASM | 真实 Chromium 客户端在 relay TCP 黑洞后继续原附件、原 shell PID 与变量，正常分离、重附着、停止服务和结束工作区均通过。 | `/tmp/miopunch-174-browser/result.json` |
| Browser 显式关闭压力 | 最终包在 8 MiB 输出且应用未读取时分离，明确返回 failed=true、workspace_ended=false，独立输出完成文件存在，之后重附着及服务停止正常。 | `/tmp/miopunch-174-browser-close/result.json`、同目录 `events.jsonl` |

Browser 使用现有 headless 技术宿主及其默认 Chromium 启动参数，包含关闭 Chromium sandbox。整个用例在一次性回环网络命名空间中运行。Native 与 Browser 的共享通道格式通过最终 WASM 构建，回执 `/tmp/miopunch-174-final-wasm/receipt.json`。最终 Native CLI 构建回执 `/tmp/miopunch-174-final-cli/receipt.json`，测试构建回执 `/tmp/miopunch-174-close/receipt.json`。

现有 SDK 的工作区／附着上限用例通过。原生命周期用例在最终关闭修复后通过，覆盖取消初始 future、节点 close/Drop、原身份重开、后台工作保留和真实前台进程结束。压力用例保留原 16 MiB 输出和 1024 个输入调用，按当前满额等待契约先验证原通道活着、已确认字节可读，再验证显式关闭的实际结果与独立清理。最后输出分离 0.018 秒结算为失败且保留工作区，输入关闭在 5.003 秒结束，所有 1024 调用结算，后台工作完成。记录为 `/tmp/miopunch-174-final-lifecycle/suite.log`、`/tmp/miopunch-174-close-lifecycle/suite.log`、`/tmp/miopunch-174-pressure-lifecycle/suite.log`。

## 完整高铁回放

保留原完整源轨迹、NFQUEUE 下行模型、独立 UDP 探针、250 ms 固定命令时刻、独立远端执行文件及回显编号。每个场景仍记录原回放结束后 6 秒窗口，并允许在通道恢复期内继续观察。以下两案均已在原窗口收齐。

| 轨迹 | 计划／入队／远端执行／客户端回显 | 独立校准 | 通道与 shell |
| --- | --- | --- | --- |
| 20210805-006 | 256 / 256 / 256 / 256，66.40 秒完成 | 1199 个已知回包全部匹配，RTT 绝对误差中位数 0.3955 ms、P95 0.5548 ms | 通道 `1fdcb120ba059bae8698ed7147935c4e` 保持，QUIC `127283570724336` → `127283570561568`，原 shell PID 与变量保持，恢复后新命令通过。 |
| 20210530-007 | 111 / 111 / 111 / 111，29.31 秒完成 | 53 个已知回包全部匹配，460 个已知缺失保持缺失，RTT 绝对误差中位数 0.4266 ms、P95 3.9072 ms | 通道 `9e19767dba26933f416953eb75c19daa` 保持，同一 QUIC `106357821294096`，原 shell PID 与变量保持，输入各执行一次。 |

实际结果与完整索引在 `/tmp/miopunch-174-hsr/20210805-006-fixed/summary.json`、`/tmp/miopunch-174-hsr/20210530-007/summary.json`。每目录保留探针、排队输入、逐编号回显及两端 stderr。006 原始红灯及明确 superseded 诊断在该父目录的 `20210805-006/`、`20210805-006-diagnose/`。回放驱动为同目录 `run-profile.py`、`lab.py`、`trace-run.py`，原轨迹位于 `/tmp/miopunch-hsrnet-full.4xhcN9/timeline/`。[原全量基线](hsrnet-terminal-replay-2026-11-23.md)

独立 Standards/Spec 审查及同根因限定复核完成，四项原发现已修复。最终 provider 到期输入问题由上述独立 tmux 文件结果验证并经 Spec 只读复核关闭。报告留在 `/tmp/miopunch-174-review.md`。

## 仍需区分的结果

Native 服务整文件运行一次得到 16 通过、7 失败、29 环境忽略，69.80 秒，原始记录 `/tmp/miopunch-174-final/suite.log`。七个失败名与170的失败名单一致，逐项根因在本票未重新定位。最后关闭与压力修改只做受影响入口的定向验证，复用未受影响的通过证据。

SDK 生命周期前轮曾出现一次没有原始 stderr 的 private tmux session list failed。当前错误已附带真实 stderr，最终相同生命周期入口通过。首轮 16 附件场景已完成恢复与新命令，但关闭时未消费各自广播输出，原关闭期限触发；最终场景读尽各自输出后通过，显式 detach 也已加入相应排空。

MIOPUNCH-175 继续用同一最终集成产物复跑五个历史失败场景。共享 Cargo target 可被后继构建替换，按本提交及构建回执复现。前票记录的 immutable store 容量限制仍适用，本票未增加持久缓存或另建 store。
