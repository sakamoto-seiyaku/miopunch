"""Keep controlled check compilation and artifact pinning in the shared lease."""

from contextlib import contextmanager
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

import cargo_cache
from pin_artifact import pin_artifact, sha256

CLI_TESTS = {"native_network", "native_probe", "tcp_direct", "resources", "mqtt_probe"}


def runtime_artifact(value, mappings=None):
    value = str(value)
    paths = json.loads(os.environ.get("MIOPUNCH_TEST_ARTIFACTS", "{}"))
    paths.update(mappings or {})
    direct = paths.get(str((cargo_cache.ROOT / value).resolve()))
    if direct:
        return direct
    if value.startswith("/repo/"):
        direct = paths.get(str(cargo_cache.ROOT / value[len('/repo/'):]))
        if direct:
            return "/repo/" + Path(direct).relative_to(cargo_cache.ROOT).as_posix()
    return value


def cargo_arguments(command):
    command = list(map(str, command))
    if command[:2] == ["rtk", "proxy"]:
        command = command[2:]
    elif command[:1] == ["rtk"]:
        command = command[1:]
    if command[:1] == ["cargo"] and command[1:2] in (["build"], ["test"], ["check"], ["rustc"]):
        return command[1:]
    return None


def test_request(arguments):
    """Separate Cargo target selection from libtest filtering, without a second manifest."""
    end = arguments.index("--") if "--" in arguments else len(arguments)
    runtime = arguments[end + 1:]
    options = arguments[:end]
    wants_run = "--no-run" not in options
    values = {"--test", "--bin", "--example", "--bench", "--package", "-p", "--exclude",
              "--features", "-F", "--target", "--target-dir", "--manifest-path", "--profile",
              "--jobs", "-j", "--color", "--config"}
    compile_args, filters, targets = ["test"], [], []
    i = 1
    while i < len(options):
        value = options[i]
        if value in values:
            compile_args.extend(options[i:i + 2])
            if value == "--test" and i + 1 < len(options):
                targets.append(options[i + 1])
            i += 2
        elif value == "--no-run" or value.startswith("--message-format="):
            i += 1
        elif value.startswith("-"):
            compile_args.append(value)
            if value.startswith("--test="):
                targets.append(value.split("=", 1)[1])
            i += 1
        else:
            filters.append(value)
            i += 1
    if len(filters) > 1:
        raise cargo_cache.CacheError("Cargo accepts one test-name filter")
    compile_args.append("--no-run")
    return compile_args, filters + runtime, wants_run, targets


