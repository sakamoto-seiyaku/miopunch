#!/usr/bin/env python3
"""Build a frozen native-client graph under the shared Cargo lease and pin it.

Inputs are a JSON mapping of repository-relative paths to SHA256, captured by
the caller before the batch. Every listed input must still match before Cargo.
Windows uses the existing image and stable, image-identified compiler wrappers.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import time
import tomllib

from cargo_cache import ROOT, build_environment, cache_lease, cargo_command, disk_state
from pin_artifact import pin_artifact

IMAGE = "sha256:af5de852d8a8ae263b8eaa6278aea87c3fedc9030d7eb51a0d1b2613c9ce4c4f"
RUSTC_VERSION = "rustc 1.98.1 "


def input_paths():
    """Conservatively freeze all local source/assets, not just selected .rs files.

    This also catches newly added modules, include_bytes assets and vendored
    build inputs before Cargo can observe a different graph. Cargo's offline
    locked registry graph remains governed by Cargo.lock checksums.
    """
    names = {"Cargo.toml", "Cargo.lock", ".cargo/config.toml",
             "scripts/build_native_client.py", "scripts/cargo_cache.py", "scripts/pin_artifact.py"}
    # Cargo reads workspace member manifests even when only the native package
    # is selected. Browser source is not part of this native compilation.
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    for member in manifest.get("workspace", {}).get("members", []):
        for directory in ROOT.glob(member):
            path = directory / "Cargo.toml"
            if not path.is_file():
                raise RuntimeError(f"workspace manifest missing: {path}")
            names.add(path.relative_to(ROOT).as_posix())
    for directory in [ROOT / "src", ROOT / "vendor"]:
        for path in directory.rglob("*"):
            relative = path.relative_to(ROOT)
            if path.is_file() and not {".git", "target", "__pycache__"}.intersection(relative.parts):
                names.add(relative.as_posix())
    for name in ["build.rs", "rust-toolchain", "rust-toolchain.toml"]:
        if (ROOT / name).exists():
            names.add(name)
    return names


def validate_inputs(expected):
    required = input_paths()
    missing = sorted(required - expected.keys())
    if missing:
        raise RuntimeError("frozen graph omits local source/build inputs: " + ", ".join(missing))
    for name, digest in expected.items():
        path = (ROOT / name).resolve()
        if not path.is_relative_to(ROOT) or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError(f"source precondition failed: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["linux", "windows"], required=True)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--validate-only", action="store_true", help="Check complete inputs/toolchain without Cargo or pinning")
    args = parser.parse_args()
    expected = json.loads(args.inputs.read_text())
    validate_inputs(expected)
    toolchain = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)
    if not toolchain.startswith(RUSTC_VERSION):
        raise RuntimeError("native client graph requires the declared rustc 1.98.1 toolchain")
    if args.validate_only:
        print(json.dumps({"validatedInputs": len(expected), "toolchain": toolchain, "cargoInvoked": False}))
        return
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "inputs.json").write_text(json.dumps(expected, indent=2) + "\n")
    target = ROOT / "target"
    environment = build_environment(target)
    container = None
    report = {"platform": args.platform, "profile": "dev", "features": [],
              "toolchain": toolchain,
              "exit": None}
    try:
        with cache_lease(target) as before:
            validate_inputs(expected)
            report["diskBefore"] = before
            arguments = ["build", "--locked", "--offline", "--bin", "miopunch-client", "--jobs", "2"]
            binary = target / "debug/miopunch-client"
            if args.platform == "windows":
                actual = subprocess.check_output(["rtk", "proxy", "docker", "image", "inspect",
                    "miopunch-desktop-compiler:bookworm", "--format", "{{.Id}}"], text=True).strip()
                if actual != IMAGE:
                    raise RuntimeError("Windows compiler image changed")
                tools = target / ".miopunch-toolchains" / "windows" / IMAGE.removeprefix("sha256:")
                tools.mkdir(parents=True, exist_ok=True)
                container = "miopunch-client-compiler-" + IMAGE[7:19]
                command = ["rtk", "proxy", "docker", "run", "--rm", "-d", "--name", container,
                    "--network", "none", "--user", f"{os.getuid()}:{os.getgid()}",
                    "-v", f"{ROOT}:{ROOT}", "-v", f"{Path.home()}:{Path.home()}:ro",
                    "-v", "/tmp:/tmp", "-w", str(ROOT), IMAGE, "sleep", "infinity"]
                subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
                for tool in ["gcc", "ar", "dlltool"]:
                    path = tools / ("x86_64-w64-mingw32-" + tool)
                    content = "#!/bin/sh\nexec rtk proxy docker exec -i -w \"$PWD\" " + shlex.quote(container) + " " + path.name + ' "$@"\n'
                    if path.exists() and path.read_text() != content:
                        raise RuntimeError("stable compiler wrapper changed")
                    if not path.exists():
                        path.write_text(content)
                        path.chmod(0o700)
                environment.update(CC_x86_64_pc_windows_gnu=str(tools / "x86_64-w64-mingw32-gcc"),
                    AR_x86_64_pc_windows_gnu=str(tools / "x86_64-w64-mingw32-ar"),
                    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=str(tools / "x86_64-w64-mingw32-gcc"))
                environment["PATH"] = str(tools) + os.pathsep + environment["PATH"]
                arguments += ["--target", "x86_64-pc-windows-gnu"]
                binary = target / "x86_64-pc-windows-gnu/debug/miopunch-client.exe"
                report["compilerImage"] = IMAGE
            command = cargo_command(arguments, target)
            report["command"] = command
            started = time.monotonic()
            with (args.output / "cargo.jsonl").open("w") as out, (args.output / "cargo.stderr.log").open("w") as err:
                result = subprocess.run(command, cwd=ROOT, env=environment, stdout=out, stderr=err)
            report.update(exit=result.returncode, seconds=time.monotonic() - started)
            artifacts = [json.loads(line) for line in (args.output / "cargo.jsonl").read_text().splitlines()]
            artifacts = [entry for entry in artifacts if entry.get("reason") == "compiler-artifact"]
            report.update(freshArtifacts=sum(entry["fresh"] for entry in artifacts),
                          rebuiltArtifacts=sum(not entry["fresh"] for entry in artifacts))
            if result.returncode:
                raise RuntimeError("Cargo failed; inspect raw logs before retrying")
            validate_inputs(expected)
            pin_path = ROOT / ".artifacts" / "runs" / args.output.name / binary.name
            report["pin"] = pin_artifact(binary, pin_path)
            report["sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
            report["diskAfter"] = disk_state(target)
    finally:
        if container:
            report["compilerCleanupExit"] = subprocess.run(["rtk", "proxy", "docker", "rm", "-f", container],
                stdout=subprocess.DEVNULL).returncode
        (args.output / "build.json").write_text(json.dumps(report, indent=2, default=str) + "\n")
    print(json.dumps(report, indent=2, default=str))


if __name__ == "__main__":
    main()
