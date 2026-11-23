#!/usr/bin/env python3
"""Replay one full HSRNet recording through the existing private Linux lab.

The fixture supplies the network model and independent numbered UDP probes.
This controller records the original +6s window and the retained-channel result.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def metric(values):
    values = sorted(values)
    if not values:
        return {}
    return {"count": len(values), "median": statistics.median(values),
            "p95": values[(95 * len(values) + 99) // 100 - 1], "max": max(values)}


def calibration(output, source):
    rows = [json.loads(line) for line in (output / "clip-probe.jsonl").read_text().splitlines()]
    sent = {row["n"]: row for row in rows if row["kind"] == "send"}
    received = {row["n"]: row for row in rows if row["kind"] == "reply"}
    expected = {row["n"]: row for row in source["points"] if row["status"] == "reply"}
    missing = {row["n"] for row in source["points"] if row["status"] == "missing"}
    errors = [abs((received[n]["received"] - received[n]["sent"] - expected[n]["rtt_ns"]) / 1e6)
              for n in expected.keys() & received.keys()]
    result = dict(probes=len(sent), expected_returns=len(expected),
                  matched=len(expected.keys() & received.keys()), source_missing=len(missing),
                  censored=sum(row["status"] == "censored" for row in source["points"]),
                  extra_returns=sorted(missing & received.keys()),
                  extra_missing=sorted(expected.keys() - received.keys()), rtt_error_ms=metric(errors),
                  send_schedule_error_ms=metric([(row["sent"] - row["target"]) / 1e6 for row in sent.values()]))
    passed = (set(sent) == {row["n"] for row in source["points"]}
              and result["matched"] == len(expected)
              and not result["extra_returns"] and not result["extra_missing"]
              and (not expected or (result["rtt_error_ms"]["median"] <= 5
                                    and result["rtt_error_ms"]["p95"] <= 15)))
    return passed, result


def effects_at(path):
    return path.read_text().splitlines() if path.exists() else []


def terminal_window(lab, log, source, start, phone, attachment, markers, effects_path):
    sent, received, echo_order = {}, {}, []
    buffer = bytearray()
    cursor = 0
    original_deadline = start + source["clip_ns"] + 6_000_000_000
    final_deadline = start + source["clip_ns"] + 60_000_000_000
    snapshot = None
    failure = None
    while True:
        now = time.monotonic_ns()
        if snapshot is None and now >= original_deadline:
            observed = effects_at(effects_path)
            snapshot = dict(planned=len(markers), queued=len(sent), executed=len(observed),
                            echoes=len(received), observed_ns=now, deadline_ns=original_deadline)
            lab.record(log, "original_window", **snapshot)
        if snapshot is not None and (len(received) == len(markers) or failure is not None):
            break
        if now >= final_deadline:
            failure = failure or "terminal output incomplete at trace end + recovery window"
            break
        if failure is not None:
            time.sleep(min(0.02, max(0, (original_deadline - now) / 1e9)))
            continue
        try:
            if cursor < len(markers) and now >= markers[cursor][1]:
                marker, due = markers[cursor]
                actual = time.monotonic_ns()
                lab.send(phone, attachment,
                         f"printf '%s\\n' {marker} >> observed; printf '%s\\n' {marker}\n")
                sent[marker] = actual
                lab.record(log, "input", marker=marker, target_ns=due, sent_ns=actual)
                cursor += 1
            response = phone.call("terminal_read", attachment=attachment, timeout_ms=15)
            arrived = time.monotonic_ns()
            if not response["attached"]:
                raise RuntimeError(f"terminal attachment ended: {response!r}")
            message = response.get("message", {})
            if message.get("type") == "bytes":
                buffer.extend(message["data"])
                consumed = 0
                for match in re.finditer(rb"CLIP_[0-9]+\r?\n", buffer):
                    marker = match[0].decode().strip()
                    echo_order.append(marker)
                    if marker not in sent or marker in received:
                        raise RuntimeError(f"unexpected or duplicate echo {marker}")
                    received[marker] = arrived
                    lab.record(log, "data_echo", marker=marker,
                               latency_ms=round((arrived - sent[marker]) / 1e6, 3))
                    consumed = match.end()
                if consumed:
                    del buffer[:consumed]
        except Exception as error:
            failure = repr(error)
            lab.record(log, "terminal_failure", error=failure, queued=len(sent), echoes=len(received))
    return sent, received, echo_order, snapshot, failure


def exercise(options, lab, clip, source, output, log, phone_pid, phone, provider,
             original, attachment, workspace, directory):
    count = math.ceil(source["clip_ns"] / 250_000_000)
    before = phone.call("terminal_status")
    shell_before = (directory / workspace / "shell-before").read_text()
    effects_path = directory / workspace / "observed"
    start = time.monotonic_ns() + 700_000_000
    markers = [(f"CLIP_{n:05d}", start + n * 250_000_000) for n in range(count)]
    network_error = None
    with (output / "clip-probe.jsonl").open("w") as stream, ThreadPoolExecutor(max_workers=1) as pool:
        probe = subprocess.Popen(["nsenter", "-t", str(phone_pid), "-n", "--", sys.executable,
                                  str(options.fixture / "probe.py"), str(options.timeline), str(start)],
                                 stdout=stream)
        network = pool.submit(clip.network, start, output, log, phone_pid)
        try:
            sent, received, echo_order, snapshot, failure = terminal_window(
                lab, log, source, start, phone, attachment, markers, effects_path)
            try:
                network.result(timeout=max(12, source["clip_ns"] / 1e9 + 3))
                if probe.wait(timeout=12) != 0:
                    raise RuntimeError("independent probe process failed")
            except Exception as error:
                network_error = repr(error)
        finally:
            if probe.poll() is None:
                probe.terminate()
                probe.wait(timeout=3)
    terminal_seconds = (time.monotonic_ns() - start) / 1e9
    effects = effects_at(effects_path)
    expected = [name for name, _ in markers]
    (output / "executed.txt").write_text("".join(name + "\n" for name in effects))
    (output / "echoes.txt").write_text("".join(name + "\n" for name in echo_order))
    try:
        after = phone.call("terminal_status")
    except Exception as error:
        after = {"operations": [], "error": repr(error)}
        failure = failure or repr(error)
    post = {"passed": False, "attempted": True}
    began = time.monotonic_ns()
    try:
        lab.send(phone, attachment,
                 "printf '%s\\n' $$ > shell-after; printf '%s\\n' \"$MIO174_STATE\" > shell-state; "
                 "printf 'MIO175_'; printf 'POST\\n'\n")
        lab.read_marker(phone, attachment, "MIO175_POST")
        shell_after = (directory / workspace / "shell-after").read_text()
        shell_state = (directory / workspace / "shell-state").read_text()
        post = dict(attempted=True, passed=shell_before == shell_after and shell_state == "original\n",
                    elapsed_ms=(time.monotonic_ns() - began) / 1e6,
                    shell_before=shell_before.strip(), shell_after=shell_after.strip(), state=shell_state)
    except Exception as error:
        post["error"] = repr(error)
    post["elapsed_ms"] = (time.monotonic_ns() - began) / 1e6
    try:
        calibrated, measured = calibration(output, source)
    except Exception as error:
        calibrated, measured = False, {"error": repr(error)}
    if network_error is not None:
        measured["controller_error"] = network_error
        calibrated = False
    first, last = before["operations"][0], after["operations"][0] if after["operations"] else {}
    identity = dict(channel_before=first.get("channel"), channel_after=last.get("channel"),
                    connection_before=first["transport"]["connection"],
                    connection_after=last.get("transport", {}).get("connection"),
                    stream_before=first["transport"]["stream"], stream_after=last.get("transport", {}).get("stream"))
    identity["kind"] = "same_quic" if identity["connection_before"] == identity["connection_after"] else "replacement_quic"
    passed = (failure is None and effects == expected and echo_order == expected and post["passed"]
              and identity["channel_before"] == identity["channel_after"])
    report = dict(version=options.version, build_receipt=str(options.build_receipt),
                  artifacts=options.artifacts, source={key: value for key, value in source.items() if key != "points"},
                  timeline=str(options.timeline), calibrated=calibrated, calibration=measured,
                  status="fixture_calibration_failed" if not calibrated else "passed" if passed else "terminal_failed",
                  terminal_failure=failure, planned=count, queued=len(sent), executed=len(effects), echoes=len(received),
                  writes_exact=effects == expected, echoes_exact=echo_order == expected,
                  missing_execution=sorted(set(expected) - set(effects)),
                  missing_echoes=sorted(set(expected) - set(echo_order)),
                  duplicate_execution=sorted({name for name in effects if effects.count(name) > 1}),
                  original_window=snapshot, final_seconds=terminal_seconds, post_command=post,
                  attachment=attachment, identity=identity, before=before, after=after,
                  terminal_ms=metric([(received[name] - sent[name]) / 1e6 for name in received]))
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ["status", "planned", "queued", "executed", "echoes", "original_window", "identity"]}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--timeline", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = json.loads(options.timeline.read_text())
    receipt = json.loads(options.build_receipt.read_text())
    if receipt["exit"] != 0 or subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip() != options.version:
        parser.error("source or build precondition failed")
    options.artifacts = {}
    for name in ["miopunch-client", "browser-probe-native"]:
        path = root / "target/debug" / name
        stat = path.stat()
        options.artifacts[name] = dict(path=str(path), inode=stat.st_ino, size=stat.st_size, modified_ns=stat.st_mtime_ns)
    sys.path.insert(0, str(options.fixture))
    lab = load("lab", options.fixture / "lab.py")
    sys.modules["lab"] = lab
    clip = load("clip", options.fixture / "trace-run.py")
    clip.SOURCE = source
    clip.TIMELINE = options.timeline
    clip.POINTS = []
    last = None
    for raw in source["points"]:
        point = dict(raw)
        if point["status"] == "reply":
            last = point["rtt_ns"]
        elif point["status"] == "censored":
            point.update(status="reply", rtt_ns=last or 0)
        clip.POINTS.append(point)
    os.environ.update(MIO_LAB_TRACE="1", MIO_LAB_FULL="1")
    os.environ.setdefault("TOKIO_WORKER_THREADS", "2")
    lab.exercise = lambda *args: exercise(options, lab, clip, source, *args)
    try:
        lab.run(options.output)
    except Exception as error:
        path = options.output / "summary.json"
        if path.exists():
            report = json.loads(path.read_text())
            report["cleanup_error"] = repr(error)
        else:
            log = options.output / "terminal.jsonl"
            rows = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            try:
                calibrated, measured = calibration(options.output, source)
            except Exception as calibration_error:
                calibrated, measured = False, {"error": repr(calibration_error)}
            report = dict(version=options.version, build_receipt=str(options.build_receipt),
                          artifacts=options.artifacts, timeline=str(options.timeline),
                          source={key: value for key, value in source.items() if key != "points"},
                          status="terminal_failed" if calibrated else "fixture_calibration_failed",
                          calibrated=calibrated, calibration=measured, terminal_failure=repr(error),
                          queued=sum(row["event"] == "input" for row in rows),
                          echoes=sum(row["event"] == "data_echo" for row in rows),
                          last_progress=rows[-1] if rows else None,
                          post_command={"attempted": False, "error": "fixture failed before post-command observation"})
        path.write_text(json.dumps(report, indent=2) + "\n")
    for name, before in options.artifacts.items():
        stat = Path(before["path"]).stat()
        if (stat.st_ino, stat.st_size, stat.st_mtime_ns) != (before["inode"], before["size"], before["modified_ns"]):
            raise RuntimeError(f"artifact changed while running {name}")
    report = json.loads((options.output / "summary.json").read_text())
    return 0 if report["status"] == "passed" and "cleanup_error" not in report else 3 if not report["calibrated"] else 2


if __name__ == "__main__":
    sys.exit(main())
