#!/usr/bin/env python3
"""Run existing public device scenarios with private cross-platform resources.

The remote SDK is linked into a fresh run directory. Cleanup uses the launched
hosts' protocol/EOF and waits; it never adopts processes for forced signalling.
"""
import argparse
import json
from pathlib import Path
import shutil

from check_device_management import DeviceRunner
from check_node_backup_consistency import ConsistencyRunner
from check_node_replacement import ReplacementRunner
from check_native_client import quote


class PlatformResources:
    def deploy(self):
        config = {
            "defaults": {"revision": 2, "resources": []}, "replacement": [],
            "disabled": [], "disabled_uses": ["Observation", "Lookup", "Signaling"],
            "lan_discovery": "Disabled", "port_mapping": "Disabled",
        }
        self.local_configuration = self.private_root / "resources.json"
        self.local_configuration.write_text(json.dumps(config))
        self.local_configuration.chmod(0o600)
        if self.local_only:
            return
        root, binary = quote(self.remote_root), quote(self.remote_binary)
        existing = quote(self.existing_windows_sdk)
        expected = quote(self.expected_windows_sha)
        self.remote(
            "if(Test-Path -LiteralPath " + root + "){throw 'fresh run collision'}; "
            "if((Get-FileHash -LiteralPath " + existing + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne "
            + expected + "){throw 'SDK hash changed'}; "
            "New-Item -ItemType Directory -Path " + root + " | Out-Null"
        )
        self.remote_owned = True
        self.remote(
            "New-Item -ItemType HardLink -Path " + binary + " -Value " + existing + " | Out-Null; "
            "if((Get-FileHash -LiteralPath " + binary + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne "
            + expected + "){throw 'SDK link hash changed'}; "
            "[IO.File]::WriteAllText(" + quote(self.native_configuration(True)) + ","
            + quote(json.dumps(config)) + ", (New-Object Text.UTF8Encoding($false)))"
        )
        self.record("laptop", "deployment", {"existing_sdk_reused": True,
                    "sha256": self.expected_windows_sha, "fresh_root": self.remote_root})
        self.record("runner", "explicit_resources", config)

    def native_configuration(self, remote):
        return self.remote_root + "/resources.json" if remote else str(self.local_configuration)

    def cleanup(self):
        results = []
        for host in self.hosts:
            failures = []
            try:
                host.stop()
            except Exception as error:
                failures.append(type(error).__name__)
                try:
                    if host.process.stdin and not host.process.stdin.closed:
                        host.process.stdin.close()
                    host.process.wait(timeout=20)
                except Exception as cleanup_error:
                    failures.append(type(cleanup_error).__name__)
            host.reader.join(timeout=5)
            if host.reader.is_alive():
                failures.append("ReaderStillAlive")
            if host.process.poll() is None:
                failures.append("OriginalProcessStillAlive")
            else:
                host.stderr.close()
            row = {"host": host.label, "exit": host.process.poll(),
                   "reader_joined": not host.reader.is_alive(), "errors": failures}
            if failures:
                row["error"] = "host_cleanup_failed"
            results.append(row)
        terminal = all(host.process.poll() is not None and not host.reader.is_alive()
                       for host in self.hosts)
        remote_clear = self.local_only or not self.remote_owned
        if self.remote_owned:
            try:
                observed = json.loads(self.remote(
                    "$exe=[IO.Path]::GetFullPath(" + quote(self.remote_binary) + "); "
                    "@{remaining=@(Get-CimInstance Win32_Process | Where-Object "
                    "{$_.ExecutablePath -eq $exe}).Count} | ConvertTo-Json -Compress"))
                remote_clear = observed["remaining"] == 0
                if not remote_clear:
                    raise RuntimeError("remote host still running")
                if terminal:
                    self.remote("Remove-Item -LiteralPath " + quote(self.remote_root) + " -Recurse")
                    results.append({"host": "laptop", "private_removed": True})
            except Exception as error:
                results.append({"host": "laptop", "error": type(error).__name__})
        if terminal and remote_clear:
            shutil.rmtree(self.private_root)
            results.append({"host": "host", "private_removed": True})
        return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=["management", "replacement", "ledgers"], required=True)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--windows", type=Path, required=True)
    parser.add_argument("--windows-sha", required=True)
    parser.add_argument("--existing-windows-sdk", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.local_only, args.failure_only, args.focus = False, False, "smoke"
    scenario = {"management": DeviceRunner, "replacement": ReplacementRunner,
                "ledgers": ConsistencyRunner}[args.scenario]
    runner_type = type("DevicePlatformRunner", (PlatformResources, scenario), {})
    runner = runner_type(args)
    runner.existing_windows_sdk = args.existing_windows_sdk
    runner.expected_windows_sha = args.windows_sha
    runner.run()


if __name__ == "__main__":
    main()
