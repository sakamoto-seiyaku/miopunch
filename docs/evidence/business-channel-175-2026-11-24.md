# MIOPUNCH-175 最终集成版高铁历史失败五案回放

2026-11-24，最终产品提交 `006ab7307ee27d8cef646dde914d6c3a08c7d329` 的五个完整历史失败用例全部通过。870 条计划命令均入队、在原 shell 各执行一次并按序回显。原轨迹结束后六秒的观察窗口内已收齐，恢复期限允许的后续观察结果与该窗口一致。每案均在原附件提交并核对新的命令和 shell 状态。[MIOPUNCH-175](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/a576855c-dc62-4dda-b25b-6a64343d6880)

## 逐案前后对照

四个计数依次为计划、客户端成功入队、独立远端执行、客户端回显。基线来自 `47042ee4` 的全量回放。最终窗口为轨迹原始时长加六秒，最多继续观察到轨迹结束加默认恢复窗口六十秒。五案均在原窗口收齐，因此无需延长等待。[基线报告](hsrnet-terminal-replay-2026-11-23.md)

| 用例 | 基线 | 最终原窗口 | 最终恢复观察 | 观察时长 / 秒 | 结果 |
| --- | --- | --- | --- | ---: | --- |
| `20210530-007` | 111/111/111/54 | 111/111/111/111 | 111/111/111/111 | 33.725 | 通过 |
| `20210805-005` | 115/84/80/63 | 115/115/115/115 | 115/115/115/115 | 34.511 | 通过 |
| `20210805-006` | 256/121/109/0 | 256/256/256/256 | 256/256/256/256 | 69.787 | 通过 |
| `20210911-004` | 252/252/251/244 | 252/252/252/252 | 252/252/252/252 | 68.803 | 通过 |
| `20210911-source-01` | 136/106/106/60 | 136/136/136/136 | 136/136/136/136 | 39.766 | 通过 |

每案的 `executed.txt` 保存独立远端文件，`echoes.txt` 保存实际回显顺序，均与完整计划序列逐条相等。未发现遗漏、重复或乱序。`terminal.jsonl` 保留每条命令的原目标时刻、实际提交时刻和回显时刻，原窗口快照由同一控制器读取独立远端文件。快照时刻超过目标窗口的最大差值为 10.764 ms。

## 网络校准与源范围

沿用原 NFQUEUE 下行模型与独立编号 UDP 探针。UDP direct 与 relay 共用模拟蜂窝链路，TCP direct 在夹具中关闭。每段完整 timeline 按原始偏移连续播放，原 250 ms 命令目标时刻保留。尾部结果未知的探针沿用此前最近一次观测 RTT，无此前观测时为零附加延迟，并排除在已知丢包和 RTT 评分之外。阈值为逐号匹配全部已知回包、已知缺失不得收到回包，以及 RTT 绝对误差中位数 ≤5 ms、P95 ≤15 ms。

| 用例 | 原时长 / 秒 | 探针 | 已知回包匹配 | 已知缺失 | 尾部未知 | RTT 误差中位数 / ms | P95 / ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `20210530-007` | 27.716108 | 545 | 53/53 | 460 | 32 | 0.411 | 0.562 |
| `20210805-005` | 28.500525 | 561 | 455/455 | 6 | 100 | 0.425 | 0.742 |
| `20210805-006` | 63.784996 | 1200 | 1199/1199 | 0 | 1 | 0.406 | 0.554 |
| `20210911-004` | 62.791483 | 1200 | 1146/1146 | 53 | 1 | 0.402 | 0.611 |
| `20210911-source-01` | 33.765034 | 664 | 662/662 | 0 | 2 | 0.409 | 0.569 |

合计 4,170 个探针，3,515 个已知回包全部匹配，519 个已知缺失无额外回包，136 个尾部未知结果单列。五案均一次校准通过。`20210911-source-01` 的 `source_tail_truncated=true` 保留，使用原截断抓包中全部可读数据；该可读范围为 33.765034 秒、664 个探针。每案 summary 保留原抓包归档内路径与首请求 epoch，未抽取或拼接故障片段。

## 原窗口、通道和 shell

同一附件句柄在回放前后保持。每案初始化 shell 变量，回放后通过原附件提交新命令，核对 shell PID 与变量值，并收到新命令回显。表内耗时从提交这条新命令开始计。各私有命名空间的 PID 独立，均实测为 45。

