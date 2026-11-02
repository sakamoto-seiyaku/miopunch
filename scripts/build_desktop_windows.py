#!/usr/bin/env python3
"""Build the Tauri Windows GNU graph with the existing stable SDK cross tools."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import time
import tomllib

from build_native_client import IMAGE
from build_desktop_host import ROOT, APP, GENERATED, TOOLCHAIN, inputs as desktop_inputs, capture_desktop_depfiles, sha
from cargo_cache import build_environment, cache_lease, cargo_command, disk_state
from pin_artifact import pin_artifact


def inputs():
    result = desktop_inputs()
    result["scripts/build_desktop_windows.py"] = sha(Path(__file__).resolve())
    return result


def validate(expected, plan):
    if inputs() != expected:
        raise RuntimeError("Windows desktop source precondition failed")
    if sha(ROOT / "Cargo.lock") != plan["sdk_lock_sha256"] or sha(APP / "Cargo.lock") != plan["desktop_lock_sha256"]:
        raise RuntimeError("Windows desktop lock precondition failed")
    version = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)
    if version != plan["rustc"]:
        raise RuntimeError("Windows desktop Rust toolchain changed")
    image = subprocess.check_output(["rtk", "proxy", "docker", "image", "inspect", IMAGE, "--format", "{{.Id}}"], text=True).strip()
    if image != IMAGE or image != plan["compiler_image"]:
        raise RuntimeError("Windows compiler image changed")
    loader = Path(plan["loader_path"])
    package = next(p for p in tomllib.loads((APP / "Cargo.lock").read_text())["package"] if p["name"] == "webview2-com-sys")
    archive = Path(plan["loader_crate_archive"])
    if sha(archive) != package["checksum"]:
        raise RuntimeError("WebView2 crate archive differs from the product lock")
    with tarfile.open(archive, "r:gz") as contents:
        original = contents.extractfile(f"webview2-com-sys-{package['version']}/x64/WebView2Loader.dll").read()
    if sha(loader) != hashlib.sha256(original).hexdigest() or sha(loader) != plan["loader_sha256"]:
        raise RuntimeError("pinned WebView2 loader precondition failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["inputs", "plan", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    expected, plan = json.loads(args.inputs.read_text()), json.loads(args.plan.read_text())
    if sha(args.inputs) != plan["source_manifest_sha256"] or not plan["decision"]:
        raise RuntimeError("unreviewed Windows desktop graph")
    validate(expected, plan)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "inputs.json").write_text(json.dumps(expected, indent=2) + "\n")
    (output / "plan.json").write_bytes(args.plan.read_bytes())
    with tarfile.open(output / "source.tar.gz", "w:gz") as archive:
        for name in sorted(expected):
            archive.add(ROOT / name, arcname=name)
    target = ROOT / "target"
    environment = build_environment(target)
    report = {"exit":None,"platform":"x86_64-pc-windows-gnu","profile":"dev","features":[],"compilerImage":IMAGE,
        "toolchain":plan["rustc"],"sourceArchiveSHA256":sha(output / "source.tar.gz"),"storeBudgetBytes":plan["store_cap"]}
    container = None
    try:
        with cache_lease(target) as before:
            validate(expected, plan)
            used = sum(p.stat().st_blocks * 512 for p in (ROOT / ".artifacts/sha256").rglob("*") if p.is_file())
            if used != plan["allocated_before"] or used + plan["pin_budget"] > plan["store_cap"]:
                raise RuntimeError("Windows retention precondition changed")
            report["diskBefore"] = before
            tools = target / ".miopunch-toolchains/windows" / IMAGE.removeprefix("sha256:")
            tools.mkdir(parents=True, exist_ok=True)
            name = "miopunch-client-compiler-" + IMAGE[7:19]
            launch = ["rtk","proxy","docker","run","--rm","-d","--name",name,"--network","none","--user",f"{os.getuid()}:{os.getgid()}",
                "-v",f"{ROOT}:{ROOT}:ro","-v",f"{target}:{target}",
                "-v",f"{Path.home() / '.cargo/registry'}:{Path.home() / '.cargo/registry'}:ro",
                "-v",f"{TOOLCHAIN}:{TOOLCHAIN}:ro","-v","/tmp:/tmp","-w",str(ROOT),IMAGE,"sleep","infinity"]
            container = subprocess.check_output(launch, text=True).strip()
            report.update(compilerContainer=container, compilerContainerCommand=launch)
            for tool in ["gcc","ar","dlltool","windres"]:
                path = tools / ("x86_64-w64-mingw32-" + tool)
                content = '#!/bin/sh\nexec rtk proxy docker exec -i -w "$PWD" ' + shlex.quote(name) + ' ' + path.name + ' "$@"\n'
                if path.exists() and path.read_text() != content:
                    raise RuntimeError("stable Windows compiler wrapper changed")
                if not path.exists():
                    path.write_text(content)
                    path.chmod(0o700)
            environment.update(CC_x86_64_pc_windows_gnu=str(tools / "x86_64-w64-mingw32-gcc"),
                AR_x86_64_pc_windows_gnu=str(tools / "x86_64-w64-mingw32-ar"),
                RC_x86_64_pc_windows_gnu=str(tools / "x86_64-w64-mingw32-windres"),
                CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=str(tools / "x86_64-w64-mingw32-gcc"))
            environment["PATH"] = str(tools) + os.pathsep + environment["PATH"]
            command = cargo_command(["build","--manifest-path","platform/desktop/Cargo.toml","--locked","--offline", "--target",report["platform"],"--bin","miopunch-desktop","--jobs","2"], target)
            report["command"] = command
            started = time.monotonic()
            with (output / "cargo.jsonl").open("w") as stdout, (output / "cargo.stderr.log").open("w") as stderr:
                completed = subprocess.run(command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr)
            report.update(exit=completed.returncode,seconds=time.monotonic()-started)
            artifacts = [json.loads(l) for l in (output / "cargo.jsonl").read_text().splitlines()]
            artifacts = [a for a in artifacts if a.get("reason") == "compiler-artifact"]
            report.update(freshArtifacts=sum(a["fresh"] for a in artifacts),rebuiltArtifacts=sum(not a["fresh"] for a in artifacts),
                rebuiltTargets=[{"package":a["package_id"],"target":a["target"]["name"]} for a in artifacts if not a["fresh"]])
            if completed.returncode:
                raise RuntimeError("Windows desktop Cargo failed; inspect preserved evidence")
            validate(expected, plan)
            generated = {p.relative_to(ROOT).as_posix():sha(p) for folder in GENERATED for p in folder.rglob("*") if p.is_file()}
            (output / "generated-output-hashes.json").write_text(json.dumps(generated,indent=2)+'\n')
            binary = target / report["platform"] / "debug/miopunch-desktop.exe"
            selected = capture_desktop_depfiles(artifacts,output,expected|generated,binary.with_suffix('.d'))
            (output / "selected-dependency-inputs.json").write_text(json.dumps(selected,indent=2)+'\n')
            loader = Path(plan["loader_path"])
            if binary.stat().st_size + loader.stat().st_size > plan["pin_budget"]:
                raise RuntimeError("Windows desktop graph exceeds reserved pin bytes")
            pins = ROOT / ".artifacts/runs" / output.name
            report.update(pin=str(pin_artifact(binary,pins/binary.name,max_bytes=plan["store_cap"])),sha256=sha(binary),
                loaderPin=str(pin_artifact(loader,pins/loader.name,max_bytes=plan["store_cap"])),loaderSHA256=sha(loader),diskAfter=disk_state(target))
            if report["diskAfter"]["cacheBytes"] - plan.get("cache_baseline_bytes", before["cacheBytes"]) > plan["cache_growth_budget"]:
                raise RuntimeError("Windows desktop cache exceeds the batch allowance")
    finally:
        if container:
            report["compilerCleanupExit"] = subprocess.run(["rtk","proxy","docker","stop","--time","10",container],stdout=subprocess.DEVNULL).returncode
        (output / "build.json").write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ["command","compilerContainerCommand","rebuiltTargets"]}))


if __name__ == "__main__":
    main()
