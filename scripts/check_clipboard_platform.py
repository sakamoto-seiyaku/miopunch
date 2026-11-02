#!/usr/bin/env python3
"""Real interactive Windows and private Linux X11 clipboard acceptance.

SSH carries control and private admission files, never clipboard business data.
Existing native pins are reused. The Windows task is on-demand, least-privilege,
and uses the current user's interactive token; the host checks the input desktop.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import time
import uuid

from check_clipboard import Desktop, Host, Runner
from check_native_client import ps, quote


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class WindowsHost:
    def __init__(self, runner, invitation):
        self.runner, self.label = runner, "windows"
        self.remote_root = "C:/Users/stati/AppData/Local/Temp/m117-" + uuid.uuid4().hex
        self.runtime = self.remote_root + "/runtime"
        self.mail_sequence = 0
        self.ready = False
        self.process = None
        self.remote_owned = False
        self.closed = False
        self.task_terminal = False
        runner.hosts.append(self)
        source = Path(__file__).resolve().parent
        for name in ["windows_clipboard_host.ps1", "windows_clipboard_atomic.cs"]:
            runner.freeze_source(source / name)
        self.remote("$ErrorActionPreference='Stop'; if(Test-Path -LiteralPath " + quote(self.remote_root)
                    + "){throw 'fresh root collision'}; if((Get-FileHash -LiteralPath "
                    + quote(runner.windows_sdk) + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne "
                    + quote(runner.windows_sha) + "){throw 'SDK pin mismatch'}; "
                    + "New-Item -ItemType Directory -Path " + quote(self.remote_root) + " | Out-Null")
        self.remote_owned = True
        for path, name in [(source / "windows_clipboard_host.ps1", "windows_clipboard_host.ps1"),
                           (source / "windows_clipboard_atomic.cs", "windows_clipboard_atomic.cs"),
                           (runner.configuration, "resources.json"), (invitation, "invitation.json")]:
            result = subprocess.run(["scp", "-q", str(path), "laptop:" + self.remote_root + "/" + name],
                                    capture_output=True, text=True, timeout=30)
            assert result.returncode == 0, "owned deployment transfer failed"
            self.remote("if((Get-FileHash -LiteralPath " + quote(self.remote_root + "/" + name)
                        + " -Algorithm SHA256).Hash.ToLowerInvariant() -ne " + quote(sha(path))
                        + "){throw 'deployed source mismatch'}")
        # Compile and parse with actual Windows PowerShell before any clipboard use.
        preflight = self.remote("$ErrorActionPreference='Stop'; $t=$null; $e=$null; "
            "[void][Management.Automation.Language.Parser]::ParseFile(" + quote(self.remote_root + "/windows_clipboard_host.ps1")
            + ",[ref]$t,[ref]$e); if($e.Count){throw 'host syntax failure'}; Add-Type -Path "
            + quote(self.remote_root + "/windows_clipboard_atomic.cs") + "; 'WINDOWS_SOURCE_OK'")
        assert preflight.strip() == "WINDOWS_SOURCE_OK"
        runner.record(self.label, "actual_windows_source_preflight", {"passed": True})
        args = ['-NoLogo', '-NoProfile', '-STA', '-File', self.remote_root + '/windows_clipboard_host.ps1',
                '-Binary', runner.windows_sdk, '-BinarySha256', runner.windows_sha,
                '-ControllerSha256', sha(source / 'windows_clipboard_host.ps1'),
                '-HelperSha256', sha(source / 'windows_clipboard_atomic.cs'),
                '-RunDirectory', self.runtime, '-Action', 'join',
                '-Invitation', self.remote_root + '/invitation.json',
                '-Resources', self.remote_root + '/resources.json']
        arguments = ' '.join('"' + item + '"' for item in args)
        self.task_name = 'miopunch-m117-' + uuid.uuid4().hex
        task = """$ErrorActionPreference='Stop'