| 用例 | 业务通道 ID（前后相同） | QUIC 标识（前 → 后） | 观察到的承载 | 新命令 / ms |
| --- | --- | --- | --- | ---: |
| `20210530-007` | `f7ca17220c05d2d7422b53a1acddce99` | `106224349624640` → `106224349624640` | 同 QUIC，选中路径 1 → 2 | 4.543 |
| `20210805-005` | `d598e890eed06ab98bacf0f00df3cd17` | `132891254412176` → `132891254412176` | 同 QUIC 保持 | 6.836 |
| `20210805-006` | `348447e58d7045c07cf50bc35c5ab225` | `133790311732880` → `133790312072800` | 新 QUIC 接回 | 4.688 |
| `20210911-004` | `36d666b788889827904884e526c72d75` | `106688093945968` → `106688093945968` | 同 QUIC 保持 | 20.796 |
| `20210911-source-01` | `9ca327b1515b86f19e099d9004852c68` | `134495357380688` → `134495357380688` | 同 QUIC 保持 | 17.016 |

006 的 QUIC 与流标识发生变化，原业务通道、附件和 shell 保持。007 的选中路径由 PathId 1 变为 2。其余同 QUIC 样本的完整路径 ACK 与 STREAM 帧计数保存在 summary 的前后状态中。表格仅描述实际观测到的身份变化。

## 同一构建与证据

最终批次通过共享 Cargo 缓存一次构建 Native CLI 和 relay，使用 `--locked --offline`、现有 profile。回执为 `/tmp/miopunch-175-build/receipt.json`，465 个 artifact Fresh、3 个重编，退出码 0。五案之间没有构建，所有 summary 的两个产物 inode、大小和修改时间逐项一致，控制器也在每案开始及结束核对。

| 产物 | inode | 字节数 | mtime_ns |
| --- | ---: | ---: | ---: |
| `target/debug/miopunch-client` | 2757102 | 200488888 | 1795453397442849252 |
| `target/debug/browser-probe-native` | 2757086 | 122259944 | 1795453398472449274 |

产物固定存在一项已确认限制。`scripts/pin_artifact.py` 在共享构建锁内固定本批 CLI 时被现有 32 GiB immutable store 门槛拒绝，未生成 pin。该批运行使用 Cargo 输出，依赖五案共同的前后产物身份、固定源码提交和构建回执。当前结果可复核。历史二进制归档仍受该容量限制。Standards 审查保留这一项构建规范偏离。

| 用例 | 汇总与完整原始证据目录 |
| --- | --- |
| `20210530-007` | [/tmp/miopunch-175-20210530-007/summary.json](/tmp/miopunch-175-20210530-007/summary.json) |
| `20210805-005` | [/tmp/miopunch-175-20210805-005/summary.json](/tmp/miopunch-175-20210805-005/summary.json) |
| `20210805-006` | [/tmp/miopunch-175-20210805-006/summary.json](/tmp/miopunch-175-20210805-006/summary.json) |
| `20210911-004` | [/tmp/miopunch-175-20210911-004/summary.json](/tmp/miopunch-175-20210911-004/summary.json) |
| `20210911-source-01` | [/tmp/miopunch-175-20210911-source-01/summary.json](/tmp/miopunch-175-20210911-source-01/summary.json) |

每个目录包含 `summary.json`、`terminal.jsonl`、`executed.txt`、`echoes.txt`、`clip-probe.jsonl`、独立 echo server 记录及双方 CLI 的 stdout/stderr。后四案的进程退出码和耗时在 `/tmp/miopunch-175-run-results.json`，五案进程均退出 0。

## 复现与本票检查

控制器为 [scripts/check_hsrnet_continuity.py](../../scripts/check_hsrnet_continuity.py)。既有网络夹具在 `/tmp/miopunch-174-hsr`，完整 timeline 在 `/tmp/miopunch-hsrnet-full.4xhcN9/timeline`，原公开归档在 `/tmp/miopunch-hsrnet.2VLrXS`。在仓库当前检出提交重编所需二进制后，将版本参数替换为该检出的完整提交 ID，每案使用新的输出目录。

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio175-build -- \
  build --locked --offline -p miopunch-native-tools \
  --bin miopunch-client --bin browser-probe-native --jobs 2
rtk proxy timeout 220s unshare -Urn --fork --pid --mount-proc --kill-child \
  python3 scripts/check_hsrnet_continuity.py \
  --fixture /tmp/miopunch-174-hsr \
  --timeline /tmp/miopunch-hsrnet-full.4xhcN9/timeline/20210805-006.json \
  --output /tmp/mio175-new-006 --version "$(rtk proxy git rev-parse HEAD)" \
  --build-receipt /tmp/mio175-build/receipt.json
```

运行器将终端失败与夹具校准失败写入各案 summary，保留最后进展并尝试原附件的新命令，记录明确错误与耗时。专用失败路径回归确认独立网络模型异常仍产生分类结果、终端错误后仍尝试新命令。Python 编译检查、`git diff --check` 和一次五案完整回放均通过。一次独立 Standards/Spec 审查及两项报告错误路径的限定复核完成。产品源码未在本票改动，174 的 Native/Browser、TCP、授权与压力有效结果继续引用 [174 验收报告](business-channel-174-2026-11-24.md)。该报告列出的七项普通整文件路径类失败仍未逐项归因。
