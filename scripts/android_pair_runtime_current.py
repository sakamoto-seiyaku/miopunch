#!/usr/bin/env python3
"""Current M82 Android pair runtime adapters.

This module keeps the existing Android actor/page lifecycle intact while
binding the current story to the two observed USB devices.  The legacy M76
controller imports :mod:`android_pair_runtime` by name; the current
controller installs this module under that name only for its own process.
"""

from __future__ import annotations

import importlib
from pathlib import Path
import sys

_SCRIPT_DIR = Path(__file__).resolve().parent
if str(_SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(_SCRIPT_DIR))

try:
    _base = importlib.import_module("android_pair_runtime")
except ModuleNotFoundError:  # ``python -m scripts...`` from the repository root.
    _base = importlib.import_module("scripts.android_pair_runtime")

ROOT = _base.ROOT
ADB = _base.ADB
BrowserPage = _base.BrowserPage
NativeActor = _base.NativeActor
browser_cleanup_succeeded = _base.browser_cleanup_succeeded

# These are observed runtime roles, not device serials.  ADB remains a
# control channel; it is never treated as proof of the SDK data route.
CURRENT_DEVICES = {
    "native": {"model": "PJZ110", "transport": "usb", "platform": "Android/PJZ110"},
    "browser": {"model": "Pixel_6a", "transport": "usb", "platform": "Android/Pixel_6a"},
}

# The old controller's only Android call still names TB322FC/network.  It is
# translated at this seam so the frozen controller is not edited.  The
# resulting object is always constructed with the actual current model and
# USB transport; no network ADB alias is accepted.
LEGACY_NATIVE_CALL = ("TB322FC", "network")


class CurrentDevice(_base.Device):
    """An existing ``Device`` with an exact current-pair binding."""

    def __init__(self, model: str, transport: str, adb=ADB, *, role: str | None = None):
        requested = (str(model), str(transport))
        if requested == LEGACY_NATIVE_CALL:
            requested = (CURRENT_DEVICES["native"]["model"], CURRENT_DEVICES["native"]["transport"])
        match = next((name for name, value in CURRENT_DEVICES.items()
                      if (value["model"], value["transport"]) == requested), None)
        if match is None:
            raise RuntimeError("current M82 pair requires the observed USB model/transport")
        if role is not None and role != match:
            raise RuntimeError("current device role does not match its observed model")
        super().__init__(requested[0], requested[1], adb)
        self.runtime_role = match
        self.runtime_platform = CURRENT_DEVICES[match]["platform"]
        self.observed_model = requested[0]
        self.observed_transport = requested[1]
        self.requested_model = str(model)
        self.requested_transport = str(transport)


def device_for(role: str, adb=ADB) -> CurrentDevice:
    """Select one current device by role, never by an unobserved serial."""

    if role not in CURRENT_DEVICES:
        raise ValueError("unknown current M82 device role")
    if role == "native":
        return CurrentDevice("PJZ110", "usb", adb, role="native")
    return CurrentDevice("Pixel_6a", "usb", adb, role="browser")


def legacy_compatible_device(model: str, transport: str, adb=ADB) -> CurrentDevice:
    """Adapter used by the frozen controller's old constructor call."""

    return CurrentDevice(model, transport, adb)


# The frozen controller imports ``Device`` by name.  Its Android branch still
# passes the historical TB322FC/network pair, which the adapter translates
# above before constructing the actual PJZ110/usb object.
Device = legacy_compatible_device


def runtime_selection() -> dict:
    """Public-safe role labels; no serial, address, identity, or secrets."""

    return {
        "native": {
            "model": CURRENT_DEVICES["native"]["model"],
            "transport": CURRENT_DEVICES["native"]["transport"],
            "platform": CURRENT_DEVICES["native"]["platform"],
        },
        "browser": {
            "model": CURRENT_DEVICES["browser"]["model"],
            "transport": CURRENT_DEVICES["browser"]["transport"],
            "platform": CURRENT_DEVICES["browser"]["platform"],
        },
    }


def install_for_frozen_controller() -> None:
    """Make the frozen controller import this current seam in this process.

    The shim deliberately does not change any shared runtime module on disk.
    It is installed after artifact guards and before the delegated controller
    creates its first actor.
    """

    module = sys.modules[__name__]
    sys.modules["android_pair_runtime"] = module
