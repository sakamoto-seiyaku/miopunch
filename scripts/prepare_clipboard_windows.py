#!/usr/bin/env python3
"""Prepare a reviewable interactive Windows clipboard action; never uses SSH."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--controller", type=Path, default=Path("scripts/windows_clipboard_host.ps1"))
    parser.add_argument("--helper", type=Path, default=Path("scripts/windows_clipboard_atomic.cs"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build = json.loads(args.build_receipt.read_text())
    binary = Path(build["pin"])
    assert build["platform"] == "windows" and build["exit"] == 0
    assert sha(binary) == build["sha256"]
    assert args.controller.is_file()
    assert args.helper.is_file()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    controller = output / args.controller.name
    shutil.copyfile(args.controller, controller)
    helper = output / args.helper.name
    shutil.copyfile(args.helper, helper)
    deployment = "C:/Users/stati/AppData/Local/Temp/m117-clipboard-native5"
    run = deployment + "/runtime"
    invocation = ["powershell.exe", "-NoProfile", "-STA", "-File", deployment + "/" + controller.name,
                  "-Binary", deployment + "/miopunch-client.exe", "-BinarySha256", build["sha256"],
                  "-ControllerSha256", sha(controller), "-HelperSha256", sha(helper),
                  "-RunDirectory", run, "-Action", "create"]
    manifest = {
        "status": "prepared_only_not_executed", "authorization": "PENDING genuine applicable user grant and tool approval",
        "endpoint": "stati@100.64.10.8 (existing SSH alias laptop)",
        "deployment": deployment, "runtime": run,
        "binary": {"source": str(binary), "sha256": build["sha256"], "originating_build_receipt": str(args.build_receipt)},
        "controller": {"source": str(args.controller.resolve()), "copy": str(controller), "sha256": sha(controller)},
        "helper": {"source": str(args.helper.resolve()), "copy": str(helper), "sha256": sha(helper)},
        "interactive_invocation": invocation,
        "invocation_context": "Current unlocked interactive Windows desktop. SSH Session0 is not qualification; no service, scheduled task, installation, security or route change.",
        "mailbox_protocol": {
            "ready": run + "/ready.json",
            "request": run + "/requests/000001.json (atomic publication, monotonically increasing ids)",
            "response": run + "/responses/000001.json (safe SDK status or expected-owned-marker equality only)",
            "example_copy": {"id": 1, "op": "copy", "label": "reverse", "bytes": 128},
            "example_expected_receive": {"id": 2, "op": "expect", "label": "forward", "bytes": 128},
            "example_sdk": {"id": 3, "op": "sdk", "expectedSdkOk": True, "command": {"id": 101, "op": "clipboard_status"}},
            "example_finish": {"id": 4, "op": "finish"},
            "result": run + ".result.json (finish removes mailbox; poll final result instead of finish response)",
            "run_binding": "ready and final result have the same newly generated runId; reject stale ready/result files",
            "owned_marker": "M117 owned Windows clipboard <ready.runId> <label> Unicode中文🙂, padded with ASCII x to requested UTF-8 byte length",
        },
        "private_material": "Fresh own network/node/card/invitation state only, private runtime with current-user-only NTFS ACL. Existing clipboard formats copied as bounded raw HGLOBAL bytes in RAM only; handle/GDI/private-owner or unsupported formats abort before any change. Snapshot and first marker share one OpenClipboard lock.",
        "business_transport": "miopunch only; SSH/SCP deployment/control/fresh invitations only",
        "clipboard_effects": "Owned marker first, SDK enable/start then subsequent test-only changes; independent STA Win32 equality checks. No clipboard content/digest exported. Restore comparison, sequence ownership and writes share one OpenClipboard lock. Only the last directly written controller revision qualifies; later SDK/user changes (including same/empty text) are preserved. Expected matching SDK data never establishes restoration ownership.",
        "cleanup": "Normal stop, join owned process/stdout/stderr readers, atomically restore only a directly owned revision, remove exact owned runtime only after all owners terminal; failed ownership retains state. Each cleanup exception is aggregated and still attempts final failed receipt. Public final result contains counters/flags only. Controller cannot delete a preexisting run directory/result.",
        "pending_prerequisites": ["Actual remote/deployment authorization", "Actual PowerShell parsing and C# helper compilation/platform execution", "Actual capable unlocked interactive Windows session", "Real Linux↔Windows system clipboard/negative/cleanup assertions"],
        "remote_operations_executed": False, "windows_operations_executed": False,
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (output / "interactive-command.txt").write_text(" ".join('"' + part + '"' if "/" in part else part for part in invocation) + "\n")
    print(json.dumps({"prepared": str(output), "controller_sha256": sha(controller), "remote_operations_executed": False}))


if __name__ == "__main__":
    main()
