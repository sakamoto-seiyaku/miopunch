# WSL2 / Windows shell 真机测试执行记录

日期：2026-05-15

状态：进行中。本文记录本轮按 `docs/notes/2026-05-15-wsl2-windows-shell-real-test-plan.md` 执行真实环境 smoke 时发现的事实、证据、临时修复和结论。本文不是最终验收报告。

## 1. 执行范围

本轮先尝试第一批：

- G0 preflight 与环境清理。
- G1 fresh network / membership bootstrap。
- 视 G1 结果继续进入 G2/G3/G4 的最小闭环。

约束：

- 允许在受控 debug batch 中修改代码、重编译、重跑。
- 不提交 commit。
- 重要发现、命令结果和结论必须记录到本文。

## 2. 环境事实

- WSL 内核：`6.6.114.1-microsoft-standard-WSL2`。
- WSL 发行版环境可调用：
  - `powershell.exe`
  - `wsl.exe`
  - `cmd.exe`
- Windows 可见 WSL distros：
  - `Ubuntu`
  - `Debian`
- 初始可用 bundle：
  - Linux: `dist/extracted/miopunch_0.0.0-git78ff8d5_linux_amd64_session/miopunch`
  - Windows: `dist/extracted/miopunch_0.0.0-git78ff8d5_windows_amd64_session/miopunch.exe`

## 3. G0 preflight 记录

### 3.1 Linux tmux 缺失

事实：

```text
tmux -V
zsh:1: command not found: tmux
```

结论：

- Windows -> Linux `local` shell attach 会被 Linux 被控端 `tmux` 前置依赖阻塞。
- 后续如果要跑 G4-01/G4-02/G4-03，需要先安装或修复 Linux `tmux`。
- 这不是 punching、dataplane 或 LocalAPI 问题。

### 3.2 Windows 旧进程存在但 LocalAPI 不可用

事实：

- Windows 侧存在 `miopunch.exe` 进程，路径来自 repo 的 extracted Windows bundle。
- 但 Windows CLI 默认 `ls` 返回：

```json
{
  "kind": "ls",
  "stage": "cli",
  "reason_code": "DAEMON_NOT_RUNNING",
  "exit_code": 3
}
```

结论：

- 再次验证“进程存在”不等于 “LocalAPI 可用”。
- 该问题归类为 G0 daemon / LocalAPI gate 失败。
- 用户随后手动清理旧进程；清理后重新启动 Windows daemon，`ls` gate 通过。

### 3.3 隔离 daemon 启动与 ls gate

Linux daemon：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  up --state_path /tmp/miopunch-wsl2-windows-shell/run/linux/state.json

miopunch up: serving LocalAPI (user) at unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock
```

Windows daemon：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  up --state_path 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\run\windows\state.json'

miopunch up: serving LocalAPI (user) at npipe:\\.\pipe\miopunch\localapi-S-1-5-21-1396067383-3453746490-1104383483-1001
```

双方 `ls --format json` 结果：

```json
{"kind":"ls","status":"done","stage":"ControlPlaneReady","reason_code":"OK","facts":[{"message":"peer_count=0"}]}
```

结论：

- 新隔离 daemon 的 LocalAPI gate 通过。
- G0 的 daemon 可用性前置条件满足。

## 4. G1 bootstrap 记录

### 4.1 旧 extracted binary 缺少 init-network

事实：

```text
unknown command: init-network
reason_code=UNKNOWN_COMMAND
```

结论：

- `dist/extracted/...git78ff8d5...` 中的 Linux CLI 比当前源码能力旧。
- 后续 G1 不能继续使用旧 extracted binary 作为执行二进制。
- 已切换到从当前源码构建的新二进制：
  - `/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux`
  - `/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe`

### 4.2 invite 首次失败：broker IP 写入 invite code 后 join 端不可达

第一次 workaround：

- 通过 LocalAPI `PATCH /api/v0/desktop/config` 把 Linux runtime broker 固定为 `broker.emqx.io:1883`。

旧二进制 invite 成功事实：

```text
invite_brokers=35.172.255.228:1883
brokers_effective=broker.emqx.io:1883
reason_code=OK
```

随后 Windows join 失败：

```json
{
  "kind": "join",
  "stage": "PeerContact",
  "reason_code": "UNAVAILABLE",
  "facts": [
    {"message":"invite_brokers=35.172.255.228:1883"},
    {"message":"mqtt connect failed: 35.172.255.228:1883: future canceled"}
  ]
}
```

结论：

- `invite` runtime broker 探测能连接 `broker.emqx.io:1883`。
- 但 invite code 写入的是 canonicalized IP `35.172.255.228:1883`。
- Windows `join` 只按 invite code 里的 IP 连接，无法回退 hostname，因此失败。
- 这是 invite broker 选择 / canonicalization 问题，不是 shell、punching 或 governance 问题。

### 4.3 代码临时修复

修改文件：

- `internal/task/brokers.go`
- `internal/task/invite_join_test.go`

修复内容：

- `selectReachableInviteSubset` 不再把 hostname broker canonicalize 成 A 记录 IP 后写入 invite code。
- 仍保留 `host:port` 格式验证。
- 仍保留 MQTT reachability probe。
- 新增测试：reachable hostname broker 应保持 hostname 写入 invite code。

聚焦测试：

```text
/usr/local/go/bin/go test ./internal/task -run 'TestRunInviteTask(KeepsReachableHostnameBrokerInInviteCode|UsesReachableBuiltinBrokerWhenExplicitConfigAbsent|DoesNotMixBuiltinBrokerWhenExplicitConfigExists)|TestJoinApprovePersistEffectiveBrokerForPostJoinSignaling' -count=1 -v

PASS
ok github.com/miopunch/miopunch/internal/task 0.479s
```

构建：

```text
/usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux ./cmd/miopunch
GOOS=windows GOARCH=amd64 /usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe ./cmd/miopunch
```

结论：

- 该修复已通过聚焦 Go 测试。
- 该修复尚未提交。
- 后续真实环境测试使用新构建二进制。

### 4.4 新二进制 init-network 成功

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g1-01-linux-init-network-new.md \
  init-network
```

结果：

```json
{
  "kind": "init_network",
  "stage": "SelfDiscovery",
  "reason_code": "OK",
  "facts": [
    {"message":"peer_id=NSW2B4OYFY4NRRKFENKRWI6XKE"},
    {"message":"net_id=7KEWGPWRVEWKCA6WCLROHQGSGQ"},
    {"message":"governance_head_b64=U0b6boEbU-UumTmsMmtS2VGOMRnUU1hoRVamH6sd1vY"}
  ]
}
```

结论：

- Linux admin network 初始化成功。
- G1 可以继续 invite/join/approve。

### 4.5 新二进制 invite 成功并保留 hostname broker

前置：

- Linux runtime broker 固定为 `broker.emqx.io:1883`。

结果：

```text
invite_brokers=broker.emqx.io:1883
brokers_effective=broker.emqx.io:1883
peer_id=NSW2B4OYFY4NRRKFENKRWI6XKE
net_id=7KEWGPWRVEWKCA6WCLROHQGSGQ
reason_code=OK
```

结论：

- 修复后的 invite code 不再把 broker hostname 改写为 IP。
- 这是继续 Windows join 的必要前置。

## 5. 当前待执行

- 继续 G2 最小 connectivity gate。
- 如果 G2 通过，继续 G3 shell discovery。
- 如果 G2 失败，按 signaling、candidate、punching、dataplane、hello/governance、payload、session lifecycle 分层记录。

## 6. G1 join / approve 闭环结果

并行执行：

```text
Linux admin:
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g1-03-linux-approve-new.md \
  approve <invite_code>

Windows member:
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g1-04-windows-join-new.md' \
  join <invite_code>
```

Linux approve 结果：

```json
{
  "kind": "approve",
  "stage": "CapabilityHandshake",
  "reason_code": "OK",
  "facts": [
    {"message":"invite_brokers=broker.emqx.io:1883"},
    {"message":"peer_id=NSW2B4OYFY4NRRKFENKRWI6XKE"},
    {"message":"net_id=7KEWGPWRVEWKCA6WCLROHQGSGQ"},
    {"message":"member_peer_id=CSF7LBLQZ5WGRM7DMJZ6YJ3AYI"},
    {"message":"known_seed_peers=1"}
  ]
}
```

Windows join 结果：

```json
{
  "kind": "join",
  "stage": "PeerContact",
  "reason_code": "OK",
  "facts": [
    {"message":"invite_brokers=broker.emqx.io:1883"},
    {"message":"peer_id=CSF7LBLQZ5WGRM7DMJZ6YJ3AYI"},
    {"message":"net_id=7KEWGPWRVEWKCA6WCLROHQGSGQ"},
    {"message":"seed_peers=1"},
    {"message":"bootstrap_recommendations=1"}
  ]
}
```

结论：

- G1 membership bootstrap 已闭环成功。
- Linux admin peer: `NSW2B4OYFY4NRRKFENKRWI6XKE`。
- Windows member peer: `CSF7LBLQZ5WGRM7DMJZ6YJ3AYI`。
- 双方 net id 一致：`7KEWGPWRVEWKCA6WCLROHQGSGQ`。
- 修复后的 broker hostname 保留策略已被真实 Windows join 验证。
- 可以进入 G2 connectivity gate。

## 7. G2 connectivity gate 结果

### 7.1 双方 peer 落盘确认

Linux `ls --format json`：

```json
{"kind":"ls","stage":"ControlPlaneReady","reason_code":"OK","facts":[{"message":"peer_count=1"}]}
```

Windows `ls --format json`：

```json
{"kind":"ls","stage":"ControlPlaneReady","reason_code":"OK","facts":[{"message":"peer_count=1"}]}
```

结论：

- G1 的 peer config 已在双方落盘。
- 可以执行双向 ping gate。

### 7.2 Linux -> Windows UDP

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g2-01-linux-to-windows-ping-u.md \
  ping -u CSF7LBLQZ5WGRM7DMJZ6YJ3AYI
```

结果：

```text
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

结论：

- Linux -> Windows UDP punching 和 QUIC payload path 正常。
- 未见 `session_reused=true`。

### 7.3 Linux -> Windows TCP

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g2-02-linux-to-windows-ping-t-after-u.md \
  ping -t CSF7LBLQZ5WGRM7DMJZ6YJ3AYI
```

结果：

```text
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

结论：

- Linux -> Windows TCP punching 和 TLS payload path 正常。
- 尽管前一个 case 已建立 UDP path，本次没有报告 `session_reused=true`，实际走了 TCP。

### 7.4 Windows -> Linux UDP

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g2-04-windows-to-linux-ping-u.md' \
  ping -u NSW2B4OYFY4NRRKFENKRWI6XKE
```

结果：

```text
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

结论：

- Windows -> Linux UDP punching 和 QUIC payload path 正常。

### 7.5 Windows -> Linux TCP

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g2-05-windows-to-linux-ping-t-after-u.md' \
  ping -t NSW2B4OYFY4NRRKFENKRWI6XKE
```

结果：

```text
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

结论：

- Windows -> Linux TCP punching 和 TLS payload path 正常。
- G2 双向 UDP/TCP 最小 gate 全部通过。
- 可以进入 G3 shell discovery。

## 8. G3 shell discovery 初次结果

### 8.1 Linux -> Windows target discovery

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g3-01-linux-to-windows-sh-ls-targets.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI
```

结果摘要：

```text
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=ssh:ale
target=ssh:...
target=wsl:\u0000
target=wsl:\u0000D\u0000e\u0000b\u0000i\u0000a\u0000n...
target=wsl:U\u0000b\u0000u\u0000n\u0000t\u0000u...
```

事实：

- `ssh:*` target 枚举正常。
- `wsl:*` target 枚举被 NUL 字节污染。
- 原始 `wsl.exe -l -q` 输出确认是 UTF-16LE：

```text
00000000: 55 00 62 00 75 00 6e 00 74 00 75 00 0d 00 0a 00  U.b.u.n.t.u.....
00000010: 44 00 65 00 62 00 69 00 61 00 6e 00 0d 00 0a 00  D.e.b.i.a.n.....
```

结论：

- Windows target discovery 存在 WSL distro 输出解码 bug。
- 该问题会阻塞 `wsl:Debian` / `wsl:Ubuntu` 作为 shell target。
- 这不是 shell transport、punching 或 tmux 问题。
- 需要修复 `internal/shelltarget/targets_windows.go` 对 `wsl.exe -l -q` 输出的解码。
- 按本轮 debug 规则，已做临时最小 patch 以继续后续测试；该 patch 不提交，最终是否纳入由后续提交前判断。

