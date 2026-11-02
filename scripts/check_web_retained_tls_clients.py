#!/usr/bin/env python3
"""Real owned TLS client held open during gateway/target close, without faults."""
import argparse
from concurrent.futures import Future
import hashlib
import json
from pathlib import Path
import socket
import ssl
import sys
import tempfile
import time


def snapshot(owner):
    result = owner.observer.snapshot()
    result["transports"] = []
    manager = getattr(owner.runner, "server", None) if hasattr(owner, "runner") else None
    result["http_manager_connections"] = len(manager.connections) if manager is not None else None
    for name, server in owner.observer.servers:
        clients = list(getattr(server, "_clients", ()))
        assert len(clients) <= 8, "local one-client fixture exceeded its ownership bound"
        for transport in clients:
            value = owner.observer.transport_state(transport)
            protocol = transport.get_protocol()
            state = getattr(protocol, "_state", None)
            value.update(server_name=name, server_id=id(server), protocol=type(protocol).__name__,
                         tls_state=getattr(state, "name", None),
                         app_protocol=type(getattr(protocol, "_app_protocol", None)).__name__)
            result["transports"].append(value)
    return result


def running_snapshot(owner):
    future = Future()
    def read_owned():
        try:
            future.set_result(snapshot(owner))
        except BaseException as failure:
            future.set_exception(failure)
    owner.loop.call_soon_threadsafe(read_owned)
    return future.result(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--controllers", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    sys.path.insert(0, str(args.controllers.resolve()))
    from urllib.parse import urlsplit
    from web_content_gateway import ContentGateway
    from web_protocol_target import WebTarget
    cases = []
    with tempfile.TemporaryDirectory(prefix="m119-owned-idle-TLS-") as temporary:
        for kind in ["gateway", "target"]:
            for client_closed_first in [False, True]:
                private = Path(temporary) / (kind + ("-control" if client_closed_first else "-open-client"))
                private.mkdir()
                events = []
                class Runner:
                    def __init__(self):
                        self.private, self.targets = private, []
                    def record(self, actor, operation, value):
                        events.append(dict(actor=actor, operation=operation, value=value))
                runner = Runner()
                owner = (ContentGateway("https://127.0.0.1:9/page", lambda request: None, private, [])
                         if kind == "gateway" else WebTarget(runner, tls=True))
                if kind == "gateway":
                    owner.record = lambda operation, value: runner.record("gateway", operation, value)
                address = urlsplit(owner.address if kind == "gateway" else owner.url)
                context = ssl.create_default_context(cafile=owner.certificate)
                client = context.wrap_socket(socket.create_connection((address.hostname, address.port), timeout=5),
                                             server_hostname=address.hostname)
                try:
                    peer = client.getpeercert(binary_form=True)
                    assert peer and context.verify_mode == ssl.CERT_REQUIRED and context.check_hostname
                    before = running_snapshot(owner)
                    assert sum(server["attached_clients"] for server in before["servers"]) == 1
                    if client_closed_first:
                        client.close()
                    started = time.monotonic()
                    outcome, failure = None, None
                    try:
                        outcome = owner.close()
                    except BaseException as error:
                        failure = error
                    elapsed = time.monotonic() - started
                    after = snapshot(owner)
                    thread = owner.thread if kind == "gateway" else owner.reader
                    checks = dict(actual_verified_one_TLS_client=True,
                        server_closed_while_client_open_or_explicit_control=True,
                        original_close_under5=elapsed < 5,
                        original_loop_thread_settled=owner.loop.is_closed() and not thread.is_alive(),
                        no_callback_errors=owner.errors == [],
                        close_reported_success=failure is None,
                        all_owned_server_clients_zero=all(server["attached_clients"] == 0 for server in after["servers"]),
                        all_owned_listeners_zero=all(server["listeners"] == 0 for server in after["servers"]))
                    cases.append(dict(kind=kind, client_closed_first=client_closed_first,
                        peer_certificate_sha256=hashlib.sha256(peer).hexdigest(), before=before, after=after,
                        close_seconds=elapsed, first_error=type(failure).__name__ if failure else None,
                        checks=checks, outcome=outcome, events=events))
                    assert checks["original_loop_thread_settled"], "local owned loop did not settle"
                finally:
                    client.close()
    result = dict(result="PASS" if all(all(case["checks"].values()) for case in cases) else "FAILED",
                  scope="local normal verified TLS client close; no live RST/pause/pressure/sleeps or Windows cause claim",
                  python=sys.version, cases=cases)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(dict(result=result["result"], cases=[dict(kind=case["kind"],
        client_closed_first=case["client_closed_first"], close_seconds=case["close_seconds"],
        attached=[server["attached_clients"] for server in case["after"]["servers"]],
        failed=[name for name, passed in case["checks"].items() if not passed]) for case in cases])), flush=True)
    if result["result"] != "PASS":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
