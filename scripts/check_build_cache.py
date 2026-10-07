"""Keep controlled check compilation and artifact pinning in the shared lease."""

from contextlib import contextmanager
import json
from pathlib import Path
import time

import cargo_cache


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