### 8.2 Windows -> Linux target discovery

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g3-04-windows-to-linux-sh-ls-targets.md' \
  sh ls NSW2B4OYFY4NRRKFENKRWI6XKE
```

结果摘要：

```text
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
```

结论：

- Windows -> Linux shell target discovery 正常返回 `local`。
- 但 Linux `tmux` 当前缺失，后续 `local` session discovery / attach 仍预计会失败在 shell target 依赖层。

### 8.3 ssh target 约束

用户确认：

- 后续 `ssh:*` 真实 shell target 只使用 `ssh:ale`。
- 其它 `ssh:*` host 可能没有安装 `tmux`，只作为 discovery 事实，不进入 session discovery 或 attach 测试。

代码事实：

- `internal/pocacceptor/acceptor.go` 的 `serveShLS` 在 `shelltarget.ListSessions` 返回 `shelltarget.ErrTmuxMissing` 时，会返回 remote reason `SH_TMUX_MISSING`。
- `serveShAttach` 在 `shelltarget.Attach` 返回 `shelltarget.ErrTmuxMissing` 时，也会返回 `SH_TMUX_MISSING`。
- `internal/task/poc_dial.go` 会把 remote `SH_TMUX_MISSING` 映射成 POC reason `SH_TMUX_MISSING`。
- `tmuxMissingSuggestions(target)` 会按 target 类型给出明确建议：
  - `local`: 在被控节点安装 tmux。
  - `wsl:<distro>`: 在对应 WSL distro 内安装 tmux。
  - `ssh:<host>`: 在 SSH host 上安装 tmux。

结论：

- G3/G4 后续可执行 target 暂定：
  - Windows 被控端：`wsl:Debian` / `ssh:ale`
  - Linux 被控端：`local`，但当前被 Linux `tmux` 缺失阻塞
- 对其它 `ssh:*`，如果 SSH host 可达但缺少 tmux，正确分类应是 `SH_TMUX_MISSING`，不是 connectivity / punching failure。
- 如果 SSH 本身不可达或认证失败，则仍应归类为 `SH_CONNECTOR_FAIL` 或对应 connector 层失败。

### 8.4 WSL target 解码临时 patch 验证

临时 patch：

- 新增 `internal/shelltarget/windows_text.go`。
- `internal/shelltarget/targets_windows.go` 对 `wsl.exe -l -q` 输出调用 `decodeWindowsCommandOutput`。
- 新增 `internal/shelltarget/windows_text_test.go` 覆盖 UTF-16LE 和 UTF-8 输出。

聚焦测试：

```text
/usr/local/go/bin/go test ./internal/shelltarget -run 'TestDecodeWindowsCommandOutput' -count=1 -v

PASS
ok github.com/miopunch/miopunch/internal/shelltarget 0.003s
```

重新构建并重启 Windows daemon 后，Linux -> Windows target discovery 结果包含：

```text
target=ssh:ale
target=wsl:Debian
target=wsl:Ubuntu
```

结论：

- `wsl.exe -l -q` 输出确认为 UTF-16LE，原先按 UTF-8 解析导致 NUL 污染。
- 临时 patch 已验证能恢复干净 WSL target。
- G3 可以继续测试 `wsl:Debian` session discovery。

## 9. G3 session discovery 继续执行

### 9.1 tmux 缺失分类临时 patch

问题复现事实：

```text
tmux list-sessions: exit status 127: zsh:1: command not found: tmux
reason_code=SH_CONNECTOR_FAIL
```

结论：

- Windows 被控端 `wsl:Debian` 能启动 connector 并执行命令。
- 失败原因是 WSL distro 内缺少 `tmux`，不应归类为 connector/network 失败。
- 需要把 `zsh:1: command not found: tmux` 映射为 `SH_TMUX_MISSING`。

临时 patch：

- `internal/shelltarget/tmux_text.go`：把 `looksLikeTmuxMissing` 移到普通 Go 文件，便于在 Linux 测试环境中执行聚焦测试。
- `internal/shelltarget/tmux_text_test.go`：新增 `TestLooksLikeTmuxMissing`，覆盖 `sh: 1: tmux: not found`、`zsh:1: command not found: tmux`、Windows cmd 的 `'tmux' is not recognized`，并确认 SSH auth failure 不会被误判为 tmux 缺失。

聚焦测试：

```text
/usr/local/go/bin/go test ./internal/shelltarget -run 'Test(DecodeWindowsCommandOutput|LooksLikeTmuxMissing)' -count=1 -v

PASS
ok github.com/miopunch/miopunch/internal/shelltarget 0.003s
```

回归测试：

```text
/usr/local/go/bin/go test ./internal/task -run 'TestRunInviteTask(KeepsReachableHostnameBrokerInInviteCode|UsesReachableBuiltinBrokerWhenExplicitConfigAbsent|DoesNotMixBuiltinBrokerWhenExplicitConfigExists)|TestJoinApprovePersistEffectiveBrokerForPostJoinSignaling' -count=1 -v

PASS
ok github.com/miopunch/miopunch/internal/task 0.380s
```

构建：

```text
/usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux ./cmd/miopunch
GOOS=windows GOARCH=amd64 /usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe ./cmd/miopunch
```

daemon 重启：

- Linux daemon 旧进程：`2163720`，已停止。
- Windows daemon 旧进程：Windows `miopunch`，路径包含 `miopunch-wsl2-windows-shell`，已停止。
- 新 Linux daemon 启动后 LocalAPI：`unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock`。
- 新 Windows daemon 启动后 LocalAPI：`npipe:\\.\pipe\miopunch\localapi-S-1-5-21-1396067383-3453746490-1104383483-1001`。
- 双方 `ls --format json` 均为 `reason_code=OK`、`peer_count=1`。

### 9.2 Linux -> Windows `wsl:Debian` session discovery

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g3-05-linux-to-windows-sh-ls-wsl-debian-after-tmux-classification.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian
```

结果：

```text
task_id=NWK6TRMM62S7DEDH7WRZKN22AE
reason_code=SH_TMUX_MISSING
exit_code=3
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
tmux missing
suggestion=install tmux inside WSL distro (example: wsl.exe -d "Debian" -- sudo apt-get install tmux)
```

结论：

- `wsl:Debian` 失败已从 `SH_CONNECTOR_FAIL` 正确收敛为 `SH_TMUX_MISSING`。
- connector、punching、dataplane、hello 都已通过；阻塞点是被控 WSL distro 的 tmux 前置依赖。
- 不安装 Debian WSL 内 tmux 前，不继续 `wsl:Debian` attach。

### 9.3 Linux -> Windows `ssh:ale` session discovery

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g3-06-linux-to-windows-sh-ls-ssh-ale-serial.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale
```

结果：

```text
task_id=KLW5I2HWTZIJUTJACDO5PTYZ7U
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
session=main: 1 windows (created Thu May 14 15:13:04 2026)
```

结论：

- `ssh:ale` session discovery 串行执行通过。
- 之前 `ssh:ale` 的 `read sh_ls response: EOF` 是在与其它 `sh ls` case 并发执行、复用同一 session 的上下文中出现；本轮串行复测不能复现。
- 当前可继续使用 `ssh:ale` 作为 Windows 被控端真实 shell attach target。

## 10. G4 `ssh:ale` 交互基础闭环尝试

### 10.1 执行命令

使用独立 tmux session，避免影响已存在的 `main`：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-07-linux-to-windows-sh-ssh-ale-basic.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-g4-ssh-ale
```

交互输入：

```text
printf 'MIO_SHELL_OK_G4_SSH_ALE\n'
exit
```

本地 PTY 观测到远端输出：

```text
MIO_SHELL_OK_G4_SSH_ALE
Connection to 192.168.5.14 closed.
```

### 10.2 daemon report 与日志事实

task report：

```text
/tmp/miopunch-wsl2-windows-shell/run/linux/reports/6LMRXBAO7KFZHN33DNF4GEFYHM.md

task_id=6LMRXBAO7KFZHN33DNF4GEFYHM
kind=sh_attach
status=done
stage=SessionAttach
reason_code=UNAVAILABLE
exit_code=3
sid=f57943919812849d5a065f96004563b3
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-g4-ssh-ale
shell_layer=localapi_ws
shell_close=local websocket closed (1006): unexpected EOF
```

daemon log 关键证据：

```text
pocacceptor sh_attach ready
shell websocket attached
sh_attach bridge first remote stream data
sh_attach bridge first local websocket winsize
sh_attach bridge first local websocket data
sh_attach bridge first remote stream data write
conpty first read returned
conpty first write returned
pocacceptor sh_attach visitor disconnected: err=EOF
```

补充事实：

- `--report /tmp/.../reports/g4-07-linux-to-windows-sh-ssh-ale-basic.md` 没有生成最终导出文件，因为本地 CLI 挂住后被手动 `kill 2174143` 结束。
- daemon 内部 task report 已生成在 state path 下的 `run/linux/reports/6LMRXBAO7KFZHN33DNF4GEFYHM.md`。
- G4 后再次执行 `sh ls ... ssh:ale` 仍为 `reason_code=OK`，说明 daemon 和后续 `sh_ls` 能继续工作。

结论：

- `ssh:ale` 的真实交互 attach 已证明输入、输出、winsize、ConPTY read/write、远端 PTY write 链路可用。
- 该 case 不能标记为完整通过，因为远端 `exit` 后本地 CLI 没有自然收尾；最终 task reason 为 `UNAVAILABLE`，close fact 是 `localapi_ws` 的 abnormal close 1006。
- 后续需要把“远端退出后本地 CLI/task 正常收尾”作为单独生命周期问题继续定位。

## 11. G4 `ssh:ale` 生命周期收尾临时修复与复测

### 11.1 根因定位

代码事实：

- 被控端 `internal/pocacceptor/acceptor.go` 的 `serveShAttach` 会启动 `ptySess.Wait()` goroutine。
- 旧逻辑只有 `ptySess.Wait()` 返回非 nil error 时才通过 `runtimeFailCh` 通知主循环。
- 如果 Windows ConPTY/SSH/tmux 正常退出并返回 nil，主循环不会收到任何“后端已正常结束”的信号。
- Windows ConPTY 的读循环在该场景下没有及时驱动主循环退出；因此发起端本地 CLI 继续等待，直到本地进程被 kill，最终变成 `localapi_ws` abnormal close 1006。

结论：

- G4 首次失败不是输入输出链路问题。
- 阻塞点是 shell attach 生命周期协议缺少“远端 shell 正常退出”的明确完成信号。

### 11.2 临时 patch

修改文件：

- `internal/shellproto/control.go`
- `internal/pocacceptor/acceptor.go`
- `internal/task/sh_attach.go`

修复内容：

- 新增控制帧 op：`shell_exit`。
- 被控端 `serveShAttach` 在 PTY/ConPTY `Wait()` 正常返回 nil 后，发送 `shell_exit` / `ok=true`，然后结束 stream。
- 发起端 `bridgeShell` 收到 `shell_exit` / `ok=true` 后，把 task 结为 OK 并关闭桥接。
- 如果后端在初始 attach ready 前就退出，则仍归类为 setup failure，不把它误当成功。

聚焦测试：

```text
/usr/local/go/bin/go test ./internal/shellproto ./internal/pocacceptor ./internal/task -run 'Test(Frame|WriteJSON|ReadFrame|ShellAttach|LooksLike|RunInviteTask|JoinApprove|Bridge|DesktopState)' -count=1

ok github.com/miopunch/miopunch/internal/shellproto 0.003s
ok github.com/miopunch/miopunch/internal/pocacceptor 0.005s
ok github.com/miopunch/miopunch/internal/task 0.431s
```

构建并重启：

```text
/usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux ./cmd/miopunch
GOOS=windows GOARCH=amd64 /usr/local/go/bin/go build -o /tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe ./cmd/miopunch
```

重启后双方 `ls --format json`：

```text
reason_code=OK
peer_count=1
```

