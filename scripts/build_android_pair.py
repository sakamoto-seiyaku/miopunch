#!/usr/bin/env python3
"""Build selected formal SDK Android actors in a caller-owned target directory."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def capture(args):
    return subprocess.check_output(["rtk", "proxy", *map(str, args)], text=True).strip()


def sources(extra=()):
    paths = {ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "build.rs",
             ROOT / ".cargo/config", ROOT / ".cargo/config.toml", Path(__file__).resolve(),
             ROOT / "scripts/android/MioPublicShell.java", *map(Path, extra)}
    for directory in (ROOT / "src", ROOT / "vendor"):
        paths.update(path for path in directory.rglob("*")
                     if not {".git", "target", "node_modules", "__pycache__"}.intersection(path.parts))
    return {str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else "external:" + str(path):
            hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(paths) if path.is_file()}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def compiler(ndk, output):
    native = ndk / "toolchains/llvm/prebuilt/linux-x86_64"
    if native.is_dir():
        return native / "bin/aarch64-linux-android24-clang", native / "bin/llvm-ar"
    windows = ndk / "toolchains/llvm/prebuilt/windows-x86_64"
    if not (windows / "sysroot").is_dir():
        raise ValueError("NDK Linux toolchain or Windows target sysroot is required")
    clang = shutil.which("clang")
    archiver = shutil.which("llvm-ar")
    if not clang or not archiver:
        raise ValueError("native Linux clang and llvm-ar are required")
    sysroot = Path(capture(["rustc", "--print", "sysroot"]))
    host = next(line.split(": ", 1)[1] for line in capture(["rustc", "-vV"]).splitlines()
                if line.startswith("host: "))
    linker = sysroot / "lib/rustlib" / host / "bin/rust-lld"
    if not linker.is_file():
        raise ValueError("Rust lld is required")
    (output / "ld.lld").symlink_to(linker)
    clang_lib = sorted((windows / "lib/clang").iterdir())[-1] / "lib/linux"
    wrapper = output / "android-clang"
    arguments = ["rtk", "proxy", clang, "--target=aarch64-linux-android24",
                 "--sysroot=" + str(windows / "sysroot"), "-B" + str(output), "-fuse-ld=lld",
                 "-L" + str(clang_lib), "-L" + str(clang_lib / "aarch64")]
    wrapper.write_text("#!/bin/sh\nexec " + shlex.join(arguments) + ' "$@"\n')
    wrapper.chmod(0o700)
    return wrapper, Path(archiver)


class DeploymentFailure(RuntimeError):
    def __init__(self, message, partial, raw_exit=None):
        super().__init__(message)
        self.partial = partial
        self.raw_exit = raw_exit


def prepare_deployment(artifacts, destination):
    """Copy owned build outputs and strip debug sections only from ELF copies."""
    destination = Path(destination)
    destination.mkdir(mode=0o700, exist_ok=False)
    stripper = shutil.which("llvm-strip")
    if not stripper:
        raise DeploymentFailure("native llvm-strip is required for deployment copies", {})
    result = {}
    for artifact in artifacts:
        artifact = Path(artifact)
        copied = destination / artifact.name
        original_hash = hashlib.sha256(artifact.read_bytes()).hexdigest()
        result[artifact.name] = {"compiledPath": str(artifact), "compiledSha256": original_hash,
                                 "deployedPath": str(copied), "compiledBytes": artifact.stat().st_size}
        try:
            shutil.copy2(artifact, copied)
        except OSError as error:
            result[artifact.name]["transformation"] = {"operation": "copy byte-for-byte", "exit": None,
                                                        "failureClass": type(error).__name__}
            raise DeploymentFailure("owned deployment copy failed", result) from error
        with copied.open("rb") as handle:
            elf = handle.read(4) == b"\x7fELF"
        transform = {"operation": "copy byte-for-byte", "exit": 0}
        if elf:
            command = ["rtk", "proxy", stripper, "--strip-debug", str(copied)]
            try:
                executed = subprocess.run(command, capture_output=True, text=True)
            except OSError as error:
                result[artifact.name]["transformation"] = {"operation": "strip debug sections on owned copy only",
                    "command": command, "exit": None, "failureClass": type(error).__name__}
                raise DeploymentFailure("deployment stripping process failed to launch", result) from error
            transform = {"operation": "strip debug sections on owned copy only",
                         "command": command, "exit": executed.returncode,
                         "stdout": executed.stdout, "stderr": executed.stderr}
            result[artifact.name].update(transformation=transform,
                deployedSha256=hashlib.sha256(copied.read_bytes()).hexdigest() if copied.is_file() else None,
                deployedBytes=copied.stat().st_size if copied.is_file() else None)
            if executed.returncode:
                raise DeploymentFailure("deployment-copy debug stripping failed", result, executed.returncode)
        result[artifact.name].update(deployedSha256=hashlib.sha256(copied.read_bytes()).hexdigest(),
                                     deployedBytes=copied.stat().st_size, transformation=transform)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ndk", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new private build receipt directory")
    parser.add_argument("--target-dir", type=Path, required=True, help="owned/reusable Android Cargo target")
    parser.add_argument("--bin", action="append", dest="bins")
    parser.add_argument("--features", default="rtc-probe")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--online", action="store_true", help="otherwise use existing Cargo cache offline")
    parser.add_argument("--jni-shell", action="store_true")
    parser.add_argument("--d8-jar", type=Path)
    parser.add_argument("--strip-deployment-debug", action="store_true", help="make smaller ELF deployment copies; preserve originals")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output exists; preserve prior failures/build receipts")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if args.jni_shell and (not args.d8_jar or not args.d8_jar.is_file()):
        parser.error("--jni-shell requires existing --d8-jar")
    output, target, ndk = args.output.resolve(), args.target_dir.resolve(), args.ndk.resolve()
    output.mkdir(parents=True, mode=0o700)
    clang, archiver = compiler(ndk, output)
    environment = os.environ.copy()
    environment.update(CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=str(clang),
                       CC_aarch64_linux_android=str(clang), AR_aarch64_linux_android=str(archiver),
                       CARGO_BUILD_JOBS=str(args.jobs))
    bins = args.bins or ["miopunch-native-probe", "browser_wss_member_native"]
    command = ["rtk", "proxy", "cargo", "build", "--locked", "--target", "aarch64-linux-android",
               "--target-dir", str(target)]
    if not args.online:
        command.append("--offline")
    if args.features:
        command += ["--features", args.features]
    for binary in bins:
        command += ["--bin", binary]
    if args.jni_shell:
        command.append("--lib")
    input_tools = [clang, archiver, ndk / "source.properties"]
    if args.jni_shell:
        input_tools.append(args.d8_jar.resolve())
    if args.strip_deployment_debug and shutil.which("llvm-strip"):
        input_tools.append(Path(shutil.which("llvm-strip")))
    before = sources(input_tools)
    (output / "sources-before.json").write_text(json.dumps(before, indent=2) + "\n")
    report = {"scope": "formal SDK Android build only; no device/NAT/SDK story pass",
              "head": capture(["git", "rev-parse", "HEAD"]), "command": command,
              "targetDirectory": str(target), "rust": capture(["rustc", "-vV"]),
              "compiler": capture([str(clang), "--version"]),
              "ndk": (ndk / "source.properties").read_text(), "sourceDigestBefore": digest(before),
              "builderSha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    print(json.dumps({"event": "android-build-started", "output": str(output), "target": str(target), "bins": bins}), flush=True)
    started = time.monotonic()
    with (output / "build.log").open("w") as log:
        result = subprocess.run(command, cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT)
    report.update(exit=result.returncode, cargoExit=result.returncode,
                  callerElapsedMs=round((time.monotonic() - started) * 1000, 3))
    artifacts = [target / "aarch64-linux-android/debug" / binary for binary in bins]
    if args.jni_shell and result.returncode == 0:
        artifacts.append(target / "aarch64-linux-android/debug/libmiopunch_native_probe.so")
        classes, dex = output / "classes", output / "dex"
        classes.mkdir()
        dex.mkdir()
        shell_source = ROOT / "scripts/android/MioPublicShell.java"
        shell_commands = [
            ["rtk", "proxy", "javac", "--release", "8", "-d", str(classes), str(shell_source)],
            ["rtk", "proxy", "java", "-cp", str(args.d8_jar), "com.android.tools.r8.D8",
             "--min-api", "24", "--output", str(dex), str(classes / "MioPublicShell.class")],
            ["rtk", "proxy", "jar", "cf", str(output / "android-shell.jar"), "-C", str(dex), "classes.dex"],
        ]
        report["jniCommands"] = []
        with (output / "jni-build.log").open("w") as log:
            for shell_command in shell_commands:
                built = subprocess.run(shell_command, stdout=log, stderr=subprocess.STDOUT)
                report["jniCommands"].append({"command": shell_command, "exit": built.returncode})
                if built.returncode:
                    report["exit"] = built.returncode
                    break
        if report["exit"] == 0:
            artifacts.append(output / "android-shell.jar")
    report["artifacts"] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                           for path in artifacts if path.is_file() and report["exit"] == 0}
    if args.strip_deployment_debug and report["exit"] == 0:
        try:
            report["deployment"] = prepare_deployment(artifacts, output / "deploy")
        except DeploymentFailure as error:
            report["deployment"] = error.partial
            report["deploymentFailure"] = str(error)
            report["exit"] = error.raw_exit if error.raw_exit is not None and error.raw_exit != 0 else 1
        except OSError as error:
            # Also retain the successful compilation receipt for filesystem failures.
            report["deploymentFailure"] = type(error).__name__ + ": " + str(error)
            report["exit"] = 1
    # Include Java compilation and all deployment transformations in the freeze.
    after = sources(input_tools)
    report.update(sourceDigestAfter=digest(after), sourceFrozenDuringBuild=before == after)
    (output / "sources-after.json").write_text(json.dumps(after, indent=2) + "\n")
    report["qualifiedForFrozenRun"] = report["exit"] == 0 and report["sourceFrozenDuringBuild"]
    (output / "build.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"event": "android-build-ended", "exit": report["exit"],
                      "qualifiedForFrozenRun": report["qualifiedForFrozenRun"], "receipt": str(output / "build.json")}), flush=True)
    return report["exit"]


if __name__ == "__main__":
    raise SystemExit(main())
