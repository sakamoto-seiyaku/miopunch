#!/usr/bin/python3
"""Minimal actual Linux WebKit content host, controlled only by private pipes.

No node object, native management handler or credentials enter this WebContext.
This technical host is not the final Tauri product shell.
"""
import json
from pathlib import Path
import sys
import threading
from urllib.parse import urlsplit

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.0")
from gi.repository import Gio, GLib, Gtk, WebKit2


def emit(value):
    print(json.dumps(value), flush=True)


def main():
    url = sys.argv[1]
    allowed = urlsplit(url)
    assert allowed.scheme == "https" and allowed.hostname == "127.0.0.1" and allowed.port
    context = WebKit2.WebContext.new_ephemeral()
    context.set_sandbox_enabled(True)
    proxy = urlsplit(sys.argv[3])
    assert proxy.scheme == "http" and proxy.hostname == "127.0.0.1" and proxy.port
    certificate = Gio.TlsCertificate.new_from_file(sys.argv[2])
    context.allow_tls_certificate_for_host(certificate, "127.0.0.1")
    # Even an omitted/unsupported CSP directive cannot permit direct egress.
    # The owned gateway rejects CONNECT and requests for another Host.
    # Only the exact owned content endpoint is direct. Every other HTTP(S)
    # origin goes to the gateway's rejecting proxy path. This also avoids a
    # CONNECT loop when the content endpoint itself uses TLS.
    context.set_network_proxy_settings(WebKit2.NetworkProxyMode.CUSTOM,
        WebKit2.NetworkProxySettings.new(sys.argv[3], [allowed.netloc]))
    view = WebKit2.WebView.new_with_context(context)
    settings = view.get_settings()
    settings.set_allow_file_access_from_file_urls(False)
    settings.set_allow_universal_access_from_file_urls(False)
    settings.set_javascript_can_access_clipboard(False)
    settings.set_javascript_can_open_windows_automatically(False)
    settings.set_enable_developer_extras(False)
    settings.set_enable_media_stream(False)
    settings.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
    view.connect("permission-request", lambda _view, request: (request.deny(), True)[1])
    view.connect("create", lambda *_: None)

    def policy(_view, decision, kind):
        if kind in {WebKit2.PolicyDecisionType.NAVIGATION_ACTION, WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION}:
            destination = urlsplit(decision.get_request().get_uri())
            if (destination.scheme, destination.netloc) != (allowed.scheme, allowed.netloc):
                emit({"event": "navigation_denied", "scheme": destination.scheme})
                decision.ignore()
                return True
        return False

    view.connect("decide-policy", policy)
    view.connect("load-changed", lambda _view, event: emit({"event": "load_state", "state": event.value_nick}))
    def failed(_view, _event, _uri, error):
        emit({"event": "load_failed", "domain": error.domain, "code": error.code})
        return False
    view.connect("load-failed", failed)
    view.connect("web-process-terminated", lambda _view, reason: emit({"event": "web_process_terminated", "reason": reason.value_nick}))
    window = Gtk.Window()
    window.set_default_size(800, 600)
    window.add(view)
    window.connect("destroy", lambda *_: Gtk.main_quit())
    view.load_uri(url)
    window.show_all()
    emit({"event": "ready", "ephemeral_context": context.is_ephemeral(), "sandbox_enabled": context.get_sandbox_enabled(),
          "native_management_handlers": False})

    def command(value):
        identifier = value["id"]
        if value["op"] == "close":
            window.destroy()
            emit({"id": identifier, "ok": True, "result": {"closed": True}})
        elif value["op"] == "evaluate":
            def evaluated(_view, result, _data):
                try:
                    outcome = view.run_javascript_finish(result).get_js_value()
                    emit({"id": identifier, "ok": True, "result": json.loads(outcome.to_json(0))})
                except BaseException:
                    emit({"id": identifier, "ok": False, "error": "evaluation_failed"})
            view.run_javascript("(" + value["expression"] + ")", None, evaluated, None)
        else:
            emit({"id": identifier, "ok": False, "error": "unsupported_command"})
        return False

    def read():
        for line in sys.stdin:
            if len(line) > 64 * 1024:
                break
            GLib.idle_add(command, json.loads(line))
        GLib.idle_add(window.destroy)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    Gtk.main()
    reader.join(timeout=5)
    if reader.is_alive():
        raise RuntimeError("host control reader did not stop")


if __name__ == "__main__":
    main()
