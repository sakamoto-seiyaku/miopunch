"""Drop or duplicate one MQTT PUBLISH on a real-broker iroh exchange.

Usage: python3 scripts/mqtt_loss_probe.py HOST:PORT [drop|duplicate]
Build miopunch-native-probe first. The proxy only intercepts the client's
MQTT TCP connection; the server connects directly to the given broker.
"""

import pathlib
import socket
import subprocess
import sys
import tempfile
import threading


ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target/debug/miopunch-native-probe"


def read_exact(sock, count):
    data = bytearray()
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise EOFError
        data.extend(chunk)
    return bytes(data)


def frame(sock):
    header = bytearray(read_exact(sock, 1))
    length = 0
    multiplier = 1
    for _ in range(4):
        byte = read_exact(sock, 1)[0]
        header.append(byte)
        length += (byte & 0x7F) * multiplier
        if byte < 128:
            return bytes(header) + read_exact(sock, length)
        multiplier *= 128
    raise ValueError("invalid MQTT length")


def pipe(src, dst):
    try:
        while data := src.recv(65536):
            dst.sendall(data)
    except (EOFError, OSError):
        pass


def run(broker, mode):
    host, port = broker.rsplit(":", 1)
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    proxy_addr = f"127.0.0.1:{listener.getsockname()[1]}"
    altered = []

    def proxy():
        client, _ = listener.accept()
        upstream = socket.create_connection((host, int(port)), timeout=5)
        forward = threading.Thread(target=pipe, args=(upstream, client))
        forward.start()
        try:
            while True:
                packet = frame(client)
                if packet[0] >> 4 == 3 and not altered:
                    altered.append(len(packet))
                    if mode == "drop":
                        continue
                    upstream.sendall(packet)
                upstream.sendall(packet)
        except (EOFError, OSError):
            pass
        finally:
            upstream.close()
            client.close()
            forward.join(timeout=1)

    thread = threading.Thread(target=proxy)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="mio20-loss-") as temp:
            root = pathlib.Path(temp)
            client_dir = root / "client"
            client_id = subprocess.run(
                [BIN, "identity", client_dir], check=True, capture_output=True, text=True
            ).stdout.strip()
            server = subprocess.Popen(
                [BIN, "serve", "mqtt", root / "server", broker, client_id],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            try:
                ready = server.stdout.readline().strip().split()
                assert len(ready) == 3 and ready[0] == "READY", ready
                result = subprocess.run(
                    [BIN, "send", "mqtt", client_dir, ready[1], ready[2], proxy_addr],
                    capture_output=True, text=True, timeout=15,
                )
                assert result.returncode == 0 and result.stdout == "PONG\n", result
                server.wait(timeout=4)
                assert server.returncode == 0, server.stderr.read()
                assert len(altered) == 1
                print(f"MQTT_FAULT mode={mode} first_publish_bytes={altered[0]} result=PONG")
            finally:
                if server.poll() is None:
                    server.kill()
                    server.wait()
    finally:
        listener.close()
        thread.join(timeout=2)


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] not in ("drop", "duplicate")):
        raise SystemExit(__doc__)
    run(sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else "drop")
