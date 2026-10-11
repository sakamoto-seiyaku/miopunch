# M87：慢速 IRC 的预算与监听寿命

任务：[MIOPUNCH-87](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/baf08657-c7c4-4ebd-9e8c-3d91ddad33e4)。
实施记录：[显式预算与节点监听寿命](../../notes/2026-10-11-irc-budget-and-node-listener-lifetime.md)。

最终无产品调试输出构件已经完成真实公网严格准入、实际批准、材料保存、
双方身份重开、获准 relay 的 member/1 PING/PONG，以及自然资源清理。
首轮 Native SDK close 120 s 超时仍保留；随后只调整测试驱动关闭顺序，
让 IRC 在 SDK 关闭通知排空前继续存活，实测 Native close 自然返回。

| 场景 | 实际 caller 时间 | 结果 |
| --- | ---: | --- |
| owned，60 s，queue 1.5 s | 52.04 s | 请求失败，Native TLS 未完成 |
| owned，180 s，queue 1.5 s | 52.02 s | 仅提高预算仍失败 |
| owned，180 s，queue 30 s，bounded listener | 180.01 s | TLS/共同协议推进，入口按节点年龄退休，未保存请求 |
| owned，300 s，queue 30 s，maintained listener | 217.90 s | 严格 purpose2、AwaitingApproval、实际 Native 未批准请求 |
| public Ergo，同一显式慢速配置 | 250.46 s | 严格 purpose2 与未批准请求通过；500 ms WSS close 观察未通过 |
| public Ergo，批准/材料/重开/relay 业务 | 249.11 s 首次请求 | 后续 Complete、双方持久身份重开、member/1 PING/PONG 通过 |
| 最终构件，同一服务与配置，正式 SDK 关闭顺序 | 228.69 s 首次请求 | 全流程及自然清理通过；总 runtime 405.41 s |

前三个失败没有被删除。改变 queue TTL 后 Native TLS 才继续推进；仅扩大
caller 不足以过关。正式节点 listener 与实验 attempt 的寿命分开后，完整
严格协议与实际未批准请求通过。默认总预算仍为 60 s；本次成功使用两端
匹配的显式 300 s caller、600 ms pacing、8 s assembly TTL、30 s queue TTL。
上限及默认值分别记录，未改 QUIC/RTC 算法、重传参数或普通数据权限。

公网故事使用真正 `https://testnet.ergo.chat` Origin、正常证书验证、Native
TLS 6697 与 WSS `text.ircv3.net`。初始 Browser 没有 data relay/RTC，只能经
IRC 完成受限准入。经用户对具体测试的明确许可，实际批准全新测试网络的
请求，再通过获准 `aps1-1.relay.n0.iroh.link` 保存测试材料；重开两端后普通
member/1 STREAM 返回 PONG。Native 观察到授权成立、selected relay 及普通
STREAM 进展；业务不是经 IRC 承载。

最终 Browser 构件的六个完整 SDK 回归均通过，并要求自然清理通过：非法
超大预算在 timer/I/O 前拒绝；显式 2 s caller 约 2008 ms 结束；profile 寿命
1 s 的 maintained listener 闲置 3100 ms 后仍接受独立请求（约 1043 ms）；
取消约 259 ms 生效；禁用 IRC 为零浏览器 WSS；MQTT 约 186 ms 完成严格准入。
31 项已有 Rust 回归通过：IRC 10、admission 4、共同 connection 11、接受上下文
3、受保护路径协议 3。最终 Browser 改动未影响 Native 所选编译输入，因此
Native 构件及这些测试不重复编译或运行。

SDK close/free 返回、启动 WSS close handshake 与收到实际 close event 分别
记录。最终 relay WSS 在 500 ms 观察时尚未关闭，追加观察 2006 ms 后全部
收到 close event，随后才销毁 context/browser。Native 第一阶段 SDK close
自然返回耗时 119383 ms，未把进程退出码 0 或超时后 drop 称为 graceful；
最后成员阶段 Native close 也自然返回。关闭过程可能很慢，caller deadline
与清理观察时钟分别记录。当前 120 s Native 观察窗余量很小，不能推导一个
更短的通用关闭上限。

`iroh::Socket::close` 先通知 QUIC close、等待 draining、再退休传输并等待
runtime driver 收尾；测试驱动原先先关闭 IRC，扰动了这一顺序。成功 case
保留正确顺序及相同 120 s 观察窗，未修改产品/QUIC 算法，也未仅凭顺序变化
认定全部关闭延迟均由它造成。原始失败保留在独立 JSON。

文件：

- `diagnosis-and-prior-runs.json`：保留失败、阶段时钟、历史成功及未通过的清理观察。
- `final-affected-regressions.json`：六项最终 SDK 实跑与各自清理。
- `rust-regressions.json`：31 项测试、实际 exit 与日志 hash。
- `sdk-builds.json`：共享缓存、实际 Fresh/rebuilt、selected `.d`/source counts 和构件 hash。
- `owned-source-scope.json`：固定基点、自有五份产品源码及保护既有 WIP 的 manifest。
- `final-closed-carrier-failure.json`：最终 Browser 构件的已保留 Native 清理失败，不能被进程退出码 0 覆盖。
- `final-public-story.json`：最终实际严格 gate、持久重开、同 M relay fresh ACK/PING/PONG、SDK 自然关闭和晚期 WSS 事件。
- `review-summary.json` / 两份 review：规范和规格独立审查均为零发现，产品 source pins 自审查后未变。
- `probe-sources/`：实测 controllers/operator/精确 selector 与 lock，以及全故事标量核验器；不含任何生成的测试 store 或 secret。

这些 JSON 仅发布 allowlist 标量与 hash；原始事件、临时身份、grant、secret、
SDP、ciphertext 及 store 均留在所列私有目录。构件、raw receipt、selected source
copies 与 `.d` 原件保留；每次 runtime 在 actors 前及结束后检查源码、工具、
精确 lock graph、controllers、实际 selected inputs 和 immutable artifacts。
Native 历史 whole-source digest 没有冒称最终 Browser digest，复用依据是其
实际 selected graph 完全未变。

该资格属于当前 SDK、上述服务与显式配置；不提升为任意 IRC 服务或 M84 物理
异网全矩阵资格。
