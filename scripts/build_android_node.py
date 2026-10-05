#!/usr/bin/env python3
"""Build the native CLI and optional Android JNI shell through the shared cache."""
import argparse

import build_android_pair


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ndk", required=True, help="Android NDK directory")
    parser.add_argument("--output", required=True, help="new build receipt directory")
    parser.add_argument("--jni-shell", action="store_true")
    parser.add_argument("--d8-jar", help="Android build-tools lib/d8.jar")
    args = parser.parse_args()
    # Preserve this entry's original non-RTC CLI graph and online Cargo behavior.
    arguments = ["--ndk", args.ndk, "--output", args.output, "--online",
                 "--bin", "miopunch-native-probe", "--features", ""]
    if args.jni_shell:
        arguments.append("--jni-shell")
    if args.d8_jar:
        arguments += ["--d8-jar", args.d8_jar]
    return build_android_pair.main(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
