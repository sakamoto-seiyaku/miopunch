#!/usr/bin/env python3
"""Real Windows SDK/provider-bound gateway through the qualified WebView2 host."""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path
import ssl
import sys
import time
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

from web_protocol_target import WebTarget
from web_windows_content_runtime import WindowsContentHost
from web_windows_gateway import WindowsWebNode, provider_gateway

SDK_GRAPHS = {
    "sdk11": ("sdk/miopunch-client-sdk11.exe",
              "6de63b7756d7e9d627d6437bdc5c5c6fea22dc0b326ba44d4f2d58b1a7159690"),
    "sdk11-init-diagnostic": ("sdk/miopunch-client-sdk11-init-diagnostic.exe",
                              "70aba7ef847b4ca4409c28d643ba6ac1a5bf32ecbc28b6ed48c70529cd1c1f40"),
}


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class IntegrationRunner:
    def __init__(self, args):
        self.args = args
        assert os.name == "nt" and sys.maxsize > 2**32 and sys.flags.isolated and sys.flags.no_site
        self.root, self.output = args.root.resolve(), args.output.resolve()
        assert self.output.is_relative_to(self.root)
        self.output.mkdir(parents=True, exist_ok=False)
        self.manifest = json.loads((self.root / args.manifest).read_text())
        assert all(sha(self.root / name) == digest for name, digest in self.manifest["files"].items())
        relative, self.sdk_sha = SDK_GRAPHS[args.sdk_graph]
        self.binary = self.root / relative
        assert sha(self.binary) == self.sdk_sha
        self.private = self.output / "private"
        self.private.mkdir()
        for name in ["temp", "local-appdata", "appdata"]:
            (self.private / name).mkdir()
        windows = Path(os.environ["SystemRoot"])
        self.environment = dict(SystemRoot=str(windows), WINDIR=str(windows), PATH=str(windows / "System32"),
                                TEMP=str(self.private / "temp"), TMP=str(self.private / "temp"),
                                LOCALAPPDATA=str(self.private / "local-appdata"), APPDATA=str(self.private / "appdata"))
        self.configuration = self.private / "resources.json"
        self.configuration.write_text(json.dumps(dict(defaults=dict(revision=2, resources=[]), replacement=[],
            disabled=[], disabled_uses=["Observation", "Lookup", "Signaling"], lan_discovery="Disabled", port_mapping="Disabled")))
        self.nodes, self.targets, self.gateways, self.hosts = [], [], [], []
        self.records = []

    def record(self, actor, operation, value):
        item = dict(actor=actor, operation=operation, value=value, monotonic=time.monotonic())
        self.records.append(item)
        with (self.output / "events.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(item) + "\n")

    def until(self, predicate, label, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.05)
        raise AssertionError(label)

    def member(self, provider, label):
        invitation = self.private / (label + "-invitation")
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        consumer = WindowsWebNode(self.binary, self.private, self.output, label, self.environment,
                                  self.configuration, self.nodes, action="join", invitation=invitation)
        consumer.call("request", path=str(invitation))
        provider.call("service_sync", address=consumer.address)
        assert provider.identity != consumer.identity
        self.record("controller", "actual_distinct_members", dict(provider=provider.identity, consumer=consumer.identity))
        return consumer

    def scenario(self):
        target = WebTarget(self, tls=True)
        verification = ssl.create_default_context(cafile=target.certificate)
        with build_opener(ProxyHandler({}), HTTPSHandler(context=verification)).open(target.url, timeout=3) as response:
            assert response.read() == b"owned M119 web target"
        asyncio.run(target.socket_prerequisite())
        self.record("controller", "actual_target_preconditions", dict(HTTPS_exact=True, nativeTLS_validation=True,
                                                                      WebSocket_duplex_exact=True))
        provider = WindowsWebNode(self.binary, self.private, self.output, "provider", self.environment,
                                  self.configuration, self.nodes)
        consumer = self.member(provider, "consumer")
        provider.call("web_enable", targets=[target.url], trust_roots_pem=[target.trust_root_pem])
        gateway_private = self.private / "content-gateway"
        gateway_private.mkdir()
        gateway = provider_gateway(consumer, provider.address, target.url + "page", gateway_private, self.gateways)
        gateway.record = lambda operation, value: self.record("gateway", operation, value)
        host = WindowsContentHost(self, gateway)
        self.until(lambda: host.evaluate("window.fixtureFetch===\"owned M119 fetched resource\""),
                   "actual Windows provider-bound page script/fetch did not complete")
        observed = host.evaluate("({css:getComputedStyle(document.getElementById('resource')).color,image:document.getElementById('image').naturalWidth})")
        assert observed == dict(css="rgb(12, 34, 56)", image=2), observed
        assert all(path in target.requests for path in ["/page", "/app.js", "/style.css", "/pixel.svg", "/api"])
        self.record("controller", "actual_SDK_provider_WebView2_resources", dict(actual_page=True, script_fetch_exact=True,
            stylesheet_applied=True, image_decoded=True, independent_target_observed=True,
            provider_bound=True, callback_is_real_SDK=True))
        if self.args.case == "content":
            from web_windows_content_checks import WindowsContentChecks
            WindowsContentChecks(self).exercise(host, target, gateway, provider, consumer)
        self.record("controller", "actual_integrated_private_close", host.close())

    def run(self):
        result = dict(result="FAILED", scope="actual Windows SDK11/provider/gateway/WebView2 integration",
                      case=self.args.case, SDK_SHA256=self.sdk_sha, SDK_graph=self.args.sdk_graph,
                      whole_host_zero_egress=False,
                      engine_background_destinations="UNKNOWN")
        primary = None
        cleanup, errors = [], []
        try:
            self.scenario()
            result["result"] = "PASS"
        except BaseException as failure:
            primary = failure
            result["primary_failure"] = dict(type=type(failure).__name__, message=str(failure))
        finally:
            for group in [self.hosts, self.gateways, self.nodes, self.targets]:
                for owner in reversed(group):
                    if getattr(owner, "closed", False) is True:
                        continue
                    try:
                        cleanup.append(owner.close())
                    except BaseException as failure:
                        errors.append(dict(owner=getattr(owner, "label", type(owner).__name__),
                                           type=type(failure).__name__, message=str(failure)))
            unsettled = [node.label for node in self.nodes for child in node.child_registry
                         if child.process.poll() is None or any(reader.is_alive() for reader in child.readers)]
            for host in self.hosts:
                for child in getattr(host, "child_registry", []):
                    if child.process.poll() is None or any(reader.is_alive() for reader in child.readers):
                        unsettled.append(host.label)
            result.update(cleanup=cleanup, cleanup_errors=errors, unsettled=unsettled, events=len(self.records),
                          source_drift=[name for name, digest in self.manifest["files"].items() if sha(self.root / name) != digest])
            if errors or unsettled or result["source_drift"]:
                result["result"] = "FAILED"
            (self.output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            print(json.dumps(result), flush=True)
        if primary:
            raise primary
        if result["result"] != "PASS":
            raise RuntimeError("Windows integrated ownership/custody failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--case", choices=["entry", "content"], default="entry")
    parser.add_argument("--sdk-graph", choices=SDK_GRAPHS, default="sdk11")
    IntegrationRunner(parser.parse_args()).run()
