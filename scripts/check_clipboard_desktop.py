#!/usr/bin/env python3
"""No-business preflight for authenticated independent X11 desktop isolation."""
import argparse
from pathlib import Path
import subprocess

from check_clipboard import Desktop, Runner


class DesktopRunner(Runner):
    def scenario(self):
        first, second = Desktop(self, "isolation-first"), Desktop(self, "isolation-second")
        first.copy("M117 owned first private display marker")
        second.copy("M117 owned second private display marker")
        assert first.read() == "M117 owned first private display marker"
        assert second.read() == "M117 owned second private display marker"
        assert first.env["DISPLAY"] != second.env["DISPLAY"]
        wrong_authority = dict(second.env, XAUTHORITY=first.env["XAUTHORITY"])
        rejected = subprocess.run([str(self.tools / "xclip"), "-selection", "clipboard", "-out", "-target", "UTF8_STRING"],
                                  env=wrong_authority, capture_output=True, timeout=3)
        assert rejected.returncode != 0 and not rejected.stdout, "owned X server did not enforce its private authority"
        first.close()
        unavailable = subprocess.run([str(self.tools / "xclip"), "-selection", "clipboard", "-out", "-target", "UTF8_STRING"],
                                     env=first.env, capture_output=True, timeout=3)
        assert unavailable.returncode != 0 and not unavailable.stdout, "closed private display unexpectedly produced data"
        assert second.read() == "M117 owned second private display marker"
        self.record("isolation", "no_business_authenticated_preflight", {"distinct_high_displays": True,
                    "wrong_private_authority_rejected_without_data": True,
                    "closed_display_unavailable_without_data": True,
                    "other_owned_selection_unchanged": True, "no_sdk_or_business_transport": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--linux", type=Path, required=True)
    parser.add_argument("--linux-sha", required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.case = "desktop"
    DesktopRunner(args).run()


if __name__ == "__main__":
    main()
