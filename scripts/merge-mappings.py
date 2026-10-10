#!/usr/bin/env python3
"""Fold what record mode learned into a game's definition, so everyone gets it.

Record mode saves each achievement it pinned down to
  <profile>/mapping/<game id>/<achievement id>.json
This writes the ones down to a single candidate into definitions/<game id>.toml
as `when` conditions (only for achievements that don't have any yet).

  scripts/merge-mappings.py silent-hill-3-pc [--profile DIR] [--dry-run]

(Players' shared mappings arrive through the requests sheet instead: see
scripts/merge-submissions.py, which uses the same rules.)
"""
import argparse, json, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROFILE = Path.home() / "Documents" / "My Games" / "Achievement Hub"
READS = {1: "u8", 2: "u16", 4: "u32"}


def condition(c, target=None, relative=False):
    """The `when` lines for a candidate (same rules as the overlay's
    recorder.rs Candidate::condition)."""
    rel = "\nrelative = true" if relative else ""
    if c["new"] == c["old"] + 1 and (target or c["old"] == 0):
        return f'read = "{READS[c["size"]]}"\naddress = {c["addr"]:#x}{rel}\nge = {target or 1}'
    on, off = c["new"] & ~c["old"], c["old"] & ~c["new"]
    if off == 0 and on and on & (on - 1) == 0:
        bit = on.bit_length() - 1
        mask = 1 << (bit % 8)
        return f'read = "u8"\naddress = {c["addr"] + bit // 8:#x}{rel}\nmask = {mask:#x}\neq = {mask:#x}'
    cmp = "ge" if c["new"] > c["old"] else "eq"
    return f'read = "{READS[c["size"]]}"\naddress = {c["addr"]:#x}{rel}\n{cmp} = {c["new"]}'


def builds(text):
    return {(int(t, 16), int(s, 16)) for t, s in re.findall(r"timestamp = (0x[0-9a-fA-F]+)\s*\nimage_size = (0x[0-9a-fA-F]+)", text)}


def apply(text, achievement, learned, note=""):
    """Adds a learned mapping to a definition's text. Returns (new text or
    None, message)."""
    if tuple(learned["build"]) not in builds(text):
        return None, "recorded on a different exe build"
    if len(learned["candidates"]) != 1:
        return None, f"{len(learned['candidates'])} possibilities left"
    block = re.search(rf'(\[\[achievement\]\]\nid = "{re.escape(achievement)}"\n(?:(?!\[\[achievement\]\]).*\n?)*)', text)
    if not block:
        return None, "no such achievement"
    if "[[achievement.when]]" in block.group(1):
        return None, "already has conditions"
    body = block.group(1).rstrip("\n")
    cond = condition(learned["candidates"][0], learned.get("target"), learned.get("relative", False))
    comment = f"# learned by record mode ({learned['recordings']} recordings){note}"
    new = f"{body}\n{comment}\n[[achievement.when]]\n{cond}\n"
    return text.replace(block.group(1), new + ("\n" if block.group(1).endswith("\n\n") else ""), 1), "added"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("game")
    ap.add_argument("--profile", type=Path, default=PROFILE)
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    path = ROOT / "definitions" / f"{a.game}.toml"
    text = path.read_text()
    added = 0
    for f in sorted((a.profile / "mapping" / a.game).glob("*.json")):
        new, msg = apply(text, f.stem, json.loads(f.read_text()))
        print(f"{f.stem}: {msg}")
        if new:
            text, added = new, added + 1
    if added and not a.dry_run:
        path.write_text(text)
    print(f"{added} achievement(s) {'would be ' if a.dry_run else ''}added to {path}")


if __name__ == "__main__":
    main()
