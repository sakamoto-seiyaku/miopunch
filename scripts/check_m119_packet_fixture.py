#!/usr/bin/env python3
"""Check a frozen M119 packet's Linux desktop fixture before remote execution."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace


sys.dont_write_bytecode = True


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def packet_inputs(directory: Path) -> tuple[dict, dict, dict]:
    packet = json.loads((directory / "packet.json").read_text())
    plan_path = directory / "plan.json"
    plan = json.loads(plan_path.read_text())
    assert digest(plan_path) == packet["plan_sha256"]

    tree = {
        path.relative_to(directory).as_posix(): [path.stat().st_size, digest(path)]
        for path in directory.rglob("*")
        if path.is_file()
        and path.name != "packet.json"
        and path.relative_to(directory).parts[0] != "runtime"
    }
    assert len(tree) == packet["source_tree_files"]
    encoded = json.dumps(tree, sort_keys=True, separators=(",", ":")).encode()
    assert hashlib.sha256(encoded).hexdigest() == packet["source_tree_sha256"]

    for row in plan["payload"].values():
        path = Path(row["path"])
        assert path.is_file() and path.stat().st_size == row["bytes"]
        assert digest(path) == row["sha256"]

    deployment = Path(plan["deployment_manifest"]["path"])
    assert digest(deployment) == plan["deployment_manifest"]["sha256"]
    manifest = json.loads(deployment.read_text())
    for name, expected in manifest["source_pins"].items():
        path = Path(name)
        assert path.is_file() and digest(path) == expected, name
    for name in manifest["module_origins"]["linux"].values():
        assert str(Path(name).resolve()) in manifest["source_pins"], name
    return packet, plan, manifest


def desktop_fixture(directory: Path, plan: dict, manifest: dict) -> dict:
    if os.name == "nt":
        raise RuntimeError("Linux desktop fixture requires a Linux host")
    wrappers = [
        Path(name)
        for name in manifest["source_pins"]
        if Path(name).name == "sandbox_bwrap.py"
    ]
    assert len(wrappers) == 1
    wrapper = wrappers[0].resolve()
    assert digest(wrapper) == manifest["source_pins"][str(wrapper)]

    sys.path.insert(0, str(directory))
    spec = importlib.util.spec_from_file_location("m119_packet_worker_preflight", directory / "worker.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    class PrivateDesktop:
        def __init__(self, view, _name):
            self.root = view.private / _name
            self.root.mkdir()
            self.env = {}

        def namespace_command(self):
            return []

    class PrivateBus:
        def __init__(self, _view, _desktop):
            pass

    runtime = SimpleNamespace(PrivateDesktop=PrivateDesktop, PrivateBus=PrivateBus)
    case = plan["cases"]["windows-provider"]

    with tempfile.TemporaryDirectory(prefix="m119-") as scratch:
        def actor(name: str, pins: dict):
            instance = module.Worker.__new__(module.Worker)
            instance.plan = dict(manifest, source_pins=pins)
            instance.case = case
            instance.private = Path(scratch) / name
            instance.private.mkdir()
            instance.controller_sources = {}
            instance.record = lambda *_args, **_kwargs: None
            return instance

        actual_private = Path(manifest["linux"]["private_batch"]) / case["id"]
        padding = max(0, len(os.fsencode(actual_private)) - len(os.fsencode(Path(scratch) / "positive")))
        selected = actor("positive" + "x" * padding, manifest["source_pins"])
        selected.prepare_linux_desktop(runtime)
        assert selected.controller_sources[str(wrapper)] == digest(wrapper)

        selected.output = Path(scratch) / "output"
        selected.output.mkdir()
        selected.hosts = []

        class WebKitHost:
            def __init__(self, view, desktop, _gateway):
                view.hosts.append(self)
                self.socket = Path(desktop.env["XDG_RUNTIME_DIR"]) / "webkitgtk/bus-proxy-XXXXXX"
                assert len(os.fsencode(self.socket)) <= 107, self.socket

        previous = sys.modules.get("web_content_runtime")
        sys.modules["web_content_runtime"] = SimpleNamespace(WebKitHost=WebKitHost)
        try:
            content = selected.host(None, "content")
            control = selected.host(None, "management-control")
            assert content.socket != control.socket
        finally:
            if previous is None:
                del sys.modules["web_content_runtime"]
            else:
                sys.modules["web_content_runtime"] = previous

        missing = actor(
            "missing",
            {name: value for name, value in manifest["source_pins"].items() if name != str(wrapper)},
        )
        try:
            missing.prepare_linux_desktop(runtime)
        except AssertionError:
            pass
        else:
            raise AssertionError("missing sandbox pin was accepted")

        wrong = actor("wrong", dict(manifest["source_pins"], **{str(wrapper): "0" * 64}))
        try:
            wrong.prepare_linux_desktop(runtime)
        except AssertionError:
            pass
        else:
            raise AssertionError("incorrect sandbox pin was accepted")

    return {
        "sandbox_wrapper": str(wrapper),
        "sandbox_wrapper_sha256": digest(wrapper),
        "positive": True,
        "missing_pin_rejected": True,
        "wrong_hash_rejected": True,
        "content_and_management_socket_paths_bounded": True,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packet_directory", type=Path)
    args = parser.parse_args()
    directory = args.packet_directory.resolve()
    packet, plan, manifest = packet_inputs(directory)
    fixture = desktop_fixture(directory, plan, manifest)
    print(
        json.dumps(
            {
                "status": "PASS",
                "packet_sha256": digest(directory / "packet.json"),
                "mode": packet["mode"],
                "source_pin_count": len(manifest["source_pins"]),
                "payload_file_count": len(plan["payload"]),
                "fixture": fixture,
                "remote_actions": 0,
                "builds": 0,
            }
        )
    )


if __name__ == "__main__":
    main()
