#!/usr/bin/env python3
"""Local regression for recorded callback failure and repeated owner cleanup.

Replays a captured error context through loop.call_exception_handler. It never
induces a socket reset and does not claim to reproduce Windows Proactor I/O.
"""
import argparse
import asyncio
import gc
import importlib.metadata
import json
from pathlib import Path
import sys
import tempfile
import threading
import warnings


def second_close(owner):
    """Profile only this test thread's second close; alter no loop/library API."""
    watched = {asyncio.run_coroutine_threadsafe.__code__: "run_coroutine_threadsafe",
               owner.loop.call_soon_threadsafe.__func__.__code__: "call_soon_threadsafe"}
    calls = []
    def profile(frame, event, argument):
        if event == "call" and frame.f_code in watched:
            calls.append(watched[frame.f_code])
    previous = sys.getprofile()
    if previous is not None:
        raise RuntimeError("test thread already has a profiler")
    outcome, failure = None, None
    with warnings.catch_warnings(record=True) as observed:
        warnings.simplefilter("always")
        sys.setprofile(profile)
        try:
            outcome = owner.close()
        except BaseException as error:
            failure = error
        finally:
            sys.setprofile(previous)
        gc.collect()
    return outcome, failure, calls, [dict(type=type(row.message).__name__, text=str(row.message)) for row in observed]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--controllers", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    sys.path.insert(0, str(args.controllers.resolve()))
    from web_content_gateway import ContentGateway
    from web_protocol_target import WebTarget
    cases = []
    with tempfile.TemporaryDirectory(prefix="m119-local-owned-loop-") as temporary:
        for kind in ["gateway", "target"]:
            for replay in [False, True]:
                private = Path(temporary) / (kind + ("-callback" if replay else "-normal"))
                private.mkdir()
                events = []
                class Runner:
                    def __init__(self):
                        self.private, self.targets = private, []
                    def record(self, actor, operation, value):
                        events.append(dict(actor=actor, operation=operation, value=value))
                runner = Runner()
                owner = (ContentGateway("https://127.0.0.1:9/page", lambda request: None, private, [])
                         if kind == "gateway" else WebTarget(runner, tls=True))
                if kind == "gateway":
                    owner.record = lambda operation, value: runner.record("gateway", operation, value)
                if replay:
                    error = ConnectionResetError(10054, "replayed recorded WinError10054 callback context")
                    error.winerror = 10054
                    completed = threading.Event()
                    def replay_context():
                        try:
                            owner.loop.call_exception_handler(dict(
                                message="Exception in callback _ProactorBasePipeTransport._call_connection_lost()",
                                exception=error))
                        finally:
                            completed.set()
                    owner.loop.call_soon_threadsafe(replay_context)
                    assert completed.wait(5), "owned callback replay did not complete"
                first, first_error = None, None
                try:
                    first = owner.close()
                except BaseException as error:
                    first_error = error
                thread = owner.thread if kind == "gateway" else owner.reader
                terminal = owner.loop.is_closed() and not thread.is_alive()
                executor_zero = (not any(thread.is_alive() for thread in owner.executor._threads)
                                 if kind == "gateway" else True)
                second, second_error, calls, observed = second_close(owner)
                callback = [row for row in owner.observer.events if row["kind"] == "callback_failure"]
                checks = dict(original_loop_thread_executor_settled=terminal and executor_zero,
                    callback_enters_owner_failure=(bool(first_error) and "OwnedLoopCallbackError" in owner.errors
                        and len(callback) == 1 and callback[0]["exception_type"] == "ConnectionResetError"
                        and callback[0]["winerror"] == 10054) if replay else first_error is None,
                    first_close_error_retained=(second_error is first_error) if replay else second_error is None,
                    normal_receipt_reused=True if replay else isinstance(first, dict) and second is first,
                    second_close_schedules_nothing=calls == [], second_close_emits_no_warnings=observed == [],
                    target_closed_stays_WS_count=(type(owner.closed) is int and owner.closed == 0) if kind == "target" else True,
                    observer_summary_JSON=True)
                json.dumps(owner.observer.summary(), allow_nan=False)
                cases.append(dict(kind=kind, replay=replay, checks=checks,
                    first_error=type(first_error).__name__ if first_error else None,
                    second_error=type(second_error).__name__ if second_error else None,
                    second_close_calls=calls, second_close_warnings=observed,
                    observer=owner.observer.summary(), events=events))
                assert terminal and executor_zero, "original local owner did not settle; preserve fixture"
    result = dict(result="PASS" if all(all(case["checks"].values()) for case in cases) else "FAILED",
                  scope="local callback qualification/repeated-close seam; not Windows10054 I/O reproduction or fix",
                  python=sys.version, packages={name: importlib.metadata.version(name) for name in ["aiohttp", "cryptography"]},
                  cases=cases)
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(dict(result=result["result"], cases=[dict(kind=case["kind"], replay=case["replay"],
        failed_checks=[name for name, passed in case["checks"].items() if not passed]) for case in cases])), flush=True)
    if result["result"] != "PASS":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
