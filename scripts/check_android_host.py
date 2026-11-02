#!/usr/bin/env python3
"""Real installed Android Service, current Native SDK, and independent OS effects.

ADB supplies fresh fixture inputs and controls the owned debug APK. File/socket
business bytes travel over miopunch. No clipboard, device reboot, or global
network setting changes are part of this case.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import time

from check_forwarding import Target, isolated_environment
from check_terminal_service import WorkspaceObserver
from check_web_service import OwnedNode, WebRunner
from web_process_owner import register_process

PACKAGE = "org.miopunch.host.verify"


class AndroidActor:
    def __init__(self, runner):
        self.runner, self.process, self.owner, self.installed = runner, None, None, False
        self.reader = self
        runner.targets.append(self)
        self.adb = [str(runner.args.adb), "-s", runner.args.serial]
        self.counter, self.finished = 100, False
        self.ready = None
        self.logs = []
        existing = self.command("shell", "pm path " + PACKAGE, check=False)
        assert existing.returncode != 0 and not existing.stdout, "verification package already exists; do not replace unknown installation"
        apk = json.loads(runner.args.apk_receipt.read_text())
        assert apk["exit"] == 0 and hashlib.sha256(Path(apk["pin"]).read_bytes()).hexdigest() == apk["sha256"]
        root = Path(__file__).resolve().parents[1]
        for name, digest in apk["sources"].items():
            assert hashlib.sha256((root/name).read_bytes()).hexdigest() == digest, name
            runner.freeze_source(root/name)
        installed = self.command("install", "-g", apk["pin"], timeout=90, check=False)
        (runner.output / "install.stdout").write_bytes(installed.stdout)
        (runner.output / "install.stderr").write_bytes(installed.stderr)
        actual = self.command("shell", "pm path " + PACKAGE, check=False)
        if actual.returncode == 0 and actual.stdout.startswith(b"package:"):
            package_path = actual.stdout.decode().strip().removeprefix("package:")
            actual_hash = self.command("shell", "sha256sum " + shlex.quote(package_path)).stdout.decode().split()[0]
            assert actual_hash == apk["sha256"], "unexpected installed APK; preserve for ownership review"
            self.installed = True
        lines = installed.stdout.decode().splitlines()
        assert installed.returncode == 0 and self.installed and lines[-1:] == ["Success"] and all(
            line == "Performing Streamed Install" for line in lines[:-1]), "APK install not qualified; original output retained"
        runner.record("android", "apk_installed", {"sha256": apk["sha256"], "bytes": apk["bytes"], "package": PACKAGE, "previously_absent": True})
        for suffix in ["stdout", "stderr"]:
            self.logs.append((runner.output / ("instrumentation." + suffix)).open("wb"))
        self.process = subprocess.Popen(self.adb + ["shell", "am instrument -w -r " + PACKAGE + "/org.miopunch.host.VerifyInstrumentation"],
                                        stdin=subprocess.DEVNULL, stdout=self.logs[0], stderr=self.logs[1], env=isolated_environment())
        self.owner = register_process(self.process, runner.output / "instrumentation-owner.jsonl")
        runner.until(lambda: self.read_json("verify-ready.json") is not None or self.process.poll() is not None,
                     "real APK instrumentation startup", seconds=30)
        self.bind_ready()
        assert self.process.poll() is None

    def bind_ready(self):
        ready = self.read_json("verify-ready.json")
        assert ready and ready["ready"] and len(ready["lease"]) == 36
        actual = self.command("shell", "pidof " + PACKAGE).stdout.decode().strip()
        assert actual == str(ready["pid"]), "owned instrumentation identity changed"
        if self.ready is not None:
            assert self.ready == ready, "owned verification lease changed"
        self.ready = ready
        self.files, self.pid, self.lease = ready["files"], ready["pid"], ready["lease"]
        self.runner.record("android", "real_application_ready", ready)

    def command(self, *args, input=None, timeout=30, check=True):
        if args and args[0] == "shell": args = ("shell", "-T", *args[1:])
        result = subprocess.run(self.adb + list(args), input=input, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=isolated_environment(), timeout=timeout)
        if check:
            assert result.returncode == 0, (args[:2], result.returncode, result.stderr.decode(errors="replace"))
        return result

    def read_json(self, name):
        assert name in {"verify-ready.json", "verify-result.json", "host-state.json"}
        result = self.command("shell", "run-as " + PACKAGE + " cat " + shlex.quote("files/" + name), check=False)
        if result.returncode == 1 and not result.stdout and b"No such file or directory" in result.stderr:
            return None
        try:
            assert result.returncode == 0 and len(result.stdout) <= 262144
            return json.loads(result.stdout)
        except BaseException:
            self.runner.record("android", "control_read_failure", {"file":name,"exit":result.returncode,
                "stdout":result.stdout.decode(errors="replace"),"stderr":result.stderr.decode(errors="replace")})
            raise

    def write_private(self, name, data):
        assert "/" not in name and name not in {".", ".."}
        command = "cat > " + shlex.quote("files/" + name + ".pending") + " && mv " + shlex.quote("files/" + name + ".pending") + " " + shlex.quote("files/" + name)
        self.command("shell", "run-as " + PACKAGE + " sh -c " + shlex.quote(command), input=data)
        return self.files + "/" + name

    def call(self, kind, **arguments):
        self.counter += 1
        identifier = self.counter
        self.write_private("verify-request.json", json.dumps(dict(id=identifier, lease=self.lease, kind=kind, **arguments)).encode())
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            response = self.read_json("verify-result.json")
            if response and response.get("id") == identifier and response.get("lease") == self.lease:
                self.runner.record("android", kind, response)
                assert response["ok"], response
                if kind == "finish": self.finished = True
                return response["result"]
            if kind != "finish":
                assert self.process.poll() is None, "original instrumentation ended before result"
            time.sleep(.1)
        raise TimeoutError("Android owned mailbox response deadline")

    def sdk(self, operation, ok=True, **arguments):
        response = self.call("request", request=dict(id=self.counter + 1, op=operation, **arguments))
        assert response["ok"] == ok, response
        return response.get("result")

    def check_control_boundaries(self, resources):
        for identifier, lease, label in [(10, "00000000-0000-0000-0000-000000000000", "wrong-lease"), (0, self.lease, "stale-id")]:
            packet = dict(id=identifier, lease=lease, kind="start", mode={"action":"create"}, resources=resources)
            self.write_private("verify-request.json", json.dumps(packet).encode())
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                response = self.read_json("verify-result.json")
                if response and response.get("id") == identifier and response.get("lease") == self.lease:
                    break
                time.sleep(.05)
            else: raise TimeoutError("verification negative control deadline")
            assert not response["ok"] and response["error"] == "IllegalArgumentException"
            exists = self.command("shell", "run-as " + PACKAGE + " test -e files/node/node.key", check=False)
            assert exists.returncode == 1 and not exists.stdout and not exists.stderr, "invalid control produced a node"
            self.runner.record("android", "verification_control_boundary", {"case":label,"rejected":True,"node_key_absent":True})

    def is_alive(self):
        return self.process is not None and self.process.poll() is None

    def close(self):
        errors = []
        if self.process is not None and self.installed:
            try:
                if not self.finished:
                    self.bind_ready()
                    self.call("finish")
                self.process.wait(timeout=30)
            except BaseException as error:
                errors.append(type(error).__name__)
        if self.process is not None and self.process.poll() is None:
            self.cleanup = dict(owner="android-APK", errors=errors, original_instrumentation_live=True)
            raise RuntimeError("owned APK did not finish; preserve it for exact settlement")
        for stream in self.logs: stream.close()
        if self.owner is not None: self.owner.close()
        if self.installed and not errors:
            if self.process is None:
                observed = self.command("shell", "pidof " + PACKAGE, check=False)
                if observed.stdout.strip(): errors.append("UnexpectedApplicationBeforeInstrumentation")
            else:
                state = self.read_json("host-state.json")
                if not state or state["state"] != "stopped" or state["running"] or state["wake_lock_held"] or state["pid"] != self.pid:
                    errors.append("NativeStopNotObserved")
                else: self.runner.record("android", "native_service_final_state", state)
            if not errors:
                if not getattr(self.runner, "retain_package_for_reboot", False):
                    removed = self.command("uninstall", PACKAGE)
                    if removed.stdout.strip() != b"Success": errors.append("OwnedApkUninstallFailed")
                    else:
                        absent = self.command("shell", "pm path " + PACKAGE, check=False)
                        if absent.stdout.strip(): errors.append("OwnedApkStillInstalled")
                        else: self.installed = False
        self.cleanup = dict(owner="android-APK", exit=self.process.poll() if self.process else None,
                            errors=errors, owned_package_removed=not self.installed, forced_device_stop=False,
                            retained_for_explicit_reboot_handoff=bool(getattr(self.runner, "retain_package_for_reboot", False)))
        assert not errors and (self.process is None or self.process.poll() == 0), self.cleanup
        return self.cleanup


class AndroidRunner(WebRunner):
    def scenario(self):
        self.freeze_source(Path(__file__).with_name("check_terminal_service.py"))
        self.freeze_source(Path(__file__).with_name("check_android_host.py"))
        self.loopback = "127.0.0.1"
        resources = {"defaults":{"revision":2,"resources":[]},"replacement":[],"disabled":[],
                     "disabled_uses":["Observation","Lookup","Signaling"],"lan_discovery":"Disabled","port_mapping":"Disabled"}
        self.native_configuration = self.private / "resources.json"
        self.native_configuration.write_text(json.dumps(resources))
        provider = OwnedNode(self, "linux-provider")
        android = AndroidActor(self)
        android.check_control_boundaries(resources)
        invitation = self.private / "phone-invitation"
        provider.call("issue_precode", path=str(invitation), seconds=600, max_uses=1)
        private_invitation = android.write_private("invitation.json", invitation.read_bytes())
        ready = android.call("start", mode={"action":"join","invitation":invitation.read_text()}, resources=resources)
        assert ready["ok"]
        phone = ready["result"]
        identity, address = phone["status"]["node"], phone["address"]
        assert identity != provider.identity
        assert android.sdk("request", path=private_invitation)["stage"] == "Complete"
        assert android.sdk("ping", address=provider.address)["reply"] == "PONG"
        initial = android.call("snapshot")
        assert initial["activity_exists"] and initial["service"]["running"] and initial["service"]["wake_lock_held"]
        assert initial["service"]["restore_after_reboot"]
        assert android.call("reboot_restore", enabled=False)["saved"]
        assert not android.call("snapshot")["service"]["restore_after_reboot"]
        assert android.call("reboot_restore", enabled=True)["saved"]
        assert android.call("snapshot")["service"]["restore_after_reboot"]
        generation = initial["service"]["service_generation"]
        receive = self.private / "receive"
        provider.call("files_enable", directory=str(receive))
        android.sdk("service_sync", address=provider.address)
        data = bytes(index % 251 for index in range(8193))
        input_path = android.write_private("fixture-input.bin", data)
        task = android.sdk("file_send", address=provider.address, path=input_path)["task"]
        self.until(lambda: any(t["id"] == task and t["state"] == "complete" for t in android.sdk("files_status")["tasks"]), "Android send actual completion", seconds=30)
        saved = list(receive.iterdir())
        assert len(saved) == 1 and saved[0].read_bytes() == data
        self.record("controller", "android_to_linux_independent_file", {"bytes":len(data),"sha256":hashlib.sha256(data).hexdigest()})
        android.sdk("files_enable", directory=android.files + "/received")
        provider.call("service_sync", address=address)
        android.call("close_window")
        self.until(lambda: not android.call("snapshot")["activity_exists"], "Activity destruction", seconds=10)
        background = android.call("snapshot")
        assert background["service"]["service_generation"] == generation and background["service"]["running"]
        if self.args.lock_screen:
            android.command("shell", "input keyevent KEYCODE_SLEEP")
            power = android.command("shell", "dumpsys power").stdout.decode()
            policy = android.command("shell", "dumpsys window policy").stdout.decode()
            assert "mWakefulness=Asleep" in power or "mWakefulness=Dozing" in power
            assert "showing=true" in policy
            self.record("controller", "actual_locked_screen_precondition", {
                "screen_asleep_or_dozing": True, "OS_keyguard_showing": True,
                "same_running_Service": android.call("snapshot")["service"]["service_generation"] == generation,
            })
        source = self.private / "background-file.bin"
        source.write_bytes(bytes(index % 239 for index in range(65539)))
        task = provider.call("file_send", address=address, path=str(source))["task"]
        self.until(lambda: any(t["id"] == task and t["state"] == "complete" for t in provider.call("files_status")["tasks"]), "background Android saved completion", seconds=30)
        digest = android.command("shell", "run-as " + PACKAGE + " sh -c 'sha256sum files/received/*'").stdout.decode().split()
        assert len(digest) == 2 and digest[0] == hashlib.sha256(source.read_bytes()).hexdigest()
        self.record("controller", "linux_to_android_background_file", {"bytes":source.stat().st_size,"sha256":digest[0],"same_service_generation":True,"Activity_destroyed":True})
        for protocol in ["tcp", "udp"]:
            target = Target(self, protocol, prefix=b"")
            provider.call("forward_enable", protocol=protocol, targets=[target.address])
            android.sdk("service_sync", address=provider.address)
            forwarding = android.sdk("forward_start", address=provider.address, protocol=protocol, target=target.address, bind="127.0.0.1:0")
            host, port = forwarding["listen"].rsplit(":", 1)
            assert host == "127.0.0.1"
            payload = bytes(index % 251 for index in range(257))
            command = "toybox nc -n -w 15 -W 10 -q 3 " + ("-u " if protocol == "udp" else "") + "127.0.0.1 " + str(int(port))
            result = android.command("shell", command, input=payload, timeout=25)
            assert result.stdout == payload and b"".join(target.received) == payload
            android.sdk("forward_stop", task=forwarding["id"])
            self.record("controller", "android_forward_socket", {"protocol":protocol,"bytes":len(payload),"target_received_exact":True,"reply_exact":True})
        terminal = self.private / "terminal"
        observer = WorkspaceObserver(self, terminal)
        provider.call("terminal_enable", directory=str(terminal))
        android.sdk("service_sync", address=provider.address)
        workspace = android.sdk("terminal_create", address=provider.address)["workspace"]
        observer.observe_server()
        attachment = android.sdk("terminal_attach", address=provider.address, workspace=workspace)["id"]
        android.sdk("terminal_send", attachment=attachment, message={"type":"bytes","data":list(b"printf 'android-terminal-effect' > owned-effect; printf 'ANDROID_'; printf 'READY\\n'\n")})
        self.until(lambda: (terminal/workspace/"owned-effect").is_file(), "Android terminal command effect")
        assert (terminal/workspace/"owned-effect").read_bytes() == b"android-terminal-effect"
        assert not android.sdk("terminal_detach", attachment=attachment)["failed"]
        attachment = android.sdk("terminal_attach", address=provider.address, workspace=workspace)["id"]
        android.sdk("terminal_detach", attachment=attachment)
        android.sdk("terminal_end", address=provider.address, workspace=workspace)
        self.record("controller", "android_terminal_independent_effect", {"actual_pty":True,"independent_file_exact":True,"same_workspace_reattached":True,"explicitly_ended":True})
        if self.args.lock_screen:
            (self.output / "await-user-unlock.json").write_text(json.dumps({
                "phase":"locked_background_business_complete", "requires":"Physical phone unlock before showing Activity",
                "SDK_not_stopped":True}) + "\n")
            android.command("shell", "input keyevent KEYCODE_WAKEUP")
            def unlocked():
                policy = android.command("shell", "dumpsys window policy").stdout.decode()
                return "showing=false" in policy
            self.until(unlocked, "Physical phone unlock after actual locked-background operations", seconds=180)
            self.record("controller", "phone_unlocked_after_background_business", {"actual_OS_keyguard_dismissed":True})
        android.call("show_window")
        shown = android.call("snapshot")
        assert shown["activity_exists"] and shown["activities_created"] >= 2 and shown["service"]["service_generation"] == generation
        assert android.sdk("status")["node"] == identity
        android.call("quit")
        android.call("show_window")
        self.until(lambda: (android.call("snapshot").get("service") or {}).get("running"), "Service durable reopen", seconds=30)
        reopened = android.sdk("status")
        assert reopened["node"] == identity and reopened["stage"] == "Complete"
        assert android.call("snapshot")["service"]["service_generation"] != generation
        assert android.sdk("ping", address=provider.address)["reply"] == "PONG"
        self.record("controller", "android_service_lifecycle", {"window_reopen_kept_owner":True,"explicit_quit_stopped_owner":True,"new_service_same_persisted_node":True,"post_reopen_actual_ping":True,"whole_process_restart_tested":False})
        if self.args.retain_for_reboot:
            self.retain_private_state = True
            self.retain_package_for_reboot = True
            apk = json.loads(self.args.apk_receipt.read_text())
            handoff = {"scope":"Owned stopped SDKs and retained fresh state for actual reboot qualification",
                "private":str(self.private), "private_owner":str(self.output), "linux_node_directory":provider.directory,
                "linux_identity":provider.identity, "Android_identity":identity,
                "Android_service_generation_before_reboot":android.call("snapshot")["service"]["service_generation"],
                "APK_receipt":str(self.args.apk_receipt), "APK_sha256":apk["sha256"], "node_state_or_credentials_exported":False}
            (self.output / "reboot-handoff.json").write_text(json.dumps(handoff,indent=2) + "\n")
        android.call("finish")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "source-receipt", "apk-receipt", "output", "adb"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--lock-screen", action="store_true", help="Prove OS keyguard/background effects, then await physical unlock")
    parser.add_argument("--retain-for-reboot", action="store_true", help="After success, keep only the explicit owned APK/node handoff for a subsequent actual reboot check")
    args = parser.parse_args(); args.case = "android-host"
    AndroidRunner(args).run()
