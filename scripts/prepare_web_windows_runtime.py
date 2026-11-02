#!/usr/bin/env python3
"""Freeze, acquire and extract one app-local Windows Web fixture runtime."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import stat
import sys
from urllib.request import Request, urlopen
import zipfile

from pin_artifact import pin_artifact

ROOT = Path(__file__).resolve().parents[1]
VERSIONS = {
    "aiohttp": "3.13.3", "cryptography": "46.0.5", "aiohappyeyeballs": "2.7.1",
    "aiosignal": "1.4.0", "attrs": "25.4.0", "frozenlist": "1.8.0",
    "multidict": "6.7.1", "propcache": "0.4.1", "yarl": "1.22.0",
    "idna": "3.18", "cffi": "2.0.0", "pycparser": "2.23",
}
PYTHON = "3.13.17"
PYTHON_SHA256 = "9e35319bffedab8b36ad44d8f377cb4f1733bcec257a0b6d04d94f01486ddb38"
CAP = 128 * 1024 * 1024


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_url(url, limit):
    with urlopen(Request(url, headers={"User-Agent": "miopunch-owned-windows-fixture"}), timeout=30) as stream:
        value = stream.read(limit + 1)
    if len(value) > limit:
        raise RuntimeError("primary metadata exceeds declared bound")
    return value


def freeze(output):
    # Local primary-metadata resolution only. No package code is imported.
    from packaging.markers import default_environment
    from packaging.requirements import Requirement
    from packaging.specifiers import SpecifierSet
    from packaging.tags import compatible_tags, cpython_tags
    from packaging.utils import canonicalize_name, parse_wheel_filename
    from packaging.version import Version
    output.mkdir(parents=True, exist_ok=False)
    environment = default_environment()
    environment.update(python_version="3.13", python_full_version=PYTHON, os_name="nt",
                       sys_platform="win32", platform_machine="AMD64", platform_system="Windows", extra="")
    tags = set(cpython_tags((3, 13), abis=["cp313"], platforms=["win_amd64"]))
    tags.update(compatible_tags((3, 13), interpreter="cp313", platforms=["win_amd64"]))
    selected, requirements = [], {}
    for name, version in VERSIONS.items():
        metadata_url = f"https://pypi.org/pypi/{name}/{version}/json"
        raw = read_url(metadata_url, 2 * 1024 * 1024)
        (output / (name + ".json")).write_bytes(raw)
        data = json.loads(raw)
        assert data["info"]["version"] == version
        candidates = []
        for item in data["urls"]:
            if item["packagetype"] != "bdist_wheel" or item["yanked"]:
                continue
            wheel_name, wheel_version, _, wheel_tags = parse_wheel_filename(item["filename"])
            if not wheel_tags.intersection(tags) or (item["requires_python"] and
                    not SpecifierSet(item["requires_python"]).contains(PYTHON)):
                continue
            assert canonicalize_name(wheel_name) == canonicalize_name(name) and str(wheel_version) == version
            candidates.append(item)
        if not candidates:
            raise RuntimeError("no compatible pinned Windows wheel: " + name)
        item = min(candidates, key=lambda item: (item["size"], item["filename"]))
        assert item["url"].startswith("https://files.pythonhosted.org/")
        selected.append(dict(name=name, version=version, filename=item["filename"], url=item["url"],
                             size=item["size"], sha256=item["digests"]["sha256"], metadata_url=metadata_url,
                             metadata_sha256=hashlib.sha256(raw).hexdigest(), kind="wheel"))
        active = []
        for value in data["info"]["requires_dist"] or []:
            requirement = Requirement(value)
            if requirement.marker and not requirement.marker.evaluate(environment):
                continue
            dependency = canonicalize_name(requirement.name)
            assert dependency in VERSIONS, (name, value, "dependency closure missing")
            assert Version(VERSIONS[dependency]) in requirement.specifier, (name, value)
            active.append(value)
        requirements[name] = active
    python_url = f"https://www.python.org/ftp/python/{PYTHON}/python-{PYTHON}-embed-amd64.zip"
    with urlopen(Request(python_url, method="HEAD"), timeout=30) as stream:
        size = int(stream.headers["Content-Length"])
    selected.insert(0, dict(name="cpython", version=PYTHON, filename=python_url.rsplit("/", 1)[1],
                           url=python_url, size=size, sha256=PYTHON_SHA256, kind="runtime",
                           primary_hash_source=f"https://www.python.org/downloads/release/python-{PYTHON.replace('.', '')}/"))
    assert sum(item["size"] for item in selected) <= CAP
    manifest = dict(python=PYTHON, platform="win_amd64", abi="cp313", items=selected,
                    active_requirements=requirements, marker_environment=environment,
                    compressed_bytes=sum(item["size"] for item in selected), max_cohort_bytes=CAP,
                    managed_graph=None, Rust_graph=None, helper_sha256=sha(__file__),
                    pin_helper_sha256=sha(ROOT / "scripts/pin_artifact.py"))
    (output / "graph.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"graph": str(output / "graph.json"), "archives": len(selected),
                      "bytes": manifest["compressed_bytes"]}))


def graph(path):
    value = json.loads(path.read_text())
    assert value["helper_sha256"] == sha(__file__) and value["pin_helper_sha256"] == sha(ROOT / "scripts/pin_artifact.py")
    assert sum(item["size"] for item in value["items"]) == value["compressed_bytes"] <= CAP
    return value


def acquire(path, staging, pins):
    value = graph(path)
    staging.mkdir(parents=True, exist_ok=False)
    pins.mkdir(parents=True, exist_ok=False)
    for item in value["items"]:
        destination = staging / item["filename"]
        payload = read_url(item["url"], item["size"])
        assert len(payload) == item["size"] and hashlib.sha256(payload).hexdigest() == item["sha256"], item["filename"]
        destination.write_bytes(payload)
        pin_artifact(destination, pins / item["filename"])
        assert sha(pins / item["filename"]) == item["sha256"]
    print(json.dumps({"acquired": len(value["items"]), "bytes": value["compressed_bytes"], "Cargo": False}))


def extract(path, pins, output):
    value = graph(path)
    members, total = [], 0
    for item in value["items"]:
        archive = pins / item["filename"]
        assert sha(archive) == item["sha256"], item["filename"]
        with zipfile.ZipFile(archive) as source:
            for entry in source.infolist():
                name = PurePosixPath(entry.filename)
                mode = entry.external_attr >> 16
                if (name.is_absolute() or ".." in name.parts or "\\" in entry.filename or
                        ":" in entry.filename or stat.S_ISLNK(mode)):
                    raise RuntimeError("unsafe archive member")
                total += entry.file_size
                assert total <= 192 * 1024 * 1024
                if not entry.is_dir():
                    members.append((item, entry.filename))
    output.mkdir(parents=True, exist_ok=False)
    hashes = {}
    for item, name in members:
        prefix = "python" if item["kind"] == "runtime" else "packages"
        relative = str(PurePosixPath(prefix) / name)
        target = output / relative
        assert not target.exists(), relative
        target.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(pins / item["filename"]) as archive:
            target.write_bytes(archive.read(name))
        hashes[relative] = sha(target)
    # Explicit local paths, no site import or .pth/customize startup.
    pth = output / "python/python313._pth"
    pth.write_text("python313.zip\n.\n../packages\n../controllers\n", encoding="ascii")
    hashes["python/python313._pth"] = sha(pth)
    receipt = dict(graph_sha256=sha(path), extracted_archive_bytes=total,
                   extracted_files=hashes, configured_no_site=True, Windows_execution=False)
    (output / "runtime-manifest.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"files": len(hashes), "bytes": total, "tree": str(output)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["freeze", "acquire", "extract"])
    parser.add_argument("--output", type=Path)
    parser.add_argument("--graph", type=Path)
    parser.add_argument("--staging", type=Path)
    parser.add_argument("--pins", type=Path)
    args = parser.parse_args()
    if args.mode == "freeze": freeze(args.output)
    elif args.mode == "acquire": acquire(args.graph, args.staging, args.pins)
    else: extract(args.graph, args.pins, args.output)


if __name__ == "__main__":
    main()
