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
package = next(package for package in metadata["packages"] if package["name"] == "wordcraft-layout")
if not package["source"].startswith("git+https://github.com/avacadorun-dev/wordcraft?") or not package["source"].endswith("#" + REVISION):
    raise SystemExit("The backend regression must use the exact qualified maintained fork")
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
    "scope": "Three standard nested nonfloating table-margin regressions; not upstream full-app acceptance",
    "source_hashes": {
        path: hashlib.sha256((root / path).read_bytes()).hexdigest()
        for path in ("crates/layout/src/boxes.rs", "crates/layout/src/table_flow.rs", "crates/layout/src/tests.rs")
    },
}
output = Path("artifacts/wordcraft-backend.json")
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(report, indent=2) + "\n")
