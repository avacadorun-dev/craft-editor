#!/usr/bin/env python3
"""Verify pinned supplemental syntax data and its redistribution provenance."""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "crates/craft-editor-egui"
manifest = json.loads((root / "assets/syntaxes/manifest.json").read_text())
if len(manifest) != 10:
    raise SystemExit("expected ten supplemental native grammars")
for item in manifest:
    path = root / item["file"]
    if hashlib.sha256(path.read_bytes()).hexdigest() != item["sha256"]:
        raise SystemExit(f"syntax source checksum differs: {item['file']}")
    if not item["source"].startswith("https://github.com/") or len(item["source_revision"]) != 40:
        raise SystemExit(f"syntax source revision is not pinned: {item['file']}")
    if not (root / item["license_file"]).read_text().strip():
        raise SystemExit(f"missing syntax license: {item['file']}")
samples = json.loads((root / "assets/syntaxes/source-format-samples.json").read_text())
if len(samples) != 77 or len({row["language"] for row in samples}) != 77:
    raise SystemExit("expected 77 distinct original format samples")
print("Verified ten pinned native supplemental grammars, licenses and 77 synthetic source formats.")
