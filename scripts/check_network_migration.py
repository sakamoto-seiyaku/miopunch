#!/usr/bin/env python3
"""MIOPUNCH-54: ordered real migration checks, owned MQTT, affected regressions."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--skip-regressions", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve()
    receipts = []
    name = f"miopunch54-mqtt-{os.getpid()}"
    created = False
    with tempfile.TemporaryDirectory(prefix="miopunch54-") as temporary:
        config = Path(temporary) / "mosquitto.conf"
        config.write_text("listener 1883 0.0.0.0\nallow_anonymous true\npersistence false\n")
        os.chmod(temporary, 0o755)
        os.chmod(config, 0o644)
        try:
            subprocess.run(["docker", "run", "-d", "--name", name,
                            "-p", "127.0.0.1:0:1883", "-v",
                            f"{config}:/mosquitto/config/mosquitto.conf:ro",
                            "eclipse-mosquitto:2"], check=True, capture_output=True)
            created = True
            broker = subprocess.check_output(["docker", "port", name, "1883"], text=True).strip()
            env = dict(os.environ, MIOPUNCH_MQTT_BROKER=broker)
            base = ["cargo", "test", "--locked", "--features", "replica-test-hooks", "--test", "network_migration"]
            commands = [
                ("first", base + ["source_works_until_saved_target_member_data_then_switch_and_exit", "--", "--exact", "--nocapture"]),
                ("priority", base + ["signaling_only_target_cannot_switch_and_formal_exit_cannot_roll_back", "--", "--exact", "--ignored", "--nocapture"]),
                ("remaining", base + ["--", "--nocapture", "--skip", "source_works_until_saved_target_member_data_then_switch_and_exit", "--skip", "signaling_only_target_cannot_switch_and_formal_exit_cannot_roll_back"]),
            ]
            if not args.skip_regressions:
                affected = ["native_network", "manual_admission", "admission_completion", "daily_management", "admission_gate", "offline_replica", "revocation", "precode"]
                commands.append(("affected", ["cargo", "test", "--locked", "--features", "replica-test-hooks"] + [part for test in affected for part in ("--test", test)] + ["--", "--nocapture"]))
                commands.extend([
                    ("check-native", ["cargo", "check", "--locked", "--all-targets", "--features", "replica-test-hooks"]),
                    ("check-wasm", ["cargo", "check", "--locked", "-p", "miopunch-browser-probe", "--target", "wasm32-unknown-unknown"]),
                    ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
                ])
            for label, command in commands:
                with (output / f"{label}.log").open("w") as log:
                    completed = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT)
                receipts.append({"check": label, "command": command, "exit": completed.returncode})
                (output / "results.json").write_text(json.dumps(receipts, indent=2) + "\n")
                print(f"{label}: exit {completed.returncode}", flush=True)
                if completed.returncode:
                    raise SystemExit(completed.returncode)
        finally:
            if created:
                removed = subprocess.run(["docker", "rm", "-f", "-v", name], capture_output=True, text=True)
                (output / "cleanup.json").write_text(json.dumps({"container": name, "exit": removed.returncode, "stdout": removed.stdout, "stderr": removed.stderr}, indent=2) + "\n")
                if removed.returncode:
                    raise RuntimeError("owned MQTT cleanup failed")


if __name__ == "__main__":
    main()
