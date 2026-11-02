#!/usr/bin/env python3
"""Own loopback-only network and PID namespace for complete Web test actors.

Browser flags cannot constrain background traffic. The whole controller,
provider, relay, targets and browser inherit this single fresh namespace.
No host route, firewall, DNS, network interface or global setting is changed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import sys

from check_forwarding import isolated_environment
from web_process_owner import run_process

ROOT = Path(__file__).resolve().parents[1]


def validate_namespace():
    parent = os.environ.get("M119_WEB_PARENT_NETWORK_NAMESPACE")
    current = os.readlink("/proc/self/ns/net")
    interfaces = socket.if_nameindex()
    assert parent and current != parent, "Web browser requires a fresh owned network namespace"
    assert [name for _, name in interfaces] == ["lo"], "Web namespace has a nonloopback interface"
    routes = Path("/proc/net/route").read_text().splitlines()[1:]
    assert all(line.split()[0] == "lo" for line in routes), "Web namespace has an external route"
    # No external address is contacted. Only this freshly bound own socket is
    # used to prove loopback is up in the namespace.
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(2)
        with socket.create_connection(listener.getsockname(), timeout=2) as client:
            with listener.accept()[0] as peer:
                client.sendall(b"M119 own namespace")
                assert peer.recv(64) == b"M119 own namespace"
    status = dict(line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines() if ":" in line)
    assert int(status["CapEff"].strip(), 16) == 0, "Web namespace controller retained effective capabilities"
    return {"network_namespace": current, "parent_network_namespace": parent,
            "pid_namespace": os.readlink("/proc/self/ns/pid"), "interfaces": [name for _, name in interfaces],
            "only_loopback_routes": True, "effective_capabilities": "0", "own_loopback_socket_exact": True,
            "external_probe_performed": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if args.inside:
        assert sys.stdin.readline() == "M119 RUN\n", "owned namespace launch was not released by its original pipe"
        report = validate_namespace()
        args.receipt.with_suffix(".namespace.json").write_text(json.dumps(report, indent=2)+"\n")
        if args.preflight:
            print(json.dumps(report))
            return
        assert command and Path(command[0]).resolve().parent == ROOT / "scripts"
        assert (Path(command[0]).name.startswith("check_web_")
                or Path(command[0]).name in {"check_browser_host.py", "check_headless_host.py", "check_device_effects.py", "check_terminal_service.py", "check_terminal_browser.py", "check_terminal_sdk.py", "check_terminal_policy.py"}), "only a declared service controller may run"
        os.execv(sys.executable, [sys.executable, *command])
    environment = isolated_environment()
    environment["M119_WEB_PARENT_NETWORK_NAMESPACE"] = os.readlink("/proc/self/ns/net")
    launch = ["/usr/bin/bwrap", "--unshare-net", "--unshare-pid", "--die-with-parent",
              "--ro-bind", "/", "/", "--bind", "/tmp", "/tmp", "--bind", str(ROOT), str(ROOT),
              "--dev", "/dev", "--proc", "/proc", "--chdir", str(ROOT),
              sys.executable, str(Path(__file__).resolve()), "--inside", "--receipt", str(args.receipt.resolve())]
    if args.preflight:
        launch.append("--preflight")
    if command:
        launch += ["--", *command]
    owners = []
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    try:
        result = run_process(launch, env=environment, receipt=args.receipt.with_suffix(".owner.jsonl"),
                             timeout=180, owners=owners, input="M119 RUN\n")
        args.receipt.with_suffix(".stdout.log").write_text(result.stdout)
        args.receipt.with_suffix(".stderr.log").write_text(result.stderr)
        report = {"exit": result.returncode, "all_owned_terminal": all(owner.terminal() for owner in owners),
                  "namespace_receipt": str(args.receipt.with_suffix(".namespace.json")),
                  "controller_source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  "global_network_changes": False}
        args.receipt.write_text(json.dumps(report, indent=2)+"\n")
        print(json.dumps(report))
        assert result.returncode == 0 and report["all_owned_terminal"], "owned namespace controller failed; raw logs retained"
    finally:
        for owner in owners:
            owner.close()


if __name__ == "__main__":
    main()
