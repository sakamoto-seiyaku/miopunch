#!/usr/bin/env python3
"""Validate Windows host sources locally without executing host/Win32 code."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

from pin_artifact import pin_artifact

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pwsh", type=Path, required=True)
    parser.add_argument("--pwsh-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = ROOT / "scripts"
    inputs = {name: sha(source / name) for name in ["windows_clipboard_host.ps1", "windows_clipboard_atomic.cs",
              "check_windows_clipboard_sources.ps1", "check_windows_clipboard_sources.py", "pin_artifact.py"]}

    def validate():
        assert sha(args.pwsh) == args.pwsh_sha and os.access(args.pwsh, os.X_OK), "verified runnable Linux parser required"
        assert all(sha(source / name) == digest for name, digest in inputs.items()), "static source input changed"

    validate()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")
    (output / "plan.json").write_text(json.dumps({"monotonic": time.monotonic(), "inputs": inputs,
        "tool_sha256": args.pwsh_sha, "profile": "Linux PowerShell7.4.13 parser/.NET managed C#5 static compile",
        "budget_bytes": 512 * 1024**2, "host_executed": False, "assembly_loaded": False,
        "native_methods_invoked": False, "clipboard_accessed": False, "Cargo_invoked": False}, indent=2) + "\n")
    environment = dict(os.environ, POWERSHELL_TELEMETRY_OPTOUT="1", XDG_CACHE_HOME=str(output / "cache"),
                       PSModuleAnalysisCachePath=str(output / "cache/module-analysis"), DOTNET_CLI_HOME=str(output / "dotnet"))
    for key in ["LD_PRELOAD", "FAKETIME", "DOTNET_STARTUP_HOOKS", "DOTNET_ADDITIONAL_DEPS"]:
        environment.pop(key, None)
    command = ["rtk", "proxy", str(args.pwsh.resolve()), "-NoLogo", "-NoProfile", "-NonInteractive", "-File",
               str(source / "check_windows_clipboard_sources.ps1"), "-HostScript", str(source / "windows_clipboard_host.ps1"),
               "-Helper", str(source / "windows_clipboard_atomic.cs"), "-Output", str(output)]
    validate()
    with (output / "stdout").open("w") as stdout, (output / "stderr").open("w") as stderr:
        result = subprocess.run(command, env=environment, stdout=stdout, stderr=stderr, timeout=60)
    drift = [name for name, digest in inputs.items() if sha(source / name) != digest]
    (output / "exit.json").write_text(json.dumps({"exit": result.returncode, "command": command,
        "source_drift": drift}, indent=2) + "\n")
    assert result.returncode == 0 and not drift, "local static source validation failed; inspect preserved raw output"
    report = json.loads((output / "result.json").read_text())
    assert report["csharpCompiled"] and not report["powerShellParseErrors"]
    pin_artifact(output / "helper.dll", output / "pinned/helper.dll")
    report["managed_assembly_pin"] = str(output / "pinned/helper.dll")
    report["managed_assembly_sha256"] = sha(output / "pinned/helper.dll")
    (output / "qualified.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"source_parse_and_managed_compile": "pass", "native_methods_invoked": False,
                      "Windows_execution": False, "receipt": str(output / "qualified.json")}))


if __name__ == "__main__":
    main()
