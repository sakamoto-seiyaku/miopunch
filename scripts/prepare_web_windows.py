#!/usr/bin/env python3
"""Prepare verified Windows Web pins and content-host source; never deploy/run."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from build_native_client import ROOT
from pin_artifact import pin_artifact


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--host-receipt",type=Path,required=True)
    parser.add_argument("--sdk-selected",type=Path,required=True)
    args = parser.parse_args()
    receipt = args.source_receipt.resolve()
    build = json.loads(receipt.read_text())
    assert build["exit"] == build["compilerCleanupExit"] == 0 and build["platform"] == "windows"
    binary = Path(build["pin"])
    assert sha(binary) == build["sha256"]
    payload = binary.read_bytes()
    offset = struct.unpack_from("<I", payload, 0x3c)[0]
    assert payload[:2] == b"MZ" and payload[offset:offset+4] == b"PE\0\0"
    assert struct.unpack_from("<H", payload, offset+4)[0] == 0x8664
    selected = json.loads(receipt.with_name("selected-dependency-inputs.json").read_text())["selectedRepositoryInputs"]
    assert all(sha(ROOT/name) == digest for name,digest in selected.items()), "current selected Windows SDK source differs"
    host = json.loads(args.host_receipt.read_text())
    assert host["exit"] == 0 and host["allOwnedTerminal"] and not host["WindowsExecuted"]
    assert sha(host["pin"]) == host["sha256"]
    host_graph = json.loads(args.host_receipt.with_name("inputs.json").read_text())
    for name in ["windows_web_content_host.cs","windows_web_content_host.ps1"]:
        frozen = next(item for item in host_graph["files"] if item["path"] == str(ROOT/"scripts"/name))
        assert sha(frozen["path"]) == frozen["sha256"], "typed host source differs"
    sdk = json.loads(args.sdk_selected.read_text())
    assert all(sha(item["path"]) == item["sha256"] for item in sdk), "SDK/reference source differs"
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    pin_artifact(binary, output/"miopunch-client.exe")
    pin_artifact(Path(host["pin"]),output/"Miopunch.WindowsWebContentHost.dll")
    bundle = {}
    for name in ["Microsoft.Web.WebView2.Core.dll","Microsoft.Web.WebView2.WinForms.dll","WebView2Loader.dll"]:
        item = next(item for item in sdk if Path(item["path"]).name == name)
        pin_artifact(Path(item["path"]),output/name)
        bundle[name] = {"sha256":item["sha256"],"sourcePackage":item["package"],"sourceVersion":item["version"]}
    names = ["windows_web_content_host.ps1", "windows_web_content_host.cs", "web_content_gateway.py", "web_tls_fixture.py",
             "web_process_owner.py", "forwarding_local_resources.py", "check_forwarding.py",
             "prepare_web_windows.py","build_windows_web_host.py","compile_windows_web_host.ps1"]
    hashes = {}
    for name in names:
        source = ROOT/"scripts"/name
        (output/name).write_bytes(source.read_bytes())
        hashes[name] = sha(source)
    manifest = {"SDKReceipt":str(receipt),"SDKReceiptSHA256":sha(receipt),"exeSHA256":build["sha256"],
                "SDKSourceArchiveSHA256":sha(receipt.with_name("source.tar.gz")),"selectedInputs":selected,
                "hostSources":hashes,"actualWindowsExecuted":False,"SSHExecuted":False,"deployed":False,
                "typedHostReceipt":str(args.host_receipt.resolve()),"typedHostSHA256":host["sha256"],
                "typedHostCompiledLocally":True,"WebView2Bundle":bundle,"WebView2BundleAvailable":True,
                "fixedWebView2RuntimeAvailable":False,"gatewayWindowsBindingQualified":False,"WindowsNetworkContainmentQualified":False,
                "preparationOnly":True,"readyForWindowsExecution":False}
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2)+"\n")
    missing = {"path":"REQUIRES_ACTUAL_OWNED_WINDOWS_FILE","sha256":"REQUIRES_VERIFIED_SHA256"}
    options = {"gateway":"https://127.0.0.1:REQUIRES_OWNED_PORT/__miopunch_open/REQUIRES_FRESH_SESSION",
               "gatewayCertificateSHA256":"REQUIRES_EXACT_PUBLIC_LEAF_DER_SHA256", "profile":"REQUIRES_FRESH_PRIVATE_CONTENT_PROFILE",
               "coreAssembly":missing,"formsAssembly":missing,"loader":missing,"runtimeExecutable":missing,
               "runtimeDirectory":"REQUIRES_VERIFIED_EXISTING_FIXED_RUNTIME_DIRECTORY", "runtimeVersion":"REQUIRES_VERIFIED_VERSION"}
    for field,name in [("coreAssembly","Microsoft.Web.WebView2.Core.dll"),("formsAssembly","Microsoft.Web.WebView2.WinForms.dll"),
                       ("loader","WebView2Loader.dll"),("hostAssembly","Miopunch.WindowsWebContentHost.dll")]:
        options[field] = {"path":"OWNED_WINDOWS_PREPARATION_ROOT\\"+name,"sha256":sha(output/name)}
    (output/"owned-options.NOT-READY.json").write_text(json.dumps(options,indent=2)+"\n")
    action = f"""# Windows Web preparation — UNCOMPLETED / DO NOT EXECUTE