### 11.3 G4 `ssh:ale` 复测结果

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-07b-linux-to-windows-sh-ssh-ale-shell-exit-patch.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-g4-ssh-ale-exit2
```

交互输入：

```text
printf 'MIO_SHELL_OK_G4_SSH_ALE_EXIT2\n'
exit
```

本地 PTY 观测到：

```text
MIO_SHELL_OK_G4_SSH_ALE_EXIT2
Connection to 192.168.5.14 closed.
```

task report：

```text
task_id=WZKRPPWVQCABTBZMEY4FXKBO5U
kind=sh_attach
status=done
stage=SessionAttach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-g4-ssh-ale-exit2
```

daemon log 关键证据：

```text
pocacceptor sh_attach ready
shell websocket attached
sh_attach bridge first local websocket winsize
sh_attach bridge first local websocket data
conpty first read returned
conpty first write returned
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
sh_attach bridge remote shell exited
task done: task_id=WZKRPPWVQCABTBZMEY4FXKBO5U kind=sh_attach reason_code=OK exit_code=0
```

结论：

- 临时 `shell_exit` patch 已通过真实 Windows/WSL2/SSH/ConPTY 路径复测。
- `ssh:ale` 基础交互闭环现在完整通过：attach、winsize、输入、输出、远端 exit、本地 CLI 收尾、task OK。
- 该修复尚未提交，仍属于本轮 debug batch 临时工作树变更。

## 12. G6 `ssh:ale` tmux session detach / reconnect / cleanup

目的：

- 覆盖同一 target/session 的断开与重连。
- 区分“shell client 断开”与“tmux session 仍存在”。
- 验证 `shell_exit` patch 不只适用于 `exit`，也适用于 `tmux detach-client` 导致的 SSH/ConPTY 客户端结束。

### 12.1 第一次 attach 后 detach

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-ssh-ale-reconnect-01-attach-detach.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-reconnect1
```

交互输入：

```text
printf 'MIO_RECONNECT_START\n'
tmux detach-client
```

本地 PTY 观测到：

```text
MIO_RECONNECT_START
```

task report：

```text
task_id=JEPTURSFD7X3B7HB57SOQ73KIE
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-reconnect1
```

结论：

- `tmux detach-client` 后本地 CLI 自然退出。
- `shell_exit` patch 同样覆盖 detach 场景。

### 12.2 detach 后 session discovery

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-ssh-ale-reconnect-02-sh-ls-after-detach.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale
```

结果：

```text
task_id=W4CLF62JI53GHVPVPXIS5FXZHQ
reason_code=OK
session=main: 1 windows (created Thu May 14 15:13:04 2026)
session=mio-realtest-reconnect1: 1 windows (created Fri May 15 11:17:15 2026)
```

结论：

- detach 后 `mio-realtest-reconnect1` tmux session 仍存在。
- 这验证了“客户端断开”没有误杀远端 tmux session。

### 12.3 重新 attach 同一 session 并清理

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-ssh-ale-reconnect-03-reattach-exit.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-reconnect1
```

本地 PTY 观测到重连后的历史内容：

```text
printf 'MIO_RECONNECT_START\n'
MIO_RECONNECT_START
tmux detach-client
```

交互输入：

```text
printf 'MIO_RECONNECT_RESUME\n'
exit
```

本地 PTY 观测到：

```text
MIO_RECONNECT_RESUME
Connection to 192.168.5.14 closed.
```

task report：

```text
task_id=XXTGUQYHNBEPM2PYVJ5AWNSLW4
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-reconnect1
```

cleanup 验证：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-ssh-ale-reconnect-04-sh-ls-after-exit.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale

reason_code=OK
session=main: 1 windows (created Thu May 14 15:13:04 2026)
```

daemon log 关键证据：

```text
task done: task_id=JEPTURSFD7X3B7HB57SOQ73KIE kind=sh_attach reason_code=OK exit_code=0
task done: task_id=W4CLF62JI53GHVPVPXIS5FXZHQ kind=sh_ls reason_code=OK exit_code=0
task done: task_id=XXTGUQYHNBEPM2PYVJ5AWNSLW4 kind=sh_attach reason_code=OK exit_code=0
sh_attach bridge first remote stream json: op=shell_exit ok=true
sh_attach bridge remote shell exited
```

结论：

- `ssh:ale` 的 detach / session discovery / reattach / exit cleanup 闭环通过。
- 该 case 使用了既有 dataplane session，facts 中有 `session_reused=true`；它验证的是 shell session lifecycle，不是 fresh punching。

## 13. G4 `ssh:ale` transport 约束 attach

目的：

- 验证交互式 `sh` 在真实 attach 场景下遵守 `-u` / `-t` transport 约束。
- 每个 transport case 前重启隔离 daemon，避免被既有 dataplane session reuse 污染。

### 13.1 UDP-only attach

前置：

- 已停止并重启 `/tmp/miopunch-wsl2-windows-shell` 隔离 Linux / Windows daemon。
- 重启后双方 `ls --format json` 均为 `reason_code=OK`、`peer_count=1`。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-ssh-ale-udp-only-attach.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-udp-only1 -u
```

交互输入：

```text
printf 'MIO_SHELL_OK_SSH_ALE_UDP_ONLY\n'
exit
```

本地 PTY 观测到：

```text
MIO_SHELL_OK_SSH_ALE_UDP_ONLY
Connection to 192.168.5.14 closed.
```

task report：

```text
task_id=MCN6QI4E65TEYURC23RQGU7A3A
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=ssh:ale
session=mio-realtest-udp-only1
```

daemon log 关键证据：

```text
task done: task_id=MCN6QI4E65TEYURC23RQGU7A3A kind=sh_attach reason_code=OK exit_code=0
sh_attach bridge first remote stream json: op=shell_exit ok=true
pocacceptor sh_attach backend exited
```

结论：

- `ssh:ale` 交互 attach 的 `-u` case 通过。
- 真实链路为 UDP punching + QUIC payload：`attempt_path=punching_ipv4`、`path_family=udp4`、`data_proto=quic`。
- 输入、输出、远端 exit、本地 CLI 收尾均正常。

### 13.2 TCP-only attach

前置：

- UDP-only case 后再次停止并重启 `/tmp/miopunch-wsl2-windows-shell` 隔离 Linux / Windows daemon。
- 重启后双方 `ls --format json` 均为 `reason_code=OK`、`peer_count=1`。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-ssh-ale-tcp-only-attach.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-tcp-only1 -t
```

交互输入：

```text
printf 'MIO_SHELL_OK_SSH_ALE_TCP_ONLY\n'
exit
```

本地 PTY 观测到：

```text
MIO_SHELL_OK_SSH_ALE_TCP_ONLY
Connection to 192.168.5.14 closed.
```

task report：

```text
task_id=NRTSC2JGYVIRJW4BX7RUXD6374
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-tcp-only1
```

daemon log 关键证据：

```text
task done: task_id=NRTSC2JGYVIRJW4BX7RUXD6374 kind=sh_attach reason_code=OK exit_code=0
sh_attach bridge remote shell exited
pocacceptor sh_attach backend exited
```

cleanup 验证：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-ssh-ale-transport-cleanup-sh-ls.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale

reason_code=OK
session=main: 1 windows (created Thu May 14 15:13:04 2026)
```

结论：

- `ssh:ale` 交互 attach 的 `-t` case 通过。
- 真实链路为 TCP punching + TLS payload：`attempt_path=punching_tcp4`、`path_family=tcp4`、`data_proto=tls`。
- transport 约束矩阵中 `ssh:ale` 的 `-u` / `-t` 交互 attach 均已通过。

## 14. G5 `ssh:ale` 大输出 smoke

目的：

- 覆盖连续较大文本输出不会导致 LocalAPI WebSocket、dataplane stream、Windows ConPTY 或 tmux/ssh 路径死锁。
- 本轮先用 1000 行 marker 做 smoke，不等同于后续 1 MiB / 5000 行压力测试。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-ssh-ale-large-output-1000-lines.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale -s mio-realtest-big-output1
```

交互输入：

```text
for i in $(seq 1 1000); do printf 'MIO_BIG_%04d\n' "$i"; done; printf 'MIO_BIG_DONE\n'
exit
```

本地 PTY 观测到：

```text
MIO_BIG_0001
...
MIO_BIG_1000
MIO_BIG_DONE
```

说明：

- 终端输出包含大量 ANSI/tmux 重绘序列，记录中只保留关键 marker。
- 已观测到起始 marker、末尾 marker 和完成 marker。

task report：

```text
task_id=VG2OLIORGKRSITTMCKSPGZCESU
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=ssh:ale
session=mio-realtest-big-output1
```

daemon log 关键证据：

```text
shell websocket attached
sh_attach bridge first local websocket data
sh_attach bridge first remote stream data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=VG2OLIORGKRSITTMCKSPGZCESU kind=sh_attach reason_code=OK exit_code=0
```

cleanup 验证：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-ssh-ale-large-output-cleanup-sh-ls.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI ssh:ale

reason_code=OK
session=main: 1 windows (created Thu May 14 15:13:04 2026)
```

结论：

- `ssh:ale` 1000 行连续输出 smoke 通过。
- 未观察到死锁、提前关闭或 task failure。
- 后续仍需要单独补更大的 1 MiB / 5000 行压力 case，以及输出完整性自动校验脚本；本轮只记录人工可见 marker 与 task/log 证据。

## 15. G3/G4 `local` / `wsl:Debian` tmux 实测恢复

日期：2026-05-15

用户补充事实：

- 当前 WSL2 host 已安装 `tmux`，等价于 Windows 侧 `wsl:Debian` 环境也具备 `tmux`。

前置验证：

```text
tmux -V
tmux 3.1c

wsl.exe -d Debian -- tmux -V
tmux 3.1c
```

### 15.1 `sh ls` 空 tmux server 兼容问题

首次恢复 `local` / `wsl:Debian` 后发现两个真实问题：

```text
tmux list-sessions: exit status 1: error connecting to /tmp/tmux-1000/default (No such file or directory)
```

```text
tmux: option requires an argument -- F
error connecting to /tmp//tmux-1000/default (No such file or directory)
```

事实与结论：

- `tmux 3.1c` 在没有 tmux server 时返回 `error connecting to ... (No such file or directory)`，应归类为空 session 列表，不应归类为 shell target 失败。
- Windows 通过 `wsl.exe -- tmux list-sessions -F "#S"` 时，`#S` 参数被 WSL 调用链吃掉或改写，导致 `tmux` 看到 `-F` 但没有 format 参数。
- 临时最小改动：增加 no-server 文本识别；Windows `wsl:<distro>` session discovery 暂时改为不传 `-F "#S"`。

补充 focused test：

```text
go test ./internal/shelltarget -run 'TestLooksLike(TmuxMissing|NoTmuxServer)|TestDecodeWindowsCommandOutput' -count=1 -v
PASS
```

重编译并重启双端 daemon 后，`sh ls` 通过：

```text
Windows -> Linux local
task_id=QPJJ5QKHRAQ6DJQ6BGVF2GITAE
kind=sh_ls
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
```

```text
Linux -> Windows wsl:Debian
task_id=NC6GWPM4EAPY736XWJE6SBTQQ4
kind=sh_ls
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
```

结论：

- `local` 和 `wsl:Debian` 在无 tmux server 时均能返回 `OK` 空 session 列表。
- 该 case 证明 target/session discovery 可用，但不证明 attach 输入输出闭环。

### 15.2 Windows -> Linux `local` 基础 attach

首次 attach 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g4-01-windows-to-linux-local-basic-after-tmux.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-linux-local1
```

交互输入：

```text
printf 'MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL\n'
exit
```

本地 PTY 观测：

```text
MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL
```

失败 report：

```text
task_id=SDKPLE32ZP3O4CHVNMLCSCTCSY
kind=sh_attach
reason_code=SH_CONNECTOR_FAIL
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
session=mio-realtest-linux-local1
shell_layer=pty
shell_close=pty read failed: read /dev/ptmx: input/output error
```

daemon log 关键证据：

```text
pocacceptor sh_attach first pty read
pocacceptor sh_attach first stream data write
sh_attach bridge first local websocket data
pocacceptor sh_attach first pty write
pocacceptor sh_attach runtime failed: reason_code=SH_CONNECTOR_FAIL shell_layer=pty err=read /dev/ptmx: input/output error
```

结论：

- 输入输出已经到达远端 PTY，失败发生在 shell 退出后的 PTY read close 处理。
- Linux PTY 在 child shell 正常退出后可能返回 `read /dev/ptmx: input/output error`；该错误不应覆盖后续 `Wait()` 的正常退出结果。

临时最小改动：

- `internal/pocacceptor/acceptor.go`：PTY read 遇到 expected close error 时等待 backend `Wait()`，由 backend exit 决定最终 shell result。
- `internal/pocacceptor/acceptor_test.go`：增加 expected PTY close / wait failure focused test。

补充 focused test：

```text
go test ./internal/pocacceptor -run 'Test(IsExpectedShellAttachPTYReadClose|ShellAttachWaitFailure)' -count=1 -v
PASS
```

重编译并重启双端 daemon 后重试：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g4-01-windows-to-linux-local-basic-after-pty-eio-patch.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-linux-local2
```

交互输入：

```text
printf 'MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL2\n'
exit
```

本地 PTY 观测：

```text
MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL2
```

task report：

