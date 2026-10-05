#!/usr/bin/env python3
"""Reuse one Cargo cache, bound its disk use, and report actual cache hits."""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
GIB = 1024 ** 3
MIN_FREE = 10 * GIB
MAX_CACHE = 96 * GIB


class CacheError(RuntimeError):
    pass


def target_directory(root=ROOT, requested=None, isolation_reason=None):
    shared = (root / "target").resolve()
    selected = Path(requested).resolve() if requested is not None else shared
    if selected != shared and not isolation_reason:
        raise CacheError("use the shared target directory; an isolated cache requires an explicit reason")
    return selected


def build_environment(target, environment=None):
    result = dict(os.environ if environment is None else environment)
    overrides = [key for key in result if key.startswith(("CARGO_PROFILE_DEV_", "CARGO_PROFILE_TEST_"))]
    if overrides:
        raise CacheError("profile overrides split the cache: " + ", ".join(sorted(overrides)))
    if result.get("CARGO_INCREMENTAL") not in (None, "0"):
        raise CacheError("incremental compilation must stay disabled for this shared cache")
    result.update(CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0")
    return result


def disk_state(target):
    existing = target
    while not existing.exists():
        existing = existing.parent
    free = shutil.disk_usage(existing).free
    size = 0
    if target.exists():
        size = int(subprocess.check_output(["rtk", "proxy", "du", "-s", "-B1", str(target)], text=True).split()[0])
    return {"freeBytes": free, "cacheBytes": size}


@contextmanager
def cache_lease(target, min_free=MIN_FREE, max_cache=MAX_CACHE):
    target.mkdir(parents=True, exist_ok=True)
    with (target / ".miopunch-build.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = disk_state(target)
        if state["freeBytes"] < min_free:
            raise CacheError(f"only {state['freeBytes'] / GIB:.2f} GiB free; need {min_free / GIB:.2f} GiB before compiling")
        if state["cacheBytes"] > max_cache:
            raise CacheError(f"cache is {state['cacheBytes'] / GIB:.2f} GiB; limit is {max_cache / GIB:.2f} GiB; inspect before cleanup")
        yield state


def incremental_directories(target):
    candidates = [target / profile / "incremental" for profile in ("debug", "release")]
    for triple in target.iterdir() if target.exists() else ():
        if triple.is_dir() and not triple.is_symlink():
            candidates += [triple / profile / "incremental" for profile in ("debug", "release")]
    owned = []
    for path in candidates:
        relative = path.relative_to(target)
        ancestors = [target.joinpath(*relative.parts[:index]) for index in range(1, len(relative.parts) + 1)]
        if path.is_dir() and not any(ancestor.is_symlink() for ancestor in ancestors):
            owned.append(path)
    return owned


def active_compilers(root):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            name = (entry / "comm").read_text().strip()
            if name not in ("cargo", "rustc"):
                continue
            cwd = (entry / "cwd").resolve()
            if cwd.is_relative_to(root):
                found.append({"pid": int(entry.name), "name": name})
        except (OSError, RuntimeError):
            continue
    return found


def cleanup_incremental(target, root=ROOT):
    target.mkdir(parents=True, exist_ok=True)
    with (target / ".miopunch-build.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if active_compilers(root):
            raise CacheError("wait for this repository's Cargo/rustc processes to end before removing incremental caches")
        removed = []
        for path in incremental_directories(target):
            size = disk_state(path)["cacheBytes"]
            shutil.rmtree(path)
            removed.append({"path": str(path), "bytes": size})
        return {"removedIncrementalCaches": removed, "diskAfter": disk_state(target)}


def cargo_command(arguments, target):
    arguments = list(arguments)
    if arguments and arguments[0] == "cargo":
        arguments.pop(0)
    if not arguments or arguments[0] not in ("build", "check", "test"):
        raise CacheError("run accepts cargo build, check or test")
    cargo_end = arguments.index("--") if "--" in arguments else len(arguments)
    for index, argument in enumerate(arguments[:cargo_end]):
        if argument == "--target-dir":
            if index + 1 >= cargo_end or Path(arguments[index + 1]).resolve() != target:
                raise CacheError("Cargo target override differs from the selected shared cache")
        elif argument.startswith("--target-dir=") and Path(argument.split("=", 1)[1]).resolve() != target:
            raise CacheError("Cargo target override differs from the selected shared cache")
        elif argument.startswith("--config"):
            raise CacheError("use the repository Cargo configuration; --config overrides require a separate documented build workflow")
        elif argument.startswith("--message-format"):
            raise CacheError("cache runner records Cargo JSON itself; omit --message-format")
    arguments[cargo_end:cargo_end] = ["--message-format=json"]
    return ["rtk", "proxy", "cargo", *arguments]


def run_cargo(arguments, target, output, min_free=MIN_FREE, max_cache=MAX_CACHE, isolation_reason=None):
    command = cargo_command(arguments, target)
    environment = build_environment(target)
    output.mkdir(parents=True, exist_ok=False)
    report = {"command": command, "targetDirectory": str(target), "isolationReason": isolation_reason,
              "incremental": False, "exit": None}
    try:
        with cache_lease(target, min_free, max_cache) as before:
            report["diskBefore"] = before
            started = time.monotonic()
            with (output / "cargo.jsonl").open("w") as stdout, (output / "cargo.stderr.log").open("w") as stderr:
                result = subprocess.run(command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr)
            report.update(exit=result.returncode, cargoSeconds=time.monotonic() - started)
            artifacts = []
            for line in (output / "cargo.jsonl").read_text().splitlines():
                try:
                    entry = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if entry.get("reason") == "compiler-artifact":
                    artifacts.append(entry)
            report.update(freshArtifacts=sum(entry["fresh"] is True for entry in artifacts),
                          rebuiltArtifacts=sum(entry["fresh"] is False for entry in artifacts),
                          rebuiltTargets=[{"package": entry["package_id"], "target": entry["target"]["name"]}
                                          for entry in artifacts if entry["fresh"] is False],
                          diskAfter=disk_state(target))
    except (CacheError, OSError) as error:
        report["failure"] = str(error)
    (output / "receipt.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in
                      ("exit", "cargoSeconds", "freshArtifacts", "rebuiltArtifacts", "diskAfter", "failure")
                      if key in report} | {"receipt": str(output / "receipt.json")}))
    return report["exit"] if report["exit"] is not None else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--isolation-reason")
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run")
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--min-free-gib", type=float, default=MIN_FREE / GIB)
    run.add_argument("--max-cache-gib", type=float, default=MAX_CACHE / GIB)
    run.add_argument("cargo", nargs=argparse.REMAINDER)
    sub.add_parser("inspect")
    sub.add_parser("cleanup-incremental")
    args = parser.parse_args()
    try:
        target = target_directory(requested=args.target_dir, isolation_reason=args.isolation_reason)
        if args.action == "run":
            if args.min_free_gib < 0 or args.max_cache_gib <= 0:
                parser.error("disk limits must be nonnegative/free and positive/cache")
            arguments = args.cargo[1:] if args.cargo[:1] == ["--"] else args.cargo
            return run_cargo(arguments, target, args.output.resolve(), int(args.min_free_gib * GIB),
                             int(args.max_cache_gib * GIB), args.isolation_reason)
        report = disk_state(target) if args.action == "inspect" else cleanup_incremental(target)
        print(json.dumps(report, indent=2))
        return 0
    except (CacheError, OSError) as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
