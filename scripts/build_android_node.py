#!/usr/bin/env python3
"""Build the shared native CLI for Android, using an NDK or its installed sysroot."""
import argparse
import json
import os
import pathlib
import shlex
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ndk", required=True, help="Android NDK directory")
    parser.add_argument("--output", required=True, help="new directory for build log/tool metadata")
    args = parser.parse_args()
    output = pathlib.Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    ndk = pathlib.Path(args.ndk).resolve()
    native = ndk / "toolchains/llvm/prebuilt/linux-x86_64"
    env = os.environ.copy()
    if native.is_dir():
        compiler = native / "bin/aarch64-linux-android24-clang"
        archiver = native / "bin/llvm-ar"
    else:
        # Reuse the installed Windows NDK's target files in WSL without running PE tools.
        target = ndk / "toolchains/llvm/prebuilt/windows-x86_64"
        assert (target / "sysroot").is_dir(), "NDK sysroot not found"
        clang = shutil.which("clang")
        assert clang, "native Clang is required for the sysroot fallback"
        sysroot = pathlib.Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
        host = next(line.split(": ", 1)[1] for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
        linker = sysroot / "lib/rustlib" / host / "bin/rust-lld"
        assert linker.is_file(), "Rust lld not found"
        (output / "ld.lld").symlink_to(linker)
        clang_lib = sorted((target / "lib/clang").iterdir())[-1] / "lib/linux"
        compiler = output / "android-clang"
        command = [clang, "--target=aarch64-linux-android24", "--sysroot=" + str(target / "sysroot"),
                   "-B" + str(output), "-fuse-ld=lld", "-L" + str(clang_lib), "-L" + str(clang_lib / "aarch64")]
        compiler.write_text("#!/bin/sh\nexec " + shlex.join(command) + ' "$@"\n')
        compiler.chmod(0o700)
        archiver = pathlib.Path(shutil.which("llvm-ar") or "/usr/bin/llvm-ar")
    env.update(CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=str(compiler),
               CC_aarch64_linux_android=str(compiler), AR_aarch64_linux_android=str(archiver))
    command = ["cargo", "build", "--locked", "--target", "aarch64-linux-android", "--bin", "miopunch-native-probe"]
    versions = {"rust": subprocess.check_output(["rustc", "-vV"], text=True),
                "compiler": subprocess.check_output([str(compiler), "--version"], text=True),
                "ndk": (ndk / "source.properties").read_text(), "command": command}
    with (output / "build.log").open("w") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    versions["exit"] = result.returncode
    (output / "build.json").write_text(json.dumps(versions, indent=2))
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
