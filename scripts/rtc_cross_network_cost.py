#!/usr/bin/env python3
"""Sample only the launched owned child tree at50ms using PID+starttime identity."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def process_rows():
    result = {}
    for path in Path("/proc").iterdir():
        if not path.name.isdecimal():
            continue
        try:
            value = (path/"stat").read_text()
            fields = value[value.rindex(")")+2:].split()
            result[int(path.name)] = {"ppid": int(fields[1]), "identity": (int(path.name), int(fields[19])),
                "userTicks": int(fields[11]), "systemTicks": int(fields[12]), "rssPages": max(0, int(fields[21])),
                "name": value[value.index("(")+1:value.rindex(")")],
                "exe": (path/"exe").resolve().name}
        except (OSError, ValueError):
            continue
    return result


def main():
    destination, kind, *command = sys.argv[1:]
    child = subprocess.Popen(command)
    if kind == "broker":
        print(json.dumps({"event": "owned-broker-started", "pid": child.pid}), flush=True)
    ticks, page_size = os.sysconf("SC_CLK_TCK"), os.sysconf("SC_PAGE_SIZE")
    known, samples = {}, 0
    peaks = {"browser": 0, "driver": 0, "service": 0}
    started = time.monotonic()
    while True:
        rows = process_rows()
        included = {child.pid}
        changed = True
        while changed:
            changed = False
            for pid, row in rows.items():
                if pid not in included and (row["ppid"] in included or row["identity"] in known):
                    included.add(pid)
                    changed = True
        current = {name: 0 for name in peaks}
        for pid in included:
            if pid not in rows:
                continue
            row = rows[pid]
            identity = row["identity"]
            prior = known.get(identity)
            category = prior["category"] if prior else ("service" if kind in ("service", "broker") else "driver")
            ancestor = pid
            visited = set()
            while ancestor in rows and ancestor not in visited:
                visited.add(ancestor)
                if rows[ancestor]["exe"] in ("chrome", "chrome_crashpad_handler") or rows[ancestor]["name"].startswith("chrome"):
                    category = "browser"
                    break
                ancestor = rows[ancestor]["ppid"]
            known[identity] = {"pid": pid, "startTicks": identity[1], "category": category,
                "userTicks": max(row["userTicks"], prior["userTicks"] if prior else 0),
                "systemTicks": max(row["systemTicks"], prior["systemTicks"] if prior else 0)}
            current[category] += row["rssPages"] * page_size
        for category, value in current.items():
            peaks[category] = max(peaks[category], value)
        samples += 1
        if child.poll() is not None:
            break
        time.sleep(0.05)
    groups = {}
    for category in peaks:
        members = [row for row in known.values() if row["category"] == category]
        groups[category] = {"userCpuSeconds": sum(row["userTicks"] for row in members)/ticks,
            "systemCpuSeconds": sum(row["systemTicks"] for row in members)/ticks,
            "peakSimultaneousRssBytes": peaks[category], "processes": members}
    Path(destination).write_text(json.dumps({"scope": "only launched child descendants, identity PID+starttime",
        "sampleIntervalMs": 50, "samples": samples, "seconds": time.monotonic()-started, "exit": child.returncode,
        "limitations": ["processes living entirely between samples may be missed", "exited processes retain last sampled CPU, not unsampled final ticks",
            "RSS sums include shared pages and exclude kernel memory; shared host, no CPU isolation", "collector Python CPU excluded"],
        "groups": groups}, indent=2)+"\n")
    raise SystemExit(child.returncode)


if __name__ == "__main__":
    main()
