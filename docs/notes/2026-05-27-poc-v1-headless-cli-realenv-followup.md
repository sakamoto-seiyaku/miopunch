# 2026-05-27 POC v1 Headless CLI Real-Env Follow-Up

状态：本批次完成。目标不是做最终 release 验收，而是把 `poc-v1-06x-headless-cli-runtime` 在真实 Windows + WSL2 mirrored 环境下的 punch 污染问题用证据定位并修正。

## 背景

上一轮真实环境 smoke 已经证明：

- `init-network` / `invite` / `approve` / `join` / `ls` 可以走通。
- UDP punch 失败时，双方 candidate 集合被共享网络、link-local 和容器桥接地址污染。
- 代表性坏样本包括：
  - Linux：`10.255.255.254`、`169.254.83.107`、`172.17.0.1`、`172.18.0.1`、`192.168.4.5`
  - Windows：`169.254.101.51`、`169.254.228.208`、`169.254.247.191`、`169.254.83.107`、`192.168.144.1`、`192.168.4.5`
- 甚至出现了 `169.254.83.107 <-> 169.254.83.107` 这种明显错误的“自镜像” candidate 被选中。

因此本批次目标是：

1. 不再猜测。
2. 给 candidate gather 增加接口级别日志。
3. 只过滤已经被证据证明是污染源的地址。
4. 保留在 mirrored 网络里已经验证过可用的 `192.168.4.5`。

## 实现

本批次在 `internal/pocv1/runtime` 增加了 candidate gather 诊断和过滤：

- 记录每个 candidate 的接口名、接口 flags、接受/拒绝原因。
- 过滤：
  - loopback interface / loopback IP
  - link-local IPv4
  - `docker*`
  - `br-*`
  - `veth*`
  - `virbr*`
  - `cni*`
  - `vEthernet (Default Switch)` 这类默认虚拟交换机接口
- 保留 mirrored 环境里真实可达的 `192.168.4.5`。

focused 验证：

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./internal/pocv1/runtime ./internal/pocv1/punch ./cmd/miopunch -count=1
```

结果：`97 passed in 3 packages`

## 接口证据

Linux 当轮接口：

- `lo`: `127.0.0.1/8`, `10.255.255.254/32`
- `eth1`: `169.254.83.107/16`
- `eth2`: `192.168.4.5/24`
- `docker0`: `172.17.0.1/16`
- `br-cecf21e17fe9`: `172.18.0.1/16`

Windows 当轮接口：

- `Loopback Pseudo-Interface 1`: `127.0.0.1/8`
- `Tailscale`: `169.254.83.107/16`
- `vEthernet (Default Switch)`: `192.168.144.1/20`
- `Wi-Fi 2`: `169.254.101.51/16`
- `區域連線* 11`: `169.254.228.208/16`
- `區域連線* 12`: `169.254.247.191/16`
- `乙太網路 2`: `192.168.4.5/24`

这组证据解释了上一轮为什么会出现共享/镜像 candidate 污染。

## Fresh Real-Env Run

CLI-only fresh session 重新编译并重启后，使用：

- broker: `tcp://broker.emqx.io:1883`
- Linux session root:
  - `dist/extracted/miopunch_current_linux_amd64_session`
- Windows session root:
  - `C:\Users\stati\AppData\Local\Temp\miopunch_current_windows_amd64_session`

fresh 运行结果：

- `network_id=FI3OTE72BIC66QJOI4KXFV7LGU`
- Linux admin peer:
  - `DVWVHKVL5OATAPJAYUKLWGJ3EI`
- Windows member peer:
  - `JZOQOFKB4XMVFP7Q5B7O5UJIQ4`

命令闭环：

1. Linux `init-network --new --confirm create-new-network`: 成功
2. Linux `invite --mode approve`: 成功
3. Linux `approve <invite_code>`: 成功
4. Windows `join <invite_code>`: 成功
5. Linux `ls`: 成功，看到 Windows `online`
6. Linux `ping <windows_peer> -u`: 成功
7. Windows `ping <linux_peer> -u`: 成功
8. Linux `sh ls <windows_peer> -u`: 成功
9. Windows `sh ls <linux_peer> -u`: 成功

## 新日志事实

Linux candidate gather 日志：

- `candidate_count=1`
- `candidates=host@192.168.4.5:55998`
- `eth1 169.254.83.107` 被拒绝：`reason=link_local_ip`
- `docker0 172.17.0.1` 被拒绝：`reason=virtual_iface`
- `br-cecf21e17fe9 172.18.0.1` 被拒绝：`reason=virtual_iface`
- `lo 10.255.255.254` 被拒绝：`reason=iface_loopback`

Windows candidate gather 日志：

- `candidate_count=1`
- `candidates=host@192.168.4.5:65326`
- `Tailscale 169.254.83.107` 被拒绝：`reason=link_local_ip`
- `vEthernet (Default Switch) 192.168.144.1` 被拒绝：`reason=virtual_iface`
- `Wi-Fi 2 169.254.101.51` 被拒绝：`reason=iface_down`
- `區域連線* 11/12 169.254.*` 被拒绝：`reason=iface_down`

punch 交换和选择日志：

- Linux offer: `host@192.168.4.5:55998`
- Windows answer: `host@192.168.4.5:65326`
- planned pairs: `1`
- selected pair:
  - Linux local `192.168.4.5:55998`
  - Windows local `192.168.4.5:65326`

这说明本批次修复后的 punch 已经不再被污染 candidate 拖偏。

## 结论

当前已经在真实 Windows + WSL2 mirrored 环境证明：

- headless CLI runtime 的 fresh `join -> approve -> ls -> ping -u -> sh ls -u` 闭环可用。
- 这次失败根因主要不是 attempt budget，而是 candidate gather/export 被共享网络和虚拟接口污染。
- 基于接口级日志做最小过滤后，UDP punch 已恢复为稳定单 pair 选择。

## 剩余问题

- Windows `ls` 仍可能把 Linux 显示成 `online_state=offline`，但同一轮 `ping -u` 和 `sh ls -u` 已经成功。
  - 这更像 presence/discover 投影视图刷新滞后，不是 join/punch/dataplane 失败。
- 交互式 `sh` attach 本轮没有做脚本化 smoke。
  - 当前只证明了 shell discovery 和 shell attach 前置 gate。
  - 交互 attach 仍建议单独做一轮人工真实环境 smoke。

## 证据文件

- Linux daemon log:
  - `dist/extracted/miopunch_current_linux_amd64_session/logs/miopunch.log`
- Windows daemon log:
  - `C:\Users\stati\AppData\Local\Temp\miopunch_current_windows_amd64_session\logs\miopunch.log`
- Linux runtime state:
  - `dist/extracted/miopunch_current_linux_amd64_session/data/runtime_v1.json`
- Windows runtime state:
  - `C:\Users\stati\AppData\Local\Temp\miopunch_current_windows_amd64_session\data\runtime_v1.json`
