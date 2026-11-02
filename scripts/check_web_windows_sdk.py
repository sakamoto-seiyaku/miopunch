#!/usr/bin/env python3
"""Actual headless Windows public Web SDK/gateway/owned-target integration."""
import argparse
import asyncio
import base64
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import ssl
import sys
import time
from urllib.request import HTTPCookieProcessor, HTTPSHandler, ProxyHandler, build_opener

from aiohttp import ClientSession, WSMsgType
from web_asyncio_observer import OwnedLoopObserver
from web_protocol_target import WebTarget
from web_windows_gateway import WindowsWebNode, provider_gateway

SDK_SHA = "6de63b7756d7e9d627d6437bdc5c5c6fea22dc0b326ba44d4f2d58b1a7159690"


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def until(predicate, label, seconds=5):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.02)
    raise AssertionError(label)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", required=True)
    args = parser.parse_args()
    assert os.name == "nt" and sys.flags.isolated and sys.flags.no_site
    root, output = args.root.resolve(), args.output.resolve()
    assert output.is_relative_to(root)
    output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((root / args.manifest).read_text())
    assert all(sha(root / name) == digest for name, digest in manifest["files"].items())
    binary = root / "sdk/miopunch-client-sdk11.exe"
    assert sha(binary) == SDK_SHA
    private = output / "private"
    private.mkdir()
    for name in ["temp", "local-appdata", "appdata"]:
        (private / name).mkdir()
    windows = Path(os.environ["SystemRoot"])
    environment = dict(SystemRoot=str(windows), WINDIR=str(windows),
                       PATH=str(windows / "System32"), TEMP=str(private / "temp"), TMP=str(private / "temp"),
                       LOCALAPPDATA=str(private / "local-appdata"), APPDATA=str(private / "appdata"))
    configuration = private / "resources.json"
    configuration.write_text(json.dumps(dict(defaults=dict(revision=2, resources=[]), replacement=[], disabled=[],
        disabled_uses=["Observation", "Lookup", "Signaling"], lan_discovery="Disabled", port_mapping="Disabled")))
    nodes, targets, gateways, records, cleanup, errors = [], [], [], [], [], []

    def record(actor, operation, value):
        item = dict(actor=actor, operation=operation, value=value, monotonic=time.monotonic())
        records.append(item)
        with (output / "events.jsonl").open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(item) + "\n")

    class Runner:
        pass
    runner = Runner()
    runner.private, runner.targets, runner.record = private, targets, record
    result = dict(result="FAILED", scope="actual headless Windows SDK11/publicWeb/gateway, GUI/context not qualified",
                  SDK_SHA256=SDK_SHA, local_resources_only=True, whole_host_zero_egress=False)
    primary = None
    try:
        target = WebTarget(runner, tls=True)
        direct = build_opener(ProxyHandler({}), HTTPSHandler(context=ssl.create_default_context(cafile=target.certificate)))
        with direct.open(target.url, timeout=3) as response:
            assert response.read() == b"owned M119 web target"
        asyncio.run(target.socket_prerequisite())
        record("controller", "actual_target_preconditions", dict(HTTPS_exact=True, nativeTLS_validation=True,
                                                                 WebSocket_duplex_exact=True))
        provider = WindowsWebNode(binary, private, output, "provider", environment, configuration, nodes)
        invitation = private / "consumer-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        consumer = WindowsWebNode(binary, private, output, "consumer", environment, configuration,
                                  nodes, action="join", invitation=invitation)
        consumer.call("request", path=str(invitation))
        provider.call("service_sync", address=consumer.address)
        record("controller", "distinct_members", dict(provider=provider.identity, consumer=consumer.identity,
                                                     provider_address=provider.address, consumer_address=consumer.address))
        assert provider.identity != consumer.identity
        before = len(target.requests)
        consumer.call("web_open", address=provider.address, url=target.url, ok=False)
        time.sleep(2.05)
        assert len(target.requests) == before, "disabled service produced target request"
        record("controller", "disabled_no_effect", dict(target_requests_unchanged=True))
        provider.call("web_enable", targets=[target.url], trust_roots_pem=[target.trust_root_pem])
        job = consumer.call("web_begin", address=provider.address, url=target.url)
        observed = {}
        def http_done():
            observed.update(consumer.call("web_result", job=job["id"]))
            return not observed["pending"]
        until(http_done, "public Windows HTTP result pending")
        assert observed["response"]["status"] == 200 and base64.b64decode(observed["response"]["body"]) == b"owned M119 web target"
        record("controller", "public_async_HTTP_exact", dict(status=200, independent_target_observed=True))
        gateway = provider_gateway(consumer, provider.address, target.url + "page", private, gateways)
        cookies = http.cookiejar.CookieJar()
        client = build_opener(ProxyHandler({}), HTTPCookieProcessor(cookies),
                              HTTPSHandler(context=ssl.create_default_context(cafile=gateway.certificate)))
        with client.open(gateway.bootstrap, timeout=10) as response:
            assert b"M119 resource" in response.read()
        assert "/page" in target.requests

        async def socket_case():
            observer = OwnedLoopObserver(asyncio.get_running_loop(), "windows-gateway-direct-client", target.errors,
                                         lambda row: record("controller", "owned_asyncio", row))
            target.client_observers.append(observer)
            observer.phase("client_session")
            jar = {cookie.name: cookie.value for cookie in cookies}
            try:
                async with ClientSession(trust_env=False, cookies=jar) as session:
                    async with session.ws_connect(gateway.address.replace("https:", "wss:") + "/socket",
                        ssl=ssl.create_default_context(cafile=gateway.certificate), origin=gateway.address) as socket:
                        assert await socket.receive_str(timeout=5) == "owned M119 ready"
                        await socket.send_str("M119 real Windows gateway")
                        assert await socket.receive_str(timeout=5) == "owned-target:M119 real Windows gateway"
                        await socket.send_bytes(b"M119 Windows binary")
                        received = await socket.receive(timeout=5)
                        assert received.type == WSMsgType.BINARY and received.data == b"M119 Windows binary"
                        assert await socket.close(code=1000)
                        assert socket.close_code == 1000
            finally:
                observer.phase("client_coroutine_return")
        asyncio.run(socket_case())
        until(lambda: bool(gateway.socket_close_results), "gateway SDK socket owner did not close")
        assert gateway.socket_close_results[-1]["closed"] is True and gateway.socket_close_results[-1]["failed"] is False
        assert ("text", "M119 real Windows gateway") in target.observed and ("binary", b"M119 Windows binary") in target.observed
        record("controller", "real_gateway_HTTP_WebSocket_exact", dict(text=True, binary=True, code=1000,
                                                                       SDK_closed=True, SDK_failed=False))
        pending = consumer.call("web_begin", address=provider.address, url=target.url + "pending")
        until(target.pending_started.is_set, "public pending request did not reach target")
        assert consumer.call("web_result", job=pending["id"])["pending"]
        consumer.call("web_cancel", job=pending["id"])
        consumer.call("web_result", job=pending["id"], ok=False)
        until(lambda: provider.call("web_status")["active_flows"] == 0, "provider HTTP owner remained after cancel")
        assert consumer.call("web_status")["active_flows"] == 0
        record("controller", "actual_pending_cancel", dict(target_started=True, stale_job_rejected=True, active_flows=0))
        result["result"] = "PASS"
    except BaseException as failure:
        primary = failure
        result["primary_failure"] = dict(type=type(failure).__name__, message=str(failure))
    finally:
        for owner in reversed(gateways):
            try: cleanup.append(owner.close())
            except BaseException as failure: errors.append(dict(owner="gateway", type=type(failure).__name__))
        for owner in reversed(nodes):
            try: cleanup.append(owner.close())
            except BaseException as failure: errors.append(dict(owner=owner.label, type=type(failure).__name__))
        for owner in reversed(targets):
            try: cleanup.append(owner.close())
            except BaseException as failure: errors.append(dict(owner="target", type=type(failure).__name__))
        unsettled = [node.label for node in nodes for child in node.child_registry
                     if child.process.poll() is None or any(reader.is_alive() for reader in child.readers)]
        result.update(cleanup=cleanup, cleanup_errors=errors, unsettled=unsettled, events=len(records),
                      source_drift=[name for name, digest in manifest["files"].items() if sha(root / name) != digest])
        if errors or unsettled or result["source_drift"]: result["result"] = "FAILED"
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result), flush=True)
    if primary: raise primary
    if result["result"] != "PASS": raise RuntimeError("Windows SDK/gateway cleanup failed")


if __name__ == "__main__":
    main()
