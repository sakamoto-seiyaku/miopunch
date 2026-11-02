#!/usr/bin/env python3
"""Actual Windows WebView2 loopback page/private-pipe/engine-exit qualification."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import sys
import time

from cryptography import x509
from cryptography.hazmat.primitives import serialization
from web_content_gateway import ContentGateway
from web_windows_process import WindowsChild

RUNTIME = Path(r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\155.0.4283.62")
RUNTIME_SHA = "93acbcad0b6f390ae1ce6a1dbf1e09e165520a7f1196eb22323afa64a9bba96b"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", default="deployment-manifest.json")
    parser.add_argument("--host-assembly", type=Path)
    args = parser.parse_args()
    if os.name != "nt" or sys.maxsize <= 2**32 or not sys.flags.isolated or not sys.flags.no_site:
        raise RuntimeError("isolated x64 Windows runtime required")
    root, output = args.root.resolve(), args.output.resolve()
    assert output.is_relative_to(root)
    output.mkdir(parents=True, exist_ok=False)
    manifest_path = (root / args.manifest).resolve()
    assert manifest_path.is_relative_to(root)
    manifest = json.loads(manifest_path.read_text())
    assert all(sha(root / name) == digest for name, digest in manifest["files"].items())
    assert sha(RUNTIME / "msedgewebview2.exe") == RUNTIME_SHA, "existing runtime changed, no launch"
    private = output / "private"
    private.mkdir()
    for name in ["temp", "local-appdata", "appdata"]:
        (private / name).mkdir()
    windows = Path(os.environ["SystemRoot"])
    powershell = windows / "System32/WindowsPowerShell/v1.0/powershell.exe"
    environment = dict(SystemRoot=str(windows), WINDIR=str(windows),
                       PATH=str(powershell.parent) + ";" + str(windows / "System32"),
                       TEMP=str(private / "temp"), TMP=str(private / "temp"),
                       LOCALAPPDATA=str(private / "local-appdata"), APPDATA=str(private / "appdata"))
    page = b'<!doctype html><html><head><title>M119 owned Windows fixture</title></head><body><p id="proof">M119 real WebView2 loopback</p><script>window.fixtureLoaded=true;</script></body></html>'
    callback_observed = []

    def request(arguments):
        assert arguments["url"] == "https://127.0.0.1:9/page"
        callback_observed.append({"method": arguments["method"], "url": arguments["url"]})
        return dict(status=200, headers=[["Content-Type", "text/html; charset=utf-8"]],
                    body=base64.b64encode(page).decode())

    owners, children, failures, cleanup = [], [], [], []
    primary = None
    result = dict(result="FAILED", scope="actual Windows host plumbing only, no SDK/provider/ACL business",
                  runtime_version="155.0.4283.62", runtime_sha256=RUNTIME_SHA,
                  engine_background_destinations="UNKNOWN", whole_host_zero_egress=False,
                  Python=sys.version, site_imported="site" in sys.modules)
    try:
        gateway = ContentGateway("https://127.0.0.1:9/page", request, private, owners)
        leaf = x509.load_pem_x509_certificate(gateway.certificate.read_bytes())
        leaf_hash = hashlib.sha256(leaf.public_bytes(serialization.Encoding.DER)).hexdigest()
        host = root / "host"
        options = dict(gateway=gateway.bootstrap, gatewayCertificateSHA256=leaf_hash,
                       profile=str(private / "content-profile"), runtimeDirectory=str(RUNTIME),
                       runtimeVersion="155.0.4283.62",
                       runtimeExecutable=dict(path=str(RUNTIME / "msedgewebview2.exe"), sha256=RUNTIME_SHA))
        for field, name in [("coreAssembly", "Microsoft.Web.WebView2.Core.dll"),
                            ("formsAssembly", "Microsoft.Web.WebView2.WinForms.dll"),
                            ("loader", "WebView2Loader.dll"), ("hostAssembly", "Miopunch.WindowsWebContentHost.dll")]:
            options[field] = dict(path=str(host / name), sha256=sha(host / name))
        if args.host_assembly:
            assembly = args.host_assembly.resolve()
            assert assembly.is_relative_to(root) and str(assembly.relative_to(root)).replace("\\", "/") in manifest["files"]
            options["hostAssembly"] = dict(path=str(assembly), sha256=sha(assembly))
        bad = dict(options, hostAssembly=dict(options["hostAssembly"], sha256="0" * 64),
                   profile=str(private / "negative-profile-must-not-exist"))
        bad_path = output / "negative-options.json"
        bad_path.write_text(json.dumps(bad), encoding="utf-8")
        negative = WindowsChild([str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive", "-STA", "-File",
                                 str(host / "windows_web_content_host.ps1"), "-OptionsPath", str(bad_path)],
                                environment, output, "negative")
        children.append(negative)
        negative.join(expected_exit=1)
        assert not Path(bad["profile"]).exists() and not negative.events
        result["early_negative"] = dict(pin_mismatch_rejected=True, profile_not_created=True, host_events=[])
        options_path = output / "options.json"
        options_path.write_text(json.dumps(options), encoding="utf-8")
        content = WindowsChild([str(powershell), "-NoLogo", "-NoProfile", "-NonInteractive", "-STA", "-File",
                               str(host / "windows_web_content_host.ps1"), "-OptionsPath", str(options_path)],
                              environment, output, "content")
        children.append(content)
        result["ready"] = content.wait_ready()
        deadline = time.monotonic() + 10
        while True:
            observed = content.call("evaluate", expression="({title:document.title,text:document.getElementById('proof')?.textContent,loaded:window.fixtureLoaded===true,managementBridge:!!window.chrome?.webview})")
            if observed.get("loaded"):
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("actual loopback fixture did not load within existing content observation bound")
            time.sleep(.02)
        assert observed["title"] == "M119 owned Windows fixture" and observed["text"] == "M119 real WebView2 loopback"
        # A WebView2 chrome.webview object is engine plumbing, not evidence of
        # a node-management handler. Full management/IPC effects are separate
        # real business-isolation checks, not this initial page/pipe assertion.
        assert result["ready"]["native_management_handlers"] is False
        result["actual_page"] = observed
        assert callback_observed == [{"method": "GET", "url": "https://127.0.0.1:9/page"}]
        assert content.call("close") == {"closed": True}
        result["host_join"] = content.join()
        closed = [event for event in content.events if event.get("event") == "closed"]
        assert len(closed) == 1 and closed[0]["host_failed"] is False and closed[0]["input_joined"] is True and closed[0]["engine_exit_observed"] is True
        result["closed"] = closed[0]
        result["runtime_checked_pre_and_post"] = sha(RUNTIME / "msedgewebview2.exe") == RUNTIME_SHA
        assert result["runtime_checked_pre_and_post"]
        result["result"] = "PASS"
    except BaseException as failure:
        primary = failure
        result["primary_failure"] = dict(type=type(failure).__name__, message=str(failure))
    finally:
        for child in reversed(children):
            try:
                cleanup.append(child.cleanup())
            except BaseException as failure:
                failures.append(dict(owner=child.label, type=type(failure).__name__, message=str(failure)))
        for owner in reversed(owners):
            try:
                cleanup.append(owner.close())
            except BaseException as failure:
                failures.append(dict(owner="gateway", type=type(failure).__name__, message=str(failure)))
        result.update(cleanup=cleanup, cleanup_errors=failures,
                      callback_observed=callback_observed,
                      source_drift=[name for name, digest in manifest["files"].items() if sha(root / name) != digest])
        if failures or result["source_drift"]:
            result["result"] = "FAILED"
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result), flush=True)
    if primary:
        raise primary
    if result["result"] != "PASS":
        raise RuntimeError("actual Windows chain cleanup or custody failed")


if __name__ == "__main__":
    main()
