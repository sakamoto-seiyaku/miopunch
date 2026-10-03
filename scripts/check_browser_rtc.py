#!/usr/bin/env python3
"""Build and run step 4: synthetic RTC DataChannel over protected relay-M signaling."""
import argparse
import json
import hashlib
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time
import tempfile
from browser_mqtt_trust import certificates, private_nss

ROOT = Path(__file__).resolve().parent.parent
RELAYS = tuple(f"https://{region}.relay.n0.iroh.link./" for region in ("aps1-1", "use1-1", "usw1-1", "euc1-1"))


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def source_snapshot():
    changed = subprocess.check_output(["rtk", "proxy", "git", "diff", "--name-only", "HEAD"], cwd=ROOT, text=True)
    untracked = subprocess.check_output(["rtk", "proxy", "git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, text=True)
    paths = {"scripts/check_browser_paths.py", "scripts/check_browser_rtc.py", "browser-probe/harness/run-rtc.mjs", "browser-probe/harness/run-rtc-member.mjs"}
    paths.update(path for path in (changed + untracked).splitlines() if Path(path).suffix in (".rs", ".toml", ".lock", ".py", ".mjs"))
    return {path: sha256(ROOT / path) for path in sorted(paths) if (ROOT / path).is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--case", choices=("rtc-channel", "relay-member-rtc"), default="rtc-channel")
    parser.add_argument("--relay", choices=("owned", *RELAYS), default="owned")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--pump-failure-only", action="store_true", help="narrow owned S5 pump termination regression")
    parser.add_argument("--wasm-bindgen", default=shutil.which("wasm-bindgen") or "/tmp/miopunch-wasm-tools/bin/wasm-bindgen")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    target = args.target_dir.resolve()
    result = {"scope": "MIOPUNCH-68 step 5; same Ordinary-M relay to RTC" if args.case == "relay-member-rtc" else "MIOPUNCH-68 step 4; synthetic RTC, not MIO path or same-M upgrade", "startedUtc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "relay": args.relay, "passed": False, "commands": [], "sourceBefore": source_snapshot()}

    def save():
        (output / "results.json").write_text(json.dumps(result, indent=2) + "\n")

    def run(name, command, timeout=900):
        started = time.monotonic()
        with (output / f"{name}.log").open("w") as log:
            process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except BaseException as error:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                code = "timeout" if isinstance(error, (subprocess.TimeoutExpired, TimeoutError)) else "cancelled"
        entry = {"name": name, "command": command, "seconds": round(time.monotonic()-started, 3), "exit": code}
        result["commands"].append(entry)
        save()
        print(json.dumps(entry), flush=True)
        if code != 0:
            raise RuntimeError(f"{name} failed; see {output / (name+'.log')}")

    def interrupted(signum, frame):
        raise TimeoutError("fixture preparation deadline") if signum == signal.SIGALRM else KeyboardInterrupt()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    try:
        run("revision", ["rtk", "git", "rev-parse", "HEAD"])
        run("rust-version", ["rtk", "proxy", "rustc", "--version"])
        run("bindgen-version", ["rtk", "proxy", args.wasm_bindgen, "--version"])
        if not args.skip_build:
            run("native-build", ["rtk", "cargo", "build", "--locked", "--offline", "--features", "rtc-probe", "--bin", "browser_network_native", "--bin", "browser-probe-native", "--target-dir", str(target), "--jobs", "2"])
            run("wasm-build", ["rtk", "cargo", "build", "--locked", "--offline", "-p", "miopunch-browser-probe", "--features", "rtc-probe", "--target", "wasm32-unknown-unknown", "--target-dir", str(target), "--jobs", "2"])
            run("wasm-bindgen", ["rtk", "proxy", args.wasm_bindgen, "--target", "web", "--out-dir", "browser-probe/pkg", str(target / "wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm")])
        result["builtArtifacts"] = {str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path): sha256(path) for path in (target / "debug/browser_network_native", target / "debug/browser-probe-native", ROOT / "browser-probe/pkg/miopunch_browser_probe.js", ROOT / "browser-probe/pkg/miopunch_browser_probe_bg.wasm")}
        command = ["rtk", "proxy", "node", "browser-probe/harness/run-rtc.mjs", "--case", args.case, "--relay", args.relay, "--native", str(target / "debug/browser_network_native"), "--evidence", str(output)]
        if args.pump_failure_only:
            command.append("--pump-failure-only")
        if args.relay == "owned":
            with tempfile.TemporaryDirectory(prefix="mio-rtc-s4-trust-") as fixture_name:
                fixture = Path(fixture_name)
                started = time.monotonic()
                signal.setitimer(signal.ITIMER_REAL, 15)
                ca, leaf, key = certificates(fixture)
                private_nss(fixture / "nssdb", ca)
                for source, destination in [(ca, "ca.der"), (leaf, "leaf.der")]:
                    subprocess.run(["rtk", "proxy", "openssl", "x509", "-in", str(source), "-outform", "DER", "-out", str(fixture / destination)], check=True, capture_output=True, timeout=10)
                subprocess.run(["rtk", "proxy", "openssl", "pkcs8", "-topk8", "-nocrypt", "-in", str(key), "-outform", "DER", "-out", str(fixture / "leaf-key.der")], check=True, capture_output=True, timeout=10)
                result["certificate"] = {"normalVerification": True, "privateNssOnly": True, "caPemSha256": hashlib.sha256(ca.read_bytes()).hexdigest()}
                signal.setitimer(signal.ITIMER_REAL, 0)
                elapsed = time.monotonic()-started
                result["fixtureSetupSeconds"] = round(elapsed, 3)
                run(args.case, command + ["--fixture", str(fixture), "--relay-fixture", str(target / "debug/browser-probe-native")], timeout=max(0.001, (45 if args.case == "relay-member-rtc" else 60)-elapsed))
        else:
            run(args.case, command, timeout=45 if args.case == "relay-member-rtc" else 60)
        result["passed"] = True
    except (Exception, KeyboardInterrupt) as error:
        result["failure"] = str(error)
        raise
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        result["sourceAfter"] = source_snapshot()
        result["sourceUnchanged"] = result["sourceBefore"] == result["sourceAfter"]
        result["completedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        save()


if __name__ == "__main__":
    main()