$scheduler=New-Object -ComObject Schedule.Service
$scheduler.Connect()
$folder=$scheduler.GetFolder('\\')
$definition=$scheduler.NewTask(0)
$definition.Principal.UserId=[Security.Principal.WindowsIdentity]::GetCurrent().Name
$definition.Principal.LogonType=3
$definition.Principal.RunLevel=0
$definition.Settings.Enabled=$true
$definition.Settings.AllowDemandStart=$true
$definition.Settings.DisallowStartIfOnBatteries=$false
$definition.Settings.StopIfGoingOnBatteries=$false
$definition.Settings.ExecutionTimeLimit='PT0S'
$action=$definition.Actions.Create(0)
$action.Path=$env:SystemRoot+'\\System32\\WindowsPowerShell\\v1.0\\powershell.exe'
$action.Arguments=ARGUMENTS
$task=$folder.RegisterTaskDefinition(NAME,$definition,2,$null,$null,3,$null)
$instance=$task.Run($null)
$guid=$instance.InstanceGuid
@{event='started';instance=$guid;interactiveToken=$true;runLevel=0}|ConvertTo-Json -Compress
$clock=[Diagnostics.Stopwatch]::StartNew()
while($clock.Elapsed.TotalSeconds -lt 1320) {
  try { $instance.Refresh() } catch {
    $cause=$_.Exception
    while($null -ne $cause.InnerException){$cause=$cause.InnerException}
    if ($cause.HResult -ne -2147216629) { throw }
    break
  }
  if($instance.State -notin @(2,4)){break}
  Start-Sleep -Milliseconds 200
}
# IRunningTask.Refresh throws SCHED_E_TASK_NOT_RUNNING after natural exit.
# A fresh registration with one explicit Run has no unrelated instances.
if($task.GetInstances(0).Count -ne 0){throw 'original_task_not_terminal_retained'}
$result=$task.LastTaskResult
$folder.DeleteTask(NAME,0)
@{event='terminal';instance=$guid;exit=$result;registrationRemoved=$true}|ConvertTo-Json -Compress
if($result -ne 0){exit 1}
""".replace('ARGUMENTS', quote(arguments)).replace('NAME', quote(self.task_name))
        (runner.output / "task-controller.ps1").write_text(task)
        self.out = (runner.output / "task.stdout").open("w")
        self.err = (runner.output / "task.stderr").open("w")
        self.process = subprocess.Popen(ps(task), stdout=self.out, stderr=self.err)
        ready = self.wait_json(self.runtime + "/ready.json", 100)
        assert ready["interactive"] and ready["clipboardMarkerOnly"] and ready["ownsFreshState"]
        self.run_id = ready["runId"]
        self.node, self.address = ready["ready"]["status"]["node"], ready["ready"]["address"]
        self.ready = True
        self.remote("Copy-Item -LiteralPath " + quote(self.remote_root + "/invitation.json")
                    + " -Destination " + quote(self.runtime + "/invitation.json"))
        runner.record(self.label, "interactive_ready", ready)

    def remote(self, script):
        result = subprocess.run(ps("$ErrorActionPreference='Stop'; " + script), capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, "remote control failed: " + result.stderr[:400]
        return result.stdout

    def get_json(self, path):
        value = self.remote("if(Test-Path -LiteralPath " + quote(path)
                            + "){[IO.File]::ReadAllText(" + quote(path) + ")}")
        return json.loads(value) if value.strip() else None

    def wait_json(self, path, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = self.get_json(path)
            if value is not None:
                return value
            if self.process is not None and self.process.poll() is not None:
                final = self.get_json(self.runtime + ".result.json")
                self.runner.record(self.label, "early_final", final)
                raise AssertionError("interactive task ended before response")
            time.sleep(.2)
        raise TimeoutError("owned Windows response timeout")

    def send(self, operation, **fields):
        self.mail_sequence += 1
        request = dict(id=self.mail_sequence, op=operation, **fields)
        encoded = base64.b64encode(json.dumps(request).encode()).decode()
        path = self.runtime + '/requests/%06d.json' % self.mail_sequence
        self.remote("[IO.File]::WriteAllBytes(" + quote(path + '.pending')
                    + ",[Convert]::FromBase64String(" + quote(encoded) + ")); "
                    + "Move-Item -LiteralPath " + quote(path + '.pending') + " -Destination " + quote(path))
        if operation == "finish":
            return
        value = self.wait_json(self.runtime + '/responses/%06d.json' % self.mail_sequence, 100)
        assert value["id"] == self.mail_sequence and value["ok"]
        return value["result"]

    def call(self, operation, ok=True, **fields):
        self.runner.sequence += 1
        value = self.send("sdk", expectedSdkOk=ok, command=dict(id=self.runner.sequence, op=operation, **fields))
        self.runner.record(self.label, operation, value)
        assert value["ok"] is ok
        return value.get("result")

    def marker(self, label, size=128):
        value = 'M117 owned Windows clipboard ' + self.run_id + ' ' + label + ' Unicode中文🙂'
        return value + 'x' * (size - len(value.encode()))

    def close(self):
        if self.closed:
            return self.cleanup_result
        failures = []
        final = None
        if self.process is not None:
            try:
                if self.process.poll() is None and self.ready:
                    self.send("finish")
            except Exception as failure:
                failures.append(type(failure).__name__)
            try:
                self.process.wait(timeout=1350)
                self.out.close()
                self.err.close()
                final = self.get_json(self.runtime + ".result.json")
                (self.runner.output / "windows-final.json").write_text(json.dumps(final, indent=2))
                rows = [json.loads(line) for line in (self.runner.output / "task.stdout").read_text().splitlines() if line]
                assert len(rows) == 2 and rows[0]["instance"] == rows[1]["instance"]
                assert rows[1]["event"] == "terminal" and rows[1]["registrationRemoved"]
                self.task_terminal = True
                assert final and final["allOwnedProcessesTerminal"] and final["privateStateRemoved"]
                if self.ready:
                    assert final["runId"] == self.run_id
                assert final["status"] == "pass" and self.process.returncode == 0
            except Exception as failure:
                failures.append(type(failure).__name__)
        if self.remote_owned and self.terminal() and (self.process is None or
                (final and final.get("allOwnedProcessesTerminal") and final.get("privateStateRemoved"))):
            try:
                self.remote("Remove-Item -LiteralPath " + quote(self.remote_root) + " -Recurse")
            except Exception as failure:
                failures.append(type(failure).__name__)
        self.closed = True
        self.cleanup_result = {"host": self.label, "terminal": self.terminal(), "errors": failures, "final": final}
        if failures:
            raise RuntimeError("interactive Windows cleanup or task failed")
        return self.cleanup_result

    def terminal(self):
        return self.process is None or (self.process.poll() is not None and self.task_terminal)


class PlatformRunner(Runner):
    def __init__(self, args):
        super().__init__(args)
        self.windows_sdk, self.windows_sha = args.windows_sdk, args.windows_sha
        self.configuration = self.private / "resources.json"
        self.configuration.write_text(json.dumps({"defaults": {"revision": 2, "resources": []},
            "replacement": [], "disabled": [], "disabled_uses": ["Observation", "Lookup", "Signaling"],
            "lan_discovery": "Disabled", "port_mapping": "Disabled"}))
        self.freeze_source(Path(__file__).resolve().parent / "check_native_client.py")

    def scenario(self):
        if self.case == "remaining":
            return self.remaining()
        desktop = Desktop(self, "linux")
        desktop.copy("M117 owned initial Linux marker")
        source = Host(self, "linux", desktop)
        invitation = self.private / "windows-invitation.json"
        source.call("issue_precode", path=str(invitation), seconds=3600, max_uses=1)
        windows = WindowsHost(self, invitation)
        assert windows.call("request", path=windows.runtime + "/invitation.json")["stage"] == "Complete"
        for host in [source, windows]:
            state = host.call("clipboard_enable")
            assert state["enabled"] and state["capability"]["read"] and state["capability"]["write"]
        source.call("clipboard_start", address=windows.address)
        assert windows.call("clipboard_status")["peer"] == source.node
        windows.send("expect", label="initial", bytes=128)
        assert desktop.read() == "M117 owned initial Linux marker"
        desktop.copy(windows.marker("forward"))
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "Windows write acknowledgment")
        windows.send("expect", label="forward", bytes=128)
        assert windows.call("clipboard_status")["written"] == 1
        windows.send("copy", label="reverse", bytes=128)
        self.until(lambda: desktop.read() == windows.marker("reverse"), "independent Linux read")
        self.until(lambda: windows.call("clipboard_status")["acknowledged"] == 1, "Linux write acknowledgment")
        time.sleep(1)
        assert source.call("clipboard_status")["sent"] == 1
        assert windows.call("clipboard_status")["sent"] == 1
        source.call("clipboard_stop")
        self.until(lambda: windows.call("clipboard_status")["peer"] is None, "Windows observes stop")
        desktop.copy(windows.marker("after-stop"))
        time.sleep(1)
        windows.send("expect", label="reverse", bytes=128)
        self.record("pair", "actual_duplex_stop", {"unicode": True, "independent_reads": True,
            "acknowledged_each": 1, "no_echo": True, "stop_prevents_write": True})

    def remaining(self):
        desktop = Desktop(self, "linux")
        desktop.copy("M117 current independently owned Linux baseline")
        source = Host(self, "linux", desktop)
        invitation = self.private / "windows-invitation.json"
        source.call("issue_precode", path=str(invitation), seconds=1200, max_uses=1)
        windows = WindowsHost(self, invitation)
        assert windows.call("request", path=windows.runtime + "/invitation.json")["stage"] == "Complete"
        assert windows.call("ping", address=source.address)["reply"] == "PONG"
        for host in [source, windows]:
            state = host.call("clipboard_enable")
            assert state["enabled"] and state["capability"]["read"] and state["capability"]["write"]
        # Exercise the previously unqualified Windows initiator role against
        # independent Win32/X11 clipboards, preserving each initial baseline.
        windows.call("clipboard_start", address=source.address)
        assert desktop.read() == "M117 current independently owned Linux baseline"
        windows.send("expect", label="initial", bytes=128)
        desktop.copy(windows.marker("before-third"))
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 1, "current Windows actual write acknowledgment")
        windows.send("expect", label="before-third", bytes=128)
        windows.send("copy", label="reverse-initiator", bytes=128)
        self.until(lambda: desktop.read() == windows.marker("reverse-initiator"), "Windows initiator actual reverse X11 read")
        self.until(lambda: windows.call("clipboard_status")["acknowledged"] == 1, "reverse actual write acknowledgment")
        time.sleep(1)
        assert source.call("clipboard_status")["sent"] == windows.call("clipboard_status")["sent"] == 1
        third_desktop = Desktop(self, "third")
        third_value = "M117 actual unrelated third system clipboard"
        third_desktop.copy(third_value)
        third = self.member(source, "third", third_desktop)
        state = third.call("clipboard_enable")
        assert state["capability"]["read"] and state["capability"]["write"]
        assert third.call("ping", address=windows.address)["reply"] == "PONG"
        third.call("clipboard_start", address=windows.address, ok=False)
        assert windows.call("clipboard_status")["peer"] == source.node
        assert third.call("clipboard_status")["peer"] is None and third_desktop.read() == third_value
        desktop.copy(windows.marker("selected-after-third"))
        self.until(lambda: source.call("clipboard_status")["acknowledged"] == 2, "selected pair continues after admitted third request")
        windows.send("expect", label="selected-after-third", bytes=128)
        assert third_desktop.read() == third_value
        self.record("pair", "actual_Windows_third_node_isolation", {
            "third_current_member_and_authenticated_PING":True, "third_real_X11_capability":True,
            "third_pair_rejected":True, "third_system_clipboard_unchanged":True,
            "selected_Windows_pair_continues":True, "Windows_initiator_duplex_and_no_echo":True})
        windows.call("clipboard_disable")
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "Windows disable ends real pair")
        desktop.copy(windows.marker("after-disable"))
        time.sleep(.8)
        windows.send("expect", label="selected-after-third", bytes=128)
        assert not windows.call("clipboard_status")["enabled"]
        windows.call("clipboard_enable")
        assert windows.call("clipboard_status")["peer"] is None
        windows.call("clipboard_start", address=source.address)
        desktop.copy(windows.marker("before-ACL"))
        self.until(lambda: windows.call("clipboard_status")["written"] >= 1, "real positive before ACL")
        self.until(lambda: source.call("clipboard_status")["acknowledged"] >= 1, "ACL positive acknowledgment")
        windows.send("expect", label="before-ACL", bytes=128)
        deny = dict(members=True, guests=True, allowed=[], denied=[source.node])
        revision = windows.call("service_rule", provider=windows.node, service="clipboard", previous=None, rule=deny)["revision"]
        self.until(lambda: windows.call("clipboard_status")["peer"] is None, "current Windows effective ACL ends pair")
        self.until(lambda: source.call("clipboard_status")["peer"] is None, "received Windows ACL closes connection")
        desktop.copy(windows.marker("after-ACL"))
        time.sleep(.8)
        windows.send("expect", label="before-ACL", bytes=128)
        source.call("clipboard_start", address=windows.address, ok=False)
        windows.send("expect", label="before-ACL", bytes=128)
        self.record("pair", "actual_Windows_disable_and_effective_ACL", {
            "disable_prevents_further_system_write":True, "reenable_does_not_start_pair":True,
            "current_provider_rule_revision":revision, "effective_denial_ends_access":True,
            "denied_new_pair_has_no_system_effect":True})
        for host in [third, source, windows]:
            host.call("clipboard_disable")
        # Restoration remains conditional on the final direct Win32 revision;
        # original contents/formats never pass through the controller mailbox.
        windows.send("copy", label="final-owned", bytes=128)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--windows-sdk", required=True)
    parser.add_argument("--windows-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--case", choices=["core", "remaining"], default="core")
    args = parser.parse_args()
    PlatformRunner(args).run()


if __name__ == "__main__":
    main()