class CheckBuilds:
    """One runner's builds and pins. Cargo owns invalidation; nothing is cached across runners."""

    def __init__(self, output=None):
        self.built = {}
        self.planned = {}
        self.paths = {}
        self.cli = None
        self.last_programs = []
        self._sources = None
        self._source_stats = {}
        self._hashes = {}
        self._pin_stats = {}
        self._pin_root = cargo_cache.ROOT / ".artifacts/runs/check-builds" / f"{os.getpid()}-{time.time_ns()}"
        self.output = Path(output).resolve() if output is not None else self._pin_root / "receipts"

    @staticmethod
    def compiles(command):
        return cargo_arguments(command) is not None

    @staticmethod
    def _request(arguments):
        arguments = [a for a in arguments if not a.startswith("--message-format=")]
        if arguments[0] == "rustc" and "--test" in arguments and "--" in arguments:
            raise cargo_cache.CacheError("test batches use committed compiler settings; transform a deployment copy instead")
        if arguments[0] == "test" and "--doc" not in arguments or arguments[0] == "rustc" and "--test" in arguments:
            return test_request(arguments)
        return arguments, [], False, []

    @staticmethod
    def _group(arguments):
        base, targets = [], []
        i = 0
        while i < len(arguments):
            if arguments[i] == "--test":
                targets.append(arguments[i + 1]); i += 2
            elif arguments[i].startswith("--test="):
                targets.append(arguments[i].split("=", 1)[1]); i += 1
            else:
                base.append(arguments[i]); i += 1
        return tuple(base), targets

    def plan(self, checks):
        """Union named test targets before the first scenario; filters never create a build."""
        for item in checks:
            arguments = cargo_arguments(item[1])
            if arguments is None:
                continue
            arguments, _, _, targets = self._request(arguments)
            if targets:
                key, _ = self._group(arguments)
                self.planned.setdefault(key, set()).update(targets)

    def _source_guard(self):
        from build_native_client import input_paths
        names = input_paths() | {"scripts/check_build_cache.py"}
        names.update(p.relative_to(cargo_cache.ROOT).as_posix()
                     for p in (cargo_cache.ROOT / "browser-probe/src").rglob("*") if p.is_file())
        stats = {}
        for name in names:
            s = (cargo_cache.ROOT / name).stat()
            stats[name] = (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        if self._sources is None:
            self._sources = {name: sha256(cargo_cache.ROOT / name) for name in sorted(names)}
            self._source_stats = stats
        elif stats != self._source_stats:
            raise cargo_cache.CacheError("source/manifest/lock inputs changed during this verification batch")

    def path(self, value):
        return runtime_artifact(value, self.paths)

    def environment(self, environment=None):
        result = dict(os.environ if environment is None else environment)
        inherited = json.loads(result.get("MIOPUNCH_TEST_ARTIFACTS", "{}"))
        inherited.update(self.paths)
        if inherited:
            result["MIOPUNCH_TEST_ARTIFACTS"] = json.dumps(inherited)
        if self.cli:
            result["MIOPUNCH_TEST_CLI"] = self.cli
        return result

    def _runtime_command(self, command, environment):
        command = [self.path(a) for a in command]
        mappings = environment.get("MIOPUNCH_TEST_ARTIFACTS")
        if mappings:
            for i, value in enumerate(command[:-1]):
                if value == "docker" and command[i + 1] in ("run", "exec"):
                    options = ["--env", "MIOPUNCH_TEST_ARTIFACTS=" + mappings]
                    cli = environment.get("MIOPUNCH_TEST_CLI")
                    if cli:
                        if any("/repo/" in a for a in command) and Path(cli).is_relative_to(cargo_cache.ROOT):
                            cli = "/repo/" + Path(cli).relative_to(cargo_cache.ROOT).as_posix()
                        options += ["--env", "MIOPUNCH_TEST_CLI=" + cli]
                    command[i + 2:i + 2] = options
                    break
        return command

    def _verify_pins(self, paths=None):
        for name in paths if paths is not None else self._hashes:
            info = Path(name).stat()
            identity = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
            if self._pin_stats.get(name) != identity:
                if sha256(name) != self._hashes[name]:
                    raise cargo_cache.CacheError("verification pin changed: " + name)
                self._pin_stats[name] = identity

    @contextmanager
    def deployment(self, source, *, strip_debug=False):
        """Transform only an owned copy and remove it after the runtime owner finishes."""
        source = Path(self.path(source))
        if source.resolve().is_relative_to(cargo_cache.ROOT / "target"):
            with cargo_cache.cache_lease(cargo_cache.target_directory()):
                source = pin_artifact(source, self._pin_root / "workers" / source.name)
        if not strip_debug:
            yield source
            return
        destination = self._pin_root / "deployment" / source.name
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            raise FileExistsError(destination)
        self._verify_pins()
        shutil.copy2(source, destination)
        destination.chmod(0o700)
        try:
            stripper = shutil.which("llvm-strip") or shutil.which("strip")
            if not stripper:
                raise cargo_cache.CacheError("--strip-debug requires an installed strip tool")
            self.run(["rtk", "proxy", stripper, "--strip-debug", str(destination)], check=True)
            yield destination
        finally:
            destination.unlink(missing_ok=True)

    def popen(self, command, **kwargs):
        if self.compiles(command):
            raise cargo_cache.CacheError("compile synchronously under the lease before starting a runtime process")
        kwargs["env"] = self.environment(kwargs.get("env"))
        self._verify_pins()
        return subprocess.Popen(self._runtime_command(command, kwargs["env"]), **kwargs)

    def _compile(self, arguments, environment, timeout=None):
        arguments, _, _, targets = self._request(arguments)
        if targets:
            group, _ = self._group(arguments)
            union = sorted(set(targets) | self.planned.get(group, set()))
            arguments = list(group)
            for target in union:
                arguments += ["--test", target]
            targets = union
        key = tuple(arguments)
        self._source_guard()
        if key in self.built:
            record = self.built[key]
            self._verify_pins(record["pins"])
            self.last_programs = record["programs"]
            self.paths.update(record["paths"])
            self.cli = record.get("cli")
            return record, False
        target = cargo_cache.target_directory()
        env = cargo_cache.build_environment(target, environment)
        if "--locked" not in arguments and "--frozen" not in arguments:
            arguments.insert(1, "--locked")
        if "--offline" not in arguments and "--frozen" not in arguments:
            arguments.insert(1, "--offline")
        if not any(a in ("--jobs", "-j") or a.startswith(("--jobs=", "-j")) for a in arguments):
            arguments[1:1] = ["--jobs", "2"]
        command = cargo_cache.cargo_command(arguments, target)
        directory = self.output / "builds" / f"{len(self.built) + 1:03d}"
        directory.mkdir(parents=True, exist_ok=False)
        report = {"command": command, "exit": None, "cargoInvoked": True, "targetDirectory": str(target)}
        record = {"stdout": "", "stderr": "", "exit": None, "programs": [], "pins": {}, "paths": {},
                  "receipt": str(directory / "receipt.json")}
        try:
            with cargo_cache.cache_lease(target) as before:
                self._source_guard()
                report["diskBefore"] = before
                (directory / "inputs.json").write_text(json.dumps(self._sources, indent=2) + "\n")
                started = time.monotonic()
                process = subprocess.Popen(command, cwd=cargo_cache.ROOT, env=env, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, text=True, start_new_session=True)
                try:
                    stdout, stderr = process.communicate(timeout=timeout)
                except BaseException:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        stdout, stderr = process.communicate(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        stdout, stderr = process.communicate()
                    (directory / "cargo.jsonl").write_text(stdout)
                    (directory / "cargo.stderr.log").write_text(stderr)
                    report["exit"] = process.returncode
                    raise
                record.update(stdout=stdout, stderr=stderr, exit=process.returncode)
                (directory / "cargo.jsonl").write_text(stdout)
                (directory / "cargo.stderr.log").write_text(stderr)
                report.update(exit=process.returncode, cargoSeconds=time.monotonic() - started)
                entries = []
                for line in stdout.splitlines():
                    try:
                        item = json.loads(line)
                    except ValueError:
                        continue
                    if item.get("reason") == "compiler-artifact":
                        entries.append(item)
                report.update(freshArtifacts=sum(e.get("fresh") is True for e in entries),
                              rebuiltArtifacts=sum(e.get("fresh") is False for e in entries),
                              rebuiltTargets=[{"package": e["package_id"], "target": e["target"]["name"]}
                                              for e in entries if e.get("fresh") is False])
                if process.returncode:
                    return record, True
                if not entries:
                    raise cargo_cache.CacheError("successful compilation emitted no artifact records")
                self._source_guard()
                needs_cli = bool(set(targets) & CLI_TESTS) or arguments[0] == "test" and not targets and "--lib" not in arguments
                for entry in entries:
                    executable = entry.get("executable")
                    is_test = entry.get("profile", {}).get("test") is True
                    name = entry["target"]["name"]
                    selected = executable and (arguments[0] != "test" or is_test or needs_cli and name == "miopunch-native-probe")
                    files = [executable] if selected else []
                    if arguments[0] == "build" and not executable and any(k in entry["target"]["kind"] for k in ("lib", "rlib", "cdylib")) and name == "miopunch_browser_probe":
                        files = [p for p in entry["filenames"] if p.endswith(".wasm")]
                    for filename in files:
                        original = Path(filename).resolve()
                        pinned = pin_artifact(original, self._pin_root / str(len(self.built) + 1) / original.name)
                        self.paths[str(original)] = str(pinned)
                        record["paths"][str(original)] = str(pinned)
                        record["pins"][str(pinned)] = sha256(pinned)
                        self._hashes[str(pinned)] = record["pins"][str(pinned)]
                        info = pinned.stat()
                        self._pin_stats[str(pinned)] = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
                        if name == "miopunch-native-probe" and not is_test:
                            self.cli = str(pinned)
                            record["cli"] = self.cli
                        if is_test:
                            record["programs"].append({"name": name, "pin": str(pinned), "sha256": record["pins"][str(pinned)]})
                report.update(pins=record["pins"], programs=record["programs"], diskAfter=cargo_cache.disk_state(target))
                # Retain actual Cargo messages separately; expose pins to existing JSON readers.
                rendered = []
                for line in stdout.splitlines():
                    try:
                        item = json.loads(line)
                    except ValueError:
                        rendered.append(line); continue
                    if item.get("reason") == "compiler-artifact":
                        original = item.get("executable")
                        if original and original in self.paths:
                            item.update(originalExecutable=original, executable=self.paths[original])
                        item["filenames"] = [self.paths.get(p, p) for p in item["filenames"]]
                    rendered.append(json.dumps(item))
                record["stdout"] = "\n".join(rendered) + "\n"
                self.built[key] = record
                self.last_programs = record["programs"]
        except BaseException as error:
            report["failure"] = str(error) or type(error).__name__
            raise
        finally:
            (directory / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
        return record, True

    @staticmethod
    def _deliver(command, code, stdout, stderr, kwargs):
        text = kwargs.get("text") or kwargs.get("universal_newlines") or kwargs.get("encoding")
        out = stdout if text else stdout.encode()
        err = stderr if text else stderr.encode()
        capture = kwargs.get("capture_output", False)
        def write(stream, data):
            try:
                stream.write(data)
            except TypeError:
                stream.write(data.decode() if isinstance(data, bytes) else data.encode())
            stream.flush()
        destination = kwargs.get("stdout")
        error_destination = kwargs.get("stderr")
        if error_destination == subprocess.STDOUT:
            out += err
            err = "" if text else b""
        if not capture and destination != subprocess.PIPE:
            if destination != subprocess.DEVNULL:
                write(destination or sys.stdout, out)
            out = None
        if not capture and error_destination != subprocess.PIPE:
            if error_destination not in (subprocess.STDOUT, subprocess.DEVNULL):
                write(error_destination or sys.stderr, err)
            err = None
        result = subprocess.CompletedProcess(command, code, out, err)
        if code and kwargs.get("check"):
            raise subprocess.CalledProcessError(code, command, output=out, stderr=err)
        return result

    def run(self, command, **kwargs):
        arguments = cargo_arguments(command)
        if arguments is None:
            kwargs["env"] = self.environment(kwargs.get("env"))
            self._verify_pins()
            return subprocess.run(self._runtime_command(command, kwargs["env"]), **kwargs)
        started = time.monotonic()
        compile_args, runtime, wants_run, targets = self._request(arguments)
        record, invoked = self._compile(arguments, kwargs.get("env") or os.environ, kwargs.get("timeout"))
        stdout, stderr, code = record["stdout"], record["stderr"], record["exit"]
        stdout = json.dumps({"reason": "miopunch-build-receipt", "path": record["receipt"],
                             "cargoInvoked": invoked}) + "\n" + stdout
        if not invoked:
            stdout = json.dumps({"reason": "miopunch-build-reuse", "originatingReceipt": record["receipt"],
                                 "cargoInvoked": False}) + "\n" + stdout
        if not code and wants_run:
            selected = [p for p in record["programs"] if not targets or p["name"] in targets]
            if not selected:
                raise cargo_cache.CacheError("Cargo emitted no selected test executable")
            self.last_programs = []
            for program in selected:
                self.last_programs.append(program)
                self._verify_pins([program["pin"]])
                run = ["rtk", "proxy", program["pin"], *runtime]
                stderr += "Running " + shlex.join(run[2:]) + "\n"
                timeout = kwargs.get("timeout")
                if timeout is not None:
                    timeout = max(0.001, timeout - (time.monotonic() - started))
                result = subprocess.run(run, cwd=kwargs.get("cwd", cargo_cache.ROOT), env=self.environment(kwargs.get("env")),
                                        capture_output=True, text=True, timeout=timeout)
                stdout += result.stdout; stderr += result.stderr
                if result.returncode:
                    code = result.returncode
                    if "--no-fail-fast" not in compile_args:
                        break
            explicit = any(a in compile_args for a in ("--lib", "--bin", "--test", "--tests", "--bins", "--all-targets", "--examples"))
            if not code and not explicit:
                doc = [a for a in arguments[:arguments.index("--") if "--" in arguments else len(arguments)]
                       if a != "--no-run"]
                doc.insert(1, "--doc")
                doc_runtime = arguments[arguments.index("--") + 1:] if "--" in arguments else []
                result = self.run(["cargo", *doc, *(["--", *doc_runtime] if doc_runtime else [])], capture_output=True, text=True,
                                  env=kwargs.get("env"), cwd=kwargs.get("cwd", cargo_cache.ROOT))
                stdout += result.stdout or ""; stderr += result.stderr or ""; code = result.returncode
        return self._deliver(command, code, stdout, stderr, kwargs)


@contextmanager
def compiled_artifacts(output, name, arguments, source_reader, expected_sources, run, guard):
    """Yield raw compiler artifacts while the caller pins them under the lease."""
    target = cargo_cache.target_directory()
    command = cargo_cache.cargo_command(arguments, target)
    environment = cargo_cache.build_environment(target)
    receipt = {"command": command, "targetDirectory": str(target), "exit": None,
               "artifactPinningFinishedUnderLease": False}
    entries = []
    try:
        with cargo_cache.cache_lease(target) as before:
            receipt["diskBefore"] = before
            if source_reader() != expected_sources:
                raise cargo_cache.CacheError("source or locked input changed before Cargo")
            started = time.monotonic()
            code = run(name, command, guard, environment=environment)
            receipt.update(exit=code, cargoSeconds=time.monotonic() - started)
            for line in (Path(output) / f"{name}.stdout.log").read_text().splitlines():
                try:
                    item = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if item.get("reason") == "compiler-artifact":
                    entries.append(item)
            receipt.update(freshArtifacts=sum(item.get("fresh") is True for item in entries),
                           rebuiltArtifacts=sum(item.get("fresh") is False for item in entries),
                           rebuiltTargets=[item["target"]["name"] for item in entries
                                           if item.get("fresh") is False])
            if code != 0:
                raise cargo_cache.CacheError(f"{name} exit {code}; raw compiler logs retained")
            if source_reader() != expected_sources:
                raise cargo_cache.CacheError("source or locked input changed during Cargo")
            yield entries
            receipt.update(artifactPinningFinishedUnderLease=True,
                           diskAfter=cargo_cache.disk_state(target))
    except BaseException as error:
        receipt["failure"] = str(error) or type(error).__name__
        raise
    finally:
        (Path(output) / f"{name}.cache.json").write_text(json.dumps(receipt, indent=2) + "\n")
