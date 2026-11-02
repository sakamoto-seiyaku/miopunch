#!/usr/bin/env python3
"""Compile the typed Windows host against pinned SDK/reference metadata only."""
import argparse
import hashlib
import json
from pathlib import Path

from check_forwarding import isolated_environment
from pin_artifact import pin_artifact
from web_process_owner import run_process

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pwsh",type=Path,required=True)
    parser.add_argument("--pwsh-sha",required=True)
    parser.add_argument("--sdk-selected",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--finalize-existing",action="store_true",help="Finalize a verified successful compile after receipt-only failure; do not invoke compiler")
    args = parser.parse_args()
    if args.finalize_existing:
        output = args.output.resolve()
        graph = json.loads((output/"inputs.json").read_text())
        script = Path(__file__).resolve()
        for item in graph["files"]:
            path = Path(item["path"])
            if path == script:
                path = output/"controller-source/scripts"/script.name
            assert sha(path) == item["sha256"], "original compile input differs"
        checks = json.loads((output/"compiler-result.json").read_text(encoding="utf-8-sig"))
        assert checks["csharpCompiled"] and not checks["compileErrors"] and not checks["powerShellParseErrors"]
        assert not checks["WindowsExecuted"] and not checks["nativeMethodsInvoked"]
        rows = [json.loads(line) for line in (output/"owner.jsonl").read_text().splitlines()]
        assert any(row.get("phase")=="ordinary_cleanup" and row.get("exit")==0 and not row.get("errors") for row in rows)
        library = output/"pinned/Miopunch.WindowsWebContentHost.dll"
        assert sha(library)==sha(output/"Miopunch.WindowsWebContentHost.dll"), "compiled host pin differs"
        report = {"exit":0,"pin":str(library),"sha256":sha(library),"allOwnedTerminal":True,
                  "WindowsExecuted":False,"hostAssemblyLoaded":False,"WebView2Instantiated":False,"CargoInvoked":False,
                  "compilerReinvokedForFinalization":False,"originalControllerSource":"controller-source/scripts/"+script.name,
                  "finalizerSHA256":sha(script),"originalPackagingFailure":"packaging-failure.json"}
        (output/"build.json").write_text(json.dumps(report,indent=2)+"\n")
        print(json.dumps(report))
        return
    assert sha(args.pwsh) == args.pwsh_sha
    selected = json.loads(args.sdk_selected.read_text())
    assert all(sha(item["path"]) == item["sha256"] for item in selected), "SDK/reference pin changed"
    references = [item["path"] for item in selected if item["path"].endswith(".dll") and not item["path"].endswith("WebView2Loader.dll")]
    source = ROOT/"scripts/windows_web_content_host.cs"
    wrapper = ROOT/"scripts/windows_web_content_host.ps1"
    roslyn = [args.pwsh.resolve().parent/name for name in ["Microsoft.CodeAnalysis.dll","Microsoft.CodeAnalysis.CSharp.dll"]]
    paths = [source, wrapper, Path(__file__).resolve(), ROOT/"scripts/compile_windows_web_host.ps1", args.pwsh.resolve(),
             args.sdk_selected.resolve(), ROOT/"scripts/web_process_owner.py", ROOT/"scripts/forwarding_local_resources.py",
             ROOT/"scripts/pin_artifact.py", *roslyn, *(Path(path) for path in references)]
    files = [{"path":str(path),"sha256":sha(path)} for path in paths]
    output = args.output.resolve(); output.mkdir(parents=True,exist_ok=False)
    graph = {"source":str(source),"wrapper":str(wrapper),"roslynCore":str(roslyn[0]),"roslynCSharp":str(roslyn[1]),
             "references":references,"files":files,"CargoInvoked":False,"WindowsExecuted":False}
    (output/"inputs.json").write_text(json.dumps(graph,indent=2)+"\n")
    (output/source.name).write_bytes(source.read_bytes()); (output/wrapper.name).write_bytes(wrapper.read_bytes())
    environment = isolated_environment()
    environment.update(POWERSHELL_TELEMETRY_OPTOUT="1",XDG_CACHE_HOME=str(output/"cache"),
                       PSModuleAnalysisCachePath=str(output/"cache/module-analysis"),DOTNET_CLI_HOME=str(output/"dotnet"))
    owners = []
    report = {"exit":None,"WindowsExecuted":False,"hostAssemblyLoaded":False,"WebView2Instantiated":False,"CargoInvoked":False}
    try:
        assert all(sha(item["path"]) == item["sha256"] for item in files), "compile input changed before invocation"
        result = run_process([str(args.pwsh.resolve()),"-NoLogo","-NoProfile","-NonInteractive","-File",
            str(ROOT/"scripts/compile_windows_web_host.ps1"),"-Inputs",str(output/"inputs.json"),"-Output",str(output)],
            env=environment,receipt=output/"owner.jsonl",timeout=60,owners=owners)
        (output/"stdout").write_text(result.stdout); (output/"stderr").write_text(result.stderr)
        report["exit"] = result.returncode
        report["sourceDrift"] = [item["path"] for item in files if sha(item["path"]) != item["sha256"]]
        assert result.returncode == 0 and not report["sourceDrift"], "typed host compile failed; original diagnostic retained"
        checks = json.loads((output/"compiler-result.json").read_text(encoding="utf-8-sig"))
        assert checks["csharpCompiled"] and not checks["powerShellParseErrors"] and not checks["nativeMethodsInvoked"]
        assert (output/"Miopunch.WindowsWebContentHost.dll").stat().st_size <= 1024 * 1024, "managed host exceeds declared 1MiB pin allowance"
        report["pin"] = str(pin_artifact(output/"Miopunch.WindowsWebContentHost.dll",output/"pinned/Miopunch.WindowsWebContentHost.dll"))
        report["sha256"] = sha(report["pin"])
    finally:
        report["allOwnedTerminal"] = all(owner.terminal() for owner in owners)
        (output/"build.json").write_text(json.dumps(report,indent=2)+"\n")
        for owner in owners: owner.close()
    print(json.dumps(report))


if __name__ == "__main__":
    main()
