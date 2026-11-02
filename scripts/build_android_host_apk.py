#!/usr/bin/env python3
"""Package the minimal real Service verification APK from a verified JNI pin.

This debug-only entry is separate from the later shared Tauri application.
No Cargo invocation, dependency resolution, device installation or key logging.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import struct
import subprocess
import zipfile

from build_native_client import ROOT
from pin_artifact import pin_artifact


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def resource_table_layout(path):
    """Android R+ maps the uncompressed, four-byte-aligned resource table."""
    with zipfile.ZipFile(path) as archive:
        assert archive.namelist().count("resources.arsc") == 1
        entry = archive.getinfo("resources.arsc")
        assert entry.compress_type == zipfile.ZIP_STORED, "compressed resource table is not installable"
        with Path(path).open("rb") as source:
            source.seek(entry.header_offset)
            header = source.read(30)
        assert len(header) == 30 and header[:4] == b"PK\x03\x04"
        assert struct.unpack_from("<H", header, 8)[0] == zipfile.ZIP_STORED
        name_bytes, extra_bytes = struct.unpack_from("<HH", header, 26)
        offset = entry.header_offset + 30 + name_bytes + extra_bytes
        assert offset % 4 == 0, "resource table is not four-byte aligned"
        return {"compression": "stored", "data_offset": offset, "alignment_bytes": 4,
                "sha256": hashlib.sha256(archive.read(entry)).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["library-receipt", "tool-receipt", "plan", "output", "signing-directory"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--reuse-native-deployment", type=Path, help="Prior successful APK build receipt for the exact same JNI pin")
    parser.add_argument("--reuse-platform-package", type=Path, help="Reuse verified unchanged Java DEX and binary manifest from a prior APK receipt")
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())
    library = json.loads(args.library_receipt.read_text())
    tool = json.loads(args.tool_receipt.read_text())
    assert library["exit"] == 0 and sha(library["pin"]) == library["sha256"]
    assert sha(tool["path"]) == tool["binary_sha256"]
    selected = json.loads(args.library_receipt.with_name("selected-dependency-inputs.json").read_text())["selectedRepositoryInputs"]
    for name, digest in selected.items():
        assert sha(ROOT / name) == digest, name
    assert sha(ROOT / "Cargo.lock") == plan["lock_sha256"]
    sources = {str(path.relative_to(ROOT)): sha(path) for path in (ROOT / "platform/android/host").rglob("*") if path.is_file()}
    sources[str(Path(__file__).resolve().relative_to(ROOT))] = sha(Path(__file__))
    assert sources == plan["sources"]
    used = sum(path.stat().st_blocks * 512 for path in (ROOT / ".artifacts/sha256").rglob("*") if path.is_file())
    assert used == plan["store_allocated_before"] and used + plan["apk_budget_bytes"] < plan["store_cap"]
    sdk = Path("/mnt/c/Android/SDK")
    android = sdk / "platforms/android-36/android.jar"
    d8 = sdk / "build-tools/36.1.0/lib/d8.jar"
    signer = sdk / "build-tools/36.1.0/lib/apksigner.jar"
    for path, digest in plan["tools"].items():
        assert sha(path) == digest, path
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    for name in sources:
        destination = output / "source" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes((ROOT / name).read_bytes())
    report = dict(scope="debug real-APK Service verification; not final Tauri/shared UI delivery", commands=[], sources=sources,
                  library_receipt=str(args.library_receipt), library_sha256=library["sha256"], tools=plan["tools"], exit=None)

    def run(label, command, env=None):
        with (output / (label + ".stdout")).open("w") as stdout, (output / (label + ".stderr")).open("w") as stderr:
            result = subprocess.run(["rtk", "proxy", *map(str, command)], cwd=ROOT, env=env, stdout=stdout, stderr=stderr)
        report["commands"].append(dict(label=label, command=list(map(str, command)), exit=result.returncode))
        if result.returncode:
            raise RuntimeError(label + " failed; original output retained")

    try:
        signing = args.signing_directory.resolve()
        signing.mkdir(mode=0o700, parents=True, exist_ok=True)
        password = signing / "password"
        key = signing / "verification.keystore"
        if not key.exists():
            with password.open("x") as stream:
                os.chmod(password, 0o600)
                stream.write(secrets.token_urlsafe(32))
            run("keytool", ["keytool", "-genkeypair", "-keystore", key, "-storepass:file", password,
                            "-keypass:file", password, "-alias", "verification", "-keyalg", "RSA", "-keysize", "2048",
                            "-validity", "3650", "-dname", "CN=miopunch host verification", "-noprompt"])
            key.chmod(0o600)
        unsigned = output / "unsigned.apk"
        if args.reuse_platform_package:
            previous = json.loads(args.reuse_platform_package.read_text())
            platform_sources = lambda values: {name: digest for name, digest in values.items()
                                              if name.startswith("platform/android/host/")}
            assert previous["exit"] == 0 and previous["tools"] == plan["tools"]
            assert platform_sources(previous["sources"]) == platform_sources(sources)
            assert sha(previous["pin"]) == previous["sha256"]
            with zipfile.ZipFile(previous["pin"]) as prior:
                # Keep aapt2's resource table even when no resource source
                # directory exists. Refuse an unreviewed package shape.
                entries = {name for name in prior.namelist()
                           if not name.startswith(("META-INF/", "lib/"))}
                assert entries == {"AndroidManifest.xml", "resources.arsc", "classes.dex"}, entries
                assert {name for name in prior.namelist() if name.startswith("lib/")} == {
                    "lib/arm64-v8a/libmiopunch_native_probe.so"}
                reused = {name: prior.read(name) for name in sorted(entries)}
                methods = {name: prior.getinfo(name).compress_type for name in entries}
            with zipfile.ZipFile(unsigned, "x", compression=zipfile.ZIP_DEFLATED) as archive:
                for name, content in reused.items():
                    # Reusing the bytes must also retain an installable layout.
                    # zipalign cannot repair a compressed resources.arsc entry.
                    method = zipfile.ZIP_STORED if name == "resources.arsc" else methods[name]
                    archive.writestr(name, content, compress_type=method)
            report.update(reused_platform_package_receipt=str(args.reuse_platform_package),
                          reused_platform_package_sha256=previous["sha256"],
                          reused_platform_entries={name: hashlib.sha256(content).hexdigest()
                                                   for name, content in reused.items()})
        else:
            classes, dex = output / "classes", output / "dex"
            classes.mkdir(); dex.mkdir()
            java = sorted((ROOT / "platform/android/host/src").rglob("*.java"))
            run("javac", ["javac", "--release", "8", "-classpath", android, "-d", classes, *java])
            run("d8", ["java", "-cp", d8, "com.android.tools.r8.D8", "--min-api", "26", "--lib", android,
                       "--output", dex, *sorted(classes.rglob("*.class"))])
            run("manifest", [tool["path"], "link", "-I", android, "--manifest", ROOT / "platform/android/host/AndroidManifest.xml", "-o", unsigned])
        if args.reuse_native_deployment:
            previous = json.loads(args.reuse_native_deployment.read_text())
            assert previous["exit"] == 0 and previous["library_sha256"] == library["sha256"]
            deployed = Path(previous["deployment"]["path"])
            assert sha(deployed) == previous["deployment"]["sha256"]
            report["reused_native_deployment_receipt"] = str(args.reuse_native_deployment)
        else:
            deployed = output / "deploy/libmiopunch_native_probe.so"
            deployed.parent.mkdir()
            shutil.copyfile(library["pin"], deployed)
            run("strip-deployment", ["llvm-strip", "--strip-debug", deployed])
        assert sha(library["pin"]) == library["sha256"]
        report["deployment"] = dict(path=str(deployed), bytes=deployed.stat().st_size, sha256=sha(deployed), original_pin_unchanged=True)
        with zipfile.ZipFile(unsigned, "a", compression=zipfile.ZIP_DEFLATED) as archive:
            if not args.reuse_platform_package:
                archive.write(dex / "classes.dex", "classes.dex")
            archive.write(deployed, "lib/arm64-v8a/libmiopunch_native_probe.so")
        aligned, apk = output / "aligned.apk", output / "miopunch-host-verification.apk"
        run("align", ["zipalign", "-f", "4", unsigned, aligned])
        run("sign", ["java", "-jar", signer, "sign", "--ks", key, "--ks-key-alias", "verification",
                     "--ks-pass", "file:" + str(password), "--out", apk, aligned])
        run("verify", ["java", "-jar", signer, "verify", "--verbose", "--print-certs", apk])
        run("alignment-check", ["zipalign", "-c", "4", apk])
        report["resource_table_layout"] = resource_table_layout(apk)
        assert sha(deployed) == report["deployment"]["sha256"]
        for name, digest in sources.items():
            assert sha(ROOT / name) == digest, name
        assert apk.stat().st_size <= plan["apk_budget_bytes"]
        assert sum(p.stat().st_size for p in output.rglob("*") if p.is_file()) < plan["staging_budget_bytes"]
        pin = pin_artifact(apk, ROOT / ".artifacts/runs" / output.name / apk.name, max_bytes=plan["store_cap"])
        report.update(exit=0, pin=str(pin), sha256=sha(pin), bytes=pin.stat().st_size)
    finally:
        (output / "build.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["scope", "exit", "pin", "sha256", "bytes"]}))


if __name__ == "__main__":
    main()
