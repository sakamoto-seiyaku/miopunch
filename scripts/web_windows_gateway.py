#!/usr/bin/env python3
"""Provider-bound public SDK callbacks on original Windows child pipes."""
import json
from pathlib import Path

from web_content_gateway import ContentGateway
from web_windows_process import WindowsChild


class WindowsWebNode:
    def __init__(self, binary, private, output, label, environment, configuration,
                 owners, action="create", invitation=None):
        self.label, self.output, self.private = label, Path(output), Path(private)
        self.child, self.child_registry, self.ready_received = None, [], False
        owners.append(self)
        argv = [str(binary), action, str(self.private / label)]
        if invitation is not None:
            argv.append(str(invitation))
        argv.append(str(configuration))
        # Public SDK setup/response budget is the retained Native Node80s;
        # the content-host private pipe keeps its separate original10s budget.
        self.child = WindowsChild(argv, environment, output, label, response_timeout=80, owners=self.child_registry)
        ready = self.child.wait_ready(timeout=80)
        self.ready_received = True
        self.identity, self.address = ready["status"]["node"], ready["address"]
        if ready["status"]["stage"] == "AwaitingRecoverySave":
            # Fresh synthetic credential stays at this endpoint. Never export
            # old user/node/genesis data or transfer this local test card.
            self.call("export_recovery_card", path=str(self.private / (label + "-synthetic-card")))
            self.call("acknowledge_recovery_saved")

    def call(self, operation, ok=True, **arguments):
        return self.child.call(operation, expected_ok=ok, **arguments)

    def close(self):
        if self.child is None:
            if not self.child_registry:
                return dict(owner=self.label, process_created=False)
            return self.child_registry[0].cleanup()
        primary = None
        try:
            if self.ready_received and self.child.process.poll() is None:
                self.call("stop")
        except BaseException as failure:
            primary = failure
        finally:
            if primary is None:
                result = self.child.join(timeout=20)
            else:
                # Attempt cleanup while preserving the public stop failure.
                try:
                    self.child.cleanup()
                except BaseException:
                    pass
        if primary:
            raise primary
        return result


def provider_gateway(consumer, provider_address, target, private, owners):
    # Copy the controller-selected public address once; page inputs cannot
    # select a peer, operation, node store or management argument.
    address = json.loads(json.dumps(provider_address))
    sockets = {
        "open": lambda url, headers: consumer.call("web_socket_open", address=address, url=url, headers=headers),
        "send": lambda identifier, message: consumer.call("web_socket_send", socket=identifier, message=message),
        "read": lambda identifier: consumer.call("web_socket_read", socket=identifier, timeout_ms=100),
        "close": lambda identifier: consumer.call("web_socket_close", socket=identifier),
    }
    requests = {
        "begin": lambda arguments: consumer.call("web_begin", address=address, **arguments),
        "result": lambda identifier: consumer.call("web_result", job=identifier),
        "cancel": lambda identifier: consumer.call("web_cancel", job=identifier),
    }
    return ContentGateway(target, lambda arguments: consumer.call("web_open", address=address, **arguments),
                          private, owners, socket_callbacks=sockets, http_callbacks=requests)
