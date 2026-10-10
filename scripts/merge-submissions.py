#!/usr/bin/env python3
"""Adds mappings players shared from the desktop app to the game
definitions.

On the website's Admin page, click "Download new mappings (JSON)", then:

  scripts/merge-submissions.py ~/Downloads/shared-mappings.json [--dry-run]
  scripts/publish-db.sh

and mark them "merged" on the Admin page. (A CSV from the old requests
sheet works too: rows marked MAPPING.)
"""
import argparse, csv, importlib.util, json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("mm", ROOT / "scripts" / "merge-mappings.py")
mm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mm)


def submissions(path):
    if path.endswith(".json"):
        for m in json.load(open(path, encoding="utf-8")):
            yield m["game"], m["achievement"], m["data"], m.get("gamertag", "")
        return
    for row in csv.reader(open(path, newline="", encoding="utf-8")):
        if "MAPPING" not in row:
            continue
        i = row.index("MAPPING")
        learned = next((json.loads(c) for c in row if c.strip().startswith("{") and '"candidates"' in c), None)
        if i == 0 or i + 1 >= len(row) or learned is None:
            continue
        # The gamertag: the non-empty cell before the JSON that isn't one of the others.
        others = [c for c in row[i + 2:] if c and not c.strip().startswith("{")]
        yield row[i - 1], row[i + 1], learned, (others[-1] if others else "")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("file", help="shared-mappings.json from the Admin page (or an old sheet CSV)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    changed = {}
    for game, achievement, learned, who in submissions(a.file):
        path = ROOT / "definitions" / f"{game}.toml"
        if not path.exists():
            print(f"{game}/{achievement}: no definitions/{game}.toml")
            continue
        text = changed.get(path) or path.read_text()
        new, msg = mm.apply(text, achievement, learned, f", shared by {who}" if who else "")
        print(f"{game}/{achievement}: {msg}")
        if new:
            changed[path] = new
    for path, text in changed.items():
        if not a.dry_run:
            path.write_text(text)
    print(f"{len(changed)} definition(s) {'would be ' if a.dry_run else ''}updated")


if __name__ == "__main__":
    main()