```text
task_id=BQR2RMMXTWW2QIXTCRDCROFGSY
kind=sh_attach
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=local
session=mio-realtest-linux-local2
```

daemon log 关键证据：

```text
shell websocket attached
sh_attach bridge first remote stream data
sh_attach bridge first local websocket data
pocacceptor sh_attach first visitor data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=BQR2RMMXTWW2QIXTCRDCROFGSY kind=sh_attach reason_code=OK exit_code=0
```

结论：

- Windows -> Linux `local` 基础 attach 通过。
- 真实链路为 TCP punching + TLS payload：`attempt_path=punching_tcp4`、`path_family=tcp4`、`data_proto=tls`。
- `local` target 的 PTY 正常退出闭环已被验证：输入、输出、远端 exit、本地 CLI 收尾均正常。

### 15.3 Linux -> Windows `wsl:Debian` 基础 attach

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-04-linux-to-windows-wsl-debian-basic-after-tmux.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-debian1
```

交互输入：

```text
printf 'MIO_SHELL_OK_LINUX_TO_WINDOWS_WSL_DEBIAN\n'
exit
```

本地 PTY 观测：

```text
MIO_SHELL_OK_LINUX_TO_WINDOWS_WSL_DEBIAN
[exited]
```

task report：

```text
task_id=JRKYJRZ4WZUEBAAASCA6X3IZGE
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-debian1
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-debian1"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
sh_attach bridge first local websocket winsize: size=80x24
conpty resize done: size=80x24 err=<nil>
sh_attach bridge first local websocket data
conpty first write returned: bytes=57 requested=57 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=JRKYJRZ4WZUEBAAASCA6X3IZGE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- Linux -> Windows `wsl:Debian` 基础 attach 通过。
- Windows ConPTY、`wsl.exe`、WSL2 Debian 内 `tmux`、LocalAPI WebSocket、dataplane stream、远端 `shell_exit` 收尾链路均有实测证据。
- 真实链路为 TCP punching + TLS payload：`attempt_path=punching_tcp4`、`path_family=tcp4`、`data_proto=tls`。

### 15.4 Windows -> Linux `local` transport 约束

#### UDP-only

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g4-02-windows-to-linux-local-udp-only.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-linux-local-udp1 -u
```

交互输入：

```text
printf 'MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL_UDP\n'
exit
```

task report：

```text
task_id=2CKQVTSDI45TXJCTDK3HUWQV4U
kind=sh_attach
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=local
session=mio-realtest-linux-local-udp1
```

daemon log 关键证据：

```text
pocacceptor connectivity attempt ready: path=punching_ipv4 protocol=quic
shell websocket attached
sh_attach bridge first remote stream data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=2CKQVTSDI45TXJCTDK3HUWQV4U kind=sh_attach reason_code=OK exit_code=0
```

#### TCP-only

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g4-03-windows-to-linux-local-tcp-only.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-linux-local-tcp1 -t
```

交互输入：

```text
printf 'MIO_SHELL_OK_WINDOWS_TO_LINUX_LOCAL_TCP\n'
exit
```

task report：

```text
task_id=GFMVX2MZH2UMHC2JQ6T4MGCDYI
kind=sh_attach
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=local
session=mio-realtest-linux-local-tcp1
```

daemon log 关键证据：

```text
tcp tls handshake ok
shell websocket attached
sh_attach bridge first remote stream data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=GFMVX2MZH2UMHC2JQ6T4MGCDYI kind=sh_attach reason_code=OK exit_code=0
```

结论：

- Windows -> Linux `local` 的 `-u` / `-t` 交互 attach 均通过。
- `-u` 使用 UDP punching + QUIC payload：`attempt_path=punching_ipv4`、`path_family=udp4`、`data_proto=quic`。
- `-t` 使用 TCP punching + TLS payload：`attempt_path=punching_tcp4`、`path_family=tcp4`、`data_proto=tls`。

### 15.5 Linux -> Windows `wsl:Debian` transport 约束

#### UDP-only 首次尝试

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-05-linux-to-windows-wsl-debian-udp-only.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-debian-udp1 -u
```

失败 report：

```text
task_id=2KBGZRGED34C5UJK4MDFU46RY4
kind=sh_attach
stage=PunchAttempt
reason_code=UNAVAILABLE
exit_code=3
attempt_diag: attempt.punching.timeout msg=punching timeout err=context deadline exceeded kvs={"elapsed_ms":5009}
attempt_diag: attempt.punching.fail msg=punching failed err=wait detect message error: context deadline exceeded
dial peer: wait detect message error: context deadline exceeded
suggestions:
- retry
```

结论：

- 首次 `wsl:Debian -u` 失败发生在 UDP punching 阶段，尚未进入 dataplane / hello / shell attach。
- 该失败不能归类为 ConPTY、`wsl.exe`、tmux 或 shell target 问题。

#### TCP-only

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-06-linux-to-windows-wsl-debian-tcp-only.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-debian-tcp1 -t
```

交互输入：

```text
printf 'MIO_SHELL_OK_LINUX_TO_WINDOWS_WSL_DEBIAN_TCP\n'
exit
```

task report：

```text
task_id=C6LZX63JVOHM2OZMZHOSTZUCRE
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-debian-tcp1
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-debian-tcp1"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
conpty resize done: size=80x24 err=<nil>
conpty first write returned: bytes=61 requested=61 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=C6LZX63JVOHM2OZMZHOSTZUCRE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian -t` 交互 attach 通过。
- 因 report 包含 `session_reused=true`，该条只能作为 TCP session 上的 shell 闭环证据，不作为 fresh TCP punching 证据。

#### UDP-only retry

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g4-05b-linux-to-windows-wsl-debian-udp-only-retry.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-debian-udp2 -u
```

交互输入：

```text
printf 'MIO_SHELL_OK_LINUX_TO_WINDOWS_WSL_DEBIAN_UDP_RETRY\n'
exit
```

task report：

```text
task_id=ZZPVLSDX7K6LI5QIT7Z5RAJXLE
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-debian-udp2
```

daemon log 关键证据：

```text
pocacceptor connectivity attempt ready: path=punching_ipv4 protocol=quic
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-debian-udp2"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
conpty resize done: size=80x24 err=<nil>
conpty first write returned: bytes=67 requested=67 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=ZZPVLSDX7K6LI5QIT7Z5RAJXLE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian -u` retry 通过，真实链路为 UDP punching + QUIC payload。
- 本轮 `wsl:Debian` 已覆盖 auto、`-u`、`-t` 的交互 attach；其中 `-u` 有一次真实 UDP punching transient failure，重试成功。

### 15.6 G4 cleanup discovery

G4 transport 约束测试后执行 cleanup `sh ls`：

```text
Windows -> Linux local
task_id=QS7OFYWS4ABUY2CGK5KLRWWNQE
kind=sh_ls
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
```

```text
Linux -> Windows wsl:Debian
task_id=7AFEBBOCJ5J4MVWZLYBRLFDBYE
kind=sh_ls
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
```

结论：

- 两个 `sh ls` 均没有 `session=` facts，表示刚才 `exit` 后未残留已命名 tmux session。
- 后续 G5 继续使用新的 session 名，避免被旧 session 污染。

## 16. G5 `local` / `wsl:Debian` 大输出 smoke

目的：

- 在 `ssh:ale` 之外补齐 Linux PTY `local` 与 Windows ConPTY + WSL `wsl:Debian` 的连续输出 smoke。
- 本轮仍用 1000 行 marker，不等同于完整 1 MiB / 5000 行压力测试。

### 16.1 Windows -> Linux `local` 1000 行输出

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g5-local-large-output-1000-lines.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-local-big-output1
```

交互输入：

```text
for i in $(seq 1 1000); do printf 'MIO_LOCAL_BIG_%04d\n' "$i"; done; printf 'MIO_LOCAL_BIG_DONE\n'
exit
```

本地 PTY 观测到：

```text
MIO_LOCAL_BIG_1000
MIO_LOCAL_BIG_DONE
```

task report：

```text
task_id=FHQSUB7H7GD6MCDTQPUZDCZOZM
kind=sh_attach
reason_code=OK
sid=950bf945aa3901ad4a9d979d7ff8e159
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
session=mio-realtest-local-big-output1
```

daemon log 关键证据：

```text
pocacceptor sh_attach first pty read
shell websocket attached
sh_attach bridge first local websocket data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=FHQSUB7H7GD6MCDTQPUZDCZOZM kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `local` 1000 行连续输出 smoke 通过。
- 因 report 包含 `session_reused=true`，该条验证 payload/session 上的终端输出行为，不作为 fresh punching 证据。

### 16.2 Linux -> Windows `wsl:Debian` 1000 行输出

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-wsl-debian-large-output-1000-lines.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-big-output1
```

交互输入：

```text
for i in $(seq 1 1000); do printf 'MIO_WSL_BIG_%04d\n' "$i"; done; printf 'MIO_WSL_BIG_DONE\n'
exit
```

本地 PTY 观测到：

```text
MIO_WSL_BIG_1000
MIO_WSL_BIG_DONE
```

task report：

```text
task_id=N6M5BE434E2GG4JUEOY5JP5AQU
kind=sh_attach
reason_code=OK
sid=f57943919812849d5a065f96004563b3
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-big-output1
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-big-output1"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
sh_attach bridge first local websocket winsize: size=80x24
conpty resize done: size=80x24 err=<nil>
conpty first write returned: bytes=100 requested=100 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge first remote stream json: op=shell_exit ok=true
task done: task_id=N6M5BE434E2GG4JUEOY5JP5AQU kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian` 1000 行连续输出 smoke 通过。
- Windows ConPTY + `wsl.exe` + WSL2 Debian tmux 路径未观察到死锁、提前关闭或 task failure。
- 因 report 包含 `session_reused=true`，该条验证既有 UDP/QUIC session 上的终端输出行为，不作为 fresh punching 证据。

## 17. G6 `local` / `wsl:Debian` detach / reconnect

目的：

- 验证 `ssh:ale` 之外的 shell session 恢复 / 重连语义。
- 流程为 attach -> 写 marker -> tmux detach -> `sh ls` 看到残留 session -> reattach 同名 session -> 看到旧 marker -> 写新 marker -> `exit` 清理 -> `sh ls` 不再看到 session。

### 17.1 Windows -> Linux `local`

attach + detach 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g6-local-reconnect-01-attach-detach.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-local-reconnect1
```

交互输入：

```text
printf 'MIO_LOCAL_RECONNECT_BEFORE_DETACH\n'
<tmux prefix Ctrl-b> d
```

本地 PTY 观测到：

```text
MIO_LOCAL_RECONNECT_BEFORE_DETACH
[detached (from session mio-realtest-local-reconnect1)]
```

task report：

```text
task_id=A2GDQMJ76VZJGSU6KRFGDMIEBM
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
session=mio-realtest-local-reconnect1
```

detach 后 `sh ls`：

```text
task_id=HYBZAVF5HZDPUQPMQ7CCKWVOWY
kind=sh_ls
reason_code=OK
session=mio-realtest-local-reconnect1
```

reattach + exit 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g6-local-reconnect-03-reattach-exit.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-local-reconnect1
```

reattach 后本地 PTY 观测到旧 marker：

```text
MIO_LOCAL_RECONNECT_BEFORE_DETACH
```

交互输入：

```text
printf 'MIO_LOCAL_RECONNECT_AFTER_REATTACH\n'
exit
```

本地 PTY 观测到：

```text
MIO_LOCAL_RECONNECT_AFTER_REATTACH
[exited]
```

task report：

```text
task_id=NAPRWSETTC3ETPKWUCMQX7ELNE
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
session=mio-realtest-local-reconnect1
```

exit 后 cleanup `sh ls`：

```text
task_id=WA2ZKDEIERWTB5Q2UGF6BYZLXY
kind=sh_ls
reason_code=OK
session facts: none
```

结论：

- `local` detach / reconnect / exit cleanup 通过。
- `sh ls` 能正确暴露 detached tmux session，并在最终 `exit` 后不再暴露该 session。

### 17.2 Linux -> Windows `wsl:Debian`

attach + detach 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-wsl-debian-reconnect-01-attach-detach.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-reconnect1
```

交互输入：

```text
printf 'MIO_WSL_RECONNECT_BEFORE_DETACH\n'
<tmux prefix Ctrl-b> d
```

本地 PTY 观测到：

```text
MIO_WSL_RECONNECT_BEFORE_DETACH
```

task report：

```text
task_id=JGTQFCD47DLIO4FW4VYJT56MQM
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-reconnect1
```

detach 后 `sh ls`：

```text
task_id=36CFK2YCRW6KFEYC23EZIYZAVM
kind=sh_ls
reason_code=OK
session=mio-realtest-wsl-reconnect1: 1 windows (created Fri May 15 11:50:16 2026)
```

reattach + exit 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-wsl-debian-reconnect-03-reattach-exit.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-reconnect1
```

