# MIOPUNCH-156 原生双端争用及冷却基线

冻结版本为 `694f004d562d6d68548d44f0dafc5c2f784b89c9`。在一次性 Linux 网络命名空间运行两个正式 `NativeNetwork` 节点，完成真实成员准入，通过获准的本地 iroh relay 拨号、接收连接和开流。relay 的每条 TCP 腿延迟 35 毫秒，双端首拨一道栅栏只控制同时发起的时序。两边在同一条打开的业务流上持续写入并独立校验彼此的 64 位序号。每节点有两个独立 STUN 监听地址，直接统计收到的请求。沿用 139 的增强-only 夹具和原连接 relay 路径约束，可以观察真实增强探测，同时保留健康原流作对照。未修改冻结版运行源码。

| 操作 | A 端 | B 端 | 原业务与后续结果 |
| --- | --- | --- | --- |
| 同一原连接，双方自动维护，Hello 交接 | 原业务拨号方的出站 Hello 获得执行机会，增强名额在 4,291 ms 预留并开始 8,000 ms 冷却。2 个样本，实际发包 1 个。 | 原接收方的出站 Hello 取消，未进入增强预算、未采样或发探测包。4,215 ms 交接给获准入站控制。同一获胜轮次在 4,215 ms 预留并开始冷却，2 个样本，实际发包 5 个。 | 一条原流从双向序号 1 到 36。已记录的 140 Hello 正例另在同一快照跑通，完成普通候选验证和原流后续字节。 |
| 两条独立原连接，同时从各自的原拨号端启动公开 repair | A 的出站是第 1 条连接。A 的唯一增强名额被第 2 条连接的入站轮次在 5,750 ms 占用，启动 8,000 ms 冷却。第 1 条轮次本端被拒。A 的实际 STUN 和探测包均为 0。 | B 的出站是第 2 条连接。B 的唯一增强名额被第 1 条连接的入站轮次在 5,729 ms 占用，启动 8,000 ms 冷却。第 2 条轮次本端被拒。B 的实际 STUN 和探测包均为 0。 | 两条受保护尝试分别达到了两侧，一个持有者和一个缺额方。两侧持有者均因对端不可用而在取样前退场，两次 repair 均返回可重试的本地预算暂缓。两条原流的双向序号从 1 到 16。 |
| 零工作后紧接着对第 1 条原连接做一次单端 repair | 已无占用，仍因前轮冷却等待 6,559 ms，没采样或发探测包。 | 已无占用，仍因前轮冷却等待 6,612 ms，没采样或发探测包。 | 旧版红灯。两条原流均继续到 30。原操作的第二次独立复跑剩余冷却分别为 6,514 / 6,607 ms，两个持有者同样都在零工作后等待。 |
| 等旧冷却结束，对第 1 条原连接单端重复同一操作 | 2 个样本，实际发包 1 个，收到匹配回包。紧接的重试剩 5,027 ms 冷却。 | 2 个样本，实际发包 5 个，收到匹配探测。紧接的重试剩 5,026 ms 冷却。 | 同一增强轮次，每端实际预留的方案包数上限为 30。公开 repair 在 3,000 ms 结算，夹具持续限定原业务走 relay，返回可重试的期限耗尽。原连接、原流未中断，两条流均继续到 189。冷却保留有效。 |

每次运行在同一进程使用单调诊断时钟，表中时间只能在同一次运行内部比较。两端的 `stableId` 和本地尝试编号只标识各自实例。通过 Hello 完成后的共享 `attemptId` 配对，通过增强 `round` 配对真实发包。两端保留的编号、冷却起点、发送时间、分段业务序号和结果见 [首轮结构化见证](native-iroh-path-contention-156.json)。采样也由实际 STUN 观察端的收包独立确认。包数来自成功的 UDP 提交计数，包含探测与应答。

