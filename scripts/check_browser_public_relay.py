#!/usr/bin/env python3
"""Qualify the current real browser/native implementation on one official n0 relay."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
OFFICIAL_RELAYS = tuple(f"https://{region}.relay.n0.iroh.link./"
                        for region in ("aps1-1", "use1-1", "usw1-1", "euc1-1"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new evidence directory")
    parser.add_argument("--relay", choices=OFFICIAL_RELAYS, default=OFFICIAL_RELAYS[0])
    parser.add_argument("--skip-build", action="store_true", help="explicitly reuse current build")
    parser.add_argument("--checks", action="store_true", help="short affected checks; unchanged slow NAT excluded")
    parser.add_argument("--wasm-bindgen", default=shutil.which("wasm-bindgen") or "/tmp/miopunch-wasm-tools/bin/wasm-bindgen")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "scope": "MIOPUNCH-65 official public browser relay; unchanged slow NAT excluded",
               "relay": args.relay, "reused_build": args.skip_build, "passed": False, "checks": []}

    def run(name, command, timeout=600, environment=None):
        start = time.monotonic()
        env = os.environ.copy()
        env.update(environment or {})
        with (output / f"{name}.log").open("w") as log:
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                       env=env, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                code = "timeout"
        result = {"name": name, "command": command, "exit": code,
                  "seconds": round(time.monotonic() - start, 3)}
        results["checks"].append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if code != 0:
            raise SystemExit(f"{name} failed; see {output / (name + '.log')}")

    run("rust-version", ["rtk", "proxy", "rustc", "--version"])
    run("node-version", ["rtk", "proxy", "node", "--version"])
    run("bindgen-version", ["rtk", "proxy", args.wasm_bindgen, "--version"])
    run("revision", ["rtk", "git", "rev-parse", "HEAD"])
    run("working-tree", ["rtk", "git", "status", "--short"])
    # 404 at / is a valid relay response; this records normal system TLS trust,
    # not application success. Actual WSS/data qualification follows below.
    run("relay-https", ["rtk", "proxy", "curl", "--head", "--max-time", "20", args.relay], 30)
    if not args.skip_build:
        native_build = ["rtk", "cargo", "build", "--locked", "--offline", "--bin", "browser_network_native"]
        if args.checks:
            native_build += ["--bin", "browser-probe-native"]
        run("native-build", native_build)
        run("wasm-build", ["rtk", "cargo", "build", "--locked", "--offline", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"])
        run("wasm-bindgen", ["rtk", "proxy", args.wasm_bindgen, "--target", "web", "--out-dir", "browser-probe/pkg", "target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm"])
    with tempfile.TemporaryDirectory(prefix="mio65-browser-") as fixture:
        run("public-browser", ["rtk", "proxy", "node", "browser-probe/harness/run-public-relay.mjs", "--relay", args.relay, "--evidence", str(output)], 180,
            {"MIO65_FIXTURE_DIR": fixture})
    if args.checks:
        run("browser-replica-regression", ["rtk", "proxy", "node", "browser-probe/harness/run-network.mjs", "--case", "replica"], 180)
        run("workspace-typecheck", ["rtk", "cargo", "check", "--locked", "--offline", "--workspace"])
        run("affected-native", ["rtk", "cargo", "test", "--locked", "--offline", "--test", "manual_admission", "--test", "offline_replica", "--test", "admission_gate", "--", "--test-threads=1"])
        run("format", ["rtk", "cargo", "fmt", "--all", "--", "--check"])
    results["completed_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    results["passed"] = True
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