reattach 后本地 PTY 观测到旧 marker：

```text
MIO_WSL_RECONNECT_BEFORE_DETACH
```

交互输入：

```text
printf 'MIO_WSL_RECONNECT_AFTER_REATTACH\n'
exit
```

本地 PTY 观测到：

```text
MIO_WSL_RECONNECT_AFTER_REATTACH
```

task report：

```text
task_id=HDJHEUE24BJMZOY7IYCLECPIWY
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-reconnect1
```

exit 后 cleanup `sh ls`：

```text
task_id=UEAPIAFDZMQQ5AXH6BSMLDLJEQ
kind=sh_ls
reason_code=OK
session facts: none
```

结论：

- `wsl:Debian` detach / reconnect / exit cleanup 通过。
- Windows ConPTY + `wsl.exe` + WSL2 Debian tmux 路径支持 detach 后 `sh ls` discovery、同名 reattach 和最终 exit cleanup。

## 18. G3 discovery transport variants

目的：

- 补齐空 target discovery 的 `-u` / `-t` 约束，确认 target 枚举不只在 auto transport 下可用。
- 本节只验证 target discovery，不验证 target session discovery 或 attach。

### 18.1 Linux -> Windows empty target

UDP-only：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g3-02-linux-to-windows-sh-ls-targets-udp.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI -u
```

结果：

```text
task_id=UKPDAESYCK2EPE4SX4TEEKHFGQ
kind=sh_ls
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=ssh:ale
target=wsl:Debian
target=wsl:Ubuntu
```

TCP-only：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g3-03-linux-to-windows-sh-ls-targets-tcp.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI -t
```

结果：

```text
task_id=VR6GETXH3DIHERDADIGRC3P6HA
kind=sh_ls
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=ssh:ale
target=wsl:Debian
target=wsl:Ubuntu
```

说明：

- Windows 侧 `ssh:*` target 枚举来自 Windows 用户 ssh config；本轮 attach/session 测试仍只使用 `ssh:ale`，不代表其他 `ssh:*` host 都具备远端 tmux。

### 18.2 Windows -> Linux empty target

UDP-only 首次尝试：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g3-05-windows-to-linux-sh-ls-targets-udp.md' \
  sh ls NSW2B4OYFY4NRRKFENKRWI6XKE -u
```

失败结果：

```text
task_id=X3WBA7OM5TTZOHRFXIQCM4H4FM
kind=sh_ls
stage=CandidateExchange
reason_code=UNAVAILABLE
mqtt broker skipped: broker.emqx.io:1883: future canceled
dial peer: broker.emqx.io:1883: future canceled
suggestions:
- retry
```

结论：

- 首次 `-u` 失败发生在 CandidateExchange，还没进入 shell discovery。
- 该失败不归类为 target discovery 问题。

UDP-only retry：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g3-05b-windows-to-linux-sh-ls-targets-udp-retry.md' \
  sh ls NSW2B4OYFY4NRRKFENKRWI6XKE -u
```

结果：

```text
task_id=Y7FVPVXSHOCYRZ2CAOC5J2D5OI
kind=sh_ls
reason_code=OK
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=local
```

TCP-only：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g3-06-windows-to-linux-sh-ls-targets-tcp.md' \
  sh ls NSW2B4OYFY4NRRKFENKRWI6XKE -t
```

结果：

```text
task_id=G62XZZLMKY63R2FOL4TEPRILRI
kind=sh_ls
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
```

结论：

- 空 target discovery 的 `-u` / `-t` variants 已通过。
- Windows -> Linux `-u` 出现过一次 broker future canceled transient，retry 后 UDP/QUIC discovery 正常。

## 19. G5 Ctrl-C interrupt / continue

目的：

- 验证长命令被 `Ctrl-C` 中断后，shell session 不会断开，仍能继续输入命令并正常 `exit`。

### 19.1 Windows -> Linux `local`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g5-local-ctrl-c-continues.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-local-ctrlc1
```

交互输入：

```text
sleep 30
<Ctrl-C>
printf 'MIO_LOCAL_CTRL_C_STILL_ALIVE\n'
exit
```

本地 PTY 观测到：

```text
sleep 30
^C
MIO_LOCAL_CTRL_C_STILL_ALIVE
[exited]
```

task report：

```text
task_id=EJ5L3NRITJT2VJU4X3FWT3EBSE
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=local
session=mio-realtest-local-ctrlc1
```

daemon log 关键证据：

```text
shell websocket attached
sh_attach bridge first local websocket data
pocacceptor sh_attach first pty write
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=EJ5L3NRITJT2VJU4X3FWT3EBSE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `local` target 中 `Ctrl-C` 可以中断长命令，shell 后续仍可输入 marker 并正常退出。

### 19.2 Linux -> Windows `wsl:Debian`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-wsl-debian-ctrl-c-continues.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-ctrlc1
```

交互输入：

```text
sleep 30
<Ctrl-C>
printf 'MIO_WSL_CTRL_C_STILL_ALIVE\n'
exit
```

本地 PTY 观测到：

```text
sleep 30
^C
MIO_WSL_CTRL_C_STILL_ALIVE
[exited]
```

task report：

```text
task_id=RWQWHX3WC52JWGEBN5F7EWNRTM
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-ctrlc1
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-ctrlc1"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
conpty resize done: size=80x24 err=<nil>
conpty first write returned: bytes=9 requested=9 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=RWQWHX3WC52JWGEBN5F7EWNRTM kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian` target 中 `Ctrl-C` 可以中断长命令，shell 后续仍可输入 marker 并正常退出。
- Windows ConPTY + `wsl.exe` 路径中未观察到 Ctrl-C 后 session 损坏。

## 20. G5 backspace line editing

目的：

- 验证本地输入中的 backspace 能穿过 LocalAPI WebSocket、dataplane stream、远端 PTY/ConPTY 和 tmux，由远端 shell 行编辑正确处理。
- 测试方法：先输入 `..._EDIT_X`，再发送 backspace 删除 `X`，补 `OK` 后提交。期望最终执行并输出 `..._EDIT_OK`，而不是 `..._EDIT_XOK`。

### 20.1 Windows -> Linux `local`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g5-local-backspace-edit.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-local-edit1
```

交互输入：

```text
printf 'MIO_LOCAL_EDIT_X<backspace>OK\n'
exit
```

本地 PTY 观测到提交行已被修正：

```text
printf 'MIO_LOCAL_EDIT_OK\n'
MIO_LOCAL_EDIT_OK
[exited]
```

task report：

```text
task_id=CTI44QHZ2TPD23BPHIBSUTNVHI
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=local
session=mio-realtest-local-edit1
```

结论：

- `local` target 的 backspace 行编辑通过。

### 20.2 Linux -> Windows `wsl:Debian`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-wsl-debian-backspace-edit.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-wsl-edit1
```

交互输入：

```text
printf 'MIO_WSL_EDIT_X<backspace>OK\n'
exit
```

本地 PTY 观测到提交行已被修正：

```text
printf 'MIO_WSL_EDIT_OK\n'
MIO_WSL_EDIT_OK
[exited]
```

task report：

```text
task_id=SUYVUI2CKWOWHGIOZJ6DZQAD6M
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-wsl-edit1
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-wsl-edit1"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
conpty resize done: size=80x24 err=<nil>
conpty first write returned: bytes=34 requested=34 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=SUYVUI2CKWOWHGIOZJ6DZQAD6M kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian` target 的 backspace 行编辑通过。
- Windows ConPTY + `wsl.exe` 路径中未观察到输入编辑控制字符导致 session 损坏。

## 21. G6 fresh daemon transport reuse semantics

目的：

- 验证显式 transport 约束不会错误复用已有的相反 transport session。
- 流程为 fresh daemon 后先建立一种 transport，再用相反 transport 执行 shell attach。

环境操作：

- 为清除内存 session，重启隔离 Linux / Windows daemon，但保留既有 state/network。
- 首次批量 kill 命令使用 `pgrep` 时匹配到了当前 shell wrapper，导致该条命令自身被 kill，未完成启动检查；随后改为读取具体 PID 并重启长期 `up` session。
- 恢复后双端 LocalAPI gate 均为 `reason_code=OK`、`peer_count=1`。

### 21.1 `ping -u` 后 `sh -t`

precondition：

```text
Linux LocalAPI ls: reason_code=OK peer_count=1
Windows LocalAPI ls: reason_code=OK peer_count=1
```

先执行 UDP-only ping：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-01-linux-to-windows-ping-udp-before-sh-tcp.md \
  ping CSF7LBLQZ5WGRM7DMJZ6YJ3AYI -u
```

ping report：

```text
task_id=X6GGK2NQOH74YH5TXRXZBJ6LVY
kind=ping
reason_code=OK
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

再执行 TCP-only shell：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-02-linux-to-windows-sh-tcp-after-ping-udp.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-reuse-udp-before-tcp -t
```

交互输入：

```text
printf 'MIO_REUSE_UDP_BEFORE_TCP_OK\n'
exit
```

shell report：

```text
task_id=JOUUMTQAZZ2RJYI7CUKYR7HBTI
kind=sh_attach
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-reuse-udp-before-tcp
```

结论：

- `ping -u` 建立 UDP/QUIC 后，随后的 `sh -t` 没有复用 UDP session。
- `sh -t` 明确执行 TCP punching + TLS payload：`attempt_path=punching_tcp4`、`path_family=tcp4`、`data_proto=tls`。

### 21.2 `ping -t` 后 `sh -u`

precondition：

- 再次重启隔离 daemon，清除上一组内存 session。
- 恢复后双端 LocalAPI gate 均为 `reason_code=OK`、`peer_count=1`。

先执行 TCP-only ping：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-03-linux-to-windows-ping-tcp-before-sh-udp.md \
  ping CSF7LBLQZ5WGRM7DMJZ6YJ3AYI -t
```

ping report：

```text
task_id=BKPHSOXIDOVO2P3DM7NF4ONG6U
kind=ping
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

再执行 UDP-only shell：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-04-linux-to-windows-sh-udp-after-ping-tcp.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-reuse-tcp-before-udp -u
```

交互输入：

```text
printf 'MIO_REUSE_TCP_BEFORE_UDP_OK\n'
exit
```

shell report：

```text
task_id=4OEU7BUCH4YFUPGJ4DG7HUBZOU
kind=sh_attach
reason_code=OK
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-reuse-tcp-before-udp
```

结论：

- `ping -t` 建立 TCP/TLS 后，随后的 `sh -u` 没有复用 TCP session。
- `sh -u` 明确执行 UDP punching + QUIC payload：`attempt_path=punching_ipv4`、`path_family=udp4`、`data_proto=quic`。
- 显式 `-u` / `-t` transport 约束在 shell attach 上能覆盖已有相反 transport session，符合测试预期。

### 21.3 `sh ls -u` / `sh -t` 与 `sh ls -t` / `sh -u`

目的：

- 继续验证 shell discovery (`sh ls`) 建立的 transport 不会污染后续 interactive shell attach 的显式相反 transport。
- 这组比 21.1 / 21.2 更贴近 shell 本身，因为前置任务同样是 `sh_ls`，不是 `ping`。

#### 21.3.1 `sh ls -u` 后 `sh -t`

precondition：

- 重启隔离 daemon，清除上一组内存 session。
- 恢复后双端 LocalAPI gate 均为 `reason_code=OK`、`peer_count=1`。

先执行 UDP-only `sh ls`：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-05-sh-ls-wsl-udp-before-sh-tcp.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -u
```

`sh ls` report：

```text
task_id=5WYMAQYCHPUKR7E36UE64YKURI
kind=sh_ls
reason_code=OK
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
```

再执行 TCP-only shell：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-06-sh-tcp-after-sh-ls-udp.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-ls-udp-before-tcp -t
```

交互输入：

```text
printf 'MIO_LS_UDP_BEFORE_TCP_OK\n'
exit
```

shell report：

```text
task_id=AXXGTDY7TPHAXH22MNEHLHPJ5I
kind=sh_attach
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-ls-udp-before-tcp
```

结论：

- `sh ls -u` 建立 UDP/QUIC 后，随后的 `sh -t` 没有复用 UDP session。
- `sh -t` 明确执行 TCP punching + TLS payload。

#### 21.3.2 `sh ls -t` 后 `sh -u`

precondition：

- 再次重启隔离 daemon，清除上一组内存 session。
- 恢复后双端 LocalAPI gate 均为 `reason_code=OK`、`peer_count=1`。

