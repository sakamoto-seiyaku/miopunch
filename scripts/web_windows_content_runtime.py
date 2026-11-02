#!/usr/bin/env python3
"""Owned Windows WebView2 process for provider-bound content fixtures."""
import hashlib
import json
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import serialization

from web_windows_process import WindowsChild

RUNTIME = Path(r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\155.0.4283.62")
RUNTIME_SHA = "93acbcad0b6f390ae1ce6a1dbf1e09e165520a7f1196eb22323afa64a9bba96b"
HOST_SHA = "3f7c4de580909474f8a153a745ee6d49b90f1c490790b724db96365cfc363f30"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class WindowsContentHost:
    platform = "Windows WebView2"

    def __init__(self, runner, gateway, label="content"):
        self.runner, self.label = runner, label
        self.child, self.ready, self.closed = None, None, None
        runner.hosts.append(self)
        root, output, private = runner.root, runner.output, runner.private
        assert sha(RUNTIME / "msedgewebview2.exe") == RUNTIME_SHA, "existing WebView2 runtime changed"
        host = root / "host"
        assembly = root / "host-handle/Miopunch.WindowsWebContentHost.dll"
        assert sha(assembly) == HOST_SHA, "qualified typed-host pin changed"
        leaf = x509.load_pem_x509_certificate(gateway.certificate.read_bytes())
        fingerprint = hashlib.sha256(leaf.public_bytes(serialization.Encoding.DER)).hexdigest()
        options = dict(gateway=gateway.bootstrap, gatewayCertificateSHA256=fingerprint,
                       profile=str(private / (label + "-profile")), runtimeDirectory=str(RUNTIME),
                       runtimeVersion="155.0.4283.62",
                       runtimeExecutable=dict(path=str(RUNTIME / "msedgewebview2.exe"), sha256=RUNTIME_SHA))
        for field, name in [("coreAssembly", "Microsoft.Web.WebView2.Core.dll"),
                            ("formsAssembly", "Microsoft.Web.WebView2.WinForms.dll"),
                            ("loader", "WebView2Loader.dll")]:
            options[field] = dict(path=str(host / name), sha256=sha(host / name))
        options["hostAssembly"] = dict(path=str(assembly), sha256=HOST_SHA)
        self.profile = Path(options["profile"])
        assert not self.profile.exists(), "content profile must be fresh"
        path = output / (label + "-options.json")
        path.write_text(json.dumps(options), encoding="utf-8")
        powershell = Path(runner.environment["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
        children = []
        self.child_registry = children
        self.child = WindowsChild([str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive", "-STA", "-File",
                                   str(host / "windows_web_content_host.ps1"), "-OptionsPath", str(path)],
                                  runner.environment, output, label, owners=children)
        self.ready = self.child.wait_ready()
        assert self.ready["native_management_handlers"] is False
        assert self.ready["runtime_version"] == "155.0.4283.62"
        runner.record("controller", "actual_content_ready", dict(platform=self.platform, label=label,
                      ready=self.ready, original_pid=self.child.process.pid,
                      original_handle=self.child.handle))

    def evaluate(self, expression):
        return self.child.call("evaluate", expression=expression)

    def close(self):
        if self.closed is not None:
            return self.closed
        if self.child is None:
            if not getattr(self, "child_registry", []):
                return dict(owner=self.label, process_created=False)
            return self.child_registry[0].cleanup()
        failure = None
        try:
            if self.child.process.poll() is None and self.ready is not None:
                assert self.child.call("close") == {"closed": True}
            joined = self.child.join()
            closed = [event for event in self.child.events if event.get("event") == "closed"]
            assert len(closed) == 1 and closed[0]["host_failed"] is False
            assert closed[0]["input_joined"] is True and closed[0]["engine_exit_observed"] is True
            assert sha(RUNTIME / "msedgewebview2.exe") == RUNTIME_SHA, "WebView2 runtime changed during actual content run"
            self.closed = dict(owner=self.label, process=joined, engine=closed[0], runtime_hash_unchanged=True)
            self.runner.record("controller", "actual_content_owner_settled", self.closed)
            return self.closed
        except BaseException as error:
            failure = error
        finally:
            if failure is not None:
                try:
                    self.child.cleanup()
                except BaseException as cleanup_error:
                    self.runner.record("controller", "content_cleanup_failure",
                                       dict(label=self.label, type=type(cleanup_error).__name__))
        raise failure
