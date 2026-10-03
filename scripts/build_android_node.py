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
    parser.add_argument("--jni-shell", action="store_true", help="also build the foreground JNI shell and dex jar")
    parser.add_argument("--d8-jar", help="Android build-tools lib/d8.jar, required with --jni-shell")
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
    if args.jni_shell:
        command.append("--lib")
    versions = {"rust": subprocess.check_output(["rustc", "-vV"], text=True),
                "compiler": subprocess.check_output([str(compiler), "--version"], text=True),
                "ndk": (ndk / "source.properties").read_text(), "command": command}
    with (output / "build.log").open("w") as log:
        result = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
    versions["exit"] = result.returncode
    if result.returncode == 0 and args.jni_shell:
        assert args.d8_jar and pathlib.Path(args.d8_jar).is_file(), "--d8-jar required for JNI shell"
        classes = output / "classes"
        dex = output / "dex"
        classes.mkdir()
        dex.mkdir()
        source = pathlib.Path(__file__).resolve().parent / "android/MioPublicShell.java"
        shell_commands = [
            ["javac", "--release", "8", "-d", str(classes), str(source)],
            ["java", "-cp", args.d8_jar, "com.android.tools.r8.D8", "--min-api", "24", "--output", str(dex), str(classes / "MioPublicShell.class")],
            ["jar", "cf", str(output / "android-shell.jar"), "-C", str(dex), "classes.dex"],
        ]
        with (output / "shell-build.log").open("w") as log:
            for shell_command in shell_commands:
                built = subprocess.run(shell_command, stdout=log, stderr=subprocess.STDOUT)
                if built.returncode:
                    versions["exit"] = built.returncode
                    break
        versions["shell_commands"] = shell_commands
    (output / "build.json").write_text(json.dumps(versions, indent=2))
    raise SystemExit(versions["exit"])


if __name__ == "__main__":
    main()
