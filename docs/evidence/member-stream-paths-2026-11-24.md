# 原生成员共享 QUIC 按流分配验收

2026-11-24，在一次性 Linux 网络命名空间中，用原生 CLI 建立两个已准入成员。终端先在共享成员 QUIC 上收发，再在同一连接上启动并发 TCP 转发。双端分别记录实际编码、解码的 STREAM 帧，按连接、流、发送方向和直连绑定匹配。转发在每个方向交付 2,293,760 字节，发端逐块比较回送字节，服务端核对总量。终端逐条核对 26 次回显，并独立读取远端 shell 写入的有序记录。

| 开放直连 | 实测业务路径（双向） | 本机运行记录 |
| --- | --- | --- |
| A（10.92.0.0/30） | 两条流均走 A | `/tmp/mio177-final-veth-direct/` |
| B（10.93.0.0/30） | 两条流均走 B | `/tmp/mio177-final-veth-relay/` |
| A、B，单向各 4 ms | 已有终端流 12 留在 A（路径 1），新转发流 20 走 B（路径 2） | `/tmp/mio177-final-both/` |
| A 单向 4 ms、B 单向 45 ms | B 已获 ACK，终端流 12 与新转发流 20 均走 A（路径 1） | `/tmp/mio177-final-slow/` |

两路开放的场景均先核对已有终端在 A 收发，再开放 B 并在双端验证近期 ACK 和实测 QUIC RTT。同档时双端 A/B 分别为 9.095/8.796 ms、10.243/8.781 ms。慢 B 时分别为 10.668/75.090 ms、10.570/54.645 ms。

每个目录中的 `stream-paths.json` 记录双端逐流匹配，`result.json` 记录业务校验、引导时终端身份及传输统计。四种条件均从 relay 引导至业务完成保持同一终端、流和成员连接，直连就绪后业务数据走 UDP 直连。启用 `nat-lab-test-hooks` 的 CLI 记录实际 STREAM 的路径、偏移和长度。

在双路夹具中追加一次回归复测。TCP 转发在 B 上交付后只切断 B 的 UDP，原流在 A 上完成双向交付，终端继续在 A 上交付 26 次交互。双端成员连接和流标识保持一致。临时驱动为 `/tmp/mio177-cut-backup.py`，结果为 `/tmp/mio177-final-cut-backup/`。

单条直连时沿用既有连接级换路。原生 TCP 修复测试 `restricted_contact_repairs_the_live_native_forwarding_stream` 通过，切断初始 TCP 路径后，两端实际在替代路径上收发原流并核对应用字节。日志为 `/tmp/mio-177-final-tcp-repair.log`。

在一次性 user/network namespace 用相同产物分别运行 `--case veth-direct`、`--case veth-relay`、`--case both` 和 `--case slow`。 [验收脚本](../../scripts/check_member_stream_paths.py) · [MIOPUNCH-177](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/62278649-a24e-4b69-a3b7-a6f674413199)

本轮 CLI 编译的 Rust 源码和 `Cargo.lock` 对应提交 `529166d7`，构建回执为 `/tmp/mio-177-build-single-route/receipt.json`。两项产物固定在 `.artifacts/runs/mio-177/pinned/`，分别链接到以下不可变对象。

- `miopunch-client` 从当轮共享缓存的 `target/debug/miopunch-client` 固定。对象为 `.artifacts/sha256/3b/3ba0da59090c7bb8760df61316d099c98b973b612ff86c9a3630f2506bfbcbc8-555`。
- `browser-probe-native` 复用开始验收时从 `target/debug/browser-probe-native` 固定的版本。保留固定对象 `.artifacts/sha256/8d/8d5204f6f47363fc1a197a1adf095c2839dce1e2dfecae1a4eb5c9d5506d55d9-555`，原构建回执未保留。
