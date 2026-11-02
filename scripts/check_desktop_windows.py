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


from check_build_cache import CheckBuilds
from build_native_client import input_paths
from pin_artifact import sha256

builds = CheckBuilds()


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

    def run(label, command, command_env=None):
        started = time.monotonic()
        with (output / (label + ".log")).open("w") as log:
            result = builds.run(command, cwd=ROOT, env=command_env, stdout=log,
                                    stderr=subprocess.STDOUT)
        commands.append({"label": label, "command": command, "exit": result.returncode,
                         "seconds": round(time.monotonic() - started, 3)})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(f"{label} failed; see {output / (label + '.log')}")

    try:
        for stage, image in [("compiler", "miopunch-desktop-compiler:bookworm"),
                             ("runtime", "miopunch-desktop-runtime:trixie")]:
            run("image-" + stage, ["rtk", "proxy", "docker", "image", "inspect", image])
        mounts = ["-v", f"{pathlib.Path.home()}:{pathlib.Path.home()}:ro",
                  "-v", f"{ROOT}:{ROOT}",
                  "-v", f"{tempfile.gettempdir()}:{tempfile.gettempdir()}",
                  "-v", f"{output}:{output}"]
        for name, image in [(runtime_name, "miopunch-desktop-runtime:trixie")]:
            containers.append(name)
            run("start-" + name, ["docker", "run", "-d", "--name", name, "--network", "host",
                                 "--user", f"{os.getuid()}:{os.getgid()}", *mounts,
                                 "-w", str(ROOT), image, "sleep", "infinity"])
        wrappers = output / "tools"
        wrappers.mkdir()
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
        run("wine-version", ["docker", "exec", runtime_name, "/usr/lib/wine/wine64", "--version"])
        installed = subprocess.check_output(["rtk", "proxy", "rustup", "target", "list", "--installed"], text=True)
        if "x86_64-pc-windows-gnu" not in installed.splitlines():
            raise RuntimeError("install the declared Windows target before this verification")
        inputs = output / "native-inputs.json"
        inputs.write_text(json.dumps({name: sha256(ROOT / name) for name in sorted(input_paths())}, indent=2) + "\n")
        pins = {}
        for platform in ("linux", "windows"):
            receipt = output / (platform + "-build")
            run(platform + "-build", ["rtk", "proxy", "python3", "-B", "scripts/build_native_client.py",
                                      "--platform", platform, "--bin", "miopunch-native-probe",
                                      "--inputs", str(inputs), "--output", str(receipt)])
            pins[platform] = json.loads((receipt / "build.json").read_text())["pin"]
        run("desktop-matrix", ["python3", "scripts/check_desktop_management.py", "--output",
                               str(output / "matrix"), "--linux-bin", pins["linux"], "--windows-bin", pins["windows"],
                               "--windows-runner", str(runner), "--case", args.case])
    finally:
        cleanup = []
        for name in reversed(containers):
            result = builds.run(["docker", "rm", "-f", name], capture_output=True, text=True)
            cleanup.append({"container": name, "exit": result.returncode})
        (output / "cleanup.json").write_text(json.dumps(cleanup, indent=2) + "\n")
    print(f"Desktop checks passed; evidence: {output}")


if __name__ == "__main__":
    main()
