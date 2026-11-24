# 原生成员共享 QUIC 按流分配验收

2026-11-24，在一次性 Linux 网络命名空间中，用原生 CLI 建立两个已准入成员。终端先在共享成员 QUIC 上收发，再在同一连接上启动并发 TCP 转发。双端分别记录实际编码、解码的 STREAM 帧，按连接、流、发送方向和直连绑定匹配。转发在每个方向交付 2,293,760 字节，发端逐块比较回送字节，服务端核对总量。终端逐条核对 26 次回显，并独立读取远端 shell 写入的有序记录。

| 开放直连 | 实测业务路径（双向） | 本机运行记录 |
| --- | --- | --- |
| A（10.92.0.0/30） | 两条流均走 A | `/tmp/mio177-relay-bootstrap-veth-direct/` |
| B（10.93.0.0/30） | 两条流均走 B | `/tmp/mio177-relay-bootstrap-veth-relay/` |
| A、B，单向各 4 ms | 已有终端流 12 留在 B（路径 1），新转发流 16 走 A（路径 2） | `/tmp/mio177-relay-bootstrap-both/` |
| A 单向 4 ms、B 单向 45 ms | B 已获 ACK，终端流 8 与新转发流 12 均走 A（路径 1） | `/tmp/mio177-relay-bootstrap-slow/` |

每个目录中的 `stream-paths.json` 记录双端逐流匹配，`result.json` 记录业务校验及传输统计。四种条件均沿用引导时建立的成员连接，建路后真实业务数据走 UDP 直连。抓取仅在启用 `nat-lab-test-hooks` 的 CLI 中运行，记录 STREAM 的路径、偏移和长度。

使用 [验收脚本](../../scripts/check_member_stream_paths.py) 和已固定产物，在一次性 user/network namespace 中分别运行 `--case veth-direct`、`--case veth-relay`、`--case both` 和 `--case slow`。CLI 构建回执为 `/tmp/mio-177-build-qualified/receipt.json`。 [MIOPUNCH-177](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/62278649-a24e-4b69-a3b7-a6f674413199)
