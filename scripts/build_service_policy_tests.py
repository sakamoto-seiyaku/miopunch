#!/usr/bin/env python3
"""Build the public service-policy tests against one verified pinned SDK.

No Cargo invocation, selector lock, alternate target or dependency compilation.
The SDK graph and test source must match before rustc starts. Runtime cases then
reuse the immutable test pin.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

from build_native_client import ROOT, RUSTC_VERSION, validate_inputs
from cargo_cache import cache_lease, disk_state
from pin_artifact import pin_artifact


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-receipt", type=Path, required=True)
    parser.add_argument("--test-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    receipt = args.native_receipt.resolve()
    build = json.loads(receipt.read_text())
    expected = json.loads(receipt.with_name("inputs.json").read_text())
    test = ROOT / "tests/service_policy.rs"
    library = Path(build["sdkLibrary"]["pin"])
    sdk = Path(build["pin"])
    toolchain = subprocess.check_output(["rtk", "proxy", "rustc", "-Vv"], text=True)
    if not toolchain.startswith(RUSTC_VERSION):
        raise RuntimeError("test consumer requires declared rustc 1.98.1")
    artifacts = [json.loads(line) for line in receipt.with_name("cargo.jsonl").read_text().splitlines()]
    externs = {"miopunch_native_probe": library}
    selected_versions = {"rand": "0.10.3", "sha2": "0.10.9"}
    for name in ["p2panda_core", "p2panda_auth", "serde_json", "iroh", "tokio", "futures_util",
                 "sha2", "hex", "rand", "tokio_util", "n0_future", "serde", "thiserror"]:
        artifact = next(a for a in artifacts if a.get("reason") == "compiler-artifact" and a["target"]["name"] == name
            and (name not in selected_versions or a["package_id"].endswith("@" + selected_versions[name])))
        externs[name] = next(Path(name) for name in artifact["filenames"] if name.endswith(".rlib"))
    frozen = {str(path): sha(path) for path in externs.values()}
    controllers = {str(path): sha(path) for path in [Path(__file__).resolve(), ROOT / "scripts/build_native_client.py",
        ROOT / "scripts/cargo_cache.py", ROOT / "scripts/pin_artifact.py"]}

    def validate():
        validate_inputs(expected)
        if build["exit"] != 0 or sha(sdk) != build["sha256"] or sha(library) != build["sdkLibrary"]["sha256"]:
            raise RuntimeError("SDK artifact precondition failed")
        if sha(test) != args.test_sha:
            raise RuntimeError("test source precondition failed")
        for path, digest in frozen.items():
            if sha(Path(path)) != digest:
                raise RuntimeError("selected dependency changed: " + path)
        for path, digest in controllers.items():
            if sha(Path(path)) != digest:
                raise RuntimeError("test build controller changed: " + path)

    validate()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "service_policy.rs").write_bytes(test.read_bytes())
    for name in controllers:
        target = output / "controller-source" / Path(name).relative_to(ROOT)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(Path(name).read_bytes())
    report = dict(toolchain=toolchain, SDKReceipt=str(receipt), SDKLibrary=str(library),
        selectedDependencies=frozen, controllerInputs=controllers, testSHA256=args.test_sha, CargoInvoked=False, dependencyRebuilds=0,
        platform="x86_64-unknown-linux-gnu", profile="standalone test consumer with line tables", features=[],
        budgetBytes=256 * 1024**2, exit=None)
    try:
        with cache_lease(ROOT / "target") as before:
            validate()
            binary = ROOT / "target/.miopunch-test-consumers/service-policy-tests"
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
                raise RuntimeError("test compile failed; inspect retained rustc stderr")
            validate()
            report["pin"] = pin_artifact(binary, output / "service-policy-tests")
            report["sha256"] = sha(binary)
            report["diskAfter"] = disk_state(ROOT / "target")
    finally:
        (output / "build.json").write_text(json.dumps(report, indent=2, default=str) + "\n")
    print(json.dumps(report, indent=2, default=str))


if __name__ == "__main__":
    main()