This preparation is local source and an immutable Windows x64 CLI pin. It is
not authorization, deployment, a Windows run, or an in-app Windows acceptance.
No SSH destination is selected. The old unanswered Windows request remains
unanswered; a synthetic continuation or this file does not grant it.

SDK binary SHA256: `{build['sha256']}`. Full graph, source archive, real foreign
overlay and host-source hashes are in manifest.json and the originating receipt.
The pin remains immutable. Transform only a separate owned deployment copy.

Concrete future content-host command, only after a fresh actual Windows grant,
qualified whole-scenario local network boundary, and all missing inputs below:

    powershell.exe -NoProfile -STA -File windows_web_content_host.ps1 -OptionsPath owned-options.json

The host is an embedded WinForms WebView2, not an external/system browser.
Its private pipe accepts evaluate/close for technical acceptance; opened content
has no management message handler or host object. Navigation stays at the exact
owned gateway origin. A gateway certificate exception requires that origin and
the exact public leaf DER pin; provider-to-target TLS remains SDK-validated.
Fresh content storage is distinct from the node store. These source properties
are not actual isolation evidence.

The official WebView2 SDK Core/WinForms/loader assemblies1.0.2849.39 and the typed
CSharp5/x64 host are pinned here. The host compiled against checked net48
reference assemblies with no compile or wrapper AST parse errors. This did not
load the host assembly, instantiate WebView2 or invoke native methods. Only SDK
assemblies are deployment inputs; framework REFERENCE assemblies are not.

Missing actual prerequisites: independently verified existing fixed WebView2
runtime/version pins and .NET4.8 runtime compatibility; a fresh private
profile; a real provider-bound Windows gateway using the public Native Web
begin/result/cancel and WebSocket methods; coordinated gateway/SDK/content-host
ownership and a qualified Windows network boundary including engine background
traffic. The prior Chromium GCM/Auth401 incident showed that a private profile
and background-networking flags alone are not a containment proof. No actual
Windows/WebView2 launch is authorized until a genuine scoped owned whole-scenario
network boundary is qualified. A boolean options flag is not such proof.
No laptop dependency install, certificate-store change, registry
write, firewall/route/DNS/VPN change or security bypass is authorized here.
The NOT-READY options intentionally fail before runtime creation. Do not fill
them with placeholders and call the result ready.

The current Windows gateway binding and the complete host are UNQUALIFIED.
The portable gateway and WebView2 adapter source are preparation for that work;
their combined Windows lifecycle has not run. Actual assembly compatibility,
profile/runtime override behavior and engine teardown must be observed there.
The Python dependency sources are included with the gateway. Its selected
monotonic-future helper is portable; the included Linux /proc process-ownership
implementation is not a qualified Windows owner adapter and must not be used
as one. Actual Windows pipe/process ownership remains part of the missing binding.

Whole M119 Windows requirements remain: both Windows/Linux provider roles and
independent Browser/WASM, actual provider HTTP(S) targets, in-app resources,
navigation/redirect/login/Cookie, bidirectional WebSocket, real origin/storage/
native-IPC isolation with only fresh management canaries, off/identity/target/
current single-ACL/guest original deadline/exit/revoke, pending/backpressured
queues and bounded cancel/close/drop/reopen, truthful failures and every owned
process/reader/transport joined. Source parsing, this PE, Linux, Wine and M121
cannot substitute for them. No criteria are moved to M121; task stays In Progress.

API sources used to prepare the adapter:
- [WebView2 initialization](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.winforms.corewebview2creationproperties)
- [Content/host separation settings](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2settings)
- [Scoped certificate handler](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2.servercertificateerrordetected)
"""
    (output/"ACTION.md").write_text(action)
    assert sha(output/"miopunch-client.exe") == build["sha256"]
    print(json.dumps({"prepared":str(output),"manifestSHA256":sha(output/"manifest.json"),
                      "actualWindows":False,"SSH":False,"ready":False}))


if __name__ == "__main__":
    main()
