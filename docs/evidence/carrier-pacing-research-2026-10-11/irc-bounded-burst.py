#!/usr/bin/env python3
"""One bounded synthetic TLS-to-TLS IRC wire observation, not an SDK test.

At most 38 PRIVMSGs / 20 KiB sent, only between two generated owned nicknames.
No accounts, channels, real identities, admission materials or throughput loop.
Require both connections to advertise SAFERATE before sending data bursts.
"""
import asyncio
import hashlib
import json
import secrets
import ssl
import sys
import time
from pathlib import Path

HOST = "testnet.ergo.chat"
MAX_MESSAGES = 38
MAX_DATA_BYTES = 20 * 1024
report = {
    "service": HOST,
    "scope": "one synthetic bounded TLS pair; wire observations only, no SDK qualification",
    "normalCertificateValidation": True,
    "maximumPrivmsgs": MAX_MESSAGES,
    "maximumSentPrivmsgBytes": MAX_DATA_BYTES,
    "privmsgsSent": 0,
    "sentPrivmsgBytes": 0,
    "requestedFragmentSleepMs": 0,
    "connections": [],
    "cases": [],
    "passed": False,
}


class Peer:
    def __init__(self, nick):
        self.nick = nick
        self.ready = asyncio.Event()
        self.marker = asyncio.Event()
        self.saferate = False
        self.prefix_bytes = None
        self.groups = {}
        self.writer = None
        self.task = None
        self.receipt = {"normalTLS": False, "saferate": False, "closed": False}
        report["connections"].append(self.receipt)

    async def start(self):
        start = time.monotonic()
        self.reader, self.writer = await asyncio.open_connection(
            HOST, 6697, ssl=ssl.create_default_context(), server_hostname=HOST
        )
        self.receipt["normalTLS"] = True
        self.task = asyncio.create_task(self.read())
        self.send(f"NICK {self.nick}")
        self.send("USER mio 0 * :bounded synthetic wire observation")
        await asyncio.wait_for(self.ready.wait(), 12)
        self.receipt["registrationMs"] = round((time.monotonic() - start) * 1000, 3)
        self.receipt["saferate"] = self.saferate

    def send(self, line, data=False):
        encoded = (line + "\r\n").encode("ascii")
        if len(encoded) > 512:
            raise RuntimeError("outgoing full IRC line exceeds 512 bytes")
        if data:
            if report["privmsgsSent"] >= MAX_MESSAGES:
                raise RuntimeError("message budget exceeded")
            if report["sentPrivmsgBytes"] + len(encoded) > MAX_DATA_BYTES:
                raise RuntimeError("byte budget exceeded")
            report["privmsgsSent"] += 1
            report["sentPrivmsgBytes"] += len(encoded)
        self.writer.write(encoded)
        return len(encoded)

    async def read(self):
        while raw := await self.reader.readline():
            line = raw.decode("utf-8", "strict").rstrip("\r\n")
            if line.startswith("PING "):
                self.send("PONG " + line[5:])
                await self.writer.drain()
                continue
            parts = line.split(" ")
            if len(parts) > 1 and parts[1] == "005":
                self.saferate |= "SAFERATE" in parts
            if len(parts) > 1 and parts[1] in ("376", "422"):
                self.ready.set()
            if len(parts) > 1 and parts[1] == "ERROR":
                raise RuntimeError("server ERROR")
            if len(parts) < 4 or parts[1] != "PRIVMSG" or parts[2] != self.nick:
                continue
            self.prefix_bytes = len(parts[0].encode()) - 1
            if " :" not in line:
                continue
            body = line.split(" :", 1)[1]
            if body == "mio-size-marker":
                self.marker.set()
                continue
            fields = body.split(" ")
            if len(fields) != 7 or fields[0] != "MIR1":
                continue
            message_id = fields[3]
            if message_id not in self.groups:
                continue
            group = self.groups[message_id]
            index = int(fields[4], 16)
            count = int(fields[5], 16)
            if count != group["count"] or index in group["parts"]:
                raise RuntimeError("unexpected fragment count or duplicate")
            group["parts"][index] = bytes.fromhex(fields[6])
            group["arrivals"].append(time.monotonic())
            group["maxLine"] = max(group["maxLine"], len(raw))
            if len(group["parts"]) == count:
                group["done"].set()

    async def close(self):
        if self.writer is not None:
            try:
                self.send("QUIT :bounded observation complete")
                await asyncio.wait_for(self.writer.drain(), 2)
            except Exception:
                pass
            self.writer.close()
            try:
                await asyncio.wait_for(self.writer.wait_closed(), 3)
                self.receipt["closed"] = True
            except Exception as error:
                self.receipt["closeErrorType"] = type(error).__name__
        if self.task is not None:
            self.task.cancel()
            await asyncio.gather(self.task, return_exceptions=True)


