#!/usr/bin/env python3
"""Compile one public device consumer against a verified immutable SDK graph.

Never invokes Cargo or resolves dependencies. A frozen prior SDK can reproduce
an original RED while subsequent consumers use the corrected immutable graph.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import time

from build_native_client import ROOT, RUSTC_VERSION
from cargo_cache import cache_lease, disk_state
from pin_artifact import pin_artifact


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-receipt", type=Path, required=True)
    parser.add_argument("--source-archive-sha", required=True)
    parser.add_argument("--test", choices=["device_replacement", "device_operations", "clipboard_lifecycle", "clipboard_guest_projection", "forwarding_lifecycle", "web_lifecycle"], required=True)
    parser.add_argument("--test-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    receipt = args.native_receipt.resolve()
    sdk = json.loads(receipt.read_text())
    expected = json.loads(receipt.with_name("inputs.json").read_text())
    archive = receipt.with_name("source.tar.gz")
    test = ROOT / "tests" / (args.test + ".rs")
    library = Path(sdk["sdkLibrary"]["pin"])
    artifacts = [json.loads(line) for line in receipt.with_name("cargo.jsonl").read_text().splitlines()]
    externs = {"miopunch_native_probe": library}
    for name in ["p2panda_core", "p2panda_auth", "tokio", "serde_json", "iroh"]:
        artifact = next(item for item in artifacts if item.get("reason") == "compiler-artifact" and item["target"]["name"] == name)
        externs[name] = next(Path(path) for path in artifact["filenames"] if path.endswith(".rlib"))
    dependencies = {str(path): sha(path) for path in externs.values()}
    controllers = {str(path): sha(path) for path in [Path(__file__).resolve(), ROOT / "scripts/build_native_client.py",
        ROOT / "scripts/cargo_cache.py", ROOT / "scripts/pin_artifact.py"]}
    toolchain = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)

    def validate():
        if not toolchain.startswith(RUSTC_VERSION) or sdk["exit"] != 0:
            raise RuntimeError("SDK/toolchain precondition failed")
        if sha(archive) != args.source_archive_sha or sha(test) != args.test_sha:
            raise RuntimeError("frozen SDK archive or test source precondition failed")
        with tarfile.open(archive, "r:gz") as source:
            files = {entry.name: entry for entry in source.getmembers() if entry.isfile()}
            if files.keys() != expected.keys():
                raise RuntimeError("SDK archive does not match its graph")
            for name, digest in expected.items():
                if hashlib.sha256(source.extractfile(files[name]).read()).hexdigest() != digest:
                    raise RuntimeError("SDK source precondition failed: " + name)
        if sha(Path(sdk["pin"])) != sdk["sha256"] or sha(library) != sdk["sdkLibrary"]["sha256"]:
            raise RuntimeError("SDK pin changed")
        for name, digest in {**dependencies, **controllers}.items():
            if sha(Path(name)) != digest:
                raise RuntimeError("selected dependency/controller changed: " + name)

    validate()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / test.name).write_bytes(test.read_bytes())
    for name in controllers:
        target = output / "controller-source" / Path(name).relative_to(ROOT)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(Path(name).read_bytes())
    report = dict(toolchain=toolchain, SDKReceipt=str(receipt), SDKLibrary=str(library),
        SDKSourceArchive=str(archive), SDKSourceArchiveSHA256=args.source_archive_sha,
        selectedDependencies=dependencies, controllerInputs=controllers, testSHA256=args.test_sha,
        CargoInvoked=False, dependencyRebuilds=0, platform="x86_64-unknown-linux-gnu",
        profile="standalone line tables", features=[], budgetBytes=256 * 1024**2, exit=None)
    try:
        with cache_lease(ROOT / "target") as before:
            validate()
            binary = ROOT / "target/.miopunch-test-consumers" / (args.test.replace("_", "-") + "-tests")
            binary.parent.mkdir(parents=True, exist_ok=True)
            command = ["rtk", "proxy", "rustc", "--edition=2024", "--test", "-C", "debuginfo=line-tables-only",
                str(test), "-L", "dependency=" + str(ROOT / "target/debug/deps"), "-o", str(binary)]
            for name, path in externs.items():
                command += ["--extern", name + "=" + str(path)]
            report.update(command=command, diskBefore=before)
            started = time.monotonic()
            with (output / "rustc.stdout").open("w") as stdout, (output / "rustc.stderr").open("w") as stderr:
                result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
            report.update(exit=result.returncode, seconds=time.monotonic() - started)
            if result.returncode:
                raise RuntimeError("device consumer compile failed; inspect retained stderr")
            validate()
            report["pin"] = pin_artifact(binary, output / binary.name)
            report["sha256"] = sha(binary)
            report["diskAfter"] = disk_state(ROOT / "target")
    finally:
        (output / "build.json").write_text(json.dumps(report, indent=2, default=str) + "\n")
    print(json.dumps(report, indent=2, default=str))


if __name__ == "__main__":
    main()
