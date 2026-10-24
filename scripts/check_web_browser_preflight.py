#!/usr/bin/env python3
"""Own browser launch and normal private-CDP close before SDK attachment."""
import argparse
from pathlib import Path

from check_web_service import HttpTarget, OwnedNode, WebRunner
from forwarding_local_resources import configure_local_resources
from web_browser_runtime import WebBrowser
from web_content_gateway import ContentGateway
from web_network_namespace import validate_namespace
from web_process_owner import register_process


class PreflightRunner(WebRunner):
    def scenario(self):
        self.record("controller", "whole_scenario_network_namespace", validate_namespace())
        configure_local_resources(self, self.args, owner_factory=register_process)
        target = HttpTarget(self)
        provider = OwnedNode(self, "provider")
        gateway = ContentGateway(target.url, lambda _: None, self.private, self.nodes, browser_origins=True)
        gateway.record = lambda operation, result: self.record("controller", operation, result)
        browser = WebBrowser(self, self.args, gateway)
        result = browser.call("open", {"attach": False})
        assert result["processQualified"] and not result["cdpAttached"] and not result["browserSandboxDisabledByHarness"]
        self.record("controller", "owned_browser_before_attach", result)
        cleanup = browser.close()
        self.record("controller", "owned_browser_private_cdp_normal_close", cleanup)
        self.nodes.remove(browser)
        assert cleanup["actors_accounted"] and all(actor["exit"] == 0 and actor["closed"] for actor in cleanup["actors"])
        assert not target.requests and not target.active and provider.call("web_status")["active_flows"] == 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--relay-binary", type=Path, required=True)
    parser.add_argument("--relay-receipt", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--browser-receipt", type=Path, required=True)
    parser.add_argument("--chromium", type=Path, default=Path("/home/js/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome"))
    parser.add_argument("--case", default="preflight")
    PreflightRunner(parser.parse_args()).run()
