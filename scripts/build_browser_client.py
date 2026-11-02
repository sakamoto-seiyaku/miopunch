#!/usr/bin/env python3
"""Build one frozen WASM client graph and pin its complete web package.

No runtime controller builds. Source, lock and standalone bindgen preconditions
are checked before Cargo and again before publishing immutable artifacts.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

from build_native_client import input_paths as native_inputs, capture_depfiles, RUSTC_VERSION
from cargo_cache import ROOT, build_environment, cache_lease, cargo_command, disk_state
from pin_artifact import pin_artifact

WASM_BINDGEN_VERSION = "0.2.129"
WASM_BINDGEN = ROOT / ".artifacts/tools/wasm-bindgen" / WASM_BINDGEN_VERSION / "wasm-bindgen"


def default_bindgen():
    """Prefer the project pin; PATH remains available for explicitly installed tools."""
    if WASM_BINDGEN.is_file():
        return str(WASM_BINDGEN)
    return shutil.which("wasm-bindgen") or str(WASM_BINDGEN)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def input_paths():
    names = native_inputs() | {"scripts/build_browser_client.py"}
    names.update(path.relative_to(ROOT).as_posix()
                 for path in (ROOT / "browser-probe/src").rglob("*") if path.is_file())
    return names


def validate(expected, bindgen, bindgen_sha):
    omitted = input_paths() - expected.keys()
    if omitted:
        raise RuntimeError("frozen browser graph omits inputs: " + ", ".join(sorted(omitted)))
    for name, digest in expected.items():
        path = (ROOT / name).resolve()
        if not path.is_relative_to(ROOT) or sha(path) != digest:
            raise RuntimeError("source precondition failed: " + name)
    if sha(bindgen) != bindgen_sha:
        raise RuntimeError("wasm-bindgen pin changed")
    toolchain = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)
    if not toolchain.startswith(RUSTC_VERSION):
        raise RuntimeError("browser graph requires the declared rustc 1.98.1")
    version = subprocess.check_output(["rtk", "proxy", str(bindgen), "--version"], text=True).strip()
    if version != "wasm-bindgen 0.2.129":
        raise RuntimeError("browser graph requires pinned wasm-bindgen 0.2.129")
    return toolchain, version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--wasm-bindgen", type=Path, default=default_bindgen())
    parser.add_argument("--wasm-bindgen-sha", required=True)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    expected = json.loads(args.inputs.read_text())
    bindgen = args.wasm_bindgen.resolve()
    toolchain, version = validate(expected, bindgen, args.wasm_bindgen_sha)
    if args.validate_only:
        print(json.dumps({"validatedInputs": len(expected), "toolchain": toolchain,
                          "bindgen": version, "cargoInvoked": False}))
        return
    output = args.output.resolve()
    if not output.is_relative_to(ROOT / ".artifacts/runs"):
        raise RuntimeError("browser receipts and pins require the persistent shared artifact filesystem")
    output.mkdir(parents=True, exist_ok=False)
    (output / "inputs.json").write_text(json.dumps(expected, indent=2) + "\n")
    with tarfile.open(output / "source.tar.gz", "w:gz") as archive:
        for name in sorted(expected):
            archive.add(ROOT / name, arcname=name)
    target = ROOT / "target"
    report = {"platform": "wasm32-unknown-unknown", "profile": "dev", "features": ["rtc-probe"],
              "toolchain": toolchain, "bindgen": version, "bindgenSHA256": args.wasm_bindgen_sha,
              "exit": None}
    try:
        with cache_lease(target) as before:
            validate(expected, bindgen, args.wasm_bindgen_sha)
            report["diskBefore"] = before
            arguments = ["build", "--locked", "--offline", "-p", "miopunch-browser-probe",
                         "--target", "wasm32-unknown-unknown", "--features", "rtc-probe", "--lib", "--jobs", "2"]
            command = cargo_command(arguments, target)
            report["command"] = command
            started = time.monotonic()
            with (output / "cargo.jsonl").open("w") as stdout, (output / "cargo.stderr.log").open("w") as stderr:
                result = subprocess.run(command, cwd=ROOT, env=build_environment(target), stdout=stdout, stderr=stderr)
            report.update(exit=result.returncode, seconds=time.monotonic() - started)
            artifacts = [json.loads(line) for line in (output / "cargo.jsonl").read_text().splitlines()]
            artifacts = [entry for entry in artifacts if entry.get("reason") == "compiler-artifact"]
            report.update(freshArtifacts=sum(entry["fresh"] for entry in artifacts),
                          rebuiltArtifacts=sum(not entry["fresh"] for entry in artifacts))
            if result.returncode:
                raise RuntimeError("Cargo failed; retained raw logs must be inspected before another build")
            validate(expected, bindgen, args.wasm_bindgen_sha)
            dependencies = capture_depfiles(artifacts, output, expected, target / "wasm32-unknown-unknown/debug/miopunch_browser_probe.d")
            (output / "selected-dependency-inputs.json").write_text(json.dumps(dependencies, indent=2) + "\n")
            report.update(selectedRepositoryInputCount=len(dependencies["selectedRepositoryInputs"]),
                          selectedDepfileCount=len(dependencies["depfiles"]))
            wasm = target / "wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm"
            report["compiledPin"] = pin_artifact(wasm, output / "compiled.wasm")
            generated = output / "generated"
            binding = ["rtk", "proxy", str(bindgen), "--target", "web", "--out-dir", str(generated), str(output / "compiled.wasm")]
            with (output / "bindgen.stdout.log").open("w") as stdout, (output / "bindgen.stderr.log").open("w") as stderr:
                result = subprocess.run(binding, cwd=ROOT, stdout=stdout, stderr=stderr)
            report.update(bindgenCommand=binding, bindgenExit=result.returncode)
            if result.returncode:
                raise RuntimeError("wasm-bindgen failed; inspect retained raw logs")
            validate(expected, bindgen, args.wasm_bindgen_sha)
            report["artifacts"] = {}
            for path in sorted(generated.rglob("*")):
                if path.is_file():
                    name = path.relative_to(generated)
                    report["artifacts"][name.as_posix()] = sha(path)
                    pin_artifact(path, output / "pkg" / name)
            report["packageSHA256"] = hashlib.sha256(json.dumps(report["artifacts"], sort_keys=True).encode()).hexdigest()
            report["sourceFrozenDuringBuild"] = True
            report["diskAfter"] = disk_state(target)
    finally:
        (output / "build.json").write_text(json.dumps(report, indent=2, default=str) + "\n")
    print(json.dumps(report, indent=2, default=str))


if __name__ == "__main__":
    main()
