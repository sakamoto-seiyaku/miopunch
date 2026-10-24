#!/usr/bin/env python3
"""Run a pinned public SDK lifecycle consumer in an actual owned X11 session.

The two SDK nodes share this one system clipboard. This qualifies startup/stop
ownership only; independent-desktop data delivery has separate real evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from check_clipboard import Desktop, Runner


class LifecycleRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        report = json.loads(args.consumer_receipt.read_text())
        assert report["exit"] == 0 and report["dependencyRebuilds"] == 0 and not report["CargoInvoked"]
        self.consumer = Path(report["pin"])
        assert hashlib.sha256(self.consumer.read_bytes()).hexdigest() == report["sha256"]
        self.record("runner", "public_sdk_consumer", {"pin": str(self.consumer), "sha256": report["sha256"],
                    "sdk_receipt": report["SDKReceipt"], "scope": "startup/stop only; shared actual X11 system clipboard"})

    def scenario(self):
        desktop = Desktop(self, "sdk-lifecycle")
        desktop.copy("M117 owned public SDK lifecycle marker")
        environment = dict(desktop.env, M117_CLIPBOARD_PRIVATE=str(self.private))
        with (self.output / "consumer.stdout").open("w") as stdout, (self.output / "consumer.stderr").open("w") as stderr:
            command = [str(self.consumer), "--test-threads=1", "--nocapture"]
            if self.filter:
                command.extend(["--exact", self.filter])
            result = subprocess.run(command,
                                    env=environment, stdout=stdout, stderr=stderr, timeout=180)
        self.record("consumer", "exit", {"exit": result.returncode})
        assert result.returncode == 0, "public clipboard startup/stop regression failed; raw output retained"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--consumer-receipt", type=Path, required=True)
    parser.add_argument("--filter", help="run only one newly changed public SDK case on the same verified pin")
    args = parser.parse_args()
    args.case = "lifecycle"
    runner = LifecycleRunner(args)
    runner.filter = args.filter
    runner.run()


if __name__ == "__main__":
    main()
