"""Stop an isolated Mosquitto broker while a control endpoint is waiting.

Usage: python3 scripts/mqtt_disconnect_probe.py
Build miopunch-native-probe first. Requires Docker and eclipse-mosquitto:2.
"""

import pathlib
import subprocess
import tempfile
import time
import uuid


ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target/debug/miopunch-native-probe"
CONFIG = ROOT / "scripts/mosquitto-test.conf"


def docker(*args):
    return subprocess.run(
        ["rtk", "proxy", "docker", *args],
        check=True, capture_output=True, text=True,
    ).stdout.strip()


def run():
    name = f"miopunch-mio20-disconnect-{uuid.uuid4().hex[:8]}"
    try:
        docker("run", "--rm", "-d", "--name", name,
               "-p", "127.0.0.1::1883",
               "-v", f"{CONFIG}:/mosquitto/config/mosquitto.conf:ro",
               "eclipse-mosquitto:2")
        broker = docker("port", name, "1883/tcp").splitlines()[0]
        with tempfile.TemporaryDirectory(prefix="mio20-disconnect-") as temp:
            root = pathlib.Path(temp)
            client_id = subprocess.run(
                [BIN, "identity", root / "client"],
                check=True, capture_output=True, text=True,
            ).stdout.strip()
            server = subprocess.Popen(
                [BIN, "serve", "mqtt", root / "server", broker, client_id],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            try:
                ready = server.stdout.readline().strip()
                assert ready.startswith("READY "), ready
                started = time.monotonic()
                docker("stop", name)
                _, stderr = server.communicate(timeout=8)
                elapsed = time.monotonic() - started
                assert server.returncode != 0, (server.returncode, stderr)
                assert "MQTT_CLOSE" in stderr and "MQTT carrier closed" in stderr, stderr
                print(f"MQTT_DISCONNECT exit={server.returncode} elapsed_ms={elapsed * 1000:.0f} error={stderr.strip()}")
            finally:
                if server.poll() is None:
                    server.kill()
                    server.wait()
    finally:
        subprocess.run(["rtk", "proxy", "docker", "rm", "-f", name], capture_output=True)


if __name__ == "__main__":
    run()
