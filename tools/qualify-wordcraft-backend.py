#!/usr/bin/env python3
"""Verify the exact maintained backend and run its focused nested-table regressions on CI."""
import hashlib
import json
from pathlib import Path
import subprocess

REVISION = "82ace77a17d71f8fce770227624319203e74cc7b"
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--all-features", "--locked", "--format-version", "1"
], text=True))
backend = [package for package in metadata["packages"] if package["name"].startswith("wordcraft-")]
for package in backend:
    source = package.get("source") or ""
    if not source.startswith("git+https://github.com/avacadorun-dev/wordcraft?") or not source.endswith("#" + REVISION):
        raise SystemExit("Every backend package must share the exact maintained source; mixed font globals are invalid")
if len({package["name"] for package in backend}) != len(backend):
    raise SystemExit("Duplicate backend package identities would split shared font/layout state")
package = next(package for package in backend if package["name"] == "wordcraft-layout")
manifest = Path(package["manifest_path"])
root = manifest.parent.parent.parent
actual = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
if actual != REVISION:
    raise SystemExit("The backend checkout identity differs from the dependency source")
subprocess.run([
    "cargo", "test", "--manifest-path", str(manifest), "--locked", "--lib", "nested_table_margin_"
], check=True)
report = {
    "repository": "https://github.com/avacadorun-dev/wordcraft",
    "revision": REVISION,
    "upstream_base": "da914ce674f18ebc4e08cca425cc8f8acca3372f",
    "packages": {package["name"]: package["source"] for package in backend},
    "scope": "Three standard nested nonfloating table-margin regressions; not upstream full-app acceptance",
    "source_hashes": {
        path: hashlib.sha256((root / path).read_bytes()).hexdigest()
        for path in ("crates/layout/src/boxes.rs", "crates/layout/src/table_flow.rs", "crates/layout/src/tests.rs")
    },
}
output = Path("artifacts/wordcraft-backend.json")
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(report, indent=2) + "\n")