async def series(sender, receiver, chunk):
    payload = bytes(i % 251 for i in range(1200))
    message_id = secrets.token_hex(8)
    count = (len(payload) + chunk - 1) // chunk
    group = {"count": count, "parts": {}, "arrivals": [], "maxLine": 0,
             "done": asyncio.Event()}
    receiver.groups[message_id] = group
    start = time.monotonic()
    max_sent = 0
    for index in range(count):
        body = (f"MIR1 {'0' * 32} {'1' * 32} {message_id} {index:02x} {count:02x} "
                + payload[index * chunk:(index + 1) * chunk].hex())
        projected = receiver.prefix_bytes + len(receiver.nick) + 14 + len(body)
        if projected > 512:
            raise RuntimeError("projected received line exceeds 512 bytes")
        max_sent = max(max_sent, sender.send(f"PRIVMSG {receiver.nick} :{body}", data=True))
    await sender.writer.drain()
    submitted = time.monotonic()
    await asyncio.wait_for(group["done"].wait(), 15)
    received = b"".join(group["parts"][i] for i in range(count))
    case = {"chunkBytes": chunk, "datagramBytes": len(payload), "fragments": count,
            "submitMs": round((submitted - start) * 1000, 3),
            "firstToLastArrivalMs": round((group["arrivals"][-1] - group["arrivals"][0]) * 1000, 3),
            "sendToCompleteMs": round((group["arrivals"][-1] - start) * 1000, 3),
            "maximumSentLineBytesIncludingCRLF": max_sent,
            "maximumReceivedLineBytesIncludingCRLF": group["maxLine"],
            "senderPrefixBytes": receiver.prefix_bytes,
            "syntheticWireByteEquality": received == payload,
            "payloadSha256": hashlib.sha256(payload).hexdigest(),
            "currentSdkChunkCompatible": chunk == 128}
    report["cases"].append(case)
    if received != payload:
        raise RuntimeError("synthetic payload mismatch")


async def main():
    suffix = secrets.token_hex(5)
    peers = [Peer("mioa" + suffix), Peer("miob" + suffix)]
    try:
        await asyncio.gather(*(peer.start() for peer in peers))
        if not all(peer.saferate for peer in peers):
            raise RuntimeError("SAFERATE not advertised; burst test skipped")
        for sender, receiver in (peers, peers[::-1]):
            sender.send(f"PRIVMSG {receiver.nick} :mio-size-marker", data=True)
            await sender.writer.drain()
            await asyncio.wait_for(receiver.marker.wait(), 8)
        for chunk in (128, 153):
            for sender, receiver in (peers, peers[::-1]):
                await series(sender, receiver, chunk)
        report["passed"] = True
    finally:
        await asyncio.gather(*(peer.close() for peer in peers))


if __name__ == "__main__":
    output = Path(sys.argv[1])
    started = time.monotonic()
    report["startedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    try:
        asyncio.run(asyncio.wait_for(main(), 45))
    except Exception as error:
        report["errorType"] = type(error).__name__
        report["error"] = str(error)
    report["wallMs"] = round((time.monotonic() - started) * 1000, 3)
    report["finishedUtc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    sys.exit(0 if report["passed"] and all(p["closed"] for p in report["connections"]) else 1)
