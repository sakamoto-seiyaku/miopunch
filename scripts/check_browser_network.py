#!/usr/bin/env python3
"""Real Chromium/native manual admission and affected short regression checks."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
CASES = ["first", "lifecycle", "persistent-restart", "multi-page", "delivery-interruption",
         "storage-clear", "commit-loss", "no-relay", "disabled-relay", "unavailable-relay", "tls"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new evidence directory")
    parser.add_argument("--case", choices=CASES, help="one scenario after the first admission check")
    parser.add_argument("--checks", action="store_true", help="also run affected native checks")
    parser.add_argument("--skip-build", action="store_true", help="use the existing compiled build")
    parser.add_argument("--wasm-bindgen", default=shutil.which("wasm-bindgen") or "/tmp/miopunch-wasm-tools/bin/wasm-bindgen")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "scope": "MIOPUNCH-50 browser manual admission; unchanged slow NAT excluded", "checks": []}

    def run(name, command, timeout=300):
        start = time.monotonic()
        with (output / f"{name}.log").open("w") as log:
            try:
                process = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                         timeout=timeout, env=os.environ.copy(), check=False)
                code = process.returncode
            except subprocess.TimeoutExpired:
                code = "timeout"
        result = {"name": name, "command": command, "exit": code, "seconds": round(time.monotonic() - start, 3)}
        results["checks"].append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if code != 0:
            raise SystemExit(f"{name} failed; see {output / (name + '.log')}")

    run("rust-version", ["rtk", "proxy", "rustc", "--version"])
    run("node-version", ["rtk", "proxy", "node", "--version"])
    run("bindgen-version", ["rtk", "proxy", args.wasm_bindgen, "--version"])
    if not args.skip_build:
        run("native-build", ["rtk", "cargo", "build", "--locked", "--offline", "--bin", "browser_network_native", "--bin", "browser-probe-native"])
        run("wasm-build", ["rtk", "cargo", "build", "--locked", "--offline", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"])
        run("wasm-bindgen", ["rtk", "proxy", args.wasm_bindgen, "--target", "web", "--out-dir", "browser-probe/pkg", "target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm"])
    browser = ["rtk", "proxy", "node", "browser-probe/harness/run-network.mjs"]
    browser += ["--case", args.case] if args.case else ["--suite"]
    run("browser", browser, timeout=180)
    if args.checks:
        run("workspace-typecheck", ["rtk", "cargo", "check", "--locked", "--offline", "--workspace"])
        run("native-proof", ["rtk", "cargo", "test", "--locked", "--offline", "--test", "admission_proof", "--", "--test-threads=1"])
        run("native-admission-gate", ["rtk", "cargo", "test", "--locked", "--offline", "--test", "admission_gate", "--", "--test-threads=1"])
        run("native-manual-admission", ["rtk", "cargo", "test", "--locked", "--offline", "--test", "manual_admission", "--", "--test-threads=1"])
        run("native-rfc-proof", ["rtk", "cargo", "test", "--locked", "--offline", "--lib", "admission_proof", "--", "--test-threads=1"])
        run("format", ["rtk", "cargo", "fmt", "--all", "--", "--check"])
    results["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    results["passed"] = True
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
