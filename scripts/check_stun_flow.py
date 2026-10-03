#!/usr/bin/env python3
"""Build and run the formal UDP STUN lifecycle plus explicit domain TCP slice."""
import argparse
import json
import pathlib
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--case", choices=["udp", "tcp", "both", "coalesced"], default="both")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"case": args.case, "commands": [], "passed": False}

    def run(command, name):
        started = time.monotonic()
        with (output / name).open("w") as log:
            result = subprocess.run(["rtk", "proxy", *command], cwd=root, stdout=log, stderr=subprocess.STDOUT)
        receipt["commands"].append({"argv": command, "exit": result.returncode, "seconds": round(time.monotonic() - started, 3), "log": name})
        (output / "results.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if result.returncode:
            raise SystemExit(f"FAIL exit={result.returncode}; read {output / name}")

    run(["cargo", "test", "--locked", "--test", "tcp_punch", "--no-run", "--message-format=json"], "build.log")
    executables = [item["executable"] for line in (output / "build.log").read_text().splitlines() if line.startswith("{") for item in [json.loads(line)] if item.get("executable") and item.get("target", {}).get("name") == "tcp_punch"]
    worker = pathlib.Path(executables[-1]).resolve().relative_to(root)
    if args.case in ["udp", "both", "coalesced"]:
        run(["cargo", "build", "--locked", "--bin", "udp_observation_probe"], "qad-build.log")
        command = ["docker", "run", "--rm", "--name", f"miopunch67-{time.time_ns()}", "--privileged", "--network", "none", "--mount", f"type=bind,src={root},target=/repo,readonly", "--mount", f"type=bind,src={output},target=/evidence", "miopunch-nat61-lab:bookworm", "python3", "-B", "/repo/scripts/stun_flow_lab.py", str(pathlib.Path("/repo") / worker), "/repo/target/debug/udp_observation_probe", "/evidence/udp"]
        if args.case == "coalesced":
            command.append("coalesced")
        run(command, "udp-lab.log")
    if args.case in ["tcp", "both"]:
        run(["python3", "-B", "scripts/check_tcp_punch.py", "--case", "domain_stun", "--worker", str(worker), "--output", str(output / "tcp")], "tcp-lab.log")
    receipt["passed"] = True
    (output / "results.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(f"PASS formal STUN flow: {output}")


if __name__ == "__main__":
    main()
