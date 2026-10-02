#!/usr/bin/env python3
"""Real browser control replicas/management, then affected short regressions."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
CASES = ["replica", "precode", "replica-faults", "management", "browser-admission",
         "new-code", "revocation", "storage-corrupt", "browser-precode"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new evidence directory")
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--checks", action="store_true", help="affected native and previous browser regressions")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--wasm-bindgen", default=shutil.which("wasm-bindgen") or "/tmp/miopunch-wasm-tools/bin/wasm-bindgen")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "scope": "MIOPUNCH-51; already verified unchanged slow NAT excluded", "checks": []}
    scenarios = []

    def run(name, command, timeout=300):
        start = time.monotonic()
        logfile = output / f"{name}.log"
        with logfile.open("w") as log:
            try:
                process = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                         timeout=timeout, env=os.environ.copy(), check=False)
                code = process.returncode
            except subprocess.TimeoutExpired:
                code = "timeout"
        result = {"name": name, "command": command, "exit": code,
                  "seconds": round(time.monotonic() - start, 3)}
        results["checks"].append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        for line in logfile.read_text().splitlines():
            if line.startswith("RESULT "):
                scenarios.append({"run": name, "cases": json.loads(line[7:])})
        (output / "scenarios.json").write_text(json.dumps(scenarios, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if code != 0:
            raise SystemExit(f"{name} failed; see {logfile}")

    run("rust-version", ["rtk", "proxy", "rustc", "--version"])
    run("node-version", ["rtk", "proxy", "node", "--version"])
    run("bindgen-version", ["rtk", "proxy", args.wasm_bindgen, "--version"])
    if not args.skip_build:
        run("native-build", ["rtk", "cargo", "build", "--locked", "--offline", "--features", "replica-test-hooks", "--bin", "browser_network_native", "--bin", "browser-probe-native"])
        run("wasm-build", ["rtk", "cargo", "build", "--locked", "--offline", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"])
        run("wasm-bindgen", ["rtk", "proxy", args.wasm_bindgen, "--target", "web", "--out-dir", "browser-probe/pkg", "target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm"])
    for case in [args.case] if args.case else CASES:
        run(f"browser-{case}", ["rtk", "proxy", "node", "browser-probe/harness/run-network.mjs", "--case", case], 180)
    if args.checks:
        run("browser-manual-regression", ["rtk", "proxy", "node", "browser-probe/harness/run-network.mjs", "--suite"], 180)
        run("workspace-typecheck", ["rtk", "cargo", "check", "--locked", "--offline", "--workspace"])
        tests = ["management_auth", "management_store", "daily_management", "offline_replica",
                 "revocation", "precode", "admission_completion", "admission_gate"]
        command = ["rtk", "cargo", "test", "--locked", "--offline", "--features", "replica-test-hooks"]
        for test in tests:
            command += ["--test", test]
        run("native-affected", command + ["--", "--test-threads=1"], 600)
        run("format", ["rtk", "cargo", "fmt", "--all", "--", "--check"])
    results["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    results["passed"] = True
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
