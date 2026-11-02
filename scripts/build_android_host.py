#!/usr/bin/env python3
"""Build one frozen Android JNI graph with the shared cache and stable NDK tools."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import time

from build_android_pair import compiler
from build_native_client import ROOT, RUSTC_VERSION, capture_depfiles, validate_inputs
from cargo_cache import build_environment, cache_lease, cargo_command, disk_state
from pin_artifact import pin_artifact


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["inputs", "ndk", "plan", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    expected = json.loads(args.inputs.read_text())
    plan = json.loads(args.plan.read_text())
    plan_hash = sha(args.plan)
    assert sha(args.inputs) == plan["source_manifest_sha256"]
    toolchain = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)
    assert toolchain.startswith(RUSTC_VERSION)
    clang, archiver = compiler(args.ndk.resolve(), args.output)
    tools = {str(path): sha(path) for path in [clang, archiver, args.ndk / "source.properties", Path(__file__).resolve(), ROOT / "scripts/build_android_pair.py"]}

    def validate():
        assert sha(args.plan) == plan_hash
        validate_inputs(expected)
        for name, digest in tools.items():
            assert sha(name) == digest, name
        used = sum(path.stat().st_blocks * 512 for path in (ROOT / ".artifacts/sha256").rglob("*") if path.is_file())
        assert used + plan["reserved_artifact_bytes"] < plan["store_cap"]

    validate()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "inputs.json").write_text(json.dumps(expected, indent=2) + "\n")
    with tarfile.open(output / "source.tar.gz", "w:gz") as archive:
        for name in sorted(expected):
            archive.add(ROOT / name, arcname=name)
    report = dict(platform="aarch64-linux-android", profile="dev", features=[], toolchain=toolchain,
                  plan=str(args.plan), plan_sha256=plan_hash, tool_inputs=tools, exit=None)
    try:
        with cache_lease(ROOT / "target") as before:
            validate()
            environment = build_environment(ROOT / "target")
            environment.update(CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=str(clang),
                               CC_aarch64_linux_android=str(clang), AR_aarch64_linux_android=str(archiver))
            command = cargo_command(["rustc", "--locked", "--offline", "--lib", "--crate-type", "cdylib",
                                     "--target", "aarch64-linux-android", "--jobs", "2"], ROOT / "target")
            report.update(command=command, diskBefore=before)
            started = time.monotonic()
            with (output / "cargo.jsonl").open("w") as stdout, (output / "cargo.stderr").open("w") as stderr:
                result = subprocess.run(command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr)
            report.update(exit=result.returncode, seconds=time.monotonic() - started)
            messages = [json.loads(line) for line in (output / "cargo.jsonl").read_text().splitlines()]
            artifacts = [item for item in messages if item.get("reason") == "compiler-artifact"]
            report.update(freshArtifacts=sum(item["fresh"] for item in artifacts),
                          rebuiltArtifacts=sum(not item["fresh"] for item in artifacts),
                          rebuiltTargets=[item["target"]["name"] for item in artifacts if not item["fresh"]])
            if result.returncode:
                raise RuntimeError("Android JNI build failed; original logs retained")
            validate()
            library = next(Path(name) for item in artifacts if item["target"]["name"] == "miopunch_native_probe"
                           for name in item["filenames"] if name.endswith(".so"))
            selected = capture_depfiles(artifacts, output, expected, ROOT / "target/aarch64-linux-android/debug/libmiopunch_native_probe.d")
            (output / "selected-dependency-inputs.json").write_text(json.dumps(selected, indent=2) + "\n")
            assert library.stat().st_size <= plan["reserved_artifact_bytes"]
            pin = pin_artifact(library, ROOT / ".artifacts/runs" / output.name / library.name, max_bytes=plan["store_cap"])
            report.update(pin=str(pin), sha256=sha(pin), bytes=pin.stat().st_size, diskAfter=disk_state(ROOT / "target"))
    finally:
        (output / "build.json").write_text(json.dumps(report, indent=2, default=str) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
