#!/usr/bin/env python3
"""Build and run the Windows/Linux CLI matrix in isolated, owned Docker tools.

This executes Windows PE code under Wine 10, not a native Windows kernel.
No networking, crypto, admission or storage success is mocked.
"""
import argparse
import json
import os
import pathlib
import shlex
import subprocess
import tempfile
import time


ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--case", default="all",
                        choices=["all", "first", "restart", "management", "precode", "resources"])
    args = parser.parse_args()
    output = pathlib.Path(args.output).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    commands = []
    containers = []
    runtime_name = f"miopunch-desktop-runtime-{os.getpid()}"
    compiler_name = f"miopunch-desktop-compiler-{os.getpid()}"
    env = os.environ.copy()

    def run(label, command, command_env=None):
        started = time.monotonic()
        with (output / (label + ".log")).open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=command_env, stdout=log,
                                    stderr=subprocess.STDOUT)
        commands.append({"label": label, "command": command, "exit": result.returncode,
                         "seconds": round(time.monotonic() - started, 3)})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(f"{label} failed; see {output / (label + '.log')}")

    try:
        for stage, image in [("compiler", "miopunch-desktop-compiler:bookworm"),
                             ("runtime", "miopunch-desktop-runtime:trixie")]:
            run("image-" + stage, ["docker", "build", "--target", stage, "-t", image,
                                  "-f", "scripts/desktop_windows_lab.Dockerfile", "scripts"])
        mounts = ["-v", f"{pathlib.Path.home()}:{pathlib.Path.home()}:ro",
                  "-v", f"{ROOT}:{ROOT}",
                  "-v", f"{tempfile.gettempdir()}:{tempfile.gettempdir()}",
                  "-v", f"{output}:{output}"]
        for name, image in [(compiler_name, "miopunch-desktop-compiler:bookworm"),
                            (runtime_name, "miopunch-desktop-runtime:trixie")]:
            containers.append(name)
            run("start-" + name, ["docker", "run", "-d", "--name", name, "--network", "host",
                                 "--user", f"{os.getuid()}:{os.getgid()}", *mounts,
                                 "-w", str(ROOT), image, "sleep", "infinity"])
        wrappers = output / "tools"
        wrappers.mkdir()
        for tool in ["x86_64-w64-mingw32-gcc", "x86_64-w64-mingw32-ar", "x86_64-w64-mingw32-dlltool"]:
            path = wrappers / tool
            path.write_text("#!/bin/sh\nexec docker exec -i -w \"$PWD\" " +
                            shlex.quote(compiler_name) + " " + tool + ' "$@"\n')
            path.chmod(0o700)
        prefix = output / "wine-prefix"
        runtime_dir = output / "xdg-runtime"
        runtime_dir.mkdir(mode=0o700)
        runner = wrappers / "wine"
        wine_command = ["docker", "exec", "-i", "-e", "WINEDEBUG=-all",
                        "-e", f"WINEPREFIX={prefix}", "-e", f"XDG_RUNTIME_DIR={runtime_dir}",
                        "-e", "WINEDLLOVERRIDES=winemenubuilder.exe=d", runtime_name,
                        "/usr/lib/wine/wine64"]
        runner.write_text("#!/bin/sh\nexec " + shlex.join(wine_command) + ' "$@"\n')
        runner.chmod(0o700)
        run("rust-version", ["rustc", "--version"])
        run("compiler-version", ["docker", "exec", compiler_name, "x86_64-w64-mingw32-gcc", "--version"])
        run("wine-version", ["docker", "exec", runtime_name, "/usr/lib/wine/wine64", "--version"])
        run("windows-target", ["rustup", "target", "add", "x86_64-pc-windows-gnu"])
        run("linux-build", ["cargo", "build", "--locked", "--bin", "miopunch-native-probe"])
        env["PATH"] = str(wrappers) + os.pathsep + env["PATH"]
        env["CC_x86_64_pc_windows_gnu"] = str(wrappers / "x86_64-w64-mingw32-gcc")
        env["AR_x86_64_pc_windows_gnu"] = str(wrappers / "x86_64-w64-mingw32-ar")
        env["CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER"] = env["CC_x86_64_pc_windows_gnu"]
        run("windows-build", ["cargo", "build", "--locked", "--target", "x86_64-pc-windows-gnu",
                              "--bin", "miopunch-native-probe"], env)
        run("desktop-matrix", ["python3", "scripts/check_desktop_management.py", "--output",
                               str(output / "matrix"), "--windows-runner", str(runner), "--case", args.case])
    finally:
        cleanup = []
        for name in reversed(containers):
            result = subprocess.run(["docker", "rm", "-f", name], capture_output=True, text=True)
            cleanup.append({"container": name, "exit": result.returncode})
        (output / "cleanup.json").write_text(json.dumps(cleanup, indent=2) + "\n")
    print(f"Desktop checks passed; evidence: {output}")


if __name__ == "__main__":
    main()
