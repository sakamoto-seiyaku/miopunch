#!/usr/bin/env python3
"""Compile public typed-admission consumers against one verified Cargo build.

The positive consumer must compile first. Each negative consumer must then fail
with its expected compiler diagnostic, rather than an unavailable dependency.
This runs rustc against the build's actual iroh rlib, within the shared cache
lease; it neither creates another Cargo target nor resolves a lockfile.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

import cargo_cache

ROOT = Path(__file__).resolve().parents[1]
CONSUMERS = ROOT / "tests/restricted_endpoint_consumers"
CASES = {
    "ordinary_handoff": None,
    "endpoint_conversion": "E0277",
    "endpoint_borrow": "E0599",
    "builder_conversion": "E0277",
    "incoming_conversion": "E0277",
    "accepting_conversion": "E0277",
    "outgoing_early_data": "E0599",
    "incoming_early_data": "E0599",
    "control_outgoing_early_data": "E0599",
    "control_incoming_early_data": "E0599",
    "control_conversion": "E0277",
    "control_borrow": "E0599",
    "control_stream": "E0599",
    "control_datagram": "E0599",
    "weak_upgrade": "E0599",
    "router_endpoint": "E0308",
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", type=Path, required=True,
                        help="build directory containing Cargo JSON, receipt and adjacent frozen context")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native", action="store_true", help="also check that Native member links reject control capabilities")
    args = parser.parse_args()
    cases = dict(CASES)
    if args.native:
        cases.update(native_member_links=None, native_member_links_control="E0308")
    build = args.build.resolve()
    context_path = build.with_name(build.name + "-context.json")
    receipt_path = build / "receipt.json"
    context = json.loads(context_path.read_text())
    receipt = json.loads(receipt_path.read_text())
    assert receipt["exit"] == 0 and receipt["qualifiedForFrozenRun"] and receipt["sourceFrozenDuringBuild"]
    records = [json.loads(line) for line in (build / "cargo.jsonl").read_text().splitlines() if line.startswith("{")]
    rlibs = {Path(name) for row in records if row.get("reason") == "compiler-artifact"
             and row["target"]["name"] == "iroh" and "lib" in row["target"]["kind"]
             for name in row["filenames"] if name.endswith(".rlib")}
    assert len(rlibs) == 1, "one selected iroh library required"
    rlib = rlibs.pop()
    native_library = None
    if args.native:
        native_rlibs = {Path(name) for row in records if row.get("reason")=="compiler-artifact"
                        and row["target"]["name"]=="miopunch_native_probe"
                        and any(kind in row["target"]["kind"] for kind in ("lib", "rlib"))
                        for name in row["filenames"] if name.endswith(".rlib")}
        assert len(native_rlibs)==1, "one selected Native library required"
        native_library=native_rlibs.pop()
    inputs = {str(ROOT / path): digest for path, digest in context["sources"].items()}
    inputs.update(context["selectorPins"])
    inputs.update(context["artifactPins"])
    inputs.update(context.get("fixedInputs", {}))
    inputs.update(context["tools"]["binaries"])
    inputs.update(receipt["artifacts"])
    inputs.update({str(CONSUMERS / (name + ".rs")): sha(CONSUMERS / (name + ".rs")) for name in cases})
    inputs.update({str(rlib): sha(rlib), str(context_path): sha(context_path),
                   str(receipt_path): sha(receipt_path), str(Path(__file__).resolve()): sha(__file__)})

    if native_library is not None:
        inputs[str(native_library)]=sha(native_library)

    def guard():
        assert Path.cwd().resolve() == ROOT, "repository-root CWD required"
        assert all(sha(path) == digest for path, digest in inputs.items()), "source/lock/artifact/tool precondition failed"

    guard()
    target = cargo_cache.target_directory(ROOT)
    assert rlib.parent == target / "debug/deps", "expected native shared-cache library"
    output = args.output.resolve()
    results = []
    with cargo_cache.cache_lease(target):
        guard()
        output.mkdir(parents=True, exist_ok=False)
        pinned = output / "pinned"
        pinned.mkdir()
        library = pinned / rlib.name
        shutil.copy2(rlib, library)
        assert sha(library) == inputs[str(rlib)]
        native_copy=None
        if native_library is not None:
            assert native_library.parent==rlib.parent
            native_copy=pinned/native_library.name
            shutil.copy2(native_library,native_copy)
            assert sha(native_copy)==inputs[str(native_library)]
        for name, expected in cases.items():
            guard()
            assert sha(library) == inputs[str(rlib)]
            out = output / name
            out.mkdir()
            command = ["rtk", "proxy", context["tools"]["stableToolchain"]["rustc"],
                       "--edition=2024", "--crate-type=lib", "--emit=metadata",
                       "--error-format=json", "--extern", f"iroh={library}", "-L", f"dependency={rlib.parent}",
                       "--out-dir", str(out), str(CONSUMERS / (name + ".rs"))]
            if native_copy is not None:
                assert sha(native_copy)==inputs[str(native_library)]
                command.extend(["--extern", f"miopunch_native_probe={native_copy}"])
            with (out / "stdout.log").open("w") as stdout, (out / "stderr.jsonl").open("w") as stderr:
                result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=60)
            diagnostics = []
            for line in (out / "stderr.jsonl").read_text().splitlines():
                try:
                    diagnostics.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
            codes = {row["code"]["code"] for row in diagnostics if row.get("level") == "error" and row.get("code")}
            passed = result.returncode == 0 if expected is None else result.returncode != 0 and codes == {expected}
            results.append({"consumer": name, "command": command, "exit": result.returncode,
                            "expectedError": expected, "actualErrorCodes": sorted(codes), "passed": passed,
                            "stderrSha256": sha(out / "stderr.jsonl")})
            if not passed:
                break
        guard()
        report = {"buildReceiptSha256": sha(receipt_path), "contextSha256": sha(context_path),
                  "librarySha256": sha(library), "inputs": inputs, "cases": results,
                  "passed": len(results) == len(cases) and all(row["passed"] for row in results)}
        (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "caseCount": len(results),
                      "failures": [row for row in results if not row["passed"]],
                      "receipt": str(output / "receipt.json")}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