先执行 TCP-only `sh ls`：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-07-sh-ls-wsl-tcp-before-sh-udp.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -t
```

`sh ls` report：

```text
task_id=23J2SJD6AOBKWZQM5ZQ3SEURPE
kind=sh_ls
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
```

再执行 UDP-only shell：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-reuse-08-sh-udp-after-sh-ls-tcp.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-ls-tcp-before-udp -u
```

交互输入：

```text
printf 'MIO_LS_TCP_BEFORE_UDP_OK\n'
exit
```

shell report：

```text
task_id=ILPI4S4S3Q26LU3KEP4OHAGUA4
kind=sh_attach
reason_code=OK
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-ls-tcp-before-udp
```

结论：

- `sh ls -t` 建立 TCP/TLS 后，随后的 `sh -u` 没有复用 TCP session。
- `sh -u` 明确执行 UDP punching + QUIC payload。
- 到目前为止，`ping` 前置和 `sh ls` 前置两类相反 transport reuse 组合均通过。

## 22. G5 5000 行输出压力

目的：

- 在真实 interactive shell 中输出 5000 行，确认 shell 不只适合短 marker，而能处理较大连续 stdout。
- 本组以观察尾部 marker 和 task report 为准；由于终端屏幕会滚动/重绘，CLI 捕获内容不是逐行完整归档工具。

### 22.1 Linux -> Windows `wsl:Debian`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-pressure-01-wsl-5000-lines.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-5000
```

交互输入：

```text
printf 'MIO_G5_WSL_5000_START\n'; seq -f 'MIO_G5_WSL_5000_%04g' 1 5000; printf 'MIO_G5_WSL_5000_END\n'
exit
```

观察到的尾部证据：

```text
MIO_G5_WSL_5000_4998
MIO_G5_WSL_5000_4999
MIO_G5_WSL_5000_5000
MIO_G5_WSL_5000_END
```

task report：

```text
task_id=A5QSBW25RBW7WP6K5EVRQMTLXA
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-g5-wsl-5000
```

daemon log 关键证据：

```text
conpty create start: application=wsl.exe size=80x24
conpty process started: command_line="wsl.exe -d Debian -- tmux new -A -s mio-realtest-g5-wsl-5000"
conpty first read returned: bytes=16 err=<nil>
shell websocket attached
conpty first write returned: bytes=108 requested=108 err=<nil>
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=A5QSBW25RBW7WP6K5EVRQMTLXA kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian` 5000 行输出未死锁，尾部 marker 到达，`exit` 后 task 正常收尾。

### 22.2 Windows -> Linux `local`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g5-pressure-02-local-5000-lines.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-g5-local-5000
```

执行事实：

- Windows CLI 启动后先输出 `ESC[6n`。
- 按既有真机经验回传 `ESC[1;1R` 后进入远端 `local` prompt。

交互输入：

```text
printf 'MIO_G5_LOCAL_5000_START\n'; seq -f 'MIO_G5_LOCAL_5000_%04g' 1 5000; printf 'MIO_G5_LOCAL_5000_END\n'
exit
```

观察到的尾部证据：

```text
MIO_G5_LOCAL_5000_4998
MIO_G5_LOCAL_5000_4999
MIO_G5_LOCAL_5000_5000
MIO_G5_LOCAL_5000_END
```

task report：

```text
task_id=BSNI47JLDF3YSUZUDJLEOG6YLQ
kind=sh_attach
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=local
session=mio-realtest-g5-local-5000
```

daemon log 关键证据：

```text
pocacceptor sh_attach ready: task_id=BSNI47JLDF3YSUZUDJLEOG6YLQ target=local session=mio-realtest-g5-local-5000
shell websocket attached
sh_attach bridge first remote stream data
pocacceptor sh_attach first visitor data
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=BSNI47JLDF3YSUZUDJLEOG6YLQ kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `local` 5000 行输出未死锁，尾部 marker 到达，`exit` 后 task 正常收尾。
- 本组覆盖 Windows CLI 发起端 + Linux PTY 被控端路径，也再次暴露 Windows CLI 的 cursor-position query 需要测试驱动响应。

## 23. G5 初始 winsize / resize 观测

目的：

- 验证发起端 PTY 初始尺寸能经 LocalAPI WebSocket 传到远端 shell backend。
- 本轮工具无法直接对已运行 PTY 发送真实 SIGWINCH 动态 resize，因此先用 `stty rows <r> cols <c>` 控制发起端 CLI 启动时尺寸，观察远端 `stty size` 和 daemon winsize 日志。

方法：

```text
stty rows <rows> cols <cols>; miopunch-linux ... sh <windows-peer> wsl:Debian -s <session>
```

远端交互输入：

```text
printf '<START>\n'; stty size; printf '<END>\n'
exit
```

### 23.1 `100x30`

命令：

```text
stty rows 30 cols 100; \
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-resize-01-wsl-initial-100x30.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-resize-100x30
```

观察输出：

```text
MIO_G5_RESIZE_WSL_100x30_START
29 100
MIO_G5_RESIZE_WSL_100x30_END
```

task report：

```text
task_id=HFRAM5DPNJKKAA2F2OS6CBXYSM
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
target=wsl:Debian
session=mio-realtest-g5-wsl-resize-100x30
```

daemon log：

```text
sh_attach bridge first local websocket winsize: task_id=HFRAM5DPNJKKAA2F2OS6CBXYSM size=100x30
pocacceptor sh_attach first visitor json: task_id=HFRAM5DPNJKKAA2F2OS6CBXYSM op=winsize
conpty resize done: pid=84528 size=100x30 err=<nil>
task done: task_id=HFRAM5DPNJKKAA2F2OS6CBXYSM kind=sh_attach reason_code=OK exit_code=0
```

### 23.2 `120x40`

命令：

```text
stty rows 40 cols 120; \
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-resize-02-wsl-initial-120x40.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-resize-120x40
```

观察输出：

```text
MIO_G5_RESIZE_WSL_120x40_START
39 120
MIO_G5_RESIZE_WSL_120x40_END
```

task report：

```text
task_id=V2IFOW5UIMF4LX62QLCLGMTAIU
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
target=wsl:Debian
session=mio-realtest-g5-wsl-resize-120x40
```

daemon log：

```text
sh_attach bridge first local websocket winsize: task_id=V2IFOW5UIMF4LX62QLCLGMTAIU size=120x40
pocacceptor sh_attach first visitor json: task_id=V2IFOW5UIMF4LX62QLCLGMTAIU op=winsize
conpty resize done: pid=82392 size=120x40 err=<nil>
task done: task_id=V2IFOW5UIMF4LX62QLCLGMTAIU kind=sh_attach reason_code=OK exit_code=0
```

### 23.3 `80x24`

命令：

```text
stty rows 24 cols 80; \
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-resize-03-wsl-initial-80x24.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-resize-80x24
```

观察输出：

```text
MIO_G5_RESIZE_WSL_80x24_START
23 80
MIO_G5_RESIZE_WSL_80x24_END
```

task report：

```text
task_id=6COAGHWH3AIUQGETFY2C5J5HDU
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
target=wsl:Debian
session=mio-realtest-g5-wsl-resize-80x24
```

daemon log：

```text
sh_attach bridge first local websocket winsize: task_id=6COAGHWH3AIUQGETFY2C5J5HDU size=80x24
pocacceptor sh_attach first visitor json: task_id=6COAGHWH3AIUQGETFY2C5J5HDU op=winsize
conpty resize done: pid=78500 size=80x24 err=<nil>
task done: task_id=6COAGHWH3AIUQGETFY2C5J5HDU kind=sh_attach reason_code=OK exit_code=0
```

结论：

- 发起端初始 winsize 能通过 LocalAPI WebSocket 进入远端 acceptor，并触发 Windows ConPTY resize。
- 远端 `stty size` 的列数与设置值一致。
- 远端 `stty size` 的行数比设置值少 1，是因为命令运行在 tmux pane 中，tmux status line 占用一行；这不是 resize 传输失败。
- 本组未覆盖运行中动态 SIGWINCH；需要支持 PTY resize 的测试驱动或脚本后再补。

## 24. G5 Tab 与方向键控制序列

目的：

- 在真实 `wsl:Debian` interactive shell 中验证 Tab 补全和 Up-arrow 历史回放。
- 这补齐了前面已覆盖的 backspace 与 Ctrl-C 之外的常见输入控制序列。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-control-01-wsl-tab-arrow.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-tab-arrow
```

交互输入：

```text
printf 'MIO_G5_TAB_OK\n' > /tmp/mio_g5_tab_completion_target
cat /tmp/mio_g5_tab_completion_t<TAB>
printf 'MIO_G5_ARROW_ONCE\n'
<UP>
exit
```

观察输出：

```text
cat /tmp/mio_g5_tab_completion_target
MIO_G5_TAB_OK
printf 'MIO_G5_ARROW_ONCE\n'
MIO_G5_ARROW_ONCE
printf 'MIO_G5_ARROW_ONCE\n'
MIO_G5_ARROW_ONCE
```

task report：

```text
task_id=YTUR65RP37656LHZO7GKZQM7IY
kind=sh_attach
reason_code=OK
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
target=wsl:Debian
session=mio-realtest-g5-wsl-tab-arrow
```

daemon log 关键证据：

```text
shell websocket attached
sh_attach bridge first local websocket data: task_id=YTUR65RP37656LHZO7GKZQM7IY bytes=133
sh_attach bridge first remote stream data write: task_id=YTUR65RP37656LHZO7GKZQM7IY bytes=133
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=YTUR65RP37656LHZO7GKZQM7IY kind=sh_attach reason_code=OK exit_code=0
```

结论：

- Tab 字节能穿过 CLI / WebSocket / dataplane / ConPTY / WSL / tmux / shell，完成唯一文件名前缀补全。
- Up-arrow 字节能触发 shell 历史回放，回车后命令再次执行。
- session 正常 `exit` 收尾。

## 25. G6 daemon 重启后的 fresh shell

目的：

- 单独验证 daemon 重启后首次 shell attach 不复用旧内存 session。
- 这与 21 节的相反 transport reuse 组合不同，本节只验证重启清空 session cache 后的 fresh 行为。

环境操作：

```text
kill 2219354 2219465
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  up --state_path /tmp/miopunch-wsl2-windows-shell/run/linux/state.json
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  up --state_path 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\run\windows\state.json'
```

重启后 gate：

```text
Linux LocalAPI ls: reason_code=OK peer_count=1
Windows LocalAPI ls: reason_code=OK peer_count=1
Linux daemon PID: 2227351
Windows daemon PID: 2227352
```

shell 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g6-fresh-01-after-daemon-restart-wsl-shell.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g6-fresh-after-restart
```

交互输入：

```text
printf 'MIO_G6_FRESH_AFTER_RESTART_OK\n'
exit
```

观察输出：

```text
MIO_G6_FRESH_AFTER_RESTART_OK
```

task report：

```text
task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA
kind=sh_attach
reason_code=OK
stage=SessionAttach
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g6-fresh-after-restart
```

timeline 关键事实：

```text
CandidateExchange: gather candidates
CandidateExchange: mqtt exchange
PunchAttempt: punch attempt
DataplaneHandshake: data plane dial stream
CapabilityHandshake: hello handshake
SessionAttach: shell websocket attached
```

daemon log：

```text
task fact: task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA message=data_proto=tls
task fact: task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA message=attempt_path=punching_tcp4
task fact: task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA message=path_family=tcp4
task stage: task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA stage=SessionAttach message=shell websocket attached
task done: task_id=ID42G3QKTZ7HR6DOQX6HWFU3BA kind=sh_attach reason_code=OK exit_code=0
```

结论：

- daemon 重启后首次 `sh` 没有出现 `session_reused=true`。
- report 重新经过 candidate exchange、punch attempt 和 dataplane handshake，符合 fresh 行为。
- marker 输出与 `exit` 收尾正常。

## 26. G7 空闲 30 秒后继续输入

目的：

- 验证 shell attach 后短时间空闲不会断链。
- 本组在远端 prompt 出现后不输入，等待约 35 秒，再发送 marker 和 `exit`。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-idle-01-wsl-30s.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-wsl-idle-30s
```

执行事实：

```text
task_id=25B72VRMDOECL3T4Z34WQ2SZPE
shell prompt visible at about 2026-05-15 12:29:55 Asia/Taipei
first user marker sent at about 2026-05-15 12:30:41 Asia/Taipei
```

交互输入：

```text
printf 'MIO_G7_IDLE_30S_OK\n'
exit
```

观察输出：

```text
MIO_G7_IDLE_30S_OK
```

task report：

