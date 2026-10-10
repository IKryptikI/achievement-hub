#!/usr/bin/env python3
"""Writes definitions/index.json for the online game database: every
definition's file name, id, title and SHA-256 (the app downloads a file when
its hash changes).

  scripts/build-index.py
"""
import hashlib, json, sys, tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "definitions"
games = []
for f in sorted(ROOT.glob("*.toml")):
    data = f.read_bytes()
    try:
        d = tomllib.loads(data.decode())
    except Exception as e:
        sys.exit(f"{f.name}: {e}")
    games.append({
        "file": f.name,
        "id": d["game"]["id"],
        "title": d["game"]["title"],
        "achievements": len(d.get("achievement", [])),
        "sha256": hashlib.sha256(data).hexdigest(),
    })
(ROOT / "index.json").write_text(json.dumps({"version": 1, "games": games}, indent=1) + "\n")
print(f"index.json: {len(games)} games")
