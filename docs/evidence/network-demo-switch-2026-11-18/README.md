# r_proxy / u22sz 切网实测

2026-11-18，测试版本为 `9d3db16c`。`u22sz` 从原网络切到手机热点，再切回原网络。

- [r_proxy](r_proxy/) 和 [u22sz](u22sz/) 的 `stdout.log` 与 `stderr.log` 是复盘采集时从进程启动到采集点的完整日志。
- 两端各有 `status.json`（原始诊断快照）、`switch-events.txt`（切网相关事件）和 `timeline.txt`（日志阶段摘录）。
- [paired-status.jsonl](paired-status.jsonl) 是复盘时两端的选路和业务计数快照。

切网事件中的墙钟为 UTC+08:00。`monotonicMs` 在各自进程内计时，两端用共同的 attempt ID 或打洞 round ID 对齐。
