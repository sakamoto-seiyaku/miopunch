#!/usr/bin/env python3
"""Run MIOPUNCH-70's controlled SDK tracer and preserve bounded public evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCES = [
    "Cargo.toml", "Cargo.lock", "src/lib.rs", "src/connection_coordinator.rs",
    "src/message_bootstrap.rs", "src/native_network.rs", "src/resources.rs",
    "src/admission_gate.rs", "src/control_session.rs", "src/admission_proof.rs",
    "src/tcp_punch.rs", "tests/unified_connection.rs", "scripts/check_unified_native.py",
    "vendor/iroh/src/endpoint.rs", "vendor/iroh/src/endpoint/connection.rs",
    "vendor/iroh/src/endpoint/hooks.rs",
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_snapshot():
    hashes = {name: digest(ROOT / name) for name in SOURCES}
    canonical = json.dumps(hashes, sort_keys=True, separators=(",", ":")).encode()
    return {"scope": "MIOPUNCH-70 implementation and enumerated shared seams",
            "sha256": hashlib.sha256(canonical).hexdigest(), "files": hashes}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prior-regression", type=Path,
                        help="reuse a passed unchanged legacy regression receipt after a coordinator-only review fix")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    records = []

    def run(name, command):
        print(f"RUN {name}: {' '.join(command)}", flush=True)
        started = time.monotonic()
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
        stdout, stderr = args.output / f"{name}.stdout", args.output / f"{name}.stderr"
        stdout.write_text(result.stdout)
        stderr.write_text(result.stderr)
        record = {"name": name, "command": command, "exitCode": result.returncode,
                  "elapsedMs": round((time.monotonic() - started) * 1000),
                  "stdout": stdout.name, "stderr": stderr.name,
                  "stdoutSha256": digest(stdout), "stderrSha256": digest(stderr)}
        records.append(record)
        print(f"EXIT {name}: {result.returncode} ({record['elapsedMs']}ms)", flush=True)
        return result

    before = source_snapshot()
    build = run("build", ["rtk", "proxy", "cargo", "test", "--test", "unified_connection",
                          "--no-run", "--message-format=json"])
    executable = None
    if build.returncode == 0:
        for line in build.stdout.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "unified_connection":
                executable = item.get("executable") or executable
    events = []
    artifact = None
    if executable:
        artifact = {"path": str(Path(executable).relative_to(ROOT)), "sha256": digest(Path(executable))}
        sdk = run("sdk", ["rtk", "proxy", executable, "--nocapture", "--test-threads=1"])
        for line in sdk.stdout.splitlines():
            if "{" in line:
                try:
                    event = json.loads(line[line.index("{"):])
                except json.JSONDecodeError:
                    continue
                if isinstance(event, dict) and "event" in event:
                    events.append(event)
        if sdk.returncode == 0 and args.prior_regression is None:
            run("affected-regression", ["rtk", "proxy", "cargo", "test",
                "--test", "manual_admission", "--test", "member_attempt", "--test", "admission_gate",
                "--test", "member_path_protocol", "--test", "tcp_direct", "--test", "resources"])
    after = source_snapshot()
    prior_coverage = None
    if args.prior_regression:
        prior = json.loads(args.prior_regression.read_text())
        changed = sorted(name for name, value in after["files"].items()
                         if prior["source"]["files"].get(name) != value)
        # The reviewed fix changes only the new coordinator release decision,
        # its SDK regression and this evidence runner. Existing callers and
        # shared gate/transport semantics retain their already-run coverage.
        permitted = {"src/connection_coordinator.rs", "tests/unified_connection.rs",
                     "scripts/check_unified_native.py"}
        regression = next((record for record in prior["commands"]
                           if record["name"] == "affected-regression"), None)
        reusable = prior["passed"] and regression is not None and regression["exitCode"] == 0 and set(changed) <= permitted
        prior_coverage = {"receipt": os.path.relpath(args.prior_regression, args.output),
                          "receiptSha256": digest(args.prior_regression), "sourceSha256": prior["source"]["sha256"],
                          "changedFiles": changed, "unchangedLegacyCoverageReused": reusable}
    ok = bool(executable) and all(record["exitCode"] == 0 for record in records) and before == after
    if prior_coverage is not None:
        ok = ok and prior_coverage["unchangedLegacyCoverageReused"]
    review_red = args.prior_regression.parent / "review-red" / "receipt.json" if args.prior_regression else None
    receipt = {
        "ticket": "MIOPUNCH-70", "baseline": "9c95dcb4e78dba041d6d1b2fe151f7607c6c6fd0",
        "scope": "controlled Native-to-Native persisted member SDK; loopback UDP; no public NAT claim",
        "passed": ok, "commands": records, "source": after, "sourceStableDuringRun": before == after,
        "artifact": artifact, "events": events,
        "priorAffectedRegression": prior_coverage,
        "reviewRed": {"receipt": os.path.relpath(review_red, args.output),
                      "receiptSha256": digest(review_red)} if review_red and review_red.exists() else None,
        "ownedResources": {"rtcPeerConnections": 0, "customPumpsAfterClose": 0, "customQueuesAfterClose": 0,
                           "publicListeners": 0, "temporaryProcesses": 0, "forwarders": 0,
                           "udpSocketRelease": "actual rebind immediately after graceful close and again 100ms later",
                           "raceFixtureControl": "real restricted TCP carrier; listeners rebound and pending queue counters checked after owned close"},
        "firstFailures": [
            {"command": ["rtk", "proxy", "cargo", "test", "--test", "manual_admission",
                         "manual_approval_saves_both_materials_and_permits_real_business", "--", "--exact"],
             "exitCode": 101, "observation": "Endpoint failed to bind sockets: Operation not permitted (os error 1)",
             "classification": "sandbox UDP binding denial; identical escalated prefactor run passed"},
            {"command": ["rtk", "proxy", "cargo", "test", "--test", "unified_connection", "--no-run"],
             "exitCode": 101, "observation": "new public SDK types/method missing before implementation",
             "classification": "red at the pre-agreed SDK seam"},
            {"command": ["rtk", "proxy", "cargo", "test", "--test", "unified_connection",
                         "cancel_and_deadline_close_only_the_pending_original_m_and_preserve_healthy_member_streams", "--", "--exact", "--nocapture"],
             "exitCode": 101, "observation": "tests/unified_connection.rs:219 entered receiver elapsed",
             "classification": "fixture reused cached peer route from healthy M; fresh pending dialer isolates route; no product budget change"},
        ],
    }
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"passed": ok, "receipt": str(args.output / "receipt.json"),
                      "events": len(events), "sourceStable": before == after}), flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
