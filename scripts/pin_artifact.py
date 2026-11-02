#!/usr/bin/env python3
"""Pin immutable build outputs once by SHA-256, independently of Cargo's cache."""
import argparse
import fcntl
import hashlib
import os
from pathlib import Path
import shutil
import stat
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STORE = ROOT / ".artifacts"
GIB = 1024 ** 3


def sha256(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def regular_file(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode):
        raise RuntimeError(f"artifact must be a regular file, not a symlink: {path}")
    return info


def identity(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def pin_artifact(source, destination, *, store=STORE, min_free=10 * GIB, max_bytes=32 * GIB):
    """Copy a mutable output once; hardlink only from the independent read-only store.

    Call under the build lease when source belongs to Cargo. Destinations are
    immutable: transform/strip/chmod a separate deployment copy instead.
    """
    source, destination, store = Path(source), Path(destination), Path(store)
    target = (ROOT / "target").resolve()
    if destination.resolve().is_relative_to(target) or store.resolve().is_relative_to(target):
        raise RuntimeError("immutable pins and their store must be outside the mutable Cargo target")
    before = regular_file(source)
    digest = sha256(source)
    mode = 0o555 if before.st_mode & 0o111 else 0o444
    store.mkdir(parents=True, exist_ok=True)
    with (store / ".pin.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        object_path = store / "sha256" / digest[:2] / f"{digest}-{mode:o}"
        if not object_path.exists():
            free = shutil.disk_usage(store).free
            if free - before.st_size < min_free:
                raise RuntimeError("artifact pin would leave less than the minimum free space")
            stored = sum(p.stat().st_blocks * 512 for p in (store / "sha256").rglob("*") if p.is_file())
            if stored + before.st_size > max_bytes:
                raise RuntimeError("immutable artifact store budget exceeded; review retained graphs before pinning")
            object_path.parent.mkdir(parents=True, exist_ok=True)
            descriptor, name = tempfile.mkstemp(prefix=".pin-", dir=object_path.parent)
            temporary = Path(name)
            try:
                with os.fdopen(descriptor, "wb") as output, source.open("rb") as incoming:
                    shutil.copyfileobj(incoming, output)
                    os.fchmod(output.fileno(), mode)
                if sha256(temporary) != digest or identity(regular_file(source)) != identity(before):
                    raise RuntimeError("source artifact changed while pinning")
                os.replace(temporary, object_path)
            finally:
                temporary.unlink(missing_ok=True)
        stored_info = regular_file(object_path)
        if (sha256(object_path) != digest or stat.S_IMODE(stored_info.st_mode) != mode
                or identity(regular_file(source)) != identity(before)):
            raise RuntimeError("immutable object is corrupt or source artifact changed")
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists() or destination.is_symlink():
            if destination.is_symlink() or not destination.samefile(object_path):
                raise FileExistsError(f"refusing to replace an existing artifact: {destination}")
        else:
            # EXDEV deliberately fails: no silent per-run copy on another filesystem.
            os.link(object_path, destination, follow_symlinks=False)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    print(pin_artifact(args.source, args.destination))


if __name__ == "__main__":
    main()
