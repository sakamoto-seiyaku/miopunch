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

from check_build_cache import compiled_artifacts

ROOT = Path(__file__).resolve().parents[1]
CASES = ["available_member_contact_precedes_ready_configured_mqtt",
         "unavailable_member_contact_falls_back_with_original_deadline"]
DATA_HINT_CASES = ["stale_ip_hint_still_uses_healthy_member_before_configured_mqtt",
                   "live_ip_data_precedes_member_and_configured_mqtt_contact",
                   "live_ip_bootstrap_ignores_actual_cached_mqtt_control_route",
                   "announced_ip_without_live_provenance_uses_member_before_configured_mqtt"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    paths = set(ROOT.glob("src/**/*.rs")) | set(ROOT.glob("vendor/**/*.rs"))
    paths |= {p for p in ROOT.glob("tests/fixtures/**/*") if p.is_file()}
    paths |= set(ROOT.glob("tests/**/*.rs")) | {path for path in ROOT.glob("browser-probe/src/**/*") if path.is_file()}
    paths |= set(ROOT.glob("vendor/**/Cargo.toml"))
    paths |= {ROOT / name for name in ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "browser-probe/Cargo.toml", __file__,
        "scripts/cargo_cache.py", "scripts/check_build_cache.py",
        "tests/contact_member_priority.rs", "tests/support/member_priority_broker.rs",
        "tests/support/member_contact_observation.rs"]}
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths)
            if "target" not in path.relative_to(ROOT).parts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reuse-build", type=Path, help="reuse the exact frozen test artifact; never rebuild for socket permission")
    parser.add_argument("--typecheck", action="store_true", help="check native and shared Browser coordinator before the new cases")
    parser.add_argument("--focused-coordinator-check", action="store_true", help="run the existing exact pending-cancel/deadline/healthy-stream check")
    parser.add_argument("--case", action="append", choices=CASES + DATA_HINT_CASES,
                        help="run only these affected actual SDK cases")
    args = parser.parse_args()
    selected_cases = args.case or CASES
    if len(set(selected_cases)) != len(selected_cases):
        parser.error("duplicate case would repeat an already selected test")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = {"scope": "actual three saved SDK members; owned loopback protected member links and MQTT broker; no public or physical qualification",
              "publicTraffic": False, "physicalDevices": False, "passed": False, "checks": [],
              "singleOriginalAbsoluteDeadlineMs": 12000, "cases": {}}

    def save():
        (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")

    def run(name, command, guard, environment=None):
        started = time.monotonic()
        argv = command if command[:2] == ["rtk", "proxy"] else ["rtk", "proxy", *command]
        with (output / f"{name}.stdout.log").open("w") as stdout, (output / f"{name}.stderr.log").open("w") as stderr:
            child = subprocess.Popen(argv, cwd=ROOT,
                stdout=stdout, stderr=stderr, env=environment, start_new_session=True)
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
        report["checks"].append({"name": name, "argv": argv,
            "exit": code, "guardSeconds": guard, "seconds": time.monotonic() - started,
            "terminatedByGuard": terminated, "forcedKill": forced})
        save()
        return code

    try:
        report["sourceHead"] = subprocess.check_output(["rtk", "proxy", "git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        report["sourcesBeforeBuild"] = sources()
        save()
        prior_binary = None
        if args.reuse_build:
            if args.focused_coordinator_check:
                raise RuntimeError("focused check requires an artifact from this build")
            prior = json.loads(args.reuse_build.read_text())
            if prior.get("sourcesFrozenDuringBuild") is not True:
                raise RuntimeError("prior build had no exact frozen-source identity")
            expected = prior["sourcesAfterBuild"]
            current = report["sourcesBeforeBuild"]
            runner_name = str(Path(__file__).resolve().relative_to(ROOT))
            deltas = {name: {"compiled": expected.get(name), "current": current.get(name)}
                      for name in set(expected) | set(current)
                      if current.get(name) != expected.get(name)}
            if set(deltas) - {runner_name}:
                raise RuntimeError("compiled source, lock or configuration changed before reuse")
            prior_binary = Path(prior["artifact"]["frozenPath"])
            if digest(prior_binary) != prior["artifact"]["sha256"]:
                raise RuntimeError("prior immutable test artifact changed")
            report["priorBuildReceipt"] = {"path": str(args.reuse_build.resolve()), "sha256": digest(args.reuse_build),
                "runnerOnlySourceDelta": deltas, "compiledSnapshot": expected}
        if args.typecheck:
            commands = [
                ("native-typecheck", ["check", "--locked", "--offline", "-j", "2", "--lib", "--features", "rtc-probe"]),
                ("browser-typecheck", ["check", "--locked", "--offline", "-j", "2",
                    "--manifest-path", "browser-probe/Cargo.toml", "--target", "wasm32-unknown-unknown", "--features", "rtc-probe"]),
            ]
            for name, command in commands:
                with compiled_artifacts(output, name, command, sources,
                        report["sourcesBeforeBuild"], run, 300):
                    pass
        artifact = output / "contact_member_priority.test-bin"
        focused = None
        if prior_binary is not None:
            shutil.copy2(prior_binary, artifact)
            if digest(artifact) != prior["artifact"]["sha256"]:
                raise RuntimeError("immutable artifact changed while copying")
            report["artifact"] = {"builtPath": str(prior_binary), "frozenPath": str(artifact),
                "sha256": digest(artifact), "bytes": artifact.stat().st_size}
            report["sourcesAfterBuild"] = sources()
            report["sourcesFrozenDuringBuild"] = True
        else:
            command = ["test", "--locked", "--offline", "-j", "2", "--features", "rtc-probe", "--test", "contact_member_priority"]
            if args.focused_coordinator_check:
                command += ["--test", "unified_connection"]
            with compiled_artifacts(output, "build", [*command, "--no-run"], sources,
                    report["sourcesBeforeBuild"], run, 600) as entries:
                binaries = [Path(value["executable"]) for value in entries
                    if value["target"]["name"] == "contact_member_priority"
                    and value.get("profile", {}).get("test") and value.get("executable")]
                if len(binaries) != 1:
                    raise RuntimeError("one actual priority test artifact required")
                shutil.copy2(binaries[0], artifact)
                report["artifact"] = {"builtPath": str(binaries[0]), "frozenPath": str(artifact),
                    "sha256": digest(artifact), "bytes": artifact.stat().st_size}
                if args.focused_coordinator_check:
                    focused_binaries = [Path(value["executable"]) for value in entries
                        if value["target"]["name"] == "unified_connection"
                        and value.get("profile", {}).get("test") and value.get("executable")]
                    if len(focused_binaries) != 1:
                        raise RuntimeError("one focused coordinator artifact required")
                    focused = output / "unified_connection.test-bin"
                    shutil.copy2(focused_binaries[0], focused)
                    report["focusedCoordinatorArtifact"] = {"builtPath": str(focused_binaries[0]),
                        "frozenPath": str(focused), "sha256": digest(focused), "bytes": focused.stat().st_size}
                report["sourcesAfterBuild"] = sources()
                report["sourcesFrozenDuringBuild"] = True
                save()
        for case in selected_cases:
            code = run(case, [str(artifact), case, "--exact", "--nocapture"], 35)
            lines = (output / f"{case}.stdout.log").read_text().splitlines()
            if "CONTACT_PRIORITY_ATTEMPT_BEGIN" in lines:
                lines = lines[lines.index("CONTACT_PRIORITY_ATTEMPT_BEGIN") + 1:]
            actual = [json.loads(line.removeprefix("CONTACT_PRIORITY_RECEIPT "))
                      for line in lines if line.startswith("CONTACT_PRIORITY_RECEIPT ")]
            proofs = [json.loads(line) for line in lines if line.startswith('{"') and '"event":"unified-path-ready"' in line]
            report["cases"][case] = {"exit": code, "actualReceipt": actual[0] if len(actual) == 1 else None,
                "currentMProofs": proofs, "oneActualReceipt": len(actual) == 1,
                "bothFreshAckRoles": len(proofs) == 2 and {proof.get("role") for proof in proofs} == {"initiator", "responder"}
                    and all(proof.get("freshAck") is True and proof.get("version") == 1 for proof in proofs)}
            save()
        focused_passed = True
        if args.focused_coordinator_check:
            if focused is None:
                raise RuntimeError("focused artifact was not pinned under the build lease")
            focused_passed = run("focused-pending-cancel-deadline-cleanup", [str(focused),
                "cancel_and_deadline_close_only_the_pending_original_m_and_preserve_healthy_member_streams", "--exact", "--nocapture"], 30) == 0
            report["focusedCoordinatorArtifactSha256AfterRun"] = digest(focused)
            focused_passed &= report["focusedCoordinatorArtifactSha256AfterRun"] == report["focusedCoordinatorArtifact"]["sha256"]
        report["artifactSha256AfterRun"] = digest(artifact)
        report["artifactFrozenDuringRun"] = report["artifactSha256AfterRun"] == report["artifact"]["sha256"]
        report["sourcesAfterRun"] = sources()
        report["sourcesFrozenDuringRun"] = report["sourcesAfterRun"] == report["sourcesAfterBuild"]
        report["passed"] = (focused_passed and report["artifactFrozenDuringRun"] and all(case["exit"] == 0 and case["oneActualReceipt"]
            and case["bothFreshAckRoles"] for case in report["cases"].values()) and len(report["cases"]) == len(selected_cases) and report["sourcesFrozenDuringRun"])
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
