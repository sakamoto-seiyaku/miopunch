#!/usr/bin/env python3
"""Freeze and run only the two MIO78 controlled member/MQTT priority cases."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
CASES = ["available_member_contact_precedes_ready_configured_mqtt",
         "unavailable_member_contact_falls_back_with_original_deadline"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = set(ROOT.glob("src/**/*.rs")) | set(ROOT.glob("vendor/**/*.rs"))
    paths |= set(ROOT.glob("vendor/**/Cargo.toml"))
    paths |= {ROOT / name for name in ["Cargo.toml", "Cargo.lock", __file__,
        "tests/contact_member_priority.rs", "tests/support/member_priority_broker.rs",
        "tests/support/member_contact_observation.rs"]}
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths)
            if "target" not in path.relative_to(ROOT).parts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reuse-build", type=Path, help="reuse the exact frozen test artifact; never rebuild for socket permission")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = {"scope": "actual three saved SDK members; owned loopback protected member links and MQTT broker; no public or physical qualification",
              "publicTraffic": False, "physicalDevices": False, "passed": False, "checks": [],
              "singleOriginalAbsoluteDeadlineMs": 12000, "cases": {}}

    def save():
        (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")

    def run(name, command, guard):
        started = time.monotonic()
        with (output / f"{name}.stdout.log").open("w") as stdout, (output / f"{name}.stderr.log").open("w") as stderr:
            child = subprocess.Popen(["rtk", "proxy", *command], cwd=ROOT,
                stdout=stdout, stderr=stderr, start_new_session=True)
            terminated = forced = False
            try:
                code = child.wait(timeout=guard)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                terminated = True
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    forced = True
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
                code = child.returncode
        report["checks"].append({"name": name, "argv": ["rtk", "proxy", *command],
            "exit": code, "guardSeconds": guard, "seconds": time.monotonic() - started,
            "terminatedByGuard": terminated, "forcedKill": forced})
        save()
        return code

    try:
        report["sourceHead"] = subprocess.check_output(["rtk", "proxy", "git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        report["sourcesBeforeBuild"] = sources()
        save()
        if args.reuse_build:
            prior = json.loads(args.reuse_build.read_text())
            if prior.get("sourcesFrozenDuringBuild") is not True:
                raise RuntimeError("prior build had no exact frozen-source identity")
            expected = prior["sourcesAfterBuild"]
            current = report["sourcesBeforeBuild"]
            runner_name = str(Path(__file__).resolve().relative_to(ROOT))
            deltas = {name: {"compiled": value, "current": current.get(name)} for name, value in expected.items()
                      if current.get(name) != value}
            if set(deltas) - {runner_name}:
                raise RuntimeError("compiled Rust or test source changed before reuse")
            binary = Path(prior["artifact"]["frozenPath"])
            if digest(binary) != prior["artifact"]["sha256"]:
                raise RuntimeError("prior immutable test artifact changed")
            report["priorBuildReceipt"] = {"path": str(args.reuse_build.resolve()), "sha256": digest(args.reuse_build),
                "runnerOnlySourceDelta": deltas, "compiledSnapshot": expected}
            report["sourcesAfterBuild"] = expected
            report["sourcesFrozenDuringBuild"] = True
            binaries = [binary]
        else:
            build_exit = run("build", ["cargo", "test", "--locked", "--offline", "-j", "2",
                "--features", "rtc-probe", "--test", "contact_member_priority", "--no-run", "--message-format=json"], 600)
            report["sourcesAfterBuild"] = sources()
            report["sourcesFrozenDuringBuild"] = report["sourcesBeforeBuild"] == report["sourcesAfterBuild"]
            if build_exit != 0:
                raise RuntimeError(f"build exit {build_exit}; raw first failure retained")
            binaries = []
            for line in (output / "build.stdout.log").read_text().splitlines():
                try:
                    value = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if (value.get("reason") == "compiler-artifact" and value["target"]["name"] == "contact_member_priority"
                        and value.get("profile", {}).get("test") and value.get("executable")):
                    binaries.append(Path(value["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("one actual priority test artifact required")
        artifact = output / "contact_member_priority.test-bin"
        shutil.copy2(binaries[0], artifact)
        report["artifact"] = {"builtPath": str(binaries[0]), "frozenPath": str(artifact), "sha256": digest(artifact), "bytes": artifact.stat().st_size}
        if not report["sourcesFrozenDuringBuild"]:
            raise RuntimeError("source changed during build; retain receipt before any SDK qualification")
        for case in CASES:
            code = run(case, [str(artifact), case, "--exact", "--nocapture"], 35)
            lines = (output / f"{case}.stdout.log").read_text().splitlines()
            actual = [json.loads(line.removeprefix("CONTACT_PRIORITY_RECEIPT "))
                      for line in lines if line.startswith("CONTACT_PRIORITY_RECEIPT ")]
            proofs = [json.loads(line) for line in lines if line.startswith('{"') and '"event":"unified-path-ready"' in line]
            report["cases"][case] = {"exit": code, "actualReceipt": actual[0] if len(actual) == 1 else None,
                "currentMProofs": proofs, "oneActualReceipt": len(actual) == 1,
                "bothFreshAckRoles": len(proofs) == 2 and {proof.get("role") for proof in proofs} == {"initiator", "responder"}
                    and all(proof.get("freshAck") is True and proof.get("version") == 1 for proof in proofs)}
            save()
        report["artifactSha256AfterRun"] = digest(artifact)
        report["artifactFrozenDuringRun"] = report["artifactSha256AfterRun"] == report["artifact"]["sha256"]
        report["sourcesAfterRun"] = sources()
        report["sourcesFrozenDuringRun"] = report["sourcesAfterRun"] == report["sourcesAfterBuild"]
        report["passed"] = (report["artifactFrozenDuringRun"] and all(case["exit"] == 0 and case["oneActualReceipt"]
            and case["bothFreshAckRoles"] for case in report["cases"].values()) and len(report["cases"]) == 2)
    except (Exception, KeyboardInterrupt) as error:
        report["failureClass"] = type(error).__name__
        report["failure"] = str(error) or "cancelled"
    finally:
        report["completedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        report["commandExit"] = 0 if report["passed"] else 1
        save()
    print(json.dumps({"passed": report["passed"], "exit": report["commandExit"], "output": str(output)}))
    return report["commandExit"]


if __name__ == "__main__":
    raise SystemExit(main())
