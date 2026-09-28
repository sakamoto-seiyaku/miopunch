#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
asset_dir="${1:-${MIOPUNCH_RELEASE_OUT:-${repo_root}/dist/release}}"
version="${MIOPUNCH_VERSION:-}"
commit="$(git -C "${repo_root}" rev-parse HEAD 2>/dev/null || echo unknown)"

mkdir -p "${asset_dir}"

(
  cd "${asset_dir}"
  find . -maxdepth 1 -type f \
    ! -name checksums.txt \
    ! -name release-manifest.json \
    -printf '%f\0' \
    | sort -z \
    | xargs -0 --no-run-if-empty sha256sum > checksums.txt
)

python3 - "${asset_dir}" "${version}" "${commit}" <<'PY'
import hashlib
import json
import pathlib
import sys
from datetime import datetime, timezone

asset_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
commit = sys.argv[3]

assets = []
for path in sorted(asset_dir.iterdir(), key=lambda p: p.name):
    if not path.is_file() or path.name in {"checksums.txt", "release-manifest.json"}:
        continue
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    assets.append({
        "name": path.name,
        "size": path.stat().st_size,
        "sha256": digest,
    })

manifest = {
    "version": version,
    "commit": commit,
    "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    "assets": assets,
}

(asset_dir / "release-manifest.json").write_text(
    json.dumps(manifest, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

echo "wrote ${asset_dir}/checksums.txt"
echo "wrote ${asset_dir}/release-manifest.json"
