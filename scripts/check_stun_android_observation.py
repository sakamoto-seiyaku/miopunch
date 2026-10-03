#!/usr/bin/env python3
"""Narrow physical Android regression for shared same-port STUN/TCP observation."""
import argparse
import copy
import json
import pathlib
import re
import shlex
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", type=pathlib.Path, required=True)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--library", type=pathlib.Path, required=True)
    parser.add_argument("--shell", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    remote = f"/data/local/tmp/miopunch67-{time.time_ns()}"
    receipt = {"scope": "physical Android STUN/TCP observation only; no Android punching", "phases": [], "passed": False}
    owned_pids = []
    created = False

    def adb(*command, **kwargs):
        return subprocess.run(["rtk", "proxy", str(args.adb), *command], capture_output=True, text=True, check=True, timeout=30, **kwargs).stdout

    def shell(command, **kwargs):
        return adb("shell", command, **kwargs)

    def save_config(config):
        path = output / "configuration.json"
        path.write_text(json.dumps(config))
        # This is the documented startup configuration input in a new owned store.
        adb("push", str(path), remote + "/node/resources.json")
        actual = json.loads(shell(shlex.join([remote + "/probe", "network", "resources", remote + "/node"])))
        assert actual == config

    def observe(phase, enabled):
        command = shlex.join(["app_process", "/system/bin", "MioPublicShell", remote + "/libnode.so", remote + "/node", "0.0.0.0"])
        command = "echo $$ > " + shlex.quote(remote + "/session.pid") + "; echo MIOPID $$; exec env CLASSPATH=" + shlex.quote(remote + "/shell.jar") + " " + command
        request = "\n".join(json.dumps({"id": index, "op": op}) for index, op in [(1, "status"), (2, "observe"), (3, "stop")]) + "\n"
        result = shell(command, input=request)
        (output / f"{phase}.log").write_text(result)
        owned_pids.extend(int(pid) for pid in re.findall(r"^MIOPID (\d+)$", result, re.M))
        events = [json.loads(line) for line in result.splitlines() if line.startswith("{")]
        ready = next(event for event in events if event.get("event") == "ready")
        status = next(event["result"] for event in events if event.get("id") == 1)
        observations = next(event["result"]["observations"] for event in events if event.get("id") == 2)
        assert status["node"] == ready["node"]
        assert status["stage"] == "Complete"
        if enabled:
            assert len(observations) == 1
            item = observations[0]
            assert item["protocol"] == "STUN/TCP/RFC8489"
            assert item["fresh"] and item["transaction_and_source_validated"] and item["retained_connection"]
            assert item["local_socket"].rsplit(":", 1)[1] == item["transport_listener"].rsplit(":", 1)[1]
        else:
            assert not observations
        receipt["phases"].append({"phase": phase, "node": ready["node"], "observations": observations, "exit": 0})

    try:
        shell("mkdir " + shlex.quote(remote))
        created = True
        for source, target in [(args.binary, "probe"), (args.library, "libnode.so"), (args.shell, "shell.jar")]:
            adb("push", str(source.resolve()), remote + "/" + target)
        shell("chmod 700 " + shlex.quote(remote + "/probe"))
        shell(shlex.join([remote + "/probe", "network", "create", remote + "/node"]))
        config = json.loads(shell(shlex.join([remote + "/probe", "network", "resources", remote + "/node"])))
        tcp = [item for item in config["defaults"]["resources"] if item["url"].startswith("stun+tcp://")]
        assert len(tcp) == 1
        config["replacement"] = tcp
        config["tcp_listen"] = "0.0.0.0:0"
        config["udp_direct_disabled"] = True
        save_config(config)
        observe("first", True)
        disabled = copy.deepcopy(config)
        disabled["disabled_uses"] = ["Observation"]
        save_config(disabled)
        observe("disabled", False)
        save_config(config)
        observe("restored", True)
        assert len({phase["node"] for phase in receipt["phases"]}) == 1
        receipt["platform"] = {"model": shell("getprop ro.product.model").strip(), "android": shell("getprop ro.build.version.release").strip(), "sdk": shell("getprop ro.build.version.sdk").strip()}
        receipt["passed"] = True
    finally:
        if created:
            last_pid = shell("cat " + shlex.quote(remote + "/session.pid") + " 2>/dev/null || true").strip()
            if last_pid.isdecimal():
                owned_pids.append(int(last_pid))
            for pid in set(owned_pids):
                command = shell(f"cat /proc/{pid}/cmdline 2>/dev/null || true")
                if remote + "/libnode.so" in command:
                    shell(f"kill {pid}")
            shell("rm -rf " + shlex.quote(remote))
            receipt["owned_directory_removed"] = shell("test ! -e " + shlex.quote(remote) + " && echo removed").strip() == "removed"
        (output / "results.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"PASS physical Android TCP observation: {output}")


if __name__ == "__main__":
    main()
