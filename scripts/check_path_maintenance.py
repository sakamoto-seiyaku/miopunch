#!/usr/bin/env python3
"""Ordered real path maintenance and affected short regression, without NAT matrices."""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import time

CASES = [
    "first_same_stream_survives_validated_tcp_to_udp_switch",
    "priority_closed_connection_fails_old_stream_and_reports_new_identity",
    "living_paths_are_reevaluated_when_real_latency_changes",
    "restricted_control_paths_do_not_displace_business_quality_selection",
    "quality_selects_better_native_relay_and_keeps_tcp_as_existing_backup",
    "control_paths_recover_a_new_tcp_carrier_within_the_original_connection_window",
    "explicit_relay_preference_switches_the_same_udp_stream_and_retains_udp_backup",
    "explicit_direct_preference_wins_even_when_permitted_relay_is_faster",
    "disabled_relay_is_never_opened_or_selected_even_with_explicit_relay_preference",
    "control_paths_never_recover_a_closed_business_connection",
    "control_paths_stay_live_while_physical_business_loss_ends_queued_bytes",
    "control_paths_reject_an_unknown_original_tls_binding_without_changing_business",
    "control_paths_recover_when_the_original_accepting_side_initiates",
    "control_paths_do_not_report_stale_same_address_as_recovered",
]
FEATURES = "nat-lab-test-hooks,replica-test-hooks,member-signaling-test-hooks"

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--suite", action="store_true")
    parser.add_argument("--start-at", choices=["resources", "native-gates-proofs", "mqtt-carrier-and-purpose", "node-state", "tcp-member-forwarding", "physical-bindings", "formal-node-budget", "library", "all-targets", "wasm", "format", "fork-format"], help="resume affected suite at this check; skip already passed acceptance")
    args = parser.parse_args()
    if args.case and args.suite:
        parser.error("case and suite are separate modes")
    if args.start_at and not args.suite:
        parser.error("start-at requires suite")
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"checks": [], "passed": False}
    environment = os.environ.copy()
    owned = []

    def interrupted(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)

    def save():
        (output / "results.json").write_text(json.dumps(receipt, indent=2) + "\n")

    def run(name, command):
        command = ["rtk", "proxy", *command]
        print(f"Starting {name}", flush=True)
        started = time.monotonic()
        with (output / (name + ".log")).open("w") as log:
            result = subprocess.run(command, cwd=root, env=environment, stdout=log, stderr=subprocess.STDOUT)
        log = (output / (name + ".log")).read_text()
        counts = [int(value) for value in re.findall(r"test result: ok\. (\d+) passed", log)]
        receipt["checks"].append({"command": command, "exit": result.returncode, "seconds": round(time.monotonic() - started, 3), "log": name + ".log", "reported_passed": sum(counts)})
        save()
        if result.returncode:
            print(log[-6000:], flush=True)
            raise RuntimeError(f"{name} exited {result.returncode}")
        return log

    try:
        build = ["cargo", "test", "--locked", "--features", FEATURES, "--test", "path_maintenance", "--no-run", "--message-format=json"]
        log = run("build", build)
        binary = None
        for line in log.splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("reason") == "compiler-artifact" and event.get("target", {}).get("name") == "path_maintenance":
                binary = event.get("executable") or binary
        if not binary:
            raise RuntimeError("cargo reported no path-maintenance executable")
        listed = subprocess.check_output(["rtk", "proxy", binary, "--list"], cwd=root, text=True)
        cases = [] if args.start_at else ([args.case] if args.case else CASES)
        for case in cases:
            if f"{case}: test" not in listed:
                raise RuntimeError(f"test case absent: {case}")
        if args.suite or any("control_paths" in case for case in cases):
            for _ in range(2 if args.suite else 1):
                name = f"miopunch47-maintenance-{time.time_ns()}"
                command = ["rtk", "proxy", "docker", "run", "--detach", "--name", name, "--publish", "127.0.0.1::1883", "--mount", f'type=bind,src={root / "scripts/mosquitto-test.conf"},target=/mosquitto/config/mosquitto.conf,readonly', "eclipse-mosquitto:2"]
                subprocess.run(command, check=True, capture_output=True, text=True)
                owned.append(name)
                broker = subprocess.check_output(["rtk", "proxy", "docker", "port", name, "1883/tcp"], text=True).strip()
                host, port = broker.rsplit(":", 1)
                deadline = time.monotonic() + 10
                while True:
                    try:
                        with socket.create_connection((host, int(port)), timeout=1):
                            break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.1)
                receipt.setdefault("brokers", []).append({"command": command, "owned": name, "address": broker})
            environment["MIOPUNCH_MQTT_BROKER"] = receipt["brokers"][0]["address"]
            if args.suite:
                environment["MIOPUNCH_MQTT_SECOND_BROKER"] = receipt["brokers"][1]["address"]
        for case in cases:
            run(case, [binary, case, "--exact", "--nocapture", "--test-threads=1"])
            if receipt["checks"][-1]["reported_passed"] != 1:
                raise RuntimeError(f"expected one executed acceptance test: {case}")
        if args.suite:
            def tests(*targets, selector=None):
                command = ["cargo", "test", "--locked", "--features", FEATURES]
                for target in targets:
                    command += ["--test", target]
                if selector:
                    command.append(selector)
                return command + ["--", "--test-threads=1", "--nocapture"]
            checks = [
                ("resources", tests("resources")),
                ("native-gates-proofs", tests("native_network", "admission_gate", "admission_proof", "revocation")),
                ("mqtt-carrier-and-purpose", tests("mqtt_probe", "mqtt_upgrade", "unknown_member_bootstrap", "mqtt_bootstrap")),
                ("node-state", tests("manual_admission", "offline_replica", "network_recovery")),
                ("tcp-member-forwarding", tests("tcp_direct", "member_forward")),
                ("physical-bindings", tests("legacy_multi_socket")),
                ("formal-node-budget", tests("legacy_nat_runtime", selector="formal_")),
                ("library", ["cargo", "test", "--locked", "--lib", "--", "--test-threads=1"]),
                ("all-targets", ["cargo", "check", "--locked", "--workspace", "--all-targets", "--features", FEATURES]),
                ("wasm", ["cargo", "check", "--locked", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
                ("format", ["cargo", "fmt", "--check"]),
                ("fork-format", ["rustfmt", "--check", "--edition", "2024", "--config", "skip_children=true", "vendor/iroh/src/endpoint.rs", "vendor/iroh/src/socket.rs", "vendor/iroh/src/socket/remote_map/remote_state.rs", "vendor/iroh/src/socket/remote_map/remote_state/path_watcher.rs", "vendor/noq/src/lib.rs", "vendor/noq/src/path.rs", "vendor/noq-proto/src/connection/mod.rs", "vendor/noq-proto/src/tests/multipath.rs"]),
            ]
            if args.start_at:
                checks = checks[next(index for index, (name, _) in enumerate(checks) if name == args.start_at):]
            for name, command in checks:
                run(name, command)
        receipt["passed"] = True
    except KeyboardInterrupt:
        receipt["interrupted"] = True
    except Exception as error:
        receipt["error"] = str(error)
        print(f"FAIL: {error}", flush=True)
    finally:
        receipt["cleanup"] = []
        for name in owned:
            result = subprocess.run(["rtk", "proxy", "docker", "rm", "--force", "--volumes", name], capture_output=True, text=True)
            receipt["cleanup"].append({"owned": name, "exit": result.returncode})
            receipt["passed"] &= result.returncode == 0
        save()
    print(f'{"PASS" if receipt["passed"] else "FAIL"}: {output}', flush=True)
    return 0 if receipt["passed"] else 1

if __name__ == "__main__":
    sys.exit(main())