```text
task_id=25B72VRMDOECL3T4Z34WQ2SZPE
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g7-wsl-idle-30s
```

daemon log：

```text
shell websocket attached
sh_attach bridge first local websocket data write: task_id=25B72VRMDOECL3T4Z34WQ2SZPE bytes=16
sh_attach bridge first local websocket data: task_id=25B72VRMDOECL3T4Z34WQ2SZPE bytes=35
sh_attach bridge remote shell exited: task_id=25B72VRMDOECL3T4Z34WQ2SZPE
task done: task_id=25B72VRMDOECL3T4Z34WQ2SZPE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- shell attach 后约 35 秒空闲未断链。
- 空闲后输入 marker 正常回显，`exit` 后 task 正常收尾。

## 27. G7 空闲 2 分钟后继续输入

目的：

- 验证 shell attach 后超过 2 分钟空闲仍可继续输入。
- 本组在远端 prompt 出现后不输入，等待约 125 秒，再发送 marker 和 `exit`。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-idle-02-wsl-2m.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-wsl-idle-2m
```

执行事实：

```text
task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E
shell websocket attached at 2026-05-15 12:31:20 Asia/Taipei
first user marker observed in daemon log at 2026-05-15 12:33:29 Asia/Taipei
idle before user input: about 129 seconds
```

交互输入：

```text
printf 'MIO_G7_IDLE_2M_OK\n'
exit
```

观察输出：

```text
MIO_G7_IDLE_2M_OK
```

task report：

```text
task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g7-wsl-idle-2m
```

daemon log：

```text
shell websocket attached
sh_attach bridge first local websocket data write: task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E bytes=16
sh_attach bridge first local websocket data: task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E bytes=34
sh_attach bridge remote shell exited: task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E
task done: task_id=5CCBYPDEQ7RGSY3SVMCJ3OL35E kind=sh_attach reason_code=OK exit_code=0
```

结论：

- shell attach 后约 129 秒空闲未断链。
- 空闲后输入 marker 正常回显，`exit` 后 task 正常收尾。

## 28. G7 活动 shell 期间并发 `ping` / `sh ls`

目的：

- 验证同一 peer 上已有 interactive shell 连接时，并发执行 `ping` 和 `sh ls` 不破坏 shell。
- 同时确认 `sh ls` 能观测到当前 tmux session attached 状态。

### 28.1 保持 shell 已连接

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-concurrent-01-wsl-shell.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-wsl-concurrent
```

shell report：

```text
task_id=S24YWCW22Q4JIEIRM4UHC2XJA4
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
target=wsl:Debian
session=mio-realtest-g7-wsl-concurrent
```

### 28.2 shell 活动期间并发 `ping`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-concurrent-02-ping-while-shell.md \
  ping CSF7LBLQZ5WGRM7DMJZ6YJ3AYI
```

report：

```text
task_id=RI5LRGIDZYJ43EJBTFJUH4CFTY
kind=ping
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
ping=ok
```

### 28.3 shell 活动期间并发 `sh ls`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-concurrent-03-sh-ls-while-shell.md \
  sh ls CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian
```

report：

```text
task_id=WPFE74YTH45YDJVZOQ6ZT2GMVA
kind=sh_ls
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
session=mio-realtest-g7-wsl-concurrent: 1 windows (created Fri May 15 12:34:11 2026) (attached)
```

### 28.4 并发 task 后 shell 继续输入

交互输入：

```text
printf 'MIO_G7_CONCURRENT_SHELL_STILL_OK\n'
exit
```

观察输出：

```text
MIO_G7_CONCURRENT_SHELL_STILL_OK
```

daemon log：

```text
task done: task_id=RI5LRGIDZYJ43EJBTFJUH4CFTY kind=ping reason_code=OK exit_code=0
task fact: task_id=WPFE74YTH45YDJVZOQ6ZT2GMVA message=session=mio-realtest-g7-wsl-concurrent: 1 windows (created Fri May 15 12:34:11 2026) (attached)
task done: task_id=WPFE74YTH45YDJVZOQ6ZT2GMVA kind=sh_ls reason_code=OK exit_code=0
sh_attach bridge first local websocket data: task_id=S24YWCW22Q4JIEIRM4UHC2XJA4 bytes=49
task done: task_id=S24YWCW22Q4JIEIRM4UHC2XJA4 kind=sh_attach reason_code=OK exit_code=0
```

结论：

- 活动 shell 期间并发 `ping` 成功。
- 活动 shell 期间并发 `sh ls` 成功，并且能看到当前 session 是 `(attached)`。
- 并发 task 完成后，原 shell 仍可输入 marker 并正常 `exit`。

## 29. G5 1MiB stdout smoke

目的：

- 在真实 interactive shell 中输出 1MiB 连续 stdout，确认不会死锁、提前关闭或丢失尾部 marker。
- 本组不是逐字节完整性校验；终端屏幕会滚动/重绘，证据标准是尾部 marker 可见、report OK、daemon 正常收尾。

### 29.1 Linux -> Windows `wsl:Debian`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g5-pressure-03-wsl-1m-output.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g5-wsl-1m
```

交互输入：

```text
printf 'MIO_G5_WSL_1M_START\n'; head -c 1048576 /dev/zero | tr '\0' A; printf '\nMIO_G5_WSL_1M_END\n'
exit
```

观察输出：

```text
MIO_G5_WSL_1M_END
```

task report：

```text
task_id=5PCIEMYX4ZULWQTLWUM3GJL4HQ
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g5-wsl-1m
```

daemon log：

```text
shell websocket attached
sh_attach bridge first local websocket data: task_id=5PCIEMYX4ZULWQTLWUM3GJL4HQ bytes=107
pocacceptor sh_attach first visitor data: task_id=5PCIEMYX4ZULWQTLWUM3GJL4HQ bytes=107
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=5PCIEMYX4ZULWQTLWUM3GJL4HQ kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `wsl:Debian` 1MiB stdout smoke 通过，尾部 marker 到达，`exit` 后 task 正常收尾。

### 29.2 Windows -> Linux `local`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g5-pressure-04-local-1m-output.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-g5-local-1m
```

执行事实：

- Windows CLI 启动后先输出 `ESC[6n`。
- 回传 `ESC[1;1R` 后进入远端 `local` prompt。

交互输入：

```text
printf 'MIO_G5_LOCAL_1M_START\n'; head -c 1048576 /dev/zero | tr '\0' B; printf '\nMIO_G5_LOCAL_1M_END\n'
exit
```

观察输出：

```text
MIO_G5_LOCAL_1M_END
```

task report：

```text
task_id=ZCE752ZEHNXR2M7N7H4VUDTIVE
kind=sh_attach
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=local
session=mio-realtest-g5-local-1m
```

daemon log：

```text
shell websocket attached
sh_attach bridge first local websocket data: task_id=ZCE752ZEHNXR2M7N7H4VUDTIVE bytes=31
pocacceptor sh_attach first visitor data: task_id=ZCE752ZEHNXR2M7N7H4VUDTIVE bytes=31
pocacceptor sh_attach backend exited
sh_attach bridge remote shell exited
task done: task_id=ZCE752ZEHNXR2M7N7H4VUDTIVE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- `local` 1MiB stdout smoke 通过，尾部 marker 到达，`exit` 后 task 正常收尾。

## 30. G7 活动 shell 期间并发 `maintain-neighbors`

目的：

- 验证已有 interactive shell 连接时，并发执行 `maintain-neighbors -u` / `maintain-neighbors -t` 不破坏 shell。
- 本轮重点是并发安全和已连接 shell 的可继续输入；如果已有 active neighbor，`maintain-neighbors` 预期可能跳过实际拨号。

### 30.1 保持 shell 已连接

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-maintain-01-wsl-shell.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-maintain
```

shell report：

```text
task_id=H5TXBSWWY7DRNH2POFNLRYCXIE
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g7-maintain
```

### 30.2 并发 `maintain-neighbors -u`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-maintain-02-udp-while-shell.md \
  maintain-neighbors -u
```

report：

```text
task_id=UU2CZ2ILHN6ZVKFZGBGSFY4IWE
kind=maintain_neighbors
reason_code=OK
maintain_neighbors_selected=1
maintain_neighbors_active_before=1
maintain_neighbors_attempted=0
maintain_neighbors_succeeded=0
maintain_neighbors_failed=0
maintain_neighbors_skipped_active=1
active_neighbors=2
```

### 30.3 并发 `maintain-neighbors -t`

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-maintain-03-tcp-while-shell.md \
  maintain-neighbors -t
```

report：

```text
task_id=EVBI6DG2LV2BBXPZNXUXSKHDAQ
kind=maintain_neighbors
reason_code=OK
maintain_neighbors_selected=1
maintain_neighbors_active_before=1
maintain_neighbors_attempted=0
maintain_neighbors_succeeded=0
maintain_neighbors_failed=0
maintain_neighbors_skipped_active=1
active_neighbors=2
```

### 30.4 并发 task 后 shell 继续输入

交互输入：

```text
printf 'MIO_G7_MAINTAIN_SHELL_STILL_OK\n'
exit
```

观察输出：

```text
MIO_G7_MAINTAIN_SHELL_STILL_OK
```

daemon log：

```text
task done: task_id=UU2CZ2ILHN6ZVKFZGBGSFY4IWE kind=maintain_neighbors reason_code=OK exit_code=0
task done: task_id=EVBI6DG2LV2BBXPZNXUXSKHDAQ kind=maintain_neighbors reason_code=OK exit_code=0
sh_attach bridge first local websocket data: task_id=H5TXBSWWY7DRNH2POFNLRYCXIE bytes=47
task done: task_id=H5TXBSWWY7DRNH2POFNLRYCXIE kind=sh_attach reason_code=OK exit_code=0
```

结论：

- 活动 shell 期间 `maintain-neighbors -u` / `-t` 都返回 OK。
- 两个 maintain task 都因已有 active neighbor 而 `maintain_neighbors_skipped_active=1`，没有实际新拨号；这是当前环境下的预期事实。
- maintain task 完成后，原 shell 仍可输入 marker 并正常 `exit`。

## 31. G7 被控端 tmux pane 主动退出

目的：

- 验证远端 tmux pane 主动消失时，发起端 shell task 能收尾且 CLI 不挂死。
- 本组不使用普通 `exit`，而是在远端 shell 内执行 `tmux kill-pane -t "$TMUX_PANE"`。

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-pane-exit-01-wsl-kill-pane.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-pane-exit
```

交互输入：

```text
printf 'MIO_G7_PANE_EXIT_BEFORE_KILL\n'; tmux kill-pane -t "$TMUX_PANE"
```

观察输出：

```text
MIO_G7_PANE_EXIT_BEFORE_KILL
```

task report：

```text
task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g7-pane-exit
```

daemon log：

```text
shell websocket attached
sh_attach bridge first local websocket data: task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y bytes=72
pocacceptor sh_attach backend exited: task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y
sh_attach bridge first remote stream json: task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y bytes=29 op=shell_exit ok=true
sh_attach bridge remote shell exited: task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y
task done: task_id=NAFFASKANP7FKHQ3H6FRMZQU3Y kind=sh_attach reason_code=OK exit_code=0
```

结论：

- 远端 tmux pane 主动 `kill-pane` 后，发起端收到 `shell_exit ok=true`。
- task 正常 `reason_code=OK exit_code=0`，CLI 未挂死。

## 32. G7 被控端 daemon 退出时 shell CLI 收尾

目的：

- 验证 shell attach 已连接后，被控端 daemon 正常收到 `SIGTERM` 退出时，发起端 CLI 不挂死，并给出可解释原因。
- 为保留发起端 report，本组杀被控端 Windows daemon，不杀发起端 Linux daemon。

precondition：

```text
Linux daemon PID: 2227351
Windows daemon PID before test: 2227352
双端 LocalAPI gate: reason_code=OK peer_count=1
```

shell 命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-wsl2-windows-shell/run/linux/localapi.sock \
  --report /tmp/miopunch-wsl2-windows-shell/reports/g7-daemon-exit-01-wsl-remote-daemon-term.md \
  sh CSF7LBLQZ5WGRM7DMJZ6YJ3AYI wsl:Debian -s mio-realtest-g7-remote-daemon-term
```

确认 shell attached 后执行：

```text
kill 2227352
```

CLI 输出摘要：

```text
stage=SessionAttach
reason_code=SH_CONNECTOR_FAIL
exit_code=3
shell_layer=acceptor
shell_close=read shell stream: EOF
suggestions:
- retry
```

task report：