旧版每连接 Hello 名额和节点增强名额分别生效。旧节点 ledger 在预留 `RoundPermit` 时设定冷却到期时间，许可析构释放 holder。尚未进入采样的两个持有者退场后，节点仍按预留时设下的 32 × 250 ms 结算。用原 deadline、真实发包验证的对照也会消耗该预算。现有 `native_upgrade_reserves_qad_and_both_stun_observers_on_each_eligible_round` 与 `timed_out_enhanced_plan_does_not_reuse_its_allowance_for_round_two` 另行通过。后者的观测套接字收到原轮探测，诊断记录发包 23 个，第二轮返回预算暂缓。[既有 140 交接正例](network-demo-140-2026-11-18.md#混合观察与-hello-仲裁)

## 复跑

下列 Bash 命令从包含本夹具的仓库根目录执行。临时目录取冻结提交内容，只叠加测试模块、复用的 relay 迟滞夹具和测试模块入口。`CheckBuilds` 在共享 Cargo 锁内编译并固定三个选中的测试产物，再运行这些固定产物。日志、回执和行为见证写入最后输出的目录。

```bash
set -euo pipefail
repo=$PWD
baseline=$(rtk proxy mktemp -d /tmp/miopunch-156-694f004d-XXXXXX)
run=$(rtk proxy mktemp -d /tmp/miopunch-156-run-XXXXXX)
rtk proxy git archive 694f004d | rtk proxy tar -x -C "$baseline"
rtk proxy ln -s "$repo/target" "$baseline/target"
rtk proxy mkdir -p "$repo/.artifacts"
rtk proxy ln -s "$repo/.artifacts" "$baseline/.artifacts"
rtk proxy cp tests/support/path_contention_baseline.rs tests/support/delayed_relay.rs "$baseline/tests/support/"
rtk proxy tee -a "$baseline/tests/network_maintenance.rs" >/dev/null <<'RUST'

#[cfg(all(target_os = "linux", feature = "nat-lab-test-hooks"))]
#[path = "support/path_contention_baseline.rs"]
mod path_contention_baseline;
RUST

(
  cd "$baseline"
  rtk proxy python3 - "$run" <<'PY'
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path.cwd() / "scripts"))
from check_build_cache import CheckBuilds

output = Path(sys.argv[1])
builds = CheckBuilds(output / "receipts")
with (output / "build.log").open("w") as log:
    builds.run([
        "rtk", "proxy", "cargo", "test", "--locked", "--offline",
        "-p", "miopunch-native-probe", "--features", "nat-lab-test-hooks",
        "--lib", "--test", "legacy_nat_runtime", "--test", "network_maintenance",
        "--no-run", "--jobs", "2",
    ], stdout=log, stderr=subprocess.STDOUT, check=True)
pins = {program["name"]: program["pin"] for program in builds.last_programs}
cases = [
    ("network_maintenance", "path_contention_baseline::same_original_hello_hands_off_live_business"),
    ("network_maintenance", "path_contention_baseline::independent_originals_contend_for_node_rounds"),
    ("miopunch_native_probe", "connection_coordinator::active_commit_tests::colliding_active_hello_hands_off_to_the_original_dialer"),
    ("legacy_nat_runtime", "native_upgrade_reserves_qad_and_both_stun_observers_on_each_eligible_round"),
    ("legacy_nat_runtime", "timed_out_enhanced_plan_does_not_reuse_its_allowance_for_round_two"),
]
for target, case in cases:
    namespace = [
        "rtk", "proxy", "unshare", "--user", "--map-root-user", "--net",
        "sh", "-ec", 'ip link set lo up\nexec "$@"', "mio156",
    ]
    enhanced = target == "network_maintenance"
    environment = ["env", "MIOPUNCH_NAT_LAB_ENHANCED_ONLY=1"] if enhanced else []
    flags = ["--exact", "--nocapture", "--ignored"] if enhanced else ["--exact", "--nocapture"]
    name = case.split("::")[-1]
    with (output / (name + ".log")).open("w") as log:
        builds.run([*namespace, *environment, pins[target], case, *flags],
                   stdout=log, stderr=subprocess.STDOUT, timeout=45, check=True)
PY
)

rtk proxy python3 scripts/check_native_path_contention.py \
  "$run/same_original_hello_hands_off_live_business.log" \
  "$run/independent_originals_contend_for_node_rounds.log" --expect-baseline-red >"$run/witness.json"
rtk proxy realpath "$run"
```

无 `--expect-baseline-red` 时，原输入以退出码 1 报告行为断言失败。带该参数时，退出码 0 要求观察到真正重叠的两端名额、零工作且已经释放仍在冷却、冷却后真实采样与发包，以及实质工作后冷却仍生效。夹具未形成完整见证时退出 2。首轮和首次复跑的原始日志在 `/tmp/miopunch-156-fixture/`，旧入口日志在 `/tmp/miopunch-156-existing/`。上面的完整命令另一次产生了 `/tmp/miopunch-156-run-TBzL88/`，包含三个固定测试产物的构建回执、五组通过的宿主测试日志和预期旧红见证。该轮零工作后的余量为 6,556 / 6,611 ms。

## MIOPUNCH-151 共用断言

将同一业务输入、双端同步触发、紧接的单端再请求、250 ms 节点窗口、3 秒 caller 期限和原流序号核对带到融合端点。如两个业务方向复用一条成员连接，保留向两侧各自发出的真实改善请求，再按融合诊断配对。沿用脚本中的 `check_unspent_handoff` 原断言，给它同一上限内的实际采样与发送事件，以及在让出后仍占完整冷却的零工作名额。允许胜出轮次在首次重叠时推进，也允许后续单端请求推进。收到实际样本并提交探测包才算工作，需保留有实质工作后的期限和包数检查。旧版证据要求形成两条不同原连接和两个不同的受保护尝试。融合版按自己的真实配对和名额观察提供同一断言输入。
