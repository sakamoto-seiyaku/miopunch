#!/usr/bin/env python3
"""Prepare exact Windows socket action/pins locally; never SSH or execute it."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
from pin_artifact import pin_artifact

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows-receipt", type=Path, required=True)
    parser.add_argument("--static-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", required=True)
    args = parser.parse_args()
    assert args.label.isascii() and args.label.replace("-", "").isalnum(), "safe Windows run label required"
    build = json.loads(args.windows_receipt.read_text())
    static = json.loads(args.static_receipt.read_text())
    assert build["exit"] == 0 and build["compilerCleanupExit"] == 0 and build["platform"] == "windows"
    assert static["csharpCompiled"] and not static["hostScriptExecuted"] and not static["Windows_execution"]
    static_inputs = json.loads(args.static_receipt.with_name("inputs.json").read_text())
    for name, digest in static_inputs.items():
        assert sha(ROOT / "scripts" / name) == digest, "reviewed static host source drift"
    binary = Path(build["pin"])
    assert sha(binary) == build["sha256"]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    pin_artifact(binary, output / "miopunch-client.exe")
    for name in ["windows_forwarding_host.ps1", "windows_forwarding_sockets.cs"]:
        shutil.copyfile(ROOT / "scripts" / name, output / name)
    manifest = {"task": "MIOPUNCH-118", "qualification": "PREPARED_ONLY_ACTUAL_WINDOWS_UNRUN",
                "files": {"exe": build["sha256"], "host": sha(output / "windows_forwarding_host.ps1"), "helper": sha(output / "windows_forwarding_sockets.cs")},
                "source_receipts": {"Windows_build": str(args.windows_receipt), "local_static": str(args.static_receipt)},
                "external_resources": False, "no_system_install_route_firewall_time_clipboard_changes": True,
                "max_control_requests": 4096, "max_control_bytes": 65536, "max_fixture_clients": 16,
                "native_actual_windows_role_swap_required": True}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    command = 'powershell.exe -NoLogo -NoProfile -NonInteractive -File "$env:LOCALAPPDATA\\Temp\\m118-forward-native5\\windows_forwarding_host.ps1" -Manifest "$env:LOCALAPPDATA\\Temp\\m118-forward-native5\\manifest.json" -RunRoot "$env:TEMP\\m118-forward-native5-runtime" -Mode create'
    command = command.replace("native5", args.label)
    (output / "invocation.txt").write_text(command + "\n")
    (output / "preparation.json").write_text(json.dumps({"ssh_executed": False, "Windows_executed": False,
                 "manifest_sha256": sha(output / "manifest.json"), "files": manifest["files"]}, indent=2) + "\n")
    print(json.dumps({"prepared": str(output), "ssh_executed": False, "Windows_executed": False}))


if __name__ == "__main__":
    main()
