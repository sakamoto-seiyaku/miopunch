#!/usr/bin/env python3
"""Compile and run the real formal TCP node checks with command receipts."""
import argparse
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--case", default="")
    parser.add_argument("--strip-debug", action="store_true", help="strip test executable debug sections at link time on Linux")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    build = ["rtk", "proxy", "cargo", "rustc", "--locked", "--test", "tcp_direct", "--message-format=json"]
    if args.strip_debug:
        build.extend(["--", "-C", "link-arg=-Wl,--strip-debug"])
    with (args.output / "build.log").open("w") as log:
        result = subprocess.run(build, stdout=log, stderr=subprocess.STDOUT)
    receipt = {"build": {"command": build, "exit": result.returncode}}
    binary = None
    if result.returncode == 0:
        for line in (args.output / "build.log").read_text().splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "tcp_direct":
                binary = event.get("executable") or binary
    if binary:
        command = ["rtk", "proxy", binary, args.case, "--nocapture", "--test-threads=1"]
        with (args.output / "test.log").open("w") as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
        receipt["test"] = {"command": command, "exit": result.returncode}
        print((args.output / "test.log").read_text())
    else:
        print((args.output / "build.log").read_text()[-6000:])
        if result.returncode == 0:
            result.returncode = 1
            receipt["error"] = "cargo reported no tcp_direct executable"
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
