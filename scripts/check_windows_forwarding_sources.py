#!/usr/bin/env python3
"""Parse/compile the prepared Windows socket host locally; never invoke it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from check_forwarding import isolated_environment
from forwarding_local_resources import OwnedProcess
from pin_artifact import pin_artifact

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pwsh", type=Path, required=True)
    parser.add_argument("--pwsh-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source = ROOT / "scripts"
    names = ["windows_forwarding_host.ps1", "windows_forwarding_sockets.cs", "check_windows_forwarding_sources.py",
             "check_windows_clipboard_sources.ps1", "forwarding_local_resources.py", "check_forwarding.py", "pin_artifact.py"]
    expected = {name: sha(source / name) for name in names}
    assert sha(args.pwsh) == args.pwsh_sha and os.access(args.pwsh, os.X_OK)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "inputs.json").write_text(json.dumps(expected, indent=2) + "\n")
    environment = dict(isolated_environment(), POWERSHELL_TELEMETRY_OPTOUT="1", XDG_CACHE_HOME=str(output / "cache"),
                       PSModuleAnalysisCachePath=str(output / "cache/module-analysis"), DOTNET_CLI_HOME=str(output / "dotnet"))
    for key in ["LD_PRELOAD", "FAKETIME", "DOTNET_STARTUP_HOOKS", "DOTNET_ADDITIONAL_DEPS"]:
        environment.pop(key, None)
    command = [str(args.pwsh.resolve()), "-NoLogo", "-NoProfile", "-NonInteractive", "-File",
               str(source / "check_windows_clipboard_sources.ps1"), "-HostScript", str(source / "windows_forwarding_host.ps1"),
               "-Helper", str(source / "windows_forwarding_sockets.cs"), "-Output", str(output)]
    started = time.monotonic()
    with (output / "stdout").open("w") as stdout, (output / "stderr").open("w") as stderr:
        process = subprocess.Popen(command, env=environment, stdout=stdout, stderr=stderr)
        owner = OwnedProcess(process, output / "owner.jsonl")
        try:
            process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            owner.send(signal.SIGTERM)
            process.wait(timeout=5)
        finally:
            if process.poll() is not None:
                owner.close()
    drift = [name for name, digest in expected.items() if sha(source / name) != digest]
    (output / "exit.json").write_text(json.dumps({"exit": process.returncode, "command": command,
                 "source_drift": drift, "seconds": time.monotonic() - started, "host_executed": False,
                 "socket_helper_invoked": False, "Windows_execution": False}, indent=2) + "\n")
    assert process.returncode == 0 and not drift, "static Windows host qualification failed; original output retained"
    report = json.loads((output / "result.json").read_text())
    assert report["csharpCompiled"] and not report["powerShellParseErrors"] and not report["hostScriptExecuted"]
    pin_artifact(output / "helper.dll", output / "pinned/helper.dll")
    report.update(socket_helper_invoked=False, Windows_execution=False, managed_assembly_pin=str(output / "pinned/helper.dll"),
                  managed_assembly_sha256=sha(output / "pinned/helper.dll"))
    (output / "qualified.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"local_parse_and_CSharp5_compile": "pass", "host_executed": False, "Windows_execution": False}))


if __name__ == "__main__":
    main()