```text
task_id=TPCAKIMAA4GTZ2TXRZT4SODWAA
kind=sh_attach
status=done
stage=SessionAttach
reason_code=SH_CONNECTOR_FAIL
exit_code=3
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=wsl:Debian
session=mio-realtest-g7-remote-daemon-term
shell_layer=acceptor
shell_close=read shell stream: EOF
```

恢复操作：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  up --state_path 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\run\windows\state.json'
```

恢复后 gate：

```text
Linux LocalAPI ls: reason_code=OK peer_count=1
Windows LocalAPI ls: reason_code=OK peer_count=1
Windows daemon PID after restart: 2239101
```

结论：

- 被控端 Windows daemon 退出后，发起端 shell CLI 未挂死。
- 当前收尾语义是 `SH_CONNECTOR_FAIL`，并明确归因到 `shell_layer=acceptor`、`shell_close=read shell stream: EOF`。
- 这是可解释失败，不是 signaling / candidate / punching / hello 问题。

## 33. G7 Windows -> Linux `local` idle

目的：

- 补齐 Windows CLI 发起端、Linux `local` 被控端方向的 idle 生命周期验证。
- 本组覆盖 30 秒 idle 与 2 分钟 idle；Windows CLI 的 cursor-position query 仍需测试驱动回传 `ESC[1;1R`。

### 33.1 30 秒 idle

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g7-idle-03-local-30s.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-g7-local-idle-30s
```

执行事实：

```text
Windows CLI emitted ESC[6n
test driver replied ESC[1;1R
remote prompt visible at about 2026-05-15 12:43:47 Asia/Taipei
marker sent after about 30 seconds idle
```

交互输入：

```text
printf 'MIO_G7_LOCAL_IDLE_30S_OK\n'
exit
```

观察输出：

```text
MIO_G7_LOCAL_IDLE_30S_OK
```

task report：

```text
task_id=GMIY4JYJL627E3Y5KYHESKT6HI
kind=sh_attach
reason_code=OK
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
target=local
session=mio-realtest-g7-local-idle-30s
```

daemon log：

```text
shell websocket attached
sh_attach bridge remote shell exited: task_id=GMIY4JYJL627E3Y5KYHESKT6HI
task done: task_id=GMIY4JYJL627E3Y5KYHESKT6HI kind=sh_attach reason_code=OK exit_code=0
```

### 33.2 2 分钟 idle

命令：

```text
/tmp/miopunch-wsl2-windows-shell/bin/miopunch.exe \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\reports\g7-idle-04-local-2m.md' \
  sh NSW2B4OYFY4NRRKFENKRWI6XKE local -s mio-realtest-g7-local-idle-2m
```

执行事实：

```text
Windows CLI emitted ESC[6n
test driver replied ESC[1;1R
remote prompt visible at about 2026-05-15 12:44:44 Asia/Taipei
marker sent after about 125 seconds idle
```

交互输入：

```text
printf 'MIO_G7_LOCAL_IDLE_2M_OK\n'
exit
```

观察输出：

```text
MIO_G7_LOCAL_IDLE_2M_OK
```

task report：

```text
task_id=LIJE6N5H4ODBHW6PWFURZLK2LI
kind=sh_attach
reason_code=OK
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
target=local
session=mio-realtest-g7-local-idle-2m
```

daemon log：

```text
shell websocket attached
sh_attach bridge remote shell exited: task_id=LIJE6N5H4ODBHW6PWFURZLK2LI
task done: task_id=LIJE6N5H4ODBHW6PWFURZLK2LI kind=sh_attach reason_code=OK exit_code=0
```

结论：

- Windows -> Linux `local` 方向 30 秒 idle 后仍可输入 marker 并正常 `exit`。
- Windows -> Linux `local` 方向约 2 分钟 idle 后仍可输入 marker 并正常 `exit`。
- Windows CLI 的 `ESC[6n` 仍是测试驱动必须处理的前置事实，不是 shell 失败。

## 34. 当前收口结论

截至 2026-05-15 12:47 Asia/Taipei，当前这批 shell 真机测试中“现在能验证、且不需要新增专门动态 resize 驱动”的项目已经完成。

已完成覆盖：

- G0 / G1 / G2：隔离 daemon、membership、双向 UDP/TCP connectivity gate。
- G3：target / session discovery，含空 target、transport variants、`local`、`wsl:Debian`、`ssh:ale`。
- G4：`ssh:ale`、`local`、`wsl:Debian` interactive shell 基础闭环及 `-u` / `-t`。
- G5：1000 行 smoke、5000 行压力、1MiB stdout smoke、Ctrl-C、backspace、Tab、Up-arrow、初始 winsize 传递。
- G6：detach / reconnect / exit cleanup、相反 transport reuse、daemon restart 后 fresh shell。
- G7：30 秒 idle、2 分钟 idle、活动 shell 期间并发 `ping` / `sh ls` / `maintain-neighbors`、tmux pane 主动退出、被控端 daemon 退出时 CLI 可解释收尾、Windows -> Linux `local` idle。

明确未覆盖：

- 运行中动态 SIGWINCH resize：当前手动 PTY 工具不能稳定对已运行 CLI 发送 resize/SIGWINCH。已覆盖初始 winsize 传递和 Windows ConPTY resize 成功；动态 resize 需要后续新增临时 expect/python PTY 驱动或正式自动化脚本。
- 逐字节完整性校验：5000 行与 1MiB 测试以尾部 marker + report + daemon log 为证据，不是完整捕获和 hash 校验。若要做严格校验，需要专门测试驱动捕获完整 PTY 输出或通过协议层 instrumentation 统计 payload。

当前环境状态：

```text
Linux LocalAPI ls: reason_code=OK peer_count=1
Windows LocalAPI ls: reason_code=OK peer_count=1
Linux daemon PID: 2227351
Windows daemon PID: 2239101
```

结论：

- 当前 shell 真机回归的主要功能、生命周期、transport 约束和失败口径已覆盖到可以收口。
- 剩余项属于自动化能力增强，不阻塞本批真实环境验证结论。

## 35. 已识别问题归因汇总

本节只汇总本轮真实运行中已经定位过原因的问题。未把“尚未覆盖”的自动化能力缺口混入产品缺陷；动态 SIGWINCH 与逐字节 hash 校验仍归类为后续测试能力增强。

### 35.1 Windows 旧进程存在但 LocalAPI 不可用

- 现象：Windows 侧能看到 `miopunch.exe` 进程，但 CLI `ls` 返回 `reason_code=DAEMON_NOT_RUNNING`。
- 原因：进程存在不等价于当前用户/当前配置的 LocalAPI 可用；旧进程没有提供本轮测试期望的 LocalAPI gate。
- 结论：这是 G0 daemon / LocalAPI 前置失败，不是 shell、punching 或 dataplane 问题。后续以 `ls --format json reason_code=OK` 作为 daemon 可用性事实。

### 35.2 旧 extracted binary 能力落后

- 现象：旧 bundle 执行 `init-network` 返回 `reason_code=UNKNOWN_COMMAND`。
- 原因：`dist/extracted/...git78ff8d5...` 二进制早于当前源码能力，缺少当前测试需要的命令。
- 结论：这是测试二进制版本问题，不是网络或 shell 问题。本轮后续切换为从当前源码构建的 Linux/Windows 二进制。

### 35.3 invite broker hostname 被改写成 IP 导致 join 失败

- 现象：Linux invite 写入 `invite_brokers=35.172.255.228:1883`，Windows join 连接该 IP 返回 `reason_code=UNAVAILABLE` / `mqtt connect failed ... future canceled`。
- 原因：`selectReachableInviteSubset` 在可达性探测后把 hostname broker canonicalize 成 A 记录 IP 写入 invite code；join 端只按 invite code 中的 IP 连接，无法回退 `broker.emqx.io:1883`。
- 结论：这是 invite broker 选择 / canonicalization 问题，不是 shell、punching 或 governance 问题。临时修复为保留 hostname 写入 invite code，已被 Windows join 真实验证。

### 35.4 WSL target discovery 解码错误

- 现象：`sh ls` 枚举 Windows target 时出现 `target=wsl:\u0000...`，`wsl:Debian` / `wsl:Ubuntu` 被 NUL 污染。
- 原因：Windows `wsl.exe -l -q` 输出为 UTF-16LE；旧逻辑按 UTF-8 文本解析，导致每个 ASCII 字符后带 NUL。
- 结论：这是 Windows target discovery 文本解码 bug，不是 shell transport、punching 或 tmux 问题。临时修复 `decodeWindowsCommandOutput` 后，真实枚举恢复为 `target=wsl:Debian` / `target=wsl:Ubuntu`。

### 35.5 tmux 缺失被误归类为 connector 失败

- 现象：`tmux list-sessions` 返回 `zsh:1: command not found: tmux` 时，任务曾返回 `reason_code=SH_CONNECTOR_FAIL`。
- 原因：tmux 缺失文本没有被统一识别为 `ErrTmuxMissing`，上层只能看到 connector 命令失败。
- 结论：这是 shell target 依赖缺失的错误分类问题。临时修复后收敛为 `reason_code=SH_TMUX_MISSING`，并按 `local` / `wsl:<distro>` / `ssh:<host>` 给出安装 tmux 的建议；其它 `ssh:*` host 缺 tmux 时不应被判定为连不上。

### 35.6 空 tmux server 与 WSL `-F "#S"` 参数兼容问题

- 现象 1：没有 tmux server 时，`tmux list-sessions` 返回 `error connecting to /tmp/tmux-1000/default (No such file or directory)` 并导致 discovery 失败。
- 现象 2：Windows 通过 `wsl.exe -- tmux list-sessions -F "#S"` 时，`#S` 被调用链吃掉或改写，tmux 看到 `-F` 但缺少 format 参数。
- 原因：tmux 3.1c 的 no-server 输出需要被当成空 session 列表；WSL 调用链对 `#S` 参数传递不稳定。
- 结论：这是 session discovery 兼容性问题。临时修复为识别 no-server 文本并在 Windows `wsl:<distro>` discovery 路径避免传 `-F "#S"`；真实 `local` / `wsl:Debian` 空 session discovery 已返回 `reason_code=OK`。

### 35.7 shell 正常退出缺少协议级完成信号

- 现象：`ssh:ale` attach 输入输出正常，远端 `exit` 后本地 CLI 没有自然收尾，最终因手动 kill 变成 `reason_code=UNAVAILABLE` / `localapi_ws 1006`。
- 原因：被控端 `ptySess.Wait()` 正常返回 nil 时，旧协议没有向发起端发送“远端 shell 已正常结束”的控制信号；发起端 bridge 继续等待。
- 结论：这是 shell attach 生命周期协议缺口。临时增加 `shell_exit ok=true` 控制帧后，`ssh:ale`、`local`、`wsl:Debian` 的 `exit`、detach、pane kill 均能正常收尾。

### 35.8 Linux PTY 正常退出后的 EIO 被误判为失败

- 现象：Windows -> Linux `local` attach 中，远端输出已出现，但 `exit` 后 report 为 `reason_code=SH_CONNECTOR_FAIL`，`shell_layer=pty`，`read /dev/ptmx: input/output error`。
- 原因：Linux PTY 在 child shell 正常退出后可能从 `/dev/ptmx` read 返回 EIO；旧逻辑把这个 read close 直接当 runtime failure，覆盖了后续 `Wait()` 的正常退出结果。
- 结论：这是 Unix PTY close 语义处理问题。临时修复为遇到 expected PTY read close 时等待 backend `Wait()` 决定最终结果；复测后 `local` attach 正常 `reason_code=OK`。

### 35.9 UDP-only 个别失败属于连接路径阶段，不是 shell 阶段

- 现象：个别 `-u` case 首次返回 `reason_code=UNAVAILABLE`，例如 `wsl:Debian -u` 首次失败在 UDP punching，Windows -> Linux empty target `-u` 首次失败在 CandidateExchange。
- 原因：失败发生在 candidate / punching 阶段，尚未进入 dataplane hello 或 shell discovery / attach。
- 结论：这些不能归类为 ConPTY、WSL、tmux、PTY 或 shell lifecycle 问题；相同方向 TCP 通过，后续 UDP retry 也通过，当前作为真实网络路径的瞬时失败事实记录。

### 35.10 Windows CLI 的 `ESC[6n` 是测试驱动前置事实

- 现象：Windows CLI 发起 shell 时会先输出 cursor position query `ESC[6n`，测试驱动需要回传 `ESC[1;1R` 后才能继续进入远端 prompt。
- 原因：终端交互初始化需要查询光标位置；当前人工/PTY 测试驱动不会自动响应该控制序列。
- 结论：这不是 shell 失败。后续自动化测试驱动需要显式处理 `ESC[6n`，否则可能把测试驱动卡住误判为 shell 卡住。
