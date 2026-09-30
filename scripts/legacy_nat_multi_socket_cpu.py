#!/usr/bin/env python3
"""Three predeclared functional CPU measurements; no NAT probability samples."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import signal
import subprocess
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("plan", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    require(os.geteuid() == 0, "run in the dedicated disposable container")
    plan = json.loads(args.plan.read_text())
    repo = pathlib.Path(plan["repo"])
    binary = pathlib.Path(plan["binary"])
    require(plan["modes"] == [0, 2, 4] and plan["repeats"] == 1, "changed measurement denominator")
    require(not args.output.exists(), "never overwrite or silently repeat CPU measurements")
    args.output.mkdir()
    interfaces = json.loads(subprocess.check_output(["ip", "-j", "link", "show"], text=True))
    require([item["ifname"] for item in interfaces] == ["lo"], "CPU fixture is not loopback-only")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    rule = ["INPUT", "-i", "lo", "-p", "udp", "-m", "string", "--algo", "bm", "--hex-string", "|004d494f50554e43483633|", "-j", "DROP"]
    subprocess.run(["iptables", "-A", *rule], check=True)
    subprocess.run(["iptables", "-C", *rule], check=True)
    rows = []
    for mode in plan["modes"]:
        require(digest(binary) == plan["binary_sha256"], "measurement binary changed")
        for file, expected in plan["inputs"].items():
            require(digest(repo / file) == expected, "frozen measurement input changed: " + file)
        subprocess.run(["iptables", "-Z", "INPUT"], check=True)
        env = dict(os.environ, MIOPUNCH_CPU_MODE=str(mode), MIOPUNCH_CPU_RAW_DROP_CONFIRMED="1")
        log_path = args.output / f"mode{mode}.log"
        started = time.monotonic()
        with log_path.open("wb") as output:
            child = subprocess.Popen([str(binary), "--ignored", "--exact", "isolated_functional_cpu_cost_supplement", "--nocapture", "--test-threads=1"], stdout=output, stderr=subprocess.STDOUT, env=env)

            def expired(_signal, _frame):
                child.kill()
                raise RuntimeError("CPU child exceeded the predeclared65s limit")

            signal.signal(signal.SIGALRM, expired)
            signal.alarm(65)
            try:
                waited, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
            finally:
                signal.alarm(0)
            require(waited == child.pid, "rusage does not belong to the measured child")
        wall = time.monotonic() - started
        counters = subprocess.check_output(["iptables", "-L", "INPUT", "-n", "-v", "-x"], text=True)
        (args.output / f"mode{mode}-raw-drop.txt").write_text(counters)
        drops = [line.split() for line in counters.splitlines() if len(line.split()) > 2 and line.split()[2] == "DROP"]
        require(len(drops) == 1, "unexpected raw-drop rule inventory")
        log = log_path.read_text()
        marker = re.search(rf"^CPU_FUNCTIONAL_COMPLETE mode={mode} probes=(\d+) elapsed_ms=(\d+) fd_before=(\d+) fd_after=(\d+) control_alive=true c_stable_id=(\d+) v_stable_id=(\d+)$", log, re.M)
        row = dict(mode=mode, index=0, exit=child.returncode, user_cpu_seconds=usage.ru_utime, system_cpu_seconds=usage.ru_stime,
                   wall_seconds=wall, max_rss_kib=usage.ru_maxrss, voluntary_context_switches=usage.ru_nvcsw,
                   involuntary_context_switches=usage.ru_nivcsw, raw_dropped_packets=int(drops[0][0]),
                   raw_dropped_ip_bytes=int(drops[0][1]), log_sha256=digest(log_path),
                   raw_drop_sha256=digest(args.output / f"mode{mode}-raw-drop.txt"))
        rows.append(row)
        (args.output / "results.json").write_text(json.dumps(dict(result="PENDING", rows=rows), indent=2) + "\n")
        require(child.returncode == 0 and marker, "functional child failed; retain it without replay")
        require(int(marker[1]) == row["raw_dropped_packets"] > 0, "raw no-hit enforcement differs from submitted probes")
        require(marker[3] == marker[4], "physical listeners leaked")
        traces = re.findall(rf"^CPU_FUNCTIONAL_TRACE mode={mode} side=([cv]) stable_id=(\d+) probes=(\d+) responses=0 random_iterations=(\d+) random_draws=(\d+) random_skipped=(\d+) listener_attempted=(\d+) listener_bound=(\d+) listener_failed=0 send_errors=0 send_aborted=(\d+)$", log, re.M)
        require(len(traces) == 2 and [trace[0] for trace in traces] == ["c", "v"], "missing exact full-work traces")
        require([trace[1] for trace in traces] == [marker[5], marker[6]], "original control connection changed")
        require(sum(int(trace[3]) for trace in traces) == (0 if mode == 0 else 1000), "old random work truncated")
        require(sum(int(trace[6]) for trace in traces) == (0 if mode == 0 else 256), "old physical listener work truncated")
        require(all(trace[6] == trace[7] for trace in traces), "functional fixture failed a physical bind")
        stops = re.findall(r"^LEGACY_PHASE_STOP .*elapsed_ms=(\d+) outcome=failed$", log, re.M)
        expected_ms = 5200 if mode == 0 else 38000
        require(len(stops) == 2 and all(expected_ms - 50 <= int(value) <= expected_ms + 1000 for value in stops), "old absolute phase deadline differs")
        if mode == 4:
            require("range: Some((42999, 43003))" in log, "mode4 did not execute the inclusive range2 formula")
        row.update(phase_elapsed_ms=[int(value) for value in stops], traces=traces, control_alive=True,
                   fd_before=int(marker[3]), fd_after=int(marker[4]))
        print(f"CPU mode={mode}/0 user={usage.ru_utime:.6f}s system={usage.ru_stime:.6f}s wall={wall:.3f}s raw_drops={row['raw_dropped_packets']}", flush=True)
    (args.output / "results.json").write_text(json.dumps(dict(result="PASS", scope="whole isolated two-node Rust test process", nat_probability_samples_added=0, rows=rows), indent=2) + "\n")


if __name__ == "__main__":
    main()
