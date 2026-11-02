#!/usr/bin/env python3
"""Build the current demo with Cargo, or reuse explicitly supplied verified receipts."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from check_build_cache import CheckBuilds
from pin_artifact import sha256
import build_browser_client as browser_builder
from build_native_client import input_paths as native_inputs, validate_inputs as validate_native_inputs

OUTPUT = ROOT / ".artifacts/runs/client-demo"


def verify_map(entries):
    for name, digest in entries.items():
        path = Path(name)
        if not path.is_file() or sha256(path) != digest:
            raise RuntimeError("source/artifact precondition failed: " + str(path))


def read_browser(receipt):
    receipt = Path(receipt).resolve()
    record = json.loads(receipt.read_text())
    if record.get("exit") != 0 or not record.get("sourceFrozenDuringBuild") or record.get("bindgenExit") != 0:
        raise RuntimeError("browser build is not qualified: " + str(receipt))
    inputs = json.loads(receipt.with_name("inputs.json").read_text())
    if browser_builder.input_paths() - inputs.keys():
        raise RuntimeError("browser receipt omits current source/manifest inputs")
    verify_map({str(ROOT / name): digest for name, digest in inputs.items()})
    package = receipt.parent / "pkg"
    artifacts = {str(package / name): digest for name, digest in record["artifacts"].items()}
    verify_map(artifacts)
    return dict(originatingReceipt=str(receipt), artifacts=artifacts, pkg=str(package))


def read_native(receipt):
    receipt = Path(receipt).resolve()
    record = json.loads(receipt.read_text())
    if record.get("exit") != 0 or not {"miopunch_demo", "browser-probe-native"} <= record.get("programs", {}).keys():
        raise RuntimeError("native demo receipt lacks the required programs")
    validate_native_inputs(record["inputs"])
    verify_map(record["artifacts"])
    return record


def build(args):
    # Validate explicit reuse and the bindgen tool before invoking any compiler.
    browser = read_browser(args.browser_receipt) if args.browser_receipt else None
    native = read_native(args.native_receipt) if args.native_receipt else None
    bindgen = None
    if browser is None:
        selected = args.wasm_bindgen or browser_builder.default_bindgen()
        if not selected:
            raise RuntimeError("provide --browser-receipt or an installed --wasm-bindgen 0.2.129")
        bindgen = Path(selected).resolve()
        if subprocess.check_output(["rtk", "proxy", str(bindgen), "--version"], text=True).strip() != "wasm-bindgen 0.2.129":
            raise RuntimeError("demo requires wasm-bindgen 0.2.129")
    output = args.output.resolve()
    run = output / "builds" / str(time.time_ns())
    run.mkdir(parents=True, exist_ok=False)
    if browser is None:
        inputs = {name: sha256(ROOT / name) for name in sorted(browser_builder.input_paths())}
        selected = run / "browser-inputs.json"
        selected.write_text(json.dumps(inputs, indent=2) + "\n")
        browser_output = ROOT / ".artifacts/runs/client-demo-browser" / run.name
        command = ["rtk", "proxy", sys.executable, "-B", str(ROOT / "scripts/build_browser_client.py"),
                   "--inputs", str(selected), "--output", str(browser_output),
                   "--wasm-bindgen", str(bindgen), "--wasm-bindgen-sha", sha256(bindgen)]
        with (run / "browser.log").open("w") as log:
            subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        browser = read_browser(browser_output / "build.json")
    if native is None:
        builds = CheckBuilds(run / "native")
        command = ["cargo", "build", "--locked", "--offline", "--features", "rtc-probe",
                   "--bin", "miopunch_demo", "--bin", "browser-probe-native"]
        with (run / "native.log").open("w") as log:
            builds.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        built = next(iter(builds.built.values()))
        original = json.loads(Path(built["receipt"]).read_text())
        inputs = json.loads(Path(built["receipt"]).with_name("inputs.json").read_text())
        inputs = {name: inputs[name] for name in native_inputs() | {"scripts/check_build_cache.py"}}
        programs = {name: builds.path(ROOT / "target/debug" / name)
                    for name in ("miopunch_demo", "browser-probe-native")}
        receipt = run / "native.json"
        native = dict(exit=0, originatingReceipt=str(receipt), buildReceipt=built["receipt"],
                      programs=programs, artifacts=built["pins"], inputs=inputs,
                      freshArtifacts=original["freshArtifacts"], rebuiltArtifacts=original["rebuiltArtifacts"])
        receipt.write_text(json.dumps(native, indent=2) + "\n")
    relay = native["programs"]["browser-probe-native"]
    result = dict(browser=browser, native=native,
                  relay=dict(path=relay, sha256=native["artifacts"][relay], originatingReceipt=native["originatingReceipt"]))
    (run / "artifacts.json").write_text(json.dumps(result, indent=2) + "\n")
    (output / "artifacts.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--browser-receipt", type=Path, help="Reuse a qualified build_browser_client build.json")
    parser.add_argument("--native-receipt", type=Path, help="Reuse this builder's verified native.json")
    parser.add_argument("--wasm-bindgen", help="Existing wasm-bindgen 0.2.129; needed when building the browser SDK")
    parser.add_argument("--rebuild-relay", action="store_true", help="Compile the current relay with the demo; Cargo reuses unchanged dependencies")
    args = parser.parse_args()
    if args.rebuild_relay and args.native_receipt:
        parser.error("--rebuild-relay cannot reuse --native-receipt")
    result = build(args)
    print(json.dumps(dict(nativeReceipt=result["native"]["originatingReceipt"], browser=result["browser"]["pkg"])))


if __name__ == "__main__":
    main()
